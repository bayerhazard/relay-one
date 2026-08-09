//! EML archive: raw RFC822 messages persisted to disk under
//! `<data>/archive/<account>/YYYY/MM/<uid>-<sha1-of-msgid>.eml`.
//!
//! This is the local-first source of truth (Concept §3.1). SQLite keeps the
//! parsed index (fast UI), the EML files back up / export cleanly.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::Digest as _;

/// Build the archive path for a message:
/// `<data>/archive/<account>/<YYYY>/<MM>/<uid>-<sha1>.eml`
///
/// The `<sha1>` is the first 12 hex chars of the message-id (fallback: uid),
/// so re-importing the same message twice produces the same file.
pub fn archive_path_for(
    data_root: &Path,
    account_id: i64,
    uid: u32,
    date: Option<&str>,
    message_id: Option<&str>,
) -> PathBuf {
    let (year, month) = split_date(date);
    let slug = msg_slug(uid, message_id);
    data_root
        .join("archive")
        .join(account_id.to_string())
        .join(year)
        .join(month)
        .join(format!("{}-{}.eml", uid, slug))
}

/// Persist raw RFC822 bytes to the archive. Returns the absolute path or an
/// error string. Idempotent: if the file already exists with identical
/// content, it is left untouched.
pub fn write_eml(
    data_root: &Path,
    account_id: i64,
    uid: u32,
    date: Option<&str>,
    message_id: Option<&str>,
    raw: &[u8],
) -> Result<PathBuf, String> {
    let path = archive_path_for(data_root, account_id, uid, date, message_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("EML-Archiv: Ordner {parent:?} anlegen fehlgeschlagen: {e}"))?;
    }
    if path.exists() {
        let existing = fs::read(&path).map_err(|e| format!("EML lesen fehlgeschlagen: {e}"))?;
        if existing == raw {
            return Ok(path);
        }
        // Different content with the same uid+slug: overwrite (same message
        // re-fetched after a change on the server).
    }
    fs::write(&path, raw).map_err(|e| format!("EML-Archiv: Schreiben {path:?} fehlgeschlagen: {e}"))?;
    Ok(path)
}

/// SHA-256 of raw content (for attachment dedup / verify).
pub fn sha256_hex(raw: &[u8]) -> String {
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(raw);
    let out = h.finalize();
    out.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Store attachment content deduplicated under `<data>/attachments/<sha256>`.
/// Returns (absolute path, sha256 hex, is_new). Identical content is stored
/// exactly once — re-downloads reuse the existing file.
pub fn save_attachment(data_root: &Path, content: &[u8]) -> Result<(PathBuf, String, bool), String> {
    let digest = sha256_hex(content);
    let path = data_root.join("attachments").join(&digest);
    if path.exists() {
        return Ok((path, digest, false));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Attachment-Archiv: Ordner {parent:?} anlegen fehlgeschlagen: {e}"))?;
    }
    fs::write(&path, content).map_err(|e| format!("Attachment-Archiv: Schreiben {path:?} fehlgeschlagen: {e}"))?;
    Ok((path, digest, true))
}

/// Verify an archived EML: file exists, size matches, and content hash matches.
pub fn verify_eml(path: &Path, expected_sha256: Option<&str>, expected_size: Option<u64>) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if let Some(sz) = expected_size {
        if meta.len() != sz {
            return false;
        }
    }
    if let Some(hash) = expected_sha256 {
        let Ok(bytes) = fs::read(path) else { return false };
        if sha256_hex(&bytes) != hash {
            return false;
        }
    }
    true
}

/// Extract a reasonable `<date>` string (YYYY-MM-DD) from a stored date.
fn split_date(date: Option<&str>) -> (String, String) {
    let d = date.unwrap_or_default();
    // Date is stored as ISO-ish (e.g. "2026-08-09T10:11:12Z" or "2026-08-09").
    let ymd: Vec<&str> = d.split('T').next().unwrap_or("").split('-').collect();
    let year = if ymd.len() >= 1 && ymd[0].len() == 4 && ymd[0].starts_with("20") {
        ymd[0].to_string()
    } else {
        "1970".to_string()
    };
    let month = if ymd.len() >= 2 && ymd[1].len() == 2 {
        ymd[1].to_string()
    } else {
        "01".to_string()
    };
    (year, month)
}

/// Message-id slug for the filename (stable, filesystem-safe).
fn msg_slug(uid: u32, message_id: Option<&str>) -> String {
    match message_id {
        Some(mid) if !mid.trim().is_empty() => {
            let mut h = sha2::Sha256::new();
            h.update(mid.trim().as_bytes());
            let digest = h.finalize();
            digest.iter().take(6).map(|b| format!("{:02x}", b)).collect()
        }
        _ => {
            // Fallback: hash of uid so filenames stay stable.
            let mut h = sha2::Sha256::new();
            h.update(uid.to_string().as_bytes());
            let digest = h.finalize();
            digest.iter().take(6).map(|b| format!("{:02x}", b)).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_write_eml_creates_file() {
        let dir = tempdir().unwrap();
        let path = write_eml(
            dir.path(),
            7,
            42,
            Some("2026-08-09T10:00:00Z"),
            Some("<abc@example.com>"),
            b"Subject: test\r\n\r\nbody",
        )
        .unwrap();
        assert!(path.exists());
        let rel = path.strip_prefix(dir.path()).unwrap();
        assert!(rel.to_string_lossy().starts_with("archive/7/2026/08/42-"), "layout: {rel:?}");
        assert!(rel.to_string_lossy().ends_with(".eml"), "extension: {rel:?}");
    }

    #[test]
    fn test_write_eml_idempotent_same_content() {
        let dir = tempdir().unwrap();
        let raw = b"Subject: x\r\n\r\nhello";
        let p1 = write_eml(dir.path(), 1, 1, Some("2026-08-09"), Some("<m1@x>"), raw).unwrap();
        let p2 = write_eml(dir.path(), 1, 1, Some("2026-08-09"), Some("<m1@x>"), raw).unwrap();
        assert_eq!(p1, p2);
    }

    #[test]
    fn test_verify_eml_ok_and_bad_hash() {
        let dir = tempdir().unwrap();
        let raw = b"Subject: v\r\n\r\nbody";
        let path = write_eml(dir.path(), 2, 3, Some("2026-08-09"), Some("<v@x>"), raw).unwrap();
        let hash = sha256_hex(raw);
        assert!(verify_eml(&path, Some(&hash), Some(raw.len() as u64)));
        assert!(!verify_eml(&path, Some("deadbeef"), Some(raw.len() as u64)));
    }

    #[test]
    fn test_split_date_variants() {
        assert_eq!(split_date(Some("2026-08-09T10:00:00Z")), ("2026".into(), "08".into()));
        assert_eq!(split_date(Some("2026-08-09")), ("2026".into(), "08".into()));
        assert_eq!(split_date(None), ("1970".into(), "01".into()));
    }
}
