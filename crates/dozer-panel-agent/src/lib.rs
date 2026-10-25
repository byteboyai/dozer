//! 共享 Agent 面板库(S6c)。
//!
//! S6c-1 包含:会话服务接缝的消费端、页签/会话状态、消息与更新(不含
//! 卡片列表、选择器菜单内容与面板外壳视图——那些是 S6c-2)。
//!
//! 依赖闭包不含 `dozer-app` / `dozer-client` / `dozer-core` / `dozerd` /
//! `wry` / `winit` / `portable-pty` / `russh`:所有产品专属语义(守护进程
//! 客户端、hook 安装器、原生菜单、退出前等待)都经注入的 trait / 闭包提供。

pub mod adapter;
pub mod card;
pub mod codex_config;
pub mod display;
pub mod i18n;
pub mod launch;
pub mod picker;
pub mod restore;
pub mod state;
pub mod style;
pub mod tabs;
pub mod transcript_tail;
pub mod update;
pub mod view;

/// 将终端画布事件映射为 Agent 面板消息。
pub fn term_message(event: dozer_panel_term::view::TermEvent) -> update::Message {
    use dozer_panel_term::view::TermEvent;

    match event {
        TermEvent::Input(bytes) => update::Message::Input(bytes),
        TermEvent::Scroll(delta) => update::Message::Scroll(delta),
        TermEvent::SelStart {
            col,
            row,
            right_half,
        } => update::Message::SelStart {
            col,
            row,
            right: right_half,
        },
        TermEvent::SelUpdate {
            col,
            row,
            right_half,
        } => update::Message::SelUpdate {
            col,
            row,
            right: right_half,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_panel_term::view::TermEvent;

    #[test]
    fn term_message_maps_every_term_event_variant() {
        assert_eq!(
            term_message(TermEvent::Input(vec![1])),
            update::Message::Input(vec![1])
        );
        assert_eq!(
            term_message(TermEvent::Scroll(3)),
            update::Message::Scroll(3)
        );
        assert_eq!(
            term_message(TermEvent::SelStart {
                col: 1,
                row: 2,
                right_half: true
            }),
            update::Message::SelStart {
                col: 1,
                row: 2,
                right: true
            }
        );
        assert_eq!(
            term_message(TermEvent::SelUpdate {
                col: 3,
                row: 4,
                right_half: false
            }),
            update::Message::SelUpdate {
                col: 3,
                row: 4,
                right: false
            }
        );
    }
}
