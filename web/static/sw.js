// Relay — service worker: Web Push + offline app shell.
// Served from the site root as /sw.js (SvelteKit static adapter copies static/*).

const SHELL_CACHE = "relay-shell-v1";
const MAX_CACHE_ENTRIES = 50;

self.addEventListener("install", () => {
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    self.caches.keys().then((keys) =>
      Promise.all(
        keys.filter((k) => k !== SHELL_CACHE).map((k) => self.caches.delete(k))
      )
    ).then(() => self.clients.claim())
  );
});

self.addEventListener("fetch", (event) => {
  const url = new URL(event.request.url);
  if (url.origin !== self.location.origin) return;

  // API calls: network-only (never cache)
  if (url.pathname.startsWith("/api/")) return;

  // Navigation: network-first, fallback to cached shell
  if (event.request.mode === "navigate") {
    event.respondWith(
      fetch(event.request)
        .then((res) => {
          const copy = res.clone();
          self.caches.open(SHELL_CACHE).then((cache) => cache.put(event.request, copy));
          return res;
        })
        .catch(() => self.caches.match(event.request).then((cached) => cached || self.caches.match("/")))
    );
    return;
  }

  // Static assets: cache-first, then network
  if (event.request.method === "GET") {
    event.respondWith(
      self.caches.match(event.request).then((cached) => {
        if (cached) return cached;
        return fetch(event.request).then((res) => {
          if (res.ok) {
            const copy = res.clone();
            self.caches.open(SHELL_CACHE).then((cache) => {
              cache.put(event.request, copy);
              trimCache();
            });
          }
          return res;
        });
      })
    );
  }
});

async function trimCache() {
  const cache = await self.caches.open(SHELL_CACHE);
  const keys = await cache.keys();
  if (keys.length > MAX_CACHE_ENTRIES) {
    await cache.delete(keys[0]);
    trimCache();
  }
}

self.addEventListener("push", (event) => {
  let title = "Neue E-Mail";
  let body = "Du hast neue Nachrichten in Relay.";
  try {
    const data = event.data ? event.data.json() : {};
    if (data.title) title = data.title;
    if (data.body) body = data.body;
  } catch { /* keep defaults */ }

  const options = {
    body,
    icon: "/icon.png",
    badge: "/icon.png",
    tag: "relay-new-mail",
    renotify: true,
  };
  event.waitUntil(self.registration.showNotification(title, options));
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const url = self.location.origin + "/";
  event.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((clients) => {
      for (const client of clients) {
        if ("focus" in client) {
          client.focus();
          return;
        }
      }
      return self.clients.openWindow(url);
    })
  );
});
