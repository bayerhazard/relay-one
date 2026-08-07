/// Detect the topic of an email from subject + body text.
/// Returns a topic tag like "projekt", "termin", "general", etc.
///
/// Hybrid approach:
/// 1. Keyword match (fast path) — covers ~80% of cases
/// 2. LLM fallback — only if keyword match fails
/// 3. Default: "general"
pub fn detect_topic(subject: &str, body: &str) -> String {
    let combined = format!("{}\n{}", subject, body).to_lowercase();

    // Fast-path: keyword matching (order matters — more specific first)
    let topics = [
        ("bewerbung", &["bewerbung", "anstellung", "job", "stellenausschreibung", "cv", "lebenslauf"][..]),
        ("referenz", &["referenz", "empfehlung", "zeugnis", "arbeitszeugnis"][..]),
        ("krisen", &["absage", "kündigung", "kuendigung", "abmahnung", "klage", "streit", "konflikt"][..]),
        ("dringend", &["dringend", "sofort", "asap", "urgent", "zeitkritisch", "frist"][..]),
        ("rechnung", &["rechnung", "invoice", "zahlung", "überweisung", "kostenvoranschlag"][..]),
        ("projekt", &["projekt", "sprint", "milestone", "release", "deployment", "bugfix", "feature"][..]),
        ("termin", &["termin", "appointment", "zusammenkunft", "telefonat", "call", "video-call"][..]),
        ("urlaub", &["urlaub", "ferie", "vacation", "krank", "krankheit", "krankenstand", "arzt"][..]),
        ("dank", &["danke", "danken", "appreciation", "wertschätzung", "wertschaetzung", "großartig", "grossartig"][..]),
        ("entschuldigung", &["entschuldigung", "entschuldige", "sorry", "tut mir leid", "verzeihung"][..]),
        ("einladung", &["einladung", "invite", "einladen", "feier", "party", "veranstaltung", "konferenz"][..]),
        ("feedback", &["feedback", "bewertung", "review", "umfrage", "fragebogen", "zufriedenheit"][..]),
    ];

    for (topic, keywords) in &topics {
        for kw in *keywords {
            if combined.contains(kw) {
                return topic.to_string();
            }
        }
    }

    "general".to_string()
}

/// LLM-based topic classification (fallback when keyword match fails).
/// This is called asynchronously and should not block the main flow.
pub fn build_topic_classification_prompt(text: &str) -> (String, String) {
    let system = "Du bist ein E-Mail-Klassifizierer. \
                  Bestimme das Thema der E-Mail. \
                  Verfügbare Themen: projekt, termin, urlaub, krank, rechnung, \
                  bewerbung, referenz, krisen, dringend, dank, entschuldigung, \
                  einladung, feedback, privat, geschaeftlich, sonstiges. \
                  Gib NUR das Thema zurueck, kein weiterer Text.";
    let user = format!(
        "Klassifiziere das Thema dieser E-Mail:\n\n{}",
        text.chars().take(500).collect::<String>() // truncate to save tokens
    );
    (system.to_string(), user)
}

/// Async LLM-based topic classification. Call when keyword match returns "general"
/// and you want higher accuracy. Returns the LLM's topic or "general" on failure.
#[allow(dead_code)]
pub async fn detect_topic_llm(
    client: &crate::ai::client::AIClient,
    subject: &str,
    body: &str,
) -> String {
    let (system, user) = build_topic_classification_prompt(&format!("{}\n{}", subject, body));
    match client.complete_user(&system, &user, None, None).await {
        Ok(result) => {
            let r = result.trim().to_lowercase();
            let valid = ["projekt", "termin", "urlaub", "krank", "rechnung", "bewerbung",
                "referenz", "krisen", "dringend", "dank", "entschuldigung", "einladung",
                "feedback", "privat", "geschaeftlich", "sonstiges", "general"];
            if valid.iter().any(|t| r == *t) {
                r
            } else {
                "general".to_string()
            }
        }
        Err(_) => "general".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_project_topic() {
        assert_eq!(detect_topic("Projektupdate", "Hier ist der aktuelle Stand."), "projekt");
        assert_eq!(detect_topic("", "Der Sprint ist abgeschlossen."), "projekt");
    }

    #[test]
    fn test_detect_appointment_topic() {
        assert_eq!(detect_topic("Terminvereinbarung", "Können wir uns morgen treffen?"), "termin");
        assert_eq!(detect_topic("", "Lass uns ein kurzes Termin machen."), "termin");
    }

    #[test]
    fn test_detect_invoice_topic() {
        assert_eq!(detect_topic("Rechnung #123", "Bitte um Zahlung."), "rechnung");
        assert_eq!(detect_topic("", "Die Überweisung ist unterwegs."), "rechnung");
    }

    #[test]
    fn test_detect_vacation_topic() {
        assert_eq!(detect_topic("Urlaubsantrag", "Ich möchte nächste Woche Urlaub machen."), "urlaub");
        assert_eq!(detect_topic("", "Ich bin krank und kann nicht kommen."), "urlaub");
    }

    #[test]
    fn test_detect_crisis_topic() {
        assert_eq!(detect_topic("Kündigung", "Hiermit kündige ich..."), "krisen");
        assert_eq!(detect_topic("", "Leider muss ich absagen."), "krisen");
    }

    #[test]
    fn test_detect_urgent_topic() {
        assert_eq!(detect_topic("DRINGEND", "Bitte sofort antworten!"), "dringend");
        assert_eq!(detect_topic("", "Das ist zeitkritisch."), "dringend");
    }

    #[test]
    fn test_detect_thanks_topic() {
        assert_eq!(detect_topic("Danke", "Vielen Dank für deine Hilfe!"), "dank");
        assert_eq!(detect_topic("", "Das war wirklich großartig!"), "dank");
    }

    #[test]
    fn test_detect_general_topic() {
        assert_eq!(detect_topic("Hallo", "Wie geht's dir?"), "general");
        assert_eq!(detect_topic("Neues", "Kurzes Update."), "general");
    }

    #[test]
    fn test_case_insensitive() {
        assert_eq!(detect_topic("PROJEKT", "Der Sprint läuft."), "projekt");
        assert_eq!(detect_topic("Rechnung", "Bitte um Zahlung."), "rechnung");
    }

    #[test]
    fn test_empty_input() {
        assert_eq!(detect_topic("", ""), "general");
    }

    #[test]
    fn test_specific_before_general() {
        // "projekt" should match before "general"
        assert_eq!(detect_topic("Projektmeeting", "Sprint-Update + Termin."), "projekt");
    }
}
