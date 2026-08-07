//! Settings endpoints (AI config, move-to-trash).

use axum::extract::State;
use axum::Json;
use serde::Deserialize;

use crate::ai::client::{AIClient, AIConfig};
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
