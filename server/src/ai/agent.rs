//! Agent loop (Concept §5.2). Native function-calling with hard budgets:
//! ≤ 5 steps, 120 s timeout, client-disconnect abort. Read tools run
//! immediately; Write/External tools collect cards into a server-side plan
//! (nothing is written until the user confirms). `status`/`plan`/`effect`/
//! `done` events are emitted on an optional channel for SSE bridging.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::ai::client::{AIClient, ChatCompletionResult, ChatMessage, ToolSpec};
use crate::ai::plaene::{self, ActionPlan};
use crate::ai::prompts;
use crate::ai::sessions::{self, SessionMessage};
use crate::ai::tools::{self, PreparedCard, ToolCtx, ToolOutcome};
use crate::db::with_db;
use crate::AppState;

/// Hard step budget (Concept §5.2.5, §6.6).
pub const MAX_STEPS: usize = 5;
/// Per-tool-result byte cap before truncation (Concept §5.2.5).
pub const MAX_TOOL_RESULT_BYTES: usize = 16 * 1024;
/// Whole-loop timeout (Concept §5.2.5).
pub const LOOP_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Deserialize, Clone)]
pub struct AgentRequest {
    pub message: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
    /// 'chat' | 'mail_followup' (Concept §6.5).
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub source_message_id: Option<i64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct AgentStep {
    pub tool: String,
    pub label: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct AgentResult {
    pub answer: String,
    pub plans: Vec<ActionPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub navigation: Option<String>,
    pub effects: Vec<serde_json::Value>,
    pub steps: Vec<AgentStep>,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nachfrage: Option<String>,
}

/// A streamable agent event (SSE bridging, Concept §9.1).
pub enum AgentEvent {
    Status { step: String, label: String },
    Plan { plan: ActionPlan },
    Effect { effect: serde_json::Value },
    Done { result: AgentResult },
}

/// One LLM round-trip. Production builds this from [`AIClient`]; tests inject a
/// scripted model (Concept §5.2, mock-LLM loop tests).
pub type Completer = Box<
    dyn FnMut(Vec<ChatMessage>)
        -> Pin<Box<dyn std::future::Future<Output = Result<ChatCompletionResult, String>> + Send>>
        + Send,
>;

/// `se-` + 8 hex chars.
pub fn new_session_id() -> String {
    let u = uuid::Uuid::new_v4().simple().to_string();
    format!("se-{}", &u[..8])
}

fn get_client(state: &AppState) -> Result<Arc<AIClient>, String> {
    let guard = state.ai_client.read();
    guard
        .as_ref()
        .cloned()
        .ok_or_else(|| "KI-Client nicht konfiguriert".to_string())
}

/// A short, locale-aware status label for a tool (Concept §9.1 `status` event).
pub fn tool_label(tool: &str, locale: &str) -> String {
    let de = locale != "en";
    let s = if de {
        match tool {
            "contacts_search" => "sucht in Kontakten…",
            "contacts_get" => "lädt Kontakt…",
            "contacts_create" => "bereitet Kontakt vor…",
            "mail_search" => "sucht in Mails…",
            "mail_get" => "lädt Mail…",
            "mail_propose_reply" => "entwirft Antwort…",
            "calendar_list_events" => "liest Termine…",
            "calendar_find_free_slots" => "prüft freie Slots…",
            "calendar_create_event" => "bereitet Termin vor…",
            "calendar_update_event" => "bereitet Terminänderung vor…",
            "calendar_delete_event" => "bereitet Löschung vor…",
            "calendar_invite" => "bereitet Einladung vor…",
            "calendar_rsvp" => "bereitet Antwort vor…",
            "tasks_list" => "liest Aufgaben…",
            "tasks_create" => "bereitet Aufgabe vor…",
            "tasks_toggle" => "bereitet Statusänderung vor…",
            "tasks_delete" => "bereitet Löschung vor…",
            _ if tool.starts_with("ui_") => "navigiert…",
            _ => "arbeitet…",
        }
    } else {
        match tool {
            "contacts_search" => "searching contacts…",
            "mail_search" => "searching mail…",
            "calendar_list_events" => "reading events…",
            "calendar_find_free_slots" => "checking free slots…",
            "tasks_list" => "reading tasks…",
            _ if tool.starts_with("ui_") => "navigating…",
            _ => "working…",
        }
    };
    s.to_string()
}

/// Truncate a tool-result JSON to `max_bytes` (Concept §5.2.5). Over-cap results
/// become a plain string with a truncation marker so the model can still read it.
pub fn clamp_json(v: &serde_json::Value, max_bytes: usize) -> serde_json::Value {
    let s = v.to_string();
    if s.len() <= max_bytes {
        return v.clone();
    }
    let mut out: String = s.chars().take(max_bytes).collect();
    out.push_str("…[gekürzt]");
    serde_json::Value::String(out)
}

fn tool_msg(id: &str, content: &str) -> ChatMessage {
    ChatMessage {
        role: "tool".to_string(),
        content: content.to_string(),
        tool_calls: None,
        tool_call_id: Some(id.to_string()),
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Production entry point (Concept §5.2): builds the real completer from the
/// configured [`AIClient`] and runs the loop. Emits `status`/`plan`/`effect`/
/// `done` events on `tx` when provided (SSE); returns the final result anyway.
pub async fn run_agent(
    state: &AppState,
    req: &AgentRequest,
    tx: Option<mpsc::UnboundedSender<AgentEvent>>,
) -> Result<AgentResult, String> {
    let client = get_client(state)?;
    let lang = req.lang.clone().unwrap_or_else(|| "de".to_string());
    let tool_specs: Vec<ToolSpec> = tools::all_tools(&lang).iter().map(|t| t.spec.clone()).collect();
    let mut completer: Completer = Box::new(move |messages| {
        let client = client.clone();
        let tool_specs = tool_specs.clone();
        Box::pin(async move {
            client
                .complete_with_tools(messages, tool_specs, Some("auto"), Some(0.0), Some(1500))
                .await
        })
    });
    run_agent_core(state, req, tx, &mut completer).await
}

/// The agent loop, decoupled from the concrete LLM client so tests can inject a
/// scripted model (Concept §5.2, mock-LLM loop tests). Bounded by [`LOOP_TIMEOUT`].
pub async fn run_agent_core(
    state: &AppState,
    req: &AgentRequest,
    tx: Option<mpsc::UnboundedSender<AgentEvent>>,
    completer: &mut Completer,
) -> Result<AgentResult, String> {
    let origin = req.origin.clone().unwrap_or_else(|| "chat".to_string());
    let lang = req.lang.clone().unwrap_or_else(|| "de".to_string());
    let session_id = req
        .session_id
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(new_session_id);

    // Expire stale plans before deciding (Concept §5.3).
    let _ = with_db(state, |conn| plaene::expire_pending(conn)).ok();

    let heute = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let system = prompts::build_agent_prompt(&lang, &heute);

    // Messages: system + session history (text roles only) + current message.
    let mut messages: Vec<ChatMessage> = vec![ChatMessage::text("system", system)];
    if let Ok(Some(hist)) = with_db(state, |conn| sessions::get_session(conn, &session_id)) {
        for m in &hist.messages {
            if m.role == "user" || m.role == "assistant" {
                if !m.text.trim().is_empty() {
                    messages.push(ChatMessage::text(&m.role, &m.text));
                }
            }
        }
    }
    messages.push(ChatMessage::text("user", &req.message));

    let known_ids = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let ctx = ToolCtx {
        state: state.clone(),
        locale: lang.clone(),
        known_ids,
    };

    let mut cards: Vec<PreparedCard> = Vec::new();
    let mut effects: Vec<serde_json::Value> = Vec::new();
    let mut navigation: Option<String> = None;
    let mut steps: Vec<AgentStep> = Vec::new();
    let mut nachfrage: Option<String> = None;
    let mut answer = String::new();
    let mut got_answer = false;

    let loop_body = async {
        for _ in 0..MAX_STEPS {
            // Abort when the SSE client disconnected (receiver dropped).
            if let Some(tx) = &tx {
                if tx.is_closed() {
                    break;
                }
            }
            let result = completer(messages.clone()).await?;

            if result.tool_calls.is_empty() {
                answer = result.content;
                got_answer = true;
                break;
            }

            // Record the assistant's tool-call turn.
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: result.content.clone(),
                tool_calls: Some(result.tool_calls.clone()),
                tool_call_id: None,
            });

            for tc in &result.tool_calls {
                let tool_name = tc.function.name.clone();
                let args: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::Value::Null);
                let label = tool_label(&tool_name, &lang);
                if let Some(tx) = &tx {
                    let _ = tx.send(AgentEvent::Status { step: tool_name.clone(), label: label.clone() });
                }
                steps.push(AgentStep { tool: tool_name.clone(), label });
                let _ = with_db(state, |conn| {
                    crate::ai::audit::log_tool_call(conn, Some(&session_id), &origin, &tool_name, &tc.function.arguments)
                        .map_err(|e| e.to_string())?;
                    Ok(())
                });

                match tools::execute(&tool_name, args, ctx.clone()).await {
                    Ok(ToolOutcome::Data(v)) => {
                        let clamped = clamp_json(&v, MAX_TOOL_RESULT_BYTES);
                        messages.push(tool_msg(&tc.id, &clamped.to_string()));
                    }
                    Ok(ToolOutcome::Card(c)) => {
                        cards.push(c.clone());
                        messages.push(tool_msg(&tc.id, &format!("Karte vorbereitet: {}", c.title)));
                    }
                    Ok(ToolOutcome::Nachfrage(s)) => {
                        nachfrage = Some(s.clone());
                        messages.push(tool_msg(&tc.id, &format!("Rückfrage: {s}")));
                    }
                    Ok(ToolOutcome::Nav(json)) => {
                        if let Ok(eff) = serde_json::from_str::<serde_json::Value>(&json) {
                            if let Some(tx) = &tx {
                                let _ = tx.send(AgentEvent::Effect { effect: eff.clone() });
                            }
                            effects.push(eff);
                            navigation = Some(json);
                        }
                        messages.push(tool_msg(&tc.id, "Navigation vorbereitet."));
                    }
                    Err(e) => {
                        messages.push(tool_msg(&tc.id, &format!("Fehler: {e}")));
                    }
                }
            }
        }
        Ok::<(), String>(())
    };

    let outcome = tokio::time::timeout(LOOP_TIMEOUT, loop_body).await;
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            // Timeout: deliver what we have (cards) with a standard note.
            if !got_answer {
                answer = if lang == "en" {
                    "Please split the request into smaller steps.".to_string()
                } else {
                    "Bitte teile den Auftrag in kleinere Schritte.".to_string()
                };
            }
        }
    }

    if !got_answer && answer.is_empty() {
        answer = if lang == "en" {
            "Please split the request into smaller steps.".to_string()
        } else {
            "Bitte teile den Auftrag in kleinere Schritte.".to_string()
        };
    }

    // Build + persist a plan from the collected cards (Concept §5.2.7).
    let mut plans: Vec<ActionPlan> = Vec::new();
    if !cards.is_empty() {
        let plan = plaene::build_plan(Some(session_id.clone()), &origin, req.source_message_id, cards);
        let created = with_db(state, |conn| {
            plaene::create_plan(conn, &plan)?;
            crate::ai::audit::log_plan(conn, Some(&session_id), &origin, &plan.id, "created")
                .map_err(|e| e.to_string())?;
            Ok(())
        });
        if created.is_ok() {
            if let Some(tx) = &tx {
                let _ = tx.send(AgentEvent::Plan { plan: plan.clone() });
            }
            plans.push(plan);
        }
    }

    // Persist session history (user + assistant text only).
    let _ = with_db(state, |conn| {
        sessions::append_message(
            conn,
            &session_id,
            &lang,
            SessionMessage {
                role: "user".to_string(),
                text: req.message.clone(),
                plan_ids: vec![],
                ts: now_rfc3339(),
            },
        )?;
        sessions::append_message(
            conn,
            &session_id,
            &lang,
            SessionMessage {
                role: "assistant".to_string(),
                text: answer.clone(),
                plan_ids: plans.iter().map(|p| p.id.clone()).collect(),
                ts: now_rfc3339(),
            },
        )?;
        Ok(())
    });

    let result = AgentResult {
        answer,
        plans,
        navigation,
        effects,
        steps,
        session_id: session_id.clone(),
        nachfrage,
    };
    if let Some(tx) = &tx {
        let _ = tx.send(AgentEvent::Done { result: result.clone() });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use crate::ai::client::{ToolCall, ToolCallFunction};

    /// An [`AppState`] backed by an in-memory DB with the three agent tables.
    fn test_state() -> AppState {
        let state = AppState::new();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE ai_action_plans (
                 id TEXT PRIMARY KEY, session_id TEXT, origin TEXT NOT NULL,
                 source_message_id INTEGER, status TEXT NOT NULL, steps_json TEXT NOT NULL,
                 created_at TEXT NOT NULL, expires_at TEXT NOT NULL, executed_at TEXT, result_json TEXT);
             CREATE TABLE ai_sessions (
                 id TEXT PRIMARY KEY, created_at TEXT, last_active TEXT, locale TEXT,
                 messages_json TEXT NOT NULL DEFAULT '[]');
             CREATE TABLE ai_audit (
                 id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT, origin TEXT,
                 event TEXT NOT NULL, detail TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')));",
        )
        .unwrap();
        *state.cache_db.lock() = Some(conn);
        state
    }

    /// A scripted model: pops the next canned response per call (Concept §5.2 mock-LLM).
    fn scripted(responses: Vec<ChatCompletionResult>) -> Completer {
        let q = Arc::new(std::sync::Mutex::new(responses.into_iter().collect::<VecDeque<_>>()));
        Box::new(move |_messages| {
            let q = q.clone();
            Box::pin(async move {
                let mut g = q.lock().unwrap();
                Ok(g.pop_front().unwrap_or(ChatCompletionResult {
                    content: "fertig".into(),
                    tool_calls: vec![],
                }))
            })
        })
    }

    fn tool_call(name: &str, args: &str) -> ChatCompletionResult {
        ChatCompletionResult {
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                call_type: "function".into(),
                function: ToolCallFunction {
                    name: name.into(),
                    arguments: args.into(),
                },
            }],
        }
    }

    fn final_answer(content: &str) -> ChatCompletionResult {
        ChatCompletionResult { content: content.into(), tool_calls: vec![] }
    }

    fn req(msg: &str) -> AgentRequest {
        AgentRequest {
            message: msg.into(),
            session_id: None,
            module: None,
            lang: Some("de".into()),
            origin: Some("chat".into()),
            source_message_id: None,
        }
    }

    #[test]
    fn tool_label_de_en_fallback() {
        assert_eq!(tool_label("contacts_search", "de"), "sucht in Kontakten…");
        assert_eq!(tool_label("contacts_search", "en"), "searching contacts…");
        assert_eq!(tool_label("ui_navigate", "de"), "navigiert…");
        assert_eq!(tool_label("unknown_tool", "de"), "arbeitet…");
    }

    #[test]
    fn clamp_json_under_cap_unchanged() {
        let v = serde_json::json!({"a": 1});
        assert_eq!(clamp_json(&v, 1024), v);
    }

    #[test]
    fn clamp_json_over_cap_truncated() {
        let v = serde_json::json!({"a": "x".repeat(100)});
        let out = clamp_json(&v, 20);
        assert!(out.is_string());
        let s = out.as_str().unwrap();
        assert!(s.ends_with("…[gekürzt]"));
    }

    #[test]
    fn new_session_id_format() {
        let id = new_session_id();
        assert!(id.starts_with("se-"));
        assert_eq!(id.len(), 11);
    }

    #[tokio::test]
    async fn loop_collects_card_into_plan_and_persists_session() {
        let state = test_state();
        let mut completer = scripted(vec![
            tool_call("tasks_create", "{\"summary\":\"Rückruf\"}"),
            final_answer("Ich habe eine Aufgabe vorbereitet."),
        ]);
        let result = run_agent_core(&state, &req("Erinner mich an den Rückruf."), None, &mut completer)
            .await
            .unwrap();
        assert_eq!(result.answer, "Ich habe eine Aufgabe vorbereitet.");
        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.steps[0].tool, "tasks_create");
        assert_eq!(result.plans.len(), 1);
        assert_eq!(result.plans[0].steps.len(), 1);
        assert!(result.plans[0].id.starts_with("pl-"));
        let sess = with_db(&state, |conn| sessions::get_session(conn, &result.session_id))
            .unwrap()
            .unwrap();
        assert_eq!(sess.messages.len(), 2);
        assert_eq!(sess.messages[0].role, "user");
        assert_eq!(sess.messages[1].role, "assistant");
    }

    #[tokio::test]
    async fn loop_emits_status_plan_done_events() {
        let state = test_state();
        let mut completer = scripted(vec![
            tool_call("tasks_create", "{\"summary\":\"X\"}"),
            final_answer("fertig"),
        ]);
        let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();
        let result = run_agent_core(&state, &req("m"), Some(tx), &mut completer)
            .await
            .unwrap();
        assert_eq!(result.plans.len(), 1);
        let mut got_status = false;
        let mut got_plan = false;
        let mut got_done = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                AgentEvent::Status { .. } => got_status = true,
                AgentEvent::Plan { .. } => got_plan = true,
                AgentEvent::Effect { .. } => {}
                AgentEvent::Done { .. } => got_done = true,
            }
        }
        assert!(got_status, "expected a status event");
        assert!(got_plan, "expected a plan event");
        assert!(got_done, "expected a done event");
    }

    /// Contract (Concept §12.5): „nichts geschrieben vor Bestätigung". A write
    /// tool call in the loop must NOT touch the data table — it yields a
    /// prepared card, and the todos count stays unchanged until the user
    /// confirms (execution is a separate, server-side step).
    #[tokio::test]
    async fn contract_no_write_before_confirmation() {
        let state = test_state();
        with_db(&state, |conn| {
            conn.execute_batch(
                "CREATE TABLE todos (
                     id INTEGER PRIMARY KEY AUTOINCREMENT, calendar_id INTEGER NOT NULL,
                     uid TEXT NOT NULL, url TEXT NOT NULL, summary TEXT, description TEXT,
                     due_at TEXT, completed_at TEXT, status TEXT NOT NULL DEFAULT 'NEEDS-ACTION',
                     priority INTEGER, ics_raw TEXT NOT NULL,
                     synced_at TEXT NOT NULL DEFAULT (datetime('now')),
                     updated_at TEXT NOT NULL DEFAULT (datetime('now')));
                 INSERT INTO todos (calendar_id, uid, url, summary, ics_raw)
                 VALUES (1, 'seed-1', 'http://x/1', 'Bestand', 'BEGIN:VTODO');",
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
        let count = |s: &AppState| -> i64 {
            with_db(s, |conn| {
                conn.query_row("SELECT COUNT(*) FROM todos", [], |r| r.get(0))
                    .map_err(|e| e.to_string())
            })
            .unwrap()
        };
        assert_eq!(count(&state), 1, "seed row present before the agent call");

        let mut completer = scripted(vec![
            tool_call("tasks_create", "{\"summary\":\"Neu\"}"),
            final_answer("Aufgabe vorbereitet."),
        ]);
        let result = run_agent_core(&state, &req("Leg eine Aufgabe an: Neu"), None, &mut completer)
            .await
            .unwrap();

        // Core contract: the loop wrote nothing to the data table.
        assert_eq!(
            count(&state),
            1,
            "agent loop must not write to the data table before confirmation"
        );
        // …but it must have produced a prepared (unexecuted) card.
        assert_eq!(result.plans.len(), 1);
        let card = &result.plans[0].steps[0];
        assert_eq!(card.tool, "tasks_create");
        assert_eq!(card.tier, crate::ai::tools::Tier::Write);
        assert_eq!(card.request.method, "POST");
        assert!(card.request.path.ends_with("/todos"));
    }

    /// Seed the read-only data tables the acceptance scenarios rely on
    /// (Concept §2): one writable calendar, one contact "Kai Meyer", empty events.
    fn seed_data_tables(state: &AppState) {
        with_db(state, |conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS calendars (
                     id INTEGER PRIMARY KEY AUTOINCREMENT, url TEXT UNIQUE NOT NULL, display_name TEXT);
                 INSERT INTO calendars (url, display_name) VALUES ('http://cal/1', 'Privat');
                 CREATE TABLE IF NOT EXISTS contacts (
                     id INTEGER PRIMARY KEY AUTOINCREMENT, vcard_uid TEXT UNIQUE NOT NULL,
                     given_name TEXT, family_name TEXT, display_name TEXT, email TEXT, phone TEXT,
                     organization TEXT, vcard_raw TEXT NOT NULL, source TEXT NOT NULL DEFAULT 'carddav',
                     synced_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL DEFAULT (datetime('now')));
                 INSERT INTO contacts (vcard_uid, given_name, family_name, display_name, email, phone, vcard_raw)
                     VALUES ('kai-1', 'Kai', 'Meyer', 'Kai Meyer', 'kai@example.com', '+49 170 1234567', 'BEGIN:VCARD');
                 CREATE TABLE IF NOT EXISTS events (
                     id INTEGER PRIMARY KEY AUTOINCREMENT, calendar_id INTEGER NOT NULL,
                     uid TEXT NOT NULL, url TEXT NOT NULL, summary TEXT, start_at TEXT NOT NULL,
                     end_at TEXT, all_day INTEGER NOT NULL DEFAULT 0, ics_raw TEXT NOT NULL,
                     synced_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL DEFAULT (datetime('now')));",
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
    }

    /// S1 (Concept §2): an info query resolves via a read tool, returns the
    /// answer + the source step, and produces NO write plan.
    #[tokio::test]
    async fn s1_info_query_returns_answer_with_source() {
        let state = test_state();
        seed_data_tables(&state);
        let mut completer = scripted(vec![
            tool_call("contacts_search", "{\"query\":\"Kai\"}"),
            final_answer("Kais Nummer: +49 170 1234567."),
        ]);
        let result = run_agent_core(&state, &req("Wie ist die Handynummer von Kai?"), None, &mut completer)
            .await
            .unwrap();
        assert_eq!(result.answer, "Kais Nummer: +49 170 1234567.");
        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.steps[0].tool, "contacts_search");
        assert!(result.plans.is_empty(), "a pure info query must not produce a write plan");
        assert!(result.nachfrage.is_none());
    }

    /// S1.3 (Concept §2): when the model cannot resolve the reference it asks
    /// back — surfaced via the tool's `Nachfrage`, carried on the result.
    #[tokio::test]
    async fn s1_unresolvable_reference_triggers_nachfrage() {
        let state = test_state();
        let mut completer = scripted(vec![
            tool_call("contacts_search", "{}"), // empty query -> tool asks back
            final_answer("Meinst du Kai Meyer oder Kai Sommer?"),
        ]);
        let result = run_agent_core(&state, &req("Kai?"), None, &mut completer).await.unwrap();
        assert!(result.nachfrage.is_some(), "an unresolvable reference must set nachfrage");
        assert!(result.plans.is_empty());
    }

    /// S2 (Concept §2): a single action yields a prepared card (no write until
    /// the user confirms) with the correct target path and preview rows.
    #[tokio::test]
    async fn s2_single_action_produces_card_without_write() {
        let state = test_state();
        seed_data_tables(&state);
        let events_count = || -> i64 {
            with_db(&state, |c| c.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0)).map_err(|e| e.to_string())).unwrap()
        };
        assert_eq!(events_count(), 0);
        let mut completer = scripted(vec![
            tool_call("calendar_create_event", "{\"summary\":\"Dentist\",\"start\":\"2026-09-08T14:00:00Z\"}"),
            final_answer("Termin vorbereitet."),
        ]);
        let result = run_agent_core(&state, &req("Erstelle einen Termin für morgen 14 Uhr."), None, &mut completer)
            .await
            .unwrap();
        assert_eq!(events_count(), 0, "no event may be written before confirmation");
        assert_eq!(result.plans.len(), 1);
        let card = &result.plans[0].steps[0];
        assert_eq!(card.tool, "calendar_create_event");
        assert_eq!(card.tier, crate::ai::tools::Tier::Write);
        assert_eq!(card.request.method, "POST");
        assert_eq!(card.request.path, "/api/v1/calendars/events");
        assert!(card.rows.iter().any(|(k, v)| k == "Beginn" && v == "2026-09-08T14:00:00Z"));
    }

    /// S3 (Concept §2): an orchestrated chain runs several read steps and ends
    /// in exactly one write card; the full tool trace is preserved.
    #[tokio::test]
    async fn s3_orchestrated_chain_reads_then_single_write_card() {
        let state = test_state();
        seed_data_tables(&state);
        let mut completer = scripted(vec![
            tool_call("calendar_find_free_slots", "{\"start\":\"2026-09-08T13:00:00Z\",\"end\":\"2026-09-08T18:00:00Z\",\"duration_minutes\":30}"),
            tool_call("contacts_search", "{\"query\":\"Kai\"}"),
            tool_call("calendar_create_event", "{\"summary\":\"Meeting Kai\",\"start\":\"2026-09-08T15:00:00Z\",\"attendees\":[\"kai@example.com\"]}"),
            final_answer("Ich habe den Termin mit Einladung vorbereitet."),
        ]);
        let result = run_agent_core(&state, &req("Such den nächsten freien Termin und lade Kai ein."), None, &mut completer)
            .await
            .unwrap();
        // Full trace: all three tool calls are recorded in order.
        assert_eq!(result.steps.len(), 3);
        assert_eq!(result.steps[0].tool, "calendar_find_free_slots");
        assert_eq!(result.steps[1].tool, "contacts_search");
        assert_eq!(result.steps[2].tool, "calendar_create_event");
        // Only the write becomes a card in the plan.
        assert_eq!(result.plans.len(), 1);
        assert_eq!(result.plans[0].steps.len(), 1);
        assert_eq!(result.plans[0].steps[0].tool, "calendar_create_event");
    }

    /// S6 (Concept §2): when the SSE client disconnects, the loop detects the
    /// closed channel and stops instead of hanging.
    #[tokio::test]
    async fn s6_abort_when_client_disconnects() {
        let state = test_state();
        let mut completer = scripted(vec![
            tool_call("tasks_create", "{\"summary\":\"X\"}"),
            final_answer("fertig"),
        ]);
        let (tx, rx) = mpsc::unbounded_channel::<AgentEvent>();
        drop(rx); // simulate the client leaving mid-run
        let result = run_agent_core(&state, &req("m"), Some(tx), &mut completer).await.unwrap();
        assert!(result.steps.is_empty(), "loop must break at the first iteration once the client is gone");
    }

    /// S6 (Concept §2): with no LLM endpoint configured, the entry point fails
    /// with the clear "not configured" error (the API maps it to a 409).
    #[tokio::test]
    async fn s6_not_configured_returns_error() {
        let state = test_state(); // ai_client is None
        let err = run_agent(&state, &req("hallo"), None).await.unwrap_err();
        assert!(err.contains("KI-Client nicht konfiguriert"), "got: {err}");
    }
}
