# Digger×bytehost 集成 Ledger

> 每个 Task 一条记录：状态、输入 revision、输出 revision、运行命令与退出码、结果、偏差、未决项。
> 被前置条件阻塞的 Task 标 `blocked-by:<task/evidence>`，不打完成勾。
> 路径前缀 `B/`=bytehost、`Z/`=Dozer、`D/`=Digger（非真实目录）。见[计划](../../superpowers/plans/2026-10-11-digger-bytehost-full-integration.md) §0。

状态图例：`TODO` / `IN_PROGRESS` / `DONE` / `BLOCKED`。

---

## Task 01 — 基线、归属清单和验收台账

- **状态：** DONE
- **输入 revision：**
  - Z `a4cbf340dec7be39d8f055612f15d50240f0a943`（main；工作区含用户既有 WIP，已隔离）
  - B `6fdca5dcfad12472244ef05a7550c6060d3a0342`（main；干净）
  - D `8ef11953c1745fa538025373a17ce3f69ce3b133`（main；含 `spike/md-mirror/src/lib.rs` WIP 与 `?? .dozer/`）
- **输出 revision：** 本任务产出（见下）随 `docs(integration): record host extraction baseline` 提交组落地；提交 hash 见 §提交记录。
- **产出文件：**
  - `Z/docs/dozer-v2/digger-bytehost-integration/baseline.md`
  - `Z/docs/dozer-v2/digger-bytehost-integration/ownership.md`
  - `Z/docs/dozer-v2/digger-bytehost-integration/acceptance.md`
  - `Z/docs/dozer-v2/digger-bytehost-integration/ledger.md`（本文件）
- **运行命令与退出码：**

| 仓库 | 命令 | 退出码 | 结果 |
|---|---|---|---|
| Z | `python3 scripts/audit/check_panel_boundary.py` | 1 | 已知回退 `app/app.rs: R-HOST-EXECUTORS 0 -> 1` |
| Z | `scripts/check-log-scope.sh` | 0 | 通过 |
| Z | `python3 -m unittest discover -s scripts/audit -p 'test_*.py'` | 1 | 46 中 1 失败（`test_unit_edges.ScanUsage`，既有） |
| Z | `cargo test -p dozer-app` | 1 | 1 既有失败 `extensions::files::tests::delete_confirm_spec_reflects_pending_target` |
| Z | `rg -c "PanelKind::" ...` / `wc -l ...` | 0 | 统计数值见 ownership.md §4 |
| B | `bash scripts/check-deps.sh` | 0 | 通过 |
| B | `cargo fmt --all --check` | 0 | 干净 |
| B | `cargo test --workspace --all-features` | 0 | 通过（408+95+13+3，另有 ignore） |
| B | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 | 通过 |
| D | （无根 Cargo.toml） | — | 正式 workspace 尚不存在，无正式基线 |
- **结果：** 三仓库 revision/环境/依赖版本已记录；Dozer 面板边界与统计口径已固化；A1–A18 台账与兼容桥清单（C1–C9）已建立；所有既有失败复现一致。
- **偏差：**
  - Dozer 工作区含用户既有 WIP（`crates/dozerd/src/{headless_agent,summary_pipeline}.rs` 及三份 `??` 文档、版本自增），本任务未触碰；提交仅含本任务四份记录文件。
  - `bytehost-*` 在 Dozer `Cargo.lock` 中 package version 为 `0.1.0`，而在 `crates/dozer-app/Cargo.toml` 以 `tag = "v0.1.1"` 引入；task 记录两者对应同一来源。
- **未决项 / 阻塞：** 无（D 无正式 workspace 属已声明的现状，不是阻塞）。

---

## Task 02 — 真实窗口入口与类型契约探针

- **状态：** DONE
- **输入 revision：**
  - Z `d5ab7f83` 之上一提交（Task 01 后主线）；依据 `D/spike/editor-host/`（`8ef11953`）
  - B `6fdca5dc` → `0ca8320`（新增探针）
- **输出 revision：**
  - B `0ca8320` `spike(desktop-contract): iced×wry 桌面契约探针(Task 02 §11 闸门1)`
  - Z `d5ab7f83` `docs(integration): Task 02 窗口入口与泛型 API 决策`
- **产出文件：**
  - `Z/docs/dozer-v2/digger-bytehost-integration/api-decisions.md`
  - `B/spike/desktop-contract/{Cargo.toml,Cargo.lock,.gitignore,README.md,src/lib.rs,src/main.rs}`（独立 `[workspace]`，工作区外，避开 `check-deps.sh` 的 iced/wry 禁令）
- **运行命令与退出码：**

| 仓库 | 命令 | 退出码 | 结果 |
|---|---|---|---|
| B | `cargo build`（spike） | 0 | 编译通过（iced 0.14.0 / iced_winit 0.14.1 / wry 0.55.1） |
| B | `cargo test --lib`（spike） | 0 | 3/3 证据测试通过 |
| B | `SELFTEST=1 cargo run`（spike） | 0 | 真实窗口入口建窗后退出，无 panic |
| B | `cargo clippy --all-targets -- -D warnings`（spike） | 0 | 干净 |
| B | `cargo fmt --check`（spike） | 0 | 干净 |
| B | `bash scripts/check-deps.sh` | 0 | 通过（spike 在工作区外，未污染） |
| Z | （文档任务，未跑代码） | — | — |
- **结果：** 规格 §11 闸门 1 回填为"成立"。锁定 iced/wry 版本；入口模式（`iced::application` 单循环 + `window::run` + thread_local + channel↔Subscription）；几何/DPI/overlay 分工；`Product<M>`/`HostMessage<M>` 对两个不同产品消息类型成立；同一 Surface 描述表达 Usage 与 CodeHealth 两类推送；core/desktop 分层签名与编译证据。
- **偏差：**
  - 探针窗口入口用 `window::oldest()` 取窗口 id（非 `Id::MAIN`，后者在 iced 0.14 不存在）；与 `D/spike/editor-host` 一致。
  - `block v0.1.6` 触发 future-incompat 警告（传递依赖，非本探针代码；digger spike 同样存在）。
- **未决项 / 阻塞：** 无。

## Task 03 — AppService 缺口与 Digger 数据源调查

- **状态：** DONE
- **输入 revision：**
  - B `0ca8320`（`service.rs`/`port.rs`/`in_process.rs` 未改）
  - Z 主线（`docs/dozer-v2/bytehost-H0/{07,08}`、规格 §11）
  - D `8ef11953`（领域/技术架构 + ADR-011）
- **输出 revision：** Z 见 §提交记录（`docs(integration): Task 03 §11 闸门回填`）
- **产出文件：** `Z/docs/dozer-v2/digger-bytehost-integration/gates.md`
- **运行命令与退出码：**

| 仓库 | 命令 | 退出码 | 结果 |
|---|---|---|---|
| Z/B/D | `rg`/`read` 调查（只读，未改代码） | 0 | 证据见 gates.md |
- **结果：**
  - **闸门 2：** Digger 侧无真实 Usage 账本，仅领域占位与架构意图（`Logical_Domain_Model_v0.3:1661-1668,1688`；`Technical_Architecture_v0.1:676,698,781`）；无生产 workspace。**缺口已定位**（Rust 持久化账本→映射 `UsagePayload`，不连 dozerd；未实现时须返回 Unavailable）。**S6/S7 不得声称取数完成。**
  - **闸门 3：** AppService 多数够用（启动永不失败→Unavailable；端口不静默重选、冲突可观测、损坏报错不静默；`desired` 保留、reconcile 回收）。**不足 2 项**：`shutdown()` 返回 `()`（退出失败不可观测，规格 `:134`）、缺端口重置管理接口（规格 `:138`）。二者须在 bytehost 增补受测接口（只追加，不改 wire 形状）。
  - **闸门 1** 已在 Task 02 回填；**闸门 4/5** 留后续 Task。
- **偏差：** 无。
- **未决项 / 阻塞：** 无（bytehost 增补接口属后续 Task，非本任务阻塞）。

---

## Task 04 — 引入 bytehost-core 纯契约层与三组依赖门禁

- **状态：** DONE
- **输入 revision：**
  - B `0ca8320`（Task 02 探针之上）
  - Z 主线（计划 §Task 04 `:79-90`；规格 §4/§5）
- **输出 revision：** B `972e461` `feat(core): introduce product-neutral panel identities`
- **产出文件：**
  - `B/crates/bytehost-core/{Cargo.toml,src/lib.rs,src/id.rs,src/registry.rs,src/layout.rs,tests/catalog.rs}`
  - `B/scripts/check-deps.sh`（改为 apps/core/desktop 三组 + `--self-test` 变异测试）
  - `B/.github/workflows/ci.yml`（接入 `--self-test`）
- **运行命令与退出码：**

| 仓库 | 命令 | 退出码 | 结果 |
|---|---|---|---|
| B | `cargo test -p bytehost-core` | 0 | 22 通过（15 单测 + 7 集成） |
| B | `cargo test --workspace --all-features` | 0 | 全通过（default feature 下 `bytehost-client` 示例既缺 feature，非本任务） |
| B | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 | 干净 |
| B | `cargo fmt --all --check` | 0 | 干净（提交前已 fmt） |
| B | `bash scripts/check-deps.sh` | 0 | 通过；desktop 组尚未创建（Task 07 起自动纳入） |
| B | `bash scripts/check-deps.sh --self-test` | 0 | 变异 1（core→iced）、变异 2（apps→digger-*）均被拦下；恢复即通过 |
- **结果：**
  - 新增 `bytehost-core` 纯契约层:`PanelId`（`<ns>.<name>` / `<ns>:<name>`，应用面板 `app:<validated-id>`）、`ScopeId`（不透明、产品提供）、`PanelInstanceId`（slot+generation，重建递增）、`SurfaceSlot`；`PanelCatalog` 注册目录（重复 id 立即报错、未知历史 id 保留）；`Layout` DTO（schema version、尺寸钳制到 `[160,4000]`、未知更高版本拒绝覆盖、产品默认布局作为输入）。
  - 依赖门禁扩为三组:apps（四 crate，禁 `dozer*`/`digger*`/iced/wry/tauri/objc2）、core（只允许 serde 家族）、desktop（允许 iced/wry，禁产品与 tauri/objc2）。新增 `--self-test` 证明门禁非恒真,并接入 CI。
- **偏差：**
  - `cargo test --workspace`（不加 `--all-features`）的失败来自 `bytehost-client/examples/embed.rs` 需 `in-process` feature,属既有现状,与本次改动无关;CI 与验证用 `--all-features`。
  - `scripts/check-deps.sh` 中 `bytehost-desktop` 组在 crate 出现前打印 note 并跳过（Task 07 自动纳入）。
- **未决项 / 阻塞：** 无。

---

## Task 05 — bytehost-core 请求身份与执行上下文

- **状态：** DONE
- **输入 revision：**
  - B `972e461`（Task 04 之上）
  - Z 主线（计划 §Task 05 `:92+`；规格 §5.3 请求信封与结果路由）
  - 原型输入：`Z/crates/dozer-app/src/request_tracker.rs`（迁移前行为已由既有单测钉住）
- **输出 revision：** B `4c4d748` `feat(core): preserve scoped writes and reject stale reads`
- **产出文件：**
  - `B/crates/bytehost-core/src/request.rs`
  - `B/crates/bytehost-core/tests/request_routing.rs`
  - `B/crates/bytehost-core/src/lib.rs`（导出 `request`）
- **运行命令与退出码：**

| 仓库 | 命令 | 退出码 | 结果 |
|---|---|---|---|
| B | `cargo test -p bytehost-core` | 0 | 38 通过（含既有 + 请求路由单测/集成） |
| B | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 | 干净 |
| B | `cargo fmt --all --check` | 0 | 干净 |
| B | `bash scripts/check-deps.sh` | 0 | 通过 |
- **结果：**
  - 新增中性请求身份：`Generation`（`ZERO`/`new`/`get`/`next`）、`RequestKey(&'static str)`、`RequestToken{owner,scope,instance,generation,key:Option<RequestKey>,seq}`（`Clone`，非 `Copy`，持 `ScopeId`）、`RequestTracker::new(owner,scope,instance,generation)` 提供 `begin(key)`（latest-only）、`scope()`（作用域写入，不被新读作废）、`invalidate(key)`、`accepts(&token)`（校验 owner+instance+generation+scope+按键 seq）；`ResultEnvelope<T>`；执行器/发射器抽象 `BackgroundExecutor`/`Emitter`；执行原语 `run_scoped` 在**派发路径**校验、超期结果丢弃。
  - core 不含 Tokio/iced 具体实现；具体实现留待 `bytehost-desktop`（Task 07）。
- **偏差：** 无（原型 `request_tracker.rs` 的行为——旧回包失效、lane 独立、重开面板拒绝旧结果——由 core 单测逐一对应）。
- **未决项 / 阻塞：** 无。

---

## Task 06 — 拆分 Dozer 客户端适配 + 接入公共请求身份

- **状态：** DONE
- **输入 revision：**
  - B `4c4d748`（`bytehost-core` 含 `id`/`request`）
  - Z 主线 `9959bd4e`（Task 04 记录之上）
- **输出 revision：** 本任务随提交组落地；提交 hash 见 §提交记录。
- **产出文件：**
  - `Z/crates/dozer-app/src/host_adapters/{mod.rs,services.rs}`（新）
  - `Z/crates/dozer-app/src/panel_host.rs`（`services_for`/`ClientDaemon`/`ensure_error_to_service` 移出；`PanelIo::new` 改为接收 `Services`）
  - `Z/crates/dozer-app/src/request_tracker.rs`（改为 `bytehost_core::request` 之上的产品适配层）
  - 调用点更新：`app/app.rs`、`app/update.rs`、`workspace/state.rs`、`usage_probe/host.rs`、`main.rs`，及各扩展测试
  - `Z/crates/dozer-app/Cargo.toml`（新增 `bytehost-core` 依赖；`.cargo/config.toml` 本地 `[patch]` 指向 `../bytehost`，**不提交**）
- **运行命令与退出码：**

| 仓库 | 命令 | 退出码 | 结果 |
|---|---|---|---|
| Z | `cargo check -p dozer-app` | 0 | 通过 |
| Z | `cargo test -p dozer-app --bin dozer request_tracker` | 0 | 3 通过（含新增"一 tracker 服务多 scope 互不接受"） |
| Z | `cargo test -p dozer-app --bin dozer -- host_adapters panel_host extensions::…` | 0 | 508 通过（settings/usage/group_chat/todo/project/browser/conversations/agent_context/codehealth） |
| Z | `cargo test -p dozer-app --bin dozer` | 1 | 仅既有失败 `extensions::files::tests::delete_confirm_spec_reflects_pending_target`（2008 通过） |
| Z | `cargo clippy -p dozer-app --all-targets` | 0 | 无新告警（既有 `agent_token_share`/`GitProvider::ALL` 未用告警） |
| Z | `cargo fmt -p dozer-app -- --check` | 0 | 干净 |
| Z | `python3 scripts/audit/check_panel_boundary.py` | 0 | 仅既有回退 `app/app.rs: R-HOST-EXECUTORS 0 -> 1` |
- **结果：**
  - 服务构造从 `panel_host.rs`（宿主机制）移入 `host_adapters/services.rs`（产品适配层），组合根不再在 host 里直接构造适配（兼容桥 **C4** 收口）；`PanelIo::new` 改收 `Services`。
  - 测试夹具不再"先构造离线 `Client` 再 `.with_services(...)` 覆盖"：`offline_ctx`/`recording_io`/`usage_probe::activate_with` 直接用 `Services`（兼容桥 **C3** 收口）。
  - `request_tracker.rs` 改为 `bytehost_core::request` 之上的**产品适配层**：`ProjectId(i64) → ScopeId` 的转换只在此文件（`scope_of`），一 tracker 惰性持有每个 scope 的 core tracker（支持浏览器"全局 + 项目"多条 lane）。
  - `RequestToken` 保留为**遗留类型包装**（compat bridge **C8** → Task 21），因 Dozer 面板 `Message::AsyncResult` 需 `project_id` 做跨项目路由；core 身份已在其内。
- **偏差：**
  - `bytehost-core` 尚未随 tag 发布（正式发布在 Task 22），本地以不提交的 `[patch."https://github.com/byteboyai/bytehost"]` 指向 `../bytehost` 联调；`Cargo.lock` 新增 `bytehost-core` 条目。
  - `accepts` 由按值改为按引用（core 同形），相应测试调用点补 `&`/`.clone()`（`RequestToken` 现为 `Clone` 非 `Copy`）。
- **未决项 / 阻塞：** 无。C8 的真正删除在 Task 21。

---

## 提交记录

| Task | 提交 | 仓库 | 说明 |
|---|---|---|---|
| 01 | `a9af4903` | Z | `docs(integration): record host extraction baseline` |
| 01 | `7924dc6c` | Z | `docs(integration): record Task 01 commit hash in ledger` |
| 02 | `0ca8320` | B | `spike(desktop-contract): iced×wry 桌面契约探针(Task 02 §11 闸门1)` |
| 02 | `d5ab7f83` | Z | `docs(integration): Task 02 窗口入口与泛型 API 决策` |
| 03 | `45f95ce0` | Z | `docs(integration): Task 03 §11 闸门回填` |
| 04 | `972e461` | B | `feat(core): introduce product-neutral panel identities` |
| 05 | `4c4d748` | B | `feat(core): preserve scoped writes and reject stale reads` |
| 06 | `d9b90deb` | Z | `refactor(host): move service adapters to host_adapters and adopt core request identity` |
