use crate::error::AppError;
use base64::Engine;
use lettre::{
    message::{header, Mailbox, MultiPart, SinglePart},
    transport::smtp::authentication::Credentials,
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};
use std::net::IpAddr;
use std::time::Duration;

/// An email attachment with base64-encoded content.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct EmailAttachment {
    pub filename: String,
    /// Base64-encoded file content.
    pub content: String,
    pub content_type: String,
    pub size: usize,
}

/// Network timeout for SMTP operations (connect + send).
const SMTP_TIMEOUT: Duration = Duration::from_secs(60);

/// Returns `true` if `host` is a valid IPv4 or IPv6 address literal.
fn is_ip_address(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok()
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub use_tls: bool,
    pub sender_name: String,
    pub sender_email: String,
}

#[cfg(test)]
pub struct SmtpTestOverrideInner {
    pub sent: std::sync::Mutex<Vec<(String, Vec<u8>)>>,
    pub fail_send: bool,
}

#[cfg(test)]
pub type SmtpTestOverride = std::sync::Arc<SmtpTestOverrideInner>;

pub struct SmtpClient {
    config: SmtpConfig,
    transport: AsyncSmtpTransport<Tokio1Executor>,
    #[cfg(test)]
    pub test_override: Option<SmtpTestOverride>,
}

impl SmtpClient {
    pub fn new(config: SmtpConfig) -> Result<Self, AppError> {
        let creds = Credentials::new(config.username.clone(), config.password.clone());
        let is_ip = is_ip_address(&config.host);

        let transport = if config.use_tls && is_ip {
            return Err(AppError::smtp(format!(
                "SMTP: TLS ist mit IP-Adressen nicht sicher möglich (keine Hostname-Validierung). \
                 Bitte einen DNS-Namen statt '{}' verwenden oder TLS deaktivieren.",
                config.host
            ), "tls_config"));
        } else if config.use_tls {
            tracing::info!(
                "SMTP: use_tls=true, host='{}' is a DNS name — using starttls_relay",
                config.host
            );
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
                .map_err(|e| AppError::smtp(format!(
                    "TLS-Verbindung fehlgeschlagen fuer '{}': {}. Kein unsicherer Fallback.",
                    config.host, e
                ), "tls_connect"))?
                .port(config.port)
                .timeout(Some(SMTP_TIMEOUT))
                .credentials(creds)
                .build()
        } else {
            tracing::warn!(
                "SMTP: use_tls=false, host='{}' — using builder_dangerous (no TLS)",
                config.host
            );
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
                .port(config.port)
                .timeout(Some(SMTP_TIMEOUT))
                .credentials(creds)
                .build()
        };
        Ok(Self {
            config,
            transport,
            #[cfg(test)]
            test_override: None,
        })
    }

    pub async fn send(
        &self,
        to: Vec<(&str, &str)>,
        cc: Vec<(&str, &str)>,
        bcc: Vec<(&str, &str)>,
        subject: &str,
        body_text: &str,
        body_html: Option<&str>,
        in_reply_to: Option<&str>,
        references: Option<&str>,
        attachments: &[EmailAttachment],
    ) -> Result<(String, Vec<u8>), AppError> {
        #[cfg(test)]
        if let Some(ref o) = self.test_override {
            if o.fail_send {
                return Err(AppError::smtp("simulierter Sendefehler", "send"));
            }
            let email = self.build_message(
                to, cc, bcc, subject, body_text, body_html, in_reply_to, references, attachments,
            )?;
            let raw_bytes = email.formatted();
            let message_id = format!("<test-{}@localhost>", uuid::Uuid::new_v4());
            o.sent.lock().unwrap().push((message_id.clone(), raw_bytes.clone()));
            return Ok((message_id, raw_bytes));
        }

        let email = self.build_message(
            to, cc, bcc, subject, body_text, body_html, in_reply_to, references, attachments,
        )?;

        // Capture raw RFC822 bytes BEFORE sending (email.formatted() borrows, send() consumes)
        let raw_bytes = email.formatted();

        let result = self.transport
            .send(email)
            .await
            .map_err(|e| AppError::smtp(e.to_string(), "send"))?;

        let message_id = result.message().next().map(|s| s.to_string()).unwrap_or_default();
        Ok((message_id, raw_bytes))
    }

    /// Build a `lettre::Message` from parameters without sending (for draft saving).
    pub fn build_message(
        &self,
        to: Vec<(&str, &str)>,
        cc: Vec<(&str, &str)>,
        bcc: Vec<(&str, &str)>,
        subject: &str,
        body_text: &str,
        body_html: Option<&str>,
        in_reply_to: Option<&str>,
        references: Option<&str>,
        attachments: &[EmailAttachment],
    ) -> Result<Message, AppError> {
        crate::smtp::client::build_message_from_config(
            &self.config,
            to, cc, bcc, subject, body_text, body_html, in_reply_to, references, attachments,
        )
    }
}

// Make SmtpConfig accessible for build_message_from_config
impl SmtpClient {
    pub fn config(&self) -> &SmtpConfig {
        &self.config
    }
}

/// Build an RFC822 `lettre::Message` from config and fields (free function).
/// Reusable by IPC commands (e.g. save_draft) without SMTP transport.
pub fn build_message_from_config(
    config: &SmtpConfig,
    to: Vec<(&str, &str)>,
    cc: Vec<(&str, &str)>,
    bcc: Vec<(&str, &str)>,
    subject: &str,
    body_text: &str,
    body_html: Option<&str>,
    in_reply_to: Option<&str>,
    references: Option<&str>,
    attachments: &[EmailAttachment],
) -> Result<Message, AppError> {
    let sender = Mailbox::new(
        Some(config.sender_name.clone()),
        config
            .sender_email
            .parse::<lettre::Address>()
            .map_err(|_e| AppError::smtp("Ungueltige Sender-E-Mail-Adresse", "parse_sender"))?,
    );

    let mut builder = Message::builder().from(sender).subject(subject);

    for (email, name) in &to {
        let addr: lettre::Address = email
            .parse::<lettre::Address>()
            .map_err(|_e| AppError::smtp("Ungueltige Empfaenger-E-Mail-Adresse", "parse_to"))?;
        builder = builder.to(Mailbox::new(Some(name.to_string()), addr));
    }

    for (email, name) in &cc {
        let addr: lettre::Address = email
            .parse::<lettre::Address>()
            .map_err(|_e| AppError::smtp("Ungueltige CC-E-Mail-Adresse", "parse_cc"))?;
        builder = builder.cc(Mailbox::new(Some(name.to_string()), addr));
    }

    for (email, name) in &bcc {
        let addr: lettre::Address = email
            .parse::<lettre::Address>()
            .map_err(|_e| AppError::smtp("Ungueltige BCC-E-Mail-Adresse", "parse_bcc"))?;
        builder = builder.bcc(Mailbox::new(Some(name.to_string()), addr));
    }

    if let Some(msg_id) = in_reply_to {
        builder = builder.in_reply_to(msg_id.to_string());
    }
    if let Some(refs) = references {
        builder = builder.references(refs.to_string());
    }

    // Build the body part (text + optional HTML)
    let body_part = match body_html {
        Some(html) => MultiPart::alternative()
            .singlepart(SinglePart::plain(body_text.to_string()))
            .singlepart(SinglePart::html(html.to_string())),
        None => MultiPart::mixed().singlepart(SinglePart::plain(body_text.to_string())),
    };

    // If there are attachments, wrap body in multipart/mixed with attachments
    if attachments.is_empty() {
        let email = builder
            .multipart(body_part)
            .map_err(|e| AppError::smtp(e.to_string(), "build_multipart"))?;
        Ok(email)
    } else {
        let mut mixed = MultiPart::mixed().multipart(body_part);
        for att in attachments {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(att.content.as_bytes())
                .unwrap_or(att.content.as_bytes().to_vec());
            let attachment = SinglePart::builder()
                .header(header::ContentType::parse(&att.content_type)
                    .unwrap_or_else(|_| header::ContentType::parse("application/octet-stream").unwrap_or(header::ContentType::parse("text/plain").unwrap())))
                .header(header::ContentDisposition::attachment(att.filename.as_str()))
                .body(decoded);
            mixed = mixed.singlepart(attachment);
        }
        let email = builder
            .multipart(mixed)
            .map_err(|e| AppError::smtp(e.to_string(), "build_multipart"))?;
        Ok(email)
    }
}

impl SmtpClient {
    pub async fn test_connection(&self) -> Result<bool, AppError> {
        self.transport
            .test_connection()
            .await
            .map_err(|e| AppError::smtp(e.to_string(), "test_connection"))?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> SmtpConfig {
        SmtpConfig {
            host: "localhost".into(),
            port: 587,
            username: "user".into(),
            password: "pass".into(),
            use_tls: false,
            sender_name: "Tester".into(),
            sender_email: "tester@example.com".into(),
        }
    }

    #[test]
    fn test_build_message_from_config_simple() {
        let config = test_config();
        let msg = build_message_from_config(
            &config,
            vec![("empfaenger@example.com", "Empfaenger")],
            vec![],
            vec![],
            "Betreff",
            "Hallo Welt",
            None,
            None,
            None,
            &[],
        )
        .unwrap();

        let raw = msg.formatted();
        let raw_str = String::from_utf8_lossy(&raw);

        assert!(raw_str.contains("From: Tester <tester@example.com>"), "From header: {raw_str}");
        assert!(raw_str.contains("To: Empfaenger <empfaenger@example.com>"), "To header: {raw_str}");
        assert!(raw_str.contains("Subject: Betreff"), "Subject header: {raw_str}");
        assert!(raw_str.contains("Hallo Welt"), "Body: {raw_str}");
    }

    #[test]
    fn test_build_message_from_config_invalid_sender() {
        let mut config = test_config();
        config.sender_email = "invalid-email".into();
        let result = build_message_from_config(
            &config,
            vec![("e@e.com", "")],
            vec![],
            vec![],
            "S",
            "B",
            None,
            None,
            None,
            &[],
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_build_message_from_config_invalid_recipient() {
        let config = test_config();
        let result = build_message_from_config(
            &config,
            vec![("not-an-email", "")],
            vec![],
            vec![],
            "S",
            "B",
            None,
            None,
            None,
            &[],
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_build_message_from_config_with_html() {
        let config = test_config();
        let msg = build_message_from_config(
            &config,
            vec![("e@e.com", "")],
            vec![],
            vec![],
            "HTML Test",
            "Plain text",
            Some("<h1>HTML content</h1>"),
            None,
            None,
            &[],
        )
        .unwrap();

        let raw = msg.formatted();
        let raw_str = String::from_utf8_lossy(&raw);

        assert!(raw_str.contains("Content-Type: multipart/alternative"),
            "Expected multipart/alternative, got: {}", raw_str);
        assert!(raw_str.contains("Plain text"), "Plain body missing");
        assert!(raw_str.contains("<h1>HTML content</h1>"), "HTML body missing");
    }

    #[test]
    fn test_build_message_from_config_with_cc_bcc() {
        let config = test_config();
        let msg = build_message_from_config(
            &config,
            vec![("to@e.com", "An")],
            vec![("cc@e.com", "Cc")],
            vec![("bcc@e.com", "Bcc")],
            "CC/BCC Test",
            "Body",
            None,
            None,
            None,
            &[],
        )
        .unwrap();

        let raw = msg.formatted();
        let raw_str = String::from_utf8_lossy(&raw);
        assert!(raw_str.contains("To: An <to@e.com>"), "To missing");
        assert!(raw_str.contains("Cc: Cc <cc@e.com>"), "Cc missing");
        // Bcc is intentionally omitted from the formatted message per RFC 5322
    }

    #[test]
    fn test_build_message_from_config_with_attachment() {
        let config = test_config();
        let attachments = vec![EmailAttachment {
            filename: "test-datei.pdf".to_string(),
            content: base64::engine::general_purpose::STANDARD.encode("test content"),
            content_type: "application/pdf".to_string(),
            size: 12,
        }];

        let msg = build_message_from_config(
            &config,
            vec![("e@e.com", "")],
            vec![],
            vec![],
            "Anhang Test",
            "Body",
            None,
            None,
            None,
            &attachments,
        )
        .unwrap();

        let raw = msg.formatted();
        let raw_str = String::from_utf8_lossy(&raw);

        // Prüfe, dass der Dateiname korrekt im Header steht
        assert!(raw_str.contains("test-datei.pdf"), 
            "Dateiname 'test-datei.pdf' nicht gefunden in: {}", raw_str);
        assert!(raw_str.contains("application/pdf"), 
            "Content-Type 'application/pdf' nicht gefunden in: {}", raw_str);
    }

    #[test]
    fn test_tls_with_ip_address_rejected() {
        let config = SmtpConfig {
            host: "192.168.1.1".into(),
            port: 587,
            username: "user".into(),
            password: "pass".into(),
            use_tls: true,
            sender_name: "Tester".into(),
            sender_email: "tester@example.com".into(),
        };
        match SmtpClient::new(config) {
            Ok(_) => panic!("TLS mit IP-Adresse muss abgelehnt werden"),
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains("DNS-Namen"), "Error sollte DNS-Namen erwähnen: {}", msg);
            }
        }
    }

}
