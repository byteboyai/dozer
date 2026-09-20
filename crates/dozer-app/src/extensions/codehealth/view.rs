//! 代码健康度面板渲染：健康卡片（等级徽章 + 摘要 + 扫描按钮）+ 问题列表
//! （按文件分组、按严重度排序的函数级排行榜）。单块可滚动内容，不像
//! `usage`/`conversations`/`agent` 那样有独立的 list pane + 分栏
//! （spec「UI 设计」：两块 UI 堆叠展示，不做三个独立视图）。

use super::{Message, WorkspaceState};
use byteui::theme::color::ColorTokens;
use dozer_codehealth::{FunctionMetric, HealthTier, ProjectReport, Severity};
use iced_widget::core::{Element, Length};
use iced_widget::{Column, button, column, container, mouse_area, row, scrollable, text};
use std::collections::BTreeMap;

fn tier_color(tier: HealthTier, tokens: &ColorTokens) -> iced_widget::core::Color {
    match tier {
        HealthTier::Healthy => tokens.green,
        HealthTier::Watch => tokens.cyan,
        HealthTier::Critical => tokens.red,
    }
}

fn tier_label(tier: HealthTier) -> &'static str {
    match tier {
        HealthTier::Healthy => "健康",
        HealthTier::Watch => "关注",
        HealthTier::Critical => "警戒",
    }
}

fn health_card(
    report: &ProjectReport,
    scanned_at_ms: Option<u64>,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let badge_color = tier_color(report.overall_tier, &tokens);
    let summary = format!(
        "核心代码 {} 行，{} 个函数存在明显结构问题",
        report.total_loc, report.critical_functions
    );
    let scanned_at = match scanned_at_ms {
        Some(ms) => format!("上次扫描：{}", format_ms(ms)),
        None => "尚未扫描".to_string(),
    };
    column![
        row![
            text(tier_label(report.overall_tier))
                .size(20)
                .color(badge_color),
            text(summary).size(14).color(tokens.body),
        ]
        .spacing(12),
        row![
            text(scanned_at).size(12).color(tokens.dim),
            button(text("扫描").size(13)).on_press(Message::ScanRequested),
        ]
        .spacing(12),
    ]
    .spacing(8)
    .padding(16)
    .into()
}

/// 同 `git_log.rs::format_commit_time` 的处理方式：展示 UTC，不做本地
/// 时区换算——扫描时间戳是纯展示态，UTC 足够，不为此引入时区库。
fn format_ms(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let days = secs / 86_400;
    let secs_of_day = secs % 86_400;
    let (h, m, s) = (secs_of_day / 3600, (secs_of_day / 60) % 60, secs_of_day % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02} UTC")
}

/// Howard Hinnant 的 `civil_from_days` 算法：Unix epoch 起的天数 → (年, 月, 日)。
/// 范围覆盖 1970..=2100。与 `git_log.rs`/`todo.rs` 的同名函数同源，第三份
/// "照抄一份"（那两处已经说明过：跨模块复用一个几行的纯函数不值得引入耦合）。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

fn problem_row(
    f: &FunctionMetric,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let color = match f.severity {
        Severity::Critical => tokens.red,
        Severity::Watch => tokens.cyan,
        Severity::Normal => tokens.dim,
    };
    let content = row![
        text(&f.name).size(13).color(tokens.cream),
        text(format!("complexity={} loc={}", f.complexity_signal, f.loc))
            .size(12)
            .color(color),
    ]
    .spacing(8)
    .padding([2, 8]);
    mouse_area(content)
        .on_press(Message::OpenLocation(f.file.clone(), f.start_line))
        .into()
}

fn problem_list(
    report: &ProjectReport,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let mut by_file: BTreeMap<&std::path::Path, Vec<&FunctionMetric>> = BTreeMap::new();
    for f in &report.functions {
        by_file.entry(f.file.as_path()).or_default().push(f);
    }
    let mut col = Column::new().spacing(4);
    for (path, functions) in by_file {
        col = col.push(text(path.display().to_string()).size(12).color(tokens.dim));
        for f in functions {
            col = col.push(problem_row(f));
        }
    }
    scrollable(col.padding(16)).into()
}

fn error_banner<'a>(
    message: &'a str,
    tokens: &ColorTokens,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    text(format!("扫描失败，请重试：{message}"))
        .size(12)
        .color(tokens.red)
        .into()
}

pub fn content_pane(
    ws_state: &WorkspaceState,
    width: Length,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let error = ws_state.scan_error().map(|e| error_banner(e, &tokens));

    if ws_state.scanning() {
        // 扫描中：若已有上一次结果，仍展示它（不清空），只在顶部叠一条
        // "扫描中…"提示——比整块换成 loading 占位更不容易让用户以为数据
        // 丢了；若从没扫描过（`report()` 为 `None`），只显示 loading 提示。
        let scanning_text = text("扫描中…").size(14).color(tokens.body);
        return match ws_state.report() {
            Some(report) => container(column![
                scanning_text,
                health_card(report, ws_state.scanned_at_ms()),
                problem_list(report),
            ])
            .width(width)
            .into(),
            None => container(column![scanning_text]).width(width).into(),
        };
    }

    let Some(report) = ws_state.report() else {
        let mut col = column![
            text("这个项目还没有可分析的 Rust 代码，或者还没有扫描过。")
                .size(14)
                .color(tokens.body),
            button(text("扫描")).on_press(Message::ScanRequested),
        ]
        .spacing(12);
        if let Some(err) = error {
            col = col.push(err);
        }
        return container(col.padding(16)).width(width).into();
    };

    let mut col = column![health_card(report, ws_state.scanned_at_ms())];
    if let Some(err) = error {
        col = col.push(err);
    }
    col = col.push(problem_list(report));
    container(col).width(width).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_ms_matches_expected_layout() {
        // 2026-09-20 07:58:06 UTC（真实扫描数据的时间戳，固定输入 → 固定输出）。
        assert_eq!(format_ms(1_789_891_086_991), "2026-09-20 07:58:06 UTC");
        // epoch 0 边界。
        assert_eq!(format_ms(0), "1970-01-01 00:00:00 UTC");
    }
}
