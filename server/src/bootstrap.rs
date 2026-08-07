//! Server bootstrap: reconnect all stored accounts at startup.
//!
//! Extracted from the Tauri-era `ipc.rs` so the axum server can restore
//! IMAP/SMTP connections and start the sync scheduler after boot.

use std::sync::Arc;

use futures::future::join_all;
use tokio::time::timeout;

use crate::ai::client::{AIClient, AIConfig};
use crate::cache;
use crate::crypto;
use crate::imap::client::ImapClient;
use crate::smtp::client::{SmtpClient, SmtpConfig};
use crate::AppState;

/// Load AI settings from the DB and populate `ai_config` + `ai_client`.
pub fn load_ai_settings(state: &AppState) {
    let guard = state.cache_db.lock();
    let Some(conn) = guard.as_ref() else {
        return;
    };
    let Ok(Some(url)) = cache::settings::get_setting(conn, "ai_url") else {
        return;
    };
    let model = cache::settings::get_setting(conn, "ai_model")
        .ok()
        .flatten()
        .unwrap_or_else(|| "llama3.2".into());
    let stored_key = cache::settings::get_setting(conn, "api_key")
        .ok()
        .flatten()
        .unwrap_or_else(|| "ollama".into());
    let api_key = crypto::decrypt(&stored_key).unwrap_or(stored_key);
    let config = AIConfig {
        url,
        api_key,
        model,
        ..Default::default()
    };
    *state.ai_config.write() = Some(config.clone());
    *state.ai_client.write() = Some(Arc::new(AIClient::new(config)));
}

/// Reconnect all stored accounts' IMAP + SMTP clients concurrently.
pub async fn reconnect_clients(state: &AppState) {
    let accounts = {
        let guard = state.cache_db.lock();
        let conn = match guard.as_ref() {
            Some(c) => c,
            None => {
                tracing::warn!("reconnect_clients: DB nicht initialisiert, überspringe Reconnect");
                return;
            }
        };
        cache::accounts::list_accounts_with_passwords(conn).unwrap_or_default()
    };

    if accounts.is_empty() {
        tracing::info!("reconnect_clients: Keine gespeicherten Konten gefunden");
        return;
    }

    tracing::info!(
        "reconnect_clients: Starte Reconnect für {} Konten (parallel)",
        accounts.len()
    );

    let tasks = accounts.into_iter().map(|(acct, stored_imap_password, stored_smtp_password)| {
        reconnect_one_account(state, acct, stored_imap_password, stored_smtp_password)
    });
    join_all(tasks).await;

    tracing::info!(
        "reconnect_clients: Reconnect abgeschlossen — {} IMAP, {} SMTP-Clients aktiv",
        state.imap_clients.read().len(),
        state.smtp_clients.read().len(),
    );
}

/// Reconnect a single account's IMAP + SMTP clients.
async fn reconnect_one_account(
    state: &AppState,
    acct: cache::accounts::AccountRecord,
    stored_imap_password: String,
    stored_smtp_password: String,
) {
    let account_id = acct.id;
    let account_name = acct.name.clone();

    // Decrypt passwords (transparently handles plaintext migration)
    let imap_password = match crypto::decrypt(&stored_imap_password) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(
                "Failed to decrypt IMAP password for account {} ({}): {}",
                account_id, account_name, e
            );
            return;
        }
    };
    let smtp_password = if stored_smtp_password.is_empty() {
        imap_password.clone()
    } else {
        match crypto::decrypt(&stored_smtp_password) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(
                    "Failed to decrypt SMTP password for account {} ({}): {}",
                    account_id, account_name, e
                );
                imap_password.clone()
            }
        }
    };

    // Migrate plaintext → encrypted if needed
    if !crypto::is_encrypted(&stored_imap_password) {
        if let Ok(encrypted) = crypto::encrypt(&imap_password) {
            if let Some(conn) = state.cache_db.lock().as_ref() {
                let _ = conn.execute(
                    "UPDATE accounts SET password = ?1 WHERE id = ?2",
                    rusqlite::params![encrypted, account_id],
                );
                tracing::info!(
                    "Migrated plaintext IMAP password to encrypted for account {} ({})",
                    account_id, account_name
                );
            }
        }
    }

    tracing::info!(
        "reconnect_clients: Verbinde Konto {} (ID={}, IMAP={}:{}, SMTP={}:{})",
        account_name, account_id,
        acct.imap_host, acct.imap_port,
        acct.smtp_host, acct.smtp_port,
    );

    // ── IMAP reconnect with timeout ──────────────────────
    let imap_client = Arc::new(ImapClient::new(
        acct.imap_host.clone(), acct.imap_port,
        acct.username.clone(), imap_password.clone(),
        acct.imap_ssl,
    ));

    let imap_result = timeout(
        std::time::Duration::from_secs(35),
        imap_client.connect(),
    )
    .await;

    match imap_result {
        Ok(Ok(())) => {
            state.imap_clients.write().insert(account_id as u32, imap_client);
            tracing::info!(
                "reconnect_clients: IMAP verbunden für {} (Konto {})",
                account_name, account_id
            );
        }
        Ok(Err(e)) => {
            tracing::warn!(
                "reconnect_clients: IMAP-Verbindung fehlgeschlagen für {} (Konto {}): {}",
                account_name, account_id, e
            );
        }
        Err(_) => {
            tracing::warn!(
                "reconnect_clients: IMAP-Timeout für {} (Konto {}) nach 35s",
                account_name, account_id
            );
        }
    }

    // ── SMTP client creation ─────────────────────────────
    let smtp_user = if acct.smtp_username.is_empty() { acct.username.clone() } else { acct.smtp_username.clone() };
    match SmtpClient::new(SmtpConfig {
        host: acct.smtp_host.clone(),
        port: acct.smtp_port,
        username: smtp_user,
        password: smtp_password,
        use_tls: acct.smtp_tls,
        sender_name: acct.sender_name.clone(),
        sender_email: acct.sender_email.clone(),
    }) {
        Ok(smtp_client) => {
            state.smtp_clients.write().insert(account_id as u32, Arc::new(smtp_client));
            tracing::info!(
                "reconnect_clients: SMTP-Client erstellt für {} (Konto {})",
                account_name, account_id
            );
        }
        Err(e) => {
            tracing::warn!(
                "reconnect_clients: SMTP-Client-Erstellung fehlgeschlagen für {} (Konto {}): {}",
                account_name, account_id, e
            );
        }
    }
}
