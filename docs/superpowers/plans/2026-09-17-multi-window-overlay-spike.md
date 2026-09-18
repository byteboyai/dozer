# 第二原生窗口叠在 wry webview 之上 Spike 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在一次性 spike 里验证"给弹窗单独开一扇原生 `winit::Window`(而不是画在主窗口的 iced 内容里)能否天然盖过 wry webview、且不需要现在这套 `visible=false` 手动隐藏机制"这个假设是否成立。

**Architecture:** 复用 `spike/webview-child` 的最小骨架(一扇主窗口 + 一个覆盖右半区的 wry 子视图,模拟预览面板),在此基础上按键触发打开第二扇 `winit::Window`:用 `WindowAttributes::with_parent_window` + `with_window_level(WindowLevel::AlwaysOnTop)` 挂成主窗口的原生子窗口,复用同一个 `wgpu::Device`/`Queue` 开一份独立 `Surface`,先用裸 `wgpu` clear 验证"另开一扇窗口天然不受 wry 遮挡"这个核心假设,再换成真正的 `iced_wgpu::Engine`/`Renderer`/`UserInterface` 绘制一张有主题色的卡片,证明生产代码想要的渲染管线能在第二个窗口里复用同一份 GPU 资源跑起来。全程不触碰 `crates/dozer-app` 的真实代码,只在独立 spike crate 里验证。

**Tech Stack:** `winit` 0.30(`with_parent_window`/`WindowLevel::AlwaysOnTop`,macOS 走 `addChildWindow:ordered:`/`kCGFloatingWindowLevel`)、`wry` 0.55(子视图 webview,复刻现有 `webview-child` spike 的手法)、`iced_wgpu`/`iced_widget`/`iced_winit` 0.14(与 `crates/dozer-app` 完全同版本,结论才能直接套回生产代码)、`pollster` 做同步 `block_on`(生产代码用的是 `iced_winit::futures::futures::executor::block_on` 那套嵌套导出,spike 图简单换成 `pollster`,不影响结论,已在下面注明)。

**Spec:** 无独立 spec 文档——本计划由会话内研究(见 `crates/dozer-app/src/chrome/native_menu.rs` 的 NSMenu 方案、`app/layout.rs::webview_hidden_by_panel_popup` 与 `app/app.rs::preview_desired` 的现状"手动 visible=false 枚举"、以及对 `iced_winit-0.14.0`/`winit-0.30.13` 源码的直接阅读)驱动,验证结论本身就是交付物,产出一份发现记录供后续正式方案(是否要把 search modal / 确认弹窗迁到独立窗口)决策用。

## Global Constraints

- `spike/*` 是一次性技术验证,随时可删,不需要维持长期兼容或写单元测试——验收方式是人工 `cargo run` 后目视确认(同仓库 `native_menu.rs`/`webview-child` 的既有约定:原生窗口层级这类东西没有自动化测试手段)。
- Mac 先发,架构留门(CLAUDE.md 关键裁决):spike 里所有 macOS 专属 API(`with_parent_window`、`WindowLevel`)都是 `winit`/`wgpu` 的跨平台 Safe Rust 接口本身,不需要额外 `#[cfg(target_os = "macos")]` 网关也能编译(仅 mac 上语义生效,`WindowLevel` 文档明确 iOS/Android/Web/Wayland 不支持,不影响本仓库当前 mac-only 验证目标);不引入任何 Swift/AppKit 专属依赖。
- 弹窗/spike 渲染要用的主题色取自 CLAUDE.md:背景 `#0a0e16`、奶油文字 `#FFE5B4`——卡片内容用这两个颜色,不是因为 spike 要产品级好看,而是让"这真的是 Dozer 会用的弹窗"这件事看着更可信,方便人工判断。
- `iced_wgpu`/`iced_widget`/`iced_winit`/`winit`/`wry` 四个库的版本必须跟 `crates/dozer-app/Cargo.toml` 完全一致(`0.14`/`0.30`/`0.55.1`),这样 spike 得出的结论(能不能共享 device、渲染管线接不接得上)才对生产代码成立,版本不同结论不能直接套用。

**Branch:** `spike/multi-window-overlay-wry`——用 `superpowers:using-git-worktrees` 建独立 worktree 开发,验证完(无论证实还是证伪)都经审阅再决定合并到 main 还是就地留着当记录;因为只新增一个 `spike/*` crate、不改 `crates/dozer-app` 任何现有文件,合并冲突风险本身很低,但仍按仓库既有约定(`[[feedback-plans-use-worktree-branch]]`)走独立分支,避免跟其它并行开发的 plan 分支互相踩。

---

## File Structure

```
spike/multi-window-overlay/
├── Cargo.toml
└── src/
    └── main.rs        # 全部逻辑塞一个文件——spike 惯例(webview-child 同样只有一个 main.rs)
```

`Cargo.toml` 加入根 workspace 的 `members = ["crates/*", "spike/*"]` 通配符,不需要手动登记。

---

### Task 1: 脚手架——主窗口 + 右半区 wry 子视图(基线,不含 overlay)

**Files:**
- Create: `spike/multi-window-overlay/Cargo.toml`
- Create: `spike/multi-window-overlay/src/main.rs`

**Interfaces:**
- Consumes: 无(全新 crate)
- Produces: `App` struct(字段:`main_window: Option<std::sync::Arc<winit::window::Window>>`、`webview: Option<wry::WebView>`、`webview_visible: bool`、`overlay: Option<()>` 占位——Task 2 会把 `Option<()>` 换成真正的 `Overlay` 类型,这里先占住字段位置,`ApplicationHandler::window_event` 的 `match _id` 分发骨架同样先只认主窗口一个 id);`right_half(size, scale) -> wry::Rect` 复用 `webview-child` 的现成实现。

- [ ] **Step 1: 写 `Cargo.toml`**

```toml
[package]
name = "spike-multi-window-overlay"
version = "0.1.0"
edition.workspace = true
publish = false

[dependencies]
winit = "0.30.13"
wry = "0.55.1"
iced_wgpu = "0.14"
iced_widget = { version = "0.14", features = ["wgpu"] }
iced_winit = "0.14"
pollster = "0.3"
```

- [ ] **Step 2: 写基线 `main.rs`(主窗口 + 右半区 wry webview,复刻 `spike/webview-child`,额外加 `overlay` 占位字段和 `WindowId` 分发骨架)**

```rust
//! Spike：验证"给弹窗单独开一扇原生 winit::Window 能否天然盖过 wry webview"。
//! Task 1 基线:先复刻 webview-child 的主窗口 + 右半区 wry 子视图,不含 overlay。
//! 按 `h` 切换 webview 显隐(留作 Task 2/3 的对照——overlay 打开时不应该需要
//! 调用这个,才算验证成功)。

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::Key;
use winit::window::{Window, WindowId};
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{Rect, WebView, WebViewBuilder};

const PREVIEW_URL: &str = "https://byteboy.ai";

fn right_half(size: winit::dpi::PhysicalSize<u32>, scale: f64) -> Rect {
    let w = size.width as f64 / scale;
    let h = size.height as f64 / scale;
    Rect {
        position: LogicalPosition::new(w / 2.0, 0.0).into(),
        size: LogicalSize::new(w / 2.0, h).into(),
    }
}

#[derive(Default)]
struct App {
    main_window: Option<Arc<Window>>,
    webview: Option<WebView>,
    webview_visible: bool,
    overlay: Option<()>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        let window = Arc::new(
            el.create_window(
                Window::default_attributes()
                    .with_title("Spike: 第二原生窗口 vs wry webview")
                    .with_inner_size(LogicalSize::new(1200.0, 800.0)),
            )
            .expect("create main window"),
        );
        let bounds = right_half(window.inner_size(), window.scale_factor());
        let webview = WebViewBuilder::new()
            .with_url(PREVIEW_URL)
            .with_bounds(bounds)
            .build_as_child(window.as_ref())
            .expect("build child webview");
        self.main_window = Some(window);
        self.webview = Some(webview);
        self.webview_visible = true;
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(main_window) = &self.main_window else {
            return;
        };
        if id != main_window.id() {
            // Task 2 起,overlay 窗口的事件会落进这个分支单独处理。
            return;
        }
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                if let Some(wv) = &self.webview {
                    let _ = wv.set_bounds(right_half(size, main_window.scale_factor()));
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: Key::Character(ref c),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } if c.as_str() == "h" => {
                if let Some(wv) = &self.webview {
                    self.webview_visible = !self.webview_visible;
                    let _ = wv.set_visible(self.webview_visible);
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.run_app(&mut App::default()).expect("run app");
}
```

- [ ] **Step 3: 人工验证基线跑得通**

Run: `cargo run -p spike-multi-window-overlay`
预期:出现一扇窗口,右半区是 byteboy.ai 网页;按 `h` 网页显隐切换;resize 窗口网页跟着变宽/变窄。这一步只是确认脚手架没打错,还没有 overlay 可看。

- [ ] **Step 4: Commit**

```bash
git add spike/multi-window-overlay
git commit -m "spike: 第二原生窗口 vs wry webview——Task 1 基线脚手架"
```

---

### Task 2: 打开第二扇原生窗口,复用同一 wgpu device 渲染纯色——验证"天然不受 wry 遮挡"

**Files:**
- Modify: `spike/multi-window-overlay/src/main.rs`

**Interfaces:**
- Consumes: Task 1 的 `App`/`right_half`/`window_event` 分支骨架(`if id != main_window.id() { return; }` 那一行会被替换成真正判断 overlay id 的逻辑)。
- Produces: `struct Overlay { window: Arc<Window>, surface: wgpu::Surface<'static>, device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat }`;`fn overlay_bounds(main_window: &Window) -> (winit::dpi::PhysicalPosition<i32>, winit::dpi::PhysicalSize<u32>)`;`fn open_overlay(main_window: &Arc<Window>, el: &ActiveEventLoop) -> Overlay`。Task 3 会往 `Overlay` 里加 `renderer`/`cache` 字段,复用这里的 `window`/`surface`/`device`/`queue`/`format`。

- [ ] **Step 1: 加 `Overlay` 结构体和窗口定位函数**

```rust
use iced_wgpu::wgpu;
use winit::raw_window_handle::HasWindowHandle;
use winit::window::WindowLevel;

struct Overlay {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
}

/// overlay 卡片刻意定位到主窗口右半区(webview 所在区域)内部——如果
/// overlay 依然完整可见、没被 webview 吃掉,才算验证了"另开窗口天然不受
/// 同窗口子视图层级摆布"这个假设;放在左半区(没有 webview)看不出区别。
fn overlay_bounds(
    main_window: &Window,
) -> (winit::dpi::PhysicalPosition<i32>, winit::dpi::PhysicalSize<u32>) {
    let scale = main_window.scale_factor();
    let outer_pos = main_window
        .outer_position()
        .unwrap_or(winit::dpi::PhysicalPosition::new(0, 0));
    let inner = main_window.inner_size();
    let card_w = 420.0 * scale;
    let card_h = 260.0 * scale;
    // 右半区中心:x0 = 主窗口宽度的 3/4 附近,y 居中。
    let x = outer_pos.x + (inner.width as f64 * 0.75 - card_w / 2.0) as i32;
    let y = outer_pos.y + (inner.height as f64 / 2.0 - card_h / 2.0) as i32;
    (
        winit::dpi::PhysicalPosition::new(x, y),
        winit::dpi::PhysicalSize::new(card_w as u32, card_h as u32),
    )
}
```

- [ ] **Step 2: 写 `open_overlay`——挂成主窗口的原生子窗口 + AlwaysOnTop + 独立 surface,复用同一份 device/queue**

```rust
fn open_overlay(main_window: &Arc<Window>, el: &ActiveEventLoop) -> Overlay {
    let (pos, size) = overlay_bounds(main_window);
    let parent_handle = main_window
        .window_handle()
        .expect("main window handle")
        .as_raw();

    let attrs = Window::default_attributes()
        .with_title("overlay")
        .with_decorations(false)
        .with_transparent(true)
        .with_window_level(WindowLevel::AlwaysOnTop)
        .with_position(pos)
        .with_inner_size(size);
    // Safety: `parent_handle` 来自仍存活的 `main_window`(`Arc` 持有),
    // 本函数返回前 `main_window` 不会被 drop。
    let attrs = unsafe { attrs.with_parent_window(Some(parent_handle)) };
    let window = Arc::new(el.create_window(attrs).expect("create overlay window"));

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or_default(),
        ..Default::default()
    });
    let surface = instance
        .create_surface(window.clone())
        .expect("create overlay surface");

    let (format, device, queue) = pollster::block_on(async {
        let adapter =
            wgpu::util::initialize_adapter_from_env_or_default(&instance, Some(&surface))
                .await
                .expect("create overlay adapter");
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities.formats[0];
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("overlay device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            })
            .await
            .expect("request overlay device");
        (format, device, queue)
    });

    let physical = window.inner_size();
    surface.configure(
        &device,
        &wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: physical.width.max(1),
            height: physical.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        },
    );

    Overlay {
        window,
        surface,
        device,
        queue,
        format,
    }
}

fn draw_overlay_clear(overlay: &Overlay) {
    let frame = overlay
        .surface
        .get_current_texture()
        .expect("overlay surface texture");
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = overlay
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    // ByteBoy2077 背景色 #0a0e16。
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.039,
                        g: 0.055,
                        b: 0.086,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
    overlay.queue.submit([encoder.finish()]);
    frame.present();
}
```

- [ ] **Step 3: 接键盘触发(`o` 开/关 overlay)+ 把 overlay 的 `WindowEvent` 单独分发**

修改 `App` 加 `overlay: Option<Overlay>`(替掉 Task 1 占位的 `Option<()>`),`window_event` 改成:

```rust
fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
    let Some(main_window) = &self.main_window else {
        return;
    };

    if let Some(overlay) = &self.overlay {
        if id == overlay.window.id() {
            match event {
                WindowEvent::CloseRequested => {
                    self.overlay = None;
                }
                WindowEvent::RedrawRequested => {
                    draw_overlay_clear(self.overlay.as_ref().unwrap());
                }
                _ => {}
            }
            return;
        }
    }

    if id != main_window.id() {
        return;
    }
    match event {
        WindowEvent::CloseRequested => el.exit(),
        WindowEvent::Resized(size) => {
            if let Some(wv) = &self.webview {
                let _ = wv.set_bounds(right_half(size, main_window.scale_factor()));
            }
        }
        WindowEvent::KeyboardInput {
            event:
                KeyEvent {
                    logical_key: Key::Character(ref c),
                    state: ElementState::Pressed,
                    ..
                },
            ..
        } => match c.as_str() {
            "h" => {
                if let Some(wv) = &self.webview {
                    self.webview_visible = !self.webview_visible;
                    let _ = wv.set_visible(self.webview_visible);
                }
            }
            "o" => {
                if self.overlay.take().is_none() {
                    self.overlay = Some(open_overlay(main_window, el));
                }
            }
            _ => {}
        },
        _ => {}
    }
}
```

`resumed`/`main` 不用改;`App` derive 需要去掉(`Overlay` 没实现 `Default`),改成手动 `impl Default for App`(`overlay: None` 等字段手写)或在 `main` 里显式构造 `App { main_window: None, webview: None, webview_visible: false, overlay: None }`——用后者,删掉 `#[derive(Default)]`。

- [ ] **Step 4: 人工验证核心假设**

Run: `cargo run -p spike-multi-window-overlay`

验收(**不要**在按 `o` 时顺手调 `wv.set_visible(false)`——这正是要证明可以不需要的东西):
1. 按 `o`:右半区 webview 上方浮出一块深色卡片,webview 本身完全没被隐藏(仍在渲染网页),卡片完整盖住卡片范围内的网页内容,没有网页内容"透出来"或卡片被网页盖住的情况。
2. 再按 `o`:卡片消失,网页恢复完整可见,无残留半透明痕迹/崩溃。
3. 拖动主窗口:观察卡片是否跟着一起移动(验证 `with_parent_window` 的 `addChildWindow` 语义是否在这个 winit 版本上生效)——记下实际现象(跟走 / 不跟走),不强制要求跟走,Task 4 的发现记录里如实写。

- [ ] **Step 5: Commit**

```bash
git add spike/multi-window-overlay
git commit -m "spike: overlay 独立窗口 + 共享 wgpu device 纯色渲染,验证不受 wry 遮挡"
```

---

### Task 3: 把纯色 clear 换成真正的 iced_wgpu 渲染管线

**Files:**
- Modify: `spike/multi-window-overlay/src/main.rs`

**Interfaces:**
- Consumes: Task 2 的 `Overlay { window, surface, device, queue, format }`、`open_overlay`、`overlay_bounds`。
- Produces: `Overlay` 新增 `renderer: iced_wgpu::Renderer`、`cache: iced_winit::runtime::user_interface::Cache` 两个字段;`fn overlay_view() -> iced_widget::Element<'static, (), iced_winit::core::Theme, iced_wgpu::Renderer>`;`fn draw_overlay_iced(overlay: &mut Overlay)` 替换 Task 2 的 `draw_overlay_clear`(生产代码要接的正是这一步:`Engine`/`Renderer`/`UserInterface` 能否在第二个窗口的 surface 上,用跟主窗口共享的 `device`/`queue` 正常跑完一轮 `build → update → draw → present`)。

- [ ] **Step 1: 加 iced 相关 import,给 `Overlay` 加 `renderer`/`cache` 字段**

```rust
use iced_wgpu::graphics::Viewport;
use iced_wgpu::{Engine, Renderer};
use iced_widget::{center, column, container, text};
use iced_winit::core::{Color, Font, Pixels, Size, Theme};
use iced_winit::runtime::user_interface::{self, UserInterface};
```

```rust
struct Overlay {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    renderer: Renderer,
    cache: user_interface::Cache,
}
```

- [ ] **Step 2: `open_overlay` 里用同一份 `device.clone()`/`queue.clone()` 建 `Engine`/`Renderer`,和拿到的 `adapter` 一起初始化**

在 Step 1 的 `pollster::block_on` 块里把 `adapter` 一并返回(而不是只解构出 `format, device, queue`),再在 `open_overlay` 末尾补:

```rust
    let engine = Engine::new(
        &adapter,
        device.clone(),
        queue.clone(),
        format,
        None,
        iced_wgpu::graphics::Shell::headless(),
    );
    let renderer = Renderer::new(engine, Font::default(), Pixels::from(16));

    Overlay {
        window,
        surface,
        device,
        queue,
        format,
        renderer,
        cache: user_interface::Cache::new(),
    }
```

(把 Step 1 里 `pollster::block_on(async { .. (format, device, queue) })` 的返回元组改成 `(format, adapter, device, queue)`,对应解构处 `let (format, adapter, device, queue) = ...`。)

- [ ] **Step 3: 写 overlay 的 iced 视图——ByteBoy2077 主题卡片**

```rust
fn overlay_view() -> iced_widget::Element<'static, (), Theme, Renderer> {
    let bg = Color::from_rgb8(0x0a, 0x0e, 0x16);
    let cream = Color::from_rgb8(0xFF, 0xE5, 0xB4);
    center(
        container(
            column![
                text("第二原生窗口").size(20).color(cream),
                text("这块卡片是独立的 winit::Window,\n不是画在主窗口里的 iced 内容。")
                    .size(14)
                    .color(cream),
            ]
            .spacing(8),
        )
        .padding(20)
        .style(move |_theme: &Theme| container::Style {
            background: Some(bg.into()),
            ..Default::default()
        }),
    )
    .into()
}
```

- [ ] **Step 4: 写 `draw_overlay_iced`,替换调用点里的 `draw_overlay_clear`**

```rust
fn draw_overlay_iced(overlay: &mut Overlay) {
    let physical = overlay.window.inner_size();
    let viewport = Viewport::with_physical_size(
        Size::new(physical.width, physical.height),
        overlay.window.scale_factor() as f32,
    );

    let frame = overlay
        .surface
        .get_current_texture()
        .expect("overlay surface texture");
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = overlay
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
    overlay.queue.submit([encoder.finish()]);

    let mut interface = UserInterface::build(
        overlay_view(),
        viewport.logical_size(),
        std::mem::take(&mut overlay.cache),
        &mut overlay.renderer,
    );
    let _ = interface.update(
        &[],
        iced_winit::core::mouse::Cursor::Unavailable,
        &mut overlay.renderer,
        &mut iced_winit::Clipboard::unconnected(),
        &mut Vec::new(),
    );
    interface.draw(
        &mut overlay.renderer,
        &Theme::Dark,
        &iced_winit::core::renderer::Style::default(),
        iced_winit::core::mouse::Cursor::Unavailable,
    );
    overlay.cache = interface.into_cache();

    overlay
        .renderer
        .present(None, frame.texture.format(), &view, &viewport);
    frame.present();
}
```

把 `window_event` 里 overlay 分支的 `WindowEvent::RedrawRequested => draw_overlay_clear(...)` 改成:

```rust
WindowEvent::RedrawRequested => {
    if let Some(overlay) = &mut self.overlay {
        draw_overlay_iced(overlay);
    }
}
```

(用 `if let Some(overlay) = &mut self.overlay` 直接拿可变借用,不再需要 Task 2 那个 `self.overlay.as_ref().unwrap()` 写法。)

- [ ] **Step 5: 人工验证**

Run: `cargo run -p spike-multi-window-overlay`
验收:按 `o` 后卡片里出现深底奶油色文字"第二原生窗口"+ 说明文案,渲染清晰、无花屏/撕裂;文字内容仍完整盖在 webview 之上;反复按 `o` 开关几次不崩溃;resize 主窗口时 overlay 内容不必跟着变(overlay 是独立尺寸的浮窗,这点不是本 spike 要验证的目标)。

- [ ] **Step 6: Commit**

```bash
git add spike/multi-window-overlay
git commit -m "spike: overlay 换成真正的 iced_wgpu 渲染管线,共享主窗口 device/queue"
```

---

### Task 4: 记录发现——喂给后续正式方案决策

**Files:**
- Create: `docs/superpowers/specs/2026-09-17-multi-window-overlay-spike-findings.md`

**Interfaces:**
- Consumes: Task 1-3 人工验证时观察到的全部现象(尤其 Task 2 Step 4 第 3 点"主窗口拖动时 overlay 跟不跟走"的实测结果,以及本 Task 里额外做的两项负面测试)。
- Produces: 一份供未来"是否把 search modal / 确认弹窗迁到独立窗口"决策阅读的发现记录,不产出代码。

- [ ] **Step 1: 额外跑两项负面/边界测试,记录现象(不需要写代码,现有二进制直接够用)**

1. **失焦测试**:overlay 打开时点击别的 App(切到 Finder 或别的窗口)再切回来,观察 overlay 是否还在原位、层级是否还压在 webview 之上,还是被切走后掉到了后面。
2. **多次开关内存/句柄测试**:连续按 `o` 开关 20 次左右,用 Activity Monitor 粗看一下 GPU/内存有没有明显只增不减(`wgpu::Device`/`Surface` 每次 `open_overlay` 都重新创建,`Overlay` drop 时应该跟着释放——如果观察到持续增长,说明有资源没释放干净,这是要在正式方案里解决的问题,不是本 spike 直接要修的)。

- [ ] **Step 2: 写发现记录**

```markdown
# 第二原生窗口 vs wry webview Spike 发现记录

日期:2026-09-17
对应 spike:`spike/multi-window-overlay`(分支 `spike/multi-window-overlay-wry`)
起因:menu 已经靠迁到原生 NSMenu 解决了 wry 遮挡问题;调研能否把同一思路
(挪到独立原生窗口层级)用到 search modal/确认弹窗这类自定义主题的弹窗上,
绕开现在 `app/app.rs::preview_desired`/`webview_hidden_by_panel_popup` 里
"每加一个弹窗就要手动补一条 visible=false 判断"的枚举模式。

## 核心假设是否成立

[按实测结果填:成立 / 部分成立 / 不成立 —— 引用 Task 2 Step 4 的三项验收
现象]

## 关键现象

- 天然层级:[overlay 是否在不调用任何 `set_visible` 的情况下完整盖过
  wry webview]
- 拖动跟随:[`with_parent_window` 的 addChildWindow 语义,在 winit
  0.30.13 上是否让 overlay 跟着主窗口一起移动]
- 失焦行为:[Task 4 Step 1 第 1 项的观察]
- 资源释放:[Task 4 Step 1 第 2 项的观察]
- 渲染管线复用:[Task 3——共享 device/queue 的第二个 Engine/Renderer
  是否能正常跑完 build/update/draw/present,有没有校验错误/panic]

## 相对现状方案的成本

- 现状(`visible=false` 手动枚举):每加一个新弹窗要记得去
  `app/app.rs::preview_desired`/`browser_desired`/
  `app/layout.rs::webview_hidden_by_panel_popup` 补判断,漏加会导致
  webview 盖住弹窗(过去已出现过这类 bug)。
- 独立窗口方案:每个要用这条路的弹窗要单独维护一扇 `winit::Window` +
  一份 `Engine`/`Renderer`/`UserInterface`(不能复用主窗口现成的那一份,
  见 iced_winit 源码 `window.rs:68` `let renderer =
  compositor.create_renderer()`——多窗口场景下官方实现也是每扇窗口各自一个
  `Renderer`,不是共享同一个),生命周期管理(开关/resize/失焦/跟随主窗口
  移动)全部要自己手搓,量级上接近再抄一遍
  `platform/window_events.rs` 里主窗口那一整套 `Runner::Ready` 状态机的
  精简版。

## 建议

[给后续要不要真正推进"弹窗迁独立窗口"这条路线一个建议,以及如果推进,
第一个该落地的候选弹窗是谁(建议从 `search_modal` 这个最常用、最有代表性
的开始,而不是一次性把 file_history/text_input_menu 都迁)]
```

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-09-17-multi-window-overlay-spike-findings.md
git commit -m "docs: 记录第二原生窗口 vs wry webview spike 的发现"
```

---

## Self-Review 摘要

- **Spec 覆盖**:本计划没有独立 spec,四个 Task 对应研究会话里提出的完整验证链——基线(Task 1)→ 核心假设裸 wgpu 验证(Task 2)→ 生产管线复用验证(Task 3)→ 发现留档(Task 4),没有遗漏讨论过的点(拖动跟随、资源释放、失焦都在 Task 2/4 里覆盖到了)。
- **占位符检查**:已通读,Task 1-3 每一步都是可直接粘贴运行的完整代码,Task 4 的 markdown 模板里 `[按实测结果填 ...]` 不是"TODO 占位符",是要求实施者据实测填写的记录字段,本身就是该 Step 的交付物一部分。
- **类型一致性**:`Overlay` 结构体字段(`window`/`surface`/`device`/`queue`/`format`)在 Task 2 定义、Task 3 追加 `renderer`/`cache`,后续代码块引用字段名前后一致;`open_overlay`/`overlay_bounds`/`draw_overlay_clear`→`draw_overlay_iced` 的函数签名在跨 Task 引用处保持一致(Task 3 明确说明要把 Task 2 的 `draw_overlay_clear` 调用点替换掉,不是并存)。
