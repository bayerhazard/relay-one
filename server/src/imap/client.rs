use crate::error::AppError;
use crate::imap::types::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use imap::types::Flag;
use imap::Connection;

const IMAP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[cfg(test)]
pub struct ImapTestOverrideInner {
    pub folders: Vec<FolderEntry>,
    pub appended: std::sync::Mutex<Vec<(String, Vec<u8>, Option<Vec<String>>)>>,
    pub is_connected: bool,
    pub fail_connect: bool,
    pub fail_append: bool,
    pub fail_delete: bool,
    pub fail_list_folders: bool,
    /// Incremented on every real `connect_inner()` invocation (i.e. every
    /// physical connection attempt). Used to verify single-flight connect.
    pub connect_count: std::sync::Mutex<u32>,
}

#[cfg(test)]
pub type ImapTestOverride = std::sync::Arc<ImapTestOverrideInner>;

pub struct ImapClient {
    host: String,
    port: u16,
    username: String,
    password: String,
    use_ssl: bool,
    insecure: bool,
    session: Arc<Mutex<Option<imap::Session<Connection>>>>,
    connected: Arc<Mutex<bool>>,
    operation_timeout: Duration,
    /// Serializes connect/reconnect so concurrent callers (sync scheduler
    /// flag_refresh / removal_check / health check + move API) never open
    /// multiple IMAP connections to the same account simultaneously.
    connect_lock: Arc<tokio::sync::Mutex<()>>,
    /// Serializes `with_session_blocking` operations per client so concurrent
    /// IMAP ops queue instead of racing (see connect_limit fix).
    op_lock: Arc<tokio::sync::Mutex<()>>,
    /// Tracks the last SELECTed folder to skip redundant SELECT round-trips.
    current_folder: Arc<std::sync::Mutex<Option<String>>>,
    #[cfg(test)]
    pub test_override: Option<ImapTestOverride>,
}

impl ImapClient {
    pub fn new(
        host: String,
        port: u16,
        username: String,
        password: String,
        use_ssl: bool,
    ) -> Self {
        Self::new_with_options(host, port, username, password, use_ssl, false)
    }

    pub fn new_with_options(
        host: String,
        port: u16,
        username: String,
        password: String,
        use_ssl: bool,
        insecure: bool,
    ) -> Self {
        Self {
            host,
            port,
            username,
            password,
            use_ssl,
            insecure,
            session: Arc::new(Mutex::new(None)),
            connected: Arc::new(Mutex::new(false)),
            operation_timeout: Duration::from_secs(60),
            connect_lock: Arc::new(tokio::sync::Mutex::new(())),
            op_lock: Arc::new(tokio::sync::Mutex::new(())),
            current_folder: Arc::new(std::sync::Mutex::new(None)),
            #[cfg(test)]
            test_override: None,
        }
    }

    pub async fn connect(&self) -> Result<(), AppError> {
        // Single-flight: only one physical connection per client at a time.
        // Without this, concurrent callers (sync scheduler + move API) all see
        // is_connected()==false and each open a new IMAP connection, blowing
        // through the provider's per-user connection limit.
        let _guard = self.connect_lock.lock().await;
        // Double-checked: another task may have connected while we waited.
        if *self.connected.lock().await {
            return Ok(());
        }
        self.connect_inner().await
    }

    /// Actually opens the TCP+TLS connection and logs in. Only called while
    /// holding `connect_lock` (see `connect`).
    async fn connect_inner(&self) -> Result<(), AppError> {
        #[cfg(test)]
        if let Some(ref o) = self.test_override {
            if o.fail_connect {
                return Err(AppError::imap("simulierter Verbindungsfehler", "connect"));
            }
            *o.connect_count.lock().unwrap() += 1;
            *self.connected.lock().await = true;
            return Ok(());
        }
        let host = self.host.clone();
        let port = self.port;
        let username = self.username.clone();
        let password = self.password.clone();
        let use_ssl = self.use_ssl;
        let insecure = self.insecure;

        let join_handle = tokio::task::spawn_blocking(move || -> Result<_, AppError> {
            tracing::info!("IMAP: Verbinde zu {}:{} (SSL: {}, insecure: {}) ...", host, port, use_ssl, insecure);
            let tcp = std::net::TcpStream::connect((host.as_str(), port))
                .map_err(|e| AppError::imap(format!("IMAP TCP connect fehlgeschlagen: {}", e), "tcp_connect"))?;
            tcp.set_read_timeout(Some(Duration::from_secs(30)))
                .map_err(|e| AppError::imap(format!("IMAP set_read_timeout fehlgeschlagen: {}", e), "set_read_timeout"))?;
            let stream: Connection = if use_ssl {
                let connector = build_rustls_connector(insecure)?;
                let tls = connector.connect(&host, tcp)
                    .map_err(|e| AppError::imap(format!("IMAP TLS Handshake fehlgeschlagen: {}", e), "tls_handshake"))?;
                Box::new(tls)
            } else {
                Box::new(tcp)
            };
            let mut client = imap::Client::new(stream);
            client.read_greeting()
                .map_err(|e| AppError::imap(format!("IMAP Greeting fehlgeschlagen: {}", e), "greeting"))?;

            tracing::info!("IMAP: Login als {} ...", username);
            let session = client
                .login(&username, &password)
                .map_err(|(e, _)| AppError::auth(format!("IMAP login fehlgeschlagen: {}", e), "login"))?;

            tracing::info!("IMAP: Verbindung erfolgreich");
            Ok(session)
        });

        let session: imap::Session<Connection> = tokio::time::timeout(IMAP_TIMEOUT, async {
            join_handle.await
                .map_err(|e| AppError::imap(format!("Spawn fehlgeschlagen: {}", e), "spawn"))?
        })
        .await
        .map_err(|_| AppError::imap("IMAP-Timeout nach 30s", "timeout"))??;

        *self.session.lock().await = Some(session);
        *self.connected.lock().await = true;
        Ok(())
    }

    pub async fn select_inbox(&self) -> Result<(), AppError> {
        self.with_session_blocking("select_inbox", |session| {
            session
                .select("INBOX")
                .map_err(|e| AppError::imap(e.to_string(), "select_inbox"))?;
            Ok(())
        })
        .await
    }

    pub async fn list_folders(&self) -> Result<Vec<(String, String, String, String)>, AppError> {        self.with_session_blocking("list_folders", |session| {
            let folders = session
                .list(None, Some("*"))
                .map_err(|e| AppError::imap(format!("IMAP list fehlgeschlagen: {}", e), "list_folders"))?;

            tracing::info!("IMAP: {} Ordner gefunden", folders.len());

            Ok(folders
                .iter()
                .map(|f| {
                    let raw = f.name().to_string();
                    let name = decode_imap_utf7(&raw);
                    let delim = f.delimiter().map(|d| d.to_string()).unwrap_or_default();
                    let attrs = f.attributes();
                    let has_no_select = attrs.iter().any(|a| {
                        format!("{:?}", a).contains("NoSelect")
                    });
                    // RFC 3501 §6.3.8: the \Trash attribute marks the
                    // provider's trash folder. We map it onto our local
                    // "Trash" so multiple provider trash folders (Gelöscht /
                    // Papierkorb / Deleted Messages…) do not show up as
                    // separate duplicates in the UI.
                    let is_trash = attrs.iter().any(|a| {
                        format!("{:?}", a).contains("Trash")
                    });
                    let tag = if has_no_select {
                        "noselect"
                    } else if is_trash {
                        "trash"
                    } else {
                        "folder"
                    };
                    (name, raw, delim, tag.to_string())
                })
                .collect())
        })
        .await
    }

    pub async fn select_folder(&self, folder: &str) -> Result<(), AppError> {
        let folder = encode_imap_utf7(folder);
        self.with_session_blocking("select_folder", move |session| {
            session
                .select(&folder)
                .map_err(|e| AppError::imap(e.to_string(), "select_folder"))?;
            Ok(())
        })
        .await
    }

    pub async fn fetch_recent(
        &self,
        since_uid: u32,
        limit: u32,
    ) -> Result<Vec<CachedMessage>, AppError> {
        self.fetch_recent_impl(None, since_uid, limit).await
    }

    /// SELECT `folder` + fetch recent messages atomically under the session
    /// lock. Without the SELECT, a parallel API operation (fetch_body, move,
    /// delete, flag) can switch the session to a different folder between our
    /// select_folder and the UID SEARCH — the fetch would then read the wrong
    /// mailbox (or the subsequent cleanup would prune the wrong folder).
    pub async fn fetch_recent_in_folder(
        &self,
        folder: &str,
        since_uid: u32,
        limit: u32,
    ) -> Result<Vec<CachedMessage>, AppError> {
        self.fetch_recent_impl(Some(folder), since_uid, limit).await
    }

    async fn fetch_recent_impl(
        &self,
        folder: Option<&str>,
        since_uid: u32,
        limit: u32,
    ) -> Result<Vec<CachedMessage>, AppError> {
        let folder = folder.map(|f| encode_imap_utf7(f));
        self.with_session_blocking("fetch_recent", move |session| {
        if let Some(f) = &folder {
            session
                .select(f)
                .map_err(|e| AppError::imap(format!("SELECT '{}' fehlgeschlagen: {}", f, e), "select_folder"))?;
        }
        // Server-side UID filtering: only fetch UIDs newer than since_uid.
        // "UID {N}:*" means UIDs from N to the highest UID on the server.
        // Initial sync (since_uid == 0) uses "ALL" to discover all messages.
        let uid_set = if since_uid > 0 {
            session
                .uid_search(&format!("UID {}:*", since_uid + 1))
                .map_err(|e| AppError::imap(format!("IMAP uid_search fehlgeschlagen: {}", e), "uid_search"))?
        } else {
            session
                .uid_search("ALL")
                .map_err(|e| AppError::imap(format!("IMAP uid_search fehlgeschlagen: {}", e), "uid_search"))?
        };

        tracing::info!("IMAP: {} neue Nachrichten gefunden", uid_set.len());

        if uid_set.is_empty() {
            return Ok(Vec::new());
        }

        let mut uids: Vec<u32> = uid_set.into_iter().collect();
        uids.sort_unstable();

        // Process in ascending UID order in both cases. For the initial sync
        // (since_uid == 0) this means the OLDEST messages are fetched first;
        // the sync scheduler writes back `last_uid` after each batch, so the
        // NEXT cycle continues with `UID {last+1}:*` — the folder is filled
        // completely over successive cycles (no messages are ever skipped,
        // unlike the old `.rev().take(limit)` which only ever grabbed the
        // newest `limit` messages).
        let selected: Vec<&u32> = uids.iter().take(limit as usize).collect();

        let uid_list = selected.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",");

        // BODYSTRUCTURE lets us detect attachments without downloading the
        // message body — the server returns the MIME tree only.
        let messages = session
            .uid_fetch(&uid_list, "(UID FLAGS INTERNALDATE ENVELOPE BODYSTRUCTURE)")
            .map_err(|e| AppError::imap(format!("IMAP uid_fetch fehlgeschlagen: {}", e), "uid_fetch"))?;

        tracing::info!("IMAP: {} Nachrichten abgerufen", messages.len());

        let mut result = Vec::new();
        for msg in messages.iter().take(limit as usize) {
            let uid = msg.uid.unwrap_or(0);
            let is_read = msg
                .flags()
                .iter()
                .any(|f| f.to_string() == "\\Seen");

            let flags: Vec<String> = msg.flags().iter().map(|f| f.to_string()).collect();

            let subject = msg
                .envelope()
                .and_then(|e| e.subject.clone())
                .and_then(|s| String::from_utf8(s.to_vec()).ok())
                .map(|s: String| decode_rfc2047(s.as_str()))
                .unwrap_or_default();

            let from = msg.envelope().map(|e| {
                e.from.as_deref().unwrap_or(&[]).iter().map(|addr| {
                    let name = std::str::from_utf8(addr.name.as_deref().unwrap_or(b"")).unwrap_or("");
                    let name = decode_rfc2047(name);
                    let mb = std::str::from_utf8(addr.mailbox.as_deref().unwrap_or(b"")).unwrap_or("");
                    let host = std::str::from_utf8(addr.host.as_deref().unwrap_or(b"")).unwrap_or("");
                    if !name.is_empty() {
                        format!("{} <{}@{}>", name, mb, host)
                    } else {
                        format!("{}@{}", mb, host)
                    }
                }).collect::<Vec<_>>().join(", ")
            }).unwrap_or_default();

            let to = msg.envelope().map(|e| {
                e.to.as_deref().unwrap_or(&[]).iter().map(|addr| {
                    let name = std::str::from_utf8(addr.name.as_deref().unwrap_or(b"")).unwrap_or("");
                    let mb = std::str::from_utf8(addr.mailbox.as_deref().unwrap_or(b"")).unwrap_or("");
                    let host = std::str::from_utf8(addr.host.as_deref().unwrap_or(b"")).unwrap_or("");
                    if !name.is_empty() {
                        format!("{} <{}@{}>", name, mb, host)
                    } else {
                        format!("{}@{}", mb, host)
                    }
                }).collect::<Vec<_>>().join(", ")
            }).unwrap_or_default();

            let cc = msg.envelope().map(|e| {
                e.cc.as_deref().unwrap_or(&[]).iter().map(|addr| {
                    let name = std::str::from_utf8(addr.name.as_deref().unwrap_or(b"")).unwrap_or("");
                    let name = decode_rfc2047(name);
                    let mb = std::str::from_utf8(addr.mailbox.as_deref().unwrap_or(b"")).unwrap_or("");
                    let host = std::str::from_utf8(addr.host.as_deref().unwrap_or(b"")).unwrap_or("");
                    if !name.is_empty() {
                        format!("{} <{}@{}>", name, mb, host)
                    } else {
                        format!("{}@{}", mb, host)
                    }
                }).collect::<Vec<_>>().join(", ")
            }).unwrap_or_default();

            let date = msg
                .internal_date()
                .map(|d| d.to_rfc3339())
                .unwrap_or_default();

            let message_id = msg
                .envelope()
                .and_then(|e| e.message_id.clone())
                .and_then(|s| String::from_utf8(s.to_vec()).ok())
                .unwrap_or_default();

      let has_attachments = msg
                .bodystructure()
                .map(bodystructure_has_attachment)
                .unwrap_or(false);

            let attachments = msg
                .bodystructure()
                .map(|bs| extract_attachment_metadata(bs))
                .unwrap_or_default();

           let is_flagged = flags.iter().any(|f| f == "\\Flagged");

            result.push(CachedMessage {
                uid,
                envelope: MailEnvelope {
                    subject,
                    from,
                    to,
                    cc,
                    date,
                    message_id,
                },
                flags,
                body_preview: None,
                body_structure: None,
                ai_summary: None,
                ai_priority: None,
                ai_fraud_score: None,
                cached_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                is_read,
                is_flagged,
                has_attachments,
                attachments,
            });
        }
        Ok(result)
        })
        .await
    }

    /// Fetch all UIDs in the currently selected folder via UID SEARCH ALL.
    /// Used by the sync scheduler to detect messages deleted on the server.
    pub async fn fetch_all_uids(&self) -> Result<Vec<u32>, AppError> {
        self.with_session_blocking("fetch_all_uids", move |session| {
            let uid_set = session
                .uid_search("ALL")
                .map_err(|e| AppError::imap(format!("IMAP uid_search ALL fehlgeschlagen: {}", e), "fetch_all_uids"))?;

            let mut uids: Vec<u32> = uid_set.into_iter().collect();
            uids.sort_unstable();
            Ok(uids)
        })
        .await
    }

    /// SELECT a folder and fetch ALL its UIDs atomically (single session
    /// lock). Avoids the race where another thread selects a different
    /// folder between our SELECT and UID SEARCH.
    pub async fn fetch_all_uids_in_folder(&self, folder: &str) -> Result<Vec<u32>, AppError> {
        let folder = encode_imap_utf7(folder);
        self.with_session_blocking("fetch_all_uids_in_folder", move |session| {
            session
                .select(&folder)
                .map_err(|e| AppError::imap(format!("SELECT '{}' fehlgeschlagen: {}", folder, e), "fetch_all_uids_in_folder"))?;
            let uid_set = session
                .uid_search("ALL")
                .map_err(|e| AppError::imap(format!("IMAP uid_search ALL fehlgeschlagen: {}", e), "fetch_all_uids_in_folder"))?;
            let mut uids: Vec<u32> = uid_set.into_iter().collect();
            uids.sort_unstable();
            Ok(uids)
        })
        .await
    }

    /// Fetch and fully parse a message body, returning clean
    /// `(plain_text, Option<html>)`.
    ///
    /// We fetch the complete raw RFC822 message and parse it with `mail-parser`,
    /// which correctly walks the MIME tree, applies Content-Transfer-Encoding
    /// (quoted-printable / base64) and decodes the declared charset
    /// (ISO-8859-1, Windows-1252, UTF-8, …). This replaces the previous
    /// heuristic `BODY[TEXT] BODY[2]` fetch + guess-based decoding, which left
    /// raw MIME headers, boundaries and `=XX` control sequences in the output.
  pub async fn fetch_body(&self, uid: u32) -> Result<(String, Option<String>), AppError> {
        let (text, html, _raw) = self.fetch_body_with_raw(uid).await?;
        Ok((text, html))
    }

    /// Fetch message body + the raw RFC822 bytes. The raw bytes are what gets
    /// persisted to the EML archive (`archive/<acct>/YYYY/MM/<uid>-<sha>.eml`).
    pub async fn fetch_body_with_raw(&self, uid: u32) -> Result<(String, Option<String>, Vec<u8>), AppError> {
        self.fetch_body_with_raw_from_folder(uid, None).await
    }

    /// Same as `fetch_body_with_raw`, scoped to an explicit folder.
    pub async fn fetch_body_with_raw_from_folder(
        &self,
        uid: u32,
        folder: Option<String>,
    ) -> Result<(String, Option<String>, Vec<u8>), AppError> {
        let folder = folder.map(|f| encode_imap_utf7(&f));
        self.with_session_blocking("fetch_body_with_raw", move |session| {
            if let Some(f) = folder {
                session
                    .select(&f)
                    .map_err(|e| AppError::imap(format!("SELECT '{}' fehlgeschlagen: {}", f, e), "select_folder"))?;
            }

            let msgs = session
                .uid_fetch(uid.to_string(), "(BODY.PEEK[])")
                .map_err(|e| AppError::imap(e.to_string(), "fetch_body_with_raw"))?;

            let msg = msgs
                .iter()
                .next()
                .ok_or(AppError::not_found("Nachricht nicht gefunden", "fetch_body_with_raw"))?;
            let raw = msg.body().or_else(|| msg.text()).unwrap_or(b"").to_vec();

            let (text, html) = parse_message_bodies(&raw);
            Ok((text, html, raw))
        })
        .await
    }

    /// Fetch message body from a specific folder. If folder is None, uses the current folder.
    pub async fn fetch_body_from_folder(
        &self,
        uid: u32,
        folder: Option<String>,
    ) -> Result<(String, Option<String>), AppError> {
        // Check if we need to SELECT (skip if already in the target folder).
        let needs_select = {
            let cur = self.current_folder.lock().unwrap();
            cur.as_deref() != folder.as_deref()
        };
        let select_folder = if needs_select { folder.clone() } else { None };

        let result = self.with_session_blocking("fetch_body", move |session| {
            if let Some(ref f) = select_folder {
                session
                    .select(f)
                    .map_err(|e| AppError::imap(format!("SELECT '{}' fehlgeschlagen: {}", f, e), "select_folder"))?;
            }

            // BODY.PEEK[TEXT] only fetches text/* parts (skips attachments, images).
            let msgs = session
                .uid_fetch(uid.to_string(), "(BODY.PEEK[TEXT])")
                .map_err(|e| AppError::imap(e.to_string(), "fetch_body"))?;

            let msg = msgs
                .iter()
                .next()
                .ok_or(AppError::not_found("Nachricht nicht gefunden", "fetch_body"))?;
            let raw = msg.body().or_else(|| msg.text()).unwrap_or(b"");

            Ok(parse_message_bodies(raw))
        })
        .await;

        // Update the tracked folder on success.
        if result.is_ok() {
            if let Some(f) = &folder {
                *self.current_folder.lock().unwrap() = Some(f.clone());
            }
        }
        result
    }

    /// Fetch the complete raw RFC822 message (headers + all MIME parts),
    /// needed to extract attachments. Returns the bytes as a lossy UTF-8 string
    /// (binary parts remain base64/quoted-printable encoded inside the MIME).
    pub async fn fetch_raw_message(&self, uid: u32) -> Result<String, AppError> {
        self.fetch_raw_message_in_folder(uid, None).await
    }

    /// Fetch raw message with optional folder selection.
    pub async fn fetch_raw_message_in_folder(&self, uid: u32, folder: Option<String>) -> Result<String, AppError> {
        self.with_session_blocking("fetch_raw_message", move |session| {
            // Select the target folder if specified
            if let Some(ref f) = folder {
                session
                    .select(f)
                    .map_err(|e| AppError::imap(format!("SELECT '{}' fehlgeschlagen: {}", f, e), "select_folder"))?;
            }

            let msgs = session
                .uid_fetch(uid.to_string(), "(BODY.PEEK[])")
                .map_err(|e| AppError::imap(e.to_string(), "fetch_raw_message"))?;
            let msg = msgs
                .iter()
                .next()
                .ok_or(AppError::not_found("Nachricht nicht gefunden", "fetch_raw_message"))?;
            let raw = msg.body().or_else(|| msg.text()).unwrap_or(b"");
            Ok(String::from_utf8_lossy(raw).into_owned())
        })
        .await
    }

    pub async fn mark_seen(&self, uid: u32) -> Result<(), AppError> {
        self.mark_flag(uid, "\\Seen", true, None).await
    }

    /// Remove the \Seen flag from a message (mark as unread).
  pub async fn mark_unseen(&self, uid: u32) -> Result<(), AppError> {
        self.mark_flag(uid, "\\Seen", false, None).await
    }

    /// STORE a flag on a message. IMAP requires a selected mailbox before
    /// UID STORE — if `folder` is given it is selected first.
    pub async fn mark_flag(
        &self,
        uid: u32,
        flag: &str,
        set: bool,
        folder: Option<String>,
    ) -> Result<(), AppError> {
        let flag = flag.to_string();
        let folder = folder.map(|f| encode_imap_utf7(&f));
        self.with_session_blocking("mark_flag", move |session| {
            if let Some(ref f) = folder {
                session
                    .select(f)
                    .map_err(|e| AppError::imap(format!("SELECT '{}' fehlgeschlagen: {}", f, e), "mark_flag"))?;
            }
            let op = if set {
                format!("+FLAGS ({})", flag)
            } else {
                format!("-FLAGS ({})", flag)
            };
            session
                .uid_store(uid.to_string(), &op)
                .map_err(|e| AppError::imap(e.to_string(), "mark_flag"))?;
            Ok(())
        })
        .await
    }

    /// Toggle the \Flagged flag on a message (mark/unmark).
    pub async fn toggle_flagged(&self, uid: u32, flagged: bool) -> Result<(), AppError> {
        self.mark_flag(uid, "\\Flagged", flagged, None).await
    }

    /// Fetch FLAGS for a set of UIDs. Returns Vec<(uid, is_read, is_flagged)>.
    pub async fn fetch_flags(&self, uid_set: &str) -> Result<Vec<(u32, bool, bool)>, AppError> {
        let uid_set = uid_set.to_string();
        self.with_session_blocking("fetch_flags", move |session| {
            let messages = session
                .uid_fetch(&uid_set, "(FLAGS)")
                .map_err(|e| AppError::imap(format!("IMAP uid_fetch FLAGS fehlgeschlagen: {}", e), "fetch_flags"))?;

            let mut result = Vec::new();
            for msg in messages.iter() {
                let uid = msg.uid.unwrap_or(0);
                let flags = msg.flags();
                let is_read = flags.iter().any(|f| f.to_string() == "\\Seen");
                let is_flagged = flags.iter().any(|f| f.to_string() == "\\Flagged");
                result.push((uid, is_read, is_flagged));
            }
            Ok(result)
        })
        .await
    }

    pub async fn delete_message(&self, uid: u32, folder: &str) -> Result<(), AppError> {
        let folder = encode_imap_utf7(folder);
        #[cfg(test)]
        if let Some(ref o) = self.test_override {
            if o.fail_delete {
                return Err(AppError::imap("simulierter Loeschfehler", "delete_message"));
            }
            return Ok(());
        }
        let folder = folder.to_string();
        self.with_session_blocking("delete_message", move |session| {
            session
                .select(&folder)
                .map_err(|e| AppError::imap(format!("SELECT '{}' fehlgeschlagen: {}", folder, e), "select_folder"))?;
            session
                .uid_store(uid.to_string(), "+FLAGS (\\Deleted)")
                .map_err(|e| AppError::imap(format!("STORE DELETED fehlgeschlagen: {}", e), "store_deleted"))?;
            session
                .expunge()
                .map_err(|e| AppError::imap(format!("EXPUNGE fehlgeschlagen: {}", e), "expunge"))?;
            Ok(())
        })
        .await
    }

    /// Hard-delete a message on the provider (STORE \Deleted + EXPUNGE).
    /// Only to be called after the local archive guarantee holds (F1).
    pub async fn hard_delete_message(&self, uid: u32, folder: &str) -> Result<(), AppError> {
        self.delete_message(uid, folder).await
    }

    /// IMAP IDLE: wait up to `timeout` for a mailbox change in `folder`.
    /// Returns true if the mailbox changed (new mail), false on timeout.
    /// Falls back to false on any error (the caller then polls normally).
    /// The read timeout is restored afterwards so the session stays reusable.
    pub async fn idle_wait(&self, folder: &str, timeout: std::time::Duration) -> bool {
        let folder = encode_imap_utf7(folder);
        let result = self.with_session_blocking("idle_wait", move |session| {
            session
                .select(&folder)
                .map_err(|e| AppError::imap(format!("IDLE select '{}': {}", folder, e), "select_folder"))?;
            let mut handle = session.idle();
            handle.timeout(timeout);
            let outcome = handle
                .wait_while(imap::extensions::idle::stop_on_any)
                .map_err(|e| AppError::imap(format!("IDLE wait: {}", e), "idle_wait"))?;
            Ok(matches!(outcome, imap::extensions::idle::WaitOutcome::MailboxChanged))
        })
        .await;
        result.unwrap_or(false)
    }

    pub async fn move_message(&self, uid: u32, source: &str, target: &str) -> Result<(), AppError> {
        let source = encode_imap_utf7(source);
        let target = encode_imap_utf7(target);
        self.with_session_blocking("move_message", move |session| {
            session
                .select(&source)
                .map_err(|e| {
                    AppError::imap(
                        format!("SELECT source '{}' fehlgeschlagen: {}", source, e),
                        "select_folder",
                    )
                })?;

            // Try UID COPY first
            let copy_result = session.uid_copy(uid.to_string(), &target);
            if let Err(e) = copy_result {
                // Some IMAP servers (Cyrus, some Dovecot configs) reject UID COPY.
                // Fallback: fetch the full message and APPEND it to target.
                tracing::warn!(
                    "UID COPY nach '{}' fehlgeschlagen für uid={}, versuche APPEND-Fallback: {}",
                    target, uid, e
                );

                let msgs = session
               .uid_fetch(uid.to_string(), "(BODY.PEEK[])")
                    .map_err(|fetch_err| {
                        AppError::imap(
                            format!(
                                "FETCH für APPEND-Fallback fehlgeschlagen (COPY-Fehler: {}): {}",
                                e, fetch_err
                            ),
                            "uid_fetch_fallback",
                        )
                    })?;

                let msg = msgs.iter().next().ok_or_else(|| {
                    AppError::not_found(
                        format!(
                            "Nachricht uid={} nicht gefunden für APPEND-Fallback (COPY-Fehler: {})",
                            uid, e
                        ),
                        "move_message_fallback",
                    )
                })?;

                let raw_bytes = msg.body().ok_or_else(|| {
                    AppError::imap(
                        format!(
                            "Kein BODY.PEEK[] in FETCH-Antwort für APPEND-Fallback (COPY-Fehler: {})",
                            e
                        ),
                        "move_message_fallback",
                    )
                })?;

                session
                    .append(&target, raw_bytes)
                    .finish()
                    .map_err(|append_err| {
                        AppError::imap(
                            format!(
                                "APPEND nach '{}' fehlgeschlagen (COPY-Fehler: {}, APPEND-Fehler: {})",
                                target, e, append_err
                            ),
                            "append_fallback",
                        )
                    })?;
            }

            session
                .uid_store(uid.to_string(), "+FLAGS (\\Deleted)")
                .map_err(|e| {
                    AppError::imap(
                        format!("STORE DELETED in '{}' fehlgeschlagen: {}", source, e),
                        "store_deleted",
                    )
                })?;
            session
                .expunge()
                .map_err(|e| {
                    AppError::imap(
                        format!("EXPUNGE in '{}' fehlgeschlagen: {}", source, e),
                        "expunge",
                    )
                })?;
            Ok(())
        })
        .await
    }

    /// Append a raw RFC822 message to an IMAP folder with optional flags.
    /// Returns the UID assigned by the server (if available).
    pub async fn append_message(
        &self,
        folder: &str,
        raw_bytes: &[u8],
        flags: Option<&[&str]>,
    ) -> Result<(), AppError> {
        #[cfg(test)]
        if let Some(ref o) = self.test_override {
            if o.fail_append {
                return Err(AppError::imap("simulierter APPEND-Fehler", "append_message"));
            }
            let flag_strings = flags.map(|f| f.iter().map(|s| s.to_string()).collect());
            o.appended.lock().unwrap().push((folder.to_string(), raw_bytes.to_vec(), flag_strings));
            return Ok(());
        }
        // Own the inputs so they can move into the blocking closure.
        let folder = folder.to_string();
        let raw_bytes = raw_bytes.to_vec();
        let flags: Option<Vec<String>> = flags.map(|f| f.iter().map(|s| s.to_string()).collect());
        self.with_session_blocking("append_message", move |session| {
            if let Some(flag_list) = flags {
                let imap_flags: Vec<Flag<'_>> = flag_list.iter().map(|s| {
                    match s.as_str() {
                        "\\Seen" => Flag::Seen.into_owned(),
                        "\\Answered" => Flag::Answered.into_owned(),
                        "\\Flagged" => Flag::Flagged.into_owned(),
                        "\\Deleted" => Flag::Deleted.into_owned(),
                        "\\Draft" => Flag::Draft.into_owned(),
                        "\\Recent" => Flag::Recent.into_owned(),
                        _ => Flag::from(s.to_string()),
                    }
                }).collect();
                session
                    .append(&folder, &raw_bytes)
                    .flags(imap_flags)
                    .finish()
            } else {
                session
                    .append(&folder, &raw_bytes)
                    .finish()
            }
            .map_err(|e| AppError::imap(format!("APPEND nach '{}' fehlgeschlagen: {}", folder, e), "append"))?;
            Ok(())
        })
        .await
    }

    /// List folders with their SPECIAL-USE attributes.
    /// Returns FolderEntry structs with parsed attributes.
    pub async fn list_folders_detailed(&self) -> Result<Vec<FolderEntry>, AppError> {
        #[cfg(test)]
        if let Some(ref o) = self.test_override {
            if o.fail_list_folders {
                return Err(AppError::imap("simulierter Listenfehler", "list_folders_detailed"));
            }
            return Ok(o.folders.clone());
        }
        self.with_session_blocking("list_folders_detailed", |session| {
            let folders = session
                .list(None, Some("*"))
                .map_err(|e| AppError::imap(format!("IMAP list fehlgeschlagen: {}", e), "list_folders_detailed"))?;

            Ok(folders
                .iter()
                .map(|f| {
                    let raw = f.name().to_string();
                    let name = decode_imap_utf7(&raw);
                    let delim = f.delimiter().map(|d| d.to_string()).unwrap_or_default();
                    let attributes: Vec<String> = f.attributes().iter()
                        .map(|a| format!("{:?}", a))
                        .collect();
                    let has_no_select = attributes.iter().any(|a| a.contains("NoSelect"));
                    let tag = if has_no_select { "noselect" } else { "folder" };
                    FolderEntry { name, raw_name: raw, delimiter: delim, tag: tag.to_string(), attributes }
                })
                .collect())
        })
        .await
    }

    pub async fn create_folder(&self, name: &str) -> Result<(), AppError> {
        let name = encode_imap_utf7(name);
        self.with_session_blocking("create_folder", move |session| {
            session
                .create(&name)
                .map_err(|e| AppError::imap(format!("CREATE folder fehlgeschlagen: {}", e), "create_folder"))?;
            Ok(())
        })
        .await
    }

    pub async fn rename_folder(&self, old_name: &str, new_name: &str) -> Result<(), AppError> {
        let old_name = encode_imap_utf7(old_name);
        let new_name = encode_imap_utf7(new_name);
        self.with_session_blocking("rename_folder", move |session| {
            session
                .rename(&old_name, &new_name)
                .map_err(|e| AppError::imap(format!("RENAME fehlgeschlagen: {}", e), "rename_folder"))?;
            Ok(())
        })
        .await
    }

    /// Delete a folder on the provider (IMAP DELETE). Note: the provider may
    /// refuse if the mailbox contains messages or has inferior children.
    pub async fn delete_folder(&self, name: &str) -> Result<(), AppError> {
        let name = encode_imap_utf7(name);
        self.with_session_blocking("delete_folder", move |session| {
            session
                .delete(&name)
                .map_err(|e| AppError::imap(format!("DELETE folder fehlgeschlagen: {}", e), "delete_folder"))?;
            Ok(())
        })
        .await
    }

    pub async fn shutdown(&self) {
        let session_opt = self.session.lock().await.take();
        *self.connected.lock().await = false;
        if let Some(session) = session_opt {
            logout_session(session).await;
        }
    }

    pub async fn is_connected(&self) -> bool {
        #[cfg(test)]
        if let Some(ref o) = self.test_override {
            return o.is_connected;
        }
        *self.connected.lock().await
    }

    /// Check if the IMAP connection is still alive by sending a NOOP command.
    /// Returns `true` if the server responds within the timeout (5s).
    /// On failure (timeout, IO error, or not connected), marks the connection
    /// as disconnected and returns `false`.
    pub async fn ping(&self) -> bool {
        let mut guard = self.session.lock().await;
        let Some(mut session) = guard.take() else {
            return false;
        };
        drop(guard);

        let result = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::task::spawn_blocking(move || {
                let ok = session.noop().is_ok();
                (session, ok)
            })
            .await
        })
        .await;

        match result {
            Ok(Ok((session, true))) => {
                *self.session.lock().await = Some(session);
                true
            }
            Ok(Ok((_session, false))) => {
                *self.connected.lock().await = false;
                false
            }
            Ok(Err(_)) => {
                *self.connected.lock().await = false;
                false
            }
            Err(_) => {
                *self.connected.lock().await = false;
                false
            }
        }
    }

    pub async fn reconnect(&self) -> Result<(), AppError> {
        // Serialize with connect(): only one physical connection per client.
        let _guard = self.connect_lock.lock().await;
        // Best-effort logout of the old session instead of dropping it — a
        // dropped session leaves the TCP connection open at the provider until
        // its timeout, accumulating connections until the per-user limit hits.
        let old = self.session.lock().await.take();
        *self.connected.lock().await = false;
        *self.current_folder.lock().unwrap() = None;
        if let Some(session) = old {
            logout_session(session).await;
        }
        self.connect_inner().await
    }

    /// Runs a blocking IMAP session operation on a dedicated blocking thread,
    /// guarded by `operation_timeout`.
    ///
    /// The `imap` crate is synchronous: calling its methods directly inside an
    /// `async` block would block a Tokio worker thread for the entire network
    /// round-trip and make `tokio::time::timeout` ineffective (it cannot
    /// interrupt a blocking syscall). This helper takes the session out of the
    /// mutex, hands it to `spawn_blocking`, and restores it afterwards — the
    /// same pattern `connect()`/`ping()` already use.
    ///
    /// On TagMismatch (known imap crate bug) the session is dropped and the
    /// client marked disconnected. The next operation will auto-reconnect
    /// before proceeding, resetting the tag counter.
    /// On other errors or timeout the session is dropped and the client marked
    /// disconnected, so a half-broken session is never reused.
    async fn with_session_blocking<T, F>(
        &self,
        op_name: &'static str,
        f: F,
    ) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&mut imap::Session<Connection>) -> Result<T, AppError> + Send + 'static,
    {
        // Serialize IMAP operations per client so concurrent callers (sync
        // scheduler + move API) queue instead of interleaving on one session
        // and racing auto-reconnects (see connect_limit fix).
        let _op_guard = self.op_lock.lock().await;
        let mut guard = self.session.lock().await;
        let mut session = match guard.take() {
            Some(s) => {
                drop(guard);
                s
            }
            None => {
                drop(guard);
                tracing::info!("IMAP Session fehlt, versuche Auto-Reconnect fuer '{}'", op_name);
                self.connect().await?;
                self.session.lock().await
                    .take()
                    .ok_or_else(|| AppError::imap("Auto-Reconnect fehlgeschlagen", op_name))?
            }
        };

        let timeout = self.operation_timeout;
        let join = tokio::time::timeout(timeout, async move {
            tokio::task::spawn_blocking(move || {
                let result = f(&mut session);
                (session, result)
            })
            .await
        })
        .await;

        match join {
            Ok(Ok((session, result))) => {
                match result {
                    Ok(value) => {
                        *self.session.lock().await = Some(session);
                        Ok(value)
                    }
                    Err(e) => {
                        if is_tag_mismatch(&e) {
                            tracing::warn!(
                                "IMAP TagMismatch bei '{}' — Session zurückgesetzt, nächster Aufruf reconnectet automatisch",
                                op_name
                            );
                            // Logout instead of dropping so the connection is
                            // closed at the provider (prevents connection leaks).
                            logout_session(session).await;
                            *self.connected.lock().await = false;
                        } else {
                            *self.session.lock().await = Some(session);
                        }
                        Err(e)
                    }
                }
            }
            Ok(Err(join_err)) => {
                *self.connected.lock().await = false;
                // The blocking closure returned a JoinError — the session was
                // handed into it and is now unreachable. Log it out if it is
                // still alive so the provider doesn't hold the connection open
                // until its own timeout (per-user connection limit).
                *self.current_folder.lock().unwrap() = None;
                if let Some(s) = self.session.lock().await.take() {
                    logout_session(s).await;
                }
                Err(AppError::imap(
                    format!("IMAP-Operation '{}' abgebrochen: {}", op_name, join_err),
                    op_name,
                ))
            }
            Err(_) => {
                *self.connected.lock().await = false;
                tracing::warn!(
                    "IMAP-Operation '{}' Zeitüberschreitung nach {}s — Session verworfen",
                    op_name,
                    timeout.as_secs()
                );
                // Timeout: the session variable was moved into the timed-out
                // async block and dropped there; the provider keeps the TCP
                // connection until its timeout. Nothing we can recover here,
                // but make sure no stale session object survives in the slot.
                *self.current_folder.lock().unwrap() = None;
                if let Some(s) = self.session.lock().await.take() {
                    logout_session(s).await;
                }
                Err(AppError::imap(
                    format!("IMAP-Operation '{}' Zeitüberschreitung", op_name),
                    "timeout",
                ))
            }
        }
    }
}

/// Check if an AppError wraps an imap::Error::TagMismatch.
fn is_tag_mismatch(e: &AppError) -> bool {
    matches!(e, AppError::Imap { msg, .. } if msg.contains("TagMismatch"))
}

/// Best-effort logout of an IMAP session. logout() is blocking, so it runs on
/// a blocking thread with a short timeout — callers never hang on an
/// unresponsive server, and the connection is cleanly closed at the provider
/// instead of being leaked until the server-side timeout.
async fn logout_session(mut session: imap::Session<Connection>) {
    let _ = tokio::time::timeout(Duration::from_secs(5), async move {
        tokio::task::spawn_blocking(move || match session.logout() {
            Ok(()) => tracing::info!("IMAP: Logout erfolgreich"),
            Err(e) => tracing::warn!("IMAP: Logout fehlgeschlagen: {}", e),
        })
        .await
    })
    .await;
}

fn build_rustls_connector(insecure: bool) -> Result<rustls_connector::RustlsConnector, AppError> {
    use rustls_connector::rustls;
    use rustls_connector::rustls_native_certs;
    let mut store = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().unwrap_or_default() {
        let _ = store.add(cert);
    }
    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(store)
        .with_no_client_auth();
    if insecure {
        tracing::warn!("IMAP: Zertifikatsprüfung übersprungen");
        config.dangerous().set_certificate_verifier(std::sync::Arc::new(NoCertVerification));
    }
    Ok(rustls_connector::RustlsConnector::from(config))
}

#[derive(Debug)]
struct NoCertVerification;

impl rustls_connector::rustls::client::danger::ServerCertVerifier for NoCertVerification {
    fn supported_verify_schemes(&self) -> Vec<rustls_connector::rustls::SignatureScheme> {
        vec![
            rustls_connector::rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls_connector::rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls_connector::rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls_connector::rustls::SignatureScheme::RSA_PSS_SHA256,
        ]
    }

    fn verify_server_cert(
        &self,
        _: &rustls_connector::rustls::pki_types::CertificateDer<'_>,
        _: &[rustls_connector::rustls::pki_types::CertificateDer<'_>],
        _: &rustls_connector::rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls_connector::rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls_connector::rustls::client::danger::ServerCertVerified, rustls_connector::rustls::Error> {
        Ok(rustls_connector::rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_connector::rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls_connector::rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls_connector::rustls::client::danger::HandshakeSignatureValid, rustls_connector::rustls::Error> {
        Ok(rustls_connector::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_connector::rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls_connector::rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls_connector::rustls::client::danger::HandshakeSignatureValid, rustls_connector::rustls::Error> {
        Ok(rustls_connector::rustls::client::danger::HandshakeSignatureValid::assertion())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn test_decode_rfc2047_plain_text() {
        assert_eq!(decode_rfc2047("Hello World"), "Hello World");
    }

    #[test]
    fn test_decode_rfc2047_q_encoding() {
        let input = "=?UTF-8?Q?H=C3=A4llo?=";
        assert_eq!(decode_rfc2047(input), "Hällo");
    }

    #[test]
    fn test_decode_rfc2047_b_encoding() {
        let input = "=?UTF-8?B?R3LDvMOfZ290dA==?=";
        assert_eq!(decode_rfc2047(input), "Grüßgott");
    }

    #[test]
    fn test_decode_rfc2047_mixed() {
        let input = "Re: =?UTF-8?Q?R=C3=BCckmeldung?= heute";
        assert_eq!(decode_rfc2047(input), "Re: Rückmeldung heute");
    }

    #[test]
    fn test_decode_rfc2047_truncated_start() {
        assert_eq!(decode_rfc2047("=?="), "=?=");
        assert_eq!(decode_rfc2047("=?"), "=?");
    }

    #[test]
    fn test_decode_rfc2047_truncated_mid() {
        assert_eq!(decode_rfc2047("=?UTF-8?Q?"), "=?UTF-8?Q?");
        assert_eq!(decode_rfc2047("=?UTF-8?Q?abc"), "=?UTF-8?Q?abc");
    }

    #[test]
    fn test_decode_imap_utf7_plain() {
        assert_eq!(decode_imap_utf7("INBOX"), "INBOX");
        assert_eq!(decode_imap_utf7("Sent"), "Sent");
    }

    #[test]
    fn test_decode_imap_utf7_encoded() {
        let input = "Gel&APw-scht";
        let result = decode_imap_utf7(input);
        assert!(!result.is_empty());
        assert_ne!(result, "Gel&APw-scht");
    }

    #[test]
    fn test_decode_imap_utf7_ampersand_literal() {
        assert_eq!(decode_imap_utf7("&-Test"), "&Test");
    }

    #[test]
    fn test_decode_imap_utf7_empty() {
        assert_eq!(decode_imap_utf7(""), "");
    }

    #[test]
    fn test_decode_imap_utf7_no_ampersand() {
        assert_eq!(decode_imap_utf7("Plain Folder Name"), "Plain Folder Name");
    }

    // --- decode_transfer_encoding tests ---

    #[test]
    fn test_decode_transfer_encoding_empty() {
        assert_eq!(decode_transfer_encoding(""), "");
    }

    #[test]
    fn test_decode_transfer_encoding_plain() {
        assert_eq!(decode_transfer_encoding("Hello World"), "Hello World");
    }

    #[test]
    fn test_decode_transfer_encoding_base64() {
        let input = base64::engine::general_purpose::STANDARD.encode("Hello from Base64");
        assert_eq!(decode_transfer_encoding(&input), "Hello from Base64");
    }

    #[test]
    fn test_decode_transfer_encoding_qp() {
        assert_eq!(decode_transfer_encoding("H=C3=A4llo"), "Hällo");
    }

    #[test]
    fn test_decode_transfer_encoding_not_base64() {
        // String that looks like base64-ish but too short or odd chars
        let input = "!!!not-base64!!!";
        assert_eq!(decode_transfer_encoding(input), input);
    }

    // --- parse_message_bodies (mail-parser) tests ---

    #[test]
    fn test_parse_bodies_plain_quoted_printable() {
        let raw = b"From: a@b.com\r\nSubject: Test\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nH=C3=A4llo Welt=21";
        let (text, html) = parse_message_bodies(raw);
        assert_eq!(text, "Hällo Welt!");
        assert!(html.is_none());
    }

    #[test]
    fn test_parse_bodies_charset_iso8859() {
        // "Grüße" in ISO-8859-1 (ü=0xFC, ß=0xDF)
        let raw = b"Content-Type: text/plain; charset=iso-8859-1\r\n\r\nGr\xFC\xDFe";
        let (text, _) = parse_message_bodies(raw);
        assert_eq!(text, "Grüße");
    }

    #[test]
    fn test_parse_bodies_multipart_prefers_html_and_text() {
        let raw = b"Content-Type: multipart/alternative; boundary=\"BB\"\r\n\r\n\
--BB\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nPlain version\r\n\
--BB\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>HTML version</p>\r\n\
--BB--\r\n";
        let (text, html) = parse_message_bodies(raw);
        assert_eq!(text.trim(), "Plain version");
        assert_eq!(html.as_deref().map(str::trim), Some("<p>HTML version</p>"));
    }

    #[test]
    fn test_parse_bodies_base64() {
        // "Café" UTF-8 base64
        let raw = b"Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\nQ2Fmw6k=";
        let (text, _) = parse_message_bodies(raw);
        assert_eq!(text, "Café");
    }

    #[test]
    fn test_parse_bodies_html_only_derives_text() {
        let raw = b"Content-Type: text/html; charset=utf-8\r\n\r\n<html><body><p>Hallo</p><p>Welt</p></body></html>";
        let (text, html) = parse_message_bodies(raw);
        // Real HTML part is present and preserved.
        assert!(html.is_some());
        assert!(html.as_deref().unwrap().contains("Hallo"));
        // A readable plain-text rendition is derived (no raw tags).
        assert!(text.contains("Hallo"));
        assert!(text.contains("Welt"));
        assert!(!text.contains("<p>"));
    }

    #[test]
    fn test_parse_bodies_plain_has_no_html() {
        // A plain-text-only mail must NOT produce synthesized HTML, so the UI
        // renders it as text.
        let raw = b"Content-Type: text/plain; charset=utf-8\r\n\r\nNur Text, kein HTML.";
        let (text, html) = parse_message_bodies(raw);
        assert_eq!(text, "Nur Text, kein HTML.");
        assert!(html.is_none());
    }

    #[test]
    fn test_parse_bodies_html_part_with_plain_content_stays_none() {
        // A sender can declare a text/html part whose content carries no
        // markup at all (bare text). Storing that in body_html would make the
        // UI render raw text through the HTML branch and collapse the line
        // breaks — the contains_html_markup guard must drop it to None.
        let raw = b"Content-Type: text/html; charset=utf-8\r\n\r\nHallo Herr Bayer,\r\n\r\nwir haben Ihre Maschine per UPS erhalten.\r\n\r\nMit freundlichen Gruessen";
        let (text, html) = parse_message_bodies(raw);
        assert!(html.is_none());
        assert!(text.contains("Hallo Herr Bayer"));
    }

    #[test]
    fn test_contains_html_markup() {
        assert!(contains_html_markup("<p>Text</p>"));
        assert!(contains_html_markup("<div>Text</div>"));
        assert!(contains_html_markup("<table><tr><td>X</td></tr></table>"));
        assert!(contains_html_markup("Hallo <b>fett</b>"));
        assert!(!contains_html_markup("Guten Morgen,\n\nkein HTML hier."));
        assert!(!contains_html_markup(""));
    }

    // --- has_qp_pattern tests ---

    #[test]
    fn test_has_qp_pattern_found() {
        assert!(has_qp_pattern("=C3=A4"));
        assert!(has_qp_pattern("hello=20world"));
        assert!(has_qp_pattern("==20"));
    }

    #[test]
    fn test_has_qp_pattern_not_found() {
        assert!(!has_qp_pattern("hello world"));
        assert!(!has_qp_pattern(""));
    }

    #[test]
    fn test_has_qp_pattern_too_short() {
        assert!(!has_qp_pattern("="));
        assert!(!has_qp_pattern("=C"));
    }

    #[test]
    fn test_has_qp_pattern_no_hex_after_equals() {
        assert!(!has_qp_pattern("=XX"));
        assert!(!has_qp_pattern("=G0"));
    }

    // --- hex_to_byte tests ---

    #[test]
    fn test_hex_to_byte_valid() {
        assert_eq!(hex_to_byte(b'A', b'F'), Some(0xAF));
        assert_eq!(hex_to_byte(b'a', b'f'), Some(0xAF));
        assert_eq!(hex_to_byte(b'0', b'0'), Some(0x00));
        assert_eq!(hex_to_byte(b'F', b'F'), Some(0xFF));
        assert_eq!(hex_to_byte(b'9', b'9'), Some(0x99));
    }

    #[test]
    fn test_hex_to_byte_invalid() {
        assert_eq!(hex_to_byte(b'G', b'0'), None);
        assert_eq!(hex_to_byte(b'0', b'G'), None);
        assert_eq!(hex_to_byte(b'/', b'0'), None);
        assert_eq!(hex_to_byte(b' ', b'0'), None);
        assert_eq!(hex_to_byte(b'\0', b'0'), None);
    }

    // --- decode_charset tests ---

    #[test]
    fn test_decode_charset_utf8() {
        let bytes = vec![0x48, 0xC3, 0xA4, 0x6C, 0x6C, 0x6F]; // "Hällo" in UTF-8
        assert_eq!(decode_charset(&bytes, "UTF-8"), "Hällo");
    }

    #[test]
    fn test_decode_charset_iso_8859_1() {
        // ISO-8859-1: H=0x48, ä=0xE4, l=0x6C, l=0x6C, o=0x6F
        let bytes = vec![0x48, 0xE4, 0x6C, 0x6C, 0x6F];
        assert_eq!(decode_charset(&bytes, "ISO-8859-1"), "Hällo");
    }

    #[test]
    fn test_decode_charset_latin1() {
        let bytes = vec![0xC4, 0xD6, 0xDC]; // ÄÖÜ in Latin1
        assert_eq!(decode_charset(&bytes, "LATIN1"), "ÄÖÜ");
    }

    #[test]
    fn test_decode_charset_fallback() {
        // Unknown charset falls back to lossy UTF-8
        let bytes = vec![0x48, 0x65, 0x6C, 0x6C, 0x6F]; // "Hello"
        assert_eq!(decode_charset(&bytes, "unknown-charset"), "Hello");
    }

    // --- decode_quoted_printable tests ---

    #[test]
    fn test_decode_quoted_printable_hex() {
        assert_eq!(decode_quoted_printable("H=C3=A4llo"), "Hällo");
    }

    #[test]
    fn test_decode_quoted_printable_soft_break() {
        assert_eq!(decode_quoted_printable("soft=\r\nbreak"), "softbreak");
        assert_eq!(decode_quoted_printable("soft=\nbreak"), "softbreak");
    }

    #[test]
    fn test_decode_quoted_printable_no_equals() {
        assert_eq!(decode_quoted_printable("plain text"), "plain text");
    }

    // --- ImapClient state & error path tests ---

    #[tokio::test]
    async fn test_is_connected_initial_false() {
        let client = ImapClient::new("localhost".into(), 993, "u".into(), "p".into(), true);
        assert!(!client.is_connected().await);
    }

    #[tokio::test]
    async fn test_mark_seen_not_connected() {
        let client = ImapClient::new("localhost".into(), 993, "u".into(), "p".into(), true);
        let result = client.mark_seen(1).await;
        assert!(matches!(result, Err(AppError::Imap { .. })));
    }

    #[tokio::test]
    async fn test_delete_message_not_connected() {
        let client = ImapClient::new(
            "localhost".into(),
            143,
            "user".into(),
            "pass".into(),
            false,
        );
        let result = client.delete_message(42, "INBOX").await;
        assert!(matches!(result, Err(AppError::Imap { .. })));
    }

    #[tokio::test]
    async fn test_move_message_not_connected() {
        let client = ImapClient::new(
            "localhost".into(),
            143,
            "user".into(),
            "pass".into(),
            false,
        );
        let result = client.move_message(1, "INBOX", "Archive").await;
        assert!(matches!(result, Err(AppError::Imap { .. })));
    }

    #[tokio::test]
    async fn test_ping_not_connected() {
        let client = ImapClient::new("localhost".into(), 993, "u".into(), "p".into(), true);
        assert!(!client.ping().await);
    }

    #[tokio::test]
    async fn test_ping_after_reconnect_failure_stays_disconnected() {
        let client = ImapClient::new(
            "localhost".into(),
            143,
            "user".into(),
            "pass".into(),
            false,
        );
        // reconnect will fail (no real IMAP server)
        let _ = client.reconnect().await;
        // After failed reconnect, ping should also return false
        assert!(!client.ping().await);
        // Connected flag must remain false
        assert!(!client.is_connected().await);
    }

    #[tokio::test]
    async fn test_reconnect_resets_state_on_failure() {
        let client = ImapClient::new(
            "localhost".into(),
            143,
            "user".into(),
            "pass".into(),
            false,
        );
        // Initial state: not connected
        assert!(!client.is_connected().await);
        // reconnect will fail (no real IMAP server), but should reset state first
        let result = client.reconnect().await;
        assert!(result.is_err());
        // After failed reconnect, connected flag must remain false
        assert!(!client.is_connected().await);
    }

    /// Builds a client whose `connect_inner()` is short-circuited by the test
    /// override (counts real connection attempts via `connect_count`).
    fn client_with_override() -> (ImapClient, ImapTestOverride) {
        let override_ = ImapTestOverrideInner {
            folders: Vec::new(),
            appended: std::sync::Mutex::new(Vec::new()),
            is_connected: true,
            fail_connect: false,
            fail_append: false,
            fail_delete: false,
            fail_list_folders: false,
            connect_count: std::sync::Mutex::new(0),
        };
        let ov = ImapTestOverride::new(override_);
        let mut client = ImapClient::new("localhost".into(), 143, "u".into(), "p".into(), false);
        client.test_override = Some(ov.clone());
        (client, ov)
    }

    #[tokio::test]
    async fn test_connect_single_flight_only_one_connection() {
        let (client, ov) = client_with_override();
        // 8 concurrent connect() calls must result in exactly ONE physical
        // connection attempt (single-flight lock + double-check).
        let mut handles = Vec::new();
        for _ in 0..8 {
            let c = ImapClient {
                host: client.host.clone(),
                port: client.port,
                username: client.username.clone(),
                password: client.password.clone(),
                use_ssl: client.use_ssl,
                insecure: client.insecure,
                session: client.session.clone(),
                connected: client.connected.clone(),
                operation_timeout: client.operation_timeout,
                connect_lock: client.connect_lock.clone(),
                op_lock: client.op_lock.clone(),
                current_folder: client.current_folder.clone(),
                test_override: client.test_override.clone(),
            };
            handles.push(tokio::spawn(async move { c.connect().await }));
        }
        for h in handles {
            h.await.unwrap().expect("connect sollte ok sein");
        }
        assert_eq!(*ov.connect_count.lock().unwrap(), 1);
        assert!(client.is_connected().await);
    }

    #[tokio::test]
    async fn test_connect_after_success_is_noop() {
        let (client, ov) = client_with_override();
        client.connect().await.expect("connect ok");
        client.connect().await.expect("zweiter connect ok (noop)");
        client.connect().await.expect("dritter connect ok (noop)");
        assert_eq!(*ov.connect_count.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn test_select_folder_not_connected() {
        let client = ImapClient::new(
            "localhost".into(),
            143,
            "user".into(),
            "pass".into(),
            false,
        );
        let result = client.select_folder("INBOX").await;
        assert!(matches!(result, Err(AppError::Imap { .. })));
    }

    #[tokio::test]
    async fn test_create_folder_not_connected() {
        let client = ImapClient::new(
            "localhost".into(),
            143,
            "user".into(),
            "pass".into(),
            false,
        );
        let result = client.create_folder("Trash").await;
        assert!(matches!(result, Err(AppError::Imap { .. })));
    }

    #[tokio::test]
    async fn test_fetch_recent_not_connected() {
        let client = ImapClient::new(
            "localhost".into(),
            143,
            "user".into(),
            "pass".into(),
            false,
        );
        let result = client.fetch_recent(0, 10).await;
        assert!(matches!(result, Err(AppError::Imap { .. })));
    }

    // --- find_special_folder tests ---

    #[test]
    fn test_find_special_folder_by_attribute() {
        let folders = vec![
            FolderEntry {
                name: "INBOX".into(),
                raw_name: "INBOX".into(),
                delimiter: "/".into(),
                tag: "folder".into(),
                attributes: vec![],
            },
            FolderEntry {
                name: "Gesendet".into(),
                raw_name: "Gesendet".into(),
                delimiter: "/".into(),
                tag: "folder".into(),
                attributes: vec!["\\Sent".into()],
            },
        ];
        assert_eq!(
            find_special_folder(&folders, SpecialFolder::Sent),
            Some("Gesendet".into())
        );
    }

    #[test]
    fn test_find_special_folder_fallback() {
        let folders = vec![
            FolderEntry {
                name: "INBOX".into(),
                raw_name: "INBOX".into(),
                delimiter: "/".into(),
                tag: "folder".into(),
                attributes: vec![],
            },
            FolderEntry {
                name: "Sent".into(),
                raw_name: "Sent".into(),
                delimiter: "/".into(),
                tag: "folder".into(),
                attributes: vec![],
            },
        ];
        assert_eq!(
            find_special_folder(&folders, SpecialFolder::Sent),
            Some("Sent".into())
        );
    }

    #[test]
    fn test_find_special_folder_not_found() {
        let folders = vec![
            FolderEntry {
                name: "INBOX".into(),
                raw_name: "INBOX".into(),
                delimiter: "/".into(),
                tag: "folder".into(),
                attributes: vec![],
            },
            FolderEntry {
                name: "Work".into(),
                raw_name: "Work".into(),
                delimiter: "/".into(),
                tag: "folder".into(),
                attributes: vec![],
            },
        ];
        assert_eq!(find_special_folder(&folders, SpecialFolder::Drafts), None);
    }

    #[test]
    fn test_find_special_folder_prefers_attribute_over_fallback() {
        let folders = vec![
            FolderEntry {
                name: "Gesendet".into(),
                raw_name: "Gesendet".into(),
                delimiter: "/".into(),
                tag: "folder".into(),
                attributes: vec!["\\Sent".into()],
            },
            FolderEntry {
                name: "Sent".into(),
                raw_name: "Sent".into(),
                delimiter: "/".into(),
                tag: "folder".into(),
                attributes: vec![],
            },
        ];
        assert_eq!(
            find_special_folder(&folders, SpecialFolder::Sent),
            Some("Gesendet".into())
        );
    }

    #[test]
    fn test_find_special_folder_case_insensitive_fallback() {
        let folders = vec![
            FolderEntry {
                name: "DRAFTS".into(),
                raw_name: "DRAFTS".into(),
                delimiter: "/".into(),
                tag: "folder".into(),
                attributes: vec![],
            },
        ];
        assert_eq!(
            find_special_folder(&folders, SpecialFolder::Drafts),
            Some("DRAFTS".into())
        );
    }

    /// Integration test against a real GreenMail IMAP server.
    ///
    /// Requires Docker and the following command:
    /// ```bash
    /// docker run -d --rm \
    ///   -p 3143:3143 -p 3025:3025 \
    ///   -e GREENMAIL_OPTS="-Dgreenmail.setup.test.all -Dgreenmail.hostname=0.0.0.0 -Dgreenmail.users=test@localhost:test" \
    ///   greenmail/standalone:latest
    /// ```
    ///
    /// Then run with:
    /// ```bash
    /// cargo test greenmail_append_draft -- --ignored
    /// ```
    #[ignore]
    #[tokio::test]
    async fn test_greenmail_append_draft() {
        let host = std::env::var("GREENMAIL_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let port: u16 = std::env::var("GREENMAIL_IMAP_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(3143);
        let user = std::env::var("GREENMAIL_USER").unwrap_or_else(|_| "test@localhost".into());
        let pass = std::env::var("GREENMAIL_PASS").unwrap_or_else(|_| "test".into());

        let client = ImapClient::new(host.clone(), port, user.clone(), pass, false);

        client.connect().await.expect("Verbindung zu GreenMail fehlgeschlagen");

        assert!(client.is_connected().await);

        let raw_message = b"From: test@localhost\r\n\
            To: recipient@example.com\r\n\
            Subject: Test Draft\r\n\
            Message-ID: <greenmail-test-draft@localhost>\r\n\
            \r\n\
            Dies ist ein Test-Entwurf.";

        client
            .append_message("Drafts", raw_message, Some(&["\\Draft"]))
            .await
            .expect("APPEND an GreenMail fehlgeschlagen");

        let appended = client.test_override.as_ref().map(|o| o.appended.lock().unwrap().len());
        if let Some(count) = appended {
            eprintln!("[test] appended via override (cfg(test) in non-test build path — unexpected)");
        }

        // Verify the message was stored by searching the Drafts folder.
        // The imap crate uses synchronous I/O; run select on tokio's blocking thread.
        let session = std::sync::Arc::clone(&client.session);
        let result = tokio::task::spawn_blocking(move || {
            let mut guard = session.blocking_lock();
            let sess = guard
                .as_mut()
                .expect("Session nach connect nicht verfügbar");
            sess.select("Drafts")
                .expect("SELECT Drafts fehlgeschlagen")
        })
        .await
        .expect("spawn_blocking panicked");

        assert!(
            result.exists > 0,
            "GreenMail sollte mindestens eine Nachricht in Drafts haben (exists={})",
            result.exists,
        );
    }
}

pub fn decode_rfc2047(input: &str) -> String {
    let mut out = String::new();
    let mut rest = input;
    while let Some(start) = rest.find("=?") {
        if start + 2 > rest.len() { break; }
        let before = &rest[..start];
        out.push_str(before);
        rest = &rest[start + 2..];
        let Some(mid) = rest.find("?") else {
            out.push_str(&format!("=?{}", rest));
            rest = "";
            break;
        };
        let charset = &rest[..mid];
        rest = &rest[mid + 1..];
        let Some(q) = rest.find("?") else {
            out.push_str(&format!("=?{}?", charset));
            rest = "";
            break;
        };
        let encoding = &rest[..q];
        rest = &rest[q + 1..];
        if let Some(end) = rest.find("?=") {
            let encoded = &rest[..end];
            rest = &rest[end + 2..];
            let decoded = match encoding.to_uppercase().as_str() {
                "Q" => decode_q_encoding(encoded, charset),
                "B" => decode_b_encoding(encoded, charset),
                _ => format!("=?{}?{}?{}?=", charset, encoding, encoded),
            };
            out.push_str(&decoded);
        } else {
            out.push_str(&format!("=?{}?{}?{}", charset, encoding, rest));
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    out
}

fn decode_q_encoding(input: &str, charset: &str) -> String {
    let mut bytes = Vec::new();
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c == '=' {
            let hex: String = chars.by_ref().take(2).collect();
            if hex.len() == 2 {
                if let Ok(b) = u8::from_str_radix(&hex, 16) {
                    bytes.push(b);
                } else {
                    bytes.push(b'=');
                    bytes.extend_from_slice(hex.as_bytes());
                }
            } else {
                bytes.push(b'=');
                bytes.extend_from_slice(hex.as_bytes());
            }
        } else if c == '_' {
            bytes.push(b' ');
        } else {
            let mut buf = [0; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }
    decode_charset(&bytes, charset)
}

fn decode_b_encoding(input: &str, charset: &str) -> String {
    use base64::Engine;
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(input) {
        decode_charset(&bytes, charset)
    } else {
        input.to_string()
    }
}

fn decode_charset(bytes: &[u8], charset: &str) -> String {
    let charset_upper = charset.to_uppercase();
    if charset_upper == "UTF-8" || charset_upper.starts_with("UTF-8") {
        String::from_utf8(bytes.to_vec()).unwrap_or_else(|_| String::from_utf8_lossy(bytes).to_string())
    } else if charset_upper == "ISO-8859-1" || charset_upper == "ISO8859-1" || charset_upper == "LATIN1" {
        bytes.iter().map(|&b| b as char).collect()
    } else {
        String::from_utf8_lossy(bytes).to_string()
    }
}

fn decode_imap_utf7(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let mut result = String::new();
    let mut i = 0;
    let bytes = input.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'&' {
            let end = bytes[i..].iter().position(|&b| b == b'-');
            match end {
                Some(0) => {
                    result.push('&');
                    i += 2;
                }
                Some(len) => {
                    let encoded = &input[i + 1..i + len];
                    if encoded.is_empty() {
                        result.push('&');
                        i += 2;
                    } else {
                        // IMAP-UTF-7 (RFC 3501 §5.1.3) encodes characters as
                        // base64 of their UTF-16BE bytes, using a modified
                        // alphabet (',' instead of '/') and NO padding. The
                        // STANDARD decoder needs padding — append it — and
                        // the result must be decoded as UTF-16BE, not UTF-8
                        // (e.g. '&APw-' = [0x00, 0xFC] = ü; interpreting it
                        // as UTF-8 fails and the char was silently dropped:
                        // "Entw&APw-rfe" -> "Entwfe").
                        let mut b64 = encoded.replace(',', "/").to_string();
                        while b64.len() % 4 != 0 {
                            b64.push('=');
                        }
                        use base64::Engine;
                        if let Ok(vec) = base64::engine::general_purpose::STANDARD.decode(&b64) {
                            if vec.len() % 2 == 0 {
                                let mut utf16: Vec<u16> = Vec::with_capacity(vec.len() / 2);
                                for chunk in vec.chunks_exact(2) {
                                    utf16.push(((chunk[0] as u16) << 8) | chunk[1] as u16);
                                }
                                if let Ok(s) = String::from_utf16(&utf16) {
                                    result.push_str(&s);
                                }
                            }
                        }
                        // `len` is the relative index of the terminating '-'
                        // in bytes[i..] — skipping i += len + 1 moves past
                        // '&' + encoded + '-'. (len + 2 would eat one
                        // following character: "Entw&APw-rfe" -> "Entwüfe".)
                        i += len + 1;
                    }
                }
                None => {
                    if let Some(c) = input[i..].chars().next() {
                        result.push(c);
                    }
                    i += 1;
                }
            }
        } else {
            if let Some(c) = input[i..].chars().next() {
                result.push(c);
            }
            i += 1;
        }
    }
    result
}

/// Encode a UTF-8 string as IMAP-UTF-7 (RFC 3501 §5.1.3): non-ASCII runs are
/// base64-encoded (modified alphabet: ',' instead of '/', no padding) between
/// '&' and '-'. '&' itself becomes '&-'.
///
/// Needed before sending folder names to IMAP commands (RENAME, SELECT,
/// CREATE, APPEND) — the server rejects raw UTF-8 ("unsupported folder name").
pub fn encode_imap_utf7(input: &str) -> String {
    let mut result = String::new();
    let mut ascii_buf = String::new();

    fn flush_ascii(result: &mut String, ascii_buf: &mut String) {
        if !ascii_buf.is_empty() {
            result.push_str(ascii_buf);
            ascii_buf.clear();
        }
    }

    for ch in input.chars() {
        if ch == '&' {
            flush_ascii(&mut result, &mut ascii_buf);
            result.push_str("&-");
        } else if ch.is_ascii() {
            ascii_buf.push(ch);
        } else {
            flush_ascii(&mut result, &mut ascii_buf);
            // UTF-16BE bytes of the char, base64 with modified alphabet.
            let mut buf = [0u16; 2];
            let units = ch.encode_utf16(&mut buf);
            let mut raw = Vec::with_capacity(units.len() * 2);
            for &u in units.iter() {
                raw.push((u >> 8) as u8);
                raw.push((u & 0xff) as u8);
            }
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD
                .encode(&raw)
                .trim_end_matches('=')
                .replace('/', ",");
            result.push('&');
            result.push_str(&b64);
            result.push('-');
        }
    }
    flush_ascii(&mut result, &mut ascii_buf);
    result
}

/// Find the IMAP folder name for a special folder type (Sent, Drafts, etc.)
/// by checking SPECIAL-USE attributes first, then falling back to common names.
pub fn find_special_folder(
    folders: &[FolderEntry],
    folder_type: SpecialFolder,
) -> Option<String> {
    // 1. Check SPECIAL-USE attributes (RFC 6154)
    let attr = folder_type.attr_name();
    if let Some(f) = folders.iter().find(|f| f.has_attribute(attr)) {
        return Some(f.name.clone());
    }

    // 2. Fallback: check common names (case-insensitive)
    let fallbacks = folder_type.fallback_names();
    for f in folders {
        if fallbacks.iter().any(|fb| f.name.eq_ignore_ascii_case(fb)) {
            return Some(f.name.clone());
        }
    }

    None
}

#[inline]
pub fn decode_transfer_encoding(input: &str) -> String {
    if input.is_empty() {
        return input.to_string();
    }
    let trimmed = input.trim();
    if trimmed.contains("=") && has_qp_pattern(trimmed) {
        return decode_quoted_printable(input);
    }
    let non_space: Vec<char> = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    if non_space.len() >= 4
        && non_space.len() % 4 == 0
        && non_space.iter().all(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/' || *c == '=')
    {
        use base64::Engine;
        let b64_str: String = non_space.iter().collect();
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64_str) {
            if let Ok(s) = String::from_utf8(bytes) {
                return s;
            }
        }
    }
    input.to_string()
}

#[inline]
pub fn has_qp_pattern(s: &str) -> bool {
    s.as_bytes().windows(3).any(|w| w[0] == b'=' && w[1].is_ascii_hexdigit() && w[2].is_ascii_hexdigit())
}

/// Walk a parsed IMAP BODYSTRUCTURE and decide whether the message carries a
/// user-facing attachment. A part counts as an attachment when its
/// Content-Disposition is "attachment", or when it has a filename/name
/// parameter and is not plain text (covers servers that omit disposition).
fn bodystructure_has_attachment(bs: &imap_proto::types::BodyStructure) -> bool {
    use imap_proto::types::BodyStructure as BS;

    fn part_is_attachment(common: &imap_proto::types::BodyContentCommon) -> bool {
        // Explicit attachment disposition.
        if let Some(disp) = &common.disposition {
            if disp.ty.eq_ignore_ascii_case("attachment") {
                return true;
            }
            // Inline parts with a filename are still downloadable attachments
            // (e.g. some inline PDFs); treat a filename param as an attachment.
            if let Some(params) = &disp.params {
                if params.iter().any(|(k, _)| k.eq_ignore_ascii_case("filename")) {
                    return true;
                }
            }
        }
        // Fallback: a "name" parameter on a non-text content type
        // (some servers omit Content-Disposition).
        let is_text = common.ty.ty.eq_ignore_ascii_case("text");
        if !is_text {
            if let Some(params) = &common.ty.params {
                if params.iter().any(|(k, _)| k.eq_ignore_ascii_case("name")) {
                    return true;
                }
            }
        }
        false
    }

    match bs {
        BS::Basic { common, .. } => part_is_attachment(common),
        BS::Text { common, .. } => part_is_attachment(common),
        BS::Message { common, body, .. } => {
            part_is_attachment(common) || bodystructure_has_attachment(body)
        }
       BS::Multipart { bodies, .. } => bodies.iter().any(bodystructure_has_attachment),
    }
}

/// Extract attachment metadata from BODYSTRUCTURE.
/// Returns filename, content_type, size for each attachment part.
fn extract_attachment_metadata(
    bs: &imap_proto::types::BodyStructure,
) -> Vec<AttachmentMeta> {
    let mut result = Vec::new();
    extract_metadata_recursive(bs, &mut result);
    result
}

   fn extract_metadata_recursive(
    bs: &imap_proto::types::BodyStructure,
    result: &mut Vec<AttachmentMeta>,
) {
    use imap_proto::types::BodyStructure as BS;

    match bs {
        BS::Basic { common, .. } | BS::Text { common, .. } => {
    if part_is_attachment_static(common) {
                result.push(AttachmentMeta {
                    filename: get_filename_static(common),
                    content_type: get_content_type_static(common),
                    size: get_octets_static(bs),
                });
            }
        }
        BS::Message { common, body, .. } => {
            if part_is_attachment_static(common) {
                result.push(AttachmentMeta {
                    filename: get_filename_static(common),
                    content_type: get_content_type_static(common),
                    size: get_octets_static(bs),
                });
            }
            extract_metadata_recursive(body, result);
        }
        BS::Multipart { bodies, .. } => {
            for body in bodies {
                extract_metadata_recursive(body, result);
            }
        }
    }
}

// Static versions to avoid closure issues
fn part_is_attachment_static(common: &imap_proto::types::BodyContentCommon) -> bool {
    if let Some(disp) = &common.disposition {
        if disp.ty.eq_ignore_ascii_case("attachment") {
            return true;
        }
        if let Some(params) = &disp.params {
            if params.iter().any(|(k, _)| k.eq_ignore_ascii_case("filename")) {
                return true;
            }
        }
    }
    let is_text = common.ty.ty.eq_ignore_ascii_case("text");
    if !is_text {
        if let Some(params) = &common.ty.params {
            if params.iter().any(|(k, _)| k.eq_ignore_ascii_case("name")) {
                return true;
            }
        }
    }
    false
}

fn get_filename_static(common: &imap_proto::types::BodyContentCommon) -> String {
    if let Some(disp) = &common.disposition {
        if let Some(params) = &disp.params {
            if let Some((_, filename)) = params.iter().find(|(k, _)| k.eq_ignore_ascii_case("filename")) {
                return filename.to_string();
            }
        }
    }
    if let Some(params) = &common.ty.params {
        if let Some((_, name)) = params.iter().find(|(k, _)| k.eq_ignore_ascii_case("name")) {
            return name.to_string();
        }
    }
    "anhang".to_string()
}

fn get_octets_static(bs: &imap_proto::types::BodyStructure) -> usize {
    use imap_proto::types::BodyStructure as BS;
    match bs {
        BS::Basic { other, .. } => other.octets as usize,
        BS::Text { other, .. } => other.octets as usize,
        BS::Message { other, .. } => other.octets as usize,
        BS::Multipart { .. } => 0,
    }
}

fn get_content_type_static(common: &imap_proto::types::BodyContentCommon) -> String {
    let ty = common.ty.ty.to_lowercase();
    let subtype = common.ty.subtype.to_string().to_lowercase();
    format!("{}{}", ty, if subtype.is_empty() { String::new() } else { format!("/{}", subtype) })
}

/// Parse a raw RFC822 message and extract attachment data.
pub fn parse_message_attachments(raw: &[u8]) -> Vec<crate::smtp::client::EmailAttachment> {
    use mail_parser::MimeHeaders;
    use mail_parser::MessageParser;
    use base64::Engine;
    
    let Some(parsed) = MessageParser::default().parse(raw) else {
        return Vec::new();
    };
    
    let mut attachments = Vec::new();
    for part in parsed.attachments() {
        let filename = part.attachment_name().unwrap_or("anhang").to_string();
        let ct = part.content_type();
        let content_type = match ct {
            Some(c) => {
                let subtype = c.c_subtype.as_ref().map(|s| s.as_ref()).unwrap_or("octet-stream");
                format!("{}/{}", c.c_type, subtype)
            },
            None => "application/octet-stream".to_string(),
        };
        let contents = part.contents();
        let size = contents.len();
        let content = base64::engine::general_purpose::STANDARD.encode(contents);
        
        attachments.push(crate::smtp::client::EmailAttachment {
            filename,
            content_type,
            size,
            content,
        });
    }
    attachments
}

/// Parse a raw RFC822 message into clean `(plain_text, Option<html>)` using
/// `mail-parser`. Handles MIME structure, transfer-encoding and charset
/// decoding. Prefers the richest available bodies; falls back gracefully.
pub fn parse_message_bodies(raw: &[u8]) -> (String, Option<String>) {
    use mail_parser::MessageParser;

    use mail_parser::PartType;

    let Some(parsed) = MessageParser::default().parse(raw) else {
        // Could not parse as MIME — return a best-effort lossy text.
        return (String::from_utf8_lossy(raw).into_owned(), None);
    };

    // Only treat the message as HTML if it actually has an HTML part — so a
    // plain-text mail is rendered as text, not as synthesized markup. (Both
    // body_html/body_text below already have charset + transfer-encoding
    // applied by mail-parser.)
    let has_real_html = parsed
        .html_part(0)
        .map(|p| matches!(p.body, PartType::Html(_)))
        .unwrap_or(false);

    let html: Option<String> = if has_real_html {
        parsed
            .body_html(0)
            .map(|c| c.into_owned())
            .filter(|s| !s.trim().is_empty())
            // Defense in depth: never let plain text leak into body_html. Some
            // senders declare a text/html part whose content carries no markup
            // at all; storing that in body_html makes the UI render raw text
            // through the HTML branch and collapse the line breaks into a flow
            // paragraph. When there is no real markup, treat the mail as text.
            .filter(|s| contains_html_markup(s))
    } else {
        None
    };

    // Concatenate ALL text parts, not just the first one.
    // body_text(0) only returns the first text/plain part — for multipart
    // messages with multiple text parts this silently drops content.
    let mut text_parts: Vec<String> = Vec::new();
    let mut idx = 0;
    while let Some(c) = parsed.body_text(idx) {
        let s = c.into_owned();
        if !s.trim().is_empty() {
            text_parts.push(s);
        }
        idx += 1;
    }
    let text: String = if text_parts.is_empty() {
        String::new()
    } else {
        text_parts.join("\n\n")
    };

    (text, html)
}

/// Heuristic: does the (already decoded) HTML part actually contain markup?
/// Returns false for bare text / whitespace so it is never stored as
/// body_html (which would make the UI render plain text through the HTML
/// branch and lose the line breaks). Mirrors the frontend `isHtmlContent`.
pub fn contains_html_markup(s: &str) -> bool {
    let s = s.trim().to_ascii_lowercase();
    [
        "<!doctype html",
        "<html",
        "<body",
        "<p",
        "<div",
        "<br",
        "<table",
        "<span",
        "<a ",
        "<img",
        "<ul",
        "<ol",
        "<li",
        "<h1",
        "<h2",
        "<h3",
        "<strong",
        "<b",
        "<i",
        "<u",
        "<em",
        "<style",
        "<font",
        "<center",
    ]
    .iter()
    .any(|tag| s.contains(tag))
}

fn decode_quoted_printable(input: &str) -> String {
    if !input.contains('=') {
        return input.to_string();
    }
    let mut out = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' && i + 1 < bytes.len() {
            // Soft line break check
            if bytes[i + 1] == b'\r' || bytes[i + 1] == b'\n' {
                i += 1;
                while i < bytes.len() && (bytes[i] == b'\r' || bytes[i] == b'\n') {
                    i += 1;
                }
                continue;
            }
            // Hex byte check
            if i + 2 < bytes.len() {
                if let Some(byte) = hex_to_byte(bytes[i + 1], bytes[i + 2]) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[inline]
fn hex_to_byte(b1: u8, b2: u8) -> Option<u8> {
    let c1 = b1 as char;
    let c2 = b2 as char;
    let val1 = c1.to_digit(16)?;
    let val2 = c2.to_digit(16)?;
    Some((val1 * 16 + val2) as u8)
}



#[cfg(test)]
mod utf7_tests {
    use super::*;

    #[test]
    fn decode_gmx_umlaut_folders() {
        // GMX sends folder names as IMAP-UTF-7 (no base64 padding).
        // The old decoder dropped the umlaut: "Entwfe" instead of "Entwürfe".
        assert_eq!(decode_imap_utf7("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(decode_imap_utf7("Gel&APY-scht"), "Gelöscht");
        assert_eq!(decode_imap_utf7("&ANw-"), "Ü");
        assert_eq!(decode_imap_utf7("&AOQ-"), "ä");
        assert_eq!(decode_imap_utf7("Plain"), "Plain");
        assert_eq!(decode_imap_utf7("A&-B"), "A&B");
    }

    #[test]
    fn encode_umlaut_folders() {
        // Round-trip: encode(utf8) must produce what the server expects.
        assert_eq!(encode_imap_utf7("Entwürfe"), "Entw&APw-rfe");
        assert_eq!(encode_imap_utf7("Gelöscht"), "Gel&APY-scht");
        assert_eq!(encode_imap_utf7("ä"), "&AOQ-");
        assert_eq!(encode_imap_utf7("Ü"), "&ANw-");
        assert_eq!(encode_imap_utf7("Plain"), "Plain");
        assert_eq!(encode_imap_utf7("A&B"), "A&-B");
    }

    #[test]
    fn encode_decode_roundtrip() {
        for name in ["Entwürfe", "Gelöscht", "Müller & Söhne", "Privat", "Bikes", "Haus und Hof"] {
            let encoded = encode_imap_utf7(name);
            let decoded = decode_imap_utf7(&encoded);
            assert_eq!(decoded, name, "roundtrip fehlgeschlagen für {}", name);
        }
    }
}
