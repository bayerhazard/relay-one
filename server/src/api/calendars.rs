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

/// `GET /api/v1/calendars/settings` — legacy single-account view: returns the
/// FIRST stored account in the old flat shape (kept for compatibility).
pub async fn get_caldav_settings(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let account = state.caldav_accounts.read().first().cloned();
    match account {
        Some(s) => Ok(Json(serde_json::json!({
            "url": s.url, "username": s.username, "password": s.password,
            "sync_interval_minutes": s.sync_interval_minutes,
        }))),
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

/// `POST /api/v1/calendars/settings` — legacy save: upserts the "default"
/// account (new frontend uses /calendars/caldav-accounts).
pub async fn set_caldav_settings(
    State(state): State<AppState>,
    Json(req): Json<CalDavSettingsRequest>,
) -> ApiResult<serde_json::Value> {
    let account = CalDavAccountInput {
        id: Some("default".into()),
        name: Some("Standard".into()),
        url: req.url,
        username: req.username,
        password: Some(req.password),
        enabled: None,
        sync_interval_minutes: req.sync_interval_minutes,
    };
    upsert_account(&state, account).await
}

// ---------------------------------------------------------------------------
// Multi-account settings
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone)]
pub struct CalDavAccountInput {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub url: String,
    pub username: String,
    /// Empty/None on update keeps the stored password.
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub sync_interval_minutes: Option<u64>,
}

/// Persist the account list (passwords encrypted) + refresh live state.
fn store_accounts(state: &AppState, mut accounts: Vec<CalDavSettings>) -> Result<(), ApiError> {
    for a in accounts.iter_mut() {
        if a.id.is_empty() {
            a.id = format!("cal-{}", uuid::Uuid::new_v4());
        }
    }
    let mut encrypted = accounts.clone();
    for a in encrypted.iter_mut() {
        a.password = crypto::encrypt(&a.password).unwrap_or_else(|_| a.password.clone());
    }
    let raw = serde_json::to_string(&encrypted).map_err(|e| ApiError(e.to_string()))?;
    with_db(state, |conn| {
        cache::settings::set_setting(conn, "caldav_accounts", &raw).map_err(|e| e.to_string())
    })?;
    *state.caldav_accounts.write() = accounts;
    Ok(())
}

/// `GET /api/v1/calendars/caldav-accounts` — list accounts (no passwords).
pub async fn list_caldav_accounts(State(state): State<AppState>) -> ApiResult<Vec<serde_json::Value>> {
    let accounts: Vec<serde_json::Value> = state
        .caldav_accounts
        .read()
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a.id, "name": a.name, "url": a.url, "username": a.username,
                "enabled": a.enabled, "sync_interval_minutes": a.sync_interval_minutes,
                "has_password": !a.password.is_empty(),
            })
        })
        .collect();
    Ok(Json(accounts))
}

/// Add or update one account (id present & known → update, else add).
pub async fn upsert_account(
    state: &AppState,
    req: CalDavAccountInput,
) -> ApiResult<serde_json::Value> {
    let mut accounts = state.caldav_accounts.read().clone();
    let existing = req
        .id
        .as_deref()
        .filter(|id| !id.is_empty())
        .and_then(|id| accounts.iter().position(|a| a.id == id));
    let password = req.password.filter(|p| !p.is_empty());
    if let Some(pos) = existing {
        let a = &mut accounts[pos];
        a.url = req.url.clone();
        a.username = req.username.clone();
        if let Some(p) = password {
            a.password = p;
        }
        if let Some(n) = req.name {
            a.name = n;
        }
        if let Some(e) = req.enabled {
            a.enabled = e;
        }
        if let Some(i) = req.sync_interval_minutes {
            a.sync_interval_minutes = i.max(1);
        }
    } else {
        accounts.push(CalDavSettings {
            id: req.id.unwrap_or_default(),
            name: req.name.unwrap_or_else(|| req.username.clone()),
            enabled: req.enabled.unwrap_or(true),
            url: req.url.clone(),
            username: req.username.clone(),
            password: password.unwrap_or_default(),
            sync_interval_minutes: req.sync_interval_minutes.unwrap_or(30).max(1),
        });
    }
    store_accounts(state, accounts)?;
    tracing::info!("CalDAV-Konto gespeichert: {}", req.url);
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// `POST /api/v1/calendars/caldav-accounts` — add a CalDAV account.
pub async fn create_caldav_account(
    State(state): State<AppState>,
    Json(req): Json<CalDavAccountInput>,
) -> ApiResult<serde_json::Value> {
    upsert_account(&state, req).await
}

/// `PUT /api/v1/calendars/caldav-accounts/{id}` — update one account.
pub async fn update_caldav_account(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(mut req): Json<CalDavAccountInput>,
) -> ApiResult<serde_json::Value> {
    req.id = Some(id);
    upsert_account(&state, req).await
}

/// `DELETE /api/v1/calendars/caldav-accounts/{id}` — remove an account AND
/// cascade its calendars + events out of the local cache (provider data
/// untouched).
pub async fn delete_caldav_account(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let accounts: Vec<CalDavSettings> = state
        .caldav_accounts
        .read()
        .iter()
        .filter(|a| a.id != id)
        .cloned()
        .collect();
    let removed = with_db(&state, |conn| {
        crate::cache::cal::delete_calendars_for_account(conn, &id).map_err(|e| e.to_string())
    })?;
    store_accounts(&state, accounts)?;
    tracing::info!("CalDAV-Konto '{}' gelöscht ({} Kalender entfernt)", id, removed);
    Ok(Json(serde_json::json!({ "ok": true, "calendars_removed": removed })))
}

// ---------------------------------------------------------------------------
// Sync
// ---------------------------------------------------------------------------

/// `POST /api/v1/calendars/sync` — manual sync of ALL enabled accounts.
pub async fn sync_caldav(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    do_caldav_sync(&state).await
}

/// Sync every enabled account; aggregate result.
pub async fn do_caldav_sync(state: &AppState) -> ApiResult<serde_json::Value> {
    let accounts: Vec<CalDavSettings> = state
        .caldav_accounts
        .read()
        .iter()
        .filter(|a| a.enabled && !a.url.is_empty())
        .cloned()
        .collect();
    if accounts.is_empty() {
        return Err(ApiError("CalDAV nicht konfiguriert".into()));
    }
    let mut synced = 0usize;
    let mut errors: Vec<String> = Vec::new();
    for account in &accounts {
        match do_caldav_sync_account(state, account).await {
            Ok(v) => {
                if let Some(n) = v.get("synced").and_then(|s| s.as_u64()) {
                    synced += n as usize;
                }
            }
            Err(e) => errors.push(format!("{}: {}", account.name, e.0)),
        }
    }
    if !errors.is_empty() && synced == 0 {
        return Err(ApiError(errors.join("; ")));
    }
    Ok(Json(serde_json::json!({ "ok": true, "synced": synced, "errors": errors })))
}

/// Shared sync logic for ONE account (manual endpoint + background scheduler).
pub async fn do_caldav_sync_account(
    state: &AppState,
    settings: &CalDavSettings,
) -> ApiResult<serde_json::Value> {
    let client = CalDavClient::new(settings.clone());

    // Always full-sync: the incremental (SYNC-COLLECTION) path only covers the
    // first discovered calendar (single-token model), so multi-calendar setups
    // would silently miss updates on every other calendar. A full sync is the
    // only reliable way to keep all calendars current.
    let (events, new_token) = client.fetch_all_events().await.map_err(ApiError)?;

    // Persist calendars + events.
    let calendars = client.discover_calendars().await.unwrap_or_default();
    let mut saved = 0usize;
    if let Ok(mut guard) = get_db(state) {
        if let Some(conn) = guard.as_mut() {
            for cal in &calendars {
                if let Ok(cid) = crate::cache::cal::upsert_calendar(conn, cal, &settings.id) {
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
    let token_key = format!("caldav_sync_token:{}", settings.id);
    let _ = with_db(state, |conn| {
        cache::settings::set_setting(conn, &token_key, &new_token).map_err(|e| e.to_string())
    });

    tracing::info!("CalDAV-Sync '{}': {} Events gespeichert", settings.name, saved);
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

    // Resolve the calendar URL + owning CalDAV account.
    let (cal_url, cal_account_id) = with_db(&state, |conn| {
        conn.query_row(
            "SELECT url, COALESCE(caldav_account_id, 'default') FROM calendars WHERE id = ?1",
            rusqlite::params![req.calendar_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .map_err(|e| e.to_string())
    })
    .map_err(ApiError)?;
    let settings = state
        .caldav_account_by_id(&cal_account_id)
        .ok_or_else(|| ApiError("CalDAV nicht konfiguriert — Event kann nicht angelegt werden".into()))?;
    let client = CalDavClient::new(settings);

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

/// The CalDAV account owning a calendar (""/legacy rows → first account).
pub(crate) fn account_for_calendar(state: &AppState, calendar_id: i64) -> Option<CalDavSettings> {
    let acct_id = with_db(state, |conn| {
        conn.query_row(
            "SELECT COALESCE(caldav_account_id, 'default') FROM calendars WHERE id = ?1",
            rusqlite::params![calendar_id],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())
    })
    .ok()?;
    state.caldav_account_by_id(&acct_id)
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

    if let Some(settings) = account_for_calendar(&state, existing.calendar_id) {
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
    if let Some(settings) = account_for_calendar(&state, existing.calendar_id) {
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

    let client = account_for_calendar(&state, req.calendar_id).map(CalDavClient::new);

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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Path;

    fn state_with_db() -> AppState {
        let state = AppState::new();
        let _ = crate::crypto::init_key(&std::env::temp_dir().join("relay-test-crypto"));
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::cache::db::init_db(&conn).unwrap();
        *state.cache_db.lock() = Some(conn);
        state
    }

    #[test]
    fn legacy_single_account_json_deserializes_with_defaults() {
        let legacy = r#"{"url":"https://cal/x/","username":"u","password":"p","sync_interval_minutes":30}"#;
        let s: CalDavSettings = serde_json::from_str(legacy).unwrap();
        assert!(s.enabled);
        assert_eq!(s.id, "");
        assert_eq!(s.url, "https://cal/x/");
    }

    #[tokio::test]
    async fn upsert_add_assigns_id_and_persists() {
        let state = state_with_db();
        let req = CalDavAccountInput {
            id: None,
            name: Some("Privat".into()),
            url: "https://c1/".into(),
            username: "u1".into(),
            password: Some("pw1".into()),
            enabled: None,
            sync_interval_minutes: Some(15),
        };
        upsert_account(&state, req).await.unwrap();
        let accounts = state.caldav_accounts.read();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].name, "Privat");
        assert!(!accounts[0].id.is_empty());
        assert_eq!(accounts[0].sync_interval_minutes, 15);
        drop(accounts);
        // Stored under the new key, password encrypted (not plaintext).
        let raw = with_db(&state, |conn| {
            cache::settings::get_setting(conn, "caldav_accounts").map_err(|e| e.to_string())
        })
        .unwrap()
        .unwrap();
        assert!(!raw.contains("pw1"), "password must not be stored in plaintext");
    }

    #[tokio::test]
    async fn upsert_update_keeps_password_when_empty() {
        let state = state_with_db();
        let mut req = CalDavAccountInput {
            id: None,
            name: Some("A".into()),
            url: "https://c1/".into(),
            username: "u1".into(),
            password: Some("pw1".into()),
            enabled: None,
            sync_interval_minutes: None,
        };
        upsert_account(&state, req.clone()).await.unwrap();
        let id = state.caldav_accounts.read()[0].id.clone();
        req.id = Some(id.clone());
        req.password = Some("".into());
        req.url = "https://c1-changed/".into();
        upsert_account(&state, req).await.unwrap();
        let accounts = state.caldav_accounts.read();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].url, "https://c1-changed/");
        assert_eq!(accounts[0].password, "pw1", "empty password keeps existing");
    }

    #[tokio::test]
    async fn delete_account_cascades_calendars_and_events() {
        let state = state_with_db();
        let req = CalDavAccountInput {
            id: Some("acct-x".into()),
            name: Some("X".into()),
            url: "https://cx/".into(),
            username: "u".into(),
            password: Some("p".into()),
            enabled: None,
            sync_interval_minutes: None,
        };
        upsert_account(&state, req).await.unwrap();
        // Seed a calendar + event for that account.
        {
            let mut guard = state.cache_db.lock();
            let conn = guard.as_mut().unwrap();
            let cal = crate::dav::Calendar {
                href: "/x/".into(),
                display_name: Some("X".into()),
                url: "https://cx/x/".into(),
            };
            let cid = crate::cache::cal::upsert_calendar(conn, &cal, "acct-x").unwrap();
            let mut ev = crate::dav::IcsEvent {
                uid: "e1".into(),
                url: "https://cx/x/e1.ics".into(),
                summary: Some("T".into()),
                description: None,
                location: None,
                start: "2026-09-01T13:00:00Z".into(),
                end: None,
                all_day: false,
                organizer: None,
                attendees: vec![],
                status: None,
                sequence: 0,
                method: None,
                raw: "BEGIN:VEVENT\\nUID:e1\\nEND:VEVENT".into(),
                rrule: None,
                alarms: 0,
            };
            crate::cache::cal::save_event(conn, cid, &mut ev).ok();
        }
        delete_caldav_account(State(state.clone()), Path("acct-x".into())).await.unwrap();
        assert!(state.caldav_accounts.read().is_empty());
        let guard = state.cache_db.lock();
        let conn = guard.as_ref().unwrap();
        let cals: i64 = conn.query_row("SELECT COUNT(*) FROM calendars", [], |r| r.get(0)).unwrap();
        let evs: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0)).unwrap();
        assert_eq!(cals, 0);
        assert_eq!(evs, 0);
    }
}
