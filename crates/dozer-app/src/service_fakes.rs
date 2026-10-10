//! 进程内服务假实现(只在测试里):不起 UDS,返回固定结果并记录调用。
use dozer_model::{CategoryInfo, CategoryMoveDirection, TodoInfo, TodoStoredStatus};
use dozer_services::{BoxFuture, ServiceError, ServiceErrorKind, Services, TodoService};
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
            usage: Arc::new(crate::usage_probe::fake_usage::FakeUsage::ok(vec![])),
            todo: Arc::new(self),
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
