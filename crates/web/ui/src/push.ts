/**
 * Push notification management for PWA.
 * Handles subscription, unsubscription, and permission management.
 */

let currentSubscription: PushSubscription | null = null;

let vapidPublicKey: string | null = null;

/**
 * Convert a base64 string to a Uint8Array (for VAPID key).
 */
function urlBase64ToUint8Array(base64String: string): Uint8Array {
	const padding = "=".repeat((4 - (base64String.length % 4)) % 4);
	const base64 = (base64String + padding).replace(/-/g, "+").replace(/_/g, "/");
	const rawData = window.atob(base64);
	const outputArray = new Uint8Array(rawData.length);
	for (let i = 0; i < rawData.length; ++i) {
		outputArray[i] = rawData.charCodeAt(i);
	}
	return outputArray;
}

/**
 * Check if push notifications are supported.
 */
export function isPushSupported(): boolean {
	return "PushManager" in window && "serviceWorker" in navigator;
}

/**
 * Get the current notification permission state.
 */
export function getPermissionState(): NotificationPermission {
	if (!isPushSupported()) {
		return "denied";
	}
	return Notification.permission;
}

/**
 * Check if push notifications are currently enabled (subscribed).
 */
export function isSubscribed(): boolean {
	return currentSubscription !== null;
}

/**
 * Fetch the VAPID public key from the server.
 */
async function fetchVapidKey(): Promise<string | null> {
	if (vapidPublicKey) {
		return vapidPublicKey;
	}
	try {
		const response = await fetch("/api/push/vapid-key");
		if (!response.ok) {
			console.warn("Push notifications not available on server");
			return null;
		}
		const data: { public_key: string } = await response.json();
		vapidPublicKey = data.public_key;
		return vapidPublicKey;
	} catch (e) {
		console.error("Failed to fetch VAPID key:", e);
		return null;
	}
}

/**
 * Get the current push subscription from the service worker.
 */
async function getCurrentSubscription(): Promise<PushSubscription | null> {
	if (!isPushSupported()) {
		return null;
	}
	try {
		const registration = await navigator.serviceWorker.ready;
		const subscription = await registration.pushManager.getSubscription();
		currentSubscription = subscription;
		return subscription;
	} catch (e) {
		console.error("Failed to get push subscription:", e);
		return null;
	}
}

/** Result of a push subscribe/unsubscribe operation. */
interface PushResult {
	success: boolean;
	error?: string;
}

/**
 * Subscribe to push notifications.
 * Requests permission if needed, creates subscription, and registers with server.
 */
export async function subscribeToPush(): Promise<PushResult> {
	if (!isPushSupported()) {
		return { success: false, error: "Push notifications not supported" };
	}

	// Request permission
	const permission = await Notification.requestPermission();
	if (permission !== "granted") {
		return { success: false, error: "Permission denied" };
	}

	// Get VAPID key
	const key = await fetchVapidKey();
	if (!key) {
		return { success: false, error: "Push notifications not configured on server" };
	}

	try {
		const registration = await navigator.serviceWorker.ready;

		// Subscribe to push
		const subscription = await registration.pushManager.subscribe({
			userVisibleOnly: true,
			applicationServerKey: urlBase64ToUint8Array(key).buffer as ArrayBuffer,
		});

		// Send subscription to server
		const response = await fetch("/api/push/subscribe", {
			method: "POST",
			headers: {
				"Content-Type": "application/json",
			},
			body: JSON.stringify({
				endpoint: subscription.endpoint,
				keys: {
					p256dh: btoa(String.fromCharCode(...new Uint8Array(subscription.getKey("p256dh")!)))
						.replace(/\+/g, "-")
						.replace(/\//g, "_")
						.replace(/=+$/, ""),
					auth: btoa(String.fromCharCode(...new Uint8Array(subscription.getKey("auth")!)))
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
		return { success: false, error: (e as Error).message };
	}
}

/**
 * Unsubscribe from push notifications.
 */
export async function unsubscribeFromPush(): Promise<PushResult> {
	const subscription = await getCurrentSubscription();
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
		return { success: false, error: (e as Error).message };
	}
}

/**
 * Initialize push notification state.
 * Call this on page load to sync with existing subscription.
 */
export async function initPushState(): Promise<void> {
	await getCurrentSubscription();
}

/** Status returned by the push status endpoint. */
interface PushStatus {
	enabled: boolean;
	subscription_count: number;
}

/**
 * Get push notification status from server.
 */
export async function getPushStatus(): Promise<PushStatus | null> {
	try {
		const response = await fetch("/api/push/status");
		if (!response.ok) {
			return null;
		}
		return (await response.json()) as PushStatus;
	} catch (e) {
		console.error("Failed to get push status:", e);
		return null;
	}
}

/**
 * Remove a subscription from the server by its endpoint.
 * This can be called from any device to remove any subscription.
 */
export async function removeSubscription(endpoint: string): Promise<PushResult> {
	try {
		const response = await fetch("/api/push/unsubscribe", {
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
		return { success: false, error: (e as Error).message };
	}
}
