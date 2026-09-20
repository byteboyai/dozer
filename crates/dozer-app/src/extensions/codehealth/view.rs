//! 代码健康度面板渲染：健康卡片（等级徽章 + 规模/密度双分档 + 扫描按钮）
//! + 问题列表（只列 Watch/Critical、按严重度+复杂度排序的扁平排行榜，
//! 2026-09-20 UI 优化起不再按文件分组）。单块可滚动内容，不像
//! `usage`/`conversations`/`agent` 那样有独立的 list pane + 分栏
//! （spec「UI 设计」：两块 UI 堆叠展示，不做三个独立视图）。

use super::{Message, WorkspaceState};
use byteui::theme::color::ColorTokens;
use dozer_codehealth::{FunctionMetric, HealthTier, ProjectReport, Severity};
use iced_widget::core::{Element, Length};
use iced_widget::{Column, button, column, container, mouse_area, row, scrollable, text};

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

fn density_pct(critical: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        critical as f64 / total as f64 * 100.0
    }
}

fn scale_summary(report: &ProjectReport) -> String {
    format!(
        "规模：{}（{} 行）",
        tier_label(report.scale_tier),
        report.total_loc
    )
}

fn density_summary(report: &ProjectReport) -> String {
    let pct = density_pct(report.critical_functions, report.total_functions);
    format!(
        "密度：{}（{}/{}，约 {pct:.1}%）",
        tier_label(report.density_tier),
        report.critical_functions,
        report.total_functions
    )
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
            text(scale_summary(report))
                .size(12)
                .color(tier_color(report.scale_tier, &tokens)),
            text(density_summary(report))
                .size(12)
                .color(tier_color(report.density_tier, &tokens)),
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
    let (h, m, s) = (
        secs_of_day / 3600,
        (secs_of_day / 60) % 60,
        secs_of_day % 60,
    );
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

fn severity_rank(s: Severity) -> u8 {
    match s {
        Severity::Critical => 2,
        Severity::Watch => 1,
        Severity::Normal => 0,
    }
}

/// 过滤掉 `Severity::Normal`、按严重度降序 + 复杂度降序排出扁平排行榜。
/// `report.functions` 本身已经按严重度排过（`dozer-codehealth::report::scan_project`
/// 的契约，供 dozer-mcp 未来复用全量数据，这里不改那份排序，只在展示层重排一份
/// 过滤后的视图）。`severity_rank` 是本文件私有的排名映射，不是
/// `dozer_codehealth::FunctionMetric` 的 `pub(crate)` 方法（那个跨 crate 不可见）。
fn ranked_problems(report: &ProjectReport) -> Vec<&FunctionMetric> {
    let mut v: Vec<&FunctionMetric> = report
        .functions
        .iter()
        .filter(|f| f.severity != Severity::Normal)
        .collect();
    v.sort_by_key(|f| {
        (
            std::cmp::Reverse(severity_rank(f.severity)),
            std::cmp::Reverse(f.complexity_signal),
        )
    });
    v
}

fn problem_row<'a>(
    f: &'a FunctionMetric,
    tokens: ColorTokens,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (label, color) = match f.severity {
        Severity::Critical => ("警戒", tokens.red),
        Severity::Watch => ("关注", tokens.cyan),
        Severity::Normal => unreachable!("ranked_problems 已过滤掉 Normal"),
    };
    let content = column![
        row![
            text(label).size(11).color(color),
            text(&f.name).size(13).color(tokens.cream),
            text(format!("complexity={} loc={}", f.complexity_signal, f.loc))
                .size(12)
                .color(color),
        ]
        .spacing(8),
        text(f.file.display().to_string())
            .size(11)
            .color(tokens.dim),
    ]
    .spacing(2)
    .padding([4, 8]);
    mouse_area(content)
        .on_press(Message::OpenLocation(f.file.clone(), f.start_line))
        .into()
}

fn problem_list(
    report: &ProjectReport,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let ranked = ranked_problems(report);
    if ranked.is_empty() {
        return container(
            text("没有发现结构复杂的函数，代码整体健康。")
                .size(13)
                .color(tokens.dim),
        )
        .padding(16)
        .into();
    }
    let mut col = Column::new().spacing(4);
    for f in ranked {
        col = col.push(problem_row(f, tokens));
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

    fn sample_report(
        total_loc: usize,
        scale_tier: HealthTier,
        critical_functions: usize,
        total_functions: usize,
        density_tier: HealthTier,
    ) -> ProjectReport {
        ProjectReport {
            total_loc,
            total_functions,
            critical_functions,
            scale_tier,
            density_tier,
            overall_tier: scale_tier.max(density_tier),
            functions: Vec::new(),
        }
    }

    #[test]
    fn density_pct_computes_percentage() {
        assert!((density_pct(9, 3890) - 0.231_362_47).abs() < 1e-6);
    }

    #[test]
    fn density_pct_zero_total_avoids_div_by_zero() {
        assert_eq!(density_pct(0, 0), 0.0);
    }

    #[test]
    fn scale_summary_formats_loc_and_label() {
        let report = sample_report(101_052, HealthTier::Critical, 9, 3890, HealthTier::Healthy);
        assert_eq!(scale_summary(&report), "规模：警戒（101052 行）");
    }

    #[test]
    fn density_summary_formats_ratio_and_label() {
        let report = sample_report(101_052, HealthTier::Critical, 9, 3890, HealthTier::Healthy);
        assert_eq!(density_summary(&report), "密度：健康（9/3890，约 0.2%）");
    }

    fn metric(name: &str, severity: Severity, complexity_signal: usize) -> FunctionMetric {
        FunctionMetric {
            name: name.to_string(),
            file: std::path::PathBuf::from("a.rs"),
            start_line: 1,
            end_line: 2,
            loc: 2,
            complexity_signal,
            severity,
        }
    }

    fn report_with_functions(functions: Vec<FunctionMetric>) -> ProjectReport {
        ProjectReport {
            total_loc: 0,
            total_functions: functions.len(),
            critical_functions: functions
                .iter()
                .filter(|f| f.severity == Severity::Critical)
                .count(),
            scale_tier: HealthTier::Healthy,
            density_tier: HealthTier::Healthy,
            overall_tier: HealthTier::Healthy,
            functions,
        }
    }

    #[test]
    fn severity_rank_orders_critical_highest() {
        assert!(severity_rank(Severity::Critical) > severity_rank(Severity::Watch));
        assert!(severity_rank(Severity::Watch) > severity_rank(Severity::Normal));
    }

    #[test]
    fn ranked_problems_filters_out_normal() {
        let report = report_with_functions(vec![
            metric("a", Severity::Normal, 5),
            metric("b", Severity::Watch, 20),
        ]);
        let ranked = ranked_problems(&report);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].name, "b");
    }

    #[test]
    fn ranked_problems_sorts_by_severity_then_complexity_desc() {
        let report = report_with_functions(vec![
            metric("watch_low", Severity::Watch, 16),
            metric("critical_low", Severity::Critical, 41),
            metric("critical_high", Severity::Critical, 255),
            metric("watch_high", Severity::Watch, 40),
        ]);
        let ranked = ranked_problems(&report);
        let names: Vec<&str> = ranked.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["critical_high", "critical_low", "watch_high", "watch_low"]
        );
    }

    #[test]
    fn ranked_problems_empty_when_all_normal() {
        let report = report_with_functions(vec![metric("a", Severity::Normal, 0)]);
        assert!(ranked_problems(&report).is_empty());
    }
}
