//! 独立原生窗口共用的 wgpu 渲染管线建立/重配置——从 `search_overlay.rs`
//! 抽出,供 `search_overlay.rs`/`file_history_overlay.rs`(以及未来消费方)
//! 共用。原本不含 view/redraw/input 逻辑,各消费方自己写(见
//! `docs/superpowers/specs/2026-09-18-overlay-window-shared-abstraction-
//! design.md`「架构」第 1 节)——`2026-09-23-standard-dialog-overlay-
//! design.md` 迁移的 8 个"模态卡片"消费方(`ConfirmOverlay` 及 7 个定制
//! 宿主)让这个取舍走到了头:它们的 `redraw`/`handle_input` 除了"内容是
//! 什么"(各自的 `dialog::confirm`/`*_card` 调用)之外逐字相同。下面
//! `redraw`/`dispatch`/`track_and_convert` 三个方法只收敛这段纯机械的
//! 胶水(建 `UserInterface`→`update`/`draw`→存 cache→取帧/present;转换
//! `WindowEvent`→iced `Event`、顺带追踪 cursor/modifiers),`content`
//! 仍然是调用方自己拼的 `Element`——不是把"内容长什么样"也塞进来的泛型
//! `OverlayWindow<Msg>`(第一份设计与这次迁移的 spec 都明确否决过那种
//! 抽法,见两份文档各自的「架构」/「非目标」)。

use std::sync::Arc;

use iced_wgpu::graphics::{Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::Clipboard;
use iced_winit::conversion;
use iced_winit::core::time::Instant;
use iced_winit::core::window;
use iced_winit::core::{Color, Element, Event, Font, Pixels, Size, mouse};
use iced_winit::runtime::user_interface::{self, UserInterface};
use winit::event::WindowEvent;
use winit::keyboard::ModifiersState;
use winit::window::Window;

/// 按 iced 本帧算出的 `mouse_interaction` 刷新 overlay 窗口的鼠标光标。
///
/// 这扇窗口是独立原生窗口,主窗口那套 `apply_mouse_cursor`
/// (`window_events.rs`,含 webview 命中测试/顶栏守卫)不覆盖它——这里只做
/// 最基本的一件事:把光标形状写进窗口。缺了这步,弹窗里的按钮悬停时不会
/// 变成手形(2026-09-27 用户反馈)。
///
/// `OverlayGpu::redraw`/`dispatch` 内部已自动调用;自己手写 `UserInterface::
/// build`/`update` 的消费方(`search`/`file_history`/`settings`/
/// `project_create`)需在 `update` 拿到 `State::Updated` 后自行调用它。
pub(crate) fn apply_cursor(window: &Window, interaction: mouse::Interaction) {
    if let Some(icon) = conversion::mouse_interaction(interaction) {
        window.set_cursor(icon);
        window.set_cursor_visible(true);
    } else {
        window.set_cursor_visible(false);
    }
}

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
                // `Auto`在 Metal 后端会落到 `Opaque`(能力表只有
                // `[Opaque, PostMultiplied]`,`Auto` 走 wgpu-core 的兜底顺序
                // 优先选 `Opaque`),这会让 `CAMetalLayer.opaque = true`,
                // 合成时完全无视 alpha 通道——即使把内容清成
                // `Color::TRANSPARENT` 也只会显示纯黑而不是透明。显式指定
                // `PostMultiplied` 才能让窗口真正透明。
                alpha_mode: wgpu::CompositeAlphaMode::PostMultiplied,
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
                // 同 `open()`——见那边的注释,`reconfigure` 必须用一样的
                // `alpha_mode`,否则 resize 之后又退回不透明。
                alpha_mode: wgpu::CompositeAlphaMode::PostMultiplied,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            },
        );
    }

    /// `redraw` 方法体里"已经拿到这一帧的 `content`,要把它画出来"那一段
    /// 在 8 个模态卡片宿主之间逐字重复的部分:建 `UserInterface` →空事件
    /// `update`(只重算布局,不产生消息)→`draw`→存 cache→取帧→present。
    /// 调用方自己的 `redraw` 只需要拼出 `content`(如
    /// `dialog::confirm(self.spec.clone())`)再转调这个方法。
    pub(crate) fn redraw<Message>(
        &mut self,
        window: &Window,
        cursor: mouse::Cursor,
        content: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>,
    ) {
        let mut interface = UserInterface::build(
            content,
            self.viewport.logical_size(),
            std::mem::take(&mut self.cache),
            &mut self.renderer,
        );
        // 必须喂一个 `RedrawRequested` 事件而非空切片:iced 的 `button`/
        // `text_input` 只在处理这个事件时才把当帧算出的 hover/focused
        // `Status` 提交进 `draw()` 读的缓存字段(其它事件只会在状态变化时
        // `request_redraw`,不提交),空切片下 `draw()` 恒 fallback 到
        // `Status::Disabled` 的样式,hover/focus 视觉永远不生效——同
        // `window_events.rs` 主窗口重绘分支的写法。
        let (state, _) = interface.update(
            &[Event::Window(
                window::Event::RedrawRequested(Instant::now()),
            )],
            cursor,
            &mut self.renderer,
            &mut self.clipboard,
            &mut Vec::new(),
        );
        if let user_interface::State::Updated {
            mouse_interaction, ..
        } = state
        {
            apply_cursor(window, mouse_interaction);
        }
        interface.draw(
            &mut self.renderer,
            &iced_winit::core::Theme::Dark,
            &iced_winit::core::renderer::Style::default(),
            cursor,
        );
        self.cache = interface.into_cache();

        let Ok(frame) = self.surface.get_current_texture() else {
            window.request_redraw();
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        // `None`(`wgpu::LoadOp::Load`)不清空 surface——若这一帧的内容
        // 没有画满整扇窗口(如 `ConfirmOverlay` 的卡片高度按内容
        // shrink-fit,矮于固定 420×200 逻辑像素的窗口),未画到的区域会
        // 露出上一帧/未初始化纹理的内容,在这些透明弹窗窗口上表现为一块
        // 黑色矩形而不是透明。显式清成透明色,让内容之外的区域始终正确
        // 透出主窗口/webview 内容。
        self.renderer.present(
            Some(Color::TRANSPARENT),
            frame.texture.format(),
            &view,
            &self.viewport,
        );
        frame.present();
    }

    /// `handle_input` 方法体里"已经拿到转换后的 iced 事件,要建
    /// `UserInterface` 派发"那一段在 8 个模态卡片宿主之间逐字重复的
    /// 部分:建界面→带事件 `update`(收集消息)→存 cache→请求重绘→
    /// 返回消息。
    pub(crate) fn dispatch<Message>(
        &mut self,
        window: &Window,
        cursor: mouse::Cursor,
        content: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>,
        iced_event: Event,
    ) -> Vec<Message> {
        let events: [Event; 1] = [iced_event];
        let mut interface = UserInterface::build(
            content,
            self.viewport.logical_size(),
            std::mem::take(&mut self.cache),
            &mut self.renderer,
        );
        let mut messages = Vec::new();
        let (state, _) = interface.update(
            &events,
            cursor,
            &mut self.renderer,
            &mut self.clipboard,
            &mut messages,
        );
        if let user_interface::State::Updated {
            mouse_interaction, ..
        } = state
        {
            apply_cursor(window, mouse_interaction);
        }
        self.cache = interface.into_cache();
        window.request_redraw();
        messages
    }

    /// `handle_input` 开头"追踪 modifiers/cursor、把 winit 事件转换成 iced
    /// 事件"那一段在 8 个模态卡片宿主之间逐字重复的部分(不含 Esc 分支——
    /// 各宿主的取消消息不同,`ProjectScaffoldOverlay` 干脆没有 Esc 分支,
    /// 留给调用方自己处理)。返回 `None` 表示这个 winit 事件 iced 不关心
    /// (如未映射的按键),调用方应直接返回空消息列表,不需要再建
    /// `UserInterface`。
    pub(crate) fn track_and_convert(
        &self,
        cursor: &mut mouse::Cursor,
        modifiers: &mut ModifiersState,
        event: &WindowEvent,
    ) -> Option<Event> {
        if let WindowEvent::ModifiersChanged(new_modifiers) = event {
            *modifiers = new_modifiers.state();
        }
        if let WindowEvent::CursorMoved { position, .. } = event {
            *cursor = mouse::Cursor::Available(conversion::cursor_position(
                *position,
                self.viewport.scale_factor(),
            ));
        }
        conversion::window_event(event.clone(), self.viewport.scale_factor(), *modifiers)
    }
}
