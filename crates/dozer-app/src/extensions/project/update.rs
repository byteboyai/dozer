//! 项目面板的 update 消息分发 + scaffold 运行/repair/名称提交的异步 spawn。

use super::*;
use crate::panel_host::RequestTokenWrapExt;
use dozer_services::{MemoryService, ProjectService, ServiceErrorKind, Services, TranscriptService};
use std::sync::Arc;

dozer_core::scope!(LOG, panel, "project");

/// 处理全部消息——本模块不触碰终端会话域,没有需要内核拦截、`update` 里
/// `unreachable!` 的消息(不像 Files);改名需要 daemon 往返,走
/// `handle`/`emit`。
#[allow(clippy::too_many_arguments)]
pub fn update(
    ws_state: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    current_name: &str,
    repo_path: &std::path::Path,
    services: &Services,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let mut msg = msg;
    while let Message::AsyncResult(token, inner) = msg {
        if token.scope() != &crate::panel_host::scope_of(Some(project_id))
            || !ws_state.requests.accepts(&token)
        {
            return;
        }
        msg = *inner;
    }
    match msg {
        Message::AsyncResult(..) => unreachable!("nested async result"),
        Message::Noop => {}
        Message::ToolbarHover(..) => {
            unreachable!("由内核拦截处理,见 project::Message::ToolbarHover 文档")
        }
        Message::GitRefreshed(_, branch, dirty, remote_url) => {
            ws_state.branch = branch;
            ws_state.dirty = dirty;
            ws_state.remote_url = remote_url;
        }
        Message::DiskUsageLoaded(_, bytes) => {
            ws_state.disk_usage_bytes = Some(bytes);
        }
        Message::NameEditStart => {
            ws_state.name_editing = Some(current_name.to_string());
            ws_state.name_edit_focus_pending = true;
        }
        Message::NameEditInput(s) => {
            if let Some(buf) = &mut ws_state.name_editing {
                *buf = s;
            }
        }
        Message::NameEditSubmit => {
            submit_name_edit(
                ws_state,
                project_id,
                current_name,
                services.project.clone(),
                handle,
                emit,
            );
        }
        Message::NameRenamed(_, result) => match result {
            Ok(_) => {
                ws_state.name_editing = None;
                ws_state.error = None;
            }
            Err(e) => {
                ws_state.error = Some(if let Some(raw) = e {
                    keyed_args(
                        "error.rename_failed",
                        "Rename failed: {error}",
                        &[("error", &raw)],
                    )
                } else {
                    keyed("error.project_not_found", "Project not found")
                });
                // 保留编辑态原始输入,允许重试。
            }
        },
        Message::DescriptionEditStart => {
            let initial = ws_state.description.clone().unwrap_or_default();
            ws_state.description_editing =
                Some(iced_widget::text_editor::Content::with_text(&initial));
        }
        Message::TextInputMenuOpen(_) => {
            unreachable!("由内核拦截处理,见 project::Message::TextInputMenuOpen 文档")
        }
        Message::DescriptionEditAction(action) => {
            if let Some(content) = &mut ws_state.description_editing {
                content.perform(action);
            }
        }
        Message::DescriptionEditSubmit => {
            let Some(content) = ws_state.description_editing.take() else {
                return;
            };
            let text = content.text().trim().to_string();
            match crate::project_meta::write_description(repo_path, &text) {
                Ok(()) => {
                    ws_state.description = if text.is_empty() { None } else { Some(text) };
                    ws_state.error = None;
                }
                Err(e) => {
                    ws_state.error = Some(keyed_args(
                        "error.save_failed",
                        "Save failed: {error}",
                        &[("error", &e.to_string())],
                    ));
                    ws_state.description_editing = Some(content); // 保留编辑态允许重试
                }
            }
        }
        Message::LinkAdd { target, path, kind } => {
            let already_present = ws_state.links.list(target).iter().any(|e| e.path == path);
            if !already_present {
                ws_state
                    .links
                    .list_mut(target)
                    .push(links::LinkEntry { path, kind });
                match links::save(repo_path, &ws_state.links) {
                    Ok(()) => ws_state.error = None,
                    Err(e) => {
                        ws_state.links.list_mut(target).pop();
                        ws_state.error = Some(keyed_args(
                            "error.save_failed",
                            "Save failed: {error}",
                            &[("error", &e.to_string())],
                        ));
                    }
                }
            }
        }
        Message::LinkRemove { target, index } => {
            let list = ws_state.links.list_mut(target);
            if index >= list.len() {
                return;
            }
            let removed = list.remove(index);
            // 记进 dismissed:否则文件还在磁盘上的话,"修复项目"的
            // `merge_rediscovered` 下次会把这条自动发现的记录重新加回来,
            // 删除操作就形同虚设(见 `links::LinksState.dismissed` 文档)。
            ws_state.links.dismissed.push(removed.path.clone());
            if let Err(e) = links::save(repo_path, &ws_state.links) {
                ws_state.links.dismissed.pop();
                ws_state.links.list_mut(target).insert(index, removed);
                ws_state.error = Some(keyed_args(
                    "error.save_failed",
                    "Save failed: {error}",
                    &[("error", &e.to_string())],
                ));
            } else {
                ws_state.error = None;
            }
        }
        Message::LinkDirToggle { path, .. } => {
            ws_state.selected_link = Some(path.clone());
            if ws_state.expanded_link_dirs.remove(&path).is_none() {
                let rows = links::read_dir_row(&path);
                ws_state.expanded_link_dirs.insert(path, rows);
            }
        }
        Message::LinkSelect { path, .. } => {
            ws_state.selected_link = Some(path);
        }
        Message::OpenLink(_) => {
            unreachable!("由内核拦截处理,见 files::Message::OpenFile 文档")
        }
        Message::Pick(_) => {
            unreachable!("由内核拦截处理,见 files::Message::OpenFile 文档")
        }
        Message::LinkContextMenu { .. } => {
            unreachable!("由内核拦截处理,见 files::Message::ContextMenuOpen 文档")
        }
        Message::RepairProject => {
            ws_state.scaffold_run = Some(ScaffoldRunState::pending());
            spawn_repair_run(
                repo_path.to_path_buf(),
                project_id,
                services.transcript.clone(),
                handle,
                emit,
            );
        }
        Message::DeleteProjectRequest => {
            ws_state.delete_pending = Some(delete::DeleteScope::DozerOnly);
        }
        Message::DeleteProjectScopeSelect(scope) => {
            ws_state.delete_pending = Some(scope);
        }
        Message::DeleteProjectCancel => {
            ws_state.delete_pending = None;
        }
        Message::DeleteProjectConfirm => {
            unreachable!("由内核拦截处理,见 App::project_delete_confirm 文档")
        }
        Message::ScaffoldDone => {}
        Message::ScaffoldStepStarted(_, idx) => {
            if let Some(run) = &mut ws_state.scaffold_run
                && let Some((_, state)) = run.steps.get_mut(idx)
            {
                *state = ScaffoldStepState::Running;
            }
        }
        Message::ScaffoldStepFinished(_, idx, result) => {
            if let Some(run) = &mut ws_state.scaffold_run
                && let Some((_, state)) = run.steps.get_mut(idx)
            {
                *state = ScaffoldStepState::Done(result);
            }
        }
        Message::TranscriptBackfillStarted(_) => {
            if let Some(run) = &mut ws_state.scaffold_run {
                let last = run.steps.len() - 1;
                if let Some((_, state)) = run.steps.get_mut(last) {
                    *state = ScaffoldStepState::Running;
                }
            }
        }
        Message::TranscriptBackfillFinished(_, result) => {
            if let Some(run) = &mut ws_state.scaffold_run {
                let last = run.steps.len() - 1;
                if let Some((_, state)) = run.steps.get_mut(last) {
                    *state = ScaffoldStepState::Done(result);
                }
            }
        }
        Message::SummaryBackfillFailed(_, error) => {
            if let Some(run) = &mut ws_state.scaffold_run {
                run.backfill = BackfillStepState::Failed(error);
            }
        }
        Message::SummaryBackfillProgress(_, completed, total, succeeded, failed, skipped) => {
            if let Some(run) = &mut ws_state.scaffold_run {
                let progress = BackfillProgress {
                    completed,
                    total,
                    succeeded,
                    failed,
                    skipped,
                };
                run.backfill = if completed >= total {
                    BackfillStepState::Done(progress)
                } else {
                    BackfillStepState::Running(progress)
                };
            }
        }
        Message::ScaffoldPopupClose => {
            if matches!(&ws_state.scaffold_run, Some(run) if run.all_done()) {
                ws_state.scaffold_run = None;
            }
        }
        Message::MemoriesLoaded(memories) => ws_state.memories = memories,
        Message::MemoryCreateStart => {
            ws_state.memory_create_draft = Some(MemoryDraft::default());
        }
        Message::MemoryCreateCancel => {
            ws_state.memory_create_draft = None;
        }
        Message::MemoryCreateTitleInput(v) => {
            if let Some(d) = &mut ws_state.memory_create_draft {
                d.title = v;
            }
        }
        Message::MemoryCreateKindInput(v) => {
            if let Some(d) = &mut ws_state.memory_create_draft {
                d.kind = v;
            }
        }
        Message::MemoryCreateDescriptionInput(v) => {
            if let Some(d) = &mut ws_state.memory_create_draft {
                d.description = v;
            }
        }
        Message::MemoryCreateBodyInput(v) => {
            if let Some(d) = &mut ws_state.memory_create_draft {
                d.body = v;
            }
        }
        Message::MemoryCreateSubmit => {
            ws_state.requests.invalidate("memories");
            let emit = ws_state
                .requests
                .scope(Some(project_id))
                .wrap(emit, Message::AsyncResult);
            let Some(draft) = ws_state.memory_create_draft.take() else {
                return;
            };
            let memory = services.memory.clone();
            handle.spawn(async move {
                let res = memory
                    .write_memory(
                        project_id,
                        draft.title,
                        draft.kind,
                        draft.description,
                        draft.body,
                        "user".to_string(),
                    )
                    .await
                    .map(|_| ())
                    .map_err(|e| e.message);
                emit(Message::MemoryMutated(res));
            });
        }
        Message::MemoryMutated(res) => {
            if let Err(e) = res {
                ws_state.error = Some(keyed_args(
                    "error.memory_save_failed",
                    "Failed to save memory: {error}",
                    &[("error", &e)],
                ));
            } else {
                ws_state.error = None;
            }
            request_memories_refresh(
                &ws_state.requests,
                project_id,
                &services.memory,
                handle,
                emit,
            );
        }
        Message::MemoriesLoadFailed(kind, message) => {
            if kind == ServiceErrorKind::Unavailable {
                dozer_core::log_debug!(LOG, "记忆列表读取失败(dozerd 不可用): {message}");
            } else {
                ws_state.outbox.push_keyed_cooldown(
                    LOG,
                    crate::extensions::toast::Level::Error,
                    keyed_args(
                        "error.memory_load_failed",
                        "Failed to load memories: {error}",
                        &[("error", &message)],
                    ),
                    "project-memories-load",
                    std::time::Instant::now(),
                );
            }
        }
        Message::MemoryDetailOpen(id) => {
            let emit = ws_state
                .requests
                .begin(Some(project_id), "detail")
                .wrap(emit, Message::AsyncResult);
            ws_state.memory_detail = None;
            ws_state.memory_edit_draft = None;
            let memory = services.memory.clone();
            handle.spawn(async move {
                // 先 await 再调用 `emit`:`emit` 是 `Fn + Send`(非 `Sync`),
                // 写成 `emit(f(x.await))` 会让 `&emit` 横跨 await,future 就不是 `Send` 了。
                let res = memory.get_memory(project_id, id).await;
                emit(memory_detail_message(res));
            });
        }
        Message::MemoryDetailLoaded(detail) => {
            ws_state.memory_detail = Some(detail);
        }
        Message::MemoryDetailLoadFailed(e) => {
            ws_state.error = Some(keyed_args(
                "error.memory_read_failed",
                "Failed to read memory: {error}",
                &[("error", &e)],
            ));
        }
        Message::MemoryDetailClose => {
            ws_state.requests.invalidate("detail");
            ws_state.memory_detail = None;
            ws_state.memory_edit_draft = None;
            ws_state.memory_delete_pending = None;
        }
        Message::MemoryEditStart => {
            if let Some(detail) = &ws_state.memory_detail {
                ws_state.memory_edit_draft = Some(MemoryDraft {
                    title: detail.title.clone(),
                    kind: detail.kind.clone(),
                    description: detail.description.clone(),
                    body: detail.body.clone(),
                });
            }
        }
        Message::MemoryEditCancel => {
            ws_state.memory_edit_draft = None;
        }
        Message::MemoryEditTitleInput(v) => {
            if let Some(d) = &mut ws_state.memory_edit_draft {
                d.title = v;
            }
        }
        Message::MemoryEditKindInput(v) => {
            if let Some(d) = &mut ws_state.memory_edit_draft {
                d.kind = v;
            }
        }
        Message::MemoryEditDescriptionInput(v) => {
            if let Some(d) = &mut ws_state.memory_edit_draft {
                d.description = v;
            }
        }
        Message::MemoryEditBodyInput(v) => {
            if let Some(d) = &mut ws_state.memory_edit_draft {
                d.body = v;
            }
        }
        Message::MemoryEditSubmit => {
            ws_state.requests.invalidate("memories");
            let emit = ws_state
                .requests
                .scope(Some(project_id))
                .wrap(emit, Message::AsyncResult);
            let Some(draft) = ws_state.memory_edit_draft.take() else {
                return;
            };
            let detail_emit = ws_state
                .requests
                .begin(Some(project_id), "detail")
                .wrap(emit.clone(), Message::AsyncResult);
            let memory = services.memory.clone();
            handle.spawn(async move {
                match memory
                    .write_memory(
                        project_id,
                        draft.title,
                        draft.kind,
                        draft.description,
                        draft.body,
                        "user".to_string(),
                    )
                    .await
                {
                    Ok(detail) => {
                        // 编辑成功后重拉详情(带历史),覆盖 `MemoryMutated`
                        // 只刷新列表的局限——否则详情面板会停在编辑前的内容,
                        // 必须退回列表再点进去才看得到新值。
                        let fresh = memory.get_memory(project_id, detail.id).await;
                        detail_emit(memory_detail_message(fresh));
                        emit(Message::MemoryMutated(Ok(())));
                    }
                    Err(e) => emit(Message::MemoryMutated(Err(e.message))),
                }
            });
        }
        Message::MemoryDeleteRequest(id) => {
            ws_state.memory_delete_pending = Some(id);
        }
        Message::MemoryDeleteCancel => {
            ws_state.memory_delete_pending = None;
        }
        Message::MemoryDeleteConfirm => {
            ws_state.requests.invalidate("memories");
            let emit = ws_state
                .requests
                .scope(Some(project_id))
                .wrap(emit, Message::AsyncResult);
            ws_state.requests.invalidate("detail");
            let Some(id) = ws_state.memory_delete_pending.take() else {
                return;
            };
            ws_state.memory_detail = None;
            let memory = services.memory.clone();
            handle.spawn(async move {
                let res = memory
                    .delete_memory(project_id, id, "user".to_string())
                    .await
                    .map_err(|e| e.message);
                emit(Message::MemoryMutated(res));
            });
        }
    }
}

/// 静默 scaffold 跑(项目 tab 打开时触发,`app.rs::project_tab_opened`
/// 唯一调用点,不弹窗)。跑完四个同步步骤 + 转录历史补录,不产出任何
/// UI 可见结果——`emit(Message::ScaffoldDone)` 只是让调用方知道这批
/// spawn 任务已经跑完(目前没有消费方,`update()` 是空分支),不携带内容。
/// **不触发补总结**:补总结只在显式点击"修复项目"时跑(spec
/// 2026-08-28,避免每次静默打开项目都真实拉起 agent 进程)。
pub fn spawn_scaffold_run(
    repo_path: std::path::PathBuf,
    transcript: Arc<dyn TranscriptService>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
    skip_git_init: bool,
) {
    let cwd = repo_path.to_string_lossy().into_owned();
    handle.spawn(async move {
        let repo_path2 = repo_path.clone();
        let _ = tokio::task::spawn_blocking(move || {
            scaffold::run_sync_steps(&repo_path2, skip_git_init)
        })
        .await;
        if let Err(e) = transcript.backfill_project_transcripts(cwd.clone()).await {
            dozer_core::log_warn!(LOG, error = %e, "补录转录失败");
        }
        emit(Message::ScaffoldDone);
    });
}

/// 拉取某项目的共享记忆列表首屏数据,完成后 `emit(Message::MemoriesLoaded)`。
/// 打开/切入 Project 面板时触发一次(同 Todo 面板的 `request_todos_refresh`
/// 时机);之后的刷新走 `Message::MemoryMutated` 分支。
pub fn request_memories_refresh(
    requests: &crate::panel_host::RequestTrackers,
    project_id: i64,
    memory: &Arc<dyn MemoryService>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let emit = requests
        .begin(Some(project_id), "memories")
        .wrap(emit, Message::AsyncResult);
    let memory = memory.clone();
    handle.spawn(async move {
        match memory.list_memories(project_id).await {
            Ok(memories) => emit(Message::MemoriesLoaded(memories)),
            Err(e) => emit(Message::MemoriesLoadFailed(e.kind, e.message)),
        }
    });
}

/// 面板切入:刷新共享记忆列表(README 的展开/定位是 host 的预览动作,不在这里)。
pub fn on_activate(state: &WorkspaceState, ctx: &crate::panel_host::ActivationCtx<Message>) {
    request_memories_refresh(
        &state.requests,
        ctx.project_id,
        &ctx.io.services().memory,
        ctx.io.handle(),
        ctx.io.emitter(),
    );
}

const SUMMARY_BACKFILL_POLL_INTERVAL: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_millis(5)
} else {
    std::time::Duration::from_secs(2)
};

/// "修复项目"按钮触发的完整跑法:4 个同步步骤逐个 Started/Finished、
/// 转录历史补录 Started/Finished、再触发补总结并轮询进度,全部实时
/// `emit` 给弹窗(spec 2026-08-28)。所有消息都携带 `project_id`,靠
/// `app.rs` 里按 `project_id` 查找 workspace 的路由分支落地(不依赖
/// "当前激活哪个 tab"),避免用户在补总结进行中切换项目 tab 时消息投递到
/// 错误的 workspace。
pub fn spawn_repair_run(
    repo_path: std::path::PathBuf,
    project_id: i64,
    transcript: Arc<dyn TranscriptService>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let cwd = repo_path.to_string_lossy().into_owned();
    handle.spawn(async move {
        let steps = scaffold::scaffold_steps();
        for (idx, step) in steps.into_iter().enumerate() {
            emit(Message::ScaffoldStepStarted(project_id, idx));
            let repo_path3 = repo_path.clone();
            let result = tokio::task::spawn_blocking(move || (step.run)(&repo_path3))
                .await
                .unwrap_or_else(|e| {
                    scaffold::ScaffoldStepResult::Failed(keyed_args(
                        "scaffold.result.internal_error",
                        "Internal error: {error}",
                        &[("error", &e.to_string())],
                    ))
                });
            emit(Message::ScaffoldStepFinished(project_id, idx, result));
        }

        emit(Message::TranscriptBackfillStarted(project_id));
        let backfill_result = match transcript.backfill_project_transcripts(cwd.clone()).await {
            Ok(0) => scaffold::ScaffoldStepResult::AlreadyOk,
            Ok(n) => scaffold::ScaffoldStepResult::Created(keyed_args(
                "scaffold.result.imported",
                "Imported {n} history files",
                &[("n", &n.to_string())],
            )),
            Err(e) => scaffold::ScaffoldStepResult::Failed(e.message.into()),
        };
        emit(Message::TranscriptBackfillFinished(
            project_id,
            backfill_result,
        ));

        // 总结步骤改用 V2 批次协议:提交批次 → 轮询批次状态,展示成功/失败/
        // 跳过计数,不再用旧 0/0 进度掩盖失败(spec 2026-09-26 A7)。
        let (batch_id, _total) = match transcript.submit_summary_batch(cwd.clone(), None, None).await {
            Ok(pair) => pair,
            Err(e) => {
                dozer_core::log_warn!(LOG, error = %e, "提交总结批次失败");
                emit(Message::SummaryBackfillFailed(
                    project_id,
                    keyed_args(
                        "error.summary_failed",
                        "Failed to summarize: {error}",
                        &[("error", &e.message)],
                    ),
                ));
                return;
            }
        };
        loop {
            tokio::time::sleep(SUMMARY_BACKFILL_POLL_INTERVAL).await;
            match transcript.get_summary_batch(batch_id).await {
                Ok(Some(batch)) => {
                    let completed = batch
                        .total
                        .saturating_sub(batch.queued)
                        .saturating_sub(batch.running);
                    emit(Message::SummaryBackfillProgress(
                        project_id,
                        completed,
                        batch.total,
                        batch.succeeded,
                        batch.failed,
                        batch.skipped,
                    ));
                    if batch.queued == 0 && batch.running == 0 {
                        break;
                    }
                }
                Ok(None) => {
                    dozer_core::log_warn!(LOG, batch_id, "批次不存在,停止轮询");
                    emit(Message::SummaryBackfillFailed(
                        project_id,
                        keyed(
                            "error.summary_batch_missing",
                            "Summary batch not found, please retry",
                        ),
                    ));
                    break;
                }
                Err(e) => {
                    dozer_core::log_warn!(LOG, error = %e, "查询总结批次失败,停止轮询");
                    emit(Message::SummaryBackfillFailed(
                        project_id,
                        keyed_args(
                            "error.summary_failed",
                            "Failed to summarize: {error}",
                            &[("error", &e.message)],
                        ),
                    ));
                    break;
                }
            }
        }
    });
}

/// 项目名称编辑的共享提交逻辑:回车提交(`NameEditSubmit`)与失焦提交
/// (`App::set_project_name_focused` 的边缘触发)两条路径共用,避免两份
/// 重复的 `client.rename_project` 调用(现状历史遗留,这次一并合并)。
/// 空名字/未改动直接退出编辑态,不发请求。
pub fn submit_name_edit(
    ws_state: &mut WorkspaceState,
    project_id: i64,
    current_name: &str,
    project: Arc<dyn ProjectService>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let Some(raw) = ws_state.name_editing.take() else {
        return;
    };
    let name = raw.trim().to_string();
    if name.is_empty() || name == current_name {
        return;
    }
    handle.spawn(async move {
        let result = project
            .rename_project(project_id, name)
            .await
            .map_err(|e| Some(e.message))
            .and_then(|opt| opt.ok_or(None));
        emit(Message::NameRenamed(project_id, result));
    });
}
