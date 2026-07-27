/**
 * Push notification management for PWA.
 * Handles subscription, unsubscription, permission management, and reporting
 * foreground presence so the server can skip devices already watching a chat.
 */

import { activeSessionKey } from "./stores/session-store";

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
 *
 * Returns `null` without touching `pushManager` unless notification permission
 * has been granted. A subscription cannot exist without it, and querying
 * `pushManager` where no push backend is reachable is not merely slow — it can
 * take down the renderer, which would break the whole app rather than just
 * push. This runs on every page load, so it has to fail safe.
 */
async function getCurrentSubscription(): Promise<PushSubscription | null> {
	if (!isPushSupported() || getPermissionState() !== "granted") {
		currentSubscription = null;
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

/** Encode an ArrayBuffer as unpadded base64url, the wire format for push keys. */
function encodeKey(buffer: ArrayBuffer | null): string | null {
	if (!buffer) return null;
	const bytes = new Uint8Array(buffer);
	let binary = "";
	for (const byte of bytes) {
		binary += String.fromCharCode(byte);
	}
	return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** Post a subscription to the server, optionally replacing a rotated endpoint. */
async function registerWithServer(subscription: PushSubscription, replaces?: string): Promise<void> {
	const p256dh = encodeKey(subscription.getKey("p256dh"));
	const auth = encodeKey(subscription.getKey("auth"));
	if (!(p256dh && auth)) {
		throw new Error("Push subscription is missing encryption keys");
	}

	const response = await fetch("/api/push/subscribe", {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({
			endpoint: subscription.endpoint,
			keys: { p256dh, auth },
			replaces,
		}),
	});

	if (!response.ok) {
		throw new Error("Server rejected subscription");
	}
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

		await registerWithServer(subscription);

		currentSubscription = subscription;
		reportPresence();
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
 * Re-register the local subscription if the server no longer knows about it.
 *
 * Subscriptions are dropped server-side when a push returns 410/404, and the
 * browser can hand back a subscription signed with a VAPID key the server has
 * since rotated. Either way the browser still reports itself as subscribed
 * while no push can ever arrive, so reconcile both sides on load.
 */
async function reconcileSubscription(subscription: PushSubscription): Promise<void> {
	const key = await fetchVapidKey();
	if (!key) return;

	// A VAPID rotation invalidates the existing subscription outright.
	const currentKey = encodeKey(subscription.options?.applicationServerKey ?? null);
	if (currentKey && currentKey !== key) {
		const staleEndpoint = subscription.endpoint;
		await subscription.unsubscribe().catch(() => undefined);
		const registration = await navigator.serviceWorker.ready;
		const fresh = await registration.pushManager.subscribe({
			userVisibleOnly: true,
			applicationServerKey: urlBase64ToUint8Array(key).buffer as ArrayBuffer,
		});
		await registerWithServer(fresh, staleEndpoint);
		currentSubscription = fresh;
		return;
	}

	const status = await getPushStatus();
	const known = status?.subscriptions?.some((s) => s.endpoint === subscription.endpoint) ?? false;
	if (!known) {
		await registerWithServer(subscription);
	}
}

/** Clear the worker's "rotation could not be registered" flag once repaired. */
async function clearRotationPending(): Promise<void> {
	try {
		const cache = await caches.open("moltis-state");
		await cache.delete("/__moltis__/rotation-pending");
	} catch {
		// Nothing to clear, or Cache API unavailable.
	}
}

/**
 * Initialize push notification state.
 *
 * Runs on every page load, not just from the settings page: an endpoint the
 * server has forgotten — or a rotation the worker could not register while the
 * app was closed — leaves the browser believing it is subscribed while nothing
 * can ever be delivered. This load is the only chance to repair that.
 */
export async function initPushState(): Promise<void> {
	const subscription = await getCurrentSubscription();
	if (!subscription) return;
	try {
		await reconcileSubscription(subscription);
		await clearRotationPending();
	} catch (e) {
		console.warn("Failed to reconcile push subscription:", e);
	}
}

/** A subscription as reported by the server. */
interface PushSubscriptionSummary {
	endpoint: string;
	device?: string;
	ip?: string;
	created_at?: string;
}

/** Status returned by the push status endpoint. */
interface PushStatus {
	enabled: boolean;
	subscription_count: number;
	subscriptions?: PushSubscriptionSummary[];
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

/**
 * Send a test notification to every subscribed device.
 *
 * Returns how many devices the push service accepted it for.
 */
export async function sendTestNotification(): Promise<{ success: boolean; sent?: number; error?: string }> {
	try {
		const response = await fetch("/api/push/test", { method: "POST" });
		if (!response.ok) {
			return {
				success: false,
				error: response.status === 501 ? "Push notifications are not enabled on the server" : "Failed to send",
			};
		}
		const data = (await response.json()) as { sent: number };
		return { success: true, sent: data.sent };
	} catch (e) {
		return { success: false, error: (e as Error).message };
	}
}

// ── Foreground presence ─────────────────────────────────────────────────────

/** Last presence payload sent, used to avoid redundant round-trips. */
let lastPresence = "";

/**
 * Tell the server which session this device is looking at, if any.
 *
 * The server skips push delivery to an endpoint that reports itself visible on
 * the session that just produced a response — that's what stops your phone from
 * buzzing for a message you are watching stream in on that same phone.
 */
export function reportPresence(): void {
	if (!currentSubscription) return;

	const visible = document.visibilityState === "visible" && document.hasFocus();
	const sessionKey = visible ? activeSessionKey.value : null;
	const payload = JSON.stringify({
		endpoint: currentSubscription.endpoint,
		session_key: sessionKey,
		visible,
	});

	if (payload === lastPresence) return;
	lastPresence = payload;

	// keepalive lets the "hidden" report survive the page being backgrounded or
	// closed, which is exactly when it matters most.
	fetch("/api/push/presence", {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: payload,
		keepalive: true,
	})
		.then((response) => {
			if (response.ok) return;

			// `fetch` resolves for 4xx, so a 404 here would otherwise look like a
			// successful report — and the cached payload would suppress every
			// retry. 404 is the server saying it does not know this endpoint,
			// which means push delivery is broken too, not just suppression.
			lastPresence = "";
			if (response.status === 404) {
				void recoverUnknownSubscription();
			}
		})
		.catch(() => {
			// Presence is an optimisation — a failed report only means the device
			// may receive a notification it could have suppressed.
			lastPresence = "";
		});
}

/** Guards against several concurrent recovery attempts. */
let recovering = false;
/** When recovery last ran, to bound retries if the server keeps rejecting. */
let lastRecoveryAt = 0;
const RECOVERY_COOLDOWN_MS = 60_000;

/**
 * Re-register a subscription the server has forgotten.
 *
 * Reached when presence reports 404. Left alone, the browser would keep a
 * subscription that receives nothing while the server has no record to push to.
 *
 * Recovery re-reports presence on success, so a server that answers 404 even
 * after accepting the registration would otherwise drive an endless
 * register/report loop. The cooldown bounds that to one attempt a minute.
 */
async function recoverUnknownSubscription(): Promise<void> {
	if (recovering || !currentSubscription) return;
	if (Date.now() - lastRecoveryAt < RECOVERY_COOLDOWN_MS) return;

	recovering = true;
	lastRecoveryAt = Date.now();
	try {
		await registerWithServer(currentSubscription);
		reportPresence();
	} catch (e) {
		console.warn("Failed to re-register push subscription:", e);
	} finally {
		recovering = false;
	}
}

/** Ask the service worker to clear notifications for the session in view. */
function clearNotificationsForActiveSession(): void {
	navigator.serviceWorker?.controller?.postMessage({
		type: "CLEAR_NOTIFICATIONS",
		sessionKey: activeSessionKey.value,
	});
}

/**
 * Start reporting presence on visibility, focus, and session changes.
 *
 * Loads the existing subscription first: presence must work on every page load,
 * not only after the user has opened Settings → Notifications.
 */
export function initPresenceReporting(): void {
	if (!isPushSupported()) return;

	const onForeground = (): void => {
		reportPresence();
		if (document.visibilityState === "visible") {
			clearNotificationsForActiveSession();
		}
	};

	document.addEventListener("visibilitychange", onForeground);
	window.addEventListener("focus", onForeground);
	window.addEventListener("blur", reportPresence);
	window.addEventListener("pagehide", reportPresence);
	activeSessionKey.subscribe(onForeground);

	// Reconcile before the first report: this is the app-startup path, so it is
	// where a subscription the server has forgotten gets re-registered rather
	// than silently receiving nothing until someone opens Settings.
	initPushState()
		.then(() => onForeground())
		.catch(() => {
			// No subscription: presence stays a no-op, which is correct.
		});
}
