// Service Worker for moltis PWA
// Handles caching for offline support and push notifications

/// <reference lib="webworker" />

// Service Worker global: `self` is Window in DOM lib but ServiceWorkerGlobalScope at runtime.
// The double cast is unavoidable when both DOM and WebWorker types coexist in tsconfig.
const sw = self as unknown as ServiceWorkerGlobalScope;

const CACHE_NAME = "moltis-v2";
const STATIC_ASSETS: string[] = [
	"/manifest.json",
	"/assets/css/base.css",
	"/assets/css/layout.css",
	"/assets/css/chat.css",
	"/assets/css/components.css",
	"/assets/style.css",
	"/assets/icons/icon-192.png",
	"/assets/icons/icon-512.png",
	"/assets/icons/apple-touch-icon.png",
];

// Install event - cache static assets
sw.addEventListener("install", (event: ExtendableEvent) => {
	event.waitUntil(
		caches.open(CACHE_NAME).then((cache) => {
			return cache.addAll(STATIC_ASSETS);
		}),
	);
	// Activate immediately
	sw.skipWaiting();
});

// Activate event - clean up old caches
sw.addEventListener("activate", (event: ExtendableEvent) => {
	event.waitUntil(
		caches.keys().then((cacheNames) => {
			return Promise.all(cacheNames.filter((name) => name !== CACHE_NAME).map((name) => caches.delete(name)));
		}),
	);
	// Take control of all pages immediately
	sw.clients.claim();
});

// Fetch event - network first for API, cache first for assets
sw.addEventListener("fetch", (event: FetchEvent) => {
	const url = new URL(event.request.url);

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
		event.respondWith(
			caches.match(event.request).then((cached) => {
				if (cached) {
					// Return cached version, but update cache in background
					event.waitUntil(
						fetch(event.request).then((response) => {
							if (response.ok) {
								caches.open(CACHE_NAME).then((cache) => {
									cache.put(event.request, response);
								});
							}
						}),
					);
					return cached;
				}
				return fetch(event.request).then((response) => {
					if (response.ok) {
						const responseClone = response.clone();
						caches.open(CACHE_NAME).then((cache) => {
							cache.put(event.request, responseClone);
						});
					}
					return response;
				});
			}),
		);
		return;
	}

	// HTML pages - network first, fallback to cache
	if (event.request.mode === "navigate") {
		event.respondWith(
			fetch(event.request)
				.then((response) => {
					// Cache successful responses
					if (response.ok) {
						const responseClone = response.clone();
						caches.open(CACHE_NAME).then((cache) => {
							cache.put(event.request, responseClone);
						});
					}
					return response;
				})
				.catch(() => {
					// Offline - return cached version or root page
					return caches.match(event.request).then((cached) => {
						if (cached) return cached;
						return caches.match("/onboarding").then((onboardingCached) => {
							if (onboardingCached) return onboardingCached;
							return caches.match("/") as Promise<Response>;
						});
					}) as Promise<Response>;
				}),
		);
		return;
	}
});

// Push notification event
sw.addEventListener("push", (event: PushEvent) => {
	let data: Record<string, unknown> = {};
	try {
		data = event.data ? event.data.json() : {};
	} catch (_e) {
		data = { body: event.data ? event.data.text() : "New message from moltis" };
	}

	const options: NotificationOptions & { actions?: Array<{ action: string; title: string }>; vibrate?: number[] } = {
		body: (data.body as string) || "New response available",
		icon: "/assets/icons/icon-192.png",
		badge: "/assets/icons/icon-72.png",
		tag: (data.sessionKey as string) || "moltis-notification",
		data: {
			url: (data.url as string) || "/chats",
			sessionKey: data.sessionKey,
		},
		actions: [
			{ action: "open", title: "View" },
			{ action: "dismiss", title: "Dismiss" },
		],
		vibrate: [100, 50, 100],
		requireInteraction: false,
	};

	event.waitUntil(sw.registration.showNotification((data.title as string) || "moltis", options));
});

// Notification click event
sw.addEventListener("notificationclick", (event: NotificationEvent) => {
	event.notification.close();

	if (event.action === "dismiss") {
		return;
	}

	const urlToOpen = (event.notification.data?.url as string) || "/chats";

	event.waitUntil(
		sw.clients.matchAll({ type: "window", includeUncontrolled: true }).then((clientList) => {
			// Try to focus an existing window
			for (const client of clientList) {
				if (client.url.includes(self.location.origin) && "focus" in client) {
					(client as WindowClient).focus();
					// Navigate to the notification URL
					client.postMessage({
						type: "notification-click",
						url: urlToOpen,
					});
					return;
				}
			}
			// No existing window, open a new one
			return sw.clients.openWindow(urlToOpen);
		}),
	);
});

// Handle messages from the main app
sw.addEventListener("message", (event: ExtendableMessageEvent) => {
	if (event.data && event.data.type === "SKIP_WAITING") {
		sw.skipWaiting();
	}
});
