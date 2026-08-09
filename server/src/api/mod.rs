//! REST API handlers for Relay One.
//!
//! Ported from the Tauri-era `ipc.rs` commands. Each Tauri command becomes a
//! typed axum handler; `State<'_, AppState>` becomes axum `State<AppState>`.

pub mod accounts;
pub mod backup;
pub mod ai;
pub mod delete_queue;
pub mod export;
pub mod health;
pub mod messages;
pub mod push;
pub mod send;
pub mod settings;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;

use crate::AppState;

/// Assemble the full API router. `api_state` is cloned into the router.
pub fn router() -> Router<AppState> {
    Router::new()
        // Health / meta
        .route("/health", get(health::health))
        .route("/info", get(health::info))
        .route("/events", get(health::events))
        // Settings
        .route("/settings", get(settings::get_settings).post(settings::save_settings))
        .route(
            "/settings/move-to-trash",
            get(settings::get_move_to_trash).post(settings::set_move_to_trash),
        )
        // Accounts
        .route("/accounts", get(accounts::list_accounts).post(accounts::connect_account))
        .route("/accounts/{id}", axum::routing::delete(accounts::delete_account))
        .route("/accounts/{id}/settings", post(accounts::update_account))
        // Folders
        .route("/folders", get(messages::list_imap_folders).post(messages::create_folder))
        .route("/folders/rename", post(messages::rename_folder))
        // Messages
        .route("/messages", get(messages::fetch_messages))
        .route("/messages/search", get(messages::search_messages))
        .route("/messages/{uid}/body", get(messages::fetch_message_body))
        .route("/messages/{uid}/raw", get(messages::fetch_raw_message))
        .route("/messages/{uid}/attachments", get(messages::fetch_attachments))
        .route(
            "/messages/{uid}/attachments/{att_id}/content",
            get(messages::fetch_attachment_content),
        )
        .route("/messages/{uid}/read", post(messages::mark_as_read))
        .route("/messages/{uid}/unread", post(messages::mark_as_unseen))
        .route("/messages/{uid}/delete", post(messages::delete_message))
        .route("/messages/{uid}/move", post(messages::move_message))
        // Send
        .route("/send", post(send::send_message))
        // AI
        .route("/ai/reply", post(ai::ai_generate_reply))
        .route("/ai/summarize", post(ai::ai_summarize))
        .route("/ai/folder-summaries", post(ai::trigger_folder_summaries))
        .route("/ai/reset-circuit-breaker", post(ai::reset_circuit_breaker))
        .route("/ai/draft", post(ai::ai_draft_from_bullets))
        .route("/ai/format", post(ai::ai_format_text))
        .route("/ai/detect-priority", post(ai::ai_detect_priority))
        .route("/ai/fraud-check", post(ai::fraud_check))
        .route("/ai/generate-mail", post(ai::ai_generate_mail))
        .route("/ai/tone-profile", post(ai::get_tone_profile))
        .route("/ai/suggest-recipient", post(ai::ai_suggest_recipient))
        .route("/ai/suggest-subject", post(ai::ai_suggest_subject))
        // Web Push
        .route("/push/vapid", get(push::vapid_key))
        .route("/push/subscribe", post(push::subscribe))
        .route("/push/unsubscribe", post(push::unsubscribe))
        // Delete queue (verify pipeline review)
        .route("/archive/delete-queue", get(delete_queue::list_delete_queue))
        .route(
            "/archive/delete-queue/{id}/retry",
            post(delete_queue::retry_delete_queue),
        )
        .route(
            "/archive/delete-queue/{id}/remove",
            post(delete_queue::remove_delete_queue),
        )
        // Export (EML/MBox)
        .route("/export", get(export::export_archive))
        // Backup snapshot
        .route("/archive/backup", post(backup::create_backup))
        // X-Relay-Key guard (Concept §12, F6): applied AFTER all routes so
        // axum wraps them; protects against direct cluster-internal callers.
        // /health, /info and /events stay open (probes + browser SSE).
        .route_layer(axum::middleware::from_fn(relay_key_guard))
}

/// X-Relay-Key guard (Concept §12 / F6).
///
/// When `RELAY_API_KEY` is set (chart-provided K8s secret), requests from
/// INSIDE the cluster (ClusterIP / service DNS — no public host header) must
/// carry the key in `X-Relay-Key`. Browser traffic through the Olares
/// entrance arrives with a public Host header (e.g. mail.aimighty.olares.de)
/// and is already protected by the entrance authLevel — it passes through
/// without the key (Concept: "the web app does not set the header itself").
/// `/health` and `/info` stay open for K8s probes.
async fn relay_key_guard(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::StatusCode;

    let configured = std::env::var("RELAY_API_KEY").unwrap_or_default();
    if configured.is_empty() {
        return next.run(req).await;
    }

    let path = req.uri().path().to_string();
    if path == "/health" || path == "/info" || path == "/events" {
        return next.run(req).await;
    }

    // Browser path: public Host header (entrance domain). Internal callers
    // present an IP or a bare service name as Host.
    let host = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let looks_internal = host.is_empty()
        || host.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ':' || c == '-')
        || !host.contains('.');

    if !looks_internal {
        return next.run(req).await;
    }

    let has_key = req
        .headers()
        .get("x-relay-key")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == configured)
        .unwrap_or(false);

    if has_key {
        next.run(req).await
    } else {
        let body = axum::Json(serde_json::json!({ "error": "X-Relay-Key fehlt oder ungültig" }));
        (StatusCode::UNAUTHORIZED, body).into_response()
    }
}

/// Shared error type: turns a `String` error into `500 {"error": "…"}`.
#[derive(Debug)]
pub struct ApiError(pub String);

impl From<String> for ApiError {
    fn from(s: String) -> Self {
        ApiError(s)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({ "error": self.0 }));
        (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
    }
}

/// Convenience result alias for handlers.
pub type ApiResult<T> = Result<Json<T>, ApiError>;

/// Wrap a `Result<T, String>` into an `ApiResult`.
pub fn ok<T: Serialize>(r: Result<T, String>) -> ApiResult<T> {
    r.map(Json).map_err(ApiError)
}
