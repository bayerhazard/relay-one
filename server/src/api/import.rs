//! MBox import (Apple Mail / Thunderbird export format).
//!
//! `POST /api/v1/import/mbox` — takes raw mbox content, splits it into
//! individual RFC822 messages at "From " separator lines, and imports each
//! message into a LOCAL folder of the given account:
//!   1. raw RFC822 bytes are written to the EML archive (source of truth),
//!   2. a `messages` row is inserted (folder_id of the local folder,
//!      uid = max_uid+1 per folder, message_id-based dedup),
//!   3. duplicate message-ids are skipped.
//!
//! `POST /api/v1/import/mbox-dir` — scans a directory inside the app data
//! root (default `<data_root>/Mails`) for `*.mbox` files; each file becomes
//! a LOCAL folder named after the file (e.g. `Auto.mbox` -> folder `Auto`).

use axum::extract::State;
use axum::Json;
use serde::Deserialize;

use crate::AppState;
use crate::cache;
use crate::db::with_db;
use mail_parser::MimeHeaders;

#[derive(Deserialize)]
pub struct MboxImportRequest {
    pub account_id: u32,
    /// Name of the target LOCAL folder (created if missing).
    pub folder: String,
    /// Raw mbox file content (UTF-8 / bytes).
    pub mbox: String,
    /// Optional: source description (e.g. "Apple Mail Export").
    #[serde(default)]
    pub note: String,
}

pub async fn import_mbox(
    State(state): State<AppState>,
    Json(req): Json<MboxImportRequest>,
) -> crate::api::ApiResult<serde_json::Value> {
    if req.mbox.trim().is_empty() {
        return Err(crate::api::ApiError("Leere mbox-Datei".into()));
    }
    let messages = split_mbox(&req.mbox);
    if messages.is_empty() {
        return Err(crate::api::ApiError(
            "Keine Nachrichten in der mbox-Datei gefunden (Format?).".into(),
        ));
    }
    let result = import_mbox_content(&state, req.account_id, &req.folder, &messages)?;
    tracing::info!(
        "mbox-Import (account {}, Ordner '{}'): {} importiert, {} Duplikate, {} Fehler",
        req.account_id, req.folder, result.imported, result.duplicates, result.errors
    );
    Ok(Json(serde_json::json!({
        "ok": true,
        "imported": result.imported,
        "duplicates": result.duplicates,
        "errors": result.errors,
        "total_messages": messages.len(),
        "folder": req.folder,
        "note": req.note,
    })))
}

#[derive(Deserialize)]
pub struct MboxDirRequest {
    pub account_id: u32,
    /// Directory below the data root containing `*.mbox` files.
    /// Default: `Mails` (= `<data_root>/Mails`, i.e. /data/Relay/Mails).
    #[serde(default = "default_dir")]
    pub dir: String,
}

fn default_dir() -> String {
    "Mails".to_string()
}

/// `POST /api/v1/import/attachments-backfill` — for messages that already
/// have `has_attachments=1` but no `message_attachments` rows (e.g. mails
/// imported before attachment metadata was stored), extract the attachments
/// from the local EML archive and persist metadata + base64 content.
pub async fn attachments_backfill(
    State(state): State<AppState>,
) -> crate::api::ApiResult<serde_json::Value> {
    // Find messages with has_attachments but no attachment rows.
    let candidates: Vec<(i64, i64, Option<String>)> = with_db(&state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT m.id, m.account_id, m.raw_path
                 FROM messages m
                 WHERE m.has_attachments = 1
                   AND NOT EXISTS (SELECT 1 FROM message_attachments a WHERE a.message_id = m.id)
                 ORDER BY m.id
                 LIMIT 2000",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<String>>(2)?)))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    })?;

    let mut filled = 0usize;
    let mut no_eml = 0usize;
    let mut parse_failed = 0usize;

    for (message_id, _account_id, raw_rel) in candidates {
        let Some(rel) = raw_rel else {
            no_eml += 1;
            continue;
        };
        let abs = state.data_root.join(&rel);
        let Ok(bytes) = std::fs::read(&abs) else {
            no_eml += 1;
            continue;
        };
        let attachments = crate::imap::client::parse_message_attachments(&bytes);
        if attachments.is_empty() {
            parse_failed += 1;
            continue;
        }
        let ok = with_db(&state, |conn| {
            // Reconcile metadata (with stable part_index) instead of a raw
            // INSERT — the UNIQUE(message_id, part_index) constraint requires
            // a per-part index, and reconcile removes stale rows.
            let metas: Vec<crate::imap::types::AttachmentMeta> = attachments
                .iter()
                .map(|a| crate::imap::types::AttachmentMeta {
                    filename: a.filename.clone(),
                    content_type: a.content_type.clone(),
                    size: a.size,
                })
                .collect();
            crate::cache::attachments::reconcile_attachments(conn, message_id, &metas)
                .map_err(|e| e.to_string())?;
            // Persist content deduplicated to disk (base64 from the parsed raw).
            for att in &attachments {
                let att_row_id: Option<i64> = conn
                    .query_row(
                        "SELECT a.id FROM message_attachments a
                         JOIN messages m ON m.id = a.message_id
                         WHERE a.message_id = ?1 AND a.filename = ?2
                         ORDER BY a.part_index ASC LIMIT 1",
                        rusqlite::params![message_id, att.filename],
                        |r| r.get(0),
                    )
                    .ok();
                if let Some(aid) = att_row_id {
                    let _ = crate::cache::attachments::cache_content_dedup(conn, aid, &att.content, &state.data_root);
                }
            }
            Ok::<(), String>(())
        });
        match ok {
            Ok(()) => filled += 1,
            Err(_) => parse_failed += 1,
        }
    }

    tracing::info!(
        "Attachments-Backfill: {} Mails befüllt, {} ohne EML, {} ohne parsebare Anhänge",
        filled, no_eml, parse_failed
    );
    Ok(Json(serde_json::json!({
        "ok": true,
        "filled": filled,
        "no_eml": no_eml,
        "parse_failed": parse_failed,
    })))
}

/// `POST /api/v1/import/mbox-dir` — import every `*.mbox` file in
/// `<data_root>/<dir>` as a local folder named after the file.
pub async fn import_mbox_dir(
    State(state): State<AppState>,
    Json(req): Json<MboxDirRequest>,
) -> crate::api::ApiResult<serde_json::Value> {
    let dir = state.data_root.join(&req.dir);
    if !dir.exists() {
        return Err(crate::api::ApiError(format!(
            "Verzeichnis {} existiert nicht",
            dir.display()
        )));
    }

    // Apple Mail exports each mailbox as a DIRECTORY named `X.mbox`
    // containing the actual mbox file as `X.mbox/mbox` (+ table_of_contents).
    // Nested mailboxes become nested directories (e.g. `Beta Tests/Ecovacs Goat.mbox`).
    // We walk the tree and collect (display-folder-path, absolute-mbox-file).
    //
    // If `dir` itself points at a single Apple mailbox directory (it contains
    // a file literally named `mbox`), import just that one mailbox as a folder
    // named after the directory (e.g. "Mails/Gesendet.mbox" -> "Gesendet").
    let mut boxes: Vec<(String, std::path::PathBuf)> = Vec::new();
    if dir.join("mbox").is_file() {
        let raw_name = dir
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let folder = raw_name.trim_end_matches(".mbox").to_string();
        boxes.push((folder, dir.join("mbox")));
    } else {
        collect_apple_mailboxes(&dir, "", &mut boxes)?;
    }
    boxes.sort_by(|a, b| a.0.cmp(&b.0));

    if boxes.is_empty() {
        return Err(crate::api::ApiError(format!(
            "Keine .mbox-Maillboxen in {} gefunden",
            dir.display()
        )));
    }

    let mut summary = Vec::new();
    let mut total_imported = 0usize;
    let mut total_duplicates = 0usize;
    let mut total_errors = 0usize;

    for (folder, abs) in &boxes {
        // Folder name: relative path with '/' — Relay stores nested folders
        // with the IMAP delimiter ('.'), so "Beta Tests/Ecovacs Goat"
        // becomes the local folder "Beta Tests.Ecovacs Goat".
        let folder_name = folder.replace('/', ".");
        let content = std::fs::read(abs)
            .map_err(|e| crate::api::ApiError(format!("{} lesen fehlgeschlagen: {}", abs.display(), e)))?;
        let content = String::from_utf8_lossy(&content).to_string();
        let messages = split_mbox(&content);

        let result = if messages.is_empty() {
            Ok(ImportSummary {
                imported: 0,
                duplicates: 0,
                errors: 1,
            })
        } else {
            import_mbox_content(&state, req.account_id, &folder_name, &messages)
        }?;

        total_imported += result.imported;
        total_duplicates += result.duplicates;
        total_errors += result.errors;
        summary.push(serde_json::json!({
            "file": abs.display().to_string(),
            "folder": folder_name,
            "messages": messages.len(),
            "imported": result.imported,
            "duplicates": result.duplicates,
            "errors": result.errors,
        }));
        tracing::info!(
            "mbox-dir: {} -> Ordner '{}': {} importiert, {} Duplikate, {} Fehler ({} Mails)",
            abs.display(), folder_name, result.imported, result.duplicates, result.errors, messages.len()
        );
    }

    Ok(Json(serde_json::json!({
        "ok": true,
        "dir": dir.display().to_string(),
        "files": boxes.len(),
        "imported": total_imported,
        "duplicates": total_duplicates,
        "errors": total_errors,
        "summary": summary,
    })))
}

struct ImportSummary {
    imported: usize,
    duplicates: usize,
    errors: usize,
}

/// Core import: store each raw RFC822 message into the EML archive and the
/// messages table of the given account + local folder.
fn import_mbox_content(
    state: &AppState,
    account_id: u32,
    folder: &str,
    messages: &[String],
) -> Result<ImportSummary, crate::api::ApiError> {
    // Ensure the target folder exists as a LOCAL folder.
    with_db(state, |conn| {
        cache::messages::create_local_folder(conn, account_id as i64, folder)
            .map_err(|e| e.to_string())
    })?;

    let mut imported = 0usize;
    let mut duplicates = 0usize;
    let mut errors = 0usize;

    for raw in messages {
        // Parse the message to extract metadata.
        let Some(parsed) = mail_parser::MessageParser::default().parse(raw.as_bytes()) else {
            errors += 1;
            continue;
        };

        let subject = parsed.subject().map(|s| s.to_string()).unwrap_or_default();
        let from_addr = parsed.from().and_then(|a| a.first()).and_then(|a| a.address()).map(|s| s.to_string()).unwrap_or_default();
        let to_addr = parsed.to().and_then(|a| a.first()).and_then(|a| a.address()).map(|s| s.to_string()).unwrap_or_default();
        let date = parsed.date().map(|d| d.to_string()).unwrap_or_default();
        let message_id = parsed.message_id().map(|s| s.to_string()).unwrap_or_default();
        let body_text = parsed
            .text_part(0)
            .map(|p| String::from_utf8_lossy(p.contents()).to_string())
            .unwrap_or_default();
        // Only keep body_html when it really carries markup. html_part(0) can
        // return a text/html part whose content is bare text — storing that
        // makes the UI render raw text through the HTML branch (line breaks
        // collapse → the mail shows as one flow paragraph).
        let body_html = parsed
            .html_part(0)
            .map(|p| String::from_utf8_lossy(p.contents()).to_string())
            .filter(|s| crate::imap::client::contains_html_markup(s))
            .unwrap_or_default();
        let has_attachments = parsed.attachments().next().is_some();

        // Dedup by message_id within this account.
        let dup = with_db(state, |conn| {
            if message_id.is_empty() {
                return Ok(false);
            }
            Ok(conn
                .query_row(
                    "SELECT 1 FROM messages WHERE account_id = ?1 AND message_id = ?2 LIMIT 1",
                    rusqlite::params![account_id as i64, message_id],
                    |_| Ok(()),
                )
                .is_ok())
        })?;
        if dup {
            duplicates += 1;
            continue;
        }

        // Assign uid = max_uid_in_folder + 1 (atomic-ish inside the DB lock).
        let (uid, folder_id) = with_db(state, |conn| {
            let max_uid = cache::messages::get_max_uid_for_folder(conn, account_id as i64, folder)
                .unwrap_or(0);
            let folder_id: i64 = conn
                .query_row(
                    "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
                    rusqlite::params![account_id as i64, folder],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            Ok::<_, String>((max_uid + 1, folder_id))
        })?;

        // Write EML to the archive (byte-identical raw mbox message).
        let target_path = crate::cache::archive::write_eml(
            &state.data_root,
            account_id as i64,
            uid as u32,
            Some(date.as_str()),
            Some(message_id.as_str()),
            raw.as_bytes(),
        )
        .map_err(|e| crate::api::ApiError(e.to_string()))?;
        let rel = target_path
            .strip_prefix(&state.data_root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| target_path.to_string_lossy().to_string());
        let raw_sha = crate::cache::archive::sha256_hex(raw.as_bytes());

        with_db(state, |conn| {
            conn.execute(
                "INSERT OR IGNORE INTO messages
                 (account_id, folder_id, uid, message_id, subject, from_addr, to_addr, date,
                  body_text, body_html, flags, ai_summary, ai_priority, ai_fraud_score,
                  is_read, is_flagged, has_attachments, synced, raw_path, raw_sha256)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                         ?9, ?10, ?11, NULL, NULL, NULL, 0, 0, ?12, 0, ?13, ?14)",
                rusqlite::params![
                    account_id as i64, folder_id, uid,
                    message_id, subject, from_addr, to_addr, date,
                    body_text, body_html, "[]", has_attachments as i32,
                    rel, raw_sha,
                ],
            )
            .map_err(|e| e.to_string())?;

            // Attachment metadata: without these rows the UI shows the
            // paperclip (has_attachments flag) but the preview pane finds no
            // attachments and cannot open/save them. Store the metadata +
            // base64 content directly (extracted from the parsed message), so
            // attachments work for imported mails immediately.
            let message_row_id: i64 = conn
                .query_row(
                    "SELECT id FROM messages WHERE account_id = ?1 AND folder_id = ?2 AND uid = ?3",
                    rusqlite::params![account_id as i64, folder_id, uid],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let mut metas = Vec::new();
            let mut contents: Vec<(String, String, i64)> = Vec::new(); // (filename, base64, size)
            for part in parsed.attachments() {
                use base64::Engine as _;
                let filename = part.attachment_name().unwrap_or("anhang").to_string();
                let content_type = part
                    .content_type()
                    .map(|c| {
                        let subtype = c.c_subtype.as_ref().map(|s| s.as_ref()).unwrap_or("octet-stream");
                        format!("{}/{}", c.c_type, subtype)
                    })
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                let contents_bytes = part.contents();
                let size = contents_bytes.len() as i64;
                metas.push(crate::imap::types::AttachmentMeta {
                    filename: filename.clone(),
                    content_type,
                    size: size as usize,
                });
                contents.push((
                    filename,
                    base64::engine::general_purpose::STANDARD.encode(contents_bytes),
                    size,
                ));
            }
            // Reconcile metadata with stable part_index (the UNIQUE constraint
            // forbids raw inserts without a part index), then persist content
            // deduplicated to disk.
            crate::cache::attachments::reconcile_attachments(conn, message_row_id, &metas)
                .map_err(|e| e.to_string())?;
            for (filename, b64, _size) in &contents {
                let aid: Option<i64> = conn
                    .query_row(
                        "SELECT id FROM message_attachments
                         WHERE message_id = ?1 AND filename = ?2
                         ORDER BY part_index ASC LIMIT 1",
                        rusqlite::params![message_row_id, filename],
                        |r| r.get(0),
                    )
                    .ok();
                if let Some(aid) = aid {
                    let _ = crate::cache::attachments::cache_content_dedup(conn, aid, b64, &state.data_root);
                }
            }
            Ok::<(), String>(())
        })?;
        imported += 1;
    }

    Ok(ImportSummary {
        imported,
        duplicates,
        errors,
    })
}

/// Walk the Apple-Mail export tree and collect (relative-folder, mbox-file).
/// A mailbox directory is `X.mbox` containing a file literally named `mbox`.
/// Regular directories (e.g. `Beta Tests`) are traversed recursively.
fn collect_apple_mailboxes(
    dir: &std::path::Path,
    prefix: &str,
    out: &mut Vec<(String, std::path::PathBuf)>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("{} lesen fehlgeschlagen: {}", dir.display(), e))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        if path.is_dir() {
            if name.to_lowercase().ends_with(".mbox") {
                // Mailbox directory: expect a file named `mbox` inside.
                let mbox_file = path.join("mbox");
                if mbox_file.is_file() {
                    let folder = if prefix.is_empty() {
                        name.trim_end_matches(".mbox").to_string()
                    } else {
                        format!("{}/{}", prefix, name.trim_end_matches(".mbox"))
                    };
                    out.push((folder, mbox_file));
                }
            } else {
                // Nested folder (e.g. "Beta Tests").
                let nested = if prefix.is_empty() { name } else { format!("{}/{}", prefix, name) };
                collect_apple_mailboxes(&path, &nested, out)?;
            }
        }
    }
    Ok(())
}

/// Split a mbox stream into individual raw RFC822 messages.
/// A message starts at a line beginning with "From " (mbox separator).
fn split_mbox(content: &str) -> Vec<String> {
    let mut messages = Vec::new();
    let mut current = String::new();
    let mut in_message = false;

    for line in content.lines() {
        if line.starts_with("From ") {
            // Start of a new message — flush the previous one.
            if in_message {
                messages.push(std::mem::take(&mut current));
            }
            in_message = true;
            // Skip the separator line itself.
            continue;
        }
        if in_message {
            // Un-escape mbox quoting: a line starting with ">From " (or
            // ">>From " etc.) represents an original "From " line.
            if line.starts_with('>') && line[1..].starts_with("From ") {
                current.push_str(&line[1..]);
            } else {
                current.push_str(line);
            }
            current.push('\n');
        }
    }
    if in_message {
        messages.push(current);
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_basic_mbox() {
        let mbox = "From marc@example.com Mon Jan  1 00:00:00 2024\n\
                    Subject: Erste\n\
                    \n\
                    Hallo Welt\n\
                    From marc@example.com Tue Jan  2 00:00:00 2024\n\
                    Subject: Zweite\n\
                    \n\
                    Test 2\n";
        let msgs = split_mbox(mbox);
        assert_eq!(msgs.len(), 2);
        assert!(msgs[0].contains("Subject: Erste"));
        assert!(msgs[1].contains("Subject: Zweite"));
    }

    #[test]
    fn split_unescapes_from() {
        let mbox = "From marc@example.com Mon Jan  1 00:00:00 2024\n\
                    Subject: Escaped\n\
                    \n\
                    >From ist ein Zitat\n";
        let msgs = split_mbox(mbox);
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("\nFrom ist ein Zitat"));
    }

    #[test]
    fn split_empty_and_no_separator() {
        assert!(split_mbox("").is_empty());
        assert!(split_mbox("nur ein Text ohne From-Linie").is_empty());
    }

    #[test]
    fn split_handles_crlf() {
        let mbox = "From a@b Mon Jan  1 00:00:00 2024\r\nSubject: CRLF\r\n\r\nBody\r\n";
        let msgs = split_mbox(mbox);
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("Subject: CRLF"));
    }
}

#[cfg(test)]
mod apple_tests {
    use super::*;
    use std::fs;

    fn setup_tree() -> tempfile::TempDir {
        let td = tempfile::tempdir().unwrap();
        let root = td.path();
        // Auto.mbox/mbox (top-level mailbox)
        fs::create_dir_all(root.join("Auto.mbox")).unwrap();
        fs::write(root.join("Auto.mbox/mbox"), "From a@b Mon Jan  1 00:00:00 2024\nSubject: Auto\n\nBody\n").unwrap();
        // Beta Tests/Ecovacs Goat.mbox/mbox (nested)
        fs::create_dir_all(root.join("Beta Tests/Ecovacs Goat.mbox")).unwrap();
        fs::write(root.join("Beta Tests/Ecovacs Goat.mbox/mbox"), "From a@b Mon Jan  1 00:00:00 2024\nSubject: Goat\n\nBody\n").unwrap();
        // A plain file that should be ignored
        fs::write(root.join("notes.txt"), "ignore me").unwrap();
        td
    }

    #[test]
    fn collects_apple_mailboxes() {
        let td = setup_tree();
        let mut boxes = Vec::new();
        collect_apple_mailboxes(td.path(), "", &mut boxes).unwrap();
        let mut names: Vec<String> = boxes.iter().map(|(f, _)| f.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["Auto", "Beta Tests/Ecovacs Goat"]);
        assert_eq!(boxes[0].1.file_name().unwrap().to_str().unwrap(), "mbox");
    }

    #[test]
    fn folder_slash_becomes_dot() {
        let td = setup_tree();
        let mut boxes = Vec::new();
        collect_apple_mailboxes(td.path(), "", &mut boxes).unwrap();
        let folder = boxes.iter().find(|(f, _)| f.contains('/')).unwrap().0.clone();
        assert_eq!(folder.replace('/', "."), "Beta Tests.Ecovacs Goat");
    }
}
