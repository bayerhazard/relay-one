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

// ─── Voice / Text-to-Speech (Phase D) ─────────────────────────

/// `POST /api/v1/voice/speak` — TTS proxy (Concept §9.5).
///
/// The client sends the text to speak; the server forwards it to the
/// configured OpenAI-compatible TTS endpoint (`<tts_url>/audio/speech`) and
/// streams the audio bytes back (mp3). Not configured → **409** (the frontend
/// hides the speaker button in that case). An empty key sends no auth header.
#[derive(Deserialize)]
pub struct SpeakRequest {
    pub text: String,
    #[serde(default)]
    pub lang: Option<String>,
}

fn tts_json_err(status: axum::http::StatusCode, msg: &str) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

pub async fn speak_voice(
    State(state): State<AppState>,
    Json(req): Json<SpeakRequest>,
) -> axum::response::Response {
    use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
    use axum::response::IntoResponse as _;

    let (tts_enabled, tts_url, tts_key, tts_model) = with_db(&state, |conn| {
        let tts_enabled: i64 = conn
            .query_row("SELECT tts_enabled FROM voice_settings WHERE id = 1", [], |r| r.get(0))
            .unwrap_or(0);
        let tts_url: String = conn
            .query_row("SELECT tts_url FROM voice_settings WHERE id = 1", [], |r| r.get(0))
            .unwrap_or_default();
        let tts_key: String = conn
            .query_row("SELECT tts_key FROM voice_settings WHERE id = 1", [], |r| r.get(0))
            .unwrap_or_default();
        let tts_model: String = conn
            .query_row("SELECT tts_model FROM voice_settings WHERE id = 1", [], |r| r.get(0))
            .unwrap_or_default();
        Ok::<_, String>((tts_enabled, tts_url, tts_key, tts_model))
    })
    .unwrap_or((0, String::new(), String::new(), String::new()));

    if tts_enabled == 0 || tts_url.trim().is_empty() {
        return tts_json_err(StatusCode::CONFLICT, "TTS ist nicht konfiguriert");
    }
    if req.text.trim().is_empty() {
        return tts_json_err(StatusCode::BAD_REQUEST, "Kein Text zum Sprechen");
    }

    let endpoint = format!("{}/audio/speech", tts_url.trim_end_matches('/'));
    let model = if tts_model.trim().is_empty() {
        "tts-1".to_string()
    } else {
        tts_model.trim().to_string()
    };

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return tts_json_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("HTTP-Client-Fehler: {e}"),
            )
        }
    };

    let mut builder = client
        .post(&endpoint)
        .json(&serde_json::json!({
            "model": model,
            "input": req.text,
            "voice": "alloy",
            "response_format": "mp3",
        }))
        .timeout(std::time::Duration::from_secs(60));

    if !tts_key.trim().is_empty() {
        builder = builder.bearer_auth(tts_key.trim());
    }

    let resp = match builder.send().await {
        Ok(r) => r,
        Err(e) => {
            return tts_json_err(
                StatusCode::BAD_GATEWAY,
                &format!("TTS-Server nicht erreichbar: {e}"),
            )
        }
    };
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .filter(|t| t.contains("audio"))
        .unwrap_or("audio/mpeg")
        .to_string();
    let bytes = match resp.bytes().await {
        Ok(b) => b.to_vec(),
        Err(e) => {
            return tts_json_err(
                StatusCode::BAD_GATEWAY,
                &format!("TTS-Antwort konnte nicht gelesen werden: {e}"),
            )
        }
    };

    if !status.is_success() {
        let preview = String::from_utf8_lossy(&bytes[..bytes.len().min(300)]);
        return tts_json_err(
            StatusCode::BAD_GATEWAY,
            &format!("TTS-Server antwortete mit HTTP {}: {preview}", status.as_u16()),
        );
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type).unwrap_or_else(|_| HeaderValue::from_static("audio/mpeg")),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    (StatusCode::OK, headers, bytes).into_response()
}

// ─── Voice / Speech-to-Text ───────────────────────────────────

/// `GET /api/v1/voice/config` — STT + TTS configuration (Phase D adds TTS).
pub async fn get_voice_config(State(state): State<AppState>) -> ApiResult<serde_json::Value> {
    let (enabled, stt_url, stt_key, stt_model, tts_enabled, tts_url, tts_key, tts_model, tts_auto) =
        with_db(&state, |conn| {
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
            let tts_enabled: i64 = conn
                .query_row("SELECT tts_enabled FROM voice_settings WHERE id = 1", [], |r| r.get(0))
                .unwrap_or(0);
            let tts_url: String = conn
                .query_row("SELECT tts_url FROM voice_settings WHERE id = 1", [], |r| r.get(0))
                .unwrap_or_default();
            let tts_key: String = conn
                .query_row("SELECT tts_key FROM voice_settings WHERE id = 1", [], |r| r.get(0))
                .unwrap_or_default();
            let tts_model: String = conn
                .query_row("SELECT tts_model FROM voice_settings WHERE id = 1", [], |r| r.get(0))
                .unwrap_or_default();
            let tts_auto: bool = crate::cache::settings::get_setting(conn, "assistant_tts_auto")
                .ok()
                .flatten()
                .map(|v| v == "true")
                .unwrap_or(false);
            Ok::<_, String>((
                enabled, stt_url, stt_key, stt_model,
                tts_enabled, tts_url, tts_key, tts_model, tts_auto,
            ))
        })?;
    Ok(Json(serde_json::json!({
        "enabled": enabled != 0,
        "sttUrl": stt_url,
        "sttKey": stt_key,
        "sttModel": stt_model,
        "ttsEnabled": tts_enabled != 0,
        "ttsUrl": tts_url,
        "ttsKey": tts_key,
        "ttsModel": tts_model,
        "ttsAuto": tts_auto,
    })))
}

/// `POST /api/v1/voice/config` — save STT + TTS configuration (Phase D).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceConfigRequest {
    pub enabled: bool,
    pub stt_url: Option<String>,
    pub stt_key: Option<String>,
    pub stt_model: Option<String>,
    pub tts_enabled: Option<bool>,
    pub tts_url: Option<String>,
    pub tts_key: Option<String>,
    pub tts_model: Option<String>,
    pub tts_auto: Option<bool>,
}

pub async fn save_voice_config(
    State(state): State<AppState>,
    Json(req): Json<VoiceConfigRequest>,
) -> ApiResult<serde_json::Value> {
    let tts_auto = req.tts_auto.unwrap_or(false);
    with_db(&state, |conn| {
        conn.execute(
            "INSERT INTO voice_settings (id, enabled, stt_url, stt_key, stt_model,
                                         tts_enabled, tts_url, tts_key, tts_model)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                enabled = excluded.enabled, stt_url = excluded.stt_url,
                stt_key = excluded.stt_key, stt_model = excluded.stt_model,
                tts_enabled = excluded.tts_enabled, tts_url = excluded.tts_url,
                tts_key = excluded.tts_key, tts_model = excluded.tts_model",
            rusqlite::params![
                req.enabled as i32,
                req.stt_url.unwrap_or_default(),
                req.stt_key.unwrap_or_default(),
                req.stt_model.unwrap_or_default(),
                req.tts_enabled.unwrap_or(false) as i32,
                req.tts_url.unwrap_or_default(),
                req.tts_key.unwrap_or_default(),
                req.tts_model.unwrap_or_default(),
            ],
        )
        .map_err(|e| e.to_string())?;
        crate::cache::settings::set_setting(
            conn,
            "assistant_tts_auto",
            if tts_auto { "true" } else { "false" },
        )
        .map_err(|e| e.to_string())?;
        Ok(())
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

    // ─── Phase D: TTS ───────────────────────────────────────────

    fn state_with_db() -> AppState {
        let mut state = AppState::new();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::cache::db::init_db(&conn).unwrap();
        *state.cache_db.lock() = Some(conn);
        state
    }

    /// Two states can share one in-memory DB (for save-then-get roundtrips).
    fn shared_db() -> std::sync::Arc<parking_lot::Mutex<Option<rusqlite::Connection>>> {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::cache::db::init_db(&conn).unwrap();
        std::sync::Arc::new(parking_lot::Mutex::new(Some(conn)))
    }

    fn seed_tts(db: &std::sync::Arc<parking_lot::Mutex<Option<rusqlite::Connection>>>, sql: &str) {
        let g = db.lock();
        let conn = g.as_ref().unwrap();
        conn.execute(sql, []).unwrap();
    }

    #[tokio::test]
    async fn tts_not_configured_returns_409() {
        let state = state_with_db(); // default row: tts_enabled = 0
        let resp = speak_voice(
            State(state),
            Json(SpeakRequest { text: "Hallo".into(), lang: None }),
        )
        .await;
        assert_eq!(resp.status(), axum::http::StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn tts_empty_text_returns_400() {
        let db = shared_db();
        seed_tts(&db, "UPDATE voice_settings SET tts_enabled=1, tts_url='http://tts', tts_model='tts-1' WHERE id=1");
        let mut state = AppState::new();
        state.cache_db = db;
        let resp = speak_voice(
            State(state),
            Json(SpeakRequest { text: "   ".into(), lang: None }),
        )
        .await;
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn tts_config_get_returns_fields() {
        let db = shared_db();
        seed_tts(&db, "UPDATE voice_settings SET tts_enabled=1, tts_url='http://tts', tts_key='sekret', tts_model='tts-1' WHERE id=1");
        let mut state = AppState::new();
        state.cache_db = db;
        let Json(cfg) = get_voice_config(State(state)).await.unwrap();
        assert_eq!(cfg["ttsEnabled"], true);
        assert_eq!(cfg["ttsUrl"], "http://tts");
        assert_eq!(cfg["ttsKey"], "sekret");
        assert_eq!(cfg["ttsModel"], "tts-1");
        assert_eq!(cfg["ttsAuto"], false);
    }

    #[tokio::test]
    async fn tts_config_save_roundtrip() {
        let db = shared_db();
        let mut s1 = AppState::new();
        s1.cache_db = db.clone();
        save_voice_config(
            State(s1),
            Json(VoiceConfigRequest {
                enabled: false,
                stt_url: None,
                stt_key: None,
                stt_model: None,
                tts_enabled: Some(true),
                tts_url: Some("http://tts".into()),
                tts_key: Some("k".into()),
                tts_model: Some("tts-1".into()),
                tts_auto: Some(true),
            }),
        )
        .await
        .unwrap();

        let mut s2 = AppState::new();
        s2.cache_db = db;
        let Json(cfg) = get_voice_config(State(s2)).await.unwrap();
        assert_eq!(cfg["ttsEnabled"], true);
        assert_eq!(cfg["ttsUrl"], "http://tts");
        assert_eq!(cfg["ttsKey"], "k");
        assert_eq!(cfg["ttsModel"], "tts-1");
        assert_eq!(cfg["ttsAuto"], true);
    }

    #[test]
    fn tts_migration_columns_exist() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::cache::db::init_db(&conn).unwrap();
        let mut stmt = conn.prepare("PRAGMA table_info(voice_settings)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        for c in ["tts_enabled", "tts_url", "tts_key", "tts_model"] {
            assert!(cols.contains(&c.to_string()), "missing column {c}");
        }
    }
}
