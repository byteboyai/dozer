//! amis `progress`(进度条):<https://baidu.github.io/amis/zh-CN/components/progress>

use iced_widget::core::{Border, Color, Element};
use iced_widget::progress_bar;

/// `value` 是 0.0..=1.0 的完成度,超出范围会被夹到区间内。
pub fn view<'a, Message: 'a>(
    value: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    iced_widget::progress_bar(0.0..=1.0, value.clamp(0.0, 1.0))
        .girth(6.0)
        .style(|_theme: &iced_widget::Theme| {
            let colors = crate::theme::color::current();
            progress_bar::Style {
                background: colors.card.into(),
                bar: colors.gold.into(),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 3.0.into(),
                },
            }
        })
        .into()
}
