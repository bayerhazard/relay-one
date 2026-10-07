// Relay's icon names for menus, the folder tree and the message list, drawn
// from the CI set (HB-SYMBOL, lib/symbole.ts) — no more Heroicons (CI
// ABGLEICH RL-Z1). Returns markup for `{@html}`; components use <Symbol>.
import { SYMBOLE, type SymbolName, type Symbolgroesse } from "$lib/symbole";

const NAMES: Record<string, SymbolName> = {
  archive: "archiv",
  browser: "browser",
  delete: "loeschen",
  download: "herunterladen",
  draft: "bearbeiten",
  externalLink: "extern",
  flag: "markieren",
  folder: "ordner",
  forward: "weiterleiten",
  hide: "verbergen",
  image: "bild",
  inbox: "eingang",
  junk: "spam",
  markRead: "gelesen",
  markUnread: "post",
  move: "verschieben",
  newSubFolder: "ordner-neu",
  open: "dokument",
  rename: "bearbeiten",
  reply: "antworten",
  resetName: "neu-laden",
  sent: "senden",
  show: "anzeigen",
  starred: "standard",
  trash: "loeschen",
  urgent: "dringend",
};

export function iconSVG(name: string, size: Symbolgroesse = 16): string {
  const symbol = NAMES[name] ?? "ordner";
  // Stroke 1.5 px at every size (CI R2), like <Symbol>.
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="${(1.5 * 24) / size}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" data-symbol="${symbol}">${SYMBOLE[symbol]}</svg>`;
}

// Map an IMAP folder path to its semantic icon (Gmail prefixes, German and
// English names, Yahoo conventions all covered via substring matching).
export function folderIconFor(name: string, size: Symbolgroesse = 16): string {
  const leaf = (name.split("/").pop() ?? name).toLowerCase();
  if (leaf === "inbox" || leaf === "posteingang") return iconSVG("inbox", size);
  if (/(sent|gesendet|postausgang|outbox)/.test(leaf)) return iconSVG("sent", size);
  if (/(draft|entwurf)/.test(leaf)) return iconSVG("draft", size);
  if (/(trash|papierkorb|deleted|l[oö]schen|m[üu]ll|gel[oö]scht)/.test(leaf)) return iconSVG("trash", size);
  if (/(junk|spam|unerwünscht|unerwunscht)/.test(leaf)) return iconSVG("junk", size);
  if (/(archive|archiv)/.test(leaf)) return iconSVG("archive", size);
  if (/(starred|favorit|wichtig)/.test(leaf)) return iconSVG("starred", size);
  return iconSVG("folder", size);
}
