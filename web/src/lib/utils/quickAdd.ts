// Client-side Quick-Add parser — mirrors `server/src/api/quick_add.rs`.
//
// The authoritative parse happens on the server (`POST /todos/quick-add`); this
// copy exists purely so the Quick-Add bar can show live chips while the user
// types. Keep the recognised tokens in sync with the Rust implementation.

export interface ParsedQuickAdd {
  title: string;
  due: Date | null;
  dueHasTime: boolean;
  /** 1 = highest … 4 = none (Todoist UI scale). */
  priority: number;
  project: string | null;
  labels: string[];
  rrule: string | null;
}

export const PRIO_LABEL: Record<number, string> = { 1: "P1", 2: "P2", 3: "P3", 4: "P4" };

const WEEKDAYS: Record<string, number> = {
  montag: 1, monday: 1, mo: 1, mon: 1,
  dienstag: 2, tuesday: 2, di: 2, tue: 2,
  mittwoch: 3, wednesday: 3, mi: 3, wed: 3,
  donnerstag: 4, thursday: 4, do: 4, thu: 4,
  freitag: 5, friday: 5, fr: 5, fri: 5,
  samstag: 6, saturday: 6, sa: 6, sat: 6,
  sonntag: 7, sunday: 7, so: 7, sun: 7,
};

function weekdayNum(word: string): number | null {
  const w = word.toLowerCase().replace(/s$/, "");
  return WEEKDAYS[w] ?? null;
}

function nextWeekday(now: Date, wd: number): Date {
  const d = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const cur = ((d.getDay() + 6) % 7) + 1; // Mon=1 … Sun=7
  let delta = wd - cur;
  if (delta < 0) delta += 7;
  d.setDate(d.getDate() + delta);
  return d;
}

function freqForUnit(unit: string): string | null {
  const u = unit.replace(/n$/, "");
  if (["tag", "tage", "day", "days"].includes(u)) return "DAILY";
  if (["woche", "wochen", "week", "weeks"].includes(u)) return "WEEKLY";
  if (["monat", "monate", "month", "months"].includes(u)) return "MONTHLY";
  if (["jahr", "jahre", "year", "years"].includes(u)) return "YEARLY";
  return null;
}

function icalWeekday(wd: number): string {
  return ["MO", "TU", "WE", "TH", "FR", "SA", "SU"][wd - 1] ?? "MO";
}

function parseHm(s: string): { h: number; m: number } | null {
  const m = /^(\d{1,2}):(\d{2})$/.exec(s);
  if (!m) return null;
  const h = Number(m[1]);
  const mm = Number(m[2]);
  if (h > 23 || mm > 59) return null;
  return { h, m: mm };
}

function parseHourOnly(s: string): number | null {
  const n = /^(\d{1,2})$/.exec(s);
  if (!n) return null;
  const h = Number(n[1]);
  return h <= 23 ? h : null;
}

function parseGermanDate(tok: string, defaultYear: number): Date | null {
  const parts = tok.replace(/\.$/, "").split(".");
  if (parts.length < 2 || parts.length > 3) return null;
  const day = Number(parts[0]);
  const month = Number(parts[1]);
  const year = parts.length === 3 ? Number(parts[2]) : defaultYear;
  if (!day || !month || !year) return null;
  const d = new Date(year, month - 1, day);
  return isNaN(d.getTime()) ? null : d;
}

/** Parse one Quick-Add line. Never throws; unrecognised text stays in `title`. */
export function parseQuickAdd(input: string, now: Date = new Date()): ParsedQuickAdd {
  const out: ParsedQuickAdd = {
    title: "",
    due: null,
    dueHasTime: false,
    priority: 4,
    project: null,
    labels: [],
    rrule: null,
  };
  const leftover: string[] = [];
  const tokens = input.split(/\s+/).filter(Boolean);
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());

  let i = 0;
  while (i < tokens.length) {
    const tok = tokens[i];
    const lower = tok.toLowerCase();

    // Project / Label / Section
    if (tok.length > 1 && tok.startsWith("#")) { out.project = tok.slice(1); i++; continue; }
    if (tok.length > 1 && (tok.startsWith("@") || tok.startsWith("%"))) {
      const label = tok.slice(1);
      if (!out.labels.includes(label)) out.labels.push(label);
      i++; continue;
    }

    // Priority
    if (/^p[1-4]$/.test(lower)) { out.priority = Number(lower[1]); i++; continue; }

    // Recurrence
    const rec = parseRecurrence(tokens, i);
    if (rec) {
      out.rrule = rec.rrule;
      if (rec.weekday) out.due = nextWeekday(now, rec.weekday);
      i += rec.consumed;
      continue;
    }

    // Time
    const tm = parseTime(tokens, i);
    if (tm) {
      const base = out.due ?? today;
      out.due = new Date(base.getFullYear(), base.getMonth(), base.getDate(), tm.h, tm.m);
      out.dueHasTime = true;
      i += tm.consumed;
      continue;
    }

    // Date
    const dt = parseDate(tokens, i, today);
    if (dt) {
      const base = out.due ?? today;
      const hh = out.dueHasTime ? base.getHours() : 0;
      const mm = out.dueHasTime ? base.getMinutes() : 0;
      out.due = new Date(dt.date.getFullYear(), dt.date.getMonth(), dt.date.getDate(), hh, mm);
      i += dt.consumed;
      continue;
    }

    leftover.push(tok);
    i++;
  }

  out.title = leftover.join(" ").trim();
  return out;
}

function parseRecurrence(
  tokens: string[],
  i: number,
): { consumed: number; rrule: string; weekday: number | null } | null {
  const w = (n: number) => tokens[i + n]?.toLowerCase();
  const t0 = w(0);
  if (!t0) return null;

  if (t0 === "alle" || t0 === "every") {
    const n = Number(w(1));
    if (Number.isFinite(n) && n > 0) {
      const freq = freqForUnit(w(2) ?? "");
      if (freq) return { consumed: 3, rrule: `FREQ=${freq};INTERVAL=${n}`, weekday: null };
    }
  }
  if (["jeden", "jede", "jedes", "every"].includes(t0)) {
    const unit = w(1) ?? "";
    const wd = weekdayNum(unit);
    if (wd) return { consumed: 2, rrule: `FREQ=WEEKLY;BYDAY=${icalWeekday(wd)}`, weekday: wd };
    const freq = freqForUnit(unit);
    if (freq) return { consumed: 2, rrule: `FREQ=${freq}`, weekday: null };
  }
  if (["täglich", "taeglich", "daily"].includes(t0)) return { consumed: 1, rrule: "FREQ=DAILY", weekday: null };
  if (["wöchentlich", "woechentlich", "weekly"].includes(t0)) return { consumed: 1, rrule: "FREQ=WEEKLY", weekday: null };
  if (["monatlich", "monthly"].includes(t0)) return { consumed: 1, rrule: "FREQ=MONTHLY", weekday: null };
  if (["jährlich", "jaehrlich", "yearly", "annually"].includes(t0)) return { consumed: 1, rrule: "FREQ=YEARLY", weekday: null };
  return null;
}

function parseTime(tokens: string[], i: number): { consumed: number; h: number; m: number } | null {
  const t0 = tokens[i];
  const hm = parseHm(t0);
  if (hm) return { consumed: 1, ...hm };

  const lower = t0.toLowerCase();
  if (lower === "um" || lower === "at") {
    const t1 = tokens[i + 1];
    if (t1) {
      const hm2 = parseHm(t1);
      if (hm2) return { consumed: 2, ...hm2 };
      const h = parseHourOnly(t1);
      if (h !== null) return { consumed: 2, h, m: 0 };
    }
  }
  const t1 = tokens[i + 1]?.toLowerCase();
  if (t1 === "uhr" || t1 === "h") {
    const h = parseHourOnly(lower);
    if (h !== null) return { consumed: 2, h, m: 0 };
  }
  if (lower.endsWith("h") && lower.length > 1) {
    const h = parseHourOnly(lower.slice(0, -1));
    if (h !== null) return { consumed: 1, h, m: 0 };
  }
  return null;
}

function parseDate(tokens: string[], i: number, today: Date): { consumed: number; date: Date } | null {
  const lower = tokens[i].toLowerCase();

  const rel: Record<string, number> = {
    heute: 0, today: 0, morgen: 1, tomorrow: 1, "übermorgen": 2, uebermorgen: 2,
  };
  if (lower in rel) {
    const d = new Date(today);
    d.setDate(d.getDate() + rel[lower]);
    return { consumed: 1, date: d };
  }
  const wd = weekdayNum(lower);
  if (wd) return { consumed: 1, date: nextWeekday(today, wd) };

  if (["nächste", "naechste", "next"].includes(lower)) {
    const unit = tokens[i + 1]?.toLowerCase();
    if (unit === "woche" || unit === "week") {
      const d = new Date(today);
      d.setDate(d.getDate() + 7);
      return { consumed: 2, date: d };
    }
  }
  if (lower === "in") {
    const n = Number(tokens[i + 1]);
    if (Number.isFinite(n)) {
      const d = new Date(today);
      d.setDate(d.getDate() + n);
      return { consumed: tokens[i + 2] ? 3 : 2, date: d };
    }
  }
  if (lower === "am" || lower === "on") {
    const t1 = tokens[i + 1];
    if (t1) {
      const d = parseGermanDate(t1, today.getFullYear());
      if (d) return { consumed: 2, date: d };
    }
  }
  const iso = /^(\d{4})-(\d{2})-(\d{2})$/.exec(tokens[i]);
  if (iso) {
    const d = new Date(Number(iso[1]), Number(iso[2]) - 1, Number(iso[3]));
    if (!isNaN(d.getTime())) return { consumed: 1, date: d };
  }
  const gd = parseGermanDate(tokens[i], today.getFullYear());
  if (gd) return { consumed: 1, date: gd };
  return null;
}
