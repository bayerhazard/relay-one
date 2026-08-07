//! SMTP send + draft endpoints.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

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

    // Append a copy to the IMAP Sent folder (non-critical)
    let mut sent_copy_saved = false;
    let imap_client_opt = state.imap_clients.read().get(&req.account_id).cloned();
    if let Some(imap_client) = imap_client_opt {
        if !imap_client.is_connected().await {
            let _ = imap_client.connect().await;
        }
        if imap_client.is_connected().await {
            match imap_client.list_folders_detailed().await {
                Ok(folders) => {
                    if let Some(sent_folder) = find_special_folder(&folders, SpecialFolder::Sent) {
                        match imap_client.append_message(&sent_folder, &raw_bytes, None).await {
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

    Ok(Json(SendMessageResponse { message_id, sent_copy_saved }))
}
