# bytegit 调用点盘点

> 性质：现状盘点，为 `bytegit` 的 API 清单提供依据，**不是设计**。
> 日期：2026-10-02。来源：`crates/` 下 `Command::new("git")`、`git2::` 的逐处核对，加函数级阅读。
> 方法与局限：按函数边界归类；`app/`、`workspace/` 对 `delivery::*` 的 25 个调用点只统计了数量与文件，没有逐个读上下文；"GUI 是否都经 `spawn_blocking`"只确认了 `git_log`/`file_history` 两处，其余未核。

## 1. 总览

| 维度 | 数据 |
|------|------|
| 命令行 `git` 调用 | 30 处：**生产 10 处**，测试夹具 20 处 |
| `git2` | 仅 `dozer-app` 依赖；生产用于 `git_log`、`file_history`、`delivery`、`git_hotspots`、`usage` |
| 其他 git 相关依赖 | `gleisbau`（提交图布局，`git_log` 使用）、`notify`（`git_watch` 使用） |
| 写操作 | **只有 4 个**：`init`、`clone`、`checkout`、`rollback_to`（文件级还原） |
| 没有的操作 | commit、stage、push、pull、fetch、merge、stash、**git worktree**——代码里一处都没有（`Cargo.toml` 注释提到 worktree 列表，但没有对应实现） |
| 阻塞模型 | 全部同步；GUI 侧用 `tokio::task::spawn_blocking` 包（已核 `git_log`、`file_history`） |

**结论一：生产代码几乎是只读的。** `bytegit` 第一版可以很小；`dozer-v2架构分析.md` §16.4 的 Worktree Manager、agent 分支隔离需要的 worktree/commit/merge 能力，现在**完全不存在**，是纯新增，不是迁移。

## 2. 按操作归类

### 2.1 仓库发现与可用性

| 操作 | 位置 | 实现 |
|------|------|------|
| 找仓库根 | `dozerd/projects.rs:36` `git_repo_root` | CLI `rev-parse --show-toplevel` |
| 找仓库（含子目录） | `usage/mod.rs:201、218` | `git2 Repository::discover` |
| 打开仓库 | `delivery.rs` 多处、`git_log.rs`、`file_history.rs`、`git_hotspots.rs` | `git2 Repository::open`（只认仓库根） |
| 判断 git 是否可用 | `delivery.rs:369` `git_available` | CLI `--version` |

同一件事三种做法，且 `open` 与 `discover` 对"项目在仓库子目录"的处理不一致（`usage` 注释特意说明了这点）。

### 2.2 HEAD / 分支 / 远程信息

| 操作 | 位置 | 实现 |
|------|------|------|
| 当前分支名 | `delivery::branch` | git2 `head().shorthand()` |
| HEAD 短 sha | `git_hotspots::head_short_sha` | git2 |
| 本地分支列表 | `delivery::local_branches` | CLI `for-each-ref`（经 `delivery::git` 辅助函数） |
| 当前分支有无提交 | `delivery::current_branch_has_commits` | git2 |
| 远程 URL | `delivery::remote_url` | CLI `remote -v` |
| HEAD 提交时间 | `dozerd/projects.rs:55` `git_head_commit_ms` | CLI `log -1 --format=%ct` |

注意：`dozerd/code_health.rs` 存的 `git_head`/`git_branch`/`git_dirty` 是 **GUI 经协议传来的值**（GUI 用 `delivery::*` 算好再上报），不是 daemon 自己算的；daemon 自己只算项目"更新时间"用的最新提交时间。

### 2.3 工作区状态

| 操作 | 位置 | 实现 |
|------|------|------|
| 是否 dirty | `delivery::is_dirty` | git2 |
| 每文件状态（供文件树着色） | `delivery::file_statuses` → `FileGitStatus` | git2 `statuses`，映射 INDEX_*/WT_*/IGNORED 标志位 |
| 变更路径列表 | `git_hotspots::dirty_paths` | CLI `status --porcelain --untracked-files=all` |

**`is_dirty`、`file_statuses`、`dirty_paths` 是三份"工作区状态"实现**，一份 git2、一份 git2、一份 CLI，口径（是否含 untracked、ignored）是否一致没有核对。

### 2.4 历史与 diff（读）

| 操作 | 位置 | 实现 |
|------|------|------|
| 全仓库提交图 | `git_log::build(repo, max_count)` → `GitLogSnapshot` | **`gleisbau`**（含图布局）+ git2 |
| 单个提交详情（文件列表） | `git_log::commit_detail` | git2 `diff_tree_to_tree` |
| 两个 blob 的文本内容 | `git_log::diff_blob_content` / `read_side` → `DiffBlobContent` | git2 `find_blob` |
| 内容分类（二进制/过大/文本） | `git_log::classify_diff_bytes` | 纯函数，`file_history` 借用 |
| 单文件历史 | `file_history::build(repo, file, max_count)` | git2 `revwalk` + 路径过滤 |
| 与当前工作区对比 | `file_history::diff_against_current`、`diff_blob_content_against_workdir` | git2 `diff_tree_to_workdir` |
| 上一版本 | `file_history::previous_oid` | 复用 `build` 取前两条 |
| 提交计数（总/按天） | `usage::count_git_commits`、`count_git_commits_by_day` | git2 `revwalk` |
| 近 30 天文件变更频次 | `git_hotspots::recent_churn` | CLI `log --since=30.days --name-only` |

重复与泄漏：
- `git_log` 与 `file_history` 各实现一份"取 blob + 分类 + 截断"，靠注释保持一致（`MAX_PATCH_CHARS`、根提交按空树对比、`DEFAULT_MAX_COMMITS` 量级）。
- **`git2::Oid` 和 `git2::Delta` 已进入 UI 状态与视图代码**（`file_history` 里 `Oid` 约 26 处，`git_log` 约 18 处；`git_log.rs:308、1724` 的 `FileFilter::matches`、`status_glyph` 直接用 `git2::Delta`）。`bytegit` 的公开 API 若暴露 `git2` 类型，所有调用方会被锁死在 `git2` 版本上。

### 2.5 写操作

| 操作 | 位置 | 实现 | 调用方 |
|------|------|------|--------|
| 初始化仓库 | `delivery::init_repo` | CLI `init` | `project_create`、`project/scaffold` |
| 克隆 | `delivery::clone_repo(url, dest)` | CLI `clone --` | `project_create` |
| 切换分支 | `delivery::checkout_branch` | CLI `checkout` | `git_log`（分支选择器） |
| 文件还原到某提交 | `file_history::rollback_to` | git2 读 blob + `std::fs::write` | `file_history`、文件树右键「回滚」 |

`rollback_to` 严格说不是 git 写操作，而是"读 blob 再写文件"；它放在 `file_history` 里，属于"取某版本文件内容"的上层动作，归属需要决定（见 §5）。

### 2.6 变更监听

`git_watch::start(handle, repo, debounce, on_change)`：基于 `notify`，对 `.git/HEAD`、`index`、`packed-refs`、`refs/*` 放行为 `GitRefs`，其余为 `Workdir`，并按 `project::HIDDEN` 过滤 `node_modules`、`target` 等（含嵌套层，注释里提到是 code review 发现的问题）。

耦合点：**依赖 `crate::project::HIDDEN`**（文件树的隐藏名单），把 git 监听和文件树的概念绑在了一起。这正是将来事件总线的"Git 状态变化"生产者。

### 2.7 与 git 无关、但名字里有 git

`git_accounts.rs`：`GitProvider`、`RemoteRepo`、`parse_repo_list(provider, json)`，是对托管平台（GitHub/GitLab 一类）API 的账户与仓库列表，**不是本地 git 操作**。是否属于 `bytegit` 要另行判断，倾向不属于（它是"远程托管服务"，与"本地仓库底层"不同层）。

## 3. 调用方（谁在用 `delivery::*`）

`delivery::` 的函数在 `app/update.rs`(5)、`workspace/state.rs`(7)、`git_log.rs`(4)、`files/update.rs`(2)、`git_hotspots.rs`(2)、`project_create.rs`(2)、`homespace.rs`(1)、`project.rs`(1)、`project/scaffold.rs`(1) 共约 25 处被调用。Host 层（`app/`、`workspace/`）已直接依赖 `delivery`，所以 `delivery` 现实中就是一个"准 Git 底层"，只是名字叫"交付"、放在 `dozer-app` 根下。

## 4. 对 bytegit API 的启示

1. **第一版 API 可以按 §2 的 5 组操作划分**，且大部分是只读：仓库发现、HEAD/分支/远程、工作区状态、历史与 diff、少量写、监听。
2. **重复实现要在 `bytegit` 里合并：** 工作区状态 ×3、blob+分类 ×2、仓库打开 ×3、HEAD 提交时间 ×2（daemon CLI 与 usage git2）。合并时必须先确认口径（untracked/ignored 是否计入、`open` 还是 `discover`）。
3. **公开 API 不能泄漏 `git2` 类型：** 需要 `bytegit` 自己的 `CommitId`、`ChangeKind` 等，替换现在进入 UI 的 `Oid`、`Delta`。
4. **读写分明：** 4 个写操作都很简单，但 CLI 实现意味着依赖用户机器上的 `git` 可执行文件；`git2` 能否覆盖 `init`/`clone`/`checkout`（`clone` 的认证与进度、`checkout` 的冲突处理）需要评估。
5. **GUI 友好性：** 提交图布局（`gleisbau`）是否进 `bytegit`，决定 `bytegit` 是"纯 git 数据层"还是"含图布局"。图布局是 `git_log` 视图的需要，Digger 未必用，倾向**不进核心**，作为可选 feature 或留在 `git_log` 面板。
6. **监听与 `HIDDEN` 解耦：** `bytegit` 的 watch 需要自己的忽略规则参数，不依赖文件树的 `HIDDEN`。
7. **全是同步阻塞 API：** `bytegit` 提供同步 API，异步包装交给调用方（现状如此，沿用）。
8. **Worktree 与写操作是纯新增：** agent 隔离、分支合并等不在现状里，应作为 v2 新需求单独设计，不要混进"迁移现有调用点"。

## 5. 边界问题（2026-10-02 用户裁决：B1–B6 均按倾向，设计见 `docs/superpowers/specs/2026-10-02-bytegit-design.md`）

| # | 问题 | 倾向 |
|---|------|------|
| B1 | `gleisbau` 提交图布局是否进 `bytegit` | 不进核心，可选 feature 或留在面板 |
| B2 | `rollback_to`（还原文件内容）归 `bytegit` 吗 | `bytegit` 只提供"读取某提交的文件内容"，写回文件由调用方做 |
| B3 | `git_accounts`（托管平台账户/远程仓库列表）归 `bytegit` 吗 | 不归，属于另一层 |
| B4 | 写操作用 `git2` 还是继续 CLI | 需要对 `clone`（认证/进度）、`checkout` 评估后定；读操作统一 `git2` |
| B5 | `git_watch` 进 `bytegit` 还是作为事件总线生产者留在 Host | 监听逻辑（`.git` 相关路径识别）进 `bytegit`，事件发布由 Host 做 |
| B6 | `dozerd` 是否也依赖 `bytegit` | 是，替换 `projects.rs` 的两处 CLI 调用 |
