//! Health + Server-Sent-Events endpoints.

use axum::response::sse::{Event, KeepAlive};
use axum::response::Sse;
use axum::extract::State;
use axum::Json;
use std::convert::Infallible;
use std::time::Duration;

use crate::AppState;

use super::ApiResult;

/// `GET /api/v1/health` — liveness probe.
pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok", "service": "relay-one" }))
}

/// `GET /api/v1/info` — version (for debugging). The data-root path is
/// deliberately NOT exposed (CR-06: no internal layout leakage).
pub async fn info() -> ApiResult<serde_json::Value> {
    Ok(Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
    })))
}

/// `GET /api/v1/events` — Server-Sent Events stream.
///
/// Background tasks publish via [`crate::events::EventBus`]; each SSE client
/// receives `{"event": "...", "payload": {...}}` lines.
pub async fn events(State(state): State<AppState>) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.events.subscribe();
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(msg) => yield Ok(Event::default().data(msg)),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
