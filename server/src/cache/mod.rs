pub mod accounts;
pub mod archive;
pub mod attachments;
pub mod cal;
pub mod contacts;
pub mod db;
pub mod delete_queue;
pub mod fingerprint;
pub mod learning;
pub mod messages;
pub mod settings;
pub mod sync_state;
pub mod snippets;
pub mod todo;
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
            ttl: std::time::Duration::from_secs(30),
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

    /// Drop a single cached folder listing. Called by mutating endpoints
    /// (move/delete/flag/mark-read) so the raised TTL never serves a list
    /// that misses the user's own change.
    pub fn invalidate(&self, account_id: i64, folder: &str) {
        let mut inner = self.inner.lock();
        let key = (account_id, folder.to_string());
        inner.entries.remove(&key);
        if let Some(pos) = inner.order.iter().position(|k| *k == key) {
            inner.order.remove(pos);
        }
    }

    /// Drop every cached folder of one account (used when the affected
    /// folder of a mutation is unknown, e.g. search-driven deletes).
    pub fn invalidate_account(&self, account_id: i64) {
        let mut inner = self.inner.lock();
        inner.entries.retain(|(acct, _), _| *acct != account_id);
        inner.order.retain(|(acct, _)| *acct != account_id);
    }

    pub fn put(&self, account_id: i64, folder: &str, payload: Vec<Value>) {        let mut inner = self.inner.lock();
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
mod tests {
    use super::*;

    fn payload(uids: &[i64]) -> Vec<Value> {
        uids.iter()
            .map(|u| serde_json::json!({ "uid": u }))
            .collect()
    }

    #[test]
    fn put_and_get_roundtrip() {
        let cache = FolderListCache::new();
        assert!(cache.get(1, "INBOX").is_none());
        cache.put(1, "INBOX", payload(&[1, 2]));
        let got = cache.get(1, "INBOX").unwrap();
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn keys_are_per_account_and_folder() {
        let cache = FolderListCache::new();
        cache.put(1, "INBOX", payload(&[1]));
        cache.put(2, "INBOX", payload(&[9]));
        assert!(cache.get(1, "Archive").is_none());
        assert_eq!(cache.get(2, "INBOX").unwrap().len(), 1);
    }

    #[test]
    fn invalidate_drops_only_the_given_folder() {
        let cache = FolderListCache::new();
        cache.put(1, "INBOX", payload(&[1]));
        cache.put(1, "Archive", payload(&[2]));
        cache.invalidate(1, "INBOX");
        assert!(cache.get(1, "INBOX").is_none());
        assert!(cache.get(1, "Archive").is_some());
    }

    #[test]
    fn invalidate_account_drops_every_folder_of_that_account() {
        let cache = FolderListCache::new();
        cache.put(1, "INBOX", payload(&[1]));
        cache.put(1, "Sent", payload(&[2]));
        cache.put(2, "INBOX", payload(&[3]));
        cache.invalidate_account(1);
        assert!(cache.get(1, "INBOX").is_none());
        assert!(cache.get(1, "Sent").is_none());
        // Other accounts survive.
        assert!(cache.get(2, "INBOX").is_some());
    }
}

#[cfg(test)]
mod integration_tests;
