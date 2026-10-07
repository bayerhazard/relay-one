//! Provider-op queue (local-first mutations).
//!
//! The API applies a user mutation (read/unread, move, delete) to the local
//! SQLite cache, invalidates the folder caches and returns immediately. The
//! actual IMAP mutation is enqueued here and replayed by the scheduler's
//! `run_provider_ops` worker on the user connection slot, with bounded
//! retries. This keeps the interactive path free of IMAP round-trips
//! (SELECT/COPY/STORE) that previously blocked browser requests for seconds
//! on large mailboxes.

use rusqlite::{params, Connection};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderOpKind {
    Flag,
    Move,
    Delete,
}

impl ProviderOpKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderOpKind::Flag => "flag",
            ProviderOpKind::Move => "move",
            ProviderOpKind::Delete => "delete",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderOpRow {
    pub id: i64,
    pub account_id: i64,
    pub kind: String,
    pub uid: i64,
    pub folder: String,
    pub target_folder: Option<String>,
    pub flag: Option<String>,
    pub set_flag: bool,
    pub state: String,
    pub attempts: i32,
}

/// Enqueue a flag mutation (UID STORE +/-FLAGS (flag)).
pub fn enqueue_flag(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    folder: &str,
    flag: &str,
    set: bool,
) -> Result<i64, rusqlite::Error> {
    conn.execute(
        "INSERT INTO provider_ops (account_id, kind, uid, folder, flag, set_flag)
         VALUES (?1, 'flag', ?2, ?3, ?4, ?5)",
        params![account_id, uid, folder, flag, set as i64],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Enqueue a move mutation (UID COPY to target + STORE \Deleted on source).
pub fn enqueue_move(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    source: &str,
    target: &str,
) -> Result<i64, rusqlite::Error> {
    conn.execute(
        "INSERT INTO provider_ops (account_id, kind, uid, folder, target_folder)
         VALUES (?1, 'move', ?2, ?3, ?4)",
        params![account_id, uid, source, target],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Enqueue a delete mutation (STORE \Deleted on folder; EXPUNGE batched).
pub fn enqueue_delete(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    folder: &str,
) -> Result<i64, rusqlite::Error> {
    conn.execute(
        "INSERT INTO provider_ops (account_id, kind, uid, folder)
         VALUES (?1, 'delete', ?2, ?3)",
        params![account_id, uid, folder],
    )?;
    Ok(conn.last_insert_rowid())
}

/// The Message-ID header of the row (account, folder, uid), if it has one.
/// Taken before the row moves locally.
pub fn kopf_von(conn: &Connection, account_id: i64, uid: i64, folder: &str) -> Option<String> {
    conn.query_row(
        "SELECT m.message_id FROM messages m JOIN folders f ON m.folder_id = f.id
         WHERE m.account_id = ?1 AND m.uid = ?2 AND f.name = ?3 LIMIT 1",
        params![account_id, uid, folder],
        |row| row.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
    .filter(|v| !v.trim().is_empty())
}

/// Enqueue a move on the provider (Kai, 7.10.2026). With a Message-ID the
/// worker finds the mail in `source` by that header ("move_mid"): after any
/// local move a row carries the number of the folder it came from until
/// the next sync, and a number sent to the provider in that window would
/// miss the mail or hit another one. Without one, by number as before.
/// The Message-ID rides in the `flag` column.
pub fn enqueue_verschieben(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    source: &str,
    target: &str,
    kopf: Option<&str>,
) -> Result<i64, rusqlite::Error> {
    match kopf {
        Some(k) => {
            conn.execute(
                "INSERT INTO provider_ops (account_id, kind, uid, folder, target_folder, flag)
                 VALUES (?1, 'move_mid', ?2, ?3, ?4, ?5)",
                params![account_id, uid, source, target, k],
            )?;
            Ok(conn.last_insert_rowid())
        }
        None => enqueue_move(conn, account_id, uid, source, target),
    }
}

/// Enqueue a delete on the provider, by Message-ID where there is one (see
/// `enqueue_verschieben`).
pub fn enqueue_loeschen(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    folder: &str,
    kopf: Option<&str>,
) -> Result<i64, rusqlite::Error> {
    match kopf {
        Some(k) => {
            conn.execute(
                "INSERT INTO provider_ops (account_id, kind, uid, folder, flag)
                 VALUES (?1, 'delete_mid', ?2, ?3, ?4)",
                params![account_id, uid, folder, k],
            )?;
            Ok(conn.last_insert_rowid())
        }
        None => enqueue_delete(conn, account_id, uid, folder),
    }
}

/// Oldest pending ops for one account (FIFO, bounded).
pub fn take_pending_for_account(
    conn: &Connection,
    account_id: i64,
    limit: i64,
) -> Result<Vec<ProviderOpRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, kind, uid, folder, target_folder, flag, set_flag, state, attempts
         FROM provider_ops
         WHERE account_id = ?1 AND state IN ('pending', 'failed') AND attempts < 5
         ORDER BY id ASC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![account_id, limit], map_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn map_row(row: &rusqlite::Row) -> rusqlite::Result<ProviderOpRow> {
    Ok(ProviderOpRow {
        id: row.get(0)?,
        account_id: row.get(1)?,
        kind: row.get(2)?,
        uid: row.get(3)?,
        folder: row.get(4)?,
        target_folder: row.get(5)?,
        flag: row.get(6)?,
        set_flag: row.get::<_, i64>(7)? != 0,
        state: row.get(8)?,
        attempts: row.get(9)?,
    })
}

pub fn mark_done(conn: &Connection, id: i64) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE provider_ops SET state = 'done', updated_at = datetime('now') WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

pub fn mark_failed(conn: &Connection, id: i64, error: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE provider_ops SET state = 'failed', attempts = attempts + 1, last_error = ?1, updated_at = datetime('now') WHERE id = ?2",
        params![error, id],
    )?;
    Ok(())
}

/// Count of not-yet-given-up pending ops (for status/debug endpoints).
pub fn pending_count(conn: &Connection) -> Result<i64, rusqlite::Error> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM provider_ops WHERE state = 'pending'",
        [],
        |r| r.get(0),
    )?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::db;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        conn
    }

    #[test]
    fn test_enqueue_and_take_fifo() {
        let conn = setup();
        let a = enqueue_flag(&conn, 1, 10, "INBOX", "\\Seen", true).unwrap();
        let b = enqueue_move(&conn, 1, 11, "INBOX", "Archive").unwrap();
        let c = enqueue_delete(&conn, 1, 12, "INBOX").unwrap();
        let rows = take_pending_for_account(&conn, 1, 10).unwrap();
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![a, b, c]);
        assert_eq!(rows[1].kind, "move");
        assert_eq!(rows[1].target_folder.as_deref(), Some("Archive"));
        assert_eq!(rows[2].kind, "delete");
    }

    #[test]
    fn test_done_excluded_failed_retried_until_five() {
        let conn = setup();
        let a = enqueue_flag(&conn, 1, 10, "INBOX", "\\Seen", true).unwrap();
        let b = enqueue_flag(&conn, 1, 11, "INBOX", "\\Seen", true).unwrap();
        mark_done(&conn, a).unwrap();
        mark_failed(&conn, b, "boom").unwrap();
        let rows = take_pending_for_account(&conn, 1, 10).unwrap();
        assert_eq!(rows.len(), 1, "failed ops are retried, done ops are not");
        assert_eq!(rows[0].id, b);
        for _ in 0..4 {
            mark_failed(&conn, b, "boom").unwrap();
        }
        assert_eq!(take_pending_for_account(&conn, 1, 10).unwrap().len(), 0, "gives up after 5");
    }

    #[test]
    fn test_account_scoped_and_limit() {
        let conn = setup();
        for i in 1..=12 {
            enqueue_delete(&conn, 1, i, "INBOX").unwrap();
        }
        enqueue_delete(&conn, 2, 99, "INBOX").unwrap();
        let rows = take_pending_for_account(&conn, 1, 10).unwrap();
        assert_eq!(rows.len(), 10);
        assert!(rows.iter().all(|r| r.account_id == 1));
        assert_eq!(pending_count(&conn).unwrap(), 13, "take does not consume — only done/failed change state");
    }
}

/// Remember a folder for the batched EXPUNGE, across restarts.
pub fn expunge_merken(conn: &Connection, account_id: i64, folder: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT OR IGNORE INTO expunge_offen (account_id, folder) VALUES (?1, ?2)",
        params![account_id, folder],
    )?;
    Ok(())
}

/// All folders still waiting for their EXPUNGE.
pub fn expunge_liste(conn: &Connection) -> Result<Vec<(i64, String)>, rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT account_id, folder FROM expunge_offen")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

/// The folder's EXPUNGE ran.
pub fn expunge_erledigt(conn: &Connection, account_id: i64, folder: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "DELETE FROM expunge_offen WHERE account_id = ?1 AND folder = ?2",
        params![account_id, folder],
    )?;
    Ok(())
}

/// Mails the provider still has although the user deleted them: provider
/// ops and delete-queue rows that gave up after five attempts.
pub fn geloescht_gescheitert(conn: &Connection) -> Result<i64, rusqlite::Error> {
    let ops: i64 = conn.query_row(
        "SELECT COUNT(*) FROM provider_ops WHERE kind IN ('delete', 'move', 'delete_mid', 'move_mid') AND state = 'failed' AND attempts >= 5",
        [],
        |r| r.get(0),
    )?;
    let queue: i64 = conn.query_row(
        "SELECT COUNT(*) FROM delete_queue WHERE state = 'failed' AND attempts >= 5",
        [],
        |r| r.get(0),
    )?;
    Ok(ops + queue)
}

/// Try the given-up provider deletions again from the start.
pub fn geloescht_erneut(conn: &Connection) -> Result<usize, rusqlite::Error> {
    let a = conn.execute(
        "UPDATE provider_ops SET state = 'pending', attempts = 0, updated_at = datetime('now')
         WHERE kind IN ('delete', 'move', 'delete_mid', 'move_mid') AND state = 'failed'",
        [],
    )?;
    let b = conn.execute(
        "UPDATE delete_queue SET state = 'pending', attempts = 0, updated_at = datetime('now')
         WHERE state = 'failed'",
        [],
    )?;
    Ok(a + b)
}

#[cfg(test)]
mod loesch_tests {
    use super::*;

    #[test]
    fn expunge_list_survives_and_clears() {
        let conn = Connection::open_in_memory().unwrap();
        crate::cache::db::init_db(&conn).unwrap();
        expunge_merken(&conn, 1, "INBOX").unwrap();
        expunge_merken(&conn, 1, "INBOX").unwrap();
        expunge_merken(&conn, 2, "Archiv").unwrap();
        let mut l = expunge_liste(&conn).unwrap();
        l.sort();
        assert_eq!(l, vec![(1, "INBOX".to_string()), (2, "Archiv".to_string())]);
        expunge_erledigt(&conn, 1, "INBOX").unwrap();
        assert_eq!(expunge_liste(&conn).unwrap(), vec![(2, "Archiv".to_string())]);
    }

    #[test]
    fn given_up_deletes_count_and_retry() {
        let conn = Connection::open_in_memory().unwrap();
        crate::cache::db::init_db(&conn).unwrap();
        let id = enqueue_delete(&conn, 1, 7, "INBOX").unwrap();
        enqueue_loeschen(&conn, 1, 0, "Trash", Some("<a@b>")).unwrap();
        for _ in 0..5 {
            mark_failed(&conn, id, "boom").unwrap();
        }
        assert_eq!(geloescht_gescheitert(&conn).unwrap(), 1);
        assert_eq!(geloescht_erneut(&conn).unwrap(), 1);
        assert_eq!(geloescht_gescheitert(&conn).unwrap(), 0);
        assert_eq!(take_pending_for_account(&conn, 1, 10).unwrap().len(), 2);
    }
}
