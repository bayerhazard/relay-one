import { afterEach, describe, expect, it, vi } from "vitest";
import { deleteMessageCmd, getLoeschStand } from "$lib/services/tauri";

/**
 * Small changes carry keepalive, so a delete sent as the tab closes still
 * reaches the server (Kai, 7.10.2026); reads do not need it.
 */
describe("requests that outlive the page", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("a delete goes with keepalive, a read without", async () => {
    const fetch = vi.fn(async () => new Response("{}", { status: 200 }));
    vi.stubGlobal("fetch", fetch);
    await deleteMessageCmd(1, 7, "INBOX");
    await getLoeschStand();
    const [, loeschen] = fetch.mock.calls[0] as unknown as [string, RequestInit];
    const [, lesen] = fetch.mock.calls[1] as unknown as [string, RequestInit];
    expect(loeschen.keepalive).toBe(true);
    expect(lesen.keepalive).toBe(false);
  });
});
