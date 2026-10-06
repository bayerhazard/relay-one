// Fetches a stand of the AImighty CI into the copy under web/ci/ (CI
// ABGLEICH Paket 4, RL-V3; Kai 1.10.2026: the CI is the source of tokens,
// icons and building blocks). After Insilo's and Rocket's ci-holen.mjs.
//
//   node scripts/ci-holen.mjs --von <clone of aimighty-ci> --stand ci-YY.M.n
//
// Reads from a clone of the CI repo with `git show <stand>:<path>` — exactly
// the files of the tag, not what the clone has checked out. Which files make
// the package is said by the CI itself, in werkzeug/stand.py. Fetched while
// developing, never while building; the tests check against the copy without
// network (src/lib/__tests__/ci-stand.test.ts). Afterwards the token block
// and every building block are written into src/styles/global.css
// (werkzeug/bauteile.py --einsetzen) and src/lib/symbole.ts is generated
// anew from the icons.
//
// What should change in Relay changes in the CI first (STAND.md there).

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const hier = dirname(fileURLToPath(import.meta.url));
const web = join(hier, "..");
const ZIEL = join(web, "ci");

function argument(name) {
  const i = process.argv.indexOf(name);
  if (i < 0 || !process.argv[i + 1]) {
    console.error("Usage: node scripts/ci-holen.mjs --von <clone of aimighty-ci> --stand ci-YY.M.n");
    process.exit(2);
  }
  return process.argv[i + 1];
}

const von = argument("--von");
const stand = argument("--stand");
if (!/^ci-\d+\.\d+\.\d+$/.test(stand)) {
  console.error(`${stand} is not a stand (ci-YY.M.n). Apps never fetch main.`);
  process.exit(2);
}

const git = (...a) => execFileSync("git", ["-C", von, ...a], { maxBuffer: 64 * 1024 * 1024 });
const commit = git("rev-parse", `${stand}^{commit}`).toString().trim();

// The package from the stand's werkzeug/stand.py: the list PAKET = [...].
const standPy = git("show", `${stand}:werkzeug/stand.py`).toString();
const muster = [...standPy.match(/PAKET = \[([\s\S]*?)\]/)[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
const alsRegex = (m) => new RegExp(`^${m.replace(/[.]/g, "\\.").replace(/\*/g, "[^/]*")}$`);
const alle = git("ls-tree", "-r", "--name-only", commit).toString().split("\n").filter(Boolean);
const dateien = alle.filter((p) => muster.some((m) => alsRegex(m).test(p))).sort();
for (const m of muster) {
  if (!dateien.some((p) => alsRegex(m).test(p))) throw new Error(`${m} matches nothing in ${stand}`);
}

rmSync(ZIEL, { recursive: true, force: true });
const pruefsummen = {};
for (const pfad of dateien) {
  const inhalt = git("show", `${commit}:${pfad}`);
  mkdirSync(dirname(join(ZIEL, pfad)), { recursive: true });
  writeFileSync(join(ZIEL, pfad), inhalt);
  pruefsummen[pfad] = createHash("sha256").update(inhalt).digest("hex");
}
writeFileSync(
  join(ZIEL, "stand.json"),
  JSON.stringify({ stand, commit, quelle: "ska1walker/aimighty-ci", dateien: pruefsummen }, null, 2) + "\n",
);
console.log(`${stand} (${commit.slice(0, 7)}): ${dateien.length} files into ci/`);

// Token block and building blocks into global.css. bauteile.py exits 1 when
// it reports anything — blocks Relay does not carry count too, so only its
// output matters here; ci-stand.test.ts decides what is a finding.
try {
  execFileSync("python3", [join(ZIEL, "werkzeug", "bauteile.py"), join(web, "src", "styles", "global.css"), "--einsetzen", "--ohne-md"], { stdio: "inherit" });
} catch {
  // reported above
}
execFileSync("node", [join(hier, "symbole-erzeugen.mjs")], { stdio: "inherit" });
