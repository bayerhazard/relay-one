//! To-do tools (Concept §5.1). Reads query the local cache; writes return a
//! `PreparedCard`.

use std::pin::Pin;

use super::{obj_schema, PreparedCard, PreparedRequest, ToolCtx, ToolDef, ToolOutcome, Tier};
use crate::db::with_db;

pub fn tools(locale: &str) -> Vec<ToolDef> {
    let de = locale == "de";
    vec![
        ToolDef::new(
            "tasks_list",
            Tier::Read,
            if de { "Listet Aufgaben (offen oder alle)." } else { "List to-dos (open or all)."},
            obj_schema(
                &[("open_only", "boolean", "Nur offene Aufgaben (Default true)", None)],
                &[],
            ),
            tasks_list,
        ),
        ToolDef::new(
            "tasks_create",
            Tier::Write,
            if de {
                "Legt eine Aufgabe an. Erzeugt eine Bestätigungskarte — es wird noch nichts gespeichert."
            } else {
                "Create a to-do. Produces a confirmation card — nothing is saved yet."
            },
            obj_schema(
                &[
                    ("summary", "string", "Titel der Aufgabe", None),
                    ("due", "string", "Fällig (RFC 3339, z. B. 2026-09-11T09:00:00Z)", None),
                    ("priority", "integer", "Priorität 1 (hoch) – 9 (niedrig)", None),
                ],
                &["summary"],
            ),
            tasks_create,
        ),
        ToolDef::new(
            "tasks_toggle",
            Tier::Write,
            if de {
                "Markiert eine Aufgabe als erledigt/offen. Erzeugt eine Bestätigungskarte."
            } else {
                "Mark a to-do done/open. Produces a confirmation card."
            },
            obj_schema(
                &[
                    ("id", "string", "Aufgaben-ID (uid, aus tasks_list)", None),
                    ("completed", "boolean", "true = erledigt, false = offen", None),
                ],
                &["id", "completed"],
            ),
            tasks_toggle,
        ),
        ToolDef::new(
            "tasks_delete",
            Tier::External,
            if de {
                "Löscht eine Aufgabe. Erzeugt eine Karte mit doppelter Bestätigung (unwiderruflich)."
            } else {
                "Delete a to-do. Produces a card with double confirmation (irreversible)."
            },
            obj_schema(&[("id", "string", "Aufgaben-ID (uid, aus tasks_list)", None)], &["id"]),
            tasks_delete,
        ),
    ]
}

fn tasks_list(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let open_only = args.get("open_only").and_then(|v| v.as_bool()).unwrap_or(true);
        let rows = with_db(&ctx.state, |conn| {
            crate::cache::todo::list_todos(conn, if open_only { Some(false) } else { None }).map_err(|e| e.to_string())
        })?;
        let mut known = ctx.known_ids.lock().await;
        let mut out = Vec::with_capacity(rows.len());
        for t in &rows {
            let id = t.uid.clone();
            let label = t.summary.clone().unwrap_or_else(|| id.clone());
            known.insert(id.clone(), label.clone());
            out.push(serde_json::json!({
                "id": id,
                "summary": t.summary,
                "due": t.due_at,
                "completed": t.status == "COMPLETED",
                "priority": t.priority,
            }));
        }
        Ok(ToolOutcome::Data(serde_json::json!({ "count": out.len(), "tasks": out })))
    })
}

fn tasks_create(_ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let summary = args.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if summary.trim().is_empty() {
            return Ok(ToolOutcome::Nachfrage("Bitte einen Titel angeben.".into()));
        }
        let due = args.get("due").and_then(|v| v.as_str()).map(str::to_string);
        let priority = args.get("priority").and_then(|v| v.as_i64());
        let body = serde_json::json!({ "summary": summary, "due": due, "priority": priority });
        let mut rows = vec![("Titel".into(), summary.clone())];
        if let Some(d) = &due {
            rows.push(("Fällig".into(), d.clone()));
        }
        Ok(ToolOutcome::Card(PreparedCard {
            tool: "tasks_create".into(),
            tier: Tier::Write,
            title: "Aufgabe anlegen".into(),
            rows,
            request: PreparedRequest {
                method: "POST".into(),
                path: "/api/v1/todos".into(),
                body,
            },
            danach: "/tasks".into(),
        }))
    })
}

fn tasks_toggle(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let completed = args.get("completed").and_then(|v| v.as_bool()).unwrap_or(true);
        if id.is_empty() {
            return Ok(ToolOutcome::Nachfrage("Aufgaben-ID fehlt.".into()));
        }
        let (known, label) = {
            let k = ctx.known_ids.lock().await;
            (k.contains_key(id), k.get(id).cloned())
        };
        if !known {
            return Ok(ToolOutcome::Nachfrage("Diese Aufgaben-ID ist mir nicht bekannt — zuerst tasks_list aufrufen.".into()));
        }
        let body = serde_json::json!({ "completed": completed });
        Ok(ToolOutcome::Card(PreparedCard {
            tool: "tasks_toggle".into(),
            tier: Tier::Write,
            title: if completed { "Aufgabe erledigen".into() } else { "Aufgabe wieder öffnen".into() },
            rows: vec![("Titel".into(), label.unwrap_or_else(|| id.to_string()))],
            request: PreparedRequest {
                method: "PATCH".into(),
                path: format!("/api/v1/todos/{id}"),
                body,
            },
            danach: "/tasks".into(),
        }))
    })
}

fn tasks_delete(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_empty() {
            return Ok(ToolOutcome::Nachfrage("Aufgaben-ID fehlt.".into()));
        }
        let (known, label) = {
            let k = ctx.known_ids.lock().await;
            (k.contains_key(id), k.get(id).cloned())
        };
        if !known {
            return Ok(ToolOutcome::Nachfrage("Diese Aufgaben-ID ist mir nicht bekannt — zuerst tasks_list aufrufen.".into()));
        }
        Ok(ToolOutcome::Card(PreparedCard {
            tool: "tasks_delete".into(),
            tier: Tier::External,
            title: "Aufgabe löschen".into(),
            rows: vec![("Titel".into(), label.unwrap_or_else(|| id.to_string()))],
            request: PreparedRequest {
                method: "DELETE".into(),
                path: format!("/api/v1/todos/{id}"),
                body: serde_json::Value::Null,
            },
            danach: "/tasks".into(),
        }))
    })
}
