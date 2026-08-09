//! EML / MBox export (Concept §9).
//!
//! Exports the local EML archive (never the provider) as either
//!   - `mbox`: one concatenated mbox file (each message split at "From "),
//!   - `zip`: one archive with individual .eml files per message.
//! Both are streamed from disk; no full-buffer in memory.

use std::io::{Read, Write};
use std::path::PathBuf;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;

use crate::AppState;
use crate::db::with_db;

#[derive(Deserialize)]
pub struct ExportQuery {
    pub account_id: i64,
    #[serde(default = "default_format")]
    pub format: String,
}

fn default_format() -> String {
    "mbox".to_string()
}

/// `GET /api/v1/export?account_id=N&format=mbox|zip`
pub async fn export_archive(
    State(state): State<AppState>,
    Query(q): Query<ExportQuery>,
) -> impl IntoResponse {
    let messages = with_db(&state, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT m.id, m.uid, m.raw_path, f.name
                 FROM messages m
                 JOIN folders f ON f.id = m.folder_id
                 WHERE m.account_id = ?1 AND m.raw_path IS NOT NULL AND m.raw_path != ''
                 ORDER BY m.date ASC, m.uid ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![q.account_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok::<_, String>(out)
    });

    let Ok(messages) = messages else {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            axum::body::Body::from(format!("{{\"error\": \"{}\"}}", messages.unwrap_err())),
        )
            .into_response();
    };
    if messages.is_empty() {
        return (
            axum::http::StatusCode::NOT_FOUND,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            axum::body::Body::from("{\"error\": \"Keine archivierten E-Mails gefunden\"}"),
        )
            .into_response();
    }

    match q.format.as_str() {
        "zip" => zip_export(&state, q.account_id, &messages),
        _ => mbox_export(&state, q.account_id, &messages),
    }
}

fn mbox_export(
    state: &AppState,
    account_id: i64,
    messages: &[(i64, String, String)],
) -> axum::response::Response {
    let filename = format!("relay-account-{account_id}.mbox");
    let data_root = state.data_root.clone();
    let messages = messages.to_vec();
    let stream = async_stream::stream! {
        for (_, rel, _folder) in &messages {
            let abs = data_root.join(rel);
            match std::fs::File::open(&abs) {
                Ok(mut f) => {
                    let mut buf = Vec::new();
                    if f.read_to_end(&mut buf).is_ok() {
                        // mbox: escape a leading "From " line into ">From ".
                        let mut raw = String::from_utf8_lossy(&buf).to_string();
                        if raw.starts_with("From ") {
                            raw.insert_str(0, ">");
                        }
                        yield Ok::<axum::body::Bytes, std::io::Error>(axum::body::Bytes::from(raw));
                        yield Ok::<axum::body::Bytes, std::io::Error>(axum::body::Bytes::from("\n\n"));
                    }
                }
                Err(_) => {
                    yield Ok::<axum::body::Bytes, std::io::Error>(axum::body::Bytes::from(format!(
                        "From relay-export Thu Jan  1 00:00:00 1970\nX-Relay-Missing: {}\n\n",
                        rel
                    )));
                }
            }
        }
    };
    let body = axum::body::Body::from_stream(stream);
    let headers = [
        (axum::http::header::CONTENT_TYPE, "message/rfc822".to_string()),
        (
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        ),
    ];
    (headers, body).into_response()
}

fn zip_export(
    state: &AppState,
    account_id: i64,
    messages: &[(i64, String, String)],
) -> axum::response::Response {
    let filename = format!("relay-account-{account_id}-eml.zip");
    // Build the zip in a background thread to a temp file, then stream it.
    let data_root = state.data_root.clone();
    let messages = messages.to_vec();
    let stream = async_stream::stream! {
        let tmp = std::env::temp_dir().join(format!("relay-export-{}.zip", std::process::id()));
        let data_root_owned = data_root.clone();
        let messages_owned = messages.clone();
        let result = tokio::task::spawn_blocking(move || {
            let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
            let mut zip = zip_writer(&file)?;
            for (mid, rel, folder) in &messages_owned {
                let abs = data_root_owned.join(rel);
                match std::fs::read(&abs) {
                    Ok(bytes) => {
                        let folder_safe = folder.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
                        let name = format!("{folder_safe}/{mid}.eml");
                        let _ = zip.add_file(&name, bytes);
                    }
                    Err(_) => {}
                }
            }
            zip.finish()?;
            Ok::<_, String>(tmp)
        }).await;

        let path = match result {
            Ok(Ok(p)) => p,
            _ => {
                yield Ok::<axum::body::Bytes, std::io::Error>(axum::body::Bytes::from("Export fehlgeschlagen"));
                return;
            }
        };
        if let Ok(mut f) = std::fs::File::open(&path) {
            let mut buf = [0u8; 64 * 1024];
            loop {
                match f.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => yield Ok::<axum::body::Bytes, std::io::Error>(axum::body::Bytes::from(buf[..n].to_vec())),
                    Err(_) => break,
                }
            }
        }
        let _ = std::fs::remove_file(&path);
    };
    let body = axum::body::Body::from_stream(stream);
    let headers = [
        (axum::http::header::CONTENT_TYPE, "application/zip".to_string()),
        (
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        ),
    ];
    (headers, body).into_response()
}

/// Minimal zip writer (stored entries — no compression, fast).
struct ZipWriter<W: Write> {
    out: W,
    count: u16,
    central: Vec<u8>,
    pos: u32,
}

fn zip_writer<W: Write>(out: W) -> Result<ZipWriter<W>, String> {
    Ok(ZipWriter { out, count: 0, central: Vec::new(), pos: 0 })
}

impl<W: Write> ZipWriter<W> {
    fn add_file(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), String> {
        let name_bytes = name.as_bytes();
        let crc = crc32(bytes.as_slice());
        let local_offset = self.pos;

        // Local file header
        let mut local = Vec::new();
        local.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]); // PK\x03\x04
        local.extend_from_slice(&[20, 0]); // version needed
        local.extend_from_slice(&[0, 0]); // flags
        local.extend_from_slice(&[0, 0]); // method: stored
        local.extend_from_slice(&[0, 0, 0, 0]); // time/date
        local.extend_from_slice(&crc.to_le_bytes());
        local.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        local.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        local.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        local.extend_from_slice(&[0, 0]); // extra len
        local.extend_from_slice(name_bytes);
        self.out.write_all(&local).map_err(|e| e.to_string())?;
        self.out.write_all(&bytes).map_err(|e| e.to_string())?;
        self.pos += local.len() as u32 + bytes.len() as u32;

        // Central directory entry
        let mut cen = Vec::new();
        cen.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]); // PK\x01\x02
        cen.extend_from_slice(&[20, 0]); // version made by
        cen.extend_from_slice(&[20, 0]); // version needed
        cen.extend_from_slice(&[0, 0]);
        cen.extend_from_slice(&[0, 0]);
        cen.extend_from_slice(&[0, 0, 0, 0]);
        cen.extend_from_slice(&crc.to_le_bytes());
        cen.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        cen.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        cen.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        cen.extend_from_slice(&[0, 0]);
        cen.extend_from_slice(&[0, 0]);
        cen.extend_from_slice(&[0, 0]);
        cen.extend_from_slice(&[0, 0]);
        cen.extend_from_slice(&[0, 0]);
        cen.extend_from_slice(&local_offset.to_le_bytes());
        cen.extend_from_slice(name_bytes);
        self.central.extend_from_slice(&cen);
        self.count += 1;
        Ok(())
    }

    fn finish(mut self) -> Result<(), String> {
        let central_offset = self.pos;
        self.out.write_all(&self.central).map_err(|e| e.to_string())?;
        let mut eocd = Vec::new();
        eocd.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
        eocd.extend_from_slice(&[0, 0]);
        eocd.extend_from_slice(&[0, 0]);
        eocd.extend_from_slice(&self.count.to_le_bytes());
        eocd.extend_from_slice(&self.count.to_le_bytes());
        eocd.extend_from_slice(&(self.central.len() as u32).to_le_bytes());
        eocd.extend_from_slice(&central_offset.to_le_bytes());
        eocd.extend_from_slice(&[0, 0]);
        self.out.write_all(&eocd).map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB88320 ^ (c >> 1) } else { c >> 1 };
        }
        *t = c;
    }
    let mut crc = 0xFFFFFFFFu32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFFFFFF
}

#[allow(dead_code)]
fn unused(_p: PathBuf) {}
