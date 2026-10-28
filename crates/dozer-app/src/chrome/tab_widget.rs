// crates/dozer-app/src/chrome/tab_widget.rs
//! 面板内 tab 栏的共享部件。通用部分(`panel_tab`、`tab_label`、`tab_container_style`、
//! `tab_overflow_button`/`tab_overflow_menu`/`tab_overflow_items`)已搬进
//! `byteui::interaction::panel_tabs`(X1,2026-10;与 Digger 共用),这里只保留原路径的重导出,
//! 调用点不动。窗口化滚动算法 `tab_window`/`tab_window_reveal` 在
//! `byteui::interaction::tab_strip`。
//!
//! 本文件自己的是**预览专属**的三个模式切换按钮(预览/代码、网格/原文、JSON 视图),
//! 其中两个依赖 Dozer 的国际化(`crate::i18n`)。

#[cfg(target_os = "macos")]
pub(crate) use byteui::interaction::panel_tabs::tab_overflow_items;
pub(crate) use byteui::interaction::panel_tabs::{
    NO_TAB_W_LIMIT, PANEL_TAB_PAD_LEFT, PANEL_TAB_PAD_X, PANEL_TAB_PAD_Y, PanelTabArgs,
    TabOverflowEntry, TabOverflowMenuArgs, panel_tab, tab_container_style, tab_label,
    tab_overflow_button, tab_overflow_menu,
};

use byteui::interaction::icons;
use iced_widget::core::Element;

/// 预览/代码模式切换按钮:tab 组非空且当前选中 tab 满足
/// `preview::wry_toggle_eligible` 时才画一个(调用方判断,这里只管渲染,见
/// `workspace::preview_pane_for`)。`in_code_mode` 决定图标——预览态显示
/// `FileCode`(点它切到代码),代码态显示 `FilePlay`(点它切回预览)。套
/// `icons::icon_button_entry` 标准图标按钮,尺寸(图标 `icon_size::row()`、
/// 命中区 `icon_size::row() + 6.0`)特意与相邻的"收起文件树"
/// (`App::list_collapse_button`)对齐,不用自己的 `tab_button_size`
/// (验收反馈:两个按钮挤在一起,尺寸得一致)。`hover_t`/`on_hover` 由调用方
/// 接自己那组专属 `HoverId`(如 `HoverId::named(PanelKind::Files, "render_mode")`)——这个按钮
/// 现在挂在 tab 组本身(只对当前选中 tab 出一个),不再是随每个 tab 渲染、
/// 随下标漂移的旧版做法,已不需要为漂移规避 `HoverId` 动画体系。
pub(crate) fn tab_render_mode_button<'a, M: Clone + 'a>(
    in_code_mode: bool,
    hover_t: f32,
    on_press: M,
    on_hover: impl Fn(bool) -> M + 'a,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let icon = if in_code_mode {
        icons::IconKind::FilePlay
    } else {
        icons::IconKind::FileCode
    };
    icons::icon_button_entry(
        icon,
        byteui::theme::icon_size::row(),
        false,
        false,
        hover_t,
        false,
        byteui::theme::icon_size::row() + 6.0,
        true,
        on_press,
        on_hover,
        "",
    )
}

/// 表格(CSV/TSV)tab 的「网格 / 原文」切换按钮:网格下显示 `FileCode`
/// (点它看原文),原文下显示 `Table`(点它回网格)。`hover_t`/`on_hover`
/// 由调用方接专属 `HoverId`(见 `HoverId::named(PanelKind::Files, "tabular_mode")`)。
pub(crate) fn tab_tabular_mode_button<'a, M: Clone + 'a>(
    in_text: bool,
    hover_t: f32,
    on_press: M,
    on_hover: impl Fn(bool) -> M + 'a,
    locale: &str,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let (icon, tooltip) = if in_text {
        (
            icons::IconKind::Table,
            crate::i18n::host_text("tab.view_table", "View table", locale),
        )
    } else {
        (
            icons::IconKind::FileCode,
            crate::i18n::host_text("tab.view_source", "View source", locale),
        )
    };
    icons::icon_button_entry(
        icon,
        byteui::theme::icon_size::row(),
        false,
        false,
        hover_t,
        false,
        byteui::theme::icon_size::row() + 6.0,
        true,
        on_press,
        on_hover,
        tooltip,
    )
}

/// 严格 `.json` tab 的「树 / 文本」切换按钮:树视图下显示 `FileCode`
/// (点它看文本),文本下显示 `Table`(点它回树)。`hover_t`/`on_hover`
/// 由调用方接专属 `HoverId`(见 `HoverId::named(PanelKind::Files, "json_mode")`)。
pub(crate) fn tab_json_mode_button<'a, M: Clone + 'a>(
    in_text: bool,
    hover_t: f32,
    on_press: M,
    on_hover: impl Fn(bool) -> M + 'a,
    locale: &str,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let (icon, tooltip) = if in_text {
        (
            icons::IconKind::Table,
            crate::i18n::host_text("tab.view_tree", "View tree", locale),
        )
    } else {
        (
            icons::IconKind::FileCode,
            crate::i18n::host_text("tab.view_text", "View text", locale),
        )
    };
    icons::icon_button_entry(
        icon,
        byteui::theme::icon_size::row(),
        false,
        false,
        hover_t,
        false,
        byteui::theme::icon_size::row() + 6.0,
        true,
        on_press,
        on_hover,
        tooltip,
    )
}
