//! 独立原生窗口共用的 wgpu 渲染管线建立/重配置——从 `search_overlay.rs`
//! 抽出,供 `search_overlay.rs`/`file_history_overlay.rs`(以及未来消费方)
//! 共用。不含 view/redraw/input 逻辑,那些各消费方自己写(见
//! `docs/superpowers/specs/2026-09-18-overlay-window-shared-abstraction-
//! design.md`「架构」第 1 节)。

use std::sync::Arc;

use iced_wgpu::graphics::{Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::Clipboard;
use iced_winit::core::{Font, Pixels, Size};
use iced_winit::runtime::user_interface;
use winit::window::Window;

/// 独立原生窗口自己的一份渲染资源——不含 `Device`/`Queue`/`Adapter`/
/// `Instance`(全部从主窗口 `Ready` 借来的共享句柄,`Device`/`Queue`
/// 便宜 `Clone`),`iced_wgpu` 的 `Renderer` 内部持有 `Engine`,不能跨
/// 窗口共享,每扇窗口必须自己一份。字段 `pub(crate)`:消费方
/// (`search_overlay.rs`/`file_history_overlay.rs`)需要在各自的
/// `redraw`/`handle_input` 里对单个字段做细粒度可变借用(比如同时借
/// `&mut renderer` 和 `&viewport`),包一层 getter 反而更啰嗦。
pub(crate) struct OverlayGpu {
    pub(crate) surface: wgpu::Surface<'static>,
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) renderer: Renderer,
    pub(crate) cache: user_interface::Cache,
    pub(crate) viewport: Viewport,
    pub(crate) clipboard: Clipboard,
}

impl OverlayGpu {
    /// 从共享的 `Instance`/`Adapter`/`Device`/`Queue` 为 `window` 建一份
    /// 独立渲染管线,`size`/`scale` 是这扇窗口当前的物理尺寸/缩放。
    pub(crate) fn open(
        window: &Arc<Window>,
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: winit::dpi::PhysicalSize<u32>,
        scale: f64,
    ) -> OverlayGpu {
        let surface = instance
            .create_surface(window.clone())
            .expect("create overlay surface");
        let capabilities = surface.get_capabilities(adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| capabilities.formats.first().copied())
            .expect("get overlay surface format");
        surface.configure(
            device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: size.width.max(1),
                height: size.height.max(1),
                present_mode: wgpu::PresentMode::AutoVsync,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            },
        );

        let engine = Engine::new(
            adapter,
            device.clone(),
            queue.clone(),
            format,
            None,
            Shell::headless(),
        );
        let renderer = Renderer::new(engine, Font::default(), Pixels::from(16));
        let viewport =
            Viewport::with_physical_size(Size::new(size.width, size.height), scale as f32);
        let clipboard = Clipboard::connect(window.clone());

        OverlayGpu {
            surface,
            format,
            renderer,
            cache: user_interface::Cache::new(),
            viewport,
            clipboard,
        }
    }

    /// resize 后重配置 surface + viewport(不重建 `Engine`/`Renderer`)。
    /// 调用方负责判断"要不要重配"(比如只在窗口实际物理尺寸变化时调,
    /// 见 `SearchOverlay::reposition`/`FileHistoryOverlay::reposition`)。
    pub(crate) fn reconfigure(
        &mut self,
        device: &wgpu::Device,
        size: winit::dpi::PhysicalSize<u32>,
        scale: f64,
    ) {
        self.viewport =
            Viewport::with_physical_size(Size::new(size.width, size.height), scale as f32);
        self.surface.configure(
            device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: self.format,
                width: size.width.max(1),
                height: size.height.max(1),
                present_mode: wgpu::PresentMode::AutoVsync,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            },
        );
    }
}
