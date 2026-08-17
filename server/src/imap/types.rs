use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MailEnvelope {
    pub subject: String,
    pub from: String,
    pub to: String,
    pub cc: String,
    pub date: String,
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedMessage {
    pub uid: u32,
    pub envelope: MailEnvelope,
    pub flags: Vec<String>,
    pub body_preview: Option<String>,
    pub body_structure: Option<String>,
    pub ai_summary: Option<String>,
    pub ai_priority: Option<f32>,
    pub ai_fraud_score: Option<f32>,
    pub cached_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
   pub is_read: bool,
    /// True if the message is flagged (marked/starred) by the user.
    #[serde(default)]
    pub is_flagged: bool,
     /// True if the message has at least one attachment (derived from
    /// BODYSTRUCTURE, without downloading the body).
    #[serde(default)]
    pub has_attachments: bool,
    /// Attachment metadata extracted from BODYSTRUCTURE during sync.
    #[serde(default)]
    pub attachments: Vec<AttachmentMeta>,
}

/// Attachment metadata extracted from BODYSTRUCTURE or full message parse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentMeta {
    pub filename: String,
    pub content_type: String,
    pub size: usize,
}

/// IMAP SPECIAL-USE folder attributes (RFC 6154).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SpecialFolder {
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
}

impl SpecialFolder {
    /// The IMAP attribute string used in LIST responses.
    pub fn attr_name(&self) -> &'static str {
        match self {
            SpecialFolder::Sent => "\\Sent",
            SpecialFolder::Drafts => "\\Drafts",
            SpecialFolder::Trash => "\\Trash",
            SpecialFolder::Junk => "\\Junk",
            SpecialFolder::Archive => "\\Archive",
        }
    }

    /// Fallback common names when SPECIAL-USE attributes are not available.
    pub fn fallback_names(&self) -> &'static [&'static str] {
        match self {
            SpecialFolder::Sent => &["Sent", "Sent Messages", "Sent Items", "Gesendet", "INBOX.Sent"],
            SpecialFolder::Drafts => &["Drafts", "Entwürfe", "INBOX.Drafts"],
            SpecialFolder::Trash => &["Trash", "Deleted", "Gelöscht", "Papierkorb"],
            SpecialFolder::Junk => &["Spam", "Junk", "Spamverdacht", "Junk E-Mail"],
            SpecialFolder::Archive => &["Archive", "Archiv", "All Mail", "Alle Mail"],
        }
    }
}

/// A folder entry returned from IMAP LIST, including SPECIAL-USE attributes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderEntry {
    pub name: String,
    pub raw_name: String,
    pub delimiter: String,
    pub tag: String,
    pub attributes: Vec<String>,
}

impl FolderEntry {
    pub fn has_attribute(&self, attr: &str) -> bool {
        self.attributes.iter().any(|a| a.eq_ignore_ascii_case(attr))
    }

    pub fn is_noselect(&self) -> bool {
        self.attributes.iter().any(|a| a.eq_ignore_ascii_case("\\NoSelect"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_special_folder_attr_names() {
        assert_eq!(SpecialFolder::Sent.attr_name(), "\\Sent");
        assert_eq!(SpecialFolder::Drafts.attr_name(), "\\Drafts");
        assert_eq!(SpecialFolder::Trash.attr_name(), "\\Trash");
        assert_eq!(SpecialFolder::Junk.attr_name(), "\\Junk");
        assert_eq!(SpecialFolder::Archive.attr_name(), "\\Archive");
    }

    #[test]
    fn test_special_folder_fallback_names() {
        assert!(SpecialFolder::Drafts.fallback_names().contains(&"Entwürfe"));
        assert!(SpecialFolder::Sent.fallback_names().contains(&"Gesendet"));
        assert!(SpecialFolder::Trash.fallback_names().contains(&"Papierkorb"));
        assert!(SpecialFolder::Junk.fallback_names().contains(&"Spam"));
        assert!(SpecialFolder::Archive.fallback_names().contains(&"Archiv"));
    }

    #[test]
    fn test_folder_entry_has_attribute() {
        let f = FolderEntry {
            name: "INBOX".into(),
            raw_name: "INBOX".into(),
            delimiter: "/".into(),
            tag: "folder".into(),
            attributes: vec!["\\HasNoChildren".into(), "\\Seen".into()],
        };
        assert!(f.has_attribute("\\Seen"));
        assert!(f.has_attribute("\\seen"));
        assert!(!f.has_attribute("\\Sent"));
    }

    #[test]
    fn test_folder_entry_is_noselect() {
        let f = FolderEntry {
            name: "Noselect".into(),
            raw_name: "Noselect".into(),
            delimiter: "/".into(),
            tag: "noselect".into(),
            attributes: vec!["\\NoSelect".into()],
        };
        assert!(f.is_noselect());

        let f2 = FolderEntry {
            name: "Selectable".into(),
            raw_name: "Selectable".into(),
            delimiter: "/".into(),
            tag: "folder".into(),
            attributes: vec![],
        };
        assert!(!f2.is_noselect());
    }
}
