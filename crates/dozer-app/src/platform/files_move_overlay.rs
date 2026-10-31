//! 文件树"拖拽移动"确认弹窗的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`。
//! **不接失焦关闭**:"到目录"旁的浏览按钮(`MoveDirBrowse`)会同步弹出
//! 原生 `rfd` 目录选择器,那会让本窗口收到一次真实 `Focused(false)`,
//! 若照常触发失焦即关闭会把正在填的移动表单整个关掉——同
//! `ProjectCreateOverlay` 的既有考量。需要 IME(新文件名可能是中文)。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::core::mouse;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::Window;

use crate::app::{App, Message};
use crate::extensions::files;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_host::{OverlayHost, OverlayParts};
use crate::platform::overlay_window::{backdrop_card, open_overlay, popup_card_size};

pub(crate) use crate::platform::overlay_host::{SyncAction, sync_action};

fn card_logical_size(window: &Window) -> LogicalSize<f32> {
    // 280:表单(标题行/两组 label+输入框)+ 底部「取消/确定」按钮行都要放得
    // 下——此前 220 会把按钮行整个裁出卡片外(2026-09-27 用户实测)。
    popup_card_size(window, 280.0)
}

pub(crate) struct FilesMoveOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    /// Drop 时把 macOS 原生右键粘贴菜单指回主窗口(见 `Drop` 实现)。
    main_window: Arc<Window>,
}

impl OverlayHost for FilesMoveOverlay {
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

impl FilesMoveOverlay {
    /// `close_other_overlays` 摘掉这扇窗口前要用它发一条取消消息,道理同
    /// `DatabaseSourceOverlay::cancel_message`。
    pub(crate) fn cancel_message(&self) -> Message {
        Message::Files(files::Message::MoveCancel)
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        el: &ActiveEventLoop,
    ) -> FilesMoveOverlay {
        let (window, gpu) = open_overlay(
            main_window,
            adapter,
            device,
            queue,
            instance,
            "files-move",
            el,
        );
        window.set_ime_allowed(true);
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&window);
        FilesMoveOverlay {
            window,
            gpu,
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
            main_window: main_window.clone(),
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
                files::files_move_card(&ws.files, app.locale.active_locale()).map(Message::Files),
                card_logical_size(&self.window),
            ),
        );
    }

    pub(crate) fn handle_input(&mut self, app: &mut App, event: &WindowEvent) -> Vec<Message> {
        let card_size = card_logical_size(&self.window);
        self.overlay_input(
            event,
            Some(Message::Files(files::Message::MoveCancel)),
            || {
                let ws = app.active_workspace()?;
                Some(backdrop_card(
                    files::files_move_card(&ws.files, app.locale.active_locale())
                        .map(Message::Files),
                    card_size,
                ))
            },
        )
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
