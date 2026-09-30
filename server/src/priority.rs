//! Canonical task priority scale.
//!
//! Relay uses a single canonical scale everywhere (DB, API, UI, LLM prompt):
//!
//! * **1 = highest … 5 = lowest**, plus *none* (`None` = unset/undetermined).
//!
//! The iCalendar wire format (`PRIORITY`, RFC 5545) is still `0–9`
//! (1 = highest, 9 = lowest, 0/absent = undefined). Only the DAV boundary
//! translates between the two; nothing else in the system sees `6–9`.

/// Lowest canonical priority value (highest urgency).
pub const MIN: i64 = 1;
/// Highest canonical priority value (lowest urgency).
pub const MAX: i64 = 5;

/// The internal code examples for the priority gradations, used in prompts and
/// as guidance so automatic assignment stays consistent (see `AGENTS`/docs):
///
/// * `1` — critical: due today/overdue, blocks others, explicitly urgent.
/// * `2` — high: due within ~3–5 days, important and timely.
/// * `3` — normal: a real task without time pressure / mid-term.
/// * `4` — low: can wait, nice-to-have.
/// * `5` — very low: backlog/idea without a date, little value.
/// * `None` — undetermined (the default when nothing indicates urgency).
pub const LEVELS: i64 = MAX;

/// Clamp an arbitrary canonical value into `1..=5`.
pub fn clamp_canonical(p: i64) -> i64 {
    p.clamp(MIN, MAX)
}

/// Normalise a value coming from an API/LLM into the canonical scale.
/// Returns `None` for "no priority" (0, negative, or absent).
pub fn normalize(p: Option<i64>) -> Option<i64> {
    match p {
        Some(n) if n >= MIN => Some(clamp_canonical(n)),
        _ => None,
    }
}

/// Canonical → iCalendar `PRIORITY` wire value (`1→1, 2→3, 3→5, 4→7, 5→9`).
/// `None` means "emit no PRIORITY property".
pub fn to_ical(canonical: Option<i64>) -> Option<i64> {
    normalize(canonical).map(|p| p * 2 - 1)
}

/// iCalendar `PRIORITY` → canonical (`1–2→1, 3–4→2, 5–6→3, 7–8→4, 9→5`).
/// `0`/absent/out-of-range → `None`.
pub fn from_ical(ical: Option<i64>) -> Option<i64> {
    match ical {
        Some(n) if n >= 1 => Some(clamp_canonical((n + 1) / 2)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_ical_spreizt_1_bis_5() {
        assert_eq!(to_ical(Some(1)), Some(1));
        assert_eq!(to_ical(Some(2)), Some(3));
        assert_eq!(to_ical(Some(3)), Some(5));
        assert_eq!(to_ical(Some(4)), Some(7));
        assert_eq!(to_ical(Some(5)), Some(9));
        assert_eq!(to_ical(None), None);
    }

    #[test]
    fn from_ical_rundet_auf_stufen() {
        assert_eq!(from_ical(Some(1)), Some(1));
        assert_eq!(from_ical(Some(2)), Some(1));
        assert_eq!(from_ical(Some(3)), Some(2));
        assert_eq!(from_ical(Some(4)), Some(2));
        assert_eq!(from_ical(Some(5)), Some(3));
        assert_eq!(from_ical(Some(6)), Some(3));
        assert_eq!(from_ical(Some(7)), Some(4));
        assert_eq!(from_ical(Some(8)), Some(4));
        assert_eq!(from_ical(Some(9)), Some(5));
        assert_eq!(from_ical(Some(0)), None);
        assert_eq!(from_ical(None), None);
        assert_eq!(from_ical(Some(42)), Some(5));
    }

    #[test]
    fn roundtrip_ist_stabil() {
        for p in MIN..=MAX {
            assert_eq!(from_ical(to_ical(Some(p))), Some(p));
        }
    }

    #[test]
    fn normalize_klemmt_und_verwirft_none() {
        assert_eq!(normalize(Some(0)), None);
        assert_eq!(normalize(Some(-1)), None);
        assert_eq!(normalize(Some(3)), Some(3));
        assert_eq!(normalize(Some(9)), Some(5));
    }
}
