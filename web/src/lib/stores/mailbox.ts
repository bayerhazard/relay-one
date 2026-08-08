import { writable } from "svelte/store";

export interface Message {
  uid: number;
  message_id?: string;
  subject?: string;
  from?: string;
  to?: string;
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
}

interface MailboxState {
  accountId: number | null;
  messages: Message[];
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
        const oldBodies = new Map(
          s.messages.map((m) => [
            `${aId}:${fId}:${m.uid}`,
            { body_text: m.body_text, body_html: m.body_html },
          ])
        );
        const merged = messages.map((m) => {
          const key = `${aId}:${fId}:${m.uid}`;
          return {
            ...m,
            body_text: oldBodies.get(key)?.body_text ?? m.body_text,
            body_html: oldBodies.get(key)?.body_html ?? m.body_html,
          };
        });
        return {
          ...s,
          accountId: aId,
          messages: merged,
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
    updateMessage: (uid: number, changes: Partial<Message>) =>
      update((s) => {
        const idx = s.messages.findIndex((m) => m.uid === uid);
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
        selectedUids: [],
        lastClickedUid: null,
        folderId: '',
        loading: false,
        error: null,
      }),
  };
}

export const mailbox = createMailboxStore();
