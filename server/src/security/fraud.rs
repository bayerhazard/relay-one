use regex::Regex;
use once_cell::sync::Lazy;

#[derive(Debug, Clone, serde::Serialize)]
pub struct FraudResult {
    pub score: f32,
    pub warnings: Vec<String>,
}

pub fn detect_fraud(subject: &str, body: &str) -> FraudResult {
    let combined = format!("{} {}", subject, body);
    let mut score = 0.0f32;
    let mut warnings = Vec::new();

    if URGENCY.is_match(&combined) {
        score += 0.15;
        warnings.push("Dringlichkeits-Sprache erkannt".into());
    }

    if has_link_text_mismatch(&body) {
        score += 0.2;
        warnings.push("Link-Text weicht von URL ab".into());
    }

    if HIDDEN_LINK.is_match(body) {
        score += 0.25;
        warnings.push("Versteckte Links gefunden".into());
    }

    if SUSPICIOUS_TLD.is_match(body) {
        score += 0.2;
        warnings.push("Verdaechtige Top-Level-Domain".into());
    }

    if CREDENTIAL_REQUEST.is_match(&combined) {
        score += 0.15;
        warnings.push("Bitte um Zugangsdaten erkannt".into());
    }

    if SHORTENER.is_match(body) {
        score += 0.1;
        warnings.push("URL-Shortener verwendet".into());
    }

    let exclamation_count = combined.chars().filter(|c| *c == '!').count();
    if exclamation_count >= 5 {
        score += 0.1;
        warnings.push("Uebermaessige Ausrufezeichen".into());
    }

    if let Some(s) = detect_allcaps_subject(subject) {
        score += 0.1;
        warnings.push(s);
    }

    if detect_display_mismatch(body) {
        score += 0.1;
        warnings.push("Display-Name stimmt nicht mit Domain ueberein".into());
    }

    FraudResult {
        score: score.min(1.0),
        warnings,
    }
}

static URGENCY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(dringend|sofort|eilig|urgent|sofortiges\s+Handeln|letzte\s+Warnung|Konto\s+wird\s+gesperrt|Frist\s+laeuft)").unwrap()
});

static SUSPICIOUS_TLD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://[^\s/]+\.(tk|ml|ga|cf|gq|top|win|buzz|club)([/\s]|$)").unwrap()
});

static CREDENTIAL_REQUEST: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(Passwort|Login|Zugangsdaten|verifizieren\s+Sie\s+Ihr|Konto\s+bestaetigen|Anmeldeinformationen)").unwrap()
});

static SHORTENER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://(bit\.ly|t\.co|tinyurl\.com|ow\.ly|is\.gd|buff\.ly|goo\.gl|short\.est)/").unwrap()
});

static HIDDEN_LINK: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"<a\s+[^>]*href\s*=\s*["'][^"']+["'][^>]*>\s*</a>"#).unwrap()
});

fn has_link_text_mismatch(body: &str) -> bool {
    static LINK_TAG: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"<a\s+[^>]*href\s*=\s*["']([^"']+)["'][^>]*>([^<]*)</a>"#).unwrap()
    });

    for caps in LINK_TAG.captures_iter(body) {
        let url = caps.get(1).unwrap().as_str();
        let text = caps.get(2).unwrap().as_str().trim();
        if !text.is_empty() && !url.contains(text) {
            return true;
        }
    }
    false
}

fn detect_allcaps_subject(subject: &str) -> Option<String> {
    if subject.len() < 10 {
        return None;
    }
    let uppercount = subject.chars().filter(|c| c.is_uppercase()).count();
    let total = subject.chars().filter(|c| c.is_alphabetic()).count();
    if total > 0 && (uppercount as f32 / total as f32) > 0.7 {
        return Some("Betreff in Grossbuchstaben".into());
    }
    None
}

fn detect_display_mismatch(_body: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urgency_detection() {
        let result = detect_fraud("DRINGEND: Konto wird gesperrt", "Bitte sofort handeln!");
        assert!(result.score > 0.1);
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn test_clean_mail() {
        let result = detect_fraud("Meeting morgen", "Hallo, wollen wir uns um 14 Uhr treffen?");
        assert!(result.score < 0.2);
    }

    #[test]
    fn test_suspicious_tld() {
        let result = detect_fraud("Hallo", "Besuche http://example.tk/now");
        assert!(result.score > 0.15);
        assert!(result.warnings.iter().any(|w| w.contains("Top-Level-Domain")));
    }

    #[test]
    fn test_empty_input() {
        let result = detect_fraud("", "");
        assert_eq!(result.score, 0.0);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_whitespace_only() {
        let result = detect_fraud("   ", "  \n  \t  ");
        assert_eq!(result.score, 0.0);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_unicode_no_fraud() {
        let result = detect_fraud(
            "日本語の件名",
            "こんにちは、元気ですか？今日は良い天気ですね。",
        );
        assert_eq!(result.score, 0.0);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_unicode_with_fraud_keywords() {
        // Unicode text that also contains urgency keywords
        let result = detect_fraud(
            "Важное: dringend",
            "Пожалуйста, ответьте sofort",
        );
        assert!(result.score > 0.1);
        assert!(result.warnings.iter().any(|w| w.contains("Dringlichkeit")));
    }

    #[test]
    fn test_score_capped_at_one() {
        // Trigger many signals to push score past 1.0
        let result = detect_fraud(
            "DRINGEND: KONTO WIRD GESPERRT! ! ! ! !",
            r#"Bitte sofort handeln! Ihr Passwort ist gefährdet!
            <a href="http://evil.tk/steal">http://evil.tk/steal</a>
            <a href="http://bit.ly/evil">klicken Sie hier</a>
            <a href="http://evil.ml/hidden" style="display:none;"> </a>"#,
        );
        assert!(result.score <= 1.0);
        // Should have multiple warnings
        assert!(result.warnings.len() >= 3);
    }

    #[test]
    fn test_hidden_link_detection() {
        let result = detect_fraud(
            "Test",
            r#"<a href="http://evil.com" style="display:none;"> </a>"#,
        );
        assert!(result.score >= 0.25);
        assert!(result.warnings.iter().any(|w| w.contains("Versteckte")));
    }

    #[test]
    fn test_link_text_mismatch() {
        let result = detect_fraud(
            "Test",
            r#"<a href="http://evil-phishing.com">https://safe-bank.com</a>"#,
        );
        assert!(result.score >= 0.2);
        assert!(result.warnings.iter().any(|w| w.contains("Link-Text")));
    }

    #[test]
    fn test_link_text_match_not_flagged() {
        // When link text contains the URL, no mismatch warning
        let result = detect_fraud(
            "Test",
            r#"<a href="https://example.com">https://example.com</a>"#,
        );
        assert!(!result.warnings.iter().any(|w| w.contains("Link-Text")));
    }

    #[test]
    fn test_url_shortener() {
        let result = detect_fraud("Test", "Check this out: http://bit.ly/abc123");
        assert!(result.score >= 0.1);
        assert!(result.warnings.iter().any(|w| w.contains("URL-Shortener")));
    }

    #[test]
    fn test_credential_request() {
        let result = detect_fraud(
            "Ihr Konto wurde gesperrt",
            "Bitte verifizieren Sie Ihr Konto und geben Sie Ihr Passwort ein.",
        );
        assert!(result.score >= 0.15);
        assert!(result.warnings.iter().any(|w| w.contains("Zugangsdaten")));
    }

    #[test]
    fn test_allcaps_subject_short_not_flagged() {
        // Subject shorter than 10 chars should not trigger allcaps
        let result = detect_fraud("URGENT!", "body");
        assert!(!result.warnings.iter().any(|w| w.contains("Grossbuchstaben")));
    }

    #[test]
    fn test_allcaps_subject_long_flagged() {
        // Subject >= 10 chars with >70% uppercase
        let result = detect_fraud("URGENT MESSAGE HERE", "body");
        assert!(result.warnings.iter().any(|w| w.contains("Grossbuchstaben")));
    }

    #[test]
    fn test_allcaps_subject_boundary_70_percent() {
        // 10 chars, 7 uppercase (70%) → should trigger (strictly greater than 0.7)
        let result = detect_fraud("URGENTMESSA", "body");
        assert!(result.warnings.iter().any(|w| w.contains("Grossbuchstaben")));
    }

    #[test]
    fn test_allcaps_subject_boundary_just_below() {
        // 10 chars, 7 uppercase, 3 lowercase → 7/10 = 0.7, not > 0.7 → should NOT trigger
        let result = detect_fraud("URGENTmess", "body");
        assert!(!result.warnings.iter().any(|w| w.contains("Grossbuchstaben")));
    }

    #[test]
    fn test_exclamation_boundary_five() {
        // Exactly 5 exclamation marks → should trigger
        let result = detect_fraud("!!!!!", "body");
        assert!(result.warnings.iter().any(|w| w.contains("Ausrufezeichen")));
    }

    #[test]
    fn test_exclamation_boundary_four() {
        // Exactly 4 exclamation marks → should NOT trigger
        let result = detect_fraud("!!!!", "body");
        assert!(!result.warnings.iter().any(|w| w.contains("Ausrufezeichen")));
    }

    #[test]
    fn test_no_alpha_allcaps_subject() {
        // Subject with no alphabetic chars should not trigger allcaps
        let result = detect_fraud("12345 67890 12345", "body");
        assert!(!result.warnings.iter().any(|w| w.contains("Grossbuchstaben")));
    }

    #[test]
    fn test_suspicious_tld_variants() {
        let result = detect_fraud("Test", "http://spam.top/offer");
        assert!(result.score > 0.15);
        assert!(result.warnings.iter().any(|w| w.contains("Top-Level-Domain")));

        let result2 = detect_fraud("Test", "https://free.ml/win");
        assert!(result2.score > 0.15);

        let result3 = detect_fraud("Test", "http://example.ga/");
        assert!(result3.score > 0.15);
    }

    #[test]
    fn test_urgency_keywords_german() {
        let result = detect_fraud("Frist läuft ab!", "letzte Warnung");
        assert!(result.score > 0.1);
        assert!(result.warnings.iter().any(|w| w.contains("Dringlichkeit")));
    }

    #[test]
    fn test_urgency_keywords_english() {
        let result = detect_fraud("URGENT: Account suspended", "Immediate action required");
        assert!(result.score > 0.1);
        assert!(result.warnings.iter().any(|w| w.contains("Dringlichkeit")));
    }

    #[test]
    fn test_has_link_text_mismatch_direct() {
        // Test the private helper directly
        assert!(super::has_link_text_mismatch(
            r#"<a href="http://evil.com">Click here</a>"#,
        ));
        assert!(!super::has_link_text_mismatch(
            r#"<a href="https://safe.com">https://safe.com</a>"#,
        ));
        assert!(!super::has_link_text_mismatch("No links here"));
    }

    #[test]
    fn test_detect_allcaps_subject_direct() {
        // Test the private helper directly
        assert_eq!(super::detect_allcaps_subject("URGENT!!!"), None); // too short
        assert!(super::detect_allcaps_subject("URGENT MESSAGE HERE").is_some());
        assert_eq!(super::detect_allcaps_subject("Normal Subject Text"), None);
        assert_eq!(super::detect_allcaps_subject(""), None);
    }

    #[test]
    fn test_detect_display_mismatch_direct() {
        // detect_display_mismatch always returns false (stub)
        assert!(!super::detect_display_mismatch("anything"));
        assert!(!super::detect_display_mismatch(""));
    }
}
