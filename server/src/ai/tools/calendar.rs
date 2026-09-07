//! Calendar tools (Concept §5.1). Reads query the local cache; writes return a
//! `PreparedCard`. `calendar_find_free_slots` is a pure read (gap computation).

use std::pin::Pin;

use super::{obj_schema, PreparedCard, PreparedRequest, ToolCtx, ToolDef, ToolOutcome, Tier};
use crate::db::with_db;
use chrono::{DateTime, Duration, Utc};

pub fn tools(locale: &str) -> Vec<ToolDef> {
    let de = locale == "de";
    vec![
        ToolDef::new(
            "calendar_list_events",
            Tier::Read,
            if de {
                "Listet Termine in einem Zeitraum (Default: heute bis +7 Tage)."
            } else {
                "List events in a range (default: today to +7 days)."
            },
            obj_schema(
                &[
                    ("start", "string", "Beginn (RFC 3339 oder YYYY-MM-DD)", None),
                    ("end", "string", "Ende (RFC 3339 oder YYYY-MM-DD)", None),
                ],
                &[],
            ),
            calendar_list_events,
        ),
        ToolDef::new(
            "calendar_find_free_slots",
            Tier::Read,
            if de {
                "Findet freie Zeitfenster in einem Zeitraum (für Terminvorschläge)."
            } else {
                "Find free slots in a range (for meeting suggestions)."
            },
            obj_schema(
                &[
                    ("start", "string", "Beginn des Fensters (RFC 3339 oder YYYY-MM-DD)", None),
                    ("end", "string", "Ende des Fensters (RFC 3339 oder YYYY-MM-DD)", None),
                    ("duration_minutes", "integer", "Benötigte Dauer in Minuten (Default 60)", None),
                    ("limit", "integer", "Max. Anzahl Vorschläge (Default 5)", None),
                ],
                &["start", "end"],
            ),
            calendar_find_free_slots,
        ),
        ToolDef::new(
            "calendar_create_event",
            Tier::Write,
            if de {
                "Legt einen Termin an. Erzeugt eine Bestätigungskarte — es wird noch nichts gespeichert."
            } else {
                "Create an event. Produces a confirmation card — nothing is saved yet."
            },
            obj_schema(
                &[
                    ("summary", "string", "Titel des Termins", None),
                    ("start", "string", "Beginn (RFC 3339)", None),
                    ("end", "string", "Ende (RFC 3339, Default: +1 h)", None),
                    ("description", "string", "Beschreibung", None),
                    ("location", "string", "Ort", None),
                    ("attendees", "array", "Teilnehmer-E-Mail-Adressen", None),
                ],
                &["summary", "start"],
            ),
            calendar_create_event,
        ),
        ToolDef::new(
            "calendar_update_event",
            Tier::Write,
            if de {
                "Ändert einen Termin. Erzeugt eine Bestätigungskarte."
            } else {
                "Update an event. Produces a confirmation card."
            },
            obj_schema(
                &[
                    ("id", "string", "Termin-ID (aus calendar_list_events)", None),
                    ("summary", "string", "Neuer Titel", None),
                    ("start", "string", "Neuer Beginn (RFC 3339)", None),
                    ("end", "string", "Neues Ende (RFC 3339)", None),
                    ("description", "string", "Neue Beschreibung", None),
                    ("location", "string", "Neuer Ort", None),
                ],
                &["id"],
            ),
            calendar_update_event,
        ),
        ToolDef::new(
            "calendar_delete_event",
            Tier::External,
            if de {
                "Löscht einen Termin. Erzeugt eine Karte mit doppelter Bestätigung (unwiderruflich)."
            } else {
                "Delete an event. Produces a card with double confirmation (irreversible)."
            },
            obj_schema(&[("id", "string", "Termin-ID (aus calendar_list_events)", None)], &["id"]),
            calendar_delete_event,
        ),
        ToolDef::new(
            "calendar_invite",
            Tier::External,
            if de {
                "Lädt Personen per E-Mail zu einem Termin ein (iMIP). Erzeugt eine Karte mit doppelter Bestätigung."
            } else {
                "Invite people to an event by email (iMIP). Produces a card with double confirmation."
            },
            obj_schema(
                &[
                    ("id", "string", "Termin-ID (aus calendar_list_events)", None),
                    ("attendees", "array", "E-Mail-Adressen der Eingeladenen", None),
                ],
                &["id", "attendees"],
            ),
            calendar_invite,
        ),
        ToolDef::new(
            "calendar_rsvp",
            Tier::External,
            if de {
                "Beantwortet eine Einladung (iMIP: ACCEPTED/DECLINED/TENTATIVE). Erzeugt eine Karte mit doppelter Bestätigung."
            } else {
                "Reply to an invitation (iMIP: ACCEPTED/DECLINED/TENTATIVE). Produces a card with double confirmation."
            },
            obj_schema(
                &[
                    ("id", "string", "Termin-ID (aus calendar_list_events)", None),
                    ("decision", "string", "Entscheidung", Some("ACCEPTED,DECLINED,TENTATIVE")),
                ],
                &["id", "decision"],
            ),
            calendar_rsvp,
        ),
    ]
}

/// Parse a date/datetime arg into a UTC datetime. Accepts RFC 3339 or YYYY-MM-DD.
fn parse_dt(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d.and_hms_opt(0, 0, 0).unwrap().and_utc());
    }
    None
}

fn register_event_ids(ctx: &ToolCtx, events: &[crate::cache::cal::EventRow]) -> impl std::future::Future<Output = ()> {
    let ids: Vec<(String, String)> = events
        .iter()
        .map(|e| (e.id.to_string(), e.summary.clone().unwrap_or_else(|| e.id.to_string())))
        .collect();
    let known = ctx.known_ids.clone();
    async move {
        let mut k = known.lock().await;
        for (id, label) in ids {
            k.insert(id, label);
        }
    }
}

fn calendar_list_events(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let now = Utc::now();
        let start = args
            .get("start")
            .and_then(|v| v.as_str())
            .and_then(parse_dt)
            .unwrap_or_else(|| now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc());
        let end = args
            .get("end")
            .and_then(|v| v.as_str())
            .and_then(parse_dt)
            .unwrap_or_else(|| (now + Duration::days(7)).date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc());
        let events = with_db(&ctx.state, |conn| {
            crate::cache::cal::list_events(conn, None, Some(&start.to_rfc3339()), Some(&end.to_rfc3339()))
                .map_err(|e| e.to_string())
        })?;
        register_event_ids(&ctx, &events).await;
        let out: Vec<serde_json::Value> = events
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "summary": e.summary,
                    "start": e.start_at,
                    "end": e.end_at,
                    "location": e.location,
                    "all_day": e.all_day,
                    "attendees": e.attendees.iter().map(|a| a.email.clone()).collect::<Vec<_>>(),
                })
            })
            .collect();
        Ok(ToolOutcome::Data(serde_json::json!({ "count": out.len(), "events": out })))
    })
}

fn calendar_find_free_slots(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let start = args.get("start").and_then(|v| v.as_str()).and_then(parse_dt)
            .ok_or_else(|| "Ungültiges Start-Datum".to_string())?;
        let end = args.get("end").and_then(|v| v.as_str()).and_then(parse_dt)
            .ok_or_else(|| "Ungültiges End-Datum".to_string())?;
        if end <= start {
            return Ok(ToolOutcome::Nachfrage("Ende muss nach dem Beginn liegen.".into()));
        }
        let duration = Duration::minutes(args.get("duration_minutes").and_then(|v| v.as_i64()).unwrap_or(60).max(10));
        let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(5).max(1) as usize;

        let events = with_db(&ctx.state, |conn| {
            crate::cache::cal::list_events(conn, None, Some(&start.to_rfc3339()), Some(&end.to_rfc3339()))
                .map_err(|e| e.to_string())
        })?;

        // Busy intervals inside the window.
        let mut busy: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
        for e in &events {
            let s = parse_dt(&e.start_at).unwrap_or(start);
            let en = e.end_at.as_deref().and_then(parse_dt).unwrap_or(s + Duration::hours(1));
            let s = s.max(start);
            let en = en.min(end);
            if en > s {
                busy.push((s, en));
            }
        }
        busy.sort_by_key(|b| b.0);

        // Walk the window, emitting gaps that fit the duration.
        let mut slots: Vec<serde_json::Value> = Vec::new();
        let mut cursor = start;
        for (bs, be) in &busy {
            if *bs - cursor >= duration {
                slots.push(serde_json::json!({
                    "start": cursor.to_rfc3339(),
                    "end": (cursor + duration).to_rfc3339(),
                }));
                if slots.len() >= limit {
                    break;
                }
            }
            cursor = cursor.max(*be);
        }
        if slots.len() < limit && end - cursor >= duration {
            slots.push(serde_json::json!({
                "start": cursor.to_rfc3339(),
                "end": (cursor + duration).to_rfc3339(),
            }));
        }
        Ok(ToolOutcome::Data(serde_json::json!({ "count": slots.len(), "slots": slots })))
    })
}

fn calendar_create_event(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let summary = args.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let start = args.get("start").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if summary.trim().is_empty() || parse_dt(&start).is_none() {
            return Ok(ToolOutcome::Nachfrage("Titel und gültiger Beginn (RFC 3339) werden benötigt.".into()));
        }
        let end = args.get("end").and_then(|v| v.as_str()).map(str::to_string);
        let description = args.get("description").and_then(|v| v.as_str()).map(str::to_string);
        let location = args.get("location").and_then(|v| v.as_str()).map(str::to_string);
        let attendees: Vec<String> = args
            .get("attendees")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        // Pick the first writable calendar.
        let cal_id = with_db(&ctx.state, |conn| {
            conn.query_row(
                "SELECT id FROM calendars WHERE COALESCE(read_only, 0) = 0 ORDER BY id LIMIT 1",
                [],
                |r| r.get::<_, i64>(0),
            )
            .or_else(|_| {
                conn.query_row("SELECT id FROM calendars ORDER BY id LIMIT 1", [], |r| r.get::<_, i64>(0))
            })
            .map_err(|e| e.to_string())
        })
        .ok();
        let body = serde_json::json!({
            "calendar_id": cal_id,
            "summary": summary,
            "start": start,
            "end": end,
            "description": description,
            "location": location,
            "attendees": attendees.iter().map(|e| serde_json::json!({ "email": e })).collect::<Vec<_>>(),
        });
        let mut rows = vec![("Titel".into(), summary.clone()), ("Beginn".into(), start)];
        if let Some(l) = &location {
            rows.push(("Ort".into(), l.clone()));
        }
        if !attendees.is_empty() {
            rows.push(("Teilnehmer".into(), attendees.join(", ")));
        }
        Ok(ToolOutcome::Card(PreparedCard {
            tool: "calendar_create_event".into(),
            tier: Tier::Write,
            title: "Termin anlegen".into(),
            rows,
            request: PreparedRequest {
                method: "POST".into(),
                path: "/api/v1/calendars/events".into(),
                body,
            },
            danach: "/calendar".into(),
        }))
    })
}

fn calendar_update_event(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_empty() {
            return Ok(ToolOutcome::Nachfrage("Termin-ID fehlt.".into()));
        }
        let known = ctx.known_ids.lock().await;
        if !known.contains_key(id) {
            return Ok(ToolOutcome::Nachfrage("Diese Termin-ID ist mir nicht bekannt — zuerst calendar_list_events aufrufen.".into()));
        }
        let label = known.get(id).cloned();
        drop(known);
        let get = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let body = serde_json::json!({
            "summary": get("summary"),
            "start": get("start"),
            "end": get("end"),
            "description": get("description"),
            "location": get("location"),
        });
        let mut rows = vec![("Termin".into(), label.unwrap_or_else(|| id.to_string()))];
        if let Some(s) = get("summary") {
            rows.push(("Neuer Titel".into(), s));
        }
        if let Some(s) = get("start") {
            rows.push(("Neuer Beginn".into(), s));
        }
        Ok(ToolOutcome::Card(PreparedCard {
            tool: "calendar_update_event".into(),
            tier: Tier::Write,
            title: "Termin ändern".into(),
            rows,
            request: PreparedRequest {
                method: "PUT".into(),
                path: format!("/api/v1/events/{id}"),
                body,
            },
            danach: "/calendar".into(),
        }))
    })
}

fn calendar_delete_event(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_empty() {
            return Ok(ToolOutcome::Nachfrage("Termin-ID fehlt.".into()));
        }
        let known = ctx.known_ids.lock().await;
        if !known.contains_key(id) {
            return Ok(ToolOutcome::Nachfrage("Diese Termin-ID ist mir nicht bekannt — zuerst calendar_list_events aufrufen.".into()));
        }
        let label = known.get(id).cloned();
        drop(known);
        Ok(ToolOutcome::Card(PreparedCard {
            tool: "calendar_delete_event".into(),
            tier: Tier::External,
            title: "Termin löschen".into(),
            rows: vec![("Termin".into(), label.unwrap_or_else(|| id.to_string()))],
            request: PreparedRequest {
                method: "DELETE".into(),
                path: format!("/api/v1/events/{id}"),
                body: serde_json::Value::Null,
            },
            danach: "/calendar".into(),
        }))
    })
}

fn calendar_invite(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let attendees: Vec<String> = args
            .get("attendees")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        if id.is_empty() || attendees.is_empty() {
            return Ok(ToolOutcome::Nachfrage("Termin-ID und mindestens eine E-Mail werden benötigt.".into()));
        }
        let known = ctx.known_ids.lock().await;
        let ok = known.contains_key(id);
        let label = known.get(id).cloned();
        drop(known);
        if !ok {
            return Ok(ToolOutcome::Nachfrage("Diese Termin-ID ist mir nicht bekannt — zuerst calendar_list_events aufrufen.".into()));
        }
        let body = serde_json::json!({
            "account_id": 1,
            "attendees": attendees.iter().map(|e| serde_json::json!({ "email": e })).collect::<Vec<_>>(),
        });
        Ok(ToolOutcome::Card(PreparedCard {
            tool: "calendar_invite".into(),
            tier: Tier::External,
            title: "Einladung senden".into(),
            rows: vec![
                ("Termin".into(), label.unwrap_or_else(|| id.to_string())),
                ("Eingeladen".into(), attendees.join(", ")),
            ],
            request: PreparedRequest {
                method: "POST".into(),
                path: format!("/api/v1/calendars/events/{id}/invite"),
                body,
            },
            danach: "/calendar".into(),
        }))
    })
}

fn calendar_rsvp(ctx: ToolCtx, args: serde_json::Value) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>> {
    Box::pin(async move {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let decision = args.get("decision").and_then(|v| v.as_str()).unwrap_or("ACCEPTED");
        if id.is_empty() || !["ACCEPTED", "DECLINED", "TENTATIVE"].contains(&decision) {
            return Ok(ToolOutcome::Nachfrage("Termin-ID und gültige Entscheidung (ACCEPTED/DECLINED/TENTATIVE) werden benötigt.".into()));
        }
        let known = ctx.known_ids.lock().await;
        let ok = known.contains_key(id);
        let label = known.get(id).cloned();
        drop(known);
        if !ok {
            return Ok(ToolOutcome::Nachfrage("Diese Termin-ID ist mir nicht bekannt — zuerst calendar_list_events aufrufen.".into()));
        }
        let body = serde_json::json!({
            "account_id": 1,
            "attendees": [ { "part_stat": decision } ],
        });
        Ok(ToolOutcome::Card(PreparedCard {
            tool: "calendar_rsvp".into(),
            tier: Tier::External,
            title: "Einladung beantworten".into(),
            rows: vec![
                ("Termin".into(), label.unwrap_or_else(|| id.to_string())),
                ("Entscheidung".into(), decision.to_string()),
            ],
            request: PreparedRequest {
                method: "POST".into(),
                path: format!("/api/v1/calendars/events/{id}/rsvp"),
                body,
            },
            danach: "/calendar".into(),
        }))
    })
}
