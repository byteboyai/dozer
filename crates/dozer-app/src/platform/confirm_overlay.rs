//! 通用"标题+说明+取消/确认"弹窗的独立原生窗口宿主。设计见
//! `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`
//! 「架构」第 1 节。之所以能通用,是因为这一类弹窗的内容已经 100% 由同一个
//! 纯数据结构 `dialog::ConfirmDialog<Message>` 描述——宿主只需要存住这份
//! 数据,`redraw` 时调一次既有的 `dialog::confirm(spec.clone())` 即可
//! 拿到 `Element`,不需要为"内容长什么样"引入 `Box<dyn Fn>` 或标志位。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::core::mouse;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::Message;
use crate::platform::overlay_focus::FocusTracker;
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{
    backdrop_card, open_overlay, popup_card_size, reposition_overlay,
};
use byteui::feedback::dialog;

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

/// 内容有界(1-3 行说明 + 两个按钮),高度固定;宽度随主窗口走
/// `POPUP_WIDTH_FRACTION`(整窗 40%,见 `overlay_window`)。
fn card_logical_size(window: &Window) -> LogicalSize<f32> {
    popup_card_size(window, 200.0)
}

pub(crate) struct ConfirmOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    focus: FocusTracker,
    trigger: ConfirmTrigger,
    spec: dialog::ConfirmDialog<Message>,
    /// 同 `ssh_host_overlay` 等所有会接收鼠标输入的宿主——没有这个字段
    /// `redraw`/`handle_input` 只能传 `mouse::Cursor::Unavailable`,iced
    /// 就永远算不出鼠标落在哪个按钮上,`Confirm`/`Cancel` 按钮点了没反应。
    cursor: mouse::Cursor,
    /// 同其余 7 个定制宿主——自己跟踪这扇窗口收到的
    /// `WindowEvent::ModifiersChanged`,不依赖调用方传入的主窗口全局
    /// modifiers 状态(这扇窗口拿到焦点时才是 Esc/快捷键该参照的那份)。
    modifiers: ModifiersState,
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
        let (window, gpu) =
            open_overlay(main_window, adapter, device, queue, instance, "confirm", el);
        ConfirmOverlay {
            window,
            gpu,
            focus: FocusTracker::default(),
            trigger,
            spec,
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

    pub(crate) fn redraw(&mut self) {
        let card = backdrop_card(
            dialog::confirm(self.spec.clone()),
            card_logical_size(&self.window),
        );
        self.gpu.redraw(&self.window, self.cursor, card);
    }

    /// Esc 与失焦统一发送 `spec.cancel_msg`——五个消费方共用同一条关闭
    /// 路径,不需要逐个判断"这是哪个弹窗、该发哪条 Cancel 消息"。除此之外
    /// 的输入(鼠标移动/点击)要真正喂给 iced,`Confirm`/`Cancel` 按钮才能
    /// 点得动——同其余 7 个定制宿主共用的 `OverlayGpu::track_and_convert`/
    /// `dispatch` 转换+派发手法。
    pub(crate) fn handle_input(&mut self, event: &WindowEvent) -> Vec<Message> {
        if let WindowEvent::KeyboardInput {
            event: key_event,
            is_synthetic: false,
            ..
        } = event
            && key_event.state == winit::event::ElementState::Pressed
            && key_event.logical_key
                == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        {
            return vec![self.spec.cancel_msg.clone()];
        }
        let Some(iced_event) =
            self.gpu
                .track_and_convert(&mut self.cursor, &mut self.modifiers, event)
        else {
            return Vec::new();
        };
        let card = backdrop_card(
            dialog::confirm(self.spec.clone()),
            card_logical_size(&self.window),
        );
        self.gpu
            .dispatch(&self.window, self.cursor, card, iced_event)
    }

    /// 返回 `true` 表示应该关闭——与 `search_overlay`/`file_history_overlay`
    /// 同款失焦即关闭,这五个弹窗都不涉及会弹出原生模态选择器的按钮,不需要
    /// `ProjectCreateOverlay` 那种"不接失焦关闭"的例外。
    pub(crate) fn handle_focus(&mut self, focused: bool) -> bool {
        self.focus.handle_focus(focused)
    }
}

/// 按 `if/else if` 优先级链算出"此刻该显示哪个 confirm 弹窗"(至多一个)。
/// 优先级顺序决定互斥弹窗重叠时谁胜出,逐个 `else if` 追加消费方——必须
/// 跟设计文档「架构」第 1 节 `desired_confirm_spec` 伪代码的顺序一致
/// (files → database → ssh → agent-tab-close → todo-clear,todo 最后),
/// 这是当年 `app/view.rs` 那条 `if/else if` 链本来的优先级,不是随意顺序:
/// 这五个弹窗互斥展示,顺序决定"用户在别的面板还点了别的确认操作"时谁赢。
pub(crate) fn desired_confirm(
    ws: Option<&crate::workspace::Workspace>,
) -> Option<(ConfirmTrigger, dialog::ConfirmDialog<Message>)> {
    let ws = ws?;
    if let Some(spec) = crate::extensions::files::delete_confirm_spec(&ws.files) {
        return Some((ConfirmTrigger::FilesDelete, map_files_spec(spec)));
    }
    if let Some(source_id) = ws.database.delete_confirm()
        && let Some(spec) =
            crate::extensions::database::delete_confirm_spec(&ws.database, source_id)
    {
        return Some((ConfirmTrigger::DatabaseDelete, map_database_spec(spec)));
    }
    if let Some(host_id) = ws.ssh.delete_confirm()
        && let Some(spec) = crate::extensions::ssh::delete_confirm_spec(&ws.ssh, host_id)
    {
        return Some((ConfirmTrigger::SshDelete, map_ssh_spec(spec)));
    }
    if let Some(spec) = crate::workspace::agent_close_confirm_spec(ws) {
        return Some((ConfirmTrigger::AgentTabClose, spec));
    }
    if ws.todo.clear_confirm_open() {
        return Some((
            ConfirmTrigger::TodoClear,
            map_todo_spec(crate::extensions::todo::clear_confirm_spec()),
        ));
    }
    None
}

/// 把 SSH 扩展的 `ConfirmDialog<ssh::Message>` 提升到 app 级。
fn map_ssh_spec(
    spec: dialog::ConfirmDialog<crate::extensions::ssh::Message>,
) -> dialog::ConfirmDialog<Message> {
    dialog::ConfirmDialog {
        icon: spec.icon,
        title: spec.title,
        description: spec.description,
        cancel_label: spec.cancel_label,
        cancel_msg: Message::Ssh(spec.cancel_msg),
        confirm_label: spec.confirm_label,
        confirm_msg: Message::Ssh(spec.confirm_msg),
        close_msg: None,
        confirm_color: spec.confirm_color,
        content_spacing: spec.content_spacing,
    }
}

/// 把 Database 扩展的 `ConfirmDialog<database::Message>` 提升到 app 级。
fn map_database_spec(
    spec: dialog::ConfirmDialog<crate::extensions::database::Message>,
) -> dialog::ConfirmDialog<Message> {
    dialog::ConfirmDialog {
        icon: spec.icon,
        title: spec.title,
        description: spec.description,
        cancel_label: spec.cancel_label,
        cancel_msg: Message::Database(spec.cancel_msg),
        confirm_label: spec.confirm_label,
        confirm_msg: Message::Database(spec.confirm_msg),
        close_msg: None,
        confirm_color: spec.confirm_color,
        content_spacing: spec.content_spacing,
    }
}

/// 把 Files 扩展的 `ConfirmDialog<files::Message>` 提升到 app 级——同
/// `map_todo_spec`,边界处包 `Message::Files`。
fn map_files_spec(
    spec: dialog::ConfirmDialog<crate::extensions::files::Message>,
) -> dialog::ConfirmDialog<Message> {
    dialog::ConfirmDialog {
        icon: spec.icon,
        title: spec.title,
        description: spec.description,
        cancel_label: spec.cancel_label,
        cancel_msg: Message::Files(spec.cancel_msg),
        confirm_label: spec.confirm_label,
        confirm_msg: Message::Files(spec.confirm_msg),
        close_msg: None,
        confirm_color: spec.confirm_color,
        content_spacing: spec.content_spacing,
    }
}

/// 把 Todo 扩展的 `ConfirmDialog<todo::Message>` 提升到 app 级
/// `ConfirmDialog<Message>`(与旧的 in-window 分支 `.map(Message::Todo)`
/// 等价)。通用宿主只认 app 级 `Message`,各消费方自己负责在边界处包一层。
fn map_todo_spec(
    spec: dialog::ConfirmDialog<crate::extensions::todo::Message>,
) -> dialog::ConfirmDialog<Message> {
    dialog::ConfirmDialog {
        icon: spec.icon,
        title: spec.title,
        description: spec.description,
        cancel_label: spec.cancel_label,
        cancel_msg: Message::Todo(spec.cancel_msg),
        confirm_label: spec.confirm_label,
        confirm_msg: Message::Todo(spec.confirm_msg),
        close_msg: None,
        confirm_color: spec.confirm_color,
        content_spacing: spec.content_spacing,
    }
}
