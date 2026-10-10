//! 进程内服务假实现(只在测试里):不起 UDS,返回固定结果并记录调用。
use dozer_model::{
    AgentKind, CategoryInfo, CategoryMoveDirection, MemoryDetail, MemoryInfo, ProjectInfo,
    SummaryBatchInfo, TodoInfo, TodoStoredStatus,
};
use dozer_services::{
    BoxFuture, MemoryService, ProjectService, ServiceError, ServiceErrorKind, Services, TodoService,
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
#[allow(dead_code)] // Task 4 起被使用,届时去掉。
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
#[allow(dead_code)] // Task 4 起被 Project 面板测试使用,届时去掉。
pub(crate) struct FakeMemory {
    state: Arc<Mutex<MemoryState>>,
}

impl Default for FakeMemory {
    fn default() -> Self {
        Self::ok(vec![])
    }
}

#[allow(dead_code)] // Task 4 起被使用,届时去掉。
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
#[allow(dead_code)] // Task 4 起被使用,届时去掉。
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

#[allow(dead_code)] // Task 4 起被使用,届时去掉。
impl FakeProject {
    pub(crate) fn with_rename(self, r: Result<Option<ProjectInfo>, ServiceError>) -> Self {
        self.state.lock().unwrap().rename = r;
        self
    }
    pub(crate) fn with_remove(self, r: Result<(), ServiceError>) -> Self {
        self.state.lock().unwrap().remove = r;
        self
    }
    pub(crate) fn with_delete_transcripts(self, r: Result<u32, ServiceError>) -> Self {
        self.state.lock().unwrap().delete_transcripts = r;
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
