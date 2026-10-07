use anyhow::{Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use bytehost_apps::id::AppId;
use bytehost_apps::plan::{ApprovedInstallPlan, InstallPlan, Provenance, TrustLevel};
use bytehost_apps::proto::{
    AppReply, AppRequest, AppSource, AppSummary, ManagedRuntime, RuntimeInstallPlan, RuntimeProbe,
};
use bytehost_apps::registry::UninstallMode;
use dozer_core::protocol::{
    AgentKind, AgentState, BookmarkInfo, BookmarkScope, CategoryInfo, CategoryMoveDirection,
    CodeHealthReportInfo, ConversationSummary, GroupCancelScope, GroupInfo, GroupMessageInfo,
    PreviewCommand, PreviewContext, ProjectInfo, Reply, Request, SessionInfo,
    SessionSummaryPayload, TodoInfo, TurnRecord, UsagePayload, decode_line, encode_line,
};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum TermEvent {
    Output(Vec<u8>),
    Exited(Option<i32>),
    Lagged,
    Disconnected,
    /// 本会话 agent 状态变更（hook 事件驱动，dozerd 广播）。
    Agent {
        agent: AgentKind,
        state: AgentState,
        transcript_path: Option<String>,
    },
}

#[derive(Clone)]
pub struct Client {
    socket: PathBuf,
}

impl Client {
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }

    async fn roundtrip(&self, req: &Request) -> Result<Reply> {
        let stream = UnixStream::connect(&self.socket).await?;
        let (r, mut w) = stream.into_split();
        w.write_all(encode_line(req).as_bytes()).await?;
        let mut lines = BufReader::new(r).lines();
        let line = lines
            .next_line()
            .await?
            .ok_or_else(|| anyhow!("daemon 断开"))?;
        let reply: Reply = decode_line(&line)?;
        if let Reply::Error { message } = &reply {
            bail!("daemon 错误: {message}");
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

    /// 应用宿主请求(原样转给 dozerd 的 `AppService`)。失败(含"应用宿主不可用")走 `Err`。
    pub async fn app_request(&self, request: AppRequest) -> Result<AppReply> {
        match self.roundtrip(&Request::App { request }).await? {
            // 失败带类别:用 `err.downcast_ref::<AppFailure>()` 取回(其余错误是传输/协议问题)。
            Reply::App {
                reply: AppReply::Failed { failure },
            } => Err(anyhow::Error::new(failure)),
            Reply::App { reply } => Ok(reply),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn app_list(&self) -> Result<Vec<AppSummary>> {
        match self.app_request(AppRequest::List).await? {
            AppReply::Apps { apps } => Ok(apps),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 出安装计划(不安装任何东西)。
    pub async fn app_plan(
        &self,
        source: AppSource,
        provenance: Provenance,
        trust: TrustLevel,
    ) -> Result<InstallPlan> {
        match self
            .app_request(AppRequest::Plan {
                source,
                provenance,
                trust,
            })
            .await?
        {
            AppReply::Plan { plan } => Ok(*plan),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 安装一份已批准的计划(服务端对 staging 副本重新计算并核对)。
    pub async fn app_install(
        &self,
        approved: ApprovedInstallPlan,
        source: AppSource,
    ) -> Result<()> {
        let request = AppRequest::Install {
            approved: Box::new(approved),
            source,
        };
        self.app_expect_done(request).await
    }

    /// 启动应用,返回不含令牌的站点地址。
    pub async fn app_start(&self, id: AppId) -> Result<String> {
        match self.app_request(AppRequest::Start { id }).await? {
            AppReply::Started { url } => Ok(url),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn app_stop(&self, id: AppId) -> Result<()> {
        self.app_expect_done(AppRequest::Stop { id }).await
    }

    pub async fn app_uninstall(&self, id: AppId, mode: UninstallMode) -> Result<()> {
        self.app_expect_done(AppRequest::Uninstall { id, mode })
            .await
    }

    /// 首次导航用的地址(含令牌,**秘密**:不要写日志)。应用没在运行会失败。
    pub async fn app_launch_url(&self, id: AppId) -> Result<String> {
        match self.app_request(AppRequest::LaunchUrl { id }).await? {
            AppReply::LaunchUrl { url } => Ok(url),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn app_probe_runtimes(&self) -> Result<Vec<RuntimeProbe>> {
        match self.app_request(AppRequest::ProbeRuntimes).await? {
            AppReply::Runtimes { runtimes } => Ok(runtimes),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 出一份运行时安装计划(不下载任何东西)。
    pub async fn app_runtime_plan(&self, runtime: ManagedRuntime) -> Result<RuntimeInstallPlan> {
        match self.app_request(AppRequest::RuntimePlan { runtime }).await? {
            AppReply::RuntimePlan { plan } => Ok(*plan),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 安装一份已批准的运行时计划(服务端重算并逐字段核对)。
    pub async fn app_install_runtime(&self, plan: RuntimeInstallPlan) -> Result<()> {
        self.app_expect_done(AppRequest::InstallRuntime {
            plan: Box::new(plan),
        })
        .await
    }

    /// 卸载某个受管运行时版本。
    pub async fn app_uninstall_runtime(
        &self,
        runtime: ManagedRuntime,
        version: &str,
    ) -> Result<()> {
        self.app_expect_done(AppRequest::UninstallRuntime {
            runtime,
            version: version.to_string(),
        })
        .await
    }

    async fn app_expect_done(&self, request: AppRequest) -> Result<()> {
        match self.app_request(request).await? {
            AppReply::Done => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
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
