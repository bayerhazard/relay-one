import { writable } from "svelte/store";

export interface Message {
  uid: number;
  message_id?: string;
  subject?: string;
  from?: string;
  to?: string;
  cc?: string;
  date?: string;
  body_preview?: string;
  body_text?: string;
  body_html?: string;
  flags?: string;
  ai_summary?: string;
  ai_priority?: number;
  ai_fraud_score?: number;
  is_read: boolean;
  is_flagged: boolean;
  has_attachments?: boolean;
  /** Attachments metadata, present on draft/body responses. */
  attachments?: { id: number; part_index: number; filename: string; content_type: string; size: number; content?: string }[];
}

// ─── Persistent folder cache ─────────────────────────────────
// Meta-only message lists per (account, folder), persisted in localStorage.
// Purpose: switching back to an already-seen large folder is instant (no
// server round-trip). Bodies are NOT cached here — they load on demand via
// GET /messages/{uid}/body when a message is opened.

const FOLDER_CACHE_KEY = "relay:folderCache:v1";
const FOLDER_CACHE_MAX = 10000;

type FolderCacheMap = Record<string, Message[]>;

function loadFolderCache(): FolderCacheMap {
  try {
    const raw = localStorage.getItem(FOLDER_CACHE_KEY);
    return raw ? JSON.parse(raw) : {};
  } catch {
    return {};
  }
}

function persistFolderCache(cache: FolderCacheMap) {
  try {
    localStorage.setItem(FOLDER_CACHE_KEY, JSON.stringify(cache));
  } catch {
    // Quota exceeded or unavailable (e.g. private mode) — cache stays in
    // memory for the session and is simply not persisted.
  }
}

let folderCache: FolderCacheMap = loadFolderCache();

function cacheKey(accountId: number, folderId: string): string {
  return `${accountId}:${folderId}`;
}

/** Meta-only message list for (account, folder), or `null` when not cached. */
export function getFolderCache(accountId: number, folderId: string): Message[] | null {
  return folderCache[cacheKey(accountId, folderId)] ?? null;
}

/** Store a meta-only list for (account, folder) and persist it. */
export function setFolderCache(accountId: number, folderId: string, messages: Message[]) {
  const key = cacheKey(accountId, folderId);
  folderCache = {
    ...folderCache,
    [key]: messages.slice(0, FOLDER_CACHE_MAX),
  };
  persistFolderCache(folderCache);
}

/** Drop a cached (account, folder) list — used after delete/move/flag ops.
 *  Also drops the freshness timestamp so the next view re-fetches. */
export function invalidateFolderCache(accountId: number, folderId: string) {
  const key = cacheKey(accountId, folderId);
  delete lastFetchAt[key];
  if (!(key in folderCache)) return;
  const next = { ...folderCache };
  delete next[key];
  folderCache = next;
  if (Object.keys(folderCache).length === 0) {
    try {
      localStorage.removeItem(FOLDER_CACHE_KEY);
    } catch {
      // ignore
    }
  } else {
    persistFolderCache(folderCache);
  }
}

/** Clear the in-memory folder cache (test isolation). */
export function resetFolderCache() {
  folderCache = {};
  lastFetchAt = {};
  try {
    localStorage.removeItem(FOLDER_CACHE_KEY);
  } catch {
    // ignore
  }
}

// ─── Client-side freshness window ────────────────────────────
// A folder list fetched within FOLDER_FRESH_MS is considered fresh: switching
// back to it serves the cached list without hitting the network again. The
// window is in-memory only (per session). Mutations call
// invalidateFolderCache(), which also drops the timestamp so the very next
// view of the folder re-fetches. Event-driven reloads ("new-messages") pass
// force=true and bypass the window.

const FOLDER_FRESH_MS = 15_000;

let lastFetchAt: Record<string, number> = {};

/** True when (account, folder) was fetched within the freshness window. */
export function isFolderFresh(accountId: number | null, folderId: string): boolean {
  if (accountId == null) return false;
  const t = lastFetchAt[cacheKey(accountId, folderId)];
  return t !== undefined && Date.now() - t < FOLDER_FRESH_MS;
}

/** Record a successful fetch of (account, folder). */
export function markFolderFetched(accountId: number | null, folderId: string) {
  if (accountId == null) return;
  lastFetchAt[cacheKey(accountId, folderId)] = Date.now();
}

interface MailboxState {
  accountId: number | null;
  messages: Message[];
  /** Folder the current `messages` array belongs to (data state). Distinct
   *  from `folderId` (the UI-selected folder): the UI label is switched BEFORE
   *  the new list lands, so a body-merge keyed on `folderId` alone could paste
   *  the previous folder's mail body into the new folder's same-uid row. */
  messagesFolder: string | null;
  selectedUids: number[];
  lastClickedUid: number | null;
  folderId: string;
  loading: boolean;
  error: string | null;
}

function createMailboxStore() {
  const { subscribe, set, update } = writable<MailboxState>({
    accountId: null,
    messages: [],
    messagesFolder: null,
    selectedUids: [],
    lastClickedUid: null,
    folderId: '',
    loading: false,
    error: null,
  });

  return {
    subscribe,
   setMessages: (messages: Message[], folderLabel?: string, accountId?: number) =>
      update((s) => {
        const uidSet = new Set(messages.map((m) => m.uid));
        // Key by (accountId, folderId, uid) to prevent cross-account UID collisions
        const fId = folderLabel ?? s.folderId;
        const aId = accountId ?? s.accountId ?? 0;
        // Only carry over existing bodies when the incoming list belongs to the
        // SAME folder as the current store contents. Local folders assign uids
        // independently (uid=3 exists in both "Auto" and "Ecommerce"), so a
        // cross-folder uid-keyed merge would paste one folder's mail body into
        // another's row. A refresh of the same folder must keep loaded bodies.
        const sameFolder = folderLabel !== undefined && folderLabel === s.messagesFolder;
        const oldBodies = new Map(
          sameFolder
            ? s.messages.map((m) => [
                `${aId}:${fId}:${m.uid}`,
                { body_text: m.body_text, body_html: m.body_html },
              ])
            : []
        );
        const merged = messages.map((m) => {
          const key = `${aId}:${fId}:${m.uid}`;
          return {
            ...m,
            body_text: oldBodies.get(key)?.body_text ?? m.body_text,
            body_html: oldBodies.get(key)?.body_html ?? m.body_html,
          };
        });
        // Refresh the persistent folder cache (meta-only, no bodies) whenever
        // a full folder list lands in the store. Search results call
        // setMessages() without a folderLabel and must NOT overwrite it.
        if (folderLabel !== undefined && fId) {
          const metaOnly = merged.map(({ body_text: _bt, body_html: _bh, ...rest }) => rest);
          setFolderCache(aId, fId, metaOnly);
        }
        return {
          ...s,
          accountId: aId,
          messages: merged,
          messagesFolder: folderLabel ?? null,
          loading: false,
          selectedUids: s.selectedUids.filter((uid) => uidSet.has(uid)),
          lastClickedUid: uidSet.has(s.lastClickedUid ?? -1) ? s.lastClickedUid : null,
        };
      }),
    selectSingle: (uid: number) =>
      update((s) => ({ ...s, selectedUids: [uid], lastClickedUid: uid })),
    toggleSelect: (uid: number) =>
      update((s) => ({
        ...s,
        selectedUids: s.selectedUids.includes(uid)
          ? s.selectedUids.filter((u) => u !== uid)
          : [...s.selectedUids, uid],
        lastClickedUid: uid,
      })),
    selectRange: (fromIdx: number, toIdx: number, msgs: Message[]) =>
      update((s) => {
        // Clamp indices: the message list can shrink between click and
        // range-select (e.g. a refresh), so guard against out-of-bounds.
        const max = msgs.length - 1;
        if (max < 0) return s;
        const a = Math.max(0, Math.min(fromIdx, max));
        const b = Math.max(0, Math.min(toIdx, max));
        const start = Math.min(a, b);
        const end = Math.max(a, b);
        return {
          ...s,
          selectedUids: msgs.slice(start, end + 1).map((m) => m.uid),
          lastClickedUid: msgs[b]?.uid ?? null,
        };
      }),
    selectAll: (msgs: Message[]) =>
      update((s) => ({
        ...s,
        selectedUids: msgs.map((m) => m.uid),
      })),
    clearSelection: () =>
      update((s) => ({ ...s, selectedUids: [], lastClickedUid: null })),
    updateMessage: (uid: number, folderId: string, changes: Partial<Message>) =>
      update((s) => {
        // UIDs are only unique per (account, folder) — scope the match so an
        // event for another folder's message never overwrites this view.
        const idx = s.messages.findIndex(
          (m) => m.uid === uid && (folderId === "" || folderId === s.folderId)
        );
        if (idx === -1) return s;
        const updated = [...s.messages];
        updated[idx] = { ...updated[idx], ...changes };
        return { ...s, messages: updated };
      }),
    setFolderId: (folderId: string) => update((s) => ({ ...s, folderId })),
    setLoading: (loading: boolean) => update((s) => ({ ...s, loading })),
    setError: (error: string | null) => update((s) => ({ ...s, error })),
   reset: () =>
      set({
        accountId: null,
        messages: [],
        messagesFolder: null,
        selectedUids: [],
        lastClickedUid: null,
        folderId: '',
        loading: false,
        error: null,
      }),
  };
}

export const mailbox = createMailboxStore();
