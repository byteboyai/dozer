//! 相邻两次报告之间的差异计算（spec 2026-09-21「统一发现项」）。
//!
//! 用稳定发现 ID 做哈希集合比较（O(n)），按规则证据的数值指标判定改善/恶化：
//! 同一 ID 当前指标更高 → `Worsened`，更低 → `Improved`，相同或无数值指标 →
//! `Persisting`；上一份有、当前无 → `Resolved`；当前有、上一份无 → `New`。

use crate::finding::Finding;
use std::collections::HashMap;

/// 一条发现项跨两次扫描的变化类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingChange {
    /// 本轮新增。
    New,
    /// 两轮都在，且指标/严重度无恶化或改善。
    Persisting,
    /// 两轮都在，当前指标更糟。
    Worsened,
    /// 两轮都在，当前指标更好。
    Improved,
    /// 上一轮有、本轮已消失。
    Resolved,
}

/// 相邻报告差异结果。`current` 侧的变化向量携带当前 [`Finding`]（用于渲染
/// “本次新增/恶化/改善”），`resolved` 携带上一轮的 [`Finding`]（用于“已解决”）。
#[derive(Debug, Default, Clone)]
pub struct ReportDiff {
    pub new: Vec<Finding>,
    pub persisting: Vec<Finding>,
    pub worsened: Vec<Finding>,
    pub improved: Vec<Finding>,
    pub resolved: Vec<Finding>,
}

impl ReportDiff {
    pub fn new_count(&self) -> usize {
        self.new.len()
    }
    pub fn persisting_count(&self) -> usize {
        self.persisting.len()
    }
    pub fn worsened_count(&self) -> usize {
        self.worsened.len()
    }
    pub fn improved_count(&self) -> usize {
        self.improved.len()
    }
    pub fn resolved_count(&self) -> usize {
        self.resolved.len()
    }

    /// 变化卡文案用的汇总：(新增, 已解决, 恶化, 改善, 持续)。
    pub fn summary(&self) -> (usize, usize, usize, usize, usize) {
        (
            self.new_count(),
            self.resolved_count(),
            self.worsened_count(),
            self.improved_count(),
            self.persisting_count(),
        )
    }
}

/// 比较两份报告的统一发现项，产出 [`ReportDiff`]。
/// `previous` 为空（首次扫描）时全部归入 `new`。
pub fn diff_reports(previous: &[Finding], current: &[Finding]) -> ReportDiff {
    let mut prev_by_id: HashMap<&str, &Finding> =
        previous.iter().map(|f| (f.id.as_str(), f)).collect();

    let mut out = ReportDiff::default();
    for cur in current {
        match prev_by_id.remove(cur.id.as_str()) {
            None => out.new.push(cur.clone()),
            Some(prev) => {
                let change = classify(prev, cur);
                match change {
                    FindingChange::New => unreachable!("已在 prev 里"),
                    FindingChange::Persisting => out.persisting.push(cur.clone()),
                    FindingChange::Worsened => out.worsened.push(cur.clone()),
                    FindingChange::Improved => out.improved.push(cur.clone()),
                    FindingChange::Resolved => unreachable!("cur 存在就不会 resolved"),
                }
            }
        }
    }
    // 剩余的 prev 全是“已解决”。
    out.resolved = prev_by_id.into_values().cloned().collect();
    out
}

/// 同 ID 的规则证据决定改善或恶化（spec「统一发现项」）。字面量发现没有
/// 数值指标，视为 `Persisting`。
fn classify(previous: &Finding, current: &Finding) -> FindingChange {
    match (
        previous.evidence.metric_value(),
        current.evidence.metric_value(),
    ) {
        (Some(prev), Some(cur)) if cur > prev => FindingChange::Worsened,
        (Some(prev), Some(cur)) if cur < prev => FindingChange::Improved,
        _ => FindingChange::Persisting,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::{Applicability, FindingCategory, FindingEvidence, FindingSeverity};
    use std::path::PathBuf;

    fn finding(id: &str, metric: i64) -> Finding {
        Finding {
            id: id.to_string(),
            rule_id: "structure/complexity_signal".into(),
            category: FindingCategory::Structure,
            severity: FindingSeverity::Watch,
            path: PathBuf::from("a.rs"),
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
    fn first_scan_marks_everything_new() {
        let cur = vec![finding("a", 41), finding("b", 20)];
        let diff = diff_reports(&[], &cur);
        assert_eq!(diff.new_count(), 2);
        assert_eq!(diff.resolved_count(), 0);
        assert_eq!(diff.worsened_count(), 0);
    }

    #[test]
    fn line_number_move_does_not_change_id() {
        // ID 不含行号/指标值，只含 rule+path+symbol+signature；这里用相同 id
        // 模拟“同一函数挪了几行”，指标不变 → Persisting。
        let prev = vec![finding("stable", 41)];
        let cur = vec![finding("stable", 41)];
        let diff = diff_reports(&prev, &cur);
        assert_eq!(diff.persisting_count(), 1);
        assert_eq!(diff.new_count(), 0);
        assert_eq!(diff.resolved_count(), 0);
    }

    #[test]
    fn worsened_and_improved_by_metric() {
        let prev = vec![finding("worse", 20), finding("better", 50)];
        let cur = vec![finding("worse", 41), finding("better", 30)];
        let diff = diff_reports(&prev, &cur);
        assert_eq!(diff.worsened_count(), 1);
        assert_eq!(diff.worsened[0].id, "worse");
        assert_eq!(diff.improved_count(), 1);
        assert_eq!(diff.improved[0].id, "better");
    }

    #[test]
    fn resolved_when_previous_id_disappears() {
        let prev = vec![finding("gone", 41), finding("kept", 20)];
        let cur = vec![finding("kept", 20)];
        let diff = diff_reports(&prev, &cur);
        assert_eq!(diff.resolved_count(), 1);
        assert_eq!(diff.resolved[0].id, "gone");
        assert_eq!(diff.persisting_count(), 1);
    }

    #[test]
    fn severity_crossing_is_metric_driven() {
        // 指标跨档（20 Watch → 41 Critical）由 metric 比较决定 Worsened。
        let prev = vec![finding("x", 20)];
        let cur = vec![finding("x", 41)];
        let diff = diff_reports(&prev, &cur);
        assert_eq!(diff.worsened_count(), 1);
    }

    #[test]
    fn literal_finding_without_metric_is_persisting() {
        fn lit(id: &str) -> Finding {
            Finding {
                id: id.to_string(),
                rule_id: "ui/color_hardcode".into(),
                category: FindingCategory::UiConsistency,
                severity: FindingSeverity::Watch,
                path: PathBuf::from("a.rs"),
                start_line: 1,
                symbol: None,
                title: "颜色硬编码".into(),
                evidence: FindingEvidence::Literal {
                    snippet: "Color::from_rgb(0.1, 0.2, 0.3)".into(),
                },
                applicability: Applicability::Applicable,
            }
        }
        let prev = vec![lit("color1")];
        let cur = vec![lit("color1")];
        let diff = diff_reports(&prev, &cur);
        assert_eq!(diff.persisting_count(), 1);
        assert_eq!(diff.worsened_count(), 0);
        assert_eq!(diff.improved_count(), 0);
    }
}
