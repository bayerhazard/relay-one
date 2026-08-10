//! Settings endpoints (AI config, move-to-trash).

use axum::extract::State;
use axum::Json;
use serde::Deserialize;

use crate::ai::client::{AIClient, AIConfig};
use crate::api::ApiError;
use crate::cache;
use crate::crypto;
use crate::db::with_db;
use crate::AppState;

use super::{ApiResult, ok};

#[derive(Deserialize)]
pub struct SaveSettingsRequest {
    pub url: String,
    pub api_key: String,
    pub model: String,
}

/// `POST /api/v1/settings` — persist AI settings.
pub async fn save_settings(
    State(state): State<AppState>,
    Json(req): Json<SaveSettingsRequest>,
) -> ApiResult<()> {
    let config = AIConfig {
        url: req.url,
        api_key: req.api_key,
        model: req.model,
        ..Default::default()
    };
    *state.ai_config.write() = Some(config.clone());
    *state.ai_client.write() = Some(std::sync::Arc::new(AIClient::new(config.clone())));

    with_db(&state, |conn| {
        cache::settings::set_setting(conn, "ai_url", &config.url)
            .map_err(|e: rusqlite::Error| e.to_string())?;
        cache::settings::set_setting(conn, "ai_model", &config.model)
            .map_err(|e: rusqlite::Error| e.to_string())?;
        let encrypted_key =
            crypto::encrypt(&config.api_key).unwrap_or_else(|_| config.api_key.clone());
        cache::settings::set_setting(conn, "api_key", &encrypted_key)
            .map_err(|e: rusqlite::Error| e.to_string())?;
        Ok(())
    })?;

    *state.cached_settings.lock() = None;
    Ok(Json(()))
}

/// `GET /api/v1/settings` — return AI settings.
pub async fn get_settings(State(state): State<AppState>) -> ApiResult<Option<AIConfig>> {
    {
        let cached = state.cached_settings.lock();
        if let Some(ref config) = *cached {
            return Ok(Json(Some(config.clone())));
        }
    }
    {
        let config_opt = state.ai_config.read().clone();
        if config_opt.is_some() {
            return Ok(Json(config_opt));
        }
    }
    let loaded_opt = with_db(&state, |conn| {
        if let Ok(Some(url)) = cache::settings::get_setting(conn, "ai_url") {
            let model = cache::settings::get_setting(conn, "ai_model")
                .ok()
                .flatten()
                .unwrap_or_else(|| "llama3.2".into());
            let stored_key = cache::settings::get_setting(conn, "api_key")
                .ok()
                .flatten()
                .unwrap_or_else(|| "ollama".into());
            let api_key = crypto::decrypt(&stored_key).unwrap_or(stored_key);
            let config = AIConfig { url, api_key, model, ..Default::default() };
            Ok(Some(config))
        } else {
            Ok(None)
        }
    })?;
    if let Some(ref config) = loaded_opt {
        *state.ai_config.write() = Some(config.clone());
        *state.ai_client.write() = Some(std::sync::Arc::new(AIClient::new(config.clone())));
        *state.cached_settings.lock() = Some(config.clone());
    }
    Ok(Json(loaded_opt))
}

/// `GET /api/v1/settings/move-to-trash`
pub async fn get_move_to_trash(State(state): State<AppState>) -> ApiResult<bool> {
    ok(with_db(&state, |conn| {
        cache::settings::get_move_to_trash(conn).map_err(|e| e.to_string())
    }))
}

/// `POST /api/v1/settings/move-to-trash`
pub async fn set_move_to_trash(
    State(state): State<AppState>,
    Json(enabled): Json<bool>,
) -> ApiResult<()> {
    ok(with_db(&state, |conn| {
        cache::settings::set_move_to_trash(conn, enabled).map_err(|e| e.to_string())
    }))
}

// ─── CardDAV ─────────────────────────────────────────────────────

/// `GET /api/v1/carddav/settings` — stored CardDAV connection settings.
pub async fn get_carddav_settings(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let json = with_db(&state, |conn| {
        cache::settings::get_setting(conn, "carddav_settings").map_err(|e| e.to_string())
    })?;
    match json {
        Some(raw) => {
            let mut settings: crate::carddav::CardDavSettings =
                serde_json::from_str(&raw).map_err(|e| ApiError(format!("CardDAV parse: {e}")))?;
            settings.password = crate::crypto::decrypt(&settings.password).unwrap_or(settings.password);
            Ok(Json(serde_json::json!({
                "url": settings.url, "username": settings.username, "password": settings.password,
                "sync_interval_minutes": settings.sync_interval_minutes,
            })))
        }
        None => Ok(Json(serde_json::json!({
            "url": "", "username": "", "password": "", "sync_interval_minutes": 30,
        }))),
    }
}

/// `POST /api/v1/carddav/settings` — save the CardDAV connection settings.
#[derive(Deserialize)]
pub struct CardDavSettingsRequest {
    pub url: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub sync_interval_minutes: Option<u64>,
}

pub async fn set_carddav_settings(
    State(state): State<AppState>,
    Json(req): Json<CardDavSettingsRequest>,
) -> ApiResult<serde_json::Value> {
    let encrypted_pw = crate::crypto::encrypt(&req.password).unwrap_or_else(|_| req.password.clone());
    let settings = crate::carddav::CardDavSettings {
        url: req.url.clone(),
        username: req.username.clone(),
        password: encrypted_pw,
        sync_interval_minutes: req.sync_interval_minutes.unwrap_or(30),
    };
    let raw = serde_json::to_string(&settings).map_err(|e| ApiError(e.to_string()))?;
    with_db(&state, |conn| {
        cache::settings::set_setting(conn, "carddav_settings", &raw).map_err(|e| e.to_string())
    })?;
    // Update the in-memory state so the scheduler picks it up.
    let mut live = settings.clone();
    live.password = req.password.clone();
    *state.carddav_settings.write() = Some(live);
    tracing::info!("CardDAV-Einstellungen gespeichert: {}", req.url);
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// `POST /api/v1/carddav/sync` — trigger a manual CardDAV sync.
pub async fn sync_carddav(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let settings = state.carddav_settings.read().clone();
    let Some(settings) = settings else {
        return Err(ApiError("CardDAV nicht konfiguriert".into()));
    };
    let client = crate::carddav::client::CardDavClient::new(settings);
    let token = state.carddav_sync_token.read().clone();
    match client.sync_incremental(&token).await {
        Ok((contacts, _deleted, new_token)) => {
            *state.carddav_sync_token.write() = new_token.clone();
            with_db(&state, |conn| {
                crate::cache::settings::set_setting(conn, "carddav_sync_token", &new_token)
                    .map_err(|e| e.to_string())
            })?;
            Ok(Json(serde_json::json!({ "ok": true, "synced": contacts.len() })))
        }
        Err(e) => Err(ApiError(format!("CardDAV-Sync fehlgeschlagen: {e}"))),
    }
}

/// `POST /api/v1/carddav/search` — search the locally synced contacts by
/// name or email address (recipient autocomplete).
#[derive(Deserialize)]
pub struct CardDavSearchRequest {
    pub query: String,
}

pub async fn search_carddav(
    State(state): State<AppState>,
    Json(req): Json<CardDavSearchRequest>,
) -> ApiResult<serde_json::Value> {
    let q = req.query.trim().to_lowercase();
    if q.len() < 2 {
        return ok(Ok(serde_json::json!([])));
    }
    let results = with_db(&state, |conn| {
        let like = format!("%{}%", q.replace('%', "\\%").replace('_', "\\_"));
        let mut stmt = conn
            .prepare(
                "SELECT vcard_uid, given_name, family_name, display_name, email, phone, organization
                 FROM contacts
                 WHERE lower(display_name) LIKE ?1 ESCAPE '\\'
                    OR lower(given_name) LIKE ?1 ESCAPE '\\'
                    OR lower(family_name) LIKE ?1 ESCAPE '\\'
                    OR lower(email) LIKE ?1 ESCAPE '\\'
                 ORDER BY display_name COLLATE NOCASE
                 LIMIT 20",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![like], |row| {
                Ok(serde_json::json!({
                    "vcard_uid": row.get::<_, String>(0)?,
                    "given_name": row.get::<_, Option<String>>(1)?,
                    "family_name": row.get::<_, Option<String>>(2)?,
                    "display_name": row.get::<_, Option<String>>(3)?,
                    "email": row.get::<_, Option<String>>(4)?,
                    "phone": row.get::<_, Option<String>>(5)?,
                    "organization": row.get::<_, Option<String>>(6)?,
                }))
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    })?;
    ok(Ok(serde_json::json!(results)))
}

/// `POST /api/v1/carddav/resolve` — resolve a free-text recipient input to a
/// single best-matching synced contact (used for AI-assisted addressing).
#[derive(Deserialize)]
pub struct CardDavResolveRequest {
    pub text: String,
}

pub async fn resolve_carddav(
    State(state): State<AppState>,
    Json(req): Json<CardDavResolveRequest>,
) -> ApiResult<serde_json::Value> {
    let text = req.text.trim().to_lowercase();
    if text.is_empty() {
        return ok(Ok(serde_json::json!(null)));
    }
    let results = with_db(&state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT vcard_uid, given_name, family_name, display_name, email, phone, organization
                 FROM contacts
                 WHERE lower(email) = ?1
                    OR lower(display_name) = ?1
                    OR lower(display_name) LIKE ?2 ESCAPE '\\'
                    OR lower(email) LIKE ?2 ESCAPE '\\'
                 ORDER BY CASE WHEN lower(email) = ?1 OR lower(display_name) = ?1 THEN 0 ELSE 1 END
                 LIMIT 1",
            )
            .map_err(|e| e.to_string())?;
        let like = format!("%{}%", text.replace('%', "\\%").replace('_', "\\_"));
        let mut rows = stmt
            .query_map(rusqlite::params![text, like], |row| {
                Ok(serde_json::json!({
                    "vcard_uid": row.get::<_, String>(0)?,
                    "given_name": row.get::<_, Option<String>>(1)?,
                    "family_name": row.get::<_, Option<String>>(2)?,
                    "display_name": row.get::<_, Option<String>>(3)?,
                    "email": row.get::<_, Option<String>>(4)?,
                    "phone": row.get::<_, Option<String>>(5)?,
                    "organization": row.get::<_, Option<String>>(6)?,
                }))
            })
            .map_err(|e| e.to_string())?;
        let first = rows.next().transpose().map_err(|e| e.to_string())?;
        Ok(first)
    })?;
    ok(Ok(serde_json::json!(results)))
}

/// `POST /api/v1/cache/clear-ai-summaries` — delete ALL AI summaries
/// (ai_summary, ai_priority, ai_fraud_score) so they are regenerated.
/// Optionally scoped to one account via `account_id`.
#[derive(Deserialize)]
pub struct ClearAiSummariesRequest {
    #[serde(default)]
    pub account_id: Option<u32>,
}

pub async fn clear_ai_summaries(
    State(state): State<AppState>,
    Json(req): Json<ClearAiSummariesRequest>,
) -> crate::api::ApiResult<serde_json::Value> {
    let cleared = with_db(&state, |conn| {
        match req.account_id {
            Some(account_id) => conn
                .execute(
                    "UPDATE messages SET ai_summary = NULL, ai_priority = NULL, ai_fraud_score = NULL
                     WHERE account_id = ?1 AND ai_summary IS NOT NULL",
                    rusqlite::params![account_id as i64],
                )
                .map_err(|e| e.to_string()),
            None => conn
                .execute(
                    "UPDATE messages SET ai_summary = NULL, ai_priority = NULL, ai_fraud_score = NULL
                     WHERE ai_summary IS NOT NULL",
                    [],
                )
                .map_err(|e| e.to_string()),
        }
    })?;
    tracing::info!("KI-Zusammenfassungen gelöscht ({} Mails)", cleared);
    Ok(Json(serde_json::json!({ "ok": true, "cleared": cleared })))
}
