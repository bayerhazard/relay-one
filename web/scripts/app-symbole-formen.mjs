// What each app icon must show, shared by the generator (app-symbole.mjs)
// and the tour (e2e/rundgang.spec.ts), so the check compares against the
// same picture the generator draws (Kai, 7.10.2026: "always make sure the
// PWA icon and the sign in the browser tab are the right ones").

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const WEB = join(dirname(fileURLToPath(import.meta.url)), "..");

/** [file relative to web/, form, size in px] */
export const ZIELE = [
  ["static/favicon-32.png", "tab", 32],
  ["static/apple-touch-icon.png", "voll", 180],
  ["static/icon.png", "rund", 512],
  ["static/maskable-icon.png", "voll", 512],
  // The market icon (OlaresManifest: icon) — the same tile.
  ["../icon.png", "rund", 512],
];

/** The SVG for one form at one size. */
export function form(art, groesse) {
  const quelle = readFileSync(join(WEB, "..", "docs", "icon", "relay.svg"), "utf8");
  const tab = readFileSync(join(WEB, "static", "favicon.svg"), "utf8");
  let svg;
  if (art === "rund") svg = quelle;
  else if (art === "tab") svg = tab.replace(/<style>[\s\S]*?<\/style>/, "").replace("<g ", '<g stroke="#b08a3e" ');
  else {
    svg = quelle.replace('<g clip-path="url(#kachel)">', "<g>").replace(/<rect x="1" y="1" width="158"[^>]*\/>/, "");
    if (svg === quelle) throw new Error("relay.svg has changed — check the replacements");
  }
  return svg.replace(/width="(512|32)" height="(512|32)"/, `width="${groesse}" height="${groesse}"`);
}
