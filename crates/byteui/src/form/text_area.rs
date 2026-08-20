//! 多行自增高文本编辑框——包一层 `iced_widget::text_editor`,样式对齐
//! `form::input_text`(卡片底色+描边,聚焦金框由 iced 内置 `Status::Focused`
//! 驱动)。`height` 为 `None` 时用 iced text_editor 的默认 `Length::Shrink`,
//! 随内容自然撑高;为 `Some(h)` 时用 `Length::Fixed(h)`——供需要"手动拖拽
//! 定高,内容超出走内部滚动"的调用方使用(如 Todo 添加框的拖拽调高手柄)。

use iced_widget::core::{Border, Element, Length, widget};
use iced_widget::text_editor::{self, Status};

pub fn view<'a, Message: Clone + 'a>(
    content: &'a text_editor::Content,
    placeholder: &'a str,
    id: Option<widget::Id>,
    bare: bool,
    height: Option<f32>,
    on_action: impl Fn(text_editor::Action) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let editor = iced_widget::text_editor(content)
        .placeholder(placeholder)
        .on_action(on_action)
        .size(crate::theme::font::body())
        .padding(8);
    let editor = if let Some(id) = id {
        editor.id(id)
    } else {
        editor
    };
    let editor = if let Some(h) = height {
        editor.height(Length::Fixed(h))
    } else {
        editor
    };
    editor
        .style(move |_theme: &iced_widget::Theme, status: Status| {
            let colors = crate::theme::color::current();
            let focused = matches!(status, Status::Focused { .. });
            if bare {
                return text_editor::Style {
                    background: iced_widget::core::Color::TRANSPARENT.into(),
                    border: Border {
                        color: iced_widget::core::Color::TRANSPARENT,
                        width: 0.0,
                        radius: 0.0.into(),
                    },
                    placeholder: colors.dim,
                    value: colors.cream,
                    selection: crate::theme::color::mix(colors.gold, colors.card, 0.6),
                };
            }
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
