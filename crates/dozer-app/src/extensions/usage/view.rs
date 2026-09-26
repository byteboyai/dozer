//! 用量面板的 view:content_pane/list_pane/agent 筛选栏/汇总卡片。

use crate::chrome::homespace::home_panel_head;
use crate::conversation::ConversationMeta;
use byteui::interaction::icons;
use dozer_core::protocol::AgentKind;
use iced_widget::core::{Border, Color, Element, Length};
use iced_widget::{button, column, container, text};

use super::*;

pub fn content_pane<'a>(
    app: &crate::app::App,
    ws_state: &'a WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 套用统一 panel head:Lucide `BarChart3` 图标 + 暖金 `#dcc9a3` 的 "用量"
    // 标题 + 1px 分割线。刷新不再走面板内按钮——进入面板时由
    // `Workspace::spawn_usage_refresh` 自动触发(见 `panel_select`)。
    // 标题行末尾挂"收起/展开列表列"按钮(收起 agent 筛选栏后仍在此可见
    // 以便恢复),照抄 `database.rs` 的 `list_collapse_button` 用法。
    let collapse = app.list_collapse_button(
        crate::app::PanelKind::Usage,
        app.list_collapsed(crate::app::PanelKind::Usage),
        crate::app::HoverId::UsageListCollapse,
        "收起列表",
        "展开列表",
        Message::ToggleListCollapse,
        move |hovered| Message::Hover(crate::app::HoverId::UsageListCollapse, hovered),
    );
    let head = crate::chrome::homespace::home_panel_head_with_actions(
        icons::IconKind::BarChart3,
        "用量",
        Some(collapse),
    );
    let mut content = column![head].spacing(12).padding(14).width(Length::Fill);
    if ws_state.loading() {
        // 统计中(`math_curve` 动画)时没有 webview 可挂,原生渲染那个动画——
        // 唯一还留在这里的"内容态"分支,其余四态全部搬进
        // `dozer://usage-content` webview(见 protocol.rs::current_view_payload)。
        content = content.push(byteui::feedback::math_curve::loading_hint(
            byteui::feedback::math_curve::Curve::RoseThree,
            "统计中…",
            64.0,
        ));
    }
    // `rows.is_empty()`/该 agent 无数据/单 agent 趋势/全部 agent 汇总
    // 四态:webview 矩形由 `App::preview_desired` 按
    // `usage_content_pane_bounds_for` 摆放,原生这里不需要再画任何占位
    // ——`Length::Fill` 的这个 `container` 仍然提供面板背景/边框,
    // webview 盖在它上面(同 Files/Project/GitLog 现状)。
    container(content)
        .width(width)
        .height(Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(byteui::theme::color::current().panel.into()),
                border: outer,
                ..iced_widget::container::Style::default()
            },
        )
        .into()
}

/// 面板列表侧:agent 筛选栏。跟 Database/Agent 等面板的"列表列"同一套
/// 接入方式——只在有数据时才由调用方(app.rs `PanelKind::Usage` 分支)
/// 决定要不要拿这个函数拼两栏(没数据/加载中时只显示 `content_pane`,
/// 不拼分栏,同改造前 `sidebar: Option<..>` 为 `None` 时的行为)。
pub fn list_pane<'a>(
    ws_state: &'a WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    agent_filter_sidebar(
        ws_state.rows(),
        &agents_present(ws_state.rows()),
        ws_state.agent_filter,
        width,
        outer,
    )
}

/// agent 筛选栏(2026-08-29 参照 Todo 面板"任务分类"列表重新实现):
/// 头部(`home_panel_head` 图标+标题+分割线,跟 Todo 左栏头部同一套)+
/// 竖排导航列表,每项 图标+名称+右侧计数,跟 `todo_category_button` 逐字段
/// 对应,不再是没有计数、也没有独立头部/边框的一截裸列表。外面套一层
/// 卡片边框,视觉上读成一个独立的子面板,而不是浮在内容区里的按钮堆。
pub(crate) fn agent_filter_sidebar<'a>(
    rows: &[(ConversationMeta, ConversationUsage)],
    present: &[AgentKind],
    current: Option<AgentKind>,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let header = container(home_panel_head(icons::IconKind::Bot, "Agent")).padding(
        iced_widget::core::Padding {
            top: 12.0,
            right: 12.0,
            bottom: 8.0,
            left: 12.0,
        },
    );

    let mut nav = column![].spacing(4).padding(iced_widget::core::Padding {
        top: 0.0,
        right: 8.0,
        bottom: 12.0,
        left: 8.0,
    });
    nav = nav.push(agent_filter_button(
        None,
        current,
        "全部agent".to_string(),
        rows.len(),
        icons::IconKind::BarChart3,
        None,
    ));
    for &agent in present {
        let count = rows.iter().filter(|(m, _)| m.agent == agent).count();
        nav = nav.push(agent_filter_button(
            Some(agent),
            current,
            agent.label().to_string(),
            count,
            crate::workspace::agent_icon(agent),
            Some(crate::workspace::agent_dot_color(agent)),
        ));
    }

    container(column![header, nav])
        .width(width)
        .height(Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(byteui::theme::color::current().bg.into()),
                border: outer,
                ..iced_widget::container::Style::default()
            },
        )
        .into()
}

/// 单个筛选项,字段逐一对应 `todo_category_button`:图标 + 名称 + 右侧
/// 计数,选中态 `CARD` 底 + `GOLD` 1px 描边、计数变金,未选中暗色。
/// `icon_color` 为 `None` 时(仅"全部agent")图标跟着选中态在金/暗之间切;
/// 传了具体颜色(各 agent 自己的品牌色,同饼图/圆点配色)时图标固定用那个
/// 颜色,不随选中态变,方便跟面板别处的同色圆点对上号——这一点是跟
/// `todo_category_button` 唯一的差异,因为分类导航没有"每类自己的颜色"
/// 这个概念,agent 筛选栏有。
pub(crate) fn agent_filter_button<'a>(
    value: Option<AgentKind>,
    current: Option<AgentKind>,
    label: String,
    count: usize,
    icon: icons::IconKind,
    icon_color: Option<Color>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let active = value == current;
    let fg = if active {
        byteui::theme::color::current().cream
    } else {
        byteui::theme::color::current().dim
    };
    let icon_color = icon_color.unwrap_or(if active {
        byteui::theme::color::current().gold
    } else {
        byteui::theme::color::current().dim
    });
    let count_color = if active {
        byteui::theme::color::current().gold
    } else {
        byteui::theme::color::current().dim
    };
    button(
        iced_widget::row![
            icons::view(icon, byteui::theme::icon_size::row(), icon_color),
            text(label).size(byteui::theme::font::body()).color(fg),
            iced_widget::space::Space::new()
                .width(Length::Fill)
                .height(Length::Shrink),
            text(format!("{count}"))
                .size(byteui::theme::font::caption())
                .color(count_color),
        ]
        .spacing(8)
        .align_y(iced_widget::core::Alignment::Center),
    )
    .on_press(Message::AgentFilterSet(value))
    .width(Length::Fill)
    .padding([8, 10])
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: if active {
            Some(byteui::theme::color::current().card.into())
        } else {
            None
        },
        text_color: fg,
        border: Border {
            color: if active {
                byteui::theme::color::current().gold
            } else {
                Color::TRANSPARENT
            },
            width: if active { 1.0 } else { 0.0 },
            radius: 6.0.into(),
        },
        ..button::Style::default()
    })
    .into()
}
