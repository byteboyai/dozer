# 四栏可拖拽调宽设计

**状态：已批准（brainstorming 会话，2026-07-28）**

## 背景

四栏主界面（项目/预览/终端/AI）当前宽度写死：项目栏 `PROJECT_COL_WIDTH=240.0`、AI 栏
`AI_COL_WIDTH=280.0`，预览/终端栏各拿剩余空间的一半（`Length::Fill` 等权）。用户反馈
（dogfooding UI 问题清单第 1 项）：应能拖动三条分隔线自由调整四栏宽度。

## 目标 / 非目标

**目标**：三条分隔线（项目|预览、预览|终端、终端|AI）可拖拽；拖拽有最小宽度保护，不会把
某一栏挤没；调整结果跨重启持久化。

**非目标**：不引入 iced `pane_grid`（该 widget 面向自由拆分/拖拽重排的多面板场景，这里只
要固定顺序的四栏调宽，用它是杀鸡用牛刀，且它的内部 ratio 状态与本仓已有的、供 webview
bounds/IME/命中测试复用的手算几何函数难以对齐）；不做垂直方向调整；不做拖拽重排列顺序；
持久化是应用级偏好（不区分项目/窗口）。

## 架构与数据流

### 1. 唯一数据源：`PanelLayout`

新增结构体（`workspace.rs`），承担运行时状态、`view()` 渲染、离屏几何计算（webview bounds
/ IME 光标 / 鼠标命中测试）、磁盘持久化四方共享的单一数据源：

```rust
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PanelLayout {
    pub project_col_width: f32,
    pub ai_col_width: f32,
    /// 预览栏占 (窗口宽 - 项目栏 - AI栏) 剩余空间的比例；终端栏拿 (1.0 - 此值)。
    pub preview_ratio: f32,
}

impl Default for PanelLayout {
    fn default() -> Self {
        Self { project_col_width: 240.0, ai_col_width: 280.0, preview_ratio: 0.5 }
    }
}
```

`Workspace` 新增字段 `layout: PanelLayout`（替代原先的两个 `pub const`）与
`dragging: Option<Divider>`。

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Divider { ProjectPreview, PreviewTerminal, TerminalAi }
```

### 2. 既有几何函数改签名

`preview_content_bounds`、`terminal_pane_pixel_size`、`is_in_preview_column`、
`ime_cursor_area` 四处（`workspace.rs`）目前内部直接读 `PROJECT_COL_WIDTH`/
`AI_COL_WIDTH` 常量、且预览/终端按 `fill_width / 2.0` 对半分。改为：

- 前三个自由函数新增 `layout: &PanelLayout` 参数；内部 `fill_width / 2.0` 改
  `fill_width * layout.preview_ratio`（终端侧对应 `fill_width * (1.0 - layout.preview_ratio)`）。
- `ime_cursor_area` 是 `Workspace` 方法，本就能读 `self.layout`，不需要新增参数，直接把内部
  两处 `PROJECT_COL_WIDTH`/`AI_COL_WIDTH` 换成 `self.layout.project_col_width`/
  `self.layout.ai_col_width`。
- 调用方（`main.rs` 两处 `preview_content_bounds`/`terminal_pane_pixel_size`
  调用点）补传 `&workspace.layout`——调用点已持有 `workspace`，无借用冲突。

新增一个纯函数，给 `main.rs` 的拖拽命中测试与后续悬停光标判定用（与 `view()` 里
`Length::Fixed`/`Length::FillPortion` 的口径必须保持数学等价，见下）：

```rust
/// 三条分隔线的窗口逻辑 x 坐标（项目|预览、预览|终端、终端|AI）。
pub fn divider_positions(window_width: f32, layout: &PanelLayout) -> [f32; 3]
```

### 3. `view()` 渲染

`project_pane`/`ai_pane` 原来的 `.width(Length::Fixed(PROJECT_COL_WIDTH))` /
`Length::Fixed(AI_COL_WIDTH)` 改读 `self.layout.project_col_width`/`ai_col_width`
（`Workspace` 字段，非常量，其余不变）。

`preview_pane`/`terminal_pane` 原来隐式 `Length::Fill`（等权 `FillPortion(1)`）改为显式
按比例分配：`Length::FillPortion((layout.preview_ratio * 10_000.0).round() as u16)` /
`Length::FillPortion(((1.0 - layout.preview_ratio) * 10_000.0).round() as u16)`——`u16`
上限 65535，缩放系数 10000 远低于溢出线，精度约 0.01%，足够。

三条分隔线渲染为四栏之间的独立元素：视觉上 2px 宽 BORDER 色竖线，包一层更宽（≥8px）的
命中区，用 `iced_widget::MouseArea` 承载：

```rust
MouseArea::new(divider_visual)
    .interaction(mouse::Interaction::ResizingColumn)  // 悬停即变光标，走 iced 既有的
                                                        // mouse_interaction → window.set_cursor
                                                        // 管线（main.rs:808-816 已有），
                                                        // 无需另起一套光标代码
    .on_press(Message::ColumnDragStart(Divider::ProjectPreview))
```

`on_press` 只负责**发起**拖拽（记录 `self.dragging = Some(divider)`）；不依赖 `MouseArea`
的 `on_move`/`on_release`。**原因**：读过 `iced_widget::mouse_area` 0.14.2 源码——
`on_move`/`on_release` 内部都有 `if !cursor.is_over(layout.bounds()) { return; }` 早退；
分隔线命中区仅 8px 宽，稍快的横向拖拽一帧内光标就会移出该窄区，后续 move/release 事件全部
丢失，拖拽会在半途"断裂"。`MouseArea` 不做超出自身 bounds 的指针捕获。

### 4. 拖拽的连续追踪：复用 `main.rs` 的原始 `WindowEvent` 层

`main.rs` 的 `on_window_event`（`Runner::on_window_event`）已经在 iced 正常事件分发**之外**
拦截 `CursorMoved`/`MouseInput`，用于终端选区聚焦路由等——这层拿到的是整个窗口的事件，不受
某个 widget bounds 限制，天然没有上面的"指针捕获"问题。扩展：

- `CursorMoved`：更新 `cursor_phys` 后，若 `workspace.dragging.is_some()`，按现有的
  物理→逻辑坐标换算（`main.rs:242` 一带已有同款换算）得到 `logical_x` 与
  `window.inner_size()` 换算出的 `window_width`，发 `Message::ColumnDrag { window_width,
  logical_x }`，随后请求重绘。
- `MouseInput { state: Released, button: Left, .. }`：若 `workspace.dragging.is_some()`，
  发 `Message::ColumnDragEnd`（当前只有 `Pressed` 分支，需新增 `Released` 分支）。

`Workspace::update` 里的处理（`dragging` 记录"哪条线在拖"，具体量交给消息里的窗口宽/光标
x 由 `update` 统一算+夹取——单一权威位置，不与 `main.rs` 分摊裁剪逻辑）：

- `ColumnDragStart(d)` → `self.dragging = Some(d)`
- `ColumnDrag { window_width, logical_x }` → 若 `self.dragging` 为 `None` 直接忽略；否则
  按 `dragging` 的具体分支重算并夹取 `self.layout` 对应字段（公式见下节）
- `ColumnDragEnd` → `self.dragging = None`；`self.handle.spawn(...)` 把当前
  `self.layout` 异步写盘（不阻塞 UI 线程，遵守文件头注释里"UI 线程绝不 block_on IO"的
  既有规则）

### 5. 夹取边界

```rust
const MIN_PROJECT_COL_WIDTH: f32 = 180.0;
const MIN_AI_COL_WIDTH: f32 = 200.0;
/// 预览+终端剩余空间下限，防止项目/AI 栏被拖得太宽把中间两栏挤没。
const MIN_FILL_WIDTH: f32 = 480.0;
const MIN_PREVIEW_RATIO: f32 = 0.15;
const MAX_PREVIEW_RATIO: f32 = 0.85;
```

`f32::clamp(min, max)` 在 `min > max` 时会 panic——窗口足够窄时
`window_width - ai_col_width - MIN_FILL_WIDTH` 可能小于 `MIN_PROJECT_COL_WIDTH`，必须先保证
上界不低于下界再夹取：

- `Divider::ProjectPreview` 拖拽：
  `let upper = (window_width - layout.ai_col_width - MIN_FILL_WIDTH).max(MIN_PROJECT_COL_WIDTH);`
  `let new_project = logical_x.clamp(MIN_PROJECT_COL_WIDTH, upper);`（`upper` 用 `.max()`
  垫底后恒 `>= MIN_PROJECT_COL_WIDTH`，`clamp` 的 `min<=max` 前提恒成立，不会 panic；窗口太窄
  时效果是新宽度钉在 `MIN_PROJECT_COL_WIDTH`，而不是抛异常）。
- `Divider::TerminalAi` 拖拽：同构，
  `let upper = (window_width - layout.project_col_width - MIN_FILL_WIDTH).max(MIN_AI_COL_WIDTH);`
  `let new_ai = (window_width - logical_x).clamp(MIN_AI_COL_WIDTH, upper);`
- `Divider::PreviewTerminal` 拖拽：`fill_width = window_width - project - ai`；
  `fill_width <= 0.0` 时直接跳过本次更新（防除零，且此时窗口已窄到两栏都顶着下限，拖比例没
  意义）；否则 `ratio = ((logical_x - project) / fill_width).clamp(MIN_PREVIEW_RATIO,
  MAX_PREVIEW_RATIO)`。

### 6. 持久化：新模块 `layout.rs`

`dozer-app` 目前没有 `serde`（只有 `serde_json`），需在其 `Cargo.toml` 补
`serde.workspace = true`（workspace 根 `Cargo.toml` 已声明 `features=["derive"]`，无需新引
第三方 crate）。格式选 JSON（复用已有 `serde_json` 依赖，不为这一个小功能新增 `toml` crate）。

```rust
//! 四栏宽度布局的本地持久化——应用级偏好，不属于任何项目/窗口。
use crate::workspace::PanelLayout;
use std::path::Path;

pub fn default_path() -> std::path::PathBuf {
    dozer_core::paths::config_dir().join("layout.json")
}

/// 读盘失败（不存在/损坏/字段对不上）一律退化为 `PanelLayout::default()`，
/// 不 panic、不弹错——这是纯视觉偏好，丢了就丢了。真正调用方用 `load()`；
/// `load_from`/`save_to` 接收显式路径，供单测指向临时文件，不碰用户真实配置目录。
pub fn load() -> PanelLayout { load_from(&default_path()) }
fn load_from(path: &Path) -> PanelLayout { .. }

pub fn save(layout: &PanelLayout) -> std::io::Result<()> { save_to(&default_path(), layout) }

/// 写盘失败（目录不可写等）只 `tracing::warn!`，不影响正在运行的会话。
fn save_to(path: &Path, layout: &PanelLayout) -> std::io::Result<()> { .. }
```

`Workspace::new`/`Workspace::restored`（启动时构造的两条路径，见现有代码 `term_tab_first:
0` 那两处初始化）里 `layout: layout::load()` 代替原先隐式的常量默认值。

## 错误处理

- 配置文件缺失/损坏/字段类型对不上：`load()` 内部 `.ok()` 链式吞掉，退化默认值，不打断
  启动。
- 写盘失败：`tracing::warn!` 记录，不影响当前会话（与既有 `client.resize` 失败时的处理口径
  一致，见 `resize_all`）。
- 拖拽把栏宽推向不可能的区间（窗口本身太窄）：夹取上界先 `.max(下限)` 垫底，恒不触发
  `f32::clamp` 的 `min>max` panic，效果是新宽度钉在对应下限——不出现负宽度，延续现有
  `.max(0.0)` 防御风格（细节见"夹取边界"节）。

## 测试策略

- `PanelLayout` 默认值单测：`project_col_width==240.0`、`ai_col_width==280.0`、
  `preview_ratio==0.5`，锁定与当前视觉的等价性（回归）。
- 三个夹取分支的边界单测：例如窗口宽 1440 时拖 `ProjectPreview` 到极小/极大值，验证落在
  `[MIN_PROJECT_COL_WIDTH, 1440 - 280 - 480]` 内；`PreviewTerminal` 验证 ratio 落在
  `[0.15, 0.85]`；窗口窄到 `upper < MIN_PROJECT_COL_WIDTH` 的场景验证新宽度钉在下限、不 panic。
- `divider_positions` 与 `preview_content_bounds`/`terminal_pane_pixel_size` 的现有测试
  （`preview_content_bounds_is_inside_col2` 等，`workspace.rs:2576` 一带）改造为传入
  `&PanelLayout::default()`，断言值不变（回归，证明重构没改变默认行为）。
- `layout::load_from()`/`save_to()` 往返单测：指向 `tempfile`（或测试专用临时目录）而非真实
  `config_dir()`，写一个 `PanelLayout`、读回、断言相等；文件不存在/内容损坏时
  `load_from()` 返回默认值。
- 拖拽的连续鼠标追踪（`main.rs` 原始事件层）headless 无法测，留真机目测——与本仓一贯的
  "view 层改动 headless 只能编译验证、真机视觉留用户验收" 惯例一致。

## 依赖变更

`crates/dozer-app/Cargo.toml` 新增一行 `serde.workspace = true`（workspace 根已声明，非
新第三方依赖）。
