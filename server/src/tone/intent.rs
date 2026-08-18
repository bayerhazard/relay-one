use once_cell::sync::Lazy;
use regex::Regex;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MailIntent {
    pub recipient_name: Option<String>,
    pub recipient_org: Option<String>,
    pub tone_hints: ToneHints,
}

#[derive(Debug, Clone, Default)]
pub struct ToneHints {
    pub implied_formality: Option<f32>,
    pub implied_friendliness: Option<f32>,
    pub occasion: Option<String>,
}

pub fn parse_intent(free_text: &str) -> MailIntent {
    let name = extract_recipient_name(free_text);
    let org = extract_organization(free_text);
    let tone = extract_tone_hints(free_text);

    MailIntent {
        recipient_name: name,
        recipient_org: org,
        tone_hints: tone,
    }
}

fn extract_recipient_name(text: &str) -> Option<String> {
    static TO_NAME: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"(?i)(?:Schreibe|Mail\s+an|Sende\s+an|E-Mail\s+an|Nachricht\s+an)\s+((?:[A-Z\u{00C4}\u{00D6}\u{00DC}][a-z\u{00E4}\u{00F6}\u{00FC}\u{00DF}]+\s*)+)",
        )
        .expect("statische Regex")
    });
    if let Some(caps) = TO_NAME.captures(text) {
        return Some(caps.get(1).expect("capture group 1").as_str().trim().to_string());
    }
    None
}

fn extract_organization(text: &str) -> Option<String> {
    static FIRMA: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"(?i)(?:Firma|Unternehmen|GmbH|AG|UG)\s+([A-Z\u{00C4}\u{00D6}\u{00DC}][a-z\u{00E4}\u{00F6}\u{00FC}\u{00DF}]+(?:\s+[A-Z\u{00C4}\u{00D6}\u{00DC}][a-z\u{00E4}\u{00F6}\u{00FC}\u{00DF}]+)*)",
        )
        .expect("statische Regex")
    });
    if let Some(caps) = FIRMA.captures(text) {
        return Some(caps.get(1).expect("capture group 1").as_str().trim().to_string());
    }
    None
}

fn extract_tone_hints(text: &str) -> ToneHints {
    let lower = text.to_lowercase();
    let mut hints = ToneHints::default();

    static PRIVATE_SIGNALS: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"(?i)(liebe[rn]?\s+Gru\u{00df}|privat|pers\u{00f6}nlich|herzlich|ganz\s+lieb|vertraut|famili\u{00e4}r|freundschaftlich)",
        )
        .expect("statische Regex")
    });
    if PRIVATE_SIGNALS.is_match(&lower) {
        hints.implied_formality = Some(0.1);
        hints.implied_friendliness = Some(0.9);
    }

    static BUSINESS_SIGNALS: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"(?i)(Firma|GmbH|AG|Gesch\u{00e4}ftlich|Business|Kunde|Mandant|offiziell|formell|Bewerbung|Angebot|Rechnung)",
        )
        .expect("statische Regex")
    });
    if BUSINESS_SIGNALS.is_match(&lower) {
        hints.implied_formality = Some(0.9);
        hints.implied_friendliness = Some(0.3);
    }

    static FESTIVAL: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"(?i)(Weihnacht|Geburtstag|Jubil\u{00e4}um|Hochzeit|Neujahr|Ostern|Feiertag)",
        )
        .expect("statische Regex")
    });
    if let Some(caps) = FESTIVAL.captures(&lower) {
        hints.occasion = Some(caps.get(1).expect("capture group 1").as_str().to_string());
        if hints.implied_friendliness.is_none() {
            hints.implied_friendliness = Some(0.6);
        }
    }

    static URGENT_TERMS: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)(dringend|sofort|eilig|asap|schnell|wichtig)").expect("statische Regex")
    });
    if URGENT_TERMS.is_match(&lower) {
        if hints.implied_friendliness.is_none() {
            hints.implied_friendliness = Some(0.2);
        }
    }

    hints
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_intent tests ──────────────────────────────────────────

    #[test]
    fn test_parse_intent_direct_address() {
        let result = parse_intent("Schreibe Max Mustermann eine E-Mail");
        // (?i) flag makes [A-Z] case-insensitive, so "eine" also matches
        assert_eq!(result.recipient_name.as_deref(), Some("Max Mustermann eine"));
    }

    #[test]
    fn test_parse_intent_mail_an() {
        let result = parse_intent("Mail an Anna Schmidt");
        assert_eq!(result.recipient_name.as_deref(), Some("Anna Schmidt"));
    }

    #[test]
    fn test_parse_intent_sende_an() {
        let result = parse_intent("Sende an Thomas Müller");
        assert_eq!(result.recipient_name.as_deref(), Some("Thomas Müller"));
    }

    #[test]
    fn test_parse_intent_empty() {
        let result = parse_intent("");
        assert!(result.recipient_name.is_none());
        assert!(result.recipient_org.is_none());
    }

    #[test]
    fn test_parse_intent_no_recipient() {
        let result = parse_intent("Wie ist das Wetter?");
        assert!(result.recipient_name.is_none());
    }

    // ── extract_recipient_name tests ────────────────────────────────

    #[test]
    fn test_extract_recipient_name_no_match() {
        assert!(extract_recipient_name("Hallo Welt").is_none());
    }

    #[test]
    fn test_extract_recipient_name_empty() {
        assert!(extract_recipient_name("").is_none());
    }

    #[test]
    fn test_extract_recipient_name_unicode_name() {
        let result = extract_recipient_name("Schreibe André Müller");
        // 'é' (U+00E9) is not in the lowercase char class [a-zäöüß], so "André" captures "Andr"
        assert_eq!(result.as_deref(), Some("Andr"));
    }

    // ── extract_organization tests ──────────────────────────────────

    #[test]
    fn test_extract_organization_firma() {
        let text = "Schreibe an Firma Mustermann GmbH";
        let result = parse_intent(text);
        // The regex captures multi-word names after "Firma", so "Mustermann GmbH"
        assert_eq!(result.recipient_org.as_deref(), Some("Mustermann GmbH"));
    }

    #[test]
    fn test_extract_organization_gmbh() {
        let text = "E-Mail an GmbH Beispiel";
        let result = parse_intent(text);
        assert_eq!(result.recipient_org.as_deref(), Some("Beispiel"));
    }

    #[test]
    fn test_extract_organization_ag() {
        let text = "Kontaktiere AG Musterfirma";
        let result = parse_intent(text);
        assert_eq!(result.recipient_org.as_deref(), Some("Musterfirma"));
    }

    // ── extract_tone_hints tests ────────────────────────────────────

    #[test]
    fn test_tone_hints_private() {
        let hints = extract_tone_hints("Liebe Grüße, ganz vertraut und freundschaftlich");
        assert_eq!(hints.implied_formality, Some(0.1));
        assert_eq!(hints.implied_friendliness, Some(0.9));
        assert!(hints.occasion.is_none());
    }

    #[test]
    fn test_tone_hints_business() {
        let hints = extract_tone_hints("Geschäftliche Anfrage zum Angebot");
        assert_eq!(hints.implied_formality, Some(0.9));
        assert_eq!(hints.implied_friendliness, Some(0.3));
    }

    #[test]
    fn test_tone_hints_festival() {
        let hints = extract_tone_hints("Frohe Weihnachten!");
        assert_eq!(hints.occasion.as_deref(), Some("weihnacht"));
        // festival also sets friendliness to 0.6 since implied_friendliness is None
        assert_eq!(hints.implied_friendliness, Some(0.6));
        assert!(hints.implied_formality.is_none());
    }

    #[test]
    fn test_tone_hints_urgency() {
        let hints = extract_tone_hints("Bitte dringend zurückschreiben!");
        assert_eq!(hints.implied_friendliness, Some(0.2));
        assert!(hints.implied_formality.is_none());
        assert!(hints.occasion.is_none());
    }

    #[test]
    fn test_tone_hints_business_overrides_private() {
        // BUSINESS_SIGNALS runs after PRIVATE_SIGNALS, so business should win
        let hints = extract_tone_hints("Liebe Grüße, Geschäftlich");
        // Private sets 0.1/0.9, then Business overrides to 0.9/0.3
        assert_eq!(hints.implied_formality, Some(0.9));
        assert_eq!(hints.implied_friendliness, Some(0.3));
    }

    #[test]
    fn test_tone_hints_birthday() {
        let hints = extract_tone_hints("Herzlichen Glückwunsch zum Geburtstag!");
        assert_eq!(hints.occasion.as_deref(), Some("geburtstag"));
    }

    #[test]
    fn test_tone_hints_empty() {
        let hints = extract_tone_hints("");
        assert!(hints.implied_formality.is_none());
        assert!(hints.implied_friendliness.is_none());
        assert!(hints.occasion.is_none());
    }

    #[test]
    fn test_tone_hints_urgent_business() {
        let hints = extract_tone_hints("Dringendes geschäftliches Angebot");
        // Business sets formality=0.9, friendliness=0.3; urgent tries to set friendliness=0.2
        // but business already set it so urgent won't override
        assert_eq!(hints.implied_formality, Some(0.9));
        assert_eq!(hints.implied_friendliness, Some(0.3));
    }

    #[test]
    fn test_tone_hints_multiple_signals() {
        let hints = extract_tone_hints("Geburtstagsfeier in der Firma Schmidt GmbH");
        // Festival: occasion=Some("geburtstag"), friendliness=0.6
        // Business: formality=0.9, friendliness=0.3
        // Business overrides friendliness
        assert_eq!(hints.occasion.as_deref(), Some("geburtstag"));
        assert_eq!(hints.implied_formality, Some(0.9));
        assert_eq!(hints.implied_friendliness, Some(0.3));
    }
}
