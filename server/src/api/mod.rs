//! REST API handlers for Relay One.
//!
//! Ported from the Tauri-era `ipc.rs` commands. Each Tauri command becomes a
//! typed axum handler; `State<'_, AppState>` becomes axum `State<AppState>`.

pub mod accounts;
pub mod ai;
pub mod health;
pub mod messages;
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
        // Folders
        .route("/folders", get(messages::list_imap_folders))
        .route("/folders/rename", post(messages::rename_folder))
        // Messages
        .route("/messages", get(messages::fetch_messages))
        .route("/messages/search", get(messages::search_messages))
        .route("/messages/{uid}/body", get(messages::fetch_message_body))
        .route("/messages/{uid}/raw", get(messages::fetch_raw_message))
        .route("/messages/{uid}/attachments", get(messages::fetch_attachments))
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
