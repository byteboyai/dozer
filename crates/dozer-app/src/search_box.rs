//! 统一"搜索框"UI 原语——单行自绘输入 + 嵌在边框内右侧的提交按钮,
//! 处理方式对齐 `crate::menu`(泛化消息类型,任何面板的 `Message` 都能
//! 直接复用)。以 `extensions::todo::todo_search_bar` 为基准抽出来(见
//! git 历史),首个新用处是 homespace 项目列表搜索。
//!
//! 自绘输入(不用 iced 原生 `text_input`)的理由与 `todo`/`files` 搜索框
//! 一致:本 app 每帧重建界面,原生输入留不住焦点,打字会漏进已聚焦的
//! 终端——键盘走调用方自己的 main.rs 拦截层路由,这个组件只管渲染
//! 当前草稿(含可选光标插入符 `▏`)。
//!
//! 颜色/字号不内置——homespace 有自己独立配置的调色板
//! (`theme::homespace_color`,与工作区 `byteui::theme::color` 分开维护),
//! 组件因此不认死一套主题,由调用方传 [`SearchBoxColors`] + 字号。
//!
//! 光标编辑原语(`char_to_byte`/`insert_at_cursor`/`delete_before_cursor`/
//! `move_cursor_in`/`CursorDir`)原来各在 `extensions::todo` 私有一份,
//! 抽公共组件时一并挪到这里——纯字符串/下标操作,不依赖任何 todo 专属
//! 类型,`todo.rs` 现在从这里 `use` 回去,不再各写各的。

use iced_widget::core::{Border, Color, Element, Length};
use iced_widget::{MouseArea, button, container, row, text};

/// 搜索框用到的颜色集,由调用方按自己的主题命名空间填。
pub struct SearchBoxColors {
    pub bg: Color,
    pub border: Color,
    /// 编辑态/已过滤态的描边色,同时也是提交按钮 hover 终点色(两处视觉
    /// 语义都是"当前被强调/激活",复用同一个颜色 token,不再让调用方
    /// 分别传两份)。
    pub active: Color,
    pub text: Color,
    /// 占位符文字 + 提交按钮静止态颜色。
    pub dim: Color,
}

/// 方向键移动自绘光标的方向(`Home`/`End` 跳行首/行尾)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorDir {
    Left,
    Right,
    Home,
    End,
}

/// 字符下标 → 字节下标;`cursor` 越界时回落到串尾(防御性,避免越界 panic)。
pub fn char_to_byte(s: &str, cursor: usize) -> usize {
    s.char_indices()
        .nth(cursor)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

/// 在 `s` 的字符下标 `cursor` 处插入 `ins`,并把 `cursor` 前移插入的字符数。
/// 自绘输入没有原生光标,插入必须落在光标处而非强行 `push_str` 到行尾。
pub fn insert_at_cursor(s: &mut String, cursor: &mut usize, ins: &str) {
    if ins.is_empty() {
        return;
    }
    let byte = char_to_byte(s, *cursor);
    s.insert_str(byte, ins);
    *cursor += ins.chars().count();
}

/// 删除 `cursor` 前一个字符;已在行首时 no-op。返回是否真的删了字符。
pub fn delete_before_cursor(s: &mut String, cursor: &mut usize) -> bool {
    if *cursor == 0 {
        return false;
    }
    let rem = *cursor - 1;
    let start = char_to_byte(s, rem);
    let end = char_to_byte(s, *cursor);
    s.drain(start..end);
    *cursor = rem;
    true
}

/// 按方向键移动 `cursor`(字符下标)。`Left`/`Right` 单步、`Home`/`End` 跳
/// 行首/行尾。光标恒夹在 `[0, len]`,不会越界。
pub fn move_cursor_in(s: &str, cursor: &mut usize, dir: CursorDir) {
    let len = s.chars().count();
    let c = *cursor as isize;
    *cursor = match dir {
        CursorDir::Left => c.saturating_sub(1),
        CursorDir::Right => (c + 1).min(len as isize),
        CursorDir::Home => 0,
        CursorDir::End => len as isize,
    }
    .clamp(0, len as isize) as usize;
}

/// 把 `draft` 从字符下标 `cursor` 处劈开,中间塞 `▏` 当光标。`cursor`
/// 越界时回落到串尾(`char_to_byte` 的防御性兜底)。
pub fn draft_with_caret(draft: &str, cursor: usize) -> String {
    let byte = char_to_byte(draft, cursor);
    let (before, after) = draft.split_at(byte);
    format!("{before}▏{after}")
}

/// 搜索框本体。整框一个 `MouseArea`(点非提交按钮处触发 `on_edit_start`
/// 进编辑态),提交按钮是框内层真正的 `button`,自己先吃掉点击(回车 /
/// 点它都走 `on_submit`,把草稿落成生效的过滤词——这一步由调用方决定
/// 具体怎么过滤,组件不关心)。整框显式定高(内容高 = 提交按钮自身
/// padding 撑出来的高度 + 外框自己的上下 padding),避免 `row!` 内
/// `Length::Fill` 子项在没有确定高度的父级下被撑成远超一行文字的巨框。
#[allow(clippy::too_many_arguments)]
pub fn view<'a, Msg: Clone + 'a>(
    draft: &'a str,
    editing: bool,
    active: bool,
    cursor: usize,
    placeholder: &'a str,
    body_font_size: u32,
    icon_size: f32,
    colors: SearchBoxColors,
    hover_t: f32,
    on_edit_start: Msg,
    on_submit: Msg,
    on_hover: impl Fn(bool) -> Msg + 'a,
) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    let body = if draft.is_empty() && !editing {
        text(placeholder).size(body_font_size).color(colors.dim)
    } else {
        let shown = if editing {
            draft_with_caret(draft, cursor)
        } else {
            draft.to_string()
        };
        text(shown).size(body_font_size).color(colors.text)
    };

    let submit_color = byteui::theme::color::mix(colors.dim, colors.active, hover_t);
    let submit = MouseArea::new(
        button(byteui::interaction::icons::view(
            byteui::interaction::icons::IconKind::Search,
            icon_size,
            submit_color,
        ))
        .on_press(on_submit)
        .padding(6)
        .style(move |_t, _s| button::Style {
            background: None,
            border: Border {
                color: colors.border,
                width: 0.0,
                radius: 4.0.into(),
            },
            text_color: submit_color,
            ..button::Style::default()
        }),
    )
    .on_enter(on_hover(true))
    .on_exit(on_hover(false));

    let content_h = icon_size + 12.0;
    let box_h = content_h + 12.0;
    MouseArea::new(
        container(
            row![
                container(body)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_y(iced_widget::core::alignment::Vertical::Center)
                    .align_x(iced_widget::core::alignment::Horizontal::Left),
                container(submit).align_y(iced_widget::core::alignment::Vertical::Center),
            ]
            .width(Length::Fill)
            .height(Length::Fixed(content_h))
            .align_y(iced_widget::core::Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fixed(box_h))
        .padding([6, 8])
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(colors.bg.into()),
            border: Border {
                color: if editing || active {
                    colors.active
                } else {
                    colors.border
                },
                width: 1.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        }),
    )
    .on_press(on_edit_start)
    .into()
}
