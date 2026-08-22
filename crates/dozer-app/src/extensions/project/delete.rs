//! "删除项目"三级方案的执行编排(spec 2026-08-22)。dozerd 侧两步
//! (`RemoveProject`/`DeleteProjectTranscripts`)先跑、失败即中止、不碰
//! 文件系统；文件系统三步(`.dozer`/agent 缓存目录/项目文件本身)在其后
//! 尽力而为，统一用 `trash::delete`(可从系统回收站找回)。

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeleteScope {
    /// 只删 dozer 登记 + `.dozer/` 缓存。
    DozerOnly,
    /// 含 `DozerOnly`，再删三家 agent 为这个项目缓存的历史数据。
    WithAgentCache,
    /// 含 `WithAgentCache`，再删项目文件本身(含 `.git`)。
    WithProjectFiles,
}

/// 跑一次完整的删除流程。`on_done` 收到的 `Vec<String>` 是文件系统步骤
/// 里各自独立失败的原因(空 = 全部成功)；dozerd 两步任一失败时，`on_done`
/// 只收到那一条错误、后续步骤(含文件系统步骤)都不会跑。调用方
/// (`app.rs::App::project_delete_confirm`)负责在调这个函数**之前**先把
/// 这个项目的 tab 关掉——这个函数本身不碰任何 `Workspace`/UI 状态，
/// 只认 `project_id`(用于 dozerd 请求，虽然当前两个请求都不需要
/// `project_id`，只需要 `cwd`——保留这个参数是为了跟调用方的日志/未来
/// 扩展对齐，不是死代码；如果实现阶段发现完全用不上，可以去掉)。
pub fn spawn_delete_project(
    _project_id: i64,
    repo_path: PathBuf,
    scope: DeleteScope,
    client: dozer_client::Client,
    handle: &tokio::runtime::Handle,
    on_done: impl Fn(Vec<String>) + Send + 'static,
) {
    let cwd = repo_path.to_string_lossy().into_owned();
    handle.spawn(async move {
        if let Err(e) = client.remove_project(_project_id).await {
            on_done(vec![format!("取消项目登记失败: {e}")]);
            return;
        }
        if scope != DeleteScope::DozerOnly
            && let Err(e) = client.delete_project_transcripts(&cwd).await
        {
            on_done(vec![format!("删除 agent 历史失败: {e}")]);
            return;
        }
        let repo_path_fs = repo_path.clone();
        let errors = tokio::task::spawn_blocking(move || {
            let mut errors = Vec::new();
            let dozer_dir = repo_path_fs.join(".dozer");
            if dozer_dir.exists()
                && let Err(e) = trash::delete(&dozer_dir)
            {
                errors.push(format!(".dozer 目录: {e}"));
            }
            if scope != DeleteScope::DozerOnly {
                let home = dozer_core::agent_paths::home_dir();
                let agent_dirs = [
                    (
                        "Claude 缓存",
                        dozer_core::agent_paths::claude_project_dir_in(&home, &repo_path_fs),
                    ),
                    (
                        "CodeBuddy 缓存",
                        dozer_core::agent_paths::codebuddy_project_dir_in(&home, &repo_path_fs),
                    ),
                    (
                        "OpenCode 缓存",
                        dozer_core::agent_paths::opencode_project_dir_in(&home, &repo_path_fs),
                    ),
                ];
                for (label, dir) in agent_dirs {
                    if dir.exists()
                        && let Err(e) = trash::delete(&dir)
                    {
                        errors.push(format!("{label}: {e}"));
                    }
                }
            }
            if scope == DeleteScope::WithProjectFiles
                && let Err(e) = trash::delete(&repo_path_fs)
            {
                errors.push(format!("项目文件: {e}"));
            }
            errors
        })
        .await
        .unwrap_or_else(|e| vec![format!("内部错误: {e}")]);
        on_done(errors);
    });
}
