//! Shared database helpers.
//!
//! Previously lived in `ipc.rs`; extracted so both the REST API handlers
//! and the background schedulers can access the SQLite connection.

use std::sync::Arc;

use parking_lot::MutexGuard;

use crate::AppState;

/// Lock the SQLite connection guard, erroring when the DB is not initialized.
pub fn get_db(
    state: &AppState,
) -> Result<MutexGuard<'_, Option<rusqlite::Connection>>, String> {
    let guard = state.cache_db.lock();
    if guard.is_none() {
        return Err("Datenbank nicht initialisiert. Bitte App neu starten.".into());
    }
    Ok(guard)
}

/// Non-failing variant for callers that prefer `Result` semantics anyway.
#[allow(dead_code)]
pub fn get_db_inner(
    state: &AppState,
) -> Result<MutexGuard<'_, Option<rusqlite::Connection>>, String> {
    get_db(state)
}

/// Run a closure with an open DB connection, erroring when unavailable.
pub fn with_db<T>(
    state: &AppState,
    f: impl FnOnce(&rusqlite::Connection) -> Result<T, String>,
) -> Result<T, String> {
    let guard = get_db(state)?;
    let conn = guard.as_ref().ok_or("Datenbank nicht initialisiert")?;
    f(conn)
}

/// Locked access to an `Arc`-wrapped background resource.
pub type ArcLock<T> = Arc<parking_lot::RwLock<T>>;
