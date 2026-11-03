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

dozer_core::scope!(LOG, module, "server");

/// client→server 单帧上限(编码后的 JSON 字节,包含换行前正文)。
const MAX_CLIENT_FRAME_BYTES: usize = 12 * 1024 * 1024;
/// server→client 单帧上限;发出前校验,避免半个 JSON 帧。
const MAX_SERVER_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// 有状态的有界帧读取器。未完成帧保存在 `pending`,即使包在 select 里被
/// 取消,下一次读取也从原来的半帧继续,不会丢字节或从帧中间重开。
struct FrameReader<R> {
    reader: R,
    pending: Vec<u8>,
}

impl<R: tokio::io::AsyncBufRead + Unpin> FrameReader<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            pending: Vec::new(),
        }
    }

    async fn next_frame(&mut self, max: usize) -> std::io::Result<Option<String>> {
        loop {
            // We append only through the first newline in fill_buf, so a
            // complete frame always ends at pending.last(). Avoid quadratic
            // rescanning when a large frame arrives one byte at a time.
            if self.pending.last() == Some(&b'\n') {
                let mut line = std::mem::take(&mut self.pending);
                line.pop();
                return String::from_utf8(line)
                    .map(Some)
                    .map_err(|_| protocol_frame_error());
            }
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                if self.pending.is_empty() {
                    return Ok(None);
                }
                let line = std::mem::take(&mut self.pending);
                return String::from_utf8(line)
                    .map(Some)
                    .map_err(|_| protocol_frame_error());
            }
            let consumed = if let Some(newline) = available.iter().position(|byte| *byte == b'\n') {
                if self.pending.len() + newline > max {
                    return Err(protocol_frame_error());
                }
                self.pending.extend_from_slice(&available[..=newline]);
                newline + 1
            } else {
                if self.pending.len() + available.len() > max {
                    return Err(protocol_frame_error());
                }
                self.pending.extend_from_slice(available);
                available.len()
            };
            self.reader.consume(consumed);
        }
    }
}

async fn write_frame(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    bytes: &[u8],
) -> std::io::Result<()> {
    write_frame_with_budget(writer, bytes, std::time::Duration::from_secs(3)).await
}
async fn write_frame_with_budget(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    bytes: &[u8],
    budget: std::time::Duration,
) -> std::io::Result<()> {
    tokio::time::timeout(budget, writer.write_all(bytes))
        .await
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Timeout: peer did not read reply",
            )
        })?
}

fn protocol_frame_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "协议帧超过上限或不是 UTF-8",
    )
}

fn encode_reply_checked(reply: &Reply) -> Result<String, Reply> {
    dozer_core::protocol::encode_line_bounded(reply, MAX_SERVER_FRAME_BYTES).map_err(|_| {
        Reply::Error {
            message: format!(
                "Limit: 应答超过 {} 字节帧上限，拒绝发送不完整结果",
                MAX_SERVER_FRAME_BYTES
            ),
        }
    })
}

/// 连接/普通工作/控制预留准入。终端长期订阅占连接名额,但不占普通请求
/// 执行名额;Shutdown 走独立控制名额,普通过载时仍能停止 daemon。
const MAX_CONNECTIONS: usize = 120;
const MAX_CONTROL_CONNECTIONS: usize = 8;
const MAX_WORKERS: usize = 16;
const MAX_WAITERS: usize = 64;
const MAX_CONTROL: usize = 4;
const ADMISSION_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
/// 新连接首帧期限:防止只占连接不发请求。
const FIRST_FRAME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

struct WaitingRequest<'a>(&'a std::sync::atomic::AtomicUsize);
impl Drop for WaitingRequest<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
struct RuntimeLimiter {
    connections: Arc<tokio::sync::Semaphore>,
    control_connections: Arc<tokio::sync::Semaphore>,
    work: Arc<tokio::sync::Semaphore>,
    control: Arc<tokio::sync::Semaphore>,
    waiting: std::sync::atomic::AtomicUsize,
}

impl RuntimeLimiter {
    /// 返回 `(permit, control_only)`。普通名额满时使用控制连接预留;这类
    /// 连接第一条请求必须是 Shutdown,否则回 Busy 并关闭。
    fn try_connection(&self) -> Option<(tokio::sync::OwnedSemaphorePermit, bool)> {
        if let Ok(permit) = self.connections.clone().try_acquire_owned() {
            return Some((permit, false));
        }
        self.control_connections
            .clone()
            .try_acquire_owned()
            .ok()
            .map(|permit| (permit, true))
    }

    async fn acquire_work(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        let waiting = self.waiting.fetch_add(1, Ordering::SeqCst);
        if waiting >= MAX_WAITERS {
            self.waiting.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        let _waiting = WaitingRequest(&self.waiting);
        let acquired =
            tokio::time::timeout(ADMISSION_WAIT, self.work.clone().acquire_owned()).await;
        acquired.ok()?.ok()
    }

    async fn acquire_control(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        tokio::time::timeout(ADMISSION_WAIT, self.control.clone().acquire_owned())
            .await
            .ok()?
            .ok()
    }
}

fn limiter() -> &'static RuntimeLimiter {
    static LIMITER: std::sync::OnceLock<RuntimeLimiter> = std::sync::OnceLock::new();
    LIMITER.get_or_init(|| RuntimeLimiter {
        connections: Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS)),
        control_connections: Arc::new(tokio::sync::Semaphore::new(MAX_CONTROL_CONNECTIONS)),
        work: Arc::new(tokio::sync::Semaphore::new(MAX_WORKERS)),
        control: Arc::new(tokio::sync::Semaphore::new(MAX_CONTROL)),
        waiting: std::sync::atomic::AtomicUsize::new(0),
    })
}

async fn admit_request(
    request: &Request,
) -> Result<Option<tokio::sync::OwnedSemaphorePermit>, String> {
    match request {
        Request::Hello { .. } => Ok(None),
        Request::Shutdown => limiter()
            .acquire_control()
            .await
            .map(Some)
            .ok_or_else(busy_message),
        _ => limiter()
            .acquire_work()
            .await
            .map(Some)
            .ok_or_else(busy_message),
    }
}

fn request_limit(request: &Request) -> Option<&'static str> {
    match request {
        Request::Write { data_b64, .. } if data_b64.len() > ((64 * 1024 + 2) / 3) * 4 => {
            Some("PTY input exceeds 64KiB")
        }
        Request::HookEvent { data, event, .. }
            if event.len() > 256
                || extract_transcript_path(data).is_some_and(|s| s.len() > 4096) =>
        {
            Some("terminal control fields exceed event/path budget (256B/4KiB)")
        }
        Request::StartTodoExecution {
            human_reply: Some(text),
            ..
        }
        | Request::ProcessTodoNow {
            human_reply: Some(text),
            ..
        } if text.len() > 64 * 1024 => Some("task input exceeds 64KiB"),
        Request::CreateSession {
            name,
            command,
            args,
            cwd,
            ..
        } if name.len() > 4096
            || cwd.len() > 4096
            || command.len() + args.iter().map(|s| s.len() + 1).sum::<usize>() > 128 * 1024 =>
        {
            Some("session fields exceed budget")
        }
        _ => None,
    }
}

async fn await_rpc<T>(
    deadline: tokio::time::Instant,
    work: impl std::future::Future<Output = Result<T, dozer_services::ServiceError>>,
) -> Result<T, dozer_services::ServiceError> {
    tokio::time::timeout_at(deadline, work).await.map_err(|_| {
        dozer_services::ServiceError::new(
            dozer_services::ServiceErrorKind::Unavailable,
            "Timeout: 请求可能已执行，结果未知，请查询状态",
        )
    })?
}

fn busy_message() -> String {
    "Busy: 服务繁忙，请稍后重试".to_string()
}

/// `ProcessTodoNow` handler 的实际执行:查任务 → 加入 in_flight → 调
/// `task_processor::process_task` → 移出 in_flight → 回最新任务或错误。
/// 拆成独立 async 函数是为了把"多条可能中途返回不同 Reply 的路径"收敛成
/// 一个 `Reply`(handle_conn 里那个大的 `match req` 整体求值成 `Reply` 一个值,
/// 不是逐个分支 `return`)。
#[allow(clippy::too_many_arguments)]
async fn process_todo_now(
    in_flight: &crate::task_poller::InFlight,
    executions: &Arc<crate::task_execution::TaskExecutionStore>,
    todos: &Arc<crate::todo::TodoStore>,
    session_summaries: &Arc<crate::session_summary::SessionSummaryStore>,
    transcripts: &Arc<crate::transcripts::TranscriptStore>,
    projects: &Arc<crate::projects::ProjectStore>,
    id: i64,
    human_reply: Option<String>,
    summary_service: &Arc<crate::summary_service::SummaryService>,
) -> Reply {
    let todo = match todos.get(id) {
        Ok(todo) => todo,
        Err(e) => {
            return Reply::Error {
                message: format!("任务不存在: id={id}: {e}"),
            };
        }
    };
    let Some(_guard) = crate::task_poller::try_enter(in_flight, id) else {
        return Reply::Error {
            message: "该任务正在处理中".into(),
        };
    };
    let result = crate::task_processor::process_task(
        executions,
        todos,
        session_summaries,
        transcripts,
        projects,
        &todo,
        human_reply.as_deref(),
        None,
        Some(summary_service),
    )
    .await;
    match result {
        Ok(_) => match todos.get(id) {
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

/// 异步启动一次 attempt:准入成功立刻回 attempt,后台执行;重复
/// idempotency_key 返回原 attempt,不重复跑。
#[allow(clippy::too_many_arguments)]
async fn start_todo_execution(
    in_flight: &crate::task_poller::InFlight,
    executions: &Arc<crate::task_execution::TaskExecutionStore>,
    todos: &Arc<crate::todo::TodoStore>,
    session_summaries: &Arc<crate::session_summary::SessionSummaryStore>,
    transcripts: &Arc<crate::transcripts::TranscriptStore>,
    projects: &Arc<crate::projects::ProjectStore>,
    id: i64,
    human_reply: Option<String>,
    idempotency_key: Option<String>,
    summary_service: &Arc<crate::summary_service::SummaryService>,
) -> Reply {
    let _ = session_summaries;
    let todo = match todos.get(id) {
        Ok(todo) => todo,
        Err(e) => {
            return Reply::Error {
                message: format!("任务不存在: id={id}: {e}"),
            };
        }
    };
    let guard = crate::task_poller::try_enter(in_flight, id);
    let admission = crate::task_processor::start_task(
        executions,
        todos,
        projects,
        &todo,
        human_reply.as_deref(),
        idempotency_key.as_deref(),
    );
    match admission {
        Ok(crate::task_execution::Admission::Existing(execution)) => Reply::TodoExecution {
            execution: Some(execution),
        },
        Ok(crate::task_execution::Admission::Started(attempt)) => {
            let Some(guard) = guard else {
                // 内存 in_flight 与 attempts 表出现竞争(例如旧同步路径刚占位):
                // 让 attempt Drop 标 Interrupted,不启动第二条执行。
                drop(attempt);
                return Reply::Error {
                    message: "该任务正在处理中".into(),
                };
            };
            let execution = attempt.info().clone();
            let transcripts = transcripts.clone();
            let summary_service = summary_service.clone();
            let task_owner = executions.clone();
            let runtime = tokio::runtime::Handle::current();
            tokio::spawn(async move {
                let _ = task_owner
                    .run_blocking(move || {
                        runtime.block_on(async move {
                            let _guard = guard;
                            let _ = crate::task_processor::run_attempt(
                                &transcripts,
                                &todo,
                                human_reply.as_deref(),
                                attempt,
                                Some(&summary_service),
                            )
                            .await;
                        })
                    })
                    .await;
            });
            Reply::TodoExecution {
                execution: Some(execution),
            }
        }
        Err(crate::task_execution::AdmissionError::InFlight) => Reply::Error {
            message: "该任务正在处理中".into(),
        },
        Err(error) => Reply::Error {
            message: error.to_string(),
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
    pub executions: std::sync::Arc<crate::task_execution::TaskExecutionStore>,
    pub categories: std::sync::Arc<crate::todo_category::CategoryStore>,
    pub memories: std::sync::Arc<crate::memory::MemoryStore>,
    pub file_edit_history: std::sync::Arc<crate::file_edit_history::FileEditHistoryStore>,
    pub groups: std::sync::Arc<crate::group_service::GroupService>,
    pub apps: std::sync::Arc<bytehost_apps::service::AppService>,
    pub locale: std::sync::Arc<crate::locale_settings::LocaleSettingsStore>,
}

#[derive(Clone)]
struct ConnectionContext {
    stores: Stores,
    preview_contexts: Arc<PreviewContextStore>,
    preview_commands: Arc<crate::preview_commands::PreviewCommandBus>,
    ide_bridge: Arc<IdeBridgeRegistry>,
    in_flight: crate::task_poller::InFlight,
    draining: Arc<AtomicBool>,
    shutdown_signal: Arc<Notify>,
}
async fn spawn_connection(
    mut stream: UnixStream,
    context: ConnectionContext,
    connections: &mut tokio::task::JoinSet<()>,
) {
    let Some((permit, control_only)) = limiter().try_connection() else {
        let _ = write_frame(
            &mut stream,
            encode_line(&Reply::Error {
                message: busy_message(),
            })
            .as_bytes(),
        )
        .await;
        return;
    };
    connections.spawn(async move {
        let _permit = permit;
        if let Err(error) = handle_conn(stream, context, control_only).await {
            dozer_core::log_debug!(LOG,error=%error,"连接结束");
        }
    });
}

/// 单实例检查:`socket` 背后已经有活着的监听者就报错。**启动期要尽早调用**(在做任何有副作用的启动工作——
/// 例如应用宿主的对账——之前),否则误启动的第二个 dozerd 会先动手、之后才发现自己不该启动。
pub async fn ensure_single_instance(socket: &Path) -> Result<()> {
    if socket.exists() && socket_has_live_listener(socket).await {
        anyhow::bail!(
            "dozerd 已在运行（socket={}），拒绝重复启动",
            socket.display()
        );
    }
    Ok(())
}

/// 探测 `socket` 路径背后是否还有活着的 dozerd 在监听。`UnixListener::bind`
/// 对已存在的路径会直接报错，绑定前必须先删掉旧文件——但删之前得确认它
/// 背后真的没人在听：一个已存在的 socket 文件不代表监听者已死，直接删掉
/// 会把仍然存活的旧实例的监听地址生生撤下，造成"新实例其实没绑上、旧实例
/// 还在但外部再也连不上"的悬空态（2026-09-26 真实事故：Settings 里点重启，
/// 新进程绑定成功但没走 `Request::Shutdown` 就退出，留下死 socket，旧孤儿
/// 进程仍活着却谁也连不上）。对无人监听的陈旧 socket 文件，connect 在
/// macOS/Linux 上都会立即返回 `ECONNREFUSED`，不会挂起，超时只是防御性兜底。
async fn socket_has_live_listener(socket: &Path) -> bool {
    matches!(
        tokio::time::timeout(
            std::time::Duration::from_millis(300),
            UnixStream::connect(socket),
        )
        .await,
        Ok(Ok(_))
    )
}

#[allow(clippy::too_many_arguments)]
pub async fn serve(
    socket: &Path,
    ide_lock_dir: PathBuf,
    stores: Stores,
    in_flight: crate::task_poller::InFlight,
    ide_bridge_enabled: bool,
) -> Result<()> {
    serve_with_startup(
        socket,
        ide_lock_dir,
        stores,
        in_flight,
        ide_bridge_enabled,
        None,
    )
    .await
}

pub async fn serve_with_startup(
    socket: &Path,
    ide_lock_dir: PathBuf,
    stores: Stores,
    in_flight: crate::task_poller::InFlight,
    ide_bridge_enabled: bool,
    startup: Option<tokio::sync::watch::Sender<bool>>,
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
        executions,
        categories,
        memories,
        file_edit_history,
        groups,
        apps,
        locale,
    } = stores;
    let preview_contexts = Arc::new(PreviewContextStore::new());
    let preview_commands = Arc::new(crate::preview_commands::PreviewCommandBus::new());
    // 清扫是同步阻塞 IO(遍历目录+读文件),挪到阻塞线程池执行,不卡住
    // 当前 executor 线程——`serve()` 起监听前先等它跑完,保证不会跟紧接着
    // 写的新锁文件产生"锁文件刚写就被当陈旧清掉"的竞态。
    if ide_bridge_enabled {
        {
            let sweep_dir = ide_lock_dir.clone();
            let _ = tokio::task::spawn_blocking(move || {
                crate::ide_bridge::sweep_stale_locks(&sweep_dir)
            })
            .await;
        }
    } else {
        dozer_core::log_info!(LOG, "IDE 桥接已按产品配置关闭");
    }
    let ide_bridge = IdeBridgeRegistry::new(ide_lock_dir, preview_contexts.clone());
    ensure_single_instance(socket).await?;
    if socket.exists() {
        std::fs::remove_file(socket)?;
    }
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(socket)?;
    if let Some(ready) = startup {
        ready.send_replace(true);
    }
    dozer_core::log_info!(LOG, socket = %socket.display(), "dozerd 监听中");
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
        executions,
        categories,
        memories,
        file_edit_history,
        groups,
        apps,
        locale,
    };
    let context = ConnectionContext {
        stores: stores.clone(),
        preview_contexts,
        preview_commands,
        ide_bridge: ide_bridge.clone(),
        in_flight,
        draining,
        shutdown_signal: shutdown_signal.clone(),
    };
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            Some(_) = connections.join_next(), if !connections.is_empty() => {},
            accepted = listener.accept() => {
                let (stream, _) = match accepted {
                    Ok(pair) => pair,
                    Err(e) => {
                        dozer_core::log_warn!(LOG, error = %e, "accept 失败，跳过本次连接");
                        continue;
                    }
                };
                spawn_connection(stream,context.clone(),&mut connections).await;
            }
            _ = shutdown_signal.notified() => {
                let _=tokio::time::timeout(std::time::Duration::from_secs(1),ide_bridge.shutdown()).await;
                // Brief bounded grace lets already connected clients receive Draining,
                // including a duplicate Shutdown, before all transports are closed.
                let grace=std::cmp::min(tokio::time::Instant::now()+std::time::Duration::from_millis(100),tokio::time::Instant::from_std(context.stores.registry.shutdown_deadline()));
                loop {tokio::select! {
                    _=tokio::time::sleep_until(grace)=>break,
                    Some(_)=connections.join_next(),if !connections.is_empty()=>{},
                    accepted=listener.accept()=>if let Ok((stream,_))=accepted{spawn_connection(stream,context.clone(),&mut connections).await;},
                }}
                connections.abort_all();
                while connections.join_next().await.is_some() {}
                dozer_core::log_info!(LOG, "收到 Shutdown 请求收尾完成，dozerd 退出");
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
#[cfg(test)]
fn maybe_ingest_from_hook_data(
    transcripts: &crate::transcripts::TranscriptStore,
    agent: dozer_core::protocol::AgentKind,
    data: &serde_json::Value,
) {
    let Some(path) = extract_transcript_path(data) else {
        return;
    };
    if let Err(e) = transcripts.ingest_session(agent, std::path::Path::new(path)) {
        dozer_core::log_warn!(LOG, error = %e, %path, "hook 触发的对话摄取失败");
    }
}

/// 会话状态转入 `Idle`/`AwaitingInput` 时的兜底摄取——替代定时轮询,
/// 复用现有状态机,只在"这一刻状态真的变了"才触发,同态重复事件不重复
/// 摄取(避免每次 hook 事件都无谓地读一次文件)。
#[cfg(test)]
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
        dozer_core::log_warn!(LOG, error = %e, %path, "待命态兜底摄取失败");
    }
}

/// 从一个存活/已死 `Session` 的 `transcript_path` 解析出 `conversation_id`
/// (同 `TranscriptStore::ingest_session` 的派生逻辑)。没有 `transcript_path`
/// (会话从没收到过 hook 事件)时返回 `None`(2026-08-27 修正)。
pub(crate) fn conversation_id_for_session(s: &crate::session::Session) -> Option<String> {
    s.info()
        .transcript_path
        .as_deref()
        .and_then(|p| crate::transcripts::conversation_id_for_path(std::path::Path::new(p)))
}

/// `Request::Shutdown` 收尾:对 `registry` 里当前存活的每个会话,把"可总结"
/// 的持久化成 summary job(不注入 prompt、不等 LLM),再统一 kill。下次启动
/// 由 worker 恢复执行。纯 shell/无 transcript 会话不提交(无内容可总结)。
pub(crate) fn enqueue_session_summary(
    session: &crate::session::Session,
    service: &crate::summary_service::SummaryService,
    trigger: dozer_core::protocol::SummaryTrigger,
) -> Result<()> {
    let info = session.info();
    let Some(cid) = conversation_id_for_session(session) else {
        return Ok(());
    };
    let Some(claim) = session.claim_summary() else {
        return Ok(());
    };
    let Some(path) = info.transcript_path else {
        return Ok(());
    };
    service.finalize_session(&crate::summary_jobs::SessionFinalization {
        session_id: info.id,
        conversation_id: cid,
        agent: info.agent,
        transcript_path: path,
        trigger,
    })?;
    claim.commit();
    Ok(())
}

fn drain_all_sessions(
    registry: Arc<SessionRegistry>,
    summary_service: std::sync::Arc<crate::summary_service::SummaryService>,
) {
    registry.begin_draining();
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
        if let Err(e) = enqueue_session_summary(
            &s,
            &summary_service,
            dozer_core::protocol::SummaryTrigger::Shutdown,
        ) {
            dozer_core::log_warn!(LOG, error = %e, session_id = %id, "Shutdown 提交总结任务失败");
        }
    }
    kill_remaining_live_sessions(&registry);
}

/// 三种停止入口(Shutdown RPC / SIGINT / SIGTERM)共用的收尾:
/// 提交会话总结、kill 存活会话、停止 apps。调用方负责 summary worker 标志
/// 与 socket/进程锁清理。
pub async fn shutdown_stores(stores: &Stores) {
    stores.registry.begin_draining();
    stores.executions.stop();
    stores.summary_jobs.stop();
    stores.groups.shutdown();
    if let Some(worker) = stores.transcripts.ingestion.get() {
        worker.stop();
    }
    let summary_service = std::sync::Arc::new(crate::summary_service::SummaryService {
        jobs: stores.summary_jobs.clone(),
        transcripts: stores.transcripts.clone(),
        session_summaries: stores.session_summaries.clone(),
        scratch_root: dozer_core::paths::state_dir().join("summary-scratch"),
    });
    let registry = stores.registry.clone();
    let drain = tokio::task::spawn_blocking(move || drain_all_sessions(registry, summary_service));
    if tokio::time::timeout(std::time::Duration::from_secs(6), drain)
        .await
        .is_err()
    {
        dozer_core::log_warn!(
            LOG,
            "shutdown summary drain timed out; pending jobs recover on restart"
        );
    }
    kill_remaining_live_sessions(&stores.registry);
    let deadline = tokio::time::Instant::from_std(stores.registry.shutdown_deadline());
    loop {
        let pending = stores.registry.pending_creates();
        let live = stores
            .registry
            .list()
            .into_iter()
            .filter(|s| s.alive)
            .count();
        if pending == 0 && live == 0 {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            dozer_core::log_warn!(
                LOG,
                pending_creates = pending,
                live_sessions = live,
                "shutdown budget expired; resources still pending"
            );
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    if tokio::time::timeout_at(deadline, stores.apps.shutdown())
        .await
        .is_err()
    {
        dozer_core::log_warn!(LOG, "apps shutdown exceeded total budget");
    }
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
            dozer_core::log_warn!(LOG,
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
    context: ConnectionContext,
    control_only: bool,
) -> Result<()> {
    let ConnectionContext {
        stores,
        preview_contexts,
        preview_commands,
        ide_bridge,
        in_flight,
        draining,
        shutdown_signal,
    } = context;
    let request_stores = stores.clone();
    let Stores {
        registry,
        projects: _,
        bookmarks: _,
        code_health: _,
        transcripts,
        session_summaries,
        summary_jobs,
        backfill_registry,
        todos: _,
        executions,
        categories: _,
        memories: _,
        file_edit_history: _,
        groups: _,
        apps,
        locale: _,
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
    let mut reader = FrameReader::new(BufReader::new(r));
    // attach 状态：订阅 + 会话 id
    let mut sub: Option<(String, broadcast::Receiver<SessionEvent>)> = None;
    // 应用变更订阅：`Subscribe` 之后把事件写成 `Changed`/`Resync` 推给本连接。
    // 只是一条失效信号通道，不独占连接的读侧（之后仍可发普通请求）。
    let mut app_sub: Option<broadcast::Receiver<bytehost_apps::event::AppEvent>> = None;
    // 已向本连接投递到的 offset 水位：过滤 snapshot 与 broadcast 之间重叠的字节
    let mut sent_until: u64 = 0;
    // 迟到 attach 到已退出会话时，广播已经错过；在 Attached 之后补发一次 Exited。
    let mut late_exit: Option<Option<i32>> = None;
    let mut first_frame = true;
    let mut stream_epoch: Option<u64> = None;

    loop {
        let read_first = first_frame;
        let subscribed = sub.is_some() || app_sub.is_some();
        tokio::select! {
            line = async {
                if read_first {
                    tokio::time::timeout(
                        FIRST_FRAME_TIMEOUT,
                        reader.next_frame(MAX_CLIENT_FRAME_BYTES),
                    )
                    .await
                    .map_err(|_| std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "首帧读取超时",
                    ))?
                } else if subscribed {
                    reader.next_frame(MAX_CLIENT_FRAME_BYTES).await
                } else {
                    tokio::time::timeout(std::time::Duration::from_secs(30),reader.next_frame(MAX_CLIENT_FRAME_BYTES)).await.map_err(|_|std::io::Error::new(std::io::ErrorKind::TimedOut,"Timeout: idle RPC connection"))?
                }
            } => {
                first_frame = false;
                let Some(line) = line? else { break }; // 客户端断连：直接退出，不动会话
                let mut should_exit_after_reply = false;
                // 控制预留连接只处理一条请求；语法错误也必须归还预留名额。
                let close_connection_after_reply = control_only;
                let reply = match decode_line::<Request>(&line) {
                    Err(e) => Reply::Error { message: format!("协议错误: {e}") },
                    Ok(req) => {
                        let operation_deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(if matches!(&req,Request::ProcessTodoNow{..}){603}else{10});
                        if registry.is_draining() && !matches!(&req,Request::Hello{..}|Request::Shutdown) {Reply::Error{message:"dozerd 正在停止，拒绝新请求".into()}} else if let Some(limit)=request_limit(&req) {Reply::Error{message:format!("Limit: {limit}")}} else if control_only && !matches!(&req, Request::Shutdown) {
                            Reply::Error {
                                message: busy_message(),
                            }
                        } else {
                            match admit_request(&req).await {
                                Err(message) => Reply::Error { message },
                                Ok(_permit) => match req {
                        request if crate::server_handlers::supports(&request) => {
                            let stores = request_stores.clone();
                            let contexts = preview_contexts.clone();
                            let commands = preview_commands.clone();
                            let runtime = tokio::runtime::Handle::current();
                            match tokio::time::timeout_at(operation_deadline, dozer_services::blocking::run(move || {
                                let _runtime = runtime.enter();
                                Ok(crate::server_handlers::handle(request, stores, contexts, commands))
                            })).await {
                                Ok(Ok(reply)) => reply,
                                Ok(Err(error)) => Reply::Error { message: error.to_string() },
                                Err(_) => Reply::Error { message: "Timeout: 操作超时，写入结果未知，请查询状态".into() },
                            }
                        }
                        // 协议版本握手:回自己的版本与产品名。旧客户端不发
                        // Hello 也不影响——这只是一个普通请求分支。
                        Request::Hello { protocol: _ } => Reply::Hello {
                            protocol: dozer_core::protocol::PROTOCOL_VERSION,
                            product: dozer_core::paths::product().name.to_string(),
                        },
                        Request::App { request } => match request {
                            // 订阅要接管本连接后续的推送，不能塞进 `apps.handle`(它是一问一答)。
                            // 在这里拦截：拿到接收者就登记，之后由下方 select! 分支持续写出。
                            bytehost_apps::proto::AppRequest::Subscribe => {
                                match apps.subscribe() {
                                    Some(rx) => {
                                        // 重复 `Subscribe` 覆盖旧订阅(不累加)。
                                        app_sub = Some(rx);
                                        Reply::App { reply: bytehost_apps::proto::AppReply::Subscribed }
                                    }
                                    None => Reply::App {
                                        reply: bytehost_apps::proto::AppReply::Failed {
                                            failure: bytehost_apps::proto::AppFailure::new(
                                                bytehost_apps::proto::AppErrorKind::Unavailable,
                                                "应用宿主不可用",
                                            ),
                                        },
                                    },
                                }
                            }
                            request => match tokio::time::timeout_at(operation_deadline,apps.handle(request)).await {
                                Ok(Ok(reply)) => Reply::App { reply },
                                Ok(Err(failure)) => Reply::App { reply: bytehost_apps::proto::AppReply::Failed { failure } },
                                Err(_)=>Reply::Error{message:"Timeout: 应用操作可能已执行，结果未知，请查询状态".into()},
                            },
                        },
                        Request::GetBackfillState => Reply::BackfillState { state: transcripts.ingestion().status() },
                        Request::RuntimeCapabilities => Reply::RuntimeCapabilities { capabilities: dozer_model_capabilities() },
                        Request::AttachV2 {session_id, from_offset, epoch} => match registry.get(&session_id) {
                            None => Reply::Error {message: format!("会话不存在: {session_id}")},
                            Some(session) if session.history_evicted()=>Reply::Error{message:"会话历史已逐出（HistoryEvicted）".into()},
                            Some(session) => {
                                let rx = session.subscribe();
                                let snapshot = session.attach_snapshot(from_offset);
                                let mode = if from_offset > 0 && snapshot.next_offset.checked_sub(from_offset) == Some(snapshot.bytes.len() as u64) {
                                    dozer_core::protocol::StreamMode::Resume
                                } else { dozer_core::protocol::StreamMode::Reset };
                                let state = session.terminal_state(snapshot.exit_status);
                                sub = if state.output_complete { None } else { Some((session_id, rx)) };
                                sent_until = snapshot.next_offset;
                                stream_epoch = Some(epoch);
                                Reply::Terminal { event: dozer_core::protocol::TerminalEvent::Snapshot {
                                    epoch, mode, start_offset: snapshot.next_offset - snapshot.bytes.len() as u64,
                                    next_offset: snapshot.next_offset, data: snapshot.bytes, state, degraded: false,
                                }}
                            }
                        },
                        Request::ListSessions => Reply::Sessions { sessions: registry.list() },
                        Request::CreateSession { name, command, args, cwd, cols, rows, project_id, agent } => {
                            if draining.load(Ordering::SeqCst) {
                                Reply::Error { message: "dozerd 正在停止,无法创建新会话".into() }
                            } else {
                                let create_registry=registry.clone();let create_cwd=cwd.clone();
                                let created=await_rpc(operation_deadline,dozer_services::blocking::run(move||create_registry.create(SessionSpec { name, command, args, cwd:create_cwd, cols, rows, project_id }).map_err(|e|dozer_services::ServiceError::new(dozer_services::ServiceErrorKind::Rejected,e.to_string())))).await;
                                match created {
                                    Ok(s) => {
                                        // picker 在创建 PTY 时已经知道即将启动的 agent。
                                        // 立即记下，不要等可能很晚甚至本轮不会到达的 hook，
                                        // 否则 GUI 重启后会把存活的 Codex 会话误认成 shell。
                                        s.set_agent(agent);
                                        // Independent of UI attachment and IDE-bridge startup.
                                        let exit_session = s.clone();
                                        let exit_service = summary_service.clone();
                                        tokio::spawn(async move {
                                            exit_session.wait_for_exit().await;
                                            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                                            if let Ok(Err(e)) = tokio::task::spawn_blocking(move ||enqueue_session_summary(&exit_session, &exit_service, dozer_core::protocol::SummaryTrigger::NaturalExit)).await {
                                                dozer_core::log_warn!(LOG, error=%e, "自然退出总结提交失败");
                                            }
                                        });
                                        // `session_started` 失败(绑端口/写锁文件出错)时不会在
                                        // registry 里留下这次调用对应的记录——这种情况下绝不能
                                        // spawn 退出监听器,否则这个会话将来退出时会去 `session_ended`
                                        // 一个它从未真正占过的项目名额,把同项目下另一个真正活着的
                                        // 会话的 bridge 提前拆掉(复现过的 bug,见代码审查记录)。
                                        if ide_bridge.session_started(project_id, &cwd).await {
                                            let ide_bridge_watch = ide_bridge.clone();
                                            let exit_session = s.clone();
                                            tokio::spawn(async move {
                                                exit_session.wait_for_exit().await;
                                                ide_bridge_watch.session_ended(project_id).await;
                                            });
                                        }
                                        Reply::Created { session: s.info() }
                                    }
                                    Err(e) => Reply::Error { message: e.to_string() },
                                }
                            }
                        }
                        Request::Attach { session_id, from_offset } => {
                            stream_epoch = None;
                            match registry.get(&session_id) {
                            None => Reply::Error { message: format!("会话不存在: {session_id}") },
                            Some(s) => {
                                if s.history_evicted() {
                                    Reply::Error {
                                        message: "会话历史已逐出（HistoryEvicted）".into(),
                                    }
                                } else {
                                // 先 subscribe 后取快照：保证快照与订阅之间不漏事件；
                                // 二者之间可能重叠投递的字节由 sent_until 水位在转发时过滤。
                                let rx = s.subscribe();
                                let snapshot = s.attach_snapshot(from_offset);
                                sub = Some((session_id.clone(), rx));
                                sent_until = snapshot.next_offset;
                                late_exit = snapshot.exit_status;
                                Reply::Attached {
                                    session_id,
                                    snapshot_b64: B64.encode(&snapshot.bytes),
                                    next_offset: snapshot.next_offset,
                                }
                                }
                            }
                            }
                        }
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
                                shutdown_stores(&request_stores).await;
                                should_exit_after_reply = true;
                                Reply::Ok
                            }
                        }
                        Request::HookEvent { session_id, agent, event, ts_ms, data } => {
                            if let Some(s)=registry.get(&session_id) {
                                s.set_agent(agent);
                                if let Some(path)=extract_transcript_path(&data){s.set_transcript_path(path);}
                                if let Some(state)=agent_state_for(&event){s.set_agent_state(state,&event,ts_ms);}
                                match s.info().transcript_path {
                                    Some(path)=>match transcripts.ingestion().submit(agent,PathBuf::from(path)) {Ok(())=>Reply::Ok,Err(message)=>Reply::Error{message}},
                                    None=>Reply::Ok,
                                }
                            } else {Reply::Ok}
                        }
                        Request::ProcessTodoNow {id,human_reply}=>{
                            let stores=request_stores.clone();let flight=in_flight.clone();let service=summary_service.clone();let runtime=tokio::runtime::Handle::current();
                            executions.run_blocking(move||runtime.block_on(process_todo_now(&flight,&stores.executions,&stores.todos,&stores.session_summaries,&stores.transcripts,&stores.projects,id,human_reply,&service))).await.unwrap_or_else(|message|Reply::Error{message})
                        }
                        Request::StartTodoExecution{id,human_reply,idempotency_key}=>{
                            let stores=request_stores.clone();let flight=in_flight.clone();let service=summary_service.clone();let runtime=tokio::runtime::Handle::current();
                            await_rpc(operation_deadline,dozer_services::blocking::run(move||Ok(runtime.block_on(start_todo_execution(&flight,&stores.executions,&stores.todos,&stores.session_summaries,&stores.transcripts,&stores.projects,id,human_reply,idempotency_key,&service))))).await.unwrap_or_else(|error|Reply::Error{message:error.to_string()})
                        }
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
                        Request::BackfillSessionSummaries { cwd } => {
                            match summary_service.submit_batch(&cwd) {
                                Err(e) => Reply::Error { message: e.to_string() },
                                Ok((batch_id, total)) => {
                                    backfill_registry.start(&cwd, total);
                                    let jobs = summary_jobs.clone();
                                    let progress = backfill_registry.clone();
                                    tokio::spawn(async move {
                                        let mut completed = 0;
                                        while let Ok(Some(batch)) = jobs.get_batch(batch_id) {
                                            let done = batch.total - batch.queued - batch.running;
                                            while completed < done { progress.increment(&cwd); completed += 1; }
                                            if done == batch.total { break; }
                                            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                                        }
                                    });
                                    Reply::Ok
                                }
                            }
                        }
                        _ => Reply::Error { message: "unsupported request".into() },
                        },
                    }
                }
                },
            };
                let line = match encode_reply_checked(&reply) {
                    Ok(line) => line,
                    Err(error_reply) => encode_line(&error_reply),
                };
                let written = write_frame(&mut w, line.as_bytes()).await;
                if should_exit_after_reply { shutdown_signal.notify_one(); }
                written?;
                if let Some(code) = late_exit.take() {
                    let event = Reply::Exited {
                        session_id: sub
                            .as_ref()
                            .map(|(id, _)| id.clone())
                            .unwrap_or_default(),
                        code,
                    };
                    write_frame(&mut w, encode_line(&event).as_bytes()).await?;
                    sub = None;
                }
                if should_exit_after_reply {
                    shutdown_signal.notify_one();
                    return Ok(());
                }
                if close_connection_after_reply {
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
                            let reply = terminal_output(stream_epoch, sid, data, offset);
                            write_frame(&mut w, encode_line(&reply).as_bytes()).await?;
                            sent_until = offset;
                        } else {
                            // 部分重叠，只发未投递过的尾部
                            let skip = data.len() - (offset - sent_until) as usize;
                            let reply = terminal_output(stream_epoch, sid, data[skip..].to_vec(), offset);
                            write_frame(&mut w, encode_line(&reply).as_bytes()).await?;
                            sent_until = offset;
                        }
                    }
                    Ok(SessionEvent::Agent { agent, state, event, ts_ms, transcript_path }) => {
                        let reply = if let Some(epoch) = stream_epoch {
                            Reply::Terminal {event: dozer_core::protocol::TerminalEvent::State {epoch, state: registry.get(&sid).expect("subscribed session").terminal_state(None)}}
                        } else { Reply::AgentEvent { session_id: sid, agent, state, event, ts_ms, transcript_path } };
                        write_frame(&mut w, encode_line(&reply).as_bytes()).await?;
                    }
                    Ok(SessionEvent::Exited { code }) => {
                        let reply = if let Some(epoch) = stream_epoch {
                            Reply::Terminal {event: dozer_core::protocol::TerminalEvent::State {epoch, state: registry.get(&sid).expect("subscribed session").terminal_state(Some(code))}}
                        } else { Reply::Exited { session_id: sid, code } };
                        write_frame(&mut w, encode_line(&reply).as_bytes()).await?;
                        sub = None;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let reply = match stream_epoch {
                            Some(epoch) => Reply::Terminal {event: dozer_core::protocol::TerminalEvent::ResyncRequired {epoch}},
                            None => Reply::Error { message: "lagged; reattach".into() },
                        };
                        write_frame(&mut w, encode_line(&reply).as_bytes()).await?;
                        sub = None;
                    }
                    Err(broadcast::error::RecvError::Closed) => { sub = None; }
                }
            }
            ev = async {
                match &mut app_sub {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            }, if app_sub.is_some() => {
                match bytehost_apps::service::change_reply(ev) {
                    Some(reply) => {
                        write_frame(&mut w, encode_line(&Reply::App { reply }).as_bytes()).await?;
                    }
                    None => { app_sub = None; }
                }
            }
        }
    }
    Ok(())
}

/// `LocateInFile` 遇到路径/编码问题时的兜底:不把 daemon 内部错误细节透传成
/// agent 能直接摸到的报错通道,统一表现成"空匹配列表"——调用方(`dozer-mcp`
/// 的 `locate_in_file` 工具)据此提示"没搜到,检查路径和 query"就够了,不需要
/// 额外区分"路径越界"和"文件不存在",这两者对 agent 来说都是同一句"重新确认
/// 一下路径"。
pub(crate) fn locate_error_to_empty_with_log(
    err: crate::file_mutation::LocateError,
    path: &str,
) -> Vec<dozer_core::protocol::LocateMatch> {
    dozer_core::log_debug!(LOG, ?err, path, "locate_in_file 失败");
    Vec::new()
}

/// `ApplyPreciseEdit` 成功后,延迟一小段时间再把「定位到新范围 + 短暂高亮」
/// 两条命令挂进 `PreviewCommandBus`——延迟是为了大概率排在
/// `git_watch.rs`(通用文件监听)触发的 `ReloadDocument` 之后,避免定位先于
/// 重载发生、又被重载盖掉视图状态(具体排序不做强保证,是啓发式,见 spec
/// "Change Feedback"一节)。不等待这两条命令的应答——纯 best-effort,目标
/// tab 未必存在(文件没被打开过),`PreviewCommandBus` 对找不到 tab 的情况本来
/// 就有 `NotFound` 语义,这里不关心结果。
pub(crate) fn schedule_post_edit_reveal(
    bus: std::sync::Arc<crate::preview_commands::PreviewCommandBus>,
    project_id: i64,
    path: String,
    start_line: u32,
    start_col: u32,
    end_line: u32,
    end_col: u32,
) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let target = dozer_core::protocol::PreviewCommandTarget::Path { path: path.clone() };
        let select_id = format!("agent-edit-select-{}", uuid::Uuid::new_v4());
        let _ = bus.enqueue(dozer_core::protocol::PreviewCommand {
            request_id: select_id,
            project_id,
            target: target.clone(),
            action: dozer_core::protocol::PreviewCommandAction::Select {
                start_line,
                start_column: start_col,
                end_line,
                end_column: end_col,
            },
            expected_revision: None,
        });
        let highlight_id = format!("agent-edit-highlight-{}", uuid::Uuid::new_v4());
        let _ = bus.enqueue(dozer_core::protocol::PreviewCommand {
            request_id: highlight_id,
            project_id,
            target,
            action: dozer_core::protocol::PreviewCommandAction::Highlight {
                start_line,
                start_column: start_col,
                end_line,
                end_column: end_col,
                duration_ms: 2000,
            },
            expected_revision: None,
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_frame_is_bounded_and_rejects_oversized_input_without_newline() {
        let bytes = vec![b'x'; 64];
        let mut reader = FrameReader::new(BufReader::new(&bytes[..]));
        let err = reader.next_frame(16).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);

        let bytes = b"{\"ok\":true}\n{\"next\":1}\n";
        let mut reader = FrameReader::new(BufReader::new(&bytes[..]));
        assert_eq!(
            reader.next_frame(1024).await.unwrap().as_deref(),
            Some("{\"ok\":true}")
        );
        assert_eq!(
            reader.next_frame(1024).await.unwrap().as_deref(),
            Some("{\"next\":1}")
        );
        assert!(reader.next_frame(1024).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn frame_reader_preserves_partial_frame_when_cancelled() {
        let (mut tx, rx) = tokio::io::duplex(1024);
        let mut reader = FrameReader::new(BufReader::new(rx));
        write_frame(&mut tx, b"{\"type\":").await.unwrap();
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            reader.next_frame(1024),
        )
        .await;
        write_frame(&mut tx, b"\"list_sessions\"}\n").await.unwrap();
        let frame = reader.next_frame(1024).await.unwrap().unwrap();
        assert_eq!(frame, "{\"type\":\"list_sessions\"}");
    }

    #[tokio::test]
    async fn control_connection_reserve_survives_normal_connection_exhaustion() {
        let limiter = RuntimeLimiter {
            connections: Arc::new(tokio::sync::Semaphore::new(1)),
            control_connections: Arc::new(tokio::sync::Semaphore::new(1)),
            work: Arc::new(tokio::sync::Semaphore::new(1)),
            control: Arc::new(tokio::sync::Semaphore::new(1)),
            waiting: std::sync::atomic::AtomicUsize::new(0),
        };
        let (first, control_first) = limiter.try_connection().unwrap();
        assert!(!control_first);
        let (second, control_second) = limiter.try_connection().unwrap();
        assert!(control_second, "普通连接满后应使用控制预留");
        assert!(limiter.try_connection().is_none());
        drop((first, second));
        assert!(limiter.try_connection().is_some());
    }

    #[tokio::test]
    async fn socket_has_live_listener_false_when_path_missing() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("dozerd.sock");
        assert!(!socket_has_live_listener(&socket).await);
    }

    #[tokio::test]
    async fn socket_has_live_listener_false_for_stale_file() {
        // 模拟"文件还在、监听者已死"的悬空态:留一个 socket 类型的文件
        // 在路径上,但没有任何进程在 accept。connect 应该拿到
        // ECONNREFUSED——但 listener 关闭在内核里生效有极短的异步窗口
        // (全量并行跑测试时能观察到),所以用轮询代替单次断言,
        // 跟本文件其余"等待某状态最终生效"的测试手法一致。
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("dozerd.sock");
        {
            let listener = UnixListener::bind(&socket).unwrap();
            drop(listener);
        }
        assert!(socket.exists());
        let mut alive = true;
        for _ in 0..50 {
            alive = socket_has_live_listener(&socket).await;
            if !alive {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(!alive, "dropped listener 的 socket 文件不应该还有人在监听");
    }

    /// 启动期的单实例检查:有活着的监听者就报错;路径不存在或只是陈旧的 socket 文件都放行。
    #[tokio::test]
    async fn ensure_single_instance_only_fails_when_someone_is_really_listening() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("dozerd.sock");
        ensure_single_instance(&socket)
            .await
            .expect("路径不存在:放行");
        let listener = UnixListener::bind(&socket).unwrap();
        let err = ensure_single_instance(&socket)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("已在运行"), "{err}");
        drop(listener);
        // 监听者没了:留下的 socket 文件只是陈旧文件,放行(`serve` 随后会清掉它)。listener 关闭在内核里生效有
        // 极短的异步窗口(全量并行跑测试时能观察到),所以轮询——与上面 `socket_has_live_listener_false_for_stale_file`
        // 同一手法
        let mut outcome = ensure_single_instance(&socket).await;
        for _ in 0..50 {
            if outcome.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            outcome = ensure_single_instance(&socket).await;
        }
        outcome.expect("陈旧 socket 文件:放行");
    }

    #[tokio::test]
    async fn socket_has_live_listener_true_when_someone_is_listening() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("dozerd.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        assert!(socket_has_live_listener(&socket).await);
        drop(listener);
    }

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
            file_edit_history: std::sync::Arc<crate::file_edit_history::FileEditHistoryStore>,
            groups: std::sync::Arc<crate::group_service::GroupService>,
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
                    executions: std::sync::Arc::new(
                        crate::task_execution::TaskExecutionStore::open(
                            &std::env::temp_dir().join("dozer-test-executions.db"),
                        )
                        .unwrap(),
                    ),
                    categories,
                    memories,
                    file_edit_history,
                    groups,
                    apps: bytehost_apps::service::AppService::unavailable("test"),
                    locale: std::sync::Arc::new(
                        crate::locale_settings::LocaleSettingsStore::open(
                            &std::env::temp_dir().join("dozer-test-locale-settings.json"),
                        )
                        .unwrap(),
                    ),
                },
                crate::task_poller::new_in_flight(),
                true,
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
        for _ in 0..200 {
            if !registry.get(&live_id).unwrap().info().alive {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        assert!(
            !registry.get(&live_id).unwrap().info().alive,
            "残留的存活会话必须被 kill,否则 dozerd 退出后它变孤儿进程"
        );
        assert!(
            !registry.get(&shortlived_id).unwrap().info().alive,
            "已死会话应该原样容忍,不 panic"
        );
    }
    #[test]
    fn concurrent_close_and_natural_exit_have_one_persistent_summary_owner() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let path = dir.path().join("summary-owner.jsonl");
        std::fs::write(&path,"{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"fixture\"}}\n").unwrap();
        let session = Arc::new(
            crate::session::Session::spawn(SessionSpec {
                name: "summary".into(),
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "sleep 30".into()],
                cwd: "/tmp".into(),
                cols: 80,
                rows: 24,
                project_id: 1,
            })
            .unwrap(),
        );
        session.set_agent(dozer_core::protocol::AgentKind::Claude);
        session.set_transcript_path(path.to_str().unwrap());
        let jobs = Arc::new(crate::summary_jobs::SummaryJobStore::open(&db).unwrap());
        let service = Arc::new(crate::summary_service::SummaryService {
            jobs: jobs.clone(),
            transcripts: Arc::new(crate::transcripts::TranscriptStore::open(&db).unwrap()),
            session_summaries: Arc::new(
                crate::session_summary::SessionSummaryStore::open(&db).unwrap(),
            ),
            scratch_root: dir.path().join("scratch"),
        });
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let mut workers = Vec::new();
        for trigger in [
            dozer_core::protocol::SummaryTrigger::NaturalExit,
            dozer_core::protocol::SummaryTrigger::Close,
        ] {
            let session = session.clone();
            let service = service.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                enqueue_session_summary(&session, &service, trigger)
            }));
        }
        barrier.wait();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }
        assert_eq!(
            jobs.list_jobs_by_conversation("summary-owner")
                .unwrap()
                .len(),
            1
        );
        session.kill().unwrap();
    }
    #[tokio::test]
    async fn a_peer_that_does_not_read_is_limited_by_the_write_deadline() {
        let (mut writer, _reader) = tokio::io::duplex(1);
        writer.write_all(b"x").await.unwrap();
        let error = write_frame_with_budget(&mut writer, b"reply\n", std::time::Duration::ZERO)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    }
    #[tokio::test]
    async fn cancelled_request_waiter_releases_admission() {
        let runtime = Arc::new(RuntimeLimiter {
            connections: Arc::new(tokio::sync::Semaphore::new(0)),
            control_connections: Arc::new(tokio::sync::Semaphore::new(1)),
            work: Arc::new(tokio::sync::Semaphore::new(0)),
            control: Arc::new(tokio::sync::Semaphore::new(1)),
            waiting: std::sync::atomic::AtomicUsize::new(0),
        });
        let copy = runtime.clone();
        let work = tokio::spawn(async move { copy.acquire_work().await });
        while runtime.waiting.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        work.abort();
        let _ = work.await;
        assert_eq!(runtime.waiting.load(Ordering::SeqCst), 0);
    }
}

fn dozer_model_capabilities() -> dozer_core::protocol::RuntimeCapabilities {
    dozer_core::protocol::RuntimeCapabilities {
        terminal_stream_v2: true,
        task_execution_state: true,
        locate_v2: true,
        backfill_state: true,
    }
}
fn terminal_output(epoch: Option<u64>, session_id: String, data: Vec<u8>, offset: u64) -> Reply {
    match epoch {
        Some(epoch) => Reply::Terminal {
            event: dozer_core::protocol::TerminalEvent::Output {
                replay: false,
                epoch,
                start_offset: offset - data.len() as u64,
                end_offset: offset,
                data,
            },
        },
        None => Reply::Output {
            session_id,
            data_b64: B64.encode(data),
            offset,
        },
    }
}
