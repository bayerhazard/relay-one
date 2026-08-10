use rusqlite::Connection;

pub fn init_db(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS accounts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            imap_host TEXT NOT NULL,
            imap_port INTEGER NOT NULL DEFAULT 993,
            imap_ssl INTEGER NOT NULL DEFAULT 1,
            smtp_host TEXT NOT NULL,
            smtp_port INTEGER NOT NULL DEFAULT 587,
            smtp_tls INTEGER NOT NULL DEFAULT 1,
            username TEXT NOT NULL,
            password TEXT NOT NULL,
            smtp_username TEXT NOT NULL DEFAULT '',
            smtp_password TEXT NOT NULL DEFAULT '',
            sender_name TEXT NOT NULL DEFAULT '',
            sender_email TEXT NOT NULL DEFAULT '',
            sync_mode TEXT NOT NULL DEFAULT 'mirror',
            trash_retention_days INTEGER NOT NULL DEFAULT 30,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS folders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            imap_id TEXT,
            local_only INTEGER NOT NULL DEFAULT 0
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_folders_account_name ON folders(account_id, name);

        CREATE TABLE IF NOT EXISTS messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            folder_id INTEGER DEFAULT 1,
            uid INTEGER NOT NULL,
            message_id TEXT,
            subject TEXT,
            from_addr TEXT,
            to_addr TEXT,
            date TEXT,
            body_text TEXT,
            body_html TEXT,
            flags TEXT DEFAULT '[]',
            ai_summary TEXT,
            ai_priority REAL,
            ai_fraud_score REAL,
            is_read INTEGER NOT NULL DEFAULT 0,
            is_flagged INTEGER NOT NULL DEFAULT 0,
            synced INTEGER NOT NULL DEFAULT 0,
            has_attachments INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(account_id, folder_id, uid)
        );

        CREATE TABLE IF NOT EXISTS ai_audit_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            message_id TEXT,
            action TEXT NOT NULL,
            input_hash TEXT,
            output TEXT NOT NULL,
            model TEXT,
            tone_freundlich INTEGER,
            tone_professionell INTEGER,
            tone_laenge INTEGER,
            confirmed INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS contact_profiles (
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

        CREATE INDEX IF NOT EXISTS idx_messages_account ON messages(account_id, folder_id);
        CREATE INDEX IF NOT EXISTS idx_messages_uid ON messages(account_id, uid);
        CREATE INDEX IF NOT EXISTS idx_messages_date ON messages(account_id, date DESC);
        CREATE INDEX IF NOT EXISTS idx_messages_read ON messages(account_id, is_read);
        CREATE INDEX IF NOT EXISTS idx_audit_message ON ai_audit_log(message_id);
        CREATE INDEX IF NOT EXISTS idx_contact_profiles_email ON contact_profiles(account_id, email_hash);
        CREATE INDEX IF NOT EXISTS idx_contact_profiles_count ON contact_profiles(account_id, sample_count DESC);

        CREATE TABLE IF NOT EXISTS contacts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            vcard_uid TEXT UNIQUE NOT NULL,
            given_name TEXT,
            family_name TEXT,
            display_name TEXT,
            email TEXT,
            phone TEXT,
            organization TEXT,
            vcard_raw TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'carddav',
            synced_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_contacts_search ON contacts(display_name, email);

        CREATE TABLE IF NOT EXISTS mail_snippets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            email_hash TEXT NOT NULL,
            topic_tag TEXT NOT NULL DEFAULT 'general',
            text TEXT NOT NULL,
            sent_date TEXT NOT NULL,
            is_outgoing INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY (account_id, email_hash) REFERENCES contact_profiles(account_id, email_hash) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_snippets_recipient ON mail_snippets(account_id, email_hash, topic_tag);

        CREATE TABLE IF NOT EXISTS learning_diffs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            email_hash TEXT NOT NULL,
            topic_tag TEXT NOT NULL DEFAULT 'general',
            ai_draft TEXT NOT NULL,
            user_final TEXT NOT NULL,
            edit_distance REAL NOT NULL,
            style_hint TEXT,
            analyzed INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY (account_id, email_hash) REFERENCES contact_profiles(account_id, email_hash) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_learning_diffs_analyzed ON learning_diffs(account_id, email_hash, analyzed);

        CREATE TABLE IF NOT EXISTS style_fingerprints (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            email_hash TEXT NOT NULL,
            fingerprint TEXT NOT NULL DEFAULT '',
            hint_count INTEGER NOT NULL DEFAULT 0,
            last_updated TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(account_id, email_hash)
        );
        CREATE INDEX IF NOT EXISTS idx_style_fingerprints ON style_fingerprints(account_id, email_hash);

        CREATE TABLE IF NOT EXISTS voice_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            enabled INTEGER NOT NULL DEFAULT 0,
            stt_url TEXT NOT NULL DEFAULT '',
            stt_key TEXT NOT NULL DEFAULT '',
            stt_model TEXT NOT NULL DEFAULT 'Systran/faster-whisper-small'
        );
        INSERT OR IGNORE INTO voice_settings (id, enabled, stt_url, stt_key, stt_model)
        VALUES (1, 0, '', '', 'Systran/faster-whisper-small');

        CREATE TABLE IF NOT EXISTS message_attachments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
            filename TEXT NOT NULL,
            content_type TEXT NOT NULL,
            size INTEGER NOT NULL DEFAULT 0,
            content TEXT,
            content_cached INTEGER NOT NULL DEFAULT 0,
            cached_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_ma_message ON message_attachments(message_id);

        CREATE TABLE IF NOT EXISTS push_subscriptions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL DEFAULT 0,
            endpoint TEXT NOT NULL,
            p256dh TEXT NOT NULL,
            auth TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(endpoint)
        );
        CREATE INDEX IF NOT EXISTS idx_push_account ON push_subscriptions(account_id);

        -- Delete queue (Concept §5): ONLY filled by explicit user action
        -- (delete / move to local-only). State machine:
        --   pending → verified → deleted | failed
        CREATE TABLE IF NOT EXISTS delete_queue (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
            account_id INTEGER NOT NULL,
            uid INTEGER NOT NULL,
            folder TEXT NOT NULL,
            action TEXT NOT NULL DEFAULT 'delete',
            state TEXT NOT NULL DEFAULT 'pending',
            attempts INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_delete_queue_state ON delete_queue(state);
        CREATE INDEX IF NOT EXISTS idx_delete_queue_account ON delete_queue(account_id);

        -- Sync state per folder (CONDSTORE modseq + last UID) — Phase 4K
        CREATE TABLE IF NOT EXISTS sync_state (
            folder_id INTEGER PRIMARY KEY,
            account_id INTEGER NOT NULL,
            folder_name TEXT NOT NULL,
            last_uid INTEGER NOT NULL DEFAULT 0,
            highest_modseq INTEGER NOT NULL DEFAULT 0,
            last_sync_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_sync_state_account ON sync_state(account_id);
        ",
    )?;

    // Migration: add smtp columns to existing accounts tables
    let _ = conn.execute("ALTER TABLE accounts ADD COLUMN smtp_username TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE accounts ADD COLUMN smtp_password TEXT NOT NULL DEFAULT ''", []);
    // Migration: attachment indicator (derived from BODYSTRUCTURE).
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN has_attachments INTEGER NOT NULL DEFAULT 0", []);
    // Migration: flagged indicator for message star/flag support.
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN is_flagged INTEGER NOT NULL DEFAULT 0", []);
    // Migration: EML archive path (relative to data root) + content hash.
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN raw_path TEXT", []);
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN raw_sha256 TEXT", []);
    // Migration: attachment dedup storage path (relative to data root).
    let _ = conn.execute("ALTER TABLE message_attachments ADD COLUMN disk_path TEXT", []);
    // Migration: local-only folders (no IMAP counterpart).
    let _ = conn.execute("ALTER TABLE folders ADD COLUMN local_only INTEGER NOT NULL DEFAULT 0", []);
    // Migration: per-account sync mode (mirror/archive) + trash retention.
    let _ = conn.execute("ALTER TABLE accounts ADD COLUMN sync_mode TEXT NOT NULL DEFAULT 'mirror'", []);
    let _ = conn.execute("ALTER TABLE accounts ADD COLUMN trash_retention_days INTEGER NOT NULL DEFAULT 30", []);
    // Migration: insecure IMAP TLS (self-signed certs, e.g. Synology NAS).
    // Was previously only used at connect time and never persisted.
    let _ = conn.execute("ALTER TABLE accounts ADD COLUMN imap_insecure INTEGER NOT NULL DEFAULT 0", []);

    // Migration: add photo columns to settings table
    let _ = conn.execute("ALTER TABLE settings ADD COLUMN photo_data BLOB", []);
    let _ = conn.execute("ALTER TABLE settings ADD COLUMN photo_type TEXT", []);

    // Migration: EML archive path on messages (raw RFC822 file on disk).
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN raw_path TEXT", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_messages_raw ON messages(raw_path)", []);

    // Migration: sync mode per account ('mirror' | 'archive').
    let _ = conn.execute("ALTER TABLE accounts ADD COLUMN sync_mode TEXT NOT NULL DEFAULT 'mirror'", []);
    // Migration: trash retention days per account (F7, default 30).
    let _ = conn.execute("ALTER TABLE accounts ADD COLUMN trash_retention_days INTEGER NOT NULL DEFAULT 30", []);
    // Migration: local-only folders carry NULL imap_id and are not synced from IMAP.
    let _ = conn.execute("ALTER TABLE folders ADD COLUMN local_only INTEGER NOT NULL DEFAULT 0", []);

    migrate_messages_uid_constraint(conn);

    init_fts(conn);

    Ok(())
}

/// Migration: IMAP-UIDs are unique per folder, not per account. The old
/// `UNIQUE(account_id, uid)` constraint silently merged messages from
/// different folders that share the same UID range (INSERT OR IGNORE /
/// ON CONFLICT hit it) — messages "vanished" from their folder. Rebuild the
/// messages table with `UNIQUE(account_id, folder_id, uid)`.
///
/// Detection: PRAGMA index_list lists the auto index `sqlite_autoindex_messages_1`
/// only when the table-level UNIQUE constraint exists. If present, rebuild the
/// table, preserving one row per (account_id, folder_id, uid) — the newest
/// (by id) wins, others are dropped.
fn migrate_messages_uid_constraint(conn: &Connection) -> Result<(), rusqlite::Error> {
    let has_old: bool = {
        let mut stmt = conn.prepare(
            "SELECT name FROM pragma_index_list('messages') WHERE name LIKE 'sqlite_autoindex_messages_%'",
        )?;
        let mut rows = stmt.query([])?;
        let mut found = false;
        while let Some(row) = rows.next()? {
            let name: String = row.get(0)?;
            // The old table-level UNIQUE(account_id, uid) produces this auto
            // index. A table-level UNIQUE(account_id, folder_id, uid) also
            // produces one — check its columns to distinguish.
            let mut cols = conn.prepare("SELECT name FROM pragma_index_info(?)")?;
            let mut col_rows = cols.query(rusqlite::params![&name])?;
            let mut colnames = Vec::new();
            while let Some(c) = col_rows.next()? {
                colnames.push(c.get::<_, String>(0)?);
            }
            if colnames == ["account_id", "uid"] {
                found = true;
            }
        }
        found
    };
    if !has_old {
        return Ok(());
    }
    tracing::info!("migrate: messages UNIQUE(account_id, uid) -> UNIQUE(account_id, folder_id, uid) — Tabelle wird neu aufgebaut");
    conn.execute_batch(
        "BEGIN;
        ALTER TABLE messages RENAME TO messages_old;

        CREATE TABLE messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            folder_id INTEGER DEFAULT 1,
            uid INTEGER NOT NULL,
            message_id TEXT,
            subject TEXT,
            from_addr TEXT,
            to_addr TEXT,
            date TEXT,
            body_text TEXT,
            body_html TEXT,
            flags TEXT DEFAULT '[]',
            ai_summary TEXT,
            ai_priority REAL,
            ai_fraud_score REAL,
            is_read INTEGER NOT NULL DEFAULT 0,
            is_flagged INTEGER NOT NULL DEFAULT 0,
            synced INTEGER NOT NULL DEFAULT 0,
            has_attachments INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(account_id, folder_id, uid)
        );

        INSERT INTO messages (
            id, account_id, folder_id, uid, message_id, subject, from_addr, to_addr, date,
            body_text, body_html, flags, ai_summary, ai_priority, ai_fraud_score,
            is_read, is_flagged, synced, has_attachments, created_at, updated_at
        )
        SELECT
            m.id, m.account_id, m.folder_id, m.uid, m.message_id, m.subject, m.from_addr, m.to_addr, m.date,
            m.body_text, m.body_html, m.flags, m.ai_summary, m.ai_priority, m.ai_fraud_score,
            m.is_read, m.is_flagged, m.synced, m.has_attachments, m.created_at, m.updated_at
        FROM messages_old m
        WHERE m.id IN (
            SELECT MAX(id) FROM messages_old
            GROUP BY account_id, folder_id, uid
        );

        DROP TABLE messages_old;
        COMMIT;",
    )?;
    tracing::info!("migrate: messages-Tabelle neu aufgebaut (folder_id im UNIQUE)");
    Ok(())
}

/// Set up the FTS5 full-text search index over messages and keep it in sync
/// via triggers. Best-effort: if the SQLite build lacks FTS5 the app still
/// works (search just returns no results). Idempotent.
fn init_fts(conn: &Connection) {
    // `content=messages` makes this an external-content FTS5 index: it stores
    // only the inverted index, not a copy of the text, and references the
    // messages row via rowid (= messages.id).
    let created = conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
            subject, from_addr, to_addr, body_text,
            content='messages', content_rowid='id', tokenize='unicode61'
        );

        CREATE TRIGGER IF NOT EXISTS messages_fts_ai AFTER INSERT ON messages BEGIN
            INSERT INTO messages_fts(rowid, subject, from_addr, to_addr, body_text)
            VALUES (new.id, new.subject, new.from_addr, new.to_addr, new.body_text);
        END;

        CREATE TRIGGER IF NOT EXISTS messages_fts_ad AFTER DELETE ON messages BEGIN
            INSERT INTO messages_fts(messages_fts, rowid, subject, from_addr, to_addr, body_text)
            VALUES ('delete', old.id, old.subject, old.from_addr, old.to_addr, old.body_text);
        END;

        CREATE TRIGGER IF NOT EXISTS messages_fts_au AFTER UPDATE ON messages BEGIN
            INSERT INTO messages_fts(messages_fts, rowid, subject, from_addr, to_addr, body_text)
            VALUES ('delete', old.id, old.subject, old.from_addr, old.to_addr, old.body_text);
            INSERT INTO messages_fts(rowid, subject, from_addr, to_addr, body_text)
            VALUES (new.id, new.subject, new.from_addr, new.to_addr, new.body_text);
        END;",
    );

    if let Err(e) = created {
        tracing::warn!("FTS5 nicht verfügbar, Volltextsuche deaktiviert: {}", e);
        return;
    }

    // Verify the external-content index is consistent, and rebuild it from the
    // messages table if not. The FTS5 'rebuild' command atomically
    // reconstructs the whole index from the content table — this is the
    // correct way to (re)populate an external-content index and is self-
    // healing against the "database disk image is malformed" state that a
    // manual backfill could leave behind. Cheap when already consistent.
    let healthy = conn
        .execute_batch("INSERT INTO messages_fts(messages_fts) VALUES('integrity-check');")
        .is_ok();

    if !healthy {
        tracing::warn!("FTS-Index inkonsistent — wird neu aufgebaut");
    }
    if let Err(e) = conn.execute_batch("INSERT INTO messages_fts(messages_fts) VALUES('rebuild');") {
        tracing::warn!("FTS-Rebuild fehlgeschlagen, Volltextsuche evtl. unvollständig: {}", e);
    }
}
