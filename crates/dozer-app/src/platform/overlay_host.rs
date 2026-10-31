//! 模态弹窗宿主的**共用样板**(Tier2 Task3,试点)。
//!
//! 每个 overlay 宿主(`confirm`/`files_move`/`project_delete` …)都有一份完全相同的
//! 窗口/渲染管线字段与四段方法:
//!
//! - `window_id()` / `request_redraw()`:直接转发给这扇窗口;
//! - `reposition()`:转调 [`reposition_overlay`](super::overlay_window::reposition_overlay);
//! - `handle_input()`:先处理 Esc(各宿主的取消消息不同),再把 winit 事件经
//!   `OverlayGpu::track_and_convert` 转成 iced 事件、拼卡片、`dispatch`。
//!
//! 本模块用 trait 默认方法收掉这些;各宿主只需给出 `overlay_window()` 与
//! `overlay_parts()`(把四个字段拆成互不重叠的借用)。卡片内容仍由各宿主自己拼
//! ——内容不同,不属于样板。
//!
//! `SyncAction`/`sync_action` 也在多个宿主里逐字重复,一并收在这里;原模块用
//! `pub(crate) use` 保持既有路径。

use iced_wgpu::wgpu;
use iced_winit::core::mouse;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::Message;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::reposition_overlay;
use bytehost_desktop::FocusTracker;

/// 弹窗卡片的元素类型(与 `OverlayGpu::redraw`/`dispatch` 的入参一致)。
pub(crate) type OverlayElement<'a> =
    iced_winit::core::Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>;

/// 宿主字段的互不重叠借用。`gpu`/`cursor`/`modifiers` 要同时可变借出(转换事件
/// 时写 cursor 与 modifiers,派发时用 gpu),所以不能拆成多个 `&mut self` 方法。
pub(crate) struct OverlayParts<'a> {
    pub window: &'a Window,
    pub gpu: &'a mut OverlayGpu,
    pub cursor: &'a mut mouse::Cursor,
    pub modifiers: &'a mut ModifiersState,
}

/// 一个模态弹窗宿主。实现者只提供两个访问器,其余行为走默认方法。
pub(crate) trait OverlayHost {
    fn overlay_window(&self) -> &Window;
    fn overlay_parts(&mut self) -> OverlayParts<'_>;

    fn window_id(&self) -> WindowId {
        self.overlay_window().id()
    }

    fn request_redraw(&self) {
        self.overlay_window().request_redraw();
    }

    /// 主窗口移动/缩放时把弹窗重新贴到主窗口上。
    fn reposition(
        &mut self,
        device: &wgpu::Device,
        main_outer_pos: PhysicalPosition<i32>,
        main_inner_size: PhysicalSize<u32>,
        scale: f64,
    ) {
        let OverlayParts { window, gpu, .. } = self.overlay_parts();
        reposition_overlay(window, gpu, device, main_outer_pos, main_inner_size, scale);
    }

    /// 输入处理样板:Esc 优先(各宿主传入自己的取消消息),其余事件交给 iced。
    ///
    /// `card` 是"此刻的卡片内容"构造函数——各宿主的卡片不同,由调用方拼;
    /// 只在事件确实要派发时才调用(iced 不关心的事件不会白拼一次布局)。
    /// `card` 返回 `None` 表示"此刻没有可渲染的内容"(如所在项目已关闭):
    /// 事件照常转换(保持与原实现的副作用一致),但没有可派发的卡片。
    fn overlay_input<'a>(
        &mut self,
        event: &WindowEvent,
        escape: Option<Message>,
        card: impl FnOnce() -> Option<OverlayElement<'a>>,
    ) -> Vec<Message> {
        if let Some(message) = escape
            && is_escape_press(event)
        {
            return vec![message];
        }
        let OverlayParts {
            window,
            gpu,
            cursor,
            modifiers,
        } = self.overlay_parts();
        let Some(iced_event) = gpu.track_and_convert(cursor, modifiers, event) else {
            return Vec::new();
        };
        let Some(card) = card() else {
            return Vec::new();
        };
        gpu.dispatch(window, *cursor, card, iced_event)
    }
}

/// 只有部分弹窗接"失焦即关闭"(会弹原生文件选择器的宿主刻意不接)。
pub(crate) trait FocusClosable {
    fn focus_tracker(&mut self) -> &mut FocusTracker;

    fn handle_focus(&mut self, focused: bool) -> bool {
        self.focus_tracker().handle_focus(focused)
    }
}

fn is_escape_press(event: &WindowEvent) -> bool {
    matches!(
        event,
        WindowEvent::KeyboardInput {
            event: key_event,
            is_synthetic: false,
            ..
        } if key_event.state == winit::event::ElementState::Pressed
            && key_event.logical_key
                == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
    )
}

/// 弹窗"该开/该关"的三态判断。多个宿主逐字重复,收在这里。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_action_covers_all_four_combinations() {
        assert_eq!(sync_action(true, false), SyncAction::Open);
        assert_eq!(sync_action(false, true), SyncAction::Close);
        assert_eq!(sync_action(true, true), SyncAction::Noop);
        assert_eq!(sync_action(false, false), SyncAction::Noop);
    }
}
