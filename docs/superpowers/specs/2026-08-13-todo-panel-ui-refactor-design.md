# Todo 面板 UI 重构设计

**状态:已批准(brainstorming 会话,2026-08-13)**

## 背景

`extensions::todo`(`crates/dozer-app/src/extensions/todo.rs`)当前的 列表/看板/MARKDOWN
三视图、侧栏分类导航(全部/待办/进行中/已完成 + 计数)、搜索框，都是 commit
`44c2d72`("Todo 面板改版")一次性加上的，对着一张未入库的截图改的，没有走
brainstorming→spec→plan 流程，也没有留下设计文档。三视图里，列表视图完整实现
(扁平高亮行样式)；看板视图只是"三个静态列(待办/进行中/完成)+ 该状态任务标题
卡片，纯展示"的占位符，无拖拽、无交互；MARKDOWN 视图是只读源码 dump。

用户给了新的参考草图(两张:看板布局、列表布局)，要求参照 `extensions::project`
v2(`docs/superpowers/specs/2026-08-13-project-info-pane-v2-design.md`)已确立的
视觉规范(边框卡片、footer-bar 结构)重做 Todo 面板的 列表 视图和把 看板 从占位符
落成真实可用的视图。

## 目标 / 非目标

**目标**:

1. 引入统一的边框卡片组件 `todo_card()`，List/Kanban 两个视图共用同一套卡片
   视觉(边框 + 圆角 + 内部布局)，取代列表现有的扁平高亮行样式和看板现有的
   三静态列占位卡片。
2. 看板视图从"三静态列占位符"改造成真实可用视图:单列自适应换行网格(卡片按
   面板宽度自动换行，类似 CSS flex-wrap)，不做跨列拖拽。
3. 卡片状态显示统一成一种紧凑 pill 形状，文字/颜色随 `TodoState` 三态变化，
   取代现有列表行里三套不同视觉(待办=计划日期金色文字或纯文字、进行中=头像圆
   +绿点+`EXECUTING`文字、已完成=对钩图标+`SUCCESS_MM-DD`)。
4. pill 在 `Pending`/`Done` 两态下可点击，弹出"待办/已完成"二选一菜单，选中后
   显式把该任务设成对应完成态；`InProgress` 态 pill 只读展示(不进菜单——见
   "状态 pill 语义"一节，`InProgress` 是从存活派发 session 推导出来的，不是
   可直接设置的字段)。
5. 卡片新增一个独立的派发图标按钮，仅在 `Pending` 且尚未派发时展示，位置在
   卡片底部与状态 pill 并排；点击复用现有 `Message::DispatchOpen` 弹窗(不新写
   派发逻辑)。
6. 卡片顶部右侧展示日期徽章(`Pending` 显示计划日期，`Done` 显示完成日期，都
   没有则显示 `-`)，点击进入现有 `Message::PlanDateEditStart` 内联编辑流程。
7. 底部快速新建栏(`Message::AddInputChanged`/`AddSubmit`)按 `project.rs::
   project_footer_bar` 的 footer-bar 结构重新样式化(1px `BORDER` 分隔线 +
   `padding([6, 8])`)，List/Kanban 两视图共用同一个 footer-bar。
8. List/Kanban 共用同一份搜索框(`ws_state.search`)和侧栏分类筛选
   (`ws_state.filter`)状态，切换 tab 不清空当前搜索/筛选。
9. 评估 `icons::icon_button_entry`/`tabs::tab_core` 能否套用到侧栏分类按钮
   (`todo_category_button`)和顶部视图切换 tab(`todo_tab`)，评估结论(继续手写
   自定义组件的理由，见"共享组件评估"一节)写入实现计划。

**非目标**:

- 不做看板列间拖拽(卡片状态切换只通过 pill 菜单/checkbox/派发弹窗，不通过
  拖动手势)。
- 不改动侧栏分类导航(`todo_category_button`)的功能/布局，只在"共享组件评估"
  一节里评估是否迁移到 `icon_button_entry`，不预先决定必须迁移。
- 不改动 MARKDOWN 视图(`todo_markdown_view`)。
- 不改动 `.dozer/todo.md` 的解析格式(`parse_todo`/`replace_todo_line`/
  `append_todo_item`)。
- 不改动派发(dispatch)、计划日期(plan date)背后的数据模型(`TodoTaskMeta`/
  `DispatchRecord`/`todo_meta.json`)，只改前端如何触发/展示这些既有能力。
- 不新增 `TodoState` 变体或让 `InProgress` 变成可手动设置的字段——它依旧由
  `todo_display_state()` 从 `done` + 存活派发 session 推导。

## 架构与数据流

### 1. 新依赖:`iced_aw`(`wrap` feature)

看板的自适应换行网格需要一个当前 app 里没有的排版能力(类 CSS flex-wrap)。
调查确认:

- `iced_widget 0.14.2` 本身不导出任何 wrap/flow 部件。
- `iced_aw 0.13.1` 已经在 `Cargo.lock` 里被间接解析(经由 vendored 的
  `iced-code-editor`)，版本与本项目的 `iced_widget 0.14` 线兼容，但当前
  `crates/dozer-app/Cargo.toml` 没有把它列为直接依赖。
- `iced_aw` 自带 `iced_aw::widget::wrap::Wrap`(`wrap` cargo feature)，支持
  `direction::Horizontal`/`Vertical` 真正的多行/多列换行，形状正好匹配
  "卡片按宽度自动换行"的需求。

`crates/dozer-app/Cargo.toml` 新增:

```toml
iced_aw = { version = "0.13", default-features = false, features = ["wrap"] }
```

(具体版本号写计划阶段核对 `Cargo.lock` 里已解析的精确版本，保持一致，避免
二次解析出不同版本。)

### 2. 状态 pill 语义:新增 `Message::SetDone`,不是复用 `Toggle`

现有 `Message::Toggle(idx)` 是**反转** `done`。如果 pill 菜单的"待办"/"已
完成"两个选项直接发 `Toggle`，会出现:任务当前是 `InProgress`(`done ==
false`，但有存活派发)，用户点"待办"选项本意是"确认它还是待办"，如果这触发
`Toggle`，会把 `done` 从 `false` **翻转成 `true`**，误判成"已完成"——这是
个真实的正确性 bug，不是风格问题。

所以 pill 菜单要发**显式目标值**，不是"翻转":

```rust
pub enum Message {
    // ...既有变体...
    /// pill 菜单选中"待办"/"已完成"时发出，`bool` 是目标 `done` 值(非翻转)。
    /// 当前已经是目标值时(如 InProgress 点"待办"，done 本来就是 false)
    /// 视为 no-op，不重复写盘。
    SetDone(usize, bool),
    /// pill 是否展开菜单(仅 Pending/Done 态可展开，见 view 层判断)。
    StatePillOpen(usize),
    StatePillClose,
}
```

`update()` 里 `SetDone` 的实现结构镜像现有 `Toggle` 分支(读文件、
`replace_todo_line`、写盘、`reload_from_disk`、"文本没变才更新
`completed_at`"那条既有规则)，区别只是新旧行的 `done` 状态从"参数直接给"
而不是"读当前 `item.done` 取反"。`Toggle` 保留不删——checkbox 点击依旧用
`Toggle`(继续是"翻转"语义，checkbox 场景没有"选中某个目标值"的菜单，翻转
是对的)。

`StatePillOpen`/`StatePillClose` 管理菜单展开态，`WorkspaceState` 新增一个
`state_pill_open: Option<usize>` 字段，写法镜像既有 `dispatch_open:
Option<usize>`。

### 3. 卡片组件 `todo_card()`

新私有渲染函数，List/Kanban 共用:

```rust
fn todo_card<'a>(
    number: usize,
    idx: usize,
    item: &'a TodoItem,
    state: TodoState,
    meta: Option<&'a TodoTaskMeta>,
    dispatch: Option<&'a DispatchRecord>,
    selected: bool,
    dispatch_open: bool,
    state_pill_open: bool,
    editing_plan_date: Option<&'a str>,
    existing_tabs: &'a [(&'a str, String)],
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer>
```

结构(自上而下):

1. 顶部行:`#{number:03}` 编号(左) + 日期徽章(右，`calendar` 类图标 + 文字，
   `Pending` 取 `meta.plan_date`、`Done` 取 `meta.completed_at` 格式化
   `MM-DD`、都没有显示 `-`；点击发 `Message::PlanDateEditStart(idx)`，复用
   现有内联编辑，编辑态时这一行替换成现有 `todo_plan_date_edit_row` 的输入框)。
2. 中部:checkbox(复用现有勾选交互，发 `Message::Toggle`) + 任务文字(完成态
   删除线，复用现有 `rich_text`/`span` 处理)。
3. 底部行(右对齐):
   - `state == Pending && dispatch.is_none()` 时展示派发图标按钮，点击发
     `Message::DispatchOpen(idx)`，弹窗复用现有 `todo_dispatch_popup`。
   - 状态 pill:统一形状(圆角矩形，`padding([4, 10])`，1px 描边)，文字/色随
     `state` 变——`Pending` = "待办"/`DIM`边框、`InProgress` = "进行中"/
     `GREEN`边框(只读，不挂 `on_press`)、`Done` = "已完成"/`GREEN`实底。
     `Pending`/`Done` 态 pill 挂 `MouseArea`/`button`，点击发
     `Message::StatePillOpen(idx)`；`state_pill_open` 为真时在 pill 下方叠一
     个两项菜单(待办/已完成)，选中项发 `Message::SetDone(idx, 选中值)`，
     样式镜像现有 `todo_dispatch_popup`(CARD 底 + BORDER 描边)。

外层容器:边框卡片(`CARD` 背景、`BORDER` 1px 描边、6.0 圆角，对齐
`todo_kanban_placeholder` 现有卡片的描边参数)，选中态(`selected`)在左侧加
3px `GOLD` 竖条(对齐现有 `todo_row` 的 `accent` 处理)。点击卡片主体(编号/
文字区域，不含 checkbox/pill/派发按钮/日期徽章这几个可交互子区域)发
`Message::RowSelect`，语义与现有列表行一致。

`todo_row` 函数删除，`todo_kanban_placeholder` 删除，两处渲染都改调
`todo_card`。

### 4. 列表视图:单列纵向堆叠

```rust
fn todo_list_view<'a>(...) -> Element<'a, ...> {
    // 结构不变:search -> scrollable(卡片列表，column![].spacing(N)) -> footer_bar
    // 每个可见任务调 todo_card()，外层用 column! 纵向堆叠(单列)。
}
```

### 5. 看板视图:自适应换行网格

```rust
fn todo_kanban_view<'a>(
    app_state: &'a AppState,
    ws_state: &'a WorkspaceState,
    project_id: i64,
    states: &[TodoState],
    tabs: &[SessionTabSummary],
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>
```

结构对齐 `todo_list_view`:同一份 `search`(顶部) + `filter_todos` 结果集
(与列表视图完全共用 `ws_state.search`/`ws_state.filter`，不新开独立筛选
状态) + `iced_aw::widget::wrap::Wrap`(装 `todo_card` 生成的卡片元素，
`direction::Horizontal`，卡片定宽——具体宽度写计划阶段量出来，参照草图
"一行两张"在常见面板宽度下的观感调) + 底部共用 footer-bar。空结果态复用
现有"没有匹配的任务"文案。

### 6. Footer-bar

新私有函数 `todo_footer_bar()`，结构照抄 `project.rs::project_footer_bar`
(1px `BORDER` 分隔线 + `container(...).padding([6, 8])`)，内容换成现有
`add_row` 的图标 + `text_input`(`icons::IconKind::SquarePlus` GOLD +
placeholder "Initiate new task protocol.."，`on_input`
`Message::AddInputChanged`、`on_submit` `Message::AddSubmit`)。List/Kanban
两个视图 body 函数结尾都调用它，取代各自内联的 `divider` + `add_row` 拼接。

### 7. 共享组件评估:`icon_button_entry`/`tabs::tab_core`

评估结论(供实现计划采纳，非目标里已声明不强制迁移):

- `icons::icon_button_entry`(`icons.rs` L227-297)是固定尺寸的**纯图标**方块
  按钮(`view(kind, size, color)` 包一层，无文字、无计数)，形状与
  `todo_category_button`(图标 + 文字标签 + 右对齐计数，横向铺满宽度)和
  `todo_tab`(图标 + 文字 + 下划线指示条)都不匹配——套用需要先把
  `icon_button_entry` 改造成能塞任意 `content` 而不是固定 `view(kind,
  size, color)`，改造成本和收益不成比例。**结论:继续手写，不迁移**，与
  `docs/superpowers/specs/2026-08-12-icon-button-tab-full-rollout-design.md`
  已经做过的排除判断(该文档明确排除"图标+文字组合按钮")一致。
- `tabs::tab_core`(`tabs.rs`)返回 `(select, close)` 两个元素，是为**可关闭**
  tab(项目页签)设计的交互(hover 才显示关闭按钮)。`todo_tab` 是纯粹的视图
  模式切换(列表/看板/MARKDOWN)，没有"关闭"语义，套用 `tab_core` 需要传一个
  永远不触发的 `on_close`，属于削足适履。**结论:继续手写，不迁移**。

实现计划里把以上两条结论原样写进去(不需要重新评估，直接引用本节)。

## 交互细节补充

- 派发按钮的可见性由 `state == TodoState::Pending && dispatch.is_none()`
  判断——`dispatch.is_some()` 但因为目标 session 已退出导致 `state` 仍是
  `Pending`(见 `todo_display_state` 的 `target_alive` 分支)这种情况，
  仍然显示派发按钮(允许重新派发)，跟现有列表行"没有计划日期就显示派发
  按钮"里没有额外排除"已经派发过但 session 死了"的口径一致。
- pill 菜单点击选项后立即 `StatePillClose`(同现有 `DispatchOpen`/
  `DispatchClose` 选中后关闭的既有模式)。
- 日期徽章点击进入编辑态时，同一时刻只允许一个卡片处于编辑态(复用现有
  `ws_state.editing_plan_date: Option<(usize, String)>`，字段不变)。
- Kanban 视图卡片的 `RowSelect`/`selected` 高亮语义与列表视图共享同一个
  `ws_state.selected_row`，切 tab 不清空选中(同搜索/筛选一样保留)。

## 错误处理

- `SetDone` 走跟现有 `Toggle` 完全相同的错误路径:文件读取失败直接
  return；`replace_todo_line` 返回 `None`(并发冲突)→ `reload_from_disk`
  放弃这次写入；文本在重读期间发生变化 → 跳过 `completed_at` 更新。这些
  都是搬 `Toggle` 分支的既有逻辑，不新增错误处理分支。
- `iced_aw::Wrap` 渲染失败/极端宽度(面板宽度小于单张卡片最小宽度)不做特殊
  兜底——退化成单列(宽度不够时 wrap 自然只放得下一列)，属于组件自身行为，
  不需要 Dozer 侧额外处理。

## 测试策略

- `todo.rs` 单测新增:`update()` 对 `SetDone` 的三条路径——
  `Pending`(`done=false`)点"已完成"(`SetDone(idx, true)`)→ `done` 变
  `true` 且更新 `completed_at`；`InProgress`(`done=false`)点"待办"
  (`SetDone(idx, false)`)→ `done` 保持 `false`(no-op，`completed_at` 不
  变)；`Done`(`done=true`)点"待办"(`SetDone(idx, false)`)→ `done` 变
  `false` 且清空 `completed_at`。镜像现有
  `update_toggle_flips_line_on_disk_and_sets_completed_at` 的测试写法。
- `update()` 新增 `StatePillOpen`/`StatePillClose` 状态切换测试，镜像现有
  `update_dispatch_open_and_close_toggle_popup`。
- `filter_todos`/`todo_display_state` 等既有纯函数测试不变(未改动这两个
  函数的签名/行为)。
- 看板 `Wrap` 布局渲染效果、卡片视觉、pill 菜单弹出位置属于人工 GUI 验收
  范畴(iced 部件树没法用现有单测框架断言像素级布局)，写进实现计划的验收
  清单:列表/看板视图卡片边框视觉一致；看板缩窄面板宽度时卡片正确换行；
  点击 pill 在 待办/已完成 间切换且状态正确落盘；`InProgress` 态 pill 不
  可点击；派发按钮只在待办未派发任务上出现，点击后原有派发流程不受影响；
  日期徽章点击进入既有计划日期编辑流程；footer-bar 视觉与 Project 面板
  一致；List/Kanban 切换时搜索词/筛选/选中行保留。

## 依赖变更

新增 `iced_aw`(`features = ["wrap"]`，`default-features = false`)作为
`crates/dozer-app` 的直接依赖，版本对齐 `Cargo.lock` 里已解析的
`iced_aw 0.13.1`。
