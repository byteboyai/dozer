//! 拖拽状态机与落点命中已迁入 `dozer-panel-files`(S5b-1)。这里只重新导出,
//! 并给 `tree_drop_target` 补一个使用产品主题几何量的包装,让既有调用点与
//! 测试不必各自传行高/间距。

use dozer_panel_files::fs::TreeRow;
pub(crate) use dozer_panel_files::tree::DropHit;
use iced_widget::core::Element;

/// `dozer_panel_files::tree::tree_drop_target` 的包装:行高与区域间距取自
/// 产品主题,和渲染侧同源。
pub(crate) fn tree_drop_target(
    x: f32,
    y: f32,
    bounds: (f32, f32, f32, f32),
    scroll: f32,
    rows: &[TreeRow],
) -> Option<DropHit> {
    dozer_panel_files::tree::tree_drop_target(
        x,
        y,
        bounds,
        scroll,
        rows,
        crate::theme::geometry::tree_row_h(),
        crate::theme::region::project_pane().gap,
    )
}

/// 拖拽幽灵胶囊:库版本发出的是库自己的 `Message`(胶囊纯展示),这里
/// 转成产品顶层 `Message`。
pub(crate) fn tree_drag_ghost<'a>(
    host: &impl dozer_panel_files::adapter::PanelDisplay,
    source: Option<(&std::path::Path, bool, usize)>,
) -> Element<'a, crate::app::Message, iced_widget::Theme, iced_renderer::Renderer> {
    dozer_panel_files::tree::tree_drag_ghost(host, source)
        .map(|_| unreachable!("the drag ghost emits no message"))
}
