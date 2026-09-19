//! `TabularView` 的视图组装:sheet 切换条(多 sheet 时)+ 截断提示 +
//! 虚拟化网格。消息类型是 `grid::Action`,由上层(workspace)映射到 app
//! `Message::TabularAction`。

use iced_widget::core::{Alignment, Element, Length};
use iced_widget::{button, column, container, row, text};

use super::grid::{self, Action};
use super::{MAX_TABULAR_ROWS, TabularView};

impl TabularView {
    /// 组装整棵表格视图。多 sheet 时顶部一条切换条;截断时插一行提示。
    pub fn view(&self) -> Element<'_, Action, iced_widget::Theme, iced_renderer::Renderer> {
        let colors = byteui::theme::color::current();
        let mut col: iced_widget::Column<'_, Action, iced_widget::Theme, iced_renderer::Renderer> =
            column![].width(Length::Fill).height(Length::Fill);

        if self.sheets.len() > 1 {
            let tabs: Vec<Element<'_, Action, iced_widget::Theme, iced_renderer::Renderer>> = self
                .sheets
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let active = i == self.active_sheet;
                    button(
                        text(s.name.clone())
                            .size(byteui::theme::font::body())
                            .color(if active { colors.cream } else { colors.dim }),
                    )
                    .padding([4, 12])
                    .on_press(Action::SelectSheet(i))
                    .style(
                        move |_t: &iced_widget::Theme, _s: button::Status| button::Style {
                            background: if active {
                                Some(colors.tab_active_bg.into())
                            } else {
                                None
                            },
                            ..button::Style::default()
                        },
                    )
                    .into()
                })
                .collect();
            col = col.push(
                container(row(tabs).spacing(4).align_y(Alignment::Center))
                    .width(Length::Fill)
                    .padding([4, 8]),
            );
        }

        let sheet = self.active_sheet();
        if sheet.truncated {
            col = col.push(
                container(
                    text(format!(
                        "仅显示前 {} 行,共 {} 行",
                        MAX_TABULAR_ROWS, sheet.total_rows
                    ))
                    .size(byteui::theme::font::label())
                    .color(colors.dim),
                )
                .width(Length::Fill)
                .padding([4, 8]),
            );
        }

        col = col.push(
            container(grid::view(self))
                .width(Length::Fill)
                .height(Length::Fill),
        );

        col.into()
    }
}
