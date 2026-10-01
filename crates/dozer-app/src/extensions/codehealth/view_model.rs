//! 纯 view model：把 `WorkspaceState` 的原始数据格式化成渲染层直接消费的
//! 文案/结构（spec Task 7）。渲染层（`view.rs`）只消费这里已经算好的数据，
//! 不再自己拼字符串或做分支判断。

use super::{HotspotView, WorkspaceState};
use dozer_codehealth::{FindingChange, FindingSeverity, HealthTier, ProjectReport, ReportDiff};

/// 变化卡数据。`FirstScan` = 没有上一份可比较报告，展示“首次扫描”而非伪造零。
#[derive(Debug, Clone, PartialEq)]
pub enum ChangeSummary {
    FirstScan,
    HasChange(ChangeCard),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChangeCard {
    pub loc_delta: i64,
    pub functions_delta: i64,
    /// 新增风险 = 本轮新增 + 本轮恶化。
    pub new_risks: usize,
    pub resolved_risks: usize,
    pub worsened: usize,
    pub improved: usize,
}

/// 从当前/上一报告与差异算出变化卡。没有上一份或没有差异时返回 `FirstScan`
/// （禁止用零冒充没有变化）。
pub fn change_summary(
    current: &ProjectReport,
    previous: Option<&ProjectReport>,
    diff: Option<&ReportDiff>,
) -> ChangeSummary {
    let (Some(diff), Some(prev)) = (diff, previous) else {
        return ChangeSummary::FirstScan;
    };
    ChangeSummary::HasChange(ChangeCard {
        loc_delta: current.total_loc as i64 - prev.total_loc as i64,
        functions_delta: current.total_functions as i64 - prev.total_functions as i64,
        new_risks: diff.new_count() + diff.worsened_count(),
        resolved_risks: diff.resolved_count(),
        worsened: diff.worsened_count(),
        improved: diff.improved_count(),
    })
}

/// 旧报告（schema_version < 2）提示文案。新版报告返回 `None`。
pub fn legacy_report_note(schema_version: u32) -> Option<&'static str> {
    if schema_version < 2 {
        Some("旧版报告，重新扫描可查看变化与范围")
    } else {
        None
    }
}

/// 带符号的增量文案（+/- 或“新增/已解决”语义），保证不只靠颜色表达。
pub fn signed_delta(delta: i64) -> String {
    if delta > 0 {
        format!("+{delta}")
    } else {
        delta.to_string()
    }
}

/// 热点单行格式化。
#[derive(Debug, Clone, PartialEq)]
pub struct HotspotRow {
    pub finding_id: String,
    pub title: String,
    pub path: std::path::PathBuf,
    pub line: usize,
    pub severity: FindingSeverity,
    pub change: FindingChange,
    pub reasons: Vec<String>,
}

pub fn hotspot_rows(hotspots: &[HotspotView], limit: usize) -> Vec<HotspotRow> {
    hotspots
        .iter()
        .take(limit)
        .map(|h| HotspotRow {
            finding_id: h.finding.id.clone(),
            title: h.finding.title.clone(),
            path: h.finding.path.clone(),
            line: h.finding.start_line,
            severity: h.finding.severity,
            change: h.change,
            reasons: hotspot_reasons(h),
        })
        .collect()
}

/// 热点排序原因展开成文本（spec「UI 将排序原因展开为文本」）。
pub fn hotspot_reasons(h: &HotspotView) -> Vec<String> {
    let mut reasons = Vec::new();
    let change = match h.change {
        FindingChange::New => "本轮新增",
        FindingChange::Worsened => "本轮恶化",
        FindingChange::Improved => "本轮改善",
        FindingChange::Persisting => "持续存在",
        FindingChange::Resolved => "已解决",
    };
    reasons.push(change.to_string());
    if let Some(delta) = h.metric_delta {
        reasons.push(format!(
            "指标 {}{}",
            signed_delta(delta),
            metric_unit(&h.finding)
        ));
    }
    if h.recent_commits > 0 {
        reasons.push(format!("近 30 天修改 {} 次", h.recent_commits));
    }
    reasons
}

/// 指标值单位后缀：字面量发现没有数值指标，不显示单位。
fn metric_unit(_finding: &dozer_codehealth::Finding) -> &'static str {
    ""
}

/// 扫描范围摘要（扫描范围分类 + 总览“扫描可信度”共用）。
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeSummary {
    pub status_label: &'static str,
    pub analyzed_files: usize,
    pub excluded_files: usize,
    pub skipped_files: usize,
    pub languages: String,
    pub duration_ms: u64,
    pub schema_version: u32,
    pub git_head: Option<String>,
}

pub fn scope_summary(ws: &WorkspaceState) -> Option<ScopeSummary> {
    let report = ws.report()?;
    let scan = &report.scan;
    let status_label = match scan.status {
        dozer_codehealth::ScanStatus::Complete => "完整",
        dozer_codehealth::ScanStatus::Partial => "部分完成",
        dozer_codehealth::ScanStatus::Failed => "失败",
    };
    let languages = scan
        .languages
        .iter()
        .map(|l| {
            if l.analyzed {
                l.language.clone()
            } else {
                format!("{}（仅统计）", l.language)
            }
        })
        .collect::<Vec<_>>()
        .join(" · ");
    Some(ScopeSummary {
        status_label,
        analyzed_files: scan.analyzed_files,
        excluded_files: scan.excluded_files,
        skipped_files: scan.skipped_files.len(),
        languages,
        duration_ms: scan.duration_ms,
        schema_version: report.schema_version,
        git_head: ws.git().and_then(|g| g.head.clone()),
    })
}

/// UI 一致性检测是否适用于当前项目：只有语义分析了 Rust（可能用 iced）才
/// 适用；未识别到 Rust/iced 时返回 `false`，渲染层显示“不适用”而非“健康”。
pub fn ui_consistency_applicable(report: &ProjectReport) -> bool {
    report.scan.frameworks.iter().any(|f| f == "iced")
}

/// 空状态分类（spec「空状态拆成」）。渲染层据此选文案，不把几种情况合并。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyState {
    /// 尚未扫描。
    NeverScanned,
    /// 扫描过但没有任何受支持代码。
    NoSupportedCode,
    /// 扫描过，部分文件失败。
    PartialFailure,
    /// 扫描失败。
    ScanFailed,
}

/// 生成“交给 Agent 分析”的只读诊断上下文（spec「Agent 诊断上下文」）。
/// 只携带位置、规则、证据、变化、热点原因与验收目标，**不**包含自动编辑
/// 命令、不包含项目根外的敏感信息。返回多行文本，送入 agent 输入区由用户
/// 审阅后主动发送。
pub fn analyze_finding_text(
    finding: &dozer_codehealth::Finding,
    change: Option<FindingChange>,
    reasons: &[String],
    scope: Option<&ScopeSummary>,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "请分析这条代码健康发现：{}:{}",
        finding.path.display(),
        finding.start_line
    ));
    lines.push(format!("规则：{}", finding.rule_id));
    if let Some(sym) = &finding.symbol {
        lines.push(format!("符号：{sym}"));
    }
    match &finding.evidence {
        dozer_codehealth::FindingEvidence::Structure {
            complexity_signal,
            loc,
            ..
        } => {
            lines.push(format!(
                "证据：控制流信号 {complexity_signal}，函数 {loc} 行"
            ));
        }
        dozer_codehealth::FindingEvidence::Literal { snippet } => {
            lines.push(format!("证据：{snippet}"));
        }
        dozer_codehealth::FindingEvidence::Duplicate { occurrences } => {
            lines.push(format!("证据：同结构出现 {occurrences} 次"));
        }
        dozer_codehealth::FindingEvidence::NestingDepth { depth } => {
            lines.push(format!("证据：组件树嵌套深度 {depth}"));
        }
        dozer_codehealth::FindingEvidence::EventHandlers { count } => {
            lines.push(format!("证据：事件回调 {count} 个"));
        }
        dozer_codehealth::FindingEvidence::ArchitectureCycle { node_ids } => {
            lines.push(format!("证据：循环依赖，涉及 {} 个模块", node_ids.len()));
            for id in node_ids {
                lines.push(format!("  - {id}"));
            }
        }
        dozer_codehealth::FindingEvidence::ArchitectureHub { node_id, fan_out } => {
            lines.push(format!("证据：依赖枢纽 {node_id}，扇出 {fan_out}"));
        }
        dozer_codehealth::FindingEvidence::ArchitectureBoundary {
            edge_id,
            from_layer,
            to_layer,
        } => {
            lines.push(format!(
                "证据：越层依赖 {from_layer} → {to_layer}（边 {edge_id}）"
            ));
        }
    }
    if let Some(c) = change {
        lines.push(format!("变化：{}", change_label_text(c)));
    }
    if !reasons.is_empty() {
        lines.push(format!("热点原因：{}", reasons.join("、")));
    }
    if let Some(s) = scope {
        lines.push(format!(
            "扫描范围：已分析 {} 个文件（{}）",
            s.analyzed_files, s.languages
        ));
    }
    lines.push("验收目标：相关指标下降且无新增同类发现。".to_string());
    lines.join("\n")
}

fn change_label_text(c: FindingChange) -> &'static str {
    match c {
        FindingChange::New => "本轮新增",
        FindingChange::Worsened => "本轮恶化",
        FindingChange::Improved => "本轮改善",
        FindingChange::Persisting => "持续存在",
        FindingChange::Resolved => "已解决",
    }
}

/// 归类空状态。`report` 为 `None` → 尚未扫描（或扫描失败时由 `scan_error` 优先）。
pub fn empty_state(ws: &WorkspaceState) -> EmptyState {
    if ws.scan_error().is_some() {
        return EmptyState::ScanFailed;
    }
    let Some(report) = ws.report() else {
        return EmptyState::NeverScanned;
    };
    match report.scan.status {
        dozer_codehealth::ScanStatus::Failed => EmptyState::ScanFailed,
        dozer_codehealth::ScanStatus::Partial => EmptyState::PartialFailure,
        dozer_codehealth::ScanStatus::Complete => {
            if report.scan.analyzed_files == 0 {
                EmptyState::NoSupportedCode
            } else {
                // 有受支持代码时不是“空状态”，调用方按正常内容渲染。
                EmptyState::NeverScanned
            }
        }
    }
}

pub(super) fn tier_label(tier: HealthTier) -> &'static str {
    match tier {
        HealthTier::Healthy => "健康",
        HealthTier::Watch => "需要关注",
        HealthTier::Critical => "警戒",
    }
}

pub(super) fn sev_label(s: FindingSeverity) -> &'static str {
    match s {
        FindingSeverity::Critical => "警戒",
        FindingSeverity::Watch => "关注",
    }
}

pub(super) fn change_label(c: FindingChange) -> &'static str {
    match c {
        FindingChange::New => "本轮新增",
        FindingChange::Worsened => "本轮恶化",
        FindingChange::Improved => "本轮改善",
        FindingChange::Persisting => "持续存在",
        FindingChange::Resolved => "已解决",
    }
}

/// 同 `git_log.rs::format_commit_time`:展示 UTC,不引入时区库。
pub(super) fn format_ms(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let days = secs / 86_400;
    let secs_of_day = secs % 86_400;
    let (h, m, s) = (
        secs_of_day / 3600,
        (secs_of_day / 60) % 60,
        secs_of_day % 60,
    );
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02} UTC")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_codehealth::{
        Applicability, Finding, FindingCategory, FindingEvidence, SCHEMA_VERSION, ScanMetadata,
        ScanStatus,
    };

    fn report_with(loc: usize, functions: usize, findings: Vec<Finding>) -> ProjectReport {
        ProjectReport {
            schema_version: SCHEMA_VERSION,
            scan: ScanMetadata {
                status: ScanStatus::Complete,
                analyzed_files: 1,
                languages: vec![dozer_codehealth::LanguageSummary {
                    language: "rust".into(),
                    files: 1,
                    analyzed: true,
                }],
                ..ScanMetadata::default()
            },
            git: None,
            findings,
            architecture: dozer_codehealth::ArchitectureReport::not_applicable(),
            total_loc: loc,
            total_functions: functions,
            critical_functions: 0,
            scale_tier: dozer_codehealth::HealthTier::Healthy,
            density_tier: dozer_codehealth::HealthTier::Healthy,
            overall_tier: dozer_codehealth::HealthTier::Healthy,
            functions: vec![],
            color_findings: vec![],
            color_tier: dozer_codehealth::HealthTier::Healthy,
            spacing_findings: vec![],
            spacing_tier: dozer_codehealth::HealthTier::Healthy,
            font_findings: vec![],
            font_tier: dozer_codehealth::HealthTier::Healthy,
            distinct_color_values: 0,
            distinct_spacing_values: 0,
            duplicate_clusters: vec![],
            duplicate_cluster_tier: dozer_codehealth::HealthTier::Healthy,
            nesting_depth_tier: dozer_codehealth::HealthTier::Healthy,
            event_handler_tier: dozer_codehealth::HealthTier::Healthy,
            ui_tier: dozer_codehealth::HealthTier::Healthy,
        }
    }

    fn finding(id: &str, metric: i64) -> Finding {
        Finding {
            id: id.into(),
            rule_id: "structure/complexity_signal".into(),
            category: FindingCategory::Structure,
            severity: FindingSeverity::Watch,
            path: std::path::PathBuf::from("a.rs"),
            start_line: 1,
            symbol: Some("foo".into()),
            title: "foo".into(),
            evidence: FindingEvidence::Structure {
                complexity_signal: metric as usize,
                loc: 5,
                widget_nesting_depth: 0,
                event_handler_count: 0,
            },
            applicability: Applicability::Applicable,
        }
    }

    #[test]
    fn change_summary_first_scan_when_no_previous() {
        let cur = report_with(100, 5, vec![]);
        assert_eq!(change_summary(&cur, None, None), ChangeSummary::FirstScan);
    }

    #[test]
    fn change_summary_computes_signed_deltas() {
        let prev = report_with(90, 4, vec![finding("a", 20)]);
        let cur = report_with(110, 6, vec![finding("a", 41), finding("b", 30)]);
        let diff = dozer_codehealth::diff_reports(&prev.findings, &cur.findings);
        match change_summary(&cur, Some(&prev), Some(&diff)) {
            ChangeSummary::HasChange(c) => {
                assert_eq!(c.loc_delta, 20);
                assert_eq!(c.functions_delta, 2);
                assert_eq!(c.worsened, 1);
                assert_eq!(c.new_risks, 2, "新增(1) + 恶化(1)");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn legacy_report_note_for_schema_v1() {
        assert_eq!(
            legacy_report_note(1),
            Some("旧版报告，重新扫描可查看变化与范围")
        );
        assert_eq!(legacy_report_note(2), None);
    }

    #[test]
    fn signed_delta_has_plus_sign_for_positive() {
        assert_eq!(signed_delta(31), "+31");
        assert_eq!(signed_delta(-5), "-5");
        assert_eq!(signed_delta(0), "0");
    }

    #[test]
    fn ui_consistency_not_applicable_without_rust() {
        let mut r = report_with(0, 0, vec![]);
        r.scan.languages = vec![];
        assert!(!ui_consistency_applicable(&r));
    }

    #[test]
    fn ui_consistency_applicable_with_rust() {
        let mut r = report_with(0, 0, vec![]);
        r.scan.frameworks = vec!["iced".into()];
        assert!(ui_consistency_applicable(&r));
    }

    #[test]
    fn analyze_text_contains_location_and_acceptance_target() {
        let f = finding("a", 41);
        let text = analyze_finding_text(
            &f,
            Some(FindingChange::Worsened),
            &["近 30 天修改 3 次".into()],
            None,
        );
        assert!(text.contains("a.rs:1"), "应包含位置：{text}");
        assert!(text.contains("控制流信号 41"), "应包含证据：{text}");
        assert!(text.contains("本轮恶化"), "应包含变化：{text}");
        assert!(text.contains("验收目标"), "应包含验收目标：{text}");
    }

    #[test]
    fn analyze_text_has_no_auto_edit_commands_and_no_absolute_path() {
        let f = finding("a", 41);
        let text = analyze_finding_text(&f, None, &[], None);
        for banned in [
            "edit",
            "Edit",
            "修改文件",
            "替换",
            "write_file",
            "apply_patch",
        ] {
            assert!(
                !text.contains(banned),
                "不应包含自动编辑命令 `{banned}`：{text}"
            );
        }
        // 路径是相对项目根的规范化路径，不应出现绝对路径分隔的机器目录。
        assert!(!text.contains("/Users/"), "不应包含绝对路径：{text}");
    }

    #[test]
    fn format_ms_matches_expected_layout() {
        assert_eq!(format_ms(1_789_891_086_991), "2026-09-20 07:58:06 UTC");
        assert_eq!(format_ms(0), "1970-01-01 00:00:00 UTC");
    }

    #[test]
    fn tier_labels_include_text_for_all_states() {
        assert_eq!(tier_label(HealthTier::Healthy), "健康");
        assert_eq!(tier_label(HealthTier::Watch), "需要关注");
        assert_eq!(tier_label(HealthTier::Critical), "警戒");
    }
}
