import type { Message } from "$lib/stores/mailbox";
import type { AccountInfo } from "$lib/stores/accounts";
import type { AISettings } from "$lib/stores/settings";

// ─── API Base ──────────────────────────────────────────────
// Relative base so the app works behind the Olares entrance and in dev
// (vite proxy /api → localhost:3000). No Tauri dependency.
// We derive the base from the current location at runtime: in the Olares
// preview the app is served under /__preview/<port>/, so API calls must keep
// that prefix (the reverse proxy routes it to the dev server, which proxies
// /api to the backend). Fall back to import.meta.env.BASE_URL for the built
// app (Olares entrance), then to a plain /api/v1.

function resolveApiBase(): string {
  if (typeof window !== "undefined") {
    const m = window.location.pathname.match(/^(\/__preview\/\d+)/);
    if (m) return `${m[1]}/api/v1`;
  }
  return `${import.meta.env.BASE_URL ?? ""}api/v1`;
}

const API_BASE = resolveApiBase();

// ─── API Error Helper ──────────────────────────────────────

/**
 * Wraps every fetch() in try-catch so no unhandled promise rejection can
 * escape. Logs the technical error and throws a user-friendly German message.
 */
async function apiCall<T>(
  method: string,
  path: string,
  body: unknown,
  userMessage: string,
): Promise<T> {
  try {
    const res = await fetch(`${API_BASE}${path}`, {
      method,
      headers: body !== undefined ? { "Content-Type": "application/json" } : undefined,
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
    if (!res.ok) {
      let detail = `HTTP ${res.status}`;
      try {
        const j = await res.json();
        if (j?.error) detail = j.error;
      } catch { /* non-JSON error body */ }
      throw new Error(detail);
    }
    if (res.status === 204) return undefined as T;
    return await res.json() as T;
  } catch (e: unknown) {
    const detail = e instanceof Error ? e.message : String(e);
    console.error(`[API-Fehler] ${method} ${path}:`, detail);
    throw new Error(`${userMessage}\n\n(${detail})`);
  }
}

function get<T>(path: string, msg: string): Promise<T> {
  return apiCall<T>("GET", path, undefined, msg);
}
function post<T>(path: string, body: unknown, msg: string): Promise<T> {
  return apiCall<T>("POST", path, body, msg);
}

// ─── Settings ──────────────────────────────────────────────

export async function saveSettings(
  url: string,
  apiKey: string,
  model: string
): Promise<void> {
  return post("/settings", { url, api_key: apiKey, model },
    "Die KI-Einstellungen konnten nicht gespeichert werden.");
}

export async function getSettings(): Promise<AISettings | null> {
  return get("/settings", "Die KI-Einstellungen konnten nicht geladen werden.");
}

export async function getMoveToTrash(): Promise<boolean> {
  return get("/settings/move-to-trash", "Die Papierkorb-Einstellung konnte nicht geladen werden.");
}

export async function setMoveToTrash(enabled: boolean): Promise<void> {
  return post("/settings/move-to-trash", enabled,
    "Die Papierkorb-Einstellung konnte nicht gespeichert werden.");
}

// ─── Accounts ──────────────────────────────────────────────

export async function connectAccount(
  name: string,
  imapHost: string,
  imapPort: number,
  imapSsl: boolean,
  smtpHost: string,
  smtpPort: number,
  smtpTls: boolean,
  imapUsername: string,
  imapPassword: string,
  smtpUsername: string,
  smtpPassword: string,
  senderName: string,
  senderEmail: string,
  imapInsecure = false,
): Promise<AccountInfo> {
  return post("/accounts", {
    name,
    imap_host: imapHost, imap_port: imapPort, imap_ssl: imapSsl, imap_insecure: imapInsecure,
    smtp_host: smtpHost, smtp_port: smtpPort, smtp_tls: smtpTls,
    imap_username: imapUsername, imap_password: imapPassword,
    smtp_username: smtpUsername, smtp_password: smtpPassword,
    sender_name: senderName, sender_email: senderEmail,
  }, "Das Konto konnte nicht verbunden werden.");
}

export async function listAccounts(): Promise<AccountInfo[]> {
  return get("/accounts", "Die Kontenliste konnte nicht geladen werden.");
}

export async function deleteAccount(accountId: number): Promise<void> {
  return apiCall("POST", "/accounts/delete", { account_id: accountId },
    "Das Konto konnte nicht gelöscht werden.");
}

export async function updateAccountSettings(
  accountId: number,
  syncMode?: string,
  trashRetentionDays?: number,
  imapInsecure?: boolean,
): Promise<{ ok: boolean; sync_mode: string }> {
  const body: Record<string, unknown> = { account_id: accountId };
  if (syncMode !== undefined) body.sync_mode = syncMode;
  if (trashRetentionDays !== undefined) body.trash_retention_days = trashRetentionDays;
  if (imapInsecure !== undefined) body.imap_insecure = imapInsecure;
  return apiCall("POST", "/accounts/config", body,
    "Die Konto-Einstellungen konnten nicht gespeichert werden.");
}

// ─── Delete queue (verify pipeline review) ────────────────────

export interface DeleteQueueRow {
  id: number;
  account_id: number;
  uid: number;
  folder: string;
  action: string;
  state: string;
  attempts: number;
  last_error: string | null;
}

export async function getDeleteQueue(): Promise<DeleteQueueRow[]> {
  return get("/archive/delete-queue", "Die Lösch-Queue konnte nicht geladen werden.");
}

export async function retryDeleteQueueRow(id: number): Promise<{ ok: boolean }> {
  return post(`/archive/delete-queue/${id}/retry`, {}, "Der Eintrag konnte nicht erneut eingereiht werden.");
}

export async function removeDeleteQueueRow(id: number): Promise<{ ok: boolean }> {
  return post(`/archive/delete-queue/${id}/remove`, {}, "Der Eintrag konnte nicht entfernt werden.");
}

/** Download the EML/MBox export for an account (browser download). */
export function downloadExport(accountId: number, format: "mbox" | "zip"): void {
  const url = `${API_BASE}/export?account_id=${accountId}&format=${format}`;
  const a = document.createElement("a");
  a.href = url;
  a.download = format === "zip" ? `relay-account-${accountId}-eml.zip` : `relay-account-${accountId}.mbox`;
  document.body.appendChild(a);
  a.click();
  a.remove();
}

export interface BackupInfo {
  ok: boolean;
  path: string;
  size: number;
  created_at: string;
}

export async function createBackup(): Promise<BackupInfo> {
  return post("/archive/backup", {}, "Das Backup konnte nicht erstellt werden.");
}

export interface BackupListInfo {
  backups: Array<{ name: string; size: number }>;
}

export async function listBackups(): Promise<BackupListInfo> {
  return get("/archive/backups", "Die Backup-Liste konnte nicht geladen werden.");
}

export async function restoreBackupSnapshot(backupName: string): Promise<{
  ok: boolean;
  restored: string;
  bytes: number;
  note?: string;
}> {
  return post("/archive/restore", { backup_name: backupName },
    "Die Wiederherstellung konnte nicht durchgeführt werden.");
}

// ─── Badge ──────────────────────────────────────────────────
// Web version has no dock badge — unread count is returned directly.

export async function updateBadgeCount(accountId: number): Promise<number> {
  return 0;
}

// ─── IMAP ──────────────────────────────────────────────────

export async function fetchFromImap(accountId: number, folder?: string, limit?: number): Promise<Message[]> {
  const q = new URLSearchParams({ account_id: String(accountId) });
  if (folder) q.set("folder", folder);
  if (limit) q.set("limit", String(limit));
  return get(`/messages?${q}`, "Die E-Mails konnten nicht vom Server abgerufen werden.");
}

export async function listImapFolders(accountId: number): Promise<Array<{name: string; raw_name: string; delimiter: string; tag: string; attributes?: string[]; local_only?: boolean}>> {
  return get(`/folders?account_id=${accountId}`, "Die Ordnerliste konnte nicht geladen werden.");
}

export async function renameFolder(
  accountId: number,
  oldName: string,
  newName: string
): Promise<void> {
  return post("/folders/rename", { account_id: accountId, old_name: oldName, new_name: newName },
    "Der Ordner konnte nicht umbenannt werden.");
}

export async function createLocalFolder(accountId: number, name: string): Promise<{ ok: boolean; name: string; local_only: boolean }> {
  return post("/folders", { account_id: accountId, name },
    "Der lokale Ordner konnte nicht angelegt werden.");
}

export async function deleteFolder(accountId: number, name: string): Promise<{ ok: boolean }> {
  return post("/folders/delete", { account_id: accountId, name },
    "Der Ordner konnte nicht gelöscht werden.");
}

// ─── Messages ──────────────────────────────────────────────

export async function fetchMessages(
  accountId: number,
  limit?: number,
  offset?: number,
  folder?: string,
): Promise<Message[]> {
  const q = new URLSearchParams({ account_id: String(accountId) });
  if (limit) q.set("limit", String(limit));
  if (offset) q.set("offset", String(offset));
  if (folder) q.set("folder", folder);
  return get(`/messages?${q}`, "Die Nachrichten konnten nicht geladen werden.");
}

export async function searchMessages(
  accountId: number,
  query: string,
  limit?: number,
): Promise<Message[]> {
  const q = new URLSearchParams({ account_id: String(accountId), query });
  if (limit) q.set("limit", String(limit));
  return get(`/messages/search?${q}`, "Die Suche konnte nicht durchgeführt werden.");
}

export async function fetchRawMessage(
   accountId: number,
   uid: number,
 ): Promise<string> {
   return get(`/messages/raw?account_id=${accountId}&uid=${uid}`,
     "Die vollständige Nachricht konnte nicht geladen werden.");
 }

export interface AttachmentInfo {
   id: number;
   filename: string;
   content_type: string;
   size: number;
   content: string | null;
   content_cached: boolean;
}

export async function fetchAttachments(
   accountId: number,
   uid: number,
 ): Promise<AttachmentInfo[]> {
   return get(`/messages/attachments?account_id=${accountId}&uid=${uid}`,
     "Die Anhänge konnten nicht geladen werden.");
 }

 export async function loadAttachmentContent(
   accountId: number,
   uid: number,
   attachmentId: number,
 ): Promise<string> {
   return get(`/messages/attachment?account_id=${accountId}&uid=${uid}&att_id=${attachmentId}`,
     "Der Anhang konnte nicht geladen werden.");
 }

export async function getAttachmentCacheStats(): Promise<{
   total_attachments: number;
   cached_count: number;
   cached_size_mb: number;
 }> {
   return { total_attachments: 0, cached_count: 0, cached_size_mb: 0 };
 }

export async function cleanupAttachmentCache(maxKeepMb: number): Promise<number> {
   return 0;
 }

export async function clearAttachmentCache(): Promise<number> {
   return 0;
 }

 export async function saveAttachment(
  filename: string,
  contentBase64: string,
): Promise<string | null> {
  return null;
}

export interface PickedFile {
  filename: string;
  content: string;
  content_type: string;
  size: number;
}

export async function openFilePicker(): Promise<PickedFile[]> {
  // Web version: HTML file input handled by the component; returns empty list.
  return [];
}

export async function fetchMessageBody(
  accountId: number,
  uid: number
): Promise<Message> {
  return get(`/messages/body?account_id=${accountId}&uid=${uid}`,
    "Der Nachrichteninhalt konnte nicht geladen werden.");
}

export async function markAsRead(
  accountId: number,
  uid: number
): Promise<void> {
  return post("/messages/read", { account_id: accountId, uid },
    "Die Nachricht konnte nicht als gelesen markiert werden.");
}

export async function markAsUnseen(
  accountId: number,
  uid: number
): Promise<void> {
  return post("/messages/unread", { account_id: accountId, uid },
    "Die Nachricht konnte nicht als ungelesen markiert werden.");
}

export async function flagMessageCmd(
  accountId: number,
  uid: number,
  folderName: string,
  flagged: boolean,
): Promise<void> {
  return post("/messages/flag", { account_id: accountId, uid, folder_name: folderName, flagged },
    "Die Markierung konnte nicht aktualisiert werden.");
}

export async function deleteMessageCmd(
  accountId: number,
  uid: number
): Promise<void> {
  return post("/messages/delete", { account_id: accountId, uid },
    "Die Nachricht konnte nicht gelöscht werden.");
}

export async function moveMessageCmd(
  accountId: number,
  uid: number,
  sourceFolder: string,
  targetFolder: string,
  rawSourceFolder?: string,
  rawTargetFolder?: string,
): Promise<void> {
  return post("/messages/move", {
    account_id: accountId, uid, source_folder: sourceFolder, target_folder: targetFolder,
    raw_source_folder: rawSourceFolder || "", raw_target_folder: rawTargetFolder || "",
  }, "Die Nachricht konnte nicht verschoben werden.");
}

export async function moveMessageCrossAccount(
  sourceAccountId: number,
  sourceUid: number,
  sourceFolder: string,
  targetAccountId: number,
  targetFolder: string,
): Promise<void> {
  return post("/messages/move-cross-account", {
    account_id: sourceAccountId, uid: sourceUid, source_folder: sourceFolder,
    target_account_id: targetAccountId, target_folder: targetFolder,
  }, "Die Nachricht konnte nicht zwischen Accounts verschoben werden.");
}

export interface OutgoingAttachment {
  filename: string;
  content: string;
  contentType: string;
}

export async function sendMessage(
  accountId: number,
  to: string[],
  subject: string,
  bodyText: string,
  bodyHtml?: string,
  inReplyTo?: string,
  references?: string,
  recipientEmail?: string,
  cc?: string[],
  bcc?: string[],
  attachments?: OutgoingAttachment[],
  aiDraft?: string
): Promise<{ message_id: string; sent_copy_saved: boolean }> {
  return post("/send", {
    account_id: accountId, to, cc, bcc, subject,
    body_text: bodyText, body_html: bodyHtml,
    in_reply_to: inReplyTo, references,
    recipient_email: recipientEmail,
    attachments: attachments?.map(a => ({
      filename: a.filename,
      content: a.content,
      content_type: a.contentType,
      size: Math.ceil(a.content.length * 0.75),
    })),
    ai_draft: aiDraft,
  }, "Die Nachricht konnte nicht gesendet werden.");
}

export async function saveDraft(
  accountId: number,
  to: string[],
  subject: string,
  bodyText: string,
  bodyHtml?: string,
  cc?: string[],
  bcc?: string[]
): Promise<{ uid: number }> {
  return post("/draft/save", {
    account_id: accountId, to, cc, bcc, subject, body_text: bodyText, body_html: bodyHtml,
  }, "Der Entwurf konnte nicht gespeichert werden.");
}

export async function discardDraft(
  accountId: number,
  uid: number,
): Promise<void> {
  return post("/draft/discard", { account_id: accountId, uid },
    "Der Entwurf konnte nicht gelöscht werden.");
}

// ─── Diagnostics ──────────────────────────────────────────

export async function ping(): Promise<string> {
  return get("/health", "Die Verbindung zum Backend konnte nicht hergestellt werden.");
}

// ─── Cache ───────────────────────────────────────────────────

export async function cacheInit(): Promise<void> {
  // Web version: DB init happens on server startup.
  return;
}

// ─── AI ──────────────────────────────────────────────────────

export async function aiGenerateReply(
  accountId: number,
  mailChain: { text: string; html: string | null }[],
  userInput: string,
  recipientEmail: string,
  tone?: { freundlich?: number; professionell?: number; laenge?: number },
  senderName?: string,
  subject?: string,
  recipientName?: string,
): Promise<string> {
  return post("/ai/reply", {
    account_id: accountId,
    mail_chain: mailChain.map(m => m.text),
    user_input: userInput, recipient_email: recipientEmail, tone,
    sender_name: senderName, subject, recipient_name: recipientName,
  }, "Die KI-Antwort konnte nicht generiert werden.");
}

export async function aiSummarize(body: string, accountId: number, uid: number): Promise<string> {
  return post("/ai/summarize", { body, account_id: accountId, uid },
    "Die Zusammenfassung konnte nicht erstellt werden.");
}

export async function triggerFolderSummaries(accountId: number, folder: string): Promise<number> {
  return post("/ai/folder-summaries", { account_id: accountId, folder },
    "Die KI-Zusammenfassungen konnten nicht angestoßen werden.");
}

export async function resetCircuitBreaker(): Promise<void> {
  return post("/ai/reset-circuit-breaker", {},
    "Der KI-Circuit-Breaker konnte nicht zurückgesetzt werden.");
}

export async function aiDraftFromBullets(
  bullets: string,
  toneFreundlich: number,
  toneProfessionell: number,
  toneLaenge: number,
  senderName?: string,
): Promise<string> {
  return post("/ai/draft", {
    bullets,
    tone_freundlich: toneFreundlich, tone_professionell: toneProfessionell, tone_laenge: toneLaenge,
    sender_name: senderName,
  }, "Der KI-Entwurf konnte nicht erstellt werden.");
}

export async function aiFormatText(text: string): Promise<string> {
  return post("/ai/format", { text }, "Der Text konnte nicht formatiert werden.");
}

export async function aiDetectPriority(
  subject: string,
  body: string
): Promise<number> {
  return post("/ai/detect-priority", { subject, body },
    "Die Priorität konnte nicht ermittelt werden.");
}

export async function fraudCheck(
  subject: string,
  body: string
): Promise<{ score: number; warnings: string[] }> {
  return post("/ai/fraud-check", { subject, body },
    "Die Betrugsprüfung konnte nicht durchgeführt werden.");
}

export async function exportToneProfiles(accountId: number): Promise<string> {
  return post("/ai/tone-profiles/export", { accountId },
    "Die Tonfall-Profile konnten nicht exportiert werden.");
}

export async function aiGenerateMail(
  accountId: number,
  to: string,
  subject: string,
  userInput: string,
  senderName: string,
  seriousness: number,
  textLength: number,
  originalMessage?: string,
): Promise<string> {
  return post("/ai/generate-mail", {
    account_id: accountId, to, subject, user_input: userInput, sender_name: senderName,
    seriousness, text_length: textLength, original_message: originalMessage,
  }, "Der KI-Mailtext konnte nicht generiert werden.");
}

export async function getToneProfile(
  accountId: number,
  email: string,
): Promise<{ formality_score: number; friendliness_score: number; sample_count: number } | null> {
  return post("/ai/tone-profile", { account_id: accountId, email },
    "Das Ton-Profil konnte nicht geladen werden.");
}

export async function aiSuggestRecipient(
  to: string,
  subject: string,
  userInput: string,
  originalMessage?: string,
): Promise<string> {
  return post("/ai/suggest-recipient", { to, subject, user_input: userInput, original_message: originalMessage },
    "Die Empfängeradresse konnte nicht ermittelt werden.");
}

export async function aiSuggestSubject(
  to: string,
  subject: string,
  userInput: string,
  originalMessage?: string,
): Promise<string> {
  return post("/ai/suggest-subject", { to, subject, user_input: userInput, original_message: originalMessage },
    "Der Betreff konnte nicht ermittelt werden.");
}

// ─── CardDAV Types ───────────────────────────────────────────

export interface ContactInfo {
  vcard_uid: string;
  given_name: string | null;
  family_name: string | null;
  display_name: string | null;
  email: string | null;
  phone: string | null;
  organization: string | null;
  vcard_raw: string;
}

export interface CardDavSettings {
  url: string;
  username: string;
  password: string;
  sync_interval_minutes: number;
}

// ─── CardDAV ─────────────────────────────────────────────────

export async function searchContacts(query: string): Promise<ContactInfo[]> {
  return post("/carddav/search", { query },
    "Die Kontakte konnten nicht durchsucht werden.");
}

export async function resolveRecipientFromText(text: string): Promise<ContactInfo | null> {
  return post("/carddav/resolve", { text },
    "Der Empfänger konnte nicht ermittelt werden.");
}

export async function syncCardDav(): Promise<number> {
  return post("/carddav/sync", {}, "Die CardDAV-Synchronisation konnte nicht durchgeführt werden.");
}

export async function getCardDavSettings(): Promise<CardDavSettings | null> {
  return get("/carddav/settings", "Die CardDAV-Einstellungen konnten nicht geladen werden.");
}

export async function setCardDavSettings(settings: CardDavSettings): Promise<void> {
  return post("/carddav/settings", settings,
    "Die CardDAV-Einstellungen konnten nicht gespeichert werden.");
}

// ─── Voice ────────────────────────────────────────────────────────

export interface VoiceSettings {
  enabled: boolean;
  sttUrl: string;
  sttKey: string;
  sttModel: string;
}

export async function getVoiceSettings(): Promise<VoiceSettings | null> {
  try {
    return await get("/voice/config", "");
  } catch {
    return null;
  }
}

export async function saveVoiceSettings(
  enabled: boolean,
  stt_url: string,
  stt_key: string,
  stt_model: string,
): Promise<void> {
  return post("/voice/config", { enabled, sttUrl: stt_url, sttKey: stt_key, sttModel: stt_model },
    "Die Voice-Einstellungen konnten nicht gespeichert werden.");
}

export async function voiceTranscribe(audioBase64: string): Promise<string> {
  return post("/voice/transcribe", { audioBase64 },
    "Die Transkription konnte nicht durchgeführt werden.");
}

export interface OwnPhotoResult {
  data: string;
  type: string;
}

export async function getOwnPhoto(): Promise<OwnPhotoResult | null> {
  try {
    return await get("/profile/photo", "");
  } catch {
    return null;
  }
}

export async function saveOwnPhoto(base64: string, mimeType: string): Promise<void> {
  return post("/profile/photo", { photoBase64: base64, photoType: mimeType },
    "Das Profilbild konnte nicht gespeichert werden.");
}

// ─── SSE Events ──────────────────────────────────────────────
// Server → client notifications (new mail, AI summaries) via EventSource.

export function openEventStream(onEvent: (event: string, payload: unknown) => void): EventSource | null {
  if (typeof EventSource === "undefined") return null;
  const es = new EventSource(`${API_BASE}/events`);
  es.onmessage = (e) => {
    try {
      const data = JSON.parse(e.data);
      onEvent(data.event ?? "", data.payload);
    } catch { /* ignore malformed */ }
  };
  return es;
}

// (end of API wrappers)

// ─── Web Push (Notifications) ────────────────────────────────
// VAPID-keyed push: works even when the app is closed (PWA installed to
// home screen on iOS 16.4+ / macOS 13+).

export interface VapidInfo {
  publicKey: string;
  subject: string;
}

export async function getVapidInfo(): Promise<VapidInfo | null> {
  try {
    const res = await fetch(`${API_BASE}/push/vapid`);
    if (!res.ok) return null;
    const d = await res.json();
    return { publicKey: d.public_key ?? "", subject: d.subject ?? "" };
  } catch {
    return null;
  }
}

export async function subscribePush(
  subscription: PushSubscription,
  accountId: number,
): Promise<boolean> {
  try {
    const res = await fetch(`${API_BASE}/push/subscribe`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        endpoint: subscription.endpoint,
        p256dh: b64urlToBase64(arrayBufferToB64(subscription.getKey("p256dh"))),
        auth: b64urlToBase64(arrayBufferToB64(subscription.getKey("auth"))),
        account_id: accountId,
      }),
    });
    return res.ok;
  } catch {
    return false;
  }
}

export async function unsubscribePush(endpoint: string): Promise<boolean> {
  try {
    const res = await fetch(`${API_BASE}/push/unsubscribe`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ endpoint }),
    });
    return res.ok;
  } catch {
    return false;
  }
}

/** Register the service worker and request push permission. */
export async function setupPush(
  accountId: number,
  onDenied?: () => void,
): Promise<"granted" | "denied" | "unsupported" | "registered"> {
  if (typeof window === "undefined" || !("serviceWorker" in navigator) || !("PushManager" in window)) {
    return "unsupported";
  }
  try {
    const reg = await navigator.serviceWorker.register(`${import.meta.env.BASE_URL ?? ""}sw.js`);
    const vapid = await getVapidInfo();
    if (!vapid) return "unsupported";

    let sub = await reg.pushManager.getSubscription();
    if (!sub) {
      const permission = await Notification.requestPermission();
      if (permission !== "granted") {
        onDenied?.();
        return "denied";
      }
      sub = await reg.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: urlBase64ToUint8Array(vapid.publicKey) as unknown as BufferSource,
      });
    }
    const ok = await subscribePush(sub, accountId);
    return ok ? "registered" : "unsupported";
  } catch {
    return "unsupported";
  }
}

export async function teardownPush(): Promise<void> {
  if (typeof window === "undefined" || !("serviceWorker" in navigator)) return;
  try {
    const reg = await navigator.serviceWorker.getRegistration();
    const sub = await reg?.pushManager.getSubscription();
    if (sub) {
      await unsubscribePush(sub.endpoint);
      await sub.unsubscribe();
    }
  } catch { /* ignore */ }
}

export async function pushEnabled(): Promise<boolean> {
  if (typeof window === "undefined" || !("serviceWorker" in navigator) || !("PushManager" in window)) {
    return false;
  }
  try {
    const reg = await navigator.serviceWorker.getRegistration();
    const sub = await reg?.pushManager.getSubscription();
    return !!sub;
  } catch {
    return false;
  }
}

// ─── base64 helpers (server stores standard base64) ──────────
function arrayBufferToB64(buf: ArrayBuffer | null): string {
  if (!buf) return "";
  const bytes = new Uint8Array(buf);
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  return btoa(bin);
}

function b64urlToBase64(b64: string): string {
  return b64.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - (b64.length % 4)) % 4);
}

function urlBase64ToUint8Array(base64: string): Uint8Array {
  const padding = "=".repeat((4 - (base64.length % 4)) % 4);
  const base64Url = (base64 + padding).replace(/-/g, "+").replace(/_/g, "/");
  const raw = atob(base64Url);
  const out = new Uint8Array(raw.length);
  for (let i = 0; i < raw.length; i++) out[i] = raw.charCodeAt(i);
  return out;
}
