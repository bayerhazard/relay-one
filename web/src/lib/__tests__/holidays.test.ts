import { describe, it, expect } from "vitest";
import { easter, germanHolidays } from "$lib/holidays";

function isoLocal(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

describe("easter", () => {
  it("computes known Easter Sundays", () => {
    // Well-known Easter dates (local calendar date).
    expect(isoLocal(easter(2000))).toBe("2000-04-23");
    expect(isoLocal(easter(2024))).toBe("2024-03-31");
    expect(isoLocal(easter(2025))).toBe("2025-04-20");
    expect(isoLocal(easter(2026))).toBe("2026-04-05");
  });
});

describe("germanHolidays", () => {
  it("returns the fixed nationwide holidays", () => {
    const h = germanHolidays(2026);
    const byDate = new Map(h.map((x) => [x.date, x.name]));
    expect(byDate.get("2026-01-01")).toBe("Neujahr");
    expect(byDate.get("2026-05-01")).toBe("Tag der Arbeit");
    expect(byDate.get("2026-10-03")).toBe("Tag der Deutschen Einheit");
    expect(byDate.get("2026-12-25")).toBe("1. Weihnachtstag");
    expect(byDate.get("2026-12-26")).toBe("2. Weihnachtstag");
  });

  it("derives movable feasts from Easter (2026-04-05)", () => {
    const h = germanHolidays(2026);
    const byDate = new Map(h.map((x) => [x.date, x.name]));
    expect(byDate.get("2026-04-03")).toBe("Karfreitag"); // Easter - 2
    expect(byDate.get("2026-04-06")).toBe("Ostermontag"); // Easter + 1
    expect(byDate.get("2026-05-14")).toBe("Christi Himmelfahrt"); // Easter + 39
    expect(byDate.get("2026-05-25")).toBe("Pfingstmontag"); // Easter + 50
  });

  it("has no duplicate dates", () => {
    const h = germanHolidays(2026);
    const dates = h.map((x) => x.date);
    expect(new Set(dates).size).toBe(dates.length);
  });
});
