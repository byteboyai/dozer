//! 调度器行为测试:用注入后端/时钟验证合并、并发、乱序、身份与补偿语义。

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use bytegit::RepoIdentity;
use bytegit::testutil::TempRepo;
use dozer_git_runtime::{
    Categories, Clock, EventStream, GitRuntime, HeadData, QueryBackend, QueryFailure, QueryOutput,
    QueryRequest, RefreshReason, RuntimeConfig, RuntimeEvent, StatusData, SubscriptionToken,
    WorkspaceSnapshot,
};

// ---- 夹具 ----

struct TestClock(Mutex<Instant>);

impl TestClock {
    fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(Instant::now())))
    }

    fn advance(&self, duration: Duration) {
        let mut now = self.0.lock().unwrap();
        *now += duration;
    }
}

impl Clock for TestClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

struct Blocker {
    open: Mutex<bool>,
    wake: Condvar,
}

impl Blocker {
    fn closed() -> Arc<Self> {
        Arc::new(Self {
            open: Mutex::new(false),
            wake: Condvar::new(),
        })
    }

    fn wait(&self) {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.wake.wait(open).unwrap();
        }
    }

    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.wake.notify_all();
    }
}

struct TestBackend {
    identities: Mutex<HashMap<PathBuf, RepoIdentity>>,
    fallback: RepoIdentity,
    responses: Mutex<VecDeque<Result<QueryOutput, QueryFailure>>>,
    requests: Mutex<Vec<QueryRequest>>,
    discover_calls: AtomicUsize,
    query_calls: AtomicUsize,
    status_calls: AtomicUsize,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    blocker: Option<Arc<Blocker>>,
}

impl TestBackend {
    fn new(identity: RepoIdentity) -> Arc<Self> {
        Self::build(identity, None)
    }

    fn with_blocker(identity: RepoIdentity, blocker: Arc<Blocker>) -> Arc<Self> {
        Self::build(identity, Some(blocker))
    }

    fn build(identity: RepoIdentity, blocker: Option<Arc<Blocker>>) -> Arc<Self> {
        Arc::new(Self {
            identities: Mutex::new(HashMap::new()),
            fallback: identity,
            responses: Mutex::new(VecDeque::new()),
            requests: Mutex::new(Vec::new()),
            discover_calls: AtomicUsize::new(0),
            query_calls: AtomicUsize::new(0),
            status_calls: AtomicUsize::new(0),
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
            blocker,
        })
    }

    fn add_identity(&self, identity: &RepoIdentity) {
        let root = identity
            .workdir_root()
            .expect("测试仓库都有工作区")
            .to_path_buf();
        self.identities
            .lock()
            .unwrap()
            .insert(root, identity.clone());
    }

    fn push(&self, response: Result<QueryOutput, QueryFailure>) {
        self.responses.lock().unwrap().push_back(response);
    }

    fn requests(&self) -> Vec<QueryRequest> {
        self.requests.lock().unwrap().clone()
    }

    fn queries_for(&self, path: &Path) -> usize {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.path == path)
            .count()
    }
}

impl QueryBackend for TestBackend {
    fn discover(&self, path: &Path) -> Result<RepoIdentity, QueryFailure> {
        self.discover_calls.fetch_add(1, Ordering::SeqCst);
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        Ok(self
            .identities
            .lock()
            .unwrap()
            .get(&path)
            .cloned()
            .unwrap_or_else(|| self.fallback.clone()))
    }

    fn query(&self, request: &QueryRequest) -> Result<QueryOutput, QueryFailure> {
        self.query_calls.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().unwrap().push(request.clone());
        if request.categories.contains(Categories::STATUS) {
            self.status_calls.fetch_add(1, Ordering::SeqCst);
        }
        let in_flight = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(in_flight, Ordering::SeqCst);
        if let Some(blocker) = &self.blocker {
            blocker.wait();
        }
        self.in_flight.fetch_sub(1, Ordering::SeqCst);

        let identity = self
            .identities
            .lock()
            .unwrap()
            .get(&request.path)
            .cloned()
            .unwrap_or_else(|| self.fallback.clone());
        let mut responses = self.responses.lock().unwrap();
        responses
            .pop_front()
            .unwrap_or_else(|| Ok(default_output(&identity, request)))
    }
}

fn default_output(identity: &RepoIdentity, request: &QueryRequest) -> QueryOutput {
    QueryOutput {
        identity: identity.clone(),
        head: request.categories.contains(Categories::HEAD).then(|| {
            Ok(HeadData {
                branch: Some("main".into()),
                commit: None,
            })
        }),
        status: request.categories.contains(Categories::STATUS).then(|| {
            Ok(StatusData {
                entries: Vec::new(),
                dirty: false,
            })
        }),
        branches: request
            .categories
            .contains(Categories::BRANCHES)
            .then(|| Ok(vec!["main".to_string()])),
        remotes: request
            .categories
            .contains(Categories::REMOTES)
            .then(|| Ok(Vec::new())),
    }
}

fn identity(repo: &TempRepo) -> RepoIdentity {
    repo.open().identity().unwrap()
}

fn status(dirty: bool) -> Result<StatusData, String> {
    Ok(StatusData {
        entries: Vec::new(),
        dirty,
    })
}

async fn next_snapshot(events: &mut EventStream, project_id: i64) -> Arc<WorkspaceSnapshot> {
    next_snapshot_where(events, project_id, |_| true).await
}

async fn next_snapshot_where(
    events: &mut EventStream,
    project_id: i64,
    mut predicate: impl FnMut(&WorkspaceSnapshot) -> bool,
) -> Arc<WorkspaceSnapshot> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.next().await {
                Some(RuntimeEvent::Snapshot { token, snapshot })
                    if token.project_id == project_id && predicate(&snapshot) =>
                {
                    return snapshot;
                }
                Some(_) => {}
                None => panic!("event stream closed before expected snapshot"),
            }
        }
    })
    .await
    .expect("等待快照超时")
}

async fn wait_until(mut predicate: impl FnMut() -> bool) {
    for _ in 0..500 {
        if predicate() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("等待条件超时");
}

fn config() -> RuntimeConfig {
    RuntimeConfig::default()
        .with_validation_intervals(Duration::from_secs(60), Duration::from_secs(30))
        // 测试用注入后端;关闭真实 watcher,避免临时目录的 FS 事件干扰计数。
        .with_watch(false)
}

// ---- 测试 ----

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_query_discovers_and_reads_status_once() {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "1\n", "c1");
    let backend = TestBackend::new(identity(&repo));
    let (runtime, mut events) = GitRuntime::start_with(config(), backend.clone(), TestClock::new());

    let token = runtime.subscribe(1, repo.path(), true).await.unwrap();
    let snapshot = next_snapshot(&mut events, 1).await;
    assert!(snapshot.head.is_ready());
    assert_eq!(backend.discover_calls.load(Ordering::SeqCst), 1);
    assert_eq!(backend.query_calls.load(Ordering::SeqCst), 1);
    assert_eq!(backend.status_calls.load(Ordering::SeqCst), 1);

    // refs 只验证 HEAD/branches;HEAD 没变就不再扫 status。
    runtime.refresh(&token, RefreshReason::Refs);
    let refreshed =
        next_snapshot_where(&mut events, 1, |snapshot| snapshot.generations.head >= 1).await;
    assert!(refreshed.generations.head >= 1);
    let last = backend.requests().pop().unwrap();
    assert_eq!(last.categories, Categories::HEAD | Categories::BRANCHES);
    assert_eq!(
        backend.status_calls.load(Ordering::SeqCst),
        1,
        "refs 刷新不能重扫工作区"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn projects_in_the_same_repository_share_one_snapshot_and_query() {
    let repo = TempRepo::new();
    repo.commit_file("a.txt", "1\n", "c1");
    repo.write_untracked("sub/.keep", "");
    let backend = TestBackend::new(identity(&repo));
    let (runtime, mut events) = GitRuntime::start_with(config(), backend.clone(), TestClock::new());

    let _p1 = runtime.subscribe(1, repo.path(), true).await.unwrap();
    let first = next_snapshot(&mut events, 1).await;
    let queries = backend.query_calls.load(Ordering::SeqCst);

    let _p2 = runtime
        .subscribe(2, repo.path().join("sub"), true)
        .await
        .unwrap();
    let second = next_snapshot(&mut events, 2).await;
    assert_eq!(
        backend.query_calls.load(Ordering::SeqCst),
        queries,
        "同工作区第二个项目应复用已有快照,不重复查询"
    );
    assert!(
        Arc::ptr_eq(&first, &second),
        "同一次快照应共享给同仓库的多个项目"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_activation_revalidates_an_existing_workspace() {
    let repo = TempRepo::new();
    let backend = TestBackend::new(identity(&repo));
    let clock = TestClock::new();
    let (runtime, mut events) = GitRuntime::start_with(config(), backend.clone(), clock.clone());

    let token = runtime.subscribe(1, repo.path(), false).await.unwrap();
    let _ = next_snapshot(&mut events, 1).await;
    let before = backend.queries_for(repo.path());

    clock.advance(Duration::from_secs(61));
    runtime.validate_now();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        backend.queries_for(repo.path()),
        before,
        "非活动项目不参与定时重验"
    );

    runtime.set_active(&token, true);
    wait_until(|| backend.queries_for(repo.path()) > before).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn global_concurrency_is_capped_and_queued_work_finishes() {
    let blocker = Blocker::closed();
    let repos: Vec<TempRepo> = (0..6).map(|_| TempRepo::new()).collect();
    let backend = TestBackend::with_blocker(identity(&repos[0]), blocker.clone());
    for repo in &repos {
        backend.add_identity(&identity(repo));
    }
    let (runtime, mut events) = GitRuntime::start_with(
        config().with_max_concurrent_queries(4),
        backend.clone(),
        TestClock::new(),
    );

    let mut tokens = Vec::new();
    for (index, repo) in repos.iter().enumerate() {
        tokens.push(
            runtime
                .subscribe(index as i64 + 1, repo.path(), true)
                .await
                .unwrap(),
        );
    }
    wait_until(|| backend.in_flight.load(Ordering::SeqCst) == 4).await;
    assert_eq!(backend.max_in_flight.load(Ordering::SeqCst), 4);

    blocker.release();
    let mut seen = std::collections::BTreeSet::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while seen.len() < 6 {
            if let Some(RuntimeEvent::Snapshot { token, .. }) = events.next().await {
                seen.insert(token.project_id);
            }
        }
    })
    .await
    .expect("6 个工作区都应完成");
    assert_eq!(seen.len(), 6);
    assert!(
        backend.max_in_flight.load(Ordering::SeqCst) <= 4,
        "全进程读取任务不能超过配置上限"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_invalidated_in_flight_result_is_dropped_and_rerun() {
    let repo = TempRepo::new();
    let blocker = Blocker::closed();
    let backend = TestBackend::with_blocker(identity(&repo), blocker.clone());
    backend.add_identity(&identity(&repo));
    let mut first = default_output(
        &identity(&repo),
        &QueryRequest {
            path: identity(&repo).workdir_root().unwrap().to_path_buf(),
            categories: Categories::ALL,
            status_options: bytegit::StatusOptions::default(),
        },
    );
    first.status = Some(status(true));
    backend.push(Ok(first));
    let mut second = default_output(
        &identity(&repo),
        &QueryRequest {
            path: identity(&repo).workdir_root().unwrap().to_path_buf(),
            categories: Categories::STATUS,
            status_options: bytegit::StatusOptions::default(),
        },
    );
    second.status = Some(status(false));
    backend.push(Ok(second));

    let (runtime, mut events) = GitRuntime::start_with(config(), backend.clone(), TestClock::new());
    let token = runtime.subscribe(1, repo.path(), true).await.unwrap();
    wait_until(|| backend.in_flight.load(Ordering::SeqCst) == 1).await;

    // 第一份查询还在飞行时,Workdir 失效把 status generation 推进;旧结果必须丢弃。
    runtime.refresh(&token, RefreshReason::Workdir);
    blocker.release();

    let snapshot = next_snapshot_where(&mut events, 1, |snapshot| {
        snapshot.status.value().is_some_and(|status| !status.dirty)
    })
    .await;
    assert!(!snapshot.status.value().unwrap().dirty);
    assert!(
        backend.query_calls.load(Ordering::SeqCst) >= 2,
        "被作废的查询后必须补跑"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_b_a_resubscription_invalidates_old_tokens() {
    let a = TempRepo::new();
    let b = TempRepo::new();
    let backend = TestBackend::new(identity(&a));
    backend.add_identity(&identity(&b));
    let (runtime, mut events) = GitRuntime::start_with(config(), backend.clone(), TestClock::new());

    let token_a1 = runtime.subscribe(1, a.path(), true).await.unwrap();
    let _ = next_snapshot(&mut events, 1).await;
    let token_b = runtime.subscribe(1, b.path(), true).await.unwrap();
    let _ = next_snapshot(&mut events, 1).await;
    let token_a2 = runtime.subscribe(1, a.path(), true).await.unwrap();
    let _ = next_snapshot(&mut events, 1).await;

    assert!(token_a1.activation_epoch < token_b.activation_epoch);
    assert!(token_b.activation_epoch < token_a2.activation_epoch);
    assert_eq!(token_a1.workspace_key, token_a2.workspace_key);
    assert_ne!(token_a1.workspace_key, token_b.workspace_key);

    let before = backend.query_calls.load(Ordering::SeqCst);
    runtime.refresh(&token_a1, RefreshReason::Workdir);
    runtime.refresh(&token_b, RefreshReason::Workdir);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        backend.query_calls.load(Ordering::SeqCst),
        before,
        "旧 token 的请求不能生效"
    );

    runtime.refresh(&token_a2, RefreshReason::Workdir);
    wait_until(|| backend.query_calls.load(Ordering::SeqCst) > before).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unsubscribe_then_resubscribe_starts_a_new_epoch() {
    let repo = TempRepo::new();
    let backend = TestBackend::new(identity(&repo));
    let (runtime, mut events) = GitRuntime::start_with(config(), backend.clone(), TestClock::new());

    let old = runtime.subscribe(1, repo.path(), true).await.unwrap();
    let _ = next_snapshot(&mut events, 1).await;
    runtime.unsubscribe(&old);

    let before = backend.query_calls.load(Ordering::SeqCst);
    runtime.refresh(&old, RefreshReason::Workdir);
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(backend.query_calls.load(Ordering::SeqCst), before);

    let new = runtime.subscribe(1, repo.path(), true).await.unwrap();
    assert!(new.activation_epoch > old.activation_epoch);
    let _ = next_snapshot(&mut events, 1).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_field_failure_keeps_the_last_good_value_and_marks_stale() {
    let repo = TempRepo::new();
    let backend = TestBackend::new(identity(&repo));
    let id = identity(&repo);
    let workdir = id.workdir_root().unwrap().to_path_buf();
    let request = QueryRequest {
        path: workdir,
        categories: Categories::ALL,
        status_options: bytegit::StatusOptions::default(),
    };
    backend.push(Ok(default_output(&id, &request)));
    let mut failed = default_output(&id, &request);
    failed.head = Some(Err("HEAD 读取失败".into()));
    backend.push(Ok(failed));

    let (runtime, mut events) = GitRuntime::start_with(config(), backend.clone(), TestClock::new());
    let token = runtime.subscribe(1, repo.path(), true).await.unwrap();
    let first = next_snapshot(&mut events, 1).await;
    let old_head = first.head.value().cloned().expect("首次 HEAD 成功");

    runtime.refresh_manual(&token);
    let failed =
        next_snapshot_where(&mut events, 1, |snapshot| snapshot.head.error().is_some()).await;
    assert_eq!(failed.head.last(), Some(&old_head));
    assert!(failed.is_stale());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn identity_mismatch_does_not_apply_the_new_repositorys_data() {
    let subscribed = TempRepo::new();
    let observed = TempRepo::new();
    let backend = TestBackend::new(identity(&subscribed));
    let subscribed_id = identity(&subscribed);
    let observed_id = identity(&observed);
    let workdir = subscribed_id.workdir_root().unwrap().to_path_buf();
    backend.push(Ok(QueryOutput {
        identity: observed_id.clone(),
        head: Some(Ok(HeadData {
            branch: Some("other".into()),
            commit: None,
        })),
        status: Some(status(true)),
        branches: Some(Ok(vec!["other".into()])),
        remotes: Some(Ok(Vec::new())),
    }));
    let _ = workdir;

    let (runtime, mut events) = GitRuntime::start_with(config(), backend.clone(), TestClock::new());
    let _ = runtime.subscribe(1, subscribed.path(), true).await.unwrap();
    let snapshot = next_snapshot(&mut events, 1).await;
    assert_eq!(snapshot.identity_mismatch.as_ref(), Some(&observed_id));
    assert!(snapshot.head.value().is_none(), "不能把新仓库数据当旧仓库");
    assert!(snapshot.is_stale());
    assert_eq!(runtime.metrics().identity_mismatches, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_whole_query_failure_is_an_explainable_unavailable_snapshot() {
    let repo = TempRepo::new();
    let backend = TestBackend::new(identity(&repo));
    backend.push(Err(QueryFailure::new("boom")));
    let (runtime, mut events) = GitRuntime::start_with(config(), backend.clone(), TestClock::new());
    let _ = runtime.subscribe(1, repo.path(), true).await.unwrap();
    let snapshot = next_snapshot(&mut events, 1).await;
    assert!(!snapshot.has_data());
    assert!(snapshot.is_stale());
    assert_eq!(snapshot.head.error(), Some("boom"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_worktrees_are_distinct_workspaces() {
    let main = TempRepo::new();
    main.commit_file("a.txt", "1\n", "init");
    main.branch("other");
    let dir = tempfile::tempdir().unwrap();
    let wt = dir.path().canonicalize().unwrap().join("wt");
    let output = std::process::Command::new("git")
        .args(["worktree", "add", "--detach"])
        .arg(&wt)
        .current_dir(main.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    let main_id = identity(&main);
    let wt_id = bytegit::Repo::discover_workdir(&wt)
        .unwrap()
        .identity()
        .unwrap();
    let backend = TestBackend::new(main_id.clone());
    backend.add_identity(&wt_id);

    let (runtime, mut events) = GitRuntime::start_with(config(), backend.clone(), TestClock::new());
    let t1 = runtime.subscribe(1, main.path(), true).await.unwrap();
    let t2 = runtime.subscribe(2, &wt, true).await.unwrap();
    let _ = next_snapshot(&mut events, 1).await;
    let _ = next_snapshot(&mut events, 2).await;
    assert_ne!(t1.workspace_key, t2.workspace_key);
    assert_eq!(main_id.common_dir(), wt_id.common_dir());
    assert!(backend.query_calls.load(Ordering::SeqCst) >= 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn manual_refreshes_merge_into_one_replacement_query() {
    let repo = TempRepo::new();
    let blocker = Blocker::closed();
    let backend = TestBackend::with_blocker(identity(&repo), blocker.clone());
    backend.add_identity(&identity(&repo));
    let (runtime, mut events) = GitRuntime::start_with(config(), backend.clone(), TestClock::new());
    let token = runtime.subscribe(1, repo.path(), true).await.unwrap();
    wait_until(|| backend.in_flight.load(Ordering::SeqCst) == 1).await;

    let before = backend.query_calls.load(Ordering::SeqCst);
    runtime.refresh_manual(&token);
    runtime.refresh_manual(&token);
    runtime.refresh(&token, RefreshReason::Workdir);
    blocker.release();

    wait_until(|| backend.query_calls.load(Ordering::SeqCst) > before).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        backend.query_calls.load(Ordering::SeqCst),
        before + 1,
        "飞行中的旧结果 + 多次手动刷新只应产生一次补跑"
    );
    let _ = next_snapshot(&mut events, 1).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tokens_expose_the_project_and_epoch() {
    let repo = TempRepo::new();
    let backend = TestBackend::new(identity(&repo));
    let (runtime, _events) = GitRuntime::start_with(config(), backend, TestClock::new());
    let token: SubscriptionToken = runtime.subscribe(42, repo.path(), true).await.unwrap();
    assert_eq!(token.project_id, 42);
    assert_eq!(token.activation_epoch, 0);
    assert_eq!(
        token.workspace_key.workdir_root(),
        identity(&repo).workdir_root().unwrap()
    );
}

// 执行中退订重开:查询身份和资源生命周期必须独立于订阅。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unsubscribe_must_not_release_a_running_query_permit() {
    let repo = TempRepo::new();
    let blocker = Blocker::closed();
    let backend = TestBackend::with_blocker(identity(&repo), blocker.clone());
    let (runtime, _events) = GitRuntime::start_with(
        config().with_max_concurrent_queries(1),
        backend.clone(),
        TestClock::new(),
    );
    let first = runtime.subscribe(1, repo.path(), true).await.unwrap();
    wait_until(|| backend.query_calls.load(Ordering::SeqCst) == 1).await;
    runtime.unsubscribe(&first);
    let other = TempRepo::new();
    backend.add_identity(&identity(&other));
    let _second = runtime.subscribe(1, other.path(), true).await.unwrap();
    // Wait at most 200ms for the incorrectly admitted second task. Always release
    // backend work before asserting, so tokio shutdown never hangs on the probe.
    for _ in 0..20 {
        if backend.query_calls.load(Ordering::SeqCst) > 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let observed = backend.max_in_flight.load(Ordering::SeqCst);
    blocker.release();
    runtime.shutdown();
    assert!(
        observed <= 1,
        "global cap=1, observed actual concurrent queries={observed}"
    );
}

struct ReopenBackend {
    id: RepoIdentity,
    calls: AtomicUsize,
    gates: [Arc<Blocker>; 2],
}
impl QueryBackend for ReopenBackend {
    fn discover(&self, _: &Path) -> Result<RepoIdentity, QueryFailure> {
        Ok(self.id.clone())
    }
    fn query(&self, request: &QueryRequest) -> Result<QueryOutput, QueryFailure> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let mut output = default_output(&self.id, request);
        output.head = Some(Ok(HeadData {
            branch: Some(if n == 0 { "old" } else { "new" }.into()),
            commit: None,
        }));
        if n < 2 {
            self.gates[n].wait();
        }
        Ok(output)
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn old_query_cannot_be_retagged_as_reopened_subscription() {
    let repo = TempRepo::new();
    let backend = Arc::new(ReopenBackend {
        id: identity(&repo),
        calls: AtomicUsize::new(0),
        gates: [Blocker::closed(), Blocker::closed()],
    });
    let (runtime, mut events) = GitRuntime::start_with(config(), backend.clone(), TestClock::new());
    let old = runtime.subscribe(1, repo.path(), true).await.unwrap();
    wait_until(|| backend.calls.load(Ordering::SeqCst) == 1).await;
    runtime.unsubscribe(&old);
    let current = runtime.subscribe(1, repo.path(), true).await.unwrap();
    for _ in 0..20 {
        if backend.calls.load(Ordering::SeqCst) >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    backend.gates[0].release();
    let result = tokio::time::timeout(Duration::from_millis(200), events.next()).await;
    backend.gates[1].release();
    let current_snapshot = next_snapshot(&mut events, 1).await;
    runtime.shutdown();
    assert_eq!(
        current_snapshot
            .head
            .value()
            .and_then(|h| h.branch.as_deref()),
        Some("new")
    );
    if let Ok(Some(RuntimeEvent::Snapshot { token, snapshot })) = result {
        assert_ne!(
            snapshot.head.last().and_then(|h| h.branch.as_deref()),
            Some("old"),
            "old result was published with current token: event_epoch={}, current_epoch={}",
            token.activation_epoch,
            current.activation_epoch
        );
    }
}
