use once_cell::sync::Lazy;
use regex::Regex;

#[derive(Debug, Clone, Default)]
pub struct ToneSignals {
    pub formality: f32,
    pub friendliness: f32,
    pub address_mode: String,
    pub salutation: String,
    pub closing: String,
    pub pronoun: String,
    #[allow(dead_code)]
    pub has_emojis: bool,
    pub confidence: f32,
    pub emotion: String,
}

pub fn analyze_mail(text: &str) -> ToneSignals {
    let salutation = detect_salutation(text);
    let closing = detect_closing(text);
    let pronoun = detect_pronoun(text);
    let has_emojis = contains_emoji(text);
    let emotion = detect_emotion(text, &pronoun);

    let formality = detect_formality(text, &salutation, &closing, &pronoun);
    let friendliness = detect_friendliness(text, &closing);
    let address_mode = detect_address_mode(text);
    let confidence = calculate_confidence(text, &salutation, &closing, &pronoun, &address_mode);

    ToneSignals {
        formality,
        friendliness,
        address_mode,
        salutation,
        closing,
        pronoun,
        has_emojis,
        emotion,
        confidence,
    }
}

fn detect_formality(text: &str, salutation: &str, closing: &str, pronoun: &str) -> f32 {
    static FORMAL_SALUTATIONS: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)^(Sehr\s+geehrte[rn]|Guten\s+Tag\s+Herr|Guten\s+Tag\s+Frau|Werter|Werte)").expect("statische Regex")
    });
    static INFORMAL_SALUTATIONS: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)^(Hallo|Hey|Hi|Moin|Servus)").expect("statische Regex")
    });
    static FORMAL_CLOSINGS: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)(Mit\s+freundlichen\s+Gr\u{00fc}\u{00df}en|Hochachtungsvoll|Mit\s+besten\s+Gr\u{00fc}\u{00df}en|Ihr\s+)").expect("statische Regex")
    });
    static INFORMAL_CLOSINGS: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)(LG|Liebe\s+Gr\u{00fc}\u{00df}e|Ciao|Bis\s+bald|Tsch\u{00fc}ss|Mach's\s+gut)").expect("statische Regex")
    });

    let mut score = 0.5f32;

    if FORMAL_SALUTATIONS.is_match(salutation) || FORMAL_SALUTATIONS.is_match(text) {
        score += 0.3;
    }
    if INFORMAL_SALUTATIONS.is_match(salutation) || INFORMAL_SALUTATIONS.is_match(text) {
        score -= 0.3;
    }
    if FORMAL_CLOSINGS.is_match(closing) || FORMAL_CLOSINGS.is_match(text) {
        score += 0.2;
    }
    if INFORMAL_CLOSINGS.is_match(closing) || INFORMAL_CLOSINGS.is_match(text) {
        score -= 0.2;
    }

    if pronoun == "Sie" {
        score += 0.15;
    }
    if pronoun == "Du" {
        score -= 0.15;
    }

    score.clamp(0.0, 1.0)
}

fn detect_friendliness(text: &str, closing: &str) -> f32 {
    static WARM_WORDS: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)(danke|liebe[rn]?|freundlich|gerne|sch\u{00f6}n|wunderbar|herzlich|froh|freue\s+mich|hoffe\s+es\s+geht|alles\s+Gute)").expect("statische Regex")
    });
    static WARM_CLOSINGS: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)(Liebe\s+Gr\u{00fc}\u{00df}e|Herzlichst|Alles\s+Liebe|Bis\s+bald|Ganz\s+lieb)").expect("statische Regex")
    });

    let mut score = 0.3f32;
    let word_count = WARM_WORDS.find_iter(text).count() as f32;
    score += (word_count * 0.1).min(0.4);

    if WARM_CLOSINGS.is_match(closing) || WARM_CLOSINGS.is_match(text) {
        score += 0.2;
    }
    if contains_emoji(text) {
        score += 0.1;
    }

    score.clamp(0.0, 1.0)
}

fn detect_address_mode(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or("");

    if first_line.contains("Herr") || first_line.contains("Frau") {
        return "last_name".into();
    }

    static HELLO_FIRST: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)^(Hallo|Hey|Hi|Liebe[rn]?)\s+(\p{L}+)").expect("statische Regex")
    });
    if let Some(caps) = HELLO_FIRST.captures(first_line) {
        let name = caps.get(2).expect("capture group 2").as_str();
        static NAME_COUNT: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\p{Lu}\p{Ll}+\b").expect("statische Regex"));
        let full_text_names: Vec<&str> = NAME_COUNT.find_iter(text).map(|m| m.as_str()).collect();
        if full_text_names.iter().filter(|n| **n == name).count() == 1 {
            return "first_name".into();
        }
        return "full_name".into();
    }

    "unknown".into()
}

fn detect_salutation(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        return "none".into();
    }

    let lower = first_line.to_lowercase();
    if lower.starts_with("sehr geehrte") || lower.starts_with("sehr geehrter") {
        "Sehr geehrte/r".into()
    } else if lower.starts_with("guten tag") {
        "Guten Tag".into()
    } else if lower.starts_with("hallo") || lower.starts_with("hey") || lower.starts_with("hi") {
        "Hallo".into()
    } else if lower.starts_with("liebe") || lower.starts_with("lieber") {
        "Liebe/r".into()
    } else {
        first_line.to_string()
    }
}

fn detect_closing(text: &str) -> String {
    for line in text.lines().rev().take(5) {
        let trimmed = line.trim().to_lowercase();
        if trimmed.contains("mit freundlichen grüßen") || trimmed.contains("mit besten grüßen") {
            return "Mit freundlichen Grüßen".into();
        }
        if trimmed.contains("hochachtungsvoll") {
            return "Hochachtungsvoll".into();
        }
        if trimmed == "lg" || trimmed.starts_with("lg,") || trimmed.contains("liebe grüße") {
            return "LG".into();
        }
        if trimmed.starts_with("viele") && trimmed.contains("grüße") {
            return "Viele Grüße".into();
        }
        if trimmed.starts_with("ciao") || trimmed.starts_with("tschüss") {
            return "Ciao".into();
        }
    }
    "unknown".into()
}

fn detect_pronoun(text: &str) -> String {
    static SIE_COUNT: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"\b(Sie|Ihnen|Ihr|Ihre|Ihrem|Ihren)\b").expect("statische Regex")
    });
    static DU_COUNT: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)\b(Du|Dein|Dir|Dich|Deine|Deinem|Deinen)\b").expect("statische Regex")
    });

    let sie = SIE_COUNT.find_iter(text).count();
    let du = DU_COUNT.find_iter(text).count();

    if sie > du && sie > 0 {
        "Sie".into()
    } else if du > sie && du > 0 {
        "Du".into()
    } else {
        "unknown".into()
    }
}

fn detect_emotion(text: &str, _pronoun: &str) -> String {
    let lower = text.to_lowercase();

    // --- Anger signals ---
    static ANGER_WORDS: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)(inakzeptabel|unakzeptabel|unverzeihlich|empörend|skandalös|furchtbar|enttäuschend|unfassbar|widerspenstig|grotesk|absurd|unverschämt|arrogant|respektlos)").expect("statische Regex")
    });
    let anger_words = ANGER_WORDS.find_iter(&lower).count();

    // ALL CAPS detection: count lines that are mostly uppercase
    let caps_lines = text.lines().filter(|l| {
        let (total, upper) = l.chars().fold((0u32, 0u32), |(t, u), c| {
            if c.is_alphabetic() {
                (t + 1, u + if c.is_uppercase() { 1 } else { 0 })
            } else {
                (t, u)
            }
        });
        total > 3 && upper as f32 / total as f32 > 0.8
    }).count();

    // Exclamation marks
    let exclamations = text.chars().filter(|&c| c == '!').count();

    let anger_score = (anger_words as f32) * 2.0 + (caps_lines as f32) * 1.5 + (exclamations as f32) * 0.3;

    // --- Urgency signals ---
    static URGENT_WORDS: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)(sofort|dringend|asap|unverzüglich|umgehend|bitte\s+sofort|frist|deadline|bis\s+(heute|morgen|diesen\s+woche|diesen\s+monat)|spätestens|dringend|akut|kritisch|notfall|unmittelbar|sofortige\s+aktion)").expect("statische Regex")
    });
    let urgent_words = URGENT_WORDS.find_iter(&lower).count();

    // Question marks (often indicate urgency in context)
    let questions = text.chars().filter(|&c| c == '?').count();

    let urgency_score = (urgent_words as f32) * 2.0 + (questions as f32) * 0.2;

    // --- Friendly signals (reuse warm word detection) ---
    static WARM_WORDS: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)(danke|liebe|freundlich|gerne|schön|wunderbar|herzlich|froh|freu|lieb|nett|tolle|super|klasse|wunderbar|herrlich|genial|fantastisch|perfekt|superb)").expect("statische Regex")
    });
    let warm_count = WARM_WORDS.find_iter(&lower).count();

    let friendly_score = (warm_count as f32) * 0.5 + if contains_emoji(text) { 1.0 } else { 0.0 };

    // --- Decision: highest score wins, threshold 2.0 ---
    if anger_score >= 2.0 && anger_score > urgency_score && anger_score > friendly_score {
        "verärgert".into()
    } else if urgency_score >= 2.0 && urgency_score > anger_score && urgency_score > friendly_score {
        "dringend".into()
    } else if friendly_score >= 2.0 && friendly_score > anger_score && friendly_score > urgency_score {
        "freundlich".into()
    } else {
        "neutral".into()
    }
}

#[inline]
fn contains_emoji(text: &str) -> bool {
    static EMOJI: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"[\u{1F600}-\u{1F64F}\u{1F300}-\u{1F5FF}\u{1F680}-\u{1F6FF}\u{1F1E0}-\u{1F1FF}\u{2600}-\u{26FF}\u{2700}-\u{27BF}]").expect("statische Regex")
    });
    EMOJI.is_match(text)
}

fn calculate_confidence(
    _text: &str,
    salutation: &str,
    closing: &str,
    pronoun: &str,
    address_mode: &str,
) -> f32 {
    let signals = [
        salutation != "none",
        closing != "unknown",
        pronoun != "unknown",
        address_mode != "unknown",
    ];
    let found = signals.iter().filter(|&&s| s).count() as f32;
    (found / signals.len() as f32).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── analyze_mail integration tests ──────────────────────────────

    #[test]
    fn test_analyze_mail_formal() {
        let text = "Sehr geehrter Herr Müller,\n\nvielen Dank für Ihre Rückmeldung.\n\nMit freundlichen Grüßen\nMax Mustermann";
        let result = analyze_mail(text);
        assert!(result.formality > 0.5, "formal letter should have high formality, got {}", result.formality);
        assert_eq!(result.salutation, "Sehr geehrte/r");
        assert_eq!(result.closing, "Mit freundlichen Grüßen");
        assert_eq!(result.pronoun, "Sie");
        assert_eq!(result.emotion, "neutral");
    }

    #[test]
    fn test_analyze_mail_informal() {
        let text = "Hey Lisa,\n\nvoll cool, dass wir uns morgen treffen! 😊\n\nLG\nMax";
        let result = analyze_mail(text);
        assert!(result.formality < 0.5, "informal chat should have low formality, got {}", result.formality);
        assert!(result.has_emojis, "should detect emoji");
        assert_eq!(result.closing, "LG");
        assert_eq!(result.emotion, "neutral");
    }

    #[test]
    fn test_analyze_mail_mixed() {
        let text = "Hallo Herr Schmidt,\nich wollte nur kurz nachfragen, ob alles klar ist.\n\nViele Grüße\nMax";
        let result = analyze_mail(text);
        // "Hallo" informal salutation (-0.3), no formal closing, no Sie pronoun
        // Formality = 0.5 - 0.3 = 0.2 (informal lean from hallo, but mixed with Herr in body)
        assert!(result.formality < 0.5,
            "mixed informal-leaning tone should be below 0.5, got {}", result.formality);
        assert_eq!(result.salutation, "Hallo");
        assert_eq!(result.closing, "Viele Grüße");
        assert_eq!(result.emotion, "neutral");
    }

    #[test]
    fn test_analyze_mail_empty() {
        let result = analyze_mail("");
        assert_eq!(result.formality, 0.5);
        assert_eq!(result.friendliness, 0.3);
        assert_eq!(result.salutation, "none");
        assert_eq!(result.closing, "unknown");
        assert_eq!(result.pronoun, "unknown");
        assert_eq!(result.address_mode, "unknown");
        assert!(!result.has_emojis);
        assert_eq!(result.emotion, "neutral");
    }

    #[test]
    fn test_analyze_mail_all_caps() {
        let text = "SEHR GEEHRTE DAMEN UND HERREN,\n\nWICHTIGE MITTEILUNG.\n\nMIT FREUNDLICHEN GRÜßEN\nDER VORSTAND";
        let result = analyze_mail(text);
        // The regex is case-insensitive (?i), so formal patterns should still match
        assert_eq!(result.salutation, "Sehr geehrte/r");
        // "Mit freundlichen Grüßen" - the regex is case-insensitive but uses ü/ß unicode
        // The text has "GRÜßEN" which should match the case-insensitive regex
        assert_eq!(result.closing, "Mit freundlichen Grüßen");
        assert!(result.formality > 0.5, "all-caps formal should still detect high formality, got {}", result.formality);
        assert_eq!(result.emotion, "verärgert", "all-caps text should be detected as angry");
    }

    #[test]
    fn test_analyze_mail_unicode_german() {
        let text = "Liebe Genossinnen und Genossen,\n\nwir möchten Ihnen herzlich danken.\n\nMit solidarischen Grüßen\nDie Partei";
        let result = analyze_mail(text);
        assert_eq!(result.salutation, "Liebe/r");
        assert!(result.friendliness >= 0.3);
        assert_eq!(result.emotion, "neutral");
    }

    // ── detect_formality tests ──────────────────────────────────────

    #[test]
    fn test_detect_formality_max() {
        let text = "Sehr geehrter Herr Professor,\n\nMit freundlichen Grüßen";
        let score = detect_formality(text, "Sehr geehrte/r", "Mit freundlichen Grüßen", "Sie");
        assert!((score - 1.0).abs() < f32::EPSILON || score > 0.95,
            "all formal signals should yield near-1.0, got {}", score);
    }

    #[test]
    fn test_detect_formality_min() {
        let text = "Hey,\n\nLG";
        let score = detect_formality(text, "Hallo", "LG", "Du");
        assert!(score < 0.3, "all informal signals should yield low formality, got {}", score);
    }

    #[test]
    fn test_detect_formality_neutral() {
        let text = "Das Wetter ist heute schön.";
        let score = detect_formality(text, "unknown", "unknown", "unknown");
        assert!((score - 0.5).abs() < 0.01, "neutral should be ~0.5, got {}", score);
    }

    #[test]
    fn test_detect_formality_salutation_match() {
        // The FORMAL_SALUTATIONS regex expects "geehrter" or "geehrten" (ending in r/n)
        let score = detect_formality("Sehr geehrter Herr", "Sehr geehrte/r", "", "");
        assert!(score > 0.5, "formal salutation 'Sehr geehrter' should boost formality, got {}", score);
    }

    #[test]
    fn test_detect_formality_closing_only() {
        let score = detect_formality("Mit freundlichen Grüßen", "", "Mit freundlichen Grüßen", "");
        assert!(score > 0.5, "formal closing alone should boost formality");
    }

    #[test]
    fn test_detect_formality_cancel_out() {
        let score = detect_formality("Hallo,\n\nMit freundlichen Grüßen", "Hallo", "Mit freundlichen Grüßen", "unknown");
        // informal salutation -0.3, formal closing +0.2 => 0.5 - 0.1 = 0.4
        assert!((score - 0.4).abs() < 0.01, "mixed signals should cancel partially, got {}", score);
    }

    #[test]
    fn test_detect_formality_sie_pronoun() {
        let score = detect_formality("", "", "", "Sie");
        assert!((score - 0.65).abs() < 0.01, "Sie pronoun alone should give 0.65, got {}", score);
    }

    #[test]
    fn test_detect_formality_du_pronoun() {
        let score = detect_formality("", "", "", "Du");
        assert!((score - 0.35).abs() < 0.01, "Du pronoun alone should give 0.35, got {}", score);
    }

    #[test]
    fn test_detect_formality_clamp_lower() {
        // Multiple informal signals that would push below 0.0
        let text = "Hey Ho!\n\nLG Tschüss Mach's gut Ciao";
        let score = detect_formality(text, "Hallo", "LG", "Du");
        assert!(score >= 0.0, "formality should be clamped at 0.0, got {}", score);
    }

    // ── detect_friendliness tests ───────────────────────────────────

    #[test]
    fn test_detect_friendliness_warm_words() {
        let text = "Ich danke Ihnen herzlich. Es freut mich sehr!";
        let score = detect_friendliness(text, "");
        assert!(score > 0.3, "warm words should boost friendliness");
    }

    #[test]
    fn test_detect_friendliness_cold() {
        let text = "Hiermit kündige ich fristlos.";
        let score = detect_friendliness(text, "");
        assert!((score - 0.3).abs() < 0.01, "no warm words should give baseline 0.3, got {}", score);
    }

    #[test]
    fn test_detect_friendliness_warm_closing() {
        let score = detect_friendliness("", "Liebe Grüße");
        assert!(score > 0.4, "warm closing should boost friendliness");
    }

    #[test]
    fn test_detect_friendliness_emoji() {
        let text = "Danke 😊";
        let score = detect_friendliness(text, "");
        assert!(score > 0.3, "emoji should boost friendliness");
    }

    #[test]
    fn test_detect_friendliness_max_warmth() {
        let text = "danke liebe freundlich gerne schön wunderbar herzlich alles Gute";
        let score = detect_friendliness(text, "Liebe Grüße");
        // 7 warm words * 0.1 = 0.7, capped at +0.4 => 0.3 + 0.4 = 0.7, +0.2 for warm closing = 0.9
        assert!(score >= 0.7, "many warm words + warm closing should give high friendliness, got {}", score);
    }

    #[test]
    fn test_detect_friendliness_unicode_warm() {
        let text = "schöne Grüße aus München! Alles Gute!";
        let score = detect_friendliness(text, "");
        assert!(score > 0.4, "unicode warm words should be detected, got {}", score);
    }

    // ── detect_address_mode tests ───────────────────────────────────

    #[test]
    fn test_detect_address_mode_last_name_herr() {
        let text = "Sehr geehrter Herr Meier,\n\n";
        assert_eq!(detect_address_mode(text), "last_name");
    }

    #[test]
    fn test_detect_address_mode_last_name_frau() {
        let text = "Sehr geehrte Frau Schmidt,\n\n";
        assert_eq!(detect_address_mode(text), "last_name");
    }

    #[test]
    fn test_detect_address_mode_first_name() {
        let text = "Hallo Thomas,\n\nwie geht es Dir?";
        assert_eq!(detect_address_mode(text), "first_name");
    }

    #[test]
    fn test_detect_address_mode_hey_first_name() {
        let text = "Hey Lisa!\n\nwie geht's?";
        assert_eq!(detect_address_mode(text), "first_name");
    }

    #[test]
    fn test_detect_address_mode_full_name() {
        let text = "Hallo Thomas,\n\nkannst Du mir bitte helfen, Thomas?";
        assert_eq!(detect_address_mode(text), "full_name");
    }

    #[test]
    fn test_detect_address_mode_empty() {
        assert_eq!(detect_address_mode(""), "unknown");
    }

    #[test]
    fn test_detect_address_mode_no_salutation() {
        let text = "Das ist ein einfacher Satz ohne Anrede.";
        assert_eq!(detect_address_mode(text), "unknown");
    }

    // ── detect_salutation tests ─────────────────────────────────────

    #[test]
    fn test_detect_salutation_sehr_geehrte() {
        assert_eq!(detect_salutation("Sehr geehrte Damen und Herren"), "Sehr geehrte/r");
    }

    #[test]
    fn test_detect_salutation_sehr_geehrter() {
        assert_eq!(detect_salutation("Sehr geehrter Herr Professor"), "Sehr geehrte/r");
    }

    #[test]
    fn test_detect_salutation_guten_tag() {
        assert_eq!(detect_salutation("Guten Tag Herr Meier"), "Guten Tag");
    }

    #[test]
    fn test_detect_salutation_hallo() {
        assert_eq!(detect_salutation("Hallo Welt"), "Hallo");
    }

    #[test]
    fn test_detect_salutation_hey() {
        assert_eq!(detect_salutation("Hey Du!"), "Hallo");
    }

    #[test]
    fn test_detect_salutation_hi() {
        assert_eq!(detect_salutation("Hi there"), "Hallo");
    }

    #[test]
    fn test_detect_salutation_liebe() {
        assert_eq!(detect_salutation("Liebe Kollegen"), "Liebe/r");
    }

    #[test]
    fn test_detect_salutation_lieber() {
        assert_eq!(detect_salutation("Lieber Max"), "Liebe/r");
    }

    #[test]
    fn test_detect_salutation_empty() {
        assert_eq!(detect_salutation(""), "none");
    }

    #[test]
    fn test_detect_salutation_whitespace_only() {
        assert_eq!(detect_salutation("  \n  "), "none");
    }

    #[test]
    fn test_detect_salutation_unknown() {
        assert_eq!(detect_salutation("Betreff: Urlaubsanspruch"), "Betreff: Urlaubsanspruch");
    }

    #[test]
    fn test_detect_salutation_multiline_first_line() {
        assert_eq!(detect_salutation("Hallo Anna\n\nwie geht es Dir?"), "Hallo");
    }

    // ── detect_closing tests ────────────────────────────────────────

    #[test]
    fn test_detect_closing_mit_freundlichen() {
        let text = "Blabla\n\nMit freundlichen Grüßen\nMax Mustermann";
        assert_eq!(detect_closing(text), "Mit freundlichen Grüßen");
    }

    #[test]
    fn test_detect_closing_mit_besten() {
        let text = "Blabla\n\nMit besten Grüßen\nErika Muster";
        assert_eq!(detect_closing(text), "Mit freundlichen Grüßen");
    }

    #[test]
    fn test_detect_closing_hochachtungsvoll() {
        let text = "Text\n\nHochachtungsvoll\nDr. Meier";
        assert_eq!(detect_closing(text), "Hochachtungsvoll");
    }

    #[test]
    fn test_detect_closing_lg() {
        let text = "Bis morgen!\n\nLG\nMax";
        assert_eq!(detect_closing(text), "LG");
    }

    #[test]
    fn test_detect_closing_lg_with_comma() {
        let text = "Tschau!\n\nLG, Max";
        assert_eq!(detect_closing(text), "LG");
    }

    #[test]
    fn test_detect_closing_liebe_gruesse() {
        let text = "Alles Gute!\n\nLiebe Grüße\nAnna";
        assert_eq!(detect_closing(text), "LG");
    }

    #[test]
    fn test_detect_closing_viele_gruesse() {
        let text = "Bis bald!\n\nViele Grüße\nPeter";
        assert_eq!(detect_closing(text), "Viele Grüße");
    }

    #[test]
    fn test_detect_closing_ciao() {
        let text = "Tschau!\n\nCiao\nGiovanni";
        assert_eq!(detect_closing(text), "Ciao");
    }

    #[test]
    fn test_detect_closing_tschüss() {
        let text = "War schön!\n\nTschüss\nSven";
        assert_eq!(detect_closing(text), "Ciao");
    }

    #[test]
    fn test_detect_closing_unknown() {
        let text = "Das ist ein normaler Text ohne Grußformel am Ende.";
        assert_eq!(detect_closing(text), "unknown");
    }

    #[test]
    fn test_detect_closing_empty() {
        assert_eq!(detect_closing(""), "unknown");
    }

    #[test]
    fn test_detect_closing_only_last_five_lines() {
        // Closing detection only checks last 5 lines; closing beyond that is not found
        let text = "Mit freundlichen Grüßen\n\n\n\n\n\nextra line";
        assert_eq!(detect_closing(text), "unknown",
            "closing in 6th-to-last line should not be detected");
    }

    // ── detect_pronoun tests ────────────────────────────────────────

    #[test]
    fn test_detect_pronoun_sie() {
        let text = "Wir möchten Ihnen mitteilen, dass Ihre Anfrage bearbeitet wurde.";
        assert_eq!(detect_pronoun(text), "Sie");
    }

    #[test]
    fn test_detect_pronoun_du() {
        let text = "Kannst Du mir bitte Deine Unterlagen schicken?";
        assert_eq!(detect_pronoun(text), "Du");
    }

    #[test]
    fn test_detect_pronoun_sie_dominant() {
        let text = "Sehr geehrte Damen und Herren, wir möchten Ihnen für Ihre Geduld danken.";
        assert_eq!(detect_pronoun(text), "Sie");
    }

    #[test]
    fn test_detect_pronoun_du_dominant() {
        let text = "Hey, kannst Du mir bitte Dein Auto leihen? Ich gebe Dir Bescheid.";
        assert_eq!(detect_pronoun(text), "Du");
    }

    #[test]
    fn test_detect_pronoun_none() {
        let text = "Das Wetter ist heute schön.";
        assert_eq!(detect_pronoun(text), "unknown");
    }

    #[test]
    fn test_detect_pronoun_empty() {
        assert_eq!(detect_pronoun(""), "unknown");
    }

    #[test]
    fn test_detect_pronoun_equal_counts() {
        // When equal (1 each), neither > the other, so returns "unknown"
        let text = "Ihnen und Dir";
        assert_eq!(detect_pronoun(text), "unknown");
    }

    // ── contains_emoji tests ────────────────────────────────────────

    #[test]
    fn test_contains_emoji_true() {
        assert!(contains_emoji("Hallo 😊"));
    }

    #[test]
    fn test_contains_emoji_false() {
        assert!(!contains_emoji("Hallo Welt"));
    }

    #[test]
    fn test_contains_emoji_empty() {
        assert!(!contains_emoji(""));
    }

    #[test]
    fn test_contains_emoji_various() {
        assert!(contains_emoji("🔥🚀🌟"), "fire, rocket, star emojis should match");
    }

    #[test]
    fn test_contains_emoji_unicode_symbols() {
        // ☀ (U+2600) falls in the range \u{2600}-\u{26FF}
        assert!(contains_emoji("☀️ sonnig"), "sun symbol should match");
    }

    #[test]
    fn test_contains_emoji_skin_tone() {
        assert!(contains_emoji("👋🏽"), "wave with skin tone should match");
    }

    // ── calculate_confidence tests ──────────────────────────────────

    #[test]
    fn test_calculate_confidence_all_signals() {
        let c = calculate_confidence("some text", "Sehr geehrte/r", "Mit freundlichen Grüßen", "Sie", "last_name");
        assert!((c - 1.0).abs() < f32::EPSILON, "all signals should give 1.0, got {}", c);
    }

    #[test]
    fn test_calculate_confidence_no_signals() {
        let c = calculate_confidence("", "none", "unknown", "unknown", "unknown");
        assert!((c - 0.0).abs() < f32::EPSILON, "no signals should give 0.0, got {}", c);
    }

    #[test]
    fn test_calculate_confidence_partial() {
        let c = calculate_confidence("", "Sehr geehrte/r", "unknown", "unknown", "unknown");
        assert!((c - 0.25).abs() < 0.01, "one of four signals should give 0.25, got {}", c);
    }

    // ── detect_emotion tests ────────────────────────────────────────

    #[test]
    fn test_detect_emotion_angry() {
        let text = "Das ist ABSOLUT INAKZEPTABEL! So eine Frechheit! UNVERSCHÄMT!";
        let result = detect_emotion(text, "Sie");
        assert_eq!(result, "verärgert");
    }

    #[test]
    fn test_detect_emotion_urgent() {
        let text = "Bitte antworten Sie SOFORT! Das ist dringend! Die Deadline ist bis heute!";
        let result = detect_emotion(text, "Sie");
        assert_eq!(result, "dringend");
    }

    #[test]
    fn test_detect_emotion_friendly() {
        let text = "Liebes Team, vielen herzlichen Dank für die tolle Zusammenarbeit! Ihr seid super klasse! 😊";
        let result = detect_emotion(text, "Du");
        assert_eq!(result, "freundlich");
    }

    #[test]
    fn test_detect_emotion_neutral() {
        let text = "Die Sitzung findet am Montag um 14 Uhr statt. Bitte bringen Sie die Unterlagen mit.";
        let result = detect_emotion(text, "Sie");
        assert_eq!(result, "neutral");
    }

    #[test]
    fn test_detect_emotion_angry_wins() {
        // Both anger and urgency signals present; anger should win due to higher score
        let text = "Das ist ABSOLUT INAKZEPTABEL! Bitte SOFORT beheben! Dringend! UNVERSCHÄMT!";
        let result = detect_emotion(text, "Sie");
        assert_eq!(result, "verärgert");
    }
}
