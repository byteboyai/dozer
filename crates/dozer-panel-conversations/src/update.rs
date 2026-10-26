use crate::{
    adapter::{ConvHover, ConversationsCapabilities, ConversationsRequest, TextInputTarget},
    filter::conversation_agents_present,
    review::{ReviewEntry, merge_review_entries},
    snapshot::{
        ReviewSnapshot, ReviewSource, ReviewView, mark_review_loaded,
        refresh_open_conversation_summary,
    },
    state::WorkspaceState,
};
use dozer_model::{AgentKind, SessionRow};
use dozer_panel_host::RequestTokenWrapExt;
use dozer_services::{ConversationService, ServiceError, ServiceErrorKind, SummaryService};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
pub struct UpdateCtx<'a> {
    pub project_id: i64,
    pub project_path: &'a Path,
    pub conversation: &'a Arc<dyn ConversationService>,
    pub summary: &'a Arc<dyn SummaryService>,
    pub handle: &'a tokio::runtime::Handle,
    pub caps: ConversationsCapabilities,
    pub locale: &'a str,
    pub now_ms: u64,
    pub relative_time: &'a dyn Fn(u64, u64, &str) -> String,
}
/// Panel messages. The library owns list, review and summary transitions;
/// the host maps hover and text input menu messages to its desktop facilities.
#[derive(Debug, Clone)]
pub enum Message {
    AsyncResult(bytehost_core::request::RequestToken, Box<Message>),
    SessionsRefreshed(i64, Result<Vec<SessionRow>, ServiceError>),
    SessionOpen(String, AgentKind),
    DetailLoadMore(String, i64),
    ReviewLoaded {
        source: ReviewSource,
        append: bool,
        result: Result<Vec<ReviewEntry>, String>,
    },
    /// The legacy event carries no nonce: it marks the current review nonce.
    ReviewTraceLoaded,
    SummaryGenerate(String, AgentKind),
    SummaryGenerateFinished(i64, String, Result<(), dozer_model::LocalizedText>),
    ListMore,
    SearchInput(String),
    SearchSubmit,
    AgentFilterSelect(Option<AgentKind>),
    AgentFilterOpen {
        at: (f32, f32),
    },
    AgentFilterClose,
    Hover(ConvHover, bool),
    TextInputMenuOpen(TextInputTarget),
}

pub fn update(
    ws_state: &mut WorkspaceState,
    msg: Message,
    ctx: &UpdateCtx<'_>,
    emit: impl Fn(Message) + Clone + Send + 'static,
) {
    let mut msg = msg;
    while let Message::AsyncResult(token, inner) = msg {
        if !ws_state.requests.accepts(&token) {
            return;
        }
        msg = *inner;
    }
    match msg {
        Message::AsyncResult(..) => unreachable!("nested async result"),
        Message::SessionsRefreshed(_, result) => match result {
            Ok(rows) => {
                ws_state.sessions = Some(rows);
                let row = ws_state
                    .current_conversation_id()
                    .and_then(|cid| {
                        ws_state
                            .sessions()
                            .and_then(|rows| rows.iter().find(|row| row.conversation_id == cid))
                    })
                    .cloned();
                if let Some(row) = row
                    && let Some(json) = refresh_open_conversation_summary(
                        &mut ws_state.review,
                        &mut ws_state.review_nonce,
                        &row,
                    )
                {
                    *ws_state.review_snapshot.lock().expect("review snapshot 锁") = Some(json);
                }
            }
            Err(e) => {
                // 首次加载失败:置空列表(不停在 None,界面不会一直"加载中");已有列表则保留。
                if ws_state.sessions.is_none() {
                    ws_state.sessions = Some(Vec::new());
                }
                if e.kind == ServiceErrorKind::Unavailable {
                    crate::log::debug("对话会话列表查询失败(dozerd 不可用)", &e.message);
                } else {
                    ws_state.outbox.push_keyed_cooldown(
                        dozer_panel_host::NoticeLevel::Error,
                        crate::i18n::keyed_args(
                            "error.list_load_failed",
                            "Failed to load conversations: {e}",
                            &[("e", e.message.as_str())],
                        ),
                        "conversations-load",
                        std::time::Instant::now(),
                    );
                }
            }
        },
        Message::ListMore => ws_state.pages += 1,
        Message::SearchInput(s) => ws_state.search_draft = s,
        Message::SearchSubmit => {
            ws_state.search = ws_state.search_draft.clone();
            ws_state.pages = 0;
        }
        Message::AgentFilterSelect(agent) => {
            ws_state.agent_filter = agent;
            ws_state.pages = 0;
            ws_state.agent_picker_open = false;
        }
        Message::AgentFilterOpen { at } => {
            ws_state
                .host_requests
                .push(ConversationsRequest::ShowAgentFilter {
                    at,
                    current: ws_state.agent_filter,
                    present: conversation_agents_present(ws_state.sessions().unwrap_or(&[])),
                })
        }
        Message::AgentFilterClose => ws_state.agent_picker_open = false,
        Message::SessionOpen(cid, agent) => {
            let Some(row) = ws_state
                .sessions()
                .and_then(|rows| rows.iter().find(|row| row.conversation_id == cid))
            else {
                return;
            };
            ws_state.review = Some(ReviewView {
                source: ReviewSource::Conversation(cid.clone()),
                entries: vec![],
                error: None,
                agent,
                nonce: 0,
                summary_title: Some(row.display_title.clone()),
                summary_text: row.summary.clone(),
                summary_time: Some((ctx.relative_time)(row.last_ts, ctx.now_ms, ctx.locale)),
                loaded_nonce: None,
            });
            spawn_detail(ws_state, ctx, cid, -1, false, emit);
        }
        Message::DetailLoadMore(cid, after) => {
            if ws_state.current_conversation_id() == Some(cid.as_str()) {
                spawn_detail(ws_state, ctx, cid, after, true, emit);
            }
        }
        Message::ReviewLoaded {
            source,
            append,
            result,
        } => {
            let Some(rv) = ws_state.review.as_mut().filter(|rv| rv.source == source) else {
                return;
            };
            match result {
                Ok(entries) => {
                    ws_state.review_nonce = ws_state.review_nonce.wrapping_add(1);
                    merge_review_entries(&mut rv.entries, entries, append);
                    rv.nonce = ws_state.review_nonce;
                    rv.error = None;
                    let snapshot = ReviewSnapshot {
                        entries: &rv.entries,
                        agent_label: rv.agent.label(),
                        summary_title: rv.summary_title.clone(),
                        summary_text: rv.summary_text.clone(),
                        summary_time: rv.summary_time.clone(),
                    };
                    *ws_state.review_snapshot.lock().expect("review snapshot 锁") =
                        Some(serde_json::to_string(&snapshot).unwrap_or_default());
                }
                Err(error) => rv.error = Some(error),
            }
        }
        Message::ReviewTraceLoaded => mark_review_loaded(&mut ws_state.review),
        Message::SummaryGenerate(cid, _) => {
            if !ctx.caps.summaries || !ws_state.start_summary_generation(cid.clone()) {
                return;
            }
            let summary = ctx.summary.clone();
            let conversation = ctx.conversation.clone();
            let project_id = ctx.project_id;
            let path = ctx.project_path.to_path_buf();
            let requests = ws_state.requests.clone();
            // Scope-only delivery guard survives other lanes while rejecting a reopened workspace.
            let emit = ws_state
                .requests
                .scope(Some(project_id))
                .wrap(emit, Message::AsyncResult);
            ctx.handle.spawn(crate::summary::run_summary_job(
                crate::summary::SummaryJobCtx {
                    summary,
                    conversation,
                    requests,
                    project_id,
                    cwd: path,
                    conversation_id: cid,
                    poll_interval: std::time::Duration::from_secs(1),
                },
                emit,
            ));
        }
        Message::SummaryGenerateFinished(_, cid, result) => {
            ws_state.finish_summary_generation(cid, result)
        }
        Message::Hover(..) | Message::TextInputMenuOpen(..) => {}
    }
}

fn spawn_detail(
    state: &WorkspaceState,
    ctx: &UpdateCtx<'_>,
    cid: String,
    after: i64,
    append: bool,
    emit: impl Fn(Message) + Send + 'static,
) {
    let service = ctx.conversation.clone();
    let source = ReviewSource::Conversation(cid.clone());
    let emit = state
        .requests
        .begin(Some(ctx.project_id), "detail")
        .wrap(emit, Message::AsyncResult);
    ctx.handle.spawn(async move {
        let result = service
            .get_conversation_turns(cid, after, crate::filter::CONVERSATION_DETAIL_PAGE_SIZE)
            .await
            .map(|turns| crate::review::review_entries_from_turns(&turns))
            .map_err(|e| e.to_string());
        emit(Message::ReviewLoaded {
            source,
            append,
            result,
        });
    });
}
pub fn spawn_refresh(
    requests: &dozer_panel_host::RequestTrackers,
    project_id: i64,
    cwd: PathBuf,
    service: Arc<dyn ConversationService>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let emit = requests
        .begin(Some(project_id), "sessions")
        .wrap(emit, Message::AsyncResult);
    handle.spawn(async move {
        let result = service
            .list_conversations_with_summaries(cwd.to_string_lossy().into_owned(), None, 500, 0)
            .await
            .map(|rows| {
                rows.iter()
                    .map(|(c, s)| SessionRow::from_row(c, s.as_ref()))
                    .collect()
            });
        emit(Message::SessionsRefreshed(project_id, result));
    });
}

pub fn on_activate(
    state: &WorkspaceState,
    ctx: &UpdateCtx<'_>,
    emit: impl Fn(Message) + Send + 'static,
) {
    spawn_refresh(
        &state.requests,
        ctx.project_id,
        ctx.project_path.to_path_buf(),
        ctx.conversation.clone(),
        ctx.handle,
        emit,
    );
}
impl Message {
    pub fn result_project(&self) -> Option<i64> {
        match self {
            Self::AsyncResult(token, _) => dozer_panel_host::project_of(token.scope()),
            Message::SessionsRefreshed(id, _) | Message::SummaryGenerateFinished(id, ..) => {
                Some(*id)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
impl Message {
    pub fn result_payload(self) -> Self {
        match self {
            Self::AsyncResult(_, inner) => inner.result_payload(),
            other => other,
        }
    }
}
#[cfg(test)]
mod legacy_tests {
    use super::*;
    use crate::state::WorkspaceState;
    use crate::task3_tests::apply;

    #[test]
    fn summary_generation_state_prevents_duplicates_and_keeps_errors_visible() {
        let mut state = WorkspaceState::default();

        assert!(state.start_summary_generation("conversation-1".into()));
        assert!(state.summary_generating("conversation-1"));
        assert!(!state.start_summary_generation("conversation-1".into()));

        state.finish_summary_generation(
            "conversation-1".into(),
            Err(crate::i18n::keyed("test.summary_error", "provider 超时")),
        );
        assert!(!state.summary_generating("conversation-1"));
        assert_eq!(
            state
                .summary_error("conversation-1")
                .map(|t| crate::i18n::render(t, "zh-CN")),
            Some("provider 超时".to_string())
        );

        assert!(state.start_summary_generation("conversation-1".into()));
        assert_eq!(state.summary_error("conversation-1"), None);
        state.finish_summary_generation("conversation-1".into(), Ok(()));
        assert!(!state.summary_generating("conversation-1"));
        assert_eq!(state.summary_error("conversation-1"), None);
    }

    #[test]
    fn search_submit_commits_draft_and_resets_pages() {
        let mut ws_state = WorkspaceState {
            search_draft: "foo".into(),
            pages: 3,
            ..WorkspaceState::default()
        };
        apply(&mut ws_state, Message::SearchSubmit);
        assert_eq!(ws_state.search, "foo");
        assert_eq!(ws_state.pages, 0);
    }

    #[test]
    fn agent_filter_select_resets_pages_and_closes_picker() {
        let mut ws_state = WorkspaceState {
            pages: 2,
            agent_picker_open: true,
            ..WorkspaceState::default()
        };
        apply(
            &mut ws_state,
            Message::AgentFilterSelect(Some(AgentKind::Claude)),
        );
        assert_eq!(ws_state.agent_filter, Some(AgentKind::Claude));
        assert_eq!(ws_state.pages, 0);
        assert!(!ws_state.agent_picker_open);
    }

    #[test]
    fn sessions_refreshed_err_lands_as_empty_vec_not_none() {
        let mut ws_state = WorkspaceState::default();
        apply(
            &mut ws_state,
            Message::SessionsRefreshed(
                1,
                Err(ServiceError::new(ServiceErrorKind::Protocol, "boom")),
            ),
        );
        assert_eq!(ws_state.sessions(), Some([].as_slice()));
    }

    // ---- H6:切入钩子 ----

    use crate::fakes::{FakeConversation, conversation_row};
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    }
    fn sample_session(id: &str) -> SessionRow {
        let (c, s) = conversation_row(id, "标题");
        SessionRow::from_row(&c, s.as_ref())
    }

    #[test]
    fn spawn_refresh_maps_rows_and_passes_the_arguments_through() {
        let rt = runtime();
        let fake = FakeConversation::default().with_rows(vec![conversation_row("c1", "标题")]);
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        spawn_refresh(
            &Default::default(),
            7,
            PathBuf::from("/tmp/p"),
            Arc::new(fake.clone()),
            rt.handle(),
            move |m| {
                let _ = tx.lock().unwrap().send(m);
            },
        );
        match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
            Message::SessionsRefreshed(7, Ok(rows)) => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].conversation_id, "c1");
                assert_eq!(rows[0].display_title, "标题");
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(
            fake.calls(),
            vec!["list_conversations_with_summaries(/tmp/p,None,500,0)".to_string()]
        );
    }

    #[test]
    fn a_failing_service_emits_the_error_unchanged() {
        let rt = runtime();
        let fake = FakeConversation::failing(ServiceErrorKind::Rejected, "daemon 错误: boom");
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        spawn_refresh(
            &Default::default(),
            7,
            PathBuf::from("/tmp/p"),
            Arc::new(fake),
            rt.handle(),
            move |m| {
                let _ = tx.lock().unwrap().send(m);
            },
        );
        match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
            Message::SessionsRefreshed(7, Err(e)) => {
                assert_eq!(
                    (e.kind, e.message.as_str()),
                    (ServiceErrorKind::Rejected, "daemon 错误: boom")
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn first_load_failure_shows_an_empty_list_not_a_stuck_loading_state() {
        let mut ws = WorkspaceState::default();
        apply(
            &mut ws,
            Message::SessionsRefreshed(
                1,
                Err(ServiceError::new(
                    ServiceErrorKind::Unavailable,
                    "daemon 断开",
                )),
            ),
        );
        assert_eq!(ws.sessions(), Some([].as_slice()));
        assert!(
            ws.take_outbox().is_empty(),
            "Unavailable 由顶栏徽标表达,不弹 Toast"
        );
    }

    #[test]
    fn a_failed_refresh_keeps_the_sessions_already_shown() {
        let mut ws = WorkspaceState::default();
        apply(
            &mut ws,
            Message::SessionsRefreshed(1, Ok(vec![sample_session("c1")])),
        );
        apply(
            &mut ws,
            Message::SessionsRefreshed(
                1,
                Err(ServiceError::new(
                    ServiceErrorKind::Unavailable,
                    "daemon 断开",
                )),
            ),
        );
        assert_eq!(
            ws.sessions().map(|s| s.len()),
            Some(1),
            "刷新失败不得抹掉已显示的会话"
        );
    }

    #[test]
    fn repeated_rejected_failures_toast_once_per_text_with_a_key() {
        let mut ws = WorkspaceState::default();
        apply(
            &mut ws,
            Message::SessionsRefreshed(1, Ok(vec![sample_session("c1")])),
        );
        for _ in 0..3 {
            apply(
                &mut ws,
                Message::SessionsRefreshed(
                    1,
                    Err(ServiceError::new(
                        ServiceErrorKind::Rejected,
                        "daemon 错误: boom",
                    )),
                ),
            );
        }
        let toasts = ws.take_outbox();
        assert_eq!(toasts.len(), 1, "相同文本 60 秒内只提示一次");
        assert_eq!(toasts[0].level, dozer_panel_host::NoticeLevel::Error);
        assert!(crate::i18n::render(&toasts[0].text, "en").contains("daemon 错误: boom"));
        assert_eq!(toasts[0].key.as_deref(), Some("conversations-load"));
        assert_eq!(ws.sessions().map(|s| s.len()), Some(1));
        apply(
            &mut ws,
            Message::SessionsRefreshed(
                1,
                Err(ServiceError::new(ServiceErrorKind::Protocol, "另一种错误")),
            ),
        );
        assert_eq!(ws.take_outbox().len(), 1, "换了文本要再提示");
    }

    #[test]
    fn stale_async_sessions_and_failure_do_not_replace_latest_rows() {
        let mut state = WorkspaceState::default();
        let old = state.requests.begin(Some(1), "sessions");
        let new = state.requests.begin(Some(1), "sessions");
        apply(
            &mut state,
            Message::AsyncResult(
                new.clone(),
                Box::new(Message::SessionsRefreshed(
                    1,
                    Ok(vec![sample_session("new")]),
                )),
            ),
        );
        apply(
            &mut state,
            Message::AsyncResult(
                old.clone(),
                Box::new(Message::SessionsRefreshed(1, Ok(vec![]))),
            ),
        );
        apply(
            &mut state,
            Message::AsyncResult(
                old,
                Box::new(Message::SessionsRefreshed(
                    1,
                    Err(ServiceError::new(ServiceErrorKind::Rejected, "old failure")),
                )),
            ),
        );
        assert_eq!(state.sessions().unwrap()[0].conversation_id, "new");
        assert!(state.take_outbox().is_empty());
        let mut reopened = WorkspaceState::default();
        apply(
            &mut reopened,
            Message::AsyncResult(new, Box::new(Message::SessionsRefreshed(1, Ok(vec![])))),
        );
        assert!(reopened.sessions().is_none());
    }
}
