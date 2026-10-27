//! 浏览器面板的**产品侧适配层**:面板本体(状态 / 更新 / 视图)在共享库 `dozer-panel-browser`,
//! 视图向宿主要的展示能力在这里由 `App` 提供,全部转调 Dozer 现有的 chrome 函数(行为与抽库前逐项相同)。

use dozer_panel_browser::{BrowserDisplay, BrowserRegion, TabView};
use iced_widget::Theme;
use iced_widget::core::Element;

type Renderer = iced_renderer::Renderer;

impl BrowserDisplay for crate::app::App {
    fn region(&self) -> BrowserRegion {
        browser_region()
    }

    fn locale(&self) -> &str {
        crate::panel_host::PanelHost::locale(self)
    }

    fn line_height_factor(&self) -> f32 {
        crate::theme::terminal_font::line_height_factor()
    }

    fn tab_divider<'a, M: 'a>(&self) -> Element<'a, M, Theme, Renderer> {
        crate::app::tab_divider()
    }

    fn tab<'a, M: Clone + 'a>(
        &self,
        v: TabView,
        on_select: M,
        on_close: M,
        title_hover: impl Fn(bool) -> M + 'a,
        close_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, Theme, Renderer> {
        crate::chrome::tab_widget::panel_tab(crate::chrome::tab_widget::PanelTabArgs {
            title: v.title,
            active: v.active,
            hover_t: v.hover_t,
            close_hover_t: v.close_hover_t,
            prefix: None,
            suffix: None,
            on_select,
            on_close,
            show_tooltip: v.show_tooltip,
            title_hover,
            close_hover,
        })
    }

    fn bookmarks_divider<'a, M: Clone + 'a>(&self, on_drag: M) -> Element<'a, M, Theme, Renderer> {
        // 分隔条两侧都填浏览器区域底色(没有区域底色时用全局 bg),与抽库前相同。
        let bg = browser_region()
            .background
            .unwrap_or(byteui::theme::color::current().bg);
        crate::app::divider_bar(crate::app::Divider::BrowserBookmarksSplit, bg, bg, on_drag)
    }
}

/// `theme::region::browser_pane()` 投影成库的区域样式(库只用到背景 / 内边距 / 行间距)。
fn browser_region() -> BrowserRegion {
    let r = crate::theme::region::browser_pane();
    BrowserRegion {
        background: r.background,
        padding: r.padding,
        gap: r.gap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 投影逐字段等于 Dozer 主题里的浏览器区域样式。
    #[test]
    fn region_projection_matches_browser_pane_theme() {
        let theme = crate::theme::region::browser_pane();
        let r = browser_region();
        assert_eq!(r.background, theme.background);
        assert_eq!(r.padding, theme.padding);
        assert_eq!(r.gap, theme.gap);
    }
}
