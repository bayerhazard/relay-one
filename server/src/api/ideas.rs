//! Manual trigger for the Insilo idea processor.
//!
//! `POST /api/v1/ideas/process` scans the idea drop and processes up to `limit`
//! due ideas. With `dry_run: true` it returns the LLM plan per idea without
//! writing anything — a QA aid for tuning the prompt.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::AppState;

use super::ApiResult;

#[derive(Deserialize, Default)]
pub struct ProcessIdeasRequest {
    /// Max ideas to process in this call (default 50, clamped to 1..=200).
    #[serde(default)]
    pub limit: Option<i64>,
    /// When true, return the plans without creating/updating anything.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Serialize)]
pub struct ProcessIdeasResult {
    pub scanned_inserted: usize,
    pub scanned_changed: usize,
    pub processed: usize,
    pub failed: usize,
    pub dry_run: bool,
    /// Per-idea LLM plan (only populated for `dry_run`).
    pub plans: Vec<serde_json::Value>,
}

/// `POST /api/v1/ideas/process`
pub async fn process_ideas(
    State(state): State<AppState>,
    Json(req): Json<ProcessIdeasRequest>,
) -> ApiResult<ProcessIdeasResult> {
    let limit = req.limit.unwrap_or(50).clamp(1, 200);
    let out = crate::sync::insilo_ideas::process_now(&state, limit, req.dry_run).await;
    Ok(Json(out))
}
