//! 设置弹窗(主题 + Git 账户连接)的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-18-git-account-settings-design.md`。
//! 结构上是 `FileHistoryOverlay`(`FocusTracker` 失焦即关闭)与
//! `ProjectCreateOverlay`(IME + 原生右键菜单挂靠,PAT 输入框需要)两者的
//! 混合:接入失焦关闭,但"没有 PAT?点此生成"会拉起系统浏览器,那**确实**
//! 会让本窗口收到一次真实失焦(这一点上一版文档说错了,代码评审已指出:
//! PAT-link click can auto-close Settings)——`handle_focus` 需要配合
//! `extensions::settings::State::suppress_next_blur` 吞掉那一次,不能只
//! 靠 `FocusTracker` 自己判断。仍需要 IME(账户用户名可能是中文相关字符)
//! 和原生右键菜单(PAT 输入框的粘贴)。

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
use crate::extensions::settings;
use crate::platform::overlay_focus::FocusTracker;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{centered_overlay_bounds, open_child_window};

/// 卡片逻辑尺寸——固定值,不随主窗口宽高缩放:设置表单内容量有限,不需要
/// 像 file_history/project_create 那样按主窗口比例伸缩。
fn card_logical_size() -> LogicalSize<f32> {
    LogicalSize::new(480.0, 560.0)
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

/// `handle_focus` 的吞掉判定拆成纯函数,不需要真建 `SettingsOverlay`/
/// `FocusTracker` 就能单测。`Some(should_close)` 表示这次事件已经判完,
/// 不用再交给 `FocusTracker`;`None` 表示按正常失焦逻辑走。只在"这次是
/// 失焦、且标志位确实置着"时消费标志位并返回 `Some(false)`——聚焦事件或
/// 标志位未置都不消费,交还给 `FocusTracker` 正常判定。
fn suppressed_close(focused: bool, suppress_next_blur: &mut bool) -> Option<bool> {
    if !focused && std::mem::take(suppress_next_blur) {
        Some(false)
    } else {
        None
    }
}

pub(crate) struct SettingsOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    main_window: Arc<Window>,
}

impl SettingsOverlay {
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
        _main_window_size: LogicalSize<f32>,
        el: &ActiveEventLoop,
    ) -> SettingsOverlay {
        let scale = main_window.scale_factor();
        let card_logical = card_logical_size();
        let (pos, size) = centered_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
            scale,
            card_logical,
        );
        let window = open_child_window(main_window, pos, size, "settings", el);
        window.set_ime_allowed(true);
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&window);

        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);

        SettingsOverlay {
            window,
            gpu,
            focus: FocusTracker::default(),
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
            main_window: main_window.clone(),
        }
    }

    /// `suppress_next_blur` 来自 `extensions::settings::State`——"没有
    /// PAT?点此生成"拉起系统浏览器时置位,这里读到就吞掉这一次失焦、不
    /// touch `FocusTracker` 内部状态(浏览器打开后窗口重新聚焦会收到真实
    /// `Focused(true)`,届时状态自然纠正)。
    pub(crate) fn handle_focus(&mut self, focused: bool, suppress_next_blur: &mut bool) -> bool {
        if let Some(should_close) = suppressed_close(focused, suppress_next_blur) {
            return should_close;
        }
        self.focus.handle_focus(focused)
    }

    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
        _window_width: f32,
        _window_height: f32,
    ) {
        let card_logical = card_logical_size();
        let (pos, size) =
            centered_overlay_bounds(main_outer_pos, main_inner_size, scale, card_logical);
        self.window.set_outer_position(pos);
        if self.window.inner_size() != size {
            let _ = self.window.request_inner_size(size);
            self.gpu.reconfigure(device, size, scale);
        }
    }

    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(state) = app.settings.as_ref() else {
            return;
        };
        let mut interface = UserInterface::build(
            settings::settings_card(state).map(Message::Settings),
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
            return vec![Message::Settings(settings::Message::Close)];
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
        let Some(state) = app.settings.as_ref() else {
            return Vec::new();
        };
        let mut interface = UserInterface::build(
            settings::settings_card(state).map(Message::Settings),
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

impl Drop for SettingsOverlay {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&self.main_window);
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

    #[test]
    fn suppressed_close_consumes_flag_and_swallows_the_blur() {
        let mut suppress = true;
        assert_eq!(suppressed_close(false, &mut suppress), Some(false));
        assert!(!suppress, "标志位应该被消费掉,不能留着吞掉下一次真失焦");
    }

    #[test]
    fn suppressed_close_defers_to_focus_tracker_when_flag_not_set() {
        let mut suppress = false;
        assert_eq!(suppressed_close(false, &mut suppress), None);
    }

    #[test]
    fn suppressed_close_does_not_consume_flag_on_focus_gain() {
        let mut suppress = true;
        assert_eq!(suppressed_close(true, &mut suppress), None);
        assert!(suppress, "聚焦事件不是要吞的那一次失焦,标志位应该留着");
    }
}
