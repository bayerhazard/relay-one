//! Google's sign-in for calendar, contacts and tasks of a Gmail account
//! (Kai, 9.10.2026, Schritt 2: "Gmail über Google-Anmeldung").
//!
//! Gmail's calendar and contacts open only to an OAuth token, not to the app
//! password Relay reads the mail with; tasks have no CalDAV at all and come
//! from the Tasks API. The box owner creates the Google project, enters its
//! client ID and secret once in Relay, and every Gmail account then signs in
//! with "Mit Google anmelden". Relay keeps the refresh token per mail account
//! (encrypted, like every password) and fetches a fresh access token when a
//! DAV or Tasks request needs one.
//!
//! The state lives in this module, not in `AppState`: the DAV client asks for
//! a token at send time and has no handle on the state.

pub mod aufgaben;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use base64::Engine;
use once_cell::sync::Lazy;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use crate::db::with_db;
use crate::{cache, crypto, AppState};

/// Calendar (CalDAV), contacts (CardDAV) and tasks; `openid email` names the
/// Google account that signed in.
pub const BEREICHE: &str = "openid email https://www.googleapis.com/auth/calendar https://www.googleapis.com/auth/carddav https://www.googleapis.com/auth/tasks";

/// Where Google answers. `RELAY_GOOGLE_TEST_BASIS` points every address at
/// one stand-in (web/e2e/testserver/google.py) for the probe.
#[derive(Clone, Debug)]
pub struct Endpunkte {
    pub anmelden: String,
    pub token: String,
    pub widerruf: String,
    pub aufgaben: String,
    caldav: String,
    carddav: String,
}

impl Endpunkte {
    /// The principal of this Google account's calendars.
    pub fn caldav(&self, email: &str) -> String {
        format!("{}{}/user", self.caldav, email)
    }
    /// This Google account's address book.
    pub fn carddav(&self, email: &str) -> String {
        format!("{}{}/lists/default/", self.carddav, email)
    }
}

pub fn endpunkte() -> Endpunkte {
    match std::env::var("RELAY_GOOGLE_TEST_BASIS").ok().filter(|b| !b.is_empty()) {
        Some(b) => {
            let b = b.trim_end_matches('/');
            Endpunkte {
                anmelden: format!("{b}/o/oauth2/v2/auth"),
                token: format!("{b}/token"),
                widerruf: format!("{b}/revoke"),
                aufgaben: format!("{b}/tasks/v1"),
                caldav: format!("{b}/caldav/v2/"),
                carddav: format!("{b}/carddav/v1/principals/"),
            }
        }
        None => Endpunkte {
            anmelden: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token: "https://oauth2.googleapis.com/token".into(),
            widerruf: "https://oauth2.googleapis.com/revoke".into(),
            aufgaben: "https://tasks.googleapis.com/tasks/v1".into(),
            caldav: "https://apidata.googleusercontent.com/caldav/v2/".into(),
            carddav: "https://www.googleapis.com/carddav/v1/principals/".into(),
        },
    }
}

/// The path Google sends the browser back to, below the page's origin.
pub const RUECKWEG: &str = "/api/v1/google/rueckweg";

#[derive(Clone, Serialize, Deserialize)]
struct App {
    client_id: String,
    client_secret: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Anmeldung {
    refresh_token: String,
    #[serde(default)]
    email: String,
}

struct Start {
    konto: i64,
    redirect_uri: String,
    bis: Instant,
}

static APP: Lazy<RwLock<Option<App>>> = Lazy::new(|| RwLock::new(None));
static KONTEN: Lazy<RwLock<HashMap<i64, Anmeldung>>> = Lazy::new(|| RwLock::new(HashMap::new()));
static ZUGAENGE: Lazy<Mutex<HashMap<i64, (String, Instant)>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static WARTEND: Lazy<Mutex<HashMap<String, Start>>> = Lazy::new(|| Mutex::new(HashMap::new()));
/// One refresh at a time per box: two syncs starting together would both
/// ask Google for a token.
static ERNEUERN: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

const SCHLUESSEL_APP: &str = "google_app";

fn schluessel(konto: i64) -> String {
    format!("google_anmeldung:{konto}")
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default()
}

/// Read the client and every signed-in account at start.
pub fn laden(conn: &rusqlite::Connection) {
    if let Ok(Some(raw)) = cache::settings::get_setting(conn, SCHLUESSEL_APP) {
        if let Ok(mut app) = serde_json::from_str::<App>(&raw) {
            app.client_secret = crypto::decrypt(&app.client_secret).unwrap_or(app.client_secret);
            *APP.write() = Some(app);
        }
    }
    let mut stmt = match conn.prepare("SELECT key, value FROM settings WHERE key LIKE 'google_anmeldung:%'") {
        Ok(s) => s,
        Err(_) => return,
    };
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map(|r| r.filter_map(|x| x.ok()).collect())
        .unwrap_or_default();
    let mut konten = KONTEN.write();
    for (key, raw) in rows {
        let Some(konto) = key.strip_prefix("google_anmeldung:").and_then(|k| k.parse::<i64>().ok()) else {
            continue;
        };
        if let Ok(mut a) = serde_json::from_str::<Anmeldung>(&raw) {
            a.refresh_token = crypto::decrypt(&a.refresh_token).unwrap_or(a.refresh_token);
            if !a.refresh_token.is_empty() {
                konten.insert(konto, a);
            }
        }
    }
}

/// The client ID, if the box owner has entered one.
pub fn client_id() -> Option<String> {
    APP.read().as_ref().map(|a| a.client_id.clone()).filter(|c| !c.is_empty())
}

/// Store the Google project's client. The secret stays on the box.
pub fn app_setzen(state: &AppState, client_id: &str, client_secret: Option<&str>) -> Result<(), String> {
    let client_id = client_id.trim().to_string();
    if client_id.is_empty() {
        return Err("Bitte die Client-ID angeben.".into());
    }
    // An empty secret on a later save keeps the stored one.
    let secret = match client_secret.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => s.to_string(),
        None => APP
            .read()
            .as_ref()
            .map(|a| a.client_secret.clone())
            .filter(|s| !s.is_empty())
            .ok_or("Bitte den Clientschlüssel angeben.")?,
    };
    let gespeichert = App { client_id: client_id.clone(), client_secret: crypto::encrypt(&secret)? };
    let raw = serde_json::to_string(&gespeichert).map_err(|e| e.to_string())?;
    with_db(state, |conn| cache::settings::set_setting(conn, SCHLUESSEL_APP, &raw).map_err(|e| e.to_string()))
        ?;
    *APP.write() = Some(App { client_id, client_secret: secret });
    Ok(())
}

/// The Google account a mail account signed in with, if any.
pub fn angemeldet(konto: i64) -> Option<String> {
    KONTEN.read().get(&konto).map(|a| a.email.clone())
}

/// Google's sign-in page for this mail account. `redirect_uri` is the
/// page's origin plus [`RUECKWEG`]; it must be registered in the project.
pub fn start(konto: i64, redirect_uri: &str, login_hint: &str) -> Result<String, String> {
    let client_id = client_id().ok_or("Die Google-Anmeldung ist auf dieser Box noch nicht eingerichtet.")?;
    let mut roh = [0u8; 24];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut roh);
    let zustand = hex::encode(roh);
    {
        let mut w = WARTEND.lock();
        w.retain(|_, s| s.bis > Instant::now());
        w.insert(
            zustand.clone(),
            Start { konto, redirect_uri: redirect_uri.to_string(), bis: Instant::now() + Duration::from_secs(900) },
        );
    }
    let mut url = reqwest::Url::parse(&endpunkte().anmelden).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", BEREICHE)
        .append_pair("access_type", "offline")
        // Without it Google hands out the refresh token only once.
        .append_pair("prompt", "consent")
        .append_pair("include_granted_scopes", "true")
        .append_pair("state", &zustand);
    if !login_hint.is_empty() {
        url.query_pairs_mut().append_pair("login_hint", login_hint);
    }
    Ok(url.to_string())
}

#[derive(Deserialize)]
struct TokenAntwort {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct TokenFehler {
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

/// The email in an ID token. It came straight from Google over TLS, so the
/// payload is read without checking the signature.
pub fn email_aus_id_token(id_token: &str) -> Option<String> {
    let teil = id_token.split('.').nth(1)?;
    let roh = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(teil.trim_end_matches('='))
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&roh).ok()?;
    v.get("email").and_then(|e| e.as_str()).map(String::from)
}

async fn token_holen(form: &[(&str, &str)]) -> Result<TokenAntwort, String> {
    let resp = http()
        .post(endpunkte().token)
        .form(form)
        .send()
        .await
        .map_err(|e| format!("Google nicht erreichbar: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let f: TokenFehler = serde_json::from_str(&text).unwrap_or(TokenFehler { error: String::new(), error_description: String::new() });
        if f.error == "invalid_grant" {
            return Err("Die Google-Anmeldung ist abgelaufen oder wurde widerrufen. Bitte melden Sie sich neu an.".into());
        }
        if f.error == "invalid_client" {
            return Err("Google kennt diese Client-ID oder diesen Clientschlüssel nicht.".into());
        }
        return Err(format!("Google lehnt ab ({status}): {} {}", f.error, f.error_description).trim().to_string());
    }
    serde_json::from_str(&text).map_err(|e| format!("Antwort von Google nicht lesbar: {e}"))
}

/// Google sent the browser back: trade the code for tokens and keep the
/// refresh token for this mail account. Returns the mail account.
pub async fn rueckweg(state: &AppState, code: &str, zustand: &str) -> Result<i64, String> {
    let start = WARTEND
        .lock()
        .remove(zustand)
        .filter(|s| s.bis > Instant::now())
        .ok_or("Die Anmeldung ist abgelaufen. Bitte starten Sie sie neu.")?;
    let app = APP.read().clone().ok_or("Die Google-Anmeldung ist auf dieser Box nicht eingerichtet.")?;
    let t = token_holen(&[
        ("code", code),
        ("client_id", &app.client_id),
        ("client_secret", &app.client_secret),
        ("redirect_uri", &start.redirect_uri),
        ("grant_type", "authorization_code"),
    ])
    .await?;
    let refresh = t
        .refresh_token
        .clone()
        .ok_or("Google hat keinen dauerhaften Zugang erteilt. Bitte entfernen Sie Relay unter myaccount.google.com › Sicherheit › Drittanbieter-Apps und melden Sie sich neu an.")?;
    let email = t.id_token.as_deref().and_then(email_aus_id_token).unwrap_or_default();
    let gespeichert = Anmeldung { refresh_token: crypto::encrypt(&refresh)?, email: email.clone() };
    let raw = serde_json::to_string(&gespeichert).map_err(|e| e.to_string())?;
    with_db(state, |conn| cache::settings::set_setting(conn, &schluessel(start.konto), &raw).map_err(|e| e.to_string()))
        ?;
    KONTEN.write().insert(start.konto, Anmeldung { refresh_token: refresh, email });
    ZUGAENGE
        .lock()
        .insert(start.konto, (t.access_token, Instant::now() + Duration::from_secs(t.expires_in.max(60))));
    Ok(start.konto)
}

/// A valid access token for this mail account, refreshed when it has less
/// than a minute left.
pub async fn zugang(konto: i64) -> Result<String, String> {
    let gueltig = |z: &HashMap<i64, (String, Instant)>| {
        z.get(&konto)
            .filter(|(_, bis)| *bis > Instant::now() + Duration::from_secs(60))
            .map(|(t, _)| t.clone())
    };
    if let Some(t) = gueltig(&ZUGAENGE.lock()) {
        return Ok(t);
    }
    let _eins = ERNEUERN.lock().await;
    if let Some(t) = gueltig(&ZUGAENGE.lock()) {
        return Ok(t);
    }
    let refresh = KONTEN
        .read()
        .get(&konto)
        .map(|a| a.refresh_token.clone())
        .ok_or("Dieses Konto ist nicht bei Google angemeldet.")?;
    let app = APP.read().clone().ok_or("Die Google-Anmeldung ist auf dieser Box nicht eingerichtet.")?;
    let t = token_holen(&[
        ("refresh_token", &refresh),
        ("client_id", &app.client_id),
        ("client_secret", &app.client_secret),
        ("grant_type", "refresh_token"),
    ])
    .await?;
    ZUGAENGE
        .lock()
        .insert(konto, (t.access_token.clone(), Instant::now() + Duration::from_secs(t.expires_in.max(60))));
    Ok(t.access_token)
}

/// Sign the mail account out of Google: Relay forgets the token and asks
/// Google to revoke it. Calendar, contacts and tasks are switched off by
/// the caller.
pub async fn trennen(state: &AppState, konto: i64) -> Result<(), String> {
    let alt = KONTEN.write().remove(&konto);
    ZUGAENGE.lock().remove(&konto);
    with_db(state, |conn| cache::settings::set_setting(conn, &schluessel(konto), "").map_err(|e| e.to_string()))
        ?;
    if let Some(a) = alt {
        // Best effort: Relay forgets the token either way.
        let _ = http().post(endpunkte().widerruf).form(&[("token", a.refresh_token.as_str())]).send().await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_aus_dem_id_token() {
        let nutzlast = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"iss":"https://accounts.google.com","email":"kai@gmail.com","email_verified":true}"#);
        let token = format!("eyJhbGciOiJSUzI1NiJ9.{nutzlast}.unterschrift");
        assert_eq!(email_aus_id_token(&token).as_deref(), Some("kai@gmail.com"));
        assert_eq!(email_aus_id_token("kaputt"), None);
    }

    #[test]
    fn adressen_fuer_ein_konto() {
        std::env::remove_var("RELAY_GOOGLE_TEST_BASIS");
        let e = endpunkte();
        assert_eq!(e.caldav("kai@gmail.com"), "https://apidata.googleusercontent.com/caldav/v2/kai@gmail.com/user");
        assert_eq!(
            e.carddav("kai@gmail.com"),
            "https://www.googleapis.com/carddav/v1/principals/kai@gmail.com/lists/default/"
        );
        assert_eq!(e.aufgaben, "https://tasks.googleapis.com/tasks/v1");
    }

    #[test]
    fn anmeldeseite_traegt_bereiche_und_zustand() {
        *APP.write() = Some(App { client_id: "123.apps.googleusercontent.com".into(), client_secret: "x".into() });
        let url = start(7, "https://relay.kai.olares.de/api/v1/google/rueckweg", "kai@gmail.com").unwrap();
        let u = reqwest::Url::parse(&url).unwrap();
        let q: HashMap<String, String> = u.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], "123.apps.googleusercontent.com");
        assert_eq!(q["access_type"], "offline");
        assert_eq!(q["prompt"], "consent");
        assert_eq!(q["login_hint"], "kai@gmail.com");
        assert!(q["scope"].contains("auth/carddav") && q["scope"].contains("auth/tasks"));
        let z = &q["state"];
        assert_eq!(z.len(), 48);
        assert_eq!(WARTEND.lock().get(z).map(|s| s.konto), Some(7));
    }
}
