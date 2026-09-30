import { describe, it, expect } from "vitest";
import { parseQuickAdd } from "$lib/utils/quickAdd";

// Fixed reference: Wednesday, 2026-09-16, 10:00 local.
const NOW = new Date(2026, 8, 16, 10, 0, 0);

function p(s: string) {
  return parseQuickAdd(s, NOW);
}

describe("parseQuickAdd", () => {
  it("keeps a plain title", () => {
    const r = p("Budget prüfen");
    expect(r.title).toBe("Budget prüfen");
    expect(r.due).toBeNull();
    expect(r.priority).toBeNull();
  });

  it("extracts project and label", () => {
    const r = p("Angebot senden #Firma @dringend");
    expect(r.title).toBe("Angebot senden");
    expect(r.project).toBe("Firma");
    expect(r.labels).toEqual(["dringend"]);
  });

  it("accepts % as a label prefix", () => {
    expect(p("Test %wichtig").labels).toEqual(["wichtig"]);
  });

  it("parses priority p1..p5", () => {
    expect(p("A p1").priority).toBe(1);
    expect(p("A p3").priority).toBe(3);
    expect(p("A p5").priority).toBe(5);
  });

  it("resolves relative dates", () => {
    const heute = p("Heute anrufen").due!;
    expect([heute.getFullYear(), heute.getMonth(), heute.getDate()]).toEqual([2026, 8, 16]);
    const morgen = p("Morgen anrufen").due!;
    expect([morgen.getFullYear(), morgen.getMonth(), morgen.getDate()]).toEqual([2026, 8, 17]);
  });

  it("resolves a weekday to its next occurrence", () => {
    // Reference is Wednesday; "Freitag" -> 2026-09-18.
    const r = p("Freitag Bericht");
    expect([r.due!.getFullYear(), r.due!.getMonth(), r.due!.getDate()]).toEqual([2026, 8, 18]);
    expect(r.title).toBe("Bericht");
  });

  it("attaches a time and flags dueHasTime", () => {
    const r = p("Morgen um 14:30 Meeting");
    expect(r.dueHasTime).toBe(true);
    expect(r.due!.getHours()).toBe(14);
    expect(r.due!.getMinutes()).toBe(30);
    expect(r.title).toBe("Meeting");
  });

  it("recognises daily recurrence", () => {
    expect(p("Jeden tag Sport").rrule).toBe("FREQ=DAILY");
    expect(p("täglich Sport").rrule).toBe("FREQ=DAILY");
  });

  it("recognises weekly recurrence with weekday", () => {
    const r = p("Jeden Freitag Wochenbericht");
    expect(r.rrule).toBe("FREQ=WEEKLY;BYDAY=FR");
    expect(r.due).not.toBeNull();
    expect(r.title).toBe("Wochenbericht");
  });

  it("recognises an interval", () => {
    expect(p("Alle 2 Wochen putzen").rrule).toBe("FREQ=WEEKLY;INTERVAL=2");
  });

  it("parses an absolute German date", () => {
    const r = p("Am 3.10. Rechnung");
    expect([r.due!.getFullYear(), r.due!.getMonth(), r.due!.getDate()]).toEqual([2026, 9, 3]);
  });

  it("parses an ISO date", () => {
    const r = p("2026-12-24 Geschenke");
    expect([r.due!.getFullYear(), r.due!.getMonth(), r.due!.getDate()]).toEqual([2026, 11, 24]);
  });

  it("handles a full quick-add line", () => {
    const r = p("Freitag Budget prüfen p1 #Firma @dringend");
    expect(r.title).toBe("Budget prüfen");
    expect(r.priority).toBe(1);
    expect(r.project).toBe("Firma");
    expect(r.labels).toEqual(["dringend"]);
    expect(r.due).not.toBeNull();
  });

  it("is safe on empty input", () => {
    const r = p("");
    expect(r.title).toBe("");
    expect(r.due).toBeNull();
  });
});
