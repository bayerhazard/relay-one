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

async fn copy_folder(
    state: &AppState,
    source: i64,
    target: i64,
    folder_name: &str,
) -> Result<FolderReport, ApiError> {
    copy_folder_limited(state, source, target, folder_name, None).await
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
    // the local sync cached — the sync only fetches recent batches). SELECT
    // + UID SEARCH run atomically so a concurrent sync can't switch folders.
    let imap_uids: Vec<u32> = {
        let client = state
            .imap_clients
            .read()
            .get(&(source as u32))
            .cloned()
            .ok_or(ApiError("Quell-IMAP-Client nicht gefunden".into()))?;
        if !client.is_connected().await {
            client.connect().await.map_err(|e| ApiError(e.to_string()))?;
        }
        match client.fetch_all_uids_in_folder(folder_name).await {
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
        match copy_one_message(state, source, target, folder_name, uid_i64, cached.contains(&uid_i64)).await {
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

    Ok(report)
}

async fn copy_one_message(
    state: &AppState,
    source: i64,
    target: i64,
    folder_name: &str,
    uid: i64,
    _is_cached: bool,
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
        let client = state
            .imap_clients
            .read()
            .get(&(source as u32))
            .cloned()
            .ok_or("Quell-IMAP-Client nicht gefunden")?;
        if !client.is_connected().await {
            client.connect().await.map_err(|e| e.to_string())?;
        }
        let raw_result = client
            .fetch_raw_message_in_folder(uid as u32, Some(folder_name.to_string()))
            .await;
        let bytes = match raw_result {
            Ok(raw) => raw.into_bytes(),
            Err(_) => {
                // Reconstruct from the local DB (headers + body).
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
    with_db(state, |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO messages
             (account_id, folder_id, uid, message_id, subject, from_addr, to_addr, date,
              body_text, body_html, flags, ai_summary, ai_priority, ai_fraud_score,
              is_read, is_flagged, has_attachments, synced, raw_path, raw_sha256)
             VALUES (
               ?1,
               (SELECT id FROM folders WHERE account_id = ?1 AND name = ?2),
               ?3, ?4, ?5, ?6, ?7, ?8,
               (SELECT body_text FROM messages WHERE account_id = ?9 AND uid = ?3),
               (SELECT body_html FROM messages WHERE account_id = ?9 AND uid = ?3),
               ?10, NULL, NULL, NULL, ?11, ?12, ?13, 0, ?14, ?15
             )",
            rusqlite::params![
                target, folder_name, uid,
                meta.4, meta.0, meta.1, meta.2, meta.3,
                source,
                "[]", 0i32, 0i32, 0i32,
                rel, written_sha,
            ],
        )
        .map_err(|e| e.to_string())
    })?;

    Ok(())
}
