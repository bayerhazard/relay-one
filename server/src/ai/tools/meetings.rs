//! Meeting-Tools (Insilo cross-app drop). Nur Read-Tier: der Assistent kann
//! Meetings auflisten, im Volltext suchen und den Markdown-Body lesen —
//! z. B. "Was haben wir in dem Müller-Meeting beschlossen?".

use std::pin::Pin;

use super::{obj_schema, ToolCtx, ToolDef, ToolOutcome, Tier};
use crate::db::with_db;

pub fn tools(locale: &str) -> Vec<ToolDef> {
    let de = locale == "de";
    vec![
        ToolDef::new(
            "meetings_list",
            Tier::Read,
            if de {
                "Listet Meeting-Zusammenfassungen aus Insilo (neueste zuerst)."
            } else {
                "List meeting summaries from Insilo (newest first)."
            },
            obj_schema(
                &[("limit", "integer", "Anzahl (Default 10, max 50)", None)],
                &[],
            ),
            meetings_list,
        ),
        ToolDef::new(
            "meetings_get",
            Tier::Read,
            if de {
                "Liest die vollständige Zusammenfassung eines Meetings (Markdown). ID aus meetings_list."
            } else {
                "Reads the full summary (Markdown) of a meeting. ID from meetings_list."
            },
            obj_schema(&[("id", "integer", "Meeting-ID (aus meetings_list)", None)], &["id"]),
            meetings_get,
        ),
        ToolDef::new(
            "meetings_search",
            Tier::Read,
            if de {
                "Durchsucht alle Meeting-Zusammenfassungen im Volltext (Titel + Inhalt)."
            } else {
                "Full-text search across all meeting summaries (title + body)."
            },
            obj_schema(&[("query", "string", "Suchbegriff", None)], &["query"]),
            meetings_search,
        ),
    ]
}

fn meetings_list(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(10).clamp(1, 50);
        let rows = with_db(&ctx.state, |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, title, meeting_date, participants, tags, duration_min
                 FROM meetings WHERE deleted = 0
                 ORDER BY meeting_date DESC, id DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(rusqlite::params![limit], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "title": r.get::<_, String>(1)?,
                        "date": r.get::<_, String>(2)?,
                        "participants": serde_json::from_str::<Vec<String>>(&r.get::<_, String>(3)?).unwrap_or_default(),
                        "tags": serde_json::from_str::<Vec<String>>(&r.get::<_, String>(4)?).unwrap_or_default(),
                        "duration_min": r.get::<_, i64>(5)?,
                    }))
                })
                .map_err(|e| e.to_string())?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|e| e.to_string())?);
            }
            Ok(out)
        })?;
        Ok(ToolOutcome::Data(serde_json::json!({
            "count": rows.len(),
            "meetings": rows,
        })))
    })
}

fn meetings_get(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let id = args.get("id").and_then(|v| v.as_i64()).ok_or("Meeting-ID fehlt.")?;
        let row = with_db(&ctx.state, |conn| {
            conn.query_row(
                "SELECT title, meeting_date, body_md FROM meetings WHERE id = ?1 AND deleted = 0",
                rusqlite::params![id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    "Meeting nicht gefunden — zuerst meetings_list aufrufen.".to_string()
                } else {
                    e.to_string()
                }
            })
        })?;
        Ok(ToolOutcome::Data(serde_json::json!({
            "id": id,
            "title": row.0,
            "date": row.1,
            "body": row.2,
        })))
    })
}

fn meetings_search(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        if query.trim().is_empty() {
            return Ok(ToolOutcome::Nachfrage("Bitte einen Suchbegriff angeben.".into()));
        }
        // Jedes Wort als quoted FTS5-String sichern (keine Syntax-Durchreichung).
        let match_expr = query
            .split_whitespace()
            .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" ");
        let rows = with_db(&ctx.state, |conn| {
            let has_fts = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name = 'meetings_fts'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;
            let sql = if has_fts {
                "SELECT m.id, m.title, m.meeting_date,
                        snippet(meetings_fts, 1, '', '…', '<b>', 20) AS snippet
                 FROM meetings m
                 JOIN meetings_fts ON meetings_fts.rowid = m.id
                 WHERE m.deleted = 0 AND meetings_fts MATCH ?1
                 ORDER BY m.meeting_date DESC LIMIT 20"
            } else {
                "SELECT id, title, meeting_date, NULL AS snippet
                 FROM meetings
                 WHERE deleted = 0 AND (title LIKE ?1 OR body_md LIKE ?1)
                 ORDER BY meeting_date DESC LIMIT 20"
            };
            let pat = if has_fts {
                match_expr
            } else {
                format!("%{}%", query.replace('%', "").replace('_', ""))
            };
            let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(rusqlite::params![pat], |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "title": r.get::<_, String>(1)?,
                        "date": r.get::<_, String>(2)?,
                        "snippet": r.get::<_, Option<String>>(3)?,
                    }))
                })
                .map_err(|e| e.to_string())?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|e| e.to_string())?);
            }
            Ok(out)
        })?;
        Ok(ToolOutcome::Data(serde_json::json!({
            "count": rows.len(),
            "results": rows,
        })))
    })
}
