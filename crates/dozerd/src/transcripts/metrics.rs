//! Content-free ingestion counters for budget verification.
use std::sync::atomic::{AtomicU64, Ordering};
#[derive(Default)]
pub(super) struct Metrics {
    pub bytes: AtomicU64,
    pub batches: AtomicU64,
    pub max_batch: AtomicU64,
    pub max_lock_us: AtomicU64,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct Snapshot {
    pub read_bytes: u64,
    pub committed_batches: u64,
    pub max_batch_bytes: u64,
    pub max_database_lock_us: u64,
}
impl Metrics {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            read_bytes: self.bytes.load(Ordering::Relaxed),
            committed_batches: self.batches.load(Ordering::Relaxed),
            max_batch_bytes: self.max_batch.load(Ordering::Relaxed),
            max_database_lock_us: self.max_lock_us.load(Ordering::Relaxed),
        }
    }
}
