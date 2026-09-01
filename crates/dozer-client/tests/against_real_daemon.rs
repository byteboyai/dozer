use dozer_client::{Client, TermEvent};
use dozerd::registry::SessionRegistry;
use std::sync::Arc;
use std::time::Duration;

fn temp_sock() -> (std::path::PathBuf, uuid::Uuid) {
    let id = uuid::Uuid::new_v4();
    let short = &id.to_string()[..8];
    let path = std::path::PathBuf::from(format!("/tmp/dz-{short}.sock"));
    (path, id)
}

async fn start_daemon() -> (std::path::PathBuf, Arc<SessionRegistry>, CleanupGuard) {
    let (sock, id) = temp_sock();
    let registry = Arc::new(SessionRegistry::new());
    let s = sock.clone();
    let r = registry.clone();
    let db = std::path::PathBuf::from(format!("/tmp/dz-{}.db", uuid::Uuid::new_v4()));
    let store = Arc::new(dozerd::acceptance::AcceptanceStore::open(&db).unwrap());
    let projects = Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap());
    let bookmarks = Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap());
    let transcripts = Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap());
    let session_summaries =
        Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap());
    let backfill_registry = Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new());
    let todos = Arc::new(dozerd::todo::TodoStore::new(&db).unwrap());
    let categories = Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap());
    tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            r,
            store,
            projects,
            bookmarks,
            transcripts,
            session_summaries,
            backfill_registry,
            todos,
            categories,
        )
        .await
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (sock, registry, CleanupGuard(id))
}

struct CleanupGuard(uuid::Uuid);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let short = &self.0.to_string()[..8];
        let path = std::path::PathBuf::from(format!("/tmp/dz-{short}.sock"));
        let _ = std::fs::remove_file(&path);
    }
}

#[tokio::test]
async fn full_client_lifecycle() {
    let (sock, _registry, _guard) = start_daemon().await;
    let c = Client::new(sock);
    assert!(c.list().await.unwrap().is_empty());

    let info = c
        .create(
            "测试",
            "/bin/sh",
            &["-c".into(), "echo ready; cat".into()],
            "/tmp",
            80,
            24,
            1,
        )
        .await
        .unwrap();
    assert!(info.alive);

    let (snap, _next, mut rx) = c.attach(&info.id, 0).await.unwrap();
    let mut seen = snap;
    while !seen.windows(5).any(|w| w == b"ready") {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Some(TermEvent::Output(d))) => seen.extend(d),
            other => panic!("expect output, got {other:?}"),
        }
    }

    c.write(&info.id, b"pong\n").await.unwrap();
    let mut echoed = Vec::new();
    while !echoed.windows(4).any(|w| w == b"pong") {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Some(TermEvent::Output(d))) => echoed.extend(d),
            other => panic!("expect echo, got {other:?}"),
        }
    }

    c.resize(&info.id, 100, 30).await.unwrap();
    c.kill(&info.id).await.unwrap();
    // kill 后 attach 流应收到 Exited
    let mut exited = false;
    for _ in 0..50 {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Some(TermEvent::Exited(_))) => {
                exited = true;
                break;
            }
            Ok(Some(_)) => continue,
            _ => continue,
        }
    }
    assert!(exited);
    assert!(!c.list().await.unwrap()[0].alive);
}

/// 审阅 Important：attach() 内部读任务过去只在"收到新行后 tx.send 失败"
/// 才退出。空闲会话（无输出）的 tab 被关闭、receiver 被 drop 后，读任务
/// 会永远卡在 `lines.next_line().await` 上——UnixStream 不关，daemon 侧
/// `handle_conn` 也不退出，两端各滞留一个任务 + 一个 FD。
///
/// 用 `Session::subscriber_count()`（daemon 侧 broadcast 订阅数，
/// `handle_conn` 在 attach 期间持有、连接关闭时随之 drop）直接观察修复
/// 是否把资源释放传导了回去——这是比"反复 attach 不阻塞"更强的断言，
/// 不需要改 `attach()` 的公开签名。同时保留任务描述里要求的行为断言
/// （20 轮 attach→drop 后第 21 次 attach 仍需在 1s 内即时成功）作为兜底。
#[tokio::test]
async fn reader_task_exits_when_receiver_dropped() {
    let (sock, registry, _guard) = start_daemon().await;
    let c = Client::new(sock);

    // 空闲会话：sleep 30 不产生任何输出，确保读循环唯一的退出信号
    // 只能来自 receiver 被 drop（不会被 stop 事件误触发退出）。
    let info = c
        .create(
            "idle",
            "/bin/sh",
            &["-c".into(), "sleep 30".into()],
            "/tmp",
            80,
            24,
            1,
        )
        .await
        .unwrap();
    let session = registry.get(&info.id).expect("session just created");
    assert_eq!(session.subscriber_count(), 0, "尚未 attach，不应有订阅");

    for round in 0..20 {
        let (_snap, _next, rx) = c.attach(&info.id, 0).await.unwrap();
        assert_eq!(
            session.subscriber_count(),
            1,
            "round {round}: attach 应产生一个订阅"
        );
        drop(rx); // 模拟关闭 tab：不再有人消费 TermEvent

        // 给读任务一点时间在 tokio::select! 里观察到 tx.closed() 并退出，
        // 进而 lines/writer 一并 drop → UnixStream 两端关闭 → daemon 侧
        // handle_conn 在下一次 next_line() 收到 EOF 退出 → sub 被 drop。
        let mut released = false;
        for _ in 0..100 {
            if session.subscriber_count() == 0 {
                released = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            released,
            "round {round}: receiver drop 500ms 后 daemon 侧订阅仍未释放，读任务/连接滞留"
        );
    }

    // 硬断言：即便订阅计数这条信号出于某种原因不够灵敏，也要保证
    // 20 轮滞留不会让后续 attach 阻塞——1s 超时兜底暴露卡死。
    let (_snap, _next, rx21) = tokio::time::timeout(Duration::from_secs(1), c.attach(&info.id, 0))
        .await
        .expect("第 21 次 attach 不应超时/阻塞")
        .expect("第 21 次 attach 应成功返回 Attached");
    drop(rx21);

    c.kill(&info.id).await.unwrap();
}

#[tokio::test]
async fn preview_context_push_and_query_round_trips() {
    use dozer_core::protocol::PreviewContext;

    let (sock, _registry, _guard) = start_daemon().await;
    let c = Client::new(sock);

    assert_eq!(c.get_preview_context(1).await.unwrap(), None);

    let ctx = PreviewContext {
        path: "/repo/src/main.rs".into(),
        start_line: 3,
        start_col: 1,
        end_line: 5,
        end_col: 2,
        has_selection: true,
        updated_at_ms: 1_700_000_000_000,
    };
    c.update_preview_context(1, Some(ctx.clone()))
        .await
        .unwrap();
    assert_eq!(c.get_preview_context(1).await.unwrap(), Some(ctx.clone()));

    // 不同 project_id 互不影响。
    assert_eq!(c.get_preview_context(2).await.unwrap(), None);

    // 后写覆盖前写。
    let ctx2 = PreviewContext {
        path: "/repo/README.md".into(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 1,
        has_selection: false,
        updated_at_ms: 1_700_000_001_000,
    };
    c.update_preview_context(1, Some(ctx2.clone()))
        .await
        .unwrap();
    assert_eq!(c.get_preview_context(1).await.unwrap(), Some(ctx2));

    // 推 None 清空。
    c.update_preview_context(1, None).await.unwrap();
    assert_eq!(c.get_preview_context(1).await.unwrap(), None);
}

#[tokio::test]
async fn list_conversations_and_usage_roundtrip_against_real_daemon() {
    let (sock, _registry, _guard) = start_daemon().await;
    let client = Client::new(sock);
    let conversations = client
        .list_conversations("/no/such/project", None, 10, 0)
        .await
        .unwrap();
    assert!(conversations.is_empty());

    let turns = client
        .get_conversation_turns("no-such-id", -1, 10)
        .await
        .unwrap();
    assert!(turns.is_empty());

    let with_summaries = client
        .list_conversations_with_summaries("/no/such/project", None, 50, 0)
        .await
        .unwrap();
    assert!(with_summaries.is_empty());

    let usage = client
        .get_usage_summary("/no/such/project", None)
        .await
        .unwrap();
    assert!(usage.is_empty());
}

#[tokio::test]
async fn record_and_get_session_summary_via_client() {
    let (sock, _registry, _guard) = start_daemon().await;
    let client = Client::new(sock);
    let session = client
        .create(
            "测试",
            "/bin/sh",
            &["-c".into(), "sleep 5".into()],
            "/tmp",
            80,
            24,
            1,
        )
        .await
        .unwrap();

    assert_eq!(client.get_session_summary(&session.id).await.unwrap(), None);

    client
        .record_session_summary(&session.id, "标题", "摘要内容")
        .await
        .unwrap();

    let got = client
        .get_session_summary(&session.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.title, "标题");
    assert_eq!(got.summary, "摘要内容");
    assert_eq!(got.status, dozer_core::protocol::SummaryStatus::AiGenerated);

    client.kill(&session.id).await.unwrap();
}

#[tokio::test]
async fn backfill_session_summaries_on_empty_project_completes_immediately() {
    let (sock, _registry, _guard) = start_daemon().await;
    let client = Client::new(sock);
    client
        .backfill_session_summaries("/no/such/project")
        .await
        .unwrap();
    // 空项目:total=0,应该立刻可查到"已完成"(completed>=total)。
    let (completed, total) = client
        .get_session_summary_backfill_status("/no/such/project")
        .await
        .unwrap();
    assert_eq!((completed, total), (0, 0));
}

#[tokio::test]
async fn get_backfill_status_for_never_started_cwd_returns_zero() {
    let (sock, _registry, _guard) = start_daemon().await;
    let client = Client::new(sock);
    let (completed, total) = client
        .get_session_summary_backfill_status("/never/touched")
        .await
        .unwrap();
    assert_eq!((completed, total), (0, 0));
}

#[tokio::test]
async fn add_toggle_and_list_todos_roundtrip() {
    let (sock, _registry, _guard) = start_daemon().await;
    let client = Client::new(sock);

    let t1 = client.add_todo(1, "第一条").await.unwrap();
    let t2 = client.add_todo(1, "第二条").await.unwrap();

    let listed = client.list_todos(1).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, t2.id, "后加的排最前");
    assert_eq!(listed[1].id, t1.id);

    let toggled = client.toggle_todo(t1.id, true).await.unwrap();
    assert!(toggled.done);
    assert!(toggled.completed_at_ms.is_some());

    let listed = client.list_todos(1).await.unwrap();
    assert_eq!(listed[0].id, t2.id, "唯一待办排最前");
    assert_eq!(listed[1].id, t1.id, "已完成沉底");
}

#[tokio::test]
async fn edit_reorder_plan_date_and_dispatch_roundtrip() {
    let (sock, _registry, _guard) = start_daemon().await;
    let client = Client::new(sock);

    let t1 = client.add_todo(1, "旧文字").await.unwrap();
    let edited = client.edit_todo_text(t1.id, "新文字").await.unwrap();
    assert_eq!(edited.text, "新文字");

    let t2 = client.add_todo(1, "另一条").await.unwrap();
    let reordered = client.reorder_todo(t1.id, Some(t2.id)).await.unwrap();
    assert_eq!(reordered.id, t1.id);
    let listed = client.list_todos(1).await.unwrap();
    assert_eq!(listed[0].id, t2.id);
    assert_eq!(listed[1].id, t1.id);

    let with_date = client
        .set_todo_plan_date(t1.id, Some("08-10"))
        .await
        .unwrap();
    assert_eq!(with_date.plan_date, Some("08-10".to_string()));
    let cleared = client.set_todo_plan_date(t1.id, None).await.unwrap();
    assert_eq!(cleared.plan_date, None);

    let dispatched = client
        .record_todo_dispatch(t1.id, "sess-xyz")
        .await
        .unwrap();
    assert_eq!(dispatched.dispatch_session_id, Some("sess-xyz".to_string()));
}

#[tokio::test]
async fn toggle_unknown_todo_id_errors() {
    let (sock, _registry, _guard) = start_daemon().await;
    let client = Client::new(sock);
    let err = client.toggle_todo(999, true).await.unwrap_err();
    assert!(err.to_string().contains("任务不存在"));
}

#[tokio::test]
async fn category_crud_and_reorder_roundtrip() {
    let (sock, _registry, _guard) = start_daemon().await;
    let client = Client::new(sock);

    let frontend = client.add_category(1, None, "前端").await.unwrap();
    let backend = client.add_category(1, None, "后端").await.unwrap();
    let ui = client
        .add_category(1, Some(frontend.id), "UI")
        .await
        .unwrap();

    let listed = client.list_categories(1).await.unwrap();
    assert_eq!(listed.len(), 3);

    let renamed = client.rename_category(ui.id, "界面").await.unwrap();
    assert_eq!(renamed.name, "界面");

    let reparented = client
        .reparent_category(ui.id, Some(backend.id))
        .await
        .unwrap();
    assert_eq!(reparented.parent_id, Some(backend.id));

    client.delete_category(ui.id).await.unwrap();
    let listed = client.list_categories(1).await.unwrap();
    assert_eq!(listed.len(), 2);

    let moved = client
        .move_category_sibling(backend.id, dozer_core::protocol::CategoryMoveDirection::Up)
        .await
        .unwrap();
    assert_eq!(moved.id, backend.id);
}

#[tokio::test]
async fn set_todo_category_roundtrip() {
    let (sock, _registry, _guard) = start_daemon().await;
    let client = Client::new(sock);

    let category = client.add_category(1, None, "分类").await.unwrap();
    let todo = client.add_todo(1, "任务").await.unwrap();
    assert_eq!(todo.category_id, None);

    let categorized = client
        .set_todo_category(todo.id, Some(category.id))
        .await
        .unwrap();
    assert_eq!(categorized.category_id, Some(category.id));

    let cleared = client.set_todo_category(todo.id, None).await.unwrap();
    assert_eq!(cleared.category_id, None);
}
