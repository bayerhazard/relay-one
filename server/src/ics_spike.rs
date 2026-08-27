//! Spike test for the `icalendar` crate API — validates parse/build of the
//! VEVENT shapes Radicale produces before building the production ics module.
#![allow(dead_code)]

use icalendar::{
    Alarm, Attendee, Calendar, CalendarDateTime, Class, Event, EventStatus, PartStat, Property,
    Role, Trigger,
};
use chrono::Duration;

/// A realistic VEVENT as Radicale stores it (TZID, RRULE, ATTENDEE, VALARM).
const SAMPLE_ICS: &str = r#"BEGIN:VCALENDAR
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
DESCRIPTION:Agenda:\n1. Status\n2. Next Steps
RRULE:FREQ=WEEKLY;BYDAY=WE;COUNT=4
BEGIN:VALARM
ACTION:DISPLAY
TRIGGER:-PT30M
SUMMARY:Erinnerung
END:VALARM
END:VEVENT
END:VCALENDAR
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use icalendar::{Component, EventLike};
    use chrono::Timelike;

    #[test]
    fn spike_parse_radicale_event() {
        let cal: Calendar = SAMPLE_ICS.parse().expect("parse failed");
        let events: Vec<&Event> = cal.events().collect();
        assert_eq!(events.len(), 1, "expected 1 event");
        let ev = events[0];

        assert_eq!(
            ev.get_summary().map(|s| s.to_string()),
            Some("Projektbesprechung".into())
        );
        assert_eq!(
            ev.get_uid().map(|s| s.to_string()),
            Some("relay-test-001@aimighty".into())
        );
        assert_eq!(
            ev.get_location().map(|s| s.to_string()),
            Some("Büro, 3. Stock".into())
        );

        // DTSTART with TZID → UTC conversion
        let start = ev.get_start().expect("no start");
        let utc = match &start {
            icalendar::DatePerhapsTime::DateTime(dt) => dt.try_into_utc(),
            icalendar::DatePerhapsTime::Date(_) => None,
        };
        println!("[spike] DTSTART = {:?} → UTC = {:?}", start, utc);
        assert!(utc.is_some(), "TZID-Datum sollte nach UTC konvertierbar sein");
        // 15:00 Europe/Berlin (CEST, +2) = 13:00 UTC
        assert_eq!(utc.unwrap().hour(), 13);

        // Attendees
        let attendees = ev.get_attendees();
        assert_eq!(attendees.len(), 1);
        let a = &attendees[0];
        println!("[spike] ATTENDEE = {} partstat={:?}", a.cal_address, a.part_stat);
        assert_eq!(a.cal_address, "mailto:anna@example.com");
        assert_eq!(a.part_stat, Some(PartStat::NeedsAction));
        assert_eq!(a.cn.as_deref(), Some("Anna"));
        assert_eq!(a.rsvp, Some(true));

        // RRULE → RRuleSet expansion (&RRuleSet implements IntoIterator)
        let rrule = ev.get_recurrence().expect("no RRULE parsed");
        let occurrences: Vec<_> = (&rrule).into_iter().collect();
        println!("[spike] RRULE → {} occurrences: {:?}", occurrences.len(), occurrences);
        assert_eq!(occurrences.len(), 4, "COUNT=4 erwartet");
    }

    #[test]
    fn spike_build_event_with_attendee_and_tz() {
        let start = CalendarDateTime::from_ymd_hm_tzid(
            2026, 9, 1, 15, 0,
            chrono_tz::Europe::Berlin,
        )
        .expect("valid dt");
        let end = CalendarDateTime::from_ymd_hm_tzid(
            2026, 9, 1, 16, 0,
            chrono_tz::Europe::Berlin,
        )
        .expect("valid dt");

        let mut ev = Event::new();
        ev.uid("relay-new-002@aimighty")
            .summary("Neues Meeting")
            .description("Test")
            .starts(start)
            .ends(end)
            .class(Class::Public)
            .status(EventStatus::Confirmed)
            .sequence(0);

        ev.append_property(
            Property::new("ORGANIZER", "mailto:marc@example.com").add_parameter("CN", "Marc Bayer"),
        );
        ev.attendee(
            Attendee::new("mailto:anna@example.com".to_string())
                .partstat(PartStat::NeedsAction)
                .rsvp(true)
                .role(Role::ReqParticipant),
        );
        ev.alarm(Alarm::display("Erinnerung", Trigger::before_start(Duration::minutes(30))));

        let cal = Calendar::new().push(ev).done();
        let ics = cal.to_string();
        println!("[spike] built ICS:\n{}", ics);
        assert!(ics.contains("BEGIN:VEVENT"));
        assert!(ics.contains("TZID=Europe/Berlin"));
        assert!(ics.contains("ATTENDEE"));
        assert!(ics.contains("BEGIN:VALARM"));

        // Round-trip: parse what we built
        let parsed: Calendar = ics.parse().expect("round-trip parse failed");
        let p = parsed.events().next().unwrap();
        assert_eq!(p.get_summary().map(|s| s.to_string()), Some("Neues Meeting".into()));
        assert_eq!(p.get_attendees().len(), 1);
        let st = p.get_start().unwrap();
        let utc = match &st {
            icalendar::DatePerhapsTime::DateTime(dt) => dt.try_into_utc(),
            _ => None,
        };
        assert_eq!(utc.unwrap().hour(), 13, "15:00 CEST = 13:00 UTC");
    }
}
