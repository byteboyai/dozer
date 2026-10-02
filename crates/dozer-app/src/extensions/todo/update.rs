//! Todo 面板 update 消息分发 + 日历日期纯函数。

use dozer_client::Client;
use dozer_core::protocol::TodoInfo;

use super::*;

dozer_core::scope!(pub(crate) LOG, panel, "todo");

/// 本地时区无关的"今天" (年, 月, 日),用 `SystemTime::now()` 的 UTC 秒数
/// 经 `civil_from_days` 换算。只用于日历默认停在当前月,时区偏差一天内无感。
pub(crate) fn today_ymd() -> (i32, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (secs / 86400) as i64;
    let (y, m, d) = civil_from_days(days);
    (y as i32, m, d)
}

/// 完成时间(毫秒) → "MM-DD"(SUCCESS 徽章用,只取月日)。
pub(crate) fn format_todo_month_day(ms: u64) -> String {
    let secs = ms / 1000;
    let days = (secs / 86400) as i64;
    let (_y, m, d) = civil_from_days(days);
    format!("{m:02}-{d:02}")
}

/// civil-from-days:把"自 1970-01-01 的天数"换算成 (年, 月, 日)。
/// 用 Hinnant 经典公式,范围覆盖 1970..=2100,足够"完成于"提示用。
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

/// 解析 "MM-DD"(允许 `8-3` 这种缺前导零写法,兼容用户手敲的计划日期),
/// 返回 (月, 日);解析失败返回 `None`。
pub(crate) fn parse_month_day(s: &str) -> Option<(u32, u32)> {
    let (mm, dd) = s.split_once('-')?;
    let m: u32 = mm.trim().parse().ok()?;
    let d: u32 = dd.trim().parse().ok()?;
    (1..=12).contains(&m).then_some((m, d))
}

/// 新增任务的共享逻辑:插入乐观项(置顶)、起闪光、异步落库。`AddSubmit`(原生
/// 草稿)与 `AddText`(webview 文本)共用。
fn submit_new_todo(
    ws_state: &mut WorkspaceState,
    text: String,
    project_id: i64,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + Sync + 'static,
) {
    ws_state.items.insert(
        0,
        TodoInfo {
            id: OPTIMISTIC_TODO_ID,
            project_id,
            text: text.clone(),
            done: false,
            paused: false,
            rank: 0,
            created_ms: 0,
            completed_at_ms: None,
            plan_date: None,
            dispatch_session_id: None,
            dispatch_at_ms: None,
            category_id: None,
            assigned_agent: None,
        },
    );
    ws_state.start_flash(0);
    let client = client.clone();
    handle.spawn(async move {
        let res = client
            .add_todo(project_id, &text)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string());
        emit(Message::Mutated(res));
    });
}

/// 处理除 `DispatchToExisting` 之外的消息。`DispatchToExisting` 涉及终端
/// 会话读写,内核会先拦截,不会转发到这里。写操作(增/改/勾选/排序/计划
/// 日期/派发)一律走异步 `Client`,结果经 `Message::Loaded`/`Mutated`
/// 落回 `ws_state.items`。
pub fn update(
    ws_state: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + Sync + 'static,
) {
    match msg {
        // 卡片悬停由内核 `App::update` 拦截转发到 `set_hover`,不会到这。
        Message::Hover(_, _) => {}
        // footbar"清空列表"按钮:弹出确认框,不直接清空。
        Message::ClearListRequest => ws_state.clear_confirm = true,
        Message::ClearListCancel => ws_state.clear_confirm = false,
        // 确认弹窗"清空":清空本身尚未实现,先收起弹窗,仅占位。
        Message::ClearListConfirm => ws_state.clear_confirm = false,
        // `TextInputMenuOpen` 由内核拦截映射为右键菜单,不进这里。
        Message::TextInputMenuOpen(_) => {}
        // `ToggleListCollapse` 由内核拦截映射为列表列收起/展开,不进这里。
        Message::ToggleListCollapse => {}
        Message::Loaded(todos) => ws_state.items = todos,
        Message::Mutated(res) => {
            // 失败时列表随后会按磁盘状态刷新回旧样子,用户会以为没生效——Toast 说明。
            ws_state.outbox.push_err(LOG, "Todo 操作失败", &res);
            request_todos_refresh(project_id, client, handle, emit);
        }
        Message::CategoriesLoaded(categories) => ws_state.categories = categories,
        Message::CategoryMutated(res) => {
            ws_state.outbox.push_err(LOG, "分类操作失败", &res);
            let client1 = client.clone();
            let client2 = client.clone();
            let handle1 = handle.clone();
            let handle2 = handle.clone();
            let emit = std::sync::Arc::new(emit);
            let emit1 = emit.clone();
            let emit2 = emit;
            request_categories_refresh(project_id, &client1, &handle1, move |m| emit1(m));
            handle2.spawn(async move {
                let todos = client2.list_todos(project_id).await.unwrap_or_default();
                emit2(Message::Loaded(todos));
            });
        }
        Message::CategoryToggleExpand(id) => ws_state.toggle_category_expanded(id),
        Message::SelectView(view) => ws_state.view = view,
        Message::CategorySelect(filter) => {
            // 切显示分类时,前端按 `category_key` 变化重置搜索关键词(哪怕切回
            // 原来那个用关键词搜过的分类也要重置,不做"记住每个分类各自搜索词"
            // 那套)——搜索草稿/生效词现由 webview 侧持有,这里只改选中分类。
            ws_state.category_selected = filter;
        }
        Message::CategoryContextMenuOpen(_) => {}
        Message::CategoryNewChild(parent_id) => {
            let client = client.clone();
            let project_id_owned = project_id;
            handle.spawn(async move {
                let res = client
                    .add_category(project_id_owned, Some(parent_id), "新分类")
                    .await;
                match res {
                    Ok(category) => {
                        emit(Message::CategoryRenameStart(category.id));
                        emit(Message::CategoryMutated(Ok(())));
                    }
                    Err(e) => emit(Message::CategoryMutated(Err(e.to_string()))),
                }
            });
        }
        Message::CategoryNewSibling(parent_id) => {
            let client = client.clone();
            let project_id_owned = project_id;
            handle.spawn(async move {
                let res = client
                    .add_category(project_id_owned, parent_id, "新分类")
                    .await;
                match res {
                    Ok(category) => {
                        emit(Message::CategoryRenameStart(category.id));
                        emit(Message::CategoryMutated(Ok(())));
                    }
                    Err(e) => emit(Message::CategoryMutated(Err(e.to_string()))),
                }
            });
        }
        Message::CategoryDelete(id) => {
            let client = client.clone();
            handle.spawn(async move {
                let res = client.delete_category(id).await.map_err(|e| e.to_string());
                emit(Message::CategoryMutated(res));
            });
        }
        Message::CategoryRenameStart(id) => {
            let Some(current) = ws_state.categories.iter().find(|c| c.id == id) else {
                return;
            };
            ws_state.category_renaming = Some((id, current.name.clone()));
            ws_state.category_rename_focus_pending = true;
        }
        Message::CategoryRenameEdit(text) => ws_state.set_category_rename_draft(text),
        Message::CategoryRenameSubmit => {
            if let Some((id, new_name)) = ws_state.commit_category_rename() {
                let client = client.clone();
                handle.spawn(async move {
                    let res = client
                        .rename_category(id, &new_name)
                        .await
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                    emit(Message::CategoryMutated(res));
                });
            }
        }
        Message::CategoryMoveSibling(id, direction) => {
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .move_category_sibling(id, direction)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::CategoryMutated(res));
            });
        }
        Message::CategoryReparentPickerOpen(_) => {}
        Message::DetailClose => ws_state.close_detail(),
        Message::DetailReplyInput(text) => ws_state.detail_reply_draft = text,
        Message::DetailReplySubmit => {
            // 乐观插入 + 置处理中标记在这里做(纯本地状态);真正发
            // `ProcessTodoNow` RPC 在 `App::todo_detail_process`(app.rs),
            // 那边会读 `detail_reply_draft` 拿文本、读 `items()[idx].id`
            // 拿任务 id。
            let draft = std::mem::take(&mut ws_state.detail_reply_draft);
            if !draft.trim().is_empty() {
                ws_state.push_optimistic_human_turn(draft);
            }
        }
        Message::StatusPick(idx, target) => {
            let Some(item) = ws_state.items.get(idx) else {
                return;
            };
            let stored_target = match target {
                TodoState::Pending => dozer_core::protocol::TodoStoredStatus::Todo,
                TodoState::Done => dozer_core::protocol::TodoStoredStatus::Done,
                TodoState::Suspended => dozer_core::protocol::TodoStoredStatus::Suspended,
                TodoState::InProgress => return,
            };
            let client = client.clone();
            let id = item.id;
            handle.spawn(async move {
                let res = client
                    .set_todo_status(id, stored_target)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::Mutated(res));
            });
        }
        Message::CalendarPick(idx, day) => {
            if let Some(item) = ws_state.items.get(idx) {
                let id = item.id;
                let client = client.clone();
                handle.spawn(async move {
                    let res = client
                        .set_todo_plan_date(id, Some(&day))
                        .await
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                    emit(Message::Mutated(res));
                });
            }
        }
        Message::Toggle(idx) => {
            let Some(item) = ws_state.items.get(idx) else {
                return;
            };
            let id = item.id;
            let target = !item.done;
            // 乐观更新:本地立即翻转,给出与迁移前同等的零延迟反馈。
            if let Some(item) = ws_state.items.get_mut(idx) {
                item.done = target;
                // 同步完成/取消完成的时间戳(`completed_at_for_toggle`:完成
                // → `Some(now)`,取消 → `None`),让 Done 徽章的 "MM-DD" 在
                // 服务端往返回来之前就能显示(与服务端 TodoStore 口径一致)。
                item.completed_at_ms =
                    completed_at_for_toggle(target, std::time::SystemTime::now()).map(|t| {
                        t.duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64
                    });
            }
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .toggle_todo(id, target)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::Mutated(res));
            });
        }
        Message::AddText(text) => {
            submit_new_todo(ws_state, text, project_id, client, handle, emit);
        }
        Message::EditText(id, text) => {
            let text = text.trim().to_string();
            let Some(item) = ws_state.items.iter().find(|i| i.id == id) else {
                return;
            };
            if text.is_empty() || item.text == text {
                return;
            }
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .edit_todo_text(id, &text)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::Mutated(res));
            });
        }
        Message::ReorderTo { id, after_id } => {
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .reorder_todo(id, after_id)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::Mutated(res));
            });
        }
        Message::SetCategory(id, category_id) => {
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .set_todo_category(id, category_id)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                // 与原分类选择器同一条刷新路径:分类与任务列表一并重拉。
                emit(Message::CategoryMutated(res));
            });
        }
        Message::AddHeight(px) => ws_state.set_add_input_height(px),
        Message::ContentRetry => {}
    }
}
