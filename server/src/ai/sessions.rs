//! Session store for the agentic assistant (Concept §5.4).
//!
//! Server-side history (unlike Beacon's client-side): auditable, truncatable,
//! multi-device. Stores only text roles + card ids (tool raw data never goes
//! into history). Truncated to the last 10 rounds / 6000 chars.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// One persisted history entry (role `user`/`assistant` only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    pub role: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub plan_ids: Vec<String>,
    #[serde(default)]
    pub ts: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub created_at: String,
    pub last_active: String,
    pub locale: Option<String>,
    pub messages: Vec<SessionMessage>,
}

const MAX_ROUNDS: usize = 20; // 10 user+assistant rounds
const MAX_CHARS: usize = 6000;
/// Per-message text cap (Phase C session compression): a single very long
/// message (e.g. a pasted mail body) is clamped so it cannot dominate the
/// whole context on its own.
const MAX_MSG_CHARS: usize = 2000;

/// Create a new session (no-op if it already exists).
pub fn create_session(conn: &Connection, id: &str, locale: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO ai_sessions (id, created_at, last_active, locale, messages_json)
         VALUES (?1, datetime('now'), datetime('now'), ?2, '[]')",
        rusqlite::params![id, locale],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Load a session (messages parsed from JSON).
pub fn get_session(conn: &Connection, id: &str) -> Result<Option<Session>, String> {
    let row = conn
        .query_row(
            "SELECT id, created_at, last_active, locale, messages_json FROM ai_sessions WHERE id = ?1",
            rusqlite::params![id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        )
        .ok();
    let Some((id, created_at, last_active, locale, json)) = row else {
        return Ok(None);
    };
    let messages: Vec<SessionMessage> = serde_json::from_str(&json).unwrap_or_default();
    Ok(Some(Session { id, created_at, last_active, locale, messages }))
}

/// Append a message to a session, applying truncation. Creates the session if
/// it does not exist yet.
pub fn append_message(conn: &Connection, id: &str, locale: &str, msg: SessionMessage) -> Result<(), String> {
    create_session(conn, id, locale)?;
    let existing = get_session(conn, id)?.map(|s| s.messages).unwrap_or_default();
    let mut msgs = existing;
    // Per-message compression: clamp a single oversized message (Phase C).
    let mut msg = msg;
    if msg.text.chars().count() > MAX_MSG_CHARS {
        let kept: String = msg.text.chars().take(MAX_MSG_CHARS).collect();
        msg.text = format!("{kept}…[gekürzt]");
    }
    msgs.push(msg);
    // Truncate to the last 10 rounds.
    if msgs.len() > MAX_ROUNDS {
        msgs = msgs.split_off(msgs.len() - MAX_ROUNDS);
    }
    // Truncate by total character budget (drop oldest until under budget).
    let total: usize = msgs.iter().map(|m| m.text.len()).sum();
    if total > MAX_CHARS {
        let mut overflow = total - MAX_CHARS;
        while overflow > 0 && msgs.len() > 1 {
            let first = msgs[0].text.len();
            msgs.remove(0);
            overflow = overflow.saturating_sub(first);
        }
    }
    let json = serde_json::to_string(&msgs).map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE ai_sessions SET messages_json = ?1, last_active = datetime('now') WHERE id = ?2",
        rusqlite::params![json, id],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Mark a session active (bumps `last_active`).
pub fn touch(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE ai_sessions SET last_active = datetime('now') WHERE id = ?1",
        rusqlite::params![id],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection as C;

    fn mem() -> C {
        let c = C::open_in_memory().unwrap();
        c.execute(
            "CREATE TABLE ai_sessions (id TEXT PRIMARY KEY, created_at TEXT, last_active TEXT, locale TEXT, messages_json TEXT NOT NULL DEFAULT '[]')",
            [],
        )
        .unwrap();
        c
    }

    #[test]
    fn create_and_get() {
        let c = mem();
        create_session(&c, "s1", "de").unwrap();
        let s = get_session(&c, "s1").unwrap().unwrap();
        assert_eq!(s.id, "s1");
        assert_eq!(s.locale.as_deref(), Some("de"));
        assert!(s.messages.is_empty());
    }

    #[test]
    fn append_and_truncate_by_rounds() {
        let c = mem();
        create_session(&c, "s2", "de").unwrap();
        for i in 0..30 {
            append_message(&c, "s2", "de", SessionMessage {
                role: if i % 2 == 0 { "user".into() } else { "assistant".into() },
                text: format!("msg {i}"),
                plan_ids: vec![],
                ts: String::new(),
            })
            .unwrap();
        }
        let s = get_session(&c, "s2").unwrap().unwrap();
        assert_eq!(s.messages.len(), MAX_ROUNDS);
        // Oldest kept is the 10th-from-last.
        assert!(s.messages.last().unwrap().text.contains("29"));
    }

    #[test]
    fn append_truncates_by_chars() {
        let c = mem();
        create_session(&c, "s3", "de").unwrap();
        for i in 0..5 {
            append_message(&c, "s3", "de", SessionMessage {
                role: "user".into(),
                text: "x".repeat(2000),
                plan_ids: vec![],
                ts: String::new(),
            })
            .unwrap();
        }
        let s = get_session(&c, "s3").unwrap().unwrap();
        let total: usize = s.messages.iter().map(|m| m.text.len()).sum();
        assert!(total <= MAX_CHARS + 2000, "should stay near budget, got {total}");
    }

    #[test]
    fn append_clamps_single_oversized_message() {
        let c = mem();
        create_session(&c, "s4", "de").unwrap();
        append_message(
            &c,
            "s4",
            "de",
            SessionMessage {
                role: "user".into(),
                text: "y".repeat(MAX_MSG_CHARS + 500),
                plan_ids: vec![],
                ts: String::new(),
            },
        )
        .unwrap();
        let s = get_session(&c, "s4").unwrap().unwrap();
        assert_eq!(s.messages.len(), 1);
        // Clamped to the cap plus the truncation marker.
        assert!(s.messages[0].text.chars().count() <= MAX_MSG_CHARS + 16, "got {}", s.messages[0].text.chars().count());
        assert!(s.messages[0].text.ends_with("…[gekürzt]"));
    }
}
