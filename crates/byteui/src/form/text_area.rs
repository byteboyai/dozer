//! 多行自增高文本编辑框——包一层 `iced_widget::text_editor`,样式对齐
//! `form::input_text`(卡片底色+描边,聚焦金框由 iced 内置 `Status::Focused`
//! 驱动)。高度默认 `Length::Shrink`(iced text_editor 的默认值),随内容
//! 自然撑高,不需要额外的自增高逻辑。

use iced_widget::core::{Border, Element};
use iced_widget::text_editor::{self, Status};

pub fn view<'a, Message: Clone + 'a>(
    content: &'a text_editor::Content,
    placeholder: &'a str,
    on_action: impl Fn(text_editor::Action) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    iced_widget::text_editor(content)
        .placeholder(placeholder)
        .on_action(on_action)
        .size(crate::theme::font::body())
        .padding(8)
        .style(|_theme: &iced_widget::Theme, status: Status| {
            let colors = crate::theme::color::current();
            let focused = matches!(status, Status::Focused { .. });
            text_editor::Style {
                background: colors.card.into(),
                border: Border {
                    color: if focused { colors.gold } else { colors.border },
                    width: 1.0,
                    radius: 6.0.into(),
                },
                placeholder: colors.dim,
                value: colors.cream,
                selection: crate::theme::color::mix(colors.gold, colors.card, 0.6),
            }
        })
        .into()
}
