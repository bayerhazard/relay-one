/// Lightweight language detection for DE vs EN.
/// Uses character-level and word-level heuristics.
/// Returns "de" (default) or "en".
pub fn detect_language(text: &str) -> &'static str {
    if text.len() < 20 {
        return "de";
    }

    let lower = text.to_lowercase();
    let mut de_score: i32 = 0;
    let mut en_score: i32 = 0;

    for c in lower.chars() {
        match c {
            'ß' | 'ä' | 'ö' | 'ü' => de_score += 3,
            _ => {}
        }
    }

    for w in [
        "der", "die", "den", "dem", "und", "nicht", "ist", "auf", "für", "mit",
        "von", "hat", "sich", "auch", "wie", "wurde", "noch", "nach", "bei",
        "einer", "dass", "man", "kann", "mehr", "sehr", "alle", "mich", "mir",
        "uns", "euch", "ihnen", "bitte", "vielen", "dank", "gruß", "liebe",
        "hallo", "hoffentlich", "viele", "dieser", "diese", "dieses",
    ] {
        if lower.contains(&format!(" {} ", w)) {
            de_score += 1;
        }
    }

    for w in [
        "the", "and", "is", "not", "to", "for", "with", "this", "that", "was",
        "are", "been", "have", "from", "will", "their", "would", "could",
        "should", "about", "into", "than", "then", "there", "here",
        "some", "them", "these", "those", "which", "what", "when", "where",
        "please", "thank", "thanks", "regards", "best", "kind", "dear",
        "hello", "hi", "subject", "attached", "regarding", "further",
    ] {
        if lower.contains(&format!(" {} ", w)) {
            en_score += 1;
        }
    }

    if en_score > de_score + 2 {
        "en"
    } else {
        "de"
    }
}
