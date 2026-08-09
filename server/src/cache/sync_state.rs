//! Per-folder sync state (CONDSTORE modseq + last UID) — Phase 4K.
//!
//! Enables delta sync: after the first full pass, only changes since the
//! stored `highest_modseq` are fetched (when the server supports CONDSTORE),
//! falling back to UID-range polling otherwise.

use rusqlite::{params, Connection};

#[derive(Debug, Clone, Copy)]
pub struct SyncState {
    pub last_uid: i64,
    pub highest_modseq: i64,
}

/// Get sync state for an account+folder (defaults to zeros).
pub fn get(conn: &Connection, account_id: i64, folder_name: &str) -> Result<SyncState, rusqlite::Error> {
    conn.query_row(
        "SELECT last_uid, highest_modseq FROM sync_state WHERE account_id = ?1 AND folder_name = ?2",
        params![account_id, folder_name],
        |row| Ok(SyncState {
            last_uid: row.get(0)?,
            highest_modseq: row.get(1)?,
        }),
    )
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(SyncState { last_uid: 0, highest_modseq: 0 }),
        other => Err(other),
    })
}

/// Upsert sync state after a sync pass.
pub fn set(
    conn: &Connection,
    account_id: i64,
    folder_name: &str,
    last_uid: i64,
    highest_modseq: i64,
) -> Result<(), rusqlite::Error> {
    let folder_id: i64 = match conn.query_row(
        "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
        params![account_id, folder_name],
        |row| row.get(0),
    ) {
        Ok(id) => id,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            conn.execute(
                "INSERT OR IGNORE INTO folders (account_id, name) VALUES (?1, ?2)",
                params![account_id, folder_name],
            )?;
            conn.query_row(
                "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
                params![account_id, folder_name],
                |row| row.get(0),
            )?
        }
        Err(e) => return Err(e),
    };
    conn.execute(
        "INSERT INTO sync_state (folder_id, account_id, folder_name, last_uid, highest_modseq, last_sync_at)
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
         ON CONFLICT(folder_id) DO UPDATE SET
           last_uid = excluded.last_uid,
           highest_modseq = excluded.highest_modseq,
           last_sync_at = datetime('now')",
        params![folder_id, account_id, folder_name, last_uid, highest_modseq],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::db;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (name, imap_host, smtp_host, username, password) VALUES ('A','x','x','u','p')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_default_zero() {
        let conn = setup();
        let s = get(&conn, 1, "INBOX").unwrap();
        assert_eq!(s.last_uid, 0);
        assert_eq!(s.highest_modseq, 0);
    }

    #[test]
    fn test_set_and_get() {
        let conn = setup();
        set(&conn, 1, "INBOX", 42, 12345).unwrap();
        let s = get(&conn, 1, "INBOX").unwrap();
        assert_eq!(s.last_uid, 42);
        assert_eq!(s.highest_modseq, 12345);
    }

    #[test]
    fn test_upsert_updates() {
        let conn = setup();
        set(&conn, 1, "INBOX", 10, 100).unwrap();
        set(&conn, 1, "INBOX", 20, 200).unwrap();
        let s = get(&conn, 1, "INBOX").unwrap();
        assert_eq!(s.last_uid, 20);
        assert_eq!(s.highest_modseq, 200);
    }
}
