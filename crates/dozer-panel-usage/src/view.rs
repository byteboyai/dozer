//! 用量面板的 view:content_pane/list_pane/agent 筛选栏/汇总卡片。
//!
//! 库化改动(计划 §Task 19 第 2 条):workspace/chrome helper(`home_panel_head`、
//! `agent_icon`/`agent_dot_color`、`list_collapse_button`、`HoverId`/`PanelKind`)
//! 全部经注入的 [`PanelDisplay`] 提供,库内不再引用产品的 `crate::workspace` /
//! `crate::chrome` / `crate::app`。`byteui` 是公共 UI,库内直接用。

use crate::adapter::PanelDisplay;
use crate::aggregate::agents_present;
use crate::state::{Message, WorkspaceState};
use byteui::interaction::icons;
use dozer_model::AgentKind;
use dozer_model::ConversationMeta;
use iced_widget::core::{Border, Color, Element, Length};
use iced_widget::{button, column, container, text};

/// 面板内容侧:顶部"用量"标题 + 加载态动画占位。四态图表内容已迁到
/// `dozer://usage-content` Preact webview(见 protocol.rs::current_view_payload)。
/// `list_pane`(agent 筛选栏)仍原生 iced,接入跟 Database/Agent 面板一样的
/// 可拖拽 split + 收起机制(产品侧的分栏布局)。
pub fn content_pane<'a, M, D>(
    display: &'a D,
    collapsed: bool,
    ws_state: &'a WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>
where
    M: Clone + 'a,
    D: PanelDisplay<M>,
{
    // 套用统一 panel head:Lucide `BarChart3` 图标 + 暖金标题 + 1px 分割线。
    // 刷新不再走面板内按钮——进入面板时由 `state::on_activate` 自动触发。
    // 标题行末尾挂"收起/展开列表列"按钮(收起 agent 筛选栏后仍在此可见
    // 以便恢复)。
    let collapse = display.list_collapse_button(
        collapsed,
        "收起列表",
        "展开列表",
        display.toggle_list_collapse(),
        move |hovered| display.collapse_hover(hovered),
    );
    let head = display.panel_head("用量", Some(collapse));
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
    // 四态:webview 矩形由产品按 `usage_content_pane_bounds_for` 摆放,原生这里
    // 不需要再画任何占位——`Length::Fill` 的这个 `container` 仍然提供面板背景/
    // 边框,webview 盖在它上面(同 Files/Project/GitLog 现状)。
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
/// 接入方式——只在有数据时才由产品侧决定要不要拼两栏(没数据/加载中时只显示
/// `content_pane`,不拼分栏)。
pub fn list_pane<'a, M, D>(
    display: &'a D,
    ws_state: &'a WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>
where
    M: Clone + 'a,
    D: PanelDisplay<M>,
{
    agent_filter_sidebar(
        display,
        ws_state.rows(),
        &agents_present(ws_state.rows()),
        ws_state.agent_filter(),
        width,
        outer,
    )
}

/// agent 筛选栏(2026-08-29 参照 Todo 面板"任务分类"列表重新实现):
/// 头部(面板头 图标+标题+分割线,跟 Todo 左栏头部同一套)+ 竖排导航列表,
/// 每项 图标+名称+右侧计数,跟 `todo_category_button` 逐字段对应。外面套一层
/// 卡片边框,视觉上读成一个独立的子面板。
fn agent_filter_sidebar<'a, M, D>(
    display: &'a D,
    rows: &[(ConversationMeta, crate::aggregate::ConversationUsage)],
    present: &[AgentKind],
    current: Option<AgentKind>,
    width: Length,
    outer: Border,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>
where
    M: Clone + 'a,
    D: PanelDisplay<M>,
{
    let header = container(display.panel_head("Agent", None)).padding(iced_widget::core::Padding {
        top: 12.0,
        right: 12.0,
        bottom: 8.0,
        left: 12.0,
    });

    let mut nav = column![].spacing(4).padding(iced_widget::core::Padding {
        top: 0.0,
        right: 8.0,
        bottom: 12.0,
        left: 8.0,
    });
    nav = nav.push(agent_filter_button(
        display,
        None,
        current,
        "全部agent".to_string(),
        rows.len(),
        // "全部agent" 用公共 `BarChart3`,颜色按选中态在金/暗之间切——这部分
        // 不是产品专属语义,库内直接用 byteui。
        {
            let color = if current.is_none() {
                byteui::theme::color::current().gold
            } else {
                byteui::theme::color::current().dim
            };
            icons::view(
                icons::IconKind::BarChart3,
                byteui::theme::icon_size::row(),
                color,
            )
        },
    ));
    for &agent in present {
        let count = rows.iter().filter(|(m, _)| m.agent == agent).count();
        let icon = display.agent_icon(agent, display.agent_icon_color(agent));
        nav = nav.push(agent_filter_button(
            display,
            Some(agent),
            current,
            agent.label().to_string(),
            count,
            icon,
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
/// `icon` 是展示适配提供的图标元素(已着色);"全部agent" 项传公共 `BarChart3`
/// 元素。颜色只影响文字/计数描边,图标自身着色由调用方在构造 `icon` 时决定。
fn agent_filter_button<'a, M, D>(
    display: &'a D,
    value: Option<AgentKind>,
    current: Option<AgentKind>,
    label: String,
    count: usize,
    icon: Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>
where
    M: Clone + 'a,
    D: PanelDisplay<M>,
{
    let active = value == current;
    let fg = if active {
        byteui::theme::color::current().cream
    } else {
        byteui::theme::color::current().dim
    };
    let count_color = if active {
        byteui::theme::color::current().gold
    } else {
        byteui::theme::color::current().dim
    };
    button(
        iced_widget::row![
            icon,
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
    .on_press(display.wrap(Message::AgentFilterSet(value)))
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
