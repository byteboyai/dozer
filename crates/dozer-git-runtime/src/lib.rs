//! Dozer/Digger 共享的 Git 读取运行时。
//!
//! 职责边界见设计规格 §2:订阅身份、按数据类别合并的调度、generation 校验、
//! 监听桥接、失败/stale 状态与补偿校验。它**不做** UI 投影、不读数据库、不依赖
//! iced/byteui/dozer-app。
//!
//! 典型用法:
//!
//! ```no_run
//! # async fn demo() {
//! use dozer_git_runtime::{GitRuntime, RuntimeConfig};
//! let (runtime, mut events) = GitRuntime::start(RuntimeConfig::default());
//! let token = runtime
//!     .subscribe(1, "/path/to/project", true)
//!     .await
//!     .unwrap();
//! while let Some(event) = events.next().await {
//!     // 宿主校验 token 后把快照投影给 Files / Project / Agent。
//!     let _ = (&event, &token);
//! }
//! # }
//! ```
//!
//! 事件输出按 `project_id` 合并:同一项目未消费的旧监听批次会合并原因位/有界路径,
//! 旧快照会被最新快照替换。不会把 bytegit 的有界队列转成无界 UI 积压。
//!
//! 树/预览的 HIDDEN 名单不在运行时里:运行时不按目录名丢弃 Git 状态(否则
//! `target/` 里的已跟踪文件会漏 dirty),任何 Workdir 事件都保守失效 status;
//! 隐藏目录只由宿主在树投影时过滤。分类/路径合并在后台且有界。

mod identity;
mod query;
mod scheduler;
mod stats;
mod watch;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{Notify, mpsc, oneshot};

pub use identity::{Categories, Category, Generations, RefreshReason, SubscriptionToken};
pub use query::{
    BytegitBackend, Field, HeadData, QueryBackend, QueryFailure, QueryOutput, QueryRequest,
    RemoteData, StatusData, WorkspaceSnapshot,
};
pub use stats::{CommitStats, StatsCache, StatsKey};

pub(crate) use scheduler::Command;

/// 可注入时钟。生产用 [`SystemClock`];测试用可控时钟验证补偿校验与采样时间。
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Instant;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// 运行时配置。默认值与设计规格一致:全进程最多 4 个 Git 读取任务,正常 60s /
/// 降级 30s 补偿校验,组合刷新使用 `include_untracked + include_ignored +
/// detect_renames=false`。
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RuntimeConfig {
    pub max_concurrent_queries: usize,
    pub normal_validation: Duration,
    pub degraded_validation: Duration,
    pub status_options: bytegit::StatusOptions,
    pub watch_options: bytegit::WorkspaceWatchOptions,
    /// 是否桥接 bytegit 工作区监听。测试/无法使用平台 watcher 的宿主可关闭,
    /// 此时只依赖激活、手动刷新与补偿校验收敛。
    pub watch_enabled: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_concurrent_queries: 4,
            normal_validation: Duration::from_secs(60),
            degraded_validation: Duration::from_secs(30),
            status_options: bytegit::StatusOptions {
                include_untracked: true,
                include_ignored: true,
                detect_renames: false,
            },
            watch_options: bytegit::WorkspaceWatchOptions::default(),
            watch_enabled: true,
        }
    }
}

impl RuntimeConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_max_concurrent_queries(mut self, value: usize) -> Self {
        self.max_concurrent_queries = value.max(1);
        self
    }

    pub fn with_validation_intervals(mut self, normal: Duration, degraded: Duration) -> Self {
        self.normal_validation = normal;
        self.degraded_validation = degraded;
        self
    }

    pub fn with_watch(mut self, enabled: bool) -> Self {
        self.watch_enabled = enabled;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    message: String,
}

impl RuntimeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RuntimeError {}

impl From<QueryFailure> for RuntimeError {
    fn from(value: QueryFailure) -> Self {
        Self::new(value.message)
    }
}

impl From<bytegit::GitError> for RuntimeError {
    fn from(value: bytegit::GitError) -> Self {
        Self::new(value.message())
    }
}

/// 工作区监听批次中与树/预览有关的部分。`kinds` 是原因位,`paths` 有上限;
/// `FULL_RESCAN` 时路径为空。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchedPaths {
    pub kinds: bytegit::WorkspaceChangeKind,
    pub paths: Vec<PathBuf>,
}

/// 宿主消费的事件。所有事件都带当前订阅 token;token 不再有效的结果不会投递。
#[derive(Debug, Clone)]
pub enum RuntimeEvent {
    Watch {
        token: SubscriptionToken,
        change: Arc<WatchedPaths>,
    },
    Snapshot {
        token: SubscriptionToken,
        snapshot: Arc<WorkspaceSnapshot>,
    },
}

impl RuntimeEvent {
    pub fn project_id(&self) -> i64 {
        match self {
            Self::Watch { token, .. } | Self::Snapshot { token, .. } => token.project_id,
        }
    }

    pub fn token(&self) -> &SubscriptionToken {
        match self {
            Self::Watch { token, .. } | Self::Snapshot { token, .. } => token,
        }
    }
}

const MAX_PENDING_PATHS: usize = bytegit::DEFAULT_MAX_PATHS;

#[derive(Default)]
struct PendingEvent {
    token: Option<SubscriptionToken>,
    watch: Option<WatchedPaths>,
    snapshot: Option<Arc<WorkspaceSnapshot>>,
}

pub(crate) struct EventQueue {
    pending: Mutex<BTreeMap<i64, PendingEvent>>,
    notify: Notify,
    closed: AtomicBool,
}

impl EventQueue {
    fn new() -> Self {
        Self {
            pending: Mutex::new(BTreeMap::new()),
            notify: Notify::new(),
            closed: AtomicBool::new(false),
        }
    }

    pub(crate) fn publish_watch(&self, token: &SubscriptionToken, change: WatchedPaths) {
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        {
            let mut pending = self
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let entry = pending.entry(token.project_id).or_default();
            entry.token = Some(token.clone());
            match &mut entry.watch {
                Some(existing) => merge_watched(existing, change),
                None => entry.watch = Some(change),
            }
        }
        self.notify.notify_one();
    }

    pub(crate) fn publish_snapshot(
        &self,
        token: &SubscriptionToken,
        snapshot: Arc<WorkspaceSnapshot>,
    ) {
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        {
            let mut pending = self
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let entry = pending.entry(token.project_id).or_default();
            entry.token = Some(token.clone());
            entry.snapshot = Some(snapshot);
        }
        self.notify.notify_one();
    }

    pub(crate) fn remove(&self, project_id: i64) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.remove(&project_id);
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    fn take_first(&self) -> Option<RuntimeEvent> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while let Some((&project_id, _)) = pending.iter().next() {
            let entry = pending.get_mut(&project_id).unwrap();
            let Some(token) = entry.token.clone() else {
                pending.remove(&project_id);
                continue;
            };
            if let Some(change) = entry.watch.take() {
                return Some(RuntimeEvent::Watch {
                    token,
                    change: Arc::new(change),
                });
            }
            if let Some(snapshot) = entry.snapshot.take() {
                return Some(RuntimeEvent::Snapshot { token, snapshot });
            }
            pending.remove(&project_id);
        }
        None
    }
}

/// 运行时事件流。`next()` 在没有事件时等待;运行时关闭且队列排空后返回 `None`。
pub struct EventStream {
    queue: Arc<EventQueue>,
}

impl EventStream {
    pub async fn next(&mut self) -> Option<RuntimeEvent> {
        loop {
            // 先注册通知再检查队列,避免"检查为空 → 事件入队 → 才 await"的丢唤醒。
            let notified = self.queue.notify.notified();
            if let Some(event) = self.queue.take_first() {
                return Some(event);
            }
            if self.queue.closed.load(Ordering::SeqCst) {
                return None;
            }
            notified.await;
        }
    }

    /// 不等待地取一个事件(供 UI 轮询/测试用)。
    pub fn try_next(&mut self) -> Option<RuntimeEvent> {
        self.queue.take_first()
    }
}

fn merge_watched(existing: &mut WatchedPaths, incoming: WatchedPaths) {
    existing.kinds |= incoming.kinds;
    if existing
        .kinds
        .contains(bytegit::WorkspaceChangeKind::FULL_RESCAN)
    {
        existing.paths.clear();
        return;
    }
    let mut paths: BTreeSet<PathBuf> = std::mem::take(&mut existing.paths).into_iter().collect();
    for path in incoming.paths {
        if paths.len() >= MAX_PENDING_PATHS {
            existing.kinds |= bytegit::WorkspaceChangeKind::FULL_RESCAN;
            existing.paths.clear();
            return;
        }
        paths.insert(path);
    }
    existing.paths = paths.into_iter().collect();
}

#[derive(Default)]
pub(crate) struct Metrics {
    pub queries_started: AtomicU64,
    pub queries_finished: AtomicU64,
    pub queries_stale: AtomicU64,
    pub watch_batches: AtomicU64,
    pub full_rescans: AtomicU64,
    pub queue_depth: AtomicU64,
    pub max_queue_depth: AtomicU64,
    pub last_query_nanos: AtomicU64,
    pub degraded_events: AtomicU64,
    pub identity_mismatches: AtomicU64,
}

/// 可观测性快照。P50/P95 需要调用方按需采样;这里给累计次数与最近一次耗时。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MetricsSnapshot {
    pub queries_started: u64,
    pub queries_finished: u64,
    /// 因 generation 已过期/订阅已取消而丢弃的结果数。
    pub queries_stale: u64,
    pub watch_batches: u64,
    pub full_rescans: u64,
    pub queue_depth: u64,
    pub max_queue_depth: u64,
    pub last_query_nanos: u64,
    pub degraded_events: u64,
    pub identity_mismatches: u64,
}

impl Metrics {
    fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            queries_started: self.queries_started.load(Ordering::Relaxed),
            queries_finished: self.queries_finished.load(Ordering::Relaxed),
            queries_stale: self.queries_stale.load(Ordering::Relaxed),
            watch_batches: self.watch_batches.load(Ordering::Relaxed),
            full_rescans: self.full_rescans.load(Ordering::Relaxed),
            queue_depth: self.queue_depth.load(Ordering::Relaxed),
            max_queue_depth: self.max_queue_depth.load(Ordering::Relaxed),
            last_query_nanos: self.last_query_nanos.load(Ordering::Relaxed),
            degraded_events: self.degraded_events.load(Ordering::Relaxed),
            identity_mismatches: self.identity_mismatches.load(Ordering::Relaxed),
        }
    }
}

struct RuntimeHandle {
    cmd_tx: mpsc::UnboundedSender<Command>,
    /// 订阅意图版本:在 discover **之前**分配,调度器据此拒绝迟到的旧意图。
    next_subscribe_intent: AtomicU64,
    backend: Arc<dyn QueryBackend>,
    clock: Arc<dyn Clock>,
    events: Arc<EventQueue>,
    metrics: Arc<Metrics>,
}

impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(Command::Shutdown);
        self.events.close();
    }
}

/// 共享运行时句柄。可 `Clone`;所有 clone 都指向同一个进程内实例。
#[derive(Clone)]
pub struct GitRuntime {
    handle: Arc<RuntimeHandle>,
}

impl GitRuntime {
    /// 用生产后端与系统时钟启动。必须在 tokio runtime 内调用。
    pub fn start(config: RuntimeConfig) -> (Self, EventStream) {
        Self::start_with(config, Arc::new(BytegitBackend), Arc::new(SystemClock))
    }

    /// 注入后端/时钟启动(测试与宿主定制用)。
    pub fn start_with(
        config: RuntimeConfig,
        backend: Arc<dyn QueryBackend>,
        clock: Arc<dyn Clock>,
    ) -> (Self, EventStream) {
        let events = Arc::new(EventQueue::new());
        let metrics = Arc::new(Metrics::default());
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let inbox = Arc::new(watch::WatchInbox::new());
        let state = scheduler::RuntimeState::new(
            config,
            backend.clone(),
            clock.clone(),
            cmd_tx.clone(),
            events.clone(),
            metrics.clone(),
            inbox.clone(),
        );
        tokio::spawn(scheduler::run(state, cmd_rx, inbox));

        let handle = Arc::new(RuntimeHandle {
            cmd_tx,
            next_subscribe_intent: AtomicU64::new(0),
            backend,
            clock,
            events: events.clone(),
            metrics,
        });
        (Self { handle }, EventStream { queue: events })
    }

    /// 订阅一个项目。`path` 可以是仓库子目录;运行时向上解析工作区身份。
    /// `active=false` 时仍监听,但不参与定时重验,激活时才补一次全量。
    pub async fn subscribe(
        &self,
        project_id: i64,
        path: impl Into<PathBuf>,
        active: bool,
    ) -> Result<SubscriptionToken, RuntimeError> {
        let path = path.into();
        let intent = self
            .handle
            .next_subscribe_intent
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        let backend = self.handle.backend.clone();
        let discover_path = path.clone();
        let identity = tokio::task::spawn_blocking(move || backend.discover(&discover_path))
            .await
            .map_err(|e| RuntimeError::new(format!("discover 任务失败: {e}")))??;

        let (reply, reply_rx) = oneshot::channel();
        self.handle
            .cmd_tx
            .send(Command::Subscribe {
                project_id,
                intent,
                path,
                identity,
                active,
                reply,
            })
            .map_err(|_| RuntimeError::new("git runtime 已停止"))?;
        reply_rx
            .await
            .map_err(|_| RuntimeError::new("git runtime 已停止"))?
    }

    /// 退订。排队工作被清除,执行中的结果只丢弃。token 不匹配时无操作。
    pub fn unsubscribe(&self, token: &SubscriptionToken) {
        let _ = self.handle.cmd_tx.send(Command::Unsubscribe {
            token: token.clone(),
        });
    }

    /// 设置项目是否处于活动状态。由后台转为活动时补一次全量重验。
    pub fn set_active(&self, token: &SubscriptionToken, active: bool) {
        let _ = self.handle.cmd_tx.send(Command::SetActive {
            token: token.clone(),
            active,
        });
    }

    /// 按类别请求刷新。同类任务按工作区合并;手动刷新也走同一队列,不会并发出两份。
    pub fn refresh(&self, token: &SubscriptionToken, reason: RefreshReason) {
        let _ = self.handle.cmd_tx.send(Command::Refresh {
            token: token.clone(),
            reason,
        });
    }

    /// 手动刷新所有类别(绕过缓存;本版本没有统计缓存,只保证合并语义)。
    pub fn refresh_manual(&self, token: &SubscriptionToken) {
        self.refresh(token, RefreshReason::Manual);
    }

    /// 用注入时钟的当前时间跑一次补偿校验。生产还有内部 1s ticker;测试可精确控制。
    pub fn validate_now(&self) {
        let _ = self.handle.cmd_tx.send(Command::Tick {
            now: self.handle.clock.now(),
        });
    }

    pub fn metrics(&self) -> MetricsSnapshot {
        self.handle.metrics.snapshot()
    }

    /// 显式停止。所有 clone 之后都不能再订阅/刷新;事件流排空后返回 `None`。
    pub fn shutdown(self) {
        let _ = self.handle.cmd_tx.send(Command::Shutdown);
        self.handle.events.close();
    }
}

impl std::fmt::Debug for GitRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitRuntime")
            .field("metrics", &self.metrics())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytegit::testutil::TempRepo;

    fn token(project_id: i64) -> SubscriptionToken {
        let identity = TempRepo::new().open().identity().unwrap();
        SubscriptionToken {
            project_id,
            activation_epoch: 1,
            workspace_key: identity.workspace_key().unwrap(),
        }
    }

    #[test]
    fn snapshots_for_one_project_coalesce_to_the_latest() {
        let queue = EventQueue::new();
        let token = token(7);
        let older = Arc::new(WorkspaceSnapshot::empty(
            TempRepo::new().open().identity().unwrap(),
            "older",
            Instant::now(),
        ));
        let newer = Arc::new(WorkspaceSnapshot::empty(
            TempRepo::new().open().identity().unwrap(),
            "newer",
            Instant::now(),
        ));
        queue.publish_snapshot(&token, older);
        queue.publish_snapshot(&token, newer.clone());

        let event = queue.take_first().expect("应有一个合并后的事件");
        let RuntimeEvent::Snapshot {
            snapshot,
            token: got,
        } = event
        else {
            panic!("expected snapshot");
        };
        assert_eq!(got.project_id, 7);
        assert!(Arc::ptr_eq(&snapshot, &newer), "旧快照应被最新快照替换");
        assert!(queue.take_first().is_none());
    }

    #[test]
    fn watch_batches_coalesce_and_full_rescan_clears_paths() {
        let queue = EventQueue::new();
        let token = token(1);
        queue.publish_watch(
            &token,
            WatchedPaths {
                kinds: bytegit::WorkspaceChangeKind::WORKDIR,
                paths: vec![PathBuf::from("/r/a")],
            },
        );
        queue.publish_watch(
            &token,
            WatchedPaths {
                kinds: bytegit::WorkspaceChangeKind::REFS,
                paths: vec![PathBuf::from("/r/.git/HEAD")],
            },
        );
        let event = queue.take_first().unwrap();
        let RuntimeEvent::Watch { change, .. } = event else {
            panic!("expected watch");
        };
        assert!(
            change.kinds.contains(
                bytegit::WorkspaceChangeKind::WORKDIR | bytegit::WorkspaceChangeKind::REFS
            )
        );
        assert_eq!(change.paths.len(), 2);

        queue.publish_watch(
            &token,
            WatchedPaths {
                kinds: bytegit::WorkspaceChangeKind::FULL_RESCAN,
                paths: Vec::new(),
            },
        );
        queue.publish_watch(
            &token,
            WatchedPaths {
                kinds: bytegit::WorkspaceChangeKind::WORKDIR,
                paths: vec![PathBuf::from("/r/b")],
            },
        );
        let RuntimeEvent::Watch { change, .. } = queue.take_first().unwrap() else {
            panic!("expected watch");
        };
        assert!(
            change
                .kinds
                .contains(bytegit::WorkspaceChangeKind::FULL_RESCAN)
        );
        assert!(change.paths.is_empty());
    }

    #[test]
    fn unsubscribed_projects_drop_pending_events() {
        let queue = EventQueue::new();
        let token = token(3);
        queue.publish_watch(
            &token,
            WatchedPaths {
                kinds: bytegit::WorkspaceChangeKind::WORKDIR,
                paths: vec![PathBuf::from("/r/a")],
            },
        );
        queue.remove(3);
        assert!(queue.take_first().is_none());
    }
}
