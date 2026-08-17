pub mod accounts;
pub mod archive;
pub mod attachments;
pub mod db;
pub mod delete_queue;
pub mod fingerprint;
pub mod learning;
pub mod messages;
pub mod settings;
pub mod sync_state;
pub mod snippets;
pub mod topic;

use std::collections::HashMap;
use std::time::Instant;

use parking_lot::Mutex;
use serde_json::Value;

/// Small server-side LRU for the meta-only folder listings returned by
/// `GET /api/v1/messages`. Keyed by `(account_id, folder)`, stores the
/// serialized JSON so repeated folder switches skip both the DB query and the
/// JSON re-serialization. Entries expire after a short TTL so a folder refresh
/// (background sync) shows up within seconds.
pub struct FolderListCache {
    inner: Mutex<Inner>,
    ttl: std::time::Duration,
    max_entries: usize,
}

struct Inner {
    entries: HashMap<(i64, String), Entry>,
    order: Vec<(i64, String)>,
}

struct Entry {
    payload: Vec<Value>,
    created: Instant,
}

impl FolderListCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                entries: HashMap::new(),
                order: Vec::new(),
            }),
            ttl: std::time::Duration::from_secs(5),
            max_entries: 64,
        }
    }

    pub fn get(&self, account_id: i64, folder: &str) -> Option<Vec<Value>> {
        let mut inner = self.inner.lock();
        let key = (account_id, folder.to_string());
        let entry = inner.entries.get(&key)?;
        if entry.created.elapsed() > self.ttl {
            inner.entries.remove(&key);
            if let Some(pos) = inner.order.iter().position(|k| *k == key) {
                inner.order.remove(pos);
            }
            return None;
        }
        Some(entry.payload.clone())
    }

    pub fn put(&self, account_id: i64, folder: &str, payload: Vec<Value>) {
        let mut inner = self.inner.lock();
        let key = (account_id, folder.to_string());
        if inner.entries.contains_key(&key) {
            if let Some(pos) = inner.order.iter().position(|k| *k == key) {
                inner.order.remove(pos);
            }
        }
        inner.entries.insert(key.clone(), Entry {
            payload,
            created: Instant::now(),
        });
        inner.order.push(key.clone());
        // Evict oldest when over the cap.
        while inner.order.len() > self.max_entries {
            if let Some(oldest) = inner.order.first().cloned() {
                inner.order.remove(0);
                inner.entries.remove(&oldest);
            }
        }
    }
}

#[cfg(test)]
mod integration_tests;
