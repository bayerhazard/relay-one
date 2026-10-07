// The tour: every page once, on the desktop and on the phone, light and dark
// (CI Etappe 7, after Rocket's frontend/e2e/rundgang.spec.ts).
//
// Checked is what found real faults in Rocket's GUI review of 30.9.2026: an
// API answer 4xx/5xx, a script error, a page wider than the screen, not
// exactly one h1, something sticking out of the page head, the layout check
// (lage.ts), sign-only buttons without a name or tooltip (CI G4), signs off
// 16/20/24/40 or a stroke other than 1.5 px (CI R2), and axe findings of the
// levels "critical" and "serious", contrast included.

import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { lagePruefen } from "./lage";
import { WEB, ZIELE, form } from "../scripts/app-symbole-formen.mjs";

// With RELAY_DATEN=1 the tour runs against a Relay filled by
// e2e/testserver/befuellen.py: every page must then show its dummy data, and
// the mail page opens the newest mail.
const DATEN = !!process.env.RELAY_DATEN;

/** A page of the tour: its path, for settings the section to choose, and
 * what it must show with data (text, and a row to open first). */
interface Seite {
  name: string;
  pfad: string;
  teil?: string;
  inhalt?: string;
  oeffnen?: string;
}

const SEITEN: Seite[] = [
  { name: DATEN ? "Mail" : "Mail (Einrichtung)", pfad: "/", oeffnen: ".message-item", inhalt: "Angebot Messestand Frühjahr" },
  { name: "Kontakte", pfad: "/contacts", inhalt: "Jonas Weber" },
  // The phone shows only the time in a month cell.
  { name: "Kalender", pfad: "/calendar", inhalt: "12:30" },
  { name: "Meetings", pfad: "/meetings" },
  { name: "Aufgaben", pfad: "/tasks", inhalt: "Angebot Messestand prüfen" },
  ...["Allgemein", "E-Mail-Konten", "Kontakte", "Kalender", "AI & Text", "Voice", "Cache", "Archiv"].map((teil) => ({
    name: `Einstellungen ${teil}`,
    pfad: "/settings",
    teil,
  })),
];

/** Choose a settings section in the column — on the phone the column is a
 * sheet opened from the header. */
async function abschnitt(page: Page, name: string) {
  const spalte = page.locator("#relay-spalte");
  if (!(await spalte.isVisible())) await page.getByRole("button", { name: "Spalte öffnen" }).click();
  // A row may end in a count ("E-Mail-Konten 1").
  await spalte.getByRole("button", { name: new RegExp(`^${name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}( \\d+)?$`) }).click();
  await page.waitForTimeout(300);
}

for (const thema of ["light", "dark"] as const)
for (const { name, pfad, teil, inhalt, oeffnen } of SEITEN) {
  test(`${name} ${thema === "light" ? "hell" : "dunkel"}`, async ({ page, context }) => {
    await context.addInitScript((t) => {
      // Also runs in the sandboxed mail frame, which has no storage.
      try {
        localStorage.setItem("relay_appearance", t);
        localStorage.setItem("relay_onboarding_done", "1");
      } catch {
        /* sandboxed frame */
      }
    }, thema);
    const fehler: string[] = [];
    page.on("pageerror", (e) => fehler.push(`Skriptfehler: ${e.message}`));
    page.on("response", (r) => {
      // The tour has no language model; the AI answers are not its subject.
      if (r.url().includes("/api/") && !r.url().includes("/api/v1/ai/") && r.status() >= 400) {
        fehler.push(`API ${r.status()} ${r.request().method()} ${new URL(r.url()).pathname}`);
      }
    });

    // Not "networkidle": mail and meetings keep a connection open.
    await page.goto(pfad);
    await page.locator("h1").first().waitFor();
    await page.waitForTimeout(500);
    if (teil) await abschnitt(page, teil);
    if (DATEN && oeffnen) {
      await page.locator(oeffnen).first().click();
      await page.waitForTimeout(800);
    }
    if (DATEN && inhalt) await expect(page.getByText(inhalt).first(), "Beispieldaten sichtbar").toBeVisible();

    const lage = await page.evaluate(() => {
      const kopf = document.querySelector(".seitenkopf");
      const k = kopf?.getBoundingClientRect();
      const heraus = kopf
        ? [...kopf.querySelectorAll("*")]
            .filter((e) => {
              const r = e.getBoundingClientRect();
              return r.width > 0 && (r.bottom > k!.bottom + 1 || r.right > k!.right + 1 || r.left < k!.left - 1);
            })
            .map((e) => `${e.tagName.toLowerCase()}.${String(e.className).split(" ")[0]}`)
        : [];
      return {
        ueberlauf: document.documentElement.scrollWidth - window.innerWidth,
        ueberschriften: document.querySelectorAll("h1").length,
        heraus: [...new Set(heraus)],
        titel: document.title,
      };
    });

    const lageFunde = await page.evaluate(lagePruefen);

    const symbolknoepfe = await page.evaluate(() =>
      [...document.querySelectorAll<HTMLElement>("button, a[href], [role=button]")]
        .filter((e) => e.checkVisibility() && e.innerText.trim() === "")
        // The word mark is an image with a name, not a sign button.
        .filter((e) => e.getAttribute("role") !== "switch" && !e.closest("label") && !e.querySelector("img"))
        .filter((e) => !(e.getAttribute("aria-label") || e.getAttribute("aria-labelledby")) || !e.getAttribute("title"))
        .map((e) => `${e.tagName.toLowerCase()}.${String(e.className).split(" ")[0]} „${e.getAttribute("aria-label") ?? ""}“`),
    );

    const zeichen = await page.evaluate(() =>
      [...document.querySelectorAll<SVGSVGElement>("svg[data-symbol]")]
        .filter((s) => s.getBoundingClientRect().width > 0)
        .flatMap((s) => {
          const breite = Math.round(s.getBoundingClientRect().width);
          const strich = (parseFloat(getComputedStyle(s).strokeWidth) * breite) / 24;
          const n = s.getAttribute("data-symbol");
          return [16, 20, 24, 40].includes(breite) && Math.abs(strich - 1.5) < 0.05 ? [] : [`${n}: ${breite} px, Strich ${strich.toFixed(2)}`];
        }),
    );

    const axe = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "best-practice"]).analyze();
    const schwer = axe.violations
      .filter((v) => v.impact === "critical" || v.impact === "serious")
      .map((v) => `axe ${v.id}: ${v.nodes.slice(0, 2).map((n) => n.target.join(" ")).join(" ; ")}`);

    expect(fehler, "Fehler beim Laden").toEqual([]);
    expect(lage.ueberlauf, "Seite breiter als der Bildschirm").toBeLessThanOrEqual(1);
    expect(lage.ueberschriften, "genau eine h1").toBe(1);
    expect(lage.heraus, "ragt aus dem Seitenkopf").toEqual([]);
    expect(lage.titel, "Tab-Titel „Seite · Relay“ (CI G7)").toMatch(/(^| · )Relay$/);
    expect(lageFunde, "Rand, Abstand, Mitte (e2e/lage.ts)").toEqual([]);
    expect([...new Set(symbolknoepfe)], "Symbolknopf ohne Namen oder Tooltip").toEqual([]);
    expect([...new Set(zeichen)], "Zeichen außerhalb 16/20/24/40 oder Strich nicht 1,5").toEqual([]);
    expect(schwer, "axe critical/serious").toEqual([]);
  });
}

// The icons of the web app (Etappe 3): favicon as SVG and PNG with only the
// paper plane (CI G7), apple-touch icon and manifest icons as PNG.
test("Web-App-Symbole: Favicon, Apple-Touch-Icon und Manifest", async ({ page }) => {
  await page.goto("/contacts");
  const tab = await page.evaluate(() => [...document.querySelectorAll('link[rel="icon"]')].map((l) => l.getAttribute("href")!));
  const arten: string[] = [];
  for (const pfad of tab) {
    const antwort = await page.request.get(pfad);
    expect(antwort.status(), pfad).toBe(200);
    arten.push(antwort.headers()["content-type"].split(";")[0]);
  }
  expect(arten.sort(), "Tab-Zeichen als SVG und PNG").toEqual(["image/png", "image/svg+xml"]);
  const [touch, manifestPfad] = await page.evaluate(() => [
    document.querySelector('link[rel="apple-touch-icon"]')?.getAttribute("href") ?? null,
    document.querySelector('link[rel="manifest"]')?.getAttribute("href") ?? null,
  ]);
  expect(touch && manifestPfad, "Link-Tags im Kopf").toBeTruthy();
  const manifest = await (await page.request.get(manifestPfad!)).json();
  expect(manifest.short_name).toBe("Relay");
  for (const pfad of [touch!, ...manifest.icons.map((i: { src: string }) => i.src)]) {
    const antwort = await page.request.get(pfad);
    expect(antwort.status(), pfad).toBe(200);
    expect(antwort.headers()["content-type"].split(";")[0], pfad).toBe("image/png");
  }
});

// The right picture in every icon (Kai, 7.10.2026: "always make sure the PWA
// icon and the sign in the browser tab are the right ones"). Each delivered
// PNG is compared with the picture drawn fresh from docs/icon/relay.svg —
// a mean difference, not bytes, so another Chromium does not fail it; a
// wrong picture (another app's icon, the tile in the tab) is far above it.
test("Web-App-Symbole zeigen das richtige Bild", async ({ page }, info) => {
  test.skip(info.project.name !== "desktop", "einmal genügt");
  await page.goto("/contacts");
  const abstand = (png: string, svg: string, groesse: number) =>
    page.evaluate(async ({ png, svg, groesse }) => {
      const laden = (src: string) => new Promise<HTMLImageElement>((ok, nein) => {
        const bild = new Image();
        bild.onload = () => ok(bild);
        bild.onerror = nein;
        bild.src = src;
      });
      const pixel = (bild: HTMLImageElement) => {
        const c = document.createElement("canvas");
        c.width = c.height = groesse;
        const k = c.getContext("2d")!;
        k.drawImage(bild, 0, 0, groesse, groesse);
        return k.getImageData(0, 0, groesse, groesse).data;
      };
      const a = pixel(await laden("data:image/png;base64," + png));
      const b = pixel(await laden("data:image/svg+xml;charset=utf-8," + encodeURIComponent(svg)));
      let summe = 0;
      for (let i = 0; i < a.length; i++) summe += Math.abs(a[i] - b[i]);
      return summe / a.length;
    }, { png, svg, groesse });

  for (const [ziel, art, groesse] of ZIELE as [string, string, number][]) {
    const png = readFileSync(join(WEB, ziel)).toString("base64");
    expect(await abstand(png, form(art, groesse), groesse), `${ziel} zeigt nicht ${art}`).toBeLessThan(4);
  }
  // The check tells pictures apart: the tile is not the sign of the tab.
  const kachel = readFileSync(join(WEB, "static/icon.png")).toString("base64");
  expect(await abstand(kachel, form("tab", 512), 512), "Gegenprobe").toBeGreaterThan(20);
});

// "Abo beenden" (List-Unsubscribe): only where the sender offers it. The test
// server's newsletter names a one-click address that does not resolve, so
// Relay falls back to the unsubscribe mail, which the test server accepts.
test("Abo beenden am Newsletter", async ({ page, context }) => {
  test.skip(!DATEN, "braucht die Beispieldaten");
  await context.addInitScript(() => {
    try {
      localStorage.setItem("relay_onboarding_done", "1");
    } catch {
      /* sandboxed frame */
    }
  });
  await page.goto("/");
  await page.locator(".message-item").first().waitFor();
  const knopf = page.getByRole("button", { name: "Abo beenden" });
  await page.locator(".message-item", { hasText: "Newsletter Stadtwerke" }).click();
  await expect(knopf).toBeVisible();
  await knopf.click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toContainText("stadtwerke.example");
  await expect(dialog.getByRole("button", { name: "Abbrechen" })).toBeFocused();
  await dialog.getByRole("button", { name: "Abo beenden" }).click();
  await expect(page.getByRole("status")).toContainText("abmelden@stadtwerke.example");

  // A mail without List-Unsubscribe gets no button (fresh list: on the phone
  // the reading view covers it).
  await page.goto("/");
  await page.locator(".message-item", { hasText: "Jonas Weber" }).first().click();
  await page.waitForTimeout(1000);
  await expect(knopf, "kein Knopf an einer Mail ohne List-Unsubscribe").toHaveCount(0);
});

// Aufräumen (Kai, 7.10.2026). Every batch is undone again, so the dummy data
// stays the same for the other tests.
test("Aufräumen: Auswahl, Archiv und Rückgängig", async ({ page, context }, info) => {
  test.skip(!DATEN, "braucht die Beispieldaten");
  test.skip(info.project.name !== "desktop", "Mehrfachauswahl mit Strg-Klick");
  await context.addInitScript(() => {
    try { localStorage.setItem("relay_onboarding_done", "1"); } catch { /* sandboxed frame */ }
  });
  await page.goto("/");
  const zeilen = page.locator(".message-item");
  await zeilen.first().waitFor();
  const vorher = await zeilen.count();
  await zeilen.nth(1).click();
  await zeilen.nth(2).click({ modifiers: ["Control"] });
  await page.locator(".selection-toolbar").getByRole("button", { name: "Archivieren", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("2 Mails ins Archiv verschoben");
  await expect(zeilen).toHaveCount(vorher - 2);
  await page.getByRole("status").getByRole("button", { name: "Rückgängig" }).click();
  await expect(zeilen).toHaveCount(vorher);
});

test("Aufräumen: nach Absender und Durchgehen", async ({ page, context, request }) => {
  test.skip(!DATEN, "braucht die Beispieldaten");
  await context.addInitScript(() => {
    try { localStorage.setItem("relay_onboarding_done", "1"); } catch { /* sandboxed frame */ }
  });
  const schalter = await request.post("/api/v1/settings/aufraeumen", { data: true });
  expect(schalter.ok()).toBeTruthy();
  try {
    await page.goto("/");
    await page.locator(".message-item").first().waitFor();
    const spalte = page.locator("#relay-spalte");
    if (!(await spalte.isVisible())) await page.getByRole("button", { name: "Spalte öffnen" }).click();
    await spalte.getByRole("button", { name: "Aufräumen" }).click();
    await expect(page.getByRole("heading", { level: 1, name: "Aufräumen" })).toBeVisible();

    // Jonas Weber wrote twice: archive both, then take it back.
    const zeile = page.locator(".aufraeumen-zeile", { hasText: "jonas.weber@beispiel.de" });
    await zeile.getByRole("button", { name: "Alle 2 archivieren" }).click();
    await expect(page.getByRole("status")).toContainText("2 Mails ins Archiv verschoben");
    await expect(zeile).toHaveCount(0);
    await page.getByRole("status").getByRole("button", { name: "Rückgängig" }).click();
    await expect(page.locator(".aufraeumen-zeile", { hasText: "jonas.weber@beispiel.de" })).toBeVisible();
    // The newsletter offers "Abo beenden" here too.
    await expect(page.locator(".aufraeumen-zeile", { hasText: "stadtwerke.example" })
      .getByRole("button", { name: "Abo beenden" })).toBeVisible();

    const ueberlauf = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(ueberlauf, "Seite breiter als der Bildschirm").toBeLessThanOrEqual(1);
    const axe = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
    expect(axe.violations.filter((v) => v.impact === "critical" || v.impact === "serious").map((v) => v.id)).toEqual([]);

    // Durchgehen: one mail at a time, keys included.
    await page.getByRole("button", { name: "Durchgehen" }).click();
    await expect(page.getByRole("heading", { level: 1, name: "Durchgehen" })).toBeVisible();
    await expect(page.locator(".seitenkopf-zahl")).toHaveText(/^1 von \d+$/);
    await page.getByRole("button", { name: "Behalten" }).click();
    await expect(page.locator(".seitenkopf-zahl")).toHaveText(/^2 von \d+$/);
    await page.keyboard.press("e");
    await expect(page.getByRole("status")).toContainText("Mail ins Archiv verschoben");
    await page.getByRole("status").getByRole("button", { name: "Rückgängig" }).click();
    await page.getByRole("button", { name: "Beenden" }).click();
    await expect(page.getByRole("heading", { level: 1, name: "Aufräumen" })).toBeVisible();
  } finally {
    await request.post("/api/v1/settings/aufraeumen", { data: false });
  }
});
