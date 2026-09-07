use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

pub fn log_ai_action(
    conn: &Connection,
    message_id: Option<&str>,
    action: &str,
    input_hash: &str,
    output: &str,
    model: Option<&str>,
    tone_freundlich: Option<u8>,
    tone_professionell: Option<u8>,
    tone_laenge: Option<u8>,
    confirmed: bool,
) -> Result<(), rusqlite::Error> {
    let mut hasher = Sha256::new();
    hasher.update(input_hash.as_bytes());
    let hash_hex = format!("{:x}", hasher.finalize());

    conn.execute(
        "INSERT INTO ai_audit_log (message_id, action, input_hash, output, model, tone_freundlich, tone_professionell, tone_laenge, confirmed)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            message_id,
            action,
            &hash_hex,
            output,
            model,
            tone_freundlich.map(|v| v as i32),
            tone_professionell.map(|v| v as i32),
            tone_laenge.map(|v| v as i32),
            confirmed as i32,
        ],
    )?;
    Ok(())
}

pub fn confirm_action(conn: &Connection, audit_id: i64) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE ai_audit_log SET confirmed = 1 WHERE id = ?1",
        params![audit_id],
    )?;
    Ok(())
}

// ─── Agent v2 audit (Concept §6.10) ─────────────────────────────
// A second, append-only table `ai_audit` records every agent action with its
// session and origin. `event` is a stable English identifier; `detail` is a
// free-form JSON/summary. The v1 `ai_audit_log` above is left untouched.

/// Record a single agent event. `origin` is `"chat"` or `"mail_followup"`.
pub fn log_event(
    conn: &Connection,
    session_id: Option<&str>,
    origin: &str,
    event: &str,
    detail: Option<&str>,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO ai_audit (session_id, origin, event, detail) VALUES (?1, ?2, ?3, ?4)",
        params![session_id, origin, event, detail],
    )?;
    Ok(())
}

/// Record a tool invocation (Concept §6.10: "jeder Tool-Aufruf").
pub fn log_tool_call(
    conn: &Connection,
    session_id: Option<&str>,
    origin: &str,
    tool: &str,
    args: &str,
) -> Result<(), rusqlite::Error> {
    log_event(
        conn,
        session_id,
        origin,
        "tool_call",
        Some(&format!("{{\"tool\":\"{}\",\"args\":{}}}", tool, args)),
    )
}

/// Record a plan lifecycle transition (created/confirmed/executed/cancelled/failed/undone).
pub fn log_plan(
    conn: &Connection,
    session_id: Option<&str>,
    origin: &str,
    plan_id: &str,
    transition: &str,
) -> Result<(), rusqlite::Error> {
    log_event(
        conn,
        session_id,
        origin,
        &format!("plan_{}", transition),
        Some(plan_id),
    )
}

pub fn get_audit_log(
    conn: &Connection,
    limit: Option<i64>,
) -> Result<Vec<(i64, String, String, i32, String)>, rusqlite::Error> {
    let limit = limit.unwrap_or(100);
    let mut stmt = conn.prepare(
        "SELECT id, action, input_hash, confirmed, created_at
         FROM ai_audit_log
         ORDER BY created_at DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
        ))
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute(
            "CREATE TABLE ai_audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT, origin TEXT, event TEXT NOT NULL,
                detail TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')))",
            [],
        )
        .unwrap();
        c
    }

    #[test]
    fn log_event_inserts_row() {
        let c = mem();
        log_event(&c, Some("s1"), "chat", "agent_turn", Some("hello")).unwrap();
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM ai_audit", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let (event, origin, session): (String, String, Option<String>) = c
            .query_row(
                "SELECT event, origin, session_id FROM ai_audit",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(event, "agent_turn");
        assert_eq!(origin, "chat");
        assert_eq!(session, Some("s1".into()));
    }

    #[test]
    fn log_tool_call_and_plan_transitions() {
        let c = mem();
        log_tool_call(&c, None, "chat", "contacts_search", "{\"q\":\"kai\"}").unwrap();
        log_plan(&c, Some("s2"), "chat", "pl-abc", "created").unwrap();
        log_plan(&c, Some("s2"), "chat", "pl-abc", "executed").unwrap();
        let events: Vec<String> = {
            let mut stmt = c
                .prepare("SELECT event FROM ai_audit ORDER BY id")
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(events, vec!["tool_call", "plan_created", "plan_executed"]);
    }
}
