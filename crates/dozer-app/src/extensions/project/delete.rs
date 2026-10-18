//! "删除项目"三级方案:分级与执行编排已搬进 `dozer-panel-project::delete`,
//! 这里只保留 Dozer 自己的 agent 缓存路径规则(`dozer_core::agent_paths`)
//! 并转调库函数,调用点签名不变。

pub use dozer_panel_project::delete::DeleteScope;

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 三家 agent 为项目缓存的目录(`dozer_core::agent_paths` 的路径规则留在
/// 产品侧,共享库不依赖 `dozer-core`)。
fn agent_cache_dir(kind: &'static str, repo: &Path) -> PathBuf {
    let home = dozer_core::agent_paths::home_dir();
    match kind {
        "claude" => dozer_core::agent_paths::claude_project_dir_in(&home, repo),
        "codebuddy" => dozer_core::agent_paths::codebuddy_project_dir_in(&home, repo),
        "opencode" => dozer_core::agent_paths::opencode_project_dir_in(&home, repo),
        _ => PathBuf::new(),
    }
}

/// 跑一次完整的删除流程。语义同 `dozer_panel_project::delete::spawn_delete_project`:
/// 调用方负责先关掉这个项目的 tab。
pub fn spawn_delete_project(
    project_id: i64,
    repo_path: PathBuf,
    scope: DeleteScope,
    project: Arc<dyn dozer_services::ProjectService>,
    transcript: Arc<dyn dozer_services::TranscriptService>,
    handle: &tokio::runtime::Handle,
    on_done: impl Fn(i64, bool, Vec<String>) + Send + 'static,
) {
    dozer_panel_project::delete::spawn_delete_project(
        project_id,
        repo_path,
        scope,
        project,
        Some(transcript),
        agent_cache_dir,
        handle,
        on_done,
    );
}
