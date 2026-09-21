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

pub(crate) mod view_model;

use dozer_codehealth::{
    ArchitectureDiffOutcome, GitSnapshot, ImpactScope, ProjectReport, ReportDiff,
};

/// 面板右侧分类导航的四个分类：总览 / 结构复杂度 / UI 一致性 / 扫描范围。
/// 左侧内容区按当前选中分类切换展示（见 `view.rs`）。spec「信息架构与 UI」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CodeHealthCategory {
    #[default]
    Overview,
    Structure,
    UiConsistency,
    ScanScope,
}

impl CodeHealthCategory {
    pub fn label(self) -> &'static str {
        match self {
            CodeHealthCategory::Overview => "总览",
            CodeHealthCategory::Structure => "结构复杂度",
            CodeHealthCategory::UiConsistency => "UI 一致性",
            CodeHealthCategory::ScanScope => "扫描范围",
        }
    }

    pub fn all() -> [CodeHealthCategory; 4] {
        [
            CodeHealthCategory::Overview,
            CodeHealthCategory::Structure,
            CodeHealthCategory::UiConsistency,
            CodeHealthCategory::ScanScope,
        ]
    }
}

/// 结构复杂度页的筛选：只看本轮新增 / 全部。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StructureFilter {
    #[default]
    New,
    All,
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
    panel: PanelState,
    scanning: bool,
    /// 上一次 `Message::Scanned(_, Err(_))` 的错误文案，成功扫描或重新点
    /// "扫描"都会清掉（spec「错误处理」）：保留上一次成功结果的同时要能看到
    /// 这次失败了。
    scan_error: Option<String>,
    /// 右侧分类导航当前选中的分类（默认总览）。
    category: CodeHealthCategory,
    /// 结构复杂度页筛选（默认只看本轮新增）。
    structure_filter: StructureFilter,
}

impl WorkspaceState {
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

    pub fn structure_filter(&self) -> StructureFilter {
        self.structure_filter
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    /// 面板打开时读落盘缓存（最新 + 上一份）并计算差异/热点后的完整状态。
    /// `PanelState` 较大，装箱避免撑大外层 `Message` 枚举。
    Loaded(i64, Box<PanelState>),
    /// 手动扫描结果回灌（含差异/热点/Git/保存错误）。`Err` = 扫描本身失败。
    Scanned(i64, Result<Box<PanelState>, String>),
    /// 点"扫描"按钮。
    ScanRequested,
    /// 右侧分类导航点击。
    CategorySet(CodeHealthCategory),
    /// 结构复杂度页筛选切换（本轮新增 / 全部）。
    StructureFilterSet(StructureFilter),
    /// 发现行点击：文件路径 + 目标行(1-based)。由内核（`app/update.rs`）拦截
    /// 转成顶层 `Message::CodeHealthOpenLocation`，不进本模块 `update`。
    OpenLocation(std::path::PathBuf, usize),
    /// "交给 Agent 分析"：携带 finding ID（不含可被 UI 篡改的完整 prompt）。
    /// 内核拦截解析成诊断文本送入 agent 输入区。
    AnalyzeFinding(String),
}

pub fn update(ws_state: &mut WorkspaceState, msg: Message) {
    match msg {
        Message::Loaded(_project_id, panel) => {
            ws_state.panel = *panel;
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
        Message::StructureFilterSet(filter) => {
            ws_state.structure_filter = filter;
        }
        Message::OpenLocation(..) => {
            unreachable!("由内核拦截处理,见 codehealth::Message::OpenLocation 文档")
        }
        Message::AnalyzeFinding(..) => {
            unreachable!("由内核拦截处理,见 codehealth::Message::AnalyzeFinding 文档")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_codehealth::{HealthTier, SCHEMA_VERSION, ScanMetadata, ScanStatus};

    fn sample_report() -> ProjectReport {
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
}
