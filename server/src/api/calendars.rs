//! CalDAV calendar + event endpoints (Phase 0).
//!
//! Settings, sync, calendar listing and event CRUD. Reads use `with_db`
//! (`&Connection`); the few transactional writes take `&mut Connection` via
//! `get_db(..).as_mut()`.

use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::api::ApiError;
use crate::cache;
use crate::crypto;
use crate::dav::{CalDavClient, CalDavSettings, IcsAttendee, IcsEvent};
use crate::db::{get_db, with_db};
use crate::AppState;

use super::{ApiResult, ok};

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// `GET /api/v1/calendars/settings` — stored CalDAV connection settings.
pub async fn get_caldav_settings(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let json = with_db(&state, |conn| {
        cache::settings::get_setting(conn, "caldav_settings").map_err(|e| e.to_string())
    })?;
    match json {
        Some(raw) => {
            let mut settings: CalDavSettings =
                serde_json::from_str(&raw).map_err(|e| ApiError(format!("CalDAV parse: {e}")))?;
            settings.password =
                crypto::decrypt(&settings.password).unwrap_or(settings.password);
            Ok(Json(serde_json::json!({
                "url": settings.url, "username": settings.username, "password": settings.password,
                "sync_interval_minutes": settings.sync_interval_minutes,
            })))
        }
        None => Ok(Json(serde_json::json!({
            "url": "", "username": "", "password": "", "sync_interval_minutes": 30,
        }))),
    }
}

#[derive(Deserialize)]
pub struct CalDavSettingsRequest {
    pub url: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub sync_interval_minutes: Option<u64>,
}

/// `POST /api/v1/calendars/settings` — save the CalDAV connection settings.
pub async fn set_caldav_settings(
    State(state): State<AppState>,
    Json(req): Json<CalDavSettingsRequest>,
) -> ApiResult<serde_json::Value> {
    let encrypted_pw =
        crypto::encrypt(&req.password).unwrap_or_else(|_| req.password.clone());
    let settings = CalDavSettings {
        url: req.url.clone(),
        username: req.username.clone(),
        password: encrypted_pw,
        sync_interval_minutes: req.sync_interval_minutes.unwrap_or(30),
    };
    let raw = serde_json::to_string(&settings).map_err(|e| ApiError(e.to_string()))?;
    with_db(&state, |conn| {
        cache::settings::set_setting(conn, "caldav_settings", &raw).map_err(|e| e.to_string())
    })?;
    let mut live = settings.clone();
    live.password = req.password.clone();
    *state.caldav_settings.write() = Some(live);
    tracing::info!("CalDAV-Einstellungen gespeichert: {}", req.url);
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// Sync
// ---------------------------------------------------------------------------

/// `POST /api/v1/calendars/sync` — trigger a manual CalDAV sync.
pub async fn sync_caldav(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    do_caldav_sync(&state).await
}

/// Shared sync logic (manual endpoint + background scheduler).
pub async fn do_caldav_sync(state: &AppState) -> ApiResult<serde_json::Value> {
    let settings = state.caldav_settings.read().clone();
    let Some(settings) = settings else {
        return Err(ApiError("CalDAV nicht konfiguriert".into()));
    };
    let client = CalDavClient::new(settings);

    let token = state.caldav_sync_token.read().clone();
    let (events, new_token) = if token.is_empty() {
        client.fetch_all_events().await.map_err(ApiError)?
    } else {
        match client.sync_incremental(&token).await {
            Ok((added, deleted, tok)) => {
                if !deleted.is_empty() {
                    if let Ok(mut guard) = get_db(state) {
                        if let Some(conn) = guard.as_mut() {
                            let _ = crate::cache::cal::delete_events_by_uid(conn, &deleted);
                        }
                    }
                }
                (added, tok)
            }
            Err(e) => {
                tracing::warn!("CalDAV: inkrementell fehlgeschlagen, Full-Sync: {e}");
                client.fetch_all_events().await.map_err(ApiError)?
            }
        }
    };

    // Persist calendars + events.
    let calendars = client.discover_calendars().await.unwrap_or_default();
    let mut saved = 0usize;
    if let Ok(mut guard) = get_db(state) {
        if let Some(conn) = guard.as_mut() {
            for cal in &calendars {
                if let Ok(cid) = crate::cache::cal::upsert_calendar(conn, cal) {
                    let _ = crate::cache::cal::mark_calendar_synced(conn, &cal.url, &new_token);
                    for ev in events.iter().filter(|e| e.url.starts_with(&cal.url)) {
                        if crate::cache::cal::save_event(conn, cid, ev).is_ok() {
                            saved += 1;
                        }
                    }
                }
            }
        }
    }

    *state.caldav_sync_token.write() = new_token.clone();
    let _ = with_db(state, |conn| {
        cache::settings::set_setting(conn, "caldav_sync_token", &new_token).map_err(|e| e.to_string())
    });

    tracing::info!("CalDAV-Sync: {} Events gespeichert", saved);
    Ok(Json(serde_json::json!({ "ok": true, "synced": saved })))
}

// ---------------------------------------------------------------------------
// Calendars
// ---------------------------------------------------------------------------

/// `GET /api/v1/calendars` — list synced calendar collections.
pub async fn list_calendars(State(state): State<AppState>) -> ApiResult<Vec<crate::cache::cal::CalendarRow>> {
    ok(with_db(&state, |conn| {
        crate::cache::cal::list_calendars(conn).map_err(|e| e.to_string())
    }))
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ListEventsQuery {
    #[serde(default)]
    pub calendar_id: Option<i64>,
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub end: Option<String>,
}

/// `GET /api/v1/events` — list events, optionally filtered.
pub async fn list_events(
    State(state): State<AppState>,
    Query(q): Query<ListEventsQuery>,
) -> ApiResult<Vec<crate::cache::cal::EventRow>> {
    ok(with_db(&state, |conn| {
        crate::cache::cal::list_events(conn, q.calendar_id, q.start.as_deref(), q.end.as_deref())
            .map_err(|e| e.to_string())
    }))
}

/// `GET /api/v1/events/:id` — fetch a single event.
pub async fn get_event(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<crate::cache::cal::EventRow> {
    let row = with_db(&state, |conn| {
        crate::cache::cal::get_event(conn, id).map_err(|e| e.to_string())
    })
    .map_err(ApiError)?;
    let row = row.ok_or_else(|| ApiError("Event nicht gefunden".into()))?;
    Ok(Json(row))
}

#[derive(Deserialize)]
pub struct CreateEventRequest {
    pub calendar_id: i64,
    pub summary: String,
    /// RFC 3339 (UTC) start.
    pub start: String,
    #[serde(default)]
    pub end: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub organizer: Option<String>,
    #[serde(default)]
    pub attendees: Vec<IcsAttendee>,
    #[serde(default)]
    pub rrule: Option<String>,
    #[serde(default)]
    pub reminder_minutes: Option<u32>,
}

fn parse_dt(s: &str) -> Result<DateTime<Utc>, ApiError> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| ApiError(format!("Ungültiges Datum '{s}': {e}")))
}

/// `POST /api/v1/events` — create an event (PUT to the CalDAV server + DB).
pub async fn create_event(
    State(state): State<AppState>,
    Json(req): Json<CreateEventRequest>,
) -> ApiResult<crate::cache::cal::EventRow> {
    let settings = state.caldav_settings.read().clone().ok_or_else(|| {
        ApiError("CalDAV nicht konfiguriert — Event kann nicht angelegt werden".into())
    })?;
    let client = CalDavClient::new(settings);

    let start = parse_dt(&req.start)?;
    let end = match &req.end {
        Some(e) => Some(parse_dt(e)?),
        None => Some(start + chrono::Duration::hours(1)),
    };

    let uid = format!("relay-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let ics = crate::dav::ics::build_event(
        &uid,
        &req.summary,
        start,
        end,
        req.description.as_deref(),
        req.location.as_deref(),
        req.organizer.as_deref(),
        &req.attendees,
        req.rrule.as_deref(),
        req.reminder_minutes,
    )
    .map_err(ApiError)?;

    // Resolve the calendar URL.
    let cal_url = with_db(&state, |conn| {
        conn.query_row(
            "SELECT url FROM calendars WHERE id = ?1",
            rusqlite::params![req.calendar_id],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())
    })
    .map_err(ApiError)?;

    let url = client.create_event(&cal_url, &ics).await.map_err(ApiError)?;
    let mut ev = crate::dav::ics::parse_event(&ics).map_err(ApiError)?;
    ev.url = url.clone();

    let row = save_event_row(&state, req.calendar_id, &ev)?;
    Ok(Json(row))
}

#[derive(Deserialize)]
pub struct UpdateEventRequest {
    pub summary: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub attendees: Option<Vec<IcsAttendee>>,
    pub rrule: Option<String>,
    pub reminder_minutes: Option<u32>,
}

/// `PUT /api/v1/events/:id` — update an event (PUT to the server + DB).
pub async fn update_event(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateEventRequest>,
) -> ApiResult<crate::cache::cal::EventRow> {
    let existing = get_event_inner(&state, id)?;

    let start = match &req.start {
        Some(s) => parse_dt(s)?,
        None => parse_dt(&existing.start_at)?,
    };
    let end = match &req.end {
        Some(e) => Some(parse_dt(e)?),
        None => existing.end_at.as_deref().map(parse_dt).transpose()?,
    };
    let summary = req.summary.clone().unwrap_or_else(|| existing.summary.clone().unwrap_or_default());
    let attendees = req.attendees.clone().unwrap_or_else(|| existing.attendees.clone());
    let uid = existing.uid.clone();

    let ics = crate::dav::ics::build_event(
        &uid,
        &summary,
        start,
        end,
        req.description.as_deref().or(existing.description.as_deref()),
        req.location.as_deref().or(existing.location.as_deref()),
        existing.organizer.as_deref(),
        &attendees,
        req.rrule.as_deref().or(existing.rrule.as_deref()),
        req.reminder_minutes,
    )
    .map_err(ApiError)?;

    let settings = state.caldav_settings.read().clone();
    if let Some(settings) = settings {
        let client = CalDavClient::new(settings);
        client.update_event(&existing.url, &ics).await.map_err(ApiError)?;
    }

    let mut ev = crate::dav::ics::parse_event(&ics).map_err(ApiError)?;
    ev.url = existing.url.clone();
    let row = save_event_row(&state, existing.calendar_id, &ev)?;
    Ok(Json(row))
}

/// `DELETE /api/v1/events/:id` — delete an event (DELETE on server + DB).
pub async fn delete_event(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<serde_json::Value> {
    let existing = get_event_inner(&state, id)?;
    let settings = state.caldav_settings.read().clone();
    if let Some(settings) = settings {
        let client = CalDavClient::new(settings);
        client.delete_event(&existing.url).await.map_err(ApiError)?;
    }
    if let Ok(mut guard) = get_db(&state) {
        if let Some(conn) = guard.as_mut() {
            crate::cache::cal::delete_event(conn, id).map_err(|e| ApiError(e.to_string()))?;
        }
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// iMIP (Phase 2)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct InviteEventRequest {
    pub account_id: u32,
    pub attendees: Vec<IcsAttendee>,
}

/// `POST /api/v1/calendars/events/:id/invite` — send iMIP invitations
/// (`METHOD:REQUEST`) to the given attendees via the account's SMTP.
pub async fn invite_event(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<InviteEventRequest>,
) -> ApiResult<serde_json::Value> {
    let sent = crate::imip::outbound::send_invitation(&state, id, req.account_id, &req.attendees)
        .await
        .map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "ok": true, "sent": sent })))
}

/// `POST /api/v1/calendars/events/:id/rsvp` — send an RSVP reply
/// (`METHOD:REPLY`) for a received invitation.
pub async fn rsvp_event(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<InviteEventRequest>,
) -> ApiResult<serde_json::Value> {
    // The decision is carried in the (single) attendee's part_stat.
    let decision = req
        .attendees
        .first()
        .and_then(|a| a.part_stat.clone())
        .unwrap_or_else(|| "ACCEPTED".into());
    crate::imip::outbound::send_rsvp(&state, id, req.account_id, &decision)
        .await
        .map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "ok": true, "decision": decision })))
}

// ---------------------------------------------------------------------------
// Conflict detection (Phase 2.4)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ConflictQuery {
    pub start: String,
    pub end: String,
    #[serde(default)]
    pub calendar_id: Option<i64>,
    #[serde(default)]
    pub exclude_id: Option<i64>,
}

/// `GET /api/v1/calendars/conflicts` — events overlapping `[start, end)`.
pub async fn find_event_conflicts(
    State(state): State<AppState>,
    Query(q): Query<ConflictQuery>,
) -> ApiResult<Vec<crate::cache::cal::EventRow>> {
    ok(with_db(&state, |conn| {
        crate::cache::cal::find_conflicts(conn, q.calendar_id, &q.start, &q.end, q.exclude_id)
            .map_err(|e| e.to_string())
    }))
}

// ---------------------------------------------------------------------------
// ICS import / export
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ImportRequest {
    pub calendar_id: i64,
    /// Raw ICS text (VCALENDAR, may contain multiple VEVENTs).
    pub ics: String,
}

/// `POST /api/v1/events/import` — import events from an ICS body into a
/// calendar. Each VEVENT is PUT to the CalDAV server (when configured) and
/// stored locally. Returns the number of imported events.
pub async fn import_events(
    State(state): State<AppState>,
    Json(req): Json<ImportRequest>,
) -> ApiResult<serde_json::Value> {
    let events = crate::dav::ics::parse_events(&req.ics).map_err(ApiError)?;
    if events.is_empty() {
        return Ok(Json(serde_json::json!({ "ok": true, "imported": 0 })));
    }

    let cal_url = with_db(&state, |conn| {
        conn.query_row(
            "SELECT url FROM calendars WHERE id = ?1",
            rusqlite::params![req.calendar_id],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())
    })
    .map_err(ApiError)?;

    let client = state
        .caldav_settings
        .read()
        .clone()
        .map(CalDavClient::new);

    let mut imported = 0usize;
    for ev in &events {
        // Store the faithful single-event block (not the whole calendar).
        let mut ev2 = ev.clone();
        ev2.raw = crate::dav::ics::extract_vevent(&req.ics, &ev.uid)
            .unwrap_or_else(|| ev.raw.clone());
        ev2.url = String::new();

        if let Some(client) = &client {
            if let Ok(url) = client.create_event(&cal_url, &ev2.raw).await {
                ev2.url = url;
            }
        }
        if let Ok(mut guard) = get_db(&state) {
            if let Some(conn) = guard.as_mut() {
                if crate::cache::cal::save_event(conn, req.calendar_id, &ev2).is_ok() {
                    imported += 1;
                }
            }
        }
    }
    tracing::info!("ICS-Import: {} Events nach Kalender {} importiert", imported, req.calendar_id);
    Ok(Json(serde_json::json!({ "ok": true, "imported": imported })))
}

/// `GET /api/v1/events/:id/ics` — the event's raw ICS (for download/export).
pub async fn get_event_ics(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<serde_json::Value> {
    let row = get_event_inner(&state, id)?;
    let filename = format!(
        "{}.ics",
        row.summary.as_deref().unwrap_or("termin").replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "-")
    );
    Ok(Json(serde_json::json!({ "ics": row.raw, "filename": filename })))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_event_inner(
    state: &AppState,
    id: i64,
) -> Result<crate::cache::cal::EventRow, ApiError> {
    let row = with_db(state, |conn| {
        crate::cache::cal::get_event(conn, id).map_err(|e| e.to_string())
    })
    .map_err(ApiError)?;
    row.ok_or_else(|| ApiError("Event nicht gefunden".into()))
}

/// Persist an event + attendees, returning the stored row.
fn save_event_row(
    state: &AppState,
    calendar_id: i64,
    ev: &IcsEvent,
) -> Result<crate::cache::cal::EventRow, ApiError> {
    let eid = {
        let mut guard = get_db(state).map_err(ApiError)?;
        let conn = guard.as_mut().ok_or_else(|| ApiError("DB nicht initialisiert".into()))?;
        crate::cache::cal::save_event(conn, calendar_id, ev).map_err(|e| ApiError(e.to_string()))?
    };
    let row = with_db(state, |conn| {
        crate::cache::cal::get_event(conn, eid).map_err(|e| e.to_string())
    })
    .map_err(ApiError)?
        .ok_or_else(|| ApiError("Event nach Save nicht gefunden".into()))?;
    Ok(row)
}
