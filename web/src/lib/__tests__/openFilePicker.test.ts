import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { openFilePicker } from "$lib/services/tauri";

describe("openFilePicker (Web)", () => {
  let clickFn: ReturnType<typeof vi.fn>;
  let onChange: ((e: Event) => void) | null;

  beforeEach(() => {
    clickFn = vi.fn();
    onChange = null;
    const fakeInput = {
      type: "",
      multiple: false,
      files: null,
      click: vi.fn(() => {
        clickFn();
      }),
      remove: vi.fn(),
      set onchange(fn: ((e: Event) => void) | null) {
        onChange = fn;
      },
      get onchange() {
        return onChange;
      },
    };
    vi.stubGlobal("document", {
      createElement: vi.fn(() => fakeInput),
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("returns an empty array when the user cancels (no files)", async () => {
    const p = openFilePicker();
    // Simulate: input created and clicked; user cancels → files stays null.
    expect(clickFn).toHaveBeenCalledOnce();
    onChange?.({ target: {} as EventTarget } as Event);
    await expect(p).resolves.toEqual([]);
  });

  it("reads selected files as base64 content", async () => {
    const file = new File(["hallo welt"], "test.txt", { type: "text/plain" });
    Object.defineProperty(file, "size", { value: 10 });
    const fakeInput = {
      type: "",
      multiple: true,
      files: [file],
      click: vi.fn(),
      remove: vi.fn(),
      onchange: null as ((e: Event) => void) | null,
    };
    vi.stubGlobal("document", {
      createElement: vi.fn(() => fakeInput),
    });

    const p = openFilePicker();

    // Trigger the change with the fake input's files.
    fakeInput.onchange?.({ target: fakeInput } as unknown as Event);

    const result = await p;
    expect(result.length).toBe(1);
    expect(result[0].filename).toBe("test.txt");
    expect(result[0].content_type).toBe("text/plain");
    expect(result[0].size).toBe(10);
    // FileReader produces a data URL; base64 part decodes back to the content.
    expect(Buffer.from(result[0].content, "base64").toString("utf8")).toBe("hallo welt");
  });
});
