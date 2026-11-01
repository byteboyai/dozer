//! 监听事件到调度器的有界入口。
//!
//! bytegit 的回调在监听线程上运行,不能直接 await 或阻塞调度器。这里用共享合并
//! 状态 + 可合并唤醒:同一工作区的连续批次只保留原因位与有界路径集合,调度器被
//! 唤醒后一次取走全部等待项。事件再多也不会按条堆积消息。

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Mutex;

use bytegit::{WorkspaceChange, WorkspaceChangeKind, WorkspaceKey};
use tokio::sync::Notify;

/// 单个工作区待处理路径的上限。与 bytegit 单批上限一致;跨批合并超限时清空明细
/// 并升级为 `FULL_RESCAN`。
const MAX_PENDING_PATHS: usize = bytegit::DEFAULT_MAX_PATHS;

#[derive(Debug, Default)]
pub(crate) struct WatchBatch {
    pub kinds: WorkspaceChangeKind,
    pub paths: Vec<PathBuf>,
}

impl WatchBatch {
    fn merge(&mut self, kinds: WorkspaceChangeKind, paths: Vec<PathBuf>) {
        self.kinds |= kinds;
        if self.kinds.contains(WorkspaceChangeKind::FULL_RESCAN) {
            self.paths.clear();
            return;
        }
        let mut merged: BTreeSet<PathBuf> = std::mem::take(&mut self.paths).into_iter().collect();
        for path in paths {
            if merged.len() >= MAX_PENDING_PATHS {
                self.kinds |= WorkspaceChangeKind::FULL_RESCAN;
                self.paths.clear();
                return;
            }
            merged.insert(path);
        }
        self.paths = merged.into_iter().collect();
    }
}

pub(crate) struct WatchInbox {
    state: Mutex<BTreeMap<(WorkspaceKey, u64), WatchBatch>>,
    pub notify: Notify,
}

impl WatchInbox {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(BTreeMap::new()),
            notify: Notify::new(),
        }
    }

    /// 监听线程调用;只做内存合并,不碰文件系统。
    pub fn push(&self, key: WorkspaceKey, generation: u64, change: WorkspaceChange) {
        self.merge(key, generation, change.kinds, change.paths);
    }

    pub(crate) fn merge(
        &self,
        key: WorkspaceKey,
        generation: u64,
        kinds: WorkspaceChangeKind,
        paths: Vec<PathBuf>,
    ) {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state
                .entry((key, generation))
                .or_default()
                .merge(kinds, paths);
        }
        self.notify.notify_one();
    }

    /// 调度器调用:一次取走全部等待项,唤醒信号自然合并。
    pub fn take(&self) -> Vec<((WorkspaceKey, u64), WatchBatch)> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *state).into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytegit::testutil::TempRepo;

    fn key() -> WorkspaceKey {
        TempRepo::new()
            .open()
            .identity()
            .unwrap()
            .workspace_key()
            .unwrap()
    }

    fn change(kinds: WorkspaceChangeKind, paths: &[&str]) -> (WorkspaceChangeKind, Vec<PathBuf>) {
        (kinds, paths.iter().map(PathBuf::from).collect())
    }

    #[test]
    fn batches_for_the_same_workspace_merge_reasons_and_paths() {
        let inbox = WatchInbox::new();
        let key = key();
        let (kinds, paths) = change(WorkspaceChangeKind::WORKDIR, &["/r/a"]);
        inbox.merge(key.clone(), 0, kinds, paths);
        let (kinds, paths) = change(WorkspaceChangeKind::REFS, &["/r/.git/HEAD"]);
        inbox.merge(key.clone(), 0, kinds, paths);

        let batches = inbox.take();
        assert_eq!(batches.len(), 1);
        let ((got_key, _), batch) = &batches[0];
        assert_eq!(got_key, &key);
        assert!(
            batch
                .kinds
                .contains(WorkspaceChangeKind::WORKDIR | WorkspaceChangeKind::REFS)
        );
        assert_eq!(batch.paths.len(), 2);
        assert!(inbox.take().is_empty(), "取走之后状态清空");
    }

    #[test]
    fn a_full_rescan_clears_path_details_and_survives_later_path_pushes() {
        let inbox = WatchInbox::new();
        let key = key();
        let (kinds, paths) = change(WorkspaceChangeKind::WORKDIR, &["/r/a"]);
        inbox.merge(key.clone(), 0, kinds, paths);
        let (kinds, paths) = change(WorkspaceChangeKind::FULL_RESCAN, &[]);
        inbox.merge(key.clone(), 0, kinds, paths);
        let (kinds, paths) = change(WorkspaceChangeKind::WORKDIR, &["/r/b"]);
        inbox.merge(key, 0, kinds, paths);

        let batch = &inbox.take()[0].1;
        assert!(batch.kinds.contains(WorkspaceChangeKind::FULL_RESCAN));
        assert!(batch.paths.is_empty());
    }

    #[test]
    fn distinct_workspaces_do_not_mix() {
        let inbox = WatchInbox::new();
        let a = key();
        let b = key();
        let (kinds, paths) = change(WorkspaceChangeKind::WORKDIR, &["/a"]);
        inbox.merge(a.clone(), 0, kinds, paths);
        let (kinds, paths) = change(WorkspaceChangeKind::INDEX, &["/b"]);
        inbox.merge(b.clone(), 0, kinds, paths);

        let mut batches = inbox.take();
        batches.sort_by_key(|(k, _)| k.clone());
        assert_eq!(batches.len(), 2);
        assert!(
            batches.iter().any(
                |((k, _), batch)| k == &a && batch.kinds.contains(WorkspaceChangeKind::WORKDIR)
            )
        );
        assert!(
            batches
                .iter()
                .any(|((k, _), batch)| k == &b && batch.kinds.contains(WorkspaceChangeKind::INDEX))
        );
    }

    #[test]
    fn a_burst_of_paths_stays_bounded_and_keeps_reason_bits() {
        let inbox = WatchInbox::new();
        let key = key();
        for i in 0..(MAX_PENDING_PATHS + 100) {
            let (kinds, paths) = change(
                WorkspaceChangeKind::WORKDIR,
                &[&format!("/r/dir/{i}/file.txt")],
            );
            inbox.merge(key.clone(), 0, kinds, paths);
        }
        let batch = &inbox.take()[0].1;
        assert!(batch.kinds.contains(WorkspaceChangeKind::WORKDIR));
        assert!(batch.kinds.contains(WorkspaceChangeKind::FULL_RESCAN));
        assert!(batch.paths.is_empty());
    }
}

#[cfg(test)]
mod incarnation_tests {
    use super::*;

    #[test]
    fn callbacks_from_different_watch_incarnations_do_not_merge() {
        let repo = bytegit::testutil::TempRepo::new();
        let key = repo.open().identity().unwrap().workspace_key().unwrap();
        let inbox = WatchInbox::new();
        inbox.merge(
            key.clone(),
            1,
            WorkspaceChangeKind::WORKDIR,
            vec!["/old".into()],
        );
        inbox.merge(
            key.clone(),
            2,
            WorkspaceChangeKind::REFS,
            vec!["/new".into()],
        );
        let batches = inbox.take();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].0, (key.clone(), 1));
        assert_eq!(batches[0].1.paths, vec![PathBuf::from("/old")]);
        assert_eq!(batches[1].0, (key, 2));
        assert_eq!(batches[1].1.paths, vec![PathBuf::from("/new")]);
    }
}
