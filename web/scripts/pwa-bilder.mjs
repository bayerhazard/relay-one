// The screenshots the install dialog shows (manifest "screenshots"; Kai,
// 7.10.2026): Relay with the test servers' dummy data, never a real
// mailbox. Run against a Relay filled by e2e/testserver/befuellen.py:
//
//   RELAY_URL=http://127.0.0.1:3800 RELAY_CHROMIUM=… node scripts/pwa-bilder.mjs

import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { WEB } from "./app-symbole-formen.mjs";

const { chromium } = await import(process.env.RELAY_PLAYWRIGHT ? pathToFileURL(process.env.RELAY_PLAYWRIGHT).href : "playwright");
const url = process.env.RELAY_URL ?? "http://127.0.0.1:3800";

const BILDER = [
  ["static/bilder/relay-desktop.png", { width: 1440, height: 900 }],
  ["static/bilder/relay-handy.png", { width: 390, height: 844 }],
];

const browser = await chromium.launch({ executablePath: process.env.RELAY_CHROMIUM || undefined });
for (const [ziel, viewport] of BILDER) {
  const ctx = await browser.newContext({ viewport, locale: "de-DE", colorScheme: "light" });
  await ctx.addInitScript(() => { try { localStorage.setItem("relay_onboarding_done", "1"); } catch { /* */ } });
  const seite = await ctx.newPage();
  await seite.goto(url + "/");
  await seite.locator(".message-item").first().waitFor();
  // On the desktop a mail is open; on the phone the list is the picture.
  if (viewport.width > 1000) {
    await seite.locator(".message-item", { hasText: "Lena Hoffmann" }).first().click();
    await seite.locator(".werkzeugleiste").waitFor();
  }
  await seite.waitForTimeout(800);
  await seite.screenshot({ path: join(WEB, ziel) });
  console.log(ziel);
  await ctx.close();
}
await browser.close();
