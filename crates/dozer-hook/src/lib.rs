//! 供 `dozer-app` 复用的 hook 安装逻辑（`install`/`opencode_install`/
//! `goose_install`/`aider_launcher`）与通用 HookEvent 发送（`forward`）。
//! 二进制入口是 `run_main(product)`：`dozer-hook` 与 Digger 的
//! `digger-hook` 都只是给它传各自 ProductId 的薄入口；CLI 分发、
//! 事件转发全部住在这里。安装/转发模块本身不含 CLI/stdin 相关代码，纯文件读写，
//! GUI 侧可以直接调用而不必 spawn 子进程。

dozer_core::scope!(pub(crate) LOG, module, "hook");

pub mod aider_bridge;
pub mod aider_launcher;
pub mod cli;
mod codebuddy;
mod codex;
mod goose;
mod opencode;
pub mod forward;
pub mod goose_install;
pub mod install;
pub mod opencode_install;

pub use cli::run_main;
