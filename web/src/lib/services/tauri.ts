import type { Message } from "$lib/stores/mailbox";
import type { AccountInfo } from "$lib/stores/accounts";
import type { AISettings } from "$lib/stores/settings";

// ─── API Base ──────────────────────────────────────────────
// Relative base so the app works behind the Olares entrance and in dev
// (vite proxy /api → localhost:3000). No Tauri dependency.

const API_BASE = "/api/v1";

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
  return post("/settings", { url, apiKey, model },
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
  senderEmail: string
): Promise<AccountInfo> {
  return post("/accounts", {
    name, imapHost, imapPort, imapSsl, smtpHost, smtpPort, smtpTls,
    imapUsername, imapPassword, smtpUsername, smtpPassword, senderName, senderEmail,
  }, "Das Konto konnte nicht verbunden werden.");
}

export async function listAccounts(): Promise<AccountInfo[]> {
  return get("/accounts", "Die Kontenliste konnte nicht geladen werden.");
}

export async function deleteAccount(accountId: number): Promise<void> {
  return apiCall("DELETE", `/accounts/${accountId}`, undefined,
    "Das Konto konnte nicht gelöscht werden.");
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

export async function listImapFolders(accountId: number): Promise<Array<{name: string; raw_name: string; delimiter: string; tag: string; attributes?: string[]}>> {
  return get(`/folders?account_id=${accountId}`, "Die Ordnerliste konnte nicht geladen werden.");
}

export async function renameFolder(
  accountId: number,
  oldName: string,
  newName: string
): Promise<void> {
  return post("/folders/rename", { accountId, oldName, newName },
    "Der Ordner konnte nicht umbenannt werden.");
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
   return get(`/messages/${uid}/raw?account_id=${accountId}`,
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
   return get(`/messages/${uid}/attachments?account_id=${accountId}`,
     "Die Anhänge konnten nicht geladen werden.");
 }

 export async function loadAttachmentContent(
   accountId: number,
   uid: number,
   attachmentId: number,
 ): Promise<string> {
   return get(`/messages/${uid}/attachments/${attachmentId}/content?account_id=${accountId}`,
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
  return get(`/messages/${uid}/body?account_id=${accountId}`,
    "Der Nachrichteninhalt konnte nicht geladen werden.");
}

export async function markAsRead(
  accountId: number,
  uid: number
): Promise<void> {
  return post(`/messages/${uid}/read`, { accountId },
    "Die Nachricht konnte nicht als gelesen markiert werden.");
}

export async function markAsUnseen(
  accountId: number,
  uid: number
): Promise<void> {
  return post(`/messages/${uid}/unread`, { accountId },
    "Die Nachricht konnte nicht als ungelesen markiert werden.");
}

export async function flagMessageCmd(
  accountId: number,
  uid: number,
  folderName: string,
  flagged: boolean,
): Promise<void> {
  return post(`/messages/${uid}/flag`, { accountId, folderName, flagged },
    "Die Markierung konnte nicht aktualisiert werden.");
}

export async function deleteMessageCmd(
  accountId: number,
  uid: number
): Promise<void> {
  return post(`/messages/${uid}/delete`, { accountId },
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
  return post(`/messages/${uid}/move`, {
    accountId, sourceFolder, targetFolder,
    rawSourceFolder: rawSourceFolder || "", rawTargetFolder: rawTargetFolder || "",
  }, "Die Nachricht konnte nicht verschoben werden.");
}

export async function moveMessageCrossAccount(
  sourceAccountId: number,
  sourceUid: number,
  sourceFolder: string,
  targetAccountId: number,
  targetFolder: string,
): Promise<void> {
  return post(`/messages/${sourceUid}/move-cross-account`, {
    sourceAccountId, sourceFolder, targetAccountId, targetFolder,
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
    accountId, to, cc, bcc, subject, bodyText, bodyHtml, inReplyTo, references,
    recipientEmail,
    attachments: attachments?.map(a => ({
      filename: a.filename,
      content: a.content,
      content_type: a.contentType,
      size: Math.ceil(a.content.length * 0.75),
    })),
    aiDraft,
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
    accountId, to, cc, bcc, subject, bodyText, bodyHtml,
  }, "Der Entwurf konnte nicht gespeichert werden.");
}

export async function discardDraft(
  accountId: number,
  uid: number,
): Promise<void> {
  return post(`/draft/${uid}/discard`, { accountId },
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
    accountId,
    mailChain: mailChain.map(m => m.text),
    userInput, recipientEmail, tone, senderName, subject, recipientName,
  }, "Die KI-Antwort konnte nicht generiert werden.");
}

export async function aiSummarize(body: string, accountId: number, uid: number): Promise<string> {
  return post("/ai/summarize", { body, accountId, uid },
    "Die Zusammenfassung konnte nicht erstellt werden.");
}

export async function triggerFolderSummaries(accountId: number, folder: string): Promise<number> {
  return post("/ai/folder-summaries", { accountId, folder },
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
    bullets, toneFreundlich, toneProfessionell, toneLaenge, senderName,
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
    accountId, to, subject, userInput, senderName, seriousness, textLength, originalMessage,
  }, "Der KI-Mailtext konnte nicht generiert werden.");
}

export async function getToneProfile(
  accountId: number,
  email: string,
): Promise<{ formality_score: number; friendliness_score: number; sample_count: number } | null> {
  return post("/ai/tone-profile", { accountId, email },
    "Das Ton-Profil konnte nicht geladen werden.");
}

export async function aiSuggestRecipient(
  to: string,
  subject: string,
  userInput: string,
  originalMessage?: string,
): Promise<string> {
  return post("/ai/suggest-recipient", { to, subject, userInput, originalMessage },
    "Die Empfängeradresse konnte nicht ermittelt werden.");
}

export async function aiSuggestSubject(
  to: string,
  subject: string,
  userInput: string,
  originalMessage?: string,
): Promise<string> {
  return post("/ai/suggest-subject", { to, subject, userInput, originalMessage },
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
