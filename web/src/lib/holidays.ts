// German public holidays (nationwide + major movable feasts).
// Computed client-side so no backend round-trip is needed.

export interface Holiday {
  date: string; // YYYY-MM-DD
  name: string;
}

// Gregorian computus (Anonymous of Canterbury) → Easter Sunday.
export function easter(year: number): Date {
  const a = year % 19;
  const b = Math.floor(year / 100);
  const c = year % 100;
  const d = Math.floor(b / 4);
  const e = b % 4;
  const f = Math.floor((b + 8) / 25);
  const g = Math.floor((b - f + 1) / 3);
  const h = (19 * a + b - d - g + 15) % 30;
  const i = Math.floor(c / 4);
  const k = c % 4;
  const l = (32 + 2 * e + 2 * i - h - k) % 7;
  const m = Math.floor((a + 11 * h + 22 * l) / 451);
  const month = Math.floor((h + l - 7 * m + 114) / 31); // 3 = Mar, 4 = Apr
  const day = ((h + l - 7 * m + 114) % 31) + 1;
  return new Date(year, month - 1, day);
}

function iso(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

function addDays(d: Date, offset: number): Date {
  const x = new Date(d);
  x.setDate(x.getDate() + offset);
  return x;
}

// Nationwide German public holidays for a year (movable feasts via Easter).
export function germanHolidays(year: number): Holiday[] {
  const e = easter(year);
  return [
    { date: `${year}-01-01`, name: "Neujahr" },
    { date: iso(addDays(e, -2)), name: "Karfreitag" },
    { date: iso(addDays(e, 1)), name: "Ostermontag" },
    { date: `${year}-05-01`, name: "Tag der Arbeit" },
    { date: iso(addDays(e, 39)), name: "Christi Himmelfahrt" },
    { date: iso(addDays(e, 50)), name: "Pfingstmontag" },
    { date: `${year}-10-03`, name: "Tag der Deutschen Einheit" },
    { date: `${year}-12-25`, name: "1. Weihnachtstag" },
    { date: `${year}-12-26`, name: "2. Weihnachtstag" },
  ];
}
