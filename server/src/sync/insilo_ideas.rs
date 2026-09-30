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

use serde::{Deserialize, Serialize};
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

/// Idea drop directory: the shared Insilo drop (same as the meetings scanner)
/// unless an explicit override is set; `None` = disabled.
fn ideas_dir() -> Option<PathBuf> {
    if std::env::var("RELAY_INSILO_IDEAS_ENABLED").unwrap_or_default() == "false" {
        return None;
    }
    let explicit = std::env::var("RELAY_INSILO_IDEAS_DIR")
        .ok()
        .filter(|s| !s.is_empty());
    let dir = explicit.or_else(|| std::env::var("RELAY_INSILO_DIR").ok())?;
    if dir.is_empty() {
        return None;
    }
    Some(PathBuf::from(dir))
}

pub fn idea_prefix() -> String {
    std::env::var("RELAY_INSILO_IDEA_PREFIX")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Idea".to_string())
}

/// True when a memo title carries the configured "idea" prefix (case-insensitive).
pub fn is_idea_title(title: &str, prefix: &str) -> bool {
    let t = title.trim_start().to_lowercase();
    let p = prefix.trim().to_lowercase();
    !p.is_empty() && t.starts_with(&p)
}

/// Recursively collect `.md` files (the Insilo drop is flat).
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
    let mut candidates: Vec<(String, String, String, String, i64, String, String, String)> = Vec::new();
    for path in files {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(".tmp") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            report.skipped += 1;
            continue;
        };
        let Ok(p) = crate::sync::insilo::parse_frontmatter(&content) else {
            report.skipped += 1;
            continue;
        };
        if !is_idea_title(&p.title, &prefix) {
            continue; // a normal meeting/recording
        }
        report.scanned += 1;
        let sha = hex::encode(Sha256::digest(content.as_bytes()));
        candidates.push((
            p.insilo_id,
            path.to_string_lossy().to_string(),
            sha,
            p.title,
            p.duration_min,
            p.language,
            p.meeting_date,
            p.body,
        ));
    }

    let result = with_db(state, |conn| {
        let mut inserted = 0usize;
        let mut changed = 0usize;
        for (insilo_id, path, sha, title, duration_min, language, recorded_at, body) in &candidates {
            match cache::ideas::upsert_idea(
                conn,
                insilo_id,
                path,
                sha,
                title,
                recorded_at,
                *duration_min,
                language,
                body,
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

#[derive(Debug, Deserialize, Serialize, Default)]
struct PlanCalendar {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    start: String,
    #[serde(default)]
    end: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct PlanMail {
    #[serde(default)]
    to: String,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    body: String,
}

/// One task action derived from an idea.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
struct PlanItem {
    /// `create` (new task), `append` (add a note to target_uid's description)
    /// or `subtask` (new sub-task under target_uid).
    #[serde(default)]
    kind: String,
    #[serde(default)]
    target_uid: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    due: Option<String>,
    #[serde(default)]
    priority: Option<i64>,
    /// Blockers: existing task UIDs or the exact title of another item in the
    /// same plan that must be done first.
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    labels: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct IdeaPlan {
    #[serde(default)]
    items: Vec<PlanItem>,
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

/// Open tasks that share a meaningful (>4 chars) word with the memo content —
/// keeps the "existing task" list short so the model actually compares it.
fn relevant_tasks(content: &str, open_tasks: &[(String, String)]) -> Vec<(String, String)> {
    let hay = content.to_lowercase();
    let mut scored: Vec<(usize, &(String, String))> = open_tasks
        .iter()
        .filter_map(|t| {
            let lower = t.1.to_lowercase();
            let words: Vec<&str> = lower
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.chars().count() >= 5)
                .collect();
            let score = words.iter().filter(|w| hay.contains(**w)).count();
            if score > 0 {
                Some((score, t))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(15).map(|(_, t)| t.clone()).collect()
}

fn build_prompt(
    idea: &cache::ideas::IdeaRow,
    open_tasks: &[(String, String)],
    known_tags: &[(String, i64)],
) -> (String, String) {
    let system = "Du wandelst ein kurzes Sprachmemo (eine \"Idee\") in konkrete Aufgaben um. \
Antworte NUR mit einem JSON-Objekt, ohne Erklärtext.\n\
Regeln:\n\
- Prüfe ZUERST die gelisteten offenen Aufgaben: Hat eine dasselbe Thema wie das Memo?\n\
  * Gleiches Thema + reine Zusatzinfo/Ergänzung → kind \"append\" (target_uid): in deren Beschreibung ergänzen.\n\
  * Gleiches Thema + neuer, eigenständiger Schritt/Reaktion → kind \"subtask\" (target_uid): als Unteraufgabe.\n\
  * Kein passendes Thema → kind \"create\": neue, eigenständige Aufgabe.\n\
- Leite aus dem Memo die nötigen Aufgaben ab. Guideline: höchstens ~3 Aufgaben; nur wenn das Memo es hergibt.\n\
- title: maximal 8 Wörter, knapp und handlungsorientiert. note: maximal 2 kurze Zeilen. KEINE Romane, den Memo-Text nicht wiederholen.\n\
- due: wenn ein Zeitbezug genannt ist (\"bis Freitag\", \"morgen\", \"nächste Woche\", konkretes Datum), als YYYY-MM-DD (oder RFC 3339 mit Uhrzeit); sonst null. Heutiges Datum: {HEUTE}.\n\
- priority: 1-5 (1 = höchste … 5 = niedrigste) oder null. Kriterien: 1 = Frist heute/überfällig, blockiert anderes, explizit dringend; 2 = Frist in ≤ 3-5 Tagen, wichtig; 3 = normale Aufgabe ohne Zeitdruck; 4 = niedrig/Nice-to-have; 5 = Backlog ohne Zeitbezug. Konservativ bleiben, KEINE P1-Inflation, im Zweifel null.\n\
- depends_on: optionale Liste echter Abhängigkeiten (\"muss zuerst erledigt werden\"). Einträge sind entweder eine uid aus den gelisteten offenen Aufgaben ODER der EXAKTE Titel einer anderen Aufgabe aus DIESEM Plan, die zuerst kommen muss. Leer lassen, wenn keine zwingende Reihenfolge besteht.\n\
- labels: HÖCHSTENS EIN Themen-Tag pro Aufgabe, und nur wenn es ein fundamentales, wiederkehrendes Thema ist (kein Einzelfall/Feature). Bevorzuge STRENG einen der gelisteten vorhandenen Tags (exakte Schreibweise). Wenn keiner passt: leeres Array [] — erfinde KEINEN neuen Tag. Zusätzlich: setze NIE \"Insilo\", das wird automatisch vergeben.\n\
- calendar/mail NUR, wenn das Memo es klar verlangt. Mails werden NIE gesendet; Termine werden ohne Teilnehmer angelegt; dann als Entwurf/Termin vorbereitet.\n\
- mail.to MUSS eine E-Mail-Adresse sein. Ist nur ein Name bekannt, KEINE mail anlegen – stattdessen eine Aufgabe \"Mail an <Name> vorbereiten\".\n\
- Erfinde keine Fakten.\n\
Schema: {\"items\":[{\"kind\":\"create|append|subtask\",\"target_uid\":\"...\",\"title\":\"...\",\"note\":\"...\",\
\"due\":\"YYYY-MM-DD|null\",\"priority\":1-5|null,\"depends_on\":[\"uid|Titel\"],\"labels\":[\"...\"]}],\
\"calendar\":{\"summary\":\"...\",\"start\":\"RFC3339\",\"end\":\"RFC3339\"}|null,\
\"mail\":{\"to\":\"...\",\"subject\":\"...\",\"body\":\"...\"}|null}"
        .replace("{HEUTE}", &chrono::Local::now().format("%Y-%m-%d").to_string());
    let mut user = String::new();
    user.push_str(&format!(
        "Sprachmemo vom {} ({} min)\nTitel: {}\nInhalt (Insilo-Zusammenfassung):\n{}\n\n",
        idea.recorded_at,
        idea.duration_min,
        idea.title,
        crate::ai::agent::clamp_text(idea_content(&idea.transcript_md), 6000)
    ));
    let relevant = relevant_tasks(&idea.transcript_md, open_tasks);
    user.push_str("Relevante offene Aufgaben (uid | Titel):\n");
    if relevant.is_empty() {
        user.push_str("(keine passenden)\n");
    } else {
        for (uid, summary) in relevant.iter() {
            user.push_str(&format!("- {} | {}\n", uid, crate::ai::agent::clamp_text(summary, 100)));
        }
    }
    user.push_str("\nVorhandene Tags (Name | Anzahl Aufgaben) – bevorzugt wiederverwenden:\n");
    if known_tags.is_empty() {
        user.push_str("(keine)\n");
    } else {
        for (label, count) in known_tags.iter().take(30) {
            user.push_str(&format!("- {} ({})\n", label, count));
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

/// Create a fresh task (optionally as a sub-task) and return its uid.
/// The task starts with only the origin label; a thematic tag is applied
/// afterwards by the conservative tag policy.
async fn create_task(
    state: &AppState,
    title: &str,
    description: &str,
    due: Option<String>,
    priority: Option<i64>,
    parent_uid: Option<String>,
    dependencies: Vec<String>,
) -> Result<String, String> {
    let title = crate::ai::agent::clamp_text(title, 120);
    if title.trim().is_empty() {
        return Err("leerer Aufgabentitel".into());
    }
    let req = crate::api::todos::CreateTodoRequest {
        summary: title,
        description: if description.trim().is_empty() { None } else { Some(description.to_string()) },
        due: due.filter(|d| !d.trim().is_empty()),
        priority,
        labels: vec![crate::cache::tags::ORIGIN_LABEL.to_string()],
        rrule: None,
        parent_uid,
        dependencies,
        project_id: None,
        due_has_time: None,
    };
    let res = crate::api::todos::create_todo(axum::extract::State(state.clone()), axum::Json(req)).await;
    match res {
        Ok(axum::Json(row)) => Ok(row.uid),
        Err(e) => Err(e.0),
    }
}

/// Conservative tag policy for one freshly created task: reuse an active
/// label, otherwise park the proposal as a hidden candidate (promoted once it
/// covers THRESHOLD tasks). At most one thematic tag per task.
async fn apply_thematic_tag(state: &AppState, uid: &str, proposed: Option<&str>) {
    let Some(raw) = proposed.map(str::trim).filter(|s| !s.is_empty()) else {
        return;
    };
    let label = crate::cache::tags::normalize_label(raw);
    if label.is_empty() || label == crate::cache::tags::ORIGIN_LABEL {
        return;
    }
    let current = match with_db(state, |c| cache::todo::find_todo(c, uid).map_err(|e| e.to_string())) {
        Ok(Some(row)) => row,
        _ => return,
    };
    // Already carries a thematic tag → leave it (max 1).
    let has_thematic = current.labels.iter().any(|l| {
        let n = crate::cache::tags::normalize_label(l);
        n != crate::cache::tags::ORIGIN_LABEL && !n.is_empty()
    });
    if has_thematic {
        return;
    }
    let active = with_db(state, |c| cache::tags::active_labels(c)).unwrap_or_default();
    if active.contains_key(&label) {
        let _ = set_task_labels(state, uid, &current.labels, Some(&label)).await;
        return;
    }
    // Hidden candidate; promote once it reaches the threshold.
    let _ = with_db(state, |c| {
        cache::tags::add_candidate(c, &label, uid).map_err(|e| e.to_string())
    });
    let n = with_db(state, |c| cache::tags::candidate_count(c, &label).map_err(|e| e.to_string())).unwrap_or(0);
    if n >= crate::cache::tags::THRESHOLD {
        promote_label(state, &label).await;
    }
}

/// Replace a task's labels (keeps existing ones, optionally adds one label).
async fn set_task_labels(
    state: &AppState,
    uid: &str,
    current: &[String],
    add: Option<&str>,
) -> Result<(), String> {
    let mut labels: Vec<String> = current.to_vec();
    if let Some(l) = add {
        if !labels.iter().any(|x| x.eq_ignore_ascii_case(l)) {
            labels.push(l.to_string());
        }
    }
    let req = crate::api::todos::PatchTodoRequest {
        completed: None,
        summary: None,
        description: None,
        due: None,
        priority: None,
        labels: Some(labels),
        rrule: None,
        project_id: None,
        dependencies: None,
    };
    match crate::api::todos::toggle_todo(
        axum::extract::State(state.clone()),
        axum::extract::Path(uid.to_string()),
        axum::Json(req),
    )
    .await
    {
        Ok(_) => Ok(()),
        Err(e) => Err(e.0),
    }
}

/// Promote a candidate label: apply it to all its (still untagged) tasks.
async fn promote_label(state: &AppState, label: &str) {
    let uids = with_db(state, |c| {
        cache::tags::candidate_tasks(c, label).map_err(|e| e.to_string())
    })
    .unwrap_or_default();
    let mut applied = 0usize;
    for uid in &uids {
        let Some(row) = with_db(state, |c| cache::todo::find_todo(c, uid).map_err(|e| e.to_string()))
            .ok()
            .flatten()
        else {
            continue;
        };
        let has_other = row.labels.iter().any(|l| {
            let n = crate::cache::tags::normalize_label(l);
            n != crate::cache::tags::ORIGIN_LABEL && !n.is_empty() && n != label
        });
        if has_other {
            continue;
        }
        if set_task_labels(state, uid, &row.labels, Some(label)).await.is_ok() {
            applied += 1;
        }
    }
    let _ = with_db(state, |c| {
        cache::tags::remove_candidates(c, label).map_err(|e| e.to_string())
    });
    tracing::info!(label, applied, "Tag-Kandidat befördert");
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
        dependencies: None,
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
    // A draft needs a real address; a bare name would produce a broken draft.
    if !to.contains('@') {
        return Err(format!("Empfänger '{to}' ist keine E-Mail-Adresse"));
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

/// The meaningful part of an Insilo export body: from the first "## " section
/// on (skips the duplicated second front-matter block at the top).
fn idea_content(body: &str) -> &str {
    match body.find("\n## ") {
        Some(i) => body[i + 1..].trim_start(),
        None => body.trim_start(),
    }
}

/// First `n` words of the first meaningful content line (fallback title).
/// Skips headings and the Insilo metadata/front-matter lines.
fn first_words(text: &str, n: usize) -> String {
    let content = idea_content(text);
    for raw in content.lines() {
        let mut s = raw.trim();
        if s.is_empty() || s == "---" || s.starts_with('#') || s.starts_with('*') {
            continue;
        }
        // Front-matter-ish "key: value" lines (lowercase key, no space).
        if let Some((k, _)) = s.split_once(':') {
            if !k.contains(' ') && k.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                continue;
            }
        }
        s = s.trim_start_matches("- ").trim();
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
        if s.is_empty() {
            continue;
        }
        let words: Vec<&str> = s.split_whitespace().take(n).collect();
        return words.join(" ");
    }
    String::new()
}

/// Process a single idea end-to-end. `dry_run` returns the LLM plan without
/// writing anything. Errors are retried by the caller.
async fn process_one(
    state: &AppState,
    idea: &cache::ideas::IdeaRow,
    dry_run: bool,
) -> Result<IdeaPlan, String> {
    let client = {
        let guard = state.ai_client.read();
        guard.as_ref().cloned().ok_or("KI-Client nicht konfiguriert")?
    };

    // Idempotency: if a previous (possibly aborted) run already produced a task
    // for this idea, do not create a second one.
    if !dry_run {
        if let Some(uid) = idea.task_uid.as_deref().filter(|u| !u.is_empty()) {
            let uid = uid.to_string();
            with_db(state, |conn| {
                cache::ideas::mark_done(conn, &idea.insilo_id, &uid).map_err(|e| e.to_string())
            })?;
            return Ok(IdeaPlan::default());
        }
    }
    let open_tasks: Vec<(String, String)> = with_db(state, |conn| {
        cache::todo::list_todos(conn, Some(false)).map_err(|e| e.to_string())
    })?
    .into_iter()
    .map(|t| (t.uid, t.summary.unwrap_or_default()))
    .collect();

    // Existing tag vocabulary (active >= THRESHOLD + hidden candidates) so the
    // model reuses labels instead of inventing new ones.
    let mut tag_map: std::collections::HashMap<String, i64> = with_db(state, |c| {
        cache::tags::label_counts(c)
    })
    .unwrap_or_default();
    let candidates: std::collections::HashMap<String, i64> = with_db(state, |c| {
        let mut stmt = c
            .prepare("SELECT label, COUNT(*) FROM label_candidates GROUP BY label")
            .map_err(|e| e.to_string())?;
        let m = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<std::collections::HashMap<_, _>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(m)
    })
    .unwrap_or_default();
    for (label, n) in candidates {
        tag_map.entry(label).or_insert(n);
    }
    let mut tag_counts: Vec<(String, i64)> = tag_map.into_iter().collect();
    tag_counts.sort_by(|a, b| b.1.cmp(&a.1));

    let (system, user) = build_prompt(idea, &open_tasks, &tag_counts);
    let raw = client.complete_user_json(&system, &user, Some(0.2), Some(1500)).await?;
    let plan: IdeaPlan = serde_json::from_value(extract_json(&raw)?)
        .map_err(|e| format!("Plan nicht lesbar: {e}"))?;

    if dry_run {
        return Ok(plan);
    }

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

    let mut items = plan.items.clone();
    if items.len() > 5 {
        items.truncate(5); // sanity cap; the prompt already targets ~3
    }

    let mut primary: Option<String> = None;
    // Titles of tasks created within this plan → UID, for intra-plan depends_on.
    let mut created: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (idx, item) in items.iter().enumerate() {
        let extra: &[String] = if idx == 0 { &done } else { &[] };
        // Resolve depends_on: an existing open-task UID or a title created
        // earlier in this same plan.
        let deps: Vec<String> = item
            .depends_on
            .iter()
            .filter_map(|d| {
                let d = d.trim();
                if d.is_empty() {
                    return None;
                }
                if open_tasks.iter().any(|(u, _)| u == d) {
                    return Some(d.to_string());
                }
                created.get(&d.to_lowercase()).cloned()
            })
            .collect();
        match plan_item(state, &open_tasks, item, extra, &deps).await {
            Ok(Some(uid)) => {
                if let Some(title) = item.title.as_ref().filter(|t| !t.trim().is_empty()) {
                    created.insert(title.trim().to_lowercase(), uid.clone());
                }
                if primary.is_none() {
                    // Record immediately so a crash/retry cannot duplicate.
                    let _ = with_db(state, |conn| {
                        cache::ideas::set_task_uid(conn, &idea.insilo_id, &uid)
                            .map_err(|e| e.to_string())
                    });
                    primary = Some(uid);
                }
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(insilo_id = %idea.insilo_id, "Idee-Item fehlgeschlagen: {e}"),
        }
    }

    // Guarantee: every idea results in at least one task.
    let task_uid = match primary {
        Some(u) => u,
        None => {
            let title = {
                let t = first_words(&idea.transcript_md, 8);
                if t.is_empty() {
                    format!("Idee vom {}", idea.recorded_at)
                } else {
                    t
                }
            };
            let desc = build_description("", &done);
            create_task(state, &title, &desc, None, None, None, Vec::new()).await?
        }
    };

    with_db(state, |conn| {
        cache::ideas::mark_done(conn, &idea.insilo_id, &task_uid).map_err(|e| e.to_string())
    })?;
    Ok(plan)
}

/// Existing open task with the same normalised title (duplicate guard).
fn find_open_by_title(open_tasks: &[(String, String)], title: &str) -> Option<String> {
    let n = crate::api::todos::normalize_title(title);
    if n.is_empty() {
        return None;
    }
    open_tasks
        .iter()
        .find(|(_, s)| crate::api::todos::normalize_title(s) == n)
        .map(|(u, _)| u.clone())
}

/// Execute one plan item and return the affected task uid (None = skipped).
async fn plan_item(
    state: &AppState,
    open_tasks: &[(String, String)],
    item: &PlanItem,
    extra_done: &[String],
    dependencies: &[String],
) -> Result<Option<String>, String> {
    let kind = item.kind.trim().to_lowercase();
    let known = |u: &str| open_tasks.iter().any(|(x, _)| x == u);
    match kind.as_str() {
        "append" => {
            let Some(uid) = item.target_uid.clone().filter(|u| known(u)) else {
                return Ok(None);
            };
            let mut addition = String::new();
            if let Some(note) = item.note.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                addition.push_str("Mighty: ");
                addition.push_str(note);
            }
            if !extra_done.is_empty() {
                if !addition.is_empty() {
                    addition.push('\n');
                }
                addition.push_str(&extra_done.join("\n"));
            }
            if addition.is_empty() {
                addition.push_str("Mighty: Ergänzung aus Sprachmemo.");
            }
            Ok(Some(append_task(state, &uid, &addition).await?))
        }
        "subtask" => {
            let Some(uid) = item.target_uid.clone().filter(|u| known(u)) else {
                return Ok(None);
            };
            let title = item.title.clone().unwrap_or_default();
            if title.trim().is_empty() {
                return Ok(None);
            }
            let desc = build_description(&item.note.clone().unwrap_or_default(), extra_done);
            Ok(Some(
                create_task(state, &title, &desc, item.due.clone(), item.priority, Some(uid), dependencies.to_vec()).await?,
            ))
        }
        _ => {
            let title = item.title.clone().unwrap_or_default();
            if title.trim().is_empty() {
                return Ok(None);
            }
            // Duplicate of an existing open task → enrich it instead of a new task.
            if let Some(uid) = find_open_by_title(open_tasks, &title) {
                let mut addition = String::new();
                if let Some(note) = item.note.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    addition.push_str("Mighty: ");
                    addition.push_str(note);
                }
                if !extra_done.is_empty() {
                    if !addition.is_empty() {
                        addition.push('\n');
                    }
                    addition.push_str(&extra_done.join("\n"));
                }
                if addition.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(append_task(state, &uid, &addition).await?));
            }
            let desc = build_description(&item.note.clone().unwrap_or_default(), extra_done);
            let uid = create_task(state, &title, &desc, item.due.clone(), item.priority, None, dependencies.to_vec()).await?;
            apply_thematic_tag(state, &uid, item.labels.first().map(|s| s.as_str())).await;
            Ok(Some(uid))
        }
    }
}

/// Fallback: never lose an idea when the LLM keeps failing.
async fn create_fallback_task(state: &AppState, idea: &cache::ideas::IdeaRow) -> Result<String, String> {
    let mut summary = first_words(&idea.transcript_md, 8);
    if summary.is_empty() {
        summary = format!("Idee vom {}", idea.recorded_at);
    }
    let desc = "Mighty: Automatische Verarbeitung fehlgeschlagen – Inhalt bitte prüfen.";
    create_task(state, &summary, desc, None, None, None, Vec::new()).await
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
        cache::ideas::claim_due_ideas(conn, 5).map_err(|e| e.to_string())
    }) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Ideen-Queue nicht lesbar: {e}");
            return;
        }
    };
    for idea in due {
        match process_one(state, &idea, false).await {
            Ok(_) => tracing::info!(insilo_id = %idea.insilo_id, "Idee verarbeitet"),
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

/// Scan + process now (manual trigger / QA). `dry_run` returns the LLM plans
/// without writing anything. Processes up to `limit` due ideas.
pub async fn process_now(
    state: &AppState,
    limit: i64,
    dry_run: bool,
) -> crate::api::ideas::ProcessIdeasResult {
    let scan = run_ideas_scan(state);
    let due = with_db(state, |conn| {
        cache::ideas::claim_due_ideas(conn, limit).map_err(|e| e.to_string())
    })
    .unwrap_or_default();

    let mut processed = 0usize;
    let mut failed = 0usize;
    let mut plans: Vec<serde_json::Value> = Vec::new();

    for idea in due {
        match process_one(state, &idea, dry_run).await {
            Ok(plan) => {
                processed += 1;
                if dry_run {
                    plans.push(serde_json::json!({
                        "insilo_id": idea.insilo_id,
                        "title": idea.title,
                        "recorded_at": idea.recorded_at,
                        "plan": plan,
                    }));
                }
            }
            Err(e) => {
                failed += 1;
                if is_infra_error(&e) {
                    schedule_infra_retry(state, &idea, &e);
                } else {
                    let attempts = idea.attempts + 1;
                    if attempts >= BACKOFF_SECS.len() as i64 {
                        match create_fallback_task(state, &idea).await {
                            Ok(uid) => {
                                let _ = with_db(state, |c| {
                                    cache::ideas::mark_done(c, &idea.insilo_id, &uid)
                                        .map_err(|e| e.to_string())
                                });
                            }
                            Err(e2) => schedule_retry(state, &idea, attempts, &e2),
                        }
                    } else {
                        schedule_retry(state, &idea, attempts, &e);
                    }
                }
            }
        }
    }

    crate::api::ideas::ProcessIdeasResult {
        scanned_inserted: scan.inserted,
        scanned_changed: scan.changed,
        processed,
        failed,
        dry_run,
        plans,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDEE: &str = r#"---
insilo_id: "fda85ffe-751a-4b98-b459-04bc6f434c67"
title: "Idea from 09/29 · 03:46 PM"
recorded_at: "2026-09-29T13:46:05.306000+00:00"
duration_min: 2
language: "de"
participants: ["Marc"]
tags: []
template: "Schnellnotiz"
crm: false
source_url: ''
schema: 1
---

## Kerninhalt

- Newsletter neu aufsetzen.

## Offene Aufgaben

- Newsletter-Konzept erstellen
"#;

    #[test]
    fn common_idee_wird_geparst() {
        let p = crate::sync::insilo::parse_frontmatter(IDEE).unwrap();
        assert_eq!(p.insilo_id, "fda85ffe-751a-4b98-b459-04bc6f434c67");
        assert_eq!(p.title, "Idea from 09/29 · 03:46 PM");
        assert_eq!(p.duration_min, 2);
        assert!(p.body.contains("Newsletter"));
    }

    #[test]
    fn is_idea_nutzt_prefix_case_insensitive() {
        assert!(is_idea_title("Idea from 09/29", "Idea"));
        assert!(is_idea_title("idea vom 29.", "Idea"));
        assert!(!is_idea_title("Aufnahme vom 14.09.", "Idea"));
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
    fn claim_verhindert_doppelverarbeitung() {
        use crate::AppState;
        let mut state = AppState::new();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::cache::db::init_db(&conn).unwrap();
        *state.cache_db.lock() = Some(conn);

        with_db(&state, |conn| {
            cache::ideas::upsert_idea(conn, "i1", "/p", "s1", "T1", "d", 1, "de", "x")
                .map_err(|e| e.to_string())?;
            cache::ideas::upsert_idea(conn, "i2", "/p", "s2", "T2", "d", 1, "de", "y")
                .map_err(|e| e.to_string())?;
            let first = cache::ideas::claim_due_ideas(conn, 10).unwrap();
            assert_eq!(first.len(), 2, "beide Ideen werden geclaimt");
            let second = cache::ideas::claim_due_ideas(conn, 10).unwrap();
            assert!(second.is_empty(), "zweiter Claim liefert nichts (kein Doppellauf)");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn find_open_by_title_ignoriert_punktuation() {
        let tasks = vec![
            ("u1".to_string(), "LLM-Router konfigurieren".to_string()),
            ("u2".to_string(), "Rechnung senden".to_string()),
        ];
        assert_eq!(find_open_by_title(&tasks, "llm router konfigurieren"), Some("u1".into()));
        assert_eq!(find_open_by_title(&tasks, "Etwas anderes"), None);
    }

    #[test]
    fn relevante_aufgaben_werden_gefiltert() {
        let tasks = vec![
            ("u1".to_string(), "Release 1.0 Analysten vorbereiten".to_string()),
            ("u2".to_string(), "Rechnung an Nordlicht senden".to_string()),
        ];
        let r = relevant_tasks("Wir müssen das Release für Analysten finalisieren", &tasks);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0, "u1");
    }

    #[test]
    fn build_description_setzt_mighty_block_ab() {
        let d = build_description("Essenz der Idee.", &["Mighty: Mail-Entwurf vorbereitet (nicht gesendet).".into()]);
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
        std::fs::write(org.join("idea.md"), IDEE).unwrap();
        std::fs::write(
            org.join("meeting.md"),
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
