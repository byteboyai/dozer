//! 应用宿主请求经 UDS 到达 dozerd 的 `AppService`:走真实的 socket 协议与 `dozer-client`。

use bytehost_apps::gateway::GatewayConfig;
use bytehost_apps::id::AppId;
use bytehost_apps::plan::{Approval, Provenance, TrustLevel};
use bytehost_apps::proto::{AppErrorKind, AppFailure, AppSource};
use bytehost_apps::registry::UninstallMode;
use dozer_client::Client;
use dozerd::app_service::AppService;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

struct CleanupGuard(PathBuf);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct TestDaemon {
    client: Client,
    socket: PathBuf,
    _cleanup: CleanupGuard,
    _task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

async fn start_daemon(apps: Arc<AppService>) -> TestDaemon {
    let sock = PathBuf::from(format!("/tmp/dz-apps-{}.sock", uuid::Uuid::new_v4()));
    let db = PathBuf::from(format!("/tmp/dz-apps-{}.db", uuid::Uuid::new_v4()));
    let stores = dozerd::server::Stores {
        registry: Arc::new(dozerd::registry::SessionRegistry::new()),
        projects: Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap()),
        bookmarks: Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap()),
        code_health: Arc::new(dozerd::code_health::CodeHealthStore::new(&db).unwrap()),
        transcripts: Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap()),
        session_summaries: Arc::new(
            dozerd::session_summary::SessionSummaryStore::open(&db).unwrap(),
        ),
        summary_jobs: Arc::new(dozerd::summary_jobs::SummaryJobStore::open(&db).unwrap()),
        backfill_registry: Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new()),
        todos: Arc::new(dozerd::todo::TodoStore::new(&db).unwrap()),
        categories: Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap()),
        memories: Arc::new(dozerd::memory::MemoryStore::new(&db).unwrap()),
        file_edit_history: Arc::new(
            dozerd::file_edit_history::FileEditHistoryStore::new(&db).unwrap(),
        ),
        groups: dozerd::group_service::GroupService::for_tests(),
        apps,
    };
    let ide_lock_dir = tempfile::tempdir().expect("ide_lock_dir tempdir");
    let s = sock.clone();
    let task = tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            ide_lock_dir.path().to_path_buf(),
            stores,
            dozerd::task_poller::new_in_flight(),
        )
        .await
    });
    for _ in 0..100 {
        if tokio::net::UnixStream::connect(&sock).await.is_ok() {
            return TestDaemon {
                client: Client::new(sock.clone()),
                socket: sock.clone(),
                _cleanup: CleanupGuard(sock),
                _task: task,
            };
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("等待测试 dozerd 监听超时");
}

fn write_app(dir: &Path, id: &str, body: &str) -> AppSource {
    std::fs::create_dir_all(dir.join("web")).unwrap();
    std::fs::write(
        dir.join("manifest.toml"),
        format!(
            "schema_version = 1\nmin_host_version = \"0.1.0\"\nid = \"{id}\"\nname = \"{id} app\"\nversion = \"1.0.0\"\n\n\
             [presentation]\nentrypoint = \"main\"\n\n[entrypoints.main]\ntype = \"web\"\npath = \"/\"\n\n\
             [runtime]\nkind = \"static_web\"\nsource = \"web/\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("web/index.html"), body).unwrap();
    AppSource::LocalDir {
        path: dir.to_path_buf(),
    }
}

fn fetch(launch_url: &str, id: &str) -> (u16, String) {
    let rest = launch_url.strip_prefix("http://").unwrap();
    let (authority, query) = rest.split_once('/').unwrap();
    let port: u16 = authority.rsplit_once(':').unwrap().1.parse().unwrap();
    let token = query.split("bh_token=").nth(1).unwrap();
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(
        s,
        "GET / HTTP/1.1\r\nHost: {id}.localhost:{port}\r\nCookie: bh_session={token}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    let status = raw.split_whitespace().nth(1).unwrap().parse().unwrap();
    (
        status,
        raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string(),
    )
}

fn id(s: &str) -> AppId {
    AppId::new(s).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_whole_install_run_stop_uninstall_cycle_works_over_the_socket() {
    let tmp = tempfile::tempdir().unwrap();
    let apps =
        AppService::start_with(&tmp.path().join("bytehost"), GatewayConfig { port: 0 }).await;
    let d = start_daemon(apps).await;
    let c = &d.client;
    let source = write_app(&tmp.path().join("src/a"), "excalidraw", "<h1>hello</h1>");

    let plan = c
        .app_plan(source.clone(), Provenance::Local, TrustLevel::Trusted)
        .await
        .unwrap();
    assert_eq!(plan.app_id, id("excalidraw"));
    assert_eq!(plan.runtime_kind, "static_web");
    assert!(c.app_list().await.unwrap().is_empty(), "出计划不安装");
    c.app_install(
        plan.approve(Approval {
            approver: "test".into(),
            approved_ms: 1,
        }),
        source,
    )
    .await
    .unwrap();
    let listed = c.app_list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "excalidraw app");
    assert!(listed[0].url.is_none());

    let url = c.app_start(id("excalidraw")).await.unwrap();
    assert!(
        url.starts_with("http://excalidraw.localhost:") && !url.contains("bh_token"),
        "{url}"
    );
    let launch = c.app_launch_url(id("excalidraw")).await.unwrap();
    assert_eq!(
        fetch(&launch, "excalidraw"),
        (200, "<h1>hello</h1>".to_string())
    );

    c.app_stop(id("excalidraw")).await.unwrap();
    assert!(
        c.app_launch_url(id("excalidraw")).await.is_err(),
        "停止后不再发带令牌的地址"
    );
    c.app_uninstall(id("excalidraw"), UninstallMode::ProgramAndData)
        .await
        .unwrap();
    assert!(c.app_list().await.unwrap().is_empty());

    let err = c.app_start(id("excalidraw")).await.unwrap_err();
    let failure = err.downcast_ref::<AppFailure>().expect("带类别的失败");
    assert_eq!(failure.kind, AppErrorKind::NotFound);
    assert!(failure.message.contains("没有安装"), "{failure}");
    let runtimes = c.app_probe_runtimes().await.unwrap();
    assert_eq!(runtimes.len(), 3);
}

/// 应用宿主不可用(例如端口被占)不能影响会话等其他功能:其他请求照常工作,应用请求带着原因失败。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unavailable_app_host_does_not_break_the_rest_of_the_daemon() {
    let d = start_daemon(AppService::unavailable("gateway 端口 12345 已被占用")).await;
    assert!(d.client.list().await.unwrap().is_empty(), "会话列表照常");
    let err = d.client.app_list().await.unwrap_err();
    let failure = err.downcast_ref::<AppFailure>().expect("带类别的失败");
    assert_eq!(failure.kind, AppErrorKind::Unavailable);
    assert!(
        failure.message.contains("12345") && failure.message.contains("占用"),
        "{failure}"
    );
}

/// `Shutdown` 请求:应用跟随 dozerd 停止(gateway 关闭、站点撤下),但用户想要运行的意愿(desired)保留。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shutdown_request_takes_the_apps_down_but_keeps_what_the_user_wanted() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("bytehost");
    let apps = AppService::start_with(&root, GatewayConfig { port: 0 }).await;
    let d = start_daemon(apps.clone()).await;
    let c = &d.client;
    let source = write_app(&tmp.path().join("src/a"), "excalidraw", "hi");
    let plan = c
        .app_plan(source.clone(), Provenance::Local, TrustLevel::Trusted)
        .await
        .unwrap();
    c.app_install(
        plan.approve(Approval {
            approver: "t".into(),
            approved_ms: 1,
        }),
        source,
    )
    .await
    .unwrap();
    let url = c.app_start(id("excalidraw")).await.unwrap();
    let port: u16 = url
        .split(':')
        .nth(2)
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());

    c.shutdown_daemon().await.unwrap();

    assert!(apps.gateway_stopped(), "dozerd 停了,gateway 也停了");
    let state: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("apps/excalidraw/state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        state["desired"], "running",
        "用户想要运行的意愿保留,下次启动自动恢复"
    );
    assert_eq!(state["observed"]["state"], "stopped");
}

/// 订阅的实际形状:发送 `Subscribe` 后先收到 `Subscribed`;随后同一连接上 —— 装一个应用 ——
/// 收到 `Changed{app}`;且**同一连接**还能继续发普通 `List` 请求并得到应答(订阅不独占读侧)。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribing_pushes_app_changes_and_does_not_hold_the_read_side() {
    use bytehost_apps::proto::{AppReply, AppRequest};
    use dozer_core::protocol::{Reply, Request, decode_line, encode_line};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let tmp = tempfile::tempdir().unwrap();
    let apps =
        AppService::start_with(&tmp.path().join("bytehost"), GatewayConfig { port: 0 }).await;
    let d = start_daemon(apps).await;

    let stream = tokio::net::UnixStream::connect(&d.socket).await.unwrap();
    let (r, mut w) = stream.into_split();
    let mut lines = BufReader::new(r).lines();

    w.write_all(
        encode_line(&Request::App {
            request: AppRequest::Subscribe,
        })
        .as_bytes(),
    )
    .await
    .unwrap();
    let first = lines.next_line().await.unwrap().expect("先有应答");
    assert_eq!(
        decode_line::<Reply>(&first).unwrap(),
        Reply::App {
            reply: AppReply::Subscribed
        }
    );

    // 另一个连接安装一个静态应用:订阅连接应收到 Changed{excalidraw}。
    let source = write_app(&tmp.path().join("src/a"), "excalidraw", "hi");
    let plan = d
        .client
        .app_plan(source.clone(), Provenance::Local, TrustLevel::Trusted)
        .await
        .unwrap();
    d.client
        .app_install(
            plan.approve(Approval {
                approver: "t".into(),
                approved_ms: 1,
            }),
            source,
        )
        .await
        .unwrap();

    let changed = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
        .await
        .expect("5s 内应收到推送")
        .unwrap()
        .expect("连接仍开着");
    assert_eq!(
        decode_line::<Reply>(&changed).unwrap(),
        Reply::App {
            reply: AppReply::Changed {
                app: id("excalidraw")
            }
        }
    );

    // 同一连接继续发普通 List 请求,仍得到应答(订阅不独占读侧)。
    w.write_all(
        encode_line(&Request::App {
            request: AppRequest::List,
        })
        .as_bytes(),
    )
    .await
    .unwrap();
    let listed = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
        .await
        .expect("List 应有应答")
        .unwrap()
        .expect("连接仍开着");
    match decode_line::<Reply>(&listed).unwrap() {
        Reply::App {
            reply: AppReply::Apps { apps },
        } => assert_eq!(apps.len(), 1),
        other => panic!("{other:?}"),
    }
}

/// 应用宿主不可用时订阅必须立即失败(不可用),连接不挂。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribing_to_an_unavailable_host_fails_without_hanging() {
    use bytehost_apps::proto::{AppErrorKind, AppReply, AppRequest};
    use dozer_core::protocol::{Reply, Request, decode_line, encode_line};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let d = start_daemon(AppService::unavailable("x")).await;
    let stream = tokio::net::UnixStream::connect(&d.socket).await.unwrap();
    let (r, mut w) = stream.into_split();
    let mut lines = BufReader::new(r).lines();
    w.write_all(
        encode_line(&Request::App {
            request: AppRequest::Subscribe,
        })
        .as_bytes(),
    )
    .await
    .unwrap();
    let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
        .await
        .expect("应答不应挂起")
        .unwrap()
        .expect("连接仍开着");
    match decode_line::<Reply>(&line).unwrap() {
        Reply::App {
            reply: AppReply::Failed { failure },
        } => assert_eq!(failure.kind, AppErrorKind::Unavailable),
        other => panic!("{other:?}"),
    }
}
