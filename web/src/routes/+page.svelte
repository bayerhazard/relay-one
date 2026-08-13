<script lang="ts">
  import { onMount } from "svelte";
  import { untrack } from "svelte";
  import { goto } from "$app/navigation";
  import MessageList from "$lib/components/MessageList.svelte";
  import FolderList from "$lib/components/FolderList.svelte";
  import AccountGroup from "$lib/components/AccountGroup.svelte";
  import PromptDialog from "$lib/components/PromptDialog.svelte";
  import ComposeWindow from "$lib/components/ComposeWindow.svelte";
  import ReplySuggestions from "$lib/components/ReplySuggestions.svelte";
  import ConfirmationDialog from "$lib/components/ConfirmationDialog.svelte";
  import SplashScreen from "$lib/components/SplashScreen.svelte";
  import ErrorBanner from "$lib/components/ErrorBanner.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import { mailbox, getFolderCache, invalidateFolderCache, type Message } from "$lib/stores/mailbox";
  import { accounts, type AccountInfo } from "$lib/stores/accounts";
import {
    fetchMessages, fetchMessageBody, markAsRead, markAsUnseen, sendMessage,
    listAccounts, fetchFromImap, listImapFolders, createLocalFolder, deleteFolder,
    deleteMessageCmd, moveMessageCmd, moveMessageCrossAccount, renameFolder, flagMessageCmd,
    getMoveToTrash, updateBadgeCount, discardDraft, searchMessages,
    triggerFolderSummaries, fetchAttachments, loadAttachmentContent, saveAttachment,
    getOwnPhoto, openEventStream,
  } from "$lib/services/tauri";
  import { formatDate, extractEmail, extractName, isHtmlContent, extractHtmlFromMime, extractPlainFromMime, type MailAttachment } from "$lib/utils/format";
  import type { MailChainEntry } from "$lib/types/mail";

  let sidebarWidth = $state(220);
  let listWidth = $state(380);
  let showCompose = $state(false);

  // ─── Responsive layout ────────────────────────────────────
  // Below `compact` the preview becomes a full-width overlay (shown only when a
  // message/compose is open); below `narrow` the sidebar collapses to a toggle.
  // At full size everything behaves exactly as before (fixed 3-column layout).
  let viewportWidth = $state(typeof window !== "undefined" ? window.innerWidth : 1440);
  let isCompact = $derived(viewportWidth <= 900);
  let isNarrow = $derived(viewportWidth <= 600);
  let sidebarOpen = $state(false); // only relevant in narrow mode (overlay)

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
        window.open(data.url, "_blank");
      }
    };
    window.addEventListener('message', handler);
    return () => window.removeEventListener('message', handler);
  });

  // Listen for backend events (new mail, AI summaries, navigation/actions)
  // via the SSE event stream (replaces the Tauri event listeners).
  let loadingBodyUid = $state<number | null>(null);

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
        if (accountId === selectedAccountId) {
          if (folderName === selectedFolder) {
            // Debounce: if a batch of new-messages arrives within 2s, only
            // reload once. Prevents rapid successive loadFolder() calls that
            // race with handleSelectMessage().
            if (newMsgTimer !== null) clearTimeout(newMsgTimer);
            newMsgTimer = setTimeout(() => {
              newMsgTimer = null;
              loadFolder();
            }, 2000);
          }
          // Always update badge count when new mail arrives
          updateBadgeCount(accountId).catch(() => {});
        }
        return;
      }
      if (event === "ai-summary-updated") {
        const [uid, accountId, summary, priority] = payload as [number, number, string, number | null];
        if (accountId === selectedAccountId) {
          const changes: Record<string, unknown> = {};
          if (summary) changes.ai_summary = summary;
          if (priority !== undefined && priority !== null) changes.ai_priority = priority;
          if (Object.keys(changes).length) {
            mailbox.updateMessage(uid, changes);
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
  let composeMode = $state<"new" | "reply">("new");
  let replySubject = $state("");
  let replyTo = $state("");
  let recipientName = $state("");
  let mailChain = $state<MailChainEntry[]>([]);
  let selectedAccountId = $state<number>(1);
  let senderName = $state("");
  let replySuggestions = $state<string[]>([]);
  let accountList = $state<Array<{id: number; name: string; username: string; connected: boolean}>>([]);
  let selectedAccount = $derived(accountList.find(a => a.id === selectedAccountId) || (accountList.length > 0 ? accountList[0] : null));
  let initError = $state<string | null>(null);
  let initOk = $state(false);
  let folderNames = $state<string[]>([]);
  let localFolderNames = $state<Set<string>>(new Set());
  let folderRawNames = $state<Record<string, string>>({});
  let folderDelimiters = $state<Record<string, string>>({});
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
  }
  let selectedFolder = $state("INBOX");
  let theme = $state("blue");
  try { theme = localStorage.getItem("relay_theme") || "blue"; } catch {}
  let showDeleteConfirm = $state(false);
  let showDeleteFolderConfirm = $state(false);
  let pendingDeleteFolder = $state<string | null>(null);
  let pendingDeleteUid = $state<number | null>(null);
  let isDeleting = $state(false);
  let moveToTrash = $state(true);
  let ownPhoto = $state<{ data: string; type: string } | null>(null);
  let fetchLimit = $state(50);
  try { const v = localStorage.getItem("relay_fetch_limit"); if (v) fetchLimit = parseInt(v, 10) || 50; } catch {}
  let draftsFolderName = $state<string | null>(null);
let sentFolderName = $state<string | null>(null);
  let draftUid = $state<number | null>(null);
  let draftTo = $state("");
  let draftSubject = $state("");
  let draftBody = $state("");

  let showSplash = $state(false);

  async function handleSplashComplete(acct: AccountInfo) {
    const accts = await listAccounts();
    accounts.setAccounts(accts);
    await initWithAccount(acct);
    showSplash = false;
  }

  async function initWithAccount(acct: any) {
    selectedAccountId = acct.id;
    accounts.selectAccount(acct.id);
    senderName = acct.sender_name || acct.name || "";

   // Restore cached folder list from localStorage so sidebar never goes blank
    // during navigation (SPA route change from settings back to inbox).
    try {
      const cacheKey = `relay_folder_cache_${acct.id}`;
      const cached = localStorage.getItem(cacheKey);
      if (cached) {
        const parsed = JSON.parse(cached) as string[];
        if (Array.isArray(parsed) && parsed.length > 0) {
          folderNames = parsed;
          // Seed the per-account cache too (raw/delim defaults to "."), so
          // other account groups render their trees immediately.
          setAccountFolders(acct.id, { names: parsed, local: new Set(), raw: {}, delim: {} });
        }
      }
    } catch { /* ignore stale cache */ }

    // Fetch fresh folder list from IMAP (up to 3 retries for transient failures)
    let foldersFetched = false;
    for (let attempt = 0; attempt < 3; attempt++) {
      try {
        const f = await listImapFolders(acct.id);
        const seen = new Set<string>();
        const names: string[] = [];
        const rawMap: Record<string, string> = {};
        const delimMap: Record<string, string> = {};
        const localSet = new Set<string>();
        // Detect Drafts & Sent folders from SPECIAL-USE attributes or fallback names
        draftsFolderName = null;
        sentFolderName = null;
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
          // Check if this folder is the Drafts folder (SPECIAL-USE or fallback)
          if (!draftsFolderName && (x.attributes?.some(a => a.includes("Drafts")) || draftFallbacks.includes(key))) {
            draftsFolderName = x.name;
          }
          // Check if this folder is the Sent folder (SPECIAL-USE or fallback)
          if (!sentFolderName && (x.attributes?.some(a => a.includes("Sent")) || sentFallbacks.includes(key))) {
            sentFolderName = x.name;
          }
        }
        folderNames = names;
        folderRawNames = rawMap;
        folderDelimiters = delimMap;
        localFolderNames = localSet;
       // Persist to localStorage cache for fast recovery on navigation
        const cacheKey = `relay_folder_cache_${acct.id}`;
        localStorage.setItem(cacheKey, JSON.stringify(names));
        setAccountFolders(acct.id, { names, local: localSet, raw: rawMap, delim: delimMap });
        // Apply saved folder order
        try {
          const orderKey = `relay_folder_order_${acct.id}`;
          const saved = localStorage.getItem(orderKey);
          if (saved) {
            const order = JSON.parse(saved) as string[];
            const ordered = order.filter((n) => names.includes(n));
            const remaining = names.filter((n) => !ordered.includes(n));
            folderNames = [...ordered, ...remaining];
          }
        } catch (e) { console.warn("Folder order parse failed", e); }
        foldersFetched = true;
        break;
      } catch (e: unknown) {
        if (attempt < 2) {
          await new Promise(r => setTimeout(r, 1000 * (attempt + 1)));
        } else {
          console.warn("listImapFolders failed after retries, using cached list", e);
        }
      }
    }
    if (!foldersFetched && folderNames.length === 0) {
      initError = "Ordnerliste konnte nicht geladen werden. Bitte prüfe deine Internetverbindung.";
    }

    try {
      await fetchFromImap(acct.id, "INBOX", fetchLimit);
      updateBadgeCount(acct.id).catch(() => {});
    } catch (e: unknown) {
      const errMsg = e instanceof Error ? e.message : String(e);
      if (!isTransientConnError(errMsg)) {
        initError = "IMAP: " + errMsg;
      }
    }

   // Load collapsed folders for this account
    try {
      const raw = localStorage.getItem(`relay_folder_collapsed_${acct.id}`);
      if (raw) {
        collapsedFoldersMap = { ...collapsedFoldersMap, [acct.id]: new Set(JSON.parse(raw) as string[]) };
      }
    } catch { /* ignore */ }
  }

  // ─── Folder Customization (Rename/Hide) ──────────────────
  let customFolderNames = $state<Record<string, string>>({});
  let hiddenFolderNames = $state<string[]>([]);

  // Helper: per-account localStorage keys
  function getStoreKey(base: string): string {
    return `relay_${base}_${selectedAccountId}`;
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
      mailbox.setError("Server-Umbenennung fehlgeschlagen: " + (e instanceof Error ? e.message : String(e)));
    }
  }

  // ─── Plain HTML context menus (replaces the Tauri native menus) ────────
  let folderCtxMenu = $state<{ x: number; y: number; folderName: string } | null>(null);
  let moveMenu = $state<{ x: number; y: number; targets: Array<{ name: string; label: string }> } | null>(null);

  function closeMenus() {
    folderCtxMenu = null;
    moveMenu = null;
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
      mailbox.setError("Ordner konnte nicht gelöscht werden: " + (e instanceof Error ? e.message : String(e)));
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

  // The button-triggered "move selected to folder" menu (replaces the Tauri menu).
  async function moveSelectedToFolder(e: MouseEvent) {
    if (movingSelection) return;
    const uids = [...$mailbox.selectedUids];
    if (uids.length === 0) return;

    const targets = folderNames
      .filter((name) => name !== selectedFolder)
      .map((name) => ({ name, label: customFolderNames[name] || translateFolder(name) }));
    if (targets.length === 0) return;

    const rect = (e.currentTarget as HTMLElement | null)?.getBoundingClientRect();
    const pos = clampMenuPosition(rect?.left ?? e.clientX, (rect?.bottom ?? e.clientY) + 4, 220, 320);
    moveMenu = { x: pos.x, y: pos.y, targets };
  }

  $effect(() => {
    if (typeof document !== 'undefined') {
      if (theme === "dark") {
        document.documentElement.classList.add("theme-dark");
      } else {
        document.documentElement.classList.remove("theme-dark");
      }
      localStorage.setItem("relay_theme", theme);
      // macOS: the web-app titlebar uses theme-color — match it to the theme
      // (was a fixed medium blue that looked wrong in both themes).
      let meta = document.querySelector('meta[name="theme-color"]') as HTMLMetaElement | null;
      if (!meta) {
        meta = document.createElement("meta");
        meta.name = "theme-color";
        document.head.appendChild(meta);
      }
      meta.content = theme === "dark" ? "#0a2238" : "#f4f7fa";
    }
  });

  function getFolderIcon(name: string): string {
    const lower = name.toLowerCase();
    if (lower === "inbox") {
      return `<svg class="folder-svg-icon" aria-hidden="true" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" d="M2.25 13.5h3.86a2.25 2.25 0 012.008 1.24l.885 1.77a2.25 2.25 0 002.007 1.24h1.98a2.25 2.25 0 002.007-1.24l.885-1.77a2.25 2.25 0 012.007-1.24h3.86m-18 0h18m-18 0v-7.5A2.25 2.25 0 014.5 4.5h15a2.25 2.25 0 012.25 2.25v7.5m-18 0v6a2.25 2.25 0 002.25 2.25h15a2.25 2.25 0 002.25-2.25v-6" /></svg>`;
    }
    if (lower === "sent") {
      return `<svg class="folder-svg-icon" aria-hidden="true" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" d="M6 12L3.269 3.126A59.768 59.768 0 0121.485 12 59.77 59.77 0 013.27 20.876L5.999 12zm0 0h7.5" /></svg>`;
    }
    if (lower === "drafts" || lower === "entwürfe") {
      return `<svg class="folder-svg-icon" aria-hidden="true" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" d="M19.5 14.25v-2.625a3.375 3.375 0 00-3.375-3.375h-1.5A1.125 1.125 0 0113.5 7.125v-1.5a3.375 3.375 0 00-3.375-3.375H8.25m0 12.75h7.5m-7.5 3H12M10.5 2.25H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 00-9-9z" /></svg>`;
    }
    if (lower === "trash" || lower === "gelöscht") {
      return `<svg class="folder-svg-icon" aria-hidden="true" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" /></svg>`;
    }
    if (lower === "archive" || lower === "archiv") {
      return `<svg class="folder-svg-icon" aria-hidden="true" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" d="M20.25 7.5l-.625 10.632a2.25 2.25 0 01-2.247 2.118H6.622a2.25 2.25 0 01-2.247-2.118L3.75 7.5M10 11.25h4M3.375 7.5h17.25c.621 0 1.125-.504 1.125-1.125v-1.5c0-.621-.504-1.125-1.125-1.125H3.375c-.621 0-1.125.504-1.125 1.125v1.5c0 .621.504 1.125 1.125 1.125z" /></svg>`;
    }
    if (lower === "spam" || lower === "junk" || lower === "spamverdacht") {
      return `<svg class="folder-svg-icon" aria-hidden="true" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" d="M12 9v3.75m0-10.036A11.959 11.959 0 013.598 6 11.99 11.99 0 003 9.75c0 5.592 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.31-.21-2.57-.598-3.75h-.152c-3.196 0-6.1-1.249-8.25-3.286zm0 13.036h.008v.008H12v-.008z" /></svg>`;
    }
    return `<svg class="folder-svg-icon" aria-hidden="true" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.75" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" d="M2.25 12.75V12A2.25 2.25 0 014.5 9.75h15A2.25 2.25 0 0121.75 12v.75m-19.5 0A2.25 2.25 0 004.5 15h15a2.25 2.25 0 002.25-2.25m-19.5 0v.25A2.25 2.25 0 004.5 20.25h15a2.25 2.25 0 002.25-2.25v-.25m-18-10.5h4a1.5 1.5 0 001.108-.491L8.51 4.51A1.5 1.5 0 019.617 4H19.5a1.5 1.5 0 011.5 1.5v3" /></svg>`;
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
        const label = customFolderNames[name] || customFolderNames[leafName] || translateFolder(leafName);
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
      const label = customFolderNames[name] || customFolderNames[leafName] || translateFolder(leafName);
      const parentName = parts[0];
      let parent = level1Map.get(parentName.toLowerCase());
      if (!parent) {
        // Parent folder is not a standalone entry (e.g. only exists as a
        // prefix) — synthesize it so the hierarchy stays intact.
        const parentLeaf = getLeafName(parentName, delimiter);
        const parentLabel = customFolderNames[parentName] || customFolderNames[parentLeaf] || translateFolder(parentLeaf);
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
    sidebarOpen = false;
  }

  // Select folder from a specific account
  function handleAccountFolderSelect(accountId: number, folder: string) {
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
    sidebarOpen = false;
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
    if (targetAccountId && targetAccountId !== selectedAccountId) {
      // Cross-account move
      moveMessageCrossAccount(selectedAccountId, uid, selectedFolder, targetAccountId, targetFolder)
        .then(() => {
          invalidateFolderCache(selectedAccountId, selectedFolder);
          invalidateFolderCache(targetAccountId, targetFolder);
          loadFolder();
        })
        .catch((e) => {
          console.warn("Cross-Account-Verschieben fehlgeschlagen", e);
          mailbox.setError("Die Nachricht konnte nicht verschoben werden: " + (e instanceof Error ? e.message : String(e)));
        });
      return;
    }
    if (selectedFolder === targetFolder) return;
    const rawSource = folderRawNames[selectedFolder] || selectedFolder;
    const rawTarget = folderRawNames[targetFolder] || targetFolder;
    moveMessageCmd(selectedAccountId, uid, selectedFolder, targetFolder, rawSource, rawTarget)
      .then(() => {
        invalidateFolderCache(selectedAccountId, selectedFolder);
        invalidateFolderCache(selectedAccountId, targetFolder);
        loadFolder();
      })
      .catch((e) => {
        console.warn("Verschieben fehlgeschlagen", e);
        mailbox.setError("Die Nachricht konnte nicht verschoben werden: " + (e instanceof Error ? e.message : String(e)));
      });
  }

  // isHtmlContent, extractHtmlFromMime, extractPlainFromMime imported from $lib/utils/format

  let parsedContent = $state<{ html: string | null; text: string | null }>({ html: null, text: null });

  // Sync fallback: compute parsed content synchronously using regex
  function computeParsedContent(msg: Message | null): { html: string | null; text: string | null } {
    if (!msg) return { html: null, text: null };
    const html = msg.body_html || null;
    const txt = msg.body_text || null;

    if (html) return { html, text: txt };

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

    return { html: null, text: txt };
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

    const isDark = theme === "dark";
    const bg = isDark ? "#0a2238" : "#ffffff";
    const fg = isDark ? "#d3dae1" : "#0a2238";
    const muted = isDark ? "#6683a2" : "#6683a2";
    const linkColor = isDark ? "#c9a45c" : "#3f6082";
    const quoteBar = isDark ? "#294766" : "#d3dae1";

    // Check auto-download images setting
    const autoDownload = localStorage.getItem("relay_auto_download_images") !== "false";

    // Defense in depth: the iframe is already sandboxed (no scripts, no
    // same-origin). This CSP also forbids any script execution and active /
    // framed content inside the rendered email while allowing inline styles
    // and images, neutralising active content from untrusted senders.
    const imgSrc = autoDownload ? "https: http: data: cid:" : "data: cid:";
    const cspMeta =
      `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; ` +
      `img-src ${imgSrc}; style-src 'unsafe-inline'; font-src https: data:; ` +
      `script-src 'self'; object-src 'none'; frame-src 'none'; base-uri 'none'; form-action 'none'">`;

    // Placeholder styling for blocked images
    const phBg = isDark ? "#142e47" : "#E8E8E8";
    const phBorder = isDark ? "#294766" : "#D0D0D0";
    const phText = isDark ? "#6683a2" : "#718096";
    const phHoverBg = isDark ? "#294766" : "#E0E0E0";

    const baseStyle = `
      ${cspMeta}
      <meta name="viewport" content="width=device-width, initial-scale=1">
      <style>
        html, body { margin: 0; padding: 0; }
        body {
          font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
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
        return `<span class="img-placeholder" data-src="${safeUrl}" onclick="loadImage(this)">&#x1F5BC; ${shortUrl}<br><small>Bild laden</small></span>`;
      });
    }

    // Inject inline script for click-to-load when auto-download is off
    const loadScript = !autoDownload
      ? '<' + 'script>function loadImage(el){var u=el.dataset.src;var d=document.createElement(\'img\');d.src=u;d.style.maxWidth=\'100%\';d.style.height=\'auto\';el.replaceWith(d);}</' + 'script>'
      : '';

    // Inject link-click handler to open URLs in system browser
    const linkScript = '<' + 'script>document.addEventListener(\'click\',function(e){var t=e.target;while(t&&t.nodeName!==\'A\'){t=t.parentElement}if(t&&t.href&&t.hostname){e.preventDefault();parent.postMessage({type:\'open-url\',url:t.href},\'*\')}});</' + 'script>';

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

  function startResize(e: MouseEvent, target: 'sidebar' | 'list') {
    e.preventDefault();
    const startX = e.clientX;
    const startW = target === 'sidebar' ? sidebarWidth : listWidth;
    const ac = new AbortController();
    activeDragControllers.add(ac);
    const { signal } = ac;
    function finish() {
      activeDragControllers.delete(ac);
      ac.abort();
    }
    function onMove(ev: MouseEvent) {
      const dx = ev.clientX - startX;
      if (target === 'sidebar') {
        sidebarWidth = Math.max(140, Math.min(400, startW + dx));
      } else {
        listWidth = Math.max(200, Math.min(700, startW + dx));
      }
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
          const fromIdx = folderNames.indexOf(dragSource);
          const toIdx = folderNames.indexOf(dragTarget);
          if (fromIdx >= 0 && toIdx >= 0) {
            const reordered = [...folderNames];
            const [movedName] = reordered.splice(fromIdx, 1);
            reordered.splice(toIdx, 0, movedName);
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
        ghost.style.cssText = `position:absolute;left:-9999px;width:200px;padding:6px 10px;background:var(--color-list);color:var(--color-text);border-radius:8px;font-size:12px;box-shadow:0 2px 12px rgba(0,0,0,0.2);line-height:1.4;`;
        const sender = document.createElement("div");
        sender.style.cssText = "font-weight:600;margin-bottom:2px;";
        sender.textContent = extractName(msg.from) || "Unbekannt";
        const subject = document.createElement("div");
        subject.style.cssText = "font-weight:400;color:var(--color-text-secondary);overflow:hidden;text-overflow:ellipsis;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;";
        subject.textContent = msg.subject || "(Kein Betreff)";
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
  let searchFocused = $state(false);
  let searchActive = $state(false);
  let searchSeq = 0;
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

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

  // Reload when folder changes - fetch from IMAP then read cache.
  // A folder switch always exits search mode.
  $effect(() => {
    if (selectedFolder && selectedAccountId > 0) {
      searchActive = false;
      searchQuery = "";
      // untrack(): loadFolder() reads $mailbox.lastClickedUid internally; without
      // this, the effect depends on the mailbox store and its own setMessages()
      // notification re-triggers the effect → infinite reload loop (OOM).
      untrack(() => loadFolder());
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

 async function loadFolder() {
    if (loadingFolder) return;
    loadingFolder = true;
    folderGen++;
    const thisGen = folderGen;
    // Snapshot the requested target. If the user switches folder/account while
    // we await, the results are stale and must be discarded; we then re-run for
    // the latest selection. This prevents showing the wrong folder's content.
    const reqFolder = selectedFolder;
    const reqAccount = selectedAccountId;
    mailbox.setLoading(true);
    mailbox.setError(null);
    const prevLastClicked = $mailbox.lastClickedUid;
    // Instant cache-first: if a meta-only list for this (account, folder) is
    // cached, render it immediately so switching back to a large folder feels
    // instant. Bodies stay intact via the setMessages() merge; a background
    // refresh below replaces the list with fresh metadata.
    const cachedMsgs = getFolderCache(reqAccount, reqFolder);
    if (cachedMsgs && cachedMsgs.length > 0) {
      mailbox.setMessages(cachedMsgs, reqFolder, reqAccount);
    }
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
      // Trigger background AI summaries for messages without one
      triggerFolderSummaries(reqAccount, reqFolder).catch(() => {});
      if (prevLastClicked != null && !msgs.some(m => m.uid === prevLastClicked)) {
        mailbox.clearSelection();
      }
      updateBadgeCount(reqAccount).catch(() => {});
    } catch (e: unknown) {
      const errMsg = e instanceof Error ? e.message : String(e);
      if (!isTransientConnError(errMsg) && reqFolder === selectedFolder && reqAccount === selectedAccountId) {
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
    let accts: AccountInfo[] = [];
    try {
      accts = await listAccounts();
    } catch (e: unknown) {
      // Keine Kontenliste verfügbar (z.B. Backend nicht erreichbar) → trotzdem
      // den Splash (Konto-Einrichtung) zeigen statt einer leeren App.
      console.error("[init] listAccounts fehlgeschlagen:", e);
    }
    accounts.setAccounts(accts);
    if (accts.length > 0) {
      // Poll until at least one account is connected (max 15s)
      let ready = accts.some(a => a.connected);
      if (!ready) {
        for (let i = 0; i < 30; i++) {
          await new Promise(r => setTimeout(r, 500));
          accts = await listAccounts();
          accounts.setAccounts(accts);
          if (accts.some(a => a.connected)) {
            ready = true;
            break;
          }
        }
      }
      if (ready) {
        await initWithAccount(accts[0]);
      } else {
        initError = "IMAP-Verbindung konnte nicht hergestellt werden";
      }
    } else {
      showSplash = true;
    }
    initOk = true;
    try {
      moveToTrash = await getMoveToTrash();
    } catch (e) { /* use default */ }

    // Load own photo
    try {
      ownPhoto = await getOwnPhoto();
    } catch {}
  });

  async function loadInbox() {
    mailbox.setLoading(true);
    try {
      const msgs = await fetchMessages(selectedAccountId, 10000, 0, selectedFolder, true);
      mailbox.setMessages(msgs, selectedFolder);
      updateBadgeCount(selectedAccountId).catch(() => {});
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
        initError = "Startfehler: " + errMsg;
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
        const full = await fetchMessageBody(selectedAccountId, uid);
        if (lastClickedUid !== uid) return;
        draftUid = uid;
        draftTo = full.to || "";
        draftSubject = full.subject || "";
        draftBody = full.body_text || full.body_preview || "";
        composeMode = "new";
        sendError = null;
        replyTo = "";
        recipientName = "";
        replySubject = "";
        mailChain = [];
        showCompose = true;
        mailbox.updateMessage(uid, { is_read: true, body_text: full.body_text, body_html: full.body_html });
        loadingBodyUid = null;
        selectingUid = null;
      } catch (e: unknown) {
        console.warn("Draft laden fehlgeschlagen für uid", uid, e);
        loadingBodyUid = null;
        selectingUid = null;
      }
      return;
    }

    try {
      await markAsRead(selectedAccountId, uid);
      if (lastClickedUid !== uid) return;
      const full = await fetchMessageBody(selectedAccountId, uid);
      if (lastClickedUid !== uid) return;
      // Check if a concurrent loadFolder() already delivered body_text
      // (Fix 1 ensures message_to_json includes body_text in list responses).
      // Only overwrite if the store doesn't already have body_text for this uid.
      const existing = $mailbox.messages.find((m) => m.uid === uid);
      if (!existing?.body_text || existing.body_text.length < (full.body_text?.length ?? 0)) {
        mailbox.updateMessage(uid, { is_read: true, body_text: full.body_text, body_html: full.body_html });
      } else {
        // body already in the store (put there by setMessages with Fix 1);
        // still mark as read but don't wipe the full body.
        mailbox.updateMessage(uid, { is_read: true });
      }
      loadingBodyUid = null;
      selectingUid = null;
      updateBadgeCount(selectedAccountId).catch(() => {});
    } catch (e: unknown) {
      console.warn("handleSelectMessage fehlgeschlagen für uid", uid, e);
      loadingBodyUid = null;
      selectingUid = null;
    }
  }

  function handleNewMail() {
    composeMode = "new";
    sendError = null;
    replyTo = "";
    recipientName = "";
    replySubject = "";
    mailChain = [];
    showCompose = true;
  }

  async function handleReply(msg: Message) {
    composeMode = "reply";
    sendError = null;
    replySubject = msg.subject ?? "";
    replyTo = extractEmail(msg.from ?? "");
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
        const full = await fetchMessageBody(selectedAccountId, msg.uid);
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

  function closeCompose() {
    showCompose = false;
  }

  function isInputFocused(): boolean {
    const tag = (document.activeElement?.tagName || "").toUpperCase();
    const editable = document.activeElement?.getAttribute("contenteditable") === "true";
    return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || editable;
  }

  async function toggleReadStatus() {
    const uids = $mailbox.selectedUids;
    if (uids.length === 0) return;
    for (const uid of uids) {
      const msg = $mailbox.messages.find((m) => m.uid === uid);
      if (!msg) continue;
      if (msg.is_read) {
        try {
          await markAsUnseen(selectedAccountId, uid);
          mailbox.updateMessage(uid, { is_read: false });
        } catch (e) {
          console.warn("toggleReadStatus fehlgeschlagen fuer uid", uid, e);
        }
      } else {
        try {
          await markAsRead(selectedAccountId, uid);
          mailbox.updateMessage(uid, { is_read: true });
        } catch (e) {
          console.warn("toggleReadStatus fehlgeschlagen fuer uid", uid, e);
        }
      }
    }
    updateBadgeCount(selectedAccountId).catch(() => {});
  }

  // Mark all selected messages as read (toolbar action).
  async function markSelectedRead() {
    const uids = [...$mailbox.selectedUids];
    for (const uid of uids) {
      const msg = $mailbox.messages.find((m) => m.uid === uid);
      if (!msg || msg.is_read) continue;
      try {
        await markAsRead(selectedAccountId, uid);
        mailbox.updateMessage(uid, { is_read: true });
      } catch (e) {
        console.warn("markSelectedRead fehlgeschlagen fuer uid", uid, e);
      }
    }
    updateBadgeCount(selectedAccountId).catch(() => {});
  }

  // Move all selected messages to a folder chosen from a plain HTML menu.
  let movingSelection = $state(false);

  async function performMoveSelected(uids: number[], targetFolder: string) {
    if (movingSelection || targetFolder === selectedFolder) return;
    movingSelection = true;
    const rawSource = folderRawNames[selectedFolder] || selectedFolder;
    const rawTarget = folderRawNames[targetFolder] || targetFolder;
    try {
      for (const uid of uids) {
        try {
          await moveMessageCmd(selectedAccountId, uid, selectedFolder, targetFolder, rawSource, rawTarget);
        } catch (e) {
          console.warn("Verschieben fehlgeschlagen fuer uid", uid, e);
        }
      }
      mailbox.clearSelection();
      invalidateFolderCache(selectedAccountId, selectedFolder);
      invalidateFolderCache(selectedAccountId, targetFolder);
      await loadFolder();
    } finally {
      movingSelection = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    // Escape: close context menus, compose or confirmation dialog, or clear multi-selection
    if (e.key === "Escape") {
      if (folderCtxMenu || moveMenu) {
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
        case "i":
          if (e.shiftKey) {
            e.preventDefault();
            toggleReadStatus();
            return;
          }
          break;
      }
    }

    // Backspace / Delete: delete selected messages
    if ((e.key === "Backspace" || e.key === "Delete" || e.key === "Del") && $mailbox.selectedUids.length > 0) {
      e.preventDefault();
      handleDeleteSelected();
    }
  }

  let isSending = $state(false);

  async function handleSend(data: { to: string; subject: string; body: string; bodyHtml: string; cc?: string; bcc?: string; attachments?: { filename: string; content: string; contentType: string }[]; aiDraft?: string | null }) {
    if (isSending) return;
    isSending = true;
    sendError = null;
    try {
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
        data.attachments,
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
      sendError = null;
      await loadInbox();
    } catch (e: unknown) {
      sendError = e instanceof Error ? e.message : String(e);
    } finally {
      isSending = false;
    }
  }

  let selectedMessage = $derived($mailbox.lastClickedUid
    ? $mailbox.messages.find((msg) => msg.uid === $mailbox.lastClickedUid) ?? null
    : null);

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
    fetchAttachments(acct, uid)
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
      const content = await loadAttachmentContent(selectedAccountId!, selectedMessage!.uid, att.id);
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
      mailbox.setError("Anhanginhalt nicht verfügbar.");
      return;
    }
    try {
      const byteChars = atob(content);
      const bytes = new Uint8Array(byteChars.length);
      for (let i = 0; i < byteChars.length; i++) bytes[i] = byteChars.charCodeAt(i);
      const blob = new Blob([bytes]);
      const url = URL.createObjectURL(blob);
      window.open(url, "_blank");
      setTimeout(() => URL.revokeObjectURL(url), 60000);
    } catch (e) {
      mailbox.setError("Anhang konnte nicht geöffnet werden: " + (e instanceof Error ? e.message : String(e)));
    }
  }

  async function handleSaveAsAttachment(att: AttachmentInfo) {
    closeAttCtxMenu();
    const content = await ensureAttachmentContent(att);
    if (!content) {
      mailbox.setError("Anhanginhalt nicht verfügbar.");
      return;
    }
    const saved = await saveAttachment(att.filename, content);
    if (!saved) {
      mailbox.setError("Anhang konnte nicht gespeichert werden.");
    }
  }

  $effect(() => {
    if (!attCtxMenu) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") closeAttCtxMenu(); };
    const onBlur = () => closeAttCtxMenu();
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onBlur);
    };
  });

  let pendingDeleteUids: number[] = $state([]);

  function handleDeleteMessage(uid: number) {
    if (showDeleteConfirm || isDeleting) return;
    pendingDeleteUids = [uid];
    showDeleteConfirm = true;
  }

  function handleDeleteSelected() {
    if (showDeleteConfirm || isDeleting) return;
    const uids = $mailbox.selectedUids;
    if (uids.length === 0) return;
    pendingDeleteUids = uids;
    showDeleteConfirm = true;
  }

  async function handleToggleRead(uid: number) {
    const msg = $mailbox.messages.find((m) => m.uid === uid);
    if (!msg) return;
    if (msg.is_read) {
      try {
        await markAsUnseen(selectedAccountId, uid);
        mailbox.updateMessage(uid, { is_read: false });
      } catch (e) {
        console.warn("handleToggleRead fehlgeschlagen fuer uid", uid, e);
      }
    } else {
      try {
        await markAsRead(selectedAccountId, uid);
        mailbox.updateMessage(uid, { is_read: true });
      } catch (e) {
        console.warn("handleToggleRead fehlgeschlagen fuer uid", uid, e);
      }
    }
    updateBadgeCount(selectedAccountId).catch(() => {});
  }

  async function handleToggleFlag(uid: number) {
    const msg = $mailbox.messages.find((m) => m.uid === uid);
    if (!msg) return;
    try {
      await flagMessageCmd(selectedAccountId, uid, selectedFolder ?? "INBOX", !msg.is_flagged);
      mailbox.updateMessage(uid, { is_flagged: !msg.is_flagged });
      invalidateFolderCache(selectedAccountId, selectedFolder ?? "INBOX");
    } catch (e) {
      console.warn("handleToggleFlag fehlgeschlagen fuer uid", uid, e);
    }
  }

  async function confirmDelete() {
    if (isDeleting) return;
    const uids = pendingDeleteUids;
    if (uids.length === 0) return;
    isDeleting = true;
    pendingDeleteUids = [];
    showDeleteConfirm = false;
    try {
      for (const uid of uids) {
        try {
          await deleteMessageCmd(selectedAccountId, uid, selectedFolder);
        } catch (e) {
          console.warn("Loeschen von uid", uid, "fehlgeschlagen", e);
        }
      }
      invalidateFolderCache(selectedAccountId, selectedFolder);
      await loadFolder();
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
      "INBOX": "Posteingang",
      "Sent": "Gesendet",
      "Drafts": "Entwürfe",
      "Trash": "Papierkorb",
      "Spam": "Spamverdacht",
      "Archive": "Archiv",
      "Junk": "Spam",
      "Gelöscht": "Papierkorb",
      "Spamverdacht": "Spamverdacht"
    };
    return dict[name] || name;
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#snippet list()}
  <MessageList
    messages={$mailbox.messages}
    selectedUids={$mailbox.selectedUids}
    onselect={handleSelectMessage}
    onselectToggle={handleSelectToggle}
    onselectRange={handleSelectRange}
    onreply={handleReplyMessage}
    ondelete={handleDeleteMessage}
    ontoggleRead={handleToggleRead}
    ontoggleFlag={handleToggleFlag}
    ondragstart={handleDragStart}
    loading={$mailbox.loading}
    accountId={selectedAccountId}
    isDraftFolder={selectedFolder === draftsFolderName}
    isSentFolder={selectedFolder === sentFolderName}
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
      accountId={selectedAccountId}
      recipientEmail={replyTo}
      recipientName={recipientName}
      senderName={senderName}
      onclose={closeCompose}
      onsend={handleSend}
      draftTo={draftUid ? draftTo : undefined}
      draftSubject={draftUid ? draftSubject : undefined}
      draftBody={draftUid ? draftBody : undefined}
      draftUid={draftUid}
    />
  {:else if selectedMessage}
    <div class="preview-layout">
      <div class="preview-pane-header">
        <div class="preview-header-meta">
          <span class="preview-from-name">{extractName(selectedMessage.from) || "Unbekannt"}</span>
          <span class="preview-from-email">{extractEmail(selectedMessage.from)}</span>
        </div>
        <div class="preview-header-actions">
          <button type="button" class="action-btn-pill" onclick={() => handleReply(selectedMessage)}>
            <span class="pill-icon" aria-hidden="true">&#x21A9;</span>
            <span class="pill-label">Antworten</span>
          </button>
          <button type="button" class="action-btn-pill delete" onclick={() => handleDeleteMessage(selectedMessage.uid)} title="Löschen (⌫)">
            <span class="pill-icon" aria-hidden="true">&#x1F5D1;</span>
            <span class="pill-label">Löschen</span>
          </button>
        </div>
      </div>
      
      <div class="preview-scroll-wrapper">
        <div class="preview-content-area">
          <h1 class="preview-subject-large">{selectedMessage.subject || "(Kein Betreff)"}</h1>
          <div class="preview-date-line">{formatDate(selectedMessage.date)}</div>
          
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
                  title="E-Mail Inhalt"
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
              <div class="mail-body-empty">(Kein Inhalt)</div>
            {/if}
          </div>

          {#if attachments.length > 0}
            <div class="attachments">
              <div class="attachments-title">{attachments.length} Anhang{attachments.length === 1 ? "" : "e"}</div>
              <div class="attachments-list">
                {#each attachments as att, i (i)}
                  <button
                    type="button"
                    class="attachment-chip"
                    onclick={() => handleOpenAttachment(att)}
                    oncontextmenu={(e) => handleAttachmentContextMenu(e, att)}
                    title="Öffnen (Rechtsklick: Optionen)"
                  >
                    <span class="attachment-icon" aria-hidden="true">&#x1F4CE;</span>
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
            <div class="ctx-menu" class:sheet={isTouchDevice} style={isTouchDevice ? "" : `left: ${attCtxMenu.x}px; top: ${attCtxMenu.y}px;`} role="menu">
              <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => handleOpenAttachment(attCtxMenu.att)}>Öffnen</button>
              <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => handleSaveAsAttachment(attCtxMenu.att)}>Speichern als…</button>
            </div>
          {/if}

          {#if replySuggestions.length > 0}
            <ReplySuggestions suggestions={replySuggestions} onselect={(s) => {
              composeMode = "reply";
              sendError = null;
              replySubject = selectedMessage.subject ?? "";
              replyTo = extractEmail(selectedMessage.from ?? "");
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
      icon="&#x26A0;"
      title="Keine Verbindung"
      subtitle={initError}
      actionLabel="Erneut versuchen"
      onaction={retryInit}
    />
  {:else}
    <EmptyState icon="&#x1F4ED;" title="Wähle eine Nachricht" subtitle="Wähle links eine E-Mail aus, um sie hier zu lesen." offsetHeader={true} />
  {/if}
{/snippet}

{#if showSplash}
  <SplashScreen oncomplete={handleSplashComplete} />
{:else}
  <div class="app-container" class:compact={isCompact} class:narrow={isNarrow} class:preview-open={previewOpen} class:sidebar-open={sidebarOpen}>
    {#if isNarrow && sidebarOpen}
      <div class="sidebar-scrim" role="presentation" onclick={() => sidebarOpen = false}></div>
    {/if}
    <aside class="sidebar-pane" style={isNarrow ? "" : `width: ${sidebarWidth}px; min-width: ${sidebarWidth}px;`}>
      <div class="sidebar">
        <div class="sidebar-header">
          {#if isNarrow}
            <button type="button" class="icon-btn sidebar-close" onclick={() => sidebarOpen = false} title="Schließen" aria-label="Ordner schließen">
              &#8592;
            </button>
          {/if}
          <div class="account-header-btn" role="button" tabindex="0" onclick={() => goto('/settings')} onkeydown={(e) => e.key === 'Enter' && goto('/settings')} title="Konto-Einstellungen">
            <div class="account-header-avatar">
              {#if ownPhoto}
                <img src="data:{ownPhoto.type};base64,{ownPhoto.data}" alt="Profilbild" />
              {:else}
                {getInitials(selectedAccount?.name || "Konto")}
              {/if}
            </div>
            <div class="account-header-meta">
              <span class="account-header-name">{selectedAccount?.name || "Konto wählen"}</span>
              <span class="account-header-sub">
                <span class="account-header-dot" class:connected={selectedAccount?.connected}></span>
                {selectedAccount?.username || "Nicht konfiguriert"}
              </span>
            </div>
          </div>
        </div>
        <nav class="sidebar-nav" id="sidebar-nav">
          {#each $accounts.groups as group}
            <AccountGroup
              account={group.account}
              folderTree={folderTreesByAccount[group.account.id] ?? { name: "INBOX", label: "", children: [] }}
              selectedFolder={group.account.id === selectedAccountId ? selectedFolder : null}
              collapsedFolders={getCollapsedForAccount(group.account.id)}
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
        </nav>
        <div class="sidebar-footer">
          <div class="footer-row">
            <div class="search-bar-inner">
              <input
                type="text"
                class="search-input"
                aria-label="E-Mails suchen" placeholder={searchFocused ? "" : "E-Mails suchen..."}
                bind:value={searchQuery}
                oninput={onSearchInput}
                onfocus={() => { searchFocused = true; }}
                onblur={() => { searchFocused = false; }}
                onkeydown={(e) => { if (e.key === 'Escape') clearSearch(); }}
              />
              {#if searchQuery}
                <button type="button" class="search-clear" onclick={clearSearch} title="Suche löschen" aria-label="Suche löschen">&#x2715;</button>
              {/if}
            </div>
          </div>
          
          <span class="version">Relay 2.1</span>
        </div>
      </div>
    </aside>
    {#if !isNarrow}
      <div class="resize-handle" onmousedown={(e) => startResize(e, 'sidebar')}></div>
    {/if}
    <main class="list-pane" style={isCompact ? "" : `width: ${listWidth}px; min-width: ${listWidth}px;`}>
      <div class="list-header-container">
        <div class="list-header">
          <div class="list-title-area">
            {#if isNarrow}
              <button type="button" class="icon-btn menu-toggle" onclick={() => sidebarOpen = !sidebarOpen} title="Ordner" aria-label="Ordner ein-/ausblenden">
                &#9776;
              </button>
            {/if}
            <h1>{searchActive ? "Suche" : translateFolder(selectedFolder)}</h1>
            {#if $mailbox.messages.length > 0}<span class="count-badge">{$mailbox.messages.length}</span>{/if}
          </div>
          <div class="list-header-pill">
            <button type="button" class="pill-icon-btn" onclick={handleNewMail} title="Neue E-Mail (Strg+N / Cmd+N)">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5L20.5 7.5 8 20H4v-4L16.5 3.5z"/></svg>
            </button>
            <button type="button" class="pill-icon-btn" onclick={loadFolder} title="Aktualisieren (Strg+R / Cmd+R)">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M21 2v6h-6"/><path d="M3 12a9 9 0 0 1 14.9-6.5L21 8"/><path d="M3 22v-6h6"/><path d="M21 12a9 9 0 0 1-14.9 6.5L3 16"/></svg>
            </button>
          </div>
        </div>
      </div>
      {#if $mailbox.error}
        <ErrorBanner message={$mailbox.error} onretry={loadFolder} />
      {/if}
      {#if $mailbox.selectedUids.length > 1}
        <div class="selection-toolbar">
          <span class="selection-count">{$mailbox.selectedUids.length} ausgewählt</span>
          <div class="selection-actions">
            <button type="button" class="selection-btn" onclick={markSelectedRead} title="Als gelesen markieren">
              Gelesen
            </button>
            <button type="button" class="selection-btn" onclick={moveSelectedToFolder} disabled={movingSelection} title="In Ordner verschieben">
              Verschieben
            </button>
            <button type="button" class="selection-btn danger" onclick={handleDeleteSelected} title="Löschen (⌫)">
              Löschen
            </button>
            <button type="button" class="selection-btn ghost" onclick={() => mailbox.clearSelection()} title="Auswahl aufheben (Esc)">
              &#x2715;
            </button>
          </div>
        </div>
      {/if}
      <div class="list-scroll-wrapper">
        {@render list()}
      </div>
    </main>
    {#if !isCompact}
      <div class="resize-handle" onmousedown={(e) => startResize(e, 'list')}></div>
    {/if}
    <section class="preview-pane">
      {#if isCompact && previewOpen && !showCompose}
        <div class="preview-back-bar">
          <button type="button" class="icon-btn" onclick={backToList} title="Zurück zur Liste" aria-label="Zurück">
            &#8592; Zurück
          </button>
        </div>
      {/if}
      {@render preview()}
    </section>
  </div>
{/if}

  {#if showDeleteConfirm}
    {#if moveToTrash}
      <ConfirmationDialog
        open={showDeleteConfirm}
        title={pendingDeleteUids.length === 1 ? "Nachricht in Papierkorb verschieben" : "Nachrichten in Papierkorb verschieben"}
        message={pendingDeleteUids.length === 1
          ? "1 Nachricht in den Papierkorb verschieben?"
          : `${pendingDeleteUids.length} Nachrichten in den Papierkorb verschieben?`}
        confirmLabel="In Papierkorb"
        cancelLabel="Abbrechen"
        danger={true}
        onconfirm={confirmDelete}
        oncancel={cancelDelete}
      />
    {:else}
      <ConfirmationDialog
        open={showDeleteConfirm}
        title={pendingDeleteUids.length === 1 ? "Nachricht löschen" : "Nachrichten löschen"}
        message={pendingDeleteUids.length === 1
          ? "1 Nachricht wirklich löschen?"
          : `${pendingDeleteUids.length} Nachrichten wirklich löschen?`}
        confirmLabel="Löschen"
        cancelLabel="Abbrechen"
        danger={true}
        onconfirm={confirmDelete}
        oncancel={cancelDelete}
      />
    {/if}
  {/if}

  {#if showDeleteFolderConfirm}
    <ConfirmationDialog
      open={showDeleteFolderConfirm}
      title="Ordner löschen"
      message={`Ordner "${pendingDeleteFolder ?? ''}" wirklich löschen? Alle enthaltenen Mails werden entfernt.`}
      confirmLabel="Löschen"
      cancelLabel="Abbrechen"
      danger={true}
      onconfirm={confirmDeleteFolder}
      oncancel={() => { showDeleteFolderConfirm = false; pendingDeleteFolder = null; }}
    />
  {/if}

  <PromptDialog
    open={showRenameDialog}
    title="Ordner umbenennen"
    message="Neuen Namen für den Ordner auf dem Server eingeben:"
    value={renameLeafValue}
    confirmLabel="Umbenennen"
    cancelLabel="Abbrechen"
    onconfirm={confirmRename}
    oncancel={cancelRename}
  />

  <PromptDialog
    open={showNewFolderDialog}
    title="Lokalen Ordner anlegen"
    message="Dieser Ordner existiert nur lokal auf deinem Relay-Server (kein IMAP-Ordner). Mails, die du hierher verschiebst, werden lokal archiviert und vom Provider entfernt."
    placeholder="z. B. Wichtige Rechnungen"
    confirmLabel="Anlegen"
    cancelLabel="Abbrechen"
    onconfirm={confirmNewFolder}
    oncancel={cancelNewFolder}
  />

  {#if folderCtxMenu}
    <div class="ctx-menu-scrim" class:sheet-scrim={isTouchDevice} role="presentation" onclick={closeMenus} oncontextmenu={(e) => e.preventDefault()}></div>
    <div class="ctx-menu" class:sheet={isTouchDevice} style={isTouchDevice ? "" : `left: ${folderCtxMenu.x}px; top: ${folderCtxMenu.y}px;`} role="menu">
      <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => { folderCtxNewSubFolder(folderCtxMenu.folderName); }}>Neuer Ordner...</button>
      {#if folderCtxMenu.folderName !== "INBOX"}
        <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => { openRenameDialog(folderCtxMenu.folderName); closeMenus(); }}>Umbenennen...</button>
      {/if}
      {#if customFolderNames[folderCtxMenu.folderName]}
        <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => folderCtxResetName(folderCtxMenu.folderName)}>Name zurücksetzen</button>
      {/if}
      {#if folderCtxMenu.folderName !== "INBOX"}
        <div class="ctx-menu-separator" role="separator"></div>
        <button type="button" class="ctx-menu-item danger" role="menuitem" onclick={() => folderCtxDeleteFolder(folderCtxMenu.folderName)}>Löschen</button>
      {/if}
      {#if folderCtxMenu.folderName !== "INBOX"}
        <button type="button" class="ctx-menu-item" role="menuitem" onclick={() => folderCtxHideFolder(folderCtxMenu.folderName)}>Ausblenden</button>
      {/if}
      {#if hiddenFolderNames.length > 0}
        <div class="ctx-menu-separator" role="separator"></div>
        <button type="button" class="ctx-menu-item" role="menuitem" onclick={folderCtxUnhideAll}>Alle ausgeblendeten Ordner einblenden</button>
      {/if}
    </div>
  {/if}

  {#if moveMenu}
    <div class="ctx-menu-scrim" class:sheet-scrim={isTouchDevice} role="presentation" onclick={closeMenus} oncontextmenu={(e) => e.preventDefault()}></div>
    <div class="ctx-menu" class:sheet={isTouchDevice} style={isTouchDevice ? "" : `left: ${moveMenu.x}px; top: ${moveMenu.y}px;`} role="menu">
      {#each moveMenu.targets as target (target.name)}
        <button
          type="button"
          class="ctx-menu-item"
          role="menuitem"
          onclick={() => {
            const uids = [...$mailbox.selectedUids];
            const name = target.name;
            closeMenus();
            void performMoveSelected(uids, name);
          }}
        >{target.label ?? target.name}</button>
      {/each}
    </div>
  {/if}

<style>
  .app-container {
    display: flex;
    height: 100vh;
    width: 100vw;
    contain: strict;
  }
  .sidebar-pane {
    flex-shrink: 0;
    background: var(--color-sidebar);
    /* Trennlinie unsichtbar: gleiche Farbe wie der Sidebar-Hintergrund */
    border-right: 1px solid var(--color-sidebar);
    contain: layout style paint;
  }
  .resize-handle {
    width: 5px;
    cursor: col-resize;
    background: transparent;
    flex-shrink: 0;
    z-index: 10;
  }
  .resize-handle:hover {
    background: var(--color-accent);
    opacity: 0.3;
  }
  .list-pane {
    flex-shrink: 0;
    background: var(--color-list);
    border-right: 1px solid var(--color-border);
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
    background: var(--color-preview);
    /* contain: layout would create a containing block for position:fixed
       descendants — the attachment context menu would be misplaced. */
    contain: style;
    display: flex;
    flex-direction: column;
  }
  .sidebar {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 0;
  }
  .sidebar-header {
    height: 72px;
    padding: 0 15px;
    display: flex;
    align-items: center;
    gap: 8px;
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
    margin-bottom: 16px;
  }
  .sidebar-close {
    flex-shrink: 0;
    font-size: 1.25rem;
  }
  .account-header-btn {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px;
    background: var(--color-list);
    border: 1px solid var(--color-border);
    border-radius: 12px;
    cursor: pointer;
    width: 100%;
    transition: all 0.15s ease-in-out;
  }
  .account-header-btn:hover {
    background: var(--color-active-wash);
    border-color: var(--color-accent);
  }
  .account-header-avatar {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: var(--color-accent);
    color: #ffffff;
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 700;
    font-size: 0.8125rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.08);
    flex-shrink: 0;
    overflow: hidden;
  }
  .account-header-avatar img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    border-radius: 50%;
  }
  .account-header-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
    text-align: left;
  }
  .account-header-name {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--color-text);
    line-height: 1.2;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .account-header-sub {
    font-size: 0.75rem;
    color: var(--color-text-secondary);
    line-height: 1.2;
    display: flex;
    align-items: center;
    gap: 4px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .account-header-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--color-border);
    flex-shrink: 0;
    display: inline-block;
  }
  .account-header-dot.connected {
    background: var(--color-success);
    box-shadow: 0 0 6px var(--color-success);
  }

  .sidebar-nav {
    flex: 1;
    padding: 0;
    overflow-y: auto;
  }
 
  .footer-row {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    margin-bottom: 8px;
    padding: 0 15px;  }
  :global(.folder-item) {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 10px 14px;
    margin-bottom: 4px;
    border: none;
    background: none;
    font-size: 0.875rem;
    font-weight: 500;
    color: var(--color-text-secondary);
    cursor: pointer;
    border-radius: 8px;
    font-family: inherit;
    transition: all 0.15s ease-in-out;
  }
  :global(.folder-item:hover) {
    background: var(--color-active-wash);
    color: var(--color-text);
  }
  :global(.folder-item.active) {
    background: var(--color-active-wash);
    color: var(--color-accent);
    font-weight: 600;
  }
  :global(.folder-item.indent) {
    padding-left: 34px;
  }
  :global(.folder-item.drag-over) {
    background: var(--color-active-wash);
    color: var(--color-accent);
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
  :global(.folder-svg-icon) {
    width: 18px;
    height: 18px;
    pointer-events: none;
  }
  :global(.folder-name-label) {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    pointer-events: none;
  }

  .sidebar-footer {
    margin-top: auto;
    padding-top: 12px;
    border-top: 1px solid var(--color-border);
  }
  .sidebar-footer .search-bar-inner {
    margin: 0;
    width: 100%;
  }
  .version {
    font-size: 0.75rem;
    color: var(--color-text-secondary);
    opacity: 0.6;
    text-align: center;
    display: block;
    margin-top: 8px;
    margin-bottom: 20px;
    letter-spacing: 0.01em;
  }
  .list-header-container {
    display: flex;
    flex-direction: column;
    background: var(--color-list);
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
  }
  .list-header {
    height: 72px;
    padding: 0 20px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    background: transparent;
    flex-shrink: 0;
  }
  .list-title-area {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .list-header-pill {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px;
    border: 1px solid var(--color-border);
    border-radius: 12px;
    background: var(--color-list);
  }
  .list-header h1 {
    font-size: 1.125rem;
    font-weight: 700;
    color: var(--color-text);
  }
  .count-badge {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--color-text-secondary);
    background: var(--color-sidebar);
    padding: 2px 8px;
    border-radius: 100px;
  }
  .pill-icon-btn {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--color-text-secondary);
    transition: all 0.15s ease;
    padding: 6px;
    border-radius: 100px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .pill-icon-btn:hover {
    color: var(--color-accent);
    background: var(--color-active-wash);
  }
  .search-bar {
    display: flex;
    align-items: center;
    padding: 0 16px 12px 16px;
    background: transparent;
    flex-shrink: 0;
  }
  .search-bar-inner {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    height: 34px;
    padding: 0 12px;
    border-radius: 8px;
    border: 1px solid var(--color-border);
    background: var(--color-list);
    transition: all 0.15s ease-in-out;
  }
  .search-bar-inner:focus-within {
    border-color: var(--color-accent);
    background: var(--color-list);
  }
  .search-icon {
    font-size: 0.8125rem;
    opacity: 0.55;
    flex-shrink: 0;
  }
  .search-input {
    flex: 1;
    border: none;
    background: transparent;
    color: var(--color-text);
    font-size: 0.875rem;
    font-family: inherit;
    outline: none;
    min-width: 0;
    text-align: center;
  }
  .search-input::placeholder {
    color: var(--color-text-secondary);
  }
  .search-clear {
    border: none;
    background: none;
    color: var(--color-text-secondary);
    cursor: pointer;
    font-size: 0.875rem;
    padding: 2px 6px;    border-radius: 4px;
    flex-shrink: 0;
  }
  .search-clear:hover {
    background: var(--color-active-wash);
    color: var(--color-text);
  }
  .selection-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 16px;
    background: var(--color-active-wash);
    border-bottom: 1px solid var(--color-border);
    flex-shrink: 0;
  }
  .selection-count {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--color-accent);
  }
  .selection-actions {
    display: flex;
    gap: 6px;
  }
  .selection-btn {
    padding: 5px 12px;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    background: var(--color-list);
    color: var(--color-text);
    font-size: 0.75rem;
    font-weight: 600;
    font-family: inherit;
    cursor: pointer;
    transition: all 0.15s ease-in-out;
  }
  .selection-btn:hover:not(:disabled) {
    border-color: var(--color-accent);
    color: var(--color-accent);
  }
  .selection-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .selection-btn.danger:hover {
    border-color: var(--color-danger);
    color: var(--color-danger);
  }
  .selection-btn.ghost {
    border-color: transparent;
    background: transparent;
    padding: 5px 8px;
  }
  .list-scroll-wrapper {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .preview-layout {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .preview-pane-header {
    height: 72px;
    padding: 0 24px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    /* Linie unter dem Vorschau-Header unsichtbar (gleiche Farbe wie Hintergrund) */
    border-bottom: 1px solid var(--color-preview);
    background: var(--color-preview);
    flex-shrink: 0;
  }
  .preview-header-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .preview-from-name {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--color-text);
  }
  .preview-from-email {
    font-size: 0.75rem;
    color: var(--color-text-secondary);
  }
  .preview-header-actions {
    display: flex;
    gap: 8px;
  }
  .action-btn-pill {
    padding: 6px 14px;
    border: 1px solid var(--color-border);
    border-radius: 100px;
    background: var(--color-list);
    color: var(--color-text);
    cursor: pointer;
    font-size: 0.75rem;
    font-weight: 500;
    transition: all 0.15s ease-in-out;
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .pill-icon {
    line-height: 1;
  }
  .action-btn-pill:hover {
    border-color: var(--color-accent);
    color: var(--color-accent);
    background: var(--color-active-wash);
  }
  .action-btn-pill.delete:hover {
    border-color: var(--color-danger);
    color: var(--color-danger);
    background: color-mix(in srgb, var(--color-danger) 8%, transparent);
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
    background: var(--color-border);
    border-radius: 3px;
  }
  .preview-scroll-wrapper::-webkit-scrollbar-thumb:hover {
    background: var(--color-text-secondary);
  }
  .preview-content-area {
    padding: 32px 24px 24px 24px;
    max-width: 800px;
  }
  .preview-subject-large {
    font-size: 1.5rem;
    font-weight: 700;
    margin-bottom: 8px;
    color: var(--color-text);
    line-height: 1.3;
  }
.preview-date-line {
    font-size: 0.75rem;
    color: var(--color-text-secondary);
  }
  .mail-iframe-container {
    background: var(--color-list);
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
    font-size: 0.9375rem;
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
    margin: 0;
    color: var(--color-text);
  }
  .mail-body-empty {
    font-size: 0.875rem;
    color: var(--color-text-secondary);
    font-style: italic;
  }
  .attachments {
    margin-top: 20px;
    padding-top: 16px;
    border-top: 1px solid var(--color-border);
  }
  .attachments-title {
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--color-text-secondary);
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
    border: 1px solid var(--color-border);
    border-radius: 8px;
    background: var(--color-list);
    cursor: pointer;
    font-family: inherit;
    text-align: left;
    max-width: 260px;
    transition: all 0.15s ease-in-out;
  }
  .attachment-chip:hover:not(:disabled) {
    border-color: var(--color-accent);
    background: var(--color-active-wash);
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
    color: var(--color-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .attachment-size {
    font-size: 0.6875rem;
    color: var(--color-text-secondary);
  }
.preview-body {
    font-size: 0.875rem;
    line-height: 1.7;
    color: var(--color-text);
  }
  .mail-body {
    font-family: inherit;
    font-size: 0.875rem;
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
    margin: 0;
  }
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--color-text-secondary);
  }
  .empty-icon {
    font-size: 3rem;
    margin-bottom: 12px;
    opacity: 0.3;
  }
  /* ─── Splash Screen ─── */
  .splash-screen {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    background: var(--color-sidebar);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    overflow-y: auto;
    padding: 24px;
  }
  .splash-card {
    background: var(--color-list);
    border: 1px solid var(--color-border);
    border-radius: 12px;
    padding: 48px;
    width: 100%;
    max-width: 720px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.03);
    display: flex;
    flex-direction: column;
    animation: fadeIn 0.25s ease-out;
  }
  @keyframes fadeIn {
    from { opacity: 0; transform: scale(0.98); }
    to { opacity: 1; transform: scale(1); }
  }
  .splash-intro {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
  }
  .splash-intro h1 {
    font-size: 1.875rem;
    font-weight: 700;
    margin-bottom: 8px;
    color: var(--color-text);
  }
  .splash-subtitle {
    font-size: 0.9375rem;
    color: var(--color-text-secondary);
    margin-bottom: 48px;
    max-width: 500px;
  }
  .feature-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 24px;
    width: 100%;
    margin-bottom: 48px;
  }
  .feature-card {
    border-top: 1px solid var(--color-border);
    padding-top: 16px;
    text-align: left;
    display: flex;
    flex-direction: column;
  }
  .feature-card h3 {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--color-text);
    margin-bottom: 8px;
  }
  .feature-card p {
    font-size: 0.8125rem;
    color: var(--color-text-secondary);
    line-height: 1.5;
  }
  .btn-splash-primary {
    background: var(--color-accent);
    color: #ffffff;
    font-size: 0.875rem;
    font-weight: 600;
    padding: 10px 24px;
    border: none;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s ease-in-out;
  }
  .btn-splash-primary:hover:not(:disabled) {
    background: var(--color-accent-hover);
  }
  .btn-splash-primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn-splash-secondary {
    background: transparent;
    border: 1px solid var(--color-border);
    color: var(--color-text);
    font-size: 0.875rem;
    font-weight: 600;
    padding: 10px 20px;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s ease-in-out;
  }
  .btn-splash-secondary:hover {
    background: var(--color-sidebar);
  }
  .splash-form-view {
    display: flex;
    flex-direction: column;
  }
  .splash-form-view h2 {
    font-size: 1.375rem;
    font-weight: 700;
    margin-bottom: 8px;
    color: var(--color-text);
  }
  .splash-form {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 20px 24px;
    text-align: left;
    margin-top: 20px;
  }
  .form-group.span-2 {
    grid-column: span 2;
  }
  .port-ssl-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 41px;
    width: 100%;
  }
  .splash-form .form-group .port-ssl-row input[type="number"] {
    width: 75px;
    flex-shrink: 0;
  }
  .check-label {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--color-text);
    user-select: none;
    height: 100%;
  }
  .check-label input[type="checkbox"] {
    appearance: none;
    -webkit-appearance: none;
    width: 16px;
    height: 16px;
    border: 1px solid var(--color-border);
    border-radius: 4px;
    background: var(--color-list);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: all 0.1s ease-in-out;
    position: relative;
    outline: none;
    margin: 0;
  }
  .check-label input[type="checkbox"]:checked {
    background: var(--color-accent);
    border-color: var(--color-accent);
  }
  .check-label input[type="checkbox"]:checked::after {
    content: "";
    position: absolute;
    width: 4px;
    height: 8px;
    border: solid white;
    border-width: 0 2px 2px 0;
    transform: rotate(45deg);
    top: 2px;
    left: 5px;
  }
  .check-label input[type="checkbox"]:focus {
    border-color: var(--color-accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--color-accent) 12%, transparent);
  }
  .splash-form .form-group label:not(.check-label) {
    display: block;
    font-size: 0.6875rem;
    font-weight: 600;
    color: var(--color-text-secondary);
    margin-bottom: 6px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }
  .splash-form .form-group input {
    width: 100%;
    padding: 10px 14px;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    font-size: 0.875rem;
    color: var(--color-text);
    background: var(--color-list);
    box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.01), 0 1px 2px rgba(0, 0, 0, 0.01);
    transition: all 0.15s ease-in-out;
  }
  .splash-form .form-group input:focus {
    border-color: var(--color-accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--color-accent) 12%, transparent);
    background: var(--color-list);
  }
  .splash-actions {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: 12px;
    border-top: 1px solid var(--color-border);
    padding-top: 24px;
  }
  .splash-actions.span-2 {
    grid-column: span 2;
  }
  .error-message.span-2 {
    grid-column: span 2;
  }

  /* ─── Preview Skeleton ─── */
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
      var(--color-border) 0%,
      var(--color-active-wash) 40%,
      var(--color-active-wash) 60%,
      var(--color-border) 100%
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

  /* ─── Responsive layout (additive) ───────────────────────── */
  .icon-btn {
    border: none;
    background: none;
    cursor: pointer;
    color: var(--color-text-secondary);
    font-size: 1rem;
    padding: 4px 8px;
    border-radius: 6px;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-family: inherit;
    transition: all 0.15s ease;
  }
  .icon-btn:hover {
    background: var(--color-active-wash);
    color: var(--color-text);
  }
  .menu-toggle {
    font-size: 1.125rem;
    margin-right: 4px;
  }
  .preview-back-bar {
    flex-shrink: 0;
    padding: 8px 12px;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-preview);
  }
  .sidebar-scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    z-index: 40;
  }

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

  /* NARROW (≤600px): sidebar collapses to a full-width overlay (iOS Mail style). */
  .app-container.narrow .sidebar-pane {
    position: fixed;
    top: 0;
    left: 0;
    bottom: 0;
    width: 100%;
    max-width: none;
    z-index: 50;
    transform: translateX(-100%);
    transition: transform 0.25s cubic-bezier(0.32, 0.72, 0, 1);
  }
  .app-container.narrow.sidebar-open .sidebar-pane {
    transform: translateX(0);
  }
  /* In narrow mode the sidebar covers the whole width, so the dark scrim would
     only flash at the edge while the sidebar slides in — hide its shadow. */
  .app-container.narrow .sidebar-scrim {
    background: transparent;
  }

  /* ─── iPhone 15 Pro mobile optimizations ─────────────────── */
  /* Safe-area insets (Dynamic Island + home indicator). */
  .app-container.narrow .sidebar-pane {
    padding-top: env(safe-area-inset-top, 0px);
    padding-bottom: env(safe-area-inset-bottom, 0px);
  }
  .app-container.narrow .list-header-container {
    padding-top: env(safe-area-inset-top, 0px);
    background: var(--color-list);
  }
  .app-container.narrow .preview-back-bar {
    padding-top: max(8px, env(safe-area-inset-top, 0px));
    min-height: 44px;
  }
  .app-container.narrow .sidebar-footer {
    padding-bottom: max(8px, env(safe-area-inset-bottom, 0px));
  }
  .app-container.narrow .menu-toggle,
  .app-container.narrow .icon-btn {
    min-width: 44px;
    min-height: 44px;
    font-size: 1.25rem;
  }
  /* Burger menu ~33% bigger on phones (1.25rem -> 1.67rem). */
  .app-container.narrow .menu-toggle {
    font-size: 1.67rem;
    min-width: 48px;
    min-height: 48px;
    line-height: 1;
  }
  /* Preview action pills: text only (no icons) on phones. */
  .app-container.narrow .action-btn-pill .pill-icon {
    display: none;
  }
  /* Compact preview header on phones. */
  .app-container.narrow .preview-pane-header {
    height: auto;
    min-height: 64px;
    padding: 8px 14px;
    padding-top: max(8px, env(safe-area-inset-top, 0px));
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
  .app-container.narrow .action-btn-pill {
    padding: 9px 12px;
    min-height: 40px;
    font-size: 0.85rem;
  }
  /* Compose window fills the phone screen. */
  .app-container.narrow .compose-window {
    margin: 0;
    border-radius: 0;
    height: 100%;
  }

  /* ─── Plain HTML context menus (replaces Tauri native menus) ─── */
  .ctx-menu-scrim {
    position: fixed;
    inset: 0;
    z-index: 1000;
  }
  .ctx-menu-scrim.sheet-scrim {
    background: rgba(0, 0, 0, 0.35);
  }
  .ctx-menu {
    position: fixed;
    z-index: 1001;
    min-width: 200px;
    max-width: 260px;
    max-height: 320px;
    overflow-y: auto;
    background: var(--color-list);
    border: 1px solid var(--color-border);
    border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.15);
    padding: 6px;
    display: flex;
    flex-direction: column;
  }
  .ctx-menu-item {
    display: block;
    width: 100%;
    text-align: left;
    padding: 8px 12px;
    border: none;
    background: none;
    border-radius: 6px;
    font-size: 0.8125rem;
    color: var(--color-text);
    cursor: pointer;
    font-family: inherit;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ctx-menu-item:hover {
    background: var(--color-active-wash);
    color: var(--color-accent);
  }
  .ctx-menu-item.danger {
    color: var(--color-danger);
  }
  .ctx-menu-item.danger:hover {
    background: rgba(220, 38, 38, 0.10);
    color: var(--color-danger);
  }

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
    box-shadow: 0 -8px 32px rgba(0, 0, 0, 0.2);
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
    background: var(--color-active-wash);
    color: var(--color-text);
  }
  .ctx-menu.sheet .ctx-menu-item.danger:hover {
    color: var(--color-danger);
  }
  .ctx-menu.sheet .ctx-menu-separator {
    margin: 4px 16px;
  }
  .ctx-menu-separator {
    height: 1px;
    margin: 4px 8px;
    background: var(--color-border);
  }
</style>
