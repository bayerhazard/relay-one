//! ActionPlan lifecycle (Concept §5.3). Plans are persisted server-side so a
//! confirmation can come from chat, mail followups, or notifications;
//! multi-step chains need sequence safety; audit needs the plan. A plan holds
//! exactly the card formats the frontend renders.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::tools::PreparedCard;

/// Plan TTL: pending plans expire after 15 minutes (Concept §5.3).
pub const PLAN_TTL_MINUTES: i64 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanStatus {
    Pending,
    Executed,
    Cancelled,
    Expired,
    Failed,
}

impl PlanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanStatus::Pending => "pending",
            PlanStatus::Executed => "executed",
            PlanStatus::Cancelled => "cancelled",
            PlanStatus::Expired => "expired",
            PlanStatus::Failed => "failed",
        }
    }
    fn from_str(s: &str) -> Self {
        match s {
            "executed" => PlanStatus::Executed,
            "cancelled" => PlanStatus::Cancelled,
            "expired" => PlanStatus::Expired,
            "failed" => PlanStatus::Failed,
            _ => PlanStatus::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionPlan {
    pub id: String,
    pub session_id: Option<String>,
    /// 'chat' | 'mail_followup'
    pub origin: String,
    pub source_message_id: Option<i64>,
    pub status: PlanStatus,
    /// Each step is a card (tool + tier + display rows + prepared request).
    pub steps: Vec<PreparedCard>,
    pub created_at: String,
    pub expires_at: String,
    pub executed_at: Option<String>,
    pub result_json: Option<String>,
}

/// `pl-` + 8 hex chars (first 8 of a v4 UUID).
pub fn new_plan_id() -> String {
    let u = uuid::Uuid::new_v4().simple().to_string();
    format!("pl-{}", &u[..8])
}

/// Build a plan from a set of cards (the agent loop's collected Write/External
/// outcomes). Assigns id + TTL.
pub fn build_plan(
    session_id: Option<String>,
    origin: &str,
    source_message_id: Option<i64>,
    cards: Vec<PreparedCard>,
) -> ActionPlan {
    let now = chrono::Utc::now();
    let expires = now + chrono::Duration::minutes(PLAN_TTL_MINUTES);
    ActionPlan {
        id: new_plan_id(),
        session_id,
        origin: origin.to_string(),
        source_message_id,
        status: PlanStatus::Pending,
        steps: cards,
        created_at: now.to_rfc3339(),
        expires_at: expires.to_rfc3339(),
        executed_at: None,
        result_json: None,
    }
}

pub fn create_plan(conn: &Connection, plan: &ActionPlan) -> Result<(), String> {
    let steps_json = serde_json::to_string(&plan.steps).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO ai_action_plans
         (id, session_id, origin, source_message_id, status, steps_json, created_at, expires_at, executed_at, result_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            plan.id,
            plan.session_id,
            plan.origin,
            plan.source_message_id,
            plan.status.as_str(),
            steps_json,
            plan.created_at,
            plan.expires_at,
            plan.executed_at,
            plan.result_json,
        ],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub fn get_plan(conn: &Connection, id: &str) -> Result<Option<ActionPlan>, String> {
    let row = conn
        .query_row(
            "SELECT id, session_id, origin, source_message_id, status, steps_json, created_at, expires_at, executed_at, result_json
             FROM ai_action_plans WHERE id = ?1",
            rusqlite::params![id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, Option<String>>(9)?,
                ))
            },
        )
        .ok();
    let Some(r) = row else {
        return Ok(None);
    };
    let steps: Vec<PreparedCard> = serde_json::from_str(&r.5).map_err(|e| e.to_string())?;
    Ok(Some(ActionPlan {
        id: r.0,
        session_id: r.1,
        origin: r.2,
        source_message_id: r.3,
        status: PlanStatus::from_str(&r.4),
        steps,
        created_at: r.6,
        expires_at: r.7,
        executed_at: r.8,
        result_json: r.9,
    }))
}

/// Set a plan's status (and, for executed, the executed_at timestamp).
pub fn set_status(conn: &Connection, id: &str, status: PlanStatus) -> Result<(), String> {
    let executed_at = if status == PlanStatus::Executed {
        Some(chrono::Utc::now().to_rfc3339())
    } else {
        None
    };
    conn.execute(
        "UPDATE ai_action_plans SET status = ?1, executed_at = COALESCE(?2, executed_at) WHERE id = ?3",
        rusqlite::params![status.as_str(), executed_at, id],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Cancel a pending plan (user discarded the card).
pub fn cancel_plan(conn: &Connection, id: &str) -> Result<bool, String> {
    let n = conn
        .execute(
            "UPDATE ai_action_plans SET status = 'cancelled' WHERE id = ?1 AND status = 'pending'",
            rusqlite::params![id],
        )
        .map_err(|e| e.to_string())?;
    Ok(n > 0)
}

/// Expire all pending plans whose `expires_at` is in the past. Returns the
/// number expired. Called on the next agent call and on confirm attempts
/// (Concept §5.3).
pub fn expire_pending(conn: &Connection) -> Result<usize, String> {
    let n = conn
        .execute(
            "UPDATE ai_action_plans SET status = 'expired'
             WHERE status = 'pending' AND expires_at < datetime('now', 'utc')",
            [],
        )
        .map_err(|e| e.to_string())?;
    Ok(n)
}

/// A pending plan is expired if its `expires_at` is in the past.
pub fn is_expired(plan: &ActionPlan) -> bool {
    let Ok(exp) = chrono::DateTime::parse_from_rfc3339(&plan.expires_at) else {
        return false;
    };
    chrono::Utc::now() > exp.with_timezone(&chrono::Utc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute(
            "CREATE TABLE ai_action_plans (
                id TEXT PRIMARY KEY, session_id TEXT, origin TEXT NOT NULL,
                source_message_id INTEGER, status TEXT NOT NULL, steps_json TEXT NOT NULL,
                created_at TEXT NOT NULL, expires_at TEXT NOT NULL, executed_at TEXT, result_json TEXT)",
            [],
        )
        .unwrap();
        c
    }

    fn sample_card() -> PreparedCard {
        PreparedCard {
            tool: "tasks_create".into(),
            tier: super::super::tools::Tier::Write,
            title: "Aufgabe anlegen".into(),
            rows: vec![("Titel".into(), "Rückruf".into())],
            request: super::super::tools::PreparedRequest {
                method: "POST".into(),
                path: "/api/v1/todos".into(),
                body: serde_json::json!({ "summary": "Rückruf" }),
            },
            danach: "/tasks".into(),
        }
    }

    #[test]
    fn build_and_persist() {
        let c = mem();
        let plan = build_plan(Some("s1".into()), "chat", None, vec![sample_card()]);
        assert!(plan.id.starts_with("pl-"));
        assert_eq!(plan.status, PlanStatus::Pending);
        create_plan(&c, &plan).unwrap();
        let got = get_plan(&c, &plan.id).unwrap().unwrap();
        assert_eq!(got.steps.len(), 1);
        assert_eq!(got.steps[0].tool, "tasks_create");
        assert_eq!(got.origin, "chat");
    }

    #[test]
    fn cancel_pending_only() {
        let c = mem();
        let plan = build_plan(None, "chat", None, vec![sample_card()]);
        create_plan(&c, &plan).unwrap();
        assert!(cancel_plan(&c, &plan.id).unwrap());
        assert!(!cancel_plan(&c, &plan.id).unwrap(), "second cancel is a no-op");
    }

    #[test]
    fn expire_pending_marks_old() {
        let c = mem();
        let mut plan = build_plan(None, "chat", None, vec![sample_card()]);
        // Force the expiry into the past.
        plan.expires_at = "2000-01-01T00:00:00Z".into();
        create_plan(&c, &plan).unwrap();
        let n = expire_pending(&c).unwrap();
        assert_eq!(n, 1);
        let got = get_plan(&c, &plan.id).unwrap().unwrap();
        assert_eq!(got.status, PlanStatus::Expired);
    }

    #[test]
    fn is_expired_detects_past() {
        let mut plan = build_plan(None, "chat", None, vec![sample_card()]);
        assert!(!is_expired(&plan));
        plan.expires_at = "2000-01-01T00:00:00Z".into();
        assert!(is_expired(&plan));
    }
}
