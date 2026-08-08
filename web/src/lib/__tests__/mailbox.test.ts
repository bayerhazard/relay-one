import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import { mailbox } from "$lib/stores/mailbox";
import type { Message } from "$lib/stores/mailbox";

describe("mailbox store", () => {
  beforeEach(() => {
    mailbox.reset();
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
    mailbox.setMessages(messages);

    mailbox.updateMessage(1, { is_read: true });
    const updated = get(mailbox).messages;
    expect(updated[0].is_read).toBe(true);
    expect(updated[1].is_read).toBe(true);
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
