//! 有界 Git 统计缓存:容量/字节双上限 + LRU。
//!
//! 键由 `WorkspaceKey` + HEAD 提交 + 历史边界/配置指纹组成;调用方在读取前
//! 先做一次小型版本校验(HEAD 与几个关键文件),不匹配就不复用。

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use bytegit::{CommitId, WorkspaceKey};

/// 缓存键。`fingerprint` 来自 shallow/grafts/alternates/config 等小型版本信息。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StatsKey {
    pub workspace_key: WorkspaceKey,
    pub head: Option<CommitId>,
    pub fingerprint: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitStats {
    pub total: u64,
    pub by_day: BTreeMap<i64, u64>,
}

struct Entry {
    stats: CommitStats,
    bytes: usize,
    last_used: u64,
}

struct Inner {
    entries: HashMap<StatsKey, Entry>,
    bytes: usize,
    tick: u64,
}

pub struct StatsCache {
    inner: Mutex<Inner>,
    capacity: usize,
    max_bytes: usize,
}

impl Default for StatsCache {
    fn default() -> Self {
        Self::new(64, 64 * 1024 * 1024)
    }
}

impl StatsCache {
    pub fn new(capacity: usize, max_bytes: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                entries: HashMap::new(),
                bytes: 0,
                tick: 0,
            }),
            capacity: capacity.max(1),
            max_bytes,
        }
    }

    pub fn get(&self, key: &StatsKey) -> Option<CommitStats> {
        let mut inner = self.lock();
        inner.tick = inner.tick.wrapping_add(1);
        let tick = inner.tick;
        let entry = inner.entries.get_mut(key)?;
        entry.last_used = tick;
        Some(entry.stats.clone())
    }

    /// 插入结果。单项超过总预算时直接不缓存(不挤掉全部旧结果)。
    pub fn insert(&self, key: StatsKey, stats: CommitStats) {
        let bytes = estimate_bytes(&stats);
        if bytes > self.max_bytes {
            return;
        }
        let mut inner = self.lock();
        inner.tick = inner.tick.wrapping_add(1);
        let tick = inner.tick;
        if let Some(old) = inner.entries.remove(&key) {
            inner.bytes = inner.bytes.saturating_sub(old.bytes);
        }
        while !inner.entries.is_empty()
            && (inner.entries.len() >= self.capacity || inner.bytes + bytes > self.max_bytes)
        {
            let Some(victim) = inner
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(old) = inner.entries.remove(&victim) {
                inner.bytes = inner.bytes.saturating_sub(old.bytes);
            }
        }
        inner.bytes += bytes;
        inner.entries.insert(
            key,
            Entry {
                stats,
                bytes,
                last_used: tick,
            },
        );
    }

    /// 手动刷新/仓库修复时清空(失败可重建,不需要数据库迁移)。
    pub fn clear(&self) {
        let mut inner = self.lock();
        inner.entries.clear();
        inner.bytes = 0;
    }

    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn bytes(&self) -> usize {
        self.lock().bytes
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn estimate_bytes(stats: &CommitStats) -> usize {
    std::mem::size_of::<CommitStats>() + stats.by_day.len() * 16 + 64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u64) -> StatsKey {
        // 用真实仓库构造 WorkspaceKey,避免伪造 bytegit 类型。
        let repo = bytegit::testutil::TempRepo::new();
        let key = repo.open().identity().unwrap().workspace_key().unwrap();
        StatsKey {
            workspace_key: key,
            head: None,
            fingerprint: n,
        }
    }

    fn stats(n: u64) -> CommitStats {
        CommitStats {
            total: n,
            by_day: BTreeMap::from([(1, n)]),
        }
    }

    #[test]
    fn hits_and_lru_eviction() {
        let cache = StatsCache::new(2, 1024 * 1024);
        let (a, b, c) = (key(1), key(2), key(3));
        cache.insert(a.clone(), stats(1));
        cache.insert(b.clone(), stats(2));
        assert_eq!(cache.get(&a).unwrap().total, 1);
        cache.insert(c.clone(), stats(3));
        assert_eq!(cache.len(), 2);
        assert!(cache.get(&b).is_none(), "最久未使用的 b 应被淘汰");
        assert!(cache.get(&a).is_some());
        assert!(cache.get(&c).is_some());
    }

    #[test]
    fn oversized_result_is_not_cached() {
        let cache = StatsCache::new(64, 64);
        cache.insert(key(1), stats(1));
        assert!(cache.is_empty(), "超过总预算的结果直接不缓存");
    }

    #[test]
    fn clear_drops_everything() {
        let cache = StatsCache::default();
        cache.insert(key(1), stats(1));
        assert_eq!(cache.len(), 1);
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.bytes(), 0);
    }
}
