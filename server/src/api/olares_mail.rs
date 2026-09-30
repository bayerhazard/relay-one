//! `GET /api/v1/olares-mail/status` — report which mail/identity values Olares
//! injected through the chart's `envs[]` mapping, so the UI can offer to adopt
//! them on first launch. Secrets are never returned; passwords are only
//! reported as present/absent.

use axum::Json;
use serde::Serialize;

use super::ApiResult;

/// Read a trimmed env var, treating empty/whitespace as unset.
pub fn env_value(name: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_default()
}

fn opt(name: &str) -> Option<String> {
    let v = env_value(name);
    if v.is_empty() { None } else { Some(v) }
}

fn present(name: &str) -> bool {
    !env_value(name).is_empty()
}

fn port(name: &str) -> Option<u16> {
    env_value(name).parse::<u16>().ok()
}

fn boolish(name: &str) -> Option<bool> {
    opt(name).map(|s| {
        matches!(s.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on")
    })
}

#[derive(Serialize)]
pub struct MailSide {
    pub server: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password_present: bool,
}

#[derive(Serialize)]
pub struct Identity {
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub username: Option<String>,
    pub timezone: Option<String>,
}

#[derive(Serialize)]
pub struct Company {
    pub name: Option<String>,
    pub address: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub email: Option<String>,
    pub vat_id: Option<String>,
    pub register: Option<String>,
}

#[derive(Serialize)]
pub struct OlaresMailStatus {
    /// True when Olares supplied at least one relevant value.
    pub configured: bool,
    /// True when an account can be created end-to-end from the env alone.
    pub complete: bool,
    pub identity: Identity,
    pub company: Company,
    pub account_name: Option<String>,
    pub signature: Option<String>,
    pub imap: MailSide,
    pub imap_ssl: Option<bool>,
    pub smtp: MailSide,
    pub smtp_security: Option<String>,
    pub smtp_from_address: Option<String>,
    pub smtp_enabled: Option<bool>,
    /// Informational list of fields still required for a full auto-setup.
    pub missing: Vec<String>,
}

/// `GET /api/v1/olares-mail/status`
pub async fn olares_mail_status() -> ApiResult<OlaresMailStatus> {
    let identity = Identity {
        first_name: opt("RELAY_OLARES_FIRSTNAME"),
        last_name: opt("RELAY_OLARES_LASTNAME"),
        email: opt("RELAY_OLARES_EMAIL"),
        username: opt("RELAY_OLARES_USERNAME"),
        timezone: opt("RELAY_OLARES_TIMEZONE"),
    };
    let company = Company {
        name: opt("RELAY_OLARES_COMPANY_NAME"),
        address: opt("RELAY_OLARES_COMPANY_ADDRESS"),
        phone: opt("RELAY_OLARES_COMPANY_PHONE"),
        website: opt("RELAY_OLARES_COMPANY_WEBSITE"),
        email: opt("RELAY_OLARES_COMPANY_EMAIL"),
        vat_id: opt("RELAY_OLARES_COMPANY_VAT_ID"),
        register: opt("RELAY_OLARES_COMPANY_REGISTER"),
    };

    let imap_pw = present("RELAY_OLARES_IMAP_PASSWORD");
    let smtp_pw = present("RELAY_OLARES_SMTP_PASSWORD");
    let imap = MailSide {
        server: opt("RELAY_OLARES_IMAP_SERVER"),
        port: port("RELAY_OLARES_IMAP_PORT"),
        username: opt("RELAY_OLARES_IMAP_USERNAME").or_else(|| identity.email.clone()),
        password_present: imap_pw,
    };
    let from = opt("RELAY_OLARES_SMTP_FROM_ADDRESS").or_else(|| identity.email.clone());
    let smtp = MailSide {
        server: opt("RELAY_OLARES_SMTP_SERVER"),
        port: port("RELAY_OLARES_SMTP_PORT"),
        username: opt("RELAY_OLARES_SMTP_USERNAME").or_else(|| identity.email.clone()),
        // SMTP may fall back to the IMAP password (see connect_account).
        password_present: smtp_pw || imap_pw,
    };

    let mut missing = Vec::new();
    if imap.server.is_none() { missing.push("imap.server".to_string()); }
    if imap.username.is_none() { missing.push("imap.username".to_string()); }
    if !imap.password_present { missing.push("imap.password".to_string()); }
    if smtp.server.is_none() { missing.push("smtp.server".to_string()); }
    if from.is_none() { missing.push("sender.email".to_string()); }

    let complete = missing.is_empty();
    let configured = imap.server.is_some()
        || smtp.server.is_some()
        || identity.email.is_some()
        || identity.first_name.is_some()
        || company.name.is_some();

    Ok(Json(OlaresMailStatus {
        configured,
        complete,
        identity,
        company,
        account_name: opt("RELAY_OLARES_MAIL_ACCOUNT_NAME"),
        signature: opt("RELAY_OLARES_MAIL_SIGNATURE"),
        imap,
        imap_ssl: boolish("RELAY_OLARES_IMAP_SSL"),
        smtp,
        smtp_security: opt("RELAY_OLARES_SMTP_SECURITY_PROTOCOLS"),
        smtp_from_address: from,
        smtp_enabled: boolish("RELAY_OLARES_SMTP_ENABLED"),
        missing,
    }))
}
