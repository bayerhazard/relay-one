//! Contact tools (Concept §5.1). Reads query the local cache; the write tool
//! returns a `PreparedCard` (never writes itself).

use std::pin::Pin;

use super::{obj_schema, PreparedCard, PreparedRequest, ToolCtx, ToolDef, ToolOutcome, Tier};
use crate::db::with_db;

pub fn tools(locale: &str) -> Vec<ToolDef> {
    let de = locale == "de";
    let search_desc = if de {
        "Suche Kontakte nach Name oder E-Mail."
    } else {
        "Search contacts by name or email."
    };
    vec![
        ToolDef::new(
            "contacts_search",
            Tier::Read,
            search_desc,
            obj_schema(
                &[("query", "string", "Suchbegriff (Name oder E-Mail)", None)],
                &["query"],
            ),
            contacts_search,
        ),
        ToolDef::new(
            "contacts_get",
            Tier::Read,
            if de { "Zeigt einen Kontakt per ID (aus contacts_search)." } else { "Show a contact by id (from contacts_search)."},
            obj_schema(&[("id", "string", "Kontakt-ID (vcard_uid)", None)], &["id"]),
            contacts_get,
        ),
        ToolDef::new(
            "contacts_create",
            Tier::Write,
            if de {
                "Legt einen neuen Kontakt an. Erzeugt eine Bestätigungskarte — es wird noch nichts gespeichert."
            } else {
                "Create a new contact. Produces a confirmation card — nothing is saved yet."
            },
            obj_schema(
                &[
                    ("given_name", "string", "Vorname", None),
                    ("family_name", "string", "Nachname", None),
                    ("email", "string", "E-Mail-Adresse", None),
                    ("phone", "string", "Telefonnummer", None),
                    ("organization", "string", "Organisation", None),
                ],
                &["given_name", "family_name"],
            ),
            contacts_create,
        ),
    ]
}

fn contacts_search(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if query.trim().is_empty() {
            return Ok(ToolOutcome::Nachfrage("Bitte einen Suchbegriff angeben.".into()));
        }
        let rows = with_db(&ctx.state, |conn| {
            crate::cache::contacts::list_contacts(conn, &query).map_err(|e| e.to_string())
        })?;
        let mut known = ctx.known_ids.lock().await;
        let mut out = Vec::with_capacity(rows.len());
        for c in &rows {
            let id = c.vcard_uid.clone();
            let label = c
                .display_name
                .clone()
                .or_else(|| c.email.clone())
                .unwrap_or_else(|| id.clone());
            known.insert(id.clone(), label.clone());
            out.push(serde_json::json!({
                "id": id,
                "display_name": c.display_name,
                "given_name": c.given_name,
                "family_name": c.family_name,
                "email": c.email,
                "phone": c.phone,
                "organization": c.organization,
            }));
        }
        Ok(ToolOutcome::Data(serde_json::json!({ "count": out.len(), "contacts": out })))
    })
}

fn contacts_get(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_empty() {
            return Ok(ToolOutcome::Nachfrage("Kontakt-ID fehlt.".into()));
        }
        let known = ctx.known_ids.lock().await;
        if !known.contains_key(id) {
            return Ok(ToolOutcome::Nachfrage("Diese Kontakt-ID ist mir nicht bekannt — zuerst suchen.".into()));
        }
        drop(known);
        let row = with_db(&ctx.state, |conn| {
            crate::cache::contacts::get_contact(conn, id).map_err(|e| e.to_string())
        })?
        .ok_or_else(|| "Kontakt nicht gefunden".to_string())?;
        Ok(ToolOutcome::Data(serde_json::json!({
            "id": row.vcard_uid,
            "display_name": row.display_name,
            "given_name": row.given_name,
            "family_name": row.family_name,
            "email": row.email,
            "phone": row.phone,
            "organization": row.organization,
        })))
    })
}

fn contacts_create(_ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let get = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let given = get("given_name");
        let family = get("family_name");
        let email = get("email");
        let phone = get("phone");
        let org = get("organization");
        if given.trim().is_empty() && family.trim().is_empty() {
            return Ok(ToolOutcome::Nachfrage("Vor- oder Nachname wird benötigt.".into()));
        }
        let display = format!("{given} {family}").trim().to_string();
        let body = serde_json::json!({
            "given_name": given,
            "family_name": family,
            "display_name": display,
            "email": email,
            "phone": phone,
            "organization": org,
        });
        let mut rows = vec![("Name".into(), display)];
        if !email.is_empty() {
            rows.push(("E-Mail".into(), email));
        }
        if !phone.is_empty() {
            rows.push(("Telefon".into(), phone));
        }
        if !org.is_empty() {
            rows.push(("Organisation".into(), org));
        }
        Ok(ToolOutcome::Card(PreparedCard {
            tool: "contacts_create".into(),
            tier: Tier::Write,
            title: "Kontakt anlegen".into(),
            rows,
            request: PreparedRequest {
                method: "POST".into(),
                path: "/api/v1/contacts".into(),
                body,
            },
            danach: "/contacts".into(),
        }))
    })
}
