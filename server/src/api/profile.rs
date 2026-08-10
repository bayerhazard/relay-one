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
#[serde(rename_all = "camelCase")]
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

/// `POST /api/v1/voice/transcribe` — transcribe a WAV/audio recording.
///
/// The client records audio (Web Audio API / MediaRecorder, WAV) and sends
/// it base64-encoded. The server forwards it to the configured
/// OpenAI-compatible STT endpoint (`<stt_url>/audio/transcriptions`,
/// multipart form-data with model + file) and returns the transcript text.
#[derive(Deserialize)]
pub struct TranscribeRequest {
    #[serde(rename = "audioBase64")]
    pub audio_base64: String,
}

pub async fn transcribe_voice(
    State(state): State<AppState>,
    Json(req): Json<TranscribeRequest>,
) -> ApiResult<serde_json::Value> {
    use base64::Engine as _;
    use reqwest::multipart::{Form, Part};

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

    if enabled == 0 {
        return Err(ApiError("Voice2Mail ist deaktiviert".into()));
    }
    if stt_url.trim().is_empty() {
        return Err(ApiError("Keine Speech-to-Text-URL konfiguriert".into()));
    }
    if stt_model.trim().is_empty() {
        return Err(ApiError("Kein STT-Modell konfiguriert".into()));
    }

    let audio_bytes = base64::engine::general_purpose::STANDARD
        .decode(req.audio_base64.as_bytes())
        .map_err(|e| ApiError(format!("Audio-Base64 ungültig: {e}")))?;
    if audio_bytes.is_empty() {
        return Err(ApiError("Kein Audio empfangen".into()));
    }

    let endpoint = format!("{}/audio/transcriptions", stt_url.trim_end_matches('/'));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| ApiError(format!("HTTP-Client-Fehler: {e}")))?;

    let file_part = Part::bytes(audio_bytes)
        .file_name("recording.wav")
        .mime_str("audio/wav")
        .map_err(|e| ApiError(e.to_string()))?;
    let form = Form::new()
        .text("model", stt_model)
        .text("language", "de")
        .part("file", file_part);

    let mut req_builder = client
        .post(&endpoint)
        .multipart(form)
        .timeout(std::time::Duration::from_secs(120));

    if !stt_key.trim().is_empty() {
        req_builder = req_builder.bearer_auth(stt_key.trim());
    }

    let resp = req_builder
        .send()
        .await
        .map_err(|e| ApiError(format!("STT-Server nicht erreichbar: {e}")))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| ApiError(format!("STT-Antwort konnte nicht gelesen werden: {e}")))?;

    if !status.is_success() {
        return Err(ApiError(format!(
            "STT-Server antwortete mit HTTP {}: {}",
            status.as_u16(),
            &body[..body.len().min(300)]
        )));
    }

    // Parse the OpenAI-compatible response: {"text": "..."} or plain text.
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(text) = json.get("text").and_then(|t| t.as_str()) {
            return Ok(Json(serde_json::json!({ "text": text })));
        }
    }
    // Some servers return the raw transcript as text/plain.
    Ok(Json(serde_json::json!({ "text": body.trim() })))
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
        "sttUrl": stt_url,
        "sttKey": stt_key,
        "sttModel": stt_model,
    })))
}

/// `POST /api/v1/voice/config` — save STT configuration.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_stt_response(body: &str) -> String {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
            if let Some(text) = json.get("text").and_then(|t| t.as_str()) {
                return text.to_string();
            }
        }
        body.trim().to_string()
    }

    #[test]
    fn stt_json_text_extraction() {
        assert_eq!(parse_stt_response(r#"{"text":"Hallo Welt"}"#), "Hallo Welt");
        assert_eq!(parse_stt_response(r#"{"text":"  mit Leerzeichen  "}"#), "  mit Leerzeichen  ");
        assert_eq!(parse_stt_response(r#"{"error":"boom"}"#), r#"{"error":"boom"}"#);
    }

    #[test]
    fn stt_plain_text_fallback() {
        assert_eq!(parse_stt_response("Guten Morgen"), "Guten Morgen");
        assert_eq!(parse_stt_response("  getrimmt  "), "getrimmt");
        assert_eq!(parse_stt_response(""), "");
    }

    #[test]
    fn transcribe_request_camelcase_deserialization() {
        let req: TranscribeRequest =
            serde_json::from_str(r#"{"audioBase64":"aGVsbG8="}"#).unwrap();
        assert_eq!(req.audio_base64, "aGVsbG8=");
    }
}
