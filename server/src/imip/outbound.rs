//! iMIP outbound (Phase 2.1): send invitations (`METHOD:REQUEST`) and RSVP
//! replies (`METHOD:REPLY`) via SMTP with an ICS attachment.

use base64::Engine;
use chrono::{DateTime, Utc};

use crate::dav::ics::{build_event_full, IcsAttendee};
use crate::dav::CalDavClient;
use crate::db::{get_db, with_db};
use crate::smtp::client::EmailAttachment;
use crate::AppState;

fn parse_dt(s: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| format!("Ungültiges Datum '{s}': {e}"))
}

/// Send an iMIP invitation (`METHOD:REQUEST`) for an event to the given
/// attendees. Returns the number of invitations sent.
pub async fn send_invitation(
    state: &AppState,
    event_id: i64,
    account_id: u32,
    attendees: &[IcsAttendee],
) -> Result<usize, String> {
    if attendees.is_empty() {
        return Err("Keine Teilnehmer angegeben".into());
    }

    // 1. Load the event.
    let row = with_db(state, |conn| {
        crate::cache::cal::get_event(conn, event_id).map_err(|e| e.to_string())
    })?
    .ok_or_else(|| "Event nicht gefunden".to_string())?;

    // 2. Resolve the SMTP client + the organizer (the user's own sender address).
    let smtp = state
        .smtp_clients
        .read()
        .get(&account_id)
        .cloned()
        .ok_or_else(|| "SMTP-Client nicht gefunden — Konto prüfen".to_string())?;
    let organizer = smtp.config().sender_email.clone();

    // 3. Build the METHOD:REQUEST ICS (organizer = user, attendees = invitees).
    let start = parse_dt(&row.start_at)?;
    let end = row.end_at.as_deref().map(parse_dt).transpose()?;
    let ics = build_event_full(
        &row.uid,
        row.summary.as_deref().unwrap_or("Termin"),
        start,
        end,
        row.description.as_deref(),
        row.location.as_deref(),
        Some(&organizer),
        attendees,
        row.rrule.as_deref(),
        None,
        Some("REQUEST"),
        row.sequence,
    )?;

    // 4. Send to each attendee (ICS attachment + text body).
    let attachment = ics_attachment(&ics, &row.uid, "REQUEST");
    let title = row.summary.as_deref().unwrap_or("Termin");
    let subject = format!("Einladung: {title}");
    let body_text = format!(
        "Du wurdest zum Termin \"{title}\" eingeladen.\n\n\
         Details und Bestätigungsmöglichkeit findest du im angehängten Kalendereintrag."
    );
    let mut sent = 0usize;
    for a in attendees {
        let to = vec![(a.name.as_deref().unwrap_or(""), a.email.as_str())];
        smtp.send(
            to,
            vec![],
            vec![],
            &subject,
            &body_text,
            None,
            None,
            None,
            std::slice::from_ref(&attachment),
        )
        .await
        .map_err(|e| format!("SMTP-Versand an {} fehlgeschlagen: {e}", a.email))?;
        sent += 1;
    }

    // 5. Persist: invitation rows + event (attendees) on CalDAV + local DB.
    upsert_invitations(state, &row.uid, &organizer, attendees, row.sequence)?;
    persist_event(state, &row, &ics).await?;

    Ok(sent)
}

/// Build a `text/calendar` ICS attachment from raw ICS bytes.
pub fn ics_attachment(ics: &str, uid: &str, method: &str) -> EmailAttachment {
    let b64 = base64::engine::general_purpose::STANDARD.encode(ics.as_bytes());
    EmailAttachment {
        filename: format!("einladung-{uid}.ics"),
        content: b64,
        content_type: format!("text/calendar; method={method}"),
        size: ics.len(),
    }
}

/// Upsert one `invitations` row per attendee (method, status, sequence).
fn upsert_invitations(
    state: &AppState,
    event_uid: &str,
    organizer: &str,
    attendees: &[IcsAttendee],
    sequence: i64,
) -> Result<(), String> {
    let mut guard = get_db(state).map_err(|e| e.to_string())?;
    let conn = guard.as_mut().ok_or_else(|| "DB nicht initialisiert".to_string())?;
    for a in attendees {
        let part_stat = a.part_stat.clone().unwrap_or_else(|| "NEEDS-ACTION".into());
        conn.execute(
            "INSERT INTO invitations (event_uid, organizer, attendee_email, method, status, sequence, updated_at)
             VALUES (?1, ?2, ?3, 'REQUEST', ?4, ?5, datetime('now'))
             ON CONFLICT(event_uid, attendee_email) DO UPDATE SET
                organizer = excluded.organizer,
                method = 'REQUEST',
                status = excluded.status,
                sequence = excluded.sequence,
                updated_at = datetime('now')",
            rusqlite::params![event_uid, organizer, a.email, part_stat, sequence],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Update the event on the CalDAV server (so attendees persist server-side) and
/// save it locally (re-parsing the built ICS captures the attendees).
async fn persist_event(
    state: &AppState,
    row: &crate::cache::cal::EventRow,
    ics: &str,
) -> Result<(), String> {
    let settings = state.caldav_settings.read().clone();
    if let Some(settings) = settings {
        let client = CalDavClient::new(settings);
        if !row.url.is_empty() {
            client
                .update_event(&row.url, ics)
                .await
                .map_err(|e| format!("CalDAV-Update fehlgeschlagen: {e}"))?;
        }
    }
    let mut ev = crate::dav::ics::parse_event(ics).map_err(|e| e.to_string())?;
    ev.url = row.url.clone();
    let mut guard = get_db(state).map_err(|e| e.to_string())?;
    let conn = guard.as_mut().ok_or_else(|| "DB nicht initialisiert".to_string())?;
    crate::cache::cal::save_event(conn, row.calendar_id, &ev).map_err(|e| e.to_string())?;
    Ok(())
}

/// Build + send an RSVP reply (`METHOD:REPLY`) for an event. `decision` is
/// `ACCEPTED` / `DECLINED` / `TENTATIVE`. The reply goes to the original
/// organizer; the local invitation + attendee status is updated.
pub async fn send_rsvp(
    state: &AppState,
    event_id: i64,
    account_id: u32,
    decision: &str,
) -> Result<(), String> {
    let row = with_db(state, |conn| {
        crate::cache::cal::get_event(conn, event_id).map_err(|e| e.to_string())
    })?
    .ok_or_else(|| "Event nicht gefunden".to_string())?;

    let smtp = state
        .smtp_clients
        .read()
        .get(&account_id)
        .cloned()
        .ok_or_else(|| "SMTP-Client nicht gefunden — Konto prüfen".to_string())?;
    let self_email = smtp.config().sender_email.clone();
    let organizer = row
        .organizer
        .clone()
        .filter(|o| !o.is_empty())
        .ok_or_else(|| "Kein Organizer im Event — RSVP nicht möglich".to_string())?;

    // The RSVPing attendee (self) carries the decision.
    let self_name = smtp.config().sender_name.clone();
    let attendees = vec![IcsAttendee {
        email: self_email.clone(),
        name: if self_name.is_empty() { None } else { Some(self_name) },
        part_stat: Some(decision.to_string()),
        rsvp: true,
    }];

    let start = parse_dt(&row.start_at)?;
    let end = row.end_at.as_deref().map(parse_dt).transpose()?;
    let ics = build_event_full(
        &row.uid,
        row.summary.as_deref().unwrap_or("Termin"),
        start,
        end,
        row.description.as_deref(),
        row.location.as_deref(),
        Some(&organizer),
        &attendees,
        row.rrule.as_deref(),
        None,
        Some("REPLY"),
        row.sequence + 1,
    )?;

    let attachment = ics_attachment(&ics, &row.uid, "REPLY");
    let title = row.summary.as_deref().unwrap_or("Termin");
    let subject = format!("Re: Einladung: {title}");
    let body_text =
        format!("Ich habe die Einladung zu \"{title}\" erhalten und antworte: {decision}.");
    let to = vec![("", organizer.as_str())];
    smtp.send(
        to,
        vec![],
        vec![],
        &subject,
        &body_text,
        None,
        None,
        None,
        std::slice::from_ref(&attachment),
    )
    .await
    .map_err(|e| format!("SMTP-RSVP-Versand an {organizer} fehlgeschlagen: {e}"))?;

    update_rsvp_status(state, &row.uid, &self_email, decision)?;
    Ok(())
}

/// Update the local invitation + attendee status after an outgoing RSVP.
fn update_rsvp_status(
    state: &AppState,
    event_uid: &str,
    attendee_email: &str,
    decision: &str,
) -> Result<(), String> {
    let mut guard = get_db(state).map_err(|e| e.to_string())?;
    let conn = guard.as_mut().ok_or_else(|| "DB nicht initialisiert".to_string())?;
    conn.execute(
        "UPDATE invitations SET status = ?3, method = 'REPLY', updated_at = datetime('now')
         WHERE event_uid = ?1 AND attendee_email = ?2",
        rusqlite::params![event_uid, attendee_email, decision],
    )
    .map_err(|e| e.to_string())?;
    let eid: i64 = conn
        .query_row(
            "SELECT id FROM events WHERE uid = ?1",
            rusqlite::params![event_uid],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if eid > 0 {
        conn.execute(
            "UPDATE event_attendees SET part_stat = ?3 WHERE event_id = ?1 AND email = ?2",
            rusqlite::params![eid, attendee_email, decision],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ics_attachment_base64_roundtrip() {
        let ics = "BEGIN:VCALENDAR\nVERSION:2.0\nEND:VCALENDAR\n";
        let att = ics_attachment(ics, "abc-123", "REQUEST");
        assert_eq!(att.filename, "einladung-abc-123.ics");
        assert_eq!(att.content_type, "text/calendar; method=REQUEST");
        assert_eq!(att.size, ics.len());
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&att.content)
            .expect("valid base64");
        assert_eq!(std::str::from_utf8(&decoded).unwrap(), ics);
    }
}
