// Relay — service worker: Web Push + offline app shell.
// Served from the site root as /sw.js (SvelteKit static adapter copies static/*).

const SHELL_CACHE = "relay-shell-v5";
const MAX_CACHE_ENTRIES = 200;

// The start page goes into the cache at install, so the installed app opens
// without a network from the first start on.
self.addEventListener("install", (event) => {
  event.waitUntil(
    self.caches.open(SHELL_CACHE)
      .then((cache) => cache.add("/"))
      .catch(() => { /* offline at install: the next navigation fills it */ })
      .then(() => self.skipWaiting())
  );
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
          if (res.ok) {
            const copy = res.clone();
            event.waitUntil(self.caches.open(SHELL_CACHE).then((cache) => cache.put(event.request, copy)));
          }
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
            event.waitUntil(self.caches.open(SHELL_CACHE)
              .then((cache) => cache.put(event.request, copy))
              .then(trimCache));
          }
          return res;
        });
      })
    );
  }
});

// Trims the oldest files, never a page: the shell must stay.
async function trimCache() {
  const cache = await self.caches.open(SHELL_CACHE);
  const keys = await cache.keys();
  const dateien = keys.filter((req) => !istSeite(req));
  const zuviel = keys.length - MAX_CACHE_ENTRIES;
  for (const req of dateien.slice(0, Math.max(0, zuviel))) await cache.delete(req);
}

function istSeite(req) {
  const pfad = new URL(req.url).pathname;
  return pfad === "/" || !/\.[a-z0-9]+$/i.test(pfad);
}

self.addEventListener("push", (event) => {
  let title = "Neue E-Mail";
  let body = "Sie haben neue Nachrichten in Relay.";
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
