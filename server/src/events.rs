//! Event bus for server → client notifications.
//!
//! Replaces Tauri's `Emitter` in the web version. Background tasks
//! (sync scheduler, carddav) publish events; clients subscribe via
//! Server-Sent Events (`GET /api/v1/events`).

use std::sync::Arc;

use tokio::sync::broadcast;

/// Capacity of the broadcast channel (events dropped when a slow client
/// falls behind — clients reconnect and re-sync from the DB anyway).
const CHANNEL_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct EventBus {
    tx: Arc<broadcast::Sender<String>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self { tx: Arc::new(tx) }
    }

    /// Publish an event. `payload` is serialized to JSON and wrapped as
    /// `{"event": "<name>", "payload": <payload>}`.
    pub fn emit<S: serde::Serialize>(&self, event: &str, payload: S) {
        let message = match serde_json::to_string(&serde_json::json!({
            "event": event,
            "payload": payload,
        })) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("EventBus: payload serialization failed for '{event}': {e}");
                return;
            }
        };
        let _ = self.tx.send(message);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }
}
