// The iOS home screen icon, embedded in app.html as a data: address (Kai,
// 7.10.2026: iOS showed an "R"). Away from home the box answers every
// request without its sign-in cookie with the login page, and iOS fetches
// the apple-touch icon without that cookie; embedded, there is nothing to
// fetch. Runs after app-symbole.mjs; with --pruefen it only checks that
// app.html carries static/apple-touch-icon.png byte for byte.

import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { WEB } from "./app-symbole-formen.mjs";

const html = join(WEB, "src", "app.html");
const bild = readFileSync(join(WEB, "static", "apple-touch-icon.png")).toString("base64");
const zeile = `<link rel="apple-touch-icon" sizes="180x180" href="data:image/png;base64,${bild}" />`;
const muster = /<link rel="apple-touch-icon"[^>]*\/>/;

const alt = readFileSync(html, "utf8");
if (!muster.test(alt)) throw new Error("app.html: kein apple-touch-icon");
const neu = alt.replace(muster, () => zeile);
if (process.argv.includes("--pruefen")) {
  if (neu !== alt) {
    console.error("app.html trägt nicht static/apple-touch-icon.png — node scripts/touch-icon-einbetten.mjs");
    process.exit(1);
  }
} else {
  writeFileSync(html, neu);
  console.log("app.html: apple-touch-icon eingebettet,", bild.length, "Zeichen");
}
