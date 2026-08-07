use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::tone::analyzer::analyze_mail;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactProfile {
    pub email_hash: String,
    pub display_name: Option<String>,
    pub address_mode: String,
    pub formality_score: f32,
    pub friendliness_score: f32,
    pub salutation_detected: Option<String>,
    pub closing_detected: Option<String>,
    pub pronoun_detected: Option<String>,
    pub sample_count: u32,
    pub confidence: f32,
    pub last_analyzed_at: String,
}

pub struct ProfileManager;

impl ProfileManager {
    pub fn hash_email(email: &str) -> String {
        let normalized = email.trim().to_lowercase();
        let mut hasher = Sha256::new();
        hasher.update(normalized.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    pub fn get_profile(
        db: &Connection,
        account_id: i64,
        email: &str,
    ) -> Result<ContactProfile, rusqlite::Error> {
        let hash = Self::hash_email(email);

        db.query_row(
            "SELECT email_hash, display_name, address_mode, formality_score,
                    friendliness_score, salutation_detected, closing_detected,
                    pronoun_detected, sample_count, last_analyzed_at
             FROM contact_profiles WHERE account_id = ?1 AND email_hash = ?2",
            params![account_id, hash],
            |row| {
                Ok(ContactProfile {
                    email_hash: row.get(0)?,
                    display_name: row.get(1)?,
                    address_mode: row.get(2)?,
                    formality_score: row.get(3)?,
                    friendliness_score: row.get(4)?,
                    salutation_detected: row.get(5)?,
                    closing_detected: row.get(6)?,
                    pronoun_detected: row.get(7)?,
                    sample_count: row.get(8)?,
                    confidence: 0.5,
                    last_analyzed_at: row.get(9)?,
                })
            },
        )
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                let now = chrono::Utc::now().to_rfc3339();
                Ok(ContactProfile {
                    email_hash: hash,
                    display_name: None,
                    address_mode: "unknown".into(),
                    formality_score: 0.5,
                    friendliness_score: 0.5,
                    salutation_detected: None,
                    closing_detected: None,
                    pronoun_detected: None,
                    sample_count: 0,
                    confidence: 0.0,
                    last_analyzed_at: now,
                })
            }
            other => Err(other),
        })
    }

    pub fn update_profile_from_mail(
        db: &Connection,
        account_id: i64,
        email: &str,
        mail_text: &str,
        display_name: Option<&str>,
    ) -> Result<ContactProfile, String> {
        let hash = Self::hash_email(email);
        let signals = analyze_mail(mail_text);
        let now = chrono::Utc::now().to_rfc3339();

        let existing = Self::get_profile(db, account_id, email).map_err(|e| e.to_string())?;

        let n = existing.sample_count as f32;
        let weight_new = if n < 10.0 { 0.5 } else { 0.3 };
        let weight_old = 1.0 - weight_new;

        let formality = if signals.confidence > 0.3 {
            existing.formality_score * weight_old + signals.formality * weight_new
        } else {
            existing.formality_score
        };

        let friendliness = if signals.confidence > 0.3 {
            existing.friendliness_score * weight_old + signals.friendliness * weight_new
        } else {
            existing.friendliness_score
        };

        let address_mode = if signals.address_mode != "unknown" {
            signals.address_mode.clone()
        } else {
            existing.address_mode.clone()
        };

        let pronoun = if signals.pronoun != "unknown" {
            signals.pronoun.clone()
        } else {
            existing.pronoun_detected.clone().unwrap_or_default()
        };

        db.execute(
            "INSERT INTO contact_profiles (
                account_id, email_hash, display_name, address_mode,
                formality_score, friendliness_score, salutation_detected,
                closing_detected, pronoun_detected, language, sample_count,
                first_seen_at, last_analyzed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'de', 1, ?10, ?10)
             ON CONFLICT(account_id, email_hash) DO UPDATE SET
                display_name = COALESCE(?3, display_name),
                address_mode = ?4,
                formality_score = ?5,
                friendliness_score = ?6,
                salutation_detected = ?7,
                closing_detected = ?8,
                pronoun_detected = ?9,
                sample_count = sample_count + 1,
                last_analyzed_at = ?10,
                updated_at = datetime('now')",
            params![
                account_id,
                hash,
                display_name,
                address_mode,
                formality,
                friendliness,
                signals.salutation,
                signals.closing,
                pronoun,
                now,
            ],
        )
        .map_err(|e| e.to_string())?;

        Ok(ContactProfile {
            email_hash: hash,
            display_name: display_name.map(|s| s.into()),
            address_mode,
            formality_score: formality,
            friendliness_score: friendliness,
            salutation_detected: Some(signals.salutation),
            closing_detected: Some(signals.closing),
            pronoun_detected: Some(pronoun),
            sample_count: existing.sample_count + 1,
            confidence: signals.confidence,
            last_analyzed_at: now,
        })
    }

    pub fn export_as_markdown(db: &Connection, account_id: i64) -> Result<String, String> {
        let mut stmt = db
            .prepare(
                "SELECT email_hash, display_name, address_mode, formality_score,
                        friendliness_score, salutation_detected, closing_detected,
                        pronoun_detected, sample_count, last_analyzed_at
                 FROM contact_profiles
                 WHERE account_id = ?1
                 ORDER BY sample_count DESC",
            )
            .map_err(|e| e.to_string())?;

        let mut md = String::from("# Kontakt-Tonalit\u{00e4}tsprofile\n\n");
        md.push_str("| E-Mail (Hash) | Name | Anrede | Form. | Freundl. | Anredeform | Gru\u{00df} | Pronomen | Mails |\n");
        md.push_str("|---|---|---|---|---|---|---|---|---|\n");

        let rows = stmt
            .query_map(params![account_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, f32>(3)?,
                    row.get::<_, f32>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            })
            .map_err(|e| e.to_string())?;

        for row in rows {
            let (hash, name, mode, form, friend, sal, close, pron, count) =
                row.map_err(|e| e.to_string())?;
            let hash_short = &hash[..8.min(hash.len())];
            let form_bar = format_score_bar(form);
            let friend_bar = format_score_bar(friend);
            md.push_str(&format!(
                "| `{}` | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                hash_short,
                name.as_deref().unwrap_or("\u{2014}"),
                mode,
                form_bar,
                friend_bar,
                sal.as_deref().unwrap_or("\u{2014}"),
                close.as_deref().unwrap_or("\u{2014}"),
                pron.as_deref().unwrap_or("\u{2014}"),
                count,
            ));
        }

        md.push_str("\n---\n*Automatisch generiert aus deinem E-Mail-Verkehr.*\n");
        Ok(md)
    }
}

fn format_score_bar(score: f32) -> String {
    let filled = (score * 10.0).round() as usize;
    let empty = 10 - filled;
    format!(
        "[{}{}]",
        "\u{2588}".repeat(filled),
        "\u{2591}".repeat(empty)
    )
}

#[allow(dead_code)]
fn create_test_db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS contact_profiles (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            email_hash TEXT NOT NULL,
            display_name TEXT,
            address_mode TEXT NOT NULL DEFAULT 'unknown',
            formality_score REAL NOT NULL DEFAULT 0.5,
            friendliness_score REAL NOT NULL DEFAULT 0.5,
            salutation_detected TEXT,
            closing_detected TEXT,
            pronoun_detected TEXT,
            language TEXT NOT NULL DEFAULT 'de',
            sample_count INTEGER NOT NULL DEFAULT 0,
            first_seen_at TEXT NOT NULL,
            last_analyzed_at TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(account_id, email_hash)
        );
        CREATE INDEX IF NOT EXISTS idx_contact_profiles_email
            ON contact_profiles(account_id, email_hash);",
    )
    .unwrap();
    conn
}

#[allow(dead_code)]
fn insert_test_profile(conn: &rusqlite::Connection, account_id: i64, email: &str, display_name: Option<&str>) {
    let hash = ProfileManager::hash_email(email);
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO contact_profiles (account_id, email_hash, display_name, address_mode,
         formality_score, friendliness_score, salutation_detected, closing_detected,
         pronoun_detected, language, sample_count, first_seen_at, last_analyzed_at)
         VALUES (?1, ?2, ?3, 'last_name', 0.9, 0.2, 'Sehr geehrte/r', 'Mit freundlichen Grüßen',
         'Sie', 'de', 15, ?4, ?4)",
        rusqlite::params![account_id, hash, display_name, now],
    )
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── hash_email tests ────────────────────────────────────────────

    #[test]
    fn test_hash_email_case_insensitive() {
        let h1 = ProfileManager::hash_email("Max.Mustermann@Example.com");
        let h2 = ProfileManager::hash_email("max.mustermann@example.com");
        assert_eq!(h1, h2, "email hashing should be case-insensitive");
    }

    #[test]
    fn test_hash_email_deterministic() {
        let h1 = ProfileManager::hash_email("test@example.com");
        let h2 = ProfileManager::hash_email("test@example.com");
        assert_eq!(h1, h2, "same input should produce same hash");
    }

    #[test]
    fn test_hash_email_trimming() {
        let h1 = ProfileManager::hash_email("  user@example.com  ");
        let h2 = ProfileManager::hash_email("user@example.com");
        assert_eq!(h1, h2, "email hashing should trim whitespace");
    }

    #[test]
    fn test_hash_email_format() {
        let hash = ProfileManager::hash_email("a@b.com");
        // SHA-256 produces 64 hex characters
        assert_eq!(hash.len(), 64, "SHA-256 hash should be 64 hex chars");
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()), "hash should be hex");
    }

    #[test]
    fn test_hash_email_empty() {
        let hash = ProfileManager::hash_email("");
        assert_eq!(hash.len(), 64);
    }

    // ── get_profile tests ───────────────────────────────────────────

    #[test]
    fn test_get_profile_non_existent() {
        let conn = create_test_db();
        let profile = ProfileManager::get_profile(&conn, 1, "nobody@example.com").unwrap();
        assert_eq!(profile.email_hash, ProfileManager::hash_email("nobody@example.com"));
        assert!(profile.display_name.is_none());
        assert_eq!(profile.address_mode, "unknown");
        assert_eq!(profile.formality_score, 0.5);
        assert_eq!(profile.friendliness_score, 0.5);
        assert_eq!(profile.sample_count, 0);
        assert_eq!(profile.confidence, 0.0);
        assert!(profile.salutation_detected.is_none());
        assert!(profile.closing_detected.is_none());
        assert!(profile.pronoun_detected.is_none());
    }

    #[test]
    fn test_get_profile_existing() {
        let conn = create_test_db();
        insert_test_profile(&conn, 1, "existing@example.com", Some("Dr. Meier"));
        let profile = ProfileManager::get_profile(&conn, 1, "existing@example.com").unwrap();
        assert_eq!(profile.display_name.as_deref(), Some("Dr. Meier"));
        assert_eq!(profile.address_mode, "last_name");
        assert_eq!(profile.formality_score, 0.9);
        assert_eq!(profile.friendliness_score, 0.2);
        assert_eq!(profile.sample_count, 15);
        assert_eq!(profile.salutation_detected.as_deref(), Some("Sehr geehrte/r"));
        assert_eq!(profile.closing_detected.as_deref(), Some("Mit freundlichen Grüßen"));
        assert_eq!(profile.pronoun_detected.as_deref(), Some("Sie"));
    }

    #[test]
    fn test_get_profile_wrong_account() {
        let conn = create_test_db();
        insert_test_profile(&conn, 1, "user@example.com", Some("User"));
        // Different account_id should return default
        let profile = ProfileManager::get_profile(&conn, 2, "user@example.com").unwrap();
        assert_eq!(profile.sample_count, 0);
        assert_eq!(profile.confidence, 0.0);
    }

    // ── update_profile_from_mail tests ─────────────────────────────

    #[test]
    fn test_update_profile_new_contact() {
        let conn = create_test_db();
        let text = "Sehr geehrte Frau Schmidt,\n\nvielen Dank für Ihre Rückmeldung.\n\nMit freundlichen Grüßen\nMax";
        let profile = ProfileManager::update_profile_from_mail(
            &conn, 1, "new@example.com", text, Some("Frau Schmidt"),
        )
        .unwrap();
        assert_eq!(profile.display_name.as_deref(), Some("Frau Schmidt"));
        assert_eq!(profile.sample_count, 1);
        assert!(profile.formality_score > 0.5, "formal email should yield >0.5 formality");
        assert_eq!(profile.address_mode, "last_name");
    }

    #[test]
    fn test_update_profile_existing_contact() {
        let conn = create_test_db();
        insert_test_profile(&conn, 1, "existing@example.com", Some("Dr. Meier"));
        let text = "Hallo,\n\nkurze Rückmeldung.\n\nLG\nMax";
        let profile = ProfileManager::update_profile_from_mail(
            &conn, 1, "existing@example.com", text, Some("Dr. Meier"),
        )
        .unwrap();
        // Sample count should increase (15 + 1 = 16)
        assert_eq!(profile.sample_count, 16);
        // Formality with n=15 (old, weight 0.7) + new signal (weight 0.3, since n >= 10)
        // old=0.9, new= ~0.3 (informal) => weighted ~0.9*0.7 + 0.3*0.3 = 0.63 + 0.09 = 0.72
        assert!(profile.formality_score < 0.9, "formality should decrease from 0.9");
        assert!(profile.formality_score > 0.3, "formality should stay above informal value");
    }

    #[test]
    fn test_update_profile_low_confidence_does_not_update_scores() {
        let conn = create_test_db();
        insert_test_profile(&conn, 1, "stable@example.com", Some("Prof. Dr. Schulz"));
        // Empty text: salutation="none", closing="unknown", pronoun="unknown",
        // address_mode="unknown" => confidence = 0.0
        let text = "";
        let profile = ProfileManager::update_profile_from_mail(
            &conn, 1, "stable@example.com", text, Some("Prof. Dr. Schulz"),
        )
        .unwrap();
        // Confidence is 0.0 (no signals), so formality and friendliness should retain old values
        assert_eq!(profile.confidence, 0.0);
        assert!((profile.formality_score - 0.9).abs() < 0.01,
            "formality should stay at old value 0.9 when confidence is 0, got {}", profile.formality_score);
    }

    #[test]
    fn test_update_profile_new_contact_few_samples_weight() {
        let conn = create_test_db();
        // First insert: n=0 -> weight_new = 0.5
        let text = "Hallo Welt!\n\nLG\nThomas";
        let profile = ProfileManager::update_profile_from_mail(
            &conn, 1, "weighted@example.com", text, None,
        )
        .unwrap();
        assert_eq!(profile.sample_count, 1);
        // With n=0, weight_new=0.5, weight_old=0.5
        // Starting default formality is 0.5
        // New formality from informal text (Hallo + LG + unknown pronoun)
        // Hallo: -0.3, LG: -0.2 => 0.5 - 0.5 = 0.0 clamped to 0.0
        // weighted: 0.5 * 0.5 + 0.0 * 0.5 = 0.25
        assert!((profile.formality_score - 0.25).abs() < 0.01,
            "new contact with few samples should use 0.5 weight, got {}", profile.formality_score);
    }

    #[test]
    fn test_update_profile_address_mode_update() {
        let conn = create_test_db();
        insert_test_profile(&conn, 1, "addr@example.com", Some("Herr Meier"));
        // New mail has no clear address mode
        let text = "Hallo,\n\nkurze Info.\n\nLG";
        let profile = ProfileManager::update_profile_from_mail(
            &conn, 1, "addr@example.com", text, Some("Herr Meier"),
        )
        .unwrap();
        // Since signals.address_mode will be "first_name" (for "Hallo," ... wait no.
        // Actually, the text is "Hallo,\n\nkurze Info.\n\nLG" - the first line is "Hallo,"
        // HELLO_FIRST regex: (Hallo|Hey|Hi|Liebe[rn]?)\s+(\p{L}+)
        // "Hallo," - after "Hallo" there's "," not whitespace, so no capture
        // So address_mode should be "unknown" and we keep existing "last_name"
        assert_eq!(profile.address_mode, "last_name",
            "when new address_mode is unknown, should keep old value");
    }

    #[test]
    fn test_update_profile_pronoun_update() {
        let conn = create_test_db();
        insert_test_profile(&conn, 1, "pronoun@example.com", None);
        let text = "Hallo,\n\nkannst Du mir bitte helfen?\n\nLG";
        let profile = ProfileManager::update_profile_from_mail(
            &conn, 1, "pronoun@example.com", text, None,
        )
        .unwrap();
        assert_eq!(profile.pronoun_detected.as_deref(), Some("Du"),
            "pronoun should update to Du from mail text");
    }

    #[test]
    fn test_get_profile_after_update() {
        let conn = create_test_db();
        let text = "Sehr geehrte Damen,\n\nvielen Dank.\n\nMit freundlichen Grüßen\nIhr Team";
        ProfileManager::update_profile_from_mail(
            &conn, 1, "roundtrip@example.com", text, Some("Damen und Herren"),
        )
        .unwrap();
        let profile = ProfileManager::get_profile(&conn, 1, "roundtrip@example.com").unwrap();
        assert_eq!(profile.display_name.as_deref(), Some("Damen und Herren"));
        assert_eq!(profile.sample_count, 1);
        assert!(profile.formality_score > 0.5);
    }
}
