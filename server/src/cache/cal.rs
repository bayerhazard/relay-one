//! Storage for CalDAV calendars and events (Phase 0).
//!
//! Thin rusqlite layer over the `calendars` / `events` / `event_attendees`
//! tables created in [`super::db::init_db`]. Returns plain, serialisable row
//! structs so the API layer never touches rusqlite directly.

use rusqlite::{params, Connection};

use crate::dav::{Calendar, IcsAttendee, IcsEvent};
use serde::{Deserialize, Serialize};

/// A calendar collection row (for API responses).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarRow {
    pub id: i64,
    pub url: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub last_sync_at: Option<String>,
}

/// An event row (for API responses), including its attendees.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRow {
    pub id: i64,
    pub calendar_id: i64,
    pub uid: String,
    pub url: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    /// Original DTSTART (RFC 3339 UTC). For recurring occurrences, use
    /// `occurrence_start` when present.
    pub start_at: String,
    pub end_at: Option<String>,
    pub all_day: bool,
    pub organizer: Option<String>,
    pub status: Option<String>,
    pub sequence: i64,
    pub rrule: Option<String>,
    pub alarms: i64,
    pub etag: Option<String>,
    pub attendees: Vec<IcsAttendee>,
    pub synced_at: String,
    /// Start of this specific occurrence (recurring events only), RFC 3339 UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence_start: Option<String>,
    /// End of this specific occurrence (recurring events only), RFC 3339 UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence_end: Option<String>,
    /// Raw ICS of the VEVENT (internal, used for recurrence expansion).
    #[serde(skip)]
    pub raw: String,
}

/// Insert or update a calendar by URL. Returns the calendar id.
pub fn upsert_calendar(conn: &Connection, cal: &Calendar) -> Result<i64, rusqlite::Error> {
    conn.execute(
        "INSERT INTO calendars (url, display_name, description, color, updated_at)
         VALUES (?1, ?2, ?3, ?4, datetime('now'))
         ON CONFLICT(url) DO UPDATE SET
            display_name = excluded.display_name,
            updated_at = datetime('now')",
        params![cal.url, cal.display_name, Option::<String>::None, Option::<String>::None],
    )?;
    let id: i64 = conn
        .query_row("SELECT id FROM calendars WHERE url = ?1", params![cal.url], |r| {
            r.get(0)
        })
        .unwrap_or(0);
    Ok(id)
}

pub fn list_calendars(conn: &Connection) -> Result<Vec<CalendarRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, url, display_name, description, color, last_sync_at
         FROM calendars ORDER BY display_name, url",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(CalendarRow {
                id: r.get(0)?,
                url: r.get(1)?,
                display_name: r.get(2)?,
                description: r.get(3)?,
                color: r.get(4)?,
                last_sync_at: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Mark a calendar as synced and store its sync token.
pub fn mark_calendar_synced(
    conn: &Connection,
    url: &str,
    sync_token: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE calendars SET sync_token = ?2, last_sync_at = datetime('now'), updated_at = datetime('now') WHERE url = ?1",
        params![url, sync_token],
    )?;
    Ok(())
}

/// Insert or update an event by (calendar_id, uid) and replace its attendees.
/// Returns the event id.
pub fn save_event(
    conn: &mut Connection,
    calendar_id: i64,
    ev: &IcsEvent,
) -> Result<i64, rusqlite::Error> {
    let tx = conn.transaction()?;
    {
        tx.execute(
            "INSERT INTO events (calendar_id, uid, url, summary, description, location,
                    start_at, end_at, all_day, organizer, status, sequence, rrule, alarms, ics_raw, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, datetime('now'))
             ON CONFLICT(calendar_id, uid) DO UPDATE SET
                url = excluded.url,
                summary = excluded.summary,
                description = excluded.description,
                location = excluded.location,
                start_at = excluded.start_at,
                end_at = excluded.end_at,
                all_day = excluded.all_day,
                organizer = excluded.organizer,
                status = excluded.status,
                sequence = excluded.sequence,
                rrule = excluded.rrule,
                alarms = excluded.alarms,
                ics_raw = excluded.ics_raw,
                updated_at = datetime('now')",
            params![
                calendar_id,
                ev.uid,
                ev.url,
                ev.summary,
                ev.description,
                ev.location,
                ev.start,
                ev.end,
                ev.all_day as i64,
                ev.organizer,
                ev.status,
                ev.sequence,
                ev.rrule,
                ev.alarms as i64,
                ev.raw,
            ],
        )?;
    }
    let id: i64 = tx
        .query_row(
            "SELECT id FROM events WHERE calendar_id = ?1 AND uid = ?2",
            params![calendar_id, ev.uid],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Replace attendees.
    tx.execute("DELETE FROM event_attendees WHERE event_id = ?1", params![id])?;
    {
        let mut ins = tx.prepare(
            "INSERT INTO event_attendees (event_id, email, name, part_stat, rsvp)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for a in &ev.attendees {
            ins.execute(params![id, a.email, a.name, a.part_stat, a.rsvp as i64])?;
        }
    }
    tx.commit()?;
    Ok(id)
}

/// List events, optionally filtered by calendar and a half-open time range
/// `[start, end)` compared against `start_at` (RFC 3339 UTC sorts lexically).
pub fn list_events(
    conn: &Connection,
    calendar_id: Option<i64>,
    start: Option<&str>,
    end: Option<&str>,
) -> Result<Vec<EventRow>, rusqlite::Error> {
    let mut sql = String::from(
        "SELECT id, calendar_id, uid, url, summary, description, location,
                start_at, end_at, all_day, organizer, status, sequence, rrule, alarms, ics_raw, synced_at
         FROM events WHERE 1=1",
    );
    let mut args: Vec<String> = Vec::new();
    if let Some(cid) = calendar_id {
        args.push(cid.to_string());
        sql.push_str(&format!(" AND calendar_id = ?{}", args.len()));
    }
    // Recurrence-aware range filter: a recurring event may have a DTSTART far
    // before the window but still produce occurrences inside it, so it is
    // included whenever it started before the window end. Non-recurring events
    // must start inside the window.
    match (start, end) {
        (Some(s), Some(e)) => {
            let base = args.len();
            args.push(e.to_string());
            args.push(s.to_string());
            args.push(e.to_string());
            let end_i = base + 1;
            let s_i = base + 2;
            let e2_i = base + 3;
            sql.push_str(&format!(
                " AND ((rrule IS NOT NULL AND start_at < ?{end_i}) \
                 OR (rrule IS NULL AND start_at >= ?{s_i} AND start_at < ?{e2_i}))"
            ));
        }
        (Some(s), None) => {
            args.push(s.to_string());
            sql.push_str(&format!(" AND start_at >= ?{}", args.len()));
        }
        (None, Some(e)) => {
            args.push(e.to_string());
            sql.push_str(&format!(" AND start_at < ?{}", args.len()));
        }
        (None, None) => {}
    }
    sql.push_str(" ORDER BY start_at");

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        args.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(param_refs.as_slice(), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, i64>(9)?,
                r.get::<_, Option<String>>(10)?,
                r.get::<_, Option<String>>(11)?,
                r.get::<_, i64>(12)?,
                r.get::<_, Option<String>>(13)?,
                r.get::<_, i64>(14)?,
                r.get::<_, String>(15)?,
                r.get::<_, String>(16)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let attendees = attendees_for(conn, row.0)?;
        let base = EventRow {
            id: row.0,
            calendar_id: row.1,
            uid: row.2,
            url: row.3,
            summary: row.4,
            description: row.5,
            location: row.6,
            start_at: row.7,
            end_at: row.8,
            all_day: row.9 != 0,
            organizer: row.10,
            status: row.11,
            sequence: row.12,
            rrule: row.13,
            alarms: row.14,
            etag: None,
            attendees,
            synced_at: row.16,
            occurrence_start: None,
            occurrence_end: None,
            raw: row.15,
        };

        // Expand recurring events into per-occurrence rows within the window.
        if base.rrule.is_some() {
            if let (Some(s), Some(e)) = (start, end) {
                match crate::dav::ics::expand_occurrences(&base.raw, s, e) {
                    Ok(occs) if !occs.is_empty() => {
                        for (os, oe) in occs {
                            let mut occ = base.clone();
                            occ.occurrence_start = Some(os);
                            occ.occurrence_end = oe;
                            occ.raw = String::new(); // save memory; not needed per-occurrence
                            out.push(occ);
                        }
                        continue;
                    }
                    // No occurrences in window (or parse error) → skip the series.
                    _ => continue,
                }
            }
        }
        out.push(base);
    }
    Ok(out)
}

fn attendees_for(conn: &Connection, event_id: i64) -> Result<Vec<IcsAttendee>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT email, name, part_stat, rsvp FROM event_attendees WHERE event_id = ?1",
    )?;
    let rows = stmt
        .query_map(params![event_id], |r| {
            Ok(IcsAttendee {
                email: r.get(0)?,
                name: r.get(1)?,
                part_stat: r.get(2)?,
                rsvp: r.get::<_, i64>(3)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_event(conn: &Connection, id: i64) -> Result<Option<EventRow>, rusqlite::Error> {
    let row: Option<(
        i64, i64, String, String, Option<String>, Option<String>, Option<String>,
        String, Option<String>, i64, Option<String>, Option<String>, i64,
        Option<String>, i64, String, String,
    )> = conn
        .query_row(
            "SELECT id, calendar_id, uid, url, summary, description, location,
                    start_at, end_at, all_day, organizer, status, sequence, rrule, alarms, ics_raw, synced_at
             FROM events WHERE id = ?1",
            params![id],
            |r| {
                Ok((
                    r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?,
                    r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?,
                    r.get(12)?, r.get(13)?, r.get(14)?, r.get(15)?, r.get(16)?,
                ))
            },
        )
        .ok();
    let Some(row) = row else { return Ok(None) };
    let attendees = attendees_for(conn, row.0)?;
    Ok(Some(EventRow {
        id: row.0,
        calendar_id: row.1,
        uid: row.2,
        url: row.3,
        summary: row.4,
        description: row.5,
        location: row.6,
        start_at: row.7,
        end_at: row.8,
        all_day: row.9 != 0,
        organizer: row.10,
        status: row.11,
        sequence: row.12,
        rrule: row.13,
        alarms: row.14,
        etag: None,
        attendees,
        synced_at: row.16,
        occurrence_start: None,
        occurrence_end: None,
        raw: row.15,
    }))
}

pub fn delete_event(conn: &Connection, id: i64) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM events WHERE id = ?1", params![id])?;
    Ok(())
}

/// Delete events by UID (used after a SYNC-COLLECTION reports deletions).
pub fn delete_events_by_uid(conn: &mut Connection, uids: &[String]) -> Result<(), rusqlite::Error> {
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare("DELETE FROM events WHERE uid = ?1")?;
        for uid in uids {
            stmt.execute(params![uid])?;
        }
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::db::init_db;

    fn temp_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        conn
    }

    fn sample_event(uid: &str) -> IcsEvent {
        IcsEvent {
            uid: uid.into(),
            url: format!("https://cal.example.com/Marc/Arbeit/{uid}.ics"),
            summary: Some("Test".into()),
            description: None,
            location: None,
            start: "2026-09-01T13:00:00Z".into(),
            end: Some("2026-09-01T14:00:00Z".into()),
            all_day: false,
            organizer: Some("marc@example.com".into()),
            attendees: vec![IcsAttendee {
                email: "anna@example.com".into(),
                name: Some("Anna".into()),
                part_stat: Some("NEEDS-ACTION".into()),
                rsvp: true,
            }],
            status: Some("CONFIRMED".into()),
            sequence: 0,
            rrule: None,
            alarms: 0,
            raw: "BEGIN:VCALENDAR...END:VCALENDAR".into(),
        }
    }

    #[test]
    fn test_calendar_and_event_roundtrip() {
        let mut conn = temp_db();
        let cal = Calendar {
            href: "/Marc/Arbeit/".into(),
            display_name: Some("Arbeit".into()),
            url: "https://cal.example.com/Marc/Arbeit/".into(),
        };
        let cid = upsert_calendar(&mut conn, &cal).unwrap();
        assert!(cid > 0);

        let eid = save_event(&mut conn, cid, &sample_event("uid-1")).unwrap();
        assert!(eid > 0);

        let events = list_events(&conn, Some(cid), None, None).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].uid, "uid-1");
        assert_eq!(events[0].attendees.len(), 1);
        assert_eq!(events[0].attendees[0].email, "anna@example.com");

        // Upsert same uid → still one row.
        save_event(&mut conn, cid, &sample_event("uid-1")).unwrap();
        assert_eq!(list_events(&conn, Some(cid), None, None).unwrap().len(), 1);

        // Time-range filter.
        let in_range = list_events(&conn, Some(cid), Some("2026-09-01T00:00:00Z"), Some("2026-09-02T00:00:00Z")).unwrap();
        assert_eq!(in_range.len(), 1);
        let out_of_range = list_events(&conn, Some(cid), Some("2026-10-01T00:00:00Z"), None).unwrap();
        assert_eq!(out_of_range.len(), 0);

        delete_event(&mut conn, eid).unwrap();
        assert!(get_event(&conn, eid).unwrap().is_none());
    }

    #[test]
    fn test_delete_by_uid_and_sync_token() {
        let mut conn = temp_db();
        let cal = Calendar {
            href: "/Marc/Privat/".into(),
            display_name: Some("Privat".into()),
            url: "https://cal.example.com/Marc/Privat/".into(),
        };
        let cid = upsert_calendar(&mut conn, &cal).unwrap();
        save_event(&mut conn, cid, &sample_event("uid-a")).unwrap();
        save_event(&mut conn, cid, &sample_event("uid-b")).unwrap();
        assert_eq!(list_events(&conn, Some(cid), None, None).unwrap().len(), 2);

        // Incremental-sync deletion path.
        delete_events_by_uid(&mut conn, &["uid-a".to_string()]).unwrap();
        let remaining = list_events(&conn, Some(cid), None, None).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].uid, "uid-b");

        // Sync-token bookkeeping: last_sync_at gets stamped (matched by full URL).
        mark_calendar_synced(&mut conn, "https://cal.example.com/Marc/Privat/", "0-1234").unwrap();
        let cals = list_calendars(&conn).unwrap();
        assert!(cals.iter().find(|c| c.id == cid).unwrap().last_sync_at.is_some());
    }
}
