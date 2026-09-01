import { describe, it, expect, beforeEach, vi } from "vitest";
import { get } from "svelte/store";
import { settings, showDiffEnabled } from "$lib/stores/settings";
import type { AISettings } from "$lib/stores/settings";

const fetchMock = vi.hoisted(() => vi.fn());

function jsonResponse(body: unknown, ok = true) {
  return {
    ok,
    status: ok ? 200 : 500,
    json: async () => body,
  };
}

const STORAGE_KEY = "relay_settings";

describe("settings store", () => {
  beforeEach(() => {
    localStorage.clear();
    settings.reset();
    vi.clearAllMocks();
    (globalThis as any).fetch = fetchMock;
  });

  it("has correct defaults", () => {
    const value = get(settings);
    expect(value.url).toBe("https://llm.aimighty.de/v1");
    expect(value.api_key).toBe("ollama");
    expect(value.model).toBe("llama3.2");
  });

  it("loads custom settings and persists to localStorage (without api_key)", () => {
    const custom: AISettings = {
      url: "https://llm.aimighty.de/v1",
      api_key: "secret-key",
      model: "chat",
    };
    settings.load(custom);
    expect(get(settings)).toEqual(custom);

    // Verify localStorage was written WITHOUT api_key
    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY)!);
    expect(stored.url).toBe("https://llm.aimighty.de/v1");
    expect(stored.model).toBe("chat");
    expect(stored.api_key).toBeUndefined();
  });

  it("resets to defaults and persists to localStorage", () => {
    settings.load({
      url: "https://custom.url",
      api_key: "test",
      model: "gpt-4",
    });
    settings.reset();
    const value = get(settings);
    expect(value.url).toBe("https://llm.aimighty.de/v1");
    expect(value.api_key).toBe("ollama");
    expect(value.model).toBe("llama3.2");

    // Verify localStorage was updated with defaults
    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY)!);
    expect(stored.url).toBe("https://llm.aimighty.de/v1");
  });
});

describe("settings.init()", () => {
  beforeEach(() => {
    localStorage.clear();
    settings.reset();
    vi.clearAllMocks();
    (globalThis as any).fetch = fetchMock;
  });

  it("loads from backend when available", async () => {
    const expected: AISettings = {
      url: "https://llm.example.com/v1",
      api_key: "sk-test",
      model: "gpt-4",
    };
    fetchMock.mockResolvedValue(jsonResponse(expected));

    const result = await settings.init();

    expect(result).toEqual(expected);
    expect(get(settings)).toEqual(expected);
    // Should also persist to localStorage (without api_key)
    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY)!);
    expect(stored.url).toBe(expected.url);
    expect(stored.model).toBe(expected.model);
    expect(stored.api_key).toBeUndefined();
  });

  it("falls back to localStorage when backend fails", async () => {
    // Pre-populate localStorage (without api_key, as per new security policy)
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      url: "https://local-llm.example.com/v1",
      model: "llama3",
    }));

    // Backend fails
    fetchMock.mockRejectedValue(new Error("Backend unavailable"));

    const result = await settings.init();

    expect(result.url).toBe("https://local-llm.example.com/v1");
    expect(result.model).toBe("llama3");
    expect(result.api_key).toBe("");
  });

it("returns defaults when both backend and localStorage are unavailable", async () => {
    fetchMock.mockRejectedValue(new Error("IPC unavailable"));
    localStorage.clear();

    const result = await settings.init();

    expect(result.url).toBe("https://llm.aimighty.de/v1");
    expect(result.api_key).toBe("ollama");
    expect(result.model).toBe("llama3.2");
  });

  it("returns defaults when backend returns null", async () => {
    fetchMock.mockResolvedValue(jsonResponse(null));
    // Clear localStorage so we get true defaults (not the reset() fallback)
    localStorage.clear();

    const result = await settings.init();

    expect(result.url).toBe("https://llm.aimighty.de/v1");
    expect(result.api_key).toBe("ollama");
    expect(result.model).toBe("llama3.2");
  });

it("handles corrupted localStorage JSON gracefully", async () => {
    localStorage.setItem(STORAGE_KEY, "not-valid-json");
    fetchMock.mockRejectedValue(new Error("IPC unavailable"));

    const result = await settings.init();

    // Should fall through to defaults
    expect(result.url).toBe("https://llm.aimighty.de/v1");
  });

it("handles localStorage with missing fields gracefully", async () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ url: "https://partial.com/v1" }));
    fetchMock.mockRejectedValue(new Error("IPC unavailable"));

    const result = await settings.init();

    // Should fall through to defaults since validation fails
    expect(result.url).toBe("https://llm.aimighty.de/v1");
  });
});

describe("settings.save()", () => {
  beforeEach(() => {
    localStorage.clear();
    settings.reset();
    vi.clearAllMocks();
    (globalThis as any).fetch = fetchMock;
  });

  it("saves to both backend and localStorage", async () => {
    fetchMock.mockResolvedValue(jsonResponse(undefined));

    await settings.save("https://llm.example.com/v1", "sk-test", "gpt-4");

    // Verify backend was called
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/v1/settings",
      expect.objectContaining({
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          url: "https://llm.example.com/v1",
          api_key: "sk-test",
          model: "gpt-4",
        }),
      })
    );

    // Verify localStorage was written (without api_key)
    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY)!);
    expect(stored.url).toBe("https://llm.example.com/v1");
    expect(stored.model).toBe("gpt-4");
    expect(stored.api_key).toBeUndefined();

    // Verify store was updated (in-memory still has api_key)
    expect(get(settings)).toEqual({
      url: "https://llm.example.com/v1",
      api_key: "sk-test",
      model: "gpt-4",
    });
  });

  it("preserves data in localStorage when backend fails", async () => {
    fetchMock.mockRejectedValue(new Error("IPC connection failed"));

    await expect(
      settings.save("https://llm.example.com/v1", "sk-test", "gpt-4")
    ).rejects.toThrow("IPC connection failed");

    // Data should still be in localStorage (without api_key)
    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY)!);
    expect(stored.url).toBe("https://llm.example.com/v1");
    expect(stored.model).toBe("gpt-4");
    expect(stored.api_key).toBeUndefined();

    // Store should still be updated
    expect(get(settings)).toEqual({
      url: "https://llm.example.com/v1",
      api_key: "sk-test",
      model: "gpt-4",
    });
  });
});

describe("settings.syncToBackend()", () => {
  beforeEach(() => {
    localStorage.clear();
    settings.reset();
    vi.clearAllMocks();
    (globalThis as any).fetch = fetchMock;
  });

  it("syncs localStorage data to backend when different from store", async () => {
    // Set up localStorage with different data (no api_key per security policy)
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      url: "https://local-llm.com/v1",
      model: "local-model",
    }));
    fetchMock.mockResolvedValue(jsonResponse(undefined));

    const synced = await settings.syncToBackend();

    expect(synced).toBe(true);
    // Store should be updated with api_key="" (not loaded from localStorage)
    expect(get(settings).url).toBe("https://local-llm.com/v1");
    expect(get(settings).model).toBe("local-model");
    expect(get(settings).api_key).toBe("");
  });

  it("returns false when localStorage matches store", async () => {
    // Store has defaults (api_key="ollama"), localStorage has url+model only.
    // syncToBackend compares url+model (ignores api_key since it's not persisted).
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({
        url: "https://llm.aimighty.de/v1",
        model: "llama3.2",
      })
    );

    const synced = await settings.syncToBackend();

    // localStorage matches store on url+model → no sync needed
    expect(synced).toBe(false);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("returns false when localStorage is empty", async () => {
    localStorage.clear();
    const synced = await settings.syncToBackend();
    expect(synced).toBe(false);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("throws when backend fails during sync", async () => {
    const localData: AISettings = {
      url: "https://other.com/v1",
      api_key: "other-key",
      model: "other-model",
    };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(localData));
    fetchMock.mockRejectedValue(new Error("Backend down"));

    await expect(settings.syncToBackend()).rejects.toThrow("Backend down");
  });
});

describe("showDiffEnabled store", () => {
  it("defaults to false", () => {
    expect(get(showDiffEnabled)).toBe(false);
  });

  it("can be toggled", () => {
    showDiffEnabled.set(true);
    expect(get(showDiffEnabled)).toBe(true);

    showDiffEnabled.set(false);
    expect(get(showDiffEnabled)).toBe(false);
  });
});
