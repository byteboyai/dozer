//! 设置弹窗(主题 + Git 账户连接)的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-18-git-account-settings-design.md`。
//! 结构对照 `ProjectCreateOverlay`:接入 IME 与原生右键菜单(PAT 输入框的
//! 粘贴、可能的 CJK 输入),**故意不接入 `FocusTracker`**——2026-09-27 用户
//! 要求"点击窗口以外的地方不需要关闭窗口",与"新建项目"弹窗一致,只认 Esc
//! 键 / 显式"关闭"按钮。既然不再有失焦即关闭,`State::suppress_next_blur`
//! 那套吞失焦机制也随之作废("没有 PAT?点此生成"拉起浏览器导致的失焦本
//! 就不会再关窗)。

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
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{
    backdrop_card, full_window_overlay_bounds, open_child_window,
};

/// 卡片逻辑尺寸——与"新建项目"弹窗(`project_create::card_logical_size`)
/// 完全一致:宽度 = 主窗口宽度 40%(下限 560),高度 = 主窗口高度 75%(下限
/// 520),保证两类弹窗窗口大小统一(2026-09-27 用户要求样式对齐创建项目弹窗)。
fn card_logical_size(window: &Window) -> LogicalSize<f32> {
    let logical: LogicalSize<f32> = window.inner_size().to_logical(window.scale_factor());
    let size = crate::extensions::project_create::card_logical_size(logical.width, logical.height);
    LogicalSize::new(size.width, size.height)
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

pub(crate) struct SettingsOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
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
        el: &ActiveEventLoop,
    ) -> SettingsOverlay {
        let scale = main_window.scale_factor();
        let (pos, size) = full_window_overlay_bounds(
            main_window
                .outer_position()
                .unwrap_or(PhysicalPosition::new(0, 0)),
            main_window.inner_size(),
        );
        let window = open_child_window(main_window, pos, size, "settings", el);
        window.set_ime_allowed(true);
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&window);

        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);

        SettingsOverlay {
            window,
            gpu,
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
            main_window: main_window.clone(),
        }
    }

    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
    ) {
        let (pos, size) = full_window_overlay_bounds(main_outer_pos, main_inner_size);
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
            backdrop_card(
                settings::settings_card(state).map(Message::Settings),
                card_logical_size(&self.window),
            ),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let (state, _) = interface.update(
            &[],
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut Vec::new(),
        );
        if let iced_winit::runtime::user_interface::State::Updated {
            mouse_interaction, ..
        } = state
        {
            crate::platform::overlay_gpu::apply_cursor(&self.window, mouse_interaction);
        }
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
            backdrop_card(
                settings::settings_card(state).map(Message::Settings),
                card_logical_size(&self.window),
            ),
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let mut messages = Vec::new();
        let (state, _) = interface.update(
            &events,
            self.cursor,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut messages,
        );
        if let iced_winit::runtime::user_interface::State::Updated {
            mouse_interaction, ..
        } = state
        {
            crate::platform::overlay_gpu::apply_cursor(&self.window, mouse_interaction);
        }
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
}
