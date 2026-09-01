import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, extname } from "node:path";
import { get } from "svelte/store";
import { translations, setLang, translate, localizeError, t } from "$lib/i18n";

function resetLang(): void {
  try {
    localStorage.removeItem("relay_lang");
  } catch {
    /* ignore */
  }
  setLang("de");
}

beforeEach(resetLang);
afterEach(resetLang);

describe("i18n translate", () => {
  it("liefert deutschen Text bei lang=de (Default)", () => {
    expect(translate("splash.welcome")).toBe("Willkommen bei Relay");
  });

  it("liefert englischen Text nach setLang('en')", () => {
    setLang("en");
    expect(translate("splash.welcome")).toBe("Welcome to Relay");
  });

  it("liefert den Key selbst bei unbekanntem Key", () => {
    expect(translate("does.not.exist")).toBe("does.not.exist");
  });

  it("unterstützt Platzhalter {name}", () => {
    expect(translate("mail.deleteFolderMsg", { name: "Test" })).toContain("Test");
  });

  it("derived t()-Store liefert die Übersetzungsfunktion und reagiert auf setLang", () => {
    const tFn = get(t);
    expect(tFn("splash.welcome")).toBe("Willkommen bei Relay");
    setLang("en");
    expect(get(t)("splash.welcome")).toBe("Welcome to Relay");
  });

  it("setLang persistiert die Sprache in localStorage", () => {
    setLang("en");
    expect(localStorage.getItem("relay_lang")).toBe("en");
    setLang("de");
    expect(localStorage.getItem("relay_lang")).toBe("de");
  });
});

describe("i18n key consistency (Refactoring-Absicherung)", () => {
  it("de und en haben exakt dieselben Keys", () => {
    const deKeys = Object.keys(translations.de).sort();
    const enKeys = Object.keys(translations.en).sort();
    const deOnly = deKeys.filter((k) => !translations.en[k]);
    const enOnly = enKeys.filter((k) => !translations.de[k]);
    expect(deOnly).toEqual([]);
    expect(enOnly).toEqual([]);
    expect(deKeys).toEqual(enKeys);
  });

  it("kein de/en-Wert ist leer", () => {
    for (const k of Object.keys(translations.de)) {
      expect(translations.de[k].length).toBeGreaterThan(0);
      expect(translations.en[k].length).toBeGreaterThan(0);
    }
  });
});

describe("localizeError", () => {
  it("mappt bekannte X-Relay-Key-Fehler auf übersetzten Text", () => {
    expect(localizeError("X-Relay-Key fehlt oder ungültig")).toContain("Zugriffs-Schlüssel");
  });

  it("mappt IMAP-Login-Fehler", () => {
    expect(localizeError("IMAP: Login fehlgeschlagen")).toContain("Konto konnte nicht verbunden werden");
  });

  it("mappt parse_to / ungültige Empfänger", () => {
    expect(localizeError("[parse_to] SMTP: Ungültige Empfänger-E-Mail-Adresse")).toContain("Ungültige Empfänger");
  });

  it("lässt unbekannte Fehlermeldungen unverändert durch", () => {
    expect(localizeError("some exotic detail 123")).toBe("some exotic detail 123");
  });
});

describe("i18n t() key reference scan", () => {
  const SRC = join(process.cwd(), "src");

  function collectFiles(dir: string, acc: string[]): string[] {
    for (const entry of readdirSync(dir)) {
      const full = join(dir, entry);
      const st = statSync(full);
      if (st.isDirectory()) {
        if (entry === "__tests__" || entry === "workers") continue;
        collectFiles(full, acc);
      } else if (extname(full) === ".svelte" || extname(full) === ".ts") {
        if (full.endsWith("/i18n.ts")) continue; // Wörterbuch selbst (Kommentar enthält t()-Beispiele)
        acc.push(full);
      }
    }
    return acc;
  }

  function collectUsedKeys(files: string[]): Set<string> {
    const used = new Set<string>();
    // \bt(...) matches standalone t(), translate(...) as its own word; \$t(...)
    // handles the Svelte store subscription form. Word boundaries avoid false
    // positives from e.g. goto(), format(), get(), set().
    const re = /\b(?:t|translate)\s*\(\s*"([^"]+)"/g;
    const reStore = /\$t\s*\(\s*"([^"]+)"/g;
    for (const f of files) {
      const content = readFileSync(f, "utf-8");
      let m: RegExpExecArray | null;
      while ((m = re.exec(content)) !== null) used.add(m[1]);
      reStore.lastIndex = 0;
      while ((m = reStore.exec(content)) !== null) used.add(m[1]);
    }
    return used;
  }

  it("alle t()/translate()-Literale in src verweisen auf existierende Keys", () => {
    const files = collectFiles(SRC, []);
    const usedKeys = collectUsedKeys(files);
    const missing = [...usedKeys].filter((k) => !translations.de[k]);
    expect(missing).toEqual([]);
    expect(usedKeys.size).toBeGreaterThan(20);
  });

  it("alle de-Keys werden mindestens einmal verwendet (kein toter Key)", () => {
    const files = collectFiles(SRC, []);
    const used = collectUsedKeys(files);
    const unused = Object.keys(translations.de).filter((k) => !used.has(k));
    // Fehlermeldungs-Keys, die nur über localizeError/mit Platzhaltern genutzt werden, sind erlaubt:
    const allowedUnused = new Set([
      "error.keyMissing",
      "error.invalidRecipient",
      "error.imapSmtpConnect",
      "error.connection",
      "confirmation.title",
      "confirmation.confirm",
      "prompt.title",
      "prompt.cancel",
      "prompt.ok",
      "error.retry",
      "common.yes",
      "common.no",
      // Referenziert über das translateFolder()-Dictionary (keine t()-Literale).
      "mail.folderInbox",
      "mail.folderSent",
      "mail.folderDrafts",
      "mail.folderTrash",
      "mail.folderSpam",
      "mail.folderArchive",
      "mail.folderJunk",
      "mail.folderGeloescht",
      "mail.folderSpamverdacht",
      // Nutzung über translate() im iframe-preview-Skript (kein t()-Literal).
      "mail.loadImage",
      // Referenziert über das dynamische translate(`assistant.module.${module}`)
      // in AssistantDrawer.svelte (kein statisches t()-Literal).
      "assistant.module.mail",
      "assistant.module.calendar",
      "assistant.module.contacts",
      "assistant.module.tasks",
      "assistant.module.settings",
      // Referenziert über das dynamische $t(`calendar.rsvp.${ps}`) in
      // calendar/+page.svelte (kein statisches t()-Literal).
      "calendar.rsvp.needsAction",
      "calendar.rsvp.accepted",
      "calendar.rsvp.declined",
      "calendar.rsvp.tentative",
    ]);
    const realUnused = unused.filter((k) => !allowedUnused.has(k));
    expect(realUnused).toEqual([]);
  });
});
