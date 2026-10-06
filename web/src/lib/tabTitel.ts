// The title in the browser tab: the page first, then the app ("Kontakte ·
// Relay"), after Rocket's and Insilo's tab-titel.tsx (CI HB-SEITENKOPF, G7).
// With several tabs open they tell themselves apart by the page; the paper
// plane in the tab already says which app.

export const ANWENDUNG = "Relay";

export function tabTitel(seite?: string): string {
  const s = seite?.trim();
  return s ? `${s} · ${ANWENDUNG}` : ANWENDUNG;
}
