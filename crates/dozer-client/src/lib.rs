use anyhow::{Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use bytehost_apps::proto::{AppReply, AppRequest};
use dozer_core::protocol::{
    AgentKind, AgentState, BookmarkInfo, BookmarkScope, CategoryInfo, CategoryMoveDirection,
    CodeHealthReportInfo, ConversationSummary, GroupCancelScope, GroupInfo, GroupMessageInfo,
    LocalePreference, LocaleSnapshot, LocaleWriteOutcome, PreviewCommand, PreviewContext,
    ProjectInfo, Reply, Request, SessionInfo, SessionSummaryPayload, TodoInfo, TurnRecord,
    UsagePayload, decode_line, encode_line,
};
use dozer_services::{
    BookmarkService, BoxFuture, CodeHealthService, ContextService, ConversationRows,
    ConversationService, EditHistoryService, GroupChatService, LocaleService, MemoryService,
    PreciseEdit, ProjectService, ServiceError, ServiceErrorKind, SessionService, SessionSpec,
    TodoService, TranscriptService, UsageRows, UsageService,
};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

/// 会话 attach 流事件已搬到 `dozer-model`(S6c-1);这里保留旧名字作为
/// 重新导出,既有 `dozer_client::TermEvent` 调用点不动。
pub use dozer_model::SessionEvent as TermEvent;

// 应用变更事件与失败类型统一来自 `bytehost-client`(与进程内实现共用一套语义)。
pub use bytehost_client::{AppApiError, AppChange, AppHostApi};

#[derive(Clone)]
pub struct Client {
    socket: PathBuf,
    /// 握手结果缓存:check_version 只在首个业务请求前真正跑一次。
    version_checked: std::sync::Arc<std::sync::OnceLock<()>>,
}

/// 版本握手失败:客户端与守护进程协议版本不一致、或连到了别的产品的
/// 守护进程。给出可读文案而不是莫名其妙的反序列化错误。
#[derive(Debug)]
pub struct ClientError {
    pub kind: ClientErrorKind,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ClientErrorKind {
    /// 客户端与守护进程协议版本不一致。
    ProtocolMismatch { client: u32, daemon: u32 },
    /// 连到了别的产品的守护进程(Dozer 客户端连上 diggerd,或反之)。
    ProductMismatch { client: String, daemon: String },
    /// 连不上守护进程,或守护进程对握手回了无法解释的应答。
    Unreachable(String),
}

/// Hello 应答的判定结果。放行策略见 `classify_hello_reply`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelloCheck {
    /// 版本与产品都匹配,正常使用。
    Compatible,
    /// 旧守护进程(不认识 Hello,回 Reply::Error):按用户 2026-10-20 的
    /// 兼容裁决放行——常驻的同产品旧 dozerd 不能因为只升级了客户端就被
    /// 拒之门外(那会逼用户先杀守护进程,丢掉所有存活会话)。`hello_once`
    /// 对这种放行会记一条 warn。
    OldDaemonAllowed,
}

/// 判定一帧 Hello 应答:版本一致且产品相同 → Compatible;旧守护进程回的
/// Reply::Error → OldDaemonAllowed(放行,warn);其余给出对应错误。
/// 抽成独立函数是为了让"哪几种应答算放行"可单测——这是兼容裁决的
/// 落点,改动它必须连测试一起改。
pub fn classify_hello_reply(reply: &Reply) -> Result<HelloCheck, ClientErrorKind> {
    match reply {
        Reply::Hello { protocol, product } => {
            if *protocol != dozer_core::protocol::PROTOCOL_VERSION {
                return Err(ClientErrorKind::ProtocolMismatch {
                    client: dozer_core::protocol::PROTOCOL_VERSION,
                    daemon: *protocol,
                });
            }
            let own = dozer_core::paths::product().name;
            if product != own {
                return Err(ClientErrorKind::ProductMismatch {
                    client: own.to_string(),
                    daemon: product.clone(),
                });
            }
            Ok(HelloCheck::Compatible)
        }
        // 旧守护进程(2026-10-20 之前)不认识 Hello,对未知请求一律回
        // Reply::Error。放行只针对这一种情形;能正常应答 Hello 的
        // 守护进程,版本/产品不一致就是不一致,不放行。
        Reply::Error { .. } => Ok(HelloCheck::OldDaemonAllowed),
        other => Err(ClientErrorKind::Unreachable(format!(
            "守护进程对握手返回了意外的应答: {other:?}"
        ))),
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            ClientErrorKind::ProtocolMismatch { client, daemon } => write!(
                f,
                "协议版本不一致:客户端 {client},守护进程 {daemon}。\
                 请把 Digger/Dozer 与它的守护进程升级到同一版本"
            ),
            ClientErrorKind::ProductMismatch { client, daemon } => write!(
                f,
                "连到了别的产品的守护进程:客户端是 \"{client}\",守护进程属于 \"{daemon}\"。\
                 请检查两个产品各自的数据目录与守护进程是否匹配"
            ),
            ClientErrorKind::Unreachable(msg) => write!(f, "连不上守护进程: {msg}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl Client {
    pub fn new(socket: PathBuf) -> Self {
        Self {
            socket,
            version_checked: std::sync::Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// 协议版本握手:与守护进程比对 PROTOCOL_VERSION 与产品名。
    ///
    /// **接入方式(复审 R2):** `roundtrip` 与 `attach` 在发第一个业务
    /// 请求前自动调用本方法(结果缓存),握手失败时业务请求直接返回对应
    /// 错误、不再发请求。
    ///
    /// **放行策略(用户 2026-10-20 裁决):** 版本号不一致、或连到了别的
    /// 产品的守护进程,一律拒绝;唯一放行的例外是**不认识 Hello 的旧
    /// 守护进程**(早于本改动、仍在运行的 dozerd,对 Hello 只回
    /// `Reply::Error`)——放行并记一条 warn,不阻断、不向用户弹错,
    /// 否则只升级客户端的用户得先杀守护进程,会丢掉所有存活会话。
    pub async fn check_version(&self) -> Result<(), ClientError> {
        if self.version_checked.get().is_some() {
            return Ok(());
        }
        let result = self.hello_once().await;
        if result.is_ok() {
            let _ = self.version_checked.set(());
        }
        result
    }

    /// roundtrip/attach 用的 anyhow 包装:握手失败把 ClientError 原样
    /// 交给业务调用方(可读文案随之带出去)。
    async fn ensure_version_checked(&self) -> Result<()> {
        self.check_version().await.map_err(anyhow::Error::new)
    }

    async fn hello_once(&self) -> Result<(), ClientError> {
        // 不走通用 roundtrip:它会把 Reply::Error 折叠成 anyhow 错误,
        // 而握手恰恰要区分"守护进程回了 Error(旧守护进程,放行)"与
        // "连不上"。
        let reply = self
            .roundtrip_keep_error(&Request::Hello {
                protocol: dozer_core::protocol::PROTOCOL_VERSION,
            })
            .await
            .map_err(|e| ClientError {
                kind: ClientErrorKind::Unreachable(e.to_string()),
            })?;
        match classify_hello_reply(&reply) {
            Ok(HelloCheck::Compatible) => Ok(()),
            Ok(HelloCheck::OldDaemonAllowed) => {
                tracing::warn!(
                    socket = %self.socket.display(),
                    "守护进程不认识协议握手(版本过旧),按兼容策略放行;建议尽快把 dozerd 与客户端升级到同一版本"
                );
                Ok(())
            }
            Err(kind) => Err(ClientError { kind }),
        }
    }

    /// roundtrip 的握手变体:Reply::Error 原样返回而不是折叠成错误,
    /// 让调用方区分"守护进程拒绝了这个请求"与"通信失败"。
    async fn roundtrip_keep_error(&self, req: &Request) -> Result<Reply> {
        let stream = UnixStream::connect(&self.socket).await?;
        let (r, mut w) = stream.into_split();
        w.write_all(encode_line(req).as_bytes()).await?;
        let mut lines = BufReader::new(r).lines();
        let line = lines.next_line().await?.ok_or_else(|| {
            anyhow::Error::new(ServiceError::new(
                ServiceErrorKind::Unavailable,
                "daemon 断开",
            ))
        })?;
        decode_line(&line)
    }

    async fn roundtrip(&self, req: &Request) -> Result<Reply> {
        // 复审 R2:首个业务请求前自动握手(结果缓存);失败时业务请求
        // 直接返回可读错误,不再把请求发给版本不符的守护进程。
        self.ensure_version_checked().await?;
        let stream = UnixStream::connect(&self.socket).await?;
        let (r, mut w) = stream.into_split();
        w.write_all(encode_line(req).as_bytes()).await?;
        let mut lines = BufReader::new(r).lines();
        let line = lines.next_line().await?.ok_or_else(|| {
            anyhow::Error::new(ServiceError::new(
                ServiceErrorKind::Unavailable,
                "daemon 断开",
            ))
        })?;
        let reply: Reply = decode_line(&line)?;
        if let Reply::Error { message } = &reply {
            return Err(anyhow::Error::new(ServiceError::new(
                ServiceErrorKind::Rejected,
                format!("daemon 错误: {message}"),
            )));
        }
        Ok(reply)
    }

    pub async fn list(&self) -> Result<Vec<SessionInfo>> {
        match self.roundtrip(&Request::ListSessions).await? {
            Reply::Sessions { sessions } => Ok(sessions),
            other => bail!("意外应答: {other:?}"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        name: &str,
        command: &str,
        args: &[String],
        cwd: &str,
        cols: u16,
        rows: u16,
        project_id: i64,
    ) -> Result<SessionInfo> {
        self.create_for_agent(
            name,
            command,
            args,
            cwd,
            cols,
            rows,
            project_id,
            AgentKind::Unknown,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_for_agent(
        &self,
        name: &str,
        command: &str,
        args: &[String],
        cwd: &str,
        cols: u16,
        rows: u16,
        project_id: i64,
        agent: AgentKind,
    ) -> Result<SessionInfo> {
        match self
            .roundtrip(&Request::CreateSession {
                name: name.into(),
                command: command.into(),
                args: args.to_vec(),
                cwd: cwd.into(),
                cols,
                rows,
                project_id,
                agent,
            })
            .await?
        {
            Reply::Created { session } => Ok(session),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn write(&self, id: &str, data: &[u8]) -> Result<()> {
        match self
            .roundtrip(&Request::Write {
                session_id: id.into(),
                data_b64: B64.encode(data),
            })
            .await?
        {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        match self
            .roundtrip(&Request::Resize {
                session_id: id.into(),
                cols,
                rows,
            })
            .await?
        {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn kill(&self, id: &str) -> Result<()> {
        match self
            .roundtrip(&Request::Kill {
                session_id: id.into(),
            })
            .await?
        {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn close_with_summary(&self, id: &str) -> Result<()> {
        match self
            .roundtrip(&Request::CloseWithSummary {
                session_id: id.into(),
            })
            .await?
        {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 请求 dozerd 对所有存活会话收尾后退出。内部等待时长与 daemon 侧
    /// 收尾耗时挂钩(最长约 60s),外层包一个 90s 超时兜底纯通信层面的
    /// 异常(进程卡死、socket 异常等),超时视为失败,不代表 daemon 一定
    /// 没停。
    pub async fn shutdown_daemon(&self) -> Result<()> {
        let outcome =
            tokio::time::timeout(Duration::from_secs(90), self.roundtrip(&Request::Shutdown)).await;
        interpret_shutdown_reply(outcome)
    }

    /// 触发某 cwd 下缺失总结会话的批量补录(项目"修复"按钮用,spec
    /// 2026-08-28)。立即返回;实际补录在 dozerd 后台完成,进度靠
    /// `get_session_summary_backfill_status` 轮询。
    pub async fn backfill_session_summaries(&self, cwd: &str) -> Result<()> {
        match self
            .roundtrip(&Request::BackfillSessionSummaries { cwd: cwd.into() })
            .await?
        {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 查询补总结进度,返回 `(completed, total)`。找不到进行中任务时回
    /// `(0, 0)`。
    pub async fn get_session_summary_backfill_status(&self, cwd: &str) -> Result<(u32, u32)> {
        match self
            .roundtrip(&Request::GetSessionSummaryBackfillStatus { cwd: cwd.into() })
            .await?
        {
            Reply::BackfillStatus { total, completed } => Ok((completed, total)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// V2:提交单条总结任务,返回 job_id。
    pub async fn submit_summary_job(
        &self,
        conversation_id: &str,
        source_session_id: Option<&str>,
        trigger: dozer_core::protocol::SummaryTrigger,
        provider: Option<AgentKind>,
        model: Option<&str>,
        force: bool,
    ) -> Result<i64> {
        match self
            .roundtrip(&Request::SubmitSummaryJob {
                conversation_id: conversation_id.into(),
                source_session_id: source_session_id.map(|s| s.into()),
                trigger,
                provider,
                model: model.map(|s| s.into()),
                force,
            })
            .await?
        {
            Reply::SummaryJobSubmitted { job_id } => Ok(job_id),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// V2:提交一批修复任务,返回 `(batch_id, 选中数)`。
    pub async fn submit_summary_batch(
        &self,
        cwd: &str,
        provider: Option<AgentKind>,
        model: Option<&str>,
    ) -> Result<(i64, u32)> {
        match self
            .roundtrip(&Request::SubmitSummaryBatch {
                cwd: cwd.into(),
                provider,
                model: model.map(|s| s.into()),
            })
            .await?
        {
            Reply::SummaryBatchSubmitted { batch_id, total } => Ok((batch_id, total)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// V2:查询单条任务状态。
    pub async fn get_summary_job(
        &self,
        job_id: i64,
    ) -> Result<Option<dozer_core::protocol::SummaryJobInfo>> {
        match self.roundtrip(&Request::GetSummaryJob { job_id }).await? {
            Reply::SummaryJob { job } => Ok(job),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// V2:查询批次状态。
    pub async fn get_summary_batch(
        &self,
        batch_id: i64,
    ) -> Result<Option<dozer_core::protocol::SummaryBatchInfo>> {
        match self
            .roundtrip(&Request::GetSummaryBatch { batch_id })
            .await?
        {
            Reply::SummaryBatch { batch } => Ok(batch),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// V2:重试失败任务,返回新 job_id。
    pub async fn retry_summary_job(&self, job_id: i64) -> Result<i64> {
        match self.roundtrip(&Request::RetrySummaryJob { job_id }).await? {
            Reply::SummaryJobSubmitted { job_id } => Ok(job_id),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// V2:取消批次。
    pub async fn cancel_summary_batch(&self, batch_id: i64) -> Result<()> {
        match self
            .roundtrip(&Request::CancelSummaryBatch { batch_id })
            .await?
        {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// V2:查询 summary provider 解析结果。
    pub async fn get_summary_provider(
        &self,
        ui_choice: Option<AgentKind>,
        ui_model: Option<&str>,
    ) -> Result<dozer_core::protocol::SummaryProviderInfo> {
        match self
            .roundtrip(&Request::GetSummaryProvider {
                ui_choice,
                ui_model: ui_model.map(|s| s.into()),
            })
            .await?
        {
            Reply::SummaryProvider { info } => Ok(info),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// V2:查询某 conversation 的规范总结结果。
    pub async fn get_summary_result(
        &self,
        conversation_id: &str,
    ) -> Result<Option<dozer_core::protocol::ConversationSummaryResult>> {
        match self
            .roundtrip(&Request::GetSummaryResult {
                conversation_id: conversation_id.into(),
            })
            .await?
        {
            Reply::SummaryResult { result } => Ok(result),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn open_project(&self, path: &str) -> Result<Option<ProjectInfo>> {
        match self
            .roundtrip(&Request::OpenProject { path: path.into() })
            .await?
        {
            Reply::Project { project } => Ok(project),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn rename_project(&self, id: i64, name: &str) -> Result<Option<ProjectInfo>> {
        match self
            .roundtrip(&Request::RenameProject {
                id,
                name: name.into(),
            })
            .await?
        {
            Reply::Project { project } => Ok(project),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn list_projects(&self) -> Result<Vec<ProjectInfo>> {
        match self.roundtrip(&Request::ListProjects).await? {
            Reply::Projects { projects } => Ok(projects),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn add_bookmark(
        &self,
        scope: BookmarkScope,
        project_id: Option<i64>,
        url: &str,
        title: &str,
    ) -> Result<()> {
        match self
            .roundtrip(&Request::AddBookmark {
                scope,
                project_id,
                url: url.into(),
                title: title.into(),
            })
            .await?
        {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn remove_bookmark(&self, id: i64) -> Result<()> {
        match self.roundtrip(&Request::RemoveBookmark { id }).await? {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn list_bookmarks(&self, project_id: Option<i64>) -> Result<Vec<BookmarkInfo>> {
        match self
            .roundtrip(&Request::ListBookmarks { project_id })
            .await?
        {
            Reply::Bookmarks { bookmarks } => Ok(bookmarks),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn get_locale(&self) -> Result<LocaleSnapshot> {
        match self.roundtrip(&Request::GetLocale).await? {
            Reply::Locale { snapshot } => Ok(snapshot),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn set_locale(
        &self,
        preference: LocalePreference,
        expected_revision: u64,
        system_candidates: Vec<String>,
    ) -> Result<LocaleWriteOutcome> {
        match self
            .roundtrip(&Request::SetLocale {
                preference,
                expected_revision,
                system_candidates,
            })
            .await?
        {
            Reply::LocaleWrite { outcome } => Ok(outcome),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn save_code_health_report(
        &self,
        project_id: i64,
        info: &CodeHealthReportInfo,
    ) -> Result<()> {
        match self
            .roundtrip(&Request::SaveCodeHealthReport {
                project_id,
                report_json: info.report_json.clone(),
                total_loc: info.total_loc,
                total_functions: info.total_functions,
                critical_functions: info.critical_functions,
                overall_tier: info.overall_tier.clone(),
                schema_version: info.schema_version,
                git_head: info.git_head.clone(),
                git_branch: info.git_branch.clone(),
                git_dirty: info.git_dirty,
            })
            .await?
        {
            Reply::Ok => Ok(()),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn get_code_health_report(
        &self,
        project_id: i64,
    ) -> Result<Option<CodeHealthReportInfo>> {
        match self
            .roundtrip(&Request::GetCodeHealthReport { project_id })
            .await?
        {
            Reply::CodeHealthReport { report } => Ok(report),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 列出某项目最近 N 份报告快照（时间倒序）。
    pub async fn list_code_health_reports(
        &self,
        project_id: i64,
        limit: u32,
    ) -> Result<Vec<CodeHealthReportInfo>> {
        match self
            .roundtrip(&Request::ListCodeHealthReports { project_id, limit })
            .await?
        {
            Reply::CodeHealthReports { reports } => Ok(reports),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn list_todos(&self, project_id: i64) -> Result<Vec<TodoInfo>> {
        match self.roundtrip(&Request::ListTodos { project_id }).await? {
            Reply::Todos { todos } => Ok(todos),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn add_todo(&self, project_id: i64, text: &str) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::AddTodo {
                project_id,
                text: text.into(),
            })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn toggle_todo(&self, id: i64, done: bool) -> Result<TodoInfo> {
        match self.roundtrip(&Request::ToggleTodo { id, done }).await? {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn edit_todo_text(&self, id: i64, text: &str) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::EditTodoText {
                id,
                text: text.into(),
            })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn reorder_todo(&self, id: i64, after_id: Option<i64>) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::ReorderTodo { id, after_id })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn list_memories(
        &self,
        project_id: i64,
    ) -> Result<Vec<dozer_core::protocol::MemoryInfo>> {
        match self
            .roundtrip(&Request::ListMemories { project_id })
            .await?
        {
            Reply::Memories { memories } => Ok(memories),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn write_memory(
        &self,
        project_id: i64,
        title: &str,
        kind: &str,
        description: &str,
        body: &str,
        actor: &str,
    ) -> Result<dozer_core::protocol::MemoryDetail> {
        match self
            .roundtrip(&Request::WriteMemory {
                project_id,
                title: title.into(),
                kind: kind.into(),
                description: description.into(),
                body: body.into(),
                actor: actor.into(),
            })
            .await?
        {
            Reply::MemoryDetail { detail } => Ok(detail),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn get_memory(
        &self,
        project_id: i64,
        id: i64,
    ) -> Result<dozer_core::protocol::MemoryDetail> {
        match self
            .roundtrip(&Request::GetMemory { project_id, id })
            .await?
        {
            Reply::MemoryDetail { detail } => Ok(detail),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn delete_memory(&self, project_id: i64, id: i64, actor: &str) -> Result<()> {
        match self
            .roundtrip(&Request::DeleteMemory {
                project_id,
                id,
                actor: actor.into(),
            })
            .await?
        {
            Reply::Ok => Ok(()),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn create_group(&self, project_id: i64, topic: &str) -> Result<GroupInfo> {
        match self
            .roundtrip(&Request::CreateGroup {
                project_id,
                topic: topic.into(),
            })
            .await?
        {
            Reply::Group { group } => Ok(group),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn list_groups(&self, project_id: i64) -> Result<Vec<GroupInfo>> {
        match self.roundtrip(&Request::ListGroups { project_id }).await? {
            Reply::Groups { groups } => Ok(groups),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn delete_group(&self, group_id: i64) -> Result<()> {
        match self.roundtrip(&Request::DeleteGroup { group_id }).await? {
            Reply::Ok => Ok(()),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn add_group_member(
        &self,
        group_id: i64,
        agent: AgentKind,
        handle: &str,
        role_prompt: &str,
    ) -> Result<GroupInfo> {
        match self
            .roundtrip(&Request::AddGroupMember {
                group_id,
                agent,
                handle: handle.into(),
                role_prompt: role_prompt.into(),
            })
            .await?
        {
            Reply::Group { group } => Ok(group),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn update_group_member(
        &self,
        member_id: i64,
        handle: &str,
        role_prompt: &str,
    ) -> Result<GroupInfo> {
        match self
            .roundtrip(&Request::UpdateGroupMember {
                member_id,
                handle: handle.into(),
                role_prompt: role_prompt.into(),
            })
            .await?
        {
            Reply::Group { group } => Ok(group),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn remove_group_member(&self, member_id: i64) -> Result<GroupInfo> {
        match self
            .roundtrip(&Request::RemoveGroupMember { member_id })
            .await?
        {
            Reply::Group { group } => Ok(group),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 返回 `(human 消息, 排队占位, 未识别的 handle)`。
    pub async fn post_group_message(
        &self,
        group_id: i64,
        text: &str,
    ) -> Result<(GroupMessageInfo, Vec<GroupMessageInfo>, Vec<String>)> {
        match self
            .roundtrip(&Request::PostGroupMessage {
                group_id,
                text: text.into(),
            })
            .await?
        {
            Reply::GroupPosted {
                human,
                placeholders,
                unknown_handles,
            } => Ok((human, placeholders, unknown_handles)),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 返回 `rev > after_rev` 的消息与新的 `latest_rev`。
    pub async fn list_group_messages(
        &self,
        group_id: i64,
        after_rev: i64,
        limit: u32,
    ) -> Result<(Vec<GroupMessageInfo>, i64)> {
        match self
            .roundtrip(&Request::ListGroupMessages {
                group_id,
                after_rev,
                limit,
            })
            .await?
        {
            Reply::GroupMessages {
                messages,
                latest_rev,
            } => Ok((messages, latest_rev)),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn cancel_group(&self, group_id: i64, scope: GroupCancelScope) -> Result<()> {
        match self
            .roundtrip(&Request::CancelGroup { group_id, scope })
            .await?
        {
            Reply::Ok => Ok(()),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn retry_group_message(&self, message_id: i64) -> Result<GroupMessageInfo> {
        match self
            .roundtrip(&Request::RetryGroupMessage { message_id })
            .await?
        {
            Reply::GroupMessage { message } => Ok(message),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 把群消息推送为一条待办(只新建,不指派——任务分配归 Todo)。
    pub async fn push_group_message_to_todo(
        &self,
        message_id: i64,
        text: &str,
    ) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::PushGroupMessageToTodo {
                message_id,
                text: text.into(),
            })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn locate_in_file(
        &self,
        project_id: i64,
        path: &str,
        query: &str,
    ) -> Result<Vec<dozer_core::protocol::LocateMatch>> {
        match self
            .roundtrip(&Request::LocateInFile {
                project_id,
                path: path.into(),
                query: query.into(),
            })
            .await?
        {
            Reply::LocateMatches { matches } => Ok(matches),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn apply_precise_edit(
        &self,
        project_id: i64,
        path: &str,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
        expected_text: &str,
        new_text: &str,
        summary: &str,
        actor: &str,
        session_id: &str,
    ) -> Result<dozer_core::protocol::MutationOutcome> {
        match self
            .roundtrip(&Request::ApplyPreciseEdit {
                project_id,
                path: path.into(),
                start_line,
                start_col,
                end_line,
                end_col,
                expected_text: expected_text.into(),
                new_text: new_text.into(),
                summary: summary.into(),
                actor: actor.into(),
                session_id: session_id.into(),
            })
            .await?
        {
            Reply::MutationResult { outcome } => Ok(outcome),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn add_context_item(
        &self,
        project_id: i64,
        entity_kind: &str,
        entity_ref: &str,
    ) -> Result<dozer_core::protocol::ContextItemInfo> {
        match self
            .roundtrip(&Request::AddContextItem {
                project_id,
                entity_kind: entity_kind.into(),
                entity_ref: entity_ref.into(),
            })
            .await?
        {
            Reply::ContextItem { item } => Ok(item),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn remove_context_item(&self, project_id: i64, id: i64) -> Result<()> {
        match self
            .roundtrip(&Request::RemoveContextItem { project_id, id })
            .await?
        {
            Reply::Ok => Ok(()),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn list_context_items(
        &self,
        project_id: i64,
    ) -> Result<Vec<dozer_core::protocol::ContextItemInfo>> {
        match self
            .roundtrip(&Request::ListContextItems { project_id })
            .await?
        {
            Reply::ContextItems { items } => Ok(items),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn list_file_edit_history(
        &self,
        project_id: i64,
        path_filter: Option<&str>,
        limit: u32,
    ) -> Result<Vec<dozer_core::protocol::FileEditHistoryInfo>> {
        match self
            .roundtrip(&Request::ListFileEditHistory {
                project_id,
                path_filter: path_filter.map(String::from),
                limit,
            })
            .await?
        {
            Reply::FileEditHistory { entries } => Ok(entries),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn set_todo_plan_date(&self, id: i64, plan_date: Option<&str>) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::SetTodoPlanDate {
                id,
                plan_date: plan_date.map(String::from),
            })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn assign_todo_agent(&self, id: i64, agent: AgentKind) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::AssignTodoAgent { id, agent })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn set_category_auto_poll(&self, id: i64, enabled: bool) -> Result<CategoryInfo> {
        match self
            .roundtrip(&Request::SetCategoryAutoPoll { id, enabled })
            .await?
        {
            Reply::Category { category } => Ok(category),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 立即触发一次任务处理(不等轮询)。可能耗时数分钟(headless 处理
    /// 上限 10 分钟),调用方需要走异步 `handle.spawn`,不能阻塞 UI 线程。
    pub async fn process_todo_now(&self, id: i64, human_reply: Option<&str>) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::ProcessTodoNow {
                id,
                human_reply: human_reply.map(|s| s.to_string()),
            })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn get_todo_detail(&self, id: i64) -> Result<(TodoInfo, Vec<TurnRecord>)> {
        match self.roundtrip(&Request::GetTodoDetail { id }).await? {
            Reply::TodoDetail { info, turns } => Ok((info, turns)),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 把任务原子设为某个已存储逻辑状态(待办/搁置/已完成)。见协议侧
    /// `TodoStoredStatus` 各值对应哪些 `done`/`paused`/派发记录的落盘组合。
    pub async fn set_todo_status(
        &self,
        id: i64,
        status: dozer_core::protocol::TodoStoredStatus,
    ) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::SetTodoStatus { id, status })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn list_categories(&self, project_id: i64) -> Result<Vec<CategoryInfo>> {
        match self
            .roundtrip(&Request::ListCategories { project_id })
            .await?
        {
            Reply::Categories { categories } => Ok(categories),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn add_category(
        &self,
        project_id: i64,
        parent_id: Option<i64>,
        name: &str,
    ) -> Result<CategoryInfo> {
        match self
            .roundtrip(&Request::AddCategory {
                project_id,
                parent_id,
                name: name.into(),
            })
            .await?
        {
            Reply::Category { category } => Ok(category),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn rename_category(&self, id: i64, name: &str) -> Result<CategoryInfo> {
        match self
            .roundtrip(&Request::RenameCategory {
                id,
                name: name.into(),
            })
            .await?
        {
            Reply::Category { category } => Ok(category),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn delete_category(&self, id: i64) -> Result<()> {
        match self.roundtrip(&Request::DeleteCategory { id }).await? {
            Reply::Ok => Ok(()),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn reparent_category(
        &self,
        id: i64,
        new_parent_id: Option<i64>,
    ) -> Result<CategoryInfo> {
        match self
            .roundtrip(&Request::ReparentCategory { id, new_parent_id })
            .await?
        {
            Reply::Category { category } => Ok(category),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn move_category_sibling(
        &self,
        id: i64,
        direction: CategoryMoveDirection,
    ) -> Result<CategoryInfo> {
        match self
            .roundtrip(&Request::MoveCategorySibling { id, direction })
            .await?
        {
            Reply::Category { category } => Ok(category),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn set_todo_category(&self, id: i64, category_id: Option<i64>) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::SetTodoCategory { id, category_id })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn list_conversations(
        &self,
        cwd: &str,
        agent: Option<AgentKind>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<ConversationSummary>> {
        match self
            .roundtrip(&Request::ListConversations {
                cwd: cwd.into(),
                agent,
                limit,
                offset,
            })
            .await?
        {
            Reply::Conversations { conversations } => Ok(conversations),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn get_conversation_turns(
        &self,
        conversation_id: &str,
        after_turn_index: i64,
        limit: u32,
    ) -> Result<Vec<TurnRecord>> {
        match self
            .roundtrip(&Request::GetConversationTurns {
                conversation_id: conversation_id.into(),
                after_turn_index,
                limit,
            })
            .await?
        {
            Reply::ConversationTurns { turns, .. } => Ok(turns),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn backfill_project_transcripts(&self, cwd: &str) -> Result<u32> {
        match self
            .roundtrip(&Request::BackfillProjectTranscripts { cwd: cwd.into() })
            .await?
        {
            Reply::BackfillDone { imported_files } => Ok(imported_files),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn remove_project(&self, id: i64) -> Result<()> {
        match self.roundtrip(&Request::RemoveProject { id }).await? {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn delete_project_transcripts(&self, cwd: &str) -> Result<u32> {
        match self
            .roundtrip(&Request::DeleteProjectTranscripts { cwd: cwd.into() })
            .await?
        {
            Reply::DeletedTranscripts { conversations } => Ok(conversations),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 某 cwd 下会话列表，每行附上该会话的总结(`None` 表示该会话没有
    /// 已产出的总结，spec 2026-08-27)。
    pub async fn list_conversations_with_summaries(
        &self,
        cwd: &str,
        agent: Option<AgentKind>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<(ConversationSummary, Option<SessionSummaryPayload>)>> {
        match self
            .roundtrip(&Request::ListConversationsWithSummaries {
                cwd: cwd.into(),
                agent,
                limit,
                offset,
            })
            .await?
        {
            Reply::ConversationsWithSummaries { rows } => Ok(rows),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn get_usage_summary(
        &self,
        cwd: &str,
        since_ts: Option<u64>,
    ) -> Result<Vec<(ConversationSummary, UsagePayload)>> {
        match self
            .roundtrip(&Request::GetUsageSummary {
                cwd: cwd.into(),
                since_ts,
            })
            .await?
        {
            Reply::UsageSummary { rows } => Ok(rows),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn update_preview_context(
        &self,
        project_id: i64,
        context: Option<PreviewContext>,
    ) -> Result<()> {
        match self
            .roundtrip(&Request::UpdatePreviewContext {
                project_id,
                context,
            })
            .await?
        {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn get_preview_context(&self, project_id: i64) -> Result<Option<PreviewContext>> {
        match self
            .roundtrip(&Request::GetPreviewContext { project_id })
            .await?
        {
            Reply::PreviewContext { context } => Ok(context),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// T13:提交一条预览命令,等待终态(app 处理或超时)。
    pub async fn run_preview_command(
        &self,
        command: PreviewCommand,
    ) -> Result<dozer_core::protocol::PreviewCommandOutcome> {
        match self
            .roundtrip(&Request::RunPreviewCommand { command })
            .await?
        {
            Reply::PreviewCommandResult { outcome } => Ok(outcome),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// T13:app 取走某项目待处理的预览命令。
    pub async fn take_pending_preview_commands(
        &self,
        project_id: i64,
    ) -> Result<Vec<PreviewCommand>> {
        match self
            .roundtrip(&Request::TakePendingPreviewCommands { project_id })
            .await?
        {
            Reply::PendingPreviewCommands { commands } => Ok(commands),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// T13:app 回报某条预览命令的终态。
    pub async fn report_preview_command_outcome(
        &self,
        outcome: dozer_core::protocol::PreviewCommandOutcome,
    ) -> Result<()> {
        match self
            .roundtrip(&Request::ReportPreviewCommandOutcome { outcome })
            .await?
        {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn record_session_summary(&self, id: &str, title: &str, summary: &str) -> Result<()> {
        match self
            .roundtrip(&Request::RecordSessionSummary {
                session_id: id.into(),
                title: title.into(),
                summary: summary.into(),
            })
            .await?
        {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn get_session_summary(&self, id: &str) -> Result<Option<SessionSummaryPayload>> {
        match self
            .roundtrip(&Request::GetSessionSummary {
                session_id: id.into(),
            })
            .await?
        {
            Reply::SessionSummary { summary } => Ok(summary),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn attach(
        &self,
        id: &str,
        from_offset: u64,
    ) -> Result<(Vec<u8>, u64, mpsc::UnboundedReceiver<TermEvent>)> {
        // 复审 R2:attach 也是业务入口,同样先握手(缓存,只跑一次)。
        self.ensure_version_checked().await?;
        let stream = UnixStream::connect(&self.socket).await?;
        let (r, mut w) = stream.into_split();
        w.write_all(
            encode_line(&Request::Attach {
                session_id: id.into(),
                from_offset,
            })
            .as_bytes(),
        )
        .await?;
        let mut lines = BufReader::new(r).lines();
        let first = lines
            .next_line()
            .await?
            .ok_or_else(|| anyhow!("daemon 断开"))?;
        let (snapshot, next) = match decode_line::<Reply>(&first)? {
            Reply::Attached {
                snapshot_b64,
                next_offset,
                ..
            } => (B64.decode(snapshot_b64.as_bytes())?, next_offset),
            Reply::Error { message } => bail!("attach 失败: {message}"),
            other => bail!("意外应答: {other:?}"),
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let id = id.to_string();
        tokio::spawn(async move {
            let _keep_writer = w;
            loop {
                tokio::select! {
                    // consumer（rx）被 drop（例如 tab 被关闭）：没有人再消费事件，
                    // 停止读循环，随后 lines/_keep_writer 一并 drop，UnixStream
                    // 两端都关闭，daemon 侧 handle_conn 才能在下一次
                    // lines.next_line() 上收到 EOF 并退出，避免任务+FD 滞留。
                    _ = tx.closed() => {
                        tracing::debug!(session_id = %id, "attach receiver 已关闭，读任务退出");
                        break;
                    }
                    line = lines.next_line() => {
                        match line {
                            Ok(Some(line)) => {
                                let ev = match decode_line::<Reply>(&line) {
                                    Ok(Reply::Output { data_b64, .. }) => B64
                                        .decode(data_b64.as_bytes())
                                        .map(TermEvent::Output)
                                        .unwrap_or(TermEvent::Disconnected),
                                    Ok(Reply::AgentEvent { agent, state, transcript_path, .. }) => {
                                        TermEvent::Agent { agent, state, transcript_path }
                                    }
                                    Ok(Reply::Exited { code, .. }) => TermEvent::Exited(code),
                                    Ok(Reply::Error { message }) if message.contains("lagged") =>
                                        TermEvent::Lagged,
                                    _ => continue,
                                };
                                let stop = matches!(ev, TermEvent::Exited(_) | TermEvent::Disconnected);
                                if tx.send(ev).is_err() || stop {
                                    tracing::debug!(session_id = %id, "attach 读任务退出（发送失败或会话结束）");
                                    break;
                                }
                            }
                            _ => {
                                tracing::debug!(session_id = %id, "attach 读任务退出（daemon 断开）");
                                let _ = tx.send(TermEvent::Disconnected);
                                break;
                            }
                        }
                    }
                }
            }
        });
        Ok((snapshot, next, rx))
    }
}

impl SessionService for Client {
    fn list_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionInfo>, ServiceError>> {
        Box::pin(async move { self.list().await.map_err(into_service_error) })
    }

    fn create(&self, spec: SessionSpec) -> BoxFuture<'_, Result<SessionInfo, ServiceError>> {
        let SessionSpec {
            name,
            command,
            args,
            cwd,
            cols,
            rows,
            project_id,
            agent,
        } = spec;
        Box::pin(async move {
            self.create_for_agent(&name, &command, &args, &cwd, cols, rows, project_id, agent)
                .await
                .map_err(into_service_error)
        })
    }

    fn attach(
        &self,
        id: String,
        from_offset: u64,
    ) -> BoxFuture<'_, Result<(Vec<u8>, u64, mpsc::UnboundedReceiver<TermEvent>), ServiceError>>
    {
        Box::pin(async move {
            self.attach(&id, from_offset)
                .await
                .map_err(into_service_error)
        })
    }

    fn write(&self, id: String, data: Vec<u8>) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move { self.write(&id, &data).await.map_err(into_service_error) })
    }

    fn resize(&self, id: String, cols: u16, rows: u16) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            self.resize(&id, cols, rows)
                .await
                .map_err(into_service_error)
        })
    }

    fn kill(&self, id: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move { self.kill(&id).await.map_err(into_service_error) })
    }

    fn close_with_summary(&self, id: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            self.close_with_summary(&id)
                .await
                .map_err(into_service_error)
        })
    }
}

/// 经 UDS 连到 dozerd 的 [`AppHostApi`] 实现。失败映射:dozerd 回的
/// `AppReply::Failed` 是 `Host`(带类别,含"宿主不可用"),其余连接/协议问题
/// (`Reply::Error`、意外变体、连不上)是 `Transport`。
impl AppHostApi for Client {
    async fn request(&self, request: AppRequest) -> std::result::Result<AppReply, AppApiError> {
        match self
            .roundtrip(&Request::App { request })
            .await
            .map_err(|e| AppApiError::Transport(format!("{e:#}")))?
        {
            Reply::App {
                reply: AppReply::Failed { failure },
            } => Err(AppApiError::Host(failure)),
            Reply::App { reply } => Ok(reply),
            other => Err(AppApiError::Transport(format!("意外应答: {other:?}"))),
        }
    }

    async fn subscribe(
        &self,
    ) -> std::result::Result<mpsc::UnboundedReceiver<AppChange>, AppApiError> {
        self.ensure_version_checked()
            .await
            .map_err(|e| AppApiError::Transport(format!("{e:#}")))?;
        let stream = UnixStream::connect(&self.socket)
            .await
            .map_err(|e| AppApiError::Transport(format!("{e}")))?;
        let (r, mut w) = stream.into_split();
        // `Subscribe` 不是一问一答:dozerd 在同一连接上先回 `Subscribed`,之后持续推 `Changed`/`Resync`。
        let request = encode_line(&Request::App {
            request: AppRequest::Subscribe,
        });
        if let Err(e) = w.write_all(request.as_bytes()).await {
            return Err(AppApiError::Transport(format!("{e}")));
        }
        let mut lines = BufReader::new(r).lines();
        let first = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => return Err(AppApiError::Transport("daemon 断开".into())),
            Err(e) => return Err(AppApiError::Transport(format!("{e}"))),
        };
        match decode_line::<Reply>(&first) {
            Ok(Reply::App {
                reply: AppReply::Subscribed,
            }) => {}
            Ok(Reply::App {
                reply: AppReply::Failed { failure },
            }) => return Err(AppApiError::Host(failure)),
            Ok(Reply::Error { message }) => {
                return Err(AppApiError::Transport(format!("订阅失败: {message}")));
            }
            Ok(other) => return Err(AppApiError::Transport(format!("意外应答: {other:?}"))),
            Err(e) => return Err(AppApiError::Transport(format!("{e}"))),
        }
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let _keep_writer = w;
            loop {
                tokio::select! {
                    // 接收端被丢弃(例如面板不再需要订阅):停读循环,连接两端随后关闭,
                    // daemon 侧 handle_conn 才能在下一次 next_line() 收到 EOF 并退出。
                    _ = tx.closed() => break,
                    line = lines.next_line() => {
                        match line {
                            Ok(Some(line)) => {
                                let change = match decode_line::<Reply>(&line) {
                                    Ok(Reply::App { reply: AppReply::Changed { app } }) => {
                                        AppChange::Changed(app)
                                    }
                                    Ok(Reply::App { reply: AppReply::Resync }) => AppChange::Resync,
                                    Ok(Reply::Error { message }) if message.contains("lagged") => {
                                        AppChange::Resync
                                    }
                                    // 其余(含一次行的 `Subscribed` 回显)忽略。
                                    _ => continue,
                                };
                                if tx.send(change).is_err() {
                                    break;
                                }
                            }
                            _ => {
                                let _ = tx.send(AppChange::Disconnected);
                                break;
                            }
                        }
                    }
                }
            }
        });
        Ok(rx)
    }
}

/// `shutdown_daemon()` 的超时/协议错误/成功三分支映射,拆成纯函数是为了
/// 不用真的等 90s 或起一个假 UDS server 就能测到每条分支(`Client`::
/// `shutdown_daemon` 本身只做一次 `tokio::time::timeout` 包裹,逻辑全在
/// 这里)。
fn interpret_shutdown_reply(
    outcome: std::result::Result<Result<Reply>, tokio::time::error::Elapsed>,
) -> Result<()> {
    match outcome {
        Err(_) => Err(anyhow!("等待 dozerd 停止超时")),
        Ok(Err(e)) => Err(e),
        Ok(Ok(Reply::Ok)) => Ok(()),
        Ok(Ok(other)) => Err(anyhow!("意外应答: {other:?}")),
    }
}

/// 把 `Client` 内部的 `anyhow::Error` 归类成面板可区分的 `ServiceError`:带类型的原样返回,
/// IO 错误(连不上/读写断开)与握手的 `Unreachable`(守护进程不在/断开)归
/// `Unavailable`,其余(版本/产品不一致、意外应答、解码失败)归 `Protocol`。
pub fn into_service_error(e: anyhow::Error) -> ServiceError {
    match e.downcast::<ServiceError>() {
        Ok(se) => se,
        Err(e) if e.downcast_ref::<std::io::Error>().is_some() => {
            ServiceError::new(ServiceErrorKind::Unavailable, e.to_string())
        }
        Err(e)
            if e.downcast_ref::<ClientError>()
                .is_some_and(|ce| matches!(ce.kind, ClientErrorKind::Unreachable(_))) =>
        {
            ServiceError::new(ServiceErrorKind::Unavailable, e.to_string())
        }
        Err(e) => ServiceError::new(ServiceErrorKind::Protocol, e.to_string()),
    }
}

impl UsageService for Client {
    fn usage_summary(
        &self,
        cwd: String,
        since_ts: Option<u64>,
    ) -> BoxFuture<'_, Result<UsageRows, ServiceError>> {
        Box::pin(async move {
            self.get_usage_summary(&cwd, since_ts)
                .await
                .map_err(into_service_error)
        })
    }
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;

    #[test]
    fn interpret_shutdown_reply_ok_maps_to_success() {
        let outcome: std::result::Result<Result<Reply>, tokio::time::error::Elapsed> =
            Ok(Ok(Reply::Ok));
        assert!(interpret_shutdown_reply(outcome).is_ok());
    }

    #[test]
    fn interpret_shutdown_reply_unexpected_reply_is_error() {
        let outcome: std::result::Result<Result<Reply>, tokio::time::error::Elapsed> =
            Ok(Ok(Reply::Sessions { sessions: vec![] }));
        assert!(interpret_shutdown_reply(outcome).is_err());
    }

    #[test]
    fn interpret_shutdown_reply_inner_error_propagates() {
        let outcome: std::result::Result<Result<Reply>, tokio::time::error::Elapsed> =
            Ok(Err(anyhow!("daemon 错误: dozerd 正在停止中")));
        let err = interpret_shutdown_reply(outcome).unwrap_err();
        assert!(err.to_string().contains("正在停止中"));
    }

    #[tokio::test]
    async fn interpret_shutdown_reply_timeout_is_error() {
        let elapsed = tokio::time::timeout(
            std::time::Duration::from_millis(0),
            std::future::pending::<Result<Reply>>(),
        )
        .await
        .unwrap_err();
        let err = interpret_shutdown_reply(Err(elapsed)).unwrap_err();
        assert!(err.to_string().contains("超时"));
    }
}

#[cfg(test)]
mod app_subscribe_tests {
    use super::*;
    use bytehost_apps::id::AppId;

    fn temp_sock(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "dz-app-sub-{tag}-{}.sock",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// 假服务端:读到 `Subscribe` 后回 `Subscribed`,再推 2 条 `Changed`,然后关连接。
    async fn fake_server(listener: tokio::net::UnixListener) {
        // 握手是第一条连接;订阅流是下一条。先应答握手,再 accept 订阅流。
        let (stream, _) = listener.accept().await.unwrap();
        let (r, mut w) = stream.into_split();
        let mut lines = BufReader::new(r).lines();
        let _ = lines.next_line().await.unwrap();
        let hello = Reply::Hello {
            protocol: dozer_core::protocol::PROTOCOL_VERSION,
            product: dozer_core::paths::product().name.to_string(),
        };
        w.write_all(encode_line(&hello).as_bytes()).await.unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let (r, mut w) = stream.into_split();
        let mut lines = BufReader::new(r).lines();
        let _ = lines.next_line().await.unwrap();
        w.write_all(
            encode_line(&Reply::App {
                reply: AppReply::Subscribed,
            })
            .as_bytes(),
        )
        .await
        .unwrap();
        for name in ["a", "b"] {
            let reply = Reply::App {
                reply: AppReply::Changed {
                    app: AppId::new(name).unwrap(),
                },
            };
            w.write_all(encode_line(&reply).as_bytes()).await.unwrap();
        }
        drop(w);
    }

    #[tokio::test]
    async fn app_subscribe_yields_changes_then_disconnected() {
        let sock = temp_sock("ok");
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        let server = tokio::spawn(fake_server(listener));

        let mut rx = Client::new(sock.clone()).subscribe().await.unwrap();
        assert_eq!(
            rx.recv().await,
            Some(AppChange::Changed(AppId::new("a").unwrap()))
        );
        assert_eq!(
            rx.recv().await,
            Some(AppChange::Changed(AppId::new("b").unwrap()))
        );
        assert_eq!(rx.recv().await, Some(AppChange::Disconnected));
        assert_eq!(rx.recv().await, None);
        server.await.unwrap();
        let _ = std::fs::remove_file(&sock);
    }

    /// 接收端被丢弃后读任务必须退出(否则连接与 fd 滞留):服务端只发第一条
    /// `Changed` 后就什么都不做,客户端 drop 接收端,随后读任务应因 `tx.closed()` 退出——
    /// 表现为服务端下一行读取得到 EOF(连接被关)。
    #[tokio::test]
    async fn dropping_the_receiver_closes_the_connection() {
        let sock = temp_sock("drop");
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        let server = tokio::spawn(async move {
            // 先应答握手(第一条连接)
            {
                let (stream, _) = listener.accept().await.unwrap();
                let (r, mut w) = stream.into_split();
                let mut lines = BufReader::new(r).lines();
                let _ = lines.next_line().await.unwrap();
                let hello = Reply::Hello {
                    protocol: dozer_core::protocol::PROTOCOL_VERSION,
                    product: dozer_core::paths::product().name.to_string(),
                };
                w.write_all(encode_line(&hello).as_bytes()).await.unwrap();
            }
            // 再走订阅流
            let (stream, _) = listener.accept().await.unwrap();
            let (r, mut w) = stream.into_split();
            let mut lines = BufReader::new(r).lines();
            let _ = lines.next_line().await.unwrap();
            w.write_all(
                encode_line(&Reply::App {
                    reply: AppReply::Subscribed,
                })
                .as_bytes(),
            )
            .await
            .unwrap();
            // 客户端此后会 drop 接收端;连接应最终 EOF。
            let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
                .await
                .expect("读任务应在丢弃接收端后退出")
                .unwrap();
            assert!(line.is_none(), "连接应被关闭(EOF),得到 {line:?}");
        });

        let rx = Client::new(sock.clone()).subscribe().await.unwrap();
        drop(rx);
        server.await.unwrap();
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn app_subscribe_propagates_a_failed_reply() {
        let sock = temp_sock("fail");
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        tokio::spawn(async move {
            // 先应答握手(第一条连接)
            {
                let (stream, _) = listener.accept().await.unwrap();
                let (r, mut w) = stream.into_split();
                let mut lines = BufReader::new(r).lines();
                let _ = lines.next_line().await.unwrap();
                let hello = Reply::Hello {
                    protocol: dozer_core::protocol::PROTOCOL_VERSION,
                    product: dozer_core::paths::product().name.to_string(),
                };
                w.write_all(encode_line(&hello).as_bytes()).await.unwrap();
            }
            // 再走订阅流
            let (stream, _) = listener.accept().await.unwrap();
            let (r, mut w) = stream.into_split();
            let mut lines = BufReader::new(r).lines();
            let _ = lines.next_line().await.unwrap();
            let failure = bytehost_apps::proto::AppFailure::new(
                bytehost_apps::proto::AppErrorKind::Unavailable,
                "宿主不可用",
            );
            w.write_all(
                encode_line(&Reply::App {
                    reply: AppReply::Failed { failure },
                })
                .as_bytes(),
            )
            .await
            .unwrap();
        });

        let err = Client::new(sock.clone()).subscribe().await.unwrap_err();
        assert_eq!(
            err.kind(),
            Some(bytehost_apps::proto::AppErrorKind::Unavailable)
        );
        let _ = std::fs::remove_file(&sock);
    }
}

impl TodoService for Client {
    fn list_todos(&self, project_id: i64) -> BoxFuture<'_, Result<Vec<TodoInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_todos(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn list_categories(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<CategoryInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_categories(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn add_todo(&self, project_id: i64, text: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::add_todo(self, project_id, &text)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn toggle_todo(&self, id: i64, done: bool) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::toggle_todo(self, id, done)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn edit_todo_text(&self, id: i64, text: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::edit_todo_text(self, id, &text)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn reorder_todo(
        &self,
        id: i64,
        after_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::reorder_todo(self, id, after_id)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn set_todo_plan_date(
        &self,
        id: i64,
        plan_date: Option<String>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::set_todo_plan_date(self, id, plan_date.as_deref())
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn set_todo_status(
        &self,
        id: i64,
        status: dozer_core::protocol::TodoStoredStatus,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::set_todo_status(self, id, status)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn set_todo_category(
        &self,
        id: i64,
        category_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::set_todo_category(self, id, category_id)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn add_category(
        &self,
        project_id: i64,
        parent_id: Option<i64>,
        name: String,
    ) -> BoxFuture<'_, Result<CategoryInfo, ServiceError>> {
        Box::pin(async move {
            Client::add_category(self, project_id, parent_id, &name)
                .await
                .map_err(into_service_error)
        })
    }
    fn rename_category(&self, id: i64, name: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::rename_category(self, id, &name)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn delete_category(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::delete_category(self, id)
                .await
                .map_err(into_service_error)
        })
    }
    fn reparent_category(
        &self,
        id: i64,
        new_parent_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::reparent_category(self, id, new_parent_id)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn move_category_sibling(
        &self,
        id: i64,
        direction: CategoryMoveDirection,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::move_category_sibling(self, id, direction)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
}

impl MemoryService for Client {
    fn list_memories(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::MemoryInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_memories(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn get_memory(
        &self,
        project_id: i64,
        id: i64,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::MemoryDetail, ServiceError>> {
        Box::pin(async move {
            Client::get_memory(self, project_id, id)
                .await
                .map_err(into_service_error)
        })
    }
    fn write_memory(
        &self,
        project_id: i64,
        title: String,
        kind: String,
        description: String,
        body: String,
        actor: String,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::MemoryDetail, ServiceError>> {
        Box::pin(async move {
            Client::write_memory(self, project_id, &title, &kind, &description, &body, &actor)
                .await
                .map_err(into_service_error)
        })
    }
    fn delete_memory(
        &self,
        project_id: i64,
        id: i64,
        actor: String,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::delete_memory(self, project_id, id, &actor)
                .await
                .map_err(into_service_error)
        })
    }
}

impl ProjectService for Client {
    fn rename_project(
        &self,
        id: i64,
        name: String,
    ) -> BoxFuture<'_, Result<Option<dozer_core::protocol::ProjectInfo>, ServiceError>> {
        Box::pin(async move {
            Client::rename_project(self, id, &name)
                .await
                .map_err(into_service_error)
        })
    }
    fn remove_project(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::remove_project(self, id)
                .await
                .map_err(into_service_error)
        })
    }
    fn open_project(
        &self,
        path: String,
    ) -> BoxFuture<'_, Result<Option<dozer_core::protocol::ProjectInfo>, ServiceError>> {
        Box::pin(async move {
            Client::open_project(self, &path)
                .await
                .map_err(into_service_error)
        })
    }
    fn list_projects(
        &self,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::ProjectInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_projects(self)
                .await
                .map_err(into_service_error)
        })
    }
}

impl TranscriptService for Client {
    fn delete_project_transcripts(&self, cwd: String) -> BoxFuture<'_, Result<u32, ServiceError>> {
        Box::pin(async move {
            Client::delete_project_transcripts(self, &cwd)
                .await
                .map_err(into_service_error)
        })
    }
    fn backfill_project_transcripts(
        &self,
        cwd: String,
    ) -> BoxFuture<'_, Result<u32, ServiceError>> {
        Box::pin(async move {
            Client::backfill_project_transcripts(self, &cwd)
                .await
                .map_err(into_service_error)
        })
    }
    fn submit_summary_batch(
        &self,
        cwd: String,
        provider: Option<dozer_core::protocol::AgentKind>,
        model: Option<String>,
    ) -> BoxFuture<'_, Result<(i64, u32), ServiceError>> {
        Box::pin(async move {
            Client::submit_summary_batch(self, &cwd, provider, model.as_deref())
                .await
                .map_err(into_service_error)
        })
    }
    fn get_summary_batch(
        &self,
        batch_id: i64,
    ) -> BoxFuture<'_, Result<Option<dozer_core::protocol::SummaryBatchInfo>, ServiceError>> {
        Box::pin(async move {
            Client::get_summary_batch(self, batch_id)
                .await
                .map_err(into_service_error)
        })
    }
}

impl GroupChatService for Client {
    fn list_groups(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::GroupInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_groups(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn list_group_messages(
        &self,
        group_id: i64,
        after_rev: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<(Vec<dozer_core::protocol::GroupMessageInfo>, i64), ServiceError>>
    {
        Box::pin(async move {
            Client::list_group_messages(self, group_id, after_rev, limit)
                .await
                .map_err(into_service_error)
        })
    }
    fn create_group(
        &self,
        project_id: i64,
        topic: String,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::GroupInfo, ServiceError>> {
        Box::pin(async move {
            Client::create_group(self, project_id, &topic)
                .await
                .map_err(into_service_error)
        })
    }
    fn delete_group(&self, group_id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::delete_group(self, group_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn add_group_member(
        &self,
        group_id: i64,
        agent: dozer_core::protocol::AgentKind,
        handle: String,
        role_prompt: String,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::GroupInfo, ServiceError>> {
        Box::pin(async move {
            Client::add_group_member(self, group_id, agent, &handle, &role_prompt)
                .await
                .map_err(into_service_error)
        })
    }
    fn update_group_member(
        &self,
        member_id: i64,
        handle: String,
        role_prompt: String,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::GroupInfo, ServiceError>> {
        Box::pin(async move {
            Client::update_group_member(self, member_id, &handle, &role_prompt)
                .await
                .map_err(into_service_error)
        })
    }
    fn remove_group_member(
        &self,
        member_id: i64,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::GroupInfo, ServiceError>> {
        Box::pin(async move {
            Client::remove_group_member(self, member_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn post_group_message(
        &self,
        group_id: i64,
        text: String,
    ) -> BoxFuture<'_, Result<dozer_services::PostedMessages, ServiceError>> {
        Box::pin(async move {
            Client::post_group_message(self, group_id, &text)
                .await
                .map_err(into_service_error)
        })
    }
    fn cancel_group(
        &self,
        group_id: i64,
        scope: dozer_core::protocol::GroupCancelScope,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::cancel_group(self, group_id, scope)
                .await
                .map_err(into_service_error)
        })
    }
    fn retry_group_message(
        &self,
        message_id: i64,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::GroupMessageInfo, ServiceError>> {
        Box::pin(async move {
            Client::retry_group_message(self, message_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn push_group_message_to_todo(
        &self,
        message_id: i64,
        text: String,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::TodoInfo, ServiceError>> {
        Box::pin(async move {
            Client::push_group_message_to_todo(self, message_id, &text)
                .await
                .map_err(into_service_error)
        })
    }
}

impl ConversationService for Client {
    fn list_conversations_with_summaries(
        &self,
        cwd: String,
        agent: Option<dozer_core::protocol::AgentKind>,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<ConversationRows, ServiceError>> {
        Box::pin(async move {
            Client::list_conversations_with_summaries(self, &cwd, agent, limit, offset)
                .await
                .map_err(into_service_error)
        })
    }
}

impl CodeHealthService for Client {
    fn get_code_health_report(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Option<dozer_core::protocol::CodeHealthReportInfo>, ServiceError>>
    {
        Box::pin(async move {
            Client::get_code_health_report(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn list_code_health_reports(
        &self,
        project_id: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::CodeHealthReportInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_code_health_reports(self, project_id, limit)
                .await
                .map_err(into_service_error)
        })
    }
    fn save_code_health_report(
        &self,
        project_id: i64,
        info: dozer_core::protocol::CodeHealthReportInfo,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::save_code_health_report(self, project_id, &info)
                .await
                .map_err(into_service_error)
        })
    }
}

impl ContextService for Client {
    fn list_context_items(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::ContextItemInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_context_items(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn add_context_item(
        &self,
        project_id: i64,
        entity_kind: String,
        entity_ref: String,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::ContextItemInfo, ServiceError>> {
        Box::pin(async move {
            Client::add_context_item(self, project_id, &entity_kind, &entity_ref)
                .await
                .map_err(into_service_error)
        })
    }
    fn remove_context_item(
        &self,
        project_id: i64,
        id: i64,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::remove_context_item(self, project_id, id)
                .await
                .map_err(into_service_error)
        })
    }
}

impl EditHistoryService for Client {
    fn list_file_edit_history(
        &self,
        project_id: i64,
        path_filter: Option<String>,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::FileEditHistoryInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_file_edit_history(self, project_id, path_filter.as_deref(), limit)
                .await
                .map_err(into_service_error)
        })
    }
    fn apply_precise_edit(
        &self,
        edit: PreciseEdit,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::MutationOutcome, ServiceError>> {
        Box::pin(async move {
            let PreciseEdit {
                project_id,
                path,
                start_line,
                start_col,
                end_line,
                end_col,
                expected_text,
                new_text,
                summary,
                actor,
                session_id,
            } = edit;
            Client::apply_precise_edit(
                self,
                project_id,
                &path,
                start_line,
                start_col,
                end_line,
                end_col,
                &expected_text,
                &new_text,
                &summary,
                &actor,
                &session_id,
            )
            .await
            .map_err(into_service_error)
        })
    }
}

impl BookmarkService for Client {
    fn list_bookmarks(
        &self,
        project_id: Option<i64>,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::BookmarkInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_bookmarks(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn add_bookmark(
        &self,
        scope: dozer_core::protocol::BookmarkScope,
        project_id: Option<i64>,
        url: String,
        title: String,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::add_bookmark(self, scope, project_id, &url, &title)
                .await
                .map_err(into_service_error)
        })
    }
    fn remove_bookmark(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::remove_bookmark(self, id)
                .await
                .map_err(into_service_error)
        })
    }
}

impl LocaleService for Client {
    fn get_locale(&self) -> BoxFuture<'_, Result<LocaleSnapshot, ServiceError>> {
        Box::pin(async move { Client::get_locale(self).await.map_err(into_service_error) })
    }
    fn set_locale(
        &self,
        preference: LocalePreference,
        expected_revision: u64,
        system_candidates: Vec<String>,
    ) -> BoxFuture<'_, Result<LocaleWriteOutcome, ServiceError>> {
        Box::pin(async move {
            Client::set_locale(self, preference, expected_revision, system_candidates)
                .await
                .map_err(into_service_error)
        })
    }
}

#[cfg(test)]
mod service_error_tests {
    use super::*;
    use dozer_services::{ServiceError, ServiceErrorKind};

    #[test]
    fn io_errors_are_unavailable() {
        let e = anyhow::Error::from(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "refused",
        ));
        assert_eq!(into_service_error(e).kind, ServiceErrorKind::Unavailable);
    }

    #[test]
    fn typed_service_errors_pass_through_with_the_same_text() {
        let e = anyhow::Error::new(ServiceError::new(
            ServiceErrorKind::Rejected,
            "daemon 错误: boom",
        ));
        let s = into_service_error(e);
        assert_eq!(
            (s.kind, s.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
    }

    #[test]
    fn anything_else_is_protocol() {
        let s = into_service_error(anyhow::anyhow!("意外应答: Ok"));
        assert_eq!(s.kind, ServiceErrorKind::Protocol);
        assert_eq!(s.message, "意外应答: Ok");
    }
}

#[cfg(test)]
mod todo_service_tests {
    use super::*;
    use dozer_services::{ServiceErrorKind, TodoService};

    pub(super) fn temp_sock(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "dz-todo-svc-{tag}-{}.sock",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// 先应答协议握手(Hello),再读一行业务请求回 `reply`;`None` =
    /// 业务请求不应答直接挂断(握手仍正常放行,这样"挂断"只影响业务
    /// 请求,语义与旧用例一致)。
    pub(super) fn serve_once(tag: &str, reply: Option<Reply>) -> PathBuf {
        let sock = temp_sock(tag);
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        tokio::spawn(async move {
            // 每个业务调用都是一条新连接(握手一条 + 业务一条),假服务端
            // 必须持续 accept;且要按请求类型应答:Hello 请求回同版本握手,
            // 业务请求回配置的 reply(否则业务连接会把握手应答错当业务应答)。
            while let Ok((stream, _)) = listener.accept().await {
                let reply = reply.clone();
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut lines = BufReader::new(r).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let is_hello = serde_json::from_str::<serde_json::Value>(&line)
                            .ok()
                            .and_then(|v| {
                                v.get("type").and_then(|t| t.as_str()).map(|t| t == "hello")
                            })
                            .unwrap_or(false);
                        let out = if is_hello {
                            Reply::Hello {
                                protocol: dozer_core::protocol::PROTOCOL_VERSION,
                                product: dozer_core::paths::product().name.to_string(),
                            }
                        } else {
                            match reply.clone() {
                                Some(reply) => reply,
                                // None = 业务请求不应答直接挂断(旧语义)
                                None => return,
                            }
                        };
                        let _ = w.write_all(encode_line(&out).as_bytes()).await;
                    }
                });
            }
        });
        sock
    }

    #[tokio::test]
    async fn list_todos_comes_back_as_ok() {
        let sock = serve_once("list", Some(Reply::Todos { todos: vec![] }));
        let got = TodoService::list_todos(&Client::new(sock.clone()), 7).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn a_write_returns_unit_on_success() {
        let sock = serve_once("write", Some(Reply::Ok));
        let got = TodoService::delete_category(&Client::new(sock.clone()), 3).await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("hangup", None);
        let e = TodoService::list_todos(&Client::new(sock.clone()), 7)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once(
            "reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = TodoService::delete_category(&Client::new(sock.clone()), 3)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod locale_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_core::protocol::Locale;
    use dozer_services::{LocaleService, ServiceErrorKind};

    fn snap(rev: u64, loc: &str) -> LocaleSnapshot {
        LocaleSnapshot {
            schema_version: 1,
            preference: LocalePreference::Explicit {
                locale: Locale(loc.into()),
            },
            active_locale: Locale(loc.into()),
            revision: rev,
        }
    }

    #[tokio::test]
    async fn get_locale_comes_back_as_snapshot() {
        let sock = serve_once(
            "locale-get",
            Some(Reply::Locale {
                snapshot: snap(2, "zh-CN"),
            }),
        );
        let got = LocaleService::get_locale(&Client::new(sock.clone())).await;
        assert_eq!(got, Ok(snap(2, "zh-CN")));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn set_locale_returns_applied_outcome() {
        let sock = serve_once(
            "locale-set",
            Some(Reply::LocaleWrite {
                outcome: LocaleWriteOutcome::Applied {
                    snapshot: snap(1, "zh-CN"),
                },
            }),
        );
        let got = LocaleService::set_locale(
            &Client::new(sock.clone()),
            LocalePreference::Explicit {
                locale: Locale("zh-CN".into()),
            },
            0,
            vec![],
        )
        .await;
        assert_eq!(
            got,
            Ok(LocaleWriteOutcome::Applied {
                snapshot: snap(1, "zh-CN")
            })
        );
        let _ = std::fs::remove_file(&sock);
    }

    /// 旧 daemon 不认识 `GetLocale`,回 `Reply::Error` → 客户端分类为 `Rejected`,
    /// 不无限重试、不 panic。
    #[tokio::test]
    async fn old_daemon_error_reply_is_rejected() {
        let sock = serve_once(
            "locale-old",
            Some(Reply::Error {
                message: "协议错误: unknown variant `get_locale`".into(),
            }),
        );
        let e = LocaleService::get_locale(&Client::new(sock.clone()))
            .await
            .unwrap_err();
        assert_eq!(e.kind, ServiceErrorKind::Rejected);
        let _ = std::fs::remove_file(&sock);
    }

    /// 旧 daemon 直接断连 → `Unavailable`。
    #[tokio::test]
    async fn old_daemon_hang_up_is_unavailable() {
        let sock = serve_once("locale-hangup", None);
        let e = LocaleService::get_locale(&Client::new(sock.clone()))
            .await
            .unwrap_err();
        assert_eq!(e.kind, ServiceErrorKind::Unavailable);
        let _ = std::fs::remove_file(&sock);
    }

    /// 意外应答形状 → `Protocol`,不当作空结果吞掉。
    #[tokio::test]
    async fn unexpected_reply_is_protocol() {
        let sock = serve_once("locale-wrong", Some(Reply::Ok));
        let e = LocaleService::get_locale(&Client::new(sock.clone()))
            .await
            .unwrap_err();
        assert_eq!(e.kind, ServiceErrorKind::Protocol);
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod project_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{MemoryService, ProjectService, ServiceErrorKind, TranscriptService};
    #[tokio::test]
    async fn list_memories_comes_back_as_ok() {
        let sock = serve_once("mem-list", Some(Reply::Memories { memories: vec![] }));
        let got = MemoryService::list_memories(&Client::new(sock.clone()), 7).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn a_unit_write_returns_unit_on_success() {
        let sock = serve_once("proj-remove", Some(Reply::Ok));
        let got = ProjectService::remove_project(&Client::new(sock.clone()), 3).await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn a_count_is_carried_back_unchanged() {
        let sock = serve_once(
            "proj-backfill",
            Some(Reply::BackfillDone { imported_files: 3 }),
        );
        let got = TranscriptService::backfill_project_transcripts(
            &Client::new(sock.clone()),
            "/p".into(),
        )
        .await;
        assert_eq!(got, Ok(3));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("proj-hangup", None);
        let e = MemoryService::list_memories(&Client::new(sock.clone()), 7)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once(
            "proj-reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = ProjectService::remove_project(&Client::new(sock.clone()), 3)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }

    fn project_info(id: i64, path: &str) -> dozer_core::protocol::ProjectInfo {
        dozer_core::protocol::ProjectInfo {
            id,
            path: path.into(),
            name: "demo".into(),
            last_active_ms: 1,
            created_ms: 2,
            updated_ms: 3,
        }
    }

    #[tokio::test]
    async fn open_project_carries_some_and_none_back_unchanged() {
        let info = project_info(5, "/p/demo");
        let sock = serve_once(
            "pc-open-some",
            Some(Reply::Project {
                project: Some(info.clone()),
            }),
        );
        let got = ProjectService::open_project(&Client::new(sock.clone()), "/p/demo".into()).await;
        assert_eq!(got, Ok(Some(info)));
        let _ = std::fs::remove_file(&sock);

        let sock = serve_once("pc-open-none", Some(Reply::Project { project: None }));
        let got = ProjectService::open_project(&Client::new(sock.clone()), "/p/none".into()).await;
        assert_eq!(got, Ok(None));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn list_projects_comes_back_as_ok() {
        let info = project_info(5, "/p/demo");
        let sock = serve_once(
            "pc-list",
            Some(Reply::Projects {
                projects: vec![info.clone()],
            }),
        );
        let got = ProjectService::list_projects(&Client::new(sock.clone())).await;
        assert_eq!(got, Ok(vec![info]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn new_methods_map_hang_up_and_error_reply_like_the_others() {
        let sock = serve_once("pc-hangup", None);
        let e = ProjectService::list_projects(&Client::new(sock.clone()))
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);

        let sock = serve_once(
            "pc-reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = ProjectService::open_project(&Client::new(sock.clone()), "/p".into())
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod group_chat_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{GroupChatService, ServiceErrorKind};

    #[tokio::test]
    async fn list_groups_comes_back_as_ok() {
        let sock = serve_once("gc-list", Some(Reply::Groups { groups: vec![] }));
        let got = GroupChatService::list_groups(&Client::new(sock.clone()), 7).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn a_unit_write_returns_unit_on_success() {
        let sock = serve_once("gc-delete", Some(Reply::Ok));
        let got = GroupChatService::delete_group(&Client::new(sock.clone()), 3).await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("gc-hangup", None);
        let e = GroupChatService::list_groups(&Client::new(sock.clone()), 7)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once(
            "gc-reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = GroupChatService::delete_group(&Client::new(sock.clone()), 3)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn a_tuple_result_is_carried_back_unchanged() {
        let sock = serve_once(
            "gc-msgs",
            Some(Reply::GroupMessages {
                messages: vec![],
                latest_rev: 42,
            }),
        );
        let got =
            GroupChatService::list_group_messages(&Client::new(sock.clone()), 5, 3, 200).await;
        assert_eq!(got, Ok((vec![], 42)));
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod conversation_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{ConversationService, ServiceErrorKind};

    #[tokio::test]
    async fn rows_come_back_as_ok() {
        let sock = serve_once(
            "conv-ok",
            Some(Reply::ConversationsWithSummaries { rows: vec![] }),
        );
        let got = ConversationService::list_conversations_with_summaries(
            &Client::new(sock.clone()),
            "/p".into(),
            None,
            500,
            0,
        )
        .await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("conv-hangup", None);
        let e = ConversationService::list_conversations_with_summaries(
            &Client::new(sock.clone()),
            "/p".into(),
            None,
            500,
            0,
        )
        .await
        .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once(
            "conv-reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = ConversationService::list_conversations_with_summaries(
            &Client::new(sock.clone()),
            "/p".into(),
            None,
            500,
            0,
        )
        .await
        .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod code_health_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{CodeHealthService, ServiceErrorKind};

    #[tokio::test]
    async fn get_returns_the_option_unchanged() {
        let sock = serve_once("ch-get", Some(Reply::CodeHealthReport { report: None }));
        let got = CodeHealthService::get_code_health_report(&Client::new(sock.clone()), 7).await;
        assert_eq!(got, Ok(None));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn list_comes_back_as_ok() {
        let sock = serve_once(
            "ch-list",
            Some(Reply::CodeHealthReports { reports: vec![] }),
        );
        let got =
            CodeHealthService::list_code_health_reports(&Client::new(sock.clone()), 7, 2).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("ch-hangup", None);
        let e = CodeHealthService::get_code_health_report(&Client::new(sock.clone()), 7)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once(
            "ch-reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = CodeHealthService::list_code_health_reports(&Client::new(sock.clone()), 7, 2)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod context_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{ContextService, ServiceErrorKind};

    #[tokio::test]
    async fn list_comes_back_as_ok() {
        let sock = serve_once("ctx-list", Some(Reply::ContextItems { items: vec![] }));
        let got = ContextService::list_context_items(&Client::new(sock.clone()), 7).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn remove_returns_unit_on_success() {
        let sock = serve_once("ctx-remove", Some(Reply::Ok));
        let got = ContextService::remove_context_item(&Client::new(sock.clone()), 7, 3).await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("ctx-hangup", None);
        let e = ContextService::list_context_items(&Client::new(sock.clone()), 7)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once(
            "ctx-reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = ContextService::remove_context_item(&Client::new(sock.clone()), 7, 3)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn add_carries_the_item_back_unchanged() {
        let item = dozer_core::protocol::ContextItemInfo {
            id: 5,
            project_id: 7,
            entity_kind: "file".into(),
            entity_ref: "src/a.rs".into(),
            created_ms: 9,
        };
        let sock = serve_once("ctx-add", Some(Reply::ContextItem { item: item.clone() }));
        let got = ContextService::add_context_item(
            &Client::new(sock.clone()),
            7,
            "file".into(),
            "src/a.rs".into(),
        )
        .await;
        assert_eq!(got, Ok(item));
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod edit_history_service_tests {
    use super::todo_service_tests::{serve_once, temp_sock};
    use super::*;
    use dozer_core::protocol::MutationOutcome;
    use dozer_services::{EditHistoryService, PreciseEdit, ServiceErrorKind};
    use std::sync::{Arc, Mutex};

    /// 读一行业务请求,记下它,再回 `reply`。返回 (socket 路径, 记录请求行的共享槽)。
    /// 复审 R2:客户端先握手再发业务请求(两条连接),这里先应答握手。
    fn serve_capture(tag: &str, reply: Reply) -> (PathBuf, Arc<Mutex<Option<String>>>) {
        let sock = temp_sock(tag);
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        let seen = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&seen);
        tokio::spawn(async move {
            // 握手连接
            {
                let (stream, _) = listener.accept().await.unwrap();
                let (r, mut w) = stream.into_split();
                let mut lines = BufReader::new(r).lines();
                let _ = lines.next_line().await;
                let hello = Reply::Hello {
                    protocol: dozer_core::protocol::PROTOCOL_VERSION,
                    product: dozer_core::paths::product().name.to_string(),
                };
                let _ = w.write_all(encode_line(&hello).as_bytes()).await;
            }
            // 业务连接
            let (stream, _) = listener.accept().await.unwrap();
            let (r, mut w) = stream.into_split();
            let mut lines = BufReader::new(r).lines();
            if let Ok(Some(line)) = lines.next_line().await {
                *slot.lock().unwrap() = Some(line);
            }
            let _ = w.write_all(encode_line(&reply).as_bytes()).await;
        });
        (sock, seen)
    }

    fn edit() -> PreciseEdit {
        PreciseEdit {
            project_id: 7,
            path: "src/a.rs".into(),
            start_line: 1,
            start_col: 2,
            end_line: 3,
            end_col: 4,
            expected_text: "NEW".into(),
            new_text: "OLD".into(),
            summary: "撤销:改".into(),
            actor: "dozer".into(),
            session_id: "dozer-gui".into(),
        }
    }

    #[tokio::test]
    async fn list_comes_back_as_ok() {
        let sock = serve_once("eh-list", Some(Reply::FileEditHistory { entries: vec![] }));
        let got =
            EditHistoryService::list_file_edit_history(&Client::new(sock.clone()), 7, None, 200)
                .await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn list_sends_the_filter_and_limit_it_was_given() {
        let (sock, seen) = serve_capture("eh-list-req", Reply::FileEditHistory { entries: vec![] });
        let _ = EditHistoryService::list_file_edit_history(
            &Client::new(sock.clone()),
            7,
            Some("src/a.rs".into()),
            200,
        )
        .await;
        let line = seen.lock().unwrap().clone().expect("服务端应收到请求");
        match decode_line::<Request>(&line).unwrap() {
            Request::ListFileEditHistory {
                project_id,
                path_filter,
                limit,
            } => {
                assert_eq!(
                    (project_id, path_filter.as_deref(), limit),
                    (7, Some("src/a.rs"), 200)
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn apply_sends_every_field_in_the_right_place() {
        let (sock, seen) = serve_capture(
            "eh-apply-req",
            Reply::MutationResult {
                outcome: MutationOutcome::NotFound,
            },
        );
        let got = EditHistoryService::apply_precise_edit(&Client::new(sock.clone()), edit()).await;
        assert_eq!(got, Ok(MutationOutcome::NotFound), "outcome 原样带回");
        let line = seen.lock().unwrap().clone().expect("服务端应收到请求");
        match decode_line::<Request>(&line).unwrap() {
            Request::ApplyPreciseEdit {
                project_id,
                path,
                start_line,
                start_col,
                end_line,
                end_col,
                expected_text,
                new_text,
                summary,
                actor,
                session_id,
            } => {
                assert_eq!(
                    (
                        project_id,
                        path.as_str(),
                        start_line,
                        start_col,
                        end_line,
                        end_col
                    ),
                    (7, "src/a.rs", 1, 2, 3, 4)
                );
                assert_eq!((expected_text.as_str(), new_text.as_str()), ("NEW", "OLD"));
                assert_eq!(
                    (summary.as_str(), actor.as_str(), session_id.as_str()),
                    ("撤销:改", "dozer", "dozer-gui")
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("eh-hangup", None);
        let e =
            EditHistoryService::list_file_edit_history(&Client::new(sock.clone()), 7, None, 200)
                .await
                .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once(
            "eh-reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = EditHistoryService::apply_precise_edit(&Client::new(sock.clone()), edit())
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod bookmark_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{BookmarkService, ServiceErrorKind};

    #[tokio::test]
    async fn list_comes_back_as_ok() {
        let sock = serve_once("bm-list", Some(Reply::Bookmarks { bookmarks: vec![] }));
        let got = BookmarkService::list_bookmarks(&Client::new(sock.clone()), Some(7)).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn add_and_remove_return_unit_on_success() {
        let sock = serve_once("bm-add", Some(Reply::Ok));
        let got = BookmarkService::add_bookmark(
            &Client::new(sock.clone()),
            dozer_core::protocol::BookmarkScope::Global,
            None,
            "http://a.com".into(),
            "A".into(),
        )
        .await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);

        let sock = serve_once("bm-remove", Some(Reply::Ok));
        let got = BookmarkService::remove_bookmark(&Client::new(sock.clone()), 3).await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("bm-hangup", None);
        let e = BookmarkService::list_bookmarks(&Client::new(sock.clone()), None)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Unavailable, "daemon 断开")
        );
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once(
            "bm-reject",
            Some(Reply::Error {
                message: "boom".into(),
            }),
        );
        let e = BookmarkService::remove_bookmark(&Client::new(sock.clone()), 3)
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ServiceErrorKind::Rejected, "daemon 错误: boom")
        );
        let _ = std::fs::remove_file(&sock);
    }
}

#[cfg(test)]
mod session_protocol_golden {
    //! S6c-1 协议编码黄金测试:会话相关请求/应答的 `encode_line` 输出必须与
    //! 抽 `SessionService` 之前逐字一致。黄金串是在现状上跑出后写死的;
    //! 改动协议形状会让这里失败,必须显式更新并说明兼容影响。

    use super::*;

    fn sample_session() -> SessionInfo {
        SessionInfo {
            id: "sess-42".into(),
            name: "claude · demo".into(),
            command: "claude".into(),
            cwd: "/tmp/demo".into(),
            alive: true,
            created_ms: 1_700_000_000_000,
            agent_state: AgentState::Running,
            transcript_path: Some("/tmp/demo/t.jsonl".into()),
            project_id: Some(7),
            agent: AgentKind::Claude,
        }
    }

    #[test]
    fn session_protocol_encoding_golden() {
        let samples: [(&str, String, &str); 7] = [
            (
                "create_session",
                encode_line(&Request::CreateSession {
                    name: "claude · demo".into(),
                    command: "claude".into(),
                    args: vec!["--foo".into(), "bar baz".into()],
                    cwd: "/tmp/demo".into(),
                    cols: 120,
                    rows: 40,
                    project_id: 7,
                    agent: AgentKind::Claude,
                }),
                "{\"type\":\"create_session\",\"name\":\"claude · demo\",\"command\":\"claude\",\"args\":[\"--foo\",\"bar baz\"],\"cwd\":\"/tmp/demo\",\"cols\":120,\"rows\":40,\"project_id\":7,\"agent\":\"claude\"}\n",
            ),
            (
                "attach",
                encode_line(&Request::Attach {
                    session_id: "sess-42".into(),
                    from_offset: 4096,
                }),
                "{\"type\":\"attach\",\"session_id\":\"sess-42\",\"from_offset\":4096}\n",
            ),
            (
                "write",
                encode_line(&Request::Write {
                    session_id: "sess-42".into(),
                    data_b64: "aGVsbG8=".into(),
                }),
                "{\"type\":\"write\",\"session_id\":\"sess-42\",\"data_b64\":\"aGVsbG8=\"}\n",
            ),
            (
                "resize",
                encode_line(&Request::Resize {
                    session_id: "sess-42".into(),
                    cols: 132,
                    rows: 43,
                }),
                "{\"type\":\"resize\",\"session_id\":\"sess-42\",\"cols\":132,\"rows\":43}\n",
            ),
            (
                "kill",
                encode_line(&Request::Kill {
                    session_id: "sess-42".into(),
                }),
                "{\"type\":\"kill\",\"session_id\":\"sess-42\"}\n",
            ),
            (
                "created",
                encode_line(&Reply::Created {
                    session: sample_session(),
                }),
                "{\"type\":\"created\",\"session\":{\"id\":\"sess-42\",\"name\":\"claude · demo\",\"command\":\"claude\",\"cwd\":\"/tmp/demo\",\"alive\":true,\"created_ms\":1700000000000,\"agent_state\":\"running\",\"transcript_path\":\"/tmp/demo/t.jsonl\",\"project_id\":7,\"agent\":\"claude\"}}\n",
            ),
            (
                "attached",
                encode_line(&Reply::Attached {
                    session_id: "sess-42".into(),
                    snapshot_b64: "G1szMW1yZWQ=".into(),
                    next_offset: 8192,
                }),
                "{\"type\":\"attached\",\"session_id\":\"sess-42\",\"snapshot_b64\":\"G1szMW1yZWQ=\",\"next_offset\":8192}\n",
            ),
        ];
        for (name, actual, expected) in samples {
            assert_eq!(actual, expected, "会话协议编码变了: {name}");
        }
    }
}

#[cfg(test)]
mod session_service_tests {
    use super::todo_service_tests::temp_sock;
    use super::*;
    use dozer_model::SessionEvent;
    use dozer_services::{SessionService, SessionSpec};
    use std::any::TypeId;

    #[test]
    fn session_event_is_the_same_type_as_term_event() {
        assert_eq!(
            TypeId::of::<TermEvent>(),
            TypeId::of::<SessionEvent>(),
            "TermEvent 必须是 dozer_model::SessionEvent 的重新导出"
        );
    }

    fn sample_session() -> SessionInfo {
        SessionInfo {
            id: "sess-42".into(),
            name: "claude".into(),
            command: "claude".into(),
            cwd: "/tmp/demo".into(),
            alive: true,
            created_ms: 1,
            agent_state: AgentState::Idle,
            transcript_path: None,
            project_id: Some(7),
            agent: AgentKind::Claude,
        }
    }

    /// 假服务端:按请求类型应答,并把收到的业务请求回传给测试断言。
    fn serve_session_end_to_end(tag: &str) -> (PathBuf, mpsc::UnboundedReceiver<Request>) {
        let sock = temp_sock(tag);
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        let (tx, rx) = mpsc::unbounded_channel::<Request>();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut lines = BufReader::new(r).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let req: Request = decode_line(&line).unwrap();
                        if matches!(req, Request::Hello { .. }) {
                            let _ = w
                                .write_all(
                                    encode_line(&Reply::Hello {
                                        protocol: dozer_core::protocol::PROTOCOL_VERSION,
                                        product: dozer_core::paths::product().name.to_string(),
                                    })
                                    .as_bytes(),
                                )
                                .await;
                            continue;
                        }
                        let reply = match &req {
                            Request::ListSessions => Reply::Sessions {
                                sessions: vec![sample_session()],
                            },
                            Request::CreateSession { .. } => Reply::Created {
                                session: sample_session(),
                            },
                            Request::Attach { .. } => Reply::Attached {
                                session_id: "sess-42".into(),
                                snapshot_b64: B64.encode(b"snap"),
                                next_offset: 12,
                            },
                            _ => Reply::Ok,
                        };
                        let _ = tx.send(req);
                        let _ = w.write_all(encode_line(&reply).as_bytes()).await;
                    }
                });
            }
        });
        (sock, rx)
    }

    #[tokio::test]
    async fn client_implements_session_service_end_to_end() {
        let (sock, mut seen) = serve_session_end_to_end("sess-e2e");
        let client = Client::new(sock.clone());
        let svc: &dyn SessionService = &client;

        let sessions = svc.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 1);

        let info = svc
            .create(SessionSpec {
                name: "claude".into(),
                command: "claude".into(),
                args: vec!["--foo".into()],
                cwd: "/tmp/demo".into(),
                cols: 120,
                rows: 40,
                project_id: 7,
                agent: AgentKind::Claude,
            })
            .await
            .unwrap();
        assert_eq!(info.id, "sess-42");

        let (snapshot, next, rx) = svc.attach("sess-42".into(), 0).await.unwrap();
        assert_eq!((snapshot.as_slice(), next), (&b"snap"[..], 12));
        drop(rx); // 关闭 attach 读任务

        svc.write("sess-42".into(), b"hello".to_vec())
            .await
            .unwrap();
        svc.resize("sess-42".into(), 132, 43).await.unwrap();
        svc.kill("sess-42".into()).await.unwrap();
        svc.close_with_summary("sess-42".into()).await.unwrap();

        let mut requests = Vec::new();
        while let Ok(req) = seen.try_recv() {
            requests.push(req);
        }
        assert!(matches!(requests[0], Request::ListSessions), "{requests:?}");
        assert!(
            matches!(
                &requests[1],
                Request::CreateSession {
                    name,
                    command,
                    args,
                    cwd,
                    cols: 120,
                    rows: 40,
                    project_id: 7,
                    agent: AgentKind::Claude,
                } if name == "claude" && command == "claude" && args == &vec!["--foo".to_string()]
                    && cwd == "/tmp/demo"
            ),
            "{requests:?}"
        );
        assert!(
            matches!(&requests[2], Request::Attach { session_id, from_offset: 0 } if session_id == "sess-42"),
            "{requests:?}"
        );
        assert!(
            matches!(&requests[3], Request::Write { session_id, data_b64 } if session_id == "sess-42" && data_b64 == "aGVsbG8="),
            "{requests:?}"
        );
        assert!(
            matches!(&requests[4], Request::Resize { session_id, cols: 132, rows: 43 } if session_id == "sess-42"),
            "{requests:?}"
        );
        assert!(
            matches!(&requests[5], Request::Kill { session_id } if session_id == "sess-42"),
            "{requests:?}"
        );
        assert!(
            matches!(&requests[6], Request::CloseWithSummary { session_id } if session_id == "sess-42"),
            "{requests:?}"
        );
        let _ = std::fs::remove_file(&sock);
    }
}
