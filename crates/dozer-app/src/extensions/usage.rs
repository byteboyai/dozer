// crates/dozer-app/src/usage.rs
//! Agent 用量统计面板的数据层与视图层（spec
//! docs/superpowers/specs/2026-08-07-agent-usage-panel-design.md）。数据层是
//! 纯函数,不碰 iced/IO,镜像 `transcript.rs` 的按 agent 分派解析方式;
//! `dozerd`/`dozer-core::protocol` 完全不参与——所有数据直接读磁盘上的
//! agent transcript JSONL。

use crate::conversation::ConversationMeta;
use crate::icons;
use crate::theme;
use dozer_core::protocol::AgentKind;
use iced_widget::canvas::{self, Canvas};
use iced_widget::core::{Border, Color, Element, Length, Radians, Rectangle};
use iced_widget::{button, column, container, text};
use serde_json::Value;
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

    /// 供内核 `RightIconSelect(RightView::Usage)` 分支调用——切到面板时
    /// 立即标记"统计中",不等 `spawn_refresh` 的异步结果落地才置真。
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }
}

/// 对应现在顶层 `Message` 里的 `UsageRefresh`/`UsageLoaded` 两个变体,去
/// 前缀原样搬来。
#[derive(Debug, Clone)]
pub enum Message {
    Refresh,
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

/// 改动类工具——命中这些名字才计入 `mutating_tool_calls`/`files_touched`。
const MUTATING_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

/// 按 agent 分派解析,Claude/OpenCode/Kilo 共用一套 schema、CodeBuddy 独立
/// 一套,与 `transcript.rs::parse_transcript` 同一分派方式；`Codex`/`Qoder`
/// 暂时返回默认值(全零),理由同 `parse_transcript`。单行解析失败/字段
/// 缺失一律跳过该行/记 0,不 panic、不中断整份文件的解析。
pub fn parse_usage(agent: AgentKind, jsonl: &str) -> ConversationUsage {
    match agent {
        AgentKind::Claude | AgentKind::Opencode | AgentKind::Kilo | AgentKind::Unknown => {
            parse_claude_shaped_usage(jsonl)
        }
        AgentKind::Codebuddy => parse_codebuddy_shaped_usage(jsonl),
        AgentKind::Codex | AgentKind::Qoder => ConversationUsage::default(),
    }
}

fn parse_claude_shaped_usage(jsonl: &str) -> ConversationUsage {
    let mut u = ConversationUsage::default();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => u.turns += 1,
            Some("assistant") => {
                u.turns += 1;
                if let Some(usage) = v.get("message").and_then(|m| m.get("usage")) {
                    u.tokens_in += usage
                        .get("input_tokens")
                        .and_then(|n| n.as_u64())
                        .unwrap_or(0);
                    u.tokens_out += usage
                        .get("output_tokens")
                        .and_then(|n| n.as_u64())
                        .unwrap_or(0);
                    u.tokens_cache_read += usage
                        .get("cache_read_input_tokens")
                        .and_then(|n| n.as_u64())
                        .unwrap_or(0);
                    u.tokens_cache_write += usage
                        .get("cache_creation_input_tokens")
                        .and_then(|n| n.as_u64())
                        .unwrap_or(0);
                }
                let Some(blocks) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                else {
                    continue;
                };
                for b in blocks {
                    if b.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                        continue;
                    }
                    u.tool_calls += 1;
                    let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    if MUTATING_TOOLS.contains(&name) {
                        u.mutating_tool_calls += 1;
                        if let Some(path) = b
                            .get("input")
                            .and_then(|i| i.get("file_path"))
                            .and_then(|p| p.as_str())
                        {
                            u.files_touched.insert(path.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    u
}

fn parse_codebuddy_shaped_usage(jsonl: &str) -> ConversationUsage {
    let mut u = ConversationUsage::default();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }
        if v.get("role").and_then(|r| r.as_str()).is_some() {
            u.turns += 1;
        }
        if let Some(usage) = v.get("providerData").and_then(|p| p.get("usage")) {
            u.tokens_in += usage
                .get("inputTokens")
                .and_then(|n| n.as_u64())
                .unwrap_or(0);
            u.tokens_out += usage
                .get("outputTokens")
                .and_then(|n| n.as_u64())
                .unwrap_or(0);
        }
        // tool_calls/mutating_tool_calls/files_touched 恒为 0/空——CodeBuddy
        // 的 fixture 样本里没见过 tool_use 形状的消息,不臆测其结构
        // （spec"非目标"一节）。
    }
    u
}

/// 多个会话的 `ConversationUsage` 加总成项目级汇总。
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

/// 按 `AgentKind` 把会话分组，固定顺序 Claude → Codebuddy → Opencode →
/// Unknown，只返回非空分组；组内保持传入顺序。返回下标而非引用，语义同
/// `workspace.rs::group_tabs_by_agent`——渲染时既要下标回查
/// `rows[idx]` 取展示字段，直接存下标比存 `&(ConversationMeta, ConversationUsage)`
/// 省一次生命周期纠缠。
pub fn group_usage_by_agent(
    rows: &[(ConversationMeta, ConversationUsage)],
) -> Vec<(AgentKind, Vec<usize>)> {
    const ORDER: [AgentKind; 4] = [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::Unknown,
    ];
    ORDER
        .into_iter()
        .filter_map(|kind| {
            let idxs: Vec<usize> = rows
                .iter()
                .enumerate()
                .filter(|(_, (meta, _))| meta.agent == kind)
                .map(|(i, _)| i)
                .collect();
            (!idxs.is_empty()).then_some((kind, idxs))
        })
        .collect()
}

/// epoch 毫秒 → 该毫秒所在的 UTC 日索引(自 1970-01-01 起的第几天)。用于
/// 按天分桶;**不做本地时区换算**——纯 std 没有时区能力,引入 `chrono`/`time`
/// 属于新增依赖(spec 明确不新增)，UTC 分桶对"看近 7 天趋势形状"这个用途
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
/// cache——见 spec"关键语义确认"）。只保留最近 7 天，不足 7 天不补占位
/// 空天，按 `day_index` 升序（最旧在前，最新在后，图表从左到右自然是时间
/// 顺序）。
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
            AgentKind::Unknown | AgentKind::Codex | AgentKind::Qoder | AgentKind::Kilo => {}
        }
    }
    let mut days: Vec<DayAgentTotals> = by_day
        .into_iter()
        .map(|(day_index, (claude, codebuddy, opencode))| {
            // 年份在"近 7 天"这种短窗口的标签里用不上，解构时直接忽略。
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
    let start = days.len().saturating_sub(7);
    days.split_off(start)
}

/// 整个项目范围（不限"近 7 天"）按 agent 的 token 总量（四项合计），供
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

pub fn update(
    ws_state: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    project_path: PathBuf,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::Refresh => {
            ws_state.loading = true;
            spawn_refresh(project_id, project_path, handle, emit);
        }
        Message::Loaded(_, rows) => {
            ws_state.rows = rows;
            ws_state.loading = false;
        }
    }
}

/// 异步扫描项目全部 agent transcript 并逐个解析用量。内核在
/// `RightIconSelect(RightView::Usage)` 分支(切到面板首次刷新)与
/// `update` 处理 `Refresh`(手动点刷新按钮)两处调用。现有
/// `Workspace::spawn_usage_refresh` 的搬家版本,逻辑不变(读失败的会话
/// 整条跳过、不计入汇总)。
pub fn spawn_refresh(
    project_id: i64,
    project_path: PathBuf,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    handle.spawn(async move {
        let rows = tokio::task::spawn_blocking(move || {
            crate::conversation::list_all_conversations(&project_path)
                .into_iter()
                .filter_map(|meta| {
                    let jsonl = std::fs::read_to_string(&meta.path).ok()?;
                    let u = parse_usage(meta.agent, &jsonl);
                    Some((meta, u))
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        emit(Message::Loaded(project_id, rows));
    });
}

/// 头部：标题 + 项目名 + 右侧手动刷新按钮（spec"面板渲染"#1）。
fn panel_header(
    project_name: &str,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    iced_widget::row![
        column![
            text("用量统计")
                .size(theme::font::subtitle())
                .color(theme::color::CREAM),
            text(project_name)
                .size(theme::font::label())
                .color(theme::color::DIM),
        ]
        .spacing(2),
        iced_widget::Space::new().width(Length::Fill),
        button(icons::view::<Message>(
            icons::IconKind::RefreshCw,
            14.0,
            theme::color::DIM
        ))
        .on_press(Message::Refresh)
        .style(|_t, _s| button::Style::default()),
    ]
    .align_y(iced_widget::core::Alignment::Center)
    .into()
}

/// 面板主入口，对应右图标栏的"用量统计"视图（单栏，不像 Conversations
/// 那样是"列表:内容"配对分栏——见 spec）。`rows` 为空且 `loading` 为假时
/// 是"还没数据"的空态；`loading` 为真时是刷新中占位态；两者互斥由 `update`
/// 保证（`WorkspaceState::set_loading` 调用后、`Loaded` 落地时清掉）。
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    project_name: &'a str,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let rows = ws_state.rows();
    let loading = ws_state.loading();
    let mut content = column![panel_header(project_name)].spacing(12).padding(14);

    if loading {
        content = content.push(
            text("统计中…")
                .size(theme::font::body())
                .color(theme::color::DIM),
        );
    } else if rows.is_empty() {
        content = content.push(
            text("这个项目还没有 agent 对话记录")
                .size(theme::font::body())
                .color(theme::color::DIM),
        );
    } else {
        let usages: Vec<ConversationUsage> = rows.iter().map(|(_, u)| u.clone()).collect();
        content = content.push(summary_card(&aggregate(&usages)));
        let days = daily_totals_by_agent(rows);
        if !days.is_empty() {
            content = content.push(bar_chart(&days));
        }
        let share = agent_token_share(rows);
        if !share.is_empty() {
            content = content.push(column![chart_legend(&share), pie_chart(&share),].spacing(10));
        }
        content = content.push(grouped_list(rows));
    }

    container(content)
        .width(width)
        .height(Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(theme::color::PANEL.into()),
                border: outer,
                ..iced_widget::container::Style::default()
            },
        )
        .into()
}

fn summary_card(
    totals: &ProjectUsageTotals,
) -> Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> {
    fn stat(
        label: &'static str,
        value: String,
        color: Color,
    ) -> Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> {
        column![
            text(label)
                .size(theme::font::caption())
                .color(theme::color::DIM),
            text(value)
                .size(15.0)
                .color(color)
                .font(iced_widget::core::Font::MONOSPACE),
        ]
        .spacing(2)
        .into()
    }

    let row = iced_widget::row![
        stat("轮次", totals.turns.to_string(), theme::color::CREAM),
        stat(
            "工具调用(改动)",
            format!("{} ({})", totals.tool_calls, totals.mutating_tool_calls),
            theme::color::CREAM
        ),
        stat(
            "触达文件",
            totals.files_touched.to_string(),
            theme::color::CREAM
        ),
        stat("input", totals.tokens_in.to_string(), theme::color::CYAN),
        stat("output", totals.tokens_out.to_string(), theme::color::CYAN),
        stat(
            "cache 读",
            totals.tokens_cache_read.to_string(),
            theme::color::CYAN
        ),
        stat(
            "cache 写",
            totals.tokens_cache_write.to_string(),
            theme::color::CYAN
        ),
    ]
    .spacing(24);

    container(
        column![
            text(format!("项目汇总 · {} 会话", totals.conversation_count))
                .size(theme::font::caption())
                .color(theme::color::DIM),
            row,
        ]
        .spacing(10),
    )
    .padding(12)
    .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
        background: Some(theme::color::CARD.into()),
        border: Border {
            radius: 10.0.into(),
            ..Border::default()
        },
        ..iced_widget::container::Style::default()
    })
    .into()
}

fn usage_row<'a>(
    meta: &'a ConversationMeta,
    u: &'a ConversationUsage,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let activity = format!(
        "{} 轮 · {} 次工具({} 改动) · {} 文件",
        u.turns,
        u.tool_calls,
        u.mutating_tool_calls,
        u.files_touched.len()
    );
    let tokens = format!(
        "in {} · out {} · cache读 {} · cache写 {}",
        u.tokens_in, u.tokens_out, u.tokens_cache_read, u.tokens_cache_write
    );
    container(
        column![
            text(meta.title.clone())
                .size(theme::font::body())
                .color(theme::color::CREAM),
            text(activity)
                .size(theme::font::caption_sm())
                .color(theme::color::DIM)
                .font(iced_widget::core::Font::MONOSPACE),
            text(tokens)
                .size(theme::font::caption_sm())
                .color(theme::color::CYAN)
                .font(iced_widget::core::Font::MONOSPACE),
        ]
        .spacing(4),
    )
    .width(Length::Fill)
    .padding(10)
    .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
        background: Some(theme::color::CARD.into()),
        border: Border {
            radius: 10.0.into(),
            ..Border::default()
        },
        ..iced_widget::container::Style::default()
    })
    .into()
}

fn grouped_list<'a>(
    rows: &'a [(ConversationMeta, ConversationUsage)],
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let groups = group_usage_by_agent(rows);
    let mut col = column![].spacing(12);
    for (agent, idxs) in groups {
        let group_tokens: u64 = idxs
            .iter()
            .map(|&i| {
                let u = &rows[i].1;
                u.tokens_in + u.tokens_out + u.tokens_cache_read + u.tokens_cache_write
            })
            .sum();
        col = col.push(
            iced_widget::row![
                text(agent.label())
                    .size(theme::font::caption())
                    .color(crate::workspace::agent_dot_color(agent)),
                text(format!("{} 会话 · {} tokens", idxs.len(), group_tokens))
                    .size(theme::font::caption())
                    .color(theme::color::DIM),
            ]
            .spacing(8),
        );
        for &i in &idxs {
            let (meta, u) = &rows[i];
            col = col.push(usage_row(meta, u));
        }
    }
    col.into()
}

const BAR_MAX_HEIGHT: f32 = 72.0;
const BAR_WIDTH: f32 = 20.0;

fn bar_segment(
    height: f32,
    color: Color,
    round_top: bool,
) -> Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> {
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

fn bar_chart(
    days: &[DayAgentTotals],
) -> Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> {
    let max_total = days
        .iter()
        .map(|d| d.claude + d.codebuddy + d.opencode)
        .max()
        .unwrap_or(1)
        .max(1);

    let mut bars = iced_widget::row![].spacing(10);
    for d in days {
        let total = d.claude + d.codebuddy + d.opencode;
        let scale = BAR_MAX_HEIGHT / max_total as f32;
        // 自底向上固定顺序:Claude 贴基线(直角)→ CodeBuddy → OpenCode 顶部(圆角)。
        let stack = column![
            bar_segment(d.opencode as f32 * scale, theme::color::GREEN, true),
            bar_segment(d.codebuddy as f32 * scale, theme::color::PURPLE, false),
            bar_segment(d.claude as f32 * scale, theme::color::CYAN, false),
        ]
        .spacing(2);

        let col = column![
            container(
                column![
                    text(format_token_short(total))
                        .size(8.0)
                        .color(theme::color::DIM)
                        .font(iced_widget::core::Font::MONOSPACE),
                    stack,
                ]
                .spacing(2)
                .align_x(iced_widget::core::alignment::Horizontal::Center),
            )
            .height(Length::Fixed(BAR_MAX_HEIGHT + 14.0))
            .align_y(iced_widget::core::alignment::Vertical::Bottom),
            text(d.label.clone())
                .size(8.0)
                .color(theme::color::DIM)
                .font(iced_widget::core::Font::MONOSPACE),
        ]
        .spacing(4)
        .align_x(iced_widget::core::alignment::Horizontal::Center);

        bars = bars.push(col);
    }
    bars.into()
}

/// 紧凑数字标签(1234 → "1.2k"，小于 1000 原样显示)，只用于条形图顶部的
/// 总量标注，跟汇总条/明细行的完整数字(不做单位换算)是两回事——图表标签
/// 空间小，明细数字要精确,两者刻意不共用格式化函数。
fn format_token_short(n: u64) -> String {
    if n >= 1000 {
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

impl canvas::Program<Message, iced_widget::Theme, iced_widget::Renderer> for PieChart {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced_widget::Renderer,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        _cursor: iced_widget::core::mouse::Cursor,
    ) -> Vec<canvas::Geometry<iced_widget::Renderer>> {
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
) -> Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> {
    Canvas::new(PieChart {
        share: share.to_vec(),
    })
    .width(Length::Fixed(PIE_RADIUS * 2.0 + 8.0))
    .height(Length::Fixed(PIE_RADIUS * 2.0 + 8.0))
    .into()
}

fn chart_legend(
    share: &[(AgentKind, u64)],
) -> Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> {
    let total: u64 = share.iter().map(|(_, v)| v).sum();
    let mut row = iced_widget::row![].spacing(18);
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
        row = row.push(
            iced_widget::row![
                dot,
                text(format!(
                    "{} {}% · {}",
                    agent.label(),
                    pct,
                    format_token_short(*value)
                ))
                .size(theme::font::caption_sm())
                .color(theme::color::DIM)
                .font(iced_widget::core::Font::MONOSPACE),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        );
    }
    row.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_claude_shaped_counts_turns_and_tokens() {
        let jsonl = concat!(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"改一下\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"好\"}],",
            "\"usage\":{\"input_tokens\":100,\"output_tokens\":20,",
            "\"cache_read_input_tokens\":5,\"cache_creation_input_tokens\":3}}}\n",
        );
        let u = parse_usage(AgentKind::Claude, jsonl);
        assert_eq!(u.turns, 2);
        assert_eq!(u.tokens_in, 100);
        assert_eq!(u.tokens_out, 20);
        assert_eq!(u.tokens_cache_read, 5);
        assert_eq!(u.tokens_cache_write, 3);
        assert_eq!(u.tool_calls, 0);
    }

    #[test]
    fn parse_claude_shaped_counts_tool_calls_and_mutating_files() {
        let jsonl = concat!(
            "{\"type\":\"assistant\",\"message\":{\"content\":[",
            "{\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{\"file_path\":\"/a.rs\"}},",
            "{\"type\":\"tool_use\",\"name\":\"Edit\",\"input\":{\"file_path\":\"/a.rs\"}},",
            "{\"type\":\"tool_use\",\"name\":\"Write\",\"input\":{\"file_path\":\"/b.rs\"}}",
            "],\"usage\":{}}}\n",
        );
        let u = parse_usage(AgentKind::Claude, jsonl);
        assert_eq!(u.tool_calls, 3, "Read/Edit/Write 全部计入 tool_calls");
        assert_eq!(u.mutating_tool_calls, 2, "只有 Edit/Write 是改动类");
        assert_eq!(
            u.files_touched,
            BTreeSet::from(["/a.rs".to_string(), "/b.rs".to_string()])
        );
    }

    #[test]
    fn parse_usage_skips_malformed_lines_without_panicking() {
        let jsonl = "not json\n{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n";
        let u = parse_usage(AgentKind::Claude, jsonl);
        assert_eq!(u.turns, 1, "坏行跳过,好行照常计入");
    }

    #[test]
    fn parse_usage_empty_file_is_all_zero() {
        assert_eq!(
            parse_usage(AgentKind::Claude, ""),
            ConversationUsage::default()
        );
    }

    #[test]
    fn parse_usage_opencode_reuses_claude_shape() {
        let jsonl = "{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n";
        assert_eq!(parse_usage(AgentKind::Opencode, jsonl).turns, 1);
    }

    #[test]
    fn kilo_usage_reuses_claude_shaped_parser() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"hi"}}"#;
        assert_eq!(parse_usage(AgentKind::Kilo, jsonl).turns, 1);
    }

    #[test]
    fn codex_and_qoder_usage_is_default_until_schema_confirmed() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"hi"}}"#;
        assert_eq!(
            parse_usage(AgentKind::Codex, jsonl),
            ConversationUsage::default()
        );
        assert_eq!(
            parse_usage(AgentKind::Qoder, jsonl),
            ConversationUsage::default()
        );
    }

    #[test]
    fn parse_codebuddy_shaped_reads_provider_usage_and_zero_tool_calls() {
        let jsonl = include_str!("../../../dozer-hook/fixtures/codebuddy-transcript-sample.jsonl");
        let u = parse_usage(AgentKind::Codebuddy, jsonl);
        assert_eq!(u.tokens_in, 22563);
        assert_eq!(u.tokens_out, 3);
        assert_eq!(u.turns, 2, "一条 user + 一条 assistant");
        assert_eq!(u.tool_calls, 0, "CodeBuddy 工具调用形状未观测到,恒为 0");
        assert!(u.files_touched.is_empty());
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
            size_bytes: 0,
            agent,
        }
    }

    #[test]
    fn group_usage_by_agent_orders_claude_codebuddy_opencode_and_skips_empty_groups() {
        let rows = vec![
            (
                meta(AgentKind::Codebuddy, "b"),
                ConversationUsage::default(),
            ),
            (meta(AgentKind::Claude, "a"), ConversationUsage::default()),
        ];
        let groups = group_usage_by_agent(&rows);
        assert_eq!(groups.len(), 2, "没有 OpenCode 数据,不留空分组");
        assert_eq!(
            groups[0].0,
            AgentKind::Claude,
            "固定顺序:Claude 先于 CodeBuddy"
        );
        assert_eq!(groups[0].1, vec![1]);
        assert_eq!(groups[1].0, AgentKind::Codebuddy);
        assert_eq!(groups[1].1, vec![0]);
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
            size_bytes: 0,
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
        // Codex/Qoder/Kilo 目前没有专属的 DayAgentTotals 字段（这三家的
        // 用量还进不了统计，见计划 Global Constraints），跟 Unknown 一样
        // 被忽略，不能 panic。
        let rows = vec![(meta_at(AgentKind::Codex, 0), usage_with_tokens(99))];
        let days = daily_totals_by_agent(&rows);
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].claude + days[0].codebuddy + days[0].opencode, 0);
    }

    #[test]
    fn daily_totals_by_agent_keeps_only_most_recent_7_days() {
        let rows: Vec<_> = (0..10)
            .map(|i| {
                (
                    meta_at(AgentKind::Claude, (20_668 + i) as u64 * 86_400_000),
                    usage_with_tokens(1),
                )
            })
            .collect();
        let days = daily_totals_by_agent(&rows);
        assert_eq!(days.len(), 7, "超过 7 天的历史只保留最近 7 天");
        assert_eq!(
            days.last().unwrap().day_index,
            20_677,
            "最后一天是最新的那天"
        );
        assert_eq!(days.first().unwrap().day_index, 20_671);
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

    #[tokio::test]
    async fn refresh_sets_loading_true() {
        let mut ws_state = WorkspaceState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            Message::Refresh,
            1,
            std::path::PathBuf::from("/tmp/does-not-matter"),
            &handle,
            |_| {},
        );
        assert!(ws_state.loading());
    }

    #[tokio::test]
    async fn loaded_clears_loading_and_stores_rows() {
        let mut ws_state = WorkspaceState {
            loading: true,
            ..WorkspaceState::default()
        };
        let handle = tokio::runtime::Handle::current();
        let rows = vec![(
            meta(AgentKind::Claude, "a"),
            ConversationUsage {
                turns: 3,
                ..Default::default()
            },
        )];
        update(
            &mut ws_state,
            Message::Loaded(1, rows.clone()),
            1,
            std::path::PathBuf::from("/tmp/does-not-matter"),
            &handle,
            |_| {},
        );
        assert!(!ws_state.loading());
        assert_eq!(ws_state.rows(), rows.as_slice());
    }
}
