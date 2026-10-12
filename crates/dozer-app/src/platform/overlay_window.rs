//! 独立原生窗口共用的建窗样板 + 居中定位算法——从 `search_overlay.rs`
//! 抽出,供各模态卡片宿主与 Toast 共用。
//!
//! **C10**:建窗/定位/鼠标穿透/圆角遮罩几何这些**与渲染器无关**的机制已上收
//! `bytehost_desktop::overlay`(本文件只重导出 + 保留两个接了 wgpu 管线的
//! 薄包装 `open_overlay`/`reposition_overlay`)。`OverlayGpu`(wgpu 渲染管线)
//! 与 `backdrop_card*`(依赖 `iced_widget`)按 C10 方案 B 留在产品侧。
//!
//! `open_overlay`/`reposition_overlay` 是各宿主共用的建窗口+建 GPU 管线、以及
//! resize/主窗口移动跟随两段胶水。IME/原生右键菜单挂靠仍由各消费方自己在拿到
//! `open_overlay` 返回的 `window` 后按需调用。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_widget::container;
use iced_widget::core::{Element, Length, alignment};
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::platform::overlay_gpu::OverlayGpu;

// 渲染器无关的建窗/定位机制重导出(C10 上收 bytehost-desktop::overlay)。
pub(crate) use bytehost_desktop::overlay::{
    BACKDROP_CORNER_RADIUS, POPUP_WIDTH_FRACTION, centered_card_offset, full_window_overlay_bounds,
    open_child_window, open_child_window_unfocused, popup_card_size, window_logical_size,
};

/// 给卡片元素套一层"整窗口居中 + 半透明背景遮罩"的外壳——遮罩色复用
/// `theme::region::maximize_overlay().scrim_background`(放大态浮层同款
/// token,不是另起一个硬编码颜色)。遮罩容器没有 `on_press`:点击穿不透
/// 到主窗体,但也不产生任何消息去关弹窗(已与用户确认:点遮罩=无反应,
/// 不是"点外部关闭")。遮罩圆角见 `BACKDROP_CORNER_RADIUS` 注释。
pub(crate) fn backdrop_card<'a, Message: 'a>(
    card: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    card_logical: LogicalSize<f32>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let scrim = crate::theme::region::maximize_overlay().scrim_background;
    container(
        container(card)
            .width(Length::Fixed(card_logical.width))
            .height(Length::Fixed(card_logical.height)),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(alignment::Horizontal::Center)
    .align_y(alignment::Vertical::Center)
    .style(move |_theme: &iced_widget::Theme| container::Style {
        background: Some(scrim.into()),
        border: iced_widget::core::Border {
            radius: BACKDROP_CORNER_RADIUS.into(),
            ..Default::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// 同 `backdrop_card`,但卡片**高度自适应内容**(`Length::Shrink`)——内容
/// 行数不定时不会被裁切(例:多文件删除确认要把所有文件名逐行列出来,项数
/// 不固定)。宽度仍由 `card_width` 固定(沿用 `POPUP_WIDTH_FRACTION`),避免
/// 卡片过宽;高度交给 iced 按真实排版结果撑开。overlay 窗口覆盖整扇主窗口
/// 客户区,足够大,居中后不会溢出可视范围。
pub(crate) fn backdrop_card_shrink<'a, Message: 'a>(
    card: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    card_width: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let scrim = crate::theme::region::maximize_overlay().scrim_background;
    container(
        container(card)
            .width(Length::Fixed(card_width))
            .height(Length::Shrink),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(alignment::Horizontal::Center)
    .align_y(alignment::Vertical::Center)
    .style(move |_theme: &iced_widget::Theme| container::Style {
        background: Some(scrim.into()),
        border: iced_widget::core::Border {
            radius: BACKDROP_CORNER_RADIUS.into(),
            ..Default::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// `open` 方法体里"建覆盖主窗口的子窗口 + 建这扇窗口自己的 wgpu 渲染管线"
/// 那两步在多个模态卡片宿主之间逐字重复的部分。建窗经宿主
/// [`bytehost_desktop::overlay::open_overlay`],渲染管线工厂在这里注入
/// (`OverlayGpu::open`)——宿主不认 wgpu 类型。
pub(crate) fn open_overlay(
    main_window: &Arc<Window>,
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    instance: &wgpu::Instance,
    tag: &str,
    el: &ActiveEventLoop,
) -> (Arc<Window>, OverlayGpu) {
    bytehost_desktop::overlay::open_overlay(main_window, tag, el, |window, size, scale| {
        OverlayGpu::open(window, instance, adapter, device, queue, size, scale)
    })
}

/// `reposition` 方法体在多个模态卡片宿主之间逐字重复的部分——主窗口移动
/// /resize 后跟着重新覆盖 + 重配置 surface。定位经宿主
/// [`bytehost_desktop::overlay::reposition_overlay`],surface 重配闭包在这里
/// 注入(`OverlayGpu::reconfigure`)。
pub(crate) fn reposition_overlay(
    window: &Window,
    gpu: &mut OverlayGpu,
    device: &wgpu::Device,
    main_outer_pos: PhysicalPosition<i32>,
    main_inner_size: PhysicalSize<u32>,
    scale: f64,
) {
    bytehost_desktop::overlay::reposition_overlay(
        window,
        main_outer_pos,
        main_inner_size,
        |size, scale| gpu.reconfigure(device, size, scale),
        scale,
    );
}
