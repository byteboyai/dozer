//! amis `form/switch`(开关):<https://baidu.github.io/amis/zh-CN/components/form/switch>

use iced_widget::core::Element;
use iced_widget::toggler::{self, Status};

pub fn view<'a, Message: Clone + 'a>(
    label: &'a str,
    is_on: bool,
    on_toggle: impl Fn(bool) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    iced_widget::toggler(is_on)
        .label(label)
        .on_toggle(on_toggle)
        .style(|_theme: &iced_widget::Theme, status: Status| {
            let colors = crate::theme::color::current();
            let on = matches!(
                status,
                Status::Active { is_toggled: true } | Status::Hovered { is_toggled: true }
            );
            toggler::Style {
                background: if on {
                    colors.gold.into()
                } else {
                    colors.border.into()
                },
                background_border_width: 0.0,
                background_border_color: iced_widget::core::Color::TRANSPARENT,
                foreground: colors.cream.into(),
                foreground_border_width: 0.0,
                foreground_border_color: iced_widget::core::Color::TRANSPARENT,
                text_color: Some(colors.cream),
                border_radius: None,
                padding_ratio: 0.1,
            }
        })
        .into()
}
