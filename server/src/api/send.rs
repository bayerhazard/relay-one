//! SMTP send + draft endpoints.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use sha2::Digest as _;

use crate::db::with_db;
use crate::imap::client::find_special_folder;
use crate::imap::types::SpecialFolder;
use crate::smtp::client::EmailAttachment;
use crate::tone::profile::ProfileManager;
use crate::AppState;

use super::{ApiError, ApiResult};

#[derive(Serialize)]
pub struct SendMessageResponse {
    pub message_id: String,
    pub sent_copy_saved: bool,
}

#[derive(Deserialize)]
pub struct SendMessageRequest {
    pub account_id: u32,
    pub to: Vec<String>,
    pub cc: Option<Vec<String>>,
    pub bcc: Option<Vec<String>>,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub recipient_email: Option<String>,
    pub attachments: Option<Vec<EmailAttachment>>,
    pub ai_draft: Option<String>,
}

/// Draft save request (local storage in the "Entwürfe" folder).
#[derive(Deserialize)]
pub struct SaveDraftRequest {
    pub account_id: u32,
    /// Optional existing draft uid — when present and the draft still exists,
    /// the draft is updated in place instead of inserting a new row.
    pub uid: Option<u32>,
    pub to: Vec<String>,
    pub cc: Option<Vec<String>>,
    pub bcc: Option<Vec<String>>,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
    /// Attachments to persist with the draft. `Some([])` clears all previous
    /// attachments; `None` leaves them untouched (legacy clients).
    pub attachments: Option<Vec<EmailAttachment>>,
}

/// Max total size (base64 length) of attachments in a saved draft.
/// Chosen to match typical mail-provider limits; larger saves are rejected
/// with a clear error instead of silently dropping the attachments.
pub const DRAFT_ATTACHMENT_CAP_BYTES: usize = 25 * 1024 * 1024;

/// Draft discard request.
#[derive(Deserialize)]
pub struct DiscardDraftRequest {
    pub account_id: u32,
    pub uid: u32,
}

/// `POST /api/v1/draft/save` — persist a draft in the LOCAL "Entwürfe" folder
/// (extended mode) or the provider Drafts folder (mirror mode).
pub async fn save_draft(
    State(state): State<AppState>,
    Json(req): Json<SaveDraftRequest>,
) -> ApiResult<serde_json::Value> {
    let account_id = req.account_id as i64;

    // Ensure the local "Entwürfe" folder exists.
    with_db(&state, |conn| {
        crate::cache::messages::create_local_folder(conn, account_id, "Entwürfe")
            .map_err(|e| e.to_string())
    })?;

    let drafts_folder_id: i64 = with_db(&state, |conn| {
        conn.query_row(
            "SELECT id FROM folders WHERE account_id = ?1 AND name = 'Entwürfe'",
            rusqlite::params![account_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())
    })?;

    let now = chrono::Utc::now().to_rfc3339();
    let uid = with_db(&state, |conn| {
        // If the client sent an existing draft uid that still exists in the
        // "Entwürfe" folder, update it in place (fixes duplicate/stale drafts:
        // every "save" used to INSERT a brand-new row).
        if let Some(existing) = req.uid {
            let exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE account_id = ?1 AND uid = ?2 AND folder_id = ?3)",
                rusqlite::params![account_id, existing as i64, drafts_folder_id],
                |r| r.get(0),
            ).unwrap_or(false);
            if exists {
                conn.execute(
                    "UPDATE messages SET subject = ?3, to_addr = ?4, date = ?5, body_text = ?6, body_html = ?7, synced = 0
                     WHERE account_id = ?1 AND uid = ?2 AND folder_id = ?8",
                    rusqlite::params![
                        account_id,
                        existing as i64,
                        req.subject,
                        req.to.join(", "),
                        now,
                        req.body_text,
                        req.body_html,
                        drafts_folder_id,
                    ],
                )
                .map_err(|e| e.to_string())?;
                return Ok(existing as i64);
            }
        }

        // New draft — allocate the next uid and insert.
        let uid: i64 = conn.query_row(
            "SELECT COALESCE(MAX(uid), 0) + 1 FROM messages WHERE account_id = ?1 AND folder_id = ?2",
            rusqlite::params![account_id, drafts_folder_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO messages (account_id, folder_id, uid, subject, from_addr, to_addr, date, body_text, body_html, synced)
             VALUES (?1, ?2, ?3, ?4, '', ?5, ?6, ?7, ?8, 0)",
            rusqlite::params![
                account_id,
                drafts_folder_id,
                uid,
                req.subject,
                req.to.join(", "),
                now,
                req.body_text,
                req.body_html,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(uid)
    })?;

    // Persist attachments: reconcile metadata with a stable part_index and
    // store the content deduplicated under <data>/attachments/<sha256>. The
    // draft's message row is the sole reference — content lives on disk.
    if let Some(atts) = &req.attachments {
        let total_b64: usize = atts.iter().map(|a| a.content.len()).sum();
        if total_b64 > DRAFT_ATTACHMENT_CAP_BYTES {
            return Err(ApiError(format!(
                "Anhänge zu groß für einen Entwurf (max. {} MB).",
                DRAFT_ATTACHMENT_CAP_BYTES / (1024 * 1024)
            )));
        }
        let message_id: i64 = with_db(&state, |conn| {
            conn.query_row(
                "SELECT id FROM messages WHERE account_id = ?1 AND uid = ?2 AND folder_id = ?3",
                rusqlite::params![account_id, uid as i64, drafts_folder_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
        })?;

        let metas: Vec<crate::imap::types::AttachmentMeta> = atts
            .iter()
            .map(|a| crate::imap::types::AttachmentMeta {
                filename: a.filename.clone(),
                content_type: a.content_type.clone(),
                size: a.size,
            })
            .collect();
        with_db(&state, |conn| {
            crate::cache::attachments::reconcile_attachments(conn, message_id, &metas)
                .map_err(|e| e.to_string())
        })?;
        // Persist content deduplicated to disk for each attachment, matched by
        // its stable part_index.
        with_db(&state, |conn| {
            for (idx, att) in atts.iter().enumerate() {
                let aid: i64 = conn
                    .query_row(
                        "SELECT id FROM message_attachments WHERE message_id = ?1 AND part_index = ?2",
                        rusqlite::params![message_id, idx as i64],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                if aid > 0 {
                    let _ = crate::cache::attachments::cache_content_dedup(
                        conn, aid, &att.content, &state.data_root,
                    );
                }
            }
            Ok::<(), String>(())
        })?;
    }

    Ok(Json(serde_json::json!({ "uid": uid })))
}

/// `POST /api/v1/draft/discard` — remove a local draft.
pub async fn discard_draft(
    State(state): State<AppState>,
    Json(req): Json<DiscardDraftRequest>,
) -> ApiResult<serde_json::Value> {
    let account_id = req.account_id as i64;
    let uid = req.uid as i64;
    with_db(&state, |conn| {
        conn.execute(
            "DELETE FROM messages
             WHERE account_id = ?1 AND uid = ?2
               AND folder_id = (SELECT id FROM folders WHERE account_id = ?1 AND name = 'Entwürfe')",
            rusqlite::params![account_id, uid],
        )
        .map_err(|e| e.to_string())
    })?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// `POST /api/v1/send`
pub async fn send_message(
    State(state): State<AppState>,
    Json(req): Json<SendMessageRequest>,
) -> ApiResult<SendMessageResponse> {
    let smtp_client = state
        .smtp_clients
        .read()
        .get(&req.account_id)
        .cloned()
        .ok_or(ApiError("SMTP-Client nicht gefunden".into()))?;

    let to_parsed: Vec<(&str, &str)> = req.to.iter().map(|s| (s.as_str(), "")).collect();
    let cc_default: Vec<String> = vec![];
    let cc_values = req.cc.as_ref().unwrap_or(&cc_default);
    let cc_parsed: Vec<(&str, &str)> = cc_values.iter().map(|s| (s.as_str(), "")).collect();
    let bcc_default: Vec<String> = vec![];
    let bcc_values = req.bcc.as_ref().unwrap_or(&bcc_default);
    let bcc_parsed: Vec<(&str, &str)> = bcc_values.iter().map(|s| (s.as_str(), "")).collect();
    let attachments_vec = req.attachments.unwrap_or_default();

    // Send via SMTP and capture raw RFC822 bytes
    let (message_id, raw_bytes) = smtp_client
        .send(
            to_parsed,
            cc_parsed,
            bcc_parsed,
            &req.subject,
            &req.body_text,
            req.body_html.as_deref(),
            req.in_reply_to.as_deref(),
            req.references.as_deref(),
            &attachments_vec,
        )
        .await
        .map_err(|e| ApiError(e.to_string()))?;

    // Ton-Profil aktualisieren (lernen aus gesendeter Mail)
    if let Some(ref email) = req.recipient_email {
        let _ = with_db(&state, |conn| {
            ProfileManager::update_profile_from_mail(
                conn,
                req.account_id as i64,
                email,
                &req.body_text,
                req.to.first().map(|s| s.as_str()),
            )
            .map_err(|e| e.to_string())
        });
    }

    // Save snippet for each recipient (for few-shot learning)
    let now = chrono::Utc::now().to_rfc3339();
    let all_recipients: Vec<String> = req
        .to
        .iter()
        .chain(req.cc.as_ref().unwrap_or(&cc_default).iter())
        .chain(req.bcc.as_ref().unwrap_or(&bcc_default).iter())
        .cloned()
        .collect();
    if !all_recipients.is_empty() {
        let _ = with_db(&state, |conn| {
            let topic = crate::cache::topic::detect_topic(&req.subject, &req.body_text);
            for recipient in &all_recipients {
                let hash = ProfileManager::hash_email(recipient);
                let _ = crate::cache::snippets::add_snippet(
                    conn,
                    req.account_id as i64,
                    &hash,
                    &topic,
                    &req.body_text,
                    &now,
                );
            }
            Ok(())
        });
    }

    // Queue diff for learning loop (AI draft vs user's final text)
    if let Some(ref draft) = req.ai_draft {
        if !draft.is_empty() && req.to.len() == 1 {
            let _ = with_db(&state, |conn| {
                let topic = crate::cache::topic::detect_topic(&req.subject, &req.body_text);
                let hash = ProfileManager::hash_email(&req.to[0]);
                let queued = crate::cache::learning::queue_diff(
                    conn,
                    req.account_id as i64,
                    &hash,
                    &topic,
                    draft,
                    &req.body_text,
                    0.05,
                )
                .map_err(|e| e.to_string())?;
                if queued {
                    tracing::info!("Diff für {} gespeichert (edit_distance > 5%)", req.to[0]);
                }
                Ok(())
            });
        }
    }

    // Append a copy to the Sent folder.
    //   mirror mode: IMAP APPEND into the provider's Sent folder.
    //   archive mode: store locally in the "Gesendet" folder (EML + index).
    let mut sent_copy_saved = false;
    let sync_mode: String = with_db(&state, |conn| {
        Ok(conn
            .query_row(
                "SELECT sync_mode FROM accounts WHERE id = ?1",
                rusqlite::params![req.account_id as i64],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "mirror".to_string()))
    })
    .unwrap_or_else(|_| "mirror".to_string());

    if sync_mode == "archive" {
        // Local "Gesendet" folder + EML archive + index row.
        let account_id = req.account_id as i64;
        let subject = req.subject.clone();
        let to_addr = req.to.join(", ");
        let date = chrono::Utc::now().to_rfc3339();
        let message_row_id: Option<i64> = with_db(&state, |conn| {
            crate::cache::messages::create_local_folder(conn, account_id, "Gesendet").map_err(|e| e.to_string())?;
            let uid: i64 = conn.query_row(
                "SELECT COALESCE(MAX(uid), 0) + 1 FROM messages WHERE account_id = ?1 AND folder_id = (SELECT id FROM folders WHERE account_id = ?1 AND name = 'Gesendet')",
                rusqlite::params![account_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO messages (account_id, folder_id, uid, subject, from_addr, to_addr, date, body_text, body_html, synced, is_read)
                 VALUES (?1, (SELECT id FROM folders WHERE account_id = ?1 AND name = 'Gesendet'), ?2, ?3, '', ?4, ?5, ?6, ?7, 0, 1)",
                rusqlite::params![account_id, uid, subject, to_addr, date, req.body_text, req.body_html],
            )
            .map_err(|e| e.to_string())?;
            Ok::<i64, String>(conn.last_insert_rowid())
        })
        .ok();
        // Persist attachment metadata + content for the sent copy so the
        // "Gesendet" list shows attachments (mirror mode gets this via the
        // provider sync; the local archive copy has no BODYSTRUCTURE pass).
        if let Some(message_row_id) = message_row_id {
            let attachments = crate::imap::client::parse_message_attachments(&raw_bytes);
            if !attachments.is_empty() {
                let meta: Vec<crate::imap::types::AttachmentMeta> = attachments
                    .iter()
                    .map(|a| crate::imap::types::AttachmentMeta {
                        filename: a.filename.clone(),
                        content_type: a.content_type.clone(),
                        size: a.size,
                    })
                    .collect();
                let _ = with_db(&state, |conn| {
                    crate::cache::attachments::reconcile_attachments(conn, message_row_id, &meta)
                        .map_err(|e| e.to_string())?;
                    conn.execute(
                        "UPDATE messages SET has_attachments = 1 WHERE id = ?1",
                        rusqlite::params![message_row_id],
                    )
                    .map_err(|e| e.to_string())?;
                    // Cache base64 content so attachments are viewable
                    // without an IMAP round-trip.
                    for (idx, att) in attachments.iter().enumerate() {
                        let att_id: Option<i64> = conn
                            .query_row(
                                "SELECT id FROM message_attachments WHERE message_id = ?1 AND part_index = ?2",
                                rusqlite::params![message_row_id, idx as i64],
                                |r| r.get(0),
                            )
                            .ok();
                        if let Some(att_id) = att_id {
                            let _ = crate::cache::attachments::cache_content_dedup(
                                conn, att_id, &att.content, &state.data_root,
                            );
                        }
                    }
                    Ok(())
                });
                tracing::info!(
                    "send_message (archive): {} Anhänge für 'Gesendet'-Kopie (message row {}) persistiert",
                    attachments.len(),
                    message_row_id
                );
            }
        }
        // Persist the raw RFC822 copy as an EML archive file (Concept §3.1).
        // The uid is stable per message_id, so re-sending the same mail does
        // not create duplicates.
        let eml_uid = {
            let mut h = sha2::Sha256::new();
            h.update(message_id.as_bytes());
            let d = h.finalize();
            u32::from_le_bytes([d[0], d[1], d[2], d[3]])
        };
        let _ = crate::cache::archive::write_eml(
            &state.data_root,
            account_id,
            eml_uid,
            Some(&chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()),
            Some(&message_id),
            &raw_bytes,
        );
        sent_copy_saved = true;
        tracing::info!("send_message (archive): gesendete Mail lokal in 'Gesendet' abgelegt");
    } else {
        let imap_client_opt = state.imap_clients.read().get(&req.account_id).cloned();
        if let Some(imap_client) = imap_client_opt {
            if !imap_client.is_connected().await {
                let _ = imap_client.connect().await;
            }
            if imap_client.is_connected().await {
                match imap_client.list_folders_detailed().await {
                    Ok(folders) => {
                        if let Some(sent_folder) = find_special_folder(&folders, SpecialFolder::Sent) {
                            // APPEND with \Seen so freshly sent messages are
                            // not "unread" on the server and stay in sync with
                            // the local Sent list.
                            match imap_client.append_message(&sent_folder, &raw_bytes, Some(&["\\Seen"])).await {
                                Ok(()) => {
                                    sent_copy_saved = true;
                                    tracing::info!("send_message: Kopie nach '{}' gespeichert", sent_folder);
                                }
                                Err(e) => tracing::warn!(
                                    "send_message: APPEND nach '{}' fehlgeschlagen: {}",
                                    sent_folder, e
                                ),
                            }
                        } else {
                            tracing::warn!("send_message: Kein Sent-Ordner auf dem Server gefunden");
                        }
                    }
                    Err(e) => tracing::warn!("send_message: list_folders fehlgeschlagen: {}", e),
                }
            } else {
                tracing::warn!("send_message: IMAP nicht verbunden, überspringe Sent-Kopie");
            }
        }
    }

    Ok(Json(SendMessageResponse { message_id, sent_copy_saved }))
}
