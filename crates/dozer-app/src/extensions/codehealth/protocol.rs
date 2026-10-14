//! 代码健康度内容侧 webview 推送协议:把 `WorkspaceState` 算成要序列化推给
//! webview 的 `CodeHealthViewPayload`。前端只渲染,不做聚合/排序/差异计算——
//! 这些全在 `view_model.rs`/`aggregate.rs`/`git_hotspots.rs`。
//! 形态仿 `extensions/usage/protocol.rs`(声明式比较 + 单槽)。

use super::view_model::{self, ChangeSummary, EmptyState};
use super::{CodeHealthCategory, WorkspaceState};
use dozer_codehealth::{FindingChange, FindingEvidence, FindingSeverity, HealthTier, rule_ids};
use dozer_model::LocaleStamp;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

pub const CODEHEALTH_PROTOCOL_VERSION: u32 = 1;

/// webview 加载后超过这个时长还没发 `ready` 就判失败,回落原生占位页。
pub const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// 面板分类(JSON 里的小写下划线形式)。
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CategoryKey {
    Overview,
    Structure,
    UiConsistency,
    ScanScope,
    Architecture,
}

impl From<CodeHealthCategory> for CategoryKey {
    fn from(c: CodeHealthCategory) -> Self {
        match c {
            CodeHealthCategory::Overview => CategoryKey::Overview,
            CodeHealthCategory::Structure => CategoryKey::Structure,
            CodeHealthCategory::UiConsistency => CategoryKey::UiConsistency,
            CodeHealthCategory::ScanScope => CategoryKey::ScanScope,
            CodeHealthCategory::Architecture => CategoryKey::Architecture,
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
    /// 扫描错误的稳定 key + 参数(spec §10);前端优先用它本地化。
    pub scan_error_key: Option<ErrorDto>,
    pub save_error: Option<String>,
    /// 保存错误的稳定 key + 参数(spec §10);前端优先用它本地化。
    pub save_error_key: Option<ErrorDto>,
}

/// 面板错误的稳定 key + 插值参数(spec §10)。旧 `*_error` 文案保留兼容,
/// 前端优先用本字段按 locale 渲染。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ErrorDto {
    /// 项目路径不可用。
    PathUnavailable,
    /// 扫描任务被取消:`{message}`。
    ScanCancelled { message: String },
    /// 扫描失败(来自服务/扫描器的原始消息,`{message}` 不翻译)。
    ScanFailed { message: String },
    /// 报告未保存,无法用于下次比较:`{message}`。
    SaveFailed { message: String },
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

/// 发现行的一条「成因」的结构化形态(spec §167):前端按 locale 渲染。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReasonDto {
    /// 指标变化:`{delta}{unit}`。
    Metric { delta: i64, unit: String },
    /// 近 30 天修改次数。
    RecentCommits { count: usize },
}

/// 一条发现行(总览优先处理、结构复杂度、UI 一致性共用)。**迁移期保留**
/// `severity_label`/`change_label`/`reasons`(已格式化文案)兼容旧 Host/Web,
/// 新增结构化字段供前端按 locale 渲染(spec §167)。
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
    /// 结构化成因(与 `reasons` 一一对应);前端优先用它本地化。
    pub reason_items: Vec<ReasonDto>,
    /// 标题的稳定 key(如架构风险 `arch.risk.cycle`);无则为 `None`。
    pub title_key: Option<&'static str>,
    /// 结构复杂度行的函数名与指标值(其它规则为 `None`)。
    pub symbol: Option<String>,
    pub metric: Option<i64>,
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
        /// 原始计数(前端本地化 `{delta}` 用;迁移期与 `*_text` 并存)。
        loc_delta: i64,
        functions_delta: i64,
    },
}

/// 概览页头和副标题的结构化片段(spec §167)。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GitLineDto {
    pub branch: Option<String>,
    pub head: Option<String>,
    pub dirty: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SummaryDto {
    pub loc: usize,
    pub functions: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ScopeSummaryDto {
    pub analyzed_files: usize,
    pub excluded_files: usize,
    pub skipped_files: usize,
    pub languages: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct OverviewBody {
    /// "main · abc123 · 有未提交改动";无 Git 时为 `None`。
    pub git_line: Option<String>,
    /// 结构化 Git 行片段;前端优先用它本地化。
    pub git: Option<GitLineDto>,
    pub tier: TierKey,
    pub tier_label: &'static str,
    pub summary: String,
    /// 结构化核心指标(行数 / 问题函数数);前端优先用它本地化。
    pub summary_parts: SummaryDto,
    pub change: ChangeDto,
    pub priorities: Vec<FindingRowDto>,
    /// "已分析 N 个文件 · … · 排除 N · 跳过 N";无摘要时为空串。
    pub scope_line: String,
    /// 结构化扫描可信度;无摘要时为 `None`。
    pub scope_parts: Option<ScopeSummaryDto>,
    /// 旧版报告提示的 key(前端本地化)。
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
    /// UI 分组文案的稳定 key(spec §167);前端优先用它本地化。
    pub group_key: &'static str,
    pub findings: Vec<FindingRowDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UiBody {
    pub applicable: bool,
    pub groups: Vec<UiGroup>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkipReasonKey {
    Ignored,
    UnsupportedLanguage,
    Generated,
    NonUtf8,
    ReadFailed,
    ParseFailed,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SkippedDto {
    pub path: String,
    pub reason: &'static str,
    /// 跳过原因 key(spec §167);前端优先用它本地化。
    pub reason_key: SkipReasonKey,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScanStatusKey {
    Complete,
    Partial,
    Failed,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LanguageDto {
    pub language: String,
    pub files: usize,
    pub analyzed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ScopeBody {
    pub status_label: &'static str,
    /// 扫描状态 key(spec §167);前端优先用它本地化。
    pub status: ScanStatusKey,
    pub analyzed_files: usize,
    pub excluded_files: usize,
    pub skipped_count: usize,
    pub languages_detail: String,
    /// 结构化语言明细;前端优先用它本地化。
    pub languages: Vec<LanguageDto>,
    pub duration_ms: u64,
    pub schema_version: u32,
    pub git_baseline: Option<String>,
    pub skipped: Vec<SkippedDto>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmptyStateKey {
    Scanning,
    NeverScanned,
    NoSupportedCode,
    PartialFailure,
    ScanFailed,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Empty {
        message: String,
        /// 空态 key(spec §167);前端优先用它本地化。
        empty_state: EmptyStateKey,
    },
    Overview(OverviewBody),
    Structure(StructureBody),
    UiConsistency(UiBody),
    ScanScope(ScopeBody),
    Architecture(super::arch_payload::ArchitectureBody),
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

/// `row_dto` 的具名参数(spec:参数多且同类型相邻,用具名字段结构体防顺序传错)。
pub(super) struct RowInput<'a> {
    pub id: String,
    pub title: String,
    pub path: &'a Path,
    pub line: usize,
    pub severity: FindingSeverity,
    pub change: Option<FindingChange>,
    pub reasons: Vec<String>,
    pub reason_items: Vec<ReasonDto>,
    pub title_key: Option<&'static str>,
    pub symbol: Option<String>,
    pub metric: Option<i64>,
}

impl<'a> RowInput<'a> {
    /// 无结构化附加信息(成因/标题 key/指标)的最小构造。
    pub(super) fn plain(
        id: String,
        title: String,
        path: &'a Path,
        line: usize,
        severity: FindingSeverity,
        change: Option<FindingChange>,
    ) -> Self {
        RowInput {
            id,
            title,
            path,
            line,
            severity,
            change,
            reasons: Vec::new(),
            reason_items: Vec::new(),
            title_key: None,
            symbol: None,
            metric: None,
        }
    }
}

pub(super) fn row_dto(r: RowInput<'_>) -> FindingRowDto {
    FindingRowDto {
        id: r.id,
        title: r.title,
        path: r.path.to_string_lossy().into_owned(),
        line: r.line,
        severity: r.severity.into(),
        severity_label: view_model::sev_label(r.severity),
        change: r.change.map(Into::into),
        change_label: r.change.map(view_model::change_label),
        reasons: r.reasons,
        reason_items: r.reason_items,
        title_key: r.title_key,
        symbol: r.symbol,
        metric: r.metric,
    }
}

fn overview_body(ws: &WorkspaceState, report: &dozer_codehealth::ProjectReport) -> OverviewBody {
    let git_line = ws.git().map(|git| {
        let branch = git.branch.clone().unwrap_or_else(|| "detached".to_string());
        let head = git.head.clone().unwrap_or_default();
        let mut s = format!("{branch} · {head}");
        if git.dirty {
            s.push_str(" · uncommitted changes");
        }
        s
    });
    let git = ws.git().map(|git| GitLineDto {
        branch: git.branch.clone(),
        head: git.head.clone(),
        dirty: git.dirty,
    });
    let scope_line = view_model::scope_summary(ws)
        .map(|s| {
            format!(
                "Analyzed {} files · {} · Excluded {} · Skipped {}",
                s.analyzed_files, s.languages, s.excluded_files, s.skipped_files
            )
        })
        .unwrap_or_default();
    let scope_parts = view_model::scope_summary(ws).map(|s| ScopeSummaryDto {
        analyzed_files: s.analyzed_files,
        excluded_files: s.excluded_files,
        skipped_files: s.skipped_files,
        languages: s.languages,
    });
    let change = match view_model::change_summary(report, ws.previous_report(), ws.diff()) {
        ChangeSummary::FirstScan => ChangeDto::FirstScan,
        ChangeSummary::HasChange(c) => ChangeDto::HasChange {
            loc_delta_text: view_model::signed_delta(c.loc_delta),
            functions_delta_text: view_model::signed_delta(c.functions_delta),
            new_risks: c.new_risks,
            resolved_risks: c.resolved_risks,
            worsened: c.worsened,
            improved: c.improved,
            loc_delta: c.loc_delta,
            functions_delta: c.functions_delta,
        },
    };
    let priorities = view_model::hotspot_rows(ws.hotspots(), 50)
        .into_iter()
        .map(|r| {
            row_dto(RowInput {
                reason_items: hotspot_reason_items(&r),
                reasons: r.reasons,
                id: r.finding_id,
                title: r.title,
                path: &r.path,
                line: r.line,
                severity: r.severity,
                change: Some(r.change),
                title_key: None,
                symbol: None,
                metric: None,
            })
        })
        .collect();
    OverviewBody {
        git_line,
        git,
        tier: report.overall_tier.into(),
        tier_label: view_model::tier_label(report.overall_tier),
        summary: format!(
            "Core code {} lines · {} functions with obvious structural issues",
            report.total_loc, report.critical_functions
        ),
        summary_parts: SummaryDto {
            loc: report.total_loc,
            functions: report.critical_functions,
        },
        change,
        priorities,
        scope_line,
        scope_parts,
        legacy_note: view_model::legacy_report_note(report.schema_version),
    }
}

/// 把热点成因转成结构化条目(spec §167),顺序与 `hotspot_reasons` 对齐:
/// change 标签由 `FindingRowDto.change`/`change_label` 单独携带,此处置
/// 指标与近 30 天修改两项。
fn hotspot_reason_items(r: &view_model::HotspotRow) -> Vec<ReasonDto> {
    let mut items = Vec::new();
    if let Some(delta) = r.metric_delta {
        items.push(ReasonDto::Metric {
            delta,
            unit: String::new(),
        });
    }
    if r.recent_commits > 0 {
        items.push(ReasonDto::RecentCommits {
            count: r.recent_commits,
        });
    }
    items
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
            row_dto(RowInput {
                id: f.id.clone(),
                title: format!(
                    "{} control-flow signal {}",
                    f.symbol.as_deref().unwrap_or("?"),
                    metric
                ),
                path: &f.path,
                line: f.start_line,
                severity: f.severity,
                change: finding_change(ws, &f.id),
                reasons: Vec::new(),
                reason_items: Vec::new(),
                title_key: None,
                symbol: f.symbol.clone(),
                metric: Some(metric as i64),
            })
        })
        .collect();
    StructureBody {
        metric_note: "Metric definition: control-flow signal (not standard cyclomatic complexity)",
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
    let rules: [(&'static str, &'static str, &str); 6] = [
        (
            "Hardcoded color",
            "ui.group.color_hardcode",
            rule_ids::COLOR_HARDCODE,
        ),
        (
            "Hardcoded spacing",
            "ui.group.spacing_hardcode",
            rule_ids::SPACING_HARDCODE,
        ),
        (
            "Hardcoded font",
            "ui.group.font_hardcode",
            rule_ids::FONT_HARDCODE,
        ),
        (
            "Component tree nesting depth",
            "ui.group.nesting_depth",
            rule_ids::NESTING_DEPTH,
        ),
        (
            "Event handler density",
            "ui.group.event_handler_density",
            rule_ids::EVENT_HANDLER_DENSITY,
        ),
        (
            "Duplicated component structure",
            "ui.group.duplicate_structure",
            rule_ids::DUPLICATE_STRUCTURE,
        ),
    ];
    let groups = rules
        .iter()
        .map(|(title, group_key, rule)| UiGroup {
            title,
            group_key,
            findings: report
                .findings
                .iter()
                .filter(|f| f.rule_id == *rule)
                .map(|f| {
                    row_dto(RowInput::plain(
                        f.id.clone(),
                        f.title.clone(),
                        &f.path,
                        f.start_line,
                        f.severity,
                        None,
                    ))
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
        dozer_codehealth::ScanStatus::Complete => "Complete",
        dozer_codehealth::ScanStatus::Partial => "Partially complete",
        dozer_codehealth::ScanStatus::Failed => "Failed",
    };
    let status = match scan.status {
        dozer_codehealth::ScanStatus::Complete => ScanStatusKey::Complete,
        dozer_codehealth::ScanStatus::Partial => ScanStatusKey::Partial,
        dozer_codehealth::ScanStatus::Failed => ScanStatusKey::Failed,
    };
    let languages_detail = scan
        .languages
        .iter()
        .map(|l| {
            if l.analyzed {
                format!("{} ({} files, structurally analyzed)", l.language, l.files)
            } else {
                format!("{} ({} files, counted only)", l.language, l.files)
            }
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let languages = scan
        .languages
        .iter()
        .map(|l| LanguageDto {
            language: l.language.clone(),
            files: l.files,
            analyzed: l.analyzed,
        })
        .collect();
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
                dozer_codehealth::SkipReason::Ignored => "Ignore rule",
                dozer_codehealth::SkipReason::UnsupportedLanguage => "Unsupported language",
                dozer_codehealth::SkipReason::Generated => "Generated file",
                dozer_codehealth::SkipReason::NonUtf8 => "Non-UTF-8",
                dozer_codehealth::SkipReason::ReadFailed => "Read failed",
                dozer_codehealth::SkipReason::ParseFailed => "Parse failed",
            },
            reason_key: match s.reason {
                dozer_codehealth::SkipReason::Ignored => SkipReasonKey::Ignored,
                dozer_codehealth::SkipReason::UnsupportedLanguage => {
                    SkipReasonKey::UnsupportedLanguage
                }
                dozer_codehealth::SkipReason::Generated => SkipReasonKey::Generated,
                dozer_codehealth::SkipReason::NonUtf8 => SkipReasonKey::NonUtf8,
                dozer_codehealth::SkipReason::ReadFailed => SkipReasonKey::ReadFailed,
                dozer_codehealth::SkipReason::ParseFailed => SkipReasonKey::ParseFailed,
            },
        })
        .collect();
    ScopeBody {
        status_label,
        status,
        analyzed_files: scan.analyzed_files,
        excluded_files: scan.excluded_files,
        skipped_count: scan.skipped_files.len(),
        languages_detail,
        languages,
        duration_ms: scan.duration_ms,
        schema_version: report.schema_version,
        git_baseline,
        skipped,
    }
}

fn empty_state(ws: &WorkspaceState) -> (EmptyStateKey, &'static str) {
    if ws.scanning() {
        return (EmptyStateKey::Scanning, "Scanning...");
    }
    match view_model::empty_state(ws) {
        EmptyState::NeverScanned => (
            EmptyStateKey::NeverScanned,
            "This project has not been scanned yet.",
        ),
        EmptyState::NoSupportedCode => (
            EmptyStateKey::NoSupportedCode,
            "No supported code was found.",
        ),
        EmptyState::PartialFailure => (
            EmptyStateKey::PartialFailure,
            "Scan partially completed; some files failed (see Scan Scope).",
        ),
        EmptyState::ScanFailed => (EmptyStateKey::ScanFailed, "Scan failed."),
    }
}

/// 当前该显示什么。**唯一权威实现**:`view.rs` 旧渲染删除后不再有第二份判定。
pub fn current_view_payload(ws: &WorkspaceState) -> CodeHealthViewPayload {
    let scan = ScanBar {
        scanning: ws.scanning(),
        scanned_at: ws.scanned_at_ms().map(view_model::format_ms),
        scan_error: ws.scan_error().map(str::to_owned),
        scan_error_key: ws.scan_error_key().cloned(),
        save_error: ws.save_error().map(str::to_owned),
        save_error_key: ws.save_error_key().cloned(),
    };
    let category: CategoryKey = ws.category().into();
    let body = match ws.report() {
        None => Body::Empty {
            message: empty_state(ws).1.to_string(),
            empty_state: empty_state(ws).0,
        },
        Some(report) => match category {
            CategoryKey::Overview => Body::Overview(overview_body(ws, report)),
            CategoryKey::Structure => Body::Structure(structure_body(ws, report)),
            CategoryKey::UiConsistency => Body::UiConsistency(ui_body(report)),
            CategoryKey::ScanScope => Body::ScanScope(scope_body(ws, report)),
            CategoryKey::Architecture => {
                Body::Architecture(super::arch_payload::architecture_body(ws, report))
            }
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
    /// 语言上下文(spec §8)。缺省(None)表示旧 Host,前端按 `en` 处理。
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub locale: Option<LocaleStamp>,
    pub payload: CodeHealthViewPayload,
}

pub fn encode_codehealth_push(
    revision: u64,
    locale: Option<LocaleStamp>,
    payload: CodeHealthViewPayload,
) -> String {
    serde_json::to_string(&CodeHealthPushEnvelope {
        protocol_version: CODEHEALTH_PROTOCOL_VERSION,
        revision,
        locale,
        payload,
    })
    .unwrap_or_else(|_| "null".to_string())
}

/// 代码健康度内容侧**面板级降级**状态(Task 09 / api-decisions §9.4)。
///
/// webview 生命周期、ready 归属、逐帧推送去重与 envelope `revision` 已收进
/// `extensions::codehealth::CodeHealthSurface`(共享宿主 Surface)。本结构只保留
/// **面板业务语义**:加载超时判定(`waiting_since`)与"超时/渲染失败 → 回落原生
/// 占位"(`failed`)。`ready` 由 Surface 持有,`observe_availability` 每帧由消费点
/// 传入,不再在本结构里记账。
#[derive(Default)]
pub struct WebviewPushState {
    /// webview 已进池但尚未 ready 的起点,用于判定加载超时。
    waiting_since: Option<Instant>,
    /// 加载失败/超时原因;`Some` 时内容区回落原生占位页(重试清除)。
    failed: Option<String>,
}

impl WebviewPushState {
    /// 每帧由消费点调用:`available` = webview 当前在池里;`ready` = Surface 已握手。
    /// 只有"在池但还没 ready"这段时间才计入加载超时。
    pub fn observe_availability(&mut self, available: bool, ready: bool, now: Instant) {
        if !available || ready {
            self.waiting_since = None;
            return;
        }
        let since = *self.waiting_since.get_or_insert(now);
        if self.failed.is_none() && now.duration_since(since) > READY_TIMEOUT {
            self.failed = Some("Code Health page failed to load".to_string());
        }
    }

    pub fn failed(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    pub fn set_failed(&mut self, reason: String) {
        self.failed = Some(reason);
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
            Body::Empty { message, .. } => assert_eq!(message, "Scanning..."),
            other => panic!("expected Empty, got {other:?}"),
        }
    }

    #[test]
    fn scan_error_without_report_reports_failure() {
        let mut ws = WorkspaceState::default();
        super::super::update(
            &mut ws,
            super::super::Message::Scanned(
                1,
                Err((
                    ErrorDto::ScanFailed {
                        message: "boom".into(),
                    },
                    "boom".into(),
                )),
            ),
        );
        let p = current_view_payload(&ws);
        assert_eq!(p.scan.scan_error.as_deref(), Some("boom"));
        match p.body {
            Body::Empty { message, .. } => assert_eq!(message, "Scan failed."),
            other => panic!("expected Empty, got {other:?}"),
        }
    }

    #[test]
    fn overview_first_scan_has_first_scan_change_and_tier() {
        let ws = ws_with(sample_report());
        let p = current_view_payload(&ws);
        assert_eq!(p.category, CategoryKey::Overview);
        assert_eq!(
            p.scan.scanned_at.as_deref(),
            Some("1970-01-01 00:00:00 UTC")
        );
        match p.body {
            Body::Overview(o) => {
                assert_eq!(o.tier_label, "Needs attention");
                assert_eq!(
                    o.summary,
                    "Core code 100 lines · 1 functions with obvious structural issues"
                );
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
                assert_eq!(
                    s.metric_note,
                    "Metric definition: control-flow signal (not standard cyclomatic complexity)"
                );
                assert_eq!(s.findings.len(), 1);
                assert_eq!(s.findings[0].title, "run control-flow signal 12");
                assert_eq!(s.findings[0].path, "src/a.rs");
                assert_eq!(s.findings[0].line, 7);
                assert_eq!(s.findings[0].severity_label, "Critical");
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
                assert_eq!(s.status_label, "Complete");
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

    /// spec §167 / §10:迁移期旧字段(预格式化文案)保留,同时追加语义字段,
    /// 前端按 locale 渲染。此测试钉住"旧+新并存"的兼容契约。
    #[test]
    fn overview_keeps_legacy_fields_and_appends_semantic_fields() {
        let ws = ws_with(sample_report());
        let json = serde_json::to_value(current_view_payload(&ws)).unwrap();
        let body = &json["body"];
        // 旧字段仍在(兼容旧 Web)。
        assert!(body["summary"].is_string());
        assert!(body["scope_line"].is_string());
        // 新语义字段。
        assert_eq!(body["summary_parts"]["loc"], sample_report().total_loc);
        assert!(
            body["summary_parts"]["functions"].is_number(),
            "summary_parts.functions 应为原始计数"
        );
        assert!(body["scope_parts"].is_null() || body["scope_parts"]["analyzed_files"].is_number());
        assert!(body["git"].is_null() || body["git"].is_object());
    }

    #[test]
    fn scan_scope_appends_status_key_and_language_rows() {
        let mut ws = ws_with(sample_report());
        super::super::update(
            &mut ws,
            super::super::Message::CategorySet(CodeHealthCategory::ScanScope),
        );
        let json = serde_json::to_value(current_view_payload(&ws)).unwrap();
        let body = &json["body"];
        assert_eq!(body["status_label"], "Complete");
        assert_eq!(body["status"], "complete");
        assert!(body["languages"].is_array());
    }

    #[test]
    fn empty_body_appends_empty_state_key() {
        let mut ws = WorkspaceState::default();
        super::super::update(
            &mut ws,
            super::super::Message::Scanned(
                1,
                Err((
                    ErrorDto::ScanFailed {
                        message: "boom".into(),
                    },
                    "boom".into(),
                )),
            ),
        );
        let json = serde_json::to_value(current_view_payload(&ws)).unwrap();
        assert_eq!(json["body"]["kind"], "empty");
        assert_eq!(json["body"]["message"], "Scan failed.");
        assert_eq!(json["body"]["empty_state"], "scan_failed");
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

    // ---- 面板级降级状态(超时/失败);推送/ready/revision 已移至 CodeHealthSurface ----

    fn payload() -> CodeHealthViewPayload {
        current_view_payload(&WorkspaceState::default())
    }

    /// Review Focus 2:一直不 ready 超时 → failed。
    #[test]
    fn never_ready_times_out_to_failed() {
        let mut s = WebviewPushState::default();
        let t0 = Instant::now();
        s.observe_availability(true, false, t0);
        assert_eq!(s.failed(), None);
        s.observe_availability(true, false, t0 + READY_TIMEOUT + Duration::from_millis(1));
        assert!(s.failed().is_some());
        s.clear_failed();
        assert_eq!(s.failed(), None);
    }

    #[test]
    fn ready_before_timeout_never_fails() {
        let mut s = WebviewPushState::default();
        let t0 = Instant::now();
        s.observe_availability(true, false, t0);
        // 已握手 → 不计超时。
        s.observe_availability(true, true, t0 + READY_TIMEOUT * 10);
        assert_eq!(s.failed(), None);
    }

    /// 不在池中(没挂载)不计超时——只有"在池但还没 ready"才算。
    #[test]
    fn unavailable_does_not_tick_the_timeout() {
        let mut s = WebviewPushState::default();
        let t0 = Instant::now();
        s.observe_availability(false, false, t0);
        s.observe_availability(false, false, t0 + READY_TIMEOUT * 10);
        assert_eq!(s.failed(), None);
    }

    #[test]
    fn envelope_carries_version_and_revision() {
        let json: serde_json::Value =
            serde_json::from_str(&encode_codehealth_push(5, None, payload())).unwrap();
        assert_eq!(json["protocol_version"], CODEHEALTH_PROTOCOL_VERSION);
        assert_eq!(json["revision"], 5);
        assert!(
            json["ui_locale"].is_null(),
            "无语言上下文时不内联 locale 字段"
        );
        assert!(json["payload"]["scan"].is_object());
    }
}
