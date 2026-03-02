/**
 * Push notification management for PWA.
 * Handles subscription, unsubscription, and permission management.
 */

/** @type {PushSubscription|null} */
var currentSubscription = null;

/** @type {string|null} */
var vapidPublicKey = null;

/**
 * Convert a base64 string to a Uint8Array (for VAPID key).
 * @param {string} base64String - Base64 URL-safe encoded string
 * @returns {Uint8Array}
 */
function urlBase64ToUint8Array(base64String) {
	var padding = "=".repeat((4 - (base64String.length % 4)) % 4);
	var base64 = (base64String + padding).replace(/-/g, "+").replace(/_/g, "/");
	var rawData = window.atob(base64);
	var outputArray = new Uint8Array(rawData.length);
	for (var i = 0; i < rawData.length; ++i) {
		outputArray[i] = rawData.charCodeAt(i);
	}
	return outputArray;
}

/**
 * Check if push notifications are supported.
 * @returns {boolean}
 */
export function isPushSupported() {
	return "PushManager" in window && "serviceWorker" in navigator;
}

/**
 * Get the current notification permission state.
 * @returns {'granted'|'denied'|'default'}
 */
export function getPermissionState() {
	if (!isPushSupported()) {
		return "denied";
	}
	return Notification.permission;
}

/**
 * Check if push notifications are currently enabled (subscribed).
 * @returns {boolean}
 */
export function isSubscribed() {
	return currentSubscription !== null;
}

/**
 * Fetch the VAPID public key from the server.
 * @returns {Promise<string|null>}
 */
async function fetchVapidKey() {
	if (vapidPublicKey) {
		return vapidPublicKey;
	}
	try {
		var response = await fetch("/api/push/vapid-key");
		if (!response.ok) {
			console.warn("Push notifications not available on server");
			return null;
		}
		var data = await response.json();
		vapidPublicKey = data.public_key;
		return vapidPublicKey;
	} catch (e) {
		console.error("Failed to fetch VAPID key:", e);
		return null;
	}
}

/**
 * Get the current push subscription from the service worker.
 * @returns {Promise<PushSubscription|null>}
 */
async function getCurrentSubscription() {
	if (!isPushSupported()) {
		return null;
	}
	try {
		var registration = await navigator.serviceWorker.ready;
		var subscription = await registration.pushManager.getSubscription();
		currentSubscription = subscription;
		return subscription;
	} catch (e) {
		console.error("Failed to get push subscription:", e);
		return null;
	}
}

/**
 * Subscribe to push notifications.
 * Requests permission if needed, creates subscription, and registers with server.
 * @returns {Promise<{success: boolean, error?: string}>}
 */
export async function subscribeToPush() {
	if (!isPushSupported()) {
		return { success: false, error: "Push notifications not supported" };
	}

	// Request permission
	var permission = await Notification.requestPermission();
	if (permission !== "granted") {
		return { success: false, error: "Permission denied" };
	}

	// Get VAPID key
	var key = await fetchVapidKey();
	if (!key) {
		return { success: false, error: "Push notifications not configured on server" };
	}

	try {
		var registration = await navigator.serviceWorker.ready;

		// Subscribe to push
		var subscription = await registration.pushManager.subscribe({
			userVisibleOnly: true,
			applicationServerKey: urlBase64ToUint8Array(key),
		});

		// Send subscription to server
		var response = await fetch("/api/push/subscribe", {
			method: "POST",
			headers: {
				"Content-Type": "application/json",
			},
			body: JSON.stringify({
				endpoint: subscription.endpoint,
				keys: {
					p256dh: btoa(String.fromCharCode(...new Uint8Array(subscription.getKey("p256dh"))))
						.replace(/\+/g, "-")
						.replace(/\//g, "_")
						.replace(/=+$/, ""),
					auth: btoa(String.fromCharCode(...new Uint8Array(subscription.getKey("auth"))))
						.replace(/\+/g, "-")
						.replace(/\//g, "_")
						.replace(/=+$/, ""),
				},
			}),
		});

		if (!response.ok) {
			throw new Error("Server rejected subscription");
		}

		currentSubscription = subscription;
		return { success: true };
	} catch (e) {
		console.error("Failed to subscribe to push:", e);
		return { success: false, error: e.message };
	}
}

/**
 * Unsubscribe from push notifications.
 * @returns {Promise<{success: boolean, error?: string}>}
 */
export async function unsubscribeFromPush() {
	var subscription = await getCurrentSubscription();
	if (!subscription) {
		return { success: true }; // Already unsubscribed
	}

	try {
		// Unsubscribe locally
		await subscription.unsubscribe();

		// Notify server
		await fetch("/api/push/unsubscribe", {
			method: "POST",
			headers: {
				"Content-Type": "application/json",
			},
			body: JSON.stringify({
				endpoint: subscription.endpoint,
			}),
		});

		currentSubscription = null;
		return { success: true };
	} catch (e) {
		console.error("Failed to unsubscribe from push:", e);
		return { success: false, error: e.message };
	}
}

/**
 * Initialize push notification state.
 * Call this on page load to sync with existing subscription.
 * @returns {Promise<void>}
 */
export async function initPushState() {
	await getCurrentSubscription();
}

/**
 * Get push notification status from server.
 * @returns {Promise<{enabled: boolean, subscription_count: number}|null>}
 */
export async function getPushStatus() {
	try {
		var response = await fetch("/api/push/status");
		if (!response.ok) {
			return null;
		}
		return await response.json();
	} catch (e) {
		console.error("Failed to get push status:", e);
		return null;
	}
}

/**
 * Remove a subscription from the server by its endpoint.
 * This can be called from any device to remove any subscription.
 * @param {string} endpoint - The subscription endpoint to remove
 * @returns {Promise<{success: boolean, error?: string}>}
 */
export async function removeSubscription(endpoint) {
	try {
		var response = await fetch("/api/push/unsubscribe", {
			method: "POST",
			headers: {
				"Content-Type": "application/json",
			},
			body: JSON.stringify({ endpoint }),
		});

		if (!response.ok) {
			return { success: false, error: "Failed to remove subscription" };
		}

		// If this was our own subscription, clear local state
		if (currentSubscription?.endpoint === endpoint) {
			try {
				await currentSubscription.unsubscribe();
			} catch (_e) {
				// Ignore errors - subscription may already be gone
			}
			currentSubscription = null;
		}

		return { success: true };
	} catch (e) {
		console.error("Failed to remove subscription:", e);
		return { success: false, error: e.message };
	}
}
