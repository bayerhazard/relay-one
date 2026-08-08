//! Web Push API: VAPID public key, subscribe/unsubscribe.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::api::ApiResult;

#[derive(Debug, Deserialize)]
pub struct SubscribeRequest {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    #[serde(default)]
    pub account_id: i64,
}

#[derive(Debug, Serialize)]
pub struct VapidInfo {
    pub public_key: String,
    pub subject: String,
}

/// GET /api/v1/push/vapid — public key for PushManager.subscribe().
pub async fn vapid_key(State(state): State<AppState>) -> ApiResult<VapidInfo> {
    let public_key = crate::db::with_db(&state, |conn| {
        crate::push::ensure_vapid_keys(conn, "mailto:relay@aimighty.olares.de")
    })?;
    Ok(Json(VapidInfo {
        public_key,
        subject: "mailto:relay@aimighty.olares.de".into(),
    }))
}

/// POST /api/v1/push/subscribe — store a subscription for push delivery.
pub async fn subscribe(
    State(state): State<AppState>,
    Json(req): Json<SubscribeRequest>,
) -> ApiResult<serde_json::Value> {
    let id = crate::db::with_db(&state, |conn| {
        crate::push::upsert_subscription(conn, req.account_id, &req.endpoint, &req.p256dh, &req.auth)
    })?;
    tracing::info!("WebPush: Subscription {} registriert (account {})", id, req.account_id);
    Ok(Json(serde_json::json!({ "ok": true, "id": id })))
}

/// POST /api/v1/push/unsubscribe — remove a subscription.
pub async fn unsubscribe(
    State(state): State<AppState>,
    Json(req): Json<SubscribeRequest>,
) -> ApiResult<serde_json::Value> {
    crate::db::with_db(&state, |conn| {
        crate::push::remove_subscription(conn, &req.endpoint)
    })?;
    tracing::info!("WebPush: Subscription entfernt: {}", req.endpoint);
    Ok(Json(serde_json::json!({ "ok": true })))
}
