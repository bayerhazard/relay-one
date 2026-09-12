//! Insilo-Meeting-Scanner: pollt das geteilte appCommon-Drop-Verzeichnis,
//! ingested Meeting-Summaries in SQLite (`meetings` + FTS) und soft-deleted
//! Zeilen, deren Datei verschwunden ist.
//!
//! Bewusst simpel: flaches Verzeichnis, handgemachter Frontmatter-Parser
//! (keine YAML-Lib), SHA-256 als Änderungsmerkmal. Ein Scan heilt immer —
//! verpasste Events, Neustarts, Insilo-Ausfälle.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::db::with_db;
use crate::AppState;

/// Dateien jünger als das werden übersprungen (Backstop: Insilo schreibt
/// zwar atomar, aber ein Scan mitten im Rename sieht nie eine halbe Datei —
/// die Grenze schützt vor exotischen Dateisystemen ohne atomares Rename).
const FRESH_SECS: u64 = 15;

/// Ergebnis eines Scan-Durchlaufs (Logs + `POST /meetings/scan`).
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ScanReport {
    pub scanned: usize,
    pub inserted: usize,
    pub updated: usize,
    pub deleted: usize,
    pub skipped: usize,
}

/// Drop-Verzeichnis aus der Umgebung; `None` = deaktiviert.
fn drop_dir() -> Option<PathBuf> {
    if std::env::var("RELAY_INSILO_ENABLED").unwrap_or_default() == "false" {
        return None;
    }
    let dir = std::env::var("RELAY_INSILO_DIR").ok()?;
    if dir.is_empty() {
        return None;
    }
    Some(PathBuf::from(dir))
}

/// Eine geparste Meeting-Datei (Frontmatter + Body).
struct Parsed {
    insilo_id: String,
    title: String,
    participants: Vec<String>,
    tags: Vec<String>,
    meeting_date: String,
    duration_min: i64,
    language: String,
    template: String,
    source_url: String,
    body: String,
}

/// Führende/abschließende Anführungszeichen von einem Skalar abstreifen.
fn unquote(s: &str) -> &str {
    s.trim().trim_matches('"')
}

/// Handgemachter Frontmatter-Parser (Contract §3): erste Zeile `---`,
/// `key: value`-Zeilen bis schließendes `---`. Skalare = Rest nach erstem
/// `:` (Anführungszeichen abgestrifen); `participants`/`tags` sind
/// JSON-Array-Literale (Insilo schreibt `json.dumps`). Der Body folgt
/// unverändert nach dem schließenden `---`.
fn parse_frontmatter(content: &str) -> Result<Parsed, String> {
    let mut lines = content.lines();
    let first = lines.next().ok_or("leere Datei")?;
    if first.trim() != "---" {
        return Err("keine Frontmatter (erste Zeile != ---)".into());
    }
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut geschlossen = false;
    for line in lines {
        if line.trim() == "---" {
            geschlossen = true;
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            fields.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    if !geschlossen {
        return Err("Frontmatter nicht geschlossen".into());
    }

    let field = |key: &str| fields.get(key).map(|s| s.as_str()).unwrap_or("");
    let insilo_id = unquote(field("insilo_id")).to_string();
    if insilo_id.is_empty() {
        return Err("insilo_id fehlt".into());
    }
    let parse_list = |key: &str| -> Result<Vec<String>, String> {
        serde_json::from_str(field(key))
            .map_err(|e| format!("{key} ist kein gültiges JSON-Array: {e}"))
    };
    let body = content
        .find("\n---\n")
        .map(|i| content[i + 5..].trim_start_matches('\n').to_string())
        .unwrap_or_default();

    Ok(Parsed {
        insilo_id,
        title: unquote(field("title")).to_string(),
        participants: parse_list("participants")?,
        tags: parse_list("tags")?,
        meeting_date: unquote(field("recorded_at")).to_string(),
        duration_min: field("duration_min").parse().unwrap_or(0),
        language: unquote(field("language")).to_string(),
        template: unquote(field("template")).to_string(),
        source_url: unquote(field("source_url")).to_string(),
        body,
    })
}

const UPSERT_SQL: &str = "
    INSERT INTO meetings (
        insilo_id, path, sha256, title, participants, tags, meeting_date,
        duration_min, language, template, source_url, body_md,
        first_seen_at, updated_at
    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
              datetime('now'), datetime('now'))
    ON CONFLICT(insilo_id) DO UPDATE SET
        path = excluded.path,
        sha256 = excluded.sha256,
        title = excluded.title,
        participants = excluded.participants,
        tags = excluded.tags,
        meeting_date = excluded.meeting_date,
        duration_min = excluded.duration_min,
        language = excluded.language,
        template = excluded.template,
        source_url = excluded.source_url,
        body_md = excluded.body_md,
        deleted = 0,
        updated_at = datetime('now')";

/// Ein kompletter Scan-Durchlauf. Synchron (kleines flaches Verzeichnis),
/// wirft nicht: DB-/Dateifehler werden geloggt und als leerer Report
/// zurückgegeben.
pub fn run_insilo_scan(state: &AppState) -> ScanReport {
    let dir = match drop_dir() {
        Some(d) => d,
        None => return ScanReport::default(),
    };
    if !dir.exists() {
        // Insilo evtl. nicht installiert — kein Fehlerzustand.
        tracing::debug!(?dir, "Insilo-Drop-Verzeichnis fehlt — Scan übersprungen");
        return ScanReport::default();
    }

    // Phase 1: Verzeichnis lesen, Dateien parsen. `seen` enthält ALLE .md-
    // Dateien (auch ungeparste) — nur eine wirklich fehlende Datei ist ein
    // Löschsignal, kein Parse-Fehler.
    let mut gesehen: HashSet<String> = HashSet::new();
    let mut gefunden: Vec<(String, String, Parsed)> = Vec::new();
    let mut skipped = 0usize;
    let mut scanned = 0usize;

    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.ends_with(".md") || name.ends_with(".tmp") {
                    continue;
                }
                gesehen.insert(name.clone());
                let meta = match entry.metadata() {
                    Ok(m) if m.is_file() => m,
                    _ => {
                        skipped += 1;
                        continue;
                    }
                };
                let frisch = meta
                    .modified()
                    .ok()
                    .and_then(|t| SystemTime::now().duration_since(t).ok())
                    .is_some_and(|d| d < Duration::from_secs(FRESH_SECS));
                if frisch {
                    skipped += 1;
                    continue;
                }
                let content = match std::fs::read_to_string(entry.path()) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(file = %name, "Insilo-Datei nicht lesbar: {e}");
                        skipped += 1;
                        continue;
                    }
                };
                let sha = hex::encode(Sha256::digest(content.as_bytes()));
                match parse_frontmatter(&content) {
                    Ok(p) => {
                        scanned += 1;
                        gefunden.push((name, sha, p));
                    }
                    Err(e) => {
                        tracing::warn!(file = %name, "Insilo-Frontmatter nicht parstbar: {e}");
                        skipped += 1;
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!(?dir, "Insilo-Verzeichnis nicht lesbar: {e}");
            return ScanReport::default();
        }
    }

    // Phase 2: DB upserten + Lösch-Erkennung.
    let report = with_db(state, |conn| {
        let mut inserted = 0usize;
        let mut updated = 0usize;
        for (path, sha, p) in &gefunden {
            // Vom Nutzer gelöscht → bleibt weg, auch wenn die Export-Datei
            // noch existiert (der Upsert würde sonst `deleted = 0` setzen).
            let ignored: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM meetings_ignored WHERE insilo_id = ?1",
                    rusqlite::params![p.insilo_id],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if ignored > 0 {
                continue;
            }
            let existing: Option<(String, i64)> = conn
                .query_row(
                    "SELECT sha256, deleted FROM meetings WHERE insilo_id = ?1",
                    rusqlite::params![p.insilo_id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
                )
                .ok();
            match existing {
                // Unverändert → skip.
                Some((alt, 0)) if alt == *sha => {}
                _ => {
                    let neu = existing.is_none();
                    conn.execute(
                        UPSERT_SQL,
                        rusqlite::params![
                            p.insilo_id,
                            path,
                            sha,
                            p.title,
                            serde_json::to_string(&p.participants).unwrap_or_else(|_| "[]".into()),
                            serde_json::to_string(&p.tags).unwrap_or_else(|_| "[]".into()),
                            p.meeting_date,
                            p.duration_min,
                            p.language,
                            p.template,
                            p.source_url,
                            p.body,
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                    if neu {
                        inserted += 1;
                    } else {
                        updated += 1;
                    }
                }
            }
        }

        // Datei verschwunden → Soft-Delete (Eintrag bleibt für Historie).
        let mut stmt = conn
            .prepare("SELECT path FROM meetings WHERE deleted = 0")
            .map_err(|e| e.to_string())?;
        let pfade: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        let mut deleted = 0usize;
        for pfad in pfade {
            if !gesehen.contains(&pfad) {
                conn.execute(
                    "UPDATE meetings SET deleted = 1, updated_at = datetime('now') WHERE path = ?1",
                    rusqlite::params![pfad],
                )
                .map_err(|e| e.to_string())?;
                deleted += 1;
            }
        }

        Ok(ScanReport {
            scanned,
            inserted,
            updated,
            deleted,
            skipped,
        })
    })
    .unwrap_or_else(|e| {
        tracing::error!("Insilo-Scan: DB-Fehler: {e}");
        ScanReport::default()
    });

    if report.inserted + report.updated + report.deleted > 0 {
        state.events.emit(
            "meetings-changed",
            &serde_json::json!({ "count": report.inserted + report.updated + report.deleted }),
        );
    }
    report
}

/// Hintergrund-Takt: ein Scan direkt beim Start, dann alle
/// `RELAY_INSILO_SCAN_SECS` (Default 600). Entkoppelt vom Mail-Sync-Backoff.
pub async fn spawn_loop(state: std::sync::Arc<AppState>, mut shutdown_rx: mpsc::Receiver<()>) {
    let interval = Duration::from_secs(
        std::env::var("RELAY_INSILO_SCAN_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(600),
    );
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                tracing::info!("Insilo-Scan-Loop wurde gestoppt");
                break;
            }
            _ = ticker.tick() => {
                let report = run_insilo_scan(&state);
                if report.inserted + report.updated + report.deleted > 0 {
                    tracing::info!(
                        inserted = report.inserted,
                        updated = report.updated,
                        deleted = report.deleted,
                        "Insilo-Scan: Meetings geändert"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATEI: &str = r#"---
insilo_id: "b1e4f0aa-1234-4abc-9def-0123456789ab"
title: "Kundenmeeting Müller GmbH"
recorded_at: "2026-09-12T14:30:00+00:00"
duration_min: 47
language: "de"
participants: ["Anna Weber", "Dr. Marc Böhm"]
tags: ["Mandant Müller", "Vertrag"]
template: "Standard-Notiz"
source_url: "https://insilo.example.de/meetings/b1e4f0aa"
schema: 1
---

# Kundenmeeting Müller GmbH

## Kernthemen
- Nachtragsforderung

## Nächste Schritte
- [ ] Angebot erstellen (verantwortlich: Herr Muster, frist: 2026-09-20)
"#;

    #[test]
    fn frontmatter_wird_geparst() {
        let p = parse_frontmatter(DATEI).unwrap();
        assert_eq!(p.insilo_id, "b1e4f0aa-1234-4abc-9def-0123456789ab");
        assert_eq!(p.title, "Kundenmeeting Müller GmbH");
        assert_eq!(p.meeting_date, "2026-09-12T14:30:00+00:00");
        assert_eq!(p.duration_min, 47);
        assert_eq!(p.participants, vec!["Anna Weber", "Dr. Marc Böhm"]);
        assert_eq!(p.tags, vec!["Mandant Müller", "Vertrag"]);
        assert_eq!(p.template, "Standard-Notiz");
        assert!(p.source_url.ends_with("/b1e4f0aa"));
        // Der Body beginnt mit der kanonischen Überschrift.
        assert!(p.body.starts_with("# Kundenmeeting Müller GmbH"));
        // Die Frontmatter ist aus dem Body raus.
        assert!(!p.body.contains("insilo_id"));
    }

    #[test]
    fn horizontale_linie_im_body_bricht_den_parser_nicht() {
        let content = format!("{DATEI}\n---\n\nNachtrag unter der Linie.\n");
        let p = parse_frontmatter(&content).unwrap();
        assert!(p.body.contains("Nachtrag unter der Linie."));
    }

    #[test]
    fn fehlendes_insilo_id_wird_abgelehnt() {
        let content = "---\ntitle: \"X\"\n---\n\nBody\n";
        assert!(parse_frontmatter(content).is_err());
    }

    #[test]
    fn kaputtes_participants_array_wird_abgelehnt() {
        let content = "---\ninsilo_id: \"a\"\nparticipants: [Anna\n---\n\nBody\n";
        assert!(parse_frontmatter(content).is_err());
    }

    #[test]
    fn ohne_frontmatter_wird_abgelehnt() {
        assert!(parse_frontmatter("# Nur Markdown\n").is_err());
    }

    /// Nutzer-Delete übersteht Re-Scans: solange die Export-Datei existiert,
    /// darf der Scan den Eintrag nicht wiederherstellen.
    #[test]
    fn user_deleted_meeting_bleibt_weg_beim_rescan() {
        use crate::AppState;

        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("meeting.md");
        std::fs::write(&file, DATEI).unwrap();
        // Datei "alt" machen (Freshness-Check: < 15 s = noch in Write).
        let past = filetime::FileTime::from_unix_time(0, 0);
        filetime::set_file_times(&file, past, past).unwrap();

        std::env::set_var("RELAY_INSILO_DIR", tmp.path());
        std::env::remove_var("RELAY_INSILO_ENABLED");

        let mut state = AppState::new();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::cache::db::init_db(&conn).unwrap();
        *state.cache_db.lock() = Some(conn);

        // 1. Scan: Meeting wird importiert.
        let r1 = run_insilo_scan(&state);
        assert_eq!(r1.inserted, 1, "erster Scan importiert das Meeting");

        // 2. Nutzer-Delete: Soft-Delete + Tombstone.
        with_db(&state, |conn| {
            conn.execute(
                "UPDATE meetings SET deleted = 1 WHERE insilo_id = 'b1e4f0aa-1234-4abc-9def-0123456789ab'",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO meetings_ignored (insilo_id, deleted_at)
                 VALUES ('b1e4f0aa-1234-4abc-9def-0123456789ab', datetime('now'))",
                [],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();

        // 3. Re-Scan: Datei existiert noch — Eintrag bleibt gelöscht.
        let r2 = run_insilo_scan(&state);
        assert_eq!(r2.inserted, 0);
        assert_eq!(r2.updated, 0);
        let deleted: i64 = with_db(&state, |conn| {
            let d: i64 = conn
                .query_row(
                    "SELECT deleted FROM meetings WHERE insilo_id = 'b1e4f0aa-1234-4abc-9def-0123456789ab'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            Ok(d)
        })
        .unwrap();
        assert_eq!(deleted, 1, "Tombstone verhindert die Wiederherstellung");

        std::env::remove_var("RELAY_INSILO_DIR");
    }
}
