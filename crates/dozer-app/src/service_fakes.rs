//! 进程内服务假实现(只在测试里):不起 UDS,返回固定结果并记录调用。
use dozer_model::{
    AgentKind, BookmarkInfo, BookmarkScope, CategoryInfo, CategoryMoveDirection,
    CodeHealthReportInfo, ContextItemInfo, ConversationSummary, FileEditHistoryInfo, GroupAuthor,
    GroupCancelScope, GroupInfo, GroupMessageInfo, MemoryDetail, MemoryInfo, MutationOutcome,
    ProjectInfo, SessionSummaryPayload, SummaryBatchInfo, TodoInfo, TodoStoredStatus,
};
use dozer_services::{
    BookmarkService, BoxFuture, CodeHealthService, ContextService, ConversationRows,
    ConversationService, EditHistoryService, GroupChatService, MemoryService, PostedMessages,
    PreciseEdit, ProjectService, ServiceError, ServiceErrorKind, Services, TodoService,
};
use std::sync::{Arc, Mutex};

struct State {
    todos: Result<Vec<TodoInfo>, ServiceError>,
    categories: Result<Vec<CategoryInfo>, ServiceError>,
    write_error: Option<ServiceError>,
    calls: Vec<String>,
}

/// `Clone` 的克隆共享同一份状态(测试里一份给面板、一份用来看调用记录)。
#[derive(Clone)]
pub(crate) struct FakeTodo {
    state: Arc<Mutex<State>>,
}

impl Default for FakeTodo {
    fn default() -> Self {
        Self::ok(vec![], vec![])
    }
}

impl FakeTodo {
    pub(crate) fn ok(todos: Vec<TodoInfo>, categories: Vec<CategoryInfo>) -> Self {
        Self::with(Ok(todos), Ok(categories), None)
    }
    pub(crate) fn failing_reads(kind: ServiceErrorKind, message: &str) -> Self {
        let e = ServiceError::new(kind, message);
        Self::with(Err(e.clone()), Err(e), None)
    }
    pub(crate) fn failing_writes(kind: ServiceErrorKind, message: &str) -> Self {
        Self::with(
            Ok(vec![]),
            Ok(vec![]),
            Some(ServiceError::new(kind, message)),
        )
    }
    fn with(
        todos: Result<Vec<TodoInfo>, ServiceError>,
        categories: Result<Vec<CategoryInfo>, ServiceError>,
        write_error: Option<ServiceError>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                todos,
                categories,
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
            todo: Arc::new(self),
            ..base_services()
        }
    }
    fn write(&self, call: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(call);
        let r = match &s.write_error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        };
        Box::pin(async move { r })
    }
}

impl TodoService for FakeTodo {
    fn list_todos(&self, project_id: i64) -> BoxFuture<'_, Result<Vec<TodoInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("list_todos({project_id})"));
        let r = s.todos.clone();
        Box::pin(async move { r })
    }
    fn list_categories(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<CategoryInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("list_categories({project_id})"));
        let r = s.categories.clone();
        Box::pin(async move { r })
    }
    fn add_todo(&self, project_id: i64, text: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("add_todo({project_id},{text})"))
    }
    fn toggle_todo(&self, id: i64, done: bool) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("toggle_todo({id},{done})"))
    }
    fn edit_todo_text(&self, id: i64, text: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("edit_todo_text({id},{text})"))
    }
    fn reorder_todo(
        &self,
        id: i64,
        after_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("reorder_todo({id},{after_id:?})"))
    }
    fn set_todo_plan_date(
        &self,
        id: i64,
        plan_date: Option<String>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("set_todo_plan_date({id},{plan_date:?})"))
    }
    fn set_todo_status(
        &self,
        id: i64,
        status: TodoStoredStatus,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("set_todo_status({id},{status:?})"))
    }
    fn set_todo_category(
        &self,
        id: i64,
        category_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("set_todo_category({id},{category_id:?})"))
    }
    /// 写成功时返回一个固定 id=900 的分类,供"新建后进入重命名"的测试使用;失败时返回预设错误。
    fn add_category(
        &self,
        project_id: i64,
        parent_id: Option<i64>,
        name: String,
    ) -> BoxFuture<'_, Result<CategoryInfo, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls
            .push(format!("add_category({project_id},{parent_id:?},{name})"));
        let r = match &s.write_error {
            Some(e) => Err(e.clone()),
            None => Ok(CategoryInfo {
                id: 900,
                project_id,
                parent_id,
                name,
                rank: 0,
                created_ms: 0,
                auto_poll_enabled: false,
            }),
        };
        Box::pin(async move { r })
    }
    fn rename_category(&self, id: i64, name: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("rename_category({id},{name})"))
    }
    fn delete_category(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("delete_category({id})"))
    }
    fn reparent_category(
        &self,
        id: i64,
        new_parent_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("reparent_category({id},{new_parent_id:?})"))
    }
    fn move_category_sibling(
        &self,
        id: i64,
        direction: CategoryMoveDirection,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("move_category_sibling({id},{direction:?})"))
    }
}

/// 四个能力都是默认成功的假实现;各假实现的 `into_services` 用 `..base_services()` 补齐其余字段。
pub(crate) fn base_services() -> Services {
    Services {
        usage: Arc::new(crate::usage_probe::fake_usage::FakeUsage::ok(vec![])),
        todo: Arc::new(FakeTodo::default()),
        project: Arc::new(FakeProject::default()),
        memory: Arc::new(FakeMemory::default()),
        group_chat: Arc::new(FakeGroupChat::default()),
        conversation: Arc::new(FakeConversation::default()),
        codehealth: Arc::new(FakeCodeHealth::default()),
        context: Arc::new(FakeContext::default()),
        edit_history: Arc::new(FakeEditHistory::default()),
        bookmark: Arc::new(FakeBookmark::default()),
    }
}

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
            project: Arc::new(self),
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

/// 固定 `GroupInfo`(无成员)。
pub(crate) fn group_info(id: i64, topic: &str) -> GroupInfo {
    GroupInfo {
        id,
        project_id: 1,
        topic: topic.into(),
        created_ms: 0,
        members: vec![],
    }
}

/// 固定 `GroupMessageInfo`(人类发言,无状态)。
pub(crate) fn group_message(id: i64, group_id: i64, rev: i64) -> GroupMessageInfo {
    GroupMessageInfo {
        id,
        group_id,
        seq: rev,
        rev,
        author: GroupAuthor::Human,
        text: format!("m{id}"),
        mentions: vec![],
        status: None,
        duration_ms: None,
        created_ms: 0,
        todo_id: None,
    }
}

/// 固定 `TodoInfo`。
pub(crate) fn todo_info(id: i64) -> TodoInfo {
    TodoInfo {
        id,
        project_id: 1,
        text: "t".into(),
        done: false,
        paused: false,
        rank: 0,
        created_ms: 0,
        completed_at_ms: None,
        plan_date: None,
        dispatch_session_id: None,
        dispatch_at_ms: None,
        category_id: None,
        assigned_agent: None,
    }
}

// ---- FakeGroupChat ----

struct GroupChatState {
    groups: Result<Vec<GroupInfo>, ServiceError>,
    messages: Result<(Vec<GroupMessageInfo>, i64), ServiceError>,
    write_error: Option<ServiceError>,
    calls: Vec<String>,
}

/// 克隆共享同一份状态。`Default` = 读写都成功、空。
#[derive(Clone)]
pub(crate) struct FakeGroupChat {
    state: Arc<Mutex<GroupChatState>>,
}

impl Default for FakeGroupChat {
    fn default() -> Self {
        Self::with(Ok(vec![]), Ok((vec![], 0)), None)
    }
}

impl FakeGroupChat {
    fn with(
        groups: Result<Vec<GroupInfo>, ServiceError>,
        messages: Result<(Vec<GroupMessageInfo>, i64), ServiceError>,
        write_error: Option<ServiceError>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(GroupChatState {
                groups,
                messages,
                write_error,
                calls: vec![],
            })),
        }
    }
    pub(crate) fn with_messages(self, messages: Vec<GroupMessageInfo>, latest_rev: i64) -> Self {
        self.state.lock().unwrap().messages = Ok((messages, latest_rev));
        self
    }
    pub(crate) fn failing_reads(kind: ServiceErrorKind, message: &str) -> Self {
        let e = ServiceError::new(kind, message);
        Self::with(Err(e.clone()), Err(e), None)
    }
    pub(crate) fn failing_writes(kind: ServiceErrorKind, message: &str) -> Self {
        Self::with(
            Ok(vec![]),
            Ok((vec![], 0)),
            Some(ServiceError::new(kind, message)),
        )
    }
    pub(crate) fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }
    pub(crate) fn into_services(self) -> Services {
        Services {
            group_chat: Arc::new(self),
            ..base_services()
        }
    }
    /// 记录一次写调用并按预设返回:有 `write_error` 则失败,否则 `ok()`。
    fn write<T: Send + 'static>(
        &self,
        call: String,
        ok: impl FnOnce() -> T,
    ) -> BoxFuture<'_, Result<T, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(call);
        let r = match &s.write_error {
            Some(e) => Err(e.clone()),
            None => Ok(ok()),
        };
        Box::pin(async move { r })
    }
}

impl GroupChatService for FakeGroupChat {
    fn list_groups(&self, project_id: i64) -> BoxFuture<'_, Result<Vec<GroupInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("list_groups({project_id})"));
        let r = s.groups.clone();
        Box::pin(async move { r })
    }
    fn list_group_messages(
        &self,
        group_id: i64,
        after_rev: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<(Vec<GroupMessageInfo>, i64), ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!(
            "list_group_messages({group_id},{after_rev},{limit})"
        ));
        let r = s.messages.clone();
        Box::pin(async move { r })
    }
    fn create_group(
        &self,
        project_id: i64,
        topic: String,
    ) -> BoxFuture<'_, Result<GroupInfo, ServiceError>> {
        let call = format!("create_group({project_id},{topic})");
        self.write(call, move || group_info(900, &topic))
    }
    fn delete_group(&self, group_id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("delete_group({group_id})"), || ())
    }
    fn add_group_member(
        &self,
        group_id: i64,
        agent: AgentKind,
        handle: String,
        _role_prompt: String,
    ) -> BoxFuture<'_, Result<GroupInfo, ServiceError>> {
        self.write(
            format!("add_group_member({group_id},{agent:?},{handle})"),
            || group_info(1, "g"),
        )
    }
    fn update_group_member(
        &self,
        member_id: i64,
        handle: String,
        _role_prompt: String,
    ) -> BoxFuture<'_, Result<GroupInfo, ServiceError>> {
        self.write(format!("update_group_member({member_id},{handle})"), || {
            group_info(1, "g")
        })
    }
    fn remove_group_member(
        &self,
        member_id: i64,
    ) -> BoxFuture<'_, Result<GroupInfo, ServiceError>> {
        self.write(format!("remove_group_member({member_id})"), || {
            group_info(1, "g")
        })
    }
    fn post_group_message(
        &self,
        group_id: i64,
        text: String,
    ) -> BoxFuture<'_, Result<PostedMessages, ServiceError>> {
        self.write(
            format!("post_group_message({group_id},{text})"),
            move || {
                (
                    group_message(901, group_id, 1),
                    vec![group_message(902, group_id, 2)],
                    vec!["unknown".to_string()],
                )
            },
        )
    }
    fn cancel_group(
        &self,
        group_id: i64,
        scope: GroupCancelScope,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("cancel_group({group_id},{scope:?})"), || ())
    }
    fn retry_group_message(
        &self,
        message_id: i64,
    ) -> BoxFuture<'_, Result<GroupMessageInfo, ServiceError>> {
        self.write(format!("retry_group_message({message_id})"), move || {
            group_message(message_id, 1, 1)
        })
    }
    fn push_group_message_to_todo(
        &self,
        message_id: i64,
        text: String,
    ) -> BoxFuture<'_, Result<TodoInfo, ServiceError>> {
        self.write(
            format!("push_group_message_to_todo({message_id},{text})"),
            || todo_info(77),
        )
    }
}

/// 固定的会话行(Claude、无总结)。
pub(crate) fn conversation_row(
    id: &str,
    title: &str,
) -> (ConversationSummary, Option<SessionSummaryPayload>) {
    (
        ConversationSummary {
            conversation_id: id.into(),
            agent: AgentKind::Claude,
            file_path: format!("/t/{id}.jsonl"),
            title: title.into(),
            first_ts: 0,
            last_ts: 0,
            turn_count: 1,
        },
        None,
    )
}

// ---- FakeConversation ----

struct ConversationState {
    rows: Result<ConversationRows, ServiceError>,
    calls: Vec<String>,
}

/// 克隆共享同一份状态。`Default` = 成功且空。
#[derive(Clone)]
pub(crate) struct FakeConversation {
    state: Arc<Mutex<ConversationState>>,
}

impl Default for FakeConversation {
    fn default() -> Self {
        Self::with(Ok(vec![]))
    }
}

impl FakeConversation {
    fn with(rows: Result<ConversationRows, ServiceError>) -> Self {
        Self {
            state: Arc::new(Mutex::new(ConversationState {
                rows,
                calls: vec![],
            })),
        }
    }
    pub(crate) fn with_rows(self, rows: ConversationRows) -> Self {
        self.state.lock().unwrap().rows = Ok(rows);
        self
    }
    pub(crate) fn failing(kind: ServiceErrorKind, message: &str) -> Self {
        Self::with(Err(ServiceError::new(kind, message)))
    }
    pub(crate) fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }
    pub(crate) fn into_services(self) -> Services {
        Services {
            conversation: Arc::new(self),
            ..base_services()
        }
    }
}

impl ConversationService for FakeConversation {
    fn list_conversations_with_summaries(
        &self,
        cwd: String,
        agent: Option<AgentKind>,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<ConversationRows, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!(
            "list_conversations_with_summaries({cwd},{agent:?},{limit},{offset})"
        ));
        let r = s.rows.clone();
        Box::pin(async move { r })
    }
}

// ---- FakeCodeHealth ----

#[derive(Default)]
struct CodeHealthState {
    latest: Option<CodeHealthReportInfo>,
    get_error: Option<ServiceError>,
    list_error: Option<ServiceError>,
    save_error: Option<ServiceError>,
    calls: Vec<String>,
}

/// 克隆共享同一份状态。`Default` = 全部成功(get → `Ok(None)`、list → `Ok(vec![])`、save → `Ok(())`)。
#[derive(Clone, Default)]
pub(crate) struct FakeCodeHealth {
    state: Arc<Mutex<CodeHealthState>>,
}

impl FakeCodeHealth {
    pub(crate) fn with_latest(self, info: CodeHealthReportInfo) -> Self {
        self.state.lock().unwrap().latest = Some(info);
        self
    }
    pub(crate) fn with_get_error(self, kind: ServiceErrorKind, message: &str) -> Self {
        self.state.lock().unwrap().get_error = Some(ServiceError::new(kind, message));
        self
    }
    pub(crate) fn with_list_error(self, kind: ServiceErrorKind, message: &str) -> Self {
        self.state.lock().unwrap().list_error = Some(ServiceError::new(kind, message));
        self
    }
    pub(crate) fn with_save_error(self, kind: ServiceErrorKind, message: &str) -> Self {
        self.state.lock().unwrap().save_error = Some(ServiceError::new(kind, message));
        self
    }
    pub(crate) fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }
    pub(crate) fn into_services(self) -> Services {
        Services {
            codehealth: Arc::new(self),
            ..base_services()
        }
    }
}

impl CodeHealthService for FakeCodeHealth {
    fn get_code_health_report(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Option<CodeHealthReportInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls
            .push(format!("get_code_health_report({project_id})"));
        let r = match &s.get_error {
            Some(e) => Err(e.clone()),
            None => Ok(s.latest.clone()),
        };
        Box::pin(async move { r })
    }
    fn list_code_health_reports(
        &self,
        project_id: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<CodeHealthReportInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls
            .push(format!("list_code_health_reports({project_id},{limit})"));
        let r = match &s.list_error {
            Some(e) => Err(e.clone()),
            None => Ok(vec![]),
        };
        Box::pin(async move { r })
    }
    fn save_code_health_report(
        &self,
        project_id: i64,
        _info: CodeHealthReportInfo,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls
            .push(format!("save_code_health_report({project_id})"));
        let r = match &s.save_error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        };
        Box::pin(async move { r })
    }
}

// ---- FakeContext ----

#[derive(Default)]
struct ContextState {
    items: Vec<ContextItemInfo>,
    list_error: Option<ServiceError>,
    write_error: Option<ServiceError>,
    calls: Vec<String>,
}

/// 克隆共享同一份状态。`Default` = 全部成功、列表为空。
#[derive(Clone, Default)]
pub(crate) struct FakeContext {
    state: Arc<Mutex<ContextState>>,
}

impl FakeContext {
    pub(crate) fn with_items(self, items: Vec<ContextItemInfo>) -> Self {
        self.state.lock().unwrap().items = items;
        self
    }
    pub(crate) fn with_list_error(self, kind: ServiceErrorKind, message: &str) -> Self {
        self.state.lock().unwrap().list_error = Some(ServiceError::new(kind, message));
        self
    }
    /// add 与 remove 共用。
    pub(crate) fn with_write_error(self, kind: ServiceErrorKind, message: &str) -> Self {
        self.state.lock().unwrap().write_error = Some(ServiceError::new(kind, message));
        self
    }
    pub(crate) fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }
    pub(crate) fn into_services(self) -> Services {
        Services {
            context: Arc::new(self),
            ..base_services()
        }
    }
}

impl ContextService for FakeContext {
    fn list_context_items(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<ContextItemInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("list_context_items({project_id})"));
        let r = match &s.list_error {
            Some(e) => Err(e.clone()),
            None => Ok(s.items.clone()),
        };
        Box::pin(async move { r })
    }
    fn add_context_item(
        &self,
        project_id: i64,
        entity_kind: String,
        entity_ref: String,
    ) -> BoxFuture<'_, Result<ContextItemInfo, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!(
            "add_context_item({project_id},{entity_kind},{entity_ref})"
        ));
        let r = match &s.write_error {
            Some(e) => Err(e.clone()),
            None => Ok(ContextItemInfo {
                id: 77,
                project_id,
                entity_kind,
                entity_ref,
                created_ms: 0,
            }),
        };
        Box::pin(async move { r })
    }
    fn remove_context_item(
        &self,
        project_id: i64,
        id: i64,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls
            .push(format!("remove_context_item({project_id},{id})"));
        let r = match &s.write_error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        };
        Box::pin(async move { r })
    }
}

// ---- FakeEditHistory ----

struct EditHistoryState {
    entries: Vec<FileEditHistoryInfo>,
    list_error: Option<ServiceError>,
    outcome: MutationOutcome,
    edit_error: Option<ServiceError>,
    list_calls: Vec<(i64, Option<String>, u32)>,
    edits: Vec<PreciseEdit>,
}

/// 克隆共享同一份状态。`Default` = 全部成功(list → `Ok(vec![])`、apply → `Ok(Applied{全 0})`)。
#[derive(Clone)]
pub(crate) struct FakeEditHistory {
    state: Arc<Mutex<EditHistoryState>>,
}

impl Default for FakeEditHistory {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(EditHistoryState {
                entries: vec![],
                list_error: None,
                outcome: MutationOutcome::Applied {
                    new_start_line: 0,
                    new_start_col: 0,
                    new_end_line: 0,
                    new_end_col: 0,
                    history_id: 0,
                },
                edit_error: None,
                list_calls: vec![],
                edits: vec![],
            })),
        }
    }
}

impl FakeEditHistory {
    pub(crate) fn with_entries(self, entries: Vec<FileEditHistoryInfo>) -> Self {
        self.state.lock().unwrap().entries = entries;
        self
    }
    pub(crate) fn with_list_error(self, kind: ServiceErrorKind, message: &str) -> Self {
        self.state.lock().unwrap().list_error = Some(ServiceError::new(kind, message));
        self
    }
    pub(crate) fn with_edit_error(self, kind: ServiceErrorKind, message: &str) -> Self {
        self.state.lock().unwrap().edit_error = Some(ServiceError::new(kind, message));
        self
    }
    pub(crate) fn list_calls(&self) -> Vec<(i64, Option<String>, u32)> {
        self.state.lock().unwrap().list_calls.clone()
    }
    pub(crate) fn edits(&self) -> Vec<PreciseEdit> {
        self.state.lock().unwrap().edits.clone()
    }
    pub(crate) fn into_services(self) -> Services {
        Services {
            edit_history: Arc::new(self),
            ..base_services()
        }
    }
}

impl EditHistoryService for FakeEditHistory {
    fn list_file_edit_history(
        &self,
        project_id: i64,
        path_filter: Option<String>,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<FileEditHistoryInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.list_calls.push((project_id, path_filter, limit));
        let r = match &s.list_error {
            Some(e) => Err(e.clone()),
            None => Ok(s.entries.clone()),
        };
        Box::pin(async move { r })
    }
    fn apply_precise_edit(
        &self,
        edit: PreciseEdit,
    ) -> BoxFuture<'_, Result<MutationOutcome, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.edits.push(edit);
        let r = match &s.edit_error {
            Some(e) => Err(e.clone()),
            None => Ok(s.outcome.clone()),
        };
        Box::pin(async move { r })
    }
}

// ---- FakeBookmark ----

#[derive(Default)]
struct BookmarkState {
    items: Vec<BookmarkInfo>,
    list_error: Option<ServiceError>,
    write_error: Option<ServiceError>,
    calls: Vec<String>,
}

/// 克隆共享同一份状态。`Default` = 全部成功、列表为空。
#[derive(Clone, Default)]
pub(crate) struct FakeBookmark {
    state: Arc<Mutex<BookmarkState>>,
}

impl FakeBookmark {
    pub(crate) fn with_items(self, items: Vec<BookmarkInfo>) -> Self {
        self.state.lock().unwrap().items = items;
        self
    }
    pub(crate) fn with_list_error(self, kind: ServiceErrorKind, message: &str) -> Self {
        self.state.lock().unwrap().list_error = Some(ServiceError::new(kind, message));
        self
    }
    /// add 与 remove 共用。
    pub(crate) fn with_write_error(self, kind: ServiceErrorKind, message: &str) -> Self {
        self.state.lock().unwrap().write_error = Some(ServiceError::new(kind, message));
        self
    }
    pub(crate) fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }
    pub(crate) fn into_services(self) -> Services {
        Services {
            bookmark: Arc::new(self),
            ..base_services()
        }
    }
}

impl BookmarkService for FakeBookmark {
    fn list_bookmarks(
        &self,
        project_id: Option<i64>,
    ) -> BoxFuture<'_, Result<Vec<BookmarkInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("list_bookmarks({project_id:?})"));
        let r = match &s.list_error {
            Some(e) => Err(e.clone()),
            None => Ok(s.items.clone()),
        };
        Box::pin(async move { r })
    }
    fn add_bookmark(
        &self,
        scope: BookmarkScope,
        project_id: Option<i64>,
        url: String,
        title: String,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!(
            "add_bookmark({scope:?},{project_id:?},{url},{title})"
        ));
        let r = match &s.write_error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        };
        Box::pin(async move { r })
    }
    fn remove_bookmark(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("remove_bookmark({id})"));
        let r = match &s.write_error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        };
        Box::pin(async move { r })
    }
}
