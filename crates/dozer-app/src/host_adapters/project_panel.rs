//! Project 面板的**产品侧适配层**(S5a)。
//!
//! 面板本体(状态 / 更新 / 视图)已抽到独立库 `dozer-panel-project`,Dozer 与 Digger 共用唯一实现。
//! 库通过接缝(`dozer_panel_project::adapter`)反过来要产品提供展示语义:
//!
//! - **展示语义**:[`PanelDisplay`](dozer_panel_project::adapter::PanelDisplay) 的 `App` 实现(面板头、
//!   头部内边距、分割线、locale)——这是产品展示适配,天然属于 host 侧,故放本层而非
//!   `extensions/project/`(面板边界门禁 `R-APP` 要求 `extensions/` 下不出现 `crate::app::App`)。
//! - **类型映射**:库的 `TextInputTarget` → Dozer 的 `crate::app::TextInputTarget`。
//!
//! `extensions/project.rs` 只做重新导出。

/// Project 视图向宿主要的展示能力:全部转调 Dozer 现有的 chrome 函数(行为与抽库前逐项相同)。
impl dozer_panel_project::adapter::PanelDisplay<crate::app::Message> for crate::app::App {
    fn wrap(&self, message: dozer_panel_project::Message) -> crate::app::Message {
        crate::app::Message::Project(message)
    }

    fn panel_head<'a>(
        &self,
        icon: byteui::interaction::icons::IconKind,
        title: String,
    ) -> iced_widget::core::Element<
        'a,
        crate::app::Message,
        iced_widget::Theme,
        iced_renderer::Renderer,
    > {
        crate::chrome::homespace::home_panel_head(icon, title)
    }

    fn locale(&self) -> &str {
        crate::panel_host::PanelHost::locale(self)
    }
}

/// 本面板的悬停动画进度:从内核的 hover 动画表取值。
pub(crate) fn pane_hover(app: &crate::app::App) -> dozer_panel_project::view::ProjectPaneHover {
    use crate::app::{HoverId, PanelKind};
    dozer_panel_project::view::ProjectPaneHover {
        docs_add: app.hover_progress(HoverId::named(PanelKind::Project, "docs_add")),
        remote_add: app.hover_progress(HoverId::named(PanelKind::Project, "remote_add")),
        memory_add: app.hover_progress(HoverId::named(PanelKind::Project, "memory_add")),
    }
}

/// 库的文本框目标 → Dozer 的。
pub(crate) fn text_target_of(
    t: dozer_panel_project::adapter::TextInputTarget,
) -> crate::app::TextInputTarget {
    crate::app::TextInputTarget {
        id: t.id,
        secure: t.secure,
    }
}

/// Dozer 在「删除项目」/「修复项目」卡片上的产品参数:有 agent 转录,文案里的产品名是 `dozer`(与抽库前逐字相同)。
pub(crate) const DOZER_CARD: dozer_panel_project::view::CardOptions<'static> =
    dozer_panel_project::view::CardOptions {
        agent_history: true,
        product: "dozer",
    };
