//! 通用"标题+说明+取消/确认"弹窗的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`
//! 「架构」第 1 节。之所以能通用,是因为这一类弹窗的内容已经 100% 由同一个
//! 纯数据结构 `dialog::ConfirmDialog<Message>` 描述——宿主只需要存住这份
//! 数据,`redraw` 时调一次既有的 `dialog::confirm(spec.clone(), w)` 即可
//! 拿到 `Element`,不需要为"内容长什么样"引入 `Box<dyn Fn>` 或标志位。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::core::mouse;
use iced_winit::runtime::user_interface::UserInterface;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::Message;
use crate::dialog;
use crate::platform::overlay_focus::FocusTracker;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{centered_overlay_bounds, open_child_window};

/// 五个 confirm 形态弹窗的判别标签——只用来在 `sync_confirm_overlay` 里
/// 判断"这次 desired 和已开的窗口是不是同一个弹窗",不需要 `Message`/
/// `ConfirmDialog` 派生 `PartialEq`(`Message` 枚举很大,不适合整体派生)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfirmTrigger {
    FilesDelete,
    DatabaseDelete,
    SshDelete,
    AgentTabClose,
    TodoClear,
}

/// 内容有界(1-3 行说明 + 两个按钮),固定逻辑尺寸覆盖全部 5 个用例,不需要
/// 按主窗口比例缩放(同 `settings`/`search` 的取舍,不同于 `file_history`)。
fn card_logical_size() -> LogicalSize<f32> {
    LogicalSize::new(420.0, 200.0)
}

pub(crate) struct ConfirmOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    trigger: ConfirmTrigger,
    spec: dialog::ConfirmDialog<Message>,
}

impl ConfirmOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(crate) fn trigger(&self) -> ConfirmTrigger {
        self.trigger
    }

    /// Esc/失焦/原生关闭按钮统一走这条关闭路径——发 `spec.cancel_msg`,
    /// 五个消费方共用,不需要逐个判断"这是哪个弹窗、该发哪条 Cancel 消息"。
    pub(crate) fn cancel_message(&self) -> Message {
        self.spec.cancel_msg.clone()
    }

    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        trigger: ConfirmTrigger,
        spec: dialog::ConfirmDialog<Message>,
        el: &ActiveEventLoop,
    ) -> ConfirmOverlay {
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
        let window = open_child_window(main_window, pos, size, "confirm", el);
        let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);
        ConfirmOverlay {
            window,
            gpu,
            focus: FocusTracker::default(),
            trigger,
            spec,
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

    pub(crate) fn redraw(&mut self) {
        let card = dialog::confirm(self.spec.clone(), card_logical_size().width);
        let mut interface = UserInterface::build(
            card,
            self.gpu.viewport.logical_size(),
            std::mem::take(&mut self.gpu.cache),
            &mut self.gpu.renderer,
        );
        let _ = interface.update(
            &[],
            mouse::Cursor::Unavailable,
            &mut self.gpu.renderer,
            &mut self.gpu.clipboard,
            &mut Vec::new(),
        );
        interface.draw(
            &mut self.gpu.renderer,
            &iced_winit::core::Theme::Dark,
            &iced_winit::core::renderer::Style::default(),
            mouse::Cursor::Unavailable,
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

    /// Esc 与失焦统一发送 `spec.cancel_msg`——五个消费方共用同一条关闭
    /// 路径,不需要逐个判断"这是哪个弹窗、该发哪条 Cancel 消息"。
    pub(crate) fn handle_input(
        &mut self,
        event: &WindowEvent,
        modifiers: ModifiersState,
    ) -> Vec<Message> {
        if let WindowEvent::KeyboardInput {
            event: key_event, ..
        } = event
            && key_event.state == winit::event::ElementState::Pressed
            && key_event.logical_key
                == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            return vec![self.spec.cancel_msg.clone()];
        }
        let _ = modifiers;
        Vec::new()
    }

    /// 返回 `true` 表示应该关闭——与 `search_overlay`/`file_history_overlay`
    /// 同款失焦即关闭,这五个弹窗都不涉及会弹出原生模态选择器的按钮,不需要
    /// `ProjectCreateOverlay` 那种"不接失焦关闭"的例外。
    pub(crate) fn handle_focus(&mut self, focused: bool) -> bool {
        self.focus.handle_focus(focused)
    }
}

/// 按 `if/else if` 优先级链算出"此刻该显示哪个 confirm 弹窗"(至多一个)。
/// Task 3 起逐个把 `else if` 换成真实触发条件——现在还没有任何消费方接入
/// 通用宿主,诚实返回 `None`,不是"以后再实现"的占位符。
pub(crate) fn desired_confirm(
    _ws: Option<&crate::workspace::Workspace>,
) -> Option<(ConfirmTrigger, dialog::ConfirmDialog<Message>)> {
    None
}
