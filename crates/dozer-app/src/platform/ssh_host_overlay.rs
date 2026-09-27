//! SSH「添加/编辑主机」表单的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`。
//! 简单失焦即关闭 + IME + 原生右键粘贴菜单——结构同 `settings_overlay.rs`,
//! 无需 `suppress_next_blur`(表单内无会拉起系统浏览器/原生选择器的按钮)。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::core::mouse;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::extensions::ssh;
use crate::platform::overlay_focus::FocusTracker;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{backdrop_card, open_overlay, reposition_overlay};

fn card_logical_size() -> LogicalSize<f32> {
    LogicalSize::new(480.0, 480.0)
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

pub(crate) struct SshHostOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    /// Drop 时把 macOS 原生右键粘贴菜单指回主窗口(见 `Drop` 实现)。
    main_window: Arc<Window>,
}

impl SshHostOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// `close_other_overlays` 摘掉这扇窗口前要用它发一条取消消息,道理同
    /// `DatabaseSourceOverlay::cancel_message`。
    pub(crate) fn cancel_message(&self) -> Message {
        Message::Ssh(ssh::Message::DraftCancel)
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        el: &ActiveEventLoop,
    ) -> SshHostOverlay {
        let (window, gpu) = open_overlay(
            main_window,
            adapter,
            device,
            queue,
            instance,
            "ssh-host",
            el,
        );
        window.set_ime_allowed(true);
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&window);
        SshHostOverlay {
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

    /// `draft` 从 `ws.ssh.editing()` 取——`redraw`/`handle_input` 顶部短路,
    /// `None` 表示表单已经在别处被关掉(过期一帧),直接不画。
    fn card<'a>(
        ws: &'a crate::workspace::Workspace,
    ) -> Option<
        iced_widget::core::Element<'a, ssh::Message, iced_widget::Theme, iced_renderer::Renderer>,
    > {
        let draft = ws.ssh.editing()?;
        let status = draft
            .id
            .as_deref()
            .map(|id| ws.ssh.test_status(id))
            .unwrap_or(&ssh::TestStatus::Idle);
        Some(ssh::ssh_host_card(draft, status))
    }

    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(ws) = app.active_workspace() else {
            return;
        };
        let Some(card) = Self::card(ws) else {
            return;
        };
        self.gpu.redraw(
            &self.window,
            self.cursor,
            backdrop_card(card.map(Message::Ssh), card_logical_size()),
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
            return vec![Message::Ssh(ssh::Message::DraftCancel)];
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
        let Some(card) = Self::card(ws) else {
            return Vec::new();
        };
        self.gpu.dispatch(
            &self.window,
            self.cursor,
            backdrop_card(card.map(Message::Ssh), card_logical_size()),
            iced_event,
        )
    }
}

impl Drop for SshHostOverlay {
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
