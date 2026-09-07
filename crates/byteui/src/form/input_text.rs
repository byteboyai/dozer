//! amis `form/input-text`(单行文本输入):<https://baidu.github.io/amis/zh-CN/components/form/input-text>

use iced_widget::core::widget;
use iced_widget::core::{Border, Element, Length};
use iced_widget::text_input::{self, Status};

/// [`view`] 的实装:把真 `text_input` 按默认 UI 字号(box 缺省 `body`)烤出来。
#[allow(clippy::too_many_arguments)]
pub fn view<'a, Message: Clone + 'a>(
    placeholder: &str,
    value: &str,
    secure: bool,
    id: Option<widget::Id>,
    highlight: bool,
    on_submit: Option<Message>,
    bare: bool,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    view_at_size(
        crate::theme::font::body() as f32,
        placeholder,
        value,
        secure,
        id,
        highlight,
        on_submit,
        bare,
        on_input,
    )
}

/// [`view`] 的字号可显式给的版本:`size`(px)放在最前,供只需某只在单一字号
/// 下输入的调用方(如原生预览的 File-Find 条,要跟右侧编辑器的代码字号对齐)
/// 用,其余同 [`view`]。通用 `view` 仍走缺省 `body`,别动别的调用方观感。
#[allow(clippy::too_many_arguments)]
pub fn view_at_size<'a, Message: Clone + 'a>(
    size: f32,
    placeholder: &str,
    value: &str,
    secure: bool,
    id: Option<widget::Id>,
    highlight: bool,
    on_submit: Option<Message>,
    bare: bool,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let input = iced_widget::text_input(placeholder, value)
        .secure(secure)
        .on_input(on_input)
        .on_submit_maybe(on_submit)
        .size(size)
        .padding(8);
    let input = if let Some(id) = id {
        input.id(id)
    } else {
        input
    };
    input
        .style(move |_theme: &iced_widget::Theme, status: Status| {
            let colors = crate::theme::color::current();
            let focused = matches!(status, Status::Focused { .. });
            if bare {
                return text_input::Style {
                    background: iced_widget::core::Color::TRANSPARENT.into(),
                    border: Border {
                        color: iced_widget::core::Color::TRANSPARENT,
                        width: 0.0,
                        radius: 0.0.into(),
                    },
                    icon: colors.dim,
                    placeholder: colors.dim,
                    value: colors.cream,
                    selection: crate::theme::color::mix(colors.gold, colors.card, 0.6),
                };
            }
            text_input::Style {
                background: colors.card.into(),
                border: Border {
                    color: if focused || highlight {
                        colors.gold
                    } else {
                        colors.border
                    },
                    width: 1.0,
                    radius: 6.0.into(),
                },
                icon: colors.dim,
                placeholder: colors.dim,
                value: colors.cream,
                selection: crate::theme::color::mix(colors.gold, colors.card, 0.6),
            }
        })
        .into()
}

/// `view` 的变体:在输入框**同一圈边框内**靠右内嵌一个后缀控件(`suffix`,
/// 如 Find 条的「Aa」大小写开关)。结构参照 `search_box::view_with_prefix`
/// 的既有做法:text_input 自身恒透明无边框(`padding(0)`),外框由 container
/// 统一画 card 底色/边框——观感与非 bare `view` 对齐(radius 6、card 底)。
/// 代价同 search_box:边框高亮不再看 iced `text_input::Status::Focused`,由
/// 调用方传 `highlight`(如"查询词非空"),聚焦但空 query 时无金框提示。
#[allow(clippy::too_many_arguments)]
pub fn view_with_suffix<'a, Message: Clone + 'a>(
    placeholder: &str,
    value: &str,
    secure: bool,
    id: Option<widget::Id>,
    highlight: bool,
    on_submit: Option<Message>,
    on_input: impl Fn(String) -> Message + 'a,
    suffix: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    view_with_suffix_at_size(
        crate::theme::font::body() as f32,
        placeholder,
        value,
        secure,
        id,
        highlight,
        on_submit,
        on_input,
        suffix,
    )
}

/// [`view_with_suffix`] 的字号可显式给的版本(`size` 在最前)。配套
/// [`view_at_size`],供原生预览 File-Find 这类要跟代码编辑器字号走同一条线的
/// 输入框用。
#[allow(clippy::too_many_arguments)]
pub fn view_with_suffix_at_size<'a, Message: Clone + 'a>(
    size: f32,
    placeholder: &str,
    value: &str,
    secure: bool,
    id: Option<widget::Id>,
    highlight: bool,
    on_submit: Option<Message>,
    on_input: impl Fn(String) -> Message + 'a,
    suffix: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = crate::theme::color::current();
    let mut input = iced_widget::text_input(placeholder, value)
        .secure(secure)
        .on_input(on_input)
        .on_submit_maybe(on_submit)
        .size(size)
        .padding(0)
        .style(move |_theme: &iced_widget::Theme, _status: Status| {
            // 边框/底色由外层容器统一画(见下),输入框自己恒透明。
            text_input::Style {
                background: iced_widget::core::Color::TRANSPARENT.into(),
                border: Border {
                    color: iced_widget::core::Color::TRANSPARENT,
                    width: 0.0,
                    radius: 0.0.into(),
                },
                icon: colors.dim,
                placeholder: colors.dim,
                value: colors.cream,
                selection: crate::theme::color::mix(colors.gold, colors.card, 0.6),
            }
        });
    if let Some(id) = id {
        input = input.id(id);
    }
    let field: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> = input.into();
    iced_widget::container(
        iced_widget::row![
            iced_widget::container(field)
                .width(Length::Fill)
                .align_y(iced_widget::core::alignment::Vertical::Center),
            iced_widget::container(suffix).align_y(iced_widget::core::alignment::Vertical::Center),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    )
    // padding 8 对齐非 bare `view` 里 text_input 自带的 `.padding(8)`,
    // 条高观感不因内嵌后缀而变。
    .padding(8)
    .style(
        move |_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(colors.card.into()),
            border: Border {
                color: if highlight {
                    colors.gold
                } else {
                    colors.border
                },
                width: 1.0,
                radius: 6.0.into(),
            },
            ..iced_widget::container::Style::default()
        },
    )
    .into()
}
