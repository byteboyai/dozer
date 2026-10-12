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
| R3–R6 | `ba52c94` | B | `fix(core): harden Surface lifecycle, panel identity and request routing` |
| R2 | `fc3a636` | B | `fix(desktop): make physical-slot replacement atomic and unregister occupant-checked` |
| R7 | `2d924d9` | B | `fix(spike): SELFTEST must build the wry child and fail nonzero` |
| R1 | `870817b6` | Z | `fix(usage): open a new Surface generation on same-frame slot replacement` |
