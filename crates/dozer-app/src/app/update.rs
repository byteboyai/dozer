//! `App::update` 消息分发 + 全部 handler 方法。Phase 3 结构重组时从
//! `app/app.rs` 拆出,逻辑保持原样。

use crate::chrome::homespace::{self, load_home_recents};
use crate::chrome::rail;
use crate::chrome::tab_widget;
use crate::extensions::browser;
use crate::extensions::codehealth;
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
use crate::extensions::toast;
use crate::extensions::todo;
use crate::extensions::usage;
use crate::panel_host::ActivationCtx;
use crate::term::terminal;
use crate::workspace::{
    RestorePayload, SshOut, TabAttachedArgs, TabBackend, Workspace, exited_marker,
    spawn_disk_usage_refresh, spawn_project_git_refresh, tab_display_width, tab_title,
};
use byteui::interaction::icons;
use dozer_core::protocol::{BookmarkInfo, ProjectInfo};
use iced_widget::core::{Element, Length, Padding};
use iced_widget::{column, container};
use std::path::PathBuf;

use super::*;

dozer_core::scope!(LOG, module, "shell");
dozer_core::scope!(PREVIEW_LOG, module, "preview");

/// 由工作区的项目信息造切入上下文;工作区还没有项目时返回 `None`。
fn workspace_activation_ctx<M: Send + 'static>(
    ws: &Workspace,
    io: &crate::workspace::ShellIo,
    wrap: fn(M) -> Message,
) -> Option<ActivationCtx<M>> {
    let project = ws.project.as_ref()?;
    Some(ActivationCtx {
        project_id: project.id,
        project_path: Some(PathBuf::from(&project.path)),
        io: io.panel_io(wrap),
    })
}

impl App {
    pub fn update(&mut self, message: Message) {
        self.update_inner(message);
        // 放在包装里而不是 `match` 末尾:`update_inner` 的许多分支会提前 `return`,
        // 放在 `match` 里会漏排空。
        self.drain_outboxes();
        self.drain_host_requests();
    }

    /// 排空各面板 state 的 `HostOutbox` 并执行(系统对话框、跨面板命令)。先把所有需求收出来再执行:
    /// 执行过程会再进 `self.update`(回投面板消息),不能在持有 workspace 借用时做。
    fn drain_host_requests(&mut self) {
        let mut files_requests = Vec::new();
        for slot in self.projects.values_mut() {
            if let WorkspaceSlot::Loaded(ws) = slot {
                files_requests.extend(ws.files.host.take());
            }
        }
        let create_requests = self
            .project_create
            .as_mut()
            .map(|s| s.host.take())
            .unwrap_or_default();
        self.run_host_requests(files_requests, Message::Files);
        self.run_host_requests(create_requests, Message::ProjectCreate);
    }

    /// 执行一批面板需求;`wrap` 把提需求的面板自己的 `Message` 包成宿主 `Message`。
    fn run_host_requests<M: Send + 'static>(
        &mut self,
        requests: Vec<crate::panel_host::HostRequest<M>>,
        wrap: fn(M) -> Message,
    ) {
        use crate::panel_host::HostRequest;
        for request in requests {
            match request {
                HostRequest::PickDirectory { start, on_picked } => {
                    let mut dialog = rfd::FileDialog::new();
                    if let Some(start) = start {
                        dialog = dialog.set_directory(start);
                    }
                    if let Some(dir) = dialog.pick_folder() {
                        self.update(wrap(on_picked(dir)));
                    }
                }
                HostRequest::Command(command) => self.run_panel_command(command),
            }
        }
    }

    /// 跨面板命令的唯一派发点:面板只说"想让别人做什么",由这里决定谁来做。
    fn run_panel_command(&mut self, command: crate::panel_host::PanelCommand) {
        use crate::panel_host::PanelCommand;
        match command {
            PanelCommand::SearchIn { path, is_dir } => {
                let scope = if is_dir {
                    search::Scope::Dir(path)
                } else {
                    search::Scope::File(path)
                };
                self.update(Message::Search(search::Message::SearchOpen(scope)));
            }
            PanelCommand::ShowFileHistory { path } => self.show_file_history(path),
            PanelCommand::OpenFile { path } => self.preview_open_path(path),
        }
    }

    /// 解析出仓库相对路径、组出 `FileHistoryTarget`、异步跑一次 `build()`。
    fn show_file_history(&mut self, path: PathBuf) {
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
                file_history::build(&repo_path2, &file_path2, file_history::DEFAULT_MAX_COUNT)
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            let _ = proxy.send_event(Message::FileHistory(file_history::Message::SnapshotLoaded(
                repo_path, file_path, result,
            )));
        });
    }

    /// 排空所有 extension state 的 outbox 并推成 Toast(extension 的 `update`
    /// 拿不到 `App`,失败提示先写进自己 state 的 outbox)。遍历已加载工作区的
    /// 代价只是几个 `Vec::is_empty`,可以每条消息都做。
    fn drain_outboxes(&mut self) {
        let mut pending: Vec<toast::Pending> = Vec::new();
        for slot in self.projects.values_mut() {
            if let WorkspaceSlot::Loaded(ws) = slot {
                pending.extend(
                    ws.files
                        .take_outbox()
                        .into_iter()
                        .map(|n| toast::pending_from_notice(files::LOG, n)),
                );
                pending.extend(
                    ws.todo
                        .take_outbox()
                        .into_iter()
                        .map(|n| toast::pending_from_notice(todo::LOG, n)),
                );
                // 只写日志、不弹 Toast 的诊断(例如 dozerd 不可用时的静默读取失败)。
                for line in ws.todo.take_diagnostics() {
                    dozer_core::log_debug!(todo::LOG, "{line}");
                }
                pending.extend(
                    ws.project_panel
                        .take_outbox()
                        .into_iter()
                        .map(|n| toast::pending_from_notice(project::LOG, n)),
                );
                pending.extend(
                    ws.conversations
                        .take_outbox()
                        .into_iter()
                        .map(|n| toast::pending_from_notice(conversations::LOG, n)),
                );
                pending.extend(ws.codehealth.take_outbox());
                pending.extend(
                    ws.group_chat
                        .take_outbox()
                        .into_iter()
                        .map(|n| toast::pending_from_notice(crate::extensions::group_chat::LOG, n)),
                );
                pending.extend(ws.ssh.take_outbox());
                pending.extend(ws.database.take_outbox());
                pending.extend(ws.agent_context.take_outbox());
                pending.extend(usage::notices_to_pending(ws.usage.take_outbox()));
            }
        }
        pending.extend(self.database.take_outbox());
        if let Some(settings) = self.settings.as_mut() {
            pending.extend(settings.take_outbox());
        }
        if !pending.is_empty() {
            self.flush_outbox(pending);
        }
    }

    fn update_inner(&mut self, message: Message) {
        match message {
            Message::GitLogDiffWebviewEvent(_binding, event) => {
                if matches!(event.payload, crate::preview::EditorEvent::Ready { .. }) {
                    // 真正的推送发生在 `App::take_git_log_diff_script`(每帧轮询,
                    // 见下),这里只翻状态:webview 刚确认 `__dozer.dispatch`
                    // 已注册,`diff_sent_for` 清空强制下一帧重发一次当前内容
                    // (覆盖"webview 被销毁重建,新实例第一次 ready"的场景)。
                    self.git_log.set_diff_webview_ready(true);
                    self.git_log.clear_diff_sent_for();
                }
                // 其余事件(selection_changed/viewport_changed/...):diff 面板
                // 恒只读、不需要 Agent 跳转/保存,忽略即可。
            }
            Message::FileHistoryDiffWebviewEvent(_binding, event) => {
                if matches!(event.payload, crate::preview::EditorEvent::Ready { .. })
                    && let Some(s) = self.file_history.as_mut()
                {
                    s.set_diff_webview_ready(true);
                }
                // 其余事件忽略,同 GitLogDiffWebviewEvent 的处理。
            }
            Message::EditHistory(msg) => self.edit_history_message(msg),
            Message::EditHistoryDiffWebviewEvent(_binding, event) => {
                if matches!(event.payload, crate::preview::EditorEvent::Ready { .. })
                    && let Some(s) = self.edit_history.as_mut()
                {
                    s.set_diff_webview_ready(true);
                }
            }
            Message::UsageContentWebviewEvent(event, surface) => {
                // 先经共享宿主路由层校验归属:旧代数/未知来源的事件一律丢弃
                // (webview 已销毁重建后,旧实例的在途事件不能写进新实例)。
                if !self.usage_surface.accepts(surface) {
                    dozer_core::log_warn!(
                        LOG,
                        panel = "usage",
                        "拒绝非当前代数的 usage-content IPC"
                    );
                } else if matches!(event, crate::extensions::usage::UsageWebviewEvent::Ready) {
                    // 内置面板走 Ready 握手:推进到 Ready 后下一帧会重发当前内容
                    // (覆盖"webview 被销毁重建,新实例第一次 ready"的场景)。
                    self.usage_surface.apply_ready();
                }
            }
            Message::CodeHealthContentWebviewEvent(event, surface) => {
                use crate::extensions::codehealth::CodeHealthWebviewEvent as Ev;
                // 先经共享宿主路由层校验归属:旧代数/未知来源的事件一律丢弃
                // (webview 已销毁重建后,旧实例的在途事件不能写进新实例)。
                if !self.codehealth_surface.accepts(surface) {
                    dozer_core::log_warn!(
                        LOG,
                        panel = "code_health",
                        "拒绝非当前代数的 codehealth-content IPC"
                    );
                } else {
                    match event {
                        Ev::Ready => self.codehealth_surface.apply_ready(),
                        Ev::ScanRequested => self.code_health_request_scan(),
                        Ev::AnalyzeFinding { id } => self.code_health_analyze_finding(id),
                        Ev::OpenLocation { path, line } => {
                            // webview 内容不可信:拒绝绝对路径与 `..`。
                            if codehealth::is_safe_relative_path(&path) {
                                self.code_health_open_location(path, line);
                            } else {
                                dozer_core::log_warn!(
                                    LOG,
                                    panel = "code_health",
                                    path = %path.display(),
                                    "拒绝 webview 发来的非相对路径跳转"
                                );
                            }
                        }
                        Ev::Failed { reason } => {
                            dozer_core::log_warn!(
                                LOG,
                                panel = "code_health",
                                %reason,
                                "代码健康度内容页渲染失败,回落原生占位"
                            );
                            self.codehealth_webview.set_failed(reason);
                        }
                    }
                }
            }
            Message::TodoContentWebviewEvent(event) => self.todo_content_event(event),
            Message::ReviewTraceWebviewEvent(_event) => {
                self.conversations_message(None, conversations::Message::ReviewTraceLoaded)
            }
            Message::Host(host_event) => self.host_event(host_event),
            Message::Preview(kind, msg) => self.preview_ui(kind, msg),
            Message::PreviewResult(project_id, kind, msg) => {
                self.preview_result(project_id, kind, msg)
            }
            Message::PreviewCommandsFetched(project_id, commands) => {
                self.with_project(project_id, move |ws, io| {
                    for cmd in commands {
                        let outcome = ws.apply_preview_command(&cmd, io);
                        let client = io.client.clone();
                        io.handle.spawn(async move {
                            if let Err(e) = client.report_preview_command_outcome(outcome).await {
                                dozer_core::log_warn!(PREVIEW_LOG, %e, "回报预览命令结果失败");
                            }
                        });
                    }
                });
            }
            Message::Agent(project_id, msg) => {
                self.agent_panel_message(project_id, msg);
            }
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
            Message::Conversations(msg) => self.conversations_message(None, msg),
            Message::ProjectDeleteDone {
                project_id,
                deregistered,
                errors,
            } => {
                if !errors.is_empty() {
                    self.push_toast(
                        LOG,
                        toast::Level::Warning,
                        dozer_model::LocalizedText::keyed(
                            crate::i18n::HOST_OWNER,
                            "toast.delete_projects_incomplete",
                            "Some projects could not be deleted: {errors}",
                        )
                        .with_arg("errors", errors.join("; ")),
                    );
                }
                // 登记一取消就把这个项目从所有 recent 列表里剪掉——
                // `recent_projects` 只是 `list_projects()` 的快照,删除流程
                // 只改了 dozerd 的 DB,不刷这里的话,首页项目卡/顶栏"＋"菜单/
                // 未打开项目占位列表会继续留着一条已删除的死条目(点它会
                // 报"打开项目失败")。
                if deregistered {
                    self.prune_recent_projects(project_id);
                }
            }
            Message::Usage(
                msg @ (usage::Message::Loaded(project_id, ..)
                | usage::Message::LoadFailed(project_id, ..)),
            ) => {
                self.with_project(project_id, move |ws, _io| {
                    usage::update(&mut ws.usage, msg);
                });
            }
            // `ToggleListCollapse`/`CollapseHover` 不再经 `Message::Usage` 上桌:
            // 面板 view 的 `[adapter::PanelDisplay]`(见 extensions/usage.rs)直接把
            // 折叠按钮点击/悬停映射成顶层 `Message::TogglePanelListCollapse` /
            // `Message::Hover`,不进本分支。库 `update` 对这两个变体保 `unreachable!`
            // 维持 match 穷尽。
            Message::Usage(msg) => {
                self.with_focused_project(|ws, _io| {
                    usage::update(&mut ws.usage, msg);
                });
            }
            Message::Toast(msg) => {
                // 单变体枚举,模式不可反驳;先写日志再入队(与 `emit_toast` 同一个
                // `log_toast`)。
                let toast::Message::Push {
                    scope,
                    level,
                    ref text,
                    ..
                } = msg;
                toast::log_toast(scope, level, text.clone());
                toast::update(&mut self.toast, msg, std::time::Instant::now());
            }
            Message::CodeHealth(
                msg @ (codehealth::Message::Loaded(..)
                | codehealth::Message::LoadFailed(..)
                | codehealth::Message::AsyncResult(..)),
            ) => {
                let project_id = msg.result_project().expect("codehealth result project");
                self.with_project(project_id, move |ws, _io| {
                    codehealth::update(&mut ws.codehealth, msg);
                });
            }
            Message::CodeHealth(msg @ codehealth::Message::Scanned(project_id, ..)) => {
                self.with_project(project_id, move |ws, _io| {
                    codehealth::update(&mut ws.codehealth, msg);
                });
            }
            Message::CodeHealth(codehealth::Message::ScanRequested) => {
                self.code_health_request_scan();
            }
            Message::CodeHealth(codehealth::Message::ContentRetry) => {
                self.codehealth_webview.clear_failed();
            }
            Message::CodeHealth(msg) => {
                self.with_focused_project(|ws, _io| {
                    codehealth::update(&mut ws.codehealth, msg);
                });
            }
            Message::SelectTab(idx) => self.select_tab(idx),
            Message::TermTabCloseConfirm => {
                let mut closed = None;
                self.with_focused_project(|ws, io| {
                    // 按 id 现查当前下标,不信打开确认框那一刻存的下标——
                    // 弹窗展示期间主窗口仍可交互,tab 列表可能已经变了
                    // (见 `pending_close_tab` 字段文档)。查不到说明这个
                    // 会话已经通过别的路径被关掉,安全地什么都不做,好过
                    // 用陈旧下标关掉列表里当前占着那个位置的另一个会话。
                    if let Some(id) = ws.pending_close_tab.take()
                        && let Some(idx) = ws.tabs.iter().position(|t| t.info.id == id)
                    {
                        closed = Some((idx, ws.tabs.len()));
                        ws.close_tab(io, idx);
                        ws.ensure_project_terminal(io);
                    }
                });
                if let Some((idx, old_len)) = closed {
                    self.dehover_after_tab_close(
                        HoverId::TermTabItem,
                        HoverId::TermTabClose,
                        idx,
                        old_len,
                    );
                }
            }
            Message::TermTabCloseCancel => {
                self.with_focused_project(|ws, _io| {
                    ws.pending_close_tab = None;
                });
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
                    // 激活页签判定与 `project_tabs_row` 同一套互斥口径。
                    let active_project_id = (self.current_page == AppPage::Workspace)
                        .then_some(self.active_project_id)
                        .flatten();
                    let items = crate::chrome::topbar::project_add_menu_items(
                        &self.recent_projects,
                        &open_ids,
                        active_project_id,
                        self.locale.active_locale(),
                    );
                    if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                        // 经事件循环代理回送选择结果,使其重新走 `Runner::dispatch`
                        // ——rfd 文件夹选择器、剪贴板等原生副作用只在 `dispatch`
                        // 拦截层启动。直接 `self.update(msg)` 会绕过该层,导致
                        // 「打开项目」(`ProjectTabPickFolder`)静默失效(macOS 原生
                        // 菜单同步返回、不经 iced 事件管线)。自洽型菜单项
                        // (如「创建项目」)经 `dispatch` 同样正常。
                        let _ = self.proxy.send_event(msg);
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
                    database::DatabaseTabHoverTarget::Title => {
                        HoverId::tab_item(PanelKind::Database, idx as u64)
                    }
                    database::DatabaseTabHoverTarget::Close => {
                        HoverId::tab_close(PanelKind::Database, idx as u64)
                    }
                };
                self.set_hover(id, hovered);
            }
            Message::Database(database::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(crate::app::TextInputTarget {
                    id: target.id,
                    secure: target.secure,
                }));
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
                        .map(|ws| {
                            database::tab_overflow_items(&ws.database, self.locale.active_locale())
                        })
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
                todo::Message::Hover(slot, h) => self.set_hover(todo::hover_id_of(slot), h),
                todo::Message::TextInputMenuOpen(target) => {
                    self.update(Message::TextInputMenuOpen(todo::text_target_of(target)));
                }
                todo::Message::ToggleListCollapse => {
                    self.toggle_panel_list_collapse(PanelKind::Todo);
                }
                todo::Message::ContentRetry => self.todo_webview.clear_failed(),
                todo::Message::CategoryContextMenuOpen(id) => {
                    self.todo_category_context_menu(id);
                }
                // 分类行被按下:内核拦截以取 `last_cursor`(同文件树
                // `TreeRowPress` 的做法),武装 `Pending`。之后是否确认、松手
                // 收尾、Esc 取消都交给窗口事件与 `todo::update`。
                todo::Message::CategoryRowPress(id) => {
                    let press_pos = self.last_cursor;
                    self.with_focused_project(|ws, _io| {
                        ws.todo
                            .arm_category_drag(id, press_pos, std::time::Instant::now());
                    });
                }
                // 以下几种分类动作都是从右键菜单里点出来的:先关掉菜单本
                // 身,镜像 `files.rs::RenameStart` 落盘动作时
                // `app_state.context_menu = None;` 的既有口径。
                todo::Message::CategoryNewChild(_)
                | todo::Message::CategoryNewSibling(_)
                | todo::Message::CategoryDelete(_)
                | todo::Message::CategoryRenameStart(_)
                | todo::Message::CategoryMoveSibling(_, _) => {
                    self.category_context_menu = None;
                    self.todo_message(msg);
                }
                other => self.todo_message(other),
            },
            Message::GroupChatContentWebviewEvent(event) => self.on_group_chat_webview_event(event),
            Message::GroupChatShell(crate::extensions::group_chat::ShellMessage::Retry) => {
                self.group_chat_webview.clear_failed();
            }
            Message::AppHost(msg) => self.app_host_update(msg),
            Message::AppStoreRemoval(id, outcome) => self.app_store_removal(&id, outcome),
            Message::GroupChat(msg) => {
                let project_id = msg.project_id();
                self.with_project(project_id, |ws, io| {
                    let pio = io.panel_io(Message::GroupChat);
                    for effect in crate::extensions::group_chat::update(&mut ws.group_chat, msg) {
                        crate::extensions::group_chat::run_effect(
                            &ws.group_chat.requests(),
                            effect,
                            project_id,
                            &pio,
                        );
                    }
                });
            }
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
                self.update(Message::TextInputMenuOpen(crate::app::TextInputTarget {
                    id: target.id,
                    secure: target.secure,
                }));
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
                    let state = self.shell_state();
                    self.dims = apply_row_drag(state, divider, window_height, logical_y);
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
            Message::PanelSelect(v) => self.panel_select(v),
            Message::ToggleFileTreeCollapse => self.toggle_files_tree_collapse(),
            Message::TogglePanelListCollapse(kind) => self.toggle_panel_list_collapse(kind),
            Message::TextInputMenuOpen(target) => {
                #[cfg(target_os = "macos")]
                {
                    let (x, y) = self.files.last_right_click();
                    let items = text_input_menu_items(&target, self.locale.active_locale());
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
                browser::update(
                    &mut self.home_browser,
                    msg,
                    None,
                    &crate::host_adapters::services::services_for(&client).bookmark,
                    &handle,
                    emit,
                );
            }
            Message::BrowserTitle(id, title) => {
                let msg = browser::Message::TitleLoaded(id, title);
                if self.is_home() {
                    // 首页全局浏览器:webview 属于 `home_browser`,直接落地。
                    browser::update(
                        &mut self.home_browser,
                        msg,
                        None,
                        &crate::host_adapters::services::services_for(&self.client).bookmark,
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
                            &crate::host_adapters::services::services_for(&io.client).bookmark,
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
                        &crate::host_adapters::services::services_for(&self.client).bookmark,
                        &self.handle,
                        |_| {},
                    );
                } else {
                    self.with_focused_project(|ws, io| {
                        browser::update(
                            &mut ws.browser,
                            msg,
                            ws.project.as_ref().map(|p| p.id),
                            &crate::host_adapters::services::services_for(&io.client).bookmark,
                            &io.handle,
                            |_| {},
                        );
                    });
                }
            }
            Message::BrowserFindShortcut => {
                // 浏览器没在显示就忽略(菜单项只在浏览器的 webview 有焦点时启用,这里再兜一层)。
                let visible = self.is_home()
                    || self.left_view == PanelKind::Web
                    || self.right_view == PanelKind::Web;
                if visible && let Some(state) = self.active_browser_state_mut() {
                    state.open_find_bar();
                }
            }
            Message::BrowserNewWindow(url) => {
                let msg = browser::Message::OpenUrl(url);
                if self.is_home() {
                    browser::update(
                        &mut self.home_browser,
                        msg,
                        None,
                        &crate::host_adapters::services::services_for(&self.client).bookmark,
                        &self.handle,
                        |_| {},
                    );
                } else {
                    self.with_focused_project(|ws, io| {
                        browser::update(
                            &mut ws.browser,
                            msg,
                            ws.project.as_ref().map(|p| p.id),
                            &crate::host_adapters::services::services_for(&io.client).bookmark,
                            &io.handle,
                            |_| {},
                        );
                    });
                }
            }
            Message::Noop => {}
            Message::TermScroll(target, delta) => {
                if target == terminal::TermTarget::Shared {
                    if let Some(project_id) = self.active_project_id() {
                        self.agent_panel_message(
                            project_id,
                            dozer_panel_agent::update::Message::Scroll(delta),
                        );
                    }
                    return;
                }
                self.with_focused_project(|ws, _io| {
                    let tab = ws.ssh_active_tab_mut();
                    if let Some(tab) = tab {
                        tab.model.scroll_display(delta);
                    }
                });
            }
            Message::TermTabOverflowDismiss => {
                self.with_focused_project(|ws, _io| {
                    ws.term_tab_overflow_anchor = None;
                });
            }
            Message::TermSelStart {
                target,
                col,
                row,
                right,
            } => {
                if target == terminal::TermTarget::Shared {
                    if let Some(project_id) = self.active_project_id() {
                        self.agent_panel_message(
                            project_id,
                            dozer_panel_agent::update::Message::SelStart { col, row, right },
                        );
                    }
                    return;
                }
                self.with_focused_project(|ws, _io| {
                    let tab = ws.ssh_active_tab_mut();
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
                if target == terminal::TermTarget::Shared {
                    if let Some(project_id) = self.active_project_id() {
                        self.agent_panel_message(
                            project_id,
                            dozer_panel_agent::update::Message::SelUpdate { col, row, right },
                        );
                    }
                    return;
                }
                self.with_focused_project(|ws, _io| {
                    let tab = ws.ssh_active_tab_mut();
                    if let Some(tab) = tab {
                        tab.model.selection_update(col, row, right);
                    }
                });
            }
            Message::TermPaste(target, text) => self.term_paste(target, text),
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
                self.update(Message::TextInputMenuOpen(crate::app::TextInputTarget {
                    id: target.id,
                    secure: target.secure,
                }));
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
                #[cfg(target_os = "macos")]
                {
                    // macOS:版本选择菜单直接弹原生 NSMenu(与 tab 组下拉/
                    // 文件树分支菜单同款,2026-09-28),选中即转 `BranchSwitch`
                    // 走下方既有 checkout 回路。原生菜单是同步模态的,没有
                    // "先弹层、列表异步落地再填充"的窗口,故首次展开(分支
                    // 列表还没缓存)时直接在主线程同步查一次本地分支——纯
                    // 本地 git refs 读取,毫秒级,可接受。
                    if self.git_log.branches_is_empty()
                        && let Some(repo_path) = self
                            .active_workspace()
                            .and_then(|ws| ws.active_project_path())
                    {
                        let (branches, dirty) = git_log::load_branch_picker_data(&repo_path);
                        git_log::update(
                            &mut self.git_log,
                            git_log::Message::BranchesLoaded(repo_path, branches, dirty),
                            &self.handle,
                            |_| {},
                        );
                    }
                    let (head_branch, branches, dirty) = self.git_log.branch_picker_snapshot();
                    let last_cursor = self.last_cursor;
                    let items = crate::workspace::branch_picker_items(
                        head_branch.as_deref(),
                        &branches,
                        dirty,
                        |name| Message::GitLog(git_log::Message::BranchSwitch(name)),
                    );
                    if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    // 非 macOS:iced 弹层兜底——先把"展开"状态位落地(纯状态
                    // 机部分仍走 update),首次展开且还没缓存过分支列表时,
                    // 顺带异步查一次本地分支。
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
                                git_log::load_branch_picker_data(&repo_path2)
                            })
                            .await
                            .unwrap_or_default();
                            emit(git_log::Message::BranchesLoaded(repo_path, branches, dirty));
                        });
                    }
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
                        bytegit::Repo::discover(&repo_path2)
                            .and_then(|repo| repo.checkout_branch(&name))
                            .map_err(|e| {
                                crate::i18n::panel_keyed_args(
                                    git_log::GIT_LOG_OWNER,
                                    "error.branch_switch_failed",
                                    "Failed to switch branch: {err}",
                                    &[("err", e.message())],
                                )
                            })
                    })
                    .await
                    .unwrap_or_else(|e| {
                        Err(crate::i18n::panel_keyed_args(
                            git_log::GIT_LOG_OWNER,
                            "error.branch_switch_failed",
                            "Failed to switch branch: {err}",
                            &[("err", &e.to_string())],
                        ))
                    });
                    let _ = proxy
                        .send_event(Message::GitLog(git_log::Message::BranchSwitchDone(result)));
                });
            }
            Message::GitLog(git_log::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(crate::app::TextInputTarget {
                    id: target.id,
                    secure: target.secure,
                }));
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
                file_history::update(
                    &mut self.file_history,
                    msg,
                    self.locale.active_locale(),
                    &handle,
                    emit,
                );
            }
            Message::AgentContext(project_id, msg) => self.agent_context_message(project_id, msg),
            Message::ProjectCreateOpen => {
                self.project_create = Some(project_create::State::default());
            }
            Message::ProjectCreate(project_create::Message::GoToSettings) => {
                self.project_create = None;
                self.open_settings();
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
                let project = crate::host_adapters::services::services_for(&self.client).project;
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let locale = self.locale.active_locale().to_owned();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::ProjectCreate(m));
                };
                project_create::update(
                    &mut self.project_create,
                    msg,
                    &locale,
                    project,
                    &handle,
                    emit,
                );
            }
            Message::Files(files::Message::CopyPath(path, kind)) => {
                let _ = (path, kind); // main.rs 拦截处理写剪贴板,这里维持现状空分支
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
                let locale = self.locale.active_locale().to_owned();
                handle.spawn(async move {
                    let repo_path2 = repo_path.clone();
                    let file_path2 = file_path.clone();
                    let path2 = path.clone();
                    let locale_task = locale.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        file_history::previous_commit(&repo_path2, &file_path2).and_then(|opt| {
                            match opt {
                                Some(oid) => file_history::rollback_to(
                                    &repo_path2,
                                    &file_path2,
                                    oid,
                                    &locale_task,
                                ),
                                None => Err(crate::i18n::host_text(
                                    "file_history.no_version_to_rollback",
                                    "This file has no version to roll back to",
                                    &locale_task,
                                )),
                            }
                        })
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let _ = proxy.send_event(Message::Files(
                        files::Message::FileHistoryRollbackDone(project_id, path2, result),
                    ));
                });
            }
            Message::Files(
                msg @ (files::Message::StatusesRefreshed(project_id, ..)
                | files::Message::PasteDone(project_id, ..)
                | files::Message::DeleteApplied(project_id, ..)
                | files::Message::OpDone { project_id, .. }
                | files::Message::GitInfoLoaded(project_id, ..)
                | files::Message::BranchSwitchDone(project_id, ..)
                | files::Message::GitInitDone(project_id, ..)
                | files::Message::FileDropDone(project_id, ..)
                | files::Message::OpenWithDefaultDone(project_id, ..)),
            ) => self.files_project_message(project_id, msg),

            Message::Files(files::Message::TabReloadFromDisk(path)) => {
                // 预览 tab 右键"从磁盘重新加载":先收起菜单,再在两个预览面板里
                // 按路径找出对应 tab,调 `PreviewPane::bump_reload` 重建编辑器/
                // 推进 webview 的 `reload_nonce` 让 webview 重新 `load_url` 读盘
                // 最新内容(原生 editor 档重读磁盘、wry 档换 URL 重载,见
                // `preview.rs::PreviewPane::bump_reload` 文档)。路径同时命中两
                // 个面板的概率极低(同一项目),找到即止。
                self.files.close_context_menu();
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                if bump_reload_for_path(&mut ws.preview, &path) {
                    return;
                }
                bump_reload_for_path(&mut ws.project_preview, &path);
            }

            Message::Files(files::Message::TabContextMenuCloseTab(_kind, path)) => {
                // 预览 tab 右键"关闭":先收起菜单,再在两个预览面板里按路径定位
                // 出对应 tab,调 `PreviewPane::close` 把它关掉(原生 editor 档释放
                // 编辑器、wry 档回收 webview,见 `preview.rs::PreviewPane::close`
                // 文档)。路径同时命中两个面板的概率极低(同一项目),找到即止。
                self.files.close_context_menu();
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                if let Some(idx) = ws.preview.find_existing_file_tab(&path) {
                    ws.preview.close(idx);
                    return;
                }
                if let Some(idx) = ws.project_preview.find_existing_file_tab(&path) {
                    ws.project_preview.close(idx);
                }
            }

            Message::Files(files::Message::ToolbarHover(target, hovered)) => {
                // 文件树工具行 icon 按钮的 hover:本面板不挂 App 的 hover 动画
                // 表,把进入/离开转发成 `HoverId` 由内核统一驱动动画进度。
                let id = match target {
                    files::FilesToolbarTarget::SearchSubmit => {
                        HoverId::search_submit(PanelKind::Files)
                    }
                    files::FilesToolbarTarget::Dotfiles => {
                        HoverId::named(PanelKind::Files, "dotfiles")
                    }
                    files::FilesToolbarTarget::BranchSwitch => {
                        HoverId::named(PanelKind::Files, "branch_switch")
                    }
                };
                self.set_hover(id, hovered);
            }
            Message::Files(files::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(crate::app::TextInputTarget {
                    id: target.id,
                    secure: target.secure,
                }));
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
            Message::Files(files::Message::TreeDragRelease { shift, command }) => {
                let confirmed = self
                    .active_workspace()
                    .is_some_and(|ws| ws.files.tree_drag_confirmed());
                self.update(Message::Files(files::Message::TreeDragEnd {
                    confirmed,
                    shift,
                    command,
                }));
            }
            // 树行双击:目录复用 `Message::Toggle` 那条本地消息直接切换展开
            // 态(同点箭头效果),文件跨到 `PreviewOpenPath`——该面板本身
            // 不认识这条消息,见 `files::Message::TreeRowDoubleClick` 文档。
            Message::Files(files::Message::TreeRowDoubleClick { path, is_dir }) => {
                if is_dir {
                    self.update(Message::Files(files::Message::Toggle(path)));
                } else {
                    self.preview_open_path(path);
                }
            }
            #[cfg(target_os = "macos")]
            Message::Files(files::Message::BranchPickerOpen) => {
                // macOS:文件树 git 底栏的版本选择菜单直接弹原生 NSMenu(与
                // tab 组下拉/git log 面板同款,2026-09-28)——`files::update`
                // 的 app_state 够不到内核 `last_cursor`,故与
                // `GitLog(BranchPickerOpen)` 一样在内核拦截。选中经
                // `Message::Files(BranchSwitch)` 走既有 checkout 回路。iced
                // 弹层只在非 macOS 兜底平台保留。
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let items = crate::workspace::branch_picker_items(
                    ws.files.current_branch.as_deref(),
                    &ws.files.git_branches,
                    ws.files.git_dirty(),
                    files::Message::BranchSwitch,
                );
                if let Some(msg) = crate::chrome::native_menu::show(items, self.last_cursor) {
                    self.update(Message::Files(msg));
                }
            }
            Message::Files(files::Message::RequestSendToAgentTerminal { is_dir, relative }) => {
                // 文件树右键"添加到 Agent 上下文":`files::update` 算出相对路径
                // 后经 `emit` 送回内核,这里按当前 locale 拼模板文本并写进当前
                // 激活的 agent 终端输入框(真正的 PTY 句柄只有内核有)。
                //
                // 先发起落库(异步),再写终端;落库失败不影响粘贴——上下文条会
                // 显示"已发送到终端,但未能记录到上下文列表"(见
                // `agent_context::apply`)。
                let text = files::agent_context_reference_text(
                    &relative,
                    is_dir,
                    self.locale.active_locale(),
                );
                if let Some(project_id) = self.active_project_id {
                    let context =
                        crate::host_adapters::services::services_for(&self.client).context;
                    let handle = self.handle.clone();
                    let proxy = self.proxy.clone();
                    let Some(ws) = self.active_workspace() else {
                        return;
                    };
                    crate::extensions::agent_context::request_add(
                        &ws.agent_context.requests,
                        project_id,
                        is_dir,
                        relative,
                        context,
                        &handle,
                        move |m| {
                            let _ = proxy.send_event(Message::AgentContext(project_id, m));
                        },
                    );
                }
                self.term_paste(terminal::TermTarget::Shared, text);
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
                let external_apps = self.external_apps.clone();
                let locale = self.locale.active_locale().to_string();
                let app_files = &mut self.files;
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                files::update(
                    &mut ws.files,
                    app_files,
                    msg,
                    project_id,
                    &handle,
                    emit,
                    &external_apps,
                    &dozer_panel_files::adapter::FilesCapabilities::all(),
                    &locale,
                );
                let requests = ws.files.take_requests();
                self.run_files_requests(requests);
            }
            Message::Project(
                msg @ (project::Message::GitRefreshed(project_id, ..)
                | project::Message::NameRenamed(project_id, ..)
                | project::Message::DiskUsageLoaded(project_id, ..)
                | project::Message::ScaffoldStepStarted(project_id, ..)
                | project::Message::ScaffoldStepFinished(project_id, ..)
                | project::Message::TranscriptBackfillStarted(project_id, ..)
                | project::Message::TranscriptBackfillFinished(project_id, ..)
                | project::Message::SummaryBackfillProgress(project_id, ..)
                | project::Message::SummaryBackfillFailed(project_id, ..)),
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
                    project::Message::SummaryBackfillProgress(
                        _,
                        completed,
                        total,
                        ..
                    ) if completed >= total
                );
                let services = crate::host_adapters::services::services_for(&self.client);
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
                    &services,
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
                self.project_preview_open_path(path);
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
                let services = crate::host_adapters::services::services_for(&self.client);
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
                    &services,
                    &handle,
                    emit,
                );
            }
            Message::Project(project::Message::DeleteProjectConfirm) => {
                self.project_delete_confirm();
            }
            Message::Project(project::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(
                    crate::host_adapters::project_panel::text_target_of(target),
                ));
            }
            Message::Project(project::Message::ToolbarHover(target, hovered)) => {
                // Project 面板头部"＋"按钮的 hover:本面板不挂 App 的 hover 动画表,
                // 把进入/离开转发成 `HoverId` 由内核统一驱动动画进度(同
                // `Message::Files(files::Message::ToolbarHover(..))` 的既有先例)。
                let id = match target {
                    project::ProjectToolbarTarget::Docs => {
                        HoverId::named(PanelKind::Project, "docs_add")
                    }
                    project::ProjectToolbarTarget::Remote => {
                        HoverId::named(PanelKind::Project, "remote_add")
                    }
                    project::ProjectToolbarTarget::Memory => {
                        HoverId::named(PanelKind::Project, "memory_add")
                    }
                };
                self.set_hover(id, hovered);
            }
            Message::Project(msg) => {
                let Some(project_id) = msg.result_project().or(self.active_project_id) else {
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
                let services = crate::host_adapters::services::services_for(&self.client);
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
                    &services,
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
                    ws.ssh_tab_first = byteui::interaction::tab_strip::tab_window_reveal(
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
                    ws.ssh_tab_first = byteui::interaction::tab_strip::tab_window_reveal(
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
                            &mut ws.sftp_tabs,
                            io,
                            ssh::sftp::Message::ContextMenuOpen {
                                host_id: host_id.clone(),
                                is_local,
                                path: path.clone(),
                            },
                        );
                    });
                    let (x, y) = self.files.last_right_click();
                    let items = ssh::sftp::context_menu_items(
                        &host_id,
                        is_local,
                        self.locale.active_locale(),
                    );
                    if let Some(msg) = crate::chrome::native_menu::show(items, (x, y)) {
                        self.update(Message::Ssh(ssh::Message::Sftp(msg)));
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.with_focused_project(|ws, io| {
                        ssh::sftp::route(
                            &mut ws.sftp_tabs,
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
                    ssh::sftp::route(&mut ws.sftp_tabs, io, msg);
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
                self.update(Message::TextInputMenuOpen(crate::app::TextInputTarget {
                    id: target.id,
                    secure: target.secure,
                }));
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
            Message::PreviewSelectionMenuOpen {
                project_id,
                panel,
                tab_id,
                path,
                x,
                y,
                range,
                selected_text,
            } => {
                // 闭包外:换算窗口坐标(webview 本地 + webview 原点),判断当前
                // 是否有可见/激活的 agent 终端(无则菜单项置灰,不报错)。
                let side = self.shell_state().layout.rail_layout.side_of(panel);
                let (x0, y0, _, _) = crate::webview_geometry::content_pane_bounds_for(
                    side,
                    self.window_size.0,
                    self.window_size.1,
                    &self.shell_state(),
                );
                let menu = PreviewSelectionContextMenu {
                    project_id,
                    x: x0 + x,
                    y: y0 + y,
                    panel,
                    tab_id,
                    path,
                    range,
                    selected_text,
                    agent_terminal_visible: self.terminal_visible(),
                };
                #[cfg(target_os = "macos")]
                {
                    // 原生菜单阻塞弹出,mac 不存状态,弹完即取返回值。
                    let items = files::preview_selection_context_menu_items(
                        &menu,
                        self.locale.active_locale(),
                    );
                    let pos = (menu.x, menu.y);
                    if let Some(msg) =
                        crate::chrome::native_menu::show_align_no_icon_left(items, pos)
                    {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.preview_context_menu = Some(menu);
                }
            }
            Message::SendSelectionToAgent {
                project_id,
                path,
                range,
                selected_text,
            } => {
                self.preview_context_menu = None;
                // 非 mac 的 iced 弹层不阻塞输入,用户可能在菜单开着期间切到
                // 别的项目标签再点"发送给 Agent"——`project_id` 是建菜单时
                // 捕获的发起项目,与"当前激活"的项目不再一致就直接放弃,不
                // 能把内容送错项目(同 `term_input` 里"看不见的地方不能
                // 敲字"的既有原则)。
                if self.active_project_id != Some(project_id) {
                    return;
                }
                let Some(ws) = self.active_workspace() else {
                    return;
                };
                let Some(tree) = ws.files.file_tree.as_ref() else {
                    return;
                };
                let relative = crate::project::path_string(
                    crate::project::PathKind::Relative,
                    &path,
                    tree.root(),
                );
                let text = files::selection_reference_text(
                    &relative,
                    range,
                    &selected_text,
                    self.locale.active_locale(),
                );
                self.term_paste(terminal::TermTarget::Shared, text);
            }
            Message::PreviewSelectionMenuClose => {
                self.preview_context_menu = None;
            }
            Message::LocaleFetched { seq, outcome } => self.locale_fetched(seq, outcome),
            Message::SettingsOpen => {
                self.open_settings();
            }
            Message::Settings(msg) => {
                // 主题切换要在设置 update(它会 set_scheme 改全局配色)之后,把
                // 所有已开预览 webview 按新主题重载——flyfish 的 theme 是 URL
                // 参数(见 preview::flyfish_url),不重载不会跟着变。两个预览
                // 面板(Files/Project)各推进各自 wry tab 的 reload_nonce。
                let theme_changed = matches!(msg, settings::Message::ThemeSelected(_));
                // 停止/重新启动 dozerd 的结果要顺带更新 `daemon_unavailable`——
                // 这是"daemon 连不上"的整程序共享状态(`app/view.rs`/
                // `term/terminal.rs` 已经在读),不新建 UI 组件(spec「复用
                // App.daemon_unavailable」)。跟 `theme_changed` 一样,要在
                // `msg` 被 move 进 `settings::update` 之前取值。
                let stop_succeeded = matches!(msg, settings::Message::AdvancedStopResult(Ok(())));
                let restart_succeeded =
                    matches!(msg, settings::Message::AdvancedRestartResult(Ok(())));
                // 卸载"连数据一起删":在请求发出前记下意图(id 取自确认步骤),结果回来时据此清数据存储。
                // 意图存在 `App` 上而不是设置状态里,所以确认后立刻关窗也不丢。
                if let settings::Message::Apps(
                    crate::extensions::settings_apps::Message::UninstallConfirmed(
                        bytehost_apps::registry::UninstallMode::ProgramAndData,
                    ),
                ) = &msg
                    && let Some(crate::extensions::settings_apps::Flow::ConfirmUninstall {
                        id, ..
                    }) = self.settings.as_ref().map(|s| &s.apps.flow)
                {
                    self.purge_intents.insert(id.clone());
                }
                let uninstall_result = match &msg {
                    settings::Message::Apps(
                        crate::extensions::settings_apps::Message::ActionDone(
                            id,
                            crate::extensions::settings_apps::ActKind::Uninstall,
                            result,
                        ),
                    ) => Some((id.clone(), result.is_ok())),
                    _ => None,
                };
                // 语言写入结果(i18n Task 05):App 先采纳权威快照(更新
                // 显示语言/广播/清缓存),再交给设置状态机更新它的 UI 状态。
                if let settings::Message::LanguageSaveOutcome(outcome) = &msg {
                    match outcome {
                        settings::LanguageSaveOutcome::Applied(snapshot)
                        | settings::LanguageSaveOutcome::Conflict(snapshot) => {
                            let effect = self.locale.apply_local_write(snapshot.clone());
                            self.locale_cache_save();
                            if effect.active_changed {
                                self.locale_broadcast_change();
                            }
                        }
                        settings::LanguageSaveOutcome::Failed(_)
                        | settings::LanguageSaveOutcome::Unsupported => {}
                    }
                }
                // 用户在语言页点了新偏好:设置状态机做乐观显示,真正的
                // `set_locale` 由下面这段发起(需要权威 revision + 系统候选)。
                let language_selected = match &msg {
                    settings::Message::LanguageSelected(p) => Some(p.clone()),
                    _ => None,
                };
                // 设置窗口已经关了(状态是 `None`)时才到达的安装/停止/卸载结果:状态机不在了,事情照样发生了,
                // 仍要刷新应用宿主、把结果告诉用户(见 `settings_apps::orphan_result_effects`)。
                let orphan_effects = match (&self.settings, &msg) {
                    (None, settings::Message::Apps(m)) => {
                        crate::extensions::settings_apps::orphan_result_effects(m)
                    }
                    _ => Vec::new(),
                };
                let daemon = crate::host_adapters::services::services_for(&self.client).daemon;
                let apps = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Settings(m));
                };
                settings::update(&mut self.settings, msg, daemon, apps, &handle, emit);
                if let Some((id, ok)) = uninstall_result
                    && self.purge_intents.remove(&id)
                    && ok
                {
                    self.purge_app_data_store(&id);
                }
                for effect in orphan_effects {
                    match effect {
                        crate::extensions::settings_apps::Effect::HostChanged => {
                            self.app_host_update(crate::extensions::app_host::Message::Refresh);
                        }
                        crate::extensions::settings_apps::Effect::Notice { level, text, key } => {
                            let level = crate::extensions::app_host::notice_level(level);
                            self.push_toast_keyed(
                                LOG,
                                level,
                                crate::extensions::toast::literal(text),
                                &key,
                            );
                        }
                        _ => {}
                    }
                }
                // 设置里的安装/停止/卸载改了已安装集合或运行状态:立刻刷新应用宿主的列表(同步图标栏)。
                if self
                    .settings
                    .as_mut()
                    .is_some_and(|s| s.take_host_changed())
                {
                    self.app_host_update(crate::extensions::app_host::Message::Refresh);
                }
                // 语言页提交:用权威 revision + 系统候选发起 set_locale;结果经
                // `Message::Settings(LanguageSaveOutcome(..))` 回灌。
                if let Some(pref) = language_selected {
                    self.spawn_locale_write(pref);
                }
                if stop_succeeded {
                    self.daemon_unavailable = Some(crate::app::DaemonUnavailable::Stopped);
                }
                if restart_succeeded {
                    self.daemon_unavailable = None;
                    // 新拉起的 dozerd 是当前版本:丢掉握手缓存,"过旧"徽标随下次
                    // 握手重新判定(不丢的话会一直显示旧结果)。
                    self.client.forget_version_check();
                    // dozerd 刚被重新拉起:立刻拉一次应用列表(同步图标栏),不等下一次轮询。
                    self.app_host_update(crate::extensions::app_host::Message::Refresh);
                    // i18n Task 04:连接换代——重建 revision 基线并立即刷新语言快照。
                    self.on_locale_reconnect();
                }
                if theme_changed {
                    self.with_focused_project(|ws, io| {
                        ws.preview.reload_all_webviews_for_theme();
                        ws.project_preview.reload_all_webviews_for_theme();
                        ws.spawn_pending_plantuml_reloads(PanelKind::Files, io);
                        ws.spawn_pending_plantuml_reloads(PanelKind::Project, io);
                    });
                }
            }
            // WebViewFocused 只在 main.rs 的 dispatch 里设 pending_focus,
            // App::update 无需处理。
            Message::WebViewFocused => {}
            // 同上:只在 main.rs 的 dispatch 里设 pending_focus。
            Message::AppWebViewFocused(_) => {}
            // 应用 webview 导航完成、请求推 locale 快照:副作用(对 webview 句柄
            // `evaluate_script`)在 `window_events::dispatch` 里执行,`App::update` 不处理。
            Message::AppWebViewLocaleReady(_) => {}
            // 鼠标在子 webview 上松开(见 `WebViewMouseUp` 文档):一并结束页签
            // 拖拽,避免"松开还能继续拖"。
            Message::WebViewMouseUp => self.end_tab_drag(),
            // ⌘F 页内查找(见两条消息的文档):原生副作用(设 first
            // responder / 调 WKWebView `findString:`)都在 main.rs 的
            // `dispatch` 里对 webview 句柄执行,`App::update` 无需处理。
            Message::WebViewFindFocus(_) | Message::WebViewFindNative(_) => {}
            // ⌘V 粘贴进子 webview(见消息文档):副作用(读剪贴板 +
            // `evaluate_script`)在 `window_events::dispatch` 里执行。
            Message::WebViewPasteRequest(_) => {}
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
        // (`spawn_new_tab` 的 expect)。失败文案走 Toast——它不依赖任何
        // `Workspace` 存在,一个项目都没打开时也能显示(不再挂
        // `daemon_unavailable`)。
        let Some(project) = project else {
            dozer_core::log_warn!(LOG, "打开项目页签失败,页签集合保持不变");
            self.push_toast_keyed(
                LOG,
                toast::Level::Error,
                dozer_model::LocalizedText::keyed(
                    crate::i18n::HOST_OWNER,
                    "error.open_project_failed",
                    "Could not open the project; make sure dozerd is running and try again",
                ),
                "open-project-failed",
            );
            self.with_focused_project(move |ws, _io| {
                ws.recent_projects = recent;
            });
            return;
        };
        self.daemon_unavailable = None;
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
            // 前台化同样是"换到另一个项目"(focus_project_tab 是唯一一条
            // 换项目的路),Git Log 面板开着时要补同步,理由同
            // `ProjectTabSwitch`。
            if self.git_log_panel_active() {
                self.sync_git_log_to_active_project();
            }
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
        // 全新页签同样过一遍 Git Log 同步(默认布局没开它时是廉价 no-op)。
        if self.git_log_panel_active() {
            self.sync_git_log_to_active_project();
        }
        self.sync_terminal_grid(); // 同上
        self.persist_open_projects();
        // 新开的项目 tab(区别于"已开着、只是前台化"那条 `focus_project_tab`
        // 早退分支):静默跑一次 ensure(README/.dozer/git/agent 历史),不
        // 展示结果(见 project_scaffold 设计"打开即 ensure"一节)。
        let transcript = crate::host_adapters::services::services_for(&self.client).transcript;
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Project(m));
        };
        project::spawn_scaffold_run(repo_path, Some(transcript), &handle, emit, skip_git_init);
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
        // 刷新的 worktree 速览条对不上。两栏都查:GitLog 被拖到右栏后
        // `left_view` 不再是它,只查左栏会漏(用户实测:右栏 Git Log 切
        // 页签后一直显示前一项目的提交)。
        if self.git_log_panel_active() {
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
            // 邻居页签的布局里开着 Git Log 的话,提交图缓存还停在刚被关
            // 掉的那个项目上——同 `ProjectTabSwitch` 的补同步理由。
            if self.git_log_panel_active() {
                self.sync_git_log_to_active_project();
            }
        }
        self.sync_terminal_grid(); // 清放大态后重算网格,理由同 `ProjectTabSwitch`
        self.persist_open_projects();
        // 关掉(或删掉)的是最后一个项目:落到首页而不是"未打开任何项目"的占位。
        self.enter_home_if_no_projects();
    }

    /// 把一个项目从所有"最近项目"镜像列表里剪掉(`App::recent_projects`
    /// 与每个已打开 `Workspace` 各自持有的那份快照)。删除项目后调用,
    /// 保证首页项目卡/顶栏菜单/占位列表不再出现死条目。
    fn prune_recent_projects(&mut self, id: i64) {
        self.recent_projects.retain(|p| p.id != id);
        for slot in self.projects.values_mut() {
            if let WorkspaceSlot::Loaded(ws) = slot {
                ws.recent_projects.retain(|p| p.id != id);
            }
        }
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
        let services = crate::host_adapters::services::services_for(&self.client);
        let (project, transcript) = (services.project, services.transcript);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let on_done = move |project_id, deregistered, errors: Vec<String>| {
            let _ = proxy.send_event(Message::ProjectDeleteDone {
                project_id,
                deregistered,
                errors,
            });
        };
        project::delete::spawn_delete_project(
            project_id, repo_path, scope, project, transcript, &handle, on_done,
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
                .map(|session| session.info.id.clone())
                .collect();
            let task = self.handle.spawn(async move {
                for sid in ids {
                    if let Err(e) = client.kill(&sid).await {
                        dozer_core::log_warn!(LOG, "丢弃过期促成结果时结束会话失败: {e}");
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
        // 恢复出来的工作区变为 Loaded:补拉一次上下文列表,重启后条上不是空的。
        self.agent_context_refresh(id);
    }

    pub(crate) fn project_fs_changed(
        &mut self,
        project_id: ProjectId,
        changes: bytegit::GitChange,
    ) {
        // 工作区类变更:文件树 + 打开的 webview 预览即时跟进。`notify` 递上
        // 的是具体变更路径 `changes.paths`,文件树按"受影响即相关"整棵从盘重
        // 读已缓存目录(`reload_tree_from_disk`,只重读已展开/缓存过的层,开销
        // 小),预览则只重载路径命中的 webview tab。
        if changes.workdir_changed || changes.refs_changed {
            let locale = self.locale.active_locale().to_string();
            self.with_project(project_id, move |ws, io| {
                ws.files.reload_tree_from_disk();
                ws.preview.reload_webviews_for(&changes.paths, &locale);
                ws.project_preview
                    .reload_webviews_for(&changes.paths, &locale);
                // Task 7:根文件/include 变化后重跑受影响的 PlantUML 图,重读
                // include。放在 `reload_webviews_for` 之后立即取走队列。
                ws.spawn_pending_plantuml_reloads(PanelKind::Files, io);
                ws.spawn_pending_plantuml_reloads(PanelKind::Project, io);
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
        if changes.refs_changed
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
        result: Result<(), dozer_model::LocalizedText>,
    ) {
        let locale = self.locale.active_locale().to_string();
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
            &locale,
        );
    }

    pub(crate) fn database_browse_result(
        &mut self,
        project_id: i64,
        tab_id: usize,
        run_seq: u64,
        result: Result<database::BrowsePage, dozer_model::LocalizedText>,
    ) {
        let locale = self.locale.active_locale().to_string();
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
            &locale,
        );
    }

    pub(crate) fn database_query_result(
        &mut self,
        project_id: i64,
        tab_id: usize,
        run_seq: u64,
        result: Result<database::QueryOutcome, dozer_model::LocalizedText>,
    ) {
        let locale = self.locale.active_locale().to_string();
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
            &locale,
        );
    }

    pub(crate) fn database_tables_loaded(
        &mut self,
        project_id: i64,
        source_id: String,
        result: Result<Vec<database::TableRef>, dozer_model::LocalizedText>,
    ) {
        let locale = self.locale.active_locale().to_string();
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
            &locale,
        );
    }

    pub(crate) fn database_columns_loaded(
        &mut self,
        project_id: i64,
        source_id: String,
        schema: Option<String>,
        table: String,
        result: Result<Vec<database::ColumnInfo>, dozer_model::LocalizedText>,
    ) {
        let locale = self.locale.active_locale().to_string();
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
            &locale,
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
        let locale = self.locale.active_locale().to_string();
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
            &locale,
        );
    }

    pub(crate) fn ssh_test_connection_result(
        &mut self,
        project_id: i64,
        host_id: String,
        result: Result<(), dozer_model::LocalizedText>,
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
        err: dozer_model::LocalizedText,
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
        let handle = self.handle.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        let Some(ws) = self.active_workspace() else {
            return;
        };
        ws.todo.requests.invalidate("todos");
        let request = ws.todo.requests.scope(self.active_project_id);
        handle.spawn(async move {
            let res = client
                .assign_todo_agent(id, agent)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = proxy.send_event(Message::Todo(todo::Message::AsyncResult(
                request,
                Box::new(todo::Message::Mutated(res)),
            )));
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
            match client.get_todo_detail(id).await {
                Ok((_, turns)) => {
                    let _ = proxy.send_event(Message::TodoDetailLoaded(idx, turns));
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
            // 失败此前被 `let _ =` 吞掉,随后照常刷新详情,用户看到的是"点了什么都没发生"。
            let processed = client.process_todo_now(id, reply_text.as_deref()).await;
            let failed = processed.is_err();
            if let Some(msg) = toast::failure_message(
                todo::LOG,
                "todo.process_failed",
                "Failed to process task: {e}",
                &processed,
            ) {
                let _ = proxy.send_event(Message::Toast(msg));
            }
            // 无论处理成败都用服务端权威回合列表刷新;刷新本身也失败时,只有在"处理成功"的
            // 情况下才再提示——处理已经失败时不在其上再叠一条(daemon 不通时两个请求会一起失败)。
            match client.get_todo_detail(id).await {
                Ok((_, turns)) => {
                    let _ = proxy.send_event(Message::TodoDetailLoaded(idx, turns));
                }
                Err(e) if !failed => {
                    let _ = proxy.send_event(Message::Toast(toast::Message::Push {
                        scope: todo::LOG,
                        level: toast::Level::Error,
                        text: dozer_model::LocalizedText::keyed(
                            crate::i18n::HOST_OWNER,
                            "toast.todo_detail_refresh_failed",
                            "Could not refresh task details: {error}",
                        )
                        .with_arg("error", e.to_string()),
                        key: None,
                    }));
                }
                Err(_) => {}
            }
        });
    }

    pub(crate) fn todo_message(&mut self, msg: todo::Message) {
        let Some(project_id) = msg.result_project().or(self.active_project_id) else {
            return;
        };
        let service = crate::host_adapters::services::services_for(&self.client).todo;
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        // 异步结果/写确认透过 `proxy` 重发回主循环,回调里会借用 `self`
        // 的 service/handle ——闭包捕获是 move 出来的副本,行得通(同
        // `Message::Search` 分支的既有手法)。
        let emit = move |m: todo::Message| {
            let _ = proxy.send_event(Message::Todo(m));
        };
        self.with_project(project_id, move |ws, _io| {
            todo::update(&mut ws.todo, msg, project_id, &service, &handle, emit);
        });
    }

    /// Todo 内容区 webview 事件入口。`Ready`/`Failed` 在这里直接处理;其余事件交给
    /// 纯函数 `todo::route_event` 把 `id` 映射成当前下标并校验入参,再复用现有的
    /// 下标式入口(`todo_message` / `todo_assign_agent` / `todo_detail_open`)。
    pub(crate) fn todo_content_event(&mut self, event: todo::TodoWebviewEvent) {
        match &event {
            todo::TodoWebviewEvent::Ready => {
                self.todo_webview.set_ready(true);
                return;
            }
            todo::TodoWebviewEvent::Failed { reason } => {
                dozer_core::log_warn!(
                    LOG,
                    panel = "todo",
                    %reason,
                    "Todo 内容页渲染失败,回落原生占位"
                );
                self.todo_webview
                    .set_failed(dozer_model::LocalizedText::literal(
                        crate::i18n::HOST_OWNER,
                        reason.clone(),
                    ));
                return;
            }
            _ => {}
        }
        let routed = self
            .active_workspace()
            .and_then(|ws| todo::route_event(ws.todo.items(), event));
        match routed {
            Some(todo::Routed::Message(msg)) => self.todo_message(msg),
            Some(todo::Routed::AssignAgent { idx, agent }) => self.todo_assign_agent(idx, agent),
            Some(todo::Routed::OpenDetail { idx }) => self.todo_detail_open(idx),
            None => {}
        }
    }

    /// 群聊 webview 事件入口。`Ready`/`Failed` 在这里直接处理;其余事件交给
    /// 纯函数 `group_chat::route_event` 校验并映射成命令,再派发:`SelectGroup`
    /// 同步改状态、`OpenTodo` 显示 Todo 面板,其余异步命令走 `spawn_command`。
    pub(crate) fn on_group_chat_webview_event(
        &mut self,
        event: crate::extensions::group_chat::GroupChatWebviewEvent,
    ) {
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let io = self.panel_io(Message::GroupChat);
        let mut push = std::mem::take(&mut self.group_chat_webview);
        let Some(ws) = self.active_workspace_mut() else {
            self.group_chat_webview = push;
            return;
        };
        let action = crate::extensions::group_chat::host::handle_webview_event(
            &mut ws.group_chat,
            &mut push,
            project_id,
            event,
            &io,
        );
        self.group_chat_webview = push;
        if action == crate::extensions::group_chat::host::HostAction::ShowTodoPanel {
            self.show_panel(PanelKind::Todo);
        }
    }

    fn group_chat_command(&mut self, project_id: i64, cmd: crate::extensions::group_chat::Command) {
        use crate::extensions::group_chat::{self as gc, Command};
        match cmd {
            Command::SelectGroup { group_id } => {
                self.with_project(project_id, |ws, io| {
                    let pio = io.panel_io(Message::GroupChat);
                    gc::select_group(&mut ws.group_chat, project_id, group_id, &pio);
                });
            }
            Command::OpenTodo { .. } => {
                // 只确保 Todo 面板看得见;不做"定位到某条待办"(Todo 面板没有这个入口,
                // 且不在本功能范围)。不能用 `panel_select`:它对已激活的面板是"收起",
                // 还会武装图标栏拖拽。(跨面板动作,留在 host。)
                self.show_panel(PanelKind::Todo);
            }
            // 命令结果绑定发起时的工作区实例;已关闭的项目不再发起命令。
            other => self.with_project(project_id, |ws, io| {
                gc::spawn_command(
                    &ws.group_chat.requests(),
                    project_id,
                    other,
                    &io.panel_io(Message::GroupChat),
                );
            }),
        }
    }

    pub(crate) fn browser_bookmarks_loaded(
        &mut self,
        pid: Option<i64>,
        bookmarks: Result<Vec<BookmarkInfo>, dozer_services::ServiceError>,
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
                &crate::host_adapters::services::services_for(&client).bookmark,
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
                &crate::host_adapters::services::services_for(&client).bookmark,
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
                &crate::host_adapters::services::services_for(&client).bookmark,
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
                &crate::host_adapters::services::services_for(&client).bookmark,
                &handle,
                emit,
            );
        });
    }

    pub(crate) fn browser_message(&mut self, msg: browser::Message) {
        // 按下浏览器页签＝选中＋准备被拖走(`SelectTab` 在
        // `browser::update` 里真正选中为 `active`,这里按它记下拖起源)。
        let was_select = matches!(msg, browser::Message::SelectTab(_));
        let Some(pid) = msg.result_project().or(self.active_project_id) else {
            return;
        };
        self.with_project(pid, |ws, io| {
            let project_id = ws.project.as_ref().map(|p| p.id);
            let client = io.client.clone();
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(
                &mut ws.browser,
                msg,
                project_id,
                &crate::host_adapters::services::services_for(&client).bookmark,
                &handle,
                emit,
            );
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
                    let active = ws.active;
                    if let Some(tab) = ws.tabs.get_mut(active) {
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
                            dozer_core::log_warn!(LOG, "回写终端查询应答失败: {e}");
                        }
                    });
                }
                TabBackend::Custom(out) => {
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
                    let active = ws.active;
                    if let Some(tab) = ws.tabs.get_mut(active) {
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
                ws.term_tab_first = byteui::interaction::tab_strip::tab_window_reveal(
                    &widths,
                    4.0,
                    avail_px,
                    ws.term_tab_first,
                    idx,
                );
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

    /// 面板专属的"切入时动作"(刷新/重载/标记过期等)。图标栏点击(`panel_select`)
    /// 与程序化显示(`show_panel`)共用同一份,保证两条路径切进同一个面板时行为一致。
    fn fire_panel_switch_in(&mut self, kind: PanelKind) {
        match kind {
            PanelKind::GitLog => self.sync_git_log_to_active_project(),
            PanelKind::Todo => self.with_focused_project(|ws, io| {
                if let Some(ctx) = workspace_activation_ctx(ws, io, Message::Todo) {
                    todo::on_activate(&ws.todo, &ctx);
                }
            }),
            PanelKind::Database => self.with_focused_project(|ws, _io| {
                if let Some(project) = ws.project.as_ref() {
                    database::reload_from_disk(
                        &mut ws.database,
                        std::path::Path::new(&project.path),
                    );
                }
            }),
            PanelKind::Project => {
                self.ensure_project_readme_and_reveal();
                self.with_focused_project(|ws, io| {
                    if let Some(ctx) = workspace_activation_ctx(ws, io, Message::Project) {
                        project::on_activate(&ws.project_panel, &ctx);
                    }
                });
            }
            PanelKind::Ssh => self.with_focused_project(|ws, _io| {
                if let Some(project) = ws.project.as_ref() {
                    ssh::reload_from_disk(&mut ws.ssh, std::path::Path::new(&project.path));
                }
            }),
            PanelKind::Usage => self.with_focused_project(|ws, io| {
                let ctx = workspace_activation_ctx(ws, io, Message::Usage);
                usage::activate(&mut ws.usage, ctx);
            }),
            // 代码健康度面板切入时只读上次落盘结果，不自动扫描（spec：
            // 手动触发，与 Usage 的"打开即自动扫"是明确的行为差异）。
            PanelKind::CodeHealth => self.with_focused_project(|ws, io| {
                if let Some(ctx) = workspace_activation_ctx(ws, io, Message::CodeHealth) {
                    codehealth::on_activate(&ws.codehealth, &ctx);
                }
            }),
            // 会话列表原本只在项目打开时和回合结束时刷新,切进这个面板时
            // 没有任何补救手段——离开一段时间再切回来看到的还是上次的
            // 快照。补一次切入即刷新,同 `Usage` 面板的既有口径。
            PanelKind::Conversations => self.with_focused_project(|ws, io| {
                if let Some(ctx) = workspace_activation_ctx(ws, io, Message::Conversations) {
                    conversations::on_activate(&ws.conversations, &ctx);
                }
            }),
            // 切入时刷新群列表(别的入口可能新增过群);`mark_stale` 不清已有内容,
            // 切入瞬间不会闪成"没有群聊"。实际加载由 `poll_group_chat_if_active` 发起。
            PanelKind::GroupChat => self.with_focused_project(|ws, _io| {
                crate::extensions::group_chat::on_activate(&mut ws.group_chat);
            }),
            PanelKind::App(slot) => {
                self.app_host_update(crate::extensions::app_host::Message::PanelShown(slot));
            }
            PanelKind::Files | PanelKind::Web | PanelKind::Agent => {}
        }
    }

    /// 程序化"确保这个面板看得见"(如群聊里点"已转待办 ↗"去 Todo)。与图标栏点击
    /// (`panel_select`)的区别:**绝不收起**、**不武装图标栏拖拽**——点已激活的图标
    /// 是"退回未选中并收起",且按下即武装拖拽,这两个副作用都不该由程序化跳转触发。
    /// 逻辑在纯函数 `rail::show_panel_in`(带单测)。
    pub(crate) fn show_panel(&mut self, kind: PanelKind) {
        let side = self.shell_layout.rail_layout.side_of(kind);
        let (view, collapsed) = match side {
            Side::Left => (self.left_view, self.left_collapsed),
            Side::Right => (self.right_view, self.right_collapsed),
        };
        let out = rail::show_panel_in(side, kind, view, collapsed, self.maximized);
        if !out.changed {
            return;
        }
        match side {
            Side::Left => {
                self.left_view = out.view;
                self.left_collapsed = out.collapsed;
            }
            Side::Right => {
                self.right_view = out.view;
                self.right_collapsed = out.collapsed;
            }
        }
        self.maximized = out.maximized;
        self.fire_panel_switch_in(kind);
        self.on_shell_layout_changed();
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
            self.fire_panel_switch_in(kind);
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

    /// 两栏面板(Project/Todo/Database/Ssh/Agent/Conversations/GroupChat/
    /// Usage)的列表列当前是否被收起。语义同 `files_tree_collapsed`:列表不
    /// 渲染、内容拿满配对宽度,split 比例保留(展开时按原宽度恢复)。
    pub(crate) fn list_collapsed(&self, kind: PanelKind) -> bool {
        match kind {
            PanelKind::Files => false,
            _ => self.dims.collapsed(kind).unwrap_or(false),
        }
    }

    /// 翻转某两栏面板列表列的展开/收起(`Message::TogglePanelListCollapse` 的
    /// 处理)。只改对应布尔、不动 split 比例,并像 `panel_select` 一样退出
    /// 放大态 + 落盘/重算网格。非两栏面板(`Files` 走独立的
    /// `files_tree_collapsed`,其余单/两栏面板无此能力)直接忽略。
    pub(crate) fn toggle_panel_list_collapse(&mut self, kind: PanelKind) {
        if kind == PanelKind::Files {
            return;
        }
        let Some(flag) = self.dims.collapsed_mut(kind) else {
            return;
        };
        *flag = !*flag;
        self.maximized = None;
        self.on_shell_layout_changed();
    }

    /// 内容侧"收起/展开列表列"按钮:用户点击某面板内容区的按钮翻转其列表列
    /// 显隐。语义完全对齐文件预览的 `HoverId::list_collapse(PanelKind::Files)` 按钮(见 `preview_pane_for`),
    /// 只是图标按该面板当前所在栏(左/右)与收起态四选一、tooltip 由调用方
    /// 给静态文案。泛型 `M` 兼容顶层 `Message` 与各扩展模块的本地 `Message`。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn list_collapse_button<'a, M: Clone + 'a>(
        &self,
        kind: PanelKind,
        collapsed: bool,
        hover_id: HoverId,
        tooltip_collapse: impl Into<std::borrow::Cow<'a, str>>,
        tooltip_expand: impl Into<std::borrow::Cow<'a, str>>,
        on_select: M,
        on_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
        let side = self.shell_layout.rail_layout.side_of(kind);
        let tooltip_collapse = tooltip_collapse.into();
        let tooltip_expand = tooltip_expand.into();
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

    /// 上下文条的消息路由。`project_id` 来自信封,异步应答即使在用户切到别的
    /// 项目之后到达,也只写回发起它的那个 `Workspace`。
    pub(crate) fn agent_context_message(
        &mut self,
        project_id: i64,
        msg: crate::extensions::agent_context::Message,
    ) {
        use crate::extensions::agent_context as ctx;
        let Some(root) = loaded_workspace_mut(&mut self.projects, project_id)
            .and_then(|ws| ws.project.as_ref().map(|p| PathBuf::from(&p.path)))
        else {
            return;
        };
        match msg {
            ctx::Message::Open(id) => {
                let Some(entry) = loaded_workspace_mut(&mut self.projects, project_id)
                    .and_then(|ws| ws.agent_context.entry(id).cloned())
                else {
                    return;
                };
                let path = root.join(&entry.info.entity_ref);
                if entry.info.entity_kind == "dir" {
                    // 目录:在文件树里选中它(不展开/收起,避免误 toggle)。
                    if let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) {
                        ws.files.set_tree_selected(path);
                    }
                } else {
                    self.preview_open_path(path);
                }
            }
            ctx::Message::OpenHistory(item) => {
                let filter = item.and_then(|id| {
                    loaded_workspace_mut(&mut self.projects, project_id)
                        .and_then(|ws| ws.agent_context.entry(id))
                        .map(|e| e.info.entity_ref.clone())
                });
                self.open_edit_history(project_id, filter);
            }
            other => {
                let context = crate::host_adapters::services::services_for(&self.client).context;
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::AgentContext(project_id, m));
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                ctx::update(
                    &mut ws.agent_context,
                    project_id,
                    root,
                    other,
                    context,
                    &handle,
                    emit,
                );
            }
        }
    }

    /// 拉取某项目的上下文列表(切入项目时调用;工作区未加载则空操作)。
    pub(crate) fn agent_context_refresh(&mut self, project_id: i64) {
        let Some(root) = loaded_workspace_mut(&mut self.projects, project_id)
            .and_then(|ws| ws.project.as_ref().map(|p| PathBuf::from(&p.path)))
        else {
            return;
        };
        let context = crate::host_adapters::services::services_for(&self.client).context;
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        crate::extensions::agent_context::request_refresh(
            &ws.agent_context.requests,
            project_id,
            root,
            context,
            &handle,
            move |m| {
                let _ = proxy.send_event(Message::AgentContext(project_id, m));
            },
        );
    }

    /// 打开修改历史弹窗(独立原生窗口由 `window_events` 的 sync 按
    /// `edit_history.is_some()` 开出)。`filter`:上下文项的 `entity_ref`,
    /// `None` = 全部。
    pub(crate) fn open_edit_history(&mut self, project_id: i64, filter: Option<String>) {
        self.edit_history = Some(crate::extensions::edit_history::State::new(
            project_id,
            filter.clone(),
            self.terminal_visible(),
        ));
        let edit_history = crate::host_adapters::services::services_for(&self.client).edit_history;
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        crate::extensions::edit_history::request_load(
            project_id,
            filter,
            edit_history,
            &handle,
            move |m| {
                let _ = proxy.send_event(Message::EditHistory(m));
            },
        );
    }

    pub(crate) fn edit_history_message(&mut self, msg: crate::extensions::edit_history::Message) {
        use crate::extensions::edit_history as eh;
        match msg {
            eh::Message::Locate(id) => {
                let Some((entry, project_id)) = self
                    .edit_history
                    .as_ref()
                    .and_then(|s| s.entry(id).cloned().map(|e| (e, s.project_id())))
                else {
                    return;
                };
                let Some(root) = loaded_workspace_mut(&mut self.projects, project_id)
                    .and_then(|ws| ws.project.as_ref().map(|p| PathBuf::from(&p.path)))
                else {
                    return;
                };
                let path = root.join(&entry.target_path);
                self.edit_history = None; // 关弹窗
                if path.is_file() {
                    // 定位到修改起始行(不选中整段范围,见 plan「有意偏差 3」)。
                    self.preview_open_path_at(path, Some(entry.start_line as usize));
                } else {
                    self.with_project(project_id, move |ws, _io| {
                        ws.preview_error = Some(
                            dozer_model::LocalizedText::keyed(
                                crate::i18n::HOST_OWNER,
                                "preview.file_missing_locate",
                                "The file no longer exists; cannot locate: {path}",
                            )
                            .with_arg("path", path.display().to_string()),
                        );
                    });
                }
            }
            eh::Message::AskAgent(id) => {
                let Some(text) = self
                    .edit_history
                    .as_ref()
                    .filter(|s| s.agent_terminal_visible())
                    .and_then(|s| {
                        s.entry(id)
                            .map(|e| eh::ask_agent_text(e, self.locale.active_locale()))
                    })
                else {
                    return;
                };
                self.edit_history = None; // 关弹窗
                self.term_paste(terminal::TermTarget::Shared, text);
            }
            other => {
                let edit_history =
                    crate::host_adapters::services::services_for(&self.client).edit_history;
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                eh::update(
                    &mut self.edit_history,
                    other,
                    self.locale.active_locale(),
                    edit_history,
                    &handle,
                    move |m| {
                        let _ = proxy.send_event(Message::EditHistory(m));
                    },
                );
            }
        }
    }

    /// 打开文件预览(Files 面板)。`target_line` 仅 CodeMirror 类页签生效。
    pub(crate) fn preview_open_path(&mut self, path: PathBuf) {
        self.preview_open_path_at(path, None);
    }

    pub(crate) fn preview_open_path_at(&mut self, path: PathBuf, target_line: Option<usize>) {
        self.preview_ui(
            PanelKind::Files,
            dozer_panel_preview::Message::OpenPath { path, target_line },
        );
    }

    /// Project 面板右配对预览打开文件(独立的 `ws.project_preview`)。
    pub(crate) fn project_preview_open_path(&mut self, path: PathBuf) {
        self.preview_ui(
            PanelKind::Project,
            dozer_panel_preview::Message::OpenPath {
                path,
                target_line: None,
            },
        );
    }

    /// 预览面板的 UI 消息:交给库的 `update`,再处理需要 `App` 级状态的通知(拖拽武装、悬停清理)。
    pub(crate) fn preview_ui(&mut self, kind: PanelKind, msg: dozer_panel_preview::Message) {
        self.preview_ui_for(None, kind, msg);
    }

    /// 预览面板消息的定向下发:`Some(project_id)` 时只投给该项目(页面事件必须
    /// 回到它自己的工作区,不能跟着「当前聚焦项目」跑);`None` = 聚焦项目。
    pub(crate) fn preview_ui_for(
        &mut self,
        project_id: Option<ProjectId>,
        kind: PanelKind,
        msg: dozer_panel_preview::Message,
    ) {
        use dozer_panel_preview::Message as PM;
        let cursor = self.last_cursor;
        #[cfg(target_os = "macos")]
        if matches!(msg, PM::TabOverflowToggle { .. }) {
            // macOS:原生菜单,选中项回投 `SelectTab`。
            let workspace = match project_id {
                Some(id) => loaded_workspace_mut(&mut self.projects, id).map(|ws| &*ws),
                None => self.active_workspace(),
            };
            let items = match workspace {
                Some(ws) => {
                    let pane = ws.preview_pane(kind);
                    let entries: Vec<(usize, String, bool)> = pane
                        .tabs()
                        .iter()
                        .enumerate()
                        .map(|(idx, tab)| (idx, tab.title.clone(), idx == pane.active_idx()))
                        .collect();
                    tab_widget::tab_overflow_items(&entries, move |i| {
                        Message::Preview(kind, PM::SelectTab(i))
                    })
                }
                None => Vec::new(),
            };
            if let Some(msg) = crate::chrome::native_menu::show(items, cursor) {
                self.update(msg);
            }
            return;
        }
        let msg = match msg {
            PM::TabOverflowToggle { .. } => PM::TabOverflowToggle { cursor },
            other => other,
        };
        // 得在借用 `ws` 之前算好——`preview_tab_bar_avail_px` 要 `&self`。
        let avail_px = self.preview_tab_bar_avail_px(kind);
        let locale = self.locale.active_locale().to_string();
        let mut app_level = Vec::new();
        match project_id {
            Some(project_id) => self.with_project(project_id, |ws, io| {
                app_level = ws.preview_dispatch(kind, msg, io, &locale, avail_px);
            }),
            None => self.with_focused_project(|ws, io| {
                app_level = ws.preview_dispatch(kind, msg, io, &locale, avail_px);
            }),
        }
        self.handle_preview_app_effects(kind, app_level);
    }

    /// S5b-2 P3:预览 host 事件的统一入口。分类/校验已在 `runtime` 的 IPC
    /// 闭包(`host_ipc::classify_ipc`)完成。P4 起与预览 tab 有关的事件直接进库
    /// (`dozer-panel-preview::update`),面板外单槽宿主仍由本 crate 处理。
    fn host_event(&mut self, event: dozer_panel_preview::host_ipc::HostEvent) {
        use dozer_panel_preview::host_ipc::HostEvent;
        // 页面事件按**绑定里的项目**路由,不能跟着「当前聚焦项目」跑:后台项目的
        // webview 迟到的事件若投给聚焦项目,会改错工作区(甚至改错文件)。
        // 绑定项目已不存在(已关闭)时丢弃。
        let route = super::host_event_route(&event, &self.projects);
        let project_id = match route {
            super::HostEventRoute::AppLevelReviewTrace => {
                if let HostEvent::ReviewTrace(trace) = event {
                    self.update(Message::ReviewTraceWebviewEvent(trace));
                }
                return;
            }
            // 绑定项目已关闭(槽位不存在 / 仍是 `Stub`):丢弃,不投给任何仍在的项目。
            super::HostEventRoute::Drop => return,
            super::HostEventRoute::Project { project_id, .. } => project_id,
        };
        match event {
            HostEvent::GitLogDiff { binding, envelope } => {
                self.update(Message::GitLogDiffWebviewEvent(binding, envelope));
            }
            HostEvent::ReviewTrace(_) => {}
            HostEvent::Editor { binding, envelope } => self.preview_ui_for(
                Some(project_id),
                binding.panel.into(),
                dozer_panel_preview::Message::Host(HostEvent::Editor { binding, envelope }),
            ),
            HostEvent::Json { binding, envelope } => self.preview_ui_for(
                Some(project_id),
                binding.panel.into(),
                dozer_panel_preview::Message::Host(HostEvent::Json { binding, envelope }),
            ),
            HostEvent::Tabular { binding, envelope } => self.preview_ui_for(
                Some(project_id),
                binding.panel.into(),
                dozer_panel_preview::Message::Host(HostEvent::Tabular { binding, envelope }),
            ),
            HostEvent::Flyfish { binding, envelope } => self.preview_ui_for(
                Some(project_id),
                binding.panel.into(),
                dozer_panel_preview::Message::Host(HostEvent::Flyfish { binding, envelope }),
            ),
            HostEvent::ImageAnnotate { binding, envelope } => self.preview_ui_for(
                Some(project_id),
                binding.panel.into(),
                dozer_panel_preview::Message::Host(HostEvent::ImageAnnotate { binding, envelope }),
            ),
            HostEvent::PlantUml { binding, envelope } => self.preview_ui_for(
                Some(project_id),
                binding.panel.into(),
                dozer_panel_preview::Message::Host(HostEvent::PlantUml { binding, envelope }),
            ),
        }
    }

    /// 预览面板的后台任务结果:按 `project_id` 路由到发起它的项目。
    pub(crate) fn preview_result(
        &mut self,
        project_id: ProjectId,
        kind: PanelKind,
        msg: dozer_panel_preview::Message,
    ) {
        if let dozer_panel_preview::Message::OpenExternalFailed { path, error } = &msg {
            #[cfg(target_os = "macos")]
            let program = "open";
            #[cfg(not(target_os = "macos"))]
            let program = "xdg-open";
            let reason = crate::external_apps::OpenError::Spawn {
                program: program.to_string(),
                error: error.clone(),
            };
            self.update(Message::Toast(toast::Message::Push {
                scope: PREVIEW_LOG,
                level: toast::Level::Error,
                text: crate::external_apps::open_failure_message(None, path, &reason),
                key: None,
            }));
            return;
        }
        let locale = self.locale.active_locale().to_string();
        let mut app_level = Vec::new();
        self.with_project(project_id, |ws, io| {
            app_level = ws.preview_dispatch(kind, msg, io, &locale, 0.0);
        });
        self.handle_preview_app_effects(kind, app_level);
    }

    fn handle_preview_app_effects(
        &mut self,
        kind: PanelKind,
        effects: Vec<dozer_panel_preview::Effect>,
    ) {
        use dozer_panel_preview::Effect;
        for effect in effects {
            match effect {
                // 按住页签(选中处理)＝准备拖的来源。
                Effect::TabSelected { index } => {
                    let group = match kind {
                        PanelKind::Project => TabGroup::ProjectPreview,
                        _ => TabGroup::Preview,
                    };
                    self.tab_drag = Some(TabDrag {
                        group,
                        source: index,
                        press_pos: self.last_cursor,
                    });
                }
                // 预览内选区右键:库只报原始 webview 坐标,窗口坐标换算 /
                // 原生菜单弹出仍在本 crate(需要几何与终端可见性)。
                Effect::SelectionContextMenu {
                    panel,
                    tab_id,
                    path,
                    x,
                    y,
                    range,
                    selected_text,
                } => {
                    if let Some(project_id) = self.active_workspace().and_then(|ws| ws.project_id())
                    {
                        self.update(Message::PreviewSelectionMenuOpen {
                            project_id,
                            panel: panel.into(),
                            tab_id,
                            path,
                            x,
                            y,
                            range,
                            selected_text,
                        });
                    }
                }
                // 关掉的 tab 若正被悬停,残留的 hover 记录会被将来复用同一下标的新 tab 继承。
                Effect::TabClosed { index, old_len } => {
                    if matches!(kind, PanelKind::Project) {
                        self.dehover_after_tab_close(
                            |i| HoverId::tab_item(PanelKind::Project, i as u64),
                            |i| HoverId::tab_close(PanelKind::Project, i as u64),
                            index,
                            old_len,
                        );
                    } else {
                        self.dehover_after_tab_close(
                            |i| HoverId::tab_item(PanelKind::Files, i as u64),
                            |i| HoverId::tab_close(PanelKind::Files, i as u64),
                            index,
                            old_len,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    /// 点"扫描"(原生按钮与 webview 事件共用入口)。
    pub(crate) fn code_health_request_scan(&mut self) {
        self.with_focused_project(|ws, io| {
            if ws.codehealth.scanning() {
                return;
            }
            codehealth::update(&mut ws.codehealth, codehealth::Message::ScanRequested);
            ws.spawn_codehealth_scan(io);
        });
    }

    /// 代码健康度面板"点击函数跳转"入口：确保 Files 面板可见，再打开该
    /// 文件并跳到目标行。`panel_select` 在已选中同一面板时会触发"收起/
    /// 展开"的 toggle 副作用（见 `panel_select` 文档），这里先判断避免
    /// 误触。发现项的路径是相对项目根的规范化路径，这里解析成绝对路径。
    pub(crate) fn code_health_open_location(&mut self, path: PathBuf, line: usize) {
        if self.right_view != PanelKind::Files {
            self.panel_select(PanelKind::Files);
        }
        let resolved = if path.is_absolute() {
            path
        } else {
            self.active_workspace()
                .and_then(|ws| ws.active_project_path())
                .map(|root| root.join(&path))
                .unwrap_or(path)
        };
        self.preview_open_path_at(resolved, Some(line));
    }

    /// 代码健康度"交给 Agent 分析"：按 finding ID 从当前报告解析发现，生成
    /// 只读诊断上下文，送入当前 agent 会话输入区（不自动发送，用户仍需主动
    /// 回车）。报告更新后找不到 ID 时降级为无操作（不伪造发现）。
    pub(crate) fn code_health_analyze_finding(&mut self, id: String) {
        let locale = self.locale.active_locale().to_string();
        // 先只读解析出诊断文本（不可变借用，离开块即释放）。
        let text = {
            let Some(ws) = self.active_workspace() else {
                return;
            };
            let Some(report) = ws.codehealth.report() else {
                return;
            };
            let Some(finding) = report.findings.iter().find(|f| f.id == id) else {
                return;
            };
            let change = ws.codehealth.diff().map(|d| {
                if d.new.iter().any(|x| x.id == id) {
                    dozer_codehealth::FindingChange::New
                } else if d.worsened.iter().any(|x| x.id == id) {
                    dozer_codehealth::FindingChange::Worsened
                } else if d.improved.iter().any(|x| x.id == id) {
                    dozer_codehealth::FindingChange::Improved
                } else {
                    dozer_codehealth::FindingChange::Persisting
                }
            });
            let reasons: Vec<String> = ws
                .codehealth
                .hotspots()
                .iter()
                .find(|h| h.finding.id == id)
                .map(codehealth::view_model::hotspot_reasons)
                .unwrap_or_default();
            let scope = codehealth::view_model::scope_summary(&ws.codehealth);
            codehealth::view_model::analyze_finding_text(
                finding,
                change,
                &reasons,
                scope.as_ref(),
                &locale,
            )
        };

        // 送入当前会话输入区（无换行 → 不自动发送）。
        let bytes = text.into_bytes();
        self.with_focused_project(move |ws, io| {
            ws.send_input(io, bytes);
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

    /// 处理 `dozer-panel-agent` 的消息:先经库 `update` 落状态,再排空
    /// 发件箱(提示 → Toast、请求 → 原生菜单、宿主事件 → 跨面板刷新)。
    fn agent_panel_message(
        &mut self,
        project_id: ProjectId,
        msg: dozer_panel_agent::update::Message,
    ) {
        // macOS:页签溢出走原生菜单(与预览、SSH 一致),选中项回投 `SelectTab`。
        // S6c 把 Agent 面板搬进共享库时丢了这条分支,于是 macOS 上走 iced 浮层、锚点 `(0, 0)`。
        #[cfg(target_os = "macos")]
        if matches!(
            msg,
            dozer_panel_agent::update::Message::OverflowToggle { .. }
        ) {
            let cursor = self.last_cursor;
            let entries: Vec<(usize, String, bool)> =
                match loaded_workspace_mut(&mut self.projects, project_id) {
                    Some(ws) => ws
                        .agent
                        .tabs
                        .iter()
                        .enumerate()
                        .map(|(idx, tab)| {
                            (
                                idx,
                                dozer_panel_agent::tabs::tab_title(
                                    tab.agent,
                                    tab.cwd.as_deref(),
                                    &tab.info.name,
                                ),
                                idx == ws.agent.active,
                            )
                        })
                        .collect(),
                    None => Vec::new(),
                };
            let items = tab_widget::tab_overflow_items(&entries, move |i| {
                Message::Agent(
                    project_id,
                    dozer_panel_agent::update::Message::SelectTabNoDrag(i),
                )
            });
            if let Some(msg) = crate::chrome::native_menu::show(items, cursor) {
                self.update(msg);
            }
            return;
        }
        // 其他平台:iced 浮层,锚点取光标(库视图拿不到按钮位置,`at: None` 会退回 `(0, 0)`)。
        let msg = with_overflow_cursor(msg, self.last_cursor);
        let avail_px = self.terminal_tab_bar_avail_px();
        // 按下页签栏里的页签＝选中＋准备被拖走;卡片列表用 `SelectTabNoDrag`,不武装。
        let pressed_tab = match &msg {
            dozer_panel_agent::update::Message::SelectTab(idx) => Some(*idx),
            _ => None,
        };
        let mut notices = Vec::new();
        let mut requests = Vec::new();
        let mut host_events = Vec::new();
        self.with_project(project_id, |ws, io| {
            let Some(project) = ws.project.clone() else {
                return;
            };
            let hooks: std::sync::Arc<dyn dozer_panel_agent::adapter::HookInstaller> =
                std::sync::Arc::new(crate::host_adapters::agent_panel::DozerHooks);
            let services = crate::host_adapters::services::services_for(&io.client);
            let ctx = dozer_panel_agent::update::UpdateCtx {
                project: &project,
                svc: &services.session,
                handle: &io.handle,
                cols: io.cols,
                rows: io.rows,
                term_tab_bar_avail_px: avail_px,
                caps: dozer_panel_agent::adapter::AgentCapabilities::all(),
                hooks: &hooks,
                track_exit_critical: &|task| io.track_exit_critical(task),
            };
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Agent(project_id, m));
            };
            dozer_panel_agent::update::update(&mut ws.agent, msg, &ctx, emit);
            notices = ws.agent.take_outbox();
            requests = ws.agent.take_requests();
            host_events = ws.agent.take_host_events();
        });
        if let Some(idx) = pressed_tab
            && self
                .active_workspace()
                .is_some_and(|ws| idx < ws.tabs.len())
        {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Terminal,
                source: idx,
                press_pos: self.last_cursor,
            });
        }
        for n in notices {
            let level = match n.level {
                dozer_panel_host::NoticeLevel::Info => toast::Level::Info,
                dozer_panel_host::NoticeLevel::Success => toast::Level::Success,
                dozer_panel_host::NoticeLevel::Warning => toast::Level::Warning,
                dozer_panel_host::NoticeLevel::Error => toast::Level::Error,
            };
            match n.key {
                Some(key) => self.push_toast_keyed(LOG, level, n.text, &key),
                None => self.push_toast(LOG, level, n.text),
            }
        }
        if requests.iter().any(|r| {
            matches!(
                r,
                dozer_panel_agent::adapter::AgentRequest::ShowPicker { .. }
            )
        }) {
            let last_cursor = self.last_cursor;
            let items = crate::workspace::agent_picker_items();
            if let Some(msg) = crate::chrome::native_menu::show(items, last_cursor) {
                let _ = self.proxy.send_event(msg);
            }
        }
        for ev in host_events {
            if let dozer_panel_agent::adapter::AgentHostEvent::TurnEnded { tab_id } = ev {
                self.agent_turn_ended(project_id, tab_id);
            }
        }
    }

    /// 回合结束后的跨面板刷新(会话列表、项目 git/磁盘占用)。
    /// 卡片本身的刷新由库在 `AgentState` 分支内完成。
    fn agent_turn_ended(&mut self, project_id: ProjectId, _tab_id: usize) {
        self.with_project(project_id, |ws, io| {
            ws.spawn_conversations_refresh(io);
            if let Some(project) = &ws.project {
                let repo_path = PathBuf::from(&project.path);
                spawn_project_git_refresh(project_id, repo_path.clone(), io);
                spawn_disk_usage_refresh(project_id, repo_path, io);
            }
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
        let locale = self.locale.active_locale().to_string();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Files(m));
        };
        let app_files = &mut self.files;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let external_apps = self.external_apps.clone();
        files::update(
            &mut ws.files,
            app_files,
            msg,
            project_id,
            &handle,
            emit,
            &external_apps,
            &dozer_panel_files::adapter::FilesCapabilities::all(),
            &locale,
        );
        let requests = ws.files.take_requests();
        self.run_files_requests(requests);
    }

    /// 排空文件面板排给产品的请求:目前只有"弹菜单"。macOS 走原生 NSMenu
    /// (阻塞等用户选择,选中后把消息发回顶层);其余平台落成 iced 浮层状态,
    /// 由 `view` 渲染。
    fn run_files_requests(&mut self, requests: Vec<dozer_panel_files::adapter::FilesRequest>) {
        use dozer_panel_files::adapter::FilesRequest;
        for request in requests {
            let (items, at) = match request {
                FilesRequest::ShowContextMenu { items, at }
                | FilesRequest::ShowNativeMenu { items, at } => (items, at),
                FilesRequest::ShowBranchPicker { .. } => continue,
            };
            #[cfg(target_os = "macos")]
            {
                let native = items
                    .into_iter()
                    .map(|item| match item {
                        dozer_panel_files::menu::MenuItem::Entry {
                            icon,
                            label,
                            enabled,
                            msg,
                        } => {
                            let body = byteui::theme::color::current().body;
                            crate::chrome::native_menu::Item::Entry {
                                icon,
                                icon_color: None,
                                label,
                                color: body,
                                enabled,
                                msg,
                            }
                        }
                        dozer_panel_files::menu::MenuItem::Separator => {
                            crate::chrome::native_menu::Item::Separator
                        }
                    })
                    .collect();
                if let Some(msg) = crate::chrome::native_menu::show_align_no_icon_left(native, at) {
                    let _ = self.proxy.send_event(Message::Files(msg));
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                // iced 浮层从状态重建菜单,这里只落坐标与目标。
                let _ = (items, at);
            }
        }
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
                crate::i18n::host_text("menu.delete", "Delete", self.locale.active_locale()),
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
    /// 下移。定位坐标复用 `files.last_right_click`。
    pub(crate) fn category_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.category_context_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let locale = self.locale.active_locale();
        // 右键的是"全部"/"未分类"伪节点:菜单只含"新建分类"(新建顶层
        // 分类,不挂在任何真实名字下——那两个只是视图桶)。真实节点才给
        // 完整节点操作集。
        let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            match menu.id {
                None => vec![crate::chrome::menu::item::<Message>(
                    Some(icons::IconKind::SquarePlus),
                    crate::i18n::host_text("menu.new_category", "New category", locale),
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
                            crate::i18n::host_text(
                                "menu.new_subcategory",
                                "New subcategory",
                                locale,
                            ),
                            Message::Todo(todo::Message::CategoryNewChild(id)),
                        ),
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::SquarePlus),
                            crate::i18n::host_text(
                                "menu.new_sibling_category",
                                "New sibling category",
                                locale,
                            ),
                            Message::Todo(todo::Message::CategoryNewSibling(sibling_parent_id)),
                        ),
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::ChevronUp),
                            crate::i18n::host_text("menu.move_up", "Move up", locale),
                            Message::Todo(todo::Message::CategoryMoveSibling(
                                id,
                                dozer_core::protocol::CategoryMoveDirection::Up,
                            )),
                        ),
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::ChevronDown),
                            crate::i18n::host_text("menu.move_down", "Move down", locale),
                            Message::Todo(todo::Message::CategoryMoveSibling(
                                id,
                                dozer_core::protocol::CategoryMoveDirection::Down,
                            )),
                        ),
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::Rename),
                            crate::i18n::host_text("menu.rename", "Rename", locale),
                            Message::Todo(todo::Message::CategoryRenameStart(id)),
                        ),
                        crate::chrome::menu::item::<Message>(
                            Some(icons::IconKind::Trash),
                            crate::i18n::host_text("menu.delete", "Delete", locale),
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
        let locale = self.locale.active_locale();
        let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> = vec![
            crate::chrome::menu::item::<Message>(
                Some(icons::IconKind::RefreshCw),
                crate::i18n::host_text("menu.test_connection", "Test connection", locale),
                Message::Database(database::Message::TestConnection(source_id.clone())),
            ),
            crate::chrome::menu::item::<Message>(
                Some(icons::IconKind::Settings),
                crate::i18n::host_text("menu.edit", "Edit", locale),
                Message::Database(database::Message::EditSourceStart(source_id.clone())),
            ),
            crate::chrome::menu::item::<Message>(
                Some(icons::IconKind::Trash),
                crate::i18n::host_text("menu.delete", "Delete", locale),
                Message::Database(database::Message::DeleteSourceRequest(source_id.clone())),
            ),
        ];
        items.push(if expanded {
            crate::chrome::menu::item::<Message>(
                Some(icons::IconKind::RotateCw),
                crate::i18n::host_text("menu.refresh", "Refresh", locale),
                Message::Database(database::Message::SchemaRefresh(source_id.clone())),
            )
        } else {
            crate::chrome::menu::item_locked(
                Some(icons::IconKind::RotateCw),
                crate::i18n::host_text("menu.refresh", "Refresh", locale),
                dim,
            )
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
        let locale = self.locale.active_locale();
        let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            Vec::new();
        if menu.target.secure {
            // 密码框:复制/剪切同原生快捷键一样被禁用,置灰不可点。
            items.push(crate::chrome::menu::item_locked(
                Some(icons::IconKind::Scissors),
                crate::i18n::host_text("menu.cut", "Cut", locale),
                dim,
            ));
            items.push(crate::chrome::menu::item_locked(
                Some(icons::IconKind::Copy),
                crate::i18n::host_text("menu.copy", "Copy", locale),
                dim,
            ));
        } else {
            items.push(crate::chrome::menu::item(
                Some(icons::IconKind::Scissors),
                crate::i18n::host_text("menu.cut", "Cut", locale),
                Message::TextInputMenuCut,
            ));
            items.push(crate::chrome::menu::item(
                Some(icons::IconKind::Copy),
                crate::i18n::host_text("menu.copy", "Copy", locale),
                Message::TextInputMenuCopy,
            ));
        }
        items.push(crate::chrome::menu::item(
            Some(icons::IconKind::ClipboardPaste),
            crate::i18n::host_text("menu.paste", "Paste", locale),
            Message::TextInputMenuPaste,
        ));
        items.push(crate::chrome::menu::item(
            Some(icons::IconKind::SelectAll),
            crate::i18n::host_text("menu.select_all", "Select All", locale),
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

#[cfg(test)]
mod host_event_routing_tests {
    use super::*;
    use crate::workspace::Workspace;
    use dozer_core::protocol::ProjectInfo;

    fn loading(id: i64) -> crate::app::WorkspaceSlot {
        // 促成期的"加载中"占位恒有 `project: Some(_)`,与真实的已加载工作区同形。
        crate::app::WorkspaceSlot::Loaded(Box::new(Workspace::loading_for_project(ProjectInfo {
            id,
            path: format!("/tmp/p{id}"),
            name: format!("p{id}"),
            last_active_ms: 0,
            created_ms: 0,
            updated_ms: 0,
        })))
    }

    /// 页签 7 的图片 host 回 `ready` 会清掉 `web_error`,正好当作"这个工作区
    /// 收到了自己的事件"的观测点。
    const TAB: usize = 7;

    fn give_dirty_tab(ws: &mut Workspace) {
        let id = ws
            .preview
            .open_path(std::path::PathBuf::from("/tmp/p/photo.png"));
        // `open_path` 新建的页签 id 从 1 起数;测试里统一改叫 `TAB`,两边项目同号
        // 才复现"误投会改错工作区"的场景。
        ws.preview
            .tabs_mut()
            .iter_mut()
            .find(|t| t.id == id)
            .expect("刚打开的页签")
            .id = TAB;
        let tab = ws
            .preview
            .tabs_mut()
            .iter_mut()
            .find(|t| t.id == TAB)
            .expect("改号后的页签");
        tab.web_error = Some("before".into());
    }

    fn error_of(
        projects: &std::collections::HashMap<i64, crate::app::WorkspaceSlot>,
        project: i64,
        tab_id: usize,
    ) -> Option<String> {
        match projects.get(&project) {
            Some(crate::app::WorkspaceSlot::Loaded(ws)) => ws
                .preview
                .tabs()
                .iter()
                .find(|t| t.id == tab_id)
                .and_then(|t| t.web_error.clone()),
            _ => None,
        }
    }

    /// 后台项目(1)的页面事件不能按「当前聚焦项目」(2)路由:两边第一个页签
    /// 必然同号,误投会改错工作区的状态。走的是 `App::host_event` 真正用的
    /// 那一条推导(`host_event_route`),不是平行包装。
    #[test]
    fn host_event_for_a_project_that_is_not_focused_applies_to_its_own_state() {
        let mut projects = std::collections::HashMap::new();
        for id in [1, 2] {
            let mut slot = loading(id);
            if let crate::app::WorkspaceSlot::Loaded(ws) = &mut slot {
                give_dirty_tab(ws);
            }
            projects.insert(id, slot);
        }
        let binding = image_binding(1);
        let event = dozer_panel_preview::host_ipc::HostEvent::ImageAnnotate {
            binding,
            envelope: image_envelope(1),
        };
        // 路由结论:绑定项目 = 1,与聚焦项目 2 无关。
        assert_eq!(
            crate::app::host_event_route(&event, &projects),
            crate::app::HostEventRoute::Project {
                project_id: 1,
                panel: Some(PanelKind::Files),
            }
        );
        // 执行:事件落到项目 1 自己的工作区。
        let dozer_panel_preview::host_ipc::HostEvent::ImageAnnotate { binding, envelope } = event
        else {
            unreachable!()
        };
        let ws = loaded_workspace_mut(&mut projects, binding.project_id).expect("1 已加载");
        let _ = envelope;
        ws.preview
            .apply_image_annotate_event(TAB, dozer_panel_preview::ImageAnnotateEvent::Ready);
        assert_eq!(error_of(&projects, 1, TAB), None, "事件落到它自己的项目");
        assert_eq!(
            error_of(&projects, 2, TAB),
            Some("before".into()),
            "聚焦项目不能被后台项目的迟到事件改到"
        );
    }

    /// 绑定项目已关闭:事件丢弃,不投给任何仍在的项目,也不 panic。
    #[test]
    fn host_event_for_a_removed_project_is_dropped() {
        let mut projects = std::collections::HashMap::new();
        let mut slot = loading(1);
        if let crate::app::WorkspaceSlot::Loaded(ws) = &mut slot {
            give_dirty_tab(ws);
        }
        projects.insert(1, slot);
        // 未促成过的 `Stub` 也拿不到工作区。
        projects.insert(
            9,
            crate::app::WorkspaceSlot::Stub {
                info: ProjectInfo {
                    id: 9,
                    path: "/tmp/p9".into(),
                    name: "p9".into(),
                    last_active_ms: 0,
                    created_ms: 0,
                    updated_ms: 0,
                },
                activity: None,
            },
        );
        for id in [9, 42] {
            let event = dozer_panel_preview::host_ipc::HostEvent::ImageAnnotate {
                binding: image_binding(id),
                envelope: image_envelope(id),
            };
            assert_eq!(
                crate::app::host_event_route(&event, &projects),
                crate::app::HostEventRoute::Drop,
                "已关闭 / 未促成的项目必须丢弃事件(id={id})"
            );
        }
        // 被丢弃的事件没有波及仍在的项目。
        assert_eq!(error_of(&projects, 1, TAB), Some("before".into()));
    }

    /// `ReviewTrace` 没有绑定项目,是唯一允许绕过项目路由的例外。
    #[test]
    fn host_event_without_a_project_only_lets_review_trace_through() {
        let projects = std::collections::HashMap::new();
        let trace = dozer_panel_preview::host_ipc::HostEvent::ReviewTrace(
            dozer_panel_preview::ReviewTraceEvent::DocumentLoaded,
        );
        assert_eq!(
            crate::app::host_event_route(&trace, &projects),
            crate::app::HostEventRoute::AppLevelReviewTrace
        );
    }

    fn image_binding(project_id: i64) -> dozer_panel_preview::HostBinding {
        dozer_panel_preview::HostBinding::new(
            project_id,
            dozer_panel_preview::ViewerPanel::Files,
            TAB,
            format!("/tmp/p{project_id}/pic.png"),
        )
    }

    fn image_envelope(
        project_id: i64,
    ) -> dozer_panel_preview::WebviewEnvelope<dozer_panel_preview::ImageAnnotateEvent> {
        dozer_panel_preview::WebviewEnvelope {
            protocol_version: 1,
            project_id,
            panel: "files".into(),
            tab_id: TAB,
            document_id: String::new(),
            revision: 0,
            request_id: None,
            payload: dozer_panel_preview::ImageAnnotateEvent::Ready,
        }
    }
}

/// 按路径重载一个文件页签。`find_existing_file_tab` 返回页签**下标**,`bump_reload` 要页签 **id**——
/// 关过页签后两者不再相等,不能把下标当 id 传。找不到该路径的页签返回 `false`。
fn bump_reload_for_path(pane: &mut crate::preview::PreviewPane, path: &std::path::Path) -> bool {
    let Some(idx) = pane.find_existing_file_tab(path) else {
        return false;
    };
    let Some(tab_id) = pane.tabs().get(idx).map(|tab| tab.id) else {
        return false;
    };
    pane.bump_reload(tab_id);
    true
}

/// 溢出下拉的锚点:库视图拿不到按钮位置,发出的 `OverflowToggle` 带 `at: None`,库对此退回
/// `(0, 0)`。这里把当前光标位置补上(迁移前就是取光标);已有位置的原样保留。
fn with_overflow_cursor(
    msg: dozer_panel_agent::update::Message,
    cursor: (f32, f32),
) -> dozer_panel_agent::update::Message {
    use dozer_panel_agent::update::Message as AM;
    match msg {
        AM::OverflowToggle { at: None } => AM::OverflowToggle { at: Some(cursor) },
        other => other,
    }
}

#[cfg(test)]
mod agent_overflow_anchor_tests {
    use super::*;
    use dozer_panel_agent::update::Message as AM;

    #[test]
    fn an_overflow_toggle_without_a_position_gets_the_cursor() {
        // 库视图拿不到按钮位置,发的是 `at: None`,库对此退回 `(0, 0)`——菜单就跑到窗口
        // 左上角(S6c 把 Agent 面板搬进库时丢了迁移前"锚点取光标"的行为)。
        let msg = with_overflow_cursor(AM::OverflowToggle { at: None }, (321.0, 45.0));
        assert!(matches!(
            msg,
            AM::OverflowToggle {
                at: Some((321.0, 45.0))
            }
        ));
    }

    #[test]
    fn an_explicit_position_is_kept() {
        let msg = with_overflow_cursor(
            AM::OverflowToggle {
                at: Some((1.0, 2.0)),
            },
            (9.0, 9.0),
        );
        assert!(matches!(
            msg,
            AM::OverflowToggle {
                at: Some((1.0, 2.0))
            }
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_native_agent_overflow_menu_does_not_arm_a_tab_drag() {
        // 原生菜单吞掉鼠标松开事件:选中走 `SelectTab` 会武装页签拖拽(`pressed_tab`),拖拽状态
        // 一直挂着,之后鼠标扫过页签就被换位。从菜单选的不是"按下页签",要用 `SelectTabNoDrag`。
        let source = include_str!("update.rs");
        let start = source
            .find("fn agent_panel_message(")
            .expect("找不到 agent_panel_message");
        let rest = &source[start..];
        let end = rest.find("let avail_px").expect("找不到原生分支结尾");
        let branch = &rest[..end];
        assert!(branch.contains("native_menu::show"), "应有原生菜单分支");
        assert!(branch.contains("SelectTabNoDrag"));
        assert!(!branch.contains("update::Message::SelectTab("));
    }

    #[test]
    fn other_messages_pass_through_untouched() {
        assert!(matches!(
            with_overflow_cursor(AM::SelectTab(3), (9.0, 9.0)),
            AM::SelectTab(3)
        ));
    }
}

#[cfg(test)]
mod reload_by_path_tests {
    use super::*;
    use crate::preview::PreviewPane;

    fn touch(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, "x\n").unwrap();
        p
    }

    #[test]
    fn reloading_by_path_bumps_the_right_tab_after_another_tab_was_closed() {
        // `find_existing_file_tab` 返回页签**下标**,`bump_reload` 要页签 **id**。关过页签后两者
        // 不再相等:原来直接把下标当 id 传,会重载错页签(或者什么也没发生)。
        let dir = std::env::temp_dir().join(format!("dozer-reload-by-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (touch(&dir, "a.rs"), touch(&dir, "b.rs"));
        let mut pane = PreviewPane::default();
        let a_idx = pane.open_path(a.clone());
        let _b_idx = pane.open_path(b.clone());
        pane.close(a_idx);
        assert!(bump_reload_for_path(&mut pane, &b));
        let bumped: Vec<String> = pane
            .tabs()
            .iter()
            .filter(|t| t.reload_nonce > 0)
            .map(|t| t.title.clone())
            .collect();
        assert_eq!(bumped, vec!["b.rs".to_string()]);
    }

    #[test]
    fn reloading_an_unopened_path_is_a_no_op() {
        let mut pane = PreviewPane::default();
        assert!(!bump_reload_for_path(
            &mut pane,
            std::path::Path::new("/nope/x.rs")
        ));
    }
}
