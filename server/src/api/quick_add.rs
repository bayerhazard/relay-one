//! Deterministic Quick-Add parser for to-dos (Todoist-style syntax).
//!
//! Turns a single free-text line into a structured [`ParsedTask`]:
//!
//! ```text
//! "Freitag Budget prüfen p1 #Firma @dringend /Q3"
//!   -> title="Budget prüfen", due=next Friday, priority=1,
//!      project="Firma", labels=["dringend"]
//! ```
//!
//! Supported tokens (case-insensitive, German + English):
//! - Dates: `heute`/`today`, `morgen`/`tomorrow`, `übermorgen`, weekday names,
//!   `nächste woche`, `in N tagen`, `am 3.10.`, `3.10.`, `2026-10-03`, `03.10.2026`
//! - Times: `um 14`, `14:30`, `9 uhr`, `at 14:00`
//! - Recurrence: `jeden tag`, `täglich`, `jede woche`, `wöchentlich`,
//!   `jeden freitag`, `monatlich`, `jeden monat`, `jährlich`, `alle 2 wochen`
//! - Priority: `p1`–`p5` (1 = highest … 5 = lowest)
//! - Project: `#Name`
//! - Label: `@Name` or `%Name`
//! - Section: `/Name`
//!
//! The parser is intentionally local and instant (no AI round-trip). It never
//! fails: unrecognised text stays in the title.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, TimeZone, Utc};

/// Priority on the app's canonical scale (1 = highest … 5 = lowest, `None` = unset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prio {
    P1,
    P2,
    P3,
    P4,
    P5,
    None,
}

impl Prio {
    /// Canonical value (1–5), or `None` for "no priority".
    pub fn to_canonical(self) -> Option<i64> {
        match self {
            Prio::P1 => Some(1),
            Prio::P2 => Some(2),
            Prio::P3 => Some(3),
            Prio::P4 => Some(4),
            Prio::P5 => Some(5),
            Prio::None => None,
        }
    }

    /// Map a canonical value (1–5) back to the UI enum.
    pub fn from_canonical(p: Option<i64>) -> Prio {
        match p {
            Some(1) => Prio::P1,
            Some(2) => Prio::P2,
            Some(3) => Prio::P3,
            Some(4) => Prio::P4,
            Some(5) => Prio::P5,
            _ => Prio::None,
        }
    }
}

/// The structured result of parsing one Quick-Add line.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedTask {
    /// Title with all recognised tokens removed.
    pub title: String,
    /// RFC 3339 (UTC) due timestamp, if a date/time was recognised.
    pub due: Option<DateTime<Utc>>,
    /// True when a time-of-day was part of the input.
    pub due_has_time: bool,
    pub priority: Prio,
    pub project: Option<String>,
    pub labels: Vec<String>,
    /// Raw RRULE value when a recurrence was recognised.
    pub rrule: Option<String>,
}

impl Default for ParsedTask {
    fn default() -> Self {
        ParsedTask {
            title: String::new(),
            due: None,
            due_has_time: false,
            priority: Prio::None,
            project: None,
            labels: Vec::new(),
            rrule: None,
        }
    }
}

/// Weekday names (German + English) -> chrono weekday number (Mon = 1).
fn weekday_num(word: &str) -> Option<u32> {
    let w = word.to_lowercase();
    // Handle common German inflections ("freitags", "montag").
    let w = w.trim_end_matches('s');
    Some(match w {
        "montag" | "monday" | "mo" | "mon" => 1,
        "dienstag" | "tuesday" | "di" | "tue" => 2,
        "mittwoch" | "wednesday" | "mi" | "wed" => 3,
        "donnerstag" | "thursday" | "do" | "thu" => 4,
        "freitag" | "friday" | "fr" | "fri" => 5,
        "samstag" | "saturday" | "sa" | "sat" => 6,
        "sonntag" | "sunday" | "so" | "sun" => 7,
        _ => return None,
    })
}

/// Parse a line into a [`ParsedTask`], relative to `now_local`.
pub fn parse(input: &str, now_local: DateTime<Local>) -> ParsedTask {
    let mut out = ParsedTask::default();
    let mut leftover: Vec<String> = Vec::new();

    let tokens: Vec<&str> = input.split_whitespace().collect();
    let mut i = 0usize;
    while i < tokens.len() {
        let tok = tokens[i];
        let lower = tok.to_lowercase();

        // ── Project #Name ────────────────────────────────────────────────
        if let Some(rest) = tok.strip_prefix('#') {
            if !rest.is_empty() {
                out.project = Some(rest.to_string());
                i += 1;
                continue;
            }
        }
        // ── Label @Name / %Name ──────────────────────────────────────────
        if let Some(rest) = tok.strip_prefix('@').or_else(|| tok.strip_prefix('%')) {
            if !rest.is_empty() {
                let label = rest.to_string();
                if !out.labels.contains(&label) {
                    out.labels.push(label);
                }
                i += 1;
                continue;
            }
        }
        // ── Priority p1..p4 ──────────────────────────────────────────────
        if let Some(p) = parse_priority(&lower) {
            out.priority = p;
            i += 1;
            continue;
        }
        // ── Recurrence (may span tokens) ─────────────────────────────────
        if let Some((consumed, rrule, weekday)) = parse_recurrence(&tokens, i) {
            out.rrule = Some(rrule);
            // A weekday inside a recurrence also sets the first due date.
            if let Some(wd) = weekday {
                let d = next_weekday(now_local, wd);
                out.due = Some(local_date_to_utc(d.and_time(NaiveTime::MIN)));
            }
            i += consumed;
            continue;
        }
        // ── Time of day: "14:30", "um 14", "9 uhr" ───────────────────────
        if let Some((consumed, time)) = parse_time(&tokens, i) {
            if apply_time(&mut out, now_local, time) {
                i += consumed;
                continue;
            }
        }
        // ── Date: today/tomorrow/weekday/absolute ────────────────────────
        if let Some((consumed, date)) = parse_date(&tokens, i, now_local) {
            out.due = Some(local_date_to_utc(date.and_time(
                out.due.map(|d| d.with_timezone(&Local).time()).unwrap_or(NaiveTime::MIN),
            )));
            out.due_has_time = false;
            i += consumed;
            continue;
        }

        leftover.push(tok.to_string());
        i += 1;
    }

    out.title = leftover.join(" ").trim().to_string();
    out
}

fn parse_priority(lower: &str) -> Option<Prio> {
    match lower {
        "p1" => Some(Prio::P1),
        "p2" => Some(Prio::P2),
        "p3" => Some(Prio::P3),
        "p4" => Some(Prio::P4),
        "p5" => Some(Prio::P5),
        _ => None,
    }
}

/// Try to parse a recurrence starting at `i`. Returns (tokens consumed, RRULE,
/// optional weekday for the first due date).
fn parse_recurrence(tokens: &[&str], i: usize) -> Option<(usize, String, Option<u32>)> {
    let w = |n: usize| tokens.get(i + n).map(|s| s.to_lowercase());
    let t0 = w(0)?;

    // "alle N wochen/tage/monate" / "every N weeks"
    if t0 == "alle" || t0 == "every" {
        if let Some(nstr) = w(1) {
            if let Ok(n) = nstr.parse::<i64>() {
                if let Some(unit) = w(2) {
                    let freq = freq_for_unit(&unit)?;
                    return Some((3, format!("FREQ={freq};INTERVAL={}", n.max(1)), None));
                }
            }
        }
    }

    // "jeden tag", "jede woche", "jeden montag", "jeden monat", "jedes jahr"
    if t0 == "jeden" || t0 == "jede" || t0 == "jedes" || t0 == "every" {
        let unit = w(1)?;
        if let Some(wd) = weekday_num(&unit) {
            return Some((2, format!("FREQ=WEEKLY;BYDAY={}", ical_weekday(wd)), Some(wd)));
        }
        let freq = freq_for_unit(&unit)?;
        return Some((2, format!("FREQ={freq}"), None));
    }

    // Single-word recurrence.
    match t0.as_str() {
        "täglich" | "taeglich" | "daily" => return Some((1, "FREQ=DAILY".into(), None)),
        "wöchentlich" | "woechentlich" | "weekly" => {
            return Some((1, "FREQ=WEEKLY".into(), None))
        }
        "monatlich" | "monthly" => return Some((1, "FREQ=MONTHLY".into(), None)),
        "jährlich" | "jaehrlich" | "yearly" | "annually" => {
            return Some((1, "FREQ=YEARLY".into(), None))
        }
        _ => {}
    }
    None
}

fn freq_for_unit(unit: &str) -> Option<&'static str> {
    let u = unit.trim_end_matches('n');
    Some(match u {
        "tag" | "tage" | "day" | "days" => "DAILY",
        "woche" | "wochen" | "week" | "weeks" => "WEEKLY",
        "monat" | "monate" | "month" | "months" => "MONTHLY",
        "jahr" | "jahre" | "year" | "years" => "YEARLY",
        _ => return None,
    })
}

fn ical_weekday(wd: u32) -> &'static str {
    match wd {
        1 => "MO",
        2 => "TU",
        3 => "WE",
        4 => "TH",
        5 => "FR",
        6 => "SA",
        _ => "SU",
    }
}

/// Try to parse a time starting at `i`. Returns (tokens consumed, time).
fn parse_time(tokens: &[&str], i: usize) -> Option<(usize, NaiveTime)> {
    let t0 = *tokens.get(i)?;
    // "14:30"
    if let Some(t) = parse_hm(t0) {
        return Some((1, t));
    }
    let lower = t0.to_lowercase();
    // "um 14", "at 14", "um 14:30"
    if lower == "um" || lower == "at" {
        if let Some(t1) = tokens.get(i + 1) {
            if let Some(t) = parse_hm(t1).or_else(|| parse_hour_only(t1)) {
                return Some((2, t));
            }
        }
    }
    // "14 uhr" / "14h"
    if let Some(t1) = tokens.get(i + 1) {
        let u = t1.to_lowercase();
        if u == "uhr" || u == "h" {
            if let Some(rest) = lower.strip_suffix("h") {
                if let Some(t) = parse_hour_only(rest) {
                    return Some((1, t));
                }
            }
            if let Some(t) = parse_hour_only(&lower) {
                return Some((2, t));
            }
        }
    }
    None
}

/// `14:30` -> NaiveTime.
fn parse_hm(s: &str) -> Option<NaiveTime> {
    let (h, m) = s.split_once(':')?;
    let hour: u32 = h.parse().ok()?;
    let min: u32 = m.parse().ok()?;
    if hour > 23 || min > 59 {
        return None;
    }
    NaiveTime::from_hms_opt(hour, min, 0)
}

/// Bare hour `14` -> 14:00 (only 0–23).
fn parse_hour_only(s: &str) -> Option<NaiveTime> {
    let h: u32 = s.parse().ok()?;
    if h > 23 {
        return None;
    }
    NaiveTime::from_hms_opt(h, 0, 0)
}

/// Apply a parsed time to the task: attach it to the current due date (or
/// today) and mark `due_has_time`.
fn apply_time(out: &mut ParsedTask, now_local: DateTime<Local>, time: NaiveTime) -> bool {
    let date = out
        .due
        .map(|d| d.with_timezone(&Local).date_naive())
        .unwrap_or_else(|| now_local.date_naive());
    out.due = Some(local_date_to_utc(date.and_time(time)));
    out.due_has_time = true;
    true
}

/// Try to parse a date starting at `i`. Returns (tokens consumed, date).
fn parse_date(tokens: &[&str], i: usize, now_local: DateTime<Local>) -> Option<(usize, NaiveDate)> {
    let lower = tokens[i].to_lowercase();
    let today = now_local.date_naive();

    // Relative single words.
    match lower.as_str() {
        "heute" | "today" => return Some((1, today)),
        "morgen" | "tomorrow" => return Some((1, today + Duration::days(1))),
        "übermorgen" | "uebermorgen" => return Some((1, today + Duration::days(2))),
        "gestern" | "yesterday" => return Some((1, today - Duration::days(1))),
        _ => {}
    }
    // Weekday alone -> next occurrence (today counts as this week).
    if let Some(wd) = weekday_num(&lower) {
        return Some((1, next_weekday(now_local, wd)));
    }
    // "nächste woche" / "next week"
    if (lower == "nächste" || lower == "naechste" || lower == "next")
        && tokens
            .get(i + 1)
            .map(|t| {
                let u = t.to_lowercase();
                u == "woche" || u == "week"
            })
            .unwrap_or(false)
    {
        return Some((2, today + Duration::days(7)));
    }
    // "in N tagen" / "in N days"
    if lower == "in" {
        if let Some(nstr) = tokens.get(i + 1) {
            if let Ok(n) = nstr.parse::<i64>() {
                // optional unit token
                let consumes = if tokens.get(i + 2).is_some() { 3 } else { 2 };
                return Some((consumes, today + Duration::days(n)));
            }
        }
    }
    // "am 3.10." / "3.10." / "3.10.2026" / "am 03.10."
    let (consumed_prefix, date_tok) = if lower == "am" || lower == "on" {
        (1usize, tokens.get(i + 1)?.to_string())
    } else {
        (0usize, tokens[i].to_string())
    };
    if let Some(d) = parse_german_date(&date_tok, today.year()) {
        let has_prefix = if consumed_prefix == 1 { 2 } else { 1 };
        return Some((has_prefix, d));
    }
    // ISO "2026-10-03"
    if let Ok(d) = NaiveDate::parse_from_str(&date_tok, "%Y-%m-%d") {
        return Some((1, d));
    }
    None
}

/// Parse `3.10.`, `3.10.2026`, `03.10` into a NaiveDate (year defaults to
/// `default_year`; a date already past rolls to next year).
fn parse_german_date(tok: &str, default_year: i32) -> Option<NaiveDate> {
    let cleaned = tok.trim_end_matches('.');
    let parts: Vec<&str> = cleaned.split('.').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let day: u32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let year: i32 = if parts.len() == 3 {
        parts[2].parse().ok()?
    } else {
        default_year
    };
    NaiveDate::from_ymd_opt(year, month, day)
}

/// Next occurrence of a weekday (Mon=1 … Sun=7), starting from `now`. If today
/// *is* that weekday, today is returned.
fn next_weekday(now_local: DateTime<Local>, wd: u32) -> NaiveDate {
    let today = now_local.date_naive();
    let cur = today.weekday().num_days_from_monday() + 1; // 1..7
    let mut delta = (wd as i64) - (cur as i64);
    if delta < 0 {
        delta += 7;
    }
    today + Duration::days(delta)
}

/// Convert a local date+time into a UTC timestamp.
fn local_date_to_utc(dt: chrono::NaiveDateTime) -> DateTime<Utc> {
    match Local.from_local_datetime(&dt).earliest() {
        Some(local) => local.with_timezone(&Utc),
        None => Utc.from_utc_datetime(&dt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    /// Fixed reference: Wednesday, 2026-09-16, 10:00 local.
    fn now() -> DateTime<Local> {
        let naive = NaiveDate::from_ymd_opt(2026, 9, 16)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();
        Local.from_local_datetime(&naive).unwrap()
    }

    fn p(s: &str) -> ParsedTask {
        parse(s, now())
    }

    #[test]
    fn plain_title() {
        let r = p("Budget prüfen");
        assert_eq!(r.title, "Budget prüfen");
        assert!(r.due.is_none());
        assert_eq!(r.priority, Prio::None);
    }

    #[test]
    fn project_and_labels() {
        let r = p("Angebot senden #Firma @dringend");
        assert_eq!(r.title, "Angebot senden");
        assert_eq!(r.project.as_deref(), Some("Firma"));
        assert_eq!(r.labels, vec!["dringend"]);
    }

    #[test]
    fn percent_label_variant() {
        let r = p("Test %wichtig");
        assert_eq!(r.labels, vec!["wichtig"]);
    }

    #[test]
    fn priority_p1_to_p5() {
        assert_eq!(p("A p1").priority, Prio::P1);
        assert_eq!(p("A p5").priority, Prio::P5);
        assert_eq!(Prio::P1.to_canonical(), Some(1));
        assert_eq!(Prio::P2.to_canonical(), Some(2));
        assert_eq!(Prio::P3.to_canonical(), Some(3));
        assert_eq!(Prio::P4.to_canonical(), Some(4));
        assert_eq!(Prio::P5.to_canonical(), Some(5));
        assert_eq!(Prio::None.to_canonical(), None);
        assert_eq!(Prio::from_canonical(Some(3)), Prio::P3);
        assert_eq!(Prio::from_canonical(None), Prio::None);
    }

    #[test]
    fn relative_dates() {
        assert_eq!(p("Heute anrufen").due.unwrap().with_timezone(&Local).date_naive(),
                   NaiveDate::from_ymd_opt(2026, 9, 16).unwrap());
        assert_eq!(p("Morgen anrufen").due.unwrap().with_timezone(&Local).date_naive(),
                   NaiveDate::from_ymd_opt(2026, 9, 17).unwrap());
    }

    #[test]
    fn weekday_resolves_to_next_occurrence() {
        // Reference is a Wednesday; "Freitag" -> 2026-09-18.
        let r = p("Freitag Bericht");
        assert_eq!(r.due.unwrap().with_timezone(&Local).date_naive(),
                   NaiveDate::from_ymd_opt(2026, 9, 18).unwrap());
        assert_eq!(r.title, "Bericht");
    }

    #[test]
    fn time_attaches_and_sets_flag() {
        let r = p("Morgen um 14:30 Meeting");
        assert!(r.due_has_time);
        let local = r.due.unwrap().with_timezone(&Local);
        assert_eq!(local.hour(), 14);
        assert_eq!(local.minute(), 30);
        assert_eq!(r.title, "Meeting");
    }

    #[test]
    fn bare_hour_with_uhr() {
        let r = p("Heute 9 uhr Zählerstand");
        assert!(r.due_has_time);
        assert_eq!(r.due.unwrap().with_timezone(&Local).hour(), 9);
    }

    #[test]
    fn recurrence_daily() {
        assert_eq!(p("Jeden tag Sport").rrule.as_deref(), Some("FREQ=DAILY"));
        assert_eq!(p("täglich Sport").rrule.as_deref(), Some("FREQ=DAILY"));
    }

    #[test]
    fn recurrence_weekly_weekday() {
        let r = p("Jeden Freitag Wochenbericht");
        assert_eq!(r.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=FR"));
        assert_eq!(r.title, "Wochenbericht");
        assert!(r.due.is_some(), "first occurrence is set");
    }

    #[test]
    fn recurrence_interval() {
        assert_eq!(p("Alle 2 Wochen putzen").rrule.as_deref(),
                   Some("FREQ=WEEKLY;INTERVAL=2"));
    }

    #[test]
    fn absolute_german_date() {
        let r = p("Am 3.10. Rechnung");
        assert_eq!(r.due.unwrap().with_timezone(&Local).date_naive(),
                   NaiveDate::from_ymd_opt(2026, 10, 3).unwrap());
    }

    #[test]
    fn absolute_iso_date() {
        let r = p("2026-12-24 Geschenke");
        assert_eq!(r.due.unwrap().with_timezone(&Local).date_naive(),
                   NaiveDate::from_ymd_opt(2026, 12, 24).unwrap());
    }

    #[test]
    fn full_quick_add_line() {
        let r = p("Freitag Budget prüfen p1 #Firma @dringend");
        assert_eq!(r.title, "Budget prüfen");
        assert_eq!(r.priority, Prio::P1);
        assert_eq!(r.project.as_deref(), Some("Firma"));
        assert_eq!(r.labels, vec!["dringend"]);
        assert_eq!(r.due.unwrap().with_timezone(&Local).date_naive(),
                   NaiveDate::from_ymd_opt(2026, 9, 18).unwrap());
    }

    #[test]
    fn in_n_days() {
        let r = p("In 3 tagen Nachfassen");
        assert_eq!(r.due.unwrap().with_timezone(&Local).date_naive(),
                   NaiveDate::from_ymd_opt(2026, 9, 19).unwrap());
    }

    #[test]
    fn next_week() {
        let r = p("Nächste Woche Reise planen");
        assert_eq!(r.due.unwrap().with_timezone(&Local).date_naive(),
                   NaiveDate::from_ymd_opt(2026, 9, 23).unwrap());
    }

    #[test]
    fn empty_input_is_safe() {
        let r = p("");
        assert_eq!(r.title, "");
        assert!(r.due.is_none());
    }
}
