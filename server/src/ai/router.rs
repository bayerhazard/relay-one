//! Olares Router integration (Olares 1.12.7+).
//!
//! Router is the platform AI gateway. Every model app on the box is reachable
//! through one address and one system model name per capability:
//!
//! * LLM   — `<base>/chat/completions`, model `default-chat`
//! * STT   — `<base>/audio/transcriptions`, model `default-stt`
//! * TTS   — `<base>/audio/speech`, model `default-tts`
//!
//! An app inside Olares sends **no API key**: the platform stamps
//! `X-Olares-App-ID` at the edge. `default-*` are routing names and are
//! deliberately absent from `/v1/models`, so they must never be validated
//! against that list.
//!
//! Relay defaults to Router and lets the user override each capability
//! manually (`Source::Manual`).

use rusqlite::Connection;

use crate::ai::client::AIConfig;
use crate::cache::settings::{get_setting, set_setting};

/// Compiled fallback. Matches the AIMighty deployment; overridable by the
/// `RELAY_ROUTER_URL` env or the stored `router_url` setting.
pub const DEFAULT_ROUTER_BASE: &str = "https://router.aimighty.olares.de/v1";

pub const CHAT_MODEL: &str = "default-chat";
pub const STT_MODEL: &str = "default-stt";
pub const TTS_MODEL: &str = "default-tts";

pub const KEY_AI_SOURCE: &str = "ai_source";
pub const KEY_VOICE_SOURCE: &str = "voice_source";
pub const KEY_ROUTER_URL: &str = "router_url";
pub const KEY_ZONE: &str = "olares_zone";

/// Which endpoint a capability uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Router,
    Manual,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Router => "router",
            Source::Manual => "manual",
        }
    }

    pub fn parse(s: &str) -> Source {
        if s.trim().eq_ignore_ascii_case("manual") {
            Source::Manual
        } else {
            Source::Router
        }
    }
}

/// A Router base URL that the user configured (or that we learned) counts as
/// "already on Router" for the legacy-config inference.
pub fn is_router_url(url: &str) -> bool {
    let u = url.trim();
    if u.is_empty() {
        return false;
    }
    // Host part after scheme.
    let host = u
        .split("://")
        .nth(1)
        .unwrap_or(u)
        .split('/')
        .next()
        .unwrap_or("");
    host == "router"
        || host.starts_with("router.")
        || host.contains(".router.")
        || host.ends_with("router.olares.de")
}

/// Derive the Olares zone (`aimighty.olares.de`) from a public request Host
/// (`mail.aimighty.olares.de` → `aimighty.olares.de`).
///
/// Returns `None` for IPs, bare service names and in-cluster DNS so an
/// internal caller can never plant a bogus zone.
pub fn zone_from_host(host: &str) -> Option<String> {
    let host = host.trim();
    if host.is_empty() || host.starts_with('[') {
        return None;
    }
    // Strip a port suffix (IPv6 is already excluded by the '[' guard).
    let host = host.split(':').next().unwrap_or(host);
    if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
        return None;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return None;
    }
    // Reject all-numeric labels (an IP without dots would not get here, but
    // `1.2.3.4` would).
    if labels.iter().all(|l| l.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let zone = labels[1..].join(".");
    if zone.ends_with(".cluster.local")
        || zone.ends_with(".svc")
        || zone.ends_with(".local")
        || zone == "local"
    {
        return None;
    }
    Some(zone)
}

/// Pure resolver: env → stored → learned zone → compiled default.
pub fn resolve_base(env_url: Option<&str>, stored_url: Option<&str>, zone: Option<&str>) -> String {
    for candidate in [env_url, stored_url] {
        if let Some(u) = candidate {
            let u = u.trim().trim_end_matches('/');
            if !u.is_empty() {
                return u.to_string();
            }
        }
    }
    if let Some(z) = zone {
        let z = z.trim().trim_matches('.').trim_end_matches('/');
        if !z.is_empty() {
            return format!("https://router.{z}/v1");
        }
    }
    DEFAULT_ROUTER_BASE.to_string()
}

/// Effective base URL, reading env + DB. `zone` comes from the live request
/// host (see `AppState::router_zone`).
pub fn base_from_conn(conn: &Connection, zone: Option<&str>) -> String {
    let env_url = std::env::var("RELAY_ROUTER_URL").ok();
    let stored_url = get_setting(conn, KEY_ROUTER_URL).ok().flatten();
    resolve_base(env_url.as_deref(), stored_url.as_deref(), zone)
}

/// Legacy inference: an explicitly stored `ai_source` wins; otherwise an
/// existing non-Router `ai_url` means the user configured manually and we
/// must not silently switch them to Router.
pub fn ai_source(conn: &Connection) -> Source {
    if let Ok(Some(v)) = get_setting(conn, KEY_AI_SOURCE) {
        return Source::parse(&v);
    }
    match get_setting(conn, "ai_url") {
        Ok(Some(url)) if !url.trim().is_empty() && !is_router_url(&url) => Source::Manual,
        _ => Source::Router,
    }
}

pub fn voice_source(conn: &Connection) -> Source {
    if let Ok(Some(v)) = get_setting(conn, KEY_VOICE_SOURCE) {
        return Source::parse(&v);
    }
    if voice_endpoints_are_manual(conn) {
        Source::Manual
    } else {
        Source::Router
    }
}

/// True when the stored `voice_settings` row holds a non-Router STT or TTS URL
/// (legacy manual configuration). The URLs live in the `voice_settings` table,
/// not in `settings`.
pub fn voice_endpoints_are_manual(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT stt_url, tts_url FROM voice_settings WHERE id = 1",
        [],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )
    .map(|(stt, tts)| {
        let manual = |u: &str| !u.trim().is_empty() && !is_router_url(u);
        manual(&stt) || manual(&tts)
    })
    .unwrap_or(false)
}

pub fn set_ai_source(conn: &Connection, source: Source) -> Result<(), rusqlite::Error> {
    set_setting(conn, KEY_AI_SOURCE, source.as_str())
}

pub fn set_voice_source(conn: &Connection, source: Source) -> Result<(), rusqlite::Error> {
    set_setting(conn, KEY_VOICE_SOURCE, source.as_str())
}

pub fn set_router_url(conn: &Connection, url: &str) -> Result<(), rusqlite::Error> {
    set_setting(conn, KEY_ROUTER_URL, url.trim())
}

pub fn stored_zone(conn: &Connection) -> Option<String> {
    get_setting(conn, KEY_ZONE).ok().flatten()
}

pub fn set_zone(conn: &Connection, zone: &str) -> Result<(), rusqlite::Error> {
    set_setting(conn, KEY_ZONE, zone.trim())
}

/// Synthesize the LLM config for Router mode.
pub fn chat_config(base: &str) -> AIConfig {
    AIConfig {
        url: base.to_string(),
        api_key: String::new(),
        model: CHAT_MODEL.to_string(),
        ..Default::default()
    }
}

/// The Router-mode STT/TTS values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceEndpoints {
    pub stt_url: String,
    pub stt_key: String,
    pub stt_model: String,
    pub tts_url: String,
    pub tts_key: String,
    pub tts_model: String,
}

pub fn voice_endpoints(base: &str) -> VoiceEndpoints {
    VoiceEndpoints {
        stt_url: base.to_string(),
        stt_key: String::new(),
        stt_model: STT_MODEL.to_string(),
        tts_url: base.to_string(),
        tts_key: String::new(),
        tts_model: TTS_MODEL.to_string(),
    }
}

/// Effective base URL for the live state (DB + learned zone + env).
pub fn base_for_state(state: &crate::AppState) -> String {
    let zone = state.router_zone.read().clone();
    let guard = state.cache_db.lock();
    match guard.as_ref() {
        Some(conn) => base_from_conn(conn, zone.as_deref()),
        None => resolve_base(None, None, zone.as_deref()),
    }
}

/// Cached reachability probe (60 s TTL) so the settings UI can show status
/// without hitting Router on every render.
pub async fn available_cached(state: &crate::AppState) -> bool {
    {
        let g = state.router_status.lock();
        if let Some((val, at)) = *g {
            if at.elapsed() < std::time::Duration::from_secs(60) {
                return val;
            }
        }
    }
    let base = base_for_state(state);
    let ok = probe(&base).await;
    *state.router_status.lock() = Some((ok, std::time::Instant::now()));
    ok
}

/// Probe Router reachability (`GET <base>/models`). `default-*` is not listed
/// there, so this only proves Router answers — that is all we need.
pub async fn probe(base: &str) -> bool {
    let url = format!("{}/models", base.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    matches!(client.get(&url).send().await, Ok(r) if r.status().is_success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::db;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        db::init_db(&c).unwrap();
        c
    }

    #[test]
    fn zone_from_public_host() {
        assert_eq!(
            zone_from_host("mail.aimighty.olares.de").as_deref(),
            Some("aimighty.olares.de")
        );
        assert_eq!(
            zone_from_host("31747cb8.aimighty.olares.de:443").as_deref(),
            Some("aimighty.olares.de")
        );
    }

    #[test]
    fn zone_from_host_rejects_internal_and_ip() {
        assert_eq!(zone_from_host("127.0.0.1:3000"), None);
        assert_eq!(zone_from_host("[::1]:8080"), None);
        assert_eq!(zone_from_host("localhost"), None);
        assert_eq!(zone_from_host("relay"), None);
        assert_eq!(
            zone_from_host("relay-svc.relay-aimighty.svc.cluster.local"),
            None
        );
    }

    #[test]
    fn resolve_prefers_env_then_stored_then_zone_then_default() {
        assert_eq!(
            resolve_base(Some("https://env/v1"), Some("https://stored/v1"), Some("z.de")),
            "https://env/v1"
        );
        assert_eq!(
            resolve_base(None, Some("https://stored/v1"), Some("z.de")),
            "https://stored/v1"
        );
        assert_eq!(
            resolve_base(None, None, Some("aimighty.olares.de")),
            "https://router.aimighty.olares.de/v1"
        );
        assert_eq!(resolve_base(None, None, None), DEFAULT_ROUTER_BASE);
        // Blank candidates are ignored.
        assert_eq!(resolve_base(Some("  "), Some(""), None), DEFAULT_ROUTER_BASE);
    }

    #[test]
    fn chat_config_is_router() {
        let c = chat_config("https://router.example/v1");
        assert_eq!(c.url, "https://router.example/v1");
        assert_eq!(c.model, CHAT_MODEL);
        assert!(c.api_key.is_empty());
    }

    #[test]
    fn voice_endpoints_are_router() {
        let v = voice_endpoints("https://router.example/v1");
        assert_eq!(v.stt_model, STT_MODEL);
        assert_eq!(v.tts_model, TTS_MODEL);
        assert!(v.stt_key.is_empty() && v.tts_key.is_empty());
    }

    #[test]
    fn is_router_url_detects_router_hosts() {
        assert!(is_router_url("https://router.aimighty.olares.de/v1"));
        assert!(!is_router_url("https://llm.aimighty.olares.de/v1"));
        assert!(!is_router_url(""));
    }

    #[test]
    fn source_defaults_router_on_fresh_db() {
        let c = conn();
        assert_eq!(ai_source(&c), Source::Router);
        assert_eq!(voice_source(&c), Source::Router);
    }

    #[test]
    fn legacy_manual_url_infers_manual() {
        let c = conn();
        set_setting(&c, "ai_url", "https://llm.aimighty.olares.de/v1").unwrap();
        assert_eq!(ai_source(&c), Source::Manual);
    }

    #[test]
    fn legacy_router_url_infers_router() {
        let c = conn();
        set_setting(&c, "ai_url", "https://router.aimighty.olares.de/v1").unwrap();
        assert_eq!(ai_source(&c), Source::Router);
    }

    #[test]
    fn explicit_source_overrides_inference() {
        let c = conn();
        set_setting(&c, "ai_url", "https://llm.aimighty.olares.de/v1").unwrap();
        set_ai_source(&c, Source::Router).unwrap();
        assert_eq!(ai_source(&c), Source::Router);
    }

    #[test]
    fn base_from_conn_uses_stored_url() {
        let c = conn();
        set_router_url(&c, "https://router.custom.tld/v1").unwrap();
        assert_eq!(base_from_conn(&c, None), "https://router.custom.tld/v1");
        assert_eq!(
            base_from_conn(&c, Some("aimighty.olares.de")),
            "https://router.custom.tld/v1"
        );
    }
}
