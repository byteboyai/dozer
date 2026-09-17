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
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId, WindowLevel};

use crate::app::{App, Message};
use crate::extensions::search;

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

/// 焦点转移的纯判定:`(新的 focused 状态, 是否该因失焦关闭)`。
/// winit 在窗口刚创建时会无条件排一个合成 `Focused(false)`,必须忽略它——
/// 只有"先 `Focused(true)` 再 `Focused(false)`"才算真正的失焦。
fn focus_transition(was_focused: bool, now_focused: bool) -> (bool, bool) {
    if now_focused {
        (true, false)
    } else if was_focused {
        (false, true)
    } else {
        (false, false)
    }
}

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
    /// 是否真的获得过 OS 键盘焦点。winit 在窗口刚创建时会**无条件**排一个
    /// 合成的 `Focused(false)`(见 winit `window_delegate.rs` 里那句
    /// "Send Focused(false) right after creating the window delegate")——
    /// 若直接拿它当"点外部关闭"信号,窗口一打开就会被这个合成事件误关。
    /// 只有先收到过 `Focused(true)`、再收到 `Focused(false)` 才算真正的
    /// 失焦(用户点到别处/别的 App)。
    focused: bool,
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
        let card_logical = card_logical_size(window_width);
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
        let window = Arc::new(
            el.create_window(attrs)
                .expect("create search overlay window"),
        );
        window.focus_window();

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
            focused: false,
        }
    }

    /// 记录焦点事件,返回"是否该因失焦而关闭"。winit 在窗口创建时会先排
    /// 一个合成 `Focused(false)`,所以只有"先 `Focused(true)` 再
    /// `Focused(false)`"才算真正的失焦(用户点到别处),返回 `true`;
    /// 合成的那一发 `Focused(false)`(以及任何还没聚焦过的 `Focused(false)`)
    /// 返回 `false`,不触发关闭。
    pub(crate) fn handle_focus(&mut self, focused: bool) -> bool {
        let (new_focused, should_close) = focus_transition(self.focused, focused);
        self.focused = new_focused;
        should_close
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
        // 一次性聚焦位要在 `UserInterface` 建立之前取走:接口借住 `ws`
        // (视图树借 `&ws.search`),期间不能再 `&mut ws`。
        let focus_pending = ws.take_query_focus_pending();

        // `search_card` 返回 `Element<search::Message>`,`map(Message::Search)`
        // 升到 `app::Message` 后 `UserInterface` 才能接进
        // `crate::runtime::run_operate`(它硬编码 `UserInterface<app::Message>`)
        // 与 `handle_input` 的 `Vec<app::Message>` 收集。
        let mut interface = UserInterface::build(
            search::search_card(&ws.search, project_root.as_deref()).map(Message::Search),
            self.viewport.logical_size(),
            std::mem::take(&mut self.cache),
            &mut self.renderer,
        );

        if focus_pending {
            let mut op = iced_winit::core::widget::operation::focusable::focus::<()>(
                search::query_field_id(),
            );
            crate::runtime::run_operate(&mut interface, &mut self.renderer, &mut op);
        }
        crate::runtime::run_operate(
            &mut interface,
            &mut self.renderer,
            &mut search::CaptureQueryFocus,
        );
        let query_focused = search::take_query_focused();

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
        // `into_cache` 消费掉 `interface`,释放它对 `ws` 的借用,下面才能再
        // 可变借用 `ws` 写回焦点态。
        self.cache = interface.into_cache();
        ws.search.set_query_focused(query_focused);

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
            search::search_card(&ws.search, project_root.as_deref()).map(Message::Search),
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
        messages
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

    #[test]
    fn focus_transition_ignores_synthetic_false_and_closes_on_real_loss() {
        // 合成 Focused(false):窗口刚创建、还没真聚焦过——不关闭。
        assert_eq!(focus_transition(false, false), (false, false));
        // 真拿到焦点:不关闭。
        assert_eq!(focus_transition(false, true), (true, false));
        // 已聚焦过、现在失焦(用户点到别处):关闭。
        assert_eq!(focus_transition(true, false), (false, true));
        // 持续聚焦:不关闭。
        assert_eq!(focus_transition(true, true), (true, false));
    }
}
