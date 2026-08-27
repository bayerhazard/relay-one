//! iMIP invitation endpoints (Phase 2.3).
//!
//! Pending-invitation queue plus accept/decline (sends a `METHOD:REPLY` to the
//! organizer via the account's SMTP and updates the local status).

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::db::with_db;
use crate::AppState;

use super::{ApiError, ApiResult};

/// One row of the pending-invitation queue, joined with event details.
#[derive(Serialize)]
pub struct InvitationView {
    pub event_uid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<i64>,
    pub organizer: String,
    pub attendee_email: String,
    pub status: String,
    pub sequence: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

#[derive(Deserialize)]
pub struct RsvpRequest {
    pub account_id: u32,
}

/// `GET /api/v1/invitations` — the pending-invitation queue (NEEDS-ACTION).
pub async fn list_invitations(State(state): State<AppState>) -> ApiResult<Vec<InvitationView>> {
    let rows = with_db(&state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT i.event_uid, i.organizer, i.attendee_email, i.status, i.sequence,
                        e.id, e.summary, e.start_at, e.end_at, e.location
                 FROM invitations i
                 LEFT JOIN events e ON e.uid = i.event_uid
                 WHERE i.status = 'NEEDS-ACTION'
                 ORDER BY e.start_at",
            )
            .map_err(|e| e.to_string())?;
        let mapped = stmt
            .query_map([], |r| {
                Ok(InvitationView {
                    event_uid: r.get(0)?,
                    organizer: r.get(1)?,
                    attendee_email: r.get(2)?,
                    status: r.get(3)?,
                    sequence: r.get(4)?,
                    event_id: r.get(5)?,
                    summary: r.get(6)?,
                    start_at: r.get(7)?,
                    end_at: r.get(8)?,
                    location: r.get(9)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in mapped {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    })?;
    Ok(Json(rows))
}

fn resolve_event_id(state: &AppState, uid: &str) -> Result<i64, String> {
    with_db(state, |conn| {
        conn.query_row(
            "SELECT id FROM events WHERE uid = ?1",
            rusqlite::params![uid],
            |r| r.get(0),
        )
        .map_err(|e| format!("Event für Einladung nicht gefunden: {e}"))
    })
}

/// Send the RSVP reply and flip the local invitation out of the queue.
async fn do_rsvp(state: &AppState, uid: &str, account_id: u32, decision: &str) -> Result<(), String> {
    let event_id = resolve_event_id(state, uid)?;
    crate::imip::outbound::send_rsvp(state, event_id, account_id, decision).await?;
    with_db(state, |conn| {
        conn.execute(
            "UPDATE invitations SET status = ?2, updated_at = datetime('now')
             WHERE event_uid = ?1 AND status = 'NEEDS-ACTION'",
            rusqlite::params![uid, decision],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })?;
    Ok(())
}

/// `POST /api/v1/invitations/:uid/accept` — accept the invitation.
pub async fn accept_invitation(
    State(state): State<AppState>,
    Path(uid): Path<String>,
    Json(req): Json<RsvpRequest>,
) -> ApiResult<serde_json::Value> {
    do_rsvp(&state, &uid, req.account_id, "ACCEPTED")
        .await
        .map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "ok": true, "status": "ACCEPTED" })))
}

/// `POST /api/v1/invitations/:uid/decline` — decline the invitation.
pub async fn decline_invitation(
    State(state): State<AppState>,
    Path(uid): Path<String>,
    Json(req): Json<RsvpRequest>,
) -> ApiResult<serde_json::Value> {
    do_rsvp(&state, &uid, req.account_id, "DECLINED")
        .await
        .map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "ok": true, "status": "DECLINED" })))
}
