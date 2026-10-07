import { describe, it, expect, beforeEach, vi } from "vitest";
import { get } from "svelte/store";

// The store reads localStorage when the module loads, so each case imports a
// fresh copy after preparing storage.
async function load() {
  vi.resetModules();
  return import("$lib/stores/appearance");
}

describe("appearance (CI RL-T2)", () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.classList.remove("dunkel");
  });

  it("defaults to system", async () => {
    const { appearance } = await load();
    expect(get(appearance)).toBe("system");
  });

  it("migrates the legacy dark choice once and drops the old key", async () => {
    localStorage.setItem("relay_theme", "dark");
    const { appearance } = await load();
    expect(get(appearance)).toBe("dark");
    expect(localStorage.getItem("relay_appearance")).toBe("dark");
    expect(localStorage.getItem("relay_theme")).toBeNull();
  });

  it("migrates the legacy light choice ('blue') to light", async () => {
    localStorage.setItem("relay_theme", "blue");
    const { appearance } = await load();
    expect(get(appearance)).toBe("light");
  });

  it("prefers the new key over a leftover legacy key", async () => {
    localStorage.setItem("relay_appearance", "light");
    localStorage.setItem("relay_theme", "dark");
    const { appearance } = await load();
    expect(get(appearance)).toBe("light");
  });

  it("toggles the CI dark class on <html> and stores the choice", async () => {
    const { appearance, applyAppearance } = await load();
    const stop = applyAppearance();
    appearance.set("dark");
    expect(document.documentElement.classList.contains("dunkel")).toBe(true);
    expect(localStorage.getItem("relay_appearance")).toBe("dark");
    appearance.set("light");
    expect(document.documentElement.classList.contains("dunkel")).toBe(false);
    stop();
  });

  it("system follows prefers-color-scheme", async () => {
    const listeners: Array<(e: MediaQueryListEvent) => void> = [];
    const original = window.matchMedia;
    window.matchMedia = ((query: string) => ({
      matches: true,
      media: query,
      addEventListener: (_: string, cb: (e: MediaQueryListEvent) => void) => listeners.push(cb),
      removeEventListener: () => {},
    })) as unknown as typeof window.matchMedia;
    try {
      const { isDark, applyAppearance } = await load();
      const stop = applyAppearance();
      expect(get(isDark)).toBe(true);
      expect(document.documentElement.classList.contains("dunkel")).toBe(true);
      listeners.forEach((cb) => cb({ matches: false } as MediaQueryListEvent));
      expect(document.documentElement.classList.contains("dunkel")).toBe(false);
      stop();
    } finally {
      window.matchMedia = original;
    }
  });
});
