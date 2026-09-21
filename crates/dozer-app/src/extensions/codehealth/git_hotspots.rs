//! Git 热点：近 30 天文件修改频率与风险排序（spec 2026-09-21「Git 快照与热点」）。
//!
//! 纯逻辑层（可单元测试，不碰 iced）：采集 HEAD/分支/dirty 元数据，单次批量
//! `git log --since=30.days --name-only --relative` 构建文件修改计数，再结合
//! 稳定发现 ID 的差异把“本轮新增/恶化 + 严重度 + 近期修改 + 指标增量 + 当前值”
//! 排成可解释的优先处理顺序。Git 不可用一律降级返回空/`None`，不让扫描失败。

use crate::delivery;
use dozer_codehealth::{Finding, FindingChange, FindingSeverity, GitSnapshot, diff_reports};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// 采集当前 Git 元数据。非 git 目录返回 `None`（detached HEAD 时 `head` 有值、
/// `branch` 为 `None`，仍返回 `Some`）。
pub fn git_snapshot(project_root: &Path) -> Option<GitSnapshot> {
    delivery::repo_root(project_root)?;
    let head = head_short_sha(project_root);
    let branch = delivery::branch(project_root);
    let dirty = delivery::is_dirty(project_root);
    Some(GitSnapshot {
        head,
        branch,
        dirty,
    })
}

fn head_short_sha(dir: &Path) -> Option<String> {
    let repo = git2::Repository::open(dir).ok()?;
    let head = repo.head().ok()?;
    let commit = head.peel_to_commit().ok()?;
    Some(commit.id().to_string().chars().take(7).collect())
}

/// 近 30 天各文件（相对项目根的规范化路径）的提交次数。Git 不可用/失败返回
/// `None`；成功但无提交返回 `Some(空)`。路径经 `--relative` 相对项目根，
/// 与扫描发现的相对路径同口径，可直接按路径匹配。
pub fn recent_churn(project_root: &Path) -> Option<HashMap<String, usize>> {
    let out = Command::new("git")
        .args([
            "log",
            "--since=30.days",
            "--name-only",
            "--relative",
            "--pretty=format:",
        ])
        .current_dir(project_root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut map: HashMap<String, usize> = HashMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        *map.entry(line.to_string()).or_insert(0) += 1;
    }
    Some(map)
}

/// 一条热点视图：一条当前发现 + 它的变化类别 + 近期修改次数 + 指标增量。
#[derive(Debug, Clone, PartialEq)]
pub struct HotspotView {
    pub finding: Finding,
    pub change: FindingChange,
    pub recent_commits: usize,
    /// 当前指标 − 上一轮指标（`None` = 无数值指标，如字面量发现）。
    pub metric_delta: Option<i64>,
}

impl HotspotView {
    /// 当前指标值（排序用；字面量发现无指标按 0 处理）。
    pub fn metric(&self) -> i64 {
        self.finding.evidence.metric_value().unwrap_or(0)
    }
}

/// 按 spec 排序元组对当前发现排热点顺序：新增/恶化优先 → Critical 高于 Watch
/// → 近期修改次数更多优先 → 指标增量更大优先 → 当前指标更高优先 → 路径+规则
/// ID 保证稳定并列顺序。只对当前仍存在的发现（New/Worsened/Persisting/
/// Improved）排序；`Resolved` 已不在当前列表，天然排除。
pub fn rank_hotspots(
    current: &[Finding],
    previous: &[Finding],
    churn: &HashMap<String, usize>,
) -> Vec<HotspotView> {
    let diff = diff_reports(previous, current);
    let prev_by_id: HashMap<&str, &Finding> = previous.iter().map(|f| (f.id.as_str(), f)).collect();
    let change_of = |f: &Finding| -> FindingChange {
        if diff.new.iter().any(|x| x.id == f.id) {
            FindingChange::New
        } else if diff.worsened.iter().any(|x| x.id == f.id) {
            FindingChange::Worsened
        } else if diff.improved.iter().any(|x| x.id == f.id) {
            FindingChange::Improved
        } else {
            FindingChange::Persisting
        }
    };

    let mut views: Vec<HotspotView> = current
        .iter()
        .map(|f| {
            let change = change_of(f);
            let metric_delta = match (f.evidence.metric_value(), prev_by_id.get(f.id.as_str())) {
                (Some(cur), Some(prev)) => prev.evidence.metric_value().map(|p| cur - p),
                _ => None,
            };
            let path_key = normalize_for_churn(&f.path);
            let recent_commits = churn.get(&path_key).copied().unwrap_or(0);
            HotspotView {
                finding: f.clone(),
                change,
                recent_commits,
                metric_delta,
            }
        })
        .collect();

    views.sort_by_key(hotspot_key);
    views
}

fn normalize_for_churn(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

type HotspotKey = (
    u8,
    u8,
    std::cmp::Reverse<usize>,
    std::cmp::Reverse<i64>,
    std::cmp::Reverse<i64>,
    std::path::PathBuf,
    String,
);

fn hotspot_key(v: &HotspotView) -> HotspotKey {
    (
        change_rank(v.change),
        severity_rank(v.finding.severity),
        std::cmp::Reverse(v.recent_commits),
        std::cmp::Reverse(v.metric_delta.unwrap_or(0)),
        std::cmp::Reverse(v.metric()),
        v.finding.path.clone(),
        v.finding.rule_id.clone(),
    )
}

fn change_rank(c: FindingChange) -> u8 {
    match c {
        FindingChange::New | FindingChange::Worsened => 0,
        FindingChange::Improved => 1,
        FindingChange::Persisting => 2,
        FindingChange::Resolved => 3,
    }
}

fn severity_rank(s: FindingSeverity) -> u8 {
    match s {
        FindingSeverity::Critical => 0,
        FindingSeverity::Watch => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_codehealth::{Applicability, FindingCategory, FindingEvidence};
    use std::path::PathBuf;

    fn finding(
        id: &str,
        rule: &str,
        path: &str,
        sev: FindingSeverity,
        metric: Option<i64>,
    ) -> Finding {
        Finding {
            id: id.to_string(),
            rule_id: rule.to_string(),
            category: FindingCategory::Structure,
            severity: sev,
            path: PathBuf::from(path),
            start_line: 1,
            symbol: Some("foo".into()),
            title: "foo".into(),
            evidence: match metric {
                Some(m) => FindingEvidence::Structure {
                    complexity_signal: m as usize,
                    loc: 5,
                    widget_nesting_depth: 0,
                    event_handler_count: 0,
                },
                None => FindingEvidence::Literal {
                    snippet: "x".into(),
                },
            },
            applicability: Applicability::Applicable,
        }
    }

    #[test]
    fn rank_puts_new_critical_before_persisting_watch() {
        let prev: Vec<Finding> = vec![finding(
            "watch",
            "structure/complexity_signal",
            "a.rs",
            FindingSeverity::Watch,
            Some(20),
        )];
        let cur: Vec<Finding> = vec![
            finding(
                "watch",
                "structure/complexity_signal",
                "a.rs",
                FindingSeverity::Watch,
                Some(20),
            ),
            finding(
                "new_crit",
                "structure/complexity_signal",
                "b.rs",
                FindingSeverity::Critical,
                Some(45),
            ),
        ];
        let churn = HashMap::new();
        let ranked = rank_hotspots(&cur, &prev, &churn);
        assert_eq!(ranked[0].finding.id, "new_crit");
        assert_eq!(ranked[0].change, FindingChange::New);
        assert_eq!(ranked[1].finding.id, "watch");
    }

    #[test]
    fn rank_prioritizes_higher_churn_within_same_change_and_severity() {
        let prev: Vec<Finding> = vec![];
        let cur: Vec<Finding> = vec![
            finding(
                "low",
                "structure/complexity_signal",
                "a.rs",
                FindingSeverity::Watch,
                Some(20),
            ),
            finding(
                "high",
                "structure/complexity_signal",
                "b.rs",
                FindingSeverity::Watch,
                Some(20),
            ),
        ];
        let mut churn = HashMap::new();
        churn.insert("a.rs".to_string(), 1);
        churn.insert("b.rs".to_string(), 9);
        let ranked = rank_hotspots(&cur, &prev, &churn);
        assert_eq!(ranked[0].finding.id, "high");
        assert_eq!(ranked[1].finding.id, "low");
    }

    #[test]
    fn worsened_metric_delta_is_positive() {
        let prev: Vec<Finding> = vec![finding(
            "x",
            "structure/complexity_signal",
            "a.rs",
            FindingSeverity::Watch,
            Some(20),
        )];
        let cur: Vec<Finding> = vec![finding(
            "x",
            "structure/complexity_signal",
            "a.rs",
            FindingSeverity::Critical,
            Some(41),
        )];
        let churn = HashMap::new();
        let ranked = rank_hotspots(&cur, &prev, &churn);
        assert_eq!(ranked[0].change, FindingChange::Worsened);
        assert_eq!(ranked[0].metric_delta, Some(21));
    }

    #[test]
    fn tie_break_is_stable_by_path_and_rule() {
        let prev: Vec<Finding> = vec![];
        let cur: Vec<Finding> = vec![
            finding(
                "b",
                "ui/color_hardcode",
                "z.rs",
                FindingSeverity::Watch,
                None,
            ),
            finding(
                "a",
                "ui/color_hardcode",
                "a.rs",
                FindingSeverity::Watch,
                None,
            ),
        ];
        let churn = HashMap::new();
        let ranked = rank_hotspots(&cur, &prev, &churn);
        let paths: Vec<&str> = ranked
            .iter()
            .map(|v| v.finding.path.to_str().unwrap())
            .collect();
        assert_eq!(paths, vec!["a.rs", "z.rs"]);
    }

    #[test]
    fn git_snapshot_none_outside_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert!(git_snapshot(dir.path()).is_none());
    }

    #[test]
    fn recent_churn_none_outside_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert!(recent_churn(dir.path()).is_none());
    }

    #[test]
    fn recent_churn_counts_file_edits_in_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let git = |args: &[&str]| {
            let st = Command::new("git")
                .args(args)
                .current_dir(repo)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
            assert!(st.status.success(), "git {args:?} failed");
        };
        git(&["init", "-q"]);
        std::fs::write(repo.join("hot.rs"), "fn a() {}").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "c1"]);
        std::fs::write(repo.join("hot.rs"), "fn a() {}\nfn b() {}").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "c2"]);

        let churn = recent_churn(repo).expect("git repo 应有 churn");
        assert_eq!(churn.get("hot.rs").copied(), Some(2));
    }
}
