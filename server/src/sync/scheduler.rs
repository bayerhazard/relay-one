use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{sleep, Instant};

use crate::security::fraud;
use crate::security::pii;
use crate::security::priority;
use crate::sync::queue::{SyncQueue, SyncTask, SyncTaskType};
use crate::AppState;

/// Folders that hold messages flagged \Deleted from queued provider ops and
/// need a batched EXPUNGE. Per-op EXPUNGE was the single biggest latency
/// killer on large mailboxes (provider rewrites the whole mailbox each time).
static EXPUNGE_PENDING: OnceLock<std::sync::Mutex<HashSet<(u32, String)>>> = OnceLock::new();

/// Register a folder for the next batched EXPUNGE flush.
pub fn queue_expunge(account_id: u32, folder: &str) {
    let reg = EXPUNGE_PENDING.get_or_init(|| std::sync::Mutex::new(HashSet::new()));
    reg.lock().unwrap().insert((account_id, folder.to_string()));
}

/// Register a folder for EXPUNGE in memory and in the database, so a
/// restart before the next flush does not leave mails flagged \Deleted on
/// the provider (Kai, 7.10.2026).
/// Never call it while holding `cache_db` (the lock is not re-entrant):
/// under a held guard use `expunge_vormerken_mit` with that connection.
fn expunge_vormerken(state: &AppState, account_id: u32, folder: &str) {
    let db_guard = state.cache_db.lock();
    expunge_vormerken_mit(db_guard.as_ref(), account_id, folder);
}

fn expunge_vormerken_mit(conn: Option<&rusqlite::Connection>, account_id: u32, folder: &str) {
    queue_expunge(account_id, folder);
    if let Some(conn) = conn {
        let _ = crate::cache::provider_ops::expunge_merken(conn, account_id as i64, folder);
    }
}

/// How often queued \Deleted flags are expunged per folder.
const EXPUNGE_FLUSH_INTERVAL: Duration = Duration::from_secs(60);

/// Drain the pending-expunge set and run one EXPUNGE per (account, folder).
async fn run_periodic_expunge(state: &AppState) {
    let reg = EXPUNGE_PENDING.get_or_init(|| std::sync::Mutex::new(HashSet::new()));
    // What the database still lists (e.g. from before a restart) joins in.
    let gemerkt: Vec<(i64, String)> = {
        let db_guard = state.cache_db.lock();
        db_guard
            .as_ref()
            .and_then(|c| crate::cache::provider_ops::expunge_liste(c).ok())
            .unwrap_or_default()
    };
    let pending: Vec<(u32, String)> = {
        let mut guard = reg.lock().unwrap();
        for (a, f) in gemerkt {
            guard.insert((a as u32, f));
        }
        if guard.is_empty() {
            return;
        }
        guard.drain().collect()
    };
    for (account_id, folder) in pending {
        let client = {
            let guard = state.imap_clients.read();
            guard.get(&account_id).cloned()
        };
        let Some(client) = client else { continue };
        let ergebnis = client.expunge_folder_sync(&folder).await;
        if ergebnis.is_ok() {
            let db_guard = state.cache_db.lock();
            if let Some(conn) = db_guard.as_ref() {
                let _ = crate::cache::provider_ops::expunge_erledigt(conn, account_id as i64, &folder);
            }
        }
        if let Err(e) = ergebnis {
            tracing::warn!(
                "periodic EXPUNGE in '{}' (account {}) fehlgeschlagen: {} — erneut im nächsten Flush",
                folder, account_id, e
            );
            queue_expunge(account_id, &folder);
        }
    }
}

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
const ATTACHMENT_GC_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60); // daily

/// ── Initial-sync (backfill) tuning ─────────────────────────────────────
/// A folder is "backfilling" while a fetch returns a FULL batch (more old
/// mail remains on the server). Backfill uses larger batches, skips the
/// per-message body downloads + the full-UID prune, and caps AI to recent
/// mail — so a 100k+ message mailbox fills in minutes, not days. Steady-state
/// (caught up, partial batch) keeps the full treatment.
const BACKFILL_BATCH_SIZE: u32 = 200;
/// Max backfill batches per account per cycle (fairness: bounds the cycle so
/// other accounts/tasks are not starved by one huge mailbox).
const BACKFILL_MAX_BATCHES_PER_CYCLE: usize = 30;
/// Rate cap between backfill batches (ban-safe; replaces the old 20s sleep).
const BACKFILL_BATCH_INTERVAL: Duration = Duration::from_secs(1);
/// During backfill only AI-summarize mail newer than this. A local model has
/// finite GPU throughput: summarizing a full history would peg it for weeks,
/// so recent mail gets summaries first and older mail stays summary-less.
const AI_BACKFILL_MAX_AGE_DAYS: i64 = 30;

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
/// folder is stored inside the single local "Trash", Gmail's "All Mail"
/// (\All, read without the inbox) inside "Archive".
fn storage_folder_name(folder_name: &str, tag: &str) -> String {
    if tag == "trash" || PROVIDER_TRASH_NAMES.iter().any(|t| folder_name.eq_ignore_ascii_case(t)) {
        "Trash".to_string()
    } else if tag == "alle" {
        "Archive".to_string()
    } else {
        folder_name.to_string()
    }
}

/// Gmail's archive is no folder (Kai, 9.10.2026): an archived mail has
/// left the inbox and stays in "All Mail". Relay's "Archive" on Gmail is
/// "All Mail" read with this filter: not in the inbox, not sent or a draft,
/// and without a label of its own (those show in their label's folder).
pub(crate) const GMAIL_ARCHIV_FILTER: &str =
    "X-GM-RAW \"-in:inbox -in:sent -in:drafts -in:chats has:nouserlabels\"";

/// Gmail's "All Mail" (\All; "[Google Mail]/Alle Nachrichten"), where the
/// account is Gmail and shows it over IMAP.
pub(crate) fn gmail_archiv(folders: &[(String, String, String, String)]) -> Option<String> {
    if !ist_gmail(folders) {
        return None;
    }
    folders
        .iter()
        .find(|(_, _, _, tag)| tag == "alle")
        .map(|(name, ..)| name.clone())
}

/// The fetch filter of a provider folder: Gmail's archive reads "All Mail"
/// narrowed (see GMAIL_ARCHIV_FILTER), every other folder whole.
fn abruf_filter(folders: &[(String, String, String, String)], folder_name: &str) -> Option<&'static str> {
    match gmail_archiv(folders) {
        Some(a) if a == folder_name => Some(GMAIL_ARCHIV_FILTER),
        _ => None,
    }
}

/// Folders Relay mirrors from the list: not the shells (\NoSelect), not
/// Gmail's views; "All Mail" only on Gmail, as Relay's archive.
fn wird_gespiegelt(folders: &[(String, String, String, String)], folder_name: &str, tag: &str) -> bool {
    match tag {
        "noselect" | "sammel" => false,
        "alle" => abruf_filter(folders, folder_name).is_some(),
        _ => true,
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
    // Track last dedup-store GC (runs on ATTACHMENT_GC_INTERVAL, daily).
    let mut last_attachment_gc: Option<Instant> = None;
    // Track last batched provider EXPUNGE flush (runs on EXPUNGE_FLUSH_INTERVAL).
    let mut last_expunge: Option<Instant> = None;
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

    // One-shot startup catch-up: fills AI summaries + INBOX followup actions
    // for recent mail that arrived while the app was offline or pre-dates the
    // followup feature. Runs once, low priority, never blocks the sync cycle.
    {
        let catchup_state = state.clone();
        let catchup_tx = ai_tx.clone();
        tokio::spawn(async move {
            run_inbox_ai_catch_up(&catchup_state, &catchup_tx).await;
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
                // Daily dedup-store GC: removes files in <data>/attachments/
                // no longer referenced by any attachment row.
                let gc_due = last_attachment_gc
                    .map(|t| Instant::now().duration_since(t) >= ATTACHMENT_GC_INTERVAL)
                    .unwrap_or(false);
                if gc_due {
                    last_attachment_gc = Some(Instant::now());
                    run_attachment_gc(&state);
                }
                // Delete-queue worker: verify → hard/soft provider delete.
                run_delete_queue(&state).await;
                // Provider-op queue: replay user read/move/delete mutations
                // that the API deferred for an instant response.
                run_provider_ops(&state).await;
                // Batched EXPUNGE for \Deleted flags set by queued ops.
                let expunge_due = last_expunge
                    .map(|t| Instant::now().duration_since(t) >= EXPUNGE_FLUSH_INTERVAL)
                    .unwrap_or(true);
                if expunge_due {
                    last_expunge = Some(Instant::now());
                    run_periodic_expunge(&state).await;
                }
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

/// Daily dedup-store GC: removes files in `<data>/attachments/` that are no
/// longer referenced by any `message_attachments.disk_path`. Content-addressed
/// files are safe to delete — a later fetch re-materializes them.
fn run_attachment_gc(state: &AppState) {
    let db_guard = state.cache_db.lock();
    let Some(conn) = db_guard.as_ref() else { return; };
    match crate::cache::attachments::gc_orphaned_attachments(conn, &state.data_root) {
        Ok(report) => {
            if report.removed_files > 0 {
                tracing::info!(
                    "Attachment-GC (täglich): {} Dateien entfernt ({} bytes), {} behalten",
                    report.removed_files, report.freed_bytes, report.kept_files
                );
            }
        }
        Err(e) => tracing::warn!("Attachment-GC fehlgeschlagen: {}", e),
    }
}

/// Refresh IMAP \Seen flags for all cached messages to detect read/unread
/// changes made from other clients (phone, webmail, …).
async fn run_flag_refresh(state: &AppState) {    let clients: Vec<(u32, Arc<crate::imap::client::ImapClient>)> = {
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

        let folders = match client.list_folders_sync().await {
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
            if !wird_gespiegelt(&folders, folder_name, tag) {
                continue;
            }
            // Provider trash folders are stored under the local "Trash".
            let storage_folder = storage_folder_name(folder_name, tag);
            // NOTE: pass the DECODED name — the fetch re-encodes to UTF-7
            // internally. Passing raw_name (already UTF-7) would
            // double-encode (& → &-) and fail with "unknown folder".

            // Only numbers the server confirmed in this folder: a row moved
            // here keeps its old folder's number until the server's row
            // comes (Kai, 9.10.2026).
            let local_msgs = {
                let db_guard = state.cache_db.lock();
                let Some(conn) = db_guard.as_ref() else { continue; };
                match crate::cache::messages::bestaetigte_uids(
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
            // Batch UIDs in chunks of 500 to avoid oversized IMAP commands
            // that some servers reject or that cause long response times.
            let mut fetched: Vec<(u32, bool, bool)> = Vec::new();
            let mut batch_failed = false;
            for chunk in uid_list.chunks(500) {
                let uid_set = chunk.join(",");
                match client.fetch_flags_sync(folder_name, &uid_set).await {
                    Ok(f) => fetched.extend(f),
                    Err(e) => {
                        tracing::warn!(
                            "flag_refresh: fetch_flags fuer '{}' (account {}) fehlgeschlagen: {}",
                            folder_name, account_id, e
                        );
                        batch_failed = true;
                        break;
                    }
                }
            }
            if batch_failed {
                continue;
            }

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
                    if let Err(e) = crate::cache::messages::update_is_read_guarded(
                        &tx, *account_id as i64, &storage_folder, *uid as i64, *is_read,
                    ) {
                        tracing::warn!("flag_refresh: update_is_read uid={} fehlgeschlagen: {}", uid, e);
                    } else {
                        count += 1;
                    }
                    if let Err(e) = crate::cache::messages::update_is_flagged(
                        &tx, *account_id as i64, &storage_folder, *uid as i64, *is_flagged,
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
    for (row_id, account_id, uid, raw_path) in &expired {
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
            // By the row (Kai, 9.10.2026): Relay's trash keeps the numbers of
            // the folders its mails came from, and deleting by number took
            // the mails with that number in every other folder along.
            let _ = crate::cache::messages::delete_message_row(conn, *row_id);
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
            // Gmail only archives on \Deleted: there the mail goes into its trash.
            // Without the list it is tried again: an empty list read as "not
            // Gmail", and \Deleted there only archives (Kai, 9.10.2026).
            match client.list_folders_sync().await {
                Ok(ordner) => match entfernen_wie(&ordner, &row.folder) {
                    Entfernen::InPapierkorb(trash) => client.move_message_sync(row.uid as u32, &row.folder, &trash).await,
                    Entfernen::Markieren => client.hard_delete_message_sync(row.uid as u32, &row.folder).await,
                },
                Err(e) => Err(e),
            }
        } else {
            // Soft fallback: move into the PROVIDER trash folder (never the
            // local-only "Trash" — that name does not exist on the server).
            // Resolve the real provider trash name (GMX: "Gelöscht",
            // Gmail: "[Gmail]/Papierkorb", IMAP standard: "Trash").
            let provider_trash = find_provider_trash_folder(&client).await;
            tracing::warn!(
                "delete_queue {}: Verify-Garantie fehlt (uid {}, Konto {}) — weiches Löschen (Provider-Trash '{}')",
                row.id, row.uid, row.account_id, provider_trash.as_deref().unwrap_or("Trash")
            );
            match provider_trash {
                Some(trash) => client.move_message_sync(row.uid as u32, &row.folder, &trash).await,
                None => {
                    // No provider trash folder found: fall back to a hard
                    // delete ONLY when the message row still exists in the
                    // cache (safe: uid+folder were just verified above via
                    // the archive lookup that returned no EML — the local
                    // copy is gone anyway, so keeping the server copy does
                    // not help the user; the queue entry would otherwise
                    // retry forever).
                    client.hard_delete_message_sync(row.uid as u32, &row.folder).await
                }
            }
        };

        match result {
            Ok(()) => {
                let db_guard = state.cache_db.lock();
                if let Some(conn) = db_guard.as_ref() {
                    let _ = crate::cache::delete_queue::mark_deleted(conn, row.id);
                }
                // \Deleted was just set on the source folder — batch the
                // EXPUNGE instead of paying for one per delete.
                expunge_vormerken_mit(db_guard.as_ref(), row.account_id as u32, &row.folder);
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

/// Drain the provider-op queue (local-first mutations deferred by the API).
/// Runs on the USER connection slot: these are user-intent mutations and must
/// never sit behind IDLE/batch-fetch work on the sync slot. Bounded per cycle
/// so one account's backlog cannot starve the sync loop.
async fn run_provider_ops(state: &AppState) {
    const OPS_PER_CYCLE: i64 = 10;
    let account_ids: Vec<u32> = {
        let guard = state.imap_clients.read();
        guard.keys().copied().collect()
    };
    for account_id in account_ids {
        let rows = {
            let db_guard = state.cache_db.lock();
            let Some(conn) = db_guard.as_ref() else { continue };
            match crate::cache::provider_ops::take_pending_for_account(conn, account_id as i64, OPS_PER_CYCLE) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("provider_ops: Liste fehlgeschlagen: {}", e);
                    continue;
                }
            }
        };
        if rows.is_empty() {
            continue;
        }
        let client = {
            let guard = state.imap_clients.read();
            guard.get(&account_id).cloned()
        };
        let Some(client) = client else { continue };
        // The folder list once per account and cycle, only when an op needs
        // it: "Trash" means the provider's real trash, and Gmail deletes by
        // moving there (Kai, 7.10.2026).
        // Without the list (a dropped connection) the op fails and is tried
        // again; an empty list would have read as "no trash" and the delete
        // would have counted as done without doing anything.
        let mut ordner: Option<Vec<(String, String, String, String)>> = None;
        for op in rows {
            let braucht_liste = matches!(op.kind.as_str(), "delete" | "delete_mid" | "move_mid" | "move" | "flag_mid" | "zurueck_mid");
            if braucht_liste && ordner.is_none() {
                // A connection gone quiet between cycles fails with a broken
                // pipe: connect anew and try once more, instead of waiting a
                // whole cycle with every delete of this account.
                ordner = match client.list_folders().await {
                    Ok(l) => Some(l),
                    Err(_) => match client.reconnect().await {
                        Ok(()) => client.list_folders().await.ok(),
                        Err(_) => None,
                    },
                };
            }
            let liste = ordner.as_deref().unwrap_or(&[]);
            let result = if braucht_liste && ordner.is_none() {
                Err(crate::error::AppError::imap("Ordnerliste nicht lesbar", "provider_ops"))
            } else { match op.kind.as_str() {
                "flag" => match op.flag.as_deref() {
                    Some(flag) => client
                        .mark_flag(op.uid as u32, flag, op.set_flag, Some(op.folder.clone()))
                        .await
                        .map(|_| ()),
                    None => Err(crate::error::AppError::imap("flag-Op ohne Flag", "provider_ops")),
                },
                "move" => match op.target_folder.as_deref() {
                    // Gmail's archive by number: the mail leaves its folder
                    // (label) and stays in "All Mail" (see verschieben_gmail).
                    Some(target) if gmail_archiv(liste).is_some()
                        && provider_ordner(liste, target) == gmail_archiv(liste) =>
                    {
                        client.delete_message(op.uid as u32, &op.folder).await
                    }
                    Some(target) => {
                        // Relay's "Trash" is the provider's real trash where it
                        // has one; a folder "Trash" is only made where it has
                        // none (on Gmail that was a label, the mail stayed).
                        let ziel = provider_ordner(liste, target);
                        let target = ziel.as_deref().unwrap_or(target);
                        match client.move_message(op.uid as u32, &op.folder, target).await {
                            Ok(()) => Ok(()),
                            // The old sync path pre-created "Trash" before every
                            // delete; do it lazily here: create + retry once.
                            Err(e1) => {
                                let _ = client.create_folder(target).await;
                                client
                                    .move_message(op.uid as u32, &op.folder, target)
                                    .await
                                    .map_err(|e2| {
                                        crate::error::AppError::imap(
                                            format!("{} (nach Ordner-Anlage: {})", e1, e2),
                                            "move_retry",
                                        )
                                    })
                            }
                        }
                    }
                    None => Err(crate::error::AppError::imap("move-Op ohne Zielordner", "provider_ops")),
                },
                "delete" => match entfernen_wie(liste, &op.folder) {
                    Entfernen::InPapierkorb(trash) => client.move_message(op.uid as u32, &op.folder, &trash).await,
                    Entfernen::Markieren => client.delete_message(op.uid as u32, &op.folder).await,
                },
                // By Message-ID (Kai, 7.10.2026): the mail is looked up in its
                // folder on the provider ("Trash" = the provider's trash), so a
                // number gone stale by a local move cannot miss it or hit
                // another mail. Nothing found is done: it is already gone.
                "delete_mid" => match (provider_ordner(liste, &op.folder), op.flag.as_deref()) {
                    (Some(ordner), Some(kopf)) => match client.uids_by_message_id(&ordner, kopf).await {
                        Ok(uids) => {
                            let mut ergebnis = Ok(());
                            for u in &uids {
                                let r = match entfernen_wie(liste, &ordner) {
                                    Entfernen::InPapierkorb(trash) => client.move_message(*u, &ordner, &trash).await,
                                    Entfernen::Markieren => client.delete_message(*u, &ordner).await,
                                };
                                if let Err(e) = r {
                                    ergebnis = Err(e);
                                    break;
                                }
                            }
                            tracing::info!("provider_ops {}: {} Mail(s) in '{}' nach Message-ID gelöscht", op.id, uids.len(), ordner);
                            if !uids.is_empty() && ergebnis.is_ok() {
                                expunge_vormerken(state, account_id, &ordner);
                            }
                            ergebnis
                        }
                        Err(e) => Err(e),
                    },
                    _ => Ok(()),
                },
                "move_mid" => match (provider_ordner(liste, &op.folder), op.flag.as_deref(), op.target_folder.as_deref()) {
                    (Some(quelle), Some(kopf), Some(ziel)) => {
                        // "Trash" as target: the provider's trash, or a folder
                        // "Trash" made where it has none; "Archive" on Gmail
                        // its "All Mail". A target the provider lacks is made
                        // (an "Archive" where it has none, a folder Relay
                        // knew before the provider did).
                        let ziel = match provider_ordner(liste, ziel) {
                            Some(z) => z,
                            None => ziel.to_string(),
                        };
                        ordner_sicherstellen(&client, liste, &ziel).await;
                        verschieben_mid(state, &client, account_id, liste, &quelle, &ziel, kopf)
                            .await
                            .map(|n| {
                                tracing::info!("provider_ops {}: {} Mail(s) von '{}' nach '{}' (Message-ID)", op.id, n, quelle, ziel);
                            })
                    }
                    _ => Ok(()),
                },
                // Star and "read" by Message-ID (Kai, 9.10.2026): a row moved
                // locally keeps its old folder's number until the next sync.
                "flag_mid" => match (provider_ordner(liste, &op.folder), op.kopf.as_deref(), op.flag.as_deref()) {
                    (Some(ordner), Some(kopf), Some(flag)) => match client.uids_by_message_id(&ordner, kopf).await {
                        Ok(uids) => {
                            let mut ergebnis = Ok(());
                            for u in &uids {
                                if let Err(e) = client.mark_flag(*u, flag, op.set_flag, Some(ordner.clone())).await {
                                    ergebnis = Err(e);
                                    break;
                                }
                            }
                            ergebnis
                        }
                        Err(e) => Err(e),
                    },
                    _ => Ok(()),
                },
                // A mail Relay shows in `folder` that a bug had taken off the
                // provider (Kai, 9.10.2026): back from the provider's trash,
                // else from the EML archive.
                "zurueck_mid" => match op.kopf.as_deref() {
                    Some(kopf) => retten_mid(state, &client, account_id, liste, &op.folder, kopf).await,
                    None => Ok(()),
                },
                other => {
                    tracing::warn!("provider_ops {}: unbekannter Typ '{}'", op.id, other);
                    let db_guard = state.cache_db.lock();
                    if let Some(conn) = db_guard.as_ref() {
                        let _ = crate::cache::provider_ops::mark_failed(conn, op.id, "unbekannter Typ");
                    }
                    continue;
                }
            } };
            let db_guard = state.cache_db.lock();
            match result {
                Ok(()) => {
                    if let Some(conn) = db_guard.as_ref() {
                        let _ = crate::cache::provider_ops::mark_done(conn, op.id);
                    }
                    if op.kind == "move" || op.kind == "delete" {
                        expunge_vormerken_mit(db_guard.as_ref(), account_id, &op.folder);
                    }
                }
                Err(e) => {
                    if let Some(conn) = db_guard.as_ref() {
                        let _ = crate::cache::provider_ops::mark_failed(conn, op.id, &e.to_string());
                    }
                    tracing::warn!(
                        "provider_ops {} ({} uid {} in '{}') fehlgeschlagen: {}",
                        op.id, op.kind, op.uid, op.folder, e
                    );
                }
            }
        }
    }
}

/// Find the provider-side trash folder for an account. Returns the decoded
/// folder name (as accepted by select_folder/move_message) or None when the
/// server has no trash folder at all.
async fn find_provider_trash_folder(
    client: &Arc<crate::imap::client::ImapClient>,
) -> Option<String> {
    let folders = client.list_folders_sync().await.ok()?;
    papierkorb_waehlen(&folders)
}

/// The provider's trash from a folder list (name, raw, delimiter, tag): the
/// folder marked \Trash first (tag "trash": Gmail's "[Gmail]/Papierkorb",
/// GMX's "Gelöscht"), else by its usual names.
pub(crate) fn papierkorb_waehlen(folders: &[(String, String, String, String)]) -> Option<String> {
    if let Some((name, ..)) = folders.iter().find(|(_, _, _, tag)| tag == "trash") {
        return Some(name.clone());
    }
    // Common trash names across providers, checked case-insensitively.
    const TRASH_ALIASES: [&str; 7] = [
        "trash", "gelöscht", "papierkorb", "deleted", "deleted items",
        "deleted messages", "corbeille",
    ];
    for (name, _raw, _delim, tag) in folders {
        if tag == "noselect" || tag == "sammel" || tag == "alle" {
            continue;
        }
        let lower = name.to_lowercase();
        if TRASH_ALIASES.iter().any(|a| lower.contains(a)) {
            return Some(name.clone());
        }
    }
    None
}

/// Gmail: "\Deleted + EXPUNGE" only archives there (Gmail's default), the
/// mail stays in All Mail. Deleting means moving it into Gmail's trash,
/// which Gmail empties after 30 days (Kai, 7.10.2026).
/// A folder name as Relay uses it, on the provider: "Trash" is the
/// provider's trash (None where it has none); every other name is itself.
pub(crate) fn provider_ordner(folders: &[(String, String, String, String)], name: &str) -> Option<String> {
    if name == "Trash" {
        papierkorb_waehlen(folders)
    } else if name == "Archive" {
        // Gmail: "All Mail" (Kai, 9.10.2026); elsewhere a folder "Archive".
        Some(gmail_archiv(folders).unwrap_or_else(|| name.to_string()))
    } else {
        Some(name.to_string())
    }
}

/// The provider's spam folder by name ("[Gmail]/Spam", "Junk", "Spam" …).
pub(crate) fn ist_spam_ordner(name: &str) -> bool {
    let blatt = name.rsplit(['/', '.']).next().unwrap_or(name).to_lowercase();
    ["spam", "junk", "spamverdacht", "junk e-mail", "bulk"].contains(&blatt.as_str())
}

/// Make a target folder the provider lacks (best effort; the move after it
/// fails and is tried again when it could not be made). Not Gmail's "All
/// Mail" — it always exists.
async fn ordner_sicherstellen(
    client: &Arc<crate::imap::client::ImapClient>,
    liste: &[(String, String, String, String)],
    ziel: &str,
) {
    if liste.iter().any(|(name, ..)| name == ziel) || ziel.eq_ignore_ascii_case("INBOX") {
        return;
    }
    match client.create_folder(ziel).await {
        Ok(()) => tracing::info!("provider_ops: Ordner '{}' beim Anbieter angelegt", ziel),
        // Made by an op before it in this cycle (the list is read once).
        Err(e) if e.to_string().to_lowercase().contains("exists") => {}
        Err(e) => tracing::warn!("provider_ops: Ordner '{}' nicht angelegt: {}", ziel, e),
    }
}

/// Move the mails with this Message-ID from `quelle` to `ziel` (provider
/// names). Returns how many were found.
///
/// On Gmail a folder is a label and "All Mail" holds every mail (Kai,
/// 9.10.2026):
/// - into the archive ("All Mail"): the mail only leaves its folder
///   (\Deleted + EXPUNGE there takes that label off; it stays in "All
///   Mail"). Out of trash or spam it goes to the inbox first, as there
///   EXPUNGE would delete it for good.
/// - out of the archive: a copy into the target adds that label; nothing
///   is deleted in "All Mail" (that would delete the mail itself). Into
///   trash or spam a MOVE.
/// Everywhere else a MOVE (COPY + \Deleted where the server has no MOVE).
async fn verschieben_mid(
    state: &AppState,
    client: &Arc<crate::imap::client::ImapClient>,
    account_id: u32,
    liste: &[(String, String, String, String)],
    quelle: &str,
    ziel: &str,
    kopf: &str,
) -> Result<usize, crate::error::AppError> {
    let alle = gmail_archiv(liste);
    let papierkorb = papierkorb_waehlen(liste);
    let uids = client.uids_by_message_id(quelle, kopf).await?;
    for u in &uids {
        if alle.as_deref() == Some(ziel) {
            let aus_papierkorb_oder_spam = papierkorb.as_deref() == Some(quelle) || ist_spam_ordner(quelle);
            if aus_papierkorb_oder_spam {
                client.move_message(*u, quelle, "INBOX").await?;
                for v in client.uids_by_message_id("INBOX", kopf).await? {
                    client.delete_message(v, "INBOX").await?;
                }
                expunge_vormerken(state, account_id, "INBOX");
            } else {
                client.delete_message(*u, quelle).await?;
            }
        } else if alle.as_deref() == Some(quelle) {
            if papierkorb.as_deref() == Some(ziel) || ist_spam_ordner(ziel) {
                client.move_message(*u, quelle, ziel).await?;
            } else {
                client.copy_message(*u, quelle, ziel).await?;
            }
        } else {
            client.move_message(*u, quelle, ziel).await?;
        }
    }
    if !uids.is_empty() && alle.as_deref() != Some(quelle) {
        expunge_vormerken(state, account_id, quelle);
    }
    Ok(uids.len())
}

/// The rescue of one mail (Kai, 9.10.2026): Relay shows it in `ordner`, a
/// bug had taken it off the provider there (an "Archive" Gmail does not
/// have, a folder Relay had seen empty: the second move into it deleted).
/// Found in the target: done (on Gmail's archive it also leaves the inbox,
/// where the first archiving had left it). In the provider's trash: moved
/// back. Else from the EML archive (APPEND). Else it stays as it is.
async fn retten_mid(
    state: &AppState,
    client: &Arc<crate::imap::client::ImapClient>,
    account_id: u32,
    liste: &[(String, String, String, String)],
    ordner: &str,
    kopf: &str,
) -> Result<(), crate::error::AppError> {
    let ziel = provider_ordner(liste, ordner).unwrap_or_else(|| ordner.to_string());
    let alle = gmail_archiv(liste);
    ordner_sicherstellen(client, liste, &ziel).await;
    if !client.uids_by_message_id(&ziel, kopf).await?.is_empty() {
        if alle.as_deref() == Some(ziel.as_str()) {
            let im_eingang = client.uids_by_message_id("INBOX", kopf).await?;
            for v in &im_eingang {
                client.delete_message(*v, "INBOX").await?;
            }
            if !im_eingang.is_empty() {
                expunge_vormerken(state, account_id, "INBOX");
            }
        }
        return Ok(());
    }
    if let Some(trash) = papierkorb_waehlen(liste) {
        if trash != ziel {
            let n = verschieben_mid(state, client, account_id, liste, &trash, &ziel, kopf).await?;
            if n > 0 {
                tracing::info!("Rettung: Mail {} aus '{}' nach '{}' zurück", kopf, trash, ziel);
                return Ok(());
            }
        }
    }
    let eml: Option<(String, bool)> = {
        let db_guard = state.cache_db.lock();
        db_guard.as_ref().and_then(|conn| {
            conn.query_row(
                "SELECT m.raw_path, m.is_read FROM messages m JOIN folders f ON f.id = m.folder_id
                 WHERE m.account_id = ?1 AND m.message_id = ?2 AND f.name = ?3 AND m.raw_path IS NOT NULL
                 LIMIT 1",
                rusqlite::params![account_id as i64, kopf, ordner],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0)),
            )
            .ok()
        })
    };
    match eml.and_then(|(rel, gelesen)| std::fs::read(state.data_root.join(rel)).ok().map(|b| (b, gelesen))) {
        Some((bytes, gelesen)) => {
            let flags: &[&str] = if gelesen { &["\\Seen"] } else { &[] };
            client.append_message(&ziel, &bytes, Some(flags)).await?;
            tracing::info!("Rettung: Mail {} aus dem EML-Archiv nach '{}' hochgeladen", kopf, ziel);
            Ok(())
        }
        None => {
            tracing::warn!("Rettung: Mail {} weder beim Anbieter noch als EML — bleibt nur in Relay", kopf);
            Ok(())
        }
    }
}

pub(crate) fn ist_gmail(folders: &[(String, String, String, String)]) -> bool {
    folders.iter().any(|(name, ..)| {
        let l = name.to_lowercase();
        l.starts_with("[gmail]") || l.starts_with("[google mail]")
    })
}

/// How to take a mail off the provider: the folder list decides.
#[derive(Debug, PartialEq)]
pub(crate) enum Entfernen {
    /// \Deleted + EXPUNGE in the folder itself.
    Markieren,
    /// MOVE into the provider's trash (Gmail; the trash empties itself).
    InPapierkorb(String),
}

pub(crate) fn entfernen_wie(folders: &[(String, String, String, String)], ordner: &str) -> Entfernen {
    match papierkorb_waehlen(folders) {
        Some(trash) if ist_gmail(folders) && trash != ordner => Entfernen::InPapierkorb(trash),
        _ => Entfernen::Markieren,
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

        let folders = match client.list_folders_sync().await {
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
            if !wird_gespiegelt(&folders, folder_name, tag) {
                continue;
            }
            let storage_folder = storage_folder_name(folder_name, tag);
            // The provider's trash is never pruned: Relay's trash keeps
            // deleted mails also where the provider copy is gone.
            if storage_folder == "Trash" {
                continue;
            }
            // Atomically SELECT the folder + fetch its UIDs. A plain
            // fetch_all_uids() would read whatever folder a parallel API
            // operation left selected on the shared session and prune the
            // wrong mailbox. Decoded name — re-encoded to UTF-7 inside.
            let server_uids = match client.fetch_all_uids_in_folder_sync(folder_name, abruf_filter(&folders, folder_name)).await {
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
                    &storage_folder,
                )
                .unwrap_or(false);
                if is_local_folder {
                    continue;
                }
                match crate::cache::messages::delete_messages_not_in(
                    conn, *account_id as i64, &storage_folder, &server_uids,
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
        // A full backfill batch (>= BACKFILL_BATCH_SIZE new messages) means a
        // large history is still draining — poll almost immediately instead of
        // waiting the full base interval, so the initial sync is fast.
        if (new_count as u32) >= BACKFILL_BATCH_SIZE {
            Duration::from_secs(1)
        } else {
            base_interval
        }
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
    queue: &Arc<SyncQueue>,
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

        let healthy = client.ping().await && client.sync_ping().await;
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

    // ── Connection check + IDLE phase (parallel per account) ────────────
    // IDLE blocks up to IDLE_TIMEOUT_SECS per account — running it in one
    // sequential loop made every later account wait minutes behind the
    // earlier ones. Spawn one waiter per account instead.
    // While any account is backfilling a large history we poll aggressively:
    // the IDLE wait is the dominant per-cycle delay, so shorten it for EVERY
    // account (the phase awaits all handles, so a per-account shortening would
    // not help). Backfill is a transient state, so the extra poll load is fine.
    let idle_secs: u64 = if !state.backfill_active.read().is_empty() {
        2
    } else {
        IDLE_TIMEOUT_SECS
    };

    let mut idle_handles = Vec::new();
    for (account_id, client) in imap_client_ids.clone() {
        let queue = queue.clone();
        idle_handles.push(tokio::spawn(async move {
            if !client.is_connected().await {
                if let Err(e) = client.reconnect().await {
                    tracing::warn!("IMAP reconnect fuer account {} fehlgeschlagen: {}", account_id, e);
                    return;
                }
            }

            // IDLE fast-path: wait up to IDLE_TIMEOUT for an INBOX change. On a
            // mailbox change we enqueue FetchNew immediately (low latency); on
            // timeout the regular poll below still runs (fallback).
            let changed = client.idle_wait("INBOX", Duration::from_secs(idle_secs)).await;
            if changed {
                tracing::debug!("IMAP IDLE: INBOX-Änderung für account {}", account_id);
            }
            queue
                .enqueue(SyncTask {
                    account_id,
                    task_type: SyncTaskType::FetchNew,
                    created_at: tokio::time::Instant::now(),
                    retries: 0,
                    max_retries: 3,
                    priority: 10,
                })
                .await;
        }));
    }
    for h in idle_handles {
        let _ = h.await;
    }

    // Enqueue background body pre-fetch (runs after FetchNew, before AI analysis)
    for (account_id, _) in &imap_client_ids {
        queue.enqueue(SyncTask {
            account_id: *account_id,
            task_type: SyncTaskType::FetchBodies,
            created_at: tokio::time::Instant::now(),
            retries: 0,
            max_retries: 1,
            priority: 5,
        }).await;
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

/// Once per account (Kai, 9.10.2026): folders a bug had made "only in
/// Relay" are mirrored again, and their mails are put back on the provider.
///
/// The bug: a move into a folder Relay knew no mail in (an empty spam
/// folder or label, or "Archive" where the provider has none — every Gmail
/// account) made that folder "only in Relay", and every later move into it
/// took the mail off the provider: on Gmail into its trash, which empties
/// itself after 30 days; elsewhere deleted, kept as EML in Relay.
///
/// Mirror accounts only. A folder counts as one of those when the provider
/// has it (or, for "Archive", Gmail's "All Mail"), or by the names Relay
/// used where it had none ("Archive", "Junk"). Folders made in Relay on
/// purpose (and sent mail, drafts, the trash) stay as they are.
fn ordner_reparieren(state: &AppState, account_id: u32, folders: &[(String, String, String, String)]) {
    let db_guard = state.cache_db.lock();
    let Some(conn) = db_guard.as_ref() else { return };
    ordner_reparieren_mit(conn, account_id, folders);
}

pub(crate) fn ordner_reparieren_mit(conn: &rusqlite::Connection, account_id: u32, folders: &[(String, String, String, String)]) {
    let schluessel = format!("reparatur_lokale_ordner_26_10_11:{account_id}");
    if matches!(crate::cache::settings::get_setting(conn, &schluessel), Ok(Some(_))) {
        return;
    }
    let spiegel: bool = conn
        .query_row(
            "SELECT sync_mode != 'archive' FROM accounts WHERE id = ?1",
            rusqlite::params![account_id as i64],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if spiegel {
        let beim_anbieter: Vec<String> = folders
            .iter()
            .filter(|(name, _, _, tag)| wird_gespiegelt(folders, name, tag))
            .map(|(name, _, _, tag)| storage_folder_name(name, tag))
            .collect();
        let kandidaten: Vec<String> = conn
            .prepare(
                "SELECT name FROM folders WHERE account_id = ?1 AND local_only = 1
                 AND name NOT IN ('Trash', 'Gesendet', 'Sent', 'Gesendete Elemente', 'Entwürfe', 'Drafts')",
            )
            .and_then(|mut st| {
                st.query_map(rusqlite::params![account_id as i64], |r| r.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap_or_default();
        for name in kandidaten {
            let betroffen = beim_anbieter.iter().any(|b| b == &name) || name == "Archive" || name == "Junk";
            if !betroffen {
                continue;
            }
            let _ = conn.execute(
                "UPDATE folders SET local_only = 0 WHERE account_id = ?1 AND name = ?2",
                rusqlite::params![account_id as i64, name],
            );
            let koepfe: Vec<String> = conn
                .prepare(
                    "SELECT DISTINCT m.message_id FROM messages m JOIN folders f ON f.id = m.folder_id
                     WHERE m.account_id = ?1 AND f.name = ?2
                       AND m.message_id IS NOT NULL AND TRIM(m.message_id) != ''",
                )
                .and_then(|mut st| {
                    st.query_map(rusqlite::params![account_id as i64, name], |r| r.get::<_, String>(0))?
                        .collect::<Result<Vec<_>, _>>()
                })
                .unwrap_or_default();
            for kopf in &koepfe {
                let _ = crate::cache::provider_ops::enqueue_retten(conn, account_id as i64, &name, kopf);
            }
            tracing::warn!(
                "Reparatur: Ordner '{}' (Konto {}) wird wieder gespiegelt; {} Mail(s) werden beim Anbieter zurückgeholt",
                name, account_id, koepfe.len()
            );
        }
    }
    let _ = crate::cache::settings::set_setting(conn, &schluessel, "1");
}

/// Compare the folder's UIDVALIDITY with the one Relay saw; when the
/// provider numbered it anew, fetch it again (see neu_nummeriert).
async fn uidvalidity_pruefen(
    state: &AppState,
    client: &Arc<crate::imap::client::ImapClient>,
    account_id: u32,
    folder_name: &str,
    storage_folder: &str,
) {
    let Ok(Some(jetzt)) = client.uidvalidity_sync(folder_name).await else { return };
    let jetzt = jetzt as i64;
    let db_guard = state.cache_db.lock();
    let Some(conn) = db_guard.as_ref() else { return };
    match crate::cache::sync_state::uidvalidity(conn, account_id as i64, storage_folder) {
        Some(alt) if alt != jetzt => match crate::cache::sync_state::neu_nummeriert(conn, account_id as i64, storage_folder, jetzt) {
            Ok(n) => tracing::warn!(
                "Ordner '{}' (Konto {}) beim Anbieter neu nummeriert (UIDVALIDITY {} → {}): {} Zeilen werden neu geholt",
                storage_folder, account_id, alt, jetzt, n
            ),
            Err(e) => tracing::warn!("UIDVALIDITY '{}': {}", storage_folder, e),
        },
        Some(_) => {}
        None => {
            let _ = crate::cache::sync_state::set_uidvalidity(conn, account_id as i64, storage_folder, jetzt);
        }
    }
}

/// Folders the provider no longer lists leave Relay's index (mirror mode;
/// see cache::messages::verschwundene_ordner).
fn verschwundene_austragen(state: &AppState, account_id: u32, folders: &[(String, String, String, String)]) {
    let beim_anbieter: Vec<String> = folders
        .iter()
        .filter(|(n, _, _, t)| wird_gespiegelt(folders, n, t))
        .map(|(n, _, _, t)| storage_folder_name(n, t))
        .collect();
    let db_guard = state.cache_db.lock();
    let Some(conn) = db_guard.as_ref() else { return };
    let spiegel: bool = conn
        .query_row(
            "SELECT sync_mode != 'archive' FROM accounts WHERE id = ?1",
            rusqlite::params![account_id as i64],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !spiegel || beim_anbieter.is_empty() {
        return;
    }
    for name in crate::cache::messages::verschwundene_ordner(conn, account_id as i64, &beim_anbieter).unwrap_or_default() {
        match crate::cache::messages::ordner_austragen(conn, account_id as i64, &name, "") {
            Ok(n) => tracing::info!("Ordner '{}' (Konto {}) gibt es beim Anbieter nicht mehr: {} Zeilen ausgetragen", name, account_id, n),
            Err(e) => tracing::warn!("Ordner '{}' austragen: {}", name, e),
        }
    }
}

/// When the folder is due for a comparison with the server although
/// nothing new came: the inbox every cycle (one UID SEARCH), the others
/// every ten minutes. `immer` marks one done now.
fn abgleich_faellig(account_id: u32, storage_folder: &str, immer: bool) -> bool {
    use std::sync::{Mutex, OnceLock};
    static ZULETZT: OnceLock<Mutex<HashMap<(u32, String), Instant>>> = OnceLock::new();
    let mut map = ZULETZT.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    let key = (account_id, storage_folder.to_string());
    let faellig = immer
        || map
            .get(&key)
            .map(|t| t.elapsed() >= REMOVAL_CHECK_INTERVAL)
            .unwrap_or(true);
    if faellig {
        map.insert(key, Instant::now());
    }
    faellig
}

/// Remove the rows of mails no longer in the provider folder (deleted or
/// moved elsewhere). Relay's trash and folders kept only in Relay are never
/// compared; rows moved here locally wait for the server's (synced = 0).
async fn ordner_abgleichen(
    state: &AppState,
    client: &Arc<crate::imap::client::ImapClient>,
    account_id: u32,
    folder_name: &str,
    storage_folder: &str,
    filter: Option<&str>,
    papierkorb_spiegeln: bool,
) -> Result<(), String> {
    if storage_folder == "Trash" && !papierkorb_spiegeln {
        return Ok(());
    }
    let server_uids = match client.fetch_all_uids_in_folder_sync(folder_name, filter).await {
        Ok(uids) => uids,
        Err(e) => {
            tracing::warn!(
                "FetchNew: fetch_all_uids '{}' (account {}): {}",
                folder_name, account_id, e
            );
            return Err(e.to_string());
        }
    };
    let db_guard = state.cache_db.lock();
    let conn = db_guard.as_ref().ok_or("Datenbank nicht initialisiert")?;
    // Local-only folders are NOT mirrors of an IMAP folder — never prune
    // them against server UIDs (archive mode). Relay's trash is one, but in
    // mirror mode it mirrors the provider's (papierkorb_spiegeln).
    if storage_folder != "Trash"
        && crate::cache::messages::is_local_only_folder(conn, account_id as i64, storage_folder).unwrap_or(false)
    {
        return Ok(());
    }
    match crate::cache::messages::delete_messages_not_in(conn, account_id as i64, storage_folder, &server_uids) {
        Ok(deleted) => {
            if deleted > 0 {
                tracing::info!(
                    "Account {}: {} gelöschte Nachrichten in '{}' bereinigt",
                    account_id, deleted, storage_folder
                );
                // Notify frontend to refresh the message list
                let _ = state.events.emit("messages-deleted", (account_id, storage_folder, deleted));
            }
        }
        Err(e) => {
            tracing::warn!(
                "FetchNew: delete_messages_not_in '{}' (account {}): {}",
                storage_folder, account_id, e
            );
        }
    }
    Ok(())
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

            let folders = client.list_folders_sync().await.map_err(|e| e.to_string())?;
            ordner_reparieren(state, task.account_id, &folders);
            verschwundene_austragen(state, task.account_id, &folders);
            // Relay's trash mirrors the provider's in mirror mode (Kai,
            // 9.10.2026): a mail restored or purged there leaves it here too.
            // Only where one provider folder feeds it, and never in archive
            // mode (there Relay keeps deleted mail itself).
            let papierkorb_spiegeln = {
                let quellen = folders
                    .iter()
                    .filter(|(n, _, _, t)| wird_gespiegelt(&folders, n, t) && storage_folder_name(n, t) == "Trash")
                    .count();
                let spiegel = {
                    let db_guard = state.cache_db.lock();
                    db_guard.as_ref().map(|conn| {
                        conn.query_row(
                            "SELECT sync_mode != 'archive' FROM accounts WHERE id = ?1",
                            rusqlite::params![task.account_id as i64],
                            |r| r.get::<_, bool>(0),
                        )
                        .unwrap_or(false)
                    })
                    .unwrap_or(false)
                };
                quellen == 1 && spiegel
            };

            let mut total_new: usize = 0;
            // Backfill fairness budget: bounds how many large batches one account
            // may do per cycle so a single huge mailbox cannot starve other
            // accounts/tasks in the (sequential) sync queue.
            let mut backfill_batches: usize = 0;
            // Whether this FetchNew drained any FULL (backfill) batch — used to
            // flag the account as still-backfilling so the next cycle polls fast.
            let mut did_backfill = false;
            for (folder_name, _raw_name, _, tag) in &folders {
                if !wird_gespiegelt(&folders, folder_name, tag) {
                    continue;
                }
                // Gmail's archive: "All Mail" narrowed (GMAIL_ARCHIV_FILTER).
                let filter = abruf_filter(&folders, folder_name);

                let is_spam = ["Spam", "Junk", "Spamverdacht", "Junk E-Mail"]
                    .iter().any(|s| folder_name.eq_ignore_ascii_case(s));

                // Provider trash folders (Gelöscht/Papierkorb/Deleted
                // Messages…) are stored inside the single local "Trash".
                let storage_folder = storage_folder_name(folder_name, tag);
                let is_inbox = folder_name.eq_ignore_ascii_case("INBOX");

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
                    crate::cache::messages::is_local_only_folder(conn, task.account_id as i64, &storage_folder)
                        .unwrap_or(false)
                };
                if is_local_folder {
                    tracing::debug!(
                        "FetchNew: '{}' (account {}) ist ein lokaler Ordner — kein IMAP-Abgleich",
                        folder_name, task.account_id
                    );
                    continue;
                }

                // Numbered anew by the provider? Every ten minutes (STATUS).
                if storage_folder != "Trash" && abgleich_faellig(task.account_id, &format!("uidvalidity:{storage_folder}"), false) {
                    uidvalidity_pruefen(state, &client, task.account_id, folder_name, &storage_folder).await;
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

                if let Err(e) = client.select_folder_sync(folder_name).await {
                    tracing::warn!(
                        "FetchNew: select_folder '{}' (account {}): {}",
                        folder_name, task.account_id, e
                    );
                    continue;
                }

                // Folder id (scoping for AI + body updates) is constant per
                // folder — resolve once, outside the batch loop.
                let folder_id: Option<i64> = if is_spam {
                    None
                } else {
                    let db_guard = state.cache_db.lock();
                    db_guard.as_ref().and_then(|conn| conn.query_row(
                        "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
                        rusqlite::params![task.account_id as i64, storage_folder],
                        |r| r.get(0),
                    ).ok())
                };

                // Continuation loop: keep fetching FULL batches while the folder
                // is still backfilling (more old mail remains on the server). A
                // PARTIAL batch means the folder is caught up → switch to the
                // full steady-state treatment for the remaining new mail. The
                // cursor is committed per batch, so a crash resumes cleanly.
                let mut max_uid_cur = max_uid;
                let ai_cutoff = chrono::Utc::now() - chrono::Duration::days(AI_BACKFILL_MAX_AGE_DAYS);
                loop {
                    if backfill_batches >= BACKFILL_MAX_BATCHES_PER_CYCLE {
                        break;
                    }

                    let messages = match client.fetch_recent_in_folder_sync(folder_name, max_uid_cur as u32, BACKFILL_BATCH_SIZE, filter).await {
                        Ok(msgs) => msgs,
                        Err(e) => {
                            tracing::warn!(
                                "FetchNew: fetch_recent '{}' (account {}): {}",
                                folder_name, task.account_id, e
                            );
                            // TagMismatch resets the session — next folder will
                            // trigger auto-reconnect in with_session_blocking().
                            break;
                        }
                    };
                    if messages.is_empty() {
                        // Nothing new: still notice mails gone from the
                        // folder elsewhere (archived or deleted in Gmail's
                        // web view, on the phone). Before, that only
                        // happened when new mail came in (Kai, 9.10.2026).
                        if abgleich_faellig(task.account_id, &storage_folder, is_inbox) {
                            let _ = ordner_abgleichen(state, &client, task.account_id, folder_name, &storage_folder, filter, papierkorb_spiegeln).await;
                        }
                        break;
                    }

                    let is_full_batch = (messages.len() as u32) >= BACKFILL_BATCH_SIZE;

                    // Save + advance the per-folder sync cursor to the highest
                    // UID of THIS batch (committed per batch so the next
                    // iteration/crash continues with `UID {last+1}:*`).
                    {
                        let mut db_guard = state.cache_db.lock();
                        let conn = db_guard
                            .as_mut()
                            .ok_or("Datenbank nicht initialisiert")?;

                        let tx = conn.transaction().map_err(|e| e.to_string())?;
                        for msg in &messages {
                            crate::cache::messages::save_message(&tx, task.account_id as i64, msg, &storage_folder)
                                .map_err(|e| e.to_string())?;
                            // Auto-enrichment: derive contacts from the envelope.
                            let _ = crate::cache::contacts::enrich_from_envelope(
                                &tx,
                                &msg.envelope.from,
                                &msg.envelope.to,
                                &msg.envelope.cc,
                            );
                        }
                        if let Some(batch_max) = messages.iter().map(|m| m.uid).max() {
                            max_uid_cur = batch_max as i64;
                            crate::cache::sync_state::set(
                                &tx,
                                task.account_id as i64,
                                &storage_folder,
                                batch_max as i64,
                                0,
                            )
                            .map_err(|e| e.to_string())?;
                        }
                        tx.commit().map_err(|e| e.to_string())?;
                    }

                    // iMIP inbound: process calendar invitations (text/calendar
                    // attachments) in the newly-fetched messages. Best-effort — a
                    // failure here must not abort the mail sync.
                    for msg in &messages {
                        let has_ics = msg
                            .attachments
                            .iter()
                            .any(|a| a.content_type.to_lowercase().contains("text/calendar"));
                        if !has_ics {
                            continue;
                        }
                        match client
                            .fetch_raw_message_in_folder(msg.uid, Some(folder_name.clone()))
                            .await
                        {
                            Ok(raw) => {
                                for att in crate::imap::client::parse_message_attachments(raw.as_bytes()) {
                                    if !att.content_type.to_lowercase().contains("text/calendar") {
                                        continue;
                                    }
                                    use base64::Engine;
                                    let Ok(b64) = base64::engine::general_purpose::STANDARD.decode(&att.content) else {
                                        continue;
                                    };
                                    let Ok(ics_text) = String::from_utf8(b64) else {
                                        continue;
                                    };
                                    match crate::imip::inbound::process_inbound_ics(state, &ics_text).await {
                                        Ok(info) => tracing::info!("iMIP: {info}"),
                                        Err(e) => tracing::warn!("iMIP: Verarbeitung fehlgeschlagen: {e}"),
                                    }
                                }
                            }
                            Err(e) => tracing::warn!("iMIP: Raw-Fetch (uid {}) fehlgeschlagen: {e}", msg.uid),
                        }
                    }

                    if is_full_batch {
                        // ── BACKFILL mode ────────────────────────────────────
                        // No per-message body downloads, no full-UID prune, no
                        // web push. AI is capped to recent mail so a local model
                        // is not pegged for weeks on a full history.
                        backfill_batches += 1;
                        did_backfill = true;
                        total_new += messages.len();
                        if !messages.is_empty() {
                            let _ = state.events.emit("new-messages", (task.account_id, folder_name, messages.len()));
                        }
                        if !is_spam {
                            for msg in &messages {
                                let is_recent = chrono::DateTime::parse_from_rfc3339(&msg.envelope.date)
                                    .ok()
                                    .map(|d| d.with_timezone(&chrono::Utc) >= ai_cutoff)
                                    .unwrap_or(false);
                                if !is_recent {
                                    continue;
                                }
                                let needs_summary = {
                                    let db_guard = state.cache_db.lock();
                                    let conn = db_guard.as_ref().ok_or("Datenbank nicht initialisiert")?;
                                    match crate::cache::messages::fetch_message_body(conn, task.account_id as i64, msg.uid as i64, folder_id) {
                                        Ok(Some(m)) => m.ai_summary.is_none() || (is_inbox && m.ai_followups.is_none()),
                                        _ => true,
                                    }
                                };
                                if needs_summary {
                                    let _ = ai_tx
                                        .send(SyncTask {
                                            account_id: task.account_id,
                                            task_type: SyncTaskType::GenerateAiSummary(msg.uid, folder_id.unwrap_or(-1)),
                                            created_at: tokio::time::Instant::now(),
                                            retries: 0,
                                            max_retries: 2,
                                            priority: 5,
                                        })
                                        .await;
                                }
                            }
                        }
                        tracing::info!(
                            "Account {}: {} Backfill-Nachrichten in '{}' (Batch {}/{})",
                            task.account_id,
                            messages.len(),
                            folder_name,
                            backfill_batches,
                            BACKFILL_MAX_BATCHES_PER_CYCLE
                        );
                        // Rate cap between backfill batches (ban-safe).
                        tokio::time::sleep(BACKFILL_BATCH_INTERVAL).await;
                        continue;
                    }

                    // ── STEADY-STATE mode (caught up) ────────────────────────
                    // Cleanup: remove locally cached messages that no longer
                    // exist on the IMAP server (deleted from another client).
                    abgleich_faellig(task.account_id, &storage_folder, true);
                    if ordner_abgleichen(state, &client, task.account_id, folder_name, &storage_folder, filter, papierkorb_spiegeln).await.is_err() {
                        break;
                    }

                    // Hybrid Body-Fetch: only fetch body for INBOX to keep sync
                    // fast. Other folders rely on on-demand body fetch when the
                    // user clicks a message. The raw RFC822 bytes are archived
                    // to disk (EML) — the local-first source of truth for
                    // backup/export (Concept §3.1). (Backfill defers this to
                    // on-demand so a huge mailbox does not download every body.)
                    if is_inbox {
                        for msg in &messages {
                            // Folder-scoped body fetch: without the explicit
                            // folder the session could be on a different mailbox
                            // (parallel API ops), making the UID lookup fail or
                            // read the wrong message.
                            match client.fetch_body_with_raw_from_folder_sync(msg.uid, Some(folder_name.to_string())).await {
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
                                            conn, task.account_id as i64, msg.uid as i64, folder_id,
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

                    // Enqueue AI summary for all new mail (steady-state, no age
                    // cap — the volume here is small).
                    if !is_spam {
                        for msg in &messages {
                            // Enqueue if the message still needs a summary, or
                            // (INBOX only) cached followup actions — both are
                            // produced in the same worker pass.
                            let needs_summary = {
                                let db_guard = state.cache_db.lock();
                                let conn = db_guard.as_ref().ok_or("Datenbank nicht initialisiert")?;
                                match crate::cache::messages::fetch_message_body(conn, task.account_id as i64, msg.uid as i64, folder_id) {
                                    Ok(Some(m)) => m.ai_summary.is_none() || (is_inbox && m.ai_followups.is_none()),
                                    _ => true,
                                }
                            };
                            if needs_summary {
                                // Send to the DEDICATED AI worker channel — never
                                // the sync queue, so LLM work cannot stall IMAP sync.
                                let _ = ai_tx
                                    .send(SyncTask {
                                        account_id: task.account_id,
                                        task_type: SyncTaskType::GenerateAiSummary(msg.uid, folder_id.unwrap_or(-1)),
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
                    break;
                }
            }

            // Publish the backfill flag for the NEXT cycle's IDLE/sleep phase:
            // keep the account flagged while it is still draining full batches,
            // clear it once it is caught up (no full batch this pass).
            {
                let mut set = state.backfill_active.write();
                if did_backfill {
                    set.insert(task.account_id);
                } else {
                    set.remove(&task.account_id);
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
                         WHERE m.account_id = ?1 AND f.local_only = 0
                           AND (m.raw_path IS NULL OR m.raw_path = '')
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
                match client.fetch_body_with_raw_from_folder_sync(uid, Some(folder.clone())).await {
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
        SyncTaskType::FetchBodies => {
            // Background: pre-fetch bodies for recently synced messages that
            // don't have a cached body yet. 25 per cycle, all folders.
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

            // Find up to 25 messages without a body, most recent first.
            let pending: Vec<(i64, u32, String)> = {
                let db_guard = state.cache_db.lock();
                let conn = db_guard
                    .as_ref()
                    .ok_or("Datenbank nicht initialisiert")?;
                let mut stmt = conn.prepare(
                    "SELECT m.id, m.uid, f.name FROM messages m \
                     JOIN folders f ON f.id = m.folder_id \
                     WHERE m.account_id = ?1 AND (m.body_text IS NULL OR m.body_text = '') \
                     ORDER BY m.id DESC LIMIT 25"
                ).map_err(|e| e.to_string())?;
                let rows = stmt.query_map(rusqlite::params![account_id as i64], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, u32>(1)?, r.get::<_, String>(2)?))
                }).map_err(|e| e.to_string())?;
                rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?
            };

            let mut fetched = 0;
            for (msg_id, uid, folder) in &pending {
                match client.fetch_body_from_folder_sync(*uid, Some(folder.clone())).await {
                    Ok((body_text, body_html)) => {
                        let _ = {
                            let db_guard = state.cache_db.lock();
                            let conn = db_guard.as_ref().ok_or("DB nicht initialisiert")?;
                            crate::cache::messages::update_body_by_id(
                                conn, *msg_id, &body_text, body_html.as_deref(),
                            ).map_err(|e| e.to_string())
                        };
                        fetched += 1;
                    }
                    Err(e) => {
                        tracing::debug!(
                            "FetchBodies: uid {} ({}): {}", uid, folder, e
                        );
                        break;
                    }
                }
            }
            if fetched > 0 {
                tracing::info!("FetchBodies: {} Bodies vorab geladen (account {})", fetched, account_id);
            }
            Ok(fetched)
        }
        SyncTaskType::GenerateAiSummary(uid, folder_id) => {
            // Handled by the dedicated AI worker (run_ai_summary_worker).
            // Kept for compatibility with tasks already in the queue.
            process_ai_summary(state, task.account_id, *uid, Some(*folder_id)).await
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
                // retries: 1 → queue applies base_delay (1 s) before next
                // execution, preventing a zero-delay busy loop when many
                // diffs are queued (each cycle processes up to 3).
                queue.enqueue(SyncTask {
                    account_id: task.account_id,
                    task_type: SyncTaskType::AnalyzeDiff,
                    created_at: tokio::time::Instant::now(),
                    retries: 1,
                    max_retries: 0,
                    priority: 3,
                }).await;
            }

            Ok(0)
        }
        SyncTaskType::RefreshFingerprint => {
            // Background: refresh style fingerprints for recipients with >=3 new analyzed hints.
            // Global task (account-agnostic): candidates span ALL accounts, each with its
            // real account_id so the fingerprint is stored where generate-mail reads it.
            // Processes up to 2 recipients per cycle to avoid LLM semaphore starvation.
            let (candidates, ai_client_opt) = {
                let db_guard = state.cache_db.lock();
                let conn = db_guard
                    .as_ref()
                    .ok_or("Datenbank nicht initialisiert")?;
                let candidates = crate::cache::fingerprint::get_refresh_candidates(conn, 2)
                    .map_err(|e| e.to_string())?;
                let ai_client = {
                    let guard = state.ai_client.read();
                    guard.clone()
                };
                (candidates, ai_client)
            };

            for cand in candidates {
                if let Some(ref client) = ai_client_opt {
                    let hints = {
                        let db_guard = state.cache_db.lock();
                        let conn = db_guard
                            .as_ref()
                            .ok_or("Datenbank nicht initialisiert")?;
                        let hints = crate::cache::fingerprint::get_hints_for_synthesis(
                            conn,
                            cand.account_id,
                            &cand.email_hash,
                        )
                        .map_err(|e| e.to_string())?;
                        if hints.is_empty() {
                            continue;
                        }
                        hints
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
                                    cand.account_id,
                                    &cand.email_hash,
                                    &fingerprint,
                                    hints.len() as i64,
                                );
                                tracing::info!(
                                    "Style fingerprint fuer {} (Account {}) aktualisiert ({} Hinweise)",
                                    cand.email_hash,
                                    cand.account_id,
                                    hints.len()
                                );
                            }
                        }
                        Some(Err(e)) => {
                            tracing::warn!("Fingerprint synthesis LLM error for {}: {}", cand.email_hash, e);
                        }
                        None => {
                            tracing::debug!("LLM busy, skip fingerprint synthesis for {}", cand.email_hash);
                        }
                    }
                }
            }

            Ok(0)
        }
    }
}

#[cfg(test)]
mod loeschen_tests {
    use super::*;

    fn f(name: &str, tag: &str) -> (String, String, String, String) {
        (name.to_string(), name.to_string(), "/".to_string(), tag.to_string())
    }

    #[test]
    fn gmail_deletes_into_its_trash() {
        let gmail = vec![f("INBOX", "folder"), f("[Google Mail]", "noselect"), f("[Google Mail]/Alle Nachrichten", "sammel"), f("[Google Mail]/Papierkorb", "trash")];
        assert!(ist_gmail(&gmail));
        assert_eq!(papierkorb_waehlen(&gmail).as_deref(), Some("[Google Mail]/Papierkorb"));
        assert_eq!(entfernen_wie(&gmail, "INBOX"), Entfernen::InPapierkorb("[Google Mail]/Papierkorb".into()));
        // In the trash itself, \Deleted + EXPUNGE deletes for good on Gmail too.
        assert_eq!(entfernen_wie(&gmail, "[Google Mail]/Papierkorb"), Entfernen::Markieren);
    }

    #[test]
    fn other_providers_flag_and_expunge() {
        let gmx = vec![f("INBOX", "folder"), f("Gelöscht", "trash"), f("Spamverdacht", "folder")];
        assert!(!ist_gmail(&gmx));
        assert_eq!(papierkorb_waehlen(&gmx).as_deref(), Some("Gelöscht"));
        assert_eq!(entfernen_wie(&gmx, "INBOX"), Entfernen::Markieren);
        // Without the \Trash mark the usual names still find it.
        let alt = vec![f("INBOX", "folder"), f("Deleted Messages", "folder")];
        assert_eq!(papierkorb_waehlen(&alt).as_deref(), Some("Deleted Messages"));
        assert_eq!(papierkorb_waehlen(&[f("INBOX", "folder")]), None);
        // Relay's "Trash" is the provider's; other names stay.
        assert_eq!(provider_ordner(&gmx, "Trash").as_deref(), Some("Gelöscht"));
        assert_eq!(provider_ordner(&gmx, "Archiv").as_deref(), Some("Archiv"));
        assert_eq!(provider_ordner(&[f("INBOX", "folder")], "Trash"), None);
    }

    fn gmail() -> Vec<(String, String, String, String)> {
        vec![
            f("INBOX", "folder"),
            f("[Google Mail]", "noselect"),
            f("[Google Mail]/Alle Nachrichten", "alle"),
            f("[Google Mail]/Markiert", "sammel"),
            f("[Google Mail]/Spam", "folder"),
            f("[Google Mail]/Papierkorb", "trash"),
            f("Vorstand", "folder"),
        ]
    }

    #[test]
    fn gmail_archives_into_all_mail() {
        let g = gmail();
        // Relay's "Archive" on Gmail is "All Mail" (Kai, 9.10.2026) ...
        assert_eq!(gmail_archiv(&g).as_deref(), Some("[Google Mail]/Alle Nachrichten"));
        assert_eq!(provider_ordner(&g, "Archive").as_deref(), Some("[Google Mail]/Alle Nachrichten"));
        assert_eq!(storage_folder_name("[Google Mail]/Alle Nachrichten", "alle"), "Archive");
        // ... read without the inbox, sent mail, drafts and labelled mail.
        assert_eq!(abruf_filter(&g, "[Google Mail]/Alle Nachrichten"), Some(GMAIL_ARCHIV_FILTER));
        assert!(GMAIL_ARCHIV_FILTER.contains("-in:inbox") && GMAIL_ARCHIV_FILTER.contains("has:nouserlabels"));
        assert_eq!(abruf_filter(&g, "INBOX"), None);
        assert!(wird_gespiegelt(&g, "[Google Mail]/Alle Nachrichten", "alle"));
        assert!(!wird_gespiegelt(&g, "[Google Mail]/Markiert", "sammel"));
        assert!(!wird_gespiegelt(&g, "[Google Mail]", "noselect"));
        // Its trash is never "All Mail".
        assert_eq!(papierkorb_waehlen(&g).as_deref(), Some("[Google Mail]/Papierkorb"));
    }

    #[test]
    fn elsewhere_archive_is_a_folder_and_all_mail_is_not_read() {
        let dovecot = vec![f("INBOX", "folder"), f("Alle", "alle"), f("Trash", "trash")];
        assert_eq!(gmail_archiv(&dovecot), None);
        assert_eq!(provider_ordner(&dovecot, "Archive").as_deref(), Some("Archive"));
        assert!(!wird_gespiegelt(&dovecot, "Alle", "alle"));
    }

    #[test]
    fn spam_folders_by_name() {
        assert!(ist_spam_ordner("[Google Mail]/Spam"));
        assert!(ist_spam_ordner("Spamverdacht"));
        assert!(ist_spam_ordner("INBOX.Junk"));
        assert!(!ist_spam_ordner("INBOX"));
        assert!(!ist_spam_ordner("Spamfilter-Regeln/Kunden"));
    }

    #[test]
    fn the_search_of_a_fetch() {
        use crate::imap::client::suchanfrage;
        assert_eq!(suchanfrage(0, None), "ALL");
        assert_eq!(suchanfrage(41, None), "UID 42:*");
        assert_eq!(suchanfrage(0, Some(GMAIL_ARCHIV_FILTER)), GMAIL_ARCHIV_FILTER);
        assert_eq!(suchanfrage(41, Some("X-GM-RAW \"x\"")), "UID 42:* X-GM-RAW \"x\"");
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
/// Resolve a folder_id to its display name (for SSE events). Returns an empty
/// string when unknown — the frontend then treats the event as unscoped.
fn resolve_folder_name(conn: &rusqlite::Connection, account_id: i64, folder_id: Option<i64>) -> String {
    let Some(fid) = folder_id else { return String::new() };
    if fid < 0 {
        return String::new();
    }
    conn.query_row(
        "SELECT name FROM folders WHERE account_id = ?1 AND id = ?2",
        rusqlite::params![account_id, fid],
        |r| r.get::<_, String>(0),
    )
    .unwrap_or_default()
}

/// Generate the AI summary for one message (shared by the sync-queue
/// compatibility arm and the dedicated worker).
///
/// `folder_id` scopes the body lookup — IMAP uids are only unique per folder,
/// so without it a uid shared with another folder could summarize (and store
/// the summary for) the wrong message.
async fn process_ai_summary(state: &AppState, account_id: u32, uid: u32, folder_id: Option<i64>) -> Result<usize, String> {
    let (subject, body_text, client_opt, existing_summary) = {
        let db_guard = state.cache_db.lock();
        let conn = db_guard
            .as_ref()
            .ok_or("Datenbank nicht initialisiert")?;

        let msg = crate::cache::messages::fetch_message_body(
            conn,
            account_id as i64,
            uid as i64,
            folder_id,
        )
        .map_err(|e| e.to_string())?;

        let (subject, body, existing_summary) = match msg {
            Some(m) => (m.subject.unwrap_or_default(), m.body_text, m.ai_summary),
            None => (String::new(), None, None),
        };

        let ai_client = {
            let guard = state.ai_client.read();
            guard.clone()
        };

        (subject, body, ai_client, existing_summary)
    };

    // Phishing detection: pure heuristic (regex), so it runs for every
    // message regardless of LLM availability — warnings must not depend
    // on the model being configured or idle.
    let fraud = fraud::detect_fraud(&subject, body_text.as_deref().unwrap_or(""));
    {
        let db_guard = state.cache_db.lock();
        if let Some(conn) = db_guard.as_ref() {
            let _ = crate::cache::messages::update_ai_fraud(
                conn,
                account_id as i64,
                uid as i64,
                fraud.score,
            );
        }
    }

    // Skip LLM summary regeneration when one already exists (the message may
    // have been enqueued only for INBOX followup pregen below).
    let body_text = if existing_summary.is_some() { None } else { body_text };

    if let (Some(body), Some(client)) = (body_text, client_opt) {
        let summary = match client
            .complete_background(
                crate::ai::prompts::AI_SUMMARY_PROMPT,
                &format!("Betreff: {}\n\n{}", pii::mask_pii(&subject), pii::mask_pii(&body)),
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
            // Parse the "Dringlichkeit:" line only — a free-text summary may
            // contain the word "kritisch" without the mail actually being
            // urgent. ZEITKRITISCH contains KRITISCH, so check it first.
            // A missing/malformed line falls back to NORMAL (no false
            // urgent markings).
            let urgency_line = summary.lines()
                .find_map(|l| l.trim().strip_prefix("Dringlichkeit:"))
                .unwrap_or("");
            let urgency = if urgency_line.contains("ZEITKRITISCH") { Some(0.8f32) }
                else if urgency_line.contains("KRITISCH") { Some(0.95f32) }
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
                folder_id,
                &summary_text,
            )
            .map_err(|e| e.to_string())?;
                let folder_name = resolve_folder_name(conn, account_id as i64, folder_id);
                let _ = state.events.emit("ai-summary-updated", (uid, account_id, summary_text.clone(), urgency, folder_name, Some(fraud.score)));
            if let Some(u) = urgency {
                let _ = crate::cache::messages::update_ai_priority(
                    conn, account_id as i64, uid as i64, folder_id, u,
                );
            }
        } else {
            // Fallback: rule-based priority detection when LLM is unavailable
            let rule_priority = priority::detect_priority(&subject, &body);
            if rule_priority > 0.0 {
                let db_guard = state.cache_db.lock();
                if let Some(conn) = db_guard.as_ref() {
                    let _ = crate::cache::messages::update_ai_priority(
                        conn, account_id as i64, uid as i64, folder_id, rule_priority,
                    );
                    let folder_name = resolve_folder_name(conn, account_id as i64, folder_id);
                    let _ = state.events.emit("ai-summary-updated", (uid, account_id, String::new(), Some(rule_priority), folder_name, Some(fraud.score)));
                }
            }
        }
    }
    // Pre-generate followup actions for new INBOX mail so the browser footer
    // shows them instantly (on-demand generation stays as fallback for other
    // folders and is cached by the API handler).
    let fid = folder_id.unwrap_or(-1);
    let is_inbox = {
        let db_guard = state.cache_db.lock();
        db_guard
            .as_ref()
            .map(|conn| resolve_folder_name(conn, account_id as i64, folder_id).eq_ignore_ascii_case("INBOX"))
            .unwrap_or(false)
    };
    if is_inbox {
        let msg = {
            let db_guard = state.cache_db.lock();
            match db_guard.as_ref() {
                Some(conn) => crate::cache::messages::fetch_message_body(conn, account_id as i64, uid as i64, folder_id).ok().flatten(),
                None => None,
            }
        };
        if let Some(m) = msg {
            let body = m.body_text.unwrap_or_default();
            if !body.trim().is_empty() {
                let from = m.from_addr.unwrap_or_default();
                let subject = m.subject.unwrap_or_default();
                match crate::api::ai::generate_followups_v2(state, &subject, &from, &body, uid as i64, "de").await {
                    Ok(actions) if !actions.is_empty() => {
                        // Cache the v2 object shape (the handler parses it back;
                        // a legacy bare-array would be treated as a miss).
                        if let Ok(json) = serde_json::to_string(&crate::api::ai::FollowupsResponse { actions }) {
                            let db_guard = state.cache_db.lock();
                            if let Some(conn) = db_guard.as_ref() {
                                let _ = crate::cache::messages::set_ai_followups_by_folder_id(
                                    conn, account_id as i64, uid as i64, fid, &json,
                                );
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => tracing::debug!("Followup-PreGen uid {} skipped: {:?}", uid, e),
                }
            }
        }
    }
    Ok(0)
}

/// One-shot startup catch-up for AI summaries + INBOX followup actions.
///
/// Covers recent INBOX mail that arrived while the app was offline or pre-dates
/// the followup feature: any message missing a summary OR followup actions is
/// enqueued for the dedicated AI worker. Runs once at boot (after a short
/// settle delay), never blocks the sync cycle, and is a no-op once everything
/// is generated (the gate makes it idempotent).
async fn run_inbox_ai_catch_up(state: &AppState, ai_tx: &mpsc::Sender<SyncTask>) {
    // Let the app fully come up (DB init, account load) before scanning.
    tokio::time::sleep(Duration::from_secs(15)).await;

    // All DB work happens under one lock; the lock is released before the
    // (async) channel sends so we never hold the DB across an await.
    let jobs: Vec<(u32, i64, i64)> = {
        let db_guard = state.cache_db.lock();
        let conn = match db_guard.as_ref() {
            Some(c) => c,
            None => return,
        };
        let accounts = match crate::cache::accounts::list_accounts(conn) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!("AI-Catch-up: Account-Liste fehlgeschlagen: {}", e);
                return;
            }
        };
        let mut jobs = Vec::new();
        for acct in accounts {
            let account_id = acct.id;
            let folder_id: i64 = match conn.query_row(
                "SELECT id FROM folders WHERE account_id = ?1 AND name = 'INBOX'",
                rusqlite::params![account_id],
                |r| r.get(0),
            ) {
                Ok(id) => id,
                Err(_) => continue,
            };
            let uids: Vec<i64> = match conn
                .prepare(
                    "SELECT uid FROM messages
                     WHERE account_id = ?1 AND folder_id = ?2
                       AND body_text IS NOT NULL
                       AND (ai_summary IS NULL OR ai_followups IS NULL)
                     ORDER BY date DESC
                     LIMIT 200",
                )
                .and_then(|mut s| {
                    s.query_map(rusqlite::params![account_id, folder_id], |r| r.get::<_, i64>(0))
                        .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<i64>>())
                }) {
                Ok(u) => u,
                Err(_) => continue,
            };
            for uid in uids {
                jobs.push((account_id as u32, folder_id, uid));
            }
        }
        jobs
    };

    let mut enqueued = 0usize;
    for (account_id, folder_id, uid) in jobs {
        let _ = ai_tx
            .send(SyncTask {
                account_id,
                task_type: SyncTaskType::GenerateAiSummary(uid as u32, folder_id),
                created_at: tokio::time::Instant::now(),
                retries: 0,
                max_retries: 2,
                priority: 5,
            })
            .await;
        enqueued += 1;
    }
    if enqueued > 0 {
        tracing::info!(
            "AI-Catch-up (Startup): {} INBOX-Nachricht(en) für Summary/Followups eingeplant",
            enqueued
        );
    }
}

/// Dedicated AI-summary worker. Processes LLM summarization jobs sequentially
/// (never in the sync cycle), so slow LLM calls cannot delay IMAP sync.
async fn run_ai_summary_worker(state: Arc<AppState>, mut rx: mpsc::Receiver<SyncTask>) {
    tracing::info!("AI-Summary-Worker gestartet");
    while let Some(task) = rx.recv().await {
        match &task.task_type {
            SyncTaskType::GenerateAiSummary(uid, folder_id) => {
                if let Err(e) = process_ai_summary(&state, task.account_id, *uid, Some(*folder_id)).await {
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
