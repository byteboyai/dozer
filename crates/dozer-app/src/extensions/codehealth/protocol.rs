//! 代码健康度内容侧 webview 推送协议:把 `WorkspaceState` 算成要序列化推给
//! webview 的 `CodeHealthViewPayload`。前端只渲染,不做聚合/排序/差异计算——
//! 这些全在 `view_model.rs`/`aggregate.rs`/`git_hotspots.rs`。
//! 形态仿 `extensions/usage/protocol.rs`(声明式比较 + 单槽)。

use super::view_model::{self, ChangeSummary, EmptyState};
use super::{CodeHealthCategory, WorkspaceState};
use dozer_codehealth::{FindingChange, FindingEvidence, FindingSeverity, HealthTier, rule_ids};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

pub const CODEHEALTH_PROTOCOL_VERSION: u32 = 1;

/// webview 加载后超过这个时长还没发 `ready` 就判失败,回落原生占位页。
pub const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// 面板分类(JSON 里的小写下划线形式)。阶段二会加 `Architecture`。
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CategoryKey {
    Overview,
    Structure,
    UiConsistency,
    ScanScope,
}

impl From<CodeHealthCategory> for CategoryKey {
    fn from(c: CodeHealthCategory) -> Self {
        match c {
            CodeHealthCategory::Overview => CategoryKey::Overview,
            CodeHealthCategory::Structure => CategoryKey::Structure,
            CodeHealthCategory::UiConsistency => CategoryKey::UiConsistency,
            CodeHealthCategory::ScanScope => CategoryKey::ScanScope,
        }
    }
}

/// 页头(扫描按钮 + 状态条)需要的数据。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ScanBar {
    pub scanning: bool,
    /// 已格式化的"2026-09-20 07:58:06 UTC";`None` = 尚未扫描。
    pub scanned_at: Option<String>,
    pub scan_error: Option<String>,
    pub save_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SeverityKey {
    Critical,
    Watch,
}

impl From<FindingSeverity> for SeverityKey {
    fn from(s: FindingSeverity) -> Self {
        match s {
            FindingSeverity::Critical => SeverityKey::Critical,
            FindingSeverity::Watch => SeverityKey::Watch,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKey {
    New,
    Worsened,
    Improved,
    Persisting,
    Resolved,
}

impl From<FindingChange> for ChangeKey {
    fn from(c: FindingChange) -> Self {
        match c {
            FindingChange::New => ChangeKey::New,
            FindingChange::Worsened => ChangeKey::Worsened,
            FindingChange::Improved => ChangeKey::Improved,
            FindingChange::Persisting => ChangeKey::Persisting,
            FindingChange::Resolved => ChangeKey::Resolved,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TierKey {
    Healthy,
    Watch,
    Critical,
}

impl From<HealthTier> for TierKey {
    fn from(t: HealthTier) -> Self {
        match t {
            HealthTier::Healthy => TierKey::Healthy,
            HealthTier::Watch => TierKey::Watch,
            HealthTier::Critical => TierKey::Critical,
        }
    }
}

/// 一条发现行(总览优先处理、结构复杂度、UI 一致性共用)。文案在 Rust 里
/// 算好,前端不拼业务文案。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FindingRowDto {
    pub id: String,
    pub title: String,
    /// 相对项目根的路径(与报告里的 `Finding.path` 一致)。
    pub path: String,
    pub line: usize,
    pub severity: SeverityKey,
    pub severity_label: &'static str,
    pub change: Option<ChangeKey>,
    pub change_label: Option<&'static str>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChangeDto {
    FirstScan,
    HasChange {
        loc_delta_text: String,
        functions_delta_text: String,
        new_risks: usize,
        resolved_risks: usize,
        worsened: usize,
        improved: usize,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct OverviewBody {
    /// "main · abc123 · 有未提交改动";无 Git 时为 `None`。
    pub git_line: Option<String>,
    pub tier: TierKey,
    pub tier_label: &'static str,
    pub summary: String,
    pub change: ChangeDto,
    pub priorities: Vec<FindingRowDto>,
    /// "已分析 N 个文件 · … · 排除 N · 跳过 N";无摘要时为空串。
    pub scope_line: String,
    pub legacy_note: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StructureBody {
    pub metric_note: &'static str,
    /// 全部结构复杂度发现;「本轮新增 / 全部」筛选由前端按 `change` 本地做
    /// (`new`/`worsened` 算本轮新增,`change == None`(无差异数据)不算)。
    pub findings: Vec<FindingRowDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UiGroup {
    pub title: &'static str,
    pub findings: Vec<FindingRowDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UiBody {
    pub applicable: bool,
    pub groups: Vec<UiGroup>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SkippedDto {
    pub path: String,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ScopeBody {
    pub status_label: &'static str,
    pub analyzed_files: usize,
    pub excluded_files: usize,
    pub skipped_count: usize,
    pub languages_detail: String,
    pub duration_ms: u64,
    pub schema_version: u32,
    pub git_baseline: Option<String>,
    pub skipped: Vec<SkippedDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Empty { message: String },
    Overview(OverviewBody),
    Structure(StructureBody),
    UiConsistency(UiBody),
    ScanScope(ScopeBody),
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CodeHealthViewPayload {
    pub scan: ScanBar,
    pub category: CategoryKey,
    pub body: Body,
}

fn finding_change(ws: &WorkspaceState, id: &str) -> Option<FindingChange> {
    ws.diff().map(|d| {
        if d.new.iter().any(|x| x.id == id) {
            FindingChange::New
        } else if d.worsened.iter().any(|x| x.id == id) {
            FindingChange::Worsened
        } else if d.improved.iter().any(|x| x.id == id) {
            FindingChange::Improved
        } else {
            FindingChange::Persisting
        }
    })
}

pub(super) fn row_dto(
    id: String,
    title: String,
    path: &Path,
    line: usize,
    severity: FindingSeverity,
    change: Option<FindingChange>,
    reasons: Vec<String>,
) -> FindingRowDto {
    FindingRowDto {
        id,
        title,
        path: path.to_string_lossy().into_owned(),
        line,
        severity: severity.into(),
        severity_label: view_model::sev_label(severity),
        change: change.map(Into::into),
        change_label: change.map(view_model::change_label),
        reasons,
    }
}

fn overview_body(ws: &WorkspaceState, report: &dozer_codehealth::ProjectReport) -> OverviewBody {
    let git_line = ws.git().map(|git| {
        let branch = git.branch.clone().unwrap_or_else(|| "detached".to_string());
        let head = git.head.clone().unwrap_or_default();
        let mut s = format!("{branch} · {head}");
        if git.dirty {
            s.push_str(" · 有未提交改动");
        }
        s
    });
    let scope_line = view_model::scope_summary(ws)
        .map(|s| {
            format!(
                "已分析 {} 个文件 · {} · 排除 {} · 跳过 {}",
                s.analyzed_files, s.languages, s.excluded_files, s.skipped_files
            )
        })
        .unwrap_or_default();
    let change = match view_model::change_summary(report, ws.previous_report(), ws.diff()) {
        ChangeSummary::FirstScan => ChangeDto::FirstScan,
        ChangeSummary::HasChange(c) => ChangeDto::HasChange {
            loc_delta_text: view_model::signed_delta(c.loc_delta),
            functions_delta_text: view_model::signed_delta(c.functions_delta),
            new_risks: c.new_risks,
            resolved_risks: c.resolved_risks,
            worsened: c.worsened,
            improved: c.improved,
        },
    };
    let priorities = view_model::hotspot_rows(ws.hotspots(), 50)
        .into_iter()
        .map(|r| {
            row_dto(
                r.finding_id,
                r.title,
                &r.path,
                r.line,
                r.severity,
                Some(r.change),
                r.reasons,
            )
        })
        .collect();
    OverviewBody {
        git_line,
        tier: report.overall_tier.into(),
        tier_label: view_model::tier_label(report.overall_tier),
        summary: format!(
            "核心代码 {} 行 · {} 个函数存在明显结构问题",
            report.total_loc, report.critical_functions
        ),
        change,
        priorities,
        scope_line,
        legacy_note: view_model::legacy_report_note(report.schema_version),
    }
}

fn structure_body(ws: &WorkspaceState, report: &dozer_codehealth::ProjectReport) -> StructureBody {
    let findings = report
        .findings
        .iter()
        .filter(|f| f.rule_id == rule_ids::STRUCTURE_COMPLEXITY)
        .map(|f| {
            let metric = match &f.evidence {
                FindingEvidence::Structure {
                    complexity_signal, ..
                } => *complexity_signal,
                _ => 0,
            };
            row_dto(
                f.id.clone(),
                format!("{} 控制流信号 {}", f.symbol.as_deref().unwrap_or("?"), metric),
                &f.path,
                f.start_line,
                f.severity,
                finding_change(ws, &f.id),
                Vec::new(),
            )
        })
        .collect();
    StructureBody {
        metric_note: "指标口径:控制流信号(非标准圈复杂度)",
        findings,
    }
}

fn ui_body(report: &dozer_codehealth::ProjectReport) -> UiBody {
    if !view_model::ui_consistency_applicable(report) {
        return UiBody {
            applicable: false,
            groups: Vec::new(),
        };
    }
    let rules: [(&'static str, &str); 6] = [
        ("颜色硬编码", rule_ids::COLOR_HARDCODE),
        ("边距硬编码", rule_ids::SPACING_HARDCODE),
        ("字体硬编码", rule_ids::FONT_HARDCODE),
        ("组件树嵌套深度", rule_ids::NESTING_DEPTH),
        ("事件回调密度", rule_ids::EVENT_HANDLER_DENSITY),
        ("组件化重复结构", rule_ids::DUPLICATE_STRUCTURE),
    ];
    let groups = rules
        .iter()
        .map(|(title, rule)| UiGroup {
            title,
            findings: report
                .findings
                .iter()
                .filter(|f| f.rule_id == *rule)
                .map(|f| {
                    row_dto(
                        f.id.clone(),
                        f.title.clone(),
                        &f.path,
                        f.start_line,
                        f.severity,
                        None,
                        Vec::new(),
                    )
                })
                .collect(),
        })
        .collect();
    UiBody {
        applicable: true,
        groups,
    }
}

fn scope_body(ws: &WorkspaceState, report: &dozer_codehealth::ProjectReport) -> ScopeBody {
    let scan = &report.scan;
    let status_label = match scan.status {
        dozer_codehealth::ScanStatus::Complete => "完整",
        dozer_codehealth::ScanStatus::Partial => "部分完成",
        dozer_codehealth::ScanStatus::Failed => "失败",
    };
    let languages_detail = scan
        .languages
        .iter()
        .map(|l| {
            if l.analyzed {
                format!("{}({} 个文件,结构分析)", l.language, l.files)
            } else {
                format!("{}({} 个文件,仅统计)", l.language, l.files)
            }
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let git_baseline = ws.git().map(|git| {
        format!(
            "{}{}",
            git.branch.clone().unwrap_or_else(|| "detached".into()),
            git.head
                .clone()
                .map(|h| format!(" @ {h}"))
                .unwrap_or_default()
        )
    });
    let skipped = scan
        .skipped_files
        .iter()
        .map(|s| SkippedDto {
            path: s.path.to_string_lossy().into_owned(),
            reason: match s.reason {
                dozer_codehealth::SkipReason::Ignored => "忽略规则",
                dozer_codehealth::SkipReason::UnsupportedLanguage => "不支持的语言",
                dozer_codehealth::SkipReason::Generated => "生成文件",
                dozer_codehealth::SkipReason::NonUtf8 => "非 UTF-8",
                dozer_codehealth::SkipReason::ReadFailed => "读取失败",
                dozer_codehealth::SkipReason::ParseFailed => "解析失败",
            },
        })
        .collect();
    ScopeBody {
        status_label,
        analyzed_files: scan.analyzed_files,
        excluded_files: scan.excluded_files,
        skipped_count: scan.skipped_files.len(),
        languages_detail,
        duration_ms: scan.duration_ms,
        schema_version: report.schema_version,
        git_baseline,
        skipped,
    }
}

fn empty_message(ws: &WorkspaceState) -> String {
    if ws.scanning() {
        return "扫描中…".to_string();
    }
    match view_model::empty_state(ws) {
        EmptyState::NeverScanned => "这个项目还没有扫描过。",
        EmptyState::NoSupportedCode => "没有发现受支持的代码。",
        EmptyState::PartialFailure => "扫描部分完成,部分文件失败(见扫描范围)。",
        EmptyState::ScanFailed => "扫描失败。",
    }
    .to_string()
}

/// 当前该显示什么。**唯一权威实现**:`view.rs` 旧渲染删除后不再有第二份判定。
pub fn current_view_payload(ws: &WorkspaceState) -> CodeHealthViewPayload {
    let scan = ScanBar {
        scanning: ws.scanning(),
        scanned_at: ws.scanned_at_ms().map(view_model::format_ms),
        scan_error: ws.scan_error().map(str::to_owned),
        save_error: ws.save_error().map(str::to_owned),
    };
    let category: CategoryKey = ws.category().into();
    let body = match ws.report() {
        None => Body::Empty {
            message: empty_message(ws),
        },
        Some(report) => match category {
            CategoryKey::Overview => Body::Overview(overview_body(ws, report)),
            CategoryKey::Structure => Body::Structure(structure_body(ws, report)),
            CategoryKey::UiConsistency => Body::UiConsistency(ui_body(report)),
            CategoryKey::ScanScope => Body::ScanScope(scope_body(ws, report)),
        },
    };
    CodeHealthViewPayload {
        scan,
        category,
        body,
    }
}

// ---------------------------------------------------------------------------
// webview → Rust 事件
// ---------------------------------------------------------------------------

/// webview 发回的事件。不复用 `preview::webview_protocol::EditorEvent`
/// (那套带 CodeMirror 专属变体)。
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CodeHealthWebviewEvent {
    Ready,
    ScanRequested,
    OpenLocation { path: PathBuf, line: usize },
    AnalyzeFinding { id: String },
    Failed { reason: String },
}

pub fn parse_codehealth_event(body: &str) -> Result<CodeHealthWebviewEvent, String> {
    serde_json::from_str(body).map_err(|e| e.to_string())
}

/// webview 内容不可信任到能指定任意路径:只接受非空、无 `..`、非绝对的相对路径
/// (发现项的路径本来就是相对项目根的规范化路径)。
pub fn is_safe_relative_path(path: &Path) -> bool {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return false;
    }
    path.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

// ---------------------------------------------------------------------------
// Rust → webview 推送
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CodeHealthPushEnvelope {
    pub protocol_version: u32,
    /// 单调递增;前端丢弃小于已应用值的推送。
    pub revision: u64,
    pub payload: CodeHealthViewPayload,
}

pub fn encode_codehealth_push(revision: u64, payload: CodeHealthViewPayload) -> String {
    serde_json::to_string(&CodeHealthPushEnvelope {
        protocol_version: CODEHEALTH_PROTOCOL_VERSION,
        revision,
        payload,
    })
    .unwrap_or_else(|_| "null".to_string())
}

/// App 级(固定单槽,不按项目分)内容侧 webview 推送状态。纯状态,可单测,
/// 不碰 webview 池——调用方(`App::take_codehealth_content_script`)据
/// `pending_push` 组 envelope,真正注入后调 `mark_sent`。
#[derive(Default)]
pub struct WebviewPushState {
    ready: bool,
    last_sent: Option<CodeHealthViewPayload>,
    revision: u64,
    /// webview 已进池但尚未 ready 的起点,用于判定加载超时。
    waiting_since: Option<Instant>,
    /// 加载失败/超时原因;`Some` 时内容区回落原生占位页(重试清除)。
    failed: Option<String>,
}

impl WebviewPushState {
    pub fn set_ready(&mut self, ready: bool) {
        self.ready = ready;
        if ready {
            // 新实例(或重新确认 ready)一律强制重发当前内容。
            self.last_sent = None;
            self.waiting_since = None;
        }
    }

    /// 每帧由消费点调用:`available` = webview 当前在池里。
    pub fn observe_availability(&mut self, available: bool, now: Instant) {
        if !available {
            self.ready = false;
            self.waiting_since = None;
            return;
        }
        if self.ready {
            self.waiting_since = None;
            return;
        }
        let since = *self.waiting_since.get_or_insert(now);
        if self.failed.is_none() && now.duration_since(since) > READY_TIMEOUT {
            self.failed = Some("面板页面加载超时".to_string());
        }
    }

    pub fn pending_push(&self, desired: &CodeHealthViewPayload) -> Option<CodeHealthViewPayload> {
        if !self.ready || self.last_sent.as_ref() == Some(desired) {
            return None;
        }
        Some(desired.clone())
    }

    /// 记录已送达并返回本次 revision(从 1 起)。
    pub fn mark_sent(&mut self, payload: CodeHealthViewPayload) -> u64 {
        self.revision += 1;
        self.last_sent = Some(payload);
        self.revision
    }

    pub fn failed(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    pub fn set_failed(&mut self, reason: String) {
        self.failed = Some(reason);
        self.ready = false;
        self.last_sent = None;
        self.waiting_since = None;
    }

    pub fn clear_failed(&mut self) {
        self.failed = None;
        self.waiting_since = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_codehealth::{
        ArchitectureReport, Finding, FindingCategory, HealthTier, ProjectReport, SCHEMA_VERSION,
        ScanMetadata, ScanStatus,
    };

    fn sample_report() -> ProjectReport {
        ProjectReport {
            schema_version: SCHEMA_VERSION,
            scan: ScanMetadata {
                status: ScanStatus::Complete,
                analyzed_files: 3,
                ..ScanMetadata::default()
            },
            git: None,
            findings: vec![],
            architecture: ArchitectureReport::not_applicable(),
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

    fn structure_finding(id: &str, signal: usize) -> Finding {
        Finding {
            id: id.into(),
            rule_id: rule_ids::STRUCTURE_COMPLEXITY.into(),
            category: FindingCategory::Structure,
            severity: FindingSeverity::Critical,
            path: PathBuf::from("src/a.rs"),
            start_line: 7,
            symbol: Some("run".into()),
            title: "run".into(),
            evidence: FindingEvidence::Structure {
                complexity_signal: signal,
                loc: 40,
                widget_nesting_depth: 0,
                event_handler_count: 0,
            },
            applicability: Default::default(),
        }
    }

    fn ws_with(report: ProjectReport) -> WorkspaceState {
        let mut ws = WorkspaceState::default();
        super::super::update(
            &mut ws,
            super::super::Message::Loaded(
                1,
                Box::new(super::super::PanelState {
                    report: Some(report),
                    scanned_at_ms: Some(0),
                    ..Default::default()
                }),
            ),
        );
        ws
    }

    #[test]
    fn no_report_yields_empty_never_scanned() {
        let p = current_view_payload(&WorkspaceState::default());
        assert!(matches!(p.body, Body::Empty { .. }));
        assert!(!p.scan.scanning);
        assert_eq!(p.scan.scanned_at, None);
    }

    #[test]
    fn scanning_without_report_says_scanning() {
        let mut ws = WorkspaceState::default();
        super::super::update(&mut ws, super::super::Message::ScanRequested);
        match current_view_payload(&ws).body {
            Body::Empty { message } => assert_eq!(message, "扫描中…"),
            other => panic!("expected Empty, got {other:?}"),
        }
    }

    #[test]
    fn scan_error_without_report_reports_failure() {
        let mut ws = WorkspaceState::default();
        super::super::update(
            &mut ws,
            super::super::Message::Scanned(1, Err("boom".into())),
        );
        let p = current_view_payload(&ws);
        assert_eq!(p.scan.scan_error.as_deref(), Some("boom"));
        match p.body {
            Body::Empty { message } => assert_eq!(message, "扫描失败。"),
            other => panic!("expected Empty, got {other:?}"),
        }
    }

    #[test]
    fn overview_first_scan_has_first_scan_change_and_tier() {
        let ws = ws_with(sample_report());
        let p = current_view_payload(&ws);
        assert_eq!(p.category, CategoryKey::Overview);
        assert_eq!(p.scan.scanned_at.as_deref(), Some("1970-01-01 00:00:00 UTC"));
        match p.body {
            Body::Overview(o) => {
                assert_eq!(o.tier_label, "需要关注");
                assert_eq!(o.summary, "核心代码 100 行 · 1 个函数存在明显结构问题");
                assert!(matches!(o.change, ChangeDto::FirstScan));
                assert!(o.priorities.is_empty());
            }
            other => panic!("expected Overview, got {other:?}"),
        }
    }

    #[test]
    fn structure_page_carries_all_structure_findings_with_titles() {
        let mut report = sample_report();
        report.findings.push(structure_finding("f1", 12));
        let mut ws = ws_with(report);
        super::super::update(
            &mut ws,
            super::super::Message::CategorySet(CodeHealthCategory::Structure),
        );
        match current_view_payload(&ws).body {
            Body::Structure(s) => {
                assert_eq!(s.metric_note, "指标口径:控制流信号(非标准圈复杂度)");
                assert_eq!(s.findings.len(), 1);
                assert_eq!(s.findings[0].title, "run 控制流信号 12");
                assert_eq!(s.findings[0].path, "src/a.rs");
                assert_eq!(s.findings[0].line, 7);
                assert_eq!(s.findings[0].severity_label, "警戒");
                assert_eq!(s.findings[0].change, None);
            }
            other => panic!("expected Structure, got {other:?}"),
        }
    }

    #[test]
    fn ui_consistency_not_applicable_without_iced() {
        let mut ws = ws_with(sample_report());
        super::super::update(
            &mut ws,
            super::super::Message::CategorySet(CodeHealthCategory::UiConsistency),
        );
        match current_view_payload(&ws).body {
            Body::UiConsistency(u) => {
                assert!(!u.applicable);
                assert!(u.groups.is_empty());
            }
            other => panic!("expected UiConsistency, got {other:?}"),
        }
    }

    #[test]
    fn scan_scope_reports_counts_and_status() {
        let mut ws = ws_with(sample_report());
        super::super::update(
            &mut ws,
            super::super::Message::CategorySet(CodeHealthCategory::ScanScope),
        );
        match current_view_payload(&ws).body {
            Body::ScanScope(s) => {
                assert_eq!(s.status_label, "完整");
                assert_eq!(s.analyzed_files, 3);
                assert_eq!(s.schema_version, SCHEMA_VERSION);
                assert!(s.skipped.is_empty());
            }
            other => panic!("expected ScanScope, got {other:?}"),
        }
    }

    #[test]
    fn payload_serializes_with_kind_tags() {
        let ws = ws_with(sample_report());
        let json = serde_json::to_value(current_view_payload(&ws)).unwrap();
        assert_eq!(json["category"], "overview");
        assert_eq!(json["body"]["kind"], "overview");
        assert_eq!(json["body"]["change"]["kind"], "first_scan");
        assert_eq!(json["scan"]["scanning"], false);
    }

    // ---- 事件 ----

    #[test]
    fn parses_all_event_kinds() {
        assert_eq!(
            parse_codehealth_event(r#"{"kind":"ready"}"#).unwrap(),
            CodeHealthWebviewEvent::Ready
        );
        assert_eq!(
            parse_codehealth_event(r#"{"kind":"scan_requested"}"#).unwrap(),
            CodeHealthWebviewEvent::ScanRequested
        );
        assert_eq!(
            parse_codehealth_event(r#"{"kind":"open_location","path":"src/a.rs","line":7}"#)
                .unwrap(),
            CodeHealthWebviewEvent::OpenLocation {
                path: PathBuf::from("src/a.rs"),
                line: 7
            }
        );
        assert_eq!(
            parse_codehealth_event(r#"{"kind":"analyze_finding","id":"f1"}"#).unwrap(),
            CodeHealthWebviewEvent::AnalyzeFinding { id: "f1".into() }
        );
        assert_eq!(
            parse_codehealth_event(r#"{"kind":"failed","reason":"x"}"#).unwrap(),
            CodeHealthWebviewEvent::Failed { reason: "x".into() }
        );
        assert!(parse_codehealth_event("not json").is_err());
        assert!(parse_codehealth_event(r#"{"kind":"nope"}"#).is_err());
    }

    /// Review Focus 1:webview 发来的路径不能是绝对路径或含 `..`。
    #[test]
    fn unsafe_paths_are_rejected() {
        assert!(is_safe_relative_path(Path::new("src/a.rs")));
        assert!(is_safe_relative_path(Path::new("crates/x/src/lib.rs")));
        assert!(!is_safe_relative_path(Path::new("/etc/passwd")));
        assert!(!is_safe_relative_path(Path::new("../outside.rs")));
        assert!(!is_safe_relative_path(Path::new("src/../../outside.rs")));
        assert!(!is_safe_relative_path(Path::new("")));
    }

    // ---- 推送状态 ----

    fn payload() -> CodeHealthViewPayload {
        current_view_payload(&WorkspaceState::default())
    }

    #[test]
    fn pending_push_requires_ready_and_changes() {
        let mut s = WebviewPushState::default();
        assert_eq!(s.pending_push(&payload()), None, "未 ready 不推");
        s.set_ready(true);
        let p = payload();
        assert_eq!(s.pending_push(&p), Some(p.clone()));
        let rev = s.mark_sent(p.clone());
        assert_eq!(rev, 1);
        assert_eq!(s.pending_push(&p), None, "同值不重推");
    }

    /// Review Focus 6:webview 重建后新实例第一次 ready 必须强制重发。
    #[test]
    fn set_ready_true_forces_resend() {
        let mut s = WebviewPushState::default();
        s.set_ready(true);
        let p = payload();
        s.mark_sent(p.clone());
        assert_eq!(s.pending_push(&p), None);
        s.set_ready(true);
        assert_eq!(s.pending_push(&p), Some(p));
    }

    /// Review Focus 3:revision 单调递增。
    #[test]
    fn revision_increments_per_send() {
        let mut s = WebviewPushState::default();
        s.set_ready(true);
        assert_eq!(s.mark_sent(payload()), 1);
        assert_eq!(s.mark_sent(payload()), 2);
    }

    /// Review Focus 2:一直不 ready 超时 → failed。
    #[test]
    fn never_ready_times_out_to_failed() {
        let mut s = WebviewPushState::default();
        let t0 = Instant::now();
        s.observe_availability(true, t0);
        assert_eq!(s.failed(), None);
        s.observe_availability(true, t0 + READY_TIMEOUT + Duration::from_millis(1));
        assert!(s.failed().is_some());
        s.clear_failed();
        assert_eq!(s.failed(), None);
    }

    #[test]
    fn ready_before_timeout_never_fails() {
        let mut s = WebviewPushState::default();
        let t0 = Instant::now();
        s.observe_availability(true, t0);
        s.set_ready(true);
        s.observe_availability(true, t0 + READY_TIMEOUT * 10);
        assert_eq!(s.failed(), None);
    }

    #[test]
    fn unavailable_resets_ready() {
        let mut s = WebviewPushState::default();
        s.set_ready(true);
        s.observe_availability(false, Instant::now());
        assert_eq!(s.pending_push(&payload()), None);
    }

    #[test]
    fn envelope_carries_version_and_revision() {
        let json: serde_json::Value =
            serde_json::from_str(&encode_codehealth_push(5, payload())).unwrap();
        assert_eq!(json["protocol_version"], CODEHEALTH_PROTOCOL_VERSION);
        assert_eq!(json["revision"], 5);
        assert!(json["payload"]["scan"].is_object());
    }
}
