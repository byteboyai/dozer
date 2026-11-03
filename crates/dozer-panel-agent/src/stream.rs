//! 生产终端流到 UI 的有界交付。消息只 Clone Arc，应用后显式 ack。
use crate::{
    state::AgentPanelState,
    update::{Message, UpdateCtx},
};
use dozer_model::{SessionInfo, StreamMode, TerminalEvent};
use dozer_services::{ServiceError, SessionService, SessionStream, StreamItem};
use std::sync::atomic::{AtomicU64, Ordering};

pub enum Connection {
    Legacy(dozer_services::SessionAttach),
    Stream(SessionStream),
}
impl Connection {
    pub fn snapshot(&self) -> Vec<u8> {
        match self {
            Self::Legacy((snapshot, ..)) => snapshot.clone(),
            Self::Stream(_) => Vec::new(),
        }
    }
}
static OWNER: AtomicU64 = AtomicU64::new(1);
pub(crate) fn next_owner() -> u64 {
    OWNER.fetch_add(1, Ordering::Relaxed)
}
pub async fn connect(svc: &dyn SessionService, id: String) -> Result<Connection, ServiceError> {
    match svc.attach_stream(id.clone(), 0).await {
        Ok(stream) => Ok(Connection::Stream(stream)),
        Err(e) if e.message == "terminal stream unsupported" => {
            svc.attach(id, 0).await.map(Connection::Legacy)
        }
        Err(e) => Err(e),
    }
}
pub async fn forward(
    tab_id: usize,
    owner: u64,
    connection: Connection,
    emit: impl Fn(Message) + Send + 'static,
) {
    match connection {
        Connection::Legacy((_, _, rx)) => crate::update::forward_events(tab_id, rx, emit).await,
        Connection::Stream(mut stream) => {
            while let Some(item) = stream.events.recv().await {
                emit(Message::Stream {
                    tab_id,
                    owner,
                    item,
                });
            }
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ready,
    Recovering,
    Failed,
    Degraded,
    HistoryIncomplete,
}

pub(crate) fn apply(
    state: &mut AgentPanelState,
    ctx: &UpdateCtx<'_>,
    tab_id: usize,
    owner: u64,
    item: StreamItem,
) {
    if state.stream_owners.get(&tab_id) != Some(&owner) {
        return;
    }
    let epoch = item.event.epoch();
    let current = *state.stream_epochs.get(&tab_id).unwrap_or(&0);
    if epoch < current {
        return;
    }
    match item.event.as_ref() {
        TerminalEvent::Recovering { .. } => {
            state.stream_status.insert(tab_id, Status::Recovering);
            state.stream_epochs.insert(tab_id, epoch);
        }
        TerminalEvent::Failed { message, .. } => {
            state.stream_errors.insert(tab_id, message.clone());
            state.stream_epochs.insert(tab_id, epoch);
            state.stream_status.insert(tab_id, Status::Failed);
        }
        TerminalEvent::Snapshot {
            mode,
            data,
            state: snapshot,
            degraded,
            ..
        } => {
            state.stream_epochs.insert(tab_id, epoch);
            state.stream_errors.remove(&tab_id);
            state.stream_versions.insert(
                tab_id,
                (snapshot.lifecycle_revision, snapshot.agent_revision),
            );
            state.stream_status.insert(
                tab_id,
                if *degraded {
                    Status::Degraded
                } else if snapshot.output_truncated || (*mode == StreamMode::Reset && current > 0) {
                    Status::HistoryIncomplete
                } else {
                    Status::Ready
                },
            );
            if let Some(tab) = state.tab_by_id_mut(tab_id) {
                if *mode == StreamMode::Reset {
                    tab.model = dozer_panel_term::model::TerminalModel::new(ctx.cols, ctx.rows);
                    tab.model.set_answer_dynamic_color(
                        snapshot.agent == dozer_model::AgentKind::Opencode,
                    );
                    tab.osc = dozer_panel_term::osc::OscScanner::new();
                    tab.cwd = None;
                    tab.last_exit = None;
                }
                tab.ingest_osc(data);
                // Replayed terminal queries must not be written back as live input.
                let _responses = tab.model.feed(data);
                tab.agent = snapshot.agent;
                tab.agent_state = snapshot.agent_state;
                tab.transcript_path = snapshot.transcript_path.clone();
                tab.alive = !snapshot.output_complete;
                if snapshot.output_complete {
                    let _ = tab.model.feed(&crate::tabs::exited_marker());
                } else {
                    crate::tabs::resize_tab(tab, ctx.svc, ctx.handle, ctx.cols, ctx.rows);
                }
            }
        }
        TerminalEvent::Output { data, replay, .. } => {
            if epoch != current || state.stream_status.get(&tab_id) == Some(&Status::Recovering) {
                return;
            }
            if let Some(tab) = state.tab_by_id_mut(tab_id) {
                tab.ingest_osc(data);
                let responses = tab.model.feed(data);
                if tab.alive && !*replay && !responses.is_empty() {
                    crate::tabs::send_to_tab(tab, ctx.svc, ctx.handle, responses);
                }
            }
        }
        TerminalEvent::State {
            state: snapshot, ..
        } => {
            if epoch != current {
                return;
            }
            if snapshot.output_truncated {
                state
                    .stream_status
                    .insert(tab_id, Status::HistoryIncomplete);
            }
            let versions = state.stream_versions.entry(tab_id).or_insert((0, 0));
            let agent_current = snapshot.agent_revision >= versions.1;
            let lifecycle_current = snapshot.lifecycle_revision >= versions.0;
            if agent_current {
                versions.1 = snapshot.agent_revision;
            }
            if lifecycle_current {
                versions.0 = snapshot.lifecycle_revision;
            }
            if let Some(tab) = state.tab_by_id_mut(tab_id) {
                if agent_current {
                    tab.agent = snapshot.agent;
                    tab.agent_state = snapshot.agent_state;
                    tab.transcript_path = snapshot.transcript_path.clone();
                }
                if lifecycle_current && snapshot.output_complete && tab.alive {
                    tab.alive = false;
                    let _ = tab.model.feed(&crate::tabs::exited_marker());
                    state
                        .host_events
                        .push(crate::adapter::AgentHostEvent::SessionExited { tab_id });
                }
            }
        }
        TerminalEvent::ResyncRequired { .. } => return,
    }
    item.acknowledge();
}

pub fn adopt(
    state: &mut AgentPanelState,
    info: SessionInfo,
    connection: Connection,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Clone + Send + 'static,
) {
    match connection {
        Connection::Legacy((snapshot, _, events)) => state.adopt_restored(
            vec![crate::restore::RestoredSession {
                info,
                snapshot,
                events,
            }],
            handle,
            emit,
        ),
        connection => {
            let tab_id = state.next_tab_id;
            state.next_tab_id += 1;
            let owner = next_owner();
            state.stream_owners.insert(tab_id, owner);
            let forwarder = handle.spawn(forward(tab_id, owner, connection, emit));
            let mut model = dozer_panel_term::model::TerminalModel::new(80, 24);
            model.set_answer_dynamic_color(info.agent == dozer_model::AgentKind::Opencode);
            state.tabs.push(crate::tabs::SessionTab {
                agent_state: info.agent_state,
                agent: info.agent,
                transcript_path: info.transcript_path.clone(),
                info,
                model,
                alive: true,
                tab_id,
                forwarder,
                osc: dozer_panel_term::osc::OscScanner::new(),
                cwd: None,
                last_exit: None,
                llm_model: None,
                permission_mode: None,
                last_activity: None,
                workspace_override: None,
                backend: crate::tabs::TabBackend::Daemon,
            });
        }
    }
}
