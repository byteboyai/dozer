//! git CLI 薄封装（spec P1f D3/D4/D6）。
//! 全部同步阻塞——调用方负责放进 tokio 任务，不许在 UI 线程直呼。

use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

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
///
/// 渲染热路径已改用一次性的 [`rollup_dir_statuses`](O(M×depth)),这个
/// 逐目录 O(N×M) 版本保留下来作为语义参照——`rollup_dir_statuses` 的
/// 测试用它当 oracle 逐目录比对,确保重构不漂移。只在 test 构建可见。
#[cfg(test)]
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

/// 预聚合:每个目录 → 子孙改动里优先级最高的 `TreeState`(语义与
/// `dir_status` 完全一致,只是把 O(N×M) 的逐行全表扫描换成一次
/// O(M×depth) 的祖先传播)。文件树渲染时按路径 O(1) 查表。被忽略的
/// 条目不向上传播(同 `dir_status` 的既有语义);目录自身被忽略(精确
/// 路径在 `statuses` 里标 `ignored`)时整目录落 `Ignored`,且这条规则
/// 优先级恒高于任何聚合结果——分两遍写:第一遍只聚合非忽略条目,第二遍
/// 把"自身被忽略"的目录直接覆盖成 `Ignored`。这样不管 `statuses`(一个
/// `HashMap`,迭代顺序不固定)先遍历到自身条目还是子孙条目,结果都一样;
/// 合成一遍写、靠"先 or_insert 再比优先级"处理自身条目会让结果随机取决
/// 于遍历顺序(`Ignored` 优先级最低,子孙条目若晚于自身条目被处理,会把
/// `Ignored` 覆盖掉)。
pub fn rollup_dir_statuses(
    statuses: &HashMap<PathBuf, FileGitStatus>,
) -> HashMap<PathBuf, TreeState> {
    let mut dirs: HashMap<PathBuf, TreeState> = HashMap::new();
    for (path, st) in statuses {
        if st.ignored {
            continue;
        }
        let state = TreeState::from(*st);
        let mut cur = path.parent();
        while let Some(dir) = cur {
            let entry = dirs.entry(dir.to_path_buf()).or_insert(state);
            if state_priority(state) > state_priority(*entry) {
                *entry = state;
            }
            cur = dir.parent();
        }
    }
    for (path, st) in statuses {
        if st.ignored {
            dirs.insert(path.clone(), TreeState::Ignored);
        }
    }
    dirs
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

/// 返回仓库**全部** remote 的 fetch URL(去重,保持 `git remote -v`
/// 出现顺序)。`git remote -v` 每行形如 `origin  https://x.git (fetch)`,
/// 只取 `(fetch)` 方向避免 `(push)` 重复;完全没有 remote / 非 git 目录
/// → 空 `Vec`(语义上即"未设置")。
pub fn remote_url(repo: &Path) -> Vec<String> {
    let Ok(out) = Command::new("git")
        .args(["remote", "-v"])
        .current_dir(repo)
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    let mut urls = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if !line.contains("(fetch)") {
            continue;
        }
        let Some(url) = line.split_whitespace().nth(1) else {
            continue;
        };
        if seen.insert(url.to_string()) {
            urls.push(url.to_string());
        }
    }
    urls
}

/// 所有本地分支名(按 refs/heads 前缀,short 名)。非 git 仓库返回 None;
/// git 仓库但没有分支(空仓未提交)返回 Some(空 vec)。
pub fn local_branches(repo: &Path) -> Option<Vec<String>> {
    let out = git(
        repo,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
    )?;
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

/// 系统是否装了可用的 git——URL 签出 tab 提交前的轻量检测,不解析
/// 具体版本号,只看子进程能否成功跑起来。
pub fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// `git clone <url> <dest>`,鉴权完全委托系统已配置的 SSH agent/凭证
/// 管理器(不接 `RemoteCallbacks`,不做任何 Token 输入,见
/// `docs/superpowers/specs/2026-09-18-new-project-creation-design.md`)。
/// `dest` 必须还不存在(调用方在此之前已经校验过,见 `project_create`
/// 模块的 `validate_target_not_exists`),失败把 git 的 stderr 原样透传。
/// `url` 可能来自用户直接粘贴,也可能来自第三方 API 返回的 clone_url
/// (见 `git_accounts::list_repos`)——两者都不可信,用 `--` 结束选项解析,
/// 防止以 `-` 开头的伪造 URL 被 git 当成命令行选项吃掉(同 CVE-2017-1000117
/// 那一类问题)。
pub fn clone_repo(url: &str, dest: &Path) -> Result<(), String> {
    let out = Command::new("git")
        .arg("clone")
        .arg("--")
        .arg(url)
        .arg(dest)
        .output()
        .map_err(|e| format!("无法运行 git: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
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
        assert!(!is_dirty(&repo));
        std::fs::write(repo.join("a.txt"), "two\n").unwrap();
        assert!(is_dirty(&repo));
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
    fn rollup_dir_statuses_matches_dir_status_per_dir() {
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
            PathBuf::from("/r/src/main.rs"),
            FileGitStatus {
                kind: ChangeKind::Modified,
                staged: false,
                unstaged: true,
                ignored: false,
            },
        );
        // 被忽略文件不向上传播(否则 .DS_Store 会污染整个目录)。
        s.insert(
            PathBuf::from("/r/src/.DS_Store"),
            FileGitStatus {
                kind: ChangeKind::Modified,
                staged: false,
                unstaged: false,
                ignored: true,
            },
        );

        let rolled = rollup_dir_statuses(&s);
        // 对每个关心的目录,聚合结果必须与逐目录 dir_status 一致。
        for dir in ["/r", "/r/logo", "/r/src", "/r/docs"] {
            assert_eq!(
                rolled.get(Path::new(dir)).copied(),
                dir_status(Path::new(dir), &s),
                "dir {dir} 聚合不一致"
            );
        }
        // 跨层传播:子目录 /r/logo 的未加入版本改动滚到根 /r → 红。
        assert_eq!(
            rolled.get(Path::new("/r")).copied(),
            Some(TreeState::Untracked)
        );
        // 被忽略文件本身在表里,但 /r/src 的聚合不应被 .DS_Store 抬高。
        assert_eq!(
            rolled.get(Path::new("/r/src")).copied(),
            Some(TreeState::Modified)
        );
        // 无关目录无条目 → 调用方补成 Unchanged。
        assert_eq!(rolled.get(Path::new("/r/docs")).copied(), None);
    }

    #[test]
    fn rollup_dir_statuses_marks_dir_itself_ignored() {
        use std::path::{Path, PathBuf};
        let mut s = HashMap::new();
        s.insert(
            PathBuf::from("/r/out"),
            FileGitStatus {
                kind: ChangeKind::Modified,
                staged: false,
                unstaged: false,
                ignored: true,
            },
        );
        let rolled = rollup_dir_statuses(&s);
        assert_eq!(
            rolled.get(Path::new("/r/out")).copied(),
            Some(TreeState::Ignored)
        );
        // 忽略目录不向祖先传播。
        assert_eq!(rolled.get(Path::new("/r")).copied(), None);
    }

    /// 防漂移锚:目录自身被忽略、同时又有非忽略子孙把状态传播上来这个
    /// 组合(理论上不该从真实 `file_statuses()` 输出里出现,但函数自己
    /// 不能靠这个假设——两条规则在同一个 `HashMap` 上跑,不能让结果随
    /// 迭代顺序摇摆),自身 Ignored 必须恒赢,不能被子孙的更高优先级状态
    /// 覆盖。
    #[test]
    fn rollup_dir_statuses_self_ignored_wins_over_descendant() {
        use std::path::{Path, PathBuf};
        let mut s = HashMap::new();
        s.insert(
            PathBuf::from("/r/out"),
            FileGitStatus {
                kind: ChangeKind::Modified,
                staged: false,
                unstaged: false,
                ignored: true,
            },
        );
        s.insert(
            PathBuf::from("/r/out/keep.rs"),
            FileGitStatus {
                kind: ChangeKind::New,
                staged: false,
                unstaged: true,
                ignored: false,
            },
        );
        let rolled = rollup_dir_statuses(&s);
        assert_eq!(
            rolled.get(Path::new("/r/out")).copied(),
            Some(TreeState::Ignored),
            "目录自身被忽略时应恒为 Ignored,不受子孙状态或 HashMap 迭代顺序影响"
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
        let out = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap();
        let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
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
    fn remote_url_reads_origin() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://example.com/x.git"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert_eq!(
            remote_url(dir.path()),
            vec!["https://example.com/x.git".to_string()]
        );
    }

    #[test]
    fn remote_url_falls_back_to_first_remote_without_origin() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["remote", "add", "upstream", "https://example.com/y.git"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert_eq!(
            remote_url(dir.path()),
            vec!["https://example.com/y.git".to_string()]
        );
    }

    #[test]
    fn remote_url_none_without_any_remote() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert_eq!(remote_url(dir.path()), Vec::<String>::new());
    }

    #[test]
    fn remote_url_none_for_non_git_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(remote_url(dir.path()), Vec::<String>::new());
    }

    #[test]
    fn git_available_detects_system_git() {
        // 仓库里其它 git 相关测试都要求系统装了真 git 才能跑
        // (`mkrepo()` 本身就 shell 出真 git),这里断言同一个前提。
        assert!(git_available());
    }

    #[test]
    fn clone_repo_copies_local_source_repo() {
        let (_src_dir, src_repo) = mkrepo();
        let dest_parent = tempfile::tempdir().unwrap();
        let dest = dest_parent.path().join("cloned");
        clone_repo(&src_repo.to_string_lossy(), &dest).unwrap();
        assert!(dest.join(".git").exists());
        assert_eq!(
            std::fs::read_to_string(dest.join("a.txt")).unwrap(),
            "one\n"
        );
    }

    #[test]
    fn clone_repo_fails_on_missing_source() {
        let dest_parent = tempfile::tempdir().unwrap();
        let dest = dest_parent.path().join("cloned");
        let missing = dest_parent.path().join("does-not-exist");
        assert!(clone_repo(&missing.to_string_lossy(), &dest).is_err());
    }

    #[test]
    fn clone_repo_treats_dash_prefixed_url_as_a_path_not_an_option() {
        // 回归测试:确保 `--` 结束选项解析这道防线还在——若被误删,git 会
        // 把这个"URL"解析成 `--upload-pack` 选项而不是报"找不到仓库",
        // 这个测试就会失败(同 CVE-2017-1000117 那一类问题)。
        let dest_parent = tempfile::tempdir().unwrap();
        let dest = dest_parent.path().join("cloned");
        let err = clone_repo("--upload-pack=touch /tmp/dozer-test-pwned", &dest).unwrap_err();
        assert!(
            !dest.exists(),
            "伪造的 --upload-pack 参数不应该被当成选项接受"
        );
        assert!(!err.is_empty());
    }
}
