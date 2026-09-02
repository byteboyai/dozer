use anyhow::{Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use dozer_core::protocol::{
    AgentKind, AgentState, BookmarkInfo, BookmarkScope, CategoryInfo, CategoryMoveDirection,
    ConversationSummary, PreviewContext, ProjectInfo, Reply, Request, SessionInfo,
    SessionSummaryPayload, TodoInfo, TurnRecord, UsagePayload, decode_line, encode_line,
};
use std::path::PathBuf;
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

/// `Client::record_acceptance` 的参数对象:原先 7 个位置参数里 `repo`/
/// `goal`/`verdict`/`comment`/`ref_name` 五个都是 `&str`,顺序传错编译器
/// 发现不了(Rust Design Patterns:Builder,用具名字段替代同类型位置参数)。
pub struct RecordAcceptanceParams<'a> {
    pub repo: &'a str,
    pub goal: &'a str,
    pub criteria_checked: &'a [String],
    pub verdict: &'a str,
    pub comment: &'a str,
    pub ref_name: &'a str,
    pub ts_ms: u64,
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
        match self
            .roundtrip(&Request::CreateSession {
                name: name.into(),
                command: command.into(),
                args: args.to_vec(),
                cwd: cwd.into(),
                cols,
                rows,
                project_id,
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

    pub async fn record_acceptance(&self, params: RecordAcceptanceParams<'_>) -> Result<()> {
        match self
            .roundtrip(&Request::RecordAcceptance {
                repo: params.repo.into(),
                goal: params.goal.into(),
                criteria_checked: params.criteria_checked.to_vec(),
                verdict: params.verdict.into(),
                comment: params.comment.into(),
                ref_name: params.ref_name.into(),
                ts_ms: params.ts_ms,
            })
            .await?
        {
            Reply::Ok => Ok(()),
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

    pub async fn acceptance_count(&self, repo: &str) -> Result<u64> {
        match self
            .roundtrip(&Request::GetAcceptanceCount { repo: repo.into() })
            .await?
        {
            Reply::AcceptanceCount { count } => Ok(count),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => Err(anyhow::anyhow!("非预期应答: {other:?}")),
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

    pub async fn record_todo_dispatch(&self, id: i64, session_id: &str) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::RecordTodoDispatch {
                id,
                session_id: session_id.into(),
            })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
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
