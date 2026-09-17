//! Todo 面板 update 消息分发 + 日历日期纯函数。

use dozer_client::Client;
use dozer_core::protocol::TodoInfo;

use super::*;

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

/// 解析 "MM-DD"(允许 `8-3` 这种缺前导零写法,兼容用户手敲的计划日期),
/// 返回 (月, 日);解析失败返回 `None`。
pub(crate) fn parse_month_day(s: &str) -> Option<(u32, u32)> {
    let (mm, dd) = s.split_once('-')?;
    let m: u32 = mm.trim().parse().ok()?;
    let d: u32 = dd.trim().parse().ok()?;
    (1..=12).contains(&m).then_some((m, d))
}

/// `civil_from_days` 的逆运算:把 (年, 月, 日) 换算回"自 1970-01-01 的天数",
/// 给日历算"某月 1 号是星期几"和"某月有多少天"用。只覆盖 1970..=2100,
/// 与 `civil_from_days` 同范围。
pub(crate) fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = y as i64;
    let m = m as i64;
    let d = d as i64;
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// 某年某月有多少天。
pub(crate) fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
            if leap { 29 } else { 28 }
        }
        _ => 0,
    }
}

/// 某年某月 1 号是星期几:0 = 周日,1 = 周一 … 6 = 周六(1970-01-01 是周四)。
pub(crate) fn first_weekday_of_month(y: i32, m: u32) -> u32 {
    (days_from_civil(y, m, 1) + 4).rem_euclid(7) as u32
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
            if let Err(e) = res {
                tracing::warn!("Todo 写操作失败: {e}");
            }
            request_todos_refresh(project_id, client, handle, emit);
        }
        Message::CategoriesLoaded(categories) => ws_state.categories = categories,
        Message::CategoryMutated(res) => {
            if let Err(e) = res {
                tracing::warn!("分类写操作失败: {e}");
            }
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
            ws_state.category_selected = filter;
            // 切显示分类重置搜索关键词过滤(原在切换状态分类时做,现左栏只留
            // 自定义分类这一口径,故改到切换分类这里):哪怕切回原来那个用
            // 关键词搜过的分类也要重置,不做"记住每个分类各自搜索词"那套。
            ws_state.clear_search();
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
        Message::CategoryPickerOpenForTodo(_) => {}
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
        Message::AddEdit(action) => ws_state.add_draft.perform(action),
        Message::AddSubmit => {
            let text = ws_state.add_draft.text();
            let text = text.trim().to_string();
            if text.is_empty() {
                return;
            }
            ws_state.add_draft = iced_widget::text_editor::Content::new();
            // 乐观置顶:插入一条临时 id 的条目,立即触发既有的"新增闪光+
            // 滚回顶部"效果,不等服务端往返。
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
            let project_id_owned = project_id;
            handle.spawn(async move {
                let res = client
                    .add_todo(project_id_owned, &text)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::Mutated(res));
            });
        }
        // 高度拖拽在 app 层 `todo_message` 已早退,不会到这里;保留 arm 仅
        // 为 match 穷尽。
        Message::AddResizeStart => {}
        Message::RowSelect(idx) => {
            ws_state.selected_row = idx;
            // 用户手动选中(点卡片空白处)会打断"新增闪光":否则 2 秒计时到点
            // 会把用户刚主动选的卡片又自动取消选中。`take_scroll_to_top` 未定
            // 时用户主动点才会走到这里,新增闪光阶段不处理(见 `Message::Toggle`
            // 之上对闪光来源的约定)。
            ws_state.flash = None;
            // 活动卡片被按下即"准备拖":记下它的 item-index 作为拖拽源。
            // 搁置/完成不参与拖拽(只有活动段才进 `drag`)。注意这跟选中态是
            // 两件独立的事——纯点击(不移动)也会落到这里,但松手时
            // source==target 不写盘,只是正常选中切换(同 `TabDragMove`
            // 的"按住=准备拖,移动才换位"语义)。
            if let Some(i) = idx
                && let Some(item) = ws_state.items.get(i)
                && is_active_todo(item)
            {
                ws_state.drag = Some(TodoDrag {
                    source_idx: i,
                    target_idx: i,
                });
            }
        }
        Message::SearchInput(s) => ws_state.search_draft = s,
        Message::SearchSubmit => ws_state.commit_search(),
        Message::DragMove(over_idx) => {
            // 只有"正在拖"才生效;纯悬停不会动任何东西。
            let Some(drag) = ws_state.drag else {
                return;
            };
            // 活动段尾部下标:最后一个活动(非 done && 非 paused)项的
            // item-index。搁置/完成都夹不到活动段的"之后"(活动段是整它自己
            // 那一段),落到非活动行就一律取"最后一个活动"当落点。
            let last_active = ws_state
                .items
                .iter()
                .enumerate()
                .rfind(|(_, it)| is_active_todo(it))
                .map(|(i, _)| i);
            let target = match ws_state.items.get(over_idx) {
                Some(it) if is_active_todo(it) => over_idx,
                // 走到这个分支时表示悬停到搁置/完成(或在它之上)。为了让用户把
                // 活动拖到"搁置段之前",落点取最后一个活动项的 idx(下面换算成
                // after_id);若根本没有可落的活动行则走 usize::MAX(交服务端
                // 按"待办块挪到最前"处理)。注意 source 等于该 last_active(正要
                // 把它挪到自己之后)会让 last_active 与 itself 结算成 no-op。
                _ => last_active.unwrap_or(usize::MAX),
            };
            if target != drag.target_idx {
                ws_state.drag = Some(TodoDrag {
                    source_idx: drag.source_idx,
                    target_idx: target,
                });
            }
        }
        Message::DragEnd => {
            let Some(drag) = ws_state.drag.take() else {
                return;
            };
            if drag.source_idx == drag.target_idx {
                return;
            }
            let Some(source_item) = ws_state.items.get(drag.source_idx) else {
                return;
            };
            let id = source_item.id;
            // 落点 target 只在"另一个活动项"上(DragMove 里已保证夹在活动段
            // 内、且 source != target 已在上方早退),after_id 直接取它的 id,
            // 让服务端把本任务挪到它之后。活动段/搁置段边界由 DragMove 夹好,
            // 这里不需要再区分 usize::MAX。
            let after_id = ws_state.items.get(drag.target_idx).map(|it| it.id);
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
        Message::DispatchOpen(idx) => {
            // 与卡片浮层互斥:开派发层顺手收起状态筛选、日历、卡片状态下拉。
            ws_state.close_status_filter_popup();
            ws_state.calendar_open = None;
            ws_state.status_open = None;
            ws_state.dispatch_open = Some(idx);
        }
        Message::DispatchClose => {
            ws_state.dispatch_open = None;
            ws_state.dispatch_anchor = None;
        }
        Message::AssignAgent(_, _) => {
            // 真正的 RPC 调用在 `App::todo_assign_agent`(app.rs),这里
            // 只负责关掉选择层——与 `DispatchClose` 同款收尾。
            ws_state.dispatch_open = None;
            ws_state.dispatch_anchor = None;
        }
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
        Message::DetailOpen(_) => {
            // 真正拉 `GetTodoDetail` 在 `App::todo_detail_open`(app.rs),
            // `todo::update` 不处理这条(no-op arm 保持 match 穷尽,同
            // `AddResizeStart` 的既有模式)。
        }
        Message::StatusOpen(idx) => {
            // 同一时刻只允许一个卡片弹层(状态/日历/派发互斥):打开状态下拉时
            // 顺手把另外两个收起,避免叠两层卡片浮层。
            ws_state.close_status_filter_popup();
            ws_state.dispatch_open = None;
            ws_state.calendar_open = None;
            ws_state.status_open = Some(idx);
        }
        Message::StatusClose => ws_state.close_status_popup(),
        Message::StatusPick(idx, target) => {
            // 关闭状态下拉;无论目标是什么都先收起浮层,再做分支处理。
            ws_state.close_status_popup();
            let Some(item) = ws_state.items.get(idx) else {
                return;
            };
            // 需要把状态落到 dozerd 的存储态。InProgress 不是存储位:真正
            // "进进行中"要一次指派(选到某个存活会话);这里要么把这个分支
            // 转给派发选择层(待办/搁置想进进行中),要么撤销搁置先变"待办"。
            let stored_target = match target {
                TodoState::Pending => Some(dozer_core::protocol::TodoStoredStatus::Todo),
                TodoState::Done => Some(dozer_core::protocol::TodoStoredStatus::Done),
                TodoState::Suspended => Some(dozer_core::protocol::TodoStoredStatus::Suspended),
                TodoState::InProgress => None,
            };
            // InProgress:当前没存活会话(从 Pending/Suspended 想转)就交给
            // 派发选择层;已经 InProgress 的选它不做事。搁置(拿回暂停)想
            // 进"进行中"先把 stored 位撤回待办(撤销暂停),下一拍用户再走
            // "指派"一个存活会话；这里不连做两次写只因想分清"取消搁置"跟
            // "指派会话"两件原子动作。
            if target == TodoState::InProgress {
                if item.paused {
                    let client = client.clone();
                    let id = item.id;
                    handle.spawn(async move {
                        let res = client
                            .set_todo_status(id, dozer_core::protocol::TodoStoredStatus::Todo)
                            .await
                            .map(|_| ())
                            .map_err(|e| e.to_string());
                        emit(Message::Mutated(res));
                    });
                } else {
                    // 非搁置想进进行中 = 想指派一个会话 → 把状态浮层让位给
                    // 派发浮层,并沿用它自己的弹出锚点(刚记录的那次点击)。
                    if let Some(a) = ws_state.status_anchor.take() {
                        ws_state.dispatch_anchor = Some(a);
                    }
                    ws_state.dispatch_open = Some(idx);
                }
                ws_state.status_open = None;
                ws_state.status_anchor = None;
                return;
            }
            let Some(stored_target) = stored_target else {
                return;
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
        Message::StatusFilterOpen => {
            // 与卡片浮层互斥:开搜索框筛选浮层时先收起派发层/日历/卡片状态。
            ws_state.status_open = None;
            ws_state.dispatch_open = None;
            ws_state.calendar_open = None;
            ws_state.open_status_filter();
        }
        Message::StatusFilterClose => ws_state.close_status_filter_popup(),
        Message::StatusFilterPick(filter) => {
            // 纯本地筛选轴:只改选中项,不落盘、不动搜索关键词。选中某项或
            // "全部"后顺带关闭浮层。
            ws_state.status_filter = filter;
            ws_state.close_status_filter_popup();
        }
        Message::CalendarOpen(idx) => {
            // 与 StatusOpen/派发同样"同时只能有一个浮层":开日历时收起状态
            // 提层/派发层,以及搜索框的状态筛选浮层。
            ws_state.close_status_filter_popup();
            ws_state.status_open = None;
            ws_state.status_anchor = None;
            ws_state.dispatch_open = None;
            // 打开日历:默认停在"当前月",若任务已有计划日期且能解析成 MM-DD,
            // 则把视图拨到该月(年份取当前年——plan_date 只有月日,无年份)。
            let (now_y, now_m, _) = today_ymd();
            let (y, m) = ws_state
                .items
                .get(idx)
                .and_then(|item| item.plan_date.clone())
                .and_then(|s| parse_month_day(&s))
                .map(|(mm, _)| (now_y, mm))
                .unwrap_or((now_y, now_m));
            ws_state.calendar_open = Some(idx);
            ws_state.calendar_view = (y, m);
        }
        Message::CalendarClose => ws_state.close_calendar_popup(),
        Message::CalendarPrevMonth => {
            let (y, m) = ws_state.calendar_view;
            ws_state.calendar_view = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
        }
        Message::CalendarNextMonth => {
            let (y, m) = ws_state.calendar_view;
            ws_state.calendar_view = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
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
            ws_state.close_calendar_popup();
        }
        Message::ContentEditStart(idx) => {
            // 已经在编辑同一张卡:保持草稿(重击只用于鼠标定位,不重置,
            // 否则点一下就把刚改了一半的文字丢掉)。新卡进入编辑态则重置草稿。
            if ws_state
                .editing_content
                .as_ref()
                .is_some_and(|(eidx, _)| *eidx == idx)
            {
                return;
            }
            if let Some(item) = ws_state.items.get(idx) {
                ws_state.editing_content = Some((
                    idx,
                    iced_widget::text_editor::Content::with_text(&item.text),
                ));
            }
            // 点卡片文字这个点击落在旧的文字 `MouseArea` 上,真 `text_editor`
            // 下一帧才出现、不会自己拿聚焦,置位一次性聚焦标记。
            ws_state.content_edit_focus_pending = true;
        }
        Message::ContentEdit(action) => {
            let is_enter = matches!(
                action,
                iced_widget::text_editor::Action::Edit(iced_widget::text_editor::Edit::Enter)
            );
            if let Some((idx, draft)) = ws_state.editing_content.as_mut() {
                draft.perform(action.clone());
                if is_enter {
                    let idx = *idx;
                    let new_text = draft.text().trim().to_string();
                    let old_item = ws_state.items.get(idx).cloned();
                    ws_state.editing_content = None;
                    if let Some(old_item) = old_item
                        && !new_text.is_empty()
                        && old_item.text != new_text
                    {
                        let id = old_item.id;
                        let client = client.clone();
                        handle.spawn(async move {
                            let res = client
                                .edit_todo_text(id, &new_text)
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string());
                            emit(Message::Mutated(res));
                        });
                    }
                }
            }
        }
    }
}
