import { derived, readable, writable } from "svelte/store";

// Appearance (CI ABGLEICH RL-T2, Kai 2026-10-06): light, dark or follow the
// system. Dark mode is the class `dunkel` on <html> — the switch the CI's
// tokens/app.css expects. Default is "system".
//
// The choice used to live in `relay_theme` ("blue" | "dark") and was applied
// in three places. It is migrated once so nobody loses their setting; after
// that only `relay_appearance` is read.
//
// The inline script in app.html does the same before first paint so the page
// does not flash light. Keep both in sync.

export type Appearance = "light" | "dark" | "system";

const KEY = "relay_appearance";
const LEGACY_KEY = "relay_theme";

function read(): Appearance {
  try {
    const value = localStorage.getItem(KEY);
    if (value === "light" || value === "dark" || value === "system") return value;
    const legacy = localStorage.getItem(LEGACY_KEY);
    if (legacy === "dark" || legacy === "blue") {
      const migrated: Appearance = legacy === "dark" ? "dark" : "light";
      localStorage.setItem(KEY, migrated);
      localStorage.removeItem(LEGACY_KEY);
      return migrated;
    }
  } catch {
    // No storage (private window): fall back to the default.
  }
  return "system";
}

export const appearance = writable<Appearance>(typeof window === "undefined" ? "system" : read());

appearance.subscribe((value) => {
  if (typeof window === "undefined") return;
  try {
    localStorage.setItem(KEY, value);
  } catch {
    // ignore
  }
});

const systemDark = readable(false, (set) => {
  if (typeof window === "undefined" || !window.matchMedia) return;
  const query = window.matchMedia("(prefers-color-scheme: dark)");
  set(query.matches);
  const onChange = (e: MediaQueryListEvent) => set(e.matches);
  query.addEventListener("change", onChange);
  return () => query.removeEventListener("change", onChange);
});

/** Whether the UI is drawn dark right now — the choice or, for "system", the device. */
export const isDark = derived(
  [appearance, systemDark],
  ([$appearance, $systemDark]) => $appearance === "dark" || ($appearance === "system" && $systemDark),
);

/** Applies the class and the browser bar colour; call once from the layout. */
export function applyAppearance(): () => void {
  return isDark.subscribe((dark) => {
    if (typeof document === "undefined") return;
    document.documentElement.classList.toggle("dunkel", dark);
    // Same colour as the page (--am-seite) so the browser bar sits flush. Once
    // a choice is applied it overrides both media-specific tags from app.html.
    const colour = tokenValue("--am-seite") || (dark ? "#051729" : "#ffffff");
    const metas = document.querySelectorAll<HTMLMetaElement>('meta[name="theme-color"]');
    if (metas.length === 0) {
      const meta = document.createElement("meta");
      meta.name = "theme-color";
      meta.content = colour;
      document.head.appendChild(meta);
    }
    metas.forEach((meta) => {
      meta.content = colour;
    });
  });
}

/**
 * Current value of a CI token, for places that cannot use CSS variables —
 * e.g. the document inside the mail preview iframe.
 */
export function tokenValue(name: string): string {
  if (typeof document === "undefined") return "";
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}
