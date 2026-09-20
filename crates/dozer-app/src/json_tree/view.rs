//! `JsonTreeView` 的视图组装：Tree/RawText 切换头 + 树主体。
//!
//! 与 tabular 不同：JSON 的「原始文本」半边不在这里渲染 —— 外层
//! `workspace/view.rs` 在 `RawText` 模式下渲染 tab 已有的原生 `editor`
//! (CodeView)，本模块只负责切换控件，以及 Tree 模式下的树主体。这样切换
//! 复用同一份已加载状态，不重新解析文件。
//!
//! 切换控件用 `button` + `icons::view`（统一图标渲染入口），而非
//! `icon_button_entry`：后者需要调用方传入 `app.hover_progress(..)` 算好的
//! hover 进度与 `HoverId`，而 `JsonTreeView::view(&self)` 拿不到 `App`。

use iced_widget::core::{Element, Length};
use iced_widget::{button, column, container, row, text};

use super::{Action, JsonTreeView, ViewMode};

impl JsonTreeView {
    pub fn view(&self) -> Element<'_, Action, iced_widget::Theme, iced_renderer::Renderer> {
        let colors = byteui::theme::color::current();
        let mut col: iced_widget::Column<'_, Action, iced_widget::Theme, iced_renderer::Renderer> =
            column![].width(Length::Fill).height(Length::Fill);

        let (icon, tooltip) = match self.view_mode {
            ViewMode::Tree => (
                byteui::interaction::icons::IconKind::FileCode,
                "查看原始文本",
            ),
            ViewMode::RawText => (
                byteui::interaction::icons::IconKind::FolderTree,
                "查看 Tree",
            ),
        };
        let toggle = button(
            container(byteui::interaction::icons::view(
                icon,
                byteui::theme::icon_size::scale() * 16.0,
                colors.dim,
            ))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
        )
        .width(Length::Fixed(byteui::theme::icon_size::scale() * 26.0))
        .height(Length::Fixed(byteui::theme::icon_size::scale() * 26.0))
        .padding(0)
        .on_press(Action::ToggleViewMode)
        .style(
            |_t: &iced_widget::Theme, _s: button::Status| button::Style {
                background: None,
                ..button::Style::default()
            },
        );

        col = col.push(
            container(
                row![
                    text(tooltip)
                        .size(byteui::theme::font::label())
                        .color(colors.dim)
                        .width(Length::Fill),
                    toggle,
                ]
                .align_y(iced_widget::core::Alignment::Center),
            )
            .width(Length::Fill)
            .padding([4, 8]),
        );

        if self.view_mode == ViewMode::RawText {
            // 原始文本由外层渲染；这里只保留头部（含切换控件），保证用户
            // 在 RawText 模式下仍能看到并点回 Tree。
            return col.into();
        }

        col = col.push(
            container(super::tree::view(self))
                .width(Length::Fill)
                .height(Length::Fill),
        );
        col.into()
    }
}
