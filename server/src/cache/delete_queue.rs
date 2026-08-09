//! Delete queue + verify pipeline (Concept §5).
//!
//! Only explicit user actions enqueue rows (delete, or move to a local-only
//! folder). The pipeline verifies the local archive guarantee (EML exists +
//! hash matches) BEFORE any provider deletion:
//!   - verified  → hard delete (STORE \Deleted + EXPUNGE)   [F1 hard]
//!   - unverifiable → soft (MOVE into Provider-Trash)       [F1 soft fallback]
//! Local data is NEVER deleted automatically.

use rusqlite::{params, Connection};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteState {
    Pending,
    Verified,
    Deleted,
    Failed,
}

impl DeleteState {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeleteState::Pending => "pending",
            DeleteState::Verified => "verified",
            DeleteState::Deleted => "deleted",
            DeleteState::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeleteQueueRow {
    pub id: i64,
    pub message_id: i64,
    pub account_id: i64,
    pub uid: i64,
    pub folder: String,
    pub action: String,
    pub state: String,
    pub attempts: i32,
    pub last_error: Option<String>,
}

/// Enqueue a provider-deletion request (only called by explicit user action).
pub fn enqueue(
    conn: &Connection,
    message_id: i64,
    account_id: i64,
    uid: i64,
    folder: &str,
    action: &str,
) -> Result<i64, rusqlite::Error> {
    conn.execute(
        "INSERT INTO delete_queue (message_id, account_id, uid, folder, action)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![message_id, account_id, uid, folder, action],
    )?;
    Ok(conn.last_insert_rowid())
}

/// List queue rows by state (default: pending + failed — the review list).
pub fn list_by_state(conn: &Connection, state: Option<&str>) -> Result<Vec<DeleteQueueRow>, rusqlite::Error> {
    let mut stmt = match state {
        Some(_s) => conn.prepare(
            "SELECT id, message_id, account_id, uid, folder, action, state, attempts, last_error
             FROM delete_queue WHERE state = ?1 ORDER BY created_at DESC",
        )?,
        None => conn.prepare(
            "SELECT id, message_id, account_id, uid, folder, action, state, attempts, last_error
             FROM delete_queue WHERE state IN ('pending', 'failed') ORDER BY created_at DESC",
        )?,
    };
    let rows = match state {
        Some(s) => stmt.query_map(params![s], map_row)?,
        None => stmt.query_map([], map_row)?,
    };
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn map_row(row: &rusqlite::Row) -> rusqlite::Result<DeleteQueueRow> {
    Ok(DeleteQueueRow {
        id: row.get(0)?,
        message_id: row.get(1)?,
        account_id: row.get(2)?,
        uid: row.get(3)?,
        folder: row.get(4)?,
        action: row.get(5)?,
        state: row.get(6)?,
        attempts: row.get(7)?,
        last_error: row.get(8)?,
    })
}

/// Mark a row verified (archive guarantee holds).
pub fn mark_verified(conn: &Connection, id: i64) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE delete_queue SET state = 'verified', updated_at = datetime('now') WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

/// Mark a row deleted (provider removal done).
pub fn mark_deleted(conn: &Connection, id: i64) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE delete_queue SET state = 'deleted', updated_at = datetime('now') WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

/// Mark a row failed with a message (retryable).
pub fn mark_failed(conn: &Connection, id: i64, error: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE delete_queue SET state = 'failed', attempts = attempts + 1, last_error = ?1, updated_at = datetime('now') WHERE id = ?2",
        params![error, id],
    )?;
    Ok(())
}

/// Archive verify guarantee for a message: raw EML exists and hash matches.
/// Returns (verified: bool, eml_path_abs, raw_sha256).
pub fn verify_archive_guarantee(
    conn: &Connection,
    data_root: &std::path::Path,
    message_id_row: i64,
) -> Result<(bool, Option<std::path::PathBuf>, Option<String>), String> {
    let (raw_path, raw_sha): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT raw_path, raw_sha256 FROM messages WHERE id = ?1",
            params![message_id_row],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| format!("verify: messages query: {e}"))?;
    let Some(rel) = raw_path else {
        return Ok((false, None, raw_sha));
    };
    let abs = data_root.join(&rel);
    let ok = crate::cache::archive::verify_eml(&abs, raw_sha.as_deref(), None);
    Ok((ok, Some(abs), raw_sha))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::db;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        // A minimal account + folder + message row so FK constraints hold.
        conn.execute(
            "INSERT INTO accounts (name, imap_host, smtp_host, username, password)
             VALUES ('A', 'imap.x', 'smtp.x', 'u', 'p')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO folders (account_id, name) VALUES (1, 'INBOX')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (account_id, folder_id, uid, subject, from_addr, date)
             VALUES (1, (SELECT id FROM folders LIMIT 1), 42, 'T', 'a@b', '2026-08-09')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_enqueue_and_list() {
        let conn = setup();
        let id = enqueue(&conn, 1, 1, 42, "INBOX", "delete").unwrap();
        assert!(id > 0);
        let rows = list_by_state(&conn, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, "pending");
        assert_eq!(rows[0].uid, 42);
    }

    #[test]
    fn test_state_transitions() {
        let conn = setup();
        let id = enqueue(&conn, 1, 1, 42, "INBOX", "delete").unwrap();
        mark_verified(&conn, id).unwrap();
        assert_eq!(list_by_state(&conn, Some("verified")).unwrap().len(), 1);
        mark_deleted(&conn, id).unwrap();
        assert_eq!(list_by_state(&conn, Some("deleted")).unwrap().len(), 1);
        assert_eq!(list_by_state(&conn, None).unwrap().len(), 0, "deleted not in review");
    }

    #[test]
    fn test_failed_increments_attempts() {
        let conn = setup();
        let id = enqueue(&conn, 1, 1, 42, "INBOX", "move").unwrap();
        mark_failed(&conn, id, "boom").unwrap();
        let rows = list_by_state(&conn, Some("failed")).unwrap();
        assert_eq!(rows[0].attempts, 1);
        assert_eq!(rows[0].last_error.as_deref(), Some("boom"));
        // failed rows appear in the review list
        assert_eq!(list_by_state(&conn, None).unwrap().len(), 1);
    }
}
