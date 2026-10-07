<script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
  import { onDestroy, onMount } from "svelte";
  import { untrack } from "svelte";
  import { goto } from "$app/navigation";
  import MessageList from "$lib/components/MessageList.svelte";
  import FolderList from "$lib/components/FolderList.svelte";
  import Huelle from "$lib/components/Huelle.svelte";
  import { tabTitel } from "$lib/tabTitel";
  import AccountGroup from "$lib/components/AccountGroup.svelte";
  import PromptDialog from "$lib/components/PromptDialog.svelte";
  import ComposeWindow from "$lib/components/ComposeWindow.svelte";
  import ReplySuggestions from "$lib/components/ReplySuggestions.svelte";
  import AssistantFab from "$lib/components/AssistantFab.svelte";
  import ConfirmationDialog from "$lib/components/ConfirmationDialog.svelte";
  import Aufraeumen from "$lib/components/Aufraeumen.svelte";
  import Durchgehen from "$lib/components/Durchgehen.svelte";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import type { SymbolName } from "$lib/symbole";
  import SplashScreen from "$lib/components/SplashScreen.svelte";
  import ErrorBanner from "$lib/components/ErrorBanner.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import { t, lang, setLang, translate, localizeError } from "$lib/i18n";
  import { mailbox, getFolderCache, invalidateFolderCache, isFolderFresh, markFolderFetched, type Message } from "$lib/stores/mailbox";
  import { accounts, type AccountInfo } from "$lib/stores/accounts";
  import { assistantAction } from "$lib/stores/assistantAction";
import {
    fetchMessages, fetchMessageBody, markAsRead, markAsUnseen, markBatchAsRead, markBatchAsUnseen, sendMessage,
    listAccounts, listImapFolders, createLocalFolder, deleteFolder,
    deleteMessageCmd, moveMessageCmd, moveMessageCrossAccount, renameFolder, flagMessageCmd, urgentMessageCmd,
    getMoveToTrash, getUnreadCounts, discardDraft, searchMessages,
    triggerFolderSummaries, fetchAttachments, loadAttachmentContent, saveAttachment,
    openEventStream, type AttachmentInfo,
    getFollowups, createPlanFromSuggestion, parseCachedFollowups, type FollowupSuggestion,
    getUnsubscribeOffer, unsubscribe, type AbmeldeArt,
    moveMessagesBatch, STAPEL_MAX, getAufraeumen,
  } from "$lib/services/tauri";
  import { assistantCommand } from "$lib/stores/assistantCommand";
  import { isFollowupDone } from "$lib/utils/followupMemory";
  import { dataVersion } from "$lib/stores/invalidation";
  import { formatDate, extractEmail, extractEmails, extractName, replyAllRecipients, isSafeOpenUrl, isHtmlContent, extractHtmlFromMime, extractPlainFromMime, parseMimeWithWorker, type MailAttachment } from "$lib/utils/format";
  import { iconSVG, folderIconFor } from "$lib/icons";
  import type { MailChainEntry } from "$lib/types/mail";
  import { cacheBody, getCachedBody } from "$lib/offline/bodyCache";
  import { queueDraft, getQueuedDrafts, removeQueuedDraft } from "$lib/offline/draftQueue";
  import { isOnline, initOnlineListener } from "$lib/offline/online";
  import { isDark, tokenValue } from "$lib/stores/appearance";

  let listWidth = $state(380);
  let showCompose = $state(false);

  // ─── Responsive layout ────────────────────────────────────
  // Below `compact` the preview becomes a full-width overlay (shown only when a
  // message/compose is open); below `narrow` the reading pane tightens its
  // header. The folder column is the shell's (Huelle): on the desktop under
  // the five areas, on the phone a sheet opened from the header (RL-G2).
  let viewportWidth = $state(typeof window !== "undefined" ? window.innerWidth : 1440);
  let isCompact = $derived(viewportWidth <= 900);
  let isNarrow = $derived(viewportWidth <= 600);
  // The shell's folder sheet on the phone; a chosen folder closes it.
  let folderSheetOpen = $state(false);

  // Touch devices: context menus render as iOS-style bottom sheets.
  let isTouchDevice = $state(false);
  $effect(() => {
    if (typeof window === "undefined") return;
    try {
      isTouchDevice = window.matchMedia("(pointer: coarse)").matches;
    } catch {
      isTouchDevice = false;
    }
  });

  $effect(() => {
    if (typeof window === "undefined") return;
    let rafId: number | null = null;
    const onResize = () => {
      if (!rafId) {
        rafId = requestAnimationFrame(() => {
          viewportWidth = window.innerWidth;
          rafId = null;
        });
      }
    };
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      if (rafId) cancelAnimationFrame(rafId);
    };
  });

  $effect(() => {
    if (typeof window === "undefined") return;
    const handler = (e: MessageEvent) => {
      const data = e.data;
      if (data && typeof data === 'object' && data.type === 'open-url' && typeof data.url === 'string') {
        // Nur http(s) erlauben — blockt data:/javascript:/file:-Phishing-Vektoren
        // aus (sandboxed) Mail-HTML. (M2, Code-Review 2026-08-28)
        if (isSafeOpenUrl(data.url)) window.open(data.url, "_blank");
      }
      if (
        data && typeof data === 'object' && data.type === 'link-contextmenu' &&
        typeof data.url === 'string' && typeof data.x === 'number' && typeof data.y === 'number'
      ) {
        if (!isSafeOpenUrl(data.url)) return;
        // iframe-Viewport-Koordinaten → Seitenkoordinaten
        const frame = document.querySelector(".mail-iframe") as HTMLIFrameElement | null;
        const rect = frame?.getBoundingClientRect();
        const pos = clampMenuPosition(
          (rect?.left ?? 0) + data.x,
          (rect?.top ?? 0) + data.y,
          260,
          96,
        );
        linkMenu = { url: data.url, x: pos.x, y: pos.y };
      }
    };
    window.addEventListener('message', handler);
    return () => window.removeEventListener('message', handler);
  });

  // Listen for backend events (new mail, AI summaries, navigation/actions)
  // via the SSE event stream (replaces the Tauri event listeners).
  let loadingBodyUid = $state<number | null>(null);

  // Globaler AI-Assistent (Phase 4.5) — FAB + drawer live in <AssistantFab>.

  // AI-Followups v2 (Phase C) — automatisch im Hintergrund erkannt (einmal pro
  // Mail, gecacht) und als typisierte Vorschläge im Footer angezeigt. Klick auf
  // einen Chip erzeugt einen Plan (origin=mail_followup) und öffnet den Drawer.
  let followups = $state<FollowupSuggestion[]>([]);
  let followupsLoading = $state(false);
  let followupsError = $state<string | null>(null);
  // The background analysis failed (model unreachable): a quiet line, not a
  // red error — nobody asked for it (CI HB-ZUSTAND). Red stays for a click.
  let followupsUnavailable = $state(false);
  let followupsForUid = $state<number | null>(null);
  let followupsInFlight: number | null = null; // nicht reaktiv, nur Duplikat-Guard
  const followupsCache = new Map<number, FollowupSuggestion[]>();
  let followupPlanBusy = $state(false);

  // Bereits ausgeführte Vorschläge (gleiche Mail, gleiche Aktion) ausblenden —
  // die Erinnerung ist pro Message-UID + content-Fingerprint gespeichert.
  function filterDoneFollowups(uid: number, actions: FollowupSuggestion[]): FollowupSuggestion[] {
    return actions.filter((a) => !isFollowupDone(uid, a));
  }

  // Nach einer Plan-Ausführung (dataVersion-Bump durch den Drawer) die
  // Vorschläge der aktuell geöffneten Mail sofort neu filtern, damit der soeben
  // ausgeführte Chip verschwindet.
  $effect(() => {
    const unsub = dataVersion.subscribe(() => {
      const uid = followupsForUid;
      if (uid == null) return;
      const cached = followupsCache.get(uid);
      if (cached) {
        followups = filterDoneFollowups(uid, cached);
      }
    });
    return unsub;
  });

  // Automatische Erkennung: wenn eine Mail geoeffnet wird (und kein Compose),
  // prueft die KI im Hintergrund auf Termin-Anfragen + weitere Aktionen.
  $effect(() => {
    const uid = selectedMessage?.uid;
    if (uid == null || showCompose) return;
    followupsForUid = uid;
    if (followupsCache.has(uid)) {
      followups = filterDoneFollowups(uid, followupsCache.get(uid)!);
      followupsLoading = false;
      followupsError = null;
      followupsUnavailable = false;
      return;
    }
    // Instant path: the list payload may already carry pre-generated followup
    // actions (INBOX pre-gen). Parse them — no spinner, no roundtrip.
    const cachedActions = parseCachedFollowups(selectedMessage?.ai_followups);
    if (cachedActions) {
      followupsCache.set(uid, cachedActions);
      followups = filterDoneFollowups(uid, cachedActions);
      followupsLoading = false;
      followupsError = null;
      followupsUnavailable = false;
      return;
    }
    // Auf geladenen Mailtext warten, sonst body_preview als Fallback.
    if (loadingBodyUid === uid) {
      followupsLoading = true;
      followups = [];
      followupsError = null;
      followupsUnavailable = false;
      return;
    }
    if (followupsInFlight === uid) return;
    const msg = selectedMessage;
    if (!msg) return;
    followupsInFlight = uid;
    followupsLoading = true;
    followupsError = null;
    followupsUnavailable = false;
    followups = [];
    const body = parsedContent.text || msg.body_preview || "";
    getFollowups(msg.subject || "", msg.from || "", body, {
      accountId: selectedAccountId,
      uid,
      folder: selectedFolder,
    })
      .then((res) => {
        if (followupsForUid !== uid) return;
        followupsCache.set(uid, res.actions);
        followups = filterDoneFollowups(uid, res.actions);
      })
      .catch(() => {
        if (followupsForUid !== uid) return;
        followupsUnavailable = true;
        followups = [];
      })
      .finally(() => {
        if (followupsInFlight === uid) followupsInFlight = null;
        if (followupsForUid === uid) followupsLoading = false;
      });
  });

  $effect(() => {
    if (typeof window === "undefined") return;
    const es = openEventStream((event, payload) => {
      if (event === "navigate-to" && payload === "/settings") {
        goto("/settings");
        return;
      }
      if (event === "trigger-action" && payload === "new-mail") {
        handleNewMail();
        return;
      }
      if (event === "new-messages") {
        const [accountId, folderName] = payload as [number, string, number];
        // Sidebar badges track EVERY account — refresh on any new-mail event
        // (debounced: a backfill fires one event per fetched batch).
        scheduleUnreadRefresh();
        if (accountId === selectedAccountId) {
          if (folderName === selectedFolder) {
            // Debounce: if a batch of new-messages arrives within 2s, only
            // reload once. Prevents rapid successive loadFolder() calls that
            // race with handleSelectMessage().
            if (newMsgTimer !== null) clearTimeout(newMsgTimer);
            newMsgTimer = setTimeout(() => {
              newMsgTimer = null;
              // force: a push event means the server cache has new rows —
              // bypass the client freshness window.
              loadFolder(true);
            }, 2000);
          }
        }
        return;
      }
      if (event === "ai-summary-updated") {
        const [uid, accountId, summary, priority, folderName, fraudScore] = payload as [number, number, string, number | null, string | null, number | null];
        if (accountId === selectedAccountId) {
          const changes: Record<string, unknown> = {};
          if (summary) changes.ai_summary = summary;
          if (priority !== undefined && priority !== null) changes.ai_priority = priority;
          if (fraudScore !== undefined && fraudScore !== null) changes.ai_fraud_score = fraudScore;
          if (Object.keys(changes).length) {
            // The event carries the source folder name. updateMessage only
            // touches rows whose (account, folder, uid) matches the current
            // view — a summary computed for a different folder's uid must
            // never overwrite what the user is looking at.
            mailbox.updateMessage(uid, folderName ?? "", changes);
          }
        }
        return;
      }
    });

    return () => {
      if (es) es.close();
      if (newMsgTimer !== null) clearTimeout(newMsgTimer);
    };
  });

  // In compact mode the preview overlay is visible when a message is selected
  // or the compose window is open.
  let previewOpen = $derived(showCompose || $mailbox.lastClickedUid != null);

  function backToList() {
    // Close the compact preview overlay: clear selection / close compose.
    if (showCompose) { showCompose = false; }
    mailbox.clearSelection();
  }
  let composeMode = $state<"new" | "reply" | "forward">("new");
  let replySubject = $state("");
  let replyTo = $state("");
  let replyCc = $state("");
  let recipientName = $state("");
  let mailChain = $state<MailChainEntry[]>([]);
  // 0 = no account yet; set by initWithAccount. A default of 1 made the
  // page ask for account 1's mail before any account existed (500).
  let selectedAccountId = $state<number>(0);
  let senderName = $state("");
  // Assistant hand-off: a fresh compose pre-filled from the AI assistant.
  let assistantCompose = $state<{ to: string; subject: string; body: string } | null>(null);
  $effect(() => {
    const action = $assistantAction;
    if (!action) return;
    if (action.type === "open_compose") {
      assistantCompose = { to: action.to, subject: action.subject, body: action.body };
      composeMode = "new";
      showCompose = true;
    } else if (action.type === "search") {
      searchQuery = action.query;
      onSearchInput();
    }
    assistantAction.clear();
  });
  let replySuggestions = $state<string[]>([]);
  let accountList = $state<AccountInfo[]>([]);
  let selectedAccount = $derived(accountList.find(a => a.id === selectedAccountId) || (accountList.length > 0 ? accountList[0] : null));
  let initError = $state<string | null>(null);
  let initOk = $state(false);
  let folderNames = $state<string[]>([]);
  let localFolderNames = $state<Set<string>>(new Set());
  let folderRawNames = $state<Record<string, string>>({});
  let folderDelimiters = $state<Record<string, string>>({});
  // ── Unread INBOX counts per account (sidebar badges) ────────────────
  // One cheap endpoint returns every account's unread inbox count, so a
  // single refresh covers all badges. Refreshed on load, on new-mail events
  // (debounced — backfill fires them per batch), and after read/unread ops.
  let unreadByAccount = $state<Record<number, number>>({});
  let unreadTimer: ReturnType<typeof setTimeout> | null = null;
  async function refreshUnreadCounts() {
    try {
      unreadByAccount = await getUnreadCounts();
    } catch {
      // Non-critical: the next sync/event retries. Keep the last-known counts.
    }
  }
  function scheduleUnreadRefresh() {
    if (unreadTimer !== null) clearTimeout(unreadTimer);
    unreadTimer = setTimeout(() => {
      unreadTimer = null;
      refreshUnreadCounts();
    }, 1500);
  }
  // Per-account folder data so EVERY account group in the sidebar can render
  // its own folder tree independently (previously only the selected account
  // had folders, which made other accounts appear collapsed/unopenable).
  interface AccountFolders {
    names: string[];
    local: Set<string>;
    raw: Record<string, string>;
    delim: Record<string, string>;
  }
  let foldersByAccount = $state<Record<number, AccountFolders>>({});
  function getAccountFolders(accountId: number): AccountFolders {
    return foldersByAccount[accountId] ?? { names: [], local: new Set(), raw: {}, delim: {} };
  }
  function setAccountFolders(accountId: number, f: AccountFolders) {
    foldersByAccount = { ...foldersByAccount, [accountId]: f };
    if (accountId === selectedAccountId) {
      folderNames = f.names;
      localFolderNames = f.local;
      folderRawNames = f.raw;
      folderDelimiters = f.delim;
    }
    // Default: subfolders COLLAPSED on first run. Only when the user has no
    // persisted collapsed state yet, seed it with every folder that has
    // children (folders with subfolders start hidden; double-click expands).
    if (!collapsedFoldersMap[accountId]) {
      try {
        const raw = localStorage.getItem(`relay_folder_collapsed_${accountId}`);
        if (raw === null) {
          const tree = buildFolderTree(f.names, f.delim);
          const seeded = new Set<string>();
          const visit = (nodes: FolderNode[]) => {
            for (const n of nodes) {
              if (n.children.length > 0) {
                seeded.add(n.name);
                visit(n.children);
              }
            }
          };
          visit(tree.children);
          collapsedFoldersMap = { ...collapsedFoldersMap, [accountId]: seeded };
        } else {
          collapsedFoldersMap = { ...collapsedFoldersMap, [accountId]: new Set(JSON.parse(raw) as string[]) };
        }
      } catch { /* ignore */ }
    }
  }
  let selectedFolder = $state("INBOX");
  let showDeleteConfirm = $state(false);
  let showDeleteFolderConfirm = $state(false);
  let pendingDeleteFolder = $state<string | null>(null);
  let pendingDeleteUid = $state<number | null>(null);
  let isDeleting = $state(false);
  let moveToTrash = $state(true);
  let fetchLimit = $state(50);
  try { const v = localStorage.getItem("relay_fetch_limit"); if (v) fetchLimit = parseInt(v, 10) || 50; } catch {}
  let draftsFolderName = $state<string | null>(null);
let sentFolderName = $state<string | null>(null);
  // Spam and archive of the account (attribute \Junk / \Archive, else by
  // name); without one, "Junk"/"Archive" — the provider queue creates it.
  let junkFolderName = $state<string | null>(null);
  let archiveFolderName = $state<string | null>(null);
  const spamOrdner = () => junkFolderName ?? "Junk";
  const archivOrdner = () => archiveFolderName ?? "Archive";
  const SPAM_NAMEN = ["junk", "spam", "spamverdacht", "junk e-mail", "junk-e-mail", "junk email", "inbox.junk", "inbox.spam"];
  const ARCHIV_NAMEN = ["archive", "archiv", "inbox.archive", "inbox.archiv"];
  function istSpamOrdner(name: string): boolean {
    return name === junkFolderName || SPAM_NAMEN.includes(name.toLowerCase());
  }
  let draftUid = $state<number | null>(null);
  let draftTo = $state("");
  let draftCc = $state("");
  let draftSubject = $state("");
  let draftBody = $state("");
  let draftInitialAttachments = $state<{ filename: string; content: string; contentType: string; size: number }[]>([]);
  // Forward source message (lazy attachment content resolution at send time).
  let forwardSourceUid = $state<number | null>(null);
  let forwardSourceFolder = $state("");

  let showSplash = $state(false);

  async function handleSplashComplete(acct: AccountInfo) {
    const accts = await listAccounts();
    accounts.setAccounts(accts);
    await initWithAccount(acct);
    showSplash = false;
  }

  function initWithAccount(acct: any) {
    selectedAccountId = acct.id;
    accounts.selectAccount(acct.id);
    senderName = acct.sender_name || acct.name || "";

    // Restore cached folder list from localStorage so sidebar is instant.
    try {
      const cacheKey = `relay_folder_cache_${acct.id}`;
      const cached = localStorage.getItem(cacheKey);
      if (cached) {
        const parsed = JSON.parse(cached) as string[];
        if (Array.isArray(parsed) && parsed.length > 0) {
          const ordered = applySavedFolderOrder(acct.id, parsed);
          folderNames = ordered;
          setAccountFolders(acct.id, { names: ordered, local: new Set(), raw: {}, delim: {} });
        }
      }
    } catch { /* ignore stale cache */ }

    // Load collapsed folders for this account
    try {
      const raw = localStorage.getItem(`relay_folder_collapsed_${acct.id}`);
      if (raw) {
        collapsedFoldersMap = { ...collapsedFoldersMap, [acct.id]: new Set(JSON.parse(raw) as string[]) };
      }
    } catch { /* ignore */ }

    // Background: refresh folder list from IMAP (deferred to idle, 3 retries).
    // The sidebar already has the localStorage folder cache; the live IMAP
    // LIST (TLS handshake) is deferred so it doesn't compete with first paint.
    const doRefresh = () => refreshFoldersBackground(acct.id);
    if (typeof requestIdleCallback !== "undefined") {
      requestIdleCallback(doRefresh, { timeout: 5000 });
    } else {
      setTimeout(doRefresh, 3000);
    }
  }

  async function refreshFoldersBackground(accountId: number) {
    for (let attempt = 0; attempt < 3; attempt++) {
      try {
        const f = await listImapFolders(accountId);
        const seen = new Set<string>();
        const names: string[] = [];
        const rawMap: Record<string, string> = {};
        const delimMap: Record<string, string> = {};
        const localSet = new Set<string>();
        draftsFolderName = null;
        sentFolderName = null;
        junkFolderName = null;
        archiveFolderName = null;
        const draftFallbacks = ["drafts", "entwürfe", "inbox.drafts"];
        const sentFallbacks = ["sent", "sent messages", "gesendet", "inbox.sent", "inbox.gesendet"];
        for (const x of f) {
          if (x.tag === "noselect") continue;
          if (!x.name || typeof x.name !== "string" || x.name.length === 0) continue;
          const key = x.name.toLowerCase();
          if (seen.has(key)) continue;
          seen.add(key);
          names.push(x.name);
          rawMap[x.name] = x.raw_name || x.name;
          delimMap[x.name] = x.delimiter || ".";
          if ((x as { local_only?: boolean }).local_only) localSet.add(x.name);
          if (!draftsFolderName && (x.attributes?.some(a => a.includes("Drafts")) || draftFallbacks.includes(key))) {
            draftsFolderName = x.name;
          }
          if (!sentFolderName && (x.attributes?.some(a => a.includes("Sent")) || sentFallbacks.includes(key))) {
            sentFolderName = x.name;
          }
          if (!junkFolderName && (x.attributes?.some(a => a.includes("Junk")) || SPAM_NAMEN.includes(key))) {
            junkFolderName = x.name;
          }
          if (!archiveFolderName && (x.attributes?.some(a => a.includes("Archive")) || ARCHIV_NAMEN.includes(key))) {
            archiveFolderName = x.name;
          }
        }
        const orderedNames = applySavedFolderOrder(accountId, names);
        folderNames = orderedNames;
        folderRawNames = rawMap;
        folderDelimiters = delimMap;
        localFolderNames = localSet;
        const cacheKey = `relay_folder_cache_${accountId}`;
        localStorage.setItem(cacheKey, JSON.stringify(orderedNames));
        setAccountFolders(accountId, { names: orderedNames, local: localSet, raw: rawMap, delim: delimMap });
        return;
      } catch (e: unknown) {
        if (attempt < 2) {
          await new Promise(r => setTimeout(r, 1000 * (attempt + 1)));
        } else {
          console.warn("listImapFolders failed after retries, using cached list", e);
        }
      }
    }
  }

  // ─── Folder Customization (Rename/Hide) ──────────────────
  let customFolderNames = $state<Record<string, string>>({});
  let hiddenFolderNames = $state<string[]>([]);

  // Helper: per-account localStorage keys
  function getStoreKey(base: string): string {
    return `relay_${base}_${selectedAccountId}`;
  }

  // Apply the saved drag-reorder order (relay_folder_order_<acct>) on top of a
  // freshly fetched server list. New folders (not in the saved order) are
  // appended at the end, removed folders dropped. The sidebar renders from
  // `foldersByAccount`, so the order MUST be applied before setAccountFolders.
  function applySavedFolderOrder(accountId: number, names: string[]): string[] {
    try {
      const saved = localStorage.getItem(`relay_folder_order_${accountId}`);
      if (!saved) return names;
      const order = JSON.parse(saved) as string[];
      if (!Array.isArray(order)) return names;
      const ordered = order.filter((n) => names.includes(n));
      const remaining = names.filter((n) => !ordered.includes(n));
      return [...ordered, ...remaining];
    } catch {
      return names;
    }
  }

  // Parent path of a folder ("" for top-level), using its delimiter. IMAP
  // subfolders are stored as "<parent><delim><child>" full names, so the flat
  // folder list is NOT a linear sort order: reordering a child must only move
  // it relative to its own siblings, never across parents.
  function folderParent(name: string, delimMap: Record<string, string>): string {
    const delim = (delimMap[name] || "").length > 0 ? delimMap[name] : ".";
    const idx = name.lastIndexOf(delim);
    return idx > 0 ? name.slice(0, idx) : "";
  }

  // Tree-aware folder reorder: moves `source` to the position of `target`
  // within their shared sibling group (same parent). Returns null when the two
  // folders belong to different parents (cross-parent moves are not supported
  // by drag-drop) or when a folder is missing.
  function reorderFolderSiblings(
    names: string[],
    delimMap: Record<string, string>,
    source: string,
    target: string,
  ): string[] | null {
    const sourceParent = folderParent(source, delimMap);
    const targetParent = folderParent(target, delimMap);
    if (sourceParent !== targetParent) return null;

    // Sibling order within a parent is their relative order in the flat list.
    // Extract the sibling group, splice within it, then re-insert preserving
    // the position of every non-sibling folder.
    const siblings: string[] = [];
    const positions: number[] = [];
    for (let i = 0; i < names.length; i++) {
      if (folderParent(names[i], delimMap) === sourceParent) {
        siblings.push(names[i]);
        positions.push(i);
      }
    }
    const fromIdx = siblings.indexOf(source);
    const toIdx = siblings.indexOf(target);
    if (fromIdx < 0 || toIdx < 0) return null;

    const reorderedSiblings = [...siblings];
    const [moved] = reorderedSiblings.splice(fromIdx, 1);
    reorderedSiblings.splice(toIdx, 0, moved);

    const result = [...names];
    positions.forEach((pos, i) => {
      result[pos] = reorderedSiblings[i];
    });
    return result;
  }

  function loadFolderCustomization() {
    try {
      const names = localStorage.getItem(getStoreKey("folder_custom_names"));
      if (names) customFolderNames = JSON.parse(names);
    } catch (e) { console.warn("Failed to load custom folder names", e); }

    try {
      const hidden = localStorage.getItem(getStoreKey("hidden_folders"));
      if (hidden) hiddenFolderNames = JSON.parse(hidden);
    } catch (e) { console.warn("Failed to load hidden folders", e); }
  }

  loadFolderCustomization();

  // ─── Rename dialog (replaces window.prompt, unavailable in WKWebView) ──
  let showRenameDialog = $state(false);
  let renameOriginalName = $state<string | null>(null);
  let renameLeafValue = $state("");

  // ─── New local folder dialog ─────────────────────────────────
  let showNewFolderDialog = $state(false);
  let newFolderName = $state("");
  let newFolderParent = $state<string | null>(null);

  function openNewFolderDialog() {
    newFolderName = "";
    newFolderParent = null;
    showNewFolderDialog = true;
  }

  function cancelNewFolder() {
    showNewFolderDialog = false;
    newFolderParent = null;
  }

  async function confirmNewFolder(name: string) {
    showNewFolderDialog = false;
    const parent = newFolderParent;
    newFolderParent = null;
    const trimmed = name.trim();
    if (!trimmed || !selectedAccountId) return;
    try {
      // Sub-folders are created below the clicked folder using the IMAP
      // delimiter (local-only folders are still stored in our DB).
      const delim = parent && parent !== "INBOX" ? (folderDelimiters[parent] || ".") : ".";
      const fullName = parent && parent !== "INBOX" ? `${parent}${delim}${trimmed}` : trimmed;
      await createLocalFolder(selectedAccountId, fullName);
      await reloadFolders();
    } catch (e) {
      console.error("createLocalFolder failed", e);
    }
  }

  async function reloadFolders() {
    if (!selectedAccountId) return;
    try {
      const f = await listImapFolders(selectedAccountId);
      const seen = new Set<string>();
      const names: string[] = [];
      const rawMap: Record<string, string> = {};
      const delimMap: Record<string, string> = {};
      const localSet = new Set<string>();
      for (const x of f) {
        if (x.tag === "noselect") continue;
        if (!x.name || typeof x.name !== "string" || x.name.length === 0) continue;
        const key = x.name.toLowerCase();
        if (seen.has(key)) continue;
        seen.add(key);
        names.push(x.name);
        rawMap[x.name] = x.raw_name || x.name;
        delimMap[x.name] = x.delimiter || ".";
        if ((x as { local_only?: boolean }).local_only) localSet.add(x.name);
      }
      folderNames = names;
      folderRawNames = rawMap;
      folderDelimiters = delimMap;
      localFolderNames = localSet;
      setAccountFolders(selectedAccountId, { names, local: localSet, raw: rawMap, delim: delimMap });
      try {
        localStorage.setItem(`relay_folder_cache_${selectedAccountId}`, JSON.stringify(names));
      } catch { /* ignore */ }
    } catch (e) {
      console.warn("reloadFolders failed", e);
    }
  }

  function openRenameDialog(originalName: string) {
    const delim = folderDelimiters[originalName] || ".";
    const parts = originalName.split(delim);
    renameOriginalName = originalName;
    renameLeafValue = parts[parts.length - 1];
    showRenameDialog = true;
  }

  function cancelRename() {
    showRenameDialog = false;
    renameOriginalName = null;
    renameLeafValue = "";
  }

  async function confirmRename(newLeafName: string) {
    const originalName = renameOriginalName;
    showRenameDialog = false;
    renameOriginalName = null;
    renameLeafValue = "";
    if (!originalName) return;

    const delim = folderDelimiters[originalName] || ".";
    const parts = originalName.split(delim);
    const leafName = parts[parts.length - 1];
    const parentPath = parts.slice(0, parts.length - 1).join(delim);

    const trimmedLeaf = newLeafName.trim();
    if (!trimmedLeaf || trimmedLeaf === leafName) return;

    const newPath = parentPath ? `${parentPath}${delim}${trimmedLeaf}` : trimmedLeaf;
    try {
      const rawOld = folderRawNames[originalName] || originalName;
      const rawParentPath = parentPath ? (folderRawNames[parentPath] || parentPath) : "";
      const rawNewPath = rawParentPath ? `${rawParentPath}${delim}${trimmedLeaf}` : trimmedLeaf;

      await renameFolder(selectedAccountId, rawOld, rawNewPath);

      // Update local state (reassign so $derived visibleFolders recomputes)
      const idx = folderNames.indexOf(originalName);
      if (idx !== -1) {
        const nextNames = [...folderNames];
        nextNames[idx] = newPath;
        folderNames = nextNames;
        if (selectedFolder === originalName) selectedFolder = newPath;
      }
      // Migrate custom display name if one exists
      if (customFolderNames[originalName]) {
        const nextCustom = { ...customFolderNames };
        nextCustom[newPath] = nextCustom[originalName];
        delete nextCustom[originalName];
        customFolderNames = nextCustom;
        localStorage.setItem(getStoreKey("folder_custom_names"), JSON.stringify(customFolderNames));
      }

      // Migrate raw-name and delimiter maps to the new path
      const nextRaw = { ...folderRawNames };
      nextRaw[newPath] = rawNewPath;
      delete nextRaw[originalName];
      folderRawNames = nextRaw;

      if (folderDelimiters[originalName]) {
        const nextDelim = { ...folderDelimiters };
        nextDelim[newPath] = nextDelim[originalName];
        delete nextDelim[originalName];
        folderDelimiters = nextDelim;
      }

      // Persist updated folder order under the new path
      try {
        const saved = localStorage.getItem(getStoreKey("folder_order"));
        if (saved) {
          const order = (JSON.parse(saved) as string[]).map((n) => n === originalName ? newPath : n);
          localStorage.setItem(getStoreKey("folder_order"), JSON.stringify(order));
        }
      } catch { /* non-critical */ }

      // The sidebar renders from the per-account folder store — refresh it so
      // the rename is visible immediately (no manual reload required).
      setAccountFolders(selectedAccountId, {
        names: folderNames,
        local: localFolderNames,
        raw: folderRawNames,
        delim: folderDelimiters,
      });
    } catch (e: unknown) {
      mailbox.setError(translate("mail.renameFailed") + (e instanceof Error ? e.message : String(e)));
    }
  }

  // ─── Plain HTML context menus (replaces the Tauri native menus) ────────
  let folderCtxMenu = $state<{ x: number; y: number; folderName: string } | null>(null);
  interface MoveTarget { name: string; label: string; accountId: number; depth?: number; full?: string; }
  interface MoveSection { header: string | null; items: MoveTarget[]; }
  let moveMenu = $state<{ x: number; y: number; sections: MoveSection[] } | null>(null);

  // Eigenes Kontextmenü für Rechtsklick/Long-Press auf Links im Mail-iframe
  // (das native Menü schlägt dort fehl: "Link öffnen" navigiert das Sandbox-Frame).
  let linkMenu = $state<{ url: string; x: number; y: number } | null>(null);

  function openLinkInTab() {
    if (!linkMenu) return;
    window.open(linkMenu.url, "_blank", "noopener");
    linkMenu = null;
  }

  function openLinkInBrowser() {
    if (!linkMenu) return;
    // popup-Fenster: in der installierten PWA öffnet der Browser damit den
    // System-Standardbrowser (man verlässt Relay); im Tab ein echtes Neues Fenster.
    window.open(linkMenu.url, "_blank", "popup=yes,width=1100,height=800");
    linkMenu = null;
  }

  function closeLinkMenu() {
    linkMenu = null;
  }

  function closeMenus() {
    folderCtxMenu = null;
    moveMenu = null;
    linkMenu = null;
  }

  // Close any open context menu when the window loses focus.
  $effect(() => {
    if (typeof window === "undefined") return;
    const onBlur = () => closeMenus();
    window.addEventListener("blur", onBlur);
    return () => window.removeEventListener("blur", onBlur);
  });

  function clampMenuPosition(x: number, y: number, w: number, h: number): { x: number; y: number } {
    const vw = typeof window !== "undefined" ? window.innerWidth : w;
    const vh = typeof window !== "undefined" ? window.innerHeight : h;
    return {
      x: Math.max(4, Math.min(x, vw - w)),
      y: Math.max(4, Math.min(y, vh - h)),
    };
  }

  function handleFolderContextMenu(e: { clientX: number; clientY: number; preventDefault: () => void }, originalName: string) {
    e.preventDefault();
    const pos = clampMenuPosition(e.clientX, e.clientY, 220, 150);
    folderCtxMenu = { x: pos.x, y: pos.y, folderName: originalName };
  }

  // "Neuer Ordner" creates a LOCAL folder below the clicked folder
  // (for INBOX / top-level → a new top-level local folder).
  async function folderCtxNewSubFolder(parentName: string) {
    closeMenus();
    if (!selectedAccountId) return;
    newFolderParent = parentName;
    newFolderName = "";
    showNewFolderDialog = true;
  }

  async function folderCtxDeleteFolder(folderName: string) {
    closeMenus();
    if (!selectedAccountId) return;
    pendingDeleteFolder = folderName;
    showDeleteFolderConfirm = true;
  }

  async function confirmDeleteFolder() {
    const name = pendingDeleteFolder;
    showDeleteFolderConfirm = false;
    pendingDeleteFolder = null;
    if (!name || !selectedAccountId) return;
    try {
      await deleteFolder(selectedAccountId, name);
      await reloadFolders();
      if (selectedFolder === name) {
        selectedFolder = "INBOX";
        mailbox.setFolderId("INBOX");
      }
    } catch (e) {
      console.error("deleteFolder failed", e);
      mailbox.setError(translate("mail.deleteFolderFailed") + (e instanceof Error ? e.message : String(e)));
    }
  }

  function folderCtxResetName(originalName: string) {
    const nextCustom = { ...customFolderNames };
    delete nextCustom[originalName];
    customFolderNames = nextCustom;
    localStorage.setItem(getStoreKey("folder_custom_names"), JSON.stringify(customFolderNames));
    closeMenus();
  }

  function folderCtxHideFolder(originalName: string) {
    if (!hiddenFolderNames.includes(originalName)) {
      hiddenFolderNames = [...hiddenFolderNames, originalName];
      localStorage.setItem(getStoreKey("hidden_folders"), JSON.stringify(hiddenFolderNames));
      if (selectedFolder === originalName) {
        selectedFolder = "INBOX";
      }
    }
    closeMenus();
  }

  function folderCtxUnhideAll() {
    hiddenFolderNames = [];
    localStorage.removeItem(getStoreKey("hidden_folders"));
    closeMenus();
  }

  // The button-triggered "move selected to folder" menu (replaces the Tauri
  // menu). Targets are grouped by account: the current account first (no
  // header), then every other account under its name — enabling cross-account
  // moves for single mails and multi-selections alike. Nested folders appear
  // in tree order, indented, with the leaf name as label (full path in title)
  // so a deep Yahoo structure stays readable.
  function buildMoveSections(): MoveSection[] {
    const sections: MoveSection[] = [];
    // Known folders of an account: live per-account state, falling back to
    // the localStorage folder cache; INBOX is always available.
    const foldersOf = (accountId: number): string[] => {
      const known = getAccountFolders(accountId).names;
      if (known.length > 0) return known;
      try {
        const cached = JSON.parse(
          localStorage.getItem(`relay_folder_cache_${accountId}`) ?? "[]"
        ) as string[];
        if (Array.isArray(cached) && cached.length > 0) return cached;
      } catch { /* ignore */ }
      return ["INBOX"];
    };
    const itemsOf = (accountId: number): MoveTarget[] => {
      let names = foldersOf(accountId);
      const out: MoveTarget[] = [];
      // INBOX is excluded from the tree (sidebar renders it as fixed row) —
      // it must still be a move target. Use the account's own alias name.
      const inboxName = names.find((n) => isInboxAlias(n)) ?? "INBOX";
      if (!names.includes(inboxName)) names = [...names, inboxName];
      out.push({
        name: inboxName,
        accountId,
        label: customFolderNames[inboxName] || translate(translateFolder("INBOX")),
        depth: 0,
        full: inboxName,
      });
      const tree = buildFolderTree(names, getAccountFolders(accountId).delim);
      const walk = (nodes: FolderNode[], depth: number) => {
        for (const n of nodes) {
          out.push({ name: n.name, accountId, label: n.label || n.name, depth, full: n.name });
          walk(n.children, depth + 1);
        }
      };
      walk(tree.children, 1);
      return out;
    };
    const own = itemsOf(selectedAccountId).filter((t) => t.name !== selectedFolder);
    if (own.length > 0) sections.push({ header: null, items: own });
    for (const acct of accountList) {
      if (acct.id === selectedAccountId) continue;
      if (!acct.connected) continue;
      sections.push({ header: acct.name, items: itemsOf(acct.id) });
    }
    return sections;
  }

  async function moveSelectedToFolder(e: MouseEvent) {
    if (movingSelection) return;
    if ($mailbox.selectedUids.length === 0) return;

    const sections = buildMoveSections();
    if (sections.every((s) => s.items.length === 0)) return;

    const rect = (e.currentTarget as HTMLElement | null)?.getBoundingClientRect();
    const pos = clampMenuPosition(rect?.left ?? e.clientX, (rect?.bottom ?? e.clientY) + 4, 220, 320);
    moveMenu = { x: pos.x, y: pos.y, sections };
  }


  function getInitials(name: string): string {
    if (!name) return "@";
    return name.trim().split(/\s+/).map(n => n[0]).join("").toUpperCase().slice(0, 2);
  }

  // Folder tree — hierarchical structure (max 3 levels: root → level 1 → level 2)
  interface FolderNode {
    name: string;
    label: string;
    children: FolderNode[];
    local_only?: boolean;
  }

   function getLeafName(fullName: string, delimiter: string): string {
    const parts = fullName.split(delimiter);
    return parts[parts.length - 1];
  }

  // Check if a folder name is an INBOX alias (various languages)
  function isInboxAlias(name: string): boolean {
    const lower = name.toLowerCase().trim();
    return ["inbox", "posteingang", "e-mails", "bpostin", "postan", "mailbox", "mail"].includes(lower);
  }

  function buildFolderTree(names: string[], delimMap: Record<string, string>): FolderNode {
    const root: FolderNode = { name: "INBOX", label: "", children: [] };
    const level1Map = new Map<string, FolderNode>();

    // Per-folder delimiter: IMAP folders carry their provider delimiter
    // (GMX = "/"), while LOCAL folders (imported/migration targets) have no
    // delimiter and use "." as the hierarchy separator. Using one global
    // delimiter (e.g. INBOX's "/") would leave "Beta Tests.Ecovacs Goat"
    // flat — exactly the bug where the tree looks right briefly (empty delim
    // -> "." fallback) and then flattens once the sync loads the IMAP
    // delimiter.
    const delimFor = (name: string): string => {
      const d = delimMap[name];
      return d && d.length > 0 ? d : ".";
    };

    // Pass 1: register every top-level folder first, so a child like
    // "Beta Tests.Ecovacs Goat" ALWAYS finds its real parent — regardless
    // of the order in the list. Previously, if the parent appeared AFTER
    // its child, it was created twice (once synthetic with children, once
    // real), which made "Beta Tests.Ecovacs Goat" render on the same level
    // as "Beta Tests".
    for (const name of names) {
      const delimiter = delimFor(name);
      const lowerName = name.toLowerCase();
      const leafLower = getLeafName(name, delimiter).toLowerCase();
      if (isInboxAlias(lowerName) || isInboxAlias(leafLower) || hiddenFolderNames.includes(name)) continue;
      const parts = name.split(delimiter);
      if (parts.length === 1) {
        const leafName = getLeafName(name, delimiter);
        const label = customFolderNames[name] || customFolderNames[leafName] || translate(translateFolder(leafName));
        const node: FolderNode = { name, label, children: [], local_only: localFolderNames.has(name) };
        root.children.push(node);
        level1Map.set(lowerName, node);
      }
    }

    // Pass 2: attach children to their (now existing) parent.
    for (const name of names) {
      const delimiter = delimFor(name);
      const lowerName = name.toLowerCase();
      const leafLower = getLeafName(name, delimiter).toLowerCase();
      if (isInboxAlias(lowerName) || isInboxAlias(leafLower) || hiddenFolderNames.includes(name)) continue;
      const parts = name.split(delimiter);
      if (parts.length < 2) continue;
      const leafName = getLeafName(name, delimiter);
      const label = customFolderNames[name] || customFolderNames[leafName] || translate(translateFolder(leafName));
      const parentName = parts[0];
      let parent = level1Map.get(parentName.toLowerCase());
      if (!parent) {
        // Parent folder is not a standalone entry (e.g. only exists as a
        // prefix) — synthesize it so the hierarchy stays intact.
        const parentLeaf = getLeafName(parentName, delimiter);
        const parentLabel = customFolderNames[parentName] || customFolderNames[parentLeaf] || translate(translateFolder(parentLeaf));
        parent = { name: parentName, label: parentLabel, children: [] };
        root.children.push(parent);
        level1Map.set(parentName.toLowerCase(), parent);
      }
      parent.children.push({ name, label, children: [], local_only: localFolderNames.has(name) });
    }

    return root;
  }

  let folderTree = $derived(
    buildFolderTree(folderNames, folderDelimiters)
  );

  // Per-account folder tree so every account group renders independently.
  let folderTreesByAccount = $derived.by(() => {
    const out: Record<number, FolderNode> = {};
    for (const acct of accountList) {
      const f = getAccountFolders(acct.id);
      out[acct.id] = buildFolderTree(f.names, f.delim);
    }
    return out;
  });

  // Collapsed folders state — per account, persisted to localStorage
  let collapsedFoldersMap = $state<Record<number, Set<string>>>({});

  function getCollapsedForAccount(accountId: number): Set<string> {
    return collapsedFoldersMap[accountId] ?? new Set();
  }

  function setCollapsedForAccount(accountId: number, folders: Set<string>) {
    collapsedFoldersMap = { ...collapsedFoldersMap, [accountId]: folders };
    try {
      // Save to localStorage scoped by accountId
      localStorage.setItem(`relay_folder_collapsed_${accountId}`, JSON.stringify([...folders]));
    } catch { /* ignore */ }
  }

  function handleToggleFolder(accountId: number, folderName: string) {
    const current = getCollapsedForAccount(accountId);
    const next = new Set(current);
    if (next.has(folderName)) {
      next.delete(folderName);
    } else {
      next.add(folderName);
    }
    setCollapsedForAccount(accountId, next);
  }

  function handleFolderSelect(name: string) {
    selectedFolder = name;
    mailbox.setFolderId(name);
  }

  // Select folder from a specific account
  function handleAccountFolderSelect(accountId: number, folder: string) {
    ansicht = "liste";
    if (accountId !== selectedAccountId) {
      const acct = accountList.find(a => a.id === accountId);
      if (acct) {
        selectedAccountId = accountId;
        accounts.selectAccount(accountId);
        initWithAccount(acct);
      }
    }
    selectedFolder = folder;
    mailbox.setFolderId(folder);
    folderSheetOpen = false;
  }

  // Toggle account collapsed state (root level)
  function handleToggleCollapse(accountId: number) {
    const current = getCollapsedForAccount(accountId);
    const next = new Set(current);
    if (next.has("INBOX")) {
      next.delete("INBOX");
    } else {
      next.add("INBOX");
    }
    setCollapsedForAccount(accountId, next);
  }

  // Drop of a message (dragged from the message list) onto a sidebar folder.
  // Uses the raw IMAP path so nested folders and non-ASCII names resolve.
  function handleMoveMessage(uid: number, targetFolder: string, targetAccountId?: number) {
    const isCrossAccount = targetAccountId != null && targetAccountId !== selectedAccountId;
    if (!isCrossAccount && selectedFolder === targetFolder) return;
    if (isCrossAccount && targetAccountId != null) {
      // Cross-account move: raw IMAP names on BOTH sides — the source from the
      // current account's map, the target from the receiving account's map
      // (nested/non-ASCII folder paths differ between display and raw name).
      const rawSource = folderRawNames[selectedFolder] || selectedFolder;
      const targetRaw = getAccountFolders(targetAccountId).raw[targetFolder] || targetFolder;
      moveMessageCrossAccount(selectedAccountId, uid, rawSource, targetAccountId, targetRaw)
        .then(() => {
          invalidateFolderCache(selectedAccountId, selectedFolder);
          invalidateFolderCache(targetAccountId, targetFolder);
          loadFolder();
        })
        .catch((e) => {
          console.warn("Cross-Account-Verschieben fehlgeschlagen", e);
          mailbox.setError(translate("mail.moveFailed") + (e instanceof Error ? e.message : String(e)));
        });
      return;
    }
    const rawSource = folderRawNames[selectedFolder] || selectedFolder;
    const rawTarget = folderRawNames[targetFolder] || targetFolder;
    // Optimistic: drop the row from the visible list immediately; the server
    // applies the local DB move synchronously and syncs the provider in the
    // background, so a rollback is only ever cosmetic (loadFolder on error).
    mailbox.removeMessage(uid);
    moveMessageCmd(selectedAccountId, uid, selectedFolder, targetFolder, rawSource, rawTarget)
      .then(() => {
        invalidateFolderCache(selectedAccountId, selectedFolder);
        invalidateFolderCache(selectedAccountId, targetFolder);
        loadFolder();
      })
      .catch((e) => {
        console.warn("Verschieben fehlgeschlagen", e);
        mailbox.setError(translate("mail.moveFailed") + (e instanceof Error ? e.message : String(e)));
        loadFolder();
      });
  }

  // isHtmlContent, extractHtmlFromMime, extractPlainFromMime imported from $lib/utils/format

  let parsedContent = $state<{ html: string | null; text: string | null }>({ html: null, text: null });

  // Sync fallback: compute parsed content synchronously using regex
  function computeParsedContent(msg: Message | null): { html: string | null; text: string | null } {
    if (!msg) return { html: null, text: null };
    const html = msg.body_html || null;
    const txt = msg.body_text || null;

    // Only treat body_html as HTML when it really contains markup. Older sync
    // paths could store the plain-text body into body_html instead of NULL, and
    // rendering that raw text through the HTML branch loses the line breaks
    // (HTML collapses "\n" to whitespace → the mail shows as a single flow
    // paragraph). Falling back to the text branch keeps readable formatting.
    if (html && isHtmlContent(html)) return { html, text: txt };

    if (txt) {
      const parsedHtml = extractHtmlFromMime(txt);
      if (parsedHtml) {
        const parsedPlain = extractPlainFromMime(txt);
        return { html: parsedHtml, text: parsedPlain };
      }
      if (isHtmlContent(txt)) {
        return { html: txt, text: null };
      }
    }

    return { html: null, text: html || txt };
  }

  // Async worker-based parsing: uses mime-parser worker when available,
  // falls back to regex-based sync parsing
  $effect(() => {
    const msg = selectedMessage;
    if (!msg) {
      parsedContent = { html: null, text: null };
      return;
    }

    // Set initial content via sync regex fallback (fast path)
    const initial = computeParsedContent(msg);
    parsedContent = initial;

    // If there's body_text that looks like MIME, try the worker for better parsing
    const txt = msg.body_text;
    if (txt && (txt.includes("Content-Type:") || txt.includes("boundary="))) {
      let cancelled = false;
      // Capture the fallback values in the closure (don't re-read the shared
      // parsedContent, which may belong to a newer message by the time the
      // worker resolves) — prevents showing one mail's body under another.
      parseMimeWithWorker(txt).then((result) => {
        if (!cancelled) {
          parsedContent = {
            html: result.bodyHtml || initial.html,
            text: result.bodyText || initial.text,
          };
        }
      });
      return () => { cancelled = true; };
    }
  });

  // Escape text for safe HTML embedding.
  function escapeHtml(s: string): string {
    return s
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  // Convert a plain-text mail body into safe, readable HTML:
  // escapes everything, auto-links URLs and bare emails, styles quoted
  // reply lines (">") and preserves line structure.
  function textToSafeHtml(text: string): string {
    const lines = text.replace(/\r\n/g, "\n").split("\n");
    const urlRe = /(https?:\/\/[^\s<]+[^\s<.,;:!?)\]}'"])/g;
    const emailRe = /([A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,})/g;
    const htmlLines = lines.map((line) => {
      let esc = escapeHtml(line);
      esc = esc.replace(urlRe, (u) => `<a href="${u}" target="_blank" rel="noopener noreferrer">${u}</a>`);
      esc = esc.replace(emailRe, (m) => `<a href="mailto:${m}">${m}</a>`);
      const isQuote = /^\s*&gt;/.test(esc);
      return isQuote ? `<span class="quote">${esc}</span>` : esc;
    });
    return `<div class="plain">${htmlLines.join("\n")}</div>`;
  }

  // Builds the sandboxed iframe document for both HTML and plain-text mails,
  // so every message looks consistently styled and is well readable.
  let previewSrcdoc = $derived.by(() => {
    const html = parsedContent.html;
    const text = parsedContent.text;
    const isPlain = !html;
    const inner = html ?? (text ? textToSafeHtml(text) : null);
    if (!inner) return null;

    // The iframe document cannot read CSS variables, so the CI token values
    // are resolved here; reading $isDark re-renders it when the mode flips.
    const isDarkMode = $isDark;
    const bg = tokenValue("--am-seite") || (isDarkMode ? "#051729" : "#ffffff");
    const fg = tokenValue("--am-text-primaer") || (isDarkMode ? "#eef2f6" : "#051729");
    const muted = tokenValue("--am-text-gedaempft") || (isDarkMode ? "#afc0d2" : "#4f6c8a");
    const linkColor = tokenValue("--am-handlung-ruhend") || (isDarkMode ? "#caa960" : "#002f56");
    const quoteBar = tokenValue("--am-rand") || (isDarkMode ? "#142e47" : "#cfdbe7");

    // Check auto-download images setting
    const autoDownload = localStorage.getItem("relay_auto_download_images") !== "false";

    // Defense in depth: the iframe is already sandboxed (no scripts, no
    // same-origin). This CSP also forbids any script execution and active /
    // framed content inside the rendered email while allowing inline styles
    // and images, neutralising active content from untrusted senders.
    const imgSrc = autoDownload ? "https: http: data: cid:" : "data: cid:";
    const cspMeta =
      `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; ` +
      `img-src ${imgSrc}; style-src 'unsafe-inline'; font-src data:; ` +
      `script-src 'unsafe-inline'; object-src 'none'; frame-src 'none'; base-uri 'none'; form-action 'none'">`;

    // Placeholder styling for blocked images
    const phBg = tokenValue("--am-flaeche-2") || (isDarkMode ? "#142e47" : "#eff4f9");
    const phBorder = tokenValue("--am-rand") || (isDarkMode ? "#142e47" : "#cfdbe7");
    const phText = muted;
    const phHoverBg = tokenValue("--am-flaeche-3") || (isDarkMode ? "#142e47" : "#e3eaf3");

    const baseStyle = `
      ${cspMeta}
      <meta name="viewport" content="width=device-width, initial-scale=1">
      <style>
        html, body { margin: 0; padding: 0; }
        body {
          font-family: "Geist", sans-serif;
          font-size: 15px;
          line-height: 1.65;
          color: ${fg};
          padding: 4px 2px 24px;
          word-break: break-word;
          overflow-wrap: anywhere;
          background-color: ${bg};
          -webkit-text-size-adjust: 100%;
        }
        a { color: ${linkColor}; text-decoration: none; }
        a:hover { text-decoration: underline; }
        img { max-width: 100%; height: auto; }
        table { max-width: 100%; border-collapse: collapse; }
        pre, code { white-space: pre-wrap; word-break: break-word; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 0.9em; }
        blockquote { margin: 0 0 0 4px; padding: 2px 0 2px 14px; border-left: 3px solid ${quoteBar}; color: ${muted}; }
        /* Plain-text rendering */
        .plain { white-space: pre-wrap; }
        .plain .quote { color: ${muted}; }
        /* Make wide HTML mails fit instead of overflowing */
        * { max-width: 100%; }
        ${!autoDownload ? `
        /* Image placeholder */
        .img-placeholder {
          display: inline-block;
          background: ${phBg};
          border: 1px solid ${phBorder};
          border-radius: 6px;
          padding: 12px 16px;
          margin: 4px 0;
          cursor: pointer;
          font-size: 13px;
          color: ${phText};
          text-align: center;
          min-width: 120px;
          max-width: 280px;
          transition: background 0.15s ease;
        }
        .img-placeholder:hover {
          background: ${phHoverBg};
        }` : ''}
      </style>
    `;

    // Replace external <img> tags with placeholders when auto-download is off
    let processed = inner;
    if (!autoDownload && !isPlain) {
      const imgRe = new RegExp('<img([^>]*)src=([\\x22\\x27])(https?://[^\\x22\\x27]+)\\2([^>]*)>', 'gi');
      processed = inner.replace(imgRe, (_m, before, _q, url, after) => {
        const safeUrl = url.replace(/&/g, '&amp;').replace(/"/g, '&quot;');
        const shortUrl = url.length > 60 ? url.slice(0, 57) + '...' : url;
        return `<span class="img-placeholder" data-src="${safeUrl}" onclick="loadImage(this)">${iconSVG("image")} ${shortUrl}<br><small>${translate('mail.loadImage')}</small></span>`;
      });
    }

    // Inject inline script for click-to-load when auto-download is off
    const loadScript = !autoDownload
      ? '<' + 'script>function loadImage(el){var u=el.dataset.src;var d=document.createElement(\'img\');d.src=u;d.style.maxWidth=\'100%\';d.style.height=\'auto\';el.replaceWith(d);}</' + 'script>'
      : '';

    // Inject link handlers: left-click opens via the parent (system browser);
    // right-click/long-press suppresses the broken native menu (its "Link
    // öffnen" navigates this sandboxed frame and dies on X-Frame-Options) and
    // asks the parent to show Relay's own link menu.
    const linkScript =
      '<' + 'script>' +
      'function findA(t){while(t&&t.nodeName!==\'A\'){t=t.parentElement}return t&&t.href&&t.hostname?t:null}' +
      'document.addEventListener(\'click\',function(e){var a=findA(e.target);if(a){e.preventDefault();parent.postMessage({type:\'open-url\',url:a.href},\'*\')}});' +
      'document.addEventListener(\'contextmenu\',function(e){var a=findA(e.target);if(a){e.preventDefault();parent.postMessage({type:\'link-contextmenu\',url:a.href,x:e.clientX,y:e.clientY},\'*\')}});' +
      '</' + 'script>';

    if (!isPlain && processed.includes("<head>")) {
      return processed.replace("<head>", `<head>${baseStyle}${loadScript}${linkScript}`);
    }
    return baseStyle + loadScript + linkScript + processed;
  });
  let sendError = $state<string | null>(null);
  let dragSource = $state<string | null>(null);
  let dragTarget = $state<string | null>(null);
  let queuedDrop: (() => void) | null = null;

  // Tracks active drag/resize AbortControllers so they are guaranteed to be
  // torn down on component unmount (prevents leaked document listeners when
  // the mouse is released outside the window — common on macOS).
  let activeDragControllers = new Set<AbortController>();

  // The one resize handle left on the page: between list and reading pane.
  // The column is the shell's and keeps its fixed 240 px (AM-HUELLE).
  function startResize(e: MouseEvent) {
    e.preventDefault();
    const startX = e.clientX;
    const startW = listWidth;
    const ac = new AbortController();
    activeDragControllers.add(ac);
    const { signal } = ac;
    function finish() {
      activeDragControllers.delete(ac);
      ac.abort();
    }
    function onMove(ev: MouseEvent) {
      const dx = ev.clientX - startX;
      listWidth = Math.max(200, Math.min(700, startW + dx));
    }
    document.addEventListener('mousemove', onMove, { signal });
    document.addEventListener('mouseup', finish, { signal });
    // Safety net: release if the pointer leaves the window or it loses focus.
    window.addEventListener('blur', finish, { signal });
  }

  function commitDrop() {
    if (queuedDrop) {
      queuedDrop();
      queuedDrop = null;
    }
  }

  function handleFolderMouseDown(e: MouseEvent, folderName: string) {
    if (e.button !== 0) return;
    const startX = e.clientX;
    const startY = e.clientY;
    let moved = false;
    const ac = new AbortController();
    activeDragControllers.add(ac);
    const { signal } = ac;

    function onMove(ev: MouseEvent) {
      if (!moved && (Math.abs(ev.clientX - startX) > 4 || Math.abs(ev.clientY - startY) > 4)) {
        moved = true;
        dragSource = folderName;
      }
      if (moved) {
        const el = document.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null;
        const folderEl = el?.closest("[data-folder]") as HTMLElement | null;
        dragTarget = folderEl?.dataset.folder ?? null;
      }
    }

    function onUp() {
      activeDragControllers.delete(ac);
      ac.abort();
      requestAnimationFrame(() => {
        if (moved && dragSource && dragTarget && dragSource !== dragTarget) {
          const reordered = reorderFolderSiblings(folderNames, folderDelimiters, dragSource, dragTarget);
          if (reordered) {
            folderNames = reordered;
            localStorage.setItem(getStoreKey("folder_order"), JSON.stringify(reordered));
            // The sidebar renders from the per-account store — keep it in sync
            // so the reorder is visible immediately.
            setAccountFolders(selectedAccountId, {
              names: folderNames,
              local: localFolderNames,
              raw: folderRawNames,
              delim: folderDelimiters,
            });
          }
        }
        dragSource = null;
        dragTarget = null;
      });
    }

    document.addEventListener("mousemove", onMove, { signal });
    document.addEventListener("mouseup", onUp, { signal });
    window.addEventListener("blur", onUp, { signal });
  }

  function handleDragStart(e: DragEvent, uid: number) {
    queuedDrop = null;
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", uid.toString());
      // Compact drag image: sender + subject with wrap
      const msg = $mailbox.messages.find(m => m.uid === uid);
      if (msg) {
        const ghost = document.createElement("div");
        ghost.style.cssText = `position:absolute;left:-9999px;width:200px;padding:6px 10px;background:var(--am-seite);color:var(--am-text-primaer);border-radius:8px;font-size:12px;box-shadow:none;line-height:1.4;`;
        const sender = document.createElement("div");
        sender.style.cssText = "font-weight:600;margin-bottom:2px;";
        sender.textContent = extractName(msg.from) || translate("mail.unknown");
        const subject = document.createElement("div");
        subject.style.cssText = "font-weight:400;color:var(--am-text-gedaempft);overflow:hidden;text-overflow:ellipsis;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;";
        subject.textContent = msg.subject || translate("mail.noSubject");
        ghost.appendChild(sender);
        ghost.appendChild(subject);
        document.body.appendChild(ghost);
        e.dataTransfer.setDragImage(ghost, 0, 0);
        setTimeout(() => ghost.remove(), 0);
      }
    }
    dragSource = uid.toString();
  }

  $effect(() => {
    const unsub = accounts.subscribe((v) => {
      if (v.selectedId) selectedAccountId = v.selectedId;
      accountList = v.accounts;
    });
    return unsub;
  });

  // Abort any in-flight drag/resize listeners when the page unmounts.
  $effect(() => {
    return () => {
      for (const ac of activeDragControllers) ac.abort();
      activeDragControllers.clear();
    };
  });

  // ─── Full-text search ─────────────────────────────────────
  let searchQuery = $state("");
  let searchActive = $state(false);
  let searchSeq = 0;
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  // Flag filter: the star toggle in the list head toggles the is:flagged
  // operator (RL-G1: a filter of the list, not part of the search field). Active state derives from the query text so manual typing of
  // `is:flagged` lights the star up as well.
  let flaggedSearchActive = $derived(
    ["is:flagged", "is:flag"].includes(searchQuery.trim().toLowerCase())
  );

  function toggleFlagFilter() {
    if (flaggedSearchActive) {
      clearSearch();
      return;
    }
    searchQuery = "is:flagged";
    searchSeen = searchQuery; // ran right here; the header effect must not run it again
    runSearch();
  }

  function runSearch() {
    const q = searchQuery.trim();
    if (q.length < 2) {
      // Too short → leave/return to the folder view.
      if (searchActive) { searchActive = false; loadFolder(); }
      return;
    }
    searchActive = true;
    const seq = ++searchSeq;
    mailbox.setLoading(true);
    searchMessages(selectedAccountId, q, 200)
      .then((msgs) => {
        if (seq !== searchSeq) return; // stale result
        mailbox.setMessages(msgs);
      })
      .catch((e) => {
        if (seq !== searchSeq) return;
        mailbox.setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (seq === searchSeq) mailbox.setLoading(false);
      });
  }

  function onSearchInput() {
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(runSearch, 250);
  }

  function clearSearch() {
    if (searchTimer) clearTimeout(searchTimer);
    searchQuery = "";
    if (searchActive) {
      searchActive = false;
      loadFolder();
    }
  }

  $effect(() => {
    return () => { if (searchTimer) clearTimeout(searchTimer); };
  });

  // The search sits in the header now (CI HB-SUCHE, RL-G1). Typing runs the
  // debounced search as the field in the column did; emptying the field
  // (also Escape there) leaves the search at once, as Escape did before.
  let searchSeen = "";
  $effect(() => {
    const q = searchQuery;
    if (q === searchSeen) return;
    searchSeen = q;
    untrack(() => {
      if (q === "") clearSearch();
      else onSearchInput();
    });
  });

  // Reload when folder changes - fetch from IMAP then read cache.
  // A folder switch always exits search mode.
  $effect(() => {
    if (selectedFolder && selectedAccountId > 0) {
      searchActive = false;
      searchQuery = "";
      // untrack(): loadFolder() reads $mailbox.lastClickedUid internally; without
      // this, the effect depends on the mailbox store and its own setMessages()
      // notification re-triggers the effect → infinite reload loop (OOM).
      untrack(() => {
        // Sync the store's folderId with the UI-selected folder. On cold start
        // selectedFolder defaults to "INBOX" but the store initializes with
        // folderId="" — without this, selectedMessage stays null (its guard
        // requires folderId === messagesFolder, and setMessages() only sets
        // messagesFolder) and no mail can be opened until the user manually
        // switches folders. Done inside untrack() so it adds no dependency.
        if ($mailbox.folderId !== selectedFolder) mailbox.setFolderId(selectedFolder);
        loadFolder();
      });
    }
  });

  let loadingFolder = false;
  // Generation counter: incremented on each loadFolder() call so stale
  // setMessages() payloads can be ignored (prevents body_text wipe race
  // with handleSelectMessage).
  let folderGen = 0;
  // Guards handleSelectMessage() from being interrupted by a concurrent
  // loadFolder(). When true, loadFolder() still fetches data but skips
  // setMessages() so the selected message's body is not replaced.
  let selectingUid: number | null = null;
  // Debounce timer for new-messages-triggered reloads
  let newMsgTimer: ReturnType<typeof setTimeout> | null = null;

  // Transient connection errors (e.g. during startup before the IMAP client
  // has finished connecting) must NOT surface as a red banner — the periodic
  // sync reconnects and reloads automatically. Only show genuine errors.
  function isTransientConnError(msg: string): boolean {
    const m = msg.toLowerCase();
    return (
      m.includes("imap-client nicht gefunden") ||
      m.includes("nicht verbunden") ||
      m.includes("zeitüberschreitung") ||
      m.includes("timeout") ||
      m.includes("verbindung konnte nicht") ||
      m.includes("connection")
    );
  }

 async function loadFolder(force = false) {
    if (loadingFolder) return;
    const reqFolder = selectedFolder;
    const reqAccount = selectedAccountId;
    // Freshness window: a recently fetched folder is served purely from the
    // persistent cache — no network round-trip at all. Event-driven reloads
    // (new mail) pass force=true; mutations drop freshness via
    // invalidateFolderCache().
    if (!force && getFolderCache(reqAccount, reqFolder) && isFolderFresh(reqAccount, reqFolder)) {
      mailbox.setMessages(getFolderCache(reqAccount, reqFolder)!, reqFolder, reqAccount);
      return;
    }
    loadingFolder = true;
    folderGen++;
    // Snapshot the requested target. If the user switches folder/account while
    // we await, the results are stale and must be discarded; we then re-run for
    // the latest selection. This prevents showing the wrong folder's content.
    // Silent background refresh: when a cached list exists for this
    // (account, folder), render it instantly and refresh WITHOUT any visible
    // loading state — fresh rows swap in place via setMessages() (the body
    // merge keeps an open message intact). Only a cold open (no cache at all)
    // shows the skeleton and clears the error banner.
    const cachedMsgs = getFolderCache(reqAccount, reqFolder);
    const hasCache = !!cachedMsgs && cachedMsgs.length > 0;
    if (hasCache) {
      mailbox.setMessages(cachedMsgs!, reqFolder, reqAccount);
    } else {
      mailbox.setLoading(true);
      mailbox.setError(null);
    }
    const thisGen = folderGen;
    const prevLastClicked = $mailbox.lastClickedUid;
    try {
      // Read from cache only — the sync scheduler keeps the cache up-to-date
      // via periodic IMAP fetches. list_only omits body_text/body_html so a
      // 10k-message folder transfers as metadata-only JSON.
      const msgs = await fetchMessages(reqAccount, 10000, 0, reqFolder, true);
      // Re-check after the await: only apply if still the active selection.
      if (reqFolder !== selectedFolder || reqAccount !== selectedAccountId) return;
      // Always update the store — setMessages() preserves body_text/body_html
      // for existing messages via the folderId:uid key merge, so the selected
      // message's body is safe even during a concurrent handleSelectMessage().
      mailbox.setMessages(msgs, reqFolder);
      markFolderFetched(reqAccount, reqFolder);
      // Trigger background AI summaries for messages without one
      triggerFolderSummaries(reqAccount, reqFolder).catch(() => {});
      if (prevLastClicked != null && !msgs.some(m => m.uid === prevLastClicked)) {
        mailbox.clearSelection();
      }
      refreshUnreadCounts();
    } catch (e: unknown) {
      const errMsg = e instanceof Error ? e.message : String(e);
      // Silent refresh: with stale-but-visible data a failed background fetch
      // must not pop an error banner — the next sync/reload retries.
      if (!hasCache && !isTransientConnError(errMsg) && reqFolder === selectedFolder && reqAccount === selectedAccountId) {
        mailbox.setError(errMsg);
      }
    } finally {
      loadingFolder = false;
      mailbox.setLoading(false);
      // If the selection moved on while we were loading, load the new target.
      if (reqFolder !== selectedFolder || reqAccount !== selectedAccountId) {
        loadFolder();
      }
    }
  }

  onMount(async () => {
    // Olares-Desktop öffnet Apps ggf. mit `?pathto=<route>` (z.B. beim
    // Öffnen der App-Einstellungen) — Route direkt anspringen.
    try {
      const pathto = new URLSearchParams(window.location.search).get("pathto");
      if (pathto && pathto.startsWith("/")) {
        goto(pathto);
        return;
      }
    } catch { /* ignore */ }
    // Olares-Desktop (Electron) kann Navigation an die eingebettete Web-App
    // per postMessage senden (Menüpunkt "Relay → Einstellungen"). Bekannte
    // Payloads: { type: "navigate", path: "/settings" } /
    // { type: "navigate-to", path: "/settings" } / { navigate: "/settings" }.
    try {
      const handleNavMessage = (event: MessageEvent) => {
        if (!event.data || typeof event.data !== "object") return;
        const d = event.data as Record<string, unknown>;
        const path =
          (typeof d.path === "string" && d.path.startsWith("/") && d.path) ||
          (typeof d.navigate === "string" && d.navigate.startsWith("/") && d.navigate) ||
          (typeof d.url === "string" && d.url.startsWith("/") && d.url);
        if (!path) return;
        const t = typeof d.type === "string" ? d.type.toLowerCase() : "";
        if (t.includes("navigate") || t.includes("settings") || t === "") {
          if (path !== window.location.pathname) goto(path);
        }
      };
      window.addEventListener("message", handleNavMessage);
    } catch { /* ignore */ }
    let accts: AccountInfo[] = [];
    try {
      accts = await listAccounts();
    } catch (e: unknown) {
      console.error("[init] listAccounts fehlgeschlagen:", e);
    }
    accounts.setAccounts(accts);
    if (accts.length > 0) {
      // Init immediately from cache — the server-side scheduler keeps data fresh.
      // No need to wait for IMAP connection; the background folder refresh
      // and SSE events handle real-time updates.
      initWithAccount(accts[0]);
      // Background: poll for connection status (non-blocking, max 10s).
      // If not connected yet, the UI still shows cached data.
      if (!accts.some(a => a.connected)) {
        (async () => {
          for (let i = 0; i < 20; i++) {
            await new Promise(r => setTimeout(r, 500));
            try {
              const fresh = await listAccounts();
              accounts.setAccounts(fresh);
              if (fresh.some(a => a.connected)) return;
            } catch { /* ignore */ }
          }
        })();
      }
    } else {
      showSplash = true;
    }
    initOk = true;
    // Fire-and-forget: not needed for first paint
    getMoveToTrash().then(v => { moveToTrash = v; }).catch(() => {});
    getAufraeumen().then(v => { aufraeumenAn = v; }).catch(() => {});
    refreshUnreadCounts();

    // Offline support: listen for connectivity changes, sync queued drafts on reconnect
    initOnlineListener();
    window.addEventListener("online", async () => {
      const drafts = await getQueuedDrafts();
      for (const d of drafts) {
        try {
          await sendMessage(
            d.accountId, d.to, d.subject, d.bodyText, d.bodyHtml,
            d.inReplyTo, d.references, d.recipientEmail, d.cc, d.bcc,
            d.attachments?.map(a => ({ filename: a.filename, content: a.content, contentType: a.content_type })),
          );
          if (d.id != null) await removeQueuedDraft(d.id);
        } catch { /* keep in queue for next retry */ }
      }
      if (drafts.length > 0) loadInbox().catch(() => {});
    });
  });

  async function loadInbox() {
    if (selectedAccountId <= 0) return;
    mailbox.setLoading(true);
    try {
      const msgs = await fetchMessages(selectedAccountId, 10000, 0, selectedFolder, true);
      mailbox.setMessages(msgs, selectedFolder);
      refreshUnreadCounts();
    } catch (e: unknown) {
      const errMsg = e instanceof Error ? e.message : String(e);
      if (!isTransientConnError(errMsg)) {
        mailbox.setError(errMsg);
      }
    }
  }

  async function retryInit() {
    initError = null;
    try {
      const accts = await listAccounts();
      accounts.setAccounts(accts);
      if (accts.length > 0) {
        await initWithAccount(accts[0]);
      }
    } catch (e: unknown) {
      const errMsg = e instanceof Error ? e.message : String(e);
      if (!errMsg.includes("IMAP-Client nicht gefunden")) {
        initError = translate("mail.startError") + errMsg;
      }
    }
  }

  let lastClickedUid = $state<number | null>(null);

  function handleSelectToggle(uid: number) {
    mailbox.toggleSelect(uid);
  }

  function handleSelectRange(fromIdx: number, toIdx: number) {
    mailbox.selectRange(fromIdx, toIdx, $mailbox.messages);
  }

  async function handleSelectMessage(uid: number) {
    mailbox.selectSingle(uid);
    lastClickedUid = uid;
    loadingBodyUid = uid;
    selectingUid = uid;

    // If we're in the Drafts folder, open ComposeWindow pre-filled
    if (selectedFolder === draftsFolderName) {
      try {
        const full = await fetchMessageBody(selectedAccountId, uid, draftsFolderName);
        if (lastClickedUid !== uid) return;
        draftUid = uid;
        draftTo = full.to || "";
        draftCc = full.cc || "";
        draftSubject = full.subject || "";
        draftBody = full.body_text || full.body_preview || "";
        draftInitialAttachments = (full.attachments ?? []).map((a: any) => ({
          filename: a.filename,
          content: a.content ?? "",
          contentType: a.content_type,
          size: a.size ?? 0,
        }));
        composeMode = "new";
        sendError = null;
        replyTo = "";
        replyCc = "";
        recipientName = "";
        replySubject = "";
        mailChain = [];
        showCompose = true;
        mailbox.updateMessage(uid, $mailbox.folderId, { is_read: true, body_text: full.body_text, body_html: full.body_html });
        loadingBodyUid = null;
        selectingUid = null;
      } catch (e: unknown) {
        console.warn("Draft laden fehlgeschlagen für uid", uid, e);
        loadingBodyUid = null;
        selectingUid = null;
      }
      return;
    }

    // Short-circuit: if the body is already in the in-memory store, skip the API call.
    const existing = $mailbox.messages.find(m => m.uid === uid);
    if (existing?.body_text && existing.body_text.trim().length > 0) {
      markAsRead(selectedAccountId, uid, selectedFolder).catch(() => {});
      mailbox.updateMessage(uid, $mailbox.folderId, { is_read: true });
      loadingBodyUid = null;
      selectingUid = null;
      return;
    }

    // Optimistic: show the preview immediately while the full body loads.
    if (existing?.body_preview) {
      mailbox.updateMessage(uid, $mailbox.folderId, { body_text: existing.body_preview, is_read: true });
    }

    try {
      markAsRead(selectedAccountId, uid, selectedFolder).catch(() => {});
      if (lastClickedUid !== uid) return;
      const full = await fetchMessageBody(selectedAccountId, uid, selectedFolder);
      if (lastClickedUid !== uid) return;
      mailbox.updateMessage(uid, $mailbox.folderId, { is_read: true, body_text: full.body_text, body_html: full.body_html });
      cacheBody(selectedAccountId, selectedFolder, uid, {
        body_text: full.body_text ?? "", body_html: full.body_html,
        subject: full.subject, from: full.from, to: full.to, cc: full.cc,
        date: full.date, flags: full.flags,
      }).catch(() => {});
      loadingBodyUid = null;
      selectingUid = null;
      refreshUnreadCounts();
    } catch (e: unknown) {
      // Offline fallback: try IndexedDB cache
      const cached = await getCachedBody(selectedAccountId, selectedFolder, uid);
      if (cached) {
        mailbox.updateMessage(uid, $mailbox.folderId, { is_read: true, body_text: cached.body_text, body_html: cached.body_html });
      }
      loadingBodyUid = null;
      selectingUid = null;
    }
  }

  function handleNewMail() {
    composeMode = "new";
    sendError = null;
    replyTo = "";
    replyCc = "";
    recipientName = "";
    replySubject = "";
    mailChain = [];
    draftUid = null;
    draftTo = "";
    draftCc = "";
    draftSubject = "";
    draftBody = "";
    draftInitialAttachments = [];
    showCompose = true;
  }

  // "Antworten" and "Allen antworten" stand at the mail (CI ABGLEICH RL-B1,
  // Kai 06.10.2026); the question with three buttons is gone.
  function handleReply(msg: Message, replyAll = false) {
    void doHandleReply(msg, replyAll);
  }

  /** The mail went to more than one person: To has several, or there is a CC. */
  function hasSeveralRecipients(msg: Message): boolean {
    const toList = (msg.to ?? "").split(",").map((s) => s.trim()).filter(Boolean);
    const ccList = (msg.cc ?? "").split(",").map((s) => s.trim()).filter(Boolean);
    return toList.length > 1 || ccList.length > 0;
  }

  async function doHandleReply(msg: Message, replyAll: boolean) {
    composeMode = "reply";
    sendError = null;
    replySubject = msg.subject ?? "";
    // Reply-All (Standard-Semantik): `to` = Absender + Original-To, `cc` =
    // Original-CC — jeweils ohne die eigene Adresse. (H2, Code-Review 2026-08-28;
    // 26.9.143 To/CC-Trennung)
    const replyAllRecips = replyAllRecipients(msg.from ?? "", msg.to ?? "", msg.cc ?? "", selectedAccount?.sender_email);
    replyTo = replyAll ? replyAllRecips.to.join(", ") : extractEmail(msg.from ?? "");
    replyCc = replyAll ? replyAllRecips.cc.join(", ") : "";
    recipientName = extractName(msg.from ?? "");
    showCompose = true;

    // If the full body hasn't been loaded yet (e.g. user clicked reply
    // immediately after selecting a non-INBOX message), fetch it before
    // building the mail chain. Otherwise the reply dialog permanently
    // truncates to the 200-char body_preview.
    let bodyText = msg.body_text;
    let bodyHtml = msg.body_html;
    if (!bodyText && !bodyHtml) {
      try {
        const full = await fetchMessageBody(selectedAccountId, msg.uid, selectedFolder);
        bodyText = full.body_text;
        bodyHtml = full.body_html;
      } catch (e) {
        console.warn("handleReply: body fetch failed, falling back to preview", e);
      }
    }

    const text = parsedContent.text || bodyText || msg.body_preview || "";
    const html = parsedContent.html || bodyHtml || null;
    if (text) {
      mailChain = [{ text, html }];
    } else {
      mailChain = [];
    }
  }

  function handleReplyMessage(uid: number) {
    const msg = $mailbox.messages.find((m) => m.uid === uid);
    if (msg) handleReply(msg);
  }

  async function handleForward(msg: Message) {
    composeMode = "forward";
    sendError = null;
    replySubject = msg.subject ?? "";
    replyTo = "";
    replyCc = "";
    recipientName = "";
    mailChain = [];
    draftUid = null;
    draftTo = "";
    draftSubject = "";
    draftBody = "";
    draftInitialAttachments = [];
    forwardSourceUid = msg.uid;
    forwardSourceFolder = selectedFolder;
    showCompose = true;

    // Fetch the full body if not already loaded (same as reply).
    let bodyText = msg.body_text;
    let bodyHtml = msg.body_html;
    if (!bodyText && !bodyHtml) {
      try {
        const full = await fetchMessageBody(selectedAccountId, msg.uid, selectedFolder);
        bodyText = full.body_text;
        bodyHtml = full.body_html;
      } catch (e) {
        console.warn("handleForward: body fetch failed, falling back to preview", e);
      }
    }

    const text = parsedContent.text || bodyText || msg.body_preview || "";
    const html = parsedContent.html || bodyHtml || null;
    if (text) {
      mailChain = [{ text, html }];
    } else {
      mailChain = [];
    }

    // Pre-fill attachment PILLS with metadata only (lazy content): the file
    // contents are fetched per attachment on demand when the mail is sent
    // (handleSend resolves missing content via loadAttachmentContent). This
    // keeps the forward lightweight even for large attachments.
    if (msg.has_attachments) {
      try {
        const atts = await fetchAttachments(selectedAccountId, msg.uid, selectedFolder);
        draftInitialAttachments = atts.map((a) => ({
          id: a.id,
          filename: a.filename,
          content: "",
          contentType: a.content_type,
          size: a.size,
        }));
      } catch (e) {
        console.warn("handleForward: attachment metadata fetch failed", e);
      }
    }
  }

  function handleForwardMessage(uid: number) {
    const msg = $mailbox.messages.find((m) => m.uid === uid);
    if (msg) handleForward(msg);
  }

  function closeCompose() {
    showCompose = false;
    // Keep draftUid so a later "save again" still updates the same draft, but
    // clear the pre-fill fields so a fresh compose doesn't resurrect stale text.
    draftTo = "";
    draftSubject = "";
    draftBody = "";
    draftInitialAttachments = [];
    assistantCompose = null;
    forwardSourceUid = null;
    forwardSourceFolder = "";
  }

  function isInputFocused(): boolean {
    const tag = (document.activeElement?.tagName || "").toUpperCase();
    const editable = document.activeElement?.getAttribute("contenteditable") === "true";
    return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || editable;
  }

  async function toggleReadStatus() {
    const uids = [...$mailbox.selectedUids];
    if (uids.length === 0) return;
    // Determine the desired state from the first selected message (they share
    // one toolbar action). Batch the request so large selections (e.g. whole
    // folders) don't fire one HTTP round-trip per message.
    const first = $mailbox.messages.find((m) => m.uid === uids[0]);
    const targetRead = first ? !first.is_read : false;
    try {
      if (targetRead) {
        await markBatchAsRead(selectedAccountId, uids, selectedFolder);
      } else {
        await markBatchAsUnseen(selectedAccountId, uids, selectedFolder);
      }
      for (const uid of uids) {
        mailbox.updateMessage(uid, $mailbox.folderId, { is_read: targetRead });
      }
    } catch (e) {
      console.warn("toggleReadStatus fehlgeschlagen", e);
    }
    refreshUnreadCounts();
  }

  // Mark all selected messages as read (toolbar action).
  async function markSelectedRead() {
    const uids = [...$mailbox.selectedUids];
    if (uids.length === 0) return;
    try {
      await markBatchAsRead(selectedAccountId, uids, selectedFolder);
      for (const uid of uids) {
        mailbox.updateMessage(uid, $mailbox.folderId, { is_read: true });
      }
    } catch (e) {
      console.warn("markSelectedRead fehlgeschlagen", e);
    }
    refreshUnreadCounts();
  }

  // Move all selected messages to a folder chosen from a plain HTML menu.
  let movingSelection = $state(false);

  async function performMoveSelected(uids: number[], targetFolder: string, targetAccountId?: number) {
    const isCrossAccount = targetAccountId != null && targetAccountId !== selectedAccountId;
    if (movingSelection || (!isCrossAccount && targetFolder === selectedFolder)) return;
    movingSelection = true;
    try {
      if (isCrossAccount && targetAccountId != null) {
        // Cross-account batch: raw IMAP names on both sides (source from the
        // current account's map, target from the receiving account's map).
        const rawSource = folderRawNames[selectedFolder] || selectedFolder;
        const rawTarget = getAccountFolders(targetAccountId).raw[targetFolder] || targetFolder;
        let failures = 0;
        for (const uid of uids) {
          try {
            await moveMessageCrossAccount(selectedAccountId, uid, rawSource, targetAccountId, rawTarget);
          } catch (e) {
            failures++;
            console.warn("Cross-Account-Verschieben fehlgeschlagen fuer uid", uid, e);
          }
        }
        if (failures > 0) {
          mailbox.setError(translate("mail.moveFailed") + translate("mail.moveBatchPartial", { count: String(failures) }));
        }
        invalidateFolderCache(targetAccountId, targetFolder);
      } else {
        const rawSource = folderRawNames[selectedFolder] || selectedFolder;
        const rawTarget = folderRawNames[targetFolder] || targetFolder;
        for (const uid of uids) {
          try {
            await moveMessageCmd(selectedAccountId, uid, selectedFolder, targetFolder, rawSource, rawTarget);
          } catch (e) {
            console.warn("Verschieben fehlgeschlagen fuer uid", uid, e);
          }
        }
        invalidateFolderCache(selectedAccountId, targetFolder);
      }
      mailbox.clearSelection();
      invalidateFolderCache(selectedAccountId, selectedFolder);
      await loadFolder();
    } finally {
      movingSelection = false;
    }
  }

  /** Open the grouped move menu anchored at an arbitrary point — used by the
   *  message context menu ("Verschieben…") so a single mail can be moved to
   *  any account's folder without drag & drop. */
  function openMoveMenuAt(x: number, y: number) {
    if (movingSelection) return;
    const sections = buildMoveSections();
    if (sections.every((s) => s.items.length === 0)) return;
    const pos = clampMenuPosition(x, y + 4, 220, 320);
    moveMenu = { x: pos.x, y: pos.y, sections };
  }

  function handleKeydown(e: KeyboardEvent) {
    // The clean-up views bring their own keys.
    if (ansicht !== "liste") return;
    // Escape: close context menus, compose or confirmation dialog, or clear multi-selection
    if (e.key === "Escape") {
      if (folderCtxMenu || moveMenu || linkMenu) {
        closeMenus();
        return;
      }
      if (showCompose) {
        closeCompose();
        return;
      }
      if (showDeleteConfirm) {
        cancelDelete();
        return;
      }
      if ($mailbox.selectedUids.length > 1) {
        mailbox.clearSelection();
        return;
      }
    }

    // Don't fire shortcuts when typing in input fields
    if (isInputFocused()) return;

    // Arrow navigation through the message list (↑/↓)
    if ((e.key === "ArrowDown" || e.key === "ArrowUp") && !showCompose) {
      const msgs = $mailbox.messages;
      if (msgs.length > 0) {
        e.preventDefault();
        const curUid = $mailbox.lastClickedUid;
        const curIdx = curUid != null ? msgs.findIndex((m) => m.uid === curUid) : -1;
        let nextIdx: number;
        if (curIdx === -1) {
          nextIdx = e.key === "ArrowDown" ? 0 : msgs.length - 1;
        } else {
          nextIdx = e.key === "ArrowDown"
            ? Math.min(curIdx + 1, msgs.length - 1)
            : Math.max(curIdx - 1, 0);
        }
        const next = msgs[nextIdx];
        if (next) handleSelectMessage(next.uid);
        return;
      }
    }

    // Ctrl/Cmd shortcuts (cross-platform)
    if (e.ctrlKey || e.metaKey) {
      switch (e.key.toLowerCase()) {
        case "a":
          e.preventDefault();
          mailbox.selectAll($mailbox.messages);
          return;
        case "r":
          e.preventDefault();
          loadFolder();
          return;
        case "n":
          e.preventDefault();
          handleNewMail();
          return;
        case ",":
          // macOS-Standard "Einstellungen…" (Cmd+,) — auch vom Olares-Desktop
          // als App-Menüpunkt "Relay → Einstellungen" ausgelöst.
          e.preventDefault();
          goto("/settings");
          return;
        case "i":
          if (e.shiftKey) {
            e.preventDefault();
            toggleReadStatus();
            return;
          }
          break;
      }
    }

    // E: archive, !: spam / not spam — the selection or the open mail.
    if (!e.ctrlKey && !e.metaKey && !e.altKey && !showCompose) {
      if (e.key === "e" || e.key === "E") {
        if (gemeinteUids().length > 0) { e.preventDefault(); archivieren(); }
        return;
      }
      if (e.key === "!") {
        if (gemeinteUids().length > 0) { e.preventDefault(); spamUmschalten(); }
        return;
      }
    }

    // Backspace / Delete: delete selected messages
    if ((e.key === "Backspace" || e.key === "Delete" || e.key === "Del") && $mailbox.selectedUids.length > 0) {
      e.preventDefault();
      handleDeleteSelected();
    }
  }

  let isSending = $state(false);

  async function handleSend(data: { to: string; subject: string; body: string; bodyHtml: string; cc?: string; bcc?: string; attachments?: { id?: number; filename: string; content: string; contentType: string }[]; aiDraft?: string | null }) {
    if (isSending) return;
    isSending = true;
    sendError = null;

    // Offline: queue as local draft, will sync on reconnect
    if (!navigator.onLine) {
      await queueDraft({
        accountId: selectedAccountId,
        to: data.to.split(",").map((s) => s.trim()),
        cc: data.cc ? data.cc.split(",").map((s) => s.trim()).filter(Boolean) : undefined,
        bcc: data.bcc ? data.bcc.split(",").map((s) => s.trim()).filter(Boolean) : undefined,
        subject: data.subject,
        bodyText: data.body,
        bodyHtml: data.bodyHtml,
        attachments: data.attachments?.map(a => ({ filename: a.filename, content: a.content ?? "", content_type: a.contentType, size: Math.ceil((a.content ?? "").length * 0.75) })),
      });
      showCompose = false;
      assistantCompose = null;
      isSending = false;
      return;
    }

    try {
      // Resolve lazy forward-attachment content (metadata-only pills) before
      // building the SMTP payload. Per-attachment, folder-scoped.
      let resolvedAttachments = data.attachments;
      if (data.attachments && data.attachments.some((a) => a.id != null && !a.content)) {
        resolvedAttachments = [];
        for (const a of data.attachments) {
          if (a.id != null && !a.content) {
            const content = await loadAttachmentContent(
              selectedAccountId, forwardSourceUid ?? selectedMessage!.uid, a.id, forwardSourceFolder || selectedFolder
            ).catch(() => "");
            resolvedAttachments.push({ ...a, content });
          } else {
            resolvedAttachments.push(a);
          }
        }
      }
      const recipientEmail = extractEmail(data.to) || data.to.split(",")[0]?.trim() || "";
      const result = await sendMessage(
        selectedAccountId,
        data.to.split(",").map((s) => s.trim()),
        data.subject,
        data.body,
        data.bodyHtml,
        undefined,
        undefined,
        recipientEmail,
        data.cc ? data.cc.split(",").map((s) => s.trim()).filter(Boolean) : undefined,
        data.bcc ? data.bcc.split(",").map((s) => s.trim()).filter(Boolean) : undefined,
        resolvedAttachments,
        data.aiDraft || undefined,
      );

      // Discard draft if we were editing one
      if (draftUid != null) {
        discardDraft(selectedAccountId, draftUid).catch((e: unknown) =>
          console.warn("Draft discard fehlgeschlagen", e)
        );
        draftUid = null;
      }

      // Show warning if sent copy couldn't be saved
      if (!result.sent_copy_saved) {
        console.warn("Mail gesendet, aber Kopie konnte nicht im Gesendet-Ordner gespeichert werden");
      }

      showCompose = false;
      assistantCompose = null;
      sendError = null;
      await loadInbox();
    } catch (e: unknown) {
      sendError = localizeError(e instanceof Error ? e.message : String(e));
    } finally {
      isSending = false;
    }
  }

  // The message list in the store always belongs to exactly one folder
  // (mailbox.messagesFolder). Guard that the UI-selected folder matches the
  // data currently in the store: during a folder switch the UI label changes
  // BEFORE the new list arrives, and uid is only unique per folder — showing
  // the stale row would display the previous folder's mail under the new one.
  let selectedMessage = $derived(
    $mailbox.lastClickedUid != null &&
      $mailbox.folderId === $mailbox.messagesFolder
      ? $mailbox.messages.find((msg) => msg.uid === $mailbox.lastClickedUid) ?? null
      : null
  );

  $effect(() => {
    if ($mailbox.lastClickedUid != null && selectedMessage === null) {
      mailbox.clearSelection();
    }
  });

  // ─── Attachments (Progressive Loading) ────────────────────
  let attachments = $state<AttachmentInfo[]>([]);
  let attachmentsLoading = $state(false);

  // Load cached attachment metadata immediately (no IMAP fetch).
  $effect(() => {
    const uid = selectedMessage?.uid;
    const acct = selectedAccountId;
    attachments = [];
    if (uid == null || showCompose) return;
    let cancelled = false;
    attachmentsLoading = true;
    fetchAttachments(acct, uid, selectedFolder)
        .then((cached) => {
        if (cancelled) return;
        attachments = cached;
      })
      .catch(() => { if (!cancelled) attachments = []; })
      .finally(() => { if (!cancelled) attachmentsLoading = false; });
    return () => { cancelled = true; };
  });

  function formatBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  }

  // ─── Attachment context menu (right-click: Open / Save as) ───────────
  let attCtxMenu = $state<{ x: number; y: number; att: AttachmentInfo } | null>(null);
  let attPreview = $state<{ url: string; filename: string; contentType: string } | null>(null);

  function handleAttachmentContextMenu(e: MouseEvent, att: AttachmentInfo) {
    e.preventDefault();
    attCtxMenu = { x: e.clientX, y: e.clientY, att };
  }

  function closeAttCtxMenu() {
    attCtxMenu = null;
  }

  async function ensureAttachmentContent(att: AttachmentInfo): Promise<string | null> {
    if (att.content) return att.content;
    try {
      const content = await loadAttachmentContent(selectedAccountId!, selectedMessage!.uid, att.id, selectedFolder);
      if (content) {
        attachments = attachments.map(a =>
          a.id === att.id ? { ...a, content, content_cached: true } : a
        );
      }
      return content;
    } catch {
      return null;
    }
  }

  async function handleOpenAttachment(att: AttachmentInfo) {
    closeAttCtxMenu();
    const content = await ensureAttachmentContent(att);
    if (!content) {
      mailbox.setError(translate("mail.attachmentUnavailable"));
      return;
    }
    try {
      const byteChars = atob(content);
      const bytes = new Uint8Array(byteChars.length);
      for (let i = 0; i < byteChars.length; i++) bytes[i] = byteChars.charCodeAt(i);
      const blob = new Blob([bytes], { type: att.content_type || "application/octet-stream" });
      const url = URL.createObjectURL(blob);
      attPreview = { url, filename: att.filename, contentType: att.content_type || "application/octet-stream" };
    } catch (e) {
      mailbox.setError(translate("mail.attachmentOpenFailed") + (e instanceof Error ? e.message : String(e)));
    }
  }

  function closeAttPreview() {
    if (attPreview) {
      URL.revokeObjectURL(attPreview.url);
      attPreview = null;
    }
  }

  function downloadAttPreview() {
    if (!attPreview) return;
    const a = document.createElement("a");
    a.href = attPreview.url;
    a.download = attPreview.filename;
    document.body.appendChild(a);
    a.click();
    a.remove();
  }

  async function handleSaveAsAttachment(att: AttachmentInfo) {
    closeAttCtxMenu();
    const content = await ensureAttachmentContent(att);
    if (!content) {
      mailbox.setError(translate("mail.attachmentUnavailable"));
      return;
    }
    const saved = await saveAttachment(att.filename, content, att.content_type || undefined);
    if (!saved) {
      mailbox.setError(translate("mail.attachmentSaveFailed"));
    }
  }

  $effect(() => {
    if (!attCtxMenu && !attPreview) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (attPreview) closeAttPreview();
        else closeAttCtxMenu();
      }
    };
    const onBlur = () => {
      if (attCtxMenu) closeAttCtxMenu();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onBlur);
    };
  });

  let pendingDeleteUids: number[] = $state([]);

  // Deleting (CI RL-R2): a mail that goes to the trash can be brought back,
  // so it asks nothing and offers "Rückgängig" instead. The server is only
  // called after UNDO_MS — undo then simply cancels the timer (the trash
  // would assign new UIDs, so moving back would be guesswork). Deleting from
  // the trash, or with the trash switched off, is final and asks first.
  const UNDO_MS = 5000;
  // Abo beenden (List-Unsubscribe): asked of the server per opened mail; the
  // button shows only when the sender offers it, never in the spam folder.
  let abmeldeAngebot = $state<{ uid: number; art: AbmeldeArt; ziel: string } | null>(null);
  let abmeldenFragen = $state(false);
  // "Mehr" in the reading pane's toolbar: what is rarer (reply to all).
  let mehrMenue = $state<{ x: number; y: number } | null>(null);
  let abmeldenLaeuft = $state(false);
  let abmeldeMeldung = $state<string | null>(null);
  let abmeldeMeldungTimer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    const msg = selectedMessage;
    const account = selectedAccountId;
    const folder = selectedFolder;
    abmeldeAngebot = null;
    if (!msg || showCompose || account <= 0) return;
    const uid = msg.uid;
    getUnsubscribeOffer(account, uid, folder)
      .then((a) => {
        if (a.art && selectedMessage?.uid === uid) abmeldeAngebot = { uid, art: a.art, ziel: a.ziel ?? "" };
      })
      .catch(() => { /* no button — nothing to tell */ });
  });

  function abmeldeMeldungZeigen(text: string) {
    abmeldeMeldung = text;
    if (abmeldeMeldungTimer) clearTimeout(abmeldeMeldungTimer);
    abmeldeMeldungTimer = setTimeout(() => (abmeldeMeldung = null), 6000);
  }

  async function aboBeenden() {
    const angebot = abmeldeAngebot;
    abmeldenFragen = false;
    if (!angebot || abmeldenLaeuft) return;
    abmeldenLaeuft = true;
    try {
      const r = await unsubscribe(selectedAccountId, angebot.uid, selectedFolder);
      if (r.art === "link" && r.url) {
        window.open(r.url, "_blank", "noopener,noreferrer");
      } else {
        // The server says where it actually went: a failed one click falls
        // back to the sender's unsubscribe address.
        const ziel = r.ziel ?? angebot.ziel;
        abmeldeMeldungZeigen(
          r.art === "mail"
            ? $t("mail.unsubscribeMailDone", { ziel })
            : $t("mail.unsubscribeDone", { ziel }),
        );
        abmeldeAngebot = null;
      }
    } catch (e: unknown) {
      abmeldeMeldungZeigen(localizeError(e instanceof Error ? e.message : String(e)));
    } finally {
      abmeldenLaeuft = false;
    }
  }

  let undoDelete = $state<{ uids: number[]; accountId: number; folder: string; timer: ReturnType<typeof setTimeout> } | null>(null);

  function deleteIsRecoverable(): boolean {
    return moveToTrash && selectedFolder !== "Trash";
  }

  function requestDelete(uids: number[]) {
    if (uids.length === 0 || showDeleteConfirm || isDeleting) return;
    if (deleteIsRecoverable()) {
      deferDelete(uids);
    } else {
      pendingDeleteUids = uids;
      showDeleteConfirm = true;
    }
  }

  function handleDeleteMessage(uid: number, uids?: number[]) {
    if (uids && !$mailbox.selectedUids.includes(uid)) mailbox.selectSingle(uid);
    requestDelete(uids ?? [uid]);
  }

  function handleDeleteSelected() {
    requestDelete($mailbox.selectedUids);
  }

  // ─── Stapel verschieben mit Rückgängig (Aufräumen) ───────────
  // Like the trash: the rows go at once, the server hears of it after
  // UNDO_MS — "Rückgängig" before that only reloads, no new UIDs needed.
  let undoVerschieben = $state<{ uids: number[]; accountId: number; folder: string; ziel: string; text: string; timer: ReturnType<typeof setTimeout> } | null>(null);

  function verschiebenMitRueckgaengig(uids: number[], ziel: string, text: string) {
    if (uids.length === 0 || ziel === selectedFolder) return;
    flushPendingDelete();
    flushVerschieben();
    const job = { uids, accountId: selectedAccountId, folder: selectedFolder, ziel, text };
    for (const uid of uids) mailbox.removeMessage(uid);
    mailbox.clearSelection();
    undoVerschieben = { ...job, timer: setTimeout(() => flushVerschieben(), UNDO_MS) };
  }

  function flushVerschieben() {
    const job = undoVerschieben;
    if (!job) return;
    clearTimeout(job.timer);
    undoVerschieben = null;
    void stapelAusfuehren(job);
  }

  async function stapelAusfuehren(job: { uids: number[]; accountId: number; folder: string; ziel: string }) {
    const raw = getAccountFolders(job.accountId).raw;
    let fehler = 0;
    for (let i = 0; i < job.uids.length; i += STAPEL_MAX) {
      try {
        const r = await moveMessagesBatch(job.accountId, job.uids.slice(i, i + STAPEL_MAX), job.folder, job.ziel,
          raw[job.folder] || job.folder, raw[job.ziel] || job.ziel);
        fehler += r.fehler;
      } catch {
        fehler += Math.min(STAPEL_MAX, job.uids.length - i);
      }
    }
    invalidateFolderCache(job.accountId, job.folder);
    invalidateFolderCache(job.accountId, job.ziel);
    if (fehler > 0) {
      mailbox.setError(translate("mail.moveFailed") + translate("mail.moveBatchPartial", { count: String(fehler) }));
      if (job.accountId === selectedAccountId && job.folder === selectedFolder) void loadFolder(true);
    }
  }

  function undoVerschiebenRueckgaengig() {
    const job = undoVerschieben;
    if (!job) return;
    clearTimeout(job.timer);
    undoVerschieben = null;
    invalidateFolderCache(job.accountId, job.folder);
    if (job.accountId === selectedAccountId && job.folder === selectedFolder) void loadFolder();
    aufraeumenNeuLaden += 1;
  }


  /** The mails an action means: the selection, else the open mail. */
  function gemeinteUids(): number[] {
    if ($mailbox.selectedUids.length > 0) return [...$mailbox.selectedUids];
    return selectedMessage ? [selectedMessage.uid] : [];
  }

  function archivieren(uids = gemeinteUids()) {
    const n = uids.length;
    verschiebenMitRueckgaengig(uids, archivOrdner(),
      n === 1 ? translate("mail.archivedOne") : translate("mail.archivedMany", { count: String(n) }));
  }

  /** Spam, or back to the inbox from the spam folder ("Kein Spam"). */
  function spamUmschalten(uids = gemeinteUids()) {
    if (istSpamOrdner(selectedFolder)) {
      verschiebenMitRueckgaengig(uids, "INBOX",
        uids.length === 1 ? translate("mail.notSpamOne") : translate("mail.notSpamMany", { count: String(uids.length) }));
    } else {
      verschiebenMitRueckgaengig(uids, spamOrdner(),
        uids.length === 1 ? translate("mail.spamOne") : translate("mail.spamMany", { count: String(uids.length) }));
    }
  }

  onDestroy(() => flushVerschieben());

  // ─── Aufräumen (behind the switch in Einstellungen › Allgemein) ───
  let aufraeumenAn = $state(false);
  let ansicht = $state<"liste" | "absender" | "durchgehen">("liste");
  let aufraeumenNeuLaden = $state(0);

  function aufraeumenAktion(uids: number[], art: "archiv" | "papierkorb" | "spam") {
    if (art === "archiv") archivieren(uids);
    else if (art === "spam") spamUmschalten(uids);
    else requestDelete(uids);
  }

  function aufraeumenOeffnen() {
    ansicht = "absender";
    folderSheetOpen = false;
  }

  function deferDelete(uids: number[]) {
    flushPendingDelete();
    flushVerschieben();
    const job = { uids, accountId: selectedAccountId, folder: selectedFolder };
    for (const uid of uids) mailbox.removeMessage(uid);
    undoDelete = { ...job, timer: setTimeout(() => flushPendingDelete(), UNDO_MS) };
  }

  /** Runs a pending trash move now (timer, a second delete, leaving the page). */
  function flushPendingDelete() {
    const job = undoDelete;
    if (!job) return;
    clearTimeout(job.timer);
    undoDelete = null;
    void executeDelete(job.uids, job.accountId, job.folder);
  }

  function undoPendingDelete() {
    const job = undoDelete;
    if (!job) return;
    clearTimeout(job.timer);
    undoDelete = null;
    // Nothing reached the server yet — reloading brings the mails back.
    invalidateFolderCache(job.accountId, job.folder);
    if (job.accountId === selectedAccountId && job.folder === selectedFolder) void loadFolder();
    aufraeumenNeuLaden += 1;
  }

  onDestroy(() => flushPendingDelete());

  // Phase C (Concept §9.4): a follow-up chip builds a pending plan
  // (origin=mail_followup) and hands it to the assistant drawer, where the T1
  // card is confirmed. Nothing executes here — the user confirms in the drawer.
  async function handleFollowupChip(s: FollowupSuggestion) {
    if (followupPlanBusy) return;
    followupPlanBusy = true;
    try {
      const plan = await createPlanFromSuggestion(s, {
        sourceMessageId: followupsForUid ?? undefined,
      });
      assistantCommand.showPlan(plan);
    } catch (e) {
      followupsError = localizeError(e instanceof Error ? e.message : String(e));
    } finally {
      followupPlanBusy = false;
    }
  }

  async function handleToggleRead(uid: number, uids?: number[]) {
    const msg = $mailbox.messages.find((m) => m.uid === uid);
    if (!msg) return;
    if (uids && !$mailbox.selectedUids.includes(uid)) mailbox.selectSingle(uid);
    const targets = uids ?? [uid];
    const targetRead = !msg.is_read;
    try {
      if (targets.length > 1) {
        if (targetRead) await markBatchAsRead(selectedAccountId, targets, selectedFolder);
        else await markBatchAsUnseen(selectedAccountId, targets, selectedFolder);
        for (const u of targets) mailbox.updateMessage(u, $mailbox.folderId, { is_read: targetRead });
      } else if (targetRead) {
        await markAsRead(selectedAccountId, uid, selectedFolder);
        mailbox.updateMessage(uid, $mailbox.folderId, { is_read: true });
      } else {
        await markAsUnseen(selectedAccountId, uid, selectedFolder);
        mailbox.updateMessage(uid, $mailbox.folderId, { is_read: false });
      }
    } catch (e) {
      console.warn("handleToggleRead fehlgeschlagen fuer uid", uid, e);
    }
    refreshUnreadCounts();
  }

  async function handleToggleFlag(uid: number, uids?: number[]) {
    // Folder-scoped lookup: UIDs are only unique per folder, and the store's
    // message list can briefly belong to the PREVIOUS folder while a new
    // folder loads (messagesFolder vs folderId). Only act when the list still
    // matches the folder the user is looking at.
    const folder = selectedFolder ?? "INBOX";
    if ($mailbox.messagesFolder !== null && $mailbox.messagesFolder !== folder) {
      console.warn("handleToggleFlag uebersprungen: Ordnerwechsel im Gange (uid", uid, ")");
      return;
    }
    if (uids && !$mailbox.selectedUids.includes(uid)) mailbox.selectSingle(uid);
    const targets = uids ?? [uid];
    const msg = $mailbox.messages.find((m) => m.uid === uid);
    if (!msg) return;
    const next = !msg.is_flagged;
    try {
      await Promise.all(targets.map((u) => flagMessageCmd(selectedAccountId, u, folder, next)));
      for (const u of targets) mailbox.updateMessage(u, $mailbox.folderId, { is_flagged: next });
      invalidateFolderCache(selectedAccountId, folder);
    } catch (e) {
      console.warn("handleToggleFlag fehlgeschlagen fuer uid", uid, e);
    }
  }

  async function handleToggleUrgent(uid: number, uids?: number[]) {
    const folder = selectedFolder ?? "INBOX";
    if ($mailbox.messagesFolder !== null && $mailbox.messagesFolder !== folder) {
      console.warn("handleToggleUrgent uebersprungen: Ordnerwechsel im Gange (uid", uid, ")");
      return;
    }
    if (uids && !$mailbox.selectedUids.includes(uid)) mailbox.selectSingle(uid);
    const targets = uids ?? [uid];
    const msg = $mailbox.messages.find((m) => m.uid === uid);
    if (!msg) return;
    const next = !msg.is_urgent;
    try {
      await Promise.all(targets.map((u) => urgentMessageCmd(selectedAccountId, u, folder, next)));
      for (const u of targets) mailbox.updateMessage(u, $mailbox.folderId, { is_urgent: next });
      invalidateFolderCache(selectedAccountId, folder);
    } catch (e) {
      console.warn("handleToggleUrgent fehlgeschlagen fuer uid", uid, e);
    }
  }

  async function confirmDelete() {
    if (isDeleting) return;
    const uids = pendingDeleteUids;
    if (uids.length === 0) return;
    pendingDeleteUids = [];
    showDeleteConfirm = false;
    for (const uid of uids) mailbox.removeMessage(uid);
    await executeDelete(uids, selectedAccountId, selectedFolder);
  }

  async function executeDelete(uids: number[], accountId: number, folder: string) {
    isDeleting = true;
    try {
      // Parallel: the server applies local deletes instantly and replays the
      // provider moves in the background.
      await Promise.all(
        uids.map(async (uid) => {
          try {
            await deleteMessageCmd(accountId, uid, folder);
          } catch (e) {
            console.warn("Loeschen von uid", uid, "fehlgeschlagen", e);
          }
        }),
      );
      invalidateFolderCache(accountId, folder);
      if (accountId === selectedAccountId && folder === selectedFolder) await loadFolder();
    } finally {
      isDeleting = false;
    }
  }

  function cancelDelete() {
    pendingDeleteUids = [];
    showDeleteConfirm = false;
  }

  function translateFolder(name: string): string {
    const dict: Record<string, string> = {
      "INBOX": "mail.folderInbox",
      "Sent": "mail.folderSent",
      "Drafts": "mail.folderDrafts",
      "Trash": "mail.folderTrash",
      "Spam": "mail.folderSpam",
      "Archive": "mail.folderArchive",
      "Junk": "mail.folderJunk",
      "Gelöscht": "mail.folderGeloescht",
      "Spamverdacht": "mail.folderSpamverdacht"
    };
    return dict[name] || name;
  }

  // Page head (HB-SEITENKOPF): the chosen folder by the name the column shows
  // it under (own name, else the translated leaf), and the account's unread
  // count where the server has one — the inbox.
  let ordnerTitel = $derived.by(() => {
    if (searchActive) return $t("mail.searchTitle");
    const custom = customFolderNames[selectedFolder];
    if (custom) return custom;
    if (selectedFolder === "INBOX") return $t(translateFolder("INBOX"));
    const leaf = getLeafName(selectedFolder, folderDelimiters[selectedFolder] || ".");
    return customFolderNames[leaf] || $t(translateFolder(leaf));
  });
  let ordnerUngelesen = $derived(
    !searchActive && selectedFolder === "INBOX" ? (unreadByAccount[selectedAccountId] ?? 0) : 0
  );
</script>

<svelte:window onkeydown={handleKeydown} />
<svelte:head><title>{showSplash ? tabTitel() : tabTitel(ordnerTitel)}</title></svelte:head>

<!-- A sign-only button of a toolbar (CI G4): the word as tooltip and name,
     40 px target from .btn-symbol. -->
{#snippet zeichen(name: SymbolName, wort: string, aktion: (e: MouseEvent) => void, taste?: string, gedrueckt?: boolean)}
  <button type="button" class="btn btn-still btn-symbol" title={taste ? `${wort} (${taste})` : wort} aria-label={wort}
    aria-pressed={gedrueckt === undefined ? undefined : gedrueckt} onclick={aktion}>
    <Symbol {name} size={20} />
  </button>
{/snippet}

{#snippet list()}
  <MessageList
    messages={$mailbox.messages}
    selectedUids={$mailbox.selectedUids}
    onselect={handleSelectMessage}
    onauswahl={(uid) => mailbox.auswahlUmschalten(uid)}
    onselectToggle={handleSelectToggle}
    onselectRange={handleSelectRange}
    onreply={handleReplyMessage}
    onforward={handleForwardMessage}
    ondelete={handleDeleteMessage}
    ontoggleRead={handleToggleRead}
    ontoggleFlag={handleToggleFlag}
    ontoggleUrgent={handleToggleUrgent}
    onmove={(uid, x, y) => {
      // A context-menu move acts on the right-clicked mail; select it first
      // so performMoveSelected() (which reads selectedUids) picks it up.
      if (!$mailbox.selectedUids.includes(uid)) mailbox.selectSingle(uid);
      openMoveMenuAt(x, y);
    }}
    ondragstart={handleDragStart}
    loading={$mailbox.loading}
    accountId={selectedAccountId}
    isDraftFolder={selectedFolder === draftsFolderName}
    isSentFolder={selectedFolder === sentFolderName}
    loeschenEndgueltig={!deleteIsRecoverable()}
    searchActive={searchActive}
  />
{/snippet}

{#snippet preview()}
  {#if showCompose}
    <ComposeWindow
      mode={composeMode}
      mailChain={mailChain}
      sendError={sendError}
      replySubject={replySubject}
      replyTo={replyTo}
      replyCc={replyCc}
      accountId={selectedAccountId}
      recipientEmail={replyTo}
      recipientName={recipientName}
      senderName={senderName}
      onclose={closeCompose}
      onsend={handleSend}
      ondraftSaved={(uid) => { draftUid = uid; }}
      draftTo={assistantCompose?.to ?? (draftUid ? draftTo : undefined)}
      draftCc={draftUid ? draftCc : undefined}
      draftSubject={assistantCompose?.subject ?? (draftUid ? draftSubject : undefined)}
      draftBody={assistantCompose?.body ?? (draftUid ? draftBody : undefined)}
      draftUid={assistantCompose ? null : draftUid}
      prefill={assistantCompose}
      initialAttachments={draftInitialAttachments}
    />
  {:else if selectedMessage}
    {@const msg = selectedMessage}
    {@const imSpam = istSpamOrdner(selectedFolder)}
    <div class="preview-layout">
      <div class="preview-pane-header">
        <div class="preview-header-meta">
          <span class="preview-from-name">{extractName(selectedMessage.from) || $t("mail.unknown")}</span>
          <span class="preview-from-email">
            {extractEmail(selectedMessage.from)}
            <!-- "Abo beenden" has no clear sign: a word beside the sender, as
                 in Gmail (CI G4). -->
            {#if abmeldeAngebot && abmeldeAngebot.uid === selectedMessage.uid}
              <span aria-hidden="true">·</span>
              <button type="button" class="preview-abmelden" disabled={abmeldenLaeuft} onclick={() => (abmeldenFragen = true)}>
                {$t("mail.unsubscribe")}
              </button>
            {/if}
          </span>
        </div>
        <!-- One toolbar of signs (CI G4, Kai 07.10.2026): known, reversible,
             named with tooltip and key. Words stay for what has no clear
             sign or cannot be undone. -->
        <div class="werkzeugleiste" role="toolbar" aria-label={$t("mail.aktionen")}>
          <div class="werkzeug-gruppe">
            {@render zeichen("antworten", $t("mail.reply"), () => handleReply(msg))}
            {@render zeichen("weiterleiten", $t("mail.forward"), () => handleForwardMessage(msg.uid))}
          </div>
          <div class="werkzeug-gruppe">
            {@render zeichen("archiv", $t("mail.archive"), () => archivieren([msg.uid]), "E")}
            {#if imSpam}
              <button type="button" class="btn btn-still btn-klein" onclick={() => spamUmschalten([msg.uid])} title={$t("mail.notSpamTitle")}>
                {$t("mail.notSpam")}
              </button>
            {:else}
              {@render zeichen("spam", $t("mail.alsSpam"), () => spamUmschalten([msg.uid]), "!")}
            {/if}
            {#if deleteIsRecoverable()}
              {@render zeichen("loeschen", $t("mail.inPapierkorb"), () => handleDeleteMessage(msg.uid), "⌫")}
            {:else}
              <!-- Final: red, with its object and a question (CI G2). -->
              <button type="button" class="btn btn-gefahr btn-klein" onclick={() => handleDeleteMessage(msg.uid)}>
                {$t("mail.deleteFinal1")}
              </button>
            {/if}
            {@render zeichen("verschieben", $t("mail.moveFolderTitle"), (e) => {
              mailbox.selectSingle(msg.uid);
              const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
              openMoveMenuAt(r.left, r.bottom + 4);
            })}
          </div>
          <div class="werkzeug-gruppe">
            {@render zeichen("gelesen", msg.is_read ? $t("mail.markUnread") : $t("mail.markRead"), () => handleToggleRead(msg.uid))}
            {@render zeichen("markieren", msg.is_flagged ? $t("mail.flagOff") : $t("mail.flagOn"), () => handleToggleFlag(msg.uid), undefined, msg.is_flagged)}
            {#if hasSeveralRecipients(msg)}
              {@render zeichen("mehr", $t("common.more"), (e) => {
                const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
                mehrMenue = { x: r.right - 220, y: r.bottom + 4 };
              })}
            {/if}
          </div>
        </div>
      </div>
      
      <div class="preview-scroll-wrapper">
        <div class="preview-content-area">
          <!-- h2: the page's one h1 is the folder in the list head. -->
          <h2 class="preview-subject-large">{selectedMessage.subject || $t("mail.noSubject")}</h2>
          <div class="preview-date-line">{formatDate(selectedMessage.date)}</div>
          {#if selectedMessage.to || selectedMessage.cc}
            <div class="preview-recipients">
              {#if selectedMessage.to}
                <span class="preview-recipient-line"><strong>{$t("mail.to")}</strong> {selectedMessage.to}</span>
              {/if}
              {#if selectedMessage.cc}
                <span class="preview-recipient-line"><strong>CC:</strong> {selectedMessage.cc}</span>
              {/if}
            </div>
          {/if}
          
          <div class="preview-body">
            {#if loadingBodyUid === selectedMessage.uid}
              <div class="preview-skeleton-body">
                <div class="skeleton-line skeleton-body-line w-100" style="animation-delay: 0.15s"></div>
                <div class="skeleton-line skeleton-body-line w-90" style="animation-delay: 0.2s"></div>
                <div class="skeleton-line skeleton-body-line w-80" style="animation-delay: 0.25s"></div>
                <div class="skeleton-line skeleton-body-line w-95" style="animation-delay: 0.3s"></div>
                <div class="skeleton-line skeleton-body-line w-70" style="animation-delay: 0.35s"></div>
                <div class="skeleton-line skeleton-body-line w-85" style="animation-delay: 0.4s"></div>
                <div class="skeleton-line skeleton-body-line w-60" style="animation-delay: 0.45s"></div>
                <div class="skeleton-line skeleton-body-line w-75" style="animation-delay: 0.5s"></div>
              </div>
            {:else if previewSrcdoc}
              <div class="mail-iframe-container">
                <iframe
                  title={$t("mail.emailContent")}
                  srcdoc={previewSrcdoc}
                  class="mail-iframe"
                  sandbox="allow-scripts"
                  referrerpolicy="no-referrer"
                ></iframe>
              </div>
            {:else if parsedContent.text}
              <div class="mail-body">{parsedContent.text}</div>
            {:else if selectedMessage.body_preview}
              <div class="mail-body">{selectedMessage.body_preview}</div>
            {:else}
              <div class="mail-body-empty">{$t("mail.noContent")}</div>
            {/if}
          </div>

          {#if attachments.length > 0}
            <div class="attachments">
              <div class="attachments-title">{attachments.length === 1 ? $t("mail.attachmentsOne") : $t("mail.attachmentsMany", { count: attachments.length })}</div>
              <div class="attachments-list">
                {#each attachments as att, i (i)}
                  <button
                    type="button"
                    class="attachment-chip"
                    onclick={() => handleOpenAttachment(att)}
                    oncontextmenu={(e) => handleAttachmentContextMenu(e, att)}
                    title={$t("mail.openAttachmentTitle")}
                  >
                    <span class="attachment-icon" aria-hidden="true"><Symbol name="anhang" size={16} /></span>
                    <span class="attachment-meta">
                      <span class="attachment-name">{att.filename}</span>
                      <span class="attachment-size">{formatBytes(att.size)}</span>
                    </span>
                  </button>
                {/each}
              </div>
            </div>
          {/if}

          {#if attCtxMenu}
            <div class="ctx-menu-scrim" class:sheet-scrim={isTouchDevice} role="presentation" onclick={closeAttCtxMenu} oncontextmenu={(e) => e.preventDefault()}></div>
            <div class="ctx-menu" class:sheet={isTouchDevice} style={isTouchDevice ? "" : `left: ${attCtxMenu!.x}px; top: ${attCtxMenu!.y}px;`} role="menu">
              <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => handleOpenAttachment(attCtxMenu!.att)}><span class="ctx-icon">{@html iconSVG("open")}</span>{$t("mail.open")}</button>
              <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => handleSaveAsAttachment(attCtxMenu!.att)}><span class="ctx-icon">{@html iconSVG("download")}</span>{$t("mail.download")}</button>
            </div>
          {/if}

          {#if attPreview}
            <!-- HB-DIALOG; a click on the layer itself (not the card) closes it. -->
            <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
            <div
              class="dialog-schicht att-preview-schicht"
              role="dialog"
              tabindex="-1"
              aria-modal="true"
              aria-label={$t("mail.attachmentPreview")}
              onclick={(e) => { if ((e.target as HTMLElement).classList.contains("dialog-schicht")) closeAttPreview(); }}
            >
              <div class="karte dialog-karte att-preview-karte" data-breite="breit">
                <div class="dialog-kopf">
                  <h2 class="att-preview-name" title={attPreview.filename}>{attPreview.filename}</h2>
                  <button type="button" class="btn btn-still btn-symbol" onclick={downloadAttPreview} title={$t("mail.downloadTitle")} aria-label={$t("mail.downloadTitle")}>
                    <Symbol name="herunterladen" size={20} />
                  </button>
                  <button type="button" class="dialog-zu" onclick={closeAttPreview} title={$t("mail.close")} aria-label={$t("mail.close")}>
                    <Symbol name="schliessen" size={20} />
                  </button>
                </div>
                <div class="dialog-koerper att-preview-body">
                  {#if attPreview.contentType.startsWith("image/")}
                    <img src={attPreview.url} alt={attPreview.filename} class="att-preview-image" />
                  {:else if attPreview.contentType.startsWith("text/") || attPreview.contentType.includes("json") || attPreview.contentType.includes("xml") || attPreview.contentType.includes("javascript")}
                    <iframe src={attPreview.url} title={attPreview.filename} class="att-preview-frame"></iframe>
                  {:else if attPreview.contentType === "application/pdf"}
                    <iframe src={attPreview.url} title={attPreview.filename} class="att-preview-frame"></iframe>
                  {:else}
                    <div class="att-preview-unsupported">
                      <span>{$t("mail.previewUnsupported")}</span>
                      <button type="button" class="btn btn-primaer" onclick={downloadAttPreview}>{$t("mail.download")}</button>
                    </div>
                  {/if}
                </div>
              </div>
            </div>
          {/if}

          {#if replySuggestions.length > 0}
            <ReplySuggestions suggestions={replySuggestions} onselect={(s) => {
              composeMode = "reply";
              sendError = null;
              replySubject = selectedMessage.subject ?? "";
              replyTo = extractEmail(selectedMessage.from ?? "");
              replyCc = "";
              showCompose = true;
            }} />
          {/if}
        </div>
      </div>
    </div>
  {:else if $mailbox.loading}
    <div class="preview-skeleton" aria-hidden="true">
      <div class="preview-skeleton-header">
        <div class="skeleton-line skeleton-preview-subject" style="animation-delay: 0s"></div>
        <div class="skeleton-line skeleton-preview-sender" style="animation-delay: 0.05s"></div>
        <div class="skeleton-line skeleton-preview-date" style="animation-delay: 0.1s"></div>
      </div>
      <div class="preview-skeleton-body">
        <div class="skeleton-line skeleton-body-line w-100" style="animation-delay: 0.15s"></div>
        <div class="skeleton-line skeleton-body-line w-90" style="animation-delay: 0.2s"></div>
        <div class="skeleton-line skeleton-body-line w-80" style="animation-delay: 0.25s"></div>
        <div class="skeleton-line skeleton-body-line w-95" style="animation-delay: 0.3s"></div>
        <div class="skeleton-line skeleton-body-line w-70" style="animation-delay: 0.35s"></div>
        <div class="skeleton-line skeleton-body-line w-85" style="animation-delay: 0.4s"></div>
        <div class="skeleton-line skeleton-body-line w-60" style="animation-delay: 0.45s"></div>
        <div class="skeleton-line skeleton-body-line w-75" style="animation-delay: 0.5s"></div>
      </div>
    </div>
  {:else if initError}
    <EmptyState
      tone="error"
      icon="achtung"
      title={$t("mail.noConnection")}
      subtitle={initError}
      actionLabel={$t("mail.retry")}
      onaction={retryInit}
    />
  {:else}
    <EmptyState icon="eingang" title={$t("mail.selectMessage")} subtitle={$t("mail.selectMessageDesc")} offsetHeader={true} />
  {/if}
{/snippet}

{#if showSplash}
  <!-- No account yet: the onboarding takes the whole screen, outside the shell. -->
  <SplashScreen oncomplete={handleSplashComplete} />
{:else}
  <Huelle bereich="mail" bind:suche={searchQuery} bind:spalteOffen={folderSheetOpen} suchePlatzhalter={$t("mail.search")}>
    {#snippet spalte()}
      <!-- The inside of the mail area: accounts and their folder trees (RL-G2).
           On the phone the shell shows it as a sheet; a folder closes it. -->
      <div class="mail-spalte">
        <!-- The page's one primary, where Gmail and Outlook put it. -->
        <button type="button" class="btn btn-primaer mail-neu-spalte" onclick={handleNewMail} title={$t("mail.newMail")}>
          <Symbol name="plus" size={16} />
          {$t("mail.new")}
        </button>
        {#each $accounts.groups as group}
          <AccountGroup
            account={group.account}
            folderTree={folderTreesByAccount[group.account.id] ?? { name: "INBOX", label: "", children: [] }}
            selectedFolder={group.account.id === selectedAccountId ? selectedFolder : null}
            collapsedFolders={getCollapsedForAccount(group.account.id)}
            unreadCount={unreadByAccount[group.account.id] ?? 0}
            bind:dragSource
            bind:dragTarget
            onSelectFolder={handleAccountFolderSelect}
            onToggleCollapse={handleToggleCollapse}
            onToggleFolder={handleToggleFolder}
            onMoveMessage={handleMoveMessage}
            onFolderMouseDown={handleFolderMouseDown}
            onContextMenu={handleFolderContextMenu}
          />
        {/each}
        {#if aufraeumenAn}
          <!-- Erweitert: only with the switch on (Einstellungen › Allgemein). -->
          <button type="button" class="mail-aufraeumen" class:aktiv={ansicht !== "liste"}
            aria-current={ansicht !== "liste" ? "page" : undefined} onclick={aufraeumenOeffnen}>
            <Symbol name="archiv" size={20} />
            <span>{$t("mail.aufraeumen")}</span>
          </button>
        {/if}
      </div>
    {/snippet}

  {#if ansicht === "absender"}
    <Aufraeumen
      accountId={selectedAccountId}
      folder={selectedFolder}
      folderLabel={translate(translateFolder(selectedFolder))}
      istSpamOrdner={istSpamOrdner(selectedFolder)}
      neuLaden={aufraeumenNeuLaden}
      onaktion={aufraeumenAktion}
      onmeldung={abmeldeMeldungZeigen}
      ondurchgehen={() => (ansicht = "durchgehen")}
      onschliessen={() => (ansicht = "liste")}
    />
  {:else if ansicht === "durchgehen"}
    <Durchgehen
      accountId={selectedAccountId}
      folder={selectedFolder}
      messages={$mailbox.messages}
      istSpamOrdner={istSpamOrdner(selectedFolder)}
      onaktion={aufraeumenAktion}
      onschliessen={() => (ansicht = "absender")}
    />
  {:else}
  <div class="app-container" class:compact={isCompact} class:narrow={isNarrow} class:preview-open={previewOpen}>
    <main class="list-pane" style={isCompact ? "" : `width: ${listWidth}px; min-width: ${listWidth}px;`}>
      <!-- HB-SEITENKOPF in a narrow column: one line, the folder with its
           unread count left, the list's signs right (Kai, 07.10.2026). The
           one primary "Neue E-Mail" sits atop the column on the desktop and
           only here where the column is a sheet. -->
      <div class="seitenkopf mail-kopf">
        <div class="seitenkopf-zeile">
          <h1>{ordnerTitel}</h1>
          {#if ordnerUngelesen > 0}
            <span class="seitenkopf-zahl" title={$t("mail.unreadCount", { count: ordnerUngelesen })}>{ordnerUngelesen}</span>
          {/if}
        </div>
        <div class="btn-reihe">
          <button
            type="button"
            class="btn btn-still btn-symbol"
            onclick={toggleFlagFilter}
            title={flaggedSearchActive ? $t("mail.flagHide") : $t("mail.flagOnly")}
            aria-label={$t("mail.flagOnly")}
            aria-pressed={flaggedSearchActive}
          >
            <Symbol name="markieren" size={20} filled={flaggedSearchActive} />
          </button>
          <button type="button" class="btn btn-still btn-symbol" onclick={() => loadFolder(true)} title={$t("mail.refresh")} aria-label={$t("mail.refresh")}>
            <Symbol name="neu-laden" size={20} />
          </button>
          <!-- On the phone the plus alone, as in the CI's phone head (G5). -->
          <button type="button" class="btn btn-primaer btn-klein mail-neu-kopf" onclick={handleNewMail} title={$t("mail.newMail")} aria-label={$t("mail.new")}>
            <Symbol name="plus" size={16} />
            <span class="mail-neu-wort">{$t("mail.new")}</span>
          </button>
        </div>
      </div>
      {#if $mailbox.error}
        <ErrorBanner message={$mailbox.error} onretry={() => loadFolder(true)} />
      {/if}
      {#if $mailbox.selectedUids.length > 1}
        <div class="selection-toolbar">
          <span class="selection-count">{$t("mail.selectedCount", { count: $mailbox.selectedUids.length })}</span>
          <div class="selection-actions">
            <!-- Signs as above the mail (CI G4); the trash is not red,
                 it comes back with "Rückgängig" (CI G2, Kai 07.10.2026). -->
            {@render zeichen("gelesen", $t("mail.markReadTitle"), markSelectedRead)}
            {@render zeichen("archiv", $t("mail.archive"), () => archivieren(), "E")}
            {@render zeichen("verschieben", $t("mail.moveFolderTitle"), moveSelectedToFolder)}
            {#if istSpamOrdner(selectedFolder)}
              <button type="button" class="btn btn-still btn-klein" onclick={() => spamUmschalten()} title={$t("mail.notSpamTitle")}>
                {$t("mail.notSpam")}
              </button>
            {:else}
              {@render zeichen("spam", $t("mail.alsSpam"), () => spamUmschalten(), "!")}
            {/if}
            {#if deleteIsRecoverable()}
              {@render zeichen("loeschen", $t("mail.inPapierkorb"), handleDeleteSelected, "⌫")}
            {:else}
              <button type="button" class="btn btn-sekundaer btn-klein" onclick={handleDeleteSelected}>
                {$t("mail.deleteFinalN")}
              </button>
            {/if}
            {@render zeichen("schliessen", $t("mail.auswahlAufheben"), () => mailbox.clearSelection(), "Esc")}
          </div>
        </div>
      {/if}
      <div class="list-scroll-wrapper">
        {@render list()}
      </div>
    </main>
    {#if !isCompact}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div class="resize-handle" role="separator" aria-orientation="vertical" onmousedown={startResize}></div>
    {/if}
    <section class="preview-pane">
      {#if isCompact && previewOpen && !showCompose}
        <div class="preview-back-bar">
          <button type="button" class="btn btn-still" onclick={backToList} title={$t("mail.backToList")} aria-label={$t("mail.back")}>
            <Symbol name="zurueck" size={20} />{$t("mail.back")}
          </button>
        </div>
      {/if}
      {@render preview()}
      {#if !showCompose && followupsForUid === selectedMessage?.uid && (followupsLoading || followups.length > 0 || followupsError || followupsUnavailable)}
        <div class="followups-footer">
          <div class="followups-footer-head">
            <span class="followups-footer-title">AI-Vorschläge</span>
            {#if followupsError}
              <div class="hinweis followups-footer-error" data-art="fehler"><Symbol name="achtung" size={16} /><span>{followupsError}</span></div>
            {/if}
          </div>
          <div class="followups-footer-scroll">
            {#if followupsLoading}
              <div class="followups-footer-row"><span class="followups-footer-muted">Analysiere E-Mail…</span></div>
            {:else if followupsUnavailable}
              <div class="followups-footer-row"><span class="followups-footer-muted">AI-Vorschläge sind gerade nicht verfügbar.</span></div>
            {:else if followups.length === 0}
              <div class="followups-footer-row"><span class="followups-footer-muted">Keine Vorschläge.</span></div>
            {:else}
              {#each followups as a (a.id)}
                <div class="followups-footer-row">
                  <div class="followups-footer-label">
                    <span>{a.titel}</span>
                  </div>
                  <button
                    type="button"
                    class="btn btn-sekundaer btn-klein followups-footer-btn"
                    disabled={followupPlanBusy}
                    onclick={() => handleFollowupChip(a)}
                  >
                    Vorschlag annehmen
                  </button>
                </div>
              {/each}
            {/if}
          </div>
        </div>
      {/if}
    </section>

  </div>
  {/if}
  </Huelle>
{/if}

  {#if showDeleteConfirm}
    <ConfirmationDialog
      open={showDeleteConfirm}
      title={pendingDeleteUids.length === 1 ? $t("mail.deleteConfirmTitle1") : $t("mail.deleteConfirmTitleN")}
      message={pendingDeleteUids.length === 1
        ? $t("mail.deleteConfirmMsg1")
        : $t("mail.deleteConfirmMsgN", { count: pendingDeleteUids.length })}
      confirmLabel={pendingDeleteUids.length === 1 ? $t("mail.deleteFinal1") : $t("mail.deleteFinalN")}
      cancelLabel={$t("common.cancel")}
      danger={true}
      onconfirm={confirmDelete}
      oncancel={cancelDelete}
    />
  {/if}

  {#if undoDelete}
    <div class="undo-toast" role="status" aria-live="polite">
      <span>{undoDelete.uids.length === 1 ? $t("mail.trashedOne") : $t("mail.trashedMany", { count: undoDelete.uids.length })}</span>
      <button type="button" class="btn btn-sekundaer" onclick={undoPendingDelete}>{$t("mail.undo")}</button>
    </div>
  {/if}

  {#if mehrMenue && selectedMessage}
    {@const msg = selectedMessage}
    <ContextMenu
      menu={mehrMenue}
      items={[{ label: $t("mail.replyAll"), action: () => handleReply(msg, true) }]}
      onclose={() => (mehrMenue = null)}
    />
  {/if}
  {#if abmeldenFragen && abmeldeAngebot}
    <!-- Goes out to the sender and cannot be taken back: a question first,
         focus on "Abbrechen" (HB-DIALOG). Not red — nothing is deleted. -->
    <ConfirmationDialog
      open={abmeldenFragen}
      title={$t("mail.unsubscribeTitle")}
      message={abmeldeAngebot.art === "link"
        ? $t("mail.unsubscribeLinkMsg", { ziel: abmeldeAngebot.ziel })
        : $t("mail.unsubscribeMsg", { ziel: abmeldeAngebot.ziel })}
      confirmLabel={abmeldeAngebot.art === "link" ? $t("mail.unsubscribeLinkConfirm") : $t("mail.unsubscribe")}
      cancelLabel={$t("common.cancel")}
      onconfirm={aboBeenden}
      enterConfirms={false}
      oncancel={() => (abmeldenFragen = false)}
    />
  {/if}

  {#if undoVerschieben}
    <div class="undo-toast" role="status" aria-live="polite">
      <span>{undoVerschieben.text}</span>
      <button type="button" class="btn btn-sekundaer" onclick={undoVerschiebenRueckgaengig}>{$t("mail.undo")}</button>
    </div>
  {/if}

  {#if abmeldeMeldung}
    <div class="undo-toast" role="status" aria-live="polite">
      <span>{abmeldeMeldung}</span>
    </div>
  {/if}

  {#if showDeleteFolderConfirm}
    <ConfirmationDialog
      open={showDeleteFolderConfirm}
      title={$t("mail.deleteFolderTitle")}
      message={$t("mail.deleteFolderMsg", { name: pendingDeleteFolder ?? "" })}
      confirmLabel={$t("mail.deleteFolderTitle")}
      cancelLabel={$t("common.cancel")}
      danger={true}
      onconfirm={confirmDeleteFolder}
      oncancel={() => { showDeleteFolderConfirm = false; pendingDeleteFolder = null; }}
    />
  {/if}


  <PromptDialog
    open={showRenameDialog}
    title={$t("mail.renameFolderTitle")}
    message={$t("mail.renameFolderMsg")}
    value={renameLeafValue}
    confirmLabel={$t("mail.rename")}
    cancelLabel={$t("common.cancel")}
    onconfirm={confirmRename}
    oncancel={cancelRename}
  />

  <PromptDialog
    open={showNewFolderDialog}
    title={$t("mail.newFolderTitle")}
    message={$t("mail.newFolderMsg")}
    placeholder={$t("mail.newFolderPlaceholder")}
    confirmLabel={$t("mail.create")}
    cancelLabel={$t("common.cancel")}
    onconfirm={confirmNewFolder}
    oncancel={cancelNewFolder}
  />

  <AssistantFab
    module="mail"
    context={selectedMessage ? `Aktive Mail: ${selectedMessage.subject || "(ohne Betreff)"} von ${selectedMessage.from}` : ""}
  />

  {#if folderCtxMenu}
    <div class="ctx-menu-scrim" class:sheet-scrim={isTouchDevice} role="presentation" onclick={closeMenus} oncontextmenu={(e) => e.preventDefault()}></div>
    <div class="ctx-menu" class:sheet={isTouchDevice} style={isTouchDevice ? "" : `left: ${folderCtxMenu!.x}px; top: ${folderCtxMenu!.y}px;`} role="menu">
      <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => { folderCtxNewSubFolder(folderCtxMenu!.folderName); }}><span class="ctx-icon">{@html iconSVG("newSubFolder")}</span>{$t("mail.newSubFolder")}</button>
      {#if folderCtxMenu!.folderName !== "INBOX"}
        <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => { openRenameDialog(folderCtxMenu!.folderName); closeMenus(); }}><span class="ctx-icon">{@html iconSVG("rename")}</span>{$t("mail.renameEllipsis")}</button>
      {/if}
      {#if customFolderNames[folderCtxMenu!.folderName]}
        <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => folderCtxResetName(folderCtxMenu!.folderName)}><span class="ctx-icon">{@html iconSVG("resetName")}</span>{$t("mail.resetName")}</button>
      {/if}
      {#if folderCtxMenu!.folderName !== "INBOX"}
        <div class="ctx-menu-separator" role="separator"></div>
        <button type="button" class="ctx-menu-item danger" role="menuitem" onclick={() => folderCtxDeleteFolder(folderCtxMenu!.folderName)}><span class="ctx-icon">{@html iconSVG("delete")}</span>{$t("mail.delete")}</button>
      {/if}
      {#if folderCtxMenu!.folderName !== "INBOX"}
        <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => folderCtxHideFolder(folderCtxMenu!.folderName)}><span class="ctx-icon">{@html iconSVG("hide")}</span>{$t("mail.hide")}</button>
      {/if}
      {#if hiddenFolderNames.length > 0}
        <div class="ctx-menu-separator" role="separator"></div>
        <button type="button" class="ctx-menu-item" role="menuitem" onclick={folderCtxUnhideAll}><span class="ctx-icon">{@html iconSVG("show")}</span>{$t("mail.showAllHidden")}</button>
      {/if}
    </div>
  {/if}

  {#if moveMenu}
    <div class="ctx-menu-scrim" class:sheet-scrim={isTouchDevice} role="presentation" onclick={closeMenus} oncontextmenu={(e) => e.preventDefault()}></div>
    <div class="ctx-menu" class:sheet={isTouchDevice} style={isTouchDevice ? "" : `left: ${moveMenu.x}px; top: ${moveMenu.y}px;`} role="menu">
      {#each moveMenu.sections as section}
        {#if section.header != null}
          <div class="ctx-menu-header">{section.header}</div>
        {/if}
        {#each section.items as target (target.accountId + ":" + target.name)}
          <button
            type="button"
            class="ctx-menu-item"
            role="menuitem"
            title={target.full ?? target.name}
            style={target.depth ? `padding-left: calc(var(--am-raum-4) + ${target.depth} * var(--am-raum-4));` : ""}
            onclick={() => {
              const uids = [...$mailbox.selectedUids];
              const name = target.name;
              const accountId = target.accountId;
              closeMenus();
              void performMoveSelected(uids, name, accountId);
            }}
          ><span class="ctx-icon">{@html folderIconFor(target.full ?? target.name)}</span>{target.label ?? target.name}</button>
        {/each}
      {/each}
    </div>
  {/if}

  {#if linkMenu}
    <div class="ctx-menu-scrim" role="presentation" onclick={closeLinkMenu} oncontextmenu={(e) => e.preventDefault()}></div>
    <div class="ctx-menu" style={`left: ${linkMenu.x}px; top: ${linkMenu.y}px;`} role="menu">
      <button type="button" class="ctx-menu-item" role="menuitem" onclick={openLinkInTab}><span class="ctx-icon">{@html iconSVG("externalLink")}</span>{$t("mail.linkOpen")}</button>
      <button type="button" class="ctx-menu-item" role="menuitem" onclick={openLinkInBrowser}><span class="ctx-icon">{@html iconSVG("browser")}</span>{$t("mail.linkOpenBrowser")}</button>
    </div>
  {/if}

<style>
  /* ── Mail inside the shell: list and reading pane [RL-HUELLE] ────────────
     The shell (Huelle) hands the page its height; the two panes scroll
     themselves. Only `style` containment: layout/paint/strict would make a
     containing block for the position:fixed context menus inside the list
     and shift them by the header and the column. */
  .app-container {
    display: flex;
    min-width: 0;
    min-height: 0;
    contain: style;
  }
  .resize-handle {
    width: 5px;
    cursor: col-resize;
    background: transparent;
    flex-shrink: 0;
    z-index: 10;
  }
  .resize-handle:hover {
    background: var(--am-handlung-ruhend);
    opacity: 0.3;
  }
  .list-pane {
    flex-shrink: 0;
    min-height: 0;
    background: var(--am-seite);
    border-right: 1px solid var(--am-rand);
    /* NOTE: contain: layout/paint/strict would create a containing block for
       position:fixed descendants — the context menu (rendered inside the
       message list) would be positioned relative to this pane instead of the
       viewport and appear shifted to the right. `style` only is safe. */
    contain: style;
    display: flex;
    flex-direction: column;
  }
  .preview-pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
    background: var(--am-seite);
    /* contain: layout would create a containing block for position:fixed
       descendants — the attachment context menu would be misplaced. */
    contain: style;
    display: flex;
    flex-direction: column;
  }

  /* ── Responsive: compact and narrow [RL-HUELLE] ──────────────────────────
     The folder column is the shell's sheet on small screens; here only the
     list and the reading pane change. */
  /* COMPACT (≤900px): preview becomes a full-width overlay over the list,
     shown only when a message/compose is open. List fills the width. */
  .app-container.compact .list-pane {
    flex: 1;
    min-width: 0;
  }
  .app-container.compact .preview-pane {
    position: absolute;
    inset: 0;
    z-index: 30;
    display: none;
  }
  .app-container.compact.preview-open .preview-pane {
    display: flex;
  }
  .app-container.compact {
    position: relative;
  }

  /* NARROW (≤600px): a tighter reading pane. Touch targets come from
     AM-KNOPF (44 px on a coarse pointer). */
  .app-container.narrow .preview-back-bar {
    min-height: 44px;
  }
  .app-container.narrow .preview-pane-header {
    height: auto;
    min-height: 64px;
    padding: 8px 14px;
  }
  .app-container.narrow .preview-from-name {
    font-size: 1rem;
  }
  .app-container.narrow .preview-subject-large {
    font-size: 1.2rem;
  }
  .app-container.narrow .mail-iframe-container {
    height: calc(100vh - 220px);
    min-height: 320px;
  }

  /* ── The mail column: accounts and folder trees [RL-ORDNERBAUM] ─────────
     Rows after HB-UNTERNAV (RL-G2): 40 px, 8 px radius, hover surface 2;
     the chosen folder carries the gold edge, bold, its sign in gold — no
     filled surface. The rows themselves live in AccountGroup. */
  .mail-spalte {
    display: flex;
    flex-direction: column;
    padding-bottom: var(--am-raum-4);
  }
  .mail-spalte :global(.tree-row) {
    min-height: 40px;
    box-sizing: border-box;
    border-radius: var(--am-radius-mittel);
  }
  .mail-spalte :global(.tree-row:hover) {
    background: var(--am-flaeche-2);
  }
  .mail-spalte :global(.tree-row.active) {
    background: none;
    box-shadow: inset 2px 0 0 var(--am-gold-auszeichnung);
    color: var(--am-text-primaer);
    font-weight: 600;
  }
  .mail-spalte :global(.tree-row.active:hover) {
    background: var(--am-flaeche-2);
  }
  .mail-spalte :global(.tree-row.active .tree-icon) {
    color: var(--am-gold-beschriftung);
  }
  .mail-spalte :global(.tree-row .tree-icon),
  .mail-spalte :global(.tree-row .tree-icon svg) {
    width: 20px;
    height: 20px;
  }

  /* ── Folder rows, rendered by FolderList [RL-ORDNERBAUM] ─────────────────── */
  :global(.folder-item) {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 10px 12px;
    margin-bottom: 4px;
    border: none;
    background: none;
    font-size: var(--fs-base);
    font-weight: 500;
    color: var(--am-text-gedaempft);
    cursor: pointer;
    border-radius: var(--am-radius-mittel);
    font-family: inherit;
    transition: all var(--am-dauer-schnell) var(--am-kurve);
  }
  :global(.folder-item:hover) {
    background: var(--am-flaeche-2);
    color: var(--am-text-primaer);
  }
  :global(.folder-item.active) {
    background: var(--am-flaeche-2);
    color: var(--am-handlung-ruhend);
    font-weight: 600;
  }
  :global(.folder-item.indent) {
    padding-left: 34px;
  }
  :global(.folder-item.drag-over) {
    background: var(--am-flaeche-2);
    color: var(--am-handlung-ruhend);
    font-weight: 600;
  }
  :global(.folder-icon-wrapper) {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    color: currentColor;
    flex-shrink: 0;
    pointer-events: none;
  }
  :global(.folder-name-label) {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    pointer-events: none;
  }

  /* ── Message list: page head, selection toolbar [RL-POSTLISTE] ──────────
     The list column carries HB-SEITENKOPF. It is narrower than a page, so
     the head pads less to the side and the title gives way (ellipsis)
     before the buttons wrap. */
  .mail-kopf {
    padding-inline: var(--am-raum-4);
    flex-shrink: 0;
  }
  .mail-kopf .seitenkopf-zeile {
    min-width: 0;
    flex-wrap: nowrap;
  }
  .mail-kopf h1 {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mail-kopf {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--am-raum-2);
  }
  .mail-kopf .btn-reihe {
    flex-shrink: 0;
    flex-wrap: nowrap;
    gap: 2px;
    margin: 0;
  }
  .mail-kopf .mail-neu-kopf { margin-left: var(--am-raum-2); }
  .mail-neu-spalte {
    width: calc(100% - 2 * var(--am-raum-4));
    margin: 0 var(--am-raum-4) var(--am-raum-3);
    justify-content: center;
  }
  @media (min-width: 1024px) {
    .mail-kopf .mail-neu-kopf { display: none; }
  }
  @media (max-width: 1023px) {
    .mail-neu-spalte { display: none; }
  }
  @media (max-width: 40rem) {
    .mail-kopf { flex-direction: row; align-items: center; }
    .mail-kopf .mail-neu-kopf { width: var(--am-ziel-beruehrung); height: var(--am-ziel-beruehrung); padding: 0; justify-content: center; }
    .mail-neu-wort { display: none; }
  }
  @media (max-width: 40rem) {
    .mail-kopf { padding: var(--am-raum-3) var(--am-raum-4); }
  }
  .selection-toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 16px;
    background: var(--am-flaeche-2);
    border-bottom: 1px solid var(--am-rand);
    flex-shrink: 0;
  }
  .selection-count {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--am-handlung-ruhend);
  }
  .selection-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    align-items: center;
    gap: var(--am-raum-2);
  }
  .list-scroll-wrapper {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  /* ── Undo toast after a delete [RL-POSTLISTE] ────────────────────────────── */
  /* "In den Papierkorb verschoben · Rückgängig" — floats, so it carries the
     one shadow and the emphasised border (CI R6). */
  /* ── Clean-up entry in the mail column [RL-AUFRAEUMEN] ────────────────────
     An HB-UNTERNAV row under the accounts: 40 px, 20 px sign; chosen = gold
     edge, bold, no surface. Only with the switch on. */
  .mail-aufraeumen {
    display: flex;
    align-items: center;
    gap: var(--am-raum-2);
    width: 100%;
    min-height: 40px;
    margin-top: var(--am-raum-2);
    padding: 8px 14px;
    font-size: 0.8125rem;
    border: none;
    border-radius: var(--am-radius-mittel);
    background: none;
    color: var(--am-text-sekundaer);
    font-family: inherit;
    text-align: left;
    cursor: pointer;
  }
  .mail-aufraeumen:hover { background: var(--am-flaeche-2); color: var(--am-text-primaer); }
  .mail-aufraeumen.aktiv {
    font-weight: 600;
    color: var(--am-text-primaer);
    box-shadow: inset 2px 0 0 var(--am-gold-auszeichnung);
  }
  .mail-aufraeumen.aktiv :global(svg) { color: var(--am-gold-beschriftung); }
  .mail-aufraeumen:focus-visible { outline: 2px solid var(--am-fokus-ring); outline-offset: 2px; }

  .undo-toast {
    position: fixed;
    left: 50%;
    bottom: calc(var(--am-raum-8) + env(safe-area-inset-bottom, 0px));
    transform: translateX(-50%);
    z-index: var(--am-ebene-menue);
    display: flex;
    align-items: center;
    gap: var(--am-raum-4);
    padding: var(--am-raum-2) var(--am-raum-2) var(--am-raum-2) var(--am-raum-4);
    background: var(--am-flaeche-3);
    color: var(--am-text-primaer);
    border: 1px solid var(--am-rand-betont-farbe);
    border-radius: var(--am-radius-mittel);
    box-shadow: var(--am-schatten-1);
    font-size: var(--fs-base);
  }

  /* ── Reading pane: header, subject, body [RL-LESEANSICHT] ────────────────── */
  .preview-layout {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  /* Wraps when the actions do not fit beside the sender (phone, a narrow
     reading pane, a third button such as "Abo beenden"). */
  .preview-pane-header {
    min-height: var(--am-leistenhoehe);
    padding: var(--am-raum-2) 24px;
    display: flex;
    flex-wrap: wrap;
    gap: var(--am-raum-2) var(--am-raum-4);
    align-items: center;
    justify-content: space-between;
    /* Linie unter dem Vorschau-Header unsichtbar (gleiche Farbe wie Hintergrund) */
    border-bottom: 1px solid var(--am-seite);
    background: var(--am-seite);
    flex-shrink: 0;
  }
  .preview-header-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .preview-from-name {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--am-text-primaer);
  }
  .preview-from-email {
    font-size: 0.75rem;
    color: var(--am-text-gedaempft);
  }
  /* ── Toolbar of signs [RL-WERKZEUGLEISTE] ────────────────────────────────
     Signs in groups, a thin divider between groups (CI G4, Kai 07.10.2026;
     Relay builds it first, then it goes to the CI as an HB block). Wraps
     whole groups when the pane is narrow. */
  .werkzeugleiste {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--am-raum-1);
    margin-left: auto;
  }
  .werkzeug-gruppe {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .werkzeug-gruppe + .werkzeug-gruppe {
    padding-left: var(--am-raum-1);
    border-left: 1px solid var(--am-trennlinie);
  }
  .werkzeugleiste .btn-symbol[aria-pressed="true"] {
    color: var(--am-gold-auszeichnung);
  }
  /* "Abo beenden" as a word beside the sender, like a link. */
  .preview-abmelden {
    border: 0;
    padding: 0;
    background: none;
    font: inherit;
    color: var(--am-text-primaer);
    text-decoration: underline;
    text-underline-offset: 2px;
    cursor: pointer;
  }
  .preview-abmelden:disabled {
    color: var(--am-text-deaktiviert);
    cursor: default;
  }
  .preview-abmelden:focus-visible {
    outline: var(--am-fokus-ring);
    outline-offset: 2px;
  }
  .preview-back-bar {
    flex-shrink: 0;
    padding: 8px 12px;
    border-bottom: 1px solid var(--am-rand);
    background: var(--am-seite);
  }
  .preview-scroll-wrapper {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
  }
  .preview-scroll-wrapper::-webkit-scrollbar {
    width: 6px;
  }
  .preview-scroll-wrapper::-webkit-scrollbar-track {
    background: transparent;
  }
  .preview-scroll-wrapper::-webkit-scrollbar-thumb {
    background: var(--am-rand);
    border-radius: 3px;
  }
  .preview-scroll-wrapper::-webkit-scrollbar-thumb:hover {
    background: var(--am-text-gedaempft);
  }
  .preview-content-area {
    padding: 32px 24px 24px 24px;
    max-width: 800px;
  }
  .preview-subject-large {
    margin-top: 0;
    font-size: 1.5rem;
    font-weight: 700;
    margin-bottom: 8px;
    color: var(--am-text-primaer);
    line-height: 1.3;
  }
  .preview-date-line {
    font-size: 0.75rem;
    color: var(--am-text-gedaempft);
  }
  .preview-recipients {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 8px 0 4px;
    font-size: 0.78rem;
    color: var(--am-text-gedaempft);
  }
  .preview-recipient-line strong {
    color: var(--am-text-primaer);
    font-weight: 600;
  }
  .preview-body {
    font-size: 0.875rem;
    line-height: 1.7;
    color: var(--am-text-primaer);
  }
  .mail-iframe-container {
    background: var(--am-seite);
    /* Umrandung der Mail-Vorschau unsichtbar */
    border: 1px solid transparent;
    border-radius: 8px;
    overflow: hidden;
    box-shadow: none;
    margin-bottom: 16px;
    height: calc(100vh - 280px);
    min-height: 450px;
    width: 100%;
  }
  .mail-iframe {
    content-visibility: auto;
    width: 100%;
    height: 100%;
    border: none;
    display: block;
    background: transparent;
  }
  .mail-body {
    font-family: inherit;
    font-size: 0.875rem;
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
    margin: 0;
    color: var(--am-text-primaer);
  }
  .mail-body-empty {
    font-size: 0.875rem;
    color: var(--am-text-gedaempft);
    font-style: italic;
  }

  /* ── Attachment chips [RL-LESEANSICHT] ───────────────────────────────────── */
  .attachments {
    margin-top: 20px;
    padding-top: 16px;
    border-top: 1px solid var(--am-rand);
  }
  .attachments-title {
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--am-text-gedaempft);
    margin-bottom: 10px;
  }
  .attachments-list {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }
  .attachment-chip {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border: 1px solid var(--am-rand);
    border-radius: 8px;
    background: var(--am-seite);
    cursor: pointer;
    font-family: inherit;
    text-align: left;
    max-width: 260px;
    transition: all var(--am-dauer-schnell) var(--am-kurve);
  }
  .attachment-chip:hover:not(:disabled) {
    border-color: var(--am-handlung-ruhend);
    background: var(--am-flaeche-2);
  }
  .attachment-chip:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .attachment-icon {
    font-size: 1.125rem;
    flex-shrink: 0;
  }
  .attachment-meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .attachment-name {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--am-text-primaer);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .attachment-size {
    font-size: 0.6875rem;
    color: var(--am-text-gedaempft);
  }

  /* ── Attachment preview, on HB-DIALOG [RL-LESEANSICHT] ───────────────────── */
  /* A viewer, not a question: wider and taller than the breit dialog, the
     body edge to edge on a dark ground. */
  .att-preview-karte {
    max-width: min(57.5rem, 100%);
    max-height: calc(100dvh - var(--am-raum-8));
  }
  .att-preview-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .att-preview-body {
    padding: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--am-blau-950);
  }
  .att-preview-frame {
    width: 100%;
    height: 100%;
    border: none;
    min-height: 55vh;
  }
  .att-preview-image {
    max-width: 100%;
    max-height: 75vh;
    object-fit: contain;
  }
  .att-preview-unsupported {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 14px;
    padding: 40px;
    color: var(--am-text-gedaempft);
    font-size: 0.875rem;
    text-align: center;
  }
  @media (max-width: 600px) {
    .att-preview-schicht {
      padding: 0;
    }
    .att-preview-karte {
      max-width: none;
      max-height: 100dvh;
      height: 100%;
      border: none;
      border-radius: 0;
    }
    .att-preview-frame {
      min-height: 0;
    }
  }

  /* ── Loading skeleton of the reading pane [RL-LESEANSICHT] ───────────────── */
  .preview-skeleton {
    padding: 32px 24px;
    max-width: 800px;
    display: flex;
    flex-direction: column;
    gap: 0;
  }
  .preview-skeleton-header {
    margin-bottom: 32px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .preview-skeleton-body {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .skeleton-line {
    height: 12px;
    border-radius: 6px;
    background: linear-gradient(
      90deg,
      var(--am-rand) 0%,
      var(--am-flaeche-2) 40%,
      var(--am-flaeche-2) 60%,
      var(--am-rand) 100%
    );
    background-size: 200% 100%;
    animation: previewShimmer 1.8s ease-in-out infinite;
  }
  .skeleton-preview-subject {
    width: 320px;
    height: 22px;
    border-radius: 8px;
  }
  .skeleton-preview-sender {
    width: 180px;
    height: 14px;
  }
  .skeleton-preview-date {
    width: 120px;
    height: 11px;
    opacity: 0.6;
  }
  .skeleton-body-line {
    height: 13px;
  }
  .w-100 { width: 100%; }
  .w-95 { width: 95%; }
  .w-90 { width: 90%; }
  .w-85 { width: 85%; }
  .w-80 { width: 80%; }
  .w-75 { width: 75%; }
  .w-70 { width: 70%; }
  .w-60 { width: 60%; }
  @keyframes previewShimmer {
    0% { background-position: 200% 0; }
    100% { background-position: -200% 0; }
  }

  /* ── AI follow-up suggestions footer [RL-VORSCHLAEGE] ────────────────────── */
  .followups-footer {
    flex-shrink: 0;
    margin-top: auto;
    border-top: 1px solid var(--am-rand);
    background: var(--am-flaeche-1);
    display: flex;
    flex-direction: column;
    /* Fixed height: 107px from the line to the bottom edge (incl. 1px border). */
    height: 107px;
    box-sizing: border-box;
    padding: 12px 16px;
    gap: 8px;
  }
  .followups-footer-head {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .followups-footer-title {
    font-size: 0.72rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--am-handlung-ruhend);
  }
  /* HB-ZUSTAND line; one line only, the footer has a fixed height. */
  .followups-footer-error {
    min-width: 0;
  }
  .followups-footer-error > span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .followups-footer-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .followups-footer-row {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }
  .followups-footer-label {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 0 1 auto;
    font-size: 0.85rem;
    color: var(--am-text-primaer);
  }
  .followups-footer-label > span:first-child {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .followups-footer-muted {
    font-size: 0.85rem;
    color: var(--am-text-gedaempft);
  }
  .followups-footer-btn {
    flex-shrink: 0;
  }

  /* ── Context menus: folder, move, link, attachment [RL-MENUE] ────────────── */
  .ctx-menu-scrim {
    position: fixed;
    inset: 0;
    z-index: 1000;
  }
  .ctx-menu-scrim.sheet-scrim {
    background: var(--am-deckschicht);
  }
  .ctx-menu {
    position: fixed;
    z-index: 1001;
    min-width: 200px;
    max-width: 320px;
    max-height: min(70vh, 560px);
    overflow-y: auto;
    background: var(--am-seite);
    border: 1px solid var(--am-rand);
    border-radius: 8px;
    box-shadow: none;
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ctx-menu-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    text-align: left;
    padding: var(--am-raum-2) var(--am-raum-4);
    border: none;
    background: none;
    border-radius: 6px;
    font-size: 0.875rem;
    line-height: 1.45;
    color: var(--am-text-primaer);
    cursor: pointer;
    font-family: inherit;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ctx-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    flex: none;
    color: var(--am-text-gedaempft);
  }
  .ctx-menu-item:hover .ctx-icon {
    color: var(--am-handlung-ruhend);
  }
  .ctx-menu-item.danger .ctx-icon {
    color: var(--am-fehler);
  }
  .ctx-menu-header {
    padding: 6px 12px 2px;
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--am-text-gedaempft);
    user-select: none;
  }
  .ctx-menu-item:hover {
    background: var(--am-flaeche-2);
    color: var(--am-handlung-ruhend);
  }
  .ctx-menu-item.danger {
    color: var(--am-fehler);
  }
  .ctx-menu-item.danger:hover {
    background: var(--am-fehler-flaeche);
    color: var(--am-fehler);
  }
  .ctx-menu-separator {
    height: 1px;
    margin: 4px 8px;
    background: var(--am-rand);
  }

  /* ── Context menu as bottom sheet on touch [RL-MENUE] ────────────────────── */
  /* iOS-style bottom sheet (touch devices): slides up from the bottom edge,
     full width, large touch targets, safe-area aware. */
  @keyframes sheetUp {
    from { transform: translateY(100%); }
    to { transform: translateY(0); }
  }
  .ctx-menu.sheet {
    left: 0;
    right: 0;
    bottom: 0;
    top: auto;
    width: 100%;
    min-width: 0;
    max-width: none;
    max-height: 65vh;
    border: none;
    border-radius: 16px 16px 0 0;
    box-shadow: none;
    padding: 8px 12px calc(12px + env(safe-area-inset-bottom, 0px));
    animation: sheetUp 0.28s cubic-bezier(0.32, 0.72, 0, 1);
  }
  .ctx-menu.sheet .ctx-menu-item {
    display: flex;
    align-items: center;
    min-height: 48px;
    padding: 12px 16px;
    font-size: 1rem;
    border-radius: 10px;
  }
  .ctx-menu.sheet .ctx-menu-item:hover {
    background: var(--am-flaeche-2);
    color: var(--am-text-primaer);
  }
  .ctx-menu.sheet .ctx-menu-item.danger:hover {
    color: var(--am-fehler);
  }
  .ctx-menu.sheet .ctx-menu-separator {
    margin: 4px 16px;
  }
</style>
