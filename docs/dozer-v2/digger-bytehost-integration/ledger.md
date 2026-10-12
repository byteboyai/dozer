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
| B | `SELFTEST=1 cargo run`（spike） | 0 | 建窗**并真建 wry 子视图**（等待 `HostBuilt(Ok)`）后自退；建窗或子视图创建失败以非零码退出（见 §Review 修复 R7） |
| B | `cargo clippy --all-targets -- -D warnings`（spike） | 0 | 干净 |
| B | `cargo fmt --check`（spike） | 0 | 干净 |
| B | `bash scripts/check-deps.sh` | 0 | 通过（spike 在工作区外，未污染） |
| Z | （文档任务，未跑代码） | — | — |
- **结果：** 规格 §11 闸门 1 回填为"成立"。锁定 iced/wry 版本；入口模式（`iced::application` 单循环 + `window::run` + thread_local + channel↔Subscription）；几何/DPI/overlay 分工；`Product<M>`/`HostMessage<M>` 对两个不同产品消息类型成立；同一 Surface 描述表达 Usage 与 CodeHealth 两类推送；core/desktop 分层签名与编译证据。**证据边界（R7 修订）：** `SELFTEST=1 cargo run` 只证明 iced 窗口入口可启动且 wry 子视图可建（`HostBuilt(Ok)`），**不**证明 resize、隐藏恢复、原生浮层等行为——那些属真实窗口人工确认（见 Task 08）。
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
- **偏差：** 本任务只做了只读调查（`rg`/`read`），**未**执行计划该 Task 要求的隔离双启动/退出实验（`B/spike/desktop-contract` 的独立进程双启动）；该实验仍缺（R7 指出）。
- **未决项 / 阻塞：** 无阻塞。隔离双启动/退出实验待补（可并入 Task 04 之后的 bytehost 生命周期工作，或 Task 08 真实窗口确认）。

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

## Task 07 — bytehost-desktop 窗口壳与产品回调

- **状态：** DONE
- **输入 revision：**
  - B `4c4d748`（Task 05 之上）
  - 计划 §Task 07 `:116-125`；规格 §4/§5；Task 02 决策 `api-decisions.md`（窗口入口、泛型契约）
  - 参考实现：`D/spike/editor-host`（真实 iced 窗口 + wry，ADR-001）、`B/spike/desktop-contract`（双产品类型编译探针）
- **输出 revision：** B `4a8dba0` `feat(desktop): add reusable shell and product callbacks`
- **产出文件：**
  - `B/crates/bytehost-desktop/{Cargo.toml,src/lib.rs,src/shell.rs,src/panel.rs,src/dispatch.rs,examples/minimal.rs,tests/product_contract.rs}`
  - `B/crates/bytehost-core/src/geometry.rs`（新增 `Insets`/`LogicalRect`/`host_rect`）+ `lib.rs` 导出
  - `B/Cargo.toml`（workspace 依赖加 `bytehost-core` path）
  - `B/scripts/check-deps.sh`（desktop 组 objc2 误禁修正）
- **运行命令与退出码：**

| 仓库 | 命令 | 退出码 | 结果 |
|---|---|---|---|
| B | `cargo test -p bytehost-core` | 0 | 41 通过（含新增 geometry 3） |
| B | `cargo test -p bytehost-desktop` | 0 | 15 通过（6 单测 + 6 dispatch/panel + 3 product_contract） |
| B | `cargo test --workspace --all-features` | 0 | 全通过 |
| B | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 | 干净 |
| B | `cargo fmt --all --check` | 0 | 干净 |
| B | `bash scripts/check-deps.sh` | 0 | 通过（desktop 组已自动纳入并校验） |
| B | `bash scripts/check-deps.sh --self-test` | 0 | 两处变异仍被拦下；恢复即通过 |
| B | `SELFTEST=1 cargo run -p bytehost-desktop --example minimal` | 0 | 真实窗口示例启动并自退（无 panic） |
- **结果：**
  - `bytehost-desktop` 提供**框架壳**，产品以泛型消息 `M` + 回调接入：`HostMessage<M>`（仅 `Product(M)` / `Surface(事件)` 两类，业务命令不进宿主 enum）、`Product<M>` trait（`update`/`view`/`on_activated`/`on_closed`）、`route_lifecycle`。
  - `panel` 模块实现 Task 02 锁定的入口：`window::run` + `build_as_child` 建 wry 子视图、非 `Send` 句柄 thread_local、`set_bounds`/`evaluate`/`destroy`；创建失败经 `ChildBuilt.result=Err` 回传，不 panic。
  - `dispatch` 提供 `TokioExecutor`（`BackgroundExecutor`）与 `ProxyEmitter`（iced `futures::channel::mpsc::Sender`，`Send+Clone`），接 core `run_scoped` 的**派发前**身份校验。
  - `bytehost-core` 补 `geometry`（`host_rect` 顶部条下推，纯函数，无 iced 依赖）。
  - 双消费者夹具 `tests/product_contract.rs`：两个不同产品消息类型 + 两个不同服务上下文类型实例化同一 Host 接口，均不引用 Dozer。
- **偏差：**
  - **门禁修正：** Task 04 的 desktop 组误禁 `objc2*`——`objc2` 是 wry/winit 在 macOS 的**传递**平台依赖，禁它等于禁 wry；规格 §67 的 desktop 白名单（iced/wry/byteui/core）并未禁平台绑定。本次从 desktop 组移除 `objc2*` 禁项，保留 `tauri*` 禁项，并更新脚本注释。apps/core 两组仍禁 objc2（它们不应出现 iced/wry，故传递也不会出现）。
  - iced 0.14 的跨线程回灌通道是 `iced::futures::channel::mpsc::Sender`（非 `iced::task::Sender`）；`ProxyEmitter` 按此定形。
- **未决项 / 阻塞：** 无。本任务只做机制，Surface 状态机与真实面板接入在 Task 08。

---

## Task 08 — Surface 状态机、原生执行与 Usage 接入

- **状态：** IN_PROGRESS（bytehost 部分 DONE，Dozer 接入部分代码完成、待提交与真实窗口人工确认）
- **输入 revision：**
  - B `4a8dba0`（Task 07 之上）
  - 计划 §Task 08 `:127-138`；规格 §5.4 `:85-170`、§11 闸门
- **输出 revision（bytehost 两部分已落地）：**
  - B `17c4ad4` `feat(core): add Surface lifecycle state machine`
  - B `2aaf935` `feat(desktop): execute Surface lifecycle, routing and geometry`
  - B `f4639bb` `fix(desktop): stop forcing iced renderer features on consumers`（接入使能修复）
  - Z `5b940f11` `refactor(usage): route Usage content host through shared Surface`（Dozer 接入部分）
- **产出文件：**
  - `B/crates/bytehost-core/src/surface.rs`（新；状态机 + `PushState` + `ReadyPolicy`）+ `lib.rs` 导出
  - `B/crates/bytehost-desktop/src/surface/{mod,lifecycle,geometry,routing}.rs`（新）+ `lib.rs` 注册
  - `B/crates/bytehost-desktop/tests/surface_lifecycle.rs`（新）
  - `Z/crates/dozer-app/src/extensions/usage/surface.rs`（新；`UsageSurface` 适配层，固定单槽）
  - `Z/crates/dozer-app/src/preview/webview.rs`（`WebviewSpec.surface: Option<SurfaceId>`）
  - `Z/crates/dozer-app/src/app/app.rs`（`take_usage_content_script` 走 `observe_instance`/`set_desired`/`take_pending_script`；Usage spec 带 `surface`）
  - `Z/crates/dozer-app/src/app/update.rs`（Ready 先经 `accepts` 路由校验再 `apply_ready`）
  - `Z/crates/dozer-app/src/app/message.rs`（`UsageContentWebviewEvent(.., Option<SurfaceId>)`）
  - `Z/crates/dozer-app/src/runtime.rs`（IPC 带 spec 的 `surface` 身份）
  - `Z/crates/dozer-app/src/usage_probe/{host.rs,surface.rs,scenarios.rs}`（探针/漂移守卫改到 Surface 模型）
- **运行命令与退出码：**

| 仓库 | 命令 | 退出码 | 结果 |
|---|---|---|---|
| B | `cargo test -p bytehost-core` | 0 | 全通过（surface 10 新单测） |
| B | `cargo test -p bytehost-desktop` | 0 | 全通过（surface 24 单测 + surface_lifecycle 8 集成） |
| B | `cargo test --workspace --all-features` | 0 | 全通过（408+…） |
| B | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 | 干净 |
| B | `cargo fmt --all --check` | 0 | 干净 |
| B | `bash scripts/check-deps.sh` + `--self-test` | 0 | 通过 |
| B | `SELFTEST=1 cargo run -p bytehost-desktop --example minimal` | 0 | 真实窗口示例自退，无 panic |
| Z | `cargo check -p dozer-app` | 0 | 通过（仅既有 2 warning） |
| Z | `cargo test -p dozer-app --bin dozer usage` | 0 | 100 通过（含 6 个 UsageSurface 状态测试 + 迁移后探针） |
| Z | `cargo test -p dozer-app --bin dozer` | 1 | 2009 通过；1 既有失败（`extensions::files::tests::delete_confirm_spec_reflects_pending_target`，非本任务） |
| Z | `cargo clippy -p dozer-app --all-targets` | 0 | 仅既有 warning（`agent_token_share`/`GitProvider::ALL` 等） |
| Z | `cargo fmt -p dozer-app -- --check` | 0 | 干净 |
| Z | `python3 scripts/audit/check_panel_boundary.py` | 1 | 仅既有回退 `app/app.rs: R-HOST-EXECUTORS 0 -> 1`（`run_app_host_effects`，非本任务） |
| Z | `bash scripts/build-macos-app.sh release` + 安装运行 | 0 | 0.5.29x 真实启动、无 panic（日志见下） |
- **结果（bytehost 部分）：**
  - `bytehost-core::surface`：`SurfaceId{instance,slot,generation}`、`SurfaceKind{Builtin,Application}`、`ReadyPolicy{Handshake,NavigationCompleted}`、`SurfaceState`（`Absent→Creating→WaitingReady→Ready→Hidden/Destroyed`，失败进可重试 `Failed`）、`PushState`（首次全量、去重、隐藏缓存、重建重发、`has_unsent`）、`StaleGeneration` 丢弃。
  - `bytehost-desktop::surface`：`routing`（`(instance,slot,generation)` 显式归属，`Routed::{Accepted,UnknownSurface,StaleGeneration}`，重建驱逐旧代）；`geometry`（`Placement`/`surface_rect`/`rect_after_overlay`，insets/split/零面积/浮层还原）；`lifecycle`（`LifecycleAction` + `actions_for` + `execute_on_handle`；创建唯一入口仍是 `panel::build_child`）；`SurfaceExecutor`（组合状态 + 路由 + 载荷缓存，载荷→脚本由产品委托 `ScriptBuilder`）。
- **结果（Dozer 接入部分）：**
  - `UsageSurface` 内部持有 `SurfaceExecutor<UsageViewPayload>`，身份固定 `PanelInstanceId(0,0)` + `SurfaceSlot(USAGE_CONTENT_ID_OFFSET)` + 递增 `generation`。
  - 每帧 `observe_instance(句柄在池中)` 驱动生命周期：进池 → `Created`/`WaitingReady`；离池 → `Destroyed` 并递增代数（旧实例在途 IPC 自此被判过期）。
  - Ready IPC 先经 `accepts(Option<SurfaceId>)`（路由层校验代数/槽）再 `apply_ready()`；非当前代数 `log_warn` 丢弃。
  - 载荷构造（`current_view_payload`/`encode_usage_push`）仍留在 `extensions::usage`，executor 只按 `PushState` 语义决定何时推；`take_pending_script` 与旧 `pending_push`+`mark_sent` 契约一致（附带物理槽 id）。
  - `WebviewSpec` 增加 `surface: Option<SurfaceId>`，仅 Usage 槽填 `Some(...)`，其余 14 处填 `None`；`runtime.rs` IPC 回传带上创建时登记的身份。
  - 删除被替代的 `extensions::usage::WebviewPushState`；漂移守卫（`usage_probe`）改到 Surface 模型，仍钉住真实 host 的步骤顺序、提前返回个数与行号引用。
- **真实窗口人工确认（未完成，需人工）：**
  - 自动化验证受环境限制：`screencapture` 无屏幕录制权限；Orca 辅助 App 无辅助功能权限，`lsappinfo windowInfo=[NULL]`，无法读取 Dozer 窗口控件树。
  - 已确认：新二进制 0.5.29x 真实启动、无 panic/ERROR 日志（`dozer-app.log.2026-10-12`：字体剔除、runtime 能力快照、预览 tab ready 均正常）。
  - **待人工执行：** 打开 Usage 面板确认首帧渲染；切换项目确认内容随之更新且旧项目数据不残留；快速隐藏/恢复面板确认恢复后重发当前内容、无空白。步骤与预期记录于 `acceptance.md`。
- **偏差：** 无（bytehost 部分）。真实窗口确认因宿主权限缺失转人工，已在 acceptance 记录。
  - 计划 Task 08 把 `webview_geometry.rs` 列入"修改"，但本任务把几何仍留在产品侧：Usage 窗格矩形由 `ShellState`/`dims.usage_split`/`theme::geometry` 常量算出（2.1k 行、多面板共用），bytehost `surface::geometry` 提供的是 shell 级通用 `Placement`/`surface_rect`/`rect_after_overlay`，为它改造会动像素行为、收益低。计划的具体要求（clause 135）只针对逐帧推送/ready/IPC 经 Surface；几何经 spec 的零面积裁剪与 `observe_instance` 已与 Surface 生命周期联动。此口径待 Task 后续或用户裁决，见 `api-decisions.md`。
- **未决项 / 阻塞：** 真实窗口确认待人工；其余无。

---

## Task 09 — CodeHealth 接入共享 Surface(第二消费方)

- **目标（计划 §Task 09 `:140-149`）：** 让 CodeHealth 作为 Surface API 的第二个消费方,在不改变既有行为(进面板不自动扫描)的前提下接入共享生命周期;编辑器/预览/应用三类维持各自独立的信任配置与构建路径;以测试证明应用够不到编辑器/预览的自定义协议与业务 IPC、未知 Surface/旧代数是拒绝的;把第二个消费方暴露出来的通用缺口补进公共层,禁止新增 `if panel == codehealth`。
- **规格依据:** §5.4 `:85-170`;设计 `api-decisions.md` §9(提交 `b8ae6bb3`)。

**实现：**
- **共享层零改动(`api-decisions §9.6`)。** 第二消费方未暴露 `bytehost-desktop`/`bytehost-core` 的通用缺口——Usage 已验证的 `SurfaceExecutor` 接口(register/observe/`set_desired`/`take_pending_script(true)`/路由)对 CodeHealth 的形状完全够用。故本任务 bytehost 无提交。
- **信任建模落在 Dozer,不进 bytehost。** 新增 `preview::trust::WebviewTrust{Application,Editor,Preview,BuiltinPanel}`,由 `WebviewTrust::of(&WebviewSpec)` 从既有字段**唯一**派生(不新增易漏填字段):应用 id 区间 → `Application`;固定槽 id 或带 `surface` → `BuiltinPanel`;带 `editor_binding` → `Editor`;其余 → `Preview`。`runtime.rs` 的池分派由 `is_app_webview_id(spec.id)` 改为 `WebviewTrust::of(&spec).is_application()`,并在可信宿主分支 `debug_assert!(trust.allows_custom_protocol())` 后安装 `dozer://`——把"应用拿不到 `dozer://`"从字符串判断升级为类型/派生层的保证。`bytehost-desktop` 仍只认 core 的 `SurfaceKind{Builtin,Application}`,不认识 editor/preview。
- **CodeHealth 接入 Surface:** 新增 `extensions::codehealth::surface::CodeHealthSurface`(包裹 `SurfaceExecutor<CodeHealthViewPayload>`,固定 `PanelInstanceId(0,0)` + `SurfaceSlot(CODEHEALTH_CONTENT_ID_OFFSET)`,持有 `generation` 与单调 `revision: Arc<AtomicU64>`)。
- **面板级降级留在面板侧:** `WebviewPushState` 收敛为只持 `waiting_since` + `failed`,`observe_availability(available, ready, now)` 增一个 `ready` 入参(由 `surface.is_ready()` 供给);ready/revision/去重/重建重发全部移交 Surface。`failed` 时 `take_codehealth_content_script` 早退,原生占位页接管。
- **接线:** `app.rs`(`codehealth_surface` 字段 + 重写 `take_codehealth_content_script` + `WebviewSpec.surface`)、`app/message.rs`(`CodeHealthContentWebviewEvent(event, Option<SurfaceId>)`)、`app/update.rs`(经 `codehealth_surface.accepts(surface)` 校验归属,`Ready => apply_ready()`)、`runtime.rs`(IPC 回传送 `spec.surface`)、`platform/window_events.rs`(传 `pool_dropped_ids`)。

**测试:**
- `extensions/codehealth/surface.rs`:`pending_push_requires_ready_and_changes`;`rebuild_forces_a_resend_even_when_desired_unchanged`;`same_frame_destroy_and_recreate_opens_a_new_generation`(R1 同型);`stale_generation_ipc_is_rejected_after_rebuild`(旧代数 `route_of`/`accepts` 均拒);`revision_is_monotonic_across_rebuilds`(跨重建继续递增,新 host 计数从零恒接受)。
- `preview/trust.rs`:`app_id_derives_application`(含 `allows_custom_protocol()==false`);`fixed_slot_derives_builtin_panel`;`surface_marks_builtin_panel_even_without_offset`;`editor_binding_derives_editor`;`plain_host_derives_preview`。
- `app_webview::build_app_webview_pins_the_restrictive_settings`(既有,源码扫描)持续钉住应用路径无 `with_custom_protocol`、无 `APP_INIT_SCRIPT`、devtools/下载/新窗口关闭、走 `AppIpc::parse` nonce 白名单——与 `WebviewTrust::Application` 互为交叉验证。
- 未新增 `if panel == codehealth` 生命周期门控(既有的 `if kind == PanelKind::CodeHealth` 是逐面板 spec 发射,与 Usage 同形,非本任务新增)。

**验证:**
- `cargo fmt -p dozer-app` 干净;`cargo clippy -p dozer-app --all-targets` 干净(仅两条预存在 `agent_token_share`/`GitProvider::ALL` 未用告警)。
- `cargo test -p dozer-app --bin dozer`:**2018 通过 / 1 预存在失败**(`extensions::files::tests::delete_confirm_spec_reflects_pending_target`,非本次改动);codehealth 子集 102 通过;trust 5 通过。
- 边界门禁 `scripts/audit/check_panel_boundary.py`:仅预存在 `app/app.rs: R-HOST-EXECUTORS 0 -> 1`。
- 漂移守卫 `usage_probe::host::host_site_citations_point_at_the_code_they_name`:因本次改动使 `update.rs`/`app.rs`/`runtime.rs` 行号位移,同步修正 7 处引用(`app/update.rs` 5307→5317、1631/1638/1641→1641/1648/1651;`app/app.rs` 1242/1252/1259→1247/1257/1264;`runtime.rs` 725→729)后通过。

- **提交:** `9b7b5121` refactor(codehealth): adopt shared surface lifecycle (Z)
- **偏差:** 无。
- **未决项 / 阻塞:** 无。

---

## Task 10 — 兼容桥 + 黄金样本(计划 §Task 10 `:151-162`)

- **目标（计划 §Task 10 `:151-162`）：** 把 Dozer 历史布局迁移到共享 `Layout`;先存真实样本与黄金期望;产品提供"枚举 → 稳定 `PanelId`"映射与默认布局,宿主提供与产品无关的迁移流程(版本闸门 / 未知保留 / 原子落盘 + 备份)。
- **范围裁决（用户批准,2026-10-12）：** 选"兼容桥 + 黄金样本"——**只抽取可移植的布局 DTO 映射**(Dozer 历史 `layout.json`/`rail_layout` → `bytehost-core::Layout`),配黄金样本、迁移前备份、未知更高版本拒绝、原子落盘。rail/pane 渲染与 focus/overlay **暂留 Dozer**,作为已记录、待逐条登记的兼容桥(clause 160:不把"文件搬走"当作行为验证;`chrome/rail.rs` 1444 行、`app/layout.rs` 1514 行与 `PanelKind`/`App`/`ShellState` 深度耦合,整块抽取有像素行为风险)。不做完整抽取/overlay 抽取,除非再获批准。
- **规格依据:** §5.2 `:97-103`;`api-decisions.md` §9。

**实现:**
- **宿主侧通用流程(`bytehost-desktop/src/layout.rs`,新):** `trait LegacyPanelAdapter{ rails_from_legacy(&Value)->Vec<Vec<String>>; map_token(&str)->Option<PanelId>; default_rail_width()->f32 }`;`migrate_legacy_layout`(版本闸门:`schema_version > LAYOUT_SCHEMA_VERSION` → 拒绝;产品切栏 + 逐 token 映射;映射不出 `PanelId` 的历史 token 收进 `MigrationOutcome.retained`);`migrate_legacy_file`(缺失/损坏 → `Ok(None)`);`atomic_save_layout`(建目录;`<stem>.json.bak` 一次性备份;写 `.json.tmp` 再 `rename`);`reconcile_with_catalog`(序列化再走 core `Layout::load`,做钳制 + 未知摘进 `retained_unknown`)。`lib.rs` 注册模块与再导出;新增 `serde_json` 依赖。
- **产品侧薄包装(`dozer-app/src/app/layout_compat.rs`,新):** `panel_id_for(PanelKind)->PanelId`(内置 → `dozer.<slug>`,`slug` 为小写 kebab;应用 → `PanelId::for_app(slot.id())`)、`builtin_for_panel_id`/`app_slot_for`/`panel_kind_for`(反向)、`token_to_panel_id`(历史 `"Files"`/`"app:<id>"` → 稳定 id)、`DozerLayoutAdapter`(切 `rail_layout.left/right`)、`default_layout(&RailLayout, width)`。`builtin_kind_by_token` 加在 `app/state.rs`,与 `BUILTIN_NAMES` 同源(避免第二份真相)。
- **接线状态:** 本模块是计划 clause 160 的"Dozer 先以薄包装接入"步骤,**尚未接入 app 启动/落盘路径**(真实接入需真实窗口布局回归,单列后续提交),故模块级 `#![allow(dead_code)]` + 注释说明。

**测试:**
- bytehost `tests/layout_compat.rs`(8):假产品适配器(两栏 + `"Files"`/`"Usage"`/`"Agent"` token)钉住——`legacy_rails_translate_to_stable_ids`;`unknown_tokens_are_retained_not_dropped`;`newer_schema_version_is_rejected`;`non_object_legacy_is_rejected`;`migrate_file_missing_or_corrupt_yields_none`;`unknown_valid_ids_go_to_retained_unknown_via_catalog`;`atomic_save_writes_and_backs_up_once`(首次无备份、二次生成且保留旧内容、再存不覆盖);`empty_and_single_rail_legacy_is_legal`。
- dozer `app/layout_compat.rs` 内嵌黄金样本(7):`golden_current_layout_json_maps_builtin_rails_in_order`(现役双栏形状 → `dozer.*` 顺序保持);`golden_app_token_maps_to_app_namespace`;`golden_legacy_dims_only_yields_empty_rails`(老尺寸字段被忽略、缺 `rail_layout` 退化空栏);`golden_newer_schema_is_rejected`;`golden_unknown_token_is_retained`;`panel_id_and_kind_round_trip_for_all_builtins`;`default_layout_mirrors_default_rail_order`。

**验证:**
- bytehost:`cargo fmt --all` 干净;`cargo clippy --workspace --all-features --all-targets -- -D warnings` 干净;`cargo test --workspace --all-features` 全绿(含 layout_compat 8 通过);`scripts/check-deps.sh` ok。
- dozer:`cargo fmt -p dozer-app` 干净;`cargo clippy -p dozer-app --all-targets` 干净(仅两条预存在 `agent_token_share`/`GitProvider::ALL` 未用告警);`cargo test -p dozer-app --bin dozer`:**2025 通过 / 1 预存在失败**(`extensions::files::tests::delete_confirm_spec_reflects_pending_target`,非本次改动);layout_compat 子集 7 通过。
- 边界门禁 `scripts/audit/check_panel_boundary.py`:仅预存在 `app/app.rs: R-HOST-EXECUTORS 0 -> 1`。

- **提交:** `f3edcef` feat(desktop): migrate legacy layouts to stable Layout (B);`a152a51d` feat(app): compat bridge from legacy Dozer layout to shared Layout (Z)
- **偏差:** 模块暂未接线(见上);rail/pane 渲染与 focus/overlay 抽取留待后续兼容桥条目。
- **未决项 / 阻塞:** 真实样本已以黄金单测固定但尚未接入真实读盘路径;整体验收仍受 Task 08 真实窗口人工确认阻塞。

### Task 10 续 — 焦点机制抽取(2026-10-12)

- **裁决（用户批准,推荐项）：** 只抽取**零依赖、与产品无关**的机制。侦察结论:建窗机制把 `iced_wgpu`/`iced_widget`/`iced_renderer`/`winit` 与渲染器类型一起拉进来,与 bytehost-desktop 刻意不绑渲染器 feature 的约束冲突(`Cargo.toml:12-14`,即 R2 修复 `fbc3a636` 移除的东西);rail/pane 需要尚不存在的 `PanelId` 键控 API。故本续只做纯 `FocusTracker`。
- **实现:** bytehost-desktop 新增 `src/focus.rs`(`FocusTracker` + `focus_transition`,`lib.rs` 注册/再导出);Dozer 删除 `platform/overlay_focus.rs`,10 个原生浮层宿主改 `use bytehost_desktop::FocusTracker;`,`platform/mod.rs` 摘除模块,`search_overlay.rs` 文档引用同步。
- **登记:** 建窗机制(C10)、rail/pane 容器(C11)、焦点路由(C12)登记为待收口兼容桥,写入 `acceptance.md` §2。
- **验证:** bytehost fmt/`clippy -D warnings` 干净;`cargo test --workspace --all-features` 全绿(bytehost-desktop 34,含 focus 2;`bytehost-apps` 的 `a_python_app_runs_through_the_wire_and_dies_with_dozerd` 为既有偶发,单跑通过);`scripts/check-deps.sh` ok。dozer fmt/clippy 干净(仅两条预存在告警);`cargo test -p dozer-app --bin dozer` 2023 通过 / 1 预存在失败;边界门禁仅预存在项。
- **提交:** `93b2ac3` feat(desktop): extract product-neutral overlay focus tracker (B);`239aa399` refactor(platform): use shared overlay focus tracker (Z)
- **偏差:** overlay 建窗/rail/pane/焦点路由三项**未抽取**(理由见上,已登记 C10–C12)。
- **未决项 / 阻塞:** 上述三项待 Task 21 收口;整体验收仍受 Task 08 真实窗口人工确认阻塞。

---

## Task 11 — Digger 正式 workspace 与桌面组合根(计划 §Task 11 `:164-174`)

- **目标（计划 §Task 11 `:164-174`）：** 建 Digger 正式 workspace 与桌面组合根,接共享 Host + byteui;显式排除一次性 spike;注册产品信息/状态面板 + 应用区占位(明示未接服务);自有数据根/主题/布局保存/退出,**不复制** Dozer `App`/`Workspace`;建依赖门禁。
- **规格依据:** §4 所有权 `:67-70`;§5.2 布局 `:97-103`;§6 Digger 应用平台 `:128+`;§8 S3 `:193`;§9 A1/A15/A16;ADR-001/011/012。

**实现（Digger 仓库,新）：**
- **根 workspace(`Cargo.toml`,`Cargo.lock`):** `members = ["crates/*"]`,`resolver = "3"`,edition 2024;`exclude` 显式列出全部 `spike/*`(各自带 `[workspace]`,不进生产锁文件)。`.gitignore` 排除 `/target`、`.cargo/config.toml`(本地兄弟目录 patch,不提交)、`.digger/`。
- **`crates/desktop`(`digger-desktop`,bin `digger`):** 依赖 `bytehost-core`/`bytehost-desktop`(tag `v0.1.1`,本地 patch 兄弟目录)、`byteui`(tag `v0.4.1`)、`iced`/`wry`/`serde`/`tokio`/`directories`。
  - `src/paths.rs`:独立产品数据根——`config_dir()` = `ai.byteboy.digger`(与 Dozer 的 `ai.byteboy.dozer` 不同),`bytehost_root()` = `<config>/digger/bytehost/`(ADR-011 / 架构 §6),`layout_path()`/`state_dir()`/`logs_dir()`;`ensure_data_root()` 启动建目录。
  - `src/product.rs`:`ProductShell` 实现 `bytehost_desktop::Product<Msg>`,**无 Project 启动**;`PanelCatalog` 注册 `digger.info`/`digger.apps`(零 Surface 的纯 iced 面板);信息面板显示数据根/宿主根/无 Project/启动说明,应用区占位明确标注"服务尚未接通(Task 12/13)"、"不会伪造任何安装或运行能力";`DiggerLayoutAdapter` 实现 `LegacyPanelAdapter`(稳定 id 透传),`load_layout` 走 `migrate_legacy_file` + `Layout::load` 回落 `default_layout`;`atomic_save_layout` 退出时落盘;`boot()` 设 ByteBoy2077 主题(`color`/`font`)+ 确保数据根。
  - `src/main.rs`:`iced::application(boot, update, view)`,产品消息经 `HostMessage<Product>` 包装进入宿主(证明产品消息不进宿主 enum)。
- **`scripts/check-host-deps.sh`（新）：** 业务层 crate(`BUSINESS_PREFIX` 前缀)不得依赖 `bytehost-*`;宿主适配层白名单 `digger-desktop`/`digger-app-host`。当前仅 `digger-desktop` 存在,门禁审视 0 个业务 crate 并**打印 COVERAGE GAP**:领域/持久化/业务运行时 crate 尚未创建,新建后落入前缀即自动纳入。`--self-test` 在 `crates/_host_deps_probe` 注入一个依赖 `bytehost-core` 的 `digger-domain-probe`,断言门禁**必须失败**,移除后必须通过。

**测试(`digger-desktop` 内嵌,9 通过):**
- paths(5):`config_dir_ends_with_digger`;`config_dir_is_not_dozer`(A15);`bytehost_root_is_under_config`;`layout_path_is_under_config`;`state_and_logs_dirs_are_under_digger_root`。
- product(4):`starts_without_project`(A16);`registers_product_panels`;`has_default_layout`;`layout_round_trips_through_adapter`。

**验证:**
- `cargo check -p digger-desktop`:干净。
- `cargo test --workspace`:9 通过 / 0 失败。
- `cargo clippy --workspace --all-targets -- -D warnings`:干净。
- `cargo fmt --all -- --check`:干净。
- `scripts/check-host-deps.sh`:ok;`--self-test`:ok(注入即失败、恢复即通过)。
- **最小窗口手动运行:** `target/aarch64-apple-darwin/debug/digger` 启动 5s 保持运行、日志空(无 panic);数据根 `~/Library/Application Support/ai.byteboy.digger/{bytehost,logs}` 被创建,与 `ai.byteboy.dozer` 分离(A15)。

- **提交:** `3072d62` feat(desktop): bootstrap Digger on bytehost shell (D)
- **偏差:** 应用服务/app-host 未接(计划 Task 12/13);应用区为纯 iced 占位,不创建 wry 子视图(无 Surface 承载);`FocusTracker` 等浮层机制未引入(本壳无独立浮层窗口,避免未接线死代码)。
- **未决项 / 阻塞:** 依赖门禁当前无业务 crate 可查(已记录 COVERAGE GAP);整体验收仍受 Task 08 真实窗口人工确认阻塞。

---

## Task 12 — 补齐共享应用服务的管理能力(计划 §Task 12 `:176-186`)

- **目标（计划 §Task 12 `:176-186`）：** 以 Task 03 结论(`gates.md` 闸门 3,两处 AppService 缺口)为准,在 `bytehost-apps` 补齐:a) 同根双服务互斥;b) 受控端口重置 API;c) 可观察 shutdown 结果;d) 孤儿回收核对进程身份;e) 只补缺失测试,不写第二套 API,更新公共 API 兼容记录。
- **规格依据:** §6.1 `:134`("退出失败显示可操作错误,不静默遗留子进程")、`:138`(端口重置流程);§11 闸门。

**现场核实(Task 12 开工前):**
- (a) 同根互斥 **缺失** → 实现。
- (b) 受控端口重置 API **缺失** → 实现。
- (c) `shutdown()` 返回 `()`、无可观察结果 → 补兼容入口。
- (d) 孤儿回收核对进程身份 **已实现且有测试**(`process/supervise.rs` `reap_orphan`:命令行 + `lstart` + 进程组组长三重比对;测试 `orphans_are_matched_by_start_time_and_never_by_a_bogus_or_non_leader_pid` @ `:676`,故障场景仅针对测试自建的 `sleep` 进程组)→ **只验证,零代码改动**。

**实现(bytehost 仓库,新/改):**
- **`bytehost-apps/src/service_lock.rs`(新,`#[cfg(all(feature="server",unix))]`):** `<root>/service.lock` 上一把 `flock(2)` 独占锁(`LOCK_EX|LOCK_NB`,`EWOULDBLOCK` → `AlreadyOwned`),fd 显式 `O_CLOEXEC` + `fcntl(F_SETFD,FD_CLOEXEC)` 双保险,spawn 的应用进程不会继承持有;不新引入 crate 依赖。
- **`service.rs`:** `AppService` 增 `lock: Mutex<Option<ServiceLock>>`;`start`/`start_with`/`start_with_monitor_for_test` 先 `acquire_lock` 再进 `start_with_locked`;拿不到(同根已有所有者)→ `Unavailable("…已有一个应用服务在运行")`,**绝不触碰第一方**;`shutdown`/超时/panic 各路径都 `release_lock`(内核进程退出也自动释放,崩溃安全)。
- **受控端口重置(新公共 API):** `AppService::reset_port(root, requested) -> Result<u16, PortResetError>` 与 `clear_port(root)`:先做**占有权校验**(取同一把锁,服务在跑即 `ServiceRunning` 拒绝),再经 `port::persist_port`(临时文件 + rename,原子)**只改 `<root>/gateway.json`**;写失败时旧文件原样保留(可恢复);范围校验 → `OutOfRange`;不暴露任意写内部文件能力。
- **可观察 shutdown(新公共 API,保留兼容):** `shutdown()` 仍是 `()` 包装(失败记日志,现有调用方不动);新增 `shutdown_reporting() -> Result<ShutdownReport, ShutdownError>`(及可配超时的 `shutdown_reporting_within`):`ShutdownReport{app_failures, problems, gateway_released}`、`ShutdownError{TaskFailed, TimedOut, GatewayNotReleased}`;收尾上界 `AppService::SHUTDOWN_TIMEOUT`(30s,`suspend_all` 按应用给 `DEFAULT_GRACE` 后留足余量)。超时/失败也放锁让进程能退出,遗留子进程由下次启动的孤儿回收兜底。

**测试(增量,不写第二套 API):**
- `service_lock`(3):同进程二次 `acquire` 被拒直至首个 `drop`;锁文件落在 `<root>/service.lock`;跨进程(`python3 -c 'fcntl.flock'` 持锁)被拒、其退出后可拿。
- `service`(6):`a_second_service_on_the_same_root_is_unavailable_and_never_disturbs_the_first`(第二个 `Unavailable`、第一个 gateway/应用不受影响、首个 shutdown 后第三个可接管);`reset_port_changes_the_persisted_port_when…`、`reset_port_is_refused_while_a_service_owns_the_root`、`reset_port_rejects_out_of_range_ports_without_touching_the_file`、`reset_port_keeps_the_old_file_when_persisting_fails`、`clear_port_forgets_the_port_but_only_when_nobody_owns_the_root`;`a_clean_shutdown_reports_gateway_released_and_no_failures`、`a_shutdown_that_overruns_the_timeout_is_reported_not_swallowed`、`a_stop_failure_shows_up_in_the_report_not_only_in_logs`(目录置 0o500 令 `set_observed` 落盘失败 → 逐应用报进 `app_failures`)、`shutdown_reporting_on_an_unavailable_service_is_clean`。退出结果与失败**由返回结构证明,不用日志推断**。
- **锁释放修正(实测发现,非计划项):** `flock` 锁隶属"打开文件描述",`fork`→`exec` 窗口内被并发 `Command::spawn` 的子进程短暂共享,导致 `drop` 单靠 close 偶发不释放、同根重启偶发误判 `Unavailable`。`ServiceLock` 的 `Drop` 改显式 `LOCK_UN` 再关 fd(close 仍兜底)。

**验证(bytehost):**
- `cargo test -p bytehost-apps --all-features --lib`:421 通过 / 0 失败(并行 `service::tests` 反复跑不再抖动)。
- `cargo test --workspace --all-features`:全绿(唯一 `FAILED` 为既有事件时序 flaky `manager::tests::a_python_app_installs_starts_runs_behind_the_gateway_and_stops`,单独跑稳定通过,非本次改动)。
- `bytehost-client --all-features`(embedded_host / in_process / conformance)全绿。
- `cargo fmt --all -- --check` 干净;`cargo clippy --workspace --all-features --all-targets -- -D warnings` 干净;`scripts/check-deps.sh` ok。

- **提交:** `ccb32a8` feat(apps): one service owner per data root via flock;`b332684` feat(apps): controlled gateway port reset management API;`7daa064` feat(apps): observable shutdown result without breaking callers;`3ab1c34` fix(apps): release the service lock explicitly on drop(B)。按互斥 / 管理接口 / 退出观测拆分。
- **偏差:** `shutdown()` 保持 `()` 兼容入口(计划要求"保留现有调用方兼容"),Dozen `dozerd/src/main.rs:181` 调用点**不改**;`SHUTDOWN_TIMEOUT` 常量名取自定义项(计划未指定);孤儿回收项按现场核实为"已实现",未新增代码。
- **未决项 / 阻塞:** 无。端口重置的 GUI 流程(停服务 → 撤 Surface → 说明 origin 影响 → 确认 → 重启)由 Task 13 app-host 编排消费本 API。

---

## Task 13 — Digger app-host 适配与事件闭环(计划 §Task 13 `:188-198`)

- **目标（计划 §Task 13 `:188-198`）：** 在 Digger 侧新增 `crates/app-host`(包名 `digger-app-host`),把 `bytehost-panel` 的状态机、`bytehost-client` 的后端 API 与 Digger 的产品语义接成闭环:a) fake `AppHostApi`;b) 泛型 effect 执行器,生产用 `InProcess`,启动服务异步不阻塞窗口;c) 复用 host/install/logs 状态,`Changed`/`Resync` 合并刷新、`Disconnected` 按共享退避重订阅;d) 列表/launch URL/日志结果带 scope/request 身份,卸载与面板关闭使 UI 结果失效而写操作仍由服务完成;e) 单一服务实例 + 取消令牌,退出先停新操作再 `await shutdown`。
- **规格依据:** §4/§5.2(`:67-70`/`:97-103`);§5.3(身份与失效);§5.4;§6.1 `:134`("退出失败显示可操作错误,不静默遗留子进程")、`:138`(端口重置流程);§11 闸门。

**关键设计裁决(编译期逼出,非计划预设):**
- **`AppHostApi` 是 RPITIT(`impl Future` 返回),不是 dyn-safe** → 不能 `Arc<dyn AppHostApi>`。因此 `Executor`/`HostService` **全程泛型于 `A: AppHostApi`**:生产 `A = InProcess`,测试 `A = FakeHost`。后端句柄放 `Arc<tokio::sync::RwLock<A>>` 之后,使"启动占位 → 真正服务"就地替换(`*guard = new_value`)不改类型。
- **`InProcess` 不是 `Clone`** → `install_service` 拿其内部 `Arc<AppService>` 经 `InProcess::new(service)` 重建等价后端。
- **`AppKeyId`(Digger 的 `AppKey`)**:`bytehost_apps::id::AppId` 持有 `String`、非 `Copy`,而面板状态机要求 `K: AppKey + Copy`。故 `digger-app-host` 定义 `pub struct AppKeyId(u32)` + 进程级驻留表(`Mutex<OnceLock<Table>>`,`Box::leak` id 串,`HashMap<String,AppKeyId>` ↔ `Vec<&'static str>`),使 `AppKeyId` 为 `Copy + 'static`。注意:`slot.app_id()` 解析到 **trait** 方法返回 `&str`;要 `owned AppId` 用 `slot.into_app_id()`。

**实现(Digger 仓库,新):**
- **`Cargo.toml`:** 包 `digger-app-host`;依赖 `bytehost-apps`(features `["server"]`)、`bytehost-client`(features `["in-process"]`,蕴含 `bytehost-apps/server`)、`bytehost-panel`、`tokio`、`serde`;dev-dep `serde_json`。
- **`src/lib.rs`:** crate 文档(Task 13、规格 §5.3/§6.1、依赖方向);模块接线(`effects`/`request`/`service`/`subscription`/`testing`);`AppKeyId` + `AppKey` impl + `into_app_id`;驻留表。
- **`src/request.rs`(身份台账):** `ScopeId`/`PanelInstanceId`/`RequestTicket{scope,panel,generation,key,sequence}`/`RequestKey{List,LaunchUrl,Logs,Action}`(`latest_only()` 仅对同键的最新一次有效);`RequestTable::dispatch`/`accept`(一次性消费)/`invalidate_panel`/`invalidate_app`/`bump_generation`。**卸载 `invalidate_app` 只作废该应用的 UI 结果,写操作(如卸载本身)身份不受影响;面板关闭 `invalidate_panel` 作废该实例全部结果。** 7 单测。
- **`src/subscription.rs`:** `change_to_message(AppChange) -> PanelMessage<AppKeyId>`(`Changed`/`Resync` → `PanelMessage::Changed`;`Disconnected` → `PanelMessage::SubscriptionLost`);`SUBSCRIBED` 常量。2 单测。
- **`src/service.rs`:** `ShutdownOutcome{Reported,Failed,NeverStarted}`;`HostService<A>{api,service,cancelled}`;生产 `impl HostService<InProcess>` 的 `starting()`/`unavailable(reason)`/`install_service(Arc<InProcess>)`;泛型 `with_api`/`with_api_lock`/`api()`/`cancel_token()`/`is_shutting_down()`/`begin_shutdown()`/`await_shutdown()`;`CancelToken{Arc<AtomicBool>}`。4 单测。
- **`src/testing.rs`:** `FakeHost`——可编程 `AppHostApi`(成功 / `Host` 带类别失败 / `Transport` 无类别失败;`SubscribeMode{Live,Disconnected,Unavailable}`;请求日志与计数);未预设调用**显式失败**(`Transport("意外应答: …")`)。8 单测。
- **`src/effects.rs`:** 泛型 `Executor<A>{api,requests,sink,cancel}`;`Outcome`(`List`/`LaunchUrl`/`Action`/`Logs`/`Install`/`ServiceReady`/`ServiceFailed`,手写 `Debug`,不打印句柄/秘密);`ReplySink` trait;`run(scope,panel,generation,PanelEffect)` 处理 `FetchList`/`FetchLaunchUrl`/`StartApp`/`StopApp`/`FetchLogs`,UI-only 效果返回 `false` 交组合根;`run_install(...)` 处理 `Probe`/`List`/`Install`/`Stop`/`Rollback`/`Uninstall`/`FetchLogs`;**审批门仍由组合根把守**,执行器只发已批准的请求;`Executor<InProcess>::start_service` 异步建服务、回 `Outcome::ServiceReady`,不阻塞窗口。派发前核对取消令牌:退出已开始一律拒绝。

**测试(增量):**
- 单测 21:身份台账(7)、订阅映射(2)、宿主生命周期(4)、fake 后端(8)。
- 集成 `tests/embedded.rs` 12:状态机 → 执行器 → fake 后端 → 回信 → 状态机完整回路;在途时 `Changed` 合并成一次补拉;`Disconnected` 退避到点重订阅;跨代迟到回包按身份丢弃;卸载作废该应用 UI 结果而写操作照旧、面板关闭作废该实例全部结果;`begin_shutdown` 拒绝新派发;安装流 `Probe` effect 派发并带回信身份;UI-only effect 留给组合根;推送变更转成状态机消息;**真实临时数据根嵌入 `InProcess`**(退出干净 + 空根列表可达)。

**验证(Digger):**
- `cargo test -p digger-app-host`:21 + 12 全绿。
- `cargo test --workspace`:全绿;`cargo fmt --all -- --check` 干净;`cargo clippy --workspace --all-targets -- -D warnings` 干净;`scripts/check-host-deps.sh` ok。

- **提交:** `2305775` feat(app-host): wire InProcess effects and subscriptions(D)。
- **偏差:** `AppHostApi` 非 dyn-safe 与 `InProcess: !Clone` 逼出"全程泛型 + `Arc<RwLock<A>>` 就地替换"设计(计划未预设);`AppKeyId` 驻留表为满足 `Copy` 约束新增(计划未预设);`HostService::install_service` 用 `try_write().expect(..)`——启动占位读取窗口极短,列 Task 14 复核,当前测试全绿。crate 尚未接入 `digger-desktop` UI(计划归 Task 14)。
- **未决项 / 阻塞:** 无。GUI 编排(接列表/审批/日志/退出呈现)由 Task 14 消费本 crate。

---

## Review 修复 (2026-10-12, R1–R7)

> 来源：`review-2026-10-12-tasks-01-08.md`（快照 Dozer `b8ae6bb3` / bytehost `f4639bb`）。七项均经真实代码复核为**有效缺陷**（非误报）。修复顺序 R5/R6 → R4 → R2 → R3 → R1 → R7。

| 编号 | 位置 | 修复 | 仓库 |
|---|---|---|---|
| R5 | `bytehost-core/src/registry.rs` `register` | 允许零 Surface（纯 iced 面板）；`NoSurfaces` 错误替换为 `DuplicateSurface{id,slot}`（同一面板内槽位重复） | B |
| R6 | `bytehost-core/src/id.rs` `PanelId::new` | `app:` 命名空间用 `valid_app_name_segment`（允许数字开头、`[a-z0-9-]`、不含 `_`），与已校验 `AppId` 形状兼容 | B |
| R4 | `bytehost-core/src/surface.rs` | 新增 `ready_once`：`Hidden + Resumed` 仅在曾握手过时 → Ready，否则 WaitingReady；`Hidden + Ready` 记住握手、清 push、保持 Hidden | B |
| R2 | `bytehost-desktop/src/surface/mod.rs` | `register` 原子清除同 id/同物理槽的旧 entry 与路由；`unregister` 加占用者检查（旧 id 的迟到注销不得摘掉新实例路由） | B |
| R3 | `bytehost-core/src/request.rs` `run_scoped` | 更正文档契约：`accepts` 是**尽力而为的入队前预过滤**，权威校验必须在消费点（Dozer `group_chat` 已在消费处再校验，确认为既有行为）；补边界测试 | B |
| R1 | `dozer-app` `runtime.rs`/`window_events.rs`/`app.rs`/`extensions/usage/surface.rs` | 同帧"销毁+重建"（整池 clear 后同帧重建）经 `PoolSyncOutcome.dropped` → `observe_instance(in_pool, native_replaced)` 显式通知，开新代数；不再仅凭"下一帧数字槽还在池里"推断实例未变 | Z |
| R7 | `bytehost/spike/desktop-contract/src/main.rs` | SELFTEST 返回并真正执行 `build`（建 wry 子视图），等待 `HostBuilt(Ok)` 后退出；失败/建窗失败以非零码退出。Task 02/03 证据与状态据实修订 | B |

**验证（修复后）：**
- bytehost：`cargo fmt --all --check` 干净；`cargo clippy --workspace --all-features --all-targets -- -D warnings` 干净；`cargo test --workspace --all-features` 全绿（bytehost-core 43 单测 + 7 + 6；bytehost-desktop 32 + 3 + 8）。
- dozer：`cargo fmt -p dozer-app --check` 干净；`cargo test -p dozer-app --bin dozer` 2011 通过 / 1 预存在失败（`extensions::files::tests::delete_confirm_spec_reflects_pending_target`，非本次改动）；usage 子集 101 通过（含新增 R1 单测与 `s5b` 场景）。
- spike：`SELFTEST=1 cargo run` 退出码 0 且已真建子视图（失败路径结构性非零退出）。

**结论：** R1–R7 全部修复并有测试/证据。Task 01–08 验收仍受 Task 08 真实窗口人工确认阻塞（见上）；除该人工项外，review 提出的代码缺陷已清除。

---

## Task 14 — 安装审批与管理界面(计划 §Task 14 `:200-210`)

- **目标（计划 §Task 14 `:200-210`）：** 新增 Digger 的 `crates/desktop/src/apps/{mod,management,approval}.rs` 与 app-host 的 `trust.rs`、`tests/install_flow.rs`。(1) 列表/详情 + 本机目录/压缩包/URL 三种选择;计划生成失败 / 空列表 / 服务不可用各自可见。(2) 展示真实计划、来源、摘要、权限、强制等级;保留 LocalDir "整目录复制" 警示。(3) 人工确认**只接受当前这一份计划的身份**;重复点击防重;内容/来源变了要重新出计划。(4) LocalDir 可信**进程**应用经明确批准可装;Agent/URL 的 Untrusted 来源**只允许 static_web**;产品信任裁决**只在 Rust 适配层**。(5) 装完刷新列表;新应用用稳定 id;应用不绑定当前 Project。**验证:** fake 测试覆盖拒绝/取消/过期/重复批准;真实 static 应用从选来源到装完可见。
- **规格依据:** §4 `:67-70`(依赖方向);§5.2 `:97-103`;§5.3(身份与失效);§5.4;§6.1 `:134`、`:138`(端口重置);§6.2 `:140-155`(能力矩阵);§6.3 `:157-163`(审批/来源策略);§6.4 `:165-172`(应用 WebView);§11 闸门。

**关键设计裁决:**
- **产品信任裁决只在 Rust 适配层**:`trust::policy_for(SourceChoice) -> SourcePolicy{provenance,trust}`(Dir/Archive → Local/Trusted;Url → ThirdParty/Untrusted)在组合根派发 `Effect::Plan` 时注入(`run_install_with_policy`),返回的计划再经 `trust::review(&InstallPlan) -> Review` 复核(不受信来源的进程型 runtime 一律 `Rejected`)。**GUI 不自己算信任级别**;服务端 `AppSource::Url` 无论调用方传什么都会重解析为 ThirdParty/Untrusted,本机来源保留调用方值——Digger 的 `trust.rs` 提供本机默认。
- **`Effect::Plan` 的信任级别需要产品输入**:共享状态机的 `Effect::Plan(AppSource)` 只带来源,不带信任级别,故执行器新增 `run_install_with_policy(.., Option<SourcePolicy>)`;`run_install` 委托给它并传 `None`(默认 Local/Trusted)。
- **审批身份 == 展示的计划**:批准走状态机自身逻辑——`ApproveClicked` 只在 `Reviewing` 生效,就地 `plan.clone().approve(..)` 后进 `Installing`,**不重算**;重复点击此时不在 `Reviewing`,不再吐 `Install`。(**注:** 来源切换在 `Planning` 窗口内状态机不清理,但 GUI 的来源单选行只在 `Flow::Idle` 渲染,`Planning` 期间被 "正在检查应用…" 取代,用户无法在窗口内改来源,故该窗口在 Digger 里不可达——身份端到端成立。)
- **修复 Task 13 遗留的 `try_write().expect` panic 风险**:`HostService::install_service` 不再 `try_write().expect`(有并发读即 panic)。改为 **genesis 任务在 tokio 工作线程用 `api.write().await` 换后端**(窗口线程不能阻塞等锁),`install_service` 只记录 `Arc<AppService>` 供 shutdown,并在锁空闲时做等价的就地后备写入,`try_write` 失败即跳过(后端已是真服务)。新增测试 `the_start_service_task_installs_the_backend_before_announcing_ready` 钉住"`ServiceReady` 到达时共享后端已是真服务"的时序。
- **应用面板启动延迟到首次打开**:`AppsState::new()` 不再在构造期 `start_service`(那会在单元测试里脱离 tokio 运行时 panic);改在首次 `Message::Opened`(永远在 iced 运行时内)触发,`ServiceReady` 到达后经 `service_ready()` 再喂一次 `Opened` 做真正的探测 + 拉列表。
- **`Outcome` 回流用 `iced::Task::stream` 而非 `Subscription`**:接收端唯一、不该反复重建;`Task::stream` 只在 `boot` 挂一次。`Message` 需 `Clone + Debug`,故 `Outcome` 以 `Arc<Outcome>` 携带(自带手写 `Debug`),`ServiceReady`/`ServiceFailed` 不再经 `Message` 中转,直接在 `deliver` 内按 `Arc` 借用处理。
- **原生选目录/压缩包走 `rfd::AsyncFileDialog`**:iced 0.14 无 winit 主线程钩子(Dozer 在 `window_events.rs` 拦截),故用 rfd 的 async API 在既有 AppKit run loop 内跑,经 `Task::perform` 挂到窗口。

**实现(Digger 仓库):**
- **`crates/app-host/src/trust.rs`(新):** `SourcePolicy{provenance,trust}` + `LOCAL_TRUSTED`/`THIRD_PARTY_UNTRUSTED`;`policy_for(SourceChoice)`;`Review{Allowed,Rejected(String)}` + `is_allowed()`;`PROCESS_RUNTIMES = ["node","python"]`;`review(&InstallPlan)`;`provenance_label`。模块文档声明产品信任裁决只在本适配层。5 单测。
- **`crates/app-host/src/effects.rs`:** `run_install_with_policy(...)` 新增;`run_install` 委托并传 `None`;`Outcome::Install` 增 `plan: Option<Box<InstallPlan>>`;`PLAN_ACTION = u64::MAX - 6`;`pub enum InstallAction {Plan,Install,Stop,Rollback,Uninstall,Probe,Genesis}` + `from_key(RequestKey)`(单一来源:把私有 action-id 常量映射到变体,组合根不再抄 id)。2 单测(7 个完成动作往返 + `List`/未知 `Action` → `None`)。`start_service` 改为在工作线程换后端再回 `ServiceReady`。
- **`crates/app-host/src/service.rs`:** `install_service` 去 panic(见上)。
- **`crates/app-host/src/testing.rs`:** `FakeHost` 增 `on_plan`/`on_install`/`on_uninstall`/`on_probe`;`canned_for` 映射 `AppRequest::Plan/Install/Uninstall/ProbeRuntimes`;pub `plan(id,version,kind,trust,provenance)` 便捷构造(手写 JSON,含 `upgrading_from: null`、`source_info`);`serde_json` 提到 `[dependencies]`(库内 `testing` 模块供集成测试复用)。
- **`crates/app-host/tests/install_flow.rs`(新):** 6 测试——本机进程应用到 `Reviewing` 且计划身份绑定;重复批准只装一次;取消不装;来源切换作废旧审批卡且迟到计划不复活;不受信来源的进程应用变流程内失败而非审批卡;计划请求失败在流程内呈现。
- **`crates/app-host/tests/embedded.rs`:** +2——`start_service` 先装后端再宣告就绪;真实 static 应用从本机来源 `app_plan` → 批准 → `app_install` → `app_list` 可见(稳定 id)。
- **`crates/desktop/src/apps/mod.rs`(新):** `Startup{Starting,Ready,Unavailable(String)}`;`Message{Opened,Install(InstallMessage),Outcome(Arc<Outcome>),SourcePicked(Option<PathBuf>)}`;`AppsState`(组合根:状态机 + 执行器 + 请求台账 + 回流通道 + 服务宿主);`update`/`step`/`dispatch`(PickSource/PickArchive → rfd 任务;`Plan` 注入 `trust::policy_for`)/`deliver`(按 `requests.accept(ticket)` 身份核验)/`install_outcome`(按 `InstallAction` 分支:Plan→`trust::review`→`PlanLoaded` 或拒绝失败,Install→`InstallDone`,Probe→`ProbesLoaded`)/`pick_task`(rfd async)/`take_reply_receiver`/`reply_stream`/失败翻译。
- **`crates/desktop/src/apps/approval.rs`(新):** `review_view(plan, running)` 渲染 `plan_view` —— 升级 notices(金)、facts、来源块(`from_network` → "来自网络,不可信" + "只允许静态应用,运行在严格 CSP 下"、`SourceDisclosure.rows/warnings`)、`will_run`(LocalDir 整目录复制警示经此带出)、权限(升级标 `↑` 金、`enforcement` 右对齐)、按钮 取消(`FlowDismissed`)/批准并安装(`ApproveClicked`)。
- **`crates/desktop/src/apps/management.rs`(新):** `view(state)` —— 顶部 `Unavailable` 原因(红)+ "修复后重开此面板会再试。" 且不出安装表单;`Starting` → "应用服务正在启动…";已装区(`Loading`/`Failed`/空/行=名称 + "版本 v · 稳定 id");安装表单(来源单选行、`InstallClicked`/`PickArchive`、URL 模式用 `byteui::form::input_text::view` + `url_error` 红 + "获取计划");`flow_view` 映射 Idle→表单、Picking→对话框提示、Planning→"正在检查应用…"、Installing→"正在安装…"、Failed→红 + "知道了"、Reviewing→`review_view`、运行时/确认分支 → Task 15 占位。`scrollable` + byteui 滚动条。
- **`crates/desktop/src/product.rs`:** `Msg` 增 `Apps(crate::apps::Message)`;`ProductShell` 增 `apps`;`SelectPanel(Apps)` 触发 `Opened`;`view` 接 `apps_view`;`boot` 把 `reply_stream` 挂成 `Task::stream` 与窗口尺寸任务 `Task::batch`。
- **`crates/desktop/{Cargo.toml,src/main.rs}`:** 依赖 `bytehost-apps`(server)/`bytehost-client`(in-process)/`bytehost-panel`/`digger-app-host`/`rfd`;`mod apps;`。

**测试(增量):** app-host 单测 28(含 `trust` 5 + `InstallAction` 2,原 21);集成 `embedded.rs` 14(原 12);集成 `install_flow.rs` 6(新);desktop 单测 9。

**验证(Digger):**
- `cargo test -p digger-app-host`(28 + 14 + 6)、`cargo test -p digger-desktop`(9)、`cargo test --workspace` 全绿。
- `cargo fmt --all -- --check` 干净;`cargo clippy --workspace --all-targets -- -D warnings` 干净;`scripts/check-host-deps.sh` ok(桌面依赖白名单含 `digger-desktop`/`digger-app-host`)。

- **提交:** `a21b730` feat(apps): add reviewed install and management flow(D)。
- **偏差:** `Effect::Plan` 不携带产品信任级别,逼出 `run_install_with_policy` 旁路(计划未预设);`install_service` 的 `try_write().expect` 改为工作线程 `write().await` 换后端 + 后备就地写入(修复 Task 13 遗留风险,计划未预设);回流用 `Task::stream` 而非 `Subscription`(接收端唯一,计划未预设);`Message` 携带 `Arc<Outcome>` 以同时满足 `Clone`/`Debug`。
- **未决项 / 阻塞:** **iced UI 是 build + 源码级验证;真实窗口交互验收受 Task 08 同一 OS 权限限制,待人工确认**(与 Task 08 人工项合并)。运行时装卸 / 回滚确认 / 卸载确认为 Task 15 占位。

---

## Task 15 — 运行时、日志、升级回滚和卸载操作(计划 §Task 15 `:212-222`)

- **目标（计划 §Task 15 `:212-222`）：** 新增/修改 Digger desktop `apps/{runtime,logs,actions}.rs` 与 app-host effects tests。(1) 接运行时 probe/plan/install/uninstall;显示依赖应用、失败原因和持续问题页。(2) 接 start/stop、upgrade plan、rollback;升级失败后再次拉取权威状态,不由 UI 猜运行版本。(3) 复用 `LogsState`:跟随/取消跟随、首次失败、保留旧文本、迟到回包丢弃。(4) 卸载 UI 明确保留数据与删除数据两种行为;先调用共享业务流程,浏览器清理接 Task 17。(5) 为能力矩阵每行记录 UI 入口、effect、API、成功和失败测试名称;无入口的 API 不算接通。**验证:** 管理界面状态测试、进程应用真实安装/启停;A8/A9/A13 的服务与 UI 部分。提交按运行时、日志、版本/卸载三组。
- **规格依据:** §4 `:67-70`;§5.2 `:97-103`;§5.3(身份与失效);§5.4;§6.1 `:134`、`:138`;§6.2 `:140-155`(能力矩阵);§6.3 `:157-163`;§6.4 `:165-172`;§11 闸门。

**关键设计裁决:**
- **扩展适配层而非共享 crate**:任务要求接运行时/日志/动作,但共享 `bytehost-panel::install` 的 `Outcome`/`InstallAction` 只覆盖 Task 14 的出计划/安装。**不修改 bytehost 与 `bytehost-panel`**(保持共享 crate 不动,避免版本 bump 与 Dozer 波及面),改为在 `digger-app-host` 内扩展 `Outcome`(新增 `Runtimes`、`Install.detail: InstallDetail`)与 `InstallAction`(新增 `Probe`/`RuntimePlan`/`RuntimeInstall`/`RuntimeUninstall`/`Stop`/`Rollback`/`Uninstall`,共 10 变体)。组合根只认适配层类型。
- **`Probe` 复用 `Outcome::Runtimes` 而非空占位**:执行器 `Probe` 分支返回真实 `Vec<RuntimeProbe>`(修掉 Task 14 `ProbesLoaded(result.map(|()| Vec::new()))` 的丢弃 bug);`InstallDetail::App(id)` 让每个 per-app 动作(Stop/Rollback/Uninstall)带回目标 id,组合根据此路由到对应 `Message`。
- **升级/回滚以"再次拉权威状态"为准**:回滚走 `RollbackClicked → ConfirmRollback → RollbackConfirmed → Effect::Rollback`;成功后状态机吐 `List` 刷新(权威 `AppSummary.previous_version`/`observed` 重取),UI 不猜运行版本。运行版本一律来自 `AppSummary.observed`,不由回滚结果推断。
- **日志复用共享 `LogsState`,不新建**:桌面侧 `logs.rs` 只把 `bytehost_panel::logs::LogsView` 渲染成 iced 视图;跟随/取消跟随、`stale` 保留旧文本、首次失败 vs 刷新失败、迟到回包(经 `requests.accept(ticket)` 身份核验)全在共享状态机 + 请求台账里。刷新定时由组合根 `arm_tick`/`Tick` 排一拍(`REFRESH_INTERVAL`),滚动钉底由 `take_log_scroll()` 一次性消费生成 `scroll_to` 任务。
- **卸载保留/删除数据二选一显式呈现**:`UninstallClicked → ConfirmUninstall → UninstallConfirmed(UninstallMode::{Program,ProgramAndData})`;UI 两个按钮文案明确"保留数据"/"删除数据",经共享状态机发起,浏览器数据清理归 Task 17。
- **提交按三组**:运行时(`effects.rs`/`testing.rs`/`embedded.rs`/`runtime.rs`)、日志(`logs.rs`)、版本/卸载(`actions.rs`/`management.rs`/`mod.rs` 共享接线/`install_flow.rs`)。`mod.rs` 含跨组共享接线(step 任务批处理、`log_scroll_id`),依计划"可独立审查"口径放在最后一组;各组 diff 自洽。

**能力矩阵接线记录(计划 §Task 15 第 5 条,每行:UI 入口 / effect / API / 成功测试 / 失败测试):**
| 能力 | UI 入口 | effect | API | 成功测试 | 失败测试 |
|---|---|---|---|---|---|
| 列表/详情/刷新 | `management.rs` 已装区(行=名称/版本/observed/id) | `Effect::List` | `app_list` | `embedded::install_then_list...`(Task 14) | `install_flow::...`(服务不可用页) |
| 本地目录/归档/URL 安装 | 安装表单来源单选 + `InstallClicked`/`PickArchive` | `Effect::Plan`→审批→`Effect::Install` | `app_plan`/`app_install` | `install_flow::a_local_process_app_reaches_review...` | `install_flow::a_rejected_source_becomes_a_flow_failure...` |
| 启动/停止/重新打开 | `actions.rs` 行按钮(过渡态不给卸载) | `Effect::Stop`;启动在 `bytehost_panel::host` | `app_stop`(启动 `app_start`) | `install_flow::stopping_an_app_reports_action_done` | — |
| 升级/回滚 | `actions.rs` 行"回滚到 X"(仅 `previous_version` 为 Some) | `Effect::Rollback` | `app_rollback` | `install_flow::rolling_back_an_app_requires_confirmation` | — |
| 卸载(保留/删除数据) | `actions.rs` 行"卸载"→确认二选一 | `Effect::Uninstall(id, mode)` | `app_uninstall` | `install_flow::uninstalling_an_app_goes_through_confirmation_and_reports_done` | — |
| 运行时探测/安装/卸载 | `runtime.rs` 运行时区块(装/卸按钮、job 进度、持续问题页) | `Effect::Probe`/`RuntimePlan`/`InstallRuntime`/`UninstallRuntime` | `app_probe_runtimes`/`app_runtime_plan`/`app_install_runtime`/`app_uninstall_runtime` | `install_flow::a_runtime_install_flows_from_click_through_approval_to_done`;`embedded::runtime_effects_carry_their_action_identity_and_payload` | `install_flow::a_runtime_plan_failure_is_shown_in_the_flow`;`uninstalling_a_runtime_requires_confirmation` |
| 日志 | `actions.rs` 行"日志/收起日志"(任意状态可用) | `Effect::FetchLogs` + `ProbeAfter`/`Tick` | `app_logs` | 共享 `logs::LogsState` 单测;`runtime.rs`/`logs.rs` 渲染 | 共享 `logs::LogsState` 单测(首次失败 vs 刷新失败) |
| 崩溃/依赖缺失/宿主不可用 | `runtime.rs`/`management.rs` 原生持久状态页(`AppIssue` 经 `issue_texts`) | — | — | `runtime.rs` 渲染 + 共享视图模型 | `management.rs::Unavailable` 页 |

**实现(Digger 仓库):**
- **`crates/app-host/src/effects.rs`:** `InstallDetail{None,Plan,App(String),RuntimePlan}`;`Outcome::Runtimes{ticket,result}`、`Outcome::Install.detail`;手写 `Debug` 覆盖 `Install`/`Runtimes`;动作常量 `PROBE_ACTION`/`RUNTIME_PLAN_ACTION`/`RUNTIME_INSTALL_ACTION`/`RUNTIME_UNINSTALL_ACTION`/`STOP_ACTION`/`ROLLBACK_ACTION`/`UNINSTALL_ACTION`;`InstallAction` 10 变体 + `from_key`(单测 `every_completion_action_round_trips_through_its_key` 覆盖全部);`run_install_with_policy` 各 effect 分支:`Probe→Runtimes`、`Stop/Rollback/Uninstall→App(id)`、`RuntimePlan→RuntimePlan`、`InstallRuntime`/`UninstallRuntime`、`FetchLogs`。
- **`crates/app-host/src/testing.rs`:** `FakeHost` 增 `on_probe`/`on_probe_host_fail`/`on_rollback`/`on_runtime_plan`/`on_runtime_install`/`on_runtime_uninstall`;`canned_for` 映射 `ProbeRuntimes`/`Rollback{id}`/`RuntimePlan`/`InstallRuntime`/`UninstallRuntime`。
- **`crates/app-host/tests/embedded.rs`:** +2——`runtime_effects_carry_their_action_identity_and_payload`(Probe→`Outcome::Runtimes` + `InstallAction::Probe`;RuntimePlan→`Outcome::Install` + `InstallDetail::RuntimePlan`);`per_app_actions_carry_their_kind_and_target_id`(Stop/Rollback/Uninstall 各带 `InstallDetail::App("notes")` 与正确 `InstallAction`)。(14→16)
- **`crates/app-host/tests/install_flow.rs`:** `Rig::dispatch` 增 `RuntimePlan`/`InstallRuntime`/`UninstallRuntime`/`Stop`/`Rollback`/`Uninstall` 分支;+6 测试(共 12):运行时安装全流程、运行时计划失败入流程、卸载运行时需确认、回滚应用需确认(先 `ListLoaded` 带 `previous_version`)、卸载应用经确认并报完成、停止应用报完成。
- **`crates/desktop/src/apps/runtime.rs`(新):** `runtime_section` 渲染 `RuntimePlanning`/`RuntimeReviewing`/`ConfirmRuntimeUninstall`,否则按 `probes: Load` 给 Loading / 持续问题页(红 + 重试 `PollProbes`)/ 逐行 `probe_row`;`probe_row` 用 `runtime_line`/`progress_text`/`managed_runtime_label`,job 失败内联红显示(不走 Toast),受管版本行带 `RuntimeUninstallClicked`,可自动安装者给"安装 X"(进行中→"安装中…"、docker→"此平台不支持由 Digger 自动安装")。
- **`crates/desktop/src/apps/logs.rs`(新):** `log_viewer(LogsView, app_id)` —— `Hidden→[]`、`Loading→"加载日志…"`、`Failed→红 + 保留文案`、`Loaded→` 滚动容器(`id = log_scroll_id`、`on_scroll` 回 `LogsScrolled`)+ "跟随/取消跟随" 切换 + `stale`/`truncated` 提示;`Shaping::Advanced` 保证中文正常。
- **`crates/desktop/src/apps/actions.rs`(新):** `app_row(state, app)` —— 名称/版本/`observed_label` + id、`AppIssue` 经 `issue_texts` 渲染、动作行(过渡态不给卸载;回滚仅 `previous_version` 为 Some;日志入口任意状态可用且展开时变"收起日志");展开时就地渲染 `log_viewer`。
- **`crates/desktop/src/apps/mod.rs`:** `update` 批处理 `step` 任务 + `log_scroll_task` + `log_tick_task`;`dispatch` 增 `ProbeAfter→sleep→PollProbes`;`deliver` 增 `Outcome::Runtimes`/`Outcome::Logs`(含滚动钉底);`install_outcome` 增 `RuntimePlan`/`RuntimeInstall`/`RuntimeUninstall`/`Rollback`/`Stop`/`Uninstall` 分支(按 `InstallAction` + `InstallDetail::App(id)`);新增 `log_scroll_id`。
- **`crates/desktop/src/apps/management.rs`:** 已装行改用 `actions::app_row`;`flow_view` 的运行时分支护到 `runtime_section`。
- **`crates/desktop/Cargo.toml` + 根 `Cargo.toml`:** 直接依赖 `iced_core`(iced 0.14 未公开重导出 `text::Shaping`,组合根按需依赖同版本)。

**测试(增量):** app-host 单测 28(不变);集成 `embedded.rs` 16(原 14);集成 `install_flow.rs` 12(原 6);desktop 单测 9。

**验证(Digger):**
- `cargo test -p digger-app-host`(28 + 16 + 12)、`cargo test -p digger-desktop`(9)、`cargo test --workspace` 全绿。
- `cargo fmt --all -- --check` 干净;`cargo clippy --workspace --all-targets -- -D warnings` 干净(修掉 `runtime.rs` 一处 `collapsible_if`);`scripts/check-host-deps.sh` ok。

- **提交(三组,仓库 D):** `b24dd20` 运行时;`8a098d4` 日志;`aa91dfd` 版本/卸载 + 共享接线。
- **偏差:** 共享 `bytehost-panel` 状态机对"打开日志时迟到回包丢弃"依赖请求台账身份核验而非状态机内计数(与 Task 14 同口径,计划未预设具体机制);`iced_core` 直接依赖为绕开 iced 0.14 未导出 `text::Shaping` 的替代(计划未预设);`mod.rs` 共享接线随最后一组提交(见上"提交按三组")。
- **未决项 / 阻塞:** **iced UI 是 build + 源码级验证;真实窗口交互验收(含运行时真实安装/启停、回滚、卸载点击流)受 Task 08 同一 OS 权限限制,与 Task 08 人工项合并待人工确认**(与 Task 14 同口径)。

---

## Task 16 — 真实应用 Surface 与 builder 强制策略(计划 §Task 16 `:224-234`)

- **目标（计划 §Task 16 `:224-234`）：** (1) builder 接 `bytehost-webview` 的 `AppOrigin`/`AppIpc`/`data_store_identifier`;明确关闭下载、新窗口、devtools。(2) launch URL 从 effect 直接送内存 Surface,不进持久状态;销毁重开重新取地址。(3) 表格测试逐项检查 builder 决策;真实恶意页面检查跨 origin、伪造 nonce、自定义协议与合成事件结果。(4) 剪贴板按授权和实测能力保守处理,媒体保持无用途声明;记录无法强制的项,不虚报策略。(5) 静态/py-notes/node-notes 各运行到真实可交互内容;验证同 origin 正常跳转与焦点。**验证:** A7/A10/A11;探针结果与实际 builder 同源,不能只测复制的配置。
- **规格依据（§6.4 `:165-172`,决定性）：** "新 desktop 层统一实现 builder,产品只给策略配置与资源,不各自复制 wry 初始化代码" —— builder **必须**住在共享 desktop 层(`bytehost-desktop`),不注入产品自定义协议或业务 IPC,拒绝跨 origin 导航/新窗口/下载,devtools 所有构建下关闭,不声明摄像头/麦克风用途,剪贴板按授权+平台能力(无法强制则保守禁用),CSP 复用 Gateway 共享策略,launch URL 中 token 只驻内存。

**架构决策(锁定):**
- **builder 落在 `bytehost-desktop`(非仅适配层)**,依赖 `bytehost-webview`;Digger `apps/surface.rs` 驱动它。对 Dozer 纯增量、不破坏。bytehost 提交 `79e7d8d`;正式接入前 Digger 经本地兄弟仓库 patch(`digger/.cargo/config.toml`)解析 bytehost,不提交。
- **Digger 范围 = 控制器 + 接线,不做真实挂载**(用户裁决):Digger 一期无应用视口(无布局渲染、`PanelMeta.surfaces: vec![]`、应用面板 `HostState` 未驱动)→ 没有窗口 id + 矩形可挂真实 webview。故只交付 `AppSurfaces` 控制器 + `SurfaceAction → iced::Task` 助手(接窗口+矩形),仅表格测试;真实挂载与真实 WebView 运行与 Task 08 人工项合并(同 Tasks 14/15 口径)。

**实现(bytehost 仓库 B):**
- **`crates/bytehost-desktop/src/app_webview.rs`(新):** 共享受限应用 webview builder。
  - 类型:`AppSurfaceSpec{slot,url,rect,visible}`;`AppSurfaceBuilt{slot,result: Result<(),String>}`;`AppSurfaceIpc{Focus,MouseUp,ZoomIn,ZoomOut,ZoomReset}` + `From<AppIpc>`;线程局部 `APP_HANDLES: RefCell<HashMap<SurfaceSlot, wry::WebView>>`。
  - `build_app_child<M>(window_id, spec, on_ipc, on_ready, wrap) -> Task<M>`(经 `iced::window::run`)+ `set_app_bounds`/`evaluate_app`/`destroy_app`/`app_handle_count`。
  - 钉死的受限设置(逐项显式):`AppOrigin::from_url`(fail-closed)、`.with_download_started_handler(|_, _| false)`(wry 默认**放行**)、`.with_devtools(false)`(debug 默认**开**)、`.with_allow_link_preview(false)`(需 `use wry::WebViewBuilderExtDarwin;`)、`.with_clipboard(false)`(保守关闭;wry 的 clipboard 只影响 Linux/Windows,macOS 归系统,但显式写 false 让跨平台一致)、`.with_data_store_identifier(...)`、`.with_initialization_script(app_init_script(&nonce))`、`.with_ipc_handler`(只认 `AppIpc::parse(body,&nonce)`)、`.with_navigation_handler(origin.allows_navigation)`、`.with_new_window_req_handler(|_,_| Deny)`、`.with_on_page_load_handler`(Finished→`on_ready`)。
  - 测试:`build_app_child_pins_the_restrictive_settings` + `no_second_copy_of_the_navigation_policy`(均只扫生产源码:`&src[..src.find("#[cfg(test)]")]`,函数体从 `prod.find("pub fn build_app_child<M>(")` 起;**不可**用 `\n}\n` 作结束标记);`app_surface_ipc_maps_every_whitelisted_verb`。
- **`crates/bytehost-desktop/src/panel.rs`:** `rect_to_wry` 改 `pub(crate)`。
- **`crates/bytehost-desktop/src/lib.rs` + `Cargo.toml`:** 导出 `app_webview` 模块与类型;增 `bytehost-webview = { path = "../bytehost-webview" }` 依赖。

**实现(Digger 仓库 D):**
- **`crates/desktop/src/apps/surface.rs`(新):** `AppSurfaces` 控制器 + `SurfaceAction`。
  - `OpenApp{app,slot,generation,rect,open_error: Option<String>}`;`SurfaceAction::{Create{app,slot,generation,url,rect}, Resize{slot,rect}, Destroy{slot}}`。
  - `AppSurfaces{open,next_generation: HashMap<String,u64>,next_slot(起 4096),pending}`;方法 `new/is_open/get/slot_of/len/is_empty/open/reopen/close/close_all/resize/accepts/report_open_result/open_error/set_pending_url/pending_url/open_pending/take_pending/alloc_slot`;自由函数 `ipc_label(AppSurfaceIpc)->&'static str`。
  - **代数按 per-app 计**(不是 per-slot):`close` 使该应用 `next_generation` +1,旧实例的迟到回包被 `accepts(app,generation)` 拒;`open` 每次取新槽(不别名旧实例)。`close` 先丢 `pending`(不留过期地址);`reopen`/`open_pending` = `close` 再 `open`,代数 +1;已打开时 `open` 幂等。
  - 文件级 `#![allow(dead_code)]`(模块文档后),理由:机制就绪、视口后续接入。
- **`crates/desktop/src/apps/mod.rs`:** `mod surface;`;`use ... LogicalRect` + `surface::{AppSurfaces, SurfaceAction}`;`Message::SurfaceOpened(String, Result<(),String>)`;`AppsState.surfaces`;`deliver` 的 `Outcome::LaunchUrl{ticket,app,result}`:接单后 `Ok(url)→set_pending_url`、`Err→close`(不留凭证);`Message::SurfaceOpened→report_open_result`(持久状态,非 Toast——Digger 无 Toast/日志框架);表面访问器 `surfaces/surface_close/surface_open_pending/surface_resize/surface_close_all/take_pending_surfaces`(均 `#[allow(dead_code)]` + 注释);自由函数 `surface_task(window, action) -> iced::Task<Message>`(Digger 侧**唯一**碰 `bytehost_desktop::app_webview` 处;`on_ipc`/`on_ready` 暂 no-op,接入视口后改为经 `UnboundedSender<Message>` 回流)。
- **`crates/desktop/src/product.rs`:** `update_surface` 注释更新(应用 Surface 结果已在 `apps` 组合根处理)。

**实现(探针,Digger 仓库 D):**
- **`spike/editor-host/src/hosts.rs`:** 应用类别 `clipboard` 由 `true` 改 `false`(保守,与共享 builder 对齐);新增 **同源测试** `spike_app_pins_match_the_shared_builder_verbatim` —— `include_str!` 读共享 `bytehost-desktop/src/app_webview.rs`(只取 `#[cfg(test)]` 之前的生产段),断言其中确实写下 Spike 期望的每一项受限设置(`.with_download_started_handler(|_, _| false)`/`.with_devtools(false)`/`.with_clipboard(false)`/`.with_new_window_req_handler(`/`.with_navigation_handler(`/`.with_data_store_identifier(`/`.with_initialization_script(app_init_script(`/`AppIpc::parse(`/`AppOrigin::from_url(`),并断言**无** `with_custom_protocol`。任何一方漂移即失败 → 探针测的是真实 builder,不是复制配置。

**能力矩阵接线记录(计划 §Task 16 第 3/4 条):**
| 决策 | 强制点 | 表格测试 | 真实页面探针 |
|---|---|---|---|
| 仅本应用 origin 导航 | 共享 builder `.with_navigation_handler` + `AppOrigin::allows_navigation` | `build_app_child_pins_the_restrictive_settings`;`no_second_copy_of_the_navigation_policy`;Spike 同源测试 | 待 Task 08 人工项 |
| 拒绝新窗口 | 共享 builder `.with_new_window_req_handler(Deny)` | 同上 | 待 Task 08 人工项 |
| 拒绝下载(wry 默认放行) | 共享 builder `.with_download_started_handler(|_,_| false)` | 同上 | 待 Task 08 人工项 |
| devtools 全关(debug 默认开) | 共享 builder `.with_devtools(false)` | 同上 | 待 Task 08 人工项 |
| IPC 白名单 + per-webview nonce | 共享 builder `.with_initialization_script`+`.with_ipc_handler`(`AppIpc::parse`) | `app_surface_ipc_maps_every_whitelisted_verb`;Spike 同源测试 | 伪造 nonce 探针待 Task 08 人工项 |
| 无自定义协议 | 共享 builder 全程无 `with_custom_protocol` | `build_app_child_pins_the_restrictive_settings`(forbidden)+ Spike 同源测试 | 待 Task 08 人工项 |
| 每应用独立存储 | 共享 builder `.with_data_store_identifier`(实现只在 `bytehost-webview`) | `no_second_copy_of_the_navigation_policy`;Spike 同源测试 | 待 Task 08 人工项 |
| 剪贴板保守关闭 | 共享 builder `.with_clipboard(false)` | `build_app_child_pins_the_restrictive_settings`;Spike `app_webview_is_the_most_restricted_class`+同源测试 | 待 Task 08 人工项 |
| 媒体**无**用途声明 | 无 `Info.plist` 用途键(Digger 本阶段无 bundle;不新增) | — | 无法强制项,见下 |
| launch URL 只驻内存 | `handlers.rs` surface 登记;`deliver` 只送控制器;不入日志/布局/错误详情 | `surface.rs` 单测(下) | — |

- **无法强制的项(不虚报策略):** (a) macOS 剪贴板归系统管辖,wry 的 `clipboard` 开关对 macOS 无效 → 显式 `false` 只是跨平台一致性,不宣称"应用剪贴板已被强制禁用";(b) 应用 webview 媒体权限:代码未做任何媒体声明/授权闸门,保持"无用途声明";(c) 出站网络仍是 `Advisory`(静态应用),未升为 `Enforced`;(d) 真实恶意页面探针(跨 origin 跳转、伪造 nonce、自定义协议、合成事件)受窗口服务器/OS 权限限制,与 Task 08 人工项合并执行。

**测试(增量):**
- bytehost `bytehost-desktop`:`cargo test -p bytehost-desktop` → 37 + 8 + 3 + 8 pass(含 `build_app_child_pins_the_restrictive_settings`/`no_second_copy_of_the_navigation_policy`/`app_surface_ipc_maps_every_whitelisted_verb` 及 `with_clipboard(false)` 钉死)。
- Digger `digger-desktop`:desktop 单测 9 → 21(`apps::surface` +12:重开换代、迟到回包拒绝、待创建契约、换地址只留最新、无实例关闭丢弃地址、创建失败保留/清除等;`apps::surface::tests` 全绿)。
- Digger Spike `editor-host-spike`:37 pass(含新同源测试)。

**验证:**
- bytehost:`cargo build/test -p bytehost-desktop` 绿;`cargo fmt` + `clippy --all-targets -D warnings` 干净;`scripts/check-deps.sh` exit 0(`bytehost-webview` 在 desktop 组内已放行)。
- Digger:`cargo test --workspace` 全绿;`cargo fmt --all` + `clippy --workspace --all-targets -D warnings` 干净;`scripts/check-host-deps.sh` exit 0。
- Spike:`cargo test -p editor-host-spike` 绿(同源测试证明探针与真实 builder 一致)。

- **偏差:** (a) `AppSurfaces` 代数判据用 **per-app** 而非计划未预设的 per-slot —— 新实例开新槽、代数按应用递增,更简单且杜绝槽别名;(b) Digger 一期无应用视口,`surface_task` 的 `on_ipc`/`on_ready` 暂 no-op,真实回流留待视口接入(计划 §Task 16 第 5 条的真实运行与 Task 08 人工项合并,同 Tasks 14/15);(c) 3 处 dormant 接线(`surface_task`、`AppsState` 表面访问器)与 `surface.rs` 模块用 `#[allow(dead_code)]` + 注释记录"机制就绪、视口后续接入"——先行机制性是用户裁决的范围内产品。
- **未决项 / 阻塞:** **真实 WebView 交互验收(静态/py-notes/node-notes 真实可交互内容、同 origin 跳转与焦点、恶意页面跨 origin/伪造 nonce/合成事件探针、剪贴板实测能力)受 Task 08 同一 OS 权限/窗口服务器限制,与 Task 08 人工项合并待人工确认**。bytehost 当前经本地兄弟仓库 patch 解析;正式验收固定一个 bytehost tag(计划 §:13)。

- **提交(仓库 B + D):** `79e7d8d` bytehost 共享 builder;`57378a9` Digger 控制器 + 接线 + 探针。

---

## Task 17 — 浏览器存储、卸载清理与端口恢复(计划 §Task 17 `:236-246`)

- **目标（计划 §Task 17 `:236-246`）：** (1) 先销毁相关 app Surface，再执行共享 `StoreRemovals`；清理失败显示持续待重试状态。(2) 清理待办退出重启后可恢复，防止重装复活被删数据；服务数据与浏览器数据分别记录结果。(3) 两 app 验证存储隔离、同 app 重启连续性、保留/删除数据后重装差异。(4) 设置接端口冲突重试与显式重置：展示 origin 变化影响、停服务并撤 Surface、调 Task 12 API、恢复订阅。(5) 测失败恢复与重复操作幂等；不在 Digger 直接编辑 `gateway.json`。**验证:** A8/A12，端口冲突不损坏已有注册表和应用数据。
- **范围裁决（用户确认）：** 机制 + 测试；**推迟设置页实时交互 UI**。理由：Digger 一期无应用视口、无设置面板（`product.rs` 仅 "Digger"/"应用" 两面板），真实窗口线程驱动（`remove_data_store` 需主线程 + 事件循环）与视口同一约束，并入 Task 08 人工项（与 Tasks 14/15/16 同一先例）。
- **实现：**
  - **bytehost B — 共享层补一条清除入口**:`crates/bytehost-desktop/src/app_webview.rs` 新增 `host_os_major()`(objc2-foundation,与 Dozer 同法)与 `remove_app_data_store<M>`——**先过** `bytehost_webview::supports_store_removal(host_os_major())`(wry 不检查系统版本,macOS 12/13 直接调会崩),不支持则同步回 `Unsupported` 且不起 wry 任务;否则主线程 `window::run` 内调 `wry::WebView::remove_data_store`,完成回调(WebKit 事件循环异步触发)翻成 `Done`/`InUse`/`Failed` 经 `on_result` 回宿主。`Cargo.toml` 加 `objc2-foundation`。新增源码扫描门禁 `remove_app_data_store_gates_on_the_os_version`(钉住"先过版本门槛再调 wry"、不得另抄 `data_store_identifier`);bytehost-desktop 测试 37→38 通过。
  - **Digger D — 持久台账(app-host,无 wry/iced)**:新增 `crates/app-host/src/cleanups.rs::StoreCleanups`——落盘到 `<state>/store_cleanups.json` 的 `BTreeMap<app_id, CleanupState{Pending|Failed{reason}}>`;`request`(幂等)、`finish`、`fail`、`pending`、`failed`、`state`;原子落盘(临时文件 + rename);**损坏文件当空表**(读不出不能让启动失败)。5 个表格测试(重启恢复 / 幂等 / 完成落盘 / 失败可见 / 损坏容错)。
  - **Digger D — 宿主适配**:新增 `crates/desktop/src/apps/store.rs::StoreCleanup` 把三件拼起来——`StoreCleanups`(持久台账)+ `bytehost_webview::StoreRemovals`(内存退避重试)+ 确定性 `data_store_identifier`。`load` 时把上次 `Pending` **重新排进内存队列**(重启恢复);`request`(先经 `StoreRemovals::request` + 台账落盘);`settle`:`Done`→双清、`InUse`→`retry`(超上限记终态失败)、`Unsupported`/`Failed`→终态失败留台账;`take_ready`(仅离池者)/`next_wake`/`failures`。9 个表格测试(仅 ProgramAndData 清 / 离池才可清 / Done 清台账 / InUse 重试 / 反复占用终态失败 / Unsupported+Failed 终态可见 / 重启恢复重驱动 / 失败不重驱动 / 请求幂等)。
  - **Digger D — 组合根接线**(`apps/mod.rs`):`AppsState` 加 `store: StoreCleanup`(经 `with_ledger_path` 注入路径,生产走 `paths::state_dir()`)+ `pending_uninstall: HashMap<app_id, UninstallMode>`;`dispatch` 截获 `InstallEffect::Uninstall(id, mode)` 记模式(状态机回的 `ActionDone` 不带模式);`install_outcome(Uninstall, Ok)` 时按 `store::purges_browser_data(mode)` **先 `surfaces.close` 撤 Surface 再 `store.request`**;`Message::StoreRemovalResult` → `settle`;窗口线程入口 `store_removal_task(window, ready)`(一次性 futures 通道把 `on_result` 异步结果接回 `Message`)。3 个接线测试(ProgramAndData 排队 / Program 不碰 / 卸载失败不排队)。
  - **Digger D — 端口恢复(纯顺序契约)**:新增 `crates/app-host/src/port.rs::PortResetPlan`——**不编辑 `gateway.json`**,只转发 Task 12 的 `AppService::reset_port`/`clear_port`(同根 flock 保证"服务在跑就拒绝");`origin_before/after` 展示端口变化影响、`changes_port`、`PortResetStage` 记录进度。4 个测试(origin 变化 / 无旧端口 / 同端口非变更 / `apply` 转发到受控接口)。
  - `Executor`/`InstallAction` 加 `InstallAction::key()`(完成型动作 → 请求键的逆映射,供组合根/测试构造身份)。
- **失败恢复与幂等:** 重复 `request` 幂等(台账 contains 即短路);`InUse` 按共享退避重试至上限后转终态失败;`Unsupported`/`Failed` 不自动重试但**台账留存可见**(设置页提示"重装后可能仍看到旧数据");损坏台账容错不阻启动。
- **验证:** 三仓 `cargo fmt --all`、`cargo clippy --all-targets -D warnings`(digger 全 workspace)干净;digger `cargo test --workspace` 全绿(desktop 33、app-host 新增 8);bytehost `cargo test -p bytehost-desktop` 38 通过、`-p bytehost-webview` 13 通过;digger `scripts/check-host-deps.sh` exit 0、bytehost `scripts/check-deps.sh` exit 0。**A8/A12 的真人部分(两 app 真实存储隔离/同 app 重启连续性/重装差异、真实窗口线程驱动清除、设置页实时交互)并入 Task 08 人工项。**
- **偏差:** (a) 计划 §Task 17 `:238` 提"Digger app-host 管理操作及设置页",一期无设置面板,按用户裁决实现机制 + 测试、推迟 UI;(b) 端口重置顺序契约落在 app-host 的纯 `PortResetPlan`(可单测),真实异步"停→改→起→重订阅"由组合根在窗口/运行时驱动(与真实窗口同约束);(c) `remove_app_data_store` 的 wry 回调是 `FnOnce + Send + 'static` 且由事件循环异步触发,故经 `on_result` 回调 + 一次性 futures 通道回流,而非返回值。
- **未决项 / 阻塞:** **真实窗口线程清除驱动与设置页实时端口重置交互受 Task 08 同一 OS 权限/窗口服务器限制,与 Task 08 人工项合并待人工确认**。bytehost 当前经本地兄弟仓库 patch 解析(Task 17 新用 `bytehost-webview`,该 crate 在 tag `v0.1.1` 已存在,digger `Cargo.toml` 按 tag 引用 + 本地 patch 覆盖);正式验收固定一个 bytehost tag。

---

## Task 18 — 正常退出、崩溃恢复与双产品并行运行(计划 §Task 18 `:248-258`)

- **目标（计划 §Task 18 `:248-258`）：** (1) 独立根起应用，正常关窗后证明 gateway 端口关闭、子进程停止、desired 按库约定恢复。(2) 强杀测试专用 Digger 进程，重启同根查孤儿回收；测试无关进程绝不被回收。(3) 同根双启动拒绝；不同根 Dozer/Digger 并行不串端口/应用/浏览器数据。(4) shutdown 故障有可见结果；测超时与强退，不静默无限等待。(5) 无 Project 启动应用；Project 导出/移动有实现则真实验证，无实现必须在 Task 23 标未验收。**验证:** A14/A15/A16；所有 kill 目标由夹具创建并记录身份。
- **范围裁决:** 孤儿回收/双启动/端口释放/shutdown 报告的**机制层**在 Digger 集成测试覆盖;"强杀真实子进程 → 重启"与真实窗口的强退 UI 需要真实进程/窗口夹具,并入 Task 08 人工项(同 Tasks 14–17),**不以"丢弃 in-process 句柄"冒充强退**。
- **实现(仓库 D — `crates/app-host/tests/recovery.rs`,7 用例):**
  - **第 1 条**:`a_normal_close_releases_the_gateway_port_and_keeps_the_persisted_port` —— 真实 `InProcess::start` 起服务,轮询确认 gateway 监听;`host.await_shutdown()` 得 `Reported` 且 `report.is_clean()`、`gateway_released`;轮询确认端口**不再接受连接**;重启沿用同一持久化端口(不静默换,换了丢所有 origin 存储)。
  - **第 2 条**:`a_restart_after_a_kill_reaps_the_recorded_orphan_and_leaves_unrelated_processes_alone` + `reaping_a_stale_record_never_kills_a_pid_that_does_not_match_the_record` —— 造进程组组长(`pre_exec(setpgid)`)的遗留进程 + `run_dir/process.json`;`reap_orphan` 按**命令行 + 启动时刻 + 组长**三重复核,记录相符者 `Killed`,命令行不符者 `Stale`(只清记录,**不杀**被复用 pid 的无关进程)。无关进程 `try_wait()` 全程 `None`(绝不被误伤)。
  - **第 3 条**:`one_root_refuses_a_second_owner_while_different_roots_run_side_by_side` —— 同根第二个 `InProcess::start` 不可用(经真实 API 探测 `app_list()` 回 `Unavailable`),不改写端口文件,第一个服务端口仍可连;不同根并行、端口互不相同。
  - **第 4 条**:`a_shutdown_result_is_observable_and_never_hangs` —— 零超时 `shutdown_reporting_within` 要么干净要么 `TimedOut`,**一定返回**(不静默挂住)。
  - **第 5 条**:`the_app_service_starts_without_any_project_directory` —— 数据根由产品路径给出,不依赖任何 Project 目录。
  - **强退边界**:`force_quit_recovery_is_verified_at_the_process_level_not_by_dropping_a_handle` —— 显式记录"in-process 句柄丢弃 ≠ 进程被杀"(库内后台任务持有服务的 Arc,丢外层句柄释放不了),真实强杀留进程级夹具。
  - 测试加 `libc` dev-dependency(仅测试用 `setpgid` 造组长)。
- **bytehost 侧(既有,无需新增):** `service.rs::a_shutdown_that_overruns_the_timeout_is_reported_not_swallowed`(超时可见)、`a_clean_shutdown_reports_gateway_released_and_no_failures`、`requests_arriving_after_shutdown_do_not_start_or_install_anything`、`shutdown_cancels_a_running_download`;`manager.rs::reconcile_reaps_a_leftover_process_recorded_in_the_run_dir`(孤儿回收)、`a_crashing_app_ends_up_failed_and_not_retryable_and_its_site_is_gone`、`after_a_supervisor_restart_reconcile_restarts_apps_that_wanted_to_run`(desired 恢复)、`service_lock.rs` 跨进程 flock 用例。Task 18 的 bytehost 故障注入已由这些覆盖。
- **验证:** `cargo fmt --all`;`cargo clippy -p digger-app-host --all-targets -D warnings` 干净;`cargo test --workspace` 全绿(app-host 集成 7、lib 37;desktop 33)。
- **偏差:** (a) "强杀真实子进程"与"强退 UI"按裁决并入 Task 08 人工项;(b) 新增 `libc` 仅 dev-dependency(测试造进程组组长),不进生产依赖。
- **未决项 / 阻塞:** 真实进程级强杀夹具 + 真实窗口强退选择并入 Task 08 人工项。Project 导出/移动产品层无实现,Task 23 标未验收。

---

## Task 19 — 提取同源 Usage 面板库(计划 §Task 19 `:260-272`)

- **目标（计划 §Task 19 `:260-272`）：** (1) 先列明现有行为/资源契约,连同现有测试迁入库;不能在两处各留完整实现。(2) 把 workspace/chrome UI helper 依赖换成公共 UI 或注入的产品展示适配。(3) 审计本地 Git 统计路径:直接 `bytegit` Repo 读取改为面板所需的窄数据提供接口;Dozer 实现保持现状,Digger 明确其能提供的语义,不把 Git 实现塞进 Host。(4) 迁移离线 Web bundle、构建脚本、许可证与资源版本;运行原 npm typecheck/test/build,实际使用迁后 bundle。(5) Dozer 仅保留重新导出/注册包装;库依赖闭包无 dozer-app/dozer-client/dozer-core,允许领域 model/services 与公共 UI。(6) 以可固定 revision 提供 Digger 引用,验证依赖中的领域类型不出现两份不兼容来源。**验证:** Usage 旧测试、fake 独立面板真窗口、Dozer 真数据行为对照;A2。**提交按 Rust 提取、资源提取、Dozer 接入拆分。**
- **范围裁决(用户已定,4 项):** (a) 方式=先确认决策再分阶段实现(Rust 提取 → 资源提取 → Dozer 接入,三个提交);(b) crate 位置=**在 dozer 仓**(`dozer/crates/dozer-panel-usage`),Digger 在 `digger/Cargo.toml` 固定 git rev;(c) 展示适配=**注入适配 trait + 泛型 Message**(库定义窄 trait:agent 图标/颜色、面板头、折叠按钮,泛型于宿主 Message,Dozer 实现);(d) Git 数据=**注入窄 provider trait**(`GitStatsProvider{commit_count, commits_by_day}`,Dozer 实现包 `bytegit`,Digger 自备或 `NoGitStats`)。
- **阶段 1 — Rust 提取(DONE):** 新增 crate `dozer/crates/dozer-panel-usage`,内含 `aggregate.rs`/`protocol.rs`(纯数据/线协议,仅 `dozer-model`)、`state.rs`/`activation.rs`/`surface.rs`/`view.rs`/`adapter.rs`/`lib_tests.rs`。`extensions/usage/`(mod 1205 + aggregate 416 + protocol 481 + view 217 + surface 165 行)的数据层/视图层全部迁入。
  - **接缝(计划第 2/3/5 条):** `adapter.rs` 定义 `GitStatsProvider`(+`NoGitStats` 默认)、`UsageOutbox`/`UsageNotice`/`NoticeLevel`(替代 `toast::Outbox`;库**不**硬编码 `dozer_core::log::Scope`)、`ScriptDispatch = fn(&str)->String`(替代 `preview::dispatch_script`)、`PanelDisplay<M>`(agent_icon_color/agent_icon/panel_head/list_collapse_button/collapse_hover/toggle_list_collapse/**wrap**);`activation.rs` 提供库自有 `ActivationCtx<M>`。
  - **依赖闭包(第 5 条):** `cargo tree -p dozer-panel-usage` 仅 `dozer-model`、`dozer-services`、`byteui`(v0.4.1)、`bytehost-core`/`bytehost-desktop`(v0.1.1)+ iced/serde/tokio。**无** `dozer-app`/`dozer-client`/`dozer-core`/`bytegit`。
  - **验证:** `cargo test -p dozer-panel-usage` **52 passed**;`cargo clippy -p dozer-panel-usage --all-targets` 干净。行为契约(固定单槽 Surface 代数、payload 构造在面板内、宿主只决定推送时机等)逐条保留。
- **阶段 2 — 资源提取(DONE):** web 源码 `dozer-app/web/usage-content/` → `dozer-panel-usage/web/usage-content/`;产物 `dozer-app/assets/usage-content/` → `dozer-panel-usage/assets/usage-content/`(唯一来源,删旧副本)。`build.mjs` outdir 改指本 crate;新增 `NOTICE.md`(一手代码、`legalComments:none`、无 CDN、重建流程)与 `.gitignore`(node_modules)。
  - **消费方接入:** dozer-app `assets.rs::usage_content_root_for` 优先打包态 `Resources/usage-content`,dev 态回退 `dozer_panel_usage::web_assets_root()`;打包脚本 `build-macos-app.sh` 改从本 crate 拷;bundle/CSP 测试改指 `web_assets_root()`。
  - **验证:** `npm ci && npm run typecheck && npm test`(14 passed)`&& npm run build` —— 产物**确定性**,哈希(host.html `9cf50280`、usage-content.css `e5f67c70`、usage-content.js `59184938`)**与迁移前逐字节一致**;`cargo test -p dozer-app` 2023 passed(仅既有无关失败 `files::delete_confirm_spec_reflects_pending_target` + 一次并行 scratch 抖动);边界门禁 `check_panel_boundary.py` exit 0(仅既有 `R-HOST-EXECUTORS 0->1`)。
- **阶段 3 — Dozer 接入(DONE):** 旧 `extensions/usage/`(mod/aggregate/protocol/view/surface)删除,新增 `extensions/usage.rs`(纯 `pub use dozer_panel_usage::*;` + 重导出 host 适配函数)与 **host 侧** `host_adapters/usage_panel.rs`:`impl PanelDisplay<AppMessage> for App`(`wrap`→`Message::Usage`;`agent_icon_color`→`workspace::agent_dot_color`;`agent_icon`→`workspace::agent_icon` 经 `icons::view`;`panel_head` 按标题选图标 `"Agent"`→`Bot` 否则 `BarChart3` 交给 `home_panel_head_with_actions`;`list_collapse_button`→`App::list_collapse_button(PanelKind::Usage, HoverId::list_collapse(Usage), ..)`;`collapse_hover`→顶层 `Message::Hover`;`toggle_list_collapse`→顶层 `Message::TogglePanelListCollapse`)、`DoGitStats`(包 `bytegit::Repo::discover`→`commit_count`/`commit_count_by_day`,非仓库/错误一律 0/空)、`notices_to_pending`(`UsageNotice`→`toast::Pending`,scope=`module::"usage_panel"`)、`activate`(host `ActivationCtx`→库 `ActivationCtx`,注入 `DoGitStats`)。
  - **接缝位置裁决(用户已定):** 产品展示适配放 host 侧 `host_adapters/usage_panel.rs` 而非 `extensions/usage.rs`——面板边界门禁(`check_panel_boundary.py`)要求 `extensions/` 下不出现 `crate::app::App`,`PanelDisplay` 的 `App` 实现天然是产品/host 代码。`extensions/usage.rs` 因此为纯重导出(第 5 条)。
  - **接线:** `app/update.rs`(:223 outbox 转 Toast、`Message::Usage` 臂删去 `ToggleListCollapse`/`Hover` 死臂、:5318 activation 改 `usage::activate`)、`app/view.rs`(三处 `content_pane` + 一处 `list_pane` 新签名,不再 `.map(Message::Usage)`——库返回已是 `Element<AppMessage>`)、`app/app.rs`(`UsageSurface::new(USAGE_CONTENT_ID_OFFSET as u32, preview::dispatch_script)` 注入槽 id 与脚本包装)。`usage_probe`(host/fake_panel_host/scenarios)与 `panel_host.rs` 测试同步到新签名 + `CollapseHover`;`Duty::host_site()` 行号与漂移守卫改指 `host_adapters/usage_panel.rs:84/88` 等真实行。
  - **验证:** `cargo test -p dozer-panel-usage` 52 passed;`cargo test -p dozer-app` 1966 passed(仅既有无关失败 `files::delete_confirm_spec_reflects_pending_target`);`usage_probe` 27 passed(行号钉住 + 漂移守卫);clippy 干净;`check-log-scope.sh` ok;边界门禁 `check_panel_boundary.py` 无**新增**回退(仅既有 `app/app.rs: R-HOST-EXECUTORS 0->1`,系 A4b2 `run_app_host_effects`,与本次无关,已核实存在于 `HEAD`)。
- **偏差:** 阶段 2 顺带把 dozer-app 的 serve/packaging 指向新 crate(计划把"实际使用迁后 bundle"归阶段 2、把面板逻辑接入归阶段 3);Cargo.lock 只提交 `dozer-panel-usage` 相关 hunk,本地 `[patch]` 注入的 bytehost 重复项与既有 0.5.291 版本 bump 均**不**入库。阶段 3 未改 Cargo.lock(无新依赖)。
- **未决项 / 阻塞:** 阶段 3 代码已完成并提交;**fake 独立面板真窗口**验收与 **Dozer 真数据行为对照**仍待人工真窗口确认(与 Task 08/14/15/16 同一 OS 权限/窗口服务约束,并入 Task 08 人工项,用户已批)。Digger 侧引用(第 6 条,可固定 revision)与"依赖中领域类型不出现两份不兼容来源"待在 Task 20 处理。

---

## Task 20 — Digger 真实 Usage 数据闭环(计划 §Task 20 `:274-284`)

- **目标（计划 §Task 20 `:274-284`）：** 若账本/生产者已有接查询,否则补最小真实任务完成事件→幂等持久记录→区间查询链路(按 Digger runtime 架构归属,不写进 Project 内容模型或 Host);写清文件/事件来源/去重键/单位时区/错误恢复;实现 `UsageService` 与 Task 19 其他数据接口,不存在的能力明确呈现(不假数/不恒空);真任务结束生成可查记录、重启仍在、重复不翻倍、两 scope 不串、读失败与成功空区分;同一 Usage 库注册进 Digger 用真实服务展示,fake 仅在测试/演示。
- **范围裁决(用户已定,4 轮):** (a) 生产者范围=**新 digger-store 业务 crate + 真实执行器钩子**;(b) 共享库消费=**本地 patch 联调,Task 22 正式 pin**;(c) `UsageService` 位置=**业务 crate 实现,宿主适配层注入**;(d) 真实生产者身份=**一次经真实 Executor 派发的用户发起应用操作**(安装/启动/停止/卸载/回滚/运行时安装/卸载);记录**真实测量**墙上时钟;**token 如实 0**(无 agent 运行时,不伪造);只在真实用户操作触发。
- **业务层 — 新 crate `digger-store`(DONE):** `crates/store/src/{lib,ledger,service,runner}.rs`。
  - `UsageLedger`:落盘 `<config>/digger/usage_ledger.json`(形状 `{"runs":{run_id:{...}}}`)、原子写(临时+rename)、`run_id` 幂等 upsert;`load` 不存在→空、解析失败→`Corrupt`、I/O→`Io`。
  - `LedgerUsageService`:实现 `dozer_services::UsageService`;`cwd` 精确匹配 `project_path` 隔离 scope;`since_ts` 为 Unix ms,过滤 `finished_ts_ms >= since_ts`;锁污染/读失败→`Unavailable`(非空结果)。
  - `TaskRunner`/`RunningTasks`/`Clock`:在途运行登记(键 = ticket sequence),`finish` 以新时刻为结束时刻结算成 `CompletionEvent`(真实测量)。
  - **门禁:** `scripts/check-host-deps.sh` EXIT=0(`digger-store` 属业务前缀、**不依赖 `bytehost-*`**)。
- **宿主适配层 — `crates/desktop/src/adapters/{mod,usage}.rs`(DONE):** `UsagePanel` 持库 `WorkspaceState` + 注入 `Arc<dyn UsageService>` + scope 路径 + 可选运行时句柄 + 回流 `mpsc`;`activate()` 构造库 `ActivationCtx`(注入 `NoGitStats`,`emit` 投回通道);`impl PanelDisplay<Msg>`(agent 一律公共 `Bot` + 主题 `dim` 色,面板头 byteui 原语拼接);`receiver_stream` 供组合根接成 `Task::stream`。
- **组合根 — `product.rs`/`apps/mod.rs`(DONE):** 构造唯一共享 `Arc<Mutex<UsageLedger>>`(载入损坏→回落空账本 + `boot_note` 提示,不 panic);`LedgerUsageService` 注入 `UsagePanel`;`AppsState::attach_usage` 共享同账本同 scope;`dispatch` 用 `run_install_traced` 对用户动作 `begin_usage_for`、`deliver` 的 `Outcome::Install` 在 `accept(ticket)` 后 `settle_usage_for(ticket.sequence)`;注册 `digger.usage` 面板 + `ActivePanel::Usage` + `Msg::Usage`;`boot()` 挂运行时句柄与 usage 回流流。
- **验证:** `cargo test -p digger-store` **15 passed**;`cargo test -p digger-desktop` **41 passed**(含 5 个新集成测试:真实事件→真实服务可查 / 重启仍可查 / 重复不双记 / 两 scope 不串 / 读失败≠空);`cargo clippy -p digger-store -p digger-desktop --all-targets` 无警告;`cargo fmt --check` ok;`cargo build --workspace` 全绿;`check-host-deps.sh` EXIT=0。
- **偏差 / 发现:** dozer 远端尚无含 `dozer-panel-usage` 的 revision(`origin/main` = `ca0a279e`,`main` 领先 154 提交,`c3d57928` 不在远端),故 Digger 对 dozer 依赖走 **bare git + 本地 uncommitted `[patch]`** 联调;正式 pin 归 Task 22。`digger-store` 命中 cargo `patch not used` 警告(属预期:该 crate 不直接依赖 `dozer-panel-usage`)。
- **未决项 / 阻塞:** **A17** 由真实用户发起的应用操作驱动(非测试插入),但 token 如实为 0、代表应用操作而非 agent 会话——无真实 token 承载的 agent 运行时,**S6/S7 保持未完成**(计划 `:284`)。dozer 固定 revision(计划 Task 19 第 6 条)仍阻塞,归 Task 22。


## Task 21 — 迁移剩余通用 Host 路径并删除兼容桥(计划 §Task 21 `:286+`)

- **目标（计划 §Task 21 `:286+`）：** 按应用 WebView → Todo/GroupChat → editor/preview 承载 → 通用窗口事件顺序逐组切共享实现,每组独立提交回归;宿主不存产品专属字段/不解析业务协议/不 match 产品面板名;清理兼容桥台账(每项以无调用者+行为测试+单一实现证据关闭,不靠移动文件或放宽扫描目录降门禁);回归 Dozer 新旧布局/面板激活/产品命令/原生菜单/窗口退出。**验证:** 同口径结构度量、公共 Host 反向依赖为零、兼容桥零、两产品同用 Host crate。
- **实施进度:** (切片 1 应用面板 Effect 执行器外移 DONE);(切片 2 C5 预览窗格选择统一 DONE);(切片 3 C8 请求身份改挂 core `RequestToken`/`ScopedTrackers`,含设计 move 1+2 DONE);其余切片(C7/C10/C11/C12)待续。


### 切片 1 — 应用面板 Effect 执行器外移出 `app/`(DONE)

- **动机 / 桥:** **C6**(`R-HOST-EXECUTORS` 实测 1,**回退项**):`app/app.rs` 的 `fn run_app_host_effects` 是 host 替应用面板执行副作用,规格 §6 验收 2 要求 host 只给 `PanelIo`。
- **改动:** 新 `crates/dozer-app/src/host_adapters/app_host.rs`(产品侧适配层,非 `app/` 也非 `extensions/`——避开 `R-HOST-EXECUTORS` 与 `R-APP`/`R-WS` 棘轮),承载应用面板 [`Effect`](`bytehost_panel::host::PanelEffect`) 的**产品执行器**:顶层 `apply_effects(app, effects)` + 两个入口 `update(app, msg)`/`poll_if_due(app)`。`app/app.rs` 的 `poll_app_host_if_due`/`app_host_update` 改为一行转发,`run_app_host_effects` 整体删除(净 -114 行)。语义**逐字一致**(同批 Effect、同 `proxy` 回传形状、同 Toast 键去重、同"启动地址是秘密不写日志")。
- **验证:**
  - `python3 scripts/audit/check_panel_boundary.py` → **ok(120 refs in 5 files)**,`R-HOST-EXECUTORS` 回退**消失**(0),基线未改(棘轮回落到基线,非上调阈值)。
  - `cargo build -p dozer-app` 绿(仅既有 warning)。
  - `cargo clippy -p dozer-app --bin dozer` 对新文件零告警(其余 warning 为既有:workspace/state.rs、preview 等)。
  - `cargo test -p dozer-app --bin dozer` **1970 passed**;唯一失败 `usage_probe::host::tests::host_site_citations_point_at_the_code_they_name` 经核实由**用户 WIP(`app/update.rs` 行号位移)**引起,非本切片——把该 WIP 暂存后该测试即通过,已复原 WIP。
- **偏差 / 发现:** 未动用户 WIP(`app/update.rs`、`extensions/files/**`、`platform/*`、`dozerd/*`、`Info.plist`、`Cargo.lock`)。
- **未决项 / 阻塞:** C6 计数已归零;但"宿主不 match 产品面板名/不存产品专属字段"的更彻底口径(A4b2 剩余)与其余桥(C5/C7/C8/C10/C11/C12)待后续切片。


### 切片 2 — 预览窗格选择统一(C5,DONE)

- **动机 / 桥:** **C5**(`R-PANE-PICK`):`workspace/state.rs`(2)、`app/update.rs`(1)、`platform/window_events.rs`(3) 手写"按 `PanelKind::Project` 选预览窗格/判可见"的写法未走 `Workspace::preview_pane[_mut]`。
- **改动:**
  - 新 `App::panel_in_either_view(kind) -> bool`(`app/app.rs`):统一"某面板是否在左右任一栏"判断,取代散落的 `matches!(left_view, ..) || right_view == PanelKind::Project`。
  - `platform/window_events.rs` 三处(Find 面板可见性、项目名称/描述编辑框焦点)改用该访问器。
  - `app/update.rs` 的 `PreviewLargeFileSearchGo` 改为 `ws.preview_pane_mut(kind)` + 末尾 `ws.preview_error_mut(kind)`,把原来并存借用两字段改为**顺序借用**(pane 借用在取错误槽前结束),不再手写 match。
- **验证:**
  - `python3 scripts/audit/check_panel_boundary.py` → **ok(116 refs in 4 files)**;`R-PANE-PICK` 由 6 处降至 **2**(仅剩 `preview_pane`/`preview_pane_mut` 两个访问器本体,即单一实现);调用点文件条目从基线移除,基线**只降不升**。
  - `cargo build -p dozer-app` 绿;`cargo clippy -p dozer-app --bin dozer` 对本切片改动零告警。
  - `cargo test -p dozer-app --bin dozer` **1970 passed**;唯一失败仍是既有 WIP 引起的 `usage_probe::host` 引用行号漂移(与切片 1 同因,非本切片)。
  - **基线附带修正:** `R-HOST-PANEL-ARMS` 95→93——以 `git show HEAD:...update.rs` 复核,HEAD 真实为 93(95 为 Task 01 遗留的陈旧值,已由后续已提交工作降到 93,非本次改动亦非 WIP),顺带收紧棘轮。
- **偏差 / 发现:** C5 验收文案"三处计数归零"按字面无法达成(2 处在访问器本体,是**目标**单一实现而非违规);本切片把**调用点**归零,`state.rs` 保留 2 为唯一实现,已在台账注明口径。
- **未决项 / 阻塞:** 无新增;其余桥(C7/C8/C10/C11/C12)待后续切片。


### 切片 3 — 面板请求身份改挂 core `RequestToken`/`ScopedTrackers`(C8,含设计 move 1+2,DONE)

- **动机 / 桥:** **C8**(`acceptance.md:46`:「使用 core 请求身份且 delete 该文件后编译通过」):产品 `crates/dozer-app/src/request_tracker.rs` 自持 `RequestToken` 包装,唯一目的就是给 core token 外挂 `project_id: Option<i64>`。用途二:①**路由键**——`App::update` 的 `msg.project_id()`(`app/update.rs:1951`)选 `with_project` 目标;②**守卫**——`browser.rs:1691`/`todo/update.rs:127`/`project/update.rs:25` 三处 `token.project_id != Some(project_id)`。
- **前置(bytehost,已提交 `f50ad77`):** `bytehost-core` 新增中立的 [`ScopedTrackers`](`crates/bytehost-core/src/request.rs`):`new(owner, instance, generation)` / `begin(&ScopeId, RequestKey)` / `scope(&ScopeId)` / `invalidate(RequestKey)`(广播全部已知 scope)/ `accepts(&RequestToken)`(按 `token.scope()` 判)。它把产品"每 scope 一张在途表"的行为一般化,使包装类型可删。含 4 单测 + 1 集成测试(`scoped_trackers_route_dual_lane_panel_by_scope`),`cargo test -p bytehost-core request` 15+7 passed,clippy 干净。
- **改动(dozer,`300c7dab`):**
  - **删除 `crates/dozer-app/src/request_tracker.rs`**(整文件,含其 3 个单测),`main.rs` 去掉 `mod request_tracker;`。
  - **`panel_host.rs` 成为产品唯一收口:**
    - `scope_of(Option<i64>) -> ScopeId`(业务 → 中性:`Some(id)→"project:{id}"`、`None→"global"`)与 `project_of(&ScopeId) -> Option<i64>`(逆)。**业务含义只在产品这一处**,Host 不见 `project:5` 的业务意义。
    - `RequestTrackers` 产品 newtype(内包 `ScopedTrackers`),`Default` 每次经 `next_request_owner()` 取新 owner(保留旧 `NEXT` 语义:重开的同 slot 面板互不接受在途结果),转发 `begin(Option<i64>, &str)`/`scope`/`invalidate`/`accepts`——**签名保持产品友好**(吃 `project_id`/lane 名),故 ~46 处调用点**零改动**。
    - `wrap()` 从产品 `RequestToken` 移到 `panel_host::wrap_token` + 扩展 trait `RequestTokenWrapExt`,面板 `.begin(..).wrap(emit, Message::AsyncResult)` 链式写法不变。
  - 7 个面板(`project`/`agent_context`/`browser`/`todo/state`/`group_chat/mod`/`conversations`/`codehealth`)的 `AsyncResult` 载荷、`State.requests` 字段、spawn 形参改 `bytehost_core::request::RequestToken` / `crate::panel_host::RequestTrackers`。
  - 守卫改比 scope:`token.scope() != &scope_of(Some(project_id))`;`result_project()`/`project_id()` getter 改 `project_of(token.scope())`(group_chat 仍 `.expect("group project")`)。
  - 一处浏览器测试断言从 `global.project_id == None` 改 `project_of(global.scope()) == None`。
- **验证:**
  - `cargo build -p dozer-app` 绿;`cargo build -p dozer-app --bin dozer --tests` 绿。
  - `cargo clippy -p dozer-app --bin dozer --all-targets`:16 warning,**全为既有**(git_accounts `ALL`、project `move_item`、homespace doc list 等),本切片文件(含 `panel_host.rs`)零新增。
  - `cargo test -p dozer-app --bin dozer` **1967 passed / 1 failed**(测试数由 1970 降 3 即删掉的 `request_tracker` 3 个单测);唯一失败仍是**既有用户 WIP(`app/update.rs` 行号位移)**引起的 `usage_probe::host::tests::host_site_citations_point_at_the_code_they_name`(`app/update.rs:5318`),本切片未触碰该文件,非本切片。
  - `python3 scripts/audit/check_panel_boundary.py` → **ok(116 refs in 4 files)**,基线未改。
  - C8 验收证据:git 历史可见 `D crates/dozer-app/src/request_tracker.rs`(整文件删除,非移动),删除后编译通过;全仓 `grep request_tracker` 只剩函数名 `new_request_trackers`(非模块引用)。
- **偏差 / 发现:** 设计 move 2「`App` 维护 `PanelInstanceId → 区域` 路由表」中,**C8 所需的部分未新建表**——因保留了 `RequestTrackers` 的产品友好签名与 `project_of` 反解,`msg.project_id()` 路由(`app/update.rs:1951`)语义不变即可工作。完整的「`PanelInstanceId → 区域/焦点`注册表」留到 C11/C12 落地时再建,避免此处先造一套键控(与设计"一次设计、分步入地"一致)。
- **未决项 / 阻塞:** 产品面板仍传 `PanelInstanceId::new(0,0)`(惰性 scope 表不依赖它);把真实 `PanelInstanceId` 线程进 `PanelIo` 是 C11 的工作。C7/C10/C11/C12 待续。


### 切片 4 — 焦点路由抽通用 `FocusRouter`(C12,设计 move 3,DONE)

- **动机 / 桥:** **C12**(`acceptance.md:50`):`platform/window_events.rs` 的焦点路由(`FocusIntent`/`pending_focus`/`current_focus`/`apply_pending_focus`)全以产品 `PanelKind` 为键,**通用"焦点去向"机制未抽**(纯 `FocusTracker` 已于 Task 10 抽取,但与焦点去向无关)。验收要求:通用焦点目标类型进 bytehost-desktop,产品只留业务路由(SearchIn/ShowFileHistory 等),门禁计数只降。
- **改动(bytehost,`c80d443`):** `bytehost-desktop/src/focus.rs` 新增产品中立的 `FocusRouter<K>` + `FocusTarget<K>{ Native(K), Window }`:
  - `FocusRouter` 内部即原产品 `pending_focus`/`current_focus` 两字段(泛型键 `K`),提供 `set`(排待处理+设当前)、`set_current`(只设当前)、`current`/`current_native`、`take_pending`、`apply(resolve_native, focus_native, focus_window)`(消费待处理:解 `K→u64` 原生句柄,命中则聚焦,否则回落窗口)。`FocusTarget::Window` 为 `Default`。宿主对 `K` 完全不透明、零业务依赖。含 6 个单测(原 `FocusTracker` 2 + 新 `FocusRouter` 4)。
- **改动(dozer,`fa9f37a2`):**
  - `window_events.rs` 的 `Runner::Ready` 两字段 `pending_focus: Option<FocusIntent>` + `current_focus: FocusIntent` **合并为 `focus: bytehost_desktop::FocusRouter<FocusIntent>`**(产品键仍是 `FocusIntent`;宿主管机制)。
  - 新增 `FocusIntent::target() -> FocusTarget<FocusIntent>`(产品把业务键喂给 host 的唯一转换点:`Terminal → Window`,其余 `→ Native(intent)`)。所有 `*pending_focus = Some(x); *current_focus = x;` 调用点改为 `focus.set(x.target())`(含鼠标点击、`dispatch` 各分支、Search Pick)。
  - `apply_pending_focus` 改为:`resolve` 闭包把 `FocusIntent` 解成 webview id(`active_preview_webview_id`/`active_browser_webview_id`),`focus_native` 在预览/浏览器两池里查并聚焦,`focus_window` 兜底——行为与迁移前逐字等价(解析不到/池中缺失均回落窗口)。
  - 按键闸门 `if let FocusIntent::Preview(kind) = *current_focus` 改 `focus.current_native()`;测试初始化改 `FocusRouter` + `set_current`。
  - 新增门禁规则 **`R-FOCUS-PRODUCT-KEYS`**:`platform/window_events.rs` 里业务键 `FocusIntent` 的出现次数(C12 后宿主机机制已不在产品,残留产品键是**允许的业务路由**),基线建为 25,只降不升。
- **验证:**
  - bytehost:`cargo test -p bytehost-desktop focus` 6 passed;`cargo clippy -p bytehost-desktop` 零告警;`bash scripts/check-deps.sh` **ok**(desktop 组仍无产品依赖)。
  - dozer:`cargo build -p dozer-app` 绿;`cargo clippy -p dozer-app --bin dozer --all-targets` 对 `window_events.rs` 零告警;`cargo test -p dozer-app --bin dozer` **1967 passed / 1 failed**(唯一失败仍是既有用户 WIP 引起的 `usage_probe::host` 引用行号漂移)。
  - `python3 scripts/audit/check_panel_boundary.py` → **ok(141 refs in 5 files)**;新增 `R-FOCUS-PRODUCT-KEYS 25`,其余基线值未动。
- **偏差 / 发现:** `window_events.rs` 同时含用户 WIP(命令修饰键 `command`/`TreeDragRelease { shift, command }` 那一处 hunk);本切片用 `git add -p` 只暂存 C12 的 13 个 hunk,未触碰该 WIP。
- **未决项 / 阻塞:** `FocusIntent`/`PanelKind` 仍是产品的焦点业务键(符合 C12 "产品只留业务路由"口径);把它们进一步中性化不在本桥范围。C7/C10/C11 待续。



### 切片 5 — rail/pane 容器改挂 `PanelId` 键控宿主容器(C11,设计 move 4,DONE)

- **动机 / 桥:** **C11**(`acceptance.md:49`):`chrome/rail.rs` 的 `RailLayout{left/right:Vec<PanelKind>}` 与 `app/layout.rs` 的 `PanelDims`(逐面板具名 `_split`/`_list_collapsed` 字段 + 巨型 `match`)与产品 `PanelKind`/`App`/`ShellState` 深度耦合;bytehost-desktop 侧**无 `PanelId` 键控的 rail/pane/drag API**。验收要求:设计出 `PanelId` 键控的 rail/pane 容器后 Dozer 适配接上,且像素布局回归通过(A4/real-window)。
- **改动(bytehost,`3dad53e`):** 新增 `bytehost-desktop/src/panes.rs`,产品中立:
  - `PanelId` 是唯一键(来自 `bytehost_core::id::PanelId`);host 完全不认 `PanelKind`。
  - `PaneDims{split, collapsed}`、`PaneSpec{default_first_first, collapsible}`、`PaneStore`(`BTreeMap<PanelId, PaneDims>`,`#[serde(transparent)]`,提供 `new/get/get_or/ensure/set/set_split/set_collapsed/len/is_empty`)。
  - `RailItems = Vec<PanelId>`(rail 顺序)。
  - `apply_pane_split_drag(current: PaneDims, spec: PaneSpec, mirrored, input: PaneDragInput) -> PaneDragOutcome`——把 Dozer `apply_pair_split_drag` 的几何(夹取 min/max 比例、`default_first_first != mirrored` 的镜像换算、拖窄到 `min_first_px` 即收起并冻结 split)逐字外移。`PaneDragInput` 用具名字段承载 6 个几何参数(仓库"参数结构体"约定,避免多个 `f32` 位置参数相邻易传错)。
  - 含 8 个单测(拖拽镜像/夹取/收起/展开/不可收起面板永不收起/`PaneStore` 读写)。
- **改动(dozer,`b50b5ca0`):**
  - `PanelKind::stable_id() -> PanelId`(`app/state.rs`):产品业务枚举 → 中立身份的唯一映射点,命名空间 `dozer.`+小写名(`dozer.files`/`dozer.gitlog`…),应用面板走 `app:<id>`。新增 `PanelKind::ALL_BUILTIN` 常量。
  - `panel_registry`:`PanelCatalog::panel_id(kind)` + 自由函数 `panel_id(kind)`(经当前清单),`PanelKind → PanelId` 经描述符/`stable_id`。
  - `PanelDims::to_pane_store(self) -> PaneStore` / `apply_pane_store(&mut self, &PaneStore)`:命名字段(落盘真相)与宿主 `PanelId` 键控容器的**互逆投影**。
  - `apply_pair_split_drag` 的算术**改为调 `bytehost_desktop::apply_pane_split_drag`**——Dozer 只提供 `pane_spec(kind)`(是否可收起 + 默认哪个 slot 在前)与当前 `PaneDims`,宿主算新状态;结果经 `to_pane_store → set → apply_pane_store` 回写。原 `list_rendered_first`(镜像换算)删除(逻辑已在宿主)。
  - `RailLayout::side_of(kind)` 改为 `side_of_id(kind.stable_id())`;新增 `side_of_id(PanelId)` 与 `to_rail_items(Side) -> RailItems`(rail 投影成 `PanelId` 键控条目)。
  - 新增门禁规则 **`R-PANE-KEYED-FIELDS`**:`app/layout.rs` 里逐面板命名的 pane 维度字段出现次数(残留的**产品侧落盘兼容面**),基线建为 81,只降不升——新维度应进宿主 `PanelId` 容器,不再新增 named 字段。
- **验证:**
  - bytehost:`cargo test -p bytehost-desktop panes` **8 passed**;`cargo test -p bytehost-desktop focus` 6 passed(未回归);`bash scripts/check-deps.sh` **ok**。
  - dozer:`cargo build -p dozer-app` + `--tests` 绿;`cargo clippy -p dozer-app --all-targets` 对我的四文件零告警(仅剩既有 `GitProvider::ALL`/`move_item` dead-code);`cargo test -p dozer-app --bin dozer` **1970 passed / 1 failed**(唯一失败仍是既有用户 WIP 引起的 `usage_probe::host` 引用行号漂移;新增 3 个 C11 单测:`panel_dims_accessor_tests::pane_store_round_trips_every_builtin_panel`/`panel_id_is_stable_dozer_namespace`、`chrome::rail::tests::side_of_id_matches_kind_side`);`bash scripts/check-log-scope.sh` **ok**。
  - `python3 scripts/audit/check_panel_boundary.py` → **ok(222 refs in 6 files)**;新增 `app/layout.rs R-PANE-KEYED-FIELDS 81`,其余基线值未动(141→222 仅新增本规则)。
- **偏差 / 发现:**
  - **命名字段未做 store 整体重写。** `PanelDims` 的逐面板 named `_split`/`_list_collapsed` 字段保留为**落盘序列化面**(`panel_layouts.json` 形状不变),宿主容器是键控适配视图;全字段→store 重写会波及约 660 处引用(含 `view.rs`/`webview_geometry.rs`/`app.rs`),在 A4 像素回归受 Task 08 权限阻塞时风险过高。故采"宿主容器 + 投影"的渐进接法,`R-PANE-KEYED-FIELDS` 棘轮锁住残留面只降不升。
  - `Divider` 的 12 个面板/配对变体、`Side`/`MaximizedPane` 未移入 host(设计 move 4 未决项 ②③:`RightPairSplit` 无通用等价,留产品)。
  - 本片四文件不含用户 WIP(已校验 staged diff 无 `TreeDragRelease`/`command` token);`layout.rs`/`state.rs`/`chrome/rail.rs`/`panel_registry.rs` 全属本片。
- **未决项 / 阻塞:**
  - **A4 真窗像素回归**仍受 Task 08(OS 权限)阻塞,本片只以既有几何单测 + 像素无关的算术等价(逐字外移)担保;真窗回归待 Task 08 解除后补。
  - `PanelInstanceId` 线程进 `PanelIo`(切片 3 遗留)未在本片完成;`PanelId` 键控容器已是其前置。
  - `Divider`/`Side`/`maximized` 中性化(设计 move 4 未决 ②③)未做。**C7/C10 待续。**



### 切片 6 — 面板内槽位词汇移入宿主(C9,DONE)

- **动机 / 桥:** **C9**(`acceptance.md:47`):`HoverId`(`app/state.rs`)与 `HoverSlot`(`panel_host.rs`)的词汇尚未命名空间化/注册制。验收要求:`HoverSlot`/`PanelKind` 变体表纳入注册制后**门禁计数只降**。
- **核对:** `HoverSlot` 本就是产品中立词汇(9 个通用槽位 + `Named` 兜底,注释即写"词汇通用,不含任何面板名"),与具体面板无关——是天然的宿主类型;`PanelKind` 的注册制已由 `panel_registry::PanelCatalog` 落地(Task 04+)。产品专属部分(`HoverId` 的 `Topbar`/`ProjectTab*`/`HomeTab`/`TermTab*` 等)必须留产品侧(它们绑定产品 UI 结构),`HoverId::Panel(PanelKind, HoverSlot)` 是"产品身份 + 宿主槽位"的组合点。
- **改动(bytehost,`17c0736`):** 新增 `bytehost-desktop/src/hover.rs`,产品中立的 `HoverSlot`(9 通用变体 + `Named(&'static str)`);`lib.rs` 注册 `pub mod hover;` + `pub use hover::HoverSlot;`。
- **改动(dozer,`5669776c`):** `panel_host.rs` 删除本地 `HoverSlot` 定义,改为 `pub use bytehost_desktop::HoverSlot;`(调用点零改动,`HoverId::Panel`/`HoverId::named(..)` 自动指向宿主类型);新增测试 `hover_slot_vocabulary_is_the_host_type` 钉住"槽位词汇即宿主类型"。
- **门禁:** 退役 `R-HOVERSLOT-VARIANTS` 规则(主体已移入宿主,`panel_host.rs` 里该枚举为 0),`ENUM_RULES` 只留 `HoverId`;基线按棘轮下降删除 `panel_host.rs` 条目(9→无),其余值未动(`222 refs/6 files` → `213 refs/5 files`,仅少去该条目)。
- **验证:** bytehost `cargo test -p bytehost-desktop` 全绿(50+8+3+8)、`cargo clippy` 零告警、`check-deps.sh` ok;dozer `cargo build`/`--tests` 绿、`cargo clippy -p dozer-app --all-targets` 对本片文件零告警、`cargo test -p dozer-app --bin dozer` **1970 passed / 1 failed**(唯一失败仍是既有用户 WIP 行号漂移)、`panel_host` 22 passed(含新测试)、`check-log-scope.sh` ok、`check_panel_boundary.py` ok(213/5)。
- **偏差 / 发现:** 本片是纯"搬类型 + 重导出",零调用点改动——`HoverSlot` 本就是中立词汇,只是错放在产品 crate。
- **未决项 / 阻塞:** 无。C9 收口。**C7/C10 待续。**



### 切片 7 — webview 几何函数命名中立化(C7,DONE)

- **动机 / 桥:** **C7**(`acceptance.md:45`):"几何与文件预览业务命名混同"——要求几何函数用宿主通用名、无 "preview" 命名残留。
- **核对 / 口径:** 依 Task 08 既有裁决(ledger `:286`),`webview_geometry.rs` 的几何计算**留在产品侧**(它吃 `ShellState`,搬进宿主会动像素行为、收益低,bytehost 只提供 shell 级通用 `Placement`)。故 C7 的可执行部分是**纯改名**——把"文件预览"业务词换成中立的"内容列/列表列"口径。
- **改动(`0f9ecc44`):** 三处公开函数改名,调用点/文档/测试名同步:
  - `preview_content_bounds_for` → `content_pane_bounds_for`
  - `is_in_preview_column` → `is_in_content_column`
  - `left_files_tree_bounds_for` → `left_list_pane_bounds_for`
  连带 `pair_content_pane_bounds_for` 等派生名与约 30 处测试名。触及 11 个文件、111/111 行对称改(零行为变化)。产品 API 名 `App::preview_desired`/`ws.preview` 等**保留**(它们是真实的预览业务入口,不是几何命名)。
- **门禁(`6ecadf22`):** 新增 `R-PREVIEW-NAMING`——扫 `webview_geometry.rs` 里 `preview_content_bounds*`/`is_in_preview_column`/`left_files_tree_bounds_for` 三个旧名残留,目标恒为 0(不进基线);已单测证明"旧名重现即命中、新名不命中"。
- **验证:** dozer `cargo build`/`--tests` 绿、`cargo clippy -p dozer-app --all-targets` 对 `webview_geometry.rs` 零告警、`cargo test -p dozer-app --bin dozer` **1971 passed / 1 failed**(唯一失败仍是既有用户 WIP 行号漂移)、`check-log-scope.sh` ok、`check_panel_boundary.py` ok(213/5,新规则贡献 0)。
- **偏差 / 发现:** C7 的真正"移入宿主"被 Task 08 明确否决,故本片只关"命名中立"这半边;若日后要把 `ShellState` 相关几何上收宿主,需先设计 shell 级几何接缝(C10 同域)。
- **未决项 / 阻塞:** 无。C7 收口。**C10 待续。**



### 切片 8 — C10 原生浮层建窗机制上收:设计成稿(方案已裁决,见切片 9)

- **动机 / 桥:** **C10**(`acceptance.md:48`):原生浮层建窗/渲染机制(`overlay_window.rs`/`overlay_gpu.rs`/`toast_overlay.rs`)上收 `bytehost-desktop`,届时 `R-OVERLAY-DUP`(新门禁)归零。
- **障碍(核心约束):** `bytehost-desktop` 刻意不绑渲染器 feature(iced 只 `default-features=false` + `tokio`,R2 修复 `fbc3a636`)且**不依赖 winit**。整体搬迁必然给 desktop 加 `iced_wgpu`/`iced_winit`/`winit`,改变其依赖性质。
- **成稿:** `docs/superpowers/specs/2026-10-12-c10-overlay-windowing-seam-design.md`(`1b1f858f`)。三文件拆分依据 + 方案 A(feature-gated 整搬)/B(只搬无渲染器依赖的一半)/C(B 起步再续 A);**用户裁决采方案 B**。
- **未决项:** 是否要求 `R-OVERLAY-DUP` 完全归零(续做 A/C)见切片 9 记录。

### 切片 9 — C10 原生浮层建窗机制上收:方案 B 落地(DONE)

- **裁决:** 用户选 **方案 B**——只搬"不碰渲染器后端"的建窗/定位/鼠标穿透/圆角遮罩几何;`OverlayGpu`(wgpu 管线)与 `backdrop_card*`(依赖 `iced_widget`)留产品侧;`R-OVERLAY-DUP` 基线为剩余数(不追求 0)。
- **改动(bytehost,`147285f`):** `bytehost-desktop/Cargo.toml` 加 `winit`/`raw-window-handle`/`objc2-app-kit`(**不加**渲染器 feature);新增 `src/overlay.rs`——`full_window_overlay_bounds`/`centered_card_offset`/`window_logical_size`/`popup_card_size`/`POPUP_WIDTH_FRACTION`/`BACKDROP_CORNER_RADIUS`/`open_child_window`/`open_child_window_unfocused`/`enable_overlay_mouse_moved_events`/`open_overlay<G>`(GPU 经泛型工厂注入)/`reposition_overlay`(surface 重配经闭包注入)/`stack_bounds`;7 个单测(3 几何 + 4 Toast 定位)。`lib.rs` 注册 + 重导出。
- **改动(dozer,`ea1202a0`):** `platform/overlay_window.rs` 缩为薄壳(重导出宿主建窗/几何,保留 `backdrop_card*`,`open_overlay`/`reposition_overlay` 签名不变、内部注入 `OverlayGpu` 工厂);`platform/toast_overlay.rs` 的 `stack_bounds` 改调宿主(只绑 Dozer 像素常量)。新增 `R-OVERLAY-DUP` 门禁,基线 `platform/overlay_gpu.rs R-OVERLAY-DUP 3`(`create_surface` + 2× `CompositeAlphaMode::PostMultiplied`)。
- **门禁:** `check_panel_boundary.py` 新增 `R-OVERLAY-DUP`(扫 `platform/*overlay*.rs` 的建窗/透明/穿透/alpha 特征串,只降不升);基线由 213/5 → 216/6(仅新增该条)。
- **验证:** bytehost `cargo test -p bytehost-desktop` 57+8+3+8 全绿、`cargo clippy` 零告警、`check-deps.sh` ok;dozer build/`--tests` 绿、`cargo clippy -p dozer-app --all-targets` 对两文件零告警、`cargo test -p dozer-app --bin dozer` **1968 passed / 1 failed**(少 3 = 移入宿主的几何测试;唯一失败仍是既有用户 WIP 漂移)、`check_panel_boundary.py` ok(216/6)、`check-log-scope.sh` ok。
- **偏差 / 发现:** 方案 B 达不到"`R-OVERLAY-DUP` 归零"——`OverlayGpu` 的 wgpu surface 配置有意留产品侧;要归零须续做方案 A/C(给 desktop 加 feature-gated 渲染器依赖 + 把 `OverlayGpu` 也上收)。
- **未决项 / 阻塞:** C10 的"完全归零"续做待裁决(A/C)。**至此 Task 21 全部已知桥(C5/C6/C7/C8/C9/C10/C11/C12)收口;后续进 Task 22(依赖钉版 + 独立构建/发布打包)。**



### 设计:C8 + C11 + C12 共同的"PanelId/Scope 键控"缺口(DONE — 设计待裁决)

**背景。** 三个剩余桥(C8 请求身份路由、C11 rail/pane 容器、C12 焦点路由)共享同一根因:**Host 已有稳定中性身份(`bytehost_core::id::{PanelId,ScopeId,PanelInstanceId}`、`Layout/Rail{items:Vec<PanelId>}`、`PanelCatalog`),但产品从未把身份线程到面板运行时**。因此每处都退化成以产品 `PanelKind`/`project_id` 为键。核对结论:

- `panel_host.rs` **完全不含** `PanelInstanceId`/`ScopeId`;产品 `RequestTracker` 内部只固定 `PanelInstanceId::new(0,0)` 靠惰性 scope 表工作。面板运行时拿不到 host 身份。
- C8:`request_tracker.rs` 的 `RequestToken` 包装只为携带 `project_id: Option<i64>`。用途二:①**路由键**——`App::update` 里 `msg.project_id()`(`app/update.rs:1951`)决定 `with_project(project_id, …)` 把回包投给哪个工作区;②**守卫**——3 处 `token.project_id != Some(project_id)`(`browser.rs:1691`/`todo/update.rs:127`/`project/update.rs:25`)。其余 4 面板(codehealth/agent_context/conversations/group_chat)只用 `requests.accepts(&token)`,不读 project_id。`CoreToken` 有 `scope(): &ScopeId`(值形如 `project:5`/`global`),无 `project_id`。
- C11:`PanelKind` 是 13 变体闭枚举(仅 `App(AppSlot)` 带数据),几何/折叠是 `PanelDims` 的**逐面板具名字段 + 巨型 match**(`app/layout.rs:161/179/199/214/631`),`Divider` 12 个面板/配对变体 + 1 个 `LeftRight`(`app/layout.rs:343`)。`RailLayout{left/right:Vec<PanelKind>}` 与 `bytehost_core::Layout.rails:Vec<Rail{items:Vec<PanelId>}>` 近似 1:1,但 host 侧**无 iced rail/pane/divider 控件**、`Rail` 也只有 `width` 无每面板 split/collapsed。`chrome/rail.rs` 自述仅"物理搬迁",状态留在 `App`/`ShellLayout`。
- C12:`FocusIntent{Preview(PanelKind),Browser,Terminal}`(`window_events.rs:273`)全以 `PanelKind` 为键;`apply_pending_focus`(`:2418`)按 `Preview(kind)` 查 `active_preview_webview_id(kind)`;纯 `FocusTracker` 已抽(host),但"焦点去向"未抽。

**提议的最小公共 API(待用户裁决,方向以既能关 C8 又能喂 C11/C12 为准):**

1. **身份线程(基础)。** `PanelIo` 在绑定时携带其 `PanelInstanceId` + `ScopeId`(由产品组合根按 `project_id→scope` 映射给出,映射仍只活在产品侧);`CoreToken` 已自带 scope,故**无需包装**。`Message::AsyncResult` 载荷由 `crate::request_tracker::RequestToken` 改为 `bytehost_core::request::RequestToken`(直接携带 scope+instance)。→ **C8 收口**:删除 `request_tracker.rs`;面板守卫改为 `token.scope() != expected_scope`(expected_scope 由面板在 `on_activate` 拿到并存下),`accepts` 直接吃 `CoreToken`。
2. **产品路由表。** `App` 维护 `PanelInstanceId → 路由目标(project_id / 区域)` 的**产品侧注册表**,`msg.project_id()` 改为查该表——`project_id` 不再搭在 token 上,Host 不解析 `project:5` 业务含义。→ 喂 C11 的"面板实例归属"与 C12 的"焦点去向"。
3. **通用焦点目标(喂 C12)。** host 抽 `FocusTarget`(数据槽/内容槽/window 三类,`PanelInstanceId` 键控);产品只留业务路由(SearchIn/ShowFileHistory 仍映射到具体内容槽)。
4. **PanelId 键控容器(喂 C11)。** host 增 iced rail/pane/divider 控件 + `PanelId→split/collapsed` 通用 dims 存储;产品提供 descriptor(图标/标题/命令)与排序规则。像素布局需真窗回归(A4)。

**为何一次设计而非三片:** C8 的"面板按 scope 路由"落地物就是 C11/C12 所需的"实例→区域/焦点"注册表;分三次会各造一套键控。**风险/未决:** ① home-pane `RailButton` 变体是否进通用控件;② `Side`/`maximized` 是否移入 `bytehost-core`;③ `RightPairSplit`(Agent/Conversations/Usage 轮转,`layout.rs:779`)无通用等价,留产品;④ 真窗像素回归受 Task 08 权限阻塞,只能靠现有几何单测(A4 部分)。**跨仓:** 均需 bytehost 侧新增(按 tag 发布),本地用 uncommitted patch 联调;正式 pin 归 Task 22。

**状态:** 侦察与设计成稿;用户已裁决按此方向实现,建议顺序 **(1)+(2) 关 C8 → (3) 关 C12 → (4) 关 C11**。move (1)+(2) 的 C8 部分已成切片 3(`300c7dab`,见上);move (3) 的 C12 已成切片 4(`c80d443` + `fa9f37a2`,见上);move (4) 的 C11 已成切片 5(`3dad53e` + `b50b5ca0`,见上)。至此设计 move 1–4 全部落地;C9 已成切片 6(`17c0736` + `5669776c`,见上);C7 已成切片 7(`0f9ecc44` + `6ecadf22`,见上)但仅关"命名中立"(几何上收宿主经 Task 08 裁决不做)。C10 已成切片 8 的**设计成稿**(`1b1f858f`)并按**方案 B 落地**(切片 9:`147285f` + `ea1202a0`)。任一实现片需独立提交+回归。


## Task 22 — 固定依赖、独立构建与发布包(计划 §Task 22 `:302+`)— 本地部分 DONE,远端部分阻塞

- **范围裁决(用户已定):** **本地打包 only**——做计划清单第 3/4(部分)/5 条;第 1/2 条(发布 bytehost tag、把 dozer 固定 revision pin 到远端)需**远端写授权**,未做,见阻塞。
- **改动(digger `f718e76`):**
  - **release 构建修复:** `iced` facade 加 `wgpu` feature(`iced_renderer` 在 release 下无后端 feature 会 `compile_error!`);此前 Digger 仅声明 `iced_renderer`(default-features=false、无后端),故 **release 构建从未真正可用**。与 Dozer 同栈。
  - **打包:** 新增 `crates/desktop/packaging/macos/Info.plist`(`ai.byteboy.digger`、可执行 `digger`、`LSMinimumSystemVersion 12.0`、放开 ATS;**本期无图标**,不设 `CFBundleIconFile`;用户裁决)与 `scripts/package-macos.sh`(仿 Dozer `build-macos-app.sh` 的 JSON 取产物路径 + 整体 ad-hoc 重签 + `lsregister`)。Digger 为单二进制应用(无 dozerd/hook/mcp 兄弟),usage-content 等 web 资源树**尚未接通**故不打包。
  - **升级/降级演练单测:** `product.rs` 新增 `a_legacy_layout_migrates_to_writable_and_retains_unknown_ids`(无 `schema_version` 的历史布局经产品适配器迁移为可写、未知 id 进 `retained_unknown`、落盘为稳定 schema)。
- **验证(实机,隔离 HOME):**
  - release 构建成功(`cargo build --release -p digger-desktop`);`cargo test -p digger-desktop --bin digger` **42 passed**(+1 新测);`cargo clippy -p digger-desktop --all-targets` 零告警;`scripts/check-host-deps.sh` ok。
  - 打包产物:`target/aarch64-apple-darwin/bundle/macos/Digger.app`,结构合规、`codesign -dv` = `ai.byteboy.digger` / `Signature=adhoc`、`lipo` = **arm64 only**、`LSMinimumSystemVersion 12.0`。
  - **冷启动:** `open Digger.app` 进程常驻、创建 per-user 数据根(`~/Library/Application Support/ai.byteboy.digger/{bytehost,logs}`)、退出无残留进程。
  - **升级演练:** 旧格式 `layout.json`(无 `schema_version`,`rails[].items` 含未注册 id)→ 隔离 HOME 下启动常驻(迁移为可写)。
  - **降级演练:** `schema_version: 999`(高于支持 1)→ 应用仍启动(内存用默认布局),原文件**逐字节保持**(SHA 前后一致)。
  - **回滚保数据:** `bytehost/apps/<app>/state.json` 用户在降级启动后**原样保留**。
- **偏差 / 发现:**
  - **release 构建此前不可用**(`iced_renderer` 缺后端 feature)——属 Task 22 才暴露的缺口,已修。
  - `has_default_layout` 单测**非 hermetic**(直接读真实 `~/Library/.../digger` 配置根):演练污染该目录会让它失败。已恢复配置并改为**隔离 HOME** 复跑;该非 hermetic 属既有问题,未改(不在本任务范围)。
  - 打包需 `jq`(与 Dozer 一致)。
- **未决项 / 阻塞:**
  - **第 1 条(bytehost 发布 tag):** bytehost 领先 `v0.1.1` 若干提交(C5/C8/C9/C10/C11/C12 宿主侧 + Task 12 管理能力),需**发布新 tag**并两产品统一引用——**待会话授权**(计划 `:13`/`:306`)。当前三仓经**不提交的本地 `[patch]`** 联调。
  - **第 2 条(dozer 固定 revision):** `origin/main`(`ca0a279e`)落后 **176 提交**且**不含 `dozer-panel-usage`**;Digger 对 dozer 的三个 crate 现走本地 patch。正式 pin 需**推送 dozer 到远端**——待授权。干净 clone 不依赖 patch 的目标**未达成**。
  - **第 4 条(签名/公证):** `security find-identity -v -p codesigning` = **0 valid identities**;因无 Developer ID 凭据,**只能产出 ad-hoc 签名的候选包**,不得称"已分发发行版"。
  - **第 3 条真实"安装"**:冷启动已做,但未做全系统范围的安装(拷贝到 `/Applications`);候选包范畴。


## Task 23 — 最终验收与文档收口(计划 §Task 23 `:316+`)— DONE

- **改动:** `acceptance.md` 增 §4 最终验收报告(A1–A18 判定 + 证据 + 未验清单 + API/回滚总结 + 桥收口);bytehost README 修正 Digger 消费者条目;并修复跑批中暴露的两处**真实回归**(见下)。
- **§4 命令实跑结果(本机):**
  - **bytehost** (`fe9373d` 区间): `fmt` clean、`clippy --workspace --all-targets --all-features -D warnings` 0 error、`test --workspace --all-features` 全过、`check-deps.sh` ok、**`process_apps_live` 7/7**(含 Python/Node/archive/升级回滚/敌意压缩包真实进程)、**`embedded_host` 1/1**(进程内无守护进程嵌入)。
  - **dozer** (`8ff87cba`): `check_panel_boundary.py` **ok (216/6)**、`check-log-scope.sh` ok、`test --workspace --no-fail-fast` 仅 `dozer-app --bin dozer` 1 项既有失败(1968 passed/1 failed = `usage_probe::host::host_site_citations...`,基线 §4.1.3,指向用户 WIP)、审计单测 45 项 2 失败(既有 `ScanUsage` 迁库后扫旧路径)。
  - **digger** (`99ad282`): `fmt` clean、`test --workspace --no-fail-fast` 全过、`clippy --workspace --all-targets -D warnings` 0 error、`check-host-deps.sh` ok、`build -p digger-desktop --release` 产出 arm64 adhoc 二进制。
  - **Usage Web**: `typecheck`/`test`(14)/`build` 全过,产物无 diff。
- **跑批中修复的真实回归(本次新增提交):**
  - **bytehost `cbb4719`**: C5–C12 host 侧改动带入的 **`cargo fmt` 违规**(`focus.rs`/`lib.rs` 未 fmt 即提交)——此前切片批漏跑 `fmt`。纯重排,无语义变化。
  - **bytehost `b7183bb` + dozer `8f479a81`**: `clippy -D warnings` 报 `bytehost-desktop::overlay::stack_bounds` **9 参数超阈值**;按仓库约定(**≥7 且相邻同类型参数**用具名字段结构体)重构为 `ToastMetrics{width,height,gap,margin_right,margin_bottom}`,dozer `toast_overlay.rs` 适配,并删 `overlay_window.rs` 未用的 `window_logical_size` 重导出。
  - **dozer `8ff87cba`**: C9 切片 6 退役 `R-HOVERSLOT-VARIANTS` 规则时**漏删陈旧单测** `test_slot_enum_counted_in_panel_host`(仍断言已退役规则);删除该测试并同步 `ENUM` 夹具去掉 `HoverSlot`。
  - **digger `99ad282`**: Task 22 迁移测试 `product.rs` **未 fmt 即提交**,补 fmt。
  - **bytehost `fe9373d`**: README 消费者条目原写「Tauri 宿主,嵌入待做」(两处均误),改为 iced+wry / 进程内嵌入已实现。
- **判定汇总:** **pass 6**(A3/A7/A10/A11/A12 + A1 代码层)· **partial 12**;硬缺口三处——**A1 发布层**(无 tag/revision,靠不提交 patch)、**A17 生产端**(真实生产者未跑、token 如实 0)、**A18 签名**(0 有效身份→仅 adhoc 候选包)。**整体不标"完全接入"。** L3/L4 真机项(A5/A6/A9/A14/A15/A16 等)因屏幕录制/辅助功能权限与无 GUI runner → not-run,并入 §3 人工项。
- **ADR:**Digger ADR-001/011 **保持 Proposed**(A1/A18 未过,不升 Accepted);正文已正确声明"不用 Tauri",未改。
- **未决项 / 阻塞(移交用户裁决):**
  - **C10 完全归零**:方案 B 落地后 `R-OVERLAY-DUP` 残留 **3**(基线上限);到 0 需方案 A/C——**开放后续决策**。
  - **Task 22 远端部分**:bytehost 发 tag、dozer pin revision、签名——**均待远端写授权/Developer ID 凭据**。
- **提交记录(本任务):** bytehost `cbb4719`/`b7183bb`/`fe9373d`;dozer `8f479a81`/`8ff87cba` + 本验收提交;digger `99ad282`。


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
| 07 | `4a8dba0` | B | `feat(desktop): add reusable shell and product callbacks` |
| 08 | `17c4ad4` | B | `feat(core): add Surface lifecycle state machine` |
| 08 | `2aaf935` | B | `feat(desktop): execute Surface lifecycle, routing and geometry` |
| 08 | `f4639bb` | B | `fix(desktop): stop forcing iced renderer features on consumers` |
| 08 | `5b940f11` | Z | `refactor(usage): route Usage content host through shared Surface` |
| 09 | `9b7b5121` | Z | `refactor(codehealth): adopt shared surface lifecycle` |
| 10 | `f3edcef` | B | `feat(desktop): migrate legacy layouts to stable Layout` |
| 10 | `a152a51d` | Z | `feat(app): compat bridge from legacy Dozer layout to shared Layout` |
| 10 | `93b2ac3` | B | `feat(desktop): extract product-neutral overlay focus tracker` |
| 10 | `239aa399` | Z | `refactor(platform): use shared overlay focus tracker` |
| 11 | `3072d62` | D | `feat(desktop): bootstrap Digger on bytehost shell` |
| 12 | `ccb32a8` | B | `feat(apps): one service owner per data root via flock` |
| 12 | `b332684` | B | `feat(apps): controlled gateway port reset management API` |
| 12 | `7daa064` | B | `feat(apps): observable shutdown result without breaking callers` |
| 12 | `3ab1c34` | B | `fix(apps): release the service lock explicitly on drop` |
| 13 | `2305775` | D | `feat(app-host): wire InProcess effects and subscriptions` |
| 14 | `a21b730` | D | `feat(apps): add reviewed install and management flow` |
| 15 | `b24dd20` | D | `feat(apps): wire runtime probe and install effects` |
| 15 | `8a098d4` | D | `feat(apps): add follow-aware log viewer` |
| 15 | `aa91dfd` | D | `feat(apps): wire start-stop, rollback, uninstall and log wiring` |
| 16 | `79e7d8d` | B | `feat(desktop): add the shared restricted application webview builder` |
| 16 | `57378a9` | D | `feat(desktop): enforce shared application surface policy` |
| 17 | `8d0f7f7` | B | `feat(desktop): purge an app data store behind the OS-version gate` |
| 17 | `3399866` | D | `feat(apps): recover store cleanup and gateway settings` |
| R3–R6 | `ba52c94` | B | `fix(core): harden Surface lifecycle, panel identity and request routing` |
| R2 | `fc3a636` | B | `fix(desktop): make physical-slot replacement atomic and unregister occupant-checked` |
| R7 | `2d924d9` | B | `fix(spike): SELFTEST must build the wry child and fail nonzero` |
| R1 | `870817b6` | Z | `fix(usage): open a new Surface generation on same-frame slot replacement` |
| 19 | `d2fd25cc` | Z | `feat(panel-usage): extract shared Usage panel library (Task 19 stage 1)` |
| 19 | `4f2ce8be` | Z | `refactor(panel-usage): move usage-content bundle into shared crate (Task 19 stage 2)` |
| 19 | `df0f0e59` | Z | `refactor(panel-usage): wire Dozer onto shared Usage panel library (Task 19 stage 3)` |
| 19 | `9d0b7278` | Z | `docs(integration): record Task 19 shared Usage panel extraction (stages 1-2)` |
| 19 | `c3d57928` | Z | `docs(integration): record Task 19 stage 3 commit hash in ledger` |
| 20 | `a4c0e5a` | D | `feat(store): introduce digger-store business crate for task usage ledger` |
| 20 | `f529b72` | D | `feat(app-host): expose request ticket from run_install_traced` |
| 20 | `8daf437` | D | `feat(usage): connect Digger task usage to shared panel` |
| 20 | `09111352` | Z | `docs(integration): record Task 20 Digger task usage loop (api-decisions §14)` |
| 21 | `a77af9bb` | Z | `refactor(host): move app-panel effect executor to host_adapters (Task 21 slice 1)` |
| 21 | `6668ce8f` | Z | `docs(integration): record Task 21 slice 1 app-panel executor move` |
| 21 | `ec0f912c` | Z | `refactor(panel): route preview-pane selection through Workspace accessors (Task 21 slice 2)` |
| 21 | `182195a7` | Z | `docs(integration): record Task 21 slice 2 preview-pane unification` |
| 21 | `db2779b9` | Z | `docs(integration): design the shared PanelId/Scope routing seam for bridges C8/C11/C12` |
| 21 | `f50ad77` | B | `feat(core): add ScopedTrackers for product-neutral per-scope request routing` |
| 21 | `300c7dab` | Z | `refactor(host): move panel request identity onto core RequestToken/ScopedTrackers (C8)` |
| 21 | `c80d443` | B | `feat(desktop): add product-neutral FocusRouter for keyboard focus routing` |
| 21 | `fa9f37a2` | Z | `refactor(platform): route keyboard focus through shared FocusRouter (C12)` |
| 21 | `3dad53e` | B | `feat(desktop): add product-neutral PanelId-keyed pane/rail container` |
| 21 | `b50b5ca0` | Z | `refactor(host): route pane split/collapse and rail lookup onto shared PanelId container (C11)` |
| 21 | `17c0736` | B | `feat(desktop): add product-neutral HoverSlot vocabulary` |
| 21 | `5669776c` | Z | `refactor(host): adopt shared HoverSlot vocabulary (C9)` |
| 21 | `0f9ecc44` | Z | `refactor(geometry): rename webview geometry helpers to neutral host-generic names (C7)` |
| 21 | `6ecadf22` | Z | `chore(audit): add R-PREVIEW-NAMING gate for webview geometry naming (C7)` |
| 21 | `1b1f858f` | Z | `docs(spec): C10 overlay windowing seam design note (pending decision)` |
| 21 | `147285f` | B | `feat(desktop): add product-neutral overlay windowing glue` |
| 21 | `ea1202a0` | Z | `refactor(platform): route overlay windowing glue through shared host API (C10)` |
| 22 | `f718e76` | D | `build(digger): pin shared host and package desktop (Task 22, local)` |
| 22 | `031eb956` | Z | `docs(integration): record Task 22 local packaging progress and blockers` |
| 23 | `cbb4719` | B | `style(desktop): cargo fmt focus.rs and lib.rs` |
| 23 | `b7183bb` | B | `refactor(desktop): pass toast geometry via ToastMetrics struct` |
| 23 | `99ad282` | D | `style(digger): cargo fmt product.rs migration test` |
| 23 | `fe9373d` | B | `docs(readme): correct Digger consumer entry (iced+wry, in-process embedding done)` |
| 23 | `8f479a81` | Z | `style(dozer-app): adapt toast overlay to ToastMetrics, drop unused reexport` |
| 23 | `8ff87cba` | Z | `test(audit): drop stale HOVERSLOT test after C9 retired the rule` |
