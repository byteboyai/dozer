# S0 Task 01 验收台账与兼容桥清单

> 计划 Task 01 第四项：为规格 §9 的 A1–A18 各建**未完成条目**，记录验证层级、平台和所需夹具；建立兼容桥清单，删除条件必须可测。
> 规格：[完整接入设计](../../superpowers/specs/2026-10-11-digger-bytehost-full-integration-design.md) §9/§10。
> 验证层级：**L1** 纯状态/契约测试 · **L2** 进程与 Gateway 集成测试 · **L3** 真实 WKWebView/窗口行为 · **L4** 人工（视觉/真实鼠标焦点/浮层/IME）。禁止把 handler 直接调用当作真实点击验证。
> 所有条目当前状态为 **未完成（Unmet）**。

## 1. A1–A18 台账

| 编号 | 场景 | 验证层级 | 平台 | 所需夹具 | 状态 |
|---|---|---|---|---|---|
| A1 | 独立构建 Digger/Dozer：固定依赖可解析、无兄弟目录 patch、共享 Host 无产品依赖 | L1+L2 | macOS | Digger 正式 workspace（尚不存在）；bytehost tag；`scripts/check-deps.sh` 分组门禁 | Unmet |
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

## 2. 兼容桥清单（临时适配器）

删除条件均**可测**（有明确断言或门禁指标）。目标收口 Task：21（通用兼容桥清零）。

| 编号 | 桥 | 位置 | 存在理由 | 删除条件（可测） | 目标 Task |
|---|---|---|---|---|---|
| C1 | `dozer_core::protocol` 兼容出口（线上类型） | `crates/dozer-core/src/protocol` | 面板代码历史上从 `dozer_core::protocol` 取类型 | 无面板/模块再 `use dozer_core::protocol`（`R-PROTOCOL` 门禁计数归零） | 21 |
| C2 | `Client` 直连面板 | `crates/dozer-app/src/extensions/**` | 渐进迁移到 `Services` 接缝 | `extensions/**` 不再出现 `dozer_client::Client`（`R-CLIENT` 门禁归零） | 06 / 21 |
| C3 | `service_fakes.rs` 先构造离线 `Client` 再覆盖 | `crates/dozer-app/src/service_fakes.rs` | 旧 fake 构造路径 | fake 不再构造 `Client`（构造点断言） | 06 |
| C4 | `services_for`/`ClientDaemon` 在产品组合根 | `crates/dozer-app/src/product.rs` | 服务构造尚未移入产品适配层 | 移入 `host_adapters/services.rs` 且组合根不再直接构造 | 06 |
| C5 | 预览窗格按 `PanelKind::Project` 手写选择 | `workspace/state.rs`（`R-PANE-PICK` 2）、`app/update.rs`（1）、`platform/window_events.rs`（3） | 未走 `Workspace::preview_pane[_mut]` | `R-PANE-PICK` 三处计数归零 | 21 |
| C6 | `app/` 下 `run_<面板>_effects` 执行器 | `app/app.rs`（`R-HOST-EXECUTORS` 实测 1） | host 替面板做事，未外移 | `R-HOST-EXECUTORS` 计数归零（≤ 基线 0） | 06 / 08 / 09 / 21 |
| C7 | `webview_geometry.rs` 的"文件预览"命名遗留 | `crates/dozer-app/src/webview_geometry.rs` | 几何与文件预览业务命名混同 | 几何函数迁入 Host 通用名；无"preview"命名残留 | 08 / 09 |
| C8 | `request_tracker.rs` 原型迁移后删除 | `crates/dozer-app/src/request_tracker.rs` | 是 core 请求身份的移植来源 | 使用 core 请求身份且 delete 该文件后编译通过 | 05 / 21 |
| C9 | `HoverId`/`PanelKind` 作为宿主类型 | `panel_host.rs` / `app/state.rs` | 词汇尚未命名空间化/注册制 | `HoverSlot`/`PanelKind` 变体表纳入注册制后门禁计数只降 | 09 |

**门禁约束：** 上述计数类删除条件与 `scripts/audit/panel-boundary.baseline.json` 及 `R-PROTOCOL`/`R-CLIENT` 门禁一一对应；删除桥时只允许基线**下降**，禁止上调阈值掩盖。
