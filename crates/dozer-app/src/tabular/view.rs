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

        if self.sheet_names.len() > 1 {
            // 按名字画 tab,不看是否已加载——未加载的 sheet 也要能点(点了
            // 才会触发懒加载,见 `TabularView::apply` 的 `SheetLoadRequest`)。
            let tabs: Vec<Element<'_, Action, iced_widget::Theme, iced_renderer::Renderer>> = self
                .sheet_names
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let active = i == self.active_sheet;
                    button(
                        text(name.clone())
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

        if self.sheet_names.is_empty() {
            // 工作簿本身就没有 sheet(损坏文件之类的极端情况)——加载其实
            // 已经"完成"了,只是没数据,不该一直显示"正在加载"(它不会
            // 再变了,`apply_sheet_loaded` 也没有下标能命中这个空列表)。
            col = col.push(
                container(
                    text("这个文件没有可显示的工作表")
                        .size(byteui::theme::font::label())
                        .color(colors.dim),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill),
            );
            return col.into();
        }
        let Some(sheet) = self.active_sheet() else {
            // 切到的 sheet 还没加载完(初次打开文件、或懒加载中)。
            // `loading_hint` 自带 `center_x/center_y(Fill)`,不需要再包一层
            // 容器(同 usage/search/database/git_log 等既有调用点的手法)。
            col = col.push(byteui::feedback::math_curve::loading_hint(
                byteui::feedback::math_curve::Curve::RoseThree,
                "正在加载表格…",
                48.0,
            ));
            return col.into();
        };
        if sheet.truncated {
            col = col.push(
                container(
                    text(format!(
                        "仅显示前 {MAX_TABULAR_ROWS} 行(文件还有更多,未精确统计总行数)"
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
