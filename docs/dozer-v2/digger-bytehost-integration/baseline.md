# S0 Task 01 基线（三仓库）

> 计划：[2026-10-11-digger-bytehost-full-integration.md](../../superpowers/plans/2026-10-11-digger-bytehost-full-integration.md) Task 01。
> 规格：[完整接入设计](../../superpowers/specs/2026-10-11-digger-bytehost-full-integration-design.md)。
> 本文记录执行当时的三仓库 revision、环境、门禁结果和既有失败。**不修改任何门禁阈值**；所有既有失败如实记录，不归为"已修复"，新增失败不得归入历史。

## 1. 采集时间与环境

- 采集日期：2026-10-11（会话当日）。
- 主机：Darwin 25.6.0 arm64（macOS 26.6.2，BuildVersion 25G83）。
- 工具链：`rustc 1.98.1 (48a229cea 2026-09-01)`、`cargo 1.98.1 (797e8a9bc 2026-08-05)`、`node v24.14.0`、`npm 11.11.0`、`Python 3.13.1`。

## 2. 三仓库 revision 与工作区状态

仓库根：`/Users/chrischiang/Projects/CoralProjects/byteboy/`；兄弟目录 `dozer`、`bytehost`、`digger`、`byteui`、`bytegit`。

| 仓库 | HEAD | 分支 | 工作区状态（本任务开始时） |
|---|---|---|---|
| Dozer (`dozer/`) | `a4cbf340dec7be39d8f055612f15d50240f0a943` | main | 见 §2.1（含用户既有 WIP，**隔离、不触碰**） |
| bytehost (`bytehost/`) | `6fdca5dcfad12472244ef05a7550c6060d3a0342` | main | 干净 |
| Digger (`digger/`) | `8ef11953c1745fa538025373a17ce3f69ce3b133` | main | 含用户既有 WIP（见 §2.2） |

参考（非本次执行目标，仅锁版本参照）：byteui `8ef64af`、bytegit `0a3d2ab`。

### 2.1 Dozer 用户既有 WIP（本任务不触碰）

`git status --short`（`dozer/`）：

```
 M Cargo.lock
 M crates/dozer-app/Cargo.toml
 M crates/dozer-app/packaging/macos/Info.plist
 M crates/dozerd/src/headless_agent.rs
 M crates/dozerd/src/summary_pipeline.rs
?? docs/superpowers/plans/2026-10-11-digger-bytehost-full-integration.md
?? docs/superpowers/specs/2026-10-11-digger-bytehost-full-integration-design.md
?? docs/superpowers/specs/2026-10-11-dozer-memory-engine-upgrade-design.md
```

- `Cargo.lock` / `crates/dozer-app/Cargo.toml` / `Info.plist` 的修改是上一次 `scripts/build-macos-app.sh release` 触发的版本自增（0.5.290→0.5.291 / build 321），属构建产物，非本任务内容。
- `crates/dozerd/src/{headless_agent,summary_pipeline}.rs` 是用户既有 WIP，本任务不读、不改、不提交。
- 三份 `?? docs/superpowers/...` 中，本次集成计划与规格是本任务的输入；`2026-10-11-dozer-memory-engine-upgrade-design.md` 与本任务无关，仅记录其存在以示隔离。

### 2.2 Digger 用户既有 WIP（本任务不触碰）

`git status --short`（`digger/`）：

```
 M spike/md-mirror/src/lib.rs
?? .dozer/
```

- `spike/md-mirror/src/lib.rs` 是用户既有 spike WIP，本任务不触碰。
- `.dozer/` 为运行期生成目录（含 `description.md`），不纳入本任务提交。

## 3. 平台/架构与锁文件

- 平台：macOS 先发（规格 §1）；架构 `arm64`。
- Dozer 锁文件：`dozer/Cargo.lock`（存在）。关键依赖解析版本：
  - `iced_wgpu 0.14.0`、`iced_widget 0.14.2`、`iced_winit 0.14.1`（iced 0.14 生态）。
  - `wry 0.55.1`、`winit 0.30.13`。
  - `byteui 0.4.1`、`bytegit 0.7.0`。
  - `bytehost-apps 0.1.0`、`bytehost-client 0.1.0`、`bytehost-panel 0.1.0`、`bytehost-webview 0.1.0`（`crates/dozer-app/Cargo.toml` 以 `tag = "v0.1.1"` git 依赖引入；lock 中 package version 为 `0.1.0`）。
- bytehost 锁文件：`bytehost/Cargo.lock`；workspace 使用 edition 2024、resolver 3。
- Digger：**无根 `Cargo.toml`，无根锁文件**；仅有 `digger/spike/*` 下的独立 spike 工程（见 §5）。

## 4. 三仓库基线门禁结果

### 4.1 Dozer

| 检查 | 命令 | 退出码 | 结果 |
|---|---|---|---|
| 面板边界门禁 | `python3 scripts/audit/check_panel_boundary.py` | **1** | 已知回退：`app/app.rs: R-HOST-EXECUTORS 0 -> 1`（见 §4.1.1） |
| 日志来源门禁 | `scripts/check-log-scope.sh` | 0 | 通过 |
| 审计单测 | `python3 -m unittest discover -s scripts/audit -p 'test_*.py'` | **1** | 46 项中 1 项失败：`test_unit_edges.ScanUsage.test_usage_has_no_unclassified_edges`（见 §4.1.2） |
| dozer-app 单测 | `cargo test -p dozer-app` | **1** | 1 项既有失败：`extensions::files::tests::delete_confirm_spec_reflects_pending_target`（见 §4.1.3） |

#### 4.1.1 面板边界基线（`scripts/audit/panel-boundary.baseline.json`）

已提交基线值（只降不升）：

| 文件 | 规则 | 基线值 |
|---|---|---|
| `app/state.rs` | R-HOVERID-VARIANTS | 12 |
| `app/update.rs` | R-HOST-PANEL-ARMS | 95 |
| `app/update.rs` | R-PANE-PICK | 1 |
| `panel_host.rs` | R-HOVERSLOT-VARIANTS | 9 |
| `platform/window_events.rs` | R-PANE-PICK | 3 |
| `workspace/state.rs` | R-PANE-PICK | 2 |

实测**回退**（未修改基线掩盖）：`app/app.rs` 的 `R-HOST-EXECUTORS`（`app/` 下 `run_<面板>_effects` 执行器个数）实测 `1`，基线 `0`。该回退在 `a4cbf340`（request_tracker）之前/之后的定位见 ledger；本任务按"门禁只降不升"记录，不掩盖。

#### 4.1.2 审计单测既有失败

`test_unit_edges.ScanUsage.test_usage_has_no_unclassified_edges` 断言"Usage 无未分类边"，实测存在 2 条 `?` 边：

- `extensions/toast/mod.rs:4`
- `usage_probe/fake_usage/mod.rs:1`

在基线上即可复现，属既有失败；**不得**通过放宽断言掩盖。

#### 4.1.3 dozer-app 既有失败

`extensions::files::tests::delete_confirm_spec_reflects_pending_target`：
期望 `删除文件 "报告.pdf"?`，实得 `删除文件 "/tmp/报告.pdf"?`。在基线上即可复现，与本次集成无关，如实记录。

### 4.2 bytehost

| 检查 | 命令 | 退出码 | 结果 |
|---|---|---|---|
| 依赖/驻留门禁 | `bash scripts/check-deps.sh` | 0 | 通过 |
| 格式 | `cargo fmt --all --check` | 0 | 干净 |
| 全 feature 测试 | `cargo test --workspace --all-features` | 0 | 通过（408+95+13+3，另有 ignore 项） |
| Clippy | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 | 通过（无告警） |

### 4.3 Digger

- 无正式 workspace：**正式 `Cargo.toml` 尚不存在**，因此无正式测试/clippy/门禁基线可测。本任务明确记录："正式 workspace 尚不存在"，不把 spike 的测试当作正式产物基线。
- spike 目录：`digger/spike/{block-identity,editor-host,md-engine,md-mirror,parser-agreement,text-merge}`。
- 关键输入 `spike/editor-host`（Task 02 依据）：`iced 0.14`（feature `tokio`）+ `wry 0.55.1`；以路径依赖 `bytehost-webview = { path = "../../../bytehost/crates/bytehost-webview" }` 复用导航/存储标识/IPC nonce 的唯一真相（ADR-011）。
- Digger 文档：`digger/docs/adr/ADR-001-iced-wry-desktop-shell.md`、`ADR-011-bytehost-app-host-embedding.md`、`ADR-012-markdown-text-as-authoritative-content.md`、`ADR-003-project-sqlite-consistency-island.md`、`ADR-006-writing-document-representation.md` 等（规格 §10 要求完工后同步的 ADR 为 001/011）。

## 5. 统计口径（后续 Task 保持同口径）

PanelKind 点名 / 宿主面板消息臂 / 业务状态字段的统计命令与数值，见 [ownership.md](./ownership.md) §4。命令：

```bash
rg -c "PanelKind::" crates/dozer-app/src/{app/update.rs,app/app.rs,panel_host.rs,panel_registry.rs,workspace/state.rs,platform/window_events.rs}
python3 scripts/audit/check_panel_boundary.py
```

## 6. 未决/注意

- Dozer 面板边界存在 1 项**已知回退**（R-HOST-EXECUTORS 0→1），须在后续宿主抽取中归还；不得为过门禁而改基线。
- Dozer 两处既有失败（§4.1.2、§4.1.3）复现一致，非本任务引入。
- Digger 无正式 workspace，A1/A18 等依赖正式构建的验收在其建立后（Task 11+）方可执行。
