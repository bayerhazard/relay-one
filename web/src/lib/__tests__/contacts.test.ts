import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  listContacts, createContact, updateContact, deleteContact,
  type ContactInput,
} from "$lib/services/tauri";

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

const input: ContactInput = {
  given_name: "Max", family_name: "Mustermann", display_name: "Max Mustermann",
  email: "max@example.com", phone: "+49123", organization: "ACME",
};

describe("contacts service", () => {
  afterEach(() => { vi.unstubAllGlobals(); });

  it("listContacts calls GET /contacts with search", async () => {
    mockFetchOnce(200, []);
    await listContacts("max");
    const fetchMock = vi.mocked(fetch);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, opts] = fetchMock.mock.calls[0];
    expect(String(url)).toContain("/contacts?search=max");
    expect(opts?.method).toBe("GET");
  });

  it("listContacts without search omits query", async () => {
    mockFetchOnce(200, []);
    await listContacts();
    const [url] = vi.mocked(fetch).mock.calls[0];
    const u = String(url);
    expect(u).toContain("/contacts");
    expect(u).not.toContain("?search");
  });

  it("createContact POSTs the input", async () => {
    mockFetchOnce(200, { vcard_uid: "u1", ...input, source: "carddav", synced_at: "now" });
    const res = await createContact(input);
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/contacts");
    expect(opts?.method).toBe("POST");
    expect(JSON.parse(opts?.body as string)).toEqual(input);
    expect(res.vcard_uid).toBe("u1");
  });

  it("updateContact PUTs to /contacts/:uid", async () => {
    mockFetchOnce(200, { vcard_uid: "u1", ...input, source: "carddav", synced_at: "now" });
    await updateContact("u1", input);
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/contacts/u1");
    expect(opts?.method).toBe("PUT");
  });

  it("deleteContact DELETEs /contacts/:uid", async () => {
    mockFetchOnce(200, { deleted: true });
    const res = await deleteContact("u1");
    const [url, opts] = vi.mocked(fetch).mock.calls[0];
    expect(String(url)).toContain("/contacts/u1");
    expect(opts?.method).toBe("DELETE");
    expect(res.deleted).toBe(true);
  });

  it("throws a friendly error on HTTP failure", async () => {
    mockFetchOnce(500, { error: "boom" });
    await expect(listContacts()).rejects.toThrow(/boom/);
  });
});
