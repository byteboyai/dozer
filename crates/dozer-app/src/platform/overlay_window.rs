//! 独立原生窗口共用的建窗样板 + 居中定位算法——从 `search_overlay.rs`
//! 抽出,供 `search_overlay.rs`/`file_history_overlay.rs`(以及未来消费方)
//! 共用。

use std::sync::Arc;

use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowLevel};

/// 主窗口外框物理位置 + 物理尺寸 + scale + 卡片逻辑尺寸 → overlay 应放的
/// 物理位置与物理尺寸(居中于主窗口)。纯函数,不碰真实 `Window`,方便测试。
pub(crate) fn centered_overlay_bounds(
    main_outer_pos: PhysicalPosition<i32>,
    main_inner_size: PhysicalSize<u32>,
    scale: f64,
    card_logical_size: LogicalSize<f32>,
) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let card_w = card_logical_size.width as f64 * scale;
    let card_h = card_logical_size.height as f64 * scale;
    let x = main_outer_pos.x as f64 + (main_inner_size.width as f64 - card_w) / 2.0;
    let y = main_outer_pos.y as f64 + (main_inner_size.height as f64 - card_h) / 2.0;
    (
        PhysicalPosition::new(x.round() as i32, y.round() as i32),
        PhysicalSize::new(card_w.round() as u32, card_h.round() as u32),
    )
}

/// 挂成主窗口子窗口 + `AlwaysOnTop` 的无装饰透明窗口。只建窗口本身,不建
/// wgpu 渲染管线(见 `OverlayGpu`)、不设 IME/原生菜单挂靠(消费方按需
/// 自己调用,大多数消费方不需要——见 spec「架构」第 1 节)。
pub(crate) fn open_child_window(
    main_window: &Window,
    pos: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    title: &str,
    el: &ActiveEventLoop,
) -> Arc<Window> {
    use winit::raw_window_handle::HasWindowHandle;

    let parent_handle = main_window
        .window_handle()
        .expect("main window handle")
        .as_raw();
    let attrs = Window::default_attributes()
        .with_title(title)
        .with_decorations(false)
        .with_transparent(true)
        .with_window_level(WindowLevel::AlwaysOnTop)
        .with_position(pos)
        .with_inner_size(size);
    // Safety: `parent_handle` 取自仍存活的主窗口(`Ready` 持有的
    // `Arc<Window>`),本函数返回前主窗口不会被 drop。
    let attrs = unsafe { attrs.with_parent_window(Some(parent_handle)) };
    let window = Arc::new(el.create_window(attrs).expect("create overlay window"));
    window.focus_window();
    window
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_overlay_bounds_centers_within_main_window() {
        let (pos, size) = centered_overlay_bounds(
            PhysicalPosition::new(100, 50),
            PhysicalSize::new(1200, 800),
            2.0, // Retina 2x
            LogicalSize::new(400.0, 640.0),
        );
        // 卡片物理尺寸 = 逻辑尺寸 * scale。
        assert_eq!(size, PhysicalSize::new(800, 1280));
        // 居中:主窗口物理宽 1200,卡片物理宽 800 → 左右各留 200。
        assert_eq!(pos.x, 100 + 200);
        assert_eq!(pos.y, 50 + (800 - 1280) / 2);
    }

    #[test]
    fn centered_overlay_bounds_at_scale_one() {
        let (pos, size) = centered_overlay_bounds(
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1000, 1000),
            1.0,
            LogicalSize::new(600.0, 640.0),
        );
        assert_eq!(size, PhysicalSize::new(600, 640));
        assert_eq!(pos.x, (1000 - 600) / 2);
        assert_eq!(pos.y, (1000 - 640) / 2);
    }
}
