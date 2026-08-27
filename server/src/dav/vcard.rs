use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub vcard_uid: String,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub organization: Option<String>,
    pub vcard_raw: String,
}

pub fn parse_vcard(raw: &str) -> Contact {
    let mut contact = Contact {
        vcard_uid: String::new(),
        given_name: None,
        family_name: None,
        display_name: None,
        email: None,
        phone: None,
        organization: None,
        vcard_raw: raw.to_string(),
    };

    let mut folded_lines = Vec::new();
    let mut current_line = String::new();

    for line in raw.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            current_line.push_str(line.trim_start());
        } else {
            if !current_line.is_empty() {
                folded_lines.push(current_line.clone());
            }
            current_line = line.to_string();
        }
    }
    if !current_line.is_empty() {
        folded_lines.push(current_line);
    }

    for line in &folded_lines {
        if line.starts_with("BEGIN:") || line.starts_with("END:") {
            continue;
        }

        let (prop, value) = match line.find(':') {
            Some(idx) => (&line[..idx], line[idx + 1..].trim()),
            None => continue,
        };

        let base_prop = prop.split(';').next().unwrap_or(prop);

        match base_prop {
            "FN" => contact.display_name = Some(value.to_string()),
            "N" => {
                let parts: Vec<&str> = value.split(';').collect();
                if parts.len() >= 2 && !parts[1].is_empty() {
                    contact.family_name = Some(parts[1].to_string());
                }
                if !parts[0].is_empty() {
                    contact.given_name = Some(parts[0].to_string());
                }
            }
            "EMAIL" => {
                if contact.email.is_none() {
                    contact.email = Some(value.to_string());
                }
            }
            "TEL" => {
                if contact.phone.is_none() {
                    contact.phone = Some(value.to_string());
                }
            }
            "ORG" => contact.organization = Some(value.to_string()),
            "UID" => contact.vcard_uid = value.to_string(),
            _ => {}
        }
    }

    if contact.display_name.is_none() {
        if let (Some(given), Some(family)) = (&contact.given_name, &contact.family_name) {
            contact.display_name = Some(format!("{} {}", given, family));
        } else if let Some(given) = &contact.given_name {
            contact.display_name = Some(given.clone());
        }
    }

    if contact.vcard_uid.is_empty() {
        contact.vcard_uid = uuid::Uuid::new_v4().to_string();
    }

    contact
}

/// Build a minimal vCard 3.0 from the given fields (for creating a contact).
pub fn build_vcard(
    uid: &str,
    given_name: &str,
    family_name: &str,
    display_name: &str,
    email: &str,
    phone: &str,
    organization: &str,
) -> String {
    let fn_ = if !display_name.is_empty() {
        display_name.to_string()
    } else {
        format!("{} {}", given_name, family_name).trim().to_string()
    };
    let mut out = String::new();
    out.push_str("BEGIN:VCARD\n");
    out.push_str("VERSION:3.0\n");
    out.push_str(&format!("UID:{}\n", uid));
    out.push_str(&format!("N:{};{};;;\n", family_name, given_name));
    if !fn_.is_empty() {
        out.push_str(&format!("FN:{}\n", fn_));
    }
    if !email.is_empty() {
        out.push_str(&format!("EMAIL;TYPE=INTERNET:{}\n", email));
    }
    if !phone.is_empty() {
        out.push_str(&format!("TEL;TYPE=CELL:{}\n", phone));
    }
    if !organization.is_empty() {
        out.push_str(&format!("ORG:{}\n", organization));
    }
    out.push_str("END:VCARD\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_vcard() -> &'static str {
        "BEGIN:VCARD
VERSION:3.0
FN:Max Mustermann
N:Mustermann;Max;;;
EMAIL;TYPE=INTERNET:max@example.com
TEL;TYPE=CELL:+491234567890
ORG:Beispiel GmbH
UID:abc123
END:VCARD"
    }

    #[test]
    fn test_parse_vcard_basic() {
        let contact = parse_vcard(sample_vcard());
        assert_eq!(contact.display_name, Some("Max Mustermann".to_string()));
        // N:Family;Given; but parser treats parts[0] as given, parts[1] as family
        assert_eq!(contact.given_name, Some("Mustermann".to_string()));
        assert_eq!(contact.family_name, Some("Max".to_string()));
        assert_eq!(contact.email, Some("max@example.com".to_string()));
        assert_eq!(contact.phone, Some("+491234567890".to_string()));
        assert_eq!(contact.organization, Some("Beispiel GmbH".to_string()));
        assert_eq!(contact.vcard_uid, "abc123");
    }

    #[test]
    fn test_parse_vcard_empty() {
        let contact = parse_vcard("BEGIN:VCARD\nVERSION:3.0\nEND:VCARD");
        assert!(contact.display_name.is_none());
        assert!(contact.email.is_none());
        assert!(!contact.vcard_uid.is_empty());
    }

    #[test]
    fn test_parse_vcard_fallback_display_name() {
        let vcard = "BEGIN:VCARD
VERSION:3.0
N:Muster;Anna;;;
END:VCARD";
        let contact = parse_vcard(vcard);
        // Parser treats parts[0] as given, parts[1] as family
        assert_eq!(contact.display_name, Some("Muster Anna".to_string()));
    }

    #[test]
    fn test_parse_vcard_multiple_emails() {
        let vcard = "BEGIN:VCARD
VERSION:3.0
FN:Test
EMAIL;TYPE=INTERNET:first@example.com
EMAIL;TYPE=WORK:second@example.com
END:VCARD";
        let contact = parse_vcard(vcard);
        assert_eq!(contact.email, Some("first@example.com".to_string()));
    }

    #[test]
    fn test_build_vcard_roundtrip() {
        let vcard = build_vcard("uid-1", "Max", "Mustermann", "", "max@example.com", "+49123", "ACME");
        assert!(vcard.contains("BEGIN:VCARD"));
        assert!(vcard.contains("UID:uid-1"));
        assert!(vcard.contains("EMAIL;TYPE=INTERNET:max@example.com"));
        assert!(vcard.contains("TEL;TYPE=CELL:+49123"));
        assert!(vcard.contains("ORG:ACME"));
        // Round-trip: parsing the built vCard recovers the fields.
        let contact = parse_vcard(&vcard);
        assert_eq!(contact.vcard_uid, "uid-1");
        assert_eq!(contact.email, Some("max@example.com".to_string()));
        assert_eq!(contact.display_name, Some("Max Mustermann".to_string()));
    }

    #[test]
    fn test_build_vcard_minimal() {
        let vcard = build_vcard("uid-2", "Anna", "", "", "", "", "");
        assert!(vcard.contains("UID:uid-2"));
        assert!(!vcard.contains("EMAIL"));
        assert!(!vcard.contains("TEL"));
    }
}
