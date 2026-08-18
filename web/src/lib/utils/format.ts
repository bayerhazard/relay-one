// Cache "now" for 60s — relative date formatting doesn't need second precision.
import DOMPurify from "dompurify";

let cachedNowVal = 0;
let cachedNowDate = new Date();
function cachedNow(): Date {
    const t = Date.now();
    if (t - cachedNowVal > 60_000) {
        cachedNowVal = t;
        cachedNowDate = new Date();
    }
    return cachedNowDate;
}

/** Reset the now-cache. Exposed for testing with mocked timers. */
export function _resetNowCache(): void {
    cachedNowVal = 0;
    cachedNowDate = new Date();
}

/**
 * Sanitize untrusted email HTML (CR-13 / Stored-XSS).
 *
 * Email bodies are attacker-controlled: rendering them via `{@html}` or
 * embedding them into an outgoing reply without sanitization lets a sender
 * execute scripts/event handlers in the recipient's session. DOMPurify strips
 * scripts, event-handler attributes and `javascript:` URLs while keeping
 * benign formatting. Falls back to a tag-stripping regex when the DOM
 * (DOMPurify) is unavailable.
 */
export function sanitizeHtml(html: string): string {
  if (!html) return "";
  try {
    return DOMPurify.sanitize(html);
  } catch {
    // Last-resort fallback: strip everything that could carry code.
    return html
      .replace(/<script\b[^>]*>[\s\S]*?<\/script\s*>/gi, "")
      .replace(/\son\w+\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)/gi, "")
      .replace(/\shref\s*=\s*(["'])\s*javascript:[^"']*\1/gi, " href=\"#\"")
      .replace(/\ssrc\s*=\s*(["'])\s*javascript:[^"']*\1/gi, "")
      .replace(/<\/?[^>]+>/g, " ");
  }
}

export function extractEmail(from: string | undefined | null): string {
  if (!from) return "";
  const match = from.match(/<([^>]+)>/);
  return match ? match[1] : from;
}

/** Extract every address from one or more comma-separated recipient strings. */
export function extractEmails(...parts: (string | undefined | null)[]): string[] {
  const seen = new Set<string>();
  for (const part of parts) {
    if (!part) continue;
    for (const chunk of part.split(",")) {
      const email = extractEmail(chunk.trim());
      if (email && !seen.has(email)) seen.add(email);
    }
  }
  return Array.from(seen);
}

export function extractName(from: string | undefined | null): string {
  if (!from) return "";
  const match = from.match(/^([^<]+)\s*</);
  return match ? match[1].trim() : from.replace(/<[^>]+>/, '').trim() || from;
}

export function formatDate(dateStr?: string): string {
  if (!dateStr) return "";
  try {
    const date = new Date(dateStr);
    const now = cachedNow();
    const diff = now.getTime() - date.getTime();
    const days = Math.floor(diff / (1000 * 60 * 60 * 24));

    if (days === 0) {
      return date.toLocaleTimeString("de-DE", { hour: "2-digit", minute: "2-digit" });
    } else if (days === 1) {
      return "Gestern";
    } else if (days < 7) {
      return date.toLocaleDateString("de-DE", { weekday: "short" });
    } else {
      return date.toLocaleDateString("de-DE", { day: "2-digit", month: "2-digit", year: "numeric" });
    }
  } catch {
    return dateStr;
  }
}

// ---------------------------------------------------------------------------
// HTML / MIME helpers — regex fallback + mime-parser worker integration
// ---------------------------------------------------------------------------

/**
 * Synchronous regex-based HTML detection. Used as fallback when the
 * mime-parser web worker is unavailable (e.g. test environment, SSR).
 */
export function isHtmlContent(str: string | null | undefined): boolean {
  if (!str) return false;
  const s = str.trim().toLowerCase();
  return (
    s.includes("<!doctype html") ||
    s.includes("<html") ||
    s.includes("<body") ||
    s.includes("<p") ||
    s.includes("<div") ||
    s.includes("<br") ||
    s.includes("<table") ||
    s.includes("<span") ||
    s.includes("<a ") ||
    s.includes("<img") ||
    s.includes("<ul") ||
    s.includes("<ol") ||
    s.includes("<li") ||
    s.includes("<h1") ||
    s.includes("<h2") ||
    s.includes("<h3") ||
    s.includes("<strong") ||
    s.includes("<b") ||
    s.includes("<i") ||
    s.includes("<u") ||
    s.includes("<em") ||
    s.includes("<style") ||
    s.includes("<font") ||
    s.includes("<center")
  );
}

/**
 * Synchronous regex-based HTML extraction from raw MIME.
 * Used as fallback when the mime-parser web worker is unavailable.
 */
export function extractHtmlFromMime(mime: string): string | null {
  if (!mime) return null;
  if (!mime.includes("Content-Type:") && !mime.includes("boundary=")) {
    return null;
  }
  const parts = mime.split(/--[a-zA-Z0-9'()+_,-./=?]+[a-zA-Z0-9'()+_,-./=?\s]*/);
  for (const part of parts) {
    if (part.toLowerCase().includes("content-type:") && part.toLowerCase().includes("text/html")) {
      const index = part.indexOf("\n\n");
      if (index !== -1) return part.slice(index + 2).trim();
      const indexCarriage = part.indexOf("\r\n\r\n");
      if (indexCarriage !== -1) return part.slice(indexCarriage + 4).trim();
    }
  }
  return null;
}

/**
 * Synchronous regex-based plain-text extraction from raw MIME.
 * Used as fallback when the mime-parser web worker is unavailable.
 */
export function extractPlainFromMime(mime: string): string | null {
  if (!mime) return null;
  if (!mime.includes("Content-Type:") && !mime.includes("boundary=")) {
    return mime;
  }
  const parts = mime.split(/--[a-zA-Z0-9'()+_,-./=?]+[a-zA-Z0-9'()+_,-./=?\s]*/);
  for (const part of parts) {
    if (part.toLowerCase().includes("content-type:") && part.toLowerCase().includes("text/plain")) {
      const index = part.indexOf("\n\n");
      if (index !== -1) return part.slice(index + 2).trim();
      const indexCarriage = part.indexOf("\r\n\r\n");
      if (indexCarriage !== -1) return part.slice(indexCarriage + 4).trim();
    }
  }
  return mime;
}

// ---------------------------------------------------------------------------
// Mime-parser web worker integration
// ---------------------------------------------------------------------------

/**
 * Converts plain text to simple HTML: paragraphs for double newlines,
 * &lt;br&gt; for single newlines, HTML entities escaped.
 */
export function textToHtml(text: string): string {
  const escaped = text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
  const paragraphs = escaped.split(/\n\n+/);
  return paragraphs
    .map(p => `<p>${p.replace(/\n/g, "<br>")}</p>`)
    .join("\n");
}

/**
 * Wraps an HTML mail body in a blockquote for quoted replies.
 */
export function wrapHtmlQuote(html: string): string {
  return `<blockquote style="border-left:2px solid #ccc;padding-left:12px;margin:16px 0;color:#666;">\n${html}\n</blockquote>`;
}

export interface MailAttachment {
  filename: string;
  contentType: string;
  size: number;
  /** Base64-encoded content, ready for download. */
  content: string;
}

interface ParsedMimeResult {
  bodyHtml: string | null;
  bodyText: string;
  attachments: MailAttachment[];
}

/**
 * Creates a mime-parser web worker and returns a promise-based API.
 * Falls back to regex parsing if the worker cannot be created.
 */
function createMimeParser(): {
  parse(raw: string): Promise<ParsedMimeResult>;
  destroy(): void;
} | null {
  try {
    const worker = new Worker(
      new URL("../workers/mime-parser.ts", import.meta.url),
      { type: "module" },
    );

    let nextId = 0;
    // Map of request-id -> resolver. Using IDs ensures a response is matched to
    // its exact request, so a slow parse for an old email can never resolve the
    // promise of a newer one (prevents cross-message body bleed).
    const pending = new Map<number, (r: ParsedMimeResult) => void>();

    // Single, persistent listeners (added once, not per parse() call — the old
    // code accumulated an "error" listener on every call → unbounded leak).
    worker.addEventListener("message", (event: MessageEvent) => {
      const id = event.data?.id;
      if (typeof id !== "number") return;
      const resolve = pending.get(id);
      if (!resolve) return;
      pending.delete(id);
      if (event.data?.type === "result" && event.data?.email) {
        resolve({
          bodyHtml: event.data.email.bodyHtml,
          bodyText: event.data.email.bodyText,
          attachments: Array.isArray(event.data.email.attachments) ? event.data.email.attachments : [],
        });
      } else {
        resolve({ bodyHtml: null, bodyText: "", attachments: [] });
      }
    });
    worker.addEventListener("error", () => {
      // On a worker-level error, fail all pending requests gracefully.
      for (const [, resolve] of pending) resolve({ bodyHtml: null, bodyText: "", attachments: [] });
      pending.clear();
    });

    return {
      parse(raw: string): Promise<ParsedMimeResult> {
        return new Promise((resolve) => {
          const id = nextId++;
          pending.set(id, resolve);
          worker.postMessage({ id, raw });
        });
      },
      destroy() {
        pending.clear();
        worker.terminate();
      },
    };
  } catch {
    return null;
  }
}

let _parser: ReturnType<typeof createMimeParser> | null = null;

function getParser() {
  if (!_parser) {
    _parser = createMimeParser();
  }
  return _parser;
}

/**
 * Parses raw MIME content using the dedicated mime-parser web worker.
 * Falls back to synchronous regex parsing if the worker is unavailable.
 *
 * Returns an object with `bodyHtml` (HTML content or null) and `bodyText`
 * (plain text content).
 */
export async function parseMimeWithWorker(mime: string): Promise<ParsedMimeResult> {
  const parser = getParser();
  if (parser) {
    try {
      return await parser.parse(mime);
    } catch {
      // Worker failed — fall through to regex fallback
    }
  }
  // Fallback: use regex-based extraction (no attachment extraction here)
  const bodyHtml = extractHtmlFromMime(mime);
  const bodyText = extractPlainFromMime(mime) || mime;
  return { bodyHtml, bodyText, attachments: [] };
}
