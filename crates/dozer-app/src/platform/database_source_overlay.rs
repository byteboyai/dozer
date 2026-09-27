//! Database「新增/编辑数据源」表单的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`。
//! 简单失焦即关闭 + IME + 原生右键粘贴菜单——结构同 `settings_overlay.rs`,
//! 无需 `suppress_next_blur`(表单内无会拉起系统浏览器/原生选择器的按钮,
//! brainstorming 阶段已用 `grep -rn "rfd::" extensions/database/` 核实)。

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

fn card_logical_size(window: &Window) -> LogicalSize<f32> {
    popup_card_size(window, 480.0)
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

pub(crate) struct DatabaseSourceOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    /// Drop 时把 macOS 原生右键粘贴菜单指回主窗口(见 `Drop` 实现)。
    main_window: Arc<Window>,
}

impl DatabaseSourceOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// `close_other_overlays` 摘掉这扇窗口前要用它发一条取消消息——不然
    /// 只摘窗口不清业务状态,`ws.database.editing()` 还是 `Some`,下一帧
    /// 同款 `sync_database_source_overlay` 会发现触发条件仍成立又把窗口
    /// 建回来,跟被摘掉的另一个弹窗打架(同 Escape 分支发的消息)。
    pub(crate) fn cancel_message(&self) -> Message {
        Message::Database(database::Message::DraftCancel)
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        el: &ActiveEventLoop,
    ) -> DatabaseSourceOverlay {
        let (window, gpu) = open_overlay(
            main_window,
            adapter,
            device,
            queue,
            instance,
            "database-source",
            el,
        );
        window.set_ime_allowed(true);
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&window);
        DatabaseSourceOverlay {
            window,
            gpu,
            focus: FocusTracker::default(),
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

    /// `draft` 从 `ws.database.editing()` 取——`redraw`/`handle_input` 顶部
    /// 短路,`None` 表示表单已经在别处被关掉(过期一帧),直接不画。
    fn card<'a>(
        ws: &'a crate::workspace::Workspace,
        app_state: &'a database::AppState,
    ) -> Option<
        iced_widget::core::Element<
            'a,
            database::Message,
            iced_widget::Theme,
            iced_renderer::Renderer,
        >,
    > {
        let draft = ws.database.editing()?;
        Some(database::database_source_card(
            draft,
            app_state,
            ws.database.draft_test_status(),
        ))
    }

    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(ws) = app.active_workspace() else {
            return;
        };
        let Some(card) = Self::card(ws, &app.database) else {
            return;
        };
        self.gpu.redraw(
            &self.window,
            self.cursor,
            backdrop_card(card.map(Message::Database), card_logical_size(&self.window)),
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
            return vec![Message::Database(database::Message::DraftCancel)];
        }
        let Some(iced_event) =
            self.gpu
                .track_and_convert(&mut self.cursor, &mut self.modifiers, event)
        else {
            return Vec::new();
        };
        let Some(ws) = app.active_workspace() else {
            return Vec::new();
        };
        let Some(card) = Self::card(ws, &app.database) else {
            return Vec::new();
        };
        self.gpu.dispatch(
            &self.window,
            self.cursor,
            backdrop_card(card.map(Message::Database), card_logical_size(&self.window)),
            iced_event,
        )
    }
}

impl Drop for DatabaseSourceOverlay {
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
