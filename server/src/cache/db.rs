use rusqlite::{params, Connection};

/// Current schema version. Bump this and add a numbered forward-migration
/// step in `init_db` when the schema changes. v1 is the baseline: the schema
/// as of 26.9.142, applied as a tolerant catch-up for legacy DBs.
pub const CURRENT_SCHEMA_VERSION: i64 = 3;

pub fn init_db(conn: &Connection) -> Result<(), rusqlite::Error> {
    let user_version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0);

    // 1. Table bootstrap — always run (idempotent `IF NOT EXISTS`; needed for
    //    fresh DBs, harmless on existing ones).
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
            part_index INTEGER NOT NULL DEFAULT 0,
            filename TEXT NOT NULL,
            content_type TEXT NOT NULL,
            size INTEGER NOT NULL DEFAULT 0,
            content TEXT,
            content_cached INTEGER NOT NULL DEFAULT 0,
            cached_at TEXT NOT NULL DEFAULT (datetime('now')),
            disk_path TEXT,
            sha256 TEXT,
            UNIQUE(message_id, part_index)
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

        -- Provider-op queue (local-first mutations): the API writes the
        -- SQLite change, invalidates caches and returns immediately; this
        -- queue replays the mutation on the IMAP provider in the background.
        --   kind: 'flag' (STORE) | 'move' (COPY+STORE) | 'delete' (STORE \\Deleted)
        --   state: pending → done | failed
        CREATE TABLE IF NOT EXISTS provider_ops (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            kind TEXT NOT NULL,
            uid INTEGER NOT NULL,
            folder TEXT NOT NULL,
            target_folder TEXT,
            flag TEXT,
            set_flag INTEGER NOT NULL DEFAULT 1,
            state TEXT NOT NULL DEFAULT 'pending',
            attempts INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_provider_ops_state ON provider_ops(state, account_id);

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

        -- CalDAV: calendar collections (Phase 0)
        CREATE TABLE IF NOT EXISTS calendars (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            url TEXT UNIQUE NOT NULL,
            display_name TEXT,
            description TEXT,
            color TEXT,
            sync_token TEXT NOT NULL DEFAULT '',
            last_sync_at TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_calendars_url ON calendars(url);

        -- CalDAV: events (parsed VEVENTs)
        CREATE TABLE IF NOT EXISTS events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            calendar_id INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
            uid TEXT NOT NULL,
            url TEXT NOT NULL,
            summary TEXT,
            description TEXT,
            location TEXT,
            start_at TEXT NOT NULL,
            end_at TEXT,
            all_day INTEGER NOT NULL DEFAULT 0,
            organizer TEXT,
            status TEXT,
            sequence INTEGER NOT NULL DEFAULT 0,
            rrule TEXT,
            alarms INTEGER NOT NULL DEFAULT 0,
            etag TEXT,
            ics_raw TEXT NOT NULL,
            synced_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(calendar_id, uid)
        );
        CREATE INDEX IF NOT EXISTS idx_events_uid ON events(uid);
        CREATE INDEX IF NOT EXISTS idx_events_start ON events(start_at);
        CREATE INDEX IF NOT EXISTS idx_events_calendar ON events(calendar_id);

        -- CalDAV: event attendees
        CREATE TABLE IF NOT EXISTS event_attendees (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
            email TEXT NOT NULL,
            name TEXT,
            part_stat TEXT,
            rsvp INTEGER NOT NULL DEFAULT 0,
            UNIQUE(event_id, email)
        );
        CREATE INDEX IF NOT EXISTS idx_event_attendees_email ON event_attendees(email);

        -- CalDAV: invitations (organizer/attendee status tracking)
        CREATE TABLE IF NOT EXISTS invitations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_uid TEXT NOT NULL,
            organizer TEXT,
            attendee_email TEXT NOT NULL,
            method TEXT NOT NULL DEFAULT 'REQUEST',
            status TEXT NOT NULL DEFAULT 'NEEDS-ACTION',
            sequence INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(event_uid, attendee_email)
        );
        CREATE INDEX IF NOT EXISTS idx_invitations_attendee ON invitations(attendee_email);

        -- CalDAV: todos / tasks (schema now, populated in a later phase)
        CREATE TABLE IF NOT EXISTS todos (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            calendar_id INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
            uid TEXT NOT NULL,
            url TEXT NOT NULL,
            summary TEXT,
            description TEXT,
            due_at TEXT,
            completed_at TEXT,
            status TEXT NOT NULL DEFAULT 'NEEDS-ACTION',
            priority INTEGER,
            ics_raw TEXT NOT NULL,
            synced_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(calendar_id, uid)
        );
        CREATE INDEX IF NOT EXISTS idx_todos_uid ON todos(uid);
        CREATE INDEX IF NOT EXISTS idx_todos_due ON todos(due_at);
        ",
    )?;

    // Agentic assistant (v2) tables — ActionPlans, sessions, audit (Concept §5.3/§5.4/§6.10).
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS ai_action_plans (
            id TEXT PRIMARY KEY,
            session_id TEXT,
            origin TEXT NOT NULL,
            source_message_id INTEGER,
            status TEXT NOT NULL,
            steps_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            expires_at TEXT NOT NULL,
            executed_at TEXT,
            result_json TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_ai_plans_status ON ai_action_plans(status);
        CREATE INDEX IF NOT EXISTS idx_ai_plans_session ON ai_action_plans(session_id);
        CREATE TABLE IF NOT EXISTS ai_sessions (
            id TEXT PRIMARY KEY,
            created_at TEXT,
            last_active TEXT,
            locale TEXT,
            messages_json TEXT NOT NULL DEFAULT '[]'
        );
        CREATE INDEX IF NOT EXISTS idx_ai_sessions_active ON ai_sessions(last_active);
        CREATE TABLE IF NOT EXISTS ai_audit (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT,
            origin TEXT,
            event TEXT NOT NULL,
            detail TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_ai_audit_session ON ai_audit(session_id);

        -- Meetings (Insilo cross-app drop): one row per meeting summary file
        -- that Insilo exports into the shared appCommon directory. The
        -- scanner (sync/insilo.rs) upserts by insilo_id and soft-deletes
        -- rows whose file disappeared. body_md holds the Markdown without
        -- frontmatter; participants/tags are JSON arrays of names.
        CREATE TABLE IF NOT EXISTS meetings (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            insilo_id     TEXT NOT NULL UNIQUE,
            path          TEXT NOT NULL,
            sha256        TEXT NOT NULL,
            title         TEXT NOT NULL,
            participants  TEXT NOT NULL DEFAULT '[]',
            tags          TEXT NOT NULL DEFAULT '[]',
            meeting_date  TEXT NOT NULL,
            duration_min  INTEGER NOT NULL DEFAULT 0,
            language      TEXT NOT NULL DEFAULT 'de',
            template      TEXT NOT NULL DEFAULT '',
            source_url    TEXT NOT NULL DEFAULT '',
            body_md       TEXT NOT NULL,
            deleted       INTEGER NOT NULL DEFAULT 0,
            first_seen_at TEXT NOT NULL,
            updated_at    TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_meetings_date ON meetings(meeting_date DESC);

        -- User-deleted meetings: the Insilo scan must not re-import these
        -- while the export file still exists (the upsert would otherwise
        -- reset `deleted = 0`). Keyed by insilo_id so a re-export of the
        -- same meeting stays hidden.
        CREATE TABLE IF NOT EXISTS meetings_ignored (
            insilo_id   TEXT PRIMARY KEY,
            deleted_at  TEXT NOT NULL
        );
        ",
    )?;

    // 2. Baseline (v1) — tolerant catch-up for legacy DBs (user_version < 1).
    //    Deduplicated `ADD COLUMN`s (each applied once) + one-time schema
    //    indexes/backfill + the UID-constraint rebuild. Best-effort so a
    //    partially-migrated DB converges to v1. Fresh DBs already have every
    //    column from the CREATE above, so this is a no-op for them.
    if user_version < 1 {
        baseline_v1(conn)?;
        conn.pragma_update(None, "user_version", 1)?;
    }

    // 3. Forward migrations — strict, versioned. Bring the schema from the
    //    current user_version up to CURRENT_SCHEMA_VERSION. Each step runs
    //    once and must not fail silently (propagate the error). Add new steps
    //    here as the schema evolves (bump CURRENT_SCHEMA_VERSION to match):
    //        if user_version < 2 {
    //            conn.execute("ALTER TABLE ... ADD COLUMN ...")?;
    //            conn.pragma_update(None, "user_version", 2)?;
    //        }
    // v2: meetings table (Insilo cross-app drop). The table itself is created
    // idempotently in the bootstrap above, so this step only marks the
    // version for DBs that predate it.
    if user_version < 2 {
        conn.pragma_update(None, "user_version", 2)?;
    }
    // v3: meetings_ignored table (user-deleted meetings survive re-scans).
    // The table itself is created idempotently in the bootstrap above.
    if user_version < 3 {
        conn.pragma_update(None, "user_version", 3)?;
    }

    // 4. Recurring startup work — idempotent + self-healing, runs every boot
    //    (NOT gated by user_version): new accounts/folders can appear after
    //    v1, and the FTS index must re-verify after any table rebuild.
    // Legacy sync paths could store the plain-text body into body_html instead
    // of NULL; where the two columns are identical the stored "html" is just
    // the text — drop it so the message renders from body_text.
    let _ = conn.execute(
        "UPDATE messages SET body_html = NULL
         WHERE body_html IS NOT NULL AND body_html != ''
           AND body_html = body_text AND body_text IS NOT NULL AND body_text != ''",
        [],
    );
    // The "Gesendet"/"Sent" folder is a LOCAL folder (full sent history kept
    // locally, not mirrored against the provider). Re-applied every boot so
    // newly-synced sent folders are also converted.
    let _ = conn.execute(
        "UPDATE folders SET local_only = 1, imap_id = NULL
         WHERE name IN ('Gesendet', 'Sent', 'Gesendete Elemente')",
        [],
    );
    // Consolidate provider trash folders into the single local "Trash" folder
    // (re-applied every boot so new accounts are also handled).
    migrate_provider_trash_folders(conn)?;
    init_fts(conn);

    Ok(())
}

/// Baseline schema (v1): the layout as of 26.9.142, applied once to legacy
/// DBs (`user_version < 1`). Every `ADD COLUMN` appears exactly once (the
/// duplicates from earlier ad-hoc migrations are removed); all statements are
/// best-effort so a partially-migrated DB converges cleanly to v1.
fn baseline_v1(conn: &Connection) -> Result<(), rusqlite::Error> {
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN cc_addr TEXT", []);
    let _ = conn.execute("ALTER TABLE accounts ADD COLUMN smtp_username TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE accounts ADD COLUMN smtp_password TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN has_attachments INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN is_flagged INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN is_urgent INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN raw_path TEXT", []);
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN raw_sha256 TEXT", []);
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN ai_followups TEXT", []);
    let _ = conn.execute("ALTER TABLE calendars ADD COLUMN caldav_account_id TEXT NOT NULL DEFAULT 'default'", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_calendars_caldav_account ON calendars(caldav_account_id)", []);
    let _ = conn.execute("ALTER TABLE message_attachments ADD COLUMN disk_path TEXT", []);
    let _ = conn.execute("ALTER TABLE message_attachments ADD COLUMN part_index INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE message_attachments ADD COLUMN sha256 TEXT", []);
    let _ = conn.execute("ALTER TABLE voice_settings ADD COLUMN tts_enabled INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE voice_settings ADD COLUMN tts_url TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE voice_settings ADD COLUMN tts_key TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE voice_settings ADD COLUMN tts_model TEXT NOT NULL DEFAULT ''", []);
    // Backfill part_index for pre-existing rows: ordinal position per message,
    // ordered by id (id ascending == the historical insertion/BODYSTRUCTURE order).
    let _ = conn.execute(
        "UPDATE message_attachments SET part_index = (
            SELECT COUNT(*) FROM message_attachments a2
            WHERE a2.message_id = message_attachments.message_id AND a2.id <= message_attachments.id
        ) - 1",
        [],
    );
    let _ = conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_ma_message_part ON message_attachments(message_id, part_index)",
        [],
    );
    let _ = conn.execute("ALTER TABLE folders ADD COLUMN local_only INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE accounts ADD COLUMN sync_mode TEXT NOT NULL DEFAULT 'mirror'", []);
    let _ = conn.execute("ALTER TABLE accounts ADD COLUMN trash_retention_days INTEGER NOT NULL DEFAULT 30", []);
    let _ = conn.execute("ALTER TABLE accounts ADD COLUMN imap_insecure INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE settings ADD COLUMN photo_data BLOB", []);
    let _ = conn.execute("ALTER TABLE settings ADD COLUMN photo_type TEXT", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_messages_raw ON messages(raw_path)", []);

    // One-time table rebuild: UNIQUE(account_id, uid) -> UNIQUE(account_id,
    // folder_id, uid). Idempotent (no-op once the constraint is correct).
    migrate_messages_uid_constraint(conn)?;

    Ok(())
}

/// Consolidate provider trash folders (by locale) into the single local
/// "Trash" folder: move their messages + sync cursors over, then drop the
/// provider-named folder rows. Idempotent.
fn migrate_provider_trash_folders(conn: &Connection) -> Result<(), rusqlite::Error> {
    let trash_names = [
        "Trash",
        "Gelöscht",
        "Gelöschte Elemente",
        "Papierkorb",
        "Deleted Messages",
        "Deleted",
        "Deleted Items",
        "INBOX.Trash",
        "INBOX.Gelöscht",
        "INBOX.Papierkorb",
        "INBOX.Deleted Messages",
        "INBOX.Deleted",
    ];

    // Ensure a local Trash row exists per account, then consolidate every
    // provider trash folder of that account into it. Idempotent: rows that
    // are already "Trash" (or already migrated) are skipped.
    let account_ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM accounts")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        out
    };

    for account_id in &account_ids {
        let trash_id: i64 = conn
            .query_row(
                "SELECT id FROM folders WHERE account_id = ?1 AND name = 'Trash'",
                params![account_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let trash_id = if trash_id == 0 {
            conn.execute(
                "INSERT INTO folders (account_id, name, imap_id, local_only) VALUES (?1, 'Trash', NULL, 1)",
                params![account_id],
            )?;
            conn.last_insert_rowid()
        } else {
            // Make sure the local Trash is marked local_only (the provider's
            // own "Trash" folder is mirrored INTO this row by the sync).
            let _ = conn.execute(
                "UPDATE folders SET local_only = 1 WHERE account_id = ?1 AND name = 'Trash'",
                params![account_id],
            );
            trash_id
        };

        // Move messages from every provider trash folder of this account.
        for name in &trash_names {
            if *name == "Trash" {
                continue;
            }
            let src_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2 AND local_only = 0",
                    params![account_id, name],
                    |row| row.get(0),
                )
                .ok();
            let Some(src_id) = src_id else { continue };

            // Fold messages with an identical message_id already in Trash
            // (the local copy wins), others move with a free uid (offset on
            // collision, mirroring update_folder_from semantics).
            let mut src_msgs: Vec<(i64, i64, Option<String>)> = {
                let mut stmt = conn.prepare(
                    "SELECT id, uid, message_id FROM messages
                     WHERE account_id = ?1 AND folder_id = ?2",
                )?;
                let rows = stmt.query_map(params![account_id, src_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                out
            };
            src_msgs.sort_by_key(|(_, uid, _)| *uid);

            for (msg_id, uid, message_id) in &src_msgs {
                if let Some(mid) = message_id {
                    let dup: bool = conn
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM messages
                             WHERE account_id = ?1 AND folder_id = ?2 AND message_id = ?3)",
                            params![account_id, trash_id, mid],
                            |row| row.get(0),
                        )
                        .unwrap_or(false);
                    if dup {
                        conn.execute("DELETE FROM messages WHERE id = ?1", params![msg_id])?;
                        continue;
                    }
                }
                // Find a free uid in Trash.
                let mut new_uid = *uid;
                loop {
                    let occupied: bool = conn
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM messages
                             WHERE account_id = ?1 AND folder_id = ?2 AND uid = ?3)",
                            params![account_id, trash_id, new_uid],
                            |row| row.get(0),
                        )
                        .unwrap_or(false);
                    if !occupied {
                        conn.execute(
                            "UPDATE messages SET folder_id = ?1, uid = ?2 WHERE id = ?3",
                            params![trash_id, new_uid, msg_id],
                        )?;
                        break;
                    }
                    new_uid += 100000;
                }
            }

            // Move the sync cursor over, keeping the highest value.
            conn.execute(
                "INSERT OR IGNORE INTO sync_state (account_id, folder_name, last_uid, highest_modseq)
                 VALUES (?1, 'Trash', 0, 0)",
                params![account_id],
            )?;
            conn.execute(
                "UPDATE sync_state
                 SET last_uid = MAX(last_uid, (SELECT COALESCE(MAX(last_uid), 0) FROM sync_state WHERE account_id = ?1 AND folder_name = ?2)),
                     highest_modseq = MAX(highest_modseq, (SELECT COALESCE(MAX(highest_modseq), 0) FROM sync_state WHERE account_id = ?1 AND folder_name = ?2))
                 WHERE account_id = ?1 AND folder_name = 'Trash'",
                params![account_id, name],
            )?;
            conn.execute(
                "DELETE FROM sync_state WHERE account_id = ?1 AND folder_name = ?2",
                params![account_id, name],
            )?;

            // Drop the provider-named folder row (messages already moved).
            conn.execute("DELETE FROM folders WHERE id = ?1", params![src_id])?;
            tracing::info!(
                "Migration: Provider-Papierkorb '{}' (Konto {}) nach 'Trash' konsolidiert",
                name, account_id
            );
        }
    }
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
    // Stale FTS triggers (pointing at messages_old after a previous partial
    // rebuild) must be dropped even when no rebuild is needed — init_fts()
    // recreates them afterwards. Idempotent.
    let _ = conn.execute("DROP TRIGGER IF EXISTS messages_fts_ai", []);
    let _ = conn.execute("DROP TRIGGER IF EXISTS messages_fts_au", []);
    let _ = conn.execute("DROP TRIGGER IF EXISTS messages_fts_ad", []);
    if !has_old {
        return Ok(());
    }
    tracing::info!("migrate: messages UNIQUE(account_id, uid) -> UNIQUE(account_id, folder_id, uid) — Tabelle wird neu aufgebaut");
    // Drop FTS triggers first: SQLite rewrites trigger/VIEW references during
    // RENAME TO, so the old triggers would point at messages_old afterwards
    // and fire with "no such table" on every INSERT. init_fts() recreates
    // them after the rebuild.
    let _ = conn.execute("DROP TRIGGER IF EXISTS messages_fts_ai", []);
    let _ = conn.execute("DROP TRIGGER IF EXISTS messages_fts_au", []);
    let _ = conn.execute("DROP TRIGGER IF EXISTS messages_fts_ad", []);
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
            raw_path TEXT,
            raw_sha256 TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(account_id, folder_id, uid)
        );

        INSERT INTO messages (
            id, account_id, folder_id, uid, message_id, subject, from_addr, to_addr, date,
            body_text, body_html, flags, ai_summary, ai_priority, ai_fraud_score,
            is_read, is_flagged, synced, has_attachments, raw_path, raw_sha256, created_at, updated_at
        )
        SELECT
            m.id, m.account_id, m.folder_id, m.uid, m.message_id, m.subject, m.from_addr, m.to_addr, m.date,
            m.body_text, m.body_html, m.flags, m.ai_summary, m.ai_priority, m.ai_fraud_score,
            m.is_read, m.is_flagged, m.synced, m.has_attachments, m.raw_path, m.raw_sha256, m.created_at, m.updated_at
        FROM messages_old m
        WHERE m.id IN (
            SELECT MAX(id) FROM messages_old
            GROUP BY account_id, folder_id, uid
        );

        DROP TABLE messages_old;
        COMMIT;",
    )?;
    // The rebuild dropped the FTS triggers; init_fts() (called after this
    // migration) recreates them. Drop the stale ones again in case a previous
    // partial run left references to messages_old behind (idempotent).
    let _ = conn.execute("DROP TRIGGER IF EXISTS messages_fts_ai", []);
    let _ = conn.execute("DROP TRIGGER IF EXISTS messages_fts_au", []);
    let _ = conn.execute("DROP TRIGGER IF EXISTS messages_fts_ad", []);
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

    init_meetings_fts(conn);
}

/// FTS5 index over meetings (title + body), mirroring `messages_fts`.
/// External-content, trigger-synced, best-effort. Idempotent.
fn init_meetings_fts(conn: &Connection) {
    let created = conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS meetings_fts USING fts5(
            title, body_md,
            content='meetings', content_rowid='id', tokenize='unicode61'
        );

        CREATE TRIGGER IF NOT EXISTS meetings_fts_ai AFTER INSERT ON meetings BEGIN
            INSERT INTO meetings_fts(rowid, title, body_md)
            VALUES (new.id, new.title, new.body_md);
        END;

        CREATE TRIGGER IF NOT EXISTS meetings_fts_ad AFTER DELETE ON meetings BEGIN
            INSERT INTO meetings_fts(meetings_fts, rowid, title, body_md)
            VALUES ('delete', old.id, old.title, old.body_md);
        END;

        CREATE TRIGGER IF NOT EXISTS meetings_fts_au AFTER UPDATE ON meetings BEGIN
            INSERT INTO meetings_fts(meetings_fts, rowid, title, body_md)
            VALUES ('delete', old.id, old.title, old.body_md);
            INSERT INTO meetings_fts(rowid, title, body_md)
            VALUES (new.id, new.title, new.body_md);
        END;",
    );

    if let Err(e) = created {
        tracing::warn!("FTS5 (meetings) nicht verfügbar, Meeting-Suche deaktiviert: {}", e);
        return;
    }

    if let Err(e) = conn.execute_batch("INSERT INTO meetings_fts(meetings_fts) VALUES('rebuild');") {
        tracing::warn!("Meetings-FTS-Rebuild fehlgeschlagen: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap()
    }

    fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).unwrap();
        let mut rows = stmt.query_map([], |r| r.get::<_, String>(1)).unwrap();
        rows.any(|r| r.unwrap() == column)
    }

    fn table_column_count(conn: &Connection, table: &str) -> i64 {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).unwrap();
        let rows = stmt.query_map([], |r| r.get::<_, i32>(0)).unwrap();
        rows.count() as i64
    }

    #[test]
    fn fresh_db_reaches_v1() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        assert_eq!(user_version(&conn), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn fresh_db_has_meetings_ignored_table() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'meetings_ignored'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn init_db_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let tables = [
            "messages", "voice_settings", "accounts", "folders", "settings", "message_attachments",
        ];
        let before: i64 = tables.iter().map(|t| table_column_count(&conn, t)).sum();
        // Second call must be a no-op (schema unchanged, version stays put).
        init_db(&conn).unwrap();
        assert_eq!(user_version(&conn), CURRENT_SCHEMA_VERSION);
        let after: i64 = tables.iter().map(|t| table_column_count(&conn, t)).sum();
        assert_eq!(before, after);
    }

    #[test]
    fn legacy_db_upgrades_to_v1_with_tts_columns() {
        let conn = Connection::open_in_memory().unwrap();
        // Pre-create voice_settings in its pre-Phase-D shape (no tts_* columns),
        // simulating a 26.9.142-era DB at user_version 0.
        conn.execute_batch(
            "CREATE TABLE voice_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                enabled INTEGER NOT NULL DEFAULT 0,
                stt_url TEXT NOT NULL DEFAULT '',
                stt_key TEXT NOT NULL DEFAULT '',
                stt_model TEXT NOT NULL DEFAULT 'Systran/faster-whisper-small'
            );",
        )
        .unwrap();
        init_db(&conn).unwrap();
        assert_eq!(user_version(&conn), CURRENT_SCHEMA_VERSION);
        for col in ["tts_enabled", "tts_url", "tts_key", "tts_model"] {
            assert!(column_exists(&conn, "voice_settings", col), "missing {col}");
        }
    }

    #[test]
    fn no_duplicate_add_column_statements() {
        let src = std::fs::read_to_string(format!(
            "{}/src/cache/db.rs",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        // Every `ADD COLUMN <name>` must appear at most once (the dedup
        // guarantee of the v1 baseline).
        let mut seen = std::collections::HashSet::new();
        for line in src.lines() {
            if line.trim_start().starts_with("//") {
                continue; // skip comments (e.g. the forward-migration example)
            }
            if let Some(pos) = line.find("ALTER TABLE") {
                let rest = &line[pos + "ALTER TABLE".len()..];
                if let Some(col_pos) = rest.find(" ADD COLUMN ") {
                    let after = &rest[col_pos + " ADD COLUMN ".len()..];
                    let col = after.trim().split_whitespace().next().unwrap_or("");
                    if !col.is_empty() {
                        assert!(seen.insert(col.to_string()), "duplicate ADD COLUMN {col}");
                    }
                }
            }
        }
        assert!(!seen.is_empty(), "expected at least one ADD COLUMN");
    }
}
