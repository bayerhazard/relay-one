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
    pub project_id: Option<i64>,
    pub parent_uid: Option<String>,
    pub labels: Vec<String>,
    pub rrule: Option<String>,
    pub sort_order: i64,
    pub due_has_time: bool,
    /// Blocking task UIDs (`RELATED-TO;RELTYPE=DEPENDS-ON`).
    pub dependencies: Vec<String>,
}

fn row_to_todo(row: &rusqlite::Row) -> rusqlite::Result<TodoRow> {
    let labels_raw: Option<String> = row.get("labels")?;
    let labels: Vec<String> = labels_raw
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
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
        project_id: row.get("project_id")?,
        parent_uid: row.get("parent_uid")?,
        labels,
        rrule: row.get("rrule")?,
        sort_order: row.get("sort_order").unwrap_or(0),
        due_has_time: row.get::<_, Option<i64>>("due_has_time")?.unwrap_or(0) != 0,
        dependencies: Vec::new(),
    })
}

/// Load the `blocked_uid -> [blocker_uid]` map for the given task UIDs.
fn deps_for(conn: &Connection, uids: &[String]) -> Result<std::collections::HashMap<String, Vec<String>>, String> {
    use std::collections::HashMap;
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    if uids.is_empty() {
        return Ok(map);
    }
    let placeholders = std::iter::repeat("?").take(uids.len()).collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT blocked_uid, blocker_uid FROM todo_deps WHERE blocked_uid IN ({placeholders}) ORDER BY blocker_uid"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let params: Vec<&dyn rusqlite::types::ToSql> =
        uids.iter().map(|u| u as &dyn rusqlite::types::ToSql).collect();
    let rows = stmt
        .query_map(params.as_slice(), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    for r in rows {
        let (blocked, blocker) = r.map_err(|e| e.to_string())?;
        map.entry(blocked).or_default().push(blocker);
    }
    Ok(map)
}

/// Attach dependencies from `todo_deps` to a set of rows.
fn attach_deps(conn: &Connection, rows: &mut [TodoRow]) -> Result<(), String> {
    let uids: Vec<String> = rows.iter().map(|r| r.uid.clone()).collect();
    let mut map = deps_for(conn, &uids)?;
    for r in rows.iter_mut() {
        if let Some(deps) = map.remove(&r.uid) {
            r.dependencies = deps;
        }
    }
    Ok(())
}

const COLS: &str = "id, calendar_id, uid, summary, description, due_at, completed_at, status, \
                    priority, project_id, parent_uid, labels, rrule, sort_order, due_has_time";

/// List to-dos, optionally filtered: `completed` = Some(true) only done,
/// Some(false) only open, None = all. Ordered by manual order, then due date.
pub fn list_todos(conn: &Connection, completed: Option<bool>) -> Result<Vec<TodoRow>, String> {
    let sql = match completed {
        Some(true) => format!(
            "SELECT {COLS} FROM todos WHERE status = 'COMPLETED' \
             ORDER BY sort_order, due_at IS NULL, due_at, id"
        ),
        Some(false) => format!(
            "SELECT {COLS} FROM todos WHERE status != 'COMPLETED' \
             ORDER BY sort_order, due_at IS NULL, due_at, id"
        ),
        None => format!(
            "SELECT {COLS} FROM todos ORDER BY sort_order, due_at IS NULL, due_at, id"
        ),
    };
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], row_to_todo).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    attach_deps(conn, &mut out)?;
    Ok(out)
}

/// Fetch a single to-do by UID.
pub fn find_todo(conn: &Connection, uid: &str) -> Result<Option<TodoRow>, String> {
    let sql = format!("SELECT {COLS} FROM todos WHERE uid = ?1");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map(rusqlite::params![uid], row_to_todo)
        .map_err(|e| e.to_string())?;
    match rows.next() {
        Some(r) => {
            let mut row: TodoRow = r.map_err(|e| e.to_string())?;
            row.dependencies = deps_for(conn, std::slice::from_ref(&row.uid))?
                .remove(&row.uid)
                .unwrap_or_default();
            Ok(Some(row))
        }
        None => Ok(None),
    }
}

fn labels_to_json(labels: &[String]) -> String {
    serde_json::to_string(labels).unwrap_or_else(|_| "[]".to_string())
}

/// Upsert a to-do into the local cache. Server-provided task fields (labels,
/// rrule, parent, due_has_time, project, section) are preserved when the
/// incoming sync carries them; project_id is only set when the caller passes a
/// concrete value (else the existing one is kept).
pub fn upsert_todo(conn: &Connection, calendar_id: i64, t: &IcsTodo) -> Result<(), String> {
    let labels = labels_to_json(&t.labels);
    conn.execute(
        "INSERT INTO todos (calendar_id, uid, url, summary, description, due_at, completed_at, status,
                            priority, project_id, parent_uid, labels, rrule, due_has_time, ics_raw, synced_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, datetime('now'))
         ON CONFLICT(calendar_id, uid) DO UPDATE SET
            url = excluded.url,
            summary = excluded.summary,
            description = excluded.description,
            due_at = excluded.due_at,
            completed_at = excluded.completed_at,
            status = excluded.status,
            priority = excluded.priority,
            project_id = COALESCE(excluded.project_id, todos.project_id),
            parent_uid = excluded.parent_uid,
            labels = excluded.labels,
            rrule = excluded.rrule,
            due_has_time = excluded.due_has_time,
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
            t.project_id,
            t.parent_uid,
            labels,
            t.rrule,
            t.due_has_time as i64,
            t.raw,
        ],
    )
    .map_err(|e| e.to_string())?;
    reconcile_deps(conn, &t.uid, &t.dependencies)?;
    Ok(())
}

/// Replace the dependency set (`blocked by`) of a task. Self-references are
/// dropped. Non-existent blocker UIDs are kept — the task may not be synced yet.
fn reconcile_deps(conn: &Connection, uid: &str, deps: &[String]) -> Result<(), String> {
    conn.execute("DELETE FROM todo_deps WHERE blocked_uid = ?1", rusqlite::params![uid])
        .map_err(|e| e.to_string())?;
    if !deps.is_empty() {
        let mut stmt = conn
            .prepare("INSERT OR IGNORE INTO todo_deps (blocked_uid, blocker_uid) VALUES (?1, ?2)")
            .map_err(|e| e.to_string())?;
        for dep in deps {
            let dep = dep.trim();
            if dep.is_empty() || dep == uid {
                continue;
            }
            stmt.execute(rusqlite::params![uid, dep]).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Set the dependency list of a task by UID (used by the API PATCH).
pub fn set_dependencies(conn: &Connection, uid: &str, deps: &[String]) -> Result<(), String> {
    reconcile_deps(conn, uid, deps)
}

/// Fields the API accepts on PATCH. `None` = leave unchanged.
#[derive(Debug, Default, Clone)]
pub struct TodoUpdate {
    pub summary: Option<String>,
    pub description: Option<Option<String>>,
    pub due_at: Option<Option<String>>,
    pub due_has_time: Option<bool>,
    pub priority: Option<Option<i64>>,
    pub labels: Option<Vec<String>>,
    pub rrule: Option<Option<String>>,
    pub project_id: Option<Option<i64>>,
    /// Replacement dependency list (`blocked by`), applied after the columns.
    pub dependencies: Option<Vec<String>>,
}

/// Apply a partial update. Returns the number of rows touched.
pub fn update_todo(conn: &Connection, uid: &str, u: &TodoUpdate) -> Result<usize, String> {
    let mut sets: Vec<&str> = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(v) = &u.summary {
        sets.push("summary = ?");
        params.push(Box::new(v.clone()));
    }
    if let Some(v) = &u.description {
        sets.push("description = ?");
        params.push(Box::new(v.clone()));
    }
    if let Some(v) = &u.due_at {
        sets.push("due_at = ?");
        params.push(Box::new(v.clone()));
    }
    if let Some(v) = u.due_has_time {
        sets.push("due_has_time = ?");
        params.push(Box::new(v as i64));
    }
    if let Some(v) = &u.priority {
        sets.push("priority = ?");
        params.push(Box::new(*v));
    }
    if let Some(v) = &u.labels {
        sets.push("labels = ?");
        params.push(Box::new(labels_to_json(v)));
    }
    if let Some(v) = &u.rrule {
        sets.push("rrule = ?");
        params.push(Box::new(v.clone()));
    }
    if let Some(v) = &u.project_id {
        sets.push("project_id = ?");
        params.push(Box::new(*v));
    }
    if sets.is_empty() && u.dependencies.is_none() {
        return Ok(0);
    }
    let mut touched = 0usize;
    if !sets.is_empty() {
        sets.push("updated_at = datetime('now')");
        let sql = format!("UPDATE todos SET {} WHERE uid = ?", sets.join(", "));
        params.push(Box::new(uid.to_string()));
        let refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        touched = conn.execute(&sql, refs.as_slice()).map_err(|e| e.to_string())?;
    }
    if let Some(deps) = &u.dependencies {
        set_dependencies(conn, uid, deps)?;
        touched = touched.max(1);
    }
    Ok(touched)
}

/// Mark a to-do completed (or reopen) by UID.
pub fn set_completed(conn: &Connection, uid: &str, completed: bool) -> Result<(), String> {
    if completed {
        conn.execute(
            "UPDATE todos SET status = 'COMPLETED', completed_at = datetime('now'), updated_at = datetime('now') WHERE uid = ?1",
            rusqlite::params![uid],
        )
    } else {
        conn.execute(
            "UPDATE todos SET status = 'NEEDS-ACTION', completed_at = NULL, updated_at = datetime('now') WHERE uid = ?1",
            rusqlite::params![uid],
        )
    }
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Persist a new manual order: `(uid, sort_order)` pairs applied in one txn.
pub fn reorder_todos(conn: &mut Connection, order: &[(String, i64)]) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    {
        let mut stmt = tx
            .prepare("UPDATE todos SET sort_order = ?2 WHERE uid = ?1")
            .map_err(|e| e.to_string())?;
        for (uid, pos) in order {
            stmt.execute(rusqlite::params![uid, pos]).map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

/// Delete a to-do by UID (and its dependency links).
pub fn delete_todo(conn: &Connection, uid: &str) -> Result<(), String> {
    conn.execute("DELETE FROM todos WHERE uid = ?1", rusqlite::params![uid])
        .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM todo_deps WHERE blocked_uid = ?1 OR blocker_uid = ?1",
        rusqlite::params![uid],
    )
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
            rrule: None,
            labels: Vec::new(),
            parent_uid: None,
            dependencies: Vec::new(),
            project_id: None,
            due_has_time: true,
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

    #[test]
    fn test_extended_fields_roundtrip() {
        let conn = test_db();
        let mut t = test_todo("t3", "Wochenbericht", "NEEDS-ACTION");
        t.labels = vec!["Firma".into(), "Reporting".into()];
        t.rrule = Some("FREQ=WEEKLY;BYDAY=FR".into());
        t.parent_uid = Some("parent-1".into());
        upsert_todo(&conn, 1, &t).unwrap();

        let row = find_todo(&conn, "t3").unwrap().unwrap();
        assert_eq!(row.labels, vec!["Firma", "Reporting"]);
        assert_eq!(row.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=FR"));
        assert_eq!(row.parent_uid.as_deref(), Some("parent-1"));
        assert!(row.due_has_time);
    }

    #[test]
    fn test_dependencies_reconcile_and_cascade() {
        let conn = test_db();
        let a = test_todo("a", "Blocker", "NEEDS-ACTION");
        let mut b = test_todo("b", "Blocked", "NEEDS-ACTION");
        b.dependencies = vec!["a".into(), "b".into(), "ghost".into()]; // self dropped
        upsert_todo(&conn, 1, &a).unwrap();
        upsert_todo(&conn, 1, &b).unwrap();

        let row = find_todo(&conn, "b").unwrap().unwrap();
        assert_eq!(row.dependencies, vec!["a", "ghost"], "self reference dropped");

        // Deleting the blocker removes the link (ghost stays: unknown UID).
        delete_todo(&conn, "a").unwrap();
        assert_eq!(find_todo(&conn, "b").unwrap().unwrap().dependencies, vec!["ghost"]);

        // Re-upsert with a different set replaces the old one.
        let mut b2 = test_todo("b", "Blocked", "NEEDS-ACTION");
        b2.dependencies = vec!["y".into()];
        upsert_todo(&conn, 1, &b2).unwrap();
        assert_eq!(find_todo(&conn, "b").unwrap().unwrap().dependencies, vec!["y"]);
    }

    #[test]
    fn test_update_todo_partial_fields() {
        let conn = test_db();
        upsert_todo(&conn, 1, &test_todo("t4", "Alt", "NEEDS-ACTION")).unwrap();

        let u = TodoUpdate {
            summary: Some("Neu".into()),
            labels: Some(vec!["X".into()]),
            priority: Some(Some(1)),
            rrule: Some(Some("FREQ=DAILY".into())),
            ..Default::default()
        };
        let n = update_todo(&conn, "t4", &u).unwrap();
        assert_eq!(n, 1);

        let row = find_todo(&conn, "t4").unwrap().unwrap();
        assert_eq!(row.summary.as_deref(), Some("Neu"));
        assert_eq!(row.labels, vec!["X"]);
        assert_eq!(row.priority, Some(1));
        assert_eq!(row.rrule.as_deref(), Some("FREQ=DAILY"));
    }

    #[test]
    fn test_update_todo_clears_optional_fields() {
        let conn = test_db();
        let mut t = test_todo("t5", "Mit Prio", "NEEDS-ACTION");
        t.priority = Some(1);
        upsert_todo(&conn, 1, &t).unwrap();

        let u = TodoUpdate {
            priority: Some(None),
            due_at: Some(None),
            ..Default::default()
        };
        update_todo(&conn, "t5", &u).unwrap();
        let row = find_todo(&conn, "t5").unwrap().unwrap();
        assert_eq!(row.priority, None);
        assert_eq!(row.due_at, None);
    }

    #[test]
    fn test_reorder_todos() {
        let mut conn = test_db();
        upsert_todo(&conn, 1, &test_todo("a", "A", "NEEDS-ACTION")).unwrap();
        upsert_todo(&conn, 1, &test_todo("b", "B", "NEEDS-ACTION")).unwrap();

        reorder_todos(&mut conn, &[("b".into(), 0), ("a".into(), 1)]).unwrap();
        let rows = list_todos(&conn, None).unwrap();
        assert_eq!(rows[0].uid, "b");
        assert_eq!(rows[1].uid, "a");
    }

    #[test]
    fn test_project_id_survives_sync_without_project() {
        let conn = test_db();
        let mut t = test_todo("p1", "Projekt-Task", "NEEDS-ACTION");
        t.project_id = Some(1);
        upsert_todo(&conn, 1, &t).unwrap();

        // A later sync that carries no project must not wipe the assignment.
        let mut sync = test_todo("p1", "Projekt-Task", "NEEDS-ACTION");
        sync.project_id = None;
        upsert_todo(&conn, 1, &sync).unwrap();

        let row = find_todo(&conn, "p1").unwrap().unwrap();
        assert_eq!(row.project_id, Some(1));
    }
}
