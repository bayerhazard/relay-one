//! Contact CRUD API — CardDAV-backed local contact management.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;

use crate::api::ApiError;
use crate::cache::{self, contacts::ContactRow};
use crate::dav::{CardDavClient, CardDavSettings};
use crate::db::with_db;
use crate::AppState;

use super::ApiResult;

/// Build a CardDAV client from the in-memory settings (set during sync),
/// falling back to the stored settings in the DB.
fn carddav_client(state: &AppState) -> Result<CardDavClient, ApiError> {
    if let Some(settings) = state.carddav_settings.read().clone() {
        return Ok(CardDavClient::new(settings));
    }
    let json = with_db(state, |conn| {
        cache::settings::get_setting(conn, "carddav_settings").map_err(|e| e.to_string())
    })?;
    match json {
        Some(raw) => {
            let mut settings: CardDavSettings =
                serde_json::from_str(&raw).map_err(|e| ApiError(format!("CardDAV parse: {e}")))?;
            settings.password =
                crate::crypto::decrypt(&settings.password).unwrap_or(settings.password);
            Ok(CardDavClient::new(settings))
        }
        None => Err(ApiError(
            "Kein CardDAV-Server konfiguriert — bitte zuerst im Settings-Tab verbinden."
                .to_string(),
        )),
    }
}

/// `GET /api/v1/contacts?search=` — list local contacts.
#[derive(Deserialize)]
pub struct ContactsQuery {
    #[serde(default)]
    pub search: Option<String>,
}

pub async fn list_contacts(
    State(state): State<AppState>,
    Query(q): Query<ContactsQuery>,
) -> ApiResult<Vec<ContactRow>> {
    let search = q.search.unwrap_or_default();
    let rows = with_db(&state, |conn| cache::contacts::list_contacts(conn, &search))?;
    Ok(Json(rows))
}

/// `POST /api/v1/contacts` — create a contact (CardDAV + local cache).
#[derive(Deserialize)]
pub struct CreateContactRequest {
    #[serde(default)]
    pub given_name: String,
    #[serde(default)]
    pub family_name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub organization: String,
}

pub async fn create_contact(
    State(state): State<AppState>,
    Json(req): Json<CreateContactRequest>,
) -> ApiResult<ContactRow> {
    let uid = uuid::Uuid::new_v4().to_string();
    let vcard = crate::dav::vcard::build_vcard(
        &uid,
        &req.given_name,
        &req.family_name,
        &req.display_name,
        &req.email,
        &req.phone,
        &req.organization,
    );

    let client = carddav_client(&state)?;
    client.put_vcard(&vcard, &uid).await.map_err(ApiError)?;

    let contact = crate::dav::vcard::parse_vcard(&vcard);
    with_db(&state, |conn| cache::contacts::upsert_contact(conn, &contact))?;

    let row = with_db(&state, |conn| {
        cache::contacts::get_contact(conn, &uid).map(|o| o.ok_or_else(|| "Kontakt nicht gefunden".to_string()))
    })??;
    Ok(Json(row))
}

/// `PUT /api/v1/contacts/:uid` — update a contact.
pub async fn update_contact(
    State(state): State<AppState>,
    Path(uid): Path<String>,
    Json(req): Json<CreateContactRequest>,
) -> ApiResult<ContactRow> {
    // Ensure the contact exists locally before overwriting.
    with_db(&state, |conn| {
        cache::contacts::get_contact(conn, &uid).map(|o| o.ok_or_else(|| "Kontakt nicht gefunden".to_string()))
    })??;

    let vcard = crate::dav::vcard::build_vcard(
        &uid,
        &req.given_name,
        &req.family_name,
        &req.display_name,
        &req.email,
        &req.phone,
        &req.organization,
    );

    let client = carddav_client(&state)?;
    // Re-PUT to the canonical URL for this UID (idempotent overwrite).
    client.put_vcard(&vcard, &uid).await.map_err(ApiError)?;

    let contact = crate::dav::vcard::parse_vcard(&vcard);
    with_db(&state, |conn| cache::contacts::upsert_contact(conn, &contact))?;

    let row = with_db(&state, |conn| {
        cache::contacts::get_contact(conn, &uid).map(|o| o.ok_or_else(|| "Kontakt nicht gefunden".to_string()))
    })??;
    Ok(Json(row))
}

/// `DELETE /api/v1/contacts/:uid` — delete a contact.
pub async fn delete_contact(
    State(state): State<AppState>,
    Path(uid): Path<String>,
) -> ApiResult<serde_json::Value> {
    let client = carddav_client(&state);
    if let Ok(client) = client {
        // Best-effort remote delete: resolve the contact URL via PROPFIND.
        if let Err(e) = client.delete_vcard_by_uid(&uid).await {
            tracing::warn!("CardDAV remote delete for {uid} failed: {e}");
        }
    }
    with_db(&state, |conn| cache::contacts::delete_contact(conn, &uid))?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}
