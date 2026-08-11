use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{sleep, Instant};

use crate::security::pii;
use crate::security::priority;
use crate::sync::queue::{SyncQueue, SyncTask, SyncTaskType};
use crate::AppState;

/// Interval between proactive IMAP connection health checks.
/// Each client is pinged at most once per interval to detect stale connections.
const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(60);

/// How often the local message cache is pruned to bound disk growth.
const RETENTION_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60); // 6h
/// Legacy default (pre-v3): cached messages older than this become
/// prune-eligible. Overridden by the `retention_days` setting — 0 disables
/// pruning entirely (archive mode).
#[allow(dead_code)]
const RETENTION_DAYS: u32 = 90;
/// Always keep at least this many newest messages per account, regardless of age.
const RETENTION_KEEP_MINIMUM: u32 = 200;

/// How often to scan all remote UIDs and remove locally cached messages that
/// no longer exist on the IMAP server (deleted from another client).
const REMOVAL_CHECK_INTERVAL: Duration = Duration::from_secs(10 * 60); // 10 min

/// IMAP IDLE window per account (seconds). Servers drop IDLE after ~29 min;
/// 20s keeps the session fresh and the poll fallback tight.
const IDLE_TIMEOUT_SECS: u64 = 20;

/// How often to refresh IMAP flags (\Seen) for existing cached messages to
/// detect read/unread changes made from other clients (phone, webmail, …).
const FLAG_REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60); // 5 min

/// Provider trash folders (by name, per provider locale) that are mapped onto
/// the single local "Trash" folder. Prevents duplicate Papierkorb/Trash/
/// Gelöscht/Deleted-Messages folders showing up in the UI.
const PROVIDER_TRASH_NAMES: &[&str] = &[
    "Trash",
    "Gelöscht",
    "Gelöschte Elemente",
    "Papierkorb",
    "Deleted Messages",
    "Deleted",
    "Deleted Items",
    "INBOX.Trash",
    "INBOX.Gelöscht",
    "INBOX.Papierkorb",
    "INBOX.Deleted Messages",
    "INBOX.Deleted",
];

/// Map a provider folder onto the local storage folder: every provider trash
/// folder is stored inside the single local "Trash".
fn storage_folder_name(folder_name: &str, tag: &str) -> String {
    if tag == "trash" || PROVIDER_TRASH_NAMES.iter().any(|t| folder_name.eq_ignore_ascii_case(t)) {
        "Trash".to_string()
    } else {
        folder_name.to_string()
    }
}

pub async fn start_periodic_sync(state: Arc<AppState>, mut shutdown_rx: mpsc::Receiver<()>) {
    let queue = state.sync_queue.clone();
    let base_interval = Duration::from_secs(20);
    let max_interval = Duration::from_secs(300);
    let mut last_health_checks: HashMap<u32, Instant> = HashMap::new();
    // Run the first prune shortly after startup, then on RETENTION_INTERVAL.
    let mut last_retention: Option<Instant> = None;
    // Track last server-side deletion cleanup (runs on REMOVAL_CHECK_INTERVAL).
    let mut last_removal_check: Option<Instant> = None;
    // Track last IMAP flag refresh (runs on FLAG_REFRESH_INTERVAL).
    let mut last_flag_refresh: Option<Instant> = None;
    // Smart sync: exponential backoff when no new mail arrives (20s → 40s → 80s → 160s → 300s).
    // Resets to base_interval as soon as any new message is found.
    let mut consecutive_empty: u32 = 0;

    // AI summaries run on a DEDICATED worker channel. LLM calls take seconds
    // each — queueing them on the sync queue would block the whole IMAP sync
    // cycle behind hundreds of summarization tasks (observed: 200+ tasks →
    // no sync for 20+ minutes).
    let (ai_tx, ai_rx) = mpsc::channel::<SyncTask>(512);
    {
        let worker_state = state.clone();
        tokio::spawn(async move {
            run_ai_summary_worker(worker_state, ai_rx).await;
        });
    }

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                tracing::info!("Sync-Loop wurde gestoppt");
                break;
            }
            result = do_sync_cycle(&state, &queue, base_interval, &mut last_health_checks, &ai_tx) => {
                let new_count = match result {
                    Ok(count) => count,
                    Err(e) => {
                        tracing::error!("Sync-Zyklus fehlgeschlagen: {}", e);
                        0
                    }
                };
                // Periodic cache retention (cheap no-op between intervals).
                let due = last_retention
                    .map(|t| Instant::now().duration_since(t) >= RETENTION_INTERVAL)
                    .unwrap_or(true);
                if due {
                    last_retention = Some(Instant::now());
                    run_retention(&state);
                }
                // Periodic server-side deletion cleanup
                let removal_due = last_removal_check
                    .map(|t| Instant::now().duration_since(t) >= REMOVAL_CHECK_INTERVAL)
                    .unwrap_or(true);
                if removal_due {
                    last_removal_check = Some(Instant::now());
                    run_removal_check(&state).await;
                }
                // Periodic IMAP flag refresh (\Seen sync from other clients)
                let flag_due = last_flag_refresh
                    .map(|t| Instant::now().duration_since(t) >= FLAG_REFRESH_INTERVAL)
                    .unwrap_or(false);
                if flag_due {
                    last_flag_refresh = Some(Instant::now());
                    run_flag_refresh(&state).await;
                }
                // Delete-queue worker: verify → hard/soft provider delete.
                run_delete_queue(&state).await;
                // Local-trash retention (archive mode): remove local copies
                // older than the per-account retention window.
                run_trash_retention(&state);
                // Smart interval: back off on empty cycles, reset on new mail
                let wait_time = calculate_backoff(
                    new_count,
                    base_interval,
                    max_interval,
                    &mut consecutive_empty,
                );
                tracing::debug!(
                    "Sync-Intervall: {:?} (consecutive_empty: {}, new: {})",
                    wait_time, consecutive_empty, new_count
                );
                sleep(wait_time).await;
            }
        }
    }
}

/// Prune the local message cache and reclaim disk space if rows were removed.
/// Prune runs under the cache_db mutex (fast DELETE). VACUUM runs outside the
/// mutex using a separate connection — VACUUM cannot run inside a transaction
/// and would otherwise block all UI operations for seconds.
///
/// Controlled by the `retention_days` setting (default 0 = archive mode):
/// 0 disables pruning entirely — local mail is never deleted automatically.
fn run_retention(state: &AppState) {
    let retention_days = {
        let db_guard = state.cache_db.lock();
        let Some(conn) = db_guard.as_ref() else { return; };
        match crate::cache::settings::get_retention_days(conn) {
            Ok(days) => days,
            Err(e) => {
                tracing::warn!("Cache-Retention: Settings unlesbar: {}", e);
                return;
            }
        }
    };
    if retention_days == 0 {
        tracing::debug!("Cache-Retention: deaktiviert (retention_days=0, Archiv-Modus)");
        return;
    }

    let pruned = {
        let db_guard = state.cache_db.lock();
        let Some(conn) = db_guard.as_ref() else { return; };
        match crate::cache::messages::prune_old_messages(conn, retention_days, RETENTION_KEEP_MINIMUM) {
            Ok(0) => 0,
            Ok(n) => {
                tracing::info!("Cache-Retention: {} alte Nachrichten entfernt", n);
                n
            }
            Err(e) => {
                tracing::warn!("Cache-Retention: Pruning fehlgeschlagen: {}", e);
                return;
            }
        }
    };

    // VACUUM outside mutex — uses a separate connection to avoid blocking UI
    if pruned > 0 {
        if let Some(db_path_str) = state.db_path.lock().as_ref() {
            match rusqlite::Connection::open(db_path_str) {
                Ok(vacuum_conn) => {
                    if let Err(e) = crate::cache::messages::vacuum(&vacuum_conn) {
                        tracing::warn!("Cache-Retention: VACUUM fehlgeschlagen: {}", e);
                    }
                }
                Err(e) => {
                    tracing::warn!("Cache-Retention: konnte VACUUM-Connection nicht öffnen: {}", e);
                }
            }
        }
    }
}

/// Refresh IMAP \Seen flags for all cached messages to detect read/unread
/// changes made from other clients (phone, webmail, …).
async fn run_flag_refresh(state: &AppState) {
    let clients: Vec<(u32, Arc<crate::imap::client::ImapClient>)> = {
        let guard = state.imap_clients.read();
        guard.iter().map(|(k, v)| (*k, v.clone())).collect()
    };

    for (account_id, client) in &clients {
        if !client.is_connected().await {
            if let Err(e) = client.reconnect().await {
                tracing::warn!(
                    "flag_refresh: reconnect fuer account {} fehlgeschlagen: {}",
                    account_id, e
                );
                continue;
            }
        }

        let folders = match client.list_folders().await {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(
                    "flag_refresh: list_folders fuer account {} fehlgeschlagen: {}",
                    account_id, e
                );
                continue;
            }
        };

        for (folder_name, _raw_name, _, tag) in &folders {
            if tag == "noselect" {
                continue;
            }
            // Provider trash folders are stored under the local "Trash".
            let storage_folder = storage_folder_name(folder_name, tag);
            // NOTE: pass the DECODED name — select_folder() re-encodes to
            // UTF-7 internally. Passing raw_name (already UTF-7) would
            // double-encode (& → &-) and fail with "unknown folder".

            if let Err(e) = client.select_folder(folder_name).await {
                tracing::warn!(
                    "flag_refresh: select_folder '{}' fuer account {} fehlgeschlagen: {}",
                    folder_name, account_id, e
                );
                continue;
            }

            let local_msgs = {
                let db_guard = state.cache_db.lock();
                let Some(conn) = db_guard.as_ref() else { continue; };
                match crate::cache::messages::get_messages_with_uids_for_folder(
                    conn, *account_id as i64, &storage_folder,
                ) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(
                            "flag_refresh: get_messages_with_uids_for_folder '{}' (account {}) fehlgeschlagen: {}",
                            folder_name, account_id, e
                        );
                        continue;
                    }
                }
            };

            if local_msgs.is_empty() {
                continue;
            }

            let uid_list: Vec<String> = local_msgs.iter().map(|uid| uid.to_string()).collect();
            let uid_set = uid_list.join(",");

            let fetched = match client.fetch_flags(&uid_set).await {
                    Ok(f) => f,
                    Err(e) => {
                        tracing::warn!(
                            "flag_refresh: fetch_flags fuer '{}' (account {}) fehlgeschlagen: {}",
                            folder_name, account_id, e
                        );
                        continue;
                    }
                };

            let updated = {
                let mut db_guard = state.cache_db.lock();
                let Some(conn) = db_guard.as_mut() else { continue; };
                let mut count = 0;
                let tx = match conn.transaction() {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!("flag_refresh: transaction fehlgeschlagen: {}", e);
                        continue;
                    }
                };
                for (uid, is_read, is_flagged) in &fetched {
                    if let Err(e) = crate::cache::messages::update_is_read(
                        &tx, *account_id as i64, *uid as i64, *is_read,
                    ) {
                        tracing::warn!("flag_refresh: update_is_read uid={} fehlgeschlagen: {}", uid, e);
                    } else {
                        count += 1;
                    }
                    if let Err(e) = crate::cache::messages::update_is_flagged(
                        &tx, *account_id as i64, *uid as i64, *is_flagged,
                    ) {
                        tracing::warn!("flag_refresh: update_is_flagged uid={} fehlgeschlagen: {}", uid, e);
                    }
                }
                match tx.commit() {
                    Ok(()) => count,
                    Err(e) => {
                        tracing::warn!("flag_refresh: commit fehlgeschlagen: {}", e);
                        0
                    }
                }
            };

            if updated > 0 {
                tracing::info!(
                    "flag_refresh: {} Flags in '{}' (account {}) aktualisiert",
                    updated, folder_name, account_id
                );
            }
        }
    }
}

/// Scan all IMAP folders for each account and remove local messages whose
/// UIDs no longer exist on the server (deleted from another client).
/// Local-trash retention (archive mode). Local copies of messages in the
/// "Trash" folder older than the account's `trash_retention_days` are removed
/// (index row + EML file). The provider copy is already gone (delete queue).
/// Runs cheaply: only rows in the Trash folder are touched.
fn run_trash_retention(state: &AppState) {
    let (expired, _accounts) = {
        let db_guard = state.cache_db.lock();
        let Some(conn) = db_guard.as_ref() else { return; };
        // (message_id, account_id, uid, raw_path, retention_days)
        let mut stmt = match conn.prepare(
            "SELECT m.id, m.account_id, m.uid, m.raw_path, a.trash_retention_days
             FROM messages m
             JOIN folders f ON f.id = m.folder_id
             JOIN accounts a ON a.id = m.account_id
             WHERE f.name = 'Trash'
               AND m.updated_at < datetime('now', '-' || a.trash_retention_days || ' days')",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Trash-Retention: Query fehlgeschlagen: {}", e);
                return;
            }
        };
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            });
        let mut expired = Vec::new();
        let mut accounts = std::collections::HashSet::new();
        match rows {
            Ok(iter) => {
                for r in iter {
                    if let Ok((mid, acct, u, rp, days)) = r {
                        if days > 0 {
                            expired.push((mid, acct, u, rp));
                            accounts.insert(acct);
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Trash-Retention: Zeilen fehlgeschlagen: {}", e);
                return;
            }
        }
        (expired, accounts)
    };

    if expired.is_empty() {
        return;
    }

    let mut removed_eml = 0usize;
    for (_message_id, account_id, uid, raw_path) in &expired {
        // Remove the EML archive file if present (local source of truth gone
        // by user intent after the retention window — provider already deleted).
        if let Some(rel) = raw_path {
            let abs = state.data_root.join(rel);
            if abs.exists() {
                if std::fs::remove_file(&abs).is_ok() {
                    removed_eml += 1;
                }
            }
        }
        let db_guard = state.cache_db.lock();
        if let Some(conn) = db_guard.as_ref() {
            let _ = crate::cache::messages::delete_message(conn, *account_id, *uid);
        }
        tracing::info!(
            "Trash-Retention: lokale Kopie uid {} (Konto {}) nach Ablauf entfernt",
            uid, account_id
        );
    }
    let _ = removed_eml;
}

/// Delete-queue worker (Concept §5). Only rows enqueued by explicit user
/// action are processed. For each pending/failed row:///   1. Verify the local archive guarantee (EML exists + hash matches).
///   2. verified → hard delete (STORE \Deleted + EXPUNGE) on the provider.
///   3. not verified → soft fallback (MOVE into Provider-Trash).
///   4. Provider failure → mark failed (retried next cycle, max 5 attempts).
async fn run_delete_queue(state: &AppState) {
    let rows = {
        let db_guard = state.cache_db.lock();
        let Some(conn) = db_guard.as_ref() else { return; };
        match crate::cache::delete_queue::list_by_state(conn, None) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("delete_queue: Liste fehlgeschlagen: {}", e);
                return;
            }
        }
    };

    for row in rows {
        // Local-only folders (mbox-import/migration targets, e.g. "Auto",
        // "Beta Tests") have NO provider copy — there is nothing to delete on
        // the server. Skip the attempt counter too: these rows are done by
        // definition and must never linger as "failed".
        let is_local_folder = {
            let db_guard = state.cache_db.lock();
            let Some(conn) = db_guard.as_ref() else { continue; };
            crate::cache::messages::is_local_only_folder(conn, row.account_id, &row.folder)
                .unwrap_or(false)
        };
        if is_local_folder {
            let db_guard = state.cache_db.lock();
            if let Some(conn) = db_guard.as_ref() {
                let _ = crate::cache::delete_queue::mark_deleted(conn, row.id);
            }
            tracing::info!(
                "delete_queue {}: Ordner '{}' ist lokal (kein Provider-Gegenstück) — Eintrag abgeschlossen",
                row.id, row.folder
            );
            continue;
        }

        if row.attempts >= 5 {
            continue; // give up permanently; user reviews in UI
        }

        // 1. Verify local archive guarantee.
        let verified = {
            let db_guard = state.cache_db.lock();
            let conn = db_guard.as_ref().ok_or("DB nicht initialisiert");
            match conn {
                Ok(c) => crate::cache::delete_queue::verify_archive_guarantee(c, &state.data_root, row.message_id),
                Err(e) => Err(e.to_string()),
            }
        };
        let (ok, _path, _sha) = verified.unwrap_or((false, None, None));

        // 2. Mark verified before touching the provider (guarantee proven).
        {
            let db_guard = state.cache_db.lock();
            if let Some(conn) = db_guard.as_ref() {
                let _ = crate::cache::delete_queue::mark_verified(conn, row.id);
            }
        }

        // 3. Provider delete (hard if verified, else soft fallback).
        let client = {
            let guard = state.imap_clients.read();
            guard.get(&(row.account_id as u32)).cloned()
        };
        let Some(client) = client else {
            tracing::warn!("delete_queue: kein IMAP-Client für Konto {}", row.account_id);
            continue;
        };
        if !client.is_connected().await {
            if let Err(e) = client.reconnect().await {
                let db_guard = state.cache_db.lock();
                if let Some(conn) = db_guard.as_ref() {
                    let _ = crate::cache::delete_queue::mark_failed(conn, row.id, &format!("reconnect: {e}"));
                }
                continue;
            }
        }

        let result = if ok {
            client.hard_delete_message(row.uid as u32, &row.folder).await
        } else {
            tracing::warn!(
                "delete_queue {}: Verify-Garantie fehlt (uid {}, Konto {}) — weiches Löschen (Provider-Trash)",
                row.id, row.uid, row.account_id
            );
            client.move_message(row.uid as u32, &row.folder, "Trash").await
        };

        match result {
            Ok(()) => {
                let db_guard = state.cache_db.lock();
                if let Some(conn) = db_guard.as_ref() {
                    let _ = crate::cache::delete_queue::mark_deleted(conn, row.id);
                }
                tracing::info!("delete_queue {}: Provider-Kopie entfernt (uid {})", row.id, row.uid);
            }
            Err(e) => {
                let db_guard = state.cache_db.lock();
                if let Some(conn) = db_guard.as_ref() {
                    let _ = crate::cache::delete_queue::mark_failed(conn, row.id, &e.to_string());
                }
                tracing::warn!("delete_queue {}: Provider-Löschung fehlgeschlagen: {}", row.id, e);
            }
        }
    }
}

async fn run_removal_check(state: &AppState) {
    // Archive mode: local copies are never deleted because the provider
    // deleted the mail. Only run when explicitly enabled (default off).
    let enabled = {
        let db_guard = state.cache_db.lock();
        let Some(conn) = db_guard.as_ref() else { return; };
        match crate::cache::settings::get_removal_check_enabled(conn) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("removal_check: Settings unlesbar: {}", e);
                return;
            }
        }
    };
    if !enabled {
        tracing::debug!("removal_check: deaktiviert (Archiv-Modus)");
        return;
    }

    let clients: Vec<(u32, Arc<crate::imap::client::ImapClient>)> = {
        let guard = state.imap_clients.read();
        guard.iter().map(|(k, v)| (*k, v.clone())).collect()
    };

    for (account_id, client) in &clients {
        if !client.is_connected().await {
            if let Err(e) = client.reconnect().await {
                tracing::warn!(
                    "removal_check: reconnect fuer account {} fehlgeschlagen: {}",
                    account_id, e
                );
                continue;
            }
        }

        let folders = match client.list_folders().await {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(
                    "removal_check: list_folders fuer account {} fehlgeschlagen: {}",
                    account_id, e
                );
                continue;
            }
        };

        for (folder_name, _raw_name, _, tag) in &folders {
            if tag == "noselect" {
                continue;
            }
            // Decoded name — select_folder() re-encodes to UTF-7 internally
            // (raw_name would be double-encoded → "unknown folder").
            if let Err(e) = client.select_folder(folder_name).await {
                tracing::warn!(
                    "removal_check: select_folder '{}' fuer account {} fehlgeschlagen: {}",
                    folder_name, account_id, e
                );
                continue;
            }

            let server_uids = match client.fetch_all_uids().await {
                Ok(uids) => uids,
                Err(e) => {
                    tracing::warn!(
                        "removal_check: fetch_all_uids fuer '{}' (account {}) fehlgeschlagen: {}",
                        folder_name, account_id, e
                    );
                    continue;
                }
            };

            let deleted = {
                let db_guard = state.cache_db.lock();
                let Some(conn) = db_guard.as_ref() else { continue; };
                // Local-only folders are never mirrors of an IMAP folder —
                // never prune them against server UIDs (archive mode).
                let is_local_folder = crate::cache::messages::is_local_only_folder(
                    conn,
                    *account_id as i64,
                    folder_name,
                )
                .unwrap_or(false);
                if is_local_folder {
                    continue;
                }
                match crate::cache::messages::delete_messages_not_in(
                    conn, *account_id as i64, folder_name, &server_uids,
                ) {
                    Ok(n) => n,
                    Err(e) => {
                        tracing::warn!(
                            "removal_check: delete_messages_not_in fuer '{}' (account {}) fehlgeschlagen: {}",
                            folder_name, account_id, e
                        );
                        continue;
                    }
                }
            };

            if deleted > 0 {
                tracing::info!(
                    "removal_check: {} Nachrichten aus '{}' (account {}) entfernt (geloescht auf Server)",
                    deleted, folder_name, account_id
                );
            }
        }
    }
}

/// Exponential backoff based on consecutive empty sync cycles.
/// Resets to `base_interval` when new mail is found (`new_count > 0`).
/// Otherwise doubles each cycle: 1× → 2× → 4× → 8× → 16×, capped at `max_interval`.
fn calculate_backoff(
    new_count: usize,
    base_interval: Duration,
    max_interval: Duration,
    consecutive_empty: &mut u32,
) -> Duration {
    if new_count > 0 {
        *consecutive_empty = 0;
        base_interval
    } else {
        *consecutive_empty = consecutive_empty.saturating_add(1);
        std::cmp::min(
            base_interval * 2u32.pow((*consecutive_empty).min(4)),
            max_interval,
        )
    }
}

#[allow(dead_code)]
fn calculate_wait_time(fail_count: u32, base_interval: Duration) -> Duration {
    if fail_count >= 5 {
        std::cmp::min(
            base_interval * 2u32.pow(fail_count.saturating_sub(4).min(5)),
            Duration::from_secs(300),
        )
    } else {
        base_interval
    }
}

async fn do_sync_cycle(
    state: &AppState,
    queue: &SyncQueue,
    _base_interval: Duration,
    last_health_checks: &mut HashMap<u32, Instant>,
    ai_tx: &mpsc::Sender<SyncTask>,
) -> Result<usize, String> {
    let imap_client_ids: Vec<(u32, Arc<crate::imap::client::ImapClient>)> = {
        let guard = state.imap_clients.read();
        guard.iter().map(|(k, v)| (*k, v.clone())).collect()
    };

    // ── Health check phase ──────────────────────────────────────────────
    // Proactively ping each connected client if the health check interval
    // has elapsed. Detects stale connections (connected flag true but TCP
    // pipe broken) before the sync work begins.
    let now = Instant::now();
    for (account_id, client) in &imap_client_ids {
        let last = last_health_checks.get(account_id).copied().unwrap_or(Instant::now() - HEALTH_CHECK_INTERVAL - Duration::from_secs(1));
        if now.duration_since(last) < HEALTH_CHECK_INTERVAL {
            continue;
        }
        last_health_checks.insert(*account_id, now);

        if !client.is_connected().await {
            // Already disconnected — will be handled by the reconnect phase below
            continue;
        }

        let healthy = client.ping().await;
        if healthy {
            tracing::debug!(
                "IMAP health check OK fuer account {}",
                account_id
            );
        } else {
            tracing::warn!(
                "IMAP health check fehlgeschlagen fuer account {} - Verbindung ist stale, leite Reconnect ein",
                account_id
            );
            if let Err(e) = client.reconnect().await {
                tracing::warn!(
                    "IMAP reconnect fuer account {} nach health check fehlgeschlagen: {}",
                    account_id,
                    e
                );
            } else {
                tracing::info!(
                    "IMAP reconnect fuer account {} nach health check erfolgreich",
                    account_id
                );
            }
        }
    }

    // ── Connection check + sync phase ───────────────────────────────────
    for (account_id, client) in &imap_client_ids {
        if !client.is_connected().await {
            if let Err(e) = client.reconnect().await {
                tracing::warn!("IMAP reconnect fuer account {} fehlgeschlagen: {}", account_id, e);
                continue;
            }
        }

        // IDLE fast-path: wait up to IDLE_TIMEOUT for an INBOX change. On a
        // mailbox change we enqueue FetchNew immediately (low latency); on
        // timeout the regular poll below still runs (fallback).
        let changed = client.idle_wait("INBOX", Duration::from_secs(IDLE_TIMEOUT_SECS)).await;
        if changed {
            tracing::debug!("IMAP IDLE: INBOX-Änderung für account {}", account_id);
            queue
                .enqueue(SyncTask {
                    account_id: *account_id,
                    task_type: SyncTaskType::FetchNew,
                    created_at: tokio::time::Instant::now(),
                    retries: 0,
                    max_retries: 3,
                    priority: 10,
                })
                .await;
            continue;
        }

        queue
            .enqueue(SyncTask {
                account_id: *account_id,
                task_type: SyncTaskType::FetchNew,
                created_at: tokio::time::Instant::now(),
                retries: 0,
                max_retries: 3,
                priority: 10,
            })
            .await;
    }

    // Enqueue background diff analysis (low priority, runs after mail sync)
    queue.enqueue(SyncTask {
        account_id: 0,
        task_type: SyncTaskType::AnalyzeDiff,
        created_at: tokio::time::Instant::now(),
        retries: 0,
        max_retries: 0,
        priority: 3,
    }).await;

    // Enqueue fingerprint refresh (lowest priority, runs after diff analysis)
    queue.enqueue(SyncTask {
        account_id: 0,
        task_type: SyncTaskType::RefreshFingerprint,
        created_at: tokio::time::Instant::now(),
        retries: 0,
        max_retries: 0,
        priority: 2,
    }).await;

    let pre_queue_size = queue.len().await;
    tracing::info!(
        account_count = imap_client_ids.len(),
        queue_size = pre_queue_size,
        "Sync-Zyklus: {} Konten, {} Tasks in Queue",
        imap_client_ids.len(),
        pre_queue_size
    );

    // Process all tasks in the queue, accumulating total new messages
    let mut total_new: usize = 0;
    while let Some(task) = queue.dequeue().await {
        let delay = queue.calculate_delay(task.retries);
        if delay > Duration::ZERO {
            sleep(delay).await;
        }

        match process_sync_task(state, &task, queue, ai_tx).await {
            Ok(count) => {
                total_new += count;
                queue.record_success().await;
            }
            Err(e) => {
                tracing::warn!(
                    "Sync task {:?} fehlgeschlagen (attempt {}): {}",
                    task.task_type,
                    task.retries + 1,
                    e
                );
                queue.record_failure().await;

                if task.retries < task.max_retries {
                    let mut retry_task = task.clone();
                    retry_task.retries += 1;
                    retry_task.priority = retry_task.priority.saturating_sub(1);
                    queue.enqueue(retry_task).await;
                }
            }
        }
    }

    tracing::debug!(
        "Sync-Zyklus beendet. {} neue Nachrichten (failures: {})",
        total_new,
        queue.failure_count().await
    );
    Ok(total_new)
}

async fn process_sync_task(
    state: &AppState,
    task: &SyncTask,
    queue: &SyncQueue,
    ai_tx: &mpsc::Sender<SyncTask>,
) -> Result<usize, String> {
    match &task.task_type {
        SyncTaskType::FetchNew => {
            let client = {
                let guard = state.imap_clients.read();
                guard
                    .get(&task.account_id)
                    .cloned()
                    .ok_or("IMAP-Client nicht gefunden")?
            };

            if !client.is_connected().await {
                client.reconnect().await.map_err(|e| e.to_string())?;
            }

            let folders = client.list_folders().await.map_err(|e| e.to_string())?;

            let mut total_new: usize = 0;
            for (folder_name, _raw_name, _, tag) in &folders {
                if tag == "noselect" {
                    continue;
                }

                let is_spam = ["Spam", "Junk", "Spamverdacht", "Junk E-Mail"]
                    .iter().any(|s| folder_name.eq_ignore_ascii_case(s));

                // Decoded name — select_folder() re-encodes to UTF-7
                // internally (raw_name would be double-encoded → "unknown
                // folder").

                // Provider trash folders (Gelöscht/Papierkorb/Deleted
                // Messages…) are stored inside the single local "Trash".
                let storage_folder = storage_folder_name(folder_name, tag);

                // NOTE: SPAM folders used to be skipped entirely in archive
                // mode ("stays exclusively on the provider"). The user wants
                // Spam handled like every other folder — cached and shown.
                // (AI summaries below are still skipped for spam.)

                // LOCAL folders (e.g. "Gesendet" after the local-only
                // conversion, or migration/import targets) are not mirrored
                // against the provider: skip the IMAP fetch + prune for them
                // so the local history is never touched by the sync.
                // Exception: provider trash folders map ONTO the local Trash
                // row (which is local_only) — they must still be fetched.
                let is_local_folder = if storage_folder == "Trash" {
                    false
                } else {
                    let db_guard = state.cache_db.lock();
                    let conn = db_guard
                        .as_ref()
                        .ok_or("Datenbank nicht initialisiert")?;
                    crate::cache::messages::is_local_only_folder(conn, task.account_id as i64, folder_name)
                        .unwrap_or(false)
                };
                if is_local_folder {
                    tracing::debug!(
                        "FetchNew: '{}' (account {}) ist ein lokaler Ordner — kein IMAP-Abgleich",
                        folder_name, task.account_id
                    );
                    continue;
                }

                // Per-folder error handling: a TagMismatch or transient error in
                // one folder must NOT abort the entire sync cycle. Log and continue.
                let sync_state = {
                    let db_guard = state.cache_db.lock();
                    let conn = db_guard
                        .as_ref()
                        .ok_or("Datenbank nicht initialisiert")?;
                    crate::cache::sync_state::get(conn, task.account_id as i64, &storage_folder)
                };
                let (max_uid, _modseq) = match sync_state {
                    Ok(s) => (s.last_uid, s.highest_modseq),
                    Err(rusqlite::Error::QueryReturnedNoRows) => (0, 0),
                    Err(e) => {
                        tracing::warn!(
                            "FetchNew: sync_state '{}' (account {}): {}",
                            storage_folder, task.account_id, e
                        );
                        continue;
                    }
                };

                if let Err(e) = client.select_folder(folder_name).await {
                    tracing::warn!(
                        "FetchNew: select_folder '{}' (account {}): {}",
                        folder_name, task.account_id, e
                    );
                    continue;
                }

                let messages = match client.fetch_recent(max_uid as u32, 50).await {
                    Ok(msgs) => msgs,
                    Err(e) => {
                        tracing::warn!(
                            "FetchNew: fetch_recent '{}' (account {}): {}",
                            folder_name, task.account_id, e
                        );
                        // TagMismatch resets the session — next folder will
                        // trigger auto-reconnect in with_session_blocking().
                        continue;
                    }
                };

                {
                    let mut db_guard = state.cache_db.lock();
                    let conn = db_guard
                        .as_mut()
                        .ok_or("Datenbank nicht initialisiert")?;

                    let tx = conn.transaction().map_err(|e| e.to_string())?;
                    for msg in &messages {
                        crate::cache::messages::save_message(&tx, task.account_id as i64, msg, &storage_folder)
                            .map_err(|e| e.to_string())?;
                    }
                    // Advance the per-folder sync cursor to the highest UID of
                    // THIS batch, so the next cycle continues with the next
                    // slice (`UID {last+1}:*`) instead of re-fetching the same
                    // messages or skipping older ones. Without this the sync
                    // never progresses past the first batch.
                    if let Some(max_uid) = messages.iter().map(|m| m.uid).max() {
                        crate::cache::sync_state::set(
                            &tx,
                            task.account_id as i64,
                            &storage_folder,
                            max_uid as i64,
                            0,
                        )
                        .map_err(|e| e.to_string())?;
                    }
                    tx.commit().map_err(|e| e.to_string())?;
                }

                // Cleanup: Remove locally cached messages that no longer exist on the IMAP server.
                // This handles the case where messages were deleted in another app (e.g., GMX Webmail).
                let server_uids = match client.fetch_all_uids().await {
                    Ok(uids) => uids,
                    Err(e) => {
                        tracing::warn!(
                            "FetchNew: fetch_all_uids '{}' (account {}): {}",
                            folder_name, task.account_id, e
                        );
                        continue;
                    }
                };

                {
                    let mut db_guard = state.cache_db.lock();
                    let conn = db_guard
                        .as_mut()
                        .ok_or("Datenbank nicht initialisiert")?;

                    // Local-only folders are NOT mirrors of an IMAP folder —
                    // never prune them against server UIDs (archive mode).
                    // Provider trash folders map onto the local Trash — also
                    // never pruned: deleted mails must stay in the local
                    // Trash even if the provider copy is gone.
                    let storage_folder = storage_folder_name(folder_name, tag);
                    if storage_folder == "Trash" {
                        continue;
                    }
                    let is_local_folder = crate::cache::messages::is_local_only_folder(
                        conn,
                        task.account_id as i64,
                        folder_name,
                    )
                    .unwrap_or(false);
                    if is_local_folder {
                        continue;
                    }

                    match crate::cache::messages::delete_messages_not_in(
                        conn,
                        task.account_id as i64,
                        folder_name,
                        &server_uids,
                    ) {
                        Ok(deleted) => {
                            if deleted > 0 {
                                tracing::info!(
                                    "Account {}: {} gelöschte Nachrichten in '{}' bereinigt",
                                    task.account_id,
                                    deleted,
                                    folder_name
                                );
                                // Notify frontend to refresh the message list
                                let _ = state.events.emit("messages-deleted", (task.account_id, folder_name, deleted));
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "FetchNew: delete_messages_not_in '{}' (account {}): {}",
                                folder_name, task.account_id, e
                            );
                        }
                    }
                }

                // Hybrid Body-Fetch: only fetch body for INBOX to keep sync fast.
                // Other folders rely on on-demand body fetch when the user clicks a message.
                // The raw RFC822 bytes are archived to disk (EML) — the local-first
                // source of truth for backup/export (Concept §3.1).
                let is_inbox = folder_name.eq_ignore_ascii_case("INBOX");
                if is_inbox {
                    for msg in &messages {
                        match client.fetch_body_with_raw(msg.uid).await {
                            Ok((body_text, body_html, raw)) => {
                                let raw_path = crate::cache::archive::write_eml(
                                    &state.data_root,
                                    task.account_id as i64,
                                    msg.uid,
                                    Some(msg.envelope.date.as_str()),
                                    Some(msg.envelope.message_id.as_str()),
                                    &raw,
                                )
                                .ok();
                                let raw_sha256 = Some(crate::cache::archive::sha256_hex(&raw));
                                let db_guard = state.cache_db.lock();
                                if let Some(conn) = db_guard.as_ref() {
                                    let _ = crate::cache::messages::update_body_with_raw(
                                        conn, task.account_id as i64, msg.uid as i64,
                                        &body_text, body_html.as_deref(),
                                        raw_path.as_deref().and_then(|p| p.to_str()),
                                        raw_sha256.as_deref(),
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Body-Fetch fehlgeschlagen für UID {} in '{}': {}",
                                    msg.uid, folder_name, e
                                );
                            }
                        }
                    }
                }

                // Notify frontend so the message list refreshes
                if !messages.is_empty() {
                    let _ = state.events.emit("new-messages", (task.account_id, folder_name, messages.len()));
                }

                // Web Push: notify installed PWAs even when the app is closed.
                // Only for INBOX — other folders are fetched on demand anyway.
                if !messages.is_empty() && is_inbox {
                    let account_id = task.account_id as i64;
                    let count = messages.len();
                    let sender = messages.first().map(|m| m.envelope.from.clone());
                    let body = match sender {
                        Some(s) if !s.is_empty() => format!("{} neue E-Mail(s) von {}", count, s),
                        _ => format!("{} neue E-Mail(s)", count),
                    };
                    let state_push = state.clone();
                    tokio::spawn(async move {
                        if let Err(e) =
                            crate::push::notify_account(&state_push, account_id, "Neue E-Mail", &body).await
                        {
                            tracing::warn!("WebPush fehlgeschlagen (account {}): {}", account_id, e);
                        }
                    });
                }

                // Enqueue AI summary only for messages that don't already have one
                if !is_spam {
                    for msg in &messages {
                        let needs_summary = {
                            let db_guard = state.cache_db.lock();
                            let conn = db_guard.as_ref().ok_or("Datenbank nicht initialisiert")?;
                            match crate::cache::messages::fetch_message_body(conn, task.account_id as i64, msg.uid as i64) {
                                Ok(Some(m)) => m.ai_summary.is_none(),
                                _ => true,
                            }
                        };

                        if needs_summary {
                            // Send to the DEDICATED AI worker channel — never
                            // the sync queue, so LLM work cannot stall IMAP sync.
                            let _ = ai_tx
                                .send(SyncTask {
                                    account_id: task.account_id,
                                    task_type: SyncTaskType::GenerateAiSummary(msg.uid),
                                    created_at: tokio::time::Instant::now(),
                                    retries: 0,
                                    max_retries: 2,
                                    priority: 5,
                                })
                                .await;
                        }
                    }
                }

                total_new += messages.len();
                if !messages.is_empty() {
                    tracing::info!(
                        "Account {}: {} neue Nachrichten in '{}' synchronisiert",
                        task.account_id,
                        messages.len(),
                        folder_name
                    );
                }

                // Persist per-folder sync state (last UID + modseq) for delta sync.
                {
                    let db_guard = state.cache_db.lock();
                    if let Some(conn) = db_guard.as_ref() {
                        let new_max = messages.iter().map(|m| m.uid as i64).max().unwrap_or(max_uid);
                        let _ = crate::cache::sync_state::set(
                            conn,
                            task.account_id as i64,
                            folder_name,
                            new_max.max(max_uid),
                            0,
                        );
                    }
                }
            }

            Ok(total_new)
        }
        SyncTaskType::RefreshFlags => {
            // Flag refresh is handled by run_flag_refresh in the main loop.
            // This task type exists for future use if we need to enqueue it.
            Ok(0)
        }
        SyncTaskType::BackfillEmails => {
            // Backfill: for every cached message without raw_path, fetch the
            // raw RFC822 bytes and write the EML archive file. Runs in small
            // batches so a single sync cycle stays bounded.
            let account_id = task.account_id;
            let client = {
                let guard = state.imap_clients.read();
                guard
                    .get(&account_id)
                    .cloned()
                    .ok_or("IMAP-Client nicht gefunden")?
            };
            if !client.is_connected().await {
                client.reconnect().await.map_err(|e| e.to_string())?;
            }

            let pending: Vec<(u32, String)> = {
                let db_guard = state.cache_db.lock();
                let conn = db_guard.as_ref().ok_or("Datenbank nicht initialisiert")?;
                let mut stmt = conn
                    .prepare(
                        "SELECT m.uid, f.name FROM messages m
                         JOIN folders f ON f.id = m.folder_id
                         WHERE m.account_id = ?1 AND (m.raw_path IS NULL OR m.raw_path = '')
                         ORDER BY m.date DESC LIMIT 20",
                    )
                    .map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map(rusqlite::params![account_id as i64], |row| {
                        Ok((row.get::<_, i64>(0)? as u32, row.get::<_, String>(1)?))
                    })
                    .map_err(|e| e.to_string())?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r.map_err(|e| e.to_string())?);
                }
                out
            };

            let mut backfilled = 0usize;
            for (uid, folder) in pending {
                match client.fetch_body_with_raw_from_folder(uid, Some(folder.clone())).await {
                    Ok((_text, _html, raw)) => {
                        let (msg_id, date) = {
                            let db_guard = state.cache_db.lock();
                            let conn = db_guard.as_ref().ok_or("Datenbank nicht initialisiert")?;
                            let mid: Option<String> = conn
                                .query_row(
                                    "SELECT message_id FROM messages WHERE account_id = ?1 AND uid = ?2",
                                    rusqlite::params![account_id as i64, uid as i64],
                                    |r| r.get(0),
                                )
                                .ok();
                            (mid, None::<String>)
                        };
                        let path = crate::cache::archive::write_eml(
                            &state.data_root,
                            account_id as i64,
                            uid,
                            date.as_deref(),
                            msg_id.as_deref(),
                            &raw,
                        );
                        let sha = crate::cache::archive::sha256_hex(&raw);
                        if let Ok(abs) = path {
                            let rel = abs
                                .strip_prefix(&state.data_root)
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_else(|_| abs.to_string_lossy().to_string());
                            let db_guard = state.cache_db.lock();
                            if let Some(conn) = db_guard.as_ref() {
                                let _ = conn.execute(
                                    "UPDATE messages SET raw_path = ?1, raw_sha256 = ?2 WHERE account_id = ?3 AND uid = ?4",
                                    rusqlite::params![rel, sha, account_id as i64, uid as i64],
                                );
                            }
                            backfilled += 1;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Backfill uid {} ({}): {}", uid, folder, e);
                    }
                }
            }

            if backfilled > 0 {
                tracing::info!("Backfill: {} EMLs für Konto {} nachgezogen", backfilled, account_id);
            }
            // If more messages remain, re-enqueue so subsequent cycles continue.
            let remaining: i64 = {
                let db_guard = state.cache_db.lock();
                let conn = db_guard.as_ref().ok_or("Datenbank nicht initialisiert")?;
                conn.query_row(
                    "SELECT COUNT(*) FROM messages WHERE account_id = ?1 AND (raw_path IS NULL OR raw_path = '')",
                    rusqlite::params![account_id as i64],
                    |r| r.get(0),
                )
                .unwrap_or(0)
            };
            if remaining > 0 {
                queue
                    .enqueue(SyncTask {
                        account_id,
                        task_type: SyncTaskType::BackfillEmails,
                        created_at: tokio::time::Instant::now(),
                        retries: 0,
                        max_retries: 2,
                        priority: 3,
                    })
                    .await;
            }
            Ok(backfilled)
        }
        SyncTaskType::GenerateAiSummary(uid) => {
            // Handled by the dedicated AI worker (run_ai_summary_worker).
            // Kept for compatibility with tasks already in the queue.
            process_ai_summary(state, task.account_id, *uid).await
        }
        SyncTaskType::AnalyzeDiff => {
            // Background: analyze queued diffs between AI draft and user's final text.
            // Processes up to 3 diffs per cycle to avoid blocking the LLM.
            let (diffs, ai_client_opt) = {
                let db_guard = state.cache_db.lock();
                let conn = db_guard
                    .as_ref()
                    .ok_or("Datenbank nicht initialisiert")?;
                let diffs = crate::cache::learning::get_unanalyzed_diffs(conn, 3)
                    .map_err(|e| e.to_string())?;
                let ai_client = {
                    let guard = state.ai_client.read();
                    guard.clone()
                };
                (diffs, ai_client)
            };

            for diff in diffs {
                if let Some(ref client) = ai_client_opt {
                    let (system, user) = crate::ai::prompts::build_diff_analysis_prompt(
                        &diff.ai_draft,
                        &diff.user_final,
                    );
                    match client
                        .complete_background(
                            &system,
                            &user,
                            Some(0.3),
                            Some(200),
                        )
                        .await
                    {
                        Some(Ok(hint)) => {
                            let db_guard = state.cache_db.lock();
                            if let Some(conn) = db_guard.as_ref() {
                                let _ = crate::cache::learning::mark_analyzed(conn, diff.id, &hint);
                                tracing::info!(
                                    "Diff {} analysiert: {}",
                                    diff.id, hint
                                );
                            }
                        }
                        Some(Err(e)) => {
                            tracing::warn!("Diff analysis LLM error for diff {}: {}", diff.id, e);
                        }
                        None => {
                            tracing::debug!("LLM busy, skip diff analysis for diff {}", diff.id);
                        }
                    }
                }
            }

            // Re-enqueue if there are more diffs to process
            let has_more = {
                let db_guard = state.cache_db.lock();
                if let Some(conn) = db_guard.as_ref() {
                    match crate::cache::learning::get_unanalyzed_diffs(conn, 1) {
                        Ok(d) => !d.is_empty(),
                        Err(e) => {
                            tracing::warn!("Failed to check remaining diffs: {}", e);
                            false
                        }
                    }
                } else {
                    false
                }
            };
            if has_more {
                queue.enqueue(SyncTask {
                    account_id: task.account_id,
                    task_type: SyncTaskType::AnalyzeDiff,
                    created_at: tokio::time::Instant::now(),
                    retries: 0,
                    max_retries: 0,
                    priority: 3,
                }).await;
            }

            Ok(0)
        }
        SyncTaskType::RefreshFingerprint => {
            // Background: refresh style fingerprints for recipients with >=3 new analyzed hints.
            // Processes up to 2 recipients per cycle to avoid LLM semaphore starvation.
            let (recipients, ai_client_opt) = {
                let db_guard = state.cache_db.lock();
                let conn = db_guard
                    .as_ref()
                    .ok_or("Datenbank nicht initialisiert")?;
                let recipients = crate::cache::fingerprint::get_recipients_needing_refresh(conn, 0, 2)
                    .map_err(|e| e.to_string())?;
                let ai_client = {
                    let guard = state.ai_client.read();
                    guard.clone()
                };
                (recipients, ai_client)
            };

            for email_hash in recipients {
                if let Some(ref client) = ai_client_opt {
                    let (hints, account_id) = {
                        let db_guard = state.cache_db.lock();
                        let conn = db_guard
                            .as_ref()
                            .ok_or("Datenbank nicht initialisiert")?;
                        let hints = crate::cache::fingerprint::get_hints_for_synthesis(conn, 0, &email_hash)
                            .map_err(|e| e.to_string())?;
                        if hints.is_empty() {
                            continue;
                        }
                        (hints, 0)
                    };

                    let (system, user) = crate::ai::prompts::build_fingerprint_synthesis_prompt(&hints);
                    match client
                        .complete_background(
                            &system,
                            &user,
                            Some(0.3),
                            Some(200),
                        )
                        .await
                    {
                        Some(Ok(fingerprint)) => {
                            let db_guard = state.cache_db.lock();
                            if let Some(conn) = db_guard.as_ref() {
                                let _ = crate::cache::fingerprint::save_fingerprint(
                                    conn,
                                    account_id,
                                    &email_hash,
                                    &fingerprint,
                                    hints.len() as i64,
                                );
                                tracing::info!(
                                    "Style fingerprint fuer {} aktualisiert ({} Hinweise)",
                                    email_hash,
                                    hints.len()
                                );
                            }
                        }
                        Some(Err(e)) => {
                            tracing::warn!("Fingerprint synthesis LLM error for {}: {}", email_hash, e);
                        }
                        None => {
                            tracing::debug!("LLM busy, skip fingerprint synthesis for {}", email_hash);
                        }
                    }
                }
            }

            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::Instant;

    // ── helpers ──────────────────────────────────────────────────────────

    /// Simulates the retry re-enqueue logic from `do_sync_cycle` lines 81-86.
    async fn simulate_retry(queue: &SyncQueue, task: &SyncTask) {
        if task.retries < task.max_retries {
            let mut retry_task = task.clone();
            retry_task.retries += 1;
            retry_task.priority = retry_task.priority.saturating_sub(1);
            queue.enqueue(retry_task).await;
        }
    }

    fn make_task(account_id: u32, retries: u32, max_retries: u32, priority: u8) -> SyncTask {
        SyncTask {
            account_id,
            task_type: SyncTaskType::FetchNew,
            created_at: Instant::now(),
            retries,
            max_retries,
            priority,
        }
    }

    // ── retry re-enqueue logic ───────────────────────────────────────────

    #[tokio::test]
    async fn test_retry_increments_retries_and_decrements_priority() {
        let queue = SyncQueue::new();
        let task = make_task(1, 0, 3, 10);
        queue.enqueue(task).await;
        let dequeued = queue.dequeue().await.unwrap();
        simulate_retry(&queue, &dequeued).await;
        let retried = queue.dequeue().await.unwrap();
        assert_eq!(retried.retries, 1, "retry count should increment");
        assert_eq!(retried.priority, 9, "priority should decrement by 1");
    }

    #[tokio::test]
    async fn test_retry_preserves_account_id_and_task_type() {
        let queue = SyncQueue::new();
        let task = make_task(99, 1, 3, 5);
        queue.enqueue(task).await;
        let dequeued = queue.dequeue().await.unwrap();
        simulate_retry(&queue, &dequeued).await;
        let retried = queue.dequeue().await.unwrap();
        assert_eq!(retried.account_id, 99);
        assert_eq!(retried.task_type, SyncTaskType::FetchNew);
    }

    #[tokio::test]
    async fn test_max_retries_not_re_enqueued() {
        let queue = SyncQueue::new();
        // retries == max_retries → should NOT be re-enqueued
        let task = make_task(1, 3, 3, 10);
        queue.enqueue(task).await;
        let dequeued = queue.dequeue().await.unwrap();
        simulate_retry(&queue, &dequeued).await;
        assert!(
            queue.dequeue().await.is_none(),
            "task at max_retries should not be re-enqueued"
        );
    }

    #[tokio::test]
    async fn test_retries_exceeding_max_not_re_enqueued() {
        let queue = SyncQueue::new();
        // retries > max_retries (edge case) → should NOT be re-enqueued
        let task = make_task(1, 5, 3, 10);
        queue.enqueue(task).await;
        let dequeued = queue.dequeue().await.unwrap();
        simulate_retry(&queue, &dequeued).await;
        assert!(queue.dequeue().await.is_none());
    }

    #[tokio::test]
    async fn test_priority_saturating_sub_at_zero() {
        let queue = SyncQueue::new();
        let task = make_task(1, 0, 3, 0);
        queue.enqueue(task).await;
        let dequeued = queue.dequeue().await.unwrap();
        simulate_retry(&queue, &dequeued).await;
        let retried = queue.dequeue().await.unwrap();
        assert_eq!(
            retried.priority, 0,
            "priority 0 saturating_sub(1) should stay 0"
        );
    }

    // ── wait_time calculation (exponential backoff on failures) ──────────

    #[test]
    fn test_wait_time_below_threshold_uses_base_interval() {
        let base = Duration::from_secs(20);
        for fail_count in 0..=4 {
            let wait = calculate_wait_time(fail_count, base);
            assert_eq!(
                wait, base,
                "fail_count={} should return base interval",
                fail_count
            );
        }
    }

    #[test]
    fn test_wait_time_exponential_backoff() {
        let base = Duration::from_secs(20);
        // fail_count=5 → 20 * 2^1 = 40s
        assert_eq!(calculate_wait_time(5, base), Duration::from_secs(40));
        // fail_count=6 → 20 * 2^2 = 80s
        assert_eq!(calculate_wait_time(6, base), Duration::from_secs(80));
        // fail_count=7 → 20 * 2^3 = 160s
        assert_eq!(calculate_wait_time(7, base), Duration::from_secs(160));
    }

    #[test]
    fn test_wait_time_capped_at_300_seconds() {
        let base = Duration::from_secs(20);
        // fail_count=8 → 20 * 2^4 = 320s → min(320, 300) = 300s
        assert_eq!(calculate_wait_time(8, base), Duration::from_secs(300));
        // fail_count=9 → 20 * 2^5 = 640s → min(640, 300) = 300s
        assert_eq!(calculate_wait_time(9, base), Duration::from_secs(300));
        // very large fail_count → still capped at 300s
        assert_eq!(calculate_wait_time(100, base), Duration::from_secs(300));
    }

    #[test]
    fn test_wait_time_different_base_intervals() {
        let short_base = Duration::from_secs(10);
        assert_eq!(calculate_wait_time(0, short_base), Duration::from_secs(10));
        assert_eq!(calculate_wait_time(5, short_base), Duration::from_secs(20));
        assert_eq!(calculate_wait_time(8, short_base), Duration::from_secs(160));

        let long_base = Duration::from_secs(60);
        assert_eq!(calculate_wait_time(0, long_base), Duration::from_secs(60));
        // 60 * 2^1 = 120
        assert_eq!(calculate_wait_time(5, long_base), Duration::from_secs(120));
        // 60 * 2^5 = 1920 → min(1920, 300) = 300
        assert_eq!(calculate_wait_time(9, long_base), Duration::from_secs(300));
    }

    // ── smart backoff (new-count-based) ───────────────────────────────────

    #[test]
    fn test_backoff_new_mail_resets_empty_counter() {
        let base = Duration::from_secs(20);
        let max_int = Duration::from_secs(300);
        let mut consecutive_empty: u32 = 4; // was backed off

        // new_count > 0 should reset to base and set consecutive_empty = 0
        let wait = calculate_backoff(1, base, max_int, &mut consecutive_empty);
        assert_eq!(wait, base);
        assert_eq!(consecutive_empty, 0);
    }

    #[test]
    fn test_backoff_steps_exponential() {
        let base = Duration::from_secs(20);
        let max_int = Duration::from_secs(300);

        // Empty cycle #1 → 20 * 2^1 = 40s
        let mut empty: u32 = 0;
        let w1 = calculate_backoff(0, base, max_int, &mut empty);
        assert_eq!(w1, Duration::from_secs(40));
        assert_eq!(empty, 1);

        // Empty cycle #2 → 20 * 2^2 = 80s
        let w2 = calculate_backoff(0, base, max_int, &mut empty);
        assert_eq!(w2, Duration::from_secs(80));
        assert_eq!(empty, 2);

        // Empty cycle #3 → 20 * 2^3 = 160s
        let w3 = calculate_backoff(0, base, max_int, &mut empty);
        assert_eq!(w3, Duration::from_secs(160));
        assert_eq!(empty, 3);

        // Empty cycle #4 → 20 * 2^4 = 320s → capped to 300s
        let w4 = calculate_backoff(0, base, max_int, &mut empty);
        assert_eq!(w4, Duration::from_secs(300));
        assert_eq!(empty, 4);

        // Empty cycle #5 → still min(20 * 2^4, 300) = 300s (clamped exponent)
        let w5 = calculate_backoff(0, base, max_int, &mut empty);
        assert_eq!(w5, Duration::from_secs(300));
        assert_eq!(empty, 5);
    }

    #[test]
    fn test_backoff_new_mail_in_middle_of_backoff() {
        let base = Duration::from_secs(20);
        let max_int = Duration::from_secs(300);
        let mut empty: u32 = 2; // previously 2 empty cycles → 80s wait

        // New mail arrives → reset
        let w = calculate_backoff(5, base, max_int, &mut empty);
        assert_eq!(w, base);
        assert_eq!(empty, 0);

        // Next empty cycle starts fresh at 40s
        let w2 = calculate_backoff(0, base, max_int, &mut empty);
        assert_eq!(w2, Duration::from_secs(40));
        assert_eq!(empty, 1);
    }

    #[test]
    fn test_backoff_different_base_intervals() {
        let max_int = Duration::from_secs(300);

        let short_base = Duration::from_secs(10);
        let mut empty: u32 = 0;
        assert_eq!(calculate_backoff(0, short_base, max_int, &mut empty), Duration::from_secs(20));
        assert_eq!(empty, 1);

        let long_base = Duration::from_secs(60);
        let mut empty2: u32 = 0;
        assert_eq!(calculate_backoff(0, long_base, max_int, &mut empty2), Duration::from_secs(120));
    }

    #[test]
    fn test_backoff_counter_saturation_does_not_panic() {
        let base = Duration::from_secs(20);
        let max_int = Duration::from_secs(300);
        let mut empty: u32 = u32::MAX;

        // saturating_add should prevent overflow; backoff stays capped
        let w = calculate_backoff(0, base, max_int, &mut empty);
        assert_eq!(w, Duration::from_secs(300));
        // counter saturates at u32::MAX (shouldn't overflow)
        assert_eq!(empty, u32::MAX);
    }

    // ── full retry cycle integration ─────────────────────────────────────

    #[tokio::test]
    async fn test_full_retry_cycle_until_max_retries() {
        let queue = SyncQueue::new();

        // Start with a fresh task
        let task = make_task(42, 0, 2, 10);
        queue.enqueue(task).await;

        // Attempt 1: dequeue, fail, re-enqueue
        let t1 = queue.dequeue().await.expect("first task");
        assert_eq!(t1.retries, 0);
        queue.record_failure().await;
        simulate_retry(&queue, &t1).await;

        // Attempt 2: dequeue retry, fail, re-enqueue
        let t2 = queue.dequeue().await.expect("first retry");
        assert_eq!(t2.retries, 1);
        assert_eq!(t2.priority, 9);
        queue.record_failure().await;
        simulate_retry(&queue, &t2).await;

        // Attempt 3: dequeue second retry, fail → max_retries reached, no re-enqueue
        let t3 = queue.dequeue().await.expect("second retry");
        assert_eq!(t3.retries, 2);
        assert_eq!(t3.priority, 8);
        queue.record_failure().await;
        simulate_retry(&queue, &t3).await;

        // Queue should be empty now
        assert!(
            queue.dequeue().await.is_none(),
            "no more tasks after max_retries exhausted"
        );

        // Failure count should reflect all 3 failures
        assert_eq!(queue.failure_count().await, 3);
    }

    // ── health check interval logic ──────────────────────────────────────

    #[test]
    fn test_health_check_interval_elapsed() {
        // Verify that a check older than HEALTH_CHECK_INTERVAL triggers a new check
        let interval = Duration::from_secs(60);
        let now = Instant::now();

        // Last check was 61s ago → interval elapsed
        let old_check = now - interval - Duration::from_secs(1);
        assert!(
            now.duration_since(old_check) >= interval,
            "check older than interval should be considered elapsed"
        );

        // Last check was 30s ago → interval NOT elapsed
        let recent_check = now - Duration::from_secs(30);
        assert!(
            now.duration_since(recent_check) < interval,
            "check within interval should NOT be considered elapsed"
        );

        // Last check was exactly at the boundary → NOT elapsed (strictly less)
        let boundary_check = now - interval;
        assert!(
            now.duration_since(boundary_check) < interval + Duration::from_nanos(1),
            "boundary check should not trigger until strictly past interval"
        );
    }

    #[test]
    fn test_health_check_first_run_triggers_immediately() {
        // When no prior check exists (HashMap miss), the fallback value
        // (now - interval - 1s) ensures the first check runs immediately.
        let interval = Duration::from_secs(60);
        let now = Instant::now();
        let fallback = now - interval - Duration::from_secs(1);
        assert!(
            now.duration_since(fallback) >= interval,
            "fallback should trigger an immediate health check"
        );
    }

    #[tokio::test]
    async fn test_retry_cycle_with_success_resets_failures() {
        let queue = SyncQueue::new();

        // Two failures
        let task = make_task(7, 0, 3, 10);
        queue.enqueue(task).await;
        let t1 = queue.dequeue().await.unwrap();
        queue.record_failure().await;
        simulate_retry(&queue, &t1).await;

        let t2 = queue.dequeue().await.unwrap();
        queue.record_failure().await;
        simulate_retry(&queue, &t2).await;

        assert_eq!(queue.failure_count().await, 2);

        // Third attempt succeeds
        let _t3 = queue.dequeue().await.unwrap();
        queue.record_success().await;
        assert_eq!(
            queue.failure_count().await,
            0,
            "success should reset consecutive failure count"
        );
    }
}
/// Generate the AI summary for one message (shared by the sync-queue
/// compatibility arm and the dedicated worker).
async fn process_ai_summary(state: &AppState, account_id: u32, uid: u32) -> Result<usize, String> {
    let (body_text, client_opt) = {
        let db_guard = state.cache_db.lock();
        let conn = db_guard
            .as_ref()
            .ok_or("Datenbank nicht initialisiert")?;

        let msg = crate::cache::messages::fetch_message_body(
            conn,
            account_id as i64,
            uid as i64,
        )
        .map_err(|e| e.to_string())?;

        let body = msg.and_then(|m| m.body_text);

        let ai_client = {
            let guard = state.ai_client.read();
            guard.clone()
        };

        (body, ai_client)
    };

    if let (Some(body), Some(client)) = (body_text, client_opt) {
        let summary = match client
            .complete_background(
                crate::ai::prompts::AI_SUMMARY_PROMPT,
                &pii::mask_pii(&body),
                Some(0.3),
                Some(300),
            )
            .await
        {
            Some(Ok(s)) => Some(s),
            Some(Err(e)) => {
                tracing::warn!("AI Summary LLM error for uid {}: {}", uid, e);
                None
            }
            None => {
                tracing::debug!("LLM busy, skip summary for uid {}", uid);
                None
            }
        };

        if let Some(summary) = summary {
            let db_guard = state.cache_db.lock();
            let conn = db_guard
                .as_ref()
                .ok_or("Datenbank nicht initialisiert")?;
            let urgency = if summary.contains("KRITISCH") { Some(0.95f32) }
                else if summary.contains("ZEITKRITISCH") { Some(0.8f32) }
                else { Some(0.3f32) };
            let summary_text = summary.lines()
                .find(|l| l.starts_with("Zusammenfassung:"))
                .map(|l| l.trim_start_matches("Zusammenfassung:").trim())
                .unwrap_or(&summary)
                .to_string();
            crate::cache::messages::update_ai_summary(
                conn,
                account_id as i64,
                uid as i64,
                &summary_text,
            )
            .map_err(|e| e.to_string())?;
                let _ = state.events.emit("ai-summary-updated", (uid, account_id, summary_text.clone(), urgency));
            if let Some(u) = urgency {
                let _ = crate::cache::messages::update_ai_priority(
                    conn, account_id as i64, uid as i64, u,
                );
            }
        } else {
            // Fallback: rule-based priority detection when LLM is unavailable
            let rule_priority = priority::detect_priority("", &body);
            if rule_priority > 0.0 {
                let db_guard = state.cache_db.lock();
                if let Some(conn) = db_guard.as_ref() {
                    let _ = crate::cache::messages::update_ai_priority(
                        conn, account_id as i64, uid as i64, rule_priority,
                    );
                }
                let _ = state.events.emit("ai-summary-updated", (uid, account_id, String::new(), Some(rule_priority)));
            }
        }
    }
    Ok(0)
}

/// Dedicated AI-summary worker. Processes LLM summarization jobs sequentially
/// (never in the sync cycle), so slow LLM calls cannot delay IMAP sync.
async fn run_ai_summary_worker(state: Arc<AppState>, mut rx: mpsc::Receiver<SyncTask>) {
    tracing::info!("AI-Summary-Worker gestartet");
    while let Some(task) = rx.recv().await {
        match &task.task_type {
            SyncTaskType::GenerateAiSummary(uid) => {
                if let Err(e) = process_ai_summary(&state, task.account_id, *uid).await {
                    tracing::warn!(
                        "AI-Summary-Worker: uid {} (account {}) fehlgeschlagen: {}",
                        uid, task.account_id, e
                    );
                }
                // Slight pacing so the LLM is not hammered by bulk imports.
                sleep(Duration::from_millis(300)).await;
            }
            other => {
                tracing::debug!("AI-Summary-Worker: unerwarteter Task {:?}", other);
            }
        }
    }
}
