//! Message endpoints: folders, fetch, search, body, read-state, delete, move.

use axum::extract::{Query, State};
use axum::Json;
use base64::Engine as _;
use serde::Deserialize;

use crate::cache;
use crate::cache::messages::MessageRecord;
use sha2::Digest as _;
use crate::db::{get_db, with_db};
use crate::imap::client;
use crate::AppState;

use super::{ApiError, ApiResult};

// ─── Response helpers ─────────────────────────────────────────

/// Decode body text: only allocates a String if QP-decoding is actually needed.
fn decode_body_text(b: &str) -> String {
    if client::has_qp_pattern(b) {
        client::decode_transfer_encoding(b)
    } else {
        b.to_string()
    }
}

/// Serialize a MessageRecord to JSON.
fn message_to_json(m: &MessageRecord) -> serde_json::Value {
    let body_preview = m.body_text.as_deref().map(|b| {
        decode_body_text(b).chars().take(200).collect::<String>()
    });
    serde_json::json!({
        "id": m.id,
        "uid": m.uid,
        "message_id": m.message_id,
        "subject": m.subject.as_deref().map(client::decode_rfc2047),
        "from": m.from_addr,
        "to": m.to_addr,
        "date": m.date,
        "body_preview": body_preview,
        "body_text": m.body_text.as_deref().map(decode_body_text),
        "body_html": m.body_html.clone(),
        "flags": m.flags,
        "ai_summary": m.ai_summary,
        "ai_priority": m.ai_priority,
        "ai_fraud_score": m.ai_fraud_score,
        "is_read": m.is_read,
        "has_attachments": m.has_attachments,
    })
}

// ─── Query types ───────────────────────────────────────────────

#[derive(Deserialize)]
pub struct FetchMessagesQuery {
    pub account_id: u32,
    pub folder: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// Query for single-message endpoints (uid + account in the query string).
#[derive(Deserialize)]
pub struct MessageUidQuery {
    pub account_id: u32,
    pub uid: u32,
}

/// Query for attachment content (uid + att_id + account in the query string).
#[derive(Deserialize)]
pub struct AttachmentContentQuery {
    pub account_id: u32,
    pub uid: u32,
    pub att_id: u32,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub account_id: u32,
    pub query: String,
    pub limit: Option<u32>,
}

// ─── Endpoints ─────────────────────────────────────────────────

/// Query for folder listing — only account_id is required (no `query` field).
#[derive(Deserialize)]
pub struct FolderQuery {
    pub account_id: u32,
}

/// `GET /api/v1/folders?account_id=…`
/// Returns IMAP folders (live) + local-only folders (from cache).
pub async fn list_imap_folders(
    State(state): State<AppState>,
    Query(q): Query<FolderQuery>,
) -> ApiResult<Vec<serde_json::Value>> {
    // IMAP folders (live) — if the client is disconnected or the fetch fails,
    // fall back to the locally cached folder list instead of erroring. This
    // keeps the sidebar populated even while the account shows "Getrennt".
    // IMAP folders (live). The client is cloned out of the shared map and the
    // fetch runs in a helper so no non-Send TLS state crosses await points
    // inside this handler.
    let json_imap = fetch_imap_folder_list(&state, q.account_id).await;
    let mut json: Vec<serde_json::Value> = json_imap;

    // Local folders (from cache) — includes local-only folders AND every
    // folder that ever appeared in a sync, so the tree stays usable offline.
    let locals = with_db(&state, |conn| {
        cache::messages::list_all_folders(conn, q.account_id as i64).map_err(|e| e.to_string())
    })
    .unwrap_or_default();
    for (name, local) in locals {
        if !json.iter().any(|f| f["name"] == name) {
            json.push(serde_json::json!({
                "name": name, "raw_name": "", "delimiter": "", "tag": "", "attributes": [],
                "local_only": local,
            }));
        }
    }
    Ok(Json(json))
}

/// Fetch the live IMAP folder list for an account. Runs in a helper so the
/// client (with its TLS session) is fully owned here; never errors — on any
/// failure an empty list is returned and the cached folders still show.
async fn fetch_imap_folder_list(state: &AppState, account_id: u32) -> Vec<serde_json::Value> {
    let Some(client) = state.imap_clients.read().get(&account_id).cloned() else {
        return Vec::new();
    };
    if !client.is_connected().await {
        let _ = client.connect().await;
    }
    if !client.is_connected().await {
        return Vec::new();
    }

    // Extended mode (archive): only INBOX + the provider SPAM folder are
    // shown — the user works exclusively with local folders. Mirror mode
    // shows the full IMAP tree.
    let extended = with_db(state, |conn| {
        Ok(conn
            .query_row(
                "SELECT sync_mode FROM accounts WHERE id = ?1",
                rusqlite::params![account_id as i64],
                |row| row.get::<_, String>(0),
            )
            .map(|m| m == "archive")
            .unwrap_or(false))
    })
    .unwrap_or(false);

    match client.list_folders_detailed().await {
        Ok(folders) => folders
            .iter()
            .filter(|f| {
                if !extended {
                    return true;
                }
                // Extended: INBOX + spam-like folders only.
                let lower = f.name.to_lowercase();
                lower == "inbox"
                    || ["spam", "junk", "spamverdacht", "junk e-mail", "junkemail", "bulk"]
                        .iter()
                        .any(|s| lower.contains(s))
            })
            .map(|f| {
                serde_json::json!({
                    "name": f.name, "raw_name": f.raw_name, "delimiter": f.delimiter, "tag": f.tag, "attributes": f.attributes,
                    "local_only": false,
                })
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// `POST /api/v1/folders` — create a local-only folder.
#[derive(Deserialize)]
pub struct CreateFolderRequest {
    pub account_id: u32,
    pub name: String,
}

pub async fn create_folder(
    State(state): State<AppState>,
    Json(req): Json<CreateFolderRequest>,
) -> ApiResult<serde_json::Value> {
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError("Ordnername darf nicht leer sein".into()));
    }
    with_db(&state, |conn| {
        cache::messages::create_local_folder(conn, req.account_id as i64, &name).map_err(|e| e.to_string())
    })?;
    tracing::info!("Lokal-only Ordner angelegt: {} (account {})", name, req.account_id);
    Ok(Json(serde_json::json!({ "ok": true, "name": name, "local_only": true })))
}

/// `POST /api/v1/folders/rename` — rename local-only folders locally;
/// IMAP folders are renamed on the server (existing behaviour).

/// `GET /api/v1/messages?account_id=&folder=&limit=&offset=`
pub async fn fetch_messages(
    State(state): State<AppState>,
    Query(q): Query<FetchMessagesQuery>,
) -> ApiResult<Vec<serde_json::Value>> {
    let messages = with_db(&state, |conn| {
        let folder_name = q.folder.as_deref().unwrap_or("INBOX");
        cache::messages::fetch_inbox(
            conn,
            q.account_id as i64,
            q.limit.map(|v| v as i64),
            q.offset.map(|v| v as i64),
            folder_name,
        )
        .map_err(|e| e.to_string())
    })?;
    Ok(Json(messages.iter().map(message_to_json).collect()))
}

/// `GET /api/v1/messages/search?account_id=&query=&limit=`
pub async fn search_messages(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Vec<serde_json::Value>> {
    let messages = with_db(&state, |conn| {
        cache::messages::search_messages(
            conn,
            q.account_id as i64,
            &q.query,
            q.limit.map(|v| v as i64).unwrap_or(100),
        )
        .map_err(|e| e.to_string())
    })?;
    Ok(Json(messages.iter().map(message_to_json).collect()))
}

/// `GET /api/v1/messages/{uid}/body?account_id=…`
pub async fn fetch_message_body(
    State(state): State<AppState>,
    Query(q): Query<MessageUidQuery>,
) -> ApiResult<serde_json::Value> {
    let uid = q.uid;
    // Fast path: cached body with folder info.
    let cached_with_folder = with_db(&state, |conn| {
        cache::messages::fetch_message_body_with_folder(conn, q.account_id as i64, uid as i64)
            .map_err(|e| e.to_string())
    })?;

    if let Some((ref msg, _)) = cached_with_folder {
        if msg.body_text.is_some() {
            return Ok(Json(serde_json::json!({
                "id": msg.id,
                "uid": msg.uid,
                "subject": msg.subject.as_deref().map(client::decode_rfc2047),
                "from": msg.from_addr,
                "to": msg.to_addr,
                "date": msg.date,
                "body_text": msg.body_text.as_deref().map(decode_body_text),
                "body_html": msg.body_html,
                "flags": msg.flags,
                "ai_summary": msg.ai_summary,
                "ai_priority": msg.ai_priority,
                "ai_fraud_score": msg.ai_fraud_score,
                "is_read": msg.is_read,
            })));
        }
    }

    // IMAP fallback
    let client = state
        .imap_clients
        .read()
        .get(&q.account_id)
        .cloned()
        .ok_or(ApiError("IMAP-Client nicht gefunden".into()))?;
    if !client.is_connected().await {
        client.connect().await.map_err(|e| ApiError(e.to_string()))?;
    }

    let folder_name = cached_with_folder.as_ref().map(|(_, f)| f.clone());

    match client.fetch_body_from_folder(uid, folder_name).await {
        Ok((body_text, body_html)) => {
            let _: Result<(), String> = with_db(&state, |conn| {
                cache::messages::update_body(
                    conn, q.account_id as i64, uid as i64,
                    &body_text, body_html.as_deref(),
                )
                .map_err(|e| e.to_string())
            });
            Ok(Json(serde_json::json!({
                "uid": uid as i64,
                "body_text": decode_body_text(&body_text),
                "body_html": body_html,
                "is_read": false,
            })))
        }
        Err(e) => {
            if let Some((msg, _)) = cached_with_folder {
                Ok(Json(message_to_json(&msg)))
            } else {
                Err(ApiError(format!(
                    "Nachricht nicht gefunden und IMAP-Fetch fehlgeschlagen: {}",
                    e
                )))
            }
        }
    }
}

#[derive(Deserialize)]
pub struct MoveMessageRequest {
    pub account_id: u32,
    pub uid: u32,
    pub source_folder: String,
    pub target_folder: String,
    #[serde(default)]
    pub raw_source_folder: String,
    #[serde(default)]
    pub raw_target_folder: String,
}

/// `POST /api/v1/messages/{uid}/move`
pub async fn move_message(
    State(state): State<AppState>,
    Json(req): Json<MoveMessageRequest>,
) -> ApiResult<()> {
    let uid = req.uid;
    let account_id_i64 = req.account_id as i64;
    let uid_i64 = uid as i64;

    // Local-only target: the mail is archived locally (EML), the provider copy
    // is deleted ONLY after verify (F1): EML exists + hash matches. If the
    // guarantee cannot be proven, fall back to a soft MOVE into Provider-Trash.
    let target_is_local = with_db(&state, |conn| {
        cache::messages::is_local_only_folder(conn, account_id_i64, &req.target_folder)
            .map_err(|e| e.to_string())
    })
    .unwrap_or(false);

    if target_is_local {
        return move_to_local_folder(&state, &req, uid, account_id_i64, uid_i64).await;
    }

    // Step 1: Get IMAP client and ensure connected
    let client = {
        let guard = state.imap_clients.read();
        guard
            .get(&req.account_id)
            .cloned()
            .ok_or(ApiError("IMAP-Client nicht gefunden".into()))?
    };

    if !client.is_connected().await {
        client.connect().await.map_err(|e| ApiError(e.to_string()))?;
    }

    // Step 2: Update cache to target folder (use decoded name for cache)
    with_db(&state, |conn| {
        cache::messages::update_folder(conn, account_id_i64, uid_i64, &req.target_folder)
            .map_err(|e| e.to_string())
    })?;

    // Step 3: Attempt IMAP move (use raw IMAP-UTF7 names for server operations)
    let imap_source = if req.raw_source_folder.is_empty() {
        &req.source_folder
    } else {
        &req.raw_source_folder
    };
    let imap_target = if req.raw_target_folder.is_empty() {
        &req.target_folder
    } else {
        &req.raw_target_folder
    };
    if let Err(e) = client.move_message(uid, imap_source, imap_target).await {
        tracing::error!(
            "move_message: account={}, uid={} — IMAP move failed: {}. Rolling back cache to folder '{}'.",
            req.account_id, uid, e, req.source_folder
        );
        let _ = with_db(&state, |conn| {
            cache::messages::update_folder(conn, account_id_i64, uid_i64, &req.source_folder)
                .map_err(|e| e.to_string())
        });
        return Err(ApiError(format!(
            "Verschieben von '{}' nach '{}' fehlgeschlagen: {}",
            req.source_folder, req.target_folder, e
        )));
    }

    Ok(Json(()))
}

/// Move a message into a local-only folder. The local index moves immediately;
/// the provider copy is removed only when the archive guarantee holds.
async fn move_to_local_folder(
    state: &AppState,
    req: &MoveMessageRequest,
    uid: u32,
    account_id_i64: i64,
    uid_i64: i64,
) -> ApiResult<()> {
    // 1. Locally move the index row (never blocks on IMAP).
    with_db(state, |conn| {
        cache::messages::update_folder(conn, account_id_i64, uid_i64, &req.target_folder)
            .map_err(|e| e.to_string())
    })?;

    // 2. Verify archive guarantee: raw EML exists + hash matches.
    let verified = with_db(state, |conn| {
        let raw_path: Option<String> = conn
            .query_row(
                "SELECT raw_path FROM messages WHERE account_id = ?1 AND uid = ?2",
                rusqlite::params![account_id_i64, uid_i64],
                |row| row.get(0),
            )
            .ok();
        let raw_sha: Option<String> = conn
            .query_row(
                "SELECT raw_sha256 FROM messages WHERE account_id = ?1 AND uid = ?2",
                rusqlite::params![account_id_i64, uid_i64],
                |row| row.get(0),
            )
            .ok();
        Ok::<_, String>((raw_path, raw_sha))
    })
    .unwrap_or((None, None));

    let eml_ok = verified
        .0
        .and_then(|rel| {
            let abs = state.data_root.join(&rel);
            let exists = crate::cache::archive::verify_eml(&abs, verified.1.as_deref(), None);
            Some(exists)
        })
        .unwrap_or(false);

    // 3. Provider copy: hard delete (EXPUNGE) only with guarantee, else soft.
    let client = {
        let guard = state.imap_clients.read();
        guard
            .get(&req.account_id)
            .cloned()
            .ok_or(ApiError("IMAP-Client nicht gefunden".into()))?
    };
    if client.is_connected().await {
        let imap_source = if req.raw_source_folder.is_empty() {
            &req.source_folder
        } else {
            &req.raw_source_folder
        };
        let result = if eml_ok {
            client.hard_delete_message(uid, imap_source).await
        } else {
            tracing::warn!(
                "move_to_local: Verify-Garantie fehlt für uid {} (account {}) — weiches Löschen (Provider-Trash)",
                uid, req.account_id
            );
            client.move_message(uid, imap_source, "Trash").await
        };
        if let Err(e) = result {
            tracing::warn!(
                "move_to_local: Provider-Kopie für uid {} konnte nicht entfernt werden: {}",
                uid, e
            );
        }
    }

    tracing::info!(
        "move_to_local: uid {} (account {}) → '{}' (verify={})",
        uid, req.account_id, req.target_folder, eml_ok
    );
    Ok(Json(()))
}

#[derive(Deserialize)]
pub struct RenameFolderRequest {
    pub account_id: u32,
    pub old_name: String,
    pub new_name: String,
}

/// `POST /api/v1/folders/rename`
pub async fn rename_folder(
    State(state): State<AppState>,
    Json(req): Json<RenameFolderRequest>,
) -> ApiResult<()> {
    // Local-only folders: rename purely in the cache (no IMAP involved).
    let is_local = with_db(&state, |conn| {
        cache::messages::is_local_only_folder(conn, req.account_id as i64, &req.old_name)
            .map_err(|e| e.to_string())
    })
    .unwrap_or(false);
    if is_local {
        with_db(&state, |conn| {
            cache::messages::rename_local_folder(conn, req.account_id as i64, &req.old_name, &req.new_name)
                .map_err(|e| e.to_string())
        })?;
        return Ok(Json(()));
    }

    let client = {
        let guard = state.imap_clients.read();
        guard
            .get(&req.account_id)
            .cloned()
            .ok_or(ApiError("IMAP-Client nicht gefunden".into()))?
    };
    client
        .rename_folder(&req.old_name, &req.new_name)
        .await
        .map(|_| Json(()))
        .map_err(|e| ApiError(e.to_string()))
}

/// `POST /api/v1/folders/delete` — delete a folder.
/// Local-only folders are removed from the cache (incl. EML files);
/// IMAP folders are deleted on the provider.
#[derive(Deserialize)]
pub struct DeleteFolderRequest {
    pub account_id: u32,
    pub name: String,
}

pub async fn delete_folder(
    State(state): State<AppState>,
    Json(req): Json<DeleteFolderRequest>,
) -> ApiResult<serde_json::Value> {
    let is_local = with_db(&state, |conn| {
        cache::messages::is_local_only_folder(conn, req.account_id as i64, &req.name)
            .map_err(|e| e.to_string())
    })
    .unwrap_or(false);

    if is_local {
        let deleted = with_db(&state, |conn| {
            cache::messages::delete_local_folder(conn, &state.data_root, req.account_id as i64, &req.name)
        })?;
        tracing::info!("Lokaler Ordner '{}' gelöscht ({} Mails)", req.name, deleted);
        return Ok(Json(serde_json::json!({ "ok": true, "deleted": deleted })));
    }

    let client = {
        let guard = state.imap_clients.read();
        guard.get(&req.account_id).cloned()
    };
    if let Some(client) = client {
        // Try the IMAP deletion; the folder may exist only locally (e.g. a
        // migration target folder that the sync mirrored but GMX doesn't
        // know). If the remote deletion fails for any reason, still remove
        // the local rows — the user explicitly asked to delete this folder.
        match client.delete_folder(&req.name).await {
            Ok(()) => {
                with_db(&state, |conn| {
                    conn.execute(
                        "DELETE FROM messages WHERE account_id = ?1 AND folder_id = (SELECT id FROM folders WHERE account_id = ?1 AND name = ?2)",
                        rusqlite::params![req.account_id as i64, req.name],
                    )
                    .map_err(|e| e.to_string())?;
                    conn.execute(
                        "DELETE FROM folders WHERE account_id = ?1 AND name = ?2",
                        rusqlite::params![req.account_id as i64, req.name],
                    )
                    .map_err(|e| e.to_string())
                })?;
                return Ok(Json(serde_json::json!({ "ok": true })));
            }
            Err(e) => {
                tracing::warn!(
                    "IMAP-Löschung '{}' fehlgeschlagen ({}), lösche lokal weiter",
                    req.name, e
                );
            }
        }
    } else {
        tracing::warn!("Kein IMAP-Client — Ordner '{}' wird nur lokal gelöscht", req.name);
    }
    // Local fallback: remove the folder rows + EML archives regardless of
    // the IMAP state (folder may not exist remotely, or no session is up).
    let deleted = with_db(&state, |conn| {
        cache::messages::delete_local_folder(conn, &state.data_root, req.account_id as i64, &req.name)
    })?;
    tracing::info!("Ordner '{}' lokal gelöscht ({} Mails)", req.name, deleted);
    Ok(Json(serde_json::json!({ "ok": true, "deleted": deleted, "local_only": true })))
}

/// `GET /api/v1/messages/{uid}/raw?account_id=…`
pub async fn fetch_raw_message(
    State(state): State<AppState>,
    Query(q): Query<MessageUidQuery>,
) -> ApiResult<String> {
    let uid = q.uid;
    let client = state
        .imap_clients
        .read()
        .get(&q.account_id)
        .cloned()
        .ok_or(ApiError("IMAP-Client nicht gefunden".into()))?;
    client
        .fetch_raw_message(uid)
        .await
        .map(Json)
        .map_err(|e| ApiError(e.to_string()))
}

/// `GET /api/v1/messages/{uid}/attachments?account_id=…`
pub async fn fetch_attachments(
    State(state): State<AppState>,
    Query(q): Query<MessageUidQuery>,
) -> ApiResult<Vec<crate::cache::attachments::CachedAttachment>> {
    let uid = q.uid;
    let db_guard = get_db(&state).map_err(|e| ApiError(e))?;
    let conn = db_guard.as_ref().ok_or(ApiError("Datenbank nicht initialisiert".into()))?;

    let message_id: i64 = conn
        .query_row(
            "SELECT id FROM messages WHERE account_id = ? AND uid = ?",
            rusqlite::params![q.account_id as i64, uid as i64],
            |r| r.get(0),
        )
        .map_err(|e| ApiError(e.to_string()))?;

    crate::cache::attachments::get_attachments(conn, message_id)
        .map(Json)
        .map_err(|e| ApiError(e.to_string()))
}

/// `GET /api/v1/messages/{uid}/attachments/{att_id}/content?account_id=…`
///
/// Loads the attachment content (from the local dedup store if present,
/// otherwise from IMAP raw fetch), persists it deduplicated under
/// `attachments/<sha256>` (Concept §3.1 / 4J), and returns base64.
pub async fn fetch_attachment_content(
    State(state): State<AppState>,
    Query(q): Query<AttachmentContentQuery>,
) -> ApiResult<serde_json::Value> {
    let uid = q.uid;
    let att_id = q.att_id;
    // 1. Look up the message + attachment metadata.
    let filename = {
        let db_guard = get_db(&state).map_err(|e| ApiError(e))?;
        let conn = db_guard.as_ref().ok_or(ApiError("Datenbank nicht initialisiert".into()))?;
        let message_id: i64 = conn
            .query_row(
                "SELECT id FROM messages WHERE account_id = ?1 AND uid = ?2",
                rusqlite::params![q.account_id as i64, uid as i64],
                |r| r.get(0),
            )
            .map_err(|e| ApiError(format!("Nachricht nicht gefunden: {e}")))?;
        conn.query_row(
            "SELECT filename FROM message_attachments WHERE id = ?1 AND message_id = ?2",
            rusqlite::params![att_id as i64, message_id],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| ApiError(format!("Anhang nicht gefunden: {e}")))?
    };

    // 2. If already on disk (dedup store), serve from there.
    let disk_hit = {
        let db_guard = get_db(&state).map_err(|e| ApiError(e))?;
        let conn = db_guard.as_ref().ok_or(ApiError("Datenbank nicht initialisiert".into()))?;
        let rel: Option<String> = conn
            .query_row(
                "SELECT disk_path FROM message_attachments WHERE id = ?1",
                rusqlite::params![att_id as i64],
                |r| r.get(0),
            )
            .ok();
        rel.and_then(|r| {
            let abs = state.data_root.join(&r);
            std::fs::read(&abs).ok().map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
        })
    };
    if let Some(content) = disk_hit {
        let _ = filename;
        return Ok(Json(serde_json::json!({ "content": content, "cached": true })));
    }

    // 3a. Local EML archive: if the message has a raw EML on disk, extract
    //     the attachment from there — works for archived/migrated messages
    //     with no IMAP session (and is faster).
    let local_eml: Option<String> = {
        let db_guard = get_db(&state).map_err(|e| ApiError(e))?;
        let conn = db_guard.as_ref().ok_or(ApiError("Datenbank nicht initialisiert".into()))?;
        let raw_rel: Option<String> = conn
            .query_row(
                "SELECT raw_path FROM messages WHERE account_id = ?1 AND uid = ?2",
                rusqlite::params![q.account_id as i64, uid as i64],
                |r| r.get(0),
            )
            .ok();
        raw_rel.and_then(|rel| {
            let abs = state.data_root.join(&rel);
            std::fs::read(&abs).ok().map(|bytes| String::from_utf8_lossy(&bytes).to_string())
        })
    };
    if let Some(raw) = local_eml {
        let attachments = client::parse_message_attachments(raw.as_bytes());
        // Match by filename; if the DB row carries no metadata (filename is
        // empty — happens when BODYSTRUCTURE provided no names), fall back to
        // matching by position: the attachments are stored in parse order, so
        // the k-th DB row corresponds to the k-th parsed attachment.
        let found = if !filename.is_empty() {
            attachments.iter().find(|a| a.filename == filename)
        } else {
            // Determine our ordinal position among the message's attachments.
            let ordinal: Option<usize> = {
                let db_guard = get_db(&state).map_err(|e| ApiError(e))?;
                let conn = db_guard.as_ref().ok_or(ApiError("Datenbank nicht initialisiert".into()))?;
                conn.query_row(
                    "SELECT COUNT(*) FROM message_attachments WHERE message_id = ?1 AND id <= ?2",
                    rusqlite::params![
                        conn.query_row(
                            "SELECT id FROM messages WHERE account_id = ?1 AND uid = ?2",
                            rusqlite::params![q.account_id as i64, uid as i64],
                            |r| r.get::<_, i64>(0),
                        )
                        .unwrap_or(0),
                        att_id as i64,
                    ],
                    |r| r.get::<_, i64>(0),
                )
                .ok()
                .map(|c| c.saturating_sub(1) as usize)
            };
            ordinal.and_then(|idx| attachments.get(idx))
        };
        if let Some(att) = found {
            {
                let db_guard = get_db(&state).map_err(|e| ApiError(e))?;
                let conn = db_guard.as_ref().ok_or(ApiError("Datenbank nicht initialisiert".into()))?;
                let _ = crate::cache::attachments::cache_content_dedup(conn, att_id as i64, &att.content, &state.data_root);
            }
            return Ok(Json(serde_json::json!({ "content": att.content, "cached": false })));
        }
    }

    // 3b. Fallback: fetch raw from IMAP, extract attachments, find ours.
    let client = state
        .imap_clients
        .read()
        .get(&q.account_id)
        .cloned()
        .ok_or(ApiError("IMAP-Client nicht gefunden".into()))?;
    if !client.is_connected().await {
        client.connect().await.map_err(|e| ApiError(e.to_string()))?;
    }
    let raw = client
        .fetch_raw_message(uid)
        .await
        .map_err(|e| ApiError(e.to_string()))?;
    let attachments = client::parse_message_attachments(raw.as_bytes());
    let att = attachments
        .iter()
        .find(|a| a.filename == filename)
        .ok_or(ApiError("Anhang im IMAP-Fetch nicht gefunden".into()))?;

    // 4. Persist deduplicated to disk + record disk_path (Phase 4J).
    {
        let db_guard = get_db(&state).map_err(|e| ApiError(e))?;
        let conn = db_guard.as_ref().ok_or(ApiError("Datenbank nicht initialisiert".into()))?;
        let _ = crate::cache::attachments::cache_content_dedup(conn, att_id as i64, &att.content, &state.data_root);
    }

    Ok(Json(serde_json::json!({ "content": att.content, "cached": false })))
}

// ─── State mutations ───────────────────────────────────────────

/// `POST /api/v1/messages/{uid}/read` body: `{"account_id": N}`
pub async fn mark_as_read(
    State(state): State<AppState>,
    Json(req): Json<MessageActionRequest>,
) -> ApiResult<()> {
    let uid = req.uid;
    with_db(&state, |conn| {
        cache::messages::mark_as_read(conn, req.account_id as i64, uid as i64)
            .map_err(|e| e.to_string())
    })?;
    let client = {
        let guard = state.imap_clients.read();
        guard.get(&req.account_id).cloned()
    };
    if let Some(client) = client {
        let folder = folder_for_message(&state, req.account_id, uid);
        client.mark_flag(uid, "\\Seen", true, folder).await.map_err(|e| ApiError(e.to_string()))?;
    }
    Ok(Json(()))
}

/// `POST /api/v1/messages/{uid}/unread` body: `{"account_id": N}`
pub async fn mark_as_unseen(
    State(state): State<AppState>,
    Json(req): Json<MessageActionRequest>,
) -> ApiResult<()> {
    let uid = req.uid;
    {
        let db_guard = get_db(&state).map_err(ApiError)?;
        if let Some(conn) = db_guard.as_ref() {
            conn.execute(
                "UPDATE messages SET is_read = 0, updated_at = datetime('now') WHERE account_id = ?1 AND uid = ?2",
                rusqlite::params![req.account_id as i64, uid as i64],
            )
            .map_err(|e| ApiError(e.to_string()))?;
        }
    }
    let client = {
        let guard = state.imap_clients.read();
        guard.get(&req.account_id).cloned()
    };
    if let Some(client) = client {
        let folder = folder_for_message(&state, req.account_id, uid);
        client.mark_flag(uid, "\\Seen", false, folder).await.map_err(|e| ApiError(e.to_string()))?;
    }
    Ok(Json(()))
}

/// `POST /api/v1/messages/flag` — toggle the \Flagged (star) flag.
#[derive(Deserialize)]
pub struct FlagRequest {
    pub account_id: u32,
    pub uid: u32,
    pub folder_name: String,
    pub flagged: bool,
}

pub async fn flag_message(
    State(state): State<AppState>,
    Json(req): Json<FlagRequest>,
) -> ApiResult<()> {
    with_db(&state, |conn| {
        conn.execute(
            "UPDATE messages SET is_flagged = ?1, updated_at = datetime('now') WHERE account_id = ?2 AND uid = ?3",
            rusqlite::params![req.flagged as i32, req.account_id as i64, req.uid as i64],
        )
        .map_err(|e| e.to_string())
    })?;
    // IMAP flag sync runs in a helper so the TLS session never crosses await
    // points inside this handler.
    let account_id = req.account_id;
    let uid = req.uid;
    let flagged = req.flagged;
    set_imap_flag(&state, account_id, uid, flagged).await;
    Ok(Json(()))
}

async fn set_imap_flag(state: &AppState, account_id: u32, uid: u32, flagged: bool) {
    let Some(client) = state.imap_clients.read().get(&account_id).cloned() else {
        return;
    };
    if client.is_connected().await {
        let folder = folder_for_message(state, account_id, uid);
        let _ = client.toggle_flagged(uid, flagged).await;
        let _ = folder;
    }
}

/// Look up the folder a message currently lives in (for IMAP SELECT before
/// UID STORE operations).
fn folder_for_message(state: &AppState, account_id: u32, uid: u32) -> Option<String> {
    let db_guard = state.cache_db.lock();
    let conn = db_guard.as_ref()?;
    conn.query_row(
        "SELECT f.name FROM messages m JOIN folders f ON f.id = m.folder_id WHERE m.account_id = ?1 AND m.uid = ?2",
        rusqlite::params![account_id as i64, uid as i64],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

/// `POST /api/v1/messages/move-cross-account` — move a message between two
/// accounts (fetch raw from source, append to target, delete source copy).
#[derive(Deserialize)]
pub struct MoveCrossAccountRequest {
    pub account_id: u32,
    pub uid: u32,
    pub source_folder: String,
    pub target_account_id: u32,
    pub target_folder: String,
}

pub async fn move_cross_account(
    State(state): State<AppState>,
    Json(req): Json<MoveCrossAccountRequest>,
) -> ApiResult<()> {
    // 1. Fetch raw from the source account.
    let raw = {
        let client = state
            .imap_clients
            .read()
            .get(&req.account_id)
            .cloned()
            .ok_or(ApiError("Quell-IMAP-Client nicht gefunden".into()))?;
        if !client.is_connected().await {
            client.connect().await.map_err(|e| ApiError(e.to_string()))?;
        }
        client.fetch_raw_message_in_folder(req.uid, Some(req.source_folder.clone()))
            .await
            .map_err(|e| ApiError(e.to_string()))?
    };

    // 2. Determine whether the target folder is a LOCAL-only folder (e.g. a
    //    migration target or a locally created folder that has no IMAP
    //    counterpart). For those, an IMAP APPEND would fail with "folder does
    //    not exist" — instead copy the message into the target account's local
    //    archive (EML + DB row), byte-identical.
    let target_is_local = with_db(&state, |conn| {
        Ok(conn
            .query_row(
                "SELECT local_only FROM folders WHERE account_id = ?1 AND name = ?2",
                rusqlite::params![req.target_account_id as i64, req.target_folder],
                |row| row.get::<_, i32>(0),
            )
            .unwrap_or(0)
            == 1)
    })
    .unwrap_or(false);

    if target_is_local {
        // Local cross-account move: write EML into the target archive.
        let (subject, from_addr, to_addr, date, mid, raw_sha256) = with_db(&state, |conn| {
            Ok((
                conn.query_row(
                    "SELECT subject FROM messages WHERE account_id = ?1 AND uid = ?2",
                    rusqlite::params![req.account_id as i64, req.uid as i64],
                    |r| r.get::<_, Option<String>>(0),
                )
                .unwrap_or(None),
                conn.query_row(
                    "SELECT from_addr FROM messages WHERE account_id = ?1 AND uid = ?2",
                    rusqlite::params![req.account_id as i64, req.uid as i64],
                    |r| r.get::<_, Option<String>>(0),
                )
                .unwrap_or(None),
                conn.query_row(
                    "SELECT to_addr FROM messages WHERE account_id = ?1 AND uid = ?2",
                    rusqlite::params![req.account_id as i64, req.uid as i64],
                    |r| r.get::<_, Option<String>>(0),
                )
                .unwrap_or(None),
                conn.query_row(
                    "SELECT date FROM messages WHERE account_id = ?1 AND uid = ?2",
                    rusqlite::params![req.account_id as i64, req.uid as i64],
                    |r| r.get::<_, Option<String>>(0),
                )
                .unwrap_or(None),
                conn.query_row(
                    "SELECT message_id FROM messages WHERE account_id = ?1 AND uid = ?2",
                    rusqlite::params![req.account_id as i64, req.uid as i64],
                    |r| r.get::<_, Option<String>>(0),
                )
                .unwrap_or(None),
                conn.query_row(
                    "SELECT raw_sha256 FROM messages WHERE account_id = ?1 AND uid = ?2",
                    rusqlite::params![req.account_id as i64, req.uid as i64],
                    |r| r.get::<_, Option<String>>(0),
                )
                .unwrap_or(None),
            ))
        })
        .map_err(|e: String| ApiError(e))?;

        let date_str = date.as_deref();
        let mid_str = mid.as_deref();
        let target_path = crate::cache::archive::write_eml(
            &state.data_root,
            req.target_account_id as i64,
            req.uid,
            date_str,
            mid_str,
            raw.as_bytes(),
        )
        .map_err(|e| ApiError(e.to_string()))?;

        let rel = target_path
            .strip_prefix(&state.data_root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| target_path.to_string_lossy().to_string());

        let mut h = sha2::Sha256::new();
        h.update(&raw);
        let computed_sha = format!("{:x}", h.finalize());
        let verify_sha = raw_sha256.clone().unwrap_or(computed_sha.clone());

        with_db(&state, |conn| {
            let folder_id: i64 = conn
                .query_row(
                    "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
                    rusqlite::params![req.target_account_id as i64, req.target_folder],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT OR IGNORE INTO messages
                 (account_id, folder_id, uid, message_id, subject, from_addr, to_addr, date,
                  body_text, body_html, flags, ai_summary, ai_priority, ai_fraud_score,
                  is_read, is_flagged, has_attachments, synced, raw_path, raw_sha256)
                 VALUES (
                   ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                   (SELECT body_text FROM messages WHERE account_id = ?9 AND uid = ?3),
                   (SELECT body_html FROM messages WHERE account_id = ?9 AND uid = ?3),
                   ?10, NULL, NULL, NULL, ?11, ?12, ?13, 0, ?14, ?15
                 )",
                rusqlite::params![
                    req.target_account_id as i64, folder_id, req.uid as i64,
                    mid, subject, from_addr, to_addr, date,
                    req.account_id as i64,
                    "[]", 0i32, 0i32, 0i32,
                    rel, verify_sha,
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok::<(), String>(())
        })
        .map_err(|e: String| ApiError(e))?;

        // 3. Delete the source copy (hard) — from the local DB only. The
        //    source account's EML stays (source account untouched on IMAP).
        with_db(&state, |conn| {
            conn.execute(
                "DELETE FROM messages WHERE account_id = ?1 AND uid = ?2",
                rusqlite::params![req.account_id as i64, req.uid as i64],
            )
            .map_err(|e| e.to_string())?;
            Ok::<(), String>(())
        })
        .map_err(|e: String| ApiError(e))?;

        return Ok(Json(()));
    }

    // 3. (non-local target) Append to the target account's IMAP folder.
    {
        let client = state
            .imap_clients
            .read()
            .get(&req.target_account_id)
            .cloned()
            .ok_or(ApiError("Ziel-IMAP-Client nicht gefunden".into()))?;
        if !client.is_connected().await {
            client.connect().await.map_err(|e| ApiError(e.to_string()))?;
        }
        client.append_message(&req.target_folder, raw.as_bytes(), None)
            .await
            .map_err(|e| ApiError(format!("Append zum Ziel fehlgeschlagen: {}", e)))?;
    }

    // 4. Delete the source copy (hard).
    {
        let client = state
            .imap_clients
            .read()
            .get(&req.account_id)
            .cloned()
            .ok_or(ApiError("Quell-IMAP-Client nicht gefunden".into()))?;
        let _ = client.hard_delete_message(req.uid, &req.source_folder).await;
    }

    // 5. Update local cache: move the row to the target account's folder.
    with_db(&state, |conn| {
        let folder_id: i64 = conn
            .query_row(
                "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
                rusqlite::params![req.target_account_id as i64, req.target_folder],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if folder_id > 0 {
            conn.execute(
                "UPDATE messages SET account_id = ?1, folder_id = ?2 WHERE account_id = ?3 AND uid = ?4",
                rusqlite::params![req.target_account_id as i64, folder_id, req.account_id as i64, req.uid as i64],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok::<(), String>(())
    })?;

    Ok(Json(()))
}

/// `POST /api/v1/messages/{uid}/delete` body: `{"account_id": N}`
pub async fn delete_message(
    State(state): State<AppState>,
    Json(req): Json<MessageActionRequest>,
) -> ApiResult<()> {
    let uid = req.uid;
    let account_id = req.account_id;
    let account_id_i64 = account_id as i64;
    let uid_i64 = uid as i64;

    let move_to_trash = with_db(&state, |conn| {
        cache::settings::get_move_to_trash(conn).map_err(|e| e.to_string())
    })
    .unwrap_or(true);

    // Archive-mode accounts use the LOCAL trash (EML stays; provider deletion
    // is queued + verified). Mirror accounts keep the classic IMAP-trash move.
    let sync_mode: String = with_db(&state, |conn| {
        Ok(conn
            .query_row(
                "SELECT sync_mode FROM accounts WHERE id = ?1",
                rusqlite::params![account_id_i64],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "mirror".to_string()))
    })
    .unwrap_or_else(|_| "mirror".to_string());

    if move_to_trash && sync_mode == "archive" {
        return delete_message_archive_trash(&state, account_id, uid, account_id_i64, uid_i64).await;
    }

    if move_to_trash {
        return delete_message_trash_mode(&state, account_id, uid, account_id_i64, uid_i64).await;
    }

    delete_message_permanent_delete(&state, account_id, uid, account_id_i64, uid_i64).await
}

#[derive(Deserialize)]
pub struct AccountIdRequest {
    pub account_id: u32,
}

/// Action request for single-message mutations (uid + account in the body).
#[derive(Deserialize)]
pub struct MessageActionRequest {
    pub account_id: u32,
    pub uid: u32,
}

/// Delete by moving the message to the Trash folder instead of permanent delete.
async fn delete_message_trash_mode(
    state: &AppState,
    account_id: u32,
    uid: u32,
    account_id_i64: i64,
    uid_i64: i64,
) -> ApiResult<()> {
    // Step 1: Look up the source folder
    let folder: Option<String> = with_db(state, |conn| {
        Ok(conn
            .query_row(
                "SELECT f.name FROM messages m \
                 JOIN folders f ON m.folder_id = f.id \
                 WHERE m.account_id = ?1 AND m.uid = ?2",
                rusqlite::params![account_id_i64, uid_i64],
                |row| row.get(0),
            )
            .ok())
    })
    .unwrap_or(None);

    let source_folder = match folder {
        Some(ref f) if f != "Trash" => f.clone(),
        Some(_) => {
            return delete_message_permanent_delete(state, account_id, uid, account_id_i64, uid_i64)
                .await;
        }
        None => {
            let _ = with_db(state, |conn| {
                cache::messages::delete_message(conn, account_id_i64, uid_i64)
                    .map_err(|e| e.to_string())
            });
            return Ok(Json(()));
        }
    };

    // Step 2: Get IMAP client
    let client = match state.imap_clients.read().get(&account_id).cloned() {
        Some(c) => c,
        None => {
            let _ = with_db(state, |conn| {
                cache::messages::update_folder(conn, account_id_i64, uid_i64, "Trash")
                    .map_err(|e| e.to_string())
            });
            return Ok(Json(()));
        }
    };

    // Step 3: Ensure IMAP connection
    if !client.is_connected().await {
        client
            .connect()
            .await
            .map_err(|e| ApiError(format!("IMAP reconnect fehlgeschlagen: {}", e)))?;
    }

    // Step 4: Auto-create Trash folder if it doesn't exist
    let _ = client.create_folder("Trash").await;

    // Step 5: Move message to Trash on IMAP
    if let Err(e) = client.move_message(uid, &source_folder, "Trash").await {
        return Err(ApiError(format!("Verschieben in Papierkorb fehlgeschlagen: {}", e)));
    }

    // Step 6: Update folder in cache
    with_db(state, |conn| {
        cache::messages::update_folder(conn, account_id_i64, uid_i64, "Trash")
            .map_err(|e| e.to_string())
    })?;
    Ok(Json(()))
}

/// Archive-mode trash: move the local index row into the local Trash folder
/// (EML archive untouched) and enqueue the provider deletion. A retention
/// worker removes the local copy after `trash_retention_days`.
async fn delete_message_archive_trash(
    state: &AppState,
    account_id: u32,
    uid: u32,
    account_id_i64: i64,
    uid_i64: i64,
) -> ApiResult<()> {
    let source_folder: Option<String> = with_db(state, |conn| {
        Ok(conn
            .query_row(
                "SELECT f.name FROM messages m \
                 JOIN folders f ON m.folder_id = f.id \
                 WHERE m.account_id = ?1 AND m.uid = ?2",
                rusqlite::params![account_id_i64, uid_i64],
                |row| row.get(0),
            )
            .ok())
    })
    .unwrap_or(None);

    let message_id: Option<i64> = with_db(state, |conn| {
        Ok(conn
            .query_row(
                "SELECT id FROM messages WHERE account_id = ?1 AND uid = ?2",
                rusqlite::params![account_id_i64, uid_i64],
                |row| row.get(0),
            )
            .ok())
    })
    .unwrap_or(None);

    // 1. Locally move the row into the local Trash folder (EML stays).
    //    Scoped to the source folder — uid is only unique per folder, so
    //    without the scope every message sharing this uid (across folders)
    //    would be moved/orphaned.
    with_db(state, |conn| {
        match source_folder.as_deref() {
            Some(src) => cache::messages::update_folder_from(conn, account_id_i64, uid_i64, src, "Trash")
                .map_err(|e| e.to_string()),
            None => cache::messages::update_folder(conn, account_id_i64, uid_i64, "Trash")
                .map_err(|e| e.to_string()),
        }
    })?;

    // 2. Enqueue provider deletion (verified by the worker before hard delete).
    if let (Some(mid), Some(ref f)) = (message_id, source_folder) {
        let _ = with_db(state, |conn| {
            crate::cache::delete_queue::enqueue(conn, mid, account_id_i64, uid_i64, f, "delete")
                .map_err(|e| e.to_string())
        });
    }

    tracing::info!(
        "delete_message (archive): uid {} (Konto {}) → lokaler Papierkorb, Provider-Löschung in Queue",
        uid, account_id
    );
    Ok(Json(()))
}

/// Permanent delete helper.
async fn delete_message_permanent_delete(
    state: &AppState,
    account_id: u32,
    uid: u32,
    account_id_i64: i64,
    uid_i64: i64,
) -> ApiResult<()> {
    let folder: Option<String> = with_db(state, |conn| {
        Ok(conn
            .query_row(
                "SELECT f.name FROM messages m \
                 JOIN folders f ON m.folder_id = f.id \
                 WHERE m.account_id = ?1 AND m.uid = ?2",
                rusqlite::params![account_id_i64, uid_i64],
                |row| row.get(0),
            )
            .ok())
    })
    .unwrap_or(None);

    let message_id: Option<i64> = with_db(state, |conn| {
        Ok(conn
            .query_row(
                "SELECT id FROM messages WHERE account_id = ?1 AND uid = ?2",
                rusqlite::params![account_id_i64, uid_i64],
                |row| row.get(0),
            )
            .ok())
    })
    .unwrap_or(None);

    // 1. Remove the local index row (user intent; EML archive stays untouched).
    let cache_result = with_db(state, |conn| {
        cache::messages::delete_message(conn, account_id_i64, uid_i64).map_err(|e| e.to_string())
    });
    if let Err(e) = cache_result {
        return Err(ApiError(format!("Cache delete fehlgeschlagen: {}", e)));
    }

    // 2. Enqueue provider deletion — verified by the worker before touching IMAP.
    if let (Some(mid), Some(ref f)) = (message_id, folder) {
        let _ = with_db(state, |conn| {
            crate::cache::delete_queue::enqueue(conn, mid, account_id_i64, uid_i64, f, "delete")
                .map_err(|e| e.to_string())
        });
        tracing::info!(
            "delete_message: uid {} (Konto {}) lokal entfernt, Provider-Löschung in Queue",
            uid, account_id
        );
    }

    Ok(Json(()))
}
