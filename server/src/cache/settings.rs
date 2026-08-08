use rusqlite::{params, Connection};

/// Current settings schema version.
/// Increment this when making backward-compatible schema changes.
/// Migration functions must be added for each version increment.
pub const SETTINGS_VERSION: u32 = 3;

/// Key used to store the settings version in the settings table.
const SETTINGS_VERSION_KEY: &str = "settings_version";

/// Key for the move_to_trash setting (default: "true").
const MOVE_TO_TRASH_KEY: &str = "move_to_trash";

/// Key for the cache retention window in days (default: "0" = archive mode,
/// no automatic pruning of locally cached mail).
pub const RETENTION_DAYS_KEY: &str = "retention_days";

/// Key for the server-side removal sync (default: "false" = archive mode,
/// local copies are never deleted because the provider deleted the mail).
pub const REMOVAL_CHECK_KEY: &str = "removal_check_enabled";

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>, rusqlite::Error> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

pub fn get_move_to_trash(conn: &Connection) -> Result<bool, rusqlite::Error> {
    let val = get_setting(conn, MOVE_TO_TRASH_KEY)?;
    Ok(val.as_deref() == Some("true"))
}

pub fn set_move_to_trash(conn: &Connection, enabled: bool) -> Result<(), rusqlite::Error> {
    set_setting(conn, MOVE_TO_TRASH_KEY, if enabled { "true" } else { "false" })
}

/// Retention window in days for the local message cache. `0` means archive
/// mode: local copies are never pruned automatically.
pub fn get_retention_days(conn: &Connection) -> Result<u32, rusqlite::Error> {
    let val = get_setting(conn, RETENTION_DAYS_KEY)?;
    Ok(val.and_then(|v| v.parse().ok()).unwrap_or(0))
}

/// Whether server-side deletions are mirrored locally (default off).
pub fn get_removal_check_enabled(conn: &Connection) -> Result<bool, rusqlite::Error> {
    let val = get_setting(conn, REMOVAL_CHECK_KEY)?;
    Ok(val.as_deref() == Some("true"))
}

#[allow(dead_code)]
pub fn get_all_settings(conn: &Connection) -> Result<Vec<(String, String)>, rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Migrate settings from any older version to the current version.
/// Safe to call multiple times (idempotent) — checks current version first.
/// Each migration step is additive: old fields are preserved, new ones are added.
pub fn migrate_settings(conn: &Connection) -> Result<(), rusqlite::Error> {
    let current: u32 = get_setting(conn, SETTINGS_VERSION_KEY)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    if current >= SETTINGS_VERSION {
        return Ok(());
    }

    // Run migrations sequentially: current → current+1 → ... → SETTINGS_VERSION
    for version in current..SETTINGS_VERSION {
        match version {
            0 => migrate_v0_to_v1(conn)?,
            1 => migrate_v1_to_v2(conn)?,
            2 => migrate_v2_to_v3(conn)?,
            _ => {
                // Future migrations: add arms here as SETTINGS_VERSION is bumped.
                // Each arm should be additive — never delete or rename existing keys.
            }
        }
    }

    // Persist the updated version
    set_setting(conn, SETTINGS_VERSION_KEY, &SETTINGS_VERSION.to_string())?;
    tracing::info!(
        "Settings migrated from v{} to v{}",
        current,
        SETTINGS_VERSION
    );
    Ok(())
}

/// Migration from v0 (no version key) to v1.
/// v1 is the initial schema — records the version marker.
/// No structural changes needed since the settings table already exists.
fn migrate_v0_to_v1(_conn: &Connection) -> Result<(), rusqlite::Error> {
    // v0 → v1: Initial versioning. All existing settings are forward-compatible.
    // The version key is written by migrate_settings() after all steps complete.
    tracing::info!("Migrating settings from v0 to v1");
    Ok(())
}

/// Migration from v1 to v2: add move_to_trash setting (default: true).
fn migrate_v1_to_v2(conn: &Connection) -> Result<(), rusqlite::Error> {
    tracing::info!("Migrating settings from v1 to v2: adding move_to_trash=true");
    // Only set default if not already explicitly configured
    if get_setting(conn, MOVE_TO_TRASH_KEY)?.is_none() {
        set_setting(conn, MOVE_TO_TRASH_KEY, "true")?;
    }
    Ok(())
}

/// Migration from v2 to v3: archive mode by default.
/// retention_days=0 disables automatic pruning of the local mail cache;
/// removal_check_enabled=false stops mirroring provider-side deletions.
fn migrate_v2_to_v3(conn: &Connection) -> Result<(), rusqlite::Error> {
    tracing::info!(
        "Migrating settings from v2 to v3: archive mode (retention_days=0, removal_check_enabled=false)"
    );
    if get_setting(conn, RETENTION_DAYS_KEY)?.is_none() {
        set_setting(conn, RETENTION_DAYS_KEY, "0")?;
    }
    if get_setting(conn, REMOVAL_CHECK_KEY)?.is_none() {
        set_setting(conn, REMOVAL_CHECK_KEY, "false")?;
    }
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

    #[test]
    fn test_set_and_get_setting() {
        let conn = setup_db();
        set_setting(&conn, "ai_url", "https://test.local/v1").unwrap();
        set_setting(&conn, "ai_model", "gpt-4").unwrap();
        set_setting(&conn, "api_key", "sk-test123").unwrap();
        assert_eq!(get_setting(&conn, "ai_url").unwrap(), Some("https://test.local/v1".into()));
        assert_eq!(get_setting(&conn, "ai_model").unwrap(), Some("gpt-4".into()));
        assert_eq!(get_setting(&conn, "api_key").unwrap(), Some("sk-test123".into()));
    }

    #[test]
    fn test_upsert_overwrites() {
        let conn = setup_db();
        set_setting(&conn, "key", "value1").unwrap();
        set_setting(&conn, "key", "value2").unwrap();
        assert_eq!(get_setting(&conn, "key").unwrap(), Some("value2".into()));
    }

    #[test]
    fn test_get_non_existent_returns_none() {
        let conn = setup_db();
        assert_eq!(get_setting(&conn, "nonexistent").unwrap(), None);
    }

    #[test]
    fn test_get_all_settings() {
        let conn = setup_db();
        set_setting(&conn, "a", "1").unwrap();
        set_setting(&conn, "b", "2").unwrap();
        let all = get_all_settings(&conn).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.contains(&("a".into(), "1".into())));
        assert!(all.contains(&("b".into(), "2".into())));
    }

    // ─── Migration tests ──────────────────────────────────────

    #[test]
    fn test_migrate_fresh_db_sets_version() {
        let conn = setup_db();
        // Fresh DB has no version key
        assert_eq!(get_setting(&conn, SETTINGS_VERSION_KEY).unwrap(), None);

        migrate_settings(&conn).unwrap();

        // Version should now be set
        let version = get_setting(&conn, SETTINGS_VERSION_KEY).unwrap();
        assert_eq!(version, Some(SETTINGS_VERSION.to_string()));
    }

    #[test]
    fn test_migrate_idempotent() {
        let conn = setup_db();

        // Run migration twice
        migrate_settings(&conn).unwrap();
        migrate_settings(&conn).unwrap();

        let version = get_setting(&conn, SETTINGS_VERSION_KEY).unwrap();
        assert_eq!(version, Some(SETTINGS_VERSION.to_string()));
    }

    #[test]
    fn test_migrate_preserves_existing_settings() {
        let conn = setup_db();
        set_setting(&conn, "ai_url", "https://test.local/v1").unwrap();
        set_setting(&conn, "ai_model", "gpt-4").unwrap();
        set_setting(&conn, "api_key", "sk-test123").unwrap();

        migrate_settings(&conn).unwrap();

        // All existing settings preserved
        assert_eq!(get_setting(&conn, "ai_url").unwrap(), Some("https://test.local/v1".into()));
        assert_eq!(get_setting(&conn, "ai_model").unwrap(), Some("gpt-4".into()));
        assert_eq!(get_setting(&conn, "api_key").unwrap(), Some("sk-test123".into()));
        // Version key also present
        assert_eq!(get_setting(&conn, SETTINGS_VERSION_KEY).unwrap(), Some(SETTINGS_VERSION.to_string()));
    }

    #[test]
    fn test_migrate_already_current_version_does_nothing() {
        let conn = setup_db();
        // Pre-set version to current
        set_setting(&conn, SETTINGS_VERSION_KEY, &SETTINGS_VERSION.to_string()).unwrap();
        set_setting(&conn, "ai_url", "https://existing.local/v1").unwrap();

        migrate_settings(&conn).unwrap();

        // Settings unchanged
        assert_eq!(get_setting(&conn, "ai_url").unwrap(), Some("https://existing.local/v1".into()));
        assert_eq!(get_setting(&conn, SETTINGS_VERSION_KEY).unwrap(), Some(SETTINGS_VERSION.to_string()));
    }

    #[test]
    fn test_migrate_from_v0_without_version_key() {
        let conn = setup_db();
        // Simulate v0 state: settings exist but no version key
        set_setting(&conn, "ai_url", "https://v0.local/v1").unwrap();
        set_setting(&conn, "ai_model", "v0-model").unwrap();

        migrate_settings(&conn).unwrap();

        // v0 settings preserved, version set
        assert_eq!(get_setting(&conn, "ai_url").unwrap(), Some("https://v0.local/v1".into()));
        assert_eq!(get_setting(&conn, "ai_model").unwrap(), Some("v0-model".into()));
        assert_eq!(get_setting(&conn, SETTINGS_VERSION_KEY).unwrap(), Some(SETTINGS_VERSION.to_string()));
    }

    // ─── move_to_trash tests ───────────────────────────────────

    #[test]
    fn test_move_to_trash_default_true_after_migration() {
        let conn = setup_db();
        // Fresh DB → migrate → should have move_to_trash=true
        migrate_settings(&conn).unwrap();
        assert!(get_move_to_trash(&conn).unwrap());
    }

    #[test]
    fn test_set_move_to_trash_false() {
        let conn = setup_db();
        migrate_settings(&conn).unwrap();
        set_move_to_trash(&conn, false).unwrap();
        assert!(!get_move_to_trash(&conn).unwrap());
    }

    #[test]
    fn test_set_move_to_trash_true() {
        let conn = setup_db();
        migrate_settings(&conn).unwrap();
        set_move_to_trash(&conn, false).unwrap();
        set_move_to_trash(&conn, true).unwrap();
        assert!(get_move_to_trash(&conn).unwrap());
    }

    #[test]
    fn test_move_to_trash_preserved_across_migration() {
        let conn = setup_db();
        migrate_settings(&conn).unwrap();
        // User explicitly disables trash
        set_move_to_trash(&conn, false).unwrap();
        // Re-run migration — should NOT reset to true
        migrate_settings(&conn).unwrap();
        assert!(!get_move_to_trash(&conn).unwrap());
    }

    #[test]
    fn test_migrate_v1_to_v2_sets_default() {
        let conn = setup_db();
        // Simulate v1 state: version=1, no move_to_trash
        set_setting(&conn, SETTINGS_VERSION_KEY, "1").unwrap();
        set_setting(&conn, "ai_url", "https://v1.local/v1").unwrap();

        migrate_settings(&conn).unwrap();

        // move_to_trash should be set to true
        assert!(get_move_to_trash(&conn).unwrap());
        // Existing settings preserved
        assert_eq!(get_setting(&conn, "ai_url").unwrap(), Some("https://v1.local/v1".into()));
        // Version bumped to current
        assert_eq!(get_setting(&conn, SETTINGS_VERSION_KEY).unwrap(), Some(SETTINGS_VERSION.to_string()));
    }

    // ─── archive-mode settings (v3) tests ───────────────────────

    #[test]
    fn test_retention_days_default_zero_after_migration() {
        let conn = setup_db();
        migrate_settings(&conn).unwrap();
        assert_eq!(get_retention_days(&conn).unwrap(), 0);
    }

    #[test]
    fn test_retention_days_explicit_value() {
        let conn = setup_db();
        migrate_settings(&conn).unwrap();
        set_setting(&conn, RETENTION_DAYS_KEY, "90").unwrap();
        assert_eq!(get_retention_days(&conn).unwrap(), 90);
        set_setting(&conn, RETENTION_DAYS_KEY, "0").unwrap();
        assert_eq!(get_retention_days(&conn).unwrap(), 0);
    }

    #[test]
    fn test_removal_check_default_disabled() {
        let conn = setup_db();
        migrate_settings(&conn).unwrap();
        assert!(!get_removal_check_enabled(&conn).unwrap());
    }

    #[test]
    fn test_removal_check_explicit_enable() {
        let conn = setup_db();
        migrate_settings(&conn).unwrap();
        set_setting(&conn, REMOVAL_CHECK_KEY, "true").unwrap();
        assert!(get_removal_check_enabled(&conn).unwrap());
        set_setting(&conn, REMOVAL_CHECK_KEY, "false").unwrap();
        assert!(!get_removal_check_enabled(&conn).unwrap());
    }

    #[test]
    fn test_migrate_v2_to_v3_sets_archive_defaults() {
        let conn = setup_db();
        // Simulate v2 state: version=2, move_to_trash set, no archive keys
        set_setting(&conn, SETTINGS_VERSION_KEY, "2").unwrap();
        set_setting(&conn, MOVE_TO_TRASH_KEY, "false").unwrap();
        set_setting(&conn, "ai_url", "https://v2.local/v1").unwrap();

        migrate_settings(&conn).unwrap();

        assert_eq!(get_retention_days(&conn).unwrap(), 0);
        assert!(!get_removal_check_enabled(&conn).unwrap());
        // Existing settings preserved
        assert!(!get_move_to_trash(&conn).unwrap());
        assert_eq!(get_setting(&conn, "ai_url").unwrap(), Some("https://v2.local/v1".into()));
    }
}
