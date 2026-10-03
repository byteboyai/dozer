# bytegit P6:收尾——拆掉 `delivery.rs`、去掉 `git2` 直接依赖 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 P1–P5 留下的 `delivery.rs` 适配层拆干净:约 20 处调用点改成直接使用 `bytegit`,文件树着色这块 Dozer 自己的呈现语义搬到 `extensions/files/git_status.rs`,`delivery.rs` 与 `mod delivery;` 删除,`dozer-app` 不再直接依赖 `git2`(规格 §6 P6)。bytegit 为此补两个**显式**的构造函数:`Repo::open_exact`(只认仓库根)与 `Repo::discover_workdir`(目录、非 bare、向上查找),把 P1 起悬而未决的"只认仓库根 vs 向上查找"(规格 O9)从适配层里的暗语变成 API 上看得见的两个名字。行为保持等价。

**Architecture:** 三步走,每步都能单独通过测试。(1) bytegit 加两个构造函数并发版。(2) dozer 先**在适配层还在时**把四处"调用点组合逻辑"抽成可测试的函数并补 16 个刻画测试(它们会把 `discover` 与 `open_exact` 混用造成的口径不一致原样钉住),再把着色代码连同它的 10 个测试搬到 `git_status.rs`。(3) 调用点逐个改成 bytegit 的链式调用,删除 `delivery.rs`,去掉 `git2` 依赖;同一批刻画测试在新旧实现上都通过。验收是规格写明的三件套:`cargo machete`、clippy、全量测试。

**Tech Stack:** Rust edition 2024;dozer 侧 `bytegit` 以 git tag 依赖;`gleisbau` 仍通过自己的依赖带入 `git2`(与 bytegit 要求同为 `git2 = "0.21"`、无默认 feature,cargo 统一成同一份)。

**Spec:** `docs/superpowers/specs/2026-10-02-bytegit-design.md`(§4.1 的 `Repo::open_exact`、§6 的 P6、§8 的 O6/O9)。前序计划:P0–P5。

**本计划中的代码已在草稿里完整跑过。** 草稿基线 = `bytegit-p3` 分支(含 P1–P3)+ P5 计划里已验证的适配代码(用来模拟"P1–P5 都合并后"的 `main`)。bytegit:加上两个构造函数后默认构建 135 个、`--all-features` 158 个测试通过(新增 10 个),3 个变异检验会让对应测试失败。dozer 侧三个状态的 `cargo test -p dozer-app` 通过数:基线 1747;Task 3 之后 1763(+16);Task 4 之后 1764(+1,新增的"冲突文件没有文件状态"测试);Task 5 之后 1732(删除 `delivery.rs` 里 32 个测试,覆盖去向见"测试去向");每一步都只剩那个已知基线失败(`delete_confirm_spec_reflects_pending_target`)。最终状态下 `cargo machete` 报告"没有未使用的依赖"(基线上它**正好只报 `git2`**),`Cargo.lock` 里 `git2` 仍只有一份,`cargo clippy -p dozer-app --all-targets` 的诊断与基线逐文件一致,4 个 dozer 侧变异检验会让对应测试失败。执行者照抄即可;结果与此不符先停下排查,不要改测试迎合实现。

## 前置条件

- **P6 必须最后执行:** dozer 侧 P1–P5 都已合并 `main`(尤其 P5:`delivery.rs` 里要有 `checkout_branch`/`init_repo`/`git_available`/`clone_repo` 四个适配函数,以及 P3 的 `usage`、P2 的 `diff_content` 已迁移)。本计划的替换文本逐字对应那个状态;若 `main` 上某个函数还是旧的命令行实现,先完成对应阶段。
- bytegit 仓库 `main` 的最新 tag 在 P5 之后应是 `v0.6.0`(P4 的 `watch` 是 `v0.5.0`,P5 的写操作是 `v0.6.0`),本计划发 `v0.7.0`。**规则是"最新 tag 加一档"**:若实际顺序不同,把本计划里所有 `0.7.0`/`v0.7.0` 顺延成实际的下一个版本(`grep -n "0\.7\.0" <计划文件>` 能找全);dozer 侧的 `tag` 同理。
- dozer 侧 P6 分支从合并后的 `main` 开(Task 3 Step 1)。

## Global Constraints

- bytegit:Rust edition `2024`;公开 API 不得出现 `git2::*`;全部同步。
- dozer 侧**所有命令都在专用 worktree 里执行,路径写全**(`~/Projects/CoralProjects/byteboy/dozer-bytegit-p6/...`);主 checkout 常有并发会话与未提交改动,不得在其上改代码或 `git add -A`。提交前 `git status` 与 `git diff --cached --stat` 看全貌,只 `git add` 指定路径。
- dozer 的 `bytegit` 依赖(`dozer-app` 的 `[dependencies]` 与 `[dev-dependencies]`、`dozerd`)一律用 tag;本地联调的 `[patch]` 只放 `.cargo/config.toml`,**不提交**。带着本地 `[patch]` 构建会让 `Cargo.lock` 里 `bytegit`/`byteui` 的 `source = "git+..."` 行消失;提交 `Cargo.lock` 前必须把 `.cargo/config.toml` 里 `bytegit` 的 `[patch]` 段暂时注释掉再 `cargo update -p bytegit`(见 Task 3 Step 2),确认 diff 里 `source` 行都在、只有 bytegit 的 tag 变了。
- **迁移不得改变用户可见行为——包括那个不一致的口径。** 哪个调用点原来向上查找、哪个只认仓库根,逐个保持原样(见"口径清单");统一它是产品决策(待决事项 D14),不在 P6 里顺手做。
- 变异检验的还原方式:**先 `git add` 暂存当前版本,变异后用 `git checkout -- <文件>` 还原到暂存版本**(新文件不暂存没法还原;未暂存的已修改文件直接 `git checkout` 会把迁移改动一起冲掉)。
- dozer 门禁:触碰的文件不产生新诊断、基线之外无新失败(见"已知基线")。`cargo fmt -p dozer-app` 后用 `git status --short` 确认**只有本任务触碰的文件**变了;fmt 动了别的文件就 `git checkout` 回去。
- 提交信息用 `feat:`/`fix:`/`refactor:`/`test:`/`docs:` 前缀;提交末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## 已知基线

- `extensions::files::tests::delete_confirm_spec_reflects_pending_target` 在 `main` 上就失败。
- `assets::tests::serves_vendored_asset_with_mime` **偶发**失败(基线上我见过一次,单独跑、重跑都通过;与 git 无关)。
- `extensions::git_log::tests::build_marks_head_branch_and_labels`、`build_populates_time_and_is_merge` 对 dozer 自己的仓库跑 `gleisbau`,在克隆/linked worktree 里可能失败(P1 起记录;合并回 `main` 后在 `main` 上重跑确认)。
- `cargo build -p dozer-app` 有两条与本计划无关的 dead_code 警告(`usage/aggregate.rs::agent_token_share`、`git_accounts.rs::GitProvider::ALL`);`cargo clippy` 在 `workspace/state.rs`、`homespace.rs` 等处有既有诊断(Task 5 Step 7 用"与基线逐文件对比"验收)。
- `cargo machete` 在 P3 之后的基线上**只**报 `dozer-app` 的 `git2` 未使用——这正是本计划要消除的。

## 口径清单(P6 之后,哪个调用点用哪个构造函数)

迁移前适配层混用了两种语义(`open_exact` 只认仓库根;`discover` 向上查找),P1 起一直保持原样。P6 把每个调用点用的语义**显式写出来**:

| 构造函数 | 调用点(含它取的字段) |
|---|---|
| `Repo::open_exact`(只认仓库根;项目在仓库子目录里时拿不到) | `workspace::project_git_snapshot` 的分支/是否有改动/文件状态;`git_log::load_branch_picker_data` 的"是否有改动";`git_hotspots::git_snapshot` 的分支/是否有改动,`head_short_sha`;`file_history`/`git_log`/`diff_content` 的历史、blob、工作区内容、回滚、上一版本(经 `diff_content::open_exact_repo`);`files::git_status::file_statuses` |
| `Repo::discover`(向上查找) | `workspace::project_git_snapshot` 的 remote URL;`git_log::load_branch_picker_data` 的分支列表;`checkout_branch` 的两个调用点;`usage` 的提交计数(P3);`recent_churn`(P3);`dozerd` 项目更新时间(P3) |
| `Repo::discover_workdir`(目录、非 bare、向上查找) | `workspace::workspace_git_info`;`files::load_git_info`;`git_hotspots::git_snapshot` 的"是否在仓库里"判断;首页最近文件(`homespace::load_home_recents`) |

同一个"项目在仓库子目录里"的场景,在不同面板里表现不同(分支栏有分支,Project 面板没有分支、Git Log 的"有改动"恒为否但分支列表是全的……)。**这是迁移前就有的不一致**,P6 用刻画测试把它钉住;要不要统一见 D14。

## Review Focus

规格对下列输入没有逐条说明,但真实使用者会遇到;每条都由指明的任务里的测试固定下来(行为均在草稿里实测确认):

1. **项目目录在仓库子目录里时,各调用点的口径必须各保持原样。** `project_git_snapshot` 只找得到 remote;分支选择器能列分支但"有改动"恒为否;`workspace_git_info`/`load_git_info` 向上找到仓库并如实报告。dozer Task 3 的 `project_git_snapshot_in_a_repo_subdirectory_only_finds_remotes`、`branch_picker_data_in_a_repo_subdirectory_finds_branches_but_never_dirtiness`、`workspace_git_info_looks_upward_from_a_subdirectory`、`a_repo_subdirectory_is_looked_up_to_the_repo`;Task 5 的变异检验 1、2 守着它们。
2. **`discover_workdir` 的两条拒绝规则。** 参数是个文件路径时按"不是仓库"处理(`Repo::discover` 对文件路径会成功,所以不能直接用);bare 仓库没有工作区,按"不是仓库"处理。bytegit Task 1 的 `discover_workdir_rejects_a_file_path`、`discover_workdir_rejects_a_bare_repo`;dozer Task 3 的 `workspace_git_info_is_none_outside_a_repo_and_for_a_file_path`、`a_file_path_is_not_a_repo`;Task 5 变异检验 3、4。
3. **删掉的 32 个 `delivery.rs` 测试不能让覆盖悄悄消失。** 见下面"测试去向":每一个要么搬走、要么在 bytegit 里有同义测试、要么在 Task 3 的新测试里有对应,没有"直接删掉"的。
4. **着色语义搬家后不变。** 10 个原有的 `file_statuses_*`/`dir_status_*`/`rollup_dir_statuses_*` 测试一字不改地搬到 `git_status.rs` 并通过;冲突文件在 `file_statuses` 里没有状态(原来夹在 `merge_conflict_makes_the_repo_dirty_but_has_no_file_status` 里的那半条断言,改写成 `a_merge_conflicted_file_has_no_file_status`)。
5. **去掉 `git2` 直接依赖不能拉出第二份 `git2`,也不能影响 `gleisbau`。** `gleisbau` 要求 `git2 = "0.21"`(无默认 feature),与 bytegit 的要求相同,所以锁文件里仍只有一份;`git_log::build` 用 `gleisbau` 的返回值时只调用方法、不点名 `git2::` 类型,所以不需要这个依赖也能编译。验收:`grep -c '^name = "git2"' Cargo.lock` 为 `1`,`cargo machete` 干净,`cargo build` 与全量测试通过。

## 测试去向(`delivery.rs` 的 42 个测试,P5 之后)

| 测试 | 去向 |
|---|---|
| `file_statuses_maps_modified_new_deleted`、`file_statuses_distinguishes_staged_and_unstaged`、`file_statuses_marks_ignored_files`、`dir_status_*`(4 个)、`rollup_dir_statuses_*`(3 个) | **搬走**到 `git_status.rs`,一字不改(Task 4) |
| `merge_conflict_makes_the_repo_dirty_but_has_no_file_status` | "仓库算有改动"那半在 bytegit(`conflicted_files_are_flagged_and_count_as_dirty`);"冲突文件没有文件状态"那半改写为 `git_status.rs` 的 `a_merge_conflicted_file_has_no_file_status`(Task 4) |
| `repo_root_and_head_and_dirty`、`branch_of_repo`、`branch_none_when_head_detached`、`detached_head_still_counts_as_having_commits`、`empty_repo_has_no_commits_no_branch_but_an_empty_branch_list`、`non_git_dir_answers_the_empty_way`、`ignored_files_do_not_make_the_repo_dirty_but_untracked_ones_do` | bytegit 的 `head_*`/`detached_head_*`/`status`/`is_dirty` 测试(P1),加上 dozer 的组合测试 `workspace_git_info_*`、`project_git_snapshot_*`、`a_repo_*`(Task 3) |
| `remote_url_*`(4 个)、`remote_urls_are_deduped_and_ordered_by_remote_name` | bytegit 的 `remotes_*`(P1);去重与排序在 dozer 的 `unique_remote_urls`,由 `project_git_snapshot_remote_urls_are_deduped_in_remote_name_order` 覆盖(Task 3) |
| `local_branches_are_listed_in_name_order` | bytegit 的 `local_branches_are_sorted_*`(P1)与 `branch_picker_data_lists_sorted_branches_and_flags_dirtiness`(Task 3) |
| `project_in_a_repo_subdirectory_is_treated_as_not_a_repo_by_open_based_queries` | bytegit 的 `open_exact_rejects_a_subdirectory_of_a_repo`(Task 1)与 dozer 的三个子目录组合测试(Task 3) |
| `git_available_detects_system_git`、`clone_repo_*`(3 个)、`init_repo_*`(5 个)、`checkout_*`/`a_conflicting_edit_*`/`an_untracked_file_*`/`checking_out_*`(8 个) | bytegit 的 `write::tests`(P5 搬进去的,有同义测试) |

## 待决事项

- **D14(请你定是否排期):统一"只认仓库根 vs 向上查找"。** 上面的"口径清单"就是它的全貌:同一个"项目目录在仓库子目录里"(典型的 monorepo 子包),分支栏有分支、Project 面板没有分支也没有"有改动"标记、文件树不着色、Git Log 面板的历史会报"不是 git 仓库的根目录"。我的建议:状态类查询(`project_git_snapshot` 的分支/改动/文件状态、`load_branch_picker_data` 的改动、`git_snapshot`)改成向上查找,文件状态的键仍按项目路径拼,这是安全的改动;**历史类**(`file_history`/`git_log` 的 `file_path` 是相对项目根的,而 pathspec 相对仓库根)统一需要先把 `file_path` 换算成相对仓库根,是单独的、有风险的改动。P6 完成后做这件事就是"逐个调用点把 `Repo::open_exact` 换成 `Repo::discover`",bytegit 与 API 都不用动。

规格补充(Task 6 回写):§4.1 增加 `Repo::open_exact`(只认仓库根,否则 `NotARepo`,错误文案 `不是 git 仓库的根目录: <路径>`)与 `Repo::discover_workdir`(目录、非 bare);把"口径清单"整张表写进 §8 的 O9;O6 标为已完成(25 个调用点逐个核对并迁移);P6 一行标完成。

---

## File Structure

**bytegit 仓库**(`~/Projects/CoralProjects/byteboy/bytegit/`):

| 文件 | 变更 |
|------|------|
| `Cargo.toml` | 版本升一档(本计划写 `0.7.0`) |
| `src/repo.rs` | 新增 `Repo::open_exact`、`Repo::discover_workdir`、私有 `same_dir` 与 10 个测试 |

**dozer 仓库**(专用 worktree `~/Projects/CoralProjects/byteboy/dozer-bytegit-p6/`,分支 `bytegit-p6`):

| 文件 | 变更 |
|------|------|
| `crates/dozer-app/src/delivery.rs`、`main.rs` 里的 `mod delivery;` | **删除** |
| `crates/dozer-app/src/extensions/files/git_status.rs` | **新增**:`ChangeKind`、`FileGitStatus`、`file_statuses`、`TreeState`、`dir_status`、`rollup_dir_statuses` 与它们的测试(从 `delivery.rs` 搬来) |
| `crates/dozer-app/src/extensions/files/{mod,state,tree,update,view}.rs` | 引用路径 `delivery::` → `git_status::`;`update.rs` 里 `load_git_info`、checkout/init 调用点 |
| `crates/dozer-app/src/workspace/state.rs` | 新增 `workspace_git_info`、`project_git_snapshot`、`unique_remote_urls`(原先是闭包里的内联调用) |
| `crates/dozer-app/src/extensions/git_log.rs` | 新增 `load_branch_picker_data`;`open_exact_or_err` → `open_exact_repo` |
| `crates/dozer-app/src/extensions/{file_history,diff_content}.rs` | `diff_content` 新增 `open_exact_repo`;`file_history` 改用它 |
| `crates/dozer-app/src/extensions/codehealth/git_hotspots.rs`、`chrome/homespace.rs`、`extensions/project_create.rs`、`extensions/project/scaffold.rs`、`app/update.rs` | 调用点改成 bytegit |
| `crates/dozer-app/Cargo.toml`、`crates/dozerd/Cargo.toml`、`Cargo.lock` | bytegit tag 升级;`dozer-app` 去掉 `git2`、加 `bytegit` 的 `testutil` dev-dependency |
| 规格、`CLAUDE.md`、要求文档、盘点文档 | Task 6 同步 |

---

### Task 1: bytegit — 两个显式的构造函数

**Files:**
- Modify: `src/repo.rs`

**Interfaces:**
- Consumes(已有):`Repo::{discover, root, is_bare}`、`GitError`/`GitErrorKind`、`TempRepo`
- Produces(Task 3–5 依赖):
  - `Repo::open_exact(path: &Path) -> Result<Repo, GitError>`:`path` 规范化后必须等于仓库根(bare 仓库为它的 git 目录),否则 `GitErrorKind::NotARepo`、文案 `不是 git 仓库的根目录: <路径>`;不存在的路径是 `Io`
  - `Repo::discover_workdir(dir: &Path) -> Result<Repo, GitError>`:`dir` 必须是目录(否则 `Io`),向上查找,bare 仓库返回 `NotARepo`

- [ ] **Step 1: 在 `impl Repo` 里加两个构造函数**

在 `src/repo.rs` 的 `impl Repo` 里,`/// 工作区根目录(bare 仓库为 git 目录)。` 那个 `root()` 方法**之前**加入:

```rust
    /// 只认仓库根:`path` 必须正好是某个仓库的工作区根(bare 仓库为它的 git 目录)才打开,
    /// 比较前两边都先规范化(符号链接、尾部分隔符都不影响)。`path` 在仓库**子目录**里时返回
    /// `GitErrorKind::NotARepo`——与 [`Repo::discover`] 的向上查找相反。
    ///
    /// 这是 Dozer 早期按"项目路径就是仓库根"写的那批查询(分支、是否有改动、文件状态)的语义,
    /// 保留它为一个**显式**的构造函数,而不是悄悄改成向上查找(规格 §8 O9:是否统一是产品决策)。
    pub fn open_exact(path: &Path) -> Result<Self, GitError> {
        let repo = Self::discover(path)?;
        if same_dir(repo.root(), path) {
            Ok(repo)
        } else {
            Err(GitError::new(
                GitErrorKind::NotARepo,
                format!("不是 git 仓库的根目录: {}", path.display()),
            ))
        }
    }

    /// 从**目录** `dir` 向上查找它所属的、有工作区的仓库。`dir` 不是目录(含是个文件)返回
    /// `GitErrorKind::Io`,仓库是 bare 的返回 `GitErrorKind::NotARepo`——bare 仓库没有可
    /// 供"看状态"的工作区。([`Repo::discover`] 对文件路径与 bare 仓库都会成功。)
    pub fn discover_workdir(dir: &Path) -> Result<Self, GitError> {
        if !dir.is_dir() {
            return Err(GitError::new(
                GitErrorKind::Io,
                format!("不是目录: {}", dir.display()),
            ));
        }
        let repo = Self::discover(dir)?;
        if repo.is_bare() {
            return Err(GitError::new(
                GitErrorKind::NotARepo,
                format!("裸仓库没有工作区: {}", dir.display()),
            ));
        }
        Ok(repo)
    }
```

- [ ] **Step 2: 加私有辅助函数 `same_dir`**

在 `src/repo.rs` 里 `impl std::fmt::Debug for Repo {` 之前加入:

```rust
/// 两个目录是同一个目录(规范化后相等);任一规范化失败按不同处理。
fn same_dir(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}
```

- [ ] **Step 3: 在 `mod tests` 末尾加 10 个测试**

在 `src/repo.rs` 的 `mod tests` 里,最后一个测试之后、模块结尾 `}` 之前追加:

```rust
    // ---- open_exact / discover_workdir ----

    #[test]
    fn open_exact_opens_the_repo_root() {
        let t = TempRepo::new();
        let repo = Repo::open_exact(t.path()).unwrap();
        assert!(same(repo.root(), t.path()));
    }

    #[test]
    fn open_exact_ignores_a_trailing_separator_and_dot_components() {
        let t = TempRepo::new();
        let with_dot = t.path().join(".");
        assert!(Repo::open_exact(&with_dot).is_ok());
        let mut with_slash = t.path().as_os_str().to_os_string();
        with_slash.push("/");
        assert!(Repo::open_exact(Path::new(&with_slash)).is_ok());
    }

    #[test]
    fn open_exact_rejects_a_subdirectory_of_a_repo() {
        let t = TempRepo::new();
        t.write_untracked("sub/a.txt", "x");
        let err = Repo::open_exact(&t.path().join("sub")).unwrap_err();
        assert_eq!(err.kind(), GitErrorKind::NotARepo);
        assert!(
            err.message().starts_with("不是 git 仓库的根目录: "),
            "{}",
            err.message()
        );
        // 对比:`discover` 向上查找,同一个路径能打开。
        assert!(Repo::discover(&t.path().join("sub")).is_ok());
    }

    #[test]
    fn open_exact_outside_any_repo_and_for_a_missing_path_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Repo::open_exact(dir.path()).unwrap_err().kind(),
            GitErrorKind::NotARepo
        );
        assert_eq!(
            Repo::open_exact(&dir.path().join("nope"))
                .unwrap_err()
                .kind(),
            GitErrorKind::Io
        );
    }

    #[cfg(unix)]
    #[test]
    fn open_exact_accepts_a_symlink_to_the_repo_root() {
        let t = TempRepo::new();
        let links = tempfile::tempdir().unwrap();
        let link = links.path().join("link");
        std::os::unix::fs::symlink(t.path(), &link).unwrap();
        assert!(Repo::open_exact(&link).is_ok());
    }

    #[test]
    fn open_exact_accepts_a_bare_repo_directory() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init_bare(dir.path()).unwrap();
        let repo = Repo::open_exact(dir.path()).unwrap();
        assert!(repo.is_bare());
    }

    #[test]
    fn discover_workdir_finds_the_repo_from_a_directory_and_from_its_subdirectory() {
        let t = TempRepo::new();
        t.write_untracked("sub/a.txt", "x");
        assert!(same(
            Repo::discover_workdir(t.path()).unwrap().root(),
            t.path()
        ));
        assert!(same(
            Repo::discover_workdir(&t.path().join("sub"))
                .unwrap()
                .root(),
            t.path()
        ));
    }

    #[test]
    fn discover_workdir_rejects_a_file_path() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "x", "one");
        let err = Repo::discover_workdir(&t.path().join("a.txt")).unwrap_err();
        assert_eq!(err.kind(), GitErrorKind::Io);
    }

    #[test]
    fn discover_workdir_rejects_a_bare_repo() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init_bare(dir.path()).unwrap();
        let err = Repo::discover_workdir(dir.path()).unwrap_err();
        assert_eq!(err.kind(), GitErrorKind::NotARepo);
    }

    #[test]
    fn discover_workdir_outside_any_repo_and_for_a_missing_dir_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Repo::discover_workdir(dir.path()).unwrap_err().kind(),
            GitErrorKind::NotARepo
        );
        assert_eq!(
            Repo::discover_workdir(&dir.path().join("nope"))
                .unwrap_err()
                .kind(),
            GitErrorKind::Io
        );
    }
```

- [ ] **Step 4: 运行测试与门禁**

Run: `cargo test repo:: && cargo test && cargo test --all-features`
Expected: `repo::` 23 个通过(原有 13 个 + 新增 10 个);全量比基线多 10 个。

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings`
Expected: 干净。

- [ ] **Step 5: 变异检验**

先 `git add src/repo.rs` 暂存当前版本;逐个做、逐个用 `git checkout -- src/repo.rs` 还原:

1. 把 `open_exact` 里的 `if same_dir(repo.root(), path) {` 改成 `if true || same_dir(repo.root(), path) {`。Run: `cargo test repo::`。Expected: `open_exact_rejects_a_subdirectory_of_a_repo` 失败。
2. 把 `discover_workdir` 里的 `if !dir.is_dir() {` 改成 `if false {`。Expected: `discover_workdir_rejects_a_file_path` 失败。
3. 把 `discover_workdir` 里的 `if repo.is_bare() {` 改成 `if false && repo.is_bare() {`。Expected: `discover_workdir_rejects_a_bare_repo` 失败。

- [ ] **Step 6: Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/bytegit
git status --short
git add src/repo.rs
git diff --cached --stat
git commit -m "feat: add Repo::open_exact and Repo::discover_workdir

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: bytegit — 发布 v0.7.0

**Files:**
- Modify: `Cargo.toml`(版本升一档,本计划写 `0.7.0`)

**Interfaces:** Produces:git tag `v0.7.0`(dozer 的依赖)。

- [ ] **Step 1: 升版本并全量门禁**

把 `Cargo.toml` 的版本改为 `0.7.0`(版本号顺延规则见"前置条件")。

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings && cargo test && cargo test --all-features`
Expected: 全部通过(基线 + 10 个)。

- [ ] **Step 2: Commit 并推送 main(对外可见操作:先向用户确认)**

向用户确认"现在推送 `main` 并打 tag `v0.7.0`"。获得明确同意后:

```bash
cd ~/Projects/CoralProjects/byteboy/bytegit
git add Cargo.toml
git commit -m "chore: release 0.7.0

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git push origin main
gh run watch "$(gh run list --repo byteboyai/bytegit --limit 1 --json databaseId -q '.[0].databaseId')" --repo byteboyai/bytegit --exit-status
```

Expected: CI(macOS)通过。**CI 红则先修,不打 tag。**

- [ ] **Step 3: 打 tag 并验证可被消费**

```bash
git tag -a v0.7.0 -m "bytegit v0.7.0: Repo::open_exact, Repo::discover_workdir"
git push origin v0.7.0
cd "$(mktemp -d)" && cargo new --lib consume -q && cd consume
cargo add bytegit --git https://github.com/byteboyai/bytegit --tag v0.7.0 --features testutil
cat >> src/lib.rs <<'EOF'

#[cfg(test)]
mod t {
    #[test]
    fn consumes_bytegit_constructors() {
        let t = bytegit::testutil::TempRepo::new();
        t.write_untracked("sub/a.txt", "x");
        assert!(bytegit::Repo::open_exact(t.path()).is_ok());
        assert!(bytegit::Repo::open_exact(&t.path().join("sub")).is_err());
        assert!(bytegit::Repo::discover_workdir(&t.path().join("sub")).is_ok());
    }
}
EOF
cargo test
```

Expected: `consumes_bytegit_constructors ... ok`。

---

### Task 3: dozer — 抽出调用点组合逻辑,在适配层上补刻画测试

**Files:**
- Modify(worktree 内): `crates/dozer-app/Cargo.toml`(`[dev-dependencies]`)、`crates/dozer-app/src/workspace/state.rs`、`crates/dozer-app/src/extensions/files/update.rs`、`crates/dozer-app/src/extensions/git_log.rs`、`crates/dozer-app/src/app/update.rs`

**Interfaces:**
- Consumes: 现有的 `delivery::{repo_root, branch, is_dirty, file_statuses, remote_url, local_branches, current_branch_has_commits}`(此刻适配层还在)
- Produces(Task 5 保持签名不变、只换函数体):
  - `workspace::state::workspace_git_info(cwd: &Path) -> Option<WorkspaceGitInfo>`
  - `workspace::state::project_git_snapshot(repo_path: &Path) -> (Option<String>, bool, HashMap<PathBuf, FileGitStatus>, Vec<String>)`
  - `extensions::files::load_git_info(root: &Path) -> GitInfo`
  - `extensions::git_log::load_branch_picker_data(repo_path: &Path) -> (Vec<String>, bool)`

- [ ] **Step 1: 建专用 worktree 与分支**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-bytegit-p6 -b bytegit-p6 main
mkdir -p ../dozer-bytegit-p6/.cargo && cp .cargo/config.toml ../dozer-bytegit-p6/.cargo/config.toml
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p6 && git log --oneline | head -1 && git status --short
grep -n "pub fn \(checkout_branch\|init_repo\|git_available\|clone_repo\)" crates/dozer-app/src/delivery.rs
```

Expected: 干净、分支 `bytegit-p6`,HEAD 与 `main` 一致;最后一条 `grep` 能找到四个函数(说明 P5 已合并)。`.cargo/config.toml`(不提交,已被 `.gitignore`)里的 `[patch."https://github.com/byteboyai/bytegit"]` 指向本地 `../bytegit`,此时它已含 Task 1–2 的改动。此后**所有命令都用 `~/Projects/CoralProjects/byteboy/dozer-bytegit-p6/` 前缀**。

- [ ] **Step 2: 加 `testutil` dev-dependency**

在 `crates/dozer-app/Cargo.toml` 的 `[dev-dependencies]` 里 `tempfile = "3"` 之后加(`tag` 写成与 `[dependencies]` 里 `bytegit` 那一行**相同**的值——下面的 `v0.6.0` 是按 P5 之后写的示例,以 `main` 上实际的 tag 为准;Task 5 Step 1 再统一升到新版本):

```toml
# 面板单元测试用 `TempRepo` 造真实仓库。
bytegit = { git = "https://github.com/byteboyai/bytegit", tag = "v0.6.0", features = ["testutil"] }
```

(`dozer-app` 里这条依赖同时出现在 `[dependencies]` 与 `[dev-dependencies]` 是合法的,后者只是多开一个 feature。)

- [ ] **Step 3: `workspace/state.rs` 抽出两个函数并加测试**

1. 把 agent 卡片刷新里这一段:

```rust
                let workspace = if needs_workspace {
                    delivery::repo_root(&cwd).map(|repo| WorkspaceGitInfo {
                        branch: delivery::branch(&repo),
                        dirty: delivery::is_dirty(&repo),
                    })
                } else {
                    None
                };
```

   改为:

```rust
                let workspace = if needs_workspace {
                    workspace_git_info(&cwd)
                } else {
                    None
                };
```

2. 把 `spawn_project_git_refresh` 里的这一段:

```rust
        let (b, d, s, r) = tokio::task::spawn_blocking({
            let repo_path = repo_path.clone();
            move || {
                (
                    delivery::branch(&repo_path),
                    delivery::is_dirty(&repo_path),
                    delivery::file_statuses(&repo_path),
                    delivery::remote_url(&repo_path),
                )
            }
        })
        .await
        .unwrap_or((None, false, HashMap::new(), Vec::new()));
```

   改为:

```rust
        let (b, d, s, r) = tokio::task::spawn_blocking({
            let repo_path = repo_path.clone();
            move || project_git_snapshot(&repo_path)
        })
        .await
        .unwrap_or((None, false, HashMap::new(), Vec::new()));
```

3. 在 `/// 磁盘占用是独立于组合 git 刷新的异步任务` 之前加入(此刻内部仍调用适配层):

```rust
/// agent 会话当前 cwd 所属仓库的分支/脏标(卡片工作区行用)。cwd 不是目录、不在任何仓库里、
/// 或仓库是 bare 时为 `None`;cwd 在仓库**子目录**里照样向上找到仓库。
pub(crate) fn workspace_git_info(cwd: &Path) -> Option<WorkspaceGitInfo> {
    delivery::repo_root(cwd).map(|repo| WorkspaceGitInfo {
        branch: delivery::branch(&repo),
        dirty: delivery::is_dirty(&repo),
    })
}

/// 项目的 git 快照:`(当前分支, 是否有改动, 文件级状态, 全部 remote 的 URL)`。
///
/// **注意口径不一致(迁移前就如此,保持原样):** 分支、改动、文件状态只认仓库根——项目目录
/// 在仓库子目录里时它们是 `None`/`false`/空;而 remote URL 向上查找,子目录里照样能取到。
pub(crate) fn project_git_snapshot(
    repo_path: &Path,
) -> (
    Option<String>,
    bool,
    HashMap<PathBuf, delivery::FileGitStatus>,
    Vec<String>,
) {
    (
        delivery::branch(repo_path),
        delivery::is_dirty(repo_path),
        delivery::file_statuses(repo_path),
        delivery::remote_url(repo_path),
    )
}
```

4. 在 `workspace/state.rs` 文件末尾追加测试模块:

```rust
#[cfg(test)]
mod git_snapshot_tests {
    use super::*;
    use bytegit::testutil::TempRepo;

    fn info(branch: Option<&str>, dirty: bool) -> Option<WorkspaceGitInfo> {
        Some(WorkspaceGitInfo {
            branch: branch.map(str::to_string),
            dirty,
        })
    }

    #[test]
    fn workspace_git_info_is_none_outside_a_repo_and_for_a_file_path() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(workspace_git_info(dir.path()), None);
        let t = TempRepo::new();
        t.commit_file("a.txt", "x\n", "one");
        assert_eq!(workspace_git_info(&t.path().join("a.txt")), None);
    }

    #[test]
    fn workspace_git_info_reports_branch_and_dirtiness() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "x\n", "one");
        assert_eq!(workspace_git_info(t.path()), info(Some("main"), false));
        t.write_untracked("new.txt", "y");
        assert_eq!(workspace_git_info(t.path()), info(Some("main"), true));
    }

    #[test]
    fn workspace_git_info_looks_upward_from_a_subdirectory() {
        let t = TempRepo::new();
        t.commit_file("sub/a.txt", "x\n", "one");
        t.write_untracked("new.txt", "y");
        assert_eq!(
            workspace_git_info(&t.path().join("sub")),
            info(Some("main"), true)
        );
    }

    #[test]
    fn workspace_git_info_has_no_branch_on_a_detached_head() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "x\n", "one");
        t.detach_head();
        assert_eq!(workspace_git_info(t.path()), info(None, false));
    }

    #[test]
    fn project_git_snapshot_of_a_non_repo_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let (branch, dirty, statuses, remotes) = project_git_snapshot(dir.path());
        assert_eq!((branch, dirty), (None, false));
        assert!(statuses.is_empty() && remotes.is_empty());
    }

    #[test]
    fn project_git_snapshot_collects_branch_dirtiness_statuses_and_remotes() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "x\n", "one");
        t.add_remote("origin", "https://example.com/x.git");
        t.write_untracked("a.txt", "changed\n");
        t.write_untracked("new.txt", "y");
        let (branch, dirty, statuses, remotes) = project_git_snapshot(t.path());
        assert_eq!(branch.as_deref(), Some("main"));
        assert!(dirty);
        assert!(statuses.contains_key(&t.path().join("a.txt")));
        assert!(statuses.contains_key(&t.path().join("new.txt")));
        assert_eq!(remotes, vec!["https://example.com/x.git".to_string()]);
    }

    #[test]
    fn project_git_snapshot_remote_urls_are_deduped_in_remote_name_order() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "x\n", "one");
        t.add_remote("c", "https://x/2");
        t.add_remote("b", "https://x/1");
        t.add_remote("a", "https://x/1");
        let (_, _, _, remotes) = project_git_snapshot(t.path());
        assert_eq!(
            remotes,
            vec!["https://x/1".to_string(), "https://x/2".to_string()]
        );
    }

    /// 口径不一致(迁移前就如此):项目在仓库子目录里时,分支/改动/文件状态按"不是仓库"处理,
    /// remote URL 却能向上找到。是否统一是规格 O9 的待决事项,迁移保持原样。
    #[test]
    fn project_git_snapshot_in_a_repo_subdirectory_only_finds_remotes() {
        let t = TempRepo::new();
        t.commit_file("sub/a.txt", "x\n", "one");
        t.add_remote("origin", "https://example.com/x.git");
        t.write_untracked("new.txt", "y");
        let (branch, dirty, statuses, remotes) = project_git_snapshot(&t.path().join("sub"));
        assert_eq!((branch, dirty), (None, false));
        assert!(statuses.is_empty());
        assert_eq!(remotes, vec!["https://example.com/x.git".to_string()]);
    }
}
```

- [ ] **Step 4: `extensions/files/update.rs` 抽出 `load_git_info` 并加测试**

1. 把 `spawn_git_info_load` 里 `spawn_blocking` 的闭包(从 `let info = tokio::task::spawn_blocking(move || {` 起到闭包结尾 `})` 止,里面是 `use crate::delivery::{...}` 与一个 `match repo_root(&root) {...}`)替换为:

```rust
        let info = tokio::task::spawn_blocking(move || load_git_info(&root))
```

   (后面的 `.await.unwrap_or_else(|_| GitInfo { ... })` 不动。)
2. 在 `#[cfg(test)]\nmod send_payload_tests {` 之前加入:

```rust
/// 分支栏用的 git 信息:`root` 所属的(非 bare)仓库;不在仓库里为 `is_repo: false`。
/// `root` 在仓库子目录里照样向上找到仓库。
pub(crate) fn load_git_info(root: &Path) -> GitInfo {
    use crate::delivery::{branch, current_branch_has_commits, local_branches, repo_root};
    match repo_root(root) {
        Some(repo) => GitInfo {
            is_repo: true,
            current_branch: branch(&repo),
            current_branch_has_commits: current_branch_has_commits(&repo),
            branches: local_branches(&repo).unwrap_or_default(),
        },
        None => GitInfo {
            is_repo: false,
            current_branch: None,
            current_branch_has_commits: false,
            branches: Vec::new(),
        },
    }
}

#[cfg(test)]
mod git_info_tests {
    use super::*;
    use bytegit::testutil::TempRepo;

    fn not_a_repo() -> GitInfo {
        GitInfo {
            is_repo: false,
            current_branch: None,
            current_branch_has_commits: false,
            branches: Vec::new(),
        }
    }

    #[test]
    fn a_non_repo_is_not_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_git_info(dir.path()), not_a_repo());
    }

    #[test]
    fn a_repo_reports_branch_commits_and_sorted_branches() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "x\n", "one");
        t.branch("feature");
        assert_eq!(
            load_git_info(t.path()),
            GitInfo {
                is_repo: true,
                current_branch: Some("main".to_string()),
                current_branch_has_commits: true,
                branches: vec!["feature".to_string(), "main".to_string()],
            }
        );
    }

    #[test]
    fn a_repo_without_commits_has_no_branch_and_no_branches() {
        let t = TempRepo::new();
        assert_eq!(
            load_git_info(t.path()),
            GitInfo {
                is_repo: true,
                current_branch: None,
                current_branch_has_commits: false,
                branches: Vec::new(),
            }
        );
    }

    #[test]
    fn a_repo_subdirectory_is_looked_up_to_the_repo() {
        let t = TempRepo::new();
        t.commit_file("sub/a.txt", "x\n", "one");
        let info = load_git_info(&t.path().join("sub"));
        assert!(info.is_repo);
        assert_eq!(info.current_branch.as_deref(), Some("main"));
    }

    #[test]
    fn a_file_path_is_not_a_repo() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "x\n", "one");
        assert_eq!(load_git_info(&t.path().join("a.txt")), not_a_repo());
    }
}
```

- [ ] **Step 5: `extensions/git_log.rs` 加 `load_branch_picker_data` 与测试;`app/update.rs` 两处改用它**

1. 在 `git_log.rs` 的 `/// commit 搜索框(iced 原生 `text_input`)的 `widget::Id`` 之前加入:

```rust
/// 分支选择器要的数据:`(本地分支名升序, 工作区是否有改动)`。
///
/// **注意口径不一致(迁移前就如此,保持原样):** 分支列表向上查找(`repo_path` 在仓库子目录里
/// 也能取到),"是否有改动"只认仓库根(子目录里恒为 `false`)。不在仓库里为 `(空, false)`。
pub(crate) fn load_branch_picker_data(repo_path: &Path) -> (Vec<String>, bool) {
    let branches = crate::delivery::local_branches(repo_path).unwrap_or_default();
    let dirty = crate::delivery::is_dirty(repo_path);
    (branches, dirty)
}
```

2. 在 `git_log.rs` 的 `mod tests` 末尾(最后一个测试之后、模块结尾 `}` 之前)追加:

```rust
    // ---- bytegit P6:分支选择器数据迁移前后必须一致的口径 ----

    #[test]
    fn branch_picker_data_of_a_non_repo_is_empty_and_clean() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_branch_picker_data(dir.path()), (Vec::new(), false));
    }

    #[test]
    fn branch_picker_data_lists_sorted_branches_and_flags_dirtiness() {
        use bytegit::testutil::TempRepo;
        let t = TempRepo::new();
        t.commit_file("a.txt", "x\n", "one");
        t.branch("feature");
        let branches = vec!["feature".to_string(), "main".to_string()];
        assert_eq!(load_branch_picker_data(t.path()), (branches.clone(), false));
        t.write_untracked("new.txt", "y");
        assert_eq!(load_branch_picker_data(t.path()), (branches, true));
    }

    /// 口径不一致(迁移前就如此):子目录里分支列表能取到,"是否有改动"恒为 false。
    #[test]
    fn branch_picker_data_in_a_repo_subdirectory_finds_branches_but_never_dirtiness() {
        use bytegit::testutil::TempRepo;
        let t = TempRepo::new();
        t.commit_file("sub/a.txt", "x\n", "one");
        t.write_untracked("new.txt", "y");
        assert_eq!(
            load_branch_picker_data(&t.path().join("sub")),
            (vec!["main".to_string()], false)
        );
    }
```

3. `crates/dozer-app/src/app/update.rs`:
   - macOS 分支里把

     ```rust
                        let branches =
                            crate::delivery::local_branches(&repo_path).unwrap_or_default();
                        let dirty = crate::delivery::is_dirty(&repo_path);
     ```

     改为 `let (branches, dirty) = git_log::load_branch_picker_data(&repo_path);`;
   - 非 macOS 分支里把

     ```rust
                            let (branches, dirty) = tokio::task::spawn_blocking(move || {
                                let branches = crate::delivery::local_branches(&repo_path2)
                                    .unwrap_or_default();
                                let dirty = crate::delivery::is_dirty(&repo_path2);
                                (branches, dirty)
                            })
     ```

     改为:

     ```rust
                            let (branches, dirty) = tokio::task::spawn_blocking(move || {
                                git_log::load_branch_picker_data(&repo_path2)
                            })
     ```

- [ ] **Step 6: 在适配层上运行,确认全部通过**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p6 && cargo fmt -p dozer-app && git status --short`
Expected: 只有上面列出的文件(含 `Cargo.lock`,见 Step 7)变了。

Run: `cargo test -p dozer-app workspace::state::git_snapshot_tests && cargo test -p dozer-app extensions::files::update::git_info_tests && cargo test -p dozer-app extensions::git_log::tests::branch_picker && cargo test -p dozer-app 2>&1 | grep -E "FAILED|test result"`
Expected: 8 个 + 5 个 + 3 个新测试全部通过;全量比基线多 16 个,失败只有"已知基线"里的。**任何新增测试在适配层上失败,说明我对旧行为的描述有误——先停下确认旧行为,不要改实现,也不要直接改断言迎合。**

- [ ] **Step 7: 门禁与 Commit**

`Cargo.lock` 提交前按"Global Constraints"里的方法处理(本任务只是多了 `bytegit` 的 `testutil` feature,锁文件里 `bytegit` 多出 `tempfile` 依赖是预期的)。

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p6
git status --short
git add Cargo.lock crates/dozer-app/Cargo.toml crates/dozer-app/src/workspace/state.rs crates/dozer-app/src/extensions/files/update.rs crates/dozer-app/src/extensions/git_log.rs crates/dozer-app/src/app/update.rs
git diff --cached --stat
git commit -m "test(git): characterize call-site git composition before dropping delivery.rs

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

Expected: `git diff --cached --stat` 里没有 `.cargo/config.toml`。

---

### Task 4: dozer — 着色语义搬到 `files/git_status.rs`

**Files:**
- Create: `crates/dozer-app/src/extensions/files/git_status.rs`
- Modify: `crates/dozer-app/src/delivery.rs`、`extensions/files/{mod,state,tree,update,view}.rs`、`workspace/state.rs`、`chrome/homespace.rs`

**Interfaces:**
- Consumes: 现有 `delivery::{ChangeKind, FileGitStatus, file_statuses, TreeState, dir_status, rollup_dir_statuses}`(原样搬走);bytegit 的 `Repo::open_exact`
- Produces(Task 5 依赖):`extensions::files::git_status::{ChangeKind, FileGitStatus, file_statuses(&Path), TreeState, dir_status, rollup_dir_statuses}`,签名与原先一致

- [ ] **Step 1: 新建 `git_status.rs`**

1. 文件头(模块文档与 `use`):

```rust
//! 文件树的 git 状态装饰:把 `bytegit` 的状态条目映射成文件树名称颜色要用的档位。
//!
//! 这是 Dozer 自己的**呈现语义**(哪几种状态归为"未跟踪/新增/修改",目录取子孙中的最高档),
//! 不是 git 查询——查询在 `bytegit`。原先在 `delivery.rs`,P6 起随 `delivery.rs` 一起拆掉后搬到这里。

use bytegit::{ChangeKind as GitChange, Repo, StatusOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
```

2. 把 `delivery.rs` 里从 `/// 文件树装饰用的 git 改动类型` 起、到 `/// 当前分支名;非 git` 之前的整段(`ChangeKind`、`FileGitStatus`、`file_statuses`、`TreeState` 及其 `From<FileGitStatus>`、`dir_status`、`state_priority`、`rollup_dir_statuses`)**原样**剪切到新文件头部之后。只改一处:`file_statuses` 里的

```rust
    let Some(git_repo) = open_exact(repo) else {
        return map;
    };
```

改为

```rust
    let Ok(git_repo) = Repo::open_exact(repo) else {
        return map;
    };
```

(文档注释里的"见 [`open_exact`]"改成"见 `bytegit::Repo::open_exact`"。)
3. 在新文件末尾加测试模块:先从 `delivery.rs` 的 `mod tests` 里把 `mkrepo()` 辅助函数**复制**一份、把 10 个测试(`file_statuses_maps_modified_new_deleted`、`file_statuses_distinguishes_staged_and_unstaged`、`file_statuses_marks_ignored_files`、`dir_status_marks_ignored_dir_only_when_dir_itself_ignored`、`dir_status_aggregates_untracked_over_modified`、`dir_status_priority_staged_new_over_modified`、`dir_status_untracked_beats_every_other`、`rollup_dir_statuses_matches_dir_status_per_dir`、`rollup_dir_statuses_marks_dir_itself_ignored`、`rollup_dir_statuses_self_ignored_wins_over_descendant`)**剪切**过来,一字不改,再加一个新测试:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    // <此处:从 delivery.rs 复制来的 mkrepo(),以及 10 个剪切来的测试,一字不改>

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
```

- [ ] **Step 2: 更新 `delivery.rs` 与所有引用路径**

1. `delivery.rs`:去掉不再使用的 `use bytegit::ChangeKind as GitChange` 与 `use std::collections::HashMap`(`use bytegit::{ChangeKind as GitChange, Repo, StatusOptions};` → `use bytegit::{Repo, StatusOptions};`,删 `use std::collections::HashMap;`);在 `mod tests` 里紧跟 `use super::*;` 之后加 `use crate::extensions::files::git_status::file_statuses;`(剩下的 `delivery` 测试里还有 4 处用它)。
2. `extensions/files/mod.rs`:`use crate::delivery::FileGitStatus;\n\nmod state;` 改为 `use git_status::FileGitStatus;\n\npub(crate) mod git_status;\nmod state;`;`mod tests` 里删掉 `use crate::delivery;`;文件里所有 `delivery::TreeState::` → `git_status::TreeState::`,`crate::delivery::FileGitStatus` → `git_status::FileGitStatus`,`crate::delivery::ChangeKind` → `git_status::ChangeKind`。
3. `extensions/files/tree.rs`、`state.rs`:`use crate::delivery;` → `use super::git_status;`;`delivery::` → `git_status::`,`crate::delivery::TreeState` → `git_status::TreeState`。
4. `extensions/files/view.rs`:`use crate::{delivery, theme};` → `use super::git_status;\nuse crate::theme;`;`delivery::` → `git_status::`。
5. `extensions/files/update.rs`:`use crate::delivery;` → `use super::git_status;`(文件里 `delivery::rollup_dir_statuses` → `git_status::rollup_dir_statuses`;`checkout_branch`/`init_repo`/`load_git_info` 里的 `crate::delivery::...` 是带完整路径的,此任务不动)。
6. `workspace/state.rs`:`project_git_snapshot` 的返回类型 `HashMap<PathBuf, delivery::FileGitStatus>` → `HashMap<PathBuf, files::git_status::FileGitStatus>`,调用 `delivery::file_statuses(repo_path)` → `files::git_status::file_statuses(repo_path)`。
7. `chrome/homespace.rs`:加 `use crate::extensions::files::git_status;`(保留 `use crate::delivery;`),`delivery::file_statuses(&repo)` → `git_status::file_statuses(&repo)`。

- [ ] **Step 3: 编译并运行**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p6 && cargo fmt -p dozer-app && git status --short && cargo build -p dozer-app 2>&1 | grep -E "^(warning|error)" -A7`
Expected: 无 error;警告只有"已知基线"里与本计划无关的两条(若出现 `unused import`,按提示删掉对应的 `use`)。

Run: `cargo test -p dozer-app 2>&1 | grep -E "FAILED|test result"`
Expected: 通过数比 Task 3 之后**多 1 个**(新增的 `a_merge_conflicted_file_has_no_file_status`);失败只有"已知基线"里的。

Run: `cargo test -p dozer-app extensions::files::git_status`
Expected: 11 个通过(搬来的 10 个 + 新增 1 个)。

- [ ] **Step 4: 门禁与 Commit**

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" -A4 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c`,与 Task 3 之前(在主 checkout 上同样跑一次)逐文件对比。
Expected: 除 `delivery.rs` → `git_status.rs` 的搬迁外,诊断条数不变。

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p6
git status --short
git add crates/dozer-app/src/delivery.rs crates/dozer-app/src/extensions/files crates/dozer-app/src/workspace/state.rs crates/dozer-app/src/chrome/homespace.rs
git diff --cached --stat
git commit -m "refactor(files): move git status decoration out of delivery.rs into files/git_status.rs

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: dozer — 调用点改用 bytegit,删除 `delivery.rs`,去掉 `git2`

**Files:**
- Delete: `crates/dozer-app/src/delivery.rs`;`crates/dozer-app/src/main.rs` 里的 `mod delivery;`
- Modify: `crates/dozer-app/Cargo.toml`、`crates/dozerd/Cargo.toml`、`Cargo.lock`、`workspace/state.rs`、`extensions/files/update.rs`、`extensions/git_log.rs`、`extensions/file_history.rs`、`extensions/diff_content.rs`、`extensions/codehealth/git_hotspots.rs`、`chrome/homespace.rs`、`extensions/project_create.rs`、`extensions/project/scaffold.rs`、`app/update.rs`

**Interfaces:**
- Consumes: bytegit 新版本的 `Repo::{open_exact, discover, discover_workdir, head, is_dirty, remotes, local_branches, checkout_branch}`、`init`、`clone`、`CloneOptions`、`git_available`
- Produces: 所有被改函数的**签名不变**;`diff_content::open_exact_repo(&Path) -> Result<Repo, String>`

- [ ] **Step 1: 升依赖、让 `Cargo.lock` 指向 tag**

1. 把 `crates/dozer-app/Cargo.toml`(`[dependencies]` 与 Task 3 加的 `[dev-dependencies]`)和 `crates/dozerd/Cargo.toml` 里 `bytegit` 的 tag 都改成 Task 2 发布的版本(通常是 `v0.7.0`)。
2. 让 `Cargo.lock` 指向真实 tag(而不是本地 patch):临时把 `.cargo/config.toml` 里 `[patch."https://github.com/byteboyai/bytegit"]` 与下一行注释掉(保留 byteui 的 patch),运行 `cargo update -p bytegit`,再 `git diff Cargo.lock` 确认**只有** `bytegit` 的 version 与 `source` 的 tag 变化,`source = "git+..."` 行都在。确认后恢复 `.cargo/config.toml`(之后若 `cargo build` 又去掉了 `source` 行,提交前 `git checkout Cargo.lock` 还原再重复本步)。

- [ ] **Step 2: `workspace/state.rs`——三个函数换成 bytegit**

1. 删掉 `use crate::delivery::{self};`。
2. 把 Task 3 加的 `workspace_git_info`、`project_git_snapshot` 两个函数整体替换,并新增 `unique_remote_urls`:

```rust
/// agent 会话当前 cwd 所属仓库的分支/脏标(卡片工作区行用)。cwd 不是目录、不在任何仓库里、
/// 或仓库是 bare 时为 `None`;cwd 在仓库**子目录**里照样向上找到仓库。
pub(crate) fn workspace_git_info(cwd: &Path) -> Option<WorkspaceGitInfo> {
    let repo = bytegit::Repo::discover_workdir(cwd).ok()?;
    Some(WorkspaceGitInfo {
        branch: repo.head().ok().and_then(|h| h.branch),
        dirty: repo
            .is_dirty(bytegit::StatusOptions::default())
            .unwrap_or(false),
    })
}

/// 项目的 git 快照:`(当前分支, 是否有改动, 文件级状态, 全部 remote 的 URL)`。
///
/// **注意口径不一致(迁移前就如此,保持原样):** 分支、改动、文件状态只认仓库根
/// (`Repo::open_exact`)——项目目录在仓库子目录里时它们是 `None`/`false`/空;而 remote URL
/// 向上查找(`Repo::discover`),子目录里照样能取到。
pub(crate) fn project_git_snapshot(
    repo_path: &Path,
) -> (
    Option<String>,
    bool,
    HashMap<PathBuf, files::git_status::FileGitStatus>,
    Vec<String>,
) {
    let exact = bytegit::Repo::open_exact(repo_path).ok();
    let branch = exact
        .as_ref()
        .and_then(|repo| repo.head().ok())
        .and_then(|head| head.branch);
    let dirty = exact.as_ref().is_some_and(|repo| {
        repo.is_dirty(bytegit::StatusOptions::default())
            .unwrap_or(false)
    });
    (
        branch,
        dirty,
        files::git_status::file_statuses(repo_path),
        unique_remote_urls(repo_path),
    )
}

/// 仓库**全部** remote 的 fetch URL:去重、按 remote 名字升序(与 `git remote -v` 的顺序一致);
/// 没有 remote / 不在仓库里为空 `Vec`。向上查找。
fn unique_remote_urls(path: &Path) -> Vec<String> {
    let Ok(remotes) = bytegit::Repo::discover(path).and_then(|repo| repo.remotes()) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut urls = Vec::new();
    for remote in remotes {
        if seen.insert(remote.url.clone()) {
            urls.push(remote.url);
        }
    }
    urls
}
```

- [ ] **Step 3: `extensions/files/update.rs`——`load_git_info`、checkout、init**

1. `load_git_info` 整体替换为:

```rust
/// 分支栏用的 git 信息:`root` 所属的(非 bare)仓库;不在仓库里为 `is_repo: false`。
/// `root` 在仓库子目录里照样向上找到仓库。
pub(crate) fn load_git_info(root: &Path) -> GitInfo {
    match bytegit::Repo::discover_workdir(root) {
        Ok(repo) => {
            let head = repo.head().ok();
            GitInfo {
                is_repo: true,
                current_branch: head.as_ref().and_then(|h| h.branch.clone()),
                current_branch_has_commits: head.as_ref().is_some_and(|h| h.has_commits()),
                branches: repo.local_branches().unwrap_or_default(),
            }
        }
        Err(_) => GitInfo {
            is_repo: false,
            current_branch: None,
            current_branch_has_commits: false,
            branches: Vec::new(),
        },
    }
}
```

2. checkout 调用点:把 `crate::delivery::checkout_branch(&root, &name)` 替换为

```rust
bytegit::Repo::discover(&root)
    .and_then(|repo| repo.checkout_branch(&name))
    .map_err(|e| e.message().to_string())
```

3. init 调用点:把 `tokio::task::spawn_blocking(move || crate::delivery::init_repo(&root))` 替换为

```rust
tokio::task::spawn_blocking(move || {
    bytegit::init(&root)
        .map(|_| ())
        .map_err(|e| e.message().to_string())
})
```

- [ ] **Step 4: 历史类调用点——`diff_content::open_exact_repo`**

1. 在 `extensions/diff_content.rs` 里 `/// 两个 blob 之间的双侧文本。` 之前加入:

```rust
/// 按"项目路径就是仓库根"打开仓库(项目在仓库子目录里时报错,而不是向上查找),失败信息可直接展示。
/// 两个面板的历史/文件内容查询共用它:`file_path` 是相对项目根的,必须和仓库根一致,否则查错文件。
pub(crate) fn open_exact_repo(path: &Path) -> Result<Repo, String> {
    Repo::open_exact(path).map_err(|e| e.message().to_string())
}
```

2. `extensions/file_history.rs`:删掉 `use crate::delivery::open_exact_or_err;`;把 `use crate::extensions::diff_content::{DiffBlobContent, workdir_content};` 改为 `use crate::extensions::diff_content::{DiffBlobContent, open_exact_repo, workdir_content};`;全文件 `open_exact_or_err(` → `open_exact_repo(`。
3. `extensions/git_log.rs`:删掉 `use crate::delivery::open_exact_or_err;`;`use crate::extensions::diff_content::{DiffBlobContent, blob_pair_content};` 改为 `use crate::extensions::diff_content::{DiffBlobContent, blob_pair_content, open_exact_repo};`;`use bytegit::{BlobId, ChangeKind, CommitId};` 改为 `use bytegit::{BlobId, ChangeKind, CommitId, Repo, StatusOptions};`;全文件 `open_exact_or_err(` → `open_exact_repo(`。
4. `git_log.rs` 的 `load_branch_picker_data` 整个函数(含文档注释)替换为:

```rust
/// 分支选择器要的数据:`(本地分支名升序, 工作区是否有改动)`。
///
/// **注意口径不一致(迁移前就如此,保持原样):** 分支列表向上查找(`repo_path` 在仓库子目录里
/// 也能取到),"是否有改动"只认仓库根(子目录里恒为 `false`)。不在仓库里为 `(空, false)`。
pub(crate) fn load_branch_picker_data(repo_path: &Path) -> (Vec<String>, bool) {
    let branches = Repo::discover(repo_path)
        .and_then(|repo| repo.local_branches())
        .unwrap_or_default();
    let dirty = Repo::open_exact(repo_path)
        .and_then(|repo| repo.is_dirty(StatusOptions::default()))
        .unwrap_or(false);
    (branches, dirty)
}
```

   并把 `mod tests` 里已有的 `use bytegit::Repo;`(若有)删掉,避免与上面的导入重复。

- [ ] **Step 5: 其余调用点**

1. `extensions/codehealth/git_hotspots.rs`:删掉 `use crate::delivery;`;`git_snapshot` 里这几行

   ```rust
       delivery::repo_root(project_root)?;
       let head = head_short_sha(project_root);
       let branch = delivery::branch(project_root);
       let dirty = delivery::is_dirty(project_root);
   ```

   替换为:

```rust
    // 先确认在某个仓库里(向上查找),分支/改动再按"项目路径就是仓库根"取——项目在仓库子目录
    // 时它们是 `None`/`false`(迁移前就如此,保持原样)。
    Repo::discover_workdir(project_root).ok()?;
    let head = head_short_sha(project_root);
    let exact = Repo::open_exact(project_root).ok();
    let branch = exact
        .as_ref()
        .and_then(|repo| repo.head().ok())
        .and_then(|head| head.branch);
    let dirty = exact
        .as_ref()
        .is_some_and(|repo| repo.is_dirty(StatusOptions::default()).unwrap_or(false));
```

   `head_short_sha` 里的 `delivery::open_exact(dir)?.head().ok()?.commit?` 改为 `Repo::open_exact(dir).ok()?.head().ok()?.commit?`(文档注释里的"见 `delivery::open_exact`"改成"见 `Repo::open_exact`")。
2. `chrome/homespace.rs`:把 `use crate::delivery;` 删掉(保留 Task 4 加的 `use crate::extensions::files::git_status;`);把

   ```rust
            if let Some(repo) = delivery::repo_root(&cwd) {
                for (path, _status) in git_status::file_statuses(&repo) {
   ```

   替换为:

```rust
            if let Ok(repo) = bytegit::Repo::discover_workdir(&cwd) {
                for (path, _status) in git_status::file_statuses(repo.root()) {
```

3. `extensions/project_create.rs`:删掉 `use crate::delivery;`;`delivery::git_available()` → `bytegit::git_available()`;`crate::delivery::clone_repo(&url2, &target2)` 的那个 `spawn_blocking` 替换为:

```rust
tokio::task::spawn_blocking(move || {
    bytegit::clone(&url2, &target2, bytegit::CloneOptions::default())
        .map(|_| ())
        .map_err(|e| e.message().to_string())
})
```

4. `extensions/project/scaffold.rs`:把

   ```rust
       match crate::delivery::init_repo(repo) {
           Ok(()) => ScaffoldStepResult::Created("已初始化 git 仓库".into()),
           Err(e) => ScaffoldStepResult::Failed(e),
       }
   ```

   替换为:

   ```rust
       match bytegit::init(repo) {
           Ok(_) => ScaffoldStepResult::Created("已初始化 git 仓库".into()),
           Err(e) => ScaffoldStepResult::Failed(e.message().to_string()),
       }
   ```

5. `app/update.rs`:把 `crate::delivery::checkout_branch(&repo_path2, &name)` 替换为

```rust
bytegit::Repo::discover(&repo_path2)
    .and_then(|repo| repo.checkout_branch(&name))
    .map_err(|e| e.message().to_string())
```

- [ ] **Step 6: 删除 `delivery.rs`、去掉 `git2`、清理过期注释**

1. 删除 `crates/dozer-app/src/delivery.rs`(`git rm`),并删掉 `crates/dozer-app/src/main.rs` 里的 `mod delivery;`。
2. `crates/dozer-app/Cargo.toml`:删掉 `git2 = "0.21"` 那一行;把那段注释

   ```toml
   # git2 = git 数据层(git_log 提交图 + delivery 状态查询 + worktree 列表),
   # 依赖版本与 gleisbau 传递依赖对齐,避免 cargo 拉两份。
   ```

   替换为

   ```toml
   # git 读写统一走 bytegit(见上);这里不直接依赖 git2——gleisbau 与 bytegit 都要求 `git2 = "0.21"`
   # (无默认 feature),cargo 会统一成同一份。
   ```

3. 清理仍提到 `delivery` 的注释(`grep -rn 'delivery' --include='*.rs' crates | grep -v delivery_checked` 应只剩下面这些,逐条改):
   - `workspace/state.rs`:`来自 `delivery::branch`/`delivery::is_dirty`。` → `来自 bytegit 的 `Repo::head`(分支)与 `Repo::is_dirty`。`
   - `extensions/git_log.rs`:所有 `` `delivery::local_branches` `` → `` `bytegit::Repo::local_branches` ``,`` `delivery::is_dirty` `` → `` `bytegit::Repo::is_dirty` ``;`mkrepo_with_one_commit` 文档注释里"同 `delivery.rs`\n    /// 的 `mkrepo` 惯例(用真 `git` CLI,不手搓 git2 底层对象)。"改成"(用真 `git` CLI)。"
   - `extensions/project.rs`:`` `delivery::remote_url` `` → `` `bytegit::Repo::remotes`,去重后 ``
   - `extensions/files/update.rs`:`(交付层的 git 调用都是同步阻塞,见 delivery.rs 模块头注释)` → `(bytegit 的调用都是同步阻塞的)`
   - `app/update.rs` 里的 `delivery_checked` 是另一回事(验收闭环的旧字段),**不动**。

- [ ] **Step 7: 编译、测试与门禁**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p6 && cargo fmt -p dozer-app && git status --short && cargo build -p dozer-app -p dozerd 2>&1 | grep -E "^(warning|error)" -A7`
Expected: `git status` 只有本任务列出的文件;无 error;警告只有"已知基线"里与本计划无关的两条。

Run: `cargo test -p dozer-app 2>&1 | grep -E "FAILED|test result"; cargo test -p dozerd 2>&1 | grep "test result"`
Expected: `dozer-app` 通过数比 Task 4 之后**少 32 个**(删除的 `delivery.rs` 测试,去向见"测试去向"),失败只有"已知基线"里的;`dozerd` 全部通过。

Run: `cargo machete`
Expected: `cargo-machete didn't find any unused dependencies in this directory. Good job!`(基线上它报 `dozer-app` 的 `git2`)。

Run: `grep -c '^name = "git2"' Cargo.lock`
Expected: `1`。

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" -A4 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c > /tmp/clippy.after.txt`,在 Task 3 之前的 `main` 上同样跑一次存成 `/tmp/clippy.before.txt`,`diff /tmp/clippy.before.txt /tmp/clippy.after.txt`。
Expected: 逐文件诊断条数一致(`delivery.rs`、`git_status.rs` 的条目随文件变化可以不同,但总数不增)。

Run: `grep -rn 'delivery' --include='*.rs' crates | grep -v delivery_checked; grep -rn 'git2::' --include='*.rs' crates`
Expected: 前一条只剩 `git_status.rs` 模块文档里"原先在 `delivery.rs`"那一句;后一条无输出。

- [ ] **Step 8: 变异检验(dozer 侧)**

先 `git add` 暂存本任务全部改动;逐个做、逐个用 `git checkout -- <文件>` 还原:

1. `workspace/state.rs` 的 `project_git_snapshot` 里把 `bytegit::Repo::open_exact(repo_path).ok()` 改成 `bytegit::Repo::discover(repo_path).ok()`。Run: `cargo test -p dozer-app workspace::state::git_snapshot_tests`。Expected: `project_git_snapshot_in_a_repo_subdirectory_only_finds_remotes` 失败。
2. `git_log.rs` 的 `load_branch_picker_data` 里把 `Repo::open_exact(repo_path)` 改成 `Repo::discover(repo_path)`。Run: `cargo test -p dozer-app extensions::git_log::tests::branch_picker`。Expected: `branch_picker_data_in_a_repo_subdirectory_finds_branches_but_never_dirtiness` 失败。
3. `files/update.rs` 的 `load_git_info` 里把 `bytegit::Repo::discover_workdir(root)` 改成 `bytegit::Repo::discover(root)`。Run: `cargo test -p dozer-app extensions::files::update::git_info_tests`。Expected: `a_file_path_is_not_a_repo` 失败。
4. `workspace/state.rs` 的 `workspace_git_info` 里把 `bytegit::Repo::discover_workdir(cwd)` 改成 `bytegit::Repo::discover(cwd)`。Run: `cargo test -p dozer-app workspace::state::git_snapshot_tests`。Expected: `workspace_git_info_is_none_outside_a_repo_and_for_a_file_path` 失败。

- [ ] **Step 9: Commit**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p6
git status --short
git add Cargo.lock crates/dozer-app/Cargo.toml crates/dozerd/Cargo.toml crates/dozer-app/src
git rm -q crates/dozer-app/src/delivery.rs 2>/dev/null || true
git diff --cached --stat
git commit -m "refactor(git): call bytegit directly, delete delivery.rs, drop direct git2 dependency

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

Expected: `git diff --cached --stat` 里没有 `.cargo/config.toml`,且包含 `delivery.rs` 的删除。

---

### Task 6: 文档同步与收尾

**Files:**
- Modify(worktree 内): `CLAUDE.md`、`docs/superpowers/specs/2026-10-02-bytegit-design.md`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`、`docs/dozer-v2/bytegit-调用点盘点.md`
- Modify(记忆): `~/.claude/projects/-Users-chrischiang-Projects-CoralProjects-byteboy-dozer/memory/dozer-v2-panel-independence-and-bytegit.md`

**Interfaces:** 无代码接口。

- [ ] **Step 1: `CLAUDE.md` 的 bytegit 条目定稿**

把 bytegit 条目改写为迁移完成后的终稿(保留已有的仓库/tag/`[patch]` 规则与"不要直接写 `Command::new("git")` 或 `git2::`"那句):

- 全部迁移完成(P1–P6):`delivery.rs` 已删除;生产代码里直接调用命令行 `git` 的只剩 `bytegit` 内部的 `clone`/`checkout_branch`(评估后的有意选择,见 `docs/superpowers/specs/2026-10-03-bytegit-write-ops-evaluation.md`)。
- **按语义选构造函数:** `Repo::open_exact`(项目路径就是仓库根才认)、`Repo::discover`(向上查找)、`Repo::discover_workdir`(目录、非 bare、向上查找);各调用点用哪个见规格 §8 O9 的口径清单——**同一个"项目在仓库子目录里"的场景,不同面板口径不同是迁移前就有的,未统一(待用户裁决)**,新代码不要想当然选一个。
- 文件树的着色语义在 `extensions/files/git_status.rs`(Dozer 自己的呈现语义,不是 git 查询)。
- `dozer-app` **不直接依赖 `git2`**:`gleisbau` 与 `bytegit` 都要求 `git2 = "0.21"`(无默认 feature),cargo 统一成同一份;升级 `git2` 时 `bytegit` 与 `gleisbau` 一起看。
- 已有的"`dozerd` 也依赖 `bytegit`"保留。

若条目里还没有 P1–P5 的措辞(对应分支尚未合并),只改与 P6 相关的部分,不要替它们写。

- [ ] **Step 2: 规格同步**

在 `docs/superpowers/specs/2026-10-02-bytegit-design.md`:

1. §4.1:增加 `Repo::open_exact(path)`(只认仓库根,否则 `NotARepo`,文案 `不是 git 仓库的根目录: <路径>`;比较前规范化)与 `Repo::discover_workdir(dir)`(`dir` 必须是目录,否则 `Io`;bare 仓库返回 `NotARepo`),并说明为什么要显式提供(O9)。
2. §6 P6 一行标为完成:验收补充"`cargo machete` 干净、`Cargo.lock` 里 `git2` 仍只有一份、`delivery.rs` 的 42 个测试的去向见 P6 计划"。
3. §8:
   - **O6** 标为已完成(25 个 `delivery::*` 调用点逐个核对并迁移);
   - **O9** 改写为"已显式化、未统一":把本计划"口径清单"那张表原样写进来,并附上待决事项 D14 的建议(状态类查询可统一为向上查找;历史类需先把 `file_path` 换算成相对仓库根)。

- [ ] **Step 3: 要求文档、盘点文档、记忆**

- `docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` §7 第 1 条后追加:`bytegit` P6 已完成,面板对 Git 的依赖收敛为 `bytegit` 类型与 `Repo` 句柄,`delivery.rs` 已删除,`dozer-app` 不再直接依赖 `git2`。
- `docs/dozer-v2/bytegit-调用点盘点.md`:在 §1 末尾加一行"P6 之后,盘点里的全部生产调用点都已迁移或明确留在 bytegit 内部(`clone`/`checkout_branch` 的命令行实现);`delivery.rs` 已删除";并 `grep -n "delivery" docs/dozer-v2/bytegit-调用点盘点.md`,把命中的条目标注为"P6 已拆除"。
- 记忆文件追加 P6 结果与待决事项 D14,并写明整个 bytegit 迁移(P0–P6)的收官状态与各分支合并现状。

- [ ] **Step 4: Commit(worktree)**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytegit-p6
git status --short
git add CLAUDE.md docs/superpowers/specs/2026-10-02-bytegit-design.md docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md docs/dozer-v2/bytegit-调用点盘点.md
git diff --cached --stat
git commit -m "docs(bytegit): record P6 results and make the exact-vs-upward lookup semantics explicit

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 5: 交给用户合并**

不要自己合并 `bytegit-p6` 到 `main`:向用户报告分支、提交列表、"已知基线"里的失败、**待决事项 D14(请用户决定是否排期统一口径)**,并请用户审阅后决定合并方式。合并后在 `main` 上重跑 `cargo test -p dozer-app` 与 `cargo machete`,并确认两条 `git_log` 的 `gleisbau` 测试在 `main` 上通过。合并前请用户在真实项目上手点一遍:① 打开一个普通项目,看文件树着色(改动/新增/未跟踪/忽略)、分支栏、Project 面板的分支与"有改动"、首页最近文件;② 打开一个**位于仓库子目录里**的项目,确认各面板的表现与迁移前一致(包括那些"不一致"之处);③ 切分支、初始化仓库、从 URL 克隆各一次。**这些交互没有自动化覆盖,只靠编译与上面的测试。**

---

## Self-Review

**Spec coverage(规格 §6 的 P6 行):**
- "`delivery.rs` 不再含 git 逻辑(只剩交付语义,或整体改名/删除)":Task 4 把着色语义搬到 `files/git_status.rs`(它是呈现语义,不是 git 查询),Task 5 删除 `delivery.rs`。
- "`dozer-app/Cargo.toml` 去掉直接的 `git2` 依赖(保留 `gleisbau` 的传递依赖对齐)":Task 5 Step 6;对齐靠 `gleisbau` 与 `bytegit` 都要求 `git2 = "0.21"`(无默认 feature),验收是 `Cargo.lock` 里 `git2` 仍只有一份。
- "`CLAUDE.md` 增补 bytegit 条目":Task 6 Step 1 把它定稿。
- 验收"`cargo machete`、clippy、全量测试":Task 5 Step 7 逐条列出,并与基线对比。
- 规格 O6(25 个调用点逐个核对)与 O9(只认仓库根 vs 向上查找):Task 3–5 逐个迁移、口径清单显式化,O9 的统一留作 D14。

**Placeholder scan:** 无 TBD/TODO;每个代码步骤都给了完整代码或"旧文本 → 新文本"的精确替换;搬走的代码用"从哪个锚点到哪个锚点原样剪切"描述,并列出要搬的测试名。

**Type consistency:** `Repo::open_exact`/`Repo::discover_workdir` 在 Task 1 定义,Task 4、5 使用同名同形;Task 3 抽出的四个函数签名在 Task 5 保持不变;`files::git_status::{FileGitStatus, file_statuses}` 在 Task 4 定义、Task 5 的 `project_git_snapshot` 与首页最近文件使用。

**Review Focus:** 五条各自有对应测试(分别见各条所指任务),其中 bytegit 侧 3 个、dozer 侧 4 个变异检验已在草稿里实际确认会让指定测试失败;"测试去向"表把被删除的 32 个测试逐个交代了去处;交互层面没有自动化覆盖,Task 6 Step 5 列了人工验收清单。
