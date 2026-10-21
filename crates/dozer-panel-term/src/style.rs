//! 终端视图的注入式样式(S6b)。
//!
//! 抽库前字体、字号与行高都是 `dozer-app` 的全局值(`assets::fonts::code_font()`
//! / `theme::terminal_font`)。库里不读产品应用的状态,改由调用方构造
//! [`TermStyle`] 传入;`grid_size` 也变成它的方法,公式与抽库前逐字一致。

use iced_widget::core::Font;

/// 终端网格的字体与尺寸。`font_size` 是设计基准字号(逻辑像素),实际绘制
/// 与网格换算都要乘全局 UI scale(`byteui::theme::icon_size::scale()`),这样
/// Ctrl +/- 缩放时终端字符与图标/控件一起放大。
#[derive(Debug, Clone)]
pub struct TermStyle {
    /// 等宽字体(产品注册后传入,库里不内嵌字体)。
    pub font: Font,
    /// 设计基准字号(逻辑像素),未乘全局 scale。
    pub font_size: f32,
    /// 行高倍数(行高像素 = 缩放后字号 × 该值)。
    pub line_height_factor: f32,
}

impl TermStyle {
    /// 实际字号(逻辑像素):基准字号 × 全局 UI scale。
    pub fn scaled_font_size(&self) -> f32 {
        self.font_size * byteui::theme::icon_size::scale()
    }

    /// 等宽字体单元格宽度 ≈ 0.6em(由缩放后字号驱动)。
    pub fn cell_width(&self) -> f32 {
        self.scaled_font_size() * 0.6
    }

    /// 行高(逻辑像素),供网格换算与绘制用。
    pub fn line_height_px(&self) -> f32 {
        self.scaled_font_size() * self.line_height_factor
    }

    /// 终端 pane 的像素尺寸 → 网格尺寸 `(cols, rows)`,向下取整(不足一格
    /// 的余量丢弃,避免半个字符溢出边界)。负值与 NaN 钳到 0。
    pub fn grid_size(&self, width_px: f32, height_px: f32) -> (usize, usize) {
        let cols = (width_px / self.cell_width()).floor().max(0.0) as usize;
        let rows = (height_px / self.line_height_px()).floor().max(0.0) as usize;
        (cols, rows)
    }
}
