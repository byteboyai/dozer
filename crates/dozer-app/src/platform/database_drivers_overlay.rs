//! 数据库「管理驱动」弹窗的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`
//! 「架构」第 2 节。结构与 `settings_overlay.rs` 同构,无 IME/原生右键
//! 菜单需求(纯列表展示,无文本输入)。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::core::mouse;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::extensions::database;
use crate::platform::overlay_focus::FocusTracker;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{
    backdrop_card, open_overlay, popup_card_size, reposition_overlay,
};

/// 卡片逻辑尺寸——固定值,不随主窗口宽高缩放(设置/驱动列表内容量有限,
/// 取舍同 `settings_overlay`/`search_overlay`)。
fn card_logical_size(window: &Window) -> LogicalSize<f32> {
    popup_card_size(window, 420.0)
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

pub(crate) struct DatabaseDriversOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
}

impl DatabaseDriversOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// `close_other_overlays` 摘掉这扇窗口前要用它发一条关闭消息,道理同
    /// `DatabaseSourceOverlay::cancel_message`——这个弹窗是布尔态而非草稿
    /// 表单,"取消"就是把 `drivers_popup_open` 拨回 `false`。
    pub(crate) fn cancel_message(&self) -> Message {
        Message::Database(database::Message::DriversPopupToggle)
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        el: &ActiveEventLoop,
    ) -> DatabaseDriversOverlay {
        let (window, gpu) = open_overlay(
            main_window,
            adapter,
            device,
            queue,
            instance,
            "database-drivers",
            el,
        );
        DatabaseDriversOverlay {
            window,
            gpu,
            focus: FocusTracker::default(),
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
        }
    }

    pub(crate) fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
    ) {
        reposition_overlay(
            &self.window,
            &mut self.gpu,
            device,
            main_outer_pos,
            main_inner_size,
            scale,
        );
    }

    pub(crate) fn handle_focus(&mut self, focused: bool) -> bool {
        self.focus.handle_focus(focused)
    }

    pub(crate) fn redraw(&mut self, app: &mut App) {
        self.gpu.redraw(
            &self.window,
            self.cursor,
            backdrop_card(
                database::database_drivers_card(&app.database).map(Message::Database),
                card_logical_size(&self.window),
            ),
        );
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
            return vec![Message::Database(database::Message::DriversPopupToggle)];
        }
        let Some(iced_event) =
            self.gpu
                .track_and_convert(&mut self.cursor, &mut self.modifiers, event)
        else {
            return Vec::new();
        };
        self.gpu.dispatch(
            &self.window,
            self.cursor,
            backdrop_card(
                database::database_drivers_card(&app.database).map(Message::Database),
                card_logical_size(&self.window),
            ),
            iced_event,
        )
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
