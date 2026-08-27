//! To-do (VTODO) CRUD API — CalDAV-backed local task management.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;

use crate::api::ApiError;
use crate::cache::{self, todo::TodoRow};
use crate::dav::{CalDavClient, CalDavSettings};
use crate::db::with_db;
use crate::AppState;

use super::ApiResult;

/// Build a CalDAV client from in-memory settings, falling back to the DB.
fn caldav_client(state: &AppState) -> Result<CalDavClient, ApiError> {
    if let Some(settings) = state.caldav_settings.read().clone() {
        return Ok(CalDavClient::new(settings));
    }
    let json = with_db(state, |conn| {
        cache::settings::get_setting(conn, "caldav_settings").map_err(|e| e.to_string())
    })?;
    match json {
        Some(raw) => {
            let mut settings: CalDavSettings =
                serde_json::from_str(&raw).map_err(|e| ApiError(format!("CalDAV parse: {e}")))?;
            settings.password =
                crate::crypto::decrypt(&settings.password).unwrap_or(settings.password);
            Ok(CalDavClient::new(settings))
        }
        None => Err(ApiError(
            "Kein CalDAV-Server konfiguriert — bitte zuerst im Settings-Tab verbinden.".to_string(),
        )),
    }
}

/// `GET /api/v1/todos?completed=` — list to-dos.
#[derive(Deserialize)]
pub struct TodosQuery {
    /// "true" = only completed, "false" = only open, absent = all.
    #[serde(default)]
    pub completed: Option<bool>,
}

pub async fn list_todos(
    State(state): State<AppState>,
    Query(q): Query<TodosQuery>,
) -> ApiResult<Vec<TodoRow>> {
    let rows = with_db(&state, |conn| cache::todo::list_todos(conn, q.completed))?;
    Ok(Json(rows))
}

/// `POST /api/v1/todos` — create a to-do (CalDAV + local cache).
#[derive(Deserialize)]
pub struct CreateTodoRequest {
    pub summary: String,
    #[serde(default)]
    pub description: Option<String>,
    /// RFC 3339 due time, optional.
    #[serde(default)]
    pub due: Option<String>,
    /// 1 (highest) – 9 (lowest), optional.
    #[serde(default)]
    pub priority: Option<i64>,
}

fn parse_due(s: &str) -> Result<chrono::DateTime<chrono::Utc>, ApiError> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&chrono::Utc))
        .map_err(|e| ApiError(format!("Ungültiges Fälligkeitsdatum: {e}")))
}

pub async fn create_todo(
    State(state): State<AppState>,
    Json(req): Json<CreateTodoRequest>,
) -> ApiResult<TodoRow> {
    let client = caldav_client(&state)?;

    let due = match &req.due {
        Some(d) => Some(parse_due(d)?),
        None => None,
    };
    let uid = format!("relay-todo-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let ics = crate::dav::ics::build_todo(
        &uid,
        &req.summary,
        due,
        req.description.as_deref(),
        req.priority,
        false,
    )
    .map_err(ApiError)?;

    // Resolve the first calendar URL to store the todo in.
    let cal = with_db(&state, |conn| {
        conn.query_row(
            "SELECT id, url FROM calendars ORDER BY id LIMIT 1",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .map_err(|e| e.to_string())
    })
    .map_err(ApiError)?;

    let url = client.create_event(&cal.1, &ics).await.map_err(ApiError)?;

    let todo = crate::dav::ics::IcsTodo {
        uid: uid.clone(),
        url,
        summary: Some(req.summary.clone()),
        description: req.description.clone(),
        due: req.due.clone(),
        completed: None,
        status: Some("NEEDS-ACTION".to_string()),
        priority: req.priority,
        raw: ics,
    };
    with_db(&state, |conn| cache::todo::upsert_todo(conn, cal.0, &todo))?;

    let row = with_db(&state, |conn| {
        cache::todo::list_todos(conn, None)
            .map(|v| v.into_iter().find(|t| t.uid == uid).ok_or_else(|| "Todo nicht gefunden".to_string()))
    })??;
    Ok(Json(row))
}

/// `PATCH /api/v1/todos/:uid` — toggle completion.
#[derive(Deserialize)]
pub struct ToggleTodoRequest {
    pub completed: bool,
}

pub async fn toggle_todo(
    State(state): State<AppState>,
    Path(uid): Path<String>,
    Json(req): Json<ToggleTodoRequest>,
) -> ApiResult<TodoRow> {
    with_db(&state, |conn| cache::todo::set_completed(conn, &uid, req.completed))?;

    // Best-effort CalDAV update: rebuild the todo with the new status at its
    // stored object URL.
    if let Ok(client) = caldav_client(&state) {
        let existing = with_db(&state, |conn| {
            Ok(
                conn.query_row(
                    "SELECT summary, description, due_at, priority, url FROM todos WHERE uid = ?1",
                    rusqlite::params![uid],
                    |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, Option<String>>(2)?,
                            r.get::<_, Option<i64>>(3)?,
                            r.get::<_, String>(4)?,
                        ))
                    },
                )
                .ok(),
            )
        })?;
        if let Some((summary, description, due_at, priority, url)) = existing {
            if let Ok(ics) = crate::dav::ics::build_todo(
                &uid,
                &summary.unwrap_or_default(),
                due_at.as_deref().and_then(parse_due_ok),
                description.as_deref(),
                priority,
                req.completed,
            ) {
                let _ = client.update_event(&url, &ics).await;
            }
        }
    }

    let row = with_db(&state, |conn| {
        cache::todo::list_todos(conn, None)
            .map(|v| v.into_iter().find(|t| t.uid == uid).ok_or_else(|| "Todo nicht gefunden".to_string()))
    })??;
    Ok(Json(row))
}

fn parse_due_ok(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc))
}

/// `DELETE /api/v1/todos/:uid` — delete a to-do.
pub async fn delete_todo(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> ApiResult<serde_json::Value> {
    with_db(&state, |conn| cache::todo::delete_todo(conn, &uid))?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// `POST /api/v1/todos/sync` — pull all VTODOs from CalDAV into the local cache.
pub async fn sync_todos(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let client = caldav_client(&state)?;
    let todos = client.fetch_all_todos().await.map_err(ApiError)?;

    let count = with_db(&state, |conn| {
        let cal_id: i64 = conn
            .query_row("SELECT id FROM calendars ORDER BY id LIMIT 1", [], |r| r.get(0))
            .unwrap_or(0);
        for t in &todos {
            cache::todo::upsert_todo(conn, cal_id, t)?;
        }
        Ok(todos.len())
    })
    .map_err(ApiError)?;

    Ok(Json(serde_json::json!({ "synced": count })))
}
