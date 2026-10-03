# bytegit P1:状态 / HEAD / 分支 / 远程迁移 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 `bytegit` 增加 HEAD、本地分支、远程、工作区状态四类只读 API(发 `v0.2.0`),并让 dozer 的 `delivery.rs` 查询函数与 `git_hotspots` 的 `dirty_paths`/`head_short_sha` 改用它,消除工作区状态 ×3 的重复实现,行为保持等价。

**Architecture:** P1 分两半。前半在 `bytegit` 仓库加 `info.rs`(HEAD/分支/远程)与 `status.rs`(工作区状态),全部在 `Repo` 上、不暴露 `git2` 类型。后半在 dozer 的独立 worktree 里,把 `delivery.rs` 里的查询函数**改成 bytegit 的薄适配层,函数名和签名不变**,所以 ~25 个调用点这一阶段不动;适配层的删除留给 P6。迁移用"先在旧实现上写刻画测试并通过,再换实现保持通过"证明等价。

**Tech Stack:** Rust edition 2024、`git2 0.21`(仅 bytegit 内部)、dozer 侧 `bytegit` 以 git tag 依赖。

**Spec:** `docs/superpowers/specs/2026-10-02-bytegit-design.md`(§4.2、§4.3、§6 的 P1)。背景:`docs/dozer-v2/bytegit-调用点盘点.md`。前序计划:`docs/superpowers/plans/2026-10-02-bytegit-p0-skeleton.md`(已完成,`v0.1.0`)。

**本计划中的代码已在草稿里完整跑过:** bytegit 49 个测试通过,clippy(默认与 `--all-features`)与 fmt 干净;dozer 侧在一个临时 worktree 里实际编译,`delivery::` 29 个、`git_hotspots::` 17 个测试通过,新增的刻画测试在**旧实现和新实现上都通过**(这是等价性的证据)。执行者照抄即可;结果与此不符先停下排查,不要改测试迎合实现。

## Global Constraints

- bytegit:Rust edition `2024`;公开 API 不得出现 `git2::*`;全部同步;不依赖 tokio/iced;`git2 = "0.21"` 与 dozer 对齐。
- bytegit 的测试夹具(`TempRepo`)不得依赖用户全局 git 配置(P0 已隔离)。
- dozer 侧**所有命令都在专用 worktree 里执行,路径写全**(`~/Projects/CoralProjects/byteboy/dozer-bytegit-p1/...`);主 checkout 常有并发会话与未提交改动,不得在其上改代码或 `git add -A`。提交前 `git status` 与 `git diff --cached --stat` 看全貌,只 `git add` 指定路径。
- dozer 的 `bytegit` 依赖一律用 tag:`bytegit = { git = "https://github.com/byteboyai/bytegit", tag = "v0.2.0" }`;本地联调的 `[patch]` 只放 `.cargo/config.toml`,**不提交**。
- 迁移阶段**不得改变用户可见行为**。发现旧行为有 bug 时,先等价迁移,再另起提交修复(本计划只有一处有意的差异,见 Task 5 与"待决事项")。
- 提交信息用 `feat:`/`fix:`/`refactor:`/`test:`/`docs:` 前缀;提交末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。
- dozer 门禁:`cargo test -p dozer-app` 与 `cargo clippy -p dozer-app --all-targets` 在**未改动的 main 上就有失败/告警**(见下"已知基线"),所以门禁是"**触碰的文件不产生新诊断、基线之外无新失败**",不是"全绿"。
- git2 0.21 的形状:`Reference::shorthand()`、`Remote::url()`、`Remote::pushurl()` 返回 `Result`,`Branch::name()` 返回 `Result<Option<&str>>`,`remotes().iter()` 的元素是 `Result<Option<&str>>`。

## 已知基线(在未改动的 main 上观察到,不是本计划引入的)

- `extensions::files::tests::delete_confirm_spec_reflects_pending_target` 在 main 上就失败(断言标题里含 `/tmp/` 前缀,与 macOS 的临时目录路径有关)。
- `extensions::git_log::tests::build_marks_head_branch_and_labels` 在 main 上通过,但在**新建分支的 linked worktree 里失败**(它对 dozer 自己的仓库跑 `gleisbau`,断言当前分支名出现在 HEAD 行的 refs 里)。它不经过 `delivery`/`bytegit`;我只验证了"main 上过、worktree 里不过",没有查清根因。合并回 main 后在 main 上重跑一次确认。
- `cargo clippy -p dozer-app --all-targets -- -D warnings` 在 main 上有 14–16 条既有错误(`homespace.rs`、`ssh.rs`、`todo/view.rs`、`files/update.rs` 等),**没有一条在 `delivery.rs` 或 `git_hotspots.rs`**。

## Review Focus

规格对下列输入没有逐条说明,但真实使用者会遇到;每条都由指明的任务里的测试固定下来(行为均在草稿里实测确认):

1. **项目路径在仓库子目录里**:迁移前 `branch/is_dirty/file_statuses/current_branch_has_commits` 用 `Repository::open`(只认仓库根),在子目录里等于"不是仓库";`repo_root/local_branches/remote_url` 用命令行,会向上找。两组语义必须原样保留。Task 4 `project_in_a_repo_subdirectory_is_treated_as_not_a_repo_by_open_based_queries`。
2. **合并冲突中的文件**:`is_dirty` 为真,但 `file_statuses` 不给冲突文件状态(没有暂存/工作区改动标志)。bytegit Task 2 `conflicted_files_are_flagged_and_count_as_dirty`;Task 4 `merge_conflict_makes_the_repo_dirty_but_has_no_file_status`。
3. **重命名**:libgit2 的 `entry.path()` 在重命名时返回**旧**路径,而 `git status --porcelain` 与 `dirty_paths` 要新路径。bytegit Task 2 `rename_is_one_entry_with_detection_and_delete_plus_add_without`;Task 5 `dirty_paths_reports_only_the_new_path_of_a_rename`。
4. **detached HEAD 与空仓库**:`branch` 为 `None`;`current_branch_has_commits` 在 detached 时为 **`true`**(迁移前代码如此,注释却写 `false`,保持代码行为、更正注释),空仓库为 `false`。bytegit Task 1;Task 4 `detached_head_still_counts_as_having_commits`、`empty_repo_has_no_commits_no_branch_but_an_empty_branch_list`。
5. **被忽略的文件/目录与非 ASCII 路径**:被忽略的文件不让仓库变 dirty;被忽略的目录只作为一个条目、路径带尾部 `/`;非 ASCII 文件名在 `dirty_paths` 里变成真实 UTF-8(迁移前是 git 转义的八进制,**有意的改进**)。bytegit Task 2;Task 4 `ignored_files_do_not_make_the_repo_dirty_but_untracked_ones_do`;Task 5 `dirty_paths_reports_non_ascii_names_as_real_utf8`。另:bare 仓库上 `status` 返回错误而不是 panic(bytegit Task 2)。

## 待决事项(需要用户裁决,**本计划不改变这些行为**)

- **D1:项目目录在仓库子目录时,git 信息要不要向上找?** 现状(迁移前就如此):`branch/is_dirty/file_statuses` 把子目录项目当"不是仓库",`remote_url/local_branches` 却能找到。这是明显的不一致,monorepo 里的子目录项目会显示"无分支"。迁移只保留现状(`delivery::open_exact`);要统一成向上查找是单独的用户可见改动。
- **D2:`dirty_paths` 在子目录项目里相对仓库根,而 `recent_churn` 用 `--relative`(相对项目根)**,两者口径不同。迁移前就如此,保持原样。

---

## File Structure

**bytegit 仓库**(`~/Projects/CoralProjects/byteboy/bytegit/`):

| 文件 | 变更 |
|------|------|
| `Cargo.toml` | 版本 `0.1.0` → `0.2.0` |
| `src/info.rs` | 新增:`HeadInfo`、`Remote`、`Repo::{head, local_branches, remotes}` |
| `src/status.rs` | 新增:`StatusOptions`、`FileState`、`StatusEntry`、`Repo::{status, is_dirty}` |
| `src/testutil.rs` | 增加夹具方法 `stage`、`delete_file`、`stage_remove`、`detach_head`、`make_conflict` |
| `src/lib.rs` | 声明并再导出新模块 |

**dozer 仓库**(专用 worktree `~/Projects/CoralProjects/byteboy/dozer-bytegit-p1/`,分支 `bytegit-p1`):

| 文件 | 变更 |
|------|------|
| `crates/dozer-app/Cargo.toml` | 加 `bytegit` 依赖(`git2` 保留,`git_log`/`file_history` 等仍在用) |
| `crates/dozer-app/src/delivery.rs` | 查询函数改为 bytegit 适配层;删掉 `git()` 辅助函数;加 `open_exact`;追加刻画测试。`FileGitStatus`/`TreeState`/`rollup_dir_statuses` 等文件树着色逻辑不动 |
| `crates/dozer-app/src/extensions/codehealth/git_hotspots.rs` | `dirty_paths`、`head_short_sha` 改用 bytegit;追加测试 |
| `CLAUDE.md` | 增加 bytegit 关键裁决条目 |
| `docs/superpowers/specs/2026-10-02-bytegit-design.md` 等 | 同步 P1 实测与差异 |

---

### Task 1: bytegit — 夹具扩展与 HEAD / 分支 / 远程

**Files:**
- Modify: `~/Projects/CoralProjects/byteboy/bytegit/src/testutil.rs`、`src/lib.rs`
- Create: `src/info.rs`

**Interfaces:**
- Consumes: P0 的 `Repo::discover`、`Repo::raw()`(`pub(crate)`)、`CommitId::from_oid`、`GitError`、`TempRepo::{new, open, commit_file, branch, checkout, add_remote, write_untracked}`
- Produces(Task 2、Task 4、Task 5 依赖):
  - `HeadInfo { branch: Option<String>, commit: Option<CommitId> }` + `HeadInfo::has_commits(&self) -> bool`
  - `Remote { name: String, url: String, push_url: Option<String> }`
  - `Repo::head(&self) -> Result<HeadInfo, GitError>`、`Repo::local_branches(&self) -> Result<Vec<String>, GitError>`(升序)、`Repo::remotes(&self) -> Result<Vec<Remote>, GitError>`(按名字升序)
  - `TempRepo::{stage(rel), delete_file(rel), stage_remove(rel), detach_head(), make_conflict(rel)}`,均返回 `&Self`

- [ ] **Step 1: 写夹具扩展**

在 `src/testutil.rs` 的 `impl TempRepo` 里,`add_remote` 之前插入:

```rust
    /// 把工作区里的文件加入暂存区(不提交)。
    pub fn stage(&self, rel: &str) -> &Self {
        let mut index = self.repo.index().expect("读取 index 失败");
        index.add_path(Path::new(rel)).expect("暂存失败");
        index.write().expect("写 index 失败");
        self
    }

    /// 从工作区删除文件但不暂存(未暂存的删除)。
    pub fn delete_file(&self, rel: &str) -> &Self {
        std::fs::remove_file(self.path().join(rel)).expect("删除文件失败");
        self
    }

    /// 从暂存区和工作区一起删除(等价于 `git rm`)。
    pub fn stage_remove(&self, rel: &str) -> &Self {
        self.delete_file(rel);
        let mut index = self.repo.index().expect("读取 index 失败");
        index.remove_path(Path::new(rel)).expect("移除失败");
        index.write().expect("写 index 失败");
        self
    }

    /// 让 HEAD 脱离分支,指向当前提交。
    pub fn detach_head(&self) -> &Self {
        let id = self
            .repo
            .head()
            .expect("detach 前需要至少一次提交")
            .peel_to_commit()
            .expect("HEAD 不是提交")
            .id();
        self.repo.set_head_detached(id).expect("detach 失败");
        self
    }

    /// 制造一个合并冲突:两个分支对同一文件做不同修改,再在 `main` 上合并 `other`,
    /// 合并停在冲突状态(冲突文件留在 index 里)。结束时位于 `main`。
    pub fn make_conflict(&self, rel: &str) -> &Self {
        self.commit_file(rel, "base\n", "base");
        self.branch("other").checkout("other");
        self.commit_file(rel, "other\n", "other change");
        self.checkout("main");
        self.commit_file(rel, "main\n", "main change");
        let other = self
            .repo
            .find_branch("other", git2::BranchType::Local)
            .expect("找不到 other 分支")
            .get()
            .peel_to_commit()
            .expect("other 不是提交")
            .id();
        let annotated = self
            .repo
            .find_annotated_commit(other)
            .expect("构造 annotated commit 失败");
        self.repo
            .merge(&[&annotated], None, None)
            .expect("合并失败");
        self
    }
```

- [ ] **Step 2: 写 `src/info.rs` 的测试与桩(RED)**

创建 `src/info.rs`:内容 = 下面 Step 4 完整文件里的**全部类型定义**与 `impl Repo` 块,但三个方法体都换成 `unimplemented!()`(签名不变),再加上完整的 `#[cfg(test)] mod tests`。`use` 行保持不变(桩阶段可能有未使用 import 警告,忽略)。

`src/lib.rs` 的最终内容如下。**Task 1 先不要加 `mod status;` 与 `pub use status::...` 两行**(`status.rs` 在 Task 2 才创建),它们在 Task 2 Step 1 补上:

```rust
//! bytegit:ByteBoy 系产品共用的本地 Git 底层。
//!
//! 全部同步;公开 API 不暴露 `git2` 类型。设计见 dozer 仓库
//! `docs/superpowers/specs/2026-10-02-bytegit-design.md`。

mod change;
mod error;
mod id;
mod info;
mod repo;
mod status;
#[cfg(any(test, feature = "testutil"))]
pub mod testutil;

pub use change::ChangeKind;
pub use error::{GitError, GitErrorKind};
pub use id::{BlobId, CommitId};
pub use info::{HeadInfo, Remote};
pub use repo::Repo;
pub use status::{FileState, StatusEntry, StatusOptions};
```

- [ ] **Step 3: 运行测试,确认 RED**

Run: `cd ~/Projects/CoralProjects/byteboy/bytegit && cargo test info::`
Expected: 编译通过,6 个 `info::tests::*` 全部 **FAILED**(`not implemented`)。

- [ ] **Step 4: 写真实实现(GREEN)**

`src/info.rs` 完整内容:

```rust
//! HEAD、分支与远程信息。

use crate::{CommitId, GitError, Repo};

/// HEAD 的状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadInfo {
    /// 当前分支短名。detached HEAD、HEAD 指向非分支引用、HEAD 还没有诞生(空仓库)、
    /// 或分支名不是合法 UTF-8 时为 `None`。
    pub branch: Option<String>,
    /// HEAD 解析到的提交;空仓库(HEAD 未诞生)为 `None`。detached HEAD 时有值。
    pub commit: Option<CommitId>,
}

impl HeadInfo {
    /// HEAD 能解析到一个提交。detached HEAD 也算有提交。
    pub fn has_commits(&self) -> bool {
        self.commit.is_some()
    }
}

/// 一个远程。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    /// fetch URL(已应用 `url.<base>.insteadOf` 改写)。
    pub url: String,
    /// 单独配置的 push URL;没有则为 `None`。
    pub push_url: Option<String>,
}

impl Repo {
    pub fn head(&self) -> Result<HeadInfo, GitError> {
        match self.raw().head() {
            Ok(head) => {
                let branch = if head.is_branch() {
                    head.shorthand().ok().map(str::to_string)
                } else {
                    None
                };
                let commit = head
                    .peel_to_commit()
                    .ok()
                    .map(|c| CommitId::from_oid(c.id()));
                Ok(HeadInfo { branch, commit })
            }
            Err(e) if e.code() == git2::ErrorCode::UnbornBranch => Ok(HeadInfo {
                branch: None,
                commit: None,
            }),
            Err(e) => Err(GitError::from(e)),
        }
    }

    /// 全部本地分支短名,按名字升序(与 `git for-each-ref refs/heads/` 的顺序一致)。
    /// 空仓库(没有任何分支引用)返回空列表;名字不是合法 UTF-8 的分支被跳过。
    pub fn local_branches(&self) -> Result<Vec<String>, GitError> {
        let mut names = Vec::new();
        for item in self.raw().branches(Some(git2::BranchType::Local))? {
            let (branch, _) = item?;
            // 0.21:`name()` 是 `Result<Option<&str>>`,名字不是 UTF-8 时为 `None`。
            if let Ok(Some(name)) = branch.name() {
                names.push(name.to_string());
            }
        }
        names.sort();
        Ok(names)
    }

    /// 全部远程,按名字升序。没有 URL 的远程被跳过。
    pub fn remotes(&self) -> Result<Vec<Remote>, GitError> {
        let raw = self.raw();
        let mut out = Vec::new();
        for name in raw.remotes()?.iter().flatten().flatten() {
            let remote = raw.find_remote(name)?;
            let Ok(url) = remote.url() else { continue };
            out.push(Remote {
                name: name.to_string(),
                url: url.to_string(),
                push_url: remote.pushurl().ok().flatten().map(str::to_string),
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use crate::testutil::TempRepo;

    #[test]
    fn head_on_a_branch_reports_name_and_commit() {
        let t = TempRepo::new();
        let id = t.commit_file("a.txt", "x", "one");
        let head = t.open().head().unwrap();
        assert_eq!(head.branch.as_deref(), Some("main"));
        assert_eq!(head.commit, Some(id));
        assert!(head.has_commits());
    }

    #[test]
    fn head_of_an_empty_repo_has_no_branch_and_no_commit() {
        let t = TempRepo::new();
        let head = t.open().head().unwrap();
        assert_eq!(head.branch, None);
        assert_eq!(head.commit, None);
        assert!(!head.has_commits());
    }

    #[test]
    fn detached_head_has_no_branch_but_still_has_a_commit() {
        let t = TempRepo::new();
        let id = t.commit_file("a.txt", "x", "one");
        t.detach_head();
        let head = t.open().head().unwrap();
        assert_eq!(head.branch, None);
        assert_eq!(head.commit, Some(id));
        assert!(head.has_commits());
    }

    #[test]
    fn local_branches_are_sorted_and_empty_repo_has_none() {
        let t = TempRepo::new();
        assert!(t.open().local_branches().unwrap().is_empty());
        t.commit_file("a.txt", "x", "one");
        t.branch("zeta").branch("alpha").branch("feature/x");
        assert_eq!(
            t.open().local_branches().unwrap(),
            vec!["alpha", "feature/x", "main", "zeta"]
        );
    }

    #[test]
    fn remotes_are_sorted_by_name_with_urls() {
        let t = TempRepo::new();
        t.add_remote("upstream", "https://example.com/y.git");
        t.add_remote("origin", "https://example.com/x.git");
        let remotes = t.open().remotes().unwrap();
        let got: Vec<(&str, &str)> = remotes
            .iter()
            .map(|r| (r.name.as_str(), r.url.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("origin", "https://example.com/x.git"),
                ("upstream", "https://example.com/y.git")
            ]
        );
        assert!(remotes.iter().all(|r| r.push_url.is_none()));
    }

    #[test]
    fn no_remotes_is_an_empty_list() {
        let t = TempRepo::new();
        assert!(t.open().remotes().unwrap().is_empty());
    }
}
```

- [ ] **Step 5: 运行测试**

Run: `cargo fmt && cargo test info:: && cargo test`
Expected: 6 个 `info::tests::*` 通过;全量测试通过(P0 的 31 + 6 = 37,以实际为准,全部 `ok`)。

- [ ] **Step 6: 门禁**

Run: `cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --check`
Expected: 无输出、退出码 0。

- [ ] **Step 7: Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/bytegit
git add src/info.rs src/lib.rs src/testutil.rs
git status --short
git commit -m "feat: add HeadInfo, local_branches and remotes; extend TempRepo fixture

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: bytegit — 工作区状态

**Files:**
- Create: `src/status.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: Task 1 的夹具方法;P0 的 `ChangeKind`、`Repo::raw()`
- Produces(Task 4、Task 5 依赖):
  - `StatusOptions { include_untracked: bool, include_ignored: bool, detect_renames: bool }`,`Default` = `{ true, false, false }`(`Copy`)
  - `FileState { index: Option<ChangeKind>, worktree: Option<ChangeKind>, ignored: bool, conflicted: bool }`(`Copy`)
  - `StatusEntry { path: PathBuf /* 相对仓库根;被忽略的目录带尾部 `/`;重命名为新路径 */, state: FileState }`
  - `Repo::status(&self, StatusOptions) -> Result<Vec<StatusEntry>, GitError>`、`Repo::is_dirty(&self, StatusOptions) -> Result<bool, GitError>`

- [ ] **Step 1: 写 `src/status.rs` 的测试与桩(RED)**

创建 `src/status.rs`:内容 = 下面 Step 4 完整文件里的类型定义、`Default` 实现与 `impl Repo` 块,但 `status`、`is_dirty` 的方法体换成 `unimplemented!()`,**不要包含** `index_kind`/`worktree_kind`/`entry_path`/`git2_options` 四个辅助函数(桩阶段用不到),再加上完整的 `#[cfg(test)] mod tests`。在 `src/lib.rs` 加上 `mod status;` 与 `pub use status::{FileState, StatusEntry, StatusOptions};`(其位置见上面 Task 1 Step 2 的完整 `lib.rs`)。

- [ ] **Step 2: 运行测试,确认 RED**

Run: `cargo test status::`
Expected: 12 个 `status::tests::*` 全部 **FAILED**(`not implemented`;连 `status_on_a_bare_repo_is_an_error_not_a_panic` 也因桩 panic 而失败)。

- [ ] **Step 3: 写真实实现(GREEN)**

`src/status.rs` 完整内容:

```rust
//! 工作区状态。

use std::path::PathBuf;

use crate::{ChangeKind, GitError, Repo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusOptions {
    /// 包含未跟踪文件,且递归进未跟踪目录、逐个列出其中的文件。
    pub include_untracked: bool,
    /// 包含被 `.gitignore` 忽略的条目(被忽略的目录只作为一个条目出现,路径带尾部 `/`)。
    pub include_ignored: bool,
    /// 做重命名检测(HEAD→暂存区、暂存区→工作区):重命名只报新路径,
    /// 否则表现为"旧路径删除 + 新路径新增"。与 `git status --porcelain` 的默认行为一致。
    pub detect_renames: bool,
}

impl Default for StatusOptions {
    /// 含未跟踪、不含被忽略、不做重命名检测。
    fn default() -> Self {
        Self {
            include_untracked: true,
            include_ignored: false,
            detect_renames: false,
        }
    }
}

/// 一个路径的状态。`index` 是相对 HEAD 的暂存改动,`worktree` 是相对暂存区的工作区改动,
/// 两边可同时有值(部分暂存后又改)。未跟踪的新文件表现为 `worktree == Some(Added)` 且 `index == None`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileState {
    pub index: Option<ChangeKind>,
    pub worktree: Option<ChangeKind>,
    pub ignored: bool,
    /// 合并冲突中。冲突文件的 `index`/`worktree` 可能都是 `None`。
    pub conflicted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    /// 相对仓库根。
    pub path: PathBuf,
    pub state: FileState,
}

fn index_kind(s: git2::Status) -> Option<ChangeKind> {
    use git2::Status as S;
    if s.contains(S::INDEX_NEW) {
        Some(ChangeKind::Added)
    } else if s.contains(S::INDEX_DELETED) {
        Some(ChangeKind::Deleted)
    } else if s.contains(S::INDEX_RENAMED) {
        Some(ChangeKind::Renamed)
    } else if s.contains(S::INDEX_TYPECHANGE) {
        Some(ChangeKind::TypeChange)
    } else if s.contains(S::INDEX_MODIFIED) {
        Some(ChangeKind::Modified)
    } else {
        None
    }
}

fn worktree_kind(s: git2::Status) -> Option<ChangeKind> {
    use git2::Status as S;
    if s.contains(S::WT_NEW) {
        Some(ChangeKind::Added)
    } else if s.contains(S::WT_DELETED) {
        Some(ChangeKind::Deleted)
    } else if s.contains(S::WT_RENAMED) {
        Some(ChangeKind::Renamed)
    } else if s.contains(S::WT_TYPECHANGE) {
        Some(ChangeKind::TypeChange)
    } else if s.contains(S::WT_MODIFIED) {
        Some(ChangeKind::Modified)
    } else {
        None
    }
}

/// 条目的路径。重命名时 libgit2 的 `entry.path()` 给的是**旧**路径,而 `git status --porcelain`
/// 与调用方想要的是新路径,所以重命名要从对应的 diff delta 里取 `new_file`。
/// 路径不是合法 UTF-8 的条目返回 `None`(被跳过)。
fn entry_path(entry: &git2::StatusEntry<'_>, s: git2::Status) -> Option<PathBuf> {
    let renamed = if s.contains(git2::Status::INDEX_RENAMED) {
        entry.head_to_index()
    } else if s.contains(git2::Status::WT_RENAMED) {
        entry.index_to_workdir()
    } else {
        None
    };
    let rel = match renamed.and_then(|d| d.new_file().path().map(|p| p.to_path_buf())) {
        Some(new_path) => new_path.to_str().map(str::to_string)?,
        None => entry.path().ok()?.to_string(),
    };
    (!rel.is_empty()).then(|| PathBuf::from(rel))
}

fn git2_options(opts: StatusOptions) -> git2::StatusOptions {
    let mut o = git2::StatusOptions::new();
    o.include_untracked(opts.include_untracked)
        .recurse_untracked_dirs(opts.include_untracked)
        .include_ignored(opts.include_ignored);
    if opts.detect_renames {
        o.renames_head_to_index(true).renames_index_to_workdir(true);
    }
    o
}

impl Repo {
    /// 有改动的路径及其状态。路径不是合法 UTF-8 的条目被跳过。bare 仓库返回错误。
    pub fn status(&self, opts: StatusOptions) -> Result<Vec<StatusEntry>, GitError> {
        let mut o = git2_options(opts);
        let statuses = self.raw().statuses(Some(&mut o))?;
        let mut out = Vec::new();
        for entry in statuses.iter() {
            let s = entry.status();
            if s.is_empty() {
                continue;
            }
            let Some(path) = entry_path(&entry, s) else {
                continue;
            };
            out.push(StatusEntry {
                path,
                state: FileState {
                    index: index_kind(s),
                    worktree: worktree_kind(s),
                    ignored: s.contains(git2::Status::IGNORED),
                    conflicted: s.contains(git2::Status::CONFLICTED),
                },
            });
        }
        Ok(out)
    }

    /// 是否有任何改动。与 `status(opts)` 是否非空一致,但不会因为路径不是 UTF-8 而漏报。
    pub fn is_dirty(&self, opts: StatusOptions) -> Result<bool, GitError> {
        let mut o = git2_options(opts);
        Ok(!self.raw().statuses(Some(&mut o))?.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempRepo;
    use std::path::Path;

    fn entry<'a>(v: &'a [StatusEntry], p: &str) -> &'a StatusEntry {
        v.iter()
            .find(|e| e.path == Path::new(p))
            .unwrap_or_else(|| panic!("没有 {p} 的状态: {v:?}"))
    }

    fn all() -> StatusOptions {
        StatusOptions {
            include_untracked: true,
            include_ignored: true,
            detect_renames: false,
        }
    }

    #[test]
    fn clean_repo_has_no_entries_and_is_not_dirty() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "one\n", "c1");
        let repo = t.open();
        assert!(repo.status(StatusOptions::default()).unwrap().is_empty());
        assert!(!repo.is_dirty(StatusOptions::default()).unwrap());
    }

    #[test]
    fn modified_staged_and_untracked_are_distinguished() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "one\n", "c1");
        t.commit_file("b.txt", "one\n", "c2");
        // 只暂存
        t.write_untracked("a.txt", "staged\n").stage("a.txt");
        // 未暂存修改
        t.write_untracked("b.txt", "worktree\n");
        // 未跟踪
        t.write_untracked("new.txt", "n\n");
        let v = t.open().status(StatusOptions::default()).unwrap();
        let a = entry(&v, "a.txt").state;
        assert_eq!((a.index, a.worktree), (Some(ChangeKind::Modified), None));
        let b = entry(&v, "b.txt").state;
        assert_eq!((b.index, b.worktree), (None, Some(ChangeKind::Modified)));
        let n = entry(&v, "new.txt").state;
        assert_eq!((n.index, n.worktree), (None, Some(ChangeKind::Added)));
    }

    #[test]
    fn staged_then_modified_again_has_both_sides() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "one\n", "c1");
        t.write_untracked("a.txt", "staged\n").stage("a.txt");
        t.write_untracked("a.txt", "staged\nplus more\n");
        let v = t.open().status(StatusOptions::default()).unwrap();
        let a = entry(&v, "a.txt").state;
        assert_eq!(a.index, Some(ChangeKind::Modified));
        assert_eq!(a.worktree, Some(ChangeKind::Modified));
    }

    #[test]
    fn staged_new_file_then_edited_is_added_in_index_modified_in_worktree() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "one\n", "c1");
        t.write_untracked("n.txt", "1\n").stage("n.txt");
        t.write_untracked("n.txt", "2\n");
        let v = t.open().status(StatusOptions::default()).unwrap();
        let n = entry(&v, "n.txt").state;
        assert_eq!(n.index, Some(ChangeKind::Added));
        assert_eq!(n.worktree, Some(ChangeKind::Modified));
    }

    #[test]
    fn deleted_files_staged_and_unstaged() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "one\n", "c1");
        t.commit_file("b.txt", "one\n", "c2");
        t.stage_remove("a.txt");
        t.delete_file("b.txt");
        let v = t.open().status(StatusOptions::default()).unwrap();
        assert_eq!(entry(&v, "a.txt").state.index, Some(ChangeKind::Deleted));
        assert_eq!(entry(&v, "a.txt").state.worktree, None);
        assert_eq!(entry(&v, "b.txt").state.index, None);
        assert_eq!(entry(&v, "b.txt").state.worktree, Some(ChangeKind::Deleted));
    }

    #[test]
    fn untracked_files_in_new_directories_are_listed_individually() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "one\n", "c1");
        t.write_untracked("dir/sub/x.txt", "x")
            .write_untracked("dir/y.txt", "y");
        let v = t.open().status(StatusOptions::default()).unwrap();
        entry(&v, "dir/sub/x.txt");
        entry(&v, "dir/y.txt");
    }

    #[test]
    fn untracked_can_be_excluded() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "one\n", "c1");
        t.write_untracked("new.txt", "n");
        let opts = StatusOptions {
            include_untracked: false,
            ..StatusOptions::default()
        };
        let repo = t.open();
        assert!(repo.status(opts).unwrap().is_empty());
        assert!(!repo.is_dirty(opts).unwrap());
        assert!(repo.is_dirty(StatusOptions::default()).unwrap());
    }

    #[test]
    fn ignored_entries_only_appear_when_requested_and_never_make_the_repo_dirty() {
        let t = TempRepo::new();
        t.commit_file(".gitignore", "*.log\ntarget/\n", "ignore");
        t.write_untracked("debug.log", "x")
            .write_untracked("target/out.bin", "x");
        let repo = t.open();
        assert!(repo.status(StatusOptions::default()).unwrap().is_empty());
        assert!(!repo.is_dirty(StatusOptions::default()).unwrap());
        let v = repo.status(all()).unwrap();
        assert!(entry(&v, "debug.log").state.ignored);
        // 被忽略的目录只作为一个条目出现,路径带尾部 `/`
        assert!(entry(&v, "target/").state.ignored);
        assert_eq!(entry(&v, "target/").state.index, None);
    }

    #[test]
    fn rename_is_one_entry_with_detection_and_delete_plus_add_without() {
        let t = TempRepo::new();
        let body = "line one\nline two\nline three\nline four\nline five\n";
        t.commit_file("old.txt", body, "c1");
        t.stage_remove("old.txt");
        t.write_untracked("new.txt", body).stage("new.txt");
        let repo = t.open();

        let without = repo.status(StatusOptions::default()).unwrap();
        assert_eq!(
            entry(&without, "old.txt").state.index,
            Some(ChangeKind::Deleted)
        );
        assert_eq!(
            entry(&without, "new.txt").state.index,
            Some(ChangeKind::Added)
        );

        let with = repo
            .status(StatusOptions {
                detect_renames: true,
                ..StatusOptions::default()
            })
            .unwrap();
        assert_eq!(with.len(), 1, "{with:?}");
        assert_eq!(
            entry(&with, "new.txt").state.index,
            Some(ChangeKind::Renamed)
        );
    }

    #[test]
    fn conflicted_files_are_flagged_and_count_as_dirty() {
        let t = TempRepo::new();
        t.make_conflict("c.txt");
        let repo = t.open();
        let v = repo.status(StatusOptions::default()).unwrap();
        let c = entry(&v, "c.txt").state;
        assert!(c.conflicted);
        assert_eq!((c.index, c.worktree), (None, None));
        assert!(repo.is_dirty(StatusOptions::default()).unwrap());
    }

    #[test]
    fn non_ascii_paths_are_reported_as_real_utf8() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "one\n", "c1");
        t.write_untracked("文档/说明.md", "x");
        let v = t.open().status(StatusOptions::default()).unwrap();
        entry(&v, "文档/说明.md");
    }

    #[test]
    fn status_on_a_bare_repo_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init_bare(dir.path()).unwrap();
        let repo = crate::Repo::discover(dir.path()).unwrap();
        assert!(repo.status(StatusOptions::default()).is_err());
        assert!(repo.is_dirty(StatusOptions::default()).is_err());
    }
}
```

- [ ] **Step 4: 运行测试**

Run: `cargo fmt && cargo test status:: && cargo test && cargo test --all-features`
Expected: 12 个 `status::tests::*` 通过;全量 49 个通过(默认与 `--all-features` 各一次)。

- [ ] **Step 5: 变异检验(确认 Review Focus 的测试真的会失败)**

1. 把 `entry_path` 里 `renamed.and_then(...)` 那一支临时改成总是 `None`(即总用 `entry.path()`),运行 `cargo test rename_is_one_entry`。Expected: **FAILED**(重命名返回了旧路径 `old.txt`)。还原。
2. 把 `git2_options` 里 `.include_ignored(opts.include_ignored)` 临时改成 `.include_ignored(true)`,运行 `cargo test ignored_entries_only_appear`。Expected: **FAILED**。还原。

还原后 `cargo test` 回到 49 个通过。

- [ ] **Step 6: 门禁与 Commit**

Run: `cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --check`
Expected: 无输出。

```bash
git add src/status.rs src/lib.rs
git status --short
git commit -m "feat: add Repo::status and is_dirty with rename detection and conflict flag

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: bytegit — 发布 v0.2.0

**Files:**
- Modify: `Cargo.toml`(`version = "0.2.0"`)

**Interfaces:** Produces:git tag `v0.2.0`(Task 4 的 dozer 依赖)。

- [ ] **Step 1: 升版本并全量门禁**

把 `Cargo.toml` 的 `version = "0.1.0"` 改为 `"0.2.0"`。

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings && cargo test && cargo test --all-features`
Expected: 全部通过,49 个测试。

- [ ] **Step 2: Commit 并推送 main(对外可见操作:先向用户确认)**

向用户确认"现在推送 `main` 并打 tag `v0.2.0`"。获得明确同意后:

```bash
cd ~/Projects/CoralProjects/byteboy/bytegit
git add Cargo.toml
git commit -m "chore: release 0.2.0

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git push origin main
gh run watch "$(gh run list --repo byteboyai/bytegit --limit 1 --json databaseId -q '.[0].databaseId')" --repo byteboyai/bytegit --exit-status
```

Expected: CI(macOS)通过。**CI 红则先修,不打 tag。**

- [ ] **Step 3: 打 tag 并验证可被消费**

```bash
git tag -a v0.2.0 -m "bytegit v0.2.0: head, branches, remotes, status"
git push origin v0.2.0
cd "$(mktemp -d)" && cargo new --lib consume -q && cd consume
cargo add bytegit --git https://github.com/byteboyai/bytegit --tag v0.2.0 --features testutil
cat >> src/lib.rs <<'EOF'

#[cfg(test)]
mod t {
    #[test]
    fn consumes_bytegit() {
        let t = bytegit::testutil::TempRepo::new();
        t.commit_file("a.txt", "x", "one");
        let repo = t.open();
        assert_eq!(repo.head().unwrap().branch.as_deref(), Some("main"));
        assert!(!repo.is_dirty(bytegit::StatusOptions::default()).unwrap());
    }
}
EOF
cargo test
```

Expected: `consumes_bytegit ... ok`。

---

### Task 4: dozer — `delivery.rs` 查询函数迁移

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`、`crates/dozer-app/src/delivery.rs`(均在 worktree 内)

**Interfaces:**
- Consumes: bytegit `v0.2.0` 的 `Repo`、`StatusOptions`、`HeadInfo`、`Remote`、`ChangeKind`
- Produces(Task 5 依赖):`pub(crate) fn open_exact(path: &Path) -> Option<bytegit::Repo>`;其余 `delivery::{repo_root, is_dirty, file_statuses, branch, remote_url, local_branches, current_branch_has_commits}` 签名**完全不变**

- [ ] **Step 1: 建专用 worktree 与分支**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-bytegit-p1 -b bytegit-p1 main
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p1 && git log --oneline | head -1 && git status --short
```

Expected: 干净、分支 `bytegit-p1`。此后**所有命令都用 `~/Projects/CoralProjects/byteboy/dozer-bytegit-p1/` 前缀**。

- [ ] **Step 2: 加依赖**

在 `crates/dozer-app/Cargo.toml` 的 `git2 = "0.21"` 下一行加:

```toml
bytegit = { git = "https://github.com/byteboyai/bytegit", tag = "v0.2.0" }
```

Run: `cargo build -p dozer-app`
Expected: 编译通过(首次拉取依赖,约 2 分钟)。

- [ ] **Step 3: 在旧实现上写刻画测试**

把下面整段追加到 `delivery.rs` 末尾 `mod tests` 的**最后一个测试之后、闭合的 `}` 之前**:

```rust
    // ---- bytegit P1:刻画迁移前后必须一致的口径 ----

    fn git_in(repo: &std::path::Path, args: &[&str]) {
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

    #[test]
    fn project_in_a_repo_subdirectory_is_treated_as_not_a_repo_by_open_based_queries() {
        // 迁移前 `branch/is_dirty/file_statuses/current_branch_has_commits` 用 `Repository::open`,
        // 只认仓库根;`repo_root/local_branches/remote_url` 用命令行,会向上找。
        let (_d, repo) = mkrepo();
        git_in(
            &repo,
            &["remote", "add", "origin", "https://example.com/x.git"],
        );
        let sub = repo.join("pkg");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("f.txt"), "x").unwrap();

        assert_eq!(branch(&sub), None);
        assert!(!is_dirty(&sub));
        assert!(file_statuses(&sub).is_empty());
        assert!(!current_branch_has_commits(&sub));

        assert!(repo_root(&sub).is_some());
        assert_eq!(
            remote_url(&sub),
            vec!["https://example.com/x.git".to_string()]
        );
        assert!(local_branches(&sub).is_some_and(|b| !b.is_empty()));
    }

    #[test]
    fn detached_head_still_counts_as_having_commits() {
        let (_d, repo) = mkrepo();
        git_in(&repo, &["checkout", "-q", "--detach"]);
        assert!(current_branch_has_commits(&repo));
        assert_eq!(branch(&repo), None);
    }

    #[test]
    fn empty_repo_has_no_commits_no_branch_but_an_empty_branch_list() {
        let dir = tempfile::tempdir().unwrap();
        git_in(dir.path(), &["init", "-q"]);
        assert!(!current_branch_has_commits(dir.path()));
        assert_eq!(branch(dir.path()), None);
        assert_eq!(local_branches(dir.path()), Some(Vec::new()));
        assert!(!is_dirty(dir.path()));
    }

    #[test]
    fn non_git_dir_answers_the_empty_way() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(branch(dir.path()), None);
        assert!(!is_dirty(dir.path()));
        assert!(file_statuses(dir.path()).is_empty());
        assert_eq!(local_branches(dir.path()), None);
        assert!(remote_url(dir.path()).is_empty());
        assert!(repo_root(dir.path()).is_none());
    }

    #[test]
    fn remote_urls_are_deduped_and_ordered_by_remote_name() {
        let dir = tempfile::tempdir().unwrap();
        git_in(dir.path(), &["init", "-q"]);
        git_in(
            dir.path(),
            &["remote", "add", "upstream", "https://example.com/u.git"],
        );
        git_in(
            dir.path(),
            &["remote", "add", "origin", "https://example.com/o.git"],
        );
        git_in(
            dir.path(),
            &["remote", "add", "zed", "https://example.com/o.git"],
        );
        assert_eq!(
            remote_url(dir.path()),
            vec![
                "https://example.com/o.git".to_string(),
                "https://example.com/u.git".to_string()
            ]
        );
    }

    #[test]
    fn local_branches_are_listed_in_name_order() {
        let (_d, repo) = mkrepo();
        git_in(&repo, &["branch", "zeta"]);
        git_in(&repo, &["branch", "alpha"]);
        git_in(&repo, &["branch", "feature/x"]);
        let mut expected = vec![
            "alpha".to_string(),
            "feature/x".to_string(),
            "zeta".to_string(),
        ];
        let got = local_branches(&repo).unwrap();
        // 初始分支名取决于用户的 init.defaultBranch,不假设它是什么
        let current = branch(&repo).unwrap();
        expected.push(current);
        expected.sort();
        assert_eq!(got, expected);
    }

    #[test]
    fn ignored_files_do_not_make_the_repo_dirty_but_untracked_ones_do() {
        let (_d, repo) = mkrepo();
        std::fs::write(repo.join(".gitignore"), "*.log\n").unwrap();
        git_in(&repo, &["add", "."]);
        git_in(&repo, &["commit", "-qm", "ignore"]);
        std::fs::write(repo.join("debug.log"), "x").unwrap();
        assert!(!is_dirty(&repo));
        assert!(file_statuses(&repo)[&repo.join("debug.log")].ignored);
        std::fs::write(repo.join("new.txt"), "x").unwrap();
        assert!(is_dirty(&repo));
    }

    #[test]
    fn merge_conflict_makes_the_repo_dirty_but_has_no_file_status() {
        let (_d, repo) = mkrepo();
        let base = branch(&repo).unwrap();
        git_in(&repo, &["checkout", "-q", "-b", "other"]);
        std::fs::write(repo.join("a.txt"), "other\n").unwrap();
        git_in(&repo, &["commit", "-qam", "other"]);
        git_in(&repo, &["checkout", "-q", &base]);
        std::fs::write(repo.join("a.txt"), "main\n").unwrap();
        git_in(&repo, &["commit", "-qam", "main"]);
        // 合并冲突时 git 以非零退出,这里不能用 git_in 的成功断言
        let _ = Command::new("git")
            .args(["merge", "other"])
            .current_dir(&repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(is_dirty(&repo));
        assert!(
            !file_statuses(&repo).contains_key(&repo.join("a.txt")),
            "冲突文件没有暂存/工作区改动标志,旧实现不给它状态"
        );
    }
```

- [ ] **Step 4: 在旧实现上运行,确认全部通过**

Run: `cargo test -p dozer-app -- delivery::`
Expected: `delivery::` 共 29 个测试**全部通过**(原有 21 个 + 新增 8 个)。这是**刻画测试**,描述旧行为,所以在旧实现上就该通过;若有失败,说明测试描述错了旧行为,**改测试,不改旧代码**。

- [ ] **Step 5: 换实现**

在 `delivery.rs` 里做三处替换(其余内容——`FileGitStatus`、`TreeState`、`rollup_dir_statuses`、`checkout_branch`、`init_repo`、`git_available`、`clone_repo`、`mod tests` 里原有测试——全部不动):

**(a)** 把文件开头到 `/// 文件树装饰用的 git 改动类型` 之前的整段(含 `use anyhow::Result;`、`use std::...`、`fn git(...)`、`pub fn repo_root`、`pub fn is_dirty`)替换为:

```rust
use anyhow::Result;
use bytegit::{ChangeKind as GitChange, Repo, StatusOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 两个目录是同一个目录(规范化后相等);任一规范化失败按不同处理。
fn same_dir(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// 只认仓库根:`path` 必须正好是某个仓库的工作区根才打开。保持迁移前 `git2::Repository::open`
/// 的语义——项目路径在仓库子目录时视为"不是仓库"。要不要改成向上查找是单独的产品决策
/// (见 bytegit P1 计划"待决事项"),不在迁移里顺手改变。
pub(crate) fn open_exact(path: &Path) -> Option<Repo> {
    let repo = Repo::discover(path).ok()?;
    same_dir(repo.root(), path).then_some(repo)
}

/// 向上查找所属仓库(迁移前用命令行 `git`,在子目录里同样会向上找)。
fn open_upward(path: &Path) -> Option<Repo> {
    Repo::discover(path).ok()
}

/// 取 `dir` 所属 git 仓库的工作区根;非 git、bare 仓库、`dir` 不是目录时返回 `None`。
pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let repo = open_upward(dir)?;
    (!repo.is_bare()).then(|| repo.root().to_path_buf())
}

/// 工作区是否有任何改动(暂存或未暂存,含未跟踪,不含被 `.gitignore` 排除的文件)。
pub fn is_dirty(repo: &Path) -> bool {
    open_exact(repo)
        .and_then(|r| r.is_dirty(StatusOptions::default()).ok())
        .unwrap_or(false)
}
```

**(b)** 把 `file_statuses` 的整个函数(从它的文档注释 `/// 用 git2::Repository::statuses 取代 ...` 到函数结束)替换为:

```rust
/// 取整个仓库的文件级状态(含未跟踪与被忽略)。`repo` 必须是仓库根(见 [`open_exact`]);
/// 非 git / 打开失败返回空。键是 `repo.join(相对路径)`。
pub fn file_statuses(repo: &Path) -> HashMap<PathBuf, FileGitStatus> {
    let mut map = HashMap::new();
    let Some(git_repo) = open_exact(repo) else {
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
```

**(c)** 把从 `/// 当前分支名;非 git` 到 `checkout_branch` 之前的四个函数(`branch`、`remote_url`、`local_branches`、`current_branch_has_commits`)整体替换为:

```rust
/// 当前分支名;非 git / 无提交 / detached HEAD 返回 None。
pub fn branch(repo: &Path) -> Option<String> {
    open_exact(repo)?.head().ok()?.branch
}

/// 返回仓库**全部** remote 的 fetch URL(去重,按 remote 名字升序,与 `git remote -v`
/// 的顺序一致);没有 remote / 非 git 目录 → 空 `Vec`(语义上即"未设置")。
pub fn remote_url(repo: &Path) -> Vec<String> {
    let Some(git_repo) = open_upward(repo) else {
        return Vec::new();
    };
    let Ok(remotes) = git_repo.remotes() else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut urls = Vec::new();
    for remote in remotes {
        if seen.insert(remote.url.clone()) {
            urls.push(remote.url);
        }
    }
    urls
}

/// 所有本地分支名(短名,升序)。非 git 仓库返回 None;
/// git 仓库但没有分支(空仓未提交)返回 Some(空 vec)。
pub fn local_branches(repo: &Path) -> Option<Vec<String>> {
    open_upward(repo)?.local_branches().ok()
}

/// HEAD 是否能解析到提交。空仓(刚 `git init` 未 commit 的 unborn 分支)返回 `false`;
/// 已有一条或更多提交返回 `true`;非 git 返回 `false`。
/// **detached HEAD 返回 `true`**(HEAD 仍指向一个提交)——迁移前的注释写的是 `false`,
/// 但代码一直是 `true`,这里保持代码行为、更正注释。
pub fn current_branch_has_commits(repo: &Path) -> bool {
    open_exact(repo)
        .and_then(|r| r.head().ok())
        .is_some_and(|h| h.has_commits())
}
```

- [ ] **Step 6: 运行,确认仍然全部通过**

Run: `cargo test -p dozer-app -- delivery:: && cargo test -p dozer-app 2>&1 | grep -E "test result|FAILED"`
Expected: `delivery::` 29 个通过;全量只有"已知基线"里的失败(`delete_confirm_spec_reflects_pending_target`,以及在 worktree 里的 `build_marks_head_branch_and_labels`),**没有其他失败**。

- [ ] **Step 7: 变异检验**

把 `branch()` 里的 `open_exact(repo)?` 临时改成 `open_upward(repo)?`,运行 `cargo test -p dozer-app -- project_in_a_repo_subdirectory`。Expected: **FAILED**(子目录里 `branch` 不再是 `None`)。还原。

- [ ] **Step 8: 门禁与 Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p1
rustfmt --edition 2024 crates/dozer-app/src/delivery.rs
cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "delivery.rs" ; echo "(上面应为空:delivery.rs 不产生任何诊断)"
grep -n "git2::" crates/dozer-app/src/delivery.rs ; echo "(上面应为空:delivery.rs 不再直接用 git2)"
git status --short
git add crates/dozer-app/Cargo.toml crates/dozer-app/src/delivery.rs Cargo.lock
git diff --cached --stat
git commit -m "refactor(delivery): back git queries with bytegit, keep signatures

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

Expected: 两条检查都为空;提交只含 3 个文件。

---

### Task 5: dozer — `git_hotspots` 的 `dirty_paths` 与 `head_short_sha`

**Files:**
- Modify: `crates/dozer-app/src/extensions/codehealth/git_hotspots.rs`

**Interfaces:**
- Consumes: Task 4 的 `delivery::open_exact`;bytegit `Repo::discover`、`Repo::status`、`HeadInfo::commit`、`CommitId::short`
- Produces: `dirty_paths(&Path) -> Vec<String>`、`head_short_sha(&Path) -> Option<String>` 签名不变

- [ ] **Step 1: 在旧实现上写刻画测试**

把下面整段追加到 `git_hotspots.rs` 末尾 `mod tests` 的最后一个测试之后、闭合 `}` 之前:

```rust
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
```

- [ ] **Step 2: 在旧实现上运行**

Run: `cargo test -p dozer-app -- git_hotspots::`
Expected: 除 **`dirty_paths_reports_non_ascii_names_as_real_utf8`** 外全部通过——这一条描述的是**有意的改进**(旧实现读 porcelain 文本,非 ASCII 路径会被 git 转义成八进制),所以在旧实现上 **FAILED** 是对的,这是本任务唯一的 RED。其余刻画测试若失败,改测试不改代码。

- [ ] **Step 3: 换实现**

文件顶部 `use crate::delivery;` 下加一行:`use bytegit::{Repo, StatusOptions};`

把 `fn head_short_sha` 整个函数替换为:

```rust
/// HEAD 提交的前 7 位。和迁移前一致只认仓库根(见 `delivery::open_exact`)。
fn head_short_sha(dir: &Path) -> Option<String> {
    let commit = delivery::open_exact(dir)?.head().ok()?.commit?;
    Some(commit.short(7))
}
```

把 `pub fn dirty_paths` 整个函数(含文档注释)替换为:

```rust
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
```

`use std::process::Command;` 保留(`recent_churn` 仍在用,它属于 P3)。

- [ ] **Step 4: 运行**

Run: `cargo test -p dozer-app -- git_hotspots::`
Expected: `git_hotspots::` 17 个**全部通过**,包括 Step 2 里 FAILED 的那条。

- [ ] **Step 5: 变异检验**

把 `dirty_paths` 里 `detect_renames: true` 临时改成 `false`,运行 `cargo test -p dozer-app -- dirty_paths_reports_only_the_new_path`。Expected: **FAILED**(旧路径 `old.rs` 与新路径同时出现)。还原。

- [ ] **Step 6: 门禁与 Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p1
rustfmt --edition 2024 crates/dozer-app/src/extensions/codehealth/git_hotspots.rs
cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "git_hotspots.rs" ; echo "(上面应为空)"
cargo test -p dozer-app 2>&1 | grep -E "test result|FAILED"
git status --short
git add crates/dozer-app/src/extensions/codehealth/git_hotspots.rs
git diff --cached --stat
git commit -m "refactor(codehealth): back dirty_paths and head_short_sha with bytegit

Non-ASCII file names are now real UTF-8 instead of git's octal-escaped form.

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

Expected: 全量测试只有"已知基线"里的失败。

---

### Task 6: 文档同步与收尾

**Files:**
- Modify(worktree 内): `CLAUDE.md`、`docs/superpowers/specs/2026-10-02-bytegit-design.md`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`、`docs/dozer-v2/bytegit-调用点盘点.md`
- Modify(记忆): `~/.claude/projects/-Users-chrischiang-Projects-CoralProjects-byteboy-dozer/memory/dozer-v2-panel-independence-and-bytegit.md`

**Interfaces:** 无代码接口。

- [ ] **Step 1: `CLAUDE.md` 增加 bytegit 条目**

在"关键裁决"列表的 byteui 条目之后加一条(措辞按仓库现有风格):

```markdown
- **Git 底层统一走 `bytegit`**(独立仓库 `byteboyai/bytegit`,与 digger 共用,`tag` 依赖,联调用不提交的 `[patch]`,规则同 byteui)。新增的 git 读取一律调 `bytegit::Repo`,**不要**再直接写 `Command::new("git")` 或 `git2::`。已迁移(P1):`delivery.rs` 的 `repo_root/is_dirty/file_statuses/branch/remote_url/local_branches/current_branch_has_commits`(保留为 bytegit 适配层,签名不变,P6 移除)、`git_hotspots` 的 `dirty_paths/head_short_sha`。**未迁移**(P2–P5):`git_log`/`file_history` 的历史与 diff、`usage` 的提交计数、`recent_churn`、`git_watch`、`init/clone/checkout`、`dozerd/projects.rs`。设计见 `docs/superpowers/specs/2026-10-02-bytegit-design.md`。`delivery::open_exact` 保持"只认仓库根"的旧语义,是否改为向上查找是待决事项(规格 §8)。
```

- [ ] **Step 2: 规格同步**

在 `docs/superpowers/specs/2026-10-02-bytegit-design.md`:
1. §4.2、§4.3 的 API 与实现对齐:`HeadInfo { branch, commit }`(没有 `short_id`、`has_commits` 字段,后者是方法);`Remote { name, url, push_url }`(没有 `kind`);`StatusOptions { include_untracked(隐含递归进未跟踪目录), include_ignored, detect_renames }`(没有 `include_untracked_files_in_dirs`);`FileState` 增加 `conflicted`;`local_branches`/`remotes` 升序。
2. §4.3 补一句:重命名时 libgit2 的 `entry.path()` 返回旧路径,实现改从 diff delta 取新路径;被忽略的目录只作为一个带尾部 `/` 的条目;路径不是 UTF-8 的条目被跳过。
3. §6 P1 一行:改为"`delivery.rs` 对应函数改成 bytegit 适配层,**签名不变**,调用点不动;适配层随 P6 删除"(原文是直接删除函数)。
4. §8 加 **O9**:项目目录在仓库子目录时 `open` 与向上查找语义不一致(`delivery::open_exact` 保留旧语义),待用户裁决;**O10**:`dirty_paths` 与 `recent_churn` 在子目录项目里口径不同。
5. §8 O1、O6 标为已完成并写明结论(口径:`is_dirty` 含未跟踪、不含被忽略;`file_statuses` 含未跟踪与被忽略;`current_branch_has_commits` 在 detached HEAD 时为 `true`)。

- [ ] **Step 3: 要求文档、盘点文档、记忆**

- `docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` §7 第 1 条后追加:`bytegit` P1 已完成(`v0.2.0`)。
- `docs/dozer-v2/bytegit-调用点盘点.md` §1 末尾加一行:"P1 之后,工作区状态 ×3、HEAD/分支/远程的重复实现已合并为 bytegit。"
- 记忆文件追加 P1 结果与待决事项 D1/D2(一两行)。

- [ ] **Step 4: Commit(worktree)**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p1
git status --short
git add CLAUDE.md docs/superpowers/specs/2026-10-02-bytegit-design.md docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md docs/dozer-v2/bytegit-调用点盘点.md
git diff --cached --stat
git commit -m "docs(bytegit): record P1 results, deviations and open decisions

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 5: 交给用户合并**

不要自己合并 `bytegit-p1` 到 `main`:向用户报告分支、提交列表、"已知基线"里的失败、待决事项 D1/D2,并请用户审阅后决定合并方式。合并后在 main 上重跑 `build_marks_head_branch_and_labels` 确认它在 main 上通过。

---

## Self-Review

**Spec coverage(规格 §6 的 P1 行与 §4.2–§4.3):**
- HEAD/分支/远程(§4.2):Task 1;工作区状态(§4.3):Task 2;`delivery.rs` 对应函数迁移、`git_hotspots::dirty_paths/head_short_sha`:Task 4、5;发布 `v0.2.0`:Task 3。
- 规格的 `head_commit_time`(§4.2)属于 P3(`dozerd` 的唯一使用者),不在 P1;规格 P1 一行没有列它,符合。
- 规格 §4.3 的 `include_untracked_files_in_dirs` 没有实现(现有三份实现都递归进未跟踪目录,没有不递归的使用者),Task 6 Step 2 回写规格。
- 规格 P1 行写的是"删除对应函数",本计划改为"适配层保留、签名不变",理由是 25 个调用点逐个改风险与收益不成比例,Task 6 Step 2 回写规格。
- 规格 O1(口径)与 O6(25 个调用点)在本计划的盘点阶段已核对,结论写进 Task 6 Step 2。

**Placeholder scan:** 无 TBD/TODO;"桩阶段"步骤说明了如何从最终文件机械推出桩,没有留空的实现细节。

**Type consistency:** `HeadInfo.commit`/`has_commits()`、`Remote.url`、`StatusOptions` 三字段、`FileState` 四字段、`StatusEntry.path/state` 在 Task 1、2 定义,Task 4、5 的代码使用同名;`delivery::open_exact` 在 Task 4 定义、Task 5 使用;`ChangeKind` 在 dozer 侧以 `bytegit::ChangeKind as GitChange` 区分 `delivery::ChangeKind`。

**Review Focus:** 五条各自有对应测试,并在 Task 2 Step 5、Task 4 Step 7、Task 5 Step 5 用变异检验验证了其中几条会真正失败。
