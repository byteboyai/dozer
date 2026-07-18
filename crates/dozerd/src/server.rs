use crate::registry::SessionRegistry;
use crate::session::{SessionEvent, SessionSpec};
use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use dozer_core::protocol::{Reply, Request, decode_line, encode_line};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;

pub async fn serve(socket: &Path, registry: Arc<SessionRegistry>) -> Result<()> {
    if socket.exists() {
        std::fs::remove_file(socket)?;
    }
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(socket)?;
    tracing::info!(socket = %socket.display(), "dozerd 监听中");
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(error = %e, "accept 失败，跳过本次连接");
                continue;
            }
        };
        let registry = registry.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, registry).await {
                tracing::debug!(error = %e, "连接结束");
            }
        });
    }
}

async fn handle_conn(stream: UnixStream, registry: Arc<SessionRegistry>) -> Result<()> {
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
                let reply = match decode_line::<Request>(&line) {
                    Err(e) => Reply::Error { message: format!("协议错误: {e}") },
                    Ok(req) => match req {
                        Request::ListSessions => Reply::Sessions { sessions: registry.list() },
                        Request::CreateSession { name, command, args, cwd, cols, rows } => {
                            match registry.create(SessionSpec { name, command, args, cwd, cols, rows }) {
                                Ok(s) => Reply::Created { session: s.info() },
                                Err(e) => Reply::Error { message: e.to_string() },
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
                        // T2 替换为真实实现（状态映射 + 广播）
                        Request::HookEvent { .. } => Reply::Ok,
                    },
                };
                w.write_all(encode_line(&reply).as_bytes()).await?;
            }
            ev = async {
                match &mut sub {
                    Some((_, rx)) => rx.recv().await,
                    None => std::future::pending().await,
                }
            }, if sub.is_some() => {
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
