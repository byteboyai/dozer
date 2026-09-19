//! `App::update` 消息分发 + 全部 handler 方法。Phase 3 结构重组时从
//! `app/app.rs` 拆出,逻辑保持原样。

use crate::chrome::homespace::{self, load_home_recents};
use crate::chrome::rail;
use crate::chrome::tab_widget;
use crate::extensions::browser;
use crate::extensions::conversations;
use crate::extensions::database;
use crate::extensions::file_history;
use crate::extensions::files;
use crate::extensions::footbar;
use crate::extensions::git_log;
use crate::extensions::project;
use crate::extensions::project_create;
use crate::extensions::search;
use crate::extensions::settings;
use crate::extensions::ssh;
use crate::extensions::todo;
use crate::extensions::usage;
use crate::git_watch;
use crate::term::terminal;
use crate::workspace::{
    CONVERSATION_DETAIL_PAGE_SIZE, RestorePayload, ReviewSource, ReviewView, SshOut,
    TabAttachedArgs, TabBackend, Workspace, exited_marker, preview_tab_display_width,
    relative_time_text, review_should_refresh_on_turn, spawn_disk_usage_refresh,
    spawn_project_git_refresh, tab_display_width, tab_title,
};
use byteui::interaction::icons;
use dozer_core::protocol::{AgentKind, AgentState, BookmarkInfo, ProjectInfo};
use iced_widget::core::{Border, Element, Length, Padding};
use iced_widget::{button, column, container, row, text};
use std::path::PathBuf;

use super::*;
impl App {
    pub fn update(&mut self, message: Message) {
        match message {
            Message::TermInput(target, bytes) => self.term_input(target, bytes),
            Message::TermImePreedit(target, text) => {
                if target == self.keyboard_term_target() {
                    self.term_ime_preedit = text;
                }
            }
            Message::TermOutput(project_id, tab_id, bytes) => {
                self.term_output(project_id, tab_id, bytes)
            }
            Message::SessionExited(project_id, tab_id) => {
                self.with_project(project_id, |ws, _io| {
                    if let Some(tab) = ws.tab_by_id_mut(tab_id) {
                        tab.alive = false;
                        // 本地标记行，非会话真实输出；应答无处可写，丢弃。
                        let _ = tab.model.feed(&exited_marker());
                    }
                });
            }
            Message::AgentStateChanged(project_id, tab_id, agent, state, transcript_path) => {
                self.agent_state_changed(project_id, tab_id, agent, state, transcript_path)
            }
            Message::AgentCardRefreshed(
                project_id,
                tab_id,
                llm_model,
                mode,
                activity,
                workspace,
            ) => {
                self.with_project(project_id, |ws, _io| {
                    crate::workspace::apply_agent_card_refresh(
                        &mut ws.tabs,
                        tab_id,
                        llm_model,
                        mode,
                        activity,
                        workspace,
                    );
                });
            }
            Message::ReviewLoaded(project_id, source, append, result) => {
                self.with_project(project_id, |ws, _io| {
                    if result.is_ok() {
                        ws.review_nonce = ws.review_nonce.wrapping_add(1);
                    }
                    let nonce = ws.review_nonce;
                    if let Some(rv) = &mut ws.review
                        && rv.source == source
                    {
                        match result {
                            Ok(entries) => {
                                // 快照写入必须放在 `rv.source == source` 判断
                                // 通过之后——这是它跟旧实现(main.rs 里的裸
                                // `static`,过期/乱序结果也会无条件覆盖)的
                                // 关键区别,过期加载结果到这里已经被
                                // 上面的守卫挡在外面,不会再污染快照。
                                merge_review_entries(&mut rv.entries, entries, append);
                                let snapshot = ReviewSnapshot {
                                    entries: &rv.entries,
                                    agent_label: rv.agent.label(),
                                    summary_title: rv.summary_title.clone(),
                                    summary_text: rv.summary_text.clone(),
                                    summary_time: rv.summary_time.clone(),
                                };
                                let json = serde_json::to_string(&snapshot).unwrap_or_default();
                                *ws.review_snapshot.lock().expect("review snapshot 锁") =
                                    Some(json);
                                rv.nonce = nonce;
                                rv.error = None;
                            }
                            Err(e) => rv.error = Some(e),
                        }
                    }
                });
            }
            Message::Conversations(conversations::Message::SessionsRefreshed(
                project_id,
                result,
            )) => {
                self.with_project(project_id, move |ws, _io| {
                    conversations::update(
                        &mut ws.conversations,
                        conversations::Message::SessionsRefreshed(project_id, result),
                    );
                });
            }
            Message::Conversations(conversations::Message::SessionOpen(conversation_id, agent)) => {
                self.conversation_session_open(conversation_id, agent);
            }
            Message::Conversations(conversations::Message::DetailLoadMore(
                conversation_id,
                after_turn_index,
            )) => {
                self.with_focused_project(|ws, io| {
                    ws.spawn_review_load_conversation(
                        io,
                        conversation_id,
                        after_turn_index,
                        CONVERSATION_DETAIL_PAGE_SIZE,
                        true,
                    );
                });
            }
            Message::Conversations(conversations::Message::Hover(id, h)) => self.set_hover(id, h),
            Message::Conversations(conversations::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Conversations(conversations::Message::AgentPickerOpen) => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = self
                        .active_workspace()
                        .map(|ws| conversations::agent_picker_items(&ws.conversations))
                        .unwrap_or_default();
                    if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                        self.update(Message::Conversations(msg));
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.with_focused_project(|ws, _io| {
                        conversations::update(
                            &mut ws.conversations,
                            conversations::Message::AgentPickerOpen,
                        );
                    });
                }
            }
            Message::Conversations(msg) => {
                self.with_focused_project(|ws, _io| {
                    conversations::update(&mut ws.conversations, msg);
                });
            }
            Message::ProjectDeleteDone(errors) => {
                if !errors.is_empty() {
                    self.daemon_error = Some(format!("删除项目未完全成功: {}", errors.join("; ")));
                }
            }
            Message::Usage(msg @ usage::Message::Loaded(project_id, ..)) => {
                self.with_project(project_id, move |ws, _io| {
                    usage::update(&mut ws.usage, msg);
                });
            }
            Message::Usage(usage::Message::ToggleListCollapse) => {
                self.toggle_panel_list_collapse(PanelKind::Usage);
            }
            Message::Usage(usage::Message::Hover(id, h)) => self.set_hover(id, h),
            Message::Usage(msg) => {
                self.with_focused_project(|ws, _io| {
                    usage::update(&mut ws.usage, msg);
                });
            }
            Message::SelectTab(idx) => self.select_tab(idx),
            Message::SelectTabNoDrag(idx) => self.select_tab_no_drag(idx),
            Message::CloseTab(idx) => {
                self.with_focused_project(|ws, io| {
                    ws.close_tab(io, idx);
                    ws.ensure_project_terminal(io);
                });
            }
            Message::AgentPickerToggle => {
                #[cfg(target_os = "macos")]
                {
                    let (x, y) = self.last_cursor;
                    let items = crate::workspace::agent_picker_items();
                    if let Some(msg) = crate::chrome::native_menu::show(items, (x, y)) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.with_focused_project(|ws, _io| {
                        ws.agent_picker_open = !ws.agent_picker_open;
                    });
                }
            }
            Message::AgentPickerClose => {
                self.with_focused_project(|ws, _io| {
                    ws.agent_picker_open = false;
                });
            }
            Message::AgentPickerSelect(agent) => {
                self.with_focused_project(|ws, io| {
                    ws.agent_picker_open = false;
                    ws.spawn_new_tab(io, agent, None);
                });
            }
            Message::ProjectAddMenuToggle => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let open_ids: std::collections::HashSet<i64> =
                        self.projects.keys().copied().collect();
                    let items = crate::chrome::topbar::project_add_menu_items(
                        &self.recent_projects,
                        &open_ids,
                    );
                    if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.project_add_menu_open = !self.project_add_menu_open;
                    if self.project_add_menu_open {
                        // 点"＋"时的光标逻辑坐标,作为菜单弹出锚点——同
                        // `todo::set_calendar_anchor`/`set_dispatch_anchor` 手法。
                        self.project_add_menu_anchor = self.last_cursor;
                    }
                }
            }
            Message::ProjectAddMenuClose => {
                self.project_add_menu_open = false;
            }
            Message::Todo(todo::Message::AssignAgent(idx, agent)) => {
                self.todo_assign_agent(idx, agent)
            }
            Message::Todo(todo::Message::DetailOpen(idx)) => self.todo_detail_open(idx),
            Message::Todo(todo::Message::DetailReplySubmit) => self.todo_detail_process(),
            // 数据库连接测试的异步结果带显式 `project_id`——用户可能在等待
            // 期间切走了项目页签,必须按自带 id 路由,不能用当前聚焦项目
            // (同 `TabAttached`/`ProjectSlotLoaded` 那批异步消息的约定,见设计
            // 文档"结果经 `emit` 回传"一节)。特化分支必须排在通配
            // `Message::Database(msg)` **之前**,否则永远匹配不到。
            Message::Database(database::Message::TestConnectionResult(
                project_id,
                source_id,
                result,
            )) => self.database_test_connection_result(project_id, source_id, result),
            // schema 树两个异步结果同 `TestConnectionResult` 口径:自带 project_id,
            // 按自带 id 路由,不能用当前聚焦项目。特化分支必须排在通配
            // `Message::Database(msg)` 之前,否则永远匹配不到。
            Message::Database(database::Message::TablesLoaded(project_id, source_id, result)) => {
                self.database_tables_loaded(project_id, source_id, result)
            }
            Message::Database(database::Message::ColumnsLoaded {
                project_id,
                source_id,
                schema,
                table,
                result,
            }) => self.database_columns_loaded(project_id, source_id, schema, table, result),
            Message::Database(database::Message::BrowseResult(
                project_id,
                tab_id,
                run_seq,
                result,
            )) => self.database_browse_result(project_id, tab_id, run_seq, result),
            Message::Database(database::Message::QueryResult(
                project_id,
                tab_id,
                run_seq,
                result,
            )) => self.database_query_result(project_id, tab_id, run_seq, result),
            Message::Database(database::Message::SourceContextMenu(source_id)) => {
                self.database_source_context_menu(source_id);
            }
            Message::Database(database::Message::TabHover(target, idx, hovered)) => {
                // 内容窗格 tab 本体/关闭按钮的悬停,转发成 `HoverId`(同
                // `ToolbarHover` 的口径)。
                let id = match target {
                    database::DatabaseTabHoverTarget::Title => HoverId::DatabaseTabItem(idx),
                    database::DatabaseTabHoverTarget::Close => HoverId::DatabaseTabClose(idx),
                };
                self.set_hover(id, hovered);
            }
            Message::Database(database::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Database(database::Message::ToggleListCollapse) => {
                self.toggle_panel_list_collapse(PanelKind::Database);
            }
            Message::Database(database::Message::TabOverflowToggle) => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = self
                        .active_workspace()
                        .map(|ws| database::tab_overflow_items(&ws.database))
                        .unwrap_or_default();
                    if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                        self.update(Message::Database(msg));
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let last_cursor = self.last_cursor;
                    self.with_focused_project(|ws, _io| {
                        ws.database.content_mut().toggle_tab_overflow(last_cursor);
                    });
                }
            }
            Message::Database(database::Message::TabOverflowDismiss) => {
                self.with_focused_project(|ws, _io| {
                    ws.database.content_mut().dismiss_tab_overflow();
                });
            }
            Message::Database(database::Message::Hover(id, h)) => self.set_hover(id, h),
            Message::Database(msg) => self.database_message(msg),
            Message::Todo(msg) => match msg {
                todo::Message::Hover(id, h) => self.set_hover(id, h),
                todo::Message::TextInputMenuOpen(target) => {
                    self.update(Message::TextInputMenuOpen(target));
                }
                todo::Message::ToggleListCollapse => {
                    self.toggle_panel_list_collapse(PanelKind::Todo);
                }
                todo::Message::CategoryContextMenuOpen(id) => {
                    self.todo_category_context_menu(id);
                }
                // 以下几种分类动作都是从右键菜单里点出来的:先关掉菜单本
                // 身(浮层 if-else 链里 `category_context_menu` 分支排在
                // `category_picker` 之前,不关会导致"移动到..."开了选择器
                // 却永远被菜单盖住),镜像 `files.rs::RenameStart` 落盘动作
                // 时 `app_state.context_menu = None;` 的既有口径。
                todo::Message::CategoryReparentPickerOpen(id) => {
                    self.category_context_menu = None;
                    self.todo_category_picker_open(CategoryPickerTarget::Category(id));
                }
                todo::Message::CategoryNewChild(_)
                | todo::Message::CategoryNewSibling(_)
                | todo::Message::CategoryDelete(_)
                | todo::Message::CategoryRenameStart(_)
                | todo::Message::CategoryMoveSibling(_, _) => {
                    self.category_context_menu = None;
                    self.todo_message(msg);
                }
                todo::Message::CategoryPickerOpenForTodo(todo_id) => {
                    self.todo_category_picker_open(CategoryPickerTarget::Todo(todo_id));
                }
                todo::Message::DispatchOpen(idx) => {
                    #[cfg(target_os = "macos")]
                    {
                        let last_cursor = self.last_cursor;
                        let items = todo::dispatch_items(idx);
                        if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                            self.update(Message::Todo(msg));
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        self.todo_message(todo::Message::DispatchOpen(idx));
                    }
                }
                todo::Message::StatusOpen(idx) => {
                    #[cfg(target_os = "macos")]
                    {
                        let last_cursor = self.last_cursor;
                        let items = todo::status_items(idx);
                        if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                            self.update(Message::Todo(msg));
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        self.todo_message(todo::Message::StatusOpen(idx));
                    }
                }
                other => self.todo_message(other),
            },
            Message::TodoDetailLoaded(idx, turns) => {
                self.with_focused_project(move |ws, _io| {
                    if ws.todo.detail_open_idx() == Some(idx) {
                        ws.todo.replace_detail_turns(turns);
                    }
                });
            }
            // 文件树右键"搜索"弹窗:`SearchResults` 带 `project_id`,异步结果
            // 按所属项目路由(用户可能已切走);其余交互投当前聚焦项目。
            Message::Search(search::Message::SearchResults(project_id, result)) => {
                self.search_results(project_id, result)
            }
            Message::Search(search::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Search(msg) => {
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                self.with_focused_project(move |ws, _io| {
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Search(m));
                    };
                    search::update(&mut ws.search, msg, project_id, &handle, emit);
                });
            }
            Message::TabAttached(project_id, tab_id, info, snapshot, picked_agent) => {
                let term_tab_bar_avail_px = self.terminal_tab_bar_avail_px();
                self.with_project(project_id, move |ws, io| {
                    ws.on_tab_attached(TabAttachedArgs {
                        cols: io.cols,
                        rows: io.rows,
                        tab_id,
                        info,
                        snapshot,
                        picked_agent,
                        term_tab_bar_avail_px,
                    })
                });
            }
            Message::PaneResized {
                cols,
                rows,
                ssh_cols,
                ssh_rows,
            } => self.pane_resized(cols, rows, ssh_cols, ssh_rows),
            Message::ColumnDragStart(divider) => {
                self.dragging = Some(divider);
            }
            Message::ColumnDrag {
                window_width,
                logical_x,
            } => {
                if let Some(divider) = self.dragging {
                    let state = self.shell_state();
                    self.dims = apply_column_drag(state, divider, window_width, logical_x);
                }
            }
            Message::ColumnDragEnd => {
                self.dragging = None;
                self.on_shell_layout_changed();
            }
            Message::RowDragStart(divider) => {
                self.dragging_row = Some(divider);
            }
            Message::RowDrag {
                window_height,
                logical_y,
            } => {
                if let Some(divider) = self.dragging_row {
                    match divider {
                        RowDivider::GitLogFileDiffSplit => {
                            let state = self.shell_state();
                            self.dims = apply_row_drag(state, divider, window_height, logical_y);
                        }
                        // 新增任务框高度:基线 = 框底 = 左面板区底 =
                        // `window_height - footbar_height`(顶栏在 `base`
                        // 之上,不参与);高度 = 基线 - 光标 y,向上拉变高。
                        // 上限再夹一道,避免列表区被压没(留约 140px)。
                        RowDivider::TodoAddGrow => {
                            let baseline =
                                window_height - byteui::theme::geometry::footbar_height();
                            let max_h =
                                (baseline - byteui::theme::geometry::top_bar_height() - 140.0)
                                    .max(todo::ADD_INPUT_MIN_HEIGHT);
                            let h = (baseline - logical_y).clamp(todo::ADD_INPUT_MIN_HEIGHT, max_h);
                            if let Some(ws) = self.active_workspace_mut() {
                                ws.todo.set_add_input_height(h);
                            }
                        }
                    }
                }
            }
            Message::RowDragEnd => {
                self.dragging_row = None;
                self.on_shell_layout_changed();
            }
            Message::TabDragMove { group, index } => {
                self.tab_drag_move(group, index);
            }
            Message::TabDragEnd => {
                self.end_tab_drag();
            }
            Message::RailDragMove { side, index } => {
                self.rail_drag_move(side, index);
            }
            Message::RailDragEnd => {
                self.end_rail_drag();
            }
            Message::TodoDragEnd => {
                self.todo_message(todo::Message::DragEnd);
            }
            Message::PanelSelect(v) => self.panel_select(v),
            Message::ToggleFileTreeCollapse => self.toggle_files_tree_collapse(),
            Message::TogglePanelListCollapse(kind) => self.toggle_panel_list_collapse(kind),
            Message::TextInputMenuOpen(target) => {
                #[cfg(target_os = "macos")]
                {
                    let (x, y) = self.files.last_right_click();
                    let items = text_input_menu_items(&target);
                    if let Some(msg) = crate::chrome::native_menu::show(items, (x, y))
                        && let Some(ch) = crate::platform::window_events::menu_edit_key(&msg)
                    {
                        self.pending_native_menu_edit_key = Some((ch, target.id.clone()));
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    // 与其它右键菜单互斥——关掉别的,只留本菜单(同时避免互相顶)。
                    self.files.close_context_menu();
                    self.project_link_menu = None;
                    let (x, y) = self.files.last_right_click();
                    self.text_input_menu = Some(TextInputMenu {
                        x,
                        y,
                        target: target.clone(),
                    });
                    // 右键不聚焦 iced 输入框(只有左键会),菜单的复制/粘贴需要通过
                    // `interface.operate` 把焦点移到目标输入,否则合成回的 ⌘+c/v
                    // 事件作用不到它。记录待聚焦 id,本帧后由 `apply_pending_focus`
                    // 应用(main.rs)。
                    self.pending_text_input_focus = Some(target.id);
                }
            }
            Message::TextInputMenuClose => {
                self.text_input_menu = None;
            }
            Message::TextInputMenuCut => {
                self.text_input_menu = None;
            }
            Message::TextInputMenuCopy => {
                self.text_input_menu = None;
            }
            Message::TextInputMenuPaste => {
                self.text_input_menu = None;
            }
            Message::TextInputMenuSelectAll => {
                self.text_input_menu = None;
            }
            Message::Hover(id, h) => {
                self.set_hover(id, h);
            }
            Message::MaximizeClose => {
                self.maximized = None;
                self.sync_terminal_grid();
            }
            Message::TopBarDoubleClick => {
                self.pending_zoom_toggle = true;
            }
            Message::TopBarHome => self.top_bar_home(),
            Message::HomeRecentsLoaded(files, convs) => {
                self.home_recent_files = files;
                self.home_recent_conversations = convs;
                self.home_recents_loaded = true;
            }
            Message::HomeLeftIconSelect(v) => {
                self.home_left_view = v;
            }
            Message::HomeMoreProjects => self.home_project_pages += 1,
            Message::HomeProjectSearchInput(s) => self.home_project_search_draft = s,
            Message::HomeProjectSearchSubmit => self.commit_home_project_search(),
            Message::HomeRightIconSelect(v) => {
                self.home_right_view = v;
            }
            Message::HomeBrowser(msg) => {
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::HomeBrowser(m));
                };
                browser::update(&mut self.home_browser, msg, None, &client, &handle, emit);
            }
            Message::BrowserTitle(id, title) => {
                let msg = browser::Message::TitleLoaded(id, title);
                if self.is_home() {
                    // 首页全局浏览器:webview 属于 `home_browser`,直接落地。
                    browser::update(
                        &mut self.home_browser,
                        msg,
                        None,
                        &self.client,
                        &self.handle,
                        |_| {},
                    );
                } else {
                    // 工作区浏览器:webview id 落在当前聚焦工作区的 `ws.browser`。
                    self.with_focused_project(|ws, io| {
                        browser::update(
                            &mut ws.browser,
                            msg,
                            ws.project.as_ref().map(|p| p.id),
                            &io.client,
                            &io.handle,
                            |_| {},
                        );
                    });
                }
            }
            Message::BrowserNavigated(id, url) => {
                let msg = browser::Message::Loaded(id, url);
                if self.is_home() {
                    browser::update(
                        &mut self.home_browser,
                        msg,
                        None,
                        &self.client,
                        &self.handle,
                        |_| {},
                    );
                } else {
                    self.with_focused_project(|ws, io| {
                        browser::update(
                            &mut ws.browser,
                            msg,
                            ws.project.as_ref().map(|p| p.id),
                            &io.client,
                            &io.handle,
                            |_| {},
                        );
                    });
                }
            }
            Message::BrowserNewWindow(url) => {
                let msg = browser::Message::OpenUrl(url);
                if self.is_home() {
                    browser::update(
                        &mut self.home_browser,
                        msg,
                        None,
                        &self.client,
                        &self.handle,
                        |_| {},
                    );
                } else {
                    self.with_focused_project(|ws, io| {
                        browser::update(
                            &mut ws.browser,
                            msg,
                            ws.project.as_ref().map(|p| p.id),
                            &io.client,
                            &io.handle,
                            |_| {},
                        );
                    });
                }
            }
            Message::Noop => {}
            Message::DaemonError(message) => self.daemon_error = Some(message),
            Message::TermScroll(target, delta) => {
                self.with_focused_project(|ws, _io| {
                    let tab = match target {
                        terminal::TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        terminal::TermTarget::SshPanel => ws.ssh_active_tab_mut(),
                    };
                    if let Some(tab) = tab {
                        tab.model.scroll_display(delta);
                    }
                });
            }
            Message::TermTabOverflowToggle => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = match self.active_workspace() {
                        Some(ws) => {
                            let entries: Vec<(usize, String, bool)> = ws
                                .tabs
                                .iter()
                                .enumerate()
                                .map(|(idx, tab)| {
                                    (
                                        idx,
                                        tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name),
                                        idx == ws.active,
                                    )
                                })
                                .collect();
                            tab_widget::tab_overflow_items(&entries, Message::SelectTab)
                        }
                        None => Vec::new(),
                    };
                    if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let last_cursor = self.last_cursor;
                    self.with_focused_project(|ws, _io| {
                        ws.term_tab_overflow_anchor = if ws.term_tab_overflow_anchor.is_some() {
                            None
                        } else {
                            Some(last_cursor)
                        };
                    });
                }
            }
            Message::TermTabOverflowDismiss => {
                self.with_focused_project(|ws, _io| {
                    ws.term_tab_overflow_anchor = None;
                });
            }
            Message::PreviewTabOverflowToggle => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = match self.active_workspace() {
                        Some(ws) => {
                            let entries: Vec<(usize, String, bool)> = ws
                                .preview
                                .tabs()
                                .iter()
                                .enumerate()
                                .map(|(idx, tab)| {
                                    (idx, tab.title.clone(), idx == ws.preview.active_idx())
                                })
                                .collect();
                            tab_widget::tab_overflow_items(&entries, Message::PreviewSelectTab)
                        }
                        None => Vec::new(),
                    };
                    if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let last_cursor = self.last_cursor;
                    self.with_focused_project(|ws, _io| {
                        ws.preview_tab_overflow_anchor = if ws.preview_tab_overflow_anchor.is_some()
                        {
                            None
                        } else {
                            Some(last_cursor)
                        };
                    });
                }
            }
            Message::PreviewTabOverflowDismiss => {
                self.with_focused_project(|ws, _io| {
                    ws.preview_tab_overflow_anchor = None;
                });
            }
            Message::TermSelStart {
                target,
                col,
                row,
                right,
            } => {
                self.with_focused_project(|ws, _io| {
                    let tab = match target {
                        terminal::TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        terminal::TermTarget::SshPanel => ws.ssh_active_tab_mut(),
                    };
                    if let Some(tab) = tab {
                        tab.model.selection_start(col, row, right);
                    }
                });
            }
            Message::TermSelUpdate {
                target,
                col,
                row,
                right,
            } => {
                self.with_focused_project(|ws, _io| {
                    let tab = match target {
                        terminal::TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        terminal::TermTarget::SshPanel => ws.ssh_active_tab_mut(),
                    };
                    if let Some(tab) = tab {
                        tab.model.selection_update(col, row, right);
                    }
                });
            }
            Message::TermPaste(target, text) => self.term_paste(target, text),
            Message::PreviewOpenPath(path) => self.preview_open_path(path),
            Message::PreviewSelectTab(idx) => self.preview_select_tab(idx),
            Message::PreviewCloseTab(idx) => {
                self.with_focused_project(|ws, io| {
                    // 关闭前静默保存该 tab 的就地改动(仅当它是脏的原生 tab 才
                    // 动作;不脏/走 wry 的 tab 内部直接 no-op)——复用
                    // `preview_pane_save_at` 的落盘 + 清脏 + 面板 error 管线,抵掉
                    // 关闭即丢改动。顺序:先 `save_at`(取的是关闭前的下标 + buffer)
                    // 再 `close`。
                    ws.preview_pane_save_at(PanelKind::Files, idx);
                    ws.preview.close(idx);
                    // 关 tab 后位置全变，旧 first 可能越界——归零防御（P1L T5）。
                    ws.preview_tab_first = 0;
                    ws.spawn_preview_state_save(io);
                    ws.spawn_preview_context_push(io);
                });
            }
            Message::PreviewToggleRenderMode(idx) => {
                self.with_focused_project(|ws, _io| {
                    ws.preview_pane_toggle_render_mode(PanelKind::Files, idx);
                });
            }
            Message::PreviewEditorEvent(_tab_id, _action) => {
                // main.rs 直接调 `App::preview_tab_editor_event`,不经过这里。
            }
            Message::PreviewSaveActive(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_pane_save_active(kind));
            }
            Message::PreviewTabInsertTab(kind) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_pane_active_editor_event(
                        kind,
                        iced_widget::text_editor::Action::Edit(
                            iced_widget::text_editor::Edit::Insert('\t'),
                        ),
                    );
                });
            }
            Message::PreviewUndoActive(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_pane_undo_active(kind));
            }
            Message::PreviewRedoActive(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_pane_redo_active(kind));
            }
            Message::PreviewFindOpen(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_open(kind));
            }
            Message::PreviewFindOpenWithReplace(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_open_with_replace(kind));
            }
            Message::PreviewFindReplaceToggle(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_toggle_replace(kind));
            }
            Message::PreviewFindClose(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_close(kind));
            }
            Message::PreviewFindText(kind, query) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_type(kind, query));
            }
            Message::PreviewFindGo(kind, next) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_go(kind, next));
            }
            Message::PreviewFindCase(kind, sensitive) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_case(kind, sensitive));
            }
            Message::PreviewFindReplacement(kind, repl) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_find_set_replacement(kind, repl);
                });
            }
            Message::PreviewFindReplaceCurrent(kind) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_find_replace_current(kind);
                });
            }
            Message::PreviewFindReplaceAll(kind) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_find_replace_all(kind);
                });
            }
            Message::TabularAction(kind, tab_id, action) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_pane_tabular_action(kind, tab_id, action);
                });
            }
            Message::ProjectPreviewOpenPath(path) => self.project_preview_open_path(path),
            Message::ProjectPreviewSelectTab(idx) => self.project_preview_select_tab(idx),
            Message::ProjectPreviewCloseTab(idx) => {
                self.with_focused_project(|ws, _io| {
                    // 关闭前静默保存,语义同 `PreviewCloseTab`(先 `save_at` 再 close)。
                    ws.preview_pane_save_at(PanelKind::Project, idx);
                    ws.project_preview.close(idx);
                    ws.project_preview_tab_first = 0;
                });
            }
            Message::ProjectPreviewToggleRenderMode(idx) => {
                self.with_focused_project(|ws, _io| {
                    ws.preview_pane_toggle_render_mode(PanelKind::Project, idx);
                });
            }
            Message::ProjectPreviewTabOverflowToggle => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = match self.active_workspace() {
                        Some(ws) => {
                            let entries: Vec<(usize, String, bool)> = ws
                                .project_preview
                                .tabs()
                                .iter()
                                .enumerate()
                                .map(|(idx, tab)| {
                                    (
                                        idx,
                                        tab.title.clone(),
                                        idx == ws.project_preview.active_idx(),
                                    )
                                })
                                .collect();
                            tab_widget::tab_overflow_items(
                                &entries,
                                Message::ProjectPreviewSelectTab,
                            )
                        }
                        None => Vec::new(),
                    };
                    if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let last_cursor = self.last_cursor;
                    self.with_focused_project(|ws, _io| {
                        ws.project_preview_tab_overflow_anchor =
                            if ws.project_preview_tab_overflow_anchor.is_some() {
                                None
                            } else {
                                Some(last_cursor)
                            };
                    });
                }
            }
            Message::ProjectPreviewTabOverflowDismiss => {
                self.with_focused_project(|ws, _io| {
                    ws.project_preview_tab_overflow_anchor = None;
                });
            }
            Message::ProjectPreviewEditorEvent(_tab_id, _event) => {
                // 与 `PreviewEditorEvent` 同口径:到达 `App::update` 说明未走
                // main.rs 的 Task 桥接器,直接忽略。
            }
            Message::Browser(browser::Message::BookmarksLoaded(pid, bookmarks)) => {
                self.browser_bookmarks_loaded(pid, bookmarks)
            }
            Message::Browser(browser::Message::BookmarksMutated(pid, res)) => {
                self.browser_bookmarks_mutated(pid, res)
            }
            Message::Browser(browser::Message::DragHover(idx)) => {
                // 浏览器 tab 脱的换位:光标扫过 `idx` 页签 → 走共同换位逻辑。
                self.tab_drag_move(TabGroup::Browser, idx);
            }
            Message::Browser(browser::Message::ColumnDragStart) => {
                self.update(Message::ColumnDragStart(Divider::BrowserBookmarksSplit));
            }
            Message::Browser(browser::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Browser(msg) => self.browser_message(msg),
            Message::ProjectSelect(id) => self.project_select(id),
            Message::ProjectTabPickFolder => {
                // 副作用在 main.rs(rfd 文件夹选择);从新增项目菜单触发时顺带
                // 关掉菜单,同 `project_select` 的处理口径。
                self.project_add_menu_open = false;
            }
            // rfd 弹窗在 main.rs 里同步处理,选中后转成 project::Message::LinkAdd
            // 再回送到这里;这条顶层消息本身不需要 App::update 处理任何东西。
            Message::ProjectLinkPick(_) => {}
            Message::ProjectLinkContextMenuClose => {
                self.project_link_menu = None;
            }
            Message::CategoryContextMenuClose => {
                self.category_context_menu = None;
            }
            Message::CategoryPickerClose => {
                self.category_picker = None;
            }
            Message::CategoryPickerSelect(chosen) => {
                let Some(picker) = self.category_picker.take() else {
                    return;
                };
                match picker.target {
                    CategoryPickerTarget::Todo(todo_id) => {
                        let client = self.client.clone();
                        let handle = self.handle.clone();
                        let proxy = self.proxy.clone();
                        handle.spawn(async move {
                            let res = client
                                .set_todo_category(todo_id, chosen)
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string());
                            let _ = proxy
                                .send_event(Message::Todo(todo::Message::CategoryMutated(res)));
                        });
                    }
                    CategoryPickerTarget::Category(category_id) => {
                        let client = self.client.clone();
                        let handle = self.handle.clone();
                        let proxy = self.proxy.clone();
                        handle.spawn(async move {
                            let res = client
                                .reparent_category(category_id, chosen)
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string());
                            let _ = proxy
                                .send_event(Message::Todo(todo::Message::CategoryMutated(res)));
                        });
                    }
                }
            }
            Message::DatabaseSourceContextMenuClose => {
                self.database_source_menu = None;
            }
            Message::ProjectTabOpen(path) => {
                let client = self.client.clone();
                let proxy = self.proxy.clone();
                let path_s = path.to_string_lossy().into_owned();
                self.handle.spawn(async move {
                    let opened = client.open_project(&path_s).await.ok().flatten();
                    let recent = client.list_projects().await.unwrap_or_default();
                    let _ = proxy.send_event(Message::ProjectTabOpened(opened, recent, false));
                });
            }
            Message::ProjectTabOpened(project, recent, skip_git_init) => {
                self.project_tab_opened(project, recent, skip_git_init)
            }
            Message::ProjectTabSwitch(id) => self.project_tab_switch(id),
            Message::ProjectTabClose(id) => self.project_tab_close(id),
            Message::ProjectSlotLoaded(id, payload) => self.project_slot_loaded(id, payload),
            Message::ProjectFsChanged(project_id, changes) => {
                self.project_fs_changed(project_id, changes)
            }
            Message::GitLog(git_log::Message::ColumnDragStart) => {
                // Git Log 三栏布局里左右分割线开始拖拽——扩展发不了 app 级
                // 拖拽消息,由内核代发。
                self.update(Message::ColumnDragStart(Divider::GitLogSplit));
            }
            Message::GitLog(git_log::Message::RowDragStart) => {
                self.update(Message::RowDragStart(RowDivider::GitLogFileDiffSplit));
            }
            Message::GitLog(git_log::Message::BranchPickerOpen) => {
                // 先把"展开"这个状态位落地(纯状态机部分仍走 update,不跳过),
                // 首次展开且还没缓存过分支列表时,顺带异步查一次本地分支。
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::GitLog(m));
                };
                let needs_fetch = self.git_log.branches_is_empty();
                git_log::update(
                    &mut self.git_log,
                    git_log::Message::BranchPickerOpen,
                    &handle,
                    emit.clone(),
                );
                if needs_fetch
                    && let Some(repo_path) = self
                        .active_workspace()
                        .and_then(|ws| ws.active_project_path())
                {
                    self.handle.spawn(async move {
                        let repo_path2 = repo_path.clone();
                        let (branches, dirty) = tokio::task::spawn_blocking(move || {
                            let branches =
                                crate::delivery::local_branches(&repo_path2).unwrap_or_default();
                            let dirty = crate::delivery::is_dirty(&repo_path2);
                            (branches, dirty)
                        })
                        .await
                        .unwrap_or_default();
                        emit(git_log::Message::BranchesLoaded(repo_path, branches, dirty));
                    });
                }
            }
            Message::GitLog(git_log::Message::BranchSwitch(name)) => {
                let Some(repo_path) = self
                    .active_workspace()
                    .and_then(|ws| ws.active_project_path())
                else {
                    return;
                };
                self.git_log.set_branch_switch_pending(true);
                let proxy = self.proxy.clone();
                self.handle.spawn(async move {
                    let repo_path2 = repo_path.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        crate::delivery::checkout_branch(&repo_path2, &name)
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let _ = proxy
                        .send_event(Message::GitLog(git_log::Message::BranchSwitchDone(result)));
                });
            }
            Message::GitLog(git_log::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::GitLog(msg) => {
                if let git_log::Message::Hover(id, h) = msg {
                    self.set_hover(id, h);
                    return;
                }
                // 分支切换成功后(checkout 改了 HEAD/工作区),commit 列表要重拉。
                let is_branch_switch_success =
                    matches!(&msg, git_log::Message::BranchSwitchDone(Ok(())));
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::GitLog(m));
                };
                if let Some(next) = git_log::update(&mut self.git_log, msg, &handle, emit.clone()) {
                    self.update(Message::GitLog(next));
                }
                if is_branch_switch_success
                    && let Some(repo_path) = self
                        .active_workspace()
                        .and_then(|ws| ws.active_project_path())
                {
                    let max_count = self.git_log.cache_max_count();
                    git_log::request_refresh(
                        &mut self.git_log,
                        repo_path,
                        max_count,
                        &handle,
                        emit,
                    );
                }
            }
            Message::FileHistory(msg) => {
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::FileHistory(m));
                };
                file_history::update(&mut self.file_history, msg, &handle, emit);
            }
            Message::ProjectCreateOpen => {
                self.project_create = Some(project_create::State::default());
            }
            Message::ProjectCreate(project_create::Message::GoToSettings) => {
                self.project_create = None;
                self.settings = Some(settings::State::load(self.daemon_error.as_deref()));
            }
            Message::ProjectCreate(project_create::Message::Done(Ok((
                project,
                recent,
                skip_git_init,
            )))) => {
                self.project_create = None;
                self.project_tab_opened(project, recent, skip_git_init);
            }
            // 失败分支故意不在这里拦截:`project_create::update` 自己的
            // `Done` 处理会把错误填进 `State::error` 并保持对话框打开
            // (模块文档已经这么承诺),所以让它落进下面的泛化转发分支
            // 走那条路径,不在内核这里重复处理、更不能整体关掉对话框——
            // 那样会把用户已经填好的表单内容(根目录/名称/描述)连同错误
            // 一起丢掉(代码评审 finding:Failed clone/create always
            // discards the dialog)。
            Message::ProjectCreate(msg) => {
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::ProjectCreate(m));
                };
                project_create::update(&mut self.project_create, msg, &client, &handle, emit);
            }
            Message::Files(files::Message::CopyPath(path, kind)) => {
                let _ = (path, kind); // main.rs 拦截处理写剪贴板,这里维持现状空分支
            }
            Message::Files(files::Message::OpenSearch(path, is_dir)) => {
                // 右键菜单"搜索":跨 `files::Message` 边界,由内核把它映射成
                // `search::Message::SearchOpen`。先关右键菜单(否则搜索弹窗
                // dismiss 一关,旧菜单又冒回来),作用域由 `is_dir` 决定——目录
                // 按目录递归搜,文件只搜单文件。
                self.files.close_context_menu();
                let scope = if is_dir {
                    search::Scope::Dir(path)
                } else {
                    search::Scope::File(path)
                };
                self.update(Message::Search(search::Message::SearchOpen(scope)));
            }
            Message::Files(files::Message::FileHistoryOpen(path)) => {
                // 右键"查看此文件历史":先收起右键菜单(同 OpenSearch 的既有
                // 约定),再解析出仓库相对路径、组出 `FileHistoryTarget`、
                // 异步跑一次 `build()`。
                self.files.close_context_menu();
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let repo_path = PathBuf::from(&project.path);
                let Ok(file_path) = path.strip_prefix(&repo_path).map(|p| p.to_path_buf()) else {
                    return;
                };
                let target = file_history::FileHistoryTarget {
                    project_id,
                    repo_path: repo_path.clone(),
                    file_path: file_path.clone(),
                };
                self.file_history = Some(file_history::State::new(target));
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                handle.spawn(async move {
                    let repo_path2 = repo_path.clone();
                    let file_path2 = file_path.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        file_history::build(
                            &repo_path2,
                            &file_path2,
                            file_history::DEFAULT_MAX_COUNT,
                        )
                    })
                    .await
                    .unwrap_or_else(|e| Err(format!("加载失败: {e}")));
                    let _ = proxy.send_event(Message::FileHistory(
                        file_history::Message::SnapshotLoaded(repo_path, file_path, result),
                    ));
                });
            }
            Message::Files(files::Message::FileHistoryRollbackPrevious(path)) => {
                // 右键"回滚到上一版本":先收起右键菜单,再解析出仓库相对路径,
                // 算上一版本 Oid 并 `rollback_to` 还原(同 `FileHistoryOpen` 在
                // app 层接线的写法,但这里是"一键还原"而非"打开历史浮层")。
                self.files.close_context_menu();
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let repo_path = PathBuf::from(&project.path);
                let Ok(file_path) = path.strip_prefix(&repo_path).map(|p| p.to_path_buf()) else {
                    return;
                };
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                handle.spawn(async move {
                    let repo_path2 = repo_path.clone();
                    let file_path2 = file_path.clone();
                    let path2 = path.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        file_history::previous_oid(&repo_path2, &file_path2).and_then(|opt| {
                            match opt {
                                Some(oid) => {
                                    file_history::rollback_to(&repo_path2, &file_path2, oid)
                                }
                                None => Err("该文件没有可回滚的历史版本".to_string()),
                            }
                        })
                    })
                    .await
                    .unwrap_or_else(|e| Err(format!("回滚任务失败: {e}")));
                    let _ = proxy.send_event(Message::Files(
                        files::Message::FileHistoryRollbackDone(project_id, path2, result),
                    ));
                });
            }
            Message::Files(
                msg @ (files::Message::StatusesRefreshed(project_id, ..)
                | files::Message::PasteDone(project_id, ..)
                | files::Message::OpDone { project_id, .. }
                | files::Message::GitInfoLoaded(project_id, ..)
                | files::Message::BranchSwitchDone(project_id, ..)
                | files::Message::GitInitDone(project_id, ..)
                | files::Message::FileDropDone(project_id, ..)),
            ) => self.files_project_message(project_id, msg),

            Message::Files(files::Message::ToolbarHover(target, hovered)) => {
                // 文件树工具行 icon 按钮的 hover:本面板不挂 App 的 hover 动画
                // 表,把进入/离开转发成 `HoverId` 由内核统一驱动动画进度。
                let id = match target {
                    files::FilesToolbarTarget::SearchSubmit => HoverId::FilesSearchSubmit,
                    files::FilesToolbarTarget::Dotfiles => HoverId::FilesDotfiles,
                    files::FilesToolbarTarget::BranchSwitch => HoverId::FilesBranchSwitch,
                };
                self.set_hover(id, hovered);
            }
            Message::Files(files::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            // 树内行被按下:只武装拖拽(供 main.rs 全局松开左键时收尾),
            // **不**立即执行任何点击语义(既不展开/折叠目录,也不打开文件)。
            // 两者都推迟到真正松开、且确认这其实只是一次单击(未越过拖拽
            // 确认阈值)时才在 `TreeDragEnd` 里补做——展开目录会让下方行
            // 布局位移,若按下就立即展开,静止不动的光标可能被 iced 判定成
            // 树内行被按下:只武装 `Pending`(见 `TreeDragPhase` 文档)——
            // `Pending` 期间完全没有任何反应,不挂 `on_move`、不展开目录、
            // 不打开文件。真正推进到 `Dragging`(越过距离+时长两道阈值)由
            // `maybe_confirm_tree_drag` 在每次 `CursorMoved` 时判断(main.rs
            // 调用),不在这里做。
            Message::Files(files::Message::TreeRowPress { path, is_dir }) => {
                let press_pos = self.last_cursor;
                if let Some(ws) = self.active_workspace_mut() {
                    ws.files
                        .arm_tree_drag(path, is_dir, press_pos, std::time::Instant::now());
                }
            }
            // 树内拖拽松开左键:main.rs 发这条。`confirmed` 就是"这场拖拽有
            // 没有走到 `Dragging` 阶段"——由 `maybe_confirm_tree_drag` 在
            // 越过阈值那一刻就已经推进过一次,这里直接读结果,不重新算
            // 距离/时长(2026-09 用户实测反馈带诊断日志实锤过纯距离阈值挡
            // 不住 trackpad 快速点按的真实位移,才改成阈值判断只在
            // `Pending → Dragging` 转换时做一次、结果落进状态机里的这个
            // 设计,见 `TreeDragPhase` 文档)。
            Message::Files(files::Message::TreeDragRelease) => {
                let confirmed = self
                    .active_workspace()
                    .is_some_and(|ws| ws.files.tree_drag_confirmed());
                self.update(Message::Files(files::Message::TreeDragEnd(confirmed)));
            }
            // 树行双击:目录复用 `Message::Toggle` 那条本地消息直接切换展开
            // 态(同点箭头效果),文件跨到 `PreviewOpenPath`——该面板本身
            // 不认识这条消息,见 `files::Message::TreeRowDoubleClick` 文档。
            Message::Files(files::Message::TreeRowDoubleClick { path, is_dir }) => {
                if is_dir {
                    self.update(Message::Files(files::Message::Toggle(path)));
                } else {
                    self.update(Message::PreviewOpenPath(path));
                }
            }
            Message::Files(msg) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Files(m));
                };
                let app_files = &mut self.files;
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                files::update(&mut ws.files, app_files, msg, project_id, &handle, emit);
            }
            Message::Project(
                msg @ (project::Message::GitRefreshed(project_id, ..)
                | project::Message::NameRenamed(project_id, ..)
                | project::Message::DiskUsageLoaded(project_id, ..)
                | project::Message::ScaffoldStepStarted(project_id, ..)
                | project::Message::ScaffoldStepFinished(project_id, ..)
                | project::Message::TranscriptBackfillStarted(project_id, ..)
                | project::Message::TranscriptBackfillFinished(project_id, ..)
                | project::Message::SummaryBackfillProgress(project_id, ..)),
            ) => {
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let current_name = project.name.clone();
                let repo_path = std::path::PathBuf::from(&project.path);
                // `NameRenamed(Ok(updated))` 要把顶栏项目页签等读的 `ws.project`
                // 缓存一并更新——这是这个面板第一次出现需要内核介入(而不是纯
                // 委托给 `project::update`)的消息。
                if let project::Message::NameRenamed(_, Ok(updated)) = &msg {
                    ws.project = Some(updated.clone());
                }
                // 补总结进度追到 Done 时,弹窗外面缓存的 `conversation_sessions`
                // (`spawn_conversations_refresh` 唯一写入点)不会自动感知
                // `session_summaries` 表的新增行——不重新拉一次,对话列表会一直
                // 显示"未总结"直到用户重开项目 tab 或触发别的回合结束刷新。
                let refresh_conversations = matches!(
                    &msg,
                    project::Message::SummaryBackfillProgress(_, completed, total)
                        if completed >= total
                );
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Project(m));
                };
                project::update(
                    &mut ws.project_panel,
                    msg,
                    project_id,
                    &current_name,
                    &repo_path,
                    &client,
                    &handle,
                    emit,
                );
                if refresh_conversations {
                    self.with_project(project_id, |ws, io| {
                        ws.spawn_conversations_refresh(io);
                    });
                }
            }
            Message::Project(project::Message::OpenLink(path)) => {
                // 项目链接打开的文件进 Project 面板右配对的预览(`ws.project_preview`),
                // 不冲进 Files 预览——两条预览各自独立,互相不打扰。
                self.update(Message::ProjectPreviewOpenPath(path));
            }
            Message::Project(project::Message::Pick(target)) => {
                // 文件/目录选择器依赖 macOS 主线程原生能力(rfd/NSOpenPanel 模态,
                // 见 main.rs `pick_file_or_dir`),必须由 main.rs 的 winit 事件循环
                // 里 `dispatch` 拦截同步执行。这里只用代理把这条消息回灌回事件循环
                // ——不能 `self.update(Message::ProjectLinkPick(..))` 直调:那是同步
                // 递归,只会命中 `App::update` 里那格 no-op,绝不会触发文件选择弹窗。
                let _ = self.proxy.send_event(Message::ProjectLinkPick(target));
            }
            Message::Project(project::Message::LinkContextMenu { target, index }) => {
                self.project_link_context_menu(target, index);
            }
            Message::Project(project::Message::LinkRemove { target, index }) => {
                // 删除来自行内右键菜单:落 `LinkRemove` 时把菜单浮层一并收起,
                // 然后委托 `project::update` 真正执行删除(含越界校验与保存失败
                // 回滚,见 `project.rs` 的 `Message::LinkRemove`)。不能
                // `self.update(同一条 LinkRemove)` 直调——那会命中本分支自身
                // 再次匹配 `LinkRemove`,无限递归爆栈。
                self.project_link_menu = None;
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let current_name = project.name.clone();
                let repo_path = std::path::PathBuf::from(&project.path);
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Project(m));
                };
                project::update(
                    &mut ws.project_panel,
                    project::Message::LinkRemove { target, index },
                    project_id,
                    &current_name,
                    &repo_path,
                    &client,
                    &handle,
                    emit,
                );
            }
            Message::Project(project::Message::DeleteProjectConfirm) => {
                self.project_delete_confirm();
            }
            Message::Project(project::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Project(project::Message::ToolbarHover(target, hovered)) => {
                // Project 面板头部"＋"按钮的 hover:本面板不挂 App 的 hover 动画表,
                // 把进入/离开转发成 `HoverId` 由内核统一驱动动画进度(同
                // `Message::Files(files::Message::ToolbarHover(..))` 的既有先例)。
                let id = match target {
                    project::ProjectToolbarTarget::Docs => HoverId::ProjectDocsAdd,
                    project::ProjectToolbarTarget::Memory => HoverId::ProjectMemoryAdd,
                    project::ProjectToolbarTarget::Remote => HoverId::ProjectRemoteAdd,
                };
                self.set_hover(id, hovered);
            }
            Message::Project(msg) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let current_name = project.name.clone();
                let repo_path = std::path::PathBuf::from(&project.path);
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Project(m));
                };
                project::update(
                    &mut ws.project_panel,
                    msg,
                    project_id,
                    &current_name,
                    &repo_path,
                    &client,
                    &handle,
                    emit,
                );
            }
            Message::Ssh(ssh::Message::TestConnectionResult(project_id, host_id, result)) => {
                self.ssh_test_connection_result(project_id, host_id, result)
            }
            Message::Ssh(ssh::Message::UnknownKeyDetected(
                project_id,
                host_id,
                fingerprint,
                key_bytes,
            )) => self.ssh_unknown_key_detected(project_id, host_id, fingerprint, key_bytes),
            Message::Ssh(ssh::Message::KeyChanged(project_id, host_id, fingerprint)) => {
                self.ssh_key_changed(project_id, host_id, fingerprint)
            }
            // 点"终端"按钮:与既有 `TestConnection`/其它同步交互消息不同,
            // 这个消息不走 `ssh::update`(它要新建一个 tab,需要 `&mut
            // Workspace` 整体,`ssh::update` 只拿得到 `&mut ws.ssh`)——
            // 拦截在通配 `Message::Ssh(msg)` 之前,直接调 `Workspace::
            // spawn_ssh_tab`。
            Message::Ssh(ssh::Message::OpenSshTab(host_id, ssh::SshTabKind::Terminal)) => {
                // 新 SSH 终端从一开始就用 SSH 面板自己的网格(不是共享终端的
                // 列数)——在闭包里再借 `self` 会与 `with_focused_project` 的
                // `&mut self` 冲突,先取到局变量。
                let ssh_cols = self.ssh_cols;
                let ssh_rows = self.ssh_rows;
                self.with_focused_project(|ws, io| {
                    // 已经开着这台主机的终端 tab 就直接切过去,不重新握手
                    // 连一遍(阶段 3 SFTP 决定"每个 tab 独立新建连接",但
                    // 终端 tab 本来就是"一台主机一条常驻连接",重复点
                    // "终端"图标应该是切换焦点而不是叠加新连接)。
                    let already_open = ws
                        .ssh_tabs
                        .iter()
                        .any(|t| t.info.id.strip_prefix("ssh:") == Some(host_id.as_str()));
                    if already_open {
                        ws.select_ssh_tab(host_id, ssh::SshTabKind::Terminal);
                    } else {
                        ws.ssh.record_reopen_after_trust(host_id.clone());
                        ws.spawn_ssh_tab(io, host_id, ssh_cols, ssh_rows);
                    }
                });
            }
            // Sftp 阶段 3:真实打开一个 SFTP tab(独立连接 + 命令通道)。
            Message::Ssh(ssh::Message::OpenSshTab(host_id, ssh::SshTabKind::Sftp)) => {
                self.with_focused_project(|ws, io| {
                    if ws.sftp_tabs.contains_key(&host_id) {
                        ws.select_ssh_tab(host_id, ssh::SshTabKind::Sftp);
                    } else {
                        ws.spawn_sftp_tab(io, host_id.clone());
                        ws.select_ssh_tab(host_id, ssh::SshTabKind::Sftp);
                    }
                });
            }
            Message::Ssh(ssh::Message::CloseSshTab(host_id, kind)) => {
                self.with_focused_project(|ws, io| match kind {
                    ssh::SshTabKind::Terminal => ws.close_ssh_tab(io, &host_id, kind),
                    ssh::SshTabKind::Sftp => {
                        ws.sftp_tabs.remove(&host_id);
                        if ws.ssh_active.as_ref().map(|(h, k)| (h.as_str(), *k))
                            == Some((host_id.as_str(), ssh::SshTabKind::Sftp))
                        {
                            ws.ssh_active = None; // 简化处理:关掉 SFTP tab 后不自动
                            // 切到其它 tab,和终端 tab 关闭后的
                            // "切到剩下第一个"逻辑不强行统一,
                            // 因为 ssh_tabs/sftp_tabs 是两个不同
                            // 集合,统一切换逻辑收益不大,YAGNI。
                        }
                    }
                });
            }
            Message::Ssh(ssh::Message::SelectSshTab(host_id, kind)) => {
                self.with_focused_project(|ws, _io| {
                    ws.select_ssh_tab(host_id, kind);
                    let mut widths: Vec<f32> = vec![tab_display_width("空白")];
                    widths.extend(ws.ssh_tabs.iter().map(|t| {
                        tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name))
                    }));
                    widths.extend(ws.sftp_tabs.keys().map(|host_id| {
                        tab_display_width(
                            &ws.ssh
                                .hosts()
                                .iter()
                                .find(|h| &h.id == host_id)
                                .map(|h| h.name.clone())
                                .unwrap_or_else(|| host_id.clone()),
                        )
                    }));
                    let target = match &ws.ssh_active {
                        None => 0,
                        Some((active_host, ssh::SshTabKind::Terminal)) => ws
                            .ssh_tabs
                            .iter()
                            .position(|t| {
                                t.info.id.strip_prefix("ssh:") == Some(active_host.as_str())
                            })
                            .map(|i| i + 1)
                            .unwrap_or(0),
                        Some((active_host, ssh::SshTabKind::Sftp)) => ws
                            .sftp_tabs
                            .keys()
                            .position(|h| h == active_host)
                            .map(|i| i + 1 + ws.ssh_tabs.len())
                            .unwrap_or(0),
                    };
                    ws.ssh_tab_first = tab_widget::tab_window_reveal(
                        &widths,
                        4.0,
                        byteui::theme::geometry::tab_bar_avail_px(),
                        ws.ssh_tab_first,
                        target,
                    );
                    ws.ssh_tab_overflow_anchor = None;
                });
            }
            // 点固定的"空白"占位 tab:它不对应 `ssh_tabs`/`sftp_tabs` 里
            // 任何一条记录,选中态就是 `ssh_active == None`。
            Message::Ssh(ssh::Message::SelectBlankTab) => {
                self.with_focused_project(|ws, _io| {
                    ws.ssh_active = None;
                    let mut widths: Vec<f32> = vec![tab_display_width("空白")];
                    widths.extend(ws.ssh_tabs.iter().map(|t| {
                        tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name))
                    }));
                    widths.extend(ws.sftp_tabs.keys().map(|host_id| {
                        tab_display_width(
                            &ws.ssh
                                .hosts()
                                .iter()
                                .find(|h| &h.id == host_id)
                                .map(|h| h.name.clone())
                                .unwrap_or_else(|| host_id.clone()),
                        )
                    }));
                    ws.ssh_tab_first = tab_widget::tab_window_reveal(
                        &widths,
                        4.0,
                        byteui::theme::geometry::tab_bar_avail_px(),
                        ws.ssh_tab_first,
                        0,
                    );
                    ws.ssh_tab_overflow_anchor = None;
                });
            }
            Message::Ssh(ssh::Message::TabOverflowToggle) => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = match self.active_workspace() {
                        Some(ws) => {
                            let mut entries: Vec<(usize, String, bool)> = Vec::new();
                            for (i, tab) in ws.ssh_tabs.iter().enumerate() {
                                let idx = i + 1;
                                let host_id = tab
                                    .info
                                    .id
                                    .strip_prefix("ssh:")
                                    .unwrap_or(&tab.info.id)
                                    .to_string();
                                entries.push((
                                    idx,
                                    tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name),
                                    ws.ssh_active.as_ref().is_some_and(|(h, k)| {
                                        h == &host_id && *k == ssh::SshTabKind::Terminal
                                    }),
                                ));
                            }
                            for (i, (host_id, _)) in ws.sftp_tabs.iter().enumerate() {
                                let idx = i + 1 + ws.ssh_tabs.len();
                                let label = ws
                                    .ssh
                                    .hosts()
                                    .iter()
                                    .find(|h| &h.id == host_id)
                                    .map(|h| h.name.clone())
                                    .unwrap_or_else(|| host_id.clone());
                                entries.push((
                                    idx,
                                    label,
                                    ws.ssh_active.as_ref()
                                        == Some(&(host_id.clone(), ssh::SshTabKind::Sftp)),
                                ));
                            }
                            tab_widget::tab_overflow_items(&entries, |idx| {
                                ssh_tab_overflow_select_message(ws, idx)
                            })
                        }
                        None => Vec::new(),
                    };
                    if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let last_cursor = self.last_cursor;
                    self.with_focused_project(|ws, _io| {
                        ws.ssh_tab_overflow_anchor = if ws.ssh_tab_overflow_anchor.is_some() {
                            None
                        } else {
                            Some(last_cursor)
                        };
                    });
                }
            }
            Message::Ssh(ssh::Message::TabOverflowDismiss) => {
                self.with_focused_project(|ws, _io| {
                    ws.ssh_tab_overflow_anchor = None;
                });
            }
            // SFTP tab 内部交互:按 host_id 路由到 `sftp::route`,真正的
            // 处理逻辑在那边(sftp::Message 有 7+ 个变体,内容又都操作
            // `ws.sftp_tabs`,摊平会让这里的大 match 更难读)。
            Message::Ssh(ssh::Message::Sftp(ssh::sftp::Message::ContextMenuOpen {
                host_id,
                is_local,
                path,
            })) => {
                #[cfg(target_os = "macos")]
                {
                    // 先落选中态(同 `sftp::route` 的 ContextMenuOpen 分支),
                    // 再同步弹原生菜单,结果经 `Ssh(Sftp(msg))` 回路由。
                    self.with_focused_project(|ws, io| {
                        ssh::sftp::route(
                            ws,
                            io,
                            ssh::sftp::Message::ContextMenuOpen {
                                host_id: host_id.clone(),
                                is_local,
                                path: path.clone(),
                            },
                        );
                    });
                    let (x, y) = self.files.last_right_click();
                    let items = ssh::sftp::context_menu_items(&host_id, is_local);
                    if let Some(msg) = crate::chrome::native_menu::show(items, (x, y)) {
                        self.update(Message::Ssh(ssh::Message::Sftp(msg)));
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.with_focused_project(|ws, io| {
                        ssh::sftp::route(
                            ws,
                            io,
                            ssh::sftp::Message::ContextMenuOpen {
                                host_id,
                                is_local,
                                path,
                            },
                        );
                    });
                }
            }
            Message::Ssh(ssh::Message::Sftp(msg)) => {
                self.with_focused_project(|ws, io| {
                    ssh::sftp::route(ws, io, msg);
                });
            }
            // 终端连接失败:先做内核层面的清理(pending/ssh_out_pending
            // 两处暂存——这次连接没能走到 `TabAttached`,不清理会一直占着
            // 这两个 map 的位置),再转给 `ssh::update` 落卡片状态(同
            // `TestConnectionResult` 的路由口径,带显式 project_id,套用
            // 一模一样的 `with_project` 外壳)。
            Message::Ssh(ssh::Message::TerminalConnectFailed(project_id, host_id, tab_id, err)) => {
                self.ssh_terminal_connect_failed(project_id, host_id, tab_id, err)
            }
            Message::Ssh(ssh::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Ssh(msg) => {
                if let ssh::Message::Hover(id, h) = msg {
                    self.set_hover(id, h);
                    return;
                }
                self.with_focused_project(|ws, io| {
                    let Some(project) = ws.project.as_ref() else {
                        return;
                    };
                    let project_id = project.id;
                    let repo_path = PathBuf::from(&project.path);
                    let handle = io.handle.clone();
                    let proxy = io.proxy.clone();
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Ssh(m));
                    };
                    ssh::update(&mut ws.ssh, msg, project_id, &repo_path, &handle, emit);
                });
            }
            Message::Footbar(msg) => {
                // App 级 + 纯展示,不带 project_id,不需要
                // `with_project`/`with_focused_project`,直接更新。
                footbar::update(&mut self.footbar, msg);
            }
            Message::ZoomIn => {
                byteui::theme::icon_size::zoom_by(UI_ZOOM_STEP);
                byteui::theme::icon_size::persist_scale(&crate::theme::ui_scale_path());
                self.sync_terminal_grid();
                self.pending_preview_zoom = true;
            }
            Message::ZoomOut => {
                byteui::theme::icon_size::zoom_by(1.0 / UI_ZOOM_STEP);
                byteui::theme::icon_size::persist_scale(&crate::theme::ui_scale_path());
                self.sync_terminal_grid();
                self.pending_preview_zoom = true;
            }
            Message::ZoomReset => {
                byteui::theme::icon_size::reset_scale(&crate::theme::ui_scale_path());
                self.sync_terminal_grid();
                self.pending_preview_zoom = true;
            }
            Message::SettingsOpen => {
                self.settings = Some(settings::State::load(self.daemon_error.as_deref()));
            }
            Message::Settings(msg) => {
                // 主题切换要在设置 update(它会 set_scheme 改全局配色)之后,把
                // 所有已开预览 webview 按新主题重载——flyfish 的 theme 是 URL
                // 参数(见 preview::flyfish_url),不重载不会跟着变。两个预览
                // 面板(Files/Project)各推进各自 wry tab 的 reload_nonce。
                let theme_changed = matches!(msg, settings::Message::ThemeSelected(_));
                // 停止/重新启动 dozerd 的结果要顺带更新 `daemon_error`——
                // 这是"daemon 连不上"的整程序共享状态(`app/view.rs`/
                // `term/terminal.rs` 已经在读),不新建 UI 组件(spec「复用
                // App.daemon_error」)。跟 `theme_changed` 一样,要在
                // `msg` 被 move 进 `settings::update` 之前取值。
                let stop_succeeded = matches!(msg, settings::Message::AdvancedStopResult(Ok(())));
                let restart_succeeded =
                    matches!(msg, settings::Message::AdvancedRestartResult(Ok(())));
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Settings(m));
                };
                settings::update(&mut self.settings, msg, &client, &handle, emit);
                if stop_succeeded {
                    self.daemon_error = Some("dozerd 已停止,部分功能不可用".to_string());
                }
                if restart_succeeded {
                    self.daemon_error = None;
                }
                if theme_changed && let Some(ws) = self.active_workspace_mut() {
                    ws.preview.reload_all_webviews_for_theme();
                    ws.project_preview.reload_all_webviews_for_theme();
                }
            }
            // WebViewFocused 只在 main.rs 的 dispatch 里设 pending_focus,
            // App::update 无需处理。
            Message::WebViewFocused => {}
            // 鼠标在子 webview 上松开(见 `WebViewMouseUp` 文档):一并结束页签
            // 拖拽,避免"松开还能继续拖"。
            Message::WebViewMouseUp => self.end_tab_drag(),
        }
    }

    pub(crate) fn project_select(&mut self, id: i64) {
        // 从新增项目菜单点选时顺带关掉菜单(菜单本来就该在选中后消失);
        // 从其它入口(首页最近项目卡片)调用时这里恒为 false,no-op。
        self.project_add_menu_open = false;
        // 切项目不再通知 daemon:"活跃项目"是 GUI 侧的概念了(P2a
        // Task 1-3 删掉了 SetActiveProject)。
        //
        // 这个项目已经开着页签(`Loaded` 或还没促成的 `Stub`)时,点最近
        // 项目卡片就只是"切到那个页签",走与点页签完全相同的非破坏性
        // 路径——绝不能杀掉任何已有页签的会话(设计文档 §2)。
        // 切走前先把当前(老)项目的面板布局原样存下,再换成新项目的。
        self.stash_active_panel_layout();
        if focus_project_tab(&self.projects, &mut self.active_project_id, id) {
            self.adopt_panel_layout(id);
            self.maximized = None;
            self.current_page = AppPage::Workspace;
            self.ensure_loaded(id);
            // 清放大态改变了终端 pane 的像素尺寸,网格必须跟着重算:
            // `terminal_grid_state` 把 `maximized` 算进去,不重算的话
            // PTY 会一直停在放大时的 cols/rows,直到某个无关的几何事件
            // 偶然触发一次重算(最终审查 Required Fix #2)。
            self.sync_terminal_grid();
            self.persist_open_projects();
            return;
        }
        // 还没开着:作为**新页签**打开(与顶栏"＋"同一条 `ProjectTabOpened`
        // 落地路径),而不是把当前页签的内容换掉——多页签下"点一张最近
        // 项目卡片"的直觉是"再开一个",不是"把手上这个换掉"。
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        self.handle.spawn(async move {
            let recent = client.list_projects().await.unwrap_or_default();
            let opened = recent.iter().find(|p| p.id == id).cloned();
            let _ = proxy.send_event(Message::ProjectTabOpened(opened, recent, false));
        });
    }

    pub(crate) fn project_tab_opened(
        &mut self,
        project: Option<ProjectInfo>,
        recent: Vec<ProjectInfo>,
        skip_git_init: bool,
    ) {
        self.recent_projects = recent.clone();
        // `None` = 这次打开失败(daemon 不通/回 `Reply::Error`)。硬性
        // 要求:失败绝不能落进任何 `Workspace`,否则会留下"有界面、没
        // 归属项目"的破状态,用户一点 tab 栏的"＋"就 panic
        // (`spawn_new_tab` 的 expect)。失败文案挂到 App 级的
        // `daemon_error` 上——它不依赖任何 `Workspace` 存在,一个项目
        // 都没打开时空态视图也画得出来(Required Fix #1)。
        let Some(project) = project else {
            tracing::warn!("打开项目页签失败,页签集合保持不变");
            self.daemon_error = Some("打开项目失败,请确认 dozerd 正常后重试".to_string());
            self.with_focused_project(move |ws, _io| {
                ws.recent_projects = recent;
            });
            return;
        };
        self.daemon_error = None;
        // 放大态是外壳态,换页签后留着只会挡住新页签的界面。
        self.maximized = None;
        let id = project.id;
        // 切走前先把当前(老)项目的面板布局原样存下,再换成新项目的。
        self.stash_active_panel_layout();
        if focus_project_tab(&self.projects, &mut self.active_project_id, id) {
            self.adopt_panel_layout(id);
            // 这个项目已经开着页签了:只前台化,绝不改写它的内容——
            // 那会把这个页签既有的终端全关掉、文件树对话列表全清空重来。
            self.current_page = AppPage::Workspace;
            self.ensure_loaded(id);
            self.with_focused_project(move |ws, _io| {
                ws.recent_projects = recent;
            });
            self.sync_terminal_grid(); // 清放大态后重算网格,理由见 `ProjectSelect`
            self.persist_open_projects();
            return;
        }
        let io = self.shell_io();
        let repo_path = PathBuf::from(&project.path);
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.recent_projects = recent;
        ws.adopt_project(&io, project);
        self.projects
            .insert(id, WorkspaceSlot::Loaded(Box::new(ws)));
        self.project_order.push(id);
        self.active_project_id = Some(id);
        // 换成新项目的面板布局(它自己没存过就退化成默认)。
        self.adopt_panel_layout(id);
        self.current_page = AppPage::Workspace;
        self.sync_terminal_grid(); // 同上
        self.persist_open_projects();
        // 新开的项目 tab(区别于"已开着、只是前台化"那条 `focus_project_tab`
        // 早退分支):静默跑一次 ensure(README/.dozer/git/agent 历史),不
        // 展示结果(见 project_scaffold 设计"打开即 ensure"一节)。
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Project(m));
        };
        project::spawn_scaffold_run(repo_path, client, &handle, emit, skip_git_init);
    }

    pub(crate) fn project_tab_switch(&mut self, id: i64) {
        // 切页签只有两件事:改 `active_project_id`、必要时促成 `Stub`。
        // 没有任何内容改写,因此后台项目的终端/预览/审阅原样留着,切
        // 回来还是刚才那副样子。
        // 切走前先把当前(老)项目的面板布局原样存下,再换成新项目的。
        self.stash_active_panel_layout();
        if !focus_project_tab(&self.projects, &mut self.active_project_id, id) {
            return;
        }
        self.adopt_panel_layout(id);
        self.maximized = None;
        self.current_page = AppPage::Workspace;
        self.ensure_loaded(id);
        // Git Log 面板已经开着的话,提交图缓存是 `App` 级的、不随项目
        // 页签走(见 `sync_git_log_to_active_project` 文档),不补这一
        // 下切页签会让提交图停在上一个项目,跟同一面板里已经按新项目
        // 刷新的 worktree 速览条对不上。
        if self.left_view == PanelKind::GitLog {
            self.sync_git_log_to_active_project();
        }
        // 清放大态后必须重算终端网格。`PaneResized` 那条分支只在**窗口
        // 几何变化**时触发,清 `maximized` 不会自己走到那里;而
        // `terminal_grid_state` 把 `maximized` 算进公式,不重算的话
        // "在项目 A 放大终端 → 切到 B"会让 A 的 PTY 停在放大时的
        // cols/rows(最终审查 Required Fix #2)。重算是幂等的:算出来
        // 与当前 `cols/rows` 相同时 `PaneResized` 的去重会原地返回。
        self.sync_terminal_grid();
        self.persist_open_projects();
        // 按下项目页签＝选中＋准备被拖走(同终端/预览页签)。
        if let Some(idx) = self.project_order.iter().position(|p| *p == id) {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Project,
                source: idx,
                press_pos: self.last_cursor,
            });
        }
    }

    pub(crate) fn project_tab_close(&mut self, id: i64) {
        // 关掉当前页签前先把它的面板布局原样存下(焦点还在它身上,
        // `stash` 会记进 `id` 那份),以后重开还能恢复。
        self.stash_active_panel_layout();
        let io = self.shell_io();
        let Some(slot) = take_project_tab(
            &mut self.projects,
            &mut self.project_order,
            &mut self.active_project_id,
            id,
        ) else {
            return;
        };
        if let WorkspaceSlot::Loaded(mut ws) = slot {
            // 关页签 = 结束该项目下所有会话(abort 转发任务 + kill
            // daemon 侧会话)。不 kill 的话会话会继续在 daemon 上跑,
            // 还会被下次 bootstrap 恢复出来。
            ws.close_all_tabs_for_switch(&io);
        }
        self.maximized = None;
        // 焦点被 `take_project_tab` 挪到了邻居页签上,而那个邻居可能还
        // 是个懒加载 `Stub`——`view()` 走的是只读的 `active_workspace()`,
        // 它**不促成** `Stub`,于是界面会画成"未打开任何项目",尽管顶栏
        // 那个页签明明高亮着。必须在这里显式促成(最终审查 Required
        // Fix #3)。
        if let Some(next) = self.active_project_id {
            // 焦点被挪到了邻居页签,把它的面板布局换上来。
            self.adopt_panel_layout(next);
            self.ensure_loaded(next);
        }
        self.sync_terminal_grid(); // 清放大态后重算网格,理由同 `ProjectTabSwitch`
        self.persist_open_projects();
    }

    /// "删除项目"确认弹窗的"删除"按钮触发,由 `Message::Project(project::
    /// Message::DeleteProjectConfirm)` 拦截调用(见该分支注释)。这个操作
    /// 一定作用在当前聚焦的项目上——删除按钮本来就在那个项目自己的面板
    /// 里,不存在"删除一个没打开的项目"这回事。
    pub(crate) fn project_delete_confirm(&mut self) {
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(scope) = ws.project_panel.delete_pending.take() else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = PathBuf::from(&project.path);
        // 关 tab 必须在发起删除请求之前——删除一旦成功,这个项目在
        // dozerd/磁盘上都可能已经不存在了,`Workspace` 不该继续留着。
        self.project_tab_close(project_id);
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let on_done = move |errors: Vec<String>| {
            let _ = proxy.send_event(Message::ProjectDeleteDone(errors));
        };
        project::delete::spawn_delete_project(
            project_id, repo_path, scope, client, &handle, on_done,
        );
    }

    pub(crate) fn project_slot_loaded(&mut self, id: i64, payload: RestorePayload) {
        let Some(restore) = payload.take() else {
            return; // 信封已被取走(理论上不会发生),没有素材可落地
        };
        // 只在槽位仍是那份"加载中"占位时落地。两种落空情形:
        // - 页签在促成完成前被用户关掉了(槽位已不存在);
        // - 槽位已经被别的路径换成了真正的内容(比如
        //   `ProjectTabOpened` 的 `adopt_project`)。
        // 两种情形下这份素材都没人要了,但它已经 attach 上了该项目在
        // daemon 上的存活会话——直接 drop 只是断开事件流,daemon 侧
        // 会话仍在跑,会变成"没有任何页签持有、却还占着 PTY"的野会话。
        // 所以按关页签的语义结束掉它们(`ProjectTabClose` 同款处理)。
        // 落地的同时把占位那份 `allowed_files` 句柄接过来:main.rs 的
        // webview 池只在 `active_project_id` **变化**时才清空,它看不见
        // "同一个项目换了一份 `Workspace` 对象"。促成窗口期里用户点开
        // 的文件预览已经建出一个 id 0 的 webview,其 `dozer://` 协议
        // 闭包捕获的是**占位那一个** `Arc`;新 `Workspace` 若另起一个
        // `Arc`,`restore_preview_state` 重开的 id 0 会被
        // `sync_webview_pool` 认成"这个 id 已经有 webview 了"而只调
        // `load_url`,于是文件请求走的还是旧 `Arc` 的白名单 → 对不上
        // → 空白预览。这与 Required Fix #3 是同一个失效模式,只是触发
        // 点从"切项目"变成"促成换对象"。共用同一个 `Arc` 即可,而且
        // 不损失已经建好的 webview(比清空池更省一次导航)。
        let inherited = match self.projects.get(&id) {
            Some(WorkspaceSlot::Loaded(cur)) if cur.loading => Some(cur.allowed_files()),
            _ => None,
        };
        let landed = inherited.is_some();
        if !landed {
            let client = self.client.clone();
            let ids: Vec<String> = restore
                .sessions
                .iter()
                .map(|(info, _, _)| info.id.clone())
                .collect();
            let task = self.handle.spawn(async move {
                for sid in ids {
                    if let Err(e) = client.kill(&sid).await {
                        tracing::warn!("丢弃过期促成结果时结束会话失败: {e}");
                    }
                }
            });
            if let Ok(mut pending) = self.pending_exit_tasks.lock() {
                pending.push(task);
            }
            return;
        }
        let io = self.shell_io();
        let mut ws = Workspace::from_restore(&io, *restore, inherited);
        // 重挂出来的会话,终端模型是按 `DEFAULT_COLS`×`DEFAULT_ROWS`
        // 建的,得按当前窗口几何纠正一次。这里**不能**指望
        // `sync_terminal_grid`:它算出来的网格与 `self.cols/rows` 相同
        // 时 `PaneResized` 会原地返回(去重),于是这份新装配的
        // `Workspace` 会一直停在 80×24。直接对它自己 resize 一次——
        // 共享与 SSH 两个 pane 各按自己跟踪的网格分别纠正。
        ws.resize_all(&io, io.cols, io.rows, self.ssh_cols, self.ssh_rows);
        self.projects
            .insert(id, WorkspaceSlot::Loaded(Box::new(ws)));
    }

    pub(crate) fn project_fs_changed(
        &mut self,
        project_id: ProjectId,
        changes: git_watch::FsChanges,
    ) {
        // 工作区类变更:文件树 + 打开的 webview 预览即时跟进。`notify` 递上
        // 的是具体变更路径 `changes.paths`,文件树按"受影响即相关"整棵从盘重
        // 读已缓存目录(`reload_tree_from_disk`,只重读已展开/缓存过的层,开销
        // 小),预览则只重载路径命中的 webview tab。
        if changes.relevance == Some(git_watch::Relevance::Workdir)
            || changes.relevance == Some(git_watch::Relevance::GitRefs)
        {
            self.with_project(project_id, |ws, _io| {
                ws.files.reload_tree_from_disk();
                ws.preview.reload_webviews_for(&changes.paths);
                ws.project_preview.reload_webviews_for(&changes.paths);
            });
        }
        self.with_project(project_id, |ws, io| {
            let Some(project) = &ws.project else { return };
            let repo_path = PathBuf::from(&project.path);
            spawn_project_git_refresh(project_id, repo_path.clone(), io);
            spawn_disk_usage_refresh(project_id, repo_path, io);
        });
        // 只有 `.git` 引用类变化(分支切换/外部提交/其他 worktree
        // 提交)才值得重建 Git Log 快照——纯工作区文件编辑不影响
        // 提交历史,重算是纯浪费。`git_log_cache` 是 `App` 级、不是
        // 按项目分的(见 `sync_git_log_to_active_project`),所以这里
        // 必须先核实这条事件本来就是"当前聚焦项目"发出的
        // (`project_id == self.active_project_id`)——否则后台项目
        // 的引用变化会拿"缓存路径恰好等于前台项目路径"这个巧合当
        // 通行证,把前台正打开的详情/选中态平白清掉,而其实什么都
        // 没变。项目 id 匹配之外再核一次路径,双保险防状态漂移。
        if changes.relevance == Some(git_watch::Relevance::GitRefs)
            && self.active_project_id == Some(project_id)
            && let Some(repo_path) = self.git_log.cache_repo_path().map(|p| p.to_path_buf())
            && self
                .active_workspace()
                .and_then(|ws| ws.active_project_path())
                .as_deref()
                == Some(repo_path.as_path())
        {
            // 引用变化只是要"内容不变、重新拉一遍",窗口大小维持原样——
            // 用 `cache_max_count()`(读当前缓存的 max_count),不是"加载
            // 更多"专用、会 `+LOAD_MORE_STEP` 的 `next_load_more_count()`。
            let max = self.git_log.cache_max_count();
            let handle = self.handle.clone();
            let proxy = self.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::GitLog(m));
            };
            git_log::request_refresh(&mut self.git_log, repo_path, max, &handle, emit);
        }
    }

    pub(crate) fn database_test_connection_result(
        &mut self,
        project_id: i64,
        source_id: String,
        result: Result<(), String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::TestConnectionResult(project_id, source_id, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    pub(crate) fn database_browse_result(
        &mut self,
        project_id: i64,
        tab_id: usize,
        run_seq: u64,
        result: Result<database::BrowsePage, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::BrowseResult(project_id, tab_id, run_seq, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    pub(crate) fn database_query_result(
        &mut self,
        project_id: i64,
        tab_id: usize,
        run_seq: u64,
        result: Result<database::QueryOutcome, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::QueryResult(project_id, tab_id, run_seq, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    pub(crate) fn database_tables_loaded(
        &mut self,
        project_id: i64,
        source_id: String,
        result: Result<Vec<database::TableRef>, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::TablesLoaded(project_id, source_id, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    pub(crate) fn database_columns_loaded(
        &mut self,
        project_id: i64,
        source_id: String,
        schema: Option<String>,
        table: String,
        result: Result<Vec<database::ColumnInfo>, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::ColumnsLoaded {
                project_id,
                source_id,
                schema,
                table,
                result,
            },
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    pub(crate) fn database_message(&mut self, msg: database::Message) {
        // 四个动作都可能来自数据源树 header 行的右键菜单——菜单本体没有
        // "点了就自动收起"的行为(popup 盖在 dismiss 遮罩之上,点菜单项本身
        // 吃不到遮罩的点击),落地时顺手收掉,不来自菜单时该字段本就是
        // `None`,无副作用。
        if matches!(
            msg,
            database::Message::TestConnection(_)
                | database::Message::EditSourceStart(_)
                | database::Message::DeleteSourceRequest(_)
                | database::Message::SchemaRefresh(_)
        ) {
            self.database_source_menu = None;
        }
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            msg,
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    pub(crate) fn ssh_test_connection_result(
        &mut self,
        project_id: i64,
        host_id: String,
        result: Result<(), String>,
    ) {
        self.with_project(project_id, move |ws, io| {
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::TestConnectionResult(project_id, host_id, result),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    pub(crate) fn ssh_unknown_key_detected(
        &mut self,
        project_id: i64,
        host_id: String,
        fingerprint: String,
        key_bytes: Vec<u8>,
    ) {
        self.with_project(project_id, move |ws, io| {
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::UnknownKeyDetected(project_id, host_id, fingerprint, key_bytes),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    pub(crate) fn ssh_key_changed(
        &mut self,
        project_id: i64,
        host_id: String,
        fingerprint: String,
    ) {
        self.with_project(project_id, move |ws, io| {
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::KeyChanged(project_id, host_id, fingerprint),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    pub(crate) fn ssh_terminal_connect_failed(
        &mut self,
        project_id: i64,
        host_id: String,
        tab_id: usize,
        err: String,
    ) {
        self.with_project(project_id, move |ws, io| {
            ws.pending.remove(&tab_id);
            ws.ssh_out_pending.remove(&tab_id);
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::TerminalConnectFailed(project_id, host_id, tab_id, err),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    /// 指派任务给某个 agent 种类,纯记录,不触发任何执行。异步确认经
    /// `Mutated` 刷新列表——与其它写操作同一条乐观更新链路。
    pub(crate) fn todo_assign_agent(&mut self, idx: usize, agent: dozer_core::protocol::AgentKind) {
        let Some(id) = self
            .active_workspace()
            .and_then(|ws| ws.todo.items().get(idx))
            .map(|item| item.id)
        else {
            return;
        };
        self.with_focused_project(|ws, _io| {
            ws.todo.close_dispatch_popup();
        });
        let handle = self.handle.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        handle.spawn(async move {
            let res = client
                .assign_todo_agent(id, agent)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = proxy.send_event(Message::Todo(todo::Message::Mutated(res)));
        });
    }

    /// 打开任务详情弹窗:先本地记下 `idx`(弹窗定位/后续"处理"要用),再
    /// 异步拉 `GetTodoDetail`。RPC 结果经专门的 `Message::TodoDetailLoaded`
    /// 落地——`todo::Message::Mutated` 那条通用刷新链路只刷 `items`/
    /// `categories`,不携带回合数据,不能复用。
    pub(crate) fn todo_detail_open(&mut self, idx: usize) {
        let Some(id) = self
            .active_workspace()
            .and_then(|ws| ws.todo.items().get(idx))
            .map(|item| item.id)
        else {
            return;
        };
        self.with_focused_project(|ws, _io| {
            ws.todo.open_detail(idx, Vec::new());
        });
        let handle = self.handle.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        handle.spawn(async move {
            if let Ok((_, turns)) = client.get_todo_detail(id).await {
                let _ = proxy.send_event(Message::TodoDetailLoaded(idx, turns));
            }
        });
    }

    /// 详情弹窗"处理"按钮:乐观插入已经在 `todo::update`(`DetailReplySubmit`
    /// 分支)做过,这里只管发 `ProcessTodoNow` RPC 并在结果回来后用服务端
    /// 权威回合列表刷新。耗时可能到 10 分钟,走 `handle.spawn` 不阻塞 UI。
    pub(crate) fn todo_detail_process(&mut self) {
        let Some((idx, id, reply_text)) = self.active_workspace().and_then(|ws| {
            let idx = ws.todo.detail_open_idx()?;
            let id = ws.todo.items().get(idx)?.id;
            Some((idx, id, ws.todo.last_reply_text()))
        }) else {
            return;
        };
        let handle = self.handle.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        handle.spawn(async move {
            let _ = client.process_todo_now(id, reply_text.as_deref()).await;
            if let Ok((_, turns)) = client.get_todo_detail(id).await {
                let _ = proxy.send_event(Message::TodoDetailLoaded(idx, turns));
            }
        });
    }

    pub(crate) fn todo_message(&mut self, msg: todo::Message) {
        // 新增任务框高度拖拽:只在 app 层接管,置 `dragging_row`,后续
        // `CursorMoved` → `RowDrag` 由 `update` 统一换算高度写回
        // `ws.todo`(见 `RowDrag` 的 `TodoAddGrow` 分支)。这条不到
        // `todo::update`(那里有 no-op arm 保持 match 穷尽)。
        if let todo::Message::AddResizeStart = msg {
            self.dragging_row = Some(RowDivider::TodoAddGrow);
            return;
        }
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let last_cursor = self.last_cursor;
        // 异步结果/写确认透过 `proxy` 重发回主循环,回调里会借用 `self`
        // 的 client/handle ——闭包捕获是 move 出来的副本,行得通(同
        // `Message::Search` 分支的既有手法)。
        let emit = move |m: todo::Message| {
            let _ = proxy.send_event(Message::Todo(m));
        };
        self.with_focused_project(move |ws, _io| {
            // 点日历按钮时的光标逻辑坐标,作为窗口级 overlay 的弹出锚点——
            // 先记下再交给 `todo::update` 展开(它只管 `calendar_open`/`calendar_view`)。
            if matches!(msg, todo::Message::CalendarOpen(_)) {
                ws.todo.set_calendar_anchor(last_cursor);
            }
            // 点"指派"按钮时的光标逻辑坐标,作为派发选择层 overlay 的弹出锚点。
            if matches!(msg, todo::Message::DispatchOpen(_)) {
                ws.todo.set_dispatch_anchor(last_cursor);
            }
            // 点卡片左下"状态"按钮的光标逻辑坐标,作为状态下拉选择层 overlay
            // 的弹出锚点。`StatusOpen` 自身交给 `todo::update` 展开(它只改
            // `status_open`)。
            if matches!(msg, todo::Message::StatusOpen(_)) {
                ws.todo.set_status_anchor(last_cursor);
            }
            // 点搜索框左前"状态"segment 按钮时的光标逻辑坐标,作为搜索框状态
            // 筛选浮层 overlay 的弹出锚点。`StatusFilterOpen` 自身交给
            // `todo::update` 展开(它只改 `status_filter_open`)。
            if matches!(msg, todo::Message::StatusFilterOpen) {
                ws.todo.set_status_filter_anchor(last_cursor);
            }
            todo::update(&mut ws.todo, msg, project_id, &client, &handle, emit);
        });
    }

    pub(crate) fn browser_bookmarks_loaded(
        &mut self,
        pid: Option<i64>,
        bookmarks: Vec<BookmarkInfo>,
    ) {
        if pid.is_none() {
            // 首页全局浏览器(或无项目工作区)的收藏列表刷新。
            let handle = self.handle.clone();
            let client = self.client.clone();
            let proxy = self.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::HomeBrowser(m));
            };
            browser::update(
                &mut self.home_browser,
                browser::Message::BookmarksLoaded(None, bookmarks),
                None,
                &client,
                &handle,
                emit,
            );
            return;
        }
        self.with_project(pid.unwrap(), move |ws, io| {
            let pid = pid.unwrap();
            let handle = io.handle.clone();
            let client = io.client.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(
                &mut ws.browser,
                browser::Message::BookmarksLoaded(Some(pid), bookmarks),
                Some(pid),
                &client,
                &handle,
                emit,
            );
        });
    }

    pub(crate) fn browser_bookmarks_mutated(&mut self, pid: Option<i64>, res: Result<(), String>) {
        if pid.is_none() {
            // 首页全局浏览器(或无项目工作区)的添加/删除结果回调。
            let handle = self.handle.clone();
            let client = self.client.clone();
            let proxy = self.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::HomeBrowser(m));
            };
            browser::update(
                &mut self.home_browser,
                browser::Message::BookmarksMutated(None, res),
                None,
                &client,
                &handle,
                emit,
            );
            return;
        }
        self.with_project(pid.unwrap(), move |ws, io| {
            let pid = pid.unwrap();
            let handle = io.handle.clone();
            let client = io.client.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(
                &mut ws.browser,
                browser::Message::BookmarksMutated(Some(pid), res),
                Some(pid),
                &client,
                &handle,
                emit,
            );
        });
    }

    pub(crate) fn browser_message(&mut self, msg: browser::Message) {
        // 按下浏览器页签＝选中＋准备被拖走(`SelectTab` 在
        // `browser::update` 里真正选中为 `active`,这里按它记下拖起源)。
        let was_select = matches!(msg, browser::Message::SelectTab(_));
        self.with_focused_project(|ws, io| {
            let project_id = ws.project.as_ref().map(|p| p.id);
            let client = io.client.clone();
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(&mut ws.browser, msg, project_id, &client, &handle, emit);
        });
        if was_select
            && let Some(ws) = self.active_workspace()
            && ws.browser.active_tab_idx() < ws.browser.tab_count()
        {
            let active = ws.browser.active_tab_idx();
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Browser,
                source: active,
                press_pos: self.last_cursor,
            });
        }
    }

    pub(crate) fn term_input(&mut self, target: terminal::TermTarget, bytes: Vec<u8>) {
        // 终端不在屏上时丢弃按键(不报错、不写 PTY):否则用户在读
        // 对话审阅时敲的回车/方向键会静默提交给隐藏在后面的 agent
        // 会话(Fix round 2 #3)。
        let visible = match target {
            terminal::TermTarget::Shared => self.terminal_visible(),
            terminal::TermTarget::SshPanel => self.ssh_terminal_visible(), // Task 12 新增
        };
        if !visible {
            return;
        }
        self.with_focused_project(|ws, io| {
            match target {
                terminal::TermTarget::Shared => {
                    // 键入即回底 + 清选区：正在回看历史时一敲键盘，视口跳回
                    // 实时输出（常规终端语义），再把字节写给 daemon。
                    if let Some(tab) = ws.tabs.get_mut(ws.active) {
                        tab.model.scroll_to_bottom();
                        tab.model.selection_clear();
                    }
                    ws.send_input(io, bytes);
                }
                terminal::TermTarget::SshPanel => {
                    if let Some(tab) = ws.ssh_active_tab_mut() {
                        tab.model.scroll_to_bottom();
                        tab.model.selection_clear();
                    }
                    ws.ssh_send_input(io, bytes);
                }
            }
        });
    }

    pub(crate) fn term_output(&mut self, project_id: ProjectId, tab_id: usize, bytes: Vec<u8>) {
        self.with_project(project_id, |ws, io| {
            let Some(tab) = ws.tab_by_id_mut(tab_id) else {
                return;
            };
            // 实时输出可能含设备查询（DSR/DA 等），应答必须写回 PTY
            // ——atuin/claude 等 TUI 依赖它（此前丢弃导致探测超时）。
            tab.ingest_osc(&bytes);
            let responses = tab.model.feed(&bytes);
            if responses.is_empty() || !tab.alive {
                return;
            }
            match &tab.backend {
                TabBackend::Daemon => {
                    let client = io.client.clone();
                    let id = tab.info.id.clone();
                    io.handle.spawn(async move {
                        if let Err(e) = client.write(&id, &responses).await {
                            tracing::warn!("回写终端查询应答失败: {e}");
                        }
                    });
                }
                TabBackend::Ssh { out } => {
                    let _ = out.send(SshOut::Data(responses));
                }
            }
        });
    }

    pub(crate) fn term_paste(&mut self, target: terminal::TermTarget, text: String) {
        // 同 TermInput 的可见性闸门(Fix round 3):⌘V 粘贴走同一条
        // PTY 写入路径,粘贴内容若含换行还会在看不见的会话里直接
        // 执行,比单个按键更危险,必须同样拦截。
        let visible = match target {
            terminal::TermTarget::Shared => self.terminal_visible(),
            terminal::TermTarget::SshPanel => self.ssh_terminal_visible(),
        };
        if !visible {
            return;
        }
        self.with_focused_project(move |ws, io| {
            let bracketed = match target {
                terminal::TermTarget::Shared => {
                    ws.tabs.get(ws.active).map(|t| t.model.bracketed_paste())
                }
                terminal::TermTarget::SshPanel => ws
                    .ssh_tabs
                    .iter()
                    .find(|t| {
                        ws.ssh_active.as_ref().is_some_and(|(h, _)| {
                            t.info.id.strip_prefix("ssh:") == Some(h.as_str())
                        })
                    })
                    .map(|t| t.model.bracketed_paste()),
            };
            let Some(bracketed) = bracketed else {
                return;
            };
            match target {
                terminal::TermTarget::Shared => {
                    if let Some(tab) = ws.tabs.get_mut(ws.active) {
                        tab.model.scroll_to_bottom();
                    }
                }
                terminal::TermTarget::SshPanel => {
                    if let Some(tab) = ws.ssh_active_tab_mut() {
                        tab.model.scroll_to_bottom();
                    }
                }
            }
            let bytes = if bracketed {
                let mut b = b"\x1b[200~".to_vec();
                b.extend_from_slice(text.as_bytes());
                b.extend_from_slice(b"\x1b[201~");
                b
            } else {
                text.into_bytes()
            };
            match target {
                terminal::TermTarget::Shared => ws.send_input(io, bytes),
                terminal::TermTarget::SshPanel => ws.ssh_send_input(io, bytes),
            }
        });
    }

    pub(crate) fn select_tab(&mut self, idx: usize) {
        // 按下页签＝选中＋准备被拖走:选中仍是唯一的语义,但顺带记下
        // "这一页签正被按住",随后鼠标划过其它页签时 `on_move` 触发
        // `TabDragMove` 完成换位;松开时 main.rs `TabDragEnd` 收尾。
        // 只应该被 tab 栏本身的按钮调用——见 `Message::SelectTab` 文档。
        self.select_tab_no_drag(idx);
        if let Some(ws) = self.active_workspace()
            && idx < ws.tabs.len()
        {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Terminal,
                source: idx,
                press_pos: self.last_cursor,
            });
        }
    }

    /// `select_tab` 去掉"武装拖拽状态机"那部分,给非 tab 栏的调用方
    /// (Agent 面板右侧卡片列表)用——见 `Message::SelectTabNoDrag` 文档。
    pub(crate) fn select_tab_no_drag(&mut self, idx: usize) {
        // `with_focused_project` 的闭包里借的是 `ws`,拿不到 `self`——真实
        // 可用宽度得在借用开始前算好(同 `preview_select_tab` 那批调用方的
        // 既有先例)。
        let avail_px = self.terminal_tab_bar_avail_px();
        self.with_focused_project(move |ws, _io| {
            if idx < ws.tabs.len() {
                ws.active = idx;
                // 从 tab 栏的 V 下拉里选中某一项:选中后把主条滚入可见窗口
                // (若该项仍横向可见则不受影响,见 `tab_window_reveal`),并收起
                // 下拉——避免"选中了却看不见在哪"。下拉列的是组内全部 tab,
                // 高亮常驻在可见宽度内时不会多跳一行。
                let widths: Vec<f32> = ws
                    .tabs
                    .iter()
                    .map(|t| tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name)))
                    .collect();
                ws.term_tab_first =
                    tab_widget::tab_window_reveal(&widths, 4.0, avail_px, ws.term_tab_first, idx);
                ws.term_tab_overflow_anchor = None;
            }
        });
        // `term_ime_preedit` 是 `App` 上唯一一份、不按 tab 分的组字预览态
        // (见该字段文档),只在真正 `Ime::Commit`/组字取消时才清空——切
        // tab 不会清。`TermTarget::Shared` 这道"该不该显示"闸门只判断
        // "键盘现在归不归共享终端条",不区分具体哪个 tab,于是新切过去的
        // tab 会直接"继承"上一个 tab 还没提交完的组字预览文字/候选词
        // (2026-08-21 用户实测反馈:切 tab 后串台,选完字对方也不会真的
        // 收到那些字——因为提交字节确实是发给切换后的新 tab 的 PTY,只是
        // 预览视觉是借来的)。切 tab 时无条件清掉(即使 idx 越界导致上面
        // 没真的切,清掉一份陈旧组字预览也没有副作用),避免这份陈旧状态
        // 被新激活的 tab 误当成自己的组字预览渲染出来。
        self.term_ime_preedit = None;
    }

    pub(crate) fn pane_resized(&mut self, cols: u16, rows: u16, ssh_cols: u16, ssh_rows: u16) {
        if cols == 0 || rows == 0 {
            return;
        }
        // 共享与 SSH 两个网格各自带独立去重:任一真变了都要往 dev 文件里
        // propagate,不能因为共享网格没动就跳掉 SSH 网格的同步。
        let shared_changed = (cols, rows) != (self.cols, self.rows);
        let ssh_changed = (ssh_cols, ssh_rows) != (self.ssh_cols, self.ssh_rows);
        if !shared_changed && !ssh_changed {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.ssh_cols = ssh_cols;
        self.ssh_rows = ssh_rows;
        let io = self.shell_io();
        // 终端网格是窗口级的:并行打开的每个项目各有一套终端 tab,但它们
        // 共用同一批 pane。只改当前项目的话,切回后台项目会看到一个停在
        // 旧网格、和 pane 对不上的画面,直到用户偶然再拖一次窗口才纠正——
        // 所以这里对所有已加载项目一起改(`Stub` 还没有任何 tab,促成时
        // 自然按当时的 `io.cols/rows`)。共享/SSH 各自带独立网格下发。
        for slot in self.projects.values_mut() {
            if let WorkspaceSlot::Loaded(ws) = slot {
                ws.resize_all(&io, cols, rows, ssh_cols, ssh_rows);
            }
        }
    }

    pub(crate) fn panel_select(&mut self, kind: PanelKind) {
        let side = self.shell_layout.rail_layout.side_of(kind);
        // 武装拖拽态:按住图标＝准备拖(同 `TabDrag` 的"按下即武装"手法)。
        // 同栏重排 / 跨栏移动都是靠渲染层挂在图标上的 `on_move` 驱动
        // (`Message::RailDragMove`),`MouseMotion` 期间逐帧上报；这里只记下
        // "从哪栏的哪个位置开始拖"。`RailLayout` 的不变式(sanitize 已保证
        // 10 个面板不重不漏分到两栏)确保 `kind` 一定能在 `side_of` 返回的
        // 那一栏里被 `position` 找到。
        let source_index = self
            .shell_layout
            .rail_layout
            .side(side)
            .iter()
            .position(|&k| k == kind)
            .expect("kind 应该在 side_of 返回的那一侧里,sanitize 已保证不变式");
        self.rail_drag = Some(rail::RailDrag {
            source_side: side,
            source_index,
            origin_index: source_index,
            pending_cross_side: None,
            press_pos: self.last_cursor,
        });
        // 点当前已激活的图标:退回未选中并收起对应面板区;但若对侧面板区
        // 也已收起,当前侧就是最后一个还开着的 zone,不能关(两侧对称)。
        let switched = match side {
            Side::Left => {
                if self.left_view == kind {
                    if !self.right_collapsed {
                        self.left_collapsed = !self.left_collapsed;
                    }
                    false
                } else {
                    self.left_view = kind;
                    self.left_collapsed = false;
                    true
                }
            }
            Side::Right => {
                if self.right_view == kind {
                    if !self.left_collapsed {
                        self.right_collapsed = !self.right_collapsed;
                    }
                    false
                } else {
                    self.right_view = kind;
                    self.right_collapsed = false;
                    true
                }
            }
        };
        // 面板专属的"切入时动作"。原左栏处理器把 GitLog/Todo/
        // Database/Project/Ssh 的触发放在 if/else 之后的无条件
        // `if self.left_view == PanelKind::X` 里——收起/展开当前激活的特殊
        // 面板也会跑一遍;原右栏处理器把 Usage 放在 else(真正
        // 切换)分支里——只有切换时才触发。为保持逐像素零差异,左侧面板恒
        // 触发、右侧面板仅在真正切换时触发(默认布局下它们恰好按这个分侧;
        // Stage 4 拖拽换栏后这里再按 `rail_layout.side_of` 重新对齐各面板
        // 的触发语义)。
        let fire = match side {
            Side::Left => true,
            Side::Right => switched,
        };
        if fire {
            match kind {
                PanelKind::GitLog => self.sync_git_log_to_active_project(),
                PanelKind::Todo => {
                    if let Some(project_id) = self.active_project_id {
                        let client = self.client.clone();
                        let handle = self.handle.clone();
                        let proxy = self.proxy.clone();
                        let emit = move |m: todo::Message| {
                            let _ = proxy.send_event(Message::Todo(m));
                        };
                        let emit_todos = emit.clone();
                        todo::request_todos_refresh(project_id, &client, &handle, emit_todos);
                        todo::request_categories_refresh(project_id, &client, &handle, emit);
                    }
                }
                PanelKind::Database => self.with_focused_project(|ws, _io| {
                    if let Some(project) = ws.project.as_ref() {
                        database::reload_from_disk(
                            &mut ws.database,
                            std::path::Path::new(&project.path),
                        );
                    }
                }),
                PanelKind::Project => self.ensure_project_readme_and_reveal(),
                PanelKind::Ssh => self.with_focused_project(|ws, _io| {
                    if let Some(project) = ws.project.as_ref() {
                        ssh::reload_from_disk(&mut ws.ssh, std::path::Path::new(&project.path));
                    }
                }),
                PanelKind::Usage => self.with_focused_project(|ws, io| {
                    ws.usage.set_loading(true);
                    ws.spawn_usage_refresh(io);
                }),
                // 会话列表原本只在项目打开时和回合结束时刷新,切进这个面板时
                // 没有任何补救手段——离开一段时间再切回来看到的还是上次的
                // 快照。补一次切入即刷新,同 `Usage` 面板的既有口径。
                PanelKind::Conversations => self.with_focused_project(|ws, io| {
                    ws.spawn_conversations_refresh(io);
                }),
                PanelKind::Files | PanelKind::Web | PanelKind::Agent => {}
            }
        }
        // 图标栏点击一律退出放大态。放大态浮层不拦图标栏上的点击
        // (遮罩两侧垫的是无交互 Space,点击穿到下层图标按钮),所以
        // "放大左侧 → 点文件夹图标收起左侧"是可达的:不清 `maximized`
        // 就会留下一个空的金色描边浮层,只能点变暗区才能脱身
        // (Fix round 2 #2)。切换本侧显示什么内容时,放大态本也不该
        // 存活,无条件清最简单也最不容易出意外。
        self.maximized = None;
        self.on_shell_layout_changed();
    }

    /// 文件预览右上角按钮:翻转文件树列表子栏的展开/收起。只改一个布尔
    /// (`dims.files_tree_collapsed`),不动 `files_split` 比例(展开时按原比例
    /// 恢复)。收起态下文件树列表不渲染、预览拿满整个配对宽度。与
    /// `panel_select` 一样退出放大态并落盘/重算网格。
    pub(crate) fn toggle_files_tree_collapse(&mut self) {
        self.dims.files_tree_collapsed = !self.dims.files_tree_collapsed;
        self.maximized = None;
        self.on_shell_layout_changed();
    }

    /// 该面板当前是否偏离了默认栏——8 个有内部两栏布局的面板据此决定
    /// 渲染顺序要不要反转。这个 Stage 结束时 `RailLayout` 只可能是
    /// `default()`,所以这个函数在正常运行时恒返回 `false`;它的分支
    /// 靠单元测试直接构造非默认 `RailLayout` 来触发验证,不依赖 GUI
    /// 能不能拖拽出这个状态(Stage 4 才有拖拽)。
    pub(crate) fn panel_mirrored(&self, kind: PanelKind) -> bool {
        rail::panel_mirrored_in(&self.shell_layout.rail_layout, kind)
    }

    /// 文件树列表子栏当前是否被收起(文件预览右上角按钮切换)。暴露只读
    /// 的 `dims.files_tree_collapsed` 给 workspace 层渲染收起按钮时用,
    /// `dims` 字段本身保持模块私有。
    pub(crate) fn files_tree_collapsed(&self) -> bool {
        self.dims.files_tree_collapsed
    }

    /// 七个两栏面板(Project/Todo/Database/Ssh/Agent/Conversations/Usage)的
    /// 列表列当前是否被收起。语义同 `files_tree_collapsed`:列表不渲染、内容
    /// 拿满配对宽度,split 比例保留(展开时按原宽度恢复)。
    pub(crate) fn list_collapsed(&self, kind: PanelKind) -> bool {
        match kind {
            PanelKind::Project => self.dims.project_list_collapsed,
            PanelKind::Todo => self.dims.todo_list_collapsed,
            PanelKind::Database => self.dims.database_list_collapsed,
            PanelKind::Ssh => self.dims.ssh_list_collapsed,
            PanelKind::Agent => self.dims.agent_list_collapsed,
            PanelKind::Conversations => self.dims.conversations_list_collapsed,
            PanelKind::Usage => self.dims.usage_list_collapsed,
            _ => false,
        }
    }

    /// 翻转某两栏面板列表列的展开/收起(`Message::TogglePanelListCollapse` 的
    /// 处理)。只改对应布尔、不动 split 比例,并像 `panel_select` 一样退出
    /// 放大态 + 落盘/重算网格。非两栏面板(`Files` 走独立的
    /// `files_tree_collapsed`,其余单/两栏面板无此能力)直接忽略。
    pub(crate) fn toggle_panel_list_collapse(&mut self, kind: PanelKind) {
        let flag = match kind {
            PanelKind::Project => &mut self.dims.project_list_collapsed,
            PanelKind::Todo => &mut self.dims.todo_list_collapsed,
            PanelKind::Database => &mut self.dims.database_list_collapsed,
            PanelKind::Ssh => &mut self.dims.ssh_list_collapsed,
            PanelKind::Agent => &mut self.dims.agent_list_collapsed,
            PanelKind::Conversations => &mut self.dims.conversations_list_collapsed,
            PanelKind::Usage => &mut self.dims.usage_list_collapsed,
            _ => return,
        };
        *flag = !*flag;
        self.maximized = None;
        self.on_shell_layout_changed();
    }

    /// 内容侧"收起/展开列表列"按钮:用户点击某面板内容区的按钮翻转其列表列
    /// 显隐。语义完全对齐文件预览的 `FileTreeCollapse` 按钮(见 `preview_pane_for`),
    /// 只是图标按该面板当前所在栏(左/右)与收起态四选一、tooltip 由调用方
    /// 给静态文案。泛型 `M` 兼容顶层 `Message` 与各扩展模块的本地 `Message`。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn list_collapse_button<'a, M: Clone + 'a>(
        &self,
        kind: PanelKind,
        collapsed: bool,
        hover_id: HoverId,
        tooltip_collapse: &'a str,
        tooltip_expand: &'a str,
        on_select: M,
        on_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
        let side = self.shell_layout.rail_layout.side_of(kind);
        let (icon, tooltip) = match (side, collapsed) {
            (Side::Left, false) => (icons::IconKind::PanelLeftClose, tooltip_collapse),
            (Side::Left, true) => (icons::IconKind::PanelLeftOpen, tooltip_expand),
            (Side::Right, false) => (icons::IconKind::PanelRightClose, tooltip_collapse),
            (Side::Right, true) => (icons::IconKind::PanelRightOpen, tooltip_expand),
        };
        icons::icon_button_entry(
            icon,
            byteui::theme::icon_size::row(),
            false,
            false,
            self.hover_progress(hover_id),
            false,
            byteui::theme::icon_size::row() + 6.0,
            true,
            on_select,
            on_hover,
            tooltip,
        )
    }

    pub(crate) fn top_bar_home(&mut self) {
        self.current_page = AppPage::Home;
        self.home_recents_loaded = false;
        self.home_left_view = homespace::HomeLeftView::default();
        self.home_project_pages = 1;
        self.home_project_search.clear();
        self.home_project_search_draft.clear();
        self.home_project_search_focused = false;
        self.home_right_view = homespace::HomeRightView::default();
        let projects: Vec<ProjectInfo> = self.recent_projects.iter().take(5).cloned().collect();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        self.handle.spawn(async move {
            let (files, convs) = load_home_recents(&client, &projects).await;
            let _ = proxy.send_event(Message::HomeRecentsLoaded(files, convs));
        });
    }

    pub(crate) fn preview_open_path(&mut self, path: PathBuf) {
        // 同 `preview_select_tab`:`preview_tab_bar_avail_px` 要 `&self`,
        // 得在 `with_focused_project` 的 `&mut self` 借用之前先算好。
        let avail_w = self.preview_tab_bar_avail_px(PanelKind::Files);
        self.with_focused_project(move |ws, io| {
            if !path.is_file() {
                ws.preview_error = Some(format!("文件不存在或不可读: {}", path.display()));
                return;
            }
            ws.preview_error = None;
            ws.files.set_tree_selected(path.clone());
            ws.allowed_files
                .lock()
                .expect("allowed_files 锁")
                .insert(path.clone());
            ws.preview.open_path(path);
            // 新 tab 落在末尾(复用已开的文件则落在该文件原来的位置)——
            // 用跟 `preview_select_tab` 同一套 `tab_window_reveal`,把窗口
            // 起点钳到"包含这个新激活 tab"的位置,而不是无脑滚回最左
            // (此前 `= 0` 的写法:tab 一多,新开的文件反而被滚出可见区,
            // 2026-09-14 用户反馈"新打开文件时 tab 应该跳转到对应位置")。
            let active = ws.preview.active_idx();
            let widths: Vec<f32> = ws
                .preview
                .tabs()
                .iter()
                .map(|t| preview_tab_display_width(&t.title))
                .collect();
            ws.preview_tab_first =
                tab_widget::tab_window_reveal(&widths, 4.0, avail_w, ws.preview_tab_first, active);
            ws.spawn_preview_state_save(io);
            ws.spawn_preview_context_push(io);
        });
    }

    pub(crate) fn preview_select_tab(&mut self, idx: usize) {
        let arming = self
            .active_workspace()
            .map(|ws| idx < ws.preview.tabs().len())
            .unwrap_or(false);
        // 得在借用 `ws` 之前算好——`preview_tab_bar_avail_px` 要 `&self`,
        // 跟下面 `with_focused_project` 内部的 `&mut self` 借用冲突,必须
        // 提前拿到这个值再原样传进闭包(同渲染侧 `preview_pane_for` 用的
        // 是同一个真实宽度,窗口起点算法两边才不会算出不一致的结果)。
        let avail_w = self.preview_tab_bar_avail_px(PanelKind::Files);
        self.with_focused_project(|ws, io| {
            ws.preview.select(idx);
            if idx < ws.preview.tabs().len() {
                let widths: Vec<f32> = ws
                    .preview
                    .tabs()
                    .iter()
                    .map(|t| preview_tab_display_width(&t.title))
                    .collect();
                ws.preview_tab_first =
                    tab_widget::tab_window_reveal(&widths, 4.0, avail_w, ws.preview_tab_first, idx);
                ws.preview_tab_overflow_anchor = None;
            }
            ws.spawn_preview_state_save(io);
            ws.spawn_preview_context_push(io);
        });
        if arming {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Preview,
                source: idx,
                press_pos: self.last_cursor,
            });
        }
    }

    /// Project 面板右配对预览打开文件:写入 `ws.project_preview`(独立的
    /// `PreviewPane`),完全不碰 Files 预览的 `ws.preview`/`ws.files`。
    /// tab 是项目链接点开产生的会话期状态,不持久化、也不向 daemon 推上下文,
    /// 避免与 Files 预览那份持久化 `preview_state` 互相覆盖。
    pub(crate) fn project_preview_open_path(&mut self, path: PathBuf) {
        let avail_w = self.preview_tab_bar_avail_px(PanelKind::Project);
        self.with_focused_project(|ws, _io| {
            if !path.is_file() {
                ws.project_preview_error = Some(format!("文件不存在或不可读: {}", path.display()));
                return;
            }
            ws.project_preview_error = None;
            ws.project_panel.set_selected_link(path.clone());
            ws.allowed_files
                .lock()
                .expect("allowed_files 锁")
                .insert(path.clone());
            ws.project_preview.open_path(path);
            // 新 tab 落在末尾(或复用已开文件原位),用 `tab_window_reveal`
            // 钳出包含它的窗口起点,不再无脑滚回最左(同 Files 预览)。
            let active = ws.project_preview.active_idx();
            let widths: Vec<f32> = ws
                .project_preview
                .tabs()
                .iter()
                .map(|t| preview_tab_display_width(&t.title))
                .collect();
            ws.project_preview_tab_first = tab_widget::tab_window_reveal(
                &widths,
                4.0,
                avail_w,
                ws.project_preview_tab_first,
                active,
            );
        });
    }

    /// 项目信息面板切入时调用:确保项目根目录有一份 `README.md`(没有就按
    /// 项目名 + 描述生成,已有则原样保留),然后**一律**在右侧配套预览窗打
    /// 开这份 README(首次切进来就让它展示项目文档)。
    ///
    /// 打开/生成依赖同一份"可读"保障——README 创建失败或不可读时静默返回,
    /// 绝不拿一个空文件去占预览,也绝不让面板切入失败。
    pub(crate) fn ensure_project_readme_and_reveal(&mut self) {
        let Some(project) = self.active_workspace().and_then(|ws| ws.project.clone()) else {
            return;
        };
        let Some(readme) =
            ensure_project_readme(&std::path::PathBuf::from(&project.path), &project.name)
        else {
            return;
        };
        self.project_preview_open_path(readme);
    }

    pub(crate) fn project_preview_select_tab(&mut self, idx: usize) {
        let arming = self
            .active_workspace()
            .map(|ws| idx < ws.project_preview.tabs().len())
            .unwrap_or(false);
        // 同 `preview_select_tab`:提前算好真实可用宽度,避免跟
        // `with_focused_project` 的 `&mut self` 借用冲突。
        let avail_w = self.preview_tab_bar_avail_px(PanelKind::Project);
        self.with_focused_project(|ws, _io| {
            ws.project_preview.select(idx);
            if idx < ws.project_preview.tabs().len() {
                let widths: Vec<f32> = ws
                    .project_preview
                    .tabs()
                    .iter()
                    .map(|t| preview_tab_display_width(&t.title))
                    .collect();
                ws.project_preview_tab_first = tab_widget::tab_window_reveal(
                    &widths,
                    4.0,
                    avail_w,
                    ws.project_preview_tab_first,
                    idx,
                );
                ws.project_preview_tab_overflow_anchor = None;
            }
        });
        if arming {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::ProjectPreview,
                source: idx,
                press_pos: self.last_cursor,
            });
        }
    }

    pub(crate) fn agent_state_changed(
        &mut self,
        project_id: ProjectId,
        tab_id: usize,
        agent: AgentKind,
        state: AgentState,
        transcript_path: Option<String>,
    ) {
        self.with_project(project_id, |ws, io| {
            // 当前项目路径先取出（下面要 &mut 借 tab，冲突）；重锚:项目优先。
            let active_repo = ws.project.as_ref().map(|p| PathBuf::from(&p.path));
            // 卡片刷新要传的数据在这里先摘出来（`Option` 同时充当"tab 是否
            // 存在"的哨兵）：`tab`（来自 `ws.tab_by_id_mut`）借的是整个
            // `ws`，下面 TurnEnded 分支还要再用 `tab`，中间插一句
            // `ws.spawn_agent_card_refresh`（借 `&ws`）会跟这个 `&mut ws`
            // 借用重叠、过不了借用检查；摘成局部变量、挪到这个 `if let`
            // 块结束、`tab` 借用已经释放之后再调用，规避这个冲突,语义不变
            // (仍然是"tab 存在就必调用一次,不进 TurnEnded 条件分支")。
            let mut card_refresh_args: Option<(Option<String>, PathBuf)> = None;
            if let Some(tab) = ws.tab_by_id_mut(tab_id) {
                tab.agent_state = state;
                tab.agent = agent;
                // 补一道:picker 不是唯一入口(比如在已开着的纯 shell tab 里
                // 手打 `opencode`),那种情况下 `on_tab_attached` 拿不到
                // picker 提示,只能靠这条 hook 上报事后补上——虽然大概率已经
                // 错过了 agent 进程启动瞬间那波终端探测查询,但仍是"跟真实
                // agent 保持同步"的唯一后备信号。
                tab.model
                    .set_answer_dynamic_color(agent == AgentKind::Opencode);
                if let Some(tp) = transcript_path {
                    tab.transcript_path = Some(tp);
                }
                tracing::info!(tab_id, ?state, "agent 状态变更");
                card_refresh_args = Some((tab.transcript_path.clone(), tab.effective_cwd()));
            }
            if let Some((transcript_path, cwd)) = card_refresh_args {
                ws.spawn_agent_card_refresh(
                    io,
                    tab_id,
                    agent,
                    transcript_path,
                    cwd,
                    active_repo.clone(),
                );
            }
            // 回合结束后刷新会话列表(transcript 增长/新增；P1j)与项目 git/
            // 磁盘占用状态(文件树装饰随之更新；P1h)。这两项刷新原先分别挂在
            // 会话列表自己的耦合链路、以及已删除的验收检测异步回调
            // (`delivery_checked`)上——验收闭环删除后,后者连带的刷新触发点
            // 也没了,这里改成回合结束就无条件触发,不再依赖任何验收检测结果
            // (前半"会话列表"这条 P1j 当年就已经这样修过一次,这次是把后半
            // "git/磁盘占用"也补齐同样的处理)。
            if state == AgentState::TurnEnded {
                ws.spawn_conversations_refresh(io);
                if let Some(project) = &ws.project {
                    let repo_path = PathBuf::from(&project.path);
                    spawn_project_git_refresh(project_id, repo_path.clone(), io);
                    spawn_disk_usage_refresh(project_id, repo_path, io);
                }
            }
            // 审阅 tab 若开着且属本会话,回合结束重解析 transcript（P1i）。
            if state == AgentState::TurnEnded
                && let Some(rv) = &ws.review
                && review_should_refresh_on_turn(&rv.source, tab_id)
                && let Some((path, tab_agent)) = ws
                    .tabs
                    .iter()
                    .find(|t| t.tab_id == tab_id)
                    .and_then(|t| t.transcript_path.clone().map(|p| (p, t.agent)))
            {
                ws.spawn_review_load(
                    io,
                    ReviewSource::Session(tab_id),
                    path,
                    tab_agent,
                    -1,
                    10_000,
                );
            }
        });
    }

    /// 点击对话面板扁平列表里的某一行——只把审阅面板加载到这一个回合
    /// 点对话面板会话列表某一行 → 打开该 session 的详情审阅:整段回合
    /// 列表 + 总结展示区(标题/全文)。`agent` 由调用方随行内数据一并传入。
    /// 总结数据直接取自已加载的 `SessionRow`,不为此单独发请求(2026-08-27)。
    pub(crate) fn conversation_session_open(&mut self, conversation_id: String, agent: AgentKind) {
        self.with_focused_project(move |ws, io| {
            let Some(row) = ws
                .conversations
                .sessions()
                .and_then(|rows| rows.iter().find(|r| r.conversation_id == conversation_id))
            else {
                // 列表刷新与点击之间的竞态(极小概率):这一行已经不在当前
                // 列表里了,直接不打开详情,不 panic、不报错弹窗。
                return;
            };
            let summary_title = row.display_title.clone();
            let summary_text = row.summary.clone();
            let last_ts = row.last_ts;
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let source = ReviewSource::Conversation(conversation_id.clone());
            ws.review = Some(ReviewView {
                source: source.clone(),
                entries: Vec::new(),
                error: None,
                agent,
                nonce: 0,
                summary_title: Some(summary_title),
                summary_text,
                summary_time: Some(relative_time_text(last_ts, now_ms)),
            });
            ws.spawn_review_load_conversation(
                io,
                conversation_id,
                -1,
                CONVERSATION_DETAIL_PAGE_SIZE,
                false,
            );
        });
    }

    pub(crate) fn search_results(
        &mut self,
        project_id: i64,
        result: Result<Vec<(String, Vec<search::SearchHit>)>, String>,
    ) {
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        self.with_project(project_id, move |ws, _io| {
            let emit = move |m| {
                let _ = proxy.send_event(Message::Search(m));
            };
            search::update(
                &mut ws.search,
                search::Message::SearchResults(project_id, result),
                project_id,
                &handle,
                emit,
            );
        });
    }

    pub(crate) fn files_project_message(&mut self, project_id: i64, msg: files::Message) {
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Files(m));
        };
        let app_files = &mut self.files;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        files::update(&mut ws.files, app_files, msg, project_id, &handle, emit);
    }

    /// Project 面板链接行右键菜单浮层:当前只含"删除"。定位坐标复用
    /// `files.last_right_click`(main.rs 任意右键都会先写入),"删除"回
    /// `project::Message::LinkRemove`。
    pub(crate) fn project_link_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.project_link_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            vec![crate::chrome::menu::item::<Message>(
                Some(icons::IconKind::Trash),
                "删除",
                Message::Project(project::Message::LinkRemove {
                    target: menu.target,
                    index: menu.index,
                }),
            )];

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::chrome::menu::shell_frosted(items, Length::Shrink);
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: menu.y,
                left: menu.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }

    /// Todo 分类树节点右键菜单浮层:新建子/同级分类、重命名、删除、上移/
    /// 下移、移动到...。定位坐标复用 `files.last_right_click`。
    pub(crate) fn category_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.category_context_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        // 右键的是"全部"/"未分类"伪节点:菜单只含"新建分类"(新建顶层
        // 分类,不挂在任何真实名字下——那两个只是视图桶)。真实节点才给
        // 完整节点操作集。
        let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            match menu.id {
                None => vec![crate::chrome::menu::item::<Message>(
                    Some(icons::IconKind::SquarePlus),
                    "新建分类",
                    Message::Todo(todo::Message::CategoryNewSibling(None)),
                )],
                Some(id) => {
                    // 被右键节点的 parent_id,给"新建同级分类"用(同级 =
                    // 挂在同一个 parent_id 下)。取不到就退化为顶层。
                    let sibling_parent_id = self.active_workspace().and_then(|ws| {
                        ws.todo
                            .categories()
                            .iter()
                            .find(|c| c.id == id)
                            .and_then(|c| c.parent_id)
                    });
                    vec![
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::SquarePlus),
                            "新建子分类",
                            Message::Todo(todo::Message::CategoryNewChild(id)),
                        ),
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::SquarePlus),
                            "新建同级分类",
                            Message::Todo(todo::Message::CategoryNewSibling(sibling_parent_id)),
                        ),
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::ChevronUp),
                            "上移",
                            Message::Todo(todo::Message::CategoryMoveSibling(
                                id,
                                dozer_core::protocol::CategoryMoveDirection::Up,
                            )),
                        ),
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::ChevronDown),
                            "下移",
                            Message::Todo(todo::Message::CategoryMoveSibling(
                                id,
                                dozer_core::protocol::CategoryMoveDirection::Down,
                            )),
                        ),
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::FolderOpen),
                            "移动到...",
                            Message::Todo(todo::Message::CategoryReparentPickerOpen(id)),
                        ),
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::Rename),
                            "重命名",
                            Message::Todo(todo::Message::CategoryRenameStart(id)),
                        ),
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::Trash),
                            "删除",
                            Message::Todo(todo::Message::CategoryDelete(id)),
                        ),
                    ]
                }
            };

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::chrome::menu::shell_frosted(items, Length::Shrink);
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: menu.y,
                left: menu.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }

    /// 分类选择器浮层:列出当前项目的全部分类节点(全展开按 depth 缩进
    /// 平铺),点"未分类"或某个节点即把 `category_picker` 目标落盘并关闭
    /// (Category → reparent,Todo → set_todo_category)。浮层只服务这两类
    /// "把一个节点挂到某个分类"的动作 —— 搜索框的分类速滤已移除(改为
    /// 按状态过滤),不再有 `Filter` 目标。
    pub(crate) fn category_picker_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let Some(picker) = &self.category_picker else {
            return column![].into();
        };
        let categories = self
            .active_workspace()
            .map(|ws| ws.todo.categories().to_vec())
            .unwrap_or_default();
        let mut list = column![].spacing(2);

        // "未分类"钉在树顶(`None` = 未分类)。
        list = list.push(
            button(text("未分类").size(byteui::theme::font::body()))
                .on_press(Message::CategoryPickerSelect(None))
                .width(Length::Fill)
                .padding([6, 10]),
        );
        // 复用一份没有展开态(全展开)的拍平——选择器只做单次选择,不需要
        // 折叠交互,直接把整棵树按 depth 缩进平铺出来最简单。
        let all_expanded: std::collections::HashSet<i64> =
            categories.iter().map(|c| c.id).collect();
        for row in todo::visible_category_rows(&categories, &all_expanded) {
            let id = row.id;
            let click = Message::CategoryPickerSelect(Some(id));
            list = list.push(
                button(
                    text(format!("{}{}", "  ".repeat(row.depth), row.name))
                        .size(byteui::theme::font::body()),
                )
                .on_press(click)
                .width(Length::Fill)
                .padding([6, 10]),
            );
        }
        let list =
            container(list.width(Length::Fixed(220.0))).style(move |_t: &iced_widget::Theme| {
                container::Style {
                    background: Some(byteui::theme::color::current().card.into()),
                    border: Border {
                        color: byteui::theme::color::current().border,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..container::Style::default()
                }
            });
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: picker.y,
                left: picker.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }

    /// 任务详情弹窗:原生 iced 渲染(不复用会话面板的 webview trace——
    /// 那套渲染实际内容在 `dozer://review-trace/host.html` 里,任务详情
    /// 只需要看人类/agent 往来文本,不需要工具调用折叠/trace 可视化,
    /// 塞进一个跟随光标定位、随时开合的原生弹窗里没有必要也不合适)。
    pub(crate) fn todo_detail_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let Some(ws) = self.active_workspace() else {
            return column![].into();
        };
        let Some(idx) = ws.todo.detail_open_idx() else {
            return column![].into();
        };
        let Some(item) = ws.todo.items().get(idx) else {
            return column![].into();
        };

        let header = column![
            text(item.text.clone()).size(byteui::theme::font::subtitle()),
            text(
                item.assigned_agent
                    .map(|a| format!("指派给:{}", a.label()))
                    .unwrap_or_else(|| "未指派".to_string())
            )
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim),
        ]
        .spacing(4);

        let mut turns_col = column![].spacing(8);
        for turn in ws.todo.detail_turns() {
            let label = if turn.role == "human" {
                "你".to_string()
            } else {
                item.assigned_agent
                    .map(|a| a.label().to_string())
                    .unwrap_or_else(|| "AI".to_string())
            };
            turns_col = turns_col.push(
                column![
                    text(label)
                        .size(byteui::theme::font::caption())
                        .color(byteui::theme::color::current().gold),
                    text(turn.content.clone())
                        .size(byteui::theme::font::body())
                        .width(Length::Fill),
                ]
                .spacing(2),
            );
        }
        let turns_scroll = iced_widget::Scrollable::new(turns_col)
            .width(Length::Fill)
            .height(Length::Fixed(320.0))
            .direction(iced_widget::scrollable::Direction::Vertical(
                byteui::interaction::scrollbar::scrollbar(),
            ))
            .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style());

        let reply_box = container(byteui::form::input_text::view(
            "回复...",
            ws.todo.detail_reply_draft(),
            false,
            Some(todo::detail_reply_field_id()),
            false,
            None,
            false,
            |s| Message::Todo(todo::Message::DetailReplyInput(s)),
        ))
        .width(Length::Fill);
        let submit_label = if ws.todo.detail_processing() {
            "处理中…"
        } else {
            "处理"
        };
        let submit = button(text(submit_label))
            .on_press_maybe(
                (!ws.todo.detail_processing())
                    .then_some(Message::Todo(todo::Message::DetailReplySubmit)),
            )
            .padding([6, 12]);

        // 宽度改用 `dialog::width`(整窗 1/3,2026-09-15 统一约定),取代此前
        // 写死的 480px。
        let card = column![header, turns_scroll, row![reply_box, submit].spacing(8)]
            .spacing(12)
            .padding(16)
            .width(crate::dialog::width(self.window_size.0));
        let card = container(card).style(crate::dialog::card_style);

        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced_widget::core::alignment::Horizontal::Center)
            .align_y(iced_widget::core::alignment::Vertical::Center)
            .into()
    }

    /// 数据库面板数据源树 header 行右键菜单浮层:测试连接/编辑/删除/刷新。
    /// 定位坐标复用 `files.last_right_click`。"刷新"只有该数据源当前已
    /// 展开(有 schema 树数据)才可点,未展开时置灰——展开动作本身走左键
    /// 点 header,不进这个菜单。
    pub(crate) fn database_source_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.database_source_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let source_id = menu.source_id.clone();
        let expanded = self
            .active_workspace()
            .is_some_and(|ws| ws.database.is_expanded(&source_id));
        let dim = byteui::theme::color::current().dim;
        let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> = vec![
            crate::chrome::menu::item::<Message>(
                Some(icons::IconKind::RefreshCw),
                "测试连接",
                Message::Database(database::Message::TestConnection(source_id.clone())),
            ),
            crate::chrome::menu::item::<Message>(
                Some(icons::IconKind::Settings),
                "编辑",
                Message::Database(database::Message::EditSourceStart(source_id.clone())),
            ),
            crate::chrome::menu::item::<Message>(
                Some(icons::IconKind::Trash),
                "删除",
                Message::Database(database::Message::DeleteSourceRequest(source_id.clone())),
            ),
        ];
        items.push(if expanded {
            crate::chrome::menu::item::<Message>(
                Some(icons::IconKind::RotateCw),
                "刷新",
                Message::Database(database::Message::SchemaRefresh(source_id.clone())),
            )
        } else {
            crate::chrome::menu::item_locked(Some(icons::IconKind::RotateCw), "刷新", dim)
        });

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::chrome::menu::shell_frosted(
                items,
                Length::Fixed(byteui::theme::geometry::menu_item_width()),
            );
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: menu.y,
                left: menu.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }

    /// 输入框右键菜单浮层:固定四项(剪切/复制/粘贴/全选),定位坐标复用
    /// `files.last_right_click`(main.rs 任意右键都会先写入,`TextInputMenuOpen`
    /// 已用它填好 `x/y`)。动作消息回 main.rs——由它合成回 ⌘/Ctrl+`x`/`c`/`v`/`a`
    /// 键盘事件作用到被右键的输入。密码框(`secure`)禁用剪切/复制(置灰)。
    pub(crate) fn text_input_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.text_input_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let dim = byteui::theme::color::current().dim;
        let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            Vec::new();
        if menu.target.secure {
            // 密码框:复制/剪切同原生快捷键一样被禁用,置灰不可点。
            items.push(crate::chrome::menu::item_locked(
                Some(icons::IconKind::Scissors),
                "剪切",
                dim,
            ));
            items.push(crate::chrome::menu::item_locked(
                Some(icons::IconKind::Copy),
                "复制",
                dim,
            ));
        } else {
            items.push(crate::chrome::menu::item(
                Some(icons::IconKind::Scissors),
                "剪切",
                Message::TextInputMenuCut,
            ));
            items.push(crate::chrome::menu::item(
                Some(icons::IconKind::Copy),
                "复制",
                Message::TextInputMenuCopy,
            ));
        }
        items.push(crate::chrome::menu::item(
            Some(icons::IconKind::ClipboardPaste),
            "粘贴",
            Message::TextInputMenuPaste,
        ));
        items.push(crate::chrome::menu::item(
            Some(icons::IconKind::SelectAll),
            "全选",
            Message::TextInputMenuSelectAll,
        ));

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::chrome::menu::shell_frosted(
                items,
                Length::Fixed(byteui::theme::geometry::menu_item_width()),
            );
        // 常规右键菜单就地向下/向上弹即可,这里输入框多用在面板内容区,直接
        // 以光标为左上锚弹出(必要时可在下方再夹窗口高度,留待需要时加)。
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(iced_widget::core::alignment::Vertical::Top)
            .padding(Padding {
                top: menu.y,
                left: menu.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }
}
