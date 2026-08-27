//! Generic WebDAV primitives shared by the CardDAV and CalDAV clients.
//!
//! This module owns everything that is protocol-level (WebDAV / RFC 4918)
//! rather than domain-level: the PROPFIND method, multistatus XML parsing,
//! sync-token extraction, href resolution and XML escaping. The CardDAV
//! (`carddav.rs`) and CalDAV (`caldav.rs`) clients build on top of these.

use quick_xml::events::Event;
use quick_xml::reader::Reader;

/// PROPFIND HTTP method — used by every DAV operation.
static PROPFIND_METHOD: std::sync::OnceLock<reqwest::Method> = std::sync::OnceLock::new();

pub fn propfind_method() -> &'static reqwest::Method {
    PROPFIND_METHOD.get_or_init(|| reqwest::Method::from_bytes(b"PROPFIND").unwrap())
}

/// One `<response>` element from a WebDAV multistatus body, parsed
/// namespace-agnostically (prefixes like `d:`/`D:` are ignored).
#[derive(Debug, Default, Clone)]
pub struct DavResponse {
    pub href: String,
    /// HTTP status code from `<status>` (e.g. 404 for a deleted resource).
    pub status: Option<u16>,
    /// Display name from `<displayname>`, if present.
    pub display_name: Option<String>,
    /// Whether the resourcetype contained `<addressbook/>` (CardDAV).
    pub is_addressbook: bool,
    /// Whether the resourcetype contained `<calendar/>` (CalDAV).
    pub is_calendar: bool,
}

/// Returns the local name of an element, stripping any namespace prefix.
pub fn local_name(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    match s.rsplit_once(':') {
        Some((_, local)) => local.to_ascii_lowercase(),
        None => s.to_ascii_lowercase(),
    }
}

/// Parse a WebDAV multistatus XML body into structured responses using a real
/// XML parser (quick-xml). Robust against attributes, self-closing tags,
/// arbitrary namespace prefixes and reordered children.
pub fn parse_multistatus(xml: &str) -> Result<Vec<DavResponse>, String> {
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
                // Self-closing and open tags inside <resourcetype> both mark a
                // resource type (e.g. <calendar/> or <calendar>...</calendar>).
                if in_resourcetype(&stack) && name != "resourcetype" {
                    if let Some(r) = current.as_mut() {
                        mark_resourcetype(r, &name);
                    }
                }
                stack.push(name);
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref());
                if in_resourcetype(&stack) && name != "resourcetype" {
                    if let Some(r) = current.as_mut() {
                        mark_resourcetype(r, &name);
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
            Err(e) => return Err(format!("DAV-XML-Parsing fehlgeschlagen: {}", e)),
            _ => {}
        }
        buf.clear();
    }

    Ok(responses)
}

/// True if any frame on the stack is `<resourcetype>`.
fn in_resourcetype(stack: &[String]) -> bool {
    stack.iter().any(|s| s == "resourcetype")
}

fn mark_resourcetype(r: &mut DavResponse, name: &str) {
    match name {
        "addressbook" => r.is_addressbook = true,
        "calendar" => r.is_calendar = true,
        _ => {}
    }
}

/// Extract the `<sync-token>` from a sync-collection response.
pub fn parse_sync_token_xml(xml: &str) -> Option<String> {
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

/// Resolve a potentially-relative href against a base URL.
/// If href starts with `/`, it's an absolute-path reference → resolve against origin.
/// Otherwise return as-is.
pub fn resolve_href(base_url: &str, href: &str) -> String {
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

pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Collect hrefs from a multistatus body (skips collection hrefs ending in `/`).
pub fn parse_hrefs(xml: &str) -> Result<Vec<String>, String> {
    Ok(parse_multistatus(xml)?
        .into_iter()
        .map(|r| r.href)
        .filter(|h| !h.is_empty() && !h.ends_with('/'))
        .collect())
}

pub fn parse_sync_token(xml: &str) -> Option<String> {
    parse_sync_token_xml(xml)
}

/// Parse a sync-collection multistatus into (added/changed hrefs, deleted UIDs).
/// A `<response>` with HTTP 404 marks a deletion; its trailing path segment is
/// used as the object UID.
pub fn parse_sync_response(xml: &str) -> Result<(Vec<String>, Vec<String>), String> {
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
