//! 代码健康度面板的数据层与视图层（spec
//! docs/superpowers/specs/2026-09-21-code-health-evolution-design.md）。分层：
//! `aggregate.rs` 做异步扫描/加载/差异/热点，`mod.rs` 挂 `WorkspaceState`，
//! `view_model.rs` 产出已格式化文案，`view.rs` 只渲染，`git_hotspots.rs`
//! 做 Git 变动统计与热点排序。

mod aggregate;
pub(crate) use aggregate::*;

mod git_hotspots;
pub(crate) use git_hotspots::{HotspotView, rank_hotspots};

mod view;
pub(crate) use view::{content_pane, list_pane};

pub(crate) mod arch_payload;

pub(crate) mod view_model;

pub(crate) mod protocol;
pub(crate) use protocol::*;

mod surface;
pub(crate) use surface::CodeHealthSurface;

use dozer_codehealth::{
    ArchitectureDiffOutcome, GitSnapshot, ImpactScope, ProjectReport, ReportDiff,
};
use dozer_services::ServiceErrorKind;

dozer_core::scope!(pub(crate) LOG, panel, "code_health");

/// 面板右侧分类导航的四个分类：总览 / 结构复杂度 / UI 一致性 / 扫描范围。
/// 左侧内容区按当前选中分类切换展示（见 `view.rs`）。spec「信息架构与 UI」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CodeHealthCategory {
    #[default]
    Overview,
    Structure,
    UiConsistency,
    ScanScope,
    Architecture,
}

impl CodeHealthCategory {
    pub fn label(self) -> &'static str {
        match self {
            CodeHealthCategory::Overview => "总览",
            CodeHealthCategory::Structure => "结构复杂度",
            CodeHealthCategory::UiConsistency => "UI 一致性",
            CodeHealthCategory::ScanScope => "扫描范围",
            CodeHealthCategory::Architecture => "架构",
        }
    }

    pub fn all() -> [CodeHealthCategory; 5] {
        [
            CodeHealthCategory::Overview,
            CodeHealthCategory::Structure,
            CodeHealthCategory::UiConsistency,
            CodeHealthCategory::ScanScope,
            CodeHealthCategory::Architecture,
        ]
    }
}

/// 一次加载/扫描完成后灌给 `WorkspaceState` 的完整面板状态。
#[derive(Debug, Clone, Default)]
pub struct PanelState {
    pub report: Option<ProjectReport>,
    /// 上一份可解析快照（用于差异比较）。
    pub previous_report: Option<ProjectReport>,
    pub diff: Option<ReportDiff>,
    /// 架构节点/边/环的差异（与 Finding 差异独立）。`NoBaseline` 表示首次
    /// 扫描或旧快照无架构字段，UI 不得把所有现存边渲染成“本轮新增”。
    pub architecture_diff: ArchitectureDiffOutcome,
    /// 本轮变更影响范围（直接依赖方 + 间接影响），无种子时为空。
    pub impact: ImpactScope,
    pub hotspots: Vec<HotspotView>,
    pub git: Option<GitSnapshot>,
    pub scanned_at_ms: Option<u64>,
    /// 快照保存失败提示（本次结果仍可展示，但不能用于下次比较）。
    pub save_error: Option<String>,
}

#[derive(Default)]
pub struct WorkspaceState {
    pub(crate) requests: crate::request_tracker::RequestTracker,
    panel: PanelState,
    scanning: bool,
    /// 上一次 `Message::Scanned(_, Err(_))` 的错误文案，成功扫描或重新点
    /// "扫描"都会清掉（spec「错误处理」）：保留上一次成功结果的同时要能看到
    /// 这次失败了。
    scan_error: Option<String>,
    /// 右侧分类导航当前选中的分类（默认总览）。
    category: CodeHealthCategory,
    /// 待发提示;`App::drain_outboxes` 排空成 Toast。
    outbox: crate::extensions::toast::Outbox,
}

impl WorkspaceState {
    pub fn take_outbox(&mut self) -> Vec<crate::extensions::toast::Pending> {
        self.outbox.take()
    }

    pub fn report(&self) -> Option<&ProjectReport> {
        self.panel.report.as_ref()
    }

    pub fn previous_report(&self) -> Option<&ProjectReport> {
        self.panel.previous_report.as_ref()
    }

    pub fn diff(&self) -> Option<&ReportDiff> {
        self.panel.diff.as_ref()
    }

    pub fn architecture_diff(&self) -> &ArchitectureDiffOutcome {
        &self.panel.architecture_diff
    }

    pub fn impact(&self) -> &ImpactScope {
        &self.panel.impact
    }

    pub fn hotspots(&self) -> &[HotspotView] {
        &self.panel.hotspots
    }

    pub fn git(&self) -> Option<&GitSnapshot> {
        self.panel.git.as_ref()
    }

    pub fn scanned_at_ms(&self) -> Option<u64> {
        self.panel.scanned_at_ms
    }

    pub fn scanning(&self) -> bool {
        self.scanning
    }

    pub fn scan_error(&self) -> Option<&str> {
        self.scan_error.as_deref()
    }

    pub fn save_error(&self) -> Option<&str> {
        self.panel.save_error.as_deref()
    }

    pub fn category(&self) -> CodeHealthCategory {
        self.category
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    AsyncResult(crate::request_tracker::RequestToken, Box<Message>),
    /// 面板打开时读落盘缓存（最新 + 上一份）并计算差异/热点后的完整状态。
    /// `PanelState` 较大，装箱避免撑大外层 `Message` 枚举。
    Loaded(i64, Box<PanelState>),
    /// 缓存读取失败(`get` 失败):携带照旧算出的"无报告"面板(含 git 数据)。从未成功加载过则采用它;
    /// 已有报告则**保持现有面板**。`Unavailable` 静默(顶栏徽标已表达"dozerd 不可用");其余弹带 key 的
    /// Error Toast,相同文本 60 秒内只提示一次。
    LoadFailed(i64, ServiceErrorKind, String, Box<PanelState>),
    /// 手动扫描结果回灌（含差异/热点/Git/保存错误）。`Err` = 扫描本身失败。
    Scanned(i64, Result<Box<PanelState>, String>),
    /// 点"扫描"按钮。
    ScanRequested,
    /// 右侧分类导航点击。
    CategorySet(CodeHealthCategory),
    /// 内容侧 webview 加载失败时,原生占位页的"重试"按钮。内核拦截(清除
    /// `App::codehealth_webview` 的失败状态让 webview 重新挂载),不进本模块 `update`。
    ContentRetry,
}

pub fn update(ws_state: &mut WorkspaceState, msg: Message) {
    let mut msg = msg;
    while let Message::AsyncResult(token, inner) = msg {
        if !ws_state.requests.accepts(&token) {
            return;
        }
        msg = *inner;
    }
    match msg {
        Message::AsyncResult(..) => unreachable!("nested async result"),
        Message::Loaded(_project_id, panel) => {
            ws_state.panel = *panel;
        }
        Message::LoadFailed(_project_id, kind, message, fallback) => {
            if ws_state.panel.report.is_none() {
                ws_state.panel = *fallback;
            }
            if kind == ServiceErrorKind::Unavailable {
                dozer_core::log_debug!(LOG, "代码健康度缓存读取失败(dozerd 不可用): {message}");
            } else {
                ws_state.outbox.push_keyed_cooldown(
                    LOG,
                    crate::extensions::toast::Level::Error,
                    format!("代码健康度加载失败: {message}"),
                    "codehealth-load",
                    std::time::Instant::now(),
                );
            }
        }
        Message::Scanned(_project_id, Ok(panel)) => {
            ws_state.scanning = false;
            ws_state.scan_error = None;
            ws_state.panel = *panel;
        }
        Message::Scanned(_project_id, Err(e)) => {
            ws_state.scanning = false;
            ws_state.scan_error = Some(e);
            // 保留上一次成功结果不被覆盖。
        }
        Message::ScanRequested => {
            ws_state.scanning = true;
            ws_state.scan_error = None;
        }
        Message::CategorySet(category) => {
            ws_state.category = category;
        }
        Message::ContentRetry => {
            unreachable!("由内核拦截处理,见 codehealth::Message::ContentRetry 文档")
        }
    }
}

#[cfg(test)]
impl Message {
    // Existing payload tests inspect service conversion separately from delivery guards.
    pub(crate) fn result_payload(self) -> Self {
        match self {
            Self::AsyncResult(_, inner) => inner.result_payload(),
            other => other,
        }
    }
}

impl Message {
    pub(crate) fn result_project(&self) -> Option<i64> {
        match self {
            Self::AsyncResult(token, _) => token.project_id,
            Message::Loaded(id, _) | Message::LoadFailed(id, ..) | Message::Scanned(id, _) => {
                Some(*id)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_codehealth::{HealthTier, SCHEMA_VERSION, ScanMetadata, ScanStatus};

    pub(crate) fn sample_report() -> ProjectReport {
        ProjectReport {
            schema_version: SCHEMA_VERSION,
            scan: ScanMetadata {
                status: ScanStatus::Complete,
                ..ScanMetadata::default()
            },
            git: None,
            findings: vec![],
            architecture: dozer_codehealth::ArchitectureReport::not_applicable(),
            total_loc: 100,
            total_functions: 5,
            critical_functions: 1,
            scale_tier: HealthTier::Healthy,
            density_tier: HealthTier::Watch,
            overall_tier: HealthTier::Watch,
            functions: vec![],
            color_findings: vec![],
            color_tier: HealthTier::Healthy,
            spacing_findings: vec![],
            spacing_tier: HealthTier::Healthy,
            font_findings: vec![],
            font_tier: HealthTier::Healthy,
            distinct_color_values: 0,
            distinct_spacing_values: 0,
            duplicate_clusters: vec![],
            duplicate_cluster_tier: HealthTier::Healthy,
            nesting_depth_tier: HealthTier::Healthy,
            event_handler_tier: HealthTier::Healthy,
            ui_tier: HealthTier::Healthy,
        }
    }

    #[test]
    fn loaded_populates_report_and_timestamp() {
        let mut ws = WorkspaceState::default();
        let panel = PanelState {
            report: Some(sample_report()),
            scanned_at_ms: Some(123),
            ..PanelState::default()
        };
        update(&mut ws, Message::Loaded(1, Box::new(panel)));
        assert!(ws.report().is_some());
        assert_eq!(ws.scanned_at_ms(), Some(123));
    }

    #[test]
    fn loaded_none_means_never_scanned() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::Loaded(1, Box::default()));
        assert!(ws.report().is_none());
        assert_eq!(ws.scanned_at_ms(), None);
    }

    #[test]
    fn scan_requested_sets_scanning_flag() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::ScanRequested);
        assert!(ws.scanning());
    }

    #[test]
    fn scanned_ok_clears_scanning_and_updates_report() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::ScanRequested);
        let panel = PanelState {
            report: Some(sample_report()),
            scanned_at_ms: Some(1),
            ..PanelState::default()
        };
        update(&mut ws, Message::Scanned(1, Ok(Box::new(panel))));
        assert!(!ws.scanning());
        assert!(ws.report().is_some());
        assert!(ws.scanned_at_ms().is_some());
    }

    #[test]
    fn scanned_err_clears_scanning_but_keeps_previous_report() {
        let mut ws = WorkspaceState::default();
        let panel = PanelState {
            report: Some(sample_report()),
            scanned_at_ms: Some(1),
            ..PanelState::default()
        };
        update(&mut ws, Message::Loaded(1, Box::new(panel)));
        update(&mut ws, Message::ScanRequested);
        update(&mut ws, Message::Scanned(1, Err("扫描失败".into())));
        assert!(!ws.scanning());
        assert!(ws.report().is_some(), "失败时应保留上一次成功结果");
        assert_eq!(ws.scanned_at_ms(), Some(1), "失败不应更新时间戳");
        assert_eq!(ws.scan_error(), Some("扫描失败"));
    }

    #[test]
    fn scan_requested_clears_previous_error() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::Scanned(1, Err("扫描失败".into())));
        assert_eq!(ws.scan_error(), Some("扫描失败"));
        update(&mut ws, Message::ScanRequested);
        assert_eq!(ws.scan_error(), None);
    }

    #[test]
    fn scanned_ok_clears_previous_error() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::Scanned(1, Err("扫描失败".into())));
        let panel = PanelState {
            report: Some(sample_report()),
            ..PanelState::default()
        };
        update(&mut ws, Message::Scanned(1, Ok(Box::new(panel))));
        assert_eq!(ws.scan_error(), None);
    }

    #[test]
    fn category_defaults_to_overview() {
        assert_eq!(
            WorkspaceState::default().category(),
            CodeHealthCategory::Overview
        );
    }

    #[test]
    fn default_panel_has_no_architecture_baseline_and_empty_impact() {
        let ws = WorkspaceState::default();
        assert!(matches!(
            ws.architecture_diff(),
            dozer_codehealth::ArchitectureDiffOutcome::NoBaseline
        ));
        assert!(ws.impact().is_empty());
    }

    #[test]
    fn category_set_switches_category() {
        let mut ws = WorkspaceState::default();
        update(
            &mut ws,
            Message::CategorySet(CodeHealthCategory::UiConsistency),
        );
        assert_eq!(ws.category(), CodeHealthCategory::UiConsistency);
        update(&mut ws, Message::CategorySet(CodeHealthCategory::Overview));
        assert_eq!(ws.category(), CodeHealthCategory::Overview);
    }

    #[test]
    fn save_error_is_kept_separate_from_scan_error() {
        let mut ws = WorkspaceState::default();
        let panel = PanelState {
            report: Some(sample_report()),
            save_error: Some("未保存，无法用于下次比较".into()),
            ..PanelState::default()
        };
        update(&mut ws, Message::Loaded(1, Box::new(panel)));
        assert_eq!(ws.save_error(), Some("未保存，无法用于下次比较"));
        assert_eq!(ws.scan_error(), None);
    }

    #[test]
    fn content_retry_message_is_kernel_intercepted() {
        // 由内核拦截,不进本模块 update。
        let result = std::panic::catch_unwind(|| {
            let mut ws = WorkspaceState::default();
            update(&mut ws, Message::ContentRetry);
        });
        assert!(result.is_err(), "ContentRetry 必须由内核拦截");
    }

    // ---- H6:切入钩子 ----

    use crate::panel_host::testing::{TIMEOUT, offline_ctx, runtime};
    use dozer_services::ServiceErrorKind;

    #[test]
    fn on_activate_only_loads_the_cached_report_and_never_starts_a_scan() {
        let rt = runtime();
        let (ctx, rx) = offline_ctx::<Message>(&rt, 7, Some(std::env::temp_dir()));
        let ctx = crate::panel_host::ActivationCtx {
            io: ctx
                .io
                .with_services(crate::service_fakes::FakeCodeHealth::default().into_services()),
            ..ctx
        };
        on_activate(&WorkspaceState::default(), &ctx);
        assert!(matches!(
            rx.recv_timeout(TIMEOUT).unwrap().result_payload(),
            Message::Loaded(7, _)
        ));
        // 切入不自动扫描(spec:手动触发):缓存加载之后不应再有任何消息
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(300))
                .is_err()
        );
    }

    #[test]
    fn on_activate_without_a_project_path_does_nothing() {
        let rt = runtime();
        let (ctx, rx) = offline_ctx::<Message>(&rt, 7, None);
        on_activate(&WorkspaceState::default(), &ctx);
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(200))
                .is_err()
        );
    }

    fn failed(kind: ServiceErrorKind, text: &str, marker: u64) -> Message {
        Message::LoadFailed(
            7,
            kind,
            text.into(),
            Box::new(PanelState {
                scanned_at_ms: Some(marker),
                ..PanelState::default()
            }),
        )
    }

    #[test]
    fn first_load_failure_adopts_the_carried_panel() {
        let mut ws = WorkspaceState::default();
        update(
            &mut ws,
            failed(ServiceErrorKind::Unavailable, "daemon 断开", 1),
        );
        assert_eq!(
            ws.panel.scanned_at_ms,
            Some(1),
            "从未加载过:采用携带的 panel(含 git 数据),同现状"
        );
        assert!(
            ws.take_outbox().is_empty(),
            "Unavailable 由顶栏徽标表达,不弹 Toast"
        );
    }

    #[test]
    fn a_load_failure_keeps_the_report_already_shown() {
        let mut ws = WorkspaceState {
            panel: PanelState {
                report: Some(sample_report()),
                scanned_at_ms: Some(9),
                ..PanelState::default()
            },
            ..WorkspaceState::default()
        };
        update(
            &mut ws,
            failed(ServiceErrorKind::Unavailable, "daemon 断开", 1),
        );
        assert!(ws.report().is_some(), "读失败不得把已显示的报告换成空面板");
        assert_eq!(ws.panel.scanned_at_ms, Some(9));
    }

    #[test]
    fn repeated_rejected_load_failures_toast_once_per_text_with_a_key() {
        let mut ws = WorkspaceState::default();
        for _ in 0..3 {
            update(
                &mut ws,
                failed(ServiceErrorKind::Rejected, "daemon 错误: boom", 1),
            );
        }
        let toasts = ws.take_outbox();
        assert_eq!(toasts.len(), 1, "相同文本 60 秒内只提示一次");
        assert_eq!(toasts[0].level, crate::extensions::toast::Level::Error);
        assert!(toasts[0].text.contains("daemon 错误: boom"));
        assert_eq!(toasts[0].key.as_deref(), Some("codehealth-load"));
        update(&mut ws, failed(ServiceErrorKind::Protocol, "另一种错误", 1));
        assert_eq!(ws.take_outbox().len(), 1, "换了文本要再提示");
    }

    #[test]
    fn on_activate_reports_a_failed_cache_read_as_load_failed() {
        let rt = runtime();
        // 默认服务束指向不存在的 socket → get 失败 → LoadFailed(7, Unavailable, ..)
        let (ctx, rx) = offline_ctx::<Message>(&rt, 7, Some(std::env::temp_dir()));
        on_activate(&WorkspaceState::default(), &ctx);
        assert!(matches!(
            rx.recv_timeout(TIMEOUT).unwrap().result_payload(),
            Message::LoadFailed(7, ServiceErrorKind::Unavailable, _, _)
        ));
    }

    #[test]
    fn stale_cache_and_scan_results_do_not_overwrite_latest_report() {
        let mut state = WorkspaceState::default();
        let cache = state.requests.begin(Some(1), "report");
        update(&mut state, Message::ScanRequested);
        let scan = state.requests.begin(Some(1), "report");
        update(
            &mut state,
            Message::AsyncResult(cache.clone(), Box::new(Message::Loaded(1, Box::default()))),
        );
        assert!(state.scanning());
        let panel = PanelState {
            scanned_at_ms: Some(42),
            ..Default::default()
        };
        update(
            &mut state,
            Message::AsyncResult(
                scan.clone(),
                Box::new(Message::Scanned(1, Ok(Box::new(panel)))),
            ),
        );
        update(
            &mut state,
            Message::AsyncResult(cache, Box::new(Message::Loaded(1, Box::default()))),
        );
        assert_eq!(state.scanned_at_ms(), Some(42));
        let next = state.requests.begin(Some(1), "report");
        update(&mut state, Message::ScanRequested);
        update(
            &mut state,
            Message::AsyncResult(
                scan,
                Box::new(Message::Scanned(1, Err("old failure".into()))),
            ),
        );
        assert!(state.scanning());
        assert!(state.scan_error().is_none());
        update(
            &mut state,
            Message::AsyncResult(
                next,
                Box::new(Message::Scanned(1, Err("current failure".into()))),
            ),
        );
        assert!(!state.scanning());
        assert_eq!(state.scan_error(), Some("current failure"));
    }

    #[test]
    fn activating_during_scan_does_not_supersede_scan_with_cache() {
        let rt = runtime();
        let (ctx, rx) = offline_ctx::<Message>(&rt, 7, Some(std::env::temp_dir()));
        let mut state = WorkspaceState::default();
        update(&mut state, Message::ScanRequested);
        let scan = state.requests.begin(Some(7), "report");
        on_activate(&state, &ctx);
        assert!(state.requests.accepts(&scan));
        assert!(rx.try_recv().is_err());
    }
}
