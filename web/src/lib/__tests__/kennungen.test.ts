import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * Kennungen in the CSS (CI ABGLEICH RL-K, Etappe 4; after Rocket's and
 * Insilo's globals.css). Guarded here:
 * - every CI building block global.css names stands there word for word as
 *   in the CI copy (web/ci/bauteile/) — the CI is the source, never edit it
 *   here; which blocks Relay carries is pinned in ci-stand.test.ts;
 * - every section header in global.css names a Kennung;
 * - every component's <style> starts with a section header naming an RL-
 *   Kennung, and components never claim AM-/HB- (those live in global.css).
 * werkzeug/bauteile.py in the CI repo checks the same for global.css.
 */

const WEB = join(__dirname, "..", "..", "..");
const SRC = join(WEB, "src");
const BAUTEILE = join(WEB, "ci", "bauteile");
const GLOBAL = readFileSync(join(SRC, "styles", "global.css"), "utf8");

// The two lines the CI puts above each block file; the app carries the rest.
const GENERIERT =
  "/* Baustein aus dem AImighty-CI (bauteile/) — Quelle ist das CI.\n" +
  "   Eine App trägt den Abschnitt unverändert; geprüft mit werkzeug/bauteile.py. */\n";

const KOPF = /^\s*\/\* ── .*$/gm;
const KENNUNG = /\[((?:AM|HB|RL)-[A-Z]+)\]/;

function svelteFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    if (name === "__tests__" || name.startsWith(".")) return [];
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return svelteFiles(path);
    return name.endsWith(".svelte") ? [path] : [];
  });
}

/** The CSS of a component's own <style> block — the last one, like
 * symbole.test.ts: an earlier <style> may be a string written into the
 * mail iframe. Null if the component has none. */
function style(path: string): string | null {
  const text = readFileSync(path, "utf8");
  const i = text.lastIndexOf("<style");
  if (i < 0) return null;
  const m = text.slice(i).match(/^<style[^>]*>([\s\S]*?)<\/style>/);
  return m ? m[1] : null;
}

// The AM-/HB- blocks global.css carries (its section headers name them).
const GETRAGEN = [...new Set([...GLOBAL.matchAll(/\/\* ── [^\n]*\[((?:AM|HB)-[A-Z]+)\]/g)].map((m) => m[1]))]
  .filter((k) => k !== "AM-TOKEN")
  .sort();

describe("Kennungen", () => {
  it.each(GETRAGEN)("%s stands in global.css word for word", (kennung) => {
    const text = readFileSync(join(BAUTEILE, `${kennung}.css`), "utf8");
    expect(text.startsWith(GENERIERT)).toBe(true);
    const block = text.slice(GENERIERT.length).replace(/^\n+|\n+$/g, "");
    expect(GLOBAL.includes(block)).toBe(true);
  });

  it("every section header in global.css names a Kennung", () => {
    const ohne = [...GLOBAL.matchAll(KOPF)]
      .map((m) => m[0])
      // Inside the token block the CI's own sub-headers carry none.
      .filter((kopf) => GLOBAL.indexOf(kopf) > GLOBAL.indexOf("[RL-SCHRIFT]"))
      .filter((kopf) => !KENNUNG.test(kopf));
    expect(ohne).toEqual([]);
  });

  it("every component's <style> starts with an RL- section header", () => {
    const ohne = svelteFiles(SRC)
      .map((p) => [relative(SRC, p), style(p)] as const)
      .filter(([, css]) => css !== null && css.trim() !== "")
      .filter(([, css]) => !/^\s*\/\* ── .*\[RL-[A-Z]+\]/.test(css!))
      .map(([p]) => p);
    expect(ohne).toEqual([]);
  });

  it("components name only RL- Kennungen", () => {
    const falsch = svelteFiles(SRC).flatMap((p) =>
      [...(style(p) ?? "").matchAll(/\[((?:AM|HB)-[A-Z]+)\]/g)].map((m) => `${relative(SRC, p)}: ${m[1]}`),
    );
    expect(falsch).toEqual([]);
  });
});
