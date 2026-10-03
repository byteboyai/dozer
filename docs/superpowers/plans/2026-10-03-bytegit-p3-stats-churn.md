# bytegit P3:提交统计与近期改动迁移 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 `bytegit` 增加 `head_commit_time`、`commit_count`、`commit_count_by_day`、`churn`(发 `v0.4.0`),并让 dozer 的三处命令行/重复实现改用它:`dozerd/projects.rs` 的"项目更新时间"(两处 `git` 命令)、`usage` 的提交计数(两个 `git2` revwalk 函数)、`git_hotspots::recent_churn`(一次 `git log` 命令),行为保持等价;`dozerd` 第一次依赖 `bytegit`。

**Architecture:** 前半在 `bytegit` 仓库加 `stats.rs`(四个 `Repo` 方法),并把 P2 里 `log` 内联的"从 HEAD 起遍历 + 未诞生分支分类"抽成共用的 `head_walk`。后半在 dozer 的独立 worktree 里:**先在旧实现上补刻画测试并确认通过**(26 个,覆盖提交者时间口径、子目录项目、空/裸/linked worktree/detached HEAD 仓库、合并提交、近期窗口、改名/删除/非 ASCII 路径),再按调用点分三步切换(`dozerd` → `usage` → `recent_churn`),同一批测试在新实现上通过,只有两条已知会变的测试被有意翻转。`churn` 的路径口径在 bytegit 里统一为**相对仓库根**(与 `status` 一致),`recent_churn` 的适配层再换成相对项目根,这样 P1 遗留的 O10(`dirty_paths` 相对仓库根 vs `recent_churn` 相对项目根)落实为"bytegit 统一、适配层各自转换",行为不变。

**Tech Stack:** Rust edition 2024、`git2 0.21`(仅 bytegit 内部与 `gleisbau`/`usage` 之外的既有用法)、dozer 侧 `bytegit` 以 git tag 依赖。

**Spec:** `docs/superpowers/specs/2026-10-02-bytegit-design.md`(§4.2 的 `head_commit_time`、§4.4 的 `commit_count`/`commit_count_by_day`/`churn`、§6 的 P3)。前序计划:`2026-10-02-bytegit-p1-queries.md`(`v0.2.0`)、`2026-10-03-bytegit-p2-history-diff.md`(`v0.3.0`)。

**本计划中的代码已在草稿里完整跑过:** bytegit 104 个测试通过(P2 的 84 个 + 本计划 20 个),clippy(`--all-targets --all-features`)与 fmt 干净。dozer 侧在一份克隆里实际编译:刻画测试在**旧实现上** `usage` 47 个、`git_hotspots` 27 个、`dozerd` 的 `projects` 22 个全部通过;切换后同样的三组全部通过(`git_hotspots` 里两条测试按"待决事项"有意翻转),`cargo test -p dozerd` 全量 462 个 lib 测试与全部集成测试通过,`cargo clippy -p dozer-app -p dozerd --all-targets` 在本计划触碰的文件上无诊断。bytegit 的 6 个变异检验与 dozer 的 2 个变异检验都会让对应测试失败(见各任务)。执行者照抄即可;结果与此不符先停下排查,不要改测试迎合实现。

## 前置条件

- **P2 的 bytegit 部分必须已发布**(`bytegit` 仓库 `main` 含 P2 的提交且已有 tag `v0.3.0`)。本计划的 bytegit 改动直接建在 P2 的 `history.rs`/`testutil.rs` 之上。
- dozer 侧 P3 分支依赖 P1 的 `delivery::open_exact` 等(`bytegit-p1`)与 P2 的 dozer 迁移(`bytegit-p2`)。这两个分支若尚未合并 `main`,P3 分支叠在 `bytegit-p2` 之上(Task 3 Step 1 的命令);若已合并,基点换成 `main`。

## Global Constraints

- bytegit:Rust edition `2024`;公开 API 不得出现 `git2::*`;全部同步;不依赖 tokio/iced;`git2 = "0.21"` 与 dozer 对齐。夹具新增方法只用 `git2`,不调用命令行。
- dozer 侧**所有命令都在专用 worktree 里执行,路径写全**(`~/Projects/CoralProjects/byteboy/dozer-bytegit-p3/...`);主 checkout 常有并发会话与未提交改动,不得在其上改代码或 `git add -A`。提交前 `git status` 与 `git diff --cached --stat` 看全貌,只 `git add` 指定路径。
- dozer 的 `bytegit` 依赖一律用 tag:`tag = "v0.4.0"`;本地联调的 `[patch]` 只放 `.cargo/config.toml`,**不提交**。带着本地 `[patch]` 构建会让 `Cargo.lock` 里 `bytegit`/`byteui` 的 `source = "git+..."` 行消失;提交 `Cargo.lock` 前必须把 `.cargo/config.toml` 里 `bytegit` 的 `[patch]` 段暂时注释掉再 `cargo update -p bytegit`(见 Task 4 Step 1),确认 diff 里 `source` 行都在、只有 bytegit 的版本与 tag 变了。
- 迁移阶段**不得改变用户可见行为**。本计划有意的差异只有"待决事项"里列出的(D5 保持为"更正确"的差异、非 ASCII 路径改进)。
- 新增的 git 读取一律调 `bytegit::Repo`(见 `CLAUDE.md`),不得新增 `Command::new("git")` 或 `git2::` 到生产代码。**刻画测试里**调用命令行 `git` 造仓库是允许的(它们要对**旧实现**成立,且要能控制提交者日期、造裸仓库/linked worktree)。
- dozer 门禁:触碰的文件不产生新诊断、基线之外无新失败(见"已知基线")。`cargo fmt -p <crate>` 后用 `git status --short` 确认**只有本任务触碰的文件**变了;fmt 动了别的文件就 `git checkout` 回去。
- 提交信息用 `feat:`/`fix:`/`refactor:`/`test:`/`docs:` 前缀;提交末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。
- `dozer-app/Cargo.toml` 的 `git2` 依赖**保留**:`gleisbau` 的传递依赖对齐仍需要;P6 再评估去掉。

## 已知基线

沿用 P2 计划的"已知基线"(`delete_confirm_spec_reflects_pending_target`、worktree 里的 `build_marks_head_branch_and_labels`、`cargo build -p dozer-app` 的两条无关 dead_code 警告、`cargo clippy -D warnings` 的既有错误)。**本计划触碰的文件不得新增诊断。**

## Review Focus

规格对下列输入没有逐条说明,但真实使用者会遇到;每条都由指明的任务里的测试固定下来(行为均在草稿里实测确认):

1. **时间口径是"提交者时间",不是作者时间。** 迁移前三处都用提交者时间(`commit.time()`、`git log -1 --format=%ct`、`git log --since`)。若实现里误用作者时间,经过 rebase/cherry-pick/`am` 的仓库结果会大不相同。dozer Task 3 的 `git_commits_by_day_buckets_by_utc_committer_day_not_author_day`、`recent_churn_counts_recent_commits_by_committer_date_not_author_date`(刻画测试里作者日期固定为 2000-01-01,提交者日期才是被统计的那个)。
2. **项目目录在仓库子目录里**:这三处迁移前都**向上查找**(与 P1/P2 里"只认仓库根"的那批不同),必须保持;`recent_churn` 还要求路径相对项目根、且不含项目目录之外的文件。Task 3 的 `git_commit_counts_look_upward_from_a_repo_subdirectory`、`updated_ms_looks_upward_from_a_repo_subdirectory`、`recent_churn_paths_are_relative_to_a_project_subdirectory_and_exclude_outside_files`。
3. **非常规仓库形态**:没有提交的仓库(计数 0 / 空 map / `recent_churn` 为 `None` / 更新时间回落到 `last_active_ms`)、裸仓库(更新时间**不计**——迁移前 `git rev-parse --show-toplevel` 在那里失败)、linked worktree(各取自己的 HEAD,v2 引入 worktree 后会常见)、detached HEAD。Task 3 对应测试;bytegit Task 1 的 `*_of_an_empty_repo_is_a_no_commits_error`、`head_commit_time_*`。
4. **合并提交**:被合并进来的提交只算一次(`commit_count`);`git log --name-only` 不列合并提交的文件,`churn` 同样不计(否则合并会把对侧分支的所有改动重复算一遍)。Task 3 的 `count_git_commits_counts_a_merged_commit_once`、`recent_churn_ignores_merge_commits`;bytegit Task 1 的 `commit_count_counts_a_merged_commit_once`、`churn_skips_merge_commits`。
5. **近期窗口与路径细节**:恰好等于 `since` 的提交算近期;重命名只计新路径、删除计旧路径、根提交的文件要计;非 ASCII 文件名(迁移前 git 输出转义过的八进制,churn 对这类文件恒为 0,迁移后是真实 UTF-8,**有意的改进**);HEAD 提交本身过旧而更深处有近期提交时(提交时间戳不单调)迁移前 `git log --since` 会在 HEAD 处就停止,见 D5。bytegit Task 1 的 `churn_*`;Task 3 的 `recent_churn_*`;Task 6 翻转的两条测试。

## 待决事项(本计划按下述处理,**需要你知情;D5、D6 可以要求改**)

- **D5:`recent_churn` 在"HEAD 提交的提交者日期早于 30 天、而更深处的提交反而很新"时,结果比旧实现多。** 旧实现(`git log --since`)从 HEAD 往下遇到第一个过旧的提交就停止遍历,更深处的近期提交不计;`bytegit::churn` 对提交时间做全局排序,所以会把它们算上(更符合"近 30 天"的字面含义,也不依赖遍历顺序)。真实仓库里这种时间戳不单调很少见(rebase/cherry-pick 保留旧提交日期时可能出现)。Task 3 把旧行为固定为测试,Task 6 翻转。如果希望逐字保持旧行为,需要在 `churn` 里模仿 git 的"遇到第一个过旧提交就停"并放弃全局排序——请告知。
- **D6:`churn` 在超大仓库上比命令行 `git log --since` 慢。** libgit2 对"按时间排序"的遍历会先解析整段可达历史(成本与提交总数成正比),命令行靠提交图能在窗口边界提前结束。迁移前 `usage` 的两个 revwalk 本来就扫全历史,`churn` 的逐提交 diff 只做近 30 天的那些,所以只是把一次命令行进程换成 O(提交数) 的解析;规格 §8 O5 已声明"保持现有上限、不在迁移中优化"。Code Health 扫描在几十万提交的仓库上可能多出秒级耗时;若不可接受,另立项(提交图 / `since` 下推)。
- **D7:`dozerd` 第一次依赖 `bytegit`,同时带进 libgit2(`libgit2-sys` 自带 C 源码,编译时间与二进制体积都会增加)。** 这是规格 B6 已裁决的方向("`dozerd` 也依赖"),此处只是提醒;`dozerd` 会作为 App bundle 的一部分分发,体积变化请在合并前看一眼 `target/release/dozerd` 的大小。

另有一条**有意的改进**不需要裁决:非 ASCII 文件名在 `recent_churn` 里从"git 转义过的八进制(永远匹配不上)"变成真实 UTF-8(同 P1 的 `dirty_paths`)。

规格补充(Task 7 回写):规格 §4.4 的 `LogOptions { since }` **不实现**——P3 的三个使用者要的是 `churn(since)`、计数与按日分桶,没有 `log` + `since` 的使用者(YAGNI;结构体已 `non_exhaustive`,需要时再加);`churn` 返回路径相对仓库根;所有"遍历 HEAD 历史"的查询在 HEAD 未诞生时统一返回 `GitErrorKind::NoCommits`,只有 `head_commit_time`(返回 `Option`)返回 `Ok(None)`。

---

## File Structure

**bytegit 仓库**(`~/Projects/CoralProjects/byteboy/bytegit/`):

| 文件 | 变更 |
|------|------|
| `Cargo.toml` | 版本 `0.3.0` → `0.4.0` |
| `src/stats.rs` | 新增:`Repo::{head_commit_time, commit_count, commit_count_by_day, churn}` |
| `src/history.rs` | 抽出 `Repo::head_walk`(`log` 改用它);`system_time`、`path_of` 改为 `pub(crate)` |
| `src/testutil.rs` | 新增 `commit_file_at`、`commit_staged_at`、`merge_commit_at`;`commit_staged`/`merge_commit` 改为共用内部实现 |
| `src/lib.rs` | 声明 `mod stats;` |

**dozer 仓库**(专用 worktree `~/Projects/CoralProjects/byteboy/dozer-bytegit-p3/`,分支 `bytegit-p3`):

| 文件 | 变更 |
|------|------|
| `crates/dozer-app/Cargo.toml`、`crates/dozerd/Cargo.toml`、`Cargo.lock` | `dozer-app` 的 `bytegit` tag 升到 `v0.4.0`;`dozerd` 新增 `bytegit` 依赖 |
| `crates/dozerd/src/projects.rs` | `git_repo_root`/`git_head_commit_ms` 两个命令行函数换成一个 bytegit 实现;去掉 `std::process::Command` |
| `crates/dozer-app/src/extensions/usage/mod.rs` | `count_git_commits`、`count_git_commits_by_day` 改用 bytegit |
| `crates/dozer-app/src/extensions/codehealth/git_hotspots.rs` | `recent_churn` 改用 bytegit + 路径适配;去掉 `Command` 的生产用法 |
| 规格、`CLAUDE.md`、要求文档、盘点文档 | Task 7 同步 |

---

### Task 1: bytegit — 提交统计与近期改动

**Files:**
- Create: `src/stats.rs`
- Modify: `src/history.rs`、`src/testutil.rs`、`src/lib.rs`

**Interfaces:**
- Consumes(P1/P2 已有):`Repo::raw()`、`CommitId::from_oid`、`GitError`/`GitErrorKind`、`history::{system_time, path_of}`、`TempRepo::{commit_file, commit_staged, merge_commit, stage, stage_remove, write_untracked, detach_head, branch, checkout}`
- Produces(Task 2、4、5、6 依赖):
  - `Repo::head_commit_time(&self) -> Result<Option<SystemTime>, GitError>`(提交者时间;没有提交为 `Ok(None)`)
  - `Repo::commit_count(&self) -> Result<u64, GitError>`
  - `Repo::commit_count_by_day(&self) -> Result<BTreeMap<i64, u64>, GitError>`(日索引 = 提交者时间秒 / 86400)
  - `Repo::churn(&self, since: SystemTime) -> Result<HashMap<PathBuf, u32>, GitError>`(路径相对仓库根)
  - 以上三个遍历类方法在 HEAD 未诞生时返回 `GitErrorKind::NoCommits`
  - `TempRepo::{commit_file_at(&self, rel: &str, content: &str, message: &str, unix_secs: i64) -> CommitId, commit_staged_at(&self, message: &str, unix_secs: i64) -> CommitId, merge_commit_at(&self, other: &str, message: &str, unix_secs: i64) -> CommitId}`

- [ ] **Step 1: 夹具——按指定时间提交、按指定时间合并**

在 `src/testutil.rs` 里,把从 `/// 把当前暂存区原样提交到当前分支` 开始、到 `/// 在当前 HEAD 创建分支(不切换)。` 之前的全部方法(P2 里的 `commit_staged` 与 `merge_commit`)整体替换为下面的内容。`commit_staged`/`merge_commit` 的行为不变,只是改为共用内部实现;P0–P2 已有的夹具与面板测试必须仍然全部通过:

```rust
    /// 把当前暂存区原样提交到当前分支(配合 `stage`/`stage_remove` 做一次含多个改动的提交)。
    pub fn commit_staged(&self, message: &str) -> CommitId {
        self.commit_staged_with(message, self.signature())
    }

    /// 同 [`TempRepo::commit_file`],但提交时间(作者与提交者相同)固定为 `unix_secs`,
    /// 用来构造"很久以前的提交""跨天的提交"。
    pub fn commit_file_at(
        &self,
        rel: &str,
        content: &str,
        message: &str,
        unix_secs: i64,
    ) -> CommitId {
        self.write_untracked(rel, content).stage(rel);
        self.commit_staged_at(message, unix_secs)
    }

    /// 同 [`TempRepo::commit_staged`],提交时间固定为 `unix_secs`。
    pub fn commit_staged_at(&self, message: &str, unix_secs: i64) -> CommitId {
        let sig = git2::Signature::new("Test", "test@example.com", &git2::Time::new(unix_secs, 0))
            .expect("构造签名失败");
        self.commit_staged_with(message, sig)
    }

    fn commit_staged_with(&self, message: &str, sig: git2::Signature<'static>) -> CommitId {
        let mut index = self.repo.index().expect("读取 index 失败");
        let tree = self
            .repo
            .find_tree(index.write_tree().expect("写 tree 失败"))
            .expect("找不到 tree");
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
        self.merge_commit_with(other, message, self.signature())
    }

    /// 同 [`TempRepo::merge_commit`],提交时间固定为 `unix_secs`。
    pub fn merge_commit_at(&self, other: &str, message: &str, unix_secs: i64) -> CommitId {
        let sig = git2::Signature::new("Test", "test@example.com", &git2::Time::new(unix_secs, 0))
            .expect("构造签名失败");
        self.merge_commit_with(other, message, sig)
    }

    fn merge_commit_with(
        &self,
        other: &str,
        message: &str,
        sig: git2::Signature<'static>,
    ) -> CommitId {
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

- [ ] **Step 2: `history.rs`——抽出 `head_walk`,放宽两个辅助函数的可见性**

在 `src/history.rs`:

1. `fn system_time(secs: i64) -> SystemTime {` 改为 `pub(crate) fn system_time(secs: i64) -> SystemTime {`;`fn path_of(delta: &git2::DiffDelta<'_>) -> Option<PathBuf> {` 改为 `pub(crate) fn path_of(delta: &git2::DiffDelta<'_>) -> Option<PathBuf> {`。
2. 在 `impl Repo` 里 `log` 的文档注释之前加入 `head_walk`:

```rust
    /// 从 HEAD 起的提交遍历(按 `sort` 排序)。HEAD 指向未诞生的分支(还没有提交)时返回
    /// `GitErrorKind::NoCommits`——libgit2 对这种情况报的是"引用不存在",这里换成明确的分类。
    pub(crate) fn head_walk(&self, sort: git2::Sort) -> Result<git2::Revwalk<'_>, GitError> {
        let repo = self.raw();
        let mut revwalk = repo.revwalk()?;
        if let Err(e) = revwalk.push_head() {
            return Err(match repo.head() {
                Err(h) if h.code() == git2::ErrorCode::UnbornBranch => {
                    GitError::new(GitErrorKind::NoCommits, "仓库还没有任何提交")
                }
                _ => e.into(),
            });
        }
        revwalk.set_sorting(sort)?;
        Ok(revwalk)
    }
```

3. 把 `log` 函数体开头这一段(从 `let repo = self.raw();` 到 `revwalk.set_sorting(git2::Sort::TIME)?;`):

```rust
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
```

   替换为:

```rust
        let repo = self.raw();
        let revwalk = self.head_walk(git2::Sort::TIME)?;
```

   `log` 其余部分不动。`history::` 的 23 个测试必须仍然全部通过(尤其是 `log_of_an_empty_repo_is_a_no_commits_error`)。

- [ ] **Step 3: 写 `src/stats.rs`(实现 + 测试)**

```rust
//! 历史统计:HEAD 提交时间、提交计数、按日分桶、近期文件改动频率。

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::history::{path_of, system_time};
use crate::{GitError, Repo};

const SECS_PER_DAY: i64 = 86_400;

fn unix_secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

impl Repo {
    /// HEAD 提交的提交时间(committer time,等于 `git log -1 --format=%ct`)。
    /// 仓库还没有提交返回 `None`。
    pub fn head_commit_time(&self) -> Result<Option<SystemTime>, GitError> {
        let head = match self.raw().head() {
            Ok(head) => head,
            Err(e) if e.code() == git2::ErrorCode::UnbornBranch => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let commit = head.peel_to_commit()?;
        Ok(Some(system_time(commit.time().seconds())))
    }

    /// 从 HEAD 可达的提交总数(当前分支口径,合并进来的提交只算一次)。
    /// HEAD 未诞生返回 `GitErrorKind::NoCommits`。
    pub fn commit_count(&self) -> Result<u64, GitError> {
        let mut count = 0u64;
        for oid in self.head_walk(git2::Sort::NONE)? {
            oid?;
            count += 1;
        }
        Ok(count)
    }

    /// 从 HEAD 可达的提交按**提交时间**(committer time)的 UTC 日分桶计数:
    /// 日索引 = `提交时间秒 / 86_400`(整数除法,与迁移前 `usage` 同口径)。只含"有提交的那些天"。任何一个提交读取失败整体报错;
    /// HEAD 未诞生返回 `GitErrorKind::NoCommits`。
    pub fn commit_count_by_day(&self) -> Result<BTreeMap<i64, u64>, GitError> {
        let repo = self.raw();
        let mut by_day: BTreeMap<i64, u64> = BTreeMap::new();
        for oid in self.head_walk(git2::Sort::NONE)? {
            let commit = repo.find_commit(oid?)?;
            *by_day
                .entry(commit.time().seconds() / SECS_PER_DAY)
                .or_insert(0) += 1;
        }
        Ok(by_day)
    }

    /// 提交时间不早于 `since` 的提交里,每个文件被改动的提交数(路径相对**仓库根**)。
    ///
    /// 口径与 `git log --since=<since> --name-only` 一致:
    /// - 合并提交不计(`git log --name-only` 不列合并提交的文件);根提交相对空树;
    /// - 做重命名检测(libgit2 默认参数,与 git 默认一致),重命名只计新路径;删除计旧路径;
    /// - 按提交时间倒序遍历,遇到第一个早于 `since` 的提交即停止:逐提交做 diff 的只有近期那些。
    ///   (libgit2 对"按时间排序"的遍历会先解析整段可达历史再产出,所以排序是全局的、
    ///   提前停止不会漏掉因时钟偏差而时间戳偏新的更早提交;代价是解析成本与历史长度成正比,
    ///   与 `git log --since` 借助提交图提前结束不同,超大仓库上会更慢。)
    ///
    /// HEAD 未诞生返回 `GitErrorKind::NoCommits`。
    pub fn churn(&self, since: SystemTime) -> Result<HashMap<PathBuf, u32>, GitError> {
        let repo = self.raw();
        let since_secs = unix_secs(since);
        let mut counts: HashMap<PathBuf, u32> = HashMap::new();
        for oid in self.head_walk(git2::Sort::TIME)? {
            let commit = repo.find_commit(oid?)?;
            if commit.time().seconds() < since_secs {
                break;
            }
            if commit.parent_count() > 1 {
                continue;
            }
            let new_tree = commit.tree()?;
            let old_tree = match commit.parent(0) {
                Ok(parent) => Some(parent.tree()?),
                Err(_) => None,
            };
            let mut diff = repo.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), None)?;
            diff.find_similar(None)?;
            for delta in diff.deltas() {
                if let Some(path) = path_of(&delta) {
                    *counts.entry(path).or_insert(0) += 1;
                }
            }
        }
        Ok(counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GitErrorKind;
    use crate::testutil::TempRepo;
    use std::time::Duration;

    /// 2024-10-04 00:00:00 UTC 附近的整日起点(日索引 20000)。
    const DAY_20000: i64 = 20_000 * SECS_PER_DAY;

    fn at(secs: i64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs as u64)
    }

    fn churn_of(t: &TempRepo, since: i64) -> HashMap<PathBuf, u32> {
        t.open().churn(at(since)).unwrap()
    }

    fn count_of(m: &HashMap<PathBuf, u32>, path: &str) -> Option<u32> {
        m.get(&PathBuf::from(path)).copied()
    }

    // ---- head_commit_time ----

    #[test]
    fn head_commit_time_is_none_for_an_empty_repo() {
        assert_eq!(TempRepo::new().open().head_commit_time().unwrap(), None);
    }

    #[test]
    fn head_commit_time_is_the_head_commit_time_not_the_newest_in_history() {
        let t = TempRepo::new();
        t.commit_file_at("a.txt", "1", "newer", DAY_20000 + 5_000);
        // HEAD 提交的时间比它的父提交更早(时钟偏差):取 HEAD 的,不是历史里最大的。
        t.commit_file_at("a.txt", "2", "head", DAY_20000);
        assert_eq!(t.open().head_commit_time().unwrap(), Some(at(DAY_20000)));
    }

    #[test]
    fn head_commit_time_works_on_a_detached_head() {
        let t = TempRepo::new();
        t.commit_file_at("a.txt", "1", "one", DAY_20000);
        t.detach_head();
        assert_eq!(t.open().head_commit_time().unwrap(), Some(at(DAY_20000)));
    }

    // ---- commit_count ----

    #[test]
    fn commit_count_of_an_empty_repo_is_a_no_commits_error() {
        let err = TempRepo::new().open().commit_count().unwrap_err();
        assert_eq!(err.kind(), GitErrorKind::NoCommits);
    }

    #[test]
    fn commit_count_counts_commits_reachable_from_head() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "1", "one");
        t.commit_file("a.txt", "2", "two");
        t.commit_file("a.txt", "3", "three");
        assert_eq!(t.open().commit_count().unwrap(), 3);
    }

    #[test]
    fn commit_count_counts_a_merged_commit_once() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a", "base");
        t.branch("other").checkout("other");
        t.commit_file("b.txt", "b", "other adds b");
        t.checkout("main");
        t.commit_file("c.txt", "c", "main adds c");
        t.merge_commit("other", "merge");
        // base + other + main + merge
        assert_eq!(t.open().commit_count().unwrap(), 4);
    }

    #[test]
    fn commit_count_only_counts_the_current_branch() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "a", "base");
        t.branch("other").checkout("other");
        t.commit_file("b.txt", "b", "only on other");
        t.checkout("main");
        assert_eq!(t.open().commit_count().unwrap(), 1);
    }

    #[test]
    fn commit_count_works_on_a_detached_head() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "1", "one");
        t.commit_file("a.txt", "2", "two");
        t.detach_head();
        assert_eq!(t.open().commit_count().unwrap(), 2);
    }

    // ---- commit_count_by_day ----

    #[test]
    fn commit_count_by_day_buckets_by_utc_commit_day() {
        let t = TempRepo::new();
        t.commit_file_at("a.txt", "1", "d0 start", DAY_20000);
        t.commit_file_at("a.txt", "2", "d0 end", DAY_20000 + SECS_PER_DAY - 1);
        t.commit_file_at("a.txt", "3", "d1", DAY_20000 + SECS_PER_DAY);
        let by_day = t.open().commit_count_by_day().unwrap();
        assert_eq!(by_day, BTreeMap::from([(20_000, 2), (20_001, 1)]));
    }

    #[test]
    fn commit_count_by_day_of_an_empty_repo_is_a_no_commits_error() {
        let err = TempRepo::new().open().commit_count_by_day().unwrap_err();
        assert_eq!(err.kind(), GitErrorKind::NoCommits);
    }

    // ---- churn ----

    #[test]
    fn churn_counts_the_commits_touching_each_file() {
        let t = TempRepo::new();
        t.commit_file_at("hot.rs", "1", "c1", DAY_20000);
        t.commit_file_at("hot.rs", "2", "c2", DAY_20000 + 10);
        t.commit_file_at("cold.rs", "x", "c3", DAY_20000 + 20);
        let churn = churn_of(&t, DAY_20000);
        assert_eq!(count_of(&churn, "hot.rs"), Some(2));
        assert_eq!(count_of(&churn, "cold.rs"), Some(1));
        assert_eq!(churn.len(), 2);
    }

    #[test]
    fn churn_includes_the_root_commit_files() {
        let t = TempRepo::new();
        t.write_untracked("a.rs", "1").stage("a.rs");
        t.write_untracked("sub/b.rs", "2").stage("sub/b.rs");
        t.commit_staged_at("root", DAY_20000);
        let churn = churn_of(&t, DAY_20000);
        assert_eq!(count_of(&churn, "a.rs"), Some(1));
        assert_eq!(count_of(&churn, "sub/b.rs"), Some(1), "路径相对仓库根");
    }

    #[test]
    fn churn_excludes_commits_older_than_since_and_includes_the_boundary() {
        let t = TempRepo::new();
        t.commit_file_at("a.rs", "1", "old", DAY_20000 - 1);
        t.commit_file_at("a.rs", "2", "on the boundary", DAY_20000);
        t.commit_file_at("a.rs", "3", "newer", DAY_20000 + 1);
        let churn = churn_of(&t, DAY_20000);
        assert_eq!(count_of(&churn, "a.rs"), Some(2));
    }

    #[test]
    fn churn_of_a_repo_whose_commits_are_all_too_old_is_empty_not_an_error() {
        let t = TempRepo::new();
        t.commit_file_at("a.rs", "1", "old", DAY_20000);
        assert!(churn_of(&t, DAY_20000 + 1).is_empty());
    }

    #[test]
    fn churn_skips_merge_commits() {
        let t = TempRepo::new();
        t.commit_file_at("a.rs", "a", "base", DAY_20000);
        t.branch("other").checkout("other");
        t.commit_file_at("b.rs", "b", "other adds b", DAY_20000 + 10);
        t.checkout("main");
        t.commit_file_at("c.rs", "c", "main adds c", DAY_20000 + 20);
        // 合并提交相对第一父会"新增 b.rs",但合并提交本身不计。
        t.merge_commit_at("other", "merge", DAY_20000 + 30);
        let churn = churn_of(&t, DAY_20000);
        assert_eq!(count_of(&churn, "a.rs"), Some(1));
        assert_eq!(count_of(&churn, "b.rs"), Some(1), "只算 other 上那一次");
        assert_eq!(count_of(&churn, "c.rs"), Some(1));
    }

    #[test]
    fn churn_counts_a_pure_rename_for_the_new_path_only() {
        let t = TempRepo::new();
        t.commit_file_at("old.rs", "same content\nline 2\nline 3\n", "add", DAY_20000);
        t.stage_remove("old.rs");
        t.write_untracked("new.rs", "same content\nline 2\nline 3\n")
            .stage("new.rs");
        t.commit_staged_at("rename", DAY_20000 + 10);
        let churn = churn_of(&t, DAY_20000);
        assert_eq!(count_of(&churn, "new.rs"), Some(1));
        assert_eq!(count_of(&churn, "old.rs"), Some(1), "只有最初添加那一次");
    }

    #[test]
    fn churn_counts_the_old_path_of_a_deletion() {
        let t = TempRepo::new();
        t.commit_file_at("gone.rs", "1", "add", DAY_20000);
        t.stage_remove("gone.rs");
        t.commit_staged_at("delete", DAY_20000 + 10);
        assert_eq!(count_of(&churn_of(&t, DAY_20000), "gone.rs"), Some(2));
    }

    #[test]
    fn churn_keeps_non_ascii_paths_verbatim() {
        let t = TempRepo::new();
        t.commit_file_at("文档/说明.md", "x", "cn", DAY_20000);
        assert_eq!(count_of(&churn_of(&t, DAY_20000), "文档/说明.md"), Some(1));
    }

    #[test]
    fn churn_of_an_empty_repo_is_a_no_commits_error() {
        let err = TempRepo::new().open().churn(at(0)).unwrap_err();
        assert_eq!(err.kind(), GitErrorKind::NoCommits);
    }

    /// 历史(从老到新):`skew.rs`(时钟偏差,时间戳反而新)→ 几个很老的提交 → `new.rs`(新)。
    /// 遍历是全局按时间排序的,所以提前停止不会漏掉"时间戳偏新"的更早提交。
    #[test]
    fn churn_counts_every_recent_commit_even_with_clock_skew() {
        let t = TempRepo::new();
        t.commit_file_at("skew.rs", "x", "skewed but recent", DAY_20000 + 100);
        for i in 0..8 {
            t.commit_file_at(&format!("old{i}.rs"), "x", "old", DAY_20000 - 1000 + i);
        }
        t.commit_file_at("new.rs", "x", "new", DAY_20000 + 5);
        let churn = churn_of(&t, DAY_20000);
        assert_eq!(count_of(&churn, "new.rs"), Some(1));
        assert_eq!(count_of(&churn, "skew.rs"), Some(1));
        assert_eq!(churn.len(), 2, "老提交里的文件不计");
    }
}
```

- [ ] **Step 4: 在 `src/lib.rs` 声明**

在 `mod repo;` 之后、`mod status;` 之前加一行 `mod stats;`。完成后 `src/lib.rs` 为:

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
mod stats;
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

- [ ] **Step 5: 运行测试与门禁**

Run: `cargo test stats:: && cargo test history:: && cargo test`
Expected: `stats::` 20 个、`history::` 23 个通过;全量 104 个通过。

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings`
Expected: 干净。

- [ ] **Step 6: 变异检验(确认 Review Focus 的测试真的会失败)**

逐个做、逐个还原(`git checkout src/stats.rs`):

1. 删掉 `churn` 里的 `if commit.parent_count() > 1 { continue; }` 三行。Expected: `churn_skips_merge_commits` 失败。
2. 把 `commit.time().seconds() < since_secs` 改成 `<=`。Expected: `churn_excludes_commits_older_than_since_and_includes_the_boundary` 等 7 个 `churn_*` 测试失败。
3. 删掉 `diff.find_similar(None)?;`。Expected: `churn_counts_a_pure_rename_for_the_new_path_only` 失败。
4. 把 `commit_count_by_day` 里的 `/ SECS_PER_DAY` 改成 `/ 3600`。Expected: `commit_count_by_day_buckets_by_utc_commit_day` 失败。
5. 在 `head_commit_time` 里 `let commit = head.peel_to_commit()?;` 之后加一行 `let commit = commit.parent(0).unwrap_or(commit);`。Expected: `head_commit_time_is_the_head_commit_time_not_the_newest_in_history` 失败。
6. 把 `commit_count` 的返回值改成 `Ok(count.saturating_sub(1))`。Expected: `commit_count_*` 的 4 个测试失败。

- [ ] **Step 7: Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/bytegit
git status --short
git add src/stats.rs src/history.rs src/testutil.rs src/lib.rs
git diff --cached --stat
git commit -m "feat: add head_commit_time, commit_count, commit_count_by_day and churn

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: bytegit — 发布 v0.4.0

**Files:**
- Modify: `Cargo.toml`(`version = "0.4.0"`)

**Interfaces:** Produces:git tag `v0.4.0`(Task 4、5、6 的 dozer 依赖)。

- [ ] **Step 1: 升版本并全量门禁**

把 `Cargo.toml` 的 `version = "0.3.0"` 改为 `"0.4.0"`。

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings && cargo test && cargo test --all-features`
Expected: 全部通过,104 个测试。

- [ ] **Step 2: Commit 并推送 main(对外可见操作:先向用户确认)**

向用户确认"现在推送 `main` 并打 tag `v0.4.0`"。获得明确同意后:

```bash
cd ~/Projects/CoralProjects/byteboy/bytegit
git add Cargo.toml
git commit -m "chore: release 0.4.0

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git push origin main
gh run watch "$(gh run list --repo byteboyai/bytegit --limit 1 --json databaseId -q '.[0].databaseId')" --repo byteboyai/bytegit --exit-status
```

Expected: CI(macOS)通过。**CI 红则先修,不打 tag。**

- [ ] **Step 3: 打 tag 并验证可被消费**

```bash
git tag -a v0.4.0 -m "bytegit v0.4.0: head_commit_time, commit_count, commit_count_by_day, churn"
git push origin v0.4.0
cd "$(mktemp -d)" && cargo new --lib consume -q && cd consume
cargo add bytegit --git https://github.com/byteboyai/bytegit --tag v0.4.0 --features testutil
cat >> src/lib.rs <<'EOF'

#[cfg(test)]
mod t {
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn consumes_bytegit() {
        let t = bytegit::testutil::TempRepo::new();
        t.commit_file_at("a.txt", "x\n", "one", 1_728_000_000);
        let repo = t.open();
        assert_eq!(repo.commit_count().unwrap(), 1);
        assert_eq!(
            repo.head_commit_time().unwrap(),
            Some(UNIX_EPOCH + Duration::from_secs(1_728_000_000))
        );
        let churn = repo.churn(UNIX_EPOCH).unwrap();
        assert_eq!(churn.get(std::path::Path::new("a.txt")), Some(&1));
        assert_eq!(repo.commit_count_by_day().unwrap().len(), 1);
    }
}
EOF
cargo test
```

Expected: `consumes_bytegit ... ok`。

---

### Task 3: dozer — 在旧实现上补刻画测试

**Files:**
- Modify(worktree 内,只改各文件的 `tests` 模块): `crates/dozerd/src/projects.rs`、`crates/dozer-app/src/extensions/usage/mod.rs`、`crates/dozer-app/src/extensions/codehealth/git_hotspots.rs`

**Interfaces:** 无新接口。产出 26 个刻画测试(`dozerd` 9 个、`usage` 7 个、`git_hotspots` 10 个),在**旧实现**上全部通过;Task 4–6 保持它们的断言不变,仅 Task 6 翻转其中两条。

- [ ] **Step 1: 建专用 worktree 与分支**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-bytegit-p3 -b bytegit-p3 bytegit-p2
mkdir -p ../dozer-bytegit-p3/.cargo && cp .cargo/config.toml ../dozer-bytegit-p3/.cargo/config.toml
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p3 && git log --oneline | head -1 && git status --short
```

Expected: 干净、分支 `bytegit-p3`,HEAD 与 `bytegit-p2` 一致。若 `bytegit-p1`/`bytegit-p2` 已合并进 `main`,把命令里的基点 `bytegit-p2` 换成 `main`。`.cargo/config.toml`(不提交)里的 `[patch."https://github.com/byteboyai/bytegit"]` 指向本地 `../bytegit`,此时它已含 Task 1–2 的改动。此后**所有命令都用 `~/Projects/CoralProjects/byteboy/dozer-bytegit-p3/` 前缀**。

- [ ] **Step 2: `dozerd/projects.rs` 的 `tests` 模块加 9 个刻画测试**

在 `crates/dozerd/src/projects.rs` 的 `mod tests` 末尾(最后一个测试之后、模块结尾 `}` 之前)追加。测试用命令行 `git` 造仓库(固定作者与**提交者**日期、关闭用户的全局配置),`Command` 在该模块里已经是被 `use super::*` 引入的:

```rust
    // ---- bytegit P3:compute_updated_ms 迁移前后必须一致的口径 ----

    const T: i64 = 20_000 * 86_400;

    fn git_at(dir: &Path, args: &[&str], when: Option<i64>) {
        let mut cmd = Command::new("git");
        cmd.args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            // 作者日期固定为很早以前:更新时间取的是**提交者**日期。
            .env("GIT_AUTHOR_DATE", "946684800 +0000");
        if let Some(w) = when {
            cmd.env("GIT_COMMITTER_DATE", format!("{w} +0000"));
        }
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    fn commit_at(dir: &Path, rel: &str, content: &str, when: i64) {
        let full = dir.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
        git_at(dir, &["add", "."], None);
        git_at(dir, &["commit", "-qm", "c"], Some(when));
    }

    fn updated(dir: &Path, last_active_ms: u64) -> u64 {
        compute_updated_ms(dir.to_str().unwrap(), last_active_ms)
    }

    #[test]
    fn updated_ms_is_last_active_outside_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(updated(dir.path(), 5_000), 5_000);
    }

    #[test]
    fn updated_ms_is_last_active_for_a_repo_without_commits() {
        let dir = tempfile::tempdir().unwrap();
        git_at(dir.path(), &["init", "-q"], None);
        assert_eq!(updated(dir.path(), 5_000), 5_000);
    }

    #[test]
    fn updated_ms_is_the_head_commit_time_when_it_is_not_older_than_last_active() {
        let dir = tempfile::tempdir().unwrap();
        git_at(dir.path(), &["init", "-q"], None);
        commit_at(dir.path(), "a.txt", "1", T + 123);
        assert_eq!(updated(dir.path(), 1_000), (T as u64 + 123) * 1000);
        // 恰好相等也取提交时间(`>=`)。
        assert_eq!(
            updated(dir.path(), (T as u64 + 123) * 1000),
            (T as u64 + 123) * 1000
        );
    }

    #[test]
    fn updated_ms_falls_back_to_last_active_when_the_commit_is_older() {
        let dir = tempfile::tempdir().unwrap();
        git_at(dir.path(), &["init", "-q"], None);
        commit_at(dir.path(), "a.txt", "1", T);
        let later = (T as u64 + 10) * 1000;
        assert_eq!(updated(dir.path(), later), later);
    }

    #[test]
    fn updated_ms_uses_the_head_commit_even_when_an_ancestor_has_a_newer_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        git_at(dir.path(), &["init", "-q"], None);
        commit_at(dir.path(), "a.txt", "1", T + 5_000);
        commit_at(dir.path(), "a.txt", "2", T);
        assert_eq!(updated(dir.path(), 1_000), (T as u64) * 1000);
    }

    #[test]
    fn updated_ms_looks_upward_from_a_repo_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        git_at(dir.path(), &["init", "-q"], None);
        commit_at(dir.path(), "sub/a.txt", "1", T + 7);
        assert_eq!(
            updated(&dir.path().join("sub"), 1_000),
            (T as u64 + 7) * 1000
        );
    }

    #[test]
    fn updated_ms_works_on_a_detached_head() {
        let dir = tempfile::tempdir().unwrap();
        git_at(dir.path(), &["init", "-q"], None);
        commit_at(dir.path(), "a.txt", "1", T + 9);
        git_at(dir.path(), &["checkout", "-q", "--detach"], None);
        assert_eq!(updated(dir.path(), 1_000), (T as u64 + 9) * 1000);
    }

    #[test]
    fn updated_ms_ignores_a_bare_repo_because_it_has_no_work_tree() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir(&src).unwrap();
        git_at(&src, &["init", "-q"], None);
        commit_at(&src, "a.txt", "1", T + 11);
        let bare = dir.path().join("bare.git");
        git_at(
            dir.path(),
            &[
                "clone",
                "-q",
                "--bare",
                src.to_str().unwrap(),
                bare.to_str().unwrap(),
            ],
            None,
        );
        assert_eq!(updated(&bare, 1_000), 1_000);
    }

    #[test]
    fn updated_ms_of_a_linked_worktree_follows_that_worktrees_head() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main");
        std::fs::create_dir(&main).unwrap();
        git_at(&main, &["init", "-q"], None);
        commit_at(&main, "a.txt", "1", T + 100);
        let wt = dir.path().join("wt");
        git_at(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                wt.to_str().unwrap(),
            ],
            None,
        );
        commit_at(&wt, "b.txt", "2", T + 200);
        assert_eq!(updated(&main, 1_000), (T as u64 + 100) * 1000);
        assert_eq!(updated(&wt, 1_000), (T as u64 + 200) * 1000);
    }
```

- [ ] **Step 3: `usage/mod.rs` 的 `tests` 模块加 7 个刻画测试**

在 `crates/dozer-app/src/extensions/usage/mod.rs` 的 `mod tests` 末尾追加:

```rust
    // ---- bytegit P3:git 提交计数迁移前后必须一致的口径 ----

    const DAY_20000: i64 = 20_000 * 86_400;

    /// 在 `repo` 里跑一条 git 命令。作者日期固定为 2000-01-01(证明统计用的是**提交者**时间,
    /// 不是作者时间);`when` 是提交者日期(unix 秒),`None` 用当前时间。
    fn git_at(repo: &std::path::Path, args: &[&str], when: Option<i64>) {
        let mut cmd = std::process::Command::new("git");
        cmd.args(args)
            .current_dir(repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .env("GIT_AUTHOR_DATE", "946684800 +0000");
        if let Some(w) = when {
            cmd.env("GIT_COMMITTER_DATE", format!("{w} +0000"));
        }
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    fn commit_at(repo: &std::path::Path, rel: &str, content: &str, when: Option<i64>) {
        let full = repo.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
        git_at(repo, &["add", "."], None);
        git_at(repo, &["commit", "-qm", "c"], when);
    }

    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git_at(dir.path(), &["init", "-q"], None);
        dir
    }

    #[test]
    fn git_commit_counts_are_zero_and_empty_outside_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(count_git_commits(dir.path()), 0);
        assert!(count_git_commits_by_day(dir.path()).is_empty());
    }

    #[test]
    fn git_commit_counts_are_zero_and_empty_for_a_repo_without_commits() {
        let dir = init_repo();
        assert_eq!(count_git_commits(dir.path()), 0);
        assert!(count_git_commits_by_day(dir.path()).is_empty());
    }

    #[test]
    fn count_git_commits_counts_commits_reachable_from_head() {
        let dir = init_repo();
        for i in 0..3 {
            commit_at(dir.path(), "a.txt", &i.to_string(), None);
        }
        assert_eq!(count_git_commits(dir.path()), 3);
    }

    #[test]
    fn git_commit_counts_look_upward_from_a_repo_subdirectory() {
        let dir = init_repo();
        commit_at(dir.path(), "sub/a.txt", "1", Some(DAY_20000));
        commit_at(dir.path(), "sub/a.txt", "2", Some(DAY_20000 + 1));
        let sub = dir.path().join("sub");
        assert_eq!(count_git_commits(&sub), 2);
        assert_eq!(
            count_git_commits_by_day(&sub),
            BTreeMap::from([(20_000, 2)])
        );
    }

    #[test]
    fn count_git_commits_counts_a_merged_commit_once() {
        let dir = init_repo();
        let r = dir.path();
        commit_at(r, "a.txt", "a", None);
        git_at(r, &["checkout", "-q", "-b", "other"], None);
        commit_at(r, "b.txt", "b", None);
        git_at(r, &["checkout", "-q", "-"], None);
        commit_at(r, "c.txt", "c", None);
        git_at(r, &["merge", "-q", "--no-ff", "other", "-m", "merge"], None);
        // base + other + main + merge
        assert_eq!(count_git_commits(r), 4);
    }

    #[test]
    fn git_commit_counts_work_on_a_detached_head() {
        let dir = init_repo();
        commit_at(dir.path(), "a.txt", "1", None);
        commit_at(dir.path(), "a.txt", "2", None);
        git_at(dir.path(), &["checkout", "-q", "--detach"], None);
        assert_eq!(count_git_commits(dir.path()), 2);
        assert_eq!(
            count_git_commits_by_day(dir.path()).values().sum::<u64>(),
            2
        );
    }

    #[test]
    fn git_commits_by_day_buckets_by_utc_committer_day_not_author_day() {
        let dir = init_repo();
        commit_at(dir.path(), "a.txt", "1", Some(DAY_20000));
        commit_at(dir.path(), "a.txt", "2", Some(DAY_20000 + 86_399));
        commit_at(dir.path(), "a.txt", "3", Some(DAY_20000 + 86_400));
        // 作者日期都是 2000-01-01,桶却按提交者日期落在 20000/20001 天。
        assert_eq!(
            count_git_commits_by_day(dir.path()),
            BTreeMap::from([(20_000, 2), (20_001, 1)])
        );
    }
```

- [ ] **Step 4: `git_hotspots.rs` 的 `tests` 模块加 10 个刻画测试**

在 `crates/dozer-app/src/extensions/codehealth/git_hotspots.rs` 的 `mod tests` 末尾追加(该模块已有 `use std::process::Command;` 来自 `use super::*`,生产代码还在用它):

```rust
    // ---- bytegit P3:recent_churn 迁移前后必须一致的口径 ----

    fn churn_git(repo: &Path, args: &[&str], when: Option<i64>) {
        let mut cmd = Command::new("git");
        cmd.args(args)
            .current_dir(repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            // 作者日期固定为 2000-01-01:`--since` 看的是**提交者**日期,作者日期再老也不影响。
            .env("GIT_AUTHOR_DATE", "946684800 +0000");
        if let Some(w) = when {
            cmd.env("GIT_COMMITTER_DATE", format!("{w} +0000"));
        }
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    fn churn_commit(repo: &Path, rel: &str, content: &str, when: Option<i64>) {
        let full = repo.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
        churn_git(repo, &["add", "."], None);
        churn_git(repo, &["commit", "-qm", "c"], when);
    }

    fn now_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    fn init_churn_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        churn_git(dir.path(), &["init", "-q"], None);
        dir
    }

    #[test]
    fn recent_churn_is_none_for_a_repo_without_commits() {
        let dir = init_churn_repo();
        assert!(recent_churn(dir.path()).is_none());
    }

    #[test]
    fn recent_churn_is_empty_not_none_when_every_commit_is_older_than_30_days() {
        let dir = init_churn_repo();
        churn_commit(dir.path(), "a.rs", "1", Some(now_secs() - 40 * 86_400));
        assert_eq!(recent_churn(dir.path()), Some(HashMap::new()));
    }

    #[test]
    fn recent_churn_counts_recent_commits_by_committer_date_not_author_date() {
        let dir = init_churn_repo();
        // 提交者日期 40 天前:不算。
        churn_commit(dir.path(), "old.rs", "x", Some(now_secs() - 40 * 86_400));
        // 作者日期是 2000 年,提交者日期是现在:算近期。
        churn_commit(dir.path(), "a.rs", "1", None);
        churn_commit(dir.path(), "a.rs", "2", None);
        let churn = recent_churn(dir.path()).unwrap();
        assert_eq!(churn.get("a.rs").copied(), Some(2));
        assert_eq!(churn.get("old.rs"), None);
    }

    /// 迁移前 `git log --since` 遇到(从 HEAD 往下数的)第一个过旧的提交就停止遍历:
    /// 当 HEAD 提交的提交者日期早于 30 天、而更深处的提交反而很新(时间戳不单调)时,
    /// 那些近期提交不会被计入。bytegit 的 `churn` 按时间全局排序,会把它们算上
    /// (更符合"近 30 天"的字面含义,有意的差异,见 P3 计划"待决事项" D5);
    /// 这条测试在 Task 5 里随之翻转。
    #[test]
    fn recent_churn_stops_at_an_old_head_even_if_deeper_commits_are_recent_before_migration() {
        let dir = init_churn_repo();
        churn_commit(dir.path(), "a.rs", "1", None);
        churn_commit(dir.path(), "a.rs", "2", None);
        churn_commit(dir.path(), "old.rs", "x", Some(now_secs() - 40 * 86_400));
        let churn = recent_churn(dir.path()).unwrap();
        assert_eq!(churn.get("a.rs"), None, "{churn:?}");
    }

    #[test]
    fn recent_churn_includes_the_root_commit_files() {
        let dir = init_churn_repo();
        churn_commit(dir.path(), "a.rs", "1", None);
        assert_eq!(
            recent_churn(dir.path()).unwrap().get("a.rs").copied(),
            Some(1)
        );
    }

    #[test]
    fn recent_churn_paths_are_relative_to_a_project_subdirectory_and_exclude_outside_files() {
        let dir = init_churn_repo();
        churn_commit(dir.path(), "top.rs", "1", None);
        churn_commit(dir.path(), "sub/a.rs", "1", None);
        churn_commit(dir.path(), "sub/a.rs", "2", None);
        let churn = recent_churn(&dir.path().join("sub")).unwrap();
        assert_eq!(churn.get("a.rs").copied(), Some(2), "{churn:?}");
        assert_eq!(churn.len(), 1, "仓库根里 sub 之外的文件不出现: {churn:?}");
    }

    #[test]
    fn recent_churn_ignores_merge_commits() {
        let dir = init_churn_repo();
        let r = dir.path();
        churn_commit(r, "a.rs", "a", None);
        churn_git(r, &["checkout", "-q", "-b", "other"], None);
        churn_commit(r, "b.rs", "b", None);
        churn_git(r, &["checkout", "-q", "-"], None);
        churn_commit(r, "c.rs", "c", None);
        churn_git(r, &["merge", "-q", "--no-ff", "other", "-m", "merge"], None);
        let churn = recent_churn(r).unwrap();
        assert_eq!(
            churn.get("b.rs").copied(),
            Some(1),
            "只算 other 上那一次: {churn:?}"
        );
        assert_eq!(churn.get("a.rs").copied(), Some(1));
        assert_eq!(churn.get("c.rs").copied(), Some(1));
    }

    #[test]
    fn recent_churn_counts_a_pure_rename_for_the_new_path_only() {
        let dir = init_churn_repo();
        let r = dir.path();
        churn_commit(r, "old.rs", "same content\nline 2\nline 3\n", None);
        churn_git(r, &["mv", "old.rs", "new.rs"], None);
        churn_git(r, &["commit", "-qm", "rename"], None);
        let churn = recent_churn(r).unwrap();
        assert_eq!(churn.get("new.rs").copied(), Some(1));
        assert_eq!(churn.get("old.rs").copied(), Some(1), "只有最初添加那一次");
    }

    #[test]
    fn recent_churn_counts_the_old_path_of_a_deletion() {
        let dir = init_churn_repo();
        let r = dir.path();
        churn_commit(r, "gone.rs", "1", None);
        churn_git(r, &["rm", "-q", "gone.rs"], None);
        churn_git(r, &["commit", "-qm", "delete"], None);
        assert_eq!(recent_churn(r).unwrap().get("gone.rs").copied(), Some(2));
    }

    /// 迁移前 `git log --name-only` 对非 ASCII 文件名输出 git 转义过的八进制(带引号),
    /// 所以真实的 UTF-8 路径永远匹配不上——churn 对这类文件恒为 0。bytegit 迁移后返回真实路径
    /// (有意的改进,同 P1 的 `dirty_paths`),这条测试在 Task 4 里随之翻转。
    #[test]
    fn recent_churn_keys_non_ascii_names_as_git_escaped_text_before_migration() {
        let dir = init_churn_repo();
        churn_commit(dir.path(), "文档.md", "x", None);
        let churn = recent_churn(dir.path()).unwrap();
        assert_eq!(churn.get("文档.md"), None, "{churn:?}");
        assert_eq!(churn.len(), 1, "{churn:?}");
        assert!(churn.keys().next().unwrap().contains("\\"), "{churn:?}");
    }
```

- [ ] **Step 5: 在旧实现上运行,确认全部通过**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p3 && cargo fmt -p dozer-app -p dozerd && git status --short`
Expected: 只有上面三个文件变了。

Run: `cargo test -p dozerd --lib projects && cargo test -p dozer-app extensions::usage && cargo test -p dozer-app git_hotspots`
Expected: `dozerd` `projects` 22 个、`usage` 47 个、`git_hotspots` 27 个全部通过。**任何新增测试在旧实现上失败,说明我对旧行为的描述有误——先停下确认旧行为,不要改实现,也不要直接改断言迎合。**(这些测试依赖本机 `git`;`git` 版本差异若导致失败,先确认是旧实现本身的结果。)

- [ ] **Step 6: 门禁与 Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p3
git add crates/dozerd/src/projects.rs crates/dozer-app/src/extensions/usage/mod.rs crates/dozer-app/src/extensions/codehealth/git_hotspots.rs
git diff --cached --stat
git commit -m "test(git): characterize commit stats and churn behavior before bytegit P3 migration

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: dozer — `dozerd` 项目更新时间迁移

**Files:**
- Modify: `crates/dozerd/Cargo.toml`、`crates/dozerd/src/projects.rs`、`crates/dozer-app/Cargo.toml`、`Cargo.lock`

**Interfaces:**
- Consumes: bytegit `v0.4.0` 的 `Repo::{discover, is_bare, head_commit_time}`;Task 3 的刻画测试
- Produces: `projects::git_head_commit_ms(dir: &Path) -> Option<u64>`(私有;替代原来的 `git_repo_root` + `git_head_commit_ms` 两个函数);`compute_updated_ms` 签名不变

- [ ] **Step 1: 加依赖、升 tag、让 `Cargo.lock` 指向 tag**

1. `crates/dozerd/Cargo.toml` 的 `[dependencies]` 里 `dozer-core = ...` 之后加一行:`bytegit = { git = "https://github.com/byteboyai/bytegit", tag = "v0.4.0" }`。
2. `crates/dozer-app/Cargo.toml` 里 `bytegit = { ... tag = "v0.3.0" }` 的 tag 改成 `v0.4.0`。
3. 让 `Cargo.lock` 指向真实 tag(而不是本地 patch):临时把 `.cargo/config.toml` 里 `[patch."https://github.com/byteboyai/bytegit"]` 与下一行注释掉(保留 byteui 的 patch),运行 `cargo update -p bytegit`,再 `git diff Cargo.lock` 确认**只有** `bytegit` 的 version 与 `source` 的 tag 变化(`dozerd` 的依赖列表多一项 `bytegit` 也属预期),`source = "git+..."` 行都在。确认后恢复 `.cargo/config.toml`(之后若 `cargo build` 又去掉了 `source` 行,提交前 `git checkout Cargo.lock` 还原再重复本步)。

- [ ] **Step 2: 迁移 `projects.rs` 的生产代码**

1. `use` 区:删掉 `use std::process::Command;`;`use std::path::{Path, PathBuf};` 改为 `use std::path::Path;`(`PathBuf` 不再使用);在 `use anyhow::{Context, Result};` 之后加 `use bytegit::Repo;`。
2. 把原来从 `/// 取 `dir` 所属 git 仓库的根目录` 起、到 `/// 计算项目的 git 感知更新时间` 之前的两个函数(`git_repo_root`、`git_head_commit_ms`)整体替换为:

```rust
/// 项目所属 git 仓库 HEAD 提交的提交时间(毫秒)。不是 git 工作区(含裸仓库)、
/// 仓库没有提交或读取失败时返回 `None`。`dir` 在仓库子目录里时向上查找;
/// linked worktree 取该 worktree 自己的 HEAD。
fn git_head_commit_ms(dir: &Path) -> Option<u64> {
    if !dir.is_dir() {
        return None;
    }
    let repo = Repo::discover(dir).ok()?;
    // 裸仓库没有工作区:迁移前 `git rev-parse --show-toplevel` 在那里失败,保持不计。
    if repo.is_bare() {
        return None;
    }
    let time = repo.head_commit_time().ok()??;
    let secs = time.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(secs * 1000)
}
```

3. `compute_updated_ms` 里的 `let commit = git_repo_root(Path::new(path)).and_then(|root| git_head_commit_ms(&root));` 改为 `let commit = git_head_commit_ms(Path::new(path));`。
4. `mod tests` 里紧跟 `use super::*;` 之后加 `use std::process::Command;`(生产代码不再使用它,Task 3 的测试还要用)。

- [ ] **Step 3: 编译并运行**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p3 && cargo fmt -p dozerd && cargo build -p dozerd 2>&1 | grep -E "^(warning|error)" -A6`
Expected: 无 error、无新警告(首次会编译 libgit2,约 1–2 分钟)。

Run: `cargo test -p dozerd --lib projects && cargo test -p dozerd`
Expected: `projects` 22 个通过;`dozerd` 全量 462 个 lib 测试与全部集成测试通过。

- [ ] **Step 4: 变异检验**

把 `git_head_commit_ms` 里的 `if repo.is_bare() { return None; }` 三行删掉。Run: `cargo test -p dozerd --lib projects`。Expected: `updated_ms_ignores_a_bare_repo_because_it_has_no_work_tree` 失败。还原(`git checkout crates/dozerd/src/projects.rs` 之前先确认没有别的未提交改动)。

- [ ] **Step 5: 确认旧代码已清走、门禁、Commit**

Run: `grep -n 'Command::new\|git_repo_root' crates/dozerd/src/projects.rs`
Expected: 只剩 `mod tests` 里的 `Command::new("git")`(刻画测试的夹具)。

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p3
cargo clippy -p dozerd --all-targets 2>&1 | grep -E "projects.rs"
git status --short
git add Cargo.lock crates/dozerd/Cargo.toml crates/dozer-app/Cargo.toml crates/dozerd/src/projects.rs
git diff --cached --stat
git commit -m "refactor(dozerd): back project updated-time with bytegit, drop two git commands

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

Expected: clippy 的 grep 无输出;`git diff --cached --stat` 里没有 `.cargo/config.toml`。

---

### Task 5: dozer — `usage` 提交计数迁移

**Files:**
- Modify: `crates/dozer-app/src/extensions/usage/mod.rs`

**Interfaces:**
- Consumes: bytegit 的 `Repo::{discover, commit_count, commit_count_by_day}`;Task 3 的刻画测试
- Produces: `count_git_commits(path: &Path) -> u64`、`count_git_commits_by_day(path: &Path) -> BTreeMap<i64, u64>`(私有,签名不变)

- [ ] **Step 1: 迁移生产代码**

1. `use` 区:在 `use crate::conversation::ConversationMeta;` 之后加 `use bytegit::Repo;`。
2. 把原来从 `/// 统计项目仓库 HEAD 可达的提交总数` 起、到 `/// 面板内容侧:顶部"用量"标题` 之前的两个函数(`count_git_commits`、`count_git_commits_by_day`)整体替换为:

```rust
/// 统计项目仓库 HEAD 可达的提交总数(当前分支口径,同 git_log 面板的
/// revwalk 起点)。项目不是 git 仓库、空仓库或任何错误都一律记 0
/// ——这格统计不值得让整个面板失败。
fn count_git_commits(path: &std::path::Path) -> u64 {
    // `discover` 兼容项目路径是仓库子目录的情况。
    Repo::discover(path)
        .and_then(|repo| repo.commit_count())
        .unwrap_or(0)
}

/// 统计 HEAD 可达提交按 UTC 提交日的逐日计数,供"每日行为统计"折线图用。
/// 与 `day_index_from_ms` 同口径:UTC 日索引 = `提交时间秒 / 86_400`(注意这里
/// 整段除以 86400,跟毫秒口径 `ms / 86_400_000` 等价)。项目不是 git 仓库/
/// 任何错误都返回空 map;不会因提交多而爆炸——返回的只是"有提交的那
/// 些天"的计数,不是每一条提交。
fn count_git_commits_by_day(path: &std::path::Path) -> BTreeMap<i64, u64> {
    Repo::discover(path)
        .and_then(|repo| repo.commit_count_by_day())
        .unwrap_or_default()
}
```

- [ ] **Step 2: 编译并运行**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p3 && cargo fmt -p dozer-app && cargo build -p dozer-app 2>&1 | grep -E "^(warning|error)" -A6`
Expected: 无 error;警告只有"已知基线"里与本计划无关的两条。

Run: `cargo test -p dozer-app extensions::usage`
Expected: 47 个全部通过。

- [ ] **Step 3: 确认旧代码已清走、门禁、Commit**

Run: `grep -n "git2" crates/dozer-app/src/extensions/usage/mod.rs`
Expected: 无输出。

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p3
cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "usage/mod.rs"
git status --short
git add crates/dozer-app/src/extensions/usage/mod.rs
git diff --cached --stat
git commit -m "refactor(usage): back git commit counts with bytegit

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

Expected: clippy 的 grep 无输出。

---

### Task 6: dozer — `recent_churn` 迁移

**Files:**
- Modify: `crates/dozer-app/src/extensions/codehealth/git_hotspots.rs`

**Interfaces:**
- Consumes: bytegit 的 `Repo::{discover, root, churn}`;Task 3 的刻画测试
- Produces: `git_hotspots::recent_churn(project_root: &Path) -> Option<HashMap<String, usize>>`(签名与语义不变:路径相对项目根、不含项目目录之外的文件);私有 `project_prefix(repo: &Repo, project_root: &Path) -> Option<PathBuf>`

- [ ] **Step 1: 迁移生产代码**

1. 文件头的模块文档注释里,把"单次批量\n//! `git log --since=30.days --name-only --relative` 构建文件修改计数,"改为"单次批量\n//! `bytegit` 的 `churn`(近 30 天提交的文件改动)构建文件修改计数,"。
2. `use` 区:`use std::path::Path;` 与 `use std::process::Command;` 两行替换为 `use std::path::{Path, PathBuf};` 与 `use std::time::{Duration, SystemTime};`。
3. 把原来的 `recent_churn`(从 `/// 近 30 天各文件(相对项目根的规范化路径)的提交次数。` 起、到 `/// 当前 Git dirty` 之前)整体替换为:

```rust
/// 近 30 天各文件(相对项目根的规范化路径)的提交次数。Git 不可用/失败返回
/// `None`;成功但无提交返回 `Some(空)`。路径相对项目根(项目目录之外的文件不出现),
/// 与扫描发现的相对路径同口径,可直接按路径匹配。
///
/// 查询在 `bytegit::Repo::churn`(路径相对**仓库根**),这里再换成相对项目根:
/// 项目目录在仓库子目录时,去掉子目录前缀并丢弃目录之外的文件(迁移前由
/// `git log --relative` 完成)。
pub fn recent_churn(project_root: &Path) -> Option<HashMap<String, usize>> {
    let repo = Repo::discover(project_root).ok()?;
    let since = SystemTime::now() - Duration::from_secs(CHURN_WINDOW_DAYS * 86_400);
    let churn = repo.churn(since).ok()?;
    let prefix = project_prefix(&repo, project_root)?;
    let mut map: HashMap<String, usize> = HashMap::new();
    for (path, count) in churn {
        let Ok(relative) = path.strip_prefix(&prefix) else {
            continue;
        };
        *map.entry(relative.to_string_lossy().replace('\\', "/"))
            .or_insert(0) += count as usize;
    }
    Some(map)
}

/// 近期改动统计的窗口(天)。
const CHURN_WINDOW_DAYS: u64 = 30;

/// 项目目录相对仓库根的前缀(项目就是仓库根时为空)。两边先规范化:libgit2 返回的仓库根
/// 是解析过符号链接的真实路径(macOS 上 `/var/...` 会变成 `/private/var/...`)。
fn project_prefix(repo: &Repo, project_root: &Path) -> Option<PathBuf> {
    let root = repo.root().canonicalize().ok()?;
    let project = project_root.canonicalize().ok()?;
    project.strip_prefix(&root).ok().map(Path::to_path_buf)
}
```

4. `mod tests` 里紧跟 `use super::*;` 之后加 `use std::process::Command;`(生产代码不再使用它)。

- [ ] **Step 2: 翻转两条已知会变的刻画测试**

在 `git_hotspots.rs` 的 `mod tests` 里,把 Task 3 的这两条测试**整体替换**为下面两条(其余刻画测试一字不改):

- `recent_churn_keys_non_ascii_names_as_git_escaped_text_before_migration` → `recent_churn_reports_non_ascii_names_as_real_utf8`
- `recent_churn_stops_at_an_old_head_even_if_deeper_commits_are_recent_before_migration` → `recent_churn_counts_recent_commits_even_when_head_is_old`

```rust
    /// 迁移前 `git log --name-only` 对非 ASCII 文件名输出 git 转义过的八进制(带引号),
    /// 真实的 UTF-8 路径永远匹配不上,churn 对这类文件恒为 0。迁移后返回真实路径
    /// (有意的改进,同 P1 的 `dirty_paths`)。
    #[test]
    fn recent_churn_reports_non_ascii_names_as_real_utf8() {
        let dir = init_churn_repo();
        churn_commit(dir.path(), "文档.md", "x", None);
        let churn = recent_churn(dir.path()).unwrap();
        assert_eq!(churn.get("文档.md").copied(), Some(1), "{churn:?}");
        assert_eq!(churn.len(), 1, "{churn:?}");
    }

    /// 迁移前 `git log --since` 遇到(从 HEAD 往下数的)第一个过旧的提交就停止遍历:
    /// 当 HEAD 提交的提交者日期早于 30 天、而更深处的提交反而很新(时间戳不单调)时,
    /// 那些近期提交不会被计入。bytegit 的 `churn` 按时间全局排序,会把它们算上
    /// (更符合"近 30 天"的字面含义,有意的差异,见 P3 计划"待决事项" D5)。
    #[test]
    fn recent_churn_counts_recent_commits_even_when_head_is_old() {
        let dir = init_churn_repo();
        churn_commit(dir.path(), "a.rs", "1", None);
        churn_commit(dir.path(), "a.rs", "2", None);
        churn_commit(dir.path(), "old.rs", "x", Some(now_secs() - 40 * 86_400));
        let churn = recent_churn(dir.path()).unwrap();
        assert_eq!(churn.get("a.rs").copied(), Some(2), "{churn:?}");
        assert_eq!(churn.get("old.rs"), None, "{churn:?}");
    }
```

- [ ] **Step 3: 编译并运行**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p3 && cargo fmt -p dozer-app && cargo build -p dozer-app 2>&1 | grep -E "^(warning|error)" -A6`
Expected: 无 error;警告只有"已知基线"里与本计划无关的两条。

Run: `cargo test -p dozer-app git_hotspots && cargo test -p dozer-app codehealth`
Expected: `git_hotspots` 27 个全部通过;`codehealth` 其余测试无新失败。

- [ ] **Step 4: 变异检验**

把 `recent_churn` 里的 `let Ok(relative) = path.strip_prefix(&prefix) else { continue; };` 换成 `let relative = path.as_path(); let _ = &prefix;`(相当于忽略项目子目录)。Run: `cargo test -p dozer-app git_hotspots`。Expected: `recent_churn_paths_are_relative_to_a_project_subdirectory_and_exclude_outside_files` 失败。还原。

- [ ] **Step 5: 确认旧代码已清走、门禁、Commit**

Run: `grep -n 'Command::new("git")' crates/dozer-app/src/extensions/codehealth/git_hotspots.rs; grep -n 'cfg(test)' crates/dozer-app/src/extensions/codehealth/git_hotspots.rs; grep -n -e '--relative' -e '"log"' crates/dozer-app/src/extensions/codehealth/git_hotspots.rs`
Expected: `Command::new("git")` 的行号都大于 `#[cfg(test)]` 的行号(只在测试夹具里);最后一条 `grep` 无输出(生产代码里没有 `--relative` 与 `git log`)。

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p3
cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "git_hotspots"
git status --short
git add crates/dozer-app/src/extensions/codehealth/git_hotspots.rs
git diff --cached --stat
git commit -m "refactor(codehealth): back recent_churn with bytegit

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

把"已迁移/未迁移"的描述更新为:P1(状态/HEAD/分支/远程)、P2(`git_log`/`file_history` 的历史、改动、blob 读取、回滚取内容)、P3(`usage` 的提交计数、`recent_churn`、`dozerd/projects.rs` 的项目更新时间)已迁移;**未迁移**(P4–P5):`git_watch`、`init/clone/checkout` 及 `delivery.rs`/`homespace.rs` 里剩余的命令行 `git`。并加一句:`dozerd` 现在也依赖 `bytegit`(同样用 tag,联调用不提交的 `[patch]`)。若条目里还没有 P1/P2 的措辞(对应分支尚未合并),只改与 P3 相关的部分,不要替它们写。

- [ ] **Step 2: 规格同步**

在 `docs/superpowers/specs/2026-10-02-bytegit-design.md`:

1. §4.2 的 `head_commit_time`:注明是**提交者时间**(等于 `git log -1 --format=%ct`),HEAD 指向最新提交而不是历史里最大的时间戳;没有提交为 `Ok(None)`。
2. §4.4:
   - `LogOptions` 去掉 `since`(规格原有字段),并说明"P3 的使用者要的是 `churn(since)`,没有 `log` + `since` 的使用者,需要时再加";
   - `commit_count`、`commit_count_by_day`、`churn` 与实现对齐:`commit_count` 为 HEAD 可达提交数(合并进来的只算一次);`commit_count_by_day` 的日索引 = **提交者时间**秒 / 86400(整数除法,与迁移前 `usage` 同口径);`churn(since)` 返回 `HashMap<PathBuf, u32>`,**路径相对仓库根**,不含合并提交,根提交相对空树,做重命名检测(重命名只计新路径、删除计旧路径),按提交时间全局排序、遇到第一个早于 `since` 的提交即停止;
   - 三个遍历类方法在 HEAD 未诞生时统一返回 `GitErrorKind::NoCommits`。
3. §6 P3 一行:验收补充"刻画测试在旧/新实现上都通过";"删除的旧代码"注明包含 `dozerd/projects.rs` 的 `git_repo_root`/`git_head_commit_ms`、`usage` 的两个 revwalk 函数、`recent_churn` 的命令行调用。
4. §8:
   - **O10** 标为已完成并写明结论:bytegit 的路径一律相对仓库根(`status` 与 `churn` 一致);`recent_churn` 的适配层再换成相对项目根(`dirty_paths` 仍相对仓库根,与迁移前一致);
   - 加 **O13**(= 本计划 D5):`churn` 对"HEAD 过旧而更深处有近期提交"的结果比旧实现多;
   - 加 **O14**(= D6):`churn` 在超大仓库上比命令行慢(libgit2 的时间排序遍历会先解析整段历史),与 O5 同类;
   - O5 补一句:`commit_count*`、`churn` 的全历史扫描沿用现有口径,未优化。

- [ ] **Step 3: 要求文档、盘点文档、记忆**

- `docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` §7 第 1 条后追加:`bytegit` P3 已完成(`v0.4.0`),`dozerd` 已依赖 `bytegit`。
- `docs/dozer-v2/bytegit-调用点盘点.md`:在 §1 末尾加一行"P3 之后,`dozerd` 与 `dozer-app` 的两份项目更新时间/提交统计实现、`recent_churn` 的命令行调用已合并为 bytegit";并 `grep -n "recent_churn\|count_git_commits\|git_head_commit_ms\|git_repo_root\|rev-parse" docs/dozer-v2/bytegit-调用点盘点.md`,把命中的调用点标注为"P3 已迁移"。
- 记忆文件追加 P3 结果与待决事项 D5/D6/D7(一两行),并更新各分支(`bytegit-p1/p2/p3`)的合并现状。

- [ ] **Step 4: Commit(worktree)**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p3
git status --short
git add CLAUDE.md docs/superpowers/specs/2026-10-02-bytegit-design.md docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md docs/dozer-v2/bytegit-调用点盘点.md
git diff --cached --stat
git commit -m "docs(bytegit): record P3 results, API decisions and open decisions

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 5: 交给用户合并**

不要自己合并 `bytegit-p3` 到 `main`:向用户报告分支、提交列表、"已知基线"里的失败、待决事项 D5/D6/D7,并请用户审阅后决定合并方式(`bytegit-p1`→`bytegit-p2`→`bytegit-p3` 是叠在一起的,合并顺序见"前置条件")。合并前请用户:① 在真实项目上打开 Usage 面板看提交数与每日折线图、Code Health 面板看热点排序,② 看一眼 `target/release/dozerd` 的体积变化(D7)。**GUI 渲染本计划没有自动化覆盖。**

---

## Self-Review

**Spec coverage(规格 §6 的 P3 行与 §4.2、§4.4):**
- `head_commit_time`(§4.2)、`commit_count`、`commit_count_by_day`、`churn`(§4.4):Task 1;发布 `v0.4.0`:Task 2。
- 迁移 `usage` 的 `commit_count*`:Task 5;`git_hotspots::recent_churn`:Task 6;`dozerd/projects.rs` 的两处命令行 + `dozerd` 加依赖:Task 4。
- 规格 §4.4 的 `LogOptions.since` 没有实现(没有使用者),Task 7 Step 2 回写;P2 说过"`since` 等 P3 用到再加",核对后 P3 的使用者走的是 `churn(since)`,所以不加。
- 规格 O10(`dirty_paths` 与 `recent_churn` 路径口径):Task 6 的适配层 + Task 7 Step 2 落实为"bytegit 统一相对仓库根,适配层转换"。
- 规格 §4.4 对 `churn` 写的是 `HashMap<PathBuf, u32>`,实现一致。

**Placeholder scan:** 无 TBD/TODO;每个代码步骤都给了完整代码或"旧区域起止 → 新代码"的精确替换;刻画测试与翻转测试给了完整代码。

**Type consistency:** `head_commit_time`/`commit_count`/`commit_count_by_day`/`churn`/`head_walk` 在 Task 1 定义,Task 4–6 使用同名同形;`TempRepo::{commit_file_at, commit_staged_at, merge_commit_at}` 在 Task 1 定义并仅在 bytegit 测试里使用;dozer 侧三个适配函数签名与迁移前一致。

**Review Focus:** 五条各自有对应测试(分别见各条所指任务),其中 bytegit 侧 6 个变异检验、dozer 侧 2 个变异检验已在草稿里实际确认会让指定测试失败。
