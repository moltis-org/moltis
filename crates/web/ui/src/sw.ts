// Service Worker for moltis PWA
// Handles caching for offline support and push notifications

/// <reference lib="webworker" />

// Service Worker global: `self` is Window in DOM lib but ServiceWorkerGlobalScope at runtime.
// The double cast is unavoidable when both DOM and WebWorker types coexist in tsconfig.
const sw = self as unknown as ServiceWorkerGlobalScope;

declare const __MOLTIS_SW_VERSION__: string;

// The build hashes this worker source. A waiting worker therefore fills its own
// cache instead of mutating entries still used by the active worker.
const CACHE_NAME = `moltis-shell-${__MOLTIS_SW_VERSION__}`;
const CACHE_PREFIX = "moltis-";
const OFFLINE_URL = "/offline.html";

/**
 * Small persistent flag store, kept out of the shell cache lifecycle. The
 * service worker has no localStorage, and this state must survive app closure.
 */
const STATE_CACHE = "moltis-state";
const STATE_KEYS = {
	installed: "/__moltis__/installed",
	pushEnabled: "/__moltis__/push-enabled",
	pushDisabled: "/__moltis__/push-disabled",
	pushRotationPending: "/__moltis__/push-rotation-pending",
	notificationOrder: "/__moltis__/push-notification-order",
} as const;
const PUSH_STATE_LOCK = "moltis-push-state";

const REQUIRED_ASSETS: string[] = [
	OFFLINE_URL,
	"/manifest.json",
	"/assets/css/base.css",
	"/assets/css/layout.css",
	"/assets/css/chat.css",
	"/assets/css/components.css",
	"/assets/css/mobile.css",
	"/assets/icons/icon-192.png",
	"/assets/icons/icon-512.png",
	"/assets/icons/icon-72.png",
	"/assets/icons/apple-touch-icon.png",
];

// Generated assets are absent in source-only builds and must not block install.
const OPTIONAL_ASSETS: string[] = [
	"/assets/style.css",
	"/assets/dist/main.js",
	"/assets/dist/login.js",
	"/assets/dist/onboarding.js",
];

async function cacheAsset(cache: Cache, url: string): Promise<void> {
	const response = await fetch(url, { cache: "reload" });
	if (!response.ok) {
		throw new Error(`precache ${url}: ${response.status}`);
	}
	await cache.put(url, response);
}

async function precache(): Promise<void> {
	const cache = await caches.open(CACHE_NAME);
	await Promise.all(REQUIRED_ASSETS.map((url) => cacheAsset(cache, url)));
	await Promise.allSettled(OPTIONAL_ASSETS.map((url) => cacheAsset(cache, url)));
}

// Install event - cache static assets.
// Deliberately does NOT call skipWaiting(): the worker activates normally only
// after every page using the old worker closes, avoiding old-page/new-chunk
// incompatibility during an update.
sw.addEventListener("install", (event: ExtendableEvent) => {
	event.waitUntil(precache());
});

// Activate event - clean up old caches
sw.addEventListener("activate", (event: ExtendableEvent) => {
	event.waitUntil(
		(async () => {
			const cacheNames = await caches.keys();
			await Promise.all(
				cacheNames
					.filter((name) => name.startsWith(CACHE_PREFIX) && name !== CACHE_NAME && name !== STATE_CACHE)
					.map((name) => caches.delete(name)),
			);
			await sw.clients.claim();
		})(),
	);
});

/**
 * Serve from cache, refreshing the entry in the background.
 *
 * Response bodies are streams, and cloning one tees it: if only a single branch
 * is ever read, the other buffers the whole body indefinitely. Doing that once
 * per asset per page load exhausts the renderer and crashes the tab — so clone
 * only on the path that genuinely needs two readers.
 */
async function staleWhileRevalidate(request: Request, event: FetchEvent): Promise<Response> {
	const cache = await caches.open(CACHE_NAME);
	const cached = await cache.match(request);

	if (cached) {
		// The response is only going into the cache, so hand it over whole
		// rather than cloning it — one reader, no buffered branch.
		event.waitUntil(
			fetch(request)
				.then((response) => (response.ok ? cache.put(request, response) : undefined))
				.catch(() => undefined),
		);
		return cached;
	}

	try {
		const response = await fetch(request);
		// Two readers here — the cache and the page — so the clone is consumed.
		if (response.ok) {
			event.waitUntil(cache.put(request, response.clone()));
		}
		return response;
	} catch {
		return new Response("", { status: 504, statusText: "Offline" });
	}
}

/** Network-first with a cache fallback, ending at the offline page. */
async function networkFirstNavigation(request: Request, event: FetchEvent): Promise<Response> {
	const cache = await caches.open(CACHE_NAME);
	try {
		const response = await fetch(request);
		// Both branches of the clone are read: the cache takes one, the page the
		// other. Leaving one unread would buffer the whole document in memory.
		if (response.ok) {
			event.waitUntil(cache.put(request, response.clone()));
		}
		return response;
	} catch {
		const cached = await cache.match(request);
		if (cached) return cached;
		const root = await cache.match("/");
		if (root) return root;
		const offline = await cache.match(OFFLINE_URL);
		if (offline) return offline;
		return new Response("Offline", {
			status: 503,
			statusText: "Offline",
			headers: { "Content-Type": "text/plain" },
		});
	}
}

// Fetch event - network first for API, cache first for assets
sw.addEventListener("fetch", (event: FetchEvent) => {
	const url = new URL(event.request.url);

	// Only handle same-origin GETs. Cross-origin and mutating requests go
	// straight to the network so auth/CORS behaviour is never altered here.
	if (event.request.method !== "GET" || url.origin !== sw.location.origin) {
		return;
	}

	// Skip WebSocket requests
	if (url.protocol === "ws:" || url.protocol === "wss:") {
		return;
	}

	// API requests - network only (no caching)
	if (url.pathname.startsWith("/api/") || url.pathname.startsWith("/ws/")) {
		return;
	}

	// Static assets - cache first, then network
	// Versioned assets are immutable and already handled by the HTTP cache. Not
	// duplicating them here prevents old build hashes accumulating indefinitely.
	if (url.pathname.startsWith("/assets/v/")) {
		return;
	}
	if (url.pathname.startsWith("/assets/") || url.pathname === "/manifest.json") {
		event.respondWith(staleWhileRevalidate(event.request, event));
		return;
	}

	// HTML pages - network first, fallback to cache
	if (event.request.mode === "navigate") {
		event.respondWith(networkFirstNavigation(event.request, event));
	}
});

// ── Push notifications ──────────────────────────────────────────────────────

/** Payload sent by the server in the push message body. */
interface PushData {
	title?: string;
	body?: string;
	url?: string;
	sessionKey?: string;
	/** Unique per notification — used to build a stable, non-colliding tag. */
	notificationId?: string;
	/** Legacy server-side hint; the worker derives the badge from notifications. */
	badgeCount?: number;
	/** Suppress sound/vibration for low-priority updates. */
	silent?: boolean;
	/** Keep the notification on screen until the user acts on it. */
	requireInteraction?: boolean;
	/** ISO 8601 timestamp of the underlying event. */
	timestamp?: string;
	/** Monotonic delivery order within a session. */
	order?: number;
}

/** Options accepted by showNotification beyond the DOM lib's NotificationOptions. */
interface ExtendedNotificationOptions extends NotificationOptions {
	actions?: Array<{ action: string; title: string }>;
	vibrate?: number[];
	renotify?: boolean;
	timestamp?: number;
}

interface StoredNotificationData {
	url?: string;
	sessionKey?: string;
	notificationId?: string;
	count?: number;
	order?: number;
}

interface RotationPending {
	replaces?: string;
	revive?: boolean;
}

interface OrderedPushState {
	cache: Cache;
	orders: Record<string, number>;
	sessionKey: string;
	order: number;
	stale: boolean;
}

/** Group notifications per session so one chat never floods the shade. */
function notificationTag(data: PushData): string {
	return data.sessionKey ? `moltis:session:${data.sessionKey}` : "moltis:general";
}

/**
 * Build the notification body, folding in any notification already on screen
 * for the same session.
 *
 * A per-session tag means a new message replaces the previous one rather than
 * stacking. To avoid losing that earlier message entirely, the replacement
 * summarises how many are now unread and keeps the newest text visible.
 */
function storedNotificationData(notification: Notification): StoredNotificationData {
	return (notification.data as StoredNotificationData | null) ?? {};
}

function storedNotificationCount(notification: Notification): number {
	const count = storedNotificationData(notification).count;
	return typeof count === "number" && Number.isFinite(count) ? Math.max(0, count) : 1;
}

function notificationMatchesSession(notification: Notification, sessionKey: string): boolean {
	return (
		notification.tag === `moltis:session:${sessionKey}` ||
		storedNotificationData(notification).sessionKey === sessionKey
	);
}

function buildBody(data: PushData, existing: Notification[]): { body: string; count: number } {
	const body = data.body || "New response available";
	const previousCount = existing.reduce((sum, notification) => sum + storedNotificationCount(notification), 0);
	const count = previousCount + 1;
	if (count <= 1) {
		return { body, count };
	}
	return { body: `${body}\n… and ${count - 1} earlier message${count - 1 === 1 ? "" : "s"}`, count };
}

function newNotificationId(): string {
	return typeof crypto.randomUUID === "function" ? crypto.randomUUID() : `${Date.now()}-${Math.random()}`;
}

/** Has this app ever run as an installed PWA on this device? */
async function isInstalled(): Promise<boolean> {
	try {
		const cache = await caches.open(STATE_CACHE);
		return Boolean(await cache.match(STATE_KEYS.installed));
	} catch {
		return false;
	}
}

async function setStateFlag(key: (typeof STATE_KEYS)[keyof typeof STATE_KEYS], enabled: boolean): Promise<void> {
	const cache = await caches.open(STATE_CACHE);
	if (enabled) {
		await cache.put(key, new Response("1"));
		return;
	}
	await cache.delete(key);
}

async function readRotationPending(cache: Cache): Promise<RotationPending | null> {
	const response = await cache.match(STATE_KEYS.pushRotationPending);
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
		// Legacy markers may contain only the replaced endpoint.
	}
	return { replaces: text };
}

async function setRotationPending(replaces: string | undefined, revive = false): Promise<void> {
	const cache = await caches.open(STATE_CACHE);
	await cache.put(STATE_KEYS.pushRotationPending, new Response(JSON.stringify({ replaces, revive })));
}

async function preserveRotationPending(cache: Cache, oldEndpoint: string | undefined): Promise<RotationPending> {
	const pending = await readRotationPending(cache);
	const replaces = pending?.replaces ?? oldEndpoint;
	const revive = pending?.revive === true;
	await setRotationPending(replaces, revive);
	return { replaces, revive };
}

async function markPushDisabled(): Promise<void> {
	const cache = await caches.open(STATE_CACHE);
	await Promise.all([
		cache.put(STATE_KEYS.pushDisabled, new Response("1")),
		cache.delete(STATE_KEYS.pushEnabled),
		cache.delete(STATE_KEYS.pushRotationPending),
	]);
}

/** Record a positive installed report; ordinary tabs cannot disprove it. */
async function markInstalled(): Promise<void> {
	try {
		await setStateFlag(STATE_KEYS.installed, true);
	} catch {
		// Without the flag the badge is simply left to open pages.
	}
}

/**
 * Reflect the unread count on the installed app icon.
 *
 * Two rules make this safe to call from a service worker:
 *
 * 1. The platform badge promise is never awaited. Where the platform has no
 *    badge target it may never settle, which must not wedge an event.
 * 2. **Only when the app is installed.** A badge is meaningless in a plain
 *    browser tab, and merely *invoking* the API in a headless environment wedges
 *    the worker, so the flag keeps us away from it entirely unless there is a
 *    real app icon to draw on.
 *
 * Open pages are told the count too, so a running app updates immediately
 * without waiting on the platform call.
 */
async function updateBadge(count: number): Promise<void> {
	const value = count ?? 0;
	const [clients, installed] = await Promise.all([
		sw.clients.matchAll({ type: "window", includeUncontrolled: true }).catch(() => []),
		isInstalled(),
	]);

	for (const client of clients) {
		client.postMessage({ type: "badge-count", count: value });
	}

	if (!installed) return;
	const nav = navigator as Navigator & {
		setAppBadge?: (count?: number) => Promise<void>;
		clearAppBadge?: () => Promise<void>;
	};
	try {
		const pending = value > 0 ? nav.setAppBadge?.(value) : nav.clearAppBadge?.();
		void pending?.catch(() => undefined);
	} catch {
		// Badging is unsupported here; the open-page path still applies.
	}
}

async function updateBadgeFromNotifications(
	excludedIds: ReadonlySet<string> = new Set(),
	excludedTags: ReadonlySet<string> = new Set(),
	excludedSessions: ReadonlySet<string> = new Set(),
): Promise<void> {
	const notifications = await sw.registration.getNotifications();
	const count = notifications.reduce((sum, notification) => {
		const data = storedNotificationData(notification);
		return (data.notificationId && excludedIds.has(data.notificationId)) ||
			excludedTags.has(notification.tag) ||
			(data.sessionKey && excludedSessions.has(data.sessionKey))
			? sum
			: sum + storedNotificationCount(notification);
	}, 0);
	await updateBadge(count);
}

function addClosedNotificationExclusions(
	notification: Notification,
	ids: Set<string>,
	tags: Set<string>,
	sessions: Set<string>,
): void {
	const data = storedNotificationData(notification);
	if (data.notificationId) {
		ids.add(data.notificationId);
		return;
	}
	// Legacy notifications have no stable ID, so tag/session are the only way
	// to exclude a close that has not disappeared from getNotifications() yet.
	if (notification.tag) tags.add(notification.tag);
	if (data.sessionKey) sessions.add(data.sessionKey);
}

async function readNotificationOrders(cache: Cache): Promise<Record<string, number>> {
	const response = await cache.match(STATE_KEYS.notificationOrder);
	if (!response) return Object.create(null) as Record<string, number>;
	try {
		const value: unknown = await response.json();
		if (typeof value !== "object" || value === null) return Object.create(null) as Record<string, number>;
		return Object.fromEntries(
			Object.entries(value).filter(
				(entry): entry is [string, number] => typeof entry[1] === "number" && Number.isFinite(entry[1]),
			),
		);
	} catch {
		return Object.create(null) as Record<string, number>;
	}
}

async function writeNotificationOrders(cache: Cache, orders: Record<string, number>): Promise<void> {
	await cache.put(STATE_KEYS.notificationOrder, new Response(JSON.stringify(orders)));
}

async function orderedPushState(data: PushData): Promise<OrderedPushState | null> {
	if (!(data.sessionKey && typeof data.order === "number" && Number.isFinite(data.order))) return null;
	const cache = await caches.open(STATE_CACHE);
	const orders = await readNotificationOrders(cache);
	return {
		cache,
		orders,
		sessionKey: data.sessionKey,
		order: data.order,
		stale: (orders[data.sessionKey] ?? Number.NEGATIVE_INFINITY) >= data.order,
	};
}

/**
 * True when a visible client already has this session open.
 *
 * The server suppresses pushes for a device that reported itself as actively
 * viewing, but presence reports can race with the send. This is the client-side
 * backstop for that race.
 */
async function isSessionVisible(sessionKey: string | undefined): Promise<boolean> {
	if (!sessionKey) return false;
	const target = `/chats/${sessionKey.replace(/:/g, "/")}`;
	const clients = await sw.clients.matchAll({ type: "window", includeUncontrolled: true });
	return clients.some((client) => {
		const windowClient = client as WindowClient;
		if (windowClient.visibilityState !== "visible" || !windowClient.focused) return false;

		let pathname: string;
		try {
			pathname = new URL(client.url).pathname;
		} catch {
			return false;
		}
		// Compare the whole path, not a substring: `/chats/main-2` contains
		// `/chats/main`, so a substring test would treat a different chat as
		// on-screen and silence a notification the user needed.
		return pathname === target || pathname === `${target}/`;
	});
}

let pushQueue: Promise<void> = Promise.resolve();

async function handlePush(data: PushData): Promise<void> {
	const ordered = await orderedPushState(data);
	if (ordered?.stale) return;

	const outstanding = await sw.registration.getNotifications();
	if (
		data.notificationId &&
		outstanding.some((notification) => storedNotificationData(notification).notificationId === data.notificationId)
	) {
		await updateBadgeFromNotifications();
		return;
	}

	// A visible push remains user-visible, but without sound or vibration.
	const alreadyVisible = await isSessionVisible(data.sessionKey);
	const tag = notificationTag(data);
	const sessionKey = data.sessionKey;
	const existing = sessionKey
		? outstanding.filter((notification) => notificationMatchesSession(notification, sessionKey))
		: outstanding.filter((notification) => notification.tag === tag);
	const { body, count } = buildBody(data, existing);
	const silent = alreadyVisible || data.silent === true;
	const notificationId = data.notificationId ?? newNotificationId();
	const url = data.url || "/chats";
	const options: ExtendedNotificationOptions = {
		body,
		icon: "/assets/icons/icon-192.png",
		badge: "/assets/icons/icon-72.png",
		tag,
		renotify: !silent,
		silent,
		requireInteraction: data.requireInteraction === true,
		timestamp: data.timestamp ? Date.parse(data.timestamp) || Date.now() : Date.now(),
		data: {
			url,
			sessionKey: data.sessionKey,
			notificationId,
			count,
			order: data.order,
		},
		actions: [
			{ action: "open", title: "View" },
			{ action: "dismiss", title: "Dismiss" },
		],
	};
	if (!silent) {
		options.vibrate = [100, 50, 100];
	}

	await sw.registration.showNotification(data.title || "moltis", options);
	if (ordered) {
		ordered.orders[ordered.sessionKey] = ordered.order;
		await writeNotificationOrders(ordered.cache, ordered.orders);
	}
	for (const notification of existing) {
		if (notification.tag !== tag) notification.close();
	}
	const replacedIds = new Set(
		existing
			.map((notification) => storedNotificationData(notification).notificationId)
			.filter((id) => id !== undefined),
	);
	const legacyTags = new Set(
		existing.filter((notification) => notification.tag !== tag).map((notification) => notification.tag),
	);
	await updateBadgeFromNotifications(replacedIds, legacyTags);
}

sw.addEventListener("push", (event: PushEvent) => {
	let data: PushData = {};
	try {
		data = event.data ? (event.data.json() as PushData) : {};
	} catch {
		data = { body: event.data ? event.data.text() : "New message from moltis" };
	}

	const queued = pushQueue.then(() => handlePush(data));
	pushQueue = queued.catch(() => undefined);
	event.waitUntil(queued);
});

/**
 * Re-subscribe when the browser rotates the push endpoint.
 *
 * Without this the old subscription dies silently and push stops working until
 * the user toggles it off and on again in settings.
 */
sw.addEventListener("pushsubscriptionchange", (event: Event) => {
	// The DOM lib types this event as a bare Event; at runtime it is an
	// ExtendableEvent carrying the old and new subscriptions.
	const subscriptionEvent = event as ExtendableEvent & {
		oldSubscription?: PushSubscription | null;
		newSubscription?: PushSubscription | null;
	};

	const resubscribe = async (): Promise<void> => {
		const state = await caches.open(STATE_CACHE);
		if (await state.match(STATE_KEYS.pushDisabled)) {
			await subscriptionEvent.newSubscription?.unsubscribe().catch(() => undefined);
			return;
		}
		const oldEndpoint = subscriptionEvent.oldSubscription?.endpoint;
		const pending = await preserveRotationPending(state, oldEndpoint);
		let subscription = subscriptionEvent.newSubscription ?? null;

		if (!subscription) {
			const response = await fetch("/api/push/vapid-key");
			if (!response.ok) {
				throw new Error(`VAPID key fetch failed: ${response.status}`);
			}
			const { public_key: publicKey } = (await response.json()) as { public_key: string };
			subscription = await sw.registration.pushManager.subscribe({
				userVisibleOnly: true,
				applicationServerKey: publicKey,
			});
		}

		const json = subscription.toJSON();
		const registered = await fetch("/api/push/subscribe", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({
				endpoint: subscription.endpoint,
				keys: json.keys,
				replaces: pending.replaces,
				revive: pending.revive === true,
			}),
		});

		// `fetch` resolves for 4xx/5xx too. Without this check a rejected
		// registration looks like success, and the browser is left holding an
		// endpoint the server never stored — push stays dead with nothing to
		// signal it.
		if (!registered.ok) {
			if (registered.status === 410) {
				await markPushDisabled();
				await subscription.unsubscribe().catch(() => undefined);
				return;
			}
			throw new Error(`push re-registration failed: ${registered.status}`);
		}

		await setStateFlag(STATE_KEYS.pushRotationPending, false).catch((error) => {
			console.warn("failed to clear push rotation state:", error);
		});
		const clients = await sw.clients.matchAll({ type: "window", includeUncontrolled: true });
		for (const client of clients) client.postMessage({ type: "push-subscription-changed" });
	};

	// A failed re-subscribe must not reject the event handler. The worker cannot
	// usefully retry either — it may not run again before the app is next opened
	// — so recovery is left to initPushState(), which reconciles the browser's
	// subscription against the server on every page load.
	subscriptionEvent.waitUntil(
		sw.navigator.locks.request(PUSH_STATE_LOCK, resubscribe).catch((error) => {
			console.warn("push re-subscribe failed, will reconcile on next load:", error);
		}),
	);
});

function safeNotificationUrl(value: unknown): URL {
	try {
		const url = new URL(typeof value === "string" ? value : "/chats", sw.location.origin);
		return url.origin === sw.location.origin ? url : new URL("/chats", sw.location.origin);
	} catch {
		return new URL("/chats", sw.location.origin);
	}
}

async function routeClient(client: WindowClient, targetUrl: URL): Promise<boolean> {
	// A backgrounded mobile client may not process messages until focused.
	await client.focus();
	const channel = new MessageChannel();
	const handled = new Promise<boolean>((resolve) => {
		const timer = setTimeout(() => resolve(false), 500);
		channel.port1.onmessage = (event: MessageEvent) => {
			if (event.data?.handled !== true) return;
			clearTimeout(timer);
			resolve(true);
		};
	});
	client.postMessage({ type: "notification-click", url: `${targetUrl.pathname}${targetUrl.search}${targetUrl.hash}` }, [
		channel.port2,
	]);
	if (!(await handled)) return false;
	return true;
}

async function openNotificationUrl(targetUrl: URL): Promise<void> {
	const clientList = await sw.clients.matchAll({ type: "window", includeUncontrolled: true });
	const sameOrigin = clientList.filter((client) => {
		try {
			return new URL(client.url).origin === sw.location.origin;
		} catch {
			return false;
		}
	}) as WindowClient[];
	const exact = sameOrigin.find((client) => client.url === targetUrl.href);
	if (exact) {
		await exact.focus();
		return;
	}

	const target = sameOrigin.find((client) => client.focused) ?? sameOrigin[0];
	if (target) {
		try {
			if (await routeClient(target, targetUrl)) return;
			const navigated = await target.navigate(targetUrl.href);
			await (navigated ?? target).focus();
			return;
		} catch {
			// The client may have closed between matching and navigation.
		}
	}
	await sw.clients.openWindow(targetUrl.href);
}

// Notification click event
sw.addEventListener("notificationclick", (event: NotificationEvent) => {
	event.waitUntil(
		(async () => {
			const notificationData = storedNotificationData(event.notification);
			const excludedIds = new Set<string>();
			const excludedTags = new Set<string>();
			const excludedSessions = new Set<string>();
			addClosedNotificationExclusions(event.notification, excludedIds, excludedTags, excludedSessions);
			event.notification.close();
			await updateBadgeFromNotifications(excludedIds, excludedTags, excludedSessions);
			if (event.action === "dismiss") return;

			const targetUrl = safeNotificationUrl(notificationData.url);
			await openNotificationUrl(targetUrl);
		})(),
	);
});

// Recount after browser-shade dismissal, excluding a close still being removed.
sw.addEventListener("notificationclose", (event: NotificationEvent) => {
	const ids = new Set<string>();
	const tags = new Set<string>();
	const sessions = new Set<string>();
	addClosedNotificationExclusions(event.notification, ids, tags, sessions);
	event.waitUntil(updateBadgeFromNotifications(ids, tags, sessions));
});

// Handle messages from the main app
sw.addEventListener("message", (event: ExtendableMessageEvent) => {
	// Only an installed app has an icon to badge, and the worker cannot detect
	// display-mode itself — so the page tells it, and the answer is persisted for
	// the pushes that arrive once every page is gone.
	if (event.data?.type === "PWA_INSTALLED") {
		if (event.data.installed === true) {
			event.waitUntil(markInstalled());
		}
		return;
	}
	if (event.data?.type === "SYNC_BADGE") {
		event.waitUntil(updateBadgeFromNotifications());
		return;
	}
	// The page reports focus so the badge and any stale notifications for the
	// session being viewed are cleared as soon as it comes to the foreground.
	if (event.data?.type === "CLEAR_NOTIFICATIONS") {
		const sessionKey = event.data.sessionKey as string | undefined;
		event.waitUntil(
			(async () => {
				const outstanding = await sw.registration.getNotifications();
				const notifications = sessionKey
					? outstanding.filter((notification) => notificationMatchesSession(notification, sessionKey))
					: outstanding;
				const closedIds = new Set<string>();
				const closedTags = new Set<string>();
				const closedSessions = new Set<string>();
				for (const notification of notifications) {
					addClosedNotificationExclusions(notification, closedIds, closedTags, closedSessions);
					notification.close();
				}
				await updateBadgeFromNotifications(closedIds, closedTags, closedSessions);
			})(),
		);
	}
});
