# S0 Task 01 验收台账与兼容桥清单

> 计划 Task 01 第四项：为规格 §9 的 A1–A18 各建**未完成条目**，记录验证层级、平台和所需夹具；建立兼容桥清单，删除条件必须可测。
> 规格：[完整接入设计](../../superpowers/specs/2026-10-11-digger-bytehost-full-integration-design.md) §9/§10。
> 验证层级：**L1** 纯状态/契约测试 · **L2** 进程与 Gateway 集成测试 · **L3** 真实 WKWebView/窗口行为 · **L4** 人工（视觉/真实鼠标焦点/浮层/IME）。禁止把 handler 直接调用当作真实点击验证。
> 所有条目当前状态为 **未完成（Unmet）**。

> **Task 23 最终验收（2026-10-12）见文末 [§4](#4-task-23-最终验收报告2026-10-12)。** §1 状态列为 Task 01 建台时的初始值，保留不改；最终判据以 §4 为准。

## 1. A1–A18 台账

| 编号 | 场景 | 验证层级 | 平台 | 所需夹具 | 状态 |
|---|---|---|---|---|---|
| A1 | 独立构建 Digger/Dozer：固定依赖可解析、无兄弟目录 patch、共享 Host 无产品依赖 | L1+L2 | macOS | Digger 正式 workspace（尚不存在）；bytehost tag；`scripts/check-deps.sh` 分组门禁 | Met（发布层）：bytehost `v0.2.0` tag + dozer revision `67ab0e65`，`[patch]` 已删，干净解析构建通过 |
| A2 | 一个面板 + Host + fake 服务：真实显示/激活/关闭，不加载其他面板 | L2+L3 | macOS | `bytehost-desktop` + fake Services + 最小面板夹具 | Unmet |
| A3 | 切 scope/关闭重建/乱序回包：错 scope、旧实例、旧请求结果不改变状态 | L1 | 跨平台 | core 请求身份测试（Task 05） | Unmet |
| A4 | 旧布局迁移、单面板与空栏：Dozer 布局等价；Digger 布局不受固定清单约束 | L1+L2 | macOS | 旧落盘布局样本；`bytehost-core` 布局 DTO（Task 04） | Unmet |
| A5 | ready、隐藏、恢复、销毁：首帧完整快照、恢复最新态、旧 IPC 丢弃、无重复原生实例 | L2+L3 | macOS | 应用/编辑器 webview 夹具（Task 16–18） | Unmet |
| A6 | resize、DPI、分栏、原生/in-window 浮层：几何/遮挡/点击正确，关浮层后可继续输入 | L3+L4 | macOS | 真实窗口 + 浮层；`webview_geometry` 迁移后夹具 | Unmet |
| A7 | 静态、Python、Node 应用：审批→安装→启动→交互→停止→重启，真实显示内容 | L2+L3 | macOS | 三类应用打包样例；运行环境（Python/Node） | Unmet |
| A8 | 升级失败、回滚、卸载两种模式：包版本/状态/应用数据/浏览器数据与用户选项一致 | L2+L3 | macOS | 升级/回滚包；卸载含数据样本 | Unmet |
| A9 | 运行时缺失/安装失败/运行时卸载：持久问题页、保留原因、可恢复 | L3+L4 | macOS | 受管运行时缺失/卸载夹具 | Unmet |
| A10 | Gateway 鉴权与导航：合法 cookie 200；无凭据 403；未知 Host 拒绝；跨 app/外站/新窗口被拒 | L2 | macOS | origin gateway 集成测试（bytehost） | Unmet |
| A11 | 伪造 IPC、合成事件、权限尝试：无业务桥突破；策略与真实执行一致；token 不出现在产物 | L2+L3 | macOS | 伪造 IPC/越权页面夹具（`SELFTEST=policy` 式） | Unmet |
| A12 | app A/B 存储及重启/删除数据：不串数据、同 app 保持、删除后无旧数据复活 | L3 | macOS | 两应用 + 存储清理夹具 | Unmet |
| A13 | 订阅断开、滞后、并发刷新：重同步恢复、轮询兜底、无过期列表/URL 覆盖 | L2+L3 | macOS | 订阅源夹具（Task 19） | Unmet |
| A14 | 正常退出、强杀、双启动：端口释放、孤儿处理、同根双服务拒绝、不误杀他进程 | L2 | macOS | AppService 进程夹具（Task 03/12/14/15） | Unmet |
| A15 | Dozer 与 Digger 同时运行：根目录和端口独立，不串注册表与应用数据 | L2 | macOS | 双产品并行运行夹具 | Unmet |
| A16 | Project 移动/导出与无 Project 窗口：应用不进入 Project 内容；无 Project 也能管理应用 | L2+L3 | macOS | 无 Project 窗口夹具 | Unmet |
| A17 | Digger Usage 真实服务：真实任务用量映射正确；空与失败不同；不依赖 dozerd | L1+L2 | macOS | Digger 真实用量账本（**尚不存在**，Task 03/20 决定） | Unmet |
| A18 | macOS 发布包冷启动：资源齐全、UI/内容可运行、退出正确；系统架构按实际发布构建记录 | L3+L4 | macOS | Digger/Dozer 发布包（Task 22/23） | Unmet |

**既有失败基线（贯穿所有阶段）：** Dozer 边界门禁 `R-HOST-EXECUTORS` 0→1、审计单测 `test_unit_edges.ScanUsage`、dozer-app `extensions::files::tests::delete_confirm_spec_reflects_pending_target`（见 [baseline.md](./baseline.md) §4）。这些为**历史失败**，不计入新增失败；后续每阶段门禁"只降不升"。

> **2026-10-16 更新:** 上述三项历史失败已清零（`delete_confirm_spec…` 已自行转绿；`host_site_citations…` 改为按文件内容核对；`ScanUsage` 跟随 Usage 面板搬到 `dozer-panel-usage`）。`cargo fmt --all --check` 现为 clean。clippy 仍有 33 条 warning（死代码与风格类，未纳入本次），基线数字见 `cargo clippy --workspace --all-targets 2>&1 | grep -c '^warning'`。

## 2. 兼容桥清单（临时适配器）

删除条件均**可测**（有明确断言或门禁指标）。目标收口 Task：21（通用兼容桥清零）。

| 编号 | 桥 | 位置 | 存在理由 | 删除条件（可测） | 目标 Task |
|---|---|---|---|---|---|
| C1 | `dozer_core::protocol` 兼容出口（线上类型） | `crates/dozer-core/src/protocol` | 面板代码历史上从 `dozer_core::protocol` 取类型 | 无面板/模块再 `use dozer_core::protocol`（`R-PROTOCOL` 门禁计数归零） | 21 |
| C2 | `Client` 直连面板 | `crates/dozer-app/src/extensions/**` | 渐进迁移到 `Services` 接缝 | `extensions/**` 不再出现 `dozer_client::Client`（`R-CLIENT` 门禁归零） | 06 / 21 |
| C3 | `service_fakes.rs` 先构造离线 `Client` 再覆盖 | `crates/dozer-app/src/service_fakes.rs` | 旧 fake 构造路径 | fake 不再构造 `Client`（构造点断言） | 06 ✅ |
| C4 | `services_for`/`ClientDaemon` 在产品组合根 | `crates/dozer-app/src/panel_host.rs`（Task 01 实测位置；ownership 记 `product.rs`） | 服务构造尚未移入产品适配层 | 移入 `host_adapters/services.rs` 且组合根不再直接构造 | 06 ✅ |
| C5 | 预览窗格按 `PanelKind::Project` 手写选择 | `workspace/state.rs`（`R-PANE-PICK` 2）、`app/update.rs`（1）、`platform/window_events.rs`（3） | 未走 `Workspace::preview_pane[_mut]` | `R-PANE-PICK` 三处计数归零 | 21 |
| C6 | `app/` 下 `run_<面板>_effects` 执行器 | `app/app.rs`（`R-HOST-EXECUTORS` 实测 1） | host 替面板做事，未外移 | `R-HOST-EXECUTORS` 计数归零（≤ 基线 0） | 06 / 08 / 09 / 21 |
| C7 | `webview_geometry.rs` 的"文件预览"命名遗留 | `crates/dozer-app/src/webview_geometry.rs` | 几何与文件预览业务命名混同 | 几何函数迁入 Host 通用名；无"preview"命名残留 | 08 / 09 |
| C8 | `request_tracker.rs` 原型迁移后删除 | `crates/dozer-app/src/request_tracker.rs` | 是 core 请求身份的移植来源 | 使用 core 请求身份且 delete 该文件后编译通过 | 05 / 21（06 已接入 core；`RequestToken` 产品包装待 21 删除） |
| C9 | `HoverId`/`PanelKind` 作为宿主类型 | `panel_host.rs` / `app/state.rs` | 词汇尚未命名空间化/注册制 | `HoverSlot`/`PanelKind` 变体表纳入注册制后门禁计数只降 | 09 |
| C10 | 原生浮层建窗/渲染机制（`overlay_window.rs`/`overlay_gpu.rs`/`toast_overlay.rs`） | `crates/dozer-app/src/platform/*_overlay*.rs` | 建窗需绑定 `iced_wgpu`/`iced_widget`/`iced_renderer`/`winit` 与渲染器类型 + `theme` 遮罩色；bytehost-desktop 刻意不绑渲染器 feature（`Cargo.toml:12-14`，R2 修复 `fbc3a636`），且 winit 尚非其依赖 | 把建窗/GPU/遮罩机制抽入 bytehost-desktop **且**产品接缝只留内容与主题色，或产品侧浮层全量改用共享建窗 API；届时 `R-OVERLAY-DUP`（新门禁）归零 | 21（**已收口**：方案 B 建窗/几何上收；方案 C 加 `overlay` feature 上收 `OverlayGpu`/`backdrop_card*`，`R-OVERLAY-DUP` 3→0。see ledger Task 21 切片 9–10） |
| C11 | rail/pane 容器、尺寸/折叠、拖拽（`chrome/rail.rs`、`app/layout.rs`、`app/view.rs`） | `crates/dozer-app/src/chrome/rail.rs` 等 | 与 `PanelKind`/`App`/`ShellState` 深度耦合；`RailLayout` 直接持 `Vec<PanelKind>`，`Divider` 有 13 个面板专属变体；bytehost-desktop 尚无 `PanelId` 键控的 rail/pane/drag API | 设计出 `PanelId` 键控的 rail/pane 容器后 Dozer 适配接上，且像素布局回归通过（A4/real-window） | 21 |
| C12 | 焦点路由（`FocusIntent`/`pending_focus`/`apply_pending_focus`） | `crates/dozer-app/src/platform/window_events.rs` | 全部以 `PanelKind` 为键；通用"焦点去向"机制未抽（纯 `FocusTracker` 已于本任务抽取，见 ledger Task 10） | 通用焦点目标类型进 bytehost-desktop，产品只留业务路由（SearchIn/ShowFileHistory 等）；门禁计数只降 | 21 |

**门禁约束：** 上述计数类删除条件与 `scripts/audit/panel-boundary.baseline.json` 及 `R-PROTOCOL`/`R-CLIENT` 门禁一一对应；删除桥时只允许基线**下降**，禁止上调阈值掩盖。

## 3. Task 08 真实窗口人工确认（L3+L4，待人工）

自动化受限：`screencapture` 缺屏幕录制权限；Orca 辅助 App 缺辅助功能权限（`lsappinfo windowInfo=[NULL]`），无法读取 Dozer 控件树。以下需人工在真实窗口执行：

| 步骤 | 操作 | 预期 |
|---|---|---|
| M1 | 打开 Usage 面板（有 agent 数据的项目） | 内容侧图表首帧渲染；无空白/无 `all_agents` 旧值残留 |
| M2 | 在项目 A（已渲染）切换到项目 B | 内容随 B 更新；B 未发过的 payload 也推送（不因单槽 `last_sent` 漏推） |
| M3 | 快速隐藏 Usage 面板再恢复 | 恢复后重发当前内容，不空白；隐藏期间不推（`PushState` 缓存最新，恢复一次补齐） |
| M4 | 切走再切回同一项目（webview 销毁重建） | 新实例 Ready 后强制重发同一内容（代数递增，旧实例在途 IPC 不复写新实例） |

已完成的自动化前置：`cargo test -p dozer-app --bin dozer usage`（100 通过，含上述语义的状态测试）；新二进制真实启动无 panic/ERROR（日志 `dozer-app.log.2026-10-12`）。

---

## 4. Task 23 最终验收报告（2026-10-12）

> 口径：**不把 handler 直接调用当真实点击；不以编译成功代替运行；未实机/无凭据者明确标 not-run 或 partial，整体不标"完全接入"。** 证据为三仓库本地 HEAD；未推送远端。

### 4.1 三仓库 revision

| 仓库 | HEAD | 说明 |
|---|---|---|
| `bytehost` | `7f25d71` | **已推远端 + tag `v0.2.0`**（`6fdca5d..7f25d71`，28 提交）。C5–C12 host 侧与 C10 方案 C `overlay` feature 均在此区间。 |
| `dozer` | `67ab0e65` | **已推远端**（`ca0a279e..862421aa`，含 `dozer-panel-usage`；再追加 `67ab0e65` 全仓 bytehost 依赖 `v0.1.1 -> v0.2.0`）。 |
| `digger` | `71280a7` | 本地（**该仓无远端**）；已钉 bytehost `v0.2.0` tag + dozer revision `67ab0e65`，本地 `[patch]` 已删。 |

> 三仓的 bytehost/dozer `[patch]` 已**删除**；`cargo fetch`/`build --workspace` 从已发布 tag（bytehost `v0.2.0#7f25d71f`）+ revision（dozer `67ab0e65`）解析。A1「固定依赖可解析、无兄弟目录 patch」的**代码层与发布层证据均已具备**（依赖闭包无产品依赖，见 4.2）；A1 判定见 4.6。

### 4.2 §4 命令执行结果（本机实跑）

**bytehost**（`fe9373d` 前为 `cbb4719`/`b7183bb`；下列为该区间实跑）：

| 命令 | 结果 |
|---|---|
| `cargo fmt --all --check` | ✅ clean |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ 0 error（修复 `stack_bounds` 9 参数 → `ToastMetrics` 结构体后） |
| `cargo test --workspace --all-features` | ✅ 421 + 其余目标全过，0 failed（7 处 ignored 见下） |
| `bash scripts/check-deps.sh` | ✅ ok（desktop 组零产品依赖） |
| `cargo test -p bytehost-apps --test process_apps_live -- --ignored` | ✅ 7 passed（Python/Node/archive/升级回滚/敌意压缩包，真实进程） |
| `cargo test -p bytehost-client --test embedded_host -- --ignored` | ✅ 1 passed（进程内嵌入无守护进程） |

**dozer**（`8ff87cba`）：

| 命令 | 结果 |
|---|---|
| `cargo fmt --all --check` | ⚠️ 见 4.3 说明（既有 import 排序差异，非本次引入；未新增 diff） |
| `cargo test --workspace --no-fail-fast` | ⚠️ 1 target failed：`dozer-app --bin dozer` = **1968 passed / 1 failed**，失败为既有 `usage_probe::host::tests::host_site_citations_point_at_the_code_they_name`（基线 §4.1.3，指向用户 WIP 的 `app/update.rs`，非本次回归） |
| `cargo clippy -p dozer-app --all-targets` | ⚠️ 剩余 warning 全在用户 WIP 文件（`chrome/homespace.rs`/`confirm_overlay.rs`/`extensions/files/*`）；本次触碰的 `overlay_window.rs`/`toast_overlay.rs` 零 warning |
| `python3 scripts/audit/check_panel_boundary.py` | ✅ `ok (213 refs in 5 files, baseline ratchet)`（C10 方案 C 后 `platform/overlay_gpu.rs` 从扫描消失，`R-OVERLAY-DUP` **3 → 0**） |
| `python3 -m unittest discover -s scripts/audit -p 'test_*.py'` | ⚠️ 45 项中 **2** 失败，均为既有 `test_unit_edges.ScanUsage`（Usage 迁库后审计脚本扫旧路径，见 4.4）；本次修掉 1 项 C9 遗留陈旧测试（`test_slot_enum_counted_in_panel_host`） |
| `bash scripts/check-log-scope.sh` | ✅ ok |

**digger**（`99ad282`）：

| 命令 | 结果 |
|---|---|
| `cargo fmt --all --check` | ✅ clean（修复 `product.rs` 迁移测试格式后） |
| `cargo test --workspace --no-fail-fast` | ✅ 全目标 0 failed（desktop bin 42 passed，含新增迁移单测） |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ 0 error |
| `bash scripts/check-host-deps.sh` | ✅ ok（审视 1 个业务 crate） |
| `cargo build -p digger-desktop --release` | ✅ 产出 `target/aarch64-apple-darwin/release/digger`（Mach-O thin arm64，adhoc-sign） |

**Usage Web**（`crates/dozer-panel-usage/web/usage-content`）：`npm run typecheck` ✅ · `npm test` ✅ 14 passed · `npm run build` ✅ 产物无 git diff（提交资产与源一致）。

**Task 22 远端 pin 后（2026-10-13，无任何本地 `[patch]`）**：

| 命令 | 结果 |
|---|---|
| dozer `cargo build --workspace` | ✅ 从 tag `v0.2.0#7f25d71f` 解析，`Finished`（无 bytehost patch） |
| digger `cargo fetch` / `cargo build --workspace` | ✅ 从 tag `v0.2.0` + revision `67ab0e65` 解析，`Finished` |
| digger `cargo test --workspace` | ✅ 全绿（39/16/12/7/42/15，0 failed） |
| digger `bash scripts/check-host-deps.sh` | ✅ ok（审视 1 个业务 crate） |

### 4.3 fmt 说明（dozer）

`rustfmt --check` 对 `overlay_window.rs` 报 2 处 import 排序差异，**HEAD 版本同样报 2 处**（本次未增加）；`toast_overlay.rs` HEAD 本就 clean。dozer 仓库根无 `rustfmt.toml`，此为既有历史差异，非本次引入，按"只降不升"口径记录，未强行全仓重排（会混入用户 WIP）。

### 4.4 已知失败基线（贯穿，不计新增）

- 审计单测 `test_unit_edges.ScanUsage` **2 项**失败：`test_usage_has_no_unclassified_edges`、`test_usage_depends_on_workspace_helpers`。根因同一：Task 19/20 把 Usage 迁到 `dozer-panel-usage` 后，`scripts/audit/unit_edges.py` 仍扫旧 `host_adapters/usage_panel` 路径（`baseline.md` §4.1.2 记 1 项，实际 2 项，为文档漂移）。**非本次回归**。
- `dozer-app` 单测 `usage_probe::host::tests::host_site_citations_point_at_the_code_they_name`（`baseline.md` §4.1.3），指向用户 WIP 的 `app/update.rs`。
- 环境噪声（忽略）：`libLLVM.dylib` strip 警告、`block v0.1.6` future-incompat。

### 4.5 A1–A18 最终判定

| 编号 | 判定 | 依据 / 缺口 |
|---|---|---|
| A1 独立构建 | **pass** ✅ | 代码层：`check-deps.sh`（两仓）✅、desktop 组零产品依赖 ✅、依赖闭包可解析 ✅。发布层：bytehost **tag `v0.2.0`**（推远端）+ dozer revision `67ab0e65`（推远端）、三仓 `[patch]` 已删 → 「干净 clone 不依赖 patch」**达成**（2026-10-13，Task 22 远端补齐；`cargo fetch`/`build --workspace` 全绿）。 |
| A3 请求身份 | **pass** ✅ | core 请求身份（Task 05/06）+ C8 收口（`RequestToken` 挂 core，`ScopedTrackers` 去产品包装）；`dozer-app` 全量单测含乱序/scope/实例重建语义通过。 |
| A4 布局 | **partial** ⚠️ | `bytehost-core` 布局 DTO + Digger 迁移单测 + 旧布局黄金文件（Task 04/10）✅；「空栏真窗口」属 L4 人工，见 §3。 |
| A5 Surface 生命周期 | **partial** ⚠️ | 状态机/旧 IPC 丢弃/实例代际（Task 08/09 L2）✅；真实 WKWebView 首帧/恢复快照（L3）受窗口服务器权限限制 → not-run（并入 §3 人工项）。 |
| A6 几何与浮层 | **partial** ⚠️ | 几何纯函数（resize/DPI/分栏）+ C10 宿主建窗机制 ✅；真实浮层遮挡/点击/焦点（L3+L4）→ not-run（权限，并入 §3）。 |
| A7 三种运行时应用 | **pass（L2）** ✅ | `process_apps_live` 7/7：静态拒进程、Python 端到端、Node 端到端、archive 源自 Python、依赖安装失败上报、敌意压缩包拒绝、Python 升级回滚。真实**显示/交互**（L3）→ not-run。 |
| A8 版本与卸载 | **partial** ⚠️ | 升级失败/回滚（`process_apps_live::python_sample_upgrade_and_rollback`）✅；Digger 侧升级/降级/回滚三场景**实机演练全过**（隔离 HOME，见 ledger Task 22）；「含数据卸载」两种模式未逐项实跑 → partial。 |
| A9 运行时问题 | **partial** ⚠️ | 依赖安装失败→持久问题上报（L2）✅；运行时缺失/卸载的持久状态页（L3+L4）→ not-run。 |
| A10 鉴权/导航 | **pass（L2）** ✅ | bytehost gateway 集成（合法 200/无凭据 403/未知 Host 拒/跨 app 拒）+ `AppOrigin::allows_navigation` 唯一真相 + dozer 门禁测试。真实 WKWebView 探针（L3）→ not-run。 |
| A11 IPC/权限 | **pass（L2）** ✅ | `AppIpc::parse` 随机 nonce、`build_app_webview_pins_the_restrictive_settings` 钉死 wry 限制项、CSP 不放宽；「token 不出现在产物」经 log-scope 门禁 ✅。对抗页面真机（L3）→ not-run。 |
| A12 存储隔离 | **pass（L2）** ✅ | 每应用 `data_store_identifier`（算法钉死）+ `StoreRemovals` 清队列；dozer `app_webview.rs` 门禁测试。两应用跨重启实机（L3）→ not-run。 |
| A13 订阅与迟到结果 | **partial** ⚠️ | 断线重订阅/Resync/乱序状态测试（Task 13/19）✅；Gateway 断连实机（L2/L3）→ not-run。 |
| A14 退出/恢复 | **partial** ⚠️ | 正常/强杀/双启动：同根互斥、端口重置、可观察 shutdown（Task 12/14/15，L2）✅；**真实进程强杀/双启动实机** → not-run（并入 §3）。 |
| A15 两产品并行 | **partial** ⚠️ | 独立根/端口/存储逻辑（Task 18）✅；Dozer 与 Digger **同时运行**实机 → not-run。 |
| A16 Project 隔离 | **partial** ⚠️ | 应用不进 Project 内容、无 Project 也能管理应用（逻辑/单测）✅；**Project 真实导出/移动**与无 Project 窗口实机 → not-run（用户裁决未跑）。 |
| A17 真实 Usage | **partial** ⚠️ | Digger 用量账本迁移（Task 19/20）✅、Usage Web 14 测试 ✅；**真实生产事件→账本→UI、跨重启**未实跑 → not-run。**应用运行携带 token 诚实为 0 → S6/S7 仍不完整**（计划 `:284`）。 |
| A18 发布包 | **partial** ⚠️ | Digger release 构建 ✅、候选包 `Digger.app`（`ai.byteboy.digger`, min-OS 12.0, arm64, adhoc-sign）冷启动/资源/退出演练 ✅（Task 22，隔离 HOME）；**无 Developer ID Application 证书（2026-10-13 诊断：仅有过期 Apple Development 证书与中间 CA）→ 只能 ad-hoc 候选包，不是已分发发行版**；用户裁决不申请 Developer ID；Dozer 发布包未在本次重跑。 |

**合计：pass 7 · partial 11 · not-run 0（归入 partial 内的 not-run 项见 §3/§4.5 备注）。整体不标"完全接入"**——A17 生产端、A18 签名两处硬缺口（A1 发布层已于 2026-10-13 补齐）。

### 4.6 未验项明确清单（不得标"完全接入"）

1. ~~**A1 发布层**：bytehost 未发 tag、dozer 未 pin revision；三仓依赖不提交 `[patch]`——需**远端写授权**。~~ ✅ **已完成（2026-10-13）**：bytehost tag `v0.2.0`、dozer revision `67ab0e65` 均推远端，`[patch]` 已删，干净解析构建通过。
2. **A17 生产端**：真实 Usage 生产者事件→账本→UI→跨重启；应用运行 token 诚实为 0（S6/S7）。**注（2026-10-13 复审）**：账本侧一致性缺陷 #1/#9 已修（见 §4.10），#3/#4 仍属 A17 缺口（应用视口未接 / Usage webview 未挂载）。
3. **A18 签名**：**无 Developer ID Application 证书可用**——本机诊断（2026-10-13）：Apple ID `chrischiang@live.cn` / Team `38K4BYSQTE` 已登录，但 keychain 内仅有 ①Apple 的 *Developer ID Certification Authority* 中间 CA（不可用于签名）与 ②已**过期**的 *Apple Development: Chuan Jiang*（`notAfter 2023-01-17`），`security find-identity -v -p codesigning` = **0**。用户裁决**不申请 Developer ID**（需付费 Apple Developer Program）→ 维持 ad-hoc 候选包，**不公证、不分发**；`package-macos.sh` 保持 ad-hoc-only 设计（`codesign --force --deep --sign -`，脚本注释已声明需真实身份与凭据方为发行版）。
4. **§3 真实窗口人工项（M1–M4）**：屏幕录制/辅助功能权限缺失，`screencapture` 与 Orca 辅助 App 均不可用 → 需人工在真实窗口执行。
5. **L3 真实 WKWebView 项**：A5/A6/A7(显示交互)/A9/A10/A11/A12/A13/A14/A15/A16 的真机探针 → 无 GUI runner，移交可用 macOS 环境。

### 4.7 ADR 与 README 收口

- **bytehost README** 已修：Digger 消费者条目原写「**Tauri 宿主,嵌入待做**」——两处均错，改为「iced + wry 宿主,**进程内嵌入已实现**（本地候选）」，并注明 Tauri 方案已撤回（`fe9373d`）。
- **Digger ADR-001 / ADR-011**：**保持 `Status: Proposed`**，未升 Accepted——按 Task 23「ADR 仅在其全部必需验证通过后标 Accepted」，而 **A18 签名**未过（A1 发布层已于 2026-10-13 补齐）。ADR-001/011 正文本身已正确声明"不使用 Tauri"，无需改。Digger Technical Architecture §21 亦已写明"Tauri and React are not used"。

### 4.8 新增公共 API / 剩余产品代码 / 兼容格式 / 回滚（总结）

- **bytehost 新增公共 API**（C5–C12 收口）：`bytehost-desktop::hover::HoverSlot`（C9）、`panes::{PaneDims,PaneSpec,PaneStore,RailItems,apply_pane_split_drag}`（C11）、`focus::{FocusRouter,FocusTarget}`（C12）、`overlay::{open_overlay,reposition_overlay,open_child_window[_unfocused],full_window_overlay_bounds,centered_card_offset,popup_card_size,stack_bounds,ToastMetrics,...}`（C10 方案 B）、`overlay_gpu::{OverlayGpu,apply_cursor,backdrop_card,backdrop_card_shrink}`（C10 方案 C，feature `overlay`）、`bytehost-core` 布局 DTO + `bytehost_core::request` 请求身份。
- **剩余产品代码**：Dozer 侧 `PanelKind` 业务键（`stable_id()`）、`FocusIntent` 业务路由、rail/pane 落盘命名兼容面、`overlay_window.rs`（薄适配：注入主题遮罩色 + wgpu 工厂）、`toast_overlay.rs` 薄壳（仅内容与主题色）。
- **兼容格式**：布局 DTO 保留命名字段兼容面；`schema_version` 未知高版本拒覆盖、低版本迁移；`retained_unknown` 保留未知面板键。
- **回滚**：升级失败保留原文件（SHA 不变，Task 22 演练）；用户 `bytehost/apps/<app>/state.json` 跨回滚存活；卸载「含数据」走 `StoreRemovals` 清 WKWebsiteDataStore。

### 4.9 兼容桥收口状态

C1–C12 见 §2。C1/C2/C3/C4 已归零（早期 Task）；C5/C6/C7/C8/C9/C11/C12 于 Task 21 收敛（对应门禁规则退役或棘轮下降）。**C10 已归零**：依用户裁决先后走方案 B（建窗/几何上收）与**方案 C**（feature-gated 上收 `OverlayGpu`/`backdrop_card*`），`R-OVERLAY-DUP` **3 → 0**（`platform/overlay_gpu.rs` 从扫描消失，基线 216/6 → 213/5）。全部兼容桥的删除条件均已满足。

### 4.10 复审（2026-10-13）9 项发现的处置

复审文件 `review-2026-10-13-full.md` 对「Task 01–23 全部 DONE」的结论提出 9 项异议；本体与复审就 A17/#3/#4 及 A17/A18 属已记录缺口的判断一致。处置：**「小修复优先」**——先修 #1/#7/#8/#9，大面 #2/#3/#4/#5/#6 归入后续计划。

| # | 级别 | 结论 | 处置 |
|---|------|------|------|
| 1 | P1 | `run_id` 跨重启碰撞（`action+sequence`，而 `RequestTable` 每启动从 0 计数）→ 账本按 `run_id` upsert 会覆盖上一次真实操作 | ✅ **已修**（digger `99d5aee`）：`UsageRuntime` 加 `boot_id`（nanos+pid），`run_id = digger:{boot_id}:{action}:{key}`；回归测试 `distinct_ops_across_restarts_do_not_overwrite_each_other`（修复前失败） |
| 2 | P1 | 无真实 shutdown（退出仅 `save_layout()`+`iced::exit()`） | ⏸ 归入后续计划（大面） |
| 3 | P1 | 无应用启动/视口（真机 `LaunchUrl` 未接） | ⏸ 归入后续计划（A17/§4.6 已记录） |
| 4 | P1 | Usage webview 未挂载（无 Surface/资源/payload） | ⏸ 归入后续计划（与 #3 独立，见 §4.6/复审） |
| 5 | P1 | 「含数据卸载」未接线（`take_ready`/`store_removal_task` 生产端） | ⏸ 归入后续计划（中面） |
| 6 | P2 | 事件订阅未接 | ⏸ 归入后续计划（大面） |
| 7 | P2 | 启动失败不可重试（`Opened` 提前置位、`ServiceFailed` 不复位） | ✅ **已修**（`8e75212`）：`ServiceFailed` 复位 `opened`；新增 `Message::RetryService` + 界面「重试」；回归测试 `a_failed_service_start_can_retry` |
| 8 | P2 | 状态机 `Effect::Notice` 落 `_ => None` 被静默丢弃（回滚/运行时卸载失败无提示） | ✅ **已修**（`743f452`）：组合根接住 `Notice` → 面板顶部可关闭提示条；回归测试 `a_notice_effect_becomes_a_visible_notice` |
| 9 | P2 | 损坏账本退回 PID 临时空账本：查询 `Ok(empty)`、新记录写进无引用临时文件 | ✅ **已修**（`1fd286c`）：新增 `UnavailableUsageService`（查询恒 `Unavailable`）；损坏时**不接线**生产者、原文件不动；回归测试 store+desktop 各一 |

**修订后的整体判定不变**：pass 7 · partial 11；#1/#7/#8/#9 四项真实缺陷已闭合，硬缺口仍为 **A17**（生产端）与 **A18**（签名）；#2/#3/#4/#5/#6 作为后续计划清单登记。
