//! 预览面板界面的**视图模型**:把 `workspace/view.rs` 里夹在 iced 构造代码中的
//! 文案 / 可见性 / 动作选择逻辑抽成纯函数,以便单测固定。S5b-2 P2 Task 1 在原地
//! 抽取(不搬位置),Task 5 随界面一起搬进 `dozer-panel-preview`。
//!
//! 这里的每个函数都是**现状行为的逐字等价抽取**:不改文案、不改判定顺序。

use crate::preview::BlankPaneInfo;
use crate::preview::{
    BackendState, FallbackActions, PreviewBackend, PreviewMode, PreviewPane, PreviewTab, TabKind,
    TabularFormat, TabularMode,
};
use dozer_model::ProjectInfo;

// ---------------------------------------------------------------------------
// 数字 / 时间格式化(空白信息卡用)
// ---------------------------------------------------------------------------

/// `n` 插入千位分隔符(中文惯例就是半角逗号,与英文一致)。`23456` →
/// `"23,456"`;`0` → `"0"`。纯展示,不损失精度。
pub(crate) fn with_thousands_separator(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i != 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

/// 字节数转成 KB/MB/GB 可读串:`0`~`1023` → `"N B"`;`1024` → `"1KB"`;
/// `10240` → `"10KB"`;`1536` → `"1.5KB"`;`1.2GB` 保留一位小数。整除整单位
/// 时不显示小数,避免 `"10.0KB"` 这类啰嗦写法。纯展示,精度足够项目大小
/// 展示用;精确字节数由调用方另以括号补出。
pub(crate) fn format_human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let b = bytes as f64;
    let (v, unit) = if b >= GB {
        (b / GB, "GB")
    } else if b >= MB {
        (b / MB, "MB")
    } else if b >= KB {
        (b / KB, "KB")
    } else {
        return format!("{bytes} B");
    };
    if (v.round() - v).abs() < 0.05 {
        format!("{}{}", v as i64, unit)
    } else {
        format!("{:.1}{}", v, unit)
    }
}

/// SystemTime → 本地化日期串(无秒)。中文为
/// `"YYYY年M月D日 星期X HH:MM"`,英文为 `"Mon, D MMM YYYY HH:MM"`。Howard
/// Hinnant 的 `civil_from_days` 同款算法(见 `extensions/git_log.rs::1283`,
/// 那里只输出 `YYYY-MM-DD HH:MM:SS`,这里加星期)。失败(平台不支持 /
/// 1970 之前)时回退 `"未知"` / `"Unknown"`。
pub(crate) fn format_time(t: std::time::SystemTime, locale: &str) -> String {
    let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let total_secs = dur.as_secs();
    let days = (total_secs / 86_400) as i64;
    let secs_of_day = total_secs % 86_400;
    let (h, m, _) = (
        secs_of_day / 3600,
        (secs_of_day / 60) % 60,
        secs_of_day % 60,
    );
    let (y, mo, d) = civil_from_days(days);
    // 1970-01-01 是星期四,加天数 mod 7 拿到 0..6,offset = 周一 = 0
    let weekday = (((days + 3) % 7) + 7) % 7;
    let weekday_key = [
        "date.weekday_mon",
        "date.weekday_tue",
        "date.weekday_wed",
        "date.weekday_thu",
        "date.weekday_fri",
        "date.weekday_sat",
        "date.weekday_sun",
    ][weekday as usize];
    let weekday_fallback = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ][weekday as usize];
    let weekday_name = crate::i18n::host_text(weekday_key, weekday_fallback, locale);
    // 占位符名在两种语言的模板里必须一致(资源校验要求 key 集合逐个相同),
    // 差异只在模板形状与取值:英文月份给缩写、中文给数字月。
    let month_value = if locale.starts_with("zh") {
        mo.to_string()
    } else {
        month_name_en(mo).to_string()
    };
    crate::i18n::host_text_args(
        "date.datetime",
        "{weekday}, {day} {month} {y} {hh}:{mm}",
        locale,
        &[
            ("weekday", weekday_name.as_str()),
            ("y", &y.to_string()),
            ("month", month_value.as_str()),
            ("day", &d.to_string()),
            ("hh", &format!("{h:02}")),
            ("mm", &format!("{m:02}")),
        ],
    )
}

/// 英文月份缩写(英文日期串用;中文串不用月份名,直接 `M月`)。
fn month_name_en(mo: u32) -> &'static str {
    [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(mo as usize).saturating_sub(1).min(11)]
}

/// Howard Hinnant 的 `civil_from_days`:Unix epoch 起的天数 → (年, 月, 日)。
/// 范围覆盖 1970..=2100,足够文件系统时间戳用。与 `git_log.rs` 的同名函数
/// 同源(那个是模块私有,这里复制一份给空白页信息卡用)。
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
// 页签标题与关闭动作
// ---------------------------------------------------------------------------

/// 当前项目根目录的 basename(`project.path` 的最后一段路径分量)。拿不到文件
/// 名(尾部是 `..`/`/` 等)回退到 `path.display()` 完整字符串。无项目时 `None`。
pub(crate) fn project_root_dir_name_of(project: Option<&ProjectInfo>) -> Option<String> {
    let project = project?;
    let path = std::path::Path::new(&project.path);
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .or_else(|| Some(path.display().to_string()))
}

/// 预览 tab 的渲染标题(宽度预算与实际渲染共用)。`Blank` 占位 tab 返回项目
/// 根目录 basename;没项目时回退本地化的 `Blank`;其余返回 `tab.title`。
pub(crate) fn preview_tab_title(
    project: Option<&ProjectInfo>,
    tab: &PreviewTab,
    locale: &str,
) -> String {
    if matches!(tab.kind, TabKind::Blank)
        && let Some(root) = project_root_dir_name_of(project)
    {
        return root;
    }
    if matches!(tab.kind, TabKind::Blank) {
        return crate::i18n::panel_text(
            crate::preview::PREVIEW_OWNER,
            "view.blank",
            "Blank",
            locale,
        );
    }
    tab.title.clone()
}

/// 页签条上实际显示的标题:在 [`preview_tab_title`] 之后,就地可写的 tab 有
/// 未保存改动时加 ` *` 后缀。(宽度预算按不带星的标题估,最坏多一个字符。)
pub(crate) fn preview_tab_label(
    project: Option<&ProjectInfo>,
    tab: &PreviewTab,
    locale: &str,
) -> String {
    let mut title = preview_tab_title(project, tab, locale);
    if tab.dirty {
        title.push_str(" *");
    }
    title
}

/// index 0 的 `Blank` 占位 tab 单独存在时,其 × 等价于「选中」(不真关,避免
/// 「列表瞬空再补」的视觉跳动);有兄弟 tab 时是真关。文件 tab 一律真关。
pub(crate) fn tab_close_selects_instead(tab: &PreviewTab, tab_count: usize) -> bool {
    matches!(tab.kind, TabKind::Blank) && tab_count == 1
}

// ---------------------------------------------------------------------------
// 空白信息卡
// ---------------------------------------------------------------------------

/// 空白页信息卡的文案模型。`stats` 为 `None` = 无项目,只画头部。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BlankInfoModel {
    pub header_label: String,
    /// 四行 `(label, value)`:位置 / 大小 / 创建时间 / 修改时间。
    pub stats: Option<[(String, String); 4]>,
}

/// `info` 为 `None`(后台还在跑)时三行值画 `—`;位置行始终是项目路径。
pub(crate) fn blank_info_model(
    project: Option<&ProjectInfo>,
    info: Option<&BlankPaneInfo>,
    locale: &str,
) -> BlankInfoModel {
    let header_label = project_root_dir_name_of(project)
        .or_else(|| project.map(|p| p.name.clone()))
        .unwrap_or_default();
    let Some(project) = project else {
        return BlankInfoModel {
            header_label,
            stats: None,
        };
    };
    let size_value = info.map_or("—".to_string(), |i| {
        crate::i18n::host_text_args(
            "preview.blank_size_value",
            "{human} ({bytes} bytes on disk), {count} items",
            locale,
            &[
                ("human", format_human_size(i.size_bytes).as_str()),
                ("bytes", with_thousands_separator(i.size_bytes).as_str()),
                ("count", with_thousands_separator(i.file_count).as_str()),
            ],
        )
    });
    let created_value = info
        .and_then(|i| i.created)
        .map(|t| format_time(t, locale))
        .unwrap_or_else(|| "—".to_string());
    let modified_value = info
        .and_then(|i| i.modified)
        .map(|t| format_time(t, locale))
        .unwrap_or_else(|| "—".to_string());
    BlankInfoModel {
        header_label,
        stats: Some([
            (
                crate::i18n::host_text("preview.info_location", "Location:", locale),
                project.path.clone(),
            ),
            (
                crate::i18n::host_text("preview.info_size", "Size:", locale),
                size_value,
            ),
            (
                crate::i18n::host_text("preview.info_created", "Created:", locale),
                created_value,
            ),
            (
                crate::i18n::host_text("preview.info_modified", "Modified:", locale),
                modified_value,
            ),
        ]),
    }
}

// ---------------------------------------------------------------------------
// 降级页
// ---------------------------------------------------------------------------

/// 降级页(External / Unsupported / Failed)的文案模型。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FallbackModel {
    pub tab_id: usize,
    pub title: String,
    pub headline: String,
    pub path: Option<String>,
    pub route: Option<String>,
    pub failure: Option<String>,
    pub security_note: String,
    pub actions: FallbackActions,
}

/// 该 tab 此刻是否该画降级页;是则给出全部文案与可用动作。
pub(crate) fn fallback_model(tab: &PreviewTab, locale: &str) -> Option<FallbackModel> {
    let backend = tab.backend.as_ref()?;
    let is_ext = matches!(
        backend,
        PreviewBackend::External(_) | PreviewBackend::Unsupported(_)
    );
    // Failed 一律走这一页(hosts_webview/editor/json host 都已在非 Ready 时
    // 不产出 viewer,原生子视图不会盖住它)。
    let show_failed = tab.backend_state.is_failed();
    if !(is_ext || show_failed) {
        return None;
    }
    let content_is_text = tab.route.as_ref().is_some_and(|r| r.content_is_text);
    let actions = crate::preview::fallback_actions(backend, &tab.backend_state, content_is_text);
    let reason = tab
        .route
        .as_ref()
        .map(|r| r.reason.localized(locale))
        .unwrap_or_default();
    let path = match &tab.kind {
        TabKind::File(p) => p.to_string_lossy().into_owned(),
        _ => String::new(),
    };
    let failure = tab.web_error.clone().or_else(|| match &tab.backend_state {
        BackendState::Failed(e) => Some(e.message.clone()),
        _ => None,
    });
    Some(FallbackModel {
        tab_id: tab.id,
        title: tab.title.clone(),
        headline: crate::i18n::host_text(
            "preview.fallback_title",
            "Cannot preview this file inside Dozer",
            locale,
        ),
        path: (!path.is_empty()).then_some(path),
        route: (!reason.is_empty()).then(|| {
            crate::i18n::host_text_args(
                "preview.fallback_route",
                "Route: {reason}",
                locale,
                &[("reason", reason.as_str())],
            )
        }),
        failure: failure.map(|err| {
            crate::i18n::host_text_args(
                "preview.fallback_reason",
                "Reason: {error}",
                locale,
                &[("error", err.as_str())],
            )
        }),
        security_note: crate::i18n::host_text(
            "preview.fallback_security_note",
            "For safety, Dozer will not execute or decode this file inline.",
            locale,
        ),
        actions,
    })
}

// ---------------------------------------------------------------------------
// 冲突条
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ConflictBarModel {
    pub notice: String,
    pub keep_label: String,
    pub reload_label: String,
}

/// `armed` 为真时重载键变成二次确认文案(再点一次才丢弃改动)。
pub(crate) fn conflict_bar_model(armed: bool, locale: &str) -> ConflictBarModel {
    ConflictBarModel {
        notice: crate::i18n::host_text(
            "preview.conflict_notice",
            "File on disk changed externally",
            locale,
        ),
        keep_label: crate::i18n::host_text("preview.conflict_keep", "Keep my changes", locale),
        reload_label: if armed {
            crate::i18n::host_text(
                "preview.conflict_reload_confirm",
                "Reload from disk (discard my changes)",
                locale,
            )
        } else {
            crate::i18n::host_text("preview.conflict_reload", "Reload from disk", locale)
        },
    }
}

// ---------------------------------------------------------------------------
// 工具栏模式切换按钮
// ---------------------------------------------------------------------------

/// 激活 tab 上哪些模式切换按钮出现,以及各自的当前态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ToolbarModel {
    /// 预览 / 代码切换:`Some(in_code_mode)`;仅 `wry_toggle_eligible` 的文件 tab。
    pub render_mode: Option<RenderModeButton>,
    /// CSV / TSV 的「网格 / 原文」:`Some((tab_id, in_text))`;工作簿(xlsx 等)不出现。
    pub tabular_mode: Option<(usize, bool)>,
    /// 严格 `.json` 的「树 / 文本」:`Some((tab_id, in_text))`。
    pub json_mode: Option<(usize, bool)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RenderModeButton {
    pub tab_idx: usize,
    pub in_code_mode: bool,
}

pub(crate) fn toolbar_model(preview: &PreviewPane) -> ToolbarModel {
    let Some(tab) = preview.tabs().get(preview.active_idx()) else {
        return ToolbarModel::default();
    };
    let render_mode = {
        let eligible = match &tab.kind {
            TabKind::File(p) => crate::preview::wry_toggle_eligible(p),
            _ => false,
        };
        eligible.then(|| RenderModeButton {
            tab_idx: preview.active_idx(),
            in_code_mode: tab.uses_rendered_source_editor(),
        })
    };
    let tabular_mode = (|| {
        let PreviewBackend::Tabular(tabular) = tab.backend.as_ref()? else {
            return None;
        };
        if tabular.format == TabularFormat::Workbook {
            return None;
        }
        Some((tab.id, tabular.mode == TabularMode::Text))
    })();
    let json_mode = (|| {
        if !matches!(tab.backend.as_ref(), Some(PreviewBackend::Json(_))) {
            return None;
        }
        let route = tab.route.as_ref()?;
        if !route.supports(PreviewMode::Tree) || !route.supports(PreviewMode::Text) {
            return None;
        }
        let in_text = tab
            .backend
            .as_ref()
            .is_some_and(|b| b.current_mode() == PreviewMode::Text);
        Some((tab.id, in_text))
    })();
    ToolbarModel {
        render_mode,
        tabular_mode,
        json_mode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn project(path: &str, name: &str) -> ProjectInfo {
        ProjectInfo {
            id: 1,
            name: name.into(),
            path: path.into(),
            last_active_ms: 0,
            created_ms: 0,
            updated_ms: 0,
        }
    }

    /// 以真实路由造一个文件 tab(与生产同源:`open_path` → `push_tab` →
    /// `route_and_backend`)。返回 tab id。
    fn file_tab(pane: &mut PreviewPane, path: &str) -> usize {
        pane.open_path(PathBuf::from(path))
    }

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    // ---- 格式化 ----

    #[test]
    fn thousands_separator_groups_by_three_and_handles_the_extremes() {
        assert_eq!(with_thousands_separator(0), "0");
        assert_eq!(with_thousands_separator(999), "999");
        assert_eq!(with_thousands_separator(1_000), "1,000");
        assert_eq!(with_thousands_separator(23_456), "23,456");
        assert_eq!(with_thousands_separator(1_234_567), "1,234,567");
        assert_eq!(
            with_thousands_separator(u64::MAX),
            "18,446,744,073,709,551,615"
        );
    }

    #[test]
    fn human_size_boundaries_and_rounding_are_stable() {
        assert_eq!(format_human_size(0), "0 B");
        assert_eq!(format_human_size(1023), "1023 B");
        assert_eq!(format_human_size(1024), "1KB");
        assert_eq!(format_human_size(1536), "1.5KB");
        assert_eq!(format_human_size(10 * 1024), "10KB");
        assert_eq!(format_human_size(1024 * 1024), "1MB");
        assert_eq!(format_human_size(1024 * 1024 * 1024), "1GB");
        assert_eq!(
            format_human_size((1.2 * 1024.0 * 1024.0 * 1024.0) as u64),
            "1.2GB"
        );
        // 接近整数单位(误差 < 0.05)时不显示小数。
        assert_eq!(format_human_size(1024 * 1024 + 10), "1MB");
        // u64::MAX 不 panic,落在 GB 档。
        assert!(format_human_size(u64::MAX).ends_with("GB"));
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(365), (1971, 1, 1));
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        // 闰日。
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn format_time_is_localized_for_zh_and_en_and_survives_epoch_and_future() {
        // 2024-01-01 00:00:00 UTC 是星期一。
        let en = format_time(at(1_704_067_200), "en");
        let zh = format_time(at(1_704_067_200), "zh-CN");
        assert!(en.contains("Mon") || en.contains("Monday"), "{en}");
        assert!(
            en.contains("Jan") && en.contains("2024") && en.contains("00:00"),
            "{en}"
        );
        assert!(zh.contains("2024") && zh.contains("00:00"), "{zh}");
        assert_ne!(en, zh);
        // 纪元时间(1970-01-01 是星期四)。
        let epoch = format_time(at(0), "en");
        assert!(epoch.contains("1970") && epoch.contains("00:00"), "{epoch}");
        // 1970 之前(`duration_since` 失败)按纪元处理,不 panic。
        let before = format_time(UNIX_EPOCH - Duration::from_secs(10), "en");
        assert_eq!(before, epoch);
        // 很远的未来不 panic。
        let _ = format_time(at(u64::MAX / 4), "en");
    }

    // ---- 页签标题与关闭 ----

    #[test]
    fn blank_tab_title_follows_the_project_root_basename_then_the_localized_blank() {
        let mut pane = PreviewPane::default();
        let blank = crate::preview::placeholder_tab(0);
        let p = project("/Users/a/Projects/dozer", "Dozer AI Coder");
        assert_eq!(preview_tab_title(Some(&p), &blank, "en"), "dozer");
        assert_eq!(preview_tab_title(None, &blank, "en"), "Blank");
        // 根路径(无 basename)回退整个路径串。
        let root = project("/", "r");
        assert_eq!(preview_tab_title(Some(&root), &blank, "en"), "/");
        // 文件 tab 用自己的标题,不受项目影响。
        let id = file_tab(&mut pane, "/tmp/a/b.rs");
        let t = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert_eq!(preview_tab_title(Some(&p), t, "en"), "b.rs");
    }

    #[test]
    fn tab_label_appends_a_star_only_for_dirty_tabs_and_keeps_cjk_titles() {
        let mut pane = PreviewPane::default();
        let id = file_tab(&mut pane, "/tmp/说明文档.md");
        let tab = pane.tabs_mut().iter_mut().find(|t| t.id == id).unwrap();
        assert_eq!(preview_tab_label(None, tab, "zh-CN"), "说明文档.md");
        tab.dirty = true;
        assert_eq!(preview_tab_label(None, tab, "zh-CN"), "说明文档.md *");
        // 空标题与超长标题原样透传(不截断,截断由 clip 容器负责)。
        tab.title = String::new();
        assert_eq!(preview_tab_label(None, tab, "en"), " *");
        tab.title = "x".repeat(10_000);
        tab.dirty = false;
        assert_eq!(preview_tab_label(None, tab, "en").len(), 10_000);
    }

    #[test]
    fn placeholder_close_selects_only_when_it_is_the_only_tab() {
        let mut pane = PreviewPane::default();
        let blank = crate::preview::placeholder_tab(0);
        assert!(tab_close_selects_instead(&blank, 1));
        assert!(!tab_close_selects_instead(&blank, 2));
        let id = file_tab(&mut pane, "/tmp/a.rs");
        let file = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(!tab_close_selects_instead(file, 1));
        assert!(!tab_close_selects_instead(file, 5));
    }

    // ---- 空白信息卡 ----

    #[test]
    fn blank_info_without_a_project_has_only_the_header() {
        let m = blank_info_model(None, None, "en");
        assert_eq!(m.header_label, "");
        assert!(m.stats.is_none());
    }

    #[test]
    fn blank_info_shows_dashes_until_the_background_info_arrives() {
        let p = project("/Users/a/dozer", "Dozer AI Coder");
        let m = blank_info_model(Some(&p), None, "en");
        assert_eq!(m.header_label, "dozer");
        let stats = m.stats.unwrap();
        assert_eq!(stats[0].1, "/Users/a/dozer");
        assert_eq!(stats[1].1, "—");
        assert_eq!(stats[2].1, "—");
        assert_eq!(stats[3].1, "—");
    }

    #[test]
    fn blank_info_formats_size_count_and_times_for_en_and_zh() {
        let p = project("/Users/a/dozer", "D");
        let info = BlankPaneInfo {
            path: PathBuf::from("/Users/a/dozer"),
            size_bytes: 12_582_912,
            file_count: 23_456,
            created: Some(at(1_704_067_200)),
            modified: None,
        };
        let en = blank_info_model(Some(&p), Some(&info), "en").stats.unwrap();
        assert_eq!(en[1].1, "12MB (12,582,912 bytes on disk), 23,456 items");
        assert!(en[2].1.contains("2024"));
        assert_eq!(en[3].1, "—", "没有修改时间画占位");
        let zh = blank_info_model(Some(&p), Some(&info), "zh-CN")
            .stats
            .unwrap();
        assert!(
            zh[1].1.contains("12,582,912") && zh[1].1.contains("23,456"),
            "{}",
            zh[1].1
        );
        assert_ne!(en[0].0, zh[0].0, "标签随语言变化");
        // 0 字节 / 0 文件不 panic。
        let empty = BlankPaneInfo {
            size_bytes: 0,
            file_count: 0,
            ..info.clone()
        };
        let m = blank_info_model(Some(&p), Some(&empty), "en")
            .stats
            .unwrap();
        assert!(
            m[1].1.starts_with("0 B (0 bytes on disk), 0 items"),
            "{}",
            m[1].1
        );
    }

    #[test]
    fn blank_info_header_falls_back_to_the_project_name_only_without_a_basename() {
        // 路径有 basename → 用 basename(而不是显示名)。
        let p = project("/Users/a/dozer", "Dozer AI Coder");
        assert_eq!(blank_info_model(Some(&p), None, "en").header_label, "dozer");
    }

    // ---- 降级页 ----

    #[test]
    fn fallback_model_covers_every_backend_and_state_combination() {
        let mut pane = PreviewPane::default();
        // 代码 tab 就绪:不画降级页。
        let code = file_tab(&mut pane, "/tmp/a.rs");
        let t = pane.tabs().iter().find(|t| t.id == code).unwrap();
        assert!(fallback_model(t, "en").is_none());
        // 同一 tab 失败:画降级页,带原因,可重试时有 retry。
        let tab = pane.tabs_mut().iter_mut().find(|t| t.id == code).unwrap();
        tab.backend_state = BackendState::Failed(crate::preview::PreviewError::new("boom", true));
        let m = fallback_model(tab, "en").unwrap();
        assert_eq!(m.tab_id, code);
        assert_eq!(m.failure.as_deref(), Some("Reason: boom"));
        assert!(m.actions.retry && m.actions.external_open);
        assert_eq!(m.path.as_deref(), Some("/tmp/a.rs"));
        // 不可重试的失败没有 retry。
        tab.backend_state = BackendState::Failed(crate::preview::PreviewError::new("nope", false));
        assert!(!fallback_model(tab, "en").unwrap().actions.retry);
        // `web_error` 优先于后端错误。
        tab.web_error = Some("render crashed".into());
        assert_eq!(
            fallback_model(tab, "en").unwrap().failure.as_deref(),
            Some("Reason: render crashed")
        );
        // 未知二进制:路由为 External/Unsupported → 降级页,无 plain-text 退路。
        let bin = file_tab(&mut pane, "/tmp/blob.bin");
        let t = pane.tabs().iter().find(|t| t.id == bin).unwrap();
        if let Some(m) = fallback_model(t, "en") {
            assert!(m.actions.external_open);
            assert_eq!(
                m.security_note,
                "For safety, Dozer will not execute or decode this file inline."
            );
        }
        // Blank 占位 tab 没有 backend → None。
        let blank = crate::preview::placeholder_tab(0);
        assert!(fallback_model(&blank, "en").is_none());
    }

    // ---- 冲突条 ----

    #[test]
    fn conflict_bar_reload_label_requires_a_second_click_when_armed() {
        let plain = conflict_bar_model(false, "en");
        let armed = conflict_bar_model(true, "en");
        assert_eq!(plain.notice, "File on disk changed externally");
        assert_eq!(plain.keep_label, "Keep my changes");
        assert_eq!(plain.reload_label, "Reload from disk");
        assert_eq!(armed.reload_label, "Reload from disk (discard my changes)");
        assert_eq!(plain.notice, armed.notice);
        assert_ne!(
            conflict_bar_model(false, "zh-CN").keep_label,
            plain.keep_label
        );
    }

    // ---- 工具栏 ----

    #[test]
    fn toolbar_shows_mode_buttons_only_for_the_eligible_active_tab() {
        let mut pane = PreviewPane::default();
        // 只有 Blank:都不出现。
        assert_eq!(toolbar_model(&pane), ToolbarModel::default());
        // 纯代码文件:不出现。
        let id = file_tab(&mut pane, "/tmp/a.rs");
        pane.select(pane.tabs().iter().position(|t| t.id == id).unwrap());
        assert_eq!(toolbar_model(&pane), ToolbarModel::default());
        // Markdown:出现预览 / 代码切换(`wry_toggle_eligible`)。
        let md = file_tab(&mut pane, "/tmp/a.md");
        let idx = pane.tabs().iter().position(|t| t.id == md).unwrap();
        pane.select(idx);
        let m = toolbar_model(&pane);
        assert_eq!(m.render_mode.map(|b| b.tab_idx), Some(idx));
        assert!(m.tabular_mode.is_none() && m.json_mode.is_none());
        // CSV:出现网格 / 原文切换;xlsx 工作簿不出现。
        let csv = file_tab(&mut pane, "/tmp/a.csv");
        pane.select(pane.tabs().iter().position(|t| t.id == csv).unwrap());
        assert_eq!(
            toolbar_model(&pane).tabular_mode.map(|(id, _)| id),
            Some(csv)
        );
        let xlsx = file_tab(&mut pane, "/tmp/a.xlsx");
        pane.select(pane.tabs().iter().position(|t| t.id == xlsx).unwrap());
        assert!(toolbar_model(&pane).tabular_mode.is_none());
        // 严格 .json:出现树 / 文本切换;.jsonl 只有文本,不出现。
        let json = file_tab(&mut pane, "/tmp/a.json");
        pane.select(pane.tabs().iter().position(|t| t.id == json).unwrap());
        assert_eq!(toolbar_model(&pane).json_mode.map(|(id, _)| id), Some(json));
        let jsonl = file_tab(&mut pane, "/tmp/a.jsonl");
        pane.select(pane.tabs().iter().position(|t| t.id == jsonl).unwrap());
        assert!(toolbar_model(&pane).json_mode.is_none());
    }
}
