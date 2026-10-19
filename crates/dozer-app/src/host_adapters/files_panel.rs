//! Files 面板的**产品侧适配层**(S5b-1)。
//!
//! 面板本体(状态 / 更新 / 视图 / 拖拽状态机)已抽到独立库 `dozer-panel-files`。
//! 库通过 `dozer_panel_files::adapter::PanelDisplay` 反过来要产品提供展示语义,
//! 这里全部转调 Dozer 现有的 chrome / theme 函数,行为与抽库前逐项相同。

impl dozer_panel_files::adapter::PanelDisplay for crate::app::App {
    fn panel_head<'a>(
        &self,
        icon: byteui::interaction::icons::IconKind,
        title: String,
    ) -> iced_widget::core::Element<
        'a,
        dozer_panel_files::Message,
        iced_widget::Theme,
        iced_renderer::Renderer,
    > {
        crate::chrome::homespace::home_panel_head(icon, title)
    }

    fn header_padding(&self) -> iced_widget::core::Padding {
        crate::theme::region::project_pane().padding
    }

    fn tree_row_font_size(&self) -> f32 {
        crate::workspace::tree_row_font_size()
    }

    fn line_height_factor(&self) -> f32 {
        crate::theme::terminal_font::line_height_factor()
    }

    fn region(&self) -> byteui::theme::region::RegionStyle {
        let pane = crate::theme::region::project_pane();
        byteui::theme::region::RegionStyle {
            background: pane.background,
            border: pane.border,
            padding: pane.padding,
            gap: pane.gap,
            margin: pane.margin,
        }
    }

    fn locale(&self) -> &str {
        crate::panel_host::PanelHost::locale(self)
    }

    fn window_size(&self) -> (f32, f32) {
        crate::panel_host::PanelHost::window_size(self)
    }

    fn last_cursor(&self) -> (f32, f32) {
        crate::panel_host::PanelHost::last_cursor(self)
    }
}
