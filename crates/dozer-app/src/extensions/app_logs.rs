//! 应用日志查看器的共享状态机(A6f Task 4)——状态机本体已搬到 `bytehost-panel::logs`
//! (与 Digger 共用),这里只保留**iced 专属**的滚动区 `Id`。
//!
//! [`LogsState`]/[`LogsView`]/[`LogsEffect`]/[`REFRESH_INTERVAL`]/[`FETCH_LINES`] 直接复用共享实现;
//! 两个宿主不各写一遍。

#[allow(unused_imports)] // `LogsEffect` 是共享状态机的对外接口,dozer 侧经 `LogsState` 间接使用
pub use bytehost_panel::logs::{FETCH_LINES, LogsEffect, LogsState, LogsView, REFRESH_INTERVAL};

/// 日志查看器滚动区的稳定 `Id`(按应用 id 区分,两个宿主共用同一个前缀)。
pub fn scroll_id(app_id: &str) -> iced_widget::core::widget::Id {
    iced_widget::core::widget::Id::from(format!("app-log-view:{app_id}"))
}

/// 日志正文的文字色:主题的主文字色(`cream`,浅色主题里它是深色)。
///
/// 日志 `text(..)` 不设色时用的是 iced 默认文字色,和 Dozer 自己的主题脱节:浅色主题碰巧能看,深色主题下
/// 几乎看不见(验收 2.7)。面板里的日志和设置页里的日志共用这一个入口。
pub(crate) fn log_text_color(
    tokens: &byteui::theme::color::ColorTokens,
) -> iced_widget::core::Color {
    tokens.cream
}

#[cfg(test)]
mod tests {
    use super::log_text_color;
    use byteui::theme::color::ColorTokens;
    use iced_widget::core::Color;

    /// WCAG 相对亮度(sRGB → 线性)。
    fn luminance(c: Color) -> f32 {
        let lin = |v: f32| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    fn contrast(a: Color, b: Color) -> f32 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    /// 验收 2.7/3.x:深色主题下日志文字几乎看不见(用的是 iced 默认文字色)。
    /// 深色、浅色两套 token 下,日志文字与面板底色的对比度都要 ≥ 4.5(WCAG AA 正文)。
    #[test]
    fn log_text_is_readable_on_the_panel_in_both_themes() {
        for (name, t) in [
            ("dark", ColorTokens::byteboy2077()),
            ("light", ColorTokens::byteboy2077_light()),
        ] {
            let ratio = contrast(log_text_color(&t), t.panel);
            assert!(ratio >= 4.5, "{name}: 对比度 {ratio:.2} < 4.5");
        }
    }

    /// 面板里的日志(`app/view.rs`)必须经这个函数取色;设置页的日志已迁入
    /// `bytehost-settings-ui`,那边用同一套 `cream` token(共享视图有独立测试)。
    #[test]
    fn log_views_use_the_shared_colour_or_delegate_to_the_shared_view() {
        for (name, src, needle) in [
            (
                "app/view.rs",
                include_str!("../app/view.rs"),
                "log_text_color(",
            ),
            (
                "extensions/settings_apps_view.rs",
                include_str!("settings_apps_view.rs"),
                "bytehost_settings_ui::apps::view",
            ),
        ] {
            assert!(src.contains(needle), "{name} 没有使用 {needle}");
        }
    }
}
