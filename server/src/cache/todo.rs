//! Local to-do (VTODO) cache (SQLite).

use rusqlite::Connection;

use crate::dav::ics::IcsTodo;

/// A to-do row as returned to the API.
#[derive(serde::Serialize)]
pub struct TodoRow {
    pub id: i64,
    pub calendar_id: i64,
    pub uid: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub due_at: Option<String>,
    pub completed_at: Option<String>,
    pub status: String,
    pub priority: Option<i64>,
}

fn row_to_todo(row: &rusqlite::Row) -> rusqlite::Result<TodoRow> {
    Ok(TodoRow {
        id: row.get("id")?,
        calendar_id: row.get("calendar_id")?,
        uid: row.get("uid")?,
        summary: row.get("summary")?,
        description: row.get("description")?,
        due_at: row.get("due_at")?,
        completed_at: row.get("completed_at")?,
        status: row.get("status")?,
        priority: row.get("priority")?,
    })
}

const COLS: &str = "id, calendar_id, uid, summary, description, due_at, completed_at, status, priority";

/// List to-dos, optionally filtered: `completed` = Some(true) only done,
/// Some(false) only open, None = all.
pub fn list_todos(conn: &Connection, completed: Option<bool>) -> Result<Vec<TodoRow>, String> {
    let sql = match completed {
        Some(true) => format!("SELECT {COLS} FROM todos WHERE status = 'COMPLETED' ORDER BY due_at IS NULL, due_at, id"),
        Some(false) => format!("SELECT {COLS} FROM todos WHERE status != 'COMPLETED' ORDER BY due_at IS NULL, due_at, id"),
        None => format!("SELECT {COLS} FROM todos ORDER BY due_at IS NULL, due_at, id"),
    };
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], row_to_todo).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Upsert a to-do into the local cache.
pub fn upsert_todo(conn: &Connection, calendar_id: i64, t: &IcsTodo) -> Result<(), String> {
    conn.execute(
        "INSERT INTO todos (calendar_id, uid, url, summary, description, due_at, completed_at, status, priority, ics_raw, synced_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))
         ON CONFLICT(calendar_id, uid) DO UPDATE SET
            url = excluded.url,
            summary = excluded.summary,
            description = excluded.description,
            due_at = excluded.due_at,
            completed_at = excluded.completed_at,
            status = excluded.status,
            priority = excluded.priority,
            ics_raw = excluded.ics_raw,
            synced_at = datetime('now')",
        rusqlite::params![
            calendar_id,
            t.uid,
            t.url,
            t.summary,
            t.description,
            t.due,
            t.completed,
            t.status.clone().unwrap_or_else(|| "NEEDS-ACTION".to_string()),
            t.priority,
            t.raw,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Mark a to-do completed (or reopen) by UID.
pub fn set_completed(conn: &Connection, uid: &str, completed: bool) -> Result<(), String> {
    if completed {
        conn.execute(
            "UPDATE todos SET status = 'COMPLETED', completed_at = datetime('now') WHERE uid = ?1",
            rusqlite::params![uid],
        )
    } else {
        conn.execute(
            "UPDATE todos SET status = 'NEEDS-ACTION', completed_at = NULL WHERE uid = ?1",
            rusqlite::params![uid],
        )
    }
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete a to-do by UID.
pub fn delete_todo(conn: &Connection, uid: &str) -> Result<(), String> {
    conn.execute("DELETE FROM todos WHERE uid = ?1", rusqlite::params![uid])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::db::init_db;
    use rusqlite::Connection as C;

    fn test_db() -> C {
        let conn = C::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        // The todos table references calendars(id); insert a calendar first.
        conn.execute(
            "INSERT INTO calendars (id, url, display_name, color) VALUES (1, 'http://x', 'Test', '#000')",
            [],
        )
        .unwrap();
        conn
    }

    fn test_todo(uid: &str, summary: &str, status: &str) -> IcsTodo {
        IcsTodo {
            uid: uid.to_string(),
            url: format!("http://x/{uid}.ics"),
            summary: Some(summary.to_string()),
            description: None,
            due: Some("2026-09-01T09:00:00Z".to_string()),
            completed: None,
            status: Some(status.to_string()),
            priority: None,
            raw: "BEGIN:VCALENDAR\nEND:VCALENDAR".to_string(),
        }
    }

    #[test]
    fn test_upsert_and_list() {
        let conn = test_db();
        upsert_todo(&conn, 1, &test_todo("t1", "Einkaufen", "NEEDS-ACTION")).unwrap();
        upsert_todo(&conn, 1, &test_todo("t2", "Mailen", "COMPLETED")).unwrap();

        let all = list_todos(&conn, None).unwrap();
        assert_eq!(all.len(), 2);
        let open = list_todos(&conn, Some(false)).unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].uid, "t1");
        let done = list_todos(&conn, Some(true)).unwrap();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].uid, "t2");
    }

    #[test]
    fn test_set_completed() {
        let conn = test_db();
        upsert_todo(&conn, 1, &test_todo("t1", "Einkaufen", "NEEDS-ACTION")).unwrap();
        set_completed(&conn, "t1", true).unwrap();
        let done = list_todos(&conn, Some(true)).unwrap();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].status, "COMPLETED");
        // Reopen.
        set_completed(&conn, "t1", false).unwrap();
        assert!(list_todos(&conn, Some(true)).unwrap().is_empty());
    }

    #[test]
    fn test_delete() {
        let conn = test_db();
        upsert_todo(&conn, 1, &test_todo("t1", "Einkaufen", "NEEDS-ACTION")).unwrap();
        delete_todo(&conn, "t1").unwrap();
        assert!(list_todos(&conn, None).unwrap().is_empty());
    }
}
