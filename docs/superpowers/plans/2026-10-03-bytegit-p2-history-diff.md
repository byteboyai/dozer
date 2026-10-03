# bytegit P2:历史与 diff 迁移 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 `bytegit` 增加提交历史、单提交改动、文件内容读取与分类、提交 vs 工作区对比(发 `v0.3.0`),并让 dozer 的 `file_history` 与 `git_log` 改用它:两个面板共用的 diff 内容类型下沉到中立模块,`file_history → git_log` 的面板间耦合消失,面板状态里不再出现 `git2::Oid`/`git2::Delta`,行为保持等价。

**Architecture:** 前半在 `bytegit` 仓库加 `content.rs`(内容读取与分类)与 `history.rs`(log / commit_files / previous_version / workdir_patch),全部在 `Repo` 上、不暴露 `git2` 类型。后半在 dozer 的独立 worktree 里:**先在旧实现上补刻画测试并确认通过**,再分两步切换——先迁 `file_history`(同时建中立的 `extensions/diff_content.rs`,耦合在这一步消失),再迁 `git_log`。迁移的证据是"同一批刻画测试在旧实现和新实现上都通过",测试正文只做机械的类型替换,不为迎合实现而改断言。提交图布局(`gleisbau`)按规格 §1 留在 `git_log`,只在它产出提交 id 的边界把 `git2::Oid` 转成 `CommitId`。

**Tech Stack:** Rust edition 2024、`git2 0.21`(仅 bytegit 内部与 `usage`/`gleisbau` 的既有用法)、dozer 侧 `bytegit` 以 git tag 依赖。

**Spec:** `docs/superpowers/specs/2026-10-02-bytegit-design.md`(§4.4、§5、§6 的 P2)。前序计划:`docs/superpowers/plans/2026-10-02-bytegit-p1-queries.md`(已完成,`v0.2.0`,dozer 侧在分支 `bytegit-p1`)。

**本计划中的代码已在草稿里完整跑过:** bytegit 84 个测试通过(P1 的 49 个 + 本计划 35 个),clippy(`--all-targets --all-features`)与 fmt 干净。dozer 侧在一份克隆里实际编译:刻画测试在**旧实现上** `git_log` 65 个、`file_history` 39 个测试通过;切换后 `git_log` 64 个(差的一个是随 `classify_diff_bytes` 一起删除的重复测试)、`file_history` 39 个、`delivery::` 29 个、`git_hotspots` 17 个通过,`cargo clippy -p dozer-app --all-targets` 在本计划触碰的文件上无新诊断。bytegit 的五个变异检验与 dozer 的两个变异检验都会让对应测试失败(见各任务)。执行者照抄即可;结果与此不符先停下排查,不要改测试迎合实现。

## 前置条件

**P1 的 dozer 侧迁移(分支 `bytegit-p1`,HEAD `a9d5c3d7`)尚未合并进 `main`**,而本计划依赖它引入的 `delivery::open_exact` 与 `bytegit` 依赖。两种做法,由用户决定,计划按 (b) 写命令:

- (a) 用户先审阅并合并 `bytegit-p1`,P2 从 `main` 开分支(Task 4 Step 1 里把基点 `bytegit-p1` 换成 `main`)。
- (b) 不等合并,P2 分支叠在 `bytegit-p1` 之上;P1 先合并后,P2 分支再合并即可(提交不重叠)。

## Global Constraints

- bytegit:Rust edition `2024`;公开 API 不得出现 `git2::*`;全部同步;不依赖 tokio/iced;`git2 = "0.21"` 与 dozer 对齐。
- bytegit 的测试夹具(`TempRepo`)不得依赖用户全局 git 配置(P0 已隔离);夹具新增方法同样只用 `git2`。
- dozer 侧**所有命令都在专用 worktree 里执行,路径写全**(`~/Projects/CoralProjects/byteboy/dozer-bytegit-p2/...`);主 checkout 常有并发会话与未提交改动,不得在其上改代码或 `git add -A`。提交前 `git status` 与 `git diff --cached --stat` 看全貌,只 `git add` 指定路径。
- dozer 的 `bytegit` 依赖一律用 tag:`bytegit = { git = "https://github.com/byteboyai/bytegit", tag = "v0.3.0" }`;本地联调的 `[patch]` 只放 `.cargo/config.toml`,**不提交**。**注意:** 带着本地 `[patch]` 构建会让 `Cargo.lock` 里 `bytegit`/`byteui` 的 `source = "git+..."` 行消失;提交 `Cargo.lock` 前必须把 `.cargo/config.toml` 里 `bytegit` 的 `[patch]` 段暂时注释掉再 `cargo update -p bytegit`(见 Task 5 Step 1),确认 diff 里 `source` 行都在、只有 bytegit 的版本与 tag 变了。
- 迁移阶段**不得改变用户可见行为**。发现旧行为有 bug 时,先等价迁移,再另起提交修复。本计划有意的差异只有"待决事项"里列出的两条(D3 保持、D4 仅错误文案)。
- dozer 门禁:触碰的文件不产生新诊断、基线之外无新失败,不是"全绿"(见"已知基线")。`cargo fmt -p dozer-app` 后用 `git status --short` 确认**只有本任务触碰的文件**变了;fmt 动了别的文件就 `git checkout` 回去。
- 提交信息用 `feat:`/`fix:`/`refactor:`/`test:`/`docs:` 前缀;提交末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。
- `git_log` 的 `gleisbau` 部分(`build()` 的布局)不动;`dozer-app/Cargo.toml` 的 `git2` 依赖**保留**(`usage`、`gleisbau` 仍在用,P3/P6 才能去掉)。

## 已知基线(在 `bytegit-p1` 上观察到,不是本计划引入的)

- `extensions::files::tests::delete_confirm_spec_reflects_pending_target` 在 `main` 上就失败(与 macOS 临时目录路径有关,见 P1 计划)。
- `extensions::git_log::tests::build_marks_head_branch_and_labels` 在**新建分支的 linked worktree 里**可能失败(它对 dozer 自己的仓库跑 `gleisbau`;P1 记录过,我没有查清根因;在独立克隆里它通过)。若在 worktree 里失败,先确认失败在 `bytegit-p1` 上同样存在,再按基线处理;合并回 `main` 后在 `main` 上重跑一次。
- `cargo build -p dozer-app` 有两条与本计划无关的 dead_code 警告(`usage/aggregate.rs::agent_token_share`、`git_accounts.rs::GitProvider::ALL`)。
- `cargo clippy -p dozer-app --all-targets -- -D warnings` 有既有错误(`homespace.rs`、`ssh.rs`、`todo/view.rs`、`files/update.rs` 等,见 P1 计划),**本计划触碰的文件不得新增**。

## Review Focus

规格对下列输入没有逐条说明,但真实使用者会遇到;每条都由指明的任务里的测试固定下来(行为均在草稿里实测确认):

1. **回滚二进制、超大、非 UTF-8 的文件必须字节原样写回。** 规格 §4.4 只给了带大小/编码判定的 `file_at`,拿它做回滚会让这类文件取不到内容或被有损转换;所以本计划增加 `file_bytes_at`。bytegit Task 1 `file_bytes_at_returns_exact_bytes_regardless_of_limits_or_encoding`;dozer Task 4 `rollback_to_restores_binary_non_utf8_bytes_exactly`。
2. **项目目录在仓库子目录里:报错,而不是悄悄给出错误的历史。** 迁移前 `file_history`/`git_log::commit_detail` 用 `Repository::open`(只认仓库根),子目录时报错;若换成向上查找,`file_path`(相对项目根)会被当成相对仓库根的 pathspec 而查错文件。必须沿用 P1 的 `delivery::open_exact`。dozer Task 4 `build_in_a_repo_subdirectory_is_an_error_not_a_wrong_history`、`commit_detail_in_a_repo_subdirectory_is_an_error`。
3. **文件名含 pathspec 通配符(`[`、`*`)。** 迁移前 `build` 把路径当 pathspec,`a[1].txt` 的历史里会混进 `a1.txt` 的提交(已实测)。保持原样、用测试固定为"已知怪癖",是否改成字面匹配见待决事项 D3。bytegit Task 2 与 dozer Task 4 各有一条。
4. **大小上限与编码的边界。** 恰好等于上限可渲染、多一字节不可;非 UTF-8 但不含 NUL 的内容不可渲染;磁盘上的超大文件不得整个读入内存(只读到上限 + 1 字节)且仍报真实大小;磁盘文件缺失/是目录 → 不可渲染而不是报错。bytegit Task 1 `classify_checks_size_then_nul_then_utf8`、`workdir_vs_commit_reports_the_real_size_of_an_oversized_disk_file` 等;dozer 沿用并保留现有 `diff_blob_content_accepts_blob_exactly_at_cap` 等测试。
5. **提交形态:合并提交相对第一父、根提交相对空树、改名不做检测(表现为删除 + 新增)、非 ASCII 路径原样、没有任何提交的仓库报"没有提交"而不是 panic。** bytegit Task 2 的 `commit_files_*`、`log_*` 测试;dozer Task 4 的 `commit_detail_of_a_merge_commit_is_relative_to_the_first_parent`、`commit_detail_does_not_detect_renames`、`commit_detail_keeps_non_ascii_paths_verbatim`、`build_on_a_repo_without_commits_is_an_error`。

## 待决事项(需要用户裁决,**本计划不改变这些行为**)

- **D3:`file_history` 里文件名含 `[`、`*` 时历史会混入别的文件的提交。** 这是迁移前就有的行为(`DiffOptions::pathspec` 把路径当 pathspec)。修复方式是字面匹配(`disable_pathspec_match`),属于单独的用户可见改动;要改时同时改 bytegit 的 `log_path_is_a_pathspec_so_glob_characters_match_other_files_known_quirk` 与 dozer 的 `build_treats_glob_characters_in_the_file_name_as_a_pathspec_known_quirk`。
- **D4:三处错误文案由 libgit2 的英文原文变成 bytegit/dozer 的中文文案**(只在出错时展示):不是仓库根(`不是 git 仓库的根目录: <路径>`)、仓库没有提交(`仓库还没有任何提交`)、回滚时该提交里没有这个文件(`该历史版本里没有文件 <路径>`)。"路径是目录而非文件"的文案(`该历史版本对应的不是一个文件`)与迁移前**完全相同**。如果希望逐字保持英文原文,需要在 bytegit 里透传 libgit2 的消息,请告知。

规格补充(无需裁决,Task 7 回写规格):规格 §4.4 没有覆盖 `file_history::diff_against_current` 需要的 unified patch 文本、回滚需要的原始字节,也没有区分"路径不是文件";本计划新增 `Repo::workdir_patch`、`Repo::file_bytes_at`、`Content::NotAFile`,`ContentPair` 两侧为 `Option<Content>`(`None` = 该侧没有这个文件),`LogOptions` 不含 `since`(P3 用到时再加,结构体已 `non_exhaustive`)。`git_log` 的 `DiffFileEntry.patch/truncated` 在迁移前已无任何生产代码读取(只有测试),随迁移删除。

---

## File Structure

**bytegit 仓库**(`~/Projects/CoralProjects/byteboy/bytegit/`):

| 文件 | 变更 |
|------|------|
| `Cargo.toml` | 版本 `0.2.0` → `0.3.0` |
| `src/content.rs` | 新增:`ContentLimits`、`Content`、`ContentPair`、`Repo::{blob_text, file_at, file_bytes_at, workdir_vs_commit}` |
| `src/history.rs` | 新增:`LogOptions`、`Signature`、`CommitSummary`、`FileChange`、`Patch`、`Repo::{log, previous_version, commit_files, workdir_patch}` |
| `src/testutil.rs` | `commit_file` 改为复用新的 `commit_bytes`/`commit_staged`;新增 `merge_commit` |
| `src/lib.rs` | 声明并再导出新模块 |
| `src/{change,id,repo}.rs` | 去掉不再需要的 `#[allow(dead_code)]`(这些转换现在有非测试代码在用) |

**dozer 仓库**(专用 worktree `~/Projects/CoralProjects/byteboy/dozer-bytegit-p2/`,分支 `bytegit-p2`):

| 文件 | 变更 |
|------|------|
| `crates/dozer-app/Cargo.toml` | `bytegit` tag `v0.2.0` → `v0.3.0` |
| `crates/dozer-app/src/delivery.rs` | 新增 `open_exact_or_err` |
| `crates/dozer-app/src/extensions/diff_content.rs` | **新增**中立模块:`DiffBlobContent`、`MAX_DIFF_BLOB_BYTES`、`workdir_content`(Task 5)、`blob_pair_content`(Task 6) |
| `crates/dozer-app/src/extensions.rs` | 声明 `pub mod diff_content;` |
| `crates/dozer-app/src/extensions/file_history.rs` | 查询改用 bytegit,`oid` 改 `CommitId`,`previous_oid` 改名 `previous_commit`,不再 import `git_log` |
| `crates/dozer-app/src/extensions/git_log.rs` | blob/分类代码迁出,`commit_detail` 改用 bytegit,`oid`/`status`/`blob` 改 bytegit 类型,删除死字段 `patch`/`truncated` |
| `crates/dozer-app/src/app/update.rs`、`platform/file_history_overlay.rs` | 一处调用改名、两处类型/路径替换 |
| 规格、`CLAUDE.md`、要求文档、盘点文档 | Task 7 同步 |

---

### Task 1: bytegit — 夹具扩展与内容读取

**Files:**
- Modify: `src/testutil.rs`、`src/lib.rs`、`src/id.rs`、`src/repo.rs`
- Create: `src/content.rs`

**Interfaces:**
- Consumes(P0/P1 已有):`Repo::raw()`、`CommitId::{from_oid, oid}`、`BlobId::{from_oid, oid}`、`GitError`/`GitErrorKind`、`Repo::root()`
- Produces(Task 2、5、6 依赖):
  - `ContentLimits::new(max_bytes: usize) -> ContentLimits`(`const fn`,`Copy`)
  - `enum Content { Text(String), TooLarge { bytes: usize }, Binary, NotUtf8, NotAFile }`,`Content::into_text(self) -> Option<String>`
  - `struct ContentPair { pub old: Option<Content>, pub new: Option<Content> }`
  - `Repo::blob_text(&self, BlobId, ContentLimits) -> Result<Content, GitError>`
  - `Repo::file_at(&self, CommitId, &Path, ContentLimits) -> Result<Option<Content>, GitError>`
  - `Repo::file_bytes_at(&self, CommitId, &Path) -> Result<Option<Vec<u8>>, GitError>`
  - `Repo::workdir_vs_commit(&self, CommitId, &Path, ContentLimits) -> Result<ContentPair, GitError>`
  - `TempRepo::{commit_bytes(&self, rel: &str, content: &[u8], message: &str) -> CommitId, commit_staged(&self, message: &str) -> CommitId, merge_commit(&self, other: &str, message: &str) -> CommitId}`

- [ ] **Step 1: 夹具——`commit_file` 拆出 `commit_bytes`/`commit_staged`,新增 `merge_commit`**

在 `src/testutil.rs` 里,把现有的 `commit_file` 方法(从 `/// 写文件、暂存并提交到当前分支。` 到它的结尾 `}`)整体替换为:

```rust
    /// 写文件、暂存并提交到当前分支。
    pub fn commit_file(&self, rel: &str, content: &str, message: &str) -> CommitId {
        self.commit_bytes(rel, content.as_bytes(), message)
    }

    /// 同 [`TempRepo::commit_file`],内容是任意字节(二进制、非 UTF-8)。
    pub fn commit_bytes(&self, rel: &str, content: &[u8], message: &str) -> CommitId {
        let full = self.path().join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("创建目录失败");
        }
        std::fs::write(full, content).expect("写文件失败");
        self.stage(rel);
        self.commit_staged(message)
    }

    /// 把当前暂存区原样提交到当前分支(配合 `stage`/`stage_remove` 做一次含多个改动的提交)。
    pub fn commit_staged(&self, message: &str) -> CommitId {
        let mut index = self.repo.index().expect("读取 index 失败");
        let tree = self
            .repo
            .find_tree(index.write_tree().expect("写 tree 失败"))
            .expect("找不到 tree");
        let sig = self.signature();
        let parents: Vec<git2::Commit> = match self.repo.head() {
            Ok(head) => vec![head.peel_to_commit().expect("HEAD 不是提交")],
            Err(_) => Vec::new(), // 第一次提交
        };
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        let oid = self
            .repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
            .expect("提交失败");
        self.commits.set(self.commits.get() + 1);
        CommitId::from_oid(oid)
    }

    /// 把分支 `other` 合并进当前分支,生成一个两父提交(两边改的文件不能冲突)。
    pub fn merge_commit(&self, other: &str, message: &str) -> CommitId {
        let ours = self
            .repo
            .head()
            .expect("合并前需要至少一次提交")
            .peel_to_commit()
            .expect("HEAD 不是提交");
        let theirs = self
            .repo
            .find_branch(other, git2::BranchType::Local)
            .expect("找不到分支")
            .get()
            .peel_to_commit()
            .expect("分支不是提交");
        let mut merged = self
            .repo
            .merge_commits(&ours, &theirs, None)
            .expect("合并失败");
        assert!(!merged.has_conflicts(), "merge_commit 只用于无冲突合并");
        let tree = self
            .repo
            .find_tree(merged.write_tree_to(&self.repo).expect("写 tree 失败"))
            .expect("找不到 tree");
        let sig = self.signature();
        let oid = self
            .repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&ours, &theirs])
            .expect("提交失败");
        self.commits.set(self.commits.get() + 1);
        self.repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .expect("检出失败");
        CommitId::from_oid(oid)
    }
```

(`commit_file` 的行为不变:写文件、暂存该文件、提交;P0/P1 已有的夹具测试必须仍然全部通过。)

- [ ] **Step 2: 去掉本任务起已有非测试代码使用的 `#[allow(dead_code)]`**

P0/P1 里这几处因"后续阶段才会在非测试代码里用到"而放行;本任务的 `content.rs` 用到了下面两处(`from_oid`、`from_delta` 要到 Task 2 才有非测试代码使用,**先不要删**,否则 `-D warnings` 的门禁会失败):

- `src/id.rs`:删掉宏里 `oid` 方法上的 `#[allow(dead_code)]`(`from_oid` 上的先保留)。
- `src/repo.rs`:删掉 `raw()` 上的 `#[allow(dead_code)] // 后续阶段的方法使用`。

- [ ] **Step 3: 写 `src/content.rs`(实现 + 测试)**

```rust
//! 文件内容读取与分类:blob、某提交里的文件、提交 vs 工作区。

use std::io::Read;
use std::path::Path;

use crate::{BlobId, CommitId, GitError, Repo};

/// 读内容时的大小上限。超过上限的内容不读入内存,只报字节数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentLimits {
    pub max_bytes: usize,
}

impl ContentLimits {
    pub const fn new(max_bytes: usize) -> Self {
        Self { max_bytes }
    }
}

/// 一份内容的分类结果。判定顺序:先看大小,再看是否含 NUL 字节,最后看是否合法 UTF-8。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Text(String),
    /// 超过 [`ContentLimits::max_bytes`](恰好等于上限不算超过)。`bytes` 是实际大小。
    TooLarge {
        bytes: usize,
    },
    /// 含 NUL 字节。
    Binary,
    /// 不含 NUL,但不是合法 UTF-8。
    NotUtf8,
    /// 路径指向的不是文件(目录、子模块)。
    NotAFile,
}

impl Content {
    pub fn into_text(self) -> Option<String> {
        match self {
            Content::Text(s) => Some(s),
            _ => None,
        }
    }
}

/// 提交里的文件与工作区里的同一路径。`None` 表示那一侧没有这个文件
/// (提交里不存在,或磁盘上不存在/读不了)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentPair {
    pub old: Option<Content>,
    pub new: Option<Content>,
}

pub(crate) fn classify(bytes: &[u8], limits: ContentLimits) -> Content {
    if bytes.len() > limits.max_bytes {
        return Content::TooLarge { bytes: bytes.len() };
    }
    if bytes.contains(&0u8) {
        return Content::Binary;
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => Content::Text(s.to_string()),
        Err(_) => Content::NotUtf8,
    }
}

fn classify_blob(blob: &git2::Blob<'_>, limits: ContentLimits) -> Content {
    // 先看大小,超限的 blob 不拷贝内容。
    if blob.size() > limits.max_bytes {
        return Content::TooLarge { bytes: blob.size() };
    }
    classify(blob.content(), limits)
}

impl Repo {
    /// 读一个 blob 并分类。blob 不存在返回错误。
    pub fn blob_text(&self, blob: BlobId, limits: ContentLimits) -> Result<Content, GitError> {
        let blob = self.raw().find_blob(blob.oid())?;
        Ok(classify_blob(&blob, limits))
    }

    /// `commit` 里 `path` 处的内容。该提交里没有这个路径返回 `Ok(None)`;
    /// 路径是目录或子模块返回 `Content::NotAFile`。
    pub fn file_at(
        &self,
        commit: CommitId,
        path: &Path,
        limits: ContentLimits,
    ) -> Result<Option<Content>, GitError> {
        let Some(entry) = self.tree_entry(commit, path)? else {
            return Ok(None);
        };
        let object = entry.to_object(self.raw())?;
        Ok(Some(match object.as_blob() {
            Some(blob) => classify_blob(blob, limits),
            None => Content::NotAFile,
        }))
    }

    /// `commit` 里 `path` 处的原始字节,不做大小或编码判定(回滚要原样写回,
    /// 二进制、超大、非 UTF-8 的文件也必须能取到)。该提交里没有这个路径返回
    /// `Ok(None)`;路径是目录或子模块返回错误。
    pub fn file_bytes_at(
        &self,
        commit: CommitId,
        path: &Path,
    ) -> Result<Option<Vec<u8>>, GitError> {
        let Some(entry) = self.tree_entry(commit, path)? else {
            return Ok(None);
        };
        let object = entry.to_object(self.raw())?;
        match object.as_blob() {
            Some(blob) => Ok(Some(blob.content().to_vec())),
            None => Err(GitError::new(
                crate::GitErrorKind::Backend,
                "该历史版本对应的不是一个文件",
            )),
        }
    }

    /// 提交里的 `path`(旧侧)与工作区里的同一路径(新侧)。新侧直接读磁盘上的
    /// 实时内容(可能含未提交改动);文件缺失、是目录或读取失败时新侧为 `None`。
    /// 超过上限的磁盘文件只读到上限 + 1 字节就停,不整个读入内存。
    pub fn workdir_vs_commit(
        &self,
        commit: CommitId,
        path: &Path,
        limits: ContentLimits,
    ) -> Result<ContentPair, GitError> {
        let old = self.file_at(commit, path, limits)?;
        let new = read_disk(&self.root().join(path), limits);
        Ok(ContentPair { old, new })
    }

    fn tree_entry(
        &self,
        commit: CommitId,
        path: &Path,
    ) -> Result<Option<git2::TreeEntry<'static>>, GitError> {
        let commit = self.raw().find_commit(commit.oid())?;
        let tree = commit.tree()?;
        match tree.get_path(path) {
            Ok(entry) => Ok(Some(entry.to_owned())),
            Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

fn read_disk(path: &Path, limits: ContentLimits) -> Option<Content> {
    let file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limits.max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > limits.max_bytes {
        // 只读了 max+1 字节,真实大小以 metadata 为准(读的过程中文件可能在变,取较大者)。
        let size = std::fs::metadata(path).map_or(bytes.len(), |m| m.len() as usize);
        return Some(Content::TooLarge {
            bytes: size.max(bytes.len()),
        });
    }
    Some(classify(&bytes, limits))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GitErrorKind;
    use crate::testutil::TempRepo;

    const LIMIT: ContentLimits = ContentLimits::new(16);

    #[test]
    fn classify_checks_size_then_nul_then_utf8() {
        assert_eq!(classify(b"", LIMIT), Content::Text(String::new()));
        assert_eq!(classify(&[b'a'; 16], LIMIT), Content::Text("a".repeat(16)));
        assert_eq!(
            classify(&[b'a'; 17], LIMIT),
            Content::TooLarge { bytes: 17 }
        );
        assert_eq!(classify(b"a\0b", LIMIT), Content::Binary);
        // 超限优先于二进制判定。
        assert_eq!(classify(&[0u8; 17], LIMIT), Content::TooLarge { bytes: 17 });
        assert_eq!(classify(&[0xff, 0xfe, b'a'], LIMIT), Content::NotUtf8);
        // BOM 是合法 UTF-8,原样保留。
        assert_eq!(
            classify("\u{feff}hi".as_bytes(), LIMIT),
            Content::Text("\u{feff}hi".to_string())
        );
    }

    #[test]
    fn into_text_only_yields_text() {
        assert_eq!(Content::Text("x".into()).into_text(), Some("x".to_string()));
        assert_eq!(Content::Binary.into_text(), None);
    }

    #[test]
    fn file_at_reads_text_and_reports_absent_paths_as_none() {
        let t = TempRepo::new();
        let c = t.commit_file("a.txt", "one\n", "add");
        let repo = t.open();
        assert_eq!(
            repo.file_at(c, Path::new("a.txt"), LIMIT).unwrap(),
            Some(Content::Text("one\n".to_string()))
        );
        assert_eq!(repo.file_at(c, Path::new("nope.txt"), LIMIT).unwrap(), None);
    }

    #[test]
    fn file_at_a_directory_is_not_a_file() {
        let t = TempRepo::new();
        let c = t.commit_file("sub/a.txt", "x", "add");
        assert_eq!(
            t.open().file_at(c, Path::new("sub"), LIMIT).unwrap(),
            Some(Content::NotAFile)
        );
    }

    #[test]
    fn file_at_classifies_binary_oversized_and_non_utf8_blobs() {
        let t = TempRepo::new();
        t.commit_bytes("bin.dat", b"ab\0cd", "bin");
        t.commit_bytes("big.txt", &[b'a'; 17], "big");
        let c = t.commit_bytes("latin.txt", &[0xe9, b'a'], "latin");
        let repo = t.open();
        assert_eq!(
            repo.file_at(c, Path::new("bin.dat"), LIMIT).unwrap(),
            Some(Content::Binary)
        );
        assert_eq!(
            repo.file_at(c, Path::new("big.txt"), LIMIT).unwrap(),
            Some(Content::TooLarge { bytes: 17 })
        );
        assert_eq!(
            repo.file_at(c, Path::new("latin.txt"), LIMIT).unwrap(),
            Some(Content::NotUtf8)
        );
    }

    #[test]
    fn blob_text_reads_a_blob_by_id() {
        let t = TempRepo::new();
        let c = t.commit_file("a.txt", "one\n", "add");
        let repo = t.open();
        let tree = t.raw_repo().find_commit(c.oid()).unwrap().tree().unwrap();
        let blob = BlobId::from_oid(tree.get_path(Path::new("a.txt")).unwrap().id());
        assert_eq!(
            repo.blob_text(blob, LIMIT).unwrap(),
            Content::Text("one\n".to_string())
        );
        let missing: BlobId = "0123456789abcdef0123456789abcdef01234567".parse().unwrap();
        assert!(repo.blob_text(missing, LIMIT).is_err());
    }

    #[test]
    fn file_bytes_at_returns_exact_bytes_regardless_of_limits_or_encoding() {
        let t = TempRepo::new();
        let bin = [0u8, 159, 146, 150, 0xff, 0];
        let c = t.commit_bytes("bin.dat", &bin, "bin");
        let repo = t.open();
        assert_eq!(
            repo.file_bytes_at(c, Path::new("bin.dat")).unwrap(),
            Some(bin.to_vec())
        );
        assert_eq!(repo.file_bytes_at(c, Path::new("nope")).unwrap(), None);
    }

    #[test]
    fn file_bytes_at_a_directory_is_an_error() {
        let t = TempRepo::new();
        let c = t.commit_file("sub/a.txt", "x", "add");
        let err = t.open().file_bytes_at(c, Path::new("sub")).unwrap_err();
        assert_eq!(err.kind(), GitErrorKind::Backend);
        assert_eq!(err.message(), "该历史版本对应的不是一个文件");
    }

    #[test]
    fn workdir_vs_commit_reads_the_live_disk_content() {
        let t = TempRepo::new();
        let c1 = t.commit_file("a.txt", "one\n", "add");
        t.write_untracked("a.txt", "edited, not committed\n");
        let pair = t
            .open()
            .workdir_vs_commit(c1, Path::new("a.txt"), ContentLimits::new(1024))
            .unwrap();
        assert_eq!(pair.old, Some(Content::Text("one\n".to_string())));
        assert_eq!(
            pair.new,
            Some(Content::Text("edited, not committed\n".to_string()))
        );
    }

    #[test]
    fn workdir_vs_commit_old_side_is_none_when_the_commit_has_no_such_path() {
        let t = TempRepo::new();
        let c1 = t.commit_file("a.txt", "one\n", "add");
        t.write_untracked("later.txt", "x\n");
        let pair = t
            .open()
            .workdir_vs_commit(c1, Path::new("later.txt"), LIMIT)
            .unwrap();
        assert_eq!(pair.old, None);
        assert_eq!(pair.new, Some(Content::Text("x\n".to_string())));
    }

    #[test]
    fn workdir_vs_commit_new_side_is_none_when_disk_file_is_missing_or_a_directory() {
        let t = TempRepo::new();
        let c1 = t.commit_file("a.txt", "one\n", "add");
        t.delete_file("a.txt");
        std::fs::create_dir(t.path().join("a.txt")).unwrap();
        let repo = t.open();
        assert_eq!(
            repo.workdir_vs_commit(c1, Path::new("a.txt"), LIMIT)
                .unwrap()
                .new,
            None
        );
        std::fs::remove_dir(t.path().join("a.txt")).unwrap();
        assert_eq!(
            repo.workdir_vs_commit(c1, Path::new("a.txt"), LIMIT)
                .unwrap()
                .new,
            None
        );
    }

    #[test]
    fn workdir_vs_commit_reports_the_real_size_of_an_oversized_disk_file() {
        let t = TempRepo::new();
        let c1 = t.commit_file("a.txt", "one\n", "add");
        t.write_untracked("a.txt", &"x".repeat(100));
        let pair = t
            .open()
            .workdir_vs_commit(c1, Path::new("a.txt"), LIMIT)
            .unwrap();
        assert_eq!(pair.new, Some(Content::TooLarge { bytes: 100 }));
    }
}
```

- [ ] **Step 4: 在 `src/lib.rs` 声明并再导出**

把 `src/lib.rs` 改为:

```rust
//! bytegit:ByteBoy 系产品共用的本地 Git 底层。
//!
//! 全部同步;公开 API 不暴露 `git2` 类型。设计见 dozer 仓库
//! `docs/superpowers/specs/2026-10-02-bytegit-design.md`。

mod change;
mod content;
mod error;
mod id;
mod info;
mod repo;
mod status;
#[cfg(any(test, feature = "testutil"))]
pub mod testutil;

pub use change::ChangeKind;
pub use content::{Content, ContentLimits, ContentPair};
pub use error::{GitError, GitErrorKind};
pub use id::{BlobId, CommitId};
pub use info::{HeadInfo, Remote};
pub use repo::Repo;
pub use status::{FileState, StatusEntry, StatusOptions};
```

(`history` 模块要到 Task 2 才有,所以这里先只含 `content`。)

- [ ] **Step 5: 运行测试与门禁**

Run: `cargo test content:: && cargo test`
Expected: `content::` 12 个通过;全量通过(P1 的 49 个 + 12 个 = 61,夹具测试不受影响)。

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings`
Expected: 干净。

- [ ] **Step 6: 变异检验(确认 Review Focus 的测试真的会失败)**

逐个做、逐个还原(`git checkout src/content.rs`):

1. 把 `classify` 里的 `if bytes.len() > limits.max_bytes {` 改成 `>=`。Run: `cargo test content::`。Expected: `classify_checks_size_then_nul_then_utf8` 失败。
2. 把 `read_disk` 里的 `file.take(limits.max_bytes as u64 + 1)` 改成 `file.take(limits.max_bytes as u64)`。Expected: `workdir_vs_commit_reports_the_real_size_of_an_oversized_disk_file` 失败。

- [ ] **Step 7: Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/bytegit
git status --short
git add src/content.rs src/testutil.rs src/lib.rs src/id.rs src/repo.rs
git diff --cached --stat
git commit -m "feat: add content reading and classification (blob_text, file_at, file_bytes_at, workdir_vs_commit)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: bytegit — 历史与改动

**Files:**
- Create: `src/history.rs`
- Modify: `src/lib.rs`、`src/change.rs`、`src/id.rs`

**Interfaces:**
- Consumes: Task 1 的 `TempRepo::{commit_bytes, commit_staged, merge_commit}`;P1 的 `TempRepo::{stage, stage_remove, delete_file, write_untracked}`;`ChangeKind::from_delta`
- Produces(Task 5、6 依赖):
  - `LogOptions::new(max_count: usize) -> LogOptions`、`LogOptions::path(self, impl Into<PathBuf>) -> LogOptions`(`#[non_exhaustive]`,公开字段 `max_count: usize`、`path: Option<PathBuf>`)
  - `struct Signature { pub name: Option<String>, pub email: Option<String> }`
  - `struct CommitSummary { pub id: CommitId, pub parents: Vec<CommitId>, pub author: Signature, pub time: SystemTime, pub summary: String, pub message: String }`
  - `struct FileChange { pub path: PathBuf, pub old_path: Option<PathBuf>, pub kind: ChangeKind, pub old_blob: Option<BlobId>, pub new_blob: Option<BlobId> }`
  - `struct Patch { pub text: String, pub truncated: bool }`
  - `Repo::log(&self, LogOptions) -> Result<Vec<CommitSummary>, GitError>`
  - `Repo::previous_version(&self, &Path) -> Result<Option<CommitId>, GitError>`
  - `Repo::commit_files(&self, CommitId) -> Result<Vec<FileChange>, GitError>`
  - `Repo::workdir_patch(&self, CommitId, &Path, max_bytes: usize) -> Result<Patch, GitError>`

- [ ] **Step 1: 写 `src/history.rs`(实现 + 测试)**

```rust
//! 提交历史与单个提交的改动。

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::{BlobId, ChangeKind, CommitId, GitError, GitErrorKind, Repo};

/// `Repo::log` 的参数。`max_count` 必填,其余用方法设置;结构体 `non_exhaustive`,
/// 以后加字段(如 `since`)不破坏调用方。
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LogOptions {
    pub max_count: usize,
    /// 只列出改动过这个路径的提交。按 git pathspec 解释(与 `git log -- <path>` 相同:
    /// `*`、`[` 等有通配含义;没有 `--follow` 的重命名跟踪)。
    pub path: Option<PathBuf>,
}

impl LogOptions {
    pub fn new(max_count: usize) -> Self {
        Self {
            max_count,
            path: None,
        }
    }

    pub fn path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitSummary {
    pub id: CommitId,
    pub parents: Vec<CommitId>,
    pub author: Signature,
    /// 提交时间(committer time,等于 `git log --format=%ct`,**不是**作者时间)。
    /// 早于 1970 的时间戳按 `UNIX_EPOCH` 之前表示。
    pub time: SystemTime,
    /// 提交说明首行。
    pub summary: String,
    pub message: String,
}

/// 一个提交里的一个改动文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// 新路径;删除时为旧路径。
    pub path: PathBuf,
    /// 改名/复制前的路径,只有与 `path` 不同时才有。`commit_files` 不做改名检测,
    /// 所以目前总是 `None`。
    pub old_path: Option<PathBuf>,
    pub kind: ChangeKind,
    /// 旧版本 blob(新增文件为 `None`)。
    pub old_blob: Option<BlobId>,
    /// 新版本 blob(删除文件为 `None`)。
    pub new_blob: Option<BlobId>,
}

/// 一段 unified patch 文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    pub text: String,
    /// 是否因超过上限而被截断(`text` 里不含任何截断提示,提示文案归调用方)。
    pub truncated: bool,
}

fn system_time(secs: i64) -> SystemTime {
    if secs >= 0 {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs as u64)
    } else {
        SystemTime::UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs())
    }
}

fn path_of(delta: &git2::DiffDelta<'_>) -> Option<PathBuf> {
    delta
        .new_file()
        .path()
        .or_else(|| delta.old_file().path())
        .map(Path::to_path_buf)
}

impl Repo {
    /// 从 HEAD 起按提交时间倒序列出提交。带 `path` 时只保留改动过该路径的提交
    /// (合并提交相对第一父判断,根提交相对空树)。HEAD 未诞生(没有提交)返回
    /// `GitErrorKind::NoCommits`。
    pub fn log(&self, opts: LogOptions) -> Result<Vec<CommitSummary>, GitError> {
        let repo = self.raw();
        let mut revwalk = repo.revwalk()?;
        if let Err(e) = revwalk.push_head() {
            // HEAD 指向未诞生的分支(还没有提交):libgit2 报的是"引用不存在",换成明确的分类。
            return Err(match repo.head() {
                Err(h) if h.code() == git2::ErrorCode::UnbornBranch => {
                    GitError::new(GitErrorKind::NoCommits, "仓库还没有任何提交")
                }
                _ => e.into(),
            });
        }
        revwalk.set_sorting(git2::Sort::TIME)?;
        let pathspec = opts.path.as_ref().map(|p| p.to_string_lossy().into_owned());

        let mut out = Vec::new();
        for oid in revwalk {
            if out.len() >= opts.max_count {
                break;
            }
            let commit = repo.find_commit(oid?)?;
            if let Some(spec) = &pathspec {
                let new_tree = commit.tree()?;
                let old_tree = match commit.parent(0) {
                    Ok(parent) => Some(parent.tree()?),
                    Err(_) => None,
                };
                let mut diff_opts = git2::DiffOptions::new();
                diff_opts.pathspec(spec);
                let diff = repo.diff_tree_to_tree(
                    old_tree.as_ref(),
                    Some(&new_tree),
                    Some(&mut diff_opts),
                )?;
                if diff.deltas().next().is_none() {
                    continue;
                }
            }
            let author = commit.author();
            out.push(CommitSummary {
                id: CommitId::from_oid(commit.id()),
                parents: commit.parent_ids().map(CommitId::from_oid).collect(),
                author: Signature {
                    name: author.name().ok().map(str::to_string),
                    email: author.email().ok().map(str::to_string),
                },
                time: system_time(commit.time().seconds()),
                summary: commit.summary().ok().flatten().unwrap_or("").to_string(),
                message: commit.message().ok().unwrap_or("").to_string(),
            });
        }
        Ok(out)
    }

    /// `path` 的"上一版本":最近一次改动它的提交**之前**的那个版本;只有一次提交
    /// 时回落到那唯一一次;没有任何提交碰过它(未跟踪)返回 `None`。
    pub fn previous_version(&self, path: &Path) -> Result<Option<CommitId>, GitError> {
        let log = self.log(LogOptions::new(2).path(path))?;
        Ok(log.get(1).or_else(|| log.first()).map(|c| c.id))
    }

    /// 一个提交改动了哪些文件。合并提交相对第一父,根提交相对空树(全部为新增)。
    /// 不做重命名/复制检测。
    pub fn commit_files(&self, commit: CommitId) -> Result<Vec<FileChange>, GitError> {
        let repo = self.raw();
        let commit = repo.find_commit(commit.oid())?;
        let new_tree = commit.tree()?;
        let old_tree = match commit.parent(0) {
            Ok(parent) => Some(parent.tree()?),
            Err(_) => None,
        };
        let diff = repo.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), None)?;
        let mut files = Vec::new();
        for delta in diff.deltas() {
            let Some(kind) = ChangeKind::from_delta(delta.status()) else {
                continue;
            };
            let Some(path) = path_of(&delta) else {
                continue;
            };
            let old_path = delta
                .old_file()
                .path()
                .filter(|old| *old != path.as_path())
                .map(Path::to_path_buf);
            let blob =
                |f: git2::DiffFile<'_>| (!f.id().is_zero()).then(|| BlobId::from_oid(f.id()));
            files.push(FileChange {
                path,
                old_path,
                kind,
                old_blob: blob(delta.old_file()),
                new_blob: blob(delta.new_file()),
            });
        }
        Ok(files)
    }

    /// `commit` 的树与**当前工作区**里 `path` 的 unified patch。直接读磁盘上的实时内容
    /// (含未提交改动)。两边相同时 `text` 为空。
    ///
    /// 截断规则:每追加一行之前检查已累积的字节数,达到 `max_bytes` 就停止并标记
    /// `truncated`,所以 `text` 可能略超过 `max_bytes`(最多多出最后一行)。
    pub fn workdir_patch(
        &self,
        commit: CommitId,
        path: &Path,
        max_bytes: usize,
    ) -> Result<Patch, GitError> {
        let repo = self.raw();
        let commit = repo.find_commit(commit.oid())?;
        let tree = commit.tree()?;
        let mut opts = git2::DiffOptions::new();
        opts.pathspec(path.to_string_lossy().into_owned());
        let diff = repo.diff_tree_to_workdir(Some(&tree), Some(&mut opts))?;

        let mut text = String::new();
        let mut truncated = false;
        diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
            if truncated {
                return true;
            }
            if text.len() >= max_bytes {
                truncated = true;
                return true;
            }
            if matches!(line.origin(), '+' | '-' | ' ') {
                text.push(line.origin());
            }
            text.push_str(&String::from_utf8_lossy(line.content()));
            true
        })?;
        Ok(Patch { text, truncated })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempRepo;

    /// c1: 新建 a.txt / c2: 新建无关的 b.txt / c3: 改 a.txt。
    fn three_commits() -> (TempRepo, [CommitId; 3]) {
        let t = TempRepo::new();
        let c1 = t.commit_file("a.txt", "one\n", "c1: add a.txt\n\nbody line");
        let c2 = t.commit_file("b.txt", "unrelated\n", "c2: add b.txt");
        let c3 = t.commit_file("a.txt", "two\n", "c3: change a.txt");
        (t, [c1, c2, c3])
    }

    fn ids(log: &[CommitSummary]) -> Vec<CommitId> {
        log.iter().map(|c| c.id).collect()
    }

    #[test]
    fn log_lists_newest_first_with_all_fields() {
        let (t, [c1, c2, c3]) = three_commits();
        let log = t.open().log(LogOptions::new(10)).unwrap();
        assert_eq!(ids(&log), vec![c3, c2, c1]);
        assert_eq!(log[2].summary, "c1: add a.txt");
        assert_eq!(log[2].message, "c1: add a.txt\n\nbody line");
        assert_eq!(log[2].author.name.as_deref(), Some("Test"));
        assert_eq!(log[2].author.email.as_deref(), Some("test@example.com"));
        assert!(log[2].parents.is_empty(), "根提交没有父提交");
        assert_eq!(log[1].parents, vec![c1]);
        assert!(log[0].time > log[1].time && log[1].time > log[2].time);
    }

    #[test]
    fn log_max_count_truncates_and_zero_is_empty() {
        let (t, [_, c2, c3]) = three_commits();
        let repo = t.open();
        assert_eq!(ids(&repo.log(LogOptions::new(2)).unwrap()), vec![c3, c2]);
        assert!(repo.log(LogOptions::new(0)).unwrap().is_empty());
    }

    #[test]
    fn log_with_path_keeps_only_commits_touching_it() {
        let (t, [c1, _, c3]) = three_commits();
        let log = t.open().log(LogOptions::new(10).path("a.txt")).unwrap();
        assert_eq!(ids(&log), vec![c3, c1]);
    }

    #[test]
    fn log_with_path_max_count_counts_matches_not_commits_walked() {
        let (t, [_, _, c3]) = three_commits();
        let log = t.open().log(LogOptions::new(1).path("a.txt")).unwrap();
        assert_eq!(ids(&log), vec![c3]);
    }

    #[test]
    fn log_with_path_includes_the_deleting_commit() {
        let t = TempRepo::new();
        let c1 = t.commit_file("a.txt", "one\n", "add");
        t.stage_remove("a.txt");
        let c2 = t.commit_staged("delete");
        let log = t.open().log(LogOptions::new(10).path("a.txt")).unwrap();
        assert_eq!(ids(&log), vec![c2, c1]);
    }

    #[test]
    fn log_path_does_not_match_a_sibling_with_the_same_prefix() {
        let t = TempRepo::new();
        t.commit_file("a.txt.bak", "x\n", "bak");
        let c2 = t.commit_file("a.txt", "x\n", "real");
        let log = t.open().log(LogOptions::new(10).path("a.txt")).unwrap();
        assert_eq!(ids(&log), vec![c2]);
    }

    #[test]
    fn log_path_under_a_directory_matches_by_full_relative_path() {
        let t = TempRepo::new();
        let c1 = t.commit_file("src/a.rs", "x\n", "src");
        t.commit_file("other/a.rs", "x\n", "other");
        let log = t.open().log(LogOptions::new(10).path("src/a.rs")).unwrap();
        assert_eq!(ids(&log), vec![c1]);
    }

    /// 已知怪癖(从迁移前的 `file_history::build` 原样带过来):`path` 按 pathspec 解释,
    /// 文件名里的 `[`、`*` 是通配符,所以 `a[1].txt` 的历史里会混进 `a1.txt` 的提交。
    /// 要改成字面匹配是单独的行为变更(见 P2 计划"待决事项" D3),改的时候同时改这条测试。
    #[test]
    fn log_path_is_a_pathspec_so_glob_characters_match_other_files_known_quirk() {
        let t = TempRepo::new();
        t.commit_file("a[1].txt", "x\n", "literal bracket");
        t.commit_file("a1.txt", "x\n", "a1");
        let summaries = |p: &str| -> Vec<String> {
            t.open()
                .log(LogOptions::new(10).path(p))
                .unwrap()
                .into_iter()
                .map(|c| c.summary)
                .collect()
        };
        assert_eq!(summaries("a1.txt"), vec!["a1"]);
        assert_eq!(summaries("a[1].txt"), vec!["a1", "literal bracket"]);
    }

    #[test]
    fn log_of_an_empty_repo_is_a_no_commits_error() {
        let t = TempRepo::new();
        let err = t.open().log(LogOptions::new(10)).unwrap_err();
        assert_eq!(err.kind(), GitErrorKind::NoCommits);
    }

    #[test]
    fn log_with_path_judges_a_merge_commit_against_its_first_parent_only() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "base");
        t.branch("other").checkout("other");
        let on_other = t.commit_file("b.txt", "b\n", "other adds b");
        t.checkout("main");
        t.commit_file("c.txt", "c\n", "main adds c");
        let merge = t.merge_commit("other", "merge other");
        let repo = t.open();

        let all = repo.log(LogOptions::new(10)).unwrap();
        let merge_summary = all.iter().find(|c| c.id == merge).unwrap();
        assert_eq!(merge_summary.parents.len(), 2);

        // 合并提交相对第一父(main 一侧)新增了 b.txt,所以它和 other 上的提交都算"碰过 b.txt"。
        let for_b = repo.log(LogOptions::new(10).path("b.txt")).unwrap();
        let got = ids(&for_b);
        assert_eq!(got.len(), 2);
        assert!(got.contains(&merge) && got.contains(&on_other));
    }

    #[test]
    fn previous_version_is_the_commit_before_the_latest_change() {
        let (t, [c1, _, _]) = three_commits();
        assert_eq!(
            t.open().previous_version(Path::new("a.txt")).unwrap(),
            Some(c1)
        );
    }

    #[test]
    fn previous_version_falls_back_to_the_only_commit() {
        let t = TempRepo::new();
        let c1 = t.commit_file("a.txt", "one\n", "add");
        t.commit_file("b.txt", "x\n", "other");
        assert_eq!(
            t.open().previous_version(Path::new("a.txt")).unwrap(),
            Some(c1)
        );
    }

    #[test]
    fn previous_version_of_an_untracked_path_is_none() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "one\n", "add");
        assert_eq!(
            t.open().previous_version(Path::new("nope.txt")).unwrap(),
            None
        );
    }

    #[test]
    fn commit_files_of_a_root_commit_are_all_added_without_old_blobs() {
        let t = TempRepo::new();
        t.write_untracked("a.txt", "one\n").stage("a.txt");
        t.write_untracked("sub/b.txt", "two\n").stage("sub/b.txt");
        let root = t.commit_staged("root");
        let mut files = t.open().commit_files(root).unwrap();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(files.len(), 2);
        for f in &files {
            assert_eq!(f.kind, ChangeKind::Added);
            assert_eq!(f.old_blob, None);
            assert!(f.new_blob.is_some());
            assert_eq!(f.old_path, None);
        }
        assert_eq!(files[1].path, PathBuf::from("sub/b.txt"));
    }

    #[test]
    fn commit_files_reports_modified_and_deleted_with_blob_ids() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "one\n", "a");
        t.commit_file("b.txt", "two\n", "b");
        t.write_untracked("a.txt", "one\nmodified\n").stage("a.txt");
        t.stage_remove("b.txt");
        let c = t.commit_staged("modify and delete");
        let files = t.open().commit_files(c).unwrap();
        let a = files.iter().find(|f| f.path == Path::new("a.txt")).unwrap();
        let b = files.iter().find(|f| f.path == Path::new("b.txt")).unwrap();
        assert_eq!(a.kind, ChangeKind::Modified);
        assert!(a.old_blob.is_some() && a.new_blob.is_some() && a.old_blob != a.new_blob);
        assert_eq!(b.kind, ChangeKind::Deleted);
        assert!(b.old_blob.is_some());
        assert_eq!(b.new_blob, None);
    }

    #[test]
    fn commit_files_does_not_detect_renames() {
        let t = TempRepo::new();
        t.commit_file("old.txt", "same content\nline 2\nline 3\n", "add");
        t.stage_remove("old.txt");
        t.write_untracked("new.txt", "same content\nline 2\nline 3\n")
            .stage("new.txt");
        let c = t.commit_staged("rename");
        let files = t.open().commit_files(c).unwrap();
        let kind_of = |name: &str| {
            files
                .iter()
                .find(|f| f.path == Path::new(name))
                .map(|f| f.kind)
        };
        assert_eq!(files.len(), 2);
        assert_eq!(kind_of("new.txt"), Some(ChangeKind::Added));
        assert_eq!(kind_of("old.txt"), Some(ChangeKind::Deleted));
        assert!(files.iter().all(|f| f.old_path.is_none()));
    }

    #[test]
    fn commit_files_of_a_merge_commit_are_relative_to_the_first_parent() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a\n", "base");
        t.branch("other").checkout("other");
        t.commit_file("b.txt", "b\n", "other adds b");
        t.checkout("main");
        t.commit_file("c.txt", "c\n", "main adds c");
        let merge = t.merge_commit("other", "merge");
        let files = t.open().commit_files(merge).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("b.txt"));
        assert_eq!(files[0].kind, ChangeKind::Added);
    }

    #[test]
    fn commit_files_keeps_non_ascii_paths_verbatim() {
        let t = TempRepo::new();
        let c = t.commit_file("文档/说明.md", "x\n", "cn");
        let files = t.open().commit_files(c).unwrap();
        assert_eq!(files[0].path, PathBuf::from("文档/说明.md"));
    }

    #[test]
    fn workdir_patch_is_empty_when_disk_matches_the_commit() {
        let (t, [c1, _, _]) = three_commits();
        t.write_untracked("a.txt", "one\n");
        let p = t
            .open()
            .workdir_patch(c1, Path::new("a.txt"), 20_000)
            .unwrap();
        assert_eq!(
            p,
            Patch {
                text: String::new(),
                truncated: false
            }
        );
    }

    #[test]
    fn workdir_patch_shows_prefixed_lines_when_disk_differs() {
        let (t, [c1, _, _]) = three_commits();
        let p = t
            .open()
            .workdir_patch(c1, Path::new("a.txt"), 20_000)
            .unwrap();
        assert!(p.text.contains("-one\n"), "{}", p.text);
        assert!(p.text.contains("+two\n"), "{}", p.text);
        assert!(!p.truncated);
    }

    #[test]
    fn workdir_patch_truncates_without_adding_a_notice() {
        let t = TempRepo::new();
        let many: String = (0..2000).map(|i| format!("line {i}\n")).collect();
        let c1 = t.commit_file("big.txt", &many, "big");
        let other: String = (0..2000).map(|i| format!("changed {i}\n")).collect();
        t.write_untracked("big.txt", &other);
        let p = t
            .open()
            .workdir_patch(c1, Path::new("big.txt"), 1_000)
            .unwrap();
        assert!(p.truncated);
        assert!(p.text.len() >= 1_000, "达到上限才停:{}", p.text.len());
        assert!(
            p.text.len() < 1_000 + 200,
            "最多多出最后一行:{}",
            p.text.len()
        );
        assert!(!p.text.contains("截断"));
        let full = t
            .open()
            .workdir_patch(c1, Path::new("big.txt"), usize::MAX)
            .unwrap();
        assert!(!full.truncated);
        assert!(full.text.len() > p.text.len());
    }

    #[test]
    fn workdir_patch_of_a_file_deleted_on_disk_shows_the_removal() {
        let (t, [c1, _, _]) = three_commits();
        t.delete_file("a.txt");
        let p = t
            .open()
            .workdir_patch(c1, Path::new("a.txt"), 20_000)
            .unwrap();
        assert!(p.text.contains("-one\n"), "{}", p.text);
    }

    #[test]
    fn system_time_handles_pre_epoch_timestamps() {
        assert_eq!(system_time(0), SystemTime::UNIX_EPOCH);
        assert_eq!(
            system_time(-60),
            SystemTime::UNIX_EPOCH - Duration::from_secs(60)
        );
    }
}
```

- [ ] **Step 2: 补全 `src/lib.rs`**

把 `src/lib.rs` 改为(在 Task 1 的基础上加了 `mod history;` 与对应的 `pub use`):

```rust
//! bytegit:ByteBoy 系产品共用的本地 Git 底层。
//!
//! 全部同步;公开 API 不暴露 `git2` 类型。设计见 dozer 仓库
//! `docs/superpowers/specs/2026-10-02-bytegit-design.md`。

mod change;
mod content;
mod error;
mod history;
mod id;
mod info;
mod repo;
mod status;
#[cfg(any(test, feature = "testutil"))]
pub mod testutil;

pub use change::ChangeKind;
pub use content::{Content, ContentLimits, ContentPair};
pub use error::{GitError, GitErrorKind};
pub use history::{CommitSummary, FileChange, LogOptions, Patch, Signature};
pub use id::{BlobId, CommitId};
pub use info::{HeadInfo, Remote};
pub use repo::Repo;
pub use status::{FileState, StatusEntry, StatusOptions};
```

- [ ] **Step 3: 去掉剩余的 `#[allow(dead_code)]`**

`history.rs` 现在在非测试代码里用到了这两处:

- `src/change.rs`:删掉 `ChangeKind::from_delta` 上的注释行 `// 后续阶段(commit_files/status)才会在非测试代码里用到。` 与 `#[allow(dead_code)]`。
- `src/id.rs`:删掉宏里 `from_oid` 方法上的 `#[allow(dead_code)]` 及其上一行注释 `// 后续阶段(log/diff)才会在非测试代码里用到这两个转换。`。

- [ ] **Step 4: 运行测试与门禁**

Run: `cargo test history:: && cargo test`
Expected: `history::` 23 个通过;全量 84 个通过。

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings`
Expected: 干净。

- [ ] **Step 5: 变异检验**

逐个做、逐个还原(`git checkout src/history.rs`):

1. 把 `log` 里的 `if diff.deltas().next().is_none() {` 改成 `if false && diff.deltas().next().is_none() {`(相当于忽略 `path` 过滤)。Expected: 7 个测试失败(`log_with_path_keeps_only_commits_touching_it`、`log_path_does_not_match_a_sibling_with_the_same_prefix`、`log_path_under_a_directory_matches_by_full_relative_path`、`log_with_path_judges_a_merge_commit_against_its_first_parent_only`、`log_path_is_a_pathspec_so_glob_characters_match_other_files_known_quirk`、`previous_version_is_the_commit_before_the_latest_change`、`previous_version_of_an_untracked_path_is_none`)。
2. 把 `commit_files` 里的 `commit.parent(0)` 改成 `commit.parent(1)`。Expected: `commit_files_does_not_detect_renames`、`commit_files_reports_modified_and_deleted_with_blob_ids`、`commit_files_of_a_merge_commit_are_relative_to_the_first_parent` 失败。
3. 把 `workdir_patch` 里的 `if text.len() >= max_bytes {` 改成 `if text.len() > max_bytes * 100 {`。Expected: `workdir_patch_truncates_without_adding_a_notice` 失败。

- [ ] **Step 6: Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/bytegit
git status --short
git add src/history.rs src/lib.rs src/change.rs src/id.rs
git diff --cached --stat
git commit -m "feat: add log, commit_files, previous_version and workdir_patch

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: bytegit — 发布 v0.3.0

**Files:**
- Modify: `Cargo.toml`(`version = "0.3.0"`)

**Interfaces:** Produces:git tag `v0.3.0`(Task 5、6 的 dozer 依赖)。

- [ ] **Step 1: 升版本并全量门禁**

把 `Cargo.toml` 的 `version = "0.2.0"` 改为 `"0.3.0"`。

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings && cargo test && cargo test --all-features`
Expected: 全部通过,84 个测试。

- [ ] **Step 2: Commit 并推送 main(对外可见操作:先向用户确认)**

向用户确认"现在推送 `main` 并打 tag `v0.3.0`"。获得明确同意后:

```bash
cd ~/Projects/CoralProjects/byteboy/bytegit
git add Cargo.toml
git commit -m "chore: release 0.3.0

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git push origin main
gh run watch "$(gh run list --repo byteboyai/bytegit --limit 1 --json databaseId -q '.[0].databaseId')" --repo byteboyai/bytegit --exit-status
```

Expected: CI(macOS)通过。**CI 红则先修,不打 tag。**(bytegit 不跟踪 `Cargo.lock`,只提交 `Cargo.toml`。)

- [ ] **Step 3: 打 tag 并验证可被消费**

```bash
git tag -a v0.3.0 -m "bytegit v0.3.0: log, commit_files, content reading, workdir diff"
git push origin v0.3.0
cd "$(mktemp -d)" && cargo new --lib consume -q && cd consume
cargo add bytegit --git https://github.com/byteboyai/bytegit --tag v0.3.0 --features testutil
cat >> src/lib.rs <<'EOF'

#[cfg(test)]
mod t {
    #[test]
    fn consumes_bytegit() {
        let t = bytegit::testutil::TempRepo::new();
        t.commit_file("a.txt", "x\n", "one");
        let repo = t.open();
        let log = repo.log(bytegit::LogOptions::new(5).path("a.txt")).unwrap();
        assert_eq!(log.len(), 1);
        let files = repo.commit_files(log[0].id).unwrap();
        assert_eq!(files[0].kind, bytegit::ChangeKind::Added);
        let content = repo
            .file_at(log[0].id, std::path::Path::new("a.txt"), bytegit::ContentLimits::new(1024))
            .unwrap();
        assert_eq!(content, Some(bytegit::Content::Text("x\n".to_string())));
    }
}
EOF
cargo test
```

Expected: `consumes_bytegit ... ok`。

---

### Task 4: dozer — 在旧实现上补刻画测试

**Files:**
- Modify(worktree 内): `crates/dozer-app/src/extensions/file_history.rs`(只改 `tests` 模块)、`crates/dozer-app/src/extensions/git_log.rs`(只改 `tests` 模块)

**Interfaces:** 无新接口。产出 17 个刻画测试(`file_history` 12 个、`git_log` 5 个),在**旧实现**上全部通过;Task 5、6 保持它们的断言不变,只做机械的类型替换。

- [ ] **Step 1: 建专用 worktree 与分支**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-bytegit-p2 -b bytegit-p2 bytegit-p1
mkdir -p ../dozer-bytegit-p2/.cargo && cp .cargo/config.toml ../dozer-bytegit-p2/.cargo/config.toml
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p2 && git log --oneline | head -1 && git status --short
```

Expected: 干净、分支 `bytegit-p2`,HEAD 与 `bytegit-p1` 一致。若用户已先合并 `bytegit-p1`,把命令里的基点 `bytegit-p1` 换成 `main`。`.cargo/config.toml` 是不提交的本地联调文件(已被 `.gitignore`);里面的 `[patch."https://github.com/byteboyai/bytegit"]` 指向本地 `../bytegit`,此时它已含 Task 1–3 的改动。此后**所有命令都用 `~/Projects/CoralProjects/byteboy/dozer-bytegit-p2/` 前缀**。

- [ ] **Step 2: 给 `file_history.rs` 的 `tests` 模块加 12 个刻画测试**

在 `crates/dozer-app/src/extensions/file_history.rs` 的 `mod tests` 里,`async fn close_clears_state()` 的 `#[tokio::test]` 之前插入(用的是该模块已有的 `git()`、`mkrepo()` 命令行夹具):

```rust
    /// 在 `repo` 里写入 `rel`(任意字节)并提交。
    fn commit_bytes(repo: &Path, rel: &str, bytes: &[u8], msg: &str) {
        let full = repo.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, bytes).unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-qm", msg]);
    }

    /// 刻画测试:`build` 把文件名当 pathspec,`[`、`*` 是通配符,所以 `a[1].txt` 的历史里会
    /// 混进 `a1.txt` 的提交。这是迁移前就有的怪癖,迁移保持原样(见 P2 计划"待决事项" D3)。
    #[test]
    fn build_treats_glob_characters_in_the_file_name_as_a_pathspec_known_quirk() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        git(&repo, &["init", "-q"]);
        commit_bytes(&repo, "a[1].txt", b"x\n", "literal bracket");
        commit_bytes(&repo, "a1.txt", b"x\n", "a1");
        let summaries = |p: &str| -> Vec<String> {
            build(&repo, Path::new(p), 10)
                .unwrap()
                .entries
                .into_iter()
                .map(|e| e.summary)
                .collect()
        };
        assert_eq!(summaries("a1.txt"), vec!["a1"]);
        assert_eq!(summaries("a[1].txt"), vec!["a1", "literal bracket"]);
    }

    #[test]
    fn build_in_a_repo_subdirectory_is_an_error_not_a_wrong_history() {
        // 迁移前用 `Repository::open`(只认仓库根):项目目录在仓库子目录时直接报错,
        // 而不是在"相对仓库根"的 pathspec 下悄悄给出错误的历史。
        let (_d, repo) = mkrepo();
        std::fs::create_dir(repo.join("sub")).unwrap();
        assert!(build(&repo.join("sub"), Path::new("a.txt"), 10).is_err());
    }

    #[test]
    fn build_on_a_repo_without_commits_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        assert!(build(dir.path(), Path::new("a.txt"), 10).is_err());
    }

    #[test]
    fn build_on_a_plain_directory_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(build(dir.path(), Path::new("a.txt"), 10).is_err());
    }

    #[test]
    fn rollback_to_restores_binary_non_utf8_bytes_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        git(&repo, &["init", "-q"]);
        let bin = [0u8, 159, 146, 150, 0xff, 0, 1];
        commit_bytes(&repo, "img.bin", &bin, "add binary");
        commit_bytes(&repo, "img.bin", b"changed\0", "change binary");
        let snapshot = build(&repo, Path::new("img.bin"), 10).unwrap();
        let first = snapshot.entries[1].oid;
        rollback_to(&repo, Path::new("img.bin"), first).expect("二进制文件也应能回滚");
        assert_eq!(std::fs::read(repo.join("img.bin")).unwrap(), bin);
    }

    #[test]
    fn rollback_to_errors_when_the_commit_has_no_such_file() {
        let (_d, repo) = mkrepo();
        let snapshot = build(&repo, Path::new("b.txt"), 10).unwrap();
        let c2 = snapshot.entries[0].oid;
        assert!(rollback_to(&repo, Path::new("never-existed.txt"), c2).is_err());
        assert!(!repo.join("never-existed.txt").exists(), "出错时不能写出文件");
    }

    #[test]
    fn rollback_to_a_directory_path_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        git(&repo, &["init", "-q"]);
        commit_bytes(&repo, "sub/a.txt", b"x\n", "add");
        let snapshot = build(&repo, Path::new("sub/a.txt"), 10).unwrap();
        let c = snapshot.entries[0].oid;
        assert!(rollback_to(&repo, Path::new("sub"), c).is_err());
    }

    #[test]
    fn previous_oid_is_the_version_before_the_latest_change() {
        let (_d, repo) = mkrepo();
        let snapshot = build(&repo, Path::new("a.txt"), 10).unwrap();
        let c1 = snapshot.entries[1].oid;
        assert_eq!(previous_oid(&repo, Path::new("a.txt")).unwrap(), Some(c1));
    }

    #[test]
    fn previous_oid_falls_back_to_the_only_commit() {
        let (_d, repo) = mkrepo();
        let snapshot = build(&repo, Path::new("b.txt"), 10).unwrap();
        let c2 = snapshot.entries[0].oid;
        assert_eq!(previous_oid(&repo, Path::new("b.txt")).unwrap(), Some(c2));
    }

    #[test]
    fn previous_oid_of_an_untracked_file_is_none() {
        let (_d, repo) = mkrepo();
        assert_eq!(previous_oid(&repo, Path::new("nope.txt")).unwrap(), None);
    }

    #[test]
    fn diff_against_current_truncates_and_appends_the_notice() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        git(&repo, &["init", "-q"]);
        let many: String = (0..5000).map(|i| format!("line {i}\n")).collect();
        commit_bytes(&repo, "big.txt", many.as_bytes(), "big");
        let changed: String = (0..5000).map(|i| format!("changed {i}\n")).collect();
        std::fs::write(repo.join("big.txt"), &changed).unwrap();
        let c1 = build(&repo, Path::new("big.txt"), 10).unwrap().entries[0].oid;
        let patch = diff_against_current(&repo, Path::new("big.txt"), c1).unwrap();
        assert!(
            patch.ends_with("\n… diff 过长,已截断显示\n"),
            "{}",
            &patch[patch.len() - 40..]
        );
        assert!(patch.len() >= MAX_PATCH_CHARS);
        assert!(patch.len() < MAX_PATCH_CHARS + 500);
    }
```

- [ ] **Step 3: 给 `git_log.rs` 的 `tests` 模块加 5 个刻画测试**

在 `crates/dozer-app/src/extensions/git_log.rs` 的 `mod tests` 里,`fn diff_blob_content_reads_modified_file_both_sides()` 的 `#[test]` 之前插入(用的是该模块已有的 `mkrepo_with_one_commit()`):

```rust
    /// 在 `repo` 里跑一条 git 命令(固定作者/提交者,不依赖用户配置)。
    fn git_cmd(repo: &Path, args: &[&str]) {
        let st = std::process::Command::new("git")
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

    fn commit_text(repo: &Path, rel: &str, content: &str, msg: &str) {
        let full = repo.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
        git_cmd(repo, &["add", "."]);
        git_cmd(repo, &["commit", "-qm", msg]);
    }

    /// 合并提交相对**第一父**算(与 `git show` 默认一致,不做三方 diff)。
    #[test]
    fn commit_detail_of_a_merge_commit_is_relative_to_the_first_parent() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        git_cmd(&repo, &["init", "-q"]);
        commit_text(&repo, "a.txt", "a\n", "base");
        git_cmd(&repo, &["checkout", "-q", "-b", "other"]);
        commit_text(&repo, "b.txt", "b\n", "other adds b");
        git_cmd(&repo, &["checkout", "-q", "-"]);
        commit_text(&repo, "c.txt", "c\n", "main adds c");
        git_cmd(&repo, &["merge", "-q", "--no-ff", "other", "-m", "merge other"]);

        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).unwrap();
        let merge_row = &snapshot.rows[0];
        assert!(merge_row.is_merge, "最新一行应是合并提交");
        let detail = commit_detail(&repo, merge_row.oid).unwrap();
        assert_eq!(detail.files.len(), 1, "{:?}", detail.files);
        assert_eq!(detail.files[0].path, "b.txt");
        assert_eq!(detail.files[0].status, git2::Delta::Added);
    }

    /// `commit_detail` 不做重命名检测:改名表现为"旧路径删除 + 新路径新增"。
    #[test]
    fn commit_detail_does_not_detect_renames() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        git_cmd(&repo, &["init", "-q"]);
        commit_text(&repo, "old.txt", "same content\nline 2\nline 3\n", "add");
        git_cmd(&repo, &["mv", "old.txt", "new.txt"]);
        git_cmd(&repo, &["commit", "-qm", "rename"]);

        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).unwrap();
        let detail = commit_detail(&repo, snapshot.rows[0].oid).unwrap();
        assert_eq!(detail.files.len(), 2, "{:?}", detail.files);
        let status_of = |name: &str| detail.files.iter().find(|f| f.path == name).map(|f| f.status);
        assert_eq!(status_of("new.txt"), Some(git2::Delta::Added));
        assert_eq!(status_of("old.txt"), Some(git2::Delta::Deleted));
    }

    #[test]
    fn commit_detail_keeps_non_ascii_paths_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        git_cmd(&repo, &["init", "-q"]);
        commit_text(&repo, "文档/说明.md", "x\n", "cn");
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).unwrap();
        let detail = commit_detail(&repo, snapshot.rows[0].oid).unwrap();
        assert_eq!(detail.files.len(), 1);
        assert_eq!(detail.files[0].path, "文档/说明.md");
    }

    /// 取不出文本的内容(二进制/非 UTF-8)不能当文本渲染:非 UTF-8 但不含 NUL 的 blob 也判不可渲染。
    #[test]
    fn diff_blob_content_rejects_non_utf8_without_nul() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        git_cmd(&repo, &["init", "-q"]);
        std::fs::write(repo.join("latin.txt"), [0xe9u8, b'a']).unwrap();
        git_cmd(&repo, &["add", "."]);
        git_cmd(&repo, &["commit", "-qm", "latin1"]);
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).unwrap();
        let detail = commit_detail(&repo, snapshot.rows[0].oid).unwrap();
        let entry = &detail.files[0];
        let git_repo = git2::Repository::open(&repo).unwrap();
        let content = diff_blob_content(&git_repo, entry.old_blob, entry.new_blob).unwrap();
        assert!(matches!(content, DiffBlobContent::NotRenderable { .. }));
    }

    /// 迁移前 `commit_detail` 用 `Repository::open`(只认仓库根):项目目录在仓库子目录时报错。
    #[test]
    fn commit_detail_in_a_repo_subdirectory_is_an_error() {
        let (_dir, repo) = mkrepo_with_one_commit();
        let snapshot = build(&repo, DEFAULT_MAX_COMMITS).unwrap();
        std::fs::create_dir(repo.join("sub")).unwrap();
        assert!(commit_detail(&repo.join("sub"), snapshot.rows[0].oid).is_err());
    }
```

- [ ] **Step 4: 在旧实现上运行,确认全部通过**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p2 && cargo test -p dozer-app extensions::file_history && cargo test -p dozer-app extensions::git_log`
Expected: `file_history` 39 个通过(新增 12 个),`git_log` 65 个通过(新增 5 个);失败只可能是"已知基线"里的 `build_marks_head_branch_and_labels`。**任何一个新增测试在旧实现上失败,说明我对旧行为的描述有误——先停下确认旧行为,不要改实现,也不要直接改断言迎合。**

- [ ] **Step 5: 门禁与 Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p2
cargo fmt -p dozer-app && git status --short
git add crates/dozer-app/src/extensions/file_history.rs crates/dozer-app/src/extensions/git_log.rs
git diff --cached --stat
git commit -m "test(git): characterize history/diff behavior before bytegit P2 migration

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: dozer — `file_history` 迁移与中立模块 `diff_content`

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`、`crates/dozer-app/src/delivery.rs`、`crates/dozer-app/src/extensions.rs`、`crates/dozer-app/src/extensions/file_history.rs`、`crates/dozer-app/src/app/update.rs`、`crates/dozer-app/src/platform/file_history_overlay.rs`、`Cargo.lock`
- Create: `crates/dozer-app/src/extensions/diff_content.rs`

**Interfaces:**
- Consumes: bytegit `v0.3.0`(Task 1–3 的全部 API);P1 的 `delivery::open_exact`;Task 4 的刻画测试
- Produces(Task 6 依赖):
  - `delivery::open_exact_or_err(path: &Path) -> Result<bytegit::Repo, String>`(`pub(crate)`)
  - `extensions::diff_content::{DiffBlobContent, MAX_DIFF_BLOB_BYTES, workdir_content}`:`enum DiffBlobContent { Text { old_text: String, new_text: String }, NotRenderable { reason: String } }`;`pub fn workdir_content(repo: &Repo, commit: CommitId, path: &Path) -> Result<DiffBlobContent, String>`
  - `file_history::{build, diff_against_current, rollback_to}` 的参数里 `git2::Oid` 换成 `bytegit::CommitId`;`previous_oid` 改名 `previous_commit`(返回 `Result<Option<CommitId>, String>`)

- [ ] **Step 1: 升依赖、让 `Cargo.lock` 指向 tag**

把 `crates/dozer-app/Cargo.toml` 里 `bytegit = { ... tag = "v0.2.0" }` 的 tag 改成 `v0.3.0`。

然后让 `Cargo.lock` 指向真实 tag(而不是本地 patch):临时把 `.cargo/config.toml` 里 `[patch."https://github.com/byteboyai/bytegit"]` 与下一行注释掉(保留 byteui 的 patch),运行 `cargo update -p bytegit`,再 `git diff Cargo.lock` 确认**只有** `bytegit` 的 version 与 `source` 的 tag 变化,`source = "git+..."` 行都在。确认后恢复 `.cargo/config.toml`(本地联调继续用 patch;之后若 `cargo build` 又去掉了 `source` 行,提交前用 `git checkout Cargo.lock` 还原再重复本步)。

- [ ] **Step 2: `delivery.rs` 新增 `open_exact_or_err`**

在 `crates/dozer-app/src/delivery.rs` 的 `open_exact`(P1 引入)之后、`open_upward` 之前加入:

```rust
/// [`open_exact`] 的带错误文本版本,给要把失败展示给用户的调用方(`Result<_, String>`)。
pub(crate) fn open_exact_or_err(path: &Path) -> Result<Repo, String> {
    open_exact(path).ok_or_else(|| format!("不是 git 仓库的根目录: {}", path.display()))
}
```

- [ ] **Step 3: 新建 `extensions/diff_content.rs`(中立模块),并在 `extensions.rs` 声明**

在 `crates/dozer-app/src/extensions.rs` 的 `pub mod diff_render;` 之前加一行 `pub mod diff_content;`。

新建 `crates/dozer-app/src/extensions/diff_content.rs`:

```rust
//! diff 面板共用的"两侧文本"内容:`git_log`(提交 vs 提交)与 `file_history`
//! (提交 vs 磁盘)都产出同一个 [`DiffBlobContent`],交给 CodeMirror diff 渲染。
//!
//! 这里是两个面板之间的中立层——`file_history` 不再 import `git_log`。
//! 内容读取与"能不能当文本"的判定在 `bytegit`,这里只负责把它的分类结果折成 UI 要的
//! 二选一(可渲染 / 不可渲染)与占位文案。

use bytegit::{CommitId, Content, ContentLimits, ContentPair, Repo};
use std::path::Path;

/// 单侧内容的字节上限(old/new 各自判定),超过就判定"不可渲染"。
pub const MAX_DIFF_BLOB_BYTES: usize = 512 * 1024;

const LIMITS: ContentLimits = ContentLimits::new(MAX_DIFF_BLOB_BYTES);

/// 提交 vs 磁盘的不可渲染占位文案(多一种"磁盘文件不存在"的原因)。
const REASON_WORKDIR: &str =
    "文件不是文本、超过大小上限,或磁盘文件当前不存在,不支持 CodeMirror 渲染";

/// 要么是可渲染的双侧文本,要么给出原因(供 UI 占位文案使用)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffBlobContent {
    Text { old_text: String, new_text: String },
    NotRenderable { reason: String },
}

fn not_renderable(reason: &str) -> DiffBlobContent {
    DiffBlobContent::NotRenderable {
        reason: reason.to_string(),
    }
}

/// `commit` 里 `path` 的历史内容 vs 磁盘上的实时内容。旧侧在该提交里不存在时按空字符串
/// (这是"文件历史"列表天然会包含的删除类记录,不是异常);新侧缺失/不可读视为不可渲染。
pub fn workdir_content(
    repo: &Repo,
    commit: CommitId,
    path: &Path,
) -> Result<DiffBlobContent, String> {
    let ContentPair { old, new } = repo
        .workdir_vs_commit(commit, path, LIMITS)
        .map_err(|e| e.message().to_string())?;
    let old_text = match old {
        None => Some(String::new()),
        Some(content) => content.into_text(),
    };
    let new_text = new.and_then(Content::into_text);
    match (old_text, new_text) {
        (Some(old_text), Some(new_text)) => Ok(DiffBlobContent::Text { old_text, new_text }),
        _ => Ok(not_renderable(REASON_WORKDIR)),
    }
}
```

- [ ] **Step 4: 迁移 `file_history.rs` 的生产代码**

1. 文件头部的 `use` 区(从 `use std::collections::HashMap;` 起到 `use iced_widget::{button, column, container, row, scrollable, text};` 止,模块文档注释保留)整体替换为:

```rust
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bytegit::{CommitId, LogOptions};

use crate::delivery::open_exact_or_err;
use crate::extensions::diff_content::{DiffBlobContent, workdir_content};
use byteui::interaction::icons;
use iced_widget::core::{Element, Length};
use iced_widget::{button, column, container, row, scrollable, text};
```

2. 全文件(含 `tests` 模块)做两处文本替换:`crate::extensions::git_log::DiffBlobContent` → `DiffBlobContent`;`git2::Oid` → `CommitId`。
3. `spawn_diff_content` 里 `spawn_blocking` 闭包的前两行(原先 `git2::Repository::open(...)` 与 `diff_blob_content_against_workdir(&repo, &repo_path2, &file_path2, oid)`)替换为:

```rust
            let repo = open_exact_or_err(&repo_path2)?;
            workdir_content(&repo, oid, &file_path2)
        })
```

4. 把原来从 `/// 手写 revwalk:从 HEAD 开始逐提交` 到 `previous_oid` 函数结尾(即 `build`、`diff_against_current`、`diff_blob_content_against_workdir`、`rollback_to`、`previous_oid` 五个函数及其文档注释,止于 `/// 弹窗卡片本体` 之前)整体替换为:

```rust
/// 该文件的提交历史:按时间倒序,只含真正改动过这个文件的提交,按 `max_count` 截断。
/// 没有 `git log --follow` 的 rename 跟踪;根提交相对空树对比。查询本身在
/// `bytegit::Repo::log`(带 `path`)。
///
/// 注意:文件历史稀疏时(仓库有很多提交、这个文件只被改过几次)需要遍历
/// 大量提交才能凑够 `max_count` 条结果——这是 `git log -- <path>` 的固有
/// 特性,不是 bug,不额外做"扫描上限"截断(YAGNI)。
pub fn build(
    repo_path: &Path,
    file_path: &Path,
    max_count: usize,
) -> Result<FileHistorySnapshot, String> {
    let repo = open_exact_or_err(repo_path)?;
    let log = repo
        .log(LogOptions::new(max_count).path(file_path))
        .map_err(|e| e.message().to_string())?;
    let entries = log
        .into_iter()
        .map(|c| FileHistoryEntry {
            oid: c.id,
            short_sha: c.id.short(7),
            summary: c.summary,
            author: c.author.name,
            time: unix_secs(c.time),
        })
        .collect();
    Ok(FileHistorySnapshot {
        repo_path: repo_path.to_path_buf(),
        file_path: file_path.to_path_buf(),
        entries,
    })
}

/// `SystemTime` → Unix 秒;早于 1970 的时间戳为负数。
fn unix_secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// `id` 对应提交的树 vs *当前工作目录*的这一个文件的 unified patch(含未提交改动)。
/// 两边内容相同时返回空字符串;超过 [`MAX_PATCH_CHARS`] 截断并追加一行提示。
pub fn diff_against_current(
    repo_path: &Path,
    file_path: &Path,
    id: CommitId,
) -> Result<String, String> {
    let repo = open_exact_or_err(repo_path)?;
    let patch = repo
        .workdir_patch(id, file_path, MAX_PATCH_CHARS)
        .map_err(|e| e.message().to_string())?;
    let mut text = patch.text;
    if patch.truncated {
        text.push_str("\n… diff 过长,已截断显示\n");
    }
    Ok(text)
}

/// 取 `id` 对应提交里 `file_path` 的原始字节,写入 `repo_path.join(file_path)`。
/// 不碰 git 索引,不 `git add`,是纯粹的文件系统写入——回滚后 git status 会显示这是一处
/// 未提交改动,交给用户/agent 自行决定要不要提交。二进制、超大、非 UTF-8 的文件也原样写回。
pub fn rollback_to(repo_path: &Path, file_path: &Path, id: CommitId) -> Result<(), String> {
    let repo = open_exact_or_err(repo_path)?;
    let bytes = repo
        .file_bytes_at(id, file_path)
        .map_err(|e| e.message().to_string())?
        .ok_or_else(|| format!("该历史版本里没有文件 {}", file_path.display()))?;
    std::fs::write(repo_path.join(file_path), bytes).map_err(|e| format!("写入文件失败: {e}"))?;
    Ok(())
}

/// 取文件「上一版本」对应的提交——供文件树右键菜单「回滚」一键还原用。定义:最近一次
/// 修改该文件的提交(HEAD 版本)之前的那个版本;若该文件在整个仓库历史里只有一次提交
/// (没有更早的版本),回落到那唯一一次提交(等价于把工作区还原到最近一次提交、丢弃
/// 未提交改动);文件不在 git 跟踪内(历史为空)则返回 `None`。
pub fn previous_commit(repo_path: &Path, file_path: &Path) -> Result<Option<CommitId>, String> {
    let repo = open_exact_or_err(repo_path)?;
    repo.previous_version(file_path)
        .map_err(|e| e.message().to_string())
}
```

5. `crates/dozer-app/src/app/update.rs`:`file_history::previous_oid(&repo_path2, &file_path2)` → `file_history::previous_commit(&repo_path2, &file_path2)`(只此一处)。
6. `crates/dozer-app/src/platform/file_history_overlay.rs`:`crate::extensions::git_log::DiffBlobContent` → `crate::extensions::diff_content::DiffBlobContent`(两处);`Option<(git2::Oid, String)>` → `Option<(bytegit::CommitId, String)>`。

- [ ] **Step 5: 迁移 `file_history.rs` 的测试(只做机械替换,断言不变)**

在 `file_history.rs` 的 `mod tests` 里:

1. 紧跟 `use super::*;` 之后加 `use bytegit::Repo;`。
2. `fake_oid` 改为:
   ```rust
   fn fake_oid(byte: u8) -> CommitId {
       format!("{byte:02x}").repeat(20).parse().unwrap()
   }
   ```
   `fake_entry` 里 `short_sha: oid.to_string().chars().take(7).collect(),` → `short_sha: oid.short(7),`。
3. 三个旧的 `diff_blob_content_against_workdir_*` 测试:`git2::Repository::open(&repo).unwrap()` → `Repo::discover(&repo).unwrap()`;调用 `diff_blob_content_against_workdir(&git_repo, &repo, Path::new("a.txt"), X)` → `workdir_content(&git_repo, X, Path::new("a.txt"))`;测试名里的 `diff_blob_content_against_workdir_` 前缀改成 `workdir_content_`。
4. Task 4 的 `previous_oid_*` 测试与调用:`previous_oid` → `previous_commit`。

- [ ] **Step 6: 编译并运行**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p2 && cargo fmt -p dozer-app && cargo build -p dozer-app 2>&1 | grep -E "^(warning|error)" -A6`
Expected: 无 error;警告只有"已知基线"里与本计划无关的 dead_code 两条(若 `diff_content.rs` 里出现未使用的警告,说明漏删了 Task 6 才用的代码,对照 Step 3 的完整文件)。

Run: `cargo test -p dozer-app extensions::file_history && cargo test -p dozer-app extensions::git_log && cargo test -p dozer-app delivery::`
Expected: `file_history` 39 个、`git_log` 65 个、`delivery::` 29 个全部通过(`git_log` 本任务未改动)。

- [ ] **Step 7: 变异检验(dozer 侧)**

逐个做、逐个还原:

1. 把 `file_history::build` 里的 `open_exact_or_err(repo_path)?` 换成 `bytegit::Repo::discover(repo_path).map_err(|e| e.message().to_string())?`。Expected: `build_in_a_repo_subdirectory_is_an_error_not_a_wrong_history` 失败。
2. 把 `rollback_to` 里的 `std::fs::write(repo_path.join(file_path), bytes)` 改成写 `String::from_utf8_lossy(&bytes).as_bytes()`。Expected: `rollback_to_restores_binary_non_utf8_bytes_exactly` 失败。

- [ ] **Step 8: 确认耦合已消失**

Run: `grep -n "git_log" crates/dozer-app/src/extensions/file_history.rs crates/dozer-app/src/platform/file_history_overlay.rs`
Expected: 只剩 `file_history.rs` 里一行**文档注释**(`DEFAULT_MAX_COUNT` 的说明里提到 `git_log::DEFAULT_MAX_COMMITS`),没有任何 `use` 或类型引用;`file_history_overlay.rs` 无输出。

- [ ] **Step 9: 门禁与 Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p2
cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "file_history|diff_content|delivery|file_history_overlay"
git status --short
git add Cargo.lock crates/dozer-app/Cargo.toml crates/dozer-app/src/delivery.rs crates/dozer-app/src/extensions.rs crates/dozer-app/src/extensions/diff_content.rs crates/dozer-app/src/extensions/file_history.rs crates/dozer-app/src/app/update.rs crates/dozer-app/src/platform/file_history_overlay.rs
git diff --cached --stat
git commit -m "refactor(file-history): back history queries with bytegit, neutral diff_content module

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

Expected: clippy 的 grep 无输出;`git diff --cached --stat` 里没有 `.cargo/config.toml`。

---

### Task 6: dozer — `git_log` 迁移

**Files:**
- Modify: `crates/dozer-app/src/extensions/diff_content.rs`、`crates/dozer-app/src/extensions/git_log.rs`

**Interfaces:**
- Consumes: Task 5 的 `delivery::open_exact_or_err`、`diff_content::{DiffBlobContent, MAX_DIFF_BLOB_BYTES}`;bytegit 的 `Repo::{commit_files, blob_text}`、`CommitId`、`BlobId`、`ChangeKind`
- Produces:`diff_content::blob_pair_content(repo: &Repo, old_blob: Option<BlobId>, new_blob: Option<BlobId>) -> Result<DiffBlobContent, String>`;`git_log` 里 `CommitRow.oid`、`Message::SelectCommit`/`DetailLoaded`/`DiffContentLoaded`、`State.selected`、`LoadedDiff.commit` 的类型从 `git2::Oid` 变为 `bytegit::CommitId`;`DiffFileEntry { path: String, status: ChangeKind, old_blob: Option<BlobId>, new_blob: Option<BlobId> }`

- [ ] **Step 1: `diff_content.rs` 加入 `blob_pair_content`**

1. 把文件头的 `use bytegit::{CommitId, Content, ContentLimits, ContentPair, Repo};` 改为 `use bytegit::{BlobId, CommitId, Content, ContentLimits, ContentPair, Repo};`。
2. 在 `const LIMITS` 之后加:

```rust
/// 提交 vs 提交的不可渲染占位文案。
const REASON_BLOB: &str = "文件不是文本,或超过大小上限,不支持 diff 渲染";
```

3. 在 `workdir_content` 之前加入:

```rust
/// 两个 blob 之间的双侧文本。`None` 侧按新增/删除文件语义当空字符串;
/// 非 `None` 侧任一超过 [`MAX_DIFF_BLOB_BYTES`]、含二进制内容或不是合法 UTF-8 都判定
/// "不可渲染"——不做部分截断渲染。
pub fn blob_pair_content(
    repo: &Repo,
    old_blob: Option<BlobId>,
    new_blob: Option<BlobId>,
) -> Result<DiffBlobContent, String> {
    let side = |blob: Option<BlobId>| -> Result<Option<String>, String> {
        match blob {
            None => Ok(Some(String::new())),
            Some(id) => repo
                .blob_text(id, LIMITS)
                .map(Content::into_text)
                .map_err(|e| e.message().to_string()),
        }
    };
    match (side(old_blob)?, side(new_blob)?) {
        (Some(old_text), Some(new_text)) => Ok(DiffBlobContent::Text { old_text, new_text }),
        _ => Ok(not_renderable(REASON_BLOB)),
    }
}
```

- [ ] **Step 2: 迁移 `git_log.rs` 的生产代码**

1. 文件头部的 `use` 区(从 `use crate::app::{App, HoverId};` 起到 `use std::sync::Mutex;` 止,模块文档注释保留)替换为:

```rust
use crate::app::{App, HoverId};
use crate::chrome::tab_widget::{
    NO_TAB_W_LIMIT, PANEL_TAB_PAD_LEFT, PANEL_TAB_PAD_X, PANEL_TAB_PAD_Y, tab_container_style,
    tab_label,
};
use crate::delivery::open_exact_or_err;
use crate::extensions::diff_content::{DiffBlobContent, blob_pair_content};
use crate::theme;
use bytegit::{BlobId, ChangeKind, CommitId};
use iced_widget::core::alignment;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use iced_widget::core::{Border, Element, Font, Length, Padding, Rectangle, mouse};
use iced_widget::{MouseArea, column, container, row, scrollable, text};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Mutex;
```

   (与原区域相比新增了 `delivery`、`diff_content`、`bytegit`、`FromStr` 四行 `use`;其余 `use` 原样保留。)
2. `CommitRow` 的 `oid` 字段:文档注释改为 `/// 这个 commit 的完整 id,选中详情用——`short_sha` 只够显示,\n    /// 不够拿去向仓库查提交。`,类型 `git2::Oid` → `CommitId`。
3. `build()` 里,把 `let full_sha = commit.oid.to_string();` 与 `let short_sha = full_sha.chars().take(7).collect();` 两行替换为:

```rust
            // gleisbau 给的是 git2 的 oid,在这里转成 bytegit 的 `CommitId`,
            // 不让 git2 类型流进面板状态。
            let id =
                CommitId::from_str(&commit.oid.to_string()).map_err(|e| e.message().to_string())?;
            let short_sha = id.short(7);
```

   并把同一闭包里构造 `CommitRow` 的 `oid: commit.oid,` 改为 `oid: id,`。(`graph.commit(commit.oid)`、`graph.labels.get_labels(&commit.oid)` 仍用 gleisbau 自己的 git2 oid,不动。)
4. 把原来从 `/// 单个改动文件:哪个文件、什么类型的改动、这个文件自己的 unified diff` 起,到 `diff_blob_content` 函数结尾(即 `DiffFileEntry`、`MAX_PATCH_CHARS`、`MAX_DIFF_BLOB_BYTES`、`DiffBlobContent`、`classify_diff_bytes`、`diff_blob_content`,止于 `#[derive(Debug, Clone)]\npub struct CommitDetail` 之前)整体替换为:

```rust
/// 单个改动文件:哪个文件、什么类型的改动,以及两侧 blob(CodeMirror diff 的内容来源)。
/// 派生 `Clone`(`Message::GitLogDetailLoaded` 要装 `Result<CommitDetail, _>`,
/// `Message` 本身 `derive(Debug, Clone)`)。
#[derive(Debug, Clone)]
pub struct DiffFileEntry {
    pub path: String,
    pub status: ChangeKind,
    /// 旧版本 blob(新增文件为 `None`)。
    pub old_blob: Option<BlobId>,
    /// 新版本 blob(删除文件为 `None`)。
    pub new_blob: Option<BlobId>,
}
```

5. `FileFilter::matches` 的函数体(从 `fn matches(self, status: git2::Delta) -> bool {` 到它所在 `impl` 的结尾 `}`)替换为:

```rust
    fn matches(self, status: ChangeKind) -> bool {
        match self {
            FileFilter::All => true,
            FileFilter::Added => status == ChangeKind::Added,
            FileFilter::Deleted => status == ChangeKind::Deleted,
            FileFilter::Renamed => matches!(status, ChangeKind::Renamed | ChangeKind::Copied),
            FileFilter::Modified => !matches!(
                status,
                ChangeKind::Added | ChangeKind::Deleted | ChangeKind::Renamed | ChangeKind::Copied
            ),
        }
    }
}
```

6. 把原来的 `commit_detail`(从 `/// 取某个提交改动了哪些文件、每个文件的 diff 文本。` 到该函数结尾,止于 `/// commit 搜索框` 之前)整体替换为:

```rust
/// 取某个提交改动了哪些文件。合并提交(≥2 parent)相对**第一父**算(与 `git show`
/// 默认行为一致,不做三方 diff——spec D5)。根提交(无 parent)相对空树算,等价于
/// "全部文件都是新增"。
pub fn commit_detail(repo_path: &Path, id: CommitId) -> Result<CommitDetail, String> {
    let repo = open_exact_or_err(repo_path)?;
    let files = repo
        .commit_files(id)
        .map_err(|e| e.message().to_string())?
        .into_iter()
        .map(|f| DiffFileEntry {
            path: f.path.to_string_lossy().into_owned(),
            status: f.kind,
            old_blob: f.old_blob,
            new_blob: f.new_blob,
        })
        .collect();
    Ok(CommitDetail { files })
}
```

7. `Message::SelectFile` 里 `spawn_blocking` 闭包的两行(原先 `git2::Repository::open(&repo_path2)...` 与 `diff_blob_content(&repo, old_blob, new_blob)`)替换为:

```rust
                    let repo = open_exact_or_err(&repo_path2)?;
                    blob_pair_content(&repo, old_blob, new_blob)
```

8. 文件列表视图里的颜色分支:`git2::Delta::Added =>` → `ChangeKind::Added =>`,`git2::Delta::Deleted =>` → `ChangeKind::Deleted =>`。
9. `status_glyph` 整个函数替换为:

```rust
fn status_glyph(status: ChangeKind) -> &'static str {
    match status {
        ChangeKind::Added => "+",
        ChangeKind::Deleted => "-",
        ChangeKind::Modified => "M",
        ChangeKind::Renamed => "R",
        ChangeKind::Copied => "C",
        ChangeKind::TypeChange => "?",
    }
}
```

10. 非 `tests` 模块范围内剩余的 `git2::Oid` 全部替换为 `CommitId`(`Message::SelectCommit`/`DetailLoaded`/`DiffContentLoaded`、`State.selected`、`LoadedDiff.commit`、`pending_diff_push`、`set_diff_sent_for`、`diff_sent_for`、`commit_list_view` 的 `selected` 参数等)。

- [ ] **Step 3: 迁移 `git_log.rs` 的测试(机械替换,断言不变)**

在 `git_log.rs` 的 `mod tests` 里:

1. 紧跟 `use super::*;` 之后加:

```rust
    use crate::extensions::diff_content::MAX_DIFF_BLOB_BYTES;
    use bytegit::Repo;

    /// 第 `n` 个固定的假 blob id(`n` 重复 20 次的 40 位十六进制)。
    fn test_blob(n: u8) -> BlobId {
        format!("{n:02x}").repeat(20).parse().unwrap()
    }

    /// 第 `n` 个固定的假提交 id(同 [`test_blob`])。
    fn test_oid(n: u8) -> CommitId {
        format!("{n:02x}").repeat(20).parse().unwrap()
    }
```

2. 把 `git2::Oid::from_bytes(&[N; 20]).unwrap()`(N 为数字字面量)全部替换为 `test_oid(N)`;其中 `select_file_clears_stale_diff_and_requests_fresh_load` 里的 `old_blob`、`new_blob` 两个变量改用 `test_blob(8)`、`test_blob(9)`。
3. `git2::Delta::X` → `ChangeKind::X`;`use git2::Delta;` → `use ChangeKind as Delta;`;`Delta::Typechange` → `Delta::TypeChange`;`fn file_entry(path: &str, status: git2::Delta)` → `status: ChangeKind`;测试辅助里的 `commit: git2::Oid,` → `commit: CommitId,`。
4. `git2::Repository::open(&repo).unwrap()` → `Repo::discover(&repo).unwrap()`;`diff_blob_content(&git_repo,` → `blob_pair_content(&git_repo,`。
5. 删掉所有 `DiffFileEntry` 字面量里的 `patch: ...,` 与 `truncated: false,` 两行;删掉 `commit_detail_uses_first_parent_diff` 里"至少一个文件真的算出了 diff 文本"那段 `assert!(detail.files.iter().any(|f| !f.patch.is_empty()), ...)` 及其注释,以及 `commit_detail_handles_root_commit_as_all_added` 里 `assert!(detail.files.iter().all(|f| !f.patch.is_empty()), ...)`。
6. 删掉测试 `classify_diff_bytes_matches_diff_blob_content_behavior`(函数已迁到 bytegit,等价判定由 bytegit 的 `classify_checks_size_then_nul_then_utf8` 与本文件保留的 `diff_blob_content_rejects_oversized_blob`、`diff_blob_content_accepts_blob_exactly_at_cap`、`diff_blob_content_rejects_binary_content`、Task 4 的 `diff_blob_content_rejects_non_utf8_without_nul` 覆盖)。
7. `build_populates_time_and_is_merge` 里,用 `git2::Repository::open(repo_root)` 校验 `is_merge` 的那一段(`let repo = git2::Repository::open(...)` 到该 `for` 循环结尾)替换为:

```rust
        let repo = Repo::discover(repo_root).expect("应能打开 dozer 自己的仓库");
        let parents: std::collections::HashMap<CommitId, usize> = repo
            .log(bytegit::LogOptions::new(DEFAULT_MAX_COMMITS))
            .expect("应能读出 dozer 自己的提交历史")
            .into_iter()
            .map(|c| (c.id, c.parents.len()))
            .collect();
        for row in &snapshot.rows {
            let parent_count = parents
                .get(&row.oid)
                .expect("snapshot 里的 oid 应在同一窗口的提交历史里");
            assert_eq!(
                row.is_merge,
                *parent_count >= 2,
                "is_merge 应与真实 git parent 数一致: {}",
                row.short_sha
            );
        }
```

- [ ] **Step 4: 编译并运行**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p2 && cargo fmt -p dozer-app && cargo build -p dozer-app 2>&1 | grep -E "^(warning|error)" -A6`
Expected: 无 error、无新警告。

Run: `cargo test -p dozer-app extensions::git_log && cargo test -p dozer-app extensions::file_history && cargo test -p dozer-app delivery:: && cargo test -p dozer-app git_hotspots`
Expected: `git_log` 64 个、`file_history` 39 个、`delivery::` 29 个、`git_hotspots` 17 个全部通过(`git_log` 比 Task 5 少 1 个是删掉的 `classify_diff_bytes_*`)。

- [ ] **Step 5: 确认旧代码已清走、全量无回归**

Run: `grep -n "classify_diff_bytes\|MAX_PATCH_CHARS\|git2::Delta\|git2::Oid\|find_blob\|diff_tree_to_tree" crates/dozer-app/src/extensions/git_log.rs`
Expected: 无输出(`git_log.rs` 里只剩 `gleisbau` 相关的 git2 方法调用,不再点名 `git2::` 类型)。

Run: `cargo test -p dozer-app 2>&1 | grep -E "FAILED|test result"`
Expected: 只有"已知基线"里的失败,其余通过。

- [ ] **Step 6: 门禁与 Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p2
cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "git_log|file_history|diff_content|delivery"
git status --short
git add crates/dozer-app/src/extensions/diff_content.rs crates/dozer-app/src/extensions/git_log.rs
git diff --cached --stat
git commit -m "refactor(git-log): back commit_detail and blob content with bytegit, drop dead patch fields

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

Expected: clippy 的 grep 无输出。

---

### Task 7: 文档同步与收尾

**Files:**
- Modify(worktree 内): `CLAUDE.md`、`docs/superpowers/specs/2026-10-02-bytegit-design.md`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`、`docs/dozer-v2/bytegit-调用点盘点.md`
- Modify(记忆): `~/.claude/projects/-Users-chrischiang-Projects-CoralProjects-byteboy-dozer/memory/dozer-v2-panel-independence-and-bytegit.md`

**Interfaces:** 无代码接口。

- [ ] **Step 1: `CLAUDE.md` 的 bytegit 条目更新进度**

在 `CLAUDE.md` 的 bytegit 关键裁决条目里,把"已迁移/未迁移"的描述更新为:P1(状态/HEAD/分支/远程)与 P2(`git_log`/`file_history` 的历史、改动、blob 读取、回滚取内容)已迁移;`git_log` 的提交图布局仍用 `gleisbau`(规格 §1 非目标)。并加一句:**面板之间共用的 diff 内容类型在 `extensions/diff_content.rs`,面板不得再互相 import。** 若条目里没有 P1 的措辞(`bytegit-p1` 尚未合并时),只改与 P2 相关的部分,不要替 P1 写。

- [ ] **Step 2: 规格同步**

在 `docs/superpowers/specs/2026-10-02-bytegit-design.md`:

1. §4.4 与实现对齐:
   - `LogOptions { max_count, path }`,用 `LogOptions::new(n).path(p)` 构造,`#[non_exhaustive]`;**没有 `since`**(P3 需要时再加);`path` 按 pathspec 解释(通配符有效、无 `--follow`)。
   - `CommitSummary { id, parents, author: Signature { name: Option<String>, email: Option<String> }, time: SystemTime, summary, message }`;`time` 是**提交者时间**(committer time,`git log --format=%ct`),不是作者时间(迁移前 `git_log`/`file_history` 的注释写"author time",实际取的也是 `commit.time()`,即提交者时间)。
   - `FileChange { path, old_path, kind, old_blob, new_blob }`;`commit_files` **不做改名检测**,所以 `old_path` 目前恒为 `None`;合并提交相对第一父,根提交相对空树。
   - `Content` 增加 `NotAFile`(路径是目录/子模块);`ContentPair { old: Option<Content>, new: Option<Content> }`(`None` = 该侧没有这个文件);`ContentLimits::new(max_bytes)`;判定顺序:先大小(恰好等于上限不算超)→ NUL → UTF-8。
   - `file_at` 返回 `Result<Option<Content>, _>`(提交里没有该路径为 `None`);**新增 `file_bytes_at`**(原始字节,回滚用,不做大小/编码判定;路径是目录时报错 `该历史版本对应的不是一个文件`);**新增 `workdir_patch(commit, path, max_bytes) -> Patch { text, truncated }`**(截断在追加每行之前检查,所以 `text` 可能略超上限,截断提示文案归调用方)。
   - §4.4 里"回滚(B2)"一段改为"`rollback_to` 留在 `file_history`,内部用 `file_bytes_at` + `fs::write`"。
   - `log` 在 HEAD 未诞生时返回 `GitErrorKind::NoCommits`。
2. §6 P2 一行:验收补充"刻画测试在旧/新实现上都通过";"删除的旧代码"补充"`git_log` 的 `DiffFileEntry.patch/truncated` 死字段";并注明 `extensions/diff_content.rs` 是两个面板的中立层。
3. §8 加 **O11**(= 本计划 D3):`log`/`file_history` 的 `path` 是 pathspec,文件名含 `[`、`*` 会混入别的文件的提交,待用户裁决;**O12**(= D4):三处错误文案由英文原文变中文。
4. §8 O4 补充 P2 的实际情况:并行比对用刻画测试而不是双实现并行;libgit2 与命令行 git 在 P2 覆盖的用法上未发现差异。

- [ ] **Step 3: 要求文档、盘点文档、记忆**

- `docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` §7 第 1 条后追加:`bytegit` P2 已完成(`v0.3.0`),`file_history → git_log` 的面板间耦合已消失。
- `docs/dozer-v2/bytegit-调用点盘点.md`:在 §1 末尾加一行"P2 之后,blob 读取与分类 ×2、历史/diff 的重复实现已合并为 bytegit";并 `grep -n "classify_diff_bytes\|diff_blob_content\|previous_oid\|read_side" docs/dozer-v2/bytegit-调用点盘点.md`,把命中的调用点标注为"P2 已迁移"。
- 记忆文件追加 P2 结果与待决事项 D3/D4(一两行),并更新"P1 分支是否已合并"的现状。

- [ ] **Step 4: Commit(worktree)**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p2
git status --short
git add CLAUDE.md docs/superpowers/specs/2026-10-02-bytegit-design.md docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md docs/dozer-v2/bytegit-调用点盘点.md
git diff --cached --stat
git commit -m "docs(bytegit): record P2 results, API additions and open decisions

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 5: 交给用户合并**

不要自己合并 `bytegit-p2` 到 `main`:向用户报告分支、提交列表、"已知基线"里的失败、待决事项 D3/D4,并请用户审阅后决定合并方式(`bytegit-p1` 与 `bytegit-p2` 的合并顺序见"前置条件")。合并后在 `main` 上重跑 `build_marks_head_branch_and_labels` 确认它通过。**GUI 层面的人工验收**(右键文件 → 查看此文件历史、回滚到上一版本、Git Log 面板选提交/选文件看 diff)本计划没有自动化覆盖渲染部分,合并前请用户在真实项目上点一遍。

---

## Self-Review

**Spec coverage(规格 §6 的 P2 行与 §4.4、§5):**
- `log`/`commit_files`/`previous_version`:Task 2;`blob_text`/`file_at`/`workdir_vs_commit` 与 `Content`/`ContentLimits`:Task 1;发布 `v0.3.0`:Task 3。
- 迁移 `git_log::{commit_detail, diff_blob_content, read_side, classify_diff_bytes}`:Task 6(`read_side` 是 `diff_blob_content` 的内部函数,随之被 `blob_pair_content` 取代);`file_history::*` 与 `rollback` 的取内容部分:Task 5。
- `git_log` 与 `file_history` 不再互相引用、`DiffBlobContent` 不再被 `file_history` 引用:Task 5 Step 8 用 grep 验证。
- 规格 §4.4 的 `commit_count`、`commit_count_by_day`、`churn`:属于 P3(`usage`、`recent_churn` 的使用者),不在 P2,规格 P2 一行没有列它们,符合。
- 规格 §4.4 的 `FileFilter::matches(git2::Delta)`/`status_glyph(git2::Delta)` 改用 `ChangeKind`(规格 §5 末尾):Task 6 Step 2 第 5、9 条。
- 规格没有覆盖、本计划补上的三项(`workdir_patch`、`file_bytes_at`、`NotAFile`)与 `LogOptions` 不含 `since`:在"待决事项"之后的"规格补充"说明,Task 7 Step 2 回写规格。
- 规格 O4(libgit2 与命令行 git 的边角差异):P2 覆盖的是 git2 对 git2(旧实现本来就是 git2),用刻画测试固定行为,Task 7 Step 2 回写。

**Placeholder scan:** 无 TBD/TODO;每个代码步骤都给了完整代码或"旧区域起止 → 新代码"的精确替换;测试迁移给了逐条机械替换规则,刻画测试给了完整代码。

**Type consistency:** `CommitId`/`BlobId`/`ChangeKind`/`Content`/`ContentPair`/`LogOptions`/`CommitSummary`/`FileChange`/`Patch` 在 Task 1、2 定义,Task 5、6 的代码使用同名同形;`delivery::open_exact_or_err` 在 Task 5 定义、Task 5/6 使用;`diff_content::{DiffBlobContent, workdir_content}` 在 Task 5 定义、`blob_pair_content` 在 Task 6 补入,Task 5 结束时 `diff_content.rs` 不含未使用的函数。

**Review Focus:** 五条各自有对应测试(分别见各条所指任务),其中 bytegit 侧 5 个变异检验、dozer 侧 2 个变异检验已在草稿里实际确认会让指定测试失败。
