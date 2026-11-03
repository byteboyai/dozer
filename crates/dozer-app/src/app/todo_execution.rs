//! Task execution requests, polling lifetime and project/task/revision routing.
use super::*;
use crate::extensions::{toast, todo};
impl App {
    /// 打开任务详情弹窗:先本地记下 `idx`(弹窗定位/后续"处理"要用),再
    /// 异步拉 `GetTodoDetail`。RPC 结果经专门的 `Message::TodoDetailLoaded`
    /// 落地——`todo::Message::Mutated` 那条通用刷新链路只刷 `items`/
    /// `categories`,不携带回合数据,不能复用。
    pub(crate) fn todo_detail_open(&mut self, idx: usize) {
        let Some((id, project_id)) = self
            .active_workspace()
            .and_then(|ws| ws.todo.items().get(idx))
            .map(|item| (item.id, item.project_id))
        else {
            return;
        };
        self.with_focused_project(|ws, _io| {
            ws.todo.open_detail(idx, Vec::new());
        });
        let nonce = self.active_workspace().unwrap().todo.detail_nonce;
        let handle = self.handle.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        let watch = handle.spawn(async move {
            let mut running = false;
            let capabilities = client.runtime_capabilities().await;
            if capabilities.as_ref().is_ok_and(|c| c.task_execution_state) {
                match client.get_todo_execution(id).await {
                    Ok(info) => {
                        running = info.as_ref().is_some_and(|i| {
                            matches!(
                                i.status,
                                dozer_model::TaskExecutionStatus::Running
                                    | dozer_model::TaskExecutionStatus::Queued
                            )
                        });
                        let _ = proxy.send_event(Message::TodoExecutionLoaded(
                            project_id,
                            id,
                            nonce,
                            Ok(info),
                        ));
                    }
                    Err(e) => {
                        let _ = proxy.send_event(Message::TodoExecutionLoaded(
                            project_id,
                            id,
                            nonce,
                            Err(e.to_string()),
                        ));
                    }
                }
            }
            match client.get_todo_detail(id).await {
                Ok((_, turns)) => {
                    let _ =
                        proxy.send_event(Message::TodoDetailLoaded(project_id, id, nonce, turns));
                }
                Err(e) => {
                    // 详情弹窗已经打开(空回合),读取失败时用户只会看到一片空白。
                    let _ = proxy.send_event(Message::Toast(toast::Message::Push {
                        scope: todo::LOG,
                        level: toast::Level::Error,
                        text: dozer_model::LocalizedText::keyed(
                            crate::i18n::HOST_OWNER,
                            "toast.todo_detail_load_failed",
                            "Could not load task details: {error}",
                        )
                        .with_arg("error", e.to_string()),
                        key: None,
                    }));
                }
            }
            while running {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                match client.get_todo_execution(id).await {
                    Ok(info) => {
                        running = info.as_ref().is_some_and(|i| {
                            matches!(
                                i.status,
                                dozer_model::TaskExecutionStatus::Running
                                    | dozer_model::TaskExecutionStatus::Queued
                            )
                        });
                        let _ = proxy.send_event(Message::TodoExecutionLoaded(
                            project_id,
                            id,
                            nonce,
                            Ok(info),
                        ));
                    }
                    Err(e) => {
                        let _ = proxy.send_event(Message::TodoExecutionLoaded(
                            project_id,
                            id,
                            nonce,
                            Err(e.to_string()),
                        ));
                        break;
                    }
                }
                if !running {
                    if let Ok((_, turns)) = client.get_todo_detail(id).await {
                        let _ = proxy
                            .send_event(Message::TodoDetailLoaded(project_id, id, nonce, turns));
                    }
                }
            }
        });
        self.with_project(project_id, |ws, _| {
            ws.todo.set_detail_watch(watch.abort_handle())
        });
    }

    /// 详情弹窗"处理"按钮:乐观插入已经在 `todo::update`(`DetailReplySubmit`
    /// 分支)做过,这里只管发 `ProcessTodoNow` RPC 并在结果回来后用服务端
    /// 权威回合列表刷新。耗时可能到 10 分钟,走 `handle.spawn` 不阻塞 UI。
    pub(crate) fn todo_detail_process(&mut self) {
        let Some((project_id, id, mut nonce, reply_text)) =
            self.active_workspace().and_then(|ws| {
                let idx = ws.todo.detail_open_idx()?;
                let item = ws.todo.items().get(idx)?;
                Some((item.project_id, item.id, ws.todo.detail_nonce, {
                    let draft = ws.todo.detail_reply_draft().trim();
                    (!draft.is_empty()).then(|| draft.to_string())
                }))
            })
        else {
            return;
        };
        nonce = nonce.wrapping_add(1);
        self.with_project(project_id, |ws, _| {
            ws.todo.detail_nonce = nonce;
            ws.todo.detail_processing = true;
            ws.todo.detail_execution_error = None;
            ws.todo.detail_reply_draft.clear();
        });
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        let watch = self.handle.spawn(async move {
            let outcome: Result<(), String> = async {
                if client
                    .runtime_capabilities()
                    .await
                    .map_err(|e| e.to_string())?
                    .task_execution_state
                {
                    // One key per deliberate user action. A transport timeout is never replayed.
                    let key = uuid::Uuid::new_v4().to_string();
                    let mut execution = client
                        .start_todo_execution(id, reply_text.as_deref(), Some(&key))
                        .await
                        .map_err(|e| e.to_string())?;
                    loop {
                        let running = matches!(
                            execution.status,
                            dozer_model::TaskExecutionStatus::Running
                                | dozer_model::TaskExecutionStatus::Queued
                        );
                        let _ = proxy.send_event(Message::TodoExecutionLoaded(
                            project_id,
                            id,
                            nonce,
                            Ok(Some(execution)),
                        ));
                        if !running {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        let Some(next) = client
                            .get_todo_execution(id)
                            .await
                            .map_err(|e| e.to_string())?
                        else {
                            return Err("Execution state unavailable".into());
                        };
                        execution = next;
                    }
                } else {
                    client
                        .process_todo_now(id, reply_text.as_deref())
                        .await
                        .map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            .await;
            if let Ok((_, turns)) = client.get_todo_detail(id).await {
                let _ = proxy.send_event(Message::TodoDetailLoaded(project_id, id, nonce, turns));
            }
            if let Err(error) = outcome {
                let _ = proxy.send_event(Message::TodoExecutionLoaded(
                    project_id,
                    id,
                    nonce,
                    Err(error),
                ));
            }
        });
        self.with_project(project_id, |ws, _| {
            ws.todo.set_detail_watch(watch.abort_handle())
        });
    }
}
