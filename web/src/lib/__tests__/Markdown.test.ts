import { describe, expect, it } from "vitest";
import { renderMarkdown } from "../utils/markdown";

describe("renderMarkdown", () => {
  it("rendert Überschriften bis h4", () => {
    const html = renderMarkdown("# Titel\n\n## Kernthemen\n\n#### Tiefe");
    expect(html).toContain("<h1>Titel</h1>");
    expect(html).toContain("<h2>Kernthemen</h2>");
    expect(html).toContain("<h4>Tiefe</h4>");
  });

  it("rendert Fett, Kursiv und Code", () => {
    const html = renderMarkdown("Das ist **fett**, *kursiv* und `code`.");
    expect(html).toContain("<strong>fett</strong>");
    expect(html).toContain("<em>kursiv</em>");
    expect(html).toContain("<code>code</code>");
  });

  it("rendert Aufzählungs- und nummerierte Listen", () => {
    const html = renderMarkdown("- eins\n- zwei\n\n1. erst\n2. zweit");
    expect(html).toContain("<ul>");
    expect(html).toContain("<li>eins</li>");
    expect(html).toContain("<ol>");
    expect(html).toContain("<li>erst</li>");
  });

  it("rendert GFM-Checklisten als readonly-Checkboxen", () => {
    const html = renderMarkdown("- [ ] Offen\n- [x] Erledigt");
    expect(html).toContain('class="md-checklist"');
    expect(html).toContain('<input type="checkbox" disabled');
    expect(html).toContain("checked");
    expect(html).toContain("Erledigt");
  });

  it("erlaubt nur http/https-Links", () => {
    const ok = renderMarkdown("[Seite](https://example.de)");
    expect(ok).toContain('href="https://example.de"');
    expect(ok).toContain('rel="noopener noreferrer"');
    // Nicht-HTTP-Schemata werden KEIN Link — bleiben harmloser Text.
    const bad = renderMarkdown("[X](javascript:alert(1))");
    expect(bad).not.toContain('<a href="javascript:');
  });

  it("kapselt HTML im Inhalt (XSS)", () => {
    const html = renderMarkdown("Halo <script>alert(1)</script> Welt");
    expect(html).not.toContain("<script>");
    expect(html).toContain("Halo");
  });

  it("läuft mit leerem Input", () => {
    expect(renderMarkdown("")).toBe("");
  });

  it("verbindet fortgesetzte Zeilen zu einem Absatz", () => {
    const html = renderMarkdown("Zeile eins\nZeile zwei");
    expect(html).toContain("<p>Zeile eins Zeile zwei</p>");
  });
});
