//! Tool registry for the agentic assistant (Concept §5.1).
//!
//! One place per action. Every tool is a typed handler whose arguments are
//! described by a hand-written JSON schema (OpenAI `tools` format). Read tools
//! return real data; Write/External tools return a `PreparedCard` (a prepared
//! request + display rows) and never write — execution happens only after the
//! user confirms a plan (server-side, `execute_prepared`).
//!
//! Deviation from Concept §5.1: schemas are hand-written `serde_json::Value`
//! instead of `schemars`-generated (no new dependency; identical effect).

pub mod calendar;
pub mod contacts;
pub mod mail;
pub mod tasks;
pub mod ui;

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::ai::client::ToolSpec;

/// Confirmation tier (Concept §6.2). The tier lives in registry code, never in
/// the model output — the model cannot downgrade it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// … — executed immediately (no card).
    Read,
    /// ✓ — card with [Execute][Discard].
    Write,
    /// !! — card with a second confirmation (invite, rsvp, delete).
    External,
}

/// A prepared HTTP request that a Write/External tool bundles into a card.
/// Execution happens only after the user confirms (server-side, Phase B).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedRequest {
    pub method: String,
    pub path: String,
    pub body: serde_json::Value,
}

/// A renderable display pair for a card row (`<dl>`).
pub type CardRow = (String, String);

/// A card produced by a Write/External tool. Nothing has been written yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedCard {
    pub tool: String,
    pub tier: Tier,
    pub title: String,
    pub rows: Vec<CardRow>,
    pub request: PreparedRequest,
    /// Navigation template after execution; `{id}` is substituted with the new id.
    pub danach: String,
}

/// Outcome of a tool handler (Concept §5.1).
#[derive(Debug)]
pub enum ToolOutcome {
    /// Read: the real result → becomes a `role:"tool"` message.
    Data(serde_json::Value),
    /// Write/External: a card; nothing has been written.
    Card(PreparedCard),
    /// Ambiguous/missing: the model should ask the user (Concept §8.2).
    Nachfrage(String),
    /// Effect-whitelist path (Concept §5.5).
    Nav(String),
}

impl ToolOutcome {
    /// The tool name a card belongs to (for plan steps).
    pub fn is_card(&self) -> bool {
        matches!(self, ToolOutcome::Card(_))
    }
}

/// Context passed to every tool handler. `state` is owned (cloned per call) so
/// handlers can be plain `fn`s returning boxed futures. `known_ids` is shared
/// across a loop for the ID-provenance check (Concept §6.3).
#[derive(Clone)]
pub struct ToolCtx {
    pub state: crate::AppState,
    pub locale: String,
    pub known_ids: Arc<tokio::sync::Mutex<HashMap<String, String>>>,
}

/// A tool handler: pure `fn` returning a boxed `Send` future.
pub type HandlerFn = fn(
    ToolCtx,
    serde_json::Value,
) -> Pin<Box<dyn std::future::Future<Output = Result<ToolOutcome, String>> + Send>>;

/// A registry entry (Concept §5.1 `ToolDef`).
pub struct ToolDef {
    pub name: &'static str,
    pub tier: Tier,
    pub spec: ToolSpec,
    pub handler: HandlerFn,
}

impl ToolDef {
    fn new(
        name: &'static str,
        tier: Tier,
        description: &str,
        parameters: serde_json::Value,
        handler: HandlerFn,
    ) -> Self {
        let spec = ToolSpec {
            call_type: "function".to_string(),
            function: crate::ai::client::ToolSpecFunction {
                name: name.to_string(),
                description: description.to_string(),
                parameters,
            },
        };
        Self { name, tier, spec, handler }
    }
}

/// Build the full registry. Descriptions are locale-aware (Concept §7.4);
/// tool names and schemas stay English (model stability).
pub fn all_tools(locale: &str) -> Vec<ToolDef> {
    let mut tools: Vec<ToolDef> = Vec::new();
    tools.extend(mail::tools(locale));
    tools.extend(calendar::tools(locale));
    tools.extend(contacts::tools(locale));
    tools.extend(tasks::tools(locale));
    tools.extend(ui::tools(locale));
    tools
}

/// Look up a tool by name.
pub fn find_tool(name: &str) -> Option<&'static ToolDef> {
    // Tools are static; build a throwaway registry to resolve (cheap, no I/O).
    static TOOLS: std::sync::OnceLock<Vec<ToolDef>> = std::sync::OnceLock::new();
    let tools = TOOLS.get_or_init(|| all_tools("de"));
    tools.iter().find(|t| t.name == name)
}

/// Execute a tool by name. Unknown tools yield a `Nachfrage` (Concept §5.2.4).
pub async fn execute(
    name: &str,
    args: serde_json::Value,
    ctx: ToolCtx,
) -> Result<ToolOutcome, String> {
    let Some(tool) = find_tool(name) else {
        return Ok(ToolOutcome::Nachfrage(format!(
            "Das Werkzeug '{}' gibt es nicht.",
            name
        )));
    };
    (tool.handler)(ctx, args).await
}

/// A JSON-schema object helper (keeps the hand-written schemas compact).
pub fn obj_schema(props: &[(&str, &str, &str, Option<&str>)], required: &[&str]) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    for (name, typ, desc, enum_vals) in props {
        let mut p = serde_json::Map::new();
        p.insert("type".into(), serde_json::Value::String(typ.to_string()));
        if let Some(ev) = enum_vals {
            p.insert(
                "enum".into(),
                serde_json::Value::Array(ev.split(',').map(|s| serde_json::Value::String(s.trim().to_string())).collect()),
            );
        }
        p.insert("description".into(), serde_json::Value::String(desc.to_string()));
        properties.insert(name.to_string(), serde_json::Value::Object(p));
    }
    let mut schema = serde_json::Map::new();
    schema.insert("type".into(), serde_json::Value::String("object".into()));
    schema.insert("properties".into(), serde_json::Value::Object(properties));
    schema.insert(
        "required".into(),
        serde_json::Value::Array(required.iter().map(|s| serde_json::Value::String(s.to_string())).collect()),
    );
    schema
        .into_iter()
        .collect::<serde_json::Map<_, _>>()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact tool set the agent exposes (Concept §7). Any drift here changes
    /// the wire payload the LLM sees, so it is pinned as a snapshot.
    const EXPECTED_TOOLS: &[&str] = &[
        "calendar_create_event",
        "calendar_delete_event",
        "calendar_find_free_slots",
        "calendar_invite",
        "calendar_list_events",
        "calendar_rsvp",
        "calendar_update_event",
        "contacts_create",
        "contacts_get",
        "contacts_search",
        "mail_flag",
        "mail_get",
        "mail_move",
        "mail_propose_reply",
        "mail_search",
        "tasks_create",
        "tasks_delete",
        "tasks_list",
        "tasks_toggle",
        "ui_navigate",
        "ui_open_item",
        "ui_set_view",
    ];

    /// Payload snapshot (Concept §12.5): the tool set is stable, every spec is a
    /// `function` with an object schema, and the parameter schemas are
    /// locale-independent (only descriptions differ) — the payload the LLM
    /// receives does not depend on the UI language.
    #[test]
    fn tool_registry_snapshot_names_and_schemas() {
        let de = all_tools("de");
        let en = all_tools("en");
        let mut names: Vec<&str> = de.iter().map(|t| t.name).collect();
        names.sort_unstable();
        assert_eq!(names, EXPECTED_TOOLS, "tool set drifted from the snapshot");

        for t in &de {
            assert_eq!(t.spec.call_type, "function");
            assert_eq!(t.spec.function.name, t.name);
            let params = &t.spec.function.parameters;
            assert_eq!(params.get("type").and_then(|v| v.as_str()), Some("object"));
            assert!(params.get("properties").is_some(), "{} has no properties", t.name);
        }

        let de_schemas: Vec<serde_json::Value> =
            de.iter().map(|t| t.spec.function.parameters.clone()).collect();
        let en_schemas: Vec<serde_json::Value> =
            en.iter().map(|t| t.spec.function.parameters.clone()).collect();
        assert_eq!(de_schemas, en_schemas, "parameter schemas must not depend on locale");

        let again: Vec<serde_json::Value> = all_tools("de")
            .iter()
            .map(|t| t.spec.function.parameters.clone())
            .collect();
        assert_eq!(de_schemas, again, "registry must build deterministically");
    }

    /// The system prompt carries the hard contract in both locales and injects
    /// the reference date (Concept §5.2, §7.4).
    #[test]
    fn agent_system_prompt_contract_de_en() {
        let de = crate::ai::prompts::build_agent_prompt("de", "2026-09-07");
        let en = crate::ai::prompts::build_agent_prompt("en", "2026-09-07");
        assert!(de.contains("2026-09-07"), "de prompt must inject the date");
        assert!(en.contains("2026-09-07"), "en prompt must inject the date");
        assert!(de.contains("Schreibe NIE selbst"));
        assert!(en.contains("Never write by yourself"));
        assert!(de.contains("KEIN Tool zum Mail-Versand"));
        assert!(en.contains("NO tool to send email"));
        assert!(de.contains("Antworte auf Deutsch"));
        assert!(en.contains("answer in English"));
        assert!(de.contains("MARK-Blöcken"));
        assert!(en.contains("MARK blocks"));
    }
}
