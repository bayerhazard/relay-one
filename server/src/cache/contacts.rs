//! Local contact cache (SQLite) + CardDAV CRUD helpers.

use rusqlite::Connection;

use crate::dav::vcard::Contact;

/// A contact row as returned to the API.
#[derive(serde::Serialize)]
pub struct ContactRow {
    pub vcard_uid: String,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub organization: Option<String>,
    pub source: String,
    pub synced_at: String,
}

fn row_to_contact_row(row: &rusqlite::Row) -> rusqlite::Result<ContactRow> {
    Ok(ContactRow {
        vcard_uid: row.get("vcard_uid")?,
        given_name: row.get("given_name")?,
        family_name: row.get("family_name")?,
        display_name: row.get("display_name")?,
        email: row.get("email")?,
        phone: row.get("phone")?,
        organization: row.get("organization")?,
        source: row.get("source")?,
        synced_at: row.get("synced_at")?,
    })
}

/// List contacts, optionally filtered by a case-insensitive search on
/// display name, given name, family name or email.
pub fn list_contacts(conn: &Connection, search: &str) -> Result<Vec<ContactRow>, String> {
    let like = format!("%{}%", search.to_lowercase());
    let sql = r#"
        SELECT vcard_uid, given_name, family_name, display_name, email, phone,
               organization, source, synced_at
        FROM contacts
        WHERE lower(display_name) LIKE ?1
           OR lower(coalesce(email,'')) LIKE ?1
           OR lower(coalesce(given_name,'')) LIKE ?1
           OR lower(coalesce(family_name,'')) LIKE ?1
        ORDER BY coalesce(display_name, given_name, email, '') COLLATE NOCASE
    "#;
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![like], row_to_contact_row)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Fetch a single contact by its vCard UID.
pub fn get_contact(conn: &Connection, uid: &str) -> Result<Option<ContactRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT vcard_uid, given_name, family_name, display_name, email, phone,
                    organization, source, synced_at
             FROM contacts WHERE vcard_uid = ?1",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map(rusqlite::params![uid], row_to_contact_row)
        .map_err(|e| e.to_string())?;
    match rows.next() {
        Some(Ok(c)) => Ok(Some(c)),
        Some(Err(e)) => Err(e.to_string()),
        None => Ok(None),
    }
}

/// Upsert a contact into the local cache.
pub fn upsert_contact(conn: &Connection, c: &Contact) -> Result<(), String> {
    conn.execute(
        "INSERT INTO contacts (vcard_uid, given_name, family_name, display_name, email, phone,
                               organization, vcard_raw, source, synced_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'carddav', datetime('now'))
         ON CONFLICT(vcard_uid) DO UPDATE SET
            given_name = excluded.given_name,
            family_name = excluded.family_name,
            display_name = excluded.display_name,
            email = excluded.email,
            phone = excluded.phone,
            organization = excluded.organization,
            vcard_raw = excluded.vcard_raw,
            synced_at = datetime('now')",
        rusqlite::params![
            c.vcard_uid,
            c.given_name,
            c.family_name,
            c.display_name,
            c.email,
            c.phone,
            c.organization,
            c.vcard_raw,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete a contact from the local cache by UID.
pub fn delete_contact(conn: &Connection, uid: &str) -> Result<(), String> {
    conn.execute("DELETE FROM contacts WHERE vcard_uid = ?1", rusqlite::params![uid])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Parse a mail address like `"Max Mustermann" <max@example.com>` or
/// `max@example.com` into `(display_name, email)`.
pub fn parse_address(addr: &str) -> (Option<String>, Option<String>) {
    let trimmed = addr.trim();
    if trimmed.is_empty() {
        return (None, None);
    }
    let lt = trimmed.find('<');
    let gt = trimmed.rfind('>');
    let (name, email) = match (lt, gt) {
        (Some(l), Some(g)) if g > l => {
            let name_part = trimmed[..l].trim().trim_matches('"').trim().to_string();
            let email_part = trimmed[l + 1..g].trim().to_string();
            (
                if name_part.is_empty() { None } else { Some(name_part) },
                if email_part.is_empty() { None } else { Some(email_part) },
            )
        }
        _ => {
            // No angle brackets: treat the whole thing as an email (or name).
            let email = if trimmed.contains('@') {
                Some(trimmed.to_string())
            } else {
                None
            };
            let name = if trimmed.contains('@') { None } else { Some(trimmed.to_string()) };
            (name, email)
        }
    };
    (name, email)
}

/// Auto-enrich the local contact cache from a mail envelope: upsert a contact
/// for the sender (and any recipient carrying a display name). Returns the
/// number of contacts upserted.
pub fn enrich_from_envelope(
    conn: &Connection,
    from: &str,
    to: &str,
    cc: &str,
) -> Result<usize, String> {
    let mut count = 0;
    // Sender is the primary, most reliable source.
    if let (Some(name), Some(email)) = parse_address(from) {
        if upsert_enriched(conn, &email, &name).is_ok() {
            count += 1;
        }
    }
    // Recipients with a display name (skip the user's own address — unknown
    // here, so we only enrich named recipients).
    for addr in to.split(',') {
        if let (Some(name), Some(email)) = parse_address(addr) {
            if upsert_enriched(conn, &email, &name).is_ok() {
                count += 1;
            }
        }
    }
    for addr in cc.split(',') {
        if let (Some(name), Some(email)) = parse_address(addr) {
            if upsert_enriched(conn, &email, &name).is_ok() {
                count += 1;
            }
        }
    }
    Ok(count)
}

/// Upsert a contact derived from mail (lowercase email as UID, source='mail').
fn upsert_enriched(conn: &Connection, email: &str, name: &str) -> Result<(), String> {
    let uid = format!("mail:{}", email.to_lowercase());
    conn.execute(
        "INSERT INTO contacts (vcard_uid, display_name, email, vcard_raw, source, synced_at)
         VALUES (?1, ?2, ?3, ?4, 'mail', datetime('now'))
         ON CONFLICT(vcard_uid) DO UPDATE SET
            display_name = CASE WHEN excluded.display_name IS NOT NULL AND excluded.display_name != ''
                                THEN excluded.display_name ELSE contacts.display_name END,
            synced_at = datetime('now')",
        rusqlite::params![
            uid,
            name,
            email,
            crate::dav::vcard::build_vcard(&uid, "", "", name, email, "", ""),
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::db::init_db;
    use rusqlite::Connection;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        conn
    }

    fn test_contact(uid: &str, name: &str, email: &str) -> Contact {
        Contact {
            vcard_uid: uid.to_string(),
            given_name: Some(name.to_string()),
            family_name: None,
            display_name: Some(name.to_string()),
            email: Some(email.to_string()),
            phone: None,
            organization: None,
            vcard_raw: "BEGIN:VCARD\nEND:VCARD".to_string(),
        }
    }

    #[test]
    fn test_upsert_and_list() {
        let conn = test_db();
        upsert_contact(&conn, &test_contact("u1", "Max", "max@example.com")).unwrap();
        upsert_contact(&conn, &test_contact("u2", "Erika", "erika@example.com")).unwrap();

        let all = list_contacts(&conn, "").unwrap();
        assert_eq!(all.len(), 2);

        // Upsert updates the same row (no duplicate).
        upsert_contact(&conn, &test_contact("u1", "Max", "max.new@example.com")).unwrap();
        let all = list_contacts(&conn, "").unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_search() {
        let conn = test_db();
        upsert_contact(&conn, &test_contact("u1", "Max", "max@example.com")).unwrap();
        upsert_contact(&conn, &test_contact("u2", "Erika", "erika@example.com")).unwrap();

        let found = list_contacts(&conn, "max").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].vcard_uid, "u1");

        // Email search.
        let by_email = list_contacts(&conn, "erika@").unwrap();
        assert_eq!(by_email.len(), 1);
        assert_eq!(by_email[0].vcard_uid, "u2");
    }

    #[test]
    fn test_get_and_delete() {
        let conn = test_db();
        upsert_contact(&conn, &test_contact("u1", "Max", "max@example.com")).unwrap();

        assert!(get_contact(&conn, "u1").unwrap().is_some());
        assert!(get_contact(&conn, "missing").unwrap().is_none());

        delete_contact(&conn, "u1").unwrap();
        assert!(get_contact(&conn, "u1").unwrap().is_none());
    }

    #[test]
    fn test_parse_address_full() {
        let (name, email) = parse_address("\"Max Mustermann\" <max@example.com>");
        assert_eq!(name.as_deref(), Some("Max Mustermann"));
        assert_eq!(email.as_deref(), Some("max@example.com"));
    }

    #[test]
    fn test_parse_address_email_only() {
        let (name, email) = parse_address("max@example.com");
        assert!(name.is_none());
        assert_eq!(email.as_deref(), Some("max@example.com"));
    }

    #[test]
    fn test_parse_address_name_only() {
        let (name, email) = parse_address("Max Mustermann");
        assert_eq!(name.as_deref(), Some("Max Mustermann"));
        assert!(email.is_none());
    }

    #[test]
    fn test_enrich_from_envelope() {
        let conn = test_db();
        let n = enrich_from_envelope(
            &conn,
            "\"Max Mustermann\" <max@example.com>",
            "erika@example.com, \"Erika\" <erika@example.com>",
            "",
        ).unwrap();
        // Sender (max) + named recipient (erika) = 2 (plain recipient skipped).
        assert_eq!(n, 2);
        assert!(get_contact(&conn, "mail:max@example.com").unwrap().is_some());
        assert!(get_contact(&conn, "mail:erika@example.com").unwrap().is_some());
        // Re-enriching is idempotent (no duplicate rows).
        let _ = enrich_from_envelope(&conn, "\"Max\" <max@example.com>", "", "").unwrap();
        let all = list_contacts(&conn, "max").unwrap();
        assert_eq!(all.len(), 1);
    }
}
