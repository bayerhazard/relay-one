//! Production iCalendar (RFC 5545) wrapper built on the `icalendar` crate.
//!
//! Exposes a small, serialisable domain model ([`IcsEvent`]) plus parse/build
//! helpers used by the CalDAV client and the API layer. The `icalendar` crate
//! types are kept internal to this module so the rest of the codebase only
//! deals with plain data.

use chrono::{DateTime, TimeZone, Utc};
use icalendar::{
    Alarm, Attendee, Calendar, CalendarDateTime, Class, Component, Event, EventLike, EventStatus,
    PartStat, Property, Role, Trigger,
};
use serde::{Deserialize, Serialize};

/// A parsed calendar event (VEVENT), reduced to the fields Relay needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcsEvent {
    pub uid: String,
    /// CalDAV object URL (set by the client after fetch; empty when built locally).
    #[serde(default)]
    pub url: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    /// Start time as RFC 3339 (UTC) when a time is present.
    pub start: String,
    /// End time as RFC 3339 (UTC), if present.
    pub end: Option<String>,
    /// True for all-day events (DATE value, no time-of-day).
    pub all_day: bool,
    pub organizer: Option<String>,
    pub attendees: Vec<IcsAttendee>,
    /// CONFIRMED / TENTATIVE / CANCELLED
    pub status: Option<String>,
    pub sequence: i64,
    /// Raw RRULE value, if the event recurs.
    pub rrule: Option<String>,
    /// Number of alarms (VALARM) attached to the event.
    pub alarms: usize,
    /// The raw ICS text of the single VEVENT (for round-trip / PUT).
    pub raw: String,
}

/// An attendee (ATTENDEE) of an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcsAttendee {
    pub email: String,
    pub name: Option<String>,
    /// NEEDS-ACTION / ACCEPTED / DECLINED / TENTATIVE
    pub part_stat: Option<String>,
    pub rsvp: bool,
}

/// Convert a parsed `icalendar::Event` into the Relay [`IcsEvent`] model.
fn extract_event(ev: &Event, raw: &str) -> IcsEvent {
    let (start, end, all_day) = to_rfc3339(ev.get_start(), ev.get_end());

    let organizer = ev
        .properties()
        .get("ORGANIZER")
        .map(|p| p.value().trim_start_matches("mailto:").to_string());

    let attendees = ev
        .get_attendees()
        .into_iter()
        .map(|a| IcsAttendee {
            email: a.cal_address.trim_start_matches("mailto:").to_string(),
            name: a.cn,
            part_stat: a.part_stat.map(part_stat_name),
            rsvp: a.rsvp.unwrap_or(false),
        })
        .collect();

    let rrule = ev
        .properties()
        .get("RRULE")
        .map(|p| p.value().to_string());

    // VALARM is a sub-component (not a property); count occurrences in the raw
    // ICS text — robust across icalendar-crate versions.
    let alarms = raw.matches("BEGIN:VALARM").count();

    IcsEvent {
        uid: ev.get_uid().unwrap_or_default().to_string(),
        url: String::new(),
        summary: ev.get_summary().map(|s| s.to_string()),
        description: ev.get_description().map(|s| s.to_string()),
        location: ev.get_location().map(|s| s.to_string()),
        start,
        end,
        all_day,
        organizer,
        attendees,
        status: ev
            .properties()
            .get("STATUS")
            .map(|p| p.value().to_string()),
        sequence: ev.get_sequence().unwrap_or(0) as i64,
        rrule,
        alarms,
        raw: raw.to_string(),
    }
}

/// Parse a VCALENDAR (or bare VEVENT) body into a single [`IcsEvent`].
///
/// Returns an error if no VEVENT is present.
pub fn parse_event(ics: &str) -> Result<IcsEvent, String> {
    let cal: Calendar = ics.parse().map_err(|e| format!("ICS-Parsing fehlgeschlagen: {e}"))?;
    let ev = cal
        .events()
        .next()
        .ok_or_else(|| "Kein VEVENT in der ICS gefunden".to_string())?;
    Ok(extract_event(&ev, ics))
}

/// Parse a VCALENDAR body into ALL its [`IcsEvent`]s (for ICS import).
///
/// Returns an empty Vec if the calendar has no VEVENTs.
pub fn parse_events(ics: &str) -> Result<Vec<IcsEvent>, String> {
    let cal: Calendar = ics.parse().map_err(|e| format!("ICS-Parsing fehlgeschlagen: {e}"))?;
    Ok(cal.events().map(|ev| extract_event(&ev, ics)).collect())
}

/// Extract the single VEVENT with the given UID from a (possibly multi-event)
/// ICS body, wrapped in a minimal VCALENDAR. Returns `None` if not found.
///
/// Used for faithful single-event storage / CalDAV PUT during import.
pub fn extract_vevent(ics: &str, uid: &str) -> Option<String> {
    let mut out: Vec<String> = vec!["BEGIN:VCALENDAR".into(), "VERSION:2.0".into()];
    let mut buf: Option<Vec<String>> = None;
    for line in ics.lines() {
        let t = line.trim_start();
        if t.eq_ignore_ascii_case("BEGIN:VEVENT") {
            buf = Some(vec![line.to_string()]);
            continue;
        }
        if let Some(b) = buf.as_mut() {
            b.push(line.to_string());
            if t.eq_ignore_ascii_case("END:VEVENT") {
                let has_uid = b.iter().any(|l| {
                    let lt = l.trim_start();
                    lt.starts_with("UID:") && lt[4..].trim() == uid
                });
                if has_uid {
                    out.extend(b.drain(..));
                }
                buf = None;
            }
        }
    }
    if out.len() <= 2 {
        return None;
    }
    out.push("END:VCALENDAR".into());
    Some(out.join("\r\n"))
}

/// Build a minimal VCALENDAR containing a single VEVENT, suitable for a
/// CalDAV `PUT`. Times are interpreted in the given IANA time zone.
pub fn build_event(
    uid: &str,
    summary: &str,
    start: DateTime<Utc>,
    end: Option<DateTime<Utc>>,
    description: Option<&str>,
    location: Option<&str>,
    organizer: Option<&str>,
    attendees: &[IcsAttendee],
    rrule: Option<&str>,
    reminder_minutes: Option<u32>,
) -> Result<String, String> {
    // Phase 0: single default zone (Europe/Berlin). Per-event zones land later.
    let tz = chrono_tz::Europe::Berlin;

    let mut ev = Event::new();
    ev.uid(uid).summary(summary).starts(dt_to_cal(&start, tz));
    if let Some(e) = end {
        ev.ends(dt_to_cal(&e, tz));
    }
    if let Some(d) = description {
        ev.description(d);
    }
    if let Some(l) = location {
        ev.location(l);
    }
    ev.class(Class::Public)
        .status(EventStatus::Confirmed)
        .sequence(0);

    if let Some(org) = organizer {
        ev.append_property(Property::new(
            "ORGANIZER",
            format!("mailto:{org}"),
        ));
    }
    for a in attendees {
        let mut att = Attendee::new(format!("mailto:{}", a.email))
            .partstat(PartStat::NeedsAction)
            .rsvp(a.rsvp)
            .role(Role::ReqParticipant);
        if let Some(name) = &a.name {
            att.cn = Some(name.clone());
        }
        ev.attendee(att);
    }
    if let Some(rule) = rrule {
        ev.append_property(Property::new("RRULE", rule));
    }
    if let Some(mins) = reminder_minutes {
        ev.alarm(Alarm::display(
            "Erinnerung",
            Trigger::before_start(chrono::Duration::minutes(mins as i64)),
        ));
    }

    let cal = Calendar::new().push(ev).done();
    Ok(cal.to_string())
}

/// Convert an optional start/end pair into RFC 3339 UTC strings.
fn to_rfc3339(
    start: Option<icalendar::DatePerhapsTime>,
    end: Option<icalendar::DatePerhapsTime>,
) -> (String, Option<String>, bool) {
    let all_day = matches!(start, Some(icalendar::DatePerhapsTime::Date(_)));
    let start_s = match start {
        Some(icalendar::DatePerhapsTime::Date(d)) => d.format("%Y-%m-%d").to_string(),
        Some(icalendar::DatePerhapsTime::DateTime(dt)) => cal_dt_to_rfc3339(&dt),
        None => String::new(),
    };
    let end_s = end.and_then(|e| match e {
        icalendar::DatePerhapsTime::Date(d) => Some(d.format("%Y-%m-%d").to_string()),
        icalendar::DatePerhapsTime::DateTime(dt) => Some(cal_dt_to_rfc3339(&dt)),
    });
    (start_s, end_s, all_day)
}

/// Render a [`CalendarDateTime`] as an RFC 3339 UTC string.
fn cal_dt_to_rfc3339(dt: &CalendarDateTime) -> String {
    match dt {
        CalendarDateTime::Utc(t) => t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        CalendarDateTime::Floating(naive) => {
            // Floating time — no zone info; treat as UTC (best effort).
            let dt = DateTime::<Utc>::from_naive_utc_and_offset(*naive, Utc);
            dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        }
        CalendarDateTime::WithTimezone { date_time, tzid } => tzid
            .parse::<chrono_tz::Tz>()
            .ok()
            .and_then(|tz| tz.from_local_datetime(date_time).single())
            .map(|t| {
                t.with_timezone(&Utc)
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            })
            .unwrap_or_else(|| date_time.format("%Y-%m-%dT%H:%M:%S").to_string()),
    }
}

/// Convert a `DateTime<Utc>` into a `CalendarDateTime` in the given zone.
fn dt_to_cal(dt: &DateTime<Utc>, tz: chrono_tz::Tz) -> CalendarDateTime {
    let local = dt.with_timezone(&tz);
    CalendarDateTime::WithTimezone {
        date_time: local.naive_local(),
        tzid: tz.name().to_owned(),
    }
}

/// Convert a [`CalendarDateTime`] to `DateTime<Utc>`.
fn cal_dt_to_utc(dt: &CalendarDateTime) -> Option<DateTime<Utc>> {
    match dt {
        CalendarDateTime::Utc(t) => Some(*t),
        CalendarDateTime::Floating(naive) => {
            Some(DateTime::<Utc>::from_naive_utc_and_offset(*naive, Utc))
        }
        CalendarDateTime::WithTimezone { date_time, tzid } => tzid
            .parse::<chrono_tz::Tz>()
            .ok()
            .and_then(|tz| tz.from_local_datetime(date_time).single())
            .map(|t| t.with_timezone(&Utc)),
    }
}

/// Convert a `DatePerhapsTime` to `DateTime<Utc>` (DATE → midnight UTC).
fn dpt_to_utc(dpt: &icalendar::DatePerhapsTime) -> Option<DateTime<Utc>> {
    match dpt {
        icalendar::DatePerhapsTime::Date(d) => d
            .and_hms_opt(0, 0, 0)
            .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)),
        icalendar::DatePerhapsTime::DateTime(dt) => cal_dt_to_utc(dt),
    }
}

fn parse_rfc3339_utc(s: &str) -> Result<DateTime<Utc>, String> {
    // Accept a bare date (YYYY-MM-DD) as midnight UTC — the frontend may send
    // date-only window bounds.
    if s.len() == 10 {
        let d = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|e| format!("Ungültiges Datum '{s}': {e}"))?;
        let naive = d
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| format!("Ungültiges Datum '{s}'"))?;
        return Ok(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc));
    }
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| format!("Ungültiges Datum '{s}': {e}"))
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Expand a VEVENT's occurrences within `[from, to)` (RFC 3339 UTC).
///
/// Returns `(occurrence_start, occurrence_end)` RFC 3339 UTC pairs in
/// chronological order. Non-recurring events yield at most one pair (their own
/// start/end) when they begin before `to`. Recurring events are expanded via
/// the `rrule` engine, honouring EXDATE/EXRULE, capped at 5000 iterations as a
/// safety bound against unbounded rules with a distant DTSTART.
pub fn expand_occurrences(ics: &str, from: &str, to: &str) -> Result<Vec<(String, Option<String>)>, String> {
    let cal: Calendar = ics.parse().map_err(|e| format!("ICS-Parsing fehlgeschlagen: {e}"))?;
    let ev = cal
        .events()
        .next()
        .ok_or_else(|| "Kein VEVENT in der ICS gefunden".to_string())?;

    let from_utc = parse_rfc3339_utc(from)?;
    let to_utc = parse_rfc3339_utc(to)?;

    let start_utc = ev
        .get_start()
        .as_ref()
        .and_then(dpt_to_utc)
        .ok_or_else(|| "Kein DTSTART im VEVENT".to_string())?;
    let end_utc = ev.get_end().as_ref().and_then(dpt_to_utc);
    let duration = end_utc.map(|e| e - start_utc);

    let has_rrule = ev.properties().get("RRULE").is_some();
    if !has_rrule {
        return Ok(if start_utc < to_utc {
            vec![(rfc3339(start_utc), end_utc.map(rfc3339))]
        } else {
            vec![]
        });
    }

    let set = ev
        .get_recurrence()
        .map_err(|e| format!("RRULE-Auswertung fehlgeschlagen: {e}"))?;

    let mut out: Vec<(String, Option<String>)> = Vec::new();
    let mut count = 0u32;
    for dt in &set {
        count += 1;
        if count > 5000 {
            break;
        }
        let occ_utc: DateTime<Utc> = dt.with_timezone(&Utc);
        if occ_utc >= to_utc {
            break; // occurrences are chronological
        }
        if occ_utc < from_utc {
            continue;
        }
        out.push((rfc3339(occ_utc), duration.map(|d| rfc3339(occ_utc + d))));
    }
    Ok(out)
}

fn part_stat_name(p: PartStat) -> String {
    match p {
        PartStat::NeedsAction => "NEEDS-ACTION".into(),
        PartStat::Accepted => "ACCEPTED".into(),
        PartStat::Declined => "DECLINED".into(),
        PartStat::Tentative => "TENTATIVE".into(),
        PartStat::Delegated => "DELEGATED".into(),
        PartStat::Completed => "COMPLETED".into(),
        PartStat::InProcess => "IN-PROCESS".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Radicale//NONSGML Radicale 3.5.7//EN
BEGIN:VEVENT
SUMMARY:Projektbesprechung
DTSTART;TZID=Europe/Berlin:20260901T150000
DTEND;TZID=Europe/Berlin:20260901T160000
UID:relay-test-001@aimighty
SEQUENCE:0
STATUS:CONFIRMED
ORGANIZER;CN=Marc Bayer:mailto:marc@example.com
ATTENDEE;CN=Anna;RSVP=TRUE;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION:mailto:anna@example.com
LOCATION:Büro, 3. Stock
DESCRIPTION:Agenda
RRULE:FREQ=WEEKLY;BYDAY=WE;COUNT=4
BEGIN:VALARM
ACTION:DISPLAY
TRIGGER:-PT30M
SUMMARY:Erinnerung
END:VALARM
END:VEVENT
END:VCALENDAR
"#;

    #[test]
    fn test_parse_event() {
        let ev = parse_event(SAMPLE).unwrap();
        assert_eq!(ev.uid, "relay-test-001@aimighty");
        assert_eq!(ev.summary.as_deref(), Some("Projektbesprechung"));
        assert!(!ev.all_day);
        // 15:00 CEST = 13:00 UTC
        assert!(ev.start.contains("T13:00:00Z"), "start={}", ev.start);
        assert_eq!(ev.organizer.as_deref(), Some("marc@example.com"));
        assert_eq!(ev.attendees.len(), 1);
        assert_eq!(ev.attendees[0].email, "anna@example.com");
        assert_eq!(ev.attendees[0].name.as_deref(), Some("Anna"));
        assert_eq!(ev.attendees[0].part_stat.as_deref(), Some("NEEDS-ACTION"));
        assert!(ev.rrule.as_deref().unwrap().contains("FREQ=WEEKLY"));
        assert_eq!(ev.alarms, 1);
        assert_eq!(ev.status.as_deref(), Some("CONFIRMED"));
    }

    #[test]
    fn test_build_event_roundtrip() {
        let start = DateTime::parse_from_rfc3339("2026-09-01T13:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let end = DateTime::parse_from_rfc3339("2026-09-01T14:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let attendees = vec![IcsAttendee {
            email: "anna@example.com".into(),
            name: Some("Anna".into()),
            part_stat: Some("NEEDS-ACTION".into()),
            rsvp: true,
        }];
        let ics = build_event(
            "relay-new-002@aimighty",
            "Neues Meeting",
            start,
            Some(end),
            Some("Test"),
            None,
            Some("marc@example.com"),
            &attendees,
            None,
            Some(30),
        )
        .unwrap();
        assert!(ics.contains("BEGIN:VEVENT"));
        assert!(ics.contains("TZID=Europe/Berlin"));
        assert!(ics.contains("ATTENDEE"));
        assert!(ics.contains("BEGIN:VALARM"));

        let parsed = parse_event(&ics).unwrap();
        assert_eq!(parsed.uid, "relay-new-002@aimighty");
        assert_eq!(parsed.summary.as_deref(), Some("Neues Meeting"));
        assert_eq!(parsed.attendees.len(), 1);
        assert!(parsed.start.contains("T13:00:00Z"));
    }

    const DAILY: &str = r#"BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Relay//EN
BEGIN:VEVENT
SUMMARY:Tägliche Standup
DTSTART:20260901T090000Z
DTEND:20260901T100000Z
UID:daily-001@aimighty
RRULE:FREQ=DAILY;COUNT=5
END:VEVENT
END:VCALENDAR
"#;

    #[test]
    fn test_expand_daily_full_window() {
        let occs = expand_occurrences(DAILY, "2026-09-01T00:00:00Z", "2026-09-10T00:00:00Z").unwrap();
        assert_eq!(occs.len(), 5);
        assert_eq!(occs[0].0, "2026-09-01T09:00:00Z");
        assert_eq!(occs[0].1.as_deref(), Some("2026-09-01T10:00:00Z"));
        assert_eq!(occs[4].0, "2026-09-05T09:00:00Z");
    }

    #[test]
    fn test_expand_daily_sub_window() {
        // Only Sep 3 and Sep 4 fall in [Sep 3, Sep 5).
        let occs = expand_occurrences(DAILY, "2026-09-03T00:00:00Z", "2026-09-05T00:00:00Z").unwrap();
        assert_eq!(occs.len(), 2);
        assert_eq!(occs[0].0, "2026-09-03T09:00:00Z");
        assert_eq!(occs[1].0, "2026-09-04T09:00:00Z");
    }

    #[test]
    fn test_expand_exdate_skips_occurrence() {
        let ics = DAILY.replace(
            "RRULE:FREQ=DAILY;COUNT=5",
            "EXDATE:20260903T090000Z\nRRULE:FREQ=DAILY;COUNT=5",
        );
        let occs = expand_occurrences(&ics, "2026-09-01T00:00:00Z", "2026-09-10T00:00:00Z").unwrap();
        let starts: Vec<&str> = occs.iter().map(|(s, _)| s.as_str()).collect();
        assert!(!starts.contains(&"2026-09-03T09:00:00Z"), "EXDATE not honoured: {starts:?}");
        assert_eq!(occs.len(), 4);
    }

    const MULTI: &str = r#"BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Import//EN
BEGIN:VEVENT
SUMMARY:Erstes
DTSTART:20260901T090000Z
DTEND:20260901T100000Z
UID:imp-1@aimighty
END:VEVENT
BEGIN:VEVENT
SUMMARY:Zweites
DTSTART:20260902T090000Z
DTEND:20260902T100000Z
UID:imp-2@aimighty
END:VEVENT
END:VCALENDAR
"#;

    #[test]
    fn test_parse_events_multi() {
        let evs = parse_events(MULTI).unwrap();
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].uid, "imp-1@aimighty");
        assert_eq!(evs[0].summary.as_deref(), Some("Erstes"));
        assert_eq!(evs[1].uid, "imp-2@aimighty");
        assert_eq!(evs[1].summary.as_deref(), Some("Zweites"));
    }

    #[test]
    fn test_extract_vevent_by_uid() {
        let block = extract_vevent(MULTI, "imp-2@aimighty").unwrap();
        assert!(block.contains("BEGIN:VCALENDAR"));
        assert!(block.contains("SUMMARY:Zweites"));
        assert!(!block.contains("SUMMARY:Erstes"));
        // Re-parses to a single event.
        let evs = parse_events(&block).unwrap();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].uid, "imp-2@aimighty");
    }

    #[test]
    fn test_extract_vevent_missing() {
        assert!(extract_vevent(MULTI, "does-not-exist").is_none());
    }

    // Real recurring event pulled from the live Radicale server (all-day,
    // infinite YEARLY recurrence, one VALARM). Verifies expansion against a
    // real-world RRULE, not just synthetic data.
    const REAL_YEARLY: &str = r#"BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Radicale//NONSGML Version 3.7.3//EN
BEGIN:VEVENT
UID:33F360EB-EFFC-4903-95C5-A975635E7928
DTSTART;VALUE=DATE:20071007
DTEND;VALUE=DATE:20071008
CREATED:20160416T090332Z
DTSTAMP:20260522T160615Z
RRULE:FREQ=YEARLY;BYMONTH=10;BYMONTHDAY=7
SEQUENCE:0
SUMMARY:Millas Geburtstag
TRANSP:OPAQUE
BEGIN:VALARM
ACTION:AUDIO
TRIGGER:-PT15H
END:VALARM
END:VEVENT
END:VCALENDAR
"#;

    #[test]
    fn test_expand_real_radicale_yearly() {
        // One-year window → exactly one occurrence (Oct 7).
        let occs = expand_occurrences(REAL_YEARLY, "2026-01-01", "2027-01-01").unwrap();
        assert_eq!(occs.len(), 1);
        assert_eq!(occs[0].0, "2026-10-07T00:00:00Z");
        // Two-year window → two occurrences.
        let occs2 = expand_occurrences(REAL_YEARLY, "2025-01-01", "2027-01-01").unwrap();
        assert_eq!(occs2.len(), 2);
        assert_eq!(occs2[0].0, "2025-10-07T00:00:00Z");
        assert_eq!(occs2[1].0, "2026-10-07T00:00:00Z");
        // Parses as an event with the alarm counted.
        let ev = parse_event(REAL_YEARLY).unwrap();
        assert_eq!(ev.summary.as_deref(), Some("Millas Geburtstag"));
        assert!(ev.all_day);
        assert_eq!(ev.alarms, 1);
    }

    #[test]
    fn test_expand_non_recurring() {
        let ics = r#"BEGIN:VCALENDAR
VERSION:2.0
BEGIN:VEVENT
SUMMARY:Einmalig
DTSTART:20260904T090000Z
DTEND:20260904T100000Z
UID:once-001@aimighty
END:VEVENT
END:VCALENDAR
"#;
        let in_win = expand_occurrences(ics, "2026-09-01T00:00:00Z", "2026-09-10T00:00:00Z").unwrap();
        assert_eq!(in_win.len(), 1);
        assert_eq!(in_win[0].0, "2026-09-04T09:00:00Z");
        // Window entirely before the event → empty.
        let before = expand_occurrences(ics, "2026-08-01T00:00:00Z", "2026-08-31T00:00:00Z").unwrap();
        assert!(before.is_empty());
    }
}
