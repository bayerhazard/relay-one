//! Meetings-API (Insilo cross-app drop): Liste, Detail, manueller Scan.
//!
//! Die Daten kommen vom Scanner (`sync/insilo.rs`), der das geteilte
//! appCommon-Verzeichnis pollt. Alle Routen laufen unter `relay_key_guard`
//! (zentral in `api/mod.rs` geregelt).

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::db::with_db;
use crate::sync::insilo::run_insilo_scan;
use crate::AppState;

use super::ApiResult;

/// Listeneintrag (ohne `body_md`), `snippet` nur bei Volltextsuche.
#[derive(Serialize)]
pub struct MeetingInfo {
    pub id: i64,
    pub insilo_id: String,
    pub title: String,
    pub participants: Vec<String>,
    pub tags: Vec<String>,
    pub meeting_date: String,
    pub duration_min: i64,
    pub language: String,
    pub template: String,
    pub source_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

/// Vollständiger Eintrag inkl. Markdown-Body.
#[derive(Serialize)]
pub struct MeetingDetail {
    #[serde(flatten)]
    pub info: MeetingInfo,
    pub body_md: String,
}

#[derive(Deserialize)]
pub struct MeetingsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
    /// FTS5-Volltextsuche (Titel + Body).
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub participant: Option<String>,
    /// Datumsgrenzen (YYYY-MM-DD, inklusiv).
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

fn default_limit() -> i64 {
    50
}

/// FTS5-Match-Expression aus Nutzereingabe: jedes Wort wird als
/// quoted-String gesichert (Anführungszeichen verdoppelt), damit keine
/// FTS5-Syntax (AND/OR/NEAR/Boolesches) durchrutscht.
fn fts_match(query: &str) -> String {
    query
        .split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `GET /api/v1/meetings?limit=&offset=&query=&tag=&participant=&from=&to=`
pub async fn list_meetings(
    State(state): State<AppState>,
    Query(q): Query<MeetingsQuery>,
) -> ApiResult<Vec<MeetingInfo>> {
    let limit = q.limit.clamp(1, 500);
    let offset = q.offset.max(0);

    let rows = with_db(&state, |conn| {
        let has_query = q.query.as_deref().map(str::trim).is_some_and(|s| !s.is_empty());
        let fts_da = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'meetings_fts'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        let sql = if has_query && fts_da {
            // FTS5-Treffer mit Snippet aus dem Body (Spalte 1).
            "SELECT m.id, m.insilo_id, m.title, m.participants, m.tags,
                     m.meeting_date, m.duration_min, m.language, m.template, m.source_url,
                     snippet(meetings_fts, 1, '', '…', '<b>', 20) AS snippet
              FROM meetings m
              JOIN meetings_fts ON meetings_fts.rowid = m.id
              WHERE m.deleted = 0 AND meetings_fts MATCH ?1"
        } else if has_query {
            // Kein FTS5-Build → LIKE-Fallback (ohne Snippet).
            "SELECT id, insilo_id, title, participants, tags,
                    meeting_date, duration_min, language, template, source_url,
                    NULL AS snippet
              FROM meetings
              WHERE deleted = 0 AND (title LIKE ?1 OR body_md LIKE ?1)"
        } else {
            "SELECT id, insilo_id, title, participants, tags,
                    meeting_date, duration_min, language, template, source_url,
                    NULL AS snippet
              FROM meetings
              WHERE deleted = 0"
        };

        let mut sql = String::from(sql);
        let mut params: Vec<String> = Vec::new();
        if has_query {
            let query = q.query.as_deref().unwrap_or("");
            let pat = if fts_da {
                fts_match(query)
            } else {
                format!("%{}%", query.replace('%', "").replace('_', ""))
            };
            params.push(pat);
        }
        if let Some(tag) = q.tag.as_deref().filter(|s| !s.is_empty()) {
            params.push(format!("%\"{}\"%", tag.replace('"', "")));
            sql.push_str(&format!(" AND tags LIKE ?{}", params.len()));
        }
        if let Some(part) = q.participant.as_deref().filter(|s| !s.is_empty()) {
            params.push(format!("%\"{}\"%", part.replace('"', "")));
            sql.push_str(&format!(" AND participants LIKE ?{}", params.len()));
        }
        if let Some(from) = q.from.as_deref().filter(|s| !s.is_empty()) {
            params.push(from.to_string());
            sql.push_str(&format!(" AND date(meeting_date) >= date(?{})", params.len()));
        }
        if let Some(to) = q.to.as_deref().filter(|s| !s.is_empty()) {
            params.push(to.to_string());
            sql.push_str(&format!(" AND date(meeting_date) <= date(?{})", params.len()));
        }
        sql.push_str(" ORDER BY meeting_date DESC, id DESC LIMIT ? OFFSET ?");
        params.push(limit.to_string());
        params.push(offset.to_string());

        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(params.iter()),
                |r| {
                    Ok(MeetingInfo {
                        id: r.get(0)?,
                        insilo_id: r.get(1)?,
                        title: r.get(2)?,
                        participants: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
                        tags: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
                        meeting_date: r.get(5)?,
                        duration_min: r.get(6)?,
                        language: r.get(7)?,
                        template: r.get(8)?,
                        source_url: r.get(9)?,
                        snippet: r.get(10)?,
                    })
                },
            )
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    })?;
    Ok(Json(rows))
}

/// `GET /api/v1/meetings/:id` — voller Eintrag inkl. Markdown-Body.
pub async fn get_meeting(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<MeetingDetail> {
    let detail = with_db(&state, |conn| {
        conn.query_row(
            "SELECT id, insilo_id, title, participants, tags, meeting_date,
                    duration_min, language, template, source_url, body_md
             FROM meetings WHERE id = ?1 AND deleted = 0",
            rusqlite::params![id],
            |r| {
                Ok(MeetingDetail {
                    info: MeetingInfo {
                        id: r.get(0)?,
                        insilo_id: r.get(1)?,
                        title: r.get(2)?,
                        participants: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
                        tags: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
                        meeting_date: r.get(5)?,
                        duration_min: r.get(6)?,
                        language: r.get(7)?,
                        template: r.get(8)?,
                        source_url: r.get(9)?,
                        snippet: None,
                    },
                    body_md: r.get(10)?,
                })
            },
        )
        .map_err(|e| {
            if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                "Meeting nicht gefunden".to_string()
            } else {
                e.to_string()
            }
        })
    })?;
    Ok(Json(detail))
}

/// `POST /api/v1/meetings/scan` — sofortiger Scan (Button + Tests).
pub async fn trigger_scan(State(state): State<AppState>) -> ApiResult<crate::sync::insilo::ScanReport> {
    let report = run_insilo_scan(&state);
    Ok(Json(report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppState;

    fn state_with_db() -> AppState {
        let mut state = AppState::new();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::cache::db::init_db(&conn).unwrap();
        *state.cache_db.lock() = Some(conn);
        state
    }

    fn insert_meeting(state: &AppState, insilo_id: &str, title: &str) -> i64 {
        with_db(state, |conn| {
            conn.execute(
                "INSERT INTO meetings (insilo_id, path, sha256, title, meeting_date, body_md,
                                       first_seen_at, updated_at)
                 VALUES (?1, '/x.md', 'sha', ?2, '2026-09-12T09:00:00+00:00', 'Body',
                         datetime('now'), datetime('now'))",
                rusqlite::params![insilo_id, title],
            )
            .unwrap();
            let id: i64 = conn
                .query_row(
                    "SELECT id FROM meetings WHERE insilo_id = ?1",
                    rusqlite::params![insilo_id],
                    |r| r.get(0),
                )
                .unwrap();
            Ok(id)
        })
        .unwrap()
    }

    #[tokio::test]
    async fn delete_meeting_soft_deletes_and_tombstones() {
        let state = state_with_db();
        let id = insert_meeting(&state, "abc-123", "Test");
        let resp = delete_meeting(State(state.clone()), Path(id)).await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        let (deleted, ignored) = with_db(&state, |conn| {
            let d: i64 = conn
                .query_row("SELECT deleted FROM meetings WHERE id = ?1", rusqlite::params![id], |r| r.get(0))
                .unwrap();
            let i: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM meetings_ignored WHERE insilo_id = 'abc-123'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            Ok((d, i))
        })
        .unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(ignored, 1);
    }

    #[tokio::test]
    async fn delete_meeting_unknown_id_gives_404() {
        let state = state_with_db();
        let resp = delete_meeting(State(state), Path(9999)).await;
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_meeting_already_deleted_gives_404() {
        let state = state_with_db();
        let id = insert_meeting(&state, "abc-456", "Test 2");
        delete_meeting(State(state.clone()), Path(id)).await;
        let resp = delete_meeting(State(state), Path(id)).await;
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }
}

/// `DELETE /api/v1/meetings/:id` — Meeting aus Relay entfernen.
///
/// Soft-Delete + Tombstone in `meetings_ignored`: solange die Insilo-
/// Export-Datei existiert, würde der nächste Scan den Eintrag sonst
/// wiederherstellen (Upsert setzt `deleted = 0`). Der Tombstone bleibt
/// dauerhaft — ein erneutes Exportieren desselben Meetings bleibt unsichtbar.
pub async fn delete_meeting(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> axum::response::Response {
    let err_json = |status: axum::http::StatusCode, msg: &str| {
        (
            status,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            axum::body::Body::from(format!("{{\"error\": \"{}\"}}", msg.replace('"', "\\\""))),
        )
            .into_response()
    };

    let res: Result<(), String> = with_db(&state, |conn| {
        let insilo_id: String = conn
            .query_row(
                "SELECT insilo_id FROM meetings WHERE id = ?1 AND deleted = 0",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .map_err(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    "not_found".to_string()
                } else {
                    e.to_string()
                }
            })?;
        conn.execute(
            "UPDATE meetings SET deleted = 1, updated_at = datetime('now') WHERE id = ?1",
            rusqlite::params![id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR IGNORE INTO meetings_ignored (insilo_id, deleted_at) VALUES (?1, datetime('now'))",
            rusqlite::params![insilo_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    });

    match res {
        Ok(()) => (
            axum::http::StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            axum::body::Body::from("{\"ok\": true}"),
        )
            .into_response(),
        Err(e) if e == "not_found" => {
            err_json(axum::http::StatusCode::NOT_FOUND, "Meeting nicht gefunden")
        }
        Err(e) => err_json(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}
