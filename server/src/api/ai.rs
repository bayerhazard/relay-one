//! AI endpoints: reply generation, summarization, drafting, formatting,
//! priority/fraud detection, tone profiles, recipient/subject suggestions.
//!
//! Ported from the Tauri-era `ai_*` commands in `ipc.rs`. Business logic is
//! identical; only the transport changed from Tauri IPC to axum handlers.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::ai::audit;
use crate::ai::language::detect_language;
use crate::ai::prompts::{self, EmailMode};
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
                   AND ai_summary IS NULL AND body_text IS NOT NULL
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

/// Pull the first JSON object out of a (possibly markdown-wrapped) LLM reply.
fn extract_json_object(raw: &str) -> serde_json::Value {
    let (Some(s), Some(e)) = (raw.find('{'), raw.rfind('}')) else {
        return serde_json::Value::Null;
    };
    if e < s {
        return serde_json::Value::Null;
    }
    serde_json::from_str(&raw[s..=e]).unwrap_or(serde_json::Value::Null)
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

#[derive(Serialize)]
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
    let obj = extract_json_object(&raw);
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

/// A single suggested follow-up action derived from a message.
#[derive(Serialize, Deserialize)]
pub struct FollowupItem {
    pub task: String,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Deserialize)]
pub struct FollowupsRequest {
    pub subject: String,
    pub from: String,
    pub body: String,
}

/// `POST /api/v1/ai/followups` — suggest follow-up actions for a message.
pub async fn ai_followups(
    State(state): State<AppState>,
    Json(req): Json<FollowupsRequest>,
) -> ApiResult<Vec<FollowupItem>> {
    let (system, user) = prompts::build_followups_prompt(&req.subject, &req.from, &req.body);
    let client = get_ai_client(&state)?;
    let raw = client.complete_user(&system, &user, Some(0.4), Some(800)).await?;
    let arr = extract_json_array(&raw);
    let items: Vec<FollowupItem> = arr
        .into_iter()
        .filter_map(|v| {
            let task = v.get("task")?.as_str()?.trim().to_string();
            if task.is_empty() {
                return None;
            }
            Some(FollowupItem {
                due: v.get("due").and_then(|d| d.as_str()).map(str::to_string),
                reason: v.get("reason").and_then(|r| r.as_str()).map(str::to_string),
                task,
            })
        })
        .take(5)
        .collect();
    Ok(Json(items))
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
        let obj = extract_json_object(raw);
        assert_eq!(obj.get("summary").and_then(|v| v.as_str()), Some("Meeting"));
        assert_eq!(obj.get("all_day").and_then(|v| v.as_bool()), Some(false));
    }

    #[test]
    fn test_extract_json_object_invalid() {
        assert!(extract_json_object("kein objekt").is_null());
    }
}
