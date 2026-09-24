//! 独立原生窗口共用的建窗样板 + 居中定位算法——从 `search_overlay.rs`
//! 抽出,供 `search_overlay.rs`/`file_history_overlay.rs`(以及未来消费方)
//! 共用。`open_overlay`/`reposition_overlay` 是 `2026-09-23-standard-
//! dialog-overlay-design.md` 迁移的 8 个模态卡片宿主共用的建窗口+建 GPU
//! 管线、以及 resize/主窗口移动跟随两段胶水(这两段在那 8 个消费方的
//! `open`/`reposition` 方法体里逐字重复,唯一变量是各自的
//! `card_logical_size()`/窗口 tag)。IME/原生右键菜单挂靠仍由各消费方
//! 自己在拿到 `open_overlay` 返回的 `window` 后按需调用——不是每个消费方
//! 都需要,不塞进这个共用函数。

use std::sync::Arc;

use iced_wgpu::wgpu;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowLevel};

use crate::platform::overlay_gpu::OverlayGpu;

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

/// `open` 方法体里"建居中子窗口 + 建这扇窗口自己的 wgpu 渲染管线"那两步
/// 在 8 个模态卡片宿主之间逐字重复的部分。调用方自己的 `open` 只需要传
/// 各自的 `card_logical_size()`/窗口 tag,再按需对返回的 `window` 调
/// `set_ime_allowed`/`install_content_view`、拼自己结构体里其余字段。
/// 8 个参数但两两不同类型(`Arc<Window>`/`&Adapter`/`&Device`/`&Queue`/
/// `&Instance`/`LogicalSize`/`&str`/`&ActiveEventLoop`),传错顺序编译器
/// 会直接报错而非静默接受——不属于 CLAUDE.md 那条"具名字段参数结构体"
/// 规则要防的"相邻同类型参数传反"场景,不加 `#[allow]`,留下这条
/// `too_many_arguments` warning(同各消费方自己的 `open` 现状一致)。
pub(crate) fn open_overlay(
    main_window: &Arc<Window>,
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    instance: &wgpu::Instance,
    card_logical: LogicalSize<f32>,
    tag: &str,
    el: &ActiveEventLoop,
) -> (Arc<Window>, OverlayGpu) {
    let scale = main_window.scale_factor();
    let (pos, size) = centered_overlay_bounds(
        main_window
            .outer_position()
            .unwrap_or(PhysicalPosition::new(0, 0)),
        main_window.inner_size(),
        scale,
        card_logical,
    );
    let window = open_child_window(main_window, pos, size, tag, el);
    let gpu = OverlayGpu::open(&window, instance, adapter, device, queue, size, scale);
    (window, gpu)
}

/// `reposition` 方法体在 8 个模态卡片宿主之间逐字重复的部分——唯一变量
/// 是各自的 `card_logical_size()`。
pub(crate) fn reposition_overlay(
    window: &Window,
    gpu: &mut OverlayGpu,
    device: &wgpu::Device,
    main_outer_pos: PhysicalPosition<i32>,
    main_inner_size: PhysicalSize<u32>,
    scale: f64,
    card_logical: LogicalSize<f32>,
) {
    let (pos, size) = centered_overlay_bounds(main_outer_pos, main_inner_size, scale, card_logical);
    window.set_outer_position(pos);
    if window.inner_size() != size {
        let _ = window.request_inner_size(size);
        gpu.reconfigure(device, size, scale);
    }
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
