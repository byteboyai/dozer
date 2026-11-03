//! 交付包含字节许可，只有 GUI 应用后才 ack/推进续接水位。
use super::*;
use dozer_model::{RuntimeCapabilities, StreamMode, TerminalEvent, TerminalState};
use dozer_services::{SessionStream, StreamAck, StreamItem};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::Semaphore;
const LOCAL_BYTES: usize = 1024 * 1024;
const MAX_FRAME: usize = 5 * LOCAL_BYTES;
static EPOCH: AtomicU64 = AtomicU64::new(1);
fn budget() -> Arc<Semaphore> {
    static B: OnceLock<Arc<Semaphore>> = OnceLock::new();
    B.get_or_init(|| Arc::new(Semaphore::new(16 * LOCAL_BYTES)))
        .clone()
}
fn frames() -> Arc<Semaphore> {
    static B: OnceLock<Arc<Semaphore>> = OnceLock::new();
    B.get_or_init(|| Arc::new(Semaphore::new(64 * LOCAL_BYTES)))
        .clone()
}
fn limited_diagnostic(value: &impl std::fmt::Display, max: usize) -> String {
    struct Limited {
        text: String,
        max: usize,
    }
    impl std::fmt::Write for Limited {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            let mut n = s.len().min(self.max.saturating_sub(self.text.len()));
            while !s.is_char_boundary(n) {
                n -= 1;
            }
            self.text.push_str(&s[..n]);
            if n < s.len() {
                Err(std::fmt::Error)
            } else {
                Ok(())
            }
        }
    }
    let mut out = Limited {
        text: String::new(),
        max,
    };
    let _ = std::fmt::write(&mut out, format_args!("{value}"));
    out.text
}
impl Client {
    pub async fn runtime_capabilities(&self) -> Result<RuntimeCapabilities> {
        if let Some(value) = self
            .runtime_capabilities
            .lock()
            .expect("capabilities")
            .clone()
        {
            return Ok(value);
        }
        self.ensure_version_checked().await?;
        let value = match self
            .roundtrip_keep_error(&Request::RuntimeCapabilities)
            .await?
        {
            Reply::RuntimeCapabilities { capabilities } => capabilities,
            Reply::Error { message }
                if message.contains("unknown variant") || message.contains("未知请求") =>
            {
                RuntimeCapabilities::default()
            }
            Reply::Error { message } => {
                bail!("运行时能力请求失败: {}", limited_diagnostic(&message, 4096))
            }
            _ => bail!("运行时能力请求失败: 意外应答"),
        };
        *self.runtime_capabilities.lock().expect("capabilities") = Some(value.clone());
        Ok(value)
    }
    pub async fn attach_stream(&self, id: String, offset: u64) -> Result<SessionStream> {
        let v2 = self.runtime_capabilities().await?.terminal_stream_v2;
        let client = self.clone();
        let (tx, events) = mpsc::channel(1);
        tokio::spawn(async move {
            let local = Arc::new(Semaphore::new(LOCAL_BYTES));
            let mut applied = offset;
            let mut failures = 0;
            loop {
                let epoch = EPOCH.fetch_add(1, Ordering::Relaxed);
                let before = applied;
                let result = tokio::select! {
                    _=tx.closed()=>return,
                    r=client.pump_stream(&id,epoch,v2,&tx,&local,&mut applied)=>r,
                };
                if result.is_ok() || tx.is_closed() {
                    return;
                }
                let error = limited_diagnostic(&result.unwrap_err(), 4096);
                if applied != before {
                    failures = 0;
                }
                failures += 1;
                if failures > 5 {
                    let message = format!("终端恢复失败: {error}");
                    let _ = deliver(&tx, &local, TerminalEvent::Failed { epoch, message }).await;
                    return;
                }
                if deliver(&tx, &local, TerminalEvent::Recovering { epoch })
                    .await
                    .is_err()
                {
                    return;
                }
                tokio::select! {_=tx.closed()=>return,_=tokio::time::sleep(Duration::from_millis(100 << (failures-1)))=>{}}
            }
        });
        Ok(SessionStream { events })
    }
    async fn pump_stream(
        &self,
        id: &str,
        epoch: u64,
        v2: bool,
        tx: &mpsc::Sender<StreamItem>,
        local: &Arc<Semaphore>,
        applied: &mut u64,
    ) -> Result<()> {
        let from = *applied;
        let stream =
            tokio::time::timeout(CONNECT_TIMEOUT, UnixStream::connect(&self.socket)).await??;
        let (r, mut writer) = stream.into_split();
        let req = if v2 {
            Request::AttachV2 {
                session_id: id.into(),
                from_offset: from,
                epoch,
            }
        } else {
            Request::Attach {
                session_id: id.into(),
                from_offset: 0,
            }
        };
        tokio::time::timeout(
            WRITE_TIMEOUT,
            writer.write_all(encode_request_checked(&req)?.as_bytes()),
        )
        .await??;
        let mut reader = FrameReader::new(BufReader::new(r));
        let mut first = true;
        let mut next = from;
        let mut legacy = None;
        loop {
            // 先等可读，静默连接不占全局帧预算。
            if reader.pending.is_empty() && reader.reader.buffer().is_empty() {
                if first {
                    tokio::time::timeout(HANDSHAKE_TIMEOUT, reader.reader.get_ref().readable())
                        .await??;
                } else {
                    reader.reader.get_ref().readable().await?;
                }
            }
            let frame_permit = tokio::select! {_=tx.closed()=>bail!("消费者已关闭"),p=frames().acquire_many_owned(MAX_FRAME as u32)=>p?};
            let line = tokio::time::timeout(HANDSHAKE_TIMEOUT, reader.next_frame(MAX_FRAME))
                .await??
                .ok_or_else(|| anyhow!("daemon 断开"))?;
            let event = match decode_line::<Reply>(&line)? {
                Reply::Terminal { event } => event,
                Reply::Attached {
                    snapshot_b64,
                    next_offset,
                    ..
                } if !v2 && first => {
                    let data = B64.decode(snapshot_b64)?;
                    let info = self
                        .list()
                        .await?
                        .into_iter()
                        .find(|s| s.id == id)
                        .ok_or_else(|| anyhow!("会话不存在"))?;
                    let state = TerminalState {
                        lifecycle_revision: 1,
                        agent_revision: 1,
                        alive: info.alive,
                        exit_code: None,
                        output_complete: !info.alive,
                        output_truncated: false,
                        agent: info.agent,
                        agent_state: info.agent_state,
                        transcript_path: info.transcript_path,
                    };
                    legacy = Some(state.clone());
                    TerminalEvent::Snapshot {
                        epoch,
                        mode: StreamMode::Reset,
                        start_offset: next_offset.saturating_sub(data.len() as u64),
                        next_offset,
                        data,
                        state,
                        degraded: true,
                    }
                }
                Reply::Output {
                    data_b64, offset, ..
                } if !v2 => {
                    let data = B64.decode(data_b64)?;
                    TerminalEvent::Output {
                        replay: false,
                        epoch,
                        start_offset: offset
                            .checked_sub(data.len() as u64)
                            .ok_or_else(|| anyhow!("offset 下溢"))?,
                        end_offset: offset,
                        data,
                    }
                }
                Reply::AgentEvent {
                    agent,
                    state,
                    transcript_path,
                    ..
                } if !v2 => {
                    let current = legacy.as_mut().ok_or_else(|| anyhow!("尚未 attach"))?;
                    current.agent = agent;
                    current.agent_state = state;
                    current.transcript_path = transcript_path;
                    current.agent_revision += 1;
                    TerminalEvent::State {
                        epoch,
                        state: current.clone(),
                    }
                }
                Reply::Exited { code, .. } if !v2 => {
                    let current = legacy.as_mut().ok_or_else(|| anyhow!("尚未 attach"))?;
                    current.alive = false;
                    current.output_complete = true;
                    current.exit_code = code;
                    current.lifecycle_revision += 1;
                    TerminalEvent::State {
                        epoch,
                        state: current.clone(),
                    }
                }
                Reply::Error { message } => bail!("{message}"),
                _ => bail!("终端意外应答"),
            };
            anyhow::ensure!(event.epoch() == epoch, "终端 epoch 不匹配");
            if let TerminalEvent::Snapshot { state, .. } | TerminalEvent::State { state, .. } =
                &event
            {
                anyhow::ensure!(
                    state
                        .transcript_path
                        .as_ref()
                        .is_none_or(|p| p.len() <= 4096),
                    "Limit: terminal state exceeds 4KiB"
                );
            }
            let complete = match &event {
                TerminalEvent::Snapshot {
                    mode,
                    start_offset,
                    next_offset,
                    data,
                    state,
                    ..
                } => {
                    anyhow::ensure!(
                        first && next_offset.checked_sub(*start_offset) == Some(data.len() as u64),
                        "快照范围错误"
                    );
                    if *mode == StreamMode::Resume {
                        anyhow::ensure!(*start_offset == from, "续接范围错误");
                    }
                    next = *next_offset;
                    state.output_complete
                }
                TerminalEvent::Output {
                    start_offset,
                    end_offset,
                    data,
                    ..
                } => {
                    anyhow::ensure!(
                        !first
                            && *start_offset == next
                            && end_offset.checked_sub(*start_offset) == Some(data.len() as u64),
                        "终端字节缺口"
                    );
                    next = *end_offset;
                    false
                }
                TerminalEvent::State { state, .. } => {
                    anyhow::ensure!(!first, "缺少快照");
                    state.output_complete
                }
                TerminalEvent::ResyncRequired { .. } => bail!("终端落后，需要重新 attach"),
                _ => bail!("非法服务端终端事件"),
            };
            deliver_batched(tx, local, event, applied).await?;
            drop(frame_permit);
            first = false;
            if complete {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(16)).await;
        }
    }
}
async fn deliver(
    tx: &mpsc::Sender<StreamItem>,
    local: &Arc<Semaphore>,
    event: TerminalEvent,
) -> Result<()> {
    let permits = reserve(tx, local, event.data_len().max(1)).await?;
    deliver_reserved(tx, event, permits).await
}
async fn reserve(
    tx: &mpsc::Sender<StreamItem>,
    local: &Arc<Semaphore>,
    n: usize,
) -> Result<Vec<tokio::sync::OwnedSemaphorePermit>> {
    anyhow::ensure!(n <= LOCAL_BYTES, "终端交付超过预算");
    let permits = tokio::select! {
        _=tx.closed()=>bail!("消费者已关闭"),
        permits=async {let global=budget().acquire_many_owned(n as u32).await?;let local=local.clone().acquire_many_owned(n as u32).await?;Ok::<_,anyhow::Error>(vec![global,local])}=>permits?,
    };
    Ok(permits)
}
async fn deliver_reserved(
    tx: &mpsc::Sender<StreamItem>,
    event: TerminalEvent,
    permits: Vec<tokio::sync::OwnedSemaphorePermit>,
) -> Result<()> {
    let ack = StreamAck::new();
    tx.send(StreamItem::new(event, permits, ack.clone()))
        .await
        .map_err(|_| anyhow!("消费者已关闭"))?;
    tokio::select! {_=tx.closed()=>bail!("消费者已关闭"),_=ack.notify.notified()=>{}}
    anyhow::ensure!(ack.applied.load(Ordering::Acquire), "终端交付未被应用");
    Ok(())
}

const UI_BATCH: usize = 64 * 1024;
fn split_credits(
    credits: &mut [tokio::sync::OwnedSemaphorePermit],
    n: usize,
) -> Vec<tokio::sync::OwnedSemaphorePermit> {
    credits
        .iter_mut()
        .map(|p| p.split(n).expect("reserved batch credit"))
        .collect()
}
async fn deliver_batched(
    tx: &mpsc::Sender<StreamItem>,
    local: &Arc<Semaphore>,
    event: TerminalEvent,
    applied: &mut u64,
) -> Result<()> {
    match event {
        TerminalEvent::Snapshot {
            epoch,
            mode,
            start_offset,
            data,
            state,
            degraded,
            ..
        } if data.len() > UI_BATCH => {
            let mut credits = reserve(tx, local, data.len()).await?;
            let mut offset = start_offset;
            for (index, chunk) in data.chunks(UI_BATCH).enumerate() {
                let end = offset + chunk.len() as u64;
                let event = if index == 0 {
                    let mut partial = state.clone();
                    partial.output_complete = false;
                    TerminalEvent::Snapshot {
                        epoch,
                        mode,
                        start_offset: offset,
                        next_offset: end,
                        data: chunk.to_vec(),
                        state: partial,
                        degraded,
                    }
                } else {
                    TerminalEvent::Output {
                        epoch,
                        start_offset: offset,
                        end_offset: end,
                        data: chunk.to_vec(),
                        replay: true,
                    }
                };
                deliver_reserved(tx, event, split_credits(&mut credits, chunk.len())).await?;
                *applied = end;
                offset = end;
                tokio::time::sleep(Duration::from_millis(16)).await;
            }
            if state.output_complete {
                deliver(tx, local, TerminalEvent::State { epoch, state }).await?;
            }
        }
        TerminalEvent::Output {
            epoch,
            start_offset,
            data,
            replay,
            ..
        } if data.len() > UI_BATCH => {
            let mut credits = reserve(tx, local, data.len()).await?;
            let mut offset = start_offset;
            for chunk in data.chunks(UI_BATCH) {
                let end = offset + chunk.len() as u64;
                deliver_reserved(
                    tx,
                    TerminalEvent::Output {
                        epoch,
                        start_offset: offset,
                        end_offset: end,
                        data: chunk.to_vec(),
                        replay,
                    },
                    split_credits(&mut credits, chunk.len()),
                )
                .await?;
                *applied = end;
                offset = end;
                tokio::time::sleep(Duration::from_millis(16)).await;
            }
        }
        event => {
            let offset = event.next_offset();
            deliver(tx, local, event).await?;
            if let Some(offset) = offset {
                *applied = offset;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(complete: bool) -> TerminalState {
        TerminalState {
            lifecycle_revision: 1,
            agent_revision: 1,
            alive: !complete,
            exit_code: complete.then_some(0),
            output_complete: complete,
            output_truncated: false,
            agent: dozer_model::AgentKind::Unknown,
            agent_state: Default::default(),
            transcript_path: None,
        }
    }
    #[tokio::test]
    async fn delivery_waits_for_ui_ack_and_last_clone_keeps_credit() {
        let (tx, mut rx) = mpsc::channel(1);
        let local = Arc::new(Semaphore::new(LOCAL_BYTES));
        let copy = local.clone();
        let delivery = tokio::spawn(async move {
            deliver(
                &tx,
                &copy,
                TerminalEvent::Output {
                    replay: false,
                    epoch: 1,
                    start_offset: 0,
                    end_offset: 4,
                    data: b"data".to_vec(),
                },
            )
            .await
        });
        let item = rx.recv().await.unwrap();
        let clone = item.clone();
        assert_eq!(local.available_permits(), LOCAL_BYTES - 4);
        assert!(!delivery.is_finished());
        item.acknowledge();
        delivery.await.unwrap().unwrap();
        drop(item);
        assert_eq!(local.available_permits(), LOCAL_BYTES - 4);
        drop(clone);
        assert_eq!(local.available_permits(), LOCAL_BYTES);
    }
    #[tokio::test]
    async fn dropping_unapplied_item_stops_delivery_and_releases_credit() {
        let (tx, mut rx) = mpsc::channel(1);
        let local = Arc::new(Semaphore::new(LOCAL_BYTES));
        let copy = local.clone();
        let delivery = tokio::spawn(async move {
            deliver(
                &tx,
                &copy,
                TerminalEvent::Output {
                    replay: false,
                    epoch: 1,
                    start_offset: 0,
                    end_offset: 4,
                    data: b"data".to_vec(),
                },
            )
            .await
        });
        drop(rx.recv().await.unwrap());
        assert!(delivery.await.unwrap().is_err());
        assert_eq!(local.available_permits(), LOCAL_BYTES);
    }
    #[tokio::test]
    async fn half_frame_disconnect_reconnects_from_ui_applied_watermark() {
        let dir = tempfile::Builder::new()
            .prefix("dz-stream-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("s");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            for round in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                let (mut r, mut w) = stream.into_split();
                let mut reader = BufReader::new(&mut r);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                let Request::AttachV2 {
                    from_offset, epoch, ..
                } = decode_line(&line).unwrap()
                else {
                    panic!("v2 expected")
                };
                assert_eq!(from_offset, round);
                let reply = Reply::Terminal {
                    event: TerminalEvent::Snapshot {
                        epoch,
                        mode: if round == 0 {
                            StreamMode::Reset
                        } else {
                            StreamMode::Resume
                        },
                        start_offset: round,
                        next_offset: round + 1,
                        data: vec![b'a' + round as u8],
                        state: state(round == 1),
                        degraded: false,
                    },
                };
                w.write_all(encode_line(&reply).as_bytes()).await.unwrap();
                if round == 0 {
                    w.write_all(b"{\"type\":\"terminal\",\"event\":")
                        .await
                        .unwrap();
                }
            }
        });
        let client = Client::new(socket);
        *client.runtime_capabilities.lock().unwrap() = Some(RuntimeCapabilities {
            terminal_stream_v2: true,
            ..Default::default()
        });
        let mut stream = client.attach_stream("session".into(), 0).await.unwrap();
        let first = stream.events.recv().await.unwrap();
        assert_eq!(first.event.next_offset(), Some(1));
        first.acknowledge();
        drop(first);
        let recovery = stream.events.recv().await.unwrap();
        assert!(matches!(
            recovery.event.as_ref(),
            TerminalEvent::Recovering { .. }
        ));
        recovery.acknowledge();
        drop(recovery);
        let second = stream.events.recv().await.unwrap();
        assert!(matches!(
            second.event.as_ref(),
            TerminalEvent::Snapshot {
                mode: StreamMode::Resume,
                start_offset: 1,
                next_offset: 2,
                ..
            }
        ));
        second.acknowledge();
        drop(second);
        assert!(stream.events.recv().await.is_none());
        server.await.unwrap();
    }
    #[tokio::test]
    async fn legacy_daemon_is_adapted_to_reset_with_degraded_status() {
        let dir = tempfile::Builder::new()
            .prefix("dz-legacy-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("s");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (r, mut w) = stream.into_split();
            let mut r = BufReader::new(r);
            let mut request = String::new();
            r.read_line(&mut request).await.unwrap();
            assert!(matches!(
                decode_line::<Request>(&request).unwrap(),
                Request::Attach { from_offset: 0, .. }
            ));
            w.write_all(
                encode_line(&Reply::Attached {
                    session_id: "s".into(),
                    snapshot_b64: B64.encode(b"a"),
                    next_offset: 1,
                })
                .as_bytes(),
            )
            .await
            .unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let (r, mut list_writer) = stream.into_split();
            let mut reader = BufReader::new(r);
            let mut request = String::new();
            reader.read_line(&mut request).await.unwrap();
            assert!(matches!(
                decode_line::<Request>(&request).unwrap(),
                Request::ListSessions
            ));
            let session = SessionInfo {
                id: "s".into(),
                name: "legacy".into(),
                command: "sh".into(),
                cwd: "/tmp".into(),
                alive: true,
                created_ms: 0,
                agent_state: Default::default(),
                transcript_path: None,
                project_id: Some(1),
                agent: Default::default(),
            };
            list_writer
                .write_all(
                    encode_line(&Reply::Sessions {
                        sessions: vec![session],
                    })
                    .as_bytes(),
                )
                .await
                .unwrap();
            w.write_all(
                encode_line(&Reply::Exited {
                    session_id: "s".into(),
                    code: Some(7),
                })
                .as_bytes(),
            )
            .await
            .unwrap();
        });
        let client = Client::new(socket);
        client
            .version_checked
            .store(VERSION_COMPATIBLE, Ordering::Relaxed);
        *client.runtime_capabilities.lock().unwrap() = Some(RuntimeCapabilities::default());
        let mut stream = client.attach_stream("s".into(), 99).await.unwrap();
        let snapshot = stream.events.recv().await.unwrap();
        assert!(
            matches!(snapshot.event.as_ref(),TerminalEvent::Snapshot{mode:StreamMode::Reset,degraded:true,data,..} if data==b"a")
        );
        snapshot.acknowledge();
        drop(snapshot);
        let exited = stream.events.recv().await.unwrap();
        assert!(
            matches!(exited.event.as_ref(),TerminalEvent::State{state,..} if state.output_complete&&state.exit_code==Some(7))
        );
        exited.acknowledge();
        drop(exited);
        assert!(stream.events.recv().await.is_none());
        server.await.unwrap();
    }
    #[test]
    fn control_diagnostics_are_bounded_without_splitting_unicode() {
        let text = "中".repeat(10000);
        let result = limited_diagnostic(&text, 512);
        assert!(result.len() <= 512);
        assert!(result.chars().all(|c| c == '中'));
    }
    #[tokio::test]
    async fn large_snapshot_is_applied_in_bounded_replay_batches() {
        let (tx, mut rx) = mpsc::channel(1);
        let local = Arc::new(Semaphore::new(LOCAL_BYTES));
        let copy = local.clone();
        let work = tokio::spawn(async move {
            let mut applied = 0;
            deliver_batched(
                &tx,
                &copy,
                TerminalEvent::Snapshot {
                    epoch: 1,
                    mode: StreamMode::Reset,
                    start_offset: 10,
                    next_offset: 10 + LOCAL_BYTES as u64,
                    data: vec![b'x'; LOCAL_BYTES],
                    state: state(true),
                    degraded: false,
                },
                &mut applied,
            )
            .await
            .unwrap();
            applied
        });
        let first = rx.recv().await.unwrap();
        assert_eq!(first.event.data_len(), UI_BATCH);
        assert_eq!(local.available_permits(), 0);
        assert!(!work.is_finished());
        let mut next = first.event.next_offset().unwrap();
        first.acknowledge();
        drop(first);
        let mut batches = 1;
        loop {
            let item = rx.recv().await.unwrap();
            let complete = match item.event.as_ref() {
                TerminalEvent::Output {
                    start_offset,
                    end_offset,
                    data,
                    replay,
                    ..
                } => {
                    assert!(*replay);
                    assert_eq!(*start_offset, next);
                    assert!(data.len() <= UI_BATCH);
                    next = *end_offset;
                    batches += 1;
                    false
                }
                TerminalEvent::State { state, .. } => {
                    assert!(state.output_complete);
                    true
                }
                other => panic!("unexpected event: {other:?}"),
            };
            item.acknowledge();
            drop(item);
            if complete {
                break;
            }
        }
        assert_eq!(batches, LOCAL_BYTES / UI_BATCH);
        assert_eq!(work.await.unwrap(), 10 + LOCAL_BYTES as u64);
        assert_eq!(local.available_permits(), LOCAL_BYTES);
    }
    #[tokio::test]
    async fn invalid_epoch_gap_duplicate_and_length_do_not_advance_ui_watermark() {
        for case in 0..4 {
            let dir = tempfile::Builder::new()
                .prefix("dz-invalid-")
                .tempdir_in("/tmp")
                .unwrap();
            let socket = dir.path().join("s");
            let listener = tokio::net::UnixListener::bind(&socket).unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let (r, mut w) = stream.into_split();
                let mut r = BufReader::new(r);
                let mut line = String::new();
                r.read_line(&mut line).await.unwrap();
                let first = TerminalEvent::Snapshot {
                    epoch: 42,
                    mode: StreamMode::Reset,
                    start_offset: 0,
                    next_offset: 1,
                    data: b"a".to_vec(),
                    state: state(false),
                    degraded: false,
                };
                let (epoch, start, end) = match case {
                    0 => (43, 1, 2),
                    1 => (42, 2, 3),
                    2 => (42, 0, 1),
                    _ => (42, 1, 4),
                };
                for event in [
                    first,
                    TerminalEvent::Output {
                        epoch,
                        start_offset: start,
                        end_offset: end,
                        data: b"b".to_vec(),
                        replay: false,
                    },
                ] {
                    w.write_all(encode_line(&Reply::Terminal { event }).as_bytes())
                        .await
                        .unwrap();
                }
            });
            let client = Client::new(socket);
            let (tx, mut rx) = mpsc::channel(1);
            let local = Arc::new(Semaphore::new(LOCAL_BYTES));
            let work = tokio::spawn(async move {
                let mut applied = 0;
                let result = client
                    .pump_stream("s", 42, true, &tx, &local, &mut applied)
                    .await;
                (result, applied)
            });
            let first = rx.recv().await.unwrap();
            first.acknowledge();
            drop(first);
            let (result, applied) = tokio::time::timeout(Duration::from_secs(2), work)
                .await
                .unwrap()
                .unwrap();
            assert!(result.is_err());
            assert_eq!(applied, 1);
            server.await.unwrap();
        }
    }
}
