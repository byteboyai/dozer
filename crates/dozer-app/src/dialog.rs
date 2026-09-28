//! 统一弹窗(确认框/模态对话框)样式原语。
//!
//! 之前各面板各写一份"CARD 底 + 描边 + 遮罩"(文件树删除/移动确认、
//! 项目删除/修复进度、SSH 删主机确认、搜索弹窗、Todo 详情),描边色统一
//! 用中性 `BORDER`、圆角在 6/8 之间漂移——视觉与"弹窗该有的分量感"逐处
//! 不一致。现在把外壳原语收拢成共享的几件套:
//!
//! - `card_style`: 弹窗卡片本体的容器样式(`theme::region::dialog()`——
//!   PANEL 底 + 金色 `GOLD` 描边,呼应放大态浮层同款"金色描边盒",见
//!   `theme::region::maximize_overlay`)。
//! - `actions`: 弹窗底部"取消/确认"这类操作按钮行的统一落位——靠右下角
//!   纯按钮的收尾操作行;像 Todo 详情"回复框+提交"那种输入控件占满宽度
//!   的行不适用,继续各自布局。
//! - `action_button_border_color`/`action_button_style`: 弹窗内取消/确认/
//!   删除类按钮的统一描边规则(2026-09-15 起)——此前各弹窗各写一份
//!   `button::Style` 字面量,描边色跟文字色绑死(取消=中性 `BORDER`、
//!   确认/删除=各自的语义色),且都不响应 hover。现在统一成:静止态描边
//!   固定 `BORDER`(#1c3440),悬浮/按下态一律变 `GOLD`;按钮语义只通过
//!   文字色区分(灰=取消/次要、红=危险删除、金=主要确认),描边规则本身
//!   不因语义而不同。项目信息面板 footer-bar(`extensions::project::
//!   project_footer_bar`)背景走面板底色 `bg` 而非弹窗卡片 `card`,没法直接
//!   复用 `action_button_style`,但描边规则复用同一个
//!   `action_button_border_color`。
//!
//! (2026-09 弹窗独立窗口化后,12 个模态卡片弹窗全部迁到各自的独立原生
//! 窗口(见 `platform::*_overlay`),主窗口内不再叠遮罩;原先的
//! `scrim`/`scrim_layer` 满窗遮罩原语已无调用点,删除。)

use crate::theme;
use byteui::interaction::icons::IconKind;
use iced_widget::core::{Border, Color, Element, Length, alignment::Horizontal};
use iced_widget::{Row, button, column, container, row, text};

/// 弹窗卡片容器样式:CARD 底 + 金色描边 + 圆角,取代各面板各自手写的
/// `container::Style` 字面量。
pub fn card_style(_t: &iced_widget::Theme) -> container::Style {
    let region = theme::region::dialog();
    container::Style {
        background: region.background.map(Into::into),
        border: region.border.unwrap_or_default(),
        ..container::Style::default()
    }
}

/// 弹窗底部操作按钮行:靠右下角对齐(取消在左、确认在右的相对顺序不变,
/// 只是整行不再贴左/居中)。
pub fn actions<'a, Msg: 'a>(
    row: Row<'a, Msg, iced_widget::Theme, iced_renderer::Renderer>,
) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    container(row)
        .width(Length::Fill)
        .align_x(Horizontal::Right)
        .into()
}

/// 弹窗/footer-bar 按钮共用的描边规则:静止态固定 `BORDER`(#1c3440),
/// 悬浮/按下态一律变 `GOLD`。
pub fn action_button_border_color(status: button::Status) -> Color {
    match status {
        button::Status::Hovered | button::Status::Pressed => byteui::theme::color::current().gold,
        _ => byteui::theme::color::current().border,
    }
}

/// 弹窗操作按钮(取消/确认/删除)完整样式:PANEL 底 + 统一描边规则 + 调用方
/// 指定的文字色。文字色按语义传:`dim` 给取消/次要,`red` 给危险删除,
/// `gold` 给非破坏性的主要确认(如"移动"弹窗的"确定")。底色与弹窗卡片
/// 同走 `panel` 主题色,保持一致。
pub fn action_button_style(
    text_color: Color,
) -> impl Fn(&iced_widget::Theme, button::Status) -> button::Style {
    move |_t, s| {
        let colors = byteui::theme::color::current();
        button::Style {
            // 静止态底走 `panel`，悬浮/按下态提亮到 `card`——配合描边变金的
            // 既有规则，给弹窗操作按钮一个明确的 hover 反馈。
            background: Some(
                match s {
                    button::Status::Hovered | button::Status::Pressed => colors.card,
                    _ => colors.panel,
                }
                .into(),
            ),
            text_color,
            border: Border {
                color: action_button_border_color(s),
                width: 1.0,
                radius: 4.0.into(),
            },
            ..button::Style::default()
        }
    }
}

/// `confirm()` 的入参——字段数≥7 且 `title`/`description` 两个相邻同类型
/// `String` 传错顺序编译器发现不了，按 CLAUDE.md 关键裁决用具名字段结构体
/// 代替位置参数。
#[derive(Clone)]
pub struct ConfirmDialog<Msg> {
    /// 标题前的可选图标（无图标传 `None`，如文件/主机/数据源删除确认）。
    pub icon: Option<IconKind>,
    pub title: String,
    pub description: String,
    pub cancel_label: String,
    pub cancel_msg: Msg,
    pub confirm_label: String,
    pub confirm_msg: Msg,
    /// 右上角关闭按钮：传 `Some(msg)` 才在标题行右侧渲染 × 图标按钮
    /// （如关 Agent tab 确认框），`None` 不渲染（其余四处确认框沿用旧样）。
    /// × 与「取消」语义等价——都关掉弹窗、不执行确认动作。
    pub close_msg: Option<Msg>,
    /// 确认按钮文字色：`red` 给危险删除，`gold` 给非破坏性主要确认。
    pub confirm_color: Color,
    /// 标题/说明/按钮行之间的纵向间距——四处原弹窗的 `column.spacing`
    /// 不统一(files/ssh/database 用 8、todo 用 12)，为了让 `confirm()` 覆盖
    /// 四处又不改视觉，这里不写死，由各调用方原样搬它原本的间距值。
    pub content_spacing: f32,
}

/// 标题 + 说明 + 取消/确认两按钮的确认弹窗骨架——五处 confirm 弹窗
/// (todo 清列表 / files 删除 / database 删数据源 / ssh 删主机 / workspace
/// 关 Agent tab)共用同一份组装，取代此前多处手写。
/// 只适用于"纯文字+两按钮"的简单确认框；带输入框/单选组等额外控件的弹窗
/// (`files::files_move_card`/`project::project_delete_confirm_popup`)
/// 不适用，继续各自实现。
///
/// 卡片宽度固定用 `Length::Fill`,不是 `width(window_width)`——唯一调用方
/// `confirm_overlay.rs` 已经是弹窗独立窗口化之后的独立原生子窗口(固定
/// 420×200 逻辑像素画布),不是当年"浮在整个主窗口 `Stack` 上"的那个语境,
/// 卡片理应填满自己的宿主窗口(同 `settings_card`/`file_history_card` 这些
/// 已迁移消费方的既有写法),而不是取主窗口宽度的 1/3——那样会在 420 宽的
/// 窗口里画出一张 140 宽的卡片,四周留一圈空窗口。
pub fn confirm<'a, Msg: 'a + Clone>(
    spec: ConfirmDialog<Msg>,
) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    let title_content: Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> =
        match spec.icon {
            Some(icon) => row![
                byteui::interaction::icons::view(
                    icon,
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().cream,
                ),
                text(spec.title)
                    .size(byteui::theme::font::subtitle())
                    .color(byteui::theme::color::current().cream),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center)
            .into(),
            None => text(spec.title)
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().cream)
                .into(),
        };
    // 标题行右侧的可选 × 关闭按钮：仅 `close_msg` 为 `Some` 时渲染，把按钮
    // 推到最右；无关闭按钮时整行就是标题本身。
    let header: Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> = match spec.close_msg
    {
        Some(close_msg) => {
            let colors = byteui::theme::color::current();
            let close_btn = button(byteui::interaction::icons::view(
                IconKind::X,
                byteui::theme::icon_size::row(),
                colors.dim,
            ))
            .on_press(close_msg)
            .padding(4)
            .style(
                move |_t: &iced_widget::Theme, s: button::Status| button::Style {
                    background: match s {
                        button::Status::Hovered | button::Status::Pressed => Some(
                            Color {
                                a: 0.15,
                                ..colors.gold
                            }
                            .into(),
                        ),
                        _ => None,
                    },
                    border: Border {
                        width: 0.0,
                        ..Border::default()
                    },
                    ..button::Style::default()
                },
            );
            row![title_content, iced_widget::space::horizontal(), close_btn]
                .align_y(iced_widget::core::Alignment::Center)
                .into()
        }
        None => title_content,
    };
    let cancel = button(
        text(spec.cancel_label)
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().dim),
    )
    .on_press(spec.cancel_msg)
    .padding([6, 12])
    .style(action_button_style(byteui::theme::color::current().dim));
    let confirm = button(
        text(spec.confirm_label)
            .size(byteui::theme::font::label())
            .color(spec.confirm_color),
    )
    .on_press(spec.confirm_msg)
    .padding([6, 12])
    .style(action_button_style(spec.confirm_color));

    let dialog = container(
        column![
            header,
            text(spec.description)
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().dim),
            actions(row![cancel, confirm].spacing(8)),
        ]
        .spacing(spec.content_spacing),
    )
    .width(Length::Fill)
    .padding(16)
    .style(card_style);

    container(dialog)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}

#[cfg(test)]
mod confirm_tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    enum TestMsg {
        Cancel,
        Confirm,
    }

    #[test]
    fn confirm_dialog_struct_carries_all_fields() {
        // 这个测试只验证 ConfirmDialog 结构体字段可以正常构造和读取——
        // confirm() 返回 iced Element，无法在单测里断言内部渲染结构，
        // 真正的视觉/交互核对在 Task 6-9 的人工核对步骤里做。
        let spec = ConfirmDialog {
            icon: None,
            title: "删除文件 \"a.txt\"?".to_string(),
            description: "会移入系统回收站。".to_string(),
            cancel_label: "取消".to_string(),
            cancel_msg: TestMsg::Cancel,
            confirm_label: "删除".to_string(),
            confirm_msg: TestMsg::Confirm,
            close_msg: None,
            confirm_color: byteui::theme::color::current().red,
            content_spacing: 8.0,
        };
        assert_eq!(spec.title, "删除文件 \"a.txt\"?");
        assert_eq!(spec.confirm_msg, TestMsg::Confirm);
    }

    #[test]
    fn confirm_dialog_is_cloneable() {
        let spec = ConfirmDialog {
            icon: None,
            title: "标题".to_string(),
            description: "说明".to_string(),
            cancel_label: "取消".to_string(),
            cancel_msg: TestMsg::Cancel,
            confirm_label: "确认".to_string(),
            confirm_msg: TestMsg::Confirm,
            close_msg: None,
            confirm_color: byteui::theme::color::current().red,
            content_spacing: 8.0,
        };
        let cloned = spec.clone();
        assert_eq!(cloned.title, spec.title);
        assert_eq!(cloned.confirm_msg, spec.confirm_msg);
    }
}
