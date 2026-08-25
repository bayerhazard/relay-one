use rusqlite::Connection;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StyleFingerprint {
    pub id: i64,
    pub account_id: i64,
    pub email_hash: String,
    pub fingerprint: String,
    pub hint_count: i64,
    pub last_updated: String,
}

pub fn get_fingerprint(
    conn: &Connection,
    account_id: i64,
    email_hash: &str,
) -> Result<Option<StyleFingerprint>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, email_hash, fingerprint, hint_count, last_updated
         FROM style_fingerprints
         WHERE account_id = ? AND email_hash = ?",
    )?;
    let rows = stmt.query_map(rusqlite::params![account_id, email_hash], |row| {
        Ok(StyleFingerprint {
            id: row.get(0)?,
            account_id: row.get(1)?,
            email_hash: row.get(2)?,
            fingerprint: row.get(3)?,
            hint_count: row.get(4)?,
            last_updated: row.get(5)?,
        })
    })?;
    let fps: Vec<StyleFingerprint> = rows.filter_map(|r| r.ok()).collect();
    Ok(fps.into_iter().next())
}

pub fn save_fingerprint(
    conn: &Connection,
    account_id: i64,
    email_hash: &str,
    fingerprint: &str,
    hint_count: i64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO style_fingerprints (account_id, email_hash, fingerprint, hint_count, last_updated)
         VALUES (?, ?, ?, ?, datetime('now'))
         ON CONFLICT(account_id, email_hash) DO UPDATE SET
            fingerprint = excluded.fingerprint,
            hint_count = excluded.hint_count,
            last_updated = datetime('now')",
        rusqlite::params![account_id, email_hash, fingerprint, hint_count],
    )?;
    Ok(())
}

#[allow(dead_code)]
pub fn needs_refresh(
    conn: &Connection,
    account_id: i64,
    email_hash: &str,
) -> Result<bool, rusqlite::Error> {
    let last_updated: Option<String> = conn
        .query_row(
            "SELECT last_updated FROM style_fingerprints
             WHERE account_id = ? AND email_hash = ?",
            rusqlite::params![account_id, email_hash],
            |row| row.get(0),
        )
        .ok()
        .flatten();

    match last_updated {
        None => Ok(true),
        Some(ref updated) => {
            let new_hints: i64 = conn.query_row(
                "SELECT COUNT(*) FROM learning_diffs
                 WHERE account_id = ? AND email_hash = ? AND analyzed = 1
                   AND created_at > ?",
                rusqlite::params![account_id, email_hash, updated],
                |row| row.get(0),
            )?;
            Ok(new_hints >= 3)
        }
    }
}

pub fn get_hints_for_synthesis(
    conn: &Connection,
    account_id: i64,
    email_hash: &str,
) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT style_hint FROM learning_diffs
         WHERE account_id = ? AND email_hash = ? AND analyzed = 1 AND style_hint IS NOT NULL
         ORDER BY created_at DESC
         LIMIT 20",
    )?;
    let hints: Vec<String> = stmt
        .query_map(rusqlite::params![account_id, email_hash], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(hints)
}

/// Recipient candidates for the global (account-agnostic) background
/// RefreshFingerprint task. Groups analyzed hints across ALL accounts.
#[derive(Debug, Clone)]
pub struct RefreshCandidate {
    pub account_id: i64,
    pub email_hash: String,
}

pub fn get_refresh_candidates(
    conn: &Connection,
    limit: i64,
) -> Result<Vec<RefreshCandidate>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT account_id, email_hash FROM learning_diffs
         WHERE analyzed = 1 AND style_hint IS NOT NULL
         GROUP BY account_id, email_hash
         HAVING COUNT(*) >= 3
         ORDER BY MAX(created_at) DESC
         LIMIT ?",
    )?;
    let rows = stmt.query_map(rusqlite::params![limit], |row| {
        Ok(RefreshCandidate {
            account_id: row.get(0)?,
            email_hash: row.get(1)?,
        })
    })?;
    let out: Vec<RefreshCandidate> = rows.filter_map(|r| r.ok()).collect();
    Ok(out)
}

#[allow(dead_code)]
pub fn get_recipients_needing_refresh(
    conn: &Connection,
    account_id: i64,
    limit: i64,
) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT email_hash FROM learning_diffs
         WHERE account_id = ? AND analyzed = 1 AND style_hint IS NOT NULL
         GROUP BY email_hash
         HAVING COUNT(*) >= 3
         LIMIT ?",
    )?;
    let hashes: Vec<String> = stmt
        .query_map(rusqlite::params![account_id, limit], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(hashes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS learning_diffs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                email_hash TEXT NOT NULL,
                topic_tag TEXT NOT NULL DEFAULT 'general',
                ai_draft TEXT NOT NULL,
                user_final TEXT NOT NULL,
                edit_distance REAL NOT NULL,
                style_hint TEXT,
                analyzed INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS style_fingerprints (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                email_hash TEXT NOT NULL,
                fingerprint TEXT NOT NULL DEFAULT '',
                hint_count INTEGER NOT NULL DEFAULT 0,
                last_updated TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(account_id, email_hash)
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_get_fingerprint_empty() {
        let conn = test_conn();
        let fp = get_fingerprint(&conn, 1, "hash1").unwrap();
        assert!(fp.is_none());
    }

    #[test]
    fn test_save_and_get_fingerprint() {
        let conn = test_conn();
        save_fingerprint(
            &conn,
            1,
            "hash1",
            "Verwende kurze Saetze. Vermeide formelle Floskeln.",
            5,
        )
        .unwrap();

        let fp = get_fingerprint(&conn, 1, "hash1").unwrap().expect("fingerprint should exist");
        assert_eq!(fp.hint_count, 5);
        assert!(fp.fingerprint.contains("kurze Saetze"));
    }

    #[test]
    fn test_save_fingerprint_upserts() {
        let conn = test_conn();
        save_fingerprint(&conn, 1, "hash1", "Erster Stil", 3).unwrap();
        save_fingerprint(&conn, 1, "hash1", "Aktualisierter Stil", 7).unwrap();

        let fp = get_fingerprint(&conn, 1, "hash1").unwrap().expect("fingerprint should exist");
        assert_eq!(fp.hint_count, 7);
        assert_eq!(fp.fingerprint, "Aktualisierter Stil");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM style_fingerprints WHERE account_id = 1 AND email_hash = 'hash1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_needs_refresh_no_fingerprint() {
        let conn = test_conn();
        assert!(needs_refresh(&conn, 1, "hash1").unwrap());
    }

    #[test]
    fn test_needs_refresh_few_hints() {
        let conn = test_conn();
        save_fingerprint(&conn, 1, "hash1", "Stil", 2).unwrap();
        assert!(!needs_refresh(&conn, 1, "hash1").unwrap());
    }

    #[test]
    fn test_get_hints_for_synthesis() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO learning_diffs (account_id, email_hash, ai_draft, user_final, edit_distance, style_hint, analyzed)
             VALUES (1, 'hash1', 'AI', 'User', 0.5, 'Kurze Saetze', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO learning_diffs (account_id, email_hash, ai_draft, user_final, edit_distance, style_hint, analyzed)
             VALUES (1, 'hash1', 'AI', 'User', 0.6, 'Du statt Sie', 1)",
            [],
        )
        .unwrap();

        let hints = get_hints_for_synthesis(&conn, 1, "hash1").unwrap();
        assert_eq!(hints.len(), 2);
        assert!(hints.contains(&"Kurze Saetze".to_string()));
        assert!(hints.contains(&"Du statt Sie".to_string()));
    }

    #[test]
    fn test_get_recipients_needing_refresh() {
        let conn = test_conn();
        for i in 0..3 {
            conn.execute(
                &format!(
                    "INSERT INTO learning_diffs (account_id, email_hash, ai_draft, user_final, edit_distance, style_hint, analyzed)
                     VALUES (1, 'hash1', 'AI', 'User', 0.5, 'hint{}', 1)",
                    i
                ),
                [],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO learning_diffs (account_id, email_hash, ai_draft, user_final, edit_distance, style_hint, analyzed)
             VALUES (1, 'hash2', 'AI', 'User', 0.5, 'hint', 1)",
            [],
        )
        .unwrap();

        let recipients = get_recipients_needing_refresh(&conn, 1, 10).unwrap();
        assert_eq!(recipients.len(), 1);
        assert_eq!(recipients[0], "hash1");
    }

    #[test]
    fn test_get_refresh_candidates_all_accounts() {
        let conn = test_conn();
        // Account 1: 3 analyzed hints for hash-a (qualifies)
        for i in 0..3 {
            conn.execute(
                &format!(
                    "INSERT INTO learning_diffs (account_id, email_hash, ai_draft, user_final, edit_distance, style_hint, analyzed)
                     VALUES (1, 'hash-a', 'AI', 'User', 0.5, 'hint{}', 1)",
                    i
                ),
                [],
            )
            .unwrap();
        }
        // Account 2: 3 analyzed hints for hash-b (qualifies)
        for i in 0..3 {
            conn.execute(
                &format!(
                    "INSERT INTO learning_diffs (account_id, email_hash, ai_draft, user_final, edit_distance, style_hint, analyzed)
                     VALUES (2, 'hash-b', 'AI', 'User', 0.5, 'hint{}', 1)",
                    i
                ),
                [],
            )
            .unwrap();
        }
        // Account 1: only 2 hints for hash-c (does NOT qualify)
        for i in 0..2 {
            conn.execute(
                &format!(
                    "INSERT INTO learning_diffs (account_id, email_hash, ai_draft, user_final, edit_distance, style_hint, analyzed)
                     VALUES (1, 'hash-c', 'AI', 'User', 0.5, 'hint{}', 1)",
                    i
                ),
                [],
            )
            .unwrap();
        }

        let candidates = get_refresh_candidates(&conn, 10).unwrap();
        assert_eq!(candidates.len(), 2);
        let by_hash: std::collections::HashMap<&str, i64> = candidates
            .iter()
            .map(|c| (c.email_hash.as_str(), c.account_id))
            .collect();
        assert_eq!(by_hash.get("hash-a"), Some(&1));
        assert_eq!(by_hash.get("hash-b"), Some(&2));
        assert!(!by_hash.contains_key("hash-c"));
    }

    #[test]
    fn test_get_refresh_candidates_limit() {
        let conn = test_conn();
        for account in 1..=3 {
            for i in 0..3 {
                conn.execute(
                    &format!(
                        "INSERT INTO learning_diffs (account_id, email_hash, ai_draft, user_final, edit_distance, style_hint, analyzed)
                         VALUES ({}, 'h{}', 'AI', 'User', 0.5, 'hint{}', 1)",
                        account, account, i
                    ),
                    [],
                )
                .unwrap();
            }
        }
        let candidates = get_refresh_candidates(&conn, 2).unwrap();
        assert_eq!(candidates.len(), 2);
    }
}
