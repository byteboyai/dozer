use iced_wgpu::Renderer;
use iced_widget::{bottom, button, column, row, slider, text, text_input};
use iced_winit::core::{Color, Element, Theme};

pub struct Controls {
    background_color: Color,
    input: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    BackgroundColorChanged(Color),
    InputChanged(String),
    ToggleGate, // 模拟 ⌘K：隐藏/显示预览 webview
}

impl Controls {
    pub fn new() -> Controls {
        Controls {
            background_color: Color::BLACK,
            input: String::default(),
        }
    }

    pub fn background_color(&self) -> Color {
        self.background_color
    }
}

impl Controls {
    pub fn update(&mut self, message: Message) {
        match message {
            Message::BackgroundColorChanged(color) => {
                self.background_color = color;
            }
            Message::InputChanged(input) => {
                self.input = input;
            }
            Message::ToggleGate => {
                // 实际的 wry WebView::set_visible 由 main.rs 的 Runner::Ready
                // 执行（webview 与 wgpu surface 同生命周期，不归属这个纯视图
                // 状态的 Controls）；这里留空分支以保持穷尽匹配，参见
                // main.rs 中消息派发处的拦截逻辑。
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message, Theme, Renderer> {
        let background_color = self.background_color;

        let sliders = row![
            slider(0.0..=1.0, background_color.r, move |r| {
                Message::BackgroundColorChanged(Color {
                    r,
                    ..background_color
                })
            })
            .step(0.01),
            slider(0.0..=1.0, background_color.g, move |g| {
                Message::BackgroundColorChanged(Color {
                    g,
                    ..background_color
                })
            })
            .step(0.01),
            slider(0.0..=1.0, background_color.b, move |b| {
                Message::BackgroundColorChanged(Color {
                    b,
                    ..background_color
                })
            })
            .step(0.01),
        ]
        .width(500)
        .spacing(20);

        bottom(
            column![
                text("Background color").color(Color::WHITE),
                text!("{background_color:?}").size(14).color(Color::WHITE),
                sliders,
                text_input("Type something...", &self.input)
                    .on_input(Message::InputChanged),
                button(text("Toggle Preview (⌘K)")).on_press(Message::ToggleGate),
            ]
            .spacing(10),
        )
        .padding(10)
        .into()
    }
}
