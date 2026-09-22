//! 预览域状态机(P1d):左二 tabs、webview 期望清单。纯数据,不碰
//! wry/iced——webview 副作用由 main.rs 对照 `desired_webviews()` 差集
//! 执行(spike 约束:句柄只活在事件分发环)。
//!
//! 地址栏/URL tab(`TabKind::Web`)、`AddrTarget` 这套逻辑已经随浏览器
//! 面板扩展化(`extensions::browser::Tabs`)搬走——文件预览面板从来没有
//! 地址栏,这里只保留文件/验收两种 tab。

mod backend;
mod code_host;
mod file_profile;
mod native_editor;
mod router;
mod state;
mod text_save;
mod view;
mod webview;
mod webview_protocol;

pub(crate) use backend::*;
pub(crate) use file_profile::*;
pub(crate) use native_editor::*;
pub(crate) use router::*;
pub(crate) use state::*;
pub(crate) use text_save::*;
pub(crate) use view::*;
pub(crate) use webview::*;
// `code_host` / `webview_protocol` 是 Phase B 新增的通用契约模块,当前只在
// 本模块内(及各自单测)使用;host 运行时接线完成后再对外 re-export。
#[allow(unused_imports)]
pub(crate) use code_host::*;
#[allow(unused_imports)]
pub(crate) use webview_protocol::*;
