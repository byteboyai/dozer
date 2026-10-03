//! 文件树的 git 状态装饰:把 `bytegit` 的状态条目映射成文件树名称颜色要用的档位。
//!
//! 这是 Dozer 自己的**呈现语义**(哪几种状态归为"未跟踪/新增/修改",目录取子孙中的最高档),
//! 不是 git 查询——查询在 `bytegit`。原先在 `delivery.rs`,P6 起随 `delivery.rs` 一起拆掉后搬到这里。

use bytegit::{ChangeKind as GitChange, Repo, StatusOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

/// 取整个仓库的文件级状态(含未跟踪与被忽略)。`repo` 必须是仓库根(见 `bytegit::Repo::open_exact`);
/// 非 git / 打开失败返回空。键是 `repo.join(相对路径)`。
pub fn file_statuses(repo: &Path) -> HashMap<PathBuf, FileGitStatus> {
    let mut map = HashMap::new();
    let Ok(git_repo) = Repo::open_exact(repo) else {
        return map;
    };
    let opts = StatusOptions {
        include_untracked: true,
        include_ignored: true,
        detect_renames: false,
    };
    let Ok(entries) = git_repo.status(opts) else {
        return map;
    };
    for entry in entries {
        let st = entry.state;
        if st.ignored {
            map.insert(
                repo.join(&entry.path),
                FileGitStatus {
                    kind: ChangeKind::Modified,
                    staged: false,
                    unstaged: false,
                    ignored: true,
                },
            );
            continue;
        }
        let staged = st.index.is_some();
        let unstaged = st.worktree.is_some();
        if !staged && !unstaged {
            continue; // 例如合并冲突:没有暂存/工作区改动标志,保持与旧实现一致地跳过
        }
        let has = |kind: GitChange| st.index == Some(kind) || st.worktree == Some(kind);
        let kind = if has(GitChange::Added) {
            ChangeKind::New
        } else if has(GitChange::Deleted) {
            ChangeKind::Deleted
        } else {
            ChangeKind::Modified
        };
        map.insert(
            repo.join(&entry.path),
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
    fn a_merge_conflicted_file_has_no_file_status() {
        // 冲突文件没有暂存/工作区改动标志:`file_statuses` 与迁移前一致地跳过它
        // (仓库整体是否算"有改动"是 `bytegit::Repo::is_dirty` 的事,那边有自己的测试)。
        let t = bytegit::testutil::TempRepo::new();
        t.make_conflict("c.txt");
        let m = file_statuses(t.path());
        assert!(
            !m.contains_key(&t.path().join("c.txt")),
            "冲突文件不应出现在文件状态里: {m:?}"
        );
    }
}
