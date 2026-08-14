//! Attachment metadata cache.
//! Stores attachment metadata (filename, content_type, size) extracted from
//! BODYSTRUCTURE during sync, and optionally content (base64) after full fetch.

use rusqlite::Connection;

/// Get all attachment metadata for a message (cached content included).
/// Ordered by `part_index` — the stable BODYSTRUCTURE position.
pub fn get_attachments(conn: &Connection, message_id: i64) -> Result<Vec<CachedAttachment>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, part_index, filename, content_type, size, content, content_cached, cached_at, sha256
         FROM message_attachments WHERE message_id = ? ORDER BY part_index ASC, id ASC"
    )?;
    
    let rows = stmt.query_map([message_id], |r| {
        Ok(CachedAttachment {
                id: r.get(0)?,
                part_index: r.get(1)?,
                filename: r.get(2)?,
                content_type: r.get(3)?,
                size: r.get(4)?,
                content: r.get(5)?,
                content_cached: r.get::<_, i32>(6)? != 0,
                cached_at: r.get(7)?,
                sha256: r.get(8)?,
            })
    })?;
    
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Reconcile attachment metadata for a message against the fresh BODYSTRUCTURE
/// list. Unlike the historical DELETE + re-INSERT (which changed attachment ids
/// on every sync and left stale rows when the new list was empty), this upserts
/// on `(message_id, part_index)` so ids stay stable, and removes rows whose part
/// no longer exists — including when the new list is empty.
pub fn reconcile_attachments(
    conn: &Connection,
    message_id: i64,
    attachments: &[crate::imap::types::AttachmentMeta],
) -> Result<(), rusqlite::Error> {
    let keep: Vec<i64> = (0..attachments.len() as i64).collect();

    for (idx, att) in attachments.iter().enumerate() {
        let part_index = idx as i64;
        conn.execute(
            "INSERT INTO message_attachments (message_id, part_index, filename, content_type, size)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(message_id, part_index) DO UPDATE SET
                filename = excluded.filename,
                content_type = excluded.content_type,
                size = excluded.size,
                cached_at = cached_at",
            rusqlite::params![message_id, part_index, att.filename, att.content_type, att.size as i64],
        )?;
    }

    // Remove rows whose part_index no longer exists in the fresh list.
    let existing: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT part_index FROM message_attachments WHERE message_id = ?1")?;
        let rows = stmt.query_map([message_id], |r| r.get::<_, i64>(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    for pi in existing {
        if !keep.contains(&pi) {
            conn.execute(
                "DELETE FROM message_attachments WHERE message_id = ?1 AND part_index = ?2",
                rusqlite::params![message_id, pi],
            )?;
        }
    }
    Ok(())
}

/// Cache the base64 content for a specific attachment.
/// Additionally persists the raw bytes deduplicated to
/// `<data>/attachments/<sha256>` and records `disk_path` (Concept §3.1).
pub fn cache_content(conn: &Connection, attachment_id: i64, content: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE message_attachments SET content = ?, content_cached = 1, cached_at = datetime('now') WHERE id = ?",
        rusqlite::params![content, attachment_id],
    )?;
    Ok(())
}

/// Persist attachment bytes deduplicated to disk, update `disk_path`.
/// `content` is standard-base64; returns the relative disk path.
pub fn cache_content_dedup(
    conn: &Connection,
    attachment_id: i64,
    content_b64: &str,
    data_root: &std::path::Path,
) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content_b64)
        .map_err(|e| format!("Attachment base64 decode: {e}"))?;
    let (abs, sha, _is_new) = crate::cache::archive::save_attachment(data_root, &bytes)
        .map_err(|e| e.to_string())?;
    let rel = abs
        .strip_prefix(data_root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| abs.to_string_lossy().to_string());
    conn.execute(
        "UPDATE message_attachments SET disk_path = ?1, sha256 = ?2, content_cached = 1, cached_at = datetime('now') WHERE id = ?3",
        rusqlite::params![rel, sha, attachment_id],
    )
    .map_err(|e| format!("Attachment disk_path update: {e}"))?;
    Ok(rel)
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

/// GC result summary.
#[derive(Debug, Default, serde::Serialize)]
pub struct GcReport {
    /// Attachment files in `<data>/attachments/` removed because no
    /// `message_attachments.disk_path` references them.
    pub removed_files: usize,
    /// Total bytes freed on disk.
    pub freed_bytes: u64,
    /// Files kept (still referenced).
    pub kept_files: usize,
}

/// Consistency report for attachment metadata vs. reality.
#[derive(Debug, Default, serde::Serialize)]
pub struct ConsistencyReport {
    /// Messages flagged `has_attachments=1` with no attachment rows at all.
    pub flagged_without_rows: usize,
    /// Messages flagged `has_attachments=0` that still have attachment rows.
    pub unflagged_with_rows: usize,
    /// Attachment rows whose `disk_path` file is missing on disk.
    pub rows_with_missing_file: usize,
    /// Attachment rows that were repaired (orphaned `disk_path` cleared).
    pub repaired_rows: usize,
}

/// Run the dedup-store GC: every file in `<data>/attachments/` that is not
/// referenced by any `message_attachments.disk_path` is orphaned and deleted.
/// Content-addressed files are safe to delete — a later fetch re-materializes
/// them. Never touches the EML archive or the DB content column.
pub fn gc_orphaned_attachments(
    conn: &Connection,
    data_root: &std::path::Path,
) -> Result<GcReport, String> {
    let dir = data_root.join("attachments");
    if !dir.is_dir() {
        return Ok(GcReport::default());
    }

    let referenced: std::collections::HashSet<String> = {
        let mut stmt = conn
            .prepare("SELECT disk_path FROM message_attachments WHERE disk_path IS NOT NULL")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.filter_map(|r| r.ok()).collect()
    };

    let mut report = GcReport::default();
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let rel = entry
            .path()
            .strip_prefix(data_root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| entry.file_name().to_string_lossy().to_string());
        if referenced.contains(&rel) {
            report.kept_files += 1;
            continue;
        }
        let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
        match std::fs::remove_file(entry.path()) {
            Ok(()) => {
                report.freed_bytes += bytes;
                report.removed_files += 1;
            }
            Err(e) => tracing::warn!("GC: Datei {} nicht entfernbar: {}", entry.path().display(), e),
        }
    }
    Ok(report)
}

/// Verify attachment metadata against reality and repair the cheap, safe
/// inconsistencies:
///   - `has_attachments` flag desynced from actual rows (both directions),
///   - rows whose `disk_path` file is missing (clears the stale reference so a
///     later fetch re-materializes from the DB content column or IMAP).
pub fn check_and_repair_attachments(
    conn: &Connection,
    data_root: &std::path::Path,
    repair: bool,
) -> Result<ConsistencyReport, String> {
    let mut report = ConsistencyReport::default();

    report.flagged_without_rows = conn
        .query_row(
            "SELECT COUNT(*) FROM messages m
             WHERE m.has_attachments = 1
               AND NOT EXISTS (SELECT 1 FROM message_attachments a WHERE a.message_id = m.id)",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| e.to_string())? as usize;

    report.unflagged_with_rows = conn
        .query_row(
            "SELECT COUNT(*) FROM messages m
             WHERE m.has_attachments = 0
               AND EXISTS (SELECT 1 FROM message_attachments a WHERE a.message_id = m.id)",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| e.to_string())? as usize;

    report.rows_with_missing_file = conn
        .query_row(
            "SELECT COUNT(*) FROM message_attachments WHERE disk_path IS NOT NULL AND disk_path != ''",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| e.to_string())? as usize;

    // Now count only rows whose file is actually missing.
    let rows: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT id FROM message_attachments WHERE disk_path IS NOT NULL AND disk_path != ''")
            .map_err(|e| e.to_string())?;
        let ids = stmt
            .query_map([], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        ids.filter_map(|r| r.ok()).collect()
    };
    let mut missing_ids: Vec<i64> = Vec::new();
    for id in &rows {
        let rel: Option<String> = conn
            .query_row(
                "SELECT disk_path FROM message_attachments WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        if let Some(rel) = rel {
            if !data_root.join(&rel).exists() {
                missing_ids.push(*id);
            }
        }
    }
    report.rows_with_missing_file = missing_ids.len();

    if repair {
        // Clear orphaned disk_path references (keep content column + metadata).
        for id in missing_ids {
            conn.execute(
                "UPDATE message_attachments SET disk_path = NULL, sha256 = NULL WHERE id = ?1",
                rusqlite::params![id],
            )
            .map_err(|e| e.to_string())?;
            report.repaired_rows += 1;
        }
        // Fix the has_attachments flag in both directions.
        let set_on = conn.execute(
            "UPDATE messages SET has_attachments = 1
             WHERE id IN (
               SELECT m.id FROM messages m
               WHERE m.has_attachments = 0
                 AND EXISTS (SELECT 1 FROM message_attachments a WHERE a.message_id = m.id)
             )",
            [],
        ).map_err(|e| e.to_string())?;
        let set_off = conn.execute(
            "UPDATE messages SET has_attachments = 0
             WHERE id IN (
               SELECT m.id FROM messages m
               WHERE m.has_attachments = 1
                 AND NOT EXISTS (SELECT 1 FROM message_attachments a WHERE a.message_id = m.id)
             )",
            [],
        ).map_err(|e| e.to_string())?;
        report.repaired_rows += (set_on + set_off) as usize;
    }
    Ok(report)
}

/// Cached attachment with optional content.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CachedAttachment {
    pub id: i64,
    /// Stable 0-based position within the message's attachment list
    /// (BODYSTRUCTURE order). Attachment ids themselves can change if rows are
    /// reconciled; part_index is the invariant used to re-match.
    pub part_index: i64,
    pub filename: String,
    pub content_type: String,
    pub size: usize,
    pub content: Option<String>,
    pub content_cached: bool,
    pub cached_at: Option<String>,
    /// Content sha256 (dedup key). Optional — set once content is materialized.
    pub sha256: Option<String>,
}
