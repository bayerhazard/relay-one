use chrono::Utc;
use rusqlite::{params, Connection};

use crate::cache::accounts::{
    create_account, delete_account, get_account, list_accounts,
};
use crate::cache::db;
use crate::cache::messages::{delete_message, delete_messages_not_in, fetch_inbox, fetch_message_body, save_message};
use crate::imap::types::{CachedMessage, MailEnvelope};
use base64::Engine as _;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    db::init_db(&conn).unwrap();
    conn
}

fn create_test_account(conn: &Connection, suffix: &str) -> i64 {
    create_account(
        conn,
        &format!("Test {}", suffix),
        &format!("imap{}.example.com", suffix),
        993,
        true,
        &format!("smtp{}.example.com", suffix),
        587,
        true,
        &format!("user{}@example.com", suffix),
        &format!("secret{}", suffix),
        &format!("user{}@example.com", suffix),
        &format!("secret_smtp{}", suffix),
        &format!("Sender {}", suffix),
        &format!("sender{}@example.com", suffix),
        false,
    )
    .unwrap()
}

fn make_cached_message(uid: u32, subject: &str, from: &str, body: &str) -> CachedMessage {
    CachedMessage {
        uid,
        envelope: MailEnvelope {
            subject: subject.into(),
            from: from.into(),
            to: "recipient@example.com".into(),
            date: "2025-06-10T10:00:00Z".into(),
            message_id: format!("<msg{}@example.com>", uid),
        },
        flags: vec!["\\Seen".into()],
        body_preview: Some(body.into()),
        body_structure: None,
        ai_summary: None,
        ai_priority: None,
        ai_fraud_score: None,
        cached_at: Utc::now(),
        updated_at: Utc::now(),
   is_read: false,
        is_flagged: false,
        has_attachments: false,
        attachments: vec![],
    }
}

// ===========================================================================
// Account CRUD
// ===========================================================================

#[test]
fn test_create_and_read_account() {
    let conn = setup_db();
    let id = create_test_account(&conn, "cr");

    let record = get_account(&conn, id).unwrap().expect("account should exist");
    assert_eq!(record.name, "Test cr");
    assert_eq!(record.imap_host, "imapcr.example.com");
    assert_eq!(record.imap_port, 993);
    assert!(record.imap_ssl);
    assert_eq!(record.smtp_host, "smtpcr.example.com");
    assert_eq!(record.smtp_port, 587);
    assert!(record.smtp_tls);
    assert_eq!(record.username, "usercr@example.com");
    assert_eq!(record.smtp_username, "usercr@example.com");
    assert_eq!(record.sender_name, "Sender cr");
    assert_eq!(record.sender_email, "sendercr@example.com");
}

#[test]
fn test_list_accounts_returns_all() {
    let conn = setup_db();
    let id1 = create_test_account(&conn, "a");
    let id2 = create_test_account(&conn, "b");

    let list = list_accounts(&conn).unwrap();
    assert_eq!(list.len(), 2);
    // Ordered by id ascending
    assert_eq!(list[0].id, id1);
    assert_eq!(list[1].id, id2);
}

#[test]
fn test_get_nonexistent_account_returns_none() {
    let conn = setup_db();
    assert!(get_account(&conn, 999).unwrap().is_none());
}

#[test]
fn test_update_account_fields() {
    let conn = setup_db();
    let id = create_test_account(&conn, "up");

    // Update sender_name and imap_host via raw SQL (no public update fn exists)
    conn.execute(
        "UPDATE accounts SET sender_name = ?1, imap_host = ?2 WHERE id = ?3",
        rusqlite::params!["Updated Name", "imap-new.example.com", id],
    )
    .unwrap();

    let record = get_account(&conn, id).unwrap().expect("should exist after update");
    assert_eq!(record.sender_name, "Updated Name");
    assert_eq!(record.imap_host, "imap-new.example.com");
    // Unchanged fields preserved
    assert_eq!(record.name, "Test up");
    assert_eq!(record.username, "userup@example.com");
}

#[test]
fn test_delete_account_removes_it() {
    let conn = setup_db();
    let id = create_test_account(&conn, "del");
    assert_eq!(list_accounts(&conn).unwrap().len(), 1);

    delete_account(&conn, id).unwrap();
    assert_eq!(list_accounts(&conn).unwrap().len(), 0);
    assert!(get_account(&conn, id).unwrap().is_none());
}

#[test]
fn test_delete_account_cascades_to_folders_and_messages() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "cascade");

    // Save a message (creates a folder + message row)
    let msg = make_cached_message(1, "Cascade test", "from@test.com", "body");
    save_message(&conn, account_id, &msg, "INBOX").unwrap();

    // Verify message and folder exist
    assert!(fetch_message_body(&conn, account_id, 1, None).unwrap().is_some());
    let folder_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM folders WHERE account_id = ?1",
            rusqlite::params![account_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(folder_count, 1, "folder should exist before delete");

    // Delete account
    delete_account(&conn, account_id).unwrap();

    // Verify cascade: messages gone
    assert!(fetch_message_body(&conn, account_id, 1, None).unwrap().is_none());
    // Verify cascade: folders gone
    let folder_count_after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM folders WHERE account_id = ?1",
            rusqlite::params![account_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(folder_count_after, 0, "folder should be cascaded after delete");
}

// ===========================================================================
// Message Lifecycle
// ===========================================================================

#[test]
fn test_save_and_fetch_message() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "msg");

    let msg = make_cached_message(42, "Hello World", "alice@example.com", "This is the body.");
    save_message(&conn, account_id, &msg, "INBOX").unwrap();

    let inbox = fetch_inbox(&conn, account_id, Some(10), Some(0), "INBOX").unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].uid, 42);
    assert_eq!(inbox[0].subject.as_deref(), Some("Hello World"));
    assert_eq!(inbox[0].from_addr.as_deref(), Some("alice@example.com"));
    assert_eq!(inbox[0].body_text.as_deref(), Some("This is the body."));
    assert!(inbox[0].synced);
}

#[test]
fn test_fetch_message_body_by_uid() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "body");

    let msg = make_cached_message(7, "Subject", "bob@example.com", "Detailed body text.");
    save_message(&conn, account_id, &msg, "INBOX").unwrap();

    let fetched = fetch_message_body(&conn, account_id, 7, None)
        .unwrap()
        .expect("message should exist");
    assert_eq!(fetched.uid, 7);
    assert_eq!(fetched.subject.as_deref(), Some("Subject"));
    assert_eq!(fetched.from_addr.as_deref(), Some("bob@example.com"));
    assert_eq!(fetched.body_text.as_deref(), Some("Detailed body text."));
}

#[test]
fn test_fetch_nonexistent_message_body_returns_none() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "nonexist");
    assert!(fetch_message_body(&conn, account_id, 999, None).unwrap().is_none());
}

#[test]
fn test_delete_message_removes_from_cache() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "delmsg");

    let msg = make_cached_message(10, "To delete", "spam@example.com", "Spam content.");
    save_message(&conn, account_id, &msg, "INBOX").unwrap();
    assert!(fetch_message_body(&conn, account_id, 10, None).unwrap().is_some());

    delete_message(&conn, account_id, 10).unwrap();
    assert!(
        fetch_message_body(&conn, account_id, 10, None).unwrap().is_none(),
        "deleted message should not be found"
    );
  // Verify inbox no longer lists it
    let inbox = fetch_inbox(&conn, account_id, Some(10), Some(0), "INBOX").unwrap();
    assert!(inbox.is_empty(), "inbox should be empty after deleting the only message");
  }

  // Regression test: Messages deleted on IMAP server (e.g., in GMX Webmail)
  // should be removed from local cache during sync cleanup.
  #[test]
  fn test_delete_messages_not_in_removes_deleted_imap_messages() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "cleanup");

    // Simulate 3 messages in local cache
    let msg1 = make_cached_message(1, "Msg 1", "sender@example.com", "body 1");
    let msg2 = make_cached_message(2, "Msg 2", "sender@example.com", "body 2");
    let msg3 = make_cached_message(3, "Msg 3", "sender@example.com", "body 3");
    save_message(&conn, account_id, &msg1, "INBOX").unwrap();
    save_message(&conn, account_id, &msg2, "INBOX").unwrap();
    save_message(&conn, account_id, &msg3, "INBOX").unwrap();

    // Verify all 3 are in cache
    let inbox = fetch_inbox(&conn, account_id, Some(10), Some(0), "INBOX").unwrap();
    assert_eq!(inbox.len(), 3, "should have 3 messages initially");

    // Verify folder exists
    let folder_id: i64 = conn.query_row(
        "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
        params![account_id, "INBOX"],
        |row| row.get(0),
    ).expect("folder should exist");
    assert!(folder_id > 0, "folder_id should be positive");

    // Simulate IMAP server state: Msg 2 was deleted externally (e.g., GMX Webmail)
    // Server only has UIDs 1 and 3
    let server_uids = vec![1, 3];

    // Run cleanup - should delete Msg 2
    let deleted = delete_messages_not_in(&conn, account_id, "INBOX", &server_uids).unwrap();
    assert_eq!(deleted, 1, "should have deleted exactly 1 message");

    // Verify only Msg 1 and Msg 3 remain
    let inbox = fetch_inbox(&conn, account_id, Some(10), Some(0), "INBOX").unwrap();
    assert_eq!(inbox.len(), 2, "should have 2 messages after cleanup");
    let uids: Vec<i64> = inbox.iter().map(|m| m.uid).collect();
    assert!(uids.contains(&1), "Msg 1 should still exist");
    assert!(uids.contains(&3), "Msg 3 should still exist");
    assert!(!uids.contains(&2), "Msg 2 should be deleted");
  }

  // Edge case: All messages deleted on server should clear local cache
  #[test]
  fn test_delete_messages_not_in_clears_all_when_server_empty() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "empty");

    // Save 2 messages
    let msg1 = make_cached_message(1, "Msg 1", "sender@example.com", "body 1");
    let msg2 = make_cached_message(2, "Msg 2", "sender@example.com", "body 2");
    save_message(&conn, account_id, &msg1, "INBOX").unwrap();
    save_message(&conn, account_id, &msg2, "INBOX").unwrap();

    // Server has no messages (all deleted)
    let server_uids: Vec<u32> = vec![];

    let deleted = delete_messages_not_in(&conn, account_id, "INBOX", &server_uids).unwrap();
    assert_eq!(deleted, 2, "should have deleted all 2 messages");

    let inbox = fetch_inbox(&conn, account_id, Some(10), Some(0), "INBOX").unwrap();
    assert!(inbox.is_empty(), "inbox should be empty after server cleanup");
  }

  #[test]
  fn test_save_multiple_messages_and_list() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "multi");

    for uid in 1..=5 {
        let msg = make_cached_message(uid, &format!("Subject {}", uid), "multi@test.com", "body");
        save_message(&conn, account_id, &msg, "INBOX").unwrap();
    }

    let inbox = fetch_inbox(&conn, account_id, Some(10), Some(0), "INBOX").unwrap();
    assert_eq!(inbox.len(), 5);
    // Ordered by date DESC — all have same date, so order is insertion-order dependent
    let uids: Vec<i64> = inbox.iter().map(|m| m.uid).collect();
    assert!(uids.contains(&1));
    assert!(uids.contains(&5));
}

#[test]
fn test_message_upsert_updates_existing() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "upsert");

    // Insert first version
    let msg1 = make_cached_message(1, "Original", "alice@test.com", "Original body.");
    save_message(&conn, account_id, &msg1, "INBOX").unwrap();

    // Insert second version with same uid — ON CONFLICT should update
    let msg2 = CachedMessage {
        uid: 1,
        envelope: MailEnvelope {
            subject: "Updated".into(),
            from: "alice@test.com".into(),
            to: "recipient@example.com".into(),
            date: "2025-06-10T12:00:00Z".into(),
            message_id: "<msg1@example.com>".into(),
        },
        flags: vec!["\\Seen".into(), "\\Flagged".into()],
        body_preview: Some("Updated body.".into()),
        body_structure: None,
        ai_summary: Some("AI summary".into()),
        ai_priority: Some(0.9),
        ai_fraud_score: Some(0.1),
        cached_at: Utc::now(),
        updated_at: Utc::now(),
   is_read: true,
        is_flagged: true,
        has_attachments: false,
        attachments: vec![],
    };
    save_message(&conn, account_id, &msg2, "INBOX").unwrap();

    let fetched = fetch_message_body(&conn, account_id, 1, None)
        .unwrap()
        .expect("message should exist");
    assert_eq!(fetched.subject.as_deref(), Some("Updated"));
    assert_eq!(fetched.body_text.as_deref(), Some("Updated body."));
    assert_eq!(fetched.ai_summary.as_deref(), Some("AI summary"));
    assert!((fetched.ai_priority.unwrap() - 0.9).abs() < f32::EPSILON);
    assert!((fetched.ai_fraud_score.unwrap() - 0.1).abs() < f32::EPSILON);
    assert!(fetched.is_read);
}

#[test]
fn test_fetch_inbox_pagination() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "page");

    for uid in 1..=10 {
        let msg = make_cached_message(uid, &format!("Subj {}", uid), "page@test.com", "body");
        save_message(&conn, account_id, &msg, "INBOX").unwrap();
    }

    let page1 = fetch_inbox(&conn, account_id, Some(3), Some(0), "INBOX").unwrap();
    assert_eq!(page1.len(), 3);

    let page2 = fetch_inbox(&conn, account_id, Some(3), Some(3), "INBOX").unwrap();
    assert_eq!(page2.len(), 3);

    let page_all = fetch_inbox(&conn, account_id, Some(100), Some(0), "INBOX").unwrap();
    assert_eq!(page_all.len(), 10);
}

// ===========================================================================
// Cache Consistency
// ===========================================================================

#[test]
fn test_savepoint_rollback_prevents_partial_save() {
    // Simulates an IMAP-success + cache-failure scenario:
    // The save_message function uses SAVEPOINT internally. If we corrupt
    // the DB mid-save (e.g., by dropping a required table), the rollback
    // should leave no partial state.
    let conn = setup_db();
    let account_id = create_test_account(&conn, "rollback");

    // Save one message successfully first
    let msg = make_cached_message(1, "Good", "alice@test.com", "Body.");
    save_message(&conn, account_id, &msg, "INBOX").unwrap();
    assert!(fetch_message_body(&conn, account_id, 1, None).unwrap().is_some());

    // Now simulate a failure: drop the folders table so the next save_message
    // will fail inside the SAVEPOINT (folder lookup fails).
    conn.execute_batch("DROP TABLE folders").unwrap();

    let msg2 = make_cached_message(2, "Bad", "bob@test.com", "Should not appear.");
    let result = save_message(&conn, account_id, &msg2, "INBOX");
    assert!(result.is_err(), "save should fail after dropping folders table");

    // Verify the first message is still intact (the failed save didn't corrupt it)
    // Re-create folders table to query messages
    conn.execute_batch(
        "CREATE TABLE folders (id INTEGER PRIMARY KEY AUTOINCREMENT, account_id INTEGER NOT NULL, name TEXT NOT NULL);
         INSERT INTO folders (id, account_id, name) VALUES (1, 1, 'INBOX');",
    )
    .unwrap();
    let msg1_after = fetch_message_body(&conn, account_id, 1, None)
        .unwrap()
        .expect("message 1 should survive the failed save");
    assert_eq!(msg1_after.subject.as_deref(), Some("Good"));
}

#[test]
fn test_cache_survives_imap_failure_scenario() {
    // Simulates cache-success + IMAP-failure:
    // The message is saved to the local cache successfully, but the IMAP
    // server operation fails afterward. The message should remain in cache
    // so it can be retried or displayed.
    let conn = setup_db();
    let account_id = create_test_account(&conn, "imapfail");

    // Save message to cache (this is the "cache success" part)
    let msg = make_cached_message(100, "Cached OK", "remote@test.com", "Body saved locally.");
    save_message(&conn, account_id, &msg, "INBOX").unwrap();

    // Simulate IMAP failure: the message is in cache even though IMAP would fail
    // (no actual IMAP connection — we just verify the cache state)
    let fetched = fetch_message_body(&conn, account_id, 100, None)
        .unwrap()
        .expect("message should be in cache despite IMAP failure");
    assert_eq!(fetched.subject.as_deref(), Some("Cached OK"));
    assert_eq!(fetched.from_addr.as_deref(), Some("remote@test.com"));
    assert!(fetched.synced, "synced flag should be 1 after save_message");
}

#[test]
fn test_unique_constraint_allows_same_uid_in_different_folders() {
    // IMAP-UIDs are unique per folder, not per account: the same uid may
    // exist in INBOX and in another folder. The constraint is now
    // UNIQUE(account_id, folder_id, uid).
    let conn = setup_db();
    let account_id = create_test_account(&conn, "unique");

    let msg1 = make_cached_message(1, "First", "a@test.com", "Body 1");
    save_message(&conn, account_id, &msg1, "INBOX").unwrap();

    // Same uid in a DIFFERENT folder must be allowed.
    let msg2 = make_cached_message(1, "Other folder same uid", "b@test.com", "Body 2");
    save_message(&conn, account_id, &msg2, "SENT").unwrap();

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE account_id = ?1 AND uid = 1",
            rusqlite::params![account_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2, "same uid in two folders must both persist");

    // Direct INSERT with same account_id + folder_id + uid should fail.
    let dup_result = conn.execute(
        "INSERT INTO messages (account_id, folder_id, uid, message_id, subject, from_addr, to_addr, date, body_text, is_read, synced)
         VALUES (?1, (SELECT id FROM folders WHERE account_id = ?1 AND name = 'INBOX'), 1, '<dup@test.com>', 'Duplicate', 'b@test.com', 'c@test.com', '2025-01-01', 'dup body', 0, 1)",
        rusqlite::params![account_id],
    );
    assert!(
        dup_result.is_err(),
        "duplicate account_id+folder_id+uid should be rejected by UNIQUE constraint"
    );

    // Upsert via save_message on the same folder still updates in place.
    let msg3 = make_cached_message(1, "Updated via upsert", "a@test.com", "Body updated");
    save_message(&conn, account_id, &msg3, "INBOX").unwrap();
    let fetched = fetch_message_body(&conn, account_id, 1, None)
        .unwrap()
        .expect("message should exist after upsert");
    assert_eq!(fetched.subject.as_deref(), Some("Updated via upsert"));
}

#[test]
fn test_migration_rebuilds_messages_unique_constraint() {
    // Simulate an old-schema database: messages with UNIQUE(account_id, uid),
    // two folders sharing the same uid range, then run the migration and
    // verify both rows survive and the new constraint is in place.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE accounts (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL,
            imap_host TEXT NOT NULL, imap_port INTEGER NOT NULL DEFAULT 993, imap_ssl INTEGER NOT NULL DEFAULT 1,
            smtp_host TEXT NOT NULL, smtp_port INTEGER NOT NULL DEFAULT 587, smtp_tls INTEGER NOT NULL DEFAULT 1,
            username TEXT NOT NULL, password TEXT NOT NULL, smtp_username TEXT NOT NULL DEFAULT '',
            smtp_password TEXT NOT NULL DEFAULT '', sender_name TEXT NOT NULL DEFAULT '',
            sender_email TEXT NOT NULL DEFAULT '', sync_mode TEXT NOT NULL DEFAULT 'mirror',
            trash_retention_days INTEGER NOT NULL DEFAULT 30, created_at TEXT NOT NULL DEFAULT (datetime('now')));
        CREATE TABLE folders (id INTEGER PRIMARY KEY AUTOINCREMENT, account_id INTEGER NOT NULL,
            name TEXT NOT NULL, imap_id TEXT, local_only INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            folder_id INTEGER DEFAULT 1,
            uid INTEGER NOT NULL,
            message_id TEXT, subject TEXT, from_addr TEXT, to_addr TEXT, date TEXT,
            body_text TEXT, body_html TEXT, flags TEXT DEFAULT '[]', ai_summary TEXT, ai_priority REAL,
            ai_fraud_score REAL, is_read INTEGER NOT NULL DEFAULT 0, is_flagged INTEGER NOT NULL DEFAULT 0,
            synced INTEGER NOT NULL DEFAULT 0, has_attachments INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(account_id, uid));
        INSERT INTO accounts (name, imap_host, imap_port, imap_ssl, smtp_host, smtp_port, smtp_tls, username, password) VALUES ('a','h',993,1,'h',587,1,'u','p');
        INSERT INTO folders (account_id, name) VALUES (1, 'INBOX'), (1, 'SENT');
        INSERT INTO messages (account_id, folder_id, uid, subject, synced) VALUES (1, 1, 1, 'inbox-1', 1);
        INSERT INTO messages (account_id, folder_id, uid, subject, synced) VALUES (1, 2, 3, 'sent-1', 1);
        INSERT INTO messages (account_id, folder_id, uid, subject, synced) VALUES (1, 1, 2, 'inbox-2', 1);
        INSERT INTO messages (account_id, folder_id, uid, subject, synced) VALUES (1, 2, 4, 'sent-2', 1);
    ",
    )
    .unwrap();

    // Add the remaining tables init_db expects (accounts/folders exist above;
    // init_db is idempotent for the rest). Foreign keys off in-memory default.
    db::init_db(&conn).unwrap();

    // Migration must have run: all 4 rows survive, old constraint is gone.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 4, "all (folder_id, uid) combos survive the rebuild");

    // New constraint is UNIQUE(account_id, folder_id, uid): the same uid can
    // now exist in a different folder of the same account.
    conn.execute(
        "INSERT INTO messages (account_id, folder_id, uid, subject, synced) VALUES (1, 2, 1, 'sent-uid1-after-migrate', 1)",
        [],
    )
    .unwrap();
    let sent: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE folder_id = 2 AND uid = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sent, 1, "same uid in a different folder must be allowed after migration");

    // Same (account, folder, uid) still rejected.
    let dup = conn.execute(
        "INSERT INTO messages (account_id, folder_id, uid, subject) VALUES (1, 1, 1, 'dup')",
        [],
    );
    assert!(dup.is_err(), "new UNIQUE(account_id, folder_id, uid) must reject dup");
}

#[test]
fn test_delete_account_removes_all_associated_data() {
    // Full consistency: deleting an account must remove all its messages,
    // folders, and contact_profiles — not just the account row.
    let conn = setup_db();
    let account_id = create_test_account(&conn, "fullclean");

    // Add messages in two folders
    for uid in 1..=3 {
        let msg = make_cached_message(uid, &format!("Msg {}", uid), "x@test.com", "body");
        save_message(&conn, account_id, &msg, "INBOX").unwrap();
    }
    for uid in 4..=5 {
        let msg = make_cached_message(uid, &format!("Msg {}", uid), "x@test.com", "body");
        save_message(&conn, account_id, &msg, "SENT").unwrap();
    }

    // Add a contact profile
    conn.execute(
        "INSERT INTO contact_profiles (account_id, email_hash, display_name, first_seen_at, last_analyzed_at)
         VALUES (?1, 'hash123', 'Test Contact', datetime('now'), datetime('now'))",
        rusqlite::params![account_id],
    )
    .unwrap();

    // Verify data exists
    assert_eq!(list_accounts(&conn).unwrap().len(), 1);
    let msg_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE account_id = ?1",
            rusqlite::params![account_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(msg_count, 5, "5 messages should exist before delete");
    let profile_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM contact_profiles WHERE account_id = ?1",
            rusqlite::params![account_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(profile_count, 1, "1 profile should exist before delete");

    // Delete account
    delete_account(&conn, account_id).unwrap();

    // Verify everything is gone
    assert!(list_accounts(&conn).unwrap().is_empty());
    let msg_count_after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE account_id = ?1",
            rusqlite::params![account_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(msg_count_after, 0, "messages should be cascaded");
    let folder_count_after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM folders WHERE account_id = ?1",
            rusqlite::params![account_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(folder_count_after, 0, "folders should be cascaded");
    let profile_count_after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM contact_profiles WHERE account_id = ?1",
            rusqlite::params![account_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(profile_count_after, 0, "contact_profiles should be cascaded");
}

#[test]
fn test_messages_isolated_between_accounts() {
    // Messages from different accounts must not leak into each other's queries.
    let conn = setup_db();
    let acc1 = create_test_account(&conn, "iso1");
    let acc2 = create_test_account(&conn, "iso2");

    let msg1 = make_cached_message(1, "Account 1 msg", "a1@test.com", "body1");
    let msg2 = make_cached_message(1, "Account 2 msg", "a2@test.com", "body2");
    save_message(&conn, acc1, &msg1, "INBOX").unwrap();
    save_message(&conn, acc2, &msg2, "INBOX").unwrap();

    let inbox1 = fetch_inbox(&conn, acc1, Some(10), Some(0), "INBOX").unwrap();
    assert_eq!(inbox1.len(), 1);
    assert_eq!(inbox1[0].from_addr.as_deref(), Some("a1@test.com"));

    let inbox2 = fetch_inbox(&conn, acc2, Some(10), Some(0), "INBOX").unwrap();
    assert_eq!(inbox2.len(), 1);
    assert_eq!(inbox2[0].from_addr.as_deref(), Some("a2@test.com"));
}

#[test]
fn test_save_message_creates_folder_automatically() {
    // save_message should auto-create the folder if it doesn't exist.
    let conn = setup_db();
    let account_id = create_test_account(&conn, "autofolder");

    let msg = make_cached_message(1, "Auto folder", "x@test.com", "body");
    save_message(&conn, account_id, &msg, "CustomFolder").unwrap();

    let folder_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM folders WHERE account_id = ?1 AND name = ?2",
            rusqlite::params![account_id, "CustomFolder"],
            |row| row.get(0),
        )
        .unwrap();
    assert!(folder_exists, "folder should be auto-created");

    let inbox = fetch_inbox(&conn, account_id, Some(10), Some(0), "CustomFolder").unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].uid, 1);
}

// ===========================================================================
// Cache retention / pruning
// ===========================================================================

use crate::cache::messages::{prune_old_messages, vacuum};

/// Helper: count messages for an account.
fn message_count(conn: &Connection, account_id: i64) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE account_id = ?1",
        rusqlite::params![account_id],
        |row| row.get(0),
    )
    .unwrap()
}

/// Helper: force a message's created_at to N days in the past.
fn age_message(conn: &Connection, account_id: i64, uid: u32, days: u32) {
    conn.execute(
        "UPDATE messages SET created_at = datetime('now', ?1) WHERE account_id = ?2 AND uid = ?3",
        rusqlite::params![format!("-{} days", days), account_id, uid],
    )
    .unwrap();
}

#[test]
fn test_prune_keeps_recent_messages() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "prune_recent");
    let msg = make_cached_message(1, "Recent", "x@test.com", "body");
    save_message(&conn, account_id, &msg, "INBOX").unwrap();
    // created_at is "now" → must never be pruned with a 90-day window.
    let deleted = prune_old_messages(&conn, 90, 0).unwrap();
    assert_eq!(deleted, 0);
    assert_eq!(message_count(&conn, account_id), 1);
}

#[test]
fn test_prune_removes_old_messages() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "prune_old");
    for uid in 1..=5 {
        let msg = make_cached_message(uid, "Old", "x@test.com", "body");
        save_message(&conn, account_id, &msg, "INBOX").unwrap();
        age_message(&conn, account_id, uid, 120); // 120 days old
    }
    // keep_minimum = 0 → all 5 old messages are eligible.
    let deleted = prune_old_messages(&conn, 90, 0).unwrap();
    assert_eq!(deleted, 5);
    assert_eq!(message_count(&conn, account_id), 0);
}

#[test]
fn test_prune_respects_keep_minimum() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "prune_keepmin");
    for uid in 1..=10 {
        let msg = make_cached_message(uid, "Old", "x@test.com", "body");
        save_message(&conn, account_id, &msg, "INBOX").unwrap();
        age_message(&conn, account_id, uid, 200); // all very old
    }
    // Even though all are old, keep the 3 newest (by date/uid).
    let deleted = prune_old_messages(&conn, 90, 3).unwrap();
    assert_eq!(deleted, 7);
    assert_eq!(message_count(&conn, account_id), 3);
}

#[test]
fn test_prune_does_not_drop_unsynced() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "prune_unsynced");
    let msg = make_cached_message(1, "Old unsynced", "x@test.com", "body");
    save_message(&conn, account_id, &msg, "INBOX").unwrap();
    age_message(&conn, account_id, 1, 200);
    // Mark as not-yet-synced — must be retained regardless of age.
    conn.execute(
        "UPDATE messages SET synced = 0 WHERE account_id = ?1 AND uid = 1",
        rusqlite::params![account_id],
    )
    .unwrap();
    let deleted = prune_old_messages(&conn, 90, 0).unwrap();
    assert_eq!(deleted, 0);
    assert_eq!(message_count(&conn, account_id), 1);
}

#[test]
fn test_prune_isolates_accounts() {
    let conn = setup_db();
    let a = create_test_account(&conn, "prune_a");
    let b = create_test_account(&conn, "prune_b");
    // Account A: old messages. Account B: recent messages.
    for uid in 1..=4 {
        let msg = make_cached_message(uid, "A old", "x@test.com", "body");
        save_message(&conn, a, &msg, "INBOX").unwrap();
        age_message(&conn, a, uid, 200);
    }
    for uid in 1..=4 {
        let msg = make_cached_message(uid, "B recent", "y@test.com", "body");
        save_message(&conn, b, &msg, "INBOX").unwrap();
    }
    let deleted = prune_old_messages(&conn, 90, 0).unwrap();
    assert_eq!(deleted, 4);
    assert_eq!(message_count(&conn, a), 0);
    assert_eq!(message_count(&conn, b), 4);
}

#[test]
fn test_search_finds_by_subject_and_sender() {
    use crate::cache::messages::search_messages;
    let conn = setup_db();
    let account_id = create_test_account(&conn, "search");

    let m1 = make_cached_message(1, "Rechnung Februar", "buchhaltung@firma.de", "Betrag faellig");
    let m2 = make_cached_message(2, "Urlaubsfotos", "freund@example.com", "Strand und Sonne");
    save_message(&conn, account_id, &m1, "INBOX").unwrap();
    save_message(&conn, account_id, &m2, "INBOX").unwrap();

    // By subject term
    let r = search_messages(&conn, account_id, "Rechnung", 50).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].uid, 1);

    // By sender domain
    let r = search_messages(&conn, account_id, "firma", 50).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].uid, 1);

    // Prefix match
    let r = search_messages(&conn, account_id, "Urlaub", 50).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].uid, 2);

    // No match
    let r = search_messages(&conn, account_id, "Quartalsbericht", 50).unwrap();
    assert!(r.is_empty());

    // Empty query returns nothing (not everything)
    let r = search_messages(&conn, account_id, "   ", 50).unwrap();
    assert!(r.is_empty());
}

#[test]
fn test_search_updates_on_delete() {
    use crate::cache::messages::{delete_message as del, search_messages};
    let conn = setup_db();
    let account_id = create_test_account(&conn, "search_del");
    let m = make_cached_message(1, "Einzigartig Suchbegriff", "x@test.com", "body");
    save_message(&conn, account_id, &m, "INBOX").unwrap();
    assert_eq!(search_messages(&conn, account_id, "Einzigartig", 50).unwrap().len(), 1);
    del(&conn, account_id, 1).unwrap();
    // FTS index must be updated by the delete trigger.
    assert_eq!(search_messages(&conn, account_id, "Einzigartig", 50).unwrap().len(), 0);
}

#[test]
fn test_search_isolates_accounts() {
    use crate::cache::messages::search_messages;
    let conn = setup_db();
    let a = create_test_account(&conn, "search_a");
    let b = create_test_account(&conn, "search_b");
    let m = make_cached_message(1, "Gemeinsamerbegriff", "x@test.com", "body");
    save_message(&conn, a, &m, "INBOX").unwrap();
    save_message(&conn, b, &m, "INBOX").unwrap();
    let r = search_messages(&conn, a, "Gemeinsamerbegriff", 50).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].account_id, a);
}

#[test]
fn test_has_attachments_roundtrip() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "attach");
    let mut m = make_cached_message(1, "Mit Anhang", "x@test.com", "body");
    m.has_attachments = true;
    save_message(&conn, account_id, &m, "INBOX").unwrap();

    let inbox = fetch_inbox(&conn, account_id, Some(10), Some(0), "INBOX").unwrap();
    assert_eq!(inbox.len(), 1);
    assert!(inbox[0].has_attachments, "has_attachments sollte persistiert werden");

    // A message without attachments stays false.
    let m2 = make_cached_message(2, "Ohne Anhang", "y@test.com", "body");
    save_message(&conn, account_id, &m2, "INBOX").unwrap();
    let body = fetch_message_body(&conn, account_id, 2, None).unwrap().unwrap();
    assert!(!body.has_attachments);
}

#[test]
fn test_vacuum_runs() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "vacuum");
    let msg = make_cached_message(1, "x", "x@test.com", "body");
    save_message(&conn, account_id, &msg, "INBOX").unwrap();
    // VACUUM must succeed on a normal connection.
    vacuum(&conn).unwrap();
}

#[test]
fn test_fetch_message_body_scoped_by_folder_on_uid_collision() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "uidcollision");

    // Two different messages that happen to share the same IMAP uid across
    // folders (IMAP uids are only unique per folder).
    let inbox_msg = make_cached_message(7, "INBOX Subject", "inbox@test.com", "INBOX BODY TEXT");
    save_message(&conn, account_id, &inbox_msg, "INBOX").unwrap();
    let draft_msg = make_cached_message(7, "Draft Subject", "draft@test.com", "DRAFT BODY TEXT");
    save_message(&conn, account_id, &draft_msg, "Entwürfe").unwrap();

    // Locate both folder ids.
    let inbox_id: i64 = conn.query_row(
        "SELECT id FROM folders WHERE account_id = ?1 AND name = 'INBOX'",
        rusqlite::params![account_id],
        |r| r.get(0),
    ).unwrap();
    let drafts_id: i64 = conn.query_row(
        "SELECT id FROM folders WHERE account_id = ?1 AND name = 'Entwürfe'",
        rusqlite::params![account_id],
        |r| r.get(0),
    ).unwrap();

    // Scoped lookup must return the right body for each folder.
    let inbox_body = fetch_message_body(&conn, account_id, 7, Some(inbox_id))
        .unwrap()
        .unwrap()
        .body_text
        .unwrap();
    assert_eq!(inbox_body, "INBOX BODY TEXT", "INBOX-scoped lookup returned the wrong message");

    let drafts_body = fetch_message_body(&conn, account_id, 7, Some(drafts_id))
        .unwrap()
        .unwrap()
        .body_text
        .unwrap();
    assert_eq!(drafts_body, "DRAFT BODY TEXT", "Drafts-scoped lookup returned the wrong message");
}

#[test]
fn test_attachment_metadata_scoped_by_folder_on_uid_collision() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "attcollision");

    // Two messages sharing uid 7 across folders, each with its own attachment.
    // The sync path must attach metadata to the message inside the SAME folder
    // (uid is only unique per folder) — an unscoped lookup would attach both
    // attachment sets to the same message_id.
    let mut inbox_msg = make_cached_message(7, "INBOX Att", "inbox@test.com", "INBOX BODY");
    inbox_msg.has_attachments = true;
    inbox_msg.attachments = vec![
        crate::imap::types::AttachmentMeta {
            filename: "inbox-file.pdf".into(),
            content_type: "application/pdf".into(),
            size: 100,
        },
    ];
    save_message(&conn, account_id, &inbox_msg, "INBOX").unwrap();

    let mut draft_msg = make_cached_message(7, "Draft Att", "draft@test.com", "DRAFT BODY");
    draft_msg.has_attachments = true;
    draft_msg.attachments = vec![
        crate::imap::types::AttachmentMeta {
            filename: "draft-file.pdf".into(),
            content_type: "application/pdf".into(),
            size: 200,
        },
    ];
    save_message(&conn, account_id, &draft_msg, "Entwürfe").unwrap();

    // Resolve both message ids.
    let inbox_id: i64 = conn.query_row(
        "SELECT id FROM messages WHERE account_id = ?1 AND uid = ?2 AND folder_id =
            (SELECT id FROM folders WHERE account_id = ?1 AND name = 'INBOX')",
        rusqlite::params![account_id, 7],
        |r| r.get(0),
    ).unwrap();
    let drafts_id: i64 = conn.query_row(
        "SELECT id FROM messages WHERE account_id = ?1 AND uid = ?2 AND folder_id =
            (SELECT id FROM folders WHERE account_id = ?1 AND name = 'Entwürfe')",
        rusqlite::params![account_id, 7],
        |r| r.get(0),
    ).unwrap();
    assert_ne!(inbox_id, drafts_id, "both messages must be distinct rows");

    // Each message must carry exactly its own attachment.
    let inbox_attachments = crate::cache::attachments::get_attachments(&conn, inbox_id).unwrap();
    assert_eq!(inbox_attachments.len(), 1, "INBOX must have exactly 1 attachment");
    assert_eq!(inbox_attachments[0].filename, "inbox-file.pdf");

    let drafts_attachments = crate::cache::attachments::get_attachments(&conn, drafts_id).unwrap();
    assert_eq!(drafts_attachments.len(), 1, "Drafts must have exactly 1 attachment");
    assert_eq!(drafts_attachments[0].filename, "draft-file.pdf");

    // Regression guard: the pre-fix bug attached BOTH files to the first
    // matched message (same uid). Verify no cross-contamination.
    assert!(
        inbox_attachments.iter().all(|a| a.filename != "draft-file.pdf"),
        "INBOX message must not inherit the draft's attachment"
    );
    assert!(
        drafts_attachments.iter().all(|a| a.filename != "inbox-file.pdf"),
        "Draft message must not inherit the INBOX attachment"
    );
}

#[test]
fn test_reconcile_attachments_stable_ids_and_removes_stale() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "reconcile");

    // First sync: message with two attachments.
    let mut msg = make_cached_message(9, "Reconcile", "r@test.com", "BODY");
    msg.has_attachments = true;
    msg.attachments = vec![
        crate::imap::types::AttachmentMeta { filename: "a.pdf".into(), content_type: "application/pdf".into(), size: 10 },
        crate::imap::types::AttachmentMeta { filename: "b.pdf".into(), content_type: "application/pdf".into(), size: 20 },
    ];
    save_message(&conn, account_id, &msg, "INBOX").unwrap();

    let message_id: i64 = conn.query_row(
        "SELECT id FROM messages WHERE account_id = ?1 AND uid = ?2 AND folder_id =
            (SELECT id FROM folders WHERE account_id = ?1 AND name = 'INBOX')",
        rusqlite::params![account_id, 9],
        |r| r.get(0),
    ).unwrap();

    let first = crate::cache::attachments::get_attachments(&conn, message_id).unwrap();
    assert_eq!(first.len(), 2);
    let id_a = first[0].id;
    let id_b = first[1].id;
    assert_eq!(first[0].part_index, 0);
    assert_eq!(first[1].part_index, 1);
    assert_eq!(first[0].filename, "a.pdf");

    // Second sync: identical list, same order. part_index is positional
    // (BODYSTRUCTURE order), so identical positions must keep their ids —
    // this is the invariant that replaces the old DELETE+re-INSERT churn.
    save_message(&conn, account_id, &msg, "INBOX").unwrap();
    let same = crate::cache::attachments::get_attachments(&conn, message_id).unwrap();
    assert_eq!(same.len(), 2);
    assert_eq!(same[0].id, id_a, "part_index 0 must keep its id across a no-op re-sync");
    assert_eq!(same[1].id, id_b, "part_index 1 must keep its id across a no-op re-sync");

    // Third sync: new list [b, c] (a removed, c added). Positional identity
    // means b moves to part_index 0 (new id is fine — its position changed)
    // and the stale row 'a' is removed.
    msg.attachments = vec![
        crate::imap::types::AttachmentMeta { filename: "b.pdf".into(), content_type: "application/pdf".into(), size: 20 },
        crate::imap::types::AttachmentMeta { filename: "c.pdf".into(), content_type: "application/pdf".into(), size: 30 },
    ];
    save_message(&conn, account_id, &msg, "INBOX").unwrap();

    let third = crate::cache::attachments::get_attachments(&conn, message_id).unwrap();
    assert_eq!(third.len(), 2, "stale 'a.pdf' must be removed");
    assert!(third.iter().all(|a| a.filename != "a.pdf"), "removed part must be gone");
    assert!(third.iter().any(|a| a.filename == "c.pdf"), "new part must be present");
    assert!(third.iter().any(|a| a.filename == "b.pdf"), "kept part must be present");

    // Fourth sync: message loses ALL attachments (empty BODYSTRUCTURE).
    // Stale rows must be removed entirely.
    msg.has_attachments = false;
    msg.attachments = vec![];
    save_message(&conn, account_id, &msg, "INBOX").unwrap();

    let fourth = crate::cache::attachments::get_attachments(&conn, message_id).unwrap();
    assert_eq!(fourth.len(), 0, "all attachment rows must be removed when the fresh list is empty");
    let _ = id_a;
    let _ = id_b;
}

#[test]
fn test_draft_attachment_persistence_dedup_roundtrip() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "draftatt");

    // Mirror save_draft: ensure local "Entwürfe" folder, insert a draft row,
    // reconcile metadata, then persist content deduplicated to disk.
    crate::cache::messages::create_local_folder(&conn, account_id, "Entwürfe").unwrap();
    let drafts_id: i64 = conn.query_row(
        "SELECT id FROM folders WHERE account_id = ?1 AND name = 'Entwürfe'",
        rusqlite::params![account_id],
        |r| r.get(0),
    ).unwrap();
    let uid: i64 = 1;
    conn.execute(
        "INSERT INTO messages (account_id, folder_id, uid, subject, from_addr, to_addr, date, body_text, body_html, synced)
         VALUES (?1, ?2, ?3, 'Draft', '', '', datetime('now'), 'body', NULL, 0)",
        rusqlite::params![account_id, drafts_id, uid],
    ).unwrap();
    let message_id: i64 = conn.query_row(
        "SELECT id FROM messages WHERE account_id = ?1 AND uid = ?2 AND folder_id = ?3",
        rusqlite::params![account_id, uid, drafts_id],
        |r| r.get(0),
    ).unwrap();

    // Reconcile metadata like save_draft does.
    let metas = vec![
        crate::imap::types::AttachmentMeta { filename: "draft.pdf".into(), content_type: "application/pdf".into(), size: 4 },
    ];
    crate::cache::attachments::reconcile_attachments(&conn, message_id, &metas).unwrap();

    // Persist content to a temp dedup store.
    let dir = tempfile::tempdir().unwrap();
    let data_root = dir.path();
    let b64 = base64::engine::general_purpose::STANDARD.encode(b"data");
    let atts = crate::cache::attachments::get_attachments(&conn, message_id).unwrap();
    assert_eq!(atts.len(), 1);
    assert_eq!(atts[0].part_index, 0);
    let rel = crate::cache::attachments::cache_content_dedup(&conn, atts[0].id, &b64, data_root).unwrap();

    // disk_path + sha256 recorded; the file exists on disk with the raw bytes.
    let after = crate::cache::attachments::get_attachments(&conn, message_id).unwrap();
    assert_eq!(after[0].sha256.as_deref().map(|s| s.len()), Some(64));
    assert!(data_root.join(&rel).exists());
    let raw = std::fs::read(data_root.join(&rel)).unwrap();
    assert_eq!(raw, b"data");
}

#[test]
fn test_gc_removes_only_orphaned_dedup_files() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "gc");
    crate::cache::messages::create_local_folder(&conn, account_id, "Entwürfe").unwrap();
    let drafts_id: i64 = conn.query_row(
        "SELECT id FROM folders WHERE account_id = ?1 AND name = 'Entwürfe'",
        rusqlite::params![account_id],
        |r| r.get(0),
    ).unwrap();
    let uid: i64 = 1;
    conn.execute(
        "INSERT INTO messages (account_id, folder_id, uid, subject, from_addr, to_addr, date, body_text, body_html, synced)
         VALUES (?1, ?2, ?3, 'GC', '', '', datetime('now'), 'body', NULL, 0)",
        rusqlite::params![account_id, drafts_id, uid],
    ).unwrap();
    let message_id: i64 = conn.query_row(
        "SELECT id FROM messages WHERE account_id = ?1 AND uid = ?2 AND folder_id = ?3",
        rusqlite::params![account_id, uid, drafts_id],
        |r| r.get(0),
    ).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let data_root = dir.path();
    let b64 = base64::engine::general_purpose::STANDARD.encode(b"kept");
    let metas = vec![crate::imap::types::AttachmentMeta { filename: "kept.pdf".into(), content_type: "application/pdf".into(), size: 4 }];
    crate::cache::attachments::reconcile_attachments(&conn, message_id, &metas).unwrap();
    let atts = crate::cache::attachments::get_attachments(&conn, message_id).unwrap();
    let rel = crate::cache::attachments::cache_content_dedup(&conn, atts[0].id, &b64, data_root).unwrap();

    // Drop a second orphaned file directly into the dedup store (no DB row).
    let orphan = data_root.join("attachments").join("deadbeef".repeat(8));
    std::fs::write(&orphan, b"orphan").unwrap();

    let report = crate::cache::attachments::gc_orphaned_attachments(&conn, data_root).unwrap();
    assert_eq!(report.removed_files, 1);
    assert_eq!(report.freed_bytes, 6);
    assert_eq!(report.kept_files, 1);
    assert!(!orphan.exists());
    assert!(data_root.join(&rel).exists());

    // Referenced files survive; the row's disk_path still resolves.
    let report2 = crate::cache::attachments::gc_orphaned_attachments(&conn, data_root).unwrap();
    assert_eq!(report2.removed_files, 0);
}

#[test]
fn test_repair_fixes_flag_and_orphaned_disk_path() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "repair");
    crate::cache::messages::create_local_folder(&conn, account_id, "Entwürfe").unwrap();
    let drafts_id: i64 = conn.query_row(
        "SELECT id FROM folders WHERE account_id = ?1 AND name = 'Entwürfe'",
        rusqlite::params![account_id],
        |r| r.get(0),
    ).unwrap();

    // Message A: flagged with attachments, but has none.
    let uid_a: i64 = 10;
    conn.execute(
        "INSERT INTO messages (account_id, folder_id, uid, subject, from_addr, to_addr, date, body_text, body_html, synced, has_attachments)
         VALUES (?1, ?2, ?3, 'A', '', '', datetime('now'), 'body', NULL, 0, 1)",
        rusqlite::params![account_id, drafts_id, uid_a],
    ).unwrap();

    // Message B: NOT flagged, but has an attachment row with a missing file.
    let uid_b: i64 = 20;
    conn.execute(
        "INSERT INTO messages (account_id, folder_id, uid, subject, from_addr, to_addr, date, body_text, body_html, synced, has_attachments)
         VALUES (?1, ?2, ?3, 'B', '', '', datetime('now'), 'body', NULL, 0, 0)",
        rusqlite::params![account_id, drafts_id, uid_b],
    ).unwrap();
    let msg_b_id: i64 = conn.query_row(
        "SELECT id FROM messages WHERE account_id = ?1 AND uid = ?2",
        rusqlite::params![account_id, uid_b],
        |r| r.get(0),
    ).unwrap();
    conn.execute(
        "INSERT INTO message_attachments (message_id, part_index, filename, content_type, size, disk_path, sha256, content_cached)
         VALUES (?1, 0, 'gone.pdf', 'application/pdf', 4, 'attachments/missing', NULL, 1)",
        rusqlite::params![msg_b_id],
    ).unwrap();

    let dir = tempfile::tempdir().unwrap();

    // Check-only first: numbers are reported, nothing mutated.
    let report = crate::cache::attachments::check_and_repair_attachments(&conn, dir.path(), false).unwrap();
    assert_eq!(report.flagged_without_rows, 1);
    assert_eq!(report.unflagged_with_rows, 1);
    assert_eq!(report.rows_with_missing_file, 1);
    assert_eq!(report.repaired_rows, 0);

    // Repair mutates: has_attachments fixed, orphaned disk_path cleared.
    let report = crate::cache::attachments::check_and_repair_attachments(&conn, dir.path(), true).unwrap();
    assert_eq!(report.repaired_rows, 3); // 1 disk_path + 2 flag corrections
    let flagged_a: i64 = conn.query_row(
        "SELECT has_attachments FROM messages WHERE account_id = ?1 AND uid = ?2",
        rusqlite::params![account_id, uid_a],
        |r| r.get(0),
    ).unwrap();
    let flagged_b: i64 = conn.query_row(
        "SELECT has_attachments FROM messages WHERE account_id = ?1 AND uid = ?2",
        rusqlite::params![account_id, uid_b],
        |r| r.get(0),
    ).unwrap();
    assert_eq!(flagged_a, 0);
    assert_eq!(flagged_b, 1);
    let disk_path: Option<String> = conn.query_row(
        "SELECT disk_path FROM message_attachments WHERE message_id = ?1",
        rusqlite::params![msg_b_id],
        |r| r.get(0),
    ).unwrap();
    assert!(disk_path.is_none());
}
