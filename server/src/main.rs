#![allow(
    clippy::single_component_path_imports,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::explicit_counter_loop,
    clippy::needless_borrows_for_generic_args,
    clippy::needless_borrow,
    clippy::manual_is_multiple_of,
    clippy::needless_question_mark,
    clippy::manual_ok_err,
    clippy::let_unit_value,
    clippy::collapsible_if,
    clippy::new_without_default,
)]

use std::sync::Arc;

use axum::{Router, extract::DefaultBodyLimit};
use tokio::sync::mpsc;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::trace::TraceLayer;

use relay_server::api;
use relay_server::bootstrap;
use relay_server::cache;
use relay_server::dav;
use relay_server::crypto;
use relay_server::sync;
use relay_server::AppState;

/// Data root for ALL user data. Single backup root for the Olares PVC.
/// Env override for local development; production default is /data/Relay.
fn data_root() -> std::path::PathBuf {
    std::env::var("RELAY_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/data/Relay"))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let data_dir = data_root();
    std::fs::create_dir_all(&data_dir).expect("Datenverzeichnis anlegen fehlgeschlagen");
    tracing::info!("Relay One Server — Datenstamm: {:?}", data_dir);

    // Encryption key (file-based, 0600) — protects passwords at rest.
    crypto::init_key(&data_dir).expect("Encryption-Key-Initialisierung fehlgeschlagen");

    // SEC-01: Warn (or refuse to start) when the API key is not configured.
    let relay_key = std::env::var("RELAY_API_KEY").unwrap_or_default();
    if relay_key.is_empty() {
        if std::env::var("RELAY_REQUIRE_KEY").unwrap_or_default() == "1" {
            panic!("RELAY_REQUIRE_KEY=1 ist gesetzt, aber RELAY_API_KEY fehlt — Server startet nicht.");
        }
        tracing::warn!(
            "RELAY_API_KEY ist nicht gesetzt — die API ist für alle cluster-internen \
             Aufrufer ohne Key-Schutz erreichbar. Setze RELAY_API_KEY (K8s Secret) \
             oder RELAY_REQUIRE_KEY=1, um den Start ohne Key zu verbieten."
        );
    }

    // SQLite cache/archive DB (WAL).
    let db_path = data_dir.join("index.db");
    tracing::info!("Datenbank-Pfad: {:?}", db_path);
    let conn = rusqlite::Connection::open(&db_path).expect("DB öffnen fehlgeschlagen");
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA cache_size=-8000; PRAGMA busy_timeout=5000;",
    )
    .expect("DB-PRAGMA fehlgeschlagen");
    cache::db::init_db(&conn).expect("DB-Schema-Initialisierung fehlgeschlagen");
    let _ = cache::settings::migrate_settings(&conn);

    let state = Arc::new(AppState::new());
    *state.cache_db.lock() = Some(conn);
    *state.db_path.lock() = Some(db_path);

    // Load AI + CardDAV + CalDAV settings from DB.
    bootstrap::load_ai_settings(&state);
    load_carddav_settings(&state);
    load_caldav_settings(&state);

    // Background: reconnect IMAP/SMTP clients, then start sync scheduler.
    let sync_state = state.clone();
    let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
    *state.sync_shutdown_tx.lock() = Some(shutdown_tx);
    tokio::spawn(async move {
        bootstrap::reconnect_clients(&sync_state).await;
        sync::scheduler::start_periodic_sync(sync_state, shutdown_rx).await;
    });

    // CardDAV background sync (starts only if configured).
    let carddav_state = state.clone();
    let (carddav_shutdown_tx, carddav_shutdown_rx) = mpsc::channel(1);
    *state.carddav_shutdown_tx.lock() = Some(carddav_shutdown_tx);
    tokio::spawn(async move {
        dav::scheduler::start_carddav_sync(carddav_state, carddav_shutdown_rx).await;
    });

    // CalDAV background sync (starts only if configured).
    let caldav_state = state.clone();
    let (caldav_shutdown_tx, caldav_shutdown_rx) = mpsc::channel(1);
    *state.caldav_shutdown_tx.lock() = Some(caldav_shutdown_tx);
    tokio::spawn(async move {
        dav::scheduler::start_caldav_sync(caldav_state, caldav_shutdown_rx).await;
    });

    // Build the axum app: API under /api/v1.
    // SEC-15: Global body limit is 1 MB. Routes that accept large payloads
    // (send, import) override with 64 MB via route_layer in api/mod.rs.
    // SEC-16: Max 50 concurrent requests (prevents resource exhaustion).
    let app = Router::new()
        .nest("/api/v1", api::router())
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(ConcurrencyLimitLayer::new(50))
        .layer(TraceLayer::new_for_http())
        .with_state((*state).clone());

    // Serve the static web build (SPA) from RELAY_WEB_DIR if present.
    // Anything not under /api is served from disk with an index.html
    // fallback for client-side routes.
    let web_dir = std::env::var("RELAY_WEB_DIR").unwrap_or_else(|_| "/opt/relay/web".to_string());
    let fallback = std::path::PathBuf::from(&web_dir).join("index.html");
    let static_app = tower_http::services::ServeDir::new(&web_dir)
        .not_found_service(tower_http::services::ServeFile::new(fallback));
    let app = app.fallback_service(static_app);

    let bind_addr = std::env::var("RELAY_BIND")
        .unwrap_or_else(|_| "0.0.0.0:3000".to_string());
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .expect("Bind fehlgeschlagen");
    tracing::info!("Relay One Server lauscht auf http://{}", bind_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(state.clone()))
        .await
        .expect("Server-Fehler");
}

fn load_carddav_settings(state: &AppState) {
    let guard = state.cache_db.lock();
    let Some(conn) = guard.as_ref() else {
        return;
    };
    if let Ok(Some(json)) = cache::settings::get_setting(conn, "carddav_settings") {
        if let Ok(mut settings) = serde_json::from_str::<dav::CardDavSettings>(&json) {
            settings.password = crypto::decrypt(&settings.password).unwrap_or(settings.password);
            *state.carddav_settings.write() = Some(settings);
        }
    }
    if let Ok(Some(token)) = cache::settings::get_setting(conn, "carddav_sync_token") {
        *state.carddav_sync_token.write() = token;
    }
}

fn load_caldav_settings(state: &AppState) {
    let guard = state.cache_db.lock();
    let Some(conn) = guard.as_ref() else {
        return;
    };
    if let Ok(Some(json)) = cache::settings::get_setting(conn, "caldav_settings") {
        if let Ok(mut settings) = serde_json::from_str::<dav::CalDavSettings>(&json) {
            settings.password = crypto::decrypt(&settings.password).unwrap_or(settings.password);
            *state.caldav_settings.write() = Some(settings);
        }
    }
    if let Ok(Some(token)) = cache::settings::get_setting(conn, "caldav_sync_token") {
        *state.caldav_sync_token.write() = token;
    }
}

async fn shutdown_signal(state: Arc<AppState>) {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install ctrl-c handler");
    tracing::info!("SIGINT empfangen — fahre herunter…");
    state.shutdown().await;
}
