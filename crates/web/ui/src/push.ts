/**
 * Push notification management for PWA.
 * Handles subscription, unsubscription, permission management, and reporting
 * foreground presence so the server can skip devices already watching a chat.
 */

import { sessionPath } from "./router";
import { activeSessionKey } from "./stores/session-store";

let currentSubscription: PushSubscription | null = null;

let vapidPublicKey: string | null = null;
let pushStateQueue: Promise<void> = Promise.resolve();

const STATE_CACHE = "moltis-state";
const PUSH_ENABLED_KEY = "/__moltis__/push-enabled";
const PUSH_DISABLED_KEY = "/__moltis__/push-disabled";
const PUSH_ROTATION_PENDING_KEY = "/__moltis__/push-rotation-pending";
const PUSH_INTENT_GENERATION_KEY = "/__moltis__/push-intent-generation";
const PUSH_INTENT_KEY = "moltis-push-enabled";
const PUSH_STATE_LOCK = "moltis-push-state";
let intentGeneration = 0;

interface RotationPending {
	replaces?: string;
	revive?: boolean;
}

async function getStateCache(): Promise<Cache> {
	return await caches.open(STATE_CACHE);
}

function runPushStateOperation<T>(operation: () => Promise<T>): Promise<T> {
	const globallyOrdered = async (): Promise<T> => {
		if (!navigator.locks) return operation();
		return await navigator.locks.request<Promise<T>>(PUSH_STATE_LOCK, operation);
	};
	const result = pushStateQueue.then(globallyOrdered, globallyOrdered);
	pushStateQueue = result.then(
		() => undefined,
		() => undefined,
	);
	return result;
}

async function setPushEnabledIntent(enabled: boolean): Promise<void> {
	try {
		localStorage.setItem(PUSH_INTENT_KEY, enabled ? "1" : "0");
	} catch (error) {
		console.warn("Failed to persist push preference in local storage:", error);
	}
	try {
		const cache = await getStateCache();
		if (enabled) {
			await Promise.all([cache.put(PUSH_ENABLED_KEY, new Response("1")), cache.delete(PUSH_DISABLED_KEY)]);
			return;
		}
		await Promise.all([
			cache.put(PUSH_DISABLED_KEY, new Response("1")),
			cache.delete(PUSH_ENABLED_KEY),
			cache.delete(PUSH_ROTATION_PENDING_KEY),
		]);
	} catch (error) {
		console.warn("Failed to persist push notification preference:", error);
	}
}

async function readIntentGeneration(): Promise<number> {
	try {
		const response = await (await getStateCache()).match(PUSH_INTENT_GENERATION_KEY);
		if (!response) return 0;
		const generation = Number.parseInt(await response.text(), 10);
		return Number.isSafeInteger(generation) && generation >= 0 ? generation : 0;
	} catch {
		return 0;
	}
}

async function advanceIntentGeneration(): Promise<number> {
	const generation = (await readIntentGeneration()) + 1;
	await (await getStateCache()).put(PUSH_INTENT_GENERATION_KEY, new Response(String(generation)));
	intentGeneration = generation;
	return generation;
}

async function readPushEnabledIntent(): Promise<boolean | null> {
	let cacheEnabled = false;
	try {
		const cache = await getStateCache();
		// The worker cannot update localStorage. Its durable revocation marker
		// must therefore win over a stale page-side enabled value.
		if (await cache.match(PUSH_DISABLED_KEY)) return false;
		cacheEnabled = Boolean(await cache.match(PUSH_ENABLED_KEY));
	} catch {
		// Fall through to localStorage when Cache Storage is unavailable.
	}
	try {
		const stored = localStorage.getItem(PUSH_INTENT_KEY);
		if (stored === "0") return false;
		if (stored === "1") return true;
	} catch {
		// The worker-visible cache remains authoritative.
	}
	return cacheEnabled ? true : null;
}

async function readRotationPending(): Promise<RotationPending | null> {
	let response: Response | undefined;
	try {
		response = await (await getStateCache()).match(PUSH_ROTATION_PENDING_KEY);
	} catch {
		return null;
	}
	if (!response) return null;

	const text = (await response.text()).trim();
	if (!(text && text !== "1" && text !== "true")) return {};
	try {
		const value: unknown = JSON.parse(text);
		if (typeof value === "object" && value !== null) {
			const marker = value as Record<string, unknown>;
			const replaces = marker.replaces ?? marker.old_endpoint ?? marker.endpoint;
			return {
				...(typeof replaces === "string" ? { replaces } : {}),
				...(marker.revive === true ? { revive: true } : {}),
			};
		}
	} catch {
		// A plain response body is also accepted as the replaced endpoint.
	}
	return { replaces: text };
}

async function setRotationPending(replaces?: string, revive = false): Promise<void> {
	const cache = await getStateCache();
	await cache.put(PUSH_ROTATION_PENDING_KEY, new Response(JSON.stringify({ replaces, revive })));
}

async function consumeRotationPending(): Promise<void> {
	try {
		await (await getStateCache()).delete(PUSH_ROTATION_PENDING_KEY);
	} catch {
		// Re-registration is idempotent; a stale marker only retries it later.
	}
}

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

class PushRegistrationError extends Error {
	constructor(readonly status: number) {
		super(`Server rejected subscription (${status})`);
		this.name = "PushRegistrationError";
	}
}

/** Post a subscription to the server, optionally replacing a rotated endpoint. */
async function registerWithServer(subscription: PushSubscription, replaces?: string, revive = false): Promise<void> {
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
			revive,
		}),
	});

	if (!response.ok) {
		throw new PushRegistrationError(response.status);
	}
}

async function disableRevokedSubscription(subscription: PushSubscription | null): Promise<void> {
	await setPushEnabledIntent(false);
	queuedPresence = null;
	lastPresenceState = "";
	currentSubscription = null;
	await subscription?.unsubscribe().catch(() => undefined);
}

async function registerAutomatically(
	subscription: PushSubscription,
	replaces?: string,
	revive = false,
): Promise<boolean> {
	try {
		await registerWithServer(subscription, replaces, revive);
		return true;
	} catch (error) {
		if (error instanceof PushRegistrationError && error.status === 410) {
			await disableRevokedSubscription(subscription);
			return false;
		}
		throw error;
	}
}

/**
 * Subscribe to push notifications.
 * Requests permission if needed, creates subscription, and registers with server.
 */
async function subscribeToPushInner(): Promise<PushResult> {
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

		await advanceIntentGeneration();
		currentSubscription = subscription;
		await setPushEnabledIntent(true);
		const pending = await readRotationPending();
		await setRotationPending(pending?.replaces, true);
		await registerWithServer(subscription, pending?.replaces, true);
		await consumeRotationPending();
		reportPresence();
		return { success: true };
	} catch (e) {
		console.error("Failed to subscribe to push:", e);
		return { success: false, error: (e as Error).message };
	}
}

export function subscribeToPush(): Promise<PushResult> {
	return runPushStateOperation(subscribeToPushInner);
}

/**
 * Unsubscribe from push notifications.
 */
async function unsubscribeFromPushInner(): Promise<PushResult> {
	try {
		// An explicit disable must prevent startup recovery even if the local
		// subscription has already disappeared.
		await advanceIntentGeneration();
	} catch (e) {
		console.warn("Failed to advance push notification intent generation:", e);
	}
	await setPushEnabledIntent(false);

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

export function unsubscribeFromPush(): Promise<PushResult> {
	return runPushStateOperation(unsubscribeFromPushInner);
}

/**
 * Re-register the local subscription if the server no longer knows about it.
 *
 * Subscriptions are dropped server-side when a push returns 410/404, and the
 * browser can hand back a subscription signed with a VAPID key the server has
 * since rotated. Either way the browser still reports itself as subscribed
 * while no push can ever arrive, so reconcile both sides on load.
 */
async function reconcileSubscription(subscription: PushSubscription, pending: RotationPending | null): Promise<void> {
	const key = await fetchVapidKey();
	if (!key) return;

	// A VAPID rotation invalidates the existing subscription outright.
	const currentKey = encodeKey(subscription.options?.applicationServerKey ?? null);
	if (currentKey && currentKey !== key) {
		const staleEndpoint = subscription.endpoint;
		const replaces = pending?.replaces ?? staleEndpoint;
		await subscription.unsubscribe().catch(() => undefined);
		const registration = await navigator.serviceWorker.ready;
		const fresh = await registration.pushManager.subscribe({
			userVisibleOnly: true,
			applicationServerKey: urlBase64ToUint8Array(key).buffer as ArrayBuffer,
		});
		currentSubscription = fresh;
		await setPushEnabledIntent(true);
		const revive = pending?.revive === true;
		await setRotationPending(replaces, revive);
		if (!(await registerAutomatically(fresh, replaces, revive))) return;
		await consumeRotationPending();
		return;
	}

	await setPushEnabledIntent(true);
	if (pending) {
		if (!(await registerAutomatically(subscription, pending.replaces, pending.revive === true))) return;
		await consumeRotationPending();
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
async function initPushStateInner(): Promise<void> {
	try {
		const [enabledIntent, pending, generation] = await Promise.all([
			readPushEnabledIntent(),
			readRotationPending(),
			readIntentGeneration(),
		]);
		intentGeneration = generation;
		let subscription = await getCurrentSubscription();
		if (enabledIntent === false) {
			if (subscription) {
				await subscription.unsubscribe().catch(() => undefined);
				currentSubscription = null;
			}
			if (pending) await consumeRotationPending();
			return;
		}

		// Only restore a missing browser subscription when saved intent (or a
		// rotation marker from an existing subscription) proves push was enabled,
		// and the browser still grants permission.
		if (!subscription && (enabledIntent === true || pending !== null) && getPermissionState() === "granted") {
			const key = await fetchVapidKey();
			if (!key) return;
			const registration = await navigator.serviceWorker.ready;
			subscription = await registration.pushManager.subscribe({
				userVisibleOnly: true,
				applicationServerKey: urlBase64ToUint8Array(key).buffer as ArrayBuffer,
			});
			currentSubscription = subscription;
			await setPushEnabledIntent(true);
			await setRotationPending(pending?.replaces, pending?.revive === true);
			if (!(await registerAutomatically(subscription, pending?.replaces, pending?.revive === true))) return;
			await consumeRotationPending();
			return;
		}

		if (subscription) await reconcileSubscription(subscription, pending);
	} catch (e) {
		console.warn("Failed to reconcile push subscription:", e);
	}
}

export function initPushState(): Promise<void> {
	return runPushStateOperation(initPushStateInner);
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
async function removeSubscriptionInner(endpoint: string): Promise<PushResult> {
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
			await advanceIntentGeneration();
			await setPushEnabledIntent(false);
		}

		return { success: true };
	} catch (e) {
		console.error("Failed to remove subscription:", e);
		return { success: false, error: (e as Error).message };
	}
}

export function removeSubscription(endpoint: string): Promise<PushResult> {
	return runPushStateOperation(() => removeSubscriptionInner(endpoint));
}

/**
 * Send a test notification to every subscribed device.
 *
 * Returns how many devices the push service accepted it for.
 */
interface TestNotificationResult extends PushResult {
	sent?: number;
	targeted?: number;
	failed?: number;
	timedOut?: number;
	expired?: number;
}

export async function sendTestNotification(): Promise<TestNotificationResult> {
	try {
		const response = await fetch("/api/push/test", { method: "POST" });
		if (!response.ok) {
			return {
				success: false,
				error: response.status === 501 ? "Push notifications are not enabled on the server" : "Failed to send",
			};
		}
		const data = (await response.json()) as {
			sent: number;
			targeted: number;
			failed: number;
			timed_out: number;
			expired: number;
		};
		return {
			success: data.failed === 0 && data.timed_out === 0,
			sent: data.sent,
			targeted: data.targeted,
			failed: data.failed,
			timedOut: data.timed_out,
			expired: data.expired,
			error:
				data.failed > 0 || data.timed_out > 0
					? `${data.failed + data.timed_out} device${data.failed + data.timed_out === 1 ? "" : "s"} failed delivery`
					: undefined,
		};
	} catch (e) {
		return { success: false, error: (e as Error).message };
	}
}

// ── Foreground presence ─────────────────────────────────────────────────────

interface PresencePayload {
	endpoint: string;
	session_key: string | null;
	visible: boolean;
	client_id: string;
	sequence: number;
	intent_generation: number;
}

const PRESENCE_CLIENT_KEY = "moltis-presence-client-id";
const PRESENCE_SEQUENCE_KEY = "moltis-presence-sequence";
const presenceClientId = (() => {
	const fallback =
		globalThis.crypto?.randomUUID?.() ?? `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
	try {
		const prefix = `${PRESENCE_CLIENT_KEY}:`;
		if (window.name.startsWith(prefix)) return window.name.slice(prefix.length);
		window.name = `${prefix}${fallback}`;
	} catch {
		// The in-memory id still separates this page when window.name is blocked.
	}
	return fallback;
})();
let presenceSequence = (() => {
	try {
		return Number.parseInt(sessionStorage.getItem(PRESENCE_SEQUENCE_KEY) || "0", 10) || 0;
	} catch {
		return 0;
	}
})();
let lastPresenceState = "";
let queuedPresence: PresencePayload | null = null;
let presenceRequestInFlight = false;

/** Return the active session only when its exact chat route is displayed. */
function displayedActiveSession(): string | null {
	const sessionKey = activeSessionKey.value;
	const expectedPath = sessionPath(sessionKey);
	return window.location.pathname === expectedPath ? sessionKey : null;
}

function presencePayload(): PresencePayload | null {
	if (!currentSubscription) return null;
	const sessionKey = displayedActiveSession();
	const visible = sessionKey !== null && document.visibilityState === "visible" && document.hasFocus();
	return {
		endpoint: currentSubscription.endpoint,
		session_key: visible ? sessionKey : null,
		visible,
		client_id: presenceClientId,
		sequence: 0,
		intent_generation: intentGeneration,
	};
}

function presenceState(payload: PresencePayload): string {
	return JSON.stringify({
		endpoint: payload.endpoint,
		session_key: payload.session_key,
		visible: payload.visible,
		intent_generation: payload.intent_generation,
	});
}

async function matchingBrowserSubscription(payload: PresencePayload): Promise<PushSubscription | null> {
	const registration = await navigator.serviceWorker.ready;
	const [generation, subscription] = await Promise.all([
		readIntentGeneration(),
		registration.pushManager.getSubscription(),
	]);
	if (generation === payload.intent_generation && subscription?.endpoint === payload.endpoint) return subscription;

	// Another tab changed the shared browser subscription while this request was
	// in flight. Adopt that state before ignoring the stale response so future
	// presence heartbeats use the live endpoint and generation without a reload.
	intentGeneration = generation;
	currentSubscription = subscription;
	if (subscription) reportPresence(true);
	return null;
}

async function disableRevokedPresence(payload: PresencePayload): Promise<void> {
	const subscription = await matchingBrowserSubscription(payload);
	if (!subscription) return;

	await disableRevokedSubscription(subscription);
}

async function sendPresence(payload: PresencePayload): Promise<void> {
	try {
		const response = await fetch("/api/push/presence", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(payload),
			keepalive: true,
		});
		if (response.ok) return;

		if (payload.sequence === presenceSequence) lastPresenceState = "";
		if (response.status === 410) {
			void runPushStateOperation(() => disableRevokedPresence(payload));
		} else if (response.status === 404) {
			void recoverUnknownSubscription(payload);
		}
	} catch {
		// Presence is an optimisation; retry the current state on the next event.
		if (payload.sequence === presenceSequence) lastPresenceState = "";
	}
}

async function drainPresenceQueue(): Promise<void> {
	if (presenceRequestInFlight || !queuedPresence) return;
	presenceRequestInFlight = true;
	const payload = queuedPresence;
	queuedPresence = null;
	await sendPresence(payload);
	presenceRequestInFlight = false;
	if (queuedPresence) void drainPresenceQueue();
}

/**
 * Tell the server which session this device is looking at, if any.
 *
 * The server skips push delivery to an endpoint that reports itself visible on
 * the session that just produced a response — that's what stops your phone from
 * buzzing for a message you are watching stream in on that same phone.
 */
export function reportPresence(force = false): void {
	const payload = presencePayload();
	if (!payload) return;
	const state = presenceState(payload);
	if (!force && state === lastPresenceState) return;
	payload.sequence = nextPresenceSequence();
	lastPresenceState = state;
	queuedPresence = payload;
	void drainPresenceQueue();
}

function nextPresenceSequence(): number {
	presenceSequence += 1;
	try {
		sessionStorage.setItem(PRESENCE_SEQUENCE_KEY, String(presenceSequence));
	} catch {
		// The in-memory sequence remains ordered for this page lifetime.
	}
	return presenceSequence;
}

/** Guards against several concurrent recovery attempts. */
let recovering = false;
/** When recovery last ran, to bound retries if the server keeps rejecting. */
let lastRecoveryAt = 0;
const RECOVERY_COOLDOWN_MS = 60_000;

/**
 * Replace a subscription the server has forgotten.
 *
 * Reached when presence reports 404. Left alone, the browser would keep a
 * subscription that receives nothing while the server has no record to push to.
 *
 * Recovery re-reports presence on success, so a server that answers 404 even
 * after accepting the registration would otherwise drive an endless
 * register/report loop. The cooldown bounds that to one attempt a minute.
 */
async function recoverUnknownSubscription(payload: PresencePayload): Promise<void> {
	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: recovery validates intent, generation, endpoint, key, and registration atomically.
	await runPushStateOperation(async () => {
		if (recovering) return;
		if ((await readPushEnabledIntent()) === false) return;
		if (Date.now() - lastRecoveryAt < RECOVERY_COOLDOWN_MS) return;
		const subscription = await matchingBrowserSubscription(payload);
		if (!subscription) return;
		const registration = await navigator.serviceWorker.ready;

		recovering = true;
		lastRecoveryAt = Date.now();
		try {
			const key = await fetchVapidKey();
			if (!key) return;
			const pending = await readRotationPending();
			const replaces = pending?.replaces ?? subscription.endpoint;
			const revive = pending?.revive === true;
			await subscription.unsubscribe().catch(() => undefined);
			const fresh = await registration.pushManager.subscribe({
				userVisibleOnly: true,
				applicationServerKey: urlBase64ToUint8Array(key).buffer as ArrayBuffer,
			});
			currentSubscription = fresh;
			await setPushEnabledIntent(true);
			await setRotationPending(replaces, revive);
			if (!(await registerAutomatically(fresh, replaces, revive))) return;
			await consumeRotationPending();
			reportPresence(true);
		} catch (e) {
			console.warn("Failed to re-register push subscription:", e);
		} finally {
			recovering = false;
		}
	});
}

/** Ask the service worker to clear notifications for the session in view. */
function clearNotificationsForActiveSession(): void {
	const sessionKey = displayedActiveSession();
	if (!sessionKey || document.visibilityState !== "visible" || !document.hasFocus()) return;
	navigator.serviceWorker?.controller?.postMessage({
		type: "CLEAR_NOTIFICATIONS",
		sessionKey,
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
		clearNotificationsForActiveSession();
	};
	const onPageHide = (): void => {
		const payload = presencePayload();
		if (!payload) return;
		payload.session_key = null;
		payload.visible = false;
		payload.sequence = nextPresenceSequence();
		lastPresenceState = presenceState(payload);
		queuedPresence = null;
		// Do not leave the final hidden update behind an in-flight heartbeat.
		// Sequence ordering lets the backend reject an older tab response if it
		// arrives after this keepalive request.
		void sendPresence(payload);
	};

	document.addEventListener("visibilitychange", onForeground);
	window.addEventListener("focus", onForeground);
	window.addEventListener("blur", () => reportPresence());
	window.addEventListener("pagehide", onPageHide);
	activeSessionKey.subscribe(onForeground);
	navigator.serviceWorker?.addEventListener("message", (event: MessageEvent) => {
		if (event.data?.type !== "push-subscription-changed") return;
		void runPushStateOperation(async () => {
			await getCurrentSubscription();
			reportPresence(true);
		});
	});

	// Router navigation does not emit an event for history.pushState. This cheap
	// poll clears presence promptly when a focused tab leaves the active chat.
	let observedPath = window.location.pathname;
	window.setInterval(() => {
		if (window.location.pathname === observedPath) return;
		observedPath = window.location.pathname;
		onForeground();
	}, 1_000);
	// Refresh well before the backend's 120-second presence TTL.
	window.setInterval(() => reportPresence(true), 45_000);

	// Reconcile before the first report: this is the app-startup path, so it is
	// where a subscription the server has forgotten gets re-registered rather
	// than silently receiving nothing until someone opens Settings.
	initPushState()
		.then(() => onForeground())
		.catch(() => {
			// No subscription: presence stays a no-op, which is correct.
		});
}
