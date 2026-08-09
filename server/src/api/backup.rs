//! Backup snapshot (Concept §9).
//!
//! `POST /api/v1/archive/backup` creates a consistent SQLite snapshot via
//! `VACUUM INTO '<data>/backups/index-<timestamp>.db'` and returns the path +
//! size. The EML archive + attachments are already plain files under the same
//! data root, so copying `<data>/` captures everything (single backup root, F5).

use axum::extract::State;
use axum::Json;

use crate::AppState;
use crate::api::ApiResult;
use crate::db::with_db;

/// `POST /api/v1/archive/backup` — consistent DB snapshot via VACUUM INTO.
pub async fn create_backup(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let backups_dir = state.data_root.join("backups");
    std::fs::create_dir_all(&backups_dir).map_err(|e| format!("Backup-Verzeichnis: {e}"))?;

    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let target = backups_dir.join(format!("index-{ts}.db"));

    // VACUUM INTO must run on its own connection (no open transaction).
    let result = with_db(&state, |conn| {
        conn.execute_batch(&format!("VACUUM INTO '{}'", target.to_string_lossy()))
            .map_err(|e| format!("VACUUM INTO fehlgeschlagen: {e}"))?;
        Ok(())
    });

    match result {
        Ok(()) => {
            let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
            let rel = target
                .strip_prefix(&state.data_root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| target.to_string_lossy().to_string());
            tracing::info!("Backup erstellt: {} ({} bytes)", rel, size);
            Ok(Json(serde_json::json!({
                "ok": true,
                "path": rel,
                "size": size,
                "created_at": chrono::Utc::now().to_rfc3339(),
            })))
        }
        Err(e) => Err(crate::api::ApiError(e)),
    }
}
