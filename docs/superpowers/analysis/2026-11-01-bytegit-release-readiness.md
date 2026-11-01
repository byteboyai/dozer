# bytegit / Dozer / Digger 发布就绪基线与验收记录

日期：2026-11-01。对应[交接计划](../plans/2026-11-01-bytegit-release-readiness-handoff.md)与[刷新链路规格](../specs/2026-10-31-bytegit-refresh-optimization-design.md)。

> 状态：进行中。本文先记录 G0 基线；后续每完成一个 agent 任务包，更新对应章节与证据。未执行的项明确写“未验证”，不以“代码已写”代替门禁。

## 1. G0 基线（2026-11-01）

### 1.1 仓库与工具链

| 仓库 | HEAD | 分支 | 远端 | 本地 patch / 关键声明 |
|---|---|---|---|---|
| bytegit | `8763899`（R3/R4 修复提交，基于 `4a4951b`） | main | `github.com/byteboyai/bytegit` | tag `v0.9.0` 指向 `4a4951b`；修复提交尚未发版 |
| Dozer | `f5bd3ac5` | main | `github.com/byteboyai/dozer` | `.cargo/config.toml` patch 到 `../bytegit`、`../byteui`；根 Cargo.toml 已集中声明 bytegit `v0.9.0` |
| Digger | `2b21b0f` | main | **未配置 remote** | `.cargo/config.toml` patch 到本地 Dozer 全部共享 crate 与 `../byteui`；仍声明 bytegit `v0.8.1`、Dozer libs `libs-2026-10-30a` |
| ByteUI | `541554f`（无 tag） | main | `github.com/byteboyai/byteui` | 最新 tag 仍是 `v0.5.0`；Digger 依赖本地未发布接口 |

工具链：`rustc 1.98.1`、`cargo 1.98.1`（macOS aarch64）。三仓均未发现 AGENTS.md；Dozer 使用本仓 CLAUDE.md 的边界与门禁说明。

### 1.2 工作树归属

- Dozer 未提交改动分两类：
  - 本重构相关：`Cargo.toml`/`Cargo.lock`、五个消费 crate 的 bytegit 依赖、`crates/dozer-git-runtime/`、2026-10-31 的 spec/plan/analysis/patches 文档。
  - 其它会话：`crates/dozer-core/src/agent_paths.rs`、`crates/dozerd/src/{headless_agent,summary_provider,transcripts/*}`。本任务不修改、不提交、不 reset/stash。
- bytegit 工作树：仅 `.dozer/` 未跟踪；R3/R4 修复已提交为 `8763899`。
- Digger / ByteUI：工作树干净（各自 `.dozer/` 未跟踪）。

### 1.3 已复现的测试基线

| 命令 | 结果 |
|---|---|
| bytegit `cargo test --locked` | 157 passed |
| bytegit `cargo test --locked --all-features` | 223 passed |
| bytegit `cargo clippy --locked --all-targets --all-features -- -D warnings` | 通过 |
| Dozer `cargo test --locked -p dozer-git-runtime` | 14 单测 + 15 集成 + 1 doctest 通过（含 R1/R2 回归） |
| Dozer `cargo test --locked -p dozer-panel-files -p dozer-panel-project -p dozer-panel-agent -p dozer-store-project` | 81 / 157 / 89 / 32 passed |
| Dozer `cargo check -p dozer-app` | 通过（25 个既有 warning） |
| Dozer `cargo tree -p dozer-git-runtime --depth 1` | 仅 bytegit / tokio |

未建立/未执行：两款 App 的完整 release 构建、Digger 任一构建门禁、无本地 patch 的锁文件重建、Linux/Windows 门禁、真实性能矩阵。

### 1.4 当前阻断

1. bytegit R3/R4 修复已提交 `8763899`，但尚未发布 tag；消费方无法在纯远端依赖下取得。v0.9.0 仍不包含这四项修复。
2. Dozer 运行路径尚未接入 `dozer-git-runtime`；Digger 仍用旧 bytegit v0.8.1，且没有 remote。
3. ByteUI 产品所需接口没有可获取 tag；两产品依赖本地 patch。
4. `check-shared-deps.sh --offline` 在本地 patch 存在时无法证明纯远端闭包。
5. 原计划 T2 剩余、T6–T13 未完成。

## 2. 任务包进度

| 任务包 | 状态 | 证据 / 剩余 |
|---|---|---|
| G0 | 完成（隔离 checkout 除外） | 本章 |
| A：bytegit 修复与共享依赖 | 进行中 | R3/R4 已落地 `8763899` 并通过 157/223 + clippy；T3 子项已明确边界；T12、ByteUI tag、共享版本发布材料未完成 |
| B：共享 runtime / 面板 / 存储 | 进行中 | T2 Files 纯映射、T6 Files/Project/Agent 纯入口、T9 summary + 有界缓存、T10 目录后台读取、T11 ProjectStore 锁外 Git 时间已完成；runtime 普通非 Git 目录由宿主兜底 |
| C：Dozer 宿主接入 | 进行中 | 共享 runtime 已接入 App 生命周期与 Files/Project/Agent 投影；Git Log 唯一 generation、目录异步读取/磁盘节流、首页最近文件、Usage 缓存、Codehealth 单次 HEAD 读取已完成；CLI 取消未做 |
| D：Digger 宿主接入 | 进行中 | runtime 已接入 ProductShell 生命周期与 Files/Project 投影；依赖仍为本地 patch，缺 GUI 手工证据 |
| E：纯远端闭包与最终验收 | 未开始 | — |

## 3. 本轮交付记录（2026-11-01）

### 3.1 Agent A（部分）

- 应用 `docs/patches/2026-10-31-bytegit-audit/bytegit-fixes.patch`，bytegit 提交 `8763899`：
  - `need_rescan()` 优先于 Access/路径过滤，强制 `FULL_RESCAN`。
  - linked worktree 增加共享 `info/`、`objects/info/` 非递归监听。
  - worker 持有 watcher，缺失目标重试、删除/重建后重新挂载并补扫，降级可见。
- 验证：`cargo fmt --check`、默认 157、all-features 223、all-features clippy `-D warnings` 通过。
- T3 边界：bytegit 只负责“身份失效 → FULL_RESCAN/degraded”；普通目录首次出现 `.git`、身份自动换 key 由 runtime/宿主重新订阅完成。缺失 `refs/` 由 worker 定期重试恢复。
- T12 严格 patch API 已落地（`workdir_patch_limited`，file_history 已迁移）；CLI clone/checkout 的可取消进程树回收仍未做。
- 未完成：bytegit 修复 tag；ByteUI tag。

### 3.2 Agent B（部分）

- Files `git_status::file_statuses_from_entries(project, identity, entries)`：纯映射，不打开仓库/不查状态；旧 `file_statuses` 变成 discover + status + 纯映射包装。
- Files `Message::SnapshotApplied(statuses, GitInfo)`：同一次应用文件状态与分支栏，不触发 `spawn_git_info_load`；legacy `StatusesRefreshed` 行为不变。
- Project `Message::SnapshotApplied(GitSnapshot)`：纯数据入口；legacy `GitRefreshed` 保留。
- Agent `card::apply_workspace_override`：宿主可把同一工作区快照投影到单个 tab，或清除覆盖。
- ProjectStore：DB 行改为 `RawProjectRow`，锁内只读数据库，释放锁后由可注入的 `ProjectGitTime` 计算 `updated_ms`；同一次列表按 `WorkspaceKey` 复用 HEAD 时间；新增“慢 Git 不阻塞其它 DB 访问”回归。
- 面板边界脚本通过。

验证：

```text
cargo test --locked -p dozer-panel-files     82 passed
cargo test --locked -p dozer-panel-project  159 passed
cargo test --locked -p dozer-panel-agent     90 passed
cargo test --locked -p dozer-store-project   33 passed
cargo clippy --locked -p ... --lib -D warnings  通过
python3 scripts/audit/check_panel_boundary.py   通过
```

注意：这四个 crate 的 all-targets clippy 仍有既有测试告警（`module_inception`、`field_reassign_with_default` 等），不属于本轮新增；Dozer CI 现行门禁不跑 clippy。

- 已完成补充：Usage `summary` 默认接缝 + `DoGitStats` 单次日桶遍历 + 64 项/64MiB LRU 缓存；Files 目录读取已后台化。
- 未完成：runtime 自动换 key（普通非 Git 目录已由宿主旧 watch 兜底）。

### 3.3 Agent C（部分）

- `App` 持有进程级 `GitRuntime`，启动时建立事件转发任务；`ShellIo` 把同一句柄交给所有项目。
- `Workspace` 用 `git_token` 代替旧 `git_watch`：
  - 恢复/收养/新开项目异步订阅；
  - 关闭页签、改路径、替换占位时显式退订；
  - 聚焦项目切换同步 `set_active`。
- `GitRuntimeEvent`：
  - `Snapshot` 先校验 token，再映射成 Files `SnapshotApplied`（`file_statuses_from_entries`）与 Project `SnapshotApplied`；不再有刷新后的二次分支 discover。
  - `Watch` 只驱动树/预览/磁盘，且树只对非 HIDDEN 路径重读；Git status 仍由 runtime 保守失效。REFS/FullRescan 才触发 Git Log 刷新。
- 非 Git 目录保留旧 `bytegit::watch` 作为树/预览兜底，不产生 Git 查询。
- 删除生产路径上的 `start_git_watch`、`spawn_project_git_refresh`、`ProjectFsChanged`；旧 `project_git_snapshot` 仅测试使用。

验证：

```text
cargo check -p dozer-app                       通过
cargo test --locked -p dozer-app               1136 passed, 2 ignored
cargo test --locked -p dozer-panel-usage       59 passed
cargo fmt -p dozer-app -p dozer-panel-usage    通过
```

补充完成：

- Git Log `SnapshotLoaded` 带唯一 generation；同 path/max_count 的旧结果不能清掉最新 pending（新增回归）。
- 首页最近文件改为项目子树 + 排除 ignored/非普通文件（新增回归）。
- 磁盘占用在工作区事件下按 10s 最短间隔合并；打开/激活/回合结束仍强制刷新。
- Usage `GitStatsProvider::summary`：默认兼容旧 provider；`DoGitStats` 一次日桶遍历后求和；`NoGitStats` 零遍历。
- Files 目录读取移出 UI：`begin_tree_refresh` 只计算受影响缓存目录，后台 `read_dir_snapshots` 读取，`apply_tree_refresh` 按 generation 合并；每工作区单飞行任务，期间变更合并到 pending。
- 文件树测试新增离线计划、pending 合并、旧 generation 丢弃、全量刷新覆盖缓存目录。

补充完成：Codehealth `git_snapshot` 复用同一个已打开仓库，不再为短 SHA 二次 discover。

未完成：CLI 可取消进程回收、GUI 手工验收证据。

验证补充：

```text
cargo test --locked -p dozer-app             1136 passed, 2 ignored
cargo test --locked -p dozer-panel-files      161 passed
```

### 3.4 Agent D（进行中）

- Digger workspace 引入 `dozer-git-runtime`（tag 占位，`.cargo/config.toml` 本地 patch 到 Dozer 工作树）；bytegit 升到 v0.9.0 并 patch 到本地修复树。
- `ProductShell` 持有进程级 `GitRuntime` 与当前项目 token：
  - 启动建立事件转发；项目切换/关闭时退订/重订；
  - 快照校验 token 后同时更新 Files `apply_snapshot` 与 Project `apply_git_snapshot`；
  - 监听事件只对非 HIDDEN 路径触发 Files 异步目录刷新；
  - `identity_mismatch` 时退订并按当前路径重新订阅。
- Files/Project 适配层停止自行 watcher 与 git 查询；非 Git 目录保留旧 watch 仅刷树。
- 验证：`cargo test --locked -p digger-desktop --lib` → 717 passed；`cargo tree -i bytegit` 只有一份 v0.9.0，`git2` 只有一份。
- 阻断：Digger 无 git remote；依赖仍靠本地 patch；`dozer-git-runtime` 还没有可被 Digger 引用的已发布 libs tag。

## 3.5 复审 N1–N7 修复（2026-11-01）

| 项 | 处理 | 证据 |
|---|---|---|
| N1 远端依赖 | bytegit `v0.9.1`、ByteUI `v0.5.1`、bytehost `v0.4.21` 已发布；Dozer `libs-2026-11-01b`（`e19c0966`）已推送；Digger 全部共享依赖切到该 tag | 无 patch 克隆：Dozer 1138 passed；Digger 717 passed；`cargo tree` 单一 bytegit/git2 |
| N2 切项目 in-flight | `reset_for_project` 清 pending/in-flight 并推进 generation | `switching_projects_cancels_the_old_inflight_tree_refresh` |
| N3 订阅意图乱序 | discover 前分配 intent；runtime 拒绝旧 intent；宿主用 request generation 拒绝旧 ready | runtime `a_late_older_subscribe_intent...`；宿主 request 校验 |
| N4 首快照早于 ready | ready 后显式 `refresh_manual`，保证至少一次有效结果 | 宿主 ready 分支 |
| N5 Git Log HEAD | 失效条件改为 `REFS || HEAD || FULL_RESCAN`；Index 不重建 | `git_log_refreshes_on_head...` |
| N6 普通目录升格 | Dozer/Digger 在 `.git` 创建/删除与 FullRescan 时重新 discover，切换 plain watch/runtime | `reconcile_git_mode`；Digger `refs_changed` 保留 |
| N7 Usage 缓存生命周期 | provider/cache 提升为进程级 `OnceLock`，跨激活复用 | `shared_git_stats_survives_activation...` |

无 patch 验收（隔离克隆，无 `.cargo/config.toml`，重新生成锁文件）：

```text
Dozer  cargo check -p dozer-app                 通过
Dozer  cargo test -p dozer-app                  1138 passed, 2 ignored
Dozer  runtime/panels/usage/store 测试          全部通过
Digger cargo check -p digger-desktop            通过
Digger cargo test --lib -p digger-desktop       717 passed
```

## 4. 最终结论（未就绪）

- **Dozer 实际运行路径是否完整包含本次重构：** 是（代码路径已接入 runtime，`cargo test -p dozer-app` 1136 passed）；缺 GUI 手工录像/日志证据。
- **Digger 实际运行路径是否完整包含本次重构：** 是（代码路径已接入 runtime，`cargo test -p digger-desktop` 717 passed + 集成测试通过）；缺 GUI 手工证据。
- **两者是否能在无本地 patch 的 checkout 构建并通过现行门禁：** **是**（隔离克隆、无 `.cargo/config.toml`、重新生成 Cargo.lock）：Dozer `check` + 1138 tests，Digger `check` + 717 tests。
- **是否仍有阻止发布的代码、依赖、测试或外部条件：** 仍有但已收窄。
  1. CLI `clone`/`checkout` 可取消进程树回收未实现；
  2. 两产品 release 四件套构建、打包资源/plist 一致性未执行；
  3. GUI 手工录像/日志与 A1–A9 最终矩阵未完成；
  4. macOS 已实测；Linux/Windows 事件与进程回收未验证；
  5. Digger 仍无 git remote（不影响本地无 patch 构建，但影响正式发布方式）。

结论：**依赖闭包已收口，代码路径与自动化测试就绪；发布前仍需完成 CLI 取消、release 构建、GUI 手工验收与跨平台验证。** 因此暂不宣告最终发布就绪。

### 4.1 建议发布顺序（未执行）

| 顺序 | 仓库 | 内容 | 建议版本/来源 |
|---|---|---|---|
| 1 | bytegit | `8763899`（R3/R4）+ `6c91137`（严格 patch） | `v0.9.1` |
| 2 | ByteUI | `541554f` 起的已提交接口 | 新产品可用的 `v0.5.1`（需维护者确认） |
| 3 | Dozer | `dozer-git-runtime` + 面板纯入口/T9–T12 + workspace bytegit v0.9.1 | `libs-2026-11-01a`（标签名由 Dozer 维护者确认） |
| 4 | Digger | 本轮的 runtime 接入 + 依赖切到上述真实 tag | App 版本由用户另行安排 |

发布前必须在隔离 checkout 中验证 tag 真实包含目标 commit（版本号一致不等于代码一致），然后删除两产品 `.cargo/config.toml` 的本地 patch 重建 Cargo.lock。
