//! Calendar, tasks and contacts of a mail account (Kai, 9.10.2026, 26.10.18:
//! "wenn ich meine Gmail oder auch andere Accounts zu Relay hinzufüge, will
//! ich die Möglichkeit haben, gleich den Kalender und die Aufgaben mit
//! optional auszuwählen … und nachträglich möchte ich das auch einstellen
//! können").
//!
//! For providers that give CalDAV and CardDAV with the mail password (an app
//! password) Relay knows the address and takes the account's own login; the
//! password never goes to the page. Gmail needs Google's sign-in (OAuth) for
//! both and has no CalDAV for tasks at all: since Schritt 2 the account signs
//! in with Google (`crate::google`), calendar and contacts run over CalDAV and
//! CardDAV with its token, tasks over Google Tasks.
//!
//! One CalDAV account `mail-<id>` per mail account carries "Kalender" and
//! "Aufgaben"; contacts are the one CardDAV address book, marked with the
//! mail account it came from. Switching off removes Relay's copy, never
//! anything on the server.

use axum::extract::{Path, Query, State};
use axum::response::Html;
use axum::Json;
use serde::{Deserialize, Serialize};

use super::calendars::{upsert_account, CalDavAccountInput};
use super::{ApiError, ApiResult};
use crate::dav::finden::{self, Art};
use crate::dav::{CalDavClient, CalDavSettings, CardDavClient, CardDavSettings};
use crate::db::with_db;
use crate::google::{self, aufgaben};
use crate::{cache, crypto, AppState};

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Anbieter {
    pub name: &'static str,
    pub caldav: Option<&'static str>,
    pub carddav: Option<&'static str>,
    /// Calendar, contacts and tasks only through Google's sign-in.
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

/// Google's sign-in of a Gmail account.
#[derive(Serialize)]
pub struct GoogleStand {
    /// The box owner has entered the Google project's client.
    pub eingerichtet: bool,
    /// The Google account this mail account signed in with.
    pub angemeldet: Option<String>,
}

#[derive(Serialize)]
pub struct DavStand {
    pub anbieter: Option<Anbieter>,
    /// Only for Gmail.
    pub google: Option<GoogleStand>,
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
        google: anb.as_ref().filter(|a| a.nur_google_anmeldung).map(|_| GoogleStand {
            eingerichtet: google::client_id().is_some(),
            angemeldet: google::angemeldet(id),
        }),
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
    /// Google Tasks lists (Gmail).
    pub aufgabenlisten_gefunden: usize,
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
    schalten(&state, id, wahl).await.map(Json)
}

async fn schalten(state: &AppState, id: i64, wahl: DavWahl) -> Result<DavErgebnis, ApiError> {
    let state = state.clone();
    let (rec, pw) = konto_mit_passwort(&state, id)?;
    let anb = anbieter(&rec.sender_email, &rec.imap_host).or_else(|| anbieter(&rec.username, &rec.imap_host));
    let etwas = wahl.kalender || wahl.aufgaben || wahl.kontakte;
    // Gmail signs in with Google unless an address of one's own is given.
    let mit_google = anb.as_ref().is_some_and(|a| a.nur_google_anmeldung)
        && wahl.caldav_url.as_deref().unwrap_or("").trim().is_empty()
        && wahl.carddav_url.as_deref().unwrap_or("").trim().is_empty();
    let google_konto = mit_google.then_some(id);
    let mut google_email = String::new();
    if mit_google && etwas {
        google_email = google::angemeldet(id)
            .ok_or_else(|| ApiError("Bitte melden Sie dieses Konto zuerst mit Google an.".into()))?;
        if google_email.is_empty() {
            google_email = rec.sender_email.clone();
        }
        google::zugang(id).await.map_err(ApiError)?;
    } else if pw.is_empty() && etwas {
        return Err(ApiError("Für dieses Konto ist kein Passwort gespeichert.".into()));
    }
    // With Google the token signs in, not the mail password.
    let (nutzer, passwort) = if mit_google { (String::new(), String::new()) } else { (rec.username.clone(), pw.clone()) };

    // ── Calendar and tasks: one CalDAV account per mail account ──────────
    let cid = caldav_id(id);
    let vorher = state.caldav_accounts.read().iter().find(|a| a.id == cid).cloned();
    let mut kalender_gefunden = 0usize;
    let mut aufgabenlisten_gefunden = 0usize;
    if wahl.kalender || wahl.aufgaben {
        let basis = if mit_google {
            google::endpunkte().caldav(&google_email)
        } else {
            adresse(wahl.caldav_url.clone(), anb.as_ref().and_then(|a| a.caldav), "den Kalender")?
        };
        // Google's tasks need no CalDAV address.
        let heim = if mit_google && !wahl.kalender {
            basis
        } else {
            finden::heimat(&basis, &nutzer, &passwort, Art::Kalender, google_konto)
                .await
                .map_err(|e| ApiError(format!("Kalender: {e}")))?
        };
        let probe = CalDavSettings {
            id: cid.clone(),
            url: heim.clone(),
            username: nutzer.clone(),
            password: passwort.clone(),
            mail_konto: Some(id),
            google: mit_google,
            ..Default::default()
        };
        if wahl.kalender || !mit_google {
            kalender_gefunden = CalDavClient::new(probe)
                .discover_calendars()
                .await
                .map_err(|e| ApiError(format!("Kalender: {e}")))?
                .len();
        }
        if mit_google && wahl.aufgaben {
            aufgabenlisten_gefunden = aufgaben::listen(id).await.map_err(|e| ApiError(format!("Aufgaben: {e}")))?.len();
        }
        // What is switched off leaves Relay's copy; the next sync brings
        // back what stays on.
        if let Some(v) = &vorher {
            let kalender_aus = v.kalender && !wahl.kalender;
            let aufgaben_aus = v.aufgaben && !wahl.aufgaben;
            // Google's task lists are calendar rows of their own; with
            // Google each part takes its rows along.
            let listen = format!("{}/lists/%", google::endpunkte().aufgaben);
            let _ = with_db(&state, |conn| {
                if kalender_aus {
                    conn.execute(
                        "DELETE FROM events WHERE calendar_id IN (SELECT id FROM calendars WHERE caldav_account_id = ?1)",
                        [&cid],
                    )
                    .map_err(|e| e.to_string())?;
                    if v.google {
                        conn.execute(
                            "DELETE FROM calendars WHERE caldav_account_id = ?1 AND url NOT LIKE ?2",
                            rusqlite::params![&cid, &listen],
                        )
                        .map_err(|e| e.to_string())?;
                    }
                }
                if aufgaben_aus {
                    conn.execute(
                        "DELETE FROM todos WHERE calendar_id IN (SELECT id FROM calendars WHERE caldav_account_id = ?1)",
                        [&cid],
                    )
                    .map_err(|e| e.to_string())?;
                    if v.google {
                        conn.execute(
                            "DELETE FROM calendars WHERE caldav_account_id = ?1 AND url LIKE ?2",
                            rusqlite::params![&cid, &listen],
                        )
                        .map_err(|e| e.to_string())?;
                    }
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
                username: nutzer.clone(),
                password: Some(passwort.clone()),
                enabled: Some(true),
                sync_interval_minutes: None,
                kalender: Some(wahl.kalender),
                aufgaben: Some(wahl.aufgaben),
                mail_konto: Some(id),
                google: Some(mit_google),
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
        let basis = if mit_google {
            google::endpunkte().carddav(&google_email)
        } else {
            adresse(wahl.carddav_url.clone(), anb.as_ref().and_then(|a| a.carddav), "die Kontakte")?
        };
        let heim = finden::heimat(&basis, &nutzer, &passwort, Art::Adressbuch, google_konto)
            .await
            .map_err(|e| ApiError(format!("Kontakte: {e}")))?;
        let settings = CardDavSettings {
            url: heim,
            username: nutzer.clone(),
            password: passwort.clone(),
            sync_interval_minutes: 30,
            mail_konto: Some(id),
            google: mit_google,
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

    Ok(DavErgebnis { ok: true, kalender_gefunden, adressbuecher_gefunden, aufgabenlisten_gefunden })
}

// ── Google's sign-in (Schritt 2) ─────────────────────────────────────────

#[derive(Serialize)]
pub struct GoogleApp {
    pub eingerichtet: bool,
    pub client_id: Option<String>,
    /// Path below the page's origin to register in the Google project.
    pub rueckweg: &'static str,
}

/// `GET /api/v1/google/app` — whether the Google project is entered.
pub async fn google_app(State(_): State<AppState>) -> ApiResult<GoogleApp> {
    let client_id = google::client_id();
    Ok(Json(GoogleApp { eingerichtet: client_id.is_some(), client_id, rueckweg: google::RUECKWEG }))
}

#[derive(Deserialize)]
pub struct GoogleAppEingabe {
    pub client_id: String,
    /// Empty keeps the stored secret.
    #[serde(default)]
    pub client_secret: Option<String>,
}

/// `POST /api/v1/google/app` — enter the Google project's client.
pub async fn google_app_setzen(
    State(state): State<AppState>,
    Json(e): Json<GoogleAppEingabe>,
) -> ApiResult<GoogleApp> {
    google::app_setzen(&state, &e.client_id, e.client_secret.as_deref()).map_err(ApiError)?;
    google_app(State(state)).await
}

#[derive(Deserialize)]
pub struct GoogleStart {
    /// The page's origin, e.g. `https://relay.kai.olares.de`.
    pub origin: String,
}

/// Only an https origin, or http on this machine (development).
fn rueckweg_fuer(origin: &str) -> Result<String, ApiError> {
    let o = origin.trim().trim_end_matches('/');
    let url = reqwest::Url::parse(o).map_err(|_| ApiError("Ungültige Adresse der Seite.".into()))?;
    let lokal = matches!(url.host_str(), Some("localhost") | Some("127.0.0.1"));
    if url.scheme() != "https" && !(url.scheme() == "http" && lokal) {
        return Err(ApiError("Google erlaubt die Rückkehr nur über https.".into()));
    }
    if url.path() != "/" && !url.path().is_empty() {
        return Err(ApiError("Ungültige Adresse der Seite.".into()));
    }
    Ok(format!("{o}{}", google::RUECKWEG))
}

/// `POST /api/v1/accounts/:id/google/start` — Google's sign-in page.
pub async fn google_start(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(s): Json<GoogleStart>,
) -> ApiResult<serde_json::Value> {
    let (rec, _) = konto_mit_passwort(&state, id)?;
    let rueckweg = rueckweg_fuer(&s.origin)?;
    let url = google::start(id, &rueckweg, &rec.sender_email).map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "url": url })))
}

#[derive(Deserialize)]
pub struct Rueckkehr {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// The page Google sends the browser back to: hands the result to the
/// window that opened it and closes, or returns to the settings.
fn rueckkehr_seite(konto: Option<i64>, ok: bool, meldung: &str) -> Html<String> {
    let daten = serde_json::json!({ "typ": "relay-google", "ok": ok, "konto": konto, "meldung": meldung })
        .to_string()
        .replace("</", "<\\/");
    let ziel = format!(
        "/settings?google={}{}",
        if ok { "ok" } else { "fehler" },
        konto.map(|k| format!("&konto={k}")).unwrap_or_default()
    );
    let text = meldung.replace('&', "&amp;").replace('<', "&lt;");
    Html(format!(
        "<!doctype html><html lang=\"de\"><meta charset=\"utf-8\"><title>Google · Relay</title>\
         <body style=\"font-family:system-ui,sans-serif;padding:24px\"><p>{text}</p>\
         <script>const d={daten};if(window.opener){{window.opener.postMessage(d,location.origin);window.close();}}\
         else{{location.replace({ziel:?});}}</script></body></html>"
    ))
}

/// `GET /api/v1/google/rueckweg` — Google's answer to the sign-in.
pub async fn google_rueckweg(State(state): State<AppState>, Query(q): Query<Rueckkehr>) -> Html<String> {
    if let Some(e) = q.error {
        let m = if e == "access_denied" {
            "Die Anmeldung bei Google wurde abgebrochen.".to_string()
        } else {
            format!("Google meldet: {e}")
        };
        return rueckkehr_seite(None, false, &m);
    }
    let (Some(code), Some(zustand)) = (q.code, q.state) else {
        return rueckkehr_seite(None, false, "Google hat keine Anmeldung zurückgegeben.");
    };
    match google::rueckweg(&state, &code, &zustand).await {
        Ok(konto) => rueckkehr_seite(Some(konto), true, "Mit Google angemeldet. Sie können dieses Fenster schließen."),
        Err(e) => rueckkehr_seite(None, false, &e),
    }
}

/// Before an account is deleted: calendar, tasks and contacts off, Google's
/// sign-in revoked. Best effort; the deletion goes on either way.
pub(crate) async fn alles_aus(state: &AppState, id: i64) {
    let aus = DavWahl { kalender: false, aufgaben: false, kontakte: false, caldav_url: None, carddav_url: None };
    if let Err(e) = schalten(state, id, aus).await {
        tracing::warn!("Konto {id}: Kalender und Kontakte nicht abgeschaltet: {}", e.0);
    }
    if google::angemeldet(id).is_some() {
        if let Err(e) = google::trennen(state, id).await {
            tracing::warn!("Konto {id}: Google-Anmeldung nicht getrennt: {e}");
        }
    }
}

/// `POST /api/v1/accounts/:id/google/trennen` — switch everything off and
/// forget Google's sign-in.
pub async fn google_trennen(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<serde_json::Value> {
    schalten(
        &state,
        id,
        DavWahl { kalender: false, aufgaben: false, kontakte: false, caldav_url: None, carddav_url: None },
    )
    .await?;
    google::trennen(&state, id).await.map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rueckweg_nur_ueber_https() {
        assert_eq!(
            rueckweg_fuer("https://relay.kai.olares.de").unwrap(),
            "https://relay.kai.olares.de/api/v1/google/rueckweg"
        );
        assert_eq!(rueckweg_fuer("http://127.0.0.1:3800/").unwrap(), "http://127.0.0.1:3800/api/v1/google/rueckweg");
        assert!(rueckweg_fuer("http://relay.kai.olares.de").is_err());
        assert!(rueckweg_fuer("https://relay.kai.olares.de/irgendwo").is_err());
        assert!(rueckweg_fuer("javascript:alert(1)").is_err());
    }

    #[test]
    fn rueckkehr_seite_ohne_ausbruch() {
        let Html(h) = rueckkehr_seite(Some(3), false, "</script><b>x");
        assert!(!h.contains("</script><b>"));
        assert!(h.contains("window.opener.postMessage"));
    }

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
