# Dozer Git Log 面板转正实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `crates/dozer-app/src/git_log.rs` 从 spike 转正:提交图加分支/tag 标签,选中提交能看到改动文件+diff,面板顶部加同仓库 worktree 速览条(点击直接切换/打开项目页签),不再是固定 200 条提交的一次性快照,支持"加载更多"。全程只读,不新增任何会改写仓库状态的操作。

**Architecture:** 数据仍全部经 `gleisbau`(提交图布局)+ `git2`(提交详情 diff)在 `git_log.rs` 里同步计算、`spawn_blocking` 里跑;渲染仍是 `iced_widget::canvas::Program`,新增 `update()` 处理左键点击→选中某一行;选中态与详情缓存挂在 `App`(与既有 `git_log_cache` 同一层级),不挂 `Workspace`。

**Tech Stack:** Rust,`gleisbau` 0.7(已是依赖),`git2` 0.21(本计划新用到 `Repository::diff_tree_to_tree`/`Commit::parent`/`Diff::print`,前置计划已把它转正为直接依赖),iced 0.14。

## Global Constraints

- 只读:不新增暂存/提交/checkout/merge/rebase 等任何操作入口。
- 提交图渲染保持直线连线(不做贝塞尔曲线),YAGNI——spec §7 明确列为非目标。
- 合并提交(≥2 parent)的 diff 一律相对**第一父**计算,不做三方 diff(与 `git show` 默认行为一致)。
- 每个任务完成后 `cargo build -p dozer-app`、`cargo test -p dozer-app`、`cargo clippy -p dozer-app --all-targets -- -D warnings`、`cargo fmt -p dozer-app -- --check` 全部干净,才能进入下一个任务。

## 前置依赖(必须先完成)

本计划消费 `docs/superpowers/plans/2026-08-06-dozer-git2-data-layer-and-tree-status.md`(以下简称"Plan 1")产出的:

- `crate::delivery::WorktreeInfo { name, path, branch, dirty, is_current, missing }`
- `crate::workspace::Message::ProjectFsChanged(ProjectId, git_watch::Relevance)`,及其在 `workspace.rs` 里当前留的占位处理(Plan 1 Task 6 Step 5:只调用了 `spawn_project_git_refresh`,留了一句注释说 Plan 2 要在这里加 Git Log 快照重建——本计划 Task 6 就是去填这段)。
- `Workspace.worktrees: Vec<WorktreeInfo>` 字段。

如果 Plan 1 还没跑完,先跑那份计划。

## 参考:改动前的相关代码位置(spike 现状,`crates/dozer-app/src/git_log.rs` 全文 310 行)

- `CommitRow`(39-48 行)、`GitLogSnapshot`(50-60)、`build()`(91-158,内含硬编码 `MAX_COMMITS = 200`,20 行)、`GitLogCanvas`/`canvas::Program` 实现(160-228,只有 `draw`,没有 `update`)、`view()`(233-269)、既有测试 `build_against_real_repo`(279-308)。
- `crates/dozer-app/src/workspace.rs`:`App.git_log_cache`/`git_log_error` 字段(1339-1344)、初始化(2516-2517)、`LeftIconSelect` 里触发 `git_log::build` 的逻辑(3360-3383)、左图标栏按钮(5754-5759)、`left_panel_area` 渲染分支(5937-5938 调 `crate::git_log::view(...)`)。

---

### Task 1: `CommitRow` 加分支/tag 标签,`build()` 参数化 `max_count`

**Files:**
- Modify: `crates/dozer-app/src/git_log.rs:18-20`(去掉硬编码 `MAX_COMMITS`)
- Modify: `crates/dozer-app/src/git_log.rs:39-60`(`CommitRow`/`GitLogSnapshot` 加字段)
- Modify: `crates/dozer-app/src/git_log.rs:91-158`(`build()` 签名与实现)
- Modify: `crates/dozer-app/src/git_log.rs:279-308`(既有测试改调用签名)

**Interfaces:**
- Produces:
  ```rust
  pub const DEFAULT_MAX_COMMITS: usize = 200;
  pub const LOAD_MORE_STEP: usize = 200;
  pub enum RefKind { LocalBranch, RemoteBranch, Tag }
  pub struct RefLabel { pub name: String, pub kind: RefKind }
  // CommitRow 新增字段: pub refs: Vec<RefLabel>   (原有字段仍是 private,新加这个也保持 private——view()/canvas 在同一模块内访问)
  // GitLogSnapshot 新增: head_branch: Option<String>, max_count: usize
  // GitLogSnapshot 新增方法: pub fn max_count(&self) -> usize
  pub fn build(repo_path: &Path, max_count: usize) -> Result<GitLogSnapshot, String>
  ```

- [ ] **Step 1: 写失败测试(ref 标签 + 参数化 max_count)**

在 `git_log.rs` 的 `mod tests` 里,紧跟 `build_against_real_repo` 后面加:

```rust
#[test]
fn build_marks_head_branch_and_labels() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/dozer-app 应有两层上级目录到仓库根");
    let snapshot = build(repo_root, 50).expect("gleisbau 应能解析 dozer 自己的仓库");

    // 当前 HEAD 分支(dogfooding 仓库跑测试时几乎总在 main,但不强行假设
    // 分支名——只断言"存在且第 0 行的 refs 里能找到它")。
    let head = snapshot.head_branch.clone().expect("应能取到当前分支名");
    let first = &snapshot.rows[0];
    assert!(
        first.refs.iter().any(|r| r.name == head && r.kind == RefKind::LocalBranch),
        "HEAD 所在行的 refs 应包含当前分支: {:?}",
        first.refs
    );
}

#[test]
fn build_respects_max_count() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let small = build(repo_root, 5).unwrap();
    let bigger = build(repo_root, 50).unwrap();
    assert!(small.rows.len() <= 5);
    assert!(bigger.rows.len() > small.rows.len());
    assert_eq!(small.max_count(), 5);
    assert_eq!(bigger.max_count(), 50);
    // 重算稳定性:更大窗口的前 N 行应与小窗口结果一致(同一份历史,只是
    // 走得更远,不应该导致已经算出来的部分变形)。
    for (a, b) in small.rows.iter().zip(bigger.rows.iter()) {
        assert_eq!(a.short_sha, b.short_sha);
        assert_eq!(a.column, b.column);
    }
}
```

同时把既有的 `build_against_real_repo`/`build_marks_head_branch_and_labels`/`build_respects_max_count` 里对 `build(repo_root)` 的旧式单参数调用——也就是紧接着要改的 `build_against_real_repo`(279-308 行)测试体内的:

```rust
        let snapshot = build(repo_root).expect("gleisbau 应能解析 dozer 自己的仓库");
```

改成:

```rust
        let snapshot = build(repo_root, DEFAULT_MAX_COMMITS).expect("gleisbau 应能解析 dozer 自己的仓库");
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app git_log::tests::build_marks_head_branch_and_labels`
Expected: 编译失败(`build` 签名不对、`head_branch`/`RefKind`/`refs` 不存在)。

- [ ] **Step 3: 实现**

把第 18-20 行:

```rust
/// 一次性拉多少个 commit——够看出分叉/合并的形状,又不至于让 revwalk +
/// 分支归属分析在大仓库上明显卡顿(spike 阶段没做分页/增量)。
const MAX_COMMITS: usize = 200;
```

改成:

```rust
/// 首次打开面板拉多少个 commit——够看出分叉/合并的形状,又不至于让
/// revwalk + 分支归属分析在大仓库上明显卡顿。"加载更多"每次在当前基础上
/// 加这么多再整份重算(gleisbau 的 API 是"从头按 max_count 走一遍
/// revwalk",没有增量/游标接口,重算是唯一选项——见 build() 文档)。
pub const DEFAULT_MAX_COMMITS: usize = 200;
pub const LOAD_MORE_STEP: usize = 200;
```

把第 39-60 行的 `CommitRow`/`GitLogSnapshot` 定义:

```rust
pub struct CommitRow {
    column: usize,
    color_idx: usize,
    short_sha: String,
    summary: String,
    /// 父 commit 的 (row, column, color_idx),用于画连线;可能落在
    /// `MAX_COMMITS` 截断范围之外——那种父 commit 不出现在 `rows` 里,
    /// 此处已被过滤掉。
    parents: Vec<(usize, usize, usize)>,
}

pub struct GitLogSnapshot {
    repo_path: PathBuf,
    rows: Vec<CommitRow>,
    max_column: usize,
}

impl GitLogSnapshot {
    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }
}
```

改成:

```rust
/// 一个 commit 指向的引用(分支/远程分支/tag),供图上显示彩色标签。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    LocalBranch,
    RemoteBranch,
    Tag,
}

pub struct CommitRow {
    column: usize,
    color_idx: usize,
    short_sha: String,
    summary: String,
    /// 父 commit 的 (row, column, color_idx),用于画连线;可能落在
    /// `max_count` 截断范围之外——那种父 commit 不出现在 `rows` 里,
    /// 此处已被过滤掉。
    parents: Vec<(usize, usize, usize)>,
    /// 指向这个 commit 的分支/tag(可能为空)。
    refs: Vec<RefLabel>,
    /// 这个 commit 的完整 40 位 oid,选中详情(Task 2)用——`short_sha` 只
    /// 够显示,不够拿去 `git2::Repository::find_commit`。
    oid: git2::Oid,
}

pub struct GitLogSnapshot {
    repo_path: PathBuf,
    rows: Vec<CommitRow>,
    max_column: usize,
    /// 当前 HEAD 所在的本地分支名(detached HEAD 时为 `None`)——图上给这
    /// 个分支的标签加个 `→` 前缀区分"这是我现在checkout的那条"。
    head_branch: Option<String>,
    /// 这份快照实际请求的 `max_count`("加载更多"算下一次请求值用)。
    max_count: usize,
}

impl GitLogSnapshot {
    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    pub fn max_count(&self) -> usize {
        self.max_count
    }
}
```

把第 91-158 行的 `build()`:

```rust
/// 对 `repo_path` 跑一次 `gleisbau` 布局,产出可渲染快照。同步执行——
/// `MAX_COMMITS` 量级下 revwalk + 分支归属分析是毫秒级,spike 阶段不值得
/// 为此引入异步往返。
pub fn build(repo_path: &Path) -> Result<GitLogSnapshot, String> {
    let repository =
        gleisbau::get_repo(repo_path, false).map_err(|e| e.message().to_string())?;
    let settings = std::rc::Rc::new(default_settings()?);
    let graph = gleisbau::graph::Builder::new()
        .with_repository(repository)
        .with_settings(settings)
        .with_max_count(MAX_COMMITS)
        .build()?;

    let mut max_column = 0usize;
    let rows = graph
        .tracks
        .commits
        .iter()
        .map(|commit| {
            let b_idx = commit
                .branch_trace
                .ok_or_else(|| "commit 缺少 branch_trace".to_string())?;
            let column = graph
                .layout
                .track_visual(b_idx)
                .and_then(|v| v.column)
                .unwrap_or(0);
            max_column = max_column.max(column);
            let color_idx = b_idx.index();
            let git_commit = graph
                .commit(commit.oid)
                .map_err(|e| e.message().to_string())?;
            let full_sha = commit.oid.to_string();
            let short_sha = full_sha.chars().take(7).collect();
            let summary = git_commit
                .summary()
                .ok()
                .flatten()
                .unwrap_or("")
                .to_string();
            let parents = commit
                .parents
                .iter()
                .filter_map(|poid| {
                    let p_idx = *graph.tracks.indices.get(poid)?;
                    let p_commit = graph.tracks.commits.get(p_idx)?;
                    let p_b_idx = p_commit.branch_trace?;
                    let p_column = graph
                        .layout
                        .track_visual(p_b_idx)
                        .and_then(|v| v.column)
                        .unwrap_or(0);
                    Some((p_idx, p_column, p_b_idx.index()))
                })
                .collect();
            Ok(CommitRow {
                column,
                color_idx,
                short_sha,
                summary,
                parents,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(GitLogSnapshot {
        repo_path: repo_path.to_path_buf(),
        rows,
        max_column,
    })
}
```

改成:

```rust
/// 对 `repo_path` 跑一次 `gleisbau` 布局,产出可渲染快照。同步执行——
/// `max_count` 量级(几百到几千)下 revwalk + 分支归属分析是毫秒到低两位
/// 数毫秒级。gleisbau 没有增量/游标 API,"加载更多"就是拿更大的
/// `max_count` 再整份跑一遍,见 [`LOAD_MORE_STEP`]。
pub fn build(repo_path: &Path, max_count: usize) -> Result<GitLogSnapshot, String> {
    let repository =
        gleisbau::get_repo(repo_path, false).map_err(|e| e.message().to_string())?;
    let settings = std::rc::Rc::new(default_settings()?);
    let graph = gleisbau::graph::Builder::new()
        .with_repository(repository)
        .with_settings(settings)
        .with_max_count(max_count)
        .build()?;

    let head_branch = graph.head.is_branch.then(|| graph.head.name.clone());

    let mut max_column = 0usize;
    let rows = graph
        .tracks
        .commits
        .iter()
        .map(|commit| {
            let b_idx = commit
                .branch_trace
                .ok_or_else(|| "commit 缺少 branch_trace".to_string())?;
            let column = graph
                .layout
                .track_visual(b_idx)
                .and_then(|v| v.column)
                .unwrap_or(0);
            max_column = max_column.max(column);
            let color_idx = b_idx.index();
            let git_commit = graph
                .commit(commit.oid)
                .map_err(|e| e.message().to_string())?;
            let full_sha = commit.oid.to_string();
            let short_sha = full_sha.chars().take(7).collect();
            let summary = git_commit
                .summary()
                .ok()
                .flatten()
                .unwrap_or("")
                .to_string();
            let parents = commit
                .parents
                .iter()
                .filter_map(|poid| {
                    let p_idx = *graph.tracks.indices.get(poid)?;
                    let p_commit = graph.tracks.commits.get(p_idx)?;
                    let p_b_idx = p_commit.branch_trace?;
                    let p_column = graph
                        .layout
                        .track_visual(p_b_idx)
                        .and_then(|v| v.column)
                        .unwrap_or(0);
                    Some((p_idx, p_column, p_b_idx.index()))
                })
                .collect();
            let refs = graph
                .labels
                .get_labels(&commit.oid)
                .map(|labels| {
                    labels
                        .iter()
                        .map(|l| RefLabel {
                            name: l.name.clone(),
                            kind: match l.kind {
                                gleisbau::print::label::LabelType::LocalBranch => {
                                    RefKind::LocalBranch
                                }
                                gleisbau::print::label::LabelType::RemoteBranch => {
                                    RefKind::RemoteBranch
                                }
                                gleisbau::print::label::LabelType::Tag => RefKind::Tag,
                            },
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(CommitRow {
                column,
                color_idx,
                short_sha,
                summary,
                parents,
                refs,
                oid: commit.oid,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(GitLogSnapshot {
        repo_path: repo_path.to_path_buf(),
        rows,
        max_column,
        head_branch,
        max_count,
    })
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app git_log::tests::`
Expected: `build_against_real_repo`、`build_marks_head_branch_and_labels`、`build_respects_max_count` 均 PASS。此时 `cargo build -p dozer-app` 会在 `workspace.rs` 里报 `crate::git_log::build(&p)` 参数数量不对——这是预期的中间态,Task 5 会修。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/git_log.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): git_log CommitRow 加分支/tag 标签,build() 支持指定 max_count

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: 提交详情——改动文件列表 + diff 文本

**Files:**
- Modify: `crates/dozer-app/src/git_log.rs`(`build`/`default_settings` 之后新增)

**Interfaces:**
- Consumes: `git2`(直接依赖,Plan 1 Task 1 已转正)。
- Produces:
  ```rust
  pub struct DiffFileEntry {
      pub path: String,
      pub status: git2::Delta,
      pub patch: String, // unified diff 文本,单文件
  }
  pub struct CommitDetail {
      pub files: Vec<DiffFileEntry>,
  }
  pub fn commit_detail(repo_path: &Path, oid: git2::Oid) -> Result<CommitDetail, String>
  ```

- [ ] **Step 1: 写失败测试**

在 `mod tests` 里加:

```rust
#[test]
fn commit_detail_lists_files_and_patch_for_known_commit() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let head = build(repo_root, 1)
        .unwrap()
        .rows
        .into_iter()
        .next()
        .expect("仓库应至少有一个 commit")
        .oid;
    let detail = commit_detail(repo_root, head).expect("应能取到 HEAD 的详情");
    assert!(!detail.files.is_empty(), "HEAD 提交应该改了至少一个文件");
    let first = &detail.files[0];
    assert!(!first.path.is_empty());
    assert!(!first.patch.is_empty(), "patch 文本不该是空的");
}

#[test]
fn commit_detail_handles_root_commit_as_all_added() {
    let (_d, repo) = mkrepo_with_one_commit();
    let oid = {
        let r = git2::Repository::open(&repo).unwrap();
        r.head().unwrap().target().unwrap()
    };
    let detail = commit_detail(&repo, oid).expect("根提交也该能算详情");
    assert!(
        detail.files.iter().all(|f| f.status == git2::Delta::Added),
        "根提交(无父)相对空树全是新增: {:?}",
        detail.files.iter().map(|f| f.status).collect::<Vec<_>>()
    );
}

/// tempdir 里造一个只有一次提交的真 git 仓库,给这个模块自己的测试用
/// (跟 `delivery.rs` 的 `mkrepo` 是两份独立 helper,没有共享:两个模块各自
/// 只测自己需要的最小场景,没必要为了复用抽成公共 test util)。
fn mkrepo_with_one_commit() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_path_buf();
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
    (dir, repo)
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app git_log::tests::commit_detail_lists_files_and_patch_for_known_commit`
Expected: FAIL(`commit_detail`/`CommitDetail`/`DiffFileEntry` 不存在)。

- [ ] **Step 3: 实现**

在 `git_log.rs` 里,`build()` 函数结束、`struct GitLogCanvas` 定义之前,加:

```rust
/// 单个改动文件:哪个文件、什么类型的改动、这个文件自己的 unified diff
/// 文本(不是整个提交的 diff——按文件拆开,方便 UI 逐文件展开)。
/// 派生 `Clone`(`Message::GitLogDetailLoaded` 要装 `Result<CommitDetail, _>`,
/// `Message` 本身 `derive(Debug, Clone)`——workspace.rs:889,这两个结构体的
/// 字段类型都天然 `Debug + Clone`,一起派生即可)。
#[derive(Debug, Clone)]
pub struct DiffFileEntry {
    pub path: String,
    pub status: git2::Delta,
    pub patch: String,
}

#[derive(Debug, Clone)]
pub struct CommitDetail {
    pub files: Vec<DiffFileEntry>,
}

/// 取某个提交改动了哪些文件、每个文件的 diff 文本。合并提交(≥2 parent)
/// 相对**第一父**算(与 `git show` 默认行为一致,不做三方 diff——spec D5)。
/// 根提交(无 parent)相对空树算,等价于"全部文件都是新增"。
pub fn commit_detail(repo_path: &Path, oid: git2::Oid) -> Result<CommitDetail, String> {
    let repo = git2::Repository::open(repo_path).map_err(|e| e.message().to_string())?;
    let commit = repo.find_commit(oid).map_err(|e| e.message().to_string())?;
    let new_tree = commit.tree().map_err(|e| e.message().to_string())?;
    let old_tree = match commit.parent(0) {
        Ok(parent) => Some(parent.tree().map_err(|e| e.message().to_string())?),
        Err(_) => None, // 根提交,相对空树
    };
    let diff = repo
        .diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), None)
        .map_err(|e| e.message().to_string())?;

    let mut files: Vec<DiffFileEntry> = diff
        .deltas()
        .filter_map(|delta| {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())?
                .to_string_lossy()
                .into_owned();
            Some(DiffFileEntry {
                path,
                status: delta.status(),
                patch: String::new(), // 下面按文件路径回填
            })
        })
        .collect();

    // git2 的 `Diff::print` 是整份 diff 一次性回调、按行给,不是按文件给
    // 一整块文本——这里按 `DiffLine::origin_value()` 是不是文件头
    // (`FileHeader`)切分,把每一行追加到当前文件对应的 `patch` 里。
    let mut current_path: Option<String> = None;
    diff.print(git2::DiffFormat::Patch, |delta, _hunk, line| {
        if matches!(line.origin_value(), git2::DiffLineType::FileHeader) {
            current_path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.to_string_lossy().into_owned());
        }
        if let Some(path) = &current_path
            && let Some(entry) = files.iter_mut().find(|f| &f.path == path)
        {
            let prefix = match line.origin() {
                '+' | '-' | ' ' => line.origin().to_string(),
                _ => String::new(),
            };
            entry.patch.push_str(&prefix);
            entry
                .patch
                .push_str(&String::from_utf8_lossy(line.content()));
        }
        true
    })
    .map_err(|e| e.message().to_string())?;

    Ok(CommitDetail { files })
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app git_log::tests::commit_detail_`
Expected: 两个测试均 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/git_log.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): git_log::commit_detail 取提交的改动文件与 diff 文本

合并提交按第一父算 diff,根提交按相对空树算(全新增)。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: `App` 选中态 + 详情异步加载接线

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs:34`(import)
- Modify: `crates/dozer-app/src/workspace.rs:1339-1344`(`App` 字段)
- Modify: `crates/dozer-app/src/workspace.rs:2516-2517`(初始化)
- Modify: `crates/dozer-app/src/workspace.rs`(`Message` 枚举加两个 variant + 处理分支)
- Modify: `crates/dozer-app/src/workspace.rs:3360-3383`(`LeftIconSelect` 里 `build` 调用改参数)

**Interfaces:**
- Consumes: `git_log::{CommitDetail, commit_detail, DEFAULT_MAX_COMMITS}`(Task 1/2)。
- Produces:
  - `App.git_log_selected: Option<git2::Oid>`
  - `App.git_log_detail: Option<Result<git_log::CommitDetail, String>>`
  - `Message::GitLogSelectCommit(git2::Oid)`
  - `Message::GitLogDetailLoaded(PathBuf, git2::Oid, Result<git_log::CommitDetail, String>)`

- [ ] **Step 1: 字段与 import**

第 34 行,在现有 import 基础上加(如果 Plan 1 已经把这行改过,在那份改动结果上继续加,不要整行覆盖掉 Plan 1 加的 `FileGitStatus`/`WorktreeInfo`):

```rust
use crate::delivery::{self, FileChange, FileGitStatus, WorktreeInfo};
use crate::git_watch;
```

在这两行下面新加一行:

```rust
use crate::git_log;
```

第 1339-1344 行:

```rust
    /// spike(2026-08-06):Git 提交图缓存,`LeftIconSelect(LeftView::GitLog)`
    /// 激活时按当前项目路径同步计算一次(见 `git_log::build`)。切换项目或
    /// 重新点击图标不会自动刷新——spike 阶段没做失效策略。
    git_log_cache: Option<crate::git_log::GitLogSnapshot>,
    /// 上一次 `git_log::build` 失败的错误文案(`None`=未出错)。
    git_log_error: Option<String>,
```

改成:

```rust
    /// Git 提交图缓存,`LeftIconSelect(LeftView::GitLog)` 激活、`git_watch`
    /// 检测到 `.git` 引用变化(Plan 1 D4)、或点"加载更多"时重建(见
    /// `App::refresh_git_log`)。
    git_log_cache: Option<git_log::GitLogSnapshot>,
    /// 上一次 `git_log::build` 失败的错误文案(`None`=未出错)。
    git_log_error: Option<String>,
    /// 当前选中查看详情的提交(`None`=没选中,详情区不显示)。项目切换/
    /// 快照重建时随 `git_log_cache` 一起清空。
    git_log_selected: Option<git2::Oid>,
    /// 选中提交的详情异步加载结果。`None` 有两种含义:没选中,或选中了但
    /// 还在加载中——两者靠 `git_log_selected.is_some()` 区分(渲染层:
    /// selected 有值但 detail 是 None → 画"加载中…")。
    git_log_detail: Option<Result<git_log::CommitDetail, String>>,
```

第 2516-2517 行:

```rust
            git_log_cache: None,
            git_log_error: None,
```

改成:

```rust
            git_log_cache: None,
            git_log_error: None,
            git_log_selected: None,
            git_log_detail: None,
```

- [ ] **Step 2: `Message` 新增 variant**

找到 `Message` 枚举里 `LeftIconSelect` 附近(既有的 spike 代码没有单独给 GitLog 开消息,直接找 `ProjectFsChanged` —— Plan 1 Task 6 加的那个——紧接着加两条):

```rust
    /// Git Log 面板:选中一行提交,触发异步取改动文件+diff。
    GitLogSelectCommit(git2::Oid),
    /// Git Log 面板:某个提交的详情异步加载完成。带上请求时的仓库路径与
    /// oid,处理时校验"这份结果还对不对得上当前状态"(项目切换/换选中项
    /// 后,旧请求的结果要被丢弃,不能覆盖新状态——见处理分支注释)。
    GitLogDetailLoaded(PathBuf, git2::Oid, Result<git_log::CommitDetail, String>),
    /// Git Log 面板:点"加载更多",拿更大的 `max_count` 重新跑一次
    /// `git_log::build`。
    GitLogLoadMore,
```

- [ ] **Step 3: 抽出 `refresh_git_log` 辅助方法**

在 `impl App` 块里(`spawn_project_git_refresh` 所在的 `impl Workspace` 是另一个 `impl` 块——`git_log_cache` 是 `App` 的字段,这里要加到 `impl App` 里,紧邻现有 `LeftIconSelect` 处理逻辑所在的 `update` 方法之外找个合适位置,比如挨着其他 `fn spawn_xxx` 私有方法)加:

```rust
    /// 对当前项目重建 Git Log 快照,`max_count` 由调用方决定(打开面板/
    /// 引用变化用 `git_log::DEFAULT_MAX_COMMITS`,"加载更多"用当前值 +
    /// `git_log::LOAD_MORE_STEP`)。同步跑在 `spawn_blocking` 里,完成后
    /// 直接写回 `self.git_log_cache`/`git_log_error`——调用方在 `update()`
    /// 内部同步调用,不需要经消息往返(与既有 spike 逻辑一致,只是抽成了
    /// 一个方法,给 Task 6"引用变化时重建"复用)。
    fn refresh_git_log(&mut self, repo_path: &std::path::Path, max_count: usize) {
        match git_log::build(repo_path, max_count) {
            Ok(snapshot) => {
                self.git_log_cache = Some(snapshot);
                self.git_log_error = None;
            }
            Err(err) => {
                self.git_log_cache = None;
                self.git_log_error = Some(err);
            }
        }
        self.git_log_selected = None;
        self.git_log_detail = None;
    }
```

- [ ] **Step 4: 改 `LeftIconSelect` 里的旧调用,加新消息处理分支**

第 3360-3383 行(spike 留下的这一段):

```rust
                if self.left_view == LeftView::GitLog {
                    let path = self.active_workspace().and_then(|ws| ws.active_project_path());
                    match path {
                        Some(p)
                            if self
                                .git_log_cache
                                .as_ref()
                                .is_none_or(|c| c.repo_path() != p) =>
                        {
                            match crate::git_log::build(&p) {
                                Ok(snapshot) => {
                                    self.git_log_cache = Some(snapshot);
                                    self.git_log_error = None;
                                }
                                Err(err) => {
                                    self.git_log_cache = None;
                                    self.git_log_error = Some(err);
                                }
                            }
                        }
                        None => {
                            self.git_log_cache = None;
                            self.git_log_error = None;
                        }
                        _ => {}
                    }
                }
```

改成(改用 `refresh_git_log`,签名从一个参数变两个):

```rust
                if self.left_view == LeftView::GitLog {
                    let path = self.active_workspace().and_then(|ws| ws.active_project_path());
                    match path {
                        Some(p)
                            if self
                                .git_log_cache
                                .as_ref()
                                .is_none_or(|c| c.repo_path() != p) =>
                        {
                            self.refresh_git_log(&p, git_log::DEFAULT_MAX_COMMITS);
                        }
                        None => {
                            self.git_log_cache = None;
                            self.git_log_error = None;
                            self.git_log_selected = None;
                            self.git_log_detail = None;
                        }
                        _ => {}
                    }
                }
```

在 `Message::ProjectFsChanged` 处理分支旁边(Plan 1 Task 6 加的那个 match arm)加三个新分支:

```rust
            Message::GitLogSelectCommit(oid) => {
                self.git_log_selected = Some(oid);
                self.git_log_detail = None;
                let Some(repo_path) = self.git_log_cache.as_ref().map(|c| c.repo_path().to_path_buf()) else {
                    return;
                };
                let proxy = self.proxy.clone();
                self.handle.spawn(async move {
                    let repo_path2 = repo_path.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        git_log::commit_detail(&repo_path2, oid)
                    })
                    .await
                    .unwrap_or_else(|e| Err(format!("详情加载任务失败: {e}")));
                    let _ = proxy.send_event(Message::GitLogDetailLoaded(repo_path, oid, result));
                });
            }
            Message::GitLogDetailLoaded(repo_path, oid, result) => {
                let still_current = self.git_log_cache.as_ref().map(|c| c.repo_path()) == Some(repo_path.as_path())
                    && self.git_log_selected == Some(oid);
                if still_current {
                    self.git_log_detail = Some(result);
                }
                // 否则:项目已切换,或用户点了别的提交——这份结果过期了,丢弃。
            }
            Message::GitLogLoadMore => {
                let Some(path) = self.active_workspace().and_then(|ws| ws.active_project_path()) else {
                    return;
                };
                let next = self
                    .git_log_cache
                    .as_ref()
                    .map(|c| c.max_count() + git_log::LOAD_MORE_STEP)
                    .unwrap_or(git_log::DEFAULT_MAX_COMMITS);
                // refresh_git_log 会清掉 selected/detail——"加载更多"不该
                // 打断用户正在看的详情,所以这里手动重建快照后把选中态还原。
                let selected = self.git_log_selected;
                self.refresh_git_log(&path, next);
                if let Some(oid) = selected {
                    self.update(Message::GitLogSelectCommit(oid));
                }
            }
```

- [ ] **Step 5: 编译检查**

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: 剩下的错误应集中在 `git_log.rs` 里 `view()`/`GitLogCanvas` 还没跟着更新(Task 4)、`left_panel_area` 调用 `crate::git_log::view` 的参数还是旧签名(Task 4/5 一起改)。如果 `App` 结构体字面量初始化(`new_shell` 之外还有没有别的地方构造 `App`?搜 `grep -n "App {" crates/dozer-app/src/workspace.rs`)报"缺字段",按同样的模式把 `git_log_selected: None, git_log_detail: None,` 加进去。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): Git Log 面板接入选中态与详情异步加载消息

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

（此时整个 crate 大概率仍编译不过,卡在 `git_log.rs` 本身的 `view`/`GitLogCanvas` 签名——继续 Task 4。）

---

### Task 4: 提交图可点击选中 + 详情子面板渲染

**Files:**
- Modify: `crates/dozer-app/src/git_log.rs`(`GitLogCanvas`/`canvas::Program` 实现、`view()`)

**Interfaces:**
- Consumes: `Message::GitLogSelectCommit`(Task 3)、`CommitDetail`/`DiffFileEntry`(Task 2)。
- Produces: `pub fn view<'a>(snapshot: Option<&'a GitLogSnapshot>, error: Option<&'a str>, selected: Option<git2::Oid>, detail: Option<&'a Result<CommitDetail, String>>) -> Element<'a, Message, ...>`(签名比 spike 多两个参数;Task 5 会再加 worktree 相关参数,一次改到位免得改两轮——见 Step 3 末尾的完整签名)。

- [ ] **Step 1: 给 `GitLogCanvas` 加 `update()`,点击行→发消息**

把第 171-228 行的 `impl canvas::Program ... for GitLogCanvas<'_>`:

```rust
impl canvas::Program<Message, iced_widget::Theme, iced_widget::Renderer> for GitLogCanvas<'_> {
    type State = ();

    fn draw(
```

改成(在 `type State = ();` 后面、`fn draw` 前面插入 `fn update`):

```rust
impl canvas::Program<Message, iced_widget::Theme, iced_widget::Renderer> for GitLogCanvas<'_> {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: &iced_widget::core::Event,
        bounds: Rectangle,
        cursor: iced_widget::core::mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let iced_widget::core::Event::Mouse(iced_widget::core::mouse::Event::ButtonPressed(
            iced_widget::core::mouse::Button::Left,
        )) = event
        else {
            return None;
        };
        let pos = cursor.position_in(bounds)?;
        if pos.x < 0.0 || pos.y < 0.0 {
            return None;
        }
        let row_idx = (pos.y / ROW_HEIGHT) as usize;
        let row = self.snapshot.rows.get(row_idx)?;
        Some(canvas::Action::publish(Message::GitLogSelectCommit(row.oid)).and_capture())
    }

    fn draw(
```

- [ ] **Step 2: 画分支/tag 标签,选中行加高亮描边**

在 `draw()` 里(174-228 行区间),第二个 `for (row_idx, commit) in self.snapshot.rows.iter().enumerate()` 循环(201-224 行,画圆点+文字那个)里,把:

```rust
        for (row_idx, commit) in self.snapshot.rows.iter().enumerate() {
            let center = row_center(row_idx, commit.column);
            let color = track_color(commit.color_idx);
            frame.fill(
                &canvas::Path::circle(center, DOT_RADIUS),
                color,
            );

            frame.with_save(|frame| {
                frame.translate(Vector::new(
                    text_x,
                    row_idx as f32 * ROW_HEIGHT + ROW_HEIGHT * 0.5,
                ));
                frame.fill_text(canvas::Text {
                    content: format!("{}  {}", commit.short_sha, commit.summary),
                    position: Point::ORIGIN,
                    color: theme::CREAM,
                    size: Pixels(workspace_font::body() as f32),
                    align_y: alignment::Vertical::Center,
                    font: Font::MONOSPACE,
                    ..canvas::Text::default()
                });
            });
        }
```

改成:

```rust
        for (row_idx, commit) in self.snapshot.rows.iter().enumerate() {
            let center = row_center(row_idx, commit.column);
            let color = track_color(commit.color_idx);
            if self.selected == Some(commit.oid) {
                frame.stroke(
                    &canvas::Path::circle(center, DOT_RADIUS + 2.5),
                    canvas::Stroke::default()
                        .with_color(theme::GOLD)
                        .with_width(1.5),
                );
            }
            frame.fill(&canvas::Path::circle(center, DOT_RADIUS), color);

            let refs_prefix = ref_labels_text(&commit.refs, self.head_branch.as_deref());

            frame.with_save(|frame| {
                frame.translate(Vector::new(
                    text_x,
                    row_idx as f32 * ROW_HEIGHT + ROW_HEIGHT * 0.5,
                ));
                let content = if refs_prefix.is_empty() {
                    format!("{}  {}", commit.short_sha, commit.summary)
                } else {
                    format!("{}  {}  {}", commit.short_sha, refs_prefix, commit.summary)
                };
                frame.fill_text(canvas::Text {
                    content,
                    position: Point::ORIGIN,
                    color: theme::CREAM,
                    size: Pixels(workspace_font::body() as f32),
                    align_y: alignment::Vertical::Center,
                    font: Font::MONOSPACE,
                    ..canvas::Text::default()
                });
            });
        }
```

（这里把"ref 标签"简化成彩色前缀文本而不是真的画背景色块 pill——iced canvas 没有现成的圆角矩形 helper,自己拼一个纯属视觉打磨,不是本计划的目标,spec §7 已经把"提交图的精细排版打磨"列为非目标。真要加背景色块,后续单独提个小任务。当前先用方括号文本区分,颜色仍按 `RefKind` 走,足够看出"这是哪个分支/tag"。）

在 `track_color` 函数(33-35 行)后面加一个新的纯函数,和 `refs`/`head_branch` 的着色逻辑放在一起方便单测:

```rust
/// 把一行 commit 的 `refs` 拼成形如 `[main][origin/main]` 的前缀文本;当前
/// HEAD 所在的本地分支加 `→` 标记(`[→main]`)。空 `refs` 返回空字符串。
/// 不在这里上色——canvas 文本整体只有一个 `Color`,没法给子串单独上色,
/// 颜色区分留给后续真的做背景色块 pill 时再处理(见上面 Step 2 的注释)。
fn ref_labels_text(refs: &[RefLabel], head_branch: Option<&str>) -> String {
    refs.iter()
        .map(|r| {
            let marker = if head_branch == Some(r.name.as_str()) { "→" } else { "" };
            format!("[{marker}{}]", r.name)
        })
        .collect::<Vec<_>>()
        .join("")
}
```

- [ ] **Step 3: `GitLogCanvas`/`view()` 加选中态字段与参数**

把第 160-162 行:

```rust
struct GitLogCanvas<'a> {
    snapshot: &'a GitLogSnapshot,
}
```

改成:

```rust
struct GitLogCanvas<'a> {
    snapshot: &'a GitLogSnapshot,
    selected: Option<git2::Oid>,
    head_branch: Option<&'a str>,
}
```

把第 233-269 行的 `view()`:

```rust
pub fn view<'a>(
    snapshot: Option<&'a GitLogSnapshot>,
    error: Option<&'a str>,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    if let Some(err) = error {
        return container(text(format!("git log 读取失败: {err}")).color(theme::RED))
            .padding(16)
            .into();
    }
    let Some(snapshot) = snapshot else {
        return container(text("未打开项目").color(theme::DIM))
            .padding(16)
            .into();
    };
    if snapshot.rows.is_empty() {
        return container(text("没有可显示的提交").color(theme::DIM))
            .padding(16)
            .into();
    }
    let height = ROW_HEIGHT * snapshot.rows.len() as f32;
    let canvas: Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> =
        Canvas::new(GitLogCanvas { snapshot })
            .width(Length::Fill)
            .height(Length::Fixed(height))
            .into();
    let header = row![text(snapshot.repo_path.display().to_string())
        .size(workspace_font::caption())
        .color(theme::DIM)]
    .padding([4, 8]);
    column![
        header,
        scrollable(canvas).width(Length::Fill).height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}
```

改成(先只加选中态/详情参数,worktree 速览条留给 Task 5 追加,避免这一步的 diff 和下一步的 diff 混在一起难review):

```rust
/// 渲染整块提交图面板:有数据画 Canvas + 加载更多 + 详情子面板,出错画
/// 错误文案,两者皆无(比如尚未打开项目)画空状态提示。纯函数——不碰
/// `App`/`Workspace` 内部状态,调用方(`workspace.rs`)负责取数据、决定
/// 何时重建缓存。
pub fn view<'a>(
    snapshot: Option<&'a GitLogSnapshot>,
    error: Option<&'a str>,
    selected: Option<git2::Oid>,
    detail: Option<&'a Result<CommitDetail, String>>,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    if let Some(err) = error {
        return container(text(format!("git log 读取失败: {err}")).color(theme::RED))
            .padding(16)
            .into();
    }
    let Some(snapshot) = snapshot else {
        return container(text("未打开项目").color(theme::DIM))
            .padding(16)
            .into();
    };
    if snapshot.rows.is_empty() {
        return container(text("没有可显示的提交").color(theme::DIM))
            .padding(16)
            .into();
    }
    let height = ROW_HEIGHT * snapshot.rows.len() as f32;
    let canvas: Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> = Canvas::new(
        GitLogCanvas {
            snapshot,
            selected,
            head_branch: snapshot.head_branch.as_deref(),
        },
    )
    .width(Length::Fill)
    .height(Length::Fixed(height))
    .into();
    let header = row![
        text(snapshot.repo_path.display().to_string())
            .size(workspace_font::caption())
            .color(theme::DIM),
        iced_widget::space::horizontal(),
        iced_widget::button(text("加载更多").size(workspace_font::caption()))
            .on_press(Message::GitLogLoadMore)
            .style(|_t, _s| iced_widget::button::Style {
                background: None,
                text_color: theme::CYAN,
                ..iced_widget::button::Style::default()
            }),
    ]
    .padding([4, 8])
    .align_y(iced_widget::core::Alignment::Center);
    let graph_area: Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> = column![
        header,
        scrollable(canvas).width(Length::Fill).height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into();

    let Some(_) = selected else {
        return graph_area;
    };
    column![graph_area, detail_view(detail)]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// 选中提交的详情子面板:加载中(`detail` 为 `None`)/出错/文件列表 + 每个
/// 文件的 diff 文本(全部展开显示,不做逐文件折叠——文件数一般不多,折叠
/// 交互属于打磨,不在本计划范围)。
fn detail_view<'a>(
    detail: Option<&'a Result<CommitDetail, String>>,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some(detail) = detail else {
        return container(text("加载详情中…").color(theme::DIM))
            .padding(16)
            .height(Length::Fixed(120.0))
            .into();
    };
    let body: Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> = match detail {
        Err(err) => text(format!("详情加载失败: {err}")).color(theme::RED).into(),
        Ok(detail) if detail.files.is_empty() => text("这个提交没有文件改动").color(theme::DIM).into(),
        Ok(detail) => {
            let mut col = column![].spacing(10);
            for f in &detail.files {
                let status_label = match f.status {
                    git2::Delta::Added => "新增",
                    git2::Delta::Deleted => "删除",
                    git2::Delta::Renamed => "重命名",
                    _ => "修改",
                };
                col = col.push(
                    text(format!("{status_label}  {}", f.path))
                        .size(workspace_font::label())
                        .color(theme::GOLD),
                );
                col = col.push(
                    text(f.patch.clone())
                        .size(workspace_font::caption())
                        .color(theme::BODY)
                        .font(Font::MONOSPACE),
                );
            }
            scrollable(col).height(Length::Fixed(240.0)).into()
        }
    };
    container(body)
        .padding(16)
        .width(Length::Fill)
        .height(Length::Fixed(240.0))
        .into()
}
```

- [ ] **Step 4: 更新 `workspace.rs` 里对 `git_log::view` 的调用点**

第 5937-5938 行(现在按 Task 3 加的字段,`view` 需要四个参数):

```rust
        LeftView::GitLog => {
            crate::git_log::view(app.git_log_cache.as_ref(), app.git_log_error.as_deref())
        }
```

改成:

```rust
        LeftView::GitLog => crate::git_log::view(
            app.git_log_cache.as_ref(),
            app.git_log_error.as_deref(),
            app.git_log_selected,
            app.git_log_detail.as_ref(),
        ),
```

- [ ] **Step 5: 全量编译测试**

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: 干净编译。如果报错在 Task 5(worktree 速览条)还没接的地方,先跳过——Task 5 会补齐 `view()` 的最终签名。如果这里报的是本任务自己引入的类型/借用错误,照编译器提示修,不要绕过。

Run: `cargo test -p dozer-app`
Expected: 全过。

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings`
Expected: 干净。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/git_log.rs crates/dozer-app/src/workspace.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): 提交图可点击选中,加详情子面板与加载更多

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Worktree 速览条

**Files:**
- Modify: `crates/dozer-app/src/git_log.rs`(`view()` 加 worktree 参数与渲染)
- Modify: `crates/dozer-app/src/workspace.rs:5937-5938`(调用点传 `&ws.worktrees`)

**Interfaces:**
- Consumes: `delivery::WorktreeInfo`(Plan 1)、`Message::ProjectTabOpen(PathBuf)`(既有消息,main.rs `Message::ProjectTabPickFolder` 旁边那条,见 `workspace.rs:1044`)。
- Produces: `git_log::view` 最终签名(本任务是这个函数签名的最后一次改动):
  ```rust
  pub fn view<'a>(
      snapshot: Option<&'a GitLogSnapshot>,
      error: Option<&'a str>,
      selected: Option<git2::Oid>,
      detail: Option<&'a Result<CommitDetail, String>>,
      worktrees: &'a [WorktreeInfo],
  ) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>
  ```

- [ ] **Step 1: `git_log.rs` 加 import,`view()` 加参数与速览条渲染**

在文件顶部 import 区(9-16 行)加一行:

```rust
use crate::delivery::WorktreeInfo;
```

把 Task 4 Step 3 改完之后的 `view()` 签名:

```rust
pub fn view<'a>(
    snapshot: Option<&'a GitLogSnapshot>,
    error: Option<&'a str>,
    selected: Option<git2::Oid>,
    detail: Option<&'a Result<CommitDetail, String>>,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
```

改成:

```rust
pub fn view<'a>(
    snapshot: Option<&'a GitLogSnapshot>,
    error: Option<&'a str>,
    selected: Option<git2::Oid>,
    detail: Option<&'a Result<CommitDetail, String>>,
    worktrees: &'a [WorktreeInfo],
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
```

在 `view()` 函数体内,`if let Some(err) = error { ... }` 分支**之前**插入 worktree 速览条的构造(它在 error/空态时也该显示——哪怕图还没画出来,用户也可能想先看看有哪些 worktree):

```rust
    let strip = worktree_strip(worktrees);
```

然后把函数体里所有 `return container(...).into();`(错误态、未打开项目态、空提交态三处)前面都加上这条速览条,比如把:

```rust
    if let Some(err) = error {
        return container(text(format!("git log 读取失败: {err}")).color(theme::RED))
            .padding(16)
            .into();
    }
```

改成:

```rust
    if let Some(err) = error {
        return column![
            strip,
            container(text(format!("git log 读取失败: {err}")).color(theme::RED)).padding(16),
        ]
        .into();
    }
```

对"未打开项目"和"没有可显示的提交"两处做同样的包法(`column![strip, container(...)...]`)。函数末尾正常渲染 `graph_area`/详情那部分,把最后的:

```rust
    let Some(_) = selected else {
        return graph_area;
    };
    column![graph_area, detail_view(detail)]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
```

改成:

```rust
    let Some(_) = selected else {
        return column![strip, graph_area].width(Length::Fill).height(Length::Fill).into();
    };
    column![strip, graph_area, detail_view(detail)]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
```

在 `ref_labels_text` 函数后面加 `worktree_strip`:

```rust
/// 面板顶部的 worktree 速览条:同仓库每个 worktree 一个可点条目
/// (`name · branch{*if dirty}`),点击直接复用既有"打开项目"消息——已经
/// 开着页签就聚焦、没开就新开(`Message::ProjectTabOpen` 的既有语义,见
/// `workspace.rs` 里 `ProjectTabOpen`/`ProjectTabOpened` 处理分支)。
/// `missing`(git 元数据还在但目录已被删)的项置灰不可点。空列表(比如
/// 这个仓库压根没有额外 worktree)不画任何东西。
fn worktree_strip(worktrees: &[WorktreeInfo]) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    if worktrees.is_empty() {
        return iced_widget::space::vertical().height(Length::Fixed(0.0)).into();
    }
    let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> = Vec::new();
    for w in worktrees {
        let label = match &w.branch {
            Some(b) if w.dirty => format!("{}  ·  {b}*", w.name),
            Some(b) => format!("{}  ·  {b}", w.name),
            None => format!("{}  ·  —", w.name),
        };
        let color = if w.missing {
            theme::DIM
        } else if w.is_current {
            theme::GOLD
        } else {
            theme::CYAN
        };
        let chip = container(text(label).size(workspace_font::caption()).color(color))
            .padding([4, 8]);
        if w.missing {
            items.push(chip.into());
        } else {
            items.push(
                iced_widget::button(chip)
                    .on_press(Message::ProjectTabOpen(w.path.clone()))
                    .style(|_t, _s| iced_widget::button::Style {
                        background: None,
                        ..iced_widget::button::Style::default()
                    })
                    .into(),
            );
        }
    }
    scrollable(row(items).spacing(4).padding([4, 8]))
        .direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::default(),
        ))
        .into()
}
```

- [ ] **Step 2: `workspace.rs` 调用点补最后一个参数**

第 5937-5938 行(Task 4 改完之后的样子):

```rust
        LeftView::GitLog => crate::git_log::view(
            app.git_log_cache.as_ref(),
            app.git_log_error.as_deref(),
            app.git_log_selected,
            app.git_log_detail.as_ref(),
        ),
```

改成:

```rust
        LeftView::GitLog => crate::git_log::view(
            app.git_log_cache.as_ref(),
            app.git_log_error.as_deref(),
            app.git_log_selected,
            app.git_log_detail.as_ref(),
            ws.worktrees.as_slice(),
        ),
```

（`ws` 是这个函数已有的参数,`Workspace.worktrees` 是 Plan 1 加的字段,同一个包内 `workspace.rs` 自己的私有字段,直接访问没有可见性问题。）

- [ ] **Step 3: 全量验证**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets -- -D warnings
cargo fmt -p dozer-app -- --check
```

Expected: 四条全部干净通过。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/git_log.rs crates/dozer-app/src/workspace.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): Git Log 面板加 worktree 速览条,点击打开为项目页签

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: 引用变化时自动重建 Git Log 快照

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(`Message::ProjectFsChanged` 处理分支,Plan 1 Task 6 Step 5 留的占位)

**Interfaces:**
- Consumes: `git_watch::Relevance`(Plan 1)、`App::refresh_git_log`(Task 3)。

- [ ] **Step 1: 补上 `Relevance::GitRefs` 分支**

Plan 1 执行完之后,`Message::ProjectFsChanged` 的处理分支长这样:

```rust
            Message::ProjectFsChanged(project_id, _relevance) => {
                // Plan 2 会在这里按 `_relevance == Relevance::GitRefs` 加一段
                // Git Log 快照重建;这里先只管文件树/worktree 刷新。
                self.with_project(project_id, |ws, io| {
                    ws.spawn_project_git_refresh(io);
                });
            }
```

改成:

```rust
            Message::ProjectFsChanged(project_id, relevance) => {
                self.with_project(project_id, |ws, io| {
                    ws.spawn_project_git_refresh(io);
                });
                // 只有 `.git` 引用类变化(分支切换/外部提交/其他 worktree
                // 提交)才值得重建 Git Log 快照——纯工作区文件编辑不影响
                // 提交历史,重算是纯浪费。只在这个项目的面板缓存已经建过
                // 一次、且路径匹配时才重建(用户可能根本没打开过 Git Log
                // 面板,`git_log_cache` 是 `None` 就没必要现在算)。
                if relevance == git_watch::Relevance::GitRefs
                    && let Some(repo_path) = self
                        .git_log_cache
                        .as_ref()
                        .map(|c| c.repo_path().to_path_buf())
                    && self
                        .active_workspace()
                        .and_then(|ws| ws.active_project_path())
                        .as_deref()
                        == Some(repo_path.as_path())
                {
                    let max = self
                        .git_log_cache
                        .as_ref()
                        .map(|c| c.max_count())
                        .unwrap_or(git_log::DEFAULT_MAX_COMMITS);
                    self.refresh_git_log(&repo_path, max);
                }
            }
```

- [ ] **Step 2: 全量验证**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets -- -D warnings
cargo fmt -p dozer-app -- --check
```

Expected: 四条全部干净通过。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): .git 引用变化时自动重建 Git Log 快照

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: 手动验收

**Files:** 无代码改动,纯验证。

- [ ] **Step 1: 用 `run` 技能或直接 `cargo run -p dozer-app` 启动,打开 dozer 自己这个仓库**

1. 点左图标栏第三个(Git)图标 → 应看到提交图,HEAD 所在分支那行的摘要文字前有 `[→main]`(或当前实际分支名)标签。
2. 点一个含合并(比如 `2429d15`)的历史提交行 → 圆点应出现金色描边(选中态),下方详情区应加载出改动文件列表 + 每个文件的 diff 文本。
3. 点"加载更多" → 图应变长,继续往下能看到更早的提交,且之前已经在看的部分位置不变(不应该跳动/重排)。
4. 在外部终端对这个仓库 `git worktree add ../dozer-wt-test -b test-manual` → 几百毫秒内(不用手动刷新)面板顶部速览条应出现这个新 worktree。
5. 点速览条里的另一个 worktree → 应作为新项目页签打开,顶栏出现新 tab。
6. 在外部终端对主仓库 `git commit`(哪怕是空提交 `git commit --allow-empty -m test`)→ 面板的提交图应在亚秒级自动多出这一行,不需要手动切换面板。

- [ ] **Step 2: 清理手动验收留下的痕迹**

```bash
git worktree remove ../dozer-wt-test --force 2>/dev/null || rm -rf ../dozer-wt-test
git branch -D test-manual 2>/dev/null
```

（如果 Step 1.6 用了 `--allow-empty` 的测试提交,`git reset --hard HEAD~1` 撤掉——先确认那确实是刚才手动验收加的那个空提交,不要盲目 reset。）

---

## 完成后的状态

- `git_log.rs` 的 spike 状态清除:`build()` 参数化、有分支/tag 标签、有提交详情、有点击交互、有 worktree 速览条、有加载更多。
- 提交图不再是打开一次就定死的 200 条快照:引用变化自动重建,面板内可以主动加载更多。
- 与 Plan 1 的 `delivery::WorktreeInfo`/`git_watch::Relevance` 完全打通,worktree 速览条复用既有 `Message::ProjectTabOpen`,没有引入新的"打开项目"路径。
