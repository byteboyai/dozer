//! 文件树几何推导:`tree_row_h`(单行高度)、`tree_chrome_top_px`/
//! `tree_chrome_bottom_px`(Scrollable 视口上下的 chrome 高度),供
//! `extensions/files.rs` 的外部拖拽命中测试把窗口 Y 坐标换算成"可见行
//! 序号"。
//!
//! 其余几何 token(图标栏宽、窗口尺寸下限、右键菜单尺寸等)已迁移到
//! `byteui::theme::geometry`——这里只剩这 3 个函数,因为它们依赖
//! `region`/`terminal_font`/`workspace::tree_row_font_size` 这些未迁移
//! 或 app 专属的模块,不能搬进不依赖 `dozer-app` 的 `byteui`。不解析
//! 任何 JSON,纯由其它 token 组合推导。

/// 文件树目录行的高度（逻辑像素）：行内文字按 `tree_row_font_size` ×
/// `line_height_factor` 定高，行按钮无纵向内边距，高度即行文字高度。
/// 外部拖拽命中测试用它与行间距把窗口 Y 换算成"可见行序号"，两处必须同源
/// （渲染侧在 `files::view` 的行 `line`，间距侧是 `project_pane` 的
/// `gap`），否则命中会对不上眼睛看到的行。
pub fn tree_row_h() -> f32 {
    crate::workspace::tree_row_font_size() * crate::theme::terminal_font::line_height_factor()
}

/// 文件树 Scrollable 之上（容器上内边距 + 面板头 + 搜索/工具栏 + 两段行间距）
/// 的 chrome 高度（逻辑像素），已含全局 scale。`left_files_tree_bounds` 用它
/// 与 `preview_content_bounds` 同源的谱系换算树视口的纵向起点；同样地，它底
/// 下（git 脚注栏 + 行间距 + 容器下内边距）由 `tree_chrome_bottom_px` 承担。
///
/// 这里**复刻** `files::view` 的固定堆栈，不另起字面量——内里每一项都取自
/// 渲染侧同一 token，改一处自动同步。行文字用 `LineHeight::Normal` 的高度
/// 按字号 × 1.2 近似（iced 默认行高），与实际测量的偏差至多几个逻辑像素，
/// 落在行间死区内，不影响相邻行判定。
pub fn tree_chrome_top_px() -> f32 {
    let region = crate::theme::region::project_pane();
    let row = byteui::theme::icon_size::row();
    // 面板头：`column![head_row, 分割线].spacing(8)`，高度 = max(图标, 标题文字) + 8 + 1。
    let head_row_h = row.max(byteui::theme::font::subtitle() as f32 * 1.2);
    let head_h = head_row_h + 8.0 + 1.0;
    // 搜索/工具栏行以 `.padding([6,0])` 包一行元素；搜索框高度（body 文字×1.2
    // + `[6,8]` 内边距 + 1px 边框）通常高于两侧 box 按钮（`row()+12`），取大者。
    let search_h = byteui::theme::font::body() as f32 * 1.2 + 12.0 + 2.0;
    let box_h = row + 12.0;
    let header_h = search_h.max(box_h) + 12.0;
    region.padding.top + head_h + region.gap + header_h + region.gap
}

/// 文件树 Scrollable 之下（git 脚注栏 + 一段行间距 + 容器下内边距）的 chrome
/// 高度（逻辑像素），已含全局 scale。`left_files_tree_bounds` 用它算视口下缘。
pub fn tree_chrome_bottom_px() -> f32 {
    let region = crate::theme::region::project_pane();
    // git 脚注栏：分割线 1 + 行内间距 4 + 一行内容（内容高度取分支切换按钮
    // 的 `row()+12`，与工具栏 box 按钮同高）+ 容器 `[6,8]` 内边距。
    let footer_bar_h = 1.0 + 4.0 + (byteui::theme::icon_size::row() + 12.0) + 12.0;
    region.padding.bottom + region.gap + footer_bar_h
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 文件树几何随 token 变化时同步折算（基准 scale=1 的字面量）。若
    /// 渲染侧 `files::view` 的固定堆栈高度改动，这里也要跟着改——两侧是
    /// 同一真相，漂移会让外部拖拽命中错行。
    #[test]
    fn tree_geometry_matches_composition() {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert!(near(tree_row_h(), 16.8)); // 14 × 1.2
        // tree_chrome_top = pad8 + (max(14,15*1.2)+9) + gap2 + (max(14*1.2+12+2, 26)+12) + gap2
        //                = 8 + (18.0+9) + 2 + (30.8+12) + 2 = 81.8
        // 字号基准抬到 14px(subtitle 15/body 14)后,head_h 与 header_h 各 +1.8/+1.2。
        assert!(near(tree_chrome_top_px(), 8.0 + 27.0 + 2.0 + 42.8 + 2.0));
        // tree_chrome_bottom = pad8 + gap2 + footer(1+4+(14+12)+12) = 8+2+43
        assert!(near(tree_chrome_bottom_px(), 8.0 + 2.0 + 43.0));
    }
}
