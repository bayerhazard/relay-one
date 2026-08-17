use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::imap::types::CachedMessage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRecord {
    pub id: i64,
    pub account_id: i64,
    pub uid: i64,
    pub message_id: Option<String>,
    pub subject: Option<String>,
    pub from_addr: Option<String>,
    pub to_addr: Option<String>,
    pub cc_addr: Option<String>,
    pub date: Option<String>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub flags: Option<String>,
    pub ai_summary: Option<String>,
    pub ai_priority: Option<f32>,
    pub ai_fraud_score: Option<f32>,
    pub is_read: bool,
    pub is_flagged: bool,
    pub synced: bool,
    pub has_attachments: bool,
}

pub fn save_message(conn: &Connection, account_id: i64, msg: &CachedMessage, folder_name: &str) -> Result<(), rusqlite::Error> {
    conn.execute("SAVEPOINT save_message", [])?;
    let result = save_message_inner(conn, account_id, msg, folder_name);
    if result.is_ok() {
        conn.execute("RELEASE SAVEPOINT save_message", [])?;
    } else {
        let err_text = result.as_ref().err().map(|e| e.to_string()).unwrap_or_default();
        tracing::warn!(
            "save_message FEHLER (account {}, folder '{}', uid {}): {}",
            account_id, folder_name, msg.uid, err_text
        );
        conn.execute("ROLLBACK TO SAVEPOINT save_message", [])?;
    }
    result
}

fn save_message_inner(conn: &Connection, account_id: i64, msg: &CachedMessage, folder_name: &str) -> Result<(), rusqlite::Error> {
    let folder_id: i64 = match conn.query_row(
        "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
        params![account_id, folder_name],
        |row| row.get(0),
    ) {
        Ok(id) => id,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            conn.execute(
                "INSERT INTO folders (account_id, name) VALUES (?1, ?2)",
                params![account_id, folder_name],
            )?;
            conn.last_insert_rowid()
        }
        Err(e) => return Err(e),
    };

    // Re-fetch guard: if this message_id already lives in the LOCAL Trash
    // folder (user deleted it; the provider copy may still linger), do NOT
    // re-insert it into another folder. Without this the sync would
    // "revive" deleted mails in INBOX while their row stays in Trash — and
    // the next delete would hit UNIQUE(account_id, folder_id, uid).
    if !msg.envelope.message_id.is_empty() {
        let already_in_trash: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM messages m
                    JOIN folders f ON f.id = m.folder_id
                    WHERE m.account_id = ?1 AND m.message_id = ?2
                      AND f.local_only = 1 AND f.name = 'Trash'
                )",
                params![account_id, msg.envelope.message_id],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if already_in_trash {
            return Ok(());
        }
    }

    conn.execute(
        "INSERT INTO messages (
            account_id, folder_id, uid, message_id, subject, from_addr, to_addr, cc_addr, date,
            body_text, body_html, flags, ai_summary, ai_priority, ai_fraud_score,
            is_read, is_flagged, has_attachments, synced
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, 1)
ON CONFLICT(account_id, folder_id, uid) DO UPDATE SET
            folder_id = excluded.folder_id,
            subject = excluded.subject,
            from_addr = excluded.from_addr,
            to_addr = excluded.to_addr,
            cc_addr = excluded.cc_addr,
            date = excluded.date,
            body_text = COALESCE(excluded.body_text, messages.body_text),
            body_html = COALESCE(excluded.body_html, messages.body_html),
            flags = excluded.flags,
            ai_summary = COALESCE(excluded.ai_summary, messages.ai_summary),
            ai_priority = COALESCE(excluded.ai_priority, messages.ai_priority),
            ai_fraud_score = COALESCE(excluded.ai_fraud_score, messages.ai_fraud_score),
            is_read = excluded.is_read,
            is_flagged = excluded.is_flagged,
            has_attachments = excluded.has_attachments,
            synced = 1,
            updated_at = datetime('now')",
        params![
            account_id,
            folder_id,
            msg.uid,
            msg.envelope.message_id,
            msg.envelope.subject,
            msg.envelope.from,
            msg.envelope.to,
            msg.envelope.cc,
            msg.envelope.date,
            msg.body_preview,
            msg.body_structure,
            serde_json::to_string(&msg.flags).unwrap_or_default(),
            msg.ai_summary,
            msg.ai_priority,
            msg.ai_fraud_score,
            msg.is_read as i32,
            msg.is_flagged as i32,
            msg.has_attachments as i32,
        ],
    )?;
    
    // Save attachment metadata. Reconcile on (message_id, part_index) instead
    // of the historical DELETE + re-INSERT: ids stay stable across re-syncs and
    // stale rows are removed even when the fresh BODYSTRUCTURE list is empty.
    {
        // Get the message ID — scoped by folder: uid is only unique per
        // folder (IMAP vs local-only rows like "Entwürfe"/"Mama und Papa"
        // can share a uid). An unscoped lookup would attach metadata to the
        // wrong message when uids collide across folders.
        let message_id: i64 = conn.query_row(
            "SELECT id FROM messages WHERE account_id = ?1 AND uid = ?2 AND folder_id = ?3",
            params![account_id, msg.uid, folder_id],
            |row| row.get(0),
        )?;
        crate::cache::attachments::reconcile_attachments(conn, message_id, &msg.attachments)?;
    }
    
    Ok(())
}

pub fn fetch_inbox(
    conn: &Connection,
    account_id: i64,
    limit: Option<i64>,
    offset: Option<i64>,
    folder: &str,
) -> Result<Vec<MessageRecord>, rusqlite::Error> {
    fetch_inbox_impl(conn, account_id, limit, offset, folder, false)
}

/// Meta-only variant of `fetch_inbox`: omits `body_text`/`body_html` from the
/// SELECT so large folders (up to 10k rows) transfer as lightweight JSON. The
/// body preview is computed server-side from a `substr()` over the stored
/// column, avoiding the full-body I/O that made folder switches take seconds.
pub fn fetch_inbox_meta(
    conn: &Connection,
    account_id: i64,
    limit: Option<i64>,
    offset: Option<i64>,
    folder: &str,
) -> Result<Vec<MessageRecord>, rusqlite::Error> {
    fetch_inbox_impl(conn, account_id, limit, offset, folder, true)
}

fn fetch_inbox_impl(
    conn: &Connection,
    account_id: i64,
    limit: Option<i64>,
    offset: Option<i64>,
    folder: &str,
    list_only: bool,
) -> Result<Vec<MessageRecord>, rusqlite::Error> {
    let limit = limit.unwrap_or(50);
    let offset = offset.unwrap_or(0);

    // NOTE: Spam folders were previously excluded entirely ("return empty").
    // The user wants Spam handled like every other folder — shown from the
    // local cache.

    // Lookup or create folder
    let folder_id: i64 = match conn.query_row(
        "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
        params![account_id, folder],
        |row| row.get(0),
    ) {
        Ok(id) => id,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            conn.execute(
                "INSERT INTO folders (account_id, name) VALUES (?1, ?2)",
                params![account_id, folder],
            )?;
            conn.last_insert_rowid()
        }
        Err(e) => return Err(e),
    };

    // In list_only mode select a server-side preview instead of the full body
    // columns. The MessageRecord still carries `body_text` (the preview) so the
    // existing `body_preview()` serializer keeps working unchanged.
    let body_cols = if list_only {
        "substr(body_text, 1, 200), NULL"
    } else {
        "body_text, body_html"
    };
    let sql = format!(
        "SELECT id, account_id, uid, message_id, subject, from_addr, to_addr, cc_addr, date,
                {body_cols}, flags, ai_summary, ai_priority, ai_fraud_score,
                is_read, is_flagged, synced, has_attachments
         FROM messages
         WHERE account_id = ?1 AND folder_id = ?2
           AND (flags NOT LIKE '%\\\\Deleted%' OR flags IS NULL)
         ORDER BY date DESC
         LIMIT ?3 OFFSET ?4"
    );
    let mut stmt = conn.prepare(&sql)?;

    let rows = stmt.query_map(params![account_id, folder_id, limit, offset], |row| {
        Ok(MessageRecord {
            id: row.get(0)?,
            account_id: row.get(1)?,
            uid: row.get(2)?,
            message_id: row.get(3)?,
            subject: row.get(4)?,
            from_addr: row.get(5)?,
            to_addr: row.get(6)?,
            cc_addr: row.get(7)?,
            date: row.get(8)?,
            body_text: row.get(9)?,
            body_html: row.get(10)?,
            flags: row.get(11)?,
            ai_summary: row.get(12)?,
            ai_priority: row.get(13)?,
            ai_fraud_score: row.get(14)?,
            is_read: row.get::<_, i32>(15)? != 0,
            is_flagged: row.get::<_, i32>(16)? != 0,
            synced: row.get::<_, i32>(17)? != 0,
            has_attachments: row.get::<_, i32>(18)? != 0,
        })
    })?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn fetch_message_body(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    folder_id: Option<i64>,
) -> Result<Option<MessageRecord>, rusqlite::Error> {
    let result = match folder_id {
        Some(fid) => conn.query_row(
            "SELECT id, account_id, uid, message_id, subject, from_addr, to_addr, cc_addr, date,
                    body_text, body_html, flags, ai_summary, ai_priority, ai_fraud_score,
                    is_read, is_flagged, synced, has_attachments
             FROM messages
             WHERE account_id = ?1 AND uid = ?2 AND folder_id = ?3
               AND (flags NOT LIKE '%\\\\Deleted%' OR flags IS NULL)",
            params![account_id, uid, fid],
            |row| row_to_message_record(row),
        ),
        None => conn.query_row(
            "SELECT id, account_id, uid, message_id, subject, from_addr, to_addr, cc_addr, date,
                    body_text, body_html, flags, ai_summary, ai_priority, ai_fraud_score,
                    is_read, is_flagged, synced, has_attachments
             FROM messages
             WHERE account_id = ?1 AND uid = ?2
               AND (flags NOT LIKE '%\\\\Deleted%' OR flags IS NULL)
             LIMIT 1",
            params![account_id, uid],
            |row| row_to_message_record(row),
        ),
    };
    match result {
        Ok(record) => Ok(Some(record)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

fn row_to_message_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageRecord> {
    Ok(MessageRecord {
        id: row.get(0)?,
        account_id: row.get(1)?,
        uid: row.get(2)?,
        message_id: row.get(3)?,
        subject: row.get(4)?,
        from_addr: row.get(5)?,
        to_addr: row.get(6)?,
        cc_addr: row.get(7)?,
        date: row.get(8)?,
        body_text: row.get(9)?,
        body_html: row.get(10)?,
        flags: row.get(11)?,
        ai_summary: row.get(12)?,
        ai_priority: row.get(13)?,
        ai_fraud_score: row.get(14)?,
        is_read: row.get::<_, i32>(15)? != 0,
        is_flagged: row.get::<_, i32>(16)? != 0,
        synced: row.get::<_, i32>(17)? != 0,
        has_attachments: row.get::<_, i32>(18)? != 0,
    })
}

/// Fetch message body along with its folder name for IMAP fallback.
pub fn fetch_message_body_with_folder(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    folder: Option<&str>,
) -> Result<Option<(MessageRecord, String)>, rusqlite::Error> {
    const COLS: &str = "m.id, m.account_id, m.uid, m.message_id, m.subject, m.from_addr, m.to_addr, m.cc_addr, m.date,
                m.body_text, m.body_html, m.flags, m.ai_summary, m.ai_priority, m.ai_fraud_score,
                m.is_read, m.is_flagged, m.synced, m.has_attachments, f.name";
    let result = match folder {
        Some(f) => conn.query_row(
            &format!(
                "SELECT {COLS}
                 FROM messages m
                 JOIN folders f ON m.folder_id = f.id
                 WHERE m.account_id = ?1 AND m.uid = ?2 AND f.name = ?3
                   AND (m.flags NOT LIKE '%\\\\Deleted%' OR m.flags IS NULL)
                 LIMIT 1"
            ),
            params![account_id, uid, f],
            |row| Ok((message_record_from_row(row)?, row.get::<_, String>(19)?)),
        ),
        None => conn.query_row(
            &format!(
                "SELECT {COLS}
                 FROM messages m
                 JOIN folders f ON m.folder_id = f.id
                 WHERE m.account_id = ?1 AND m.uid = ?2
                   AND (m.flags NOT LIKE '%\\\\Deleted%' OR m.flags IS NULL)
                 ORDER BY f.name = 'Entwürfe' ASC, m.id ASC
                 LIMIT 1"
            ),
            params![account_id, uid],
            |row| Ok((message_record_from_row(row)?, row.get::<_, String>(19)?)),
        ),
    };
    match result {
        Ok(record) => Ok(Some(record)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Map a row produced by `fetch_message_body_with_folder` (columns match COLS)
/// into a MessageRecord.
fn message_record_from_row(row: &rusqlite::Row<'_>) -> Result<MessageRecord, rusqlite::Error> {
    Ok(MessageRecord {
        id: row.get(0)?,
        account_id: row.get(1)?,
        uid: row.get(2)?,
        message_id: row.get(3)?,
        subject: row.get(4)?,
        from_addr: row.get(5)?,
        to_addr: row.get(6)?,
        cc_addr: row.get(7)?,
        date: row.get(8)?,
        body_text: row.get(9)?,
        body_html: row.get(10)?,
        flags: row.get(11)?,
        ai_summary: row.get(12)?,
        ai_priority: row.get(13)?,
        ai_fraud_score: row.get(14)?,
        is_read: row.get::<_, i32>(15)? != 0,
        is_flagged: row.get::<_, i32>(16)? != 0,
        synced: row.get::<_, i32>(17)? != 0,
        has_attachments: row.get::<_, i32>(18)? != 0,
    })
}

/// Check if a folder name corresponds to a spam/junk folder.
pub fn is_spam_folder(folder: &str) -> bool {
    let spam_names = ["Spam", "Junk", "Spamverdacht", "Junk E-Mail", "spam", "junk"];
    spam_names.iter().any(|name| folder.eq_ignore_ascii_case(name))
}

pub fn mark_as_read(conn: &Connection, account_id: i64, uid: i64) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE messages SET is_read = 1, updated_at = datetime('now') WHERE account_id = ?1 AND uid = ?2",
        params![account_id, uid],
    )?;
    Ok(())
}

/// Mark a message read, scoped to an optional folder_id. UID is only unique per
/// folder (local-only folders reuse uids), so an unscoped UPDATE could hit a row
/// in the wrong folder when the same uid exists in multiple folders.
pub fn folder_id_for_name(conn: &Connection, account_id: i64, folder_name: &str) -> Result<i64, rusqlite::Error> {
    conn.query_row(
        "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
        params![account_id, folder_name],
        |row| row.get(0),
    )
}

pub fn mark_as_read_in_folder(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    folder_id: Option<i64>,
) -> Result<(), rusqlite::Error> {
    match folder_id {
        Some(fid) => {
            let _ = conn.execute(
                "UPDATE messages SET is_read = 1, updated_at = datetime('now')
                 WHERE account_id = ?1 AND uid = ?2 AND folder_id = ?3",
                params![account_id, uid, fid],
            )?;
        }
        None => {
            let _ = conn.execute(
                "UPDATE messages SET is_read = 1, updated_at = datetime('now') WHERE account_id = ?1 AND uid = ?2",
                params![account_id, uid],
            )?;
        }
    }
    Ok(())
}

/// Mark a message unseen, scoped to an optional folder_id (see
/// `mark_as_read_in_folder` for the UID-collision rationale).
pub fn mark_as_unseen_in_folder(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    folder_id: Option<i64>,
) -> Result<(), rusqlite::Error> {
    match folder_id {
        Some(fid) => {
            let _ = conn.execute(
                "UPDATE messages SET is_read = 0, updated_at = datetime('now')
                 WHERE account_id = ?1 AND uid = ?2 AND folder_id = ?3",
                params![account_id, uid, fid],
            )?;
        }
        None => {
            let _ = conn.execute(
                "UPDATE messages SET is_read = 0, updated_at = datetime('now') WHERE account_id = ?1 AND uid = ?2",
                params![account_id, uid],
            )?;
        }
    }
    Ok(())
}

/// Update the is_read flag for a message (used by flag refresh from IMAP server).
pub fn update_is_read(conn: &Connection, account_id: i64, uid: i64, is_read: bool) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE messages SET is_read = ?, updated_at = datetime('now') WHERE account_id = ?1 AND uid = ?2",
        params![is_read as i32, account_id, uid],
    )?;
    Ok(())
}

/// IMAP flag refresh variant: only downgrades read → unread when the local row
/// was NOT touched within the cooldown window. Otherwise a message the user
/// just read locally would be flipped back by a stale server flag (the \Seen
/// round-trip can lag). Upgrades (unread → read) are always applied.
pub fn update_is_read_guarded(conn: &Connection, account_id: i64, uid: i64, is_read: bool) -> Result<(), rusqlite::Error> {
    let sql = if is_read {
        "UPDATE messages SET is_read = 1, updated_at = datetime('now') WHERE account_id = ?1 AND uid = ?2"
    } else {
        "UPDATE messages SET is_read = 0, updated_at = datetime('now')
         WHERE account_id = ?1 AND uid = ?2
           AND (updated_at IS NULL OR updated_at <= datetime('now', '-30 seconds'))"
    };
    conn.execute(sql, params![account_id, uid])?;
    Ok(())
}

/// Update the is_flagged flag for a message (used by flag refresh from IMAP server).
pub fn update_is_flagged(conn: &Connection, account_id: i64, uid: i64, is_flagged: bool) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE messages SET is_flagged = ?1, updated_at = datetime('now') WHERE account_id = ?2 AND uid = ?3",
        params![is_flagged as i32, account_id, uid],
    )?;
    Ok(())
}

/// Toggle the \Flagged star, scoped to an optional folder_id. UID is only unique
/// per folder, so an unscoped UPDATE could flip the wrong row when the same uid
/// exists in multiple folders.
pub fn update_is_flagged_in_folder(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    folder_id: Option<i64>,
    is_flagged: bool,
) -> Result<(), rusqlite::Error> {
    match folder_id {
        Some(fid) => {
            let _ = conn.execute(
                "UPDATE messages SET is_flagged = ?1, updated_at = datetime('now')
                 WHERE account_id = ?2 AND uid = ?3 AND folder_id = ?4",
                params![is_flagged as i32, account_id, uid, fid],
            )?;
        }
        None => {
            let _ = conn.execute(
                "UPDATE messages SET is_flagged = ?1, updated_at = datetime('now') WHERE account_id = ?2 AND uid = ?3",
                params![is_flagged as i32, account_id, uid],
            )?;
        }
    }
    Ok(())
}

/// Get all UIDs for messages in a specific folder (used by flag refresh).
pub fn get_messages_with_uids_for_folder(
    conn: &Connection,
    account_id: i64,
    folder_name: &str,
) -> Result<Vec<i64>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT m.uid FROM messages m
         JOIN folders f ON m.folder_id = f.id
         WHERE m.account_id = ?1 AND f.name = ?2"
    )?;
    let uids = stmt.query_map(params![account_id, folder_name], |row| row.get(0))
        .map(|r| r.collect::<Result<Vec<_>, _>>()).unwrap_or_else(|_| Ok(Vec::new()))?;
    Ok(uids)
}

pub fn delete_message(conn: &Connection, account_id: i64, uid: i64) -> Result<(), rusqlite::Error> {
    conn.execute(
        "DELETE FROM messages WHERE account_id = ?1 AND uid = ?2",
        params![account_id, uid],
    )?;
    Ok(())
}

/// Delete only the message row inside `folder` (uid is only unique per
/// folder — an unscoped DELETE by uid would remove every row sharing the
/// uid across all folders).
pub fn delete_message_from(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    folder: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "DELETE FROM messages
         WHERE account_id = ?1 AND uid = ?2
           AND folder_id = (SELECT id FROM folders WHERE account_id = ?1 AND name = ?3)",
        params![account_id, uid, folder],
    )?;
    Ok(())
}

/// Fetches a full message record (including folder_id) for potential rollback
/// after a failed IMAP delete. Returns None if the message doesn't exist.
pub fn fetch_message_for_restore(
    conn: &Connection,
    account_id: i64,
    uid: i64,
) -> Result<Option<MessageRestoreData>, rusqlite::Error> {
    let result = conn.query_row(
        "SELECT m.folder_id, m.account_id, m.uid, m.message_id, m.subject,
                m.from_addr, m.to_addr, m.date, m.body_text, m.body_html,
                m.flags, m.ai_summary, m.ai_priority, m.ai_fraud_score,
                m.is_read, m.synced,
                f.name AS folder_name
         FROM messages m
         LEFT JOIN folders f ON f.id = m.folder_id AND f.account_id = m.account_id
         WHERE m.account_id = ?1 AND m.uid = ?2",
        params![account_id, uid],
        |row| {
            Ok(MessageRestoreData {
                folder_id: row.get(0)?,
                account_id: row.get(1)?,
                uid: row.get(2)?,
                message_id: row.get(3)?,
                subject: row.get(4)?,
                from_addr: row.get(5)?,
                to_addr: row.get(6)?,
                date: row.get(7)?,
                body_text: row.get(8)?,
                body_html: row.get(9)?,
                flags: row.get(10)?,
                ai_summary: row.get(11)?,
                ai_priority: row.get(12)?,
                ai_fraud_score: row.get(13)?,
                is_read: row.get::<_, i32>(14)? != 0,
                synced: row.get::<_, i32>(15)? != 0,
                folder_name: row.get(16)?,
            })
        },
    );
    match result {
        Ok(data) => Ok(Some(data)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Data needed to restore a deleted message in case of IMAP failure.
#[derive(Debug, Clone)]
pub struct MessageRestoreData {
    #[allow(dead_code)]
    pub folder_id: i64,
    pub account_id: i64,
    pub uid: i64,
    pub message_id: Option<String>,
    pub subject: Option<String>,
    pub from_addr: Option<String>,
    pub to_addr: Option<String>,
    pub date: Option<String>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub flags: Option<String>,
    pub ai_summary: Option<String>,
    pub ai_priority: Option<f32>,
    pub ai_fraud_score: Option<f32>,
    pub is_read: bool,
    pub synced: bool,
    pub folder_name: Option<String>,
}

/// Restores a previously deleted message into the cache.
/// Used as a rollback when cache delete succeeded but IMAP delete failed.
pub fn restore_message(
    conn: &Connection,
    data: &MessageRestoreData,
) -> Result<(), rusqlite::Error> {
    // Ensure the folder exists (it might have been the auto-created one)
    let folder_id: i64 = match conn.query_row(
        "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
        params![data.account_id, data.folder_name.as_deref().unwrap_or("INBOX")],
        |row| row.get(0),
    ) {
        Ok(id) => id,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            conn.execute(
                "INSERT INTO folders (account_id, name) VALUES (?1, ?2)",
                params![data.account_id, data.folder_name.as_deref().unwrap_or("INBOX")],
            )?;
            conn.last_insert_rowid()
        }
        Err(e) => return Err(e),
    };

    conn.execute(
        "INSERT INTO messages (
            account_id, folder_id, uid, message_id, subject, from_addr, to_addr, date,
            body_text, body_html, flags, ai_summary, ai_priority, ai_fraud_score,
            is_read, synced
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
        ON CONFLICT(account_id, folder_id, uid) DO UPDATE SET
            folder_id = excluded.folder_id,
            subject = excluded.subject,
            from_addr = excluded.from_addr,
            to_addr = excluded.to_addr,
            date = excluded.date,
            body_text = excluded.body_text,
            body_html = excluded.body_html,
            flags = excluded.flags,
            ai_summary = excluded.ai_summary,
            ai_priority = excluded.ai_priority,
            ai_fraud_score = excluded.ai_fraud_score,
            is_read = excluded.is_read,
            synced = excluded.synced,
            updated_at = datetime('now')",
        params![
            data.account_id,
            folder_id,
            data.uid,
            data.message_id,
            data.subject,
            data.from_addr,
            data.to_addr,
            data.date,
            data.body_text,
            data.body_html,
            data.flags,
            data.ai_summary,
            data.ai_priority,
            data.ai_fraud_score,
            data.is_read as i32,
            data.synced as i32,
        ],
    )?;
    Ok(())
}

/// Returns the highest known UID for a given account+folder combination.
/// Used by the sync scheduler for incremental IMAP fetching.
/// Returns 0 if no messages exist for this folder yet.
pub fn get_max_uid_for_folder(conn: &Connection, account_id: i64, folder_name: &str) -> Result<i64, rusqlite::Error> {
    conn.query_row(
        "SELECT COALESCE(MAX(m.uid), 0)
         FROM messages m
         JOIN folders f ON f.id = m.folder_id
         WHERE m.account_id = ?1 AND f.name = ?2",
        params![account_id, folder_name],
        |row| row.get(0),
    )
}

/// Delete local messages whose UIDs are NOT in the given list for a
/// specific folder. Called after a full UID SEARCH ALL to clean up
/// messages that were deleted on the IMAP server.
pub fn delete_messages_not_in(
    conn: &Connection,
    account_id: i64,
    folder_name: &str,
    uids: &[u32],
) -> Result<usize, rusqlite::Error> {
    let folder_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
            params![account_id, folder_name],
            |row| row.get(0),
        )
        .ok();

    let Some(folder_id) = folder_id else {
        return Ok(0);
    };

    if uids.is_empty() {
        let deleted = conn.execute(
            "DELETE FROM messages WHERE account_id = ?1 AND folder_id = ?2",
            params![account_id, folder_id],
        )?;
        return Ok(deleted);
    }

    let placeholders = uids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "DELETE FROM messages WHERE account_id = ?1 AND folder_id = ?2 AND uid NOT IN ({placeholders})"
    );

    let mut stmt = conn.prepare(&sql)?;
    stmt.raw_bind_parameter(1, account_id)?;
    stmt.raw_bind_parameter(2, folder_id)?;
    for (i, uid) in uids.iter().enumerate() {
        stmt.raw_bind_parameter(3 + i, *uid as i64)?;
    }
    let deleted = stmt.raw_execute()?;
    Ok(deleted)
}

pub fn update_body(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    body_text: &str,
    body_html: Option<&str>,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE messages SET body_text = ?1, body_html = ?2, updated_at = datetime('now')
         WHERE account_id = ?3 AND uid = ?4",
        params![body_text, body_html, account_id, uid],
    )?;
    Ok(())
}

/// Update body by primary key. UID is only unique per folder, so an unscoped
/// account_id+uid update could overwrite the body of a row in a different folder
/// when the same uid exists in multiple folders. The caller resolves the correct
/// row first (folder-scoped lookup) and passes its id.
pub fn update_body_by_id(
    conn: &Connection,
    id: i64,
    body_text: &str,
    body_html: Option<&str>,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE messages SET body_text = ?1, body_html = ?2, updated_at = datetime('now')
         WHERE id = ?3",
        params![body_text, body_html, id],
    )?;
    Ok(())
}

/// Update body + archive path (raw EML file location) for a message.
pub fn update_body_with_raw(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    folder_id: Option<i64>,
    body_text: &str,
    body_html: Option<&str>,
    raw_path: Option<&str>,
    raw_sha256: Option<&str>,
) -> Result<(), rusqlite::Error> {
    match folder_id {
        Some(fid) => conn.execute(
            "UPDATE messages SET body_text = ?1, body_html = ?2, raw_path = COALESCE(?3, raw_path),
                    raw_sha256 = COALESCE(?4, raw_sha256), updated_at = datetime('now')
             WHERE account_id = ?5 AND uid = ?6 AND folder_id = ?7",
            params![body_text, body_html, raw_path, raw_sha256, account_id, uid, fid],
        )?,
        None => conn.execute(
            "UPDATE messages SET body_text = ?1, body_html = ?2, raw_path = COALESCE(?3, raw_path),
                    raw_sha256 = COALESCE(?4, raw_sha256), updated_at = datetime('now')
             WHERE account_id = ?5 AND uid = ?6",
            params![body_text, body_html, raw_path, raw_sha256, account_id, uid],
        )?,
    };
    Ok(())
}

pub fn update_ai_summary(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    folder_id: Option<i64>,
    summary: &str,
) -> Result<(), rusqlite::Error> {
    match folder_id {
        Some(fid) => {
            conn.execute(
                "UPDATE messages SET ai_summary = ?1, updated_at = datetime('now')
                 WHERE account_id = ?2 AND uid = ?3 AND folder_id = ?4",
                params![summary, account_id, uid, fid],
            )?;
        }
        None => {
            conn.execute(
                "UPDATE messages SET ai_summary = ?1, updated_at = datetime('now') WHERE account_id = ?2 AND uid = ?3",
                params![summary, account_id, uid],
            )?;
        }
    }
    Ok(())
}

pub fn update_ai_priority(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    folder_id: Option<i64>,
    priority: f32,
) -> Result<(), rusqlite::Error> {
    match folder_id {
        Some(fid) => {
            conn.execute(
                "UPDATE messages SET ai_priority = ?1, updated_at = datetime('now')
                 WHERE account_id = ?2 AND uid = ?3 AND folder_id = ?4",
                params![priority, account_id, uid, fid],
            )?;
        }
        None => {
            conn.execute(
                "UPDATE messages SET ai_priority = ?1, updated_at = datetime('now') WHERE account_id = ?2 AND uid = ?3",
                params![priority, account_id, uid],
            )?;
        }
    }
    Ok(())
}

#[allow(dead_code)]
pub fn update_ai_fraud(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    score: f32,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE messages SET ai_fraud_score = ?1, updated_at = datetime('now') WHERE account_id = ?2 AND uid = ?3",
        params![score, account_id, uid],
    )?;
    Ok(())
}

/// Count unread messages in the INBOX folder for a given account.
pub fn count_unread_inbox(conn: &Connection, account_id: i64) -> Result<u32, rusqlite::Error> {
    conn.query_row(
        "SELECT COUNT(*) FROM messages 
         WHERE account_id = ?1 
           AND folder_id = (SELECT id FROM folders WHERE account_id = ?1 AND name = 'INBOX')
           AND is_read = 0",
        params![account_id],
        |row| row.get(0),
    )
}

pub fn update_folder(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    folder: &str,
) -> Result<(), rusqlite::Error> {
    // Ensure the target folder exists — otherwise the subquery below yields
    // NULL and the row is silently orphaned (invisible in every folder).
    // Local folders (Trash, migration targets) are created as local-only so
    // the sync never prunes them.
    create_local_folder(conn, account_id, folder)?;
    conn.execute(
        "UPDATE messages SET folder_id = (SELECT id FROM folders WHERE account_id = ?1 AND name = ?2), updated_at = datetime('now') WHERE account_id = ?3 AND uid = ?4",
        params![account_id, folder, account_id, uid],
    )?;
    Ok(())
}

/// Move the message row into `folder`, scoped to ONE source folder.
/// UIDs are only unique per folder — without the source scope an UPDATE by
/// uid alone would move EVERY message that shares the uid across all folders.
///
/// Conflict handling for the target folder:
///   1. If a target row with the SAME message_id already exists (e.g. the
///      mail was previously deleted into Trash and the sync re-fetched it
///      into INBOX), DELETE the source row — the target copy wins. This is
///      the "Mail wiederaufersteht" case: the second delete must not hit
///      UNIQUE(account_id, folder_id, uid).
///   2. Otherwise, if the uid collides in the target folder, shift the uid
///      by +100000 increments until free (keeps the row visible).
pub fn update_folder_from(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    source_folder: &str,
    folder: &str,
) -> Result<(), rusqlite::Error> {
    create_local_folder(conn, account_id, folder)?;

    let source_folder_id: i64 = conn
        .query_row(
            "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
            params![account_id, source_folder],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let target_folder_id: i64 = conn
        .query_row(
            "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
            params![account_id, folder],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if source_folder_id == 0 || target_folder_id == 0 {
        return Ok(());
    }

    // Source row (id + message_id) in the source folder.
    let source = conn
        .query_row(
            "SELECT id, message_id FROM messages
             WHERE account_id = ?1 AND folder_id = ?2 AND uid = ?3",
            params![account_id, source_folder_id, uid],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .ok();
    let Some((source_id, source_message_id)) = source else {
        return Ok(()); // nothing to move
    };

    // Case 1: same message_id already in the target folder → the target copy
    // wins; drop the stale source duplicate.
    if let Some(mid) = source_message_id {
        let duplicate_in_target: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM messages
                 WHERE account_id = ?1 AND folder_id = ?2 AND message_id = ?3)",
                params![account_id, target_folder_id, mid],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if duplicate_in_target {
            conn.execute(
                "DELETE FROM messages WHERE id = ?1",
                params![source_id],
            )?;
            return Ok(());
        }
    }

    // Case 2: find a free uid in the target folder (shift on collision).
    let mut new_uid = uid;
    loop {
        let occupied: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM messages
                 WHERE account_id = ?1 AND folder_id = ?2 AND uid = ?3)",
                params![account_id, target_folder_id, new_uid],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if !occupied {
            conn.execute(
                "UPDATE messages SET folder_id = ?1, uid = ?2, updated_at = datetime('now')
                 WHERE id = ?3",
                params![target_folder_id, new_uid, source_id],
            )?;
            return Ok(());
        }
        new_uid += 100000;
        if new_uid > uid + 10_000_000 {
            return Err(rusqlite::Error::InvalidParameterName(
                "update_folder_from: keine freie uid im Zielordner".into(),
            ));
        }
    }
}

/// Create a local-only folder (no IMAP counterpart). Idempotent.
///
/// NOTE: this must NEVER convert an existing folder to local_only. The old
/// UPDATE-by-name behavior turned real IMAP folders (INBOX!) into local-only
/// folders when the name collided — the sync then silently stopped fetching
/// them. Existing rows are left untouched; the caller's subquery resolves
/// the folder id either way.
pub fn create_local_folder(
    conn: &Connection,
    account_id: i64,
    name: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT OR IGNORE INTO folders (account_id, name, imap_id, local_only) VALUES (?1, ?2, NULL, 1)",
        params![account_id, name],
    )?;
    Ok(())
}

/// Is a folder local-only (no IMAP counterpart)?
pub fn is_local_only_folder(conn: &Connection, account_id: i64, name: &str) -> Result<bool, rusqlite::Error> {
    let local: i32 = conn
        .query_row(
            "SELECT local_only FROM folders WHERE account_id = ?1 AND name = ?2",
            params![account_id, name],
            |row| row.get(0),
        )
        .unwrap_or(0);
    Ok(local == 1)
}

/// List all folders (incl. local-only) with their local_only flag.
pub fn list_all_folders(conn: &Connection, account_id: i64) -> Result<Vec<(String, bool)>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT name, local_only FROM folders WHERE account_id = ?1 ORDER BY local_only ASC, name ASC",
    )?;
    let rows = stmt.query_map(params![account_id], |row| Ok((row.get(0)?, row.get::<_, i32>(1)? != 0)))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Rename a local-only folder locally (no IMAP involvement).
pub fn rename_local_folder(
    conn: &Connection,
    account_id: i64,
    old_name: &str,
    new_name: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE folders SET name = ?1 WHERE account_id = ?2 AND name = ?3 AND local_only = 1",
        params![new_name, account_id, old_name],
    )?;
    conn.execute(
        "UPDATE messages SET folder_id = (SELECT id FROM folders WHERE account_id = ?1 AND name = ?2)
         WHERE account_id = ?3 AND folder_id = (SELECT id FROM folders WHERE account_id = ?3 AND name = ?4)",
        params![account_id, new_name, account_id, old_name],
    )?;
    Ok(())
}

/// Delete a LOCAL-only folder and all its messages (index rows + EML files).
/// Returns the number of removed message rows.
pub fn delete_local_folder(
    conn: &Connection,
    data_root: &std::path::Path,
    account_id: i64,
    name: &str,
) -> Result<usize, String> {
    // Matches ANY folder with this name (local-only OR IMAP mirror) — used by
    // the migration cleanup to purge polluted target folders.
    let folder_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
            params![account_id, name],
            |row| row.get(0),
        )
        .ok();
    let Some(folder_id) = folder_id else {
        return Err(format!("Ordner '{}' nicht gefunden", name));
    };

    // Remove EML archive files first.
    let raws: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT raw_path FROM messages WHERE folder_id = ?1 AND raw_path IS NOT NULL")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![folder_id], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            if let Ok(p) = r {
                out.push(p);
            }
        }
        out
    };
    for rel in &raws {
        let abs = data_root.join(rel);
        let _ = std::fs::remove_file(&abs);
    }

    let deleted = conn
        .execute("DELETE FROM messages WHERE folder_id = ?1", params![folder_id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM folders WHERE id = ?1", params![folder_id])
        .map_err(|e| e.to_string())?;
    Ok(deleted)
}

/// Build a safe FTS5 MATCH expression from free user input.
/// Each whitespace-separated term becomes a quoted prefix query, ANDed
/// together. Quoting neutralises FTS5 operators so arbitrary input cannot
/// break the query or inject syntax. Returns None if there is no usable term.
fn build_fts_query(raw: &str) -> Option<String> {
    let terms: Vec<String> = raw
        .split_whitespace()
        .filter_map(|t| {
            // `is:flagged` is a search operator, not an FTS term.
            if t.eq_ignore_ascii_case("is:flagged") || t.eq_ignore_ascii_case("is:flag") {
                return None;
            }
            // Strip characters that are meaningless inside a quoted token and
            // escape embedded double quotes per FTS5 rules ("" = literal ").
            let cleaned = t.replace('"', "\"\"");
            let cleaned = cleaned.trim();
            if cleaned.is_empty() {
                None
            } else {
                Some(format!("\"{}\"*", cleaned))
            }
        })
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" AND "))
    }
}

/// Does the query contain the `is:flagged` operator?
fn query_has_flag_operator(raw: &str) -> bool {
    raw.split_whitespace()
        .any(|t| t.eq_ignore_ascii_case("is:flagged") || t.eq_ignore_ascii_case("is:flag"))
}

/// Full-text search over cached messages (subject, sender, recipient, body)
/// for one account, newest first. Returns up to `limit` results.
pub fn search_messages(
    conn: &Connection,
    account_id: i64,
    query: &str,
    limit: i64,
) -> Result<Vec<MessageRecord>, rusqlite::Error> {
    // `is:flagged` is a metadata filter, not an FTS term: it must be applied
    // even when the rest of the query has no text terms (e.g. "is:flagged"
    // alone should return all flagged mail).
    let flag_only = query_has_flag_operator(query);
    let fts_terms = build_fts_query(query);

    if fts_terms.is_none() && !flag_only {
        return Ok(Vec::new());
    }

    let mut sql = String::from(
        "SELECT m.id, m.account_id, m.uid, m.message_id, m.subject, m.from_addr, m.to_addr, m.cc_addr,
                m.date, m.body_text, m.body_html, m.flags, m.ai_summary, m.ai_priority,
                m.ai_fraud_score, m.is_read, m.is_flagged, m.synced, m.has_attachments
         FROM messages m",
    );
    if let Some(_) = &fts_terms {
        sql.push_str(" JOIN messages_fts f ON m.id = f.rowid");
    }
    sql.push_str(" WHERE m.account_id = ?");
    if let Some(_) = &fts_terms {
        sql.push_str(" AND f.messages_fts MATCH ?");
    }
    if flag_only {
        sql.push_str(" AND m.is_flagged = 1");
    }
    sql.push_str(" AND (m.flags NOT LIKE '%\\\\Deleted%' OR m.flags IS NULL)");
    sql.push_str(" ORDER BY m.date DESC LIMIT ?");

    let mut stmt = conn.prepare(&sql)?;

    let mapper = |row: &rusqlite::Row| {
        Ok(MessageRecord {
            id: row.get(0)?,
            account_id: row.get(1)?,
            uid: row.get(2)?,
            message_id: row.get(3)?,
            subject: row.get(4)?,
            from_addr: row.get(5)?,
            to_addr: row.get(6)?,
            cc_addr: row.get(7)?,
            date: row.get(8)?,
            body_text: row.get(9)?,
            body_html: row.get(10)?,
            flags: row.get(11)?,
            ai_summary: row.get(12)?,
            ai_priority: row.get(13)?,
            ai_fraud_score: row.get(14)?,
            is_read: row.get::<_, i32>(15)? != 0,
            is_flagged: row.get::<_, i32>(16)? != 0,
            synced: row.get::<_, i32>(17)? != 0,
            has_attachments: row.get::<_, i32>(18)? != 0,
        })
    };

    let rows = match &fts_terms {
        Some(match_expr) => stmt.query_map(params![account_id, match_expr, limit], mapper)?,
        None => stmt.query_map(params![account_id, limit], mapper)?,
    };

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Prune old cached messages to bound local cache growth.
///
/// This only affects the **local cache** — messages remain on the IMAP server
/// and are re-fetched on demand. A row is eligible for deletion only if all of:
///   * it was cached more than `retention_days` ago (`created_at`), and
///   * it is fully synced (`synced = 1`, never drop pending local state), and
///   * it is **not** among the newest `keep_minimum` messages of its account
///     (ordered by email date), so a rarely-used account is never emptied.
///
/// Returns the number of deleted rows.
pub fn prune_old_messages(
    conn: &Connection,
    retention_days: u32,
    keep_minimum: u32,
) -> Result<usize, rusqlite::Error> {
    let cutoff = format!("-{} days", retention_days.max(1));
    let deleted = conn.execute(
        "DELETE FROM messages
         WHERE synced = 1
           AND created_at < datetime('now', ?1)
           AND id NOT IN (
               SELECT id FROM (
                   SELECT id,
                          ROW_NUMBER() OVER (
                              PARTITION BY account_id
                              ORDER BY date DESC, uid DESC
                          ) AS rn
                   FROM messages
               )
               WHERE rn <= ?2
           )",
        params![cutoff, keep_minimum],
    )?;
    Ok(deleted)
}

/// Reclaim disk space after a prune. `VACUUM` rewrites the database file and
/// must run outside any transaction; callers should invoke it only after
/// `prune_old_messages` actually deleted rows.
pub fn vacuum(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch("VACUUM")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::db;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        conn
    }

    fn create_test_account(conn: &Connection) -> i64 {
        crate::cache::accounts::create_account(
            conn,
            "Test",
            "imap.test.com", 993, true, "smtp.test.com", 587, true,
            "user@test.com", "pass", "smtp_user@test.com", "smtp_pass",
            "Sender", "sender@test.com", false,
        )
        .unwrap()
    }

    fn insert_folder(conn: &Connection, account_id: i64, name: &str, local_only: bool) -> i64 {
        conn.execute(
            "INSERT INTO folders (account_id, name, imap_id, local_only) VALUES (?1, ?2, NULL, ?3)",
            params![account_id, name, local_only as i32],
        )
        .unwrap();
        conn.query_row("SELECT id FROM folders WHERE account_id = ?1 AND name = ?2", params![account_id, name], |r| r.get(0))
            .unwrap()
    }

    fn insert_message(conn: &Connection, account_id: i64, folder_id: i64, uid: i64) -> i64 {
        conn.execute(
            "INSERT INTO messages (account_id, folder_id, uid, subject, body_text, synced) VALUES (?1, ?2, ?3, 'Test', 'Body', 1)",
            params![account_id, folder_id, uid],
        )
        .unwrap();
        conn.query_row(
            "SELECT id FROM messages WHERE account_id = ?1 AND folder_id = ?2 AND uid = ?3",
            params![account_id, folder_id, uid],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn test_mark_as_read_in_folder_only_hits_that_folder() {
        let conn = setup_db();
        let account = create_test_account(&conn);
        let imap = insert_folder(&conn, account, "INBOX", false);
        let local = insert_folder(&conn, account, "Mama und Papa", true);
        // Same uid in both folders (the local-only collision scenario).
        let imap_msg = insert_message(&conn, account, imap, 42);
        let local_msg = insert_message(&conn, account, local, 42);

        mark_as_read_in_folder(&conn, account, 42, Some(imap)).unwrap();

        let read: i64 = conn
            .query_row("SELECT is_read FROM messages WHERE id = ?1", params![imap_msg], |r| r.get(0))
            .unwrap();
        assert_eq!(read, 1);
        let local_read: i64 = conn
            .query_row("SELECT is_read FROM messages WHERE id = ?1", params![local_msg], |r| r.get(0))
            .unwrap();
        assert_eq!(local_read, 0, "local-only row with same uid must NOT be touched");
    }

    #[test]
    fn test_mark_as_read_in_folder_without_folder_falls_back_to_unscoped() {
        let conn = setup_db();
        let account = create_test_account(&conn);
        let folder = insert_folder(&conn, account, "INBOX", false);
        let msg = insert_message(&conn, account, folder, 7);

        mark_as_read_in_folder(&conn, account, 7, None).unwrap();

        let read: i64 = conn
            .query_row("SELECT is_read FROM messages WHERE id = ?1", params![msg], |r| r.get(0))
            .unwrap();
        assert_eq!(read, 1);
    }

    #[test]
    fn test_is_local_only_folder() {
        let conn = setup_db();
        let account = create_test_account(&conn);
        insert_folder(&conn, account, "INBOX", false);
        insert_folder(&conn, account, "Auto", true);
        assert!(!is_local_only_folder(&conn, account, "INBOX").unwrap());
        assert!(is_local_only_folder(&conn, account, "Auto").unwrap());
        // Unknown folder is not local-only.
        assert!(!is_local_only_folder(&conn, account, "GibtEsNicht").unwrap());
    }

    /// The legacy "plain text in body_html" rows must be cleaned on boot.
    /// This replicates the cleanup UPDATE from `db::init_db` so the semantics
    /// are pinned by a test: a row whose body_html equals its body_text (both
    /// non-empty) is plain text that the UI would wrongly render as HTML —
    /// it must be reset to NULL so the message renders from body_text.
    #[test]
    fn test_init_db_cleans_plain_text_from_body_html() {
        let conn = setup_db();
        let account = create_test_account(&conn);
        let folder = insert_folder(&conn, account, "Auto", true);

        // Body_html == body_text (the corrupted legacy pattern).
        conn.execute(
            "INSERT INTO messages (account_id, folder_id, uid, subject, body_text, body_html, synced)
             VALUES (?1, ?2, 100, 'Flow', 'Klartext\nZeile 2', 'Klartext\nZeile 2', 1)",
            params![account, folder],
        )
        .unwrap();
        // Real HTML must survive (body_html differs from body_text).
        conn.execute(
            "INSERT INTO messages (account_id, folder_id, uid, subject, body_text, body_html, synced)
             VALUES (?1, ?2, 101, 'Html', 'plain', '<p>HTML</p>', 1)",
            params![account, folder],
        )
        .unwrap();

        // Re-run init_db (idempotent) to apply the cleanup migration.
        db::init_db(&conn).unwrap();

        let flow_html: Option<String> = conn
            .query_row("SELECT body_html FROM messages WHERE uid = 100", [], |r| r.get(0))
            .unwrap();
        assert_eq!(flow_html, None, "plain text in body_html must be NULLed");
        let html_kept: Option<String> = conn
            .query_row("SELECT body_html FROM messages WHERE uid = 101", [], |r| r.get(0))
            .unwrap();
        assert_eq!(html_kept.as_deref(), Some("<p>HTML</p>"), "real HTML must be preserved");
    }

    #[test]
    fn test_mark_as_unseen_in_folder_only_hits_that_folder() {
        let conn = setup_db();
        let account = create_test_account(&conn);
        let imap = insert_folder(&conn, account, "INBOX", false);
        let local = insert_folder(&conn, account, "Entwürfe", true);
        let imap_msg = insert_message(&conn, account, imap, 99);
        let local_msg = insert_message(&conn, account, local, 99);

        // Mark both read first, then unsee only the IMAP row.
        mark_as_read_in_folder(&conn, account, 99, None).unwrap();
        mark_as_unseen_in_folder(&conn, account, 99, Some(imap)).unwrap();

        let imap_read: i64 = conn
            .query_row("SELECT is_read FROM messages WHERE id = ?1", params![imap_msg], |r| r.get(0))
            .unwrap();
        assert_eq!(imap_read, 0);
        let local_read: i64 = conn
            .query_row("SELECT is_read FROM messages WHERE id = ?1", params![local_msg], |r| r.get(0))
            .unwrap();
        assert_eq!(local_read, 1, "local-only row with same uid must stay read");
    }

    #[test]
    fn test_fetch_message_body_with_folder_disambiguates_uid_collision() {
        let conn = setup_db();
        let account = create_test_account(&conn);
        let ecommerce = insert_folder(&conn, account, "Ecommerce", true);
        let auto = insert_folder(&conn, account, "Auto", true);
        // Same uid 42 in both local-only folders — the collision that made the
        // preview show a body from "Auto" while the header came from "Ecommerce".
        conn.execute(
            "INSERT INTO messages (account_id, folder_id, uid, subject, body_text, synced) VALUES (?1, ?2, 42, 'Ecommerce-Mail', 'Ecommerce-Body', 1)",
            params![account, ecommerce],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (account_id, folder_id, uid, subject, body_text, synced) VALUES (?1, ?2, 42, 'Auto-Mail', 'Auto-Body', 1)",
            params![account, auto],
        )
        .unwrap();

        // Folder-scoped lookup must return the Ecommerce row.
        let (eco_msg, eco_folder) = fetch_message_body_with_folder(&conn, account, 42, Some("Ecommerce"))
            .unwrap()
            .expect("Ecommerce row must be found");
        assert_eq!(eco_msg.subject.as_deref(), Some("Ecommerce-Mail"));
        assert_eq!(eco_msg.body_text.as_deref(), Some("Ecommerce-Body"));
        assert_eq!(eco_folder, "Ecommerce");

        // The same call for the other folder must return the Auto row.
        let (auto_msg, auto_folder) = fetch_message_body_with_folder(&conn, account, 42, Some("Auto"))
            .unwrap()
            .expect("Auto row must be found");
        assert_eq!(auto_msg.subject.as_deref(), Some("Auto-Mail"));
        assert_eq!(auto_msg.body_text.as_deref(), Some("Auto-Body"));
        assert_eq!(auto_folder, "Auto");
    }

    #[test]
    fn test_fetch_message_body_without_folder_picks_lowest_id() {
        let conn = setup_db();
        let account = create_test_account(&conn);
        let ecommerce = insert_folder(&conn, account, "Ecommerce", true);
        let auto = insert_folder(&conn, account, "Auto", true);
        conn.execute(
            "INSERT INTO messages (account_id, folder_id, uid, subject, body_text, synced) VALUES (?1, ?2, 42, 'Auto-Mail', 'Auto-Body', 1)",
            params![account, auto],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (account_id, folder_id, uid, subject, body_text, synced) VALUES (?1, ?2, 42, 'Ecommerce-Mail', 'Ecommerce-Body', 1)",
            params![account, ecommerce],
        )
        .unwrap();

        // Unscoped lookup is ambiguous: it orders by lowest id (Auto inserted
        // first here). Callers MUST pass the folder to get the right row.
        let (msg, _folder) = fetch_message_body_with_folder(&conn, account, 42, None)
            .unwrap()
            .expect("a row must be found");
        assert_eq!(msg.subject.as_deref(), Some("Auto-Mail"));
    }

    #[test]
    fn test_update_body_by_id_only_updates_target_row() {
        let conn = setup_db();
        let account = create_test_account(&conn);
        let ecommerce = insert_folder(&conn, account, "Ecommerce", true);
        let auto = insert_folder(&conn, account, "Auto", true);
        conn.execute(
            "INSERT INTO messages (account_id, folder_id, uid, subject, body_text, synced) VALUES (?1, ?2, 42, 'Auto-Mail', 'Auto-Body', 1)",
            params![account, auto],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (account_id, folder_id, uid, subject, body_text, synced) VALUES (?1, ?2, 42, 'Ecommerce-Mail', 'Ecommerce-Body', 1)",
            params![account, ecommerce],
        )
        .unwrap();

        // Resolve the Ecommerce row via folder-scoped lookup, then write the
        // IMAP-fetched body by primary key — must not touch the Auto row.
        let (eco_msg, _) = fetch_message_body_with_folder(&conn, account, 42, Some("Ecommerce"))
            .unwrap()
            .unwrap();
        update_body_by_id(&conn, eco_msg.id, "Frisch vom IMAP", None).unwrap();

        let eco_body: String = conn
            .query_row("SELECT body_text FROM messages WHERE id = ?1", params![eco_msg.id], |r| r.get(0))
            .unwrap();
        assert_eq!(eco_body, "Frisch vom IMAP");
        let auto_body: String = conn.query_row(
            "SELECT body_text FROM messages WHERE subject = 'Auto-Mail'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(auto_body, "Auto-Body", "Auto row must keep its body");
    }
}
