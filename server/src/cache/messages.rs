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

    conn.execute(
        "INSERT INTO messages (
            account_id, folder_id, uid, message_id, subject, from_addr, to_addr, date,
            body_text, body_html, flags, ai_summary, ai_priority, ai_fraud_score,
            is_read, is_flagged, has_attachments, synced
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, 1)
ON CONFLICT(account_id, uid) DO UPDATE SET
            folder_id = excluded.folder_id,
            subject = excluded.subject,
            from_addr = excluded.from_addr,
            to_addr = excluded.to_addr,
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
    
    // Save attachment metadata
    if !msg.attachments.is_empty() {
        // Get the message ID
        let message_id: i64 = conn.query_row(
            "SELECT id FROM messages WHERE account_id = ? AND uid = ?",
            params![account_id, msg.uid],
            |row| row.get(0),
        )?;
        
        // Clear old metadata and save new
        conn.execute(
            "DELETE FROM message_attachments WHERE message_id = ?",
            params![message_id],
        )?;
        
        for att in &msg.attachments {
            conn.execute(
                "INSERT INTO message_attachments (message_id, filename, content_type, size) VALUES (?, ?, ?, ?)",
                params![message_id, att.filename, att.content_type, att.size],
            )?;
        }
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
    let limit = limit.unwrap_or(50);
    let offset = offset.unwrap_or(0);

    // Exclude spam folders entirely
    if is_spam_folder(folder) {
        return Ok(Vec::new());
    }

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

    let mut stmt = conn.prepare(
        "SELECT id, account_id, uid, message_id, subject, from_addr, to_addr, date,
                body_text, body_html, flags, ai_summary, ai_priority, ai_fraud_score,
                is_read, is_flagged, synced, has_attachments
         FROM messages
         WHERE account_id = ?1 AND folder_id = ?2
           AND (flags NOT LIKE '%\\\\Deleted%' OR flags IS NULL)
         ORDER BY date DESC
         LIMIT ?3 OFFSET ?4",
    )?;

    let rows = stmt.query_map(params![account_id, folder_id, limit, offset], |row| {
        Ok(MessageRecord {
            id: row.get(0)?,
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
            is_flagged: row.get::<_, i32>(15)? != 0,
            synced: row.get::<_, i32>(16)? != 0,
            has_attachments: row.get::<_, i32>(17)? != 0,
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
) -> Result<Option<MessageRecord>, rusqlite::Error> {
       match conn.query_row(
        "SELECT id, account_id, uid, message_id, subject, from_addr, to_addr, date,
                body_text, body_html, flags, ai_summary, ai_priority, ai_fraud_score,
                is_read, is_flagged, synced, has_attachments
         FROM messages
         WHERE account_id = ?1 AND uid = ?2
           AND (flags NOT LIKE '%\\\\Deleted%' OR flags IS NULL)",
        params![account_id, uid],
        |row| {
 Ok(MessageRecord {
            id: row.get(0)?,
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
            is_flagged: row.get::<_, i32>(15)? != 0,
            synced: row.get::<_, i32>(16)? != 0,
            has_attachments: row.get::<_, i32>(17)? != 0,
        })
        },
    ) {
        Ok(record) => Ok(Some(record)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Fetch message body along with its folder name for IMAP fallback.
pub fn fetch_message_body_with_folder(
    conn: &Connection,
    account_id: i64,
    uid: i64,
) -> Result<Option<(MessageRecord, String)>, rusqlite::Error> {
    match conn.query_row(
        "SELECT m.id, m.account_id, m.uid, m.message_id, m.subject, m.from_addr, m.to_addr, m.date,
                m.body_text, m.body_html, m.flags, m.ai_summary, m.ai_priority, m.ai_fraud_score,
                m.is_read, m.is_flagged, m.synced, m.has_attachments, f.name
         FROM messages m
         JOIN folders f ON m.folder_id = f.id
         WHERE m.account_id = ?1 AND m.uid = ?2
           AND (m.flags NOT LIKE '%\\\\Deleted%' OR m.flags IS NULL)",
        params![account_id, uid],
        |row| {
            Ok((
                MessageRecord {
                    id: row.get(0)?,
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
                    is_flagged: row.get::<_, i32>(15)? != 0,
                    synced: row.get::<_, i32>(16)? != 0,
                    has_attachments: row.get::<_, i32>(17)? != 0,
                },
                row.get::<_, String>(18)?,
            ))
        },
    ) {
        Ok(record) => Ok(Some(record)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
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

/// Update the is_read flag for a message (used by flag refresh from IMAP server).
pub fn update_is_read(conn: &Connection, account_id: i64, uid: i64, is_read: bool) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE messages SET is_read = ?, updated_at = datetime('now') WHERE account_id = ?1 AND uid = ?2",
        params![is_read as i32, account_id, uid],
    )?;
    Ok(())
}

/// Update the is_flagged flag for a message (used by flag refresh from IMAP server).
pub fn update_is_flagged(conn: &Connection, account_id: i64, uid: i64, is_flagged: bool) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE messages SET is_flagged = ?, updated_at = datetime('now') WHERE account_id = ?1 AND uid = ?2",
        params![is_flagged as i32, account_id, uid],
    )?;
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
        ON CONFLICT(account_id, uid) DO UPDATE SET
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

pub fn update_ai_summary(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    summary: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE messages SET ai_summary = ?1, updated_at = datetime('now') WHERE account_id = ?2 AND uid = ?3",
        params![summary, account_id, uid],
    )?;
    Ok(())
}

pub fn update_ai_priority(
    conn: &Connection,
    account_id: i64,
    uid: i64,
    priority: f32,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE messages SET ai_priority = ?1, updated_at = datetime('now') WHERE account_id = ?2 AND uid = ?3",
        params![priority, account_id, uid],
    )?;
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
    conn.execute(
        "UPDATE messages SET folder_id = (SELECT id FROM folders WHERE account_id = ?1 AND name = ?2), updated_at = datetime('now') WHERE account_id = ?3 AND uid = ?4",
        params![account_id, folder, account_id, uid],
    )?;
    Ok(())
}

/// Build a safe FTS5 MATCH expression from free user input.
/// Each whitespace-separated term becomes a quoted prefix query, ANDed
/// together. Quoting neutralises FTS5 operators so arbitrary input cannot
/// break the query or inject syntax. Returns None if there is no usable term.
fn build_fts_query(raw: &str) -> Option<String> {
    let terms: Vec<String> = raw
        .split_whitespace()
        .filter_map(|t| {
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

/// Full-text search over cached messages (subject, sender, recipient, body)
/// for one account, newest first. Returns up to `limit` results.
pub fn search_messages(
    conn: &Connection,
    account_id: i64,
    query: &str,
    limit: i64,
) -> Result<Vec<MessageRecord>, rusqlite::Error> {
    let Some(match_expr) = build_fts_query(query) else {
        return Ok(Vec::new());
    };

    let mut stmt = conn.prepare(
        "SELECT m.id, m.account_id, m.uid, m.message_id, m.subject, m.from_addr, m.to_addr,
                m.date, m.body_text, m.body_html, m.flags, m.ai_summary, m.ai_priority,
                m.ai_fraud_score, m.is_read, m.is_flagged, m.synced, m.has_attachments
         FROM messages_fts f
         JOIN messages m ON m.id = f.rowid
         WHERE f.messages_fts MATCH ?1 AND m.account_id = ?2
           AND (m.flags NOT LIKE '%\\\\Deleted%' OR m.flags IS NULL)
         ORDER BY m.date DESC
         LIMIT ?3",
    )?;

    let rows = stmt.query_map(params![match_expr, account_id, limit], |row| {
        Ok(MessageRecord {
            id: row.get(0)?,
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
            is_flagged: row.get::<_, i32>(15)? != 0,
            synced: row.get::<_, i32>(16)? != 0,
            has_attachments: row.get::<_, i32>(17)? != 0,
        })
    })?;

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
