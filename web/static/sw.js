// Relay — service worker for Web Push notifications.
// Served from the site root as /sw.js (SvelteKit static adapter copies static/*).

self.addEventListener("install", () => {
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});

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
