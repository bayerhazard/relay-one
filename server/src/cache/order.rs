//! Work-order for to-dos: the sequence a user should tackle tasks in.
//!
//! Rules (agreed with the product owner):
//! 1. **Dependencies are hard edges** — a task blocked by another never appears
//!    before its blocker, regardless of due date or priority.
//! 2. Among the *ready* tasks, a **bucket** decides first:
//!    overdue → today → next 7 days → later → no date (last).
//! 3. Then **priority** (1..5, `None` = lowest).
//! 4. Then the exact **due time** (a date-only task counts as 00:00, i.e. before
//!    timed tasks of the same day; no date = after everything dated).
//! 5. Then **alphabetically** (case-insensitive summary).
//! 6. Finally the row `id` (stable).
//!
//! Cycles are broken deterministically by always taking the best remaining task,
//! so the sort never stalls.

use std::collections::{HashMap, HashSet};

use crate::cache::todo::TodoRow;

/// Composite sort key; field order *is* the priority order.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    bucket: i32,
    prio: i64,
    due: i64,
    alpha: String,
    id: i64,
}

const BUCKET_OVERDUE: i32 = 0;
const BUCKET_TODAY: i32 = 1;
const BUCKET_SOON: i32 = 2;
const BUCKET_LATER: i32 = 3;
const BUCKET_NONE: i32 = 4;
/// `None` priority sorts after every set priority (1..5).
const PRIO_NONE: i64 = 6;

fn due_parts(r: &TodoRow) -> (i32, i64) {
    let today = chrono::Local::now().date_naive();
    let Some(s) = r.due_at.as_deref().filter(|s| !s.trim().is_empty()) else {
        return (BUCKET_NONE, i64::MAX);
    };
    let (day, ts) = if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        (dt.with_timezone(&chrono::Local).date_naive(), dt.timestamp())
    } else if let Ok(d) = chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d") {
        // Date-only: treat as start of day (before timed tasks).
        (d, i64::MIN)
    } else {
        return (BUCKET_NONE, i64::MAX);
    };
    let bucket = if day < today {
        BUCKET_OVERDUE
    } else if day == today {
        BUCKET_TODAY
    } else if day <= today + chrono::Duration::days(7) {
        BUCKET_SOON
    } else {
        BUCKET_LATER
    };
    (bucket, ts)
}

fn key(r: &TodoRow) -> Key {
    let (bucket, due) = due_parts(r);
    Key {
        bucket,
        prio: r.priority.unwrap_or(PRIO_NONE),
        due,
        alpha: r.summary.as_deref().unwrap_or("").trim().to_lowercase(),
        id: r.id,
    }
}

/// Return the UIDs of `rows` in work order.
pub fn work_order(rows: &[TodoRow]) -> Vec<String> {
    let by_uid: HashMap<&str, &TodoRow> = rows.iter().map(|r| (r.uid.as_str(), r)).collect();
    let keys: HashMap<&str, Key> = rows.iter().map(|r| (r.uid.as_str(), key(r))).collect();

    // indegree = number of *unsatisfied* blockers that are part of this list.
    let mut indeg: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
    for r in rows {
        let mut n = 0usize;
        for b in &r.dependencies {
            if b == &r.uid {
                continue;
            }
            if let Some(brow) = by_uid.get(b.as_str()) {
                if brow.status != "COMPLETED" {
                    n += 1;
                    dependents.entry(brow.uid.as_str()).or_default().push(r.uid.as_str());
                }
            }
        }
        indeg.insert(r.uid.as_str(), n);
    }

    let mut done: HashSet<&str> = HashSet::new();
    let mut out: Vec<String> = Vec::with_capacity(rows.len());
    while out.len() < rows.len() {
        let mut best: Option<&str> = None;
        for r in rows {
            let uid = r.uid.as_str();
            if done.contains(uid) || indeg[uid] != 0 {
                continue;
            }
            if best.is_none_or(|b| keys[uid] < keys[b]) {
                best = Some(uid);
            }
        }
        let pick = match best {
            Some(u) => u,
            None => {
                // Dependency cycle: take the best remaining task to break it.
                let mut fallback: Option<&str> = None;
                for r in rows {
                    let uid = r.uid.as_str();
                    if done.contains(uid) {
                        continue;
                    }
                    if fallback.is_none_or(|b| keys[uid] < keys[b]) {
                        fallback = Some(uid);
                    }
                }
                fallback.expect("rows is non-empty while loop runs")
            }
        };
        done.insert(pick);
        out.push(pick.to_string());
        if let Some(deps) = dependents.get(pick) {
            for d in deps {
                if let Some(v) = indeg.get_mut(d) {
                    if *v > 0 {
                        *v -= 1;
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, uid: &str, summary: &str) -> TodoRow {
        TodoRow {
            id,
            calendar_id: 1,
            uid: uid.to_string(),
            summary: Some(summary.to_string()),
            description: None,
            due_at: None,
            completed_at: None,
            status: "NEEDS-ACTION".to_string(),
            priority: None,
            project_id: None,
            parent_uid: None,
            labels: Vec::new(),
            rrule: None,
            sort_order: 0,
            due_has_time: false,
            dependencies: Vec::new(),
        }
    }

    fn day(offset: i64) -> String {
        let d = chrono::Local::now().date_naive() + chrono::Duration::days(offset);
        format!("{}T00:00:00+02:00", d.format("%Y-%m-%d"))
    }

    fn ids(order: &[String]) -> Vec<&str> {
        order.iter().map(|s| s.as_str()).collect()
    }

    #[test]
    fn faelligkeit_schlaegt_alpha() {
        let mut a = row(1, "a", "Zebra");
        a.due_at = Some(day(0)); // today
        let mut b = row(2, "b", "Alpha");
        b.due_at = Some(day(1)); // tomorrow
        let order = work_order(&[b, a]);
        assert_eq!(ids(&order), vec!["a", "b"], "heute vor morgen");
    }

    #[test]
    fn prioritaet_innerhalb_des_tages() {
        let mut low = row(1, "low", "A");
        low.due_at = Some(day(0));
        low.priority = Some(4);
        let mut high = row(2, "high", "Z");
        high.due_at = Some(day(0));
        high.priority = Some(1);
        let order = work_order(&[low, high]);
        assert_eq!(ids(&order), vec!["high", "low"], "P1 vor P4");
    }

    #[test]
    fn ohne_datum_zuletzt_trotz_hoher_prio() {
        let mut dated = row(1, "dated", "A");
        dated.due_at = Some(day(3));
        dated.priority = Some(5);
        let mut undated = row(2, "undated", "A");
        undated.priority = Some(1);
        let order = work_order(&[undated, dated]);
        assert_eq!(ids(&order), vec!["dated", "undated"], "fällig vor ohne Datum");
    }

    #[test]
    fn alpha_als_tiebreak() {
        let a = row(1, "a", "Zebra");
        let b = row(2, "b", "Apfel");
        let order = work_order(&[a, b]);
        assert_eq!(ids(&order), vec!["b", "a"], "alphabetisch");
    }

    #[test]
    fn abhaengigkeit_ist_harte_kante() {
        // "second" is due today and P1 but blocked by "first" (no date, P5).
        let mut first = row(1, "first", "Vorbereiten");
        first.priority = Some(5);
        let mut second = row(2, "second", "Senden");
        second.due_at = Some(day(0));
        second.priority = Some(1);
        second.dependencies = vec!["first".to_string()];
        let order = work_order(&[second, first]);
        assert_eq!(ids(&order), vec!["first", "second"], "Blocker zuerst");
    }

    #[test]
    fn erledigte_blocker_blockieren_nicht() {
        let mut blocker = row(1, "blocker", "Zulu");
        blocker.status = "COMPLETED".to_string();
        let mut blocked = row(2, "blocked", "Alpha");
        blocked.dependencies = vec!["blocker".to_string()];
        let order = work_order(&[blocked, blocker]);
        // The completed blocker satisfies the dependency, so the normal key
        // (alpha) decides — "blocked" (Alpha) leads despite the dependency.
        assert_eq!(order[0], "blocked");
    }

    #[test]
    fn zyklus_haengt_nicht() {
        let mut a = row(1, "a", "A");
        a.dependencies = vec!["b".to_string()];
        let mut b = row(2, "b", "B");
        b.dependencies = vec!["a".to_string()];
        let order = work_order(&[a, b]);
        assert_eq!(order.len(), 2, "beide werden ausgegeben");
        assert_eq!(ids(&order), vec!["a", "b"], "deterministisch per Alpha/Tiebreak");
    }

    #[test]
    fn unbekannter_blocker_wird_ignoriert() {
        let mut a = row(1, "a", "A");
        a.dependencies = vec!["ghost".to_string()];
        let order = work_order(&[a]);
        assert_eq!(ids(&order), vec!["a"]);
    }

    #[test]
    fn leer_und_stabil() {
        assert!(work_order(&[]).is_empty());
        // Equal keys -> stable by id.
        let a = row(1, "a", "X");
        let b = row(2, "b", "X");
        let order = work_order(&[b, a]);
        assert_eq!(ids(&order), vec!["a", "b"]);
    }
}
