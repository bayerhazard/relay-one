//! Insilo idea memos → tasks.
//!
//! Reads the Idea voice memos straight from the Insilo appData (read-only
//! mount, derived from Relay's own appData) and turns every idea into a task —
//! either a new task or an enrichment of an existing one. The snippets are
//! never shown; only the resulting task (and what the AI already prepared)
//! surfaces in Relay.
//!
//! Robust by design: ideas are persisted before processing, retried with
//! backoff, and a fallback task is created when the LLM keeps failing so
//! nothing is lost.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::cache;
use crate::db::with_db;
use crate::AppState;

/// Retry delays (seconds) per attempt before the fallback task is created.
const BACKOFF_SECS: [i64; 5] = [60, 300, 900, 3600, 21600];

/// Report of one scan pass.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct IdeaScanReport {
    pub scanned: usize,
    pub inserted: usize,
    pub changed: usize,
    pub skipped: usize,
}

/// A parsed idea memo.
struct IdeaFile {
    insilo_id: String,
    title: String,
    recorded_at: String,
    duration_min: i64,
    language: String,
    transcript: String,
}

/// Idea drop directory from the environment; `None` = disabled.
fn ideas_dir() -> Option<PathBuf> {
    if std::env::var("RELAY_INSILO_IDEAS_ENABLED").unwrap_or_default() == "false" {
        return None;
    }
    let dir = std::env::var("RELAY_INSILO_IDEAS_DIR").ok()?;
    if dir.is_empty() {
        return None;
    }
    Some(PathBuf::from(dir))
}

fn idea_prefix() -> String {
    std::env::var("RELAY_INSILO_IDEA_PREFIX")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Idea".to_string())
}

fn unquote(s: &str) -> &str {
    s.trim().trim_matches(|c| c == '"' || c == '\'')
}

/// Parse the audio-memo front-matter schema (`meeting_id`, `title`, `date`,
/// `duration_min`, `language`, ...). List lines (`  - x`) are ignored.
fn parse_idea(content: &str) -> Result<IdeaFile, String> {
    let mut lines = content.lines();
    if lines.next().map(|l| l.trim()) != Some("---") {
        return Err("keine Frontmatter".into());
    }
    let mut fields: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        // List items (speakers, tags as YAML list) have no useful key here.
        if line.trim_start().starts_with("- ") {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            fields.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    if !closed {
        return Err("Frontmatter nicht geschlossen".into());
    }
    let field = |k: &str| fields.get(k).map(|s| s.as_str()).unwrap_or("");
    let insilo_id = unquote(field("meeting_id")).to_string();
    if insilo_id.is_empty() {
        return Err("meeting_id fehlt".into());
    }
    let body = content
        .find("\n---\n")
        .map(|i| content[i + 5..].trim_start_matches('\n').to_string())
        .unwrap_or_default();
    Ok(IdeaFile {
        insilo_id,
        title: unquote(field("title")).to_string(),
        recorded_at: unquote(field("date")).to_string(),
        duration_min: field("duration_min").parse().unwrap_or(0),
        language: unquote(field("language")).to_string(),
        transcript: body,
    })
}

/// True when the memo is an "idea" (title carries the configured prefix or the
/// summary template is the quick-note one).
fn is_idea(f: &IdeaFile, prefix: &str) -> bool {
    let title = f.title.trim_start().to_lowercase();
    let p = prefix.trim().to_lowercase();
    !p.is_empty() && title.starts_with(&p)
}

/// Recursively collect `.md` files (the Insilo audio tree is small).
fn collect_md(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_md(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

/// One scan pass: parse new/changed idea files and persist them as `pending`.
pub fn run_ideas_scan(state: &AppState) -> IdeaScanReport {
    let Some(dir) = ideas_dir() else {
        return IdeaScanReport::default();
    };
    if !dir.exists() {
        return IdeaScanReport::default();
    }
    let prefix = idea_prefix();
    let mut files = Vec::new();
    collect_md(&dir, &mut files);

    let mut report = IdeaScanReport::default();
    let mut parsed: Vec<(PathBuf, IdeaFile)> = Vec::new();
    for path in files {
        // Only the transcript is relevant; summaries would duplicate it.
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.ends_with(".transkript.md") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            report.skipped += 1;
            continue;
        };
        match parse_idea(&content) {
            Ok(f) if is_idea(&f, &prefix) => {
                report.scanned += 1;
                parsed.push((path, f));
            }
            Ok(_) => {} // a normal recording, not an idea
            Err(_) => report.skipped += 1,
        }
    }

    let upserts: Vec<(String, String, String, IdeaFile)> = parsed
        .into_iter()
        .map(|(path, f)| {
            let sha = hex::encode(Sha256::digest(f.transcript.as_bytes()));
            (path.to_string_lossy().to_string(), sha, f.insilo_id.clone(), f)
        })
        .collect();

    let result = with_db(state, |conn| {
        let mut inserted = 0usize;
        let mut changed = 0usize;
        for (path, sha, insilo_id, f) in &upserts {
            match cache::ideas::upsert_idea(
                conn,
                insilo_id,
                path,
                sha,
                &f.title,
                &f.recorded_at,
                f.duration_min,
                &f.language,
                &f.transcript,
            ) {
                Ok((true, _)) => inserted += 1,
                Ok((false, true)) => changed += 1,
                Ok((false, false)) => {}
                Err(e) => tracing::warn!(insilo_id, "Idee upsert fehlgeschlagen: {e}"),
            }
        }
        Ok((inserted, changed))
    });
    if let Ok((inserted, changed)) = result {
        report.inserted = inserted;
        report.changed = changed;
    }
    report
}

// ── Verarbeitung ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
struct PlanCalendar {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    start: String,
    #[serde(default)]
    end: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct PlanMail {
    #[serde(default)]
    to: String,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    body: String,
}

#[derive(Debug, Deserialize, Default)]
struct IdeaPlan {
    #[serde(default)]
    action: String,
    #[serde(default)]
    append_to_uid: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    due: Option<String>,
    #[serde(default)]
    priority: Option<i64>,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    calendar: Option<PlanCalendar>,
    #[serde(default)]
    mail: Option<PlanMail>,
}

/// Extract the first `{...}` object from a model answer.
fn extract_json(raw: &str) -> Result<serde_json::Value, String> {
    let s = raw.find('{').ok_or("kein JSON in der Antwort")?;
    let e = raw.rfind('}').ok_or("kein JSON in der Antwort")?;
    if e < s {
        return Err("kein JSON in der Antwort".into());
    }
    serde_json::from_str(&raw[s..=e]).map_err(|err| format!("ungültiges JSON: {err}"))
}

fn build_prompt(idea: &cache::ideas::IdeaRow, open_tasks: &[(String, String)]) -> (String, String) {
    let system = "Du verarbeitest kurze Sprachmemos (Ideen) zu Aufgaben. Antworte NUR mit einem \
JSON-Objekt, ohne Erklärtext.\n\
Regeln:\n\
- Erzeuge GENAU EINE kompakte Aufgabe ODER ergänze GENAU EINE bestehende Aufgabe \
(action \"create\" oder \"append\" + append_to_uid).\n\
- Titel: maximal 8 Wörter. note: maximal 2 kurze Zeilen. Keine Romane, das Transkript nicht wiederholen.\n\
- Ist die Idee klar eine Ergänzung zu einer offenen Aufgabe, nutze action \"append\".\n\
- calendar/mail nur, wenn die Idee es klar verlangt. Mails werden NIE gesendet, Termine ohne Teilnehmer angelegt.\n\
- Erfinde keine Fakten.\n\
Schema: {\"action\":\"create|append\",\"append_to_uid\":\"...\",\"summary\":\"...\",\"note\":\"...\",\
\"due\":\"YYYY-MM-DD|RFC3339|null\",\"priority\":1-9|null,\"labels\":[\"...\"],\
\"calendar\":{\"summary\":\"...\",\"start\":\"RFC3339\",\"end\":\"RFC3339\"}|null,\
\"mail\":{\"to\":\"...\",\"subject\":\"...\",\"body\":\"...\"}|null}"
        .to_string();
    let mut user = String::new();
    user.push_str(&format!(
        "Sprachmemo vom {} ({} min)\nTitel: {}\nTranskript:\n{}\n\n",
        idea.recorded_at, idea.duration_min, idea.title, crate::ai::agent::clamp_text(&idea.transcript_md, 6000)
    ));
    user.push_str("Offene Aufgaben (uid | Titel):\n");
    if open_tasks.is_empty() {
        user.push_str("(keine)\n");
    } else {
        for (uid, summary) in open_tasks.iter().take(60) {
            user.push_str(&format!("- {} | {}\n", uid, crate::ai::agent::clamp_text(summary, 100)));
        }
    }
    (system, user)
}

/// Build the task description from the idea essence and what the AI prepared.
fn build_description(note: &str, done: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let note = note.trim();
    if !note.is_empty() {
        parts.push(note.to_string());
    }
    if !done.is_empty() {
        let mut block = String::new();
        for line in done {
            if !block.is_empty() {
                block.push('\n');
            }
            block.push_str(line);
        }
        parts.push(block);
    }
    parts.join("\n\n")
}

/// Create a fresh task and return its uid.
async fn create_task(state: &AppState, summary: &str, description: &str, plan: &IdeaPlan) -> Result<String, String> {
    let mut labels = plan.labels.clone();
    if !labels.iter().any(|l| l.eq_ignore_ascii_case("Idee")) {
        labels.push("Idee".to_string());
    }
    let req = crate::api::todos::CreateTodoRequest {
        summary: crate::ai::agent::clamp_text(summary, 120),
        description: if description.trim().is_empty() { None } else { Some(description.to_string()) },
        due: plan.due.clone().filter(|d| !d.trim().is_empty()),
        priority: plan.priority,
        labels,
        rrule: None,
        parent_uid: None,
        project_id: None,
        section: None,
        due_has_time: None,
    };
    let res = crate::api::todos::create_todo(axum::extract::State(state.clone()), axum::Json(req)).await;
    match res {
        Ok(axum::Json(row)) => Ok(row.uid),
        Err(e) => Err(e.0),
    }
}

/// Append the prepared lines to an existing task's description.
async fn append_task(state: &AppState, uid: &str, addition: &str) -> Result<String, String> {
    let current = with_db(state, |conn| {
        cache::todo::find_todo(conn, uid).map_err(|e| e.to_string())
    })?;
    let Some(row) = current else {
        return Err(format!("Aufgabe {uid} nicht gefunden"));
    };
    let base = row.description.clone().unwrap_or_default();
    let new_desc = if base.trim().is_empty() {
        addition.to_string()
    } else {
        format!("{}\n\n{}", base.trim_end(), addition)
    };
    let req = crate::api::todos::PatchTodoRequest {
        completed: None,
        summary: None,
        description: Some(Some(new_desc)),
        due: None,
        priority: None,
        labels: None,
        rrule: None,
        project_id: None,
        section: None,
    };
    match crate::api::todos::toggle_todo(axum::extract::State(state.clone()), axum::extract::Path(uid.to_string()), axum::Json(req)).await {
        Ok(_) => Ok(uid.to_string()),
        Err(e) => Err(e.0),
    }
}

/// Create a calendar event without inviting anyone. Returns a "Mighty" line.
async fn prepare_calendar(state: &AppState, cal: &PlanCalendar) -> Result<String, String> {
    let calendar_id = with_db(state, |conn| {
        cache::cal::list_calendars(conn).map_err(|e| e.to_string())
    })?
    .into_iter()
    .next()
    .map(|c| c.id)
    .ok_or("kein Kalender eingerichtet")?;
    let req = crate::api::calendars::CreateEventRequest {
        calendar_id,
        summary: crate::ai::agent::clamp_text(&cal.summary, 120),
        start: cal.start.clone(),
        end: cal.end.clone(),
        description: None,
        location: None,
        organizer: None,
        attendees: Vec::new(),
        rrule: None,
        reminder_minutes: None,
    };
    match crate::api::calendars::create_event(axum::extract::State(state.clone()), axum::Json(req)).await {
        Ok(_) => Ok(format!(
            "Mighty: Termin \"{}\" angelegt – Teilnehmer nicht eingeladen.",
            crate::ai::agent::clamp_text(&cal.summary, 60)
        )),
        Err(e) => Err(e.0),
    }
}

/// Save a mail draft (never sends). Returns a "Mighty" line.
async fn prepare_mail(state: &AppState, mail: &PlanMail) -> Result<String, String> {
    let to = mail.to.trim();
    if to.is_empty() {
        return Err("Mail ohne Empfänger".into());
    }
    let account_id = with_db(state, |conn| {
        cache::accounts::list_accounts(conn).map_err(|e| e.to_string())
    })?
    .into_iter()
    .next()
    .map(|a| a.id as u32)
    .ok_or("kein E-Mail-Konto eingerichtet")?;
    let req = crate::api::send::SaveDraftRequest {
        account_id,
        uid: None,
        to: to.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        cc: None,
        bcc: None,
        subject: crate::ai::agent::clamp_text(&mail.subject, 160),
        body_text: mail.body.clone(),
        body_html: None,
        attachments: None,
    };
    match crate::api::send::save_draft(axum::extract::State(state.clone()), axum::Json(req)).await {
        Ok(_) => Ok(format!(
            "Mighty: Mail-Entwurf an {} vorbereitet (nicht gesendet).",
            crate::ai::agent::clamp_text(to, 80)
        )),
        Err(e) => Err(e.0),
    }
}

/// First `n` words of the first meaningful text line (fallback title).
/// Skips headings and strips a leading `[0:00] **Speaker**:` transcript marker.
fn first_words(text: &str, n: usize) -> String {
    let line = text
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("");
    let mut s = line;
    if let Some(rest) = s.strip_prefix('[') {
        if let Some(pos) = rest.find(']') {
            s = rest[pos + 1..].trim();
        }
    }
    if let Some(rest) = s.strip_prefix("**") {
        if let Some(end) = rest.find("**") {
            s = rest[end + 2..].trim_start_matches(':').trim();
        }
    }
    let words: Vec<&str> = s.split_whitespace().take(n).collect();
    words.join(" ")
}

/// Process a single idea end-to-end. Errors are retried by the caller.
async fn process_one(state: &AppState, idea: &cache::ideas::IdeaRow) -> Result<(), String> {
    let client = {
        let guard = state.ai_client.read();
        guard.as_ref().cloned().ok_or("KI-Client nicht konfiguriert")?
    };
    let open_tasks: Vec<(String, String)> = with_db(state, |conn| {
        cache::todo::list_todos(conn, Some(false)).map_err(|e| e.to_string())
    })?
    .into_iter()
    .map(|t| (t.uid, t.summary.unwrap_or_default()))
    .collect();

    let (system, user) = build_prompt(idea, &open_tasks);
    let raw = client.complete_user_json(&system, &user, Some(0.2), Some(1200)).await?;
    let plan: IdeaPlan = serde_json::from_value(extract_json(&raw)?)
        .map_err(|e| format!("Plan nicht lesbar: {e}"))?;

    // Prepared side artefacts — never any external effect (no send, no invite).
    let mut done: Vec<String> = Vec::new();
    if let Some(cal) = plan
        .calendar
        .as_ref()
        .filter(|c| !c.summary.trim().is_empty() && !c.start.trim().is_empty())
    {
        match prepare_calendar(state, cal).await {
            Ok(line) => done.push(line),
            Err(e) => tracing::warn!(insilo_id = %idea.insilo_id, "Termin aus Idee fehlgeschlagen: {e}"),
        }
    }
    if let Some(mail) = plan.mail.as_ref().filter(|m| !m.to.trim().is_empty()) {
        match prepare_mail(state, mail).await {
            Ok(line) => done.push(line),
            Err(e) => tracing::warn!(insilo_id = %idea.insilo_id, "Mail-Entwurf aus Idee fehlgeschlagen: {e}"),
        }
    }

    let note = plan.note.clone().unwrap_or_default();
    let append_target = plan
        .append_to_uid
        .clone()
        .filter(|uid| plan.action.eq_ignore_ascii_case("append") && open_tasks.iter().any(|(u, _)| u == uid));

    let task_uid = match append_target {
        Some(uid) => {
            let mut addition = String::new();
            if !note.trim().is_empty() {
                addition.push_str("Mighty: ");
                addition.push_str(note.trim());
            }
            if !done.is_empty() {
                if !addition.is_empty() {
                    addition.push('\n');
                }
                addition.push_str(&done.join("\n"));
            }
            if addition.is_empty() {
                addition.push_str("Mighty: Ergänzung aus Sprachmemo.");
            }
            append_task(state, &uid, &addition).await?
        }
        None => {
            let summary = {
                let s = plan.summary.clone().unwrap_or_default();
                if s.trim().is_empty() {
                    let fallback = first_words(&idea.transcript_md, 8);
                    if fallback.is_empty() {
                        format!("Idee vom {}", idea.recorded_at)
                    } else {
                        fallback
                    }
                } else {
                    s
                }
            };
            let description = build_description(&note, &done);
            create_task(state, &summary, &description, &plan).await?
        }
    };

    with_db(state, |conn| {
        cache::ideas::mark_done(conn, &idea.insilo_id, &task_uid).map_err(|e| e.to_string())
    })?;
    Ok(())
}

/// Fallback: never lose an idea when the LLM keeps failing.
async fn create_fallback_task(state: &AppState, idea: &cache::ideas::IdeaRow) -> Result<String, String> {
    let mut summary = first_words(&idea.transcript_md, 8);
    if summary.is_empty() {
        summary = format!("Idee vom {}", idea.recorded_at);
    }
    let desc = "Mighty: Automatische Verarbeitung fehlgeschlagen – Inhalt bitte prüfen.";
    create_task(state, &summary, desc, &IdeaPlan::default()).await
}

fn schedule_retry(state: &AppState, idea: &cache::ideas::IdeaRow, attempts: i64, error: &str) {
    let idx = (attempts - 1).clamp(0, BACKOFF_SECS.len() as i64 - 1) as usize;
    let next = chrono::Utc::now() + chrono::Duration::seconds(BACKOFF_SECS[idx]);
    let next_str = next.format("%Y-%m-%d %H:%M:%S").to_string();
    let _ = with_db(state, |conn| {
        cache::ideas::mark_failed(conn, &idea.insilo_id, error, &next_str)
            .map_err(|e| e.to_string())
    });
}

/// Temporary infrastructure outage (LLM/gateway down, circuit breaker open):
/// retry without counting the attempt, so a short outage never triggers the
/// fallback task.
fn is_infra_error(e: &str) -> bool {
    let l = e.to_lowercase();
    [
        "503", "502", "504", "429",
        "temporär nicht verfügbar",
        "circuit breaker",
        "connection",
        "connect error",
        "disconnect",
        "timeout",
        "timed out",
    ]
    .iter()
    .any(|p| l.contains(p))
}

fn schedule_infra_retry(state: &AppState, idea: &cache::ideas::IdeaRow, error: &str) {
    let next = chrono::Utc::now() + chrono::Duration::seconds(900);
    let next_str = next.format("%Y-%m-%d %H:%M:%S").to_string();
    let _ = with_db(state, |conn| {
        cache::ideas::mark_retry(conn, &idea.insilo_id, error, &next_str)
            .map_err(|e| e.to_string())
    });
}

/// Process a small batch of due ideas (called after each scan tick).
pub async fn process_due_ideas(state: &AppState) {
    let due = match with_db(state, |conn| {
        cache::ideas::list_due_ideas(conn, 5).map_err(|e| e.to_string())
    }) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Ideen-Queue nicht lesbar: {e}");
            return;
        }
    };
    for idea in due {
        match process_one(state, &idea).await {
            Ok(()) => tracing::info!(insilo_id = %idea.insilo_id, "Idee verarbeitet"),
            Err(e) => {
                if is_infra_error(&e) {
                    tracing::warn!(insilo_id = %idea.insilo_id, "Idee: KI temporär nicht verfügbar, Retry in 15 min: {e}");
                    schedule_infra_retry(state, &idea, &e);
                    continue;
                }
                let attempts = idea.attempts + 1;
                if attempts >= BACKOFF_SECS.len() as i64 {
                    match create_fallback_task(state, &idea).await {
                        Ok(uid) => {
                            let _ = with_db(state, |conn| {
                                cache::ideas::mark_done(conn, &idea.insilo_id, &uid)
                                    .map_err(|e| e.to_string())
                            });
                            tracing::warn!(insilo_id = %idea.insilo_id, "Idee: Fallback-Aufgabe angelegt ({e})");
                        }
                        Err(e2) => {
                            tracing::error!(insilo_id = %idea.insilo_id, "Idee: auch Fallback fehlgeschlagen: {e2}");
                            schedule_retry(state, &idea, attempts, &e2);
                        }
                    }
                } else {
                    tracing::warn!(insilo_id = %idea.insilo_id, attempt = attempts, "Idee fehlgeschlagen, Retry: {e}");
                    schedule_retry(state, &idea, attempts, &e);
                }
            }
        }
    }
}

/// Background loop: scan the idea directory, then process due ideas.
pub async fn spawn_loop(state: std::sync::Arc<AppState>, mut shutdown_rx: mpsc::Receiver<()>) {
    let interval = Duration::from_secs(
        std::env::var("RELAY_INSILO_IDEAS_SCAN_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(120),
    );
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                tracing::info!("Insilo-Ideen-Loop wurde gestoppt");
                break;
            }
            _ = ticker.tick() => {
                let report = run_ideas_scan(&state);
                if report.inserted + report.changed > 0 {
                    tracing::info!(
                        inserted = report.inserted,
                        changed = report.changed,
                        "Insilo-Ideen: neue/geänderte Ideen erkannt"
                    );
                }
                process_due_ideas(&state).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDEE: &str = r#"---
source: insilo
meeting_id: fda85ffe-751a-4b98-b459-04bc6f434c67
title: "Idea from 09/29 · 03:46 PM"
date: "2026-09-29T13:46:05.306000+00:00"
duration_min: 2
template: Schnellnotiz
language: de
tags: []
speakers:
  - Marc
  - Marc
---

# Idea from 09/29

[0:00] **Marc**: Wir sollten den Newsletter neu aufsetzen.
"#;

    #[test]
    fn parse_idea_liest_audio_schema() {
        let f = parse_idea(IDEE).unwrap();
        assert_eq!(f.insilo_id, "fda85ffe-751a-4b98-b459-04bc6f434c67");
        assert_eq!(f.title, "Idea from 09/29 · 03:46 PM");
        assert_eq!(f.duration_min, 2);
        assert!(f.transcript.contains("Newsletter"));
        assert!(!f.transcript.contains("meeting_id"));
    }

    #[test]
    fn is_idea_nutzt_prefix_case_insensitive() {
        let f = parse_idea(IDEE).unwrap();
        assert!(is_idea(&f, "Idea"));
        assert!(is_idea(&f, "idea"));
        assert!(!is_idea(&f, "Notiz"));
    }

    #[test]
    fn first_words_kappt_auf_acht() {
        let t = "\n\n# Titel\n\n[0:00] eins zwei drei vier fünf sechs sieben acht neun zehn";
        assert_eq!(first_words(t, 8), "eins zwei drei vier fünf sechs sieben acht");
    }

    #[test]
    fn infra_fehler_werden_erkannt() {
        assert!(is_infra_error("503 ServiceUnavailable: connect error"));
        assert!(is_infra_error("KI-System temporär nicht verfügbar (Circuit Breaker offen, 15s verbleibend)"));
        assert!(is_infra_error("request timeout after 120s"));
        assert!(!is_infra_error("Plan nicht lesbar: erwartet create/append"));
        assert!(!is_infra_error("KI-Client nicht konfiguriert"));
    }

    #[test]
    fn build_description_setzt_mighty_block_ab() {        let d = build_description("Essenz der Idee.", &["Mighty: Mail-Entwurf vorbereitet (nicht gesendet).".into()]);
        assert!(d.starts_with("Essenz der Idee."));
        assert!(d.contains("Mighty: Mail-Entwurf vorbereitet"));
    }

    #[test]
    fn scan_importiert_ideen_und_ignoriert_andere_aufnahmen() {
        use crate::AppState;

        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let org = tmp.path().join("org-1");
        std::fs::create_dir_all(&org).unwrap();
        std::fs::write(org.join("idea.transkript.md"), IDEE).unwrap();
        std::fs::write(
            org.join("meeting.transkript.md"),
            IDEE.replace("Idea from 09/29 · 03:46 PM", "Aufnahme vom 14.09."),
        )
        .unwrap();

        std::env::set_var("RELAY_INSILO_IDEAS_DIR", tmp.path());
        std::env::remove_var("RELAY_INSILO_IDEAS_ENABLED");
        std::env::set_var("RELAY_INSILO_IDEA_PREFIX", "Idea");

        let mut state = AppState::new();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::cache::db::init_db(&conn).unwrap();
        *state.cache_db.lock() = Some(conn);

        let report = run_ideas_scan(&state);
        assert_eq!(report.inserted, 1, "nur die Idee wird importiert");
        let due = with_db(&state, |conn| {
            cache::ideas::list_due_ideas(conn, 10).map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].insilo_id, "fda85ffe-751a-4b98-b459-04bc6f434c67");

        // Unverändert → kein erneuter Import.
        let again = run_ideas_scan(&state);
        assert_eq!(again.inserted, 0);
        assert_eq!(again.changed, 0);

        std::env::remove_var("RELAY_INSILO_IDEAS_DIR");
        std::env::remove_var("RELAY_INSILO_IDEA_PREFIX");
    }

    #[test]
    fn geaenderte_idee_wird_erneut_faellig() {
        use crate::AppState;

        let mut state = AppState::new();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::cache::db::init_db(&conn).unwrap();
        *state.cache_db.lock() = Some(conn);

        with_db(&state, |conn| {
            cache::ideas::upsert_idea(conn, "id-1", "/p", "sha-a", "T", "d", 1, "de", "alt")
                .map_err(|e| e.to_string())?;
            cache::ideas::mark_done(conn, "id-1", "task-1").map_err(|e| e.to_string())?;
            assert!(cache::ideas::list_due_ideas(conn, 10).unwrap().is_empty());
            cache::ideas::upsert_idea(conn, "id-1", "/p", "sha-b", "T", "d", 1, "de", "neu")
                .map_err(|e| e.to_string())?;
            let due = cache::ideas::list_due_ideas(conn, 10).unwrap();
            assert_eq!(due.len(), 1, "geänderter SHA macht die Idee wieder fällig");
            assert_eq!(due[0].attempts, 0);
            Ok(())
        })
        .unwrap();
    }
}
