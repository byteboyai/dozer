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

## 提交记录

| Task | 提交 | 仓库 | 说明 |
|---|---|---|---|
| 01 | `a9af4903` | Z | `docs(integration): record host extraction baseline` |
| 01 | `7924dc6c` | Z | `docs(integration): record Task 01 commit hash in ledger` |
| 02 | `0ca8320` | B | `spike(desktop-contract): iced×wry 桌面契约探针(Task 02 §11 闸门1)` |
| 02 | `d5ab7f83` | Z | `docs(integration): Task 02 窗口入口与泛型 API 决策` |
| 03 | （见下） | Z | `docs(integration): Task 03 §11 闸门回填` |
