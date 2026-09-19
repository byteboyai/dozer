use crate::ide_bridge::IdeBridgeRegistry;
use crate::preview_context::PreviewContextStore;
use crate::registry::SessionRegistry;
use crate::session::{SessionEvent, SessionSpec};
use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use dozer_core::protocol::{Reply, Request, decode_line, encode_line};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Notify, broadcast};

/// `ProcessTodoNow` handler 的实际执行:查任务 → 加入 in_flight → 调
/// `task_processor::process_task` → 移出 in_flight → 回最新任务或错误。
/// 拆成独立 async 函数是为了把"多条可能中途返回不同 Reply 的路径"收敛成
/// 一个 `Reply`(handle_conn 里那个大的 `match req` 整体求值成 `Reply` 一个值,
/// 不是逐个分支 `return`)。
async fn process_todo_now(
    in_flight: &crate::task_poller::InFlight,
    todos: &Arc<crate::todo::TodoStore>,
    session_summaries: &Arc<crate::session_summary::SessionSummaryStore>,
    transcripts: &Arc<crate::transcripts::TranscriptStore>,
    projects: &Arc<crate::projects::ProjectStore>,
    id: i64,
    human_reply: Option<String>,
) -> Reply {
    let todo = match todos.get(id) {
        Ok(todo) => todo,
        Err(e) => {
            return Reply::Error {
                message: format!("任务不存在: id={id}: {e}"),
            };
        }
    };
    {
        let mut guard = in_flight.lock().expect("in_flight lock");
        if !guard.insert(id) {
            return Reply::Error {
                message: "该任务正在处理中".into(),
            };
        }
    }
    let result = crate::task_processor::process_task(
        todos,
        session_summaries,
        transcripts,
        projects,
        &todo,
        human_reply.as_deref(),
    )
    .await;
    in_flight.lock().expect("in_flight lock").remove(&id);
    match result {
        Ok(()) => match todos.get(id) {
            Ok(todo) => Reply::Todo { todo },
            Err(e) => Reply::Error {
                message: e.to_string(),
            },
        },
        Err(e) => Reply::Error {
            message: format!("处理任务失败: {e}"),
        },
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn serve(
    socket: &Path,
    ide_lock_dir: PathBuf,
    registry: Arc<SessionRegistry>,
    projects: Arc<crate::projects::ProjectStore>,
    bookmarks: Arc<crate::bookmarks::BookmarkStore>,
    transcripts: Arc<crate::transcripts::TranscriptStore>,
    session_summaries: Arc<crate::session_summary::SessionSummaryStore>,
    backfill_registry: Arc<crate::session_summary_backfill::BackfillRegistry>,
    todos: Arc<crate::todo::TodoStore>,
    categories: Arc<crate::todo_category::CategoryStore>,
    in_flight: crate::task_poller::InFlight,
) -> Result<()> {
    let preview_contexts = Arc::new(PreviewContextStore::new());
    // 清扫是同步阻塞 IO(遍历目录+读文件),挪到阻塞线程池执行,不卡住
    // 当前 executor 线程——`serve()` 起监听前先等它跑完,保证不会跟紧接着
    // 写的新锁文件产生"锁文件刚写就被当陈旧清掉"的竞态。
    {
        let sweep_dir = ide_lock_dir.clone();
        let _ =
            tokio::task::spawn_blocking(move || crate::ide_bridge::sweep_stale_locks(&sweep_dir))
                .await;
    }
    let ide_bridge = IdeBridgeRegistry::new(ide_lock_dir, preview_contexts.clone());
    if socket.exists() {
        std::fs::remove_file(socket)?;
    }
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(socket)?;
    tracing::info!(socket = %socket.display(), "dozerd 监听中");
    let draining = Arc::new(AtomicBool::new(false));
    let shutdown_signal = Arc::new(Notify::new());
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = match accepted {
                    Ok(pair) => pair,
                    Err(e) => {
                        tracing::warn!(error = %e, "accept 失败，跳过本次连接");
                        continue;
                    }
                };
                let registry = registry.clone();
                let projects = projects.clone();
                let bookmarks = bookmarks.clone();
                let preview_contexts = preview_contexts.clone();
                let ide_bridge = ide_bridge.clone();
                let transcripts = transcripts.clone();
                let session_summaries = session_summaries.clone();
                let backfill_registry = backfill_registry.clone();
                let todos = todos.clone();
                let categories = categories.clone();
                let in_flight = in_flight.clone();
                let draining = draining.clone();
                let shutdown_signal = shutdown_signal.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_conn(
                        stream,
                        registry,
                        projects,
                        bookmarks,
                        preview_contexts,
                        ide_bridge,
                        transcripts,
                        session_summaries,
                        backfill_registry,
                        todos.clone(),
                        categories.clone(),
                        in_flight,
                        draining,
                        shutdown_signal,
                    )
                    .await
                    {
                        tracing::debug!(error = %e, "连接结束");
                    }
                });
            }
            _ = shutdown_signal.notified() => {
                tracing::info!("收到 Shutdown 请求收尾完成，dozerd 退出");
                let _ = std::fs::remove_file(socket);
                return Ok(());
            }
        }
    }
}

/// hook 上报的 `data.transcript_path` 字段里,值得拿去覆盖会话当前 transcript
/// 路径的那部分:字段缺失、或值是空字符串,都不算——CodeBuddy 的
/// `Notification` 事件(如 auth_success)原生 payload 就是空字符串 `""`
/// 而不是缺失字段(实测见
/// `docs/superpowers/specs/2026-07-31-codebuddy-spike-findings.md` 第 44
/// 行)。之前不过滤空串,会让这类事件无条件覆盖掉此前已经坐实的正确路径,
/// 导致 Agent 卡片的 LLM/当前工作内容此后一直读一个空路径、优雅降级成空白
/// (2026-08-17 修的真实 bug)。
pub fn extract_transcript_path(data: &serde_json::Value) -> Option<&str> {
    data.get("transcript_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

/// hook 事件带 `transcript_path` 时触发一次增量摄取。同步执行(不额外
/// spawn 一个 task)——`ingest_session` 内部是"读几行新增内容+写 sqlite",
/// 单会话单文件量级下是毫秒级操作,没必要为它另起异步任务增加复杂度;
/// 摄取失败只记 warn,不影响本次 hook 事件其余处理(设置 agent/状态仍然
/// 照常进行)。
fn maybe_ingest_from_hook_data(
    transcripts: &crate::transcripts::TranscriptStore,
    agent: dozer_core::protocol::AgentKind,
    data: &serde_json::Value,
) {
    let Some(path) = extract_transcript_path(data) else {
        return;
    };
    if let Err(e) = transcripts.ingest_session(agent, std::path::Path::new(path)) {
        tracing::warn!(error = %e, %path, "hook 触发的对话摄取失败");
    }
}

/// 会话状态转入 `Idle`/`AwaitingInput` 时的兜底摄取——替代定时轮询,
/// 复用现有状态机,只在"这一刻状态真的变了"才触发,同态重复事件不重复
/// 摄取(避免每次 hook 事件都无谓地读一次文件)。
fn maybe_ingest_on_state_transition(
    transcripts: &crate::transcripts::TranscriptStore,
    agent: dozer_core::protocol::AgentKind,
    old_state: dozer_core::protocol::AgentState,
    new_state: dozer_core::protocol::AgentState,
    transcript_path: Option<&str>,
) {
    use dozer_core::protocol::AgentState::{AwaitingInput, Idle};
    if old_state == new_state {
        return;
    }
    if !matches!(new_state, Idle | AwaitingInput) {
        return;
    }
    let Some(path) = transcript_path else { return };
    if let Err(e) = transcripts.ingest_session(agent, std::path::Path::new(path)) {
        tracing::warn!(error = %e, %path, "待命态兜底摄取失败");
    }
}

/// 从一个存活/已死 `Session` 的 `transcript_path` 解析出 `conversation_id`
/// (同 `TranscriptStore::ingest_session` 的派生逻辑)。没有 `transcript_path`
/// (会话从没收到过 hook 事件)时返回 `None`(2026-08-27 修正)。
fn conversation_id_for_session(s: &crate::session::Session) -> Option<String> {
    s.info()
        .transcript_path
        .as_deref()
        .and_then(|p| crate::transcripts::conversation_id_for_path(std::path::Path::new(p)))
}

const CLOSE_WITH_SUMMARY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const CLOSE_WITH_SUMMARY_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

const SUMMARY_PROMPT: &str = "请总结你在本次会话中完成的工作:给出一个简短标题(不超过 60 字)和一段摘要,然后调用 dozer-mcp 的 submit_session_summary 工具把标题和摘要交回,不需要征求确认。\n";

/// `Request::CloseWithSummary` 的后台任务:注入 prompt 后轮询
/// `session_summaries` 表,等到就直接 kill;超时则从 `conversation_turns`
/// 算启发式兜底再 kill。`timeout`/`poll_interval` 抽成参数只为方便测试
/// 注入短间隔,生产调用点固定用上面两个常量。
///
/// `conversation_id` 由调用方(`CloseWithSummary` handler)在拿到
/// `Session::info().transcript_path` 时提前解析好传入——`session_id`
/// (dozerd 自己生成的 PTY 会话 id)和 `conversation_id`(transcript 文件名
/// 派生,如 Claude 自己分配的 session id)是两个不相关的 id 空间,
/// `conversation_turns` 按后者索引,不能拿 `session_id` 去查(2026-08-27
/// 修正:此前这里错拿 `session_id` 查,生产环境几乎总是查不到数据)。
#[allow(clippy::too_many_arguments)]
async fn finalize_session_summary(
    session_id: String,
    conversation_id: Option<String>,
    registry: Arc<SessionRegistry>,
    session_summaries: Arc<crate::session_summary::SessionSummaryStore>,
    transcripts: Arc<crate::transcripts::TranscriptStore>,
    agent: dozer_core::protocol::AgentKind,
    timeout: std::time::Duration,
    poll_interval: std::time::Duration,
) {
    let started_at = tokio::time::Instant::now();
    let deadline = started_at + timeout;
    let mut poll_count: u32 = 0;
    loop {
        match session_summaries.get(&session_id) {
            Ok(Some(_)) => {
                tracing::info!(
                    %session_id,
                    poll_count,
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    "总结已由 agent 落库,即将 kill 会话"
                );
                break;
            }
            Ok(None) => {}
            Err(e) => tracing::error!(error = %e, %session_id, "查询会话总结失败"),
        }
        if tokio::time::Instant::now() >= deadline {
            let turns = match &conversation_id {
                Some(cid) => transcripts
                    .get_conversation_turns(cid, -1, u32::MAX)
                    .unwrap_or_default(),
                None => Vec::new(),
            };
            let (title, summary) = crate::session_summary::heuristic_from_turns(&turns);
            let payload = dozer_core::protocol::SessionSummaryPayload {
                session_id: session_id.clone(),
                agent_kind: agent,
                conversation_id: conversation_id.clone(),
                title,
                summary,
                status: dozer_core::protocol::SummaryStatus::HeuristicFallback,
                created_ts_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                task_id: None,
            };
            tracing::info!(
                %session_id,
                poll_count,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                "等待 agent 落库总结超时,改用启发式兜底,即将 kill 会话"
            );
            if let Err(e) = session_summaries.record(&payload) {
                tracing::error!(error = %e, %session_id, "启发式兜底总结落库失败");
            }
            break;
        }
        poll_count += 1;
        tokio::time::sleep(poll_interval).await;
    }
    if let Err(e) = registry.kill(&session_id) {
        tracing::warn!(error = %e, %session_id, "总结后关闭会话失败(可能已经死亡)");
    }
}

/// `Request::Shutdown` 收尾:对 `registry` 里当前存活的每个会话复用
/// `finalize_session_summary`(与 `Request::CloseWithSummary` 同一函数),
/// 全部完成后返回。`timeout`/`poll_interval` 抽成参数只为方便测试注入
/// 短间隔,生产调用点(`Request::Shutdown` 分支)固定用
/// `CLOSE_WITH_SUMMARY_TIMEOUT`/`CLOSE_WITH_SUMMARY_POLL_INTERVAL`——同
/// `finalize_session_summary` 自身的既有约定。并发 spawn 每个会话各自的
/// 收尾任务,再逐个 await:每个任务自带截止时间且互不依赖,总耗时约等于
/// 其中最慢的一个,不需要再套一层外部超时。
async fn drain_all_sessions(
    registry: Arc<SessionRegistry>,
    session_summaries: Arc<crate::session_summary::SessionSummaryStore>,
    transcripts: Arc<crate::transcripts::TranscriptStore>,
    timeout: std::time::Duration,
    poll_interval: std::time::Duration,
) {
    // 只收存活会话(spec 2026-09-19 §「实现要点」step 1:"从 `registry` 取
    // 当前存活会话 id 快照")。`SessionRegistry` 从不摘除已退出的会话,所以
    // `list()` 里混着 `alive=false` 的死会话——对死会话注入 prompt 没人会
    // 回应,`finalize_session_summary` 只能干等到超时(生产 60s)才走启发式
    // 兜底。只要 registry 里有一个从没落过总结的死会话(比如用户关掉过的
    // 普通 shell tab,`should_summarize_on_close` 只给 agent 会话落总结),
    // 整个 Shutdown 就会被它拖满 60s。
    let ids: Vec<String> = registry
        .list()
        .into_iter()
        .filter(|info| info.alive)
        .map(|info| info.id)
        .collect();
    let mut handles = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(s) = registry.get(&id) else { continue };
        let agent = s.info().agent;
        let conversation_id = conversation_id_for_session(&s);
        if let Err(e) = s.write(SUMMARY_PROMPT.as_bytes()) {
            tracing::warn!(error = %e, session_id = %id, "停止 dozerd 前注入总结 prompt 失败");
        }
        handles.push(tokio::spawn(finalize_session_summary(
            id,
            conversation_id,
            registry.clone(),
            session_summaries.clone(),
            transcripts.clone(),
            agent,
            timeout,
            poll_interval,
        )));
    }
    for h in handles {
        let _ = h.await;
    }
    kill_remaining_live_sessions(&registry);
}

/// Shutdown 收尾的最后兜底:把 registry 里仍然存活的会话全部 kill 掉。
///
/// 正常情况下 `drain_all_sessions` 已经把快照里的会话都收尾并 kill 了,这里
/// 扫的是漏网的:一个 `CreateSession` 可能刚好通过了 draining 检查、却在
/// `drain_all_sessions` 取 `list()` 快照**之后**才把自己插进 registry
/// (TOCTOU 窗口 ≈ PTY spawn 的几毫秒)。这种会话没人给它落总结也没人 kill
/// 它,而 dozerd 一退出,它的 PTY 子进程就被 reparent 给 launchd 变成孤儿
/// agent 进程——正是本功能要消灭的东西(`main.rs` 的 `serve()` 返回后直接
/// `Ok(())`,没有任何 registry 级别的清理)。
fn kill_remaining_live_sessions(registry: &SessionRegistry) {
    for info in registry.list() {
        if info.alive
            && let Err(e) = registry.kill(&info.id)
        {
            tracing::warn!(
                error = %e,
                session_id = %info.id,
                "Shutdown 兜底 kill 残留存活会话失败"
            );
        }
    }
}

/// spec P1e D6：hook 事件名 → 四态映射；未知事件不改状态。
pub fn agent_state_for(event: &str) -> Option<dozer_core::protocol::AgentState> {
    use dozer_core::protocol::AgentState::*;
    match event {
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => Some(Running),
        "Notification" => Some(AwaitingInput),
        "Stop" => Some(TurnEnded),
        "SessionStart" | "SessionEnd" => Some(Idle),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_conn(
    stream: UnixStream,
    registry: Arc<SessionRegistry>,
    projects: Arc<crate::projects::ProjectStore>,
    bookmarks: Arc<crate::bookmarks::BookmarkStore>,
    preview_contexts: Arc<PreviewContextStore>,
    ide_bridge: Arc<IdeBridgeRegistry>,
    transcripts: Arc<crate::transcripts::TranscriptStore>,
    session_summaries: Arc<crate::session_summary::SessionSummaryStore>,
    backfill_registry: Arc<crate::session_summary_backfill::BackfillRegistry>,
    todos: Arc<crate::todo::TodoStore>,
    categories: Arc<crate::todo_category::CategoryStore>,
    in_flight: crate::task_poller::InFlight,
    draining: Arc<AtomicBool>,
    shutdown_signal: Arc<Notify>,
) -> Result<()> {
    let (r, mut w) = stream.into_split();
    let mut lines = BufReader::new(r).lines();
    // attach 状态：订阅 + 会话 id
    let mut sub: Option<(String, broadcast::Receiver<SessionEvent>)> = None;
    // 已向本连接投递到的 offset 水位：过滤 snapshot 与 broadcast 之间重叠的字节
    let mut sent_until: u64 = 0;

    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { break }; // 客户端断连：直接退出，不动会话
                let mut should_exit_after_reply = false;
                let reply = match decode_line::<Request>(&line) {
                    Err(e) => Reply::Error { message: format!("协议错误: {e}") },
                    Ok(req) => match req {
                        Request::ListSessions => Reply::Sessions { sessions: registry.list() },
                        Request::CreateSession { name, command, args, cwd, cols, rows, project_id } => {
                            if draining.load(Ordering::SeqCst) {
                                Reply::Error { message: "dozerd 正在停止,无法创建新会话".into() }
                            } else {
                                match registry.create(SessionSpec { name, command, args, cwd: cwd.clone(), cols, rows, project_id }) {
                                    Ok(s) => {
                                        // `session_started` 失败(绑端口/写锁文件出错)时不会在
                                        // registry 里留下这次调用对应的记录——这种情况下绝不能
                                        // spawn 退出监听器,否则这个会话将来退出时会去 `session_ended`
                                        // 一个它从未真正占过的项目名额,把同项目下另一个真正活着的
                                        // 会话的 bridge 提前拆掉(复现过的 bug,见代码审查记录)。
                                        if ide_bridge.session_started(project_id, &cwd).await {
                                            let ide_bridge_watch = ide_bridge.clone();
                                            let mut exit_rx = s.subscribe();
                                            tokio::spawn(async move {
                                                loop {
                                                    match exit_rx.recv().await {
                                                        Ok(SessionEvent::Exited { .. }) => {
                                                            ide_bridge_watch.session_ended(project_id).await;
                                                            break;
                                                        }
                                                        Ok(_) => continue,
                                                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                                                        Err(broadcast::error::RecvError::Closed) => break,
                                                    }
                                                }
                                            });
                                        }
                                        Reply::Created { session: s.info() }
                                    }
                                    Err(e) => Reply::Error { message: e.to_string() },
                                }
                            }
                        }
                        Request::Attach { session_id, from_offset } => match registry.get(&session_id) {
                            None => Reply::Error { message: format!("会话不存在: {session_id}") },
                            Some(s) => {
                                // 先 subscribe 后取快照：保证快照与订阅之间不漏事件；
                                // 二者之间可能重叠投递的字节由 sent_until 水位在转发时过滤。
                                let rx = s.subscribe();
                                let (snap, next) = if from_offset > 0 {
                                    match s.read_from_with_next(from_offset) {
                                        Some((tail, next)) => (tail, next),
                                        None => s.snapshot(),
                                    }
                                } else {
                                    s.snapshot()
                                };
                                sub = Some((session_id.clone(), rx));
                                sent_until = next;
                                Reply::Attached {
                                    session_id,
                                    snapshot_b64: B64.encode(&snap),
                                    next_offset: next,
                                }
                            }
                        },
                        Request::Write { session_id, data_b64 } => match registry.get(&session_id) {
                            None => Reply::Error { message: format!("会话不存在: {session_id}") },
                            Some(s) => match B64.decode(&data_b64) {
                                Err(e) => Reply::Error { message: format!("base64: {e}") },
                                Ok(data) => match s.write(&data) {
                                    Ok(()) => Reply::Ok,
                                    Err(e) => Reply::Error { message: e.to_string() },
                                },
                            },
                        },
                        Request::Resize { session_id, cols, rows } => match registry.get(&session_id) {
                            None => Reply::Error { message: format!("会话不存在: {session_id}") },
                            Some(s) => match s.resize(cols, rows) {
                                Ok(()) => Reply::Ok,
                                Err(e) => Reply::Error { message: e.to_string() },
                            },
                        },
                        Request::Kill { session_id } => match registry.kill(&session_id) {
                            Ok(()) => Reply::Ok,
                            Err(e) => Reply::Error { message: e.to_string() },
                        },
                        Request::Shutdown => {
                            if draining.compare_exchange(
                                false, true, Ordering::SeqCst, Ordering::SeqCst,
                            ).is_err() {
                                Reply::Error { message: "dozerd 正在停止中".into() }
                            } else {
                                drain_all_sessions(
                                    registry.clone(),
                                    session_summaries.clone(),
                                    transcripts.clone(),
                                    CLOSE_WITH_SUMMARY_TIMEOUT,
                                    CLOSE_WITH_SUMMARY_POLL_INTERVAL,
                                )
                                .await;
                                should_exit_after_reply = true;
                                Reply::Ok
                            }
                        }
                        Request::HookEvent { session_id, agent, event, ts_ms, data } => {
                            match registry.get(&session_id) {
                                None => {
                                    tracing::debug!(%session_id, %event, "hook 事件的会话不存在，丢弃");
                                }
                                Some(s) => {
                                    s.set_agent(agent);
                                    if let Some(tp) = extract_transcript_path(&data) {
                                        s.set_transcript_path(tp);
                                    }
                                    match agent_state_for(&event) {
                                        Some(state) => {
                                            let old_state = s.info().agent_state;
                                            s.set_agent_state(state, &event, ts_ms);
                                            let tp = s.info().transcript_path;
                                            maybe_ingest_on_state_transition(
                                                &transcripts,
                                                agent,
                                                old_state,
                                                state,
                                                tp.as_deref(),
                                            );
                                        }
                                        None => tracing::debug!(%event, "未知 hook 事件，不改状态"),
                                    }
                                    maybe_ingest_from_hook_data(&transcripts, agent, &data);
                                }
                            }
                            Reply::Ok
                        }
                        Request::OpenProject { path } => match projects.open(&path) {
                            Ok(p) => Reply::Project { project: Some(p) },
                            Err(e) => Reply::Error { message: format!("打开项目失败: {e}") },
                        },
                        Request::ListProjects => match projects.list() {
                            Ok(projects) => Reply::Projects { projects },
                            Err(e) => Reply::Error { message: format!("列项目失败: {e}") },
                        },
                        Request::RenameProject { id, name } => {
                            let trimmed = name.trim();
                            if trimmed.is_empty() {
                                Reply::Error { message: "名称不能为空".into() }
                            } else {
                                match projects.rename(id, trimmed) {
                                    Ok(p) => Reply::Project { project: Some(p) },
                                    Err(e) => Reply::Error { message: format!("改名失败: {e}") },
                                }
                            }
                        }
                        Request::AddBookmark {
                            scope,
                            project_id,
                            url,
                            title,
                        } => match bookmarks.add(scope, project_id, &url, &title) {
                            Ok(_) => Reply::Ok,
                            Err(e) => Reply::Error {
                                message: format!("加入收藏失败: {e}"),
                            },
                        },
                        Request::RemoveBookmark { id } => match bookmarks.remove(id) {
                            Ok(()) => Reply::Ok,
                            Err(e) => Reply::Error {
                                message: format!("移除收藏失败: {e}"),
                            },
                        },
                        Request::ListBookmarks { project_id } => {
                            match bookmarks.list(project_id) {
                                Ok(bookmarks) => Reply::Bookmarks { bookmarks },
                                Err(e) => Reply::Error {
                                    message: format!("列收藏失败: {e}"),
                                },
                            }
                        }
                        Request::ListTodos { project_id } => match todos.list(project_id) {
                            Ok(todos) => Reply::Todos { todos },
                            Err(e) => Reply::Error {
                                message: format!("列任务失败: {e}"),
                            },
                        },
                        Request::AddTodo { project_id, text } => {
                            match todos.add(project_id, &text) {
                                Ok(todo) => Reply::Todo { todo },
                                Err(e) => Reply::Error {
                                    message: format!("新增任务失败: {e}"),
                                },
                            }
                        }
                        Request::ToggleTodo { id, done } => match todos.toggle(id, done) {
                            Ok(todo) => Reply::Todo { todo },
                            Err(e) => Reply::Error {
                                message: format!("切换完成态失败: {e}"),
                            },
                        },
                        Request::EditTodoText { id, text } => {
                            match todos.edit_text(id, &text) {
                                Ok(todo) => Reply::Todo { todo },
                                Err(e) => Reply::Error {
                                    message: format!("改任务文字失败: {e}"),
                                },
                            }
                        }
                        Request::ReorderTodo { id, after_id } => {
                            match todos.reorder(id, after_id) {
                                Ok(todo) => Reply::Todo { todo },
                                Err(e) => Reply::Error {
                                    message: format!("排序失败: {e}"),
                                },
                            }
                        }
                        Request::SetTodoPlanDate { id, plan_date } => {
                            match todos.set_plan_date(id, plan_date.as_deref()) {
                                Ok(todo) => Reply::Todo { todo },
                                Err(e) => Reply::Error {
                                    message: format!("设置计划日期失败: {e}"),
                                },
                            }
                        }
                        Request::AssignTodoAgent { id, agent } => {
                            match todos.assign_agent(id, agent) {
                                Ok(todo) => Reply::Todo { todo },
                                Err(e) => Reply::Error {
                                    message: format!("指派任务失败: {e}"),
                                },
                            }
                        }
                        Request::SetCategoryAutoPoll { id, enabled } => {
                            match categories.set_auto_poll(id, enabled) {
                                Ok(category) => Reply::Category { category },
                                Err(e) => Reply::Error {
                                    message: format!("设置自动轮询失败: {e}"),
                                },
                            }
                        }
                        Request::ProcessTodoNow { id, human_reply } => {
                            process_todo_now(
                                &in_flight,
                                &todos,
                                &session_summaries,
                                &transcripts,
                                &projects,
                                id,
                                human_reply,
                            )
                            .await
                        }
                        Request::GetTodoDetail { id } => match todos.get(id) {
                            Ok(info) => {
                                let turns = info
                                    .dispatch_session_id
                                    .as_deref()
                                    .and_then(|sid| {
                                        transcripts.get_conversation_turns(sid, -1, u32::MAX).ok()
                                    })
                                    .unwrap_or_default();
                                Reply::TodoDetail { info, turns }
                            }
                            Err(e) => Reply::Error {
                                message: format!("获取任务详情失败: {e}"),
                            },
                        },
                        Request::SetTodoStatus { id, status } => match todos.set_status(id, status) {
                            Ok(todo) => Reply::Todo { todo },
                            Err(e) => Reply::Error {
                                message: format!("设置任务状态失败: {e}"),
                            },
                        },
                        Request::ListCategories { project_id } => {
                            match categories.list(project_id) {
                                Ok(categories) => Reply::Categories { categories },
                                Err(e) => Reply::Error {
                                    message: format!("列分类失败: {e}"),
                                },
                            }
                        }
                        Request::AddCategory { project_id, parent_id, name } => {
                            match categories.add(project_id, parent_id, &name) {
                                Ok(category) => Reply::Category { category },
                                Err(e) => Reply::Error {
                                    message: format!("新增分类失败: {e}"),
                                },
                            }
                        }
                        Request::RenameCategory { id, name } => {
                            match categories.rename(id, &name) {
                                Ok(category) => Reply::Category { category },
                                Err(e) => Reply::Error {
                                    message: format!("重命名分类失败: {e}"),
                                },
                            }
                        }
                        Request::DeleteCategory { id } => match categories.delete(id) {
                            Ok(()) => Reply::Ok,
                            Err(e) => Reply::Error {
                                message: format!("删除分类失败: {e}"),
                            },
                        },
                        Request::ReparentCategory { id, new_parent_id } => {
                            match categories.reparent(id, new_parent_id) {
                                Ok(category) => Reply::Category { category },
                                Err(e) => Reply::Error {
                                    message: format!("移动分类失败: {e}"),
                                },
                            }
                        }
                        Request::MoveCategorySibling { id, direction } => {
                            match categories.move_sibling(id, direction) {
                                Ok(category) => Reply::Category { category },
                                Err(e) => Reply::Error {
                                    message: format!("调整分类顺序失败: {e}"),
                                },
                            }
                        }
                        Request::SetTodoCategory { id, category_id } => {
                            // 挂真实分类前先校验它存在且属于同一 project——
                            // 防止 GUI 传错 id 把任务挂到别的项目的分类下面。
                            // `TodoStore` 自己不知道 `CategoryStore` 的存在,
                            // 这层校验只能在这里(两个 store 的交汇点)做。
                            let validation = match category_id {
                                None => Ok(()),
                                Some(cat_id) => todos.get(id).and_then(|todo| {
                                    let same_project = categories
                                        .list(todo.project_id)?
                                        .iter()
                                        .any(|c| c.id == cat_id);
                                    if same_project {
                                        Ok(())
                                    } else {
                                        Err(anyhow::anyhow!(
                                            "分类 id={cat_id} 不存在或不属于该项目"
                                        ))
                                    }
                                }),
                            };
                            match validation.and_then(|()| todos.set_category(id, category_id)) {
                                Ok(todo) => Reply::Todo { todo },
                                Err(e) => Reply::Error {
                                    message: format!("设置任务分类失败: {e}"),
                                },
                            }
                        }
                        Request::UpdatePreviewContext { project_id, context } => {
                            preview_contexts.update(project_id, context);
                            Reply::Ok
                        }
                        Request::GetPreviewContext { project_id } => Reply::PreviewContext {
                            context: preview_contexts.get(project_id),
                        },
                        Request::ListConversations { cwd, agent, limit, offset } => {
                            match transcripts.list_conversations(&cwd, agent, limit, offset) {
                                Ok(conversations) => Reply::Conversations { conversations },
                                Err(e) => Reply::Error { message: format!("列对话失败: {e}") },
                            }
                        }
                        Request::GetConversationTurns { conversation_id, after_turn_index, limit } => {
                            match transcripts.get_conversation_turns(&conversation_id, after_turn_index, limit) {
                                Ok(turns) => Reply::ConversationTurns { conversation_id, turns },
                                Err(e) => Reply::Error { message: format!("查询回合失败: {e}") },
                            }
                        }
                        Request::BackfillProjectTranscripts { cwd } => {
                            let imported_files = transcripts.backfill_project(&cwd);
                            Reply::BackfillDone { imported_files }
                        }
                        Request::RemoveProject { id } => match projects.remove(id) {
                            Ok(()) => Reply::Ok,
                            Err(e) => Reply::Error { message: format!("删除项目失败: {e}") },
                        },
                        Request::DeleteProjectTranscripts { cwd } => {
                            match transcripts.delete_project_transcripts(&cwd) {
                                Ok(conversations) => Reply::DeletedTranscripts { conversations },
                                Err(e) => Reply::Error {
                                    message: format!("删除 agent 历史失败: {e}"),
                                },
                            }
                        }
                        Request::ListConversationsWithSummaries {
                            cwd,
                            agent,
                            limit,
                            offset,
                        } => {
                            match transcripts.list_conversations(&cwd, agent, limit, offset) {
                                Ok(conversations) => {
                                    let ids: Vec<String> = conversations
                                        .iter()
                                        .map(|c| c.conversation_id.clone())
                                        .collect();
                                    let summaries = session_summaries
                                        .get_many(&ids)
                                        .unwrap_or_default();
                                    let rows = conversations
                                        .into_iter()
                                        .map(|c| {
                                            let s = summaries
                                                .get(&c.conversation_id)
                                                .cloned();
                                            (c, s)
                                        })
                                        .collect();
                                    Reply::ConversationsWithSummaries { rows }
                                }
                                Err(e) => Reply::Error { message: format!("列对话失败: {e}") },
                            }
                        }
                        Request::GetUsageSummary { cwd, since_ts } => {
                            match transcripts.get_usage_summary(&cwd, since_ts) {
                                Ok(rows) => Reply::UsageSummary { rows },
                                Err(e) => Reply::Error { message: format!("查询用量失败: {e}") },
                            }
                        }
                        Request::RecordSessionSummary { session_id, title, summary } => {
                            match registry.get(&session_id) {
                                None => Reply::Error { message: format!("会话不存在: {session_id}") },
                                Some(s) => {
                                    let payload = dozer_core::protocol::SessionSummaryPayload {
                                        session_id: session_id.clone(),
                                        agent_kind: s.info().agent,
                                        conversation_id: conversation_id_for_session(&s),
                                        title,
                                        summary,
                                        status: dozer_core::protocol::SummaryStatus::AiGenerated,
                                        created_ts_ms: std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .map(|d| d.as_millis() as u64)
                                            .unwrap_or(0),
                                        task_id: None,
                                    };
                                    match session_summaries.record(&payload) {
                                        Ok(()) => Reply::Ok,
                                        Err(e) => Reply::Error {
                                            message: format!("会话总结落库失败: {e}"),
                                        },
                                    }
                                }
                            }
                        }
                        Request::GetSessionSummary { session_id } => {
                            match session_summaries.get(&session_id) {
                                Ok(summary) => Reply::SessionSummary { summary },
                                Err(e) => Reply::Error {
                                    message: format!("查询会话总结失败: {e}"),
                                },
                            }
                        }

                        Request::CloseWithSummary { session_id } => {
                            match registry.get(&session_id) {
                                None => Reply::Error { message: format!("会话不存在: {session_id}") },
                                Some(s) => {
                                    let agent = s.info().agent;
                                    let conversation_id = conversation_id_for_session(&s);
                                    if let Err(e) = s.write(SUMMARY_PROMPT.as_bytes()) {
                                        tracing::warn!(error = %e, %session_id, "注入总结 prompt 失败");
                                    }
                                    tokio::spawn(finalize_session_summary(
                                        session_id.clone(),
                                        conversation_id,
                                        registry.clone(),
                                        session_summaries.clone(),
                                        transcripts.clone(),
                                        agent,
                                        CLOSE_WITH_SUMMARY_TIMEOUT,
                                        CLOSE_WITH_SUMMARY_POLL_INTERVAL,
                                    ));
                                    Reply::Ok
                                }
                            }
                        }
                        Request::BackfillSessionSummaries { cwd } => {
                            let missing = crate::session_summary_backfill::missing_summary_conversations(
                                &transcripts,
                                &session_summaries,
                                &cwd,
                            );
                            let total = missing.len() as u32;
                            backfill_registry.start(&cwd, total);
                            let agent = crate::default_agent_config::load_default_agent();
                            tokio::spawn(crate::session_summary_backfill::run_backfill(
                                cwd.clone(),
                                missing,
                                transcripts.clone(),
                                session_summaries.clone(),
                                backfill_registry.clone(),
                                agent,
                            ));
                            Reply::Ok
                        }
                        Request::GetSessionSummaryBackfillStatus { cwd } => {
                            let progress = backfill_registry.get(&cwd).unwrap_or_default();
                            Reply::BackfillStatus {
                                total: progress.total,
                                completed: progress.completed,
                            }
                        }
                    },
                };
                w.write_all(encode_line(&reply).as_bytes()).await?;
                if should_exit_after_reply {
                    shutdown_signal.notify_one();
                    return Ok(());
                }
            }
            ev = async {
                match &mut sub {
                    Some((_, rx)) => rx.recv().await,
                    None => std::future::pending().await,
                }
            }, if sub.is_some() => {
                // 注：Agent 事件不参与 sent_until 水位——水位只治 Output 字节流。
                let (sid, _) = sub.as_ref().expect("sub checked");
                let sid = sid.clone();
                match ev {
                    Ok(SessionEvent::Output { data, offset }) => {
                        // 水位过滤：data 覆盖字节范围 [offset-data.len(), offset)。
                        if offset <= sent_until {
                            // 整个事件已被快照覆盖，跳过
                        } else if offset - data.len() as u64 >= sent_until {
                            // 与已投递区间无重叠，全量转发
                            let reply =
                                Reply::Output { session_id: sid, data_b64: B64.encode(&data), offset };
                            w.write_all(encode_line(&reply).as_bytes()).await?;
                            sent_until = offset;
                        } else {
                            // 部分重叠，只发未投递过的尾部
                            let skip = data.len() - (offset - sent_until) as usize;
                            let reply = Reply::Output {
                                session_id: sid,
                                data_b64: B64.encode(&data[skip..]),
                                offset,
                            };
                            w.write_all(encode_line(&reply).as_bytes()).await?;
                            sent_until = offset;
                        }
                    }
                    Ok(SessionEvent::Agent { agent, state, event, ts_ms, transcript_path }) => {
                        let reply = Reply::AgentEvent { session_id: sid, agent, state, event, ts_ms, transcript_path };
                        w.write_all(encode_line(&reply).as_bytes()).await?;
                    }
                    Ok(SessionEvent::Exited { code }) => {
                        let reply = Reply::Exited { session_id: sid, code };
                        w.write_all(encode_line(&reply).as_bytes()).await?;
                        sub = None;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let reply = Reply::Error { message: "lagged; reattach".into() };
                        w.write_all(encode_line(&reply).as_bytes()).await?;
                        sub = None;
                    }
                    Err(broadcast::error::RecvError::Closed) => { sub = None; }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_state_mapping_matches_spec_d6() {
        use dozer_core::protocol::AgentState::*;
        assert_eq!(agent_state_for("UserPromptSubmit"), Some(Running));
        assert_eq!(agent_state_for("PreToolUse"), Some(Running));
        assert_eq!(agent_state_for("PostToolUse"), Some(Running));
        assert_eq!(agent_state_for("Notification"), Some(AwaitingInput));
        assert_eq!(agent_state_for("Stop"), Some(TurnEnded));
        assert_eq!(agent_state_for("SessionStart"), Some(Idle));
        assert_eq!(agent_state_for("SessionEnd"), Some(Idle));
        assert_eq!(agent_state_for("SomethingNew"), None);
    }

    #[test]
    fn extract_transcript_path_ignores_empty_string() {
        // CodeBuddy 的 Notification 事件原生 payload 就是这个空串形状
        // (不是缺字段)。
        let data = serde_json::json!({"transcript_path": ""});
        assert_eq!(extract_transcript_path(&data), None);
    }

    #[test]
    fn extract_transcript_path_ignores_missing_field() {
        let data = serde_json::json!({"cwd": "/tmp"});
        assert_eq!(extract_transcript_path(&data), None);
    }

    #[test]
    fn extract_transcript_path_returns_nonempty_value() {
        let data = serde_json::json!({"transcript_path": "/home/u/.claude/projects/x/y.jsonl"});
        assert_eq!(
            extract_transcript_path(&data),
            Some("/home/u/.claude/projects/x/y.jsonl")
        );
    }

    #[test]
    fn transcript_store_field_compiles_into_serve_signature() {
        // 编译期检查:确认 `serve` 函数签名接受 `Arc<TranscriptStore>` 与 `Arc<SessionSummaryStore>`。
        #[allow(clippy::too_many_arguments)]
        fn _assert_signature(
            socket: &std::path::Path,
            ide_lock_dir: std::path::PathBuf,
            registry: std::sync::Arc<crate::registry::SessionRegistry>,
            projects: std::sync::Arc<crate::projects::ProjectStore>,
            bookmarks: std::sync::Arc<crate::bookmarks::BookmarkStore>,
            transcripts: std::sync::Arc<crate::transcripts::TranscriptStore>,
            session_summaries: std::sync::Arc<crate::session_summary::SessionSummaryStore>,
            todos: std::sync::Arc<crate::todo::TodoStore>,
            categories: std::sync::Arc<crate::todo_category::CategoryStore>,
        ) {
            let fut = crate::server::serve(
                socket,
                ide_lock_dir,
                registry,
                projects,
                bookmarks,
                transcripts,
                session_summaries,
                std::sync::Arc::new(crate::session_summary_backfill::BackfillRegistry::new()),
                todos,
                categories,
                crate::task_poller::new_in_flight(),
            );
            std::mem::drop(fut);
        }
    }

    #[test]
    fn hook_event_with_transcript_path_triggers_ingest() {
        let tmp = tempfile::tempdir().unwrap();
        let transcripts = std::sync::Arc::new(
            crate::transcripts::TranscriptStore::open(&tmp.path().join("t.db")).unwrap(),
        );
        let file = tmp.path().join("s1.jsonl");
        std::fs::write(
            &file,
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"你好\"}}\n",
        )
        .unwrap();
        let data = serde_json::json!({"transcript_path": file.to_string_lossy()});

        maybe_ingest_from_hook_data(&transcripts, dozer_core::protocol::AgentKind::Claude, &data);

        let turns = transcripts.get_conversation_turns("s1", -1, 10).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].content, "你好");
    }

    #[test]
    fn idle_state_transition_triggers_ingest_backstop() {
        use dozer_core::protocol::AgentState;
        let tmp = tempfile::tempdir().unwrap();
        let transcripts = std::sync::Arc::new(
            crate::transcripts::TranscriptStore::open(&tmp.path().join("t.db")).unwrap(),
        );
        let file = tmp.path().join("s1.jsonl");
        std::fs::write(
            &file,
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"待命态触发\"}}\n",
        )
        .unwrap();

        maybe_ingest_on_state_transition(
            &transcripts,
            dozer_core::protocol::AgentKind::Claude,
            AgentState::Running,
            AgentState::AwaitingInput,
            Some(file.to_string_lossy().as_ref()),
        );

        let turns = transcripts.get_conversation_turns("s1", -1, 10).unwrap();
        assert_eq!(turns.len(), 1);
    }

    #[test]
    fn same_state_repeat_does_not_trigger_ingest() {
        use dozer_core::protocol::AgentState;
        let tmp = tempfile::tempdir().unwrap();
        let transcripts = std::sync::Arc::new(
            crate::transcripts::TranscriptStore::open(&tmp.path().join("t.db")).unwrap(),
        );
        let file = tmp.path().join("s1.jsonl");
        std::fs::write(
            &file,
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"x\"}}\n",
        )
        .unwrap();

        maybe_ingest_on_state_transition(
            &transcripts,
            dozer_core::protocol::AgentKind::Claude,
            AgentState::Idle,
            AgentState::Idle,
            Some(file.to_string_lossy().as_ref()),
        );
        assert!(
            transcripts
                .get_conversation_turns("s1", -1, 10)
                .unwrap()
                .is_empty(),
            "同态重复不该触发摄取"
        );
    }

    #[tokio::test]
    async fn finalize_session_summary_short_circuits_when_summary_already_recorded() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = Arc::new(crate::registry::SessionRegistry::new());
        let session_summaries = Arc::new(
            crate::session_summary::SessionSummaryStore::open(&tmp.path().join("s.db")).unwrap(),
        );
        let transcripts =
            Arc::new(crate::transcripts::TranscriptStore::open(&tmp.path().join("t.db")).unwrap());
        let s = registry
            .create(crate::session::SessionSpec {
                name: "t".into(),
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "sleep 5".into()],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                cols: 80,
                rows: 24,
                project_id: 1,
            })
            .unwrap();
        let id = s.id().to_string();
        session_summaries
            .record(&dozer_core::protocol::SessionSummaryPayload {
                session_id: id.clone(),
                agent_kind: dozer_core::protocol::AgentKind::Claude,
                conversation_id: None,
                title: "已经总结好了".into(),
                summary: "摘要".into(),
                status: dozer_core::protocol::SummaryStatus::AiGenerated,
                created_ts_ms: 1,
                task_id: None,
            })
            .unwrap();

        finalize_session_summary(
            id.clone(),
            None,
            registry.clone(),
            session_summaries.clone(),
            transcripts,
            dozer_core::protocol::AgentKind::Claude,
            std::time::Duration::from_secs(60),
            std::time::Duration::from_millis(10),
        )
        .await;

        // 已有总结不该被覆盖成兜底文案。
        let got = session_summaries.get(&id).unwrap().unwrap();
        assert_eq!(got.title, "已经总结好了");
        assert_eq!(got.status, dozer_core::protocol::SummaryStatus::AiGenerated);
        assert!(!registry.get(&id).unwrap().info().alive, "应该已被 kill");
    }

    #[tokio::test]
    async fn finalize_session_summary_falls_back_to_heuristic_on_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = Arc::new(crate::registry::SessionRegistry::new());
        let session_summaries = Arc::new(
            crate::session_summary::SessionSummaryStore::open(&tmp.path().join("s.db")).unwrap(),
        );
        let transcripts =
            Arc::new(crate::transcripts::TranscriptStore::open(&tmp.path().join("t.db")).unwrap());
        let s = registry
            .create(crate::session::SessionSpec {
                name: "t".into(),
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "sleep 5".into()],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                cols: 80,
                rows: 24,
                project_id: 1,
            })
            .unwrap();
        let id = s.id().to_string();
        // 关键:transcript 文件名(=真实 conversation_id)故意跟 dozerd 的
        // session_id 不一样(现实里 Claude 自己分配的 session id 跟
        // DOZER_SESSION_ID 从来不相等)——这个测试要验证的正是"就算两个
        // id 不同,只要显式传对 conversation_id,兜底算法依然能查到真实
        // 数据",而不是靠"文件名恰好等于 session_id"这种巧合通过
        // (2026-08-27 修正前的版本就是靠这个巧合掩盖了真实 bug)。
        let real_conversation_id = "totally-different-claude-conv-id";
        let file = tmp.path().join(format!("{real_conversation_id}.jsonl"));
        std::fs::write(
            &file,
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"人类发言\"}}\n",
        )
        .unwrap();
        transcripts
            .ingest_session(dozer_core::protocol::AgentKind::Claude, &file)
            .unwrap();

        finalize_session_summary(
            id.clone(),
            Some(real_conversation_id.to_string()),
            registry.clone(),
            session_summaries.clone(),
            transcripts,
            dozer_core::protocol::AgentKind::Claude,
            std::time::Duration::from_millis(30),
            std::time::Duration::from_millis(10),
        )
        .await;

        let got = session_summaries.get(&id).unwrap().unwrap();
        assert_eq!(
            got.status,
            dozer_core::protocol::SummaryStatus::HeuristicFallback
        );
        assert_eq!(got.title, "人类发言");
        assert_eq!(got.conversation_id.as_deref(), Some(real_conversation_id));
        assert!(!registry.get(&id).unwrap().info().alive, "应该已被 kill");
    }

    /// 会话从没收到过任何 hook 事件(`transcript_path` 恒 `None`)时,
    /// 兜底应该直接落占位文案,不该 panic 或跳过落库(2026-08-27 修正)。
    #[tokio::test]
    async fn finalize_session_summary_with_no_conversation_id_falls_back_to_placeholder() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = Arc::new(crate::registry::SessionRegistry::new());
        let session_summaries = Arc::new(
            crate::session_summary::SessionSummaryStore::open(&tmp.path().join("s.db")).unwrap(),
        );
        let transcripts =
            Arc::new(crate::transcripts::TranscriptStore::open(&tmp.path().join("t.db")).unwrap());
        let s = registry
            .create(crate::session::SessionSpec {
                name: "t".into(),
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "sleep 5".into()],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                cols: 80,
                rows: 24,
                project_id: 1,
            })
            .unwrap();
        let id = s.id().to_string();

        finalize_session_summary(
            id.clone(),
            None,
            registry.clone(),
            session_summaries.clone(),
            transcripts,
            dozer_core::protocol::AgentKind::Claude,
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(5),
        )
        .await;

        let got = session_summaries.get(&id).unwrap().unwrap();
        assert_eq!(
            got.status,
            dozer_core::protocol::SummaryStatus::HeuristicFallback
        );
        assert_eq!(got.title, "(无对话记录)");
        assert_eq!(got.conversation_id, None);
    }

    #[test]
    fn conversation_id_for_session_resolves_from_transcript_path() {
        let registry = crate::registry::SessionRegistry::new();
        let s = registry
            .create(crate::session::SessionSpec {
                name: "t".into(),
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "sleep 5".into()],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                cols: 80,
                rows: 24,
                project_id: 1,
            })
            .unwrap();
        assert_eq!(conversation_id_for_session(&s), None);
        s.set_transcript_path("/home/u/.claude/projects/x/my-conv-id.jsonl");
        assert_eq!(
            conversation_id_for_session(&s),
            Some("my-conv-id".to_string())
        );
        let _ = s.kill();
    }

    #[tokio::test]
    async fn bridge_starts_on_create_and_stops_after_process_exits() {
        use crate::ide_bridge::IdeBridgeRegistry;
        use crate::preview_context::PreviewContextStore;
        use crate::registry::SessionRegistry;
        use crate::session::SessionSpec;
        use std::time::Duration;

        fn spec(project_id: i64, cmd: &str) -> SessionSpec {
            SessionSpec {
                name: "t".into(),
                command: "/bin/sh".into(),
                args: vec!["-c".into(), cmd.into()],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                cols: 80,
                rows: 24,
                project_id,
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let ide_bridge = IdeBridgeRegistry::new(
            dir.path().to_path_buf(),
            Arc::new(PreviewContextStore::new()),
        );
        let registry = SessionRegistry::new();

        let s = registry.create(spec(7, "printf ready; sleep 5")).unwrap();
        assert!(ide_bridge.session_started(7, "/repo").await);
        assert_eq!(ide_bridge.active_projects().await, vec![7]);

        let mut exit_rx = s.subscribe();
        let ide_bridge_watch = ide_bridge.clone();
        let watcher = tokio::spawn(async move {
            loop {
                match exit_rx.recv().await {
                    Ok(SessionEvent::Exited { .. }) => {
                        ide_bridge_watch.session_ended(7).await;
                        break;
                    }
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        registry.kill(s.id()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), watcher)
            .await
            .expect("watcher 任务应在超时前结束")
            .expect("watcher 任务不应 panic");

        assert!(ide_bridge.active_projects().await.is_empty());
    }

    #[tokio::test]
    async fn drain_all_sessions_with_no_sessions_returns_immediately() {
        let registry = Arc::new(crate::registry::SessionRegistry::new());
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("t.db");
        let transcripts = Arc::new(crate::transcripts::TranscriptStore::open(&db).unwrap());
        let session_summaries =
            Arc::new(crate::session_summary::SessionSummaryStore::open(&db).unwrap());

        let started = std::time::Instant::now();
        drain_all_sessions(
            registry,
            session_summaries,
            transcripts,
            std::time::Duration::from_secs(60),
            std::time::Duration::from_secs(2),
        )
        .await;
        assert!(
            started.elapsed() < std::time::Duration::from_millis(200),
            "0 会话不应该等待任何轮询间隔"
        );
    }

    #[tokio::test]
    async fn drain_all_sessions_falls_back_to_heuristic_on_timeout_and_kills_session() {
        let registry = Arc::new(crate::registry::SessionRegistry::new());
        let session = registry
            .create(crate::session::SessionSpec {
                name: "test".into(),
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "sleep 5".into()],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                cols: 80,
                rows: 24,
                project_id: 1,
            })
            .unwrap();
        let sid = session.id().to_string();
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("t.db");
        let transcripts = Arc::new(crate::transcripts::TranscriptStore::open(&db).unwrap());
        let session_summaries =
            Arc::new(crate::session_summary::SessionSummaryStore::open(&db).unwrap());

        // 短超时/短轮询只为测试注入,不代表生产行为——生产调用点固定用
        // `CLOSE_WITH_SUMMARY_TIMEOUT`/`CLOSE_WITH_SUMMARY_POLL_INTERVAL`。
        drain_all_sessions(
            registry.clone(),
            session_summaries.clone(),
            transcripts,
            std::time::Duration::from_millis(80),
            std::time::Duration::from_millis(10),
        )
        .await;

        let summary = session_summaries
            .get(&sid)
            .unwrap()
            .expect("超时后应该落一份启发式兜底总结");
        assert_eq!(
            summary.status,
            dozer_core::protocol::SummaryStatus::HeuristicFallback
        );
        assert!(
            !registry.get(&sid).unwrap().info().alive,
            "超时兜底后应该 kill 掉这个会话"
        );
    }

    #[tokio::test]
    async fn drain_all_sessions_skips_already_dead_sessions() {
        let registry = Arc::new(crate::registry::SessionRegistry::new());
        let session = registry
            .create(crate::session::SessionSpec {
                name: "test".into(),
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "true".into()],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                cols: 80,
                rows: 24,
                project_id: 1,
            })
            .unwrap();
        let sid = session.id().to_string();
        // 等这个会话真的死掉。registry 从不摘除已死会话,所以它一定会
        // 出现在 `list()` 里。
        for _ in 0..200 {
            if !registry.get(&sid).unwrap().info().alive {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            !registry.get(&sid).unwrap().info().alive,
            "前置条件:会话应该已经退出"
        );

        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("t.db");
        let transcripts = Arc::new(crate::transcripts::TranscriptStore::open(&db).unwrap());
        let session_summaries =
            Arc::new(crate::session_summary::SessionSummaryStore::open(&db).unwrap());

        let started = std::time::Instant::now();
        drain_all_sessions(
            registry,
            session_summaries,
            transcripts,
            std::time::Duration::from_secs(3),
            std::time::Duration::from_millis(50),
        )
        .await;
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "已死会话不可能再落总结,却让它跑满 3s 超时;实际耗时 {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn kill_remaining_live_sessions_kills_live_and_tolerates_dead() {
        let registry = Arc::new(crate::registry::SessionRegistry::new());
        let spec = |cmd: &str| crate::session::SessionSpec {
            name: "test".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), cmd.into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
            project_id: 1,
        };
        let live = registry.create(spec("sleep 30")).unwrap();
        let live_id = live.id().to_string();
        let shortlived = registry.create(spec("true")).unwrap();
        let shortlived_id = shortlived.id().to_string();
        for _ in 0..200 {
            if !registry.get(&shortlived_id).unwrap().info().alive {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        kill_remaining_live_sessions(&registry);

        assert!(
            !registry.get(&live_id).unwrap().info().alive,
            "残留的存活会话必须被 kill,否则 dozerd 退出后它变孤儿进程"
        );
        assert!(
            !registry.get(&shortlived_id).unwrap().info().alive,
            "已死会话应该原样容忍,不 panic"
        );
    }
}
