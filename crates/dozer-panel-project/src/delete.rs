//! "删除项目"三级方案的执行编排(spec 2026-08-22)。dozerd 侧两步
//! (`RemoveProject`/`DeleteProjectTranscripts`)先跑、失败即中止、不碰
//! 文件系统；文件系统三步(`.dozer`/agent 缓存目录/项目文件本身)在其后
//! 尽力而为，统一用 `trash::delete`(可从系统回收站找回)。
//!
//! 产品差异只有一处:调用方在构造本库的 `Services` 时,不装 `transcript`
//! 功能的面板库(Digger)可以传 `transcript = None`——此时分级里的转录那一档
//! 不出现,`WithAgentCache`/`WithProjectFiles` 跳过转录删除这一步。

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

/// 删除弹窗里可选的层级。有转录服务时三档都在;没有转录服务时 `WithAgentCache`
/// 仍然可选(它还会删 agent 缓存目录),只是少了转录那一半。
pub fn delete_scopes() -> Vec<DeleteScope> {
    vec![
        DeleteScope::DozerOnly,
        DeleteScope::WithAgentCache,
        DeleteScope::WithProjectFiles,
    ]
}

/// 跑一次完整的删除流程。`on_done(project_id, deregistered, errors)`:
/// `deregistered` = dozerd 侧登记已成功取消(`RemoveProject` 通过)——
/// 无论后续步骤是否失败,登记一取消项目就该从各 recent 列表里消失;
/// `errors` 是文件系统步骤里各自独立失败的原因(空 = 全部成功);dozerd
/// 两步任一失败时,`on_done` 只收到那一条错误、且 `deregistered=false`、
/// 后续步骤(含文件系统步骤)都不会跑。调用方
/// 负责在调这个函数**之前**先把这个项目的 tab 关掉——这个函数本身不碰任何
/// `Workspace`/UI 状态。
///
/// `transcript = None`(Digger)时跳过转录删除;`agent_dir(label, repo)`
/// 由产品给出三家 agent 缓存目录(`dozer-core` 的路径规则留在产品侧,库不依赖
/// 它)。
#[allow(clippy::too_many_arguments)]
pub fn spawn_delete_project(
    project_id: i64,
    repo_path: PathBuf,
    scope: DeleteScope,
    project: std::sync::Arc<dyn dozer_services::ProjectService>,
    transcript: Option<std::sync::Arc<dyn dozer_services::TranscriptService>>,
    agent_dir: impl Fn(&'static str, &std::path::Path) -> PathBuf + Send + 'static,
    handle: &tokio::runtime::Handle,
    on_done: impl Fn(i64, bool, Vec<String>) + Send + 'static,
) {
    let cwd = repo_path.to_string_lossy().into_owned();
    handle.spawn(async move {
        if let Err(e) = project.remove_project(project_id).await {
            on_done(project_id, false, vec![format!("Deregister project: {e}")]);
            return;
        }
        if scope != DeleteScope::DozerOnly
            && let Some(transcript) = transcript
            && let Err(e) = transcript.delete_project_transcripts(cwd.clone()).await
        {
            on_done(project_id, true, vec![format!("Delete agent history: {e}")]);
            return;
        }
        let repo_path_fs = repo_path.clone();
        let errors = tokio::task::spawn_blocking(move || {
            let mut errors = Vec::new();
            let dozer_dir = repo_path_fs.join(".dozer");
            if dozer_dir.exists()
                && let Err(e) = trash::delete(&dozer_dir)
            {
                errors.push(format!(".dozer directory: {e}"));
            }
            if scope != DeleteScope::DozerOnly {
                let agent_dirs = [
                    ("Claude cache", agent_dir("claude", &repo_path_fs)),
                    ("CodeBuddy cache", agent_dir("codebuddy", &repo_path_fs)),
                    ("OpenCode cache", agent_dir("opencode", &repo_path_fs)),
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
                errors.push(format!("Project files: {e}"));
            }
            errors
        })
        .await
        .unwrap_or_else(|e| vec![format!("Internal error: {e}")]);
        on_done(project_id, true, errors);
    });
}
