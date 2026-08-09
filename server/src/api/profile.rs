//! Profile photo + Voice (STT) endpoints.
//!
//! These were part of the desktop relay and were never ported to the web
//! server — the frontend called the routes but the server returned 404/421.

use axum::extract::State;
use base64::Engine as _;
use axum::Json;
use serde::Deserialize;

use crate::db::with_db;
use crate::AppState;
use crate::api::{ApiError, ApiResult};

// ─── Profile photo ────────────────────────────────────────────

/// `GET /api/v1/profile/photo` — the user's own profile photo (base64).
pub async fn get_own_photo(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let (data, typ) = with_db(&state, |conn| {
        let data: Option<Vec<u8>> = conn
            .query_row("SELECT photo_data FROM settings WHERE key = 'own_photo'", [], |r| r.get(0))
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(|e| e.to_string())?;
        let typ: Option<String> = conn
            .query_row("SELECT photo_type FROM settings WHERE key = 'own_photo'", [], |r| r.get(0))
            .unwrap_or(None);
        Ok::<_, String>((data, typ))
    })?;
    match (data, typ) {
        (Some(bytes), Some(t)) => Ok(Json(serde_json::json!({
            "data": base64::engine::general_purpose::STANDARD.encode(bytes),
            "type": t,
        }))),
        _ => Err(ApiError("Kein Profilbild hinterlegt".into())),
    }
}

/// `POST /api/v1/profile/photo` — save the profile photo.
#[derive(Deserialize)]
pub struct SavePhotoRequest {
    pub photo_base64: String,
    pub photo_type: String,
}

pub async fn save_own_photo(
    State(state): State<AppState>,
    Json(req): Json<SavePhotoRequest>,
) -> ApiResult<serde_json::Value> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(req.photo_base64.as_bytes())
        .map_err(|e| ApiError(format!("Base64-Fehler: {e}")))?;
    with_db(&state, |conn| {
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('own_photo', '1')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE settings SET photo_data = ?1, photo_type = ?2 WHERE key = 'own_photo'",
            rusqlite::params![bytes, req.photo_type],
        )
        .map_err(|e| e.to_string())
    })?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ─── Voice / Speech-to-Text ───────────────────────────────────

/// `GET /api/v1/voice/config` — STT configuration.
pub async fn get_voice_config(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let (enabled, stt_url, stt_key, stt_model) = with_db(&state, |conn| {
        let enabled: i64 = conn
            .query_row("SELECT enabled FROM voice_settings WHERE id = 1", [], |r| r.get(0))
            .unwrap_or(0);
        let stt_url: String = conn
            .query_row("SELECT stt_url FROM voice_settings WHERE id = 1", [], |r| r.get(0))
            .unwrap_or_default();
        let stt_key: String = conn
            .query_row("SELECT stt_key FROM voice_settings WHERE id = 1", [], |r| r.get(0))
            .unwrap_or_default();
        let stt_model: String = conn
            .query_row("SELECT stt_model FROM voice_settings WHERE id = 1", [], |r| r.get(0))
            .unwrap_or_default();
        Ok::<_, String>((enabled, stt_url, stt_key, stt_model))
    })?;
    Ok(Json(serde_json::json!({
        "enabled": enabled != 0,
        "stt_url": stt_url,
        "stt_key": stt_key,
        "stt_model": stt_model,
    })))
}

/// `POST /api/v1/voice/config` — save STT configuration.
#[derive(Deserialize)]
pub struct VoiceConfigRequest {
    pub enabled: bool,
    pub stt_url: Option<String>,
    pub stt_key: Option<String>,
    pub stt_model: Option<String>,
}

pub async fn save_voice_config(
    State(state): State<AppState>,
    Json(req): Json<VoiceConfigRequest>,
) -> ApiResult<serde_json::Value> {
    with_db(&state, |conn| {
        conn.execute(
            "INSERT INTO voice_settings (id, enabled, stt_url, stt_key, stt_model)
             VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
               enabled = excluded.enabled, stt_url = excluded.stt_url,
               stt_key = excluded.stt_key, stt_model = excluded.stt_model",
            rusqlite::params![
                req.enabled as i32,
                req.stt_url.unwrap_or_default(),
                req.stt_key.unwrap_or_default(),
                req.stt_model.unwrap_or_default(),
            ],
        )
        .map_err(|e| e.to_string())
    })?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
