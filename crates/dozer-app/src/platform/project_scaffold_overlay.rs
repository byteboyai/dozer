//! Project「修复项目」进度弹窗的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`。
//! `scrim_blocking` 语义原样保留:进行中不可通过 Esc/失焦/原生窗口关闭
//! 按钮关闭,只有全部步骤完成(`run.all_done()`)后内容里的"关闭"按钮
//! 才能真正关闭——`handle_input` 不判断 Esc,`window_event` 分流分支
//! 不处理 `WindowEvent::Focused`/`CloseRequested`,与 `ConfirmOverlay`/
//! 其余定制宿主的默认行为都不同,不要在整理代码时"顺手"补上。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::core::mouse;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::extensions::project;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{backdrop_card, open_overlay, reposition_overlay};

fn card_logical_size() -> LogicalSize<f32> {
    LogicalSize::new(480.0, 360.0)
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

pub(crate) struct ProjectScaffoldOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
}

impl ProjectScaffoldOverlay {
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
    ) -> ProjectScaffoldOverlay {
        let (window, gpu) = open_overlay(
            main_window,
            adapter,
            device,
            queue,
            instance,
            "project-scaffold",
            el,
        );
        ProjectScaffoldOverlay {
            window,
            gpu,
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

    // 故意不提供 `handle_focus`——本宿主不接失焦关闭,`window_event`
    // 分流分支直接不匹配 `WindowEvent::Focused`,交给下面 `handle_input`
    // 的普通事件路径(iced 内部会忽略它,不产生任何消息)。

    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(ws) = app.active_workspace() else {
            return;
        };
        self.gpu.redraw(
            &self.window,
            self.cursor,
            backdrop_card(
                project::view::project_scaffold_card(&ws.project_panel).map(Message::Project),
                card_logical_size(),
            ),
        );
    }

    /// 不判断 Esc——这是本宿主与其余全部定制宿主的关键差异,进行中唯一
    /// 能产生关闭消息的方式是点内容里的"关闭"按钮(`done` 时才可点)。
    pub(crate) fn handle_input(&mut self, app: &mut App, event: &WindowEvent) -> Vec<Message> {
        let Some(iced_event) =
            self.gpu
                .track_and_convert(&mut self.cursor, &mut self.modifiers, event)
        else {
            return Vec::new();
        };
        let Some(ws) = app.active_workspace() else {
            return Vec::new();
        };
        self.gpu.dispatch(
            &self.window,
            self.cursor,
            backdrop_card(
                project::view::project_scaffold_card(&ws.project_panel).map(Message::Project),
                card_logical_size(),
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
