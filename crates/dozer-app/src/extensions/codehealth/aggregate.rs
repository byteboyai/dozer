//! 异步扫描/加载：仿 `extensions::usage::spawn_refresh` 的写法——`handle.spawn`
//! 起一个 tokio 任务，CPU/IO 密集的本地扫描与 Git 统计丢 `spawn_blocking`，
//! dozerd 往返走 `client`，结果经 `emit` 回灌成 `Message`。
//!
//! 加载与扫描都产出完整 [`PanelState`]（当前报告 + 上一份 + 差异 + 热点 +
//! Git + 保存错误），差异与热点在 blocking 任务里算（spec Task 6）。

use super::{Message, PanelState};
use dozer_codehealth::{
    ArchitectureDiffOutcome, ImpactInput, ImpactScope, ProjectReport, architecture_diff,
    diff_reports, impact_scope,
};
use dozer_core::protocol::CodeHealthReportInfo;
use std::path::{Path, PathBuf};

/// 把报告反序列化（失败返回 `None`——旧报告解析失败不影响展示其它状态）。
fn parse_report(json: &str) -> Option<ProjectReport> {
    serde_json::from_str(json).ok()
}

/// 报告 → 落盘协议信息（含 Git 元数据；`scanned_at_ms` 由 daemon 覆盖）。
fn to_info(report: &ProjectReport) -> CodeHealthReportInfo {
    CodeHealthReportInfo {
        total_loc: report.total_loc as u64,
        total_functions: report.total_functions as u64,
        critical_functions: report.critical_functions as u64,
        overall_tier: format!("{:?}", report.overall_tier),
        report_json: serde_json::to_string(report).unwrap_or_default(),
        scanned_at_ms: 0,
        schema_version: report.schema_version,
        git_head: report.git.as_ref().and_then(|g| g.head.clone()),
        git_branch: report.git.as_ref().and_then(|g| g.branch.clone()),
        git_dirty: report.git.as_ref().map(|g| g.dirty).unwrap_or(false),
    }
}

/// 阻塞地补全面板状态：采集 Git 元数据 + 近 30 天 churn，计算差异与热点。
/// 调用方负责放进 `spawn_blocking`（含 `git log` 子进程）。
fn finish_panel(
    project_path: &Path,
    current: Option<(ProjectReport, Option<u64>)>,
    previous: Option<ProjectReport>,
    save_error: Option<String>,
) -> PanelState {
    let (report, scanned_at_ms) = match current {
        Some((r, ts)) => (Some(r), ts),
        None => (None, None),
    };

    let git = super::git_hotspots::git_snapshot(project_path);
    let churn = super::git_hotspots::recent_churn(project_path).unwrap_or_default();

    let (diff, hotspots) = match &report {
        Some(cur) => {
            let prev_findings: Vec<_> = previous
                .as_ref()
                .map(|p| p.findings.clone())
                .unwrap_or_default();
            let diff = diff_reports(&prev_findings, &cur.findings);
            let hotspots = super::rank_hotspots(&cur.findings, &prev_findings, &churn);
            (Some(diff), hotspots)
        }
        None => (None, Vec::new()),
    };

    // 架构差异与影响范围：与 Finding 差异独立。无上一份 schema v3 架构报告时
    // 结果显式 NoBaseline（UI 不把所有现存边渲染成新增）。影响种子 = Git dirty
    // 文件 + 本轮变化边的端点；深度取自项目配置，默认 3。
    let (architecture_diff, impact) = match &report {
        Some(cur) => {
            let prev_arch = previous.as_ref().map(|p| &p.architecture);
            let outcome = architecture_diff(prev_arch, &cur.architecture);

            let dirty = super::git_hotspots::dirty_paths(project_path);
            let changed_nodes: Vec<String> = match &outcome {
                ArchitectureDiffOutcome::Compared(d) => {
                    previous.as_ref().map_or_else(Vec::new, |previous| {
                        dozer_codehealth::changed_edge_endpoints(
                            &previous.architecture,
                            &cur.architecture,
                            d,
                        )
                    })
                }
                ArchitectureDiffOutcome::NoBaseline | ArchitectureDiffOutcome::Unavailable => {
                    Vec::new()
                }
            };

            let cfg = dozer_codehealth::load_project_config(project_path);
            let scope = impact_scope(ImpactInput {
                report: &cur.architecture,
                dirty_paths: &dirty,
                changed_nodes: &changed_nodes,
                depth: cfg.architecture.impact_depth,
                max_visited: MAX_IMPACT_VISITED,
            });
            (outcome, scope)
        }
        None => (ArchitectureDiffOutcome::NoBaseline, ImpactScope::default()),
    };

    PanelState {
        report,
        previous_report: previous,
        diff,
        architecture_diff,
        impact,
        hotspots,
        git,
        scanned_at_ms,
        save_error,
    }
}

/// 影响范围 BFS 的访问节点上限，防止异常大图拖慢 UI（spec「影响范围」）。
const MAX_IMPACT_VISITED: usize = 2000;

/// 面板打开时调用：只读落盘缓存（最新 + 上一份），不触发扫描（spec：手动触发）。
pub fn spawn_load_cached(
    project_id: i64,
    project_path: PathBuf,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let latest = client
            .get_code_health_report(project_id)
            .await
            .ok()
            .flatten();
        let history = client
            .list_code_health_reports(project_id, 2)
            .await
            .ok()
            .unwrap_or_default();

        let current = latest.and_then(|info| {
            let ts = info.scanned_at_ms;
            parse_report(&info.report_json).map(|r| (r, Some(ts)))
        });
        let previous = history
            .get(1)
            .and_then(|info| parse_report(&info.report_json));

        let panel = tokio::task::spawn_blocking(move || {
            finish_panel(&project_path, current, previous, None)
        })
        .await
        .unwrap_or_default();

        emit(Message::Loaded(project_id, Box::new(panel)));
    });
}

/// 面板切入:只读上次落盘的扫描结果,**不**自动扫描(spec:手动触发,与 Usage 的"打开即自动扫"
/// 是明确的行为差异)。没有项目路径时什么都不做。
pub fn on_activate(ctx: &crate::panel_host::ActivationCtx<Message>) {
    let Some(path) = ctx.project_path.clone() else {
        return;
    };
    spawn_load_cached(
        ctx.project_id,
        path,
        ctx.io.client(),
        ctx.io.handle(),
        ctx.io.emitter(),
    );
}

/// 点"扫描"按钮：本地跑 `dozer_codehealth::scan_project`（CPU/IO 密集），
/// 成功后采集 Git + 计算差异/热点，再异步落盘 dozerd，最后回灌 UI。
pub fn spawn_scan(
    project_id: i64,
    project_path: PathBuf,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        // 扫描前加载上一份报告（用于差异比较）。
        let history = client
            .list_code_health_reports(project_id, 2)
            .await
            .ok()
            .unwrap_or_default();
        // 新报告尚未保存，倒序历史的第 0 项就是本次扫描应比较的直接前驱。
        let previous = history
            .first()
            .and_then(|info| parse_report(&info.report_json));

        let scan_path = project_path.clone();
        let prev_for_scan = previous.clone();
        let scanned = tokio::task::spawn_blocking(move || {
            let mut report =
                dozer_codehealth::scan_project(&scan_path).map_err(|e| e.to_string())?;
            if report.scan.status == dozer_codehealth::ScanStatus::Failed {
                return Err("扫描失败：项目路径不可用".to_string());
            }
            report.git = super::git_hotspots::git_snapshot(&scan_path);
            let panel = finish_panel(&scan_path, Some((report, None)), prev_for_scan, None);
            Ok::<PanelState, String>(panel)
        })
        .await
        .unwrap_or_else(|e| Err(format!("扫描任务被取消: {e}")));

        match scanned {
            Ok(panel) => {
                let save_error = match &panel.report {
                    Some(report) => {
                        let info = to_info(report);
                        client
                            .save_code_health_report(project_id, &info)
                            .await
                            .err()
                            .map(|e| format!("未保存，无法用于下次比较：{e}"))
                    }
                    None => None,
                };
                let mut panel = panel;
                panel.scanned_at_ms = Some(now_ms());
                panel.save_error = save_error;
                emit(Message::Scanned(project_id, Ok(Box::new(panel))));
            }
            Err(e) => emit(Message::Scanned(project_id, Err(e))),
        }
    });
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
