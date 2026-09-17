//! 用量面板的图表渲染:柱状图/趋势折线/饼图及图例、tooltip。

use crate::chrome::homespace::home_section_head;
use crate::conversation::ConversationMeta;
use byteui::interaction::icons;
use dozer_core::protocol::AgentKind;
use iced_widget::canvas::{self, Canvas};
use iced_widget::core::{Border, Color, Element, Length, Point, Radians, Rectangle};
use iced_widget::tooltip::{Position, Tooltip};
use iced_widget::{column, container, row, stack, text};

use super::*;

pub(crate) const BAR_MAX_HEIGHT: f32 = 72.0;
/// 柱宽(2026-08-23 起 20→14,当时 `DAILY_CHART_WINDOW_DAYS` 从 7 改到
/// 15 让柱数翻倍;2026-08-28 窗口改回 7 天,但沿用这个更紧凑的宽度)。
pub(crate) const BAR_WIDTH: f32 = 14.0;
/// 柱顶总量数字 + 间距预留的高度,`GridLines`/`bar_chart` 靠它对齐网格线
/// 与柱子的 0 基线(见 `bar_chart` 里 `col` 首个 `container` 的同一个值)。
pub(crate) const BAR_LABEL_GAP: f32 = 14.0;
pub(crate) const GRID_CANVAS_HEIGHT: f32 = BAR_MAX_HEIGHT + BAR_LABEL_GAP;
/// 每天间隔背景条带的高度:柱子区域(`GRID_CANVAS_HEIGHT`)加上列内
/// spacing 和日期文字行,让条带从柱顶盖到日期标签底部。后两项是估算值
/// (8px 字号文字行高约 11~12px),条带本就是装饰性的,像素级出入不影响观感。
pub(crate) const DAY_BAND_HEIGHT: f32 = GRID_CANVAS_HEIGHT + 4.0 + 12.0;
/// 网格线左侧刻度数字预留的宽度:网格线本身从这条线右边才开始画,避免
/// 刻度数字跟第一根柱子顶部的总量数字重叠。
pub(crate) const GRID_LABEL_GUTTER: f32 = 26.0;
/// 目标网格线条数——实际条数取决于 `nice_tick_step` 算出的整数步长,一般
/// 落在 3~5 条,不保证精确等于这个数。
pub(crate) const GRID_TARGET_TICKS: u32 = 4;

pub(crate) fn bar_segment(
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
pub(crate) fn nice_tick_step(max_value: u64, target_ticks: u32) -> u64 {
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
pub(crate) fn grid_ticks(max_total: u64) -> Vec<u64> {
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
pub(crate) struct GridLines {
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

pub(crate) fn grid_lines_canvas(
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
pub(crate) fn day_tooltip_bubble(
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
pub(crate) fn agent_bar(
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

pub(crate) fn bar_chart(
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

/// 一张趋势折线图里并列的若干序列标签与配色。`values` 下标与这里一一对应,
/// 渲染与图例共用同一份,避免两处各写一遍序列名/色。
type TrendSeries = Vec<(&'static str, Color)>;

/// 悬停趋势图某天柱子的气泡:日期 + 该天各序列值(>0 才列,同 `chart_stat_list`
/// 只列有效数据的口径)。样式复用 `icons::tooltip_bubble_style`。
pub(crate) fn trend_tooltip_bubble(
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

/// 横排的序列图例:色点 + 名称一排（放在趋势图标题下方）。替代对多序列折线图
/// 用鼠悬一个个去猜颜色。
pub(crate) fn trend_legend(
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

/// 折线渲染:每个序列在各天取值处画点、相邻天连线——2026-09-14 用户要求
/// Session/Token 趋势从分组柱状图换成折线图(连续多天的走势比逐天量级对比
/// 更适合折线)。y 轴换算跟 `GridLines` 共用同一套基线(0 落在
/// `GRID_CANVAS_HEIGHT`,`BAR_MAX_HEIGHT` 撑满顶部);x 轴按天数把整块画布
/// 宽度 n 等分,点落在每天格子正中央——必须跟 `trend_line_chart` 里悬浮
/// 命中区那一排"天格子"用同一个不含 spacing 的 n 等分宽度,两层才能对上。
pub(crate) struct TrendLines {
    days: Vec<DaySeries>,
    series: TrendSeries,
    max_total: u64,
}

impl canvas::Program<Message, iced_widget::Theme, iced_renderer::Renderer> for TrendLines {
    type State = ();

    /// 光标在画布内左右移动不会改变 `mouse_interaction`(我们没重写它,一直
    /// 是默认的 `None`),而 `Canvas::update` 只在这个值变化时才请求重绘——
    /// 单靠默认行为,悬浮竖线不会跟手挪动。这里对鼠标事件显式请求重绘,让
    /// `draw` 里读到的最新 `cursor` 能画出来。
    fn update(
        &self,
        _state: &mut Self::State,
        event: &canvas::Event,
        _bounds: Rectangle,
        _cursor: iced_widget::core::mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        matches!(event, canvas::Event::Mouse(_)).then(canvas::Action::request_redraw)
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced_renderer::Renderer,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        cursor: iced_widget::core::mouse::Cursor,
    ) -> Vec<canvas::Geometry<iced_renderer::Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let n = self.days.len();
        if n == 0 || self.max_total == 0 {
            return vec![frame.into_geometry()];
        }
        let scale = BAR_MAX_HEIGHT / self.max_total as f32;
        let pitch = bounds.width / n as f32;
        let point_at = |i: usize, value: u64| {
            Point::new(
                pitch * (i as f32 + 0.5),
                GRID_CANVAS_HEIGHT - value as f32 * scale,
            )
        };

        // 悬浮竖线:替代之前每天一块矩形背景的"分割"作用(2026-09-14 用户
        // 要求去掉矩形背景,改成悬停哪天就画哪天的竖线)。落在哪天用跟
        // 折线本身同一份 `pitch` 换算,严格对齐当天的点位。
        if let Some(p) = cursor.position_in(bounds) {
            let i = ((p.x / pitch) as usize).min(n - 1);
            let x = pitch * (i as f32 + 0.5);
            frame.stroke(
                &canvas::Path::line(Point::new(x, 0.0), Point::new(x, bounds.height)),
                canvas::Stroke::default()
                    .with_color(byteui::theme::color::current().border)
                    .with_width(1.0),
            );
        }

        for (j, &(_, color)) in self.series.iter().enumerate() {
            let path = canvas::Path::new(|b| {
                for (i, day) in self.days.iter().enumerate() {
                    let point = point_at(i, day.values[j]);
                    if i == 0 {
                        b.move_to(point);
                    } else {
                        b.line_to(point);
                    }
                }
            });
            frame.stroke(
                &path,
                canvas::Stroke::default().with_color(color).with_width(2.0),
            );
            for (i, day) in self.days.iter().enumerate() {
                frame.fill(
                    &canvas::Path::circle(point_at(i, day.values[j]), 3.0),
                    color,
                );
            }
        }
        vec![frame.into_geometry()]
    }
}

/// 通用"每天并列若干序列"的趋势折线图,结构上跟 `bar_chart`（叠网格/斑马带/
/// 悬停气泡）保持一致,只是可视化层从每天一组柱子换成一条跨天连续折线。
/// `days` 每项 `values` 的下标数量须 === `series.len()`（聚合时已保证每天
/// 固定那么多个序列，空天补 0）。天格子改成不留间隙的等分宽度(`spacing(0)`
/// 配 `FillPortion(1)`),这样悬浮命中区的列中心正好跟 `TrendLines` 里按
/// 同一个 n 等分算出来的点位对齐,折线图不需要像柱状图那样靠间隙区分开
/// 相邻天(斑马纹底色已经够用)。
pub(crate) fn trend_line_chart(
    days: &[DaySeries],
    series: &TrendSeries,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let max_total = days
        .iter()
        .flat_map(|d| d.values.iter().copied())
        .max()
        .unwrap_or(1)
        .max(1);

    let lines = Canvas::new(TrendLines {
        days: days.to_vec(),
        series: series.clone(),
        max_total,
    })
    .width(Length::Fill)
    .height(Length::Fixed(GRID_CANVAS_HEIGHT));

    // 每天一个等分格子,只负责悬浮命中区(气泡)+ 日期文字——2026-09-14
    // 用户要求去掉逐天矩形背景("斑马纹"),分割感改由 `TrendLines` 里悬停
    // 时画的那条竖线承担,这里不再需要用底色区分开相邻两天。
    let mut cells = iced_widget::row![].spacing(0);
    for d in days.iter() {
        let col = column![
            iced_widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fixed(GRID_CANVAS_HEIGHT)),
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

        let cell = container(hoverable)
            .width(Length::FillPortion(1))
            .height(Length::Fixed(DAY_BAND_HEIGHT));
        cells = cells.push(cell);
    }

    stack![
        grid_lines_canvas(max_total),
        container(lines).padding(iced_widget::core::Padding {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: GRID_LABEL_GUTTER,
        }),
        container(cells).padding(iced_widget::core::Padding {
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
pub(crate) fn has_any_value(days: &[DaySeries]) -> bool {
    days.iter().any(|d| d.values.iter().any(|&v| v > 0))
}

/// 某趋势窗口内所有天、所有子序列值的总和——用于把"标签 + 总量 + 天数"揉
/// 成一行摘要式图例（如 `Input/Output(23.2m/15days)`），2026-09-16 用户要求
/// 把 Token 趋势下两张子图各自的标题/图例收成这一种格式,不再分散成色点
/// 图例 + 独立的"近 N 天"标注两处。
pub(crate) fn trend_total(days: &[DaySeries]) -> u64 {
    days.iter().flat_map(|d| d.values.iter().copied()).sum()
}

/// 单个"小节标题 +（标题右侧）序列图例 + 图表"的组合。`title` 仍走统一的
/// `home_section_head` 小结标题样式(Session 趋势 / Token 趋势…)，图例挨在它
/// 右侧对齐、不占纵向。放在节内的内层 column 统一吃 `SECTION_CHART_GAP`。
pub(crate) fn trend_chart_section(
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
            .push(trend_line_chart(days, &series))
            .into(),
    )
}

/// Session 趋势的序列配色——会话/回合各一条线,分别用 cream(项目"活动类"统计的
/// 颜色习惯)与 green;对比只在同一张图内部保色相区分,跨图可重复用色。
pub(crate) fn session_trend_series() -> TrendSeries {
    let c = byteui::theme::color::current();
    vec![("会话", c.cream), ("回合", c.green)]
}

pub(crate) fn io_trend_series() -> TrendSeries {
    let c = byteui::theme::color::current();
    vec![("Input", c.cyan), ("Output", c.purple)]
}

pub(crate) fn cache_trend_series() -> TrendSeries {
    let c = byteui::theme::color::current();
    vec![("读", c.lime), ("写", c.green)]
}

/// 一张趋势图右上角的汇总小标注(如"Input/Output(8.1m/15days)"),放在标题
/// 行里,让读者一眼看到口径,不靠猜。颜色跟 `home_section_head` 的标题文字
/// 一致用 cream——2026-09-16 用户纠正:早前这里走的是 `dim`,跟金色的
/// "Token 趋势" 大标题不一致,现在大标题已改回 cream(见 `token_trend_section`
/// 里 `home_section_head` 调用处),这个子标题也要跟着统一。
pub(crate) fn trend_window_tag(
    window: impl Into<String>,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    text(window.into())
        .size(byteui::theme::font::caption_sm())
        .color(byteui::theme::color::current().cream)
        .into()
}

/// Token 趋势区:一个标题(Token 趋势)下按 15 天(Input/Output)与 15 天
/// (Cache read/write,2026-09-14 起从 5 天统一改成 15 天,跟 Input/Output
/// 同口径)分别画两张折线图。两张图量级差太多,单独成图、各自独立纵轴 max,
/// 不共用刻度。任一图整段无数据时只画另一张;都空则整区不给。
pub(crate) fn token_trend_section(
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

    // 2026-09-16 用户纠正:这里不再走专属金色标题,改回跟 Session 趋势/每日
    // 用量统计等其余小节一致的 `home_section_head`(圆点 + cream 文字)——早前
    // 2026-09-14 用金色跟别的小节区分开的做法作废。
    let head_row = home_section_head("Token 趋势");

    // 2026-09-16 用户要求:汇总标注(子标题)靠左、色点图例(区分 Input/cyan
    // 与 Output/purple、读/lime 与 写/green)靠右,中间用 `Space::Fill` 撑开;
    // 且子标题要跟上面 "Token 趋势" 的**文字**左对齐(用 `align_to_section_title`
    // 收进 `SECTION_BODY_INSET`),不是跟它的圆点图标对齐。
    let io_header = io_has.then(|| {
        align_to_section_title(
            iced_widget::row![
                trend_window_tag(format!(
                    "Input/Output({}/{IO_TREND_WINDOW}days)",
                    format_count(trend_total(&io))
                )),
                iced_widget::Space::new()
                    .width(Length::Fill)
                    .height(Length::Shrink),
                trend_legend(&io_series),
            ]
            .align_y(iced_widget::core::alignment::Vertical::Center)
            .spacing(10)
            .into(),
        )
    });

    if !io_has && !cache_has {
        return None;
    }

    let cache_header = cache_has.then(|| {
        align_to_section_title(
            iced_widget::row![
                trend_window_tag(format!(
                    "Cache read/write({}/{CACHE_TREND_WINDOW}days)",
                    format_count(trend_total(&cache))
                )),
                iced_widget::Space::new()
                    .width(Length::Fill)
                    .height(Length::Shrink),
                trend_legend(&cache_series),
            ]
            .align_y(iced_widget::core::alignment::Vertical::Center)
            .spacing(10)
            .into(),
        )
    });

    // io 有数据时,标题(挂在 `head_row`)、它的图例行(`io_header`)和图表照旧
    // 用 `SECTION_CHART_GAP`;Cache read/write 这一小节整体跟前面的 IO 图表用
    // 更大的 `SUBSECTION_GAP` 隔开——2026-09-14 用户要求这个小标题上下更疏朗
    // 一些,让它读起来是独立的第二个小节,而不是紧贴着上一张图。io 缺失时
    // (只剩 Cache)标题仍直接挂在 `head_row` 上,跟自己的图表保持普通的
    // `SECTION_CHART_GAP`。
    let section: Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match (io_has, io_header, cache_header) {
            (true, Some(io_header), Some(cache_header)) => column![
                column![head_row, io_header, trend_line_chart(&io, &io_series)]
                    .spacing(SECTION_CHART_GAP),
                column![cache_header, trend_line_chart(&cache, &cache_series)]
                    .spacing(SECTION_CHART_GAP),
            ]
            .spacing(SUBSECTION_GAP)
            .into(),
            (true, Some(io_header), None) => {
                column![head_row, io_header, trend_line_chart(&io, &io_series)]
                    .spacing(SECTION_CHART_GAP)
                    .into()
            }
            (false, _, Some(cache_header)) => column![
                head_row,
                cache_header,
                trend_line_chart(&cache, &cache_series)
            ]
            .spacing(SECTION_CHART_GAP)
            .into(),
            (false, _, None) => unreachable!("!io_has && !cache_has 已在上面提前返回"),
            (true, None, _) => {
                unreachable!("io_header 与 io_has 同源于 `.then()`,io_has 为真时必为 Some")
            }
        };
    Some(section)
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
pub(crate) fn format_count<N: Into<u64>>(n: N) -> String {
    let n: u64 = n.into();
    if n >= 1_000_000 {
        format!("{:.1}m", n as f32 / 1_000_000.0)
    } else if n >= 1000 {
        format!("{:.1}k", n as f32 / 1000.0)
    } else {
        n.to_string()
    }
}

pub(crate) const PIE_RADIUS: f32 = 52.0;
pub(crate) const PIE_GAP_RAD: f32 = 0.035;

/// 用量面板"小节(mid)"标题与它正下方图表之间的纵向间距。小节标题与图表包进
/// 内层 column 单独吃这个值(见 `content_pane`),与面板外层常规的 `spacing(12)`
/// 解耦——2026-09-05 产品要求"加大每一节标题和 chart 的间距"。
pub(crate) const SECTION_CHART_GAP: f32 = 24.0;

/// `token_trend_section` 内 IO 图表与 "Cache read/write" 子小节之间的组间距,
/// 比 `SECTION_CHART_GAP` 更疏朗——2026-09-14 用户要求"Cache read/write"这个
/// 子标题上下更宽松,让它读起来是独立的第二个小节,不是紧贴上一张图表。
pub(crate) const SUBSECTION_GAP: f32 = 32.0;

/// 子栏目标题 `home_section_head` 由「圆点图标 + 6px 间距 + 标题文字」组成,
/// 标题文字相对该行起点缩进 `icon_size::row() + 6`。每个小节标题下的卡片/图表
/// 若要和标题**文字**左对齐(而不是跟圆点起点),内容同样要收走这么远,否则视觉
/// 上卡片总比标题突出一截。2026-09-07:用量面板三个小节(项目 / Agent / 每日)
/// 的卡组统一用它对齐到标题文字。
pub(crate) const SECTION_BODY_INSET: fn() -> f32 = || byteui::theme::icon_size::row() + 6.0;

/// 把某个紧跟在 `home_section_head` 之下的图表/卡片组整体左收 `SECTION_BODY_INSET`,
/// 使它的左边缘与上一行标题文字起点对齐。
pub(crate) fn align_to_section_title<'a>(
    body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    row![
        iced_widget::Space::new().width(Length::Fixed(SECTION_BODY_INSET())),
        container(body).width(Length::Fill),
    ]
    .width(Length::Fill)
    .spacing(0)
    .into()
}

pub(crate) struct PieChart {
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

pub(crate) fn pie_chart(
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
pub(crate) fn share_total(share: &[(AgentKind, u64)]) -> u64 {
    share.iter().map(|(_, v)| v).sum()
}

pub(crate) fn agent_metric_group(
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
pub(crate) fn pair_metric_cells<'a>(
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
pub(crate) fn metric_group_banner(
    label: &'static str,
    total: u64,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let c = byteui::theme::color::current();
    iced_widget::row![
        text(label)
            .size(byteui::theme::font::label())
            .color(c.cream),
        text(format!("{} total", format_count(total)))
            .size(byteui::theme::font::caption())
            .color(c.cream)
            .font(iced_widget::core::Font::MONOSPACE),
    ]
    .spacing(8)
    .into()
}

/// Agent 用量的列表式呈现,配在饼图右边当图例:标题行 `{title}(总数)` +
/// 逐 agent `agent - 数量(百分比%)`(2026-08-28 用户反馈:圆环不能去掉,只是
/// 把图例的文字格式换成这种更直接的数字表——取代原来的 `chart_legend`/
/// `chart_label` 文字格式,饼图本体保留。标题行与列表间的 1px 分隔线于
/// 2026-09-05 用户要求去掉,标题后直接接数字表)。百分比用整数除法截断、
/// 不四舍五入,跟原图例的算法保持一致,避免几档相加超过 100%。
pub(crate) fn chart_stat_list(
    title: &'static str,
    share: &[(AgentKind, u64)],
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let total: u64 = share.iter().map(|(_, v)| v).sum();
    let dim = byteui::theme::color::current().dim;
    let mut col = column![
        text(format!("{title}({})", format_count(total)))
            .size(byteui::theme::font::caption())
            .color(dim)
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
