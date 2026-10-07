import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * Relay holds a stand of the AImighty CI, checked without network (CI
 * ABGLEICH Paket 4, RL-V3; after Insilo's tests/ci-stand.test.ts).
 *
 * The CI is the source of tokens, icons and building blocks.
 * `scripts/ci-holen.mjs` fetches a stand `ci-YY.M.n` into `web/ci/`; this
 * test checks that the copy is unchanged and that Relay matches it. If
 * something differs, it changes in the CI first and is fetched again —
 * never quietly adjusted here.
 */

const WEB = join(__dirname, "..", "..", "..");
const CI = join(WEB, "ci");
const GLOBAL = join(WEB, "src", "styles", "global.css");
const stand = JSON.parse(readFileSync(join(CI, "stand.json"), "utf8")) as {
  stand: string;
  commit: string;
  dateien: Record<string, string>;
};

function dateien(ordner: string): string[] {
  return readdirSync(ordner).flatMap((name) => {
    const pfad = join(ordner, name);
    return statSync(pfad).isDirectory() ? dateien(pfad) : [relative(CI, pfad)];
  });
}

describe(`CI stand ${stand.stand}`, () => {
  it("is a stand, not main", () => {
    expect(stand.stand).toMatch(/^ci-\d+\.\d+\.\d+$/);
    expect(stand.commit).toMatch(/^[0-9a-f]{40}$/);
  });

  it("the copy is unchanged: every file matches its checksum", () => {
    const da = dateien(CI).filter((p) => p !== "stand.json").sort();
    expect(da).toEqual(Object.keys(stand.dateien).sort());
    const abweichend = da.filter(
      (p) => createHash("sha256").update(readFileSync(join(CI, p))).digest("hex") !== stand.dateien[p],
    );
    expect(abweichend).toEqual([]);
  });

  it("the token block in global.css is tokens/app.css", () => {
    const ab = (text: string) => text.slice(text.indexOf(":root {"));
    const css = readFileSync(GLOBAL, "utf8");
    const ende = css.indexOf('html[data-dichte="kompakt"]');
    const block = ab(css.slice(0, css.indexOf("\n", ende) + 1));
    expect(block).toBe(ab(readFileSync(join(CI, "tokens", "app.css"), "utf8")));
  });

  it("every building block section equals bauteile/<KENNUNG>.css", () => {
    let meldung = "";
    try {
      meldung = execFileSync("python3", [join(CI, "werkzeug", "bauteile.py"), GLOBAL, "--ohne-md"], {
        encoding: "utf8",
        stdio: "pipe",
      });
    } catch (e) {
      meldung = (e as { stdout?: string }).stdout ?? String(e);
    }
    // Relay does not carry every block of the CI (no board, no shell yet —
    // that comes with Etappe 6). A missing section is no finding; every one
    // present must be equal, and every header needs a Kennung.
    const befunde = meldung
      .trim()
      .split("\n")
      .filter((z) => z && !/die App hat keinen Abschnitt/.test(z) && !/^\d+ Befund\(e\)$/.test(z) && !/gleich mit dem CI$/.test(z));
    expect(befunde).toEqual([]);
  });

  it("the blocks Relay carries", () => {
    const css = readFileSync(GLOBAL, "utf8");
    const kennungen = [...new Set([...css.matchAll(/\/\* ── [^\n]*\[((?:AM|HB)-[A-Z]+)\]/g)].map((m) => m[1]))];
    // Whoever drops a block Relay already had notices it here.
    expect(kennungen.sort()).toEqual(
      ["AM-BASIS", "AM-FELD", "AM-HAKEN", "AM-KARTE", "AM-KNOPF", "AM-LEER", "HB-DIALOG", "HB-SYMBOL", "HB-ZUSTAND"].sort(),
    );
  });
});
