//! 阶段 1 扩展化重构落地的模块目录:每个子模块拥有自己的 `Message`/
//! `State`/`update`/`view`,`workspace.rs` 内核只做包装转发(见
//! `docs/superpowers/specs/2026-08-07-git-log-extension-pilot-design.md`)。
//! 目前只有 `git_log` 一个试点;browser/todo 等面板视后续排期跟进。

pub mod acceptance;
pub mod browser;
pub mod database;
pub mod files;
pub mod git_log;
pub mod project;
pub mod todo;
pub mod usage;
