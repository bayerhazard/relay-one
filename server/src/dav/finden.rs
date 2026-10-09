//! Finding a user's calendars and address books from a provider's base
//! address (Kai, 9.10.2026, 26.10.18: "Kalender und Aufgaben gleich mit
//! auswählen"). iCloud, GMX, web.de, mailbox.org and Posteo hand out one
//! address for everybody; the user's own collections hang below a
//! principal. The way there (RFC 6764 / RFC 4791 / RFC 6352):
//!
//! 1. PROPFIND `current-user-principal` on the address, else on
//!    `/.well-known/caldav` or `/.well-known/carddav` of its host;
//! 2. PROPFIND `calendar-home-set` or `addressbook-home-set` on the
//!    principal;
//! 3. the home set is what the clients list with Depth 1.
//!
//! An address that is already a user's own folder (Radicale, Nextcloud with
//! the full path) answers no principal; it is then used as it is.

use quick_xml::events::Event;
use quick_xml::Reader;

use super::client::propfind_method;
use super::reqwest_digest_auth;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Art {
    Kalender,
    Adressbuch,
}

impl Art {
    fn well_known(self) -> &'static str {
        match self {
            Art::Kalender => "/.well-known/caldav",
            Art::Adressbuch => "/.well-known/carddav",
        }
    }
    fn home_set(self) -> &'static str {
        match self {
            Art::Kalender => "calendar-home-set",
            Art::Adressbuch => "addressbook-home-set",
        }
    }
    fn home_body(self) -> &'static str {
        match self {
            Art::Kalender => r#"<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><c:calendar-home-set/></d:prop></d:propfind>"#,
            Art::Adressbuch => r#"<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav"><d:prop><c:addressbook-home-set/></d:prop></d:propfind>"#,
        }
    }
}

const PRINCIPAL_BODY: &str =
    r#"<d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#;

/// The href inside the first element with this local name, or None.
pub fn href_in(xml: &str, element: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    let mut drin = 0usize;
    let mut in_href = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.local_name();
                let name = String::from_utf8_lossy(name.as_ref()).to_string();
                if name == element {
                    drin += 1;
                } else if drin > 0 && name == "href" {
                    in_href = true;
                }
            }
            Ok(Event::Text(t)) if in_href => {
                let text = t.unescape().map(|c| c.trim().to_string()).unwrap_or_default();
                if !text.is_empty() {
                    return Some(text);
                }
            }
            Ok(Event::End(e)) => {
                let name = e.local_name();
                let name = String::from_utf8_lossy(name.as_ref()).to_string();
                if name == "href" {
                    in_href = false;
                } else if name == element && drin > 0 {
                    drin -= 1;
                }
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
    }
}

/// An href against the address it came from: absolute stays, "/x" gets the
/// scheme and host.
pub fn absolut(basis: &str, href: &str) -> Option<String> {
    let basis = reqwest::Url::parse(basis).ok()?;
    basis.join(href).ok().map(|u| u.to_string())
}

/// The user's home set below `url`, or `url` itself when the server names
/// no principal there. An error only when the server refuses the login.
/// `google`: the mail account whose Google token signs in (Gmail).
pub async fn heimat(url: &str, user: &str, pass: &str, art: Art, google: Option<i64>) -> Result<String, String> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    let http = reqwest_digest_auth::ClientBuilder::new(http)
        .username(user.to_string())
        .password(pass.to_string())
        .google(google)
        .build();

    let propfind = |ziel: String, body: &'static str| {
        let req = http
            .request(propfind_method().clone(), &ziel)
            .header("Depth", "0")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(body);
        async move {
            let resp = req.send().await.map_err(|e| format!("{ziel}: {e}"))?;
            let status = resp.status();
            let end = resp.url().to_string();
            if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
                return Err(format!("Anmeldung abgelehnt ({status})"));
            }
            let text = resp.text().await.unwrap_or_default();
            Ok::<_, String>((status, end, text))
        }
    };

    let mut kandidaten = vec![url.to_string()];
    if let Ok(u) = reqwest::Url::parse(url) {
        if let Ok(w) = u.join(art.well_known()) {
            kandidaten.push(w.to_string());
        }
    }
    let mut principal = None;
    for ziel in kandidaten {
        match propfind(ziel, PRINCIPAL_BODY).await {
            Ok((status, end, text)) if status == reqwest::StatusCode::MULTI_STATUS => {
                if let Some(h) = href_in(&text, "current-user-principal").and_then(|h| absolut(&end, &h)) {
                    principal = Some(h);
                    break;
                }
            }
            Err(e) if e.starts_with("Anmeldung abgelehnt") => return Err(e),
            _ => {}
        }
    }
    let Some(principal) = principal else {
        return Ok(url.to_string());
    };
    match propfind(principal.clone(), art.home_body()).await {
        Ok((status, end, text)) if status == reqwest::StatusCode::MULTI_STATUS => Ok(
            href_in(&text, art.home_set())
                .and_then(|h| absolut(&end, &h))
                .unwrap_or_else(|| url.to_string()),
        ),
        Err(e) if e.starts_with("Anmeldung abgelehnt") => Err(e),
        _ => Ok(url.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn href_aus_multistatus() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav">
  <d:response><d:href>/</d:href><d:propstat><d:prop>
    <d:current-user-principal><d:href>/123456/principal/</d:href></d:current-user-principal>
    <cal:calendar-home-set><d:href>https://p01-caldav.icloud.com/123456/calendars/</d:href></cal:calendar-home-set>
  </d:prop></d:propstat></d:response>
</d:multistatus>"#;
        assert_eq!(href_in(xml, "current-user-principal").as_deref(), Some("/123456/principal/"));
        assert_eq!(
            href_in(xml, "calendar-home-set").as_deref(),
            Some("https://p01-caldav.icloud.com/123456/calendars/")
        );
        assert_eq!(href_in(xml, "addressbook-home-set"), None);
        // The response's own href is not the principal's.
        assert_ne!(href_in(xml, "current-user-principal").as_deref(), Some("/"));
    }

    #[test]
    fn href_relativ_und_absolut() {
        assert_eq!(
            absolut("https://caldav.icloud.com/", "/123/principal/").as_deref(),
            Some("https://caldav.icloud.com/123/principal/")
        );
        assert_eq!(
            absolut("https://caldav.icloud.com/", "https://p01-caldav.icloud.com/1/calendars/").as_deref(),
            Some("https://p01-caldav.icloud.com/1/calendars/")
        );
    }
}
