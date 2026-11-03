//! 同步领域请求。只在共享有界 IO 池中运行，不占 Tokio executor。
use crate::preview_context::PreviewContextStore;
use crate::server::{
    Stores, conversation_id_for_session, enqueue_session_summary, locate_error_to_empty_with_log,
    schedule_post_edit_reveal,
};
use dozer_core::protocol::{Reply, Request};
use std::sync::Arc;
dozer_core::scope!(LOG, module, "server");
pub(crate) fn supports(request: &Request) -> bool {
    matches!(
        request,
        Request::OpenProject { .. }
            | Request::ListProjects { .. }
            | Request::RenameProject { .. }
            | Request::AddBookmark { .. }
            | Request::RemoveBookmark { .. }
            | Request::ListBookmarks { .. }
            | Request::GetLocale { .. }
            | Request::SetLocale { .. }
            | Request::SaveCodeHealthReport { .. }
            | Request::GetCodeHealthReport { .. }
            | Request::ListCodeHealthReports { .. }
            | Request::ListTodos { .. }
            | Request::AddTodo { .. }
            | Request::ToggleTodo { .. }
            | Request::EditTodoText { .. }
            | Request::ReorderTodo { .. }
            | Request::SetTodoPlanDate { .. }
            | Request::AssignTodoAgent { .. }
            | Request::SetCategoryAutoPoll { .. }
            | Request::GetTodoExecution { .. }
            | Request::GetTodoDetail { .. }
            | Request::SetTodoStatus { .. }
            | Request::ListMemories { .. }
            | Request::WriteMemory { .. }
            | Request::GetMemory { .. }
            | Request::DeleteMemory { .. }
            | Request::CreateGroup { .. }
            | Request::ListGroups { .. }
            | Request::DeleteGroup { .. }
            | Request::AddGroupMember { .. }
            | Request::UpdateGroupMember { .. }
            | Request::RemoveGroupMember { .. }
            | Request::PostGroupMessage { .. }
            | Request::ListGroupMessages { .. }
            | Request::CancelGroup { .. }
            | Request::RetryGroupMessage { .. }
            | Request::PushGroupMessageToTodo { .. }
            | Request::ListCategories { .. }
            | Request::AddCategory { .. }
            | Request::RenameCategory { .. }
            | Request::DeleteCategory { .. }
            | Request::ReparentCategory { .. }
            | Request::MoveCategorySibling { .. }
            | Request::SetTodoCategory { .. }
            | Request::UpdatePreviewContext { .. }
            | Request::GetPreviewContext { .. }
            | Request::TakePendingPreviewCommands { .. }
            | Request::ReportPreviewCommandOutcome { .. }
            | Request::ListConversations { .. }
            | Request::GetConversationTurns { .. }
            | Request::BackfillProjectTranscripts { .. }
            | Request::RemoveProject { .. }
            | Request::DeleteProjectTranscripts { .. }
            | Request::ListConversationsWithSummaries { .. }
            | Request::GetUsageSummary { .. }
            | Request::RecordSessionSummary { .. }
            | Request::GetSessionSummary { .. }
            | Request::CloseWithSummary { .. }
            | Request::GetSessionSummaryBackfillStatus { .. }
            | Request::SubmitSummaryJob { .. }
            | Request::SubmitSummaryBatch { .. }
            | Request::GetSummaryJob { .. }
            | Request::GetSummaryBatch { .. }
            | Request::RetrySummaryJob { .. }
            | Request::CancelSummaryBatch { .. }
            | Request::GetSummaryProvider { .. }
            | Request::GetSummaryResult { .. }
            | Request::LocateInFile { .. }
            | Request::LocateInFileV2 { .. }
            | Request::ApplyPreciseEdit { .. }
            | Request::AddContextItem { .. }
            | Request::RemoveContextItem { .. }
            | Request::ListContextItems { .. }
            | Request::ListFileEditHistory { .. }
    )
}
pub(crate) fn handle(
    request: Request,
    stores: Stores,
    preview_contexts: Arc<PreviewContextStore>,
    preview_commands: Arc<crate::preview_commands::PreviewCommandBus>,
) -> Reply {
    let Stores {
        registry,
        projects,
        bookmarks,
        code_health,
        transcripts,
        session_summaries,
        summary_jobs,
        backfill_registry,
        todos,
        executions,
        categories,
        memories,
        file_edit_history,
        groups,
        apps: _,
        locale,
    } = stores;
    let summary_service = Arc::new(crate::summary_service::SummaryService {
        jobs: summary_jobs.clone(),
        transcripts: transcripts.clone(),
        session_summaries: session_summaries.clone(),
        scratch_root: dozer_core::paths::state_dir().join("summary-scratch"),
    });
    match request {
        Request::OpenProject { path } => match projects.open(&path) {
            Ok(p) => Reply::Project { project: Some(p) },
            Err(e) => Reply::Error {
                message: format!("打开项目失败: {e}"),
            },
        },
        Request::ListProjects => match projects.list() {
            Ok(projects) => Reply::Projects { projects },
            Err(e) => Reply::Error {
                message: format!("列项目失败: {e}"),
            },
        },
        Request::RenameProject { id, name } => {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                Reply::Error {
                    message: "名称不能为空".into(),
                }
            } else {
                match projects.rename(id, trimmed) {
                    Ok(p) => Reply::Project { project: Some(p) },
                    Err(e) => Reply::Error {
                        message: format!("改名失败: {e}"),
                    },
                }
            }
        }
        Request::AddBookmark {
            scope,
            project_id,
            url,
            title,
        } => match bookmarks.add(scope, project_id, &url, &title) {
            Ok(_) => Reply::Ok,
            Err(e) => Reply::Error {
                message: format!("加入收藏失败: {e}"),
            },
        },
        Request::RemoveBookmark { id } => match bookmarks.remove(id) {
            Ok(()) => Reply::Ok,
            Err(e) => Reply::Error {
                message: format!("移除收藏失败: {e}"),
            },
        },
        Request::ListBookmarks { project_id } => match bookmarks.list(project_id) {
            Ok(bookmarks) => Reply::Bookmarks { bookmarks },
            Err(e) => Reply::Error {
                message: format!("列收藏失败: {e}"),
            },
        },
        Request::GetLocale => Reply::Locale {
            snapshot: locale.get(),
        },
        Request::SetLocale {
            preference,
            expected_revision,
            system_candidates,
        } => match locale.set(preference, expected_revision, system_candidates) {
            Ok(outcome) => Reply::LocaleWrite { outcome },
            Err(e) => Reply::Error {
                message: format!("写入语言设置失败: {e}"),
            },
        },
        Request::SaveCodeHealthReport {
            project_id,
            report_json,
            total_loc,
            total_functions,
            critical_functions,
            overall_tier,
            schema_version,
            git_head,
            git_branch,
            git_dirty,
        } => {
            let info = dozer_core::protocol::CodeHealthReportInfo {
                total_loc,
                total_functions,
                critical_functions,
                overall_tier,
                report_json,
                scanned_at_ms: 0, // store 内部会用自己的 now_ms() 覆盖
                schema_version,
                git_head,
                git_branch,
                git_dirty,
            };
            match code_health.save(project_id, &info) {
                Ok(()) => Reply::Ok,
                Err(e) => Reply::Error {
                    message: format!("保存代码健康度报告失败: {e}"),
                },
            }
        }
        Request::GetCodeHealthReport { project_id } => match code_health.get(project_id) {
            Ok(report) => Reply::CodeHealthReport { report },
            Err(e) => Reply::Error {
                message: format!("查询代码健康度报告失败: {e}"),
            },
        },
        Request::ListCodeHealthReports { project_id, limit } => {
            match code_health.list(project_id, limit) {
                Ok(reports) => Reply::CodeHealthReports { reports },
                Err(e) => Reply::Error {
                    message: format!("查询代码健康度历史失败: {e}"),
                },
            }
        }
        Request::ListTodos { project_id } => match todos.list(project_id) {
            Ok(todos) => Reply::Todos { todos },
            Err(e) => Reply::Error {
                message: format!("列任务失败: {e}"),
            },
        },
        Request::AddTodo { project_id, text } => match todos.add(project_id, &text) {
            Ok(todo) => Reply::Todo { todo },
            Err(e) => Reply::Error {
                message: format!("新增任务失败: {e}"),
            },
        },
        Request::ToggleTodo { id, done } => match todos.toggle(id, done) {
            Ok(todo) => Reply::Todo { todo },
            Err(e) => Reply::Error {
                message: format!("切换完成态失败: {e}"),
            },
        },
        Request::EditTodoText { id, text } => match todos.edit_text(id, &text) {
            Ok(todo) => Reply::Todo { todo },
            Err(e) => Reply::Error {
                message: format!("改任务文字失败: {e}"),
            },
        },
        Request::ReorderTodo { id, after_id } => match todos.reorder(id, after_id) {
            Ok(todo) => Reply::Todo { todo },
            Err(e) => Reply::Error {
                message: format!("排序失败: {e}"),
            },
        },
        Request::SetTodoPlanDate { id, plan_date } => {
            match todos.set_plan_date(id, plan_date.as_deref()) {
                Ok(todo) => Reply::Todo { todo },
                Err(e) => Reply::Error {
                    message: format!("设置计划日期失败: {e}"),
                },
            }
        }
        Request::AssignTodoAgent { id, agent } => match todos.assign_agent(id, agent) {
            Ok(todo) => Reply::Todo { todo },
            Err(e) => Reply::Error {
                message: format!("指派任务失败: {e}"),
            },
        },
        Request::SetCategoryAutoPoll { id, enabled } => {
            match categories.set_auto_poll(id, enabled) {
                Ok(category) => Reply::Category { category },
                Err(e) => Reply::Error {
                    message: format!("设置自动轮询失败: {e}"),
                },
            }
        }
        Request::GetTodoExecution { id } => match executions.latest(id) {
            Ok(execution) => Reply::TodoExecution { execution },
            Err(e) => Reply::Error {
                message: format!("查询任务执行状态失败: {e}"),
            },
        },
        Request::GetTodoDetail { id } => match todos.get(id) {
            Ok(info) => {
                let turns = info
                    .dispatch_session_id
                    .as_deref()
                    .and_then(|sid| transcripts.get_conversation_turns(sid, -1, u32::MAX).ok())
                    .unwrap_or_default();
                Reply::TodoDetail { info, turns }
            }
            Err(e) => Reply::Error {
                message: format!("获取任务详情失败: {e}"),
            },
        },
        Request::SetTodoStatus { id, status } => match todos.set_status(id, status) {
            Ok(todo) => Reply::Todo { todo },
            Err(e) => Reply::Error {
                message: format!("设置任务状态失败: {e}"),
            },
        },
        Request::ListMemories { project_id } => match memories.list(project_id) {
            Ok(memories) => Reply::Memories { memories },
            Err(e) => Reply::Error {
                message: format!("列共享记忆失败: {e}"),
            },
        },
        Request::WriteMemory {
            project_id,
            title,
            kind,
            description,
            body,
            actor,
        } => match memories.write(project_id, &title, &kind, &description, &body, &actor) {
            Ok(detail) => Reply::MemoryDetail { detail },
            Err(e) => Reply::Error {
                message: format!("写共享记忆失败: {e}"),
            },
        },
        Request::GetMemory { project_id, id } => match memories.get(project_id, id) {
            Ok(detail) => Reply::MemoryDetail { detail },
            Err(e) => Reply::Error {
                message: format!("查共享记忆失败: {e}"),
            },
        },
        Request::DeleteMemory {
            project_id,
            id,
            actor,
        } => match memories.delete(project_id, id, &actor) {
            Ok(()) => Reply::Ok,
            Err(e) => Reply::Error {
                message: format!("删共享记忆失败: {e}"),
            },
        },
        Request::CreateGroup { project_id, topic } => {
            match groups.store().create_group(project_id, &topic) {
                Ok(group) => Reply::Group { group },
                Err(e) => Reply::Error {
                    message: format!("新建群聊失败: {e}"),
                },
            }
        }
        Request::ListGroups { project_id } => match groups.store().list_groups(project_id) {
            Ok(groups) => Reply::Groups { groups },
            Err(e) => Reply::Error {
                message: format!("列群聊失败: {e}"),
            },
        },
        Request::DeleteGroup { group_id } => match groups.delete_group(group_id) {
            Ok(()) => Reply::Ok,
            Err(e) => Reply::Error {
                message: format!("删除群聊失败: {e}"),
            },
        },
        Request::AddGroupMember {
            group_id,
            agent,
            handle,
            role_prompt,
        } => {
            match groups
                .store()
                .add_member(group_id, agent, &handle, &role_prompt)
            {
                Ok(group) => Reply::Group { group },
                Err(e) => Reply::Error {
                    message: format!("添加成员失败: {e}"),
                },
            }
        }
        Request::UpdateGroupMember {
            member_id,
            handle,
            role_prompt,
        } => {
            match groups
                .store()
                .update_member(member_id, &handle, &role_prompt)
            {
                Ok(group) => Reply::Group { group },
                Err(e) => Reply::Error {
                    message: format!("修改成员失败: {e}"),
                },
            }
        }
        Request::RemoveGroupMember { member_id } => match groups.store().remove_member(member_id) {
            Ok(group) => Reply::Group { group },
            Err(e) => Reply::Error {
                message: format!("移除成员失败: {e}"),
            },
        },
        Request::PostGroupMessage { group_id, text } => match groups.post(group_id, &text) {
            Ok(o) => Reply::GroupPosted {
                human: o.human,
                placeholders: o.placeholders,
                unknown_handles: o.unknown_handles,
            },
            Err(e) => Reply::Error {
                message: format!("发送失败: {e}"),
            },
        },
        Request::ListGroupMessages {
            group_id,
            after_rev,
            limit,
        } => {
            match groups
                .store()
                .list_messages_after_rev(group_id, after_rev, limit.clamp(1, 500))
            {
                Ok((messages, latest_rev)) => Reply::GroupMessages {
                    messages,
                    latest_rev,
                },
                Err(e) => Reply::Error {
                    message: format!("取群消息失败: {e}"),
                },
            }
        }
        Request::CancelGroup { group_id, scope } => {
            groups.cancel(group_id, scope);
            Reply::Ok
        }
        Request::RetryGroupMessage { message_id } => match groups.retry(message_id) {
            Ok(message) => Reply::GroupMessage { message },
            Err(e) => Reply::Error {
                message: format!("重试失败: {e}"),
            },
        },
        Request::PushGroupMessageToTodo { message_id, text } => {
            let text = text.trim().to_string();
            if text.is_empty() {
                Reply::Error {
                    message: "待办内容不能为空".into(),
                }
            } else {
                match groups.store().get_message(message_id) {
                    Err(e) => Reply::Error {
                        message: format!("推送待办失败: {e}"),
                    },
                    Ok(msg) if msg.todo_id.is_some() => Reply::Error {
                        message: "该消息已转为待办".into(),
                    },
                    Ok(msg) => match groups.store().get_group(msg.group_id) {
                        Err(e) => Reply::Error {
                            message: format!("推送待办失败: {e}"),
                        },
                        Ok(group) => match todos.add(group.project_id, &text) {
                            Err(e) => Reply::Error {
                                message: format!("新增任务失败: {e}"),
                            },
                            Ok(todo) => match groups.store().set_todo_link(message_id, todo.id) {
                                Ok(()) => Reply::Todo { todo },
                                Err(e) => {
                                    dozer_core::log_warn!(LOG, message_id, todo_id = todo.id, error = %e, "待办已创建但记录关联失败");
                                    Reply::Error {
                                        message: format!("待办已创建,但记录来源失败: {e}"),
                                    }
                                }
                            },
                        },
                    },
                }
            }
        }
        Request::ListCategories { project_id } => match categories.list(project_id) {
            Ok(categories) => Reply::Categories { categories },
            Err(e) => Reply::Error {
                message: format!("列分类失败: {e}"),
            },
        },
        Request::AddCategory {
            project_id,
            parent_id,
            name,
        } => match categories.add(project_id, parent_id, &name) {
            Ok(category) => Reply::Category { category },
            Err(e) => Reply::Error {
                message: format!("新增分类失败: {e}"),
            },
        },
        Request::RenameCategory { id, name } => match categories.rename(id, &name) {
            Ok(category) => Reply::Category { category },
            Err(e) => Reply::Error {
                message: format!("重命名分类失败: {e}"),
            },
        },
        Request::DeleteCategory { id } => match categories.delete(id) {
            Ok(()) => Reply::Ok,
            Err(e) => Reply::Error {
                message: format!("删除分类失败: {e}"),
            },
        },
        Request::ReparentCategory { id, new_parent_id } => {
            match categories.reparent(id, new_parent_id) {
                Ok(category) => Reply::Category { category },
                Err(e) => Reply::Error {
                    message: format!("移动分类失败: {e}"),
                },
            }
        }
        Request::MoveCategorySibling { id, direction } => {
            match categories.move_sibling(id, direction) {
                Ok(category) => Reply::Category { category },
                Err(e) => Reply::Error {
                    message: format!("调整分类顺序失败: {e}"),
                },
            }
        }
        Request::SetTodoCategory { id, category_id } => {
            // 挂真实分类前先校验它存在且属于同一 project——
            // 防止 GUI 传错 id 把任务挂到别的项目的分类下面。
            // `TodoStore` 自己不知道 `CategoryStore` 的存在,
            // 这层校验只能在这里(两个 store 的交汇点)做。
            let validation = match category_id {
                None => Ok(()),
                Some(cat_id) => todos.get(id).and_then(|todo| {
                    let same_project = categories
                        .list(todo.project_id)?
                        .iter()
                        .any(|c| c.id == cat_id);
                    if same_project {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("分类 id={cat_id} 不存在或不属于该项目"))
                    }
                }),
            };
            match validation.and_then(|()| todos.set_category(id, category_id)) {
                Ok(todo) => Reply::Todo { todo },
                Err(e) => Reply::Error {
                    message: format!("设置任务分类失败: {e}"),
                },
            }
        }
        Request::UpdatePreviewContext {
            project_id,
            context,
        } => {
            preview_contexts.update(project_id, context);
            Reply::Ok
        }
        Request::GetPreviewContext { project_id } => Reply::PreviewContext {
            context: preview_contexts.get(project_id),
        },
        // T13:下行预览命令。入队并等待 app 回传终态;app 不在线或
        // 超过超时时间则回 Timeout(不无限挂起)。
        Request::TakePendingPreviewCommands { project_id } => Reply::PendingPreviewCommands {
            commands: preview_commands.take(project_id),
        },
        Request::ReportPreviewCommandOutcome { outcome } => {
            preview_commands.report(outcome);
            Reply::Ok
        }
        Request::ListConversations {
            cwd,
            agent,
            limit,
            offset,
        } => match transcripts.list_conversations(&cwd, agent, limit, offset) {
            Ok(conversations) => Reply::Conversations { conversations },
            Err(e) => Reply::Error {
                message: format!("列对话失败: {e}"),
            },
        },
        Request::GetConversationTurns {
            conversation_id,
            after_turn_index,
            limit,
        } => match transcripts.get_conversation_turns(&conversation_id, after_turn_index, limit) {
            Ok(turns) => Reply::ConversationTurns {
                conversation_id,
                turns,
            },
            Err(e) => Reply::Error {
                message: format!("查询回合失败: {e}"),
            },
        },
        Request::BackfillProjectTranscripts { cwd } => {
            let report = transcripts.backfill_project_report(&cwd);
            Reply::BackfillDone {
                imported_files: report.imported_files,
                failed_total: report.failed_total,
                failures: report.failures,
            }
        }
        Request::RemoveProject { id } => match projects.remove(id) {
            Ok(()) => Reply::Ok,
            Err(e) => Reply::Error {
                message: format!("删除项目失败: {e}"),
            },
        },
        Request::DeleteProjectTranscripts { cwd } => {
            match transcripts.delete_project_transcripts(&cwd) {
                Ok(conversations) => Reply::DeletedTranscripts { conversations },
                Err(e) => Reply::Error {
                    message: format!("删除 agent 历史失败: {e}"),
                },
            }
        }
        Request::ListConversationsWithSummaries {
            cwd,
            agent,
            limit,
            offset,
        } => match transcripts.list_conversations(&cwd, agent, limit, offset) {
            Ok(conversations) => {
                let ids: Vec<String> = conversations
                    .iter()
                    .map(|c| c.conversation_id.clone())
                    .collect();
                let task_ids: std::collections::HashMap<String, i64> = projects
                    .list()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|p| p.path == cwd)
                    .flat_map(|p| todos.list(p.id).unwrap_or_default())
                    .filter_map(|t| t.dispatch_session_id.map(|cid| (cid, t.id)))
                    .collect();
                let mut summaries = session_summaries.get_many(&ids).unwrap_or_default();
                for (cid, result) in summary_jobs.get_results_many(&ids).unwrap_or_default() {
                    let old = summaries.get(&cid);
                    summaries.insert(
                        cid.clone(),
                        dozer_core::protocol::SessionSummaryPayload {
                            session_id: old
                                .map(|s| s.session_id.clone())
                                .unwrap_or_else(|| format!("summary:{}", result.source_job_id)),
                            agent_kind: result.provider,
                            conversation_id: Some(cid.clone()),
                            title: result.title,
                            summary: result.summary,
                            status: dozer_core::protocol::SummaryStatus::AiGenerated,
                            created_ts_ms: result.generated_at,
                            task_id: old
                                .and_then(|s| s.task_id)
                                .or_else(|| task_ids.get(&cid).copied()),
                        },
                    );
                }
                let rows = conversations
                    .into_iter()
                    .map(|c| {
                        let s = summaries.get(&c.conversation_id).cloned();
                        (c, s)
                    })
                    .collect();
                Reply::ConversationsWithSummaries { rows }
            }
            Err(e) => Reply::Error {
                message: format!("列对话失败: {e}"),
            },
        },
        Request::GetUsageSummary { cwd, since_ts } => {
            match transcripts.get_usage_summary(&cwd, since_ts) {
                Ok(rows) => Reply::UsageSummary { rows },
                Err(e) => Reply::Error {
                    message: format!("查询用量失败: {e}"),
                },
            }
        }
        Request::RecordSessionSummary {
            session_id,
            title,
            summary,
        } => match registry.get(&session_id) {
            None => Reply::Error {
                message: format!("会话不存在: {session_id}"),
            },
            Some(s) => {
                let payload = dozer_core::protocol::SessionSummaryPayload {
                    session_id: session_id.clone(),
                    agent_kind: s.info().agent,
                    conversation_id: conversation_id_for_session(&s),
                    title,
                    summary,
                    status: dozer_core::protocol::SummaryStatus::AiGenerated,
                    created_ts_ms: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0),
                    task_id: None,
                };
                match session_summaries.record(&payload) {
                    Ok(()) => Reply::Ok,
                    Err(e) => Reply::Error {
                        message: format!("会话总结落库失败: {e}"),
                    },
                }
            }
        },
        Request::GetSessionSummary { session_id } => match session_summaries.get(&session_id) {
            Ok(summary) => Reply::SessionSummary { summary },
            Err(e) => Reply::Error {
                message: format!("查询会话总结失败: {e}"),
            },
        },

        Request::CloseWithSummary { session_id } => match registry.get(&session_id) {
            None => Reply::Error {
                message: format!("会话不存在: {session_id}"),
            },
            Some(s) => {
                if let Err(e) = enqueue_session_summary(
                    &s,
                    &summary_service,
                    dozer_core::protocol::SummaryTrigger::Close,
                ) {
                    dozer_core::log_warn!(LOG, error=%e, %session_id, "提交关闭总结任务失败");
                }
                if let Err(e) = registry.kill(&session_id) {
                    dozer_core::log_warn!(LOG, error = %e, %session_id, "关闭会话失败(可能已死亡)");
                }
                Reply::Ok
            }
        },
        Request::GetSessionSummaryBackfillStatus { cwd } => {
            let progress = backfill_registry.get(&cwd).unwrap_or_default();
            Reply::BackfillStatus {
                total: progress.total,
                completed: progress.completed,
            }
        }
        Request::SubmitSummaryJob {
            conversation_id,
            source_session_id,
            trigger,
            provider,
            model,
            force,
        } => match crate::summary_config::resolve_provider(provider, model.clone()) {
            crate::summary_config::SummaryProviderResolution::Required => Reply::Error {
                message: "未配置 summary provider(需 summary 配置或 default_agent)".into(),
            },
            crate::summary_config::SummaryProviderResolution::Configured(cfg, _) => {
                let spec = crate::summary_service::SubmitSpec {
                    conversation_id,
                    source_session_id,
                    trigger,
                    provider: cfg.provider,
                    requested_model: cfg.model.or(model),
                    force,
                };
                match summary_service.submit_single(&spec) {
                    Ok(job_id) => Reply::SummaryJobSubmitted { job_id },
                    Err(e) => Reply::Error {
                        message: format!("提交总结任务失败: {e}"),
                    },
                }
            }
        },
        Request::SubmitSummaryBatch {
            cwd,
            provider,
            model,
        } => match summary_service.submit_batch_with_provider(&cwd, provider, model) {
            Ok((batch_id, total)) => Reply::SummaryBatchSubmitted { batch_id, total },
            Err(e) => Reply::Error {
                message: format!("提交批次失败: {e}"),
            },
        },
        Request::GetSummaryJob { job_id } => match summary_service.jobs.get_job(job_id) {
            Ok(job) => Reply::SummaryJob { job },
            Err(e) => Reply::Error {
                message: format!("查询任务失败: {e}"),
            },
        },
        Request::GetSummaryBatch { batch_id } => match summary_service.jobs.get_batch(batch_id) {
            Ok(batch) => Reply::SummaryBatch { batch },
            Err(e) => Reply::Error {
                message: format!("查询批次失败: {e}"),
            },
        },
        Request::RetrySummaryJob { job_id } => match summary_service.retry_job(job_id) {
            Ok(new_id) => Reply::SummaryJobSubmitted { job_id: new_id },
            Err(e) => Reply::Error {
                message: format!("重试失败: {e}"),
            },
        },
        Request::CancelSummaryBatch { batch_id } => match summary_service.cancel_batch(batch_id) {
            Ok(()) => Reply::Ok,
            Err(e) => Reply::Error {
                message: format!("取消失败: {e}"),
            },
        },
        Request::GetSummaryProvider {
            ui_choice,
            ui_model,
        } => {
            let info = match crate::summary_config::resolve_provider(ui_choice, ui_model) {
                crate::summary_config::SummaryProviderResolution::Configured(cfg, src) => {
                    dozer_core::protocol::SummaryProviderInfo {
                        provider: Some(cfg.provider),
                        model: cfg.model,
                        source: Some(
                            match src {
                                crate::summary_config::SummaryConfigSource::Ui => "ui",
                                crate::summary_config::SummaryConfigSource::Summary => "summary",
                                crate::summary_config::SummaryConfigSource::BuiltInDefault => {
                                    "built_in_default"
                                }
                            }
                            .to_string(),
                        ),
                        required: false,
                    }
                }
                crate::summary_config::SummaryProviderResolution::Required => {
                    dozer_core::protocol::SummaryProviderInfo {
                        provider: None,
                        model: None,
                        source: None,
                        required: true,
                    }
                }
            };
            Reply::SummaryProvider { info }
        }
        Request::GetSummaryResult { conversation_id } => {
            match summary_service.jobs.get_result(&conversation_id) {
                Ok(result) => Reply::SummaryResult { result },
                Err(e) => Reply::Error {
                    message: format!("查询结果失败: {e}"),
                },
            }
        }
        Request::LocateInFile {
            project_id,
            path,
            query,
        } => {
            let root = projects
                .list()
                .ok()
                .and_then(|ps| ps.into_iter().find(|p| p.id == project_id))
                .map(|p| std::path::PathBuf::from(p.path));
            match root {
                None => Reply::Error {
                    message: "项目不存在".into(),
                },
                Some(root) => match crate::file_mutation::locate_in_file(&root, &path, &query) {
                    Ok(matches) => Reply::LocateMatches { matches },
                    Err(crate::file_mutation::LocateError::Limit(reason)) => {
                        Reply::Error { message: reason }
                    }
                    Err(e) => Reply::LocateMatches {
                        matches: locate_error_to_empty_with_log(e, &path),
                    },
                },
            }
        }
        Request::LocateInFileV2 {
            project_id,
            path,
            query,
        } => {
            let root = projects
                .list()
                .ok()
                .and_then(|ps| ps.into_iter().find(|p| p.id == project_id))
                .map(|p| std::path::PathBuf::from(p.path));
            match root {
                None => Reply::Error {
                    message: "项目不存在".into(),
                },
                Some(root) => {
                    match crate::file_mutation::locate_in_file_report(&root, &path, &query) {
                        Ok(report) => Reply::LocateReport { report },
                        Err(crate::file_mutation::LocateError::Limit(reason)) => {
                            Reply::Error { message: reason }
                        }
                        Err(e) => Reply::LocateMatches {
                            matches: locate_error_to_empty_with_log(e, &path),
                        },
                    }
                }
            }
        }
        Request::ApplyPreciseEdit {
            project_id,
            path,
            start_line,
            start_col,
            end_line,
            end_col,
            expected_text,
            new_text,
            summary,
            actor,
            session_id,
        } => {
            let root = projects
                .list()
                .ok()
                .and_then(|ps| ps.into_iter().find(|p| p.id == project_id))
                .map(|p| std::path::PathBuf::from(p.path));
            match root {
                None => Reply::Error {
                    message: "项目不存在".into(),
                },
                Some(root) => {
                    let input = crate::file_mutation::ApplyEditInput {
                        start_line,
                        start_col,
                        end_line,
                        end_col,
                        expected_text,
                        new_text: new_text.clone(),
                    };
                    let outcome =
                        match crate::file_mutation::apply_precise_edit(&root, &path, input) {
                            Ok(Ok(applied)) => {
                                let history_id = file_edit_history
                                    .record(crate::file_edit_history::NewFileEditHistoryEntry {
                                        project_id,
                                        target_path: path.clone(),
                                        actor,
                                        session_id,
                                        start_line,
                                        start_col,
                                        end_line,
                                        end_col,
                                        old_text: applied.old_text,
                                        new_text,
                                        summary,
                                    })
                                    .unwrap_or_else(|e| {
                                        dozer_core::log_warn!(LOG,
                                            error = %e,
                                            path = %path,
                                            "file_edit_history 写入失败(文件已改盘,仅历史记录丢失)"
                                        );
                                        -1
                                    });
                                let outcome = dozer_core::protocol::MutationOutcome::Applied {
                                    new_start_line: applied.new_start_line,
                                    new_start_col: applied.new_start_col,
                                    new_end_line: applied.new_end_line,
                                    new_end_col: applied.new_end_col,
                                    history_id,
                                };
                                schedule_post_edit_reveal(
                                    preview_commands.clone(),
                                    project_id,
                                    path.clone(),
                                    applied.new_start_line,
                                    applied.new_start_col,
                                    applied.new_end_line,
                                    applied.new_end_col,
                                );
                                outcome
                            }
                            Ok(Err(actual_text)) => {
                                dozer_core::protocol::MutationOutcome::Conflict { actual_text }
                            }
                            Err(crate::file_mutation::LocateError::OutOfBounds) => {
                                dozer_core::protocol::MutationOutcome::PathOutOfBounds
                            }
                            Err(crate::file_mutation::LocateError::SymlinkRejected) => {
                                dozer_core::protocol::MutationOutcome::PathOutOfBounds
                            }
                            Err(crate::file_mutation::LocateError::NotFound) => {
                                dozer_core::protocol::MutationOutcome::NotFound
                            }
                            Err(crate::file_mutation::LocateError::UnsupportedFile(reason))
                            | Err(crate::file_mutation::LocateError::Io(reason))
                            | Err(crate::file_mutation::LocateError::Limit(reason)) => {
                                dozer_core::protocol::MutationOutcome::Unwritable { reason }
                            }
                            Err(crate::file_mutation::LocateError::Unwritable(reason)) => {
                                dozer_core::protocol::MutationOutcome::Unwritable { reason }
                            }
                            Err(crate::file_mutation::LocateError::EmptyQuery) => {
                                unreachable!("apply_precise_edit 不会产生 EmptyQuery")
                            }
                        };
                    Reply::MutationResult { outcome }
                }
            }
        }
        Request::AddContextItem {
            project_id,
            entity_kind,
            entity_ref,
        } => {
            let root = projects
                .list()
                .ok()
                .and_then(|ps| ps.into_iter().find(|p| p.id == project_id))
                .map(|p| std::path::PathBuf::from(p.path));
            match root {
                None => Reply::Error {
                    message: "项目不存在".into(),
                },
                Some(root) => {
                    match crate::agent_context::validate_context_ref(&root, &entity_ref) {
                        Err(message) => Reply::Error { message },
                        Ok(()) => match file_edit_history.add_context_item(
                            project_id,
                            &entity_kind,
                            &entity_ref,
                        ) {
                            Ok(item) => Reply::ContextItem { item },
                            Err(e) => Reply::Error {
                                message: format!("添加上下文项失败: {e}"),
                            },
                        },
                    }
                }
            }
        }
        Request::RemoveContextItem { project_id, id } => {
            match file_edit_history.remove_context_item(project_id, id) {
                Ok(()) => Reply::Ok,
                Err(e) => Reply::Error {
                    message: format!("移除上下文项失败: {e}"),
                },
            }
        }
        Request::ListContextItems { project_id } => {
            match file_edit_history.list_context_items(project_id) {
                Ok(items) => Reply::ContextItems { items },
                Err(e) => Reply::Error {
                    message: format!("列上下文项失败: {e}"),
                },
            }
        }
        Request::ListFileEditHistory {
            project_id,
            path_filter,
            limit,
        } => match file_edit_history.list_history(project_id, path_filter.as_deref(), limit) {
            Ok(entries) => Reply::FileEditHistory { entries },
            Err(e) => Reply::Error {
                message: format!("查修改历史失败: {e}"),
            },
        },
        _ => Reply::Error {
            message: "unsupported synchronous request".into(),
        },
    }
}
