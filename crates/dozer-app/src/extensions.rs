//! 阶段 1 扩展化重构落地的模块目录:每个子模块拥有自己的 `Message`/
//! `State`/`update`/`view`,`workspace.rs` 内核只做包装转发(见
//! `docs/superpowers/specs/2026-08-07-git-log-extension-pilot-design.md`)。
//! 目前只有 `git_log` 一个试点;browser/todo 等面板视后续排期跟进。

pub mod agent_context;
pub mod app_host;
pub mod app_logs;
pub mod browser;
pub mod codehealth;
pub mod conversations;
pub mod database;
pub mod diff_content;
pub mod diff_render;
pub mod edit_history;
pub mod file_history;
pub mod files;
pub mod footbar;
pub mod git_log;
pub mod group_chat;
pub mod project;
pub mod project_create;
pub mod search;
pub mod settings;
pub mod settings_apps;
pub mod settings_apps_view;
pub mod ssh;
pub mod toast;
pub mod todo;
pub mod usage;
