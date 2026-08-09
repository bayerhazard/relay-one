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
