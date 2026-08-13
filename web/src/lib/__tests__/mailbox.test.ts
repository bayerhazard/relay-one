import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import {
  mailbox,
  getFolderCache,
  setFolderCache,
  invalidateFolderCache,
  resetFolderCache,
} from "$lib/stores/mailbox";
import type { Message } from "$lib/stores/mailbox";

const FOLDER_CACHE_KEY = "relay:folderCache:v1";

describe("mailbox store", () => {
  beforeEach(() => {
    mailbox.reset();
    localStorage.clear();
  });

  it("starts with empty state", () => {
    const state = get(mailbox);
    expect(state.messages).toEqual([]);
    expect(state.selectedUids).toEqual([]);
    expect(state.lastClickedUid).toBeNull();
    expect(state.loading).toBe(false);
    expect(state.error).toBeNull();
  });

  it("sets messages", () => {
    const messages: Message[] = [
      { uid: 1, subject: "Test", from: "test@example.com", is_read: false, is_flagged: false },
      { uid: 2, subject: "Another", from: "another@example.com", is_read: true, is_flagged: false },
    ];
    mailbox.setMessages(messages);
    expect(get(mailbox).messages).toEqual(messages);
    expect(get(mailbox).loading).toBe(false);
  });

  it("selects single message by uid", () => {
    mailbox.selectSingle(42);
    const state = get(mailbox);
    expect(state.selectedUids).toEqual([42]);
    expect(state.lastClickedUid).toBe(42);
  });

  it("toggles selection with toggleSelect", () => {
    mailbox.toggleSelect(1);
    expect(get(mailbox).selectedUids).toEqual([1]);
    mailbox.toggleSelect(2);
    expect(get(mailbox).selectedUids).toEqual([1, 2]);
    mailbox.toggleSelect(1);
    expect(get(mailbox).selectedUids).toEqual([2]);
  });

  it("clears selection with clearSelection", () => {
    mailbox.selectSingle(42);
    mailbox.clearSelection();
    const state = get(mailbox);
    expect(state.selectedUids).toEqual([]);
    expect(state.lastClickedUid).toBeNull();
  });

  it("selects range with selectRange", () => {
    const messages: Message[] = [
      { uid: 1, subject: "A", from: "a@example.com", is_read: false, is_flagged: false },
      { uid: 2, subject: "B", from: "b@example.com", is_read: false, is_flagged: false },
      { uid: 3, subject: "C", from: "c@example.com", is_read: false, is_flagged: false },
      { uid: 4, subject: "D", from: "d@example.com", is_read: false, is_flagged: false },
    ];
    mailbox.setMessages(messages);
    mailbox.selectRange(1, 3, messages);
    expect(get(mailbox).selectedUids).toEqual([2, 3, 4]);
  });

  it("selects all messages with selectAll", () => {
    const messages: Message[] = [
      { uid: 10, subject: "X", from: "x@example.com", is_read: false, is_flagged: false },
      { uid: 20, subject: "Y", from: "y@example.com", is_read: true, is_flagged: false },
    ];
    mailbox.selectAll(messages);
    expect(get(mailbox).selectedUids).toEqual([10, 20]);
  });

  it("updates specific message", () => {
    const messages: Message[] = [
      { uid: 1, subject: "Test", from: "test@example.com", is_read: false, is_flagged: false },
      { uid: 2, subject: "Another", from: "another@example.com", is_read: true, is_flagged: false },
    ];
    mailbox.setMessages(messages, "INBOX", 1);
    mailbox.setFolderId("INBOX");

    mailbox.updateMessage(1, "INBOX", { is_read: true });
    const updated = get(mailbox).messages;
    expect(updated[0].is_read).toBe(true);
    expect(updated[1].is_read).toBe(true);
  });

  it("does not apply an update scoped to a different folder (uid collision)", () => {
    const messages: Message[] = [
      { uid: 5, subject: "INBOX Mail", from: "inbox@test.com", is_read: false, is_flagged: false },
    ];
    mailbox.setMessages(messages, "INBOX", 1);
    mailbox.setFolderId("INBOX");

    // A summary event for a DIFFERENT folder that shares uid 5 must not touch
    // the currently displayed INBOX row.
    mailbox.updateMessage(5, "Entwürfe", { ai_summary: "WRONG" });
    const updated = get(mailbox).messages;
    expect(updated[0].ai_summary).toBeUndefined();
  });

  it("applies an update when the folder scopes match", () => {
    const messages: Message[] = [
      { uid: 5, subject: "INBOX Mail", from: "inbox@test.com", is_read: false, is_flagged: false },
    ];
    mailbox.setMessages(messages, "INBOX", 1);
    mailbox.setFolderId("INBOX");

    mailbox.updateMessage(5, "INBOX", { ai_summary: "RIGHT" });
    const updated = get(mailbox).messages;
    expect(updated[0].ai_summary).toBe("RIGHT");
  });

  it("falls back to uid-only match when no folder scope is provided", () => {
    const messages: Message[] = [
      { uid: 5, subject: "Mail", from: "a@test.com", is_read: false, is_flagged: false },
    ];
    mailbox.setMessages(messages, "INBOX", 1);
    mailbox.setFolderId("INBOX");

    mailbox.updateMessage(5, "", { is_read: true });
    expect(get(mailbox).messages[0].is_read).toBe(true);
  });

  it("sets loading state", () => {
    mailbox.setLoading(true);
    expect(get(mailbox).loading).toBe(true);

    mailbox.setLoading(false);
    expect(get(mailbox).loading).toBe(false);
  });

  it("sets error", () => {
    mailbox.setError("Connection failed");
    expect(get(mailbox).error).toBe("Connection failed");

    mailbox.setError(null);
    expect(get(mailbox).error).toBeNull();
  });

  it("resets to initial state", () => {
    mailbox.setMessages([{ uid: 1, subject: "Test", from: "test@example.com", is_read: false, is_flagged: false }]);
    mailbox.selectSingle(1);
    mailbox.setError("Some error");

    mailbox.reset();
    const state = get(mailbox);
    expect(state.messages).toEqual([]);
    expect(state.selectedUids).toEqual([]);
    expect(state.loading).toBe(false);
    expect(state.error).toBeNull();
  });
});

describe("folder cache", () => {
  beforeEach(() => {
    localStorage.clear();
    resetFolderCache();
  });

  const meta = (uid: number): Message => ({
    uid,
    subject: `Subj ${uid}`,
    from: "a@b.c",
    is_read: false,
    is_flagged: false,
  });

  it("setFolderCache stores and getFolderCache returns it", () => {
    setFolderCache(1, "INBOX", [meta(1), meta(2)]);
    const cached = getFolderCache(1, "INBOX");
    expect(cached).not.toBeNull();
    expect(cached!.map((m) => m.uid)).toEqual([1, 2]);
  });

  it("persists to localStorage", () => {
    setFolderCache(1, "INBOX", [meta(7)]);
    const raw = localStorage.getItem(FOLDER_CACHE_KEY);
    expect(raw).not.toBeNull();
    const parsed = JSON.parse(raw!);
    expect(parsed["1:INBOX"].map((m: Message) => m.uid)).toEqual([7]);
  });

  it("is per (account, folder)", () => {
    setFolderCache(1, "INBOX", [meta(1)]);
    setFolderCache(2, "INBOX", [meta(9)]);
    expect(getFolderCache(1, "INBOX")!.map((m) => m.uid)).toEqual([1]);
    expect(getFolderCache(2, "INBOX")!.map((m) => m.uid)).toEqual([9]);
    expect(getFolderCache(1, "Archive")).toBeNull();
  });

  it("invalidateFolderCache removes the entry", () => {
    setFolderCache(1, "INBOX", [meta(1)]);
    invalidateFolderCache(1, "INBOX");
    expect(getFolderCache(1, "INBOX")).toBeNull();
    expect(localStorage.getItem(FOLDER_CACHE_KEY)).toBeNull();
  });

  it("setMessages with a folder label refreshes the cache (meta-only)", () => {
    const msgs: Message[] = [
      { uid: 1, subject: "S", from: "a@b.c", body_text: "secret body", is_read: false, is_flagged: false },
    ];
    mailbox.setMessages(msgs, "INBOX", 1);
    const cached = getFolderCache(1, "INBOX");
    expect(cached!.length).toBe(1);
    // Bodies must NOT be persisted in the cache (payload reduction).
    expect(cached![0].body_text).toBeUndefined();
    expect(cached![0].subject).toBe("S");
  });

  it("search results (no folder label) do not touch the cache", () => {
    setFolderCache(1, "INBOX", [meta(1)]);
    const search: Message[] = [meta(99)];
    mailbox.setMessages(search);
    // Cache for INBOX is untouched by the folder-less search set.
    expect(getFolderCache(1, "INBOX")!.map((m) => m.uid)).toEqual([1]);
  });
});
