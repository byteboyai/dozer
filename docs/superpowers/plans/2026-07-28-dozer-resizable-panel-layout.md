# 四栏可拖拽调宽实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让四栏主界面（项目/预览/终端/AI）的三条分隔线可拖拽调宽，调整结果跨重启持久化。

**Architecture:** 引入 `PanelLayout`（`project_col_width`/`ai_col_width`/`preview_ratio`）作为运行时状态、`view()` 渲染、离屏几何计算（webview bounds/IME 光标/鼠标命中测试）、磁盘持久化四方共享的唯一数据源，替换现有的两个 `pub const` 宽度常量。分隔线渲染为 `iced_widget::MouseArea` 包裹的 8px 命中区（视觉线 2px），`on_press` 发起拖拽；因 `MouseArea` 的 `on_move`/`on_release` 在光标移出其窄 bounds 后就不再触发（读过 0.14.2 源码确认，无指针捕获），拖拽的**持续追踪**改走 `main.rs` 已有的、绕过 iced 正常分发的原始 `WindowEvent` 拦截层（`Runner::on_window_event`，本就用于终端选区聚焦路由）。

**Tech Stack:** Rust, iced 0.14（`iced_widget::MouseArea`、`Length::FillPortion`、`mouse::Interaction::ResizingColumn`）；`serde`（新增 `dozer-app` 依赖，workspace 根已声明 `features=["derive"]`）+ 现有 `serde_json` 做本地 JSON 持久化。

## Global Constraints

- 颜色只取自 `crates/dozer-app/src/theme.rs`（本任务用 `theme::BORDER`），禁止新增硬编码色值。
- 每 task 收尾 `cargo clippy -p dozer-app --all-targets` clean、`cargo fmt -p dozer-app -- --check` 干净、`cargo test -p dozer-app` 绿。
- `Workspace::update` 里凡触碰磁盘/网络 IO 一律 `self.handle.spawn(...)`，UI 线程绝不 `block_on`（`workspace.rs` 文件头注释的既有规则）。
- iced 0.14 API 若与本计划字面不符，按编译器提示与本仓既有用法适配（本仓已用 `iced_widget::space::horizontal()`，`container.center_x(width)` 等）。
- 视觉/交互改动 headless 只能编译+单测验证，真机拖拽目测留用户（本仓一贯惯例）。
- 设计依据：`docs/superpowers/specs/2026-07-28-resizable-panel-layout-design.md`。

---

### Task 1: `PanelLayout` 数据模型 + 几何函数改为数据驱动

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（新增 `PanelLayout`/`Divider`/`DIVIDER_WIDTH`；改写 4 个几何函数；`Workspace` 新增 `layout` 字段与两处构造；`view()` 4 处宽度引用改读字段 + 插入 3 条静态分隔线；更新/新增测试）
- Modify: `crates/dozer-app/src/main.rs`（`preview_content_bounds`/`terminal_pane_pixel_size` 两个调用点补传 `&workspace.layout`）

**Interfaces:**
- Produces: `pub struct PanelLayout { project_col_width: f32, ai_col_width: f32, preview_ratio: f32 }`（`Default` 给 240.0/280.0/0.5）；`pub enum Divider { ProjectPreview, PreviewTerminal, TerminalAi }`；`pub fn preview_content_bounds(window_width: f32, window_height: f32, layout: &PanelLayout) -> (f32,f32,f32,f32)`；`pub fn terminal_pane_pixel_size(window_width: f32, window_height: f32, layout: &PanelLayout) -> (f32,f32)`；`pub fn is_in_preview_column(x: f32, window_width: f32, layout: &PanelLayout) -> bool`；`pub fn divider_positions(window_width: f32, layout: &PanelLayout) -> [f32;3]`；`Workspace::layout: PanelLayout` 字段（本任务先用 `PanelLayout::default()` 初始化，持久化留 Task 3）。
- Consumes（本任务新增调用方）: `main.rs` 两处几何调用点、`view()` 四个面板宽度、`ime_cursor_area` 内部两处。

- [ ] **Step 1: 写失败测试——`PanelLayout` 默认值 + 新签名**

在 `crates/dozer-app/src/workspace.rs` 的 `#[cfg(test)] mod tests`（约 2559 行起）里，紧挨 `preview_column_hit_test` 测试之后追加：

```rust
    #[test]
    fn panel_layout_default_matches_legacy_consts() {
        let l = PanelLayout::default();
        assert_eq!(l.project_col_width, 240.0);
        assert_eq!(l.ai_col_width, 280.0);
        assert_eq!(l.preview_ratio, 0.5);
    }

    #[test]
    fn divider_positions_account_for_divider_width() {
        // 窗口宽 1440,默认布局:项目栏 240 + AI 栏 280,三条分隔线各 8px。
        let layout = PanelLayout::default();
        let [d1, d2, d3] = divider_positions(1440.0, &layout);
        assert_eq!(d1, 240.0, "分隔线1紧贴项目栏右边");
        let fill = 1440.0 - 240.0 - 280.0 - 3.0 * DIVIDER_WIDTH;
        assert_eq!(d2, 240.0 + DIVIDER_WIDTH + fill * 0.5, "分隔线2在预览栏右边");
        assert_eq!(d3, 1440.0 - 280.0 - DIVIDER_WIDTH, "分隔线3紧贴AI栏左边");
    }
```

同时把已有的 `preview_content_bounds_is_inside_col2`、`terminal_pane_height_excludes_top_and_status_bars`、`preview_content_bounds_never_negative`、`preview_column_hit_test` 四个测试里的函数调用加上 `&PanelLayout::default()` 参数（先改测试让它们和新签名对齐，本步之后会编译失败，属预期）：

```rust
    #[test]
    fn preview_content_bounds_is_inside_col2() {
        let layout = PanelLayout::default();
        let (x, y, w, h) = preview_content_bounds(1440.0, 900.0, &layout);
        assert!(
            x > layout.project_col_width && x < layout.project_col_width + 30.0,
            "x={x}"
        );
        assert!((410.0..=460.0).contains(&w), "w={w}");
        assert!(
            y > 104.0 && y < 134.0,
            "y={y}(顶栏 44 + tab 栏+地址栏之下,表头已去)"
        );
        assert!(h > 700.0 && h < 900.0 - y, "h={h}");
    }

    #[test]
    fn terminal_pane_height_excludes_top_and_status_bars() {
        let layout = PanelLayout::default();
        let (_, h_with) = terminal_pane_pixel_size(1440.0, 900.0, &layout);
        let only_chrome = 900.0 - CHROME_HEIGHT_PX;
        assert!(
            (only_chrome - h_with - (TOP_BAR_HEIGHT + STATUS_BAR_HEIGHT)).abs() < 0.01,
            "终端 pane 高度必须再扣顶栏+状态栏"
        );
    }

    #[test]
    fn preview_content_bounds_never_negative() {
        let layout = PanelLayout::default();
        let (_, _, w, h) = preview_content_bounds(100.0, 50.0, &layout);
        assert!(w >= 0.0 && h >= 0.0);
    }

    #[test]
    fn preview_column_hit_test() {
        // 窗口宽 1440:项目栏 240 + AI 栏 280 + 三条分隔线各 8px,剩余按 0.5 对半分。
        let layout = PanelLayout::default();
        assert!(!is_in_preview_column(100.0, 1440.0, &layout), "落在项目栏");
        assert!(is_in_preview_column(248.0, 1440.0, &layout), "预览列左边界(过divider1)");
        assert!(is_in_preview_column(699.0, 1440.0, &layout), "预览列内");
        assert!(!is_in_preview_column(708.0, 1440.0, &layout), "已进divider2/终端列");
        assert!(!is_in_preview_column(1200.0, 1440.0, &layout), "终端列");
    }
```

（`preview_content_bounds_is_inside_col2`/`preview_column_hit_test` 的边界数值因新增 24px 分隔线开销而与原版略有出入，上面已按新公式重算——分隔线1在 240，预览栏从 248 开始，`fill=1440-240-280-24=896`，`preview_end=248+896*0.5=696`，故 699 仍在内、708 已出。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app panel_layout_default_matches_legacy_consts` — Expected: FAIL（`PanelLayout` 未定义）。
Run: `cargo build -p dozer-app` — Expected: 大量编译错误（签名不匹配），属预期，下一步修。

- [ ] **Step 3: 定义 `PanelLayout`/`Divider`/`DIVIDER_WIDTH`**

在 `crates/dozer-app/src/workspace.rs` 顶部 `use` 区（约第 20 行, `use iced_winit::winit::event_loop::EventLoopProxy;` 之后）新增：

```rust
use serde::{Deserialize, Serialize};
```

把（约第 25-28 行）

```rust
use iced_widget::core::{Border, Color, Element, Length};
use iced_widget::{button, column, container, row, text};
```

改为：

```rust
use iced_widget::core::mouse;
use iced_widget::core::{Border, Color, Element, Length};
use iced_widget::{button, column, container, row, text, MouseArea};
```

在原先 `pub const AI_COL_WIDTH: f32 = 280.0;`（第 51 行）那一块，把两个常量整体替换为：

```rust
/// 四栏宽度状态:项目栏/AI栏固定像素宽 + 预览:终端剩余空间分配比例。是
/// `view()` 渲染、离屏几何计算(webview bounds/IME/命中测试)、磁盘持久化
/// 三方共享的唯一数据源(见 specs/2026-07-28-resizable-panel-layout-design.md)。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PanelLayout {
    pub project_col_width: f32,
    pub ai_col_width: f32,
    /// 预览栏占 (窗口宽 - 项目栏 - AI栏 - 3*分隔线) 剩余空间的比例;
    /// 终端栏拿 (1.0 - 此值)。
    pub preview_ratio: f32,
}

impl Default for PanelLayout {
    fn default() -> Self {
        Self {
            project_col_width: 240.0,
            ai_col_width: 280.0,
            preview_ratio: 0.5,
        }
    }
}

/// 三条可拖拽分隔线的标识:项目|预览、预览|终端、终端|AI。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Divider {
    ProjectPreview,
    PreviewTerminal,
    TerminalAi,
}

/// 每条分隔线的命中区/渲染宽度(逻辑像素)。视觉线本身 2px,居中于此区间内。
/// 四栏几何公式必须把 `3 * DIVIDER_WIDTH` 从剩余空间里扣掉,否则 webview
/// bounds/IME 光标/命中测试会和 `view()` 里 `row!` 实际渲染的像素错位。
const DIVIDER_WIDTH: f32 = 8.0;
```

- [ ] **Step 4: 改写 4 个几何函数**

把（约第 71-98 行）三个自由函数：

```rust
pub fn preview_content_bounds(window_width: f32, window_height: f32) -> (f32, f32, f32, f32) {
    let fill_width = (window_width - PROJECT_COL_WIDTH - AI_COL_WIDTH).max(0.0);
    let x = PROJECT_COL_WIDTH + 8.0;
    let y = TOP_BAR_HEIGHT + PREVIEW_CHROME_TOP_PX;
    let w = (fill_width / 2.0 - 16.0).max(0.0);
    let h = (window_height - y - 8.0).max(0.0);
    (x, y, w, h)
}

pub fn is_in_preview_column(x: f32, window_width: f32) -> bool {
    let fill_width = (window_width - PROJECT_COL_WIDTH - AI_COL_WIDTH).max(0.0);
    let preview_end = PROJECT_COL_WIDTH + fill_width / 2.0;
    x >= PROJECT_COL_WIDTH && x < preview_end
}

pub fn terminal_pane_pixel_size(window_width: f32, window_height: f32) -> (f32, f32) {
    let fill_width = (window_width - PROJECT_COL_WIDTH - AI_COL_WIDTH).max(0.0);
    let pane_width = (fill_width / 2.0 - CHROME_WIDTH_PX).max(0.0);
    let pane_height =
        (window_height - TOP_BAR_HEIGHT - STATUS_BAR_HEIGHT - CHROME_HEIGHT_PX).max(0.0);
    (pane_width, pane_height)
}
```

改为：

```rust
pub fn preview_content_bounds(
    window_width: f32,
    window_height: f32,
    layout: &PanelLayout,
) -> (f32, f32, f32, f32) {
    let fill_width =
        (window_width - layout.project_col_width - layout.ai_col_width - 3.0 * DIVIDER_WIDTH)
            .max(0.0);
    let x = layout.project_col_width + DIVIDER_WIDTH + 8.0;
    let y = TOP_BAR_HEIGHT + PREVIEW_CHROME_TOP_PX;
    let w = (fill_width * layout.preview_ratio - 16.0).max(0.0);
    let h = (window_height - y - 8.0).max(0.0);
    (x, y, w, h)
}

pub fn is_in_preview_column(x: f32, window_width: f32, layout: &PanelLayout) -> bool {
    let fill_width =
        (window_width - layout.project_col_width - layout.ai_col_width - 3.0 * DIVIDER_WIDTH)
            .max(0.0);
    let preview_start = layout.project_col_width + DIVIDER_WIDTH;
    let preview_end = preview_start + fill_width * layout.preview_ratio;
    x >= preview_start && x < preview_end
}

pub fn terminal_pane_pixel_size(
    window_width: f32,
    window_height: f32,
    layout: &PanelLayout,
) -> (f32, f32) {
    let fill_width =
        (window_width - layout.project_col_width - layout.ai_col_width - 3.0 * DIVIDER_WIDTH)
            .max(0.0);
    let pane_width = (fill_width * (1.0 - layout.preview_ratio) - CHROME_WIDTH_PX).max(0.0);
    let pane_height =
        (window_height - TOP_BAR_HEIGHT - STATUS_BAR_HEIGHT - CHROME_HEIGHT_PX).max(0.0);
    (pane_width, pane_height)
}

/// 三条分隔线的窗口逻辑 x 坐标(项目|预览、预览|终端、终端|AI)。与上面三个
/// 函数同一份公式推出,数学上必须与 `view()` 的 `row!` 实际渲染位置一致
/// (`divider_positions_account_for_divider_width` 测试锁定)。
pub fn divider_positions(window_width: f32, layout: &PanelLayout) -> [f32; 3] {
    let fill_width =
        (window_width - layout.project_col_width - layout.ai_col_width - 3.0 * DIVIDER_WIDTH)
            .max(0.0);
    let d1 = layout.project_col_width;
    let d2 = layout.project_col_width + DIVIDER_WIDTH + fill_width * layout.preview_ratio;
    let d3 = window_width - layout.ai_col_width - DIVIDER_WIDTH;
    [d1, d2, d3]
}
```

- [ ] **Step 5: `Workspace` 新增 `layout` 字段 + 两处构造 + `ime_cursor_area`**

在 `pub struct Workspace {` 内、紧挨 `preview_tab_first: usize,` 字段（约 379 行）之后新增：

```rust
    /// 四栏宽度/预览终端分配比例;拖拽写入,`layout::load()` 起始值(Task 3
    /// 接线,本步先用 `PanelLayout::default()`)。
    layout: PanelLayout,
```

`bootstrap`（约 436 行 `let ws = Self {`）与 `with_daemon_error`（约 486 行 `Self {`）两处构造字面量,都在 `preview_tab_first: 0,` 那一行后加：

```rust
            layout: PanelLayout::default(),
```

`ime_cursor_area`（约 1211-1225 行）里两处 `PROJECT_COL_WIDTH`/`AI_COL_WIDTH` 与内部对 `terminal_pane_pixel_size` 的调用：

```rust
    pub fn ime_cursor_area(&self, window_w: f32, window_h: f32) -> (f32, f32, f32) {
        if self.preview.addr_editing() || self.acceptance_comment_editing() {
            return (
                PROJECT_COL_WIDTH + 12.0,
                TOP_BAR_HEIGHT + PREVIEW_CHROME_TOP_PX,
                20.0,
            );
        }
        let (pane_w, pane_h) = terminal_pane_pixel_size(window_w, window_h);
        let cell_w = pane_w / self.cols.max(1) as f32;
        let line_h = pane_h / self.rows.max(1) as f32;
        let fill_width = (window_w - PROJECT_COL_WIDTH - AI_COL_WIDTH).max(0.0);
        let x0 = PROJECT_COL_WIDTH + fill_width / 2.0 + 8.0;
```

改为：

```rust
    pub fn ime_cursor_area(&self, window_w: f32, window_h: f32) -> (f32, f32, f32) {
        if self.preview.addr_editing() || self.acceptance_comment_editing() {
            return (
                self.layout.project_col_width + 12.0,
                TOP_BAR_HEIGHT + PREVIEW_CHROME_TOP_PX,
                20.0,
            );
        }
        let (pane_w, pane_h) = terminal_pane_pixel_size(window_w, window_h, &self.layout);
        let cell_w = pane_w / self.cols.max(1) as f32;
        let line_h = pane_h / self.rows.max(1) as f32;
        let fill_width = (window_w
            - self.layout.project_col_width
            - self.layout.ai_col_width
            - 3.0 * DIVIDER_WIDTH)
            .max(0.0);
        let x0 = self.layout.project_col_width
            + 2.0 * DIVIDER_WIDTH
            + fill_width * self.layout.preview_ratio
            + 8.0;
```

（下面紧接的 `y0`/`col`/`row` 等代码不变；只改了这段涉及宽度常量的部分。）

新增一个只读 getter,供 Task 4 的 `main.rs` 用（放在 `ime_cursor_area` 之后即可）：

```rust
    /// 当前四栏宽度状态(main.rs 拖拽追踪/持久化用;`Copy` 类型直接按值返回)。
    pub fn layout(&self) -> PanelLayout {
        self.layout
    }
```

- [ ] **Step 6: `view()` 四处宽度引用改字段 + 插入三条静态分隔线**

新增一个纯视觉分隔线渲染函数（放在 `tab_bar` 附近或 `ai_pane`/`project_pane` 之前均可,建议紧邻 `terminal_pane` 之后、`TAB_BAR_AVAIL_PX` 之前）：

```rust
/// 分隔线:命中区 `DIVIDER_WIDTH` 宽、`Length::Fill` 高,内部一条 2px BORDER
/// 竖线居中。本步只做视觉,交互(悬停光标+`on_press`)留 Task 2。
fn divider_bar<'a>() -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let line = container(iced_widget::Space::new())
        .width(Length::Fixed(2.0))
        .height(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::BORDER.into()),
            ..container::Style::default()
        });
    container(line)
        .center_x(Length::Fixed(DIVIDER_WIDTH))
        .height(Length::Fill)
        .into()
}
```

把 `ai_pane` 结尾（约 1779 行）：

```rust
    container(content.padding(12))
        .width(Length::Fixed(AI_COL_WIDTH))
```

改为：

```rust
    container(content.padding(12))
        .width(Length::Fixed(ws.layout.ai_col_width))
```

把 `project_pane` 结尾（约 1905 行）：

```rust
    container(column![body, project_status_bar(ws)])
        .width(Length::Fixed(PROJECT_COL_WIDTH))
```

改为：

```rust
    container(column![body, project_status_bar(ws)])
        .width(Length::Fixed(ws.layout.project_col_width))
```

把 `preview_pane` 结尾（约 2112-2113 行）：

```rust
    container(content.padding(8))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
```

改为（`FillPortion` 用 10000 倍缩放换取连续比例的近似精度,见设计文档 §3）：

```rust
    let preview_portion = (ws.layout.preview_ratio * 10_000.0).round() as u16;
    container(content.padding(8))
        .width(Length::FillPortion(preview_portion))
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
```

把 `terminal_pane` 结尾（约 2193-2194 行,外层 `container(column![body, terminal_status_bar(ws)])`）：

```rust
    container(column![body, terminal_status_bar(ws)])
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
```

改为：

```rust
    let terminal_portion = ((1.0 - ws.layout.preview_ratio) * 10_000.0).round() as u16;
    container(column![body, terminal_status_bar(ws)])
        .width(Length::FillPortion(terminal_portion))
        .height(Length::Fill)
        .into()
}
```

最后把顶层 `view()`（约 1364-1372 行）：

```rust
    pub fn view(
        &self,
    ) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
        let top = top_bar(self);
        let col1 = project_pane(self);
        let col2 = preview_pane(self);
        let col3 = terminal_pane(self);
        let col4 = ai_pane(self);
        column![top, row![col1, col2, col3, col4]].into()
    }
```

改为：

```rust
    pub fn view(
        &self,
    ) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
        let top = top_bar(self);
        let col1 = project_pane(self);
        let col2 = preview_pane(self);
        let col3 = terminal_pane(self);
        let col4 = ai_pane(self);
        column![
            top,
            row![col1, divider_bar(), col2, divider_bar(), col3, divider_bar(), col4]
        ]
        .into()
    }
```

- [ ] **Step 7: `main.rs` 两处调用点补传 `&workspace.layout`**

`crates/dozer-app/src/main.rs:378`：

```rust
            let (x, y, w, h) = workspace::preview_content_bounds(logical_w, logical_h);
```

改为：

```rust
            let (x, y, w, h) =
                workspace::preview_content_bounds(logical_w, logical_h, &workspace.layout());
```

`crates/dozer-app/src/main.rs` 里 `terminal_pane_pixel_size` 的两处调用（约 628、865 行）,例如：

```rust
                        workspace::terminal_pane_pixel_size(logical.width, logical.height);
```

都改为：

```rust
                        workspace::terminal_pane_pixel_size(
                            logical.width,
                            logical.height,
                            &workspace.layout(),
                        );
```

（两处上下文分别是 `Resized` 处理与另一处窗口尺寸换算；`workspace` 变量在两处都已在作用域内,直接调用 `.layout()` 即可,无需额外持有。）

main.rs 里 `is_in_preview_column` 的调用（约 245 行）同样补参：

```rust
                        Some(if workspace::is_in_preview_column(logical_x, logical_w) {
```

改为：

```rust
                        Some(if workspace::is_in_preview_column(
                            logical_x,
                            logical_w,
                            &workspace.layout(),
                        ) {
```

- [ ] **Step 8: 编译 + 测试 + clippy + fmt**

Run: `cargo build -p dozer-app` — Expected: 编译通过(如有遗漏的调用点,按报错逐一补 `&layout`/`&workspace.layout()`)。
Run: `cargo test -p dozer-app` — Expected: 全绿,含 Step 1 新增/改写的测试。
Run: `cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean` — Expected: `clean`。
Run: `cargo fmt -p dozer-app -- --check` — Expected: 无输出。

- [ ] **Step 9: 真机目测（留用户）**

Run: `cargo run -p dozer-app`
Expected（用户实机）：四栏布局与之前视觉一致（默认比例未变），三条分隔线以 2px 细线出现在栏间，暂不可拖拽、无特殊光标（Task 2 才接线）。

- [ ] **Step 10: Commit**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
git commit -m "feat(resizable-layout): PanelLayout 数据模型 + 几何函数数据驱动化 + 静态分隔线"
```

---

### Task 2: 拖拽消息 + 夹取逻辑 + 分隔线交互化

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（`Message` 新增三个变体；`update()` 新增三个分支；`Workspace` 新增 `dragging` 字段与两处构造；`divider_bar` 升级为可交互；新增夹取常量与单测）

**Interfaces:**
- Consumes: Task 1 的 `PanelLayout`/`Divider`/`DIVIDER_WIDTH`/`Workspace::layout()`。
- Produces: `Message::ColumnDragStart(Divider)`/`Message::ColumnDrag { window_width: f32, logical_x: f32 }`/`Message::ColumnDragEnd`；`Workspace::dragging_divider(&self) -> Option<Divider>`（Task 4 的 `main.rs` 用）。

- [ ] **Step 1: 写失败测试——拖拽夹取行为**

夹取算法写成不依赖 `Workspace` 的纯函数 `apply_column_drag`（`update()` 只是瘦身调用它）——这样测试不必构造依赖真实 `Client`/`Handle`/`EventLoopProxy` 的完整 `Workspace`，直接调用纯函数即可。在 `mod tests` 里追加（紧邻 Task 1 新增的两个测试之后）：

```rust
    #[test]
    fn clamp_project_width_within_bounds() {
        let layout = PanelLayout::default();
        let new_layout = apply_column_drag(layout, Divider::ProjectPreview, 1440.0, 300.0);
        assert_eq!(new_layout.project_col_width, 300.0);
    }

    #[test]
    fn clamp_project_width_to_minimum() {
        let layout = PanelLayout::default();
        let new_layout = apply_column_drag(layout, Divider::ProjectPreview, 1440.0, 10.0);
        assert_eq!(new_layout.project_col_width, MIN_PROJECT_COL_WIDTH);
    }

    #[test]
    fn clamp_project_width_when_window_too_narrow_does_not_panic() {
        let layout = PanelLayout::default();
        let new_layout = apply_column_drag(layout, Divider::ProjectPreview, 700.0, 650.0);
        assert_eq!(new_layout.project_col_width, MIN_PROJECT_COL_WIDTH);
    }

    #[test]
    fn clamp_ai_width_within_bounds() {
        let layout = PanelLayout::default();
        let new_layout = apply_column_drag(layout, Divider::TerminalAi, 1440.0, 1140.0);
        assert_eq!(new_layout.ai_col_width, 300.0);
    }

    #[test]
    fn clamp_preview_ratio_within_bounds() {
        let layout = PanelLayout::default();
        let new_layout = apply_column_drag(layout, Divider::PreviewTerminal, 1440.0, 875.2);
        assert!((new_layout.preview_ratio - 0.7).abs() < 0.01);
    }

    #[test]
    fn clamp_preview_ratio_to_range() {
        let layout = PanelLayout::default();
        let new_layout = apply_column_drag(layout, Divider::PreviewTerminal, 1440.0, 10.0);
        assert_eq!(new_layout.preview_ratio, MIN_PREVIEW_RATIO);
    }

    #[test]
    fn clamp_preview_ratio_skips_update_on_zero_fill_width() {
        // project+ai 已经吃满窗口宽度,fill_width<=0,应原样返回不 panic(除零防御)。
        let layout = PanelLayout {
            project_col_width: 1000.0,
            ai_col_width: 1000.0,
            preview_ratio: 0.5,
        };
        let new_layout = apply_column_drag(layout, Divider::PreviewTerminal, 1440.0, 500.0);
        assert_eq!(new_layout, layout);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app clamp_project_width_within_bounds` — Expected: FAIL（`apply_column_drag` 未定义）。

- [ ] **Step 3: 实现夹取常量 + 纯函数 `apply_column_drag` + `Message`/`update()`/`Workspace.dragging`**

在 `DIVIDER_WIDTH` 常量之后新增：

```rust
const MIN_PROJECT_COL_WIDTH: f32 = 180.0;
const MIN_AI_COL_WIDTH: f32 = 200.0;
/// 预览+终端剩余空间下限,防止项目/AI 栏被拖得太宽把中间两栏挤没。
const MIN_FILL_WIDTH: f32 = 480.0;
const MIN_PREVIEW_RATIO: f32 = 0.15;
const MAX_PREVIEW_RATIO: f32 = 0.85;

/// 拖拽某条分隔线到窗口逻辑 x 坐标 `logical_x` 后的新 `PanelLayout`——纯函数,
/// 不依赖 `Workspace`,`update()` 与单测都调它。`f32::clamp(min,max)` 在
/// `min>max` 时会 panic,窗口太窄时用 `.max(下限)` 把上界垫到不低于下限,
/// 确保恒不 panic(细节见 specs/2026-07-28-resizable-panel-layout-design.md §5)。
fn apply_column_drag(
    layout: PanelLayout,
    divider: Divider,
    window_width: f32,
    logical_x: f32,
) -> PanelLayout {
    match divider {
        Divider::ProjectPreview => {
            let upper = (window_width - layout.ai_col_width - MIN_FILL_WIDTH)
                .max(MIN_PROJECT_COL_WIDTH);
            PanelLayout {
                project_col_width: logical_x.clamp(MIN_PROJECT_COL_WIDTH, upper),
                ..layout
            }
        }
        Divider::TerminalAi => {
            let upper = (window_width - layout.project_col_width - MIN_FILL_WIDTH)
                .max(MIN_AI_COL_WIDTH);
            PanelLayout {
                ai_col_width: (window_width - logical_x).clamp(MIN_AI_COL_WIDTH, upper),
                ..layout
            }
        }
        Divider::PreviewTerminal => {
            let fill_width = window_width - layout.project_col_width - layout.ai_col_width;
            if fill_width <= 0.0 {
                return layout;
            }
            let ratio = ((logical_x - layout.project_col_width) / fill_width)
                .clamp(MIN_PREVIEW_RATIO, MAX_PREVIEW_RATIO);
            PanelLayout {
                preview_ratio: ratio,
                ..layout
            }
        }
    }
}
```

在 `pub enum Message {` 里,紧邻 `PaneResized { cols: u16, rows: u16 },`（约 156 行）之后新增三个变体：

```rust
    /// 按下某条分隔线,记录"正在拖哪条"(main.rs 后续 CursorMoved 靠这个
    /// 状态决定要不要继续转发拖拽;Task 4 接线)。
    ColumnDragStart(Divider),
    /// 拖拽中:当前窗口逻辑宽 + 光标逻辑 x(main.rs 换算好传入,`update()`
    /// 统一算+夹取,不与 main.rs 分摊裁剪逻辑)。
    ColumnDrag { window_width: f32, logical_x: f32 },
    /// 松开左键,结束拖拽并触发写盘(Task 3 接线持久化)。
    ColumnDragEnd,
```

在 `pub struct Workspace {` 里紧邻 `layout: PanelLayout,` 之后新增：

```rust
    /// 正在拖拽的分隔线;`None` 表示未在拖拽。
    dragging: Option<Divider>,
```

`bootstrap`/`with_daemon_error` 两处构造字面量,紧邻 `layout: PanelLayout::default(),` 之后新增：

```rust
            dragging: None,
```

在 `update()` 的 `match message {` 里,紧邻 `Message::PaneResized { cols, rows } => self.resize_all(cols, rows),`（约 786 行）之后新增：

```rust
            Message::ColumnDragStart(divider) => {
                self.dragging = Some(divider);
            }
            Message::ColumnDrag {
                window_width,
                logical_x,
            } => {
                if let Some(divider) = self.dragging {
                    self.layout = apply_column_drag(self.layout, divider, window_width, logical_x);
                }
            }
            Message::ColumnDragEnd => {
                self.dragging = None;
            }
```

（Task 3 会把 `ColumnDragEnd` 分支扩成"清空 dragging + 异步写盘"，此处先只清状态，保证本任务的纯状态转换测试独立可跑。）

新增 getter（紧邻 Task 1 的 `layout()` 之后）：

```rust
    /// 当前正在拖拽的分隔线(main.rs 拖拽追踪用)。
    pub fn dragging_divider(&self) -> Option<Divider> {
        self.dragging
    }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app clamp_` — Expected: 7 个新测试全 PASS。
Run: `cargo test -p dozer-app` — Expected: 全绿。

- [ ] **Step 5: 分隔线升级为可交互**

把 Task 1 新增的 `divider_bar()` 函数签名从无参改为接收 `Divider`,并包一层 `MouseArea`：

```rust
/// 分隔线:命中区 `DIVIDER_WIDTH` 宽、`Length::Fill` 高,内部一条 2px BORDER
/// 竖线居中。悬停变 resize 光标走 `MouseArea::interaction` → iced 既有的
/// `mouse_interaction` → `window.set_cursor` 管线(main.rs:808-816 已有),
/// 不必另起一套光标代码。`on_press` 只发起拖拽状态,不指望 `MouseArea` 的
/// `on_move`/`on_release`——它们要求光标不离开这条 8px 窄带才触发,快速拖
/// 拽会在光标移出后"断掉";持续追踪交给 Task 4 的 `main.rs` 原始事件层。
fn divider_bar<'a>(divider: Divider) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let line = container(iced_widget::Space::new())
        .width(Length::Fixed(2.0))
        .height(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::BORDER.into()),
            ..container::Style::default()
        });
    let hit_area = container(line)
        .center_x(Length::Fixed(DIVIDER_WIDTH))
        .height(Length::Fill);
    MouseArea::new(hit_area)
        .interaction(mouse::Interaction::ResizingColumn)
        .on_press(Message::ColumnDragStart(divider))
        .into()
}
```

把 `view()` 里的三处调用：

```rust
            row![col1, divider_bar(), col2, divider_bar(), col3, divider_bar(), col4]
```

改为：

```rust
            row![
                col1,
                divider_bar(Divider::ProjectPreview),
                col2,
                divider_bar(Divider::PreviewTerminal),
                col3,
                divider_bar(Divider::TerminalAi),
                col4
            ]
```

- [ ] **Step 6: 编译 + 测试 + clippy + fmt**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 全绿 / clean / 无输出。

- [ ] **Step 7: 真机目测（留用户）**

Run: `cargo run -p dozer-app`
Expected（用户实机）：鼠标悬停在任一分隔线上时光标变横向拖拽样式；点击分隔线不报错、不崩溃（因 Task 4 还没接续追踪，此时拖动鼠标不会移动栏宽——这是本任务预期的中间态，非 bug）。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(resizable-layout): 拖拽消息 + 夹取算法 + 分隔线交互化(悬停光标+发起拖拽)"
```

---

### Task 3: 布局持久化

**Files:**
- Create: `crates/dozer-app/src/layout.rs`
- Modify: `crates/dozer-app/Cargo.toml`（新增 `serde.workspace = true`）
- Modify: `crates/dozer-app/src/main.rs`（新增 `mod layout;`）
- Modify: `crates/dozer-app/src/workspace.rs`（两处构造改用 `layout::load()`；`ColumnDragEnd` 分支扩成异步写盘）

**Interfaces:**
- Consumes: Task 1 的 `PanelLayout`（`Serialize`/`Deserialize` 已在 Task 1 派生）。
- Produces: `pub fn layout::load() -> PanelLayout`、`pub fn layout::save(&PanelLayout) -> std::io::Result<()>`。

- [ ] **Step 1: 加依赖**

把 `crates/dozer-app/Cargo.toml` 的 `[dependencies]` 块（约第 14 行 `serde_json.workspace = true` 之后）加一行：

```toml
serde.workspace = true
```

- [ ] **Step 2: 写失败测试**

创建 `crates/dozer-app/src/layout.rs`：

```rust
//! 四栏宽度布局的本地持久化——应用级偏好，不属于任何项目/窗口。
//! `load()`/`save()` 是真实调用方用的入口（固定读写
//! `dozer_core::paths::config_dir()/layout.json`）；`load_from`/`save_to`
//! 接收显式路径，供单测指向临时文件，不碰用户真实配置目录。

use crate::workspace::PanelLayout;
use std::path::{Path, PathBuf};

pub fn default_path() -> PathBuf {
    dozer_core::paths::config_dir().join("layout.json")
}

pub fn load() -> PanelLayout {
    load_from(&default_path())
}

pub fn save(layout: &PanelLayout) -> std::io::Result<()> {
    save_to(&default_path(), layout)
}

fn load_from(path: &Path) -> PanelLayout {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_to(path: &Path, layout: &PanelLayout) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let s = serde_json::to_string_pretty(layout).unwrap_or_default();
    std::fs::write(path, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        assert_eq!(load_from(&path), PanelLayout::default());
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layout.json");
        std::fs::write(&path, "not valid json").unwrap();
        assert_eq!(load_from(&path), PanelLayout::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("layout.json");
        let layout = PanelLayout {
            project_col_width: 300.0,
            ai_col_width: 260.0,
            preview_ratio: 0.42,
        };
        save_to(&path, &layout).unwrap();
        assert_eq!(load_from(&path), layout);
    }
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p dozer-app --lib layout::` — Expected: FAIL（`mod layout` 还没在 `main.rs` 声明，模块不存在）。

- [ ] **Step 4: 声明模块 + 接线**

在 `crates/dozer-app/src/main.rs` 顶部 `mod` 列表（约第 5-6 行 `mod goal;`/`mod keymap;` 之间，保持字母序）插入：

```rust
mod layout;
```

在 `crates/dozer-app/src/workspace.rs` 顶部 `use crate::goal::{self, Goal};` 附近新增：

```rust
use crate::layout;
```

把 `bootstrap`/`with_daemon_error` 两处构造里的 `layout: PanelLayout::default(),` 都改为：

```rust
            layout: layout::load(),
```

把 `update()` 里 Task 2 写的：

```rust
            Message::ColumnDragEnd => {
                self.dragging = None;
            }
```

改为：

```rust
            Message::ColumnDragEnd => {
                self.dragging = None;
                let layout = self.layout;
                self.handle.spawn(async move {
                    if let Err(e) = layout::save(&layout) {
                        tracing::warn!("四栏布局写盘失败: {e}");
                    }
                });
            }
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p dozer-app` — Expected: 全绿，含 `layout::tests` 三个新测试。

- [ ] **Step 6: 编译 + clippy + fmt**

Run: `cargo build -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 编译通过 / clean / 无输出。

- [ ] **Step 7: 真机目测（留用户）**

Run: `cargo run -p dozer-app`，退出后检查：

```bash
cat "$HOME/Library/Application Support/ai.byteboy.dozer/layout.json"
```

Expected（用户实机）：首次运行该文件不存在属正常（本任务还没接 main.rs 的拖拽续追踪，Task 4 之后才会真的产生非默认值并触发写盘）；文件若存在应是形如 `{"project_col_width":240.0,"ai_col_width":280.0,"preview_ratio":0.5}` 的合法 JSON。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/Cargo.toml crates/dozer-app/src/layout.rs crates/dozer-app/src/main.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(resizable-layout): 布局持久化(layout.rs) + 启动读盘/拖拽结束写盘接线"
```

---

### Task 4: `main.rs` 拖拽连续追踪

**Files:**
- Modify: `crates/dozer-app/src/main.rs`（`on_window_event` 的 `CursorMoved` 分支扩展 + 新增 `MouseInput` 释放分支）

**Interfaces:**
- Consumes: Task 2 的 `Workspace::dragging_divider()`、`Message::ColumnDrag`/`ColumnDragEnd`。

- [ ] **Step 1: 扩展 `CursorMoved` 分支**

把 `crates/dozer-app/src/main.rs` 里 `on_window_event` 的（约 232-234 行）：

```rust
                WindowEvent::CursorMoved { position, .. } => {
                    *cursor_phys = *position;
                }
```

改为：

```rust
                WindowEvent::CursorMoved { position, .. } => {
                    *cursor_phys = *position;
                    if workspace.dragging_divider().is_some() {
                        let scale = window.scale_factor();
                        let logical_x = (cursor_phys.x / scale) as f32;
                        let window_width = (window.inner_size().width as f64 / scale) as f32;
                        workspace.update(Message::ColumnDrag {
                            window_width,
                            logical_x,
                        });
                        window.request_redraw();
                    }
                }
```

- [ ] **Step 2: 新增 `MouseInput` 释放分支**

紧邻同一个 `match event {` 块里的 `MouseInput { state: ElementState::Pressed, .. }` 分支（约 235-249 行）之后、`_ => {}` 之前，插入：

```rust
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: winit::event::MouseButton::Left,
                    ..
                } => {
                    if workspace.dragging_divider().is_some() {
                        workspace.update(Message::ColumnDragEnd);
                        window.request_redraw();
                    }
                }
```

- [ ] **Step 3: 编译 + 全量测试 + clippy + fmt**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 全部通过 / clean / 无输出。

- [ ] **Step 4: 真机目测（留用户，本任务的核心验收点）**

Run: `cargo run -p dozer-app`
Expected（用户实机）：
1. 拖动项目|预览分隔线：项目栏宽度跟手变化，拖到很窄时停在下限不再收缩、不崩溃。
2. 拖动预览|终端分隔线：两栏比例跟手变化，终端网格实时跟着重排（字符不错位）。
3. 拖动终端|AI 分隔线：AI 栏宽度跟手变化。
4. 松开鼠标后退出 app 重新打开：布局保持上次拖拽结果（读 `~/Library/Application Support/ai.byteboy.dozer/layout.json`）。
5. 已知非阻塞小瑕疵（设计阶段确认过、刻意不修）：点下分隔线的同一瞬间，可能顺带触发一次"点击处于预览/终端列"的焦点路由 blur（因为 `on_window_event` 的 `MouseInput{Pressed}` 分支对所有左键按下都无差别执行 `blur_inputs`/`pending_focus`，不特判是否点在分隔线上）——不影响拖拽本身，只是拖之前如果地址栏/意见框正在编辑，会连带退出编辑态。确认这个体验可接受即可，不必修。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/main.rs
git commit -m "feat(resizable-layout): main.rs 拖拽连续追踪(CursorMoved续传+释放结束)"
```

---

## 收尾（全 task 完成后）

- [ ] **回归 + 人工验收**：全量 `cargo test -p dozer-app` 绿、clippy/fmt 干净；真机对 Task 4 Step 4 的 5 条逐项确认。
- [ ] **落档**：验收通过后勾选本计划全部 box。
- [ ] **分支收尾**：`superpowers:finishing-a-development-branch` 合入 main（若在独立分支上开发）。

## 自检记录（写计划时）

- **Spec 覆盖**：设计文档 §1(PanelLayout)→Task1；§2(几何函数改签名)→Task1；§3(view渲染/MouseArea/为何不用on_move)→Task1+Task2；§4(main.rs续追踪)→Task4；§5(夹取边界)→Task2；§6(持久化)→Task3；错误处理/测试策略均在各 task 落实。无遗漏。
- **发现并修正设计文档未覆盖的细节**：分隔线本身要占 Row 里的实际像素（`DIVIDER_WIDTH=8`），若不从 `fill_width` 里扣除会导致 webview bounds/IME/命中测试与实际渲染错位 24px——设计文档原公式未计入，本计划已在 Task 1 全部几何函数里改正（`3.0 * DIVIDER_WIDTH` 项），并用 `divider_positions_account_for_divider_width` 测试锁定。
- **占位扫描**：无 TBD/TODO；Task 2 Step 1 的"修订"说明是给实现者的决策记录（为什么弃用需要构造 `Workspace` 的测试方案），不是未定内容——最终采用的纯函数测试版本已给出完整代码。
- **类型/签名一致性**：`PanelLayout`/`Divider` 在 Task 1 定义，Task 2/3/4 全部按同名同签名引用（`apply_column_drag(PanelLayout, Divider, f32, f32) -> PanelLayout`、`Workspace::layout() -> PanelLayout`、`Workspace::dragging_divider() -> Option<Divider>`、`layout::load()/save(&PanelLayout)`）未出现前后不一致。
- **风险**：`MouseArea::center_x`/`Space::new()` 等 API 若与本计划字面签名不符，按编译器提示适配（Global Constraints 已声明此惯例）；Task 2 的测试从"构造 Workspace"改为"纯函数直测"是本计划内已经做出的决策，不留给实现者选择，避免额外返工。
