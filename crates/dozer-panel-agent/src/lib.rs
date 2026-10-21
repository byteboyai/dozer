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
pub mod i18n;
pub mod launch;
pub mod picker;
pub mod state;
pub mod style;
pub mod transcript_tail;
pub mod tabs;
pub mod update;
