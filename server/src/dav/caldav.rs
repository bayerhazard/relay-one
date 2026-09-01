//! CalDAV (RFC 4791) client for synchronising calendars and events.
//!
//! Mirrors the structure of [`super::carddav`]: a thin, async client over the
//! shared WebDAV primitives in [`super::client`], returning the serialisable
//! [`super::ics::IcsEvent`] model. Works against any CalDAV server
//! (Radicale, Nextcloud, Baïkal, …).

use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::client::{
    parse_hrefs, parse_multistatus, parse_sync_response, parse_sync_token, propfind_method,
    resolve_href, xml_escape,
};
use super::ics::{self, IcsEvent, IcsTodo};
use super::reqwest_digest_auth;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalDavSettings {
    pub url: String,
    pub username: String,
    pub password: String,
    pub sync_interval_minutes: u64,
}

impl Default for CalDavSettings {
    fn default() -> Self {
        Self {
            url: String::new(),
            username: String::new(),
            password: String::new(),
            sync_interval_minutes: 30,
        }
    }
}

/// A CalDAV calendar collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Calendar {
    pub href: String,
    pub display_name: Option<String>,
    /// Absolute URL of the calendar collection.
    pub url: String,
}

pub struct CalDavClient {
    settings: CalDavSettings,
    http: reqwest_digest_auth::Client,
}

impl CalDavClient {
    pub fn new(settings: CalDavSettings) -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();

        let http = reqwest_digest_auth::ClientBuilder::new(http)
            .username(settings.username.clone())
            .password(settings.password.clone())
            .build();

        Self { settings, http }
    }

    /// Discover calendar collections by PROPFIND on the configured (principal)
    /// URL. Keeps resources whose resourcetype contains `<calendar/>`.
    pub async fn discover_calendars(&self) -> Result<Vec<Calendar>, String> {
        if self.settings.url.is_empty() {
            return Err("CalDAV-Server-URL nicht konfiguriert".into());
        }
        let base_url = self.settings.url.trim_end_matches('/').to_string();

        let body = r#"<d:propfind xmlns:d="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
            <d:prop>
                <d:resourcetype/>
                <d:displayname/>
                <d:href/>
            </d:prop>
        </d:propfind>"#;

        let resp = self
            .http
            .request(propfind_method().clone(), &base_url)
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .header("User-Agent", "Relay-CalDAV/1.0")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("CalDAV-Discovery-PROPFIND fehlgeschlagen: {e}"))?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;

        if status != reqwest::StatusCode::MULTI_STATUS {
            return Err(format!(
                "CalDAV-Discovery: unerwarteter Status {} (erwartet 207)",
                status
            ));
        }

        let calendars = parse_multistatus(&text)?
            .into_iter()
            .filter(|r| r.is_calendar && !r.href.is_empty())
            .filter_map(|r| {
                let url = resolve_href(&base_url, &r.href)?;
                Some(Calendar {
                    href: r.href,
                    display_name: r.display_name,
                    url,
                })
            })
            .collect();

        Ok(calendars)
    }

    /// List event object URLs (members) of a calendar collection.
    async fn list_events(&self, calendar_url: &str) -> Result<Vec<String>, String> {
        let body = r#"<d:propfind xmlns:d="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
            <d:prop>
                <d:href/>
                <d:getetag/>
            </d:prop>
        </d:propfind>"#;

        let resp = self
            .http
            .request(propfind_method().clone(), calendar_url)
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .header("User-Agent", "Relay-CalDAV/1.0")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("CalDAV-PROPFIND fehlgeschlagen: {e}"))?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;

        if status != reqwest::StatusCode::MULTI_STATUS {
            return Err(format!("CalDAV-PROPFIND: unerwarteter Status {status}"));
        }

        let urls: Vec<String> = parse_hrefs(&text)?
            .into_iter()
            .filter_map(|u| resolve_href(calendar_url, &u))
            .collect();

        Ok(urls)
    }

    /// Fetch the raw ICS body of a single event object.
    async fn fetch_event(&self, url: &str) -> Result<String, String> {
        let resp = self
            .http
            .get(url)
            .header("User-Agent", "Relay-CalDAV/1.0")
            .send()
            .await
            .map_err(|e| format!("CalDAV-GET fehlgeschlagen: {e}"))?;

        let text = resp.text().await.map_err(|e| e.to_string())?;
        Ok(text)
    }

    /// Fetch all events from all discovered calendars.
    /// Returns (events, sync_token) for incremental sync.
    pub async fn fetch_all_events(&self) -> Result<(Vec<IcsEvent>, String), String> {
        let calendars = self.discover_calendars().await?;
        if calendars.is_empty() {
            return Err(
                "CalDAV-Fehler: Keine Kalender auf dem Server gefunden.\n\
                 Erwartet wird eine Principal-URL, z.B. https://cal.domain.de/Benutzername/"
                    .into(),
            );
        }

        let mut events = Vec::new();
        let mut last_token = String::new();
        for cal in &calendars {
            let urls = self.list_events(&cal.url).await?;
            for url in &urls {
                match self.fetch_event(url).await {
                    Ok(ics) => match ics::parse_event(&ics) {
                        Ok(mut ev) => {
                            ev.url = url.clone();
                            events.push(ev);
                        }
                        Err(e) => tracing::warn!("CalDAV: VEVENT {url} nicht parsbar: {e}"),
                    },
                    Err(e) => tracing::warn!("CalDAV: Event {url} nicht ladbar: {e}"),
                }
            }
            // Remember the sync token of the last calendar (single-token model).
            if let Ok(tok) = self.get_sync_token(&cal.url).await {
                last_token = tok;
            }
        }

        tracing::info!(
            "CalDAV: {} Events aus {} Kalendern synchronisiert",
            events.len(),
            calendars.len()
        );
        Ok((events, last_token))
    }

    /// Fetch all VTODO objects across all discovered calendars.
    pub async fn fetch_all_todos(&self) -> Result<Vec<IcsTodo>, String> {
        let calendars = self.discover_calendars().await?;
        let mut todos = Vec::new();
        for cal in &calendars {
            let urls = self.list_events(&cal.url).await?;
            for url in &urls {
                match self.fetch_event(url).await {
                    Ok(ics) => match ics::parse_todos(&ics) {
                        Ok(items) => {
                            for mut t in items {
                                t.url = url.clone();
                                todos.push(t);
                            }
                        }
                        Err(_) => { /* not a VTODO object — skip */ }
                    },
                    Err(e) => tracing::warn!("CalDAV: Todo-Objekt {url} nicht ladbar: {e}"),
                }
            }
        }
        tracing::info!("CalDAV: {} Todos aus {} Kalendern synchronisiert", todos.len(), calendars.len());
        Ok(todos)
    }

    /// Incremental sync of the first discovered calendar using SYNC-COLLECTION.
    /// Returns (added/changed events, deleted UIDs, new sync token).
    pub async fn sync_incremental(
        &self,
        sync_token: &str,
    ) -> Result<(Vec<IcsEvent>, Vec<String>, String), String> {
        let calendars = self.discover_calendars().await?;
        let cal = calendars
            .first()
            .ok_or("CalDAV: kein Kalender für Sync gefunden")?;

        let body = format!(
            r#"<d:propfind xmlns:d="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
                <d:sync-token>{}</d:sync-token>
                <d:prop>
                    <d:sync-token/>
                </d:prop>
            </d:propfind>"#,
            xml_escape(sync_token)
        );

        let resp = self
            .http
            .request(propfind_method().clone(), &cal.url)
            .header("Depth", "1")
            .header("Sync-Level", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .header("User-Agent", "Relay-CalDAV/1.0")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("CalDAV-SYNC-COLLECTION fehlgeschlagen: {e}"))?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;

        let new_sync_token = parse_sync_token(&text).unwrap_or_default();

        if status == reqwest::StatusCode::OK {
            return Ok((Vec::new(), Vec::new(), new_sync_token));
        }

        if status == reqwest::StatusCode::MULTI_STATUS {
            let (added_urls, deleted_uids) = parse_sync_response(&text)?;
            let mut added = Vec::new();
            for url in &added_urls {
                let abs = match resolve_href(&cal.url, url) {
                    Some(a) => a,
                    None => {
                        tracing::warn!("CalDAV: href '{}' verworfen (fremder Origin)", url);
                        continue;
                    }
                };
                match self.fetch_event(&abs).await {
                    Ok(ics) => match ics::parse_event(&ics) {
                        Ok(mut ev) => {
                            ev.url = abs.clone();
                            added.push(ev);
                        }
                        Err(e) => tracing::warn!("CalDAV: VEVENT {abs} nicht parsbar: {e}"),
                    },
                    Err(e) => tracing::warn!("CalDAV: Event {abs} nicht ladbar: {e}"),
                }
            }
            Ok((added, deleted_uids, new_sync_token))
        } else {
            Err(format!("CalDAV-SYNC-COLLECTION: unerwarteter Status {status}"))
        }
    }

    /// Get the current sync token of a calendar (Depth 0 PROPFIND).
    async fn get_sync_token(&self, calendar_url: &str) -> Result<String, String> {
        let body = r#"<d:propfind xmlns:d="DAV:">
            <d:prop>
                <d:sync-token/>
            </d:prop>
        </d:propfind>"#;

        let resp = self
            .http
            .request(propfind_method().clone(), calendar_url)
            .header("Depth", "0")
            .header("Content-Type", "application/xml; charset=utf-8")
            .header("User-Agent", "Relay-CalDAV/1.0")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("CalDAV-Sync-Token fehlgeschlagen: {e}"))?;

        let text = resp.text().await.map_err(|e| e.to_string())?;
        parse_sync_token(&text).ok_or("Kein CalDAV-Sync-Token gefunden".into())
    }

    /// Create a new event in a calendar via PUT. Returns the new object URL.
    pub async fn create_event(&self, calendar_url: &str, ics: &str) -> Result<String, String> {
        let uid = ics::parse_event(ics)
            .map(|e| e.uid)
            .unwrap_or_else(|_| format!("relay-{}", uuid_like()));
        let url = format!("{}/{}.ics", calendar_url.trim_end_matches('/'), uid);
        self.put(&url, ics).await?;
        Ok(url)
    }

    /// Update an existing event via PUT.
    pub async fn update_event(&self, url: &str, ics: &str) -> Result<(), String> {
        self.put(url, ics).await
    }

    /// Delete an event via DELETE.
    pub async fn delete_event(&self, url: &str) -> Result<(), String> {
        let resp = self
            .http
            .request(reqwest::Method::DELETE, url)
            .header("User-Agent", "Relay-CalDAV/1.0")
            .send()
            .await
            .map_err(|e| format!("CalDAV-DELETE fehlgeschlagen: {e}"))?;
        let status = resp.status();
        if !(200..300).contains(&status.as_u16()) {
            return Err(format!("CalDAV-DELETE: unerwarteter Status {status}"));
        }
        Ok(())
    }

    async fn put(&self, url: &str, ics: &str) -> Result<(), String> {
        let resp = self
            .http
            .request(reqwest::Method::PUT, url)
            .header("Content-Type", "text/calendar; charset=utf-8")
            .header("User-Agent", "Relay-CalDAV/1.0")
            .body(ics.to_string())
            .send()
            .await
            .map_err(|e| format!("CalDAV-PUT fehlgeschlagen: {e}"))?;
        let status = resp.status();
        if !(200..300).contains(&status.as_u16()) {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("CalDAV-PUT: Status {status}: {body}"));
        }
        Ok(())
    }
}

/// A short, collision-resistant pseudo-UID when the ICS carries none.
fn uuid_like() -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::process::id().hash(&mut h);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        .hash(&mut h);
    format!("{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_calendars_parses_multistatus() {
        // Validate the generic parser flags <calendar/> resourcetypes.
        let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
          <d:response>
            <d:href>/Marc/Arbeit/</d:href>
            <d:propstat>
              <d:prop>
                <d:resourcetype><d:collection/><C:calendar/></d:resourcetype>
                <d:displayname>Arbeit</d:displayname>
              </d:prop>
              <d:status>HTTP/1.1 200 OK</d:status>
            </d:propstat>
          </d:response>
          <d:response>
            <d:href>/Marc/</d:href>
            <d:propstat>
              <d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop>
              <d:status>HTTP/1.1 200 OK</d:status>
            </d:propstat>
          </d:response>
        </d:multistatus>"#;
        let responses = parse_multistatus(xml).unwrap();
        let cals: Vec<_> = responses.iter().filter(|r| r.is_calendar).collect();
        assert_eq!(cals.len(), 1);
        assert_eq!(cals[0].href, "/Marc/Arbeit/");
        assert_eq!(cals[0].display_name.as_deref(), Some("Arbeit"));
    }

    /// Integration test against Radicale on Olares (ignored by default).
    ///
    /// ```bash
    /// cargo test test_caldav -- --ignored
    /// ```
    #[ignore]
    #[tokio::test]
    async fn test_caldav_discover() {
        let url = std::env::var("CALDAV_URL")
            .unwrap_or_else(|_| "https://cal.aimighty.olares.de/Marc/".into());
        let user = std::env::var("CALDAV_USER").unwrap_or_else(|_| "Marc".into());
        let pass = std::env::var("CALDAV_PASS").unwrap_or_default();
        let client = CalDavClient::new(CalDavSettings {
            url,
            username: user,
            password: pass,
            sync_interval_minutes: 30,
        });
        let calendars = client.discover_calendars().await.unwrap();
        println!("Calendars: {calendars:?}");
        assert!(!calendars.is_empty());
    }
}
