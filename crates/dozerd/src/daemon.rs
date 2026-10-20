//! S6a Task 5:dozerd 的产品化装配。原 main.rs 里的全部启动逻辑搬进
//! `run(DaemonConfig)`:产品身份(数据目录/socket/数据库)、后台功能开关
//! 全由配置决定;dozerd 二进制只是 `run(DaemonConfig::for_product(dozer))`
//! 的薄入口,Digger 的 diggerd 传自己的产品身份与 `DaemonFeatures::minimal()`。

use crate::server::{self, Stores};
use anyhow::Result;
use dozer_core::paths::{self, ProductId};
use std::path::PathBuf;
use std::sync::Arc;

dozer_core::scope!(LOG, module, "daemon");

/// 后台功能开关。默认全开(Dozer 现状);`minimal()` 全关(Digger 用:
/// 应用宿主在 Digger 进程内跑、transcript 摄取/总结/task_poller/IDE
/// 桥接都不要——两个守护进程同机时 IDE 桥接还会往 ~/.claude/ide/ 写
/// 全局锁文件,让 Claude Code 看到两个"IDE",必须只留 Dozer 一家)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaemonFeatures {
    pub app_host: bool,
    pub transcript_backfill: bool,
    pub summaries: bool,
    pub task_poller: bool,
    pub ide_bridge: bool,
}

impl Default for DaemonFeatures {
    fn default() -> Self {
        Self {
            app_host: true,
            transcript_backfill: true,
            summaries: true,
            task_poller: true,
            ide_bridge: true,
        }
    }
}

impl DaemonFeatures {
    /// 全关(Digger 的 diggerd 用)。
    pub fn minimal() -> Self {
        Self {
            app_host: false,
            transcript_backfill: false,
            summaries: false,
            task_poller: false,
            ide_bridge: false,
        }
    }
}

/// 守护进程启动配置:产品身份 + socket 覆盖 + 功能开关。
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub product: ProductId,
    pub socket: PathBuf,
    pub features: DaemonFeatures,
}

impl DaemonConfig {
    /// 某产品的默认配置:socket 用该产品的 socket_path,功能全开。
    pub fn for_product(product: ProductId) -> Self {
        let socket = product.socket_path();
        Self {
            product,
            socket,
            features: DaemonFeatures::default(),
        }
    }
}

/// 校验功能组合的合法性(复审 R4):task_poller 依赖 summaries 的
/// 消费服务,开着 poller 关着 summaries 会让启动中途 panic。返回可读
/// 错误,由 run 在任何有副作用的启动工作之前(单实例检查之前)调用。
fn validate_features(features: DaemonFeatures) -> Result<()> {
    if features.task_poller && !features.summaries {
        anyhow::bail!(
            "非法的功能组合:task_poller 需要 summaries(轮询器派发的任务由总结 worker 消费)。\
             请把 summaries 一并打开,或把 task_poller 关掉"
        );
    }
    Ok(())
}

pub async fn run(config: DaemonConfig) -> Result<()> {
    // 第一件事:安装进程级产品身份。之后所有 paths::* 自由函数
    // (数据库路径、日志目录、socket 兜底)读到的都是该产品。
    paths::install(config.product.clone())?;

    // 复审 R4:非法组合在任何有副作用的启动工作之前校验,返回可读
    // Err 而不是让启动中途 panic(此前 task_poller && !summaries 会走到
    // .expect 直接炸)。DaemonFeatures 字段全是 pub,调用方可以合法构造
    // 出这个组合,必须在入口拦住。
    validate_features(config.features)?;

    let features = config.features;
    let socket = &config.socket;

    // 单实例检查必须在任何有副作用的启动工作之前(与原 main 相同:
    // 误启动的第二个守护进程不能先动手再发现自己不该启动)。
    server::ensure_single_instance(socket).await?;

    let _guard = dozer_core::log::init(
        dozer_core::log::Component::Daemon,
        env!("CARGO_PKG_VERSION"),
    );

    let registry = Arc::new(crate::registry::SessionRegistry::new());
    let db = config.product.db_path();
    let projects = Arc::new(crate::projects::ProjectStore::new(&db)?);
    let bookmarks = Arc::new(crate::bookmarks::BookmarkStore::new(&db)?);
    let code_health = Arc::new(crate::code_health::CodeHealthStore::new(&db)?);
    let transcripts = Arc::new(crate::transcripts::TranscriptStore::open(&db)?);
    let session_summaries = Arc::new(crate::session_summary::SessionSummaryStore::open(&db)?);
    let summary_jobs = Arc::new(crate::summary_jobs::SummaryJobStore::open(&db)?);
    let backfill_registry = Arc::new(crate::session_summary_backfill::BackfillRegistry::new());
    let todos = Arc::new(crate::todo::TodoStore::new(&db)?);
    let categories = Arc::new(crate::todo_category::CategoryStore::new(&db)?);
    let memories = Arc::new(crate::memory::MemoryStore::new(&db)?);
    let file_edit_history = Arc::new(crate::file_edit_history::FileEditHistoryStore::new(&db)?);
    let locale = Arc::new(crate::locale_settings::LocaleSettingsStore::open(
        &crate::locale_settings::LocaleSettingsStore::default_path(),
    )?);
    let groups = {
        let store = Arc::new(crate::group_store::GroupStore::new(&db)?);
        let projects_for_dir = projects.clone();
        let svc = crate::group_service::GroupService::with_headless_runner(
            store,
            Arc::new(move |id| {
                projects_for_dir
                    .path_of(id)
                    .ok()
                    .flatten()
                    .map(std::path::PathBuf::from)
            }),
        );
        svc.recover_on_startup();
        svc
    };
    let apps = if features.app_host {
        bytehost_apps::service::AppService::start(&paths::state_dir().join("bytehost")).await
    } else {
        bytehost_apps::service::AppService::unavailable("app host runs in-process in this product")
    };
    let in_flight = crate::task_poller::new_in_flight();
    if features.transcript_backfill {
        let files = crate::transcripts::scan::discover_all_transcript_files();
        dozer_core::log_info!(
            LOG,
            count = files.len(),
            "启动回填:发现历史 transcript 文件"
        );
        crate::backfill::backfill_all(&transcripts, files);
        match transcripts.ingest_codex_sqlite(None) {
            Ok(count) => dozer_core::log_info!(LOG, count, "启动回填:已摄取 Codex SQLite 会话"),
            Err(e) => dozer_core::log_warn!(LOG, error = %e, "Codex SQLite 启动回填失败"),
        }
    }
    let (summary_service, summary_worker, summary_shutdown) = if features.summaries {
        let summary_service = Arc::new(crate::summary_service::SummaryService {
            jobs: summary_jobs.clone(),
            transcripts: transcripts.clone(),
            session_summaries: session_summaries.clone(),
            scratch_root: paths::state_dir().join("summary-scratch"),
        });
        if let Err(e) = summary_service.recover_on_startup() {
            dozer_core::log_warn!(LOG, error = %e, "总结任务重启恢复失败");
        }
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker = {
            let service = summary_service.clone();
            let flag = shutdown.clone();
            let scratch_root = paths::state_dir().join("summary-scratch");
            tokio::spawn(async move {
                service
                    .run_worker(flag, move |agent, config| {
                        Box::new(crate::summary_pipeline::ProviderSummarizer {
                            agent,
                            config,
                            cwd: scratch_root.clone(),
                        })
                    })
                    .await;
            })
        };
        (Some(summary_service), Some(worker), Some(shutdown))
    } else {
        (None, None, None)
    };
    if features.task_poller {
        // validate_features 已保证 summaries == true;unwrap 只是让类型
        // 收窄,不再 panic。
        crate::task_poller::spawn(
            todos.clone(),
            categories.clone(),
            session_summaries.clone(),
            transcripts.clone(),
            projects.clone(),
            in_flight.clone(),
            summary_service
                .clone()
                .expect("validate_features 已校验:task_poller 开启时 summaries 必须为 true"),
        );
    }
    let serve = server::serve(
        socket,
        crate::ide_bridge::lock_dir(),
        Stores {
            registry,
            projects,
            bookmarks,
            code_health,
            transcripts,
            session_summaries,
            summary_jobs,
            backfill_registry,
            todos,
            categories,
            memories,
            file_edit_history,
            groups,
            apps: apps.clone(),
            locale,
        },
        in_flight,
        features.ide_bridge,
    );
    tokio::select! {
        r = serve => {
            if let Some(flag) = &summary_shutdown {
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            r?
        }
        _ = shutdown_signal() => {
            apps.shutdown().await;
            if let Some(flag) = &summary_shutdown {
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            let _ = std::fs::remove_file(socket);
        }
    }
    if let Some(worker) = summary_worker {
        let _ = worker.await;
    }
    Ok(())
}

/// 等到 Ctrl-C(SIGINT)或 SIGTERM(与原 main 相同)。
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => tokio::select! {
                _ = tokio::signal::ctrl_c() => dozer_core::log_info!(LOG, "收到 Ctrl-C，退出"),
                _ = term.recv() => dozer_core::log_info!(LOG, "收到 SIGTERM，退出"),
            },
            Err(e) => {
                dozer_core::log_warn!(LOG, "无法监听 SIGTERM: {e}");
                let _ = tokio::signal::ctrl_c().await;
                dozer_core::log_info!(LOG, "收到 Ctrl-C，退出");
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_core::paths::ProductDirs;

    #[test]
    fn default_features_enable_everything() {
        let f = DaemonFeatures::default();
        assert!(
            f.app_host && f.transcript_backfill && f.summaries && f.task_poller && f.ide_bridge
        );
    }

    #[test]
    fn minimal_features_disable_everything() {
        let f = DaemonFeatures::minimal();
        assert!(
            !f.app_host
                && !f.transcript_backfill
                && !f.summaries
                && !f.task_poller
                && !f.ide_bridge
        );
    }

    /// 复审 R4:非法组合返回可读错误,不 panic。
    #[test]
    fn task_poller_without_summaries_is_rejected_with_a_readable_error() {
        let mut features = DaemonFeatures::minimal();
        features.task_poller = true;
        let err = validate_features(features).unwrap_err().to_string();
        assert!(err.contains("task_poller"), "{err}");
        assert!(err.contains("summaries"), "{err}");
        // 合法组合不报错
        assert!(validate_features(DaemonFeatures::default()).is_ok());
        assert!(validate_features(DaemonFeatures::minimal()).is_ok());
    }

    #[test]
    fn for_product_uses_the_products_socket_and_db() {
        let root = tempfile::tempdir().unwrap();
        let product = ProductId::custom(
            "digger",
            "DIGGER_SESSION_ID",
            "digger-hook",
            "diggerd.sock",
            "digger.db",
            ProductDirs::Fixed {
                config: root.path().join("config"),
                state: root.path().join("state"),
            },
        );
        let cfg = DaemonConfig::for_product(product.clone());
        assert_eq!(cfg.socket, root.path().join("state").join("diggerd.sock"));
        assert_eq!(
            cfg.product.db_path(),
            root.path().join("state").join("digger.db")
        );
        assert_eq!(cfg.features, DaemonFeatures::default());
    }
}
