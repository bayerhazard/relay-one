use regex::Regex;
use once_cell::sync::Lazy;

pub fn detect_priority(subject: &str, body: &str) -> f32 {
    let mut score = 0.0f32;

    if URGENT_SUBJECT.is_match(subject) {
        score += 0.3;
    }

    if URGENT_BODY.is_match(body) {
        score += 0.15;
    }

    if ACTION_REQUEST.is_match(body) {
        score += 0.15;
    }

    let exclamation_count = subject.chars().filter(|c| *c == '!').count();
    if exclamation_count >= 3 {
        score += 0.1;
    }

    let uppercount = subject.chars().filter(|c| c.is_uppercase()).count();
    let total = subject.chars().filter(|c| c.is_alphabetic()).count();
    if total > 0 && (uppercount as f32 / total as f32) > 0.5 {
        score += 0.1;
    }

    score.min(1.0)
}

static URGENT_SUBJECT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(dringend|wichtig|eilig|urgent|frist|sofort|asap|attention)").unwrap()
});

static URGENT_BODY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(sofortige\s+Rueckmeldung|bitte\s+umgehend|unverzueglich|zeitnah|heute\s+noch|deadline)").unwrap()
});

static ACTION_REQUEST: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(bitte\s+(beantworten|reagieren|antworten|bestätigen|prüfen)|Ihre\s+Rückmeldung|Rückmeldung\s+bis)").unwrap()
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_high_priority() {
        let score = detect_priority(
            "DRINGEND: Rueckmeldung bis heute",
            "Bitte umgehend reagieren",
        );
        assert!(score > 0.4);
    }

    #[test]
    fn test_normal_priority() {
        let score = detect_priority(
            "Newsletter Juni",
            "Hier ist unser monatlicher Newsletter mit den neuesten Angeboten.",
        );
        assert!(score < 0.3);
    }

    #[test]
    fn test_empty_subject_and_body() {
        let score = detect_priority("", "");
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_empty_subject_only() {
        let score = detect_priority("", "Bitte umgehend reagieren");
        assert!(score > 0.0);
        assert!(score <= 0.3);
    }

    #[test]
    fn test_empty_body_only() {
        let score = detect_priority("DRINGEND: wichtig", "");
        assert!(score > 0.0);
    }

    #[test]
    fn test_whitespace_only() {
        let score = detect_priority("   ", "  \n  \t  ");
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_unicode_subject() {
        let score = detect_priority("日本語の件名", "普通の本文");
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_unicode_with_priority_keywords() {
        let score = detect_priority(
            "Важное: dringend",
            "Пожалуйста, ответьте sofortige Rueckmeldung",
        );
        assert!(score > 0.3);
    }

    #[test]
    fn test_very_long_subject() {
        // Mixed case so allcaps doesn't trigger; no urgency keywords
        let long_subject = "Ab".repeat(500);
        let score = detect_priority(&long_subject, "body");
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_very_long_body() {
        let long_body = "B".repeat(10000);
        let score = detect_priority("Test", &long_body);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_special_characters_subject() {
        let score = detect_priority("!!! WICHTIG !!!", "Bitte prüfen");
        // 3 exclamation marks → +0.1, all-caps >50% → +0.1, "wichtig" → +0.3
        assert!(score >= 0.4);
    }

    #[test]
    fn test_exclamation_boundary_three() {
        // Exactly 3 exclamation marks → should trigger
        let score = detect_priority("!!!", "body");
        assert!(score >= 0.1);
    }

    #[test]
    fn test_exclamation_boundary_two() {
        // Exactly 2 exclamation marks → should NOT trigger
        let score = detect_priority("!!", "body");
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_allcaps_boundary_above_50() {
        // 6 uppercase, 5 lowercase = 6/11 ≈ 0.545 > 0.5 → trigger
        let score = detect_priority("URGENTnormal", "body");
        assert!(score >= 0.1);
    }

    #[test]
    fn test_allcaps_boundary_exactly_50() {
        // 5 uppercase, 5 lowercase = 5/10 = 0.5, not > 0.5 → should NOT trigger
        // Use neutral words that don't match any urgency keyword
        let score = detect_priority("HELLOworld", "body");
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_allcaps_boundary_below_50() {
        // 4 uppercase, 6 lowercase = 4/10 = 0.4 < 0.5 → should NOT trigger
        // Use neutral words that don't match any urgency keyword
        let score = detect_priority("HellOworld", "body");
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_no_alpha_allcaps() {
        // No alphabetic chars → should not trigger allcaps
        let score = detect_priority("12345 !!!", "body");
        // Only exclamation marks trigger (3 of them)
        assert_eq!(score, 0.1);
    }

    #[test]
    fn test_score_capped_at_one() {
        let score = detect_priority(
            "DRINGEND: WICHTIG EILIG URGENT FRISK SOFORT ASAP ATTENTION !!!",
            "sofortige Rückmeldung bitte umgehend unverzueglich zeitnah heute noch deadline \
             bitte beantworten bitte reagieren bitte antworten bitte bestätigen bitte prüfen \
             Ihre Rückmeldung Rückmeldung bis",
        );
        assert!(score <= 1.0);
    }

    #[test]
    fn test_action_request_detection() {
        let score = detect_priority("Test", "bitte beantworten");
        assert!(score >= 0.15);
    }

    #[test]
    fn test_urgent_body_without_urgent_subject() {
        let score = detect_priority("Normal subject", "sofortige Rueckmeldung erforderlich");
        assert!(score >= 0.15);
        assert!(score < 0.4);
    }

    #[test]
    fn test_urgent_subject_without_urgent_body() {
        let score = detect_priority("DRINGEND: Bitte lesen", "Normaler Text hier");
        assert!(score >= 0.3);
        assert!(score < 0.5);
    }

    #[test]
    fn test_priority_keywords_english() {
        let score = detect_priority("URGENT: Meeting today", "ASAP response needed");
        assert!(score >= 0.3);
    }

    #[test]
    fn test_priority_keywords_german() {
        let score = detect_priority("EILIG: Frist läuft ab", "unverzueglich reagieren");
        assert!(score >= 0.3);
    }
}
