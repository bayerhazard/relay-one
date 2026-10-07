// The app's icons from the Relay app icon (docs/icon/relay.svg; Figma
// "Icon-Labor", section 0, node 472:149) — after Rocket's app-symbole.mjs.
// Never a placeholder, never another picture; if the icon changes, run this
// again. Rendered with Chromium (Playwright):
//
//   RELAY_PLAYWRIGHT=$(npm root -g)/playwright/index.mjs \
//   RELAY_CHROMIUM=/opt/pw-browsers/chromium node scripts/app-symbole.mjs
//
// Three forms: "tab" is the sign in the browser tab (static/favicon.svg, only
// the paper plane, CI G7) as PNG for Safari, in a gold that carries on light
// and dark tab bars; "rund" is the tile as in the market (manifest, market
// icon); "voll" fills the square without corners and rim — iOS and Android
// round it themselves, an own rounding would leave black corners.

import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { WEB as web, ZIELE, form } from "./app-symbole-formen.mjs";

const { chromium } = await import(process.env.RELAY_PLAYWRIGHT ? pathToFileURL(process.env.RELAY_PLAYWRIGHT).href : "playwright");

const browser = await chromium.launch({ executablePath: process.env.RELAY_CHROMIUM || undefined });
const seite = await browser.newPage();
for (const [ziel, art, groesse] of ZIELE) {
  await seite.setViewportSize({ width: groesse, height: groesse });
  const svg = form(art, groesse);
  await seite.setContent(`<style>html,body{margin:0;background:transparent}svg{display:block}</style>${svg}`);
  const bild = await seite.locator("svg").screenshot({ omitBackground: true });
  writeFileSync(join(web, ziel), bild);
  console.log(ziel, groesse, bild.length);
}
await browser.close();
// iOS gets its icon from app.html itself (see touch-icon-einbetten.mjs).
await import("./touch-icon-einbetten.mjs");
