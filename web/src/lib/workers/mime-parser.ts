interface ParsedEmail {
  subject: string;
  from: string;
  to: string;
  date: string;
  bodyText: string;
  bodyHtml: string | null;
  attachments: Attachment[];
}

interface Attachment {
  filename: string;
  contentType: string;
  size: number;
  content: string;
}

function decodeRFC2047(text: string): string {
  return text.replace(/=\?([^?]+)\?([BbQq])\?([^?]*)\?=/g, (_m: string, charset: string, encoding: string, data: string) => {
      try {
      if (encoding.toUpperCase() === "B") {
        const decoded = atob(data);
        return new TextDecoder(charset).decode(
          new Uint8Array([...decoded].map((c) => c.charCodeAt(0)))
        );
      } else if (encoding.toUpperCase() === "Q") {
        const decoded = data.replace(/_/g, " ").replace(/=([0-9A-Fa-f]{2})/g, (_q: string, hex: string) =>
          String.fromCharCode(parseInt(hex, 16))
        );
        return new TextDecoder(charset).decode(
          new Uint8Array([...decoded].map((c) => c.charCodeAt(0)))
        );
      }
    } catch {
      // fall through
    }
    return data;
  });
}

function decodeQuotedPrintable(text: string): string {
  return text
    .replace(/=\r?\n/g, "")
    .replace(/=([0-9A-Fa-f]{2})/g, (_m: string, hex: string) =>
      String.fromCharCode(parseInt(hex, 16))
    );
}

function getHeader(body: string, name: string): string {
  const headerEnd = body.indexOf("\r\n\r\n");
  const headers = headerEnd >= 0 ? body.substring(0, headerEnd) : body;
  const regex = new RegExp(`^${name}:\\s*([^\\r\\n]*(?:\\r?\\n\\s+[^\\r\\n]+)*)`, "im");
  const match = headers.match(regex);
  if (!match) return "";
  return match[1].replace(/\r?\n\s+/g, " ").trim();
}

self.onmessage = (event: MessageEvent<{ id: number; raw: string }>) => {
  const { id, raw } = event.data;
  try {
    const result = parseMime(raw);
    self.postMessage({ id, type: "result", email: result });
  } catch {
    self.postMessage({ id, type: "error", email: null });
  }
};

function parseMime(raw: string): ParsedEmail {
  const subject = decodeRFC2047(getHeader(raw, "Subject"));
  const from = decodeRFC2047(getHeader(raw, "From"));
  const to = decodeRFC2047(getHeader(raw, "To"));
  const date = getHeader(raw, "Date");
  const contentType = getHeader(raw, "Content-Type");

  const attachments: Attachment[] = [];
  let bodyText = "";
  let bodyHtml: string | null = null;

  const boundaryMatch = contentType.match(/boundary="?([^";\s]+)"?/);
  const bodyStart = raw.indexOf("\r\n\r\n") + 4;

  if (boundaryMatch) {
    const boundary = boundaryMatch[1];
    const parts = raw.split(`--${boundary}`);
    const isMultipartAlt = contentType.toLowerCase().includes("multipart/alternative");

    for (const part of parts) {
      if (part.startsWith("--") || part.trim() === "") continue;

      const partContentType = getHeader(part, "Content-Type");
      const transferEncoding = getHeader(part, "Content-Transfer-Encoding");
      const disposition = getHeader(part, "Content-Disposition");
      const partHeadersEnd = part.indexOf("\r\n\r\n");
      if (partHeadersEnd < 0) continue;

      const rawPartBody = part.substring(partHeadersEnd + 4).trimEnd();
      const enc = transferEncoding.toLowerCase();
      let partBody = rawPartBody;

      if (enc === "quoted-printable") {
        partBody = decodeQuotedPrintable(partBody);
      } else if (enc === "base64") {
        try {
          partBody = atob(partBody.replace(/\s/g, ""));
        } catch {
          // keep as-is
        }
      }

      const isAttachment =
        disposition.toLowerCase().includes("attachment") ||
        /name="?[^";\r\n]+/.test(disposition + partContentType) &&
          !partContentType.toLowerCase().startsWith("text/");

      if (isAttachment) {
        const filenameMatch =
          disposition.match(/filename="?([^";\r\n]+)"?/) ||
          partContentType.match(/name="?([^";\r\n]+)"?/);
        // Store the content as base64 for safe transport + later download.
        let contentBase64: string;
        if (enc === "base64") {
          contentBase64 = rawPartBody.replace(/\s/g, "");
        } else {
          try {
            // Re-encode decoded bytes to base64 (latin1-safe).
            contentBase64 = btoa(
              Array.from(partBody, (ch) => String.fromCharCode(ch.charCodeAt(0) & 0xff)).join("")
            );
          } catch {
            contentBase64 = "";
          }
        }
        attachments.push({
          filename: filenameMatch ? filenameMatch[1].trim() : "anhang",
          contentType: partContentType.split(";")[0].trim(),
          size: new Blob([partBody]).size,
          content: contentBase64,
        });
      } else if (isMultipartAlt) {
        if (partContentType.includes("text/html")) {
          bodyHtml = partBody;
        } else {
          bodyText = partBody;
        }
      } else {
        bodyText = partBody;
      }
    }
  } else {
    bodyText = raw.substring(bodyStart).trim();
    const cte = getHeader(raw, "Content-Transfer-Encoding");
    if (cte.toLowerCase() === "base64") {
      try {
        bodyText = atob(bodyText.replace(/\s/g, ""));
      } catch {
        // keep as-is
      }
    } else if (cte.toLowerCase() === "quoted-printable") {
      bodyText = decodeQuotedPrintable(bodyText);
    }
  }

  return {
    subject,
    from,
    to,
    date,
    bodyText,
    bodyHtml,
    attachments,
  };
}
