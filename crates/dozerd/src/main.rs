use anyhow::Result;
use dozerd::registry::SessionRegistry;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// 日志落盘:`dozerd` 由 `dozer-app`(`spawn_dozerd`)拉起时，子进程直接
/// 继承父进程的 stdio，而 `dozer-app` 本身多数时候是被 Finder/launchd
/// 拉起(不是终端 `cargo run`)——此前 `tracing::error!`/`warn!` 只写
/// stdout，没有文件落地，进程一退出这些日志就彻底没了，出问题以后没法
/// 事后回查(2026-08-29 用户反馈)。这里在原有 stdout 输出之外，按天滚动
/// 追加一份到 `dozer_core::paths::logs_dir()`。目录建不出来(权限等)时
/// 静默退化到只有 stdout，不影响 daemon 正常启动。
///
/// 返回的 `WorkerGuard` 必须被调用方一直持有到进程退出——它是
/// `tracing-appender` 后台落盘线程的存活凭证，提前 drop 会导致日志静默
/// 丢失（`main` 里绑到 `_guard`，寿命覆盖整个 `main` 函数体)。
///
/// 只按天切分，不做旧文件清理/大小上限——个人桌面场景，暂不值得为此引入
/// 额外的清理任务，需要控制体积时用户可以自己清 `logs_dir()`。
fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let env_filter =
        || tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let logs_dir = dozer_core::paths::logs_dir();
    let file_layer_and_guard = std::fs::create_dir_all(&logs_dir).ok().map(|()| {
        let appender = tracing_appender::rolling::daily(&logs_dir, "dozerd.log");
        let (non_blocking, guard) = tracing_appender::non_blocking(appender);
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(non_blocking)
            .with_ansi(false);
        (layer, guard)
    });
    let (file_layer, guard) = match file_layer_and_guard {
        Some((layer, guard)) => (Some(layer), Some(guard)),
        None => (None, None),
    };
    tracing_subscriber::registry()
        .with(env_filter())
        .with(tracing_subscriber::fmt::layer())
        .with(file_layer)
        .init();
    if guard.is_none() {
        tracing::warn!(?logs_dir, "创建日志目录失败，本次运行只输出到 stdout");
    }
    guard
}

#[tokio::main]
async fn main() -> Result<()> {
    // 极简参数解析：仅 --socket <path>（daemon 不引 clap，保持轻）。放在
    // 日志初始化之前——`--version` 是纯查询，不该有"顺带建了个日志目录"
    // 这种副作用。
    let mut args = std::env::args().skip(1);
    let mut socket: PathBuf = dozer_core::paths::socket_path();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--socket" => match args.next() {
                Some(p) => socket = PathBuf::from(p),
                None => {
                    eprintln!("--socket 需要一个路径参数");
                    std::process::exit(2);
                }
            },
            "--version" => {
                println!("dozerd {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => {
                eprintln!("未知参数: {other}（支持 --socket <path> / --version）");
                std::process::exit(2);
            }
        }
    }

    let _guard = init_logging();

    let registry = Arc::new(SessionRegistry::new());
    let projects = Arc::new(dozerd::projects::ProjectStore::new(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
    let bookmarks = Arc::new(dozerd::bookmarks::BookmarkStore::new(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
    let code_health = Arc::new(dozerd::code_health::CodeHealthStore::new(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
    let transcripts = Arc::new(dozerd::transcripts::TranscriptStore::open(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
    let session_summaries = Arc::new(dozerd::session_summary::SessionSummaryStore::open(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
    let summary_jobs = Arc::new(dozerd::summary_jobs::SummaryJobStore::open(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
    let backfill_registry = Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new());
    let todos = Arc::new(dozerd::todo::TodoStore::new(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
    let categories = Arc::new(dozerd::todo_category::CategoryStore::new(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
    let memories = Arc::new(dozerd::memory::MemoryStore::new(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
    let in_flight = dozerd::task_poller::new_in_flight();
    dozerd::task_poller::spawn(
        todos.clone(),
        categories.clone(),
        session_summaries.clone(),
        transcripts.clone(),
        projects.clone(),
        in_flight.clone(),
    );
    {
        let files = dozerd::transcripts::scan::discover_all_transcript_files();
        tracing::info!(count = files.len(), "启动回填:发现历史 transcript 文件");
        dozerd::backfill::backfill_all(&transcripts, files);
    }
    // 总结调度服务 + worker:全局并发 1,串行消费持久化队列;启动时先把上次
    // 进程中途退出遗留的 running 任务归位 queued。
    let summary_service = std::sync::Arc::new(dozerd::summary_service::SummaryService {
        jobs: summary_jobs.clone(),
        transcripts: transcripts.clone(),
        session_summaries: session_summaries.clone(),
        scratch_root: dozer_core::paths::state_dir().join("summary-scratch"),
    });
    if let Err(e) = summary_service.recover_on_startup() {
        tracing::warn!(error = %e, "总结任务重启恢复失败");
    }
    let summary_shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let service = summary_service.clone();
        let flag = summary_shutdown.clone();
        let scratch_root = dozer_core::paths::state_dir().join("summary-scratch");
        tokio::spawn(async move {
            service
                .run_worker(flag, move |agent, config| {
                    Box::new(dozerd::summary_pipeline::ProviderSummarizer {
                        agent,
                        config,
                        cwd: scratch_root.clone(),
                    })
                })
                .await;
        });
    }
    let serve = dozerd::server::serve(
        &socket,
        dozerd::ide_bridge::lock_dir(),
        dozerd::server::Stores {
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
        },
        in_flight,
    );
    tokio::select! {
        r = serve => {
            summary_shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
            r?
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("收到 Ctrl-C，退出");
            summary_shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = std::fs::remove_file(&socket);
        }
    }
    Ok(())
}
