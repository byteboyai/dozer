//! Project「删除项目」三选一确认弹窗的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`。
//! 纯单选(`delete::DeleteScope`)+ 取消/确认按钮,无文本输入,不需要
//! IME/原生右键菜单——结构同 `settings_overlay.rs` 去掉 IME 那两行。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::core::mouse;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::Window;

use crate::app::{App, Message};
use crate::extensions::project;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_host::{FocusClosable, OverlayHost, OverlayParts};
use crate::platform::overlay_window::{backdrop_card, open_overlay, popup_card_size};

pub(crate) use crate::platform::overlay_host::{SyncAction, sync_action};
use bytehost_desktop::FocusTracker;

fn card_logical_size(window: &Window) -> LogicalSize<f32> {
    popup_card_size(window, 260.0)
}

pub(crate) struct ProjectDeleteOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
}

impl OverlayHost for ProjectDeleteOverlay {
    fn overlay_window(&self) -> &Window {
        &self.window
    }

    fn overlay_parts(&mut self) -> OverlayParts<'_> {
        OverlayParts {
            window: &self.window,
            gpu: &mut self.gpu,
            cursor: &mut self.cursor,
            modifiers: &mut self.modifiers,
        }
    }
}

impl FocusClosable for ProjectDeleteOverlay {
    fn focus_tracker(&mut self) -> &mut FocusTracker {
        &mut self.focus
    }
}

impl ProjectDeleteOverlay {
    /// `close_other_overlays` 摘掉这扇窗口前要用它发一条取消消息,道理同
    /// `DatabaseSourceOverlay::cancel_message`。
    pub(crate) fn cancel_message(&self) -> Message {
        Message::Project(project::Message::DeleteProjectCancel)
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        el: &ActiveEventLoop,
    ) -> ProjectDeleteOverlay {
        let (window, gpu) = open_overlay(
            main_window,
            adapter,
            device,
            queue,
            instance,
            "project-delete",
            el,
        );
        ProjectDeleteOverlay {
            window,
            gpu,
            focus: FocusTracker::default(),
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
        }
    }

    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(ws) = app.active_workspace() else {
            return;
        };
        self.gpu.redraw(
            &self.window,
            self.cursor,
            backdrop_card(
                project::view::project_delete_card(
                    &ws.project_panel,
                    app.locale.active_locale(),
                    crate::host_adapters::project_panel::DOZER_CARD,
                )
                .map(Message::Project),
                card_logical_size(&self.window),
            ),
        );
    }

    pub(crate) fn handle_input(&mut self, app: &mut App, event: &WindowEvent) -> Vec<Message> {
        let card_size = card_logical_size(&self.window);
        self.overlay_input(
            event,
            Some(Message::Project(project::Message::DeleteProjectCancel)),
            || {
                let ws = app.active_workspace()?;
                Some(backdrop_card(
                    project::view::project_delete_card(
                        &ws.project_panel,
                        app.locale.active_locale(),
                        crate::host_adapters::project_panel::DOZER_CARD,
                    )
                    .map(Message::Project),
                    card_size,
                ))
            },
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
