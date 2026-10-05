use anyhow::Result;
use dozerd::registry::SessionRegistry;
use std::path::PathBuf;
use std::sync::Arc;

dozer_core::scope!(LOG, module, "server");

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
                    eprintln!("--socket 需要一个路径参数"); // cli-output
                    std::process::exit(2);
                }
            },
            "--version" => {
                println!("dozerd {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => {
                eprintln!("未知参数: {other}（支持 --socket <path> / --version）"); // cli-output
                std::process::exit(2);
            }
        }
    }

    // 单实例检查必须在**任何**有副作用的启动工作之前:`group_service.recover_on_startup()` 会把共享 dozer.db 里
    // "进行中"的群聊发言标成失败、应用宿主的对账会起站点写状态——误启动的第二个 dozerd 不能先动手、
    // 之后才发现自己不该启动(那会改坏正在运行的第一个 dozerd 的状态)
    dozerd::server::ensure_single_instance(&socket).await?;

    // 日志服务由 `dozer-core::log` 统一提供(按天滚动到 `logs_dir()`、14 天保留、
    // 启动横幅、panic 落盘);daemon 沿用此前的 stdout 输出。返回的 Guard 必须活到
    // 进程结束,否则落盘线程提前退出会静默丢日志。
    let _guard = dozer_core::log::init(
        dozer_core::log::Component::Daemon,
        env!("CARGO_PKG_VERSION"),
    );

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
    let file_edit_history = Arc::new(dozerd::file_edit_history::FileEditHistoryStore::new(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
    let groups = {
        let store = Arc::new(dozerd::group_store::GroupStore::new(
            &dozer_core::paths::state_dir().join("dozer.db"),
        )?);
        let projects_for_dir = projects.clone();
        let svc = dozerd::group_service::GroupService::with_headless_runner(
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
    // 应用宿主:启动永不失败(端口被占用等只会让它"不可用",不影响会话功能);启动时对账,退出时收尾
    let apps =
        dozerd::app_service::AppService::start(&dozer_core::paths::state_dir().join("bytehost"))
            .await;
    let in_flight = dozerd::task_poller::new_in_flight();
    {
        let files = dozerd::transcripts::scan::discover_all_transcript_files();
        dozer_core::log_info!(
            LOG,
            count = files.len(),
            "启动回填:发现历史 transcript 文件"
        );
        dozerd::backfill::backfill_all(&transcripts, files);
        match transcripts.ingest_codex_sqlite(None) {
            Ok(count) => dozer_core::log_info!(LOG, count, "启动回填:已摄取 Codex SQLite 会话"),
            Err(e) => dozer_core::log_warn!(LOG, error = %e, "Codex SQLite 启动回填失败"),
        }
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
        dozer_core::log_warn!(LOG, error = %e, "总结任务重启恢复失败");
    }
    dozerd::task_poller::spawn(
        todos.clone(),
        categories.clone(),
        session_summaries.clone(),
        transcripts.clone(),
        projects.clone(),
        in_flight.clone(),
        summary_service.clone(),
    );
    let summary_shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let summary_worker = {
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
        })
    };
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
            file_edit_history,
            groups,
            apps: apps.clone(),
        },
        in_flight,
    );
    tokio::select! {
        r = serve => {
            summary_shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
            r?
        }
        _ = tokio::signal::ctrl_c() => {
            dozer_core::log_info!(LOG, "收到 Ctrl-C，退出");
            apps.shutdown().await;
            summary_shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = std::fs::remove_file(&socket);
        }
    }
    let _ = summary_worker.await;
    Ok(())
}
