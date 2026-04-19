// src/sw.ts
var sw = self;
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
  "/assets/icons/apple-touch-icon.png"
];
sw.addEventListener("install", (event) => {
  event.waitUntil(
    caches.open(CACHE_NAME).then((cache) => {
      return cache.addAll(STATIC_ASSETS);
    })
  );
  sw.skipWaiting();
});
sw.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys().then((cacheNames) => {
      return Promise.all(cacheNames.filter((name) => name !== CACHE_NAME).map((name) => caches.delete(name)));
    })
  );
  sw.clients.claim();
});
sw.addEventListener("fetch", (event) => {
  const url = new URL(event.request.url);
  if (url.protocol === "ws:" || url.protocol === "wss:") {
    return;
  }
  if (url.pathname.startsWith("/api/") || url.pathname.startsWith("/ws/")) {
    return;
  }
  if (url.pathname.startsWith("/assets/") || url.pathname === "/manifest.json") {
    event.respondWith(
      caches.match(event.request).then((cached) => {
        if (cached) {
          event.waitUntil(
            fetch(event.request).then((response) => {
              if (response.ok) {
                caches.open(CACHE_NAME).then((cache) => {
                  cache.put(event.request, response);
                });
              }
            })
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
      })
    );
    return;
  }
  if (event.request.mode === "navigate") {
    event.respondWith(
      fetch(event.request).then((response) => {
        if (response.ok) {
          const responseClone = response.clone();
          caches.open(CACHE_NAME).then((cache) => {
            cache.put(event.request, responseClone);
          });
        }
        return response;
      }).catch(() => {
        return caches.match(event.request).then((cached) => {
          if (cached) return cached;
          return caches.match("/onboarding").then((onboardingCached) => {
            if (onboardingCached) return onboardingCached;
            return caches.match("/");
          });
        });
      })
    );
    return;
  }
});
sw.addEventListener("push", (event) => {
  let data = {};
  try {
    data = event.data ? event.data.json() : {};
  } catch (_e) {
    data = { body: event.data ? event.data.text() : "New message from moltis" };
  }
  const options = {
    body: data.body || "New response available",
    icon: "/assets/icons/icon-192.png",
    badge: "/assets/icons/icon-72.png",
    tag: data.sessionKey || "moltis-notification",
    data: {
      url: data.url || "/chats",
      sessionKey: data.sessionKey
    },
    actions: [
      { action: "open", title: "View" },
      { action: "dismiss", title: "Dismiss" }
    ],
    vibrate: [100, 50, 100],
    requireInteraction: false
  };
  event.waitUntil(sw.registration.showNotification(data.title || "moltis", options));
});
sw.addEventListener("notificationclick", (event) => {
  event.notification.close();
  if (event.action === "dismiss") {
    return;
  }
  const urlToOpen = event.notification.data?.url || "/chats";
  event.waitUntil(
    sw.clients.matchAll({ type: "window", includeUncontrolled: true }).then((clientList) => {
      for (const client of clientList) {
        if (client.url.includes(self.location.origin) && "focus" in client) {
          client.focus();
          client.postMessage({
            type: "notification-click",
            url: urlToOpen
          });
          return;
        }
      }
      return sw.clients.openWindow(urlToOpen);
    })
  );
});
sw.addEventListener("message", (event) => {
  if (event.data && event.data.type === "SKIP_WAITING") {
    sw.skipWaiting();
  }
});
