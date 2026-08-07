use chrono::Utc;
use rusqlite::{params, Connection};

use crate::cache::accounts::{
    create_account, delete_account, get_account, list_accounts,
};
use crate::cache::db;
use crate::cache::messages::{delete_message, delete_messages_not_in, fetch_inbox, fetch_message_body, save_message};
use crate::imap::types::{CachedMessage, MailEnvelope};

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
    assert!(fetch_message_body(&conn, account_id, 1).unwrap().is_some());
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
    assert!(fetch_message_body(&conn, account_id, 1).unwrap().is_none());
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

    let fetched = fetch_message_body(&conn, account_id, 7)
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
    assert!(fetch_message_body(&conn, account_id, 999).unwrap().is_none());
}

#[test]
fn test_delete_message_removes_from_cache() {
    let conn = setup_db();
    let account_id = create_test_account(&conn, "delmsg");

    let msg = make_cached_message(10, "To delete", "spam@example.com", "Spam content.");
    save_message(&conn, account_id, &msg, "INBOX").unwrap();
    assert!(fetch_message_body(&conn, account_id, 10).unwrap().is_some());

    delete_message(&conn, account_id, 10).unwrap();
    assert!(
        fetch_message_body(&conn, account_id, 10).unwrap().is_none(),
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

    let fetched = fetch_message_body(&conn, account_id, 1)
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
    assert!(fetch_message_body(&conn, account_id, 1).unwrap().is_some());

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
    let msg1_after = fetch_message_body(&conn, account_id, 1)
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
    let fetched = fetch_message_body(&conn, account_id, 100)
        .unwrap()
        .expect("message should be in cache despite IMAP failure");
    assert_eq!(fetched.subject.as_deref(), Some("Cached OK"));
    assert_eq!(fetched.from_addr.as_deref(), Some("remote@test.com"));
    assert!(fetched.synced, "synced flag should be 1 after save_message");
}

#[test]
fn test_unique_constraint_prevents_duplicate_uid_per_account() {
    // The UNIQUE(account_id, uid) constraint on messages should prevent
    // inserting two rows with the same account_id + uid.
    let conn = setup_db();
    let account_id = create_test_account(&conn, "unique");

    let msg1 = make_cached_message(1, "First", "a@test.com", "Body 1");
    save_message(&conn, account_id, &msg1, "INBOX").unwrap();

    // Direct INSERT with same account_id + uid should fail
    let dup_result = conn.execute(
        "INSERT INTO messages (account_id, folder_id, uid, message_id, subject, from_addr, to_addr, date, body_text, is_read, synced)
         VALUES (?1, 1, 1, '<dup@test.com>', 'Duplicate', 'b@test.com', 'c@test.com', '2025-01-01', 'dup body', 0, 1)",
        rusqlite::params![account_id],
    );
    assert!(
        dup_result.is_err(),
        "duplicate account_id+uid should be rejected by UNIQUE constraint"
    );

    // But upsert via save_message should succeed (ON CONFLICT DO UPDATE)
    let msg2 = make_cached_message(1, "Updated via upsert", "a@test.com", "Body updated");
    save_message(&conn, account_id, &msg2, "INBOX").unwrap();
    let fetched = fetch_message_body(&conn, account_id, 1)
        .unwrap()
        .expect("message should exist after upsert");
    assert_eq!(fetched.subject.as_deref(), Some("Updated via upsert"));
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
    let body = fetch_message_body(&conn, account_id, 2).unwrap().unwrap();
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
