//! "Aufräumen" (Kai, 7.10.2026): move many mails at once, list who writes
//! the most, and the switch that shows the clean-up view.
//!
//! Every action here only moves — to the archive, the trash, the spam folder
//! or another folder. Nothing is deleted for good; that stays in the trash,
//! with a question (CI ABGLEICH RL-R2).

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::Json;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::messages::{move_message, MoveMessageRequest};
use super::{ok, ApiError, ApiResult};
use crate::cache;
use crate::db::with_db;
use crate::AppState;

/// Settings key of the clean-up switch (Einstellungen › Allgemein › Erweitert).
pub const AUFRAEUMEN_KEY: &str = "aufraeumen_enabled";

/// At most this many mails per batch request; the client sends larger
/// selections in parts and shows the progress.
pub const STAPEL_MAX: usize = 500;

#[derive(Deserialize)]
pub struct StapelRequest {
    pub account_id: u32,
    pub uids: Vec<u32>,
    pub source_folder: String,
    pub target_folder: String,
    #[serde(default)]
    pub raw_source_folder: String,
    #[serde(default)]
    pub raw_target_folder: String,
}

#[derive(Serialize)]
pub struct StapelAntwort {
    pub verschoben: usize,
    pub fehler: usize,
}

/// `POST /api/v1/messages/move-batch` — the single move for every uid:
/// local first, the IMAP move queued for the provider worker, local-only
/// target folders through their archive guarantee. One failing mail does not
/// stop the others.
pub async fn stapel_verschieben(
    State(state): State<AppState>,
    Json(req): Json<StapelRequest>,
) -> ApiResult<StapelAntwort> {
    if req.uids.len() > STAPEL_MAX {
        return Err(ApiError(format!("Höchstens {STAPEL_MAX} Mails auf einmal")));
    }
    if req.source_folder == req.target_folder {
        return Ok(Json(StapelAntwort { verschoben: 0, fehler: 0 }));
    }
    let mut verschoben = 0;
    let mut fehler = 0;
    for uid in req.uids {
        let einzeln = MoveMessageRequest {
            account_id: req.account_id,
            uid,
            source_folder: req.source_folder.clone(),
            target_folder: req.target_folder.clone(),
            raw_source_folder: req.raw_source_folder.clone(),
            raw_target_folder: req.raw_target_folder.clone(),
        };
        match move_message(State(state.clone()), Json(einzeln)).await {
            Ok(_) => verschoben += 1,
            Err(e) => {
                tracing::warn!("Stapel: uid {uid} nicht verschoben: {}", e.0);
                fehler += 1;
            }
        }
    }
    Ok(Json(StapelAntwort { verschoben, fehler }))
}

#[derive(Deserialize)]
pub struct AbsenderQuery {
    pub account_id: u32,
    pub folder: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Absender {
    /// Lower-case address — the key the view groups by.
    pub adresse: String,
    /// Display name of the newest mail, else the address.
    pub name: String,
    pub anzahl: usize,
    pub ungelesen: usize,
    pub neueste: Option<String>,
    /// Newest mail first; the view asks it for "Abo beenden".
    pub uids: Vec<i64>,
}

/// "Name <a@b.de>" → ("Name", "a@b.de"); a bare address stands for both.
pub fn adresse_zerlegen(from: &str) -> (String, String) {
    let from = from.trim();
    if let (Some(a), Some(b)) = (from.rfind('<'), from.rfind('>')) {
        if a < b {
            let adresse = from[a + 1..b].trim().to_lowercase();
            let name = from[..a].trim().trim_matches('"').trim().to_string();
            let name = if name.is_empty() { adresse.clone() } else { name };
            return (name, adresse);
        }
    }
    (from.to_string(), from.to_lowercase())
}

/// Groups `(uid, from, date, is_read)` rows, newest first, by address.
pub fn gruppieren(zeilen: Vec<(i64, String, Option<String>, bool)>) -> Vec<Absender> {
    let mut nach: HashMap<String, Absender> = HashMap::new();
    for (uid, from, date, gelesen) in zeilen {
        if from.trim().is_empty() {
            continue;
        }
        let (name, adresse) = adresse_zerlegen(&from);
        let e = nach.entry(adresse.clone()).or_insert_with(|| Absender {
            adresse,
            name,
            anzahl: 0,
            ungelesen: 0,
            neueste: date.clone(),
            uids: Vec::new(),
        });
        e.anzahl += 1;
        if !gelesen {
            e.ungelesen += 1;
        }
        e.uids.push(uid);
    }
    let mut liste: Vec<Absender> = nach.into_values().collect();
    liste.sort_by(|a, b| b.anzahl.cmp(&a.anzahl).then_with(|| b.neueste.cmp(&a.neueste)));
    liste
}

/// `GET /api/v1/messages/senders?account_id=…&folder=…` — who writes the
/// most in a folder: address, name, count, unread, newest date, uids.
pub async fn absender(
    State(state): State<AppState>,
    Query(q): Query<AbsenderQuery>,
) -> ApiResult<Vec<Absender>> {
    let zeilen = with_db(&state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT m.uid, COALESCE(m.from_addr, ''), m.date, m.is_read
                   FROM messages m JOIN folders f ON f.id = m.folder_id
                  WHERE m.account_id = ?1 AND f.name = ?2
                    AND (m.flags NOT LIKE '%\\Deleted%' OR m.flags IS NULL)
                  ORDER BY m.date DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![q.account_id as i64, q.folder], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, bool>(3)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(rows)
    })?;
    Ok(Json(gruppieren(zeilen)))
}

/// `GET /api/v1/settings/aufraeumen`
pub async fn get_schalter(State(state): State<AppState>) -> ApiResult<bool> {
    ok(with_db(&state, |conn| {
        cache::settings::get_setting(conn, AUFRAEUMEN_KEY)
            .map(|v| v.as_deref() == Some("true"))
            .map_err(|e| e.to_string())
    }))
}

/// `POST /api/v1/settings/aufraeumen`
pub async fn set_schalter(State(state): State<AppState>, Json(an): Json<bool>) -> ApiResult<()> {
    ok(with_db(&state, |conn| {
        cache::settings::set_setting(conn, AUFRAEUMEN_KEY, if an { "true" } else { "false" }).map_err(|e| e.to_string())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_name_and_address() {
        assert_eq!(adresse_zerlegen("Jonas Weber <Jonas.Weber@Beispiel.de>"), ("Jonas Weber".into(), "jonas.weber@beispiel.de".into()));
        assert_eq!(adresse_zerlegen("\"Weber, Jonas\" <j@b.de>"), ("Weber, Jonas".into(), "j@b.de".into()));
        assert_eq!(adresse_zerlegen("<j@b.de>"), ("j@b.de".into(), "j@b.de".into()));
        assert_eq!(adresse_zerlegen("j@b.de"), ("j@b.de".into(), "j@b.de".into()));
    }

    #[test]
    fn groups_by_address_most_first() {
        let z = |uid, f: &str, d: &str, r| (uid, f.to_string(), Some(d.to_string()), r);
        let liste = gruppieren(vec![
            z(5, "News <n@x.de>", "2026-10-07", false),
            z(4, "Jonas <j@b.de>", "2026-10-06", true),
            z(3, "News Team <N@X.de>", "2026-10-05", true),
            z(2, "News <n@x.de>", "2026-10-01", false),
            (1, String::new(), None, true),
        ]);
        assert_eq!(liste.len(), 2);
        assert_eq!(liste[0].adresse, "n@x.de");
        assert_eq!(liste[0].name, "News");
        assert_eq!(liste[0].anzahl, 3);
        assert_eq!(liste[0].ungelesen, 2);
        assert_eq!(liste[0].uids, vec![5, 3, 2]);
        assert_eq!(liste[1].anzahl, 1);
    }
}
