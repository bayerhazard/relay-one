use rusqlite::params;
use rusqlite::Connection;

/// Maximum snippets kept per (account, recipient, topic).
const MAX_SNIPPETS_PER_TOPIC: i64 = 5;

/// Insert a sent-mail snippet. Enforces FIFO: deletes oldest when limit exceeded.
pub fn add_snippet(
    conn: &Connection,
    account_id: i64,
    email_hash: &str,
    topic_tag: &str,
    text: &str,
    sent_date: &str,
) -> Result<(), rusqlite::Error> {
    // Deduplicate: skip if identical text already exists for this recipient+topic
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM mail_snippets
         WHERE account_id = ?1 AND email_hash = ?2 AND topic_tag = ?3 AND text = ?4",
        params![account_id, email_hash, topic_tag, text],
        |row| row.get(0),
    )?;
    if exists > 0 {
        return Ok(());
    }

    conn.execute(
        "INSERT INTO mail_snippets (account_id, email_hash, topic_tag, text, sent_date, is_outgoing)
         VALUES (?1, ?2, ?3, ?4, ?5, 1)",
        params![account_id, email_hash, topic_tag, text, sent_date],
    )?;

    // FIFO cleanup: remove oldest in this topic if over limit
    cleanup_topic(conn, account_id, email_hash, topic_tag)?;

    Ok(())
}

/// Get the newest `limit` snippets for a specific topic.
pub fn get_snippets(
    conn: &Connection,
    account_id: i64,
    email_hash: &str,
    topic_tag: &str,
    limit: i64,
) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT text FROM mail_snippets
         WHERE account_id = ?1 AND email_hash = ?2 AND topic_tag = ?3
         ORDER BY sent_date DESC
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(params![account_id, email_hash, topic_tag, limit], |row| {
        row.get::<_, String>(0)
    })?;
    rows.collect()
}

/// Get general snippets (fallback when topic-specific ones are empty).
pub fn get_general_snippets(
    conn: &Connection,
    account_id: i64,
    email_hash: &str,
    limit: i64,
) -> Result<Vec<String>, rusqlite::Error> {
    get_snippets(conn, account_id, email_hash, "general", limit)
}

/// Remove oldest snippets in a topic when over MAX_SNIPPETS_PER_TOPIC.
fn cleanup_topic(
    conn: &Connection,
    account_id: i64,
    email_hash: &str,
    topic_tag: &str,
) -> Result<(), rusqlite::Error> {
    // Get IDs to keep (newest N)
    let mut stmt = conn.prepare(
        "SELECT id FROM mail_snippets
         WHERE account_id = ? AND email_hash = ? AND topic_tag = ?
         ORDER BY id DESC LIMIT ?",
    )?;
    let keep_ids: Vec<i64> = stmt
        .query_map(params![account_id, email_hash, topic_tag, MAX_SNIPPETS_PER_TOPIC], |row| {
            row.get(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;

    if keep_ids.is_empty() {
        return Ok(());
    }

    // Get all IDs in this topic
    let mut stmt2 = conn.prepare(
        "SELECT id FROM mail_snippets
         WHERE account_id = ? AND email_hash = ? AND topic_tag = ?",
    )?;
    let all_ids: Vec<i64> = stmt2
        .query_map(params![account_id, email_hash, topic_tag], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    // Delete IDs that are NOT in keep_ids
    let keep_set: std::collections::HashSet<i64> = keep_ids.into_iter().collect();
    for id in all_ids {
        if !keep_set.contains(&id) {
            let _ = conn.execute(
                "DELETE FROM mail_snippets WHERE id = ?",
                params![id],
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
                "CREATE TABLE mail_snippets (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    account_id INTEGER NOT NULL,
                    email_hash TEXT NOT NULL,
                    topic_tag TEXT NOT NULL DEFAULT 'general',
                    text TEXT NOT NULL,
                    sent_date TEXT NOT NULL,
                    is_outgoing INTEGER NOT NULL DEFAULT 1,
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))
                );",
            )
            .unwrap();
        conn
    }

    #[test]
    fn test_add_and_get_snippet() {
        let conn = create_test_conn();
        add_snippet(&conn, 1, "hash1", "general", "Hallo Anna, alles gut?", "2026-06-01").unwrap();
        let snippets = get_snippets(&conn, 1, "hash1", "general", 3).unwrap();
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0], "Hallo Anna, alles gut?");
    }

    #[test]
    fn test_deduplicate_identical_text() {
        let conn = create_test_conn();
        add_snippet(&conn, 1, "hash1", "general", "Same text", "2026-06-01").unwrap();
        add_snippet(&conn, 1, "hash1", "general", "Same text", "2026-06-02").unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM mail_snippets WHERE account_id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
        assert_eq!(count, 1, "duplicate text should not be inserted");
    }

    #[test]
    fn test_fifo_cleanup_per_topic() {
        let conn = create_test_conn();
        // Add 6 snippets (limit is 5)
        for i in 0..6 {
            add_snippet(&conn, 1, "hash1", "projekt", &format!("Mail {}", i), "2026-06-01").unwrap();
        }
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM mail_snippets WHERE account_id = 1 AND topic_tag = 'projekt'",
            [],
            |row| row.get(0),
        )
        .unwrap();
        assert_eq!(count, MAX_SNIPPETS_PER_TOPIC);

        // Oldest (Mail 0) should be removed
        let snippets = get_snippets(&conn, 1, "hash1", "projekt", 10).unwrap();
        assert!(!snippets.contains(&"Mail 0".to_string()), "oldest should be evicted");
        assert!(snippets.contains(&"Mail 5".to_string()), "newest should remain");
    }

    #[test]
    fn test_fifo_cleanup_per_topic_independent() {
        let conn = create_test_conn();
        // Add 6 snippets in "general" + 6 in "projekt"
        for i in 0..6 {
            add_snippet(&conn, 1, "hash1", "general", &format!("G{}", i), "2026-06-01").unwrap();
            add_snippet(&conn, 1, "hash1", "projekt", &format!("P{}", i), "2026-06-01").unwrap();
        }
        // Each topic capped independently at 5
        let count_general: i64 = conn.query_row(
            "SELECT COUNT(*) FROM mail_snippets WHERE account_id = 1 AND topic_tag = 'general'",
            [],
            |row| row.get(0),
        )
        .unwrap();
        let count_projekt: i64 = conn.query_row(
            "SELECT COUNT(*) FROM mail_snippets WHERE account_id = 1 AND topic_tag = 'projekt'",
            [],
            |row| row.get(0),
        )
        .unwrap();
        assert_eq!(count_general, MAX_SNIPPETS_PER_TOPIC);
        assert_eq!(count_projekt, MAX_SNIPPETS_PER_TOPIC);
    }

    #[test]
    fn test_get_snippets_returns_newest_first() {
        let conn = create_test_conn();
        add_snippet(&conn, 1, "hash1", "general", "Old", "2026-05-01").unwrap();
        add_snippet(&conn, 1, "hash1", "general", "New", "2026-06-01").unwrap();
        let snippets = get_snippets(&conn, 1, "hash1", "general", 2).unwrap();
        assert_eq!(snippets[0], "New");
        assert_eq!(snippets[1], "Old");
    }

    #[test]
    fn test_get_general_snippets() {
        let conn = create_test_conn();
        add_snippet(&conn, 1, "hash1", "general", "General text", "2026-06-01").unwrap();
        let snippets = get_general_snippets(&conn, 1, "hash1", 3).unwrap();
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0], "General text");
    }

    #[test]
    fn test_empty_snippets_for_unknown_recipient() {
        let conn = create_test_conn();
        let snippets = get_snippets(&conn, 1, "unknown", "general", 3).unwrap();
        assert!(snippets.is_empty());
    }
}
