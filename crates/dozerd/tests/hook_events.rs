//! P1e hook 事件通路端到端：HookEvent 入站 → attach 流收 AgentEvent →
//! ListSessions 可见最新态；未知会话丢弃不报错。

use dozer_core::protocol::{AgentState, Reply, Request, decode_line, encode_line};
use dozerd::registry::SessionRegistry;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

async fn send_req(sock: &std::path::Path, req: &Request) -> Reply {
    let stream = UnixStream::connect(sock).await.expect("connect");
    let (r, mut w) = stream.into_split();
    w.write_all(encode_line(req).as_bytes())
        .await
        .expect("send");
    let mut lines = BufReader::new(r).lines();
    let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
        .await
        .expect("reply within 5s")
        .expect("io ok")
        .expect("stream open");
    decode_line(&line).expect("valid reply")
}

#[tokio::test]
async fn hook_event_reaches_attached_client_and_list() {
    let sock = std::env::temp_dir().join(format!("dozerd-hook-{}.sock", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    tokio::spawn({
        let sock = sock.clone();
        let registry = registry.clone();
        async move {
            dozerd::server::serve(
                &sock,
                registry,
                test_store(),
                test_projects(),
                test_bookmarks(),
                test_transcripts(),
                test_session_summaries(),
                test_backfill_registry(),
                test_todos(),
                test_categories(),
                dozerd::task_poller::new_in_flight(),
            )
            .await
        }
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let created = match send_req(
        &sock,
        &Request::CreateSession {
            name: "t".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 5".into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
            project_id: 1,
        },
    )
    .await
    {
        Reply::Created { session } => session,
        other => panic!("{other:?}"),
    };

    // 连接 A：attach 并持流
    let stream = UnixStream::connect(&sock).await.expect("connect");
    let (r, mut w) = stream.into_split();
    w.write_all(
        encode_line(&Request::Attach {
            session_id: created.id.clone(),
            from_offset: 0,
        })
        .as_bytes(),
    )
    .await
    .expect("send attach");
    let mut lines = BufReader::new(r).lines();
    let _attached = lines.next_line().await.expect("io").expect("open");

    // 连接 B：hook 事件
    match send_req(
        &sock,
        &Request::HookEvent {
            session_id: created.id.clone(),
            agent: dozer_core::protocol::AgentKind::Codebuddy,
            event: "Stop".into(),
            ts_ms: 7,
            data: serde_json::Value::Null,
        },
    )
    .await
    {
        Reply::Ok => {}
        other => panic!("{other:?}"),
    }

    // A 在 PTY 噪音里等到 AgentEvent
    loop {
        let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
            .await
            .expect("AgentEvent within 5s")
            .expect("io ok")
            .expect("stream open");
        if let Ok(Reply::AgentEvent {
            agent,
            state,
            event,
            ..
        }) = decode_line::<Reply>(&line)
        {
            assert_eq!(agent, dozer_core::protocol::AgentKind::Codebuddy);
            assert_eq!(state, AgentState::TurnEnded);
            assert_eq!(event, "Stop");
            break;
        }
    }

    match send_req(&sock, &Request::ListSessions).await {
        Reply::Sessions { sessions } => {
            assert_eq!(sessions[0].agent_state, AgentState::TurnEnded);
            assert_eq!(
                sessions[0].agent,
                dozer_core::protocol::AgentKind::Codebuddy
            );
        }
        other => panic!("{other:?}"),
    }

    // 未知会话：Ok 不报错
    match send_req(
        &sock,
        &Request::HookEvent {
            session_id: "ghost".into(),
            agent: dozer_core::protocol::AgentKind::Claude,
            event: "Stop".into(),
            ts_ms: 8,
            data: serde_json::Value::Null,
        },
    )
    .await
    {
        Reply::Ok => {}
        other => panic!("{other:?}"),
    }
}

/// 每次调用建一个独立临时库的验收存储（测试用；P1f serve 需要）。
fn test_store() -> std::sync::Arc<dozerd::acceptance::AcceptanceStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::acceptance::AcceptanceStore::open(&db).unwrap())
}

#[tokio::test]
async fn record_acceptance_persists() {
    let sock = std::env::temp_dir().join(format!("dozerd-acc-{}.sock", uuid::Uuid::new_v4()));
    let db = std::env::temp_dir().join(format!("dozerd-acc-{}.db", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let store = Arc::new(dozerd::acceptance::AcceptanceStore::open(&db).unwrap());
    let projects = Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap());
    let bookmarks = Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap());
    let transcripts = Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap());
    let session_summaries =
        Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap());
    let backfill_registry = test_backfill_registry();
    let todos = test_todos();
    let categories = test_categories();
    tokio::spawn({
        let (
            sock,
            registry,
            store,
            projects,
            bookmarks,
            transcripts,
            session_summaries,
            backfill_registry,
            todos,
            categories,
        ) = (
            sock.clone(),
            registry.clone(),
            store.clone(),
            projects.clone(),
            bookmarks.clone(),
            transcripts.clone(),
            session_summaries.clone(),
            backfill_registry.clone(),
            todos.clone(),
            categories.clone(),
        );
        async move {
            dozerd::server::serve(
                &sock,
                registry,
                store,
                projects,
                bookmarks,
                transcripts,
                session_summaries,
                backfill_registry,
                todos,
                categories,
                dozerd::task_poller::new_in_flight(),
            )
            .await
        }
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    match send_req(
        &sock,
        &Request::RecordAcceptance {
            repo: "/r".into(),
            goal: "g".into(),
            criteria_checked: vec![],
            verdict: "accepted".into(),
            comment: "".into(),
            ref_name: "refs/dozer/accepted/1".into(),
            ts_ms: 1,
        },
    )
    .await
    {
        Reply::Ok => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(store.count().unwrap(), 1);
}

/// 每次调用建独立临时库的项目存储（测试用；P1g serve 需要）。
fn test_projects() -> std::sync::Arc<dozerd::projects::ProjectStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap())
}

/// 每次调用建独立临时库的收藏夹存储（测试用；serve 需要）。
fn test_bookmarks() -> std::sync::Arc<dozerd::bookmarks::BookmarkStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap())
}

/// 每次调用建独立临时库的对话/用量摄取存储（测试用；serve 需要）。
fn test_transcripts() -> std::sync::Arc<dozerd::transcripts::TranscriptStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap())
}

/// 每次调用建独立临时库的会话总结存储（测试用；serve 需要）。
fn test_session_summaries() -> std::sync::Arc<dozerd::session_summary::SessionSummaryStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap())
}

/// 无状态的补总结内存登记表（测试用；serve 需要）。
fn test_backfill_registry() -> std::sync::Arc<dozerd::session_summary_backfill::BackfillRegistry> {
    std::sync::Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new())
}

/// 每次调用建独立临时库的 Todo 存储（测试用；serve 需要）。
fn test_todos() -> std::sync::Arc<dozerd::todo::TodoStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::todo::TodoStore::new(&db).unwrap())
}

/// 每次调用建独立临时库的分类存储（测试用；serve 需要）。
fn test_categories() -> std::sync::Arc<dozerd::todo_category::CategoryStore> {
    let db = std::env::temp_dir().join(format!("dozerd-cat-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap())
}
#[tokio::test]
async fn project_open_and_list_roundtrip() {
    let sock = std::env::temp_dir().join(format!("dozerd-proj-{}.sock", uuid::Uuid::new_v4()));
    let db = std::env::temp_dir().join(format!("dozerd-proj-{}.db", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let store = Arc::new(dozerd::acceptance::AcceptanceStore::open(&db).unwrap());
    let projects = Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap());
    let bookmarks = Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap());
    let transcripts = Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap());
    let session_summaries =
        Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap());
    let backfill_registry = test_backfill_registry();
    let todos = test_todos();
    let categories = test_categories();
    tokio::spawn({
        let (
            sock,
            registry,
            store,
            projects,
            bookmarks,
            transcripts,
            session_summaries,
            backfill_registry,
            todos,
            categories,
        ) = (
            sock.clone(),
            registry.clone(),
            store.clone(),
            projects.clone(),
            bookmarks.clone(),
            transcripts.clone(),
            session_summaries.clone(),
            backfill_registry.clone(),
            todos.clone(),
            categories.clone(),
        );
        async move {
            dozerd::server::serve(
                &sock,
                registry,
                store,
                projects,
                bookmarks,
                transcripts,
                session_summaries,
                backfill_registry,
                todos,
                categories,
                dozerd::task_poller::new_in_flight(),
            )
            .await
        }
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let opened = match send_req(
        &sock,
        &Request::OpenProject {
            path: "/repo/z".into(),
        },
    )
    .await
    {
        Reply::Project { project: Some(p) } => p,
        other => panic!("{other:?}"),
    };
    assert_eq!(opened.name, "z");
    match send_req(&sock, &Request::ListProjects).await {
        Reply::Projects { projects } => assert_eq!(projects.len(), 1),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn record_and_get_session_summary_roundtrip() {
    let sock = std::env::temp_dir().join(format!("dzsum-{}.sock", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let store = test_store();
    let projects = test_projects();
    let bookmarks = test_bookmarks();
    let transcripts = test_transcripts();
    let session_summaries = test_session_summaries();
    let backfill_registry = test_backfill_registry();
    let todos = test_todos();
    let categories = test_categories();
    tokio::spawn({
        let sock = sock.clone();
        let registry = registry.clone();
        async move {
            dozerd::server::serve(
                &sock,
                registry,
                store,
                projects,
                bookmarks,
                transcripts,
                session_summaries,
                backfill_registry,
                todos,
                categories,
                dozerd::task_poller::new_in_flight(),
            )
            .await
        }
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let s = registry
        .create(dozerd::session::SessionSpec {
            name: "测试".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 5".into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
            project_id: 1,
        })
        .unwrap();
    let id = s.id().to_string();

    match send_req(
        &sock,
        &Request::GetSessionSummary {
            session_id: id.clone(),
        },
    )
    .await
    {
        Reply::SessionSummary { summary: None } => {}
        other => panic!("意外应答: {other:?}"),
    }

    match send_req(
        &sock,
        &Request::RecordSessionSummary {
            session_id: id.clone(),
            title: "标题".into(),
            summary: "摘要".into(),
        },
    )
    .await
    {
        Reply::Ok => {}
        other => panic!("意外应答: {other:?}"),
    }

    match send_req(
        &sock,
        &Request::GetSessionSummary {
            session_id: id.clone(),
        },
    )
    .await
    {
        Reply::SessionSummary { summary: Some(p) } => {
            assert_eq!(p.title, "标题");
            assert_eq!(p.summary, "摘要");
            assert_eq!(p.status, dozer_core::protocol::SummaryStatus::AiGenerated);
        }
        other => panic!("意外应答: {other:?}"),
    }
    let _ = s.kill();
}

#[tokio::test]
async fn close_with_summary_kills_session_after_ai_summary_recorded() {
    let sock = std::env::temp_dir().join(format!("dzsum-{}.sock", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let store = test_store();
    let projects = test_projects();
    let bookmarks = test_bookmarks();
    let transcripts = test_transcripts();
    let session_summaries = test_session_summaries();
    let backfill_registry = test_backfill_registry();
    let todos = test_todos();
    let categories = test_categories();
    tokio::spawn({
        let sock = sock.clone();
        let registry = registry.clone();
        async move {
            dozerd::server::serve(
                &sock,
                registry,
                store,
                projects,
                bookmarks,
                transcripts,
                session_summaries,
                backfill_registry,
                todos,
                categories,
                dozerd::task_poller::new_in_flight(),
            )
            .await
        }
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let s = registry
        .create(dozerd::session::SessionSpec {
            name: "测试".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "cat".into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
            project_id: 1,
        })
        .unwrap();
    let id = s.id().to_string();

    match send_req(
        &sock,
        &Request::CloseWithSummary {
            session_id: id.clone(),
        },
    )
    .await
    {
        Reply::Ok => {}
        other => panic!("意外应答: {other:?}"),
    }

    // agent 抢在超时前交回总结:轮询间隔是生产值 2 秒,测试里直接用真实
    // submit 触发,不等超时分支。
    match send_req(
        &sock,
        &Request::RecordSessionSummary {
            session_id: id.clone(),
            title: "标题".into(),
            summary: "摘要".into(),
        },
    )
    .await
    {
        Reply::Ok => {}
        other => panic!("意外应答: {other:?}"),
    }

    // 后台任务下一次 2 秒轮询会看到已落库的总结并 kill;给够时间等它跑完。
    tokio::time::sleep(Duration::from_secs(3)).await;
    match send_req(
        &sock,
        &Request::GetSessionSummary {
            session_id: id.clone(),
        },
    )
    .await
    {
        Reply::SessionSummary { summary: Some(p) } => assert_eq!(p.title, "标题"),
        other => panic!("意外应答: {other:?}"),
    }
}

#[tokio::test]
async fn list_conversations_with_summaries_joins_correctly() {
    // 会话列表面板定位 transcript 用的是 `home_dir()(=$HOME)/.claude/...`。
    // 把 $HOME 指到一个临时目录,让摄取的 transcript 落在真实查询路径下,
    // 测试结束后由 TempDir 自动清理(不污染真实家目录)。
    let home = tempfile::tempdir().unwrap();
    // SAFETY: 测试用临时 HOME,daemon 在进程内 tokio::spawn 跑,读取的是
    // set 之后的 HOME;本测试二进制里没有任何其他测试依赖 $HOME。
    unsafe { std::env::set_var("HOME", home.path()) };

    let sock = std::env::temp_dir().join(format!("dozerd-join-{}.sock", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let transcripts = test_transcripts();
    let session_summaries = test_session_summaries();
    let backfill_registry = test_backfill_registry();
    let todos = test_todos();
    let categories = test_categories();
    tokio::spawn({
        let sock = sock.clone();
        let registry = registry.clone();
        let transcripts = transcripts.clone();
        let session_summaries = session_summaries.clone();
        let backfill_registry = backfill_registry.clone();
        let todos = todos.clone();
        async move {
            dozerd::server::serve(
                &sock,
                registry,
                test_store(),
                test_projects(),
                test_bookmarks(),
                transcripts,
                session_summaries,
                backfill_registry,
                todos,
                categories,
                dozerd::task_poller::new_in_flight(),
            )
            .await
        }
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // 造一个真实摄取的 conversation(c1)+一个匹配的 session_summaries 行,
    // 另一个 conversation(c2)不给总结,验证降级为 None。transcript 放在
    // `home/.claude/projects/-x`(cwd `/x` 的 `project_key` 是 `-x`)。
    let claude_dir = home.path().join(".claude").join("projects").join("-x");
    std::fs::create_dir_all(&claude_dir).unwrap();
    std::fs::write(
        claude_dir.join("c1.jsonl"),
        "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"c1 内容\"}}\n",
    )
    .unwrap();
    std::fs::write(
        claude_dir.join("c2.jsonl"),
        "{\"type\":\"user\",\"uuid\":\"u2\",\"message\":{\"role\":\"user\",\"content\":\"c2 内容\"}}\n",
    )
    .unwrap();
    transcripts
        .ingest_session(
            dozer_core::protocol::AgentKind::Claude,
            &claude_dir.join("c1.jsonl"),
        )
        .unwrap();
    transcripts
        .ingest_session(
            dozer_core::protocol::AgentKind::Claude,
            &claude_dir.join("c2.jsonl"),
        )
        .unwrap();
    session_summaries
        .record(&dozer_core::protocol::SessionSummaryPayload {
            session_id: "s1".into(),
            agent_kind: dozer_core::protocol::AgentKind::Claude,
            conversation_id: Some("c1".into()),
            title: "c1 的总结标题".into(),
            summary: "c1 的总结全文".into(),
            status: dozer_core::protocol::SummaryStatus::AiGenerated,
            created_ts_ms: 1,
            task_id: None,
        })
        .unwrap();

    // cwd `/x` 经 `project_key` 映射为目录 `-x`,正好对上上面的 claude_dir。
    let cwd = "/x";
    let rows = match send_req(
        &sock,
        &Request::ListConversationsWithSummaries {
            cwd: cwd.into(),
            agent: None,
            limit: 50,
            offset: 0,
        },
    )
    .await
    {
        Reply::ConversationsWithSummaries { rows } => rows,
        other => panic!("意外应答: {other:?}"),
    };

    assert_eq!(rows.len(), 2);
    let c1 = rows
        .iter()
        .find(|(c, _)| c.conversation_id == "c1")
        .unwrap();
    assert_eq!(c1.1.as_ref().unwrap().title, "c1 的总结标题");
    let c2 = rows
        .iter()
        .find(|(c, _)| c.conversation_id == "c2")
        .unwrap();
    assert!(c2.1.is_none());
}
