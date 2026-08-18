//! amis `toast`(轻提示):<https://baidu.github.io/amis/zh-CN/components/toast>
//!
//! `ToastQueue` 是纯数据结构,不碰 iced 事件循环——消费方(如 dozer-app 的
//! `State`)自己持有它,在已有的动画 tick 里调 `retain_active`,在需要弹
//! 提示的地方调 `push`,决定把 `view(&queue)` 结果叠在哪一层。

use iced_widget::core::{Border, Element};
use iced_widget::{Column, container};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Success,
    Error,
    Info,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub kind: Kind,
    pub message: String,
    expires_at: Instant,
}

#[derive(Default, Debug)]
pub struct ToastQueue {
    items: Vec<Toast>,
}

impl ToastQueue {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn push(&mut self, kind: Kind, message: impl Into<String>, ttl: Duration) {
        self.items.push(Toast {
            kind,
            message: message.into(),
            expires_at: Instant::now() + ttl,
        });
    }

    pub fn retain_active(&mut self, now: Instant) {
        self.items.retain(|t| t.expires_at > now);
    }

    pub fn items(&self) -> &[Toast] {
        &self.items
    }
}

pub fn view<'a, Message: 'a>(
    queue: &ToastQueue,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = crate::theme::color::current();
    let items: Vec<_> = queue
        .items()
        .iter()
        .map(|t| {
            let accent = match t.kind {
                Kind::Success => colors.green,
                Kind::Error => colors.red,
                Kind::Info => colors.cyan,
            };
            container(
                iced_widget::text(t.message.clone())
                    .size(crate::theme::font::body())
                    .color(colors.cream),
            )
            .padding([8, 12])
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: Some(colors.card.into()),
                border: Border {
                    color: accent,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..container::Style::default()
            })
            .into()
        })
        .collect();
    Column::with_children(items).spacing(8).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retain_active_removes_expired_and_keeps_fresh() {
        let mut q = ToastQueue::new();
        q.push(Kind::Info, "old", Duration::from_millis(0));
        q.push(Kind::Success, "fresh", Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(5));
        q.retain_active(Instant::now());
        assert_eq!(q.items().len(), 1);
        assert_eq!(q.items()[0].message, "fresh");
    }

    #[test]
    fn retain_active_on_empty_queue_is_noop() {
        let mut q = ToastQueue::new();
        q.retain_active(Instant::now());
        assert!(q.items().is_empty());
    }

    #[test]
    fn push_appends_without_removing_existing_items() {
        let mut q = ToastQueue::new();
        q.push(Kind::Info, "a", Duration::from_secs(60));
        q.push(Kind::Error, "b", Duration::from_secs(60));
        assert_eq!(q.items().len(), 2);
    }
}
