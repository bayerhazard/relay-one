//! Mail tools (Concept §5.1). Reads query the local cache; writes return a
//! `PreparedCard`. `mail_propose_reply` produces a **compose** card (manual
//! send — the model never sends mail itself, Concept §6.2).

use std::pin::Pin;

use super::{obj_schema, PreparedCard, PreparedRequest, ToolCtx, ToolDef, ToolOutcome, Tier};
use crate::db::with_db;

pub fn tools(locale: &str) -> Vec<ToolDef> {
    let de = locale == "de";
    vec![
        ToolDef::new(
            "mail_search",
            Tier::Read,
            if de {
                "Sucht E-Mails (Volltext, Absender, Betreff). Unterstützt Operatoren wie is:unread."
            } else {
                "Search mail (full-text, sender, subject). Supports operators like is:unread."
            },
            obj_schema(
                &[
                    ("query", "string", "Suchbegriff", None),
                    ("limit", "integer", "Max. Treffer (Default 10)", None),
                ],
                &["query"],
            ),
            mail_search,
        ),
        ToolDef::new(
            "mail_get",
            Tier::Read,
            if de {
                "Zeigt den Inhalt einer E-Mail per ID (aus mail_search)."
            } else {
                "Show a message's content by id (from mail_search)."
            },
            obj_schema(&[("id", "string", "Nachrichten-ID (uid, aus mail_search)", None)], &["id"]),
            mail_get,
        ),
        ToolDef::new(
            "mail_propose_reply",
            Tier::Write,
            if de {
                "Erstellt einen Antwort-Entwurf und öffnet den Composer (manuelles Senden). Erzeugt eine Bestätigungskarte."
            } else {
                "Draft a reply and open the composer (manual send). Produces a confirmation card."
            },
            obj_schema(
                &[
                    ("id", "string", "Nachrichten-ID (uid, aus mail_search)", None),
                    ("body", "string", "Entwurf des Antworttextes", None),
                ],
                &["id", "body"],
            ),
            mail_propose_reply,
        ),
        ToolDef::new(
            "mail_flag",
            Tier::Write,
            if de {
                "Setzt/entfernt den Stern einer E-Mail. Erzeugt eine Bestätigungskarte."
            } else {
                "Flag/unflag a message. Produces a confirmation card."
            },
            obj_schema(
                &[
                    ("id", "string", "Nachrichten-ID (uid, aus mail_search)", None),
                    ("flagged", "boolean", "true = Stern setzen, false = entfernen", None),
                ],
                &["id", "flagged"],
            ),
            mail_flag,
        ),
        ToolDef::new(
            "mail_move",
            Tier::Write,
            if de {
                "Verschiebt eine E-Mail in einen anderen Ordner. Erzeugt eine Bestätigungskarte."
            } else {
                "Move a message to another folder. Produces a confirmation card."
            },
            obj_schema(
                &[
                    ("id", "string", "Nachrichten-ID (uid, aus mail_search)", None),
                    ("folder", "string", "Zielordner (z. B. INBOX, Archive)", None),
                ],
                &["id", "folder"],
            ),
            mail_move,
        ),
    ]
}

/// Look up the folder name + account for a message uid (best effort).
fn folder_for_uid(state: &crate::AppState, uid: i64) -> (Option<String>, Option<i64>) {
    let found = with_db(state, |conn| {
        let res = conn.query_row(
            "SELECT f.name, m.account_id FROM messages m \
             JOIN folders f ON m.folder_id = f.id \
             WHERE m.uid = ?1 LIMIT 1",
            rusqlite::params![uid],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        );
        match res {
            Ok((name, acct)) => Ok(Some((name, acct))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    })
    .ok()
    .flatten();
    match found {
        Some((name, acct)) => (Some(name), Some(acct)),
        None => (None, None),
    }
}

fn mail_search(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if query.trim().is_empty() {
            return Ok(ToolOutcome::Nachfrage("Bitte einen Suchbegriff angeben.".into()));
        }
        let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(10).max(1) as i64;
        let rows = with_db(&ctx.state, |conn| {
            crate::cache::messages::search_messages(conn, 1, &query, limit).map_err(|e| e.to_string())
        })?;
        let mut known = ctx.known_ids.lock().await;
        let mut out = Vec::with_capacity(rows.len());
        for m in &rows {
            let id = m.uid.to_string();
            let label = m.subject.clone().unwrap_or_else(|| format!("Mail von {}", m.from_addr.clone().unwrap_or_default()));
            known.insert(id.clone(), label.clone());
            let (folder, account) = folder_for_uid(&ctx.state, m.uid);
            out.push(serde_json::json!({
                "id": id,
                "subject": m.subject.as_deref().map(crate::imap::client::decode_rfc2047),
                "from": m.from_addr,
                "date": m.date,
                "folder": folder,
                "account_id": account,
                "is_read": m.is_read,
                "is_flagged": m.is_flagged,
                "preview": m.body_text.as_deref().map(|b| b.chars().take(200).collect::<String>()),
            }));
        }
        Ok(ToolOutcome::Data(serde_json::json!({ "count": out.len(), "messages": out })))
    })
}

fn mail_get(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let uid: i64 = id.parse().map_err(|_| "Ungültige Nachrichten-ID".to_string())?;
        let known = ctx.known_ids.lock().await;
        if !known.contains_key(id) {
            return Ok(ToolOutcome::Nachfrage("Diese Nachrichten-ID ist mir nicht bekannt — zuerst mail_search aufrufen.".into()));
        }
        drop(known);
        let row = with_db(&ctx.state, |conn| {
            let res = conn.query_row(
                "SELECT subject, from_addr, to_addr, date, body_text FROM messages WHERE uid = ?1 LIMIT 1",
                rusqlite::params![uid],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                },
            );
            match res {
                Ok(r) => Ok(Some(r)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.to_string()),
            }
        })?
        .ok_or_else(|| "Nachricht nicht gefunden".to_string())?;
        let body = row.4.as_deref().map(|b| b.chars().take(4000).collect::<String>());
        Ok(ToolOutcome::Data(serde_json::json!({
            "id": id,
            "subject": row.0.as_deref().map(crate::imap::client::decode_rfc2047),
            "from": row.1,
            "to": row.2,
            "date": row.3,
            "body": body,
        })))
    })
}

fn mail_propose_reply(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if id.is_empty() || body.trim().is_empty() {
            return Ok(ToolOutcome::Nachfrage("Nachrichten-ID und Antworttext werden benötigt.".into()));
        }
        let uid: i64 = id.parse().map_err(|_| "Ungültige Nachrichten-ID".to_string())?;
        let known = ctx.known_ids.lock().await;
        if !known.contains_key(id) {
            return Ok(ToolOutcome::Nachfrage("Diese Nachrichten-ID ist mir nicht bekannt — zuerst mail_search aufrufen.".into()));
        }
        drop(known);
        // Resolve the original sender + subject to pre-fill the reply.
        let (to, subject) = with_db(&ctx.state, |conn| {
            conn.query_row(
                "SELECT from_addr, subject FROM messages WHERE uid = ?1 LIMIT 1",
                rusqlite::params![uid],
                |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .map_err(|e| e.to_string())
        })
        .unwrap_or((None, None));
        let reply_subject = subject
            .as_deref()
            .map(|s| {
                let s = crate::imap::client::decode_rfc2047(s);
                if s.to_lowercase().starts_with("re:") { s } else { format!("Re: {s}") }
            })
            .unwrap_or_else(|| "Re:".into());
        let req_body = serde_json::json!({
            "to": to,
            "subject": reply_subject,
            "body": body,
        });
        Ok(ToolOutcome::Card(PreparedCard {
            tool: "mail_propose_reply".into(),
            tier: Tier::Write,
            title: "Antwort-Entwurf".into(),
            rows: vec![
                ("An".into(), to.clone().unwrap_or_else(|| "?".into())),
                ("Betreff".into(), reply_subject),
            ],
            request: PreparedRequest {
                method: "COMPOSE".into(),
                path: "/compose".into(),
                body: req_body,
            },
            danach: "/".into(),
        }))
    })
}

fn mail_flag(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let flagged = args.get("flagged").and_then(|v| v.as_bool()).unwrap_or(true);
        if id.is_empty() {
            return Ok(ToolOutcome::Nachfrage("Nachrichten-ID fehlt.".into()));
        }
        let uid: i64 = id.parse().map_err(|_| "Ungültige Nachrichten-ID".to_string())?;
        let known = ctx.known_ids.lock().await;
        if !known.contains_key(id) {
            return Ok(ToolOutcome::Nachfrage("Diese Nachrichten-ID ist mir nicht bekannt — zuerst mail_search aufrufen.".into()));
        }
        let label = known.get(id).cloned();
        drop(known);
        let (folder, account) = folder_for_uid(&ctx.state, uid);
        let body = serde_json::json!({
            "account_id": account.unwrap_or(1),
            "uid": uid,
            "folder_name": folder.unwrap_or_else(|| "INBOX".into()),
            "flagged": flagged,
        });
        Ok(ToolOutcome::Card(PreparedCard {
            tool: "mail_flag".into(),
            tier: Tier::Write,
            title: if flagged { "Stern setzen".into() } else { "Stern entfernen".into() },
            rows: vec![("Betreff".into(), label.unwrap_or_else(|| id.to_string()))],
            request: PreparedRequest {
                method: "POST".into(),
                path: "/api/v1/messages/flag".into(),
                body,
            },
            danach: "/".into(),
        }))
    })
}

fn mail_move(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let folder = args.get("folder").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if id.is_empty() || folder.trim().is_empty() {
            return Ok(ToolOutcome::Nachfrage("Nachrichten-ID und Zielordner werden benötigt.".into()));
        }
        let uid: i64 = id.parse().map_err(|_| "Ungültige Nachrichten-ID".to_string())?;
        let known = ctx.known_ids.lock().await;
        if !known.contains_key(id) {
            return Ok(ToolOutcome::Nachfrage("Diese Nachrichten-ID ist mir nicht bekannt — zuerst mail_search aufrufen.".into()));
        }
        let label = known.get(id).cloned();
        drop(known);
        let (source, account) = folder_for_uid(&ctx.state, uid);
        let body = serde_json::json!({
            "account_id": account.unwrap_or(1),
            "uid": uid,
            "source_folder": source.unwrap_or_else(|| "INBOX".into()),
            "target_folder": folder,
        });
        Ok(ToolOutcome::Card(PreparedCard {
            tool: "mail_move".into(),
            tier: Tier::Write,
            title: "E-Mail verschieben".into(),
            rows: vec![
                ("Betreff".into(), label.unwrap_or_else(|| id.to_string())),
                ("Zielordner".into(), folder),
            ],
            request: PreparedRequest {
                method: "POST".into(),
                path: format!("/api/v1/messages/{uid}/move"),
                body,
            },
            danach: "/".into(),
        }))
    })
}
