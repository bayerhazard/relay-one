//! "Abo beenden": unsubscribe from a newsletter through its List-Unsubscribe
//! header (RFC 2369) — one click (RFC 8058, a POST to the sender), a mail to
//! the sender's unsubscribe address, or only a web page the user opens.
//!
//! The browser only names the message; the server reads the header itself
//! every time and never takes a URL from the client. The one-click POST goes
//! out from the box, so it is guarded: https only, the host must resolve to
//! a public address, the connection is pinned to that checked address (no
//! DNS rebinding), and no redirects are followed.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use super::{ApiError, ApiResult};
use crate::cache;
use crate::db::with_db;
use crate::AppState;

/// What a message's List-Unsubscribe header offers.
#[derive(Debug, Default, PartialEq)]
pub struct Angebot {
    /// https URL for the one-click POST (only with List-Unsubscribe-Post).
    pub ein_klick: Option<String>,
    /// mailto: target.
    pub mail: Option<Abmeldemail>,
    /// A web page to open (http/https without one-click).
    pub link: Option<String>,
}

#[derive(Debug, PartialEq, Clone)]
pub struct Abmeldemail {
    pub an: String,
    pub betreff: String,
    pub text: String,
}

/// Unfolded value of a header field (case-insensitive), first occurrence.
fn feld(kopf: &str, name: &str) -> Option<String> {
    let mut wert: Option<String> = None;
    for zeile in kopf.split('\n') {
        let zeile = zeile.trim_end_matches('\r');
        if zeile.is_empty() {
            break; // end of the header block
        }
        if let Some(w) = wert.as_mut() {
            if zeile.starts_with(' ') || zeile.starts_with('\t') {
                w.push(' ');
                w.push_str(zeile.trim());
                continue;
            }
            break;
        }
        if let Some((n, v)) = zeile.split_once(':') {
            if n.trim().eq_ignore_ascii_case(name) {
                wert = Some(v.trim().to_string());
            }
        }
    }
    wert
}

fn prozent_decodieren(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    while i < b.len() {
        if b[i] == b'%' && i + 3 <= b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn mailto_lesen(uri: &str) -> Option<Abmeldemail> {
    let rest = uri.get(7..)?; // after "mailto:"
    let (adresse, abfrage) = rest.split_once('?').unwrap_or((rest, ""));
    let an = prozent_decodieren(adresse).trim().to_string();
    // One plain address, nothing that could smuggle a header or a second one.
    if an.is_empty() || !an.contains('@') || an.contains(',') || an.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let mut betreff = "unsubscribe".to_string();
    let mut text = "unsubscribe".to_string();
    for paar in abfrage.split('&') {
        if let Some((k, v)) = paar.split_once('=') {
            let v = prozent_decodieren(&v.replace('+', " "));
            let v: String = v.chars().filter(|c| !c.is_control() || *c == '\n').collect();
            match k.to_ascii_lowercase().as_str() {
                "subject" if !v.trim().is_empty() => betreff = v.replace('\n', " "),
                "body" if !v.trim().is_empty() => text = v,
                _ => {}
            }
        }
    }
    Some(Abmeldemail { an, betreff, text })
}

/// Reads List-Unsubscribe and List-Unsubscribe-Post from a header block.
pub fn angebot_lesen(kopf: &str) -> Angebot {
    let mut a = Angebot::default();
    let Some(liste) = feld(kopf, "List-Unsubscribe") else {
        return a;
    };
    let ein_klick = feld(kopf, "List-Unsubscribe-Post")
        .map(|v| v.replace(' ', "").eq_ignore_ascii_case("List-Unsubscribe=One-Click"))
        .unwrap_or(false);
    // The URIs stand in angle brackets, separated by commas.
    for teil in liste.split('<').skip(1) {
        let Some((uri, _)) = teil.split_once('>') else { continue };
        let uri = uri.trim();
        let klein = uri.to_ascii_lowercase();
        if klein.starts_with("mailto:") {
            if a.mail.is_none() {
                a.mail = mailto_lesen(uri);
            }
        } else if klein.starts_with("https://") {
            if ein_klick && a.ein_klick.is_none() {
                a.ein_klick = Some(uri.to_string());
            } else if a.link.is_none() {
                a.link = Some(uri.to_string());
            }
        } else if klein.starts_with("http://") && a.link.is_none() {
            a.link = Some(uri.to_string());
        }
    }
    a
}

/// Whether an address is one the box must never be sent to on a mail's
/// behalf: loopback, private, link-local, CGNAT, multicast and the like.
pub fn adresse_gesperrt(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10
                || (o[0] == 198 && (o[1] & 0xfe) == 18) // 198.18.0.0/15
                || o[0] >= 240
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return adresse_gesperrt(IpAddr::V4(v4));
            }
            let s = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80 // link-local
                || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
        }
    }
}

async fn ein_klick_senden(url: &str) -> Result<(), String> {
    let ziel = reqwest::Url::parse(url).map_err(|_| "Ungültige Abmeldeadresse".to_string())?;
    if ziel.scheme() != "https" {
        return Err("Abmeldeadresse ist nicht https".into());
    }
    let host = ziel.host_str().ok_or("Abmeldeadresse ohne Host")?.to_string();
    let port = ziel.port_or_known_default().unwrap_or(443);
    let adressen: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| format!("Absender nicht erreichbar ({e})"))?
        .collect();
    let Some(adresse) = adressen.first().copied() else {
        return Err("Absender nicht erreichbar".into());
    };
    if adressen.iter().any(|a| adresse_gesperrt(a.ip())) {
        return Err("Die Abmeldeadresse zeigt in ein internes Netz — Relay ruft sie nicht auf.".into());
    }
    let client = reqwest::Client::builder()
        .resolve(&host, adresse)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let antwort = client
        .post(ziel)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("List-Unsubscribe=One-Click")
        .send()
        .await
        .map_err(|e| format!("Absender nicht erreichbar ({e})"))?;
    if antwort.status().is_client_error() || antwort.status().is_server_error() {
        return Err(format!("Der Absender antwortet mit {}", antwort.status()));
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct NachrichtQuery {
    pub account_id: u32,
    pub uid: u32,
    pub folder: Option<String>,
}

#[derive(Deserialize)]
pub struct AbmeldenRequest {
    pub account_id: u32,
    pub uid: u32,
    pub folder: Option<String>,
}

#[derive(Serialize)]
pub struct AngebotAntwort {
    /// "ein_klick" | "mail" | "link"; absent: no button.
    pub art: Option<&'static str>,
    /// Who is unsubscribed from: host of the URL or the mail address.
    pub ziel: Option<String>,
}

#[derive(Serialize)]
pub struct AbmeldenAntwort {
    pub ok: bool,
    /// "ein_klick" | "mail" | "link" — what actually happened.
    pub art: &'static str,
    /// Where it went: host of the URL or the mail address.
    pub ziel: Option<String>,
    /// For "link": the page the browser opens.
    pub url: Option<String>,
}

fn ist_spam_ordner(ordner: &str) -> bool {
    let o = ordner.to_lowercase();
    ["spam", "junk", "unerwünscht", "unerwunscht"].iter().any(|s| o.contains(s))
}

/// The header block of a message: from the EML archive, else from IMAP.
/// Refuses messages in the spam folder — an unsubscribe click there only
/// tells a spammer that the address is alive.
async fn kopf_holen(state: &AppState, account_id: u32, uid: u32, folder: Option<String>) -> Result<String, ApiError> {
    let gefunden = with_db(state, |conn| {
        cache::messages::fetch_message_body_with_folder(conn, account_id as i64, uid as i64, folder.as_deref())
            .map_err(|e| e.to_string())
    })?;
    let (id, ordner) = match &gefunden {
        Some((m, f)) => (Some(m.id), Some(f.clone())),
        None => (None, folder.clone()),
    };
    if ordner.as_deref().is_some_and(ist_spam_ordner) {
        return Err(ApiError("Aus dem Spam-Ordner meldet Relay nicht ab.".into()));
    }
    if let Some(id) = id {
        let raw_path: Option<String> = with_db(state, |conn| {
            Ok::<_, String>(
                conn.query_row("SELECT raw_path FROM messages WHERE id = ?1", [id], |r| r.get::<_, Option<String>>(0))
                    .ok()
                    .flatten(),
            )
        })
        .ok()
        .flatten();
        if let Some(rp) = raw_path.filter(|p| !p.is_empty()) {
            if let Some(raw) = cache::archive::read_eml(&state.data_root, &rp) {
                return Ok(String::from_utf8_lossy(&raw).into_owned());
            }
        }
    }
    let client = state
        .imap_clients
        .read()
        .get(&account_id)
        .cloned()
        .ok_or(ApiError("IMAP-Client nicht gefunden".into()))?;
    let kopf = client.fetch_header(uid, ordner).await.map_err(|e| ApiError(e.to_string()))?;
    Ok(String::from_utf8_lossy(&kopf).into_owned())
}

fn host_von(url: &str) -> Option<String> {
    reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string))
}

/// `GET /api/v1/messages/unsubscribe?account_id=…&uid=…&folder=…` — whether
/// the mail offers an unsubscribe, and how. Never an error for the reader:
/// no header, spam folder or a fetch problem simply mean "no button".
pub async fn angebot(State(state): State<AppState>, Query(q): Query<NachrichtQuery>) -> ApiResult<AngebotAntwort> {
    let a = match kopf_holen(&state, q.account_id, q.uid, q.folder).await {
        Ok(kopf) => angebot_lesen(&kopf),
        Err(_) => Angebot::default(),
    };
    let antwort = if let Some(u) = &a.ein_klick {
        AngebotAntwort { art: Some("ein_klick"), ziel: host_von(u) }
    } else if let Some(m) = &a.mail {
        AngebotAntwort { art: Some("mail"), ziel: Some(m.an.clone()) }
    } else if let Some(u) = &a.link {
        AngebotAntwort { art: Some("link"), ziel: host_von(u) }
    } else {
        AngebotAntwort { art: None, ziel: None }
    };
    Ok(Json(antwort))
}

/// `POST /api/v1/messages/unsubscribe` — unsubscribe. One click first; if
/// that fails and the sender also names an address, by mail; a bare link
/// goes back to the browser to open.
pub async fn abmelden(State(state): State<AppState>, Json(req): Json<AbmeldenRequest>) -> ApiResult<AbmeldenAntwort> {
    let kopf = kopf_holen(&state, req.account_id, req.uid, req.folder).await?;
    let a = angebot_lesen(&kopf);
    let mut fehler: Option<String> = None;
    if let Some(url) = &a.ein_klick {
        match ein_klick_senden(url).await {
            Ok(()) => {
                tracing::info!("Abo beendet per Ein-Klick bei {}", host_von(url).unwrap_or_default());
                return Ok(Json(AbmeldenAntwort { ok: true, art: "ein_klick", ziel: host_von(url), url: None }));
            }
            Err(e) => {
                tracing::warn!("Ein-Klick-Abmeldung fehlgeschlagen: {e}");
                fehler = Some(e);
            }
        }
    }
    if let Some(m) = &a.mail {
        let smtp = state
            .smtp_clients
            .read()
            .get(&req.account_id)
            .cloned()
            .ok_or(ApiError("SMTP-Client nicht gefunden".into()))?;
        smtp.send(vec![(m.an.as_str(), "")], vec![], vec![], &m.betreff, &m.text, None, None, None, &[])
            .await
            .map_err(|e| ApiError(format!("Abmelde-Mail an {} nicht gesendet: {e}", m.an)))?;
        tracing::info!("Abo beendet per Mail an {}", m.an);
        return Ok(Json(AbmeldenAntwort { ok: true, art: "mail", ziel: Some(m.an.clone()), url: None }));
    }
    if let Some(e) = fehler {
        return Err(ApiError(e));
    }
    if let Some(url) = a.link {
        return Ok(Json(AbmeldenAntwort { ok: true, art: "link", ziel: host_von(&url), url: Some(url) }));
    }
    Err(ApiError("Diese Mail bietet keine Abmeldung an.".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KOPF: &str = "From: News <news@beispiel.de>\r\n\
        List-Unsubscribe: <mailto:abmelden@beispiel.de?subject=abmelden%20bitte>,\r\n \
        <https://beispiel.de/abmelden?id=42>\r\n\
        list-unsubscribe-post: List-Unsubscribe=One-Click\r\n\
        Subject: Hallo\r\n\
        \r\n\
        List-Unsubscribe: <https://body.example/x>\r\n";

    #[test]
    fn reads_one_click_and_mailto_across_folded_lines() {
        let a = angebot_lesen(KOPF);
        assert_eq!(a.ein_klick.as_deref(), Some("https://beispiel.de/abmelden?id=42"));
        let m = a.mail.unwrap();
        assert_eq!(m.an, "abmelden@beispiel.de");
        assert_eq!(m.betreff, "abmelden bitte");
        assert_eq!(a.link, None);
    }

    #[test]
    fn https_without_post_header_is_only_a_link() {
        let a = angebot_lesen("List-Unsubscribe: <https://beispiel.de/weg>\r\n\r\n");
        assert_eq!(a.ein_klick, None);
        assert_eq!(a.link.as_deref(), Some("https://beispiel.de/weg"));
    }

    #[test]
    fn no_header_no_offer_and_body_is_ignored() {
        assert_eq!(angebot_lesen("Subject: x\r\n\r\nList-Unsubscribe: <mailto:a@b.de>\r\n"), Angebot::default());
    }

    #[test]
    fn mailto_cannot_smuggle_a_second_address() {
        assert_eq!(mailto_lesen("mailto:a@b.de,c@d.de"), None);
        assert_eq!(mailto_lesen("mailto:a@b.de%0d%0aBcc:%20x@y.de"), None);
        assert_eq!(mailto_lesen("mailto:"), None);
    }

    #[test]
    fn internal_addresses_are_refused() {
        for ip in ["127.0.0.1", "10.1.2.3", "192.168.1.1", "172.16.0.5", "169.254.1.1", "100.64.0.1", "0.0.0.0", "::1", "fc00::1", "fe80::1", "::ffff:10.0.0.1"] {
            assert!(adresse_gesperrt(ip.parse().unwrap()), "{ip} must be refused");
        }
        for ip in ["93.184.215.14", "2606:4700::1111"] {
            assert!(!adresse_gesperrt(ip.parse().unwrap()), "{ip} is public");
        }
    }

    #[test]
    fn spam_folders_are_recognised() {
        assert!(ist_spam_ordner("Spam"));
        assert!(ist_spam_ordner("INBOX/Junk"));
        assert!(!ist_spam_ordner("INBOX"));
    }
}
