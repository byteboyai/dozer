//! Spike A：验证 wry WebView 作为 winit 窗口子视图叠加。
//! 验收：右半区渲染网页；窗口 resize 时 bounds 跟随；按 `h` 隐藏/显示 webview；
//!       webview 内滚动、点击、文本框输入（含中文 IME）正常。
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::Key;
use winit::window::{Window, WindowId};
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{Rect, WebView, WebViewBuilder};

const PREVIEW_URL: &str = "https://byteboy.ai";

#[derive(Default)]
struct App {
    window: Option<Window>,
    webview: Option<WebView>,
    visible: bool,
}

fn right_half(size: winit::dpi::PhysicalSize<u32>, scale: f64) -> Rect {
    let w = size.width as f64 / scale;
    let h = size.height as f64 / scale;
    Rect {
        position: LogicalPosition::new(w / 2.0, 0.0).into(),
        size: LogicalSize::new(w / 2.0, h).into(),
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        let window = el
            .create_window(
                Window::default_attributes()
                    .with_title("Spike A: wry child over winit")
                    .with_inner_size(LogicalSize::new(1200.0, 800.0)),
            )
            .expect("create window");
        let bounds = right_half(window.inner_size(), window.scale_factor());
        let webview = WebViewBuilder::new()
            .with_url(PREVIEW_URL)
            .with_bounds(bounds)
            .build_as_child(&window)
            .expect("build child webview");
        self.window = Some(window);
        self.webview = Some(webview);
        self.visible = true;
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                if let (Some(w), Some(wv)) = (&self.window, &self.webview) {
                    let _ = wv.set_bounds(right_half(size, w.scale_factor()));
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: Key::Character(ref c),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } if c.as_str() == "h" => {
                // 模拟"⌘K 打开命令面板 → 隐藏预览"
                if let Some(wv) = &self.webview {
                    self.visible = !self.visible;
                    let _ = wv.set_visible(self.visible);
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.run_app(&mut App::default()).expect("run app");
}
