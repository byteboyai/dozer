//! 项目面板的 update 消息分发 + scaffold 运行/repair/名称提交的异步 spawn。

use super::*;

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
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
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
                client.clone(),
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
                ws_state.error = Some(format!("改名失败: {e}"));
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
                    ws_state.error = Some(format!("保存失败: {e}"));
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
                        ws_state.error = Some(format!("保存失败: {e}"));
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
                ws_state.error = Some(format!("保存失败: {e}"));
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
                client.clone(),
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
            let Some(draft) = ws_state.memory_create_draft.take() else {
                return;
            };
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .write_memory(
                        project_id,
                        &draft.title,
                        &draft.kind,
                        &draft.description,
                        &draft.body,
                        "user",
                    )
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::MemoryMutated(res));
            });
        }
        Message::MemoryMutated(res) => {
            if let Err(e) = res {
                ws_state.error = Some(format!("保存记忆失败: {e}"));
            } else {
                ws_state.error = None;
            }
            let client = client.clone();
            handle.spawn(async move {
                let memories = client.list_memories(project_id).await.unwrap_or_default();
                emit(Message::MemoriesLoaded(memories));
            });
        }
        Message::MemoryDetailOpen(id) => {
            ws_state.memory_detail = None;
            ws_state.memory_edit_draft = None;
            let client = client.clone();
            handle.spawn(async move {
                if let Ok(detail) = client.get_memory(project_id, id).await {
                    emit(Message::MemoryDetailLoaded(detail));
                }
            });
        }
        Message::MemoryDetailLoaded(detail) => {
            ws_state.memory_detail = Some(detail);
        }
        Message::MemoryDetailClose => {
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
            let Some(draft) = ws_state.memory_edit_draft.take() else {
                return;
            };
            let client = client.clone();
            handle.spawn(async move {
                match client
                    .write_memory(
                        project_id,
                        &draft.title,
                        &draft.kind,
                        &draft.description,
                        &draft.body,
                        "user",
                    )
                    .await
                {
                    Ok(detail) => {
                        // 编辑成功后重拉详情(带历史),覆盖 `MemoryMutated`
                        // 只刷新列表的局限——否则详情面板会停在编辑前的内容,
                        // 必须退回列表再点进去才看得到新值。
                        if let Ok(fresh) = client.get_memory(project_id, detail.id).await {
                            emit(Message::MemoryDetailLoaded(fresh));
                        }
                        emit(Message::MemoryMutated(Ok(())));
                    }
                    Err(e) => emit(Message::MemoryMutated(Err(e.to_string()))),
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
            let Some(id) = ws_state.memory_delete_pending.take() else {
                return;
            };
            ws_state.memory_detail = None;
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .delete_memory(project_id, id, "user")
                    .await
                    .map_err(|e| e.to_string());
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
    client: dozer_client::Client,
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
        let _ = client.backfill_project_transcripts(&cwd).await;
        emit(Message::ScaffoldDone);
    });
}

/// 拉取某项目的共享记忆列表首屏数据,完成后 `emit(Message::MemoriesLoaded)`。
/// 打开/切入 Project 面板时触发一次(同 Todo 面板的 `request_todos_refresh`
/// 时机);之后的刷新走 `Message::MemoryMutated` 分支。
pub fn request_memories_refresh(
    project_id: i64,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let memories = client.list_memories(project_id).await.unwrap_or_default();
        emit(Message::MemoriesLoaded(memories));
    });
}

const SUMMARY_BACKFILL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// "修复项目"按钮触发的完整跑法:4 个同步步骤逐个 Started/Finished、
/// 转录历史补录 Started/Finished、再触发补总结并轮询进度,全部实时
/// `emit` 给弹窗(spec 2026-08-28)。所有消息都携带 `project_id`,靠
/// `app.rs` 里按 `project_id` 查找 workspace 的路由分支落地(不依赖
/// "当前激活哪个 tab"),避免用户在补总结进行中切换项目 tab 时消息投递到
/// 错误的 workspace。
pub fn spawn_repair_run(
    repo_path: std::path::PathBuf,
    project_id: i64,
    client: dozer_client::Client,
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
                .unwrap_or_else(|e| scaffold::ScaffoldStepResult::Failed(format!("内部错误: {e}")));
            emit(Message::ScaffoldStepFinished(project_id, idx, result));
        }

        emit(Message::TranscriptBackfillStarted(project_id));
        let backfill_result = match client.backfill_project_transcripts(&cwd).await {
            Ok(0) => scaffold::ScaffoldStepResult::AlreadyOk,
            Ok(n) => scaffold::ScaffoldStepResult::Created(format!("导入 {n} 个历史文件")),
            Err(e) => scaffold::ScaffoldStepResult::Failed(e.to_string()),
        };
        emit(Message::TranscriptBackfillFinished(
            project_id,
            backfill_result,
        ));

        // 总结步骤改用 V2 批次协议:提交批次 → 轮询批次状态,展示成功/失败/
        // 跳过计数,不再用旧 0/0 进度掩盖失败(spec 2026-09-26 A7)。
        let (batch_id, _total) = match client.submit_summary_batch(&cwd, None, None).await {
            Ok(pair) => pair,
            Err(e) => {
                dozer_core::log_warn!(LOG, error = %e, "提交总结批次失败");
                emit(Message::SummaryBackfillFailed(project_id, e.to_string()));
                return;
            }
        };
        loop {
            tokio::time::sleep(SUMMARY_BACKFILL_POLL_INTERVAL).await;
            match client.get_summary_batch(batch_id).await {
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
                        "总结批次不存在，请重试".into(),
                    ));
                    break;
                }
                Err(e) => {
                    dozer_core::log_warn!(LOG, error = %e, "查询总结批次失败,停止轮询");
                    emit(Message::SummaryBackfillFailed(project_id, e.to_string()));
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
    client: dozer_client::Client,
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
        let result = client
            .rename_project(project_id, &name)
            .await
            .map_err(|e| e.to_string())
            .and_then(|opt| opt.ok_or_else(|| "项目不存在".to_string()));
        emit(Message::NameRenamed(project_id, result));
    });
}
