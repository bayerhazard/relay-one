import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { saveAttachment } from "$lib/services/tauri";

describe("saveAttachment (Web-Download)", () => {
  const orig = {
    URL: globalThis.URL,
    atob: globalThis.atob,
    Blob: globalThis.Blob,
    document: globalThis.document,
  };

  beforeEach(() => {
    const fakeA = { href: "", download: "", click: vi.fn(), remove: vi.fn() };
    const fakeDoc = {
      createElement: vi.fn(() => fakeA),
      body: { appendChild: vi.fn() },
    };
    vi.stubGlobal("URL", { createObjectURL: vi.fn(() => "blob:test"), revokeObjectURL: vi.fn() });
    vi.stubGlobal("atob", (s: string) => Buffer.from(s, "base64").toString("binary"));
    vi.stubGlobal("Blob", class { parts: any[]; constructor(parts: any[]) { this.parts = parts; } });
    vi.stubGlobal("document", fakeDoc);
    vi.stubGlobal("setTimeout", (fn: () => void) => { fn(); return 0 as any; });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("triggers a download and returns the filename", async () => {
    const b64 = Buffer.from("hallo welt").toString("base64");
    const result = await saveAttachment("test.txt", b64);
    expect(result).toBe("test.txt");
    const a = (document.createElement as any).mock.results[0].value;
    expect(a.download).toBe("test.txt");
    expect(a.click).toHaveBeenCalled();
  });

  it("returns null when atob throws", async () => {
    vi.stubGlobal("atob", () => { throw new Error("invalid"); });
    const result = await saveAttachment("test.txt", "any");
    expect(result).toBeNull();
  });
});
