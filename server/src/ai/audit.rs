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
