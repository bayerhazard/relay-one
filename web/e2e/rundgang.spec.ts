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
import http from "node:http";
import type { AddressInfo } from "node:net";
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
  { name: "Kontakte", pfad: "/contacts", inhalt: "Jonas Weber", oeffnen: ".ct-item" },
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
  // iOS: embedded, nothing to fetch behind the box's sign-in.
  expect(touch!, "Apple-Touch-Icon eingebettet").toMatch(/^data:image\/png;base64,/);
  for (const pfad of manifest.icons.map((i: { src: string }) => i.src)) {
    const antwort = await page.request.get(pfad);
    expect(antwort.status(), pfad).toBe(200);
    expect(antwort.headers()["content-type"].split(";")[0], pfad).toBe("image/png");
  }
});

// Installable behind the box's sign-in (Kai, 7.10.2026: Chrome offered no
// install). Olares lets only requests with the sign-in cookie through and
// answers the rest with its login page; Chrome fetches the manifest without
// cookies unless the link asks for them. Here a gate does what the box does,
// and Chrome itself says whether Relay can be installed.
test("PWA: installierbar hinter der Anmeldung der Box", async ({ page, context, baseURL }, info) => {
  test.skip(info.project.name !== "desktop", "einmal genügt");
  const ziel = new URL(baseURL ?? "http://127.0.0.1:3000");
  // The gate: a proxy in front of Relay that lets through only requests
  // carrying the sign-in cookie, as the box's entrance does.
  const ohneCookie: string[] = [];
  const tor = http.createServer((anfrage, antwort) => {
    if (!(anfrage.headers.cookie ?? "").includes("box_anmeldung=1")) {
      ohneCookie.push(anfrage.url ?? "");
      antwort.writeHead(200, { "content-type": "text/html" }).end("<!doctype html><title>Anmelden</title>");
      return;
    }
    const weiter = http.request(
      { host: ziel.hostname, port: ziel.port, path: anfrage.url, method: anfrage.method, headers: anfrage.headers },
      (r) => { antwort.writeHead(r.statusCode ?? 502, r.headers); r.pipe(antwort); },
    );
    weiter.on("error", () => antwort.writeHead(502).end());
    anfrage.pipe(weiter);
  });
  await new Promise<void>((ok) => tor.listen(0, "127.0.0.1", ok));
  const port = (tor.address() as AddressInfo).port;
  try {
    await context.addCookies([{ name: "box_anmeldung", value: "1", domain: "127.0.0.1", path: "/" }]);
    await context.addInitScript(() => {
      try { localStorage.setItem("relay_onboarding_done", "1"); } catch { /* sandboxed frame */ }
    });
    await page.goto(`http://127.0.0.1:${port}/`);
    await page.evaluate(() => navigator.serviceWorker.ready);
    const cdp = await context.newCDPSession(page);
    const { errors, data } = await cdp.send("Page.getAppManifest");
    expect(errors, "Manifest ohne Fehler").toEqual([]);
    expect(JSON.parse(data ?? "{}").name, "das echte Manifest, nicht die Anmeldeseite").toBe("Relay");
    const { installabilityErrors } = await cdp.send("Page.getInstallabilityErrors");
    // Playwright's contexts are private windows, which Chrome never installs from.
    const fehler = installabilityErrors.map((f) => f.errorId).filter((id) => id !== "in-incognito");
    expect(fehler, "Chrome bietet „Installieren“ an").toEqual([]);
    expect(ohneCookie, "keine Anfrage ohne Anmelde-Cookie").toEqual([]);
  } finally {
    tor.close();
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
  // The icon iOS takes is the one embedded in the page as delivered.
  const touch = await page.evaluate(() => document.querySelector('link[rel="apple-touch-icon"]')?.getAttribute("href") ?? "");
  expect(await abstand(touch.replace(/^data:image\/png;base64,/, ""), form("voll", 180), 180), "iOS-Icon zeigt nicht voll").toBeLessThan(4);
  // The check tells pictures apart: the tile is not the sign of the tab.
  const kachel = readFileSync(join(WEB, "static/icon.png")).toString("base64");
  expect(await abstand(kachel, form("tab", 512), 512), "Gegenprobe").toBeGreaterThan(20);
});

// The installed app (Kai, 7.10.2026): the service worker runs without push
// switched on, and Relay opens without a network from its shell.
test("PWA: Service Worker ohne Push, Start ohne Netz", async ({ page, context }, info) => {
  test.skip(info.project.name !== "desktop", "einmal genügt");
  await context.addInitScript(() => {
    try { localStorage.setItem("relay_onboarding_done", "1"); } catch { /* sandboxed frame */ }
  });
  await page.goto("/");
  const aktiv = await page.evaluate(async () => {
    const reg = await navigator.serviceWorker.ready;
    return !!reg.active;
  });
  expect(aktiv, "Service Worker aktiv").toBe(true);
  // A second load under the worker puts the shell into its cache.
  // (The shell, or the onboarding where the server has no account yet.)
  await page.reload();
  await page.getByRole("button", { name: "Assistent öffnen" }).waitFor();
  await page.waitForFunction(async () => {
    const namen = await caches.keys();
    if (!namen.length) return false;
    const cache = await caches.open(namen[0]);
    const pfade = (await cache.keys()).map((r) => new URL(r.url).pathname);
    const geladen = performance.getEntriesByType("resource")
      .map((e) => new URL(e.name).pathname)
      .filter((p) => p.startsWith("/_app/"));
    return pfade.includes("/") && geladen.length > 0 && geladen.every((p) => pfade.includes(p));
  });
  await context.setOffline(true);
  try {
    await page.reload();
    await expect(page.locator(".kopfleiste"), "Hülle ohne Netz").toBeVisible();
  } finally {
    await context.setOffline(false);
  }
});

// The app icon's shortcut "Neue E-Mail" (manifest /?neu=1) opens the
// compose window and leaves a clean address.
test("PWA: Abkürzung Neue E-Mail", async ({ page, context }, info) => {
  test.skip(info.project.name !== "desktop", "einmal genügt");
  test.skip(!DATEN, "braucht ein Konto, sonst kommt die Einrichtung");
  await context.addInitScript(() => {
    try { localStorage.setItem("relay_onboarding_done", "1"); } catch { /* sandboxed frame */ }
  });
  await page.goto("/?neu=1");
  await expect(page.getByRole("button", { name: "Schließen" }).first()).toBeVisible();
  await expect(page).toHaveURL(/\/$/);
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
  await page.locator(".mail-kopf").getByRole("button", { name: "Mehr" }).click();
  await page.getByRole("menuitem", { name: "Archivieren", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("2 Mails ins Archiv verschoben");
  await expect(zeilen).toHaveCount(vorher - 2);
  await page.getByRole("status").getByRole("button", { name: "Rückgängig" }).click();
  await expect(zeilen).toHaveCount(vorher);
});

// Signs by place (Kai, 9.10.2026): the reading pane only for the content;
// the list head keeps "only flagged", "Neue E-Mail" and "Mehr", the rest
// for the marked mails stands under "Mehr".
test("Mail: Symbole nach Ort", async ({ page, context }, info) => {
  test.skip(!DATEN, "braucht die Beispieldaten");
  test.skip(info.project.name !== "desktop", "Lesebereich neben der Liste");
  await context.addInitScript(() => {
    try { localStorage.setItem("relay_onboarding_done", "1"); } catch { /* sandboxed frame */ }
  });
  await page.goto("/");
  const kopf = page.locator(".mail-kopf .btn-reihe");
  const zeilen = page.locator(".message-item");
  await zeilen.first().waitFor();
  await expect(kopf.getByRole("button")).toHaveCount(3);
  await expect(kopf.getByRole("button", { name: "Neue E-Mail" })).toBeVisible();

  await zeilen.first().click();
  const lese = page.locator(".werkzeugleiste");
  await expect(lese.getByRole("button", { name: "Antworten", exact: true })).toBeVisible();
  for (const weg of ["Archivieren", "Verschieben"]) {
    await expect(lese.getByRole("button", { name: weg })).toHaveCount(0);
  }
  expect(await lese.getByRole("button").count()).toBeLessThanOrEqual(5);
  await expect(kopf.getByRole("button")).toHaveCount(3);
  await kopf.getByRole("button", { name: "Mehr" }).click();
  for (const eintrag of ["Archivieren", "In Ordner verschieben", /^Als (un)?gelesen markieren$/, "Nur ungelesene", "Aktualisieren"]) {
    await expect(page.getByRole("menuitem", { name: eintrag })).toBeVisible();
  }
  await page.keyboard.press("Escape");
});

// Only unread (Kai, 9.10.2026): the count at the account shows only the
// unread mails of the inbox, clicked again all of them.
test("Mail: Klick auf die Zahl zeigt nur ungelesene", async ({ page, context }, info) => {
  test.skip(!DATEN, "braucht die Beispieldaten");
  test.skip(info.project.name !== "desktop", "Ordnerspalte sichtbar");
  await context.addInitScript(() => {
    try { localStorage.setItem("relay_onboarding_done", "1"); } catch { /* sandboxed frame */ }
  });
  await page.goto("/");
  const zeilen = page.locator(".message-item");
  await zeilen.first().waitFor();
  const alle = await zeilen.count();
  const zahl = page.locator(".unread-badge").first();
  await expect(zahl).toBeVisible();
  const ungelesen = Number(await zahl.textContent());
  expect(ungelesen).toBeLessThan(alle);

  await zahl.click();
  const titel = page.locator(".mail-kopf h1");
  await expect(titel).toHaveText("Posteingang – ungelesen");
  await expect(zeilen).toHaveCount(ungelesen);
  await expect(zeilen.locator(".ungelesen-punkt")).toHaveCount(ungelesen);

  await zahl.click();
  await expect(titel).toHaveText("Posteingang");
  await expect(zeilen).toHaveCount(alle);
});

// Contacts in three columns (Kai, 9.10.2026): the address book first, the
// senders collected from mail apart (one who is in the address book is not
// shown twice), the chosen contact with his last mails.
test("Kontakte: drei Spalten, Herkunft und letzte Mails", async ({ page, context }, info) => {
  test.skip(!DATEN, "braucht die Beispieldaten");
  test.skip(info.project.name !== "desktop", "drei Spalten nebeneinander");
  await context.addInitScript(() => {
    try { localStorage.setItem("relay_onboarding_done", "1"); } catch { /* sandboxed frame */ }
  });
  await page.goto("/contacts");
  const titel = page.locator(".ct-kopf h1");
  await expect(titel).toHaveText("Mein Adressbuch");
  const zeilen = page.locator(".ct-item");
  await expect(zeilen).toHaveCount(8);

  await zeilen.filter({ hasText: "Jonas Weber" }).click();
  const detail = page.locator(".ct-detail");
  await expect(detail.getByRole("heading", { level: 2 })).toHaveText("Jonas Weber");
  await expect(detail.getByRole("button", { name: "E-Mail schreiben" })).toBeVisible();
  await expect(detail.locator(".ct-mail-liste")).toContainText("Angebot Messestand Frühjahr");

  await page.locator(".ct-quelle", { hasText: "Aus E-Mails gesammelt" }).click();
  await expect(titel).toHaveText("Aus E-Mails gesammelt");
  await expect(zeilen.filter({ hasText: "Jonas Weber" }), "nicht doppelt").toHaveCount(0);
  await zeilen.first().click();
  await expect(detail.getByRole("button", { name: "Ins Adressbuch übernehmen" })).toBeVisible();
});

// "Ähnliche E-Mails" (Kai, 7.10.2026): from the context menu to all mails
// of the same sender, the same domain or a similar subject in the open
// folder, then all of them at once into the trash and back. Jonas Weber
// wrote twice to the inbox and once to the archive; beispiel.de is three
// mails in the inbox.
test("Ähnliche E-Mails: Absender, Domain, Betreff", async ({ page, context }, info) => {
  test.skip(!DATEN, "braucht die Beispieldaten");
  test.skip(info.project.name !== "desktop", "Rechtsklick");
  await context.addInitScript(() => {
    try { localStorage.setItem("relay_onboarding_done", "1"); } catch { /* sandboxed frame */ }
  });
  await page.goto("/");
  const zeilen = page.locator(".message-item");
  await zeilen.first().waitFor();
  const alle = await zeilen.count();
  const jonas = zeilen.filter({ hasText: "Angebot Messestand Frühjahr" });
  const aehnlich = async (weg: string) => {
    await jonas.click({ button: "right" });
    await page.getByRole("menuitem", { name: "Ähnliche E-Mails" }).click();
    await page.getByRole("menuitem", { name: weg }).click();
  };
  const leiste = page.locator(".aehnlich-leiste");

  await aehnlich("Von jonas.weber@beispiel.de");
  await expect(leiste).toContainText("2 ähnliche E-Mails in diesem Ordner");
  await expect(zeilen).toHaveCount(2);
  await expect(zeilen.filter({ hasText: "Vertrag Messe 2025" }), "nicht aus dem Archiv").toHaveCount(0);

  await page.getByRole("button", { name: "Alle auswählen" }).click();
  await page.locator(".mail-kopf").getByRole("button", { name: "Mehr" }).click();
  await page.getByRole("menuitem", { name: /Papierkorb/ }).click();
  await expect(zeilen).toHaveCount(0);
  await page.getByRole("status").getByRole("button", { name: "Rückgängig" }).click();
  await expect(zeilen).toHaveCount(2);

  await page.keyboard.press("Escape");
  await page.locator(".kopfleiste input[type=search], .kopfleiste input").first().fill("");
  await expect(zeilen).toHaveCount(alle);
  await aehnlich("Alle von @beispiel.de");
  await expect(leiste).toContainText("3 ähnliche E-Mails in diesem Ordner");

  await page.locator(".kopfleiste input[type=search], .kopfleiste input").first().fill("");
  await expect(zeilen).toHaveCount(alle);
  await aehnlich("Betreff „Angebot Messestand Frühjahr …“");
  await expect(leiste).toContainText("1 ähnliche E-Mail in diesem Ordner");
  await expect(page.locator(".kopfleiste input").first()).toHaveValue('betreff:"Angebot Messestand Frühjahr"');
});

// The search stays in the open folder (Kai, 7.10.2026): the archived
// "Vertrag Messe 2025" has the same number as a mail in the inbox, and
// every action works on the open folder — from the inbox it would have hit
// the wrong mail. The head names the folder searched.
test("Suche bleibt im geöffneten Ordner", async ({ page, context }, info) => {
  test.skip(!DATEN, "braucht die Beispieldaten");
  test.skip(info.project.name !== "desktop", "einmal genügt");
  await context.addInitScript(() => {
    try { localStorage.setItem("relay_onboarding_done", "1"); } catch { /* sandboxed frame */ }
  });
  await page.goto("/");
  const zeilen = page.locator(".message-item");
  await zeilen.first().waitFor();
  const suche = page.locator(".kopfleiste input").first();
  await suche.fill("Vertrag");
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Suche in Posteingang");
  await expect(zeilen).toHaveCount(0);
  await suche.fill("");
  await page.locator(".mail-spalte").getByText("Archiv", { exact: true }).click();
  await expect(zeilen.filter({ hasText: "Vertrag Messe 2025" })).toHaveCount(1);
  await suche.fill("Vertrag");
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Suche in Archiv");
  await expect(zeilen).toHaveCount(1);
});

// The calendar on the phone (Kai, 7.10.2026: "GUI nicht gut"): the head
// in two short lines, refresh and import behind "Mehr", the weekday row
// slim, the month as dots and the chosen day's events in a list below.
test("Kalender am Handy: kompakter Kopf, Punkte, Tagesliste", async ({ page, context }, info) => {
  test.skip(info.project.name !== "handy", "nur am Handy");
  await context.addInitScript(() => {
    try { localStorage.setItem("relay_onboarding_done", "1"); } catch { /* sandboxed frame */ }
  });
  await page.goto("/calendar");
  const liste = page.locator(".cal-tagesliste");
  await expect(liste).toBeVisible();
  const kopf = await page.locator(".cal-grid-head-cell").first().boundingBox();
  expect(kopf!.height, "Wochentagszeile schmal").toBeLessThan(40);
  await expect(page.getByRole("button", { name: "Neuer Termin" })).toBeVisible();
  await expect(page.getByRole("button", { name: ".ics importieren" })).toHaveCount(0);
  await page.getByRole("button", { name: "Mehr" }).click();
  await expect(page.getByRole("menuitem", { name: ".ics importieren" })).toBeVisible();
  await page.keyboard.press("Escape");
  // The whole month above the fold: the last week's row ends on screen.
  const letzte = await page.locator(".cal-cell").last().boundingBox();
  expect(letzte!.y + letzte!.height).toBeLessThanOrEqual(page.viewportSize()!.height);
  // A tap on another day lists that day.
  await page.locator(".cal-cell:not(.other-month)").nth(14).click();
  await expect(liste.getByRole("heading", { level: 2 })).toContainText("15.");
  if (DATEN) await expect(page.locator(".cal-punkt").first()).toBeVisible();
});

// Backspace deletes and moves on (Kai, 7.10.2026): the next mail opens at
// once, "Rückgängig" brings the deleted one back. Undone, so the dummy
// data stays the same.
test("Backspace löscht und öffnet die nächste Mail", async ({ page, context }, info) => {
  test.skip(!DATEN, "braucht die Beispieldaten");
  test.skip(info.project.name !== "desktop", "Tastatur");
  await context.addInitScript(() => {
    try { localStorage.setItem("relay_onboarding_done", "1"); } catch { /* sandboxed frame */ }
  });
  await page.goto("/");
  const zeilen = page.locator(".message-item");
  await zeilen.first().waitFor();
  const vorher = await zeilen.count();
  await zeilen.nth(1).click();
  const naechste = (await zeilen.nth(2).locator(".sender").innerText()).trim();
  await page.locator(".message-list").focus();
  await page.keyboard.press("Backspace");
  await expect(page.getByRole("status")).toContainText("Mail in den Papierkorb verschoben");
  await expect(zeilen).toHaveCount(vorher - 1);
  await expect(page.locator(".preview-from-name")).toHaveText(naechste);
  await expect(page.getByRole("alertdialog"), "keine Rückfrage").toHaveCount(0);
  await page.getByRole("status").getByRole("button", { name: "Rückgängig" }).click();
  await expect(zeilen).toHaveCount(vorher);
});

// Shift + arrow selects a run of mails (Kai, 7.10.2026); Backspace takes
// them all, "Rückgängig" brings them all back.
test("Shift und Pfeiltasten wählen mehrere Mails", async ({ page, context }, info) => {
  test.skip(!DATEN, "braucht die Beispieldaten");
  test.skip(info.project.name !== "desktop", "Tastatur");
  await context.addInitScript(() => {
    try { localStorage.setItem("relay_onboarding_done", "1"); } catch { /* sandboxed frame */ }
  });
  await page.goto("/");
  const zeilen = page.locator(".message-item");
  await zeilen.first().waitFor();
  const vorher = await zeilen.count();
  await zeilen.nth(1).click();
  await page.locator(".message-list").focus();
  await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("Shift+ArrowDown");
  await expect(page.locator(".mail-kopf h1")).toHaveText("3 ausgewählt");
  await page.keyboard.press("Shift+ArrowUp");
  await expect(page.locator(".mail-kopf h1")).toHaveText("2 ausgewählt");
  await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("Backspace");
  await expect(page.getByRole("status")).toContainText("3 Mails in den Papierkorb verschoben");
  await expect(zeilen).toHaveCount(vorher - 3);
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

    // "Nur Abos" (Kai, 9.10.2026): only senders that offer it.
    const zeilen = page.locator(".aufraeumen-zeile");
    const alle = await zeilen.count();
    await page.getByLabel("Nur Abos").check();
    await expect(zeilen.filter({ hasText: "stadtwerke.example" })).toHaveCount(1);
    const abos = await zeilen.count();
    expect(abos).toBeLessThan(alle);
    await expect(zeilen.getByRole("button", { name: "Abo beenden" })).toHaveCount(abos);
    await page.getByLabel("Nur Abos").uncheck();
    await expect(zeilen).toHaveCount(alle);

    // Unsubscribe and clean up in one step, taken back after.
    await page.locator(".aufraeumen-zeile", { hasText: "stadtwerke.example" })
      .getByRole("button", { name: "Abo beenden" }).click();
    const dialog = page.getByRole("alertdialog");
    const danach = dialog.getByRole("checkbox", { name: /in den Papierkorb/ });
    await expect(danach).toBeChecked();
    await dialog.getByRole("button", { name: "Abo beenden" }).click();
    await expect(page.locator(".aufraeumen-zeile", { hasText: "stadtwerke.example" })).toHaveCount(0);
    // Both notes stand, one above the other: "Abo beendet" and the move.
    await expect(page.getByRole("status").filter({ hasText: "abmelden@stadtwerke.example" })).toBeVisible();
    await page.getByRole("status").getByRole("button", { name: "Rückgängig" }).click();
    await expect(page.locator(".aufraeumen-zeile", { hasText: "stadtwerke.example" })).toBeVisible();

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
    // The note of the unsubscribe above may still stand beside it.
    const archiviert = page.getByRole("status").filter({ hasText: "Mail ins Archiv verschoben" });
    await expect(archiviert).toBeVisible();
    await archiviert.getByRole("button", { name: "Rückgängig" }).click();
    await page.getByRole("button", { name: "Beenden" }).click();
    await expect(page.getByRole("heading", { level: 1, name: "Aufräumen" })).toBeVisible();
  } finally {
    await request.post("/api/v1/settings/aufraeumen", { data: false });
  }
});

// Closing the tab within the five seconds of "Rückgängig" (Kai, 7.10.2026):
// the delete was lost, the mail stayed. Now it goes out as the page goes.
// Put back afterwards, so the dummy data stay the same.
test("Löschen übersteht das Schließen des Tabs", async ({ page, context, request }, info) => {
  test.skip(!DATEN, "braucht die Beispieldaten");
  test.skip(info.project.name !== "desktop", "einmal genügt");
  await context.addInitScript(() => {
    try { localStorage.setItem("relay_onboarding_done", "1"); } catch { /* sandboxed frame */ }
  });
  const betreff = "Wartung am Wochenende";
  const finden = async (ordner: string) => {
    const r = await request.get(`/api/v1/messages?account_id=1&folder=${ordner}&limit=500&list_only=true`);
    return ((await r.json()) as { uid: number; subject?: string }[]).find((m) => m.subject === betreff);
  };
  await page.goto("/");
  const zeile = page.locator(".message-item").filter({ hasText: betreff });
  await zeile.click();
  await page.locator(".message-list").focus();
  await page.keyboard.press("Backspace");
  await expect(page.getByRole("status")).toContainText("Mail in den Papierkorb verschoben");
  await page.close({ runBeforeUnload: true });
  await expect.poll(async () => !!(await finden("Trash")), { message: "in Relays Papierkorb", timeout: 10_000 }).toBe(true);
  const mail = (await finden("Trash"))!;
  await request.post("/api/v1/messages/move", {
    data: { account_id: 1, uid: mail.uid, source_folder: "Trash", target_folder: "INBOX", raw_source_folder: "", raw_target_folder: "" },
  });
  await expect.poll(async () => !!(await finden("INBOX")), { timeout: 10_000 }).toBe(true);
});
