// crates/dozer-app/src/usage.rs
//! Agent 用量统计面板的数据层与视图层（spec
//! docs/superpowers/specs/2026-08-07-agent-usage-panel-design.md）。数据层是
//! 纯函数,不碰 iced/IO,镜像 `transcript.rs` 的按 agent 分派解析方式;
//! `dozerd`/`dozer-core::protocol` 完全不参与——所有数据直接读磁盘上的
//! agent transcript JSONL。

use crate::conversation::ConversationMeta;
use crate::homespace::{home_panel_head, home_section_head};
use byteui::interaction::icons;
use dozer_core::protocol::AgentKind;
use iced_widget::canvas::{self, Canvas};
use iced_widget::core::{Border, Color, Element, Length, Point, Radians, Rectangle};
use iced_widget::tooltip::{Position, Tooltip};
use iced_widget::{column, container, stack, text};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// 挂在每个 Workspace 上的 Usage 面板状态,对应现有 `Workspace` 上
/// `usage`/`usage_loading` 两个字段。
#[derive(Default)]
pub struct WorkspaceState {
    rows: Vec<(ConversationMeta, ConversationUsage)>,
    loading: bool,
}

impl WorkspaceState {
    pub fn rows(&self) -> &[(ConversationMeta, ConversationUsage)] {
        &self.rows
    }

    pub fn loading(&self) -> bool {
        self.loading
    }

    /// 供内核 `PanelSelect(PanelKind::Usage)` 分支调用——切到面板时
    /// 立即标记"统计中",不等 `spawn_refresh` 的异步结果落地才置真。
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }
}

/// 对应现在顶层 `Message` 里的 `UsageLoaded` 变体,去前缀原样搬来。
/// `Refresh`/`Hover` 随手动刷新按钮一起移除——进入面板时由
/// `Workspace::spawn_usage_refresh` 自动刷新,不再需要面板内按钮。
#[derive(Debug, Clone)]
pub enum Message {
    Loaded(i64, Vec<(ConversationMeta, ConversationUsage)>),
}

/// 单个会话（= 一份 transcript 文件）的用量统计。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConversationUsage {
    pub turns: u32,
    pub tool_calls: u32,
    pub mutating_tool_calls: u32,
    pub files_touched: BTreeSet<String>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cache_read: u64,
    pub tokens_cache_write: u64,
}

impl From<&dozer_core::protocol::UsagePayload> for ConversationUsage {
    fn from(p: &dozer_core::protocol::UsagePayload) -> Self {
        Self {
            turns: p.turns,
            tool_calls: p.tool_calls,
            mutating_tool_calls: p.mutating_tool_calls,
            files_touched: p.files_touched.clone(),
            tokens_in: p.tokens_in,
            tokens_out: p.tokens_out,
            tokens_cache_read: p.tokens_cache_read,
            tokens_cache_write: p.tokens_cache_write,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectUsageTotals {
    pub conversation_count: u32,
    pub turns: u32,
    pub tool_calls: u32,
    pub mutating_tool_calls: u32,
    pub files_touched: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cache_read: u64,
    pub tokens_cache_write: u64,
}

pub fn aggregate(rows: &[ConversationUsage]) -> ProjectUsageTotals {
    let mut files = BTreeSet::new();
    let mut totals = ProjectUsageTotals {
        conversation_count: rows.len() as u32,
        ..Default::default()
    };
    for r in rows {
        totals.turns += r.turns;
        totals.tool_calls += r.tool_calls;
        totals.mutating_tool_calls += r.mutating_tool_calls;
        totals.tokens_in += r.tokens_in;
        totals.tokens_out += r.tokens_out;
        totals.tokens_cache_read += r.tokens_cache_read;
        totals.tokens_cache_write += r.tokens_cache_write;
        files.extend(r.files_touched.iter().cloned());
    }
    totals.files_touched = files.len() as u32;
    totals
}

/// `daily_totals_by_agent` 只保留最近这么多天的数据(2026-08-23 起
/// 7→15,产品要求看更长的趋势窗口)。
const DAILY_CHART_WINDOW_DAYS: usize = 15;

/// epoch 毫秒 → 该毫秒所在的 UTC 日索引(自 1970-01-01 起的第几天)。用于
/// 按天分桶;**不做本地时区换算**——纯 std 没有时区能力,引入 `chrono`/`time`
/// 属于新增依赖(spec 明确不新增)，UTC 分桶对"看近 15 天趋势形状"这个用途
/// 足够，不追求跟用户本地墙上时钟严格对齐。
fn day_index_from_ms(ms: u64) -> i64 {
    (ms / 86_400_000) as i64
}

/// UTC 日索引 → (year, month, day)。Howard Hinnant 的公开 civil_from_days
/// 算法(纯数学换算，不依赖任何日期库)。
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

/// 按天、按 agent 聚合的 token 合计（四项 token 加总，不细分 in/out/
/// cache——见 spec"关键语义确认"）。只保留最近 `DAILY_CHART_WINDOW_DAYS`
/// 天，不足这个天数不补占位空天，按 `day_index` 升序（最旧在前，最新在后，
/// 图表从左到右自然是时间顺序）。
#[derive(Debug, Clone, PartialEq)]
pub struct DayAgentTotals {
    pub day_index: i64,
    pub label: String,
    pub claude: u64,
    pub codebuddy: u64,
    pub opencode: u64,
}

pub fn daily_totals_by_agent(
    rows: &[(ConversationMeta, ConversationUsage)],
) -> Vec<DayAgentTotals> {
    use std::collections::BTreeMap;
    let mut by_day: BTreeMap<i64, (u64, u64, u64)> = BTreeMap::new();
    for (meta, usage) in rows {
        let day = day_index_from_ms(meta.modified_ms);
        let total =
            usage.tokens_in + usage.tokens_out + usage.tokens_cache_read + usage.tokens_cache_write;
        let entry = by_day.entry(day).or_insert((0, 0, 0));
        match meta.agent {
            AgentKind::Claude => entry.0 += total,
            AgentKind::Codebuddy => entry.1 += total,
            AgentKind::Opencode => entry.2 += total,
            AgentKind::Unknown | AgentKind::Codex | AgentKind::Kilo | AgentKind::V8agent => {}
        }
    }
    let mut days: Vec<DayAgentTotals> = by_day
        .into_iter()
        .map(|(day_index, (claude, codebuddy, opencode))| {
            // 年份在"近 15 天"这种短窗口的标签里用不上，解构时直接忽略。
            let (_, m, d) = civil_from_days(day_index);
            DayAgentTotals {
                day_index,
                label: format!("{m:02}/{d:02}"),
                claude,
                codebuddy,
                opencode,
            }
        })
        .collect();
    let start = days.len().saturating_sub(DAILY_CHART_WINDOW_DAYS);
    days.split_off(start)
}

/// 整个项目范围（不限"近 15 天"）按 agent 的 token 总量（四项合计），供
/// 饼图用；只返回项目里实际出现过的 agent，不产生全零占位记录。
pub fn agent_token_share(rows: &[(ConversationMeta, ConversationUsage)]) -> Vec<(AgentKind, u64)> {
    const ORDER: [AgentKind; 3] = [AgentKind::Claude, AgentKind::Codebuddy, AgentKind::Opencode];
    ORDER
        .into_iter()
        .filter_map(|kind| {
            let total: u64 = rows
                .iter()
                .filter(|(meta, _)| meta.agent == kind)
                .map(|(_, u)| {
                    u.tokens_in + u.tokens_out + u.tokens_cache_read + u.tokens_cache_write
                })
                .sum();
            (total > 0).then_some((kind, total))
        })
        .collect()
}

/// 整个项目范围按 agent 的"回合"数合计——口径沿用用量面板的"回合"
/// 即 human 发言数（2026-08-27 调整，见 dozerd `get_usage_summary_in`）。
/// 只返回实际有回合的 agent，不产生全零占位记录。供 Agent 会话统计饼图用。
pub fn agent_turn_share(rows: &[(ConversationMeta, ConversationUsage)]) -> Vec<(AgentKind, u64)> {
    const ORDER: [AgentKind; 3] = [AgentKind::Claude, AgentKind::Codebuddy, AgentKind::Opencode];
    ORDER
        .into_iter()
        .filter_map(|kind| {
            let total: u64 = rows
                .iter()
                .filter(|(meta, _)| meta.agent == kind)
                .map(|(_, u)| u.turns as u64)
                .sum();
            (total > 0).then_some((kind, total))
        })
        .collect()
}

pub fn update(ws_state: &mut WorkspaceState, msg: Message) {
    match msg {
        Message::Loaded(_, rows) => {
            ws_state.rows = rows;
            ws_state.loading = false;
        }
    }
}

/// 异步扫描项目全部 agent transcript 并逐个解析用量。内核在
/// `PanelSelect(PanelKind::Usage)` 分支(切到面板时自动刷新)调用,
/// 经 `Workspace::spawn_usage_refresh` 转发。现有
/// `Workspace::spawn_usage_refresh` 的搬家版本,逻辑不变(读失败的会话
/// 整条跳过、不计入汇总)。
pub fn spawn_refresh(
    project_id: i64,
    project_path: PathBuf,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let cwd = project_path.to_string_lossy().into_owned();
        let rows = client
            .get_usage_summary(&cwd, None)
            .await
            .unwrap_or_default()
            .iter()
            .map(|(summary, payload)| {
                (
                    crate::conversation::ConversationMeta::from_summary(summary),
                    ConversationUsage::from(payload),
                )
            })
            .collect::<Vec<_>>();
        emit(Message::Loaded(project_id, rows));
    });
}

/// 面板主入口，对应右图标栏的"用量"视图（单栏，不像 Conversations
/// 那样是"列表:内容"配对分栏——见 spec）。`rows` 为空且 `loading` 为假时
/// 是"还没数据"的空态；`loading` 为真时是刷新中占位态；两者互斥由 `update`
/// 保证（`WorkspaceState::set_loading` 调用后、`Loaded` 落地时清掉）。
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let rows = ws_state.rows();
    let loading = ws_state.loading();
    // 套用统一 panel head:Lucide `BarChart3` 图标 + 暖金 `#dcc9a3` 的 "用量"
    // 标题 + 1px 分割线。刷新不再走面板内按钮——进入面板时由
    // `Workspace::spawn_usage_refresh` 自动触发(见 `panel_select`)。
    let mut content = column![home_panel_head(icons::IconKind::BarChart3, "用量")]
        .spacing(12)
        .padding(14);

    if loading {
        content = content.push(
            text("统计中…")
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        );
    } else if rows.is_empty() {
        content = content.push(
            text("这个项目还没有 agent 对话记录")
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        );
    } else {
        let usages: Vec<ConversationUsage> = rows.iter().map(|(_, u)| u.clone()).collect();
        content = content.push(home_section_head("项目用量统计"));
        content = content.push(project_summary_boxes(&aggregate(&usages)));

        let token_share = agent_token_share(rows);
        let turn_share = agent_turn_share(rows);
        if !token_share.is_empty() || !turn_share.is_empty() {
            content = content.push(home_section_head("Agent 用量统计"));
            // 左侧是"会话·回合"统计饼图(按 human 发言数),右侧是 token 占比
            // 饼图——各自配一块图例并排(2026-08-27 新增回合饼图)。
            let mut agent_row = iced_widget::row![]
                .spacing(32)
                .align_y(iced_widget::core::Alignment::Center);
            if !turn_share.is_empty() {
                agent_row = agent_row.push(
                    iced_widget::row![pie_chart(&turn_share), chart_label("回合", &turn_share)]
                        .spacing(16)
                        .align_y(iced_widget::core::Alignment::Center),
                );
            }
            if !turn_share.is_empty() && !token_share.is_empty() {
                // 分隔线的高度必须是 `Length::Fixed`,不能是 `Length::Fill`——
                // iced 0.14 的 `Row`/`Column::push` 会把子元素的 `Fill` 沿
                // `Length::enclose` 一路传染给 `agent_row` 再到 `content`,
                // 让整行被撑成"吃满面板剩余高度",饼图/图例固定尺寸不变,
                // `align_y(Center)` 一居中就在上下留出大片空白(2026-08-27
                // 修的真实 bug,面板本该紧凑排布)。高度对齐 `pie_chart` 的
                // 固定尺寸,视觉上跟饼图顶/底对齐。
                agent_row = agent_row.push(
                    container(iced_widget::Space::new())
                        .width(Length::Fixed(1.0))
                        .height(Length::Fixed(PIE_RADIUS * 2.0 + 8.0))
                        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
                            background: Some(byteui::theme::color::current().border.into()),
                            ..iced_widget::container::Style::default()
                        }),
                );
            }
            if !token_share.is_empty() {
                agent_row = agent_row.push(
                    iced_widget::row![pie_chart(&token_share), chart_legend(&token_share)]
                        .spacing(16)
                        .align_y(iced_widget::core::Alignment::Center),
                );
            }
            content = content.push(agent_row);
        }

        let days = daily_totals_by_agent(rows);
        if !days.is_empty() {
            content = content.push(home_section_head("每日用量统计"));
            content = content.push(bar_chart(&days));
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

fn stat(
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
            .font(iced_widget::core::Font::MONOSPACE),
    ]
    .spacing(2)
    .into()
}

/// 一个带边框的统计卡片:一行 `stat` 并排。`project_summary_boxes` 拿它
/// 拼出两张卡(会话/回合/工具调用/触达文件 一张,四个 token 分项另一张)
/// ——参照设计草图,两张卡各自成框、并排放,不是原来的单卡通栏。
fn stat_box(
    stats: Vec<Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer>>,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut row = iced_widget::row![].spacing(24);
    for s in stats {
        row = row.push(s);
    }
    container(row)
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
fn project_summary_boxes(
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
    iced_widget::row![activity_box, token_box]
        .spacing(12)
        .into()
}

const BAR_MAX_HEIGHT: f32 = 72.0;
/// 柱宽(2026-08-23 起 20→14,配合 `DAILY_CHART_WINDOW_DAYS` 7→15——柱数
/// 翻倍,原宽度会把图表撑到 ~466px,窄一点的可调栏宽会被截断看不全)。
const BAR_WIDTH: f32 = 14.0;
/// 柱顶总量数字 + 间距预留的高度,`GridLines`/`bar_chart` 靠它对齐网格线
/// 与柱子的 0 基线(见 `bar_chart` 里 `col` 首个 `container` 的同一个值)。
const BAR_LABEL_GAP: f32 = 14.0;
const GRID_CANVAS_HEIGHT: f32 = BAR_MAX_HEIGHT + BAR_LABEL_GAP;
/// 网格线左侧刻度数字预留的宽度:网格线本身从这条线右边才开始画,避免
/// 刻度数字跟第一根柱子顶部的总量数字重叠。
const GRID_LABEL_GUTTER: f32 = 26.0;
/// 目标网格线条数——实际条数取决于 `nice_tick_step` 算出的整数步长,一般
/// 落在 3~5 条,不保证精确等于这个数。
const GRID_TARGET_TICKS: u32 = 4;

fn bar_segment(
    height: f32,
    color: Color,
    round_top: bool,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let radius = if round_top {
        iced_widget::core::border::Radius {
            top_left: 4.0,
            top_right: 4.0,
            ..iced_widget::core::border::Radius::from(0.0)
        }
    } else {
        iced_widget::core::border::Radius::from(0.0)
    };
    container(iced_widget::Space::new())
        .width(Length::Fixed(BAR_WIDTH))
        .height(Length::Fixed(height.max(1.0)))
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(color.into()),
                border: Border {
                    radius,
                    ..Border::default()
                },
                ..iced_widget::container::Style::default()
            },
        )
        .into()
}

/// 给 `max_value` 算一个"好读"的刻度步长(1/2/5 × 10ⁿ),不是简单
/// `max_value / target_ticks` 等分——那样步长会是像 733 这种没法一眼读的
/// 零头,不像典型图表库(Chart.js/D3 等)那样刻度总落在整数上。经典
/// "nice numbers" 算法:按数量级取 1/2/5/10 里最接近目标步长的一档。
fn nice_tick_step(max_value: u64, target_ticks: u32) -> u64 {
    let raw_step = max_value as f64 / target_ticks.max(1) as f64;
    if raw_step <= 0.0 {
        return 1;
    }
    let magnitude = 10f64.powf(raw_step.log10().floor());
    let residual = raw_step / magnitude;
    let nice_residual = if residual <= 1.0 {
        1.0
    } else if residual <= 2.0 {
        2.0
    } else if residual <= 5.0 {
        5.0
    } else {
        10.0
    };
    ((nice_residual * magnitude).round() as u64).max(1)
}

/// 从一个步长的整数倍往上数,数到 `max_total` 为止的刻度值(不含 0 基线
/// ——柱子本身已经贴基线,不用再画一条线)。
fn grid_ticks(max_total: u64) -> Vec<u64> {
    if max_total == 0 {
        return Vec::new();
    }
    let step = nice_tick_step(max_total, GRID_TARGET_TICKS);
    let mut ticks = Vec::new();
    let mut v = step;
    while v <= max_total {
        ticks.push(v);
        v += step;
    }
    if ticks.is_empty() {
        ticks.push(max_total);
    }
    ticks
}

/// 条形图背景网格线:水平参考线 + 左侧刻度数字,叠在柱子行后面(见
/// `bar_chart` 用 `stack!` 把它跟柱子摞在一起)。画布高度固定为
/// `GRID_CANVAS_HEIGHT`,跟柱子所在的那个 `container`(同高、底对齐)
/// 严格对齐,0 值线落在画布最底部。
struct GridLines {
    ticks: Vec<u64>,
    max_total: u64,
}

impl canvas::Program<Message, iced_widget::Theme, iced_renderer::Renderer> for GridLines {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced_renderer::Renderer,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        _cursor: iced_widget::core::mouse::Cursor,
    ) -> Vec<canvas::Geometry<iced_renderer::Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        if self.max_total == 0 {
            return vec![frame.into_geometry()];
        }
        let line_color = byteui::theme::color::current().border;
        let label_color = byteui::theme::color::current().dim;
        for &tick in &self.ticks {
            let y = GRID_CANVAS_HEIGHT - (tick as f32 / self.max_total as f32) * BAR_MAX_HEIGHT;
            frame.stroke(
                &canvas::Path::line(
                    Point::new(GRID_LABEL_GUTTER, y),
                    Point::new(bounds.width, y),
                ),
                canvas::Stroke::default()
                    .with_color(line_color)
                    .with_width(1.0),
            );
            frame.fill_text(canvas::Text {
                content: format_count(tick),
                position: Point::new(GRID_LABEL_GUTTER - 4.0, y),
                color: label_color,
                size: iced_widget::core::Pixels(7.0),
                font: iced_widget::core::Font::MONOSPACE,
                align_x: iced_widget::core::text::Alignment::Right,
                align_y: iced_widget::core::alignment::Vertical::Center,
                ..canvas::Text::default()
            });
        }
        vec![frame.into_geometry()]
    }
}

fn grid_lines_canvas(
    max_total: u64,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    Canvas::new(GridLines {
        ticks: grid_ticks(max_total),
        max_total,
    })
    .width(Length::Fill)
    .height(Length::Fixed(GRID_CANVAS_HEIGHT))
    .into()
}

/// 悬停某天柱子时弹出的明细气泡:日期 + 各 agent token 数(2026-08-26 起随
/// 全局统一走 `format_count` 的 k/m 缩写)。样式复用
/// `byteui::interaction::icons::tooltip_bubble_style`,跟 icon 按钮 tooltip
/// 同一套视觉。零值 agent 不列(同 `chart_legend` 只列有数据的 agent)。
fn day_tooltip_bubble(
    day: &DayAgentTotals,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut rows = column![
        text(day.label.clone())
            .size(byteui::theme::font::caption_sm())
            .color(byteui::theme::color::current().cream),
    ]
    .spacing(4);
    for (agent, value) in [
        (AgentKind::Claude, day.claude),
        (AgentKind::Codebuddy, day.codebuddy),
        (AgentKind::Opencode, day.opencode),
    ] {
        if value == 0 {
            continue;
        }
        let dot = container(iced_widget::Space::new())
            .width(Length::Fixed(8.0))
            .height(Length::Fixed(8.0))
            .style({
                let color = crate::workspace::agent_dot_color(agent);
                move |_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: Some(color.into()),
                    border: Border {
                        radius: 4.0.into(),
                        ..Border::default()
                    },
                    ..iced_widget::container::Style::default()
                }
            });
        rows = rows.push(
            iced_widget::row![
                dot,
                text(format!("{} {}", agent.label(), format_count(value)))
                    .size(byteui::theme::font::caption_sm())
                    .color(byteui::theme::color::current().dim)
                    .font(iced_widget::core::Font::MONOSPACE),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        );
    }
    container(rows).padding([6, 8]).into()
}

/// 单根 agent 柱:柱身上方叠一个小数字标注(2026-08-27 分组柱状图起,
/// 每个 agent 一根独立柱子、自带数值,不再只在天量汇总那一根上标)。
fn agent_bar(
    value: u64,
    scale: f32,
    color: Color,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    column![
        text(format_count(value))
            .size(8.0)
            .color(byteui::theme::color::current().dim)
            .font(iced_widget::core::Font::MONOSPACE),
        bar_segment(value as f32 * scale, color, true),
    ]
    .spacing(2)
    .align_x(iced_widget::core::alignment::Horizontal::Center)
    .into()
}

fn bar_chart(
    days: &[DayAgentTotals],
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let max_total = days
        .iter()
        .map(|d| d.claude.max(d.codebuddy).max(d.opencode))
        .max()
        .unwrap_or(1)
        .max(1);

    // 每天内部并排出 3 根(Claude/CodeBuddy/OpenCode)各自的柱子,组与组之间
    // 间隔更大,agent 相邻贴得更近,方便"同日横向对比 + 跨日纵向看趋势"
    // (2026-08-27 由"每日一根堆叠柱"改为分组柱状图)。
    let mut groups = iced_widget::row![].spacing(8);
    for d in days {
        let scale = BAR_MAX_HEIGHT / max_total as f32;
        let day_group = iced_widget::row![
            agent_bar(d.claude, scale, byteui::theme::color::current().cyan),
            agent_bar(d.codebuddy, scale, byteui::theme::color::current().purple),
            agent_bar(d.opencode, scale, byteui::theme::color::current().green),
        ]
        .spacing(3)
        .align_y(iced_widget::core::alignment::Vertical::Bottom);

        let col = column![
            container(day_group)
                .height(Length::Fixed(GRID_CANVAS_HEIGHT))
                .align_y(iced_widget::core::alignment::Vertical::Bottom),
            text(d.label.clone())
                .size(8.0)
                .color(byteui::theme::color::current().dim)
                .font(iced_widget::core::Font::MONOSPACE),
        ]
        .spacing(4)
        .align_x(iced_widget::core::alignment::Horizontal::Center);

        let hoverable = Tooltip::new(col, day_tooltip_bubble(d), Position::Top)
            .gap(6)
            .style(icons::tooltip_bubble_style());

        groups = groups.push(hoverable);
    }

    // 网格线画布叠在柱子行后面(`stack!`):柱子行整体右移 `GRID_LABEL_GUTTER`
    // 给左侧刻度数字腾地方,网格线本身(`GridLines::draw`)从这条线右边
    // 才开始画,两者不会互相遮挡。
    stack![
        grid_lines_canvas(max_total),
        container(groups).padding(iced_widget::core::Padding {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: GRID_LABEL_GUTTER,
        }),
    ]
    .width(Length::Fill)
    .into()
}

/// 用量面板数字的统一样式(2026-08-26 起所有数字共用这一套,不再区分图表
/// 标签/明细/汇总):三档自动换算 + 千分号。
///
/// - `< 1000`:原样(千以下不需要数字分隔,如 `999`)。
/// - `≥ 1000` 且 `< 1,000,000`:除以 1000 显示 `k`,1 位小数,如 `123.5k`。
/// - `≥ 1,000,000`:除以 1,000,000 显示 `m`,1 位小数,如 `1.2m`。
///
/// 边界取 `≥`(而非字面的"大于"):`1000` 直接进 `1.0k`、`1,000,000` 直接进
/// `1.0m`,避免算出 `1000.0k` 这种难读的中间档。
fn format_count<N: Into<u64>>(n: N) -> String {
    let n: u64 = n.into();
    if n >= 1_000_000 {
        format!("{:.1}m", n as f32 / 1_000_000.0)
    } else if n >= 1000 {
        format!("{:.1}k", n as f32 / 1000.0)
    } else {
        n.to_string()
    }
}

const PIE_RADIUS: f32 = 52.0;
const PIE_GAP_RAD: f32 = 0.035;

struct PieChart {
    share: Vec<(AgentKind, u64)>,
}

impl canvas::Program<Message, iced_widget::Theme, iced_renderer::Renderer> for PieChart {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced_renderer::Renderer,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        _cursor: iced_widget::core::mouse::Cursor,
    ) -> Vec<canvas::Geometry<iced_renderer::Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let center = frame.center();
        let total: u64 = self.share.iter().map(|(_, v)| v).sum();
        if total == 0 {
            return vec![frame.into_geometry()];
        }
        // 12 点钟方向起(-90°),顺时针累加每片的角度(iced 的 Radians 约定
        // "从正 x 轴顺时针"——见 iced_graphics::geometry::path::arc::Arc 文档)。
        let mut angle = Radians(-std::f32::consts::FRAC_PI_2);
        for (agent, value) in &self.share {
            let sweep = Radians(2.0 * std::f32::consts::PI * (*value as f32 / total as f32));
            let start = Radians(angle.0 + PIE_GAP_RAD / 2.0);
            let end = Radians(angle.0 + sweep.0 - PIE_GAP_RAD / 2.0);
            let path = canvas::Path::new(|b| {
                b.arc(canvas::path::Arc {
                    center,
                    radius: PIE_RADIUS,
                    start_angle: start,
                    end_angle: end,
                });
                b.line_to(center);
                b.close();
            });
            frame.fill(&path, crate::workspace::agent_dot_color(*agent));
            angle = Radians(angle.0 + sweep.0);
        }
        vec![frame.into_geometry()]
    }
}

fn pie_chart(
    share: &[(AgentKind, u64)],
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    Canvas::new(PieChart {
        share: share.to_vec(),
    })
    .width(Length::Fixed(PIE_RADIUS * 2.0 + 8.0))
    .height(Length::Fixed(PIE_RADIUS * 2.0 + 8.0))
    .into()
}

/// 图例列表——2026-08-23 起改竖排(原来是横排 `row`),配合草图把它挪到
/// 饼图右边、并排放而不是叠在下面。末尾固定加一行"全部"(100%)汇总——
/// 2026-08-26 新加点:整张饼的总 token 数,样式用暖金 + 顶部细分隔线,跟
/// 各 agent 行(暗色文字)区分开。
fn chart_legend(
    share: &[(AgentKind, u64)],
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let total: u64 = share.iter().map(|(_, v)| v).sum();
    let cream = byteui::theme::color::current().cream;
    let mut col = column![].spacing(10);
    for (agent, value) in share {
        let pct = value
            .checked_mul(100)
            .and_then(|n| n.checked_div(total))
            .unwrap_or(0);
        let dot = container(iced_widget::Space::new())
            .width(Length::Fixed(8.0))
            .height(Length::Fixed(8.0))
            .style({
                let color = crate::workspace::agent_dot_color(*agent);
                move |_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: Some(color.into()),
                    border: Border {
                        radius: 4.0.into(),
                        ..Border::default()
                    },
                    ..iced_widget::container::Style::default()
                }
            });
        col = col.push(
            iced_widget::row![
                dot,
                text(format!(
                    "{} {}% · {}",
                    agent.label(),
                    pct,
                    format_count(*value)
                ))
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().dim)
                .font(iced_widget::core::Font::MONOSPACE),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        );
    }
    // 全部:总 token 统计,恒 100%。暖金文字 + 实心圆点 + 顶部 1px 分隔线,
    // 让"总览"一眼区别于上面各 agent 的明细行。
    let total_dot = container(iced_widget::Space::new())
        .width(Length::Fixed(8.0))
        .height(Length::Fixed(8.0))
        .style({
            let c = cream;
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(c.into()),
                border: Border {
                    radius: 4.0.into(),
                    ..Border::default()
                },
                ..iced_widget::container::Style::default()
            }
        });
    let total_row = iced_widget::row![
        total_dot,
        text(format!("全部 100% · {}", format_count(total)))
            .size(byteui::theme::font::caption_sm())
            .color(cream)
            .font(iced_widget::core::Font::MONOSPACE),
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);
    // 顶部细分隔线:一条 1px、与面板分隔线同色的横线,把"全部"行和 agent
    // 明细行区隔开(不用 `Border::Side`,避免引入尚无用到的边框类型)。
    let divider = container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..iced_widget::container::Style::default()
        });
    col = col.push(column![divider, total_row].spacing(10));
    col.into()
}

/// 在 `chart_legend` 上方加一行小节标题(如"回合"),用来区分同一区里
/// 并排的多个饼图(2026-08-27 新增回合饼图时:左侧回合、右侧 token,各有
/// 自己的标题避免图例数字混看)。
fn chart_label(
    title: &'static str,
    share: &[(AgentKind, u64)],
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    column![
        text(title)
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().dim),
        chart_legend(share),
    ]
    .spacing(6)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_tick_step_rounds_to_1_2_5_family() {
        // 7 / 4 = 1.75 → 落在 (1,2] 档,取 2。
        assert_eq!(nice_tick_step(7, 4), 2);
        // 42 / 4 = 10.5 → 数量级 10,余数 1.05 → (1,2] 档,取 2*10=20。
        assert_eq!(nice_tick_step(42, 4), 20);
        // 1 / 4 = 0.25 → 数量级 0.1,余数 2.5 → (2,5] 档,取 5*0.1 四舍五入为 1。
        assert_eq!(nice_tick_step(1, 4), 1);
    }

    #[test]
    fn grid_ticks_stops_at_max_and_never_empty() {
        assert_eq!(grid_ticks(0), Vec::<u64>::new());
        // step=2(见上一条用例),数到 <=7 为止:2,4,6。
        assert_eq!(grid_ticks(7), vec![2, 4, 6]);
        // 数据量很小时(max_total < step)也至少给一条线兜底。
        assert_eq!(grid_ticks(1), vec![1]);
    }

    #[test]
    fn format_count_three_tier_k_m_rounding() {
        // 千以下原样。
        assert_eq!(format_count(0_u64), "0");
        assert_eq!(format_count(999_u64), "999");
        // ≥1000 且 <1,000,000 → k,1 位小数。`1000` 直接进 `1.0k`,
        // 不出现 `1000.0k` 的中间档(见函数注释)。
        assert_eq!(format_count(1000_u64), "1.0k");
        assert_eq!(format_count(1234_u64), "1.2k");
        assert_eq!(format_count(123_456_u64), "123.5k");
        assert_eq!(format_count(999_999_u64), "1000.0k");
        // ≥1,000,000 → m,1 位小数。`1,000,000` 直接进 `1.0m`。
        assert_eq!(format_count(1_000_000_u64), "1.0m");
        assert_eq!(format_count(1_234_567_u64), "1.2m");
    }

    fn sample_usage(files: &[&str]) -> ConversationUsage {
        ConversationUsage {
            turns: 2,
            tool_calls: 3,
            mutating_tool_calls: 1,
            files_touched: files.iter().map(|s| s.to_string()).collect(),
            tokens_in: 10,
            tokens_out: 2,
            tokens_cache_read: 1,
            tokens_cache_write: 1,
        }
    }

    #[test]
    fn conversation_usage_from_payload_maps_all_fields() {
        let payload = dozer_core::protocol::UsagePayload {
            turns: 2,
            tool_calls: 1,
            mutating_tool_calls: 1,
            files_touched: std::collections::BTreeSet::from(["README.md".to_string()]),
            tokens_in: 10,
            tokens_out: 20,
            tokens_cache_read: 1,
            tokens_cache_write: 2,
        };
        let usage = ConversationUsage::from(&payload);
        assert_eq!(usage.turns, 2);
        assert_eq!(usage.tool_calls, 1);
        assert_eq!(usage.mutating_tool_calls, 1);
        assert_eq!(
            usage.files_touched,
            std::collections::BTreeSet::from(["README.md".to_string()])
        );
        assert_eq!(usage.tokens_in, 10);
        assert_eq!(usage.tokens_out, 20);
        assert_eq!(usage.tokens_cache_read, 1);
        assert_eq!(usage.tokens_cache_write, 2);
    }

    #[test]
    fn aggregate_sums_fields_and_dedups_files_across_conversations() {
        let rows = [
            sample_usage(&["/a.rs", "/b.rs"]),
            sample_usage(&["/a.rs", "/c.rs"]),
        ];
        let totals = aggregate(&rows);
        assert_eq!(totals.conversation_count, 2);
        assert_eq!(totals.turns, 4);
        assert_eq!(totals.tool_calls, 6);
        assert_eq!(totals.mutating_tool_calls, 2);
        assert_eq!(totals.files_touched, 3, "/a.rs 在两个会话里都出现,只算一次");
        assert_eq!(totals.tokens_in, 20);
        assert_eq!(totals.tokens_out, 4);
        assert_eq!(totals.tokens_cache_read, 2);
        assert_eq!(totals.tokens_cache_write, 2);
    }

    #[test]
    fn aggregate_empty_slice_is_all_zero() {
        assert_eq!(aggregate(&[]), ProjectUsageTotals::default());
    }

    fn meta(agent: AgentKind, title: &str) -> ConversationMeta {
        ConversationMeta {
            path: std::path::PathBuf::from(format!("/{title}.jsonl")),
            title: title.to_string(),
            modified_ms: 0,
            agent,
        }
    }

    #[test]
    fn civil_from_days_known_epoch_dates() {
        // 1970-01-01 是 epoch day 0。
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2026-08-07 手工核对(用 `date -u -j -f "%Y-%m-%d" 2026-08-07 +%s`
        // 算出 epoch 秒 1786071805 再除以 86400 取整得到 day_index=20672)。
        assert_eq!(civil_from_days(20672), (2026, 8, 7));
    }

    fn meta_at(agent: AgentKind, ms: u64) -> ConversationMeta {
        ConversationMeta {
            path: std::path::PathBuf::from(format!("/{ms}.jsonl")),
            title: String::new(),
            modified_ms: ms,
            agent,
        }
    }

    fn usage_with_tokens(input: u64) -> ConversationUsage {
        ConversationUsage {
            tokens_in: input,
            ..Default::default()
        }
    }

    #[test]
    fn daily_totals_by_agent_buckets_by_day_and_sums_per_agent() {
        // day 20672 = 2026-08-07 00:00:00 UTC 起的毫秒;+3600_000 还在同一天。
        let day0_ms = 20_672u64 * 86_400_000;
        let rows = vec![
            (meta_at(AgentKind::Claude, day0_ms), usage_with_tokens(10)),
            (
                meta_at(AgentKind::Claude, day0_ms + 3_600_000),
                usage_with_tokens(5),
            ),
            (
                meta_at(AgentKind::Codebuddy, day0_ms + 1000),
                usage_with_tokens(2),
            ),
            (
                meta_at(AgentKind::Opencode, day0_ms + 86_400_000),
                usage_with_tokens(7),
            ), // 次日
        ];
        let days = daily_totals_by_agent(&rows);
        assert_eq!(days.len(), 2, "只返回实际有数据的两天,不补空天占位");
        assert_eq!(days[0].day_index, 20_672);
        assert_eq!(days[0].claude, 15, "同一天两条 Claude 会话的 token 要累加");
        assert_eq!(days[0].codebuddy, 2);
        assert_eq!(days[0].opencode, 0);
        assert_eq!(days[1].day_index, 20_673);
        assert_eq!(days[1].opencode, 7);
        assert_eq!(days[0].label, "08/07");
    }

    #[test]
    fn daily_totals_ignores_agents_without_dedicated_bucket() {
        // Codex/Kilo 目前没有专属的 DayAgentTotals 字段（这两家的
        // 用量还进不了统计，见计划 Global Constraints），跟 Unknown 一样
        // 被忽略，不能 panic。
        let rows = vec![(meta_at(AgentKind::Codex, 0), usage_with_tokens(99))];
        let days = daily_totals_by_agent(&rows);
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].claude + days[0].codebuddy + days[0].opencode, 0);
    }

    #[test]
    fn daily_totals_by_agent_keeps_only_most_recent_15_days() {
        let rows: Vec<_> = (0..20)
            .map(|i| {
                (
                    meta_at(AgentKind::Claude, (20_668 + i) as u64 * 86_400_000),
                    usage_with_tokens(1),
                )
            })
            .collect();
        let days = daily_totals_by_agent(&rows);
        assert_eq!(days.len(), 15, "超过 15 天的历史只保留最近 15 天");
        assert_eq!(
            days.last().unwrap().day_index,
            20_687,
            "最后一天是最新的那天"
        );
        assert_eq!(days.first().unwrap().day_index, 20_673);
    }

    #[test]
    fn agent_token_share_sums_four_token_fields_per_agent_and_skips_absent_agents() {
        let rows = vec![
            (meta(AgentKind::Claude, "a"), usage_with_tokens(10)),
            (meta(AgentKind::Claude, "b"), usage_with_tokens(5)),
            (meta(AgentKind::Codebuddy, "c"), usage_with_tokens(3)),
        ];
        let share = agent_token_share(&rows);
        assert_eq!(
            share,
            vec![(AgentKind::Claude, 15), (AgentKind::Codebuddy, 3)]
        );
    }

    fn usage_with_turns(turns: u32) -> ConversationUsage {
        ConversationUsage {
            turns,
            ..Default::default()
        }
    }

    #[test]
    fn agent_turn_share_sums_human_turns_per_agent_and_skips_empty() {
        let rows = vec![
            (meta(AgentKind::Claude, "a"), usage_with_turns(3)),
            (meta(AgentKind::Claude, "b"), usage_with_turns(2)),
            (meta(AgentKind::Codebuddy, "c"), usage_with_turns(5)),
            // Opencode 有会话但零回合——不产生占位,也不进图例。
            (meta(AgentKind::Opencode, "d"), usage_with_turns(0)),
        ];
        let share = agent_turn_share(&rows);
        assert_eq!(
            share,
            vec![(AgentKind::Claude, 5), (AgentKind::Codebuddy, 5)]
        );
    }

    #[test]
    fn loaded_clears_loading_and_stores_rows() {
        let mut ws_state = WorkspaceState {
            loading: true,
            ..WorkspaceState::default()
        };
        let rows = vec![(
            meta(AgentKind::Claude, "a"),
            ConversationUsage {
                turns: 3,
                ..Default::default()
            },
        )];
        update(&mut ws_state, Message::Loaded(1, rows.clone()));
        assert!(!ws_state.loading());
        assert_eq!(ws_state.rows(), rows.as_slice());
    }
}
