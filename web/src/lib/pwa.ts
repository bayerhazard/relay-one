// The service worker runs from the first start, not only once push is on
// (Kai, 7.10.2026): without it the installed app does not open without a
// network. Push stays its own switch in the settings (setupPush).

/** Registers /sw.js; quiet where the browser has no service workers. */
export async function appDienstStarten(): Promise<void> {
  if (typeof window === "undefined" || !("serviceWorker" in navigator)) return;
  // Service workers need a secure context (HTTPS, or localhost in testing).
  if (!window.isSecureContext) return;
  try {
    await navigator.serviceWorker.register(`${import.meta.env.BASE_URL ?? ""}sw.js`);
  } catch (e) {
    console.warn("Service worker not registered", e);
  }
}
