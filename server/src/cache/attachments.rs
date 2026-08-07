//! Attachment metadata cache.
//! Stores attachment metadata (filename, content_type, size) extracted from
//! BODYSTRUCTURE during sync, and optionally content (base64) after full fetch.

use rusqlite::Connection;

/// Get all attachment metadata for a message (cached content included).
pub fn get_attachments(conn: &Connection, message_id: i64) -> Result<Vec<CachedAttachment>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, filename, content_type, size, content, content_cached, cached_at 
         FROM message_attachments WHERE message_id = ? ORDER BY id"
    )?;
    
    let rows = stmt.query_map([message_id], |r| {
        Ok(CachedAttachment {
                id: r.get(0)?,
                filename: r.get(1)?,
                content_type: r.get(2)?,
                size: r.get(3)?,
                content: r.get(4)?,
                content_cached: r.get::<_, i32>(5)? != 0,
                cached_at: r.get(6)?,
            })
    })?;
    
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Cache the base64 content for a specific attachment.
pub fn cache_content(conn: &Connection, attachment_id: i64, content: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE message_attachments SET content = ?, content_cached = 1, cached_at = datetime('now') WHERE id = ?",
        rusqlite::params![content, attachment_id],
    )?;
    Ok(())
}

/// Clear cached content for old/large attachments to free space.
/// Keeps metadata always. Returns number of attachments cleaned.
pub fn cleanup_content(conn: &Connection, max_keep_mb: f64) -> Result<usize, rusqlite::Error> {
    // Get current cached size
    let current_mb: f64 = conn.query_row(
        "SELECT COALESCE(SUM(size), 0) * 1.0 / (1024.0 * 1024.0) FROM message_attachments WHERE content_cached = 1",
        [],
        |r| r.get(0)
    ).unwrap_or(0.0);
    
    if current_mb <= max_keep_mb {
        return Ok(0);
    }
    
    // Clean oldest cached content first until under limit
    let mut cleaned = 0usize;
    loop {
        let cleaned_mb: f64 = conn.query_row(
            "SELECT COALESCE(SUM(size), 0) * 1.0 / (1024.0 * 1024.0) FROM message_attachments WHERE content_cached = 1",
            [],
            |r| r.get(0)
        ).unwrap_or(0.0);
        
        if cleaned_mb <= max_keep_mb {
            break;
        }
        
        // Clear oldest cached content
        let rows = conn.execute(
            "UPDATE message_attachments SET content = NULL, content_cached = 0 
             WHERE id = (SELECT id FROM message_attachments WHERE content_cached = 1 ORDER BY cached_at ASC LIMIT 1)",
            [],
        )?;
        
        if rows == 0 {
            break;
        }
        cleaned += 1;
    }
    
    Ok(cleaned)
}

/// Clear all cached content (keep metadata).
pub fn clear_all_content(conn: &Connection) -> Result<usize, rusqlite::Error> {
    let result = conn.execute(
        "UPDATE message_attachments SET content = NULL, content_cached = 0 WHERE content_cached = 1",
        [],
    )?;
    Ok(result)
}

/// Cached attachment with optional content.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CachedAttachment {
    pub id: i64,
    pub filename: String,
    pub content_type: String,
    pub size: usize,
    pub content: Option<String>,
    pub content_cached: bool,
    pub cached_at: Option<String>,
}
