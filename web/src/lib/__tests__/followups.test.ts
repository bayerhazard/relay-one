import { describe, it, expect, vi, afterEach } from "vitest";
import { getFollowups } from "$lib/services/tauri";

function mockFetchOnce(status: number, body: unknown): void {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      ok: status >= 200 && status < 300,
      status,
      json: async () => body,
    }),
  );
}

describe("followups service", () => {
  afterEach(() => { vi.unstubAllGlobals(); });

  it("getFollowups POSTs /ai/followups with the message", async () => {
    mockFetchOnce(200, [{ id: "fu-1", kind: "task", label: "Antworten", task: { summary: "Antworten", due: null } }]);
    const res = await getFollowups("Q3-Budget", "chef@example.com", "bitte bis Freitag");
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/ai/followups");
    expect(opts?.method).toBe("POST");
    const body = JSON.parse(opts?.body as string);
    expect(body).toEqual({ subject: "Q3-Budget", from: "chef@example.com", body: "bitte bis Freitag" });
    expect(res).toHaveLength(1);
    expect(res[0].kind).toBe("task");
    expect(res[0].task?.summary).toBe("Antworten");
  });

  it("getFollowups returns an empty list when the AI finds nothing", async () => {
    mockFetchOnce(200, []);
    const res = await getFollowups("Hallo", "a@b.c", "text");
    expect(res).toEqual([]);
  });
});
