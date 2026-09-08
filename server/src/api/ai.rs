//! AI endpoints: reply generation, summarization, drafting, formatting,
//! priority/fraud detection, tone profiles, recipient/subject suggestions.
//!
//! Ported from the Tauri-era `ai_*` commands in `ipc.rs`. Business logic is
//! identical; only the transport changed from Tauri IPC to axum handlers.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{sse::Event, sse::KeepAlive, IntoResponse, Response, Sse};
use axum::Json;
use serde::{Deserialize, Serialize};

use std::convert::Infallible;
use std::time::Duration;

use crate::ai::agent::{self, AgentEvent, AgentRequest};
use crate::ai::audit;
use crate::ai::language::detect_language;
use crate::ai::plaene;
use crate::ai::prompts::{self, EmailMode};
use crate::ai::sessions;
use crate::cache;
use crate::db::{get_db, with_db};
use crate::security::{fraud, pii, priority};
use crate::sync::queue::{SyncTask, SyncTaskType};
use crate::tone::analyzer::analyze_mail;
use crate::tone::intent::parse_intent;
use crate::tone::profile::{ContactProfile, ProfileManager};
use crate::AppState;

use super::{ApiError, ApiResult};

/// Tone sliders (1–9). Missing values fall back to profile/chain signals.
#[derive(Deserialize)]
pub struct ToneSettings {
    pub freundlich: Option<u8>,
    pub professionell: Option<u8>,
    pub laenge: Option<u8>,
}

#[derive(Deserialize)]
pub struct GenerateReplyRequest {
    pub account_id: u32,
    pub mail_chain: Vec<String>,
    pub user_input: String,
    pub recipient_email: String,
    pub tone: Option<ToneSettings>,
    pub sender_name: Option<String>,
    pub subject: Option<String>,
    pub recipient_name: Option<String>,
}

/// `POST /api/v1/ai/generate-reply` — draft a reply to a mail chain.
pub async fn ai_generate_reply(
    State(state): State<AppState>,
    Json(req): Json<GenerateReplyRequest>,
) -> ApiResult<String> {
    let GenerateReplyRequest {
        account_id,
        mail_chain,
        user_input,
        recipient_email,
        tone,
        sender_name,
        subject,
        recipient_name,
    } = req;

    let tone = tone.unwrap_or(ToneSettings {
        freundlich: Some(5),
        professionell: Some(5),
        laenge: Some(5),
    });

    let (form, friend, len) = {
        let db_guard = get_db(&state).map_err(ApiError)?;
        let conn = db_guard
            .as_ref()
            .ok_or_else(|| ApiError("Datenbank nicht initialisiert".to_string()))?;

        // Mailchain analysieren
        let chain_signals: Vec<_> = mail_chain.iter().map(|m| analyze_mail(m)).collect();

        // Profil laden
        let profile = ProfileManager::get_profile(conn, account_id as i64, &recipient_email)
            .map_err(|e| e.to_string())?;

        // Merge: Profil > Chain > User-Override
        let mut formality = profile.formality_score;
        let mut friendliness = profile.friendliness_score;

        for (i, signal) in chain_signals.iter().enumerate() {
            let weight = (chain_signals.len() - i) as f32 / chain_signals.len() as f32;
            formality = formality * 0.7 + signal.formality * 0.3 * weight;
            friendliness = friendliness * 0.7 + signal.friendliness * 0.3 * weight;
        }

        let form = tone
            .professionell
            .unwrap_or((formality * 9.0 + 1.0).round() as u8);
        let friend = tone
            .freundlich
            .unwrap_or((friendliness * 9.0 + 1.0).round() as u8);
        let len = tone.laenge.unwrap_or(5);

        (form, friend, len)
    };

    let masked_chain: Vec<String> = mail_chain
        .iter()
        .map(|m| pii::mask_pii(m))
        .collect();
    let masked_user_input = pii::mask_pii(&user_input);

    // Load few-shot snippets and style fingerprint for this recipient
    let email_hash = ProfileManager::hash_email(&recipient_email);
    let snippets: Vec<String> = if let Ok(db_guard) = get_db(&state) {
        if let Some(conn) = db_guard.as_ref() {
            let topic = cache::topic::detect_topic(subject.as_deref().unwrap_or(""), &user_input);
            match cache::snippets::get_snippets(conn, account_id as i64, &email_hash, &topic, 3) {
                Ok(s) if !s.is_empty() => s,
                _ => cache::snippets::get_general_snippets(conn, account_id as i64, &email_hash, 3)
                    .unwrap_or_default(),
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let style_fingerprint: Option<String> = if !recipient_email.is_empty() {
        with_db(&state, |conn| {
            let hash = ProfileManager::hash_email(&recipient_email);
            match cache::fingerprint::get_fingerprint(conn, account_id as i64, &hash) {
                Ok(Some(fp)) => Ok(Some(fp.fingerprint)),
                _ => Ok(None),
            }
        })
        .ok()
        .flatten()
    } else {
        None
    };

    let mut context_additions = String::new();
    if !snippets.is_empty() {
        let examples: Vec<String> = snippets
            .iter()
            .enumerate()
            .map(|(i, s)| format!("Beispiel {}:\n{}", i + 1, s))
            .collect();
        context_additions.push_str(&format!(
            "\n\n=== BEISPIELE AUS DEINEM SCHREIBSTIL ===\n{}\nOrientiere dich an diesem Stil.",
            examples.join("\n\n")
        ));
    }
    if let Some(fp) = &style_fingerprint {
        if !fp.is_empty() {
            context_additions.push_str(&format!(
                "\n\n=== DEIN PERSOENLICHER SCHREIBSTIL ===\n{}\nBeachte diese Regeln.",
                fp
            ));
        }
    }

    let chain_text = format!(
        "Freitext des Nutzers:\n{}\n\n---\n{}{}",
        masked_user_input,
        masked_chain.join("\n---\n"),
        context_additions
    );

    let (system, user) = prompts::build_email_prompt(
        &chain_text,
        EmailMode::Reply,
        friend,
        form,
        len,
        sender_name.as_deref(),
        subject.as_deref(),
        recipient_name.as_deref(),
    );

    let client = {
        let ai_guard = state.ai_client.read();
        ai_guard
            .as_ref()
            .ok_or_else(|| ApiError("KI-Client nicht konfiguriert".to_string()))?
            .clone()
    };

    let result = client.complete_user(&system, &user, None, None).await?;

    // Audit log
    if let Ok(db_guard) = get_db(&state) {
        if let Some(conn) = db_guard.as_ref() {
            let _ = audit::log_ai_action(
                conn,
                None,
                "generate_reply",
                &user_input,
                &result,
                Some(
                    &state
                        .ai_config
                        .read()
                        .as_ref()
                        .map(|c| c.model.clone())
                        .unwrap_or_default(),
                ),
                tone.freundlich,
                tone.professionell,
                tone.laenge,
                false,
            );
        }
    }

    Ok(Json(result))
}

#[derive(Deserialize)]
pub struct SummarizeRequest {
    pub body: String,
    pub account_id: i64,
    pub uid: i64,
    /// Optional folder name — scopes the summary write so a uid shared with
    /// another folder never overwrites a different message.
    pub folder: Option<String>,
}

/// `POST /api/v1/ai/summarize` — summarize a message and store summary + priority.
pub async fn ai_summarize(
    State(state): State<AppState>,
    Json(req): Json<SummarizeRequest>,
) -> ApiResult<String> {
    let masked = pii::mask_pii(&req.body);
    let system = prompts::AI_SUMMARY_PROMPT;
    let user = format!("E-Mail:\n{}", masked);

    let client = {
        let ai_guard = state.ai_client.read();
        ai_guard
            .as_ref()
            .ok_or_else(|| ApiError("KI-Client nicht konfiguriert".to_string()))?
            .clone()
    };

    let response = client
        .complete_user(system, &user, Some(0.3), Some(300))
        .await?;

    let summary = response
        .lines()
        .find(|l| l.starts_with("Zusammenfassung:"))
        .map(|l| l.trim_start_matches("Zusammenfassung:").trim())
        .unwrap_or(&response)
        .to_string();

    let priority = if response.contains("KRITISCH") {
        Some(0.95f32)
    } else if response.contains("ZEITKRITISCH") {
        Some(0.8f32)
    } else {
        Some(0.3f32)
    };

    if let Ok(conn) = state.cache_db.lock().as_ref().ok_or("") {
        let folder_id: Option<i64> = req
            .folder
            .as_deref()
            .and_then(|f| {
                conn.query_row(
                    "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
                    rusqlite::params![req.account_id, f],
                    |r| r.get(0),
                )
                .ok()
            });
        if let Some(p) = priority {
            let _ = cache::messages::update_ai_priority(conn, req.account_id, req.uid, folder_id, p);
        }
        let _ = cache::messages::update_ai_summary(conn, req.account_id, req.uid, folder_id, &summary);
    }

    Ok(Json(summary))
}

#[derive(Deserialize)]
pub struct TriggerFolderSummariesRequest {
    pub account_id: u32,
    pub folder: String,
}

/// `POST /api/v1/ai/trigger-folder-summaries` — scan a folder and enqueue
/// background summary tasks for messages missing an AI summary.
pub async fn trigger_folder_summaries(
    State(state): State<AppState>,
    Json(req): Json<TriggerFolderSummariesRequest>,
) -> ApiResult<u32> {
    let queue = state.sync_queue.clone();
    let (folder_id, uids): (i64, Vec<i64>) = with_db(&state, |conn| {
        let folder_id: i64 = match conn.query_row(
            "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
            rusqlite::params![req.account_id as i64, req.folder],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok((-1, Vec::new())),
            Err(e) => return Err(e.to_string()),
        };

        let mut stmt = conn
            .prepare(
                "SELECT uid FROM messages
                  WHERE account_id = ?1 AND folder_id = ?2
                    AND body_text IS NOT NULL
                    AND (ai_summary IS NULL OR ai_followups IS NULL)
                  ORDER BY date DESC
                  LIMIT 200",
            )
            .map_err(|e| e.to_string())?;

        let uids: Vec<i64> = stmt
            .query_map(rusqlite::params![req.account_id as i64, folder_id], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        Ok((folder_id, uids))
    })?;

    if uids.is_empty() {
        return Ok(Json(0));
    }

    let count = uids.len() as u32;
    for uid in uids {
        queue
            .enqueue(SyncTask {
                account_id: req.account_id,
                task_type: SyncTaskType::GenerateAiSummary(uid as u32, folder_id),
                created_at: tokio::time::Instant::now(),
                retries: 0,
                max_retries: 2,
                priority: 5,
            })
            .await;
    }

    Ok(Json(count))
}

/// `POST /api/v1/ai/reset-circuit-breaker` — reset the AI circuit breaker to closed state.
pub async fn reset_circuit_breaker(State(state): State<AppState>) -> ApiResult<()> {
    let ai_client = {
        let guard = state.ai_client.read();
        guard
            .as_ref()
            .ok_or_else(|| ApiError("KI-Client nicht konfiguriert".to_string()))?
            .clone()
    };
    // record_success() resets the circuit breaker to closed state
    ai_client.circuit_breaker().record_success();
    Ok(Json(()))
}

#[derive(Deserialize)]
pub struct DraftFromBulletsRequest {
    pub bullets: String,
    pub tone_freundlich: u8,
    pub tone_professionell: u8,
    pub tone_laenge: u8,
    pub sender_name: Option<String>,
}

/// `POST /api/v1/ai/draft-from-bullets` — draft a full email from bullet points.
pub async fn ai_draft_from_bullets(
    State(state): State<AppState>,
    Json(req): Json<DraftFromBulletsRequest>,
) -> ApiResult<String> {
    let intent = parse_intent(&req.bullets);

    let form = req.tone_professionell;
    let friend = req.tone_freundlich;
    let len = req.tone_laenge;

    let enriched = if let Some(ref name) = intent.recipient_name {
        format!("Empfaenger: {}\n\n{}", name, req.bullets)
    } else {
        req.bullets
    };

    let masked = pii::mask_pii(&enriched);
    let (system, user) = prompts::build_email_prompt(
        &masked,
        EmailMode::FromBullets,
        friend,
        form,
        len,
        req.sender_name.as_deref(),
        None,
        None,
    );

    let client = {
        let ai_guard = state.ai_client.read();
        ai_guard
            .as_ref()
            .ok_or_else(|| ApiError("KI-Client nicht konfiguriert".to_string()))?
            .clone()
    };

    let result = client.complete_user(&system, &user, None, None).await?;

    // Audit log
    if let Ok(db_guard) = get_db(&state) {
        if let Some(conn) = db_guard.as_ref() {
            let _ = audit::log_ai_action(
                conn,
                None,
                "draft_from_bullets",
                &enriched,
                &result,
                Some(
                    &state
                        .ai_config
                        .read()
                        .as_ref()
                        .map(|c| c.model.clone())
                        .unwrap_or_default(),
                ),
                Some(req.tone_freundlich),
                Some(req.tone_professionell),
                Some(req.tone_laenge),
                false,
            );
        }
    }

    Ok(Json(result))
}

#[derive(Deserialize)]
pub struct FormatTextRequest {
    pub text: String,
}

/// `POST /api/v1/ai/format-text` — reformat and grammar-correct a text.
pub async fn ai_format_text(
    State(state): State<AppState>,
    Json(req): Json<FormatTextRequest>,
) -> ApiResult<String> {
    let system = "Formatiere den folgenden Text und verbessere Grammatik und Rechtschreibung. \
                  Layout: Leerzeile zwischen Absaetzen, Absaetze max. 4 Zeilen. \
                  Fettdruck nur für Datum, Deadline oder konkrete Handlungsaufforderung (max. 2x). \
                  Aufzaehlungen nur ab 3 Punkten, kurze Stichpunkte ohne Schlusspunkt. \
                  Keine Zwischenueberschriften. \
                  Gib NUR den formatierten Text zurueck, keine Erklaerung.";
    let masked = pii::mask_pii(&req.text);
    let user = format!("Text:\n{}", masked);

    let client = {
        let ai_guard = state.ai_client.read();
        ai_guard
            .as_ref()
            .ok_or_else(|| ApiError("KI-Client nicht konfiguriert".to_string()))?
            .clone()
    };

    let result = client.complete_user(system, &user, Some(0.3), None).await?;

    // Audit log
    if let Ok(db_guard) = get_db(&state) {
        if let Some(conn) = db_guard.as_ref() {
            let _ = audit::log_ai_action(
                conn,
                None,
                "format_text",
                &req.text,
                &result,
                Some(
                    &state
                        .ai_config
                        .read()
                        .as_ref()
                        .map(|c| c.model.clone())
                        .unwrap_or_default(),
                ),
                None,
                None,
                None,
                false,
            );
        }
    }

    Ok(Json(result))
}

#[derive(Deserialize)]
pub struct DetectPriorityRequest {
    pub subject: String,
    pub body: String,
}

/// `POST /api/v1/ai/detect-priority` — heuristic priority score (no LLM call).
pub async fn ai_detect_priority(
    State(_): State<AppState>,
    Json(req): Json<DetectPriorityRequest>,
) -> ApiResult<f32> {
    Ok(Json(priority::detect_priority(&req.subject, &req.body)))
}

#[derive(Deserialize)]
pub struct FraudCheckRequest {
    pub subject: String,
    pub body: String,
}

#[derive(Serialize)]
pub struct FraudCheckResponse {
    pub score: f32,
    pub warnings: Vec<String>,
}

/// `POST /api/v1/ai/fraud-check` — phishing/fraud heuristics (no LLM call).
pub async fn fraud_check(
    State(_): State<AppState>,
    Json(req): Json<FraudCheckRequest>,
) -> ApiResult<FraudCheckResponse> {
    let result = fraud::detect_fraud(&req.subject, &req.body);
    Ok(Json(FraudCheckResponse {
        score: result.score,
        warnings: result.warnings,
    }))
}

#[derive(Deserialize)]
pub struct GenerateMailRequest {
    pub account_id: u32,
    pub to: String,
    pub subject: String,
    pub user_input: String,
    pub sender_name: String,
    pub seriousness: u8,
    pub text_length: u8,
    pub original_message: Option<String>,
}

/// `POST /api/v1/ai/generate-mail` — full mail generation from free-text input.
pub async fn ai_generate_mail(
    State(state): State<AppState>,
    Json(req): Json<GenerateMailRequest>,
) -> ApiResult<String> {
    // Load contact profile for recipient to influence tone
    let _profile = if let Ok(db_guard) = get_db(&state) {
        if let Some(conn) = db_guard.as_ref() {
            match ProfileManager::get_profile(conn, req.account_id as i64, &req.to) {
                Ok(p) => Some(p),
                Err(_) => None,
            }
        } else {
            None
        }
    } else {
        None
    };

    // Load few-shot snippets from user's actual sent emails to this recipient
    let email_hash = ProfileManager::hash_email(&req.to);
    let topic = cache::topic::detect_topic(&req.subject, &req.user_input);
    let snippets: Vec<String> = if let Ok(db_guard) = get_db(&state) {
        if let Some(conn) = db_guard.as_ref() {
            // Try topic-specific snippets first, fall back to general
            match cache::snippets::get_snippets(conn, req.account_id as i64, &email_hash, &topic, 3)
            {
                Ok(s) if !s.is_empty() => s,
                _ => cache::snippets::get_general_snippets(
                    conn,
                    req.account_id as i64,
                    &email_hash,
                    3,
                )
                .unwrap_or_default(),
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let masked_input = pii::mask_pii(&req.user_input);
    let masked_original = req.original_message.as_ref().map(|m| pii::mask_pii(m));

    let emotion = req.original_message.as_ref().map(|m| analyze_mail(m).emotion);

    // Extract tone intent from user input (e.g. "Schreibe Max eine Mail, dringend")
    let intent = parse_intent(&req.user_input);

    // Fetch style fingerprint for this recipient (from learning loop)
    let style_fingerprint: Option<String> = if !req.to.is_empty() {
        with_db(&state, |conn| {
            let hash = ProfileManager::hash_email(&req.to);
            match cache::fingerprint::get_fingerprint(conn, req.account_id as i64, &hash) {
                Ok(Some(fp)) => Ok(Some(fp.fingerprint)),
                _ => Ok(None),
            }
        })
        .ok()
        .flatten()
    } else {
        None
    };

    // Apply tone intent hints: adjust seriousness if implied formality differs
    let effective_seriousness = intent
        .tone_hints
        .implied_formality
        .map(|f| {
            let suggested = (f * 6.0 + 1.0) as u8;
            if req.seriousness == 4 {
                suggested
            } else {
                req.seriousness
            }
        })
        .unwrap_or(req.seriousness);

    let language = req
        .original_message
        .as_ref()
        .or(Some(&req.user_input))
        .and_then(|t| if t.len() > 20 { Some(detect_language(t)) } else { None })
        .unwrap_or("de");

    let (system, user) = prompts::build_generate_mail_prompt(
        &req.to,
        &req.subject,
        &masked_input,
        &req.sender_name,
        effective_seriousness,
        req.text_length,
        masked_original.as_deref(),
        emotion.as_deref(),
        &snippets,
        style_fingerprint.as_deref(),
        intent.tone_hints.occasion.as_deref(),
        language,
    );

    let client = {
        let ai_guard = state.ai_client.read();
        ai_guard
            .as_ref()
            .ok_or_else(|| ApiError("KI-Client nicht konfiguriert".to_string()))?
            .clone()
    };

    let result = client.complete_user(&system, &user, None, None).await?;

    // Audit log
    if let Ok(db_guard) = get_db(&state) {
        if let Some(conn) = db_guard.as_ref() {
            let _ = audit::log_ai_action(
                conn,
                None,
                "generate_mail",
                &req.user_input,
                &result,
                Some(
                    &state
                        .ai_config
                        .read()
                        .as_ref()
                        .map(|c| c.model.clone())
                        .unwrap_or_default(),
                ),
                None,              // tone_freundlich — v2 doesn't have separate friendliness
                Some(req.seriousness), // tone_professionell — seriousness ≈ formality
                Some(req.text_length), // tone_laenge — text_length maps to length
                false,
            );
        }
    }

    Ok(Json(result))
}

#[derive(Deserialize)]
pub struct ToneProfileRequest {
    pub account_id: u32,
    pub email: String,
}

/// `POST /api/v1/ai/tone-profile` — return the tone profile for a recipient.
pub async fn get_tone_profile(
    State(state): State<AppState>,
    Json(req): Json<ToneProfileRequest>,
) -> ApiResult<Option<ContactProfile>> {
    let db_guard = get_db(&state).map_err(ApiError)?;
    let conn = db_guard
        .as_ref()
        .ok_or_else(|| ApiError("Datenbank nicht initialisiert".to_string()))?;

    let profile = ProfileManager::get_profile(conn, req.account_id as i64, &req.email)
        .map_err(|e| e.to_string())?;

    // Only return if we have samples
    if profile.sample_count > 0 {
        Ok(Json(Some(profile)))
    } else {
        Ok(Json(None))
    }
}

#[derive(Deserialize)]
pub struct ExportToneProfilesRequest {
    pub account_id: u32,
}

/// `POST /api/v1/ai/tone-profiles/export` — export all tone profiles of an
/// account as a Markdown table (plain JSON string response).
pub async fn export_tone_profiles(
    State(state): State<AppState>,
    Json(req): Json<ExportToneProfilesRequest>,
) -> ApiResult<String> {
    let db_guard = get_db(&state).map_err(ApiError)?;
    let conn = db_guard
        .as_ref()
        .ok_or_else(|| ApiError("Datenbank nicht initialisiert".to_string()))?;

    let markdown = ProfileManager::export_as_markdown(conn, req.account_id as i64)
        .map_err(|e| e.to_string())?;
    Ok(Json(markdown))
}

#[derive(Deserialize)]
pub struct SuggestRecipientRequest {
    pub to: String,
    pub subject: String,
    pub user_input: String,
    pub original_message: Option<String>,
}

/// `POST /api/v1/ai/suggest-recipient` — derive a recipient address from context.
pub async fn ai_suggest_recipient(
    State(state): State<AppState>,
    Json(req): Json<SuggestRecipientRequest>,
) -> ApiResult<String> {
    // If recipient is already provided, return it unchanged
    if !req.to.trim().is_empty() {
        return Ok(Json(req.to));
    }

    // Build context from available info
    let mut context = String::new();
    if !req.subject.is_empty() {
        context.push_str(&format!("Betreff: {}\n", req.subject));
    }
    if !req.user_input.is_empty() {
        context.push_str(&format!("Stichwörter: {}\n", req.user_input));
    }
    if let Some(ref orig) = req.original_message {
        if !orig.is_empty() {
            context.push_str(&format!("Ursprüngliche Nachricht:\n{}", orig));
        }
    }

    // NOTE: No PII masking here — the AI needs to see real email addresses to extract the recipient
    let (system, user) = prompts::build_recipient_suggestion_prompt(&context);

    let client = {
        let ai_guard = state.ai_client.read();
        ai_guard
            .as_ref()
            .ok_or_else(|| ApiError("KI-Client nicht konfiguriert".to_string()))?
            .clone()
    };

    let result = client
        .complete_user(&system, &user, Some(0.3), Some(100))
        .await?;

    // Audit log
    if let Ok(db_guard) = get_db(&state) {
        if let Some(conn) = db_guard.as_ref() {
            let _ = audit::log_ai_action(
                conn,
                None,
                "suggest_recipient",
                &req.user_input,
                &result,
                Some(
                    &state
                        .ai_config
                        .read()
                        .as_ref()
                        .map(|c| c.model.clone())
                        .unwrap_or_default(),
                ),
                None,
                None,
                None,
                false,
            );
        }
    }

    Ok(Json(result))
}

#[derive(Deserialize)]
pub struct SuggestSubjectRequest {
    pub to: String,
    pub subject: String,
    pub user_input: String,
    pub original_message: Option<String>,
}

/// `POST /api/v1/ai/suggest-subject` — derive a subject line from context.
pub async fn ai_suggest_subject(
    State(state): State<AppState>,
    Json(req): Json<SuggestSubjectRequest>,
) -> ApiResult<String> {
    // If subject is already provided, return it unchanged
    if !req.subject.trim().is_empty() {
        return Ok(Json(req.subject));
    }

    // Build context from available info
    let mut context = String::new();
    if !req.to.is_empty() {
        context.push_str(&format!("Empfänger: {}\n", req.to));
    }
    if !req.user_input.is_empty() {
        context.push_str(&format!("Stichwörter: {}\n", req.user_input));
    }
    if let Some(ref orig) = req.original_message {
        if !orig.is_empty() {
            context.push_str(&format!("Ursprüngliche Nachricht:\n{}", orig));
        }
    }

    let masked = pii::mask_pii(&context);
    let (system, user) = prompts::build_subject_suggestion_prompt(&masked);

    let client = {
        let ai_guard = state.ai_client.read();
        ai_guard
            .as_ref()
            .ok_or_else(|| ApiError("KI-Client nicht konfiguriert".to_string()))?
            .clone()
    };

    let result = client
        .complete_user(&system, &user, Some(0.3), Some(100))
        .await?;

    // Audit log
    if let Ok(db_guard) = get_db(&state) {
        if let Some(conn) = db_guard.as_ref() {
            let _ = audit::log_ai_action(
                conn,
                None,
                "suggest_subject",
                &req.user_input,
                &result,
                Some(
                    &state
                        .ai_config
                        .read()
                        .as_ref()
                        .map(|c| c.model.clone())
                        .unwrap_or_default(),
                ),
                None,
                None,
                None,
                false,
            );
        }
    }

    Ok(Json(result))
}

// ---------------------------------------------------------------------------
// Phase 2: Calendar AI (conflict alternatives, time extraction, RSVP drafts)
// ---------------------------------------------------------------------------

/// Pull the first JSON array out of a (possibly markdown-wrapped) LLM reply.
fn extract_json_array(raw: &str) -> Vec<serde_json::Value> {
    let (Some(s), Some(e)) = (raw.find('['), raw.rfind(']')) else {
        return Vec::new();
    };
    if e < s {
        return Vec::new();
    }
    serde_json::from_str(&raw[s..=e]).unwrap_or_default()
}

/// Strip a single ``` / ```json code fence if the reply is wrapped in one
/// (Insilo §7.1 — Qwen wraps structured output in fences).
fn unwrap_json_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(open) = trimmed.find("```") {
        let rest = &trimmed[open + 3..];
        // Drop an optional language tag on the fence's first line.
        let rest = rest.split_once('\n').map(|(_, r)| r).unwrap_or(rest);
        if let Some(close) = rest.rfind("```") {
            return rest[..close].trim();
        }
    }
    raw
}

/// Short, log-safe excerpt of a raw model reply (for loud parse errors).
fn raw_excerpt(raw: &str) -> String {
    let t = raw.trim();
    if t.len() <= 120 {
        t.to_string()
    } else {
        format!("{}…", &t[..120])
    }
}

/// Pull the outermost JSON object out of a (possibly markdown-wrapped) LLM
/// reply. Fence-unwrap → outermost `{…}` → **fail loudly** (B2): returns an
/// error carrying a raw excerpt instead of silently yielding `Null`.
fn extract_json_object(raw: &str) -> Result<serde_json::Value, String> {
    let candidate = unwrap_json_fence(raw);
    let (Some(s), Some(e)) = (candidate.find('{'), candidate.rfind('}')) else {
        return Err(format!(
            "Kein JSON-Objekt in der Modellantwort (Auszug: {})",
            raw_excerpt(raw)
        ));
    };
    if e < s {
        return Err(format!(
            "Kein JSON-Objekt in der Modellantwort (Auszug: {})",
            raw_excerpt(raw)
        ));
    }
    serde_json::from_str(&candidate[s..=e]).map_err(|e| {
        format!(
            "Ungültiges JSON in der Modellantwort: {e} (Auszug: {})",
            raw_excerpt(raw)
        )
    })
}

fn get_ai_client(state: &AppState) -> Result<std::sync::Arc<crate::ai::client::AIClient>, ApiError> {
    let ai_guard = state.ai_client.read();
    ai_guard
        .as_ref()
        .ok_or_else(|| ApiError("KI-Client nicht konfiguriert".to_string()))
        .cloned()
}

#[derive(Deserialize)]
pub struct ConflictAlternativesRequest {
    pub summary: String,
    pub start: String,
    pub end: String,
    #[serde(default)]
    pub calendar_id: Option<i64>,
}

#[derive(Serialize, Deserialize)]
pub struct TimeSlot {
    pub start: String,
    pub end: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `POST /api/v1/ai/conflict-alternatives` — suggest conflict-free alternative times.
pub async fn ai_conflict_alternatives(
    State(state): State<AppState>,
    Json(req): Json<ConflictAlternativesRequest>,
) -> ApiResult<Vec<TimeSlot>> {
    let conflict_desc = with_db(&state, |conn| {
        let conflicts = crate::cache::cal::find_conflicts(conn, req.calendar_id, &req.start, &req.end, None)
            .map_err(|e| e.to_string())?;
        Ok(if conflicts.is_empty() {
            "kein Konflikt".to_string()
        } else {
            conflicts
                .iter()
                .map(|c| format!("{} ({})", c.summary.as_deref().unwrap_or("(ohne Titel)"), c.start_at))
                .collect::<Vec<_>>()
                .join("; ")
        })
    })?;

    let duration = match (
        chrono::DateTime::parse_from_rfc3339(&req.start).ok(),
        chrono::DateTime::parse_from_rfc3339(&req.end).ok(),
    ) {
        (Some(s), Some(e)) => ((e - s).num_minutes()).max(1).min(480) as u32,
        _ => 60,
    };

    let (system, user) = prompts::build_conflict_alternatives_prompt(
        &req.summary, &req.start, &req.end, &conflict_desc, duration,
    );
    let client = get_ai_client(&state)?;
    let raw = client.complete_user(&system, &user, Some(0.4), Some(600)).await?;

    let mut out = Vec::new();
    for slot in extract_json_array(&raw) {
        let (Some(s), Some(e)) = (
            slot.get("start").and_then(|v| v.as_str()),
            slot.get("end").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        let still_conflicts = with_db(&state, |conn| {
            crate::cache::cal::find_conflicts(conn, req.calendar_id, s, e, None)
                .map(|c| !c.is_empty())
                .map_err(|err| err.to_string())
        })
        .unwrap_or(false);
        if still_conflicts {
            continue;
        }
        out.push(TimeSlot {
            start: s.to_string(),
            end: e.to_string(),
            reason: slot.get("reason").and_then(|v| v.as_str()).map(String::from),
        });
        if out.len() >= 3 {
            break;
        }
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct ExtractTimeRequest {
    pub text: String,
    #[serde(default)]
    pub reference_date: Option<String>,
}

#[derive(Serialize)]
pub struct ExtractedTime {
    pub summary: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub all_day: bool,
}

/// `POST /api/v1/ai/extract-time` — extract a meeting time from free text.
pub async fn ai_extract_time(
    State(state): State<AppState>,
    Json(req): Json<ExtractTimeRequest>,
) -> ApiResult<ExtractedTime> {
    let ref_date = req.reference_date.unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let (system, user) = prompts::build_time_extraction_prompt(&req.text, &ref_date);
    let client = get_ai_client(&state)?;
    let raw = client.complete_user(&system, &user, Some(0.1), Some(300)).await?;
    let obj = extract_json_object(&raw).unwrap_or(serde_json::Value::Null);
    Ok(Json(ExtractedTime {
        summary: obj.get("summary").and_then(|v| v.as_str()).map(String::from),
        start: obj.get("start").and_then(|v| v.as_str()).map(String::from),
        end: obj.get("end").and_then(|v| v.as_str()).map(String::from),
        all_day: obj.get("all_day").and_then(|v| v.as_bool()).unwrap_or(false),
    }))
}

#[derive(Deserialize)]
pub struct RsvpDraftRequest {
    pub summary: String,
    pub start: String,
    pub organizer: String,
    pub decision: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// `POST /api/v1/ai/rsvp-draft` — draft a short RSVP reply to an invitation.
pub async fn ai_rsvp_draft(
    State(state): State<AppState>,
    Json(req): Json<RsvpDraftRequest>,
) -> ApiResult<String> {
    let note = req.note.as_deref().unwrap_or("");
    let (system, user) = prompts::build_rsvp_draft_prompt(
        &req.summary, &req.start, &req.organizer, &req.decision, note,
    );
    let client = get_ai_client(&state)?;
    let result = client.complete_user(&system, &user, Some(0.5), Some(400)).await?;
    Ok(Json(result))
}

/// Phase C — a typed follow-up suggestion (Concept §12.3, §9.4). Each maps to
/// exactly one registry tool; `plan` is the prepared (unexecuted) card the user
/// will confirm. Nothing is executed until the user confirms the plan.
#[derive(Serialize, Deserialize, Clone)]
pub struct FollowupSuggestion {
    pub id: String,
    /// Short chip label (max ~6 words).
    pub titel: String,
    /// Registry tool name (e.g. "tasks_create").
    pub tool: String,
    /// Tool arguments (validated against the tool schema).
    pub args: serde_json::Value,
    /// The prepared card the user confirms.
    pub plan: crate::ai::tools::PreparedCard,
}

/// Phase C — the followups v2 response shape. The cache stores this object;
/// the legacy bare-array format is treated as a cache miss (re-generated).
#[derive(Serialize, Deserialize)]
pub struct FollowupsResponse {
    pub actions: Vec<FollowupSuggestion>,
}

#[derive(Deserialize)]
pub struct FollowupsRequest {
    pub subject: String,
    pub from: String,
    pub body: String,
    /// Optional message identity: enables the server-side followup cache
    /// (pre-generated by the AI worker for new mail, cached on first open).
    #[serde(default)]
    pub account_id: Option<u32>,
    #[serde(default)]
    pub uid: Option<u32>,
    #[serde(default)]
    pub folder: Option<String>,
}

/// `POST /api/v1/ai/followups` — Phase C: typed follow-up suggestions
/// (Concept §12.3). Returns `{actions: [FollowupSuggestion]}`; each suggestion
/// maps to one registry tool and carries the prepared (unexecuted) card the
/// user confirms. Cached per message; the legacy bare-array format is treated
/// as a cache miss and re-generated (alt-format tolerance, Concept §12.3).
pub async fn ai_followups(
    State(state): State<AppState>,
    Json(req): Json<FollowupsRequest>,
) -> ApiResult<FollowupsResponse> {
    let message_id = req.uid.unwrap_or(0) as i64;
    // The on-demand handler has no per-user locale yet; default "de" (matches
    // the pre-generation worker). Phase C+ threads the user locale through.
    let locale = "de";

    if let (Some(account_id), Some(uid)) = (req.account_id, req.uid) {
        let cached: Option<FollowupsResponse> = with_db(&state, |conn| {
            match crate::cache::messages::get_ai_followups(conn, account_id as i64, uid as i64, req.folder.as_deref())
                .map_err(|e| e.to_string())?
            {
                // Old-format (bare array) fails to parse as the object → None → re-gen.
                Some(json) => Ok(serde_json::from_str(&json).ok()),
                None => Ok(None),
            }
        })
        .ok()
        .flatten()
        .flatten();
        if let Some(resp) = cached {
            return Ok(Json(resp));
        }
    }

    let actions =
        generate_followups_v2(&state, &req.subject, &req.from, &req.body, message_id, locale).await?;

    if let (Some(account_id), Some(uid)) = (req.account_id, req.uid) {
        if !actions.is_empty() {
            if let Ok(json) = serde_json::to_string(&FollowupsResponse { actions: actions.clone() }) {
                let folder = req.folder.clone();
                let _ = with_db(&state, move |conn| {
                    crate::cache::messages::set_ai_followups(conn, account_id as i64, uid as i64, folder.as_deref(), &json)
                        .map_err(|e| e.to_string())
                });
            }
        }
    }
    Ok(Json(FollowupsResponse { actions }))
}

/// Phase C — generate typed follow-up suggestions for a message (LLM call +
/// registry-validated parse). Shared by the API handler and the background
/// pre-generation worker so new mail gets its suggestions ready before the
/// browser opens it.
pub async fn generate_followups_v2(
    state: &AppState,
    subject: &str,
    from: &str,
    body: &str,
    message_id: i64,
    locale: &str,
) -> Result<Vec<FollowupSuggestion>, ApiError> {
    let now_str = chrono::Utc::now().to_rfc3339();
    let (system, user) = prompts::build_followups_v2_prompt(subject, from, body, message_id, &now_str, locale);
    let client = get_ai_client(state)?;
    let raw = client.complete_user_json(&system, &user, Some(0.3), Some(1200)).await?;
    Ok(parse_followups_v2(state, &raw, message_id, subject, locale).await)
}

/// Phase C — parse the LLM's raw output into validated suggestions (Concept
/// §12.3, §6.5). Pure of the LLM: takes the raw JSON string, validates each
/// suggestion against the tool registry (unknown tools are discarded — the
/// anti-injection fixpoint), and builds the prepared card by running the tool.
/// Only `Card` outcomes become suggestions; nothing is ever executed here.
pub async fn parse_followups_v2(
    state: &AppState,
    raw: &str,
    source_message_id: i64,
    subject: &str,
    locale: &str,
) -> Vec<FollowupSuggestion> {
    let obj = extract_json_object(raw).unwrap_or(serde_json::Value::Null);
    let suggestions = obj
        .get("suggestions")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();

    // Seed the source message so mail_propose_reply passes the ID-provenance check.
    let known_ids = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    {
        let mut g = known_ids.lock().await;
        g.insert(source_message_id.to_string(), subject.to_string());
    }
    let ctx = crate::ai::tools::ToolCtx {
        state: state.clone(),
        locale: locale.to_string(),
        known_ids,
    };

    let mut out = Vec::new();
    for (i, sug) in suggestions.iter().take(3).enumerate() {
        let tool = sug.get("tool").and_then(|v| v.as_str()).unwrap_or("").to_string();
        // Registry validation: unknown tools are discarded (anti-injection).
        if crate::ai::tools::find_tool(&tool).is_none() {
            continue;
        }
        let mut args = sug.get("args").cloned().unwrap_or(serde_json::Value::Null);
        // mail_propose_reply must target the source message (override any LLM id).
        if tool == "mail_propose_reply" {
            if let Some(o) = args.as_object_mut() {
                o.insert("id".into(), serde_json::Value::String(source_message_id.to_string()));
            }
        }
        match crate::ai::tools::execute(&tool, args.clone(), ctx.clone()).await {
            Ok(crate::ai::tools::ToolOutcome::Card(card)) => {
                let titel = sug
                    .get("titel")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| card.title.clone());
                out.push(FollowupSuggestion {
                    id: format!("fu-{}", i + 1),
                    titel,
                    tool: tool.clone(),
                    args,
                    plan: card,
                });
            }
            _ => continue, // Data / Nachfrage / Nav / Err → not a suggestion
        }
    }
    out
}

#[derive(Deserialize)]
pub struct CounterEmailRequest {
    pub from: String,
    pub meeting_title: String,
    pub requested_start: String,
    pub alternative_start: String,
    pub alternative_end: String,
}

/// `POST /api/v1/ai/followups/counter-email` — draft a counter-offer email that
/// proposes the specific alternative slot the user picked.
pub async fn ai_followups_counter_email(
    State(state): State<AppState>,
    Json(req): Json<CounterEmailRequest>,
) -> ApiResult<serde_json::Value> {
    let (system, user) = prompts::build_counter_email_prompt(
        &req.from,
        &req.meeting_title,
        &req.requested_start,
        &req.alternative_start,
        &req.alternative_end,
    );
    let client = get_ai_client(&state)?;
    let raw = client.complete_user(&system, &user, Some(0.4), Some(400)).await?;
    let obj = extract_json_object(&raw).unwrap_or(serde_json::Value::Null);
    let subject = obj.get("subject").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let body = obj.get("body").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if body.is_empty() {
        return Err(ApiError("KI lieferte keinen E-Mail-Text.".to_string()));
    }
    Ok(Json(serde_json::json!({ "subject": subject, "body": body })))
}

// ─── Phase 4 — AI-First helpers ─────────────────────────────

/// Format up to `limit` contacts as one line each (for AI context).
fn gather_contact_context(conn: &rusqlite::Connection) -> Result<String, String> {
    let contacts = crate::cache::contacts::list_contacts(conn, "").map_err(|e| e.to_string())?;
    if contacts.is_empty() {
        return Ok("keine Kontakte".to_string());
    }
    let lines: Vec<String> = contacts.iter().take(30).map(|c| {
        let name = c.display_name
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| c.email.clone().filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "unbekannt".to_string());
        let mut line = format!("- {}", name);
        if let Some(org) = &c.organization {
            if !org.is_empty() {
                line.push_str(&format!(" ({})", org));
            }
        }
        if let Some(email) = &c.email {
            if !email.is_empty() {
                line.push_str(&format!(", {}", email));
            }
        }
        line
    }).collect();
    Ok(lines.join("\n"))
}

/// Fetch recent messages (subject/from/date) for AI context.
fn gather_recent_mails(conn: &rusqlite::Connection, days: i64, limit: usize) -> Result<String, String> {
    let mut stmt = conn.prepare(
        "SELECT subject, from_addr, date FROM messages \
         WHERE date >= datetime('now', ?1) ORDER BY date DESC LIMIT ?2",
    ).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![format!("-{} days", days), limit as i64], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            ))
        })
        .map_err(|e| e.to_string())?;
    let rows = rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Ok("keine relevanten Mails".to_string());
    }
    let lines: Vec<String> = rows.iter().take(limit).map(|(s, f, d)| {
        format!("- [{}] {} (von: {})", d, s, f)
    }).collect();
    Ok(lines.join("\n"))
}

/// Format calendar events in a date range as one line each.
fn gather_events(conn: &rusqlite::Connection, start: &str, end: &str) -> Result<String, String> {
    let events = crate::cache::cal::list_events(conn, None, Some(start), Some(end)).map_err(|e| e.to_string())?;
    if events.is_empty() {
        return Ok("keine Termine".to_string());
    }
    let lines: Vec<String> = events.iter().take(30).map(|e| {
        let title = e.summary.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "(ohne Titel)".to_string());
        format!("- {} {} ({} – {})", e.start_at, title, e.start_at, e.end_at.clone().unwrap_or_else(|| e.start_at.clone()))
    }).collect();
    Ok(lines.join("\n"))
}

/// Format open tasks as one line each.
fn gather_tasks(conn: &rusqlite::Connection) -> Result<String, String> {
    let todos = crate::cache::todo::list_todos(conn, Some(false)).map_err(|e| e.to_string())?;
    if todos.is_empty() {
        return Ok("keine offenen Aufgaben".to_string());
    }
    let lines: Vec<String> = todos.iter().take(30).map(|t| {
        let title = t.summary.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "(ohne Titel)".to_string());
        let due = t.due_at.clone().unwrap_or_else(|| "ohne Datum".to_string());
        format!("- {} (fällig: {})", title, due)
    }).collect();
    Ok(lines.join("\n"))
}

// ─── Phase 4.1 — NL-Erstellung ──────────────────────────────

#[derive(Deserialize)]
pub struct NlCreateRequest {
    pub text: String,
    #[serde(default)]
    pub context: Option<String>,
}

#[derive(Serialize)]
pub struct NlCreateResult {
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    #[serde(default)]
    pub attendees: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
}

/// `POST /api/v1/ai/nl-create` — parse natural language into an event or task.
pub async fn ai_nl_create(
    State(state): State<AppState>,
    Json(req): Json<NlCreateRequest>,
) -> ApiResult<NlCreateResult> {
    let now = chrono::Utc::now().to_rfc3339();
    let context = req.context.as_deref().unwrap_or("unbekannt");
    let (system, user) = prompts::build_nl_create_prompt(&req.text, &now, context);
    let client = get_ai_client(&state)?;
    let raw = client.complete_user(&system, &user, Some(0.3), Some(600)).await?;
    let obj = extract_json_object(&raw).unwrap_or(serde_json::Value::Null);
    let kind = obj.get("type").and_then(|v| v.as_str()).unwrap_or("event").to_string();
    let title = obj.get("title").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let attendees = obj.get("attendees")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|a| a.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    Ok(Json(NlCreateResult {
        kind,
        title,
        start: obj.get("start").and_then(|v| v.as_str()).map(str::to_string),
        end: obj.get("end").and_then(|v| v.as_str()).map(str::to_string),
        attendees,
        description: obj.get("description").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_string),
        due: obj.get("due").and_then(|v| v.as_str()).map(str::to_string),
    }))
}

// ─── Phase 4.2 — Smart Scheduling ───────────────────────────

#[derive(Deserialize)]
pub struct ScheduleRequest {
    pub request: String,
    #[serde(default)]
    pub participants: Option<String>,
    #[serde(default)]
    pub free_slots: Option<String>,
    #[serde(default)]
    pub constraints: Option<String>,
}

#[derive(Serialize)]
pub struct ScheduleSuggestion {
    pub start: String,
    pub end: String,
    pub confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Serialize)]
pub struct ScheduleResult {
    pub suggestions: Vec<ScheduleSuggestion>,
}

/// `POST /api/v1/ai/schedule` — suggest optimal meeting times.
pub async fn ai_schedule(
    State(state): State<AppState>,
    Json(req): Json<ScheduleRequest>,
) -> ApiResult<ScheduleResult> {
    let now = chrono::Utc::now().to_rfc3339();
    let (system, user) = prompts::build_smart_schedule_prompt(
        &req.request,
        req.participants.as_deref().unwrap_or("unbekannt"),
        req.free_slots.as_deref().unwrap_or("unbekannt"),
        req.constraints.as_deref().unwrap_or("keine"),
        &now,
    );
    let client = get_ai_client(&state)?;
    let raw = client.complete_user(&system, &user, Some(0.3), Some(1600)).await?;
    let obj = extract_json_object(&raw).unwrap_or(serde_json::Value::Null);
    let suggestions = obj.get("suggestions").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|s| {
            let start = s.get("start")?.as_str()?.to_string();
            let end = s.get("end")?.as_str()?.to_string();
            let confidence = s.get("confidence").and_then(|c| c.as_f64()).unwrap_or(0.5);
            Some(ScheduleSuggestion {
                start,
                end,
                confidence,
                reason: s.get("reason").and_then(|r| r.as_str()).map(str::to_string),
            })
        }).collect())
        .unwrap_or_default();
    Ok(Json(ScheduleResult { suggestions }))
}

// ─── Phase 4.3 — Meeting-Prep ───────────────────────────────

#[derive(Deserialize)]
pub struct MeetingPrepRequest {
    pub summary: String,
    pub start: String,
    #[serde(default)]
    pub attendees: Vec<String>,
}

#[derive(Serialize)]
pub struct MeetingPrepResult {
    pub attendees: Vec<String>,
    pub agenda: Vec<String>,
    pub prep_notes: String,
}

/// `POST /api/v1/ai/meeting-prep` — prepare for an upcoming meeting.
pub async fn ai_meeting_prep(
    State(state): State<AppState>,
    Json(req): Json<MeetingPrepRequest>,
) -> ApiResult<MeetingPrepResult> {
    let (contacts_str, mails_str) = with_db(&state, |conn| {
        let contacts = gather_contact_context(conn)?;
        let mails = gather_recent_mails(conn, 14, 15)?;
        Ok((contacts, mails))
    })?;
    let (system, user) = prompts::build_meeting_prep_prompt(
        &req.summary, &req.start, &contacts_str, &mails_str,
    );
    let client = get_ai_client(&state)?;
    let raw = client.complete_user(&system, &user, Some(0.4), Some(1600)).await?;
    let obj = extract_json_object(&raw).unwrap_or(serde_json::Value::Null);
    let attendees = obj.get("attendees").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|a| a.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let agenda = obj.get("agenda").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|a| a.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let prep_notes = obj.get("prep_notes").and_then(|v| v.as_str()).unwrap_or("").to_string();
    Ok(Json(MeetingPrepResult { attendees, agenda, prep_notes }))
}

// ─── Phase 4.4 — Agenda-Digest ──────────────────────────────

#[derive(Deserialize)]
pub struct AgendaDigestRequest {
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub horizon: Option<u32>,
}

#[derive(Serialize)]
pub struct AgendaDigestResult {
    pub digest: String,
    pub priorities: Vec<String>,
    pub followups: Vec<String>,
}

/// `POST /api/v1/ai/agenda-digest` — produce a daily/weekly agenda digest.
pub async fn ai_agenda_digest(
    State(state): State<AppState>,
    Json(req): Json<AgendaDigestRequest>,
) -> ApiResult<AgendaDigestResult> {
    let horizon = req.horizon.unwrap_or(7).clamp(1, 30);
    let (date_str, events_str, tasks_str, mails_str) = with_db(&state, |conn| {
        let now = chrono::Utc::now();
        let start = req.date.clone().unwrap_or_else(|| now.format("%Y-%m-%dT%H:%M:%SZ").to_string());
        let end = (now + chrono::Duration::days(horizon as i64)).format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let events = gather_events(conn, &start, &end)?;
        let tasks = gather_tasks(conn)?;
        let mails = gather_recent_mails(conn, -(horizon as i64), 15)?;
        Ok((start, events, tasks, mails))
    })?;
    let (system, user) = prompts::build_agenda_digest_prompt(&date_str, horizon, &events_str, &tasks_str, &mails_str);
    let client = get_ai_client(&state)?;
    let raw = client.complete_user(&system, &user, Some(0.4), Some(1600)).await?;
    let obj = extract_json_object(&raw).unwrap_or(serde_json::Value::Null);
    let digest = obj.get("digest").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let priorities = obj.get("priorities").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|a| a.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let followups = obj.get("followups").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|a| a.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    Ok(Json(AgendaDigestResult { digest, priorities, followups }))
}

// ─── Phase 4.5 — Globaler Assistent ─────────────────────────

#[derive(Deserialize)]
pub struct AssistantHistoryMsg {
    pub role: String,
    pub text: String,
}

#[derive(Deserialize)]
pub struct AssistantRequest {
    pub message: String,
    #[serde(default)]
    pub context: Option<String>,
    /// Previous turns of the same dialog (user + assistant), oldest first.
    #[serde(default)]
    pub history: Option<Vec<AssistantHistoryMsg>>,
}

#[derive(Serialize)]
pub struct AssistantAction {
    #[serde(rename = "type")]
    pub kind: String,
    pub payload: serde_json::Value,
}

#[derive(Serialize)]
pub struct AssistantResult {
    pub reply: String,
    pub actions: Vec<AssistantAction>,
}

// B5: only actions the frontend actually implements. `schedule`,
// `meeting_prep` and `agenda_digest` had no drawer handler (dead) and are
// removed from the assistant's action list. They remain available as direct
// REST endpoints (/ai/schedule, /ai/meeting-prep, /ai/agenda-digest).
const AVAILABLE_ACTIONS: &str = "event_create, task_create, find_mail, compose_mail";

/// Gather a compact, module-spanning context (upcoming events, contacts,
/// recent mails) so the assistant can answer cross-module questions without
/// being limited to the module the user opened it from.
fn gather_assistant_context(state: &AppState) -> String {
    let db = state.cache_db.lock();
    let Some(conn) = db.as_ref() else {
        return String::new();
    };
    let now = chrono::Utc::now();
    let week = now + chrono::Duration::days(7);
    let start = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let end = week.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let mut parts: Vec<String> = Vec::new();

    // Upcoming events (next 7 days, max 20).
    if let Ok(mut stmt) = conn.prepare(
        "SELECT summary, start_at, end_at, location FROM events
         WHERE start_at >= ?1 AND start_at < ?2
         ORDER BY start_at LIMIT 20",
    ) {
        if let Ok(rows) = stmt.query_map(rusqlite::params![start, end], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        }) {
            let events: Vec<String> = rows
                .filter_map(|row| {
                    row.ok().and_then(|(summary, s, e, loc)| {
                        let summary = summary.unwrap_or_else(|| "(ohne Titel)".into());
                        let end = e.unwrap_or_else(|| s.clone());
                        let loc = loc.filter(|l| !l.trim().is_empty());
                        let mut line = format!("- {} bis {}: {}", s, end, summary);
                        if let Some(l) = loc {
                            line.push_str(&format!(" ({l})"));
                        }
                        Some(line)
                    })
                })
                .collect();
            if !events.is_empty() {
                parts.push(format!("Termine (nächste 7 Tage):\n{}", events.join("\n")));
            }
        }
    }

    // Contacts (max 50). Fall back to given/family/email when display_name is
    // empty so contacts without a display name are not silently dropped.
    if let Ok(mut stmt) = conn.prepare(
        "SELECT display_name, given_name, family_name, email, phone FROM contacts
         ORDER BY display_name COLLATE NOCASE, given_name COLLATE NOCASE LIMIT 50",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        }) {
            let contacts: Vec<String> = rows
                .filter_map(|row| {
                    row.ok().and_then(|(display, given, family, email, phone)| {
                        let email = email.filter(|e| !e.trim().is_empty());
                        let phone = phone.filter(|p| !p.trim().is_empty());
                        let name = display
                            .filter(|d| !d.trim().is_empty())
                            .or_else(|| {
                                let g = given.filter(|g| !g.trim().is_empty());
                                let f = family.filter(|f| !f.trim().is_empty());
                                match (g, f) {
                                    (Some(g), Some(f)) => Some(format!("{g} {f}")),
                                    (Some(g), None) => Some(g),
                                    (None, Some(f)) => Some(f),
                                    _ => None,
                                }
                            })
                            .or_else(|| email.clone())
                            .filter(|n| !n.trim().is_empty())?;
                        let mut line = format!("- {name}");
                        if let Some(e) = &email {
                            line.push_str(&format!(" (Mail: {e})"));
                        }
                        if let Some(p) = &phone {
                            line.push_str(&format!(" (Telefon: {p})"));
                        }
                        Some(line)
                    })
                })
                .collect();
            if !contacts.is_empty() {
                parts.push(format!("Kontakte (Auszug):\n{}", contacts.join("\n")));
            }
        }
    }

    // Recent mails (max 20).
    if let Ok(mut stmt) = conn.prepare(
        "SELECT subject, from_addr, date FROM messages
         WHERE subject IS NOT NULL
         ORDER BY date DESC LIMIT 20",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        }) {
            let mails: Vec<String> = rows
                .filter_map(|row| {
                    row.ok().map(|(subject, from, date)| {
                        format!(
                            "- {}: \"{}\" von {}",
                            date.unwrap_or_default(),
                            subject,
                            from.unwrap_or_default()
                        )
                    })
                })
                .collect();
            if !mails.is_empty() {
                parts.push(format!("Letzte Mails:\n{}", mails.join("\n")));
            }
        }
    }

    parts.join("\n\n")
}

/// Search contacts by name/email fragment (case-insensitive). Used by the
/// assistant's `search_contacts` tool. Returns a formatted list or "keine Treffer".
fn search_contacts_in_db(state: &AppState, query: &str) -> String {
    let db = state.cache_db.lock();
    let Some(conn) = db.as_ref() else {
        return "Kontakte nicht verfuegbar".to_string();
    };
    let q = format!("%{}%", query.trim());
    let rows: Vec<(Option<String>, Option<String>, Option<String>, Option<String>, Option<String>)> = conn
        .prepare(
            "SELECT display_name, given_name, family_name, email, phone
             FROM contacts
             WHERE lower(COALESCE(display_name,'')) LIKE lower(?1)
                OR lower(COALESCE(given_name,'')) LIKE lower(?1)
                OR lower(COALESCE(family_name,'')) LIKE lower(?1)
                OR lower(COALESCE(email,'')) LIKE lower(?1)
             ORDER BY COALESCE(display_name, given_name, family_name, email, '') COLLATE NOCASE
             LIMIT 20",
        )
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map(rusqlite::params![q], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();
    if rows.is_empty() {
        return "keine Treffer".to_string();
    }
    rows.into_iter()
        .filter_map(|(display, given, family, email, phone)| {
            let email = email.filter(|e| !e.trim().is_empty());
            let phone = phone.filter(|p| !p.trim().is_empty());
            let name = display
                .filter(|d| !d.trim().is_empty())
                .or_else(|| {
                    let g = given.filter(|g| !g.trim().is_empty());
                    let f = family.filter(|f| !f.trim().is_empty());
                    match (g, f) {
                        (Some(g), Some(f)) => Some(format!("{g} {f}")),
                        (Some(g), None) => Some(g),
                        (None, Some(f)) => Some(f),
                        _ => None,
                    }
                })
                .or_else(|| email.clone())
                .filter(|n| !n.trim().is_empty())?;
            let mut line = format!("- {name}");
            if let Some(e) = &email {
                line.push_str(&format!(" (Mail: {e})"));
            }
            if let Some(p) = &phone {
                line.push_str(&format!(" (Telefon: {p})"));
            }
            Some(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `POST /api/v1/ai/assistant` — the global assistant (centerpiece).
pub async fn ai_assistant(
    State(state): State<AppState>,
    Json(req): Json<AssistantRequest>,
) -> ApiResult<AssistantResult> {
    let context = req.context.as_deref().unwrap_or("kein Kontext");
    let now = chrono::Utc::now().to_rfc3339();
    let data = gather_assistant_context(&state);
    let full_context = if data.is_empty() {
        context.to_string()
    } else {
        format!("{context}\n\nVerfügbare Daten:\n{data}")
    };
    // Vorherige Runden desselben Dialogs als Verlauf mitgeben, damit der
    // Assistent Zusammenhaenge ueber mehrere Nachrichten hinweg behaelt.
    let history = req
        .history
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter(|m| !m.text.trim().is_empty())
        .map(|m| format!("{}: {}", if m.role == "user" { "Nutzer" } else { "Assistent" }, m.text))
        .collect::<Vec<_>>()
        .join("\n");
    let (system, user) = prompts::build_assistant_prompt(&req.message, &full_context, AVAILABLE_ACTIONS, &now, &history);
    let client = get_ai_client(&state)?;

    // Tool loop: the LLM may call `search_contacts` to look up a specific
    // person (the context only holds a contact excerpt). We run the search and
    // feed the result back, up to 3 rounds, then expect the final answer.
    use crate::ai::client::ChatMessage;
    let mut messages = vec![
        ChatMessage::text("system", system),
        ChatMessage::text("user", user),
    ];
    let mut last_raw = String::new();
    for _ in 0..3 {
        let raw = client.complete_messages(messages.clone(), Some(0.5), Some(1500)).await?;
        last_raw = raw.clone();
        let obj = extract_json_object(&raw).unwrap_or(serde_json::Value::Null);
        let is_search = obj
            .get("tool_call")
            .and_then(|t| t.get("name"))
            .and_then(|n| n.as_str())
            == Some("search_contacts");
        if is_search {
            let query = obj
                .get("tool_call")
                .and_then(|t| t.get("query"))
                .and_then(|q| q.as_str())
                .unwrap_or("")
                .to_string();
            let results = search_contacts_in_db(&state, &query);
            messages.push(ChatMessage::text("assistant", raw));
            messages.push(ChatMessage::text(
                "user",
                format!(
                    "Ergebnis von search_contacts(\"{query}\"):\n{results}\n\n\
                     Antworte jetzt mit dem normalen JSON-Objekt {{\"reply\": ..., \"actions\": [...]}}."
                ),
            ));
            continue;
        }
        let reply = obj.get("reply").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let actions = obj
            .get("actions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| {
                        let kind = a.get("type")?.as_str()?.to_string();
                        if kind.is_empty() {
                            return None;
                        }
                        let payload = a.get("payload").cloned().unwrap_or(serde_json::Value::Null);
                        Some(AssistantAction { kind, payload })
                    })
                    .collect()
            })
            .unwrap_or_default();
        return Ok(Json(AssistantResult { reply, actions }));
    }
    // Loop exhausted without a clean final answer — return the last output so
    // the user still gets something instead of an empty reply.
    let obj = extract_json_object(&last_raw).unwrap_or(serde_json::Value::Null);
    let reply = obj.get("reply").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let reply = if reply.is_empty() { last_raw } else { reply };
    Ok(Json(AssistantResult { reply, actions: vec![] }))
}

// ── Agentic assistant v2 (Concept §5, §9) ─────────────────────────────────

/// Extract the last non-empty path segment (the id) from a prepared path.
fn last_path_segment(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_string()
}

/// Convert an `ApiResult<T>` into a JSON value (per plan-step result).
fn api_result_to_value<T: Serialize>(res: Result<Json<T>, ApiError>) -> Result<serde_json::Value, String> {
    match res {
        Ok(Json(v)) => Ok(serde_json::to_value(v).unwrap_or_default()),
        Err(e) => Err(e.0),
    }
}

/// Execute one confirmed plan step by re-invoking the matching REST handler
/// (the tested source of truth). Concept §5.3.
async fn execute_prepared(state: &AppState, step: &crate::ai::tools::PreparedCard) -> Result<serde_json::Value, String> {
    let body = step.request.body.clone();
    match step.tool.as_str() {
        "tasks_create" => {
            let req: crate::api::todos::CreateTodoRequest = serde_json::from_value(body).map_err(|e| e.to_string())?;
            api_result_to_value(crate::api::todos::create_todo(State(state.clone()), Json(req)).await)
        }
        "tasks_toggle" => {
            let uid = last_path_segment(&step.request.path);
            let req: crate::api::todos::ToggleTodoRequest = serde_json::from_value(body).map_err(|e| e.to_string())?;
            api_result_to_value(crate::api::todos::toggle_todo(State(state.clone()), Path(uid), Json(req)).await)
        }
        "tasks_delete" => {
            let uid = last_path_segment(&step.request.path);
            api_result_to_value(crate::api::todos::delete_todo(State(state.clone()), Path(uid)).await)
        }
        "calendar_create_event" => {
            let req: crate::api::calendars::CreateEventRequest = serde_json::from_value(body).map_err(|e| e.to_string())?;
            api_result_to_value(crate::api::calendars::create_event(State(state.clone()), Json(req)).await)
        }
        "calendar_update_event" => {
            let id: i64 = last_path_segment(&step.request.path).parse().map_err(|_| "Ungültige Termin-ID".to_string())?;
            let req: crate::api::calendars::UpdateEventRequest = serde_json::from_value(body).map_err(|e| e.to_string())?;
            api_result_to_value(crate::api::calendars::update_event(State(state.clone()), Path(id), Json(req)).await)
        }
        "calendar_delete_event" => {
            let id: i64 = last_path_segment(&step.request.path).parse().map_err(|_| "Ungültige Termin-ID".to_string())?;
            api_result_to_value(crate::api::calendars::delete_event(State(state.clone()), Path(id)).await)
        }
        "contacts_create" => {
            let req: crate::api::contacts::CreateContactRequest = serde_json::from_value(body).map_err(|e| e.to_string())?;
            api_result_to_value(crate::api::contacts::create_contact(State(state.clone()), Json(req)).await)
        }
        "mail_propose_reply" => {
            let req: crate::api::send::SaveDraftRequest = serde_json::from_value(body).map_err(|e| e.to_string())?;
            api_result_to_value(crate::api::send::save_draft(State(state.clone()), Json(req)).await)
        }
        "mail_flag" => {
            let req: crate::api::messages::FlagRequest = serde_json::from_value(body).map_err(|e| e.to_string())?;
            api_result_to_value(crate::api::messages::flag_message(State(state.clone()), Json(req)).await)
        }
        "mail_move" => {
            let req: crate::api::messages::MoveMessageRequest = serde_json::from_value(body).map_err(|e| e.to_string())?;
            api_result_to_value(crate::api::messages::move_message(State(state.clone()), Json(req)).await)
        }
        "calendar_invite" | "calendar_rsvp" => Err(format!(
            "'{}' wird noch nicht automatisch ausgeführt (IMIP, Phase C+).",
            step.tool
        )),
        other => Err(format!("Unbekanntes Tool '{other}' — Schritt nicht ausführbar.")),
    }
}

/// `POST /api/v1/ai/agent` — the agentic assistant (Concept §5.2, §9.1).
/// Streams `status`/`plan`/`effect`/`done` as SSE when the client accepts
/// `text/event-stream`; otherwise returns the final result as JSON.
pub async fn agent_stream(State(state): State<AppState>, headers: HeaderMap, Json(req): Json<AgentRequest>) -> Response {
    let wants_sse = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.contains("text/event-stream"))
        .unwrap_or(false);

    if !wants_sse {
        match agent::run_agent(&state, &req, None).await {
            Ok(result) => return Json(result).into_response(),
            Err(e) => return ApiError(e).into_response(),
        }
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
    let state2 = state.clone();
    let req2 = req.clone();
    tokio::spawn(async move {
        let _ = agent::run_agent(&state2, &req2, Some(tx)).await;
    });

    let stream = async_stream::stream! {
        while let Some(ev) = rx.recv().await {
            let done = matches!(ev, AgentEvent::Done { .. });
            let (name, data) = match ev {
                AgentEvent::Status { step, label } => ("status", serde_json::json!({"step": step, "label": label})),
                AgentEvent::Plan { plan } => ("plan", serde_json::to_value(&plan).unwrap_or_default()),
                AgentEvent::Effect { effect } => ("effect", effect),
                AgentEvent::Done { result } => ("done", serde_json::to_value(&result).unwrap_or_default()),
            };
            yield Ok::<_, Infallible>(Event::default().event(name).data(data.to_string()));
            if done {
                break;
            }
        }
    };
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

/// Phase C — the body of `POST /api/v1/ai/plans`: a typed follow-up suggestion
/// (tool + args) the user picked from the mail footer.
#[derive(Deserialize)]
pub struct PlanCreateRequest {
    pub tool: String,
    pub args: serde_json::Value,
    #[serde(default)]
    pub source_message_id: Option<i64>,
    #[serde(default)]
    pub locale: Option<String>,
}

/// `POST /api/v1/ai/plans` — build a pending plan from a follow-up suggestion
/// (Phase C, Concept §12.3, §9.4). The server re-runs the tool to build the
/// prepared card (re-validating against the registry), then persists a pending
/// plan with origin `mail_followup`. Nothing is executed — the user confirms
/// the plan separately (Concept §5.3).
pub async fn plan_create(
    State(state): State<AppState>,
    Json(req): Json<PlanCreateRequest>,
) -> ApiResult<plaene::ActionPlan> {
    // Registry validation: unknown tools are rejected (anti-injection).
    if crate::ai::tools::find_tool(&req.tool).is_none() {
        return Err(ApiError(format!("Unbekanntes Tool '{}'.", req.tool)));
    }
    let locale = req.locale.clone().unwrap_or_else(|| "de".to_string());
    let source_message_id = req.source_message_id;

    // Seed the source message so mail_propose_reply passes the ID-provenance check.
    let label = source_message_id
        .and_then(|id| {
            with_db(&state, |conn| {
                conn.query_row(
                    "SELECT subject FROM messages WHERE uid = ?1 LIMIT 1",
                    rusqlite::params![id],
                    |r| Ok(r.get::<_, Option<String>>(0)?),
                )
                .map_err(|e| e.to_string())
            })
            .ok()
            .flatten()
        })
        .unwrap_or_default();
    let known_ids = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
    if let Some(id) = source_message_id {
        let mut g = known_ids.lock().await;
        g.insert(id.to_string(), label);
    }
    let ctx = crate::ai::tools::ToolCtx {
        state: state.clone(),
        locale,
        known_ids,
    };

    let card = match crate::ai::tools::execute(&req.tool, req.args, ctx).await {
        Ok(crate::ai::tools::ToolOutcome::Card(card)) => card,
        Ok(_) => return Err(ApiError("Der Vorschlag erzeugt keine Bestätigungskarte.".to_string())),
        Err(e) => return Err(ApiError(e)),
    };

    let plan = plaene::build_plan(None, "mail_followup", source_message_id, vec![card]);
    with_db(&state, |conn| {
        plaene::create_plan(conn, &plan)?;
        crate::ai::audit::log_plan(conn, None, "mail_followup", &plan.id, "created")
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .map_err(ApiError)?;
    Ok(Json(plan))
}

/// `POST /api/v1/ai/plans/:id/confirm` — execute a pending plan (Concept §5.3).
pub async fn plan_confirm(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<serde_json::Value> {
    let plan = with_db(&state, |conn| plaene::get_plan(conn, &id))?
        .ok_or_else(|| ApiError("Plan nicht gefunden".to_string()))?;
    if plan.status != plaene::PlanStatus::Pending {
        return Err(ApiError(format!("Plan ist nicht mehr ausstehend ({}).", plan.status.as_str())));
    }
    let mut results = Vec::new();
    let mut failed = false;
    for step in &plan.steps {
        match execute_prepared(&state, step).await {
            Ok(v) => results.push(v),
            Err(e) => {
                failed = true;
                results.push(serde_json::json!({ "error": e }));
            }
        }
    }
    let status = if failed { plaene::PlanStatus::Failed } else { plaene::PlanStatus::Executed };
    with_db(&state, |conn| {
        plaene::set_status(conn, &id, status)?;
        audit::log_plan(conn, plan.session_id.as_deref(), &plan.origin, &id, status.as_str())
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "plan_id": id, "status": status.as_str(), "results": results })))
}

/// `POST /api/v1/ai/plans/:id/cancel` — cancel a pending plan (no undo needed).
pub async fn plan_cancel(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<serde_json::Value> {
    let plan = with_db(&state, |conn| plaene::get_plan(conn, &id))?
        .ok_or_else(|| ApiError("Plan nicht gefunden".to_string()))?;
    let cancelled = with_db(&state, |conn| plaene::cancel_plan(conn, &id)).map_err(ApiError)?;
    if cancelled {
        let _ = with_db(&state, |conn| {
            audit::log_plan(conn, plan.session_id.as_deref(), &plan.origin, &id, "cancelled")
                .map_err(|e| e.to_string())
        });
    }
    Ok(Json(serde_json::json!({ "plan_id": id, "cancelled": cancelled })))
}

/// `POST /api/v1/ai/plans/:id/undo` — revert an executed plan (best effort).
pub async fn plan_undo(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<serde_json::Value> {
    let plan = with_db(&state, |conn| plaene::get_plan(conn, &id))?
        .ok_or_else(|| ApiError("Plan nicht gefunden".to_string()))?;
    if plan.status != plaene::PlanStatus::Executed {
        return Err(ApiError(format!("Nur ausgeführte Pläne sind rückgängig machbar ({}).", plan.status.as_str())));
    }
    // Best-effort inverse operations (Concept §5.3). Only creation steps are
    // undone by deleting the created entity; the rest are reported.
    let mut undone = Vec::new();
    for step in &plan.steps {
        match step.tool.as_str() {
            "tasks_create" => {
                let uid = last_path_segment(&step.request.path);
                if !uid.is_empty() {
                    let _ = crate::api::todos::delete_todo(State(state.clone()), Path(uid)).await;
                    undone.push(serde_json::json!({ "tool": step.tool, "undone": true }));
                }
            }
            "calendar_create_event" => {
                let id_s = last_path_segment(&step.request.path);
                if let Ok(id) = id_s.parse::<i64>() {
                    let _ = crate::api::calendars::delete_event(State(state.clone()), Path(id)).await;
                    undone.push(serde_json::json!({ "tool": step.tool, "undone": true }));
                }
            }
            "contacts_create" => {
                undone.push(serde_json::json!({ "tool": step.tool, "undone": false, "note": "Kontakt manuell löschen" }));
            }
            other => undone.push(serde_json::json!({ "tool": other, "undone": false, "note": "manuell rückgängig machen" })),
        }
    }
    let _ = with_db(&state, |conn| {
        audit::log_plan(conn, plan.session_id.as_deref(), &plan.origin, &id, "undone").map_err(|e| e.to_string())
    });
    Ok(Json(serde_json::json!({ "plan_id": id, "undone": undone })))
}

/// `GET /api/v1/ai/sessions/:id` — fetch a session (history for the drawer).
pub async fn session_get(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<serde_json::Value> {
    let sess = with_db(&state, |conn| sessions::get_session(conn, &id))?
        .ok_or_else(|| ApiError("Session nicht gefunden".to_string()))?;
    Ok(Json(serde_json::to_value(&sess).unwrap_or_default()))
}

/// `DELETE /api/v1/ai/sessions/:id` — remove a session and its history.
pub async fn session_delete(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<serde_json::Value> {
    let deleted = with_db(&state, |conn| {
        let n = conn
            .execute("DELETE FROM ai_sessions WHERE id = ?1", rusqlite::params![id])
            .map_err(|e| e.to_string())?;
        Ok(n > 0)
    })
    .map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_array_plain() {
        let raw = r#"[{"start":"2026-09-01T10:00:00Z","end":"2026-09-01T11:00:00Z"}]"#;
        let arr = extract_json_array(raw);
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].get("start").and_then(|v| v.as_str()), Some("2026-09-01T10:00:00Z"));
    }

    #[test]
    fn test_extract_json_array_markdown_wrapped() {
        let raw = "Hier sind die Vorschläge:\n```json\n[{\"start\":\"a\",\"end\":\"b\"}]\n```\nFertig.";
        let arr = extract_json_array(raw);
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].get("start").and_then(|v| v.as_str()), Some("a"));
    }

    #[test]
    fn test_extract_json_array_invalid() {
        assert!(extract_json_array("kein json hier").is_empty());
        assert!(extract_json_array("[]").is_empty());
    }

    #[test]
    fn test_extract_json_object_markdown_wrapped() {
        let raw = "```json\n{\"summary\":\"Meeting\",\"start\":\"2026-09-01T10:00:00Z\",\"all_day\":false}\n```";
        let obj = extract_json_object(raw).unwrap();
        assert_eq!(obj.get("summary").and_then(|v| v.as_str()), Some("Meeting"));
        assert_eq!(obj.get("all_day").and_then(|v| v.as_bool()), Some(false));
    }

    // B2: no object → loud error with an excerpt, never a silent Null.
    #[test]
    fn test_extract_json_object_invalid_fails_loudly() {
        let err = extract_json_object("kein objekt").unwrap_err();
        assert!(err.contains("kein objekt"), "expected excerpt, got: {err}");
    }

    // B2: braces present but invalid JSON → loud parse error.
    #[test]
    fn test_extract_json_object_malformed_fails_loudly() {
        let err = extract_json_object("{\"a\": 1,}").unwrap_err();
        assert!(err.contains("Ungültiges JSON"), "got: {err}");
    }

    // B2: fence without a language tag is unwrapped too.
    #[test]
    fn test_extract_json_object_plain_fence() {
        let raw = "Ergebnis:\n```\n{\"ok\":true}\n```\nFertig.";
        let obj = extract_json_object(raw).unwrap();
        assert_eq!(obj.get("ok").and_then(|v| v.as_bool()), Some(true));
    }

    // B2: outermost braces win when the reply has prose around the object.
    #[test]
    fn test_extract_json_object_prose_around() {
        let raw = "Hier ist das Objekt: {\"x\":{\"y\":1}} — hoffentlich passt das.";
        let obj = extract_json_object(raw).unwrap();
        assert_eq!(obj.get("x").and_then(|v| v.get("y")).and_then(|v| v.as_i64()), Some(1));
    }

    // ── Phase C: Followups v2 ─────────────────────────────────────────────

    /// The v2 prompt clamps the mail body between explicit markers, echoes the
    /// registry tool names, and carries the anti-injection contract (de + en).
    #[test]
    fn v2_prompt_clamps_body_and_anti_injection() {
        let (de_sys, de_user) =
            crate::ai::prompts::build_followups_v2_prompt("Betreff", "kai@x.de", "Mach X sofort", 42, "2026-09-07T00:00:00Z", "de");
        assert!(de_user.contains("=== MAIL 42 BEGIN ==="), "body must be clamped");
        assert!(de_user.contains("=== MAIL 42 END ==="), "body must be clamped");
        assert!(de_user.contains("Mach X sofort"), "clamped body carries the content");
        assert!(de_sys.contains("tasks_create") && de_sys.contains("mail_propose_reply") && de_sys.contains("calendar_create_event"));
        assert!(de_sys.contains("reine DATEN"), "de must state the body is data");

        let (en_sys, en_user) =
            crate::ai::prompts::build_followups_v2_prompt("Subject", "kai@x.de", "Do X now", 42, "2026-09-07T00:00:00Z", "en");
        assert!(en_user.contains("=== MAIL 42 BEGIN ==="));
        assert!(en_sys.contains("pure DATA"), "en must state the body is data");
        assert!(en_sys.contains("tasks_create"));
    }

    /// A valid tasks_create suggestion becomes one suggestion with a prepared card.
    #[tokio::test]
    async fn parse_v2_tasks_create_builds_card() {
        let state = AppState::new();
        let raw = r#"{"suggestions":[{"tool":"tasks_create","args":{"summary":"Rückruf bis morgen 12","due":"2026-09-08T12:00:00Z"},"titel":"Rückruf bis 12"}]}"#;
        let out = parse_followups_v2(&state, raw, 42, "Betreff", "de").await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tool, "tasks_create");
        assert_eq!(out[0].titel, "Rückruf bis 12");
        assert_eq!(out[0].plan.tool, "tasks_create");
        assert_eq!(out[0].plan.tier, crate::ai::tools::Tier::Write);
        assert_eq!(out[0].plan.request.method, "POST");
    }

    /// Anti-injection fixpoint (Concept §6.5): a suggestion naming an unknown
    /// tool is discarded — never executed, never surfaced.
    #[tokio::test]
    async fn parse_v2_unknown_tool_discarded() {
        let state = AppState::new();
        let raw = r#"{"suggestions":[{"tool":"delete_all_data","args":{"target":"*"},"titel":"Alles löschen"}]}"#;
        let out = parse_followups_v2(&state, raw, 42, "Betreff", "de").await;
        assert!(out.is_empty(), "unknown tools must be discarded");
    }

    /// A mix of valid and invalid suggestions keeps only the valid ones.
    #[tokio::test]
    async fn parse_v2_mixed_keeps_only_valid() {
        let state = AppState::new();
        let raw = r#"{"suggestions":[
            {"tool":"evil_tool","args":{},"titel":"x"},
            {"tool":"tasks_create","args":{"summary":"Gültig"},"titel":"Gültig"},
            {"tool":"tasks_create","args":{"summary":"Auch gültig"},"titel":"Zwei"}
        ]}"#;
        let out = parse_followups_v2(&state, raw, 42, "Betreff", "de").await;
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|s| s.tool == "tasks_create"));
    }

    /// mail_propose_reply must target the source message: the LLM's id is
    /// overridden with the real source uid (ID-provenance, Concept §6.3).
    #[tokio::test]
    async fn parse_v2_mail_reply_id_overridden() {
        let state = AppState::new();
        let raw = r#"{"suggestions":[{"tool":"mail_propose_reply","args":{"id":"9999","body":"Danke, passt."},"titel":"Antwort"}]}"#;
        let out = parse_followups_v2(&state, raw, 42, "Betreff", "de").await;
        assert_eq!(out.len(), 1);
        let id = out[0].plan.request.body.get("body").is_some(); // body present
        assert!(id);
        // The stored args carry the overridden id.
        assert_eq!(out[0].args.get("id").and_then(|v| v.as_str()), Some("42"));
    }

    /// Alt-format tolerance (Concept §12.3): a legacy bare-array cache value
    /// does not parse as the v2 object → treated as a cache miss.
    #[test]
    fn followups_cache_alt_format_is_miss() {
        let legacy = r#"[{"id":"fu-1","kind":"task","label":"x","task":{"summary":"x"}}]"#;
        assert!(serde_json::from_str::<FollowupsResponse>(legacy).is_err(), "old array format must not parse as the v2 object");
        let v2 = r#"{"actions":[]}"#;
        assert!(serde_json::from_str::<FollowupsResponse>(v2).is_ok(), "v2 object must parse");
    }

    /// AppState with the agent plan table (for plan_create persistence tests).
    fn plan_state() -> AppState {
        let state = AppState::new();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE ai_action_plans (
                 id TEXT PRIMARY KEY, session_id TEXT, origin TEXT NOT NULL,
                 source_message_id INTEGER, status TEXT NOT NULL, steps_json TEXT NOT NULL,
                 created_at TEXT NOT NULL, expires_at TEXT NOT NULL, executed_at TEXT, result_json TEXT);
             CREATE TABLE ai_audit (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 session_id TEXT, origin TEXT, event TEXT NOT NULL,
                 detail TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')));",
        )
        .unwrap();
        *state.cache_db.lock() = Some(conn);
        state
    }

    /// plan_create builds a pending plan with origin `mail_followup` and the
    /// prepared card as its single step (nothing is executed).
    #[tokio::test]
    async fn plan_create_builds_mail_followup_plan() {
        let state = plan_state();
        let req = PlanCreateRequest {
            tool: "tasks_create".into(),
            args: serde_json::json!({ "summary": "Rückruf", "due": "2026-09-08T12:00:00Z" }),
            source_message_id: Some(42),
            locale: Some("de".into()),
        };
        let Json(plan) = plan_create(State(state.clone()), Json(req)).await.unwrap();
        assert_eq!(plan.origin, "mail_followup");
        assert_eq!(plan.status, plaene::PlanStatus::Pending);
        assert_eq!(plan.source_message_id, Some(42));
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].tool, "tasks_create");
        // Persisted and retrievable.
        let got = with_db(&state, |conn| plaene::get_plan(conn, &plan.id)).unwrap().unwrap();
        assert_eq!(got.origin, "mail_followup");
    }

    /// plan_create rejects unknown tools (anti-injection at the endpoint).
    #[tokio::test]
    async fn plan_create_unknown_tool_rejected() {
        let state = plan_state();
        let req = PlanCreateRequest {
            tool: "drop_database".into(),
            args: serde_json::json!({}),
            source_message_id: None,
            locale: None,
        };
        let res = plan_create(State(state.clone()), Json(req)).await;
        assert!(res.is_err(), "unknown tool must be rejected");
    }
}
