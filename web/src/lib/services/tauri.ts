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
function put<T>(path: string, body: unknown, msg: string): Promise<T> {
  return apiCall<T>("PUT", path, body, msg);
}
function del<T>(path: string, msg: string): Promise<T> {
  return apiCall<T>("DELETE", path, undefined, msg);
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
  listOnly?: boolean,
): Promise<Message[]> {
  const q = new URLSearchParams({ account_id: String(accountId) });
  if (limit) q.set("limit", String(limit));
  if (offset) q.set("offset", String(offset));
  if (folder) q.set("folder", folder);
  if (listOnly) q.set("list_only", "1");
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
   part_index: number;
   filename: string;
   content_type: string;
   size: number;
   content: string | null;
   content_cached: boolean;
}

export async function fetchAttachments(
   accountId: number,
   uid: number,
   folder?: string
 ): Promise<AttachmentInfo[]> {
   const q = new URLSearchParams({ account_id: String(accountId), uid: String(uid) });
   if (folder) q.set("folder", folder);
   return get(`/messages/attachments?${q}`,
     "Die Anhänge konnten nicht geladen werden.");
 }

 export async function loadAttachmentContent(
   accountId: number,
   uid: number,
   attachmentId: number,
   folder?: string
 ): Promise<string> {
   const q = new URLSearchParams({ account_id: String(accountId), uid: String(uid), att_id: String(attachmentId) });
   if (folder) q.set("folder", folder);
   const res = await get<{ content?: string }>(`/messages/attachment?${q}`,
     "Der Anhang konnte nicht geladen werden.");
   return res?.content ?? "";
 }

export async function getAttachmentCacheStats(): Promise<{
   total_attachments: number;
   cached_count: number;
   cached_size_mb: number;
 }> {
   const res = await get<{ total_attachments?: number; cached_count?: number; cached_size_mb?: number }>(
     "/attachments/stats",
     "Die Anhang-Cache-Stats konnten nicht geladen werden."
   );
   return {
     total_attachments: res?.total_attachments ?? 0,
     cached_count: res?.cached_count ?? 0,
     cached_size_mb: res?.cached_size_mb ?? 0,
   };
 }

 export async function cleanupAttachmentCache(maxKeepMb: number): Promise<number> {
   const res = await post<{ cleaned?: number }>("/attachments/cleanup", { max_keep_mb: maxKeepMb },
     "Der Anhang-Cache konnte nicht bereinigt werden.");
   return res?.cleaned ?? 0;
 }

 export async function clearAttachmentCache(): Promise<number> {
   const res = await post<{ cleared?: number }>("/attachments/clear", {},
     "Der Anhang-Cache konnte nicht geleert werden.");
   return res?.cleared ?? 0;
 }

 /** `POST /attachments/gc` — run the dedup-store garbage collection on demand. */
 export async function gcAttachments(): Promise<{ removed_files: number; freed_bytes: number; kept_files: number }> {
   const res = await post<{ removed_files?: number; freed_bytes?: number; kept_files?: number }>(
     "/attachments/gc", {},
     "Die Attachment-Garbage-Collection konnte nicht ausgeführt werden."
   );
   return {
     removed_files: res?.removed_files ?? 0,
     freed_bytes: res?.freed_bytes ?? 0,
     kept_files: res?.kept_files ?? 0,
   };
 }

 /** `POST /attachments/repair` — fix has_attachments flags and orphaned disk_paths. */
 export async function repairAttachments(repair = true): Promise<{
   flagged_without_rows: number; unflagged_with_rows: number;
   rows_with_missing_file: number; repaired_rows: number;
 }> {
   const res = await post<{ flagged_without_rows?: number; unflagged_with_rows?: number; rows_with_missing_file?: number; repaired_rows?: number }>(
     "/attachments/repair", { repair },
     "Die Attachment-Reparatur konnte nicht ausgeführt werden."
   );
   return {
     flagged_without_rows: res?.flagged_without_rows ?? 0,
     unflagged_with_rows: res?.unflagged_with_rows ?? 0,
     rows_with_missing_file: res?.rows_with_missing_file ?? 0,
     repaired_rows: res?.repaired_rows ?? 0,
   };
 }

export async function clearAiSummaries(accountId?: number): Promise<number> {
  const res = await post<{ cleared?: number }>("/cache/clear-ai-summaries",
    accountId != null ? { account_id: accountId } : {},
    "Die KI-Zusammenfassungen konnten nicht gelöscht werden.");
  return res?.cleared ?? 0;
}

 export async function saveAttachment(
  filename: string,
  contentBase64: string,
  contentType?: string,
): Promise<string | null> {
  // Web: trigger a browser download (Blob + <a download>). Returns null on
  // failure, otherwise a placeholder (the download itself is side-effectful).
  try {
    const byteChars = atob(contentBase64);
    const bytes = new Uint8Array(byteChars.length);
    for (let i = 0; i < byteChars.length; i++) bytes[i] = byteChars.charCodeAt(i);
    const blob = new Blob([bytes], { type: contentType || "application/octet-stream" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 10000);
    return filename;
  } catch (e) {
    console.error("saveAttachment failed", e);
    return null;
  }
}

export interface PickedFile {
  filename: string;
  content: string;
  content_type: string;
  size: number;
}

export async function openFilePicker(): Promise<PickedFile[]> {
  // Web version: open a real HTML file input and read the selected files as
  // base64 (mirrors the Tauri dialog used in the desktop build).
  const picked = await new Promise<File[]>((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.multiple = true;
    input.onchange = () => {
      const files = Array.from(input.files ?? []);
      input.remove();
      resolve(files);
    };
    input.click();
  });
  if (picked.length === 0) return [];

  const out: PickedFile[] = [];
  for (const file of picked) {
    const content = await new Promise<string>((resolve) => {
      const reader = new FileReader();
      reader.onload = () => {
        const result = reader.result as string;
        resolve(result.split(",")[1] ?? "");
      };
      reader.readAsDataURL(file);
    });
    if (content) {
      out.push({ filename: file.name, content, content_type: file.type || "application/octet-stream", size: file.size });
    }
  }
  return out;
}

export async function fetchMessageBody(
  accountId: number,
  uid: number,
  folder?: string
): Promise<Message> {
  const q = new URLSearchParams({ account_id: String(accountId), uid: String(uid) });
  if (folder) q.set("folder", folder);
  return get(`/messages/body?${q}`,
    "Der Nachrichteninhalt konnte nicht geladen werden.");
}

export async function markAsRead(
  accountId: number,
  uid: number,
  sourceFolder?: string
): Promise<void> {
  return post("/messages/read", { account_id: accountId, uid, source_folder: sourceFolder },
    "Die Nachricht konnte nicht als gelesen markiert werden.");
}

export async function markAsUnseen(
  accountId: number,
  uid: number,
  sourceFolder?: string
): Promise<void> {
  return post("/messages/unread", { account_id: accountId, uid, source_folder: sourceFolder },
    "Die Nachricht konnte nicht als ungelesen markiert werden.");
}

export async function markBatchAsRead(
  accountId: number,
  uids: number[],
  sourceFolder?: string
): Promise<void> {
  return post("/messages/read-batch", { account_id: accountId, uids, source_folder: sourceFolder },
    "Die Nachrichten konnten nicht als gelesen markiert werden.");
}

export async function markBatchAsUnseen(
  accountId: number,
  uids: number[],
  sourceFolder?: string
): Promise<void> {
  return post("/messages/unread-batch", { account_id: accountId, uids, source_folder: sourceFolder },
    "Die Nachrichten konnten nicht als ungelesen markiert werden.");
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
  uid: number,
  sourceFolder?: string
): Promise<void> {
  return post("/messages/delete", { account_id: accountId, uid, source_folder: sourceFolder || "" },
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
  bcc?: string[],
  uid?: number | null,
  attachments?: { filename: string; content: string; contentType: string; size: number }[]
): Promise<{ uid: number }> {
  return post("/draft/save", {
    account_id: accountId, uid: uid ?? null, to, cc, bcc, subject, body_text: bodyText, body_html: bodyHtml,
    attachments: attachments?.map(a => ({
      filename: a.filename,
      content: a.content,
      content_type: a.contentType,
      size: a.size,
    })) ?? null,
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
  return post("/ai/tone-profiles/export", { account_id: accountId },
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

// ─── CalDAV ───────────────────────────────────────────────────────

export interface CalDavSettings {
  url: string;
  username: string;
  password: string;
  sync_interval_minutes: number;
}

export interface CalendarInfo {
  id: number;
  name: string | null;
  color: string | null;
  read_only: boolean;
  last_synced_at: string | null;
}

export interface EventInfo {
  id: number;
  calendar_id: number;
  uid: string;
  summary: string | null;
  start: string;
  end: string | null;
  all_day: boolean;
  location: string | null;
  description: string | null;
  status: string;
  organizer: string | null;
  rrule: string | null;
  /** Number of VALARM reminders attached to the event. */
  alarms?: number;
  /** Start of this specific occurrence (recurring events), RFC 3339 UTC. */
  occurrence_start?: string;
  /** End of this specific occurrence (recurring events), RFC 3339 UTC. */
  occurrence_end?: string;
  /** Attendees (ATTENDEE) of the event, if any. */
  attendees?: { email: string; name?: string | null; part_stat?: string | null; rsvp?: boolean }[];
}

export interface EventAttendeeInfo {
  email: string;
  name: string | null;
  role: string;
  part_stat: string;
  rsvp: boolean;
}

export async function getCalDavSettings(): Promise<CalDavSettings | null> {
  return get("/calendars/settings", "Die CalDAV-Einstellungen konnten nicht geladen werden.");
}

export async function setCalDavSettings(settings: CalDavSettings): Promise<void> {
  return post("/calendars/settings", settings,
    "Die CalDAV-Einstellungen konnten nicht gespeichert werden.");
}

export async function syncCalDav(): Promise<number> {
  return post("/calendars/sync", {}, "Die CalDAV-Synchronisation konnte nicht durchgeführt werden.");
}

export async function getCalendars(): Promise<CalendarInfo[]> {
  return get("/calendars", "Die Kalender konnten nicht geladen werden.");
}

export async function listEvents(calendarId: number | null, from: string, to: string): Promise<EventInfo[]> {
  const cal = calendarId === null ? "" : `calendar_id=${calendarId}&`;
  return get(`/calendars/events?${cal}start=${encodeURIComponent(from)}&end=${encodeURIComponent(to)}`,
    "Die Termine konnten nicht geladen werden.");
}

export async function getEvent(id: number): Promise<EventInfo> {
  return get(`/calendars/events/${id}`, "Der Termin konnte nicht geladen werden.");
}

export async function createEvent(body: {
  calendar_id: number;
  summary: string;
  start: string;
  end?: string;
  description?: string;
  location?: string;
  all_day?: boolean;
  rrule?: string;
  reminder_minutes?: number;
  attendees?: { email: string; name?: string }[];
}): Promise<EventInfo> {
  return post("/calendars/events", body, "Der Termin konnte nicht erstellt werden.");
}

export async function updateEvent(id: number, body: {
  summary?: string;
  start?: string;
  end?: string;
  description?: string;
  location?: string;
  all_day?: boolean;
  rrule?: string;
  reminder_minutes?: number;
  attendees?: { email: string; name?: string }[];
}): Promise<EventInfo> {
  return put(`/calendars/events/${id}`, body, "Der Termin konnte nicht aktualisiert werden.");
}

export async function deleteEvent(id: number): Promise<void> {
  return del(`/calendars/events/${id}`, "Der Termin konnte nicht gelöscht werden.");
}

export async function importEvents(calendarId: number, ics: string): Promise<{ imported: number }> {
  return post<{ imported: number }>(
    "/calendars/events/import",
    { calendar_id: calendarId, ics },
    "Der ICS-Import ist fehlgeschlagen.",
  );
}

export async function getEventIcs(id: number): Promise<{ ics: string; filename: string }> {
  return get<{ ics: string; filename: string }>(
    `/calendars/events/${id}/ics`,
    "Der Termin konnte nicht exportiert werden.",
  );
}

// ─── iMIP Invitations (Phase 2) ───────────────────────────────────

export interface InvitationInfo {
  event_uid: string;
  event_id?: number;
  organizer: string;
  attendee_email: string;
  status: string;
  sequence: number;
  summary?: string;
  start_at?: string;
  end_at?: string;
  location?: string;
}

export async function listInvitations(): Promise<InvitationInfo[]> {
  return get("/invitations", "Die Einladungen konnten nicht geladen werden.");
}

export async function acceptInvitation(uid: string, accountId: number): Promise<void> {
  return post(
    `/invitations/${encodeURIComponent(uid)}/accept`,
    { account_id: accountId },
    "Die Annahme konnte nicht gesendet werden.",
  );
}

export async function declineInvitation(uid: string, accountId: number): Promise<void> {
  return post(
    `/invitations/${encodeURIComponent(uid)}/decline`,
    { account_id: accountId },
    "Die Absage konnte nicht gesendet werden.",
  );
}

// ─── Conflicts + Calendar AI (Phase 2.4 / 2.5) ────────────────────

export async function getConflicts(
  start: string,
  end: string,
  calendarId?: number | null,
  excludeId?: number | null,
): Promise<EventInfo[]> {
  const q = new URLSearchParams({ start, end });
  if (calendarId != null) q.set("calendar_id", String(calendarId));
  if (excludeId != null) q.set("exclude_id", String(excludeId));
  return get(`/calendars/conflicts?${q.toString()}`, "Die Konfliktprüfung ist fehlgeschlagen.");
}

export interface TimeSlot {
  start: string;
  end: string;
  reason?: string;
}

export async function getConflictAlternatives(
  summary: string,
  start: string,
  end: string,
  calendarId?: number | null,
): Promise<TimeSlot[]> {
  return post(
    "/ai/conflict-alternatives",
    { summary, start, end, calendar_id: calendarId ?? null },
    "Die KI-Alternativen konnten nicht geladen werden.",
  );
}

export interface ExtractedTime {
  summary?: string;
  start?: string;
  end?: string;
  all_day: boolean;
}

export async function extractTime(text: string, referenceDate?: string): Promise<ExtractedTime> {
  return post(
    "/ai/extract-time",
    { text, reference_date: referenceDate ?? null },
    "Die Zeit-Erkennung ist fehlgeschlagen.",
  );
}

export async function rsvpDraft(
  summary: string,
  start: string,
  organizer: string,
  decision: string,
  note?: string,
): Promise<string> {
  return post(
    "/ai/rsvp-draft",
    { summary, start, organizer, decision, note: note ?? null },
    "Der KI-Entwurf konnte nicht geladen werden.",
  );
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
  const res = await post<{ text?: string }>("/voice/transcribe", { audioBase64 },
    "Die Transkription konnte nicht durchgeführt werden.");
  return (res?.text ?? "").trim();
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

// ─── Contacts (CardDAV) ─────────────────────────────────────

export interface ContactInfo {
  vcard_uid: string;
  given_name: string | null;
  family_name: string | null;
  display_name: string | null;
  email: string | null;
  phone: string | null;
  organization: string | null;
  source: string;
  synced_at: string;
}

export interface ContactInput {
  given_name: string;
  family_name: string;
  display_name: string;
  email: string;
  phone: string;
  organization: string;
}

export async function listContacts(search = ""): Promise<ContactInfo[]> {
  const q = search ? `?search=${encodeURIComponent(search)}` : "";
  return get<ContactInfo[]>(`/contacts${q}`, "Kontakte konnten nicht geladen werden.");
}

export async function createContact(input: ContactInput): Promise<ContactInfo> {
  return post<ContactInfo>("/contacts", input, "Kontakt konnte nicht angelegt werden.");
}

export async function updateContact(uid: string, input: ContactInput): Promise<ContactInfo> {
  return put<ContactInfo>(`/contacts/${encodeURIComponent(uid)}`, input, "Kontakt konnte nicht aktualisiert werden.");
}

export async function deleteContact(uid: string): Promise<{ deleted: boolean }> {
  return apiCall<{ deleted: boolean }>("DELETE", `/contacts/${encodeURIComponent(uid)}`, undefined, "Kontakt konnte nicht gelöscht werden.");
}

// ─── Todos (VTODO / Aufgaben) ───────────────────────────────

export interface TodoInfo {
  id: number;
  calendar_id: number;
  uid: string;
  summary: string | null;
  description: string | null;
  due_at: string | null;
  completed_at: string | null;
  status: string;
  priority: number | null;
}

export interface TodoInput {
  summary: string;
  description?: string;
  due?: string;
  priority?: number;
}

export async function listTodos(completed?: boolean): Promise<TodoInfo[]> {
  const q = completed === undefined ? "" : `?completed=${completed}`;
  return get<TodoInfo[]>(`/todos${q}`, "Aufgaben konnten nicht geladen werden.");
}

export async function createTodo(input: TodoInput): Promise<TodoInfo> {
  return post<TodoInfo>("/todos", input, "Aufgabe konnte nicht angelegt werden.");
}

export async function toggleTodo(uid: string, completed: boolean): Promise<TodoInfo> {
  return apiCall<TodoInfo>("PATCH", `/todos/${encodeURIComponent(uid)}`, { completed }, "Aufgabe konnte nicht aktualisiert werden.");
}

export async function deleteTodo(uid: string): Promise<{ deleted: boolean }> {
  return apiCall<{ deleted: boolean }>("DELETE", `/todos/${encodeURIComponent(uid)}`, undefined, "Aufgabe konnte nicht gelöscht werden.");
}

export async function syncTodos(): Promise<{ synced: number }> {
  return post<{ synced: number }>("/todos/sync", {}, "Aufgaben konnten nicht synchronisiert werden.");
}

// ─── AI Followups ───────────────────────────────────────────

export interface FollowupTask {
  summary: string;
  due: string | null;
}

export interface FollowupTimeSlot {
  start: string;
  end: string;
  reason?: string | null;
}

export interface FollowupEvent {
  summary: string;
  start: string;
  end: string | null;
  attendees: string[];
  availability: "free" | "busy";
  conflicts: string[];
  alternatives: FollowupTimeSlot[];
}

export interface FollowupEmail {
  to: string;
  subject: string;
  body: string;
}

export interface FollowupAction {
  id: string;
  kind: "task" | "event" | "email";
  label: string;
  task?: FollowupTask;
  event?: FollowupEvent;
  email?: FollowupEmail;
}

export async function getFollowups(
  subject: string,
  from: string,
  body: string,
): Promise<FollowupAction[]> {
  return post<FollowupAction[]>("/ai/followups", { subject, from, body }, "Aufgaben konnten nicht generiert werden.");
}

// ─── Phase 4 — AI-First ─────────────────────────────────────

export interface NlCreateResult {
  type: "event" | "task";
  title: string;
  start: string | null;
  end: string | null;
  attendees: string[];
  description: string | null;
  due: string | null;
}

export async function nlCreate(text: string, context?: string): Promise<NlCreateResult> {
  return post<NlCreateResult>("/ai/nl-create", { text, context }, "NL-Erstellung fehlgeschlagen.");
}

export interface ScheduleSuggestion {
  start: string;
  end: string;
  confidence: number;
  reason: string | null;
}

export async function smartSchedule(
  request: string,
  participants?: string,
  freeSlots?: string,
  constraints?: string,
): Promise<ScheduleSuggestion[]> {
  const res = await post<{ suggestions: ScheduleSuggestion[] }>(
    "/ai/schedule",
    { request, participants, free_slots: freeSlots, constraints },
    "Smart Scheduling fehlgeschlagen.",
  );
  return res.suggestions;
}

export interface MeetingPrepResult {
  attendees: string[];
  agenda: string[];
  prep_notes: string;
}

export async function meetingPrep(
  summary: string,
  start: string,
  attendees: string[],
): Promise<MeetingPrepResult> {
  return post<MeetingPrepResult>("/ai/meeting-prep", { summary, start, attendees }, "Meeting-Prep fehlgeschlagen.");
}

export interface AgendaDigestResult {
  digest: string;
  priorities: string[];
  followups: string[];
}

export async function agendaDigest(date?: string, horizon?: number): Promise<AgendaDigestResult> {
  return post<AgendaDigestResult>("/ai/agenda-digest", { date, horizon }, "Agenda-Digest fehlgeschlagen.");
}

export interface AssistantAction {
  type: string;
  payload: Record<string, unknown>;
}

export interface AssistantResult {
  reply: string;
  actions: AssistantAction[];
}

export interface AssistantHistoryMsg {
  role: "user" | "assistant";
  text: string;
}

export async function askAssistant(
  message: string,
  context?: string,
  history?: AssistantHistoryMsg[],
): Promise<AssistantResult> {
  return post<AssistantResult>("/ai/assistant", { message, context, history }, "Assistent nicht erreichbar.");
}
