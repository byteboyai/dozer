//! 群聊「新建群聊」表单的独立原生窗口宿主:居中模态卡片(标题 + 主题输入 +
//! 参与 agent 勾选 + 取消/创建),与 `ssh_host_overlay` 同款 overlay 框架。
//!
//! 与 SSH 表单的区别:**失焦不关闭**(保留已填主题与已选 agent),仅 Esc / 取消 /
//! 创建按钮关闭——这是创建类表单更稳的交互(避免误点主窗口丢掉多选)。但被
//! `close_other_overlays` 摘下时仍发取消消息清状态,否则下一帧 `sync_*` 会把它
//! 重新拉起。`cancel_message` 走 `Message::GroupChat(NewGroupDialogCancel)` 清
//! `ws.group_chat.new_group_dialog_open` 与已选 agent。
//!
//! 设计见 `docs/superpowers/specs/2026-09-23-standard-dialog-overlay-design.md`
//! (独立窗口模态卡片通用规范)。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_widget::core::mouse;
use iced_widget::core::{Element, Length};
use iced_widget::{button, column, container, row};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{App, Message};
use crate::extensions::group_chat::{self, Message as GroupChatMessage};
use crate::platform::overlay_gpu::OverlayGpu;
use crate::platform::overlay_window::{backdrop_card_shrink, open_overlay, reposition_overlay};
use crate::ui_text::text;
use crate::workspace::{agent_dot_color, agent_icon};

/// 卡片固定宽度(高度随内容/agent 行数自适应,用 `backdrop_card_shrink`)。
const CARD_WIDTH: f32 = 420.0;

/// 主题文本框定高:约 3 行(body 14 × 行高 ≈ 19 + 上下 padding 16)。
/// 提交走「创建」按钮/回车提交已移除——多行文本里 Enter 是换行。
const TOPIC_AREA_HEIGHT: f32 = 72.0;

/// 开关决策拆成纯函数,便于单测(同 `ssh_host_overlay::sync_action`)。
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

pub(crate) struct GroupChatCreateOverlay {
    window: Arc<Window>,
    gpu: OverlayGpu,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    /// Drop 时把 macOS 原生右键粘贴菜单指回主窗口(见 `Drop` 实现)。
    main_window: Arc<Window>,
    /// 取消/创建消息需要 project_id,打开时记下。
    project_id: i64,
}

impl GroupChatCreateOverlay {
    pub(crate) fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// `close_other_overlays` 摘掉这扇窗口前要用它发一条取消消息,清掉
    /// `ws.group_chat.new_group_dialog_open` 与已选 agent,否则下一帧会被
    /// `sync_group_chat_create_overlay` 重新拉起。
    pub(crate) fn cancel_message(&self) -> Message {
        Message::GroupChat(GroupChatMessage::NewGroupDialogCancel(self.project_id))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn open(
        main_window: &Arc<Window>,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instance: &wgpu::Instance,
        el: &ActiveEventLoop,
        project_id: i64,
    ) -> GroupChatCreateOverlay {
        let (window, gpu) = open_overlay(
            main_window,
            adapter,
            device,
            queue,
            instance,
            "group-chat-create",
            el,
        );
        window.set_ime_allowed(true);
        #[cfg(target_os = "macos")]
        crate::chrome::native_menu::install_content_view(&window);
        GroupChatCreateOverlay {
            window,
            gpu,
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
            main_window: main_window.clone(),
            project_id,
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

    /// 弹窗内容:`ws.group_chat.new_group_dialog_open()` 为假(已被别处关掉)
    /// 时返回 `None`,`redraw`/`handle_input` 顶部短路,直接不画。
    fn card<'a>(
        ws: &'a crate::workspace::Workspace,
        project_id: i64,
        locale: &str,
    ) -> Option<Element<'a, GroupChatMessage, iced_widget::Theme, iced_renderer::Renderer>> {
        if !ws.group_chat.new_group_dialog_open() {
            return None;
        }
        let c = byteui::theme::color::current();
        let gc = &ws.group_chat;

        let input = byteui::form::text_area::view(
            gc.new_group_topic(),
            &group_chat::t("new_group.placeholder", "New group topic…", locale),
            Some(iced_widget::core::widget::Id::new("group-chat-new-topic")),
            false,
            Some(TOPIC_AREA_HEIGHT),
            move |action| GroupChatMessage::NewGroupTopicAction(project_id, action),
        );

        // 参与 agent 勾选行:遍历后端支持的成员种类。用 `Vec<Element<…>>` 收集,
        // 渲染器由本函数返回类型(`iced_renderer::Renderer`)统一锚定,避免像单独的
        // `Column`/`Button` 局部标注那样在测试构建下触发生命周期/渲染器推断冲突。
        let mut agent_rows: Vec<
            Element<'a, GroupChatMessage, iced_widget::Theme, iced_renderer::Renderer>,
        > = Vec::with_capacity(group_chat::CREATABLE_AGENTS.len());
        for agent in group_chat::CREATABLE_AGENTS {
            let selected = gc.new_group_selected_agents().contains(agent);
            let check = if selected {
                text("✓").size(byteui::theme::font::label()).color(c.green)
            } else {
                text("").size(byteui::theme::font::label()).color(c.dim)
            };
            let icon: Element<GroupChatMessage, iced_widget::Theme, iced_renderer::Renderer> =
                container(byteui::interaction::icons::view(
                    agent_icon(*agent),
                    byteui::theme::icon_size::row(),
                    agent_dot_color(*agent),
                ))
                .into();
            let pid = project_id;
            let icon = icon.map(move |_| GroupChatMessage::Noop(pid));
            let row = row![
                icon,
                text(agent.display_label())
                    .size(byteui::theme::font::body())
                    .color(if selected { c.cream } else { c.dim }),
                check,
            ]
            .align_y(iced_widget::core::alignment::Vertical::Center)
            .spacing(8);
            let btn = button(row)
                .on_press(GroupChatMessage::NewGroupAgentToggled(project_id, *agent))
                .width(Length::Fill)
                .padding(8)
                .style(byteui::interaction::cards::button_card(selected, c.card));
            agent_rows.push(btn.into());
        }

        let cancel = button(
            text(group_chat::t("cancel.button", "Cancel", locale))
                .size(byteui::theme::font::label())
                .color(c.dim),
        )
        .on_press(GroupChatMessage::NewGroupDialogCancel(project_id))
        .padding([6, 12])
        .style(byteui::feedback::dialog::action_button_style(c.dim));

        let create = button(
            text(group_chat::t("new_group.confirm", "Create", locale))
                .size(byteui::theme::font::label())
                .color(c.gold),
        )
        .on_press(GroupChatMessage::NewGroupSubmit(project_id))
        .padding([6, 12])
        .style(byteui::feedback::dialog::action_button_style(c.gold));

        let card = column![
            text(group_chat::t("new_group.title", "New group chat", locale))
                .size(byteui::theme::font::subtitle())
                .color(c.cream),
            input,
            text(group_chat::t(
                "new_group.agents",
                "Participating agents",
                locale
            ))
            .size(byteui::theme::font::caption_sm())
            .color(c.dim),
            iced_widget::Column::with_children(agent_rows).spacing(6),
            byteui::feedback::dialog::actions(row![cancel, create].spacing(8)),
        ]
        .spacing(12);

        Some(
            container(card)
                .padding(16)
                .style(byteui::feedback::dialog::card_style)
                .into(),
        )
    }

    pub(crate) fn redraw(&mut self, app: &mut App) {
        let Some(ws) = app.active_workspace() else {
            return;
        };
        let locale = app.locale.active_locale();
        let Some(card) = Self::card(ws, self.project_id, locale) else {
            return;
        };
        self.gpu.redraw(
            &self.window,
            self.cursor,
            backdrop_card_shrink(card.map(Message::GroupChat), CARD_WIDTH),
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
            return vec![self.cancel_message()];
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
        let locale = app.locale.active_locale();
        let Some(card) = Self::card(ws, self.project_id, locale) else {
            return Vec::new();
        };
        self.gpu.dispatch(
            &self.window,
            self.cursor,
            backdrop_card_shrink(card.map(Message::GroupChat), CARD_WIDTH),
            iced_event,
        )
    }
}

impl Drop for GroupChatCreateOverlay {
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
