use rusqlite::Connection;

/// Compute Levenshtein edit distance ratio (0.0 = identical, 1.0 = completely different).
/// Uses O(min(a.len(), b.len())) space DP.
pub fn edit_distance_ratio(a: &str, b: &str) -> f64 {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let (a_len, b_len) = (a_chars.len(), b_chars.len());

    if a_len == 0 && b_len == 0 {
        return 0.0;
    }
    let max_len = a_len.max(b_len);
    if max_len == 0 {
        return 0.0;
    }

    // Optimize: use smaller string as inner array
    if a_len > b_len {
        return edit_distance_ratio(b, a);
    }

    let mut prev = (0..=a_len).collect::<Vec<usize>>();
    let mut curr = vec![0; a_len + 1];

    for i in 1..=b_len {
        curr[0] = i;
        for j in 1..=a_len {
            let cost = if a_chars[j - 1] == b_chars[i - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[a_len] as f64 / max_len as f64
}

/// Store a diff between AI draft and user's final sent text.
/// Only stores if edit distance exceeds `min_ratio` (default 0.05 = 5%).
/// Enforces FIFO: max 20 diffs per (account, recipient).
pub fn queue_diff(
    conn: &Connection,
    account_id: i64,
    email_hash: &str,
    topic_tag: &str,
    ai_draft: &str,
    user_final: &str,
    min_ratio: f64,
) -> Result<bool, rusqlite::Error> {
    let ratio = edit_distance_ratio(ai_draft, user_final);
    if ratio < min_ratio {
        return Ok(false);
    }

    conn.execute(
        "INSERT INTO learning_diffs (account_id, email_hash, topic_tag, ai_draft, user_final, edit_distance, analyzed)
         VALUES (?, ?, ?, ?, ?, ?, 0)",
        rusqlite::params![account_id, email_hash, topic_tag, ai_draft, user_final, ratio],
    )?;

    // FIFO cleanup: keep only newest 20 per (account, recipient)
    cleanup_diffs(conn, account_id, email_hash, 20)?;

    Ok(true)
}

/// Get unanalyzed diffs for background processing.
pub fn get_unanalyzed_diffs(
    conn: &Connection,
    limit: i64,
) -> Result<Vec<LearningDiff>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, email_hash, topic_tag, ai_draft, user_final, edit_distance, style_hint, analyzed, created_at
         FROM learning_diffs
         WHERE analyzed = 0
         ORDER BY edit_distance DESC, created_at ASC
         LIMIT ?"
    )?;
    let rows = stmt.query_map(rusqlite::params![limit], |row| {
        Ok(LearningDiff {
            id: row.get(0)?,
            account_id: row.get(1)?,
            email_hash: row.get(2)?,
            topic_tag: row.get(3)?,
            ai_draft: row.get(4)?,
            user_final: row.get(5)?,
            edit_distance: row.get(6)?,
            style_hint: row.get(7)?,
            analyzed: row.get(8)?,
            created_at: row.get(9)?,
        })
    })?;
    let diffs: Vec<LearningDiff> = rows.filter_map(|r| r.ok()).collect();
    Ok(diffs)
}

/// Mark a diff as analyzed and store the LLM-extracted style hint.
pub fn mark_analyzed(
    conn: &Connection,
    diff_id: i64,
    style_hint: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE learning_diffs SET analyzed = 1, style_hint = ? WHERE id = ?",
        (style_hint, diff_id),
    )?;
    Ok(())
}

/// Get aggregated style hints for a recipient (from analyzed diffs).
#[allow(dead_code)]
pub fn get_style_hints(
    conn: &Connection,
    account_id: i64,
    email_hash: &str,
    topic_tag: &str,
) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT style_hint FROM learning_diffs
         WHERE account_id = ? AND email_hash = ? AND topic_tag = ? AND analyzed = 1 AND style_hint IS NOT NULL
         ORDER BY created_at DESC
         LIMIT 10"
    )?;
    let hints: Vec<String> = stmt.query_map(
        rusqlite::params![account_id, email_hash, topic_tag],
        |row| row.get(0),
    )?
    .filter_map(|r| r.ok())
    .collect();
    Ok(hints)
}

/// Remove oldest diffs when limit exceeded.
fn cleanup_diffs(
    conn: &Connection,
    account_id: i64,
    email_hash: &str,
    max_keep: i64,
) -> Result<(), rusqlite::Error> {
    // Get IDs to keep (newest N)
    let mut stmt = conn.prepare(
        "SELECT id FROM learning_diffs
         WHERE account_id = ? AND email_hash = ?
         ORDER BY id DESC LIMIT ?"
    )?;
    let keep_ids: Vec<i64> = stmt.query_map(
        rusqlite::params![account_id, email_hash, max_keep],
        |row| row.get(0),
    )?
    .filter_map(|r| r.ok())
    .collect();

    if keep_ids.is_empty() {
        return Ok(());
    }

    // Delete everything NOT in keep list (one by one to avoid param binding issues)
    let mut del_stmt = conn.prepare(
        "SELECT id FROM learning_diffs
         WHERE account_id = ? AND email_hash = ? AND id NOT IN (
             SELECT id FROM learning_diffs
             WHERE account_id = ? AND email_hash = ?
             ORDER BY id DESC LIMIT ?
         )"
    )?;
    let ids_to_delete: Vec<i64> = del_stmt.query_map(
        rusqlite::params![account_id, email_hash, account_id, email_hash, max_keep],
        |row| row.get(0),
    )?
    .filter_map(|r| r.ok())
    .collect();

    for id in ids_to_delete {
        let _ = conn.execute("DELETE FROM learning_diffs WHERE id = ?", (id,));
    }

    Ok(())
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LearningDiff {
    pub id: i64,
    pub account_id: i64,
    pub email_hash: String,
    pub topic_tag: String,
    pub ai_draft: String,
    pub user_final: String,
    pub edit_distance: f64,
    pub style_hint: Option<String>,
    pub analyzed: i32,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS contact_profiles (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                email_hash TEXT NOT NULL,
                UNIQUE(account_id, email_hash)
            );
            CREATE TABLE IF NOT EXISTS learning_diffs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                email_hash TEXT NOT NULL,
                topic_tag TEXT NOT NULL DEFAULT 'general',
                ai_draft TEXT NOT NULL,
                user_final TEXT NOT NULL,
                edit_distance REAL NOT NULL,
                style_hint TEXT,
                analyzed INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (account_id, email_hash) REFERENCES contact_profiles(account_id, email_hash) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_learning_diffs_analyzed ON learning_diffs(account_id, email_hash, analyzed);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO contact_profiles (account_id, email_hash) VALUES (1, 'hash1')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_edit_distance_identical() {
        assert!((edit_distance_ratio("hello", "hello") - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_edit_distance_empty_vs_text() {
        let ratio = edit_distance_ratio("", "hello");
        assert!((ratio - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_edit_distance_both_empty() {
        assert!((edit_distance_ratio("", "") - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_edit_distance_small_change() {
        let ratio = edit_distance_ratio("hallo", "Hallo");
        assert!(ratio > 0.0 && ratio < 0.3);
    }

    #[test]
    fn test_edit_distance_significant_change() {
        let a = "Sehr geehrte Damen und Herren, hiermit möchte ich mich bewerben.";
        let b = "Hi! Ich schick dir mal meine Unterlagen.";
        let ratio = edit_distance_ratio(a, b);
        assert!(ratio > 0.3, "expected significant difference, got {}", ratio);
    }

    #[test]
    fn test_queue_diff_below_threshold() {
        let conn = test_conn();
        // 1 char diff out of 30 = 0.033 < 0.05 → should NOT be queued
        let result = queue_diff(
            &conn,
            1,
            "hash1",
            "general",
            "This is a very long sentence with many words here",
            "This is a very long sentence with many words Here",
            0.05,
        )
        .unwrap();
        assert!(!result, "small change should not be queued");
    }

    #[test]
    fn test_queue_diff_above_threshold() {
        let conn = test_conn();
        let result = queue_diff(
            &conn,
            1,
            "hash1",
            "projekt",
            "Sehr geehrte Damen und Herren, ich schreibe Ihnen bezüglich des Projekts.",
            "Hi! Quick update zum Projekt: alles läuft gut.",
            0.05,
        )
        .unwrap();
        assert!(result, "significant change should be queued");
    }

    #[test]
    fn test_get_unanalyzed_diffs() {
        let conn = test_conn();
        queue_diff(
            &conn,
            1,
            "hash1",
            "projekt",
            "Formal text here that is quite long and detailed in nature.",
            "Short casual version of the same message.",
            0.0,
        )
        .unwrap();

        let diffs = get_unanalyzed_diffs(&conn, 10).unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].analyzed, 0);
        assert!(diffs[0].edit_distance > 0.0);
    }

    #[test]
    fn test_mark_analyzed_sets_hint() {
        let conn = test_conn();
        queue_diff(
            &conn,
            1,
            "hash1",
            "general",
            "Long formal version of the message with many words.",
            "Short casual version.",
            0.0,
        )
        .unwrap();

        let diffs = get_unanalyzed_diffs(&conn, 10).unwrap();
        let diff_id = diffs[0].id;
        mark_analyzed(&conn, diff_id, "Verwende kürzere Sätze.").unwrap();

        let diffs = get_unanalyzed_diffs(&conn, 10).unwrap();
        assert_eq!(diffs.len(), 0, "should be marked analyzed");

        let hints = get_style_hints(&conn, 1, "hash1", "general").unwrap();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0], "Verwende kürzere Sätze.");
    }

    #[test]
    fn test_fifo_cleanup_keeps_newest() {
        let conn = test_conn();
        for i in 0..25 {
            queue_diff(
                &conn,
                1,
                "hash1",
                "general",
                &format!("AI draft version {}", i),
                &format!("User final version {}", i),
                0.0,
            )
            .unwrap();
        }

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM learning_diffs WHERE account_id = 1 AND email_hash = 'hash1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 20, "should keep only newest 20");
    }

    #[test]
    fn test_fifo_independent_per_recipient() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO contact_profiles (account_id, email_hash) VALUES (1, 'hash2')",
            [],
        )
        .unwrap();

        for i in 0..25 {
            queue_diff(
                &conn,
                1,
                "hash1",
                "general",
                &format!("AI {}", i),
                &format!("User {}", i),
                0.0,
            )
            .unwrap();
            queue_diff(
                &conn,
                1,
                "hash2",
                "general",
                &format!("AI {}", i),
                &format!("User {}", i),
                0.0,
            )
            .unwrap();
        }

        let count1: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM learning_diffs WHERE account_id = 1 AND email_hash = 'hash1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let count2: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM learning_diffs WHERE account_id = 1 AND email_hash = 'hash2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count1, 20);
        assert_eq!(count2, 20);
    }

    #[test]
    fn test_get_style_hints_empty_when_none_analyzed() {
        let conn = test_conn();
        queue_diff(
            &conn,
            1,
            "hash1",
            "general",
            "AI version that is different.",
            "User version that is different.",
            0.0,
        )
        .unwrap();

        let hints = get_style_hints(&conn, 1, "hash1", "general").unwrap();
        assert!(hints.is_empty(), "no analyzed diffs → no hints");
    }
}
