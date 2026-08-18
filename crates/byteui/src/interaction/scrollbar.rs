//! 全应用统一的滚动条配置(几何 + 外观)。各面板 `Scrollable` 共用,保证滑块
//! 颜色、轨道宽度一致。iced 把几何(`scrollable::Scrollbar` 的 `width`/
//! `scroller_width`)与外观(`scrollable::Style` 的 `Scroller` 颜色)拆成了两个
//! API,这里成对提供,避免各处重复、漂移。

use crate::theme;
use iced_widget::core::{Background, Border, Color, Shadow};
use iced_widget::scrollable;

/// 统一滚动条几何:滑块(thumb)收窄一致,宽度见 `workspace_geometry`。
/// 用法:`scrollable::Direction::Vertical(crate::scrollbar::scrollbar())`。
pub fn scrollbar() -> scrollable::Scrollbar {
    scrollable::Scrollbar::new()
        .width(theme::geometry::scrollbar_width())
        .scroller_width(theme::geometry::scrollbar_thumb_width())
}

/// 统一滚动条外观:滑块(thumb)用甲方金 `#dcc9a3`(`theme::color::TAB_ACTIVE_BORDER`),
/// 轨道背景透明、无边框;竖直/水平滚动条一致。
/// 用法:`Scrollable::new(..).style(|_t, _s| crate::scrollbar::scrollbar_style())`。
pub fn scrollbar_style() -> scrollable::Style {
    let scroller = scrollable::Scroller {
        background: Background::Color(theme::color::current().tab_active_border),
        border: Border {
            radius: (theme::geometry::scrollbar_thumb_width() / 2.0).into(),
            ..Border::default()
        },
    };
    scrollable::Style {
        container: iced_widget::container::Style::default(),
        vertical_rail: scrollable::Rail {
            background: None,
            border: Border::default(),
            scroller,
        },
        horizontal_rail: scrollable::Rail {
            background: None,
            border: Border::default(),
            scroller,
        },
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: Background::Color(Color::TRANSPARENT),
            border: Border::default(),
            shadow: Shadow::default(),
            icon: Color::TRANSPARENT,
        },
    }
}
