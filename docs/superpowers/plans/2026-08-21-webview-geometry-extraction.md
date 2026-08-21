# 共享 webview 几何代码抽取 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `crates/dozer-app/src/app.rs` 里被 Files/Project/Web 三个挂了
wry webview 的面板共用的三个纯几何函数(`preview_content_bounds_for`/
`left_files_tree_bounds_for`/`is_in_preview_column`)搬进新建的
`crates/dozer-app/src/webview_geometry.rs`,不改变任何行为。这是 app.rs
巨石化拆分序列的第三个试点(继 Rail、Terminal 之后)。

**Architecture:** `webview_geometry.rs` 是纯函数模块(不进 `extensions/`
目录)。三个目标函数依赖的 7 个共享几何辅助(`pair_content_width`/
`pair_x0_and_width`/`maximized_box_x_range`/`maximized_box_height`/
`pair_columns`/`PairColumns` 结构体+4 字段)服务范围远超这三个函数,
**留在 `app.rs`**,只把可见性从私有放宽到 `pub(crate)`,供新模块跨模块
调用。

**Tech Stack:** Rust,无新增依赖。

**Spec:** `docs/superpowers/specs/2026-08-21-webview-geometry-extraction-design.md`

## Global Constraints

- **开发必须在独立分支/worktree 上进行**,不得直接提交到 `main`;完成后
  提请审阅,通过后再合并。
- **不改变任何函数的输入输出行为**——纯代码搬迁 + 可见性放宽,不允许
  顺手修任何几何公式,哪怕看起来像 bug。
- **不搬 7 个共享几何辅助本身**——只放宽可见性,函数体/结构体字段类型
  逐字不变。
- **每个 Task 结束时 `cargo build`(全 workspace,因为 `main.rs` 也要改)
  与 `cargo test -p dozer-app` 必须全绿**——这是纯移动+可见性重构,任何
  编译错误/测试失败都是这一步引入的回归,当场修好才能进入下一步。
- **行号仅供定位参考,不是权威**——本计划里给出的行号是撰写时(基于
  `main` commit `09157f9` 之前的代码状态,Terminal 试点合并之后)的快照。
  每个 Task 开工前先用计划里给出的 `grep -n` 锚点重新定位。

---

## 文件结构总览

- **新建** `crates/dozer-app/src/webview_geometry.rs`——三个目标函数
  + 15 个测试。
- **修改** `crates/dozer-app/src/main.rs`——加一行 `mod webview_geometry;`;
  1 处调用点(`app::is_in_preview_column` → `webview_geometry::
  is_in_preview_column`)。
- **修改** `crates/dozer-app/src/app.rs`——7 个共享辅助可见性放宽;
  删掉搬走的三个函数 + 15 个测试;5 处调用点(`ime_cursor_area` 2 处、
  `preview_desired` 2 处、`files_drop_target` 1 处)加
  `webview_geometry::` 前缀;顶部加 `use crate::webview_geometry;`。

---

## Task 1: 放宽 7 个共享几何辅助的可见性

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Produces(Task 2 依赖):
  - `pub(crate) struct PairColumns { pub(crate) list_x: f32, pub(crate) list_w: f32, pub(crate) content_x: f32, pub(crate) content_w: f32 }`
  - `pub(crate) fn pair_content_width(zone_width: f32) -> f32`
  - `pub(crate) fn pair_x0_and_width(side: Side, window_width: f32, state: &ShellState) -> (f32, f32)`
  - `pub(crate) fn maximized_box_x_range(window_width: f32) -> (f32, f32)`
  - `pub(crate) fn maximized_box_height(window_height: f32) -> f32`
  - `pub(crate) fn pair_columns(pair_w: f32, split: f32, mirrored: bool) -> PairColumns`
- Consumes: 无(纯可见性变更,不引入新依赖)。

这个 Task 只改可见性关键字,函数体一字不改。做完之后 `app.rs` 单独仍然
编译通过(这 7 个符号已经在 `app.rs` 内部被大量调用,放宽可见性不会产生
"未使用"警告)。

### Step 1: `PairColumns` 结构体 + 字段

- [ ] 用 `grep -n "^struct PairColumns"` 定位(约第 657 行),改成:

```rust
pub(crate) struct PairColumns {
    pub(crate) list_x: f32,
    pub(crate) list_w: f32,
    pub(crate) content_x: f32,
    pub(crate) content_w: f32,
}
```

（原文档注释不变,只加可见性关键字。）

### Step 2: `pair_content_width`

- [ ] 用 `grep -n "^fn pair_content_width"` 定位,签名行:

```rust
fn pair_content_width(zone_width: f32) -> f32 {
```

  改成:

```rust
pub(crate) fn pair_content_width(zone_width: f32) -> f32 {
```

  （函数体 `(zone_width - byteui::theme::geometry::divider_width()).max(0.0)` 不动。）

### Step 3: `pair_x0_and_width`

- [ ] 用 `grep -n "^fn pair_x0_and_width"` 定位,签名行:

```rust
fn pair_x0_and_width(side: Side, window_width: f32, state: &ShellState) -> (f32, f32) {
```

  改成:

```rust
pub(crate) fn pair_x0_and_width(side: Side, window_width: f32, state: &ShellState) -> (f32, f32) {
```

  （函数体不动。）

### Step 4: `pair_columns`

- [ ] 用 `grep -n "^fn pair_columns"` 定位,签名行:

```rust
fn pair_columns(pair_w: f32, split: f32, mirrored: bool) -> PairColumns {
```

  改成:

```rust
pub(crate) fn pair_columns(pair_w: f32, split: f32, mirrored: bool) -> PairColumns {
```

  （函数体不动。紧接着的 `#[cfg(test)] mod pair_columns_tests { .. }`
  测的是 `pair_columns` 本身,不是这次的目标函数,**不要搬**,原样留在
  `app.rs`。）

### Step 5: `maximized_box_x_range` / `maximized_box_height`

- [ ] 用 `grep -n "^fn maximized_box_x_range"` 定位,签名行:

```rust
fn maximized_box_x_range(window_width: f32) -> (f32, f32) {
```

  改成:

```rust
pub(crate) fn maximized_box_x_range(window_width: f32) -> (f32, f32) {
```

- [ ] 用 `grep -n "^fn maximized_box_height"` 定位,签名行:

```rust
fn maximized_box_height(window_height: f32) -> f32 {
```

  改成:

```rust
pub(crate) fn maximized_box_height(window_height: f32) -> f32 {
```

  （两个函数体都不动。）

  **注意**:`maximized_box_x_range` 的文档注释提到
  `terminal_pane_pixel_size` 也在用它——这个函数不搬,留在 `app.rs`
  继续用裸名 `maximized_box_x_range(..)` 调用(同模块内,不需要前缀),
  不受这次可见性放宽影响。

### Step 6: 验证 + 提交

- [ ] `cargo build -p dozer-app` 通过。
- [ ] `cargo test -p dozer-app` 全绿,测试数量与改动前一致(纯可见性
  放宽不改变任何测试)。
- [ ] `cargo clippy -p dozer-app --all-targets` 无新增警告。
- [ ] `cargo fmt`。
- [ ] 提交:

```bash
git add crates/dozer-app/src/app.rs
git commit -m "chore(dozer-app): 放宽 7 个共享几何辅助可见性为 pub(crate)"
```

---

## Task 2: 搬迁三个目标函数 + 15 个测试到 webview_geometry.rs

**Files:**
- Create: `crates/dozer-app/src/webview_geometry.rs`
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes: Task 1 产出的 7 个 `pub(crate)` 辅助;`crate::app::{PanelKind,
  Side, ShellState, MaximizedPane}`。
- Produces:
  - `pub fn webview_geometry::preview_content_bounds_for(side: Side, window_width: f32, window_height: f32, state: &ShellState) -> (f32, f32, f32, f32)`
  - `pub fn webview_geometry::left_files_tree_bounds_for(side: Side, window_width: f32, window_height: f32, state: &ShellState) -> (f32, f32, f32, f32)`
  - `pub fn webview_geometry::is_in_preview_column(x: f32, window_width: f32, state: &ShellState) -> Option<PanelKind>`

### Step 1: 新建 `webview_geometry.rs`,写入三个目标函数

- [ ] 创建 `crates/dozer-app/src/webview_geometry.rs`:

```rust
// crates/dozer-app/src/webview_geometry.rs
//! Files/Project/Web 三个挂了原生 wry webview 的面板共用的几何计算:
//! webview 矩形(`preview_content_bounds_for`)、文件树列表列矩形
//! (`left_files_tree_bounds_for`)、点击命中判断(`is_in_preview_column`)。
//! 跟"文件预览"业务域本身无关,是历史命名遗留——真正的文件预览/编辑器
//! 业务(`preview_open_path` 等)留在 `app.rs`,同 `rail.rs` 里
//! `panel_select`/`terminal.rs` 里 `term_input` 留在内核的理由一致。
//!
//! 依赖的配对列宽公式(`pair_content_width`/`pair_x0_and_width`/
//! `pair_columns`/`PairColumns`/`maximized_box_x_range`/
//! `maximized_box_height`)服务范围远超这三个函数,留在 `app.rs`,只放宽
//! 了可见性。见
//! `docs/superpowers/specs/2026-08-21-webview-geometry-extraction-design.md`。

use crate::app::{App as _, MaximizedPane, PanelKind, Side, ShellState};
use crate::theme;
```

  （`App as _` 这一行先占位——如果编译时发现三个函数体根本不引用 `App`
  类型本身,删掉这行;下面 Step 5 编译验证时按报错核实。）

- [ ] 用 `grep -n "pub fn preview_content_bounds_for"` 在 `app.rs` 里
  定位起点(往上数到紧邻它的空行,函数本身没有文档注释),终点是
  `grep -n "pub fn left_files_tree_bounds_for"` 命中行的前一行。把这段
  整体剪切、追加到 `webview_geometry.rs`:

```rust
pub fn preview_content_bounds_for(
    side: Side,
    window_width: f32,
    window_height: f32,
    state: &ShellState,
) -> (f32, f32, f32, f32) {
    let collapsed = match side {
        Side::Left => state.left_collapsed,
        Side::Right => state.right_collapsed,
    };
    if collapsed {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let kind = match side {
        Side::Left => state.left_view,
        Side::Right => state.right_view,
    };
    let mirrored = state.layout.rail_layout.side_of(kind) != kind.default_side();
    if let Some(maximized) = state.maximized {
        let (x0, avail_w) = maximized_box_x_range(window_width);
        let y0 = byteui::theme::geometry::top_bar_height()
            + byteui::theme::geometry::maximize_overlay_padding();
        let showing_side = match maximized {
            MaximizedPane::Left => Side::Left,
            MaximizedPane::Right => Side::Right,
        };
        if side != showing_side {
            return (0.0, 0.0, 0.0, 0.0);
        }
        let avail_h = (maximized_box_height(window_height)
            - byteui::theme::geometry::status_bar_height())
        .max(0.0);
        return match kind {
            PanelKind::Files => {
                let y = y0 + byteui::theme::geometry::preview_chrome_top_px();
                let h = (avail_h - byteui::theme::geometry::preview_chrome_top_px() - 8.0).max(0.0);
                let pair_w = pair_content_width(avail_w);
                let cols = pair_columns(pair_w, state.dims.files_split, mirrored);
                let x = x0 + cols.content_x + 8.0;
                let w = (cols.content_w - 16.0).max(0.0);
                (x, y, w, h)
            }
            PanelKind::Web => {
                let y = y0 + byteui::theme::geometry::browser_chrome_top_px();
                let h = (avail_h - byteui::theme::geometry::browser_chrome_top_px() - 8.0).max(0.0);
                let x = x0 + 8.0;
                let w = (avail_w - 16.0).max(0.0);
                (x, y, w, h)
            }
            PanelKind::GitLog
            | PanelKind::Todo => (0.0, 0.0, 0.0, 0.0),
            PanelKind::Project => {
                let y = y0 + byteui::theme::geometry::preview_chrome_top_px();
                let h = (avail_h - byteui::theme::geometry::preview_chrome_top_px() - 8.0).max(0.0);
                let pair_w = pair_content_width(avail_w);
                let cols = pair_columns(pair_w, state.dims.project_split, mirrored);
                let x = x0 + cols.content_x + 8.0;
                let w = (cols.content_w - 16.0).max(0.0);
                (x, y, w, h)
            }
            PanelKind::Database
            | PanelKind::Ssh
            | PanelKind::Agent
            | PanelKind::Conversations
            | PanelKind::Usage
            | PanelKind::Acceptance => (0.0, 0.0, 0.0, 0.0),
        };
    }
    let (zone_x0, zone_w) = pair_x0_and_width(side, window_width, state);
    let m = match side {
        Side::Left => theme::region::left_zone().margin,
        Side::Right => theme::region::right_zone().margin,
    };
    let y_top =
        |chrome_top: f32| -> f32 { byteui::theme::geometry::top_bar_height() + m.top + chrome_top };
    let h_for = |y: f32| -> f32 {
        (window_height - y - m.bottom - byteui::theme::geometry::status_bar_height()).max(0.0)
    };
    match kind {
        PanelKind::Files => {
            let y = y_top(byteui::theme::geometry::preview_chrome_top_px());
            let h = h_for(y);
            let cols = pair_columns(zone_w, state.dims.files_split, mirrored);
            let x = zone_x0 + cols.content_x + 8.0 + m.left;
            let w = (cols.content_w - 16.0 - m.left - m.right).max(0.0);
            (x, y, w, h)
        }
        PanelKind::Web => {
            let y = y_top(byteui::theme::geometry::browser_chrome_top_px());
            let h = h_for(y);
            let zone_raw_w = match side {
                Side::Left => left_zone_width(window_width, state),
                Side::Right => right_zone_width(window_width, state),
            };
            let (x, w) = if state.browser_bookmarks_open {
                let cols = pair_columns(
                    pair_content_width(zone_raw_w),
                    1.0 - state.dims.browser_bookmarks_split,
                    !mirrored,
                );
                let x = zone_x0 + cols.content_x + 8.0 + m.left;
                let w = (cols.content_w - 16.0 - m.left - m.right).max(0.0);
                (x, w)
            } else {
                let x = zone_x0 + 8.0 + m.left;
                let w = (zone_raw_w - 16.0 - m.left - m.right).max(0.0);
                (x, w)
            };
            (x, y, w, h)
        }
        PanelKind::GitLog => (0.0, 0.0, 0.0, 0.0),
        PanelKind::Todo => (0.0, 0.0, 0.0, 0.0),
        PanelKind::Project => {
            let y = y_top(byteui::theme::geometry::preview_chrome_top_px());
            let h = h_for(y);
            let cols = pair_columns(zone_w, state.dims.project_split, mirrored);
            let x = zone_x0 + cols.content_x + 8.0 + m.left;
            let w = (cols.content_w - 16.0 - m.left - m.right).max(0.0);
            (x, y, w, h)
        }
        PanelKind::Database => (0.0, 0.0, 0.0, 0.0),
        PanelKind::Ssh => (0.0, 0.0, 0.0, 0.0),
        PanelKind::Agent | PanelKind::Conversations | PanelKind::Usage | PanelKind::Acceptance => {
            (0.0, 0.0, 0.0, 0.0)
        }
    }
}
```

  （原有文档注释所在处只有一行"/// 终端相关"之类的邻居注释不属于这个
  函数本身——`preview_content_bounds_for` 在 `app.rs` 里现状没有自己的
  文档注释,搬迁时不用补。原样保留代码里所有中文行内注释。）

### Step 2: 剪切 `left_files_tree_bounds_for`

- [ ] 用 `grep -n "pub fn left_files_tree_bounds_for"` 命中行往上数到
  紧邻的文档注释首行("/// `side` 这一侧文件树的**目录列表 Scrollable**
  ...")。终点是 `grep -n "pub fn is_in_preview_column"` 命中行的前一行。
  整段剪切追加到 `webview_geometry.rs`:

```rust
/// `side` 这一侧文件树的**目录列表 Scrollable** 在窗口坐标系里的矩形
/// (上/左/宽/高,逻辑像素),供 main.rs 做外部文件拖拽命中测试。返回的
/// 矩形只覆盖列表视口本身——命中测试据此把窗口 Y 换算成 `tree_scroll`
/// 偏移下的"可见行序号",再推出那行是不是目录。
///
/// 与 `preview_content_bounds_for` 同源(外层)但其目标是**配对里 list 那一
/// 栏**(树),不是 webview 的 content 列,所以横向起点用 `pair_columns`
/// 的 `list_x`(mirrored 时在 content 之后)、纵向起点换用
/// `tree_chrome_top_px`(面板头+搜索/工具栏),底部扣 `git 脚注栏` 而非
/// footbar 专用常量。
///
/// 不可命中(该侧收起 / 不是 Files / 放大的是另一侧)时返回零尺寸矩形。
/// 该侧被放大(`MaximizedPane` 对应该侧)按 `maximize_overlay` 的实际盒子
/// 换算。
pub fn left_files_tree_bounds_for(
    side: Side,
    window_width: f32,
    window_height: f32,
    state: &ShellState,
) -> (f32, f32, f32, f32) {
    let zero = || (0.0, 0.0, 0.0, 0.0);
    let kind = match side {
        Side::Left => state.left_view,
        Side::Right => state.right_view,
    };
    let collapsed = match side {
        Side::Left => state.left_collapsed,
        Side::Right => state.right_collapsed,
    };
    if collapsed || kind != PanelKind::Files {
        return zero();
    }
    let m = match side {
        Side::Left => theme::region::left_zone().margin,
        Side::Right => theme::region::right_zone().margin,
    };
    let p = theme::region::project_pane();
    let mirrored =
        state.layout.rail_layout.side_of(PanelKind::Files) != PanelKind::Files.default_side();
    if let Some(maximized) = state.maximized {
        let showing_side = match maximized {
            MaximizedPane::Left => Side::Left,
            MaximizedPane::Right => Side::Right,
        };
        if side != showing_side {
            return zero();
        }
        let (x0, avail_w) = maximized_box_x_range(window_width);
        let y_top = byteui::theme::geometry::top_bar_height()
            + byteui::theme::geometry::maximize_overlay_padding();
        let avail_h = (maximized_box_height(window_height)
            - byteui::theme::geometry::status_bar_height())
        .max(0.0);
        let pair_w = pair_content_width(avail_w);
        let cols = pair_columns(pair_w, state.dims.files_split, mirrored);
        let x = if mirrored {
            x0 + cols.list_x + byteui::theme::geometry::divider_width() + m.left + p.padding.left
        } else {
            x0 + cols.list_x + m.left + p.padding.left
        };
        let w = (cols.list_w - p.padding.left - p.padding.right).max(0.0);
        let y = y_top + m.top + p.padding.top + theme::geometry::tree_chrome_top_px();
        let h = (avail_h
            - m.top
            - m.bottom
            - p.padding.top
            - p.padding.bottom
            - theme::geometry::tree_chrome_top_px()
            - theme::geometry::tree_chrome_bottom_px())
        .max(0.0);
        return (x, y, w, h);
    }
    let (zone_x0, zone_w) = pair_x0_and_width(side, window_width, state);
    let cols = pair_columns(zone_w, state.dims.files_split, mirrored);
    let x = if mirrored {
        zone_x0 + cols.list_x + byteui::theme::geometry::divider_width() + m.left + p.padding.left
    } else {
        zone_x0 + cols.list_x + m.left + p.padding.left
    };
    let w = (cols.list_w - p.padding.left - p.padding.right).max(0.0);
    let y_pane = byteui::theme::geometry::top_bar_height() + m.top;
    let y = y_pane + p.padding.top + theme::geometry::tree_chrome_top_px();
    let h = ((window_height - m.bottom - byteui::theme::geometry::status_bar_height())
        - (y_pane + p.padding.top + theme::geometry::tree_chrome_top_px())
        - p.padding.bottom
        - theme::geometry::tree_chrome_bottom_px())
    .max(0.0);
    (x, y, w, h)
}
```

### Step 3: 剪切 `is_in_preview_column`

- [ ] 用 `grep -n "pub fn is_in_preview_column"` 命中行往上数到紧邻的
  文档注释首行("/// 逻辑 x 是否落在左侧文件预览内容区列内...")。终点是
  下一个 `pub fn zone_at_x` 命中行的前一行。整段剪切追加到
  `webview_geometry.rs`:

```rust
/// 逻辑 x 是否落在左侧文件预览内容区列内。焦点路由用:点击落在
/// 该列 → 键盘交给 webview;落在别处 → 交回窗口(终端)。
///
/// 放大态(Task 5):右侧被放大时左侧内容不可见,恒不落在预览列;左侧被
/// 放大时按 `maximize_overlay` 实际渲染的更大盒子重新换算横向范围。
/// 逻辑 x 是否落在某一侧的预览列内,是则返回命中的面板种类;焦点路由
/// (`main.rs`)据此决定把键盘交给哪个 webview 池(`Web` → 浏览器池,
/// `Files`/`Project` → 预览池)、以及 `active_preview_webview_id` 该查
/// `ws.preview` 还是 `ws.project_preview`。
///
/// 2026-08-19 Stage 4a 审阅后修订:此前只查 `state.left_view`,`Project`
/// 挪到右栏后点击其预览列不会被识别;现在左右两侧各自独立判断。
///
/// `Web` 分支保留原有近似(整个 zone 都算预览列,不细分收藏夹展开时的
/// 精确切分——延续现状)。
pub fn is_in_preview_column(x: f32, window_width: f32, state: &ShellState) -> Option<PanelKind> {
    for side in [Side::Left, Side::Right] {
        let collapsed = match side {
            Side::Left => state.left_collapsed,
            Side::Right => state.right_collapsed,
        };
        if collapsed {
            continue;
        }
        let kind = match side {
            Side::Left => state.left_view,
            Side::Right => state.right_view,
        };
        if let Some(maximized) = state.maximized {
            let showing_side = match maximized {
                MaximizedPane::Left => Side::Left,
                MaximizedPane::Right => Side::Right,
            };
            if side != showing_side {
                continue;
            }
            let (x0, avail_w) = maximized_box_x_range(window_width);
            let mirrored = state.layout.rail_layout.side_of(kind) != kind.default_side();
            let hit = match kind {
                PanelKind::Files => {
                    let cols = pair_columns(
                        pair_content_width(avail_w),
                        state.dims.files_split,
                        mirrored,
                    );
                    x >= x0 + cols.content_x && x < x0 + cols.content_x + cols.content_w
                }
                PanelKind::Web => x >= x0 && x < x0 + avail_w,
                PanelKind::Project => {
                    let cols = pair_columns(
                        pair_content_width(avail_w),
                        state.dims.project_split,
                        mirrored,
                    );
                    x >= x0 + cols.content_x && x < x0 + cols.content_x + cols.content_w
                }
                _ => false,
            };
            if hit {
                return Some(kind);
            }
            continue;
        }
        let (zone_x0, zone_w) = pair_x0_and_width(side, window_width, state);
        let mirrored = state.layout.rail_layout.side_of(kind) != kind.default_side();
        let hit = match kind {
            PanelKind::Files => {
                let cols = pair_columns(zone_w, state.dims.files_split, mirrored);
                x >= zone_x0 + cols.content_x && x < zone_x0 + cols.content_x + cols.content_w
            }
            PanelKind::Web => x >= zone_x0 && x < zone_x0 + zone_w,
            PanelKind::Project => {
                let cols = pair_columns(zone_w, state.dims.project_split, mirrored);
                x >= zone_x0 + cols.content_x && x < zone_x0 + cols.content_x + cols.content_w
            }
            _ => false,
        };
        if hit {
            return Some(kind);
        }
    }
    None
}
```

### Step 4: 修正 `webview_geometry.rs` 顶部 import

- [ ] 三个函数体实际只用到 `App` 类型吗?检查一遍——三者签名都是
  `(.., state: &ShellState) -> ..`,函数体内不出现 `App`/`self`。把
  Step 1 里占位的 `use crate::app::{App as _, ..}` 改成:

```rust
use crate::app::{MaximizedPane, PanelKind, Side, ShellState};
use crate::theme;
```

  （`left_zone_width`/`right_zone_width` 已经是 `pub(crate)`,同样需要
  加进这行 `use`:最终应为
  `use crate::app::{MaximizedPane, PanelKind, Side, ShellState, left_zone_width, maximized_box_height, maximized_box_x_range, pair_columns, pair_content_width, pair_x0_and_width, right_zone_width};`
  ——`PairColumns` 类型名不需要单独导入,因为三个函数只用 `pair_columns()`
  返回值的字段(`.content_x` 等),不直接书写 `PairColumns` 这个类型名。
  生产代码(三个目标函数本身)的依赖到此为止;Step 9 搬迁测试时还要
  再加 `ShellLayout`/`PanelDims`/`rail`,那部分依赖单独在 Step 9 处理,
  不要提前塞进这里——生产代码和测试代码的依赖面不同。）

### Step 5: `app.rs` 删除已搬迁的三个函数

- [ ] 从 `app.rs` 里删除 Step 1-3 剪切的三段代码(含各自的文档注释),
  原地留空,不留多余空行(交给 Step 8 的 `cargo fmt` 收尾)。

### Step 6: `app.rs` 5 处调用点加前缀

- [ ] `app.rs` 顶部 `use` 区块,`use crate::transcript::ReviewEntry;`
  与 `use crate::workspace::{..}` 之间加一行 `use crate::webview_geometry;`。

- [ ] `ime_cursor_area` 方法内两处:

```rust
let (bx, by, _bw, _bh) = preview_content_bounds_for(side, window_w, window_h, &state);
```

  各改成:

```rust
let (bx, by, _bw, _bh) = webview_geometry::preview_content_bounds_for(side, window_w, window_h, &state);
```

  （`browser_addr_focused` 分支与 `comment_focused` 分支各一处,`grep -n
  "preview_content_bounds_for(side, window_w, window_h, &state)"` 定位
  两处后逐一改。）

- [ ] `preview_desired` 方法内两处:

```rust
let bounds =
    preview_content_bounds_for(side, window_width, window_height, &self.shell_state());
```

  各改成:

```rust
let bounds =
    webview_geometry::preview_content_bounds_for(side, window_width, window_height, &self.shell_state());
```

  （一处在遍历 Files/Project/Web 的分支里,一处在浏览器 `desired_webviews`
  分支里,`grep -n "preview_content_bounds_for(side, window_width, window_height"`
  定位两处后逐一改。）

- [ ] `files_drop_target` 方法内一处:

```rust
let bounds = left_files_tree_bounds_for(side, window_w, window_h, &self.shell_state());
```

  改成:

```rust
let bounds = webview_geometry::left_files_tree_bounds_for(side, window_w, window_h, &self.shell_state());
```

### Step 7: `main.rs` 1 处调用点 + 模块注册

- [ ] `main.rs` 的 `mod` 列表按字母序插入(`mod transcript;` 与 `mod
  workspace;` 之间):

```rust
mod transcript;
mod webview_geometry;
mod workspace;
```

- [ ] 用 `grep -n "app::is_in_preview_column"` 定位调用点,改成:

```rust
let intent = match webview_geometry::is_in_preview_column(logical_x, logical_w, &state) {
```

### Step 8: 搬迁 15 个既有测试

在 `webview_geometry.rs` 末尾新增:

```rust
#[cfg(test)]
mod tests {
    use super::*;
```

- [ ] 用 `grep -n "fn preview_content_bounds_is_inside_left_content_column"`
  起,到 `fn is_in_preview_column_false_for_right_panel_on_left` 那个
  测试函数结束(闭合大括号,紧接着下一个 `#[test] fn
  terminal_pane_height_excludes_top_and_status_bars` 前一行为止),把这
  **11 个连续测试**整段剪切进上面的 `mod tests`:
  `preview_content_bounds_is_inside_left_content_column`/
  `preview_content_bounds_web_view_spans_whole_left_zone`/
  `preview_content_bounds_web_view_shrinks_when_bookmarks_open`/
  `preview_content_bounds_web_view_mirrored_bookmarks_content_follows_render_order`/
  `preview_content_bounds_zero_when_left_collapsed`/
  `preview_content_bounds_zero_when_right_maximized`/
  `preview_content_bounds_left_maximized_files_matches_overlay_geometry`/
  `preview_content_bounds_left_maximized_web_spans_whole_overlay_box`/
  `preview_column_hit_test_respects_maximized_state`/
  `preview_content_bounds_bare_for_right_panel_on_left`/
  `is_in_preview_column_false_for_right_panel_on_left`。

  **`test_state()` 辅助函数**(这 11 个测试大量依赖它构造 `ShellState`)
  **不要跟着搬**——它是 `app.rs` 测试模块通用的夹具,被本次不搬的其他
  测试(如 `terminal_pane_height_excludes_top_and_status_bars`)也在用。
  `webview_geometry.rs` 的测试模块需要自己一份等价夹具,见下面 Step 9。

- [ ] 用 `grep -n "fn preview_content_bounds_never_negative"` 起,到
  `fn is_in_preview_column_returns_project_when_project_on_right` 那个
  测试函数结束(紧接着下一个 `#[test] fn
  zone_at_x_splits_left_and_right_at_zone_boundary` 前一行为止),把这
  **4 个连续测试**整段剪切进 `mod tests`:
  `preview_content_bounds_never_negative`/`preview_column_hit_test`/
  `preview_column_hit_test_left_collapsed_is_never_hit`/
  `is_in_preview_column_returns_project_when_project_on_right`。

  已核实这两个测试函数体只调用 `is_in_preview_column`/`test_state()`,
  不引用 `zone_at_x` 或任何其他不搬迁的符号,可以直接整体剪切,不需要
  额外处理。

  关闭 `webview_geometry.rs` 的 `mod tests { .. }` 大括号。

### Step 9: 测试夹具 `test_state()`

- [ ] 检查搬迁进来的 15 个测试对 `test_state()` 的依赖:该函数在
  `app.rs` 定义为(`grep -n "fn test_state"` 定位):

```rust
fn test_state() -> ShellState {
    ShellState {
        layout: ShellLayout::default(),
        dims: PanelDims::default(),
        left_view: PanelKind::Files,
        left_collapsed: false,
        right_view: PanelKind::Agent,
        right_collapsed: false,
        browser_bookmarks_open: false,
        maximized: None,
    }
}
```

  在 `webview_geometry.rs` 的 `mod tests` 内新增一份同名同实现的私有
  拷贝(不要尝试把 `app.rs` 的 `test_state` 标 `pub(crate)` 共享——它是
  测试专用夹具,拷贝一份比跨模块暴露测试夹具更清爽,且只有 4 行,重复
  成本可忽略)。

  `test_state()` 拷贝需要 `ShellLayout`/`PanelDims`,4-test 块里的
  `is_in_preview_column_returns_project_when_project_on_right` 还额外
  用到 `rail::RailLayout`——这些都不在 Step 4 生产代码的 `use` 列表里
  (生产代码不需要它们),在 `mod tests` 块顶部(`use super::*;` 那一行
  之后)单独加:

```rust
use crate::app::{PanelDims, ShellLayout};
use crate::rail;
```

### Step 10: 编译修补 + 验证 + 提交

- [ ] `cargo build 2>&1 | head -150`(全 workspace,因为 `main.rs` 改了),
  按报错逐条修:多半是 `webview_geometry.rs` 缺 `use`(核对 Step 4 列的
  导入清单是否齐全)、或 `app.rs`/`main.rs` 里还有遗漏的裸
  `preview_content_bounds_for(`/`left_files_tree_bounds_for(`/
  `is_in_preview_column(` 调用没加 `webview_geometry::` 前缀(用
  `grep -rn "preview_content_bounds_for(\|left_files_tree_bounds_for(\|is_in_preview_column("
  crates/dozer-app/src --include="*.rs"` 全量核对,排除
  `webview_geometry.rs` 内部自己的调用——那些不需要前缀)。
- [ ] 反复修到 `cargo build` 干净通过。
- [ ] `cargo test -p dozer-app` 全绿,确认能看到 15 个搬迁测试的名字。
- [ ] `cargo clippy --all-targets` 无新增警告。
- [ ] `cargo fmt`。
- [ ] 提交:

```bash
git add crates/dozer-app/src/webview_geometry.rs crates/dozer-app/src/app.rs \
  crates/dozer-app/src/main.rs
git commit -m "refactor(dozer-app): 共享 webview 几何函数搬迁到 webview_geometry.rs"
```

---

## Task 3: 全工作区验证 + 人工 GUI 验收

**Files:** 无代码改动,仅验证。

- [ ] `cargo build`(全 workspace)。
- [ ] `cargo test`(全 workspace)。
- [ ] `cargo clippy --all-targets`(全 workspace)。
- [ ] `cargo fmt --check`(全 workspace)。
- [ ] `wc -l crates/dozer-app/src/app.rs crates/dozer-app/src/webview_geometry.rs`,
  确认 `app.rs` 行数比 Task 1 开工前减少约 360-400 行(三个函数 + 文档
  注释)、`webview_geometry.rs` 落地约 700 行(函数 + 测试)。
- [ ] `cargo run -p dozer-app` 启动 GUI,人工过一遍(对照 spec"测试策略"
  章节):
  - 切换 Files/Project/Web 三个挂 webview 的面板,webview 位置/尺寸
    正常(非放大、放大左/右、镜像态、Web 收藏夹展开/收起)。
  - 拖拽 Files/Project 面板到对侧栏(镜像态)后 webview 仍摆位正确。
  - 文件树列表列的外部文件拖拽命中(拖一个 Finder 文件到文件树目录行)
    仍能落进正确的目录。
  - 浏览器地址栏获得焦点、验收面板评论框获得焦点时,IME 候选窗(如果
    有中文输入法测试条件)位置正常(间接依赖
    `preview_content_bounds_for`)。
  - 点击预览内容列 vs 点击文件树列表列,焦点(键盘去向)正确路由。
- [ ] 全部通过后,把 Task 1-2 的分支提请代码审阅
  (`superpowers:requesting-code-review`),审阅通过后合并回 `main`。
