use reqwest::Client;
use serde::{Deserialize, Serialize};
use quick_xml::events::Event;
use quick_xml::reader::Reader;

use super::reqwest_digest_auth;
use super::vcard::{self, Contact};

/// PROPFIND HTTP method — used by every CardDAV operation.
static PROPFIND_METHOD: std::sync::OnceLock<reqwest::Method> = std::sync::OnceLock::new();

fn propfind_method() -> &'static reqwest::Method {
    PROPFIND_METHOD.get_or_init(|| reqwest::Method::from_bytes(b"PROPFIND").unwrap())
}

/// One `<response>` element from a WebDAV multistatus body, parsed
/// namespace-agnostically (prefixes like `d:`/`D:` are ignored).
#[derive(Debug, Default, Clone)]
struct DavResponse {
    href: String,
    /// HTTP status code from `<status>` (e.g. 404 for a deleted resource).
    status: Option<u16>,
    /// Display name from `<displayname>`, if present.
    display_name: Option<String>,
    /// Whether the resourcetype contained `<addressbook/>`.
    is_addressbook: bool,
}

/// Returns the local name of an element, stripping any namespace prefix.
fn local_name(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    match s.rsplit_once(':') {
        Some((_, local)) => local.to_ascii_lowercase(),
        None => s.to_ascii_lowercase(),
    }
}

/// Parse a WebDAV/CardDAV multistatus XML body into structured responses
/// using a real XML parser (quick-xml). Robust against attributes, self-closing
/// tags, arbitrary namespace prefixes and reordered children — unlike the
/// previous substring scanning, which could mis-associate hrefs and statuses.
fn parse_multistatus(xml: &str) -> Result<Vec<DavResponse>, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut responses: Vec<DavResponse> = Vec::new();
    let mut current: Option<DavResponse> = None;
    // Tracks the local-name stack so text is attributed to the right element.
    let mut stack: Vec<String> = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref());
                if name == "response" {
                    current = Some(DavResponse::default());
                }
                stack.push(name);
            }
            Ok(Event::Empty(e)) => {
                // Self-closing element, e.g. <addressbook/> inside resourcetype.
                let name = local_name(e.name().as_ref());
                if name == "addressbook" {
                    if let Some(r) = current.as_mut() {
                        if stack.iter().any(|s| s == "resourcetype") {
                            r.is_addressbook = true;
                        } else {
                            // Some servers nest resourcetype loosely; accept it.
                            r.is_addressbook = true;
                        }
                    }
                }
            }
            Ok(Event::Text(e)) => {
                let text = e.unescape().map(|c| c.into_owned()).unwrap_or_default();
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    buf.clear();
                    continue;
                }
                if let Some(r) = current.as_mut() {
                    match stack.last().map(|s| s.as_str()) {
                        Some("href") => {
                            if r.href.is_empty() {
                                r.href = trimmed.to_string();
                            }
                        }
                        Some("displayname") => {
                            r.display_name = Some(trimmed.to_string());
                        }
                        Some("status") => {
                            // e.g. "HTTP/1.1 404 Not Found"
                            r.status = trimmed
                                .split_whitespace()
                                .find_map(|t| t.parse::<u16>().ok());
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(e)) => {
                let name = local_name(e.name().as_ref());
                stack.pop();
                if name == "response" {
                    if let Some(r) = current.take() {
                        responses.push(r);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("CardDAV-XML-Parsing fehlgeschlagen: {}", e)),
            _ => {}
        }
        buf.clear();
    }

    Ok(responses)
}

/// Extract the `<sync-token>` from a sync-collection response.
fn parse_sync_token_xml(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut in_token = false;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if local_name(e.name().as_ref()) == "sync-token" {
                    in_token = true;
                }
            }
            Ok(Event::Text(e)) if in_token => {
                let t = e.unescape().map(|c| c.into_owned()).unwrap_or_default();
                let t = t.trim().to_string();
                if !t.is_empty() {
                    return Some(t);
                }
            }
            Ok(Event::End(e)) => {
                if local_name(e.name().as_ref()) == "sync-token" {
                    in_token = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardDavSettings {
    pub url: String,
    pub username: String,
    pub password: String,
    pub sync_interval_minutes: u64,
}

impl Default for CardDavSettings {
    fn default() -> Self {
        Self {
            url: String::new(),
            username: String::new(),
            password: String::new(),
            sync_interval_minutes: 30,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Addressbook {
    pub href: String,
    pub display_name: Option<String>,
}

pub struct CardDavClient {
    settings: CardDavSettings,
    http: reqwest_digest_auth::Client,
}

impl CardDavClient {
    pub fn new(settings: CardDavSettings) -> Self {
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

    /// Discover addressbook collections by PROPFIND on the given URL.
    /// Looks for resources with <CardDAV:addressbook/> in their resourcetype.
    async fn discover_addressbooks(&self, base_url: &str) -> Result<Vec<Addressbook>, String> {
        let body = r#"<d:propfind xmlns:d="DAV:" xmlns:CardDAV="urn:ietf:params:xml:ns:carddav">
            <d:prop>
                <d:resourcetype/>
                <d:displayname/>
                <d:href/>
            </d:prop>
        </d:propfind>"#;

        let resp = self
            .http
            .request(propfind_method().clone(), base_url)
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .header("User-Agent", "Relay-CardDAV/1.0")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("Discovery-PROPFIND fehlgeschlagen: {}", e))?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;

        if status != reqwest::StatusCode::MULTI_STATUS {
            return Err(format!("Discovery: unerwarteter Status {} (erwartet 207)", status));
        }

        // Parse responses with a real XML parser and keep those whose
        // resourcetype contains <addressbook/>.
        let books = parse_multistatus(&text)?
            .into_iter()
            .filter(|r| r.is_addressbook && !r.href.is_empty())
            .map(|r| Addressbook { href: r.href, display_name: r.display_name })
            .collect();

        Ok(books)
    }

    /// Resolve the effective addressbook URL from the configured URL.
    ///
    /// 1. Discover addressbooks via PROPFIND on the configured URL.
    /// 2. If exactly one addressbook is found: return its absolute URL.
    /// 3. If multiple are found: warn and return the first.
    /// 4. If none are found: fall back to the configured URL (legacy).
    async fn resolve_addressbook_url(&self) -> Result<String, String> {
        let base_url = self.settings.url.trim_end_matches('/').to_string();
        let books = self.discover_addressbooks(&base_url).await?;

        if books.is_empty() {
            tracing::info!(
                "CardDAV: keine Adressbücher auf {} gefunden – verwende Basis-URL",
                base_url
            );
            return Ok(base_url);
        }

        let book = &books[0];
        if books.len() > 1 {
            tracing::warn!(
                "CardDAV: {} Adressbücher gefunden, verwende erstes ({})",
                books.len(),
                book.display_name.as_deref().unwrap_or(&book.href),
            );
        }

        let abs_url = resolve_href(&base_url, &book.href);
        tracing::info!(
            "CardDAV: Adressbuch '{}' auf {} gefunden",
            book.display_name.as_deref().unwrap_or("(unbenannt)"),
            abs_url,
        );
        Ok(abs_url)
    }

    /// Fetch all contacts from the CardDAV server.
    /// Automatically discovers addressbook collections.
    /// Returns (contacts, sync_token) for incremental sync.
    pub async fn fetch_all(&self) -> Result<(Vec<Contact>, String), String> {
        if self.settings.url.is_empty() {
            return Err("CardDAV-Server-URL nicht konfiguriert".into());
        }

        let ab_url = self.resolve_addressbook_url().await?;

        // Step 1: List contact URLs from the addressbook
        let contact_urls = self.list_contacts(&ab_url).await?;

        if contact_urls.is_empty() {
            let hint = format!(
                "CardDAV-Fehler: Keine Kontakte auf dem Server gefunden.\n\n\
                 URL: {}\n\n\
                 Mögliche Ursachen:\n\
                 - Die URL zeigt auf einen Benutzerordner, nicht auf ein Adressbuch.\n\
                 - Das Adressbuch ist leer.\n\n\
                 Erwartet wird eine Adressbuch-Collection, z.B.:\n\
                 https://server/Benutzer/AdressbuchName/\n\n\
                 Tipp: Auf Olares lautet die URL meist:\n\
                 https://cal.ihre-domain.de/Benutzername/AdressbuchName/",
                self.settings.url,
            );
            return Err(hint);
        }

        // Step 2: Fetch each vCard
        let mut contacts = Vec::new();
        for url in &contact_urls {
            match self.fetch_vcard(url).await {
                Ok(vcard) => {
                    let contact = vcard::parse_vcard(&vcard);
                    contacts.push(contact);
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch vCard from {}: {}", url, e);
                }
            }
        }

        // Step 3: Get sync token for incremental updates
        let sync_token = self.get_sync_token(&ab_url).await.unwrap_or_default();

        tracing::info!(
            "CardDAV: {} Kontakte von {} synchronisiert",
            contacts.len(),
            ab_url,
        );
        Ok((contacts, sync_token))
    }

    /// Incremental sync using SYNC-COLLECTION.
    /// Returns (added/modified contacts, deleted UIDs, new sync token).
    pub async fn sync_incremental(
        &self,
        sync_token: &str,
    ) -> Result<(Vec<Contact>, Vec<String>, String), String> {
        if self.settings.url.is_empty() {
            return Err("CardDAV-Server-URL nicht konfiguriert".into());
        }

        let ab_url = self.resolve_addressbook_url().await?;

        let body = format!(
            r#"<d:propfind xmlns:d="DAV:" xmlns:CardDAV="urn:ietf:params:xml:ns:carddav">
                <d:sync-token>{}</d:sync-token>
                <d:prop>
                    <d:sync-token/>
                </d:prop>
            </d:propfind>"#,
            xml_escape(sync_token)
        );

        let resp = self
            .http
            .request(propfind_method().clone(), &ab_url)
            .header("Depth", "1")
            .header("Sync-Level", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .header("User-Agent", "Relay-CardDAV/1.0")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("SYNC-COLLECTION fehlgeschlagen: {}", e))?;

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
                match self.fetch_vcard(url).await {
                    Ok(vcard) => {
                        let contact = vcard::parse_vcard(&vcard);
                        added.push(contact);
                    }
                    Err(e) => tracing::warn!("Failed to fetch updated vCard: {}", e),
                }
            }

            Ok((added, deleted_uids, new_sync_token))
        } else {
            Err(format!("SYNC-COLLECTION: unerwarteter Status {}", status))
        }
    }

    async fn list_contacts(&self, base_url: &str) -> Result<Vec<String>, String> {
        let body = r#"<d:propfind xmlns:d="DAV:" xmlns:CardDAV="urn:ietf:params:xml:ns:carddav">
            <CardDAV:addressbook-query>
                <CardDAV:filter>
                    <CardDAV:prop/>
                </CardDAV:filter>
            </CardDAV:addressbook-query>
            <d:prop>
                <d:href/>
            </d:prop>
        </d:propfind>"#;

        let resp = self
            .http
            .request(propfind_method().clone(), base_url)
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .header("User-Agent", "Relay-CardDAV/1.0")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("PROPFIND fehlgeschlagen: {}", e))?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;

        if status != reqwest::StatusCode::MULTI_STATUS {
            return Err(format!("PROPFIND: unerwarteter Status {}", status));
        }

        let urls = parse_hrefs(&text)?.into_iter()
            .map(|u| resolve_href(base_url, &u))
            .collect();

        Ok(urls)
    }

    async fn fetch_vcard(&self, url: &str) -> Result<String, String> {
        let resp = self
            .http
            .get(url)
            .header("User-Agent", "Relay-CardDAV/1.0")
            .send()
            .await
            .map_err(|e| format!("GET fehlgeschlagen: {}", e))?;

        let text = resp.text().await.map_err(|e| e.to_string())?;
        Ok(text)
    }

    async fn get_sync_token(&self, base_url: &str) -> Result<String, String> {
        let body = r#"<d:propfind xmlns:d="DAV:">
            <d:prop>
                <d:sync-token/>
            </d:prop>
        </d:propfind>"#;

        let resp = self
            .http
            .request(propfind_method().clone(), base_url)
            .header("Depth", "0")
            .header("Content-Type", "application/xml; charset=utf-8")
            .header("User-Agent", "Relay-CardDAV/1.0")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("Sync-token fehlgeschlagen: {}", e))?;

        let text = resp.text().await.map_err(|e| e.to_string())?;
        parse_sync_token(&text).ok_or("Kein Sync-Token gefunden".into())
    }
}

/// Resolve a potentially-relative href against a base URL.
/// If href starts with `/`, it's an absolute-path reference → resolve against origin.
/// Otherwise return as-is.
fn resolve_href(base_url: &str, href: &str) -> String {
    if href.starts_with('/') {
        let parts: Vec<&str> = base_url.splitn(4, '/').collect();
        if parts.len() >= 3 {
            let origin = &parts[..3].join("/");
            format!("{}{}", origin, href)
        } else {
            href.to_string()
        }
    } else {
        href.to_string()
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Collect contact hrefs from a multistatus body (skips collection hrefs
/// ending in `/`).
fn parse_hrefs(xml: &str) -> Result<Vec<String>, String> {
    Ok(parse_multistatus(xml)?
        .into_iter()
        .map(|r| r.href)
        .filter(|h| !h.is_empty() && !h.ends_with('/'))
        .collect())
}

fn parse_sync_token(xml: &str) -> Option<String> {
    parse_sync_token_xml(xml)
}

/// Parse a sync-collection multistatus into (added/changed hrefs, deleted UIDs).
/// A `<response>` with HTTP 404 marks a deletion; its trailing path segment is
/// used as the contact UID.
fn parse_sync_response(xml: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let mut added_urls = Vec::new();
    let mut deleted_uids = Vec::new();

    for r in parse_multistatus(xml)? {
        if r.href.is_empty() {
            continue;
        }
        if r.status == Some(404) {
            if let Some(filename) = r.href.rsplit('/').find(|s| !s.is_empty()) {
                deleted_uids.push(filename.to_string());
            }
        } else if !r.href.ends_with('/') {
            added_urls.push(r.href);
        }
    }

    Ok((added_urls, deleted_uids))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_multistatus_addressbook_discovery() {
        let xml = r#"<?xml version="1.0"?>
        <d:multistatus xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav">
          <d:response>
            <d:href>/dav/addressbooks/user/contacts/</d:href>
            <d:propstat>
              <d:prop>
                <d:resourcetype><d:collection/><card:addressbook/></d:resourcetype>
                <d:displayname>Meine Kontakte</d:displayname>
              </d:prop>
              <d:status>HTTP/1.1 200 OK</d:status>
            </d:propstat>
          </d:response>
          <d:response>
            <d:href>/dav/addressbooks/user/</d:href>
            <d:propstat>
              <d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop>
              <d:status>HTTP/1.1 200 OK</d:status>
            </d:propstat>
          </d:response>
        </d:multistatus>"#;

        let responses = parse_multistatus(xml).unwrap();
        assert_eq!(responses.len(), 2);
        let books: Vec<_> = responses.iter().filter(|r| r.is_addressbook).collect();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].href, "/dav/addressbooks/user/contacts/");
        assert_eq!(books[0].display_name.as_deref(), Some("Meine Kontakte"));
    }

    #[test]
    fn test_parse_hrefs_skips_collections() {
        let xml = r#"<multistatus xmlns="DAV:">
          <response><href>/c/a.vcf</href><status>HTTP/1.1 200 OK</status></response>
          <response><href>/c/</href><status>HTTP/1.1 200 OK</status></response>
          <response><href>/c/b.vcf</href><status>HTTP/1.1 200 OK</status></response>
        </multistatus>"#;
        let hrefs = parse_hrefs(xml).unwrap();
        assert_eq!(hrefs, vec!["/c/a.vcf".to_string(), "/c/b.vcf".to_string()]);
    }

    #[test]
    fn test_parse_sync_response_added_and_deleted() {
        let xml = r#"<d:multistatus xmlns:d="DAV:">
          <d:response>
            <d:href>/c/new.vcf</d:href>
            <d:propstat><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
          </d:response>
          <d:response>
            <d:href>/c/gone.vcf</d:href>
            <d:status>HTTP/1.1 404 Not Found</d:status>
          </d:response>
          <d:sync-token>http://example.com/sync/42</d:sync-token>
        </d:multistatus>"#;
        let (added, deleted) = parse_sync_response(xml).unwrap();
        assert_eq!(added, vec!["/c/new.vcf".to_string()]);
        assert_eq!(deleted, vec!["gone.vcf".to_string()]);
        assert_eq!(parse_sync_token(xml).as_deref(), Some("http://example.com/sync/42"));
    }

    #[test]
    fn test_parse_handles_uppercase_namespace_prefix() {
        let xml = r#"<D:multistatus xmlns:D="DAV:">
          <D:response><D:href>/c/x.vcf</D:href><D:status>HTTP/1.1 200 OK</D:status></D:response>
        </D:multistatus>"#;
        let hrefs = parse_hrefs(xml).unwrap();
        assert_eq!(hrefs, vec!["/c/x.vcf".to_string()]);
    }

    /// Integration test against radicale-aimighty on Olares.
    ///
    /// Tests individual CardDAV methods (list_contacts, fetch_vcard, get_sync_token)
    /// rather than the full fetch_all (which fetches all contacts sequentially
    /// and can be slow with many contacts).
    ///
    /// Configure via environment variables:
    ///   CARDDAV_URL  (default: https://cal.aimighty.olares.de/Marc/marcbayerund100weitere/)
    ///   CARDDAV_USER (default: Marc)
    ///   CARDDAV_PASS (default: empty)
    ///
    /// Run with:
    /// ```bash
    /// cargo test test_carddav -- --ignored
    /// ```
    #[ignore]
    #[tokio::test]
    async fn test_carddav_list_contacts() {
        let url = std::env::var("CARDDAV_URL")
            .unwrap_or_else(|_| "https://cal.aimighty.olares.de/Marc/marcbayerund100weitere/".into());
        let user = std::env::var("CARDDAV_USER")
            .unwrap_or_else(|_| "Marc".into());
        let pass = std::env::var("CARDDAV_PASS")
            .unwrap_or_default();

        let settings = CardDavSettings {
            url: url.clone(),
            username: user,
            password: pass,
            sync_interval_minutes: 30,
        };
        let base_url = settings.url.trim_end_matches('/').to_string();

        let client = CardDavClient::new(settings);
        let urls = client.list_contacts(&base_url).await
            .expect("list_contacts fehlgeschlagen");

        assert!(!urls.is_empty(), "Keine Kontakt-URLs abgerufen");
        assert!(urls[0].starts_with("http"), "Kontakt-URL sollte absolut sein: {}", urls[0]);

        eprintln!("[test] CardDAV: {} Kontakt-URLs geladen, erste: {}", urls.len(), urls[0]);
    }

    #[ignore]
    #[tokio::test]
    async fn test_carddav_fetch_and_parse_vcard() {
        let url = std::env::var("CARDDAV_URL")
            .unwrap_or_else(|_| "https://cal.aimighty.olares.de/Marc/marcbayerund100weitere/".into());
        let user = std::env::var("CARDDAV_USER")
            .unwrap_or_else(|_| "Marc".into());
        let pass = std::env::var("CARDDAV_PASS")
            .unwrap_or_default();

        let settings = CardDavSettings {
            url: url.clone(),
            username: user,
            password: pass,
            sync_interval_minutes: 30,
        };
        let base_url = settings.url.trim_end_matches('/').to_string();

        let client = CardDavClient::new(settings);
        let urls = client.list_contacts(&base_url).await
            .expect("list_contacts fehlgeschlagen");

        assert!(!urls.is_empty(), "Keine Kontakt-URLs");
        assert!(urls[0].starts_with("http"), "URL sollte absolut sein");

        let raw_vcard = client.fetch_vcard(&urls[0]).await
            .expect("fetch_vcard fehlgeschlagen");
        assert!(!raw_vcard.is_empty(), "vCard ist leer");
        assert!(raw_vcard.starts_with("BEGIN:VCARD"), "vCard sollte mit BEGIN:VCARD beginnen");

        let contact = vcard::parse_vcard(&raw_vcard);
        assert!(!contact.vcard_uid.is_empty(), "vcard_uid sollte vorhanden sein");
        assert!(!contact.vcard_raw.is_empty(), "vcard_raw sollte vorhanden sein");

        eprintln!(
            "[test] CardDAV: vCard geladen ({} Bytes), Kontakt: {}",
            raw_vcard.len(),
            contact.display_name.as_deref().unwrap_or("(kein Name)"),
        );
    }

    #[ignore]
    #[tokio::test]
    async fn test_carddav_discover_addressbooks() {
        let url = std::env::var("CARDDAV_URL")
            .unwrap_or_else(|_| "https://cal.aimighty.olares.de/Marc/".into());
        let user = std::env::var("CARDDAV_USER")
            .unwrap_or_else(|_| "Marc".into());
        let pass = std::env::var("CARDDAV_PASS")
            .unwrap_or_default();

        let settings = CardDavSettings {
            url: url.clone(),
            username: user,
            password: pass,
            sync_interval_minutes: 30,
        };
        let base_url = settings.url.trim_end_matches('/').to_string();

        let client = CardDavClient::new(settings);
        let books = client.discover_addressbooks(&base_url).await
            .expect("discover_addressbooks fehlgeschlagen");

        assert!(!books.is_empty(), "Es wurden keine Adressbücher gefunden");
        assert!(books[0].href.contains("marcbayerund100weitere"),
            "Erwartetes Adressbuch 'marcbayerund100weitere', gefunden: {}", books[0].href);
        assert!(books[0].display_name.is_some(), "Adressbuch sollte einen display_name haben");

        eprintln!(
            "[test] CardDAV: {} Adressbücher gefunden, erstes: '{}' ({})",
            books.len(),
            books[0].display_name.as_deref().unwrap_or("?"),
            books[0].href,
        );
    }

    #[ignore]
    #[tokio::test]
    async fn test_carddav_fetch_via_discovery() {
        let url = std::env::var("CARDDAV_URL")
            .unwrap_or_else(|_| "https://cal.aimighty.olares.de/Marc/".into());
        let user = std::env::var("CARDDAV_USER")
            .unwrap_or_else(|_| "Marc".into());
        let pass = std::env::var("CARDDAV_PASS")
            .unwrap_or_default();

        let settings = CardDavSettings {
            url: url.clone(),
            username: user,
            password: pass,
            sync_interval_minutes: 30,
        };

        let client = CardDavClient::new(settings);
        let (contacts, sync_token) = client.fetch_all().await
            .expect("fetch_all via discovery fehlgeschlagen");

        assert!(!contacts.is_empty(), "Keine Kontakte via Discovery gefunden");
        assert!(!sync_token.is_empty(), "Sync-Token sollte nicht leer sein");

        eprintln!(
            "[test] CardDAV: fetch_via_discovery: {} Kontakte, token='{}'",
            contacts.len(),
            sync_token,
        );
    }

    #[ignore]
    #[tokio::test]
    async fn test_carddav_sync_token() {
        let url = std::env::var("CARDDAV_URL")
            .unwrap_or_else(|_| "https://cal.aimighty.olares.de/Marc/marcbayerund100weitere/".into());
        let user = std::env::var("CARDDAV_USER")
            .unwrap_or_else(|_| "Marc".into());
        let pass = std::env::var("CARDDAV_PASS")
            .unwrap_or_default();

        let settings = CardDavSettings {
            url: url.clone(),
            username: user,
            password: pass,
            sync_interval_minutes: 30,
        };
        let base_url = settings.url.trim_end_matches('/').to_string();

        let client = CardDavClient::new(settings);
        let sync_token = client.get_sync_token(&base_url).await
            .expect("get_sync_token fehlgeschlagen");

        assert!(!sync_token.is_empty(), "Sync-Token sollte nicht leer sein");
        eprintln!("[test] CardDAV: sync_token='{}'", sync_token);
    }
}
