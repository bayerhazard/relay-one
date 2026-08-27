//! iMIP inbound (Phase 2.2): process incoming ICS attachments
//! (`METHOD:REQUEST`/`REPLY`/`CANCEL`) detected during IMAP sync.

use crate::dav::ics::{self, IcsEvent};
use crate::dav::CalDavClient;
use crate::db::get_db;
use crate::AppState;

/// The user's own email (first SMTP account), used to tell invitations *to me*
/// apart from echoes of my own events.
pub fn self_email(state: &AppState) -> Option<String> {
    state
        .smtp_clients
        .read()
        .values()
        .next()
        .map(|c| c.config().sender_email.clone())
}

/// Process an incoming ICS body (from a mail attachment). Returns a short
/// description of what happened (for logging / testing).
pub async fn process_inbound_ics(state: &AppState, ics: &str) -> Result<String, String> {
    let ev = ics::parse_event(ics).map_err(|e| format!("ICS nicht parsbar: {e}"))?;
    let method = ev.method.clone().unwrap_or_else(|| "REQUEST".into());
    match method.to_uppercase().as_str() {
        "REQUEST" => handle_request(state, ics, &ev).await,
        "REPLY" => handle_reply(state, &ev).await,
        "CANCEL" => handle_cancel(state, &ev).await,
        other => Ok(format!("unbekannte METHOD '{other}' ignoriert")),
    }
}

/// A `METHOD:REQUEST` from someone else → add the event to my calendar + queue
/// it as a pending invitation.
async fn handle_request(
    state: &AppState,
    ics: &str,
    ev: &IcsEvent,
) -> Result<String, String> {
    // Ignore echoes of my own events (organizer == me).
    let me = self_email(state);
    if let (Some(me), Some(org)) = (&me, &ev.organizer) {
        if me == org {
            return Ok("eigenes Event ignoriert (Organizer = ich)".into());
        }
    }

    // Target calendar: the first synced calendar.
    let (cal_id, cal_url) = crate::db::with_db(state, |conn| {
        conn.query_row(
            "SELECT id, url FROM calendars ORDER BY id LIMIT 1",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .map_err(|_| "Kein Kalender für Einladungen gefunden".to_string())
    })?;

    // Clean single-VEVENT block (no METHOD) for a faithful CalDAV PUT.
    let block = ics::extract_vevent(ics, &ev.uid).unwrap_or_else(|| ev.raw.clone());

    // Idempotency: CalDAV create is not idempotent (a re-fetch would create a
    // duplicate). Only PUT when the event is not already tracked locally;
    // otherwise reuse the existing server URL.
    let already_known = crate::db::with_db(state, |conn| {
        let n: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE uid = ?1",
                rusqlite::params![ev.uid],
                |r| r.get(0),
            )
            .unwrap_or(0);
        Ok(n > 0)
    })
    .unwrap_or(false);

    let mut url = String::new();
    if !already_known {
        let settings = state.caldav_settings.read().clone();
        if let Some(settings) = settings {
            let client = CalDavClient::new(settings);
            match client.create_event(&cal_url, &block).await {
                Ok(u) => url = u,
                Err(e) => tracing::warn!("iMIP: CalDAV-PUT der Einladung fehlgeschlagen: {e}"),
            }
        }
    } else {
        url = crate::db::with_db(state, |conn| {
            conn.query_row(
                "SELECT url FROM events WHERE uid = ?1",
                rusqlite::params![ev.uid],
                |r| r.get::<_, String>(0),
            )
            .map_err(|e| e.to_string())
        })
        .unwrap_or_default();
    }

    let mut stored = ev.clone();
    stored.url = url;
    stored.raw = block;
    let mut guard = get_db(state).map_err(|e| e.to_string())?;
    let conn = guard.as_mut().ok_or_else(|| "DB nicht initialisiert".to_string())?;
    crate::cache::cal::save_event(conn, cal_id, &stored).map_err(|e| e.to_string())?;

    // Queue the invitation (attendee = me, pending).
    let me_addr = me.unwrap_or_default();
    conn.execute(
        "INSERT INTO invitations (event_uid, organizer, attendee_email, method, status, sequence, updated_at)
         VALUES (?1, ?2, ?3, 'REQUEST', 'NEEDS-ACTION', ?4, datetime('now'))
         ON CONFLICT(event_uid, attendee_email) DO UPDATE SET
            organizer = excluded.organizer, sequence = excluded.sequence, updated_at = datetime('now')",
        rusqlite::params![ev.uid, ev.organizer, me_addr, ev.sequence],
    )
    .map_err(|e| e.to_string())?;

    Ok(format!("Einladung empfangen: {}", ev.summary.as_deref().unwrap_or("Termin")))
}

/// A `METHOD:REPLY` from an attendee I invited → update their status.
async fn handle_reply(state: &AppState, ev: &IcsEvent) -> Result<String, String> {
    // The replier is the attendee carrying RSVP=TRUE (or the first attendee).
    let replier = ev
        .attendees
        .iter()
        .find(|a| a.rsvp)
        .or_else(|| ev.attendees.first());
    let Some(replier) = replier else {
        return Ok("REPLY ohne Teilnehmer ignoriert".into());
    };
    let part_stat = replier
        .part_stat
        .clone()
        .unwrap_or_else(|| "NEEDS-ACTION".into());

    let mut guard = get_db(state).map_err(|e| e.to_string())?;
    let conn = guard.as_mut().ok_or_else(|| "DB nicht initialisiert".to_string())?;
    conn.execute(
        "INSERT INTO invitations (event_uid, organizer, attendee_email, method, status, sequence, updated_at)
         VALUES (?1, ?2, ?3, 'REPLY', ?4, ?5, datetime('now'))
         ON CONFLICT(event_uid, attendee_email) DO UPDATE SET
            status = excluded.status, method = 'REPLY', sequence = excluded.sequence, updated_at = datetime('now')",
        rusqlite::params![ev.uid, ev.organizer, replier.email, part_stat, ev.sequence],
    )
    .map_err(|e| e.to_string())?;
    let eid: i64 = conn
        .query_row(
            "SELECT id FROM events WHERE uid = ?1",
            rusqlite::params![ev.uid],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if eid > 0 {
        conn.execute(
            "UPDATE event_attendees SET part_stat = ?3 WHERE event_id = ?1 AND email = ?2",
            rusqlite::params![eid, replier.email, part_stat],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(format!("RSVP: {} → {part_stat}", replier.email))
}

/// A `METHOD:CANCEL` → mark the event cancelled.
async fn handle_cancel(state: &AppState, ev: &IcsEvent) -> Result<String, String> {
    let mut guard = get_db(state).map_err(|e| e.to_string())?;
    let conn = guard.as_mut().ok_or_else(|| "DB nicht initialisiert".to_string())?;
    let n = conn
        .execute(
            "UPDATE events SET status = 'CANCELLED', updated_at = datetime('now') WHERE uid = ?1",
            rusqlite::params![ev.uid],
        )
        .map_err(|e| e.to_string())?;
    Ok(format!("Termin abgesagt ({n} aktualisiert): {}", ev.summary.as_deref().unwrap_or("Termin")))
}
