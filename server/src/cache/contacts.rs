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
    list_contacts_aus(conn, search, Quelle::Alle)
}

/// Where a contact comes from (Kai, 9.10.2026, the contacts' left column):
/// the address book (CardDAV, or made in Relay), one of its lists, or
/// collected from mail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Quelle {
    Alle,
    Adressbuch,
    Mail,
    Liste(String),
}

impl Quelle {
    pub fn lesen(s: &str) -> Self {
        match s {
            "adressbuch" => Quelle::Adressbuch,
            "mail" => Quelle::Mail,
            _ => match s.strip_prefix("liste:") {
                Some(uid) if !uid.is_empty() => Quelle::Liste(uid.to_string()),
                _ => Quelle::Alle,
            },
        }
    }

    /// The condition on `contacts`. "Alle" is the address book, as "Alle
    /// Kontakte" on the iPhone (Kai, 10.10.2026): senders collected from mail
    /// stand apart under "Weitere Kontakte". A collected sender whose address
    /// is in the address book is the same person: it is left out. A list
    /// holds the contacts whose UID it names (`?2`).
    fn bedingung(&self) -> &'static str {
        match self {
            Quelle::Alle | Quelle::Adressbuch => "source != 'mail'",
            Quelle::Mail => GESAMMELT_EIGEN,
            Quelle::Liste(_) => "vcard_uid IN (SELECT value FROM kontakt_listen, json_each(kontakt_listen.mitglieder) WHERE kontakt_listen.vcard_uid = ?2)",
        }
    }
}

const GESAMMELT_EIGEN: &str = "(source = 'mail' AND NOT EXISTS (SELECT 1 FROM contacts b \
    WHERE b.source != 'mail' AND b.email IS NOT NULL AND lower(b.email) = lower(contacts.email)))";

/// The contacts of one source, searched as `list_contacts`.
pub fn list_contacts_aus(conn: &Connection, search: &str, quelle: Quelle) -> Result<Vec<ContactRow>, String> {
    let like = format!("%{}%", search.to_lowercase());
    let sql = format!(
        r#"
        SELECT vcard_uid, given_name, family_name, display_name, email, phone,
               organization, source, synced_at
        FROM contacts
        WHERE (lower(display_name) LIKE ?1
           OR lower(coalesce(email,'')) LIKE ?1
           OR lower(coalesce(given_name,'')) LIKE ?1
           OR lower(coalesce(family_name,'')) LIKE ?1)
          AND {}
        ORDER BY coalesce(display_name, given_name, email, '') COLLATE NOCASE
    "#,
        // Searching all contacts finds the collected ones too, as the
        // iPhone's search shows suggestions from mail below the address book.
        if quelle == Quelle::Alle && !search.trim().is_empty() {
            format!("(source != 'mail' OR {})", GESAMMELT_EIGEN)
        } else {
            quelle.bedingung().to_string()
        }
    );
    let liste = match &quelle {
        Quelle::Liste(uid) => uid.clone(),
        _ => String::new(),
    };
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = if matches!(quelle, Quelle::Liste(_)) {
        stmt.query_map(rusqlite::params![like, liste], row_to_contact_row)
    } else {
        stmt.query_map(rusqlite::params![like], row_to_contact_row)
    }
    .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// How many contacts each source holds, for the lists.
#[derive(serde::Serialize, Debug, PartialEq)]
pub struct Zahlen {
    pub alle: i64,
    pub adressbuch: i64,
    pub mail: i64,
    pub listen: Vec<ListeZahl>,
}

/// One list of the address book with how many of its members Relay knows.
#[derive(serde::Serialize, Debug, PartialEq)]
pub struct ListeZahl {
    pub uid: String,
    pub name: String,
    pub zahl: i64,
}

pub fn zahlen(conn: &Connection) -> Result<Zahlen, String> {
    let zaehle = |q: Quelle| -> Result<i64, String> {
        conn.query_row(&format!("SELECT COUNT(*) FROM contacts WHERE {}", q.bedingung()), [], |r| r.get(0))
            .map_err(|e| e.to_string())
    };
    let mut stmt = conn
        .prepare(
            "SELECT l.vcard_uid, l.name,
                    (SELECT COUNT(*) FROM json_each(l.mitglieder) m JOIN contacts c ON c.vcard_uid = m.value)
             FROM kontakt_listen l ORDER BY l.name COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;
    let listen = stmt
        .query_map([], |r| Ok(ListeZahl { uid: r.get(0)?, name: r.get(1)?, zahl: r.get(2)? }))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(Zahlen { alle: zaehle(Quelle::Alle)?, adressbuch: zaehle(Quelle::Adressbuch)?, mail: zaehle(Quelle::Mail)?, listen })
}

/// Keep a list from the address book (replaces one with the same UID).
pub fn liste_speichern(conn: &Connection, g: &crate::dav::vcard::Gruppe, raw: &str) -> Result<(), String> {
    let mitglieder = serde_json::to_string(&g.mitglieder).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO kontakt_listen (vcard_uid, name, mitglieder, vcard_raw) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(vcard_uid) DO UPDATE SET name = excluded.name, mitglieder = excluded.mitglieder, vcard_raw = excluded.vcard_raw",
        rusqlite::params![g.uid, g.name, mitglieder, raw],
    )
    .map_err(|e| e.to_string())?;
    // A list that came in as a contact before goes from the contacts.
    conn.execute("DELETE FROM contacts WHERE vcard_uid = ?1", rusqlite::params![g.uid])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Lists stored as contacts before 26.10.25 move to `kontakt_listen`.
pub fn listen_aus_kontakten_loesen(conn: &Connection) -> Result<usize, String> {
    let mut stmt = conn
        .prepare("SELECT vcard_raw FROM contacts WHERE source != 'mail' AND (vcard_raw LIKE '%KIND:group%' OR vcard_raw LIKE '%kind:group%')")
        .map_err(|e| e.to_string())?;
    let rohe: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();
    let mut n = 0;
    for raw in rohe {
        if let Some(g) = crate::dav::vcard::gruppe(&raw) {
            liste_speichern(conn, &g, &raw)?;
            n += 1;
        }
    }
    Ok(n)
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
            source = 'carddav',
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
    // M4 (Review 2026-09-14): envelope names arrive raw-RFC2047-encoded
    // ("=?utf-8?B?...?=") — decode them before storing/displaying.
    let decode_name = |s: String| -> String { crate::imap::client::decode_rfc2047(&s) };
    let (name, email) = match (lt, gt) {
        (Some(l), Some(g)) if g > l => {
            let name_part = decode_name(
                trimmed[..l].trim().trim_matches('"').trim().to_string(),
            );
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
            let name = if trimmed.contains('@') { None } else { Some(decode_name(trimmed.to_string())) };
            (name, email)
        }
    };
    (name, email)
}

/// The addresses of the user's own accounts, lower case.
fn eigene_adressen(conn: &Connection) -> Result<std::collections::HashSet<String>, String> {
    let mut stmt = conn
        .prepare("SELECT sender_email, username FROM accounts")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut out = std::collections::HashSet::new();
    for r in rows.flatten() {
        for a in [r.0, r.1] {
            let a = a.trim().to_lowercase();
            if a.contains('@') {
                out.insert(a);
            }
        }
    }
    Ok(out)
}

/// Addresses no person reads: no-reply, bounces, newsletters, notices.
/// They never become a contact, even when the user wrote to one.
pub fn automatisch(email: &str) -> bool {
    let lokal = email.split('@').next().unwrap_or("").to_lowercase();
    const MUSTER: &[&str] = &[
        "noreply", "no-reply", "no_reply", "donotreply", "do-not-reply", "do_not_reply",
        "mailer-daemon", "postmaster", "bounce", "newsletter", "notification",
        "benachrichtigung", "unsubscribe",
    ];
    MUSTER.iter().any(|m| lokal.contains(m))
}

/// "Weitere Kontakte" as Google keeps them (Kai, 10.10.2026): the people
/// the user wrote to. A mail counts when it comes from one of the user's
/// own addresses; its recipients (To and Cc) are collected. Senders who
/// only wrote to the user — shops, newsletters, notices — are not.
/// Returns the number of contacts upserted.
pub fn enrich_from_envelope(
    conn: &Connection,
    from: &str,
    to: &str,
    cc: &str,
) -> Result<usize, String> {
    let eigene = eigene_adressen(conn)?;
    let von_mir = parse_address(from)
        .1
        .map(|e| eigene.contains(&e.trim().to_lowercase()))
        .unwrap_or(false);
    if !von_mir {
        return Ok(0);
    }
    let empfaenger: Vec<&str> = to.split(',').chain(cc.split(',')).collect();
    aus_eigener_mail(conn, &empfaenger, &eigene)
}

/// Collect the recipients of a mail the user wrote (also called when Relay
/// sends one). The user's own addresses and automatic ones are left out.
pub fn aus_eigener_mail(
    conn: &Connection,
    empfaenger: &[&str],
    eigene: &std::collections::HashSet<String>,
) -> Result<usize, String> {
    let mut count = 0;
    for addr in empfaenger {
        let (name, email) = parse_address(addr);
        let Some(email) = email else { continue };
        let klein = email.trim().to_lowercase();
        if !klein.contains('@') || eigene.contains(&klein) || automatisch(&klein) {
            continue;
        }
        if upsert_enriched(conn, &klein, name.as_deref().unwrap_or("")).is_ok() {
            count += 1;
        }
    }
    Ok(count)
}

/// After a send from Relay: its recipients become "Weitere Kontakte".
pub fn nach_dem_senden(conn: &Connection, empfaenger: &[String]) -> Result<usize, String> {
    let eigene = eigene_adressen(conn)?;
    let liste: Vec<&str> = empfaenger.iter().map(String::as_str).collect();
    aus_eigener_mail(conn, &liste, &eigene)
}

/// Build "Weitere Kontakte" anew from the stored mails (26.10.25): every
/// sender collected so far goes, the recipients of the user's own mails
/// come back. Contacts the user saved stay untouched.
pub fn weitere_neu_aufbauen(conn: &Connection) -> Result<usize, String> {
    conn.execute("DELETE FROM contacts WHERE source = 'mail'", [])
        .map_err(|e| e.to_string())?;
    let eigene = eigene_adressen(conn)?;
    if eigene.is_empty() {
        return Ok(0);
    }
    let mut stmt = conn
        .prepare("SELECT coalesce(from_addr,''), coalesce(to_addr,''), coalesce(cc_addr,'') FROM messages")
        .map_err(|e| e.to_string())?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();
    let mut n = 0;
    for (from, to, cc) in rows {
        let von_mir = parse_address(&from)
            .1
            .map(|e| eigene.contains(&e.trim().to_lowercase()))
            .unwrap_or(false);
        if von_mir {
            let empfaenger: Vec<&str> = to.split(',').chain(cc.split(',')).collect();
            n += aus_eigener_mail(conn, &empfaenger, &eigene)?;
        }
    }
    Ok(n)
}

/// Upsert a contact derived from mail (lowercase email as UID, source='mail').
fn upsert_enriched(conn: &Connection, email: &str, name: &str) -> Result<(), String> {
    let uid = format!("mail:{}", email.to_lowercase());
    conn.execute(
        "INSERT INTO contacts (vcard_uid, display_name, email, vcard_raw, source, synced_at)
         VALUES (?1, NULLIF(?2, ''), ?3, ?4, 'mail', datetime('now'))
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

    /// One account with the user's own address, as the mail accounts are.
    fn mit_konto(conn: &Connection) {
        conn.execute(
            "INSERT INTO accounts (name, imap_host, smtp_host, username, password, sender_email)
             VALUES ('Ich', 'h', 'h', 'ich@example.com', 'p', 'Ich@Example.com')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn weitere_nur_wem_ich_geschrieben_habe() {
        let conn = test_db();
        mit_konto(&conn);
        // A mail to the user: the sender is not collected (Kai, 10.10.2026).
        let n = enrich_from_envelope(&conn, "\"Shop\" <info@shop.example>", "ich@example.com", "").unwrap();
        assert_eq!(n, 0);
        // A mail from the user: its recipients are, also without a name;
        // the user's own address and no-reply ones are not.
        let n = enrich_from_envelope(
            &conn,
            "\"Ich\" <ich@example.com>",
            "\"Erika\" <Erika@example.com>, max@example.com, noreply@shop.example",
            "ich@example.com",
        )
        .unwrap();
        assert_eq!(n, 2);
        assert!(get_contact(&conn, "mail:erika@example.com").unwrap().is_some());
        assert!(get_contact(&conn, "mail:max@example.com").unwrap().is_some());
        assert!(get_contact(&conn, "mail:info@shop.example").unwrap().is_none());
        assert!(get_contact(&conn, "mail:ich@example.com").unwrap().is_none());
        // Again: no duplicate rows.
        enrich_from_envelope(&conn, "ich@example.com", "Erika <erika@example.com>", "").unwrap();
        assert_eq!(list_contacts(&conn, "erika").unwrap().len(), 1);
        // After a send from Relay, too.
        nach_dem_senden(&conn, &["\"Jonas\" <jonas@example.com>".to_string()]).unwrap();
        assert!(get_contact(&conn, "mail:jonas@example.com").unwrap().is_some());
        assert!(automatisch("Mailer-Daemon@x.de") && automatisch("news.noreply@x.de") && !automatisch("erika@x.de"));
    }

    #[test]
    fn weitere_aus_den_mails_neu_aufgebaut() {
        let conn = test_db();
        mit_konto(&conn);
        conn.execute("INSERT INTO folders (account_id, name) VALUES (1, 'INBOX')", []).unwrap();
        let mail = |uid: i64, von: &str, an: &str| {
            conn.execute(
                "INSERT INTO messages (account_id, folder_id, uid, from_addr, to_addr) VALUES (1, 1, ?1, ?2, ?3)",
                rusqlite::params![uid, von, an],
            )
            .unwrap();
        };
        mail(1, "\"About You\" <newsletter@aboutyou.de>", "ich@example.com");
        mail(2, "ich@example.com", "\"Clara\" <clara@example.com>");
        // What the old rule collected goes; a saved contact stays.
        conn.execute(
            "INSERT INTO contacts (vcard_uid, display_name, email, vcard_raw, source) VALUES ('mail:info@ebay.de', 'eBay', 'info@ebay.de', '', 'mail')",
            [],
        )
        .unwrap();
        upsert_contact(&conn, &test_contact("u1", "Max", "max@example.com")).unwrap();
        assert_eq!(weitere_neu_aufbauen(&conn).unwrap(), 1);
        let weitere: Vec<String> = list_contacts_aus(&conn, "", Quelle::Mail).unwrap().into_iter().map(|c| c.vcard_uid).collect();
        assert_eq!(weitere, vec!["mail:clara@example.com"]);
        assert!(get_contact(&conn, "u1").unwrap().is_some());
    }

    #[test]
    fn listen_aus_dem_adressbuch() {
        let conn = test_db();
        upsert_contact(&conn, &test_contact("A-1", "Dennis", "d@example.com")).unwrap();
        upsert_contact(&conn, &test_contact("B-2", "Leo", "l@example.com")).unwrap();
        // Before 26.10.25 the list came in as a contact named "Fußball".
        let raw = "BEGIN:VCARD\nFN:Fußball\nX-ADDRESSBOOKSERVER-KIND:group\nX-ADDRESSBOOKSERVER-MEMBER:urn:uuid:A-1\nX-ADDRESSBOOKSERVER-MEMBER:urn:uuid:Z-9\nUID:G-1\nEND:VCARD";
        conn.execute(
            "INSERT INTO contacts (vcard_uid, display_name, vcard_raw) VALUES ('G-1', 'Fußball', ?1)",
            [raw],
        )
        .unwrap();
        assert_eq!(listen_aus_kontakten_loesen(&conn).unwrap(), 1);
        assert!(get_contact(&conn, "G-1").unwrap().is_none(), "kein Kontakt mehr");
        let z = zahlen(&conn).unwrap();
        assert_eq!(z.alle, 2);
        // Z-9 is not known here: only members Relay has count.
        assert_eq!(z.listen, vec![ListeZahl { uid: "G-1".into(), name: "Fußball".into(), zahl: 1 }]);
        let in_liste: Vec<String> = list_contacts_aus(&conn, "", Quelle::lesen("liste:G-1"))
            .unwrap().into_iter().map(|c| c.vcard_uid).collect();
        assert_eq!(in_liste, vec!["A-1"]);
    }

    #[test]
    fn quellen_trennen_adressbuch_und_gesammelte() {
        let conn = test_db();
        mit_konto(&conn);
        upsert_contact(&conn, &test_contact("u1", "Max", "Max@example.com")).unwrap();
        // The user wrote to Max too: the same person, not shown twice.
        enrich_from_envelope(&conn, "ich@example.com", "\"Max\" <max@example.com>", "").unwrap();
        enrich_from_envelope(&conn, "ich@example.com", "\"Shop\" <info@shop.example>", "").unwrap();

        let uids = |q: Quelle| -> Vec<String> {
            list_contacts_aus(&conn, "", q).unwrap().into_iter().map(|c| c.vcard_uid).collect()
        };
        assert_eq!(uids(Quelle::Adressbuch), vec!["u1"]);
        assert_eq!(uids(Quelle::Mail), vec!["mail:info@shop.example"]);
        // "Alle Kontakte" is the address book; a search finds the others too.
        assert_eq!(uids(Quelle::Alle), vec!["u1"]);
        let gefunden: Vec<String> = list_contacts_aus(&conn, "example", Quelle::Alle).unwrap()
            .into_iter().map(|c| c.vcard_uid).collect();
        assert_eq!(gefunden, vec!["u1", "mail:info@shop.example"]);
        assert_eq!(zahlen(&conn).unwrap(), Zahlen { alle: 1, adressbuch: 1, mail: 1, listen: vec![] });
        assert_eq!(Quelle::lesen("adressbuch"), Quelle::Adressbuch);
        assert_eq!(Quelle::lesen("x"), Quelle::Alle);

        // Saved in Relay, a collected sender goes into the address book.
        upsert_contact(&conn, &test_contact("mail:info@shop.example", "Shop", "info@shop.example")).unwrap();
        assert_eq!(zahlen(&conn).unwrap(), Zahlen { alle: 2, adressbuch: 2, mail: 0, listen: vec![] });
    }

    #[test]
    fn test_parse_address_rfc2047_name() {
        // M4: encoded display names (mobile/desktop review 2026-09-14) must be
        // decoded, not stored as raw RFC2047 words.
        let (name, email) = parse_address("=?UTF-8?B?TcO8bGxlcg==?= <m@example.com>");
        assert_eq!(email.as_deref(), Some("m@example.com"));
        assert_eq!(name.as_deref(), Some("Müller"));
    }
}
