mod controls;
mod scene;

use controls::Controls;
use scene::Scene;

use iced_wgpu::graphics::{Shell, Viewport};
use iced_wgpu::{Engine, Renderer, wgpu};
use iced_winit::Clipboard;
use iced_winit::conversion;
use iced_winit::core::mouse;
use iced_winit::core::renderer;
use iced_winit::core::time::Instant;
use iced_winit::core::window;
use iced_winit::core::{Event, Font, Pixels, Size, Theme};
use iced_winit::futures;
use iced_winit::runtime::user_interface::{self, UserInterface};
use iced_winit::winit;

use winit::{
    event::WindowEvent,
    event_loop::{ControlFlow, EventLoop},
    keyboard::ModifiersState,
};

use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{Rect, WebView, WebViewBuilder};

use std::sync::Arc;

/// 预览区占窗口右侧 40%（模拟左二 pane 的矩形区域），顶部留 40 逻辑像素
/// 给 iced 控件条让位。
fn preview_bounds(size: winit::dpi::PhysicalSize<u32>, scale: f64) -> Rect {
    let w = size.width as f64 / scale;
    let h = size.height as f64 / scale;
    Rect {
        position: LogicalPosition::new(w * 0.35, 40.0).into(),
        size: LogicalSize::new(w * 0.40, h - 40.0).into(),
    }
}

pub fn main() -> Result<(), winit::error::EventLoopError> {
    tracing_subscriber::fmt::init();

    // Initialize winit
    let event_loop = EventLoop::new()?;

    #[allow(clippy::large_enum_variant)]
    enum Runner {
        Loading,
        Ready {
            window: Arc<winit::window::Window>,
            queue: wgpu::Queue,
            device: wgpu::Device,
            surface: wgpu::Surface<'static>,
            format: wgpu::TextureFormat,
            renderer: Renderer,
            scene: Scene,
            controls: Controls,
            events: Vec<Event>,
            cursor: mouse::Cursor,
            cache: user_interface::Cache,
            clipboard: Clipboard,
            viewport: Viewport,
            modifiers: ModifiersState,
            resized: bool,
            webview: WebView,
            preview_visible: bool,
        },
    }

    impl winit::application::ApplicationHandler for Runner {
        fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
            if let Self::Loading = self {
                let window = Arc::new(
                    event_loop
                        .create_window(winit::window::WindowAttributes::default())
                        .expect("Create window"),
                );

                // 1) 窗口创建后，拿到 Arc<winit::window::Window> 处：叠加
                // wry 子视图（生产路径：webview 直接挂在 iced 自持的
                // winit 窗口上，而非 iced 独立管理一个 dummy 窗口）。
                let webview = WebViewBuilder::new()
                    .with_url("https://byteboy.ai")
                    .with_bounds(preview_bounds(window.inner_size(), window.scale_factor()))
                    .build_as_child(window.as_ref())
                    .expect("child webview over iced window");
                let preview_visible = true;

                let physical_size = window.inner_size();
                let viewport = Viewport::with_physical_size(
                    Size::new(physical_size.width, physical_size.height),
                    window.scale_factor() as f32,
                );
                let clipboard = Clipboard::connect(window.clone());

                let backend = wgpu::Backends::from_env().unwrap_or_default();

                let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                    backends: backend,
                    ..Default::default()
                });
                let surface = instance
                    .create_surface(window.clone())
                    .expect("Create window surface");

                let (format, adapter, device, queue) =
                    futures::futures::executor::block_on(async {
                        let adapter = wgpu::util::initialize_adapter_from_env_or_default(
                            &instance,
                            Some(&surface),
                        )
                        .await
                        .expect("Create adapter");

                        let adapter_features = adapter.features();

                        let capabilities = surface.get_capabilities(&adapter);

                        let (device, queue) = adapter
                            .request_device(&wgpu::DeviceDescriptor {
                                label: None,
                                required_features: adapter_features & wgpu::Features::default(),
                                required_limits: wgpu::Limits::default(),
                                memory_hints: wgpu::MemoryHints::MemoryUsage,
                                trace: wgpu::Trace::Off,
                                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                            })
                            .await
                            .expect("Request device");

                        (
                            capabilities
                                .formats
                                .iter()
                                .copied()
                                .find(wgpu::TextureFormat::is_srgb)
                                .or_else(|| capabilities.formats.first().copied())
                                .expect("Get preferred format"),
                            adapter,
                            device,
                            queue,
                        )
                    });

                surface.configure(
                    &device,
                    &wgpu::SurfaceConfiguration {
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        format,
                        width: physical_size.width,
                        height: physical_size.height,
                        present_mode: wgpu::PresentMode::AutoVsync,
                        alpha_mode: wgpu::CompositeAlphaMode::Auto,
                        view_formats: vec![],
                        desired_maximum_frame_latency: 2,
                    },
                );

                // Initialize scene and GUI controls
                let scene = Scene::new(&device, format);
                let controls = Controls::new();

                // Initialize iced

                let renderer = {
                    let engine = Engine::new(
                        &adapter,
                        device.clone(),
                        queue.clone(),
                        format,
                        None,
                        Shell::headless(),
                    );

                    Renderer::new(engine, Font::default(), Pixels::from(16))
                };

                // You should change this if you want to render continuously
                event_loop.set_control_flow(ControlFlow::Wait);

                *self = Self::Ready {
                    window,
                    device,
                    queue,
                    renderer,
                    surface,
                    format,
                    scene,
                    controls,
                    events: Vec::new(),
                    cursor: mouse::Cursor::Unavailable,
                    modifiers: ModifiersState::default(),
                    cache: user_interface::Cache::new(),
                    clipboard,
                    viewport,
                    resized: false,
                    webview,
                    preview_visible,
                };
            }
        }

        fn window_event(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
            _window_id: winit::window::WindowId,
            event: WindowEvent,
        ) {
            let Self::Ready {
                window,
                device,
                queue,
                surface,
                format,
                renderer,
                scene,
                controls,
                events,
                viewport,
                cursor,
                modifiers,
                clipboard,
                cache,
                resized,
                webview,
                preview_visible,
            } = self
            else {
                return;
            };

            match event {
                WindowEvent::RedrawRequested => {
                    if *resized {
                        let size = window.inner_size();

                        *viewport = Viewport::with_physical_size(
                            Size::new(size.width, size.height),
                            window.scale_factor() as f32,
                        );

                        surface.configure(
                            device,
                            &wgpu::SurfaceConfiguration {
                                format: *format,
                                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                                width: size.width,
                                height: size.height,
                                present_mode: wgpu::PresentMode::AutoVsync,
                                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                                view_formats: vec![],
                                desired_maximum_frame_latency: 2,
                            },
                        );

                        *resized = false;
                    }

                    match surface.get_current_texture() {
                        Ok(frame) => {
                            let view = frame
                                .texture
                                .create_view(&wgpu::TextureViewDescriptor::default());

                            let mut encoder =
                                device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                    label: None,
                                });

                            {
                                // Clear the frame
                                let mut render_pass =
                                    Scene::clear(&view, &mut encoder, controls.background_color());

                                // Draw the scene
                                scene.draw(&mut render_pass);
                            }

                            // Submit the scene
                            queue.submit([encoder.finish()]);

                            // Draw iced on top
                            let mut interface = UserInterface::build(
                                controls.view(),
                                viewport.logical_size(),
                                std::mem::take(cache),
                                renderer,
                            );

                            let (state, _) = interface.update(
                                &[Event::Window(
                                    window::Event::RedrawRequested(Instant::now()),
                                )],
                                *cursor,
                                renderer,
                                clipboard,
                                &mut Vec::new(),
                            );

                            // Update the mouse cursor
                            if let user_interface::State::Updated {
                                mouse_interaction, ..
                            } = state
                            {
                                // Update the mouse cursor
                                if let Some(icon) =
                                    iced_winit::conversion::mouse_interaction(mouse_interaction)
                                {
                                    window.set_cursor(icon);
                                    window.set_cursor_visible(true);
                                } else {
                                    window.set_cursor_visible(false);
                                }
                            }

                            // Draw the interface
                            interface.draw(
                                renderer,
                                &Theme::Dark,
                                &renderer::Style::default(),
                                *cursor,
                            );
                            *cache = interface.into_cache();

                            renderer.present(None, frame.texture.format(), &view, viewport);

                            // Present the frame
                            frame.present();
                        }
                        Err(error) => match error {
                            wgpu::SurfaceError::OutOfMemory => {
                                panic!(
                                    "Swapchain error: {error}. \
                                        Rendering cannot continue."
                                )
                            }
                            _ => {
                                // Try rendering again next frame.
                                window.request_redraw();
                            }
                        },
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    *cursor = mouse::Cursor::Available(conversion::cursor_position(
                        position,
                        viewport.scale_factor(),
                    ));
                }
                WindowEvent::ModifiersChanged(new_modifiers) => {
                    *modifiers = new_modifiers.state();
                }
                WindowEvent::Resized(new_size) => {
                    *resized = true;
                    // 4) webview 布局跟随窗口尺寸重算，不与 iced 的
                    // viewport/surface 重配置打架（各自独立触发）。
                    let _ = webview.set_bounds(preview_bounds(new_size, window.scale_factor()));
                }
                WindowEvent::CloseRequested => {
                    event_loop.exit();
                }
                _ => {}
            }

            // Map window event to iced event
            if let Some(event) =
                conversion::window_event(event, window.scale_factor() as f32, *modifiers)
            {
                events.push(event);
            }

            // If there are events pending
            if !events.is_empty() {
                // We process them
                let mut interface = UserInterface::build(
                    controls.view(),
                    viewport.logical_size(),
                    std::mem::take(cache),
                    renderer,
                );

                let mut messages = Vec::new();

                let _ = interface.update(events, *cursor, renderer, clipboard, &mut messages);

                events.clear();
                *cache = interface.into_cache();

                // update our UI with any messages
                for message in messages {
                    // 3) ToggleGate 的实际副作用（webview 显隐）在这里执行：
                    // webview 归 Runner::Ready 所有，与纯视图状态的
                    // Controls 分离，故不在 controls::update 内直接操作。
                    if let controls::Message::ToggleGate = message {
                        *preview_visible = !*preview_visible;
                        let _ = webview.set_visible(*preview_visible);
                    }
                    controls.update(message);
                }

                // and request a redraw
                window.request_redraw();
            }
        }
    }

    let mut runner = Runner::Loading;
    event_loop.run_app(&mut runner)
}
