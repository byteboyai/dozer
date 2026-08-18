//! amis `form/checkbox`(复选框):<https://baidu.github.io/amis/zh-CN/components/form/checkbox>

use iced_widget::checkbox::{self, Status};
use iced_widget::core::{Border, Element};

pub fn view<'a, Message: Clone + 'a>(
    label: &'a str,
    is_checked: bool,
    on_toggle: impl Fn(bool) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    iced_widget::checkbox(is_checked)
        .label(label)
        .on_toggle(on_toggle)
        .style(|_theme: &iced_widget::Theme, status: Status| {
            let colors = crate::theme::color::current();
            let checked = matches!(
                status,
                Status::Active { is_checked: true } | Status::Hovered { is_checked: true }
            );
            checkbox::Style {
                background: colors.card.into(),
                icon_color: colors.gold,
                border: Border {
                    color: if checked { colors.gold } else { colors.border },
                    width: 1.0,
                    radius: 4.0.into(),
                },
                text_color: Some(colors.cream),
            }
        })
        .into()
}
