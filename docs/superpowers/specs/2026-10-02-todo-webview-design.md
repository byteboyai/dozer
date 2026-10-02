# Todo 面板内容区迁移到 WebView（含左栏分类拖动移动）

**状态：设计已逐节获用户确认（brainstorming 会话，2026-10-02），待审阅书面 spec**

**关联 spec：**

- `2026-10-01-code-health-webview-design.md`（已合并 main）：直接先例——固定单槽 webview、Rust 推送指令、`window.__dozer.dispatch(JSON)` 注入约定、`revision` 丢弃、协议文件 `protocol.rs` 的形态。
- `2026-09-25-usage-panel-preact-migration-design.md`：长驻单槽 webview 的更早先例，构建模式与资源路由的来源。
- `2026-09-28-tabular-webview-migration-design.md`：「直接切换、不留两套并存」的先例。
- Git Log 的 diff 区（`webview_geometry.rs::git_log_diff_pane_bounds_for`）：「原生头部 / tab 行 + 其下 WebView」的几何先例。
- Todo 现有行为的来源：`extensions/todo/{view,update,state}.rs`；任务与分类的权威数据在 dozerd 的 SQLite（`todos`、`todo_categories` 表），本次不改。

## 背景与动机

Todo 面板（`extensions/todo/`，`view.rs` 1977 行）左栏为分类树，右栏为任务列表。右栏全部手写 iced：三段可拖拽排序的任务卡片、搜索条与状态筛选、底部新增框、内联编辑，以及日历、派发、状态、状态筛选、分类选择五个弹层，外加多套只为原生输入服务的焦点接线（`CaptureAddFocus` 等）。拖拽、弹层定位、输入焦点这类交互在 iced 里成本高，在 web 里天然具备。用量面板、review-trace、Code Health 已走 Preact + esbuild 离线打包的 webview 路线，Todo 迁入后技术栈统一。

动机（观感与交互成本、技术栈统一）由本次 brainstorming 中的先例对照与会话推断得出，用户未逐条否认。

## 目标 / 非目标

**目标：**

1. 把右栏中**原生 tab 行以下**的内容换成 Preact + esbuild 离线打包的 webview，新建 `crates/dozer-app/web/todo-content/`，构建模式照抄 `web/codehealth-content`（iife、无 sourcemap、`jsx: 'automatic'` + `jsxImportSource: 'preact'`、无 CDN、运行时无 Node）。
2. 现有信息结构与行为 **1:1 保留**：三段任务卡片（进行中 / 搁置 / 已完成）、搜索与状态筛选、新增、切换完成、内联编辑文字、拖拽排序、改状态、改计划日期、指派 agent、改分类、打开详情。
3. 五个弹层（状态菜单、派发菜单、状态筛选下拉、日历、分类选择器）全部做成 **WebView 内的 DOM 弹层**，在所有平台一致。
4. 左栏分类树新增**拖动移动**交互，取代「移动到…」选择器。
5. 轻量 Rust↔webview 协议，复用 `window.__dozer.dispatch(JSON)` 注入约定与 `preview::webview_protocol::dispatch_script`。
6. 前端只做渲染与纯视图状态；落库、rank 计算、派生状态、分类过滤全部在 Rust。
7. 直接切换：迁完即删旧 iced 列表渲染与相关接线，不留两套并存。

**非目标：**

- 不新增 Todo 功能，也不加新的编辑入口（核心原则：预览优先于编辑）。看板视图仍是原生占位。
- 不改 dozerd 的 Todo / 分类数据模型、协议、MCP 工具（`add_todo` 等）。
- 不改左栏的分类树渲染、「新建子分类 / 同级 / 重命名 / 删除 / 上移 / 下移」、底部「清空列表」栏。
- 不改「Todo 详情」窗口（`platform/todo_detail_overlay.rs`，独立原生子窗口），WebView 只发 `OpenDetail`。
- 不支持把任务卡片拖到左栏分类上（卡片在 WebView、左栏是原生控件，二者之间无法直接拖放）；任务换分类用卡片上的分类 chip。
- 不做兄弟节点之间的拖拽排序（继续用右键菜单的「上移 / 下移」）。
- 不保留 iced 列表作为 webview 失败的回退。
- 主题固定 ByteBoy2077，不做 `SetTheme`。
- 不修改 `cargo test --workspace` 下 dozerd `summary_pipeline` 的既有失败（与本次无关）。

## 分工与边界

| 部分 | 归属 |
|---|---|
| 左栏：Todo 头、分类树（含右键菜单、重命名、新建 / 删除 / 上移 / 下移）、底部「清空列表」栏及其确认框 | 原生 iced；新增分类拖动移动 |
| 右栏顶部 tab 行（「列表视图 / 看板视图」切换 + 「收起列表列」按钮）及其下的分割线 | 原生 iced，不变 |
| 看板视图 | 原生占位（`kanban_placeholder`），此时 webview 不挂载 |
| 搜索条、状态筛选、三段任务卡片、底部新增框 | webview |
| 状态菜单、派发菜单、状态筛选下拉、日历、分类选择器 | webview 内 DOM 弹层 |
| 「Todo 详情」窗口 | 原生独立子窗口，不变 |
| 瞬时失败（落库失败等） | Toast（`App::push_toast`），不在内容区自画 |
| `daemon_unavailable` | 顶栏徽标，不在内容区自画 |

## 协议

新增 `extensions/todo/protocol.rs`，定义 `TodoCommand`（Rust → JS）与 `TodoEvent`（JS → Rust），固定单槽、无 tab / document 字段。形态仿 `extensions/codehealth/protocol.rs`，含 `TODO_PROTOCOL_VERSION: u32 = 1` 与 `READY_TIMEOUT`（10 秒）。

**身份用 `id: i64`，不用位置下标。** 现有 `Message` 全是 `usize` 下标（`Toggle(idx)`、`StatusPick(idx, …)`），WebView 里点击与 Rust 处理之间列表可能已变（别的 agent 经 MCP 新增了任务），按下标会改错行。新协议一律带任务 `id`；Rust 收到后自行映射，`id` 已不存在则空操作。

### Rust → JS

| 命令 | 内容 |
|---|---|
| `SetView { revision, project_id, view }` | 整表推送，见下方 `view` 形状 |

`view` 形状（`Serialize`）：

- `category_key: String`：当前分类过滤的稳定标识（`all` / `uncategorized` / `node:<id>`）。前端据此在**分类切换时重置搜索**（沿用现有 `clear_search` 语义）。
- `items: [TodoCard]`：**已按左栏选中的分类过滤**，顺序为 Rust 排好的显示顺序。每项：`id`、`text`、`state`（`pending` / `in_progress` / `suspended` / `done`，**由 Rust 派生**，因为 `in_progress` 需要判断派发的 session 是否存活）、`segment`（`active` / `paused` / `done`，决定所属段与能否拖拽）、`plan_date`（`MM-DD` 字符串或 `null`）、`completed_label`（已完成时的完成日期，`MM-DD` 或 `-`）、`category_id`、`category_name`（无分类为 `null`，前端显示「未分类」）、`assigned_agent`、`has_dispatch`（是否有派发 session，用于决定「详情」「指派」按钮）。
- `categories: [{ id, name, parent_id, depth }]`：全展开拍平，供分类选择器按 `depth` 缩进（沿用 `visible_category_rows(全展开)`）。
- `agents: [{ kind, label }]`：可指派的 agent 列表（现为 Claude / Codebuddy / Opencode / V8agent）。图标由前端按 `kind` 取内置 SVG。
- `add_height_px: f32`：新增框当前高度（Rust 持有，含最小 / 最大钳制）。
- `today: "MM-DD"`：日历默认月份与「今天」高亮用，由 Rust 给，前端不读系统时钟，保证与 Rust 的 `today_ymd` 一致。

派生状态、分段、过滤、排序均在 Rust 算好；**前端不推导状态，不按分类过滤**。

**`#NNN` 编号由前端算**：它是「当前可见卡片的显示序号」（现有 `shown` 计数，跨三段连续递增，随搜索 / 状态筛选变化），所以不放进 `view`。

### JS → Rust

| 事件 | 语义 |
|---|---|
| `Ready` | webview 加载完成；Rust 收到后才推第一条 `SetView` |
| `Failed { reason }` | 渲染异常；Rust 回落原生占位页 |
| `Add { text }` | 新增任务（前端已 `trim`，空串不发） |
| `Toggle { id }` | 切换完成；现有逻辑含 `completed_at_for_toggle` 与乐观更新，落库用 `toggle_todo` |
| `EditText { id, text }` | 内联编辑提交；空文本或与原文相同不发；落库用 `edit_todo_text` |
| `Reorder { id, after_id }` | 拖拽排序，见下 |
| `SetStatus { id, state }` | `state` 取 `pending` / `suspended` / `done`，分别对应 `TodoStoredStatus::{Todo, Suspended, Done}`；「进行中」见下 |
| `SetPlanDate { id, date }` | 日历选日，`date` 为 `MM-DD`，落库用 `set_todo_plan_date`；现有日历无「清除日期」，保持 |
| `AssignAgent { id, agent }` | 指派（只记录，不立即执行，沿用 2026-09-02 起「指派与执行解耦」的口径），走现有 `App::todo_assign_agent` |
| `SetCategory { id, category_id }` | 卡片分类 chip 选择结果，`category_id: null` = 未分类，落库用 `set_todo_category` |
| `OpenDetail { id }` | 打开原生详情窗口，走现有 `App::todo_detail_open` |
| `AddHeight { px }` | 新增框拖拽结束时回报一次高度，Rust 用现有 `set_add_input_height` 钳制并保存 |

**「进行中」的状态语义**（沿用现有 `StatusPick`）：在状态菜单里选「进行中」时，若该任务当前是**搁置**，前端发 `SetStatus { id, state: "pending" }`（恢复为待办）；否则前端**不发 `SetStatus`**，而是在前端打开派发弹层，用户选 agent 后发 `AssignAgent`。「进行中」状态本身只由「已指派且 session 存活」派生，不可直接写入。

**`Reorder { id, after_id }`**：`after_id` 是放置位置**上方**那张卡片的 `id`，`null` 表示放到进行中段最前，与 `Client::reorder_todo(id, after_id)` 的语义完全一致（dozerd 的 `reorder`：把 `id` 挪到 `after_id` 之后，只在待办子集内生效，`after_id` 非法时落到末尾）。前端据放置指示线的位置算出前驱卡片。**说明**：现有 iced 实现把「目标卡片」当 `after_id` 传，与指示线语义存在偏差；本次按指示线所见即所得的前驱语义实现，属有意修正。只有「进行中」段可拖，「搁置」「已完成」段不可拖，与现状一致。

### revision 与一致性

- 每条 `SetView` 带单调递增 `revision`，前端丢弃小于当前值的指令。
- **成功不回执**：操作成功后 Rust 经 `Client` 落库，再推新的 `SetView` 即为确认；失败走 Toast，并推一条权威 `SetView` 把前端的乐观状态还原。
- 乐观更新沿用现有 Rust 侧做法（`Toggle` / 新增）：新增插入临时项、切换完成，均由 Rust 本地先改再落库；前端只对拖拽排序做本地预览（松手发 `Reorder`，落库后以权威 `SetView` 覆盖）。失败走 Toast，并推一条权威 `SetView` 把前端的乐观状态还原。
- 新增后的高亮闪烁**保留 Rust 的 `Flash` 计时**（`start_flash` / `advance_flash` / `next_flash_wake` 不删），由 Rust 把 `Flash` 命中的任务 `id` 经 payload 的 `selected_id` 与 `scroll_nonce` 带给前端：前端据 `selected_id` 覆盖一次卡片选中高亮、据 `scroll_nonce` 变化滚回顶部。

### 纯前端视图状态（不回传 Rust，按 `project_id` 分别保留）

搜索词（草稿与生效值）、状态筛选、新增框草稿文字、内联编辑中的文字与目标卡片、各弹层开合与定位、日历当前月份、滚动位置、卡片选中高亮。切换项目标签再切回来这些不丢；Rust 切到另一个项目时对新活动项目重推 `SetView`，前端据 `project_id` 取回对应状态。**卡片选中高亮是前端本地状态，Rust 仅在新增后通过 `selected_id` 覆盖一次。** **例外**：分类切换（`category_key` 变化）清空搜索草稿与生效词，沿用现有语义。

## 前端行为（1:1 保留，逐项对照现有实现）

- **搜索**：草稿与生效值分离，**回车或点搜索按钮、或失焦时提交**（现有 `commit_search` 的触发点），不做逐字实时过滤；匹配规则为大小写不敏感的子串匹配（`filter_todos`），前端以纯函数重实现并带单测。
- **状态筛选**：下拉选「全部 / 待办 / 进行中 / 搁置 / 已完成」，只决定展示哪一段，不改变数据。
- **三段与分隔线**：进行中、「搁置」分隔线 + 搁置段、「已完成」分隔线 + 已完成段；无匹配项时显示「没有匹配的任务」。
- **任务卡片**：顶部行为编号 `#NNN`、分类 chip（点开分类选择器，无分类显示「未分类」）、日期徽章（点开日历；已完成显示完成日期，否则显示计划日期或 `-`）；主体为复选框（切换完成）+ 文字（点击进入内联编辑，已完成加删除线）；底部为状态按钮（点开状态菜单）、「详情」按钮（有指派或有派发 session 时出现）、「指派」按钮（`state == pending` 且无派发 session 时出现，点开派发弹层）。
- **内联编辑**：点文字进入编辑；**回车提交**，**失焦提交**（现有 `ContentEdit` 回车分支与 `commit_content_edit` 失焦分支），无改动或空文本直接丢弃；沿用现有语义，**输入法组合期间（`isComposing`）回车不提交**。
- **新增框**：**Enter 换行，⌘↵（Windows / Linux 为 Ctrl+↵）提交**，框内右侧的「↑」按钮提交；高度可拖拽调整，拖拽结束发 `AddHeight`。提交后清空草稿，输入法组合期间 ⌘↵ 不提交。
- **拖拽排序**：仅「进行中」段，按下卡片开始、显示放置指示线，拖动期间前端乐观重排；松手发 `Reorder`；`Esc` 或拖出窗口取消。
- **弹层**：同一时刻只开一个；点空白处或 `Esc` 关闭；弹层不超出 webview 矩形（在矩形内翻转定位）。分类选择器列出「未分类」与全展开的分类树（按 `depth` 缩进）。
- 字体用系统默认字体，中文渲染由浏览器负责；颜色用 ByteBoy2077 令牌，金色只给甲方动作；状态不只靠颜色，文字标签保留。
- 前端随全局缩放（Ctrl +/-）的处理沿用其它 content webview 的做法，计划阶段核实并补全。

## 左栏分类树：拖动移动

取代「移动到…」选择器。落盘接口不变：`Client::reparent_category(category_id, new_parent_id)`，`None` 表示顶层；dozerd 的 `reparent` 把节点追加到新父节点子级末尾，目标是自己或自己的子孙时拒绝（防成环）。

- **拖起**：按下分类节点行并移动越过阈值后进入拖拽，沿用文件树 `TreeDrag` 的两阶段（按下武装、越过阈值确认）与 `TREE_DRAG_CONFIRM_THRESHOLD_PX`（12px）的做法，避免「点一下」被误判为拖拽。点击选中分类、展开 / 折叠、右键菜单等现有行为不变。
- **放下 = 成为该节点的子分类。** 放到某个分类节点上，该节点成为新父节点，被拖分类追加在其子级末尾。
- **放到「全部」伪节点上 = 移到顶层。** 「未分类」伪节点不接受放置。
- **防成环**：拖拽期间，被拖节点自己及其子孙在 UI 上不可作为放置目标（不高亮、放下无效）；服务端的拒绝作为兜底，失败走 Toast。
- **视觉**：悬停在合法目标上高亮该行；被拖节点半透明。
- 拖拽是 iced 内的交互，各平台一致，不需要分支。
- **要删**：右键菜单里的「移动到…」项（macOS 的原生 `category_context_menu_items` 与非 mac 的 iced `category_context_menu_popup` 两处），以及 `category_picker` 整套：`CategoryPicker`、`CategoryPickerTarget`、`category_picker_popup`、`app/view.rs` 中对应分支、`Message::CategoryPickerClose` / `CategoryPickerSelect`、`todo::Message::CategoryReparentPickerOpen` / `CategoryPickerOpenForTodo`、`todo_category_picker_open`、Esc 路由里的 `category_picker_open`。

## WebView 集成

仿 Code Health / Usage / Git Log 先例：

- `webview_geometry.rs` 新增 `todo_content_pane_bounds_for`，仿 `codehealth_content_pane_bounds_for` 与 `git_log_diff_pane_bounds_for`：复用分栏尺寸、收起状态与「内容在前」的列顺序；**纵向起点要扣掉原生 tab 行与其分割线的高度**（新增与 tab 行同源的固定高度函数，如 `todo_top_row_h_px`，tab 行与几何共用，禁止各写一份）。不可摆放（该侧收起 / 不是 Todo / 放大的是另一侧 / 看板视图 / 首页）时返回零尺寸矩形。
- `preview_desired` 新增 `PanelKind::Todo` 分支，替换现有按弹层展开隐藏预览的逻辑。
- 固定单槽，新增 `TODO_CONTENT_ID_OFFSET`（接在 `CODEHEALTH_CONTENT_ID_OFFSET = 5_000_000` 之后，具体取值在计划阶段确定）。
- 待推送指令走队列，仿 `take_codehealth_content_script`，在 `window_events.rs::apply_pending_editor_commands` 的同一节奏里注入，不新开每帧轮询。
- `assets.rs` 新增 `todo_content_root_for` 与 `todo-content/` 资源路由；产物 `assets/todo-content/{host.html, todo-content.js, todo-content.css}` 提交进仓库（与其它 content webview 一致），带严格 CSP、无外部引用，并补仿 `codehealth_content_bundle_assets_are_present` 的资源存在性测试。
- webview 只覆盖内容区，左栏与顶部 tab 行不被盖住。左栏的右键菜单在 macOS 上是原生 NSMenu（`todo_category_context_menu`），天然叠在 webview 之上；非 macOS 才走 iced 浮层 `category_context_menu_popup`，该回退路径在光标处弹出、可能伸进 webview 区域，但项目现状是「非 mac 从未实际编译过」（见 `2026-09-18` 起暂停的弹窗独立窗口化第二份 spec 的同一结论），本次不为它新增遮挡处理，等真发非 mac 时与其它旧模式浮层一并迁到独立原生子窗口；「清空列表」确认框是独立原生子窗口。
- **项目切换**：Todo 状态是每个 `Workspace` 一份，而 webview 只有一个槽。切换项目标签或回到首页时，Rust 对新活动项目重推一条 `SetView`（带新 `project_id` 与新 `revision`）；首页或其他面板时矩形置零。

## 迁完要删除的原生代码

直接切换，不留两套并存：

- `extensions/todo/view.rs` 中的 `todo_list_view`、`todo_list_row`、`todo_card`、`todo_search_bar`、`status_filter_*`、`todo_footer_bar`（新增框）、`drag_insert_indicator`、`todo_segment_divider`、`todo_status_button`、`todo_dispatch_overlay` / `todo_status_overlay` / `todo_status_filter_overlay` / `todo_calendar_overlay` / `todo_calendar_popup`、`dispatch_items` / `status_items`；`app/view.rs` 中对应的四个浮层挂载；`app.rs` 里按 Todo 弹层展开隐藏预览的逻辑与对应的 `dispatch_popup_open` 等访问器。
- 只为原生输入与拖拽服务的接线：`Capture{Add,ContentEdit,TodoSearch}Focus`、`take_content_edit_focused`、`add_field_id` / `content_field_id`、Todo 对应的 `TextInputTarget`、`TodoDrag`、`DragMove` / `DragEnd` / `RowSelect`、`AddEdit` / `AddSubmit` / `AddResizeStart`、`SearchInput` / `SearchSubmit`、`ContentEditStart` / `ContentEdit`、`StatusFilter*`、`Calendar*`、`Dispatch*`、`Status*`，以及一批 `HoverId::Todo*`。**保留**详情窗口用的 `CaptureDetailReplyFocus` 与 `Detail*`，以及左栏分类树与「清空列表」相关的全部消息。
- 位置下标类 `Message`（`Toggle(usize)` 等）改为按 `id` 处理的入口，落库、rank 计算、派生状态等逻辑保留并复用现有函数。
- 状态里随之无用的字段（`status_filter`、`search*`、`add_draft`、`editing_content`、`dispatch_open` / `status_open` / `calendar_*`、`drag` 等）一并清理；`add_input_height` 保留。**保留** `Flash` / `advance_flash` / `next_flash_wake`（新增后高亮的计时仍在 Rust），前端据 payload 的 `selected_id` / `scroll_nonce` 呈现。
- 左栏「移动到…」与 `category_picker` 整套（见上一节）。

## 错误与降级

- **webview 加载 / 渲染失败**：收到 `Failed` 或超时未收到 `Ready`，回落原生占位页（失败原因 + 重试），不保留旧 iced 列表。
- **落库失败**：走 `App::push_toast`，并推一条权威 `SetView` 还原前端乐观状态。Toast 来源 scope 与日志来源均为 `todo`。
- **dozerd 不可用**：由顶栏徽标统一展示，内容区不自画；此时 `Client` 调用失败按上条处理。
- **异步结果路由**：任务加载、变更结果必须带 `project_id` 路由到对应 `Workspace`，不得用当前聚焦项目（沿用现有 `Loaded` / `Mutated` 的口径），避免用户在等待期间切走项目标签后错写。
- **日志**：来源名 `todo`，用 `dozer_core::log_*!(LOG, ...)`，不写裸 `tracing` / `eprintln!`；不记录任务全文或输入内容，只记录 `id` 与操作类型。

## 测试与验收

**自动测试（Rust）**

- 协议编解码与 `revision` 丢弃逻辑。
- `SetView` 的 `Serialize` 形状快照（防字段被无意改名）。
- `todo_content_pane_bounds_for` 几何：收起 / 展开、拖拽分栏、「内容在前」列顺序、看板视图为零矩形、扣掉 tab 行高度、放大态。
- 事件处理按 `id` 映射：`id` 已不存在则空操作；`Reorder` 的 `after_id` 语义（沿用并改写现有排序测试）；`SetStatus` 三态映射；「进行中」不直接写入；派生状态映射（`todo_display_state`，沿用现有测试）。
- 分类拖动移动：合法放置、放到「全部」变顶层、放到自己或子孙被拒绝、点击不触发拖拽（阈值）。
- 资源存在性与 CSP 测试。

**自动测试（前端，`node --test` + `render-smoke.mjs`）**

- 每种数据形态的渲染冒烟：空列表、三段并存、全部完成、长文本、中文、无分类。
- 纯函数单测：搜索匹配、状态筛选、`#NNN` 编号、拖拽重排与前驱 `after_id` 计算、按 `project_id` 保留草稿、分类切换重置搜索。
- 输入法组合期间回车 / ⌘↵ 不提交（以事件 `isComposing` 模拟）。

**人工验收**

1. 拖拽排序：放置指示线所见即所得，只有进行中段可拖。
2. 内联编辑：回车提交、失焦提交、空文本丢弃；中文输入法组合期间回车不误提交。
3. 五个弹层：状态、派发、状态筛选、日历、分类选择器；打开后点空白 / `Esc` 关闭，不被遮挡。
4. 新增框：Enter 换行、⌘↵ 提交、「↑」提交、拖拽高度并在重启后保持。
5. 切换项目标签再切回来，草稿、搜索、状态筛选、滚动位置还在。
6. 快速切换左栏分类无白屏闪烁，并清空搜索。
7. 收起 / 展开列表列，内容区跟随；切到看板视图 webview 隐藏。
8. 左栏拖动：拖分类到另一个分类下、拖到「全部」变顶层、拖到自己子孙下被拒绝并有 Toast。
9. 左栏右键菜单（macOS 原生）不再有「移动到…」。
10. 外部 agent 通过 MCP 新增任务后，列表自动出现新任务。

## 交付与前置

- **前置**：`codehealth-webview` 已于 2026-10-02 合并进 main，先例代码可直接引用。
- **一次切换**：没有新功能，只做 1:1 搬迁，不拆发布切片；plan 内部拆成多个任务，每个任务自带测试，旧 iced 列表在同一个分支里被替换，不会出现新旧并存的中间发布态。
- 独立 worktree 分支开发，审阅通过后再合并 main。

## 未决项

- 全局缩放（Ctrl +/-）在 Todo webview 内的缩放同步方式：沿用其它 content webview 的做法，计划阶段核实。
- 前端进入「内联编辑」后，点击其他卡片或弹层时，当前编辑的提交时机（失焦即提交）与现有 iced 行为的边缘差异：计划阶段按现有行为写测试固定。
- 看板视图本期仍是原生占位，其真正实现不在本 spec 范围。
