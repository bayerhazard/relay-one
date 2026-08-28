//! Health + Server-Sent-Events endpoints.

use axum::response::sse::{Event, KeepAlive};
use axum::response::Sse;
use axum::extract::State;
use axum::Json;
use std::convert::Infallible;
use std::time::Duration;

use crate::AppState;

use super::ApiResult;

/// `GET /api/v1/health` — readiness probe.
///
/// Verifies the process is up AND the SQLite store responds (`SELECT 1`).
/// Returns 503 when the DB is unavailable so the entrance can route around a
/// broken instance instead of serving requests that will all fail.
pub async fn health(State(state): State<AppState>) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    let db_ok = crate::db::with_db(&state, |conn| {
        conn.query_row("SELECT 1", [], |r| r.get::<_, i64>(0))
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .is_ok();
    if db_ok {
        (
            axum::http::StatusCode::OK,
            Json(serde_json::json!({ "status": "ok", "service": "relay-one" })),
        )
    } else {
        (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "status": "degraded", "service": "relay-one", "reason": "db" })),
        )
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppState;

    /// AppState with an initialized in-memory DB (readiness = OK).
    fn state_with_db() -> AppState {
        let mut state = AppState::new();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::cache::db::init_db(&conn).unwrap();
        *state.cache_db.lock() = Some(conn);
        state
    }

    // M4 (Code-Review 2026-08-28): /health is a real readiness probe.
    #[tokio::test]
    async fn health_ok_when_db_responds() {
        let state = state_with_db();
        let (status, body) = health(State(state)).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(body.0["status"], "ok");
    }

    #[tokio::test]
    async fn health_degraded_when_db_missing() {
        let state = AppState::new(); // cache_db = None
        let (status, body) = health(State(state)).await;
        assert_eq!(status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.0["status"], "degraded");
        assert_eq!(body.0["reason"], "db");
    }
}
