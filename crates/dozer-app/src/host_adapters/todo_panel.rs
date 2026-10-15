//! Todo 面板的**产品侧适配层**(S3a)。
//!
//! 面板本体(状态 / 更新 / 协议 / 视图 / web 资源)已抽到独立库 `dozer-panel-todo`,Dozer 与 Digger 共用唯一实现。
//! 库通过接缝(`dozer_panel_todo::adapter`)反过来要产品提供:
//!
//! - **展示语义**:[`PanelDisplay`](dozer_panel_todo::adapter::PanelDisplay) 的 `App` 实现(悬停进度、折叠按钮、面板头、
//!   头部内边距、分割线)——这是产品展示适配,天然属于 host 侧,故放本层而非 `extensions/todo/`
//!   (面板边界门禁 `R-APP` 要求 `extensions/` 下不出现 `crate::app::App`;见 `scripts/audit/check_panel_boundary.py`)。
//! - **类型映射**:库的 `HoverSlot` / `TextInputTarget` / agent 图标数据 → Dozer 的 `HoverId` / `TextInputTarget` /
//!   `workspace::agent_icon`。
//!
//! `extensions/todo/mod.rs` 只做重新导出(计划:"Dozer 仅保留重新导出 / 注册包装")。

/// Todo 视图向宿主要的展示能力:全部转调 Dozer 现有的 `PanelHost` 与 chrome 函数(行为与抽库前逐项相同)。
impl dozer_panel_todo::adapter::PanelDisplay for crate::app::App {
    fn hover_progress(&self, slot: bytehost_desktop::HoverSlot) -> f32 {
        crate::panel_host::PanelHost::hover_progress(self, hover_id_of(slot))
    }

    fn list_collapsed(&self) -> bool {
        crate::panel_host::PanelHost::list_collapsed(self, crate::app::PanelKind::Todo)
    }

    fn list_collapse_button<'a, M: Clone + 'a>(
        &self,
        collapsed: bool,
        tooltip_collapse: String,
        tooltip_expand: String,
        on_select: M,
        on_hover: impl Fn(bool) -> M + 'a,
    ) -> iced_widget::core::Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
        crate::panel_host::PanelHost::list_collapse_button(
            self,
            crate::app::PanelKind::Todo,
            collapsed,
            crate::app::HoverId::list_collapse(crate::app::PanelKind::Todo),
            tooltip_collapse,
            tooltip_expand,
            on_select,
            on_hover,
        )
    }

    fn panel_head<'a, M: 'a>(
        &self,
        icon: byteui::interaction::icons::IconKind,
        title: String,
    ) -> iced_widget::core::Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
        crate::chrome::homespace::home_panel_head(icon, title)
    }

    fn header_padding(&self) -> iced_widget::core::Padding {
        crate::theme::region::project_pane().padding
    }

    fn tab_divider<'a, M: 'a>(&self) -> iced_widget::core::Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
        crate::app::tab_divider()
    }

    fn locale(&self) -> &str {
        crate::panel_host::PanelHost::locale(self)
    }
}

/// 库的悬停槽位 → Dozer 悬停动画表的键。
pub(crate) fn hover_id_of(slot: bytehost_desktop::HoverSlot) -> crate::app::HoverId {
    crate::app::HoverId::Panel(crate::app::PanelKind::Todo, slot)
}

/// 库的文本框目标 → Dozer 的。
pub(crate) fn text_target_of(t: dozer_panel_todo::adapter::TextInputTarget) -> crate::app::TextInputTarget {
    crate::app::TextInputTarget { id: t.id, secure: t.secure }
}

/// agent 图标:库不认识 `workspace::agent_icon`,在这里取字节交给它。
pub(crate) fn agent_icon_data(agent: dozer_model::AgentKind) -> dozer_panel_todo::adapter::AgentIconData {
    let icon = crate::workspace::agent_icon(agent);
    dozer_panel_todo::adapter::AgentIconData { bytes: icon.bytes().to_vec(), preserves_color: icon.preserves_original_color() }
}

/// 同库的 `current_view_payload`,但图标取自 Dozer 的 `workspace::agent_icon`,调用点不用改。
pub(crate) fn current_view_payload(
    todo: &dozer_panel_todo::WorkspaceState,
    session_alive: &dyn Fn(&str) -> bool,
    project_id: i64,
    today: (i32, u32, u32),
) -> dozer_panel_todo::TodoViewPayload {
    dozer_panel_todo::current_view_payload(todo, session_alive, project_id, today, &agent_icon_data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 评审关注点 3:库里的 `HoverSlot` 映射回的 `HoverId`,必须与抽库前 `HoverId::row(..)` / `HoverId::list_collapse(..)` 构造的完全相同。
    #[test]
    fn hover_slots_map_back_to_the_same_hover_ids() {
        use crate::app::{HoverId, PanelKind};
        use bytehost_desktop::HoverSlot;
        assert_eq!(hover_id_of(HoverSlot::Row(42)), HoverId::row(PanelKind::Todo, 42));
        assert_eq!(hover_id_of(HoverSlot::ListCollapse), HoverId::list_collapse(PanelKind::Todo));
        assert_eq!(hover_id_of(HoverSlot::Choice(3)), HoverId::choice(PanelKind::Todo, 3));
    }

    /// 库里的 `agents_are_the_four_headless_kinds_with_inline_svg` 用桩图标;这里用 Dozer 的真图标验证同一件事。
    #[test]
    fn the_real_agent_icons_are_inline_svg() {
        let agents = dozer_panel_todo::agent_vms(&agent_icon_data);
        assert_eq!(agents.len(), 4);
        assert!(agents.iter().all(|a| a.icon_svg.contains("<svg")));
    }
}
