// Where things stand on the page — Rocket's layout check (frontend/e2e/lage.ts,
// "Lageprüfung"), unchanged in what it checks: text stuck to the edge of the
// content, boxes glued to their predecessor without a gap, the sign of an
// empty state off centre. Runs in the browser via page.evaluate, so no imports.

export function lagePruefen(): string[] {
  const funde: string[] = [];
  const main = document.querySelector("main");
  if (!main) return funde;
  const m = main.getBoundingClientRect();
  const RAND = 12;
  const name = (e: Element) => {
    const k = String((e as HTMLElement).className || "").split(" ").filter(Boolean)[0];
    return `${e.tagName.toLowerCase()}${k ? "." + k : ""}`;
  };
  const sichtbar = (e: Element) => {
    const r = e.getBoundingClientRect();
    const s = getComputedStyle(e);
    return r.width > 0 && r.height > 0 && s.visibility !== "hidden" && s.display !== "none";
  };

  // 1. Text, der am Rand des Inhalts klebt.
  for (const e of main.querySelectorAll("*")) {
    if (!sichtbar(e)) continue;
    const eigenerText = [...e.childNodes].some((n) => n.nodeType === 3 && n.textContent!.trim());
    if (!eigenerText) continue;
    // Was in einem rollenden Bereich steht (Board, breite Tabellen), darf
    // über den Rand hinaus — dafür rollt er. Nur für Vorleser: unsichtbar.
    if (e.closest(".nur-vorleser")) continue;
    let rollt = false;
    for (let a = e.parentElement; a && a !== main; a = a.parentElement) {
      const ox = getComputedStyle(a).overflowX;
      if (ox === "auto" || ox === "scroll" || ox === "hidden") { rollt = true; break; }
    }
    if (rollt) continue;
    const r = e.getBoundingClientRect();
    if (r.left < m.left + RAND || r.right > m.right - RAND + 1) {
      funde.push(`am Rand: ${name(e)} „${e.textContent!.trim().slice(0, 30)}“`);
    }
  }

  // 2. Kästen mit Rand oder Fläche, die ohne Abstand am Vorgänger kleben.
  const kasten = (e: Element) => {
    const s = getComputedStyle(e);
    return parseFloat(s.borderTopWidth) > 0 || (s.backgroundColor !== "rgba(0, 0, 0, 0)" && s.backgroundColor !== "transparent");
  };
  const geschwister = [...main.querySelectorAll("*")].filter((e) => e.parentElement && sichtbar(e) && getComputedStyle(e).position === "static");
  for (const e of geschwister) {
    const vor = e.previousElementSibling;
    if (!vor || !sichtbar(vor) || !kasten(e)) continue;
    const s = getComputedStyle(e.parentElement!);
    if (s.display === "flex" && s.flexDirection.startsWith("row")) continue;
    if (s.display === "grid" || s.display === "inline-flex" || s.display === "table-row") continue;
    if (s.display.startsWith("table")) continue;
    // Zeilen einer Liste stoßen gewollt aneinander, getrennt durch ihren Rand.
    if (e.parentElement!.tagName === "UL" || e.parentElement!.tagName === "OL") continue;
    const abstand = e.getBoundingClientRect().top - vor.getBoundingClientRect().bottom;
    if (abstand >= -1 && abstand < 4 && !kasten(vor)) {
      funde.push(`ohne Abstand: ${name(e)} unter ${name(vor)}`);
    }
  }
  // 3. Zeichen im Leerzustand mittig über dem Text.
  for (const leer of main.querySelectorAll(".leerzustand")) {
    const zeichen = leer.querySelector(":scope > svg");
    if (!zeichen || !sichtbar(zeichen)) continue;
    const a = leer.getBoundingClientRect();
    const z = zeichen.getBoundingClientRect();
    if (Math.abs(z.left + z.width / 2 - (a.left + a.width / 2)) > 2) {
      funde.push("Leerzustand: Zeichen nicht mittig");
    }
  }
  return [...new Set(funde)];
}
