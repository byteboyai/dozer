//! diff 文本的逐行染色渲染——`+`/`-`/上下文三种颜色,等宽字体,`TERM_BG`
//! 背景。原先只有 `extensions/acceptance.rs::diff_view` 一份实现,Git Log
//! 面板重构(2026-08-17)需要同一套渲染,抽成共享函数避免重复(见
//! `docs/superpowers/specs/2026-08-17-git-log-panel-three-pane-redesign-design.md`
//! 第 8 节)。

use crate::theme;
use iced_widget::core::{Element, Font};
use iced_widget::{column, container, text};

/// `patch` 逐行染色:`+` 开头 GREEN、`-` 开头 RED、其余(上下文行/文件头)
/// DIM,等宽字体、`caption_sm()` 字号、`TERM_BG` 背景容器包裹。空字符串
/// 渲染成空的 `column`(不特判——调用方决定"空 patch 时是否要显示占位文案",
/// 这个函数只管染色,不管空态提示)。
pub fn colored_diff_lines<'a, M: 'a>(
    patch: &str,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![].spacing(0).padding([4, 12]);
    for line in patch.lines() {
        let color = if line.starts_with('+') {
            byteui::theme::color::current().green
        } else if line.starts_with('-') {
            byteui::theme::color::current().red
        } else {
            byteui::theme::color::current().dim
        };
        col = col.push(
            text(line.to_string())
                .size(theme::font::caption_sm())
                .color(color)
                .font(Font::MONOSPACE)
                .line_height(iced_widget::core::text::LineHeight::Relative(1.3)),
        );
    }
    container(col)
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(byteui::theme::color::current().term_bg.into()),
            ..iced_widget::container::Style::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 纯渲染函数——iced `Element` 没有公开的"读回渲染结果"API,这里只验证
    // 函数在各种输入(空串/纯 +/纯 -/混合/无换行)下不 panic,能正常构造出
    // `Element`。真正的视觉效果靠人工验收(见 spec 测试策略)。
    #[test]
    fn colored_diff_lines_does_not_panic_on_various_inputs() {
        let _: Element<'_, (), iced_widget::Theme, iced_renderer::Renderer> =
            colored_diff_lines("");
        let _: Element<'_, (), iced_widget::Theme, iced_renderer::Renderer> =
            colored_diff_lines("+added line\n-removed line\n context line\n");
        let _: Element<'_, (), iced_widget::Theme, iced_renderer::Renderer> =
            colored_diff_lines("no trailing newline");
    }
}
