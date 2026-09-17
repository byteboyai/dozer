# search_modal 迁独立原生窗口 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `search_modal`(文件树右键"搜索"弹窗)从"画在主窗口 iced `stack!` 里 + `set_visible(false)` 强制隐藏 preview webview"改成"独立原生 `winit::Window`,天然盖过 wry webview,不再需要隐藏 webview"。

**Architecture:** 新增 `platform/search_overlay.rs` 承载 `SearchOverlay`(独立窗口 + 共享主窗口 `Device`/`Queue` 的自建 `Engine`/`Renderer`)与两个纯函数(居中定位、开关判定);`platform/window_events.rs` 的 `Runner::Ready` 新增 `search_overlay` 字段 + `sync_search_overlay()`,`window_event` 顶部新增一个按 `WindowId` 分流的分支,不改动现有主窗口那套 ~1000 行的事件处理。`extensions/search.rs` 拆出无 scrim 的 `search_card()`。完成后删除旧的 `set_visible(false)` 触发点与配套的一次性聚焦/焦点回读 plumbing。

**Tech Stack:** Rust workspace;`iced_wgpu`/`iced_winit`/`winit`/`wgpu` 均复用 `crates/dozer-app` 已有依赖,不新增/升级任何依赖。

**Spec:** `docs/superpowers/specs/2026-09-17-search-modal-overlay-window-design.md`

## Global Constraints

- **在独立分支上开发**:建分支 `feature/search-modal-overlay-window`(或对应 worktree),完成后提请审阅,通过再合并回 `main`。
- **这个计划基于当前 `main`(commit `4923b81`)分析**。工作目录被多个并行会话共享,开工前用本文档的 `grep -n` 模式核对实际行号。
- **不引入新依赖**,`Cargo.toml` 不需要改动。
- **不改变除以下三点外的任何可观察行为**:①`search_modal` 从"全窗遮罩居中卡片"变成"独立卡片窗口";②"点外部关闭"从"点遮罩"变成"窗口失焦";③打开 search 时预览/浏览器 webview 不再被强制隐藏(这正是本计划的目的)。查询/结果/Esc/×按钮/自动聚焦等交互效果与现状等价。
- 每个任务结束都要 `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer && cargo fmt -- --check` 干净通过,无新增 warning;`cargo test` 数字与开工前一致(开工前先跑一次记下基线,新增测试只加不减)。
- Task 1 是地基假设验证,**不在本计划的分支/worktree 里做**——直接在已存在的 `spike/multi-window-overlay-wry` 分支的 worktree(`dozer-spike-multi-window`,若已被清理则重新 `git worktree add` 出来)里做,验证通过再回到本计划的分支开 Task 2。若验证不通过,停下不要继续,回去找用户重新讨论设计(见 spec"排期备注")。

---

### Task 1: 验证地基假设——`with_parent_window` 子窗口能否独立收到 `Focused(false)`

**Files:**
- Modify: `spike/multi-window-overlay/src/main.rs`(在 `spike/multi-window-overlay-wry` 分支的 worktree 里,不是本计划的分支)

**Interfaces:**
- Consumes: 该 spike 已有的 `Overlay`/`open_overlay`/`window_event` 分支骨架(`spike/multi-window-overlay-wry` 分支 commit `5e522a0`)。
- Produces: 一个终端可见的验证结果(控制台打印),供本任务的验收判断用,不产出任何生产代码改动。

- [ ] **Step 1: 定位 overlay 的 `window_event` 分支**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer-spike-multi-window
command grep -n "id == overlay.window.id()" spike/multi-window-overlay/src/main.rs
```

预期能看到类似:

```rust
if let Some(overlay) = &self.overlay {
    if id == overlay.window.id() {
        match event {
            WindowEvent::CloseRequested => {
                self.overlay = None;
            }
            WindowEvent::RedrawRequested => {
                draw_overlay_iced(&mut self.overlay.as_mut().unwrap());
            }
            _ => {}
        }
        return;
    }
}
```

- [ ] **Step 2: 加一行 `Focused` 打印**

把 `_ => {}` 之前加一条新分支(顺序不影响其它分支匹配):

```rust
                WindowEvent::Focused(focused) => {
                    println!("[focus-probe] overlay window Focused({focused})");
                }
```

- [ ] **Step 3: 跑起来,人工触发并观察**

```bash
cargo run -p spike-multi-window-overlay
```

操作顺序:按 `o` 打开 overlay → 点击主窗口(webview 那半区或左半区都行,只要落在主窗口内)→ 看终端输出。

**验收判断:**
- 若终端打印出 `[focus-probe] overlay window Focused(false)`——假设成立,继续 Task 2。
- 若点击主窗口后终端**没有**打印这行(overlay 窗口没收到独立的失焦事件)——假设不成立,**停止执行本计划**,把这个结果报告给用户,不要继续 Task 2 及之后的任务(spec"排期备注"已经预告这个分支需要回去重新讨论"关闭"触发方式的设计)。

- [ ] **Step 4: 验证完清理**

这行 `println!` 是临时探针,不需要保留(spike 本身随时可删,不需要维持长期状态)。可以直接 `git checkout -- spike/multi-window-overlay/src/main.rs` 丢弃这一行,或者留着不 commit 都行——这个 worktree 不是本计划要合并的分支,不影响本计划后续任务。

---

### Task 2: 从 `search_modal()` 拆出无 scrim 的 `search_card()`

**Files:**
- Modify: `crates/dozer-app/src/extensions/search.rs`

**Interfaces:**
- Consumes: 无新依赖,纯内部重构。
- Produces: `pub(crate) fn search_card<'a>(ws: &'a WorkspaceState, project_root: Option<&'a Path>) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>`——对话框卡片本体(无 scrim、`height(Length::Fill)` 填满调用方给的画布,不再是 `Length::Shrink`+`max_height`,因为调用方以后是一扇裁好尺寸的独立窗口,不是"嵌在更大画布里要收缩适配")。旧 `search_modal()` 暂时保留,内部改调 `search_card()` + 包一层 scrim/stack,签名和行为不变,`app/view.rs:131-143` 这次不用动。

- [ ] **Step 1: 定位现有 `search_modal`**

```bash
command grep -n "pub fn search_modal" crates/dozer-app/src/extensions/search.rs
```

预期在 `search.rs:340`。

- [ ] **Step 2: 把函数体拆成两半**

把 `search_modal`(`search.rs:340-444`)改成:

```rust
/// 搜索弹窗的卡片本体(标题行 + 查询行 + 结果列表),无遮罩、无外层定位——
/// 这次拆分是为了让独立 overlay 窗口(`platform/search_overlay.rs`)能直接
/// 复用同一份视图逻辑,只是换一个宿主(独立窗口取代 `App::view()` 的
/// `stack!` 层)。`height(Length::Fill)`:调用方现在总是给一块已经量好的
/// 画布(要么是旧路径里 `container(dialog)` 分配的区域,要么是新路径里
/// 整扇 overlay 窗口的画布),不需要 `Length::Shrink` 那种"在更大画布里
/// 收缩适配内容"的语义。
pub(crate) fn search_card<'a>(
    ws: &'a WorkspaceState,
    project_root: Option<&'a Path>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !ws.open {
        return column![].into();
    }

    let scope_label = match &ws.scope {
        Some(Scope::Dir(p)) => {
            let rel = rel_to_root(p, project_root);
            format!("目录: {}", rel.display())
        }
        Some(Scope::File(p)) => {
            let rel = rel_to_root(p, project_root);
            format!("文件: {}", rel.display())
        }
        None => String::new(),
    };

    let title_row = row![
        text(scope_label)
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().cream),
        iced_widget::space::horizontal(),
        button(
            text("×")
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().dim)
        )
        .on_press(Message::SearchClose)
        .padding(0)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: byteui::theme::color::current().dim,
            ..button::Style::default()
        }),
    ]
    .align_y(iced_widget::core::Alignment::Center);

    let submit_btn = button(
        text(if ws.running { "搜索中…" } else { "搜索" })
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().gold),
    )
    .on_press(Message::QuerySubmit)
    .padding([6, 12])
    .style(crate::dialog::action_button_style(
        byteui::theme::color::current().gold,
    ));

    let query_row = row![query_box(ws), submit_btn]
        .spacing(8)
        .align_y(iced_widget::core::Alignment::Center);

    let mut body = column![title_row, query_row]
        .width(Length::Fill)
        .spacing(8)
        .height(Length::Shrink);

    if ws.has_searched && ws.results.is_empty() && ws.error.is_none() {
        body = body.push(
            text("无匹配")
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        );
    } else if !ws.results.is_empty() {
        body = body.push(results_list(ws, project_root));
    }
    if let Some(err) = &ws.error {
        body = body.push(
            text(format!("⚠ {err}"))
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().red),
        );
    }

    container(body.padding(16))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(crate::dialog::card_style)
        .into()
}

/// 搜索弹窗本体:套用 `dialog` 模块统一的弹窗原语(磨砂遮罩 + 金色描边
/// 卡片)。由 `App::view()` 顶层浮层链的 `stack!` 里调用;未打开时返回空元素。
///
/// 2026-09-17:迁独立原生窗口过渡期的旧路径,`search_card()` 是新路径
/// (`platform/search_overlay.rs`)复用的部分——本函数连同调用它的
/// `app/view.rs:131-143` 那段会在本计划 Task 5 一并删除,过渡期内暂时保留
/// 让现状行为不受影响。
pub fn search_modal<'a>(
    ws: &'a WorkspaceState,
    project_root: Option<&'a Path>,
    window_width: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !ws.open {
        return column![].into();
    }

    let dialog = container(search_card(ws, project_root))
        .width(crate::dialog::width(window_width))
        .height(Length::Shrink)
        .max_height(640.0);

    let scrim = crate::dialog::scrim(Message::SearchClose);

    stack![
        scrim,
        container(dialog)
            .padding(40.0)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced_widget::core::alignment::Horizontal::Center)
            .align_y(iced_widget::core::alignment::Vertical::Center)
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}
```

注意:旧版 `dialog` 那层原本直接套 `card_style`,现在 `card_style` 挪进了
`search_card()` 内部的 `container(body.padding(16))`,所以上面新的
`search_modal()` 里 `dialog` 不再重复套 `card_style`(避免套两层描边/背景)。
`search_card()` 返回的 `Element` 本身自带卡片外观,`search_modal()` 只需要
再套一层"限定宽度、`Shrink` 高度"的容器做旧路径的尺寸约束。

- [ ] **Step 3: 编译检查**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功,无新增 warning(`search_card` 暂时只被 `search_modal` 一处调用,`pub(crate)` 可见性不会触发 dead_code)。

- [ ] **Step 4: 跑测试**

Run: `cargo test -p dozer-app --bin dozer -- extensions::search::tests`
Expected: 6 个既有测试全绿(纯重构,断言的都是 `search_scope`/`open`/`update` 的行为,不碰这次改的 view 函数)。

- [ ] **Step 5: 人工视觉核对(可选但建议)**

`cargo run -p dozer-app --bin dozer`,文件树右键"搜索"打开弹窗,视觉/交互与改动前一致(标题行、查询框、结果列表、× 按钮、遮罩点击关闭)。

- [ ] **Step 6: 提交**

```bash
git add crates/dozer-app/src/extensions/search.rs
git commit -m "$(cat <<'EOF'
refactor(app): 从 search_modal 拆出无 scrim 的 search_card

为独立原生窗口迁移(设计见 docs/superpowers/specs/2026-09-17-
search-modal-overlay-window-design.md)做准备:search_card 是卡片本体
(标题行+查询行+结果列表),search_modal 变成"套一层旧路径尺寸约束"的
过渡期外壳,过渡期结束后连同 app/view.rs 里的 stack 组合一起删除。纯
重构,行为不变。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: `SearchOverlay` ——独立窗口本体、居中定位、开关判定、事件路由

**Files:**
- Create: `crates/dozer-app/src/platform/search_overlay.rs`
- Modify: `crates/dozer-app/src/platform/mod.rs`
- Modify: `crates/dozer-app/src/platform/window_events.rs`

**Interfaces:**
- Consumes:Task 2 的 `search::search_card`/`search::query_field_id`/`search::CaptureQueryFocus`/`search::take_query_focused`;`Ready` 已有字段 `window`/`device`/`queue`/`format`(新增 `instance`/`adapter`);`crate::runtime::run_operate`(已有,`window_events.rs:2295` 同款用法);`crate::app::{App, Message}`;`crate::dialog::width`。
- Produces:
  - `pub(crate) fn centered_overlay_bounds(main_outer_pos: PhysicalPosition<i32>, main_inner_size: PhysicalSize<u32>, scale: f64, card_logical_size: LogicalSize<f32>) -> (PhysicalPosition<i32>, PhysicalSize<u32>)`——纯函数。
  - `pub(crate) enum SyncAction { Open, Close, Noop }` + `pub(crate) fn sync_action(search_open: bool, overlay_present: bool) -> SyncAction`——纯函数。
  - `pub(crate) struct SearchOverlay`,方法:`open(main_window: &Window, adapter: &wgpu::Adapter, device: &wgpu::Device, queue: &wgpu::Queue, instance: &wgpu::Instance, window_width: f32, el: &ActiveEventLoop) -> SearchOverlay`、`window_id(&self) -> WindowId`、`redraw(&mut self, app: &mut App)`、`handle_input(&mut self, app: &mut App, event: &WindowEvent) -> Vec<Message>`、`reposition(&mut self, main_outer_pos, main_inner_size, scale, window_width: f32)`。
  - `Ready` 新增字段:`instance: wgpu::Instance`、`adapter: wgpu::Adapter`、`search_overlay: Option<search_overlay::SearchOverlay>`。
  - `Runner::sync_search_overlay(&mut self, el: &ActiveEventLoop)`,`window_event` 顶部新增按 `WindowId` 分流到 `SearchOverlay` 的分支。

- [ ] **Step 1: `Ready` 补 `instance`/`adapter` 字段(现在只是被丢弃的局部变量)**

定位:

```bash
command grep -n "let (format, adapter, device, queue) = futures::futures::executor::block_on" crates/dozer-app/src/platform/window_events.rs
```

预期在 `window_events.rs:1500`。这一行已经把 `adapter` 解构出来了,只是后面没存进 `Ready`;`instance`(`:1492` 的 `let instance = wgpu::Instance::new(...)`)同样只是局部变量。

在 `Ready` 枚举变体(`window_events.rs:49-124`)的 `queue`/`device` 字段之间加两个新字段:

```rust
    Ready {
        window: Arc<winit::window::Window>,
        queue: wgpu::Queue,
        device: wgpu::Device,
        /// 建主窗口 wgpu 资源时用的 `Instance`/`Adapter`,原本只是
        /// `resumed()` 里的局部变量、用完就扔——search overlay 窗口
        /// (`platform/search_overlay.rs`)要另开一个 `Surface`+`Engine`,
        /// 必须用同一个 `Instance` 建 surface、同一个 `Adapter` 建
        /// `Engine`(不能用一个新建的、跟当前 `device`/`queue` 没有血缘
        /// 关系的 `Instance`/`Adapter`),所以补存下来。
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        surface: wgpu::Surface<'static>,
        format: wgpu::TextureFormat,
        renderer: iced_renderer::Renderer,
        app: App,
```

在 `resumed()` 的 `*self = Self::Ready { ... }` 字面量(`window_events.rs:1579-1607`)里,`device,` 那行后面加:

```rust
                instance,
                adapter,
```

- [ ] **Step 2: `platform/mod.rs` 登记新模块**

```bash
command grep -n "pub mod window_events;" crates/dozer-app/src/platform/mod.rs
```

在这行后面加:

```rust
pub mod search_overlay;
```

- [ ] **Step 3: 写 `search_overlay.rs` 的纯函数部分 + 单测**

```rust
//! search_modal 的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-17-search-modal-overlay-window-design.md`。
//! `SearchOverlay` 挂在主窗口 `Runner::Ready` 上,不是独立事件循环——
//! winit 原生按 `WindowId` 把多扇窗口的事件分发进同一个
//! `ApplicationHandler`,这扇窗口只是 `window_event` 顶部多出的一个分支。

use std::sync::Arc;

use iced_wgpu::graphics::{Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::Clipboard;
use iced_winit::conversion;
use iced_winit::core::{Event, Font, Pixels, Size, mouse};
use iced_winit::runtime::user_interface::{self, UserInterface};
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId, WindowLevel};

use crate::app::{App, Message};
use crate::extensions::search;

/// overlay 卡片的固定逻辑高度,对应现状 `search_modal` 的
/// `max_height(640.0)`。宽度不固定,随主窗口宽度变化(见 `card_logical_size`
/// 调用点,复用 `crate::dialog::width`)。
const CARD_HEIGHT: f32 = 640.0;

/// 主窗口外框物理位置 + 物理尺寸 + scale + 卡片逻辑尺寸 → overlay 应放的
/// 物理位置与物理尺寸(居中于主窗口)。纯函数,不碰真实 `Window`,方便测试。
pub(crate) fn centered_overlay_bounds(
    main_outer_pos: PhysicalPosition<i32>,
    main_inner_size: PhysicalSize<u32>,
    scale: f64,
    card_logical_size: LogicalSize<f32>,
) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let card_w = card_logical_size.width as f64 * scale;
    let card_h = card_logical_size.height as f64 * scale;
    let x = main_outer_pos.x as f64 + (main_inner_size.width as f64 - card_w) / 2.0;
    let y = main_outer_pos.y as f64 + (main_inner_size.height as f64 - card_h) / 2.0;
    (
        PhysicalPosition::new(x.round() as i32, y.round() as i32),
        PhysicalSize::new(card_w.round() as u32, card_h.round() as u32),
    )
}

/// `sync_search_overlay` 要不要开/关 overlay 的纯判定,跟真正建/毁窗口的
/// 副作用(`SearchOverlay::open`/`Drop`)分开,方便单测穷举四种组合。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncAction {
    Open,
    Close,
    Noop,
}

pub(crate) fn sync_action(search_open: bool, overlay_present: bool) -> SyncAction {
    match (search_open, overlay_present) {
        (true, false) => SyncAction::Open,
        (false, true) => SyncAction::Close,
        (true, true) | (false, false) => SyncAction::Noop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_action_opens_when_search_open_and_no_overlay() {
        assert_eq!(sync_action(true, false), SyncAction::Open);
    }

    #[test]
    fn sync_action_closes_when_search_closed_but_overlay_present() {
        assert_eq!(sync_action(false, true), SyncAction::Close);
    }

    #[test]
    fn sync_action_noop_when_states_already_match() {
        assert_eq!(sync_action(true, true), SyncAction::Noop);
        assert_eq!(sync_action(false, false), SyncAction::Noop);
    }

    #[test]
    fn centered_overlay_bounds_centers_within_main_window() {
        let (pos, size) = centered_overlay_bounds(
            PhysicalPosition::new(100, 50),
            PhysicalSize::new(1200, 800),
            2.0, // Retina 2x
            LogicalSize::new(400.0, 640.0),
        );
        // 卡片物理尺寸 = 逻辑尺寸 * scale。
        assert_eq!(size, PhysicalSize::new(800, 1280));
        // 居中:主窗口物理宽 1200,卡片物理宽 800 → 左右各留 200。
        assert_eq!(pos.x, 100 + 200);
        // 主窗口物理高 800 < 卡片物理高 1280 时,y 会算出负偏移(卡片比
        // 主窗口还高,允许溢出——这不是本函数要处理的极端情形,调用方
        // 传入的卡片尺寸在实际窗口里不会真的比主窗口还大)。
        assert_eq!(pos.y, 50 + (800 - 1280) / 2);
    }

    #[test]
    fn centered_overlay_bounds_at_scale_one() {
        let (pos, size) = centered_overlay_bounds(
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1000, 1000),
            1.0,
            LogicalSize::new(600.0, 640.0),
        );
        assert_eq!(size, PhysicalSize::new(600, 640));
        assert_eq!(pos.x, (1000 - 600) / 2);
        assert_eq!(pos.y, (1000 - 640) / 2);
    }
}
```

Run: `cargo test -p dozer-app --bin dozer -- platform::search_overlay::tests`
Expected: 5 个新测试全绿(此时 `SearchOverlay` 本体还没写,模块能独立编译测试)。

- [ ] **Step 4: 补 `SearchOverlay` 结构体与 `open`**

在 Step 3 写的测试模块之前(文件顶部纯函数之后)加:

```rust
/// 独立原生窗口宿主——`search_card()` 的独立渲染管线。不持有独立的
/// `Device`/`Queue`/`Adapter`/`Instance`:全部从主窗口 `Ready` 借来的
/// 共享句柄(`Device`/`Queue`/`Adapter` 便宜 `Clone`),只有 `Surface`/
/// `Renderer`/`Cache`/`Viewport`/`Clipboard` 是这扇窗口自己的一份(iced_wgpu
/// 的 `Renderer` 内部持有 `Engine`,不能跨窗口共享,`iced_winit` 官方多窗口
/// 场景同样每扇窗口各自一个 `Renderer`)。
pub(crate) struct SearchOverlay {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    format: wgpu::TextureFormat,
    renderer: Renderer,
    cache: user_interface::Cache,
    viewport: Viewport,
    clipboard: Clipboard,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
}

impl SearchOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    /// 开一扇挂成主窗口子窗口的独立窗口,复用主窗口的 `Device`/`Queue`/
    /// `Adapter`/`Instance`,只为这扇窗口单独建 `Surface`/`Engine`/
    /// `Renderer`(spike `spike/multi-window-overlay-wry` 已验证这条路径
    /// 可行:见 `docs/superpowers/specs/2026-09-17-multi-window-overlay-
    /// spike-findings.md`)。
    pub(crate) fn open(
        main_window: &Window,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        window_width: f32,
        el: &ActiveEventLoop,
    ) -> SearchOverlay {
        use winit::raw_window_handle::HasWindowHandle;

        let scale = main_window.scale_factor();
        let card_logical = LogicalSize::new(crate::dialog::width(window_width), CARD_HEIGHT);
        let (pos, size) = centered_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
            scale,
            card_logical,
        );

        let parent_handle = main_window
            .window_handle()
            .expect("main window handle")
            .as_raw();
        let attrs = Window::default_attributes()
            .with_title("search")
            .with_decorations(false)
            .with_transparent(true)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_position(pos)
            .with_inner_size(size);
        // Safety: `parent_handle` 取自仍存活的主窗口(`Ready` 持有的
        // `Arc<Window>`),本函数返回前主窗口不会被 drop。
        let attrs = unsafe { attrs.with_parent_window(Some(parent_handle)) };
        let window = Arc::new(el.create_window(attrs).expect("create search overlay window"));
        window.request_focus();

        let surface = instance
            .create_surface(window.clone())
            .expect("create search overlay surface");
        let capabilities = surface.get_capabilities(adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| capabilities.formats.first().copied())
            .expect("get search overlay surface format");
        surface.configure(
            device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: size.width.max(1),
                height: size.height.max(1),
                present_mode: wgpu::PresentMode::AutoVsync,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            },
        );

        let engine = Engine::new(
            adapter,
            device.clone(),
            queue.clone(),
            format,
            None,
            Shell::headless(),
        );
        let renderer = Renderer::new(engine, Font::default(), Pixels::from(16));
        let viewport =
            Viewport::with_physical_size(Size::new(size.width, size.height), scale as f32);
        let clipboard = Clipboard::connect(window.clone());

        SearchOverlay {
            window,
            surface,
            format,
            renderer,
            cache: user_interface::Cache::new(),
            viewport,
            clipboard,
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
        }
    }

    /// 主窗口 resize 后重新居中 + 重配置 surface(`with_parent_window` 的
    /// `addChildWindow` 语义只让位置跟随移动,不管尺寸/布局联动)。
    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
        window_width: f32,
    ) {
        let card_logical = LogicalSize::new(crate::dialog::width(window_width), CARD_HEIGHT);
        let (pos, size) =
            centered_overlay_bounds(main_outer_pos, main_inner_size, scale, card_logical);
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.viewport = Viewport::with_physical_size(
                Size::new(size.width, size.height),
                scale as f32,
            );
            self.surface.configure(
                device,
                &wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format: self.format,
                    width: size.width.max(1),
                    height: size.height.max(1),
                    present_mode: wgpu::PresentMode::AutoVsync,
                    alpha_mode: wgpu::CompositeAlphaMode::Auto,
                    view_formats: vec![],
                    desired_maximum_frame_latency: 2,
                },
            );
        }
    }
}
```

- [ ] **Step 5: 补渲染 + 输入方法**

接在 Step 4 写的 `reposition` 后面,加两个方法:

```rust
    /// 每帧渲染:清成透明(窗口本身 `with_transparent(true)`,卡片自己的
    /// `card_style` 背景覆盖几乎全部区域,清透明只是消掉窗口边缘的
    /// 未初始化像素),然后走标准 `UserInterface::build → draw → present`。
    /// 打开后第一帧顺带消费"查询框待自动聚焦"一次性位(复用
    /// `extensions::search::open()` 早就在设的那个标记,只是消费方从主
    /// 窗口挪到这里——见本计划 Task 6 对主窗口那份消费逻辑的删除)。
    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(ws) = app.active_workspace_mut() else {
            return;
        };
        let project_root = ws
            .project
            .as_ref()
            .map(|p| std::path::Path::new(&p.path).to_path_buf());

        let mut interface = UserInterface::build(
            search::search_card(&ws.search, project_root.as_deref()),
            self.viewport.logical_size(),
            std::mem::take(&mut self.cache),
            &mut self.renderer,
        );

        if ws.take_query_focus_pending() {
            let mut op = iced_winit::core::widget::operation::focusable::focus::<()>(
                search::query_field_id(),
            );
            crate::runtime::run_operate(&mut interface, &mut self.renderer, &mut op);
        }
        crate::runtime::run_operate(&mut interface, &mut self.renderer, &mut search::CaptureQueryFocus);
        ws.search.set_query_focused(search::take_query_focused());

        let _ = interface.update(
            &[],
            self.cursor,
            &mut self.renderer,
            &mut self.clipboard,
            &mut Vec::new(),
        );
        interface.draw(
            &mut self.renderer,
            &iced_winit::core::Theme::Dark,
            &iced_winit::core::renderer::Style::default(),
            self.cursor,
        );
        self.cache = interface.into_cache();

        let Ok(frame) = self.surface.get_current_texture() else {
            self.window.request_redraw();
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.renderer
            .present(None, frame.texture.format(), &view, &self.viewport);
        frame.present();
    }

    /// 喂一个原始 winit 事件进这扇窗口自己的 iced 管线。Esc 在这里本地
    /// 处理直接产出 `SearchClose`,不需要主窗口那套"抢在终端转发前特殊
    /// 处理"的手法——这扇窗口里没有终端要竞争按键。逐事件即时重建一次
    /// `UserInterface` 而不是像主窗口那样攒一批再统一处理:这棵视图树
    /// 很小(一个对话框),重建成本可忽略,换来的是不用再维护一份独立的
    /// 事件缓冲/两阶段处理逻辑。
    pub(crate) fn handle_input(&mut self, app: &mut App, event: &WindowEvent) -> Vec<Message> {
        if let WindowEvent::KeyboardInput {
            event: key_event,
            is_synthetic: false,
            ..
        } = event
            && key_event.state == winit::event::ElementState::Pressed
            && key_event.logical_key
                == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            return vec![Message::Search(search::Message::SearchClose)];
        }
        if let WindowEvent::ModifiersChanged(new_modifiers) = event {
            self.modifiers = new_modifiers.state();
        }
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.cursor = mouse::Cursor::Available(conversion::cursor_position(
                *position,
                self.viewport.scale_factor(),
            ));
        }
        let Some(iced_event) =
            conversion::window_event(event.clone(), self.viewport.scale_factor(), self.modifiers)
        else {
            return Vec::new();
        };
        let events: [Event; 1] = [iced_event];
        let Some(ws) = app.active_workspace_mut() else {
            return Vec::new();
        };
        let project_root = ws
            .project
            .as_ref()
            .map(|p| std::path::Path::new(&p.path).to_path_buf());
        let mut interface = UserInterface::build(
            search::search_card(&ws.search, project_root.as_deref()),
            self.viewport.logical_size(),
            std::mem::take(&mut self.cache),
            &mut self.renderer,
        );
        let mut messages = Vec::new();
        let _ = interface.update(
            &events,
            self.cursor,
            &mut self.renderer,
            &mut self.clipboard,
            &mut messages,
        );
        self.cache = interface.into_cache();
        self.window.request_redraw();
        messages.into_iter().map(Message::Search).collect()
    }
```

(`iced_widget::core::widget::operation` 与 `search::{search_card, query_field_id, CaptureQueryFocus, take_query_focused}` 都已在文件顶部 `use` 或用完整路径引用,不需要额外补 `use`;`iced_winit::core::widget::operation::focusable::focus`/`iced_winit::core::Theme`/`iced_winit::core::renderer::Style` 用完整路径写在调用点,同 `window_events.rs` 里已有代码的写法一致,不另加顶层 `use`。)

- [ ] **Step 6: `Ready` 加 `search_overlay` 字段 + `sync_search_overlay` + `window_event` 分流**

`Ready` 枚举变体(Step 1 已加过 `instance`/`adapter`)的字段列表末尾(`proxy` 前后均可)加:

```rust
        /// 独立原生窗口宿主——`None` 表示当前没开。生命周期由
        /// `sync_search_overlay` 按 `ws.search.is_open()` 单向驱动开/关。
        search_overlay: Option<search_overlay::SearchOverlay>,
```

`resumed()` 的 `*self = Self::Ready { ... }` 字面量补 `search_overlay: None,`。

文件顶部 `use` 块(`window_events.rs:8-39`)加:

```rust
use crate::platform::search_overlay;
```

在 `sync_previews`(`window_events.rs:912-1011`)后面新增方法:

```rust
    /// 独立按 `ws.search.is_open()` 开/关 search overlay 窗口,跟
    /// `sync_previews` 同款"每次分发完消息就跑一遍"模式,但各管各的
    /// (webview 池同步跟 overlay 窗口生命周期没有交集)。
    fn sync_search_overlay(&mut self, el: &ActiveEventLoop) {
        let Self::Ready {
            window,
            instance,
            adapter,
            device,
            queue,
            app,
            search_overlay,
            ..
        } = self
        else {
            return;
        };
        match search_overlay::sync_action(app.search_popup_open(), search_overlay.is_some()) {
            search_overlay::SyncAction::Open => {
                let window_width = app.window_size.0;
                *search_overlay = Some(search_overlay::SearchOverlay::open(
                    window,
                    adapter,
                    device,
                    queue,
                    instance,
                    window_width,
                    el,
                ));
            }
            search_overlay::SyncAction::Close => *search_overlay = None,
            search_overlay::SyncAction::Noop => {}
        }
    }
```

（`app.window_size` 是 `App` 的 `pub(crate)` 字段 `(f32, f32)`（`app.rs:345`），不是方法，`window_events.rs` 与 `app.rs` 同一个 crate 内可以直接按字段访问，不需要经过 getter。）

再加 `window_event` 顶部的分流分支。定位:

```bash
command grep -n "fn window_event" crates/dozer-app/src/platform/window_events.rs
```

预期 `window_events.rs:1628`。把签名的 `_window_id: winit::window::WindowId` 改成 `window_id: winit::window::WindowId`(去掉下划线,现在真的要用),在函数体最开头、`let consumed = self.on_window_event(&event);` 之前插入:

```rust
        if let Self::Ready {
            app, search_overlay, ..
        } = self
            && let Some(overlay) = search_overlay
            && window_id == overlay.window_id()
        {
            let close_by_focus_loss = matches!(event, WindowEvent::Focused(false));
            let close_by_request = matches!(event, WindowEvent::CloseRequested);
            if matches!(event, WindowEvent::RedrawRequested) {
                overlay.redraw(app);
            } else if !close_by_focus_loss && !close_by_request {
                for message in overlay.handle_input(app, &event) {
                    self.dispatch(message);
                }
            }
            if close_by_focus_loss || close_by_request {
                self.dispatch(Message::Search(extensions::search::Message::SearchClose));
            }
            self.sync_search_overlay(event_loop);
            return;
        }
```

这段早退保证不动下面那 ~1000 行主窗口的既有 `match`。`user_event`(`window_events.rs:1615`)与 `window_event` 结尾(`window_events.rs:2704` 的 `self.sync_previews();` 后面)都各加一行 `self.sync_search_overlay(event_loop);`(`user_event` 的 `_event_loop` 参数同理去掉下划线改成 `event_loop` 才能用)。

- [ ] **Step 7: 编译检查**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功,无新增 warning。

- [ ] **Step 8: 跑测试**

Run: `cargo test -p dozer-app --bin dozer -- platform::search_overlay`
Expected: Step 3 的 5 个纯函数测试全绿。

Run: `cargo test -p dozer-app --bin dozer`
Expected: 全量测试数字与开工前基线一致(只多 5 个)。

- [ ] **Step 9: 人工验证(过渡态,预期能看到"新旧同开"的重复渲染,这是本任务的正常状态,不是 bug)**

`cargo run -p dozer-app --bin dozer`,文件树右键"搜索"。预期看到**两个**弹窗同时出现:旧的居中遮罩弹窗(`search_modal`,还没删)和新的独立卡片窗口(`SearchOverlay`)。这是过渡期状态——Task 5 会删掉旧路径,现在只需要确认新窗口本身工作正常:能输入、能看到结果、点结果能跳转预览(会经 `Pick` 消息)、Esc 能关闭新窗口(旧的遮罩弹窗此时应该也跟着一起关,因为两者共享同一个 `ws.search.open` 状态)、点主窗口新窗口会消失(失焦关闭,Task 1 已验证过底层假设)。

- [ ] **Step 10: 提交**

```bash
git add crates/dozer-app/src/platform/search_overlay.rs \
        crates/dozer-app/src/platform/mod.rs \
        crates/dozer-app/src/platform/window_events.rs
git commit -m "$(cat <<'EOF'
feat(app): search overlay 独立原生窗口,与旧 search_modal 并存

新增 platform/search_overlay.rs:SearchOverlay 挂成主窗口子窗口 +
AlwaysOnTop,复用主窗口 Device/Queue/Adapter/Instance,只为自己单独建
Surface/Engine/Renderer。Ready 新增 search_overlay 字段 +
sync_search_overlay(),按 ws.search.is_open() 开关;window_event 顶部
新增按 WindowId 分流的早退分支,不动现有主窗口那套事件处理。

过渡态:旧的 search_modal(全窗遮罩+居中卡片)还没删,search 打开时会
同时看到两个弹窗——这是预期状态,下一个任务删旧路径后恢复正常。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: 主窗口 resize 时让 overlay 跟着重新居中

**Files:**
- Modify: `crates/dozer-app/src/platform/window_events.rs`

**Interfaces:**
- Consumes: Task 3 的 `SearchOverlay::reposition`。
- Produces: 无新接口,只是接一个调用点。

- [ ] **Step 1: 定位主窗口 `Resized` 处理**

```bash
command grep -n "WindowEvent::Resized(new_size)" crates/dozer-app/src/platform/window_events.rs
```

预期 `window_events.rs:2537`(注意:这是**主窗口**的 `Resized` 分支,在 Task 3 Step 6 加的早退分支**之后**才会走到,因为早退分支只拦截 `window_id == overlay.window_id()` 的事件,主窗口自己的 `Resized` 不受影响)。

- [ ] **Step 2: 加重新定位调用**

这个分支所在的 `match` 块是从 `Self::Ready { window, device, queue, surface, format, renderer, app, events, viewport, cursor, webview_rects, modifiers, clipboard, cache, resized, .. }` 解构出来的(`window_events.rs:1644-1661`),`search_overlay` 目前被 `..` 吞掉了——加进解构列表:

```bash
command grep -n "let Self::Ready {" crates/dozer-app/src/platform/window_events.rs
```

找到 `window_event` 里那处解构(`window_events.rs:1644` 附近,不是 `sync_previews`/`dispatch`/`sync_search_overlay` 各自独立的解构),`resized,` 那行后面加 `search_overlay,`。

`WindowEvent::Resized(new_size) => { ... }` 分支(`:2537-2552`)末尾追加:

```rust
                    if let Some(overlay) = search_overlay {
                        let scale = window.scale_factor();
                        overlay.reposition(
                            device,
                            window.outer_position().unwrap_or(PhysicalPosition::new(0, 0)),
                            new_size,
                            scale,
                            app.window_size.0,
                        );
                    }
```

需要 `use winit::dpi::PhysicalPosition;`——核对文件顶部 `use winit::{...}` 块(`window_events.rs:29-34`)有没有已经带出 `PhysicalPosition`,没有的话在那个 `use winit::{ dpi::LogicalSize, ... }` 里把 `dpi::LogicalSize` 改成 `dpi::{LogicalSize, PhysicalPosition}`。

- [ ] **Step 3: 编译检查**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功,无新增 warning。

- [ ] **Step 4: 人工验证**

`cargo run -p dozer-app --bin dozer`,打开搜索,拖动主窗口边缘调整大小,overlay 卡片跟着重新居中(位置/尺寸随主窗口宽度变化——宽度变化会改变 `dialog::width(...)` 算出的卡片宽度)。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/platform/window_events.rs
git commit -m "$(cat <<'EOF'
feat(app): 主窗口 resize 时 search overlay 跟随重新居中

with_parent_window 的 addChildWindow 语义只让 overlay 跟随主窗口
"移动",不管"尺寸变化后如何重新布局"——主窗口 Resized 处理里补一步
SearchOverlay::reposition,复用 Task 3 的居中定位纯函数。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: 删除旧渲染路径(原子切换)

**Files:**
- Modify: `crates/dozer-app/src/app/view.rs`
- Modify: `crates/dozer-app/src/extensions/search.rs`
- Modify: `crates/dozer-app/src/app/app.rs`

**Interfaces:**
- Consumes: 无。
- Produces: 无新接口——纯删除,删完之后 search 只剩新的独立窗口路径。

- [ ] **Step 1: `app/view.rs` 删掉旧 stack 组合**

```bash
command grep -n "ws.search_popup_open()" crates/dozer-app/src/app/view.rs
```

预期 `view.rs:131`。把整个 `if ws.search_popup_open() { ... } else if ws.files.tree_delete_confirm_is_some() {` 的第一个 `if` 分支删掉,让 `ws.files.tree_delete_confirm_is_some()` 变成这条 `if/else if` 链的第一个分支:

原文(`view.rs:131-144` 附近):

```rust
        let popped = if ws.search_popup_open() {
            // 文件树右键"搜索"弹窗:窗口级浮层。遮罩"点点即关"由
            // `search_modal` 内部自己处理(整窗 `SCRIM` 做成可点击目标,卡片
            // 是兄弟元素盖在上面),这里只需把弹窗叠在 `base` 之上。
            let project_root = ws.project.as_ref().map(|p| std::path::Path::new(&p.path));
            stack![
                base,
                search::search_modal(&ws.search, project_root, self.window_size.0)
                    .map(Message::Search)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.files.tree_delete_confirm_is_some() {
```

改成:

```rust
        let popped = if ws.files.tree_delete_confirm_is_some() {
```

- [ ] **Step 2: `extensions/search.rs` 删掉 `search_modal()`**

删掉整个 `search_modal` 函数(Task 2 Step 2 写的那版,带"过渡期"文档注释的那个)。`search_card` 保留不动。

- [ ] **Step 3: `app.rs` 的 `preview_desired` 去掉 search 检查**

```bash
command grep -n "let app_modal_open =" crates/dozer-app/src/app/app.rs
```

预期 `app.rs:2737`(这次改动后行号可能已经因为 Task 1-4 的其它修改而挪动,以 `command grep` 实测结果为准)。原文:

```rust
        let app_modal_open =
            ws.search.is_open() || self.text_input_menu.is_some() || self.file_history.is_some();
```

改成:

```rust
        let app_modal_open = self.text_input_menu.is_some() || self.file_history.is_some();
```

同时把这段上面的文档注释(`:2729-2736`)里提到 `search_modal` 的部分删掉或改写,避免注释和代码对不上:

原注释提到"`search_modal` 是满窗 SCRIM+卡片形制...打开时要隐藏 webview";改成只保留 `text_input_menu`/`file_history` 那部分的说明(删掉第一句提 search_modal 的部分,后面"`text_input_menu`...一样按'两侧都可能被盖住'从宽处理"这句里的"和 `search_modal` 一样"几个字也删掉,不再类比一个已经不存在的机制)。

- [ ] **Step 4: 编译检查**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功,无新增 warning。若 `search::search_modal` 还有其它调用点报错("找不到该函数"以外的错误,比如"存在但未使用"),说明还有遗漏的调用点,`command grep -rn "search_modal" crates/dozer-app/src` 找全,同一批处理掉。

- [ ] **Step 5: 跑测试**

Run: `cargo test -p dozer-app --bin dozer`
Expected: 数字与 Task 3 结束时的基线一致(纯删除,不增不减测试)。

- [ ] **Step 6: 人工验证**

`cargo run -p dozer-app --bin dozer`,文件树右键"搜索"——**只**看到新的独立卡片窗口,旧的全窗遮罩不再出现;预览 webview(如果当时开着预览)全程保持可见,不再有"打开搜索、webview 瞬间消失又恢复"的闪烁。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-app/src/app/view.rs \
        crates/dozer-app/src/extensions/search.rs \
        crates/dozer-app/src/app/app.rs
git commit -m "$(cat <<'EOF'
feat(app): 删除 search_modal 旧渲染路径,search overlay 独立窗口转正

app/view.rs 不再把 search_modal 叠进主窗口 stack;search_modal()(全窗
遮罩+居中卡片版本)整个删除,只留 search_card();preview_desired 的
app_modal_open 去掉 ws.search.is_open()——search 打开不再需要强制隐藏
预览 webview,这正是这一系列改动要达成的效果。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: 清理只为旧路径存在的键盘路由/焦点回读 plumbing

**Files:**
- Modify: `crates/dozer-app/src/platform/window_events.rs`
- Modify: `crates/dozer-app/src/app/app.rs`
- Modify: `crates/dozer-app/src/workspace/state.rs`

**Interfaces:**
- Consumes: 无。
- Produces: 无新接口——纯删除。

**背景**(执行前必读,避免删错):这几处代码原本服务于"search 的查询框 `text_input` 活在**主窗口**自己的 `UserInterface` 里"这个前提——主窗口每帧要去查这个输入框是不是真的拿到了 iced 焦点(`CaptureQueryFocus`)、要在弹窗刚打开时手动 `focus` 它(`query_focus_pending`)、要在键盘事件分发时知道"现在是不是该把按键让给这个输入框而不是应用级快捷键"(`app.query_focused()`)、Esc 要抢在终端转发之前特殊处理。Task 3-5 之后,查询框活在**独立窗口**自己的 `UserInterface` 里(`SearchOverlay::redraw`/`handle_input` 已经把等价逻辑搬过去了),主窗口的 `WindowEvent` 流根本不会再收到属于这个输入框的按键/焦点事件——这些代码不是"删了会破坏什么",而是"删了才对得上新架构下的实际情况,不删就是死代码,查询框那部分逻辑永远算不出有意义的值"。

- [ ] **Step 1: 删主窗口 `on_window_event` 里的 Esc-关-search 特殊处理**

```bash
command grep -n "app.search_popup_open()" crates/dozer-app/src/platform/window_events.rs
```

预期在 `on_window_event` 函数体内(`window_events.rs:500` 附近)。删掉整个块:

```rust
        // 右键"搜索"弹窗打开时,Esc 优先:...
        if app.search_popup_open()
            && let WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } = event
            && event.state == ElementState::Pressed
            && event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            app.update(Message::Search(
                crate::extensions::search::Message::SearchClose,
            ));
            window.request_redraw();
            return false;
        }
```

（这段紧挨着右键菜单的 Esc 处理块,删的时候连同它上面那段"右键'搜索'弹窗打开时…"的说明注释一起删,右键菜单那个块本身不动。）

- [ ] **Step 2: 删 `on_window_event` 原生放行闸门里的 `query_focused()`**

```bash
command grep -n "|| app.query_focused()" crates/dozer-app/src/platform/window_events.rs
```

预期 `window_events.rs:768` 附近,一整条大 `||` 链里的一行,直接删掉这一行(`|| app.query_focused()`),链的其它条目不动。

- [ ] **Step 3: 删主窗口 `RedrawRequested` 里 `query_focus_pending` 的消费与应用**

```bash
command grep -n "query_focus_pending" crates/dozer-app/src/platform/window_events.rs
```

预期两处:取值(`:1760-1762` 附近)与应用(`:1927-1933` 附近)。

取值处,删掉这两行:

```rust
                            let query_focus_pending = app
                                .active_workspace_mut()
                                .is_some_and(|ws| ws.take_query_focus_pending());
```

（连同它上面"同理,消费 项目名称编辑/右键搜索 弹窗查询框两个一次性聚焦位…"这条注释里提到"右键搜索"的部分一并改掉,只保留跟"项目名称编辑"相关的说明——那部分还在用。）

应用处,删掉整个 `if query_focus_pending { ... }` 块:

```rust
                            if query_focus_pending {
                                let mut op =
                                    iced_widget::core::widget::operation::focusable::focus::<()>(
                                        extensions::search::query_field_id(),
                                    );
                                crate::runtime::run_operate(&mut interface, renderer, &mut op);
                            }
```

- [ ] **Step 4: 删主窗口 `RedrawRequested` 里 `query_focused` 的实时回读**

```bash
command grep -n "search_popup_open" crates/dozer-app/src/platform/window_events.rs
```

定位 `RedrawRequested` 里那段(`:2294` 附近,不是 Step 1 已经删掉的那处):

```rust
                            // 右键搜索弹窗查询框(Stage 6):全局浮层,不挂靠
                            // 任何 `left_view`,gating 条件用 `search_popup_open`。
                            let query_focused = if app.search_popup_open() {
                                crate::runtime::run_operate(
                                    &mut interface,
                                    renderer,
                                    &mut extensions::search::CaptureQueryFocus,
                                );
                                extensions::search::take_query_focused()
                            } else {
                                false
                            };
```

整段删掉。再定位它的应用点:

```bash
command grep -n "app.set_query_focused(query_focused)" crates/dozer-app/src/platform/window_events.rs
```

预期 `:2485` 附近,删掉这一行:`app.set_query_focused(query_focused);`(若这一行和其它 `app.set_find_query_focused(...)` 写在同一个多行调用里,只删跟 `query_focused` 相关的这一行,`find` 相关的不动)。

- [ ] **Step 5: 删 `App::query_focused`/`App::set_query_focused`**

```bash
command grep -n "fn query_focused\|fn set_query_focused" crates/dozer-app/src/app/app.rs
```

预期 `app.rs:1479-1489` 附近,删掉这两个方法(连同各自的文档注释):

```rust
    /// 右键文件树"搜索"弹窗查询框是否持有 iced 真实焦点(main.rs 原生放行
    /// 闸门用)。
    pub fn query_focused(&self) -> bool {
        self.active_workspace().is_some_and(|ws| ws.query_focused())
    }

    /// 每帧渲染循环读走 `CaptureQueryFocus` 查到的真实焦点态后写进当前
    /// 工作区。
    pub fn set_query_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.search.set_query_focused(focused);
        }
    }
```

`search_popup_open()`(紧挨着这两个方法定义,`app.rs:1472-1475`)**不要删**——`sync_search_overlay`/`app/view.rs` 都还在用它判断"search 是不是开着"。

- [ ] **Step 6: 删 `workspace::state::WorkspaceState::query_focused`**

```bash
command grep -n "fn query_focused" crates/dozer-app/src/workspace/state.rs
```

预期 `state.rs:2214-2215` 附近:

```rust
    pub fn query_focused(&self) -> bool {
        self.search.query_focused()
    }
```

删掉整个方法。`search_popup_open()`(`state.rs:2208-2209` 附近,`self.search.is_open()` 那个)**不要删**,同 Step 5 的理由。`extensions::search::WorkspaceState` 自己的 `query_focused()`/`set_query_focused()`(`search.rs:131-138`)**也不要删**——`query_box()` 的 `active` 样式判断(`search.rs:260`)和 `SearchOverlay::redraw` 都还在用真正的字段访问器,只是这次删的是"套了一层、只服务旧主窗口路径"的外层包装。

- [ ] **Step 7: 编译检查**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功,**无新增 warning**——这一步是本任务的核心验收标准,专门为了不留孤立代码,任何 dead_code/unused 警告都要处理掉才算完成(参照 `[[feedback-verify-plan-completion-independently]]` 的教训:dead_code 警告是路由 bug 的一手信号,这里反过来,新增 dead_code 警告说明删漏了某个只服务旧路径的调用点)。

- [ ] **Step 8: 跑 clippy**

Run: `cargo clippy -p dozer-app --all-targets`
Expected: 无新增 lint。

- [ ] **Step 9: 跑测试**

Run: `cargo test -p dozer-app --bin dozer`
Expected: 数字与 Task 5 结束时的基线一致。

- [ ] **Step 10: 人工验证**

`cargo run -p dozer-app --bin dozer`:
1. 打开搜索,在查询框打字——正常输入,不受影响(路由已经全部走 overlay 自己的事件管线)。
2. 主窗口(不是搜索窗口)按 Esc——不应该有任何反应(不再有旧的"search 开着时主窗口 Esc 也能关"这条路径;真要关闭搜索,点主窗口本身触发失焦即可,或在搜索窗口自己那边按 Esc)。
3. ⌘S(或其它应用级快捷键)在**主窗口聚焦、搜索也开着**的情况下依然正常工作(验证 Step 2 删掉的 `query_focused()` 检查没有误伤到其它键盘路由分支)。

- [ ] **Step 11: 提交**

```bash
git add crates/dozer-app/src/platform/window_events.rs \
        crates/dozer-app/src/app/app.rs \
        crates/dozer-app/src/workspace/state.rs
git commit -m "$(cat <<'EOF'
refactor(app): 删除只服务旧 search_modal 路径的键盘/焦点 plumbing

查询框迁到独立窗口后,主窗口的 WindowEvent 流不会再收到属于它的按键/
焦点事件——删除:主窗口 Esc-关-search 特殊处理(overlay 自己的
handle_input 已本地处理)、原生放行闸门里的 query_focused() 分支、
RedrawRequested 里 query_focus_pending 的消费/应用与 query_focused 的
CaptureQueryFocus 回读、App::query_focused/set_query_focused、
WorkspaceState::query_focused(顶层包装,不是 extensions::search 自己
那份字段访问器)。纯删除,cargo build 无新增 warning 验证没有遗漏。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: 退出时显式释放 overlay + 端到端人工验收 + 收尾

**Files:**
- Modify: `crates/dozer-app/src/platform/window_events.rs`

**Interfaces:**
- Consumes: 无。
- Produces: 无新接口。

- [ ] **Step 1: `CloseRequested` 里显式丢弃 overlay**

```bash
command grep -n "WindowEvent::CloseRequested" crates/dozer-app/src/platform/window_events.rs
```

预期 `window_events.rs:2553` 附近(主窗口的 `CloseRequested`,不是 Task 3 早退分支里 overlay 自己那个)。同 Task 4 一样,先确认这个 `match` 所在的解构块里有没有 `search_overlay`(Task 4 Step 2 应该已经加过,若这里是同一处解构则已经有了,不用重复加)。在 `app.persist_window_size_on_exit();` 之前加一行:

```rust
                    *search_overlay = None; // 图干净,不是正确性要求——Drop 本身就会释放
```

- [ ] **Step 2: 编译检查**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功,无新增 warning。

- [ ] **Step 3: 全量端到端人工验收清单**

`cargo run -p dozer-app --bin dozer`,逐条核对(对应 spec"测试策略"一节列的窗口生命周期项,没有自动化测试手段,这是本计划里唯一需要人工过一遍全部交互的地方):

1. 文件树右键某文件/目录 →"搜索"→ 独立卡片窗口打开,查询框自动获得输入焦点(不用先点一下就能直接打字)。
2. 打字、回车提交 → 结果列表出现、可滚动。
3. 点一条结果 → 跳到对应文件预览,搜索窗口关闭。
4. 重新打开搜索,点卡片右上角 × → 关闭。
5. 重新打开搜索,按 Esc → 关闭。
6. 重新打开搜索,点主窗口任意位置(不是搜索卡片)→ 搜索窗口关闭(失焦即关)。
7. 重新打开搜索,拖动主窗口 → 卡片跟着一起移动。
8. 重新打开搜索,拖动主窗口边缘改变大小 → 卡片重新居中。
9. 预览面板开着某个文件时打开搜索 → 预览 webview 全程可见,不出现"打开搜索瞬间 webview 消失又恢复"的闪烁(这是本计划要修的核心问题)。
10. 连续开关搜索 20 次左右,用 Activity Monitor 粗看 GPU/内存没有持续增长的迹象(spike 已验证过这条,这里是在生产代码路径上复核一遍)。
11. 退出应用(⌘Q 或点关闭按钮),搜索开着的状态下退出不崩溃。

任何一条不符合预期,回到对应任务的 Step 里找问题,不要跳过。

- [ ] **Step 4: 全量检查**

```bash
cargo build -p dozer-app --bin dozer
cargo test -p dozer-app --bin dozer
cargo fmt -- --check
cargo clippy -p dozer-app --all-targets
```

Expected: 全部干净通过,测试数字与开工前基线一致(外加 Task 3 新增的 5 个)。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/platform/window_events.rs
git commit -m "$(cat <<'EOF'
chore(app): 退出时显式释放 search overlay

CloseRequested 处理里在 event_loop.exit() 前显式丢弃 search_overlay,
图干净(Drop 本身已经保证资源释放,这不是正确性要求)。端到端人工验收
清单(spec"测试策略"一节)全部过一遍,收尾这个系列。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## 完工检查

七个任务都完成后:

- [ ] `cargo build -p dozer-app --bin dozer` 无 warning。
- [ ] `cargo test -p dozer-app --bin dozer` 与开工前基线一致(外加 Task 3 的 5 个新用例)。
- [ ] `cargo fmt -- --check` 无差异。
- [ ] `cargo clippy -p dozer-app --all-targets` 无新增 lint。
- [ ] Task 7 Step 3 的端到端人工验收清单全部通过。
- [ ] `command grep -rn "search_modal\b" crates/dozer-app/src` 只剩注释/文档里提到这个已删函数名的历史说明(如果有),没有任何代码还在调用它。
- [ ] 提请审阅,通过后合并回 `main`。
