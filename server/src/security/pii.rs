use regex::Regex;
use once_cell::sync::Lazy;

pub fn mask_pii(text: &str) -> String {
    let mut result = text.to_string();
    result = EMAIL_RE.replace_all(&result, "[EMAIL_REDACTED]").into_owned();
    result = PHONE_RE
        .replace_all(&result, "[PHONE_REDACTED]")
        .into_owned();
    result = CC_RE.replace_all(&result, "[CC_REDACTED]").into_owned();
    result = SSN_RE.replace_all(&result, "[SSN_REDACTED]").into_owned();
    result = IP_RE.replace_all(&result, "[IP_REDACTED]").into_owned();
    result
}

static EMAIL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").unwrap());

static PHONE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"((?:\+\d{1,3}[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4})").unwrap()
});

static CC_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b(?:\d[ -]*?){13,16}\b").unwrap()
});

static SSN_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap()
});

static IP_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").unwrap()
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mask_email() {
        let text = "Kontakt: test@example.com";
        let result = mask_pii(text);
        assert!(!result.contains("test@example.com"));
        assert!(result.contains("[EMAIL_REDACTED]"));
    }

    #[test]
    fn test_mask_phone() {
        let text = "Tel: +49 123 4567890";
        let result = mask_pii(text);
        assert!(result.contains("[PHONE_REDACTED]"));
    }

    #[test]
    fn test_mask_credit_card() {
        let text = "Karte: 4111 1111 1111 1111";
        let result = mask_pii(text);
        assert!(result.contains("[CC_REDACTED]"));
    }

    #[test]
    fn test_empty_text() {
        let result = mask_pii("");
        assert_eq!(result, "");
    }

    #[test]
    fn test_no_pii() {
        let text = "Hallo, wie geht es Ihnen? Das Wetter ist heute schön.";
        let result = mask_pii(text);
        assert_eq!(result, text);
    }

    #[test]
    fn test_unicode_no_pii() {
        let text = "日本語のメールです。ご確認ください。";
        let result = mask_pii(text);
        assert_eq!(result, text);
    }

    #[test]
    fn test_email_subdomain() {
        let text = "Kontakt: user@sub.example.co.uk";
        let result = mask_pii(text);
        assert!(!result.contains("user@sub.example.co.uk"));
        assert!(result.contains("[EMAIL_REDACTED]"));
    }

    #[test]
    fn test_email_plus_addressing() {
        let text = "Email: test+tag@example.com";
        let result = mask_pii(text);
        assert!(!result.contains("test+tag@example.com"));
        assert!(result.contains("[EMAIL_REDACTED]"));
    }

    #[test]
    fn test_email_at_start_of_text() {
        let text = "test@example.com hat Ihnen geschrieben";
        let result = mask_pii(text);
        assert!(result.contains("[EMAIL_REDACTED]"));
        assert!(result.contains("hat Ihnen geschrieben"));
    }

    #[test]
    fn test_email_at_end_of_text() {
        let text = "Kontaktieren Sie uns unter test@example.com";
        let result = mask_pii(text);
        assert!(result.contains("[EMAIL_REDACTED]"));
    }

    #[test]
    fn test_multiple_emails() {
        let text = "a@b.com und c@d.org sind beide E-Mails";
        let result = mask_pii(text);
        assert_eq!(
            result.matches("[EMAIL_REDACTED]").count(),
            2
        );
    }

    #[test]
    fn test_phone_with_dashes() {
        let text = "Tel: 123-456-7890";
        let result = mask_pii(text);
        assert!(result.contains("[PHONE_REDACTED]"));
    }

    #[test]
    fn test_phone_with_dots() {
        let text = "Tel: 123.456.7890";
        let result = mask_pii(text);
        assert!(result.contains("[PHONE_REDACTED]"));
    }

    #[test]
    fn test_phone_with_country_code() {
        let text = "Tel: +1-555-123-4567";
        let result = mask_pii(text);
        assert!(result.contains("[PHONE_REDACTED]"));
    }

    #[test]
    fn test_phone_with_parentheses() {
        let text = "Tel: (555) 123-4567";
        let result = mask_pii(text);
        assert!(result.contains("[PHONE_REDACTED]"));
    }

    #[test]
    fn test_ssn_masking() {
        let text = "SSN: 123-45-6789";
        let result = mask_pii(text);
        assert!(!result.contains("123-45-6789"));
        assert!(result.contains("[SSN_REDACTED]"));
    }

    #[test]
    fn test_ip_masking() {
        let text = "IP: 192.168.1.1";
        let result = mask_pii(text);
        assert!(!result.contains("192.168.1.1"));
        assert!(result.contains("[IP_REDACTED]"));
    }

    #[test]
    fn test_ipv4_loopback() {
        let text = "Server: 127.0.0.1";
        let result = mask_pii(text);
        assert!(result.contains("[IP_REDACTED]"));
    }

    #[test]
    fn test_multiple_pii_types() {
        let text = "Email: user@test.com, Tel: +49 123 4567890, IP: 10.0.0.1";
        let result = mask_pii(text);
        assert!(result.contains("[EMAIL_REDACTED]"));
        assert!(result.contains("[PHONE_REDACTED]"));
        assert!(result.contains("[IP_REDACTED]"));
        assert!(!result.contains("user@test.com"));
        assert!(!result.contains("+49 123 4567890"));
        assert!(!result.contains("10.0.0.1"));
    }

    #[test]
    fn test_credit_card_with_dashes() {
        let text = "Karte: 4111-1111-1111-1111";
        let result = mask_pii(text);
        assert!(result.contains("[CC_REDACTED]"));
    }

    #[test]
    fn test_credit_card_short_number_not_masked() {
        // 12 digits (too short for CC pattern which needs 13-16)
        let text = "Zahlung: 1234 5678 9012";
        let result = mask_pii(text);
        assert!(!result.contains("[CC_REDACTED]"));
    }

    #[test]
    fn test_partial_email_not_masked() {
        // Missing domain TLD
        let text = "Email: user@example";
        let result = mask_pii(text);
        assert!(!result.contains("[EMAIL_REDACTED]"));
    }

    #[test]
    fn test_partial_phone_not_masked() {
        // Too few digits
        let text = "Tel: 12345";
        let result = mask_pii(text);
        assert!(!result.contains("[PHONE_REDACTED]"));
    }

    #[test]
    fn test_whitespace_only() {
        let result = mask_pii("   \n  \t  ");
        assert_eq!(result, "   \n  \t  ");
    }

    #[test]
    fn test_special_characters_no_pii() {
        let text = "!@#$%^&*()_+-=[]{}|;':\",./<>?`~";
        let result = mask_pii(text);
        assert_eq!(result, text);
    }
}
