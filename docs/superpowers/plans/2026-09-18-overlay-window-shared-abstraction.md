# 弹窗独立窗口机制通用化(一) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `search_overlay.rs` 里独立原生窗口的可复用机制(wgpu 渲染管线建立、焦点转移判定、建子窗口样板)抽成小型共享组件,并用它们把 `file_history`(文件历史对比弹窗)也迁到独立窗口,不再需要 `set_visible(false)` 强制隐藏 preview/browser webview。

**Architecture:** 新增 `platform/overlay_gpu.rs`(`OverlayGpu`:wgpu surface/renderer 管理)、`platform/overlay_focus.rs`(`FocusTracker`:失焦关闭判定)、`platform/overlay_window.rs`(`open_child_window`/`centered_overlay_bounds`:建窗+居中算法)三个共享件——不做泛型大一统类型,每个消费方(`SearchOverlay`、新的 `FileHistoryOverlay`)自己组合它们,`redraw`/`handle_input` 各自实现。`Runner::Ready` 新增 `file_history_overlay` 字段 + `OverlayKind` 枚举 + `close_other_overlays` 互斥助手,两类独立窗口弹窗彼此互斥。

**Tech Stack:** Rust workspace;不新增依赖。

**Spec:** `docs/superpowers/specs/2026-09-18-overlay-window-shared-abstraction-design.md`

## Global Constraints

- **在独立分支上开发**:建分支 `feature/overlay-window-shared-abstraction`(或对应 worktree),完成后提请审阅,通过再合并回 `main`。
- **这个计划基于当前 `main`(commit `1f5bcfe`)分析**。工作目录被多个并行会话共享,开工前用本文档的 `grep -n` 模式核对实际行号。
- **不引入新依赖**,`Cargo.toml` 不需要改动。
- **不改变 `file_history` 的业务逻辑**(`State`/`Message`/`update`/git2 查询/diff 渲染)——只改渲染宿主和触发/收起的桥接代码。
- **不在本计划范围内迁移 tab-overflow 下拉/agent picker**(留给第二份 spec,mac 上的 NSMenu 分支全程不动)。
- 每个任务结束都要 `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer && cargo fmt -- --check && cargo clippy -p dozer-app --all-targets` 干净通过,无新增 warning/lint;`cargo test` 数字与开工前基线一致(开工前先跑一次记下基线,新增测试只加不减)。

---

### Task 1: 抽出共享机制(`OverlayGpu`/`FocusTracker`/`open_child_window`) + 重构 `SearchOverlay`

**Files:**
- Create: `crates/dozer-app/src/platform/overlay_gpu.rs`
- Create: `crates/dozer-app/src/platform/overlay_focus.rs`
- Create: `crates/dozer-app/src/platform/overlay_window.rs`
- Modify: `crates/dozer-app/src/platform/mod.rs`
- Modify: `crates/dozer-app/src/platform/search_overlay.rs`(整体重写)

**Interfaces:**
- Produces:
  - `pub(crate) struct OverlayGpu { pub(crate) surface, pub(crate) format, pub(crate) renderer, pub(crate) cache, pub(crate) viewport, pub(crate) clipboard }`,方法 `open(window: &Arc<Window>, instance: &wgpu::Instance, adapter: &wgpu::Adapter, device: &wgpu::Device, queue: &wgpu::Queue, size: PhysicalSize<u32>, scale: f64) -> OverlayGpu`、`reconfigure(&mut self, device: &wgpu::Device, size: PhysicalSize<u32>, scale: f64)`。
  - `pub(crate) struct FocusTracker`(`Default`),方法 `handle_focus(&mut self, focused: bool) -> bool`。
  - `pub(crate) fn centered_overlay_bounds(main_outer_pos, main_inner_size, scale, card_logical_size) -> (PhysicalPosition<i32>, PhysicalSize<u32>)`、`pub(crate) fn open_child_window(main_window: &Window, pos, size, title: &str, el: &ActiveEventLoop) -> Arc<Window>`。
  - `SearchOverlay` 的公开方法签名(`window_id`/`request_redraw`/`open`/`handle_focus`/`reposition`/`redraw`/`handle_input`/`apply_pending_native_menu_edit_key`)与调用方(`window_events.rs`)已有的调用点**完全不变**,`sync_action`/`SyncAction` 也不变——本任务只重构内部实现,不改对外接口。

- [ ] **Step 1: 写 `overlay_gpu.rs`**

```rust
//! 独立原生窗口共用的 wgpu 渲染管线建立/重配置——从 `search_overlay.rs`
//! 抽出,供 `search_overlay.rs`/`file_history_overlay.rs`(以及未来消费方)
//! 共用。不含 view/redraw/input 逻辑,那些各消费方自己写(见
//! `docs/superpowers/specs/2026-09-18-overlay-window-shared-abstraction-
//! design.md`「架构」第 1 节)。

use std::sync::Arc;

use iced_wgpu::graphics::{Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::Clipboard;
use iced_winit::core::{Font, Pixels, Size};
use iced_winit::runtime::user_interface;
use winit::window::Window;

/// 独立原生窗口自己的一份渲染资源——不含 `Device`/`Queue`/`Adapter`/
/// `Instance`(全部从主窗口 `Ready` 借来的共享句柄,`Device`/`Queue`
/// 便宜 `Clone`),`iced_wgpu` 的 `Renderer` 内部持有 `Engine`,不能跨
/// 窗口共享,每扇窗口必须自己一份。字段 `pub(crate)`:消费方
/// (`search_overlay.rs`/`file_history_overlay.rs`)需要在各自的
/// `redraw`/`handle_input` 里对单个字段做细粒度可变借用(比如同时借
/// `&mut renderer` 和 `&viewport`),包一层 getter 反而更啰嗦。
pub(crate) struct OverlayGpu {
    pub(crate) surface: wgpu::Surface<'static>,
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) renderer: Renderer,
    pub(crate) cache: user_interface::Cache,
    pub(crate) viewport: Viewport,
    pub(crate) clipboard: Clipboard,
}

impl OverlayGpu {
    /// 从共享的 `Instance`/`Adapter`/`Device`/`Queue` 为 `window` 建一份
    /// 独立渲染管线,`size`/`scale` 是这扇窗口当前的物理尺寸/缩放。
    pub(crate) fn open(
        window: &Arc<Window>,
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: winit::dpi::PhysicalSize<u32>,
        scale: f64,
    ) -> OverlayGpu {
        let surface = instance
            .create_surface(window.clone())
            .expect("create overlay surface");
        let capabilities = surface.get_capabilities(adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| capabilities.formats.first().copied())
            .expect("get overlay surface format");
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

        OverlayGpu {
            surface,
            format,
            renderer,
            cache: user_interface::Cache::new(),
            viewport,
            clipboard,
        }
    }

    /// resize 后重配置 surface + viewport(不重建 `Engine`/`Renderer`)。
    /// 调用方负责判断"要不要重配"(比如只在窗口实际物理尺寸变化时调,
    /// 见 `SearchOverlay::reposition`/`FileHistoryOverlay::reposition`)。
    pub(crate) fn reconfigure(
        &mut self,
        device: &wgpu::Device,
        size: winit::dpi::PhysicalSize<u32>,
        scale: f64,
    ) {
        self.viewport =
            Viewport::with_physical_size(Size::new(size.width, size.height), scale as f32);
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
```

- [ ] **Step 2: 写 `overlay_focus.rs`**

```rust
//! 独立原生窗口共用的"是否该因失焦而关闭"判定——从 `search_overlay.rs`
//! 抽出。winit 在窗口刚创建时会无条件排一个合成 `Focused(false)`,必须
//! 先收到过真 `Focused(true)`、再收到 `Focused(false)` 才算真正失焦。

/// 焦点转移的纯判定:`(新的 focused 状态, 是否该因失焦关闭)`。
fn focus_transition(was_focused: bool, now_focused: bool) -> (bool, bool) {
    if now_focused {
        (true, false)
    } else if was_focused {
        (false, true)
    } else {
        (false, false)
    }
}

/// 记录一扇独立窗口的真实聚焦历史,`handle_focus` 返回"这次事件是否该
/// 触发关闭"。
#[derive(Default)]
pub(crate) struct FocusTracker {
    focused: bool,
}

impl FocusTracker {
    /// 记录一次焦点事件,返回"是否该因失焦而关闭"。合成的首个
    /// `Focused(false)`(以及任何还没真聚焦过就来的 `Focused(false)`)
    /// 返回 `false`,不触发关闭。
    pub(crate) fn handle_focus(&mut self, focused: bool) -> bool {
        let (new_focused, should_close) = focus_transition(self.focused, focused);
        self.focused = new_focused;
        should_close
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_transition_ignores_synthetic_false_and_closes_on_real_loss() {
        assert_eq!(focus_transition(false, false), (false, false));
        assert_eq!(focus_transition(false, true), (true, false));
        assert_eq!(focus_transition(true, false), (false, true));
        assert_eq!(focus_transition(true, true), (true, false));
    }

    #[test]
    fn handle_focus_tracks_real_focus_and_reports_close_only_on_real_loss() {
        let mut t = FocusTracker::default();
        // 合成的首个 Focused(false):不关闭。
        assert!(!t.handle_focus(false));
        // 真聚焦:不关闭。
        assert!(!t.handle_focus(true));
        // 真失焦:关闭。
        assert!(t.handle_focus(false));
    }
}
```

- [ ] **Step 3: 写 `overlay_window.rs`**

```rust
//! 独立原生窗口共用的建窗样板 + 居中定位算法——从 `search_overlay.rs`
//! 抽出,供 `search_overlay.rs`/`file_history_overlay.rs`(以及未来消费方)
//! 共用。

use std::sync::Arc;

use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowLevel};

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

/// 挂成主窗口子窗口 + `AlwaysOnTop` 的无装饰透明窗口。只建窗口本身,不建
/// wgpu 渲染管线(见 `OverlayGpu`)、不设 IME/原生菜单挂靠(消费方按需
/// 自己调用,大多数消费方不需要——见 spec「架构」第 1 节)。
pub(crate) fn open_child_window(
    main_window: &Window,
    pos: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    title: &str,
    el: &ActiveEventLoop,
) -> Arc<Window> {
    use winit::raw_window_handle::HasWindowHandle;

    let parent_handle = main_window
        .window_handle()
        .expect("main window handle")
        .as_raw();
    let attrs = Window::default_attributes()
        .with_title(title)
        .with_decorations(false)
        .with_transparent(true)
        .with_window_level(WindowLevel::AlwaysOnTop)
        .with_position(pos)
        .with_inner_size(size);
    // Safety: `parent_handle` 取自仍存活的主窗口(`Ready` 持有的
    // `Arc<Window>`),本函数返回前主窗口不会被 drop。
    let attrs = unsafe { attrs.with_parent_window(Some(parent_handle)) };
    let window = Arc::new(el.create_window(attrs).expect("create overlay window"));
    window.focus_window();
    window
}

#[cfg(test)]
mod tests {
    use super::*;

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

- [ ] **Step 4: `platform/mod.rs` 登记三个新模块**

```bash
command grep -n "pub mod search_overlay;" crates/dozer-app/src/platform/mod.rs
```

在这行**之前**(字母序不强制,放一起方便找)加:

```rust
pub mod overlay_focus;
pub mod overlay_gpu;
pub mod overlay_window;
```

- [ ] **Step 5: 整体重写 `search_overlay.rs`,改用三个共享件**

用下面的完整内容**替换整个文件**(不是增量编辑——本文件几乎每个方法体都要改字段访问路径,逐行 diff 比整体重写更容易出错):

```rust
//! search_modal 的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-17-search-modal-overlay-window-design.md`、
//! `docs/superpowers/specs/2026-09-18-overlay-window-shared-abstraction-
//! design.md`(共享机制抽取)。`SearchOverlay` 挂在主窗口 `Runner::Ready`
//! 上,不是独立事件循环——winit 原生按 `WindowId` 把多扇窗口的事件分发
//! 进同一个 `ApplicationHandler`,这扇窗口只是 `window_event` 顶部多出的
//! 一个分支。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::conversion;
use iced_winit::core::{Event, mouse};
use iced_winit::runtime::user_interface::UserInterface;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::extensions::search;
use crate::platform::overlay_focus::FocusTracker;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{centered_overlay_bounds, open_child_window};

/// overlay 卡片的固定逻辑高度,对应现状 `search_modal` 的
/// `max_height(640.0)`。宽度不固定,随主窗口宽度变化(见 `card_logical_size`
/// 调用点,复用 `crate::dialog::width` 的"整窗 1/3"口径)。
const CARD_HEIGHT: f32 = 640.0;

/// 卡片逻辑尺寸:宽度与 `dialog::width(window_width)`(`Length::Fixed(
/// window_width / 3.0)`)保持一致,高度固定 `CARD_HEIGHT`。这里直接算成
/// `f32`,因为 `LogicalSize::new` 要数值而 `dialog::width` 返回 `Length`。
fn card_logical_size(window_width: f32) -> LogicalSize<f32> {
    LogicalSize::new(window_width / 3.0, CARD_HEIGHT)
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

/// 独立原生窗口宿主——`search_card()` 的独立渲染管线。`gpu`/`focus` 组合
/// 了 `overlay_gpu`/`overlay_focus` 两个共享件(见
/// `docs/superpowers/specs/2026-09-18-overlay-window-shared-abstraction-
/// design.md`「架构」第 1 节的取舍:不做一个泛型大一统类型包办一切)。
pub(crate) struct SearchOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    /// 这扇窗口里最近一次右键按下的逻辑坐标——`TextInputMenuOpen` 的原生
    /// 菜单定位复用 `app.files.last_right_click`(全应用共享的坐标缓存,
    /// 不是 Files 专属),在这扇窗口里右键时必须先把它覆写成这扇窗口自己
    /// 的坐标,否则菜单会弹在主窗口上次右键的旧位置(见 `handle_input`)。
    last_right_click: (f32, f32),
    /// 主窗口句柄——`open()` 时把 `chrome::native_menu` 的挂靠目标临时
    /// 指向这扇窗口自己的 NSView,`Drop` 时得指回来,不然这扇窗口关掉后
    /// Files/Project 等主窗口里其它输入框的右键菜单会继续错误地尝试挂在
    /// 一个已经销毁的 NSView 上。
    main_window: Arc<Window>,
}

impl SearchOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    /// 供 `sync_search_overlay` 在"这次分发的消息可能只是改了 `ws.search`
    /// 内容"时补一次重绘(见调用点注释)。
    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// 开一扇挂成主窗口子窗口的独立窗口,复用主窗口的 `Device`/`Queue`/
    /// `Adapter`/`Instance`,只为这扇窗口单独建 `Surface`/`Engine`/
    /// `Renderer`(spike `spike/multi-window-overlay-wry` 已验证这条路径
    /// 可行:见 `docs/superpowers/specs/2026-09-17-multi-window-overlay-
    /// spike-findings.md`)。
    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        window_width: f32,
        el: &ActiveEventLoop,
    ) -> SearchOverlay {
        let scale = main_window.scale_factor();
        let card_logical = card_logical_size(window_width);
        let (pos, size) = centered_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
            scale,
            card_logical,
        );
        let window = open_child_window(main_window, pos, size, "search", el);
        // CJK 组字要靠这个才能拿到候选窗——主窗口在 `resumed()` 里也调了
        // 同一个方法(`window_events.rs:1497`),winit 对新窗口默认关闭 IME,
        // 这扇窗口是独立创建的,不会继承主窗口那次调用的效果。
        window.set_ime_allowed(true);
        // 原生右键菜单(`chrome::native_menu`)靠一个进程级 thread_local
        // 记"该往哪个 NSView 上弹",只在主窗口初始化时装过一次——这扇窗口
        // 打开期间,查询框的剪切/复制/粘贴菜单也要经这条路径,得先把挂靠
        // 目标指过来,`Drop` 里再指回主窗口(见该字段与 `impl Drop` 的文档)。
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&window);

        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);

        SearchOverlay {
            window,
            gpu,
            focus: FocusTracker::default(),
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
            last_right_click: (0.0, 0.0),
            main_window: main_window.clone(),
        }
    }

    /// 记录焦点事件,返回"是否该因失焦而关闭"。
    pub(crate) fn handle_focus(&mut self, focused: bool) -> bool {
        self.focus.handle_focus(focused)
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
        let card_logical = card_logical_size(window_width);
        let (pos, size) =
            centered_overlay_bounds(main_outer_pos, main_inner_size, scale, card_logical);
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.gpu.reconfigure(device, size, scale);
        }
    }

    /// 每帧渲染:走标准 `UserInterface::build → draw → present`。打开后
    /// 第一帧顺带消费"查询框待自动聚焦"一次性位(复用
    /// `extensions::search::open()` 早就在设的那个标记)。
    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(ws) = app.active_workspace_mut() else {
            return;
        };
        let project_root = ws
            .project
            .as_ref()
            .map(|p| std::path::Path::new(&p.path).to_path_buf());
        // 一次性聚焦位要在 `UserInterface` 建立之前取走:接口借住 `ws`
        // (视图树借 `&ws.search`),期间不能再 `&mut ws`。
        let focus_pending = ws.take_query_focus_pending();

        let mut interface = UserInterface::build(
            search::search_card(&ws.search, project_root.as_deref()).map(Message::Search),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );

        if focus_pending {
            let mut op = iced_winit::core::widget::operation::focusable::focus::<()>(
                search::query_field_id(),
            );
            crate::runtime::run_operate(&mut interface, &mut self.gpu.renderer, &mut op);
        }
        crate::runtime::run_operate(
            &mut interface,
            &mut self.gpu.renderer,
            &mut search::CaptureQueryFocus,
        );
        let query_focused = search::take_query_focused();

        let _ = interface.update(
            &[],
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut Vec::new(),
        );
        interface.draw(
            &mut self.gpu.renderer,
            &iced_winit::core::Theme::Dark,
            &iced_winit::core::renderer::Style::default(),
            self.cursor,
        );
        // `into_cache` 消费掉 `interface`,释放它对 `ws` 的借用,下面才能再
        // 可变借用 `ws` 写回焦点态。
        self.gpu.cache = interface.into_cache();
        ws.search.set_query_focused(query_focused);

        let Ok(frame) = self.gpu.surface.get_current_texture() else {
            self.window.request_redraw();
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.gpu
            .renderer
            .present(None, frame.texture.format(), &view, &self.gpu.viewport);
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
                self.gpu.viewport.scale_factor(),
            ));
        }
        // 查询框右键(剪切/复制/粘贴菜单,`extensions::search::Message::
        // TextInputMenuOpen`)复用 `app.files.last_right_click` 这个全应用
        // 共享的坐标缓存定位原生菜单——不先在这里覆写,菜单会弹在主窗口
        // 上一次右键的旧位置(那个字段只有主窗口自己的右键处理会写)。
        if let WindowEvent::MouseInput {
            state: winit::event::ElementState::Pressed,
            button: winit::event::MouseButton::Right,
            ..
        } = event
            && let mouse::Cursor::Available(point) = self.cursor
        {
            self.last_right_click = (point.x, point.y);
            app.files.last_right_click = self.last_right_click;
        }
        let Some(iced_event) = conversion::window_event(
            event.clone(),
            self.gpu.viewport.scale_factor(),
            self.modifiers,
        ) else {
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
            search::search_card(&ws.search, project_root.as_deref()).map(Message::Search),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let mut messages = Vec::new();
        let _ = interface.update(
            &events,
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut messages,
        );
        self.gpu.cache = interface.into_cache();
        self.window.request_redraw();
        messages
    }

    /// 原生右键菜单(查询框的剪切/复制/粘贴/全选)选中一项后,`App::update`
    /// 已经同步弹完 `NSMenu`、把待合成的按键写进
    /// `app.pending_native_menu_edit_key`——主窗口那份等价收尾逻辑
    /// (`window_events.rs` 尾部)建的是 `app.view()`,查询框已经不在那棵
    /// 树里,够不着,所以这扇窗口自己的事件分支里要单独收一遍。只在目标
    /// 确实是查询框时才 `take()`:这个字段是全应用共享的单槽位,`take()`
    /// 会连同"根本不是查询框"的情形一起清空,那样会偷走本该留给主窗口
    /// 其它输入框的合成按键。调用点:`window_event` 的 overlay 分支,
    /// 在 `handle_input` 派发完消息之后、早退之前。
    pub(crate) fn apply_pending_native_menu_edit_key(&mut self, app: &mut App) {
        let is_query_field = app
            .pending_native_menu_edit_key
            .as_ref()
            .is_some_and(|(_, id)| *id == search::query_field_id());
        if !is_query_field {
            return;
        }
        let Some((ch, target_id)) = app.take_pending_native_menu_edit_key() else {
            return;
        };
        let Some(ws) = app.active_workspace_mut() else {
            return;
        };
        let project_root = ws
            .project
            .as_ref()
            .map(|p| std::path::Path::new(&p.path).to_path_buf());
        let mut interface = UserInterface::build(
            search::search_card(&ws.search, project_root.as_deref()).map(Message::Search),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let mut op = iced_winit::core::widget::operation::focusable::focus::<()>(target_id);
        crate::runtime::run_operate(&mut interface, &mut self.gpu.renderer, &mut op);
        let synth = [crate::event::unique_command_event(ch)];
        // 剪切/复制/粘贴/全选作用于纯文本编辑,不会产出需要二次处理的业务
        // 消息(跟主窗口同款收尾逻辑一样直接丢弃产出);置信度来自两边走的
        // 是同一个查询框 `text_input`,行为不会因为宿主窗口不同而分叉。
        let mut ignored = Vec::new();
        let _ = interface.update(
            &synth,
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut ignored,
        );
        self.gpu.cache = interface.into_cache();
        self.window.request_redraw();
    }
}

impl Drop for SearchOverlay {
    /// 把 `chrome::native_menu` 的挂靠目标指回主窗口——`open()` 打开时
    /// 临时指到了这扇窗口自己的 NSView(见那里的调用点注释),这扇窗口
    /// 消失后,Files/Project 等主窗口里其它输入框的右键菜单还得继续正常
    /// 弹在主窗口上。
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&self.main_window);
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
}
```

（`centered_overlay_bounds`/`focus_transition` 的测试已经随函数搬进
`overlay_window.rs`/`overlay_focus.rs`,这里不再重复。）

- [ ] **Step 6: 编译检查**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功,无新增 warning。`window_events.rs` 里所有调用
`SearchOverlay::*`/`search_overlay::sync_action`/`search_overlay::SyncAction`
的地方不需要改——公开接口没变。

- [ ] **Step 7: 跑测试**

Run: `cargo test -p dozer-app --bin dozer -- platform::overlay_gpu platform::overlay_focus platform::overlay_window platform::search_overlay`
Expected:`overlay_focus` 2 个新测试、`overlay_window` 2 个(从
`search_overlay` 搬过去的)、`search_overlay` 剩 3 个 `sync_action` 测试,
全绿。`overlay_gpu` 没有单测(纯 wgpu 资源管理,同 `search_overlay.rs`
一贯的"这类代码没有自动化测试手段"处理)。

Run: `cargo test -p dozer-app --bin dozer`
Expected: 全量数字与开工前基线一致(净增 2 个:`overlay_focus` 多了
`handle_focus_tracks_real_focus_and_reports_close_only_on_real_loss`
这一个新测试,`centered_overlay_bounds`/`focus_transition` 的测试只是
搬了模块,数量不变)。

- [ ] **Step 8: `cargo fmt`/`clippy`**

Run: `cargo fmt -- --check && cargo clippy -p dozer-app --all-targets`
Expected: 无差异、无新增 lint。

- [ ] **Step 9: 人工验证(纯重构,行为应与重构前完全一致)**

`cargo run -p dozer-app --bin dozer`,文件树右键"搜索",走一遍现有
验收清单的核心几项:自动聚焦、打字/结果、Esc 关闭、点主窗口失焦关闭、
拖动主窗口跟随、resize 后重新居中。都应该和重构前完全一样(本任务
不改变任何可观察行为)。

- [ ] **Step 10: 提交**

```bash
git add crates/dozer-app/src/platform/overlay_gpu.rs \
        crates/dozer-app/src/platform/overlay_focus.rs \
        crates/dozer-app/src/platform/overlay_window.rs \
        crates/dozer-app/src/platform/mod.rs \
        crates/dozer-app/src/platform/search_overlay.rs
git commit -m "$(cat <<'EOF'
refactor(app): 抽出独立窗口共享机制(OverlayGpu/FocusTracker/建窗函数)

从 search_overlay.rs 抽出三个可复用组件:OverlayGpu(wgpu surface/
renderer 建立与 resize 重配)、FocusTracker(失焦关闭判定,含 winit
创建窗口时合成 Focused(false) 的忽略逻辑)、open_child_window +
centered_overlay_bounds(建子窗口样板 + 居中定位算法)。不做泛型大一统
类型包办一切(设计已否决,见 docs/superpowers/specs/2026-09-18-
overlay-window-shared-abstraction-design.md),SearchOverlay 改成组合
这三者,redraw/handle_input 等业务逻辑不变、纯重构。公开接口不变,
window_events.rs 调用点不需要改。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: 从 `popup_view` 拆出无外层容器的 `file_history_card`

**Files:**
- Modify: `crates/dozer-app/src/extensions/file_history.rs`

**Interfaces:**
- Consumes: 无新依赖,纯内部重构。
- Produces: `pub fn file_history_card(state: &State) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>`——卡片本体(标题 + 双栏内容),`Length::Fill` 填满调用方画布。旧 `popup_view(state, window_width, window_height)` 保留,内部改调 `file_history_card`,签名/行为不变。

- [ ] **Step 1: 定位现有 `popup_view`**

```bash
command grep -n "pub fn popup_view" crates/dozer-app/src/extensions/file_history.rs
```

预期在 `file_history.rs:376`。

- [ ] **Step 2: 把函数体拆成两半**

把 `popup_view`(`file_history.rs:376-427`)改成:

```rust
/// 弹窗卡片本体(标题 + 左侧提交列表 + 右侧 diff 区),无外层居中容器——
/// 这次拆分是为了让独立 overlay 窗口(`platform/file_history_overlay.rs`)
/// 能直接复用同一份视图逻辑,只是换一个宿主(独立窗口取代 `App::view()`
/// 的 `stack!` 层)。`Length::Fill`:调用方现在总是给一块已经量好的
/// 画布(独立窗口整扇画布),不需要 `popup_view` 原本那种按
/// `window_width`/`window_height` 算 `Length::Fixed` 像素值再居中的
/// 语义(那部分逻辑挪进 `FileHistoryOverlay::open`/`reposition` 的
/// 窗口尺寸计算,不在这里)。
pub fn file_history_card(
    state: &State,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let title = row![
        icons::view(
            icons::IconKind::History,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().cream,
        ),
        text("文件历史")
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().cream),
        iced_widget::space::horizontal(),
        button(
            text("×")
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().dim)
        )
        .on_press(Message::Close)
        .style(|_t, _s| iced_widget::button::Style {
            background: None,
            text_color: byteui::theme::color::current().dim,
            ..iced_widget::button::Style::default()
        }),
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);

    let body = row![
        container(commit_list_view(state))
            .width(Length::Fixed(240.0))
            .height(Length::Fill),
        diff_area_view(state),
    ]
    .spacing(12)
    .height(Length::Fill);

    container(column![title, body].spacing(12))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(16)
        .style(crate::dialog::card_style)
        .into()
}

/// 弹窗全貌:`file_history_card` 套一层"限定像素尺寸 + 全窗居中"的外壳。
/// 布局参照 `project::project_delete_confirm_popup` 的窗口级卡片外壳,
/// 宽度/高度不用 `dialog::width`(那是"整窗 1/3"的确认框默认值,内容是
/// 左右分栏的提交列表 + diff,1/3 窗宽放不下)。
///
/// 2026-09-18:迁独立原生窗口过渡期的旧路径,`file_history_card()` 是
/// 新路径(`platform/file_history_overlay.rs`)复用的部分——本函数连同
/// 调用它的 `app/view.rs:560-573` 那段会在本计划 Task 4 一并删除,过渡
/// 期内暂时保留让现状行为不受影响。
pub fn popup_view<'a>(
    state: &'a State,
    window_width: f32,
    window_height: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let dialog = container(file_history_card(state))
        .width(Length::Fixed(window_width * 0.75))
        .height(Length::Fixed(window_height * 0.8));

    container(dialog)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}
```

- [ ] **Step 3: 编译检查**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功,无新增 warning。

- [ ] **Step 4: 跑测试**

Run: `cargo test -p dozer-app --bin dozer -- extensions::file_history`
Expected: 既有测试全绿(纯重构,不碰这次改的 view 函数)。

- [ ] **Step 5: 人工视觉核对**

`cargo run -p dozer-app --bin dozer`,文件树右键某文件"查看此文件历史",
弹窗视觉/交互与改动前一致(标题、双栏、点提交切 diff、回滚按钮、×
关闭、点遮罩关闭)。

- [ ] **Step 6: 提交**

```bash
git add crates/dozer-app/src/extensions/file_history.rs
git commit -m "$(cat <<'EOF'
refactor(app): 从 popup_view 拆出无外层容器的 file_history_card

为独立原生窗口迁移(设计见 docs/superpowers/specs/2026-09-18-
overlay-window-shared-abstraction-design.md)做准备:file_history_card
是卡片本体,popup_view 变成"套一层旧路径尺寸约束"的过渡期外壳,过渡期
结束后连同 app/view.rs 里的 stack 组合一起删除。纯重构,行为不变。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: `FileHistoryOverlay` + 互斥机制 + 全线接入

**Files:**
- Create: `crates/dozer-app/src/platform/file_history_overlay.rs`
- Modify: `crates/dozer-app/src/platform/mod.rs`
- Modify: `crates/dozer-app/src/platform/window_events.rs`

**Interfaces:**
- Consumes:Task 1 的 `OverlayGpu`/`FocusTracker`/`centered_overlay_bounds`/`open_child_window`;Task 2 的 `file_history::file_history_card`;`Ready` 已有字段 `window`/`instance`/`adapter`/`device`/`queue`/`app`/`search_overlay`。
- Produces:
  - `pub(crate) struct FileHistoryOverlay`,方法:`open(main_window: &Arc<Window>, adapter, device, queue, instance, window_width: f32, window_height: f32, el: &ActiveEventLoop) -> FileHistoryOverlay`、`window_id(&self) -> WindowId`、`request_redraw(&self)`、`handle_focus(&mut self, focused: bool) -> bool`、`reposition(&mut self, device, main_outer_pos, main_inner_size, scale, window_width: f32, window_height: f32)`、`redraw(&mut self, app: &mut App)`、`handle_input(&mut self, app: &mut App, event: &WindowEvent) -> Vec<Message>`。
  - `pub(crate) enum SyncAction { Open, Close, Noop }` + `pub(crate) fn sync_action(open: bool, overlay_present: bool) -> SyncAction`(同 `search_overlay` 那份,独立一份,不共享枚举——`file_history` 的"开关判定"跟 `search` 的判定逻辑相同但语义上是两件独立的事,共享一个类型反而让调用点看不出"这次判定的是哪个")。
  - `Runner::Ready` 新增字段 `file_history_overlay: Option<file_history_overlay::FileHistoryOverlay>`。
  - `Runner` 新增 `pub(crate) enum OverlayKind { Search, FileHistory }`、`fn close_other_overlays(&mut self, keep: OverlayKind)`、`fn sync_file_history_overlay(&mut self, el: &ActiveEventLoop)`。

- [ ] **Step 1: 写 `file_history_overlay.rs`**

```rust
//! file_history 弹窗的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-18-overlay-window-shared-abstraction-
//! design.md`。结构对照 `search_overlay.rs::SearchOverlay`——没有文本
//! 输入,所以没有 IME/原生右键菜单挂靠、没有查询框自动聚焦这些机制,
//! 比 `SearchOverlay` 更简单。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::conversion;
use iced_winit::core::{Event, mouse};
use iced_winit::runtime::user_interface::UserInterface;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::extensions::file_history;
use crate::platform::overlay_focus::FocusTracker;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{centered_overlay_bounds, open_child_window};

/// 卡片逻辑尺寸:宽 = 主窗口宽度的 75%,高 = 主窗口高度的 80%——同现状
/// `file_history::popup_view` 的比例。
fn card_logical_size(window_width: f32, window_height: f32) -> LogicalSize<f32> {
    LogicalSize::new(window_width * 0.75, window_height * 0.8)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncAction {
    Open,
    Close,
    Noop,
}

pub(crate) fn sync_action(open: bool, overlay_present: bool) -> SyncAction {
    match (open, overlay_present) {
        (true, false) => SyncAction::Open,
        (false, true) => SyncAction::Close,
        (true, true) | (false, false) => SyncAction::Noop,
    }
}

pub(crate) struct FileHistoryOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
}

impl FileHistoryOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        window_width: f32,
        window_height: f32,
        el: &ActiveEventLoop,
    ) -> FileHistoryOverlay {
        let scale = main_window.scale_factor();
        let card_logical = card_logical_size(window_width, window_height);
        let (pos, size) = centered_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
            scale,
            card_logical,
        );
        let window = open_child_window(main_window, pos, size, "file-history", el);
        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);

        FileHistoryOverlay {
            window,
            gpu,
            focus: FocusTracker::default(),
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
        }
    }

    pub(crate) fn handle_focus(&mut self, focused: bool) -> bool {
        self.focus.handle_focus(focused)
    }

    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
        window_width: f32,
        window_height: f32,
    ) {
        let card_logical = card_logical_size(window_width, window_height);
        let (pos, size) =
            centered_overlay_bounds(main_outer_pos, main_inner_size, scale, card_logical);
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.gpu.reconfigure(device, size, scale);
        }
    }

    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(state) = app.file_history.as_ref() else {
            return;
        };
        let mut interface = UserInterface::build(
            file_history::file_history_card(state).map(Message::FileHistory),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let _ = interface.update(
            &[],
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut Vec::new(),
        );
        interface.draw(
            &mut self.gpu.renderer,
            &iced_winit::core::Theme::Dark,
            &iced_winit::core::renderer::Style::default(),
            self.cursor,
        );
        self.gpu.cache = interface.into_cache();

        let Ok(frame) = self.gpu.surface.get_current_texture() else {
            self.window.request_redraw();
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.gpu
            .renderer
            .present(None, frame.texture.format(), &view, &self.gpu.viewport);
        frame.present();
    }

    /// 喂一个原始 winit 事件进这扇窗口自己的 iced 管线。同
    /// `SearchOverlay::handle_input`,逐事件即时重建 `UserInterface`(这棵
    /// 视图树重建成本可忽略,换取不用维护独立的事件缓冲)。
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
            return vec![Message::FileHistory(file_history::Message::Close)];
        }
        if let WindowEvent::ModifiersChanged(new_modifiers) = event {
            self.modifiers = new_modifiers.state();
        }
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.cursor = mouse::Cursor::Available(conversion::cursor_position(
                *position,
                self.gpu.viewport.scale_factor(),
            ));
        }
        let Some(iced_event) = conversion::window_event(
            event.clone(),
            self.gpu.viewport.scale_factor(),
            self.modifiers,
        ) else {
            return Vec::new();
        };
        let events: [Event; 1] = [iced_event];
        let Some(state) = app.file_history.as_ref() else {
            return Vec::new();
        };
        let mut interface = UserInterface::build(
            file_history::file_history_card(state).map(Message::FileHistory),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let mut messages = Vec::new();
        let _ = interface.update(
            &events,
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut messages,
        );
        self.gpu.cache = interface.into_cache();
        self.window.request_redraw();
        messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_action_opens_when_open_and_no_overlay() {
        assert_eq!(sync_action(true, false), SyncAction::Open);
    }

    #[test]
    fn sync_action_closes_when_closed_but_overlay_present() {
        assert_eq!(sync_action(false, true), SyncAction::Close);
    }

    #[test]
    fn sync_action_noop_when_states_already_match() {
        assert_eq!(sync_action(true, true), SyncAction::Noop);
        assert_eq!(sync_action(false, false), SyncAction::Noop);
    }
}
```

（`FileHistoryOverlay` 没有 `impl Drop`——不像 `SearchOverlay` 要指回
`native_menu` 挂靠目标,`file_history` 没有文本输入,不接触原生菜单。)

- [ ] **Step 2: `platform/mod.rs` 登记新模块**

```bash
command grep -n "pub mod file_drag;" crates/dozer-app/src/platform/mod.rs
```

在这行后面加:

```rust
pub mod file_history_overlay;
```

- [ ] **Step 3: `Ready` 加 `file_history_overlay` 字段**

```bash
command grep -n "search_overlay: Option<search_overlay::SearchOverlay>," crates/dozer-app/src/platform/window_events.rs
```

预期 `window_events.rs:135`。这行后面加:

```rust
        /// 同 `search_overlay`,`file_history` 弹窗的独立窗口宿主。生命
        /// 周期由 `sync_file_history_overlay` 按 `app.file_history.
        /// is_some()` 单向驱动开/关,且与 `search_overlay` 互斥(见
        /// `OverlayKind`/`close_other_overlays`)。
        file_history_overlay: Option<file_history_overlay::FileHistoryOverlay>,
```

定位 `resumed()` 的 `*self = Self::Ready { ... }` 字面量:

```bash
command grep -n "search_overlay: None," crates/dozer-app/src/platform/window_events.rs
```

这行后面加 `file_history_overlay: None,`。

文件顶部 `use` 块(`window_events.rs:8-39` 附近,`use crate::platform::
search_overlay;` 那一行旁边)加:

```rust
use crate::platform::file_history_overlay;
```

- [ ] **Step 4: `OverlayKind` + `close_other_overlays`**

```bash
command grep -n "pub(crate) enum FocusIntent" crates/dozer-app/src/platform/window_events.rs
```

在 `FocusIntent` 定义(`window_events.rs:130-134` 附近)后面加:

```rust
/// 独立窗口弹窗的种类,给"开一个就关掉其它已开的"这条互斥规则用(见
/// `docs/superpowers/specs/2026-09-18-overlay-window-shared-abstraction-
/// design.md`「架构」第 2 节)。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum OverlayKind {
    Search,
    FileHistory,
}
```

在 `sync_search_overlay` 方法(`window_events.rs:1039` 附近)**前面**加:

```rust
    /// 打开任意一类独立窗口弹窗前,先关掉其余已开的——见 `OverlayKind`
    /// 文档。现状代码里"旧弹窗状态没真正清空、被高优先级弹窗遮住之后又
    /// 冒出来"是已确认的真实漂移(见 spec「架构」第 2 节),独立窗口没有
    /// `App::view()` 那种渲染优先级兜底,必须显式互斥。
    fn close_other_overlays(&mut self, keep: OverlayKind) {
        let Self::Ready {
            search_overlay,
            file_history_overlay,
            ..
        } = self
        else {
            return;
        };
        if keep != OverlayKind::Search {
            *search_overlay = None;
        }
        if keep != OverlayKind::FileHistory {
            *file_history_overlay = None;
        }
    }
```

- [ ] **Step 5: 改 `sync_search_overlay`,开窗前先互斥**

```bash
command grep -n "fn sync_search_overlay" crates/dozer-app/src/platform/window_events.rs
```

把整个方法体(`window_events.rs:1039-1080`)改成:

```rust
    fn sync_search_overlay(&mut self, el: &winit::event_loop::ActiveEventLoop) {
        let action = {
            let Self::Ready {
                app, search_overlay, ..
            } = self
            else {
                return;
            };
            search_overlay::sync_action(app.search_popup_open(), search_overlay.is_some())
        };
        match action {
            search_overlay::SyncAction::Open => {
                self.close_other_overlays(OverlayKind::Search);
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
            search_overlay::SyncAction::Close => {
                let Self::Ready { search_overlay, .. } = self else {
                    return;
                };
                *search_overlay = None;
            }
            search_overlay::SyncAction::Noop => {}
        }
        // 这次分发的消息可能只是改了 `ws.search` 的内容(典型例子:异步
        // `SearchResults` 从 `user_event` 落地),不涉及开/关窗口,上面的
        // `match` 不会碰它——但内容变了就得让这扇窗口重绘,不然会一直停在
        // "搜索中…"直到用户碰巧在这扇窗口里移动一下鼠标。不判断"到底是
        // 不是真的有变化",跟主窗口 `dispatch()` 对几乎每条消息都无条件
        // `request_redraw()` 的既有尺度一致。
        let Self::Ready { search_overlay, .. } = self else {
            return;
        };
        if let Some(overlay) = search_overlay {
            overlay.request_redraw();
        }
    }
```

（跟改动前相比,唯一的行为变化是 `SyncAction::Open` 分支现在先调
`close_other_overlays(OverlayKind::Search)`;其余逻辑原样保留,只是拆成
多个小的 `let Self::Ready { .. } = self else { return }` 块——`Open`
分支要在中途插入一次 `self.close_other_overlays(...)`(需要整个
`&mut self`),不能跟后面 `SearchOverlay::open(...)` 需要的字段解构写在
同一个块里,否则会有 NLL 借用冲突。）

- [ ] **Step 6: 加 `sync_file_history_overlay`(镜像 Step 5)**

紧接着 `sync_search_overlay` 方法后面加:

```rust
    /// 同 `sync_search_overlay`,按 `app.file_history.is_some()` 开/关
    /// file_history overlay 窗口。
    fn sync_file_history_overlay(&mut self, el: &winit::event_loop::ActiveEventLoop) {
        let action = {
            let Self::Ready {
                app,
                file_history_overlay,
                ..
            } = self
            else {
                return;
            };
            file_history_overlay::sync_action(app.file_history.is_some(), file_history_overlay.is_some())
        };
        match action {
            file_history_overlay::SyncAction::Open => {
                self.close_other_overlays(OverlayKind::FileHistory);
                let Self::Ready {
                    window,
                    instance,
                    adapter,
                    device,
                    queue,
                    app,
                    file_history_overlay,
                    ..
                } = self
                else {
                    return;
                };
                let (window_width, window_height) = app.window_size;
                *file_history_overlay = Some(file_history_overlay::FileHistoryOverlay::open(
                    window,
                    adapter,
                    device,
                    queue,
                    instance,
                    window_width,
                    window_height,
                    el,
                ));
            }
            file_history_overlay::SyncAction::Close => {
                let Self::Ready {
                    file_history_overlay,
                    ..
                } = self
                else {
                    return;
                };
                *file_history_overlay = None;
            }
            file_history_overlay::SyncAction::Noop => {}
        }
        let Self::Ready {
            file_history_overlay,
            ..
        } = self
        else {
            return;
        };
        if let Some(overlay) = file_history_overlay {
            overlay.request_redraw();
        }
    }
```

- [ ] **Step 7: `window_event` 顶部加 file_history 的按 `WindowId` 分流分支**

```bash
command grep -n "self.sync_search_overlay(event_loop);" crates/dozer-app/src/platform/window_events.rs
```

预期在 `window_event` 里那处(`window_events.rs:1757` 附近,search
overlay 早退分支的结尾,`return;` 之前)。在这个 `return;` **之后**、
`let consumed = self.on_window_event(&event);` **之前**插入一段新的
早退分支(结构镜像 search 那段,细节更简单——没有原生菜单收尾、没有
`sync_previews`/`apply_pending_focus` 三件套):

```rust
        // file_history overlay 窗口自己那份 `WindowId` 的事件,同 search
        // overlay 早退分支的手法,互相独立。
        if let Self::Ready {
            app,
            file_history_overlay,
            ..
        } = self
            && let Some(overlay) = file_history_overlay
            && window_id == overlay.window_id()
        {
            if matches!(event, WindowEvent::RedrawRequested) {
                overlay.redraw(app);
            } else if matches!(event, WindowEvent::CloseRequested) {
                self.dispatch(Message::FileHistory(
                    extensions::file_history::Message::Close,
                ));
            } else if let WindowEvent::Focused(focused) = event {
                if overlay.handle_focus(focused) {
                    self.dispatch(Message::FileHistory(
                        extensions::file_history::Message::Close,
                    ));
                }
            } else {
                for message in overlay.handle_input(app, &event) {
                    self.dispatch(message);
                }
            }
            self.sync_file_history_overlay(event_loop);
            return;
        }
```

- [ ] **Step 8: `user_event`/主窗口 `window_event` 尾部各加一次调用**

```bash
command grep -n "self.sync_search_overlay(event_loop);" crates/dozer-app/src/platform/window_events.rs
```

预期两处:`user_event`(`window_events.rs:1698` 附近)与主窗口
`window_event` 的最尾部(`window_events.rs:2827` 附近,Step 7 新分支
**之外**、原有那一处)。两处都在 `self.sync_search_overlay(event_loop);`
后面紧接着加一行:

```rust
        self.sync_file_history_overlay(event_loop);
```

- [ ] **Step 9: 主窗口 `Resized` 处理追加 file_history overlay 重定位**

```bash
command grep -n "if let Some(overlay) = search_overlay {" crates/dozer-app/src/platform/window_events.rs
```

预期 `window_events.rs:2658` 附近(`WindowEvent::Resized` 分支内)。这段
`if let Some(overlay) = search_overlay { overlay.reposition(...); }`
**后面**加:

```rust
                    if let Some(overlay) = file_history_overlay {
                        overlay.reposition(
                            device,
                            window
                                .outer_position()
                                .unwrap_or(winit::dpi::PhysicalPosition::new(0, 0)),
                            new_size,
                            window.scale_factor(),
                            app.window_size.0,
                            app.window_size.1,
                        );
                    }
```

这处 `Resized` 分支所在的解构块(`window_events.rs:1720` 附近的
`let Self::Ready { window, device, queue, surface, format, renderer,
app, events, viewport, cursor, webview_rects, modifiers, clipboard,
cache, resized, search_overlay, .. } = self`)需要把 `file_history_overlay`
也加进解构列表——`search_overlay,` 那行后面加 `file_history_overlay,`。

- [ ] **Step 10: 主窗口 `CloseRequested` 处理追加释放**

```bash
command grep -n "\*search_overlay = None; // 图干净" crates/dozer-app/src/platform/window_events.rs
```

预期 `window_events.rs:2673` 附近。这行后面加:

```rust
                    *file_history_overlay = None; // 同上,图干净。
```

（`file_history_overlay` 已经在 Step 9 加进了这个 `match` 所在的解构
块,这里不用重复加。）

- [ ] **Step 11: 编译检查**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功,无新增 warning。若报字段/变量未解构进某个 `let
Self::Ready {...}` 块的错误,按报错信息核对该块的字段列表补齐。

- [ ] **Step 12: 跑测试**

Run: `cargo test -p dozer-app --bin dozer -- platform::file_history_overlay`
Expected: 3 个新 `sync_action` 测试全绿。

Run: `cargo test -p dozer-app --bin dozer`
Expected: 数字比 Task 1 结束时多 3(`file_history_overlay` 的
`sync_action` 测试)。

- [ ] **Step 13: `cargo fmt`/`clippy`**

Run: `cargo fmt -- --check && cargo clippy -p dozer-app --all-targets`
Expected: 无差异、无新增 lint。

- [ ] **Step 14: 人工验证(过渡态,预期"新旧同开"的重复渲染 + 互斥生效)**

`cargo run -p dozer-app --bin dozer`:

1. 文件树右键某文件"查看此文件历史"——预期看到**两个**弹窗同时出现:
   旧的全窗遮罩+居中卡片(`popup_view`,还没删)和新的独立窗口
   (`FileHistoryOverlay`)。这是过渡期状态,Task 4 会删掉旧路径。
2. 新窗口能看到标题/双栏内容/点提交切 diff,Esc 能关(旧遮罩弹窗此时
   应该也跟着一起关,因为两者共享同一个 `app.file_history` 状态)。
3. **互斥验证**:打开 file_history 独立窗口,再去文件树右键"搜索"——
   file_history 的独立窗口应该消失(被 `close_other_overlays` 关掉),
   只剩搜索的独立窗口。反过来,search 开着时打开 file_history,search
   窗口应该消失。

- [ ] **Step 15: 提交**

```bash
git add crates/dozer-app/src/platform/file_history_overlay.rs \
        crates/dozer-app/src/platform/mod.rs \
        crates/dozer-app/src/platform/window_events.rs
git commit -m "$(cat <<'EOF'
feat(app): file_history overlay 独立原生窗口,与 search 互斥、与旧路径并存

新增 platform/file_history_overlay.rs:FileHistoryOverlay 组合 Task 1
抽出的 OverlayGpu/FocusTracker/open_child_window,比 SearchOverlay 更
简单(无文本输入,不需要 IME/原生菜单挂靠)。Ready 新增 file_history_
overlay 字段 + sync_file_history_overlay(),按 app.file_history.
is_some() 开关;新增 OverlayKind + close_other_overlays,search/
file_history 两类独立窗口弹窗互相强制互斥(开一个关另一个),堵上现状
代码里"旧弹窗状态没真正清空、被高优先级弹窗遮住之后又冒出来"这个已
确认的漂移点。

过渡态:旧的 file_history popup_view(全窗遮罩+居中卡片)还没删,打开
file_history 时会同时看到两个弹窗——这是预期状态,下一个任务删旧路径
后恢复正常。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: 删除旧渲染路径(原子切换) + 端到端验收 + 收尾

**Files:**
- Modify: `crates/dozer-app/src/app/view.rs`
- Modify: `crates/dozer-app/src/extensions/file_history.rs`
- Modify: `crates/dozer-app/src/app/app.rs`

**Interfaces:**
- Consumes: 无。
- Produces: 无新接口——纯删除,删完之后 `file_history` 只剩新的独立窗口路径。

- [ ] **Step 1: `app/view.rs` 删掉旧 stack 组合**

```bash
command grep -n "self.file_history.as_ref()" crates/dozer-app/src/app/view.rs
```

预期 `view.rs:560`。把整个 `} else if let Some(state) =
self.file_history.as_ref() { ... }` 分支(`view.rs:560-573`)删掉,让
它前后两个分支直接相连(前一个分支的 `}` 后面接 `else if` 链的下一个
分支,不留这一段)。

- [ ] **Step 2: `extensions/file_history.rs` 删掉 `popup_view`**

删掉整个 `popup_view` 函数(Task 2 Step 2 写的那版,带"过渡期"文档
注释的那个)。`file_history_card` 保留不动。

- [ ] **Step 3: `app.rs` 的 `preview_desired`/`browser_desired` 去掉 file_history 检查**

```bash
command grep -n "self.file_history.is_some()" crates/dozer-app/src/app/app.rs
```

预期两处(`preview_desired`/`browser_desired` 各一个,行号以实测为准)。
两处原文都是:

```rust
        let app_modal_open = self.text_input_menu.is_some() || self.file_history.is_some();
```

都改成:

```rust
        let app_modal_open = self.text_input_menu.is_some();
```

- [ ] **Step 4: 编译检查**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功,无新增 warning。若 `file_history::popup_view` 还有
其它调用点报错,`command grep -rn "file_history::popup_view\|popup_view("
crates/dozer-app/src` 找全,同一批处理掉。

- [ ] **Step 5: 跑测试**

Run: `cargo test -p dozer-app --bin dozer`
Expected: 数字与 Task 3 结束时的基线一致(纯删除,不增不减测试)。

- [ ] **Step 6: `cargo fmt`/`clippy`**

Run: `cargo fmt -- --check && cargo clippy -p dozer-app --all-targets`
Expected: 无差异、无新增 lint。

- [ ] **Step 7: 端到端人工验收清单**

`cargo run -p dozer-app --bin dozer`,逐条核对:

1. 文件树右键某文件"查看此文件历史"→ **只**看到新的独立窗口,旧的
   全窗遮罩不再出现。
2. 点提交列表切换 → 右侧 diff 正确刷新;预览面板(如果当时开着预览)
   全程保持可见,不再有"打开 file_history、webview 瞬间消失又恢复"的
   闪烁(这是本计划要修的核心问题)。
3. 点回滚按钮 → 走既有回滚流程,行为与改动前一致。
4. × 按钮 / Esc / 点主窗口(失焦)三种方式都能关闭。
5. 拖动主窗口 → 独立窗口跟着一起移动;resize 主窗口 → 重新居中,
   尺寸跟着 `window_width*0.75`/`window_height*0.8` 变化。
6. **互斥复核**:file_history 开着时打开 search,file_history 应该
   自动关闭;反过来同理。
7. 连续开关 file_history 20 次左右,用 Activity Monitor 粗看 GPU/
   内存没有持续增长的迹象。
8. 退出应用(⌘Q 或点关闭按钮),file_history 开着的状态下退出不崩溃。

任何一条不符合预期,回到对应任务的 Step 里找问题,不要跳过。

- [ ] **Step 8: 提交**

```bash
git add crates/dozer-app/src/app/view.rs \
        crates/dozer-app/src/extensions/file_history.rs \
        crates/dozer-app/src/app/app.rs
git commit -m "$(cat <<'EOF'
feat(app): 删除 file_history 旧渲染路径,独立窗口转正

app/view.rs 不再把 file_history 叠进主窗口 stack;popup_view(全窗遮罩+
居中卡片版本)整个删除,只留 file_history_card();preview_desired/
browser_desired 的 app_modal_open 都去掉 self.file_history.is_some()
——file_history 打开不再需要强制隐藏预览/浏览器 webview,这正是这一系列
改动要达成的效果。端到端人工验收清单全部过一遍,收尾这个系列。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## 完工检查

四个任务都完成后:

- [ ] `cargo build -p dozer-app --bin dozer` 无 warning。
- [ ] `cargo test -p dozer-app --bin dozer` 与开工前基线一致(外加
  Task 1 的 1 个新用例 + Task 3 的 3 个新用例,净增 4 个)。
- [ ] `cargo fmt -- --check` 无差异。
- [ ] `cargo clippy -p dozer-app --all-targets` 无新增 lint。
- [ ] Task 4 Step 7 的端到端人工验收清单全部通过。
- [ ] `command grep -rn "file_history::popup_view\b" crates/dozer-app/src`
  没有任何代码还在调用它(只可能剩注释/文档里的历史说明)。
- [ ] 提请审阅,通过后合并回 `main`。第二份 spec(非 mac 的 tab-overflow/
  agent picker 迁移)等这份实际跑起来、体验过共享机制好不好用之后再写。
