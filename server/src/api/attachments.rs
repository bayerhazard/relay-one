//! Attachment maintenance endpoints (Phase 5 of the attachment-management
//! overhaul):
//!   - `POST /api/v1/attachments/gc`       — on-demand dedup-store GC.
//!   - `POST /api/v1/attachments/repair`   — consistency check + repair.
//!   - `GET  /api/v1/attachments/stats`    — cache stats for the web UI.
//!   - `POST /api/v1/attachments/cleanup`  — age-based content cleanup.
//!   - `POST /api/v1/attachments/clear`    — drop all cached content (keep metadata).

use axum::extract::State;
use axum::Json;
use serde::Deserialize;

use crate::db::with_db;
use crate::AppState;

/// `POST /api/v1/attachments/gc` — delete dedup-store files no longer
/// referenced by any `message_attachments.disk_path`.
pub async fn gc_attachments(State(state): State<AppState>) -> crate::api::ApiResult<serde_json::Value> {
    let report = with_db(&state, |conn| {
        crate::cache::attachments::gc_orphaned_attachments(conn, &state.data_root)
    })?;
    tracing::info!(
        "Attachment-GC: {} Dateien entfernt ({} bytes), {} behalten",
        report.removed_files, report.freed_bytes, report.kept_files
    );
    Ok(Json(serde_json::json!(report)))
}

/// `POST /api/v1/attachments/repair` — fix desynced `has_attachments` flags
/// and clear orphaned `disk_path` references. Body optional: `{"repair": bool}`.
#[derive(Deserialize)]
pub struct RepairRequest {
    #[serde(default = "default_true")]
    pub repair: bool,
}

fn default_true() -> bool {
    true
}

pub async fn repair_attachments(
    State(state): State<AppState>,
    Json(req): Json<RepairRequest>,
) -> crate::api::ApiResult<serde_json::Value> {
    let report = with_db(&state, |conn| {
        crate::cache::attachments::check_and_repair_attachments(conn, &state.data_root, req.repair)
    })?;
    tracing::info!(
        "Attachment-Repair (repair={}): flagged_without_rows={}, unflagged_with_rows={}, missing_file={}, repaired={}",
        req.repair, report.flagged_without_rows, report.unflagged_with_rows,
        report.rows_with_missing_file, report.repaired_rows
    );
    Ok(Json(serde_json::json!(report)))
}

/// `GET /api/v1/attachments/stats` — totals for the web cache panel.
pub async fn attachment_stats(State(state): State<AppState>) -> crate::api::ApiResult<serde_json::Value> {
    let stats = with_db(&state, |conn| {
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM message_attachments",
            [],
            |r| r.get(0),
        ).map_err(|e| e.to_string())?;
        let cached_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM message_attachments WHERE content_cached = 1",
            [],
            |r| r.get(0),
        ).map_err(|e| e.to_string())?;
        let cached_size_mb: f64 = conn.query_row(
            "SELECT COALESCE(SUM(size), 0) * 1.0 / (1024.0 * 1024.0) FROM message_attachments WHERE content_cached = 1",
            [],
            |r| r.get(0),
        ).unwrap_or(0.0);
        Ok(serde_json::json!({
            "total_attachments": total,
            "cached_count": cached_count,
            "cached_size_mb": cached_size_mb,
        }))
    })?;
    Ok(Json(stats))
}

/// `POST /api/v1/attachments/cleanup` — drop oldest cached content until under
/// `max_keep_mb`.
#[derive(Deserialize)]
pub struct CleanupRequest {
    #[serde(default = "default_max_keep")]
    pub max_keep_mb: f64,
}

fn default_max_keep() -> f64 {
    1024.0
}

pub async fn cleanup_attachments(
    State(state): State<AppState>,
    Json(req): Json<CleanupRequest>,
) -> crate::api::ApiResult<serde_json::Value> {
    let cleaned = with_db(&state, |conn| {
        crate::cache::attachments::cleanup_content(conn, req.max_keep_mb)
            .map_err(|e| e.to_string())
    })?;
    tracing::info!("Attachment-Cleanup (max {} MB): {} Anhänge geleert", req.max_keep_mb, cleaned);
    Ok(Json(serde_json::json!({ "cleaned": cleaned })))
}

/// `POST /api/v1/attachments/clear` — clear all cached content (keep metadata).
pub async fn clear_attachments(State(state): State<AppState>) -> crate::api::ApiResult<serde_json::Value> {
    let cleared = with_db(&state, |conn| {
        crate::cache::attachments::clear_all_content(conn).map_err(|e| e.to_string())
    })?;
    tracing::info!("Attachment-Clear: {} Anhänge geleert", cleared);
    Ok(Json(serde_json::json!({ "cleared": cleared })))
}