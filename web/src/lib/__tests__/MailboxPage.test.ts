// @vitest-environment happy-dom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/svelte";
import Page from "../../routes/+page.svelte";
import * as tauri from "$lib/services/tauri";

const mailboxState = vi.hoisted(() => ({
  value: { messages: [] as any[], selectedUids: [] as number[], lastClickedUid: null as number | null, loading: false, error: null as string | null } as Record<string, any>,
  subscribers: [] as Array<(v: any) => void>,
}));

vi.mock("$lib/services/tauri", async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>;
  const stub = (fn: (...args: any[]) => any) => vi.fn(fn);
  return {
    ...actual,
    cacheInit: stub(() => undefined),
    listAccounts: vi.fn(),
    listImapFolders: vi.fn(),
    fetchFromImap: vi.fn(),
    fetchMessages: vi.fn(),
    connectAccount: vi.fn(),
    deleteAccount: vi.fn(),
    saveSettings: vi.fn(),
    getSettings: vi.fn().mockResolvedValue(null),
    markAsRead: vi.fn().mockResolvedValue(undefined),
    markBatchAsRead: vi.fn().mockResolvedValue(undefined),
    markBatchAsUnseen: vi.fn().mockResolvedValue(undefined),
    flagMessageCmd: vi.fn().mockResolvedValue(undefined),
    urgentMessageCmd: vi.fn().mockResolvedValue(undefined),
    fetchMessageBody: vi.fn(),
    fetchRawMessage: vi.fn().mockResolvedValue(""),
    fetchAttachments: vi.fn().mockResolvedValue([]),
    getOwnPhoto: vi.fn().mockResolvedValue(null),
    loadAttachmentContent: vi.fn().mockResolvedValue(""),
    sendMessage: vi.fn(),
    deleteMessageCmd: vi.fn().mockResolvedValue(undefined),
    moveMessageCmd: vi.fn(),
    getMoveToTrash: vi.fn().mockResolvedValue(true),
    setMoveToTrash: vi.fn(),
    getUnreadCounts: vi.fn().mockResolvedValue({}),
    ping: vi.fn().mockResolvedValue("pong"),
    searchContacts: vi.fn().mockResolvedValue([]),
    syncCardDav: vi.fn().mockResolvedValue(0),
    getCardDavSettings: vi.fn().mockResolvedValue(null),
    setCardDavSettings: vi.fn(),
    renameFolder: vi.fn(),
    searchMessages: vi.fn(),
    saveAttachment: vi.fn(),
    openFilePicker: vi.fn(),
    aiGenerateReply: vi.fn(),
    aiSummarize: vi.fn(),
    triggerFolderSummaries: vi.fn(),
    resetCircuitBreaker: vi.fn(),
    aiDraftFromBullets: vi.fn(),
    aiFormatText: vi.fn(),
    aiDetectPriority: vi.fn(),
    fraudCheck: vi.fn(),
    exportToneProfiles: vi.fn(),
    aiGenerateMail: vi.fn(),
    getToneProfile: vi.fn(),
    aiSuggestRecipient: vi.fn(),
    aiSuggestSubject: vi.fn(),
    saveDraft: vi.fn(),
    discardDraft: vi.fn(),
    getVoiceSettings: vi.fn().mockResolvedValue(null),
    saveVoiceSettings: vi.fn(),
    voiceTranscribe: vi.fn(),
    openEventStream: vi.fn(() => null),
  };
});

vi.mock("$lib/stores/mailbox", () => ({
  mailbox: {
    subscribe: (cb: (v: any) => void) => {
      cb(mailboxState.value);
      mailboxState.subscribers.push(cb);
      return () => {
        const idx = mailboxState.subscribers.indexOf(cb);
        if (idx >= 0) mailboxState.subscribers.splice(idx, 1);
      };
    },
    setMessages: vi.fn((msgs: any[]) => {
      mailboxState.value = { ...mailboxState.value, messages: msgs, loading: false };
      mailboxState.subscribers.forEach((cb) => cb(mailboxState.value));
    }),
    selectSingle: vi.fn((uid: number) => {
      mailboxState.value = { ...mailboxState.value, selectedUids: [uid], lastClickedUid: uid };
      mailboxState.subscribers.forEach((cb) => cb(mailboxState.value));
    }),
    toggleSelect: vi.fn((uid: number) => {
      const uids = mailboxState.value.selectedUids.includes(uid)
        ? mailboxState.value.selectedUids.filter((u: number) => u !== uid)
        : [...mailboxState.value.selectedUids, uid];
      mailboxState.value = { ...mailboxState.value, selectedUids: uids, lastClickedUid: uid };
      mailboxState.subscribers.forEach((cb) => cb(mailboxState.value));
    }),
    selectRange: vi.fn(),
    selectAll: vi.fn(),
    clearSelection: vi.fn(() => {
      mailboxState.value = { ...mailboxState.value, selectedUids: [], lastClickedUid: null };
      mailboxState.subscribers.forEach((cb) => cb(mailboxState.value));
    }),
    updateMessage: vi.fn(),
    removeMessage: vi.fn(),
    setFolderId: vi.fn(),
    setLoading: vi.fn(),
    setError: vi.fn(),
    reset: vi.fn(),
  },
  getFolderCache: () => null,
  invalidateFolderCache: vi.fn(),
  resetFolderCache: vi.fn(),
}));

import { mailbox } from "$lib/stores/mailbox";

const testMessage = {
  uid: 42,
  subject: "Test Betreff",
  from: "Absender <absender@test.de>",
  date: "2025-01-15T10:00:00Z",
  is_read: false,
  is_flagged: false,
};

function makeAccount() {
  return { id: 1, name: "Testkonto", imap_host: "imap.test.com", imap_port: 993, smtp_host: "smtp.test.com", smtp_port: 465, username: "test@example.com", smtp_username: "test@example.com", connected: true, sender_name: "Test", sender_email: "test@example.com" };
}

async function renderPageWithAccount(withMessages = true, selectUid: number | null = null, msgsOverride: any[] | null = null, foldersOverride: any[] | null = null) {
  const msgs = msgsOverride ?? (withMessages ? [testMessage] : []);
  mailboxState.value = { messages: msgs, selectedUids: selectUid != null ? [selectUid] : [], lastClickedUid: selectUid, loading: false, error: null };
  mailboxState.subscribers = [];

  vi.mocked(tauri.listAccounts).mockResolvedValue([makeAccount()]);
  vi.mocked(tauri.listImapFolders).mockResolvedValue(foldersOverride ?? []);
  vi.mocked(tauri.fetchFromImap).mockResolvedValue([]);
  vi.mocked(tauri.fetchMessages).mockResolvedValue(msgs);

  render(Page);

  await waitFor(() => {
    expect(document.querySelector(".ss-input")).toBeTruthy();
  });
}

describe("Sent Folder - Shows Recipient Instead of Sender (Regression Test)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const sentMsg = {
      ...testMessage,
      from: "Absender <absender@gmx.de>",
      to: "Empfänger <empfaenger@example.com>",
    };
    mailboxState.value = { messages: [sentMsg], selectedUids: [], lastClickedUid: null, loading: false, error: null };
    mailboxState.subscribers = [];
  });

  it("shows recipient (to) in sent folder instead of sender (from)", async () => {
    const sentMsg = {
      ...testMessage,
      from: "Absender <absender@gmx.de>",
      to: "Empfänger <empfaenger@example.com>",
    };
    // Provide a "Sent" folder so the sidebar renders it, then select it.
    await renderPageWithAccount(true, null, [sentMsg], [{ name: "Sent", raw_name: "Sent", delimiter: ".", tag: "", attributes: ["Sent"] }]);
    // Enter the Sent folder (label is rendered via translateFolder("Sent")).
    await waitFor(() => {
      expect(screen.getByText("Gesendet")).toBeTruthy();
    }, { timeout: 5000 });
    await fireEvent.click(screen.getByText("Gesendet"));
    await waitFor(() => {
      expect(screen.getByText(/Empfänger/)).toBeTruthy();
    });
    // The sender should NOT appear in the message list for sent folder
    expect(screen.queryByText(/Absender/)).toBeFalsy();
  });

  it("shows sender (from) in inbox (non-sent folder)", async () => {
    await renderPageWithAccount();
    // In inbox, we should see the sender
    await waitFor(() => {
      expect(screen.getByText(/Absender/)).toBeTruthy();
    });
  });
});

describe("Mailbox Page - Neue Nachricht (Bug 2)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mailboxState.value = { messages: [], selectedUids: [], lastClickedUid: null, loading: false, error: null };
    mailboxState.subscribers = [];
  });

  it("opens compose with empty An: field when clicking new mail button", async () => {
    await renderPageWithAccount();
    await fireEvent.click(screen.getByTitle("Neue E-Mail (Strg+N / Cmd+N)"));
    expect(screen.getByText("Neue Nachricht")).toBeTruthy();
    const toInput = screen.getByPlaceholderText("Name oder E-Mail-Adresse") as HTMLInputElement;
    expect(toInput.value).toBe("");
  });

  it("opens compose with empty Betreff field for new mail", async () => {
    await renderPageWithAccount();
    await fireEvent.click(screen.getByTitle("Neue E-Mail (Strg+N / Cmd+N)"));
    const subjectInput = screen.getByPlaceholderText("Betreff") as HTMLInputElement;
    expect(subjectInput.value).toBe("");
  });
});

describe("Mailbox Page - Nachricht loeschen (Bug 3)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mailboxState.value = { messages: [testMessage], selectedUids: [42], lastClickedUid: 42, loading: false, error: null };
    mailboxState.subscribers = [];
  });

  async function clickDeleteButton() {
    await waitFor(() => {
      expect(screen.getByRole("button", { name: /löschen/i })).toBeTruthy();
    });
    const deleteBtn = screen.getByRole("button", { name: /löschen/i });
    await fireEvent.click(deleteBtn);
  }

  function getDialog() {
    return screen.getByRole("alertdialog");
  }

  function getConfirmButton() {
    const dialog = getDialog();
    return within(dialog).getByRole("button", { name: "In Papierkorb" });
  }

  function getCancelButton() {
    const dialog = getDialog();
    return within(dialog).getByRole("button", { name: "Abbrechen" });
  }

  it("shows confirmation dialog when delete button is clicked", async () => {
    await renderPageWithAccount(true, 42);
    await clickDeleteButton();
    expect(getDialog()).toBeTruthy();
    expect(getDialog().textContent).toContain("Nachricht in den Papierkorb verschieben");
  });

  it("calls deleteMessageCmd after confirming deletion (trash mode)", async () => {
    await renderPageWithAccount(true, 42);
    await clickDeleteButton();
    await fireEvent.click(getConfirmButton());
    expect(tauri.deleteMessageCmd).toHaveBeenCalledWith(1, 42, expect.stringMatching(/INBOX|.*/));
    expect(tauri.deleteMessageCmd).toHaveBeenCalledTimes(1);
  });

  it("does not call deleteMessageCmd when deletion is cancelled", async () => {
    await renderPageWithAccount(true, 42);
    await clickDeleteButton();
    await fireEvent.click(getCancelButton());
    expect(tauri.deleteMessageCmd).not.toHaveBeenCalled();
  });

  it("closes dialog when cancel is clicked", async () => {
    await renderPageWithAccount(true, 42);
    await clickDeleteButton();
    expect(getDialog()).toBeTruthy();
    await fireEvent.click(getCancelButton());
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("does not render delete button when no message is selected", async () => {
    await renderPageWithAccount(true, null);
    expect(screen.queryByRole("button", { name: /löschen/i })).toBeNull();
  });

  it("shows empty-state hint when no message is selected", async () => {
    await renderPageWithAccount(false, null);
    expect(screen.getByText("Wähle eine Nachricht")).toBeTruthy();
  });

  it("confirmation dialog prevents rapid double-click from double-deleting", async () => {
    await renderPageWithAccount(true, 42);
    await clickDeleteButton();
    expect(getDialog()).toBeTruthy();
    await fireEvent.click(getConfirmButton());
    expect(tauri.deleteMessageCmd).toHaveBeenCalledTimes(1);
  });

  it("isDeleting guard prevents second delete while first is in-flight", async () => {
    const neverResolve = new Promise<never>(() => {});
    vi.mocked(tauri.deleteMessageCmd).mockResolvedValueOnce(neverResolve as any);

    await renderPageWithAccount(true, 42);
    await clickDeleteButton();
    expect(getDialog()).toBeTruthy();

    await fireEvent.click(getConfirmButton());

    const deleteBtn = screen.getByRole("button", { name: /löschen/i });
    await fireEvent.click(deleteBtn);

    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(tauri.deleteMessageCmd).toHaveBeenCalledTimes(1);
  });
});

describe("Mail-Link Kontextmenü (iframe link-contextmenu)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  function dispatchLinkMenu(url: string) {
    window.dispatchEvent(
      new MessageEvent("message", {
        data: { type: "link-contextmenu", url, x: 120, y: 140 },
      }),
    );
  }

  it("zeigt das eigene Menü mit beiden Einträgen bei sicherer URL", async () => {
    await renderPageWithAccount();
    dispatchLinkMenu("https://example.com/report");
    await waitFor(() => {
      expect(screen.getByText("Link öffnen")).toBeTruthy();
    });
    expect(screen.getByText("Link im Standardbrowser öffnen")).toBeTruthy();
  });

  it("zeigt KEIN Menü bei javascript:-URL (Phishing-Vektor)", async () => {
    await renderPageWithAccount();
    dispatchLinkMenu("javascript:alert(1)");
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByText("Link öffnen")).toBeNull();
  });

  it("öffnet den Link per window.open('_blank') und schließt das Menü", async () => {
    const openSpy = vi.fn();
    vi.stubGlobal("open", openSpy);
    await renderPageWithAccount();
    dispatchLinkMenu("https://example.com/report");
    await waitFor(() => {
      expect(screen.getByText("Link öffnen")).toBeTruthy();
    });
    await fireEvent.click(screen.getByText("Link öffnen"));
    expect(openSpy).toHaveBeenCalledWith("https://example.com/report", "_blank", "noopener");
    await waitFor(() => {
      expect(screen.queryByText("Link öffnen")).toBeNull();
    });
    vi.unstubAllGlobals();
  });
});

describe("Kontextmenü Mail-Zeile — Multiselektion (Regression 26.9.135)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  const mA = { uid: 101, subject: "Erste Mail", from: "a@x.de", date: "2025-01-15T10:00:00Z", is_read: false, is_flagged: false };
  const mB = { uid: 102, subject: "Zweite Mail", from: "b@x.de", date: "2025-01-15T10:00:00Z", is_read: false, is_flagged: false };
  const mC = { uid: 103, subject: "Dritte Mail", from: "c@x.de", date: "2025-01-15T10:00:00Z", is_read: false, is_flagged: false };

  async function rightClickRow(subject: string, sel: number[]) {
    await waitFor(() => {
      const rows = Array.from(document.querySelectorAll(".message-item"));
      expect(rows.some((r) => r.textContent?.includes(subject))).toBe(true);
    });
    const row = Array.from(document.querySelectorAll(".message-item")).find((r) => r.textContent?.includes(subject)) as Element;
    await fireEvent.contextMenu(row);
    await waitFor(() => {
      expect(document.querySelector(".ctx-menu")).toBeTruthy();
    });
    // every menu entry carries a leading icon
    for (const item of document.querySelectorAll(".ctx-menu .ctx-menu-item")) {
      expect(item.querySelector(".ctx-icon svg")).not.toBeNull();
    }
    // Apply the selection while the menu is open (runContextAction reads it
    // at click time); avoids racing the async folder-load clearSelection.
    mailboxState.value = { ...mailboxState.value, selectedUids: sel, lastClickedUid: sel[0], folderId: "INBOX", messagesFolder: "INBOX" };
    mailboxState.subscribers.forEach((cb) => cb(mailboxState.value));
    await new Promise((r) => setTimeout(r, 20));
  }

  it("Löschen im Menü wirkt auf ALLE ausgewählten Mails", async () => {
    await renderPageWithAccount(true, null, [mA, mB, mC]);
    await rightClickRow("Erste Mail", [101, 102]);
    await fireEvent.click(screen.getByRole("menuitem", { name: "Löschen" }));
    const dialog = screen.getByRole("alertdialog");
    await fireEvent.click(within(dialog).getByRole("button", { name: "In Papierkorb" }));
    await waitFor(() => {
      expect(tauri.deleteMessageCmd).toHaveBeenCalledTimes(2);
    });
    const calledUids = vi.mocked(tauri.deleteMessageCmd).mock.calls.map((c) => c[1]).sort();
    expect(calledUids).toEqual([101, 102]);
  });

  it("Rechtsklick AUSSERHALB der Selektion wirkt nur auf die eine Mail", async () => {
    await renderPageWithAccount(true, null, [mA, mB, mC]);
    await rightClickRow("Dritte Mail", [101]);
    await fireEvent.click(screen.getByRole("menuitem", { name: "Löschen" }));
    const dialog = screen.getByRole("alertdialog");
    await fireEvent.click(within(dialog).getByRole("button", { name: "In Papierkorb" }));
    await waitFor(() => {
      expect(tauri.deleteMessageCmd).toHaveBeenCalledTimes(1);
    });
    expect(tauri.deleteMessageCmd).toHaveBeenCalledWith(1, 103, expect.anything());
    expect(mailbox.selectSingle).toHaveBeenCalledWith(103);
  });

  it("Als gelesen markieren nutzt die Batch-API für die ganze Selektion", async () => {
    await renderPageWithAccount(true, null, [mA, mB, mC]);
    await rightClickRow("Erste Mail", [101, 102]);
    await fireEvent.click(screen.getByRole("menuitem", { name: "Als gelesen markieren" }));
    await waitFor(() => {
      expect(tauri.markBatchAsRead).toHaveBeenCalledWith(1, [101, 102], expect.anything());
    });
    expect(tauri.markAsRead).not.toHaveBeenCalled();
  });

  it("Markieren wirkt auf alle ausgewählten Mails", async () => {
    await renderPageWithAccount(true, null, [mA, mB, mC]);
    await rightClickRow("Erste Mail", [101, 102]);
    await fireEvent.click(screen.getByRole("menuitem", { name: "Markieren" }));
    await waitFor(() => {
      expect(tauri.flagMessageCmd).toHaveBeenCalledTimes(2);
    });
    const flagUids = vi.mocked(tauri.flagMessageCmd).mock.calls.map((c) => c[1]).sort();
    expect(flagUids).toEqual([101, 102]);
  });
});
