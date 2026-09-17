//! 整个程序只有一份的外壳态:daemon 客户端、窗口尺寸、图标栏/面板区的
//! 收起-拖宽-放大状态、并行打开的 N 个项目页签。`App::view()`/`update()`
//! 是 iced 应用的顶层入口,`Message` 是它的消息协议。
//!
//! 与 `Workspace`(单个项目的全部状态,见 `crate::workspace`)的拆分边界:
//! `impl Workspace` 的方法零处依赖 `App`(`ShellIo` 就是为此存在的解耦
//! 值类型),因此依赖方向是单向的——本文件 `use crate::workspace::
//! Workspace;`,`workspace.rs` 只反向借用本文件的 `Message` 一个类型
//! (`Workspace` 的异步方法要构造 `Message` 变体经 `EventLoopProxy`
//! 回传)。拆分细节见
//! `docs/superpowers/specs/2026-08-08-app-workspace-file-split-design.md`.

#[allow(clippy::module_inception)]
mod app;
mod layout;
mod message;
mod state;

pub(crate) use app::*;
pub(crate) use layout::*;
pub(crate) use message::*;
pub(crate) use state::*;
