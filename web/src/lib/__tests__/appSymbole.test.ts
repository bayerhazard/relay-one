import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * The app's own icons (Kai, 7.10.2026: "always make sure the PWA icon and
 * the sign in the browser tab are the right ones"; CI R5, G7). The tour
 * compares every PNG with the picture drawn from docs/icon/relay.svg
 * (e2e "Web-App-Symbole zeigen das richtige Bild"); here what needs no
 * browser:
 * - the tab sign is Relay's paper plane from the CI set, nothing else;
 * - the market icon is the same file as the manifest icon;
 * - every manifest icon exists at the size it claims, and app.html links
 *   tab sign, apple-touch icon and manifest.
 */

const WEB = join(__dirname, "..", "..", "..");
const lesen = (p: string) => readFileSync(join(WEB, p));
const pfade = (svg: string) => [...svg.matchAll(/<path d="([^"]+)"/g)].map((m) => m[1]);

/** Width and height from a PNG's IHDR. */
function pngGroesse(datei: Buffer): [number, number] {
  expect(datei.subarray(1, 4).toString()).toBe("PNG");
  return [datei.readUInt32BE(16), datei.readUInt32BE(20)];
}

describe("app icons", () => {
  it("the tab sign is the paper plane of the CI set (senden)", () => {
    const tab = lesen("static/favicon.svg").toString();
    expect(pfade(tab)).toEqual(pfade(lesen("ci/marke/icons/ui/senden.svg").toString()));
    expect(tab, "no tile, no shield in the tab").not.toMatch(/<rect|clip-path/);
  });

  it("the market icon is the manifest icon", () => {
    expect(lesen("../icon.png").equals(lesen("static/icon.png"))).toBe(true);
  });

  it("every manifest icon exists at the size it claims", () => {
    const manifest = JSON.parse(lesen("static/manifest.webmanifest").toString());
    expect(manifest.name).toBe("Relay");
    const zwecke = new Set<string>();
    for (const icon of manifest.icons as { src: string; sizes: string; purpose?: string }[]) {
      const [b, h] = pngGroesse(lesen("static" + icon.src));
      expect(`${b}x${h}`, icon.src).toBe(icon.sizes);
      zwecke.add(icon.purpose ?? "any");
    }
    expect([...zwecke].sort()).toEqual(["any", "maskable"]);
  });

  it("is ready to install: id, 192 and 512 icons, shortcuts, screenshots", () => {
    const manifest = JSON.parse(lesen("static/manifest.webmanifest").toString());
    expect(manifest.id).toBe("/");
    expect(manifest.display).toBe("standalone");
    const groessen = (manifest.icons as { sizes: string; purpose?: string }[])
      .filter((i) => (i.purpose ?? "any") === "any").map((i) => i.sizes);
    expect(groessen).toEqual(expect.arrayContaining(["192x192", "512x512"]));
    expect((manifest.shortcuts as { url: string }[]).map((k) => k.url)).toEqual(["/?neu=1", "/calendar", "/contacts"]);
    const formen = new Set<string>();
    for (const bild of manifest.screenshots as { src: string; sizes: string; form_factor: string }[]) {
      const [b, h] = pngGroesse(lesen("static" + bild.src));
      expect(`${b}x${h}`, bild.src).toBe(bild.sizes);
      formen.add(bild.form_factor);
    }
    expect([...formen].sort()).toEqual(["narrow", "wide"]);
  });

  it("speaks Sie in the push message", () => {
    const sw = lesen("static/sw.js").toString();
    expect(sw).toContain("Sie haben neue Nachrichten");
    expect(sw).not.toMatch(/\bDu hast\b/);
  });

  it("app.html links tab sign, apple-touch icon and manifest", () => {
    const html = lesen("src/app.html").toString();
    expect(html).toContain('rel="icon" href="%sveltekit.assets%/favicon.svg"');
    expect(html).toContain('rel="icon" href="%sveltekit.assets%/favicon-32.png"');
    expect(html).toContain('rel="apple-touch-icon"');
    expect(html).toMatch(/rel="manifest"[^>]*crossorigin="use-credentials"/);
    expect(pngGroesse(lesen("static/apple-touch-icon.png"))).toEqual([180, 180]);
    expect(pngGroesse(lesen("static/favicon-32.png"))).toEqual([32, 32]);
  });
});
