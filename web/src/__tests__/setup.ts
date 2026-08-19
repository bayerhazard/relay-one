import { vi, beforeEach } from "vitest";
import { setLang } from "$lib/i18n";

// i18n state isolation: every test starts with German (the app default) so
// component tests that assert German UI text stay deterministic even when a
// previous test switched to English via setLang().
beforeEach(() => {
  try {
    localStorage.removeItem("relay_lang");
  } catch {
    /* ignore */
  }
  setLang("de");
});

// Mock the svelte module to provide mount function for SSR compatibility
vi.mock("svelte", async () => {
  const client = await import("svelte");
  return {
    ...(client as any),
    mount: (client as any).hydrate,
  };
});

// EventSource stub for jsdom (the SSE-based event stream is used by
// $lib/services/tauri.openEventStream; not natively available in jsdom).
if (typeof EventSource === "undefined") {
  (globalThis as any).EventSource = class EventSource {
    static readonly CONNECTING = 0;
    static readonly OPEN = 1;
    static readonly CLOSED = 2;
    readonly CONNECTING = 0;
    readonly OPEN = 1;
    readonly CLOSED = 2;
    onmessage: ((ev: any) => void) | null = null;
    onerror: ((ev: any) => void) | null = null;
    onopen: (() => void) | null = null;
    readyState: number = 0;
    url: string;
    constructor(url: string) {
      this.url = url;
    }
    close() {}
    addEventListener() {}
    removeEventListener() {}
  };
}

// Polyfill HTMLDialogElement for jsdom (not natively supported)
if (typeof HTMLDialogElement !== "undefined") {
  HTMLDialogElement.prototype.showModal = function () {
    this.open = true;
  };
  HTMLDialogElement.prototype.close = function () {
    this.open = false;
  };
}

// Polyfill ResizeObserver for jsdom (not natively supported)
if (typeof ResizeObserver === "undefined") {
  (globalThis as any).ResizeObserver = class ResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}
