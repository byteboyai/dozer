//! 代码健康度面板渲染（spec 2026-09-21）。右侧分类导航（总览 / 结构复杂度 /
//! UI 一致性 / 扫描范围），左侧内容区按当前分类切换。渲染层只消费
//! `view_model` 与 `WorkspaceState` 已经格式化好的数据，不再自拼业务判断。

use super::view_model::{self, ChangeSummary, EmptyState};
use super::{CodeHealthCategory, Message, StructureFilter, WorkspaceState};
use byteui::interaction::icons;
use byteui::theme::color::ColorTokens;
use dozer_codehealth::{FindingChange, FindingSeverity, HealthTier, ProjectReport, rule_ids};
use iced_widget::core::{Border, Color, Element, Length};
use iced_widget::{Column, button, column, container, mouse_area, row, scrollable, space, text};

fn tier_color(tier: HealthTier, tokens: &ColorTokens) -> Color {
    match tier {
        HealthTier::Healthy => tokens.green,
        HealthTier::Watch => tokens.cyan,
        HealthTier::Critical => tokens.red,
    }
}

fn tier_label(tier: HealthTier) -> &'static str {
    match tier {
        HealthTier::Healthy => "健康",
        HealthTier::Watch => "需要关注",
        HealthTier::Critical => "警戒",
    }
}

fn sev_label(s: FindingSeverity) -> &'static str {
    match s {
        FindingSeverity::Critical => "警戒",
        FindingSeverity::Watch => "关注",
    }
}

fn sev_color(s: FindingSeverity, tokens: &ColorTokens) -> Color {
    match s {
        FindingSeverity::Critical => tokens.red,
        FindingSeverity::Watch => tokens.cyan,
    }
}

fn change_label(c: FindingChange) -> &'static str {
    match c {
        FindingChange::New => "本轮新增",
        FindingChange::Worsened => "本轮恶化",
        FindingChange::Improved => "本轮改善",
        FindingChange::Persisting => "持续存在",
        FindingChange::Resolved => "已解决",
    }
}

/// 同 `git_log.rs::format_commit_time`：展示 UTC，不引入时区库。
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

// ---------------------------------------------------------------------------
// 通用行渲染
// ---------------------------------------------------------------------------

/// 一条统一发现行：严重度徽章 + 标题 + 位置 + 变化原因。点击跳转文件。
fn finding_row(
    title: String,
    path: std::path::PathBuf,
    line: usize,
    severity: FindingSeverity,
    change: Option<FindingChange>,
    reasons: &[String],
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let mut row_elems = Column::new().spacing(2);
    let mut head = row![
        text(sev_label(severity))
            .size(11)
            .color(sev_color(severity, &tokens)),
        text(title).size(13).color(tokens.cream),
    ]
    .spacing(8);
    if let Some(c) = change {
        head = head.push(text(change_label(c)).size(11).color(tokens.dim));
    }
    row_elems = row_elems.push(head);
    let mut sub = row![
        text(format!("{}:{}", path.display(), line))
            .size(11)
            .color(tokens.dim),
    ]
    .spacing(8);
    if !reasons.is_empty() {
        sub = sub.push(text(reasons.join(" · ")).size(11).color(tokens.dim));
    }
    row_elems = row_elems.push(sub);

    let content = row_elems.padding([4, 8]);
    mouse_area(content)
        .on_press(Message::OpenLocation(path, line))
        .into()
}

// ---------------------------------------------------------------------------
// 总览
// ---------------------------------------------------------------------------

fn status_card(
    report: &ProjectReport,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let badge = tier_color(report.overall_tier, &tokens);
    let summary = format!(
        "核心代码 {} 行 · {} 个函数存在明显结构问题",
        report.total_loc, report.critical_functions
    );
    column![
        text(tier_label(report.overall_tier)).size(20).color(badge),
        text(summary).size(13).color(tokens.body),
    ]
    .spacing(6)
    .padding(16)
    .into()
}

fn change_card(
    ws: &WorkspaceState,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let Some(report) = ws.report() else {
        return column![].into();
    };
    let summary = view_model::change_summary(report, ws.previous_report(), ws.diff());
    match summary {
        ChangeSummary::FirstScan => column![
            text("本次变化").size(14).color(tokens.body),
            text("首次扫描，暂无历史可比对").size(12).color(tokens.dim),
        ]
        .spacing(4)
        .padding(16)
        .into(),
        ChangeSummary::HasChange(c) => {
            let delta_line = |label: &str, delta: i64| {
                format!("{label} {}{}", if delta > 0 { "+" } else { "" }, delta)
            };
            column![
                text("本次变化").size(14).color(tokens.body),
                row![
                    text(delta_line("代码行", c.loc_delta))
                        .size(12)
                        .color(tokens.body),
                    text(delta_line("函数数", c.functions_delta))
                        .size(12)
                        .color(tokens.body),
                ]
                .spacing(12),
                row![
                    text(format!("新增风险 +{}", c.new_risks))
                        .size(12)
                        .color(tokens.red),
                    text(format!("已解决风险 {}", c.resolved_risks))
                        .size(12)
                        .color(tokens.green),
                ]
                .spacing(12),
            ]
            .spacing(6)
            .padding(16)
            .into()
        }
    }
}

fn priority_list(
    ws: &WorkspaceState,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let rows = view_model::hotspot_rows(ws.hotspots(), 50);
    if rows.is_empty() {
        return container(text("暂无优先处理项。").size(13).color(tokens.dim))
            .padding(16)
            .into();
    }
    let mut col = Column::new().spacing(6);
    for r in rows {
        let row_el = finding_row(
            r.title,
            r.path,
            r.line,
            r.severity,
            Some(r.change),
            &r.reasons,
        );
        let analyze = button(text("交给 Agent 分析").size(11))
            .on_press(Message::AnalyzeFinding(r.finding_id.clone()))
            .padding([2, 8]);
        col = col.push(column![row_el, analyze].spacing(2));
    }
    col.padding(16).into()
}

fn overview_content(
    ws: &WorkspaceState,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let Some(report) = ws.report() else {
        return empty_content(ws);
    };

    // 标题 + Git 分支/状态。
    let mut title_row = row![text("代码健康度总览").size(16).color(tokens.body)].spacing(12);
    if let Some(git) = ws.git() {
        let branch = git.branch.clone().unwrap_or_else(|| "detached".to_string());
        let head = git.head.clone().unwrap_or_default();
        let mut status_text = format!("{branch} · {head}");
        if git.dirty {
            status_text.push_str(" · 有未提交改动");
        }
        title_row = title_row.push(text(status_text).size(11).color(tokens.dim));
    }

    // 扫描可信度摘要（点击进入扫描范围分类）。
    let scope = view_model::scope_summary(ws);
    let scope_text = scope
        .map(|s| {
            format!(
                "已分析 {} 个文件 · {} · 排除 {} · 跳过 {}",
                s.analyzed_files, s.languages, s.excluded_files, s.skipped_files
            )
        })
        .unwrap_or_default();

    let legacy_note = report
        .schema_version
        .checked_sub(0)
        .and_then(|_| view_model::legacy_report_note(report.schema_version));

    let mut col = column![
        title_row,
        status_card(report),
        change_card(ws),
        text("优先处理").size(14).color(tokens.body),
        priority_list(ws),
        text(format!("扫描可信度：{scope_text}"))
            .size(11)
            .color(tokens.dim),
    ]
    .spacing(8);
    if let Some(note) = legacy_note {
        col = col.push(text(note).size(12).color(tokens.cyan));
    }
    col.into()
}

// ---------------------------------------------------------------------------
// 结构复杂度
// ---------------------------------------------------------------------------

fn structure_content(
    ws: &WorkspaceState,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let Some(report) = ws.report() else {
        return empty_content(ws);
    };

    let filter = ws.structure_filter();
    let is_new = |f: &dozer_codehealth::Finding| -> bool {
        ws.diff()
            .map(|d| d.new.iter().any(|x| x.id == f.id) || d.worsened.iter().any(|x| x.id == f.id))
            .unwrap_or(false)
    };

    let findings: Vec<&dozer_codehealth::Finding> = report
        .findings
        .iter()
        .filter(|f| f.rule_id == rule_ids::STRUCTURE_COMPLEXITY)
        .filter(|f| match filter {
            StructureFilter::New => is_new(f),
            StructureFilter::All => true,
        })
        .collect();

    let mut col = column![
        row![
            text("结构复杂度").size(16).color(tokens.body),
            text("指标口径：控制流信号（非标准圈复杂度）")
                .size(11)
                .color(tokens.dim),
        ]
        .spacing(12),
        filter_buttons(filter),
    ]
    .spacing(8);

    if findings.is_empty() {
        let empty = if filter == StructureFilter::New {
            "本轮没有新增的结构复杂度发现。"
        } else {
            "没有发现结构复杂的函数。"
        };
        col = col.push(text(empty).size(13).color(tokens.dim));
    } else {
        for f in findings {
            let change = ws.diff().map(|d| {
                if d.new.iter().any(|x| x.id == f.id) {
                    FindingChange::New
                } else if d.worsened.iter().any(|x| x.id == f.id) {
                    FindingChange::Worsened
                } else if d.improved.iter().any(|x| x.id == f.id) {
                    FindingChange::Improved
                } else {
                    FindingChange::Persisting
                }
            });
            let metric = match &f.evidence {
                dozer_codehealth::FindingEvidence::Structure {
                    complexity_signal, ..
                } => *complexity_signal,
                _ => 0,
            };
            col = col.push(finding_row(
                format!(
                    "{} 控制流信号 {}",
                    f.symbol.as_deref().unwrap_or("?"),
                    metric
                ),
                f.path.clone(),
                f.start_line,
                f.severity,
                change,
                &[],
            ));
        }
    }
    col.spacing(8).padding(16).into()
}

fn filter_buttons(
    current: StructureFilter,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let c = byteui::theme::color::current();
    let btn = |label: &'static str, filter: StructureFilter| {
        let active = filter == current;
        let fg = if active { c.cream } else { c.dim };
        button(text(label).size(12))
            .on_press(Message::StructureFilterSet(filter))
            .padding([4, 10])
            .style(move |_t: &iced_widget::Theme, _s| button::Style {
                background: if active { Some(c.card.into()) } else { None },
                text_color: fg,
                border: Border {
                    color: if active { c.gold } else { Color::TRANSPARENT },
                    width: if active { 1.0 } else { 0.0 },
                    radius: 6.0.into(),
                },
                ..button::Style::default()
            })
    };
    row![
        btn("本轮新增", StructureFilter::New),
        btn("全部", StructureFilter::All)
    ]
    .spacing(8)
    .into()
}

// ---------------------------------------------------------------------------
// UI 一致性
// ---------------------------------------------------------------------------

fn ui_consistency_content(
    ws: &WorkspaceState,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let Some(report) = ws.report() else {
        return empty_content(ws);
    };

    if !view_model::ui_consistency_applicable(report) {
        return container(
            text("UI 一致性检测适用于 iced/Rust，当前项目未识别到适用框架。")
                .size(13)
                .color(tokens.dim),
        )
        .padding(16)
        .into();
    }

    let rules: &[(&str, &str)] = &[
        ("颜色硬编码", rule_ids::COLOR_HARDCODE),
        ("边距硬编码", rule_ids::SPACING_HARDCODE),
        ("字体硬编码", rule_ids::FONT_HARDCODE),
        ("组件树嵌套深度", rule_ids::NESTING_DEPTH),
        ("事件回调密度", rule_ids::EVENT_HANDLER_DENSITY),
        ("组件化重复结构", rule_ids::DUPLICATE_STRUCTURE),
    ];

    let mut col = column![text("UI 一致性").size(16).color(tokens.body)].spacing(8);
    for (title, rule) in rules {
        let findings: Vec<&dozer_codehealth::Finding> = report
            .findings
            .iter()
            .filter(|f| f.rule_id == *rule)
            .collect();
        let count = findings.len();
        let header = format!("{title}：{count} 处");
        col = col.push(text(header).size(13).color(tokens.body));
        for f in findings {
            col = col.push(finding_row(
                f.title.clone(),
                f.path.clone(),
                f.start_line,
                f.severity,
                None,
                &[],
            ));
        }
        if count == 0 {
            col = col.push(text("无发现").size(11).color(tokens.dim));
        }
    }
    col.padding(16).into()
}

// ---------------------------------------------------------------------------
// 扫描范围
// ---------------------------------------------------------------------------

fn scan_scope_content(
    ws: &WorkspaceState,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let Some(report) = ws.report() else {
        return empty_content(ws);
    };
    let scan = &report.scan;

    let status = match scan.status {
        dozer_codehealth::ScanStatus::Complete => "完整",
        dozer_codehealth::ScanStatus::Partial => "部分完成",
        dozer_codehealth::ScanStatus::Failed => "失败",
    };

    let languages = scan
        .languages
        .iter()
        .map(|l| {
            if l.analyzed {
                format!("{}（{} 个文件，结构分析）", l.language, l.files)
            } else {
                format!("{}（{} 个文件，仅统计）", l.language, l.files)
            }
        })
        .collect::<Vec<_>>()
        .join(" · ");

    let mut col = column![
        text("扫描范围").size(16).color(tokens.body),
        text(format!("扫描状态：{status}"))
            .size(13)
            .color(tokens.body),
        text(format!(
            "已分析 {} 个文件 · 排除 {} 个 · 跳过 {} 个",
            scan.analyzed_files,
            scan.excluded_files,
            scan.skipped_files.len()
        ))
        .size(13)
        .color(tokens.body),
        text(format!("已发现语言：{languages}"))
            .size(12)
            .color(tokens.dim),
        text(format!("扫描耗时：{} ms", scan.duration_ms))
            .size(12)
            .color(tokens.dim),
        text(format!("报告版本：schema v{}", report.schema_version))
            .size(12)
            .color(tokens.dim),
    ]
    .spacing(8);

    if let Some(git) = ws.git() {
        let baseline = format!(
            "Git 基准：{}{}",
            git.branch.clone().unwrap_or_else(|| "detached".into()),
            git.head
                .clone()
                .map(|h| format!(" @ {h}"))
                .unwrap_or_default()
        );
        col = col.push(text(baseline).size(12).color(tokens.dim));
    }

    if !scan.skipped_files.is_empty() {
        col = col.push(text("跳过/排除明细：").size(13).color(tokens.body));
        for s in &scan.skipped_files {
            let reason = match s.reason {
                dozer_codehealth::SkipReason::Ignored => "忽略规则",
                dozer_codehealth::SkipReason::UnsupportedLanguage => "不支持的语言",
                dozer_codehealth::SkipReason::Generated => "生成文件",
                dozer_codehealth::SkipReason::NonUtf8 => "非 UTF-8",
                dozer_codehealth::SkipReason::ReadFailed => "读取失败",
                dozer_codehealth::SkipReason::ParseFailed => "解析失败",
            };
            col = col.push(
                text(format!("{}（{reason}）", s.path.display()))
                    .size(11)
                    .color(tokens.dim),
            );
        }
    }

    col.padding(16).into()
}

// ---------------------------------------------------------------------------
// 空状态
// ---------------------------------------------------------------------------

fn empty_content(
    ws: &WorkspaceState,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let msg = match view_model::empty_state(ws) {
        EmptyState::NeverScanned => "这个项目还没有扫描过。",
        EmptyState::NoSupportedCode => "没有发现受支持的代码。",
        EmptyState::PartialFailure => "扫描部分完成，部分文件失败（见扫描范围）。",
        EmptyState::ScanFailed => "扫描失败。",
    };
    container(text(msg).size(14).color(tokens.body))
        .padding(16)
        .into()
}

fn error_banner(
    message: &str,
    tokens: &ColorTokens,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    text(format!("扫描失败，请重试：{message}"))
        .size(12)
        .color(tokens.red)
        .into()
}

// ---------------------------------------------------------------------------
// 分类导航 + 内容区
// ---------------------------------------------------------------------------

fn category_icon(category: CodeHealthCategory) -> icons::IconKind {
    match category {
        CodeHealthCategory::Overview => icons::IconKind::BarChart3,
        CodeHealthCategory::Structure => icons::IconKind::FileCode,
        CodeHealthCategory::UiConsistency => icons::IconKind::LayoutList,
        CodeHealthCategory::ScanScope => icons::IconKind::Search,
    }
}

fn category_button(
    category: CodeHealthCategory,
    current: CodeHealthCategory,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let active = category == current;
    let c = byteui::theme::color::current();
    let fg = if active { c.cream } else { c.dim };
    let icon_color = if active { c.gold } else { c.dim };
    button(
        row![
            icons::view(
                category_icon(category),
                byteui::theme::icon_size::row(),
                icon_color
            ),
            text(category.label())
                .size(byteui::theme::font::body())
                .color(fg),
            space::Space::new()
                .width(Length::Fill)
                .height(Length::Shrink),
        ]
        .spacing(8)
        .align_y(iced_widget::core::Alignment::Center),
    )
    .on_press(Message::CategorySet(category))
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

pub fn list_pane(
    ws_state: &WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let header = container(crate::chrome::homespace::home_panel_head(
        icons::IconKind::SquareActivity,
        "代码健康度",
    ))
    .padding(iced_widget::core::Padding {
        top: 12.0,
        right: 12.0,
        bottom: 8.0,
        left: 12.0,
    });

    let current = ws_state.category();
    let mut nav = column![].spacing(4).padding(iced_widget::core::Padding {
        top: 0.0,
        right: 8.0,
        bottom: 12.0,
        left: 8.0,
    });
    for category in CodeHealthCategory::all() {
        nav = nav.push(category_button(category, current));
    }

    container(column![header, nav])
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().bg.into()),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

fn scan_header(
    scanned_at_ms: Option<u64>,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let scanned_at = match scanned_at_ms {
        Some(ms) => format!("上次扫描：{}", format_ms(ms)),
        None => "尚未扫描".to_string(),
    };
    row![
        text(scanned_at).size(12).color(tokens.dim),
        button(text("扫描").size(13)).on_press(Message::ScanRequested),
    ]
    .spacing(12)
    .padding([12, 16])
    .into()
}

/// 内容列原生壳:webview 盖在这块 `container` 之上(同 Files/Usage 现状),
/// 它只负责面板背景/边框;`failed` 为 `Some` 时 webview 不挂载,这里显示
/// 失败原因与"重试"。
pub fn content_pane(
    failed: Option<&str>,
    width: Length,
    outer: Border,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let body: Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> = match failed
    {
        Some(reason) => column![
            text("代码健康度页面加载失败").size(14).color(tokens.body),
            text(reason.to_string()).size(12).color(tokens.dim),
            button(text("重试").size(13)).on_press(Message::ContentRetry),
        ]
        .spacing(10)
        .padding(16)
        .into(),
        None => column![].into(),
    };
    container(body)
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().panel.into()),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

fn category_content(
    category: CodeHealthCategory,
    ws_state: &WorkspaceState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    match category {
        CodeHealthCategory::Overview => overview_content(ws_state),
        CodeHealthCategory::Structure => structure_content(ws_state),
        CodeHealthCategory::UiConsistency => ui_consistency_content(ws_state),
        CodeHealthCategory::ScanScope => scan_scope_content(ws_state),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_ms_matches_expected_layout() {
        assert_eq!(format_ms(1_789_891_086_991), "2026-09-20 07:58:06 UTC");
        assert_eq!(format_ms(0), "1970-01-01 00:00:00 UTC");
    }

    #[test]
    fn tier_labels_include_text_for_all_states() {
        assert_eq!(tier_label(HealthTier::Healthy), "健康");
        assert_eq!(tier_label(HealthTier::Watch), "需要关注");
        assert_eq!(tier_label(HealthTier::Critical), "警戒");
    }

    #[test]
    fn category_labels_and_all_cover_four_categories() {
        assert_eq!(CodeHealthCategory::all().len(), 4);
        assert_eq!(CodeHealthCategory::Overview.label(), "总览");
        assert_eq!(CodeHealthCategory::Structure.label(), "结构复杂度");
        assert_eq!(CodeHealthCategory::UiConsistency.label(), "UI 一致性");
        assert_eq!(CodeHealthCategory::ScanScope.label(), "扫描范围");
    }
}
