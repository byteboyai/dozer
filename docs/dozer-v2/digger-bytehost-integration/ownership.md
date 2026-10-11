# S0 Task 01 归属清单（Dozer 宿主路径）

> 计划 Task 01 第三项：盘点 Dozer `app/`、`workspace/`、`chrome/`、`platform/`、`panel_host.rs`、`panel_registry.rs`、`app_webview.rs`、`webview_geometry.rs`、`runtime.rs`。
> 每一项标 **Host 机制**（可归 bytehost 通用宿主）/ **产品语义**（Dozer 业务，留在产品组合根）/ **临时适配器**（渐进迁移桥，有删除条件）。
> 第 4 列指向落实该归属的后续 Task（见计划 §1/§3）。**不存在未归属的宿主路径。**

## 1. 判定口径

- **Host 机制**：与面板业务无关的窗口/布局/事件/webview 生命周期骨架，可被 Dozer 与 Digger 同源复用（目标 crate：`bytehost-core` / `bytehost-desktop` / `bytehost-panel` / `bytehost-webview`）。
- **产品语义**：面板清单、默认布局、各面板业务状态与消息、服务构造——由产品组合根持有（Dozer 侧 `product.rs` 等）。
- **临时适配器**：为渐进迁移而存在的桥（旧类型包装、`Client` 直连、旧几何函数命名遗留），须登记删除条件（见 [acceptance.md](./acceptance.md) 兼容桥清单）与目标 Task。

## 2. 核心宿主路径

| 路径 | 现状职责 | 归属 | 后续 Task |
|---|---|---|---|
| `crates/dozer-app/src/panel_host.rs` (767) | 面板对宿主的**只读视图契约** trait（悬停进度、列折叠态、窗口/光标位置）；`HoverId`/`PanelKind` 仍是宿主类型 | Host 机制（接口）+ 产品语义（`HoverId` 词汇命名空间化属宿主） | 06 / 07 / 09；`HoverSlot` 变体数归 09 |
| `crates/dozer-app/src/panel_registry.rs` (354) | 面板注册清单：清单由产品组合根给出；`PanelKind` 仍是封闭枚举，每面板状态字段与 `Message` 包装不动；切入钩子分发仍是 host `match` | Host 机制（清单/校验）+ 产品语义（清单内容） | 04（core 注册）/ 08 / 09 |
| `crates/dozer-app/src/product.rs` | 产品组合根：`dozer_catalog()` 面板清单、标题/图标、默认栏位与顺序、旧版布局迁移 | 产品语义 | 06（接 Services）/ 11（Digger 另写一份） |
| `crates/dozer-app/src/app_webview.rs` (192) | 应用面板 webview 纯策略入口：origin 放行（唯一真相在 `bytehost-webview`）、池 id 段、每应用存储标识、期望清单构造 | Host 机制（策略在 bytehost-webview，本文件为适配） | 16 / 17 / 18 |
| `crates/dozer-app/src/runtime.rs` (922) | 运行时胶水：daemon 启动序列、operation 遍历、webview 池同步；`build_app_webview`（不装 `dozer://`、IPC 白名单） | 混合：`build_app_webview` = Host 机制；daemon 拉起 = 产品语义 | 07 / 16 |
| `crates/dozer-app/src/webview_geometry.rs` (2148) | 三个挂原生 wry webview 的面板共用几何（矩形、命中判断）；文件预览业务命名遗留 | Host 机制（几何）+ 命名遗留为**临时适配器** | 08 / 09 |
| `crates/dozer-app/src/app/state.rs` (536) | 外壳态类型：`PanelKind`、顶栏按钮、悬停动画状态机、页面/放大/聚焦、项目页签槽、几何快照 `ShellState` | 混合：外壳态 = Host 机制；`PanelKind`/业务槽 = 产品语义 | 04 / 05 / 06 / 09 |
| `crates/dozer-app/src/app/update.rs` (6717) | 顶层消息分发；含 `R-HOST-PANEL-ARMS`（95）与 `R-HOST-EXECUTORS`（实测 1，基线 0） | 产品语义（当前是 host 替面板做事，须外移） | 06 / 08 / 09 |
| `crates/dozer-app/src/app/app.rs` (6585) | `App` 状态、操作循环、预览/终端/rail 内核逻辑；`R-HOST-EXECUTORS` 回退落点 | 混合：内核 = 产品语义；窗口/事件 = Host 机制 | 06 / 07 / 08 / 09 |
| `crates/dozer-app/src/app/view.rs` (2175) | 根视图拼装（rail + 面板 + chrome） | Host 机制（布局拼装）+ 产品语义（面板内容调用） | 07 / 08 |
| `crates/dozer-app/src/app/layout.rs` (1514) | 栏位/尺寸布局、显隐、放大态几何 | Host 机制 | 04（core 布局 DTO）/ 07 / 10 |
| `crates/dozer-app/src/app/app_slots.rs` (80) | `AppSlot` 进程内 id 表（`Copy+Hash` 复用现有拖拽/悬停），应用 id 形状检查 | 产品语义（与 `bytehost-apps::AppId` 同口径但不依赖） | 04 / 16 |
| `crates/dozer-app/src/app/message.rs` (811) | 顶层 `Message` 枚举与包装 | 产品语义 | 06 / 09 |
| `crates/dozer-app/src/request_tracker.rs` (106) | UI 请求身份：交付时校验、每面板实例独立 tracker | Host 机制（原型）→ 移植 core | 05 |
| `crates/dozer-app/src/request_tracker.rs`（同上） | 现有实现是 core 请求原型的输入 | 临时适配器（迁移后删除） | 05 / 21 |

## 3. `workspace/`、`chrome/`、`platform/`

| 路径 | 现状职责 | 归属 | 后续 Task |
|---|---|---|---|
| `crates/dozer-app/src/workspace/state.rs` (3333) | 工作区态；含预览窗格按 `PanelKind::Project` 手写选择（`R-PANE-PICK` 2） | 混合：预览选择属**临时适配器**（应走 `Workspace::preview_pane[_mut]`）；其余产品语义 | 09 / 21 |
| `crates/dozer-app/src/workspace/view.rs` (2268) | 工作区视图拼装 | 产品语义 | 09 |
| `crates/dozer-app/src/workspace/{hook,mod,tests}.rs` | 工作区钩子/模块/测试 | 混合 | 09 |
| `crates/dozer-app/src/chrome/rail.rs` | 图标栏、`migrate_legacy_rail` 迁移、面板选择 | 混合：栏位/迁移 = Host 机制 | 04 / 07 / 09 |
| `crates/dozer-app/src/chrome/{homespace,menu,native_menu,tab_widget,topbar,mod}.rs` | 首页/菜单/原生菜单/页签/顶栏 | Host 机制（chrome）+ 产品语义（菜单项） | 07 / 10 |
| `crates/dozer-app/src/platform/overlay_window.rs` | 独立原生子窗口浮层共享机制（search/file_history 已迁） | Host 机制 | 07 / 10 |
| `crates/dozer-app/src/platform/overlay_{focus,gpu}.rs` | 浮层焦点/GPU 叠加 | Host 机制 | 07 / 10 |
| `crates/dozer-app/src/platform/*_overlay.rs`（confirm/database_drivers/database_source/edit_history/file_history/files_move/project_create/project_delete/project_scaffold/search/settings/ssh_host/todo_detail/toast） | 各业务浮层视图 | 产品语义（内容）+ Host 机制（独立窗口壳） | 07 / 10 |
| `crates/dozer-app/src/platform/window.rs` | 窗口创建/属性 | Host 机制 | 07 |
| `crates/dozer-app/src/platform/window_events.rs` | 窗口事件；含 `R-PANE-PICK` 3 | 混合：事件 = Host 机制；`R-PANE-PICK` = 临时适配器 | 09 / 21 |
| `crates/dozer-app/src/platform/{file_drag,picker,mod}.rs` | 拖拽/文件选择器 | Host 机制 | 07 |

## 4. 统计口径与数值（后续 Task 保持同口径）

命令：

```bash
rg -c "PanelKind::" crates/dozer-app/src/{app/update.rs,app/app.rs,panel_host.rs,panel_registry.rs,workspace/state.rs,platform/window_events.rs}
python3 scripts/audit/check_panel_boundary.py
```

`rg -c "PanelKind::"` 实测（2026-10-11，HEAD `a4cbf340`）：

| 文件 | `PanelKind::` 出现次数 |
|---|---|
| `app/app.rs` | 120 |
| `app/update.rs` | 57 |
| `panel_registry.rs` | 26 |
| `workspace/state.rs` | 13 |
| `panel_host.rs` | 8 |
| `platform/window_events.rs` | 46 |

门禁口径（`panel-boundary.baseline.json`，只降不升）：

| 规则 | 文件 | 基线 | 实测 |
|---|---|---|---|
| R-HOVERID-VARIANTS | `app/state.rs` | 12 | 12 |
| R-HOST-PANEL-ARMS | `app/update.rs` | 95 | 95 |
| R-PANE-PICK | `app/update.rs` | 1 | 1 |
| R-HOVERSLOT-VARIANTS | `panel_host.rs` | 9 | 9 |
| R-PANE-PICK | `platform/window_events.rs` | 3 | 3 |
| R-PANE-PICK | `workspace/state.rs` | 2 | 2 |
| R-HOST-EXECUTORS | `app/app.rs` | 0 | **1（回退）** |

- `PanelKind` 变体数：13（12 内置 + `App(AppSlot)`）；`BUILTIN_NAMES` 覆盖 12 内置。
- `R-HOST-EXECUTORS` 回退是**唯一**边界门禁未通过项，须在 06/08/09 外移执行器时归还；不得改基线掩盖。

## 5. 服务构造与面板边界（并入 06）

| 路径 | 现状职责 | 归属 | 后续 Task |
|---|---|---|---|
| `crates/dozer-app/src/service_fakes.rs` | fake 服务（先构造离线 `Client` 再覆盖） | 临时适配器 | 06 |
| `crates/dozer-app/src/product.rs` 中的 `services_for`/`ClientDaemon` 构造 | 面板取数经 `Services` 接缝 | 产品语义 → 产品适配层 `host_adapters/services.rs` | 06 |
| `crates/dozer-services/src/lib.rs` | 面板服务 trait 接缝（仅依赖 dozer-model） | Host 机制（接缝）| 06 |

**结论：**上表现存路径均已归属为 Host 机制、产品语义或临时适配器三者之一，无未归属宿主路径。
