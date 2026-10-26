//! 进程内服务假实现(只在库的测试里):返回固定结果并记录调用。
//!
//! 搬自 Dozer `service_fakes.rs` 中 Project 面板用到的三件:`FakeProject`
//! (同时实现 `ProjectService` 与 `TranscriptService`)、`FakeMemory`、`batch`,
//! 外加一个最小 `base_services()`(其余服务字段用同一个`FakeProject`占位,
//! Project 面板从不调用它们)。不做 feature 开关:只有本库的 `#[cfg(test)]`
//! 用到。

use dozer_model::{
    AgentKind, MemoryDetail, MemoryInfo, ProjectInfo, SessionEvent, SessionInfo, SummaryBatchInfo,
};
use dozer_services::{
    BoxFuture, MemoryService, ProjectService, ServiceError, ServiceErrorKind, Services,
    SessionService, SessionSpec, TranscriptService,
};
use std::sync::{Arc, Mutex};

/// 字段全填的固定 `MemoryDetail`。
pub(crate) fn memory_detail(id: i64, title: &str) -> MemoryDetail {
    MemoryDetail {
        id,
        project_id: 1,
        title: title.into(),
        kind: "project".into(),
        description: String::new(),
        body: String::new(),
        created_ms: 0,
        created_by: "user".into(),
        updated_ms: 0,
        updated_by: "user".into(),
        history: vec![],
    }
}

/// 固定 `SummaryBatchInfo`(其余字段取固定值)。
pub(crate) fn batch(
    total: u32,
    queued: u32,
    running: u32,
    succeeded: u32,
    failed: u32,
    skipped: u32,
) -> SummaryBatchInfo {
    SummaryBatchInfo {
        batch_id: 7,
        cwd: "/p".into(),
        strategy: "missing_or_stale".into(),
        total,
        queued,
        running,
        succeeded,
        failed,
        cancelled: 0,
        skipped,
        created_ts_ms: 0,
        updated_ts_ms: 0,
    }
}

// ---- FakeMemory ----

struct MemoryState {
    list: Result<Vec<MemoryInfo>, ServiceError>,
    get_error: Option<ServiceError>,
    write_error: Option<ServiceError>,
    calls: Vec<String>,
}

/// 克隆共享同一份状态。`Default` = 读写都成功、空列表。
#[derive(Clone)]
pub(crate) struct FakeMemory {
    state: Arc<Mutex<MemoryState>>,
}

impl Default for FakeMemory {
    fn default() -> Self {
        Self::ok(vec![])
    }
}

impl FakeMemory {
    pub(crate) fn ok(list: Vec<MemoryInfo>) -> Self {
        Self::with(Ok(list), None, None)
    }
    pub(crate) fn failing_list(kind: ServiceErrorKind, message: &str) -> Self {
        Self::with(Err(ServiceError::new(kind, message)), None, None)
    }
    pub(crate) fn failing_writes(kind: ServiceErrorKind, message: &str) -> Self {
        Self::with(Ok(vec![]), None, Some(ServiceError::new(kind, message)))
    }
    fn with(
        list: Result<Vec<MemoryInfo>, ServiceError>,
        get_error: Option<ServiceError>,
        write_error: Option<ServiceError>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(MemoryState {
                list,
                get_error,
                write_error,
                calls: vec![],
            })),
        }
    }
    pub(crate) fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }
    pub(crate) fn into_services(self) -> Services {
        Services {
            memory: Arc::new(self),
            ..base_services()
        }
    }
}

impl MemoryService for FakeMemory {
    fn list_memories(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<MemoryInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("list_memories({project_id})"));
        let r = s.list.clone();
        Box::pin(async move { r })
    }
    fn get_memory(
        &self,
        project_id: i64,
        id: i64,
    ) -> BoxFuture<'_, Result<MemoryDetail, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("get_memory({project_id},{id})"));
        let r = match &s.get_error {
            Some(e) => Err(e.clone()),
            None => Ok(memory_detail(id, "t")),
        };
        Box::pin(async move { r })
    }
    fn write_memory(
        &self,
        project_id: i64,
        title: String,
        _kind: String,
        _description: String,
        _body: String,
        _actor: String,
    ) -> BoxFuture<'_, Result<MemoryDetail, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("write_memory({project_id},{title})"));
        let r = match &s.write_error {
            Some(e) => Err(e.clone()),
            None => Ok(memory_detail(900, &title)),
        };
        Box::pin(async move { r })
    }
    fn delete_memory(
        &self,
        project_id: i64,
        id: i64,
        actor: String,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls
            .push(format!("delete_memory({project_id},{id},{actor})"));
        let r = match &s.write_error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        };
        Box::pin(async move { r })
    }
}

// ---- FakeProject ----

struct ProjectState {
    rename: Result<Option<ProjectInfo>, ServiceError>,
    remove: Result<(), ServiceError>,
    delete_transcripts: Result<u32, ServiceError>,
    backfill: Result<u32, ServiceError>,
    submit: Result<(i64, u32), ServiceError>,
    batches: Vec<Result<Option<SummaryBatchInfo>, ServiceError>>,
    batch_calls: usize,
    open: Result<Option<ProjectInfo>, ServiceError>,
    projects: Result<Vec<ProjectInfo>, ServiceError>,
    calls: Vec<String>,
}

/// 克隆共享同一份状态。`Default` = 全部成功(rename → `Ok(None)`、transcripts → `Ok(0)`、
/// submit → `Ok((1, 1))`、`get_summary_batch` → `Ok(None)`)。
#[derive(Clone)]
pub(crate) struct FakeProject {
    state: Arc<Mutex<ProjectState>>,
}

impl Default for FakeProject {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(ProjectState {
                rename: Ok(None),
                remove: Ok(()),
                delete_transcripts: Ok(0),
                backfill: Ok(0),
                submit: Ok((1, 1)),
                batches: vec![],
                batch_calls: 0,
                open: Ok(None),
                projects: Ok(vec![]),
                calls: vec![],
            })),
        }
    }
}

impl FakeProject {
    pub(crate) fn with_remove(self, r: Result<(), ServiceError>) -> Self {
        self.state.lock().unwrap().remove = r;
        self
    }
    pub(crate) fn with_backfill(self, r: Result<u32, ServiceError>) -> Self {
        self.state.lock().unwrap().backfill = r;
        self
    }
    pub(crate) fn with_submit(self, r: Result<(i64, u32), ServiceError>) -> Self {
        self.state.lock().unwrap().submit = r;
        self
    }
    /// 按调用顺序依次返回;用完后重复最后一项;为空时 `Ok(None)`。
    pub(crate) fn with_batches(
        self,
        r: Vec<Result<Option<SummaryBatchInfo>, ServiceError>>,
    ) -> Self {
        self.state.lock().unwrap().batches = r;
        self
    }
    pub(crate) fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }
    pub(crate) fn into_services(self) -> Services {
        Services {
            project: Arc::new(self.clone()),
            transcript: Arc::new(self),
            ..base_services()
        }
    }
}

impl ProjectService for FakeProject {
    fn rename_project(
        &self,
        id: i64,
        name: String,
    ) -> BoxFuture<'_, Result<Option<ProjectInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("rename_project({id},{name})"));
        let r = s.rename.clone();
        Box::pin(async move { r })
    }
    fn remove_project(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("remove_project({id})"));
        let r = s.remove.clone();
        Box::pin(async move { r })
    }
    fn open_project(
        &self,
        path: String,
    ) -> BoxFuture<'_, Result<Option<ProjectInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("open_project({path})"));
        let r = s.open.clone();
        Box::pin(async move { r })
    }
    fn list_projects(&self) -> BoxFuture<'_, Result<Vec<ProjectInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push("list_projects()".to_string());
        let r = s.projects.clone();
        Box::pin(async move { r })
    }
}

impl TranscriptService for FakeProject {
    fn delete_project_transcripts(&self, cwd: String) -> BoxFuture<'_, Result<u32, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("delete_project_transcripts({cwd})"));
        let r = s.delete_transcripts.clone();
        Box::pin(async move { r })
    }
    fn backfill_project_transcripts(
        &self,
        cwd: String,
    ) -> BoxFuture<'_, Result<u32, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("backfill({cwd})"));
        let r = s.backfill.clone();
        Box::pin(async move { r })
    }
    fn submit_summary_batch(
        &self,
        cwd: String,
        _provider: Option<AgentKind>,
        _model: Option<String>,
    ) -> BoxFuture<'_, Result<(i64, u32), ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("submit_summary_batch({cwd})"));
        let r = s.submit.clone();
        Box::pin(async move { r })
    }
    fn get_summary_batch(
        &self,
        batch_id: i64,
    ) -> BoxFuture<'_, Result<Option<SummaryBatchInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("get_summary_batch({batch_id})"));
        let r = if s.batches.is_empty() {
            Ok(None)
        } else {
            let i = s.batch_calls.min(s.batches.len() - 1);
            s.batches[i].clone()
        };
        s.batch_calls += 1;
        Box::pin(async move { r })
    }
}

// ---- 其余服务:Project 面板从不调用,只要类型对得上 ----

fn never<T>() -> BoxFuture<'static, Result<T, ServiceError>> {
    Box::pin(async {
        Err(ServiceError::new(
            ServiceErrorKind::Unavailable,
            "not used by the project panel",
        ))
    })
}

struct NoopUsage;
impl dozer_services::UsageService for NoopUsage {
    fn usage_summary(
        &self,
        _cwd: String,
        _since_ts: Option<u64>,
    ) -> BoxFuture<'_, Result<dozer_services::UsageRows, ServiceError>> {
        never()
    }
}

struct NoopTodo;
impl dozer_services::TodoService for NoopTodo {
    fn list_todos(
        &self,
        _project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<dozer_model::TodoInfo>, ServiceError>> {
        never()
    }
    fn list_categories(
        &self,
        _project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<dozer_model::CategoryInfo>, ServiceError>> {
        never()
    }
    fn add_todo(&self, _project_id: i64, _text: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
    fn toggle_todo(&self, _id: i64, _done: bool) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
    fn edit_todo_text(&self, _id: i64, _text: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
    fn reorder_todo(
        &self,
        _id: i64,
        _after_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
    fn set_todo_plan_date(
        &self,
        _id: i64,
        _plan_date: Option<String>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
    fn set_todo_status(
        &self,
        _id: i64,
        _status: dozer_model::TodoStoredStatus,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
    fn set_todo_category(
        &self,
        _id: i64,
        _category_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
    fn add_category(
        &self,
        _project_id: i64,
        _parent_id: Option<i64>,
        _name: String,
    ) -> BoxFuture<'_, Result<dozer_model::CategoryInfo, ServiceError>> {
        never()
    }
    fn rename_category(&self, _id: i64, _name: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
    fn delete_category(&self, _id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
    fn reparent_category(
        &self,
        _id: i64,
        _new_parent_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
    fn move_category_sibling(
        &self,
        _id: i64,
        _direction: dozer_model::CategoryMoveDirection,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
}

struct NoopGroupChat;
impl dozer_services::GroupChatService for NoopGroupChat {
    fn list_groups(
        &self,
        _project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<dozer_model::GroupInfo>, ServiceError>> {
        never()
    }
    fn list_group_messages(
        &self,
        _group_id: i64,
        _after_rev: i64,
        _limit: u32,
    ) -> BoxFuture<'_, Result<(Vec<dozer_model::GroupMessageInfo>, i64), ServiceError>> {
        never()
    }
    fn create_group(
        &self,
        _project_id: i64,
        _topic: String,
    ) -> BoxFuture<'_, Result<dozer_model::GroupInfo, ServiceError>> {
        never()
    }
    fn delete_group(&self, _group_id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
    fn add_group_member(
        &self,
        _group_id: i64,
        _agent: AgentKind,
        _handle: String,
        _role_prompt: String,
    ) -> BoxFuture<'_, Result<dozer_model::GroupInfo, ServiceError>> {
        never()
    }
    fn update_group_member(
        &self,
        _member_id: i64,
        _handle: String,
        _role_prompt: String,
    ) -> BoxFuture<'_, Result<dozer_model::GroupInfo, ServiceError>> {
        never()
    }
    fn remove_group_member(
        &self,
        _member_id: i64,
    ) -> BoxFuture<'_, Result<dozer_model::GroupInfo, ServiceError>> {
        never()
    }
    fn post_group_message(
        &self,
        _group_id: i64,
        _text: String,
    ) -> BoxFuture<'_, Result<dozer_services::PostedMessages, ServiceError>> {
        never()
    }
    fn cancel_group(
        &self,
        _group_id: i64,
        _scope: dozer_model::GroupCancelScope,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
    fn retry_group_message(
        &self,
        _message_id: i64,
    ) -> BoxFuture<'_, Result<dozer_model::GroupMessageInfo, ServiceError>> {
        never()
    }
    fn push_group_message_to_todo(
        &self,
        _message_id: i64,
        _text: String,
    ) -> BoxFuture<'_, Result<dozer_model::TodoInfo, ServiceError>> {
        never()
    }
}

struct NoopConversation;
impl dozer_services::ConversationService for NoopConversation {
    fn get_conversation_turns(
        &self,
        _conversation_id: String,
        _after_turn_index: i64,
        _limit: u32,
    ) -> BoxFuture<'_, Result<Vec<dozer_model::TurnRecord>, ServiceError>> {
        never()
    }

    fn list_conversations_with_summaries(
        &self,
        _cwd: String,
        _agent: Option<AgentKind>,
        _limit: u32,
        _offset: u32,
    ) -> BoxFuture<'_, Result<dozer_services::ConversationRows, ServiceError>> {
        never()
    }
}

struct NoopSummary;
impl dozer_services::SummaryService for NoopSummary {
    fn submit_summary_job(
        &self,
        _conversation_id: String,
        _force: bool,
    ) -> BoxFuture<'_, Result<i64, ServiceError>> {
        never()
    }
    fn get_summary_job(
        &self,
        _job_id: i64,
    ) -> BoxFuture<'_, Result<Option<dozer_model::SummaryJobInfo>, ServiceError>> {
        never()
    }
}

struct NoopCodeHealth;
impl dozer_services::CodeHealthService for NoopCodeHealth {
    fn get_code_health_report(
        &self,
        _project_id: i64,
    ) -> BoxFuture<'_, Result<Option<dozer_model::CodeHealthReportInfo>, ServiceError>> {
        never()
    }
    fn list_code_health_reports(
        &self,
        _project_id: i64,
        _limit: u32,
    ) -> BoxFuture<'_, Result<Vec<dozer_model::CodeHealthReportInfo>, ServiceError>> {
        never()
    }
    fn save_code_health_report(
        &self,
        _project_id: i64,
        _info: dozer_model::CodeHealthReportInfo,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
}

struct NoopContext;
impl dozer_services::ContextService for NoopContext {
    fn list_context_items(
        &self,
        _project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<dozer_model::ContextItemInfo>, ServiceError>> {
        never()
    }
    fn add_context_item(
        &self,
        _project_id: i64,
        _entity_kind: String,
        _entity_ref: String,
    ) -> BoxFuture<'_, Result<dozer_model::ContextItemInfo, ServiceError>> {
        never()
    }
    fn remove_context_item(
        &self,
        _project_id: i64,
        _id: i64,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
}

struct NoopEditHistory;
impl dozer_services::EditHistoryService for NoopEditHistory {
    fn list_file_edit_history(
        &self,
        _project_id: i64,
        _path_filter: Option<String>,
        _limit: u32,
    ) -> BoxFuture<'_, Result<Vec<dozer_model::FileEditHistoryInfo>, ServiceError>> {
        never()
    }
    fn apply_precise_edit(
        &self,
        _edit: dozer_services::PreciseEdit,
    ) -> BoxFuture<'_, Result<dozer_model::MutationOutcome, ServiceError>> {
        never()
    }
}

struct NoopBookmark;
impl dozer_services::BookmarkService for NoopBookmark {
    fn list_bookmarks(
        &self,
        _project_id: Option<i64>,
    ) -> BoxFuture<'_, Result<Vec<dozer_model::BookmarkInfo>, ServiceError>> {
        never()
    }
    fn add_bookmark(
        &self,
        _scope: dozer_model::BookmarkScope,
        _project_id: Option<i64>,
        _url: String,
        _title: String,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
    fn remove_bookmark(&self, _id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
}

struct NoopLocale;
impl dozer_services::LocaleService for NoopLocale {
    fn get_locale(&self) -> BoxFuture<'_, Result<dozer_model::LocaleSnapshot, ServiceError>> {
        never()
    }
    fn set_locale(
        &self,
        _preference: dozer_model::LocalePreference,
        _expected_revision: u64,
        _system_candidates: Vec<String>,
    ) -> BoxFuture<'_, Result<dozer_model::LocaleWriteOutcome, ServiceError>> {
        never()
    }
}

struct NoopDaemon;
impl dozer_services::DaemonService for NoopDaemon {
    fn list_sessions(&self) -> BoxFuture<'_, Result<Vec<dozer_model::SessionInfo>, ServiceError>> {
        never()
    }
    fn shutdown_daemon(&self) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
    fn ensure_running(&self) -> BoxFuture<'_, Result<(), ServiceError>> {
        never()
    }
}

/// 给面板钩子单测用的离线夹具:`emit` 收进 channel。形状与 Dozer
/// `panel_host::testing` 相同(库不知道产品内核)。
pub(crate) mod testing {
    use super::*;
    use dozer_panel_host::{ActivationCtx, PanelIo};
    use std::sync::{Mutex, mpsc};
    use std::time::Duration;

    pub(crate) const TIMEOUT: Duration = Duration::from_secs(5);

    pub(crate) fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap()
    }

    pub(crate) fn offline_ctx<M: Send + 'static>(
        rt: &tokio::runtime::Runtime,
        project_id: i64,
        project_path: Option<std::path::PathBuf>,
    ) -> (ActivationCtx<M>, mpsc::Receiver<M>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let io = PanelIo::new(base_services(), rt.handle().clone(), move |m| {
            let _ = tx.lock().unwrap().send(m);
        });
        (
            ActivationCtx {
                project_id,
                project_path,
                io,
            },
            rx,
        )
    }
}

/// 各假实现的 `into_services` 用 `..base_services()` 补齐其余字段。
/// Project 面板只碰 `project` / `transcript` / `memory`;其余能力是永不调用的桩。
pub(crate) fn base_services() -> Services {
    Services {
        usage: Arc::new(NoopUsage),
        todo: Arc::new(NoopTodo),
        project: Arc::new(FakeProject::default()),
        transcript: Arc::new(FakeProject::default()),
        memory: Arc::new(FakeMemory::default()),
        group_chat: Arc::new(NoopGroupChat),
        conversation: Arc::new(NoopConversation),
        summary: Arc::new(NoopSummary),
        codehealth: Arc::new(NoopCodeHealth),
        context: Arc::new(NoopContext),
        edit_history: Arc::new(NoopEditHistory),
        bookmark: Arc::new(NoopBookmark),
        locale: Arc::new(NoopLocale),
        daemon: Arc::new(NoopDaemon),
        session: Arc::new(NoopSession),
    }
}

/// Project 面板不碰会话;最小桩。
struct NoopSession;

impl SessionService for NoopSession {
    fn list_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionInfo>, ServiceError>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn create(&self, _spec: SessionSpec) -> BoxFuture<'_, Result<SessionInfo, ServiceError>> {
        Box::pin(async { Err(ServiceError::new(ServiceErrorKind::Unavailable, "stub")) })
    }
    fn attach(
        &self,
        _id: String,
        _from_offset: u64,
    ) -> BoxFuture<
        '_,
        Result<
            (
                Vec<u8>,
                u64,
                tokio::sync::mpsc::UnboundedReceiver<SessionEvent>,
            ),
            ServiceError,
        >,
    > {
        Box::pin(async {
            let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
            Ok((Vec::new(), 0, rx))
        })
    }
    fn write(&self, _id: String, _data: Vec<u8>) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async { Ok(()) })
    }
    fn resize(
        &self,
        _id: String,
        _cols: u16,
        _rows: u16,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async { Ok(()) })
    }
    fn kill(&self, _id: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async { Ok(()) })
    }
    fn close_with_summary(&self, _id: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async { Ok(()) })
    }
}
