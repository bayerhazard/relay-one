//! Account endpoints: connect, list, delete.

use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::cache;
use crate::crypto;
use crate::db::with_db;
use crate::imap::client::ImapClient;
use crate::smtp::client::{SmtpClient, SmtpConfig};
use crate::AppState;

use super::{ApiError, ApiResult};

#[derive(Serialize)]
pub struct AccountInfo {
    pub id: i64,
    pub name: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    pub smtp_username: String,
    pub connected: bool,
    pub sender_name: String,
    pub sender_email: String,
    pub sync_mode: String,
    pub trash_retention_days: i64,
    pub imap_insecure: bool,
}

#[derive(Deserialize)]
pub struct ConnectAccountRequest {
    pub name: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_ssl: bool,
    pub imap_insecure: Option<bool>,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_tls: bool,
    pub imap_username: String,
    pub imap_password: String,
    pub smtp_username: String,
    pub smtp_password: String,
    pub sender_name: String,
    pub sender_email: String,
}

/// `POST /api/v1/accounts` — validate, connect, persist a new account.
pub async fn connect_account(
    State(state): State<AppState>,
    Json(req): Json<ConnectAccountRequest>,
) -> ApiResult<AccountInfo> {
    tracing::info!(
        "connect_account body: name={:?} imap_host={:?} imap_port={} imap_ssl={} smtp_host={:?} smtp_port={} smtp_tls={} imap_username={:?} imap_password_len={} smtp_username={:?} smtp_password_len={} sender_name={:?} sender_email={:?}",
        req.name, req.imap_host, req.imap_port, req.imap_ssl,
        req.smtp_host, req.smtp_port, req.smtp_tls,
        req.imap_username, req.imap_password.len(),
        req.smtp_username, req.smtp_password.len(),
        req.sender_name, req.sender_email,
    );
    validate_account_input(
        &req.name, &req.imap_host, req.imap_port, &req.smtp_host, req.smtp_port,
        &req.imap_username, &req.sender_email,
    )?;

    // Step 1: Validate IMAP
    let imap_insecure = req.imap_insecure.unwrap_or(false);
    let imap_client = Arc::new(ImapClient::new_with_options(
        req.imap_host.clone(),
        req.imap_port,
        req.imap_username.clone(),
        req.imap_password.clone(),
        req.imap_ssl,
        imap_insecure,
    ));
    imap_client
        .connect()
        .await
        .map_err(|e| ApiError(format!("IMAP-Verbindung fehlgeschlagen: {}", e)))?;

    // Step 2: Validate SMTP
    let smtp_user = if req.smtp_username.is_empty() {
        req.imap_username.clone()
    } else {
        req.smtp_username.clone()
    };
    let smtp_pass = if req.smtp_password.is_empty() {
        req.imap_password.clone()
    } else {
        req.smtp_password.clone()
    };
    let smtp_client = SmtpClient::new(SmtpConfig {
        host: req.smtp_host.clone(),
        port: req.smtp_port,
        username: smtp_user,
        password: smtp_pass.clone(),
        use_tls: req.smtp_tls,
        sender_name: req.sender_name.clone(),
        sender_email: req.sender_email.clone(),
    })
    .map_err(|e| ApiError(format!("SMTP-Konfiguration fehlgeschlagen: {}", e)))?;

    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        smtp_client.test_connection(),
    )
    .await
    .map_err(|_| ApiError("SMTP-Timeout nach 30s".into()))?
    .map_err(|e| ApiError(format!("SMTP-Verbindung fehlgeschlagen: {}", e)))?;

    let smtp_client = Arc::new(smtp_client);

    // Step 3: Persist
    let encrypted_imap_password = crypto::encrypt(&req.imap_password)
        .map_err(ApiError)?;
    let encrypted_smtp_password =
        crypto::encrypt(&smtp_pass).map_err(ApiError)?;
    let account_id = with_db(&state, |conn| {
        Ok(cache::accounts::create_account(
            conn, &req.name, &req.imap_host, req.imap_port, req.imap_ssl,
            &req.smtp_host, req.smtp_port, req.smtp_tls,
            &req.imap_username, &encrypted_imap_password,
            &req.smtp_username, &encrypted_smtp_password,
            &req.sender_name, &req.sender_email,
            imap_insecure,
        )
        .map_err(|e| e.to_string())?)
    })?;

    state
        .imap_clients
        .write()
        .insert(account_id as u32, imap_client);
    state
        .smtp_clients
        .write()
        .insert(account_id as u32, smtp_client);

    Ok(Json(AccountInfo {
        id: account_id,
        name: req.name,
        imap_host: req.imap_host,
        imap_port: req.imap_port,
        smtp_host: req.smtp_host,
        smtp_port: req.smtp_port,
        username: req.imap_username,
        smtp_username: req.smtp_username,
        connected: true,
        sender_name: req.sender_name.clone(),
        sender_email: req.sender_email,
        sync_mode: "mirror".to_string(),
        trash_retention_days: 30,
        imap_insecure,
    }))
}

/// `GET /api/v1/accounts`
pub async fn list_accounts(State(state): State<AppState>) -> ApiResult<Vec<AccountInfo>> {    let accounts = with_db(&state, |conn| {
        let accounts = cache::accounts::list_accounts(conn).map_err(|e| e.to_string())?;
        let imap_clients = state.imap_clients.read();
        Ok(accounts
            .into_iter()
            .map(|a| AccountInfo {
                id: a.id,
                name: a.name,
                imap_host: a.imap_host,
                imap_port: a.imap_port,
                smtp_host: a.smtp_host,
                smtp_port: a.smtp_port,
                username: a.username,
                smtp_username: a.smtp_username,
                connected: imap_clients.contains_key(&(a.id as u32)),
                sender_name: a.sender_name,
                sender_email: a.sender_email,
                sync_mode: a.sync_mode,
                trash_retention_days: a.trash_retention_days,
                imap_insecure: a.imap_insecure,
            })
            .collect())
    })?;
    Ok(Json(accounts))
}

/// `PATCH /api/v1/accounts/{id}` — update sync mode / trash retention.
#[derive(Deserialize)]
pub struct UpdateAccountRequest {
    pub account_id: i64,
    #[serde(default)]
    pub sync_mode: Option<String>,
    #[serde(default)]
    pub trash_retention_days: Option<i64>,
    #[serde(default)]
    pub imap_insecure: Option<bool>,
}

pub async fn update_account(
    State(state): State<AppState>,
    Json(req): Json<UpdateAccountRequest>,
) -> ApiResult<serde_json::Value> {
    let id = req.account_id;
    let current = with_db(&state, |conn| {
        cache::accounts::get_account(conn, id).map_err(|e| e.to_string())
    })?;
    let Some(account) = current else {
        return Err(ApiError(format!("Konto {} nicht gefunden", id)));
    };
    let was_archive = account.sync_mode == "archive";
    let mode = req.sync_mode.unwrap_or(account.sync_mode);
    let retention = req.trash_retention_days.unwrap_or(account.trash_retention_days);
    let insecure = req.imap_insecure.unwrap_or(account.imap_insecure);
    with_db(&state, |conn| {
        cache::accounts::update_account_settings(conn, id as i64, &mode, retention).map_err(|e| e.to_string())
    })?;
    with_db(&state, |conn| {
        cache::accounts::update_imap_insecure(conn, id as i64, insecure).map_err(|e| e.to_string())
    })?;
    tracing::info!("Konto {}: sync_mode={}, trash_retention_days={}, imap_insecure={}", id, mode, retention, insecure);

    // imap_insecure changed → rebuild the IMAP client so the new TLS mode
    // takes effect immediately without a restart.
    if insecure != account.imap_insecure {
        if let Some(old) = state.imap_clients.write().remove(&(id as u32)) {
            drop(old);
        }
        let acct = cache::accounts::get_account(&*state.cache_db.lock().as_ref().ok_or(ApiError("DB nicht initialisiert".into()))?, id as i64)
            .map_err(|e| ApiError(e.to_string()))?
            .ok_or(ApiError(format!("Konto {} nicht gefunden", id)))?;
        let pass = cache::accounts::get_account_password(&*state.cache_db.lock().as_ref().ok_or(ApiError("DB nicht initialisiert".into()))?, id as i64)
            .map_err(|e| ApiError(e.to_string()))?
            .unwrap_or_default();
        let decrypted = crate::crypto::decrypt(&pass).unwrap_or(pass);
        let client = Arc::new(crate::imap::client::ImapClient::new_with_options(
            acct.imap_host.clone(), acct.imap_port, acct.username.clone(),
            decrypted, acct.imap_ssl, acct.imap_insecure,
        ));
        state.imap_clients.write().insert(id as u32, client);
        tracing::info!("Konto {}: IMAP-Client mit imap_insecure={} neu gebaut", id, insecure);
    }

    // Switching to archive mode → enqueue an EML backfill for cached mails
    // that do not have an archive file yet.
    if mode == "archive" && !was_archive {
        state.sync_queue.enqueue(crate::sync::queue::SyncTask {
            account_id: id as u32,
            task_type: crate::sync::queue::SyncTaskType::BackfillEmails,
            created_at: tokio::time::Instant::now(),
            retries: 0,
            max_retries: 2,
            priority: 3,
        }).await;
        tracing::info!("Konto {}: EML-Backfill für archive-Modus eingereiht", id);
    }

    Ok(Json(serde_json::json!({ "ok": true, "sync_mode": mode, "trash_retention_days": retention })))
}
/// `DELETE /api/v1/accounts/{id}`
pub async fn delete_account(
    State(state): State<AppState>,
    Json(req): Json<crate::api::messages::MessageActionRequest>,
) -> ApiResult<()> {
    let account_id = req.account_id;
    with_db(&state, |conn| {
        cache::accounts::delete_account(conn, account_id as i64).map_err(|e| e.to_string())
    })?;
    state.imap_clients.write().remove(&account_id);
    state.smtp_clients.write().remove(&account_id);
    Ok(Json(()))
}

/// Validates account input fields before attempting any network connection.
fn validate_account_input(
    name: &str,
    imap_host: &str,
    imap_port: u16,
    smtp_host: &str,
    smtp_port: u16,
    username: &str,
    sender_email: &str,
) -> Result<(), ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError("Kontoname darf nicht leer sein".into()));
    }
    if imap_port == 0 {
        return Err(ApiError("IMAP-Port muss zwischen 1 und 65535 liegen".into()));
    }
    if smtp_port == 0 {
        return Err(ApiError("SMTP-Port muss zwischen 1 und 65535 liegen".into()));
    }
    fn valid_hostname(host: &str) -> Result<(), ApiError> {
        let trimmed = host.trim();
        if trimmed.is_empty() {
            return Err(ApiError("Darf nicht leer sein".into()));
        }
        if trimmed.len() > 253 {
            return Err(ApiError("Darf maximal 253 Zeichen lang sein".into()));
        }
        if trimmed.contains(' ') {
            return Err(ApiError("Darf keine Leerzeichen enthalten".into()));
        }
        Ok(())
    }
    valid_hostname(imap_host)?;
    valid_hostname(smtp_host)?;
    if username.trim().is_empty() {
        return Err(ApiError("Benutzername darf nicht leer sein".into()));
    }
    if !sender_email.trim().is_empty() && !sender_email.contains('@') {
        return Err(ApiError("Absender-E-Mail muss eine gültige E-Mail sein".into()));
    }
    Ok(())
}
