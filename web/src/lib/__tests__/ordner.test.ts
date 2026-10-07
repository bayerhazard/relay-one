import { describe, expect, it } from "vitest";
import { ordnerSichten, rolleVon, ROLLE_RANG } from "$lib/ordner";

/**
 * Folder roles (Kai, 7.10.2026): Drafts, Sent, Archive on top, Spam and the
 * trash at the bottom; Gmail's "[Google Mail]" shell and its views (All
 * Mail, Starred, Important) never show.
 */

// What the server sends for a German Gmail account (LIST "" "*").
const GMAIL = [
  { name: "INBOX", tag: "folder", rolle: null },
  { name: "Fahrrad Leasing", tag: "folder", rolle: null },
  { name: "Politik", tag: "folder", rolle: null },
  { name: "Politik/Kommune", tag: "folder", rolle: null },
  { name: "[Google Mail]", tag: "noselect", rolle: null },
  { name: "[Google Mail]/Alle Nachrichten", tag: "sammel", rolle: "sammel" },
  { name: "[Google Mail]/Entwürfe", tag: "folder", rolle: "entwuerfe" },
  { name: "[Google Mail]/Gesendet", tag: "folder", rolle: "gesendet" },
  { name: "[Google Mail]/Markiert", tag: "sammel", rolle: "sammel" },
  { name: "[Google Mail]/Papierkorb", tag: "folder", rolle: "papierkorb" },
  { name: "[Google Mail]/Spam", tag: "folder", rolle: "spam" },
  { name: "[Google Mail]/Wichtig", tag: "sammel", rolle: "sammel" },
  // No attribute, no mail: hidden by name too (Kai, 7.10.2026).
  { name: "[Google Mail]/Chats", tag: "folder", rolle: null },
  { name: "Trash", tag: "", rolle: null },
];

describe("folder roles", () => {
  it("drops the shell, Gmail's views and the provider's second trash", () => {
    const { eintraege, rollen } = ordnerSichten(GMAIL);
    expect(eintraege.map((x) => x.name)).toEqual([
      "INBOX", "Fahrrad Leasing", "Politik", "Politik/Kommune",
      "[Google Mail]/Entwürfe", "[Google Mail]/Gesendet", "[Google Mail]/Spam", "Trash",
    ]);
    expect(rollen["[Google Mail]/Gesendet"]).toBe("gesendet");
  });

  it("keeps the provider's trash while there is no local one", () => {
    const { eintraege } = ordnerSichten(GMAIL.filter((x) => x.name !== "Trash"));
    expect(eintraege.map((x) => x.name)).toContain("[Google Mail]/Papierkorb");
  });

  it("orders system folders around the own ones, the trash last", () => {
    const { eintraege, rollen } = ordnerSichten(GMAIL);
    const rang = (n: string) => { const r = rolleVon(n, rollen, "/"); return r ? ROLLE_RANG[r] : 10; };
    const oben = eintraege.filter((x) => x.name !== "INBOX" && !x.name.includes("/") || rolleVon(x.name, rollen, "/"))
      .map((x, i) => ({ n: x.name, i }))
      .sort((a, b) => rang(a.n) - rang(b.n) || a.i - b.i)
      .map(({ n }) => n);
    expect(oben).toEqual([
      "[Google Mail]/Entwürfe", "[Google Mail]/Gesendet",
      "Fahrrad Leasing", "Politik",
      "[Google Mail]/Spam", "Trash",
    ]);
  });

  it("knows the roles by name where the server sends none", () => {
    expect(rolleVon("Gesendet", {}, "/")).toBe("gesendet");
    expect(rolleVon("INBOX.Drafts", {}, ".")).toBe("entwuerfe");
    expect(rolleVon("Spamverdacht", {}, "/")).toBe("spam");
    expect(rolleVon("Trash", {}, "/")).toBe("papierkorb");
    expect(rolleVon("[Gmail]/All Mail", {}, "/")).toBe("sammel");
    expect(rolleVon("Projekte/Markiert", {}, "/")).toBeNull();
    expect(rolleVon("Makler", {}, "/")).toBeNull();
  });
});
