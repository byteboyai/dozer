//! Files(项目文件树)面板:文件树 + 右键菜单/删除确认浮层。阶段 1 扩展化
//! 重构第四个试点,设计见
//! `docs/superpowers/specs/2026-08-07-files-extension-pilot-design.md`。

pub(crate) use dozer_panel_files::git_status;
pub(crate) mod state;
mod tree;
pub(crate) use tree::tree_drop_target;
mod update;
mod view;

pub(crate) use state::*;
pub(crate) use tree::*;
pub(crate) use update::*;
pub(crate) use view::*;

/// 本面板 i18n owner(`crate::i18n::PANEL_OWNERS` 之一;资源在
/// `locales/panels/files/`)。文件树右键菜单/确认框/底栏等文案都归它。
pub(crate) const FILES_OWNER: &str = "dozer.files";

/// 取一条本面板文案(见 `crate::i18n::panel_text`)。`fallback` 必须是英文,
/// 且与 `locales/panels/files/en.json` 一致。
pub(crate) fn t(key: &str, fallback: &str, locale: &str) -> String {
    crate::i18n::panel_text(FILES_OWNER, key, fallback, locale)
}

/// 同上,带 `{name}` 占位参数。
pub(crate) fn t_args(key: &str, fallback: &str, locale: &str, args: &[(&str, &str)]) -> String {
    crate::i18n::panel_text_args(FILES_OWNER, key, fallback, locale, args)
}
