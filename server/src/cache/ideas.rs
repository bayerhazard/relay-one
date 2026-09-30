//! Internal bookkeeping for Insilo idea memos → tasks.
//!
//! One row per idea file. Never surfaced in the UI; the transcript is kept so
//! a failed processing run can be retried without re-reading the file.

use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone)]
pub struct IdeaRow {
    pub insilo_id: String,
    pub path: String,
    pub sha256: String,
    pub title: String,
    pub recorded_at: String,
    pub duration_min: i64,
    pub language: String,
    pub transcript_md: String,
    pub status: String,
    pub attempts: i64,
    pub task_uid: Option<String>,
}

/// Insert a new idea or refresh an existing one.
///
/// Returns `(inserted, changed)`. A changed file (new SHA) is reset to
/// `pending` with a fresh attempt counter so it is processed again — the
/// existing `task_uid` is kept so no duplicate task is created.
#[allow(clippy::too_many_arguments)]
pub fn upsert_idea(
    conn: &Connection,
    insilo_id: &str,
    path: &str,
    sha: &str,
    title: &str,
    recorded_at: &str,
    duration_min: i64,
    language: &str,
    transcript: &str,
) -> Result<(bool, bool), rusqlite::Error> {
    let existing: Option<(String, String)> = conn
        .query_row(
            "SELECT sha256, status FROM ideas WHERE insilo_id = ?1",
            params![insilo_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;

    match existing {
        None => {
            conn.execute(
                "INSERT INTO ideas (insilo_id, path, sha256, title, recorded_at,
                                    duration_min, language, transcript_md, status,
                                    attempts, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', 0,
                         datetime('now'), datetime('now'))",
                params![insilo_id, path, sha, title, recorded_at, duration_min, language, transcript],
            )?;
            Ok((true, true))
        }
        Some((old_sha, _status)) if old_sha == sha => {
            // Unchanged content: refresh location/title only, keep the verdict.
            conn.execute(
                "UPDATE ideas SET path = ?2, title = ?3, updated_at = datetime('now')
                 WHERE insilo_id = ?1",
                params![insilo_id, path, title],
            )?;
            Ok((false, false))
        }
        Some(_) => {
            conn.execute(
                "UPDATE ideas SET path = ?2, sha256 = ?3, title = ?4, recorded_at = ?5,
                                  duration_min = ?6, language = ?7, transcript_md = ?8,
                                  status = 'pending', attempts = 0, last_error = NULL,
                                  next_retry_at = NULL, updated_at = datetime('now')
                 WHERE insilo_id = ?1",
                params![insilo_id, path, sha, title, recorded_at, duration_min, language, transcript],
            )?;
            Ok((false, true))
        }
    }
}

/// Ideas that still need processing (pending, or failed and due for a retry).
pub fn list_due_ideas(conn: &Connection, limit: i64) -> Result<Vec<IdeaRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT insilo_id, path, sha256, title, recorded_at, duration_min,
                language, transcript_md, status, attempts, task_uid
         FROM ideas
         WHERE status IN ('pending', 'failed')
           AND (next_retry_at IS NULL OR next_retry_at <= datetime('now'))
         ORDER BY id
         LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit], |r| {
            Ok(IdeaRow {
                insilo_id: r.get(0)?,
                path: r.get(1)?,
                sha256: r.get(2)?,
                title: r.get(3)?,
                recorded_at: r.get(4)?,
                duration_min: r.get(5)?,
                language: r.get(6)?,
                transcript_md: r.get(7)?,
                status: r.get(8)?,
                attempts: r.get(9)?,
                task_uid: r.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Mark the idea as fully processed (task created or existing task enriched).
pub fn mark_done(conn: &Connection, insilo_id: &str, task_uid: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE ideas SET status = 'done', task_uid = ?2, attempts = attempts + 1,
                          last_error = NULL, next_retry_at = NULL,
                          updated_at = datetime('now')
         WHERE insilo_id = ?1",
        params![insilo_id, task_uid],
    )?;
    Ok(())
}

/// Remember the task this idea belongs to without marking it done yet.
pub fn set_task_uid(conn: &Connection, insilo_id: &str, task_uid: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE ideas SET task_uid = ?2, updated_at = datetime('now') WHERE insilo_id = ?1",
        params![insilo_id, task_uid],
    )?;
    Ok(())
}

/// Atomically claim up to `limit` due ideas (pending/failed → processing) and
/// return them. Prevents the background loop and the manual endpoint from
/// processing the same idea twice. A `processing` row stuck for >15 min
/// (crash/restart) is reclaimed.
pub fn claim_due_ideas(conn: &Connection, limit: i64) -> Result<Vec<IdeaRow>, rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    let ids: Vec<String> = {
        let mut stmt = tx.prepare(
            "SELECT insilo_id FROM ideas
             WHERE (status IN ('pending', 'failed')
                    AND (next_retry_at IS NULL OR next_retry_at <= datetime('now')))
                OR (status = 'processing' AND updated_at <= datetime('now', '-15 minutes'))
             ORDER BY id
             LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for id in &ids {
        tx.execute(
            "UPDATE ideas SET status = 'processing', updated_at = datetime('now') WHERE insilo_id = ?1",
            params![id],
        )?;
    }
    tx.commit()?;

    let mut out = Vec::with_capacity(ids.len());
    for id in &ids {
        let row = conn.query_row(
            "SELECT insilo_id, path, sha256, title, recorded_at, duration_min,
                    language, transcript_md, status, attempts, task_uid
             FROM ideas WHERE insilo_id = ?1",
            params![id],
            |r| {
                Ok(IdeaRow {
                    insilo_id: r.get(0)?,
                    path: r.get(1)?,
                    sha256: r.get(2)?,
                    title: r.get(3)?,
                    recorded_at: r.get(4)?,
                    duration_min: r.get(5)?,
                    language: r.get(6)?,
                    transcript_md: r.get(7)?,
                    status: r.get(8)?,
                    attempts: r.get(9)?,
                    task_uid: r.get(10)?,
                })
            },
        )?;
        out.push(row);
    }
    Ok(out)
}

/// Re-queue every idea for a fresh processing run (QA / manual re-run).
pub fn reset_all(conn: &Connection) -> Result<usize, rusqlite::Error> {
    conn.execute(
        "UPDATE ideas SET status = 'pending', attempts = 0, last_error = NULL,
                          next_retry_at = NULL, updated_at = datetime('now')",
        [],
    )
}

/// Record a failed attempt and schedule the next retry.
pub fn mark_failed(
    conn: &Connection,
    insilo_id: &str,
    error: &str,
    next_retry_at: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE ideas SET status = 'failed', attempts = attempts + 1,
                          last_error = ?2, next_retry_at = ?3,
                          updated_at = datetime('now')
         WHERE insilo_id = ?1",
        params![insilo_id, error, next_retry_at],
    )?;
    Ok(())
}

/// Retry without counting the attempt (temporary infrastructure outage, e.g. the
/// LLM is down). The idea keeps its low attempt count so a short outage never
/// triggers the fallback task.
pub fn mark_retry(
    conn: &Connection,
    insilo_id: &str,
    error: &str,
    next_retry_at: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE ideas SET status = 'failed', last_error = ?2, next_retry_at = ?3,
                          updated_at = datetime('now')
         WHERE insilo_id = ?1",
        params![insilo_id, error, next_retry_at],
    )?;
    Ok(())
}
