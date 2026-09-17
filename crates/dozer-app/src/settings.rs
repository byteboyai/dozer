// crates/dozer-app/src/settings.rs
//! 顶栏设置齿轮弹窗:目前只有"配色方案"一项(深色 ByteBoy2077 / 浅色
//! ByteBoy2077-Light)。选中即调 `byteui::theme::color::set_scheme` 全局
//! 生效并落盘,不需要"确认"按钮——同 `database::drivers_popup` 勾选驱动
//! 那种"点即生效"的弱交互密度,不是表单。

use crate::app::Message;
use byteui::theme::color::ColorScheme;
use iced_widget::core::{Element, Length};
use iced_widget::{column, container, text};

fn scheme_row<'a>(
    label: &'static str,
    scheme: ColorScheme,
    current: ColorScheme,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let selected = scheme == current;
    let mark: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        text(if selected { "✓" } else { " " })
            .size(byteui::theme::font::body())
            .into();
    crate::chrome::menu::item_row_fill(
        Some(mark),
        label,
        if selected {
            byteui::theme::color::current().gold
        } else {
            byteui::theme::color::current().body
        },
        Some(Message::SettingsThemeSelected(scheme)),
    )
}

pub fn settings_modal<'a>(
    window_width: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let current = byteui::theme::color::current_scheme();

    let title = text("主题")
        .size(byteui::theme::font::subtitle())
        .color(byteui::theme::color::current().cream);

    let close = iced_widget::button(
        text("关闭")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim),
    )
    .on_press(Message::SettingsClose)
    .padding([6, 12])
    .style(crate::dialog::action_button_style(
        byteui::theme::color::current().dim,
    ));

    let dialog = container(
        column![
            title,
            scheme_row("深色 · ByteBoy2077", ColorScheme::Dark, current),
            scheme_row("浅色 · ByteBoy2077-Light", ColorScheme::Light, current),
            crate::dialog::actions(iced_widget::row![close]),
        ]
        .spacing(10),
    )
    .padding(16)
    .width(crate::dialog::width(window_width))
    .style(crate::dialog::card_style);

    container(dialog)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}
