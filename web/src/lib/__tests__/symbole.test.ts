import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";
import { translations } from "$lib/i18n";

/**
 * One icon set for every app (CI ABGLEICH R2, G4, RL-Z1; Kai 2026-10-06):
 * Relay draws only from the CI set, through HB-SYMBOL (<Symbol>). After
 * Rocket's and Insilo's symbole.test.ts. Guarded here:
 * - lib/symbole.ts is generated from the CI copy (web/ci/marke/icons/ui/);
 * - no inline <svg> outside <Symbol> and the two brand marks;
 * - sizes only 16/20/24/40;
 * - no emoji or pictographs standing in for icons, in markup or copy.
 */

const WEB = join(__dirname, "..", "..", "..");
const SRC = join(WEB, "src");

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    if (name === "__tests__" || name.startsWith(".")) return [];
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return files(path);
    return name.endsWith(".svelte") ? [path] : [];
  });
}

const COMPONENTS = files(SRC);
/** The markup of a component: everything before its <style> block. */
const markup = (path: string) => {
  const text = readFileSync(path, "utf8");
  const i = text.lastIndexOf("<style");
  return i < 0 ? text : text.slice(0, i);
};

// The brand marks are drawings, not icons: the AImighty wordmark and the
// assistant's shield. <Symbol> itself is the one place that draws an svg.
const SVG_ALLOWED = new Set(["lib/components/Symbol.svelte", "lib/components/ModuleLogo.svelte", "lib/components/AssistantFab.svelte"]);

// Pictographs and dingbats used as icons (✓ ✕ ⚠ 🔄 📎 …). Allowed: the key
// name ⌫ in a shortcut hint, typographic … · — and the arrows in comments.
const PICTO = /[←-⇿⌀-⌨〉-⏿①-➿⤀-⥿⬀-⯿\u{1F300}-\u{1FAFF}]|&#x(?:2715|2713|2190|270E|2709|26A0|1F[0-9A-Fa-f]{3});/u;

describe("HB-SYMBOL", () => {
  it("lib/symbole.ts is generated from web/ci/marke/icons/ui/", () => {
    expect(() =>
      execFileSync("node", [join(WEB, "scripts", "symbole-erzeugen.mjs"), "--pruefen"], { stdio: "pipe" }),
    ).not.toThrow();
  });

  it("no inline <svg> outside <Symbol> and the brand marks", () => {
    const offenders = COMPONENTS.filter((p) => !SVG_ALLOWED.has(relative(SRC, p)))
      .filter((p) => /<svg\b/.test(markup(p).replace(/`[^`]*`/g, "")))
      .map((p) => relative(SRC, p));
    expect(offenders).toEqual([]);
  });

  it("every <Symbol> is 16, 20, 24 or 40 px", () => {
    const wrong = COMPONENTS.flatMap((p) =>
      [...markup(p).matchAll(/<Symbol\b[^>]*?size=\{(\d+)\}/g)]
        .map((m) => m[1])
        .filter((size) => !["16", "20", "24", "40"].includes(size))
        .map((size) => `${relative(SRC, p)}: ${size} px`),
    );
    expect(wrong).toEqual([]);
  });

  it("no emoji or pictographs as icons in markup", () => {
    const withoutComments = (text: string) =>
      text.replace(/<!--[\s\S]*?-->/g, "").replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/[^\n]*/g, "$1");
    const offenders = COMPONENTS.flatMap((p) =>
      withoutComments(markup(p))
        .split("\n")
        .flatMap((line, i) => (PICTO.test(line) ? [`${relative(SRC, p)}: ${line.trim().slice(0, 80)}`] : [])),
    );
    expect(offenders).toEqual([]);
  });

  it("no emoji or pictographs in UI copy (except the ⌫ key name)", () => {
    const offenders = (["de", "en"] as const).flatMap((lang) =>
      Object.entries(translations[lang])
        .filter(([, text]) => PICTO.test(text.replace(/⌫/g, "")))
        .map(([key]) => `${lang}:${key}`),
    );
    expect(offenders).toEqual([]);
  });
});
