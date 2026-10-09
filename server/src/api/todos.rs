//! To-do (VTODO) CRUD API — CalDAV-backed local task management.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;

use crate::api::ApiError;
use crate::cache::{self, todo::TodoRow};
use crate::dav::CalDavClient;
use crate::db::with_db;
use crate::AppState;

use super::ApiResult;

/// Serde helper: distinguishes an **absent** field from an **explicit `null`**
/// in a PATCH body. `Option<T>` alone collapses both to `None`; wrapping in
/// `Option<Option<T>>` with this module lets `null` mean "clear this field".
mod serde_with_optional {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
    where
        T: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        Option::<T>::deserialize(de).map(Some)
    }
}

/// Build a CalDAV client from the first configured account (VTODOs live in
/// the primary calendar account).
fn caldav_client(state: &AppState) -> Result<CalDavClient, ApiError> {
    // The first account switched on for tasks (26.10.18), else the first.
    let konten = state.caldav_accounts.read();
    let gewaehlt = konten
        .iter()
        .find(|a| a.enabled && a.aufgaben)
        .or_else(|| konten.first())
        .cloned();
    drop(konten);
    match gewaehlt {
        Some(settings) => Ok(CalDavClient::new(settings)),
        None => Err(ApiError(
            "Kein CalDAV-Server konfiguriert — bitte zuerst im Settings-Tab verbinden.".to_string(),
        )),
    }
}

/// The Gmail account and Google URL of a task that lives in Google Tasks.
fn google_aufgabe(state: &AppState, uid: &str) -> Option<(i64, String)> {
    let (url, cal_id): (String, i64) = with_db(state, |conn| {
        conn.query_row("SELECT url, calendar_id FROM todos WHERE uid = ?1", [uid], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())
    })
    .ok()?;
    if !crate::google::aufgaben::ist_aufgabe(&url) {
        return None;
    }
    google_konto_der_liste(state, cal_id).map(|k| (k, url))
}

/// `GET /api/v1/todos?completed=` — list to-dos.
#[derive(Deserialize)]
pub struct TodosQuery {
    /// "true" = only completed, "false" = only open, absent = all.
    #[serde(default)]
    pub completed: Option<bool>,
    /// Sort mode: `work` (default, dependency-aware work order), `due`,
    /// `priority`, `title`, `created`, `manual`.
    #[serde(default)]
    pub sort: Option<String>,
    /// `asc` (default) or `desc`.
    #[serde(default)]
    pub dir: Option<String>,
}

pub async fn list_todos(
    State(state): State<AppState>,
    Query(q): Query<TodosQuery>,
) -> ApiResult<Vec<TodoRow>> {
    let mut rows = with_db(&state, |conn| cache::todo::list_todos(conn, q.completed))?;
    sort_todos(&mut rows, q.sort.as_deref().unwrap_or("work"));
    if q.dir.as_deref() == Some("desc") {
        rows.reverse();
    }
    Ok(Json(rows))
}

/// Apply a sort mode in place. `work` uses the dependency-aware work order.
fn sort_todos(rows: &mut [TodoRow], mode: &str) {
    let alpha = |r: &TodoRow| r.summary.as_deref().unwrap_or("").trim().to_lowercase();
    let due_key = |r: &TodoRow| r.due_at.clone().unwrap_or_else(|| "\u{10FFFF}".to_string());
    match mode {
        "title" => rows.sort_by(|a, b| alpha(a).cmp(&alpha(b)).then(a.id.cmp(&b.id))),
        "priority" => rows.sort_by(|a, b| {
            a.priority.unwrap_or(6).cmp(&b.priority.unwrap_or(6))
                .then(due_key(a).cmp(&due_key(b)))
                .then(alpha(a).cmp(&alpha(b)))
                .then(a.id.cmp(&b.id))
        }),
        "due" => rows.sort_by(|a, b| {
            due_key(a).cmp(&due_key(b)).then(a.id.cmp(&b.id))
        }),
        "created" => rows.sort_by_key(|r| r.id),
        "manual" => rows.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then(a.id.cmp(&b.id))),
        _ => {
            let order = cache::order::work_order(rows);
            let pos: std::collections::HashMap<&str, usize> = order
                .iter()
                .enumerate()
                .map(|(i, u)| (u.as_str(), i))
                .collect();
            rows.sort_by_key(|r| pos.get(r.uid.as_str()).copied().unwrap_or(usize::MAX));
        }
    }
}

/// `POST /api/v1/todos` — create a to-do (CalDAV + local cache).
#[derive(Deserialize)]
pub struct CreateTodoRequest {
    pub summary: String,
    #[serde(default)]
    pub description: Option<String>,
    /// RFC 3339 due time or date-only (YYYY-MM-DD), optional.
    #[serde(default)]
    pub due: Option<String>,
    /// 1 (highest) – 9 (lowest), optional.
    #[serde(default)]
    pub priority: Option<i64>,
    /// Labels (written as CATEGORIES).
    #[serde(default)]
    pub labels: Vec<String>,
    /// Recurrence rule, e.g. `FREQ=WEEKLY;BYDAY=FR`, optional.
    #[serde(default)]
    pub rrule: Option<String>,
    /// Parent task UID for sub-tasks, optional.
    #[serde(default)]
    pub parent_uid: Option<String>,
    /// Blocking task UIDs ("blocked by", `RELATED-TO;RELTYPE=DEPENDS-ON`).
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Target project (calendar id). None = Inbox.
    #[serde(default)]
    pub project_id: Option<i64>,
    /// Explicit date-vs-date-time flag. When absent it is derived from whether
    /// `due` carries a time (`T`). Quick-Add sends this so a date-only task
    /// stays all-day even though the wire format is RFC 3339 at midnight.
    #[serde(default)]
    pub due_has_time: Option<bool>,
}

/// True when a due string carries a time-of-day (RFC 3339 with a `T`).
fn due_has_time(s: &str) -> bool {
    s.contains('T')
}

fn parse_due(s: &str) -> Result<chrono::DateTime<chrono::Utc>, ApiError> {
    let s = s.trim();
    // Full RFC 3339 timestamp (e.g. "2026-09-01T09:00:00Z").
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(d.with_timezone(&chrono::Utc));
    }
    // Date-only value from an <input type="date"> (e.g. "2026-09-01"):
    // interpret it as the local start of day in the app timezone.
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        if let Some(naive) = date.and_hms_opt(0, 0, 0) {
            if let Some(local) = chrono::TimeZone::from_local_datetime(
                &chrono_tz::Europe::Berlin,
                &naive,
            )
            .earliest()
            {
                return Ok(local.with_timezone(&chrono::Utc));
            }
        }
    }
    Err(ApiError(format!("Ungültiges Fälligkeitsdatum: {s}")))
}

/// The Gmail account behind a calendar row of a Google task list.
fn google_konto_der_liste(state: &AppState, cal_id: i64) -> Option<i64> {
    let konto_id: String = with_db(state, |conn| {
        conn.query_row("SELECT caldav_account_id FROM calendars WHERE id = ?1", [cal_id], |r| r.get(0))
            .map_err(|e| e.to_string())
    })
    .ok()?;
    state
        .caldav_accounts
        .read()
        .iter()
        .find(|a| a.id == konto_id && a.google)
        .and_then(|a| a.mail_konto)
}

/// Resolve the `(id, url)` of the calendar a todo should be written to:
/// the requested project when given, else the first calendar (Inbox default).
/// When the account chosen for tasks is a Gmail one, its first task list.
fn resolve_target_calendar(
    state: &AppState,
    project_id: Option<i64>,
) -> Result<(i64, String), ApiError> {
    if project_id.is_none() {
        let google = state
            .caldav_accounts
            .read()
            .iter()
            .find(|a| a.enabled && a.aufgaben)
            .filter(|a| a.google)
            .map(|a| a.id.clone());
        if let Some(konto_id) = google {
            let muster = format!("{}/lists/%", crate::google::endpunkte().aufgaben);
            let liste = with_db(state, |conn| {
                Ok(conn
                    .query_row(
                        "SELECT id, url FROM calendars WHERE caldav_account_id = ?1 AND url LIKE ?2 ORDER BY id LIMIT 1",
                        rusqlite::params![konto_id, muster],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                    )
                    .ok())
            })?;
            if let Some(l) = liste {
                return Ok(l);
            }
        }
    }
    with_db(state, |conn| {
        let row = match project_id {
            Some(pid) => conn.query_row(
                "SELECT id, url FROM calendars WHERE id = ?1",
                rusqlite::params![pid],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            ),
            None => conn.query_row(
                "SELECT id, url FROM calendars ORDER BY id LIMIT 1",
                [],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            ),
        };
        row.map_err(|e| e.to_string())
    })
    .map_err(ApiError)
}

/// Write a new task into its project: a Google task list through Google
/// Tasks, anything else as VTODO over CalDAV. Returns the calendar row, the
/// object URL, the UID it ends up with and the ICS kept as raw.
async fn im_ziel_anlegen(
    state: &AppState,
    project_id: Option<i64>,
    spec: &crate::dav::ics::TodoSpec<'_>,
) -> Result<(i64, String, String, String), ApiError> {
    let (cal_id, cal_url) = match resolve_target_calendar(state, project_id) {
        Ok(z) => z,
        // No calendar at all: say what to set up.
        Err(e) => return Err(caldav_client(state).err().unwrap_or(e)),
    };
    if crate::google::aufgaben::ist_aufgabe(&cal_url) {
        let konto = google_konto_der_liste(state, cal_id)
            .ok_or_else(|| ApiError("Diese Aufgabenliste gehört zu keinem angemeldeten Google-Konto.".into()))?;
        let faellig = spec.due.map(|d| d.to_rfc3339());
        let (url, uid) = crate::google::aufgaben::anlegen(
            konto,
            &cal_url,
            &crate::google::aufgaben::Felder {
                titel: spec.summary,
                notizen: spec.description,
                faellig: faellig.as_deref(),
                erledigt: spec.completed,
            },
        )
        .await
        .map_err(ApiError)?;
        return Ok((cal_id, url, uid, String::new()));
    }
    let client = caldav_client(state)?;
    let ics = crate::dav::ics::build_todo(spec).map_err(ApiError)?;
    let url = client.create_event(&cal_url, &ics).await.map_err(ApiError)?;
    Ok((cal_id, url, spec.uid.to_string(), ics))
}

pub async fn create_todo(
    State(state): State<AppState>,
    Json(req): Json<CreateTodoRequest>,
) -> ApiResult<TodoRow> {
    let has_time = req
        .due_has_time
        .unwrap_or_else(|| req.due.as_deref().map(due_has_time).unwrap_or(false));
    let due = match &req.due {
        Some(d) => Some(parse_due(d)?),
        None => None,
    };
    let uid = format!("relay-todo-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let spec = crate::dav::ics::TodoSpec {
        uid: &uid,
        summary: &req.summary,
        due,
        due_has_time: has_time,
        description: req.description.as_deref(),
        priority: crate::priority::normalize(req.priority),
        completed: false,
        rrule: req.rrule.as_deref(),
        labels: &req.labels,
        parent_uid: req.parent_uid.as_deref(),
        dependencies: &req.dependencies,
    };
    let (cal_id, url, uid, ics) = im_ziel_anlegen(&state, req.project_id, &spec).await?;

    let todo = crate::dav::ics::IcsTodo {
        uid: uid.clone(),
        url,
        summary: Some(req.summary.clone()),
        description: req.description.clone(),
        due: due.map(|d| d.to_rfc3339()),
        completed: None,
        status: Some("NEEDS-ACTION".to_string()),
        priority: crate::priority::normalize(req.priority),
        rrule: req.rrule.clone(),
        labels: req.labels.clone(),
        parent_uid: req.parent_uid.clone(),
        dependencies: req.dependencies.clone(),
        project_id: req.project_id,
        due_has_time: has_time,
        raw: ics,
    };
    with_db(&state, |conn| cache::todo::upsert_todo(conn, cal_id, &todo))?;

    let row = with_db(&state, |conn| {
        cache::todo::find_todo(conn, &uid).map(|o| o.ok_or_else(|| "Todo nicht gefunden".to_string()))
    })??;
    Ok(Json(row))
}

/// `PATCH /api/v1/todos/:uid` — update fields and/or toggle completion.
///
/// Backwards compatible: a body with only `completed` still works exactly as
/// before. Any subset of the other fields may be sent; absent fields are left
/// unchanged. `null` on an optional field clears it.
#[derive(Deserialize)]
pub struct PatchTodoRequest {
    #[serde(default)]
    pub completed: Option<bool>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default, with = "serde_with_optional")]
    pub description: Option<Option<String>>,
    #[serde(default, with = "serde_with_optional")]
    pub due: Option<Option<String>>,
    #[serde(default, with = "serde_with_optional")]
    pub priority: Option<Option<i64>>,
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    #[serde(default, with = "serde_with_optional")]
    pub rrule: Option<Option<String>>,
    #[serde(default, with = "serde_with_optional")]
    pub project_id: Option<Option<i64>>,
    /// Replacement dependency list ("blocked by"); `[]` clears it.
    #[serde(default)]
    pub dependencies: Option<Vec<String>>,
}

pub async fn toggle_todo(
    State(state): State<AppState>,
    Path(uid): Path<String>,
    Json(req): Json<PatchTodoRequest>,
) -> ApiResult<TodoRow> {
    // 1. Completion toggle (if requested).
    if let Some(done) = req.completed {
        with_db(&state, |conn| cache::todo::set_completed(conn, &uid, done))?;
    }

    // 2. Field updates.
    let due_value = req.due.as_ref().and_then(|inner| inner.as_ref());
    let has_time = due_value.map(|d| due_has_time(d));
    let due_parsed = match due_value {
        Some(d) => Some(parse_due(d)?),
        None => None,
    };
    let update = cache::todo::TodoUpdate {
        summary: req.summary.clone(),
        description: req.description.clone(),
        due_at: req
            .due
            .as_ref()
            .map(|_| due_parsed.map(|d| d.to_rfc3339())),
        due_has_time: has_time,
        priority: req.priority.as_ref().map(|inner| crate::priority::normalize(*inner)),
        labels: req.labels.clone(),
        rrule: req.rrule.clone(),
        project_id: req.project_id.clone(),
        dependencies: req.dependencies.clone(),
    };
    with_db(&state, |conn| cache::todo::update_todo(conn, &uid, &update))?;

    // 3. Best-effort write-back with the full, current field set: Google
    //    Tasks for a Gmail task, else CalDAV.
    if let Some((konto, url)) = google_aufgabe(&state, &uid) {
        if let Some(t) = with_db(&state, |conn| cache::todo::find_todo(conn, &uid))? {
            if let Err(e) = crate::google::aufgaben::aendern(
                konto,
                &url,
                &crate::google::aufgaben::Felder {
                    titel: t.summary.as_deref().unwrap_or_default(),
                    notizen: t.description.as_deref(),
                    faellig: t.due_at.as_deref(),
                    erledigt: t.status == "COMPLETED",
                },
            )
            .await
            {
                tracing::warn!("Google Tasks: Änderung an '{uid}' nicht übertragen: {e}");
            }
        }
    } else if let Ok(client) = caldav_client(&state) {
        let current = with_db(&state, |conn| cache::todo::find_todo(conn, &uid))?;
        if let Some(t) = current {
            let done = t.status == "COMPLETED";
            let url = with_db(&state, |conn| {
                Ok(conn
                    .query_row(
                        "SELECT url FROM todos WHERE uid = ?1",
                        rusqlite::params![uid],
                        |r| r.get::<_, String>(0),
                    )
                    .ok())
            })?;
            if let Some(url) = url {
                if let Ok(ics) = crate::dav::ics::build_todo(&crate::dav::ics::TodoSpec {
                    uid: &uid,
                    summary: t.summary.as_deref().unwrap_or_default(),
                    due: t.due_at.as_deref().and_then(parse_due_ok),
                    due_has_time: t.due_has_time,
                    description: t.description.as_deref(),
                    priority: t.priority,
                    completed: done,
                    rrule: t.rrule.as_deref(),
                    labels: &t.labels,
                    parent_uid: t.parent_uid.as_deref(),
                    dependencies: &t.dependencies,
                }) {
                    let _ = client.update_event(&url, &ics).await;
                }
            }
        }
    }

    let row = with_db(&state, |conn| {
        cache::todo::find_todo(conn, &uid).map(|o| o.ok_or_else(|| "Todo nicht gefunden".to_string()))
    })??;
    Ok(Json(row))
}

/// `POST /api/v1/todos/quick-add` — parse one natural-language line and create
/// the task in a single step (Todoist-style instant capture, no AI round-trip).
///
/// Body: `{ "text": "Freitag Budget prüfen p1 #Firma @dringend" }`.
#[derive(Deserialize)]
pub struct QuickAddRequest {
    pub text: String,
    /// Optional default calendar id when the line names no project.
    #[serde(default)]
    pub project_id: Option<i64>,
}

pub async fn quick_add_todo(
    State(state): State<AppState>,
    Json(req): Json<QuickAddRequest>,
) -> ApiResult<TodoRow> {
    let parsed = crate::api::quick_add::parse(&req.text, chrono::Local::now());
    if parsed.title.trim().is_empty() {
        return Err(ApiError("Bitte einen Titel angeben.".to_string()));
    }

    // Resolve a #Project name to a calendar id by (case-insensitive) display
    // name match; fall back to the requested default / first calendar.
    let project_id = match &parsed.project {
        Some(name) => with_db(&state, |conn| {
            Ok(conn
                .query_row(
                    "SELECT id FROM calendars WHERE lower(display_name) = lower(?1) LIMIT 1",
                    rusqlite::params![name],
                    |r| r.get::<_, i64>(0),
                )
                .ok())
        })?
        .or(req.project_id),
        None => req.project_id,
    };

    let create = CreateTodoRequest {
        summary: parsed.title.clone(),
        description: None,
        due: parsed.due.map(|d| d.to_rfc3339()),
        priority: parsed.priority.to_canonical(),
        labels: parsed.labels.clone(),
        rrule: parsed.rrule.clone(),
        parent_uid: None,
        dependencies: Vec::new(),
        project_id,
        due_has_time: Some(parsed.due_has_time),
    };
    create_todo(State(state), Json(create)).await
}

/// `GET /api/v1/todos/views` — counts for the sidebar focus views
/// (Inbox / Heute / Nächstes / Erledigt), computed from the local cache.
pub async fn todo_views(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let rows = with_db(&state, |conn| cache::todo::list_todos(conn, None))?;
    let today = chrono::Local::now().date_naive();
    let mut inbox = 0i64;
    let mut today_n = 0i64;
    let mut upcoming = 0i64;
    let mut overdue = 0i64;
    let mut done = 0i64;
    for t in &rows {
        if t.status == "COMPLETED" {
            done += 1;
            continue;
        }
        if t.project_id.is_none() {
            inbox += 1;
        }
        match t.due_at.as_deref().and_then(parse_due_ok) {
            Some(d) => {
                let day = d.with_timezone(&chrono::Local).date_naive();
                if day < today {
                    overdue += 1;
                } else if day == today {
                    today_n += 1;
                } else {
                    upcoming += 1;
                }
            }
            None => {}
        }
    }
    Ok(Json(serde_json::json!({
        "inbox": inbox,
        "today": today_n,
        "upcoming": upcoming,
        "overdue": overdue,
        "done": done,
        "all": rows.len(),
    })))
}

/// `POST /api/v1/todos/reorder` — persist a manual order.
#[derive(Deserialize)]
pub struct ReorderRequest {
    /// Ordered UIDs; position in the array becomes the sort_order.
    pub uids: Vec<String>,
}

pub async fn reorder_todos(
    State(state): State<AppState>,
    Json(req): Json<ReorderRequest>,
) -> ApiResult<serde_json::Value> {
    let pairs: Vec<(String, i64)> = req
        .uids
        .iter()
        .enumerate()
        .map(|(i, uid)| (uid.clone(), i as i64))
        .collect();
    let n = crate::db::with_db_mut(&state, |conn| {
        cache::todo::reorder_todos(conn, &pairs).map(|_| pairs.len())
    })?;
    Ok(Json(serde_json::json!({ "reordered": n })))
}

/// `POST /api/v1/todos/:uid/succeed` — complete a recurring task and create its
/// next occurrence (client-side recurrence; the server does not expand RRULEs).
///
/// The next due date is computed from the stored RRULE (daily/weekly/monthly/
/// yearly + INTERVAL). Returns the newly created sibling task.
pub async fn succeed_todo(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> ApiResult<TodoRow> {
    let current = with_db(&state, |conn| {
        cache::todo::find_todo(conn, &uid).map(|o| o.ok_or_else(|| "Todo nicht gefunden".to_string()))
    })??;

    // Mark the current one done.
    with_db(&state, |conn| cache::todo::set_completed(conn, &uid, true))?;

    let Some(rule) = current.rrule.clone().filter(|r| !r.trim().is_empty()) else {
        // Not recurring: nothing to spawn.
        let row = with_db(&state, |conn| {
            cache::todo::find_todo(conn, &uid)
                .map(|o| o.ok_or_else(|| "Todo nicht gefunden".to_string()))
        })??;
        return Ok(Json(row));
    };

    let base = current
        .due_at
        .as_deref()
        .and_then(parse_due_ok)
        .unwrap_or_else(chrono::Utc::now);
    let next_due = next_occurrence(base, &rule);

    // A done Google task is written back as done.
    if let Some((konto, url)) = google_aufgabe(&state, &uid) {
        let erledigt = crate::google::aufgaben::Felder {
            titel: current.summary.as_deref().unwrap_or_default(),
            notizen: current.description.as_deref(),
            faellig: current.due_at.as_deref(),
            erledigt: true,
        };
        if let Err(e) = crate::google::aufgaben::aendern(konto, &url, &erledigt).await {
            tracing::warn!("Google Tasks: '{uid}' nicht als erledigt übertragen: {e}");
        }
    }
    let new_uid = format!("relay-todo-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let spec = crate::dav::ics::TodoSpec {
        uid: &new_uid,
        summary: current.summary.as_deref().unwrap_or_default(),
        due: Some(next_due),
        due_has_time: current.due_has_time,
        description: current.description.as_deref(),
        priority: current.priority,
        completed: false,
        rrule: Some(&rule),
        labels: &current.labels,
        parent_uid: current.parent_uid.as_deref(),
        dependencies: &current.dependencies,
    };
    // Into the task's own list or calendar.
    let ziel = if current.project_id.is_some() { current.project_id } else { Some(current.calendar_id) };
    let (cal_id, url, new_uid, ics) = im_ziel_anlegen(&state, ziel, &spec).await?;
    let new_todo = crate::dav::ics::IcsTodo {
        uid: new_uid.clone(),
        url,
        summary: current.summary.clone(),
        description: current.description.clone(),
        due: Some(next_due.to_rfc3339()),
        completed: None,
        status: Some("NEEDS-ACTION".to_string()),
        priority: current.priority,
        rrule: Some(rule),
        labels: current.labels.clone(),
        parent_uid: current.parent_uid.clone(),
        dependencies: current.dependencies.clone(),
        project_id: current.project_id,
        due_has_time: current.due_has_time,
        raw: ics,
    };
    with_db(&state, |conn| cache::todo::upsert_todo(conn, cal_id, &new_todo))?;

    let row = with_db(&state, |conn| {
        cache::todo::find_todo(conn, &new_uid)
            .map(|o| o.ok_or_else(|| "Todo nicht gefunden".to_string()))
    })??;
    Ok(Json(row))
}

/// Advance a due date by a single RRULE interval. Supports FREQ=DAILY/WEEKLY/
/// MONTHLY/YEARLY with optional `INTERVAL=n`; unknown rules advance by one day
/// so the task never disappears without a replacement.
fn next_occurrence(base: chrono::DateTime<chrono::Utc>, rrule: &str) -> chrono::DateTime<chrono::Utc> {
    use chrono::{Duration, Months};
    let mut freq = "DAILY";
    let mut interval: i64 = 1;
    for part in rrule.split(';') {
        let mut kv = part.splitn(2, '=');
        let k = kv.next().unwrap_or("").trim().to_ascii_uppercase();
        let v = kv.next().unwrap_or("").trim();
        match k.as_str() {
            "FREQ" => freq = Box::leak(v.to_ascii_uppercase().into_boxed_str()),
            "INTERVAL" => interval = v.parse().unwrap_or(1).max(1),
            _ => {}
        }
    }
    match freq {
        "WEEKLY" => base + Duration::days(7 * interval),
        "MONTHLY" => base
            .checked_add_months(Months::new(interval as u32))
            .unwrap_or(base + Duration::days(30 * interval)),
        "YEARLY" => base
            .checked_add_months(Months::new(12 * interval as u32))
            .unwrap_or(base + Duration::days(365 * interval)),
        _ => base + Duration::days(interval),
    }
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
    // A Google task would come back with the next sync.
    if let Some((konto, url)) = google_aufgabe(&state, &uid) {
        crate::google::aufgaben::loeschen(konto, &url).await.map_err(ApiError)?;
    }
    with_db(&state, |conn| cache::todo::delete_todo(conn, &uid))?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// Normalise a title for duplicate detection: lowercase, non-alphanumerics →
/// spaces, collapse whitespace.
pub(crate) fn normalize_title(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(serde::Serialize)]
pub struct ConsolidateReport {
    pub tags_before: usize,
    pub tags_after: usize,
    pub tasks_changed: usize,
    pub candidates_created: usize,
}

/// `POST /api/v1/todos/consolidate` — normalise labels (case/synonyms), rename
/// `Idee` -> `Insilo`, and apply the conservative tag policy to existing data:
/// thematic tags covering < THRESHOLD tasks are removed from the tasks and kept
/// as hidden candidates (so they can still be promoted later).
pub async fn consolidate_todos(State(state): State<AppState>) -> ApiResult<ConsolidateReport> {
    use std::collections::{HashMap, HashSet};

    let rows = with_db(&state, |conn| {
        cache::todo::list_todos(conn, None).map_err(|e| e.to_string())
    })?;

    // Pass 1: normalised labels per task + thematic counts.
    let mut per_task: Vec<(String, Vec<String>)> = Vec::new();
    let mut counts: HashMap<String, i64> = HashMap::new();
    for r in &rows {
        let mut labels: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for l in &r.labels {
            let n = cache::tags::normalize_label(l);
            if n.is_empty() || !seen.insert(n.clone()) {
                continue;
            }
            if n != cache::tags::ORIGIN_LABEL {
                *counts.entry(n.clone()).or_insert(0) += 1;
            }
            labels.push(n);
        }
        per_task.push((r.uid.clone(), labels));
    }
    let tags_before = counts.len();
    let kept: HashSet<String> = counts
        .iter()
        .filter(|(_, c)| **c >= cache::tags::THRESHOLD)
        .map(|(l, _)| l.clone())
        .collect();
    let tags_after = kept.len();

    let mut report = ConsolidateReport {
        tags_before,
        tags_after,
        tasks_changed: 0,
        candidates_created: 0,
    };

    // Pass 2: rewrite labels where they change; park dropped tags as candidates.
    for (idx, (uid, labels)) in per_task.iter().enumerate() {
        let mut new_labels: Vec<String> = Vec::new();
        for l in labels {
            if l == cache::tags::ORIGIN_LABEL || kept.contains(l) {
                new_labels.push(l.clone());
            } else {
                let _ = with_db(&state, |c| {
                    cache::tags::add_candidate(c, l, uid).map_err(|e| e.to_string())
                });
                report.candidates_created += 1;
            }
        }

        // Rewrite whenever the *raw* labels differ from the canonical set
        // (case unification + `Idee`->`Insilo`), not just the normalised one.
        let mut original: Vec<String> = rows[idx].labels.clone();
        original.sort();
        original.dedup();
        let mut next = new_labels.clone();
        next.sort();
        next.dedup();
        if original == next {
            continue;
        }

        let req = PatchTodoRequest {
            completed: None,
            summary: None,
            description: None,
            due: None,
            priority: None,
            labels: Some(new_labels),
            rrule: None,
            project_id: None,
            dependencies: None,
        };
        if toggle_todo(State(state.clone()), Path(uid.clone()), Json(req)).await.is_ok() {
            report.tasks_changed += 1;
        }
    }
    Ok(Json(report))
}

#[derive(serde::Serialize)]
pub struct DedupeReport {
    pub groups: usize,
    pub kept: usize,
    pub merged: usize,
    pub deleted: usize,
}

/// `POST /api/v1/todos/dedupe` — merge open tasks that share an identical
/// normalised title: keep the richest one, fold the others' descriptions into
/// it (as `Mighty:` lines) and delete the duplicates locally + on CalDAV.
pub async fn dedupe_todos(State(state): State<AppState>) -> ApiResult<DedupeReport> {
    let rows = with_db(&state, |conn| {
        cache::todo::list_todos(conn, Some(false)).map_err(|e| e.to_string())
    })?;
    let mut groups: std::collections::HashMap<String, Vec<TodoRow>> = std::collections::HashMap::new();
    for r in rows {
        let key = normalize_title(r.summary.as_deref().unwrap_or(""));
        if key.is_empty() {
            continue;
        }
        groups.entry(key).or_default().push(r);
    }
    let client = caldav_client(&state).ok();
    let mut report = DedupeReport { groups: 0, kept: 0, merged: 0, deleted: 0 };

    for (_key, mut items) in groups {
        if items.len() < 2 {
            continue;
        }
        report.groups += 1;
        // Keeper: longest description, tie-break oldest id.
        items.sort_by_key(|r| {
            (std::cmp::Reverse(r.description.as_deref().map(str::len).unwrap_or(0)), r.id)
        });
        let keeper = items.remove(0);
        let mut desc = keeper.description.clone().unwrap_or_default();
        let mut folded = 0usize;

        for dup in &items {
            if let Some(d) = dup.description.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
                if !desc.contains(d) {
                    if !desc.trim().is_empty() {
                        desc.push('\n');
                    }
                    desc.push_str("Mighty: zusammengeführt – ");
                    desc.push_str(d);
                    folded += 1;
                }
            }
            let url: Option<String> = with_db(&state, |conn| {
                conn.query_row("SELECT url FROM todos WHERE uid = ?1", [&dup.uid], |r| r.get(0))
                    .map(Some)
                    .or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(e.to_string()) })
            })?;
            if let (Some(c), Some(u)) = (client.as_ref(), url.as_deref()) {
                if !u.is_empty() {
                    let _ = c.delete_event(u).await;
                }
            }
            with_db(&state, |conn| cache::todo::delete_todo(conn, &dup.uid))?;
            report.deleted += 1;
        }

        if folded > 0 {
            let req = PatchTodoRequest {
                completed: None,
                summary: None,
                description: Some(Some(desc)),
                due: None,
                priority: None,
                labels: None,
                rrule: None,
                project_id: None,
                dependencies: None,
            };
            let _ = toggle_todo(State(state.clone()), Path(keeper.uid.clone()), Json(req)).await;
            report.merged += folded;
        }
        report.kept += 1;
    }
    Ok(Json(report))
}

/// `POST /api/v1/todos/sync` — pull all VTODOs from CalDAV into the local cache.
///
/// Each VTODO is filed under the calendar collection its object URL belongs to
/// (so tasks land in the correct project); unknown collections fall back to the
/// first calendar.
pub async fn sync_todos(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let client = caldav_client(&state)?;
    let todos = client.fetch_all_todos().await.map_err(ApiError)?;
    // Reuse the calendar-sync persistence path so manual and background syncs
    // file tasks into the same calendars (longest URL-prefix match wins).
    let count = crate::api::calendars::save_todos_to_db(&state, &todos);
    Ok(Json(serde_json::json!({ "synced": count })))
}

#[cfg(test)]
mod tests {
    use super::{due_has_time, next_occurrence, parse_due};

    #[test]
    fn parse_due_accepts_date_only() {
        let d = parse_due("2026-09-01").unwrap();
        // Midnight in Europe/Berlin (CEST, UTC+2 in September).
        assert_eq!(d.to_rfc3339(), "2026-08-31T22:00:00+00:00");
    }

    #[test]
    fn parse_due_accepts_rfc3339() {
        let d = parse_due("2026-09-01T09:00:00Z").unwrap();
        assert_eq!(d.to_rfc3339(), "2026-09-01T09:00:00+00:00");
    }

    #[test]
    fn due_has_time_detects_time_of_day() {
        assert!(due_has_time("2026-09-01T09:00:00Z"));
        assert!(!due_has_time("2026-09-01"));
    }

    #[test]
    fn next_occurrence_weekly() {
        let base = parse_due("2026-09-01T09:00:00Z").unwrap();
        let next = next_occurrence(base, "FREQ=WEEKLY;BYDAY=FR");
        assert_eq!(next, base + chrono::Duration::days(7));
    }

    #[test]
    fn next_occurrence_daily_with_interval() {
        let base = parse_due("2026-09-01T09:00:00Z").unwrap();
        let next = next_occurrence(base, "FREQ=DAILY;INTERVAL=3");
        assert_eq!(next, base + chrono::Duration::days(3));
    }

    #[test]
    fn next_occurrence_monthly() {
        let base = parse_due("2026-01-31T09:00:00Z").unwrap();
        let next = next_occurrence(base, "FREQ=MONTHLY");
        // Jan 31 + 1 month clamps to Feb 28 (2026 is not a leap year).
        assert_eq!(next.to_rfc3339(), "2026-02-28T09:00:00+00:00");
    }

    #[test]
    fn next_occurrence_unknown_freq_falls_back_to_one_day() {
        let base = parse_due("2026-09-01T09:00:00Z").unwrap();
        assert_eq!(next_occurrence(base, "FREQ=HOURLY"), base + chrono::Duration::days(1));
    }

    #[test]
    fn parse_due_rejects_invalid() {
        assert!(parse_due("").is_err());
        assert!(parse_due("not-a-date").is_err());
    }
}
