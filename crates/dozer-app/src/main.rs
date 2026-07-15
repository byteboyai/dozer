use iced::widget::{container, text};
use iced::{Color, Element, Theme};

// ByteBoy2077 tokens（P1c 提炼进 theme 模块）
const BG: Color = Color::from_rgb(0.039, 0.055, 0.086); // #0a0e16
const CREAM: Color = Color::from_rgb(1.0, 0.898, 0.706); // #FFE5B4

#[derive(Default)]
struct App;

#[derive(Debug, Clone)]
enum Message {}

fn update(_state: &mut App, _message: Message) {}

fn view(_state: &App) -> Element<'_, Message> {
    container(text("Dozer — P1a 骨架").size(24).color(CREAM))
        .center_x(iced::Fill)
        .center_y(iced::Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(BG.into()),
            ..container::Style::default()
        })
        .into()
}

fn main() -> iced::Result {
    iced::application(|| App, update, view).run()
}
