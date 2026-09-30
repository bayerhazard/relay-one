//! Conservative tag policy for the Insilo idea workflow.
//!
//! A thematic tag only becomes "real" once it marks at least [`THRESHOLD`]
//! tasks. Until then a proposal is a hidden *candidate* and is not written to
//! any task. `Insilo` marks the origin of idea-derived tasks and is exempt.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::cache::todo;

/// Fixed origin label for idea-derived tasks (exempt from the threshold).
pub const ORIGIN_LABEL: &str = "Insilo";
/// Minimum number of tasks a thematic tag must cover to be applied/promoted.
pub const THRESHOLD: i64 = 5;

/// Lowercase + small synonym table, so "ML"/"ml", "KI"/"ai", "Test"/"test"
/// etc. collapse instead of fragmenting the vocabulary.
pub fn normalize_label(s: &str) -> String {
    let l = s.trim().to_lowercase();
    if l.is_empty() {
        return String::new();
    }
    let canonical = match l.as_str() {
        "idee" | "insilo" => return ORIGIN_LABEL.to_string(),
        "ai" | "ki" | "künstliche intelligenz" => "ki",
        "machine learning" | "maschinelles lernen" => "ml",
        "benchmarks" => "benchmark",
        "modellvergleich" | "vergleiche" => "vergleich",
        "analysen" => "analyse",
        "documentation" | "doku" | "dokumente" => "dokumentation",
        "research" => "recherche",
        "mobile" => "mobil",
        "tests" | "testen" => "test",
        other => other,
    };
    canonical.to_string()
}

/// Count of tasks per thematic label (origin excluded).
pub fn label_counts(conn: &Connection) -> Result<HashMap<String, i64>, String> {
    let rows = todo::list_todos(conn, None).map_err(|e| e.to_string())?;
    let mut counts: HashMap<String, i64> = HashMap::new();
    for r in rows {
        let mut seen: Vec<String> = Vec::new();
        for l in &r.labels {
            let n = normalize_label(l);
            if n.is_empty() || n == ORIGIN_LABEL || seen.contains(&n) {
                continue;
            }
            *counts.entry(n.clone()).or_insert(0) += 1;
            seen.push(n);
        }
    }
    Ok(counts)
}

/// Thematic labels that are "active" (cover >= THRESHOLD tasks).
pub fn active_labels(conn: &Connection) -> Result<HashMap<String, i64>, String> {
    let mut counts = label_counts(conn)?;
    counts.retain(|_, v| *v >= THRESHOLD);
    Ok(counts)
}

/// Record a hidden candidate (label proposal for a task).
pub fn add_candidate(conn: &Connection, label: &str, task_uid: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT OR IGNORE INTO label_candidates (label, task_uid, created_at) VALUES (?1, ?2, datetime('now'))",
        rusqlite::params![label, task_uid],
    )?;
    Ok(())
}

/// Distinct tasks that carry a candidate label.
pub fn candidate_tasks(conn: &Connection, label: &str) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT task_uid FROM label_candidates WHERE label = ?1")?;
    let rows = stmt
        .query_map(rusqlite::params![label], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn candidate_count(conn: &Connection, label: &str) -> Result<i64, rusqlite::Error> {
    conn.query_row(
        "SELECT COUNT(*) FROM label_candidates WHERE label = ?1",
        rusqlite::params![label],
        |r| r.get(0),
    )
}

pub fn remove_candidates(conn: &Connection, label: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "DELETE FROM label_candidates WHERE label = ?1",
        rusqlite::params![label],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ics_todo(uid: &str, labels: &[&str]) -> crate::dav::ics::IcsTodo {
        crate::dav::ics::IcsTodo {
            uid: uid.to_string(),
            url: String::new(),
            summary: Some("T".into()),
            description: None,
            due: None,
            completed: None,
            status: Some("NEEDS-ACTION".into()),
            priority: None,
            rrule: None,
            labels: labels.iter().map(|s| s.to_string()).collect(),
            parent_uid: None,
            project_id: None,
            due_has_time: false,
            raw: String::new(),
        }
    }

    fn seed_calendar(conn: &rusqlite::Connection) {
        conn.execute(
            "INSERT INTO calendars (id, url, display_name, description, color, last_sync_at) \
             VALUES (1, 'x', 'Arbeit', NULL, NULL, NULL)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn normalize_kanonisiert() {
        assert_eq!(normalize_label("ML"), "ml");
        assert_eq!(normalize_label("KI"), "ki");
        assert_eq!(normalize_label("ai"), "ki");
        assert_eq!(normalize_label("Idee"), ORIGIN_LABEL);
        assert_eq!(normalize_label("Test"), "test");
        assert_eq!(normalize_label("  "), "");
    }

    #[test]
    fn label_counts_zaehlt_ohne_herkunft() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::cache::db::init_db(&conn).unwrap();
        seed_calendar(&conn);
        crate::cache::todo::upsert_todo(&conn, 1, &ics_todo("a", &["ml", "Insilo"])).unwrap();
        crate::cache::todo::upsert_todo(&conn, 1, &ics_todo("b", &["ML"])).unwrap();
        let c = label_counts(&conn).unwrap();
        assert_eq!(c.get("ml"), Some(&2));
        assert!(!c.contains_key(ORIGIN_LABEL));
    }

    #[test]
    fn kandidaten_werden_dedupliziert() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::cache::db::init_db(&conn).unwrap();
        add_candidate(&conn, "vertrieb", "u1").unwrap();
        add_candidate(&conn, "vertrieb", "u2").unwrap();
        add_candidate(&conn, "vertrieb", "u2").unwrap();
        assert_eq!(candidate_count(&conn, "vertrieb").unwrap(), 2);
        assert_eq!(candidate_tasks(&conn, "vertrieb").unwrap().len(), 2);
        remove_candidates(&conn, "vertrieb").unwrap();
        assert_eq!(candidate_count(&conn, "vertrieb").unwrap(), 0);
    }

    #[test]
    fn active_erst_ab_schwelle() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::cache::db::init_db(&conn).unwrap();
        seed_calendar(&conn);
        for i in 0..(THRESHOLD - 1) {
            crate::cache::todo::upsert_todo(&conn, 1, &ics_todo(&format!("u{i}"), &["vertrieb"])).unwrap();
        }
        assert!(active_labels(&conn).unwrap().is_empty(), "unter der Schwelle nicht aktiv");
        crate::cache::todo::upsert_todo(&conn, 1, &ics_todo("ux", &["vertrieb"])).unwrap();
        assert_eq!(active_labels(&conn).unwrap().get("vertrieb"), Some(&THRESHOLD));
    }
}
