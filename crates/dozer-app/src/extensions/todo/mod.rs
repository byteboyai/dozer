//! Todo 面板(设计 2026-08-06;SQLite 迁移计划 2026-09-01;已并入
//! `extensions::todo`):任务列表的唯一权威源在 dozerd 侧 `dozer.db` 的
//! SQLite `todos` 表(TodoStore 归档)。本模块只持有每个 `Workspace` 上的
//! 渲染态(`WorkspaceState`)与视图/更新逻辑,List/Add/Toggle 经 `Client`
//! 走 UDS 与 dozerd 同步;不再直接读写 `.dozer/todo.md`/`todo_meta.json`
//! (MARKDOWN 整文件视图已随迁移移除)。

// 面板本体已抽到共享库 `dozer-panel-todo`(S3a);这里重新导出,`todo::…` 的调用点不变。
pub(crate) use dozer_panel_todo::view::*;
pub(crate) use dozer_panel_todo::*;
// 产品侧适配(展示契约、类型映射)在 `host_adapters::todo_panel`(面板边界门禁不许 `extensions/` 下出现 `crate::app::App`)。
pub(crate) use crate::host_adapters::todo_panel::{
    current_view_payload, hover_id_of, text_target_of,
};

dozer_core::scope!(pub(crate) LOG, panel, "todo");

#[cfg(test)]
mod tests {
    use super::*;

    use dozer_model::TodoInfo;

    fn todo_info(id: i64, text: &str, done: bool) -> TodoInfo {
        TodoInfo {
            id,
            project_id: 1,
            text: text.to_string(),
            done,
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

    /// 语言无关的稳定文本(原 `toast::Pending::text_stable`,发件箱改成中性类型后在测试里直接算)。
    fn stable(n: &dozer_panel_host::Notice) -> String {
        crate::extensions::toast::normalize(&crate::i18n::host_render_stable(&n.text))
    }

    // ---- H6:切入钩子 ----

    use crate::panel_host::testing::{TIMEOUT, offline_ctx, runtime};
    use crate::service_fakes::FakeTodo;
    use dozer_services::ServiceErrorKind;
    use std::sync::{Mutex, mpsc};

    #[test]
    fn on_activate_requests_both_the_todo_list_and_the_category_tree() {
        let rt = runtime();
        let (ctx, rx) = offline_ctx::<Message>(&rt, 7, None);
        let ctx = crate::panel_host::ActivationCtx {
            io: ctx.io.with_services(FakeTodo::default().into_services()),
            ..ctx
        };
        on_activate(&WorkspaceState::default(), &ctx);
        let first = rx.recv_timeout(TIMEOUT).unwrap().result_payload();
        let second = rx.recv_timeout(TIMEOUT).unwrap().result_payload();
        let loaded = |m: &Message| matches!(m, Message::Loaded(_));
        let categories = |m: &Message| matches!(m, Message::CategoriesLoaded(_));
        assert!(
            (loaded(&first) && categories(&second)) || (categories(&first) && loaded(&second)),
            "两个请求各回一条消息"
        );
    }

    fn svc(fake: &FakeTodo) -> std::sync::Arc<dyn dozer_services::TodoService> {
        fake.clone().into_services().todo
    }

    fn collect() -> (
        impl Fn(Message) + Send + Sync + 'static,
        mpsc::Receiver<Message>,
    ) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        (
            move |m| {
                let _ = tx.lock().unwrap().send(m);
            },
            rx,
        )
    }

    #[test]
    fn unavailable_read_failure_keeps_both_lists_and_stays_silent() {
        let rt = runtime();
        let service = svc(&FakeTodo::default());
        let mut ws = WorkspaceState::default();
        let (e1, _r1) = collect();
        update(
            &mut ws,
            Message::Loaded(vec![todo_info(1, "a", false)]),
            7,
            &service,
            rt.handle(),
            e1,
        );
        let (e2, _r2) = collect();
        update(
            &mut ws,
            Message::LoadFailed(ServiceErrorKind::Unavailable, "daemon 断开".into()),
            7,
            &service,
            rt.handle(),
            e2,
        );
        assert_eq!(ws.items().len(), 1, "读失败不得清空已有任务");
        assert!(
            ws.take_outbox().is_empty(),
            "Unavailable 由顶栏徽标表达,不弹 Toast"
        );
    }

    #[test]
    fn rejected_read_failure_keeps_lists_and_toasts_once_with_a_key() {
        let rt = runtime();
        let service = svc(&FakeTodo::default());
        let mut ws = WorkspaceState::default();
        let (e1, _r1) = collect();
        update(
            &mut ws,
            Message::Loaded(vec![todo_info(1, "a", false)]),
            7,
            &service,
            rt.handle(),
            e1,
        );
        let (e2, _r2) = collect();
        update(
            &mut ws,
            Message::LoadFailed(ServiceErrorKind::Rejected, "daemon 错误: boom".into()),
            7,
            &service,
            rt.handle(),
            e2,
        );
        assert_eq!(ws.items().len(), 1);
        let toasts = ws.take_outbox();
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].level, dozer_panel_host::NoticeLevel::Error);
        assert!(stable(&toasts[0]).contains("daemon 错误: boom"));
        assert_eq!(toasts[0].key.as_deref(), Some("todo-load"));
    }

    #[test]
    fn failing_reads_emit_load_failed_instead_of_an_empty_loaded() {
        let rt = runtime();
        let service = svc(&FakeTodo::failing_reads(ServiceErrorKind::Unavailable, "x"));
        let (emit, rx) = collect();
        let emit = std::sync::Arc::new(emit);
        let e1 = emit.clone();
        request_todos_refresh(&Default::default(), 7, &service, rt.handle(), move |m| {
            e1(m)
        });
        request_categories_refresh(&Default::default(), 7, &service, rt.handle(), move |m| {
            emit(m)
        });
        for _ in 0..2 {
            assert!(
                matches!(
                    rx.recv_timeout(TIMEOUT).unwrap().result_payload(),
                    Message::LoadFailed(ServiceErrorKind::Unavailable, _)
                ),
                "读失败必须发 LoadFailed,不得发 Loaded(vec![])"
            );
        }
    }

    #[test]
    fn write_failure_text_is_the_service_message_unchanged() {
        let rt = runtime();
        let fake = FakeTodo::failing_writes(ServiceErrorKind::Rejected, "daemon 错误: boom");
        let mut ws = WorkspaceState::default();
        let (emit, rx) = collect();
        update(
            &mut ws,
            Message::ReorderTo {
                id: 1,
                after_id: None,
            },
            7,
            &svc(&fake),
            rt.handle(),
            emit,
        );
        match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
            Message::Mutated(Err(text)) => assert_eq!(text, "daemon 错误: boom"),
            other => panic!("意外消息: {other:?}"),
        }
        assert_eq!(fake.calls(), vec!["reorder_todo(1,None)".to_string()]);
    }

    #[test]
    fn new_category_enters_rename_with_the_id_the_service_returned() {
        let rt = runtime();
        let fake = FakeTodo::default();
        let mut ws = WorkspaceState::default();
        let (emit, rx) = collect();
        update(
            &mut ws,
            Message::CategoryNewSibling(None),
            7,
            &svc(&fake),
            rt.handle(),
            emit,
        );
        let first = rx.recv_timeout(TIMEOUT).unwrap().result_payload();
        let second = rx.recv_timeout(TIMEOUT).unwrap().result_payload();
        assert!(
            matches!(first, Message::CategoryRenameStart(900)),
            "{first:?}"
        );
        assert!(
            matches!(second, Message::CategoryMutated(Ok(()))),
            "{second:?}"
        );
        assert_eq!(
            fake.calls(),
            vec!["add_category(7,None,新分类)".to_string()]
        );
    }

    #[test]
    fn a_failed_category_mutation_refreshes_both_lists_from_the_service() {
        let rt = runtime();
        let fake = FakeTodo::failing_writes(ServiceErrorKind::Rejected, "daemon 错误: nope");
        let mut ws = WorkspaceState::default();
        let (emit, rx) = collect();
        update(
            &mut ws,
            Message::CategoryMutated(Err("daemon 错误: nope".into())),
            7,
            &svc(&fake),
            rt.handle(),
            emit,
        );
        let a = rx.recv_timeout(TIMEOUT).unwrap().result_payload();
        let b = rx.recv_timeout(TIMEOUT).unwrap().result_payload();
        let kinds = |m: &Message| matches!(m, Message::Loaded(_) | Message::CategoriesLoaded(_));
        assert!(kinds(&a) && kinds(&b), "{a:?} {b:?}");
        assert_eq!(ws.take_outbox().len(), 1, "分类操作失败的 Toast 保持不变");
    }

    #[test]
    fn repeated_identical_read_failures_toast_only_once() {
        let rt = runtime();
        let service = svc(&FakeTodo::default());
        let mut ws = WorkspaceState::default();
        for _ in 0..3 {
            let (emit, _rx) = collect();
            update(
                &mut ws,
                Message::LoadFailed(ServiceErrorKind::Protocol, "解码失败".into()),
                7,
                &service,
                rt.handle(),
                emit,
            );
        }
        assert_eq!(ws.take_outbox().len(), 1, "轮询每秒失败一次也只提示一次");
        let (emit, _rx) = collect();
        update(
            &mut ws,
            Message::LoadFailed(ServiceErrorKind::Rejected, "另一种错误".into()),
            7,
            &service,
            rt.handle(),
            emit,
        );
        assert_eq!(ws.take_outbox().len(), 1, "换了一种错误要再提示");
    }

    #[test]
    fn stale_async_results_preserve_todos_categories_and_errors() {
        let rt = runtime();
        let service = svc(&FakeTodo::default());
        let mut state = WorkspaceState::default();
        let apply =
            |state: &mut WorkspaceState, msg| update(state, msg, 7, &service, rt.handle(), |_| {});
        let old = state.requests.begin(Some(7), "todos");
        let category = state.requests.begin(Some(7), "categories");
        let new = state.requests.begin(Some(7), "todos");
        apply(
            &mut state,
            Message::AsyncResult(
                new.clone(),
                Box::new(Message::Loaded(vec![todo_info(2, "new", false)])),
            ),
        );
        apply(
            &mut state,
            Message::AsyncResult(old.clone(), Box::new(Message::Loaded(vec![]))),
        );
        apply(
            &mut state,
            Message::AsyncResult(
                old,
                Box::new(Message::LoadFailed(
                    ServiceErrorKind::Rejected,
                    "old failure".into(),
                )),
            ),
        );
        apply(
            &mut state,
            Message::AsyncResult(
                category,
                Box::new(Message::LoadFailed(
                    ServiceErrorKind::Rejected,
                    "category failure".into(),
                )),
            ),
        );
        assert_eq!(state.items()[0].id, 2);
        let notices = state.take_outbox();
        assert_eq!(notices.len(), 1);
        assert!(stable(&notices[0]).contains("category failure"));
        let mut other = WorkspaceState::default();
        apply(
            &mut other,
            Message::AsyncResult(
                new,
                Box::new(Message::Loaded(vec![todo_info(2, "wrong project", false)])),
            ),
        );
        assert!(other.items().is_empty());
    }
    #[test]
    fn queued_refresh_is_rejected_after_new_request_even_before_new_result_arrives() {
        let rt = runtime();
        let service = svc(&FakeTodo::ok(vec![todo_info(2, "new", false)], vec![]));
        let mut state = WorkspaceState::default();
        update(
            &mut state,
            Message::Loaded(vec![todo_info(1, "existing", false)]),
            7,
            &service,
            rt.handle(),
            |_| {},
        );
        let (emit, rx) = collect();
        request_todos_refresh(&state.requests, 7, &service, rt.handle(), emit);
        let queued = rx.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(queued.result_project(), Some(7));
        let (emit, rx) = collect();
        request_todos_refresh(&state.requests, 7, &service, rt.handle(), emit);
        update(&mut state, queued, 7, &service, rt.handle(), |_| {});
        assert_eq!(state.items()[0].id, 1);
        let current = rx.recv_timeout(TIMEOUT).unwrap();
        update(
            &mut state,
            current.clone(),
            99,
            &service,
            rt.handle(),
            |_| {},
        );
        assert_eq!(
            state.items()[0].id,
            1,
            "wrong project rejected even with same panel"
        );
        update(&mut state, current, 7, &service, rt.handle(), |_| {});
        assert_eq!(state.items()[0].id, 2);
    }
}
