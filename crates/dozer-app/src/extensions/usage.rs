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
use iced_widget::{button, column, container, row, stack, text};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// 挂在每个 Workspace 上的 Usage 面板状态,对应现有 `Workspace` 上
/// `usage`/`usage_loading` 两个字段。
#[derive(Default)]
pub struct WorkspaceState {
    rows: Vec<(ConversationMeta, ConversationUsage)>,
    loading: bool,
    /// 右侧 agent 筛选栏当前选中项:`None` = "全部agent"(默认,不过滤)。
    agent_filter: Option<AgentKind>,
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

    /// 有没有数据可展示 agent 筛选栏(`list_pane`)——加载中/还没数据时
    /// 不拼两栏,内容侧独占全宽,同改造前 `sidebar: Option<..>` 为 `None`
    /// 时的行为(见 app.rs `PanelKind::Usage` 分支)。
    pub fn has_agent_filter(&self) -> bool {
        !self.loading && !self.rows.is_empty()
    }
}

/// 对应现在顶层 `Message` 里的 `UsageLoaded` 变体,去前缀原样搬来。
/// `Refresh`/`Hover` 随手动刷新按钮一起移除——进入面板时由
/// `Workspace::spawn_usage_refresh` 自动刷新,不再需要面板内按钮。
#[derive(Debug, Clone)]
pub enum Message {
    Loaded(i64, Vec<(ConversationMeta, ConversationUsage)>),
    /// 右侧 agent 筛选栏点击(2026-08-28):`None` 选"全部agent"。
    AgentFilterSet(Option<AgentKind>),
    /// 内容侧"收起/展开列表列"按钮:内核拦截,不进 `update`——转发成顶层
    /// `Message::TogglePanelListCollapse(PanelKind::Usage)`(见 app.rs)。
    ToggleListCollapse,
    /// 任意顶部 `HoverId` 的悬停进入/离开(收起按钮等)。内核拦截转发给
    /// 顶层 `App::set_hover`,本面板 `update` 保 no-op 分支维持 match 穷尽。
    Hover(crate::app::HoverId, bool),
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

/// `daily_totals_by_agent` 只保留最近这么多天的数据(2026-08-28 起
/// 15→7,产品改回看近一周的紧凑窗口)。
const DAILY_CHART_WINDOW_DAYS: usize = 7;

/// epoch 毫秒 → 该毫秒所在的 UTC 日索引(自 1970-01-01 起的第几天)。用于
/// 按天分桶;**不做本地时区换算**——纯 std 没有时区能力,引入 `chrono`/`time`
/// 属于新增依赖(spec 明确不新增)，UTC 分桶对"看近 7 天趋势形状"这个用途
/// 足够，不追求跟用户本地墙上时钟严格对齐。
fn day_index_from_ms(ms: u64) -> i64 {
    (ms / 86_400_000) as i64
}

/// 当前时刻对应的 UTC 日索引，供趋势图当时间轴的右端点（“最近 N 天”）。同上
/// 不做时区换算，与 `day_index_from_ms`/`daily_totals_by_agent` 的分桶口径一致。
fn today_day_index() -> i64 {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    day_index_from_ms(ms)
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
/// 图表从左到右自然是时间顺序）。`totals` 只含这个项目实际用过的 agent
/// (与 `agent_token_share` 同一套判定+顺序),不再是写死的 3/4 家——某个
/// agent 这个项目压根没用过,就不该在柱状图里占一个永远是 0 的位置
/// (2026-08-27 修正,原先固定 claude/codebuddy/opencode 三个字段,V8agent
/// 完全进不了图,别的项目哪怕只用一家 agent 也照样画三根柱子)。同一个
/// agent 在所有日期分组里 `totals` 的顺序保持一致,方便跨日对比同一根
/// 柱子的颜色/位置。
#[derive(Debug, Clone, PartialEq)]
pub struct DayAgentTotals {
    pub day_index: i64,
    pub label: String,
    pub totals: Vec<(AgentKind, u64)>,
}

pub fn daily_totals_by_agent(
    rows: &[(ConversationMeta, ConversationUsage)],
) -> Vec<DayAgentTotals> {
    use std::collections::BTreeMap;
    let project_agents: Vec<AgentKind> = agent_token_share(rows)
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    if project_agents.is_empty() {
        return Vec::new();
    }
    // 内层按 `project_agents` 里的下标(而不是 `AgentKind` 本身)分桶——
    // `AgentKind` 没实现 `Ord`/`Hash`,当不了 `BTreeMap`/`HashMap` 的键;
    // agent 数量本来就是个位数,线性查下标够用,不值得为此给协议层的
    // `AgentKind` 加派生。
    let mut by_day: BTreeMap<i64, Vec<u64>> = BTreeMap::new();
    for (meta, usage) in rows {
        let Some(idx) = project_agents.iter().position(|&a| a == meta.agent) else {
            continue;
        };
        let day = day_index_from_ms(meta.modified_ms);
        let total =
            usage.tokens_in + usage.tokens_out + usage.tokens_cache_read + usage.tokens_cache_write;
        let bucket = by_day
            .entry(day)
            .or_insert_with(|| vec![0; project_agents.len()]);
        bucket[idx] += total;
    }
    let mut days: Vec<DayAgentTotals> = by_day
        .into_iter()
        .map(|(day_index, bucket)| {
            // 年份在"近 7 天"这种短窗口的标签里用不上，解构时直接忽略。
            let (_, m, d) = civil_from_days(day_index);
            let totals = project_agents.iter().copied().zip(bucket).collect();
            DayAgentTotals {
                day_index,
                label: format!("{m:02}/{d:02}"),
                totals,
            }
        })
        .collect();
    let start = days.len().saturating_sub(DAILY_CHART_WINDOW_DAYS);
    days.split_off(start)
}

/// 选中具体 agent 后的存量趋势聚合（2026-09-05,+n）数据的单天结构：某天的
/// 若干并列可度量量（`values[i]` 的含义由调用方给出序列标签）。与上面
/// `DayAgentTotals` 专为“每天按 agent 分几根柱”不同，这里“每天并列几根
/// 量”由 `values` 按序列下标承载——实际服务 Session 趋势（会话/回合）与
/// Token 趋势（Input/Output、Cache read/write）。
#[derive(Debug, Clone, PartialEq)]
pub struct DaySeries {
    pub day_index: i64,
    pub label: String,
    pub values: Vec<u64>,
}

/// 三个趋势各自的最近窗口天数（2026-09-05 用户要求：Session 与 Input/Output
/// 看近 15 天整的一条轴；Cache read/write 只看近 5 天更紧的一跳）。
const SESSION_TREND_WINDOW: i64 = 15;
const IO_TREND_WINDOW: i64 = 15;
const CACHE_TREND_WINDOW: i64 = 5;

/// 把某 agent 的会话按天分桶、摊成最近 `window` 天的一根连续时间轴（最旧在
/// 左、今天在右，`values[bar_idx]` 交给 `per_row` 逐条会话各取一个标量求和；
/// `conversation_count` 的“每会话记 1”由 `session` 桶单独算）。空的天不会
/// 被丢掉——趋势类图表如果只画实际有数据的日期，稀疏使用看起来就像是连续的
/// 活跃曲线（计划待办里测过、产品方向也认可连续轴）。时间轴右端落在
/// `last_day_index`（调方传“今天”，单测传固定值保证可复现），不从最新的
/// 数据那天开始反向截——那样一旦最近几天没数据窗口就会漂移。
///
/// 返回的 `DaySeries.values.len() == per_series_count`（统一每列都有那么多根
/// 柱，未填充的是 0），保证渲染时某天某序列缺数据也能占一根、不塌列。
fn trend_series(
    rows: &[(ConversationMeta, ConversationUsage)],
    agent: AgentKind,
    window: i64,
    last_day_index: i64,
    per_row: impl Fn(&ConversationUsage) -> Vec<u64>,
) -> Vec<DaySeries> {
    let first_day = last_day_index - window + 1;
    let count = per_row(&ConversationUsage::default()).len();
    let mut buckets: std::collections::BTreeMap<i64, Vec<u64>> = std::collections::BTreeMap::new();
    for (meta, usage) in rows {
        if meta.agent != agent {
            continue;
        }
        let day = day_index_from_ms(meta.modified_ms);
        if day < first_day || day > last_day_index {
            continue;
        }
        let row = per_row(usage);
        let slot = buckets.entry(day).or_insert_with(|| vec![0; count]);
        for (i, v) in row.into_iter().enumerate() {
            slot[i] = slot[i].saturating_add(v);
        }
    }
    (first_day..=last_day_index)
        .map(|day_index| {
            let (_, m, d) = civil_from_days(day_index);
            let values = buckets.remove(&day_index).unwrap_or_else(|| vec![0; count]);
            DaySeries {
                day_index,
                label: format!("{m:02}/{d:02}"),
                values,
            }
        })
        .collect()
}

/// Session 趋势：某 agent 最近 `SESSION_TREND_WINDOW` 天逐日 `[会话数, 回合数]`
/// 两序列。会话数=当日的 transcript（会话文件）条数，与 `agent_session_share`
/// 口径一致（含零回合空会话，各记 1）；回合数=`turns` 求和。
fn session_round_trend(
    rows: &[(ConversationMeta, ConversationUsage)],
    agent: AgentKind,
    today_index: i64,
) -> Vec<DaySeries> {
    trend_series(rows, agent, SESSION_TREND_WINDOW, today_index, |u| {
        vec![1, u.turns as u64]
    })
}

fn io_trend(
    rows: &[(ConversationMeta, ConversationUsage)],
    agent: AgentKind,
    today_index: i64,
) -> Vec<DaySeries> {
    trend_series(rows, agent, IO_TREND_WINDOW, today_index, |u| {
        vec![u.tokens_in, u.tokens_out]
    })
}

fn cache_trend(
    rows: &[(ConversationMeta, ConversationUsage)],
    agent: AgentKind,
    today_index: i64,
) -> Vec<DaySeries> {
    trend_series(rows, agent, CACHE_TREND_WINDOW, today_index, |u| {
        vec![u.tokens_cache_read, u.tokens_cache_write]
    })
}

/// 展示固定顺序（含 V8agent，CLAUDE.md：新功能默认覆盖它，不能像 Codex/
/// Kilo 那样被漏掉）。2026-09-05 起把原先三份拷贝收拢成这一份常量，供下面
/// 各 `agent_*_share` 与 `agents_present` 共用，避免动一处漏一处的旧坑。
const AGENT_ORDER: [AgentKind; 4] = [
    AgentKind::Claude,
    AgentKind::Codebuddy,
    AgentKind::Opencode,
    AgentKind::V8agent,
];

/// 整个项目范围按 agent 归总某会话级标量（`per_row` 从每条会话取一个数）。
/// 只返回总和 > 0 的 agent，不产生全零占位记录、不改变 `AGENT_ORDER` 顺序
/// ——四种统计饼图共用同一套口径与起止，图例顺序始终对齐。
fn agent_metric_share(
    rows: &[(ConversationMeta, ConversationUsage)],
    per_row: impl Fn(&ConversationUsage) -> u64,
) -> Vec<(AgentKind, u64)> {
    AGENT_ORDER
        .into_iter()
        .filter_map(|kind| {
            let total: u64 = rows
                .iter()
                .filter(|(meta, _)| meta.agent == kind)
                .map(|(_, u)| per_row(u))
                .sum();
            (total > 0).then_some((kind, total))
        })
        .collect()
}

/// 整个项目范围（不限"近 7 天"）按 agent 的 token 总量（四项合计）。四张
/// 统计饼图从这条总口径切出细分口径（见 `agent_io_token_share`、`agent_cache_
/// token_share`）前，`daily_totals_by_agent` 仍用它的 agent 集合当"本项目出现
/// 过的 agent"排序基准。
pub fn agent_token_share(rows: &[(ConversationMeta, ConversationUsage)]) -> Vec<(AgentKind, u64)> {
    agent_metric_share(rows, |u| {
        u.tokens_in + u.tokens_out + u.tokens_cache_read + u.tokens_cache_write
    })
}

/// 整个项目范围按 agent 的"回合"数合计——口径沿用用量面板的"回合"即
/// human 发言数（2026-08-27 调整，见 dozerd `get_usage_summary_in`）。
pub fn agent_turn_share(rows: &[(ConversationMeta, ConversationUsage)]) -> Vec<(AgentKind, u64)> {
    agent_metric_share(rows, |u| u.turns as u64)
}

/// 每个 agent 的"会话数"＝它在项目里留下的 transcript 会话条数。每一条
/// 会话记 1（含零回合的空会话），让"这个 agent 跑过几个会话"独立成立。
pub fn agent_session_share(
    rows: &[(ConversationMeta, ConversationUsage)],
) -> Vec<(AgentKind, u64)> {
    agent_metric_share(rows, |_| 1)
}

/// Input/Output token：每会话上下文进出量（`tokens_in + tokens_out`）。
pub fn agent_io_token_share(
    rows: &[(ConversationMeta, ConversationUsage)],
) -> Vec<(AgentKind, u64)> {
    agent_metric_share(rows, |u| u.tokens_in + u.tokens_out)
}

/// Cache Read/Write token：每会话提示缓存读写/写入量
/// （`tokens_cache_read + tokens_cache_write`），与 IO 量分开看图。
pub fn agent_cache_token_share(
    rows: &[(ConversationMeta, ConversationUsage)],
) -> Vec<(AgentKind, u64)> {
    agent_metric_share(rows, |u| u.tokens_cache_read + u.tokens_cache_write)
}

/// 项目里实际出现过的 agent,顺序固定(共用上面的 `AGENT_ORDER`,含 V8agent
/// ——新功能默认覆盖它,不能像 Codex/Kilo 那样被漏掉)。供右侧筛选栏用:传入
/// 未经筛选的全量 `rows`,这样切换到某个 agent 之后,列表本身不会跟着收缩到
/// 只剩它自己。
pub fn agents_present(rows: &[(ConversationMeta, ConversationUsage)]) -> Vec<AgentKind> {
    AGENT_ORDER
        .into_iter()
        .filter(|kind| rows.iter().any(|(meta, _)| meta.agent == *kind))
        .collect()
}

/// 按右侧筛选栏当前选中的 agent 过滤统计用的行,`None` = 不过滤("全部
/// agent")。返回拥有所有权的克隆而不是借用——下游 `aggregate`/
/// `agent_token_share`/`daily_totals_by_agent` 都吃
/// `&[(ConversationMeta, ConversationUsage)]`,筛选后条数通常不大,直接拥
/// 有一份比额外弄一层借用包装简单。
fn filter_rows_by_agent(
    rows: &[(ConversationMeta, ConversationUsage)],
    filter: Option<AgentKind>,
) -> Vec<(ConversationMeta, ConversationUsage)> {
    match filter {
        None => rows.to_vec(),
        Some(agent) => rows
            .iter()
            .filter(|(meta, _)| meta.agent == agent)
            .cloned()
            .collect(),
    }
}

pub fn update(ws_state: &mut WorkspaceState, msg: Message) {
    match msg {
        Message::Loaded(_, rows) => {
            ws_state.rows = rows;
            ws_state.loading = false;
        }
        Message::AgentFilterSet(agent) => {
            ws_state.agent_filter = agent;
        }
        Message::ToggleListCollapse => {
            unreachable!("由内核拦截处理,见 usage::Message::ToggleListCollapse 文档")
        }
        Message::Hover(_, _) => {
            unreachable!("由内核拦截处理,见 usage::Message::Hover 文档")
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

/// 面板内容侧:统计图表 + 顶部"用量"标题。原先跟 `list_pane`(agent 筛选栏)
/// 挤在同一个 `view` 函数里手写 `row![content, sidebar]`,现在拆成独立的
/// 列表/内容两个面板函数,接入跟 Database/Agent 面板一样的可拖拽 split +
/// 收起机制(见 app.rs `PanelKind::Usage` 分支、`Divider::UsageSplit`)。
/// `rows` 为空且 `loading` 为假时是"还没数据"的空态；`loading` 为真时是
/// 刷新中占位态；两者互斥由 `update` 保证(`WorkspaceState::set_loading`
/// 调用后、`Loaded` 落地时清掉)。
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
    let head = crate::homespace::home_panel_head_with_actions(
        icons::IconKind::BarChart3,
        "用量",
        Some(collapse),
    );
    let mut content = column![head].spacing(12).padding(14).width(Length::Fill);

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
fn agent_filter_sidebar<'a>(
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
fn agent_filter_button<'a>(
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
const BAR_MAX_HEIGHT: f32 = 72.0;
/// 柱宽(2026-08-23 起 20→14,当时 `DAILY_CHART_WINDOW_DAYS` 从 7 改到
/// 15 让柱数翻倍;2026-08-28 窗口改回 7 天,但沿用这个更紧凑的宽度)。
const BAR_WIDTH: f32 = 14.0;
/// 柱顶总量数字 + 间距预留的高度,`GridLines`/`bar_chart` 靠它对齐网格线
/// 与柱子的 0 基线(见 `bar_chart` 里 `col` 首个 `container` 的同一个值)。
const BAR_LABEL_GAP: f32 = 14.0;
const GRID_CANVAS_HEIGHT: f32 = BAR_MAX_HEIGHT + BAR_LABEL_GAP;
/// 每天间隔背景条带的高度:柱子区域(`GRID_CANVAS_HEIGHT`)加上列内
/// spacing 和日期文字行,让条带从柱顶盖到日期标签底部。后两项是估算值
/// (8px 字号文字行高约 11~12px),条带本就是装饰性的,像素级出入不影响观感。
const DAY_BAND_HEIGHT: f32 = GRID_CANVAS_HEIGHT + 4.0 + 12.0;
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
/// 同一套视觉。零值 agent 不列(同 `chart_stat_list` 只列有数据的 agent)。
fn day_tooltip_bubble(
    day: &DayAgentTotals,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut rows = column![
        text(day.label.clone())
            .size(byteui::theme::font::caption_sm())
            .color(byteui::theme::color::current().cream),
    ]
    .spacing(4);
    for &(agent, value) in &day.totals {
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
        .flat_map(|d| d.totals.iter().map(|(_, v)| *v))
        .max()
        .unwrap_or(1)
        .max(1);

    // 每天内部并排出这个项目实际用过的 agent 各自的柱子(`d.totals` 已经
    // 是动态集合,不再写死 3 根——2026-08-27 修正,原先固定
    // Claude/CodeBuddy/OpenCode 三根,只用一家 agent 的项目也会画出两根
    // 常年 0 的柱子),组与组之间间隔更大,agent 相邻贴得更近,方便
    // "同日横向对比 + 跨日纵向看趋势"(2026-08-27 由"每日一根堆叠柱"改为
    // 分组柱状图)。颜色统一走 `agent_dot_color`,不再在这里单独维护一份
    // cyan/purple/green 映射。
    let mut groups = iced_widget::row![].spacing(8);
    for (i, d) in days.iter().enumerate() {
        let scale = BAR_MAX_HEIGHT / max_total as f32;
        let mut day_group = iced_widget::row![].spacing(3);
        for &(agent, value) in &d.totals {
            day_group = day_group.push(agent_bar(
                value,
                scale,
                crate::workspace::agent_dot_color(agent),
            ));
        }
        let day_group = day_group.align_y(iced_widget::core::alignment::Vertical::Bottom);

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

        // 间隔背景条带:偶数日(0-based)铺一块 `card` 底色,奇数日透明,
        // 形成"一天有背景、一天没有"的斑马纹,方便按天分组扫视(2026-08-28
        // 产品要求)。奇偶两种日子共用同一份 padding/圆角,不会因为背景
        // 有无而让列宽跳动。
        let banded = container(hoverable)
            .padding([0, 4])
            .height(Length::Fixed(DAY_BAND_HEIGHT))
            .style(
                move |_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: (i % 2 == 0).then(|| byteui::theme::color::current().card.into()),
                    border: Border {
                        radius: 4.0.into(),
                        ..Border::default()
                    },
                    ..iced_widget::container::Style::default()
                },
            );

        groups = groups.push(banded);
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
    .height(Length::Fixed(DAY_BAND_HEIGHT))
    .into()
}

/// 一张趋势柱状图里并列的若干序列标签与配色。`values` 下标与这里一一对应,
/// 渲染与图例共用同一份,避免两处各写一遍序列名/色。
type TrendSeries = Vec<(&'static str, Color)>;

/// 悬停趋势图某天柱子的气泡:日期 + 该天各序列值(>0 才列,同 `chart_stat_list`
/// 只列有效数据的口径)。样式复用 `icons::tooltip_bubble_style`。
fn trend_tooltip_bubble(
    day: &DaySeries,
    series: &TrendSeries,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut rows = column![
        text(day.label.clone())
            .size(byteui::theme::font::caption_sm())
            .color(byteui::theme::color::current().cream),
    ]
    .spacing(4);
    for (i, &(label, color)) in series.iter().enumerate() {
        if day.values[i] == 0 {
            continue;
        }
        let dot = container(iced_widget::Space::new())
            .width(Length::Fixed(8.0))
            .height(Length::Fixed(8.0))
            .style(
                move |_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: Some(color.into()),
                    border: Border {
                        radius: 4.0.into(),
                        ..Border::default()
                    },
                    ..iced_widget::container::Style::default()
                },
            );
        rows = rows.push(
            iced_widget::row![
                dot,
                text(format!("{label}: {}", format_count(day.values[i])))
                    .size(byteui::theme::font::caption_sm())
                    .color(byteui::theme::color::current().dim)
                    .font(iced_widget::core::Font::MONOSPACE),
            ]
            .spacing(6)
            .align_y(iced_widget::core::alignment::Vertical::Center),
        );
    }
    container(rows).padding([6, 8]).into()
}

/// 横排的序列图例:色点 + 名称一排（放在趋势图标题下方）。替代对多序列柱状图
/// 用鼠悬一个个去猜颜色。
fn trend_legend(
    series: &TrendSeries,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut row = iced_widget::row![].spacing(12);
    for (label, color) in series {
        let dot = container(iced_widget::Space::new())
            .width(Length::Fixed(8.0))
            .height(Length::Fixed(8.0))
            .style({
                let color = *color;
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
                text(*label)
                    .size(byteui::theme::font::caption_sm())
                    .color(byteui::theme::color::current().dim),
            ]
            .spacing(5)
            .align_y(iced_widget::core::alignment::Vertical::Center),
        );
    }
    row.into()
}

/// 通用"每天并列若干序列柱"的趋势柱状图，结构和 `bar_chart`（每天并列几根
/// agent 柱、叠网格/斑马带/悬停气泡）保持一致，只是每根柱来自 `series` 对应
/// 下标的量而不是某个 agent。`days` 的下标数量须 === `series.len()`（聚合时
/// 已保证每天固定那么多数量的柱，空天补 0）。
fn trend_bar_chart(
    days: &[DaySeries],
    series: &TrendSeries,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let max_total = days
        .iter()
        .flat_map(|d| d.values.iter().copied())
        .max()
        .unwrap_or(1)
        .max(1);
    let scale = BAR_MAX_HEIGHT / max_total as f32;

    let mut groups = iced_widget::row![].spacing(8);
    for (i, d) in days.iter().enumerate() {
        let mut day_group = iced_widget::row![].spacing(3);
        for (j, (_, color)) in series.iter().enumerate() {
            let value = d.values[j];
            day_group = day_group.push(agent_bar(value, scale, *color));
        }
        let day_group = day_group.align_y(iced_widget::core::alignment::Vertical::Bottom);

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

        let hoverable = Tooltip::new(col, trend_tooltip_bubble(d, series), Position::Top)
            .gap(6)
            .style(icons::tooltip_bubble_style());

        let banded = container(hoverable)
            .padding([0, 4])
            .height(Length::Fixed(DAY_BAND_HEIGHT))
            .style(
                move |_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: (i % 2 == 0).then(|| byteui::theme::color::current().card.into()),
                    border: Border {
                        radius: 4.0.into(),
                        ..Border::default()
                    },
                    ..iced_widget::container::Style::default()
                },
            );
        groups = groups.push(banded);
    }

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
    .height(Length::Fixed(DAY_BAND_HEIGHT))
    .into()
}

/// `days` 里是否至少一天有非零值——趋势窗口若整段都是 0（目标 agent 最近
/// 这段时间其实没活动），上层就不画这个趋势区，避免白框空难读。
fn has_any_value(days: &[DaySeries]) -> bool {
    days.iter().any(|d| d.values.iter().any(|&v| v > 0))
}

/// 单个"小节标题 +（标题右侧）序列图例 + 图表"的组合。`title` 仍走统一的
/// `home_section_head` 小结标题样式(Session 趋势 / Token 趋势…)，图例挨在它
/// 右侧对齐、不占纵向。放在节内的内层 column 统一吃 `SECTION_CHART_GAP`。
fn trend_chart_section(
    title: &'static str,
    series: TrendSeries,
    days: &[DaySeries],
) -> Option<Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    if !has_any_value(days) {
        return None;
    }
    let title_row = iced_widget::row![
        home_section_head(title),
        iced_widget::Space::new()
            .width(Length::Fill)
            .height(Length::Shrink),
        trend_legend(&series),
    ]
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .spacing(8);
    Some(
        column![title_row]
            .spacing(SECTION_CHART_GAP)
            .push(trend_bar_chart(days, &series))
            .into(),
    )
}

/// Session 趋势的序列配色——会话/回合各一根柱,分别用 cream(项目"活动类"统计的
/// 颜色习惯)与 green;对比只在同一张图内部保色相区分,跨图可重复用色。
fn session_trend_series() -> TrendSeries {
    let c = byteui::theme::color::current();
    vec![("会话", c.cream), ("回合", c.green)]
}

fn io_trend_series() -> TrendSeries {
    let c = byteui::theme::color::current();
    vec![("Input", c.cyan), ("Output", c.purple)]
}

fn cache_trend_series() -> TrendSeries {
    let c = byteui::theme::color::current();
    vec![("读", c.lime), ("写", c.green)]
}

/// 一张趋势图右上角的"窗口"小标注(如"近 15 天"),放在标题行里,让读者一眼
/// 看到口径,不靠猜。
fn trend_window_tag(
    window: &'static str,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    text(window)
        .size(byteui::theme::font::caption_sm())
        .color(byteui::theme::color::current().dim)
        .into()
}

/// Token 趋势区:一个标题(Token 趋势)下按 15 天(Input/Output)与 5 天
/// (Cache read/write)分别画两根并列柱。两张窗口/量级差太多,单独成图、各自
/// 独立纵轴 max,不共用刻度。任一图整段无数据时只画另一张;都空则整区不给。
fn token_trend_section(
    agent: AgentKind,
    rows: &[(ConversationMeta, ConversationUsage)],
    today_index: i64,
) -> Option<Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let io = io_trend(rows, agent, today_index);
    let cache = cache_trend(rows, agent, today_index);

    let io_series = io_trend_series();
    let cache_series = cache_trend_series();
    let io_has = has_any_value(&io);
    let cache_has = has_any_value(&cache);

    let mut head_row = iced_widget::row![
        home_section_head("Token 趋势"),
        iced_widget::Space::new()
            .width(Length::Fill)
            .height(Length::Shrink),
    ]
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .spacing(8);
    if io_has {
        let io_group = iced_widget::row![
            trend_legend(&io_series),
            iced_widget::Space::new()
                .width(Length::Fixed(10.0))
                .height(Length::Shrink),
            trend_window_tag("近 15 天 Input/Output"),
        ]
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .spacing(6);
        head_row = head_row.push(io_group);
    }

    let mut section = column![head_row].spacing(SECTION_CHART_GAP);
    if io_has {
        section = section.push(trend_bar_chart(&io, &io_series));
    }

    if cache_has {
        let cache_header = iced_widget::row![
            text("Cache read / write")
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().dim),
            iced_widget::Space::new()
                .width(Length::Fill)
                .height(Length::Shrink),
            trend_legend(&cache_series),
            trend_window_tag("近 5 天"),
        ]
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .spacing(8);
        section = section
            .push(cache_header)
            .push(trend_bar_chart(&cache, &cache_series));
    }

    if !io_has && !cache_has {
        return None;
    }
    Some(section.into())
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

/// 用量面板"小节(mid)"标题与它正下方图表之间的纵向间距。小节标题与图表包进
/// 内层 column 单独吃这个值(见 `content_pane`),与面板外层常规的 `spacing(12)`
/// 解耦——2026-09-05 产品要求"加大每一节标题和 chart 的间距"。
const SECTION_CHART_GAP: f32 = 24.0;

/// 子栏目标题 `home_section_head` 由「圆点图标 + 6px 间距 + 标题文字」组成,
/// 标题文字相对该行起点缩进 `icon_size::row() + 6`。每个小节标题下的卡片/图表
/// 若要和标题**文字**左对齐(而不是跟圆点起点),内容同样要收走这么远,否则视觉
/// 上卡片总比标题突出一截。2026-09-07:用量面板三个小节(项目 / Agent / 每日)
/// 的卡组统一用它对齐到标题文字。
const SECTION_BODY_INSET: fn() -> f32 = || byteui::theme::icon_size::row() + 6.0;

/// 把某个紧跟在 `home_section_head` 之下的图表/卡片组整体左收 `SECTION_BODY_INSET`,
/// 使它的左边缘与上一行标题文字起点对齐。
fn align_to_section_title<'a>(
    body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    row![
        iced_widget::Space::new()
            .width(Length::Fixed(SECTION_BODY_INSET()))
            .height(Length::Fill),
        container(body).width(Length::Fill),
    ]
    .width(Length::Fill)
    .spacing(0)
    .into()
}

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

/// 一个 share 口径内全部 agent 的次数之和(横排"整组/整节总数"、横幅题头用它;
/// 与 `chart_stat_list` 内部的总数算法一致)。
fn share_total(share: &[(AgentKind, u64)]) -> u64 {
    share.iter().map(|(_, v)| v).sum()
}

fn agent_metric_group(
    title: &'static str,
    share: &[(AgentKind, u64)],
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 一节 metric:环图居左,右边竖排"标题(total)+逐 agent 数字表"。这个组合
    // 只负责把环图旁边该显示的文字带出来;两节等宽二分由外层 `pair_metric_cells`
    // 让本节的容器吃掉 `FillPortion(1)`。
    iced_widget::row![pie_chart(share), chart_stat_list(title, share)]
        .spacing(16)
        .width(Length::Fill)
        .align_y(iced_widget::core::Alignment::Center)
        .into()
}

/// 把两个指标节并排成一行,并让左右各自恰好各占面板一半宽(2026-09-05 要求:
/// “左侧饼图区域的宽度和右侧饼图区域宽度保持一致,即平分面板宽度”)。用
/// `Length::FillPortion(1)` 让两节在行内平均瓜分剩余高宽,而不是各自按内容
/// 自然宽缩放——即使左表文字比右短,中线也稳居面板正中。某一边没有数据时,
/// 只放有数据那一节并让它吃满整行,不给空白占位。
fn pair_metric_cells<'a>(
    left: Option<(&'static str, &'a [(AgentKind, u64)])>,
    right: Option<(&'static str, &'a [(AgentKind, u64)])>,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let cell = |title: &'static str, share: &'a [(AgentKind, u64)]| {
        container(agent_metric_group(title, share))
            .width(Length::FillPortion(1))
            .into()
    };
    let positioned: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match (left, right) {
            (Some((l, ls)), Some((r, rs))) => iced_widget::row![cell(l, ls), cell(r, rs)]
                .width(Length::Fill)
                .spacing(16)
                .into(),
            (Some((l, ls)), None) | (None, Some((l, ls))) => cell(l, ls),
            (None, None) => iced_widget::Space::new().into(),
        };
    positioned
}

/// 一个大组的“横幅”题头:左边浅色词(如 “Session”),右侧排在同行的等宽数字
/// 给出该组口径总数、紧跟字面 “total”(示例 “Session 190 total”)。2026-09-05
/// 用户要求这种整段首行概况。数字与列表内各行共用 `format_count` 三档缩写,
/// 避免横幅与底下各行对同样大的数目措辞不一致(如都 1.2k,而不是横幅 1234、
/// 底下 1.2k)。
fn metric_group_banner(
    label: &'static str,
    total: u64,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let c = byteui::theme::color::current();
    iced_widget::row![
        text(label)
            .size(byteui::theme::font::label())
            .color(c.cream),
        iced_widget::Space::new().width(Length::Fill),
        text(format!("{} total", format_count(total)))
            .size(byteui::theme::font::caption())
            .color(c.cream)
            .font(iced_widget::core::Font::MONOSPACE),
    ]
    .width(Length::Fill)
    .into()
}

/// Agent 用量的列表式呈现,配在饼图右边当图例:标题行 `{title}(总数)` +
/// 逐 agent `agent - 数量(百分比%)`(2026-08-28 用户反馈:圆环不能去掉,只是
/// 把图例的文字格式换成这种更直接的数字表——取代原来的 `chart_legend`/
/// `chart_label` 文字格式,饼图本体保留。标题行与列表间的 1px 分隔线于
/// 2026-09-05 用户要求去掉,标题后直接接数字表)。百分比用整数除法截断、
/// 不四舍五入,跟原图例的算法保持一致,避免几档相加超过 100%。
fn chart_stat_list(
    title: &'static str,
    share: &[(AgentKind, u64)],
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let total: u64 = share.iter().map(|(_, v)| v).sum();
    let cream = byteui::theme::color::current().cream;
    let dim = byteui::theme::color::current().dim;
    let mut col = column![
        text(format!("{title}({})", format_count(total)))
            .size(byteui::theme::font::caption())
            .color(cream)
            .font(iced_widget::core::Font::MONOSPACE),
    ]
    .spacing(8);
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
                    "{} - {}({pct}%)",
                    agent.label(),
                    format_count(*value)
                ))
                .size(byteui::theme::font::caption_sm())
                .color(dim)
                .font(iced_widget::core::Font::MONOSPACE),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        );
    }
    col.into()
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

    fn total_for(day: &DayAgentTotals, agent: AgentKind) -> u64 {
        day.totals
            .iter()
            .find(|(a, _)| *a == agent)
            .map(|(_, v)| *v)
            .unwrap_or(0)
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
        assert_eq!(
            total_for(&days[0], AgentKind::Claude),
            15,
            "同一天两条 Claude 会话的 token 要累加"
        );
        assert_eq!(total_for(&days[0], AgentKind::Codebuddy), 2);
        assert_eq!(
            total_for(&days[0], AgentKind::Opencode),
            0,
            "Opencode 这个项目里用过(次日有),当天没用则该 agent 那天是 0,不是不出现"
        );
        assert_eq!(days[1].day_index, 20_673);
        assert_eq!(total_for(&days[1], AgentKind::Opencode), 7);
        assert_eq!(days[0].label, "08/07");
    }

    #[test]
    fn daily_totals_by_agent_only_includes_agents_actually_used_in_project() {
        // 这个项目只用过 Claude,不该在图里给从没出现过的 Codebuddy/Opencode
        // 各占一根常年 0 的柱子(2026-08-27 修正的真实 bug)。
        let rows = vec![(meta_at(AgentKind::Claude, 0), usage_with_tokens(10))];
        let days = daily_totals_by_agent(&rows);
        assert_eq!(days.len(), 1);
        assert_eq!(
            days[0].totals,
            vec![(AgentKind::Claude, 10)],
            "只应该出现 Claude 一项,不该混进 Codebuddy/Opencode 的 0 值占位"
        );
    }

    #[test]
    fn daily_totals_by_agent_includes_v8agent() {
        let rows = vec![(meta_at(AgentKind::V8agent, 0), usage_with_tokens(3))];
        let days = daily_totals_by_agent(&rows);
        assert_eq!(days.len(), 1);
        assert_eq!(total_for(&days[0], AgentKind::V8agent), 3);
    }

    #[test]
    fn daily_totals_ignores_agents_without_dedicated_bucket() {
        // Codex/Kilo 目前不产出可统计的用量数据（见计划 Global
        // Constraints），跟 Unknown 一样被忽略，不能 panic,也不产生
        // 任何一天的记录(没有任何可展示的 agent,图表应该整体不渲染)。
        let rows = vec![(meta_at(AgentKind::Codex, 0), usage_with_tokens(99))];
        let days = daily_totals_by_agent(&rows);
        assert!(days.is_empty(), "没有任何已知 agent 有数据时不该产出天记录");
    }

    #[test]
    fn daily_totals_by_agent_keeps_only_most_recent_7_days() {
        let rows: Vec<_> = (0..20)
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
            20_687,
            "最后一天是最新的那天"
        );
        assert_eq!(days.first().unwrap().day_index, 20_681);
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

    /// V8agent(用户自研 agent)默认要被用量统计覆盖,不能像 Codex/Kilo 那样
    /// 被排除——2026-08-27 之前 `ORDER` 常量漏了它,饼图/图例里完全不出现
    /// 这家的用量,是真实 bug 不是刻意范围收窄。
    #[test]
    fn agent_token_share_includes_v8agent() {
        let rows = vec![(meta(AgentKind::V8agent, "a"), usage_with_tokens(7))];
        let share = agent_token_share(&rows);
        assert_eq!(share, vec![(AgentKind::V8agent, 7)]);
    }

    #[test]
    fn agent_session_share_counts_one_per_conversation_including_zero_turn() {
        let rows = vec![
            // 同一 agent 两条会话。
            (meta(AgentKind::Claude, "a"), usage_with_turns(3)),
            (meta(AgentKind::Claude, "b"), usage_with_turns(0)),
            // 不同 agent 一条会话(也给回合)。
            (meta(AgentKind::Codebuddy, "c"), usage_with_turns(1)),
        ];
        // 回合口径把"零回合会话"的 Claude 剔除 → 只有 (Claude,3),(Codebuddy,1);
        assert_eq!(
            agent_turn_share(&rows),
            vec![(AgentKind::Claude, 3), (AgentKind::Codebuddy, 1)]
        );
        // 会话数口径每会话记 1 → 空回合会话也要占一条。
        assert_eq!(
            agent_session_share(&rows),
            vec![(AgentKind::Claude, 2), (AgentKind::Codebuddy, 1)]
        );
    }

    #[test]
    fn agent_io_cache_shares_keep_io_and_cache_separate() {
        let usage = |io: u64, cache: u64| ConversationUsage {
            tokens_in: io,             // IO 读
            tokens_out: io,            // IO 写(同量便于断言合值)
            tokens_cache_read: cache,  // 缓存读
            tokens_cache_write: cache, // 缓存写
            ..Default::default()
        };
        let rows = vec![
            (meta(AgentKind::Claude, "a"), usage(10, 4)),
            (meta(AgentKind::Claude, "b"), usage(1, 1)),
            (meta(AgentKind::V8agent, "c"), usage(0, 3)),
        ];
        // IO = tokens_in+tokens_out 之和;缓存 = cache_read+cache_write 之和,
        // 两类分开计、不混在一个图里。
        assert_eq!(agent_io_token_share(&rows), vec![(AgentKind::Claude, 22)]);
        assert_eq!(
            agent_cache_token_share(&rows),
            vec![(AgentKind::Claude, 10), (AgentKind::V8agent, 6)]
        );
    }

    #[test]
    fn agents_present_returns_used_agents_in_fixed_order() {
        let rows = vec![
            (meta(AgentKind::Codebuddy, "a"), usage_with_tokens(1)),
            (meta(AgentKind::Claude, "b"), usage_with_tokens(1)),
            (meta(AgentKind::V8agent, "c"), usage_with_tokens(1)),
        ];
        assert_eq!(
            agents_present(&rows),
            vec![AgentKind::Claude, AgentKind::Codebuddy, AgentKind::V8agent]
        );
    }

    #[test]
    fn agents_present_ignores_agents_without_dedicated_bucket() {
        let rows = vec![(meta(AgentKind::Codex, "a"), usage_with_tokens(1))];
        assert_eq!(agents_present(&rows), Vec::new());
    }

    #[test]
    fn filter_rows_by_agent_none_keeps_everything() {
        let rows = vec![
            (meta(AgentKind::Claude, "a"), usage_with_tokens(1)),
            (meta(AgentKind::Codebuddy, "b"), usage_with_tokens(1)),
        ];
        assert_eq!(filter_rows_by_agent(&rows, None), rows);
    }

    #[test]
    fn filter_rows_by_agent_some_keeps_only_that_agent() {
        let rows = vec![
            (meta(AgentKind::Claude, "a"), usage_with_tokens(1)),
            (meta(AgentKind::Codebuddy, "b"), usage_with_tokens(2)),
            (meta(AgentKind::Claude, "c"), usage_with_tokens(3)),
        ];
        let filtered = filter_rows_by_agent(&rows, Some(AgentKind::Claude));
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|(m, _)| m.agent == AgentKind::Claude));
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
    fn agent_turn_share_includes_v8agent() {
        let rows = vec![(meta(AgentKind::V8agent, "a"), usage_with_turns(4))];
        let share = agent_turn_share(&rows);
        assert_eq!(share, vec![(AgentKind::V8agent, 4)]);
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

    #[test]
    fn session_round_trend_zero_fills_full_window_only_target_agent() {
        // day 20672 = 2026-08-07;13 天前 = 20659,仍在同一 15 天窗口内。
        let today = 20_672i64;
        let ms = |day: i64| (day as u64) * 86_400_000;
        let rows = vec![
            (
                meta_at(AgentKind::Claude, ms(today)),
                ConversationUsage {
                    turns: 2,
                    ..Default::default()
                },
            ),
            (
                meta_at(AgentKind::Claude, ms(today - 13)),
                ConversationUsage {
                    turns: 4,
                    ..Default::default()
                },
            ),
            // 其他 agent 即便同在今天有会话也不该被某单 agent 趋势吃进去。
            (
                meta_at(AgentKind::Codebuddy, ms(today)),
                ConversationUsage {
                    turns: 9,
                    ..Default::default()
                },
            ),
        ];
        let trend = session_round_trend(&rows, AgentKind::Claude, today);
        // 窗口从 today 往回整 15 个日历天,哪怕没有数据也补占位,不允许空窗漂。
        assert_eq!(trend.len(), 15);
        assert_eq!(trend[0].day_index, today - 14, "最左应正好是窗口首日(0 值)");
        assert_eq!(trend[0].values, vec![0, 0]);
        // 13 天前那条:today-13 落在窗口内,序列 [1,4](会话一次、回合 4)。
        let earlier = &trend[(today - 13 - (today - 14)) as usize];
        assert_eq!(earlier.day_index, today - 13);
        assert_eq!(earlier.values, vec![1, 4]);
        // 今天(窗口右端):只算 Claude,CodeBuddy 不进。
        let last = trend.last().unwrap();
        assert_eq!(last.day_index, today);
        assert_eq!(last.values, vec![1, 2]);
    }

    #[test]
    fn io_and_cache_trend_use_their_own_window_and_split_fields() {
        let today = 20_672i64;
        let ms = |day: i64| (day as u64) * 86_400_000;
        // sample_usage:in=10,out=2,read=1,write=1。
        let rows = vec![(meta_at(AgentKind::Claude, ms(today)), sample_usage(&[]))];
        let io = io_trend(&rows, AgentKind::Claude, today);
        let cache = cache_trend(&rows, AgentKind::Claude, today);
        assert_eq!(io.len(), 15, "Input/Output 看近 15 天");
        assert_eq!(cache.len(), 5, "Cache read/write 看近 5 天");
        assert_eq!(io.last().unwrap().values, vec![10, 2]);
        assert_eq!(cache.last().unwrap().values, vec![1, 1]);
    }

    #[test]
    fn window_axis_is_self_contained_when_recent_days_idle() {
        // 数据停在 14 天前,今天(窗口右端)本身没有更新——窗口仍从 today 起铺
        // 整 15 天,最新若干格是 0,不只收在有数据那几天。
        let today = 20_672i64;
        let ms = |day: i64| (day as u64) * 86_400_000;
        let rows = vec![(
            meta_at(AgentKind::Claude, ms(today - 14)),
            ConversationUsage {
                turns: 1,
                ..Default::default()
            },
        )];
        let trend = session_round_trend(&rows, AgentKind::Claude, today);
        assert_eq!(trend.len(), 15);
        assert_eq!(trend[0].values, vec![1, 1], "最旧天数据落在窗口最左");
        assert_eq!(
            trend.last().unwrap().values,
            vec![0, 0],
            "今天无活动但保留占位"
        );
    }
}
