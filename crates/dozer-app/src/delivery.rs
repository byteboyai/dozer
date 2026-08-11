//! 交付检测与沉淀：git CLI 薄封装（spec P1f D3/D4/D6）。
//! 全部同步阻塞——调用方负责放进 tokio 任务，不许在 UI 线程直呼。

use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const ACCEPTED_REF_PREFIX: &str = "refs/dozer/accepted/";

#[derive(Debug, Clone, PartialEq)]
pub struct FileChange {
    pub path: String,
    /// None = 二进制或未跟踪（numstat 给不出行数）
    pub added: Option<u32>,
    pub removed: Option<u32>,
}

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let out = git(dir, &["rev-parse", "--show-toplevel"])?;
    let line = out.lines().next()?.trim();
    (!line.is_empty()).then(|| PathBuf::from(line))
}

/// 工作区是否有任何改动(暂存或未暂存,不含被 `.gitignore` 排除的文件)。
pub fn is_dirty(repo: &Path) -> bool {
    let Ok(git_repo) = git2::Repository::open(repo) else {
        return false;
    };
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    git_repo
        .statuses(Some(&mut opts))
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

pub fn head_commit(repo: &Path) -> Option<String> {
    let out = git(repo, &["rev-parse", "HEAD"])?;
    let line = out.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}

/// 现存最大号 accepted ref：`(n, commit)`。
pub fn last_accepted(repo: &Path) -> Option<(u32, String)> {
    let out = git(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            ACCEPTED_REF_PREFIX,
        ],
    )?;
    out.lines()
        .filter_map(|l| {
            let (name, commit) = l.split_once(' ')?;
            let n: u32 = name.strip_prefix(ACCEPTED_REF_PREFIX)?.parse().ok()?;
            Some((n, commit.to_string()))
        })
        .max_by_key(|(n, _)| *n)
}

/// 变更清单：基线=最大 accepted ref（无则 HEAD）对工作区 numstat + 未跟踪文件。
pub fn changes(repo: &Path) -> Vec<FileChange> {
    let mut out = Vec::new();
    let base = last_accepted(repo)
        .map(|(n, _)| format!("{ACCEPTED_REF_PREFIX}{n}"))
        .or_else(|| head_commit(repo).map(|_| "HEAD".to_string()));
    if let Some(base) = base
        && let Some(numstat) = git(repo, &["diff", "--numstat", &base])
    {
        for line in numstat.lines() {
            let mut parts = line.split('\t');
            let (Some(a), Some(r), Some(p)) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            out.push(FileChange {
                path: p.to_string(),
                added: a.parse().ok(),
                removed: r.parse().ok(),
            });
        }
    }
    if let Some(status) = git(repo, &["status", "--porcelain"]) {
        for line in status.lines() {
            if let Some(p) = line.strip_prefix("?? ") {
                out.push(FileChange {
                    path: p.trim().to_string(),
                    added: None,
                    removed: None,
                });
            }
        }
    }
    out
}

/// 单文件相对验收基线(有上次沉淀取那个 ref,没有则 HEAD——与
/// `changes()` 用的同一套 base 解析)的 unified diff 原始文本。用
/// `diff_tree_to_workdir_with_index`(基线 tree vs 当前工作区,含已 stage
/// 的改动 + 未跟踪文件)。过长截断,截断阈值/提示文案与
/// `git_log::commit_detail` 的 `MAX_PATCH_CHARS` 处理方式类似但独立实现
/// (两个模块不共用代码,见设计文档"关键语义确认")。
const FILE_DIFF_MAX_CHARS: usize = 20_000;

pub fn file_diff(repo_path: &Path, path: &str) -> Result<String, String> {
    let repo = git2::Repository::open(repo_path).map_err(|e| e.message().to_string())?;
    let base_ref = last_accepted(repo_path)
        .map(|(n, _)| format!("{ACCEPTED_REF_PREFIX}{n}"))
        .or_else(|| head_commit(repo_path).map(|_| "HEAD".to_string()));
    let old_tree = match base_ref {
        Some(refname) => {
            let obj = repo
                .revparse_single(&refname)
                .map_err(|e| e.message().to_string())?;
            Some(obj.peel_to_tree().map_err(|e| e.message().to_string())?)
        }
        None => None,
    };
    let mut opts = git2::DiffOptions::new();
    opts.pathspec(path);
    opts.include_untracked(true);
    opts.recurse_untracked_dirs(true);
    opts.show_untracked_content(true);
    let diff = repo
        .diff_tree_to_workdir_with_index(old_tree.as_ref(), Some(&mut opts))
        .map_err(|e| e.message().to_string())?;

    let mut patch = String::new();
    let mut truncated = false;
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        if truncated {
            return true;
        }
        if patch.len() >= FILE_DIFF_MAX_CHARS {
            truncated = true;
            patch.push_str("\n… diff 过长,已截断显示\n");
            return true;
        }
        let prefix = match line.origin() {
            '+' | '-' | ' ' => line.origin().to_string(),
            _ => String::new(),
        };
        patch.push_str(&prefix);
        patch.push_str(&String::from_utf8_lossy(line.content()));
        true
    })
    .map_err(|e| e.message().to_string())?;

    if patch.is_empty() {
        return Err(format!("{path}: 没有可显示的改动"));
    }
    Ok(patch)
}

/// 通过·沉淀：脏工作区拒绝（沉淀物必须是提交，spec D6）。
pub fn accept(repo: &Path) -> Result<u32> {
    if is_dirty(repo) {
        bail!("有未提交变更，先让 agent 提交再沉淀");
    }
    let head = head_commit(repo).context("仓库没有任何提交")?;
    let n = last_accepted(repo).map(|(n, _)| n + 1).unwrap_or(1);
    let refname = format!("{ACCEPTED_REF_PREFIX}{n}");
    let ok = Command::new("git")
        .args(["update-ref", &refname, &head])
        .current_dir(repo)
        .status()
        .context("git update-ref")?
        .success();
    if !ok {
        bail!("写沉淀 ref 失败: {refname}");
    }
    Ok(n)
}

/// 文件树装饰用的 git 改动类型(P1h 三态,D2 起不再单独承担暂存/工作区
/// 区分——那部分挪进 [`FileGitStatus::staged`]/[`unstaged`]())。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    New,
    Modified,
    Deleted,
}

/// 单个文件的 git 状态:`kind` 决定名称颜色里"改动类型"那一档,`staged`/
/// `unstaged` 区分暂存与否(对应旧 porcelain 码里的 `MM`:部分暂存 + 又有
/// 新改动)。二者可同时为真。`ignored` 为真表示该路径被 `.gitignore` 忽略
/// (此时 `staged`/`unstaged` 恒为 false,`kind` 被忽略,名称颜色优先走
/// 忽略档)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileGitStatus {
    pub kind: ChangeKind,
    pub staged: bool,
    pub unstaged: bool,
    pub ignored: bool,
}

/// 用 `git2::Repository::statuses` 取代 shell 出 `git status --porcelain`
/// 文本解析(D1):`INDEX_*` 标志位=相对 HEAD 的暂存改动,`WT_*`=相对 index
/// 的工作区改动,原生区分,不用再猜双字符码语义。非 git / 打开失败返回空。
pub fn file_statuses(repo: &Path) -> HashMap<PathBuf, FileGitStatus> {
    let mut map = HashMap::new();
    let Ok(git_repo) = git2::Repository::open(repo) else {
        return map;
    };
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(true);
    let Ok(statuses) = git_repo.statuses(Some(&mut opts)) else {
        return map;
    };
    for entry in statuses.iter() {
        let Ok(rel) = entry.path() else {
            continue; // 非 UTF-8 路径,跳过而非崩(同旧版"格式意外跳过该行"精神)
        };
        if rel.is_empty() {
            continue;
        }
        let s = entry.status();
        if s.is_empty() {
            // 纯已跟踪且无改动:git 不放这类条目,出现空状态直接跳过。
            continue;
        }
        let ignored = s.contains(git2::Status::IGNORED);
        let staged = if ignored {
            false
        } else {
            s.intersects(
                git2::Status::INDEX_NEW
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::INDEX_RENAMED
                    | git2::Status::INDEX_TYPECHANGE,
            )
        };
        let unstaged = if ignored {
            false
        } else {
            s.intersects(
                git2::Status::WT_NEW
                    | git2::Status::WT_MODIFIED
                    | git2::Status::WT_DELETED
                    | git2::Status::WT_RENAMED
                    | git2::Status::WT_TYPECHANGE,
            )
        };
        if ignored {
            map.insert(
                repo.join(rel),
                FileGitStatus {
                    kind: ChangeKind::Modified,
                    staged: false,
                    unstaged: false,
                    ignored: true,
                },
            );
            continue;
        }
        if !staged && !unstaged {
            continue;
        }
        let kind = if s.intersects(git2::Status::INDEX_NEW | git2::Status::WT_NEW) {
            ChangeKind::New
        } else if s.intersects(git2::Status::INDEX_DELETED | git2::Status::WT_DELETED) {
            ChangeKind::Deleted
        } else {
            ChangeKind::Modified
        };
        map.insert(
            repo.join(rel),
            FileGitStatus {
                kind,
                staged,
                unstaged,
                ignored: false,
            },
        );
    }
    map
}

/// 文件树名称颜色要编码的 git 状态档位,按优先级从高到低:
/// `Untracked`(未加入版本,红)> `StagedNew`(加入版本未提交的新文件,绿)>
/// `Modified`(修改/删除未提交,青)> `Unchanged`(无改动/一般,灰)>
/// `Ignored`(被 `.gitignore` 忽略,弱灰)。`Unchanged` 对应不在
/// `git_statuses` 里的干净条目(调用方将 `None` 补成该档)。目录聚合取子孙
/// 中的**最高档**,让用户一眼先注意到没加入版本管理的文件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeState {
    Untracked,
    StagedNew,
    Modified,
    Unchanged,
    Ignored,
}

impl From<FileGitStatus> for TreeState {
    fn from(s: FileGitStatus) -> Self {
        if s.ignored {
            return TreeState::Ignored;
        }
        match s.kind {
            ChangeKind::New => {
                if s.staged {
                    TreeState::StagedNew
                } else {
                    TreeState::Untracked
                }
            }
            ChangeKind::Modified | ChangeKind::Deleted => TreeState::Modified,
        }
    }
}

/// 目录(含深层)的聚合 git 状态(rollup):收集所有子孙,取优先级最高的
/// `TreeState` 作为整个目录的颜色档(见 [`TreeState`] 的档位序)——
/// 未加入版本 > 加入版本未提交的新文件 > 修改未提交 > 一般 > 忽略。
/// 无任何在状态表里的子孙、自身也没被忽略时返回 `None`(调用方补成
/// `Unchanged`·灰)。被忽略的子孙不参与聚合(否则目录里顺带忽略的
/// `.DS_Store` 会往上带),忽略档只在**目录自身**被 `.gitignore` 忽略时
/// 触发。
pub fn dir_status(dir: &Path, statuses: &HashMap<PathBuf, FileGitStatus>) -> Option<TreeState> {
    if matches!(statuses.get(dir), Some(FileGitStatus { ignored: true, .. })) {
        return Some(TreeState::Ignored);
    }
    let mut best: Option<TreeState> = None;
    for (path, st) in statuses {
        if path == dir || !path.starts_with(dir) {
            continue;
        }
        if st.ignored {
            continue;
        }
        let state = TreeState::from(*st);
        if best
            .map(|b| state_priority(state) > state_priority(b))
            .unwrap_or(true)
        {
            best = Some(state);
        }
    }
    best
}

/// `TreeState` 的档位权重,越大优先级越高(用于目录聚合挑最高档)。
/// 档位序:Untracked(5)> StagedNew(4)> Modified(3)> Unchanged(2)> Ignored(1)。
fn state_priority(state: TreeState) -> u8 {
    match state {
        TreeState::Untracked => 5,
        TreeState::StagedNew => 4,
        TreeState::Modified => 3,
        TreeState::Unchanged => 2,
        TreeState::Ignored => 1,
    }
}

/// 当前分支名;非 git / 无提交 / detached HEAD 返回 None。
pub fn branch(repo: &Path) -> Option<String> {
    let git_repo = git2::Repository::open(repo).ok()?;
    let head = git_repo.head().ok()?;
    if !head.is_branch() {
        return None; // detached HEAD 或指向 tag 等非分支引用
    }
    head.shorthand().ok().map(str::to_string)
}

/// 所有本地分支名(按 refs/heads 前缀,short 名)。非 git 仓库返回 None;
/// git 仓库但没有分支(空仓未提交)返回 Some(空 vec)。
pub fn local_branches(repo: &Path) -> Option<Vec<String>> {
    let out = git(repo, &["for-each-ref", "--format=%(refname:short)", "refs/heads/"])?;
    Some(
        out.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
    )
}

/// 当前分支是否已有提交。空仓(刚 `git init` 未 commit 的 unborn 分支)——
/// HEAD 指不到任何提交对象,返回 `false`;已有一条或更多提交返回 `true`。
/// 非 git / detached HEAD 一律按 `false` 处理。分支菜单据此把其余分支
/// 置灰禁用(没有提交可切换的合理基线)。
pub fn current_branch_has_commits(repo: &Path) -> bool {
    let Ok(git_repo) = git2::Repository::open(repo) else {
        return false;
    };
    git_repo
        .head()
        .ok()
        .and_then(|h| h.peel_to_commit().ok())
        .is_some()
}

/// 切换到 `name` 指定分支(本地分支)。错误透传 git 的 stderr,便于展示给
/// 用户(如工作区有未提交改动导致的 checkout 失败)。
pub fn checkout_branch(repo: &Path, name: &str) -> Result<(), String> {
    let out = Command::new("git")
        .args(["checkout", name])
        .current_dir(repo)
        .output()
        .map_err(|e| format!("无法运行 git: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// 在项目根目录执行 `git init` 新建仓库。错误透传 git 的 stderr。
pub fn init_repo(repo: &Path) -> Result<(), String> {
    let out = Command::new("git")
        .args(["init"])
        .current_dir(repo)
        .output()
        .map_err(|e| format!("无法运行 git: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// 同仓库的一个 worktree(主或链接)。见 [`worktrees`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub name: String,
    pub path: PathBuf,
    pub branch: Option<String>,
    pub dirty: bool,
    pub is_current: bool,
    /// worktree 元数据还在(`.git/worktrees/<name>` 存在)但工作目录已被
    /// 手动删除/移走(没走 `git worktree remove`)。
    pub missing: bool,
}

/// 同仓库的其他 git worktree(D3)。主 worktree 靠 `commondir().parent()`
/// 推导(对主/链接 worktree 都成立:`commondir` 恒指向主仓库的 `.git`),
/// 链接 worktree 靠 `Repository::worktrees()` 列出(libgit2 只列链接的,
/// 不含主 worktree,所以主项要单独插入)。
pub fn worktrees(repo: &Path) -> Vec<WorktreeInfo> {
    let Ok(git_repo) = git2::Repository::open(repo) else {
        return Vec::new();
    };
    let mut out = Vec::new();

    let Some(main_workdir) = git_repo.commondir().parent().map(Path::to_path_buf) else {
        return Vec::new();
    };
    let main_name = main_workdir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let is_current_main = repo
        .canonicalize()
        .ok()
        .zip(main_workdir.canonicalize().ok())
        .is_some_and(|(a, b)| a == b);
    out.push(WorktreeInfo {
        name: main_name,
        branch: branch(&main_workdir),
        dirty: is_dirty(&main_workdir),
        is_current: is_current_main,
        missing: false,
        path: main_workdir,
    });

    let Ok(names) = git_repo.worktrees() else {
        return out;
    };
    for name in names.iter().filter_map(Result::ok).flatten() {
        let Ok(wt) = git_repo.find_worktree(name) else {
            continue;
        };
        let path = wt.path().to_path_buf();
        if !path.is_dir() {
            out.push(WorktreeInfo {
                name: name.to_string(),
                path,
                branch: None,
                dirty: false,
                is_current: false,
                missing: true,
            });
            continue;
        }
        let is_current = repo
            .canonicalize()
            .ok()
            .zip(path.canonicalize().ok())
            .is_some_and(|(a, b)| a == b);
        out.push(WorktreeInfo {
            name: name.to_string(),
            branch: branch(&path),
            dirty: is_dirty(&path),
            is_current,
            missing: false,
            path,
        });
    }
    out
}

/// spec D3 的"有变更"精确定义（纯函数，方便矩阵测试）。
pub fn delivery_pending(
    dirty: bool,
    head: Option<&str>,
    accepted: Option<&str>,
    last_turn_head: Option<&str>,
) -> bool {
    if dirty {
        return true;
    }
    match accepted {
        Some(a) => head != Some(a),
        None => head != last_turn_head,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// tempdir 里造一个带一次提交的真 git 仓库
    fn mkrepo() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            let st = Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
            assert!(st.status.success(), "git {args:?}: {st:?}");
        };
        git(&["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "c1"]);
        (dir, repo)
    }

    #[test]
    fn repo_root_and_head_and_dirty() {
        let (_d, repo) = mkrepo();
        let sub = repo.join("sub");
        std::fs::create_dir(&sub).unwrap();
        assert_eq!(
            repo_root(&sub).unwrap().canonicalize().unwrap(),
            repo.canonicalize().unwrap()
        );
        assert!(repo_root(std::path::Path::new("/")).is_none());
        assert!(head_commit(&repo).is_some());
        assert!(!is_dirty(&repo));
        std::fs::write(repo.join("a.txt"), "two\n").unwrap();
        assert!(is_dirty(&repo));
    }

    #[test]
    fn accept_writes_sequential_refs_and_refuses_dirty() {
        let (_d, repo) = mkrepo();
        assert!(last_accepted(&repo).is_none());
        assert_eq!(accept(&repo).unwrap(), 1);
        let (n, commit) = last_accepted(&repo).unwrap();
        assert_eq!(n, 1);
        assert_eq!(commit, head_commit(&repo).unwrap());
        // 再提交一轮 → v2
        std::fs::write(repo.join("b.txt"), "x\n").unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap()
        };
        git(&["add", "."]);
        git(&["commit", "-qm", "c2"]);
        assert_eq!(accept(&repo).unwrap(), 2);
        // 脏工作区拒绝
        std::fs::write(repo.join("a.txt"), "dirty\n").unwrap();
        assert!(accept(&repo).is_err());
    }

    #[test]
    fn changes_lists_modified_and_untracked_against_baseline() {
        let (_d, repo) = mkrepo();
        accept(&repo).unwrap(); // 基线 v1
        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap(); // 修改
        std::fs::write(repo.join("new.txt"), "n\n").unwrap(); // 未跟踪
        let ch = changes(&repo);
        let a = ch.iter().find(|c| c.path == "a.txt").expect("a.txt");
        assert_eq!(a.added, Some(1));
        assert_eq!(a.removed, Some(0));
        let n = ch.iter().find(|c| c.path == "new.txt").expect("new.txt");
        assert_eq!(n.added, None, "未跟踪无行数");
    }

    #[test]
    fn file_statuses_maps_modified_new_deleted() {
        let (_d, repo) = mkrepo(); // 含 a.txt 一次提交
        std::fs::write(repo.join("a.txt"), "changed\n").unwrap(); // 改
        std::fs::write(repo.join("new.txt"), "n\n").unwrap(); // 未跟踪
        // 造一个已跟踪再删的:先加提交 c,再删
        std::fs::write(repo.join("c.txt"), "c\n").unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
        };
        git(&["add", "c.txt"]);
        git(&["commit", "-qm", "c"]);
        std::fs::remove_file(repo.join("c.txt")).unwrap(); // 删已跟踪

        let m = file_statuses(&repo);
        assert_eq!(
            m.get(&repo.join("a.txt")).map(|s| s.kind),
            Some(ChangeKind::Modified)
        );
        assert_eq!(
            m.get(&repo.join("new.txt")).map(|s| s.kind),
            Some(ChangeKind::New)
        );
        assert_eq!(
            m.get(&repo.join("c.txt")).map(|s| s.kind),
            Some(ChangeKind::Deleted)
        );
        assert!(
            file_statuses(std::path::Path::new("/")).is_empty(),
            "非 git 空"
        );
    }

    #[test]
    fn file_statuses_distinguishes_staged_and_unstaged() {
        let (_d, repo) = mkrepo(); // a.txt 已提交一次
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
        };

        // 只暂存,不叠加工作区改动
        std::fs::write(repo.join("a.txt"), "staged-only\n").unwrap();
        git(&["add", "a.txt"]);
        let m = file_statuses(&repo);
        let a = m.get(&repo.join("a.txt")).expect("a.txt 应有状态");
        assert!(a.staged && !a.unstaged, "纯暂存: {a:?}");
        assert_eq!(a.kind, ChangeKind::Modified);

        // 再叠加一次未暂存改动(对应旧 porcelain 的 MM)
        std::fs::write(repo.join("a.txt"), "staged-only\nplus more\n").unwrap();
        let m2 = file_statuses(&repo);
        let a2 = m2.get(&repo.join("a.txt")).expect("a.txt 应有状态");
        assert!(a2.staged && a2.unstaged, "部分暂存+未暂存: {a2:?}");

        // 纯工作区改动:新建但没 git add 的文件
        std::fs::write(repo.join("new.txt"), "n\n").unwrap();
        let m3 = file_statuses(&repo);
        let n = m3.get(&repo.join("new.txt")).expect("new.txt 应有状态");
        assert!(!n.staged && n.unstaged, "未跟踪: {n:?}");
        assert_eq!(n.kind, ChangeKind::New);
    }

    #[test]
    fn file_statuses_marks_ignored_files() {
        let (_d, repo) = mkrepo();
        std::fs::write(repo.join(".gitignore"), "cache/\n*.log\n").unwrap();
        std::fs::create_dir(repo.join("cache")).unwrap();
        std::fs::write(repo.join("cache/x.bin"), "b\n").unwrap();
        std::fs::write(repo.join("app.log"), "l\n").unwrap();

        let m = file_statuses(&repo);
        let i = m.get(&repo.join("app.log")).expect("被忽略日志应有状态");
        assert!(i.ignored, "被忽略: {i:?}");
        assert!(
            !i.staged && !i.unstaged,
            "被忽略条目无暂存/未暂存语义: {i:?}"
        );
        // 忽略目录需要 git 递归忽略才能逐条放出;至少验证根级忽略文件命中。
        let _ = repo.join("cache/x.bin");
    }

    #[test]
    fn dir_status_marks_ignored_dir_only_when_dir_itself_ignored() {
        use std::path::{Path, PathBuf};
        let mut s = HashMap::new();
        // 目录自身被忽略(该精确路径在状态表里被标 ignored)→ 整目录走忽略档。
        s.insert(
            PathBuf::from("/r/out"),
            FileGitStatus {
                kind: ChangeKind::Modified,
                staged: false,
                unstaged: false,
                ignored: true,
            },
        );
        assert_eq!(
            dir_status(Path::new("/r/out"), &s),
            Some(TreeState::Ignored),
            "目录自身忽略"
        );

        // 仅一个被忽略的子孙不向上传播忽略(否则 .DS_Store 会污染整个目录)。
        let mut s2 = HashMap::new();
        s2.insert(
            PathBuf::from("/r/out/a.o"),
            FileGitStatus {
                kind: ChangeKind::Modified,
                staged: false,
                unstaged: false,
                ignored: true,
            },
        );
        assert!(
            dir_status(Path::new("/r/out"), &s2).is_none(),
            "只有被忽略子孙时不该标忽略"
        );
    }

    #[test]
    fn dir_status_aggregates_untracked_over_modified() {
        use std::path::{Path, PathBuf};
        let mut s = HashMap::new();
        s.insert(
            PathBuf::from("/r/logo/a.png"),
            FileGitStatus {
                kind: ChangeKind::New,
                staged: false,
                unstaged: true,
                ignored: false,
            },
        );
        s.insert(
            PathBuf::from("/r/logo/b.png"),
            FileGitStatus {
                kind: ChangeKind::New,
                staged: false,
                unstaged: true,
                ignored: false,
            },
        );
        s.insert(
            PathBuf::from("/r/src/main.rs"),
            FileGitStatus {
                kind: ChangeKind::Modified,
                staged: false,
                unstaged: true,
                ignored: false,
            },
        );
        // 纯未加入版本目录 → 红(未加入版本)
        assert_eq!(
            dir_status(Path::new("/r/logo"), &s),
            Some(TreeState::Untracked)
        );
        // 纯已修改目录 → 青(修改未提交)
        assert_eq!(
            dir_status(Path::new("/r/src"), &s),
            Some(TreeState::Modified)
        );
        // 同时含未加入版本与已修改 → 取最高档红(未加入版本)
        assert_eq!(dir_status(Path::new("/r"), &s), Some(TreeState::Untracked));
        assert!(dir_status(Path::new("/r/docs"), &s).is_none());
    }

    #[test]
    fn dir_status_priority_staged_new_over_modified() {
        use std::path::{Path, PathBuf};
        let mut s = HashMap::new();
        // 已暂存新文件(绿)
        s.insert(
            PathBuf::from("/r/src/a.rs"),
            FileGitStatus {
                kind: ChangeKind::New,
                staged: true,
                unstaged: false,
                ignored: false,
            },
        );
        // 已修改未提交(青)
        s.insert(
            PathBuf::from("/r/src/b.rs"),
            FileGitStatus {
                kind: ChangeKind::Modified,
                staged: false,
                unstaged: true,
                ignored: false,
            },
        );
        // 绿(加入版本未提交的新文件)优先于青(修改未提交)
        assert_eq!(
            dir_status(Path::new("/r/src"), &s),
            Some(TreeState::StagedNew)
        );
    }

    #[test]
    fn dir_status_untracked_beats_every_other() {
        use std::path::{Path, PathBuf};
        // 目录里同时有:未加入版本、已加入未提交、已修改、被忽略子孙。
        // 最该被关注的是未加入版本 → 红。
        let mut s = HashMap::new();
        s.insert(
            PathBuf::from("/r/mix/u.txt"),
            FileGitStatus {
                kind: ChangeKind::New,
                staged: false,
                unstaged: true,
                ignored: false,
            },
        );
        s.insert(
            PathBuf::from("/r/mix/s.rs"),
            FileGitStatus {
                kind: ChangeKind::New,
                staged: true,
                unstaged: false,
                ignored: false,
            },
        );
        s.insert(
            PathBuf::from("/r/mix/m.rs"),
            FileGitStatus {
                kind: ChangeKind::Modified,
                staged: false,
                unstaged: true,
                ignored: false,
            },
        );
        s.insert(
            PathBuf::from("/r/mix/.DS_Store"),
            FileGitStatus {
                kind: ChangeKind::Modified,
                staged: false,
                unstaged: false,
                ignored: true,
            },
        );
        assert_eq!(
            dir_status(Path::new("/r/mix"), &s),
            Some(TreeState::Untracked)
        );
    }

    #[test]
    fn branch_of_repo() {
        let (_d, repo) = mkrepo();
        let b = branch(&repo).expect("有分支");
        assert!(b == "main" || b == "master", "分支名: {b}");
        assert!(branch(std::path::Path::new("/")).is_none(), "非 git 无分支");
    }

    #[test]
    fn branch_none_when_head_detached() {
        let (_d, repo) = mkrepo();
        let head = head_commit(&repo).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap()
        };
        git(&["checkout", "-q", &head]); // detach HEAD 到当前 commit
        assert!(branch(&repo).is_none(), "detached HEAD 不该报出分支名");
    }

    #[test]
    fn worktrees_lists_main_and_linked() {
        // `mkrepo()` 的 `repo` 就是 tempdir 根,`repo.parent()` 是共享的系统
        // 临时目录,用它建 worktree 会在多次运行间撞名——这里给仓库再套一层
        // 唯一父目录,让 `repo.parent()/wt1|wt2` 每次都是全新的路径。
        let dir = tempfile::tempdir().unwrap();
        let repo2 = dir.path().join("repo");
        std::fs::create_dir_all(&repo2).unwrap();
        let git = |args: &[&str]| {
            let st = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo2)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
            assert!(st.status.success(), "git {args:?}: {st:?}");
        };
        git(&["init", "-q"]);
        std::fs::write(repo2.join("a.txt"), "one\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "c1"]);

        let parent = repo2.parent().unwrap();
        let wt1 = parent.join("wt1");
        let wt2 = parent.join("wt2");
        git(&["branch", "feature-a"]);
        git(&["worktree", "add", wt1.to_str().unwrap(), "feature-a"]);
        git(&["branch", "feature-b"]);
        git(&["worktree", "add", wt2.to_str().unwrap(), "feature-b"]);
        std::fs::write(wt1.join("a.txt"), "dirty in wt1\n").unwrap(); // wt1 弄脏

        let list = worktrees(&repo2);
        assert_eq!(list.len(), 3, "主 worktree + 2 个链接: {list:?}");

        let main = list.iter().find(|w| w.is_current).expect("应有当前项");
        assert_eq!(
            main.path.canonicalize().unwrap(),
            repo2.canonicalize().unwrap()
        );
        assert!(
            main.branch.as_deref() == Some("main") || main.branch.as_deref() == Some("master"),
            "主分支名: {:?}",
            main.branch
        );

        let w1 = list
            .iter()
            .find(|w| w.path.canonicalize().unwrap() == wt1.canonicalize().unwrap())
            .expect("应找到 wt1");
        assert_eq!(w1.branch.as_deref(), Some("feature-a"));
        assert!(w1.dirty);
        assert!(!w1.is_current);
        assert!(!w1.missing);

        let w2 = list
            .iter()
            .find(|w| w.path.canonicalize().unwrap() == wt2.canonicalize().unwrap())
            .expect("应找到 wt2");
        assert_eq!(w2.branch.as_deref(), Some("feature-b"));
        assert!(!w2.dirty);
    }

    #[test]
    fn worktrees_marks_missing_when_directory_deleted() {
        // 同 `worktrees_lists_main_and_linked`:套一层唯一父目录避免共享临时
        // 目录撞名。
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let st = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
            assert!(st.status.success(), "git {args:?}: {st:?}");
        };
        git(&["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "c1"]);

        let parent = repo.parent().unwrap();
        let wt = parent.join("wt-missing");
        git(&["branch", "gone"]);
        git(&["worktree", "add", wt.to_str().unwrap(), "gone"]);
        std::fs::remove_dir_all(&wt).unwrap(); // 手动删掉目录,不走 `git worktree remove`

        let list = worktrees(&repo);
        let missing = list
            .iter()
            .find(|w| w.name == "wt-missing")
            .expect("git 元数据还在,不该从列表消失");
        assert!(missing.missing);
        assert!(missing.branch.is_none());
    }

    #[test]
    fn file_diff_new_file_shows_all_added_lines() {
        let (dir, repo) = mkrepo();
        std::fs::write(dir.path().join("new.txt"), "a\nb\n").unwrap();
        let diff = file_diff(&repo, "new.txt").unwrap();
        assert!(diff.contains("+a"));
        assert!(diff.contains("+b"));
    }

    #[test]
    fn file_diff_modified_file_shows_plus_minus_lines() {
        let (dir, repo) = mkrepo();
        // `mkrepo()` 已经提交了内容为 "one\n" 的 a.txt,替换成不同内容,
        // 应该同时看到删除旧行(-)和新增行(+)。
        std::fs::write(dir.path().join("a.txt"), "two\n").unwrap();
        let diff = file_diff(&repo, "a.txt").unwrap();
        assert!(diff.contains("-one"));
        assert!(diff.contains("+two"));
    }

    #[test]
    fn file_diff_truncates_when_too_long() {
        let (dir, repo) = mkrepo();
        let big: String = (0..5000).map(|i| format!("line{i}\n")).collect();
        std::fs::write(dir.path().join("big.txt"), big).unwrap();
        let diff = file_diff(&repo, "big.txt").unwrap();
        assert!(diff.contains("已截断显示"));
        assert!(diff.len() < 21_000, "截断后不应远超 FILE_DIFF_MAX_CHARS");
    }

    #[test]
    fn delivery_pending_matrix() {
        // 脏 → 恒 pending
        assert!(delivery_pending(true, Some("h"), None, Some("h")));
        // 有沉淀 ref：HEAD 偏离才 pending
        assert!(delivery_pending(false, Some("h2"), Some("h1"), None));
        assert!(!delivery_pending(false, Some("h1"), Some("h1"), None));
        // 无 ref：与上回合 HEAD 比
        assert!(delivery_pending(false, Some("h2"), None, Some("h1")));
        assert!(!delivery_pending(false, Some("h1"), None, Some("h1")));
    }
}
