use dozer_client::Client;
use dozer_mcp::server::{DozerMcpServer, GetMemoryParams, NoParams, WriteMemoryParams};
use rmcp::handler::server::wrapper::Parameters;
use std::sync::Arc;
use std::time::Duration;

fn temp_sock() -> std::path::PathBuf {
    let id = uuid::Uuid::new_v4();
    std::path::PathBuf::from(format!("/tmp/dz-mcp-mem-{}.sock", &id.to_string()[..8]))
}

struct CleanupGuard(std::path::PathBuf);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn start_daemon() -> (std::path::PathBuf, CleanupGuard) {
    let sock = temp_sock();
    let db = std::path::PathBuf::from(format!("/tmp/dz-mcp-mem-{}.db", uuid::Uuid::new_v4()));
    let stores = dozerd::server::Stores {
        registry: Arc::new(dozerd::registry::SessionRegistry::new()),
        projects: Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap()),
        bookmarks: Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap()),
        code_health: Arc::new(dozerd::code_health::CodeHealthStore::new(&db).unwrap()),
        transcripts: Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap()),
        session_summaries: Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap()),
        backfill_registry: Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new()),
        todos: Arc::new(dozerd::todo::TodoStore::new(&db).unwrap()),
        categories: Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap()),
        memories: Arc::new(dozerd::memory::MemoryStore::new(&db).unwrap()),
    };
    let ide_lock_dir = tempfile::tempdir().expect("ide_lock_dir tempdir");
    let s = sock.clone();
    tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            ide_lock_dir.path().to_path_buf(),
            stores,
            dozerd::task_poller::new_in_flight(),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (sock.clone(), CleanupGuard(sock))
}

/// 建一个挂到 `project_id` 上的会话,返回 `session_id`——照抄
/// `crates/dozer-mcp/tests/todo_tools.rs::session_with_project` 的既有
/// 写法:`Client::create` 的 `project_id` 参数是裸 `i64`(不是
/// `Option<i64>`),不需要真的先建一个 `ProjectStore` 里的项目记录,
/// `SessionRegistry::create` 不校验这个 id 是否真实存在。
async fn session_with_project(sock: &std::path::Path, project_id: i64) -> String {
    let client = Client::new(sock.to_path_buf());
    let session = client
        .create("mem-test", "/bin/sh", &["-c".into(), "cat".into()], "/tmp", 80, 24, project_id)
        .await
        .expect("建会话");
    session.id
}

#[tokio::test]
async fn write_then_list_then_get_roundtrip() {
    let (sock, _guard) = start_daemon().await;
    let session_id = session_with_project(&sock, 1).await;
    let server = DozerMcpServer::new(Client::new(sock), session_id);

    let write_result = server
        .write_memory(Parameters(WriteMemoryParams {
            title: "用户偏好".into(),
            body: "喜欢简短回复".into(),
            kind: "user".into(),
            description: "沟通风格".into(),
        }))
        .await
        .expect("write_memory 应该成功");
    assert!(write_result.structured_content.is_some());

    let list_result = server
        .list_memories(Parameters(NoParams {}))
        .await
        .expect("list_memories 应该成功");
    let list_json = list_result.structured_content.expect("应有结构化结果");
    assert_eq!(list_json["memories"].as_array().unwrap().len(), 1);
    assert_eq!(list_json["memories"][0]["title"], "用户偏好");

    let get_by_title = server
        .get_memory(Parameters(GetMemoryParams {
            title_or_id: "用户偏好".into(),
        }))
        .await
        .expect("get_memory 按标题应该成功");
    let get_json = get_by_title.structured_content.expect("应有结构化结果");
    assert_eq!(get_json["body"], "喜欢简短回复");
}

#[tokio::test]
async fn write_memory_twice_same_title_updates_not_duplicates() {
    let (sock, _guard) = start_daemon().await;
    let session_id = session_with_project(&sock, 1).await;
    let server = DozerMcpServer::new(Client::new(sock), session_id);

    for body in ["v1", "v2"] {
        server
            .write_memory(Parameters(WriteMemoryParams {
                title: "同名记忆".into(),
                body: body.into(),
                kind: "project".into(),
                description: "".into(),
            }))
            .await
            .expect("write_memory 应该成功");
    }

    let list_result = server
        .list_memories(Parameters(NoParams {}))
        .await
        .expect("list_memories 应该成功");
    let list_json = list_result.structured_content.expect("应有结构化结果");
    assert_eq!(list_json["memories"].as_array().unwrap().len(), 1, "同名应该是更新不是新增");
}

#[tokio::test]
async fn tools_error_when_session_id_unresolvable() {
    // `Client::create` 的 `project_id` 是裸 `i64`,没有"建会话但不挂项目"
    // 这条路径可走(`SessionInfo.project_id: Option<i64>` 里的 `None` 只
    // 对应迁移期孤儿会话,见 `protocol.rs` 该字段文档,不是本 API 能构造
    // 的状态)。改用一个从未创建过的 `session_id`,命中
    // `resolve_project_and_agent` 里 `.find(|s| s.id == self.session_id)`
    // 返回 `None` 的分支,同样能验证"解析失败时三个工具都报错、不 panic"
    // 这条 Review Focus。
    let (sock, _guard) = start_daemon().await;
    let server = DozerMcpServer::new(Client::new(sock), "does-not-exist".into());

    let err = server
        .list_memories(Parameters(NoParams {}))
        .await
        .expect_err("session_id 不存在时 list_memories 应该报错");
    assert!(format!("{err}").contains("不存在于 dozerd"));

    let err = server
        .write_memory(Parameters(WriteMemoryParams {
            title: "x".into(),
            body: "y".into(),
            kind: "project".into(),
            description: "".into(),
        }))
        .await
        .expect_err("session_id 不存在时 write_memory 应该报错");
    assert!(format!("{err}").contains("不存在于 dozerd"));

    let err = server
        .get_memory(Parameters(GetMemoryParams {
            title_or_id: "x".into(),
        }))
        .await
        .expect_err("session_id 不存在时 get_memory 应该报错");
    assert!(format!("{err}").contains("不存在于 dozerd"));
}
