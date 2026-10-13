//! 独立原生窗口共用的建窗样板 + 居中定位算法——从 `search_overlay.rs`
//! 抽出,供各模态卡片宿主与 Toast 共用。
//!
//! **C10 方案 C**:建窗/定位/鼠标穿透/圆角遮罩几何(`bytehost_desktop::overlay`)
//! 与 wgpu 渲染管线(`OverlayGpu`)、遮罩外壳(`backdrop_card*`)
//! (`bytehost_desktop::overlay_gpu`,feature `overlay`)已全部上收宿主。本文件
//! 退化为**薄适配**:重导出宿主机制 + 给"需要产品主题 token"的两处
//! (`backdrop_card*` 的遮罩色、`open_overlay`/`reposition_overlay` 的 wgpu
//! 工厂)注入 dozer 侧的值,从而各消费方的调用点签名不变。
//!
//! dozer 侧只保留**产品语义**:遮罩色取自 `theme::region::maximize_overlay()`
//! (放大态浮层同款 token)。

use std::sync::Arc;

use iced_wgpu::wgpu;
use iced_winit::core::{Color, Element, Size};
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

// 渲染器无关的建窗/定位机制 + wgpu 管线类型,均来自宿主(C10 方案 C)。
pub(crate) use bytehost_desktop::overlay::{
    POPUP_WIDTH_FRACTION, centered_card_offset, full_window_overlay_bounds, open_child_window,
    open_child_window_unfocused, popup_card_size,
};
pub(crate) use bytehost_desktop::overlay_gpu::OverlayGpu;

/// 产品侧的遮罩色:放大态浮层同款 token(不是另起一个硬编码颜色)。
fn scrim() -> Color {
    crate::theme::region::maximize_overlay().scrim_background
}

/// 给卡片元素套一层"整窗口居中 + 半透明背景遮罩"的外壳——遮罩色由本适配层
/// 注入 `theme::region::maximize_overlay().scrim_background`;遮罩容器没有
/// `on_press`:点击穿不透到主窗体,但也不产生任何消息去关弹窗(已与用户确认:
/// 点遮罩=无反应,不是"点外部关闭")。机制见宿主 `overlay_gpu::backdrop_card`。
pub(crate) fn backdrop_card<'a, Message: 'a>(
    card: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    card_logical: LogicalSize<f32>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    bytehost_desktop::overlay_gpu::backdrop_card(
        card,
        Size::new(card_logical.width, card_logical.height),
        scrim(),
    )
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
    bytehost_desktop::overlay_gpu::backdrop_card_shrink(card, card_width, scrim())
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
