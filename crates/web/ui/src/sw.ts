// Service Worker for moltis PWA
// Handles caching for offline support and push notifications

/// <reference lib="webworker" />

// Service Worker global: `self` is Window in DOM lib but ServiceWorkerGlobalScope at runtime.
// The double cast is unavoidable when both DOM and WebWorker types coexist in tsconfig.
const sw = self as unknown as ServiceWorkerGlobalScope;

const CACHE_NAME = "moltis-v3";
const OFFLINE_URL = "/offline.html";

/**
 * Small persistent flag store, kept out of `CACHE_NAME` so a cache version bump
 * does not wipe it. The service worker has no localStorage, and this is the only
 * state it needs to survive the app being closed.
 */
const STATE_CACHE = "moltis-state";
const INSTALLED_KEY = "/__moltis__/installed";

// Best-effort precache. Generated assets (style.css, dist bundles) may not exist
// in every build, so these are cached individually — never with `cache.addAll`,
// which is atomic and would fail the whole install on a single 404.
const STATIC_ASSETS: string[] = [
	OFFLINE_URL,
	"/manifest.json",
	"/assets/css/base.css",
	"/assets/css/layout.css",
	"/assets/css/chat.css",
	"/assets/css/components.css",
	"/assets/style.css",
	"/assets/icons/icon-192.png",
	"/assets/icons/icon-512.png",
	"/assets/icons/icon-72.png",
	"/assets/icons/apple-touch-icon.png",
];

/** Cache each asset independently so one missing file cannot fail the install. */
async function precache(): Promise<void> {
	const cache = await caches.open(CACHE_NAME);
	await Promise.allSettled(
		STATIC_ASSETS.map(async (url) => {
			const response = await fetch(url, { cache: "reload" });
			if (!response.ok) {
				throw new Error(`precache ${url}: ${response.status}`);
			}
			await cache.put(url, response);
		}),
	);
}

// Install event - cache static assets.
// Deliberately does NOT call skipWaiting(): the page decides when to activate a
// new worker (see `activateUpdate()` in pwa.ts) so an update never reloads the
// app out from under someone mid-conversation.
sw.addEventListener("install", (event: ExtendableEvent) => {
	event.waitUntil(precache());
});

// Activate event - clean up old caches
sw.addEventListener("activate", (event: ExtendableEvent) => {
	event.waitUntil(
		(async () => {
			const cacheNames = await caches.keys();
			await Promise.all(
				cacheNames.filter((name) => name !== CACHE_NAME && name !== STATE_CACHE).map((name) => caches.delete(name)),
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
	/** Server-side unread count for the app badge. */
	badgeCount?: number;
	/** Suppress sound/vibration for low-priority updates. */
	silent?: boolean;
	/** Keep the notification on screen until the user acts on it. */
	requireInteraction?: boolean;
	/** ISO 8601 timestamp of the underlying event. */
	timestamp?: string;
}

/** Options accepted by showNotification beyond the DOM lib's NotificationOptions. */
interface ExtendedNotificationOptions extends NotificationOptions {
	actions?: Array<{ action: string; title: string }>;
	vibrate?: number[];
	renotify?: boolean;
	timestamp?: number;
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
async function buildBody(data: PushData, tag: string): Promise<{ body: string; count: number }> {
	const body = data.body || "New response available";
	const existing = await sw.registration.getNotifications({ tag });
	const previousCount = existing.reduce((sum, n) => sum + ((n.data as { count?: number } | null)?.count ?? 1), 0);
	const count = previousCount + 1;
	if (count <= 1) {
		return { body, count };
	}
	return { body: `${body}\n… and ${count - 1} earlier message${count - 1 === 1 ? "" : "s"}`, count };
}

/** Has this app ever run as an installed PWA on this device? */
async function isInstalled(): Promise<boolean> {
	try {
		const cache = await caches.open(STATE_CACHE);
		return Boolean(await cache.match(INSTALLED_KEY));
	} catch {
		return false;
	}
}

/** Record whether the app is installed, so a closed app can still badge. */
async function setInstalled(installed: boolean): Promise<void> {
	try {
		const cache = await caches.open(STATE_CACHE);
		if (installed) {
			await cache.put(INSTALLED_KEY, new Response("1"));
		} else {
			await cache.delete(INSTALLED_KEY);
		}
	} catch {
		// Without the flag the badge is simply left to open pages.
	}
}

/**
 * Reflect the unread count on the installed app icon.
 *
 * Two rules make this safe to call from a service worker:
 *
 * 1. **Never awaited, never inside `waitUntil`.** Where the platform has no
 *    badge target the Badging API neither resolves nor rejects — it just hangs.
 *    Awaiting it would wedge the push handler and the notification would never
 *    be shown. Nothing here may block the caller.
 * 2. **Only when the app is installed.** A badge is meaningless in a plain
 *    browser tab, and merely *invoking* the API in a headless environment wedges
 *    the worker, so the flag keeps us away from it entirely unless there is a
 *    real app icon to draw on.
 *
 * Open pages are told the count too, so a running app updates immediately
 * without waiting on the platform call.
 */
function updateBadge(count: number | undefined): void {
	const value = count ?? 0;

	void sw.clients.matchAll({ type: "window", includeUncontrolled: true }).then((clients) => {
		for (const client of clients) {
			client.postMessage({ type: "badge-count", count: value });
		}
	});

	void isInstalled().then((installed) => {
		if (!installed) return;
		const nav = navigator as Navigator & {
			setAppBadge?: (count?: number) => Promise<void>;
			clearAppBadge?: () => Promise<void>;
		};
		try {
			// Fire and forget — see rule 1 above.
			const pending = value > 0 ? nav.setAppBadge?.(value) : nav.clearAppBadge?.();
			pending?.catch(() => undefined);
		} catch {
			// Badging is unsupported here; the open-page path still applies.
		}
	});
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
		if (windowClient.visibilityState !== "visible") return false;

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

sw.addEventListener("push", (event: PushEvent) => {
	let data: PushData = {};
	try {
		data = event.data ? (event.data.json() as PushData) : {};
	} catch {
		data = { body: event.data ? event.data.text() : "New message from moltis" };
	}

	event.waitUntil(
		(async () => {
			// `userVisibleOnly` subscriptions must show a notification for every
			// push or the browser revokes the subscription, so when the session is
			// already on screen the notification is still shown — just silently,
			// with no sound or vibration. The page clears it on focus.
			const alreadyVisible = await isSessionVisible(data.sessionKey);
			const tag = notificationTag(data);
			const { body, count } = await buildBody(data, tag);
			const url = data.url || "/chats";

			const options: ExtendedNotificationOptions = {
				body,
				icon: "/assets/icons/icon-192.png",
				badge: "/assets/icons/icon-72.png",
				tag,
				// Without renotify a same-tag notification replaces the previous
				// one silently — the exact "notifications override each other"
				// problem. Re-alert unless the user is already looking at it.
				renotify: !alreadyVisible,
				silent: alreadyVisible || data.silent === true,
				requireInteraction: data.requireInteraction === true,
				timestamp: data.timestamp ? Date.parse(data.timestamp) || Date.now() : Date.now(),
				data: {
					url,
					sessionKey: data.sessionKey,
					notificationId: data.notificationId,
					count,
				},
				actions: [
					{ action: "open", title: "View" },
					{ action: "dismiss", title: "Dismiss" },
				],
				vibrate: alreadyVisible ? [] : [100, 50, 100],
			};

			await sw.registration.showNotification(data.title || "moltis", options);
			updateBadge(data.badgeCount ?? count);
		})(),
	);
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
		const oldEndpoint = subscriptionEvent.oldSubscription?.endpoint;
		let subscription = subscriptionEvent.newSubscription ?? null;

		if (!subscription) {
			const response = await fetch("/api/push/vapid-key");
			if (!response.ok) return;
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
				replaces: oldEndpoint,
			}),
		});

		// `fetch` resolves for 4xx/5xx too. Without this check a rejected
		// registration looks like success, and the browser is left holding an
		// endpoint the server never stored — push stays dead with nothing to
		// signal it.
		if (!registered.ok) {
			throw new Error(`push re-registration failed: ${registered.status}`);
		}
	};

	// A failed re-subscribe must not reject the event handler. The worker cannot
	// usefully retry either — it may not run again before the app is next opened
	// — so recovery is left to initPushState(), which reconciles the browser's
	// subscription against the server on every page load.
	subscriptionEvent.waitUntil(
		resubscribe().catch((error) => {
			console.warn("push re-subscribe failed, will reconcile on next load:", error);
		}),
	);
});

// Notification click event
sw.addEventListener("notificationclick", (event: NotificationEvent) => {
	event.notification.close();

	if (event.action === "dismiss") {
		updateBadge(0);
		return;
	}

	const urlToOpen = (event.notification.data?.url as string) || "/chats";
	const absoluteUrl = new URL(urlToOpen, sw.location.origin).href;

	event.waitUntil(
		(async () => {
			updateBadge(0);
			const clientList = await sw.clients.matchAll({ type: "window", includeUncontrolled: true });
			const sameOrigin = clientList.filter((client) => new URL(client.url).origin === sw.location.origin);

			// Prefer a window already showing the target so the click doesn't
			// yank someone away from the page they were reading.
			const exact = sameOrigin.find((client) => client.url === absoluteUrl);
			const target = exact ?? sameOrigin.find((client) => (client as WindowClient).focused) ?? sameOrigin[0];

			if (target) {
				const windowClient = target as WindowClient;
				await windowClient.focus();
				if (!exact) {
					// postMessage lets the SPA route without a full reload; the
					// page falls back to navigate() if it isn't listening.
					windowClient.postMessage({ type: "notification-click", url: urlToOpen });
				}
				return;
			}

			await sw.clients.openWindow(urlToOpen);
		})(),
	);
});

// Clear the badge once the user dismisses the last notification.
sw.addEventListener("notificationclose", (event: NotificationEvent) => {
	event.waitUntil(
		(async () => {
			const remaining = await sw.registration.getNotifications();
			if (remaining.length === 0) {
				updateBadge(0);
			}
		})(),
	);
});

// Handle messages from the main app
sw.addEventListener("message", (event: ExtendableMessageEvent) => {
	if (event.data?.type === "SKIP_WAITING") {
		sw.skipWaiting();
		return;
	}
	// Only an installed app has an icon to badge, and the worker cannot detect
	// display-mode itself — so the page tells it, and the answer is persisted for
	// the pushes that arrive once every page is gone.
	if (event.data?.type === "PWA_INSTALLED") {
		event.waitUntil(setInstalled(event.data.installed === true));
		return;
	}
	// The page reports focus so the badge and any stale notifications for the
	// session being viewed are cleared as soon as it comes to the foreground.
	if (event.data?.type === "CLEAR_NOTIFICATIONS") {
		const sessionKey = event.data.sessionKey as string | undefined;
		event.waitUntil(
			(async () => {
				const tag = sessionKey ? `moltis:session:${sessionKey}` : undefined;
				const notifications = await sw.registration.getNotifications(tag ? { tag } : {});
				for (const notification of notifications) {
					notification.close();
				}
				const remaining = await sw.registration.getNotifications();
				if (remaining.length === 0) {
					updateBadge(0);
				}
			})(),
		);
	}
});
