//! Git 热点：近 30 天文件修改频率与风险排序（spec 2026-09-21「Git 快照与热点」）。
//!
//! 纯逻辑层（可单元测试，不碰 iced）：采集 HEAD/分支/dirty 元数据，单次批量
//! `git log --since=30.days --name-only --relative` 构建文件修改计数，再结合
//! 稳定发现 ID 的差异把“本轮新增/恶化 + 严重度 + 近期修改 + 指标增量 + 当前值”
//! 排成可解释的优先处理顺序。Git 不可用一律降级返回空/`None`，不让扫描失败。

use crate::delivery;
use bytegit::{Repo, StatusOptions};
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

/// HEAD 提交的前 7 位。和迁移前一致只认仓库根(见 `delivery::open_exact`)。
fn head_short_sha(dir: &Path) -> Option<String> {
    let commit = delivery::open_exact(dir)?.head().ok()?.commit?;
    Some(commit.short(7))
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

/// 当前 Git dirty（未提交改动 + 新增未跟踪）文件路径，相对**仓库根**、`/` 分隔。
/// Git 不可用/失败返回空列表（影响范围退化为仅看图变化，不让扫描失败）。
///
/// 注意：项目目录在仓库子目录时，这里的路径仍是相对仓库根的（迁移前同样如此），
/// 与 `recent_churn` 的 `--relative`（相对项目根）口径不同——见 bytegit P1 计划“待决事项”。
pub fn dirty_paths(project_root: &Path) -> Vec<String> {
    let Ok(repo) = Repo::discover(project_root) else {
        return Vec::new();
    };
    let opts = StatusOptions {
        include_untracked: true,
        include_ignored: false,
        // 与 `git status --porcelain` 的默认一致：重命名只报新路径。
        detect_renames: true,
    };
    let Ok(entries) = repo.status(opts) else {
        return Vec::new();
    };
    let mut paths: Vec<String> = entries
        .into_iter()
        .map(|e| e.path.to_string_lossy().replace('\\', "/"))
        .collect();
    paths.sort();
    paths.dedup();
    paths
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
    fn dirty_paths_empty_outside_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert!(dirty_paths(dir.path()).is_empty());
    }

    #[test]
    fn dirty_paths_lists_modified_and_untracked() {
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
        std::fs::write(repo.join("tracked.rs"), "fn a() {}").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "c1"]);

        std::fs::write(repo.join("tracked.rs"), "fn a() {}\nfn b() {}").unwrap();
        std::fs::write(repo.join("new.rs"), "fn c() {}").unwrap();

        let dirty = dirty_paths(repo);
        assert!(dirty.contains(&"tracked.rs".to_string()));
        assert!(dirty.contains(&"new.rs".to_string()));
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

    // ---- bytegit P1:dirty_paths / head_short_sha 迁移前后必须一致的口径 ----

    fn run_git(repo: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    fn repo_with_two_files() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        run_git(dir.path(), &["init", "-q"]);
        std::fs::write(dir.path().join("tracked.rs"), "fn a() {}\n").unwrap();
        std::fs::write(dir.path().join("gone.rs"), "fn g() {}\n").unwrap();
        run_git(dir.path(), &["add", "."]);
        run_git(dir.path(), &["commit", "-qm", "c1"]);
        dir
    }

    #[test]
    fn dirty_paths_clean_repo_is_empty() {
        let dir = repo_with_two_files();
        assert!(dirty_paths(dir.path()).is_empty());
    }

    #[test]
    fn dirty_paths_covers_modified_deleted_untracked_dirs_and_staged_new() {
        let dir = repo_with_two_files();
        let r = dir.path();
        std::fs::write(r.join("tracked.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        std::fs::remove_file(r.join("gone.rs")).unwrap();
        std::fs::create_dir_all(r.join("dir/sub")).unwrap();
        std::fs::write(r.join("dir/sub/x.rs"), "x").unwrap();
        std::fs::write(r.join("dir/y.rs"), "y").unwrap();
        std::fs::write(r.join("staged.rs"), "s").unwrap();
        run_git(r, &["add", "staged.rs"]);
        assert_eq!(
            dirty_paths(r),
            vec![
                "dir/sub/x.rs",
                "dir/y.rs",
                "gone.rs",
                "staged.rs",
                "tracked.rs"
            ]
        );
    }

    #[test]
    fn dirty_paths_excludes_ignored_files() {
        let dir = repo_with_two_files();
        let r = dir.path();
        std::fs::write(r.join(".gitignore"), "*.log\n").unwrap();
        run_git(r, &["add", ".gitignore"]);
        run_git(r, &["commit", "-qm", "ignore"]);
        std::fs::write(r.join("debug.log"), "x").unwrap();
        assert!(dirty_paths(r).is_empty());
    }

    #[test]
    fn dirty_paths_reports_only_the_new_path_of_a_rename() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        run_git(r, &["init", "-q"]);
        let body = "line one\nline two\nline three\nline four\nline five\n";
        std::fs::write(r.join("old.rs"), body).unwrap();
        run_git(r, &["add", "."]);
        run_git(r, &["commit", "-qm", "c1"]);
        run_git(r, &["mv", "old.rs", "renamed.rs"]);
        assert_eq!(dirty_paths(r), vec!["renamed.rs"]);
    }

    #[test]
    fn dirty_paths_handles_spaces_in_names() {
        let dir = repo_with_two_files();
        std::fs::write(dir.path().join("my file.rs"), "x").unwrap();
        assert_eq!(dirty_paths(dir.path()), vec!["my file.rs"]);
    }

    #[test]
    fn dirty_paths_from_a_subdirectory_are_relative_to_the_repo_root() {
        let dir = repo_with_two_files();
        let r = dir.path();
        std::fs::create_dir(r.join("pkg")).unwrap();
        std::fs::write(r.join("pkg/f.rs"), "x").unwrap();
        assert_eq!(dirty_paths(&r.join("pkg")), vec!["pkg/f.rs"]);
    }

    #[test]
    fn dirty_paths_reports_non_ascii_names_as_real_utf8() {
        // 迁移前读 porcelain 文本，非 ASCII 路径会被 git 转义成 "\346\226..." 八进制；
        // 迁移后是真实路径（有意的改进，不是等价迁移）。
        let dir = repo_with_two_files();
        std::fs::write(dir.path().join("说明.md"), "x").unwrap();
        assert_eq!(dirty_paths(dir.path()), vec!["说明.md"]);
    }

    #[test]
    fn head_short_sha_is_seven_chars_at_the_root_and_none_elsewhere() {
        let dir = repo_with_two_files();
        let r = dir.path();
        let sha = head_short_sha(r).expect("有提交应有 sha");
        assert_eq!(sha.len(), 7);
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
        // 只认仓库根:子目录、空仓库、非 git 目录都是 None
        std::fs::create_dir(r.join("pkg")).unwrap();
        assert_eq!(head_short_sha(&r.join("pkg")), None);
        let empty = tempfile::tempdir().unwrap();
        run_git(empty.path(), &["init", "-q"]);
        assert_eq!(head_short_sha(empty.path()), None);
        let plain = tempfile::tempdir().unwrap();
        assert_eq!(head_short_sha(plain.path()), None);
    }

    // ---- bytegit P3:recent_churn 迁移前后必须一致的口径 ----

    fn churn_git(repo: &Path, args: &[&str], when: Option<i64>) {
        let mut cmd = Command::new("git");
        cmd.args(args)
            .current_dir(repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            // 作者日期固定为 2000-01-01:`--since` 看的是**提交者**日期,作者日期再老也不影响。
            .env("GIT_AUTHOR_DATE", "946684800 +0000");
        if let Some(w) = when {
            cmd.env("GIT_COMMITTER_DATE", format!("{w} +0000"));
        }
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    fn churn_commit(repo: &Path, rel: &str, content: &str, when: Option<i64>) {
        let full = repo.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
        churn_git(repo, &["add", "."], None);
        churn_git(repo, &["commit", "-qm", "c"], when);
    }

    fn now_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    fn init_churn_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        churn_git(dir.path(), &["init", "-q"], None);
        dir
    }

    #[test]
    fn recent_churn_is_none_for_a_repo_without_commits() {
        let dir = init_churn_repo();
        assert!(recent_churn(dir.path()).is_none());
    }

    #[test]
    fn recent_churn_is_empty_not_none_when_every_commit_is_older_than_30_days() {
        let dir = init_churn_repo();
        churn_commit(dir.path(), "a.rs", "1", Some(now_secs() - 40 * 86_400));
        assert_eq!(recent_churn(dir.path()), Some(HashMap::new()));
    }

    #[test]
    fn recent_churn_counts_recent_commits_by_committer_date_not_author_date() {
        let dir = init_churn_repo();
        // 提交者日期 40 天前:不算。
        churn_commit(dir.path(), "old.rs", "x", Some(now_secs() - 40 * 86_400));
        // 作者日期是 2000 年,提交者日期是现在:算近期。
        churn_commit(dir.path(), "a.rs", "1", None);
        churn_commit(dir.path(), "a.rs", "2", None);
        let churn = recent_churn(dir.path()).unwrap();
        assert_eq!(churn.get("a.rs").copied(), Some(2));
        assert_eq!(churn.get("old.rs"), None);
    }

    /// 迁移前 `git log --since` 遇到(从 HEAD 往下数的)第一个过旧的提交就停止遍历:
    /// 当 HEAD 提交的提交者日期早于 30 天、而更深处的提交反而很新(时间戳不单调)时,
    /// 那些近期提交不会被计入。bytegit 的 `churn` 按时间全局排序,会把它们算上
    /// (更符合"近 30 天"的字面含义,有意的差异,见 P3 计划"待决事项" D5);
    /// 这条测试在 Task 5 里随之翻转。
    #[test]
    fn recent_churn_stops_at_an_old_head_even_if_deeper_commits_are_recent_before_migration() {
        let dir = init_churn_repo();
        churn_commit(dir.path(), "a.rs", "1", None);
        churn_commit(dir.path(), "a.rs", "2", None);
        churn_commit(dir.path(), "old.rs", "x", Some(now_secs() - 40 * 86_400));
        let churn = recent_churn(dir.path()).unwrap();
        assert_eq!(churn.get("a.rs"), None, "{churn:?}");
    }

    #[test]
    fn recent_churn_includes_the_root_commit_files() {
        let dir = init_churn_repo();
        churn_commit(dir.path(), "a.rs", "1", None);
        assert_eq!(
            recent_churn(dir.path()).unwrap().get("a.rs").copied(),
            Some(1)
        );
    }

    #[test]
    fn recent_churn_paths_are_relative_to_a_project_subdirectory_and_exclude_outside_files() {
        let dir = init_churn_repo();
        churn_commit(dir.path(), "top.rs", "1", None);
        churn_commit(dir.path(), "sub/a.rs", "1", None);
        churn_commit(dir.path(), "sub/a.rs", "2", None);
        let churn = recent_churn(&dir.path().join("sub")).unwrap();
        assert_eq!(churn.get("a.rs").copied(), Some(2), "{churn:?}");
        assert_eq!(churn.len(), 1, "仓库根里 sub 之外的文件不出现: {churn:?}");
    }

    #[test]
    fn recent_churn_ignores_merge_commits() {
        let dir = init_churn_repo();
        let r = dir.path();
        churn_commit(r, "a.rs", "a", None);
        churn_git(r, &["checkout", "-q", "-b", "other"], None);
        churn_commit(r, "b.rs", "b", None);
        churn_git(r, &["checkout", "-q", "-"], None);
        churn_commit(r, "c.rs", "c", None);
        churn_git(r, &["merge", "-q", "--no-ff", "other", "-m", "merge"], None);
        let churn = recent_churn(r).unwrap();
        assert_eq!(
            churn.get("b.rs").copied(),
            Some(1),
            "只算 other 上那一次: {churn:?}"
        );
        assert_eq!(churn.get("a.rs").copied(), Some(1));
        assert_eq!(churn.get("c.rs").copied(), Some(1));
    }

    #[test]
    fn recent_churn_counts_a_pure_rename_for_the_new_path_only() {
        let dir = init_churn_repo();
        let r = dir.path();
        churn_commit(r, "old.rs", "same content\nline 2\nline 3\n", None);
        churn_git(r, &["mv", "old.rs", "new.rs"], None);
        churn_git(r, &["commit", "-qm", "rename"], None);
        let churn = recent_churn(r).unwrap();
        assert_eq!(churn.get("new.rs").copied(), Some(1));
        assert_eq!(churn.get("old.rs").copied(), Some(1), "只有最初添加那一次");
    }

    #[test]
    fn recent_churn_counts_the_old_path_of_a_deletion() {
        let dir = init_churn_repo();
        let r = dir.path();
        churn_commit(r, "gone.rs", "1", None);
        churn_git(r, &["rm", "-q", "gone.rs"], None);
        churn_git(r, &["commit", "-qm", "delete"], None);
        assert_eq!(recent_churn(r).unwrap().get("gone.rs").copied(), Some(2));
    }

    /// 迁移前 `git log --name-only` 对非 ASCII 文件名输出 git 转义过的八进制(带引号),
    /// 所以真实的 UTF-8 路径永远匹配不上——churn 对这类文件恒为 0。bytegit 迁移后返回真实路径
    /// (有意的改进,同 P1 的 `dirty_paths`),这条测试在 Task 4 里随之翻转。
    #[test]
    fn recent_churn_keys_non_ascii_names_as_git_escaped_text_before_migration() {
        let dir = init_churn_repo();
        churn_commit(dir.path(), "文档.md", "x", None);
        let churn = recent_churn(dir.path()).unwrap();
        assert_eq!(churn.get("文档.md"), None, "{churn:?}");
        assert_eq!(churn.len(), 1, "{churn:?}");
        assert!(churn.keys().next().unwrap().contains("\\"), "{churn:?}");
    }
}
