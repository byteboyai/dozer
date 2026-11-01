//! 单实例调度器:命令循环、按工作区合并、全局并发上限、generation 校验与补偿校验。
//!
//! 状态只由调度任务持有,其它任务通过命令通道回传结果;监听线程通过
//! [`WatchInbox`] 合并事件后唤醒调度器。这样没有共享可变的工作区状态,也不会在
//! 监听/查询线程上执行 UI 投影。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytegit::{RepoIdentity, WorkspaceKey};
use tokio::sync::{Semaphore, mpsc, oneshot};

use crate::identity::{Categories, Generations, RefreshReason, SubscriptionToken};
use crate::query::{
    QueryFailure, QueryOutput, QueryRequest, WorkspaceSnapshot, merge_failure,
    merge_identity_mismatch, merge_output,
};
use crate::watch::WatchInbox;
use crate::{Clock, EventQueue, Metrics, QueryBackend, RuntimeConfig, RuntimeError, WatchedPaths};

pub(crate) enum Command {
    Subscribe {
        project_id: i64,
        /// 在 discover 之前分配的订阅意图版本。
        intent: u64,
        path: PathBuf,
        identity: RepoIdentity,
        active: bool,
        reply: oneshot::Sender<Result<SubscriptionToken, RuntimeError>>,
    },
    Unsubscribe {
        token: SubscriptionToken,
    },
    SetActive {
        token: SubscriptionToken,
        active: bool,
    },
    Refresh {
        token: SubscriptionToken,
        reason: RefreshReason,
    },
    Tick {
        now: Instant,
    },
    QueryFinished {
        key: WorkspaceKey,
        query_id: u64,
        generations: Generations,
        result: Box<Result<QueryOutput, QueryFailure>>,
        sampled_at: Instant,
        duration: Duration,
    },
    WatchStarted {
        key: WorkspaceKey,
        generation: u64,
        result: Result<bytegit::WorkspaceWatchHandle, String>,
    },
    Shutdown,
}

pub(crate) struct RuntimeState {
    config: RuntimeConfig,
    backend: Arc<dyn QueryBackend>,
    clock: Arc<dyn Clock>,
    cmd_tx: mpsc::UnboundedSender<Command>,
    events: Arc<EventQueue>,
    metrics: Arc<Metrics>,
    semaphore: Arc<Semaphore>,
    inbox: Arc<WatchInbox>,
    next_epoch: u64,
    next_query_id: u64,
    // 执行中的任务独立于订阅存活,退订重开不能绕过同工作区单飞行约束。
    running: HashMap<WorkspaceKey, u64>,
    workspaces: HashMap<WorkspaceKey, WorkspaceEntry>,
    projects: HashMap<i64, SubscriptionToken>,
    /// 每个项目最近接受的订阅意图;迟到意图不能撤销新订阅。
    subscribe_intents: HashMap<i64, u64>,
}

struct Subscriber {
    token: SubscriptionToken,
    #[allow(dead_code)] // 保留给宿主投影/诊断;当前调度只需要 token 与 active。
    path: PathBuf,
    #[allow(dead_code)]
    prefix: PathBuf,
    active: bool,
}

struct WorkspaceEntry {
    identity: RepoIdentity,
    subscribers: HashMap<i64, Subscriber>,
    generations: Generations,
    last: Option<Arc<WorkspaceSnapshot>>,
    pending: Option<PendingWork>,
    inflight: Option<InFlight>,
    watch: Option<bytegit::WorkspaceWatchHandle>,
    watch_starting: bool,
    watch_generation: u64,
    watch_degraded: Vec<bytegit::WatchDegraded>,
    last_validation: Instant,
    degraded: bool,
}

#[derive(Default)]
struct PendingWork {
    categories: Categories,
    #[allow(dead_code)] // 本版本没有统计缓存;保留手动刷新标记给后续缓存层。
    manual: bool,
}

struct InFlight {
    query_id: u64,
    categories: Categories,
    generations: Generations,
}

impl RuntimeState {
    pub(crate) fn new(
        config: RuntimeConfig,
        backend: Arc<dyn QueryBackend>,
        clock: Arc<dyn Clock>,
        cmd_tx: mpsc::UnboundedSender<Command>,
        events: Arc<EventQueue>,
        metrics: Arc<Metrics>,
        inbox: Arc<WatchInbox>,
    ) -> Self {
        let permits = config.max_concurrent_queries.max(1);
        Self {
            config,
            backend,
            clock,
            cmd_tx,
            events,
            metrics,
            semaphore: Arc::new(Semaphore::new(permits)),
            inbox,
            next_epoch: 0,
            next_query_id: 0,
            running: HashMap::new(),
            workspaces: HashMap::new(),
            projects: HashMap::new(),
            subscribe_intents: HashMap::new(),
        }
    }

    fn now(&self) -> Instant {
        self.clock.now()
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Subscribe {
                project_id,
                intent,
                path,
                identity,
                active,
                reply,
            } => self.subscribe(project_id, intent, path, identity, active, reply),
            Command::Unsubscribe { token } => self.unsubscribe(token),
            Command::SetActive { token, active } => self.set_active(token, active),
            Command::Refresh { token, reason } => self.refresh(token, reason),
            Command::Tick { now } => self.on_tick(now),
            Command::QueryFinished {
                key,
                query_id,
                generations,
                result,
                sampled_at,
                duration,
            } => self.finish_query(key, query_id, generations, *result, sampled_at, duration),
            Command::WatchStarted {
                key,
                generation,
                result,
            } => self.watch_started(key, generation, result),
            Command::Shutdown => {}
        }
    }

    fn subscribe(
        &mut self,
        project_id: i64,
        intent: u64,
        path: PathBuf,
        identity: RepoIdentity,
        active: bool,
        reply: oneshot::Sender<Result<SubscriptionToken, RuntimeError>>,
    ) {
        // 订阅意图在 discover 前分配:迟到的旧意图不能撤销新订阅。
        if self
            .subscribe_intents
            .get(&project_id)
            .is_some_and(|current| intent < *current)
        {
            let _ = reply.send(Err(RuntimeError::new("subscription intent superseded")));
            return;
        }
        self.subscribe_intents.insert(project_id, intent);
        // 同一 project_id 重新订阅:先清掉旧订阅与尚未消费的旧事件。
        if let Some(old) = self.projects.remove(&project_id) {
            self.remove_subscriber(&old);
            self.events.remove(project_id);
        }

        let key = match identity.workspace_key() {
            Ok(key) => key,
            Err(error) => {
                let _ = reply.send(Err(RuntimeError::from(error)));
                return;
            }
        };

        let epoch = self.next_epoch;
        self.next_epoch = self.next_epoch.wrapping_add(1);
        let token = SubscriptionToken {
            project_id,
            activation_epoch: epoch,
            workspace_key: key.clone(),
        };

        let now = self.now();
        let canonical = path.canonicalize().unwrap_or(path);
        let workdir = identity
            .workdir_root()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| identity.git_dir().to_path_buf());
        let prefix = canonical
            .strip_prefix(&workdir)
            .map(Path::to_path_buf)
            .unwrap_or_default();

        let (is_new, last, needs_initial) = {
            let entry = self
                .workspaces
                .entry(key.clone())
                .or_insert_with(|| WorkspaceEntry {
                    identity: identity.clone(),
                    subscribers: HashMap::new(),
                    generations: Generations::default(),
                    last: None,
                    pending: None,
                    inflight: None,
                    watch: None,
                    watch_starting: false,
                    watch_generation: epoch,
                    watch_degraded: Vec::new(),
                    last_validation: now,
                    degraded: false,
                });
            let is_new = entry.subscribers.is_empty();
            entry.subscribers.insert(
                project_id,
                Subscriber {
                    token: token.clone(),
                    path: canonical,
                    prefix,
                    active,
                },
            );
            let needs_initial = is_new
                || (entry.last.is_none() && entry.pending.is_none() && entry.inflight.is_none());
            (is_new, entry.last.clone(), needs_initial)
        };

        self.projects.insert(project_id, token.clone());
        if let Some(snapshot) = last {
            // 同工作区已有快照:新订阅立即可用;飞行中的查询会把结果发给所有订阅者。
            self.events.publish_snapshot(&token, snapshot);
        }
        if is_new {
            self.start_watch(&key);
        }
        let _ = reply.send(Ok(token));
        if needs_initial {
            self.request(&key, Categories::ALL, false, RefreshReason::Activated);
        }
        self.pump();
    }

    fn unsubscribe(&mut self, token: SubscriptionToken) {
        if !self.token_is_current(&token) {
            return;
        }
        self.projects.remove(&token.project_id);
        self.events.remove(token.project_id);
        self.remove_subscriber(&token);
        self.pump();
    }

    fn remove_subscriber(&mut self, token: &SubscriptionToken) {
        let mut empty = false;
        if let Some(entry) = self.workspaces.get_mut(&token.workspace_key) {
            let current = entry
                .subscribers
                .get(&token.project_id)
                .is_some_and(|s| s.token.activation_epoch == token.activation_epoch);
            if current {
                entry.subscribers.remove(&token.project_id);
            }
            empty = entry.subscribers.is_empty();
        }
        if empty {
            // 订阅态可删除,但 running 与执行任务持有的许可必须保留到任务完成。
            self.workspaces.remove(&token.workspace_key);
        }
    }

    fn set_active(&mut self, token: SubscriptionToken, active: bool) {
        if !self.token_is_current(&token) {
            return;
        }
        let mut activated = false;
        if let Some(entry) = self.workspaces.get_mut(&token.workspace_key)
            && let Some(subscriber) = entry.subscribers.get_mut(&token.project_id)
        {
            activated = active && !subscriber.active;
            subscriber.active = active;
            if activated {
                entry.last_validation = self.clock.now();
            }
        }
        if activated {
            self.request(
                &token.workspace_key,
                Categories::ALL,
                false,
                RefreshReason::ActivatedValidation,
            );
            self.pump();
        }
    }

    fn refresh(&mut self, token: SubscriptionToken, reason: RefreshReason) {
        if !self.token_is_current(&token) {
            return;
        }
        self.request(
            &token.workspace_key,
            reason.categories(),
            reason == RefreshReason::Manual,
            reason,
        );
        self.pump();
    }

    fn token_is_current(&self, token: &SubscriptionToken) -> bool {
        self.projects.get(&token.project_id) == Some(token)
    }

    /// 合并一次失效请求:只推进受影响类别的 generation,工作区级等待集合取并集。
    fn request(
        &mut self,
        key: &WorkspaceKey,
        categories: Categories,
        manual: bool,
        reason: RefreshReason,
    ) {
        if categories.is_empty() {
            return;
        }
        if reason == RefreshReason::FullRescan {
            self.metrics
                .full_rescans
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let Some(entry) = self.workspaces.get_mut(key) else {
            return;
        };
        entry.generations.bump(categories);
        let pending = entry.pending.get_or_insert_with(PendingWork::default);
        pending.categories |= categories;
        pending.manual |= manual;
    }

    fn drain_watch(&mut self) {
        let batches = self.inbox.take();
        for ((key, generation), batch) in batches {
            if !self
                .workspaces
                .get(&key)
                .is_some_and(|entry| entry.watch_generation == generation)
            {
                continue;
            }
            self.metrics
                .watch_batches
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let kinds = batch.kinds;
            if kinds.contains(bytegit::WorkspaceChangeKind::FULL_RESCAN) {
                self.metrics
                    .full_rescans
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            let categories = Categories::from_change(kinds);
            let watched = WatchedPaths {
                kinds,
                paths: batch.paths,
            };

            let tokens = {
                let Some(entry) = self.workspaces.get_mut(&key) else {
                    continue;
                };
                if entry.subscribers.is_empty() {
                    continue;
                }
                entry
                    .subscribers
                    .values()
                    .map(|subscriber| subscriber.token.clone())
                    .collect::<Vec<_>>()
            };
            for token in tokens {
                self.events.publish_watch(&token, watched.clone());
            }
            let reason = if kinds.contains(bytegit::WorkspaceChangeKind::FULL_RESCAN) {
                RefreshReason::FullRescan
            } else {
                RefreshReason::Workdir
            };
            self.request(&key, categories, false, reason);
        }
        self.pump();
    }

    fn on_tick(&mut self, now: Instant) {
        let normal = self.config.normal_validation;
        let degraded = self.config.degraded_validation;
        let keys: Vec<WorkspaceKey> = self.workspaces.keys().cloned().collect();
        for key in keys {
            let due = {
                let Some(entry) = self.workspaces.get(&key) else {
                    continue;
                };
                // 只重验活动项目;后台项目仅监听,激活时再补一次。
                if !entry
                    .subscribers
                    .values()
                    .any(|subscriber| subscriber.active)
                {
                    continue;
                }
                let stale = entry.degraded
                    || !entry.watch_degraded.is_empty()
                    || entry.last.as_ref().is_some_and(|last| last.is_stale());
                let interval = if stale { degraded } else { normal };
                now.saturating_duration_since(entry.last_validation) >= interval
            };
            if due {
                self.request(&key, Categories::ALL, false, RefreshReason::Validation);
                if let Some(entry) = self.workspaces.get_mut(&key) {
                    entry.last_validation = now;
                }
            }
        }
        self.pump();
    }

    fn start_watch(&mut self, key: &WorkspaceKey) {
        if !self.config.watch_enabled {
            return;
        }
        let (identity, options, generation) = {
            let Some(entry) = self.workspaces.get_mut(key) else {
                return;
            };
            if entry.watch_starting || entry.watch.is_some() {
                return;
            }
            entry.watch_starting = true;
            (
                entry.identity.clone(),
                self.config.watch_options.clone(),
                entry.watch_generation,
            )
        };

        let inbox = self.inbox.clone();
        let cmd_tx = self.cmd_tx.clone();
        let callback_key = key.clone();
        let command_key = key.clone();
        tokio::spawn(async move {
            let inner_key = callback_key.clone();
            let result = tokio::task::spawn_blocking(move || {
                bytegit::watch_workspace(&identity, options, move |change| {
                    inbox.push(inner_key.clone(), generation, change);
                })
                .map_err(|error| error.message().to_string())
            })
            .await
            .unwrap_or_else(|error| Err(format!("watch 任务失败: {error}")));
            let _ = cmd_tx.send(Command::WatchStarted {
                key: command_key,
                generation,
                result,
            });
        });
    }

    fn watch_started(
        &mut self,
        key: WorkspaceKey,
        generation: u64,
        result: Result<bytegit::WorkspaceWatchHandle, String>,
    ) {
        let Some(entry) = self.workspaces.get_mut(&key) else {
            return;
        };
        if entry.watch_generation != generation {
            return;
        }
        entry.watch_starting = false;
        match result {
            Ok(handle) => {
                entry.watch_degraded = handle.degraded().to_vec();
                entry.watch = Some(handle);
            }
            Err(message) => {
                entry.watch_degraded = vec![bytegit::WatchDegraded {
                    path: None,
                    message,
                }];
            }
        }
        if !entry.watch_degraded.is_empty() {
            entry.degraded = true;
            self.metrics
                .degraded_events
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn finish_query(
        &mut self,
        key: WorkspaceKey,
        query_id: u64,
        captured: Generations,
        result: Result<QueryOutput, QueryFailure>,
        sampled_at: Instant,
        duration: Duration,
    ) {
        // 完成身份来自全运行时单调 ID,不是可被重建 entry 复用的局部计数。
        if self.running.get(&key) != Some(&query_id) {
            self.metrics
                .queries_stale
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        self.running.remove(&key);
        let Some(entry) = self.workspaces.get_mut(&key) else {
            self.metrics
                .queries_stale
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.pump();
            return;
        };
        let Some(inflight) = entry.inflight.take() else {
            self.metrics
                .queries_stale
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.pump();
            return;
        };
        if inflight.query_id != query_id {
            entry.inflight = Some(inflight);
            self.metrics
                .queries_stale
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }

        let InFlight {
            categories,
            generations: launched,
            ..
        } = inflight;

        // 只应用仍处于当前 generation 的类别;飞行期间又被失效的类别保留旧值,
        // 等排队的补跑任务返回。
        let valid = still_valid(launched, entry.generations) & categories;
        if valid != categories {
            self.metrics
                .queries_stale
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let previous_head = entry.last.as_ref().and_then(|s| s.head.value().cloned());
        let previous_generations = entry
            .last
            .as_ref()
            .map(|s| s.generations)
            .unwrap_or_default();
        let mut applied_generations = previous_generations;
        if valid.contains(Categories::HEAD) {
            applied_generations.head = captured.head;
        }
        if valid.contains(Categories::STATUS) {
            applied_generations.status = captured.status;
        }
        if valid.contains(Categories::BRANCHES) {
            applied_generations.branches = captured.branches;
        }
        if valid.contains(Categories::REMOTES) {
            applied_generations.remotes = captured.remotes;
        }

        let mut head_changed = false;
        let mut identity_mismatch = false;
        let snapshot = match result {
            Ok(output) => {
                let observed_key = output.identity.workspace_key().ok();
                if observed_key.as_ref() != Some(&key) {
                    identity_mismatch = true;
                    merge_identity_mismatch(
                        entry.last.as_deref(),
                        entry.identity.clone(),
                        output.identity,
                        categories,
                        applied_generations,
                        sampled_at,
                        duration,
                    )
                } else {
                    let merged = merge_output(
                        entry.last.as_deref(),
                        output.identity.clone(),
                        output,
                        valid,
                        applied_generations,
                        sampled_at,
                        duration,
                    );
                    // 首次加载没有"旧 HEAD",不能算作变化,否则每次订阅都会多补一次 status。
                    head_changed = previous_head.is_some()
                        && valid.contains(Categories::HEAD)
                        && merged.head.value().is_some()
                        && previous_head != merged.head.value().cloned();
                    merged
                }
            }
            Err(failure) => merge_failure(
                entry.last.as_deref(),
                entry.identity.clone(),
                categories,
                failure.message,
                applied_generations,
                sampled_at,
                duration,
            ),
        };

        self.metrics
            .queries_finished
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.metrics.last_query_nanos.store(
            duration.as_nanos().min(u64::MAX as u128) as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        if identity_mismatch {
            self.metrics
                .identity_mismatches
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        let degraded = snapshot.is_stale() || !entry.watch_degraded.is_empty();
        if degraded {
            self.metrics
                .degraded_events
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        entry.degraded = degraded;
        let snapshot = Arc::new(snapshot);
        entry.last = Some(snapshot.clone());
        let tokens: Vec<SubscriptionToken> = entry
            .subscribers
            .values()
            .map(|subscriber| subscriber.token.clone())
            .collect();
        for token in tokens {
            self.events.publish_snapshot(&token, snapshot.clone());
        }
        entry.last_validation = sampled_at;

        if head_changed && !identity_mismatch {
            // Refs 变化先验证 HEAD;HEAD 真的变了才补一次 status。
            self.request(&key, Categories::STATUS, false, RefreshReason::Head);
        }
        self.pump();
    }

    fn pump(&mut self) {
        loop {
            let mut keys: Vec<WorkspaceKey> = self
                .workspaces
                .iter()
                .filter(|(key, entry)| {
                    entry.inflight.is_none()
                        && entry.pending.is_some()
                        && !self.running.contains_key(*key)
                })
                .map(|(key, _)| key.clone())
                .collect();
            // 活动订阅优先进入全局并发名额;后台项目排在后面,但不会饿死。
            keys.sort_by_key(|key| {
                self.workspaces
                    .get(key)
                    .is_some_and(|entry| !entry.subscribers.values().any(|s| s.active))
            });
            if keys.is_empty() {
                break;
            }
            let mut started_any = false;
            for key in keys {
                if self.try_start(&key) {
                    started_any = true;
                }
            }
            if !started_any {
                break;
            }
        }
        let depth = self
            .workspaces
            .values()
            .filter(|entry| entry.pending.is_some())
            .count() as u64;
        self.metrics
            .queue_depth
            .store(depth, std::sync::atomic::Ordering::Relaxed);
        self.metrics
            .max_queue_depth
            .fetch_max(depth, std::sync::atomic::Ordering::Relaxed);
    }

    fn try_start(&mut self, key: &WorkspaceKey) -> bool {
        if self.running.contains_key(key) {
            return false;
        }
        let Some(entry) = self.workspaces.get_mut(key) else {
            return false;
        };
        if entry.inflight.is_some() || entry.pending.is_none() {
            return false;
        }
        let Ok(permit) = self.semaphore.clone().try_acquire_owned() else {
            return false;
        };

        let pending = entry.pending.take().unwrap_or_default();
        if pending.categories.is_empty() {
            return false;
        }
        let query_id = self.next_query_id;
        self.next_query_id = self
            .next_query_id
            .checked_add(1)
            .expect("query ID exhausted");
        let generations = entry.generations;
        let workdir = entry
            .identity
            .workdir_root()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| entry.identity.git_dir().to_path_buf());
        entry.inflight = Some(InFlight {
            query_id,
            categories: pending.categories,
            generations,
        });
        self.running.insert(key.clone(), query_id);
        entry.last_validation = self.clock.now();

        let status_options = self.config.status_options;
        let backend = self.backend.clone();
        let cmd_tx = self.cmd_tx.clone();
        let clock = self.clock.clone();
        let command_key = key.clone();
        self.metrics
            .queries_started
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        tokio::spawn(async move {
            let request = QueryRequest {
                path: workdir,
                categories: pending.categories,
                status_options,
            };
            let started = clock.now();
            let result = tokio::task::spawn_blocking(move || {
                // 许可由真实任务持有,包括 panic 展开;删除订阅不会提前归还名额。
                let _permit = permit;
                backend.query(&request)
            })
            .await
            .unwrap_or_else(|error| Err(QueryFailure::new(format!("查询任务失败: {error}"))));
            let finished = clock.now();
            let _ = cmd_tx.send(Command::QueryFinished {
                key: command_key,
                query_id,
                generations,
                result: Box::new(result),
                sampled_at: finished,
                duration: finished.saturating_duration_since(started),
            });
        });
        true
    }
}

pub(crate) async fn run(
    mut state: RuntimeState,
    mut commands: mpsc::UnboundedReceiver<Command>,
    inbox: Arc<WatchInbox>,
) {
    let mut ticker = tokio::time::interval(Duration::from_millis(500));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let notified = inbox.notify.notified();
        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Shutdown) | None => break,
                Some(command) => state.handle(command),
            },
            _ = notified => state.drain_watch(),
            _ = ticker.tick() => {
                let now = state.now();
                state.on_tick(now);
            }
        }
    }
    // 显式清掉所有 watcher;执行中的查询结果会因为找不到工作区被丢弃。
    state.workspaces.clear();
}

fn still_valid(captured: Generations, current: Generations) -> Categories {
    let mut valid = Categories::empty();
    if captured.head == current.head {
        valid.insert(Categories::HEAD);
    }
    if captured.status == current.status {
        valid.insert(Categories::STATUS);
    }
    if captured.branches == current.branches {
        valid.insert(Categories::BRANCHES);
    }
    if captured.remotes == current.remotes {
        valid.insert(Categories::REMOTES);
    }
    valid
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BytegitBackend, SystemClock};

    #[tokio::test]
    async fn a_late_older_subscribe_intent_cannot_replace_the_newer_one() {
        let repo = bytegit::testutil::TempRepo::new();
        let identity = repo.open().identity().unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut state = RuntimeState::new(
            RuntimeConfig::default().with_watch(false),
            Arc::new(BytegitBackend),
            Arc::new(SystemClock),
            tx,
            Arc::new(EventQueue::new()),
            Arc::new(Metrics::default()),
            Arc::new(WatchInbox::new()),
        );

        let (newer_tx, mut newer_rx) = oneshot::channel();
        state.subscribe(
            1,
            2,
            repo.path().to_path_buf(),
            identity.clone(),
            true,
            newer_tx,
        );
        let newer = newer_rx.try_recv().unwrap().unwrap();

        let (older_tx, mut older_rx) = oneshot::channel();
        state.subscribe(1, 1, repo.path().to_path_buf(), identity, true, older_tx);
        assert!(
            older_rx.try_recv().unwrap().is_err(),
            "迟到的旧意图必须被拒绝"
        );
        assert_eq!(
            state.projects.get(&1).map(|token| token.activation_epoch),
            Some(newer.activation_epoch),
            "新订阅不能被旧意图撤销"
        );
    }

    #[test]
    fn retired_watch_start_and_callbacks_cannot_attach_to_a_new_incarnation() {
        let repo = bytegit::testutil::TempRepo::new();
        let identity = repo.open().identity().unwrap();
        let key = identity.workspace_key().unwrap();
        let token = SubscriptionToken {
            project_id: 1,
            activation_epoch: 2,
            workspace_key: key.clone(),
        };
        let (tx, _rx) = mpsc::unbounded_channel();
        let inbox = Arc::new(WatchInbox::new());
        let events = Arc::new(EventQueue::new());
        let mut state = RuntimeState::new(
            RuntimeConfig::default().with_watch(false),
            Arc::new(BytegitBackend),
            Arc::new(SystemClock),
            tx,
            events,
            Arc::new(Metrics::default()),
            inbox.clone(),
        );
        state.workspaces.insert(
            key.clone(),
            WorkspaceEntry {
                identity,
                subscribers: HashMap::from([(
                    1,
                    Subscriber {
                        token,
                        path: repo.path().into(),
                        prefix: PathBuf::new(),
                        active: false,
                    },
                )]),
                generations: Generations::default(),
                last: None,
                pending: None,
                inflight: None,
                watch: None,
                watch_starting: true,
                watch_generation: 2,
                watch_degraded: Vec::new(),
                last_validation: Instant::now(),
                degraded: false,
            },
        );
        state.watch_started(key.clone(), 1, Err("retired watcher".into()));
        let entry = state.workspaces.get(&key).unwrap();
        assert!(entry.watch_starting);
        assert!(entry.watch_degraded.is_empty());
        inbox.merge(
            key.clone(),
            1,
            bytegit::WorkspaceChangeKind::WORKDIR,
            vec![],
        );
        state.drain_watch();
        let entry = state.workspaces.get(&key).unwrap();
        assert_eq!(entry.generations.status, 0);
        assert!(entry.pending.is_none());
        state.watch_started(key.clone(), 2, Err("current watcher".into()));
        let entry = state.workspaces.get(&key).unwrap();
        assert!(!entry.watch_starting);
        assert_eq!(entry.watch_degraded[0].message, "current watcher");
    }
}
