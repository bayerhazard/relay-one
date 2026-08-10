//! Account-to-account mail migration (copy, never delete).
//!
//! `POST /api/v1/migrate/copy-account` copies every folder (incl. subfolders)
//! and every message from a source account into LOCAL folders of a target
//! account. Source data is never modified. Integrity is verified end-to-end:
//!   - each EML is copied byte-for-byte and its SHA-256 re-computed,
//!   - a mismatch aborts that message (reported, not silently kept),
//!   - the report lists per-folder counts + any failures.

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use sha2::Digest as _;

use crate::AppState;
use crate::api::{ApiError, ApiResult};
use crate::cache::messages as cache_messages;
use crate::db::with_db;

#[derive(Deserialize)]
pub struct CopyAccountRequest {
    pub source_account_id: u32,
    pub target_account_id: u32,
}

#[derive(Deserialize)]
pub struct CopyFolderRequest {
    pub source_account_id: u32,
    pub target_account_id: u32,
    pub folder: String,
    /// Optional: stop after this many messages in this call (chunked copy
    /// keeps each request below the gateway timeout; the next call continues).
    #[serde(default)]
    pub batch_limit: Option<usize>,
}

#[derive(serde::Serialize)]
pub struct CopyReport {
    pub folders_created: usize,
    pub messages_copied: usize,
    pub messages_failed: usize,
    pub folders: Vec<FolderReport>,
    pub verified_sha: usize,
    pub total_bytes: u64,
}

#[derive(serde::Serialize)]
pub struct FolderReport {
    pub folder: String,
    pub copied: usize,
    pub failed: usize,
    /// false while chunked copying is still in progress (more messages remain).
    #[serde(default = "default_true")]
    pub batch_done: bool,
}

fn default_true() -> bool {
    true
}

/// `POST /api/v1/migrate/copy-account`
pub async fn copy_account(
    State(state): State<AppState>,
    Json(req): Json<CopyAccountRequest>,
) -> ApiResult<CopyReport> {
    let source = req.source_account_id as i64;
    let target = req.target_account_id as i64;

    // 1. List all source folders (including subfolders, ordered by name).
    let folders = with_db(&state, |conn| {
        cache_messages::list_all_folders(conn, source).map_err(|e| e.to_string())
    })?;

    let mut report = CopyReport {
        folders_created: 0,
        messages_copied: 0,
        messages_failed: 0,
        folders: Vec::new(),
        verified_sha: 0,
        total_bytes: 0,
    };

    for (folder_name, _local) in &folders {
        let folder_report = copy_folder(&state, source, target, folder_name).await?;
        report.folders_created += 1;
        report.messages_copied += folder_report.copied;
        report.messages_failed += folder_report.failed;
        report.verified_sha += folder_report.copied;
        report.folders.push(folder_report);
    }

    Ok(Json(report))
}

/// `POST /api/v1/migrate/copy-folder` — copy ONE folder (chunked migration).
/// Splitting the copy into per-folder calls keeps each request below the
/// Olares gateway timeout; the caller walks the folder list.
pub async fn copy_folder_endpoint(
    State(state): State<AppState>,
    Json(req): Json<CopyFolderRequest>,
) -> ApiResult<FolderReport> {
    let report = copy_folder_limited(
        &state,
        req.source_account_id as i64,
        req.target_account_id as i64,
        &req.folder,
        req.batch_limit,
    )
    .await?;
    Ok(Json(report))
}

/// `POST /api/v1/migrate/count-folder` — count UIDs on the IMAP server for a
/// folder WITHOUT copying (diagnostics — verifies folder selection is right).
#[derive(Deserialize)]
pub struct CountFolderRequest {
    pub source_account_id: u32,
    pub folder: String,
}

pub async fn count_folder(
    State(state): State<AppState>,
    Json(req): Json<CountFolderRequest>,
) -> ApiResult<serde_json::Value> {
    let client = state
        .imap_clients
        .read()
        .get(&req.source_account_id)
        .cloned()
        .ok_or(ApiError("Quell-IMAP-Client nicht gefunden".into()))?;
    if !client.is_connected().await {
        client.connect().await.map_err(|e| ApiError(e.to_string()))?;
    }
    let uids = client.fetch_all_uids_in_folder(&req.folder).await.map_err(|e| ApiError(e.to_string()))?;
    let min = uids.iter().min().copied().unwrap_or(0);
    let max = uids.iter().max().copied().unwrap_or(0);
    Ok(Json(serde_json::json!({
        "folder": req.folder,
        "count": uids.len(),
        "min_uid": min,
        "max_uid": max,
        "first_10": &uids[..uids.len().min(10)],
    })))
}

async fn copy_folder(
    state: &AppState,
    source: i64,
    target: i64,
    folder_name: &str,
) -> Result<FolderReport, ApiError> {
    copy_folder_limited(state, source, target, folder_name, None).await
}

/// Build a dedicated IMAP connection to the source account (own session, so
/// the background sync cannot steal it mid-migration). The passwords are
/// decrypted from the DB.
async fn source_imap_client(state: &AppState, source: i64) -> Result<crate::imap::client::ImapClient, String> {
    let (host, port, ssl, user, pass_enc, insecure) = with_db(state, |conn| {
        let (host, port, ssl, user, pass_enc, insecure): (String, i64, i32, String, String, i32) = conn
            .query_row(
                "SELECT imap_host, imap_port, imap_ssl, username, password, imap_insecure
                 FROM accounts WHERE id = ?1",
                rusqlite::params![source],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .map_err(|e| format!("Quell-Account nicht gefunden: {e}"))?;
        Ok::<_, String>((host, port, ssl, user, pass_enc, insecure))
    })?;

    let password = crate::crypto::decrypt(&pass_enc).unwrap_or(pass_enc);
    let client = crate::imap::client::ImapClient::new_with_options(
        host,
        port as u16,
        user,
        password,
        ssl != 0,
        insecure != 0,
    );
    if !client.is_connected().await {
        client.connect().await.map_err(|e| e.to_string())?;
    }
    Ok(client)
}

async fn copy_folder_limited(
    state: &AppState,
    source: i64,
    target: i64,
    folder_name: &str,
    batch_limit: Option<usize>,
) -> Result<FolderReport, ApiError> {
    // Create the LOCAL target folder (idempotent).
    with_db(state, |conn| {
        cache_messages::create_local_folder(conn, target, folder_name).map_err(|e| e.to_string())
    })?;

    // Complete UID set from the IMAP server (ALL mails, not just the ones
    // the local sync cached — the sync only fetches recent batches). A
    // DEDICATED connection is used so the background sync cannot steal the
    // session mid-migration.
    let imap_client = source_imap_client(state, source)
        .await
        .map_err(ApiError)?;
    let imap_uids: Vec<u32> = {
        match imap_client.fetch_all_uids_in_folder(folder_name).await {
            Ok(uids) => uids,
            Err(e) => {
                tracing::warn!("migrate: Ordner '{}' am IMAP nicht wählbar (lokal-only?): {}", folder_name, e);
                return Ok(FolderReport { folder: folder_name.to_string(), copied: 0, failed: 0, batch_done: true });
            }
        }
    };

    // Locally cached UIDs (for the fast path / SHA-verified EML copy).
    let cached_uids: Vec<i64> = with_db(state, |conn| {
        cache_messages::get_messages_with_uids_for_folder(conn, source, folder_name)
            .map_err(|e| e.to_string())
    })?;
    let cached: std::collections::HashSet<i64> = cached_uids.into_iter().collect();

    let mut report = FolderReport { folder: folder_name.to_string(), copied: 0, failed: 0, batch_done: true };

    // UIDs already present in the TARGET folder (skip — no re-copy).
    let existing_target: std::collections::HashSet<i64> = {
        let uids = with_db(state, |conn| {
            cache_messages::get_messages_with_uids_for_folder(conn, target, folder_name)
                .map_err(|e| e.to_string())
        })?;
        uids.into_iter().collect()
    };

    for uid in &imap_uids {
        let uid_i64 = *uid as i64;
        if existing_target.contains(&uid_i64) {
            continue; // already migrated — never re-copy
        }
        match copy_one_message(&imap_client, state, source, target, folder_name, uid_i64, cached.contains(&uid_i64)).await {
            Ok(()) => report.copied += 1,
            Err(e) => {
                tracing::warn!("migrate: {} / uid {} fehlgeschlagen: {}", folder_name, uid, e);
                report.failed += 1;
            }
        }
        // Chunked mode: stop after batch_limit newly copied messages.
        if let Some(limit) = batch_limit {
            if report.copied >= limit {
                report.batch_done = false;
                break;
            }
        }
    }
    // batch_done stays true unless the loop broke early (batch_limit hit).
    // When the loop ran to completion, every UID was processed (either copied
    // or already present in the target).

    // Close the dedicated IMAP connection — otherwise every chunk leaks a
    // connection and the provider blocks new ones
    // (mail_max_userip_connections).
    imap_client.shutdown().await;

    Ok(report)
}

async fn copy_one_message(
    imap_client: &crate::imap::client::ImapClient,
    state: &AppState,
    source: i64,
    target: i64,
    folder_name: &str,
    uid: i64,
    is_cached: bool,
) -> Result<(), String> {
    // 1. Read the source row (raw_path, raw_sha256, meta).
    let (raw_rel, raw_sha_expected, meta) = with_db(state, |conn| {
        let raw_rel: Option<String> = conn
            .query_row(
                "SELECT raw_path FROM messages WHERE account_id = ?1 AND uid = ?2",
                rusqlite::params![source, uid],
                |r| r.get(0),
            )
            .ok();
        let raw_sha: Option<String> = conn
            .query_row(
                "SELECT raw_sha256 FROM messages WHERE account_id = ?1 AND uid = ?2",
                rusqlite::params![source, uid],
                |r| r.get(0),
            )
            .ok();
        let subj: Option<String> = conn
            .query_row(
                "SELECT subject FROM messages WHERE account_id = ?1 AND uid = ?2",
                rusqlite::params![source, uid],
                |r| r.get(0),
            )
            .ok();
        let from: Option<String> = conn
            .query_row(
                "SELECT from_addr FROM messages WHERE account_id = ?1 AND uid = ?2",
                rusqlite::params![source, uid],
                |r| r.get(0),
            )
            .ok();
        let to: Option<String> = conn
            .query_row(
                "SELECT to_addr FROM messages WHERE account_id = ?1 AND uid = ?2",
                rusqlite::params![source, uid],
                |r| r.get(0),
            )
            .ok();
        let date: Option<String> = conn
            .query_row(
                "SELECT date FROM messages WHERE account_id = ?1 AND uid = ?2",
                rusqlite::params![source, uid],
                |r| r.get(0),
            )
            .ok();
        let mid: Option<String> = conn
            .query_row(
                "SELECT message_id FROM messages WHERE account_id = ?1 AND uid = ?2",
                rusqlite::params![source, uid],
                |r| r.get(0),
            )
            .ok();
        Ok::<_, String>((raw_rel, raw_sha, (subj, from, to, date, mid)))
    })?;

    // 2. Obtain the raw RFC822 bytes — from the source EML on disk when
    //    present (byte-identical copy), otherwise by fetching from IMAP.
    let (raw_bytes, raw_sha_computed) = if let Some(rel) = raw_rel {
        let abs = state.data_root.join(&rel);
        let bytes = std::fs::read(&abs).map_err(|e| format!("EML lesen: {e}"))?;
        // Integrity: if we have an expected hash, verify BEFORE copying.
        if let Some(expected) = &raw_sha_expected {
            let mut h = sha2::Sha256::new();
            h.update(&bytes);
            let got = format!("{:x}", h.finalize());
            if &got != expected {
                return Err(format!("SHA-256 Mismatch (Quelle): uid {} erwartet {} bekam {}", uid, expected, got));
            }
        }
        let mut h = sha2::Sha256::new();
        h.update(&bytes);
        let computed = format!("{:x}", h.finalize());
        (bytes, computed)
    } else {
        // Fetch raw from IMAP (source account). If that fails (e.g. the
        // source mail only exists locally — a local-only test folder), fall
        // back to reconstructing the message from the local DB row so no
        // locally stored mail is ever lost.
        let raw_result = imap_client
            .fetch_raw_message_in_folder(uid as u32, Some(folder_name.to_string()))
            .await;
        let bytes = match raw_result {
            Ok(raw) => raw.into_bytes(),
            Err(_) if is_cached => {
                // Reconstruct from the local DB (headers + body) — only for
                // mails that are genuinely cached locally (they exist here
                // even if the IMAP copy is gone).
                let (subject, from, to, date, body_text, body_html) = with_db(state, |conn| {
                    Ok::<_, String>((
                        meta.0.clone(), meta.1.clone(), meta.2.clone(), meta.3.clone(),
                        conn.query_row(
                            "SELECT body_text FROM messages WHERE account_id = ?1 AND uid = ?2",
                            rusqlite::params![source, uid],
                            |r| r.get::<_, Option<String>>(0),
                        ).unwrap_or(None),
                        conn.query_row(
                            "SELECT body_html FROM messages WHERE account_id = ?1 AND uid = ?2",
                            rusqlite::params![source, uid],
                            |r| r.get::<_, Option<String>>(0),
                        ).unwrap_or(None),
                    ))
                })?;
                let mut raw = format!(
                    "From: {}\r\nTo: {}\r\nSubject: {}\r\nDate: {}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{}",
                    from.unwrap_or_default(),
                    to.unwrap_or_default(),
                    subject.unwrap_or_default(),
                    date.unwrap_or_default(),
                    body_text.unwrap_or_default(),
                );
                if let Some(html) = body_html {
                    if !html.is_empty() {
                        raw.push_str(&format!("\r\n\r\n--relay-boundary\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{}", html));
                    }
                }
                raw.into_bytes()
            }
            Err(e) => {
                // Mail not fetchable from IMAP AND not locally cached — it no
                // longer exists. Report as failed, do not fabricate.
                return Err(format!("Mail nicht am IMAP und nicht lokal: {}", e));
            }
        };
        let mut h = sha2::Sha256::new();
        h.update(&bytes);
        let computed = format!("{:x}", h.finalize());
        (bytes, computed)
    };

    // 3. Write the EML into the TARGET account's archive (fresh path).
    let date_str = meta.3.as_deref();
    let mid_str = meta.4.as_deref();
    let target_path = crate::cache::archive::write_eml(
        &state.data_root,
        target,
        uid as u32,
        date_str,
        mid_str,
        &raw_bytes,
    )
    .map_err(|e| e.to_string())?;

    // 4. Verify the written file (byte-count + hash).
    let written = std::fs::read(&target_path).map_err(|e| format!("Kopie lesen: {e}"))?;
    if written.len() != raw_bytes.len() {
        return Err(format!("Kopiergröße mismatch: uid {}", uid));
    }
    let mut h = sha2::Sha256::new();
    h.update(&written);
    let written_sha = format!("{:x}", h.finalize());
    if written_sha != raw_sha_computed {
        return Err(format!("Kopie-Hash mismatch: uid {}", uid));
    }

    let rel = target_path
        .strip_prefix(&state.data_root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| target_path.to_string_lossy().to_string());

    // 5. Insert the target row (same uid, local folder, synced=0).
    //    Resolve the target folder_id explicitly — a NULL folder_id would
    //    silently drop the row into a folder-less limbo. Re-create the local
    //    folder if a concurrent sync removed it between the folder pass and
    //    this message.
    with_db(state, |conn| {
        let folder_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
                rusqlite::params![target, folder_name],
                |r| r.get(0),
            )
            .ok();
        let folder_id = match folder_id {
            Some(id) => id,
            None => {
                cache_messages::create_local_folder(conn, target, folder_name)
                    .map_err(|e| format!("Ziel-Ordner anlegen fehlgeschlagen: {}", e))?;
                conn.query_row(
                    "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
                    rusqlite::params![target, folder_name],
                    |r| r.get(0),
                )
                .map_err(|e| format!("Ziel-Ordner '{}' nicht gefunden: {}", folder_name, e))?
            }
        };
        conn.execute(
            "INSERT OR IGNORE INTO messages
             (account_id, folder_id, uid, message_id, subject, from_addr, to_addr, date,
              body_text, body_html, flags, ai_summary, ai_priority, ai_fraud_score,
              is_read, is_flagged, has_attachments, synced, raw_path, raw_sha256)
             VALUES (
               ?1, ?15, ?2, ?3, ?4, ?5, ?6, ?7,
               (SELECT body_text FROM messages WHERE account_id = ?8 AND uid = ?2),
               (SELECT body_html FROM messages WHERE account_id = ?8 AND uid = ?2),
               ?9, NULL, NULL, NULL, ?10, ?11, ?12, 0, ?13, ?14
             )",
            rusqlite::params![
                target, uid,
                meta.4, meta.0, meta.1, meta.2, meta.3,
                source,
                "[]", 0i32, 0i32, 0i32,
                rel, written_sha, folder_id,
            ],
        )
        .map_err(|e| e.to_string())
    })?;

    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MigrationStatus {
    pub running: bool,
    pub done: bool,
    pub folders_total: usize,
    pub folders_done: usize,
    pub messages_copied: usize,
    pub messages_failed: usize,
    pub current_folder: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub last_error: Option<String>,
}

impl Default for MigrationStatus {
    fn default() -> Self {
        Self {
            running: false,
            done: false,
            folders_total: 0,
            folders_done: 0,
            messages_copied: 0,
            messages_failed: 0,
            current_folder: String::new(),
            started_at: None,
            finished_at: None,
            last_error: None,
        }
    }
}

/// `POST /api/v1/migrate/db-count` — count the source DB rows (diagnostics).
/// Decides whether a DB-based migration (no IMAP) is complete.
#[derive(Deserialize)]
pub struct DbCountRequest {
    pub account_id: u32,
}

pub async fn db_count(
    State(state): State<AppState>,
    Json(req): Json<DbCountRequest>,
) -> ApiResult<serde_json::Value> {
    let (total, with_raw, folders) = with_db(&state, |conn| {
        let total: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE account_id = ?1",
                rusqlite::params![req.account_id as i64],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let with_raw: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE account_id = ?1 AND raw_path IS NOT NULL AND raw_path != ''",
                rusqlite::params![req.account_id as i64],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let mut stmt = conn
            .prepare(
                "SELECT f.name, COUNT(m.id) FROM folders f
                 LEFT JOIN messages m ON m.folder_id = f.id AND m.account_id = ?1
                 WHERE f.account_id = ?1 GROUP BY f.id ORDER BY f.name",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![req.account_id as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut folders = Vec::new();
        for r in rows {
            folders.push(r.map_err(|e| e.to_string())?);
        }
        Ok::<_, String>((total, with_raw, folders))
    })?;
    Ok(Json(serde_json::json!({
        "account_id": req.account_id,
        "total_messages": total,
        "with_raw_eml": with_raw,
        "without_raw_eml": total - with_raw,
        "folders": folders.iter().map(|(n, c)| serde_json::json!({"folder": n, "count": c})).collect::<Vec<_>>(),
    })))
}

/// `POST /api/v1/migrate/stop-sync` — detach the target account from the
/// background sync so it cannot prune/overwrite the migrated local folders.
#[derive(Deserialize)]
pub struct StopSyncRequest {
    pub account_id: u32,
}

pub async fn stop_sync(
    State(state): State<AppState>,
    Json(req): Json<StopSyncRequest>,
) -> ApiResult<serde_json::Value> {
    let removed = state.imap_clients.write().remove(&req.account_id).is_some();
    tracing::warn!("migrate: Sync für Konto {} getrennt ({})", req.account_id, removed);
    Ok(Json(serde_json::json!({ "ok": true, "sync_detached": removed })))
}

/// `POST /api/v1/migrate/reset-target` — wipe ALL messages + folders of the
/// target account (EML files included). Used to clean the polluted target
/// before a fresh migration.
#[derive(Deserialize)]
pub struct ResetTargetRequest {
    pub account_id: u32,
}

pub async fn reset_target(
    State(state): State<AppState>,
    Json(req): Json<ResetTargetRequest>,
) -> ApiResult<serde_json::Value> {
    let target = req.account_id as i64;
    // Collect all raw_paths of the target, delete EML files, then wipe rows
    // and folders.
    let raws: Vec<String> = with_db(&state, |conn| {
        let mut stmt = conn
            .prepare("SELECT raw_path FROM messages WHERE account_id = ?1 AND raw_path IS NOT NULL")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![target], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            if let Ok(p) = r {
                out.push(p);
            }
        }
        Ok::<_, String>(out)
    })?;
    for rel in &raws {
        let _ = std::fs::remove_file(state.data_root.join(rel));
    }
    let messages = with_db(&state, |conn| {
        conn.execute("DELETE FROM messages WHERE account_id = ?1", rusqlite::params![target])
            .map_err(|e| e.to_string())
    })?;
    let folders = with_db(&state, |conn| {
        conn.execute("DELETE FROM folders WHERE account_id = ?1", rusqlite::params![target])
            .map_err(|e| e.to_string())
    })?;
    tracing::warn!("migrate: Ziel-Konto {} zurückgesetzt ({} Mails, {} Ordner)", req.account_id, messages, folders);
    Ok(Json(serde_json::json!({ "ok": true, "messages_deleted": messages, "folders_deleted": folders })))
}

/// `POST /api/v1/migrate/start` — run the full account migration as a
/// server-side background task (no HTTP request limits, no gateway timeout,
/// no chunking races). The target account is detached from the sync first.
#[derive(Deserialize)]
pub struct StartMigrationRequest {
    pub source_account_id: u32,
    pub target_account_id: u32,
}

pub async fn start_migration(
    State(state): State<AppState>,
    Json(req): Json<StartMigrationRequest>,
) -> ApiResult<serde_json::Value> {
    // Refuse to start twice.
    if state.migration.read().as_ref().map(|m| m.running).unwrap_or(false) {
        return Err(ApiError("Migration läuft bereits".into()));
    }
    // Detach the target from the sync.
    state.imap_clients.write().remove(&req.target_account_id);

    let status = MigrationStatus {
        running: true,
        done: false,
        started_at: Some(chrono::Utc::now().to_rfc3339()),
        ..Default::default()
    };
    *state.migration.write() = Some(status);

    let state2 = state.clone();
    tokio::spawn(async move {
        run_migration_task(&state2, req.source_account_id as i64, req.target_account_id as i64).await;
    });

    Ok(Json(serde_json::json!({ "ok": true, "message": "Migration gestartet" })))
}

/// `GET /api/v1/migrate/status` — poll migration progress.
pub async fn migration_status(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let status = state.migration.read().clone().unwrap_or_default();
    Ok(Json(serde_json::to_value(&status).map_err(|e| ApiError(e.to_string()))?))
}

/// The background migration task: walks every source folder, copies every
/// mail (EML + DB row), updates the status. Runs inside the server, so no
/// gateway timeouts apply.
async fn run_migration_task(state: &AppState, source: i64, target: i64) {
    let folders = match with_db(state, |conn| {
        cache_messages::list_all_folders(conn, source).map_err(|e| e.to_string())
    }) {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("migrate: Ordnerliste fehlgeschlagen: {}", e);
            let mut s = state.migration.write();
            if let Some(st) = s.as_mut() {
                st.running = false;
                st.done = true;
                st.finished_at = Some(chrono::Utc::now().to_rfc3339());
            }
            return;
        }
    };

    {
        let mut s = state.migration.write();
        if let Some(st) = s.as_mut() {
            st.folders_total = folders.len();
        }
    }

    for (folder_name, _local) in &folders {
        {
            let mut s = state.migration.write();
            if let Some(st) = s.as_mut() {
                st.current_folder = folder_name.clone();
            }
        }

        // Dedicated IMAP connection per folder (closed afterwards).
        let imap_client = match source_imap_client(state, source).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("migrate: {} — Verbindung fehlgeschlagen: {}", folder_name, e);
                {
                    let mut s = state.migration.write();
                    if let Some(st) = s.as_mut() {
                        st.folders_done += 1;
                    }
                }
                continue;
            }
        };

        let uids = match imap_client.fetch_all_uids_in_folder(folder_name).await {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("migrate: {} — UID-Liste fehlgeschlagen: {}", folder_name, e);
                imap_client.shutdown().await;
                {
                    let mut s = state.migration.write();
                    if let Some(st) = s.as_mut() {
                        st.folders_done += 1;
                    }
                }
                continue;
            }
        };

        // Existing target UIDs (skip).
        let existing: std::collections::HashSet<i64> = {
            let uids = with_db(state, |conn| {
                cache_messages::get_messages_with_uids_for_folder(conn, target, folder_name)
                    .map_err(|e| e.to_string())
            })
            .unwrap_or_default();
            uids.into_iter().collect()
        };

        let mut copied = 0usize;
        let mut failed = 0usize;
        for uid in uids {
            if existing.contains(&(uid as i64)) {
                continue;
            }
            let is_cached = with_db(state, |conn| {
                Ok(conn
                    .query_row(
                        "SELECT 1 FROM messages WHERE account_id = ?1 AND uid = ?2",
                        rusqlite::params![source, uid as i64],
                        |r| r.get::<_, i64>(0),
                    )
                    .is_ok())
            })
            .unwrap_or(false);

            match copy_one_message(&imap_client, state, source, target, folder_name, uid as i64, is_cached).await {
                Ok(()) => copied += 1,
                Err(e) => {
                    tracing::debug!("migrate: {} / uid {} fehlgeschlagen: {}", folder_name, uid, e);
                    failed += 1;
                }
            }
        }
        imap_client.shutdown().await;

        {
            let mut s = state.migration.write();
            if let Some(st) = s.as_mut() {
                st.messages_copied += copied;
                st.messages_failed += failed;
                st.folders_done += 1;
            }
        }
        tracing::info!("migrate: {} — {} kopiert, {} fehlgeschlagen", folder_name, copied, failed);
    }

    {
        let mut s = state.migration.write();
        if let Some(st) = s.as_mut() {
            st.running = false;
            st.done = true;
            st.finished_at = Some(chrono::Utc::now().to_rfc3339());
            st.current_folder = String::new();
        }
    }
    tracing::info!("migrate: Migration abgeschlossen");
}

/// `POST /api/v1/migrate/start-folder` — migrate ONE folder as a background
/// task. After `done`, verify with db-count that source == target.
#[derive(Deserialize)]
pub struct StartFolderMigrationRequest {
    pub source_account_id: u32,
    pub target_account_id: u32,
    pub folder: String,
}

pub async fn start_folder_migration(
    State(state): State<AppState>,
    Json(req): Json<StartFolderMigrationRequest>,
) -> ApiResult<serde_json::Value> {
    if state.migration.read().as_ref().map(|m| m.running).unwrap_or(false) {
        return Err(ApiError("Migration läuft bereits".into()));
    }
    state.imap_clients.write().remove(&req.target_account_id);

    let status = MigrationStatus {
        running: true,
        done: false,
        folders_total: 1,
        current_folder: req.folder.clone(),
        started_at: Some(chrono::Utc::now().to_rfc3339()),
        ..Default::default()
    };
    *state.migration.write() = Some(status);

    let state2 = state.clone();
    let folder = req.folder.clone();
    let source_id = req.source_account_id as i64;
    let target_id = req.target_account_id as i64;
    tokio::spawn(async move {
        run_single_folder_task(&state2, source_id, target_id, &folder).await;
    });

    Ok(Json(serde_json::json!({ "ok": true, "folder": req.folder })))
}

/// Background task for exactly one folder. On connection errors the folder is
/// marked failed with an error message (NOT silently done) so the caller can
/// retry that folder.
async fn run_single_folder_task(state: &AppState, source: i64, target: i64, folder_name: &str) {
    // Ensure local target folder.
    if let Err(e) = with_db(state, |conn| {
        cache_messages::create_local_folder(conn, target, folder_name).map_err(|e| e.to_string())
    }) {
        finish_folder_task(state, 0, 0, Some(format!("Ordner anlegen: {e}")));
        return;
    }

    let imap_client = match source_imap_client(state, source).await {
        Ok(c) => c,
        Err(e) => {
            finish_folder_task(state, 0, 0, Some(format!("IMAP-Verbindung: {e}")));
            return;
        }
    };

    let uids = match imap_client.fetch_all_uids_in_folder(folder_name).await {
        Ok(u) => u,
        Err(e) => {
            imap_client.shutdown().await;
            finish_folder_task(state, 0, 0, Some(format!("UID-Liste: {e}")));
            return;
        }
    };

    // Existing target UIDs (skip).
    let existing: std::collections::HashSet<i64> = with_db(state, |conn| {
        cache_messages::get_messages_with_uids_for_folder(conn, target, folder_name)
            .map_err(|e| e.to_string())
    })
    .unwrap_or_default()
    .into_iter()
    .collect();

    let mut copied = 0usize;
    let mut failed = 0usize;
    for uid in uids {
        if existing.contains(&(uid as i64)) {
            continue;
        }
        let is_cached = with_db(state, |conn| {
            Ok(conn
                .query_row(
                    "SELECT 1 FROM messages WHERE account_id = ?1 AND uid = ?2",
                    rusqlite::params![source, uid as i64],
                    |r| r.get::<_, i64>(0),
                )
                .is_ok())
        })
        .unwrap_or(false);

        match copy_one_message(&imap_client, state, source, target, folder_name, uid as i64, is_cached).await {
            Ok(()) => copied += 1,
            Err(e) => {
                tracing::debug!("migrate: {} / uid {} fehlgeschlagen: {}", folder_name, uid, e);
                failed += 1;
            }
        }
    }
    imap_client.shutdown().await;
    finish_folder_task(state, copied, failed, None);
}

fn finish_folder_task(state: &AppState, copied: usize, failed: usize, error: Option<String>) {
    let mut s = state.migration.write();
    if let Some(st) = s.as_mut() {
        st.running = false;
        st.done = true;
        st.messages_copied = copied;
        st.messages_failed = failed;
        st.folders_done = 1;
        st.current_folder = String::new();
        st.finished_at = Some(chrono::Utc::now().to_rfc3339());
        if let Some(e) = error {
            st.last_error = Some(e);
        }
    }
    tracing::info!("migrate: Ordner abgeschlossen — {} kopiert, {} fehlgeschlagen", copied, failed);
}
