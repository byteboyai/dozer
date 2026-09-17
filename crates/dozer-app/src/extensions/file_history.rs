//! 单个文件的历史提交对比:右键 Files 面板文件行"查看此文件历史"弹窗的
//! 数据层。跟 `git_log.rs` 的区别是那边是"整个仓库的提交图",这里是
//! "只关心一个文件路径,且要跟*当前工作目录实时内容*比较,不是跟某个
//! commit 的父提交比较"。见
//! `docs/superpowers/specs/2026-09-17-file-history-popup-design.md`。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 该文件在某一次提交里的记录——按 `build()` 的 revwalk 顺序(时间倒序)
/// 收集,只包含真正改动过这个文件的提交。
#[derive(Debug, Clone)]
pub struct FileHistoryEntry {
    pub oid: git2::Oid,
    pub short_sha: String,
    /// commit message 首行。
    pub summary: String,
    pub author: Option<String>,
    /// author time,Unix 秒。
    pub time: i64,
}

#[derive(Debug, Clone)]
pub struct FileHistorySnapshot {
    pub repo_path: PathBuf,
    /// 仓库相对路径,git 查询与磁盘读写都用它(`repo_path.join(file_path)`
    /// 拼出实际路径)。
    pub file_path: PathBuf,
    pub entries: Vec<FileHistoryEntry>,
}

/// `build()` 没有显式指定 `max_count` 时的默认窗口,同
/// `git_log::DEFAULT_MAX_COMMITS` 的量级(见该常量文档)。
pub const DEFAULT_MAX_COUNT: usize = 200;

/// 一次「查看此文件历史」的目标——右键哪个文件、属于哪个项目/仓库。
#[derive(Debug, Clone)]
pub struct FileHistoryTarget {
    pub project_id: i64,
    pub repo_path: PathBuf,
    /// 仓库相对路径,`build`/`diff_against_current`/`rollback_to` 都吃它。
    pub file_path: PathBuf,
}

/// 弹窗全部状态。整体以 `Option<State>` 挂在顶层 `App`(同
/// `project_link_menu` 的既有模式)——`None` 表示弹窗未打开。
#[derive(Default)]
pub struct State {
    target: Option<FileHistoryTarget>,
    snapshot: Option<Result<FileHistorySnapshot, String>>,
    selected: Option<git2::Oid>,
    /// 已经查过的 commit 各自的 diff 结果,按 oid 缓存,来回切换选中项不用
    /// 重复计算。
    diff_cache: HashMap<git2::Oid, Result<String, String>>,
    rollback_pending: Option<git2::Oid>,
    rollback_error: Option<String>,
}

impl State {
    pub fn new(target: FileHistoryTarget) -> Self {
        Self {
            target: Some(target),
            ..Self::default()
        }
    }

    pub fn target(&self) -> Option<&FileHistoryTarget> {
        self.target.as_ref()
    }

    pub fn snapshot(&self) -> Option<&Result<FileHistorySnapshot, String>> {
        self.snapshot.as_ref()
    }

    pub fn selected(&self) -> Option<git2::Oid> {
        self.selected
    }

    pub fn diff_for(&self, oid: git2::Oid) -> Option<&Result<String, String>> {
        self.diff_cache.get(&oid)
    }

    pub fn rollback_pending(&self) -> Option<git2::Oid> {
        self.rollback_pending
    }

    pub fn rollback_error(&self) -> Option<&str> {
        self.rollback_error.as_deref()
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Close,
    /// `(repo_path, file_path)` 用于核对结果落地时是不是仍是当前目标
    /// (弹窗打开期间项目被切走/关闭时,过期结果直接丢弃)。
    SnapshotLoaded(PathBuf, PathBuf, Result<FileHistorySnapshot, String>),
    SelectCommit(git2::Oid),
    DiffLoaded(git2::Oid, Result<String, String>),
    RollbackRequest(git2::Oid),
    RollbackDone(git2::Oid, Result<(), String>),
}

/// 弹窗状态机。`state` 是 `&mut Option<State>`(不是 `&mut State`)——
/// `Message::Close` 需要能把它整个置回 `None`,同 `files::AppState.
/// context_menu` 的 `ContextMenuClose` 既有写法。
pub fn update(
    state: &mut Option<State>,
    msg: Message,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::Close => {
            *state = None;
        }
        Message::SnapshotLoaded(repo_path, file_path, result) => {
            let Some(s) = state else { return };
            let Some(target) = &s.target else { return };
            if target.repo_path != repo_path || target.file_path != file_path {
                return; // 已经不是当前目标(项目已切换/弹窗已重开),丢弃。
            }
            let first_oid = match &result {
                Ok(snapshot) => snapshot.entries.first().map(|e| e.oid),
                Err(_) => None,
            };
            s.snapshot = Some(result);
            if let Some(oid) = first_oid {
                s.selected = Some(oid);
                let target = s.target.as_ref().expect("刚核对过 target 非空");
                spawn_diff(target, oid, handle, emit);
            }
        }
        Message::SelectCommit(oid) => {
            let Some(s) = state else { return };
            s.selected = Some(oid);
            if !s.diff_cache.contains_key(&oid)
                && let Some(target) = &s.target
            {
                spawn_diff(target, oid, handle, emit);
            }
        }
        Message::DiffLoaded(oid, result) => {
            let Some(s) = state else { return };
            s.diff_cache.insert(oid, result);
        }
        Message::RollbackRequest(oid) => {
            let Some(s) = state else { return };
            let Some(target) = &s.target else { return };
            s.rollback_pending = Some(oid);
            s.rollback_error = None;
            let repo_path = target.repo_path.clone();
            let file_path = target.file_path.clone();
            handle.spawn(async move {
                let repo_path2 = repo_path.clone();
                let file_path2 = file_path.clone();
                let result = tokio::task::spawn_blocking(move || {
                    rollback_to(&repo_path2, &file_path2, oid)
                })
                .await
                .unwrap_or_else(|e| Err(format!("回滚任务失败: {e}")));
                emit(Message::RollbackDone(oid, result));
            });
        }
        Message::RollbackDone(oid, result) => {
            let Some(s) = state else { return };
            s.rollback_pending = None;
            match result {
                Ok(()) => {
                    s.diff_cache.remove(&oid);
                    if s.selected == Some(oid)
                        && let Some(target) = &s.target
                    {
                        spawn_diff(target, oid, handle, emit);
                    }
                }
                Err(err) => {
                    s.rollback_error = Some(err);
                }
            }
        }
    }
}

fn spawn_diff(
    target: &FileHistoryTarget,
    oid: git2::Oid,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let repo_path = target.repo_path.clone();
    let file_path = target.file_path.clone();
    handle.spawn(async move {
        let repo_path2 = repo_path.clone();
        let file_path2 = file_path.clone();
        let result = tokio::task::spawn_blocking(move || {
            diff_against_current(&repo_path2, &file_path2, oid)
        })
        .await
        .unwrap_or_else(|e| Err(format!("diff 加载任务失败: {e}")));
        emit(Message::DiffLoaded(oid, result));
    });
}

/// 手写 revwalk:从 HEAD 开始逐提交,用
/// `repo.diff_tree_to_tree(parent_tree, tree, Some(&mut opts))` 配合
/// `DiffOptions::pathspec(file_path)` 判断这次提交是否碰过这个文件
/// (pathspec 下推给 git2 做,不用自己在结果里过滤),命中的收进结果,按
/// `max_count` 截断。没有 `git log --follow` 的 rename 跟踪。根提交(无父)
/// 按空树对比,逻辑同 `git_log.rs::commit_detail` 处理根提交的既有写法。
///
/// 注意:文件历史稀疏时(仓库有很多提交、这个文件只被改过几次)需要遍历
/// 大量提交才能凑够 `max_count` 条结果——这是 `git log -- <path>` 的固有
/// 特性,不是 bug,不额外做"扫描上限"截断(YAGNI)。
pub fn build(
    repo_path: &Path,
    file_path: &Path,
    max_count: usize,
) -> Result<FileHistorySnapshot, String> {
    let repo = git2::Repository::open(repo_path).map_err(|e| e.message().to_string())?;
    let pathspec = file_path.to_string_lossy().into_owned();

    let mut revwalk = repo.revwalk().map_err(|e| e.message().to_string())?;
    revwalk.push_head().map_err(|e| e.message().to_string())?;
    revwalk
        .set_sorting(git2::Sort::TIME)
        .map_err(|e| e.message().to_string())?;

    let mut entries = Vec::new();
    for oid in revwalk {
        if entries.len() >= max_count {
            break;
        }
        let oid = oid.map_err(|e| e.message().to_string())?;
        let commit = repo.find_commit(oid).map_err(|e| e.message().to_string())?;
        let new_tree = commit.tree().map_err(|e| e.message().to_string())?;
        let old_tree = match commit.parent(0) {
            Ok(parent) => Some(parent.tree().map_err(|e| e.message().to_string())?),
            Err(_) => None,
        };
        let mut opts = git2::DiffOptions::new();
        opts.pathspec(&pathspec);
        let diff = repo
            .diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), Some(&mut opts))
            .map_err(|e| e.message().to_string())?;
        if diff.deltas().next().is_none() {
            continue;
        }
        let full_sha = oid.to_string();
        let short_sha = full_sha.chars().take(7).collect();
        let summary = commit.summary().ok().flatten().unwrap_or("").to_string();
        let author = commit.author().name().ok().map(|n| n.to_string());
        let time = commit.time().seconds();
        entries.push(FileHistoryEntry {
            oid,
            short_sha,
            summary,
            author,
            time,
        });
    }

    Ok(FileHistorySnapshot {
        repo_path: repo_path.to_path_buf(),
        file_path: file_path.to_path_buf(),
        entries,
    })
}

/// `oid` 对应提交的树 vs *当前工作目录*的这一个文件,用
/// `repo.diff_tree_to_workdir(Some(&tree), Some(&mut opts))`(`opts` 配
/// `pathspec(file_path)`)。`diff_tree_to_workdir` 直接读磁盘上的实时内容
/// (不是索引/HEAD 里的版本,可能包含未提交改动),不需要自己
/// `std::fs::read` 再手动比较——git2 0.21 并未导出
/// `git_diff_blob_to_buffer` 这个 C API,没有"blob 对内存 buffer"直接
/// 比较的安全封装,这是选 `diff_tree_to_workdir` 而不是手动读两份内容比较
/// 的原因。两边内容相同时 `diff` 是空(0 个 delta),返回空字符串。
pub fn diff_against_current(
    repo_path: &Path,
    file_path: &Path,
    oid: git2::Oid,
) -> Result<String, String> {
    let repo = git2::Repository::open(repo_path).map_err(|e| e.message().to_string())?;
    let commit = repo.find_commit(oid).map_err(|e| e.message().to_string())?;
    let tree = commit.tree().map_err(|e| e.message().to_string())?;
    let mut opts = git2::DiffOptions::new();
    opts.pathspec(file_path.to_string_lossy().into_owned());
    let diff = repo
        .diff_tree_to_workdir(Some(&tree), Some(&mut opts))
        .map_err(|e| e.message().to_string())?;

    let mut patch = String::new();
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        let prefix = match line.origin() {
            '+' | '-' | ' ' => line.origin().to_string(),
            _ => String::new(),
        };
        patch.push_str(&prefix);
        patch.push_str(&String::from_utf8_lossy(line.content()));
        true
    })
    .map_err(|e| e.message().to_string())?;

    Ok(patch)
}

/// 取 `oid` 对应提交树里 `file_path` 的 blob 字节,写入
/// `repo_path.join(file_path)`。不碰 git 索引,不 `git add`,是纯粹的文件
/// 系统写入——回滚后 git status 会显示这是一处未提交改动,交给用户/agent
/// 自行决定要不要提交。
pub fn rollback_to(repo_path: &Path, file_path: &Path, oid: git2::Oid) -> Result<(), String> {
    let repo = git2::Repository::open(repo_path).map_err(|e| e.message().to_string())?;
    let commit = repo.find_commit(oid).map_err(|e| e.message().to_string())?;
    let tree = commit.tree().map_err(|e| e.message().to_string())?;
    let entry = tree
        .get_path(file_path)
        .map_err(|e| e.message().to_string())?;
    let object = entry
        .to_object(&repo)
        .map_err(|e| e.message().to_string())?;
    let blob = object
        .into_blob()
        .map_err(|_| "该历史版本对应的不是一个文件".to_string())?;
    std::fs::write(repo_path.join(file_path), blob.content())
        .map_err(|e| format!("写入文件失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(repo: &Path, args: &[&str]) {
        let st = Command::new("git")
            .args(args)
            .current_dir(repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(st.status.success(), "git {args:?}: {st:?}");
    }

    /// c1: 新建 a.txt("one\n") / c2: 新建无关的 b.txt / c3: 改 a.txt 为
    /// "two\n"。只有 c1/c3 该出现在 `build(repo, "a.txt", _)` 的结果里。
    fn mkrepo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "c1: add a.txt"]);
        std::fs::write(repo.join("b.txt"), "unrelated\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "c2: add b.txt"]);
        std::fs::write(repo.join("a.txt"), "two\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "c3: change a.txt"]);
        (dir, repo)
    }

    fn target() -> FileHistoryTarget {
        FileHistoryTarget {
            project_id: 1,
            repo_path: PathBuf::from("/tmp/dozer-file-history-test-repo"),
            file_path: PathBuf::from("a.txt"),
        }
    }

    fn fake_oid(byte: u8) -> git2::Oid {
        git2::Oid::from_bytes(&[byte; 20]).unwrap()
    }

    fn snapshot_with(entries: Vec<FileHistoryEntry>) -> FileHistorySnapshot {
        FileHistorySnapshot {
            repo_path: target().repo_path,
            file_path: target().file_path,
            entries,
        }
    }

    fn fake_entry(oid: git2::Oid, summary: &str) -> FileHistoryEntry {
        FileHistoryEntry {
            oid,
            short_sha: oid.to_string().chars().take(7).collect(),
            summary: summary.to_string(),
            author: None,
            time: 0,
        }
    }

    #[test]
    fn build_only_returns_commits_touching_the_file() {
        let (_d, repo) = mkrepo();
        let snapshot = build(&repo, Path::new("a.txt"), 10).expect("build 应成功");
        assert_eq!(snapshot.entries.len(), 2, "只有 c1/c3 改过 a.txt");
        assert_eq!(
            snapshot.entries[0].summary, "c3: change a.txt",
            "时间倒序,最新的排最前"
        );
        assert_eq!(snapshot.entries[1].summary, "c1: add a.txt");
    }

    #[test]
    fn build_respects_max_count() {
        let (_d, repo) = mkrepo();
        let snapshot = build(&repo, Path::new("a.txt"), 1).expect("build 应成功");
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].summary, "c3: change a.txt");
    }

    #[test]
    fn build_returns_empty_for_file_never_touched() {
        let (_d, repo) = mkrepo();
        let snapshot =
            build(&repo, Path::new("never-existed.txt"), 10).expect("build 应成功");
        assert!(snapshot.entries.is_empty());
    }

    #[test]
    fn diff_against_current_empty_when_content_matches_historical_version() {
        let (_d, repo) = mkrepo();
        let snapshot = build(&repo, Path::new("a.txt"), 10).unwrap();
        let c1_oid = snapshot.entries[1].oid; // c1: a.txt == "one\n"
        // 手动把磁盘内容改回 c1 时的样子,不提交——`diff_against_current`
        // 读的是磁盘实时内容,不是 HEAD。
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        let patch = diff_against_current(&repo, Path::new("a.txt"), c1_oid).unwrap();
        assert!(patch.is_empty(), "内容相同应返回空 patch: {patch:?}");
    }

    #[test]
    fn diff_against_current_shows_changes_when_content_differs() {
        let (_d, repo) = mkrepo();
        let snapshot = build(&repo, Path::new("a.txt"), 10).unwrap();
        let c1_oid = snapshot.entries[1].oid; // c1: a.txt == "one\n"
        // 当前磁盘内容是 c3 之后的 "two\n",跟 c1 的 "one\n" 不同。
        let patch = diff_against_current(&repo, Path::new("a.txt"), c1_oid).unwrap();
        assert!(patch.contains("-one"), "应包含删除旧行: {patch:?}");
        assert!(patch.contains("+two"), "应包含新增行: {patch:?}");
    }

    #[test]
    fn rollback_to_writes_historical_bytes_to_disk() {
        let (_d, repo) = mkrepo();
        let snapshot = build(&repo, Path::new("a.txt"), 10).unwrap();
        let c1_oid = snapshot.entries[1].oid; // c1: a.txt == "one\n"
        rollback_to(&repo, Path::new("a.txt"), c1_oid).expect("回滚应成功");
        let content = std::fs::read_to_string(repo.join("a.txt")).unwrap();
        assert_eq!(content, "one\n", "回滚后磁盘内容应变回 c1 时的字节");
    }

    #[test]
    fn rollback_to_then_diff_against_current_is_empty() {
        let (_d, repo) = mkrepo();
        let snapshot = build(&repo, Path::new("a.txt"), 10).unwrap();
        let c1_oid = snapshot.entries[1].oid;
        rollback_to(&repo, Path::new("a.txt"), c1_oid).unwrap();
        let patch = diff_against_current(&repo, Path::new("a.txt"), c1_oid).unwrap();
        assert!(patch.is_empty(), "回滚后跟同一版本对比应是内容相同: {patch:?}");
    }

    #[tokio::test]
    async fn close_clears_state() {
        let mut state = Some(State::new(target()));
        let handle = tokio::runtime::Handle::current();
        update(&mut state, Message::Close, &handle, |_| {});
        assert!(state.is_none());
    }

    #[tokio::test]
    async fn snapshot_loaded_ignores_result_for_different_repo_path() {
        let mut state = Some(State::new(target()));
        let handle = tokio::runtime::Handle::current();
        let other_repo = PathBuf::from("/tmp/other-repo");
        update(
            &mut state,
            Message::SnapshotLoaded(
                other_repo,
                target().file_path,
                Ok(snapshot_with(vec![fake_entry(fake_oid(1), "s")])),
            ),
            &handle,
            |_| panic!("过期目标的结果不该触发任何 emit"),
        );
        assert!(
            state.as_ref().unwrap().snapshot().is_none(),
            "repo_path 不匹配,结果应被丢弃"
        );
    }

    #[tokio::test]
    async fn snapshot_loaded_with_empty_entries_selects_nothing() {
        let mut state = Some(State::new(target()));
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::SnapshotLoaded(
                target().repo_path,
                target().file_path,
                Ok(snapshot_with(Vec::new())),
            ),
            &handle,
            |_| panic!("空历史列表不该触发 diff 查询"),
        );
        let s = state.unwrap();
        assert!(s.snapshot().unwrap().as_ref().unwrap().entries.is_empty());
        assert!(s.selected().is_none());
    }

    #[tokio::test]
    async fn snapshot_loaded_with_entries_selects_first() {
        let mut state = Some(State::new(target()));
        let handle = tokio::runtime::Handle::current();
        let oid1 = fake_oid(1);
        let oid2 = fake_oid(2);
        update(
            &mut state,
            Message::SnapshotLoaded(
                target().repo_path,
                target().file_path,
                Ok(snapshot_with(vec![
                    fake_entry(oid1, "newest"),
                    fake_entry(oid2, "older"),
                ])),
            ),
            &handle,
            |_| {},
        );
        assert_eq!(state.unwrap().selected(), Some(oid1));
    }

    #[tokio::test]
    async fn select_commit_with_cached_diff_does_not_respawn() {
        let mut state = Some(State::new(target()));
        let oid = fake_oid(3);
        {
            let s = state.as_mut().unwrap();
            s.diff_cache.insert(oid, Ok("cached".to_string()));
        }
        let handle = tokio::runtime::Handle::current();
        update(&mut state, Message::SelectCommit(oid), &handle, |_| {
            panic!("已缓存的 diff 不该重新触发查询")
        });
        assert_eq!(state.unwrap().selected(), Some(oid));
    }

    #[tokio::test]
    async fn diff_loaded_caches_result() {
        let mut state = Some(State::new(target()));
        let oid = fake_oid(4);
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::DiffLoaded(oid, Ok("patch text".to_string())),
            &handle,
            |_| {},
        );
        assert_eq!(
            state.unwrap().diff_for(oid),
            Some(&Ok("patch text".to_string()))
        );
    }

    #[tokio::test]
    async fn rollback_request_sets_pending_and_clears_previous_error() {
        let mut state = Some(State::new(target()));
        {
            let s = state.as_mut().unwrap();
            s.rollback_error = Some("上一次失败".to_string());
        }
        let oid = fake_oid(5);
        let handle = tokio::runtime::Handle::current();
        update(&mut state, Message::RollbackRequest(oid), &handle, |_| {});
        let s = state.unwrap();
        assert_eq!(s.rollback_pending(), Some(oid));
        assert!(s.rollback_error().is_none());
    }

    #[tokio::test]
    async fn rollback_done_err_sets_error_and_clears_pending() {
        let mut state = Some(State::new(target()));
        let oid = fake_oid(6);
        {
            let s = state.as_mut().unwrap();
            s.rollback_pending = Some(oid);
        }
        let handle = tokio::runtime::Handle::current();
        update(
            &mut state,
            Message::RollbackDone(oid, Err("写入失败".to_string())),
            &handle,
            |_| {},
        );
        let s = state.unwrap();
        assert!(s.rollback_pending().is_none());
        assert_eq!(s.rollback_error(), Some("写入失败"));
    }

    #[tokio::test]
    async fn rollback_done_ok_clears_cache_entry_for_that_oid() {
        let mut state = Some(State::new(target()));
        let oid = fake_oid(7);
        {
            let s = state.as_mut().unwrap();
            s.rollback_pending = Some(oid);
            s.selected = Some(fake_oid(99)); // 不等于 oid,不会触发重新查询
            s.diff_cache.insert(oid, Ok("stale".to_string()));
        }
        let handle = tokio::runtime::Handle::current();
        update(&mut state, Message::RollbackDone(oid, Ok(())), &handle, |_| {
            panic!("selected 不是这个 oid,不该重新查询 diff")
        });
        let s = state.unwrap();
        assert!(s.rollback_pending().is_none());
        assert!(s.diff_for(oid).is_none(), "回滚成功应清掉旧缓存");
    }
}
