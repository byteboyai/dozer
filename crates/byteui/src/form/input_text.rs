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
    view_at_size_impl(
        false,
        size,
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

/// [`view_at_size`] 的背景色变体:非 `bare` 态底色用 `colors.bg`(而非默认的
/// `colors.card`)——跟原生预览"文件内搜索"输入框(`find_field_shell`)同一套
/// 底色,供需要跟深色面板背景齐平、观感对齐的表单套用(数据库/主机新增表单
/// 统一成文件内搜索输入框风格,2026-09-11 需求)。其余(边框/圆角/聚焦态
/// 描金逻辑)与 [`view_at_size`] 完全一致。
#[allow(clippy::too_many_arguments)]
pub fn view_on_bg_at_size<'a, Message: Clone + 'a>(
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
    view_at_size_impl(
        true,
        size,
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

/// [`view_on_bg_at_size`] 的缺省字号版本,参照 [`view`] 与 [`view_at_size`]
/// 的关系。
#[allow(clippy::too_many_arguments)]
pub fn view_on_bg<'a, Message: Clone + 'a>(
    placeholder: &str,
    value: &str,
    secure: bool,
    id: Option<widget::Id>,
    highlight: bool,
    on_submit: Option<Message>,
    bare: bool,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    view_on_bg_at_size(
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

/// `view_at_size`/`view_on_bg_at_size` 的共用实装,`on_bg` 选非 `bare` 态的
/// 底色 token(`true` 取 `colors.bg`,`false` 取 `colors.card`)。
#[allow(clippy::too_many_arguments)]
fn view_at_size_impl<'a, Message: Clone + 'a>(
    on_bg: bool,
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
    // `bare=true` 时不画自己的框/底(边框/底色留给调用方外层 container),
    // 内边距也该交出去——否则调用方即便把外层 padding 调到跟另一个 bare
    // 输入框一致,这里内建的 8px 还是会让占位符文字整体多缩进一截,两个
    // 框的文字起点对不上(2026-09-07 file-find 查询/替换框左对齐问题的
    // 根因)。
    let input = iced_widget::text_input(placeholder, value)
        .secure(secure)
        .on_input(on_input)
        .on_submit_maybe(on_submit)
        .size(size)
        .padding(if bare { 0 } else { 8 });
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
                background: (if on_bg { colors.bg } else { colors.card }).into(),
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
/// 的既有做法(即[`view_with_suffix_at_size_flags`] 传 `framed`);text_input
/// 自身恒透明无边框(`padding(0)`),卡片底与边框分两种:
///
/// - `framed=true`(本函数与 [`view_with_suffix_at_size`]):
///   由内部 container 统一画 card 底色/边框,观感与非 bare `view` /
///   [`view_at_size`] 对齐(radius 6、card 底)。代价:边框高亮不再看 iced
///   `text_input::Status::Focused`,由调用方传 `highlight`。
/// - `framed=false`([`view_with_suffix_unframed_at_size`]):
///   背景与边框都被调用方接管(File-Find 把"查询行+替换行"合成一整块 card
///   时用),这里的输入框只是透明行,不自己画出方框。
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
    view_with_suffix_at_size_flags(
        true,
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
/// 输入框用(带自己一圈 card 框,`framed=true`)。
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
    view_with_suffix_at_size_flags(
        true,
        size,
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

/// [`view_with_suffix_at_size`] 的 `framed=false` 同款:后缀输入框不自带 card
/// 底/框,底色边框留给调用方(File-Find 单块搜索框)。其余等同 `_at_size`。
#[allow(clippy::too_many_arguments)]
pub fn view_with_suffix_unframed_at_size<'a, Message: Clone + 'a>(
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
    view_with_suffix_at_size_flags(
        false,
        size,
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

/// 后缀输入框的共用实装。`framed=true` 画成孤立的 card 圆角小框(card 底 + 金/
/// 边色 1px 边框 + radius 6);`framed=false` 原样返回「输入框+后缀」的透明行,
/// 由调用方外面盖统一 card。
#[allow(clippy::too_many_arguments)]
fn view_with_suffix_at_size_flags<'a, Message: Clone + 'a>(
    framed: bool,
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
            // 边框/底色天然透明——framed 时由外层 container 画,unframed 时由
            // 调用方的整块 card 垫底,均不进 text_input 自身。
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
    let row = iced_widget::row![
        iced_widget::container(field)
            .width(Length::Fill)
            .align_y(iced_widget::core::alignment::Vertical::Center),
        iced_widget::container(suffix).align_y(iced_widget::core::alignment::Vertical::Center),
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);
    if !framed {
        return iced_widget::container(row).width(Length::Fill).into();
    }
    // padding 8 对齐非 bare `view` 里 text_input 自带的 `.padding(8)`,
    // 条高观感不因内嵌后缀而变。
    iced_widget::container(row)
        .width(Length::Fill)
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
