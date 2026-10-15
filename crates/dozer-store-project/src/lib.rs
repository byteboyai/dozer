//! Project 注册表与 Project 级共享记忆的存储。
//!
//! 同一份代码被 `dozerd`(经 UDS 对外服务)与 Digger(进程内直接使用)共用;两个产品各自打开自己的
//! SQLite 文件,运行时互不相识。表结构与搬走前 `dozerd` 里的 `projects` / `memories` / `memory_history` 逐字一致。

mod memory;
mod projects;

pub use memory::MemoryStore;
pub use projects::ProjectStore;
