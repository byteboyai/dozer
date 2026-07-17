use anyhow::{Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use dozer_core::protocol::{Reply, Request, SessionInfo, decode_line, encode_line};
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
        let line = lines.next_line().await?.ok_or_else(|| anyhow!("daemon 断开"))?;
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

    pub async fn create(&self, name: &str, command: &str, args: &[String],
                        cwd: &str, cols: u16, rows: u16) -> Result<SessionInfo> {
        match self.roundtrip(&Request::CreateSession {
            name: name.into(), command: command.into(), args: args.to_vec(),
            cwd: cwd.into(), cols, rows,
        }).await? {
            Reply::Created { session } => Ok(session),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn write(&self, id: &str, data: &[u8]) -> Result<()> {
        match self.roundtrip(&Request::Write {
            session_id: id.into(), data_b64: B64.encode(data),
        }).await? {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        match self.roundtrip(&Request::Resize { session_id: id.into(), cols, rows }).await? {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn kill(&self, id: &str) -> Result<()> {
        match self.roundtrip(&Request::Kill { session_id: id.into() }).await? {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn attach(&self, id: &str, from_offset: u64)
        -> Result<(Vec<u8>, u64, mpsc::UnboundedReceiver<TermEvent>)> {
        let stream = UnixStream::connect(&self.socket).await?;
        let (r, mut w) = stream.into_split();
        w.write_all(encode_line(&Request::Attach {
            session_id: id.into(), from_offset,
        }).as_bytes()).await?;
        let mut lines = BufReader::new(r).lines();
        let first = lines.next_line().await?.ok_or_else(|| anyhow!("daemon 断开"))?;
        let (snapshot, next) = match decode_line::<Reply>(&first)? {
            Reply::Attached { snapshot_b64, next_offset, .. } =>
                (B64.decode(snapshot_b64.as_bytes())?, next_offset),
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
