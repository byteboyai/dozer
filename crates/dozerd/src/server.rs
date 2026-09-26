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

/// `serve`/`handle_conn` 共用的存储句柄集合,归到一个具名字段结构体里
/// (而不是继续平铺成十几个位置参数)——纯可维护性考虑:各 `Arc<XStore>`
/// 本身是互不相同的具体类型,位置传错本来就会被 Rust 类型检查拦下,不属于
/// CLAUDE.md"同类型相邻、编译器发现不了"那条裁决针对的情况,这里单纯是
/// 参数表已经太长。全部字段都是 `Arc<..>`,派生 `Clone` 零成本。
#[derive(Clone)]
pub struct Stores {
    pub registry: std::sync::Arc<SessionRegistry>,
    pub projects: std::sync::Arc<crate::projects::ProjectStore>,
    pub bookmarks: std::sync::Arc<crate::bookmarks::BookmarkStore>,
    pub code_health: std::sync::Arc<crate::code_health::CodeHealthStore>,
    pub transcripts: std::sync::Arc<crate::transcripts::TranscriptStore>,
    pub session_summaries: std::sync::Arc<crate::session_summary::SessionSummaryStore>,
    pub summary_jobs: std::sync::Arc<crate::summary_jobs::SummaryJobStore>,
    pub backfill_registry: std::sync::Arc<crate::session_summary_backfill::BackfillRegistry>,
    pub todos: std::sync::Arc<crate::todo::TodoStore>,
    pub categories: std::sync::Arc<crate::todo_category::CategoryStore>,
    pub memories: std::sync::Arc<crate::memory::MemoryStore>,
}

#[allow(clippy::too_many_arguments)]
pub async fn serve(
    socket: &Path,
    ide_lock_dir: PathBuf,
    stores: Stores,
    in_flight: crate::task_poller::InFlight,
) -> Result<()> {
    let Stores {
        registry,
        projects,
        bookmarks,
        code_health,
        transcripts,
        session_summaries,
        summary_jobs,
        backfill_registry,
        todos,
        categories,
        memories,
    } = stores;
    let preview_contexts = Arc::new(PreviewContextStore::new());
    let preview_commands = Arc::new(crate::preview_commands::PreviewCommandBus::new());
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
    let stores = Stores {
        registry,
        projects,
        bookmarks,
        code_health,
        transcripts,
        session_summaries,
        summary_jobs,
        backfill_registry,
        todos,
        categories,
        memories,
    };
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
                let stores = stores.clone();
                let preview_contexts = preview_contexts.clone();
                let preview_commands = preview_commands.clone();
                let ide_bridge = ide_bridge.clone();
                let in_flight = in_flight.clone();
                let draining = draining.clone();
                let shutdown_signal = shutdown_signal.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_conn(
                        stream,
                        stores,
                        preview_contexts,
                        preview_commands,
                        ide_bridge,
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

/// `Request::Shutdown` 收尾:对 `registry` 里当前存活的每个会话,把"可总结"
/// 的持久化成 summary job(不注入 prompt、不等 LLM),再统一 kill。下次启动
/// 由 worker 恢复执行。纯 shell/无 transcript 会话不提交(无内容可总结)。
fn drain_all_sessions(
    registry: Arc<SessionRegistry>,
    summary_service: std::sync::Arc<crate::summary_service::SummaryService>,
) {
    // 只收存活会话(spec 2026-09-19 §「实现要点」step 1)。`SessionRegistry`
    // 从不摘除已退出的会话,`list()` 里混着 `alive=false` 的死会话——死会话
    // 没有可总结的 transcript,直接跳过。
    let ids: Vec<String> = registry
        .list()
        .into_iter()
        .filter(|info| info.alive)
        .map(|info| info.id)
        .collect();
    for id in ids {
        let Some(s) = registry.get(&id) else { continue };
        let conversation_id = conversation_id_for_session(&s);
        let Some(cid) = conversation_id else {
            tracing::warn!(session_id = %id, "Shutdown 收尾:会话无 transcript,跳过总结任务");
            continue;
        };
        match crate::summary_config::resolve_provider(None, None) {
            crate::summary_config::SummaryProviderResolution::Configured(cfg, _) => {
                let spec = crate::summary_service::SubmitSpec {
                    conversation_id: cid,
                    source_session_id: Some(id.clone()),
                    trigger: dozer_core::protocol::SummaryTrigger::Shutdown,
                    provider: cfg.provider,
                    requested_model: cfg.model,
                    force: false,
                };
                if let Err(e) = summary_service.submit_single(&spec) {
                    tracing::warn!(error = %e, session_id = %id, "Shutdown 提交总结任务失败");
                }
            }
            crate::summary_config::SummaryProviderResolution::Required => {
                tracing::warn!(session_id = %id, "Shutdown 收尾:未配置 summary provider,跳过总结任务");
            }
        }
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
        // `PostToolUseFailure`/`AfterFileEdit` 是 Goose 的原生事件名(其余
        // 四家经 dozer-hook 翻译层已折叠进 `PostToolUse` 等),这里直接纳入
        // Running——Goose 走通用状态映射(见 spec D6)。
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "PostToolUseFailure"
        | "AfterFileEdit" => Some(Running),
        "Notification" => Some(AwaitingInput),
        "Stop" => Some(TurnEnded),
        "SessionStart" | "SessionEnd" => Some(Idle),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_conn(
    stream: UnixStream,
    stores: Stores,
    preview_contexts: Arc<PreviewContextStore>,
    preview_commands: Arc<crate::preview_commands::PreviewCommandBus>,
    ide_bridge: Arc<IdeBridgeRegistry>,
    in_flight: crate::task_poller::InFlight,
    draining: Arc<AtomicBool>,
    shutdown_signal: Arc<Notify>,
) -> Result<()> {
    let Stores {
        registry,
        projects,
        bookmarks,
        code_health,
        transcripts,
        session_summaries,
        summary_jobs,
        backfill_registry,
        todos,
        categories,
        memories,
    } = stores;
    // 总结调度服务:提交/查询走持久化表,状态在 SQLite 里,跨连接可见。每个
    // 连接构造一份轻量句柄(只是 Arc 引用 + 一个 scratch 根路径)。
    let summary_service = std::sync::Arc::new(crate::summary_service::SummaryService {
        jobs: summary_jobs.clone(),
        transcripts: transcripts.clone(),
        session_summaries: session_summaries.clone(),
        scratch_root: dozer_core::paths::state_dir().join("summary-scratch"),
    });
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
                                // Shutdown:持久化待处理总结任务并 kill 存活会话,
                                // 不注入 prompt、不等模型完成;下次启动由 worker
                                // 恢复执行。
                                drain_all_sessions(registry.clone(), summary_service.clone());
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
                        Request::SaveCodeHealthReport {
                            project_id,
                            report_json,
                            total_loc,
                            total_functions,
                            critical_functions,
                            overall_tier,
                            schema_version,
                            git_head,
                            git_branch,
                            git_dirty,
                        } => {
                            let info = dozer_core::protocol::CodeHealthReportInfo {
                                total_loc,
                                total_functions,
                                critical_functions,
                                overall_tier,
                                report_json,
                                scanned_at_ms: 0, // store 内部会用自己的 now_ms() 覆盖
                                schema_version,
                                git_head,
                                git_branch,
                                git_dirty,
                            };
                            match code_health.save(project_id, &info) {
                                Ok(()) => Reply::Ok,
                                Err(e) => Reply::Error {
                                    message: format!("保存代码健康度报告失败: {e}"),
                                },
                            }
                        }
                        Request::GetCodeHealthReport { project_id } => {
                            match code_health.get(project_id) {
                                Ok(report) => Reply::CodeHealthReport { report },
                                Err(e) => Reply::Error {
                                    message: format!("查询代码健康度报告失败: {e}"),
                                },
                            }
                        }
                        Request::ListCodeHealthReports { project_id, limit } => {
                            match code_health.list(project_id, limit) {
                                Ok(reports) => Reply::CodeHealthReports { reports },
                                Err(e) => Reply::Error {
                                    message: format!("查询代码健康度历史失败: {e}"),
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
                        Request::ListMemories { project_id } => match memories.list(project_id) {
                            Ok(memories) => Reply::Memories { memories },
                            Err(e) => Reply::Error {
                                message: format!("列共享记忆失败: {e}"),
                            },
                        },
                        Request::WriteMemory {
                            project_id,
                            title,
                            kind,
                            description,
                            body,
                            actor,
                        } => match memories.write(project_id, &title, &kind, &description, &body, &actor) {
                            Ok(detail) => Reply::MemoryDetail { detail },
                            Err(e) => Reply::Error {
                                message: format!("写共享记忆失败: {e}"),
                            },
                        },
                        Request::GetMemory { project_id, id } => match memories.get(project_id, id) {
                            Ok(detail) => Reply::MemoryDetail { detail },
                            Err(e) => Reply::Error {
                                message: format!("查共享记忆失败: {e}"),
                            },
                        },
                        Request::DeleteMemory { project_id, id, actor } => {
                            match memories.delete(project_id, id, &actor) {
                                Ok(()) => Reply::Ok,
                                Err(e) => Reply::Error {
                                    message: format!("删共享记忆失败: {e}"),
                                },
                            }
                        }
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
                        // T13:下行预览命令。入队并等待 app 回传终态;app 不在线或
                        // 超过超时时间则回 Timeout(不无限挂起)。
                        Request::RunPreviewCommand { command } => {
                            let request_id = command.request_id.clone();
                            match preview_commands.enqueue(command) {
                                Err(detail) => Reply::PreviewCommandResult {
                                    outcome:
                                        dozer_core::protocol::PreviewCommandOutcome::InternalError {
                                            request_id,
                                            detail,
                                        },
                                },
                                Ok(rx) => match tokio::time::timeout(
                                    std::time::Duration::from_millis(
                                        dozer_core::protocol::PREVIEW_COMMAND_TIMEOUT_MS,
                                    ),
                                    rx,
                                )
                                .await
                                {
                                    Ok(Ok(outcome)) => Reply::PreviewCommandResult { outcome },
                                    _ => {
                                        preview_commands.forget(&request_id);
                                        Reply::PreviewCommandResult {
                                            outcome:
                                                dozer_core::protocol::PreviewCommandOutcome::Timeout {
                                                    request_id,
                                                },
                                        }
                                    }
                                },
                            }
                        }
                        Request::TakePendingPreviewCommands { project_id } => {
                            Reply::PendingPreviewCommands {
                                commands: preview_commands.take(project_id),
                            }
                        }
                        Request::ReportPreviewCommandOutcome { outcome } => {
                            preview_commands.report(outcome);
                            Reply::Ok
                        }
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
                                    let conversation_id = conversation_id_for_session(&s);
                                    // 关闭流程:持久化总结任务(不注入 prompt、不等 LLM),
                                    // 然后立即 kill。有 transcript 才提交;无 transcript
                                    // (纯 shell/agent 没配好 hook)记录可见原因。
                                    if let Some(cid) = conversation_id.clone() {
                                        match crate::summary_config::resolve_provider(None, None) {
                                            crate::summary_config::SummaryProviderResolution::Configured(cfg, _) => {
                                                let spec = crate::summary_service::SubmitSpec {
                                                    conversation_id: cid,
                                                    source_session_id: Some(session_id.clone()),
                                                    trigger: dozer_core::protocol::SummaryTrigger::Close,
                                                    provider: cfg.provider,
                                                    requested_model: cfg.model,
                                                    force: false,
                                                };
                                                if let Err(e) = summary_service.submit_single(&spec) {
                                                    tracing::warn!(error = %e, %session_id, "提交关闭总结任务失败");
                                                }
                                            }
                                            crate::summary_config::SummaryProviderResolution::Required => {
                                                tracing::warn!(%session_id, "关闭会话但未配置 summary provider,跳过总结任务");
                                            }
                                        }
                                    } else {
                                        tracing::warn!(%session_id, "关闭会话但无 transcript,跳过总结任务");
                                    }
                                    if let Err(e) = registry.kill(&session_id) {
                                        tracing::warn!(error = %e, %session_id, "关闭会话失败(可能已死亡)");
                                    }
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
                        Request::SubmitSummaryJob {
                            conversation_id,
                            source_session_id,
                            trigger,
                            provider,
                            model,
                            force,
                        } => {
                            match crate::summary_config::resolve_provider(provider, model.clone())
                            {
                                crate::summary_config::SummaryProviderResolution::Required => {
                                    Reply::Error {
                                        message: "未配置 summary provider(需 summary 配置或 default_agent)".into(),
                                    }
                                }
                                crate::summary_config::SummaryProviderResolution::Configured(
                                    cfg,
                                    _,
                                ) => {
                                    let spec = crate::summary_service::SubmitSpec {
                                        conversation_id,
                                        source_session_id,
                                        trigger,
                                        provider: cfg.provider,
                                        requested_model: cfg.model.or(model),
                                        force,
                                    };
                                    match summary_service.submit_single(&spec) {
                                        Ok(job_id) => Reply::SummaryJobSubmitted { job_id },
                                        Err(e) => Reply::Error {
                                            message: format!("提交总结任务失败: {e}"),
                                        },
                                    }
                                }
                            }
                        }
                        Request::SubmitSummaryBatch { cwd, provider, model } => {
                            let _ = (provider, model);
                            match summary_service.submit_batch(&cwd) {
                                Ok((batch_id, total)) => {
                                    Reply::SummaryBatchSubmitted { batch_id, total }
                                }
                                Err(e) => Reply::Error { message: format!("提交批次失败: {e}") },
                            }
                        }
                        Request::GetSummaryJob { job_id } => {
                            match summary_service.jobs.get_job(job_id) {
                                Ok(job) => Reply::SummaryJob { job },
                                Err(e) => Reply::Error { message: format!("查询任务失败: {e}") },
                            }
                        }
                        Request::GetSummaryBatch { batch_id } => {
                            match summary_service.jobs.get_batch(batch_id) {
                                Ok(batch) => Reply::SummaryBatch { batch },
                                Err(e) => Reply::Error { message: format!("查询批次失败: {e}") },
                            }
                        }
                        Request::RetrySummaryJob { job_id } => {
                            match summary_service.retry_job(job_id) {
                                Ok(new_id) => Reply::SummaryJobSubmitted { job_id: new_id },
                                Err(e) => Reply::Error { message: format!("重试失败: {e}") },
                            }
                        }
                        Request::CancelSummaryBatch { batch_id } => {
                            match summary_service.cancel_batch(batch_id) {
                                Ok(()) => Reply::Ok,
                                Err(e) => Reply::Error { message: format!("取消失败: {e}") },
                            }
                        }
                        Request::GetSummaryProvider { ui_choice, ui_model } => {
                            let info = match crate::summary_config::resolve_provider(
                                ui_choice, ui_model,
                            ) {
                                crate::summary_config::SummaryProviderResolution::Configured(
                                    cfg,
                                    src,
                                ) => dozer_core::protocol::SummaryProviderInfo {
                                    provider: Some(cfg.provider),
                                    model: cfg.model,
                                    source: Some(match src {
                                        crate::summary_config::SummaryConfigSource::Ui => "ui",
                                        crate::summary_config::SummaryConfigSource::Summary => {
                                            "summary"
                                        }
                                        crate::summary_config::SummaryConfigSource::DefaultAgent => {
                                            "default_agent"
                                        }
                                    }
                                    .to_string()),
                                    required: false,
                                },
                                crate::summary_config::SummaryProviderResolution::Required => {
                                    dozer_core::protocol::SummaryProviderInfo {
                                        provider: None,
                                        model: None,
                                        source: None,
                                        required: true,
                                    }
                                }
                            };
                            Reply::SummaryProvider { info }
                        }
                        Request::GetSummaryResult { conversation_id } => {
                            match summary_service.jobs.get_result(&conversation_id) {
                                Ok(result) => Reply::SummaryResult { result },
                                Err(e) => Reply::Error { message: format!("查询结果失败: {e}") },
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
    fn agent_state_mapping_goose_events_run_and_never_await_input() {
        use dozer_core::protocol::AgentState::*;
        // Goose 没有独立稳定的"等待用户批准"事件,不能把 PreToolUse 猜成
        // AwaitingInput(它也会在自动批准模式下触发,见 spec D6)。
        assert_eq!(agent_state_for("PostToolUseFailure"), Some(Running));
        assert_eq!(agent_state_for("AfterFileEdit"), Some(Running));
        assert_ne!(agent_state_for("PreToolUse"), Some(AwaitingInput));
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
            code_health: std::sync::Arc<crate::code_health::CodeHealthStore>,
            transcripts: std::sync::Arc<crate::transcripts::TranscriptStore>,
            session_summaries: std::sync::Arc<crate::session_summary::SessionSummaryStore>,
            todos: std::sync::Arc<crate::todo::TodoStore>,
            categories: std::sync::Arc<crate::todo_category::CategoryStore>,
            memories: std::sync::Arc<crate::memory::MemoryStore>,
        ) {
            let fut = crate::server::serve(
                socket,
                ide_lock_dir,
                Stores {
                    registry,
                    projects,
                    bookmarks,
                    code_health,
                    transcripts,
                    session_summaries,
                    summary_jobs: std::sync::Arc::new(
                        crate::summary_jobs::SummaryJobStore::open(
                            &std::env::temp_dir().join("dozer-test-summary-jobs.db"),
                        )
                        .unwrap(),
                    ),
                    backfill_registry: std::sync::Arc::new(
                        crate::session_summary_backfill::BackfillRegistry::new(),
                    ),
                    todos,
                    categories,
                    memories,
                },
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
