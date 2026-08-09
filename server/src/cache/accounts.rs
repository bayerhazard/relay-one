use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountRecord {
    pub id: i64,
    pub name: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_ssl: bool,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_tls: bool,
    pub username: String,
    pub smtp_username: String,
    pub sender_name: String,
    pub sender_email: String,
    pub sync_mode: String,
    pub trash_retention_days: i64,
}

/// Update per-account archive settings (sync mode / trash retention).
pub fn update_account_settings(
    conn: &Connection,
    account_id: i64,
    sync_mode: &str,
    trash_retention_days: i64,
) -> Result<(), rusqlite::Error> {
    let mode = if sync_mode == "archive" { "archive" } else { "mirror" };
    conn.execute(
        "UPDATE accounts SET sync_mode = ?1, trash_retention_days = ?2 WHERE id = ?3",
        params![mode, trash_retention_days, account_id],
    )?;
    Ok(())
}

pub fn create_account(
    conn: &Connection,
    name: &str,
    imap_host: &str,
    imap_port: u16,
    imap_ssl: bool,
    smtp_host: &str,
    smtp_port: u16,
    smtp_tls: bool,
    username: &str,
    password: &str,
    smtp_username: &str,
    smtp_password: &str,
    sender_name: &str,
    sender_email: &str,
) -> Result<i64, rusqlite::Error> {
    conn.execute(
        "INSERT INTO accounts (name, imap_host, imap_port, imap_ssl, smtp_host, smtp_port, smtp_tls, username, password, smtp_username, smtp_password, sender_name, sender_email)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![name, imap_host, imap_port, imap_ssl as i32, smtp_host, smtp_port, smtp_tls as i32, username, password, smtp_username, smtp_password, sender_name, sender_email],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn list_accounts(conn: &Connection) -> Result<Vec<AccountRecord>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, name, imap_host, imap_port, imap_ssl, smtp_host, smtp_port, smtp_tls, username, smtp_username, sender_name, sender_email, sync_mode, trash_retention_days
         FROM accounts ORDER BY id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(AccountRecord {
            id: row.get(0)?,
            name: row.get(1)?,
            imap_host: row.get(2)?,
            imap_port: row.get::<_, i32>(3)? as u16,
            imap_ssl: row.get::<_, i32>(4)? != 0,
            smtp_host: row.get(5)?,
            smtp_port: row.get::<_, i32>(6)? as u16,
            smtp_tls: row.get::<_, i32>(7)? != 0,
            username: row.get(8)?,
            smtp_username: row.get(9)?,
            sender_name: row.get(10)?,
            sender_email: row.get(11)?,
            sync_mode: row.get(12)?,
            trash_retention_days: row.get(13)?,
        })
    })?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

#[allow(dead_code)]
pub fn get_account(conn: &Connection, account_id: i64) -> Result<Option<AccountRecord>, rusqlite::Error> {
    match conn.query_row(
        "SELECT id, name, imap_host, imap_port, imap_ssl, smtp_host, smtp_port, smtp_tls, username, smtp_username, sender_name, sender_email, sync_mode, trash_retention_days
         FROM accounts WHERE id = ?1",
        params![account_id],
        |row| {
            Ok(AccountRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                imap_host: row.get(2)?,
                imap_port: row.get::<_, i32>(3)? as u16,
                imap_ssl: row.get::<_, i32>(4)? != 0,
                smtp_host: row.get(5)?,
                smtp_port: row.get::<_, i32>(6)? as u16,
                smtp_tls: row.get::<_, i32>(7)? != 0,
                username: row.get(8)?,
                smtp_username: row.get(9)?,
                sender_name: row.get(10)?,
                sender_email: row.get(11)?,
                sync_mode: row.get(12)?,
                trash_retention_days: row.get(13)?,
            })
        },
    ) {
        Ok(record) => Ok(Some(record)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

#[allow(dead_code)]
pub fn get_account_password(conn: &Connection, account_id: i64) -> Result<Option<String>, rusqlite::Error> {
    conn.query_row(
        "SELECT password FROM accounts WHERE id = ?1",
        params![account_id],
        |row| row.get(0),
    )
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

/// Returns (account_record, imap_password_encrypted, smtp_password_encrypted)
pub fn list_accounts_with_passwords(conn: &Connection) -> Result<Vec<(AccountRecord, String, String)>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, name, imap_host, imap_port, imap_ssl, smtp_host, smtp_port, smtp_tls, username, password, smtp_username, smtp_password, sender_name, sender_email, sync_mode, trash_retention_days
         FROM accounts ORDER BY id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            AccountRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                imap_host: row.get(2)?,
                imap_port: row.get::<_, i32>(3)? as u16,
                imap_ssl: row.get::<_, i32>(4)? != 0,
                smtp_host: row.get(5)?,
                smtp_port: row.get::<_, i32>(6)? as u16,
                smtp_tls: row.get::<_, i32>(7)? != 0,
                username: row.get(8)?,
                smtp_username: row.get(10)?,
                sender_name: row.get(12)?,
                sender_email: row.get(13)?,
                sync_mode: row.get(14)?,
                trash_retention_days: row.get(15)?,
            },
            row.get::<_, String>(9)?,
            row.get::<_, String>(11)?,
        ))
    })?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn delete_account(conn: &Connection, account_id: i64) -> Result<(), rusqlite::Error> {
    conn.execute_batch("BEGIN")?;
    conn.execute("DELETE FROM messages WHERE account_id = ?1", params![account_id])?;
    conn.execute("DELETE FROM folders WHERE account_id = ?1", params![account_id])?;
    conn.execute("DELETE FROM contact_profiles WHERE account_id = ?1", params![account_id])?;
    conn.execute("DELETE FROM accounts WHERE id = ?1", params![account_id])?;
    conn.execute_batch("COMMIT")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::db;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        conn
    }

    fn create_test_account(conn: &Connection, id_suffix: &str) -> i64 {
        create_account(
            conn,
            &format!("Test {}", id_suffix),
            &format!("imap{}.test.com", id_suffix),
            993, true, &format!("smtp{}.test.com", id_suffix),
            587, true, &format!("user{}@test.com", id_suffix),
            &format!("pass{}", id_suffix),
            &format!("smtp_user{}@test.com", id_suffix),
            &format!("smtp_pass{}", id_suffix),
            &format!("Sender {}", id_suffix),
            &format!("sender{}@test.com", id_suffix),
        ).unwrap()
    }

    #[test]
    fn test_create_and_list_accounts() {
        let conn = setup_db();
        create_test_account(&conn, "1");
        create_test_account(&conn, "2");
        let list = list_accounts(&conn).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Test 1");
        assert_eq!(list[1].name, "Test 2");
        assert_eq!(list[0].smtp_username, "smtp_user1@test.com");
    }

    #[test]
    fn test_list_accounts_with_passwords() {
        let conn = setup_db();
        create_test_account(&conn, "1");
        create_test_account(&conn, "2");
        let list = list_accounts_with_passwords(&conn).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].1, "pass1");
        assert_eq!(list[0].2, "smtp_pass1");
        assert_eq!(list[0].0.name, "Test 1");
        assert_eq!(list[0].0.smtp_username, "smtp_user1@test.com");
    }

    #[test]
    fn test_get_account_password() {
        let conn = setup_db();
        let id = create_test_account(&conn, "pw");
        let pw = get_account_password(&conn, id).unwrap();
        assert_eq!(pw, Some("passpw".into()));
    }

    #[test]
    fn test_get_account_password_nonexistent() {
        let conn = setup_db();
        assert_eq!(get_account_password(&conn, 999).unwrap(), None);
    }

    #[test]
    fn test_delete_account() {
        let conn = setup_db();
        let id = create_test_account(&conn, "del");
        assert_eq!(list_accounts(&conn).unwrap().len(), 1);
        delete_account(&conn, id).unwrap();
        assert_eq!(list_accounts(&conn).unwrap().len(), 0);
    }

    #[test]
    fn test_empty_list_returns_empty_vec() {
        let conn = setup_db();
        assert!(list_accounts(&conn).unwrap().is_empty());
        assert!(list_accounts_with_passwords(&conn).unwrap().is_empty());
    }
}