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

    /// `main_window_size` 用具名字段的 `LogicalSize`(`.width`/`.height`)
    /// 承载,而不是两个相邻的 `f32` 位置参数——8 个位置参数会触发
    /// `clippy::too_many_arguments`,且 width/height 相邻同型传反编译器不
    /// 报错,按 CLAUDE.md 关键裁决改具名字段(同 `workspace/state.rs` 的
    /// `TabAttachedArgs` 先例),不加 `#[allow]`。
    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        main_window_size: LogicalSize<f32>,
        el: &ActiveEventLoop,
    ) -> FileHistoryOverlay {
        let scale = main_window.scale_factor();
        let card_logical = card_logical_size(main_window_size.width, main_window_size.height);
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
