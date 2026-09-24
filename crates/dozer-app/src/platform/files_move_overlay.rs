//! 文件树"拖拽移动"确认弹窗的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`。
//! **不接失焦关闭**:"到目录"旁的浏览按钮(`MoveDirBrowse`)会同步弹出
//! 原生 `rfd` 目录选择器,那会让本窗口收到一次真实 `Focused(false)`,
//! 若照常触发失焦即关闭会把正在填的移动表单整个关掉——同
//! `ProjectCreateOverlay` 的既有考量。需要 IME(新文件名可能是中文)。

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
use crate::extensions::files;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{centered_overlay_bounds, open_child_window};

fn card_logical_size() -> LogicalSize<f32> {
    LogicalSize::new(460.0, 220.0)
}

/// 开关决策拆成纯函数,便于单测(同 `settings_overlay::sync_action`)。
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

pub(crate) struct FilesMoveOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    /// Drop 时把 macOS 原生右键粘贴菜单指回主窗口(见 `Drop` 实现)。
    main_window: Arc<Window>,
}

impl FilesMoveOverlay {
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
    ) -> FilesMoveOverlay {
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
        let window = open_child_window(main_window, pos, size, "files-move", el);
        window.set_ime_allowed(true);
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&window);
        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);
        FilesMoveOverlay {
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
        let Some(ws) = app.active_workspace() else {
            return;
        };
        let mut interface = UserInterface::build(
            files::files_move_card(&ws.files).map(Message::Files),
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
            return vec![Message::Files(files::Message::MoveCancel)];
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
        let Some(ws) = app.active_workspace() else {
            return Vec::new();
        };
        let mut interface = UserInterface::build(
            files::files_move_card(&ws.files).map(Message::Files),
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

impl Drop for FilesMoveOverlay {
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
