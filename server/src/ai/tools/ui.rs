//! UI effect tools (Concept §5.5). These never touch data — they emit a
//! whitelisted, declarative effect (§9.1 `event: effect`) that the frontend
//! effect router applies. The `Nav` outcome carries the effect as JSON with an
//! `effect` field naming the kind; the agent loop collects these into `effekte`.

use std::pin::Pin;

use super::{obj_schema, ToolCtx, ToolDef, ToolOutcome, Tier};

fn nav_desc() -> &'static str {
    "Navigiere in der App. module: mail, calendar, tasks, contacts, meetings, settings. optional date (YYYY-MM-DD) und view (day/week/month)."
}

pub fn tools(_locale: &str) -> Vec<ToolDef> {
    vec![
        ToolDef::new(
            "ui_navigate",
            Tier::Read,
            nav_desc(),
            obj_schema(
                &[
                    ("module", "string", "Zielmodul", Some("mail,calendar,tasks,contacts,meetings,settings")),
                    ("date", "string", "Optionales Datum (YYYY-MM-DD) für Kalender", None),
                    ("view", "string", "Optionale Kalenderansicht", Some("day,week,month")),
                ],
                &["module"],
            ),
            ui_navigate,
        ),
        ToolDef::new(
            "ui_set_view",
            Tier::Read,
            "Setzt die Kalenderansicht (day/week/month) und optional ein Datum.",
            obj_schema(
                &[
                    ("view", "string", "Ansicht", Some("day,week,month")),
                    ("date", "string", "Optionales Datum (YYYY-MM-DD)", None),
                ],
                &["view"],
            ),
            ui_set_view,
        ),
        ToolDef::new(
            "ui_open_item",
            Tier::Read,
            "Öffnet einen konkreten Eintrag in seinem Modul und springt dorthin: module calendar + id (aus calendar_list_events), tasks + uid (aus tasks_list), contacts + uid (aus contacts_search), meetings + id (aus meetings_list). Zuerst suchen, dann öffnen.",
            obj_schema(
                &[
                    ("module", "string", "Modul des Eintrags", Some("calendar,tasks,contacts,meetings")),
                    ("id", "string", "ID des Eintrags (aus einem vorherigen Lese-Ergebnis)", None),
                ],
                &["module", "id"],
            ),
            ui_open_item,
        ),
        // Concept §6.2 (mail): the model never sends — it opens the composer
        // pre-filled and the user sends manually. This closes the
        // "kein Tool für neue Mails"-gap (Review 2026-09-14).
        ToolDef::new(
            "ui_compose",
            Tier::Read,
            "Öffnet ein neues E-Mail-Fenster (manuelles Senden) mit Empfänger, Betreff und vorformuliertem Text. Erfinde keine Empfängeradresse — bei unbekannter Adresse zuerst contacts_search.",
            obj_schema(
                &[
                    ("to", "string", "Empfänger-Adresse (Pflicht)", None),
                    ("subject", "string", "Betreff", None),
                    ("body", "string", "Ausformulierter Mail-Text", None),
                ],
                &["to"],
            ),
            ui_compose,
        ),
    ]
}

/// Build an effect JSON object (Concept §9.1 `event: effect` payload).
fn effect(kind: &str, extra: &[(&str, serde_json::Value)]) -> String {
    let mut obj = serde_json::Map::new();
    obj.insert("effect".into(), serde_json::Value::String(kind.to_string()));
    for (k, v) in extra {
        obj.insert((*k).to_string(), v.clone());
    }
    serde_json::Value::Object(obj).to_string()
}

fn ui_navigate(_ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let module = args.get("module").and_then(|v| v.as_str()).unwrap_or("mail");
        let date = args.get("date").and_then(|v| v.as_str());
        let view = args.get("view").and_then(|v| v.as_str());
        let allowed = ["mail", "calendar", "tasks", "contacts", "meetings", "settings"];
        if !allowed.contains(&module) {
            return Ok(ToolOutcome::Nachfrage(format!("Unbekanntes Modul '{module}'.")));
        }
        // A calendar navigate with view/date becomes a set_view effect; the
        // router also navigates to /calendar.
        if module == "calendar" && (view.is_some() || date.is_some()) {
            let mut extra: Vec<(&str, serde_json::Value)> = vec![
                ("view", serde_json::Value::String(view.unwrap_or("week").to_string())),
            ];
            if let Some(d) = date {
                extra.push(("date", serde_json::Value::String(d.to_string())));
            }
            return Ok(ToolOutcome::Nav(effect("calendar.set_view", &extra)));
        }
        Ok(ToolOutcome::Nav(effect("navigate", &[("module", serde_json::Value::String(module.to_string()))])))
    })
}

fn ui_set_view(_ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let view = args.get("view").and_then(|v| v.as_str()).unwrap_or("week");
        let date = args.get("date").and_then(|v| v.as_str());
        if !["day", "week", "month"].contains(&view) {
            return Ok(ToolOutcome::Nachfrage(format!("Unbekannte Ansicht '{view}'.")));
        }
        let mut extra: Vec<(&str, serde_json::Value)> = vec![("view", serde_json::Value::String(view.to_string()))];
        if let Some(d) = date {
            extra.push(("date", serde_json::Value::String(d.to_string())));
        }
        Ok(ToolOutcome::Nav(effect("calendar.set_view", &extra)))
    })
}

fn ui_open_item(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let module = args.get("module").and_then(|v| v.as_str()).unwrap_or("");
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        // ID provenance (Concept §6.3): only open ids seen in this session.
        if !id.is_empty() {
            let known = ctx.known_ids.lock().await;
            if !known.contains_key(id) {
                return Ok(ToolOutcome::Nachfrage(
                    "Diese ID ist mir nicht bekannt — bitte den Eintrag zuerst suchen.".into(),
                ));
            }
        }
        let allowed = ["calendar", "tasks", "contacts", "meetings"];
        if !allowed.contains(&module) || id.is_empty() {
            return Ok(ToolOutcome::Nachfrage("Modul und ID werden benötigt.".into()));
        }
        let (kind, id_field, id_value) = match module {
            "calendar" => ("calendar.open_event", "id", serde_json::Value::String(id.to_string())),
            "tasks" => ("tasks.open", "uid", serde_json::Value::String(id.to_string())),
            "contacts" => ("contacts.open", "uid", serde_json::Value::String(id.to_string())),
            // Meetings-IDs sind numerisch (data-Contract des meetings.open-Effekts).
            "meetings" => ("meetings.open", "id", id.parse::<i64>().map(serde_json::Value::from).unwrap_or(serde_json::Value::Null)),
            _ => unreachable!(),
        };
        if id_value.is_null() {
            return Ok(ToolOutcome::Nachfrage("Meeting-IDs sind numerisch.".into()));
        }
        Ok(ToolOutcome::Nav(effect(kind, &[(id_field, id_value)])))
    })
}

fn ui_compose(_ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let to = args.get("to").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let subject = args.get("subject").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if to.is_empty() {
            return Ok(ToolOutcome::Nachfrage(
                "Der Empfänger fehlt. Bei unbekannter Adresse zuerst contacts_search aufrufen.".into(),
            ));
        }
        Ok(ToolOutcome::Nav(effect(
            "compose.open",
            &[
                ("to", serde_json::Value::String(to)),
                ("subject", serde_json::Value::String(subject)),
                ("body", serde_json::Value::String(body)),
            ],
        )))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The compose effect must match the frontend whitelist exactly
    /// (effects.ts `compose.open`: to/subject/body strings).
    #[tokio::test]
    async fn compose_effect_carries_fields() {
        let ctx = ToolCtx {
            state: crate::AppState::new(),
            locale: "de".into(),
            known_ids: std::sync::Arc::new(tokio::sync::Mutex::new(Default::default())),
        };
        let tool = ui_compose(ctx, serde_json::json!({ "to": "max@example.com", "subject": "Meetup", "body": "Text" }))
        .await
        .unwrap();
        match tool {
            ToolOutcome::Nav(json) => {
                let v: serde_json::Value = serde_json::from_str(&json).unwrap();
                assert_eq!(v["effect"], "compose.open");
                assert_eq!(v["to"], "max@example.com");
                assert_eq!(v["subject"], "Meetup");
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }
}
