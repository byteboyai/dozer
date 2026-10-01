# 代码健康度面板内容侧迁移到 WebView（含架构地图）

**状态：已批准（brainstorming 会话，2026-10-01，设计三节均获用户确认）**

**关联 spec：**

- `2026-09-20-code-health-panel-design.md`、`2026-09-20-code-health-panel-ui-optimization-design.md`、`2026-09-21-code-health-evolution-design.md`：现有四个分类页的内容与语义，本次 1:1 保留信息结构。
- `2026-09-21-code-health-architecture-map-design.md`：架构数据层（已落地）与架构页产品语义继续有效；**其「布局与渲染」一节（纯函数布局 + iced Canvas）被本文取代**。
- `2026-09-25-usage-panel-preact-migration-design.md`：本次的直接先例（长驻单槽 webview + Rust 推送指令）。
- `2026-09-28-tabular-webview-migration-design.md`：「直接切换、不留两套并存」的先例。

## 背景与动机

代码健康度面板（`extensions/codehealth/`）是左右分栏：右侧 `list_pane` 为分类导航（总览/结构/UI 一致性/扫描范围），左侧 `content_pane` 为内容，全部手写 iced（`view.rs` 783 行、`view_model.rs` 448 行）。三个诉求同时成立：

1. **观感**：手绘 iced 做阴影、hover、过渡、精致图表成本高，webview 天然具备。
2. **架构图表达力**：`dozer-codehealth` 已产出 crate/module 依赖图、循环/枢纽/越界分析、架构差异与影响范围，但 UI 尚未实现（`CodeHealthCategory` 目前只有四类）。原 spec 计划用 iced Canvas 手写缩放/平移/展开折叠，成本高。
3. **技术栈统一**：用量面板、review-trace 已走 Preact + esbuild 离线打包路线。

`view_model.rs` 已把数据算成「渲染层直接消费」的形状，迁移前提良好。

## 目标 / 非目标

**目标：**

1. 把内容侧**整体**（含扫描头部）换成 Preact + esbuild 离线打包的 webview，新建 `crates/dozer-app/web/codehealth-content/`，照抄 `web/usage-content`/`web/review-trace` 构建模式（iife、无 sourcemap、`jsx: 'automatic'` + `jsxImportSource: 'preact'`、无 CDN、运行时无 Node）。
2. 现有四个分类页**信息结构 1:1 保留**（不增不减），渲染质量升级：卡片阴影/圆角、hover 与展开过渡、严重度/变化徽标与色条、排行条绘制。
3. 新增第 5 个分类「架构」：Cytoscape.js + dagre（或 elk）渲染 crate/module 依赖图，支持缩放、平移、选择、模块展开/折叠、「仅看风险」、适配窗口/重置视图、图与风险列表互相定位、影响范围标注。
4. 右侧分类导航 `list_pane` 继续留在原生 iced，新增「架构」按钮。
5. 轻量 Rust↔webview 协议，复用 `window.__dozer.dispatch(JSON)` 注入约定与 `preview::webview_protocol::dispatch_script`，不复用多 tab 编辑器 envelope。
6. 前端只做渲染：聚合、排序、差异、风险分析全部在 Rust。
7. 直接切换：迁完即删旧 `view.rs` 的内容渲染，不留两套并存。

**非目标：**

- 不加任何编辑入口，只读（核心原则：预览优先于编辑）。
- 不改扫描逻辑、报告 schema、热点排序、`dozer-codehealth` 数据模型。
- 不做函数调用图（沿用架构 spec 非目标）。
- 不为非 Rust 语言做语义依赖图。
- 不保留 iced 内容渲染作为 webview 失败的回退。
- 主题固定 ByteBoy2077，不做 `SetTheme`（预留，首版不实现）。
- 「统计/扫描中」动画不进 webview（见下）。

## 分工与边界

| 部分 | 归属 |
|---|---|
| 右侧分类导航 `list_pane`（含新「架构」按钮） | 原生 iced；点击仍发 `Message` 改当前分类，同时往 webview 推一条 `SetView` |
| 内容侧（含扫描头部：扫描按钮、基准时间） | webview |
| 扫描中 | webview 内展示（状态条「扫描中…」+ 旧结果时间提示，保留旧内容；尚无报告时只显示「扫描中…」），不卸载 webview——现状 `view.rs::content_pane` 本来就是文字提示而非 `math_curve` 动画 |
| 瞬时失败（如跳转文件失败） | Toast（`App::push_toast`），不在内容区自画 |
| `daemon_unavailable` | 顶栏徽标，不在内容区自画 |

## 协议

新增 `CodeHealthCommand` / `CodeHealthEvent`，固定单槽、无 tab/document 字段。

**Rust → JS**

| 命令 | 用途 |
|---|---|
| `SetView { revision, category, payload }` | 切换分类或数据更新；`payload` 为 `view_model.rs` 输出的 `Serialize` 结构，按 `category` 取 variant |
| `SetScanState { state }` | `Idle` / `Scanning` / `Failed { reason }`，仅驱动头部状态 |
| `FocusFinding { id }` / `FocusNode { id }` | Rust 主动定位，前端滚动并高亮 |

**JS → Rust**

| 事件 | 用途 |
|---|---|
| `Ready` | webview 加载完成；Rust 收到后才推第一条 `SetView` |
| `OpenFile { path, line }` | 沿用现有「点发现项跳转文件」路径 |
| `ScanRequested` | 点页头「扫描」 |
| `AnalyzeFinding { id }` | 「交给 Agent 分析」（现有功能，原 spec 漏列） |
| `Failed { reason }` | 渲染异常；Rust 回落原生占位页 |

**revision**：每条 `SetView` 带单调递增 `revision`，前端丢弃小于当前值的指令，防止快速切换分类时旧响应覆盖新状态。

**数据**：`view_model.rs` 的输出类型（`ChangeSummary`/`ChangeCard`/`HotspotRow` 等）与架构数据可见子图直接加 `Serialize` 复用，不另设计一套 JSON。架构图布局（dagre/elk）属于渲染的一部分，在 JS 侧计算；风险分析（循环、枢纽、越界、影响范围）仍在 Rust。

**纯前端视图状态不回传 Rust**：各分类的筛选、展开、选中、架构图缩放平移与模块展开/折叠；切换分类时按分类各自保留，切回不丢。

## WebView 集成

仿 Usage/Git Log 先例：

- `preview_desired` 新增 `PanelKind::CodeHealth` 分支（目前 `webview_geometry.rs:128`、`:260` 为 `(0,0,0,0)` 占位）。
- 固定单槽，新增 `CODEHEALTH_CONTENT_ID_OFFSET`（延续现有递增序列，避免与 `USAGE_CONTENT_ID_OFFSET = 4_000_000` 冲突，具体取值在计划阶段确定）。
- 矩形由新增 `webview_geometry::codehealth_content_pane_bounds_for` 计算，复用分栏尺寸、收起状态与「内容在前」的列顺序。
- 待推送指令走队列（仿 `take_usage_content_script`），在 `window_events.rs::apply_pending_editor_commands` 同一节奏里注入，不新开每帧轮询路径。
- webview 只覆盖内容列，右侧导航仍是 iced，不被盖住。面板内现无 iced 浮层要避让；日后若加弹层，默认走独立原生子窗口。

## 前端结构

- 单入口单 bundle，按 `category` 切换组件：`Overview`、`Structure`、`UiConsistency`、`ScanScope`、`Architecture`；共享组件 `ScanHeader`、`FindingRow`、`ChangeCard`、`StatusCard`。
- 字体用系统默认字体（非代码/终端场景），中文渲染由浏览器负责。颜色用 ByteBoy2077 令牌，金色仍只给「甲方动作」。
- 变化与严重度不只靠颜色，文字标签保留（沿用无障碍要求）。

### 四个现有分类页

逐页对照现有 `overview_content`、`structure_content`、`ui_consistency_content`、`scan_scope_content`，内容不增不减；`filter_buttons` 筛选、`priority_list` 排行、`finding_row` 原因文本原样保留，筛选状态前端本地维护。结构复杂度「本轮新增/全部」筛选状态移到前端本地，Rust 侧 `StructureFilter` 随之删除。验收标准：与已批准 Figma（「S-CodeHealth 代码健康 · 本次变化与风险热点」）信息结构逐项一致，渲染质量不低于旧实现。

### 架构页（新）

- **页头**：「架构地图」、扫描基准、图层切换（crate/module）、「仅看风险」开关、适配窗口/重置视图。
- **画布**：Cytoscape.js。crate 层为 crate 节点；module 层为模块节点，以复合节点承载展开/折叠。布局 dagre（从左到右分层），展开后只重排受影响子图；若 dagre 对复合节点不稳，备选 elk（计划阶段验证）。
- **风险高亮**：循环、越界边、枢纽用颜色 + 文字徽标；「仅看风险」只留风险相关子图。
- **交互**：滚轮缩放、拖拽平移、点击选中并在侧栏显示详情（入边/出边/所属发现项）。
- **风险列表**：新增循环、越界、枢纽；点列表项 → 画布选中并居中；点画布节点 → 列表高亮。
- **影响范围**：结合 Git 改动文件标出本轮直接受影响节点（数据来自 `architecture_diff` 与影响范围分析）。
- **大图降噪**：Rust 侧按原架构 spec 的上限先裁出可见子图或聚合图，前端不遍历完整图；截断时显示「分析不完整」，不显示为健康。

## 错误与降级

- **扫描中**：webview 内展示（状态条「扫描中…」+ 旧结果时间提示，保留旧内容；尚无报告时只显示「扫描中…」），不卸载 webview。
- **webview 加载/渲染失败**：收到 `Failed` 或超时未收到 `Ready`，回落原生占位页（失败原因 + 重试），不保留旧 iced 内容渲染。
- **旧报告（schema < 2）**：沿用 `legacy_report_note`；缺 `architecture` 的旧报告在架构页显示「请重新扫描」，不得把空图解释为零风险。
- **架构专属**：Cargo metadata 失败时 module 图继续可用，原因记录在扫描范围页；非 Cargo Rust 项目显示 module 图并标注「未发现 Cargo workspace」；无 Rust 语义分析显示「暂无可生成架构图的代码」；图超上限显示聚合图与截断说明。
- **日志**：来源名为面板名 `code_health`，用 `dozer_core::log_*!(LOG, ...)`，不写裸 `tracing`/`eprintln!`；面板第一次写日志时才声明 `LOG`。

## 测试与验收

**自动测试**

- `view_model` 输出的 `Serialize` 形状快照（防字段被无意改名）。
- 协议编解码与 `revision` 丢弃逻辑。
- `codehealth_content_pane_bounds_for` 几何：收起/展开、分栏拖拽、「内容在前」列顺序。
- 前端对每个 category 的 payload fixture 做渲染冒烟，至少覆盖：空态、首次扫描、有变化、旧报告、截断。
- 架构图稳定性：同一份图连续两次渲染节点坐标逐点相同。

**人工验收**

1. 四类页与已批准 Figma 信息结构逐项对照。
2. fixture 中制造 A → B → A，重新扫描后显示新增循环，图与列表互相定位。
3. 展开 `dozer-app::extensions`，再次扫描后布局不随机跳动。
4. 删除或破坏 `Cargo.toml` 后 module 图仍可用并明确显示 Cargo 分析失败。
5. 快速连点分类无白屏闪烁。

## 发布切片

1. **骨架 + 四类页**：webview 宿主、协议、几何、四个现有分类迁完，删除旧 `view.rs` 内容渲染。可独立验收。
2. **架构页**：新增第 5 个分类，接 Cytoscape，图与列表联动，影响范围标注。

## 实现计划阶段需验证的风险

1. dagre 对复合节点展开/折叠重排是否稳定、是否确定性；不稳则评估 elk。
2. Cytoscape + 布局插件的 bundle 体积与 webview 首屏时间。
3. 重新扫描后同一份图布局是否完全一致。
4. `CODEHEALTH_CONTENT_ID_OFFSET` 取值与现有槽位序列的衔接。

## 文档同步

- 在 `2026-09-21-code-health-architecture-map-design.md` 的「布局与渲染」一节顶部加指向本文的取代说明（不改其数据模型、产品语义部分）。
- 更新 `CLAUDE.md` 若新增了需长期遵守的裁决（计划阶段判断，不在 spec 阶段预先改）。
