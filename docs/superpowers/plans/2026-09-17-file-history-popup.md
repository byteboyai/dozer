# 文件历史对比弹窗 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Files 面板文件行右键菜单加「查看此文件历史」,弹出窗口级弹窗对比
当前磁盘内容与该文件的历史提交版本,并支持一键回滚到某个历史版本。

**Architecture:** 新增自包含扩展模块 `extensions/file_history.rs`(数据类型 +
`build`/`diff_against_current`/`rollback_to` 三个 git2 查询函数 + `State`/
`Message`/`update` 状态机 + `popup_view` 渲染),状态以 `Option<State>` 挂在
顶层 `App`(同 `project_link_menu` 的既有模式)。Files 右键菜单
(`context_menu_spec`/`context_menu_items`)加一个 `is_git_repo` 参数和一条
新菜单项,触发消息由内核拦截、组出目标信息、异步跑 git2 查询,结果通过
`Message::FileHistory(file_history::Message::...)` 落地。

**Tech Stack:** Rust, iced 0.14(`iced_widget`/`iced_renderer`),git2 0.21,
tokio(`handle.spawn` + `spawn_blocking`)。

**Spec:** `docs/superpowers/specs/2026-09-17-file-history-popup-design.md`

## Global Constraints

- 只对接 git 提交历史,不做 IDE 本地保存历史、不做 rename 跟踪
  (`git log --follow` 语义)。
- diff 渲染复用现有 `extensions::diff_render::colored_diff_lines`(单栏
  染色),不新建双栏行号对照组件。
- 回滚(`rollback_to`)只做纯文件系统写入,不碰 git 索引、不
  `git add`/`commit`。
- 提交列表/diff 查询走 `handle.spawn` + `tokio::task::spawn_blocking` +
  `emit` 异步落地,不阻塞 UI 线程(参照 `git_log.rs::update`/
  `request_refresh` 的既有写法)。
- 本仓库既有硬性要求:功能分支在独立 git worktree 里开发,完工经代码审阅
  通过后再合并 main,不直接在 main 上开发。

---

## Task 1: `file_history.rs` 数据层——类型 + `build`/`diff_against_current`/`rollback_to`

**Files:**
- Create: `crates/dozer-app/src/extensions/file_history.rs`
- Modify: `crates/dozer-app/src/extensions.rs`(注册新模块)

**Interfaces:**
- Produces:
  - `pub struct FileHistoryEntry { pub oid: git2::Oid, pub short_sha: String, pub summary: String, pub author: Option<String>, pub time: i64 }`
  - `pub struct FileHistorySnapshot { pub repo_path: PathBuf, pub file_path: PathBuf, pub entries: Vec<FileHistoryEntry> }`
  - `pub const DEFAULT_MAX_COUNT: usize = 200;`
  - `pub fn build(repo_path: &Path, file_path: &Path, max_count: usize) -> Result<FileHistorySnapshot, String>`
  - `pub fn diff_against_current(repo_path: &Path, file_path: &Path, oid: git2::Oid) -> Result<String, String>`
  - `pub fn rollback_to(repo_path: &Path, file_path: &Path, oid: git2::Oid) -> Result<(), String>`
- Consumes: 无(本任务是最底层,不依赖其它任务)。

- [ ] **Step 1: 建文件,写数据类型 + `build`/`diff_against_current`/`rollback_to` 的失败测试**

`crates/dozer-app/src/extensions.rs` 里 `pub mod diff_render;` 和
`pub mod files;` 之间插入一行:

```rust
pub mod file_history;
```

创建 `crates/dozer-app/src/extensions/file_history.rs`:

```rust
//! 单个文件的历史提交对比:右键 Files 面板文件行"查看此文件历史"弹窗的
//! 数据层。跟 `git_log.rs` 的区别是那边是"整个仓库的提交图",这里是
//! "只关心一个文件路径,且要跟*当前工作目录实时内容*比较,不是跟某个
//! commit 的父提交比较"。见
//! `docs/superpowers/specs/2026-09-17-file-history-popup-design.md`。

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
}
```

- [ ] **Step 2: 跑一遍确认能编译(还没写测试函数体,先确认类型/函数签名没问题)**

Run: `cargo check -p dozer-app`
Expected: 编译通过(`file_history` 模块目前只有类型 + 三个函数 + 空
`tests` 模块,`mkrepo` 未被使用会有 `dead_code` warning,属预期,下一步补
测试后消失)。

- [ ] **Step 3: 补 `build()` 的测试**

在 `tests` 模块(`mkrepo` 函数之后)追加:

```rust
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
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app file_history::tests::build_ -- --test-threads=1`
Expected: 3 个测试全部 PASS。

- [ ] **Step 5: 补 `diff_against_current()` 的测试**

追加:

```rust
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
```

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p dozer-app file_history::tests::diff_against_current_ -- --test-threads=1`
Expected: 2 个测试全部 PASS。

- [ ] **Step 7: 补 `rollback_to()` 的测试**

追加:

```rust
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
```

- [ ] **Step 8: 跑全部测试确认通过**

Run: `cargo test -p dozer-app file_history:: -- --test-threads=1`
Expected: Step 3/5/7 加起来共 7 个测试全部 PASS。

- [ ] **Step 9: `cargo clippy`/`cargo fmt` 检查**

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | grep file_history` 和
`cargo fmt -p dozer-app -- --check`
Expected: 无 `file_history` 相关 clippy 警告;`fmt --check` 无输出(已是
标准格式)或按提示跑 `cargo fmt -p dozer-app` 修正。

- [ ] **Step 10: Commit**

```bash
git add crates/dozer-app/src/extensions.rs crates/dozer-app/src/extensions/file_history.rs
git commit -m "feat(dozer-app): add file_history data layer (build/diff/rollback)"
```

---

## Task 2: `file_history.rs` 状态机——`FileHistoryTarget`/`State`/`Message`/`update`

**Files:**
- Modify: `crates/dozer-app/src/extensions/file_history.rs`

**Interfaces:**
- Consumes: Task 1 的 `FileHistoryEntry`/`FileHistorySnapshot`/`build`/
  `diff_against_current`/`rollback_to`/`DEFAULT_MAX_COUNT`。
- Produces:
  - `pub struct FileHistoryTarget { pub project_id: i64, pub repo_path: PathBuf, pub file_path: PathBuf }`
  - `pub struct State { .. }`(私有字段,对外只暴露下面的只读访问方法)
    - `impl State { pub fn new(target: FileHistoryTarget) -> Self; pub fn target(&self) -> Option<&FileHistoryTarget>; pub fn snapshot(&self) -> Option<&Result<FileHistorySnapshot, String>>; pub fn selected(&self) -> Option<git2::Oid>; pub fn diff_for(&self, oid: git2::Oid) -> Option<&Result<String, String>>; pub fn rollback_pending(&self) -> Option<git2::Oid>; pub fn rollback_error(&self) -> Option<&str>; }`
  - `pub enum Message { Close, SnapshotLoaded(PathBuf, PathBuf, Result<FileHistorySnapshot, String>), SelectCommit(git2::Oid), DiffLoaded(git2::Oid, Result<String, String>), RollbackRequest(git2::Oid), RollbackDone(git2::Oid, Result<(), String>) }`(`derive(Debug, Clone)`,顶层
    `Message` 要装它)
  - `pub fn update(state: &mut Option<State>, msg: Message, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`

- [ ] **Step 1: 写状态迁移的失败测试(先写 `Close`/`SnapshotLoaded` 两组)**

在 `file_history.rs` 顶部 `use` 区加 `use std::collections::HashMap;`。在
`DEFAULT_MAX_COUNT` 常量之后、`build()` 函数之前插入类型与 `update`:

```rust
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
```

在 `tests` 模块(`mkrepo` 之后、`build_only_returns_commits_touching_the_file`
之前或之后均可,建议紧跟 `mkrepo` 之后)追加状态机测试用的构造小工具与测试:

```rust
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
```

- [ ] **Step 2: 跑测试确认全部通过**

Run: `cargo test -p dozer-app file_history:: -- --test-threads=1`
Expected: Task 1 的 7 个 + 本任务新增的 9 个,共 16 个测试全部 PASS。

- [ ] **Step 3: `cargo clippy`/`cargo fmt` 检查**

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | grep file_history`
Expected: 无相关警告。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/file_history.rs
git commit -m "feat(dozer-app): add file_history State/Message/update state machine"
```

---

## Task 3: Files 右键菜单接入「查看此文件历史」

**Files:**
- Modify: `crates/dozer-app/src/extensions/files/state.rs`(新增
  `Message::FileHistoryOpen` 变体)
- Modify: `crates/dozer-app/src/extensions/files/view.rs`(`context_menu_spec`/
  `context_menu_items` 签名加 `is_git_repo: bool`,插入新菜单项)
- Modify: `crates/dozer-app/src/extensions/files/update.rs`(两处调用点传参 +
  `FileHistoryOpen` 的 `unreachable!` 分支)
- Modify: `crates/dozer-app/src/extensions/files/mod.rs`(5 个既有
  `context_menu_items` 测试补参数,新增 3 个测试)

**Interfaces:**
- Consumes: 无(本任务不依赖 `file_history` 模块,`FileHistoryOpen` 消息本身
  只携带 `PathBuf`,由 App 层负责解读)。
- Produces: `files::Message::FileHistoryOpen(PathBuf)`——**由内核拦截**
  (Task 4 处理),`files::update()` 收到会 `unreachable!()`。

- [ ] **Step 1: `files::Message` 加新变体(先加变体,`update`/`view` 暂不处理,
      此时 `cargo check` 应该报"match 不穷尽"——这是本步的"失败测试")**

`crates/dozer-app/src/extensions/files/state.rs`,在 `ContextMenuClose,`
后面插入(紧邻现有右键菜单相关消息,便于阅读):

```rust
    /// 右键"查看此文件历史":内核拦截,不进 `update`——由内核解析出仓库
    /// 相对路径、组出 `file_history::FileHistoryTarget`,写入
    /// `App::file_history` 并异步跑 `file_history::build`(见
    /// `docs/superpowers/specs/2026-09-17-file-history-popup-design.md`)。
    /// 携带的是右键目标的绝对路径,同其它右键菜单消息(`DeleteRequest`/
    /// `RevealInFinder` 等)的既有口径。
    FileHistoryOpen(PathBuf),
```

Run: `cargo check -p dozer-app`
Expected: FAIL——`files::update()` 里的 `match msg` 现在漏了
`FileHistoryOpen` 分支,报 non-exhaustive match 编译错误。

- [ ] **Step 2: `files::update()` 补 `unreachable!` 分支**

`crates/dozer-app/src/extensions/files/update.rs`,在
`Message::OpenSearch(..) => { unreachable!(...) }` 之后插入:

```rust
        Message::FileHistoryOpen(_) => {
            unreachable!("由内核拦截处理,见 files::Message::FileHistoryOpen 文档")
        }
```

Run: `cargo check -p dozer-app`
Expected: 编译通过。

- [ ] **Step 3: `context_menu_spec`/`context_menu_items` 加 `is_git_repo` 参数 +
      新菜单项(先改函数体和签名)**

`crates/dozer-app/src/extensions/files/view.rs`,把:

```rust
pub fn context_menu_items(
    target: &Path,
    is_dir: bool,
    is_root: bool,
    has_clipboard: bool,
) -> Vec<crate::chrome::native_menu::Item<Message>> {
    crate::menu_spec::to_native(context_menu_spec(target, is_dir, is_root, has_clipboard))
}

/// 文件树右键菜单内容——native(`context_menu_items`)和 iced fallback
/// (`context_menu_popup`)共用同一份数据,11 个条件分支只写一遍。
pub(crate) fn context_menu_spec(
    target: &Path,
    is_dir: bool,
    is_root: bool,
    has_clipboard: bool,
) -> MenuSpec<Message> {
```

改成:

```rust
pub fn context_menu_items(
    target: &Path,
    is_dir: bool,
    is_root: bool,
    has_clipboard: bool,
    is_git_repo: bool,
) -> Vec<crate::chrome::native_menu::Item<Message>> {
    crate::menu_spec::to_native(context_menu_spec(
        target,
        is_dir,
        is_root,
        has_clipboard,
        is_git_repo,
    ))
}

/// 文件树右键菜单内容——native(`context_menu_items`)和 iced fallback
/// (`context_menu_popup`)共用同一份数据,12 个条件分支只写一遍。
pub(crate) fn context_menu_spec(
    target: &Path,
    is_dir: bool,
    is_root: bool,
    has_clipboard: bool,
    is_git_repo: bool,
) -> MenuSpec<Message> {
```

在函数体末尾(`items.push(MenuSpecItem::entry(Some(icons::IconKind::RefreshCw), "从磁盘重新加载", Message::ReloadFromDisk,));`
之后、`items` 之前)追加:

```rust
    if !is_dir && is_git_repo {
        items.push(MenuSpecItem::entry(
            Some(icons::IconKind::History),
            "查看此文件历史",
            Message::FileHistoryOpen(target.clone()),
        ));
    }
```

- [ ] **Step 4: 两处调用点补 `is_git_repo` 实参**

`crates/dozer-app/src/extensions/files/update.rs` 第 55 行附近:

```rust
                let items = context_menu_items(&path, is_dir, is_root, has_clipboard);
```

改成:

```rust
                let items = context_menu_items(
                    &path,
                    is_dir,
                    is_root,
                    has_clipboard,
                    ws_state.git_is_repo,
                );
```

`crates/dozer-app/src/extensions/files/view.rs` 的 `context_menu_popup()`
里:

```rust
    let spec = context_menu_spec(&menu.target, menu.is_dir, is_root, has_clipboard);
```

改成:

```rust
    let spec = context_menu_spec(
        &menu.target,
        menu.is_dir,
        is_root,
        has_clipboard,
        ws_state.git_is_repo,
    );
```

Run: `cargo check -p dozer-app`
Expected: 编译通过。

- [ ] **Step 5: 补 5 个既有测试的第 5 个参数(先跑一遍确认按预期报错)**

Run: `cargo test -p dozer-app files::mod::tests::context_menu_items -- --test-threads=1`
Expected: FAIL——编译错误,5 处调用少一个参数。

`crates/dozer-app/src/extensions/files/mod.rs` 里 5 处
`context_menu_items(...)` 调用(`context_menu_items_hides_delete_rename_for_root`/
`context_menu_items_shows_delete_rename_for_non_root`/
`context_menu_items_paste_locked_when_clipboard_empty`/
`context_menu_items_paste_enabled_when_clipboard_has_content`/
`context_menu_items_file_target_has_no_new_file_or_paste`),各自在现有
4 个实参后加 `true`(这些测试都不关心"是不是 git 仓库"这个新维度,传
`true` 保持原有断言的语义不变):

```rust
        let items = context_menu_items(Path::new("/proj"), true, true, false, true);
```

```rust
        let items = context_menu_items(Path::new("/proj/src"), true, false, false, true);
```

(后续三处同理,只在行尾原有 `false)`/`true)` 前加 `, true`。)

- [ ] **Step 6: 跑测试确认 5 个既有测试通过**

Run: `cargo test -p dozer-app files::mod::tests::context_menu_items -- --test-threads=1`
Expected: 5 个测试全部 PASS。

- [ ] **Step 7: 新增 3 个针对 `is_git_repo`/新菜单项的测试**

同一个 `mod tests` 里追加:

```rust
    #[cfg(target_os = "macos")]
    #[test]
    fn context_menu_items_shows_file_history_for_git_repo_file() {
        let items = context_menu_items(Path::new("/proj/src/main.rs"), false, false, false, true);
        let has_history = items.iter().any(|i| {
            matches!(
                i,
                crate::chrome::native_menu::Item::Entry {
                    msg: Message::FileHistoryOpen(..),
                    ..
                }
            )
        });
        assert!(has_history, "git 仓库里的文件行该有「查看此文件历史」");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn context_menu_items_hides_file_history_for_non_git_repo() {
        let items = context_menu_items(Path::new("/proj/src/main.rs"), false, false, false, false);
        let has_history = items.iter().any(|i| {
            matches!(
                i,
                crate::chrome::native_menu::Item::Entry {
                    msg: Message::FileHistoryOpen(..),
                    ..
                }
            )
        });
        assert!(!has_history, "非 git 仓库不该出现「查看此文件历史」");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn context_menu_items_hides_file_history_for_directory() {
        let items = context_menu_items(Path::new("/proj/src"), true, false, false, true);
        let has_history = items.iter().any(|i| {
            matches!(
                i,
                crate::chrome::native_menu::Item::Entry {
                    msg: Message::FileHistoryOpen(..),
                    ..
                }
            )
        });
        assert!(!has_history, "目录行不该出现「查看此文件历史」");
    }
```

- [ ] **Step 8: 跑全部相关测试确认通过**

Run: `cargo test -p dozer-app files::mod::tests::context_menu_items -- --test-threads=1`
Expected: 5 + 3 = 8 个测试全部 PASS。

- [ ] **Step 9: `cargo clippy`/`cargo fmt` 检查**

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "extensions/files"`
Expected: 无新增警告。

- [ ] **Step 10: Commit**

```bash
git add crates/dozer-app/src/extensions/files/state.rs \
        crates/dozer-app/src/extensions/files/view.rs \
        crates/dozer-app/src/extensions/files/update.rs \
        crates/dozer-app/src/extensions/files/mod.rs
git commit -m "feat(dozer-app): add \"view file history\" entry to Files context menu"
```

---

## Task 4: App 级接线——`Message::FileHistory`、拦截 `FileHistoryOpen`、分发状态机

**Files:**
- Modify: `crates/dozer-app/src/app/message.rs`(顶层 `Message` 加
  `FileHistory(file_history::Message)` 变体 + `use` 补 `file_history`)
- Modify: `crates/dozer-app/src/app/app.rs`(`App` 加
  `file_history: Option<file_history::State>` 字段 + 初始化 + `use` 补
  `file_history`)
- Modify: `crates/dozer-app/src/app/update.rs`(拦截
  `Message::Files(files::Message::FileHistoryOpen(path))`、分发
  `Message::FileHistory(msg)`、`use` 补 `file_history`)

**Interfaces:**
- Consumes: Task 1/2 的 `file_history::{FileHistoryTarget, State, Message,
  build, update, DEFAULT_MAX_COUNT}`;Task 3 的
  `files::Message::FileHistoryOpen(PathBuf)`。
- Produces: 顶层 `Message::FileHistory(file_history::Message)`,`App.
  file_history: Option<file_history::State>` 字段(后续 Task 5 的
  `app/view.rs` 要读它)。

- [ ] **Step 1: 顶层 `Message` 加变体(先加,产生"未使用"式的编译告警属预期,
      下面步骤补上消费方后消失)**

`crates/dozer-app/src/app/message.rs`:

```rust
use crate::extensions::{
    browser, conversations, database, files, footbar, git_log, project, search, ssh, todo, usage,
};
```

改成:

```rust
use crate::extensions::{
    browser, conversations, database, file_history, files, footbar, git_log, project, search,
    ssh, todo, usage,
};
```

在 `Files(files::Message),` 之前插入:

```rust
    /// 文件历史对比弹窗的全部消息,内核只转发不解读——见
    /// `extensions::file_history::Message`。
    FileHistory(file_history::Message),
```

Run: `cargo check -p dozer-app`
Expected: FAIL——`App::update` 里的顶层 `match` 现在漏了 `FileHistory`
分支,报 non-exhaustive match。

- [ ] **Step 2: `App` 加 `file_history` 字段**

`crates/dozer-app/src/app/app.rs`:

```rust
use crate::extensions::files;
```

改成(按现有字母序插入):

```rust
use crate::extensions::file_history;
use crate::extensions::files;
```

`pub(crate) project_link_menu: Option<ProjectLinkMenu>,` 之后插入:

```rust
    /// 文件历史对比弹窗状态——见 `extensions::file_history::State`。`None`
    /// 表示弹窗未打开。
    pub(crate) file_history: Option<file_history::State>,
```

`project_link_menu: None,` 之后(构造函数里)插入:

```rust
            file_history: None,
```

Run: `cargo check -p dozer-app`
Expected: 仍然 FAIL,原因同 Step 1(顶层 match 还没补)。

- [ ] **Step 3: `App::update` 拦截 `FileHistoryOpen` + 分发 `FileHistory`**

`crates/dozer-app/src/app/update.rs`:

```rust
use crate::extensions::files;
```

改成:

```rust
use crate::extensions::file_history;
use crate::extensions::files;
```

在 `Message::Files(files::Message::OpenSearch(path, is_dir)) => { ... }`
分支之后插入:

```rust
            Message::Files(files::Message::FileHistoryOpen(path)) => {
                // 右键"查看此文件历史":先收起右键菜单(同 OpenSearch 的既有
                // 约定),再解析出仓库相对路径、组出 `FileHistoryTarget`、
                // 异步跑一次 `build()`。
                self.files.close_context_menu();
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let repo_path = PathBuf::from(&project.path);
                let Ok(file_path) = path.strip_prefix(&repo_path).map(|p| p.to_path_buf()) else {
                    return;
                };
                let target = file_history::FileHistoryTarget {
                    project_id,
                    repo_path: repo_path.clone(),
                    file_path: file_path.clone(),
                };
                self.file_history = Some(file_history::State::new(target));
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                handle.spawn(async move {
                    let repo_path2 = repo_path.clone();
                    let file_path2 = file_path.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        file_history::build(&repo_path2, &file_path2, file_history::DEFAULT_MAX_COUNT)
                    })
                    .await
                    .unwrap_or_else(|e| Err(format!("加载失败: {e}")));
                    let _ = proxy.send_event(Message::FileHistory(
                        file_history::Message::SnapshotLoaded(repo_path, file_path, result),
                    ));
                });
            }
```

在顶层 `match` 的某处(建议紧邻 `Message::GitLog(msg) => { ... }` 分支
之后)插入:

```rust
            Message::FileHistory(msg) => {
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::FileHistory(m));
                };
                file_history::update(&mut self.file_history, msg, &handle, emit);
            }
```

- [ ] **Step 4: 跑编译确认通过**

Run: `cargo build -p dozer-app`
Expected: 编译通过,无 warning(`file_history` 字段/变体此时已被读写)。

- [ ] **Step 5: `cargo clippy` 检查**

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | tail -40`
Expected: 无新增警告(已有的、与本次改动无关的历史警告不算)。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/app/message.rs \
        crates/dozer-app/src/app/app.rs \
        crates/dozer-app/src/app/update.rs
git commit -m "feat(dozer-app): wire file_history state into App::update"
```

---

## Task 5: 弹窗渲染 + 挂载到 `app/view.rs`

**Files:**
- Modify: `crates/dozer-app/src/extensions/file_history.rs`(新增
  `popup_view` 及内部渲染辅助函数)
- Modify: `crates/dozer-app/src/app/view.rs`(挂载弹窗、`use` 补
  `file_history`)

**Interfaces:**
- Consumes: Task 2 的 `State`(`snapshot`/`selected`/`diff_for`/
  `rollback_pending`/`rollback_error` 只读访问方法)、`Message`。
- Produces: `pub fn popup_view<'a>(state: &'a State, window_width: f32, window_height: f32) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>`。

- [ ] **Step 1: 写 `popup_view` 与内部渲染函数**

在 `file_history.rs` 文件末尾(`update`/`spawn_diff` 之后、`#[cfg(test)]`
之前)追加:

```rust
/// 弹窗顶部标题 + 左侧提交列表 + 右侧 diff 区。布局参照
/// `project::project_delete_confirm_popup` 的窗口级卡片外壳(CARD 底 +
/// 圆角描边),但宽度/高度不用 `dialog::width`(那是"整窗 1/3"的确认框
/// 默认值,内容是左右分栏的提交列表 + diff,1/3 窗宽放不下,`dialog.rs`
/// 文档本身允许"字段特别多的表单"在这个默认值基础上另外调整)。
pub fn popup_view<'a>(
    state: &'a State,
    window_width: f32,
    window_height: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let title = row![
        icons::view(
            icons::IconKind::History,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().cream,
        ),
        text("文件历史")
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().cream),
        iced_widget::space::horizontal(),
        button(
            text("×")
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().dim)
        )
        .on_press(Message::Close)
        .style(|_t, _s| iced_widget::button::Style {
            background: None,
            text_color: byteui::theme::color::current().dim,
            ..iced_widget::button::Style::default()
        }),
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);

    let body = row![
        container(commit_list_view(state))
            .width(Length::Fixed(240.0))
            .height(Length::Fill),
        diff_area_view(state),
    ]
    .spacing(12)
    .height(Length::Fill);

    let dialog = container(column![title, body].spacing(12))
        .width(Length::Fixed(window_width * 0.75))
        .height(Length::Fixed(window_height * 0.8))
        .padding(16)
        .style(crate::dialog::card_style);

    container(dialog)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}

fn commit_list_view<'a>(
    state: &'a State,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(snapshot) = state.snapshot() else {
        return container(
            text("加载中…")
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().dim),
        )
        .padding(8)
        .into();
    };
    let entries = match snapshot {
        Ok(s) => &s.entries,
        Err(err) => {
            return container(
                text(err.clone())
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().red),
            )
            .padding(8)
            .into();
        }
    };
    if entries.is_empty() {
        return container(
            text("该文件没有提交记录")
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().dim),
        )
        .padding(8)
        .into();
    }
    let mut col = column![].spacing(4);
    for entry in entries {
        let is_selected = state.selected() == Some(entry.oid);
        let label = column![
            text(entry.summary.clone())
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().cream),
            text(format_commit_time(entry.time))
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().dim),
        ]
        .spacing(2);
        let row_btn = button(label)
            .on_press(Message::SelectCommit(entry.oid))
            .width(Length::Fill)
            .padding(8)
            .style(move |_t, _s| iced_widget::button::Style {
                background: if is_selected {
                    Some(byteui::theme::color::current().card.into())
                } else {
                    None
                },
                text_color: byteui::theme::color::current().body,
                ..iced_widget::button::Style::default()
            });
        col = col.push(row_btn);
    }
    scrollable(col)
        .direction(scrollable::Direction::Vertical(
            byteui::interaction::scrollbar::scrollbar(),
        ))
        .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style())
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn diff_area_view<'a>(
    state: &'a State,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(oid) = state.selected() else {
        return container(
            text("未选中版本")
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().dim),
        )
        .padding(8)
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    };

    let header = row![
        text(format!("当前 vs {}", short_sha_of(state, oid)))
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().cream),
        iced_widget::space::horizontal(),
        rollback_button(state, oid),
    ]
    .spacing(8)
    .align_y(iced_widget::core::Alignment::Center);

    let content: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match state.diff_for(oid) {
            None => container(
                text("加载中…")
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().dim),
            )
            .padding(8)
            .into(),
            Some(Err(err)) => container(
                text(err.clone())
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().red),
            )
            .padding(8)
            .into(),
            Some(Ok(patch)) if patch.is_empty() => container(
                text("内容相同")
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().dim),
            )
            .padding(8)
            .into(),
            Some(Ok(patch)) => scrollable(crate::extensions::diff_render::colored_diff_lines(
                patch,
            ))
            .direction(scrollable::Direction::Vertical(
                byteui::interaction::scrollbar::scrollbar(),
            ))
            .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style())
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
        };

    let mut col = column![header, content]
        .spacing(8)
        .width(Length::Fill)
        .height(Length::Fill);
    if let Some(err) = state.rollback_error() {
        col = col.push(
            text(format!("回滚失败: {err}"))
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().red),
        );
    }
    col.into()
}

fn rollback_button<'a>(
    state: &'a State,
    oid: git2::Oid,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let disabled = state.rollback_pending().is_some();
    button(
        text("回滚")
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().gold),
    )
    .on_press_maybe((!disabled).then_some(Message::RollbackRequest(oid)))
    .padding([4, 10])
    .style(crate::dialog::action_button_style(
        byteui::theme::color::current().gold,
    ))
    .into()
}

fn short_sha_of(state: &State, oid: git2::Oid) -> String {
    state
        .snapshot()
        .and_then(|r| r.as_ref().ok())
        .and_then(|s| s.entries.iter().find(|e| e.oid == oid))
        .map(|e| e.short_sha.clone())
        .unwrap_or_else(|| oid.to_string().chars().take(7).collect())
}

/// commit 时间戳格式化,`YYYY-MM-DD HH:MM:SS`,UTC。跟
/// `git_log.rs::format_commit_time` 同源但那边是模块私有、不便跨模块复用
/// (该函数自己的文档也是这么处理 `todo.rs` 同名函数的),这里照抄一份。
fn format_commit_time(unix_secs: i64) -> String {
    let secs = unix_secs.max(0) as u64;
    let days = (secs / 86_400) as i64;
    let secs_of_day = secs % 86_400;
    let (h, m, s) = (
        secs_of_day / 3600,
        (secs_of_day / 60) % 60,
        secs_of_day % 60,
    );
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}")
}

/// Howard Hinnant 的 `civil_from_days` 算法:Unix epoch 起的天数 → (年, 月, 日)。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
```

在文件顶部 `use` 区补齐渲染需要的类型(与 `git_log.rs`/`project/view.rs`
的既有 `use` 集合一致):

```rust
use byteui::interaction::icons;
use iced_widget::core::{Element, Length};
use iced_widget::{button, column, container, row, scrollable, text};
```

Run: `cargo check -p dozer-app`
Expected: 编译通过。

- [ ] **Step 2: 挂载到 `app/view.rs`**

`crates/dozer-app/src/app/view.rs`:

```rust
use crate::extensions::files;
```

改成:

```rust
use crate::extensions::file_history;
use crate::extensions::files;
```

在 `} else if ws.database.content().tab_overflow_anchor().is_some() { ... }`
分支(最后一个 `else if`)与最终 `} else { ... stack![base].into() };` 之间
插入:

```rust
        } else if self.file_history.is_some() {
            // 文件历史对比弹窗:窗口级 overlay,同其它面板弹窗的既有口径。
            // 状态是 `App` 级单例(不挂 `Workspace`),同 `project_link_menu`/
            // `self.database.drivers_popup_open()` 的既有先例。
            let state = self.file_history.as_ref().unwrap();
            let dismiss = crate::dialog::scrim(Message::FileHistory(file_history::Message::Close));
            stack![
                base,
                dismiss,
                file_history::popup_view(state, self.window_size.0, self.window_size.1)
                    .map(Message::FileHistory)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
```

（原本紧邻最终 `} else {` 之前的那个 `}` 保持不变，只是新分支插在它和
`} else {` 之间。）

- [ ] **Step 3: 编译 + 启动检查**

Run: `cargo build -p dozer-app`
Expected: 编译通过。

Run: `cargo run -p dozer-app`(后续在 Task 6 里做完整人工验收,这里只确认
能正常启动、不崩溃)
Expected: 应用正常打开,不因本次改动崩溃或 panic。

- [ ] **Step 4: `cargo clippy`/`cargo fmt` 检查**

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | tail -40` 和
`cargo fmt -p dozer-app -- --check`
Expected: 无新增警告;`fmt --check` 通过或按提示跑
`cargo fmt -p dozer-app` 修正。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/file_history.rs \
        crates/dozer-app/src/app/view.rs
git commit -m "feat(dozer-app): render file history popup and mount it in app view"
```

---

## Task 6: 人工 GUI 验收

**Files:** 无代码改动,本任务是最终验收清单。

**Interfaces:** 无。

- [ ] **Step 1: 准备一个真实 git 仓库项目并跑起来**

Run: `cargo run -p dozer-app`
在 Dozer 里打开一个真实的 git 项目(建议就用 dozer 自己这个仓库),Files
面板随便选一个改动过多次的文件(比如
`crates/dozer-app/src/extensions/git_log.rs`)。

- [ ] **Step 2: 逐条对照验证**

- [ ] 右键该文件 → 出现「查看此文件历史」菜单项。
- [ ] 右键一个目录 → **没有**「查看此文件历史」菜单项。
- [ ] 点击「查看此文件历史」→ 弹出窗口级弹窗,左侧提交列表 loading 后正确
      填充(commit message + 时间,倒序)。
- [ ] 默认选中第一项(最近一次改动),右侧 diff 区展示"当前 vs {sha}"的
      单栏染色 diff。
- [ ] 点列表里另一个历史提交 → 右侧 diff 区切换到新的对比结果(loading →
      内容)。
- [ ] 找一个"当前工作区内容与某历史版本完全相同"的场景(比如该文件最近一次
      提交后没有再改过,选中那次提交)→ diff 区显示"内容相同"。
- [ ] 点「回滚」→ 按钮变不可点 → 完成后磁盘文件内容确实变成历史版本的字节
      (可用外部编辑器/`cat` 核实)→ 弹窗内 diff 区自动变成"内容相同" →
      Files 树该文件的 dirty/改动图标随之更新(依赖既有 `git_watch` 自动
      刷新,不需要手动刷新文件树)。
- [ ] 关闭弹窗(点 × 或点遮罩)→ 弹窗消失,回到正常界面。
- [ ] 打开一个非 git 仓库的项目(或普通目录)→ 右键文件行**没有**「查看此
      文件历史」菜单项。
- [ ] 打开弹窗期间切换项目 tab(如果 UI 允许在弹窗打开时切换)→ 不崩溃,
      异步结果落地时按 spec"错误处理"一节的目标核对逻辑正确丢弃/接受。

- [ ] **Step 3: 核对 webview 遮挡问题(spec 遗留的实现阶段核对项)**

在预览面板正打开某个文件预览(webview 类型,比如 Markdown/HTML 预览)的
状态下打开「查看此文件历史」弹窗,检查弹窗是否被 Preview 面板的 wry 子
webview 盖住(webview 恒在 GPU 内容之上,若被盖住需要按
`webview_hidden_by_panel_popup`/`project_delete_confirm_popup` 的既有处理
方式,把 `self.file_history.is_some()` 接入相应的 webview 隐藏判断链路;
如果没有被盖住说明当前窗口级弹窗的既有处理方式已经覆盖了这个场景)。若
发现被遮挡,记录为后续修复项(不在本计划范围内新增无计划的大改,先如实
报告)。

- [ ] **Step 4: 汇总验收结果**

把上面清单的通过/不通过情况汇报给用户,任何不通过项说明现象,不自行判断
"问题不大"而跳过。
