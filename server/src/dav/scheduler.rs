use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::AppState;

/// CardDAV background sync. It keeps running when nothing is configured
/// yet, so an address book connected later is synced without a restart
/// (before, the scheduler ended at boot and the contacts never came).
pub async fn start_carddav_sync(state: Arc<AppState>, mut shutdown_rx: mpsc::Receiver<()>) {
    tracing::info!("CardDAV-Sync: Ticker gestartet (Prüfung alle 60s)");
    let mut letzter: Option<(tokio::time::Instant, String)> = None;
    // Initial pass shortly after boot.
    tokio::time::sleep(Duration::from_secs(5)).await;

    loop {
        let settings = state.carddav_settings.read().clone();
        if let Some(s) = settings.filter(|s| !s.url.is_empty()) {
            let intervall = Duration::from_secs(s.sync_interval_minutes.max(1) * 60);
            // Due after its interval, or at once when the address changed.
            let faellig = match &letzter {
                Some((t, url)) => url != &s.url || tokio::time::Instant::now().duration_since(*t) >= intervall,
                None => true,
            };
            if faellig {
                letzter = Some((tokio::time::Instant::now(), s.url.clone()));
                if let Err(e) = do_sync(&state).await {
                    tracing::warn!("CardDAV-Sync: fehlgeschlagen: {}", e);
                }
            }
        }
        tokio::select! {
            _ = shutdown_rx.recv() => {
                tracing::info!("CardDAV-Sync: gestoppt");
                break;
            }
            _ = tokio::time::sleep(Duration::from_secs(60)) => {}
        }
    }
}

/// One CardDAV sync: the changes since the last token (or everything), into
/// the contacts table. Shared by the ticker and "Aktualisieren"; the manual
/// sync used to fetch the contacts and store only the token.
pub async fn do_sync(state: &AppState) -> Result<usize, String> {
    let settings = {
        let guard = state.carddav_settings.read();
        guard.clone()
    };

    let settings = match settings {
        Some(s) if !s.url.is_empty() => s,
        _ => return Err("CardDAV nicht konfiguriert".into()),
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
                                Err(e) => return Err(format!("DB-Prepare fehlgeschlagen: {e}")),
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
                return Ok(added.len());
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
            Ok(contacts.len())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// CalDAV background sync (Phase 0 + multi-account). Mirrors the CardDAV
/// scheduler but drives the CalDAV client and persists calendars + events.
/// Delegates the actual sync to the shared `api::calendars::do_caldav_sync_account`
/// so manual and background syncs behave identically. One ticker covers ALL
/// enabled accounts, each on its own interval — and it keeps running when no
/// account is configured yet, so adding one takes effect without a restart.
pub async fn start_caldav_sync(state: Arc<AppState>, mut shutdown_rx: mpsc::Receiver<()>) {
    tracing::info!("CalDAV-Sync: Multi-Account-Ticker gestartet (Prüfung alle 60s)");
    let mut last_sync: std::collections::HashMap<String, tokio::time::Instant> =
        std::collections::HashMap::new();
    // Initial pass shortly after boot.
    tokio::time::sleep(Duration::from_secs(8)).await;

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                tracing::info!("CalDAV-Sync: gestoppt");
                break;
            }
            _ = tokio::time::sleep(Duration::from_secs(60)) => {}
        }
        let accounts: Vec<crate::dav::CalDavSettings> = state
            .caldav_accounts
            .read()
            .iter()
            .filter(|a| a.enabled && !a.url.is_empty())
            .cloned()
            .collect();
        for account in accounts {
            let interval = Duration::from_secs(account.sync_interval_minutes.max(1) * 60);
            let due = last_sync
                .get(&account.id)
                .map(|t| tokio::time::Instant::now().duration_since(*t) >= interval)
                .unwrap_or(true);
            if !due {
                continue;
            }
            last_sync.insert(account.id.clone(), tokio::time::Instant::now());
            match crate::api::calendars::do_caldav_sync_account(&state, &account).await {
                Ok(_) => {}
                Err(e) => tracing::warn!("CalDAV-Sync '{}' fehlgeschlagen: {}", account.name, e.0),
            }
        }
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
           source = 'carddav',
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
