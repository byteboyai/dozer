use anyhow::{Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use dozer_core::protocol::{
    AgentKind, AgentState, BookmarkInfo, BookmarkScope, PreviewContext, ProjectInfo, Reply,
    Request, SessionInfo, decode_line, encode_line,
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

    #[allow(clippy::too_many_arguments)]
    pub async fn record_acceptance(
        &self,
        repo: &str,
        goal: &str,
        criteria_checked: &[String],
        verdict: &str,
        comment: &str,
        ref_name: &str,
        ts_ms: u64,
    ) -> Result<()> {
        match self
            .roundtrip(&Request::RecordAcceptance {
                repo: repo.into(),
                goal: goal.into(),
                criteria_checked: criteria_checked.to_vec(),
                verdict: verdict.into(),
                comment: comment.into(),
                ref_name: ref_name.into(),
                ts_ms,
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

    pub async fn update_preview_context(
        &self,
        project_id: i64,
        context: Option<PreviewContext>,
    ) -> Result<()> {
        match self
            .roundtrip(&Request::UpdatePreviewContext { project_id, context })
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
