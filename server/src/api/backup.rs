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

/// `GET /api/v1/archive/backups` — list available backup snapshots.
pub async fn list_backups(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let backups_dir = state.data_root.join("backups");
    let mut snapshots = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&backups_dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("index-") && name.ends_with(".db") {
                if let Ok(meta) = e.metadata() {
                    snapshots.push(serde_json::json!({
                        "name": name,
                        "size": meta.len(),
                        "modified": meta.modified().ok().map(|t| {
                            let d: chrono::DateTime<chrono::Utc> = t.into();
                            d.to_rfc3339()
                        }),
                    }));
                }
            }
        }
    }
    snapshots.sort_by(|a, b| b["name"].as_str().cmp(&a["name"].as_str()));
    Ok(Json(serde_json::json!({ "backups": snapshots })))
}

/// `POST /api/v1/archive/restore` — restore the index.db from a backup
/// snapshot in backups/. The current DB is kept as a safety copy first.
#[derive(serde::Deserialize)]
pub struct RestoreRequest {
    pub backup_name: String,
}

/// A valid snapshot name is a bare filename matching the `index-*.db`
/// pattern produced by `create_backup`/`list_backups`. Anything else (path
/// separators, `..`, absolute paths, foreign extensions) is rejected to
/// prevent path traversal out of the backups directory.
fn is_valid_backup_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && name.starts_with("index-")
        && name.ends_with(".db")
        && std::path::Path::new(name)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

pub async fn restore_backup(
    State(state): State<AppState>,
    Json(req): Json<RestoreRequest>,
) -> ApiResult<serde_json::Value> {
    if !is_valid_backup_name(&req.backup_name) {
        return Err(crate::api::ApiError(format!(
            "Ungültiger Backup-Name '{}' (nur 'index-*.db' erlaubt)",
            req.backup_name
        )));
    }
    let backups_dir = state.data_root.join("backups");
    let src = backups_dir.join(&req.backup_name);
    // Defense in depth: canonicalize and re-verify the snapshot stays inside
    // the backups directory (guards against symlink/edge cases).
    let src = src.canonicalize().map_err(|_| {
        crate::api::ApiError(format!("Backup '{}' nicht gefunden", req.backup_name))
    })?;
    let backups_dir_canonical = backups_dir.canonicalize().map_err(|e| {
        crate::api::ApiError(format!("Backup-Verzeichnis: {e}"))
    })?;
    if !src.starts_with(&backups_dir_canonical) {
        return Err(crate::api::ApiError(format!(
            "Backup '{}' liegt außerhalb des Backup-Verzeichnisses",
            req.backup_name
        )));
    }
    if !src.exists() {
        return Err(crate::api::ApiError(format!("Backup '{}' nicht gefunden", req.backup_name)));
    }

    let db_path = state
        .db_path
        .lock()
        .as_ref()
        .ok_or_else(|| crate::api::ApiError("DB-Pfad unbekannt".into()))?
        .clone();

    // 1. Safety copy of the current DB.
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let safety = backups_dir.join(format!("pre-restore-{ts}.db"));
    std::fs::copy(&db_path, &safety).map_err(|e| crate::api::ApiError(format!("Sicherung: {e}")))?;

    // 2. Replace the live DB file. The server keeps the old connection open;
    //    a restart picks up the restored file. We signal a graceful shutdown
    //    so the pod restarts automatically with the restored data.
    let restored_bytes = std::fs::metadata(&src).map(|m| m.len()).unwrap_or(0);
    std::fs::copy(&src, &db_path).map_err(|e| crate::api::ApiError(format!("Restore: {e}")))?;

    tracing::warn!(
        "Restore: DB aus '{}' wiederhergestellt ({} bytes). Sicherung: {} — Server wird neu gestartet.",
        req.backup_name, restored_bytes, safety.file_name().unwrap_or_default().to_string_lossy()
    );

    // Signal shutdown → the deployment restarts the pod, which loads the
    // restored DB (WAL/schema are re-initialized on boot).
    if let Some(tx) = state.sync_shutdown_tx.lock().as_ref() {
        let _ = tx.send(());
    }

    Ok(Json(serde_json::json!({
        "ok": true,
        "restored": req.backup_name,
        "bytes": restored_bytes,
        "safety_copy": safety.file_name().unwrap_or_default().to_string_lossy().to_string(),
        "note": "Server wird neu gestartet, um die wiederhergestellte DB zu laden",
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_backup_names() {
        assert!(is_valid_backup_name("index-20260818-120000.db"));
        assert!(is_valid_backup_name("index-x.db"));
    }

    #[test]
    fn rejects_traversal_and_foreign_names() {
        for name in [
            "..",
            "../index-2026.db",
            "../../etc/passwd",
            "sub/index-2026.db",
            "index-2026.db/..",
            "/etc/passwd",
            "index-2026.tgz",
            "foo.db",
            "index-2026.db.backup",
            "index-",
            "",
            "index-2026.db/x",
        ] {
            assert!(!is_valid_backup_name(name), "should reject {name:?}");
        }
    }
}
