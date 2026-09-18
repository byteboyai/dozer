//! 项目 ensure/repair 的可扩展步骤机制(spec 2026-08-22)。README/`.dozer`
//! 目录/git 仓库三个同步步骤登记在这里；agent 历史数据导入/会话总结补录
//! 是异步的 dozerd 请求，不在这个纯函数列表里，由调用方
//! (`extensions::project::spawn_repair_run`)驱动、逐步骤实时反馈进弹窗
//! (spec 2026-08-28)。

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

/// 重新扫一遍项目文档/Agent 记忆,把新发现的路径补进 `.dozer/links.json`
/// (`links::merge_rediscovered`——只追加、不删用户手动移除过的条目)。
/// `links::load_or_discover` 本身已经处理了"首次打开"的全量发现,这一步
/// 补的是"项目打开之后新增的文件"这种场景(比如后来才加的 AGENTS.md),
/// 普通打开/`load_or_discover` 不会重新扫,只有这个 ensure 步骤会。
fn ensure_project_docs_and_memory(repo: &Path) -> ScaffoldStepResult {
    let mut state = links::load_or_discover(repo);
    let added = links::merge_rediscovered(repo, &mut state);
    if added == 0 {
        return ScaffoldStepResult::AlreadyOk;
    }
    match links::save(repo, &state) {
        Ok(()) => ScaffoldStepResult::Created(format!("补充了 {added} 条项目文档/Agent 记忆")),
        Err(e) => ScaffoldStepResult::Failed(e.to_string()),
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
        ScaffoldStep {
            label: "项目文档/Agent 记忆",
            run: ensure_project_docs_and_memory,
        },
    ]
}

/// 顺序跑完全部同步步骤。`skip_git_init=true` 时跳过"git 仓库"这一步
/// (供"新建本地项目"对话框的"创建Git仓库"复选框未勾选时使用,见
/// `docs/superpowers/specs/2026-09-18-new-project-creation-design.md`
/// 「对既有共享代码的修正」一节)。这几步都可能阻塞(尤其 `ensure_git_repo`
/// 会 shell 出子进程),批量一次跑完的调用方(静默路径
/// `extensions::project::spawn_scaffold_run`)负责把这个函数整体包进
/// `tokio::task::spawn_blocking`——需要逐步骤实时反馈的路径
/// (`spawn_repair_run`)改成对每个 `scaffold_steps()` 元素单独
/// `spawn_blocking`,不调这个批量函数,因此不受这个参数影响。
pub fn run_sync_steps(repo: &Path, skip_git_init: bool) -> Vec<(String, ScaffoldStepResult)> {
    scaffold_steps()
        .into_iter()
        .filter(|step| !(skip_git_init && step.label == "git 仓库"))
        .map(|step| (step.label.to_string(), (step.run)(repo)))
        .collect()
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
        let results = run_sync_steps(tmp.path(), false);
        let labels: Vec<&str> = results.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(
            labels,
            vec!["缓存目录", "README", "git 仓库", "项目文档/Agent 记忆"]
        );
    }

    #[test]
    fn run_sync_steps_skips_git_step_when_requested() {
        let tmp = tempfile::tempdir().unwrap();
        let results = run_sync_steps(tmp.path(), true);
        assert!(
            !results.iter().any(|(label, _)| label == "git 仓库"),
            "skip_git_init=true 时不应该出现 git 仓库这一步"
        );
        assert!(!tmp.path().join(".git").exists());
    }

    #[test]
    fn run_sync_steps_runs_git_step_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        let results = run_sync_steps(tmp.path(), false);
        assert!(results.iter().any(|(label, _)| label == "git 仓库"));
        assert!(tmp.path().join(".git").exists());
    }

    #[test]
    fn ensure_project_docs_and_memory_reports_already_ok_on_first_open() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("README.md"), "").unwrap();
        // 首次打开:`load_or_discover` 已经把 README 全量发现并落盘,
        // `merge_rediscovered` 找不到任何新东西。
        assert_eq!(
            ensure_project_docs_and_memory(tmp.path()),
            ScaffoldStepResult::AlreadyOk
        );
    }

    #[test]
    fn ensure_project_docs_and_memory_reports_created_when_new_file_appears_later() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("README.md"), "").unwrap();
        // 第一次跑,建立 links.json(等价于项目首次打开)。
        ensure_project_docs_and_memory(tmp.path());
        // 之后项目根目录多了一个 AGENTS.md。
        std::fs::write(tmp.path().join("AGENTS.md"), "").unwrap();
        let result = ensure_project_docs_and_memory(tmp.path());
        assert_eq!(
            result,
            ScaffoldStepResult::Created("补充了 1 条项目文档/Agent 记忆".into())
        );
        let state = links::load(tmp.path()).unwrap();
        assert_eq!(state.memory.len(), 1);
    }
}
