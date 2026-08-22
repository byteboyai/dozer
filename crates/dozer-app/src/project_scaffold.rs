//! 项目 ensure/repair 的可扩展步骤机制(spec 2026-08-22)。README/`.dozer`
//! 目录/git 仓库三个同步步骤登记在这里；agent 历史数据导入是异步的
//! dozerd 请求，不在这个纯函数列表里，由调用方(`extensions::project`)
//! 单独跑完再拼进同一份 `ScaffoldReport`。

use crate::extensions::project::links;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub enum ScaffoldStepResult {
    AlreadyOk,
    Created(String),
    Failed(String),
}

pub struct ScaffoldStep {
    pub label: &'static str,
    pub run: fn(&Path) -> ScaffoldStepResult,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScaffoldReport {
    pub steps: Vec<(String, ScaffoldStepResult)>,
}

fn ensure_dozer_dir(repo: &Path) -> ScaffoldStepResult {
    let dir = repo.join(".dozer");
    if dir.is_dir() {
        return ScaffoldStepResult::AlreadyOk;
    }
    match std::fs::create_dir_all(&dir) {
        Ok(()) => ScaffoldStepResult::Created("已创建 .dozer 目录".into()),
        Err(e) => ScaffoldStepResult::Failed(e.to_string()),
    }
}

/// 判据复用 `links::discover_docs`——根目录里已经有任何"看起来像文档"的
/// 文件(readme/changelog/contributing/license 前缀)就跳过,不额外判断
/// 严格意义上的 `README.md`,避免误判"已经有 CHANGELOG 但没有 README"
/// 这种情况下重复造一份。
fn ensure_readme(repo: &Path) -> ScaffoldStepResult {
    let has_doc_file = links::discover_docs(repo)
        .iter()
        .any(|e| matches!(e.kind, links::LinkKind::File));
    if has_doc_file {
        return ScaffoldStepResult::AlreadyOk;
    }
    let name = repo
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "项目".to_string());
    let content = format!("# {name}\n");
    match std::fs::write(repo.join("README.md"), content) {
        Ok(()) => ScaffoldStepResult::Created("已创建 README.md".into()),
        Err(e) => ScaffoldStepResult::Failed(e.to_string()),
    }
}

fn ensure_git_repo(repo: &Path) -> ScaffoldStepResult {
    if repo.join(".git").exists() {
        return ScaffoldStepResult::AlreadyOk;
    }
    match crate::delivery::init_repo(repo) {
        Ok(()) => ScaffoldStepResult::Created("已初始化 git 仓库".into()),
        Err(e) => ScaffoldStepResult::Failed(e),
    }
}

/// 当前登记的同步步骤。以后其它 extension(todo/ssh/database 等)需要
/// 真正的 ensure/repair 逻辑时,在这里加一行调用自己模块里的函数——不
/// 需要改 `ScaffoldStep`/`ScaffoldStepResult` 这两个类型本身。
pub fn scaffold_steps() -> Vec<ScaffoldStep> {
    vec![
        ScaffoldStep {
            label: "缓存目录",
            run: ensure_dozer_dir,
        },
        ScaffoldStep {
            label: "README",
            run: ensure_readme,
        },
        ScaffoldStep {
            label: "git 仓库",
            run: ensure_git_repo,
        },
    ]
}

/// 顺序跑完全部同步步骤。这几步都可能阻塞(尤其 `ensure_git_repo` 会
/// shell 出子进程),调用方(`extensions::project::spawn_scaffold_run`,
/// Task 8)负责把这个函数整体包进 `tokio::task::spawn_blocking`,这里
/// 本身不做任何异步处理。
pub fn run_sync_steps(repo: &Path) -> Vec<(String, ScaffoldStepResult)> {
    scaffold_steps()
        .into_iter()
        .map(|step| (step.label.to_string(), (step.run)(repo)))
        .collect()
}

/// `ScaffoldReport` → "修复项目"按钮旁边展示的一行状态文字。全部
/// `AlreadyOk` 时给一句"一切正常"的简短总结,否则按 已修复/已是最新/失败
/// 三类分组列出各自涉及的步骤标签,失败项带上错误信息。
pub fn format_scaffold_report(report: &ScaffoldReport) -> String {
    let mut created = Vec::new();
    let mut already_ok = Vec::new();
    let mut failed = Vec::new();
    for (label, result) in &report.steps {
        match result {
            ScaffoldStepResult::Created(_) => created.push(label.as_str()),
            ScaffoldStepResult::AlreadyOk => already_ok.push(label.as_str()),
            ScaffoldStepResult::Failed(msg) => failed.push(format!("{label}({msg})")),
        }
    }
    if created.is_empty() && failed.is_empty() {
        return "一切正常".to_string();
    }
    let mut parts = Vec::new();
    if !created.is_empty() {
        parts.push(format!("已修复:{}", created.join("、")));
    }
    if !already_ok.is_empty() {
        parts.push(format!("已是最新:{}", already_ok.join("、")));
    }
    if !failed.is_empty() {
        parts.push(format!("失败:{}", failed.join("、")));
    }
    parts.join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_dozer_dir_creates_when_missing_and_reports_already_ok_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            ensure_dozer_dir(tmp.path()),
            ScaffoldStepResult::Created("已创建 .dozer 目录".into())
        );
        assert!(tmp.path().join(".dozer").is_dir());
        assert_eq!(ensure_dozer_dir(tmp.path()), ScaffoldStepResult::AlreadyOk);
    }

    #[test]
    fn ensure_readme_creates_when_no_doc_files_present() {
        let tmp = tempfile::tempdir().unwrap();
        let result = ensure_readme(tmp.path());
        assert!(matches!(result, ScaffoldStepResult::Created(_)));
        assert!(tmp.path().join("README.md").is_file());
        let content = std::fs::read_to_string(tmp.path().join("README.md")).unwrap();
        assert!(content.starts_with("# "));
    }

    #[test]
    fn ensure_readme_skips_when_readme_already_exists() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("README.md"), "既有内容").unwrap();
        assert_eq!(ensure_readme(tmp.path()), ScaffoldStepResult::AlreadyOk);
        // 不覆盖既有内容。
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("README.md")).unwrap(),
            "既有内容"
        );
    }

    #[test]
    fn ensure_readme_skips_when_other_doc_prefix_file_exists() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("CHANGELOG.md"), "x").unwrap();
        assert_eq!(ensure_readme(tmp.path()), ScaffoldStepResult::AlreadyOk);
        assert!(!tmp.path().join("README.md").exists());
    }

    #[test]
    fn ensure_git_repo_skips_when_dot_git_already_exists() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
        assert_eq!(ensure_git_repo(tmp.path()), ScaffoldStepResult::AlreadyOk);
    }

    #[test]
    fn run_sync_steps_covers_all_registered_steps_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let results = run_sync_steps(tmp.path());
        let labels: Vec<&str> = results.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, vec!["缓存目录", "README", "git 仓库"]);
    }

    #[test]
    fn format_scaffold_report_lists_created_already_ok_and_failed_separately() {
        let report = ScaffoldReport {
            steps: vec![
                (
                    "git 仓库".into(),
                    ScaffoldStepResult::Created("已初始化".into()),
                ),
                ("缓存目录".into(), ScaffoldStepResult::AlreadyOk),
                (
                    "agent 历史".into(),
                    ScaffoldStepResult::Failed("daemon 断开".into()),
                ),
            ],
        };
        let text = format_scaffold_report(&report);
        assert!(text.contains("已修复:git 仓库"));
        assert!(text.contains("已是最新:缓存目录"));
        assert!(text.contains("失败:agent 历史(daemon 断开)"));
    }

    #[test]
    fn format_scaffold_report_all_already_ok_shows_single_summary() {
        let report = ScaffoldReport {
            steps: vec![
                ("缓存目录".into(), ScaffoldStepResult::AlreadyOk),
                ("README".into(), ScaffoldStepResult::AlreadyOk),
            ],
        };
        assert_eq!(format_scaffold_report(&report), "一切正常");
    }
}
