# 图标栏拖拽换栏 · 光标跟随幽灵图标 + 跨栏落点高亮 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 图标栏拖拽换栏(同栏重排 / 跨栏移动)期间,被拖动的那颗图标的
视觉表现改成跟随鼠标光标实时移动(同 OS 拖文件夹的观感),源位置图标
变淡,跨栏悬停时目标栏加一条金色插入线标出"松手会落在这"。

**Architecture:** 复用既有 `App.rail_drag: Option<RailDrag>` 状态机(不
改其字段、不改 `rail_drag_move_into`/`rail_cross_apply`/`end_rail_drag`
任何一行逻辑)与 `App::last_cursor`(每次 `CursorMoved` 已经更新、且
main.rs 已无条件 `window.request_redraw()`,幽灵图标据此天然每帧跟手,
不需要新增任何 main.rs 事件循环改动)。新增三块纯视觉:①
`byteui::interaction::icons::icon_button_entry` 加一个 `dim` 参数,给
正在被拖的源图标"变淡";② 新增 `app.rs::rail_drag_ghost` 视图函数,
叠进 `App::view` 最外层 `stack!`,用 `App::last_cursor` 定位;③
`icon_rail` 渲染循环里,`pending_cross_side` 命中当前栏时插一条金色
插入线。全程不产生新的 `Message` variant、不改变任何拖拽提交逻辑,是
纯渲染层扩展。

**Tech Stack:** Rust 2024,iced 0.14(`iced_widget`/`iced_renderer`)。

**Spec:** `docs/superpowers/specs/2026-08-19-rail-panel-drag-relocation-design.md`
(本计划是对该 spec 已完工的 Stage 1-4b 的一个纯视觉加强;不新增/不改写
该 spec 的任何"目标/非目标"条款,只是让"栏清空拒绝时 ghost 弹回原位"
[spec 第 84-85 行]里提到的"ghost"从一个抽象说法变成真的有一个跟手的
浮动图标)。

## Global Constraints

- **前提:rail-panel-relocation Stage 1-4b 均已合并到 main。** 开工前确认
  `App.rail_drag`/`RailDrag`/`rail_drag_move_into`/`rail_cross_apply`/
  `App::dragging_rail`/`icon_rail`/`panel_meta`/`App::last_cursor`/
  `App::window_size` 均已存在——这些都是本计划直接复用、不重新实现的
  既有基础设施。
- **独立分支开发,不直接提交 main。** 在新分支
  `feature/rail-drag-cursor-ghost` 上完成全部 6 个 Task,提请审阅、通过
  后再合并回 `main`。每次 `git commit`/`git add` 前先跑
  `git branch --show-current` 确认不在 `main` 上;开工前 `git status`
  确认干净,发现不相关的未提交改动(本仓库有长驻自动化在 main 上开发,
  见项目记忆)只 `git stash push -- <具体路径>` 那些具体路径,不要用
  `-u`。
- **图标按钮优先复用统一组件**(`icons::icon_button_entry`)——本计划
  唯一的例外是 Task 4 的幽灵图标本体:它没有 hover/点击交互,直接用
  `icons::view` + 手写 `container` 样式,不套 `icon_button_entry`(理由
  见 Task 4 开头)。
- **每个 Task 结束都要求 `cargo build -p byteui -p dozer-app` 通过。**
- **这是原生 iced GUI,不是浏览器页面**——Task 6 之外的每个 Task 都不要求
  手工跑 GUI 验证,但 Task 6 必须跑 `cargo run -p dozer-app` 按文档化的
  清单手动验证一遍(自动化测试覆盖不到"图标是否真的跟手"这类像素级视觉
  行为)。

---

## Task 1: `icon_button_entry` 新增 `dim` 参数(byteui)

**Files:**
- Modify: `crates/byteui/src/interaction/icons.rs`
- Modify (机械同步,新增参数处一律传 `false`,行为不变):
  - `crates/dozer-app/src/app.rs:7581`(`icon_rail` 内的调用——这次先传
    `false`,Task 3 再改成真值)
  - `crates/dozer-app/src/extensions/database.rs:1470`
  - `crates/dozer-app/src/extensions/usage.rs:404`
  - `crates/dozer-app/src/extensions/ssh.rs:749`
  - `crates/dozer-app/src/extensions/browser.rs`(3 处:`nav_button`/
    `star_button`/`bookmarks_toggle_button`,具体行号见下方——本计划写
    作时这个文件有一份不相关的并发未提交改动在挪函数位置,执行前以
    `grep -n "icon_button_entry(" crates/dozer-app/src/extensions/browser.rs`
    重新定位,不要死认行号)
  - `crates/dozer-app/src/extensions/files.rs`(2 处:`search_button`/
    `dotfiles_button` 用的公共 `box_len` 调用块 + `switch` 分支切换按钮)
- Test: `crates/byteui/src/interaction/icons.rs`(同文件内 `#[cfg(test)]
  mod tests`)

**Interfaces:**
- Produces: `dimmed(color: iced_widget::core::Color, dim: bool) ->
  iced_widget::core::Color`(私有纯函数,`dim` 为真时把 `color.a` 砍到
  `DRAG_DIM_ALPHA`,否则原样返回)。
- Produces: `icon_button_entry` 新签名——在原 `active: bool` 参数**之后**
  插入一个新参数 `dim: bool`:
  `icon_button_entry(kind, size, active, dim, hover_t, card, button_size,
  interactive, on_select, on_hover, tooltip)`。后续 Task 2/3 的调用方都
  按这个新顺序传参。

- [x] **Step 1: 写 `dimmed()` 的失败测试**

在 `crates/byteui/src/interaction/icons.rs` 的 `mod tests` 块内追加:

```rust
    #[test]
    fn dimmed_scales_alpha_when_true() {
        let c = Color::from_rgb(1.0, 0.5, 0.2);
        let d = dimmed(c, true);
        assert_eq!(d.a, DRAG_DIM_ALPHA);
        assert_eq!((d.r, d.g, d.b), (c.r, c.g, c.b), "只改透明度,不改色相");
    }

    #[test]
    fn dimmed_passthrough_when_false() {
        let c = Color::from_rgb(1.0, 0.5, 0.2);
        assert_eq!(dimmed(c, false), c);
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p byteui dimmed -- --nocapture`
Expected: 编译失败(`dimmed`/`DRAG_DIM_ALPHA` 未定义)。

- [x] **Step 3: 实现 `dimmed()` 与常量**

在 `icon_button_entry` 函数定义**之前**(紧挨着它的文档注释上方)插入:

```rust
/// 图标栏按住拖拽期间,源位置图标"变淡"的透明度系数——同色但整体透明度
/// 砍到 40%,视觉语言对齐 OS 拖文件夹时源图标半透明的既有认知。
const DRAG_DIM_ALPHA: f32 = 0.4;

/// `dim` 为真时把 `color` 的透明度砍到 `DRAG_DIM_ALPHA`,否则原样返回。
/// 纯颜色计算,不碰渲染——供 `icon_button_entry` 内部给图标色/卡片底色/
/// 选中框色统一套用,拖拽一结束(`dim` 变回 `false`)颜色立即恢复。
fn dimmed(color: Color, dim: bool) -> Color {
    if dim {
        Color {
            a: color.a * DRAG_DIM_ALPHA,
            ..color
        }
    } else {
        color
    }
}
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p byteui dimmed -- --nocapture`
Expected: `dimmed_scales_alpha_when_true` 与 `dimmed_passthrough_when_false`
均 PASS。

- [x] **Step 5: 给 `icon_button_entry` 加 `dim` 参数并接入 `dimmed()`**

把签名从:

```rust
pub fn icon_button_entry<'a, M: Clone + 'a>(
    kind: IconKind,
    size: f32,
    active: bool,
    hover_t: f32,
    card: bool,
    button_size: f32,
    interactive: bool,
    on_select: M,
    on_hover: impl Fn(bool) -> M + 'a,
    tooltip: &'a str,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
```

改成(在 `active` 后插入 `dim`):

```rust
pub fn icon_button_entry<'a, M: Clone + 'a>(
    kind: IconKind,
    size: f32,
    active: bool,
    dim: bool,
    hover_t: f32,
    card: bool,
    button_size: f32,
    interactive: bool,
    on_select: M,
    on_hover: impl Fn(bool) -> M + 'a,
    tooltip: &'a str,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
```

函数体内,把:

```rust
    let colors = crate::theme::color::current();
    let color = if active {
        colors.gold
    } else {
        crate::theme::color::mix(colors.dim, colors.gold, hover_t)
    };
    let inner = container(view(kind, size, color))
```

改成:

```rust
    let colors = crate::theme::color::current();
    let color = dimmed(
        if active {
            colors.gold
        } else {
            crate::theme::color::mix(colors.dim, colors.gold, hover_t)
        },
        dim,
    );
    let inner = container(view(kind, size, color))
```

再把 `btn` 的 `style` 闭包内:

```rust
            button::Style {
                background: if card { Some(colors.card.into()) } else { None },
                border: Border {
                    color: if active {
                        colors.gold
                    } else {
                        Color::TRANSPARENT
                    },
                    ..base_border
                },
                ..button::Style::default()
            }
```

改成:

```rust
            button::Style {
                background: if card {
                    Some(dimmed(colors.card, dim).into())
                } else {
                    None
                },
                border: Border {
                    color: dimmed(
                        if active { colors.gold } else { Color::TRANSPARENT },
                        dim,
                    ),
                    ..base_border
                },
                ..button::Style::default()
            }
```

- [x] **Step 6: 编译,确认因签名变化而报错的调用方清单**

Run: `cargo build -p byteui -p dozer-app 2>&1 | grep "error\[" `
Expected: 报一批 `icon_button_entry` 调用处"参数数量不对"的错误——这些
就是下一步要机械修的调用方(不会有别的错误,`dim` 是新增位置参数,不
影响类型系统之外的任何东西)。

- [x] **Step 7: 机械修复全部调用方,在 `active` 参数后插入 `dim: false`**

以下 9 处非图标栏调用一律传 `false`(拖拽换栏是图标栏独有的交互,其余
按钮永远不会处于"被拖拽"态)。用 `grep -n "icon_button_entry(" -A2` 定位
每一处的 `active` 那一行,在其后插入一行 `false,`(2 空格缩进对齐同一
参数列表)。逐处如下:

`crates/dozer-app/src/extensions/database.rs` 里 `back_button`:

```rust
    let back_button = icons::icon_button_entry(
        icons::IconKind::ChevronLeft,
        byteui::theme::icon_size::row(),
        false,
        false,
        schema_back_hover_t,
        false,
        box_len,
        true,
        Message::SchemaBack,
        |hovered| Message::ToolbarHover(DatabaseToolbarTarget::SchemaBack, hovered),
        "返回",
    );
```

`crates/dozer-app/src/extensions/usage.rs` 里 `refresh`:

```rust
    let refresh = icons::icon_button_entry(
        icons::IconKind::RefreshCw,
        byteui::theme::icon_size::row(),
        false,
        false,
        refresh_hover_t,
        false,
        byteui::theme::geometry::rail_button_size(),
        true,
        Message::Refresh,
        Message::Hover,
        "刷新",
    );
```

`crates/dozer-app/src/extensions/ssh.rs` 里 `icon_btn` 闭包:

```rust
    let icon_btn = |kind: icons::IconKind, on_select: Message, tooltip: &'a str, idx: u8| {
        byteui::interaction::icons::icon_button_entry(
            kind,
            byteui::theme::icon_size::row(),
            /* active */ false,
            /* dim */ false,
            /* hover_t */ if is_hover(idx) { 1.0 } else { 0.0 },
            /* card */ true,
            byteui::theme::icon_size::row() + 10.0,
            /* interactive */ true,
            on_select,
            /* on_hover */
            move |hovered| {
                Message::HoverAction(if hovered {
                    Some((host.id.clone(), idx))
                } else {
                    None
                })
            },
            tooltip,
        )
    };
```

`crates/dozer-app/src/extensions/browser.rs` 里 `nav_button`(先用
`grep -n "fn nav_button" crates/dozer-app/src/extensions/browser.rs`
重新核对当前行号,函数体本身不受并发改动影响):

```rust
    icons::icon_button_entry(
        kind,
        byteui::theme::icon_size::row(),
        false,
        false,
        state.nav_hover(action),
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::Nav(action),
        move |hovered| Message::Hover(key, false, hovered),
        tooltip,
    )
```

同文件 `star_button`:

```rust
    icons::icon_button_entry(
        icons::IconKind::Star,
        byteui::theme::icon_size::row(),
        starred,
        false,
        state.star_hover(),
        false,
        byteui::theme::geometry::tab_button_size(),
        url.is_some(),
        Message::StarClick,
        |hovered| Message::Hover(STAR_HOVER_KEY, false, hovered),
        "收藏",
    )
```

同文件 `bookmarks_toggle_button`:

```rust
    icons::icon_button_entry(
        icons::IconKind::Bookmark,
        byteui::theme::icon_size::row(),
        false,
        false,
        state.bookmark_hover(),
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::BookmarksToggle,
        |hovered| Message::Hover(STAR_HOVER_KEY, true, hovered),
        "收藏夹",
    )
```

`crates/dozer-app/src/extensions/files.rs` 里 `search_button`:

```rust
    let search_button = icons::icon_button_entry(
        icons::IconKind::FolderSearch,
        byteui::theme::icon_size::row(),
        false,
        false,
        search_hover_t,
        true,
        box_len,
        true,
        Message::SearchSubmit,
        |hovered| Message::ToolbarHover(FilesToolbarTarget::SearchSubmit, hovered),
        "搜索",
    );
```

同文件 `dotfiles_button`:

```rust
    let dotfiles_button = icons::icon_button_entry(
        if dotfiles_shown {
            icons::IconKind::Eye
        } else {
            icons::IconKind::EyeOff
        },
        byteui::theme::icon_size::row(),
        false,
        false,
        dotfiles_hover_t,
        true,
        box_len,
        true,
        Message::ToggleDotfiles,
        |hovered| Message::ToolbarHover(FilesToolbarTarget::Dotfiles, hovered),
        "切换点文件",
    );
```

同文件 `switch`(分支切换 chevron):

```rust
        let switch = icons::icon_button_entry(
            if ws_state.branch_picker_open {
                icons::IconKind::ChevronUp
            } else {
                icons::IconKind::ChevronDown
            },
            byteui::theme::icon_size::row(),
            false,
            false,
            branch_hover_t,
            false,
            box_len,
            true,
            Message::BranchPickerOpen,
            |hovered| Message::ToolbarHover(FilesToolbarTarget::BranchSwitch, hovered),
            "切换分支",
        );
```

`crates/dozer-app/src/app.rs` 里 `icon_rail` 循环内的调用(这次先传
`false` 占位,Task 3 会把这个 `false` 换成真正的表达式):

```rust
        let base: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            icons::icon_button_entry(
                icon,
                byteui::theme::icon_size::rail(),
                kind == active_kind && open,
                false,
                app.hover_progress(HoverId::Rail(RailButton::Panel(kind))),
                true,
                button_size,
                false,
                Message::PanelSelect(kind),
                move |hovered| Message::Hover(HoverId::Rail(RailButton::Panel(kind)), hovered),
                tooltip,
            );
```

- [x] **Step 8: 编译 + 全量测试确认绿**

Run: `cargo build -p byteui -p dozer-app && cargo test -p byteui -p dozer-app`
Expected: 编译通过,全部既有测试(含 Step 1-4 新增的两个)PASS,视觉上
无任何变化(所有调用方 `dim` 均为 `false`)。

- [x] **Step 9: fmt + clippy**

Run: `cargo fmt --package byteui --package dozer-app && cargo clippy -p byteui -p dozer-app --all-targets`
Expected: fmt 无改动残留(若有改动,`git add` 一并提交);clippy 无新增
警告(既有的几条历史警告不算,见 Task 6 的基线记录)。

- [x] **Step 10: Commit**

```bash
git branch --show-current
git add crates/byteui/src/interaction/icons.rs \
  crates/dozer-app/src/app.rs \
  crates/dozer-app/src/extensions/database.rs \
  crates/dozer-app/src/extensions/usage.rs \
  crates/dozer-app/src/extensions/ssh.rs \
  crates/dozer-app/src/extensions/browser.rs \
  crates/dozer-app/src/extensions/files.rs
git commit -m "feat(byteui): icon_button_entry 新增 dim 参数(拖拽变淡预留)"
```

---

## Task 2: `dragged_panel_kind` 纯函数 + `App` 包装方法

**Files:**
- Modify: `crates/dozer-app/src/app.rs`
- Test: `crates/dozer-app/src/app.rs`(`mod rail_drag_tests` 内)

**Interfaces:**
- Consumes: `RailLayout::side(&self, side: Side) -> &Vec<PanelKind>`
  (已存在);`RailDrag { source_side: Side, source_index: usize, .. }`
  (已存在,`Copy`)。
- Produces: 私有自由函数 `fn dragged_panel_kind(rail: &RailLayout, drag:
  Option<RailDrag>) -> Option<PanelKind>`——Task 3/4 都靠它拿"当前正被拖
  的是哪个面板"。
- Produces: `impl App { pub fn dragged_panel_kind(&self) -> Option<PanelKind>
  }`——薄包装,内部调用上面的自由函数(同 `App::rail_drag_move` 包装
  `rail_drag_move_into` 的既有手法),Task 3/4 的 `app.rs` 视图代码直接
  调这个方法。

- [x] **Step 1: 写失败测试**

在 `crates/dozer-app/src/app.rs` 的 `mod rail_drag_tests` 内(紧跟在已有
的 `same_side_reorder_moves_kind` 等测试之后)追加:

```rust
        #[test]
        fn dragged_panel_kind_none_when_not_dragging() {
            let rail = RailLayout::default();
            assert_eq!(dragged_panel_kind(&rail, None), None);
        }

        #[test]
        fn dragged_panel_kind_reads_source_slot() {
            let rail = RailLayout::default();
            let kind = rail.left[0];
            let drag = RailDrag {
                source_side: Side::Left,
                source_index: 0,
                origin_index: 0,
                pending_cross_side: None,
            };
            assert_eq!(dragged_panel_kind(&rail, Some(drag)), Some(kind));
        }

        #[test]
        fn dragged_panel_kind_none_when_index_out_of_bounds() {
            let rail = RailLayout::default();
            let drag = RailDrag {
                source_side: Side::Left,
                source_index: 999,
                origin_index: 999,
                pending_cross_side: None,
            };
            assert_eq!(dragged_panel_kind(&rail, Some(drag)), None);
        }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app rail_drag_tests::dragged_panel_kind -- --nocapture`
Expected: 编译失败(`dragged_panel_kind` 未定义)。

- [x] **Step 3: 实现自由函数**

在 `rail_drag_move_into`/`rail_cross_apply` 两个自由函数附近(紧跟在
`rail_cross_apply` 定义之后)插入:

```rust
/// 当前正被图标栏拖拽的面板种类(`None` = 未在拖拽)。纯查询,不修改
/// `rail`/`drag`——拖拽中同栏重排会实时更新 `drag.source_index`(见
/// `rail_drag_move_into`),所以这里查到的永远是"此刻鼠标下真正拖着的
/// 那个图标",不是拖拽开始时的原始位置。下标越界(理论不会发生,防御性)
/// 时返回 `None`,不 panic。
fn dragged_panel_kind(rail: &RailLayout, drag: Option<RailDrag>) -> Option<PanelKind> {
    let drag = drag?;
    rail.side(drag.source_side).get(drag.source_index).copied()
}
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app rail_drag_tests::dragged_panel_kind -- --nocapture`
Expected: 3 个新测试均 PASS。

- [x] **Step 5: 加 `App` 包装方法**

紧跟在 `pub fn dragging_rail(&self) -> bool { self.rail_drag.is_some() }`
(约 `app.rs:3685`)之后插入:

```rust
    /// 当前正被拖拽的面板种类(`None` = 未在拖拽)——视图层(`icon_rail`
    /// 源图标变淡 / `rail_drag_ghost` 幽灵图标取图标)据此判断"这是不是
    /// 我"。薄包装 `dragged_panel_kind` 自由函数(同 `rail_drag_move`
    /// 包装 `rail_drag_move_into` 的既有手法),不重复实现逻辑。
    pub fn dragged_panel_kind(&self) -> Option<PanelKind> {
        dragged_panel_kind(&self.shell_layout.rail_layout, self.rail_drag)
    }
```

- [x] **Step 6: 编译 + 测试**

Run: `cargo build -p dozer-app && cargo test -p dozer-app rail_drag_tests`
Expected: 编译通过,`rail_drag_tests` 模块全部 PASS。

- [x] **Step 7: fmt + clippy**

Run: `cargo fmt --package dozer-app && cargo clippy -p dozer-app --all-targets`
Expected: 无新增警告。

- [x] **Step 8: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): 新增 dragged_panel_kind 查询当前被拖拽的面板"
```

---

## Task 3: 源图标拖拽时变淡

**Files:**
- Modify: `crates/dozer-app/src/app.rs`(`icon_rail` 函数)

**Interfaces:**
- Consumes: Task 1 的 `icon_button_entry(.., dim: bool, ..)`;Task 2 的
  `App::dragged_panel_kind() -> Option<PanelKind>`。
- Produces: 无新接口,纯行为变化。

- [x] **Step 1: 把 Task 1 Step 7 里占位的 `false` 换成真表达式**

`icon_rail` 函数体内(约 `app.rs:7581`),把:

```rust
        let base: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            icons::icon_button_entry(
                icon,
                byteui::theme::icon_size::rail(),
                kind == active_kind && open,
                false,
                app.hover_progress(HoverId::Rail(RailButton::Panel(kind))),
                true,
                button_size,
                false,
                Message::PanelSelect(kind),
                move |hovered| Message::Hover(HoverId::Rail(RailButton::Panel(kind)), hovered),
                tooltip,
            );
```

改成:

```rust
        let base: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            icons::icon_button_entry(
                icon,
                byteui::theme::icon_size::rail(),
                kind == active_kind && open,
                Some(kind) == app.dragged_panel_kind(),
                app.hover_progress(HoverId::Rail(RailButton::Panel(kind))),
                true,
                button_size,
                false,
                Message::PanelSelect(kind),
                move |hovered| Message::Hover(HoverId::Rail(RailButton::Panel(kind)), hovered),
                tooltip,
            );
```

- [x] **Step 2: 编译**

Run: `cargo build -p dozer-app`
Expected: 通过,无警告。这一步没有可自动断言的纯逻辑增量(`icon_rail`
是 `Element` 视图函数,不接受单元测试——同文件里其余视图函数一贯的
测试策略,见 `rail_drag_tests` 模块注释"`App` 没有 `Default` 实现、也无
可复用的测试构造 helper"),正确性由 Task 6 的手工验证兜底。

- [x] **Step 3: fmt**

Run: `cargo fmt --package dozer-app`

- [x] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): 图标栏拖拽源图标变淡"
```

---

## Task 4: 幽灵图标跟随光标

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: `App::dragged_panel_kind()`(Task 2)、`App::dragging_rail()`
  (已存在)、`App::last_cursor: (f32, f32)`(已存在,`pub(crate)`)、
  `App::window_size: (f32, f32)`(已存在)、`panel_meta(kind: PanelKind)
  -> (icons::IconKind, &'static str)`(已存在)、
  `byteui::theme::geometry::rail_button_size() -> f32`(已存在)。
- Produces: 私有视图函数 `fn rail_drag_ghost(app: &App) -> Element<'_,
  Message, iced_widget::Theme, iced_renderer::Renderer>`,接入
  `App::view` 最外层组合。

这里不用 `icons::icon_button_entry`:幽灵图标没有 hover/点击,不需要
`MouseArea`/`button::on_press` 这套交互接线,`icon_button_entry` 的参数
(`active`/`hover_t`/`interactive`/`on_select`/`on_hover`)对它统统无意义
——套用只会传一堆假参数掩盖"这其实是个纯展示元素"的事实,所以直接用
更底层的 `icons::view` + 手写 `container` 样式(CLAUDE.md 允许的"形状/
交互模式明显不同"例外)。

- [x] **Step 1: 实现 `rail_drag_ghost`**

在 `icon_rail` 函数定义之后(紧跟其闭合的 `}`)插入:

```rust
/// 图标栏拖拽期间跟随光标的幽灵图标——视觉语言对齐 OS 拖文件夹:一个
/// 圆角方卡(同 `icon_button_entry` 选中态的 CARD 底 + 金框),里面是被
/// 拖面板的图标,整体以光标为中心悬浮。没有任何交互(不接 `MouseArea`/
/// `on_press`),纯展示——不会挡住底下 `rail_drag_surface` 的 `on_move`/
/// `on_press`(iced 里非交互 widget 天然不参与命中测试,同本文件
/// `stack![base, badge]` 徽标叠在按钮上不挡点击的既有先例)。
///
/// 定位手法同 `project_add_menu_popup`:整窗 `Length::Fill` 容器 + 用
/// `padding` 把内容推到目标坐标,这次坐标是每帧都在变的 `App::last_cursor`
/// 而不是开菜单那一刻的定格快照,所以幽灵图标才会真的"跟手"——
/// `last_cursor` 本来就在每次 `CursorMoved` 里更新,main.rs 也已经在每次
/// `CursorMoved` 后无条件 `window.request_redraw()`(见其注释"悬停也要
/// 请求重绘"),这两点凑在一起,`rail_drag_ghost` 不需要任何额外的重绘
/// 触发就能逐帧跟手。
///
/// 拖拽未在进行,或(理论不会发生的防御性分支)拖拽中但下标越界拿不到
/// 面板种类时,返回空占位——不画任何东西。
fn rail_drag_ghost(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !app.dragging_rail() {
        return column![].into();
    }
    let Some(kind) = app.dragged_panel_kind() else {
        return column![].into();
    };
    let (icon, _tooltip) = panel_meta(kind);
    let size = byteui::theme::geometry::rail_button_size();
    let colors = byteui::theme::color::current();

    let ghost = container(icons::view(icon, byteui::theme::icon_size::rail(), colors.gold))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(colors.card.into()),
            border: Border {
                color: colors.gold,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..container::Style::default()
        });

    // 幽灵图标以光标为中心(减半个按钮边长做偏移),并钳制在窗口范围内
    // ——防止贴着窗口边缘拖拽时图标一半画到窗口外。
    let (cx, cy) = app.last_cursor;
    let (window_w, window_h) = app.window_size;
    let x = (cx - size / 2.0).clamp(0.0, (window_w - size).max(0.0));
    let y = (cy - size / 2.0).clamp(0.0, (window_h - size).max(0.0));

    container(ghost)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: y,
            left: x,
            right: 0.0,
            bottom: 0.0,
        })
        .into()
}
```

- [x] **Step 2: 编译(此时 `rail_drag_ghost` 尚未被调用,预期一条
  `unused function` warning)**

Run: `cargo build -p dozer-app 2>&1 | grep -A2 "never used"`
Expected: 报 `rail_drag_ghost` 未使用——预期中的中间状态,下一步接入
调用点后消失。

- [x] **Step 3: 接入 `App::view` 最外层组合**

`App::view` 函数末尾(约 `app.rs:6658-6665`)现状:

```rust
        if let Some(which) = self.maximized {
            stack![popped, maximize_overlay(self, ws, which)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            popped
        }
    }
}
```

改成:

```rust
        let with_maximize: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            if let Some(which) = self.maximized {
                stack![popped, maximize_overlay(self, ws, which)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            } else {
                popped
            };

        if self.dragging_rail() {
            stack![with_maximize, rail_drag_ghost(self)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            with_maximize
        }
    }
}
```

- [x] **Step 4: 编译**

Run: `cargo build -p dozer-app`
Expected: 通过,无警告(`rail_drag_ghost` 现在被调用了)。

- [x] **Step 5: fmt + clippy**

Run: `cargo fmt --package dozer-app && cargo clippy -p dozer-app --all-targets`
Expected: 无新增警告。

- [x] **Step 6: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): 图标栏拖拽新增跟随光标的幽灵图标"
```

---

## Task 5: 跨栏悬停目标位插入线高亮

**Files:**
- Modify: `crates/dozer-app/src/app.rs`(`icon_rail` 函数)

**Interfaces:**
- Consumes: `RailDrag.pending_cross_side: Option<(Side, usize)>`(已
  存在,`pub` 字段);`icon_rail` 函数体内已有的局部变量 `region`/
  `step`/`button_size`/`layers`(不新增参数,直接在函数体内用)。
- Produces: 无新接口,`icon_rail` 内部新增一段渲染逻辑。

跨栏移动是"插入"(`rail_cross_apply` 把面板 `insert` 到目标下标,不是
和目标位那个按钮互换),所以高亮画成一条细金线卡在两个按钮之间的插入
点,不是把某个已存在的按钮整个描边——避免看起来像"要跟这个按钮互换
位置"。

- [x] **Step 1: 在 `icon_rail` 的按钮渲染循环之后、`Stack::with_children`
  之前插入高亮层**

`icon_rail` 函数体内,找到:

```rust
    let content = iced_widget::Stack::with_children(layers)
        .width(Length::Fill)
        .height(Length::Fill);
```

在它**之前**插入:

```rust
    // 跨栏拖拽悬停到本栏时,在悬停下标处画一条金色插入线——`rail_cross_
    // apply` 落地时是 `insert`(把已有项推后一位),不是跟目标位的按钮
    // 互换,所以高亮画成"卡在两个按钮之间的线",不描边某个已存在按钮
    // (那样会误导成"要跟它换位")。`pending_cross_side.1` 恒是目标栏
    // 某个已有按钮自己上报的下标(见 `rail_drag_surface` 的 `on_move`
    // 只挂在真实按钮上),不会是 `len()`(悬停不到"最后一个之后"这个
    // 位置——现有交互面就是如此,不是这次新引入的限制)。
    if let Some((_, idx)) = app
        .rail_drag
        .and_then(|d| d.pending_cross_side)
        .filter(|(s, _)| *s == side)
    {
        let bar_h = 3.0;
        let y = (region.padding.top + idx as f32 * step - region.gap / 2.0 - bar_h / 2.0).max(0.0);
        let marker = container(iced_widget::Space::new())
            .width(Length::Fixed(button_size))
            .height(Length::Fixed(bar_h))
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(byteui::theme::color::current().gold.into()),
                ..container::Style::default()
            });
        let positioned = container(marker)
            .padding(Padding {
                top: y,
                left: region.padding.left,
                right: region.padding.right,
                bottom: 0.0,
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced_widget::core::alignment::Horizontal::Left)
            .align_y(iced_widget::core::alignment::Vertical::Top);
        layers.push(positioned.into());
    }
```

- [x] **Step 2: 编译**

Run: `cargo build -p dozer-app`
Expected: 通过。若报 `region`/`step`/`button_size` 未定义或类型不符,
说明这些局部变量名在当前 `icon_rail` 实现里已经改名——用
`grep -n "let region\|let step\|let button_size" crates/dozer-app/src/app.rs`
核对实际变量名后调整插入的代码块,逻辑不变。

- [x] **Step 3: fmt + clippy**

Run: `cargo fmt --package dozer-app && cargo clippy -p dozer-app --all-targets`
Expected: 无新增警告。

- [x] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): 图标栏跨栏拖拽悬停新增插入线高亮"
```

---

## Task 6: 全量验证 + 手工 GUI 走查

**Files:** 无新增改动(除非验证中发现问题需要回到前面的 Task 修正)。

- [x] **Step 1: 全 workspace 构建 + 测试**

Run: `cargo build && cargo test -p byteui -p dozer-app`
Expected: 全绿。

- [x] **Step 2: clippy + fmt 全 workspace**

Run: `cargo clippy --all-targets && cargo fmt --check`
Expected: 无新增警告(开工前如果已有历史警告基线,如
`extensions/todo.rs` 的 `ws_state` 未使用参数、`files.rs` 的
`type_complexity`,这些不算——只要确认本计划改动的文件里没有新增)。

- [ ] **Step 3: 手工 GUI 走查**

Run: `cargo run -p dozer-app`

按以下清单逐条验证(每条都是本计划新增的行为,不是既有回归测试能覆盖
的):

1. 打开一个项目,长按左栏任意图标栏按钮开始拖拽:
   - 源位置图标应立即变淡。
   - 一个圆角方卡幽灵图标应出现在光标位置,内容是被拖面板的图标,随
     鼠标移动逐帧跟手(不是跳着走、也不是黏在原地不动)。
2. 在左栏内拖动到另一个位置再松手:
   - 其余按钮应实时让位(既有行为,不应被这次改动破坏)。
   - 松手后幽灵图标消失,源图标恢复正常不透明度,新顺序落定。
3. 把左栏某个按钮拖到右栏上空:
   - 幽灵图标继续跟手。
   - 悬停到右栏某个按钮附近时,该处应出现一条金色插入线。
   - 挪回左栏(源栏)时插入线应消失。
   - 在右栏松手:面板应真的搬到右栏、成为右栏新 active、幽灵图标和
     插入线都消失、源图标(现在已经不在源栏了)不再有淡出的残留。
4. 按住拖拽后原地松手(不移动):幽灵图标不应残留一帧不消失,源图标
   应立即恢复正常不透明度。
5. 某一栏只剩 1 个面板时,尝试把它拖到另一栏再松手:应被拒绝(既有
   "不支持栏清空"行为),幽灵图标/插入线/变淡应正常清空,不留任何
   视觉残留,面板应留在原栏。
6. 窗口贴边测试:把某个按钮拖到贴近窗口最左/最右/最上/最下边缘,幽灵
   图标不应画出窗口外(应被钳制在窗口内)。

若发现任何一条不符,回到对应 Task 定位问题(视觉问题通常在 Task 3/4/5
的具体某一步),修正后重新走这份清单,不要跳过已经手工验证过的条目
之外的新改动。

- [x] **Step 4: 确认全部 commit 都在特性分支上**

Run: `git log --oneline main..HEAD` (在 `feature/rail-drag-cursor-ghost`
分支上执行)
Expected: 列出 Task 1-5 的 5 个 commit,且 `git branch --show-current`
不是 `main`。

- [ ] **Step 5: 提请审阅**

不在这一步直接合并——按仓库既有约定提交 PR / 请求代码审阅,通过后再
合并回 `main`(参考 `superpowers:finishing-a-development-branch`)。
