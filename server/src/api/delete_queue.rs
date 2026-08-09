//! Delete-queue review endpoints (Concept §5).

use axum::extract::State;
use axum::Json;
use serde::Deserialize;

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::db::with_db;

/// `GET /api/v1/archive/delete-queue` — pending + failed rows for review.
pub async fn list_delete_queue(State(state): State<AppState>) -> ApiResult<Vec<serde_json::Value>> {
    let rows = with_db(&state, |conn| {
        crate::cache::delete_queue::list_by_state(conn, None).map_err(|e| e.to_string())
    })?;
    Ok(Json(rows.iter().map(|r| serde_json::json!({
        "id": r.id,
        "message_id": r.message_id,
        "account_id": r.account_id,
        "uid": r.uid,
        "folder": r.folder,
        "action": r.action,
        "state": r.state,
        "attempts": r.attempts,
        "last_error": r.last_error,
    })).collect()))
}

#[derive(Deserialize)]
pub struct RetryRequest {
    pub id: i64,
}

/// `POST /api/v1/archive/delete-queue/{id}/retry` — reset a failed row to
/// pending so the worker retries it.
pub async fn retry_delete_queue(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> ApiResult<serde_json::Value> {
    let updated = with_db(&state, |conn| {
        let n = conn.execute(
            "UPDATE delete_queue SET state = 'pending', attempts = 0, last_error = NULL, updated_at = datetime('now') WHERE id = ?1",
            rusqlite::params![id],
        ).map_err(|e| e.to_string())?;
        Ok(n)
    })?;
    if updated == 0 {
        return Err(ApiError(format!("Queue-Eintrag {} nicht gefunden", id)));
    }
    Ok(Json(serde_json::json!({ "ok": true, "id": id })))
}

/// `POST /api/v1/archive/delete-queue/{id}/remove` — drop a row without
/// retrying (user reviewed it; provider deletion is abandoned).
pub async fn remove_delete_queue(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> ApiResult<serde_json::Value> {
    let updated = with_db(&state, |conn| {
        let n = conn.execute(
            "DELETE FROM delete_queue WHERE id = ?1",
            rusqlite::params![id],
        ).map_err(|e| e.to_string())?;
        Ok(n)
    })?;
    if updated == 0 {
        return Err(ApiError(format!("Queue-Eintrag {} nicht gefunden", id)));
    }
    Ok(Json(serde_json::json!({ "ok": true, "id": id })))
}
