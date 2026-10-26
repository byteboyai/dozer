use crate::{
    Message,
    adapter::*,
    fakes::{FakeConversation, FakeSummary, conversation_row},
    review::ReviewEntry,
    snapshot::{ReviewPaneState, ReviewSource, review_pane_state},
    state::WorkspaceState,
    update::{self, UpdateCtx},
};
use dozer_model::{
    AgentKind, SessionRow, SummaryJobInfo, SummaryJobStatus, SummaryTrigger, TurnRecord,
};
use dozer_services::{
    BoxFuture, ConversationRows, ConversationService, ServiceError, ServiceErrorKind,
    SummaryService,
};
use std::{
    path::Path,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};
const TIMEOUT: Duration = Duration::from_secs(3);
fn relative_time(last: u64, now: u64, locale: &str) -> String {
    format!("{locale}:{last}:{now}")
}
struct Fixture {
    rt: tokio::runtime::Runtime,
    conversation: Arc<dyn ConversationService>,
    summary: Arc<dyn SummaryService>,
}
impl Fixture {
    fn new(
        conversation: impl ConversationService + 'static,
        summary: impl SummaryService + 'static,
    ) -> Self {
        Self {
            rt: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap(),
            conversation: Arc::new(conversation),
            summary: Arc::new(summary),
        }
    }
    fn ctx(&self) -> UpdateCtx<'_> {
        UpdateCtx {
            project_id: 7,
            project_path: Path::new("/tmp/p"),
            conversation: &self.conversation,
            summary: &self.summary,
            handle: self.rt.handle(),
            caps: ConversationsCapabilities::all(),
            locale: "zh-CN",
            now_ms: 200,
            relative_time: &relative_time,
        }
    }
    fn send(&self, state: &mut WorkspaceState, msg: Message) -> mpsc::Receiver<Message> {
        let (tx, rx) = mpsc::channel();
        update::update(state, msg, &self.ctx(), move |m| {
            tx.send(m).unwrap();
        });
        rx
    }
}
pub(crate) fn apply(state: &mut WorkspaceState, msg: Message) {
    Fixture::new(FakeConversation::default(), FakeSummary::default()).send(state, msg);
}
fn row(cid: &str) -> SessionRow {
    let (c, s) = conversation_row(cid, "标题");
    SessionRow::from_row(&c, s.as_ref())
}
fn state() -> WorkspaceState {
    let mut state = WorkspaceState::default();
    apply(
        &mut state,
        Message::SessionsRefreshed(7, Ok(vec![row("a"), row("b")])),
    );
    state
}
fn turn(index: i64, text: &str) -> TurnRecord {
    serde_json::from_value(serde_json::json!({"turn_index":index,"role":"human","content":text,"thinking":false,"ts":null,"tokens_in":0,"tokens_out":0,"tokens_cache_read":0,"tokens_cache_write":0})).unwrap()
}
fn entries(text: &str) -> Vec<ReviewEntry> {
    vec![ReviewEntry::Human { text: text.into() }]
}
fn open(f: &Fixture, state: &mut WorkspaceState, cid: &str) {
    let rx = f.send(state, Message::SessionOpen(cid.into(), AgentKind::Claude));
    f.send(state, rx.recv_timeout(TIMEOUT).unwrap());
}
fn snapshot(state: &WorkspaceState) -> Option<String> {
    state.review_snapshot().lock().unwrap().clone()
}
#[test]
fn opening_a_row_loads_turns_and_publishes_a_snapshot_with_summary_fields() {
    let fake = FakeConversation::default().with_turns("a", vec![turn(0, "hello")]);
    let f = Fixture::new(fake.clone(), FakeSummary::default());
    let mut state = state();
    state.sessions.as_mut().unwrap()[0].summary = Some("总结".into());
    open(&f, &mut state, "a");
    let json: serde_json::Value = serde_json::from_str(&snapshot(&state).unwrap()).unwrap();
    assert_eq!(json["summary_title"], "标题");
    assert_eq!(json["summary_text"], "总结");
    assert_eq!(json["summary_time"], "zh-CN:0:200");
    assert_eq!(state.review.as_ref().unwrap().entries, entries("hello"));
    assert_eq!(fake.calls(), vec!["get_conversation_turns(a,-1,200)"]);
}
#[test]
fn load_more_appends_instead_of_replacing() {
    let fake =
        FakeConversation::default().with_turns("a", vec![turn(0, "first"), turn(2, "second")]);
    let f = Fixture::new(fake, FakeSummary::default());
    let mut state = state();
    open(&f, &mut state, "a");
    state.review.as_mut().unwrap().entries = entries("first");
    let rx = f.send(&mut state, Message::DetailLoadMore("a".into(), 1));
    f.send(&mut state, rx.recv_timeout(TIMEOUT).unwrap());
    assert_eq!(
        state.review.as_ref().unwrap().entries,
        [entries("first"), entries("second")].concat()
    );
}
#[test]
fn stale_review_results_for_another_source_are_dropped_and_do_not_touch_the_snapshot() {
    let f = Fixture::new(
        FakeConversation::default().with_turns("b", vec![turn(0, "B")]),
        FakeSummary::default(),
    );
    let mut state = state();
    open(&f, &mut state, "b");
    let before = snapshot(&state);
    let nonce = state.review_nonce;
    f.send(
        &mut state,
        Message::ReviewLoaded {
            source: ReviewSource::Conversation("a".into()),
            append: false,
            result: Ok(entries("A")),
        },
    );
    assert_eq!(snapshot(&state), before);
    assert_eq!(state.review_nonce, nonce);
}
#[test]
fn opening_a_row_that_vanished_from_the_list_does_nothing() {
    let fake = FakeConversation::default();
    let f = Fixture::new(fake.clone(), FakeSummary::default());
    let mut state = state();
    let rx = f.send(
        &mut state,
        Message::SessionOpen("gone".into(), AgentKind::Claude),
    );
    assert!(state.review.is_none());
    assert!(rx.recv_timeout(Duration::from_millis(10)).is_err());
    assert!(fake.calls().is_empty());
}
type PendingTurns = (String, tokio::sync::oneshot::Sender<Vec<TurnRecord>>);
struct DelayedConversation {
    ready: mpsc::Sender<PendingTurns>,
}
impl ConversationService for DelayedConversation {
    fn get_conversation_turns(
        &self,
        cid: String,
        _after: i64,
        _limit: u32,
    ) -> BoxFuture<'_, Result<Vec<TurnRecord>, ServiceError>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.ready.send((cid, tx)).unwrap();
        Box::pin(async move { Ok(rx.await.unwrap()) })
    }
    fn list_conversations_with_summaries(
        &self,
        _cwd: String,
        _agent: Option<AgentKind>,
        _limit: u32,
        _offset: u32,
    ) -> BoxFuture<'_, Result<ConversationRows, ServiceError>> {
        Box::pin(async { Ok(vec![]) })
    }
}
fn delayed() -> (Fixture, mpsc::Receiver<PendingTurns>) {
    let (tx, rx) = mpsc::channel();
    (
        Fixture::new(DelayedConversation { ready: tx }, FakeSummary::default()),
        rx,
    )
}
#[test]
fn two_quick_opens_show_only_the_last_one_even_if_the_first_result_arrives_last() {
    let (f, ready) = delayed();
    let mut state = state();
    let a = f.send(
        &mut state,
        Message::SessionOpen("a".into(), AgentKind::Claude),
    );
    let (cid, finish_a) = ready.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(cid, "a");
    let b = f.send(
        &mut state,
        Message::SessionOpen("b".into(), AgentKind::Claude),
    );
    let (_, finish_b) = ready.recv_timeout(TIMEOUT).unwrap();
    finish_b.send(vec![turn(0, "B")]).unwrap();
    f.send(&mut state, b.recv_timeout(TIMEOUT).unwrap());
    let before = snapshot(&state);
    finish_a.send(vec![turn(0, "A")]).unwrap();
    f.send(&mut state, a.recv_timeout(TIMEOUT).unwrap());
    assert_eq!(state.current_conversation_id(), Some("b"));
    assert_eq!(snapshot(&state), before);
    assert_eq!(state.review.as_ref().unwrap().entries, entries("B"));
}
#[test]
fn append_finishing_after_switch_is_dropped_even_if_the_source_is_reopened() {
    let (f, ready) = delayed();
    let mut state = state();
    let a = f.send(
        &mut state,
        Message::SessionOpen("a".into(), AgentKind::Claude),
    );
    ready
        .recv_timeout(TIMEOUT)
        .unwrap()
        .1
        .send(vec![turn(0, "first")])
        .unwrap();
    f.send(&mut state, a.recv_timeout(TIMEOUT).unwrap());
    let append = f.send(&mut state, Message::DetailLoadMore("a".into(), 0));
    let (_, finish_append) = ready.recv_timeout(TIMEOUT).unwrap();
    let b = f.send(
        &mut state,
        Message::SessionOpen("b".into(), AgentKind::Claude),
    );
    ready
        .recv_timeout(TIMEOUT)
        .unwrap()
        .1
        .send(vec![turn(0, "B")])
        .unwrap();
    f.send(&mut state, b.recv_timeout(TIMEOUT).unwrap());
    let reopened = f.send(
        &mut state,
        Message::SessionOpen("a".into(), AgentKind::Claude),
    );
    ready
        .recv_timeout(TIMEOUT)
        .unwrap()
        .1
        .send(vec![turn(0, "fresh A")])
        .unwrap();
    f.send(&mut state, reopened.recv_timeout(TIMEOUT).unwrap());
    let before = snapshot(&state);
    finish_append.send(vec![turn(1, "old append")]).unwrap();
    f.send(&mut state, append.recv_timeout(TIMEOUT).unwrap());
    assert_eq!(snapshot(&state), before);
}
#[test]
fn review_load_failure_sets_the_error_and_the_pane_state_is_ready_not_loading() {
    let f = Fixture::new(
        FakeConversation::default().failing_turn_reads(ServiceErrorKind::Rejected, "boom"),
        FakeSummary::default(),
    );
    let mut state = state();
    open(&f, &mut state, "a");
    assert!(
        state
            .review
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("boom")
    );
    assert_eq!(
        review_pane_state(state.review.as_ref()),
        ReviewPaneState::Ready
    );
}
#[test]
fn summary_generate_is_ignored_without_the_capability() {
    let fake = FakeSummary::default();
    let f = Fixture::new(FakeConversation::default(), fake.clone());
    let mut state = state();
    let mut ctx = f.ctx();
    ctx.caps = ConversationsCapabilities::default();
    update::update(
        &mut state,
        Message::SummaryGenerate("a".into(), AgentKind::Claude),
        &ctx,
        |_| {},
    );
    assert!(!state.summary_generating("a"));
    assert!(fake.calls().is_empty());
}
#[test]
fn summary_generate_twice_submits_once() {
    let fake = FakeSummary::default();
    let f = Fixture::new(FakeConversation::default(), fake.clone());
    let mut state = state();
    let _first = f.send(
        &mut state,
        Message::SummaryGenerate("a".into(), AgentKind::Claude),
    );
    let _second = f.send(
        &mut state,
        Message::SummaryGenerate("a".into(), AgentKind::Claude),
    );
    for _ in 0..100 {
        if !fake.calls().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(fake.calls(), vec!["submit_summary_job(a,true)"]);
    assert!(state.summary_generating("a"));
}
fn job(status: SummaryJobStatus, error: Option<&str>) -> SummaryJobInfo {
    SummaryJobInfo {
        job_id: 1,
        conversation_id: "a".into(),
        source_session_id: None,
        trigger: SummaryTrigger::Manual,
        provider: AgentKind::Claude,
        requested_model: None,
        source_revision: "r".into(),
        pipeline_version: "v".into(),
        generation: 1,
        status,
        attempt: 1,
        error_kind: None,
        error_detail: error.map(str::to_owned),
        created_ts_ms: 0,
        updated_ts_ms: 0,
        phase: None,
    }
}
fn summary_messages(summary: FakeSummary, conversation: FakeConversation) -> Vec<Message> {
    summary_messages_with_requests(summary, conversation, Default::default())
}
fn summary_messages_with_requests(
    summary: FakeSummary,
    conversation: FakeConversation,
    requests: dozer_panel_host::RequestTrackers,
) -> Vec<Message> {
    use dozer_panel_host::RequestTokenWrapExt;
    let f = Fixture::new(conversation, summary);
    let (tx, rx) = mpsc::channel();
    let emit = requests
        .scope(Some(7))
        .wrap(move |msg| tx.send(msg).unwrap(), Message::AsyncResult);
    f.rt.block_on(crate::summary::run_summary_job(
        crate::summary::SummaryJobCtx {
            summary: f.summary.clone(),
            conversation: f.conversation.clone(),
            requests,
            project_id: 7,
            cwd: Path::new("/tmp/p").into(),
            conversation_id: "a".into(),
            poll_interval: Duration::from_millis(1),
        },
        emit,
    ));
    rx.try_iter().collect()
}
fn summary_error(summary: FakeSummary, conversation: FakeConversation) -> String {
    let messages = summary_messages(summary, conversation);
    match messages.into_iter().last().unwrap().result_payload() {
        Message::SummaryGenerateFinished(_, _, Err(error)) => crate::i18n::render(&error, "en"),
        other => panic!("{other:?}"),
    }
}
#[test]
fn summary_submit_failure_maps_to_the_legacy_text() {
    assert_eq!(
        summary_error(
            FakeSummary::default()
                .with_submission(Err(ServiceError::new(ServiceErrorKind::Rejected, "boom"))),
            FakeConversation::default()
        ),
        "⚠ Failed to submit summary task: boom"
    );
}
#[test]
fn summary_failed_with_detail_maps_to_the_legacy_text() {
    assert_eq!(
        summary_error(
            FakeSummary::default()
                .with_jobs(vec![Ok(Some(job(SummaryJobStatus::Failed, Some("bad"))))]),
            FakeConversation::default()
        ),
        "⚠ Failed to generate summary: bad"
    );
}
#[test]
fn summary_failed_without_detail_maps_to_the_legacy_text() {
    assert_eq!(
        summary_error(
            FakeSummary::default().with_jobs(vec![Ok(Some(job(SummaryJobStatus::Failed, None)))]),
            FakeConversation::default()
        ),
        "⚠ Failed to generate summary"
    );
}
#[test]
fn summary_cancelled_maps_to_the_legacy_text() {
    assert_eq!(
        summary_error(
            FakeSummary::default()
                .with_jobs(vec![Ok(Some(job(SummaryJobStatus::Cancelled, None)))]),
            FakeConversation::default()
        ),
        "Summary task cancelled"
    );
}
#[test]
fn summary_missing_job_maps_to_the_legacy_text() {
    assert_eq!(
        summary_error(FakeSummary::default(), FakeConversation::default()),
        "Summary task not found, please retry"
    );
}
#[test]
fn summary_poll_failure_maps_to_the_legacy_text() {
    assert_eq!(
        summary_error(
            FakeSummary::default().with_jobs(vec![Err(ServiceError::new(
                ServiceErrorKind::Rejected,
                "boom"
            ))]),
            FakeConversation::default()
        ),
        "Failed to query summary: boom"
    );
}
#[test]
fn summary_success_refresh_failure_maps_to_the_legacy_text() {
    assert_eq!(
        summary_error(
            FakeSummary::default()
                .with_jobs(vec![Ok(Some(job(SummaryJobStatus::Succeeded, None)))]),
            FakeConversation::failing(ServiceErrorKind::Rejected, "boom")
        ),
        "Summary generated, but refreshing sessions failed: boom"
    );
}
fn refresh_messages(state: &WorkspaceState) -> Vec<Message> {
    let mut c = conversation_row("a", "new title");
    c.0.title = "new title".into();
    summary_messages_with_requests(
        FakeSummary::default().with_jobs(vec![Ok(Some(job(SummaryJobStatus::Succeeded, None)))]),
        FakeConversation::default().with_rows(vec![c]),
        state.requests.clone(),
    )
}
#[test]
fn summary_success_refreshes_the_list_and_reopens_the_open_detail() {
    let f = Fixture::new(
        FakeConversation::default().with_turns("a", vec![turn(0, "unchanged")]),
        FakeSummary::default(),
    );
    let mut state = state();
    open(&f, &mut state, "a");
    let before = state.review.as_ref().unwrap().entries.clone();
    // Delivery uses the receiving workspace's request identity, as production does.
    for msg in refresh_messages(&state) {
        f.send(&mut state, msg);
    }
    assert_eq!(state.sessions().unwrap()[0].display_title, "new title");
    assert_eq!(
        state.review.as_ref().unwrap().summary_title.as_deref(),
        Some("new title")
    );
    assert_eq!(state.review.as_ref().unwrap().entries, before);
    assert!(snapshot(&state).unwrap().contains("new title"));
}
#[test]
fn summary_finishing_after_the_detail_changed_does_not_clobber_the_new_detail() {
    let f = Fixture::new(
        FakeConversation::default().with_turns("b", vec![turn(0, "B")]),
        FakeSummary::default(),
    );
    let mut state = state();
    open(&f, &mut state, "b");
    let before = snapshot(&state);
    for msg in refresh_messages(&state) {
        f.send(&mut state, msg);
    }
    assert_eq!(state.current_conversation_id(), Some("b"));
    assert_eq!(snapshot(&state), before);
}
#[test]
fn agent_filter_open_pushes_a_request_instead_of_opening_a_menu() {
    let mut state = state();
    apply(&mut state, Message::AgentFilterOpen { at: (2., 3.) });
    assert!(!state.agent_picker_open());
    assert_eq!(
        state.take_requests(),
        vec![ConversationsRequest::ShowAgentFilter {
            at: (2., 3.),
            current: None,
            present: vec![AgentKind::Claude]
        }]
    );
    assert!(state.take_requests().is_empty());
}
#[test]
fn agent_filter_select_resets_paging_and_closes() {
    let mut state = state();
    state.pages = 9;
    state.agent_picker_open = true;
    apply(
        &mut state,
        Message::AgentFilterSelect(Some(AgentKind::Claude)),
    );
    assert_eq!(state.pages, 0);
    assert!(!state.agent_picker_open);
    assert_eq!(state.agent_filter(), Some(AgentKind::Claude));
}
#[test]
fn search_submit_resets_paging() {
    let mut state = state();
    state.pages = 9;
    apply(&mut state, Message::SearchInput("中文".into()));
    apply(&mut state, Message::SearchSubmit);
    assert_eq!(state.pages, 0);
    assert_eq!(state.search, "中文");
}
#[test]
fn document_loaded_marks_only_the_current_nonce() {
    let f = Fixture::new(
        FakeConversation::default().with_turns("a", vec![turn(0, "A")]),
        FakeSummary::default(),
    );
    let mut state = state();
    state.review_nonce = u64::MAX;
    open(&f, &mut state, "a");
    assert_eq!(state.review_nonce, 0);
    state.review.as_mut().unwrap().nonce = 12;
    apply(&mut state, Message::ReviewTraceLoaded);
    assert_eq!(state.review.as_ref().unwrap().loaded_nonce, Some(12));
}
#[test]
fn sessions_refresh_failure_keeps_the_old_list_and_notifies_once_per_cooldown() {
    let mut state = state();
    let err = || {
        Message::SessionsRefreshed(
            7,
            Err(ServiceError::new(ServiceErrorKind::Protocol, "boom")),
        )
    };
    apply(&mut state, err());
    apply(&mut state, err());
    assert_eq!(state.sessions().unwrap().len(), 2);
    assert_eq!(state.take_outbox().len(), 1);
    // NoticeOutbox accepts a clock: expiration must preserve the legacy sixty-second cooldown.
    let mut outbox = dozer_panel_host::NoticeOutbox::default();
    let now = std::time::Instant::now();
    let text = crate::i18n::keyed("e", "boom");
    outbox.push_keyed_cooldown(dozer_panel_host::NoticeLevel::Error, text.clone(), "k", now);
    outbox.take();
    outbox.push_keyed_cooldown(
        dozer_panel_host::NoticeLevel::Error,
        text,
        "k",
        now + Duration::from_secs(60),
    );
    assert_eq!(outbox.take().len(), 1);
}
use byteui::interaction::icons::IconKind;
use iced_widget::core::{Border, Color, Element, Length};
#[derive(Default)]
struct Display {
    messages: Mutex<Vec<Message>>,
}
fn space<'a>() -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    iced_widget::Space::new().into()
}
impl PanelDisplay<Message> for Display {
    fn wrap(&self, msg: Message) -> Message {
        self.messages.lock().unwrap().push(msg.clone());
        msg
    }
    fn locale(&self) -> &str {
        "zh-CN"
    }
    fn hover_progress(&self, _: ConvHover) -> f32 {
        0.
    }
    fn panel_head<'a>(
        &self,
        _: IconKind,
        _: String,
        actions: Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        actions.unwrap_or_else(space)
    }
    fn list_collapse_button<'a>(
        &self,
        _: String,
        _: String,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        space()
    }
    fn tab_divider<'a>(&self) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        space()
    }
    fn pane_style(&self, _: ConversationsPane) -> PaneStyle {
        PaneStyle {
            padding: 0.into(),
            gap: 4.,
            background: None,
            header_padding: 0.into(),
            line_height: 1.2,
        }
    }
    fn agent_icon(
        &self,
        _: AgentKind,
        _: Color,
    ) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
        space()
    }
    fn agent_icon_color(&self, _: AgentKind) -> Color {
        Color::WHITE
    }
    fn relative_time(&self, last: u64, now: u64) -> String {
        relative_time(last, now, "zh-CN")
    }
    fn menu_item<'a>(
        &self,
        _: Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
        _: String,
        _: Color,
        _: Option<Message>,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        space()
    }
    fn menu_shell<'a>(
        &self,
        _: Vec<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        space()
    }
}
#[derive(Default)]
struct Todos(Mutex<usize>);
impl LinkedTodos for Todos {
    fn linked_text(&self, _: &str) -> Option<String> {
        *self.0.lock().unwrap() += 1;
        Some("关联待办".into())
    }
}
#[test]
fn list_view_hides_linked_todos_without_the_capability() {
    let state = state();
    let host = Display::default();
    let todos = Todos::default();
    let _view = crate::view::conversations_list(
        &host,
        &state,
        &todos,
        ConversationsCapabilities::default(),
        Length::Fill,
        Border::default(),
    );
    assert_eq!(*todos.0.lock().unwrap(), 0);
    let _enabled = crate::view::conversations_list(
        &host,
        &state,
        &todos,
        ConversationsCapabilities::all(),
        Length::Fill,
        Border::default(),
    );
    assert_eq!(*todos.0.lock().unwrap(), 2);
}
#[test]
fn review_pane_hides_the_generate_button_without_the_capability() {
    let f = Fixture::new(FakeConversation::default(), FakeSummary::default());
    let mut state = state();
    open(&f, &mut state, "a");
    let host = Display::default();
    let _view = crate::view::review_pane(
        &state,
        &host,
        ConversationsCapabilities::default(),
        Length::Fill,
        Border::default(),
    );
    assert!(
        !host
            .messages
            .lock()
            .unwrap()
            .iter()
            .any(|m| matches!(m, Message::SummaryGenerate(..)))
    );
    let _enabled = crate::view::review_pane(
        &state,
        &host,
        ConversationsCapabilities::all(),
        Length::Fill,
        Border::default(),
    );
    assert!(
        host.messages
            .lock()
            .unwrap()
            .iter()
            .any(|m| matches!(m, Message::SummaryGenerate(..)))
    );
}
#[test]
fn approved_search_focus_round_trips_and_builds_the_list_view() {
    let mut state = state();
    state.set_search_focused(true);
    assert!(state.search_focused());
    let host = Display::default();
    let _view = crate::view::conversations_list(
        &host,
        &state,
        &Todos::default(),
        ConversationsCapabilities::all(),
        Length::Fill,
        Border::default(),
    );
}
#[test]
fn degenerate_states_build_without_panicking() {
    let host = Display::default();
    let todos = Todos::default();
    let f = Fixture::new(FakeConversation::default(), FakeSummary::default());
    let mut state = WorkspaceState::default();
    for rows in [None, Some(vec![]), Some(vec![row(&"标题".repeat(10000))])] {
        state.sessions = rows;
        let _list = crate::view::conversations_list(
            &host,
            &state,
            &todos,
            ConversationsCapabilities::all(),
            Length::Fill,
            Border::default(),
        );
        let _detail = crate::view::review_pane(
            &state,
            &host,
            ConversationsCapabilities::all(),
            Length::Fill,
            Border::default(),
        );
    }
    state.sessions = Some(vec![row("a")]);
    open(&f, &mut state, "a");
    for error in [None, Some("failed".into())] {
        state.review.as_mut().unwrap().error = error;
        let _detail = crate::view::review_pane(
            &state,
            &host,
            ConversationsCapabilities::all(),
            Length::Fill,
            Border::default(),
        );
    }
}
#[test]
fn summary_errors_resolve_through_the_panel_owner_in_chinese() {
    let text = crate::i18n::keyed_args(
        "summary_submit_failed",
        "⚠ Failed to submit summary task: {error}",
        &[("error", "boom")],
    );
    assert_eq!(
        crate::i18n::render(&text, "zh-CN"),
        "⚠ 提交总结任务失败: boom"
    );
}
#[test]
fn successful_current_load_wraps_the_nonce_and_keeps_snapshot_and_spec_valid() {
    let f = Fixture::new(
        FakeConversation::default().with_turns("a", vec![turn(0, "A")]),
        FakeSummary::default(),
    );
    let mut state = state();
    state.review_nonce = u64::MAX;
    open(&f, &mut state, "a");
    assert_eq!(state.review.as_ref().unwrap().nonce, 0);
    assert!(snapshot(&state).unwrap().contains("A"));
    let spec =
        crate::snapshot::review_webview_spec(state.review.as_ref(), "zh-CN", "dozer").unwrap();
    assert_eq!(spec.url, "dozer://review-trace/host.html?_r=0&locale=zh-CN");
    assert!(spec.park_offscreen);
    assert!(!spec.visible);
}
#[test]
fn summary_completion_from_a_closed_workspace_is_rejected_by_the_new_owner() {
    let f = Fixture::new(
        FakeConversation::default(),
        FakeSummary::default().with_submission(Err(ServiceError::new(
            ServiceErrorKind::Rejected,
            "old error",
        ))),
    );
    let mut old = state();
    let rx = f.send(
        &mut old,
        Message::SummaryGenerate("a".into(), AgentKind::Claude),
    );
    let result = rx.recv_timeout(TIMEOUT).unwrap();
    drop(old);
    let mut reopened = state();
    f.send(&mut reopened, result);
    assert!(reopened.summary_error("a").is_none());
    assert!(!reopened.summary_generating("a"));
    assert!(reopened.take_outbox().is_empty());
}
