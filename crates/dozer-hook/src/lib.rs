//! 供 `dozer-app` 复用的 hook 安装逻辑（`install`/`opencode_install`/
//! `goose_install`）。二进制入口见 `main.rs`；这些模块本身不含 CLI/stdin
//! 相关代码，纯文件读写，GUI 侧可以直接调用而不必 spawn 子进程。

pub mod goose_install;
pub mod install;
pub mod opencode_install;
