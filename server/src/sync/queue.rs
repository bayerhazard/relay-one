use std::collections::BinaryHeap;
use std::cmp::Ordering;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncTaskType {
    FetchNew,
    #[allow(dead_code)]
    RefreshFlags,
    /// Summary job scoped to a folder: IMAP uids are only unique per folder,
    /// so the folder id must travel with the task to avoid summarizing the
    /// wrong message when two folders share a uid.
    GenerateAiSummary(u32, i64),
    /// Background: analyze queued diffs between AI draft and user's final text
    AnalyzeDiff,
    /// Background: refresh style fingerprints for recipients with new hints
    RefreshFingerprint,
    /// Background: write missing EML archive files for already-cached messages
    /// (backfill after switching an account to archive mode).
    BackfillEmails,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncTask {
    pub account_id: u32,
    pub task_type: SyncTaskType,
    pub created_at: Instant,
    pub retries: u32,
    pub max_retries: u32,
    pub priority: u8,
}

impl Ord for SyncTask {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.cmp(&other.priority)
            .then_with(|| other.created_at.cmp(&self.created_at))
    }
}

impl PartialOrd for SyncTask {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub struct SyncQueue {
    heap: Arc<Mutex<BinaryHeap<SyncTask>>>,
    base_delay: Duration,
    max_delay: Duration,
    consecutive_failures: Arc<AtomicU32>,
}

impl SyncQueue {
    pub fn new() -> Self {
        Self {
            heap: Arc::new(Mutex::new(BinaryHeap::new())),
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(300),
            consecutive_failures: Arc::new(AtomicU32::new(0)),
        }
    }

    pub async fn enqueue(&self, task: SyncTask) {
        self.heap.lock().await.push(task);
    }

    pub async fn dequeue(&self) -> Option<SyncTask> {
        self.heap.lock().await.pop()
    }

    #[allow(dead_code)]
    pub async fn is_empty(&self) -> bool {
        self.heap.lock().await.is_empty()
    }

    pub async fn len(&self) -> usize {
        self.heap.lock().await.len()
    }

    pub fn calculate_delay(&self, retries: u32) -> Duration {
        if retries == 0 {
            return Duration::ZERO;
        }
        // Clamp exponent to avoid overflow on pathological retry counts.
        let factor = 2u32.saturating_pow(retries.saturating_sub(1).min(6));
        let delay = self.base_delay.saturating_mul(factor);
        std::cmp::min(delay + jitter(), self.max_delay)
    }

    pub async fn record_success(&self) {
        self.consecutive_failures.store(0, AtomicOrdering::Relaxed);
    }

    pub async fn record_failure(&self) {
        self.consecutive_failures.fetch_add(1, AtomicOrdering::Relaxed);
    }

    pub async fn failure_count(&self) -> u32 {
        self.consecutive_failures.load(AtomicOrdering::Relaxed)
    }
}

#[inline]
fn jitter() -> Duration {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    Duration::from_millis(ns % 250)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(priority: u8, account_id: u32) -> SyncTask {
        SyncTask {
            account_id,
            task_type: SyncTaskType::FetchNew,
            created_at: Instant::now(),
            retries: 0,
            max_retries: 3,
            priority,
        }
    }

    fn task_older(priority: u8, account_id: u32, age_ms: u64) -> SyncTask {
        let mut t = task(priority, account_id);
        t.created_at = Instant::now() - Duration::from_millis(age_ms);
        t
    }

    #[tokio::test]
    async fn test_priority_ordering_higher_first() {
        let queue = SyncQueue::new();
        queue.enqueue(task(1, 1)).await;
        queue.enqueue(task(5, 2)).await;
        queue.enqueue(task(3, 3)).await;
        assert_eq!(queue.dequeue().await.unwrap().priority, 5);
        assert_eq!(queue.dequeue().await.unwrap().priority, 3);
        assert_eq!(queue.dequeue().await.unwrap().priority, 1);
    }

    #[tokio::test]
    async fn test_fifo_for_same_priority() {
        let queue = SyncQueue::new();
        queue.enqueue(task_older(5, 1, 100)).await;
        queue.enqueue(task_older(5, 2, 50)).await;
        // same priority: older tasks first (higher created_at = older)
        assert_eq!(queue.dequeue().await.unwrap().account_id, 1);
        assert_eq!(queue.dequeue().await.unwrap().account_id, 2);
    }

    #[tokio::test]
    async fn test_dequeue_empty_returns_none() {
        let queue = SyncQueue::new();
        assert_eq!(queue.dequeue().await, None);
    }

    #[tokio::test]
    async fn test_failure_count() {
        let queue = SyncQueue::new();
        assert_eq!(queue.failure_count().await, 0);
        queue.record_failure().await;
        assert_eq!(queue.failure_count().await, 1);
        queue.record_success().await;
        assert_eq!(queue.failure_count().await, 0);
    }

    #[test]
    fn test_calculate_delay_exponential_backoff() {
        let queue = SyncQueue::new();
        // retries=0 → Duration::ZERO (no jitter)
        assert_eq!(queue.calculate_delay(0), Duration::ZERO);
        // retries > 0 includes jitter (0-249ms) — assert within range
        let d1 = queue.calculate_delay(1);
        assert!(d1 >= Duration::from_secs(1) && d1 <= Duration::from_millis(1249),
            "expected 1s-1249ms, got {d1:?}");
        let d2 = queue.calculate_delay(2);
        assert!(d2 >= Duration::from_secs(2) && d2 <= Duration::from_millis(2249),
            "expected 2s-2249ms, got {d2:?}");
        let d3 = queue.calculate_delay(3);
        assert!(d3 >= Duration::from_secs(4) && d3 <= Duration::from_millis(4249),
            "expected 4s-4249ms, got {d3:?}");
    }
}
