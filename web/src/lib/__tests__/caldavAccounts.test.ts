import { describe, it, expect, vi, afterEach } from "vitest";
import { listCalDavAccounts, createCalDavAccount, updateCalDavAccount, deleteCalDavAccount } from "$lib/services/tauri";

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

describe("CalDAV accounts service", () => {
  afterEach(() => { vi.unstubAllGlobals(); });

  it("listCalDavAccounts GETs /calendars/caldav-accounts", async () => {
    mockFetchOnce(200, [{ id: "a1", name: "Privat", url: "https://c1/", username: "u", enabled: true, sync_interval_minutes: 15, has_password: true }]);
    const list = await listCalDavAccounts();
    const [url] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/calendars/caldav-accounts");
    expect(list[0].name).toBe("Privat");
  });

  it("createCalDavAccount POSTs with the payload", async () => {
    mockFetchOnce(200, { ok: true });
    await createCalDavAccount({ name: "Arbeit", url: "https://c2/", username: "u2", password: "p2" });
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/calendars/caldav-accounts");
    expect(opts?.method).toBe("POST");
    expect(JSON.parse(opts?.body as string).name).toBe("Arbeit");
  });

  it("updateCalDavAccount PUTs to /caldav-accounts/:id", async () => {
    mockFetchOnce(200, { ok: true });
    await updateCalDavAccount("a1", { url: "https://c1/", username: "u", enabled: false });
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/calendars/caldav-accounts/a1");
    expect(opts?.method).toBe("PUT");
  });

  it("deleteCalDavAccount DELETEs /caldav-accounts/:id", async () => {
    mockFetchOnce(200, { ok: true });
    await deleteCalDavAccount("a1");
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/calendars/caldav-accounts/a1");
    expect(opts?.method).toBe("DELETE");
  });
});
