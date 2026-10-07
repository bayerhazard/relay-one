// Folder roles for the mail column (Kai, 7.10.2026).
// Drafts, Sent, Archive on top, own folders in between, Spam and the
// trash at the bottom — by the server's special-use role, else by name.
// Gmail's views (All Mail, Starred, Important) are hidden; its "[Gmail]"
// shell disappears once its system folders sit at their places.
export type Rolle = "entwuerfe" | "gesendet" | "archiv" | "spam" | "papierkorb" | "sammel";
export const ROLLE_RANG: Record<Rolle, number> = { entwuerfe: 0, gesendet: 1, archiv: 2, spam: 20, papierkorb: 21, sammel: 99 };
const ROLLEN_NAMEN: [Rolle, string[]][] = [
  ["entwuerfe", ["drafts", "entwürfe", "inbox.drafts"]],
  ["gesendet", ["sent", "sent messages", "sent items", "gesendet", "gesendete elemente", "inbox.sent", "inbox.gesendet"]],
  ["archiv", ["archive", "archiv"]],
  ["spam", ["spam", "junk", "junk e-mail", "spamverdacht"]],
  ["papierkorb", ["trash"]],
];
const GMAIL_SAMMEL = ["alle nachrichten", "all mail", "markiert", "starred", "wichtig", "important"];

/** The role of a folder: what the server says, else what its name says. */
export function rolleVon(name: string, rollen: Record<string, Rolle> | undefined, delim: string): Rolle | null {
  if (rollen?.[name]) return rollen[name];
  const teile = name.split(delim || ".");
  const blatt = teile[teile.length - 1].toLowerCase();
  if (teile.length > 1 && /^\[(gmail|google mail)\]$/i.test(teile[0]) && GMAIL_SAMMEL.includes(blatt)) return "sammel";
  if (name === "Trash") return "papierkorb";
  for (const [rolle, namen] of ROLLEN_NAMEN) if (namen.includes(blatt) || namen.includes(name.toLowerCase())) return rolle;
  return null;
}

/** The server's folder list as the sidebar wants it: no unselectable
 *  shells, no Gmail views, the provider's trash only as the local
 *  "Trash" where its mails are kept (until that exists, the provider's
 *  trash stands for it). Also returns the roles. */
export function ordnerSichten<T extends { name: string; tag: string; rolle?: string | null }>(f: T[]): { eintraege: T[]; rollen: Record<string, Rolle> } {
  const rollen: Record<string, Rolle> = {};
  for (const x of f) if (x.rolle) rollen[x.name] = x.rolle as Rolle;
  const hatTrash = f.some((x) => x.name === "Trash");
  const eintraege = f.filter((x) =>
    x.tag !== "noselect" && x.tag !== "sammel" && rollen[x.name] !== "sammel"
    && !(rollen[x.name] === "papierkorb" && x.name !== "Trash" && hatTrash));
  return { eintraege, rollen };
}
