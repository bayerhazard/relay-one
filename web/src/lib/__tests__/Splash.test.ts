import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/svelte";
import Page from "../../routes/+page.svelte";
import * as tauri from "$lib/services/tauri";

vi.mock("$lib/services/tauri", () => ({
  cacheInit: vi.fn().mockResolvedValue(undefined),
  listAccounts: vi.fn(),
  listImapFolders: vi.fn(),
  fetchFromImap: vi.fn(),
  fetchMessages: vi.fn(),
  connectAccount: vi.fn(),
  deleteAccount: vi.fn(),
  saveSettings: vi.fn(),
  getSettings: vi.fn().mockResolvedValue(null),
  markAsRead: vi.fn(),
  fetchMessageBody: vi.fn(),
  fetchRawMessage: vi.fn().mockResolvedValue(""),
  sendMessage: vi.fn(),
  deleteMessageCmd: vi.fn(),
  moveMessageCmd: vi.fn(),
  getMoveToTrash: vi.fn().mockResolvedValue(true),
  setMoveToTrash: vi.fn(),
  updateBadgeCount: vi.fn().mockResolvedValue(0),
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
  openEventStream: vi.fn(() => null),
}));

// Mock Svelte transition and others if needed
vi.mock("$lib/stores/mailbox", () => {
  return {
    mailbox: {
      subscribe: (fn: any) => {
        fn({ messages: [], selectedUids: [], lastClickedUid: null, folderId: "", messagesFolder: null, loading: false, error: null });
        return () => {};
      },
      setLoading: vi.fn(),
      setMessages: vi.fn(),
      setError: vi.fn(),
      setFolderId: vi.fn(),
      selectSingle: vi.fn(),
    },
    getFolderCache: () => null,
    invalidateFolderCache: vi.fn(),
    resetFolderCache: vi.fn(),
  };
});

describe("Splash Screen Integration in +page.svelte", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows Splash Screen if there are no accounts", async () => {
    vi.mocked(tauri.listAccounts).mockResolvedValue([]);
    render(Page);

    await waitFor(() => {
      expect(screen.getByText("Willkommen bei Relay")).toBeTruthy();
    });

    expect(screen.getByText("Der intelligente, lokale E-Mail-Client.")).toBeTruthy();
    expect(screen.getByText("KI-Überwachung")).toBeTruthy();
    expect(screen.getByText("KI-Mail-Generierung")).toBeTruthy();
    expect(screen.getByText("Lokal & Sicher")).toBeTruthy();
  });

  it("by-passes Splash Screen if there is at least one account", async () => {
    const mockAccount = {
      id: 1,
      name: "Privat",
      username: "test@example.com",
      smtp_username: "test@example.com",
      imap_host: "imap.example.com",
      imap_port: 993,
      smtp_host: "smtp.example.com",
      smtp_port: 587,
      sender_name: "Test User",
      sender_email: "test@example.com",
      connected: true,
    };
    vi.mocked(tauri.listAccounts).mockResolvedValue([mockAccount]);
    vi.mocked(tauri.listImapFolders).mockResolvedValue([]);
    vi.mocked(tauri.fetchFromImap).mockResolvedValue([]);
    vi.mocked(tauri.fetchMessages).mockResolvedValue([]);

    render(Page);

    await waitFor(() => {
      // The app-container has the sidebar version info
      expect(screen.getByText("AImighty Relay 3.0")).toBeTruthy();
    });

    // Splash screen header should NOT be present
    expect(screen.queryByText("Willkommen bei Relay")).toBeNull();
  });
});
