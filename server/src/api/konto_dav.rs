//! Calendar, tasks and contacts of a mail account (Kai, 9.10.2026, 26.10.18:
//! "wenn ich meine Gmail oder auch andere Accounts zu Relay hinzufüge, will
//! ich die Möglichkeit haben, gleich den Kalender und die Aufgaben mit
//! optional auszuwählen … und nachträglich möchte ich das auch einstellen
//! können").
//!
//! For providers that give CalDAV and CardDAV with the mail password (an app
//! password) Relay knows the address and takes the account's own login; the
//! password never goes to the page. Gmail needs Google's sign-in (OAuth) for
//! both and has no CalDAV for tasks at all — that comes later.
//!
//! One CalDAV account `mail-<id>` per mail account carries "Kalender" and
//! "Aufgaben"; contacts are the one CardDAV address book, marked with the
//! mail account it came from. Switching off removes Relay's copy, never
//! anything on the server.

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use super::calendars::{upsert_account, CalDavAccountInput};
use super::{ApiError, ApiResult};
use crate::dav::finden::{self, Art};
use crate::dav::{CalDavClient, CalDavSettings, CardDavClient, CardDavSettings};
use crate::db::with_db;
use crate::{cache, crypto, AppState};

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Anbieter {
    pub name: &'static str,
    pub caldav: Option<&'static str>,
    pub carddav: Option<&'static str>,
    /// Calendar and contacts only through Google's sign-in — not yet.
    pub nur_google_anmeldung: bool,
}

/// The provider behind an address or an IMAP host, if Relay knows it.
pub fn anbieter(email: &str, imap_host: &str) -> Option<Anbieter> {
    let domain = email.rsplit('@').next().unwrap_or("").to_lowercase();
    let host = imap_host.to_lowercase();
    let ist = |domains: &[&str], hosts: &[&str]| {
        domains.iter().any(|d| domain == *d) || hosts.iter().any(|h| host == *h || host.ends_with(&format!(".{h}")))
    };
    let a = |name, caldav, carddav| Some(Anbieter { name, caldav: Some(caldav), carddav: Some(carddav), nur_google_anmeldung: false });
    if ist(&["icloud.com", "me.com", "mac.com"], &["mail.me.com"]) {
        return a("iCloud", "https://caldav.icloud.com/", "https://contacts.icloud.com/");
    }
    if ist(&["gmx.de", "gmx.net", "gmx.at", "gmx.ch"], &["gmx.net", "gmx.com"]) {
        return a("GMX", "https://caldav.gmx.net/", "https://carddav.gmx.net/");
    }
    if ist(&["web.de"], &["web.de"]) {
        return a("WEB.DE", "https://caldav.web.de/", "https://carddav.web.de/");
    }
    if ist(&["mailbox.org"], &["mailbox.org"]) {
        return a("mailbox.org", "https://dav.mailbox.org/", "https://dav.mailbox.org/");
    }
    if ist(&["posteo.de", "posteo.net", "posteo.org", "posteo.eu", "posteo.at", "posteo.ch"], &["posteo.de"]) {
        return a("Posteo", "https://posteo.de:8443/", "https://posteo.de:8843/");
    }
    if ist(&["gmail.com", "googlemail.com"], &["gmail.com", "googlemail.com"]) {
        return Some(Anbieter { name: "Google", caldav: None, carddav: None, nur_google_anmeldung: true });
    }
    None
}

fn caldav_id(konto: i64) -> String {
    format!("mail-{konto}")
}

#[derive(Serialize)]
pub struct DavStand {
    pub anbieter: Option<Anbieter>,
    pub caldav_url: String,
    pub carddav_url: String,
    pub kalender: bool,
    pub aufgaben: bool,
    pub kontakte: bool,
}

/// The mail account's record and its IMAP password (decrypted).
fn konto_mit_passwort(state: &AppState, id: i64) -> Result<(cache::accounts::AccountRecord, String), ApiError> {
    let (rec, pw) = with_db(state, |conn| {
        let rec = cache::accounts::get_account(conn, id).map_err(|e| e.to_string())?;
        let pw = cache::accounts::get_account_password(conn, id).map_err(|e| e.to_string())?;
        Ok::<_, String>((rec, pw))
    })?;
    let rec = rec.ok_or_else(|| ApiError("Konto nicht gefunden".into()))?;
    let pw = pw.map(|p| crypto::decrypt(&p).unwrap_or(p)).unwrap_or_default();
    Ok((rec, pw))
}

/// `GET /api/v1/accounts/:id/dav` — what this account brings along.
pub async fn dav_stand(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<DavStand> {
    let (rec, _) = konto_mit_passwort(&state, id)?;
    let anb = anbieter(&rec.sender_email, &rec.imap_host).or_else(|| anbieter(&rec.username, &rec.imap_host));
    let cal: Option<CalDavSettings> = state.caldav_accounts.read().iter().find(|a| a.id == caldav_id(id)).cloned();
    let card: Option<CardDavSettings> = state.carddav_settings.read().clone().filter(|c| c.mail_konto == Some(id));
    Ok(Json(DavStand {
        caldav_url: cal
            .as_ref()
            .map(|c| c.url.clone())
            .or_else(|| anb.as_ref().and_then(|a| a.caldav.map(String::from)))
            .unwrap_or_default(),
        carddav_url: card
            .as_ref()
            .map(|c| c.url.clone())
            .or_else(|| anb.as_ref().and_then(|a| a.carddav.map(String::from)))
            .unwrap_or_default(),
        kalender: cal.as_ref().map(|c| c.enabled && c.kalender).unwrap_or(false),
        aufgaben: cal.as_ref().map(|c| c.enabled && c.aufgaben).unwrap_or(false),
        kontakte: card.is_some(),
        anbieter: anb,
    }))
}

#[derive(Deserialize)]
pub struct DavWahl {
    #[serde(default)]
    pub kalender: bool,
    #[serde(default)]
    pub aufgaben: bool,
    #[serde(default)]
    pub kontakte: bool,
    /// An address of one's own (Nextcloud, another server); else the
    /// provider's.
    #[serde(default)]
    pub caldav_url: Option<String>,
    #[serde(default)]
    pub carddav_url: Option<String>,
}

#[derive(Serialize)]
pub struct DavErgebnis {
    pub ok: bool,
    /// Calendar collections found (tasks live in them too).
    pub kalender_gefunden: usize,
    pub adressbuecher_gefunden: usize,
}

fn adresse(eigen: Option<String>, vorgabe: Option<&'static str>, was: &str) -> Result<String, ApiError> {
    eigen
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .or_else(|| vorgabe.map(String::from))
        .ok_or_else(|| ApiError(format!("Bitte die Adresse für {was} angeben.")))
}

/// `POST /api/v1/accounts/:id/dav` — switch calendar, tasks and contacts.
pub async fn dav_setzen(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(wahl): Json<DavWahl>,
) -> ApiResult<DavErgebnis> {
    let (rec, pw) = konto_mit_passwort(&state, id)?;
    let anb = anbieter(&rec.sender_email, &rec.imap_host).or_else(|| anbieter(&rec.username, &rec.imap_host));
    if anb.as_ref().is_some_and(|a| a.nur_google_anmeldung) && (wahl.kalender || wahl.aufgaben || wahl.kontakte)
        && wahl.caldav_url.as_deref().unwrap_or("").is_empty()
        && wahl.carddav_url.as_deref().unwrap_or("").is_empty()
    {
        return Err(ApiError(
            "Kalender und Kontakte von Google brauchen die Google-Anmeldung, die Relay noch nicht hat.".into(),
        ));
    }
    if pw.is_empty() && (wahl.kalender || wahl.aufgaben || wahl.kontakte) {
        return Err(ApiError("Für dieses Konto ist kein Passwort gespeichert.".into()));
    }

    // ── Calendar and tasks: one CalDAV account per mail account ──────────
    let cid = caldav_id(id);
    let vorher = state.caldav_accounts.read().iter().find(|a| a.id == cid).cloned();
    let mut kalender_gefunden = 0usize;
    if wahl.kalender || wahl.aufgaben {
        let basis = adresse(wahl.caldav_url.clone(), anb.as_ref().and_then(|a| a.caldav), "den Kalender")?;
        let heim = finden::heimat(&basis, &rec.username, &pw, Art::Kalender)
            .await
            .map_err(|e| ApiError(format!("Kalender: {e}")))?;
        let probe = CalDavSettings {
            id: cid.clone(),
            url: heim.clone(),
            username: rec.username.clone(),
            password: pw.clone(),
            ..Default::default()
        };
        kalender_gefunden = CalDavClient::new(probe)
            .discover_calendars()
            .await
            .map_err(|e| ApiError(format!("Kalender: {e}")))?
            .len();
        // What is switched off leaves Relay's copy; the next sync brings
        // back what stays on.
        if let Some(v) = &vorher {
            let kalender_aus = v.kalender && !wahl.kalender;
            let aufgaben_aus = v.aufgaben && !wahl.aufgaben;
            let _ = with_db(&state, |conn| {
                if kalender_aus {
                    conn.execute(
                        "DELETE FROM events WHERE calendar_id IN (SELECT id FROM calendars WHERE caldav_account_id = ?1)",
                        [&cid],
                    )
                    .map_err(|e| e.to_string())?;
                }
                if aufgaben_aus {
                    conn.execute(
                        "DELETE FROM todos WHERE calendar_id IN (SELECT id FROM calendars WHERE caldav_account_id = ?1)",
                        [&cid],
                    )
                    .map_err(|e| e.to_string())?;
                }
                Ok::<_, String>(())
            });
        }
        let _ = upsert_account(
            &state,
            CalDavAccountInput {
                id: Some(cid.clone()),
                name: Some(rec.name.clone()),
                url: heim,
                username: rec.username.clone(),
                password: Some(pw.clone()),
                enabled: Some(true),
                sync_interval_minutes: None,
                kalender: Some(wahl.kalender),
                aufgaben: Some(wahl.aufgaben),
                mail_konto: Some(id),
            },
        )
        .await?;
        let s = state.clone();
        let konto = state.caldav_accounts.read().iter().find(|a| a.id == cid).cloned();
        if let Some(konto) = konto {
            tokio::spawn(async move {
                if let Err(e) = super::calendars::do_caldav_sync_account(&s, &konto).await {
                    tracing::warn!("CalDAV-Sync '{}' nach dem Verbinden: {}", konto.name, e.0);
                }
            });
        }
    } else if vorher.is_some() {
        let rest: Vec<CalDavSettings> = state.caldav_accounts.read().iter().filter(|a| a.id != cid).cloned().collect();
        with_db(&state, |conn| cache::cal::delete_calendars_for_account(conn, &cid).map_err(|e| e.to_string()))?;
        super::calendars::store_accounts_pub(&state, rest)?;
    }

    // ── Contacts: the one CardDAV address book ───────────────────────────
    let mut adressbuecher_gefunden = 0usize;
    let card_vorher = state.carddav_settings.read().clone();
    if wahl.kontakte {
        let basis = adresse(wahl.carddav_url.clone(), anb.as_ref().and_then(|a| a.carddav), "die Kontakte")?;
        let heim = finden::heimat(&basis, &rec.username, &pw, Art::Adressbuch)
            .await
            .map_err(|e| ApiError(format!("Kontakte: {e}")))?;
        let settings = CardDavSettings {
            url: heim,
            username: rec.username.clone(),
            password: pw.clone(),
            sync_interval_minutes: 30,
            mail_konto: Some(id),
        };
        adressbuecher_gefunden = CardDavClient::new(settings.clone())
            .adressbuecher()
            .await
            .map_err(|e| ApiError(format!("Kontakte: {e}")))?;
        let mut gespeichert = settings.clone();
        gespeichert.password = crypto::encrypt(&settings.password).unwrap_or_else(|_| settings.password.clone());
        let raw = serde_json::to_string(&gespeichert).map_err(|e| ApiError(e.to_string()))?;
        let neu = card_vorher.as_ref().map(|c| c.url != settings.url).unwrap_or(true);
        with_db(&state, |conn| {
            cache::settings::set_setting(conn, "carddav_settings", &raw).map_err(|e| e.to_string())?;
            if neu {
                cache::settings::set_setting(conn, "carddav_sync_token", "").map_err(|e| e.to_string())?;
            }
            Ok::<_, String>(())
        })?;
        if neu {
            *state.carddav_sync_token.write() = String::new();
        }
        *state.carddav_settings.write() = Some(settings);
        let s = state.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::dav::scheduler::do_sync(&s).await {
                tracing::warn!("CardDAV-Sync nach dem Verbinden: {e}");
            }
        });
    } else if card_vorher.as_ref().is_some_and(|c| c.mail_konto == Some(id)) {
        with_db(&state, |conn| {
            cache::settings::set_setting(conn, "carddav_settings", "").map_err(|e| e.to_string())?;
            cache::settings::set_setting(conn, "carddav_sync_token", "").map_err(|e| e.to_string())?;
            conn.execute("DELETE FROM contacts WHERE source != 'mail'", []).map_err(|e| e.to_string())?;
            Ok::<_, String>(())
        })?;
        *state.carddav_settings.write() = None;
        *state.carddav_sync_token.write() = String::new();
    }

    Ok(Json(DavErgebnis { ok: true, kalender_gefunden, adressbuecher_gefunden }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anbieter_nach_adresse_und_host() {
        assert_eq!(anbieter("max@gmx.de", "").unwrap().name, "GMX");
        assert_eq!(anbieter("", "imap.gmx.net").unwrap().name, "GMX");
        assert_eq!(anbieter("a@icloud.com", "").unwrap().caldav, Some("https://caldav.icloud.com/"));
        assert_eq!(anbieter("x@example.org", "imap.mail.me.com").unwrap().name, "iCloud");
        assert_eq!(anbieter("p@posteo.de", "").unwrap().carddav, Some("https://posteo.de:8843/"));
        assert_eq!(anbieter("w@web.de", "").unwrap().name, "WEB.DE");
        assert_eq!(anbieter("m@mailbox.org", "").unwrap().name, "mailbox.org");
        let g = anbieter("k@gmail.com", "imap.gmail.com").unwrap();
        assert!(g.nur_google_anmeldung && g.caldav.is_none());
        assert!(anbieter("x@firma.de", "mail.firma.de").is_none());
        // Not fooled by a host that only contains the name.
        assert!(anbieter("x@firma.de", "gmx.net.firma.de").is_none());
    }
}
