//! 预览域状态机(P1d):左二 tabs、webview 期望清单。纯数据,不碰
//! wry/iced——webview 副作用由 main.rs 对照 `desired_webviews()` 差集
//! 执行(spike 约束:句柄只活在事件分发环)。
//!
//! 地址栏/URL tab(`TabKind::Web`)、`AddrTarget` 这套逻辑已经随浏览器
//! 面板扩展化(`extensions::browser::Tabs`)搬走——文件预览面板从来没有
//! 地址栏,这里只保留文件/验收两种 tab。

mod native_editor;
mod state;
mod view;
mod webview;

pub(crate) use native_editor::*;
pub(crate) use state::*;
pub(crate) use view::*;
pub(crate) use webview::*;
