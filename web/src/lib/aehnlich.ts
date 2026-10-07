// "Ähnliche E-Mails" (Kai, 7.10.2026): from a mail's context menu to all
// mails like it in the open folder — by the same sender, the same domain or
// a similar subject — to clean them up together. The search field then
// shows the operator (von:, domain:, betreff:), which the server reads.

export type AehnlichArt = "absender" | "domain" | "betreff";

/** The address in "Name <a@b.de>" or a bare "a@b.de", lower case. */
export function adresseVon(from: string | null | undefined): string | null {
  if (!from) return null;
  const klammer = from.match(/<([^<>\s]+@[^<>\s]+)>/);
  const roh = klammer ? klammer[1] : from.trim();
  return /^[^\s@<>"]+@[^\s@<>"]+\.[^\s@<>"]+$/.test(roh) ? roh.toLowerCase() : null;
}

/** The domain of the sender's address (the server adds its subdomains). */
export function domainVon(from: string | null | undefined): string | null {
  const adresse = adresseVon(from);
  return adresse ? adresse.slice(adresse.indexOf("@") + 1) : null;
}

/**
 * The stable start of a subject: what stays the same from mail to mail of
 * one kind. Replies and forwards lose their prefix; the rest is cut before
 * the first number, comma, colon or dash and kept to four words.
 * "Ihr Einkauf bei Travelscape, Ltd." → "Ihr Einkauf bei Travelscape".
 */
export function betreffKern(subject: string | null | undefined): string | null {
  if (!subject) return null;
  let s = subject.replace(/"/g, "").trim();
  while (/^(re|aw|wg|fw|fwd|antw)\s*:\s*/i.test(s)) s = s.replace(/^(re|aw|wg|fw|fwd|antw)\s*:\s*/i, "");
  s = s.split(/[0-9,:;|–—(]| - /)[0]
    .split(/\s+/).filter(Boolean).slice(0, 4).join(" ");
  return s.length >= 4 ? s : null;
}

/** The search for one way, or null where the mail gives none. */
export function aehnlichAnfrage(art: AehnlichArt, mail: { from?: string | null; subject?: string | null }): string | null {
  if (art === "absender") {
    const a = adresseVon(mail.from);
    return a ? `von:${a}` : null;
  }
  if (art === "domain") {
    const d = domainVon(mail.from);
    return d ? `domain:${d}` : null;
  }
  const k = betreffKern(mail.subject);
  return k ? `betreff:"${k}"` : null;
}

/** Does the search use one of these operators? Then it stays in the open folder. */
export function istAehnlichSuche(q: string): boolean {
  return /(^|\s)(von|from|domain|betreff|subject):\S/i.test(q);
}
