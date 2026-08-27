//! Relay One server — library crate.
//!
//! Exposes the domain logic (IMAP/SMTP, cache, AI, security, tone, dav)
//! and the shared `AppState` so the binary (`main.rs`) and the REST API
//! handlers can share code. This crate is intentionally free of Tauri — the
//! migration target is a pure web server (axum).

pub mod ai;
pub mod api;
pub mod bootstrap;
pub mod cache;
pub mod crypto;
pub mod dav;
pub mod db;
pub mod error;
pub mod events;
pub mod imap;
#[cfg(test)]
pub mod ics_spike;
pub mod push;
pub mod security;
pub mod smtp;
pub mod sync;
pub mod tone;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::ai::client::{AIClient, AIConfig};

/// Shared application state, reachable from every REST handler and background
/// task. Mirrors the Tauri-era `AppState`, minus Tauri-specific plumbing.
#[derive(Clone)]
pub struct AppState {
    pub ai_config: Arc<parking_lot::RwLock<Option<AIConfig>>>,
    pub ai_client: Arc<parking_lot::RwLock<Option<Arc<AIClient>>>>,
    pub cache_db: Arc<parking_lot::Mutex<Option<rusqlite::Connection>>>,
    pub db_path: Arc<parking_lot::Mutex<Option<std::path::PathBuf>>>,
    pub cached_settings: Arc<parking_lot::Mutex<Option<AIConfig>>>,
    pub imap_clients: Arc<parking_lot::RwLock<HashMap<u32, Arc<imap::client::ImapClient>>>>,
    pub smtp_clients: Arc<parking_lot::RwLock<HashMap<u32, Arc<smtp::client::SmtpClient>>>>,
    pub sync_shutdown_tx: Arc<parking_lot::Mutex<Option<mpsc::Sender<()>>>>,
    pub sync_queue: Arc<sync::queue::SyncQueue>,
    pub carddav_settings: Arc<parking_lot::RwLock<Option<dav::CardDavSettings>>>,
    pub carddav_sync_token: Arc<parking_lot::RwLock<String>>,
    pub carddav_shutdown_tx: Arc<parking_lot::Mutex<Option<mpsc::Sender<()>>>>,
    /// CalDAV (Phase 0): calendar/event sync state.
    pub caldav_settings: Arc<parking_lot::RwLock<Option<dav::CalDavSettings>>>,
    pub caldav_sync_token: Arc<parking_lot::RwLock<String>>,
    pub caldav_shutdown_tx: Arc<parking_lot::Mutex<Option<mpsc::Sender<()>>>>,
    /// Server → client notification bus (replaces Tauri `Emitter`).
    pub events: events::EventBus,
    /// Data root for ALL user data (EML archive, attachments, DBs).
    pub data_root: std::path::PathBuf,
    /// Background migration status (started via /migrate/start).
    pub migration: Arc<parking_lot::RwLock<Option<crate::api::migrate::MigrationStatus>>>,
    /// Meta-only folder list cache (account_id → folder → last list), so
    /// repeated folder switches render instantly without re-reading the DB.
    pub folder_cache: Arc<parking_lot::RwLock<crate::cache::FolderListCache>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            ai_config: Arc::new(parking_lot::RwLock::new(None)),
            ai_client: Arc::new(parking_lot::RwLock::new(None)),
            cache_db: Arc::new(parking_lot::Mutex::new(None)),
            db_path: Arc::new(parking_lot::Mutex::new(None)),
            cached_settings: Arc::new(parking_lot::Mutex::new(None)),
            imap_clients: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            smtp_clients: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            sync_shutdown_tx: Arc::new(parking_lot::Mutex::new(None)),
            sync_queue: Arc::new(sync::queue::SyncQueue::new()),
            carddav_settings: Arc::new(parking_lot::RwLock::new(None)),
            carddav_sync_token: Arc::new(parking_lot::RwLock::new(String::new())),
            carddav_shutdown_tx: Arc::new(parking_lot::Mutex::new(None)),
            caldav_settings: Arc::new(parking_lot::RwLock::new(None)),
            caldav_sync_token: Arc::new(parking_lot::RwLock::new(String::new())),
            caldav_shutdown_tx: Arc::new(parking_lot::Mutex::new(None)),
            events: events::EventBus::new(),
            data_root: std::env::var("RELAY_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::PathBuf::from("/data/Relay")),
            migration: Arc::new(parking_lot::RwLock::new(None)),
            folder_cache: Arc::new(parking_lot::RwLock::new(
                crate::cache::FolderListCache::new(),
            )),
        }
    }

    /// Graceful shutdown: closes IMAP/SMTP connections, stops sync schedulers.
    /// Uses a configurable timeout (default 5s) to avoid blocking exit indefinitely.
    pub async fn shutdown(&self) {
        let deadline = Duration::from_secs(5);
        let result = tokio::time::timeout(deadline, self.shutdown_inner()).await;
        match result {
            Ok(()) => tracing::info!("App: Graceful shutdown abgeschlossen"),
            Err(_) => tracing::warn!("App: Graceful shutdown nach 5s timeout abgebrochen"),
        }
    }

    async fn shutdown_inner(&self) {
        tracing::info!("App: Graceful shutdown gestartet");

        // Step 1: Signal sync scheduler to stop
        let tx_opt = {
            let mut guard = self.sync_shutdown_tx.lock();
            guard.take()
        };
        if let Some(tx) = tx_opt {
            let _ = tx.send(()).await;
            tracing::info!("Sync: Shutdown-Signal gesendet");
        }

        // Step 1.5: Signal CardDAV sync to stop
        let carddav_tx_opt = {
            let mut guard = self.carddav_shutdown_tx.lock();
            guard.take()
        };
        if let Some(tx) = carddav_tx_opt {
            let _ = tx.send(()).await;
            tracing::info!("CardDAV: Shutdown-Signal gesendet");
        }

        // Step 1.6: Signal CalDAV sync to stop
        let caldav_tx_opt = {
            let mut guard = self.caldav_shutdown_tx.lock();
            guard.take()
        };
        if let Some(tx) = caldav_tx_opt {
            let _ = tx.send(()).await;
            tracing::info!("CalDAV: Shutdown-Signal gesendet");
        }

        // Step 2: Short grace period for pending sync operations to drain
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Step 3: Close all IMAP connections (logout + disconnect)
        let clients = {
            let mut guard = self.imap_clients.write();
            guard.drain().collect::<Vec<_>>()
        };
        let count = clients.len();
        for (id, client) in clients {
            tracing::info!("IMAP: Schließe Verbindung für Account {}", id);
            client.shutdown().await;
        }
        if count > 0 {
            tracing::info!("IMAP: {} Verbindungen geschlossen", count);
        }

        // Step 4: Close all SMTP connections (drop transports)
        {
            let mut guard = self.smtp_clients.write();
            let count = guard.len();
            guard.clear();
            if count > 0 {
                tracing::info!("SMTP: {} Verbindungen geschlossen", count);
            }
        }

        // Step 5: Close SQLite connection (flushes WAL)
        {
            let mut guard = self.cache_db.lock();
            if guard.take().is_some() {
                tracing::info!("DB: SQLite-Verbindung geschlossen");
            }
        }
    }
}
