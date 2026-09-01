import { writable, get } from "svelte/store";
import { getSettings, saveSettings as saveSettingsIpc } from "$lib/services/tauri";

export interface AISettings {
  url: string;
  api_key: string;
  model: string;
}

const STORAGE_KEY = "relay_settings";

const DEFAULTS: AISettings = {
  url: "https://llm.aimighty.de/v1",
  api_key: "ollama",
  model: "llama3.2",
};

function saveToLocalStorage(value: AISettings): void {
  try {
    // Never persist the API key in localStorage — it lives only in memory
    // and on the server. (Security: localStorage is readable by any script
    // on the same origin, including compromised iframes.)
    const { api_key: _omit, ...safe } = value;
    localStorage.setItem(STORAGE_KEY, JSON.stringify(safe));
  } catch (e) {
    console.warn("Failed to save settings to localStorage", e);
  }
}

function loadFromLocalStorage(): AISettings | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (
        parsed &&
        typeof parsed === "object" &&
        typeof parsed.url === "string" &&
        typeof parsed.model === "string"
      ) {
        // api_key is intentionally not loaded from localStorage
        return { url: parsed.url, api_key: "", model: parsed.model };
      }
    }
  } catch (e) {
    console.warn("Failed to parse settings from localStorage", e);
  }
  return null;
}

function createSettingsStore() {
  const { subscribe, set } = writable<AISettings>({ ...DEFAULTS });

  return {
    subscribe,

    load: (settings: AISettings) => {
      set(settings);
      saveToLocalStorage(settings);
    },

    reset: () => {
      set({ ...DEFAULTS });
      saveToLocalStorage(DEFAULTS);
    },

    /**
     * Initialize settings: try IPC first, fall back to localStorage.
     * Returns the loaded (or default) settings.
     */
    init: async (): Promise<AISettings> => {
      try {
        const s = await getSettings();
        if (s) {
          set(s);
          saveToLocalStorage(s);
          return s;
        }
      } catch (e) {
        console.warn("Settings IPC load failed, trying localStorage", e);
      }
      // Fallback to localStorage
      const local = loadFromLocalStorage();
      if (local) {
        set(local);
        return local;
      }
      return get(settings); // returns defaults
    },

    /**
     * Save settings to both IPC (primary) and localStorage (fallback).
     * If IPC fails, data is preserved in localStorage for later sync.
     */
    save: async (url: string, apiKey: string, model: string): Promise<void> => {
      const value: AISettings = { url, api_key: apiKey, model };
      // Save to localStorage immediately (always works)
      saveToLocalStorage(value);
      set(value);
      // Save to backend (IPC) — may throw if backend unavailable
      try {
        await saveSettingsIpc(url, apiKey, model);
      } catch (e) {
        console.warn("Settings IPC save failed, data preserved in localStorage", e);
        throw e;
      }
    },

    /**
     * Sync localStorage settings to backend.
     * Call this when backend becomes available again (e.g. after reconnect).
     * Returns true if sync was performed, false if nothing to sync.
     */
    syncToBackend: async (): Promise<boolean> => {
      const local = loadFromLocalStorage();
      if (!local) return false;
      const current = get(settings);
      // Only compare url+model — api_key is not persisted in localStorage
      if (
        local.url === current.url &&
        local.model === current.model
      ) {
        return false;
      }
      try {
        // Use the store's api_key (in-memory) since localStorage doesn't have it
        await saveSettingsIpc(local.url, current.api_key, local.model);
        set(local);
        return true;
      } catch (e) {
        console.warn("Settings sync to backend failed", e);
        throw e;
      }
    },
  };
}

export const settings = createSettingsStore();

// Persisted toggle: whether the AI diff editor is shown. Survives restarts.
function createShowDiffStore() {
  let initial = false;
  try {
    initial = localStorage.getItem("relay_show_diff") === "true";
  } catch { /* ignore */ }
  const store = writable<boolean>(initial);
  store.subscribe((v) => {
    try {
      localStorage.setItem("relay_show_diff", v ? "true" : "false");
    } catch { /* ignore */ }
  });
  return store;
}

export const showDiffEnabled = createShowDiffStore();
