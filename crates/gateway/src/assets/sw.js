// Service Worker for moltis PWA
// Handles caching for offline support and push notifications

var CACHE_NAME = "moltis-v2";
var STATIC_ASSETS = [
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
self.addEventListener("install", (event) => {
  event.waitUntil(
    caches.open(CACHE_NAME).then((cache) => {
      return cache.addAll(STATIC_ASSETS);
    }),
  );
  // Activate immediately
  self.skipWaiting();
});

// Activate event - clean up old caches
self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys().then((cacheNames) => {
      return Promise.all(
        cacheNames
          .filter((name) => name !== CACHE_NAME)
          .map((name) => caches.delete(name)),
      );
    }),
  );
  // Take control of all pages immediately
  self.clients.claim();
});

// Fetch event - network first for API, cache first for assets
self.addEventListener("fetch", (event) => {
  var url = new URL(event.request.url);

  // Skip WebSocket requests
  if (url.protocol === "ws:" || url.protocol === "wss:") {
    return;
  }

  // API requests - network only (no caching)
  if (url.pathname.startsWith("/api/") || url.pathname === "/ws") {
    return;
  }

  // Static assets - cache first, then network
  if (
    url.pathname.startsWith("/assets/") ||
    url.pathname === "/manifest.json"
  ) {
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
            var responseClone = response.clone();
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
            var responseClone = response.clone();
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
              return caches.match("/");
            });
          });
        }),
    );
    return;
  }
});

// Push notification event
self.addEventListener("push", (event) => {
  var data = {};
  try {
    data = event.data ? event.data.json() : {};
  } catch (e) {
    data = { body: event.data ? event.data.text() : "New message from moltis" };
  }

  var options = {
    body: data.body || "New response available",
    icon: "/assets/icons/icon-192.png",
    badge: "/assets/icons/icon-72.png",
    tag: data.sessionKey || "moltis-notification",
    data: {
      url: data.url || "/chats",
      sessionKey: data.sessionKey,
    },
    actions: [
      { action: "open", title: "View" },
      { action: "dismiss", title: "Dismiss" },
    ],
    vibrate: [100, 50, 100],
    requireInteraction: false,
  };

  event.waitUntil(
    self.registration.showNotification(data.title || "moltis", options),
  );
});

// Notification click event
self.addEventListener("notificationclick", (event) => {
  event.notification.close();

  if (event.action === "dismiss") {
    return;
  }

  var urlToOpen = event.notification.data?.url || "/chats";

  event.waitUntil(
    clients.matchAll({ type: "window", includeUncontrolled: true }).then((clientList) => {
      // Try to focus an existing window
      for (var client of clientList) {
        if (client.url.includes(self.location.origin) && "focus" in client) {
          client.focus();
          // Navigate to the notification URL
          client.postMessage({
            type: "notification-click",
            url: urlToOpen,
          });
          return;
        }
      }
      // No existing window, open a new one
      return clients.openWindow(urlToOpen);
    }),
  );
});

// Handle messages from the main app
self.addEventListener("message", (event) => {
  if (event.data && event.data.type === "SKIP_WAITING") {
    self.skipWaiting();
  }
});
