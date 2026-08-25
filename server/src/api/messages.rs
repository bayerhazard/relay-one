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
///
/// `synced` rows carry IMAP transfer-encoded bodies (parsed/decoded on read),
/// while local-only rows (drafts, local folders) store plain text that must be
/// returned verbatim — a draft containing e.g. `a=3Db` must not be re-encoded.
fn decode_body_text(b: &str, synced: bool) -> String {
    if !synced {
        return b.to_string();
    }
    if client::has_qp_pattern(b) {
        client::decode_transfer_encoding(b)
    } else {
        b.to_string()
    }
}

/// Serialize a MessageRecord to JSON.
fn message_to_json(m: &MessageRecord) -> serde_json::Value {
    serde_json::json!({
        "id": m.id,
        "uid": m.uid,
        "message_id": m.message_id,
        "subject": m.subject.as_deref().map(client::decode_rfc2047),
        "from": m.from_addr,
        "to": m.to_addr,
        "cc": m.cc_addr,
        "date": m.date,
        "body_preview": body_preview(m),
        "body_text": m.body_text.as_deref().map(|b| decode_body_text(b, m.synced)),
        "body_html": m.body_html.clone(),
        "flags": m.flags,
        "ai_summary": m.ai_summary,
        "ai_priority": m.ai_priority,
        "ai_fraud_score": m.ai_fraud_score,
        "is_read": m.is_read,
        "is_flagged": m.is_flagged,
        "has_attachments": m.has_attachments,
    })
}

/// Preview of a message body (first 200 chars, QP-decoded for synced rows).
fn body_preview(m: &MessageRecord) -> Option<String> {
    m.body_text.as_deref().map(|b| {
        decode_body_text(b, m.synced).chars().take(200).collect::<String>()
    })
}

/// Lightweight list serialization — omits `body_text`/`body_html` so large
/// folders (up to 10k rows) transfer as metadata-only JSON. The full body is
/// fetched on demand via `GET /messages/{uid}/body`.
fn message_to_json_meta(m: &MessageRecord) -> serde_json::Value {
    serde_json::json!({
        "id": m.id,
        "uid": m.uid,
        "message_id": m.message_id,
        "subject": m.subject.as_deref().map(client::decode_rfc2047),
        "from": m.from_addr,
        "to": m.to_addr,
        "cc": m.cc_addr,
        "date": m.date,
        "body_preview": body_preview(m),
        "flags": m.flags,
        "ai_summary": m.ai_summary,
        "ai_priority": m.ai_priority,
        "ai_fraud_score": m.ai_fraud_score,
        "is_read": m.is_read,
        "is_flagged": m.is_flagged,
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
    /// When set, the list response omits `body_text`/`body_html` (metadata-only).
    /// The full body is loaded on demand via `GET /messages/{uid}/body`.
    #[serde(default, deserialize_with = "deserialize_boolish")]
    pub list_only: Option<bool>,
}

fn deserialize_boolish<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = Option::<String>::deserialize(deserializer)?;
    match s.as_deref() {
        None | Some("") => Ok(None),
        Some("1") | Some("true") | Some("True") | Some("TRUE") | Some("on") | Some("yes") => Ok(Some(true)),
        Some("0") | Some("false") | Some("False") | Some("FALSE") | Some("off") | Some("no") => Ok(Some(false)),
        Some(other) => Err(serde::de::Error::custom(format!(
            "ungültiger bool-Wert: {other}"
        ))),
    }
}

/// Query for single-message endpoints (uid + account in the query string).
#[derive(Deserialize)]
pub struct MessageUidQuery {
    pub account_id: u32,
    pub uid: u32,
    /// Optional folder name to disambiguate uid collisions (IMAP uids are
    /// unique per folder). Drafts live in "Entwürfe", where a uid may also
    /// exist in INBOX — without this the wrong body could be returned.
    pub folder: Option<String>,
}

/// Query for attachment content (uid + att_id + account in the query string).
#[derive(Deserialize)]
pub struct AttachmentContentQuery {
    pub account_id: u32,
    pub uid: u32,
    pub att_id: u32,
    /// Optional folder name to disambiguate uid collisions — mirror of
    /// `MessageUidQuery.folder`. Without it, an attachment lookup could
    /// resolve to a message that shares the uid in another folder.
    pub folder: Option<String>,
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
    let folder_name = q.folder.clone().unwrap_or_else(|| "INBOX".to_string());
    let list_only = q.list_only.unwrap_or(false);

    // Meta-only list requests hit the server-side cache first so repeated
    // folder switches are instant (no DB query + JSON re-serialization).
    if list_only {
        if let Some(cached) = state.folder_cache.read().get(q.account_id as i64, &folder_name) {
            return Ok(Json(cached));
        }
    }

    let messages = with_db(&state, |conn| {
        if list_only {
            cache::messages::fetch_inbox_meta(
                conn,
                q.account_id as i64,
                q.limit.map(|v| v as i64),
                q.offset.map(|v| v as i64),
                &folder_name,
            )
            .map_err(|e| e.to_string())
        } else {
            cache::messages::fetch_inbox(
                conn,
                q.account_id as i64,
                q.limit.map(|v| v as i64),
                q.offset.map(|v| v as i64),
                &folder_name,
            )
            .map_err(|e| e.to_string())
        }
    })?;

    let json: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            if list_only {
                message_to_json_meta(m)
            } else {
                message_to_json(m)
            }
        })
        .collect();

    if list_only {
        state
            .folder_cache
            .write()
            .put(q.account_id as i64, &folder_name, json.clone());
    }
    Ok(Json(json))
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
        cache::messages::fetch_message_body_with_folder(conn, q.account_id as i64, uid as i64, q.folder.as_deref())
            .map_err(|e| e.to_string())
    })?;

    if let Some((ref msg, _)) = cached_with_folder {
        // A cached body only counts when it is non-empty. Rows may carry an
        // empty string (''), which the EML-fallback / IMAP path below must be
        // allowed to fill in — an empty string is not a usable body.
        if msg.body_text.as_deref().map_or(false, |b| !b.trim().is_empty()) {
            // Attachments (metadata + inline base64 content where materialized).
            // Draft attachments live in the dedup store (no IMAP needed), so
            // they can be handed straight to the compose editor.
            let attachments: Vec<serde_json::Value> = with_db(&state, |conn| {
                let list = crate::cache::attachments::get_attachments(conn, msg.id)
                    .map_err(|e| e.to_string())?;
                let mut out = Vec::new();
                for a in list {
                    let content = attachment_inline_content(conn, &state, &a);
                    out.push(serde_json::json!({
                        "id": a.id,
                        "part_index": a.part_index,
                        "filename": a.filename,
                        "content_type": a.content_type,
                        "size": a.size,
                        "content": content,
                        "content_cached": a.content_cached,
                    }));
                }
                Ok::<Vec<serde_json::Value>, String>(out)
            })?;
            return Ok(Json(serde_json::json!({
                "id": msg.id,
                "uid": msg.uid,
                "subject": msg.subject.as_deref().map(client::decode_rfc2047),
                "from": msg.from_addr,
                "to": msg.to_addr,
                "date": msg.date,
                "body_text": msg.body_text.as_deref().map(|b| decode_body_text(b, msg.synced)),
                "body_html": msg.body_html,
                "flags": msg.flags,
                "ai_summary": msg.ai_summary,
                "ai_priority": msg.ai_priority,
                "ai_fraud_score": msg.ai_fraud_score,
                "is_read": msg.is_read,
                "attachments": attachments,
            })));
        }
    }

    // EML-archive fallback: only INBOX gets its body during sync, and local-only
    // folders never reach the IMAP fallback below. If the mail has an archived
    // EML, re-parse it now and write the full body back (also caches it for the
    // next open). Missing body + missing archive → falls through to IMAP.
    if let Some((msg, _)) = &cached_with_folder {
        let raw_path: Option<String> = with_db(&state, |conn| {
            Ok::<_, String>(
                conn.query_row(
                    "SELECT raw_path FROM messages WHERE id = ?1",
                    [msg.id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .ok()
                .flatten(),
            )
        })
        .ok()
        .flatten();
        if let Some(rp) = raw_path.filter(|p| !p.is_empty()) {
            if let Some(raw) = crate::cache::archive::read_eml(&state.data_root, &rp) {
                let (text, html) = client::parse_message_bodies(&raw);
                if !text.trim().is_empty() || html.as_deref().map_or(false, |h| !h.trim().is_empty()) {
                    let _: Result<(), String> = with_db(&state, |conn| {
                        cache::messages::update_body_by_id(conn, msg.id, &text, html.as_deref())
                            .map_err(|e| e.to_string())
                    });
                    return Ok(Json(serde_json::json!({
                        "id": msg.id,
                        "uid": msg.uid,
                        "subject": msg.subject.as_deref().map(client::decode_rfc2047),
                        "from": msg.from_addr,
                        "to": msg.to_addr,
                        "date": msg.date,
                        "body_text": decode_body_text(&text, true),
                        "body_html": html,
                        "flags": msg.flags,
                        "ai_summary": msg.ai_summary,
                        "ai_priority": msg.ai_priority,
                        "ai_fraud_score": msg.ai_fraud_score,
                        "is_read": msg.is_read,
                        "attachments": [],
                    })));
                }
            }
        }
    }

    // IMAP fallback. Local-only folders (e.g. "Mama und Papa") don't exist on
    // the IMAP server — a SELECT would fail and the user sees an incomplete
    // preview. Skip the IMAP fallback entirely and return what we have cached.
    let folder_name = cached_with_folder.as_ref().map(|(_, f)| f.clone());
    let is_local_only = folder_name
        .as_deref()
        .map(|f| {
            with_db(&state, |conn| {
                Ok::<bool, String>(cache::messages::is_local_only_folder(conn, q.account_id as i64, f).unwrap_or(false))
            })
            .unwrap_or(false)
        })
        .unwrap_or(false);
    if is_local_only {
        if let Some((msg, _)) = cached_with_folder {
            return Ok(Json(message_to_json(&msg)));
        }
        return Err(ApiError("Nachricht nicht gefunden".into()));
    }

    let client = state
        .imap_clients
        .read()
        .get(&q.account_id)
        .cloned()
        .ok_or(ApiError("IMAP-Client nicht gefunden".into()))?;
    if !client.is_connected().await {
        client.connect().await.map_err(|e| ApiError(e.to_string()))?;
    }

    match client.fetch_body_from_folder(uid, folder_name).await {
        Ok((body_text, body_html)) => {
            let _: Result<(), String> = with_db(&state, |conn| {
                match &cached_with_folder {
                    // Prefer the primary-key update: the folder-scoped lookup
                    // above resolved the correct row, and an unscoped
                    // account_id+uid update could hit a row in a different
                    // folder when the same uid exists in multiple folders.
                    Some((msg, _)) => cache::messages::update_body_by_id(
                        conn, msg.id, &body_text, body_html.as_deref(),
                    ),
                    None => cache::messages::update_body(
                        conn, q.account_id as i64, uid as i64,
                        &body_text, body_html.as_deref(),
                    ),
                }
                .map_err(|e| e.to_string())
            });
            Ok(Json(serde_json::json!({
                "uid": uid as i64,
                "body_text": decode_body_text(&body_text, true),
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
    //    Scoped to the source folder — uid is only unique per folder.
    with_db(&state, |conn| {
        cache::messages::update_folder_from(
            conn,
            account_id_i64,
            uid_i64,
            &req.source_folder,
            &req.target_folder,
        )
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
            cache::messages::update_folder_from(
                conn,
                account_id_i64,
                uid_i64,
                &req.target_folder,
                &req.source_folder,
            )
            .map_err(|e| e.to_string())
        });
        return Err(ApiError(format!(
            "Verschieben von '{}' nach '{}' fehlgeschlagen: {}",
            req.source_folder, req.target_folder, e
        )));
    }

    // Bust both folder listings so the raised cache TTL never serves a list
    // that misses this move.
    {
        let mut cache = state.folder_cache.write();
        cache.invalidate(account_id_i64, &req.source_folder);
        cache.invalidate(account_id_i64, &req.target_folder);
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
    //    Scoped to the source folder — uid is only unique per folder.
    with_db(state, |conn| {
        cache::messages::update_folder_from(
            conn,
            account_id_i64,
            uid_i64,
            &req.source_folder,
            &req.target_folder,
        )
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
    {
        let mut cache = state.folder_cache.write();
        cache.invalidate(account_id_i64, &req.source_folder);
        cache.invalidate(account_id_i64, &req.target_folder);
    }
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
        state.folder_cache.write().invalidate_account(req.account_id as i64);
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
        .map(|_| {
            state.folder_cache.write().invalidate_account(req.account_id as i64);
            Json(())
        })
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
    // Every branch below removes rows or the folder itself — one account-wide
    // bust at the entry covers all of them.
    state.folder_cache.write().invalidate_account(req.account_id as i64);
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

/// `POST /api/v1/messages/reparse` — offline backfill of message bodies from
/// the EML archive. Only INBOX gets its body fetched during sync; every other
/// folder stores at most the preview. Local-only folders skip the IMAP body
/// fallback entirely, so mails without a cached body would stay empty. This
/// endpoint reparses every archived EML whose body is missing or preview-only
/// and writes the full text/html into the DB. No IMAP access needed.
///
/// Body is only written when it is genuinely missing or shorter than the
/// parsed content (a preview). Rows without an EML archive are skipped.
pub async fn reparse_eml_bodies(
    State(state): State<AppState>,
) -> ApiResult<serde_json::Value> {
    let data_root = state.data_root.clone();
    let db_guard = get_db(&state).map_err(ApiError)?;
    let conn = db_guard.as_ref().ok_or_else(|| ApiError("Datenbank nicht initialisiert".into()))?;

    // Rows with an archive but empty / missing body, plus preview-only rows.
    let rows = conn
        .prepare(
            "SELECT id, raw_path, body_text FROM messages
             WHERE raw_path IS NOT NULL AND raw_path != ''
               AND (body_text IS NULL OR body_text = '' OR LENGTH(body_text) < 250)",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?))
            })
            .and_then(|it| it.collect::<Result<Vec<_>, _>>())
        })
        .map_err(|e| ApiError(format!("Reparse: Query fehlgeschlagen: {e}")))?;

    let mut updated = 0usize;
    let mut skipped_missing = 0usize;
    let mut skipped_parse_fail = 0usize;

    for (id, raw_path, existing_body) in rows {
        // Defense in depth: read_eml() only accepts paths that resolve inside
        // data_root (absolute container paths or archive-relative paths).
        let Some(raw) = crate::cache::archive::read_eml(&data_root, &raw_path) else {
            skipped_missing += 1;
            continue;
        };
        let (text, html) = client::parse_message_bodies(&raw);
        if text.trim().is_empty() && html.as_deref().map_or(true, |h| h.trim().is_empty()) {
            skipped_parse_fail += 1;
            continue;
        }
        // Never clobber a full existing body with a shorter re-parse.
        let existing_len = existing_body.as_deref().map(|b| b.len()).unwrap_or(0);
        if existing_len >= text.len() && existing_len >= 250 {
            continue;
        }
        cache::messages::update_body_by_id(conn, id, &text, html.as_deref())
            .map_err(|e| ApiError(format!("Reparse: Update id={id} fehlgeschlagen: {e}")))?;
        updated += 1;
    }

    Ok(Json(serde_json::json!({
        "updated": updated,
        "skipped_missing_archive": skipped_missing,
        "skipped_parse_failed": skipped_parse_fail,
    })))
}

/// `GET /api/v1/messages/{uid}/attachments?account_id=…`
pub async fn fetch_attachments(
    State(state): State<AppState>,
    Query(q): Query<MessageUidQuery>,
) -> ApiResult<Vec<crate::cache::attachments::CachedAttachment>> {
    let uid = q.uid;
    let db_guard = get_db(&state).map_err(|e| ApiError(e))?;
    let conn = db_guard.as_ref().ok_or(ApiError("Datenbank nicht initialisiert".into()))?;

    // Scoped by folder (uid is only unique per folder). Falls back to the
    // unscoped lookup when no folder is given, so legacy callers keep working.
    let message_id: i64 = resolve_message_id(conn, q.account_id, uid as i64, q.folder.as_deref())
        .ok_or_else(|| ApiError("Nachricht nicht gefunden".into()))?;

    crate::cache::attachments::get_attachments(conn, message_id)
        .map(Json)
        .map_err(|e| ApiError(e.to_string()))
}

/// Resolve the DB id of a message, optionally scoped to `folder`. When no
/// folder is given, prefer "Entwürfe" first (matching `fetch_message_body`),
/// then the lowest id — this keeps legacy unscoped callers deterministic.
fn resolve_message_id(conn: &rusqlite::Connection, account_id: u32, uid: i64, folder: Option<&str>) -> Option<i64> {
    match folder {
        Some(f) => conn.query_row(
            "SELECT m.id FROM messages m
             JOIN folders f ON m.folder_id = f.id
             WHERE m.account_id = ?1 AND m.uid = ?2 AND f.name = ?3
             LIMIT 1",
            rusqlite::params![account_id as i64, uid, f],
            |row| row.get(0),
        ).ok(),
        None => conn.query_row(
            "SELECT m.id FROM messages m
             JOIN folders f ON m.folder_id = f.id
             WHERE m.account_id = ?1 AND m.uid = ?2
             ORDER BY f.name = 'Entwürfe' ASC, m.id ASC
             LIMIT 1",
            rusqlite::params![account_id as i64, uid],
            |row| row.get(0),
        ).ok(),
    }
}

/// Inline base64 content for an attachment, preferring the dedup disk store,
/// then the legacy SQLite `content` column. Returns `None` when the content is
/// not materialized locally (the caller falls back to the on-demand endpoint).
fn attachment_inline_content(
    conn: &rusqlite::Connection,
    state: &crate::AppState,
    a: &crate::cache::attachments::CachedAttachment,
) -> Option<String> {
    if let Some(content) = a.content.as_ref().filter(|c| !c.is_empty()) {
        return Some(content.clone());
    }
    let rel: Option<String> = conn
        .query_row(
            "SELECT disk_path FROM message_attachments WHERE id = ?1",
            rusqlite::params![a.id],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    rel.and_then(|r| {
        let abs = state.data_root.join(&r);
        std::fs::read(&abs)
            .ok()
            .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
    })
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
        let message_id: i64 = resolve_message_id(conn, q.account_id, uid as i64, q.folder.as_deref())
            .ok_or_else(|| ApiError("Nachricht nicht gefunden".into()))?;
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
                "SELECT raw_path FROM messages m
                 JOIN folders f ON m.folder_id = f.id
                 WHERE m.account_id = ?1 AND m.uid = ?2
                   AND (?3 IS NULL OR f.name = ?3)
                 ORDER BY f.name = 'Entwürfe' ASC, m.id ASC
                 LIMIT 1",
                rusqlite::params![q.account_id as i64, uid as i64, q.folder],
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
        // the stable part_index (BODYSTRUCTURE/parse order) instead of the
        // historical COUNT-by-id trick.
        let found = if !filename.is_empty() {
            attachments.iter().find(|a| a.filename == filename)
        } else {
            let ordinal: Option<usize> = {
                let db_guard = get_db(&state).map_err(|e| ApiError(e))?;
                let conn = db_guard.as_ref().ok_or(ApiError("Datenbank nicht initialisiert".into()))?;
                let message_id = resolve_message_id(conn, q.account_id, uid as i64, q.folder.as_deref()).unwrap_or(0);
                conn.query_row(
                    "SELECT part_index FROM message_attachments WHERE message_id = ?1 AND id = ?2",
                    rusqlite::params![message_id, att_id as i64],
                    |r| r.get::<_, i64>(0),
                )
                .ok()
                .map(|pi| pi.max(0) as usize)
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
    let (folder_id, folder_name, local_only) = message_folder_info(&state, req.account_id, uid, req.source_folder.as_deref());
    with_db(&state, |conn| {
        cache::messages::mark_as_read_in_folder(conn, req.account_id as i64, uid as i64, folder_id)
            .map_err(|e| e.to_string())
    })?;
    // Local-only folders (e.g. "Mama und Papa") don't exist on the IMAP server:
    // skip the remote flag entirely — the DB update above is authoritative.
    if !local_only {
        let client = {
            let guard = state.imap_clients.read();
            guard.get(&req.account_id).cloned()
        };
        if let Some(client) = client {
            client.mark_flag(uid, "\\Seen", true, folder_name.clone()).await.map_err(|e| ApiError(e.to_string()))?;
        }
    }
    state
        .folder_cache
        .write()
        .invalidate(req.account_id as i64, folder_name.as_deref().unwrap_or("INBOX"));
    Ok(Json(()))
}

/// `POST /api/v1/messages/{uid}/unread` body: `{"account_id": N}`
pub async fn mark_as_unseen(
    State(state): State<AppState>,
    Json(req): Json<MessageActionRequest>,
) -> ApiResult<()> {
    let uid = req.uid;
    let (folder_id, folder_name, local_only) = message_folder_info(&state, req.account_id, uid, req.source_folder.as_deref());
    with_db(&state, |conn| {
        cache::messages::mark_as_unseen_in_folder(conn, req.account_id as i64, uid as i64, folder_id)
            .map_err(|e| e.to_string())
    })?;
    if !local_only {
        let client = {
            let guard = state.imap_clients.read();
            guard.get(&req.account_id).cloned()
        };
        if let Some(client) = client {
            client.mark_flag(uid, "\\Seen", false, folder_name.clone()).await.map_err(|e| ApiError(e.to_string()))?;
        }
    }
    state
        .folder_cache
        .write()
        .invalidate(req.account_id as i64, folder_name.as_deref().unwrap_or("INBOX"));
    Ok(Json(()))
}

/// `POST /api/v1/messages/read-batch` / `unread-batch` body:
/// `{"account_id": N, "uids": [...], "source_folder": "INBOX"}`
#[derive(Deserialize)]
pub struct BatchReadRequest {
    pub account_id: u32,
    pub uids: Vec<u32>,
    #[serde(default)]
    pub source_folder: Option<String>,
}

/// Shared implementation for marking many UIDs read/unread in one request.
async fn batch_set_read(state: &AppState, req: BatchReadRequest, read: bool) -> ApiResult<()> {
    if req.uids.is_empty() {
        return Ok(Json(()));
    }
    let account_id = req.account_id;
    let folder_name = req.source_folder.clone();
    // Resolve the folder once (all UIDs live in the same source folder).
    let (folder_id, local_only) = {
        let db_guard = get_db(state).map_err(|e| ApiError(e))?;
        let conn = db_guard.as_ref().ok_or(ApiError("Datenbank nicht initialisiert".into()))?;
        conn.query_row(
            "SELECT id, local_only FROM folders WHERE account_id = ?1 AND name = ?2",
            rusqlite::params![account_id as i64, folder_name.as_deref().unwrap_or("INBOX")],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i32>(1)? != 0)),
        )
        .map_err(|e| ApiError(e.to_string()))?
    };

    with_db(state, |conn| {
        for uid in &req.uids {
            let res = if read {
                cache::messages::mark_as_read_in_folder(conn, account_id as i64, *uid as i64, Some(folder_id))
            } else {
                cache::messages::mark_as_unseen_in_folder(conn, account_id as i64, *uid as i64, Some(folder_id))
            };
            if let Err(e) = res {
                tracing::warn!("batch_set_read: uid {} fehlgeschlagen: {}", uid, e);
            }
        }
        Ok::<_, String>(())
    })?;

    // Remote \Seen sync: run after the DB writes; the per-UID client call is
    // cheap and failures are only logged (the DB state is authoritative).
    // Local-only folders (e.g. "Mama und Papa", archived Sent) don't exist on
    // the IMAP server — skip the remote flag entirely, mirroring the singular
    // handlers.
    if !local_only {
        let client = {
            let guard = state.imap_clients.read();
            guard.get(&account_id).cloned()
        };
        if let Some(client) = client {
            for uid in &req.uids {
                if let Err(e) = client.mark_flag(*uid, "\\Seen", read, Some(folder_name.clone().unwrap_or_else(|| "INBOX".to_string()))).await {
                    tracing::warn!("batch_set_read: IMAP-Flag uid {} fehlgeschlagen: {}", uid, e);
                }
            }
        }
    }
    state
        .folder_cache
        .write()
        .invalidate(account_id as i64, folder_name.as_deref().unwrap_or("INBOX"));
    Ok(Json(()))
}

pub async fn mark_batch_as_read(
    State(state): State<AppState>,
    Json(req): Json<BatchReadRequest>,
) -> ApiResult<()> {
    batch_set_read(&state, req, true).await
}

pub async fn mark_batch_as_unseen(
    State(state): State<AppState>,
    Json(req): Json<BatchReadRequest>,
) -> ApiResult<()> {
    batch_set_read(&state, req, false).await
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
    let (folder_id, _folder_name, _local_only) = message_folder_info(&state, req.account_id, req.uid, Some(&req.folder_name));
    with_db(&state, |conn| {
        cache::messages::update_is_flagged_in_folder(
            conn, req.account_id as i64, req.uid as i64, folder_id, req.flagged,
        )
        .map_err(|e| e.to_string())
    })?;
    // IMAP flag sync runs in a helper so the TLS session never crosses await
    // points inside this handler.
    let account_id = req.account_id;
    let uid = req.uid;
    let flagged = req.flagged;
    let folder_name = req.folder_name.clone();
    set_imap_flag(&state, account_id, uid, flagged, Some(&folder_name)).await;
    state
        .folder_cache
        .write()
        .invalidate(account_id as i64, &folder_name);
    Ok(Json(()))
}

async fn set_imap_flag(state: &AppState, account_id: u32, uid: u32, flagged: bool, source_folder: Option<&str>) {
    let Some(client) = state.imap_clients.read().get(&account_id).cloned() else {
        return;
    };
    if client.is_connected().await {
        let (_folder_id, folder_name, local_only) = message_folder_info(state, account_id, uid, source_folder);
        if local_only {
            return;
        }
        // Folder-scoped mark: the IMAP session is shared with the sync
        // scheduler and other API calls, so the flag must be set on the
        // message's OWN folder — an unscoped mark_flag writes to whichever
        // folder the session happens to have selected.
        let _ = client
            .mark_flag(uid, "\\Flagged", flagged, folder_name)
            .await;
    }
}

/// Resolve folder identity for a message. UID is only unique per folder, so
/// the lookup is folder-agnostic but ordered like `fetch_message_body_with_folder`
/// (drafts first, then lowest id). Returns (folder_id, folder_name, local_only).
fn message_folder_info(
    state: &AppState,
    account_id: u32,
    uid: u32,
    folder: Option<&str>,
) -> (Option<i64>, Option<String>, bool) {
    let db_guard = state.cache_db.lock();
    let Some(conn) = db_guard.as_ref() else {
        return (None, None, false);
    };
    let sql_folder = match folder {
        Some(_f) => format!(
            "SELECT f.id, f.name, f.local_only
             FROM messages m
             JOIN folders f ON f.id = m.folder_id
             WHERE m.account_id = ?1 AND m.uid = ?2 AND f.name = ?3
               AND (m.flags NOT LIKE '%\\\\Deleted%' OR m.flags IS NULL)
             ORDER BY f.name = 'Entwürfe' ASC, m.id ASC
             LIMIT 1"
        ),
        None => format!(
            "SELECT f.id, f.name, f.local_only
             FROM messages m
             JOIN folders f ON f.id = m.folder_id
             WHERE m.account_id = ?1 AND m.uid = ?2
               AND (m.flags NOT LIKE '%\\\\Deleted%' OR m.flags IS NULL)
             ORDER BY f.name = 'Entwürfe' ASC, m.id ASC
             LIMIT 1"
        ),
    };
    let account_id_i64 = account_id as i64;
    let uid_i64 = uid as i64;
    let params: Vec<Box<dyn rusqlite::types::ToSql>> = match folder {
        Some(f) => vec![Box::new(account_id_i64), Box::new(uid_i64), Box::new(f.to_string())],
        None => vec![Box::new(account_id_i64), Box::new(uid_i64)],
    };
    let params_ref: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    conn.query_row(
        &sql_folder,
        rusqlite::params_from_iter(params_ref.iter()),
        |r| Ok((r.get::<_, i64>(0).ok(), r.get::<_, String>(1).ok(), r.get::<_, i64>(2).unwrap_or(0) != 0)),
    )
    .unwrap_or((None, None, false))
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

        {
            let mut cache = state.folder_cache.write();
            cache.invalidate(req.account_id as i64, &req.source_folder);
            cache.invalidate(req.target_account_id as i64, &req.target_folder);
        }
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

    {
        let mut cache = state.folder_cache.write();
        cache.invalidate(req.account_id as i64, &req.source_folder);
        cache.invalidate(req.target_account_id as i64, &req.target_folder);
    }

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

    let res = if move_to_trash && sync_mode == "archive" {
        delete_message_archive_trash(
            &state, account_id, uid, account_id_i64, uid_i64, req.source_folder.clone(),
        )
        .await
    } else if move_to_trash {
        delete_message_trash_mode(
            &state, account_id, uid, account_id_i64, uid_i64, req.source_folder.clone(),
        )
        .await
    } else {
        delete_message_permanent_delete(
            &state, account_id, uid, account_id_i64, uid_i64, req.source_folder.clone(),
        )
        .await
    };
    // The trash helpers may move rows between folders (or drop them) — the
    // affected folder set isn't worth tracking: bust the whole account.
    if res.is_ok() {
        state.folder_cache.write().invalidate_account(account_id_i64);
    }
    res
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
    /// Folder the message was shown in. UIDs are only unique per folder, so
    /// the frontend passes the source folder to disambiguate the row.
    #[serde(default)]
    pub source_folder: Option<String>,
}

/// Delete by moving the message to the Trash folder instead of permanent delete.
async fn delete_message_trash_mode(
    state: &AppState,
    account_id: u32,
    uid: u32,
    account_id_i64: i64,
    uid_i64: i64,
    source_folder_hint: Option<String>,
) -> ApiResult<()> {
    // Step 1: Look up the source folder (uid is only unique per folder — use
    // the hint from the frontend to pick the right row when uids repeat).
    let folder: Option<String> = with_db(state, |conn| {
        let sql = match source_folder_hint {
            Some(_) => format!(
                "SELECT f.name FROM messages m \
                 JOIN folders f ON m.folder_id = f.id \
                 WHERE m.account_id = ?1 AND m.uid = ?2 AND f.name = ?3 LIMIT 1"
            ),
            None => format!(
                "SELECT f.name FROM messages m \
                 JOIN folders f ON m.folder_id = f.id \
                 WHERE m.account_id = ?1 AND m.uid = ?2 LIMIT 1"
            ),
        };
        Ok(conn
            .query_row(&sql, rusqlite::params![account_id_i64, uid_i64, source_folder_hint], |row| row.get(0))
            .ok())
    })
    .unwrap_or(None);

    let source_folder = match folder {
        Some(ref f) if f != "Trash" => f.clone(),
        Some(_) => {
            return delete_message_permanent_delete(
                state,
                account_id,
                uid,
                account_id_i64,
                uid_i64,
                source_folder_hint,
            )
            .await;
        }
        None => {
            // No folder hint or message not found. If the frontend DID send a
            // hint but no row matched, refuse instead of hitting every row
            // with this uid (uid is only unique per folder).
            if source_folder_hint.is_some() {
                return Err(ApiError(format!(
                    "Nachricht uid {} in Ordner nicht gefunden (Konto {})",
                    uid, account_id
                )));
            }
            let _ = with_db(state, |conn| {
                cache::messages::delete_message(conn, account_id_i64, uid_i64)
                    .map_err(|e| e.to_string())
            });
            return Ok(Json(()));
        }
    };

    // Step 2: Get IMAP client
    let client = match state.imap_clients.read().get(&account_id).cloned() {
        None => {
            // No client: local fallback move into Trash, scoped to the
            // source folder (uid is only unique per folder).
            let _ = with_db(state, |conn| {
                cache::messages::update_folder_from(conn, account_id_i64, uid_i64, &source_folder, "Trash")
                    .map_err(|e| e.to_string())
            });
            return Ok(Json(()));
        }
        Some(c) => c,
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

    // Step 6: Update folder in cache (scoped to the source folder)
    with_db(state, |conn| {
        cache::messages::update_folder_from(conn, account_id_i64, uid_i64, &source_folder, "Trash")
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
    source_folder_hint: Option<String>,
) -> ApiResult<()> {
    // UID is only unique per folder — prefer the frontend's folder hint so
    // repeated uids (per-folder uid counters) resolve to the right row.
    let source_folder: Option<String> = with_db(state, |conn| {
        let sql = match source_folder_hint {
            Some(_) => format!(
                "SELECT f.name FROM messages m \
                 JOIN folders f ON m.folder_id = f.id \
                 WHERE m.account_id = ?1 AND m.uid = ?2 AND f.name = ?3 LIMIT 1"
            ),
            None => format!(
                "SELECT f.name FROM messages m \
                 JOIN folders f ON m.folder_id = f.id \
                 WHERE m.account_id = ?1 AND m.uid = ?2 LIMIT 1"
            ),
        };
        let params: Vec<&dyn rusqlite::ToSql> = match source_folder_hint {
            Some(_) => vec![&account_id_i64, &uid_i64, &source_folder_hint],
            None => vec![&account_id_i64, &uid_i64],
        };
        Ok(conn
            .query_row(&sql, params.as_slice(), |row| row.get(0))
            .ok())
    })
    .unwrap_or(None);

    let message_id: Option<i64> = with_db(state, |conn| {
        let sql = match source_folder_hint {
            Some(_) => format!(
                "SELECT m.id FROM messages m \
                 JOIN folders f ON m.folder_id = f.id \
                 WHERE m.account_id = ?1 AND m.uid = ?2 AND f.name = ?3 LIMIT 1"
            ),
            None => format!(
                "SELECT id FROM messages WHERE account_id = ?1 AND uid = ?2 LIMIT 1"
            ),
        };
        let params: Vec<&dyn rusqlite::ToSql> = match source_folder_hint {
            Some(_) => vec![&account_id_i64, &uid_i64, &source_folder_hint],
            None => vec![&account_id_i64, &uid_i64],
        };
        Ok(conn
            .query_row(&sql, params.as_slice(), |row| row.get(0))
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
            None => {
                if source_folder_hint.is_some() {
                    Err(format!(
                        "Nachricht uid {} in Ordner nicht gefunden (Konto {})",
                        uid, account_id
                    ))
                } else {
                    cache::messages::update_folder(conn, account_id_i64, uid_i64, "Trash")
                        .map_err(|e| e.to_string())
                }
            }
        }
    })?;

    // 2. Enqueue provider deletion (verified by the worker before hard delete).
    match (message_id, source_folder.clone()) {
        (Some(mid), Some(f)) => {
            match with_db(state, |conn| {
                crate::cache::delete_queue::enqueue(conn, mid, account_id_i64, uid_i64, &f, "delete")
                    .map_err(|e| e.to_string())
            }) {
                Ok(_) => {
                    tracing::info!(
                        "delete_message (archive): uid {} (Konto {}) → lokaler Papierkorb, Provider-Löschung in Queue (row {})",
                        uid, account_id, mid
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "delete_message (archive): uid {} (Konto {}) lokal in Papierkorb verschoben, ABER enqueue FEHLGESCHLAGEN: {} — Mail bleibt auf dem Provider",
                        uid, account_id, e
                    );
                }
            }
        }
        _ => {
            tracing::error!(
                "delete_message (archive): uid {} (Konto {}) — message_id oder source_folder nicht auflösbar (mid={:?}, folder={:?}), Provider-Löschung NICHT in Queue",
                uid, account_id, message_id, source_folder
            );
        }
    }
    Ok(Json(()))
}

/// Permanent delete helper.
async fn delete_message_permanent_delete(
    state: &AppState,
    account_id: u32,
    uid: u32,
    account_id_i64: i64,
    uid_i64: i64,
    source_folder_hint: Option<String>,
) -> ApiResult<()> {
    // UID is only unique per folder — use the frontend hint to pick the row.
    let folder: Option<String> = with_db(state, |conn| {
        let sql = match source_folder_hint {
            Some(_) => format!(
                "SELECT f.name FROM messages m \
                 JOIN folders f ON m.folder_id = f.id \
                 WHERE m.account_id = ?1 AND m.uid = ?2 AND f.name = ?3 LIMIT 1"
            ),
            None => format!(
                "SELECT f.name FROM messages m \
                 JOIN folders f ON m.folder_id = f.id \
                 WHERE m.account_id = ?1 AND m.uid = ?2 LIMIT 1"
            ),
        };
        let params: Vec<&dyn rusqlite::ToSql> = match source_folder_hint {
            Some(_) => vec![&account_id_i64, &uid_i64, &source_folder_hint],
            None => vec![&account_id_i64, &uid_i64],
        };
        Ok(conn
            .query_row(&sql, params.as_slice(), |row| row.get(0))
            .ok())
    })
    .unwrap_or(None);

    let message_id: Option<i64> = with_db(state, |conn| {
        let sql = match source_folder_hint {
            Some(_) => format!(
                "SELECT m.id FROM messages m \
                 JOIN folders f ON m.folder_id = f.id \
                 WHERE m.account_id = ?1 AND m.uid = ?2 AND f.name = ?3 LIMIT 1"
            ),
            None => format!(
                "SELECT id FROM messages WHERE account_id = ?1 AND uid = ?2 LIMIT 1"
            ),
        };
        let params: Vec<&dyn rusqlite::ToSql> = match source_folder_hint {
            Some(_) => vec![&account_id_i64, &uid_i64, &source_folder_hint],
            None => vec![&account_id_i64, &uid_i64],
        };
        Ok(conn
            .query_row(&sql, params.as_slice(), |row| row.get(0))
            .ok())
    })
    .unwrap_or(None);

    // 1. Remove the local index row (user intent; EML archive stays untouched).
    //    Scoped to the source folder when known — uid is only unique per folder.
    let cache_result = with_db(state, |conn| match folder.as_deref() {
        Some(f) => cache::messages::delete_message_from(conn, account_id_i64, uid_i64, f)
            .map_err(|e| e.to_string()),
        None => {
            if source_folder_hint.is_some() {
                Err(format!(
                    "Nachricht uid {} in Ordner nicht gefunden (Konto {})",
                    uid, account_id
                ))
            } else {
                cache::messages::delete_message(conn, account_id_i64, uid_i64)
                    .map_err(|e| e.to_string())
            }
        }
    });
    if let Err(e) = cache_result {
        return Err(ApiError(format!("Cache delete fehlgeschlagen: {}", e)));
    }

    // 2. Enqueue provider deletion — verified by the worker before touching IMAP.
    match (message_id, folder.clone()) {
        (Some(mid), Some(f)) => {
            match with_db(state, |conn| {
                crate::cache::delete_queue::enqueue(conn, mid, account_id_i64, uid_i64, &f, "delete")
                    .map_err(|e| e.to_string())
            }) {
                Ok(_) => {
                    tracing::info!(
                        "delete_message: uid {} (Konto {}) lokal entfernt, Provider-Löschung in Queue (row {})",
                        uid, account_id, mid
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "delete_message: uid {} (Konto {}) lokal entfernt, ABER enqueue FEHLGESCHLAGEN: {} — Mail bleibt auf dem Provider",
                        uid, account_id, e
                    );
                }
            }
        }
        _ => {
            tracing::error!(
                "delete_message: uid {} (Konto {}) — message_id oder folder nicht auflösbar (mid={:?}, folder={:?}), Provider-Löschung NICHT in Queue",
                uid, account_id, message_id, folder
            );
        }
    }

    Ok(Json(()))
}

#[cfg(test)]
mod messages_query_tests {
    use super::*;

    #[test]
    fn fetch_messages_query_deserializes_list_only_1() {
        let q: FetchMessagesQuery = serde_urlencoded::from_str(
            "account_id=1&list_only=1",
        )
        .expect("query must deserialize");
        assert_eq!(q.account_id, 1);
        assert_eq!(q.list_only, Some(true));
    }

    #[test]
    fn fetch_messages_query_accepts_plain_true() {
        let q: FetchMessagesQuery = serde_urlencoded::from_str(
            "account_id=2&list_only=true",
        )
        .expect("query must deserialize");
        assert_eq!(q.list_only, Some(true));
    }

    #[test]
    fn fetch_messages_query_accepts_words_and_numeric_false() {
        for raw in ["0", "false", "off", "no"] {
            let q: FetchMessagesQuery = serde_urlencoded::from_str(&format!(
                "account_id=2&list_only={raw}",
            ))
            .unwrap_or_else(|e| panic!("{raw} must deserialize: {e}"));
            assert_eq!(q.list_only, Some(false), "raw={raw}");
        }
    }

    #[test]
    fn fetch_messages_query_defaults_list_only_absent() {
        let q: FetchMessagesQuery = serde_urlencoded::from_str(
            "account_id=3",
        )
        .expect("query must deserialize");
        assert_eq!(q.list_only, None);
    }

    #[test]
    fn fetch_messages_query_rejects_garbage() {
        assert!(serde_urlencoded::from_str::<FetchMessagesQuery>(
            "account_id=1&list_only=banana",
        )
        .is_err());
    }

    #[test]
    fn fetch_messages_query_accepts_empty_list_only() {
        let q: FetchMessagesQuery = serde_urlencoded::from_str(
            "account_id=1&list_only=",
        )
        .expect("empty value must deserialize");
        assert_eq!(q.list_only, None);
    }

    #[test]
    fn decode_body_keeps_local_draft_text_verbatim() {
        // A local (unsynced) draft containing a QP-looking pattern must not be
        // re-encoded — fixes "saved draft text differs from what was written".
        let draft_text = "Preis: 1=20 EUR und a=3Db als Literal";
        assert_eq!(decode_body_text(draft_text, false), draft_text);
        assert_eq!(decode_body_text("", false), "");
    }

    #[test]
    fn decode_body_decodes_only_synced_rows() {
        assert_eq!(decode_body_text("H=C3=A4llo", true), "Hällo");
        assert_eq!(decode_body_text("H=C3=A4llo", false), "H=C3=A4llo");
        assert_eq!(decode_body_text("plain text", true), "plain text");
    }
}
