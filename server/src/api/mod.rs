//! REST API handlers for Relay One.
//!
//! Ported from the Tauri-era `ipc.rs` commands. Each Tauri command becomes a
//! typed axum handler; `State<'_, AppState>` becomes axum `State<AppState>`.

pub mod accounts;
pub mod ai;
pub mod attachments;
pub mod backup;
pub mod calendars;
pub mod contacts;
pub mod delete_queue;
pub mod export;
pub mod health;
pub mod import;
pub mod invitations;
pub mod messages;
pub mod migrate;
pub mod profile;
pub mod push;
pub mod send;
pub mod settings;
pub mod todos;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
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
        .route("/accounts/delete", post(accounts::delete_account))
        .route("/accounts/config", post(accounts::update_account))
        // Folders
        .route("/folders", get(messages::list_imap_folders).post(messages::create_folder))
        .route("/folders/rename", post(messages::rename_folder))
        .route("/folders/delete", post(messages::delete_folder))
        // Messages
        .route("/messages", get(messages::fetch_messages))
        .route("/messages/search", get(messages::search_messages))
        .route("/messages/body", get(messages::fetch_message_body))
        .route("/messages/reparse", post(messages::reparse_eml_bodies))
        .route("/messages/raw", get(messages::fetch_raw_message))
        .route("/messages/attachments", get(messages::fetch_attachments))
.route("/messages/attachment", get(messages::fetch_attachment_content))
        .route("/messages/read", post(messages::mark_as_read))
        .route("/messages/unread", post(messages::mark_as_unseen))
        .route("/messages/read-batch", post(messages::mark_batch_as_read))
        .route("/messages/unread-batch", post(messages::mark_batch_as_unseen))
        .route("/messages/flag", post(messages::flag_message))
        .route("/messages/move-cross-account", post(messages::move_cross_account))
        .route("/messages/delete", post(messages::delete_message))
        .route("/messages/move", post(messages::move_message))
        // Send
        .route("/send", post(send::send_message))
        // Drafts (local)
        .route("/draft/save", post(send::save_draft))
        .route("/draft/discard", post(send::discard_draft))
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
        .route("/ai/tone-profiles/export", post(ai::export_tone_profiles))
        .route("/ai/suggest-recipient", post(ai::ai_suggest_recipient))
        .route("/ai/suggest-subject", post(ai::ai_suggest_subject))
        // Calendar AI (Phase 2)
        .route("/ai/conflict-alternatives", post(ai::ai_conflict_alternatives))
        .route("/ai/extract-time", post(ai::ai_extract_time))
        .route("/ai/rsvp-draft", post(ai::ai_rsvp_draft))
        .route("/ai/followups", post(ai::ai_followups))
        // Web Push
        .route("/push/vapid", get(push::vapid_key))
        .route("/push/subscribe", post(push::subscribe))
        .route("/push/unsubscribe", post(push::unsubscribe))
        // Delete queue (verify pipeline review)
        .route("/archive/delete-queue", get(delete_queue::list_delete_queue))
.route("/archive/queue-retry", post(delete_queue::retry_delete_queue))
.route("/archive/queue-remove", post(delete_queue::remove_delete_queue))
        // Export (EML/MBox)
        .route("/export", get(export::export_archive))
        .route("/import/mbox", post(import::import_mbox))
        .route("/import/mbox-dir", post(import::import_mbox_dir))
        .route("/import/attachments-backfill", post(import::attachments_backfill))
        // Attachment maintenance (GC / repair / cache stats)
        .route("/attachments/gc", post(attachments::gc_attachments))
        .route("/attachments/repair", post(attachments::repair_attachments))
        .route("/attachments/stats", get(attachments::attachment_stats))
        .route("/attachments/cleanup", post(attachments::cleanup_attachments))
        .route("/attachments/clear", post(attachments::clear_attachments))
        .route("/cache/clear-ai-summaries", post(settings::clear_ai_summaries))
        // Backup snapshot
        .route("/archive/backup", post(backup::create_backup))
        .route("/archive/backups", get(backup::list_backups))
        .route("/archive/restore", post(backup::restore_backup))
        // Migration (copy account → local folders of another account)
        .route("/migrate/copy-account", post(migrate::copy_account))
        .route("/migrate/copy-folder", post(migrate::copy_folder_endpoint))
        .route("/migrate/count-folder", post(migrate::count_folder))
        .route("/migrate/db-count", post(migrate::db_count))
        .route("/migrate/stop-sync", post(migrate::stop_sync))
        .route("/migrate/reset-target", post(migrate::reset_target))
        .route("/migrate/start", post(migrate::start_migration))
        .route("/migrate/start-folder", post(migrate::start_folder_migration))
        .route("/migrate/status", get(migrate::migration_status))
        // Profile photo + Voice
        .route("/profile/photo", get(profile::get_own_photo).post(profile::save_own_photo))
        .route("/voice/config", get(profile::get_voice_config).post(profile::save_voice_config))
        .route("/voice/transcribe", post(profile::transcribe_voice))
        // CardDAV
        .route("/carddav/settings", get(settings::get_carddav_settings).post(settings::set_carddav_settings))
        .route("/carddav/sync", post(settings::sync_carddav))
        .route("/carddav/search", post(settings::search_carddav))
        .route("/carddav/resolve", post(settings::resolve_carddav))
        // CalDAV (Phase 0)
        .route("/calendars/settings", get(calendars::get_caldav_settings).post(calendars::set_caldav_settings))
        .route("/calendars/sync", post(calendars::sync_caldav))
        .route("/calendars", get(calendars::list_calendars))
        .route("/calendars/events", get(calendars::list_events).post(calendars::create_event))
        .route("/calendars/events/import", post(calendars::import_events))
        .route("/calendars/events/:id", get(calendars::get_event).put(calendars::update_event).delete(calendars::delete_event))
        .route("/calendars/events/:id/ics", get(calendars::get_event_ics))
        .route("/calendars/events/:id/invite", post(calendars::invite_event))
        .route("/calendars/events/:id/rsvp", post(calendars::rsvp_event))
        .route("/calendars/conflicts", get(calendars::find_event_conflicts))
        // iMIP invitation queue (Phase 2.3)
        .route("/invitations", get(invitations::list_invitations))
        .route("/invitations/:uid/accept", post(invitations::accept_invitation))
        .route("/invitations/:uid/decline", post(invitations::decline_invitation))

        .route("/contacts", get(contacts::list_contacts).post(contacts::create_contact))
        .route("/contacts/:uid", put(contacts::update_contact).delete(contacts::delete_contact))

        .route("/todos", get(todos::list_todos).post(todos::create_todo))
        .route("/todos/sync", post(todos::sync_todos))
        .route("/todos/:uid", patch(todos::toggle_todo).delete(todos::delete_todo))
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
/// `/health` stays open for K8s probes.
///
/// Hardening (CR-05/CR-06): only `/health` is unconditionally open; `/info`
/// and `/events` now fall under the same Host heuristic. When
/// `RELAY_TRUSTED_HOST_SUFFIX` is set (e.g. `olares.de`), a "public-looking"
/// Host must also end with that suffix — otherwise the key is required even
/// for a public Host (an internal caller cannot bypass by faking an arbitrary
/// public Host header).
async fn relay_key_guard(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let configured = std::env::var("RELAY_API_KEY").unwrap_or_default();
    if configured.is_empty() {
        return next.run(req).await;
    }

    let path = req.uri().path().to_string();
    if path == "/health" {
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
        // A public Host is only trusted when it matches the configured
        // entrance domain suffix. Without the env var we keep the legacy
        // behavior for backward compatibility.
        if let Ok(suffix) = std::env::var("RELAY_TRUSTED_HOST_SUFFIX") {
            if !suffix.is_empty() && !host.ends_with(suffix.as_str()) {
                return reject_key();
            }
        }
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
        reject_key()
    }
}

fn reject_key() -> axum::response::Response {
    let body = axum::Json(serde_json::json!({ "error": "X-Relay-Key fehlt oder ungültig" }));
    (StatusCode::UNAUTHORIZED, body).into_response()
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn test_router() -> Router {
        Router::new()
            .route("/health", axum::routing::get(|| async { "ok" }))
            .route("/info", axum::routing::get(|| async { "info" }))
            .route("/events", axum::routing::get(|| async { "events" }))
            .route_layer(axum::middleware::from_fn(relay_key_guard))
    }

    fn get(uri: &str, host: &str, key: Option<&str>) -> axum::http::Request<Body> {
        let mut b = axum::http::Request::builder().uri(uri).header(axum::http::header::HOST, host);
        if let Some(k) = key {
            b = b.header("x-relay-key", k);
        }
        b.body(Body::empty()).unwrap()
    }

    fn status(uri: &str, host: &str, key: Option<&str>) -> axum::http::StatusCode {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(test_router().oneshot(get(uri, host, key)))
            .unwrap()
            .status()
    }

    #[test]
    fn health_is_always_open() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("RELAY_API_KEY", "secret");
        std::env::remove_var("RELAY_TRUSTED_HOST_SUFFIX");
        assert_eq!(status("/health", "10.0.0.1:8080", None), 200);
    }

    #[test]
    fn internal_host_requires_key() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("RELAY_API_KEY", "secret");
        std::env::remove_var("RELAY_TRUSTED_HOST_SUFFIX");
        assert_eq!(status("/events", "10.0.0.1:8080", None), 401);
        assert_eq!(status("/events", "10.0.0.1:8080", Some("wrong")), 401);
        assert_eq!(status("/events", "10.0.0.1:8080", Some("secret")), 200);
    }

    #[test]
    fn info_and_events_are_not_unconditionally_open() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("RELAY_API_KEY", "secret");
        std::env::remove_var("RELAY_TRUSTED_HOST_SUFFIX");
        assert_eq!(status("/info", "10.0.0.1:8080", None), 401);
        assert_eq!(status("/events", "10.0.0.1:8080", None), 401);
    }

    #[test]
    fn public_host_passes_legacy_without_suffix() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("RELAY_API_KEY", "secret");
        std::env::remove_var("RELAY_TRUSTED_HOST_SUFFIX");
        assert_eq!(status("/info", "mail.example.com", None), 200);
    }

    #[test]
    fn suffix_mismatch_rejects_spoofed_public_host() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("RELAY_API_KEY", "secret");
        std::env::set_var("RELAY_TRUSTED_HOST_SUFFIX", "olares.de");
        assert_eq!(status("/info", "mail.evil.example.com", None), 401);
    }

    #[test]
    fn suffix_match_passes_browser_traffic() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("RELAY_API_KEY", "secret");
        std::env::set_var("RELAY_TRUSTED_HOST_SUFFIX", "olares.de");
        assert_eq!(status("/events", "mail.aimighty.olares.de", None), 200);
    }

    #[test]
    fn no_key_configured_opens_everything() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var("RELAY_API_KEY");
        std::env::remove_var("RELAY_TRUSTED_HOST_SUFFIX");
        assert_eq!(status("/events", "10.0.0.1:8080", None), 200);
    }
}
