//! 语言设置的端到端契约(spec 2026-10-12-dozer-i18n-design §4.2):
//! 经 UDS 的 get/set、CAS 冲突、重启后持久化。

use dozer_client::Client;
use dozer_core::protocol::{Locale, LocalePreference, LocaleSnapshot, LocaleWriteOutcome};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn temp_sock() -> PathBuf {
    PathBuf::from(format!("/tmp/dz-locale-{}.sock", uuid::Uuid::new_v4()))
}

struct CleanupGuard(PathBuf);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct TestDaemon {
    socket: PathBuf,
    _cleanup: CleanupGuard,
    _task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

async fn start_daemon(settings_path: PathBuf) -> TestDaemon {
    let sock = temp_sock();
    let db = PathBuf::from(format!("/tmp/dz-locale-{}.db", uuid::Uuid::new_v4()));
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
        apps: bytehost_apps::service::AppService::unavailable("test"),
        locale: Arc::new(
            dozerd::locale_settings::LocaleSettingsStore::open(&settings_path).unwrap(),
        ),
    };
    let ide_lock_dir = tempfile::tempdir().expect("ide_lock_dir tempdir");
    let s = sock.clone();
    let task = tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            ide_lock_dir.path().to_path_buf(),
            stores,
            dozerd::task_poller::new_in_flight(),
            true,
        )
        .await
    });
    for _ in 0..100 {
        if tokio::net::UnixStream::connect(&sock).await.is_ok() {
            return TestDaemon {
                socket: sock.clone(),
                _cleanup: CleanupGuard(sock),
                _task: task,
            };
        }
        if task.is_finished() {
            let result = task.await.expect("测试 dozerd task panic");
            panic!("测试 dozerd 在监听前退出: {result:?}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("等待测试 dozerd 监听超时");
}

#[tokio::test]
async fn get_returns_default_then_set_and_get_roundtrips() {
    let dir = tempfile::tempdir().unwrap();
    let settings = dir.path().join("locale_settings.json");
    let d = start_daemon(settings).await;
    let client = Client::new(d.socket.clone());

    let initial = client.get_locale().await.unwrap();
    assert_eq!(initial, LocaleSnapshot::default_snapshot());

    let outcome = client
        .set_locale(
            LocalePreference::Explicit {
                locale: Locale("zh-CN".into()),
            },
            0,
            vec![],
        )
        .await
        .unwrap();
    match outcome {
        LocaleWriteOutcome::Applied { snapshot } => {
            assert_eq!(snapshot.revision, 1);
            assert_eq!(snapshot.active_locale.as_str(), "zh-CN");
        }
        other => panic!("unexpected: {other:?}"),
    }
    let now = client.get_locale().await.unwrap();
    assert_eq!(now.revision, 1);
    assert_eq!(now.active_locale.as_str(), "zh-CN");
}

#[tokio::test]
async fn stale_revision_conflicts_with_latest_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let settings = dir.path().join("locale_settings.json");
    let d = start_daemon(settings).await;
    let client = Client::new(d.socket.clone());

    let _ = client
        .set_locale(
            LocalePreference::Explicit {
                locale: Locale("zh-CN".into()),
            },
            0,
            vec![],
        )
        .await
        .unwrap();
    let conflict = client
        .set_locale(
            LocalePreference::Explicit {
                locale: Locale("en".into()),
            },
            0,
            vec![],
        )
        .await
        .unwrap();
    match conflict {
        LocaleWriteOutcome::Conflict { snapshot } => {
            assert_eq!(snapshot.revision, 1);
            assert_eq!(snapshot.active_locale.as_str(), "zh-CN");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn setting_survives_daemon_restart() {
    let dir = tempfile::tempdir().unwrap();
    let settings = dir.path().join("locale_settings.json");
    {
        let d = start_daemon(settings.clone()).await;
        let client = Client::new(d.socket.clone());
        let _ = client
            .set_locale(
                LocalePreference::Explicit {
                    locale: Locale("zh-CN".into()),
                },
                0,
                vec![],
            )
            .await
            .unwrap();
    }
    // 新 daemon 以同一配置文件启动,应读到 revision 1 + zh-CN。
    let d2 = start_daemon(settings).await;
    let client2 = Client::new(d2.socket.clone());
    let s = client2.get_locale().await.unwrap();
    assert_eq!(s.revision, 1);
    assert_eq!(s.active_locale.as_str(), "zh-CN");
}

#[tokio::test]
async fn system_preference_is_resolved_from_candidates() {
    let dir = tempfile::tempdir().unwrap();
    let settings = dir.path().join("locale_settings.json");
    let d = start_daemon(settings).await;
    let client = Client::new(d.socket.clone());

    let outcome = client
        .set_locale(
            LocalePreference::System,
            0,
            vec!["zh-HK".into(), "en-US".into()],
        )
        .await
        .unwrap();
    match outcome {
        LocaleWriteOutcome::Applied { snapshot } => {
            assert_eq!(snapshot.preference, LocalePreference::System);
            // zh-HK 不映射简体,继续看 en-US → en。
            assert_eq!(snapshot.active_locale.as_str(), "en");
        }
        other => panic!("unexpected: {other:?}"),
    }
}
