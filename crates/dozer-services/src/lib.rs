//! 面板服务 trait 接缝(X0,spec 2026-10-09)。面板只依赖这里的小 trait 和 `dozer-model`,
//! 不知道对面是 dozerd 还是内存假实现。生产实现在 `dozer-client`(为 `Client` 适配)。

use dozer_model::{
    AgentKind, BookmarkInfo, BookmarkScope, CategoryInfo, CategoryMoveDirection,
    CodeHealthReportInfo, ContextItemInfo, ConversationSummary, FileEditHistoryInfo,
    GroupCancelScope, GroupInfo, GroupMessageInfo, LocalePreference, LocaleSnapshot,
    LocaleWriteOutcome, MemoryDetail, MemoryInfo, MutationOutcome, ProjectInfo, SessionInfo,
    SessionSummaryPayload, SummaryBatchInfo, TodoInfo, TodoStoredStatus, UsagePayload,
};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// trait 方法返回的装箱 future(`async fn in trait` 不能做 trait object)。`Send` 以便在
/// `tokio::spawn`/`Handle::spawn` 里使用。
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceErrorKind {
    /// 连不上或连接中途断开。
    Unavailable,
    /// 服务端明确拒绝(dozerd 回了 `Reply::Error`)。
    Rejected,
    /// 意外应答或解码失败。
    Protocol,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceError {
    pub kind: ServiceErrorKind,
    pub message: String,
}

impl ServiceError {
    pub fn new(kind: ServiceErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ServiceError {}

pub type UsageRows = Vec<(ConversationSummary, UsagePayload)>;

/// Usage 面板取数。参数按值传,future 因此不借用调用方的局部变量。
pub trait UsageService: Send + Sync {
    fn usage_summary(
        &self,
        cwd: String,
        since_ts: Option<u64>,
    ) -> BoxFuture<'_, Result<UsageRows, ServiceError>>;
}

/// Todo 面板的读写。参数按值传(future 不借用调用方局部变量,能进 `handle.spawn`)。
/// 写操作成功只返回 `()`——面板靠随后重新拉列表拿权威状态;唯一例外 `add_category`,
/// 面板要用新分类的 id 立刻进入重命名。
pub trait TodoService: Send + Sync {
    fn list_todos(&self, project_id: i64) -> BoxFuture<'_, Result<Vec<TodoInfo>, ServiceError>>;
    fn list_categories(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<CategoryInfo>, ServiceError>>;
    fn add_todo(&self, project_id: i64, text: String) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn toggle_todo(&self, id: i64, done: bool) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn edit_todo_text(&self, id: i64, text: String) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn reorder_todo(
        &self,
        id: i64,
        after_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn set_todo_plan_date(
        &self,
        id: i64,
        plan_date: Option<String>,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn set_todo_status(
        &self,
        id: i64,
        status: TodoStoredStatus,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn set_todo_category(
        &self,
        id: i64,
        category_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn add_category(
        &self,
        project_id: i64,
        parent_id: Option<i64>,
        name: String,
    ) -> BoxFuture<'_, Result<CategoryInfo, ServiceError>>;
    fn rename_category(&self, id: i64, name: String) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn delete_category(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn reparent_category(
        &self,
        id: i64,
        new_parent_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn move_category_sibling(
        &self,
        id: i64,
        direction: CategoryMoveDirection,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
}

/// 共享记忆 CRUD(Project 面板)。参数按值传。
pub trait MemoryService: Send + Sync {
    fn list_memories(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<MemoryInfo>, ServiceError>>;
    fn get_memory(
        &self,
        project_id: i64,
        id: i64,
    ) -> BoxFuture<'_, Result<MemoryDetail, ServiceError>>;
    fn write_memory(
        &self,
        project_id: i64,
        title: String,
        kind: String,
        description: String,
        body: String,
        actor: String,
    ) -> BoxFuture<'_, Result<MemoryDetail, ServiceError>>;
    fn delete_memory(
        &self,
        project_id: i64,
        id: i64,
        actor: String,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
}

/// 项目维护(Project 面板):改名、取消登记、转录补录/删除、总结批次。
/// 返回值保持 `Client` 原样——面板用到补录条数、批次 id/进度、改名结果。
pub trait ProjectService: Send + Sync {
    fn rename_project(
        &self,
        id: i64,
        name: String,
    ) -> BoxFuture<'_, Result<Option<ProjectInfo>, ServiceError>>;
    fn remove_project(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn delete_project_transcripts(&self, cwd: String) -> BoxFuture<'_, Result<u32, ServiceError>>;
    fn backfill_project_transcripts(&self, cwd: String)
    -> BoxFuture<'_, Result<u32, ServiceError>>;
    fn submit_summary_batch(
        &self,
        cwd: String,
        provider: Option<AgentKind>,
        model: Option<String>,
    ) -> BoxFuture<'_, Result<(i64, u32), ServiceError>>;
    fn get_summary_batch(
        &self,
        batch_id: i64,
    ) -> BoxFuture<'_, Result<Option<SummaryBatchInfo>, ServiceError>>;
    /// 登记(或取回)一个项目;路径不存在等情形服务端返回 `Ok(None)` 或 `Err`,面板按原样展示。
    fn open_project(
        &self,
        path: String,
    ) -> BoxFuture<'_, Result<Option<ProjectInfo>, ServiceError>>;
    /// 项目列表(最近优先),供创建后刷新最近项目。
    fn list_projects(&self) -> BoxFuture<'_, Result<Vec<ProjectInfo>, ServiceError>>;
}

/// `post_group_message` 的结果:(人类消息, 排队中的 agent 占位消息, 未识别的 @ 句柄)。
pub type PostedMessages = (GroupMessageInfo, Vec<GroupMessageInfo>, Vec<String>);

/// 群聊面板的读写。参数按值传。返回值保持 `Client` 原样(面板确实使用)。
pub trait GroupChatService: Send + Sync {
    fn list_groups(&self, project_id: i64) -> BoxFuture<'_, Result<Vec<GroupInfo>, ServiceError>>;
    fn list_group_messages(
        &self,
        group_id: i64,
        after_rev: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<(Vec<GroupMessageInfo>, i64), ServiceError>>;
    fn create_group(
        &self,
        project_id: i64,
        topic: String,
    ) -> BoxFuture<'_, Result<GroupInfo, ServiceError>>;
    fn delete_group(&self, group_id: i64) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn add_group_member(
        &self,
        group_id: i64,
        agent: AgentKind,
        handle: String,
        role_prompt: String,
    ) -> BoxFuture<'_, Result<GroupInfo, ServiceError>>;
    fn update_group_member(
        &self,
        member_id: i64,
        handle: String,
        role_prompt: String,
    ) -> BoxFuture<'_, Result<GroupInfo, ServiceError>>;
    fn remove_group_member(&self, member_id: i64)
    -> BoxFuture<'_, Result<GroupInfo, ServiceError>>;
    fn post_group_message(
        &self,
        group_id: i64,
        text: String,
    ) -> BoxFuture<'_, Result<PostedMessages, ServiceError>>;
    fn cancel_group(
        &self,
        group_id: i64,
        scope: GroupCancelScope,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn retry_group_message(
        &self,
        message_id: i64,
    ) -> BoxFuture<'_, Result<GroupMessageInfo, ServiceError>>;
    fn push_group_message_to_todo(
        &self,
        message_id: i64,
        text: String,
    ) -> BoxFuture<'_, Result<TodoInfo, ServiceError>>;
}

/// `list_conversations_with_summaries` 的结果:每行一个会话及其可选的总结。
pub type ConversationRows = Vec<(ConversationSummary, Option<SessionSummaryPayload>)>;

/// 会话列表面板的读取。参数按值传。目前只有这一个方法;host 侧的会话相关调用随各自迁移再加入。
pub trait ConversationService: Send + Sync {
    fn list_conversations_with_summaries(
        &self,
        cwd: String,
        agent: Option<AgentKind>,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<ConversationRows, ServiceError>>;
}

/// 代码健康度面板的落盘读写。参数按值传。
pub trait CodeHealthService: Send + Sync {
    fn get_code_health_report(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Option<CodeHealthReportInfo>, ServiceError>>;
    fn list_code_health_reports(
        &self,
        project_id: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<CodeHealthReportInfo>, ServiceError>>;
    fn save_code_health_report(
        &self,
        project_id: i64,
        info: CodeHealthReportInfo,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
}

/// Agent 上下文条的读写。参数按值传。
pub trait ContextService: Send + Sync {
    fn list_context_items(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<ContextItemInfo>, ServiceError>>;
    fn add_context_item(
        &self,
        project_id: i64,
        entity_kind: String,
        entity_ref: String,
    ) -> BoxFuture<'_, Result<ContextItemInfo, ServiceError>>;
    fn remove_context_item(
        &self,
        project_id: i64,
        id: i64,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
}

/// `apply_precise_edit` 的调用参数。原 inherent 方法有 11 个位置参数(连续 4 个 `u32` 坐标、多个 `String`),
/// 顺序传错编译器发现不了,所以在接缝上用具名字段结构体(仓库约定:参数 ≥7 且同类型相邻)。
/// 这是调用参数,不是线上形状,所以不放 `dozer-model`。
#[derive(Debug, Clone, PartialEq)]
pub struct PreciseEdit {
    pub project_id: i64,
    pub path: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub expected_text: String,
    pub new_text: String,
    pub summary: String,
    pub actor: String,
    pub session_id: String,
}

/// 修改历史弹窗的读写。参数按值传。
pub trait EditHistoryService: Send + Sync {
    fn list_file_edit_history(
        &self,
        project_id: i64,
        path_filter: Option<String>,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<FileEditHistoryInfo>, ServiceError>>;
    fn apply_precise_edit(
        &self,
        edit: PreciseEdit,
    ) -> BoxFuture<'_, Result<MutationOutcome, ServiceError>>;
}

/// 浏览器收藏夹的读写。参数按值传。
pub trait BookmarkService: Send + Sync {
    fn list_bookmarks(
        &self,
        project_id: Option<i64>,
    ) -> BoxFuture<'_, Result<Vec<BookmarkInfo>, ServiceError>>;
    fn add_bookmark(
        &self,
        scope: BookmarkScope,
        project_id: Option<i64>,
        url: String,
        title: String,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn remove_bookmark(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>>;
}

/// 界面语言设置的读写(spec 2026-10-12-dozer-i18n-design §4.2)。daemon 为唯一
/// 写入者;`set_locale` 用期望 revision 做 CAS,冲突返回最新快照。`system_candidates`
/// 仅在 System 偏好下用于解析,无权更改显式偏好。参数按值传,future 不借用局部变量。
pub trait LocaleService: Send + Sync {
    fn get_locale(&self) -> BoxFuture<'_, Result<LocaleSnapshot, ServiceError>>;
    fn set_locale(
        &self,
        preference: LocalePreference,
        expected_revision: u64,
        system_candidates: Vec<String>,
    ) -> BoxFuture<'_, Result<LocaleWriteOutcome, ServiceError>>;
}

/// dozerd 自身的生命周期(设置 → 高级)。无参数,future 不借用调用方局部变量。
/// `ensure_running`:daemon 已响应则直接成功,否则拉起并重试;拉起进程属宿主职责,
/// 因此生产实现在 `dozer-app`,不在 `dozer-client`。
pub trait DaemonService: Send + Sync {
    fn list_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionInfo>, ServiceError>>;
    fn shutdown_daemon(&self) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn ensure_running(&self) -> BoxFuture<'_, Result<(), ServiceError>>;
}

/// 各能力 trait 的 `Arc<dyn>` 束;host 构造,经 `PanelIo` 交给面板。其余能力 trait 随各面板迁移逐个加入。
#[derive(Clone)]
pub struct Services {
    pub usage: Arc<dyn UsageService>,
    pub todo: Arc<dyn TodoService>,
    pub project: Arc<dyn ProjectService>,
    pub memory: Arc<dyn MemoryService>,
    pub group_chat: Arc<dyn GroupChatService>,
    pub conversation: Arc<dyn ConversationService>,
    pub codehealth: Arc<dyn CodeHealthService>,
    pub context: Arc<dyn ContextService>,
    pub edit_history: Arc<dyn EditHistoryService>,
    pub bookmark: Arc<dyn BookmarkService>,
    pub locale: Arc<dyn LocaleService>,
    pub daemon: Arc<dyn DaemonService>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct Mem(Result<UsageRows, ServiceError>);
    impl UsageService for Mem {
        fn usage_summary(
            &self,
            _cwd: String,
            _since: Option<u64>,
        ) -> BoxFuture<'_, Result<UsageRows, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
    }

    #[test]
    fn display_is_the_message_only() {
        let e = ServiceError::new(ServiceErrorKind::Rejected, "daemon 错误: boom");
        assert_eq!(e.to_string(), "daemon 错误: boom");
        assert_eq!(e.kind, ServiceErrorKind::Rejected);
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn UsageService>` 的 async 块必须满足。
    #[test]
    fn dyn_service_future_runs_inside_tokio_spawn() {
        let svc: Arc<dyn UsageService> = Arc::new(Mem(Ok(vec![])));
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let out = rt.block_on(async move {
            tokio::spawn(async move { svc.usage_summary("/p".into(), None).await })
                .await
                .unwrap()
        });
        assert_eq!(out, Ok(vec![]));
    }

    #[test]
    fn failure_kinds_are_distinguishable_from_empty_success() {
        let ok: Result<UsageRows, ServiceError> = Ok(vec![]);
        let un: Result<UsageRows, ServiceError> =
            Err(ServiceError::new(ServiceErrorKind::Unavailable, "x"));
        assert!(ok.is_ok() && un.is_err());
        assert_ne!(ServiceErrorKind::Unavailable, ServiceErrorKind::Protocol);
    }

    #[test]
    fn services_is_cloneable_and_shares_the_service() {
        let s = Services {
            usage: Arc::new(Mem(Ok(vec![]))),
            todo: Arc::new(MemTodo(Ok(vec![]))),
            project: Arc::new(MemProject),
            memory: Arc::new(MemMemory(Ok(vec![]))),
            group_chat: Arc::new(MemGroupChat(Ok(vec![]))),
            conversation: Arc::new(MemConversation(Ok(vec![]))),
            codehealth: Arc::new(MemCodeHealth(Ok(None))),
            context: Arc::new(MemContext(Ok(vec![]))),
            edit_history: Arc::new(MemEditHistory(Ok(vec![]))),
            bookmark: Arc::new(MemBookmark(Ok(vec![]))),
            locale: Arc::new(MemLocale(Ok(LocaleSnapshot::default_snapshot()))),
            daemon: Arc::new(MemDaemon {
                list: Ok(Vec::new()),
                stop: Ok(()),
                ensure: Ok(()),
            }),
        };
        let t = s.clone();
        assert!(Arc::ptr_eq(&s.usage, &t.usage));
        assert!(Arc::ptr_eq(&s.todo, &t.todo));
        assert!(Arc::ptr_eq(&s.project, &t.project));
        assert!(Arc::ptr_eq(&s.memory, &t.memory));
        assert!(Arc::ptr_eq(&s.group_chat, &t.group_chat));
        assert!(Arc::ptr_eq(&s.conversation, &t.conversation));
        assert!(Arc::ptr_eq(&s.codehealth, &t.codehealth));
        assert!(Arc::ptr_eq(&s.context, &t.context));
        assert!(Arc::ptr_eq(&s.edit_history, &t.edit_history));
        assert!(Arc::ptr_eq(&s.bookmark, &t.bookmark));
        assert!(Arc::ptr_eq(&s.locale, &t.locale));
        assert!(Arc::ptr_eq(&s.daemon, &t.daemon));
    }

    // ---- TodoService 契约 ----

    use dozer_model::{CategoryInfo, CategoryMoveDirection, TodoInfo, TodoStoredStatus};

    fn unit() -> BoxFuture<'static, Result<(), ServiceError>> {
        Box::pin(async { Ok(()) })
    }

    /// 内存实现:读返回预设结果,写一律成功。
    struct MemTodo(Result<Vec<TodoInfo>, ServiceError>);

    impl TodoService for MemTodo {
        fn list_todos(&self, _p: i64) -> BoxFuture<'_, Result<Vec<TodoInfo>, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
        fn list_categories(
            &self,
            _p: i64,
        ) -> BoxFuture<'_, Result<Vec<CategoryInfo>, ServiceError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn add_todo(&self, _p: i64, _t: String) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
        fn toggle_todo(&self, _i: i64, _d: bool) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
        fn edit_todo_text(&self, _i: i64, _t: String) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
        fn reorder_todo(
            &self,
            _i: i64,
            _a: Option<i64>,
        ) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
        fn set_todo_plan_date(
            &self,
            _i: i64,
            _d: Option<String>,
        ) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
        fn set_todo_status(
            &self,
            _i: i64,
            _s: TodoStoredStatus,
        ) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
        fn set_todo_category(
            &self,
            _i: i64,
            _c: Option<i64>,
        ) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
        fn add_category(
            &self,
            _p: i64,
            _par: Option<i64>,
            _n: String,
        ) -> BoxFuture<'_, Result<CategoryInfo, ServiceError>> {
            Box::pin(async { Err(ServiceError::new(ServiceErrorKind::Protocol, "mem")) })
        }
        fn rename_category(&self, _i: i64, _n: String) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
        fn delete_category(&self, _i: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
        fn reparent_category(
            &self,
            _i: i64,
            _p: Option<i64>,
        ) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
        fn move_category_sibling(
            &self,
            _i: i64,
            _d: CategoryMoveDirection,
        ) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn TodoService>` 的 async 块必须满足。
    #[test]
    fn dyn_todo_service_futures_run_inside_tokio_spawn() {
        let svc: Arc<dyn TodoService> = Arc::new(MemTodo(Ok(vec![])));
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (list, write) = rt.block_on(async move {
            tokio::spawn(async move {
                let list = svc.list_todos(7).await;
                let write = svc.add_todo(7, "x".into()).await;
                (list, write)
            })
            .await
            .unwrap()
        });
        assert_eq!(list, Ok(vec![]));
        assert_eq!(write, Ok(()));
    }

    #[test]
    fn todo_read_failure_is_distinguishable_from_an_empty_list() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let empty = MemTodo(Ok(vec![]));
        let down = MemTodo(Err(ServiceError::new(ServiceErrorKind::Unavailable, "x")));
        assert!(rt.block_on(empty.list_todos(1)).is_ok());
        assert_eq!(
            rt.block_on(down.list_todos(1)).unwrap_err().kind,
            ServiceErrorKind::Unavailable
        );
    }

    // ---- MemoryService / ProjectService 契约 ----

    use dozer_model::{AgentKind, MemoryDetail, MemoryInfo, ProjectInfo, SummaryBatchInfo};

    struct MemMemory(Result<Vec<MemoryInfo>, ServiceError>);

    impl MemoryService for MemMemory {
        fn list_memories(&self, _p: i64) -> BoxFuture<'_, Result<Vec<MemoryInfo>, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
        fn get_memory(
            &self,
            _p: i64,
            _i: i64,
        ) -> BoxFuture<'_, Result<MemoryDetail, ServiceError>> {
            Box::pin(async { Err(ServiceError::new(ServiceErrorKind::Protocol, "mem")) })
        }
        fn write_memory(
            &self,
            _p: i64,
            _t: String,
            _k: String,
            _d: String,
            _b: String,
            _a: String,
        ) -> BoxFuture<'_, Result<MemoryDetail, ServiceError>> {
            Box::pin(async { Err(ServiceError::new(ServiceErrorKind::Protocol, "mem")) })
        }
        fn delete_memory(
            &self,
            _p: i64,
            _i: i64,
            _a: String,
        ) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
    }

    struct MemProject;

    impl ProjectService for MemProject {
        fn rename_project(
            &self,
            _i: i64,
            _n: String,
        ) -> BoxFuture<'_, Result<Option<ProjectInfo>, ServiceError>> {
            Box::pin(async { Ok(None) })
        }
        fn remove_project(&self, _i: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
        fn delete_project_transcripts(
            &self,
            _c: String,
        ) -> BoxFuture<'_, Result<u32, ServiceError>> {
            Box::pin(async { Ok(3) })
        }
        fn backfill_project_transcripts(
            &self,
            _c: String,
        ) -> BoxFuture<'_, Result<u32, ServiceError>> {
            Box::pin(async { Ok(5) })
        }
        fn submit_summary_batch(
            &self,
            _c: String,
            _p: Option<AgentKind>,
            _m: Option<String>,
        ) -> BoxFuture<'_, Result<(i64, u32), ServiceError>> {
            Box::pin(async { Ok((7, 2)) })
        }
        fn get_summary_batch(
            &self,
            _b: i64,
        ) -> BoxFuture<'_, Result<Option<SummaryBatchInfo>, ServiceError>> {
            Box::pin(async { Ok(None) })
        }

        fn open_project(
            &self,
            _p: String,
        ) -> BoxFuture<'_, Result<Option<ProjectInfo>, ServiceError>> {
            Box::pin(async { Ok(None) })
        }
        fn list_projects(&self) -> BoxFuture<'_, Result<Vec<ProjectInfo>, ServiceError>> {
            Box::pin(async { Ok(vec![]) })
        }
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn …>` 的 async 块必须满足。
    #[test]
    fn dyn_project_and_memory_futures_run_inside_tokio_spawn() {
        let memory: Arc<dyn MemoryService> = Arc::new(MemMemory(Ok(vec![])));
        let project: Arc<dyn ProjectService> = Arc::new(MemProject);
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (list, backfilled, batch) = rt.block_on(async move {
            tokio::spawn(async move {
                let list = memory.list_memories(7).await;
                let backfilled = project.backfill_project_transcripts("/p".into()).await;
                let batch = project.submit_summary_batch("/p".into(), None, None).await;
                (list, backfilled, batch)
            })
            .await
            .unwrap()
        });
        assert_eq!(list, Ok(vec![]));
        assert_eq!(backfilled, Ok(5));
        assert_eq!(batch, Ok((7, 2)));
    }

    #[test]
    fn memory_read_failure_is_distinguishable_from_an_empty_list() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let empty = MemMemory(Ok(vec![]));
        let down = MemMemory(Err(ServiceError::new(ServiceErrorKind::Unavailable, "x")));
        assert!(rt.block_on(empty.list_memories(1)).is_ok());
        assert_eq!(
            rt.block_on(down.list_memories(1)).unwrap_err().kind,
            ServiceErrorKind::Unavailable
        );
    }

    // ---- GroupChatService 契约 ----

    use dozer_model::{GroupCancelScope, GroupInfo, GroupMessageInfo};

    struct MemGroupChat(Result<Vec<GroupInfo>, ServiceError>);

    fn proto(m: &'static str) -> ServiceError {
        ServiceError::new(ServiceErrorKind::Protocol, m)
    }

    impl GroupChatService for MemGroupChat {
        fn list_groups(&self, _p: i64) -> BoxFuture<'_, Result<Vec<GroupInfo>, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
        fn list_group_messages(
            &self,
            _g: i64,
            _a: i64,
            _l: u32,
        ) -> BoxFuture<'_, Result<(Vec<GroupMessageInfo>, i64), ServiceError>> {
            Box::pin(async { Ok((vec![], 0)) })
        }
        fn create_group(
            &self,
            _p: i64,
            _t: String,
        ) -> BoxFuture<'_, Result<GroupInfo, ServiceError>> {
            Box::pin(async { Err(proto("mem")) })
        }
        fn delete_group(&self, _g: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
        fn add_group_member(
            &self,
            _g: i64,
            _a: AgentKind,
            _h: String,
            _r: String,
        ) -> BoxFuture<'_, Result<GroupInfo, ServiceError>> {
            Box::pin(async { Err(proto("mem")) })
        }
        fn update_group_member(
            &self,
            _m: i64,
            _h: String,
            _r: String,
        ) -> BoxFuture<'_, Result<GroupInfo, ServiceError>> {
            Box::pin(async { Err(proto("mem")) })
        }
        fn remove_group_member(&self, _m: i64) -> BoxFuture<'_, Result<GroupInfo, ServiceError>> {
            Box::pin(async { Err(proto("mem")) })
        }
        fn post_group_message(
            &self,
            _g: i64,
            _t: String,
        ) -> BoxFuture<
            '_,
            Result<(GroupMessageInfo, Vec<GroupMessageInfo>, Vec<String>), ServiceError>,
        > {
            Box::pin(async { Err(proto("mem")) })
        }
        fn cancel_group(
            &self,
            _g: i64,
            _s: GroupCancelScope,
        ) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
        fn retry_group_message(
            &self,
            _m: i64,
        ) -> BoxFuture<'_, Result<GroupMessageInfo, ServiceError>> {
            Box::pin(async { Err(proto("mem")) })
        }
        fn push_group_message_to_todo(
            &self,
            _m: i64,
            _t: String,
        ) -> BoxFuture<'_, Result<TodoInfo, ServiceError>> {
            Box::pin(async { Err(proto("mem")) })
        }
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn GroupChatService>` 的 async 块必须满足。
    #[test]
    fn dyn_group_chat_futures_run_inside_tokio_spawn() {
        let svc: Arc<dyn GroupChatService> = Arc::new(MemGroupChat(Ok(vec![])));
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (groups, cancel) = rt.block_on(async move {
            tokio::spawn(async move {
                let groups = svc.list_groups(7).await;
                let cancel = svc.cancel_group(5, GroupCancelScope::Turn).await;
                (groups, cancel)
            })
            .await
            .unwrap()
        });
        assert_eq!(groups, Ok(vec![]));
        assert_eq!(cancel, Ok(()));
    }

    #[test]
    fn group_read_failure_is_distinguishable_from_an_empty_list() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let empty = MemGroupChat(Ok(vec![]));
        let down = MemGroupChat(Err(ServiceError::new(ServiceErrorKind::Unavailable, "x")));
        assert!(rt.block_on(empty.list_groups(1)).is_ok());
        assert_eq!(
            rt.block_on(down.list_groups(1)).unwrap_err().kind,
            ServiceErrorKind::Unavailable
        );
    }

    // ---- ConversationService 契约 ----

    struct MemConversation(Result<ConversationRows, ServiceError>);

    impl ConversationService for MemConversation {
        fn list_conversations_with_summaries(
            &self,
            _cwd: String,
            _agent: Option<AgentKind>,
            _limit: u32,
            _offset: u32,
        ) -> BoxFuture<'_, Result<ConversationRows, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn ConversationService>` 的 async 块必须满足。
    #[test]
    fn dyn_conversation_future_runs_inside_tokio_spawn() {
        let svc: Arc<dyn ConversationService> = Arc::new(MemConversation(Ok(vec![])));
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let out = rt.block_on(async move {
            tokio::spawn(async move {
                svc.list_conversations_with_summaries("/p".into(), None, 500, 0)
                    .await
            })
            .await
            .unwrap()
        });
        assert_eq!(out, Ok(vec![]));
    }

    #[test]
    fn conversation_read_failure_is_distinguishable_from_an_empty_list() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let empty = MemConversation(Ok(vec![]));
        let down = MemConversation(Err(ServiceError::new(ServiceErrorKind::Unavailable, "x")));
        assert!(
            rt.block_on(empty.list_conversations_with_summaries("/p".into(), None, 1, 0))
                .is_ok()
        );
        assert_eq!(
            rt.block_on(down.list_conversations_with_summaries("/p".into(), None, 1, 0))
                .unwrap_err()
                .kind,
            ServiceErrorKind::Unavailable
        );
    }

    // ---- CodeHealthService 契约 ----

    use dozer_model::CodeHealthReportInfo;

    struct MemCodeHealth(Result<Option<CodeHealthReportInfo>, ServiceError>);

    impl CodeHealthService for MemCodeHealth {
        fn get_code_health_report(
            &self,
            _p: i64,
        ) -> BoxFuture<'_, Result<Option<CodeHealthReportInfo>, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
        fn list_code_health_reports(
            &self,
            _p: i64,
            _l: u32,
        ) -> BoxFuture<'_, Result<Vec<CodeHealthReportInfo>, ServiceError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn save_code_health_report(
            &self,
            _p: i64,
            _i: CodeHealthReportInfo,
        ) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn CodeHealthService>` 的 async 块必须满足。
    #[test]
    fn dyn_code_health_futures_run_inside_tokio_spawn() {
        let svc: Arc<dyn CodeHealthService> = Arc::new(MemCodeHealth(Ok(None)));
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (got, list) = rt.block_on(async move {
            tokio::spawn(async move {
                let got = svc.get_code_health_report(7).await;
                let list = svc.list_code_health_reports(7, 2).await;
                (got, list)
            })
            .await
            .unwrap()
        });
        assert_eq!(got, Ok(None));
        assert_eq!(list, Ok(vec![]));
    }

    #[test]
    fn code_health_read_failure_is_distinguishable_from_no_report() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let none = MemCodeHealth(Ok(None));
        let down = MemCodeHealth(Err(ServiceError::new(ServiceErrorKind::Unavailable, "x")));
        assert_eq!(rt.block_on(none.get_code_health_report(1)), Ok(None));
        assert_eq!(
            rt.block_on(down.get_code_health_report(1))
                .unwrap_err()
                .kind,
            ServiceErrorKind::Unavailable
        );
    }

    // ---- ContextService 契约 ----

    use dozer_model::ContextItemInfo;

    struct MemContext(Result<Vec<ContextItemInfo>, ServiceError>);

    impl ContextService for MemContext {
        fn list_context_items(
            &self,
            _p: i64,
        ) -> BoxFuture<'_, Result<Vec<ContextItemInfo>, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
        fn add_context_item(
            &self,
            _p: i64,
            _k: String,
            _r: String,
        ) -> BoxFuture<'_, Result<ContextItemInfo, ServiceError>> {
            Box::pin(async { Err(ServiceError::new(ServiceErrorKind::Protocol, "mem")) })
        }
        fn remove_context_item(&self, _p: i64, _i: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn ContextService>` 的 async 块必须满足。
    #[test]
    fn dyn_context_futures_run_inside_tokio_spawn() {
        let svc: Arc<dyn ContextService> = Arc::new(MemContext(Ok(vec![])));
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (list, removed) = rt.block_on(async move {
            tokio::spawn(async move {
                let list = svc.list_context_items(7).await;
                let removed = svc.remove_context_item(7, 3).await;
                (list, removed)
            })
            .await
            .unwrap()
        });
        assert_eq!(list, Ok(vec![]));
        assert_eq!(removed, Ok(()));
    }

    #[test]
    fn context_read_failure_is_distinguishable_from_an_empty_list() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let empty = MemContext(Ok(vec![]));
        let down = MemContext(Err(ServiceError::new(ServiceErrorKind::Unavailable, "x")));
        assert!(rt.block_on(empty.list_context_items(1)).is_ok());
        assert_eq!(
            rt.block_on(down.list_context_items(1)).unwrap_err().kind,
            ServiceErrorKind::Unavailable
        );
    }

    // ---- EditHistoryService 契约 ----

    use dozer_model::{FileEditHistoryInfo, MutationOutcome};

    fn sample_edit() -> PreciseEdit {
        PreciseEdit {
            project_id: 1,
            path: "src/a.rs".into(),
            start_line: 2,
            start_col: 1,
            end_line: 2,
            end_col: 9,
            expected_text: "new".into(),
            new_text: "old".into(),
            summary: "撤销:改".into(),
            actor: "dozer".into(),
            session_id: "dozer-gui".into(),
        }
    }

    struct MemEditHistory(Result<Vec<FileEditHistoryInfo>, ServiceError>);

    impl EditHistoryService for MemEditHistory {
        fn list_file_edit_history(
            &self,
            _p: i64,
            _f: Option<String>,
            _l: u32,
        ) -> BoxFuture<'_, Result<Vec<FileEditHistoryInfo>, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
        fn apply_precise_edit(
            &self,
            _e: PreciseEdit,
        ) -> BoxFuture<'_, Result<MutationOutcome, ServiceError>> {
            Box::pin(async { Ok(MutationOutcome::NotFound) })
        }
    }

    #[test]
    fn precise_edit_is_cloneable_and_comparable() {
        let a = sample_edit();
        let b = a.clone();
        assert_eq!(a, b);
        assert_ne!(a, PreciseEdit { end_col: 10, ..b });
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn EditHistoryService>` 的 async 块必须满足。
    #[test]
    fn dyn_edit_history_futures_run_inside_tokio_spawn() {
        let svc: Arc<dyn EditHistoryService> = Arc::new(MemEditHistory(Ok(vec![])));
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (list, outcome) = rt.block_on(async move {
            tokio::spawn(async move {
                let list = svc.list_file_edit_history(1, None, 200).await;
                let outcome = svc.apply_precise_edit(sample_edit()).await;
                (list, outcome)
            })
            .await
            .unwrap()
        });
        assert_eq!(list, Ok(vec![]));
        assert_eq!(outcome, Ok(MutationOutcome::NotFound));
    }

    #[test]
    fn edit_history_read_failure_is_distinguishable_from_an_empty_list() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let empty = MemEditHistory(Ok(vec![]));
        let down = MemEditHistory(Err(ServiceError::new(ServiceErrorKind::Unavailable, "x")));
        assert!(
            rt.block_on(empty.list_file_edit_history(1, None, 1))
                .is_ok()
        );
        assert_eq!(
            rt.block_on(down.list_file_edit_history(1, None, 1))
                .unwrap_err()
                .kind,
            ServiceErrorKind::Unavailable
        );
    }

    // ---- BookmarkService 契约 ----

    use dozer_model::{BookmarkInfo, BookmarkScope};

    struct MemBookmark(Result<Vec<BookmarkInfo>, ServiceError>);

    impl BookmarkService for MemBookmark {
        fn list_bookmarks(
            &self,
            _p: Option<i64>,
        ) -> BoxFuture<'_, Result<Vec<BookmarkInfo>, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
        fn add_bookmark(
            &self,
            _s: BookmarkScope,
            _p: Option<i64>,
            _u: String,
            _t: String,
        ) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
        fn remove_bookmark(&self, _i: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
    }

    // ---- LocaleService 契约 ----

    struct MemLocale(Result<LocaleSnapshot, ServiceError>);

    impl LocaleService for MemLocale {
        fn get_locale(&self) -> BoxFuture<'_, Result<LocaleSnapshot, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
        fn set_locale(
            &self,
            _p: LocalePreference,
            _rev: u64,
            _c: Vec<String>,
        ) -> BoxFuture<'_, Result<LocaleWriteOutcome, ServiceError>> {
            let s = LocaleSnapshot::default_snapshot();
            Box::pin(async move { Ok(LocaleWriteOutcome::Applied { snapshot: s }) })
        }
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn BookmarkService>` 的 async 块必须满足。
    #[test]
    fn dyn_bookmark_futures_run_inside_tokio_spawn() {
        let svc: Arc<dyn BookmarkService> = Arc::new(MemBookmark(Ok(vec![])));
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (list, added) = rt.block_on(async move {
            tokio::spawn(async move {
                let list = svc.list_bookmarks(Some(7)).await;
                let added = svc
                    .add_bookmark(
                        BookmarkScope::Global,
                        None,
                        "http://a.com".into(),
                        "A".into(),
                    )
                    .await;
                (list, added)
            })
            .await
            .unwrap()
        });
        assert_eq!(list, Ok(vec![]));
        assert_eq!(added, Ok(()));
    }

    #[test]
    fn bookmark_read_failure_is_distinguishable_from_an_empty_list() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let empty = MemBookmark(Ok(vec![]));
        let down = MemBookmark(Err(ServiceError::new(ServiceErrorKind::Unavailable, "x")));
        assert!(rt.block_on(empty.list_bookmarks(None)).is_ok());
        assert_eq!(
            rt.block_on(down.list_bookmarks(None)).unwrap_err().kind,
            ServiceErrorKind::Unavailable
        );
    }

    /// 新增的两个方法同样能在 `tokio::spawn` 里使用(`Arc<dyn ProjectService>` 的 future 要 `Send + 'static`)。
    #[test]
    fn dyn_project_open_and_list_run_inside_tokio_spawn() {
        let project: Arc<dyn ProjectService> = Arc::new(MemProject);
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (opened, listed) = rt.block_on(async move {
            tokio::spawn(async move {
                let opened = project.open_project("/p".into()).await;
                let listed = project.list_projects().await;
                (opened, listed)
            })
            .await
            .unwrap()
        });
        assert_eq!(opened, Ok(None));
        assert_eq!(listed, Ok(vec![]));
    }

    // ---- DaemonService 契约 ----

    struct MemDaemon {
        list: Result<Vec<SessionInfo>, ServiceError>,
        stop: Result<(), ServiceError>,
        ensure: Result<(), ServiceError>,
    }
    impl DaemonService for MemDaemon {
        fn list_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionInfo>, ServiceError>> {
            let r = self.list.clone();
            Box::pin(async move { r })
        }
        fn shutdown_daemon(&self) -> BoxFuture<'_, Result<(), ServiceError>> {
            let r = self.stop.clone();
            Box::pin(async move { r })
        }
        fn ensure_running(&self) -> BoxFuture<'_, Result<(), ServiceError>> {
            let r = self.ensure.clone();
            Box::pin(async move { r })
        }
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn DaemonService>` 的 async 块必须满足。
    #[test]
    fn dyn_daemon_service_futures_run_inside_tokio_spawn() {
        let svc: Arc<dyn DaemonService> = Arc::new(MemDaemon {
            list: Ok(Vec::new()),
            stop: Ok(()),
            ensure: Ok(()),
        });
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (l, s, e) = rt.block_on(async move {
            tokio::spawn(async move {
                (
                    svc.list_sessions().await,
                    svc.shutdown_daemon().await,
                    svc.ensure_running().await,
                )
            })
            .await
            .unwrap()
        });
        assert_eq!((l, s, e), (Ok(Vec::new()), Ok(()), Ok(())));
    }

    #[test]
    fn daemon_list_failure_is_distinguishable_from_no_sessions() {
        let ok = MemDaemon {
            list: Ok(Vec::new()),
            stop: Ok(()),
            ensure: Ok(()),
        };
        let down = MemDaemon {
            list: Err(ServiceError::new(
                ServiceErrorKind::Unavailable,
                "daemon 断开",
            )),
            stop: Ok(()),
            ensure: Ok(()),
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        assert_eq!(rt.block_on(ok.list_sessions()), Ok(Vec::new()));
        assert_eq!(
            rt.block_on(down.list_sessions()).unwrap_err().kind,
            ServiceErrorKind::Unavailable
        );
    }
}
