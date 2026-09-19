//! 用量面板的 view:content_pane/list_pane/agent 筛选栏/汇总卡片。

use crate::chrome::homespace::{home_panel_head, home_section_head};
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
    let rows = ws_state.rows();
    let loading = ws_state.loading();
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

    if loading {
        content = content.push(byteui::feedback::math_curve::loading_hint(
            byteui::feedback::math_curve::Curve::RoseThree,
            "统计中…",
            64.0,
        ));
    } else if rows.is_empty() {
        content = content.push(
            text("这个项目还没有 agent 对话记录")
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        );
    } else {
        // agent 筛选栏拆到 `list_pane`(独立面板函数,见下)。这里下面所有
        // 统计区改吃 `filtered_rows`。
        let filtered_rows = filter_rows_by_agent(rows, ws_state.agent_filter);
        if filtered_rows.is_empty() {
            content = content.push(
                text("这个 agent 在当前项目还没有用量数据")
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().dim),
            );
        } else {
            let usages: Vec<ConversationUsage> =
                filtered_rows.iter().map(|(_, u)| u.clone()).collect();
            // 每个"小节标题 + 它的图表"作为一个独立内层 column 组装,内层用手调
            // 的较大 `SECTION_CHART_GAP`,让标题跟随后的图表之间有更富余的间距;
            // 外层 `content` 默认 spacing(12) 只负责小节与小节、与小节上方面板
            // 标题等之间的常规间距——两者解耦,避免一刀切把别处空隙也放大。
            let proj_section = column![home_section_head("项目用量统计")]
                .spacing(SECTION_CHART_GAP)
                .push(align_to_section_title(project_summary_boxes(&aggregate(
                    &usages,
                ))));
            content = content.push(proj_section);

            // 内容在"选中具体 agent"与"全部 agent"两态用两套统计:选单个 agent
            // 时,Agent 的横向对比饼图和"每日按 agent 分组"的柱子都失去意义
            // (大饼只有一片、每日柱只剩本地那一根),所以切成一连串"该 agent"
            // 的按天趋势;只有"全部 agent"才保留横向 + 每日对比布局。2026-09-05。
            match ws_state.agent_filter {
                Some(agent) => {
                    // 趋势轴右端定位到"今天"(UTC),让"最近 N 天"从今天往回铺,
                    // 不会因为最近几天没活动就把窗口漂走。
                    let today = today_day_index();
                    let session = session_round_trend(rows, agent, today);
                    let session_series = session_trend_series();
                    if let Some(sec) = trend_chart_section("Session 趋势", session_series, &session)
                    {
                        content = content.push(sec);
                    }
                    if let Some(sec) = token_trend_section(agent, rows, today) {
                        content = content.push(sec);
                    }
                }
                None => {
                    // —— 以下为"全部 agent"(`None`)态 ——
                    // "Agent 用量统计"内按两个大组竖排;每组 = 一条概况横幅 + 一
                    // 对彼此等分面板宽度的环图格:
                    //   组一 Sessions/Rounds:横幅 "Session(会话总数 total),
                    //   下配 Session 环 + Round(回合)环;
                    //   组二 Tokens:横幅 "Tokens(全项目四项 token total)",
                    //   下配 Input/Output 环 + Cache Read/Write 环。
                    // 圆环与逐 agent 数字表并存(环本体保留 2026-08-28 决定,只是图
                    // 例用 `chart_stat_list` 的数字表格式)。每个口径各自归总、跳过
                    // 空口径,避免给某 agent 画永远 0 的占位扇区。
                    let session_share = agent_session_share(&filtered_rows);
                    let turn_share = agent_turn_share(&filtered_rows);
                    let io_share = agent_io_token_share(&filtered_rows);
                    let cache_share = agent_cache_token_share(&filtered_rows);
                    let any_agent_metric = !session_share.is_empty()
                        || !turn_share.is_empty()
                        || !io_share.is_empty()
                        || !cache_share.is_empty();
                    if any_agent_metric {
                        let mut agent_metrics_section =
                            column![home_section_head("Agent 用量统计")].spacing(SECTION_CHART_GAP);

                        let sess_total = share_total(&session_share);
                        let has_session_group = !session_share.is_empty() || !turn_share.is_empty();
                        if has_session_group {
                            agent_metrics_section = agent_metrics_section.push(
                                align_to_section_title(metric_group_banner("Session", sess_total)),
                            );
                            agent_metrics_section = agent_metrics_section.push(
                                align_to_section_title(pair_metric_cells(
                                    (!session_share.is_empty())
                                        .then_some(("Session", session_share.as_slice())),
                                    (!turn_share.is_empty())
                                        .then_some(("Round", turn_share.as_slice())),
                                )),
                            );
                        }

                        let has_token_group = !io_share.is_empty() || !cache_share.is_empty();
                        if has_token_group {
                            // 组二横幅总数用"全项目(即全部 agent 视图整组)四项
                            // token 之和",与底下两个 token 细分环是"大盘 vs 细拆"
                            // 视角,不求等于两个环各自 total 相加。
                            let total_tokens: u64 = filtered_rows
                                .iter()
                                .map(|(_, u)| {
                                    u.tokens_in
                                        + u.tokens_out
                                        + u.tokens_cache_read
                                        + u.tokens_cache_write
                                })
                                .sum();
                            agent_metrics_section = agent_metrics_section.push(
                                align_to_section_title(metric_group_banner("Tokens", total_tokens)),
                            );
                            agent_metrics_section = agent_metrics_section.push(
                                align_to_section_title(pair_metric_cells(
                                    (!io_share.is_empty())
                                        .then_some(("Input/Output", io_share.as_slice())),
                                    (!cache_share.is_empty())
                                        .then_some(("Cache Read/Write", cache_share.as_slice())),
                                )),
                            );
                        }

                        content = content.push(agent_metrics_section);
                    }

                    let days = daily_totals_by_agent(&filtered_rows);
                    if !days.is_empty() {
                        let day_section = column![home_section_head("每日用量统计")]
                            .spacing(SECTION_CHART_GAP)
                            .push(align_to_section_title(bar_chart(&days)));
                        content = content.push(day_section);
                    }
                }
            }
        }
    }

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

pub(crate) fn stat(
    label: &'static str,
    value: String,
    color: Color,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    column![
        text(label)
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().dim),
        text(value)
            .size(15.0)
            .color(color)
            .font(iced_widget::core::Font::default()),
    ]
    .spacing(2)
    .into()
}

/// 一个带边框的统计卡片:一行 `stat` 并排。`project_summary_boxes` 拿它
/// 拼出两张卡(会话/回合/工具调用/触达文件 一张,四个 token 分项另一张)
/// ——参照设计草图,两张卡各自成框、并排放,不是原来的单卡通栏。
pub(crate) fn stat_box(
    stats: Vec<Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer>>,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut row = iced_widget::row![].spacing(24);
    for s in stats {
        row = row.push(s);
    }
    container(row)
        .width(Length::Fill)
        .padding(12)
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(byteui::theme::color::current().card.into()),
            border: Border {
                width: 1.0,
                color: byteui::theme::color::current().border,
                radius: 10.0.into(),
            },
            ..iced_widget::container::Style::default()
        })
        .into()
}

/// "项目用量统计"区块:两张并排的卡片。会话数(`conversation_count`)
/// 原来只出现在小字说明行里,草图把它列成正式的一格统计,这里跟着改。
/// `工具调用(改动)`的改动数细分草图没画,不在这张汇总卡上重复(单会话
/// 行已在「会话明细」阶段移除,改动数本身仍由 `mutating_tool_calls` 统计)。
pub(crate) fn project_summary_boxes(
    totals: &ProjectUsageTotals,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let cream = byteui::theme::color::current().cream;
    let cyan = byteui::theme::color::current().cyan;
    let activity_box = stat_box(vec![
        stat("会话", format_count(totals.conversation_count), cream),
        stat("回合", format_count(totals.turns), cream),
        stat("工具调用", format_count(totals.tool_calls), cream),
        stat("触达文件", format_count(totals.files_touched), cream),
    ]);
    let token_box = stat_box(vec![
        stat("Input", format_count(totals.tokens_in), cyan),
        stat("Output", format_count(totals.tokens_out), cyan),
        stat("cache 读", format_count(totals.tokens_cache_read), cyan),
        stat("cache 写", format_count(totals.tokens_cache_write), cyan),
    ]);
    // 两张卡各自等分面板宽度、上下拉开成对排布,同"Agent 用量统计"里左右两
    // 张饼图卡的宽度与排法(2026-09-07 要求):每张吃掉 `FillPortion(1)`、外层
    // 行撑满、16px 间距——不加的话 `stat_box` 天然按内容收窄,两卡会左贴紧、
    // 不等宽地摆,失去"成对均分"的观感。
    let even_half =
        |boxed: Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer>| {
            let mut cell = iced_widget::container(boxed);
            cell = cell.width(Length::FillPortion(1));
            cell
        };
    iced_widget::row![even_half(activity_box), even_half(token_box)]
        .width(Length::Fill)
        .spacing(16)
        .into()
}
