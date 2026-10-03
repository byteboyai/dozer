# bytegit 设计规格

> 状态：**草案，待用户审阅**。日期：2026-10-02。
> 依据：`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`（§2.5、§2.6）、`docs/dozer-v2/bytegit-调用点盘点.md`（B1–B6 已按盘点文档的倾向裁决）。
> 本规格只定义 `bytegit` 本身与迁移顺序，不含事件总线、面板注册制的设计（见要求文档 Q8、Q9）。

## 1. 目标与非目标

**目标**

1. 提供 byteboy 体系统一的本地 Git 底层：Dozer、Digger 及后续产品共用，`dozer-app`、`dozerd` 都依赖它。
2. 消除现有约 180 个 git 调用点里的重复实现（工作区状态 ×3、blob 读取与分类 ×2、仓库打开 ×3、HEAD 提交时间 ×2）。
3. 公开 API 不暴露 `git2` 类型，调用方不被 `git2` 版本锁死。
4. 让 `file_history`、`git_log`、`files` 等面板对 Git 的依赖收敛为「调用 bytegit + 呈现」，满足「一个面板 + Host 即可运行」。

**非目标（v0.x）**

- 不含任何 UI、`iced`、`tokio` 依赖。API 全部同步，异步包装由调用方做（沿用现状的 `spawn_blocking`）。
- 不含托管平台账户与远程仓库列表（`git_accounts`，B3）。
- 不含提交图布局（`gleisbau`，B1）：v0.1 留在 `git_log` 面板，Digger 需要时再评估做成可选 feature。
- 不含 commit、stage、push、pull、fetch、merge、stash、**worktree**。现状生产代码里这些能力为零，属 v2 新需求，另行设计；`bytegit` 的 `Repo` 句柄设计保证以后加方法是纯增量。
- 不实现 git 协议或对象存储，底层是 `git2`（libgit2）。

## 2. 仓库与形态

- **独立仓库 `byteboyai/bytegit`**，与 `byteui` 同样的发版方式：消费方用 `bytegit = { git = "...", tag = "vX.Y.Z" }`，一律用 tag；本地联调用 `.cargo/config.toml` 的 `[patch."https://github.com/byteboyai/bytegit"] bytegit = { path = "../bytegit" }`，不提交。
- 单 crate，库形态。Cargo features：`watch`（默认关，引入 `notify`）、`testutil`（默认关，导出测试夹具构建器）。
- `git2` 版本必须与 `gleisbau` 的传递依赖对齐（现状注释已要求，避免 cargo 拉两份）；升级时 `dozer-app` 与 `bytegit` 一起升。
- 许可证与 MSRV 与 byteui 保持一致（沿用，不单独决策）。

## 3. 设计原则

1. **一个入口句柄 `Repo`。** 所有操作是 `Repo` 的方法，持有 `git2::Repository`，不对外暴露。
2. **自有类型，不泄漏后端。** 公开类型只有 bytegit 自己定义的 `CommitId`、`ChangeKind` 等；`git2::Oid`、`git2::Delta` 不得出现在公开签名里。
3. **全部同步、全部可在 `spawn_blocking` 里调用。** `Repo` 是 `Send`；不保证 `Sync`，跨线程各自 `discover`。
4. **失败是类型化的。** 返回 `Result<_, GitError>`；`GitError` 是带 `kind()`（`GitErrorKind`，稳定分类）与适合直接展示的 `message()` 的结构体（迁移期调用方多数用 `Result<_, String>`，用 `.to_string()` 即可过渡）。
5. **只读操作不碰磁盘。** 写操作显式命名（`init`、`clone`、`checkout_branch`），读操作不得有副作用。
6. **口径显式化。** 现有实现里口径不一致的地方（untracked/ignored 是否计入、`open` 与 `discover`）一律变成参数或明确的默认值，写进文档与测试。

## 4. 公开 API（v0.1）

### 4.1 打开与可用性

```text
Repo::discover(path) -> Result<Repo, GitError>   // 向上查找，支持项目位于仓库子目录
Repo::root(&self) -> &Path                        // 仓库工作区根目录
Repo::is_bare / is_empty (无提交)
```

- 默认只提供 `discover`。现有 `open`（只认仓库根）的调用点，其传入路径本就是仓库根，用 `discover` 结果一致；若存在传入子目录并依赖「不是仓库根就失败」的调用点，迁移时逐个确认，必要时再加 `Repo::open_exact`。
- **P0 实测补充：** `discover` 对不存在的路径返回 `Io`（libgit2 原本报 NotFound，会被误归为 `RefNotFound`）；`root()` 已去掉尾部分隔符，但是 libgit2 解析后的真实路径（macOS 上 `/var/...` 会变成 `/private/var/...`），调用方与自己持有的项目路径比较前要先规范化；bare 仓库的 `root()` 是 git 目录。
- **`is_empty` 语义：仓库里没有任何引用（即还没有提交）。** HEAD 指向未诞生分支但其他分支有提交（如 `git checkout --orphan` 之后）时为 `false`。不直接用 `git2::Repository::is_empty`，因为它对"空"的判断依赖用户全局配置的 `init.defaultBranch`，同一个空仓库在不同机器上结果可能不同。
- **`TempRepo` 隔离全局 git 配置：** libgit2 会读 `~/.gitconfig`，`core.autocrlf` 等会改变 blob 内容与提交 id；夹具在仓库级配置里固定 `core.autocrlf=false`、`core.eol=lf`、`core.safecrlf=false` 与空的 `core.attributesFile`，才能保证"同样操作得到同样 id"。
- **git2 0.21 的 API 变化：** `Reference::shorthand()`、`Remote::url()` 返回 `Result<&str, Error>`，不再是 `Option<&str>`（P1 迁移 `delivery.rs` 时会碰到）。
- 无 `git_available()`：读操作不依赖 `git` 可执行文件。仅 `clone` 若保留命令行实现，才需要检测（见 §4.6）。

### 4.2 HEAD、分支、远程

```text
head(&self) -> Result<HeadInfo, GitError>
    HeadInfo { branch: Option<String> /* None = detached */, commit: Option<CommitId> }
head_commit_time(&self) -> Result<Option<SystemTime>, GitError>   // 空仓库为 None
local_branches(&self) -> Result<Vec<String>, GitError>            // 升序
remotes(&self) -> Result<Vec<Remote>, GitError>                   // 升序、按 name 去重
    Remote { name: String, url: String, push_url: Option<String> }
```

- `HeadInfo` 没有 `short_id`/`has_commits` 字段；少/多提交由 `commit: Option<CommitId>` 表达，调用方需要"是否有提交"时用 `head()?.commit.is_some()`（`has_commits()` 方法可另加，P1 未实现）。`CommitId::short(n)` 提供短 id。
- `Remote` 没有 `kind` 字段（`Fetch | Push` 未实现）；取不到 url 时该远程仍会出现，`url` 为空串。`push_url` 缺失或与 `url` 相同则为 `None`。

替代：`delivery::branch`、`current_branch_has_commits`、`local_branches`、`remote_url`，`git_hotspots::head_short_sha`，`dozerd::git_head_commit_ms`。

### 4.3 工作区状态

```text
status(&self, opts: StatusOptions) -> Result<Vec<StatusEntry>, GitError>
    StatusOptions { include_untracked: bool, include_ignored: bool, detect_renames: bool }
    StatusEntry { path: PathBuf /* 相对仓库根 */, state: FileState }
    FileState { index: Option<ChangeKind>, worktree: Option<ChangeKind>, conflicted: bool, ignored: bool, untracked: bool }
is_dirty(&self, opts) -> Result<bool, GitError>
```

- `StatusOptions` 没有 `include_untracked_files_in_dirs`：现有三份实现都递归进未跟踪目录，没有"不递归"的使用者，该开关无意义（P1 未实现）。
- `FileState` 与现有 `delivery::FileGitStatus` 语义等价，多个 `conflicted` 字段（合并冲突标记）；迁移时逐变体对照（P1 的验收项）。
- `dirty_paths`（命令行 `status --porcelain --untracked-files=all`）等价于 `status` 带 `include_untracked + detect_renames` 后取路径，不再另做。
- **P1 已核对的口径（O1）：** `is_dirty` 含未跟踪、不含被忽略；`file_statuses` 含未跟踪与被忽略；`current_branch_has_commits` 在 detached HEAD 时为 `true`。
- **实现细节：** 重命名时 libgit2 的 `entry.path()` 返回旧路径，实现改从 diff delta 取新路径；被忽略的目录只作为一个带尾部 `/` 的条目；路径不是 UTF-8 的条目被跳过。

### 4.4 历史与 diff

```text
log(&self, opts: LogOptions) -> Result<Vec<CommitSummary>, GitError>
    LogOptions { max_count: usize, path: Option<PathBuf>, since: Option<SystemTime> }
    CommitSummary { id: CommitId, parents: Vec<CommitId>, author: Signature, time: SystemTime, summary: String, message: String }
commit_files(&self, id: CommitId) -> Result<Vec<FileChange>, GitError>        // 根提交按空树对比
    FileChange { path: PathBuf, old_path: Option<PathBuf>, kind: ChangeKind, old_blob: Option<BlobId>, new_blob: Option<BlobId> }
blob_text(&self, blob: BlobId, limits: ContentLimits) -> Result<Content, GitError>
file_at(&self, id: CommitId, path: &Path, limits: ContentLimits) -> Result<Option<Content>, GitError>
workdir_vs_commit(&self, id: CommitId, path: &Path, limits) -> Result<ContentPair, GitError>
previous_version(&self, path: &Path) -> Result<Option<CommitId>, GitError>   // 现 previous_oid 语义
commit_count(&self) -> Result<u64, GitError>
commit_count_by_day(&self) -> Result<BTreeMap<i64, u64>, GitError>
churn(&self, since: SystemTime) -> Result<HashMap<PathBuf, u32>, GitError>   // 现 recent_churn
```

- `Content` 是 `enum { Text(String), TooLarge{bytes}, Binary, NotUtf8 }`，**吸收现在 `git_log::classify_diff_bytes` 与 `DiffBlobContent`**，`git_log` 与 `file_history` 共用同一套截断阈值（`ContentLimits`），不再靠注释对齐。
- 回滚（B2）：`bytegit` 只提供 `file_at`（取某提交的文件内容）；**写回磁盘由调用方做**，`rollback_to` 留在 `file_history` 里，内部改为 `file_at` + `fs::write`。
- `churn` 与 `commit_count*` 现状分别是命令行与 git2 实现，统一为 git2。

### 4.5 变更监听（feature `watch`，B5）

```text
watch(root: &Path, opts: WatchOptions, on_change: impl FnMut(GitChange) + Send + 'static) -> Result<WatchHandle, GitError>
    WatchOptions { debounce: Duration, ignore: IgnoreRules }
    GitChange { refs_changed: bool, workdir_changed: bool, paths: Vec<PathBuf> }
```

- 使用 `notify` + 标准线程，不依赖 `tokio`；`WatchHandle` Drop 即停止。
- `.git` 下只对 `HEAD`、`index`、`packed-refs`、`refs/*` 视为 `refs_changed`，其余忽略（沿用现有规则）。
- **`IgnoreRules` 由调用方传入**，不再依赖文件树的 `project::HIDDEN`；包括「嵌套层同名目录也排除」的现有语义（`node_modules`、`target` 等）。Dozer 传入的是它自己的 `HIDDEN` 名单。
- **要处理 `.git` 是文件的情况**（worktree、子模块里 `.git` 是指向真实 gitdir 的文件）：现有 `git_watch` 只处理 `.git` 为目录，这是已知缺口，不在 v0.1 迁移范围内强求，但 API 要为之留位（监听目标应是解析后的 gitdir）。
- 事件发布不在 `bytegit`：Host 把 `GitChange` 映射为事件总线事件（Q9）。

### 4.6 写操作（B4）

```text
init(path) -> Result<Repo, GitError>
clone(url, dest, opts: CloneOptions) -> Result<Repo, GitError>
Repo::checkout_branch(&self, name) -> Result<(), GitError>
```

- 现状三者均为命令行实现。**v0.1 先保持行为不变**：这三个操作在 `bytegit` 内仍调用 `git` 可执行文件，但封装在 `bytegit` 内部，调用方看不到命令行细节；`git_available()` 因此保留为内部检测，失败时返回 `GitError::GitBinaryUnavailable`。
- **并行评估项（不阻塞 v0.1）：** `clone` 是否能用 `git2` 实现，取决于用户机器上的凭据助手与 ssh-agent 是否可被 libgit2 继承，以及进度回调；`checkout` 在有未提交修改时的冲突处理与命令行是否一致；`init` 最简单，评估通过即可先切换。评估结论决定写操作是否最终去掉对 `git` 可执行文件的依赖，写入本规格的修订。
- 读操作一律 `git2`。

### 4.7 测试夹具（feature `testutil`）

```text
TempRepo::new()                         // 临时目录 + git init，固定作者/时间，确定性 id
    .commit_file(path, content, msg)    // 写文件并提交
    .branch(name) / .checkout(name)
    .add_remote(name, url)
    .write_untracked(path, content)
```

用 `git2` 实现，不调用命令行，替换现有 20 处测试里的 `Command::new("git")`。因为夹具返回的是真实仓库，**面板单元测试不需要为 Git 做 mock/fake**，直接用 `TempRepo` 即可，这同时是「一个面板 + Host 可运行」对 Git 依赖的满足方式。

## 5. 类型清单

| 类型 | 说明 |
|------|------|
| `CommitId`, `BlobId` | 对象 id 的新类型，`Copy + Eq + Hash`，`Display`/`short(n)`、可序列化；内部不暴露 `git2::Oid` |
| `ChangeKind` | `Added/Modified/Deleted/Renamed/Copied/TypeChange`；替换 UI 里的 `git2::Delta` |
| `FileState` | 见 §4.3 |
| `Content` | 见 §4.4 |
| `GitError` | 结构体，`kind()` 返回 `GitErrorKind`：`NotARepo`、`RefNotFound`、`NoCommits`、`GitBinaryUnavailable`、`Io`、`Backend`；`message()` 返回可展示文本 |

`git_log.rs` 的 `FileFilter::matches(git2::Delta)` 与 `status_glyph(git2::Delta)` 迁移后改用 `ChangeKind`。

## 6. 迁移计划

每个阶段独立可合并、独立可验证，并且「**新旧并行比对**」：在阶段内先把调用点切到 `bytegit`，在测试里对同一个 `TempRepo` 同时调用旧实现与新实现断言结果一致（尤其是 §4.3 的口径），验证通过后删除旧实现。

| 阶段 | 内容 | 删除的旧代码 | 验收 |
|------|------|--------------|------|
| P0 | 建 `byteboyai/bytegit` 仓库骨架、CI、`TempRepo`、`CommitId`/`GitError`/`ChangeKind`、`Repo::discover`；发 `v0.1.0` | — | `cargo test`、clippy、fmt 通过 |
| P1 | HEAD/分支/远程、工作区状态；迁移 `delivery.rs` 的 `is_dirty`/`file_statuses`/`branch`/`remote_url`/`local_branches`/`current_branch_has_commits`、`git_hotspots::dirty_paths`/`head_short_sha`；**先核对并写清 §4.3 的口径** | 适配层随 P6 删除（`delivery.rs` 对应函数改成 bytegit 适配层，**签名不变**，调用点不动） | 并行比对测试；文件树着色、首页分支显示行为不变 |
| P2 | 历史与 diff；迁移 `git_log::{commit_detail, diff_blob_content, read_side, classify_diff_bytes}`、`file_history::*`、`rollback` 的取内容部分；`git_log` 与 `file_history` 不再互相引用，`file_history → git_log` 耦合消失 | `git_log` 里的 blob/分类代码、`file_history` 里的重复实现 | 两面板 diff 展示行为不变；`DiffBlobContent` 不再被 `file_history` 引用 |
| P3 | `usage` 的 `commit_count*`、`git_hotspots::recent_churn`、`dozerd/projects.rs` 的两处命令行；`dozerd` 加依赖 | 对应实现 | `usage`、项目更新时间、Code Health 热点结果不变 |
| P4 | `watch` feature；迁移 `git_watch`，`HIDDEN` 由调用方传入 | `git_watch.rs` 里的路径分类逻辑 | 现有 `git_watch` 测试迁移后通过 |
| P5 | 写操作（`init/clone/checkout_branch`），含 §4.6 评估结论 | `delivery.rs` 剩余的命令行函数 | 新建项目、克隆、分支切换行为不变 |
| P6 | 清理：`delivery.rs` 不再含 git 逻辑（只剩交付语义，或整体改名/删除）；`dozer-app/Cargo.toml` 去掉直接的 `git2` 依赖（保留 `gleisbau` 的传递依赖对齐）；`CLAUDE.md` 增补 bytegit 条目 | — | `cargo machete`、clippy、全量测试 |

- **版本节奏：** 每个阶段在 `bytegit` 发一个小版本，`dozer` 用 tag 引用；联调期间用本地 patch，不提交。
- **不得在迁移阶段改变用户可见行为。** 若比对发现旧实现有 bug，先在原样迁移后单独提交修复，不与迁移混在同一个提交里（便于回退）。
- **并发提醒：** 本仓库常有并发会话改 `main`，每个阶段开工与提交前先核对 staged 与 `git status` 全貌。

## 7. 与面板独立性要求的关系

- 面板对 Git 的依赖变为「依赖 `bytegit` 的类型与 `Repo` 句柄」，**面板之间不再互相引用**：P2 完成后 `file_history → git_log` 的唯一真实面板间耦合消失。
- Host 负责构造 `Repo`（按项目路径 `discover`）并把变更监听的 `GitChange` 发布到事件总线；面板订阅事件、调用 `bytegit`，不碰 `notify`。
- `bytegit` 的 `TempRepo` 夹具是 Git 依赖的「fake」，让单个面板可以在最小 Host 里跑测试。

## 8. 待定与风险

| # | 问题 | 处理 |
|---|------|------|
| O1 | §4.3 口径（untracked/ignored、`discover` vs `open`）现状不一致，且未读对应测试 | **P1 已完成**：`is_dirty` 含未跟踪、不含被忽略；`file_statuses` 含未跟踪与被忽略；`current_branch_has_commits` 在 detached HEAD 为 `true`。结论写入 §4.3 |
| O2 | 写操作 `git2` 化的可行性（`clone` 认证/进度、`checkout` 冲突） | §4.6 并行评估，结论回写本规格 |
| O3 | `.git` 为文件（worktree/子模块）的监听与读取 | v0.1 不强求，API 留位；v2 引入 worktree 时必须解决 |
| O4 | `git2`（libgit2）与命令行 git 在边角行为上的差异（如大仓库 `status` 性能、`.gitattributes`/filter、submodule、sparse checkout） | P1/P2 的并行比对测试覆盖现有用法；未覆盖的差异记为已知限制 |
| O5 | 大仓库性能：`log` 的 `max_count`、`churn` 的全历史扫描，`usage` 的全量 revwalk | 保持现有上限与调用方式，不在迁移中优化；另立项 |
| O6 | 25 处 `delivery::*` 调用方的逐个核对只做了数量统计 | **P1 已完成**：核对后决定不逐个改调用点，改为把 `delivery.rs` 保留为签名不变的 bytegit 适配层，随 P6 一并删除。 |
| O7 | `gleisbau` 与 `bytegit` 的 `git2` 版本对齐的长期维护 | 升级时同步升；若 `gleisbau` 成为阻碍再评估 B1 |
| O8 | Digger 是否需要提交图 | 不影响 v0.1；需要时单独设计 `graph` feature |
| O9 | 项目目录位于仓库子目录时，`Repo::discover`（向上查找）与 `delivery::open_exact`（只认仓库根）语义不一致；P1 为保持旧行为在 `delivery` 适配层用了 `open_exact` | 待用户裁决；统一前不要擅自把 `open_exact` 改成向上查找 |
| O10 | 同一子目录项目下 `git_hotspots::dirty_paths`（相对仓库根）与 `recent_churn`（`--relative`，相对项目根）路径口径不同 | 迁移期保持现状；统一口径另议（P3 迁移 `recent_churn` 时评估） |
