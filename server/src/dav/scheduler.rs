use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::AppState;

pub async fn start_carddav_sync(state: Arc<AppState>, mut shutdown_rx: mpsc::Receiver<()>) {

    // Check if CardDAV is configured
    let settings = {
        let guard = state.carddav_settings.read();
        guard.clone()
    };

    let (interval_minutes, has_settings) = match settings {
        Some(s) => (s.sync_interval_minutes, !s.url.is_empty()),
        None => (30, false),
    };

    if !has_settings {
        tracing::info!("CardDAV-Sync: nicht konfiguriert, Scheduler nicht gestartet");
        return;
    }

    tracing::info!("CardDAV-Sync: gestartet (Interval: {} Min)", interval_minutes);

    let interval = Duration::from_secs(interval_minutes * 60);
    let mut interval = tokio::time::interval(interval);

    // Initial sync after 5 seconds
    tokio::time::sleep(Duration::from_secs(5)).await;
    do_sync(&state).await;

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                tracing::info!("CardDAV-Sync: gestoppt");
                break;
            }
            _ = interval.tick() => {
                do_sync(&state).await;
            }
        }
    }
}

async fn do_sync(state: &AppState) {
    let settings = {
        let guard = state.carddav_settings.read();
        guard.clone()
    };

    let settings = match settings {
        Some(s) if !s.url.is_empty() => s,
        _ => return,
    };

    let client = crate::dav::carddav::CardDavClient::new(settings);

    let old_token = {
        let guard = state.carddav_sync_token.read();
        guard.clone()
    };

    let result = if old_token.is_empty() {
        client.fetch_all().await
    } else {
        match client.sync_incremental(&old_token).await {
            Ok((added, deleted, new_token)) => {
                // Delete removed contacts
                if !deleted.is_empty() {
                    if let Ok(db_guard) = crate::db::get_db_inner(state) {
                        if let Some(conn) = db_guard.as_ref() {
                            let mut stmt = match conn.prepare("DELETE FROM contacts WHERE vcard_uid = ?1") {
                                Ok(s) => s,
                                Err(e) => {
                                    tracing::warn!("CardDAV-Sync: DB-Prepare fehlgeschlagen: {}", e);
                                    return;
                                }
                            };
                            for uid in &deleted {
                                if let Err(e) = stmt.execute(&[uid]) {
                                    tracing::warn!("CardDAV-Sync: Delete fehlgeschlagen: {}", e);
                                }
                            }
                        }
                    }
                }

                // Store new sync token
                {
                    let mut guard = state.carddav_sync_token.write();
                    *guard = new_token.clone();
                }

                // Save contacts to DB
                if let Ok(mut db_guard) = crate::db::get_db_inner(state) {
                    if let Some(conn) = db_guard.as_mut() {
                        save_contacts_to_db(conn, &added).unwrap_or_else(|e| {
                            tracing::warn!("CardDAV-Sync: DB-Save fehlgeschlagen: {}", e);
                        });
                    }
                }

                tracing::info!("CardDAV-Sync: {} Kontakte aktualisiert", added.len());
                return;
            }
            Err(e) => {
                tracing::warn!("CardDAV-Sync: inkrementell fehlgeschlagen, versuche Full-Sync: {}", e);
                client.fetch_all().await
            }
        }
    };

    match result {
        Ok((contacts, new_token)) => {
            // Store new sync token
            {
                let mut guard = state.carddav_sync_token.write();
                *guard = new_token.clone();
            }

            // Save contacts to DB
            if let Ok(mut db_guard) = crate::db::get_db_inner(state) {
                if let Some(conn) = db_guard.as_mut() {
                    save_contacts_to_db(conn, &contacts).unwrap_or_else(|e| {
                        tracing::warn!("CardDAV-Sync: DB-Save fehlgeschlagen: {}", e);
                    });
                }
            }

            tracing::info!("CardDAV-Sync: {} Kontakte synchronisiert", contacts.len());
        }
        Err(e) => {
            tracing::warn!("CardDAV-Sync: fehlgeschlagen: {}", e);
        }
    }
}

/// CalDAV background sync (Phase 0). Mirrors the CardDAV scheduler but drives
/// the CalDAV client and persists calendars + events. Delegates the actual
/// sync to the shared `api::calendars::do_caldav_sync` so manual and
/// background syncs behave identically.
pub async fn start_caldav_sync(state: Arc<AppState>, mut shutdown_rx: mpsc::Receiver<()>) {
    let settings = {
        let guard = state.caldav_settings.read();
        guard.clone()
    };

    let (interval_minutes, has_settings) = match settings {
        Some(s) => (s.sync_interval_minutes, !s.url.is_empty()),
        None => (30, false),
    };

    if !has_settings {
        tracing::info!("CalDAV-Sync: nicht konfiguriert, Scheduler nicht gestartet");
        return;
    }

    tracing::info!("CalDAV-Sync: gestartet (Interval: {} Min)", interval_minutes);

    let interval = Duration::from_secs(interval_minutes * 60);
    let mut interval = tokio::time::interval(interval);

    tokio::time::sleep(Duration::from_secs(8)).await;
    run_caldav_sync(&state).await;

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                tracing::info!("CalDAV-Sync: gestoppt");
                break;
            }
            _ = interval.tick() => {
                run_caldav_sync(&state).await;
            }
        }
    }
}

async fn run_caldav_sync(state: &AppState) {
    match crate::api::calendars::do_caldav_sync(state).await {
        Ok(_) => {}
        Err(e) => tracing::warn!("CalDAV-Sync: fehlgeschlagen: {}", e.0),
    }
}

fn save_contacts_to_db(
    conn: &mut rusqlite::Connection,
    contacts: &[crate::dav::Contact],
) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut stmt = tx.prepare(
        "INSERT INTO contacts (vcard_uid, given_name, family_name, display_name, email, phone, organization, vcard_raw, synced_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
         ON CONFLICT(vcard_uid) DO UPDATE SET
           given_name = excluded.given_name,
           family_name = excluded.family_name,
           display_name = excluded.display_name,
           email = excluded.email,
           phone = excluded.phone,
           organization = excluded.organization,
           vcard_raw = excluded.vcard_raw,
           synced_at = datetime('now')"
    ).map_err(|e| e.to_string())?;

    for contact in contacts {
        let params: &[&dyn rusqlite::types::ToSql] = &[
            &contact.vcard_uid as &dyn rusqlite::types::ToSql,
            &contact.given_name,
            &contact.family_name,
            &contact.display_name,
            &contact.email,
            &contact.phone,
            &contact.organization,
            &contact.vcard_raw,
        ];
        stmt.execute(params).map_err(|e| e.to_string())?;
    }

    drop(stmt);
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}
