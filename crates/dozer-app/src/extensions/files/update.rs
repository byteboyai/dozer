//! 文件树面板的 `update` 已随代码迁入 `dozer-panel-files`(S5b-1)。这里只
//! 重新导出,让既有 `files::update(..)` / `files::load_git_info(..)` 调用点
//! 保持不变。

dozer_core::scope!(pub(crate) LOG, panel, "files");

#[allow(clippy::too_many_arguments)]
pub(crate) fn update(
    ws_state: &mut dozer_panel_files::state::WorkspaceState,
    app_state: &mut dozer_panel_files::state::AppState,
    msg: dozer_panel_files::Message,
    project_id: i64,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(dozer_panel_files::Message) + Send + 'static,
    external_apps: &crate::external_apps::ExternalAppsConfig,
    caps: &dozer_panel_files::adapter::FilesCapabilities,
    locale: &str,
) {
    dozer_panel_files::update::update(
        ws_state,
        app_state,
        msg,
        project_id,
        handle,
        emit,
        external_apps,
        caps,
        locale,
    );
}
