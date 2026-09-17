//! `App::view` 顶层渲染 + 自由 view 函数。Phase 3 结构重组时从 `app/app.rs`
//! 拆出,逻辑保持原样。

use crate::chrome::homespace::{self};
use crate::chrome::rail;
use crate::chrome::tab_widget;
use crate::chrome::topbar;
use crate::extensions::browser;
use crate::extensions::conversations;
use crate::extensions::database;
use crate::extensions::files;
use crate::extensions::footbar;
use crate::extensions::git_log;
use crate::extensions::project;
use crate::extensions::search;
use crate::extensions::ssh;
use crate::extensions::todo;
use crate::extensions::usage;
use crate::settings;
use crate::term::term_view;
use crate::term::terminal;
use crate::theme;
use crate::workspace::{
    PreviewPaneKind, Workspace, agent_list_pane, agent_picker_popup, dot_color,
    no_project_placeholder, preview_pane, preview_tab_overflow_popup, project_preview_pane,
    review_content_pane, split_portions, tab_display_width, tab_title,
};
use byteui::interaction::icons;
use dozer_core::protocol::{AgentKind, AgentState, SessionInfo};
use iced_widget::core::border::Radius;
use iced_widget::core::font::Weight;
use iced_widget::core::mouse;
use iced_widget::core::{Border, Color, Element, Font, Length, Padding};
use iced_widget::tooltip::{Position, Tooltip};
use iced_widget::{MouseArea, column, container, row, stack, text};

use super::*;
impl App {
    pub fn view(
        &self,
    ) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let content = self.view_inner();
        // 设置弹窗是 App 级浮层(见 `settings_modal_open` 字段文档),必须能
        // 盖在首页/空工作区/项目工作区三种 `view_inner` 分支之上——所以放在
        // 最外层统一叠加,而不是塞进 `view_inner` 内部某个分支。
        if self.settings_modal_open {
            stack![
                content,
                crate::dialog::scrim(Message::SettingsClose),
                settings::settings_modal(self.window_size.0)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            content
        }
    }

    fn view_inner(
        &self,
    ) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
        // 顶栏先画:它是外壳的一部分(项目页签行 + "＋"就在上面),一个项目都
        // 没打开时更要画得出来——否则用户没有任何入口去打开第一个项目。
        let top = topbar::top_bar(self);
        // 首页落地页:点顶栏 Dozer 进入,独立于工作区(即使没开任何项目也画得
        // 出来)。打开/切换项目会自动退回工作区(见各 `ProjectTab*` 处理器)。
        if self.current_page == AppPage::Home {
            return column![top, homespace::home_page(self, &self.footbar)].into();
        }
        // 一个项目页签都没有(或当前页签还停在 `Stub` 没促成)时的占位正文。
        let Some(ws) = self.active_workspace() else {
            // `daemon_error` 必须在这里也画:它平时挂在 `terminal_pane`/
            // `project_status_bar` 上,而那两处都在"有 `Workspace` 才走到"的
            // 分支里。偏偏 daemon 连不上时(`App::with_daemon_error`)一个项目
            // 都恢复不出来,恰恰只会走到这条空态分支——错误文案于是在最需要它
            // 的时候恰好隐身,用户只看到"点 ＋ 打开一个",点了又静默失败
            // (`ProjectTabOpened(None, ..)` 只是再写一遍 `daemon_error`)。
            // 配色沿用 `terminal_pane` 那条同源文案的 RED(最终审查
            // Required Fix #1)。
            let mut hint_col = column![
                text("未打开任何项目——点顶栏的 ＋ 打开一个")
                    .size(byteui::theme::font::subtitle())
                    .color(byteui::theme::color::current().dim)
            ]
            .spacing(8);
            if let Some(err) = &self.daemon_error {
                hint_col = hint_col.push(
                    text(format!("⚠ {err}"))
                        .size(byteui::theme::font::body())
                        .color(byteui::theme::color::current().red),
                );
            }
            let hint = container(hint_col.padding(16))
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_t: &iced_widget::Theme| container::Style {
                    background: Some(theme::region::background().into()),
                    ..container::Style::default()
                });
            return column![top, hint].into();
        };
        // 用 `row!`(经 `Row::push`/`enclose`)构造:只要子元素里有一个声明了
        // `Length::Fill`/`FillPortion`(如某侧收起时的 `left_panel_area`),
        // 这条 row 自身的宽度就会被自动升级成 `Fill`,从而在 flex 布局里正确
        // 撑满窗口。换成 `Row::from_vec`(其文档明确说明不会检视子元素)或
        // 手动 `.width(Length::Shrink)` 会让 flex 第三阶段(fill 分配)不再
        // 执行,右图标栏就会缩到窗口中间——不要在不理解这个前提的情况下改写。
        let body = row![
            rail::icon_rail(self, Side::Left),
            column![
                row![
                    left_panel_area(self, ws, false),
                    divider_bar(
                        Divider::LeftRight,
                        byteui::theme::color::current().bg,
                        byteui::theme::color::current().bg,
                        Message::ColumnDragStart(Divider::LeftRight),
                    ),
                    right_panel_area(self, ws, false),
                ]
                .height(Length::Fill),
                footbar::view(&self.footbar).map(Message::Footbar),
            ]
            .width(Length::Fill),
            rail::icon_rail(self, Side::Right),
        ];
        let base = column![top, body];

        let popped = if ws.search_popup_open() {
            // 文件树右键"搜索"弹窗:窗口级浮层。遮罩"点点即关"由
            // `search_modal` 内部自己处理(整窗 `SCRIM` 做成可点击目标,卡片
            // 是兄弟元素盖在上面),这里只需把弹窗叠在 `base` 之上。
            let project_root = ws.project.as_ref().map(|p| std::path::Path::new(&p.path));
            stack![
                base,
                search::search_modal(&ws.search, project_root, self.window_size.0)
                    .map(Message::Search)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.files.tree_delete_confirm_is_some() {
            let dismiss = crate::dialog::scrim(Message::Files(files::Message::DeleteCancel));
            stack![
                base,
                dismiss,
                files::delete_confirm_popup(&ws.files, self.window_size.0).map(Message::Files)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.files.pending_move_is_some() {
            let dismiss = crate::dialog::scrim(Message::Files(files::Message::MoveCancel));
            stack![
                base,
                dismiss,
                files::move_confirm_popup(&ws.files, self.window_size.0).map(Message::Files)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if self.files.context_menu_is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Files(files::Message::ContextMenuClose));
            stack![
                base,
                dismiss,
                files::context_menu_popup(&self.files, &ws.files).map(Message::Files)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.files.branch_picker_is_open() {
            // 分支切换弹层:窗口级浮层。下层铺一块透明 `MouseArea` 承接
            // "点弹层外的任何地方收起"(与右键菜单同款 dismiss 约定),弹层
            // 本体(`branch_picker_popup`)只占 git 底栏上方一隅。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Files(files::Message::BranchPickerClose));
            stack![
                base,
                dismiss,
                files::branch_picker_popup(&ws.files).map(Message::Files)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if self.project_link_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectLinkContextMenuClose);
            stack![base, dismiss, self.project_link_context_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.category_context_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::CategoryContextMenuClose);
            stack![base, dismiss, self.category_context_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.category_picker.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::CategoryPickerClose);
            stack![base, dismiss, self.category_picker_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.text_input_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::TextInputMenuClose);
            stack![base, dismiss, self.text_input_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.database_source_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::DatabaseSourceContextMenuClose);
            stack![base, dismiss, self.database_source_context_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if ws.project_panel.delete_pending.is_some() {
            // 项目面板「删除项目」确认框:窗口级 overlay,同其它面板弹窗
            // 的既有口径(2026-09-15 起——此前是 panel-level `stack!`,只在
            // 本面板宽度范围内居中,不是整个软件窗体)。
            let dismiss =
                crate::dialog::scrim(Message::Project(project::Message::DeleteProjectCancel));
            stack![
                base,
                dismiss,
                project::project_delete_confirm_popup(&ws.project_panel, self.window_size.0)
                    .map(Message::Project)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.project_panel.scaffold_run.is_some() {
            // 项目面板「修复项目」进度弹窗:窗口级 overlay。进行中不可通过
            // 点遮罩关闭(`scrim_blocking` 不挂 `on_press`),同 panel-level
            // 版本的既有约定(spec"弹窗可取消性"一节)。
            let scrim = crate::dialog::scrim_blocking();
            stack![
                base,
                scrim,
                project::scaffold_progress_popup(&ws.project_panel, self.window_size.0)
                    .map(Message::Project)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.database.delete_confirm().is_some() {
            // 数据库面板「删除数据源」确认框:窗口级 overlay,同上。三个
            // 数据库弹窗互斥优先级(同一时刻只显示一个):待确认删除 >
            // 新增/编辑表单 > 驱动管理。
            let source_id = ws.database.delete_confirm().unwrap();
            let dismiss =
                crate::dialog::scrim(Message::Database(database::Message::DeleteSourceCancel));
            stack![
                base,
                dismiss,
                database::delete_confirm_popup(&ws.database, source_id, self.window_size.0)
                    .map(Message::Database)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.database.editing().is_some() {
            // 数据库面板「新增/编辑数据源」表单:窗口级 overlay,同上。
            let draft = ws.database.editing().unwrap();
            let dismiss = crate::dialog::scrim(Message::Database(database::Message::DraftCancel));
            stack![
                base,
                dismiss,
                database::source_form(
                    draft,
                    &self.database,
                    ws.database.draft_test_status(),
                    self.window_size.0,
                )
                .map(Message::Database)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if self.database.drivers_popup_open() {
            // 数据库面板「管理驱动」弹窗:窗口级 overlay,同上。
            let dismiss =
                crate::dialog::scrim(Message::Database(database::Message::DriversPopupToggle));
            stack![
                base,
                dismiss,
                database::drivers_popup(&self.database, self.window_size.0).map(Message::Database)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.ssh.delete_confirm().is_some() {
            // 主机面板「删除主机」确认框:窗口级 overlay,同上。两个主机
            // 弹窗互斥优先级(同一时刻只显示一个):待确认删除 > 新增/编辑
            // 表单。
            let host_id = ws.ssh.delete_confirm().unwrap();
            let dismiss = crate::dialog::scrim(Message::Ssh(ssh::Message::DeleteHostCancel));
            stack![
                base,
                dismiss,
                ssh::delete_confirm_popup(&ws.ssh, host_id, self.window_size.0).map(Message::Ssh)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.ssh.editing().is_some() {
            // 主机面板「添加/编辑主机」表单:窗口级 overlay,同上。
            let draft = ws.ssh.editing().unwrap();
            let status = draft
                .id
                .as_deref()
                .map(|id| ws.ssh.test_status(id))
                .unwrap_or(&ssh::TestStatus::Idle);
            let dismiss = crate::dialog::scrim(Message::Ssh(ssh::Message::DraftCancel));
            stack![
                base,
                dismiss,
                ssh::host_form(draft, status, self.window_size.0).map(Message::Ssh)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.agent_picker_open {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::AgentPickerClose);
            stack![base, dismiss, agent_picker_popup(ws)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.project_add_menu_open {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectAddMenuClose);
            stack![base, dismiss, topbar::project_add_menu_popup(self)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if ws.todo.status_popup_open() {
            // 状态下拉选择层:窗口级 overlay。点弹层外任意处经 dismiss 收起
            // (与右键菜单/分支切换同款约定),弹层本体定位到点击"状态"按钮时
            // 的光标锚点。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Todo(todo::Message::StatusClose));
            match todo::todo_status_overlay(ws, self.window_size) {
                Some(popup) => stack![base, dismiss, popup.map(Message::Todo)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.todo.calendar_popup_open() {
            // 日历浮层:窗口级 overlay。点弹层外任意处经 dismiss 收起(与右键
            // 菜单/分支切换同款约定),弹层本体定位到点击按钮时的光标锚点。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Todo(todo::Message::CalendarClose));
            match todo::todo_calendar_overlay(ws, self.window_size) {
                Some(popup) => stack![base, dismiss, popup.map(Message::Todo)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.todo.dispatch_popup_open() {
            // 派发选择层:窗口级 overlay。点弹层外任意处经 dismiss 收起(与
            // 右键菜单/分支切换同款约定),弹层本体列出可指派的 agent(带图标),
            // 定位到点击"指派"按钮时的光标锚点。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Todo(todo::Message::DispatchClose));
            match todo::todo_dispatch_overlay(ws, self.window_size) {
                Some(popup) => stack![base, dismiss, popup.map(Message::Todo)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.todo.clear_confirm_open() {
            // Todo"清空列表"确认弹窗:窗口级 overlay,与其它 Todo 浮层同款
            // "点遮罩即收起"约定。
            let dismiss = crate::dialog::scrim(Message::Todo(todo::Message::ClearListCancel));
            stack![
                base,
                dismiss,
                todo::clear_confirm_popup(&ws.todo, self.window_size.0).map(Message::Todo)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.todo.detail_popup_open() {
            // 任务详情弹窗:窗口级 overlay,原生渲染(不走 wry webview)。
            // 点弹层外任意处经 dismiss 收起,与其它 Todo 浮层同款约定。
            let dismiss = crate::dialog::scrim(Message::Todo(todo::Message::DetailClose));
            stack![base, dismiss, self.todo_detail_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if ws.todo.status_filter_popup_open() {
            // 搜索框左前"状态"筛选浮层:窗口级 overlay。点弹层外任意处经
            // dismiss 收起(与右键菜单/分支切换同款约定),弹层本体的每一项
            // (全部/待办/进行中/搁置/已完成)emit `StatusFilterPick`,选中
            // 浮层即收。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Todo(todo::Message::StatusFilterClose));
            match todo::todo_status_filter_overlay(ws, self.window_size) {
                Some(popup) => stack![base, dismiss, popup.map(Message::Todo)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.term_tab_overflow_anchor.is_some() {
            // 终端 tab 栏"溢出下拉"(V 按钮):窗口级 overlay,理由见
            // `terminal::term_tab_overflow_popup` 文档——必须在这里(顶层)
            // 拼,`anchor`/`window_size` 才与全窗口坐标系一致,否则位置算错
            // (验收反馈"菜单错位了")。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::TermTabOverflowDismiss);
            match terminal::term_tab_overflow_popup(self, ws) {
                Some(popup) => stack![base, dismiss, popup]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.preview_tab_overflow_anchor.is_some() {
            // 文件预览 tab 栏"溢出下拉",处理方式同上。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::PreviewTabOverflowDismiss);
            match preview_tab_overflow_popup(self, ws, PreviewPaneKind::Files) {
                Some(popup) => stack![base, dismiss, popup]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.project_preview_tab_overflow_anchor.is_some() {
            // Project 面板配对预览 tab 栏"溢出下拉",处理方式同上。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectPreviewTabOverflowDismiss);
            match preview_tab_overflow_popup(self, ws, PreviewPaneKind::Project) {
                Some(popup) => stack![base, dismiss, popup]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.ssh_tab_overflow_anchor.is_some() {
            // SSH 面板自己 tab 条"溢出下拉",处理方式同上。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Ssh(ssh::Message::TabOverflowDismiss));
            match ssh_tab_overflow_popup(self, ws) {
                Some(popup) => stack![base, dismiss, popup]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.database.content().tab_overflow_anchor().is_some() {
            // Database 面板内容窗格 tab 栏"溢出下拉",处理方式同上;弹层本身
            // 用的是 database 扩展自己的 `Message`,`.map` 回顶层。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Database(database::Message::TabOverflowDismiss));
            match database::tab_overflow_popup(self, &ws.database) {
                Some(popup) => stack![base, dismiss, popup.map(Message::Database)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else {
            // 始终用 `Stack` 作根,与上面两个分支(删确认弹窗 / 右键菜单)保持一致:
            // 右键菜单开关会把根 widget 类型在 `Column`(`base.into()`)与 `Stack`
            // 之间切换,而 iced 的 `Tree::diff` 在根 tag 变化时(见
            // `iced_core::widget::tree::Tree::diff`)会整体重建整棵树、丢掉所有
            // 嵌套状态——文件树 scrollable 的滚动偏移就在其中,于是右键后滚动条
            // 跳回顶部。统一成 `Stack` 后根 tag 恒定,`base` 子树被 reconcile 原地
            // 保留,滚动位置不再丢失。
            stack![base].into()
        };

        let with_maximize: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            if let Some(which) = self.maximized {
                stack![popped, maximize_overlay(self, ws, which)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            } else {
                popped
            };

        if self.rail_drag_confirmed() {
            stack![with_maximize, rail::rail_drag_ghost(self)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.tree_drag_confirmed() {
            stack![with_maximize, files::tree_drag_ghost(self)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            with_maximize
        }
    }
}

/// 左二预览 pane:表头 + tab 栏 + 地址栏;内容区本体是 wry webview
/// 子视图(不在 iced 树里),这里只留占位背景——无 tab 时显示提示文案。
/// 左一项目栏：项目卡（名称 + git 分支/脏 + 路径）+ 文件树；无项目时"打开项目…" + 最近。
/// 右一 AI 栏（P1j）：视图切换 [对话|Agents] + 对话列表（当前行金框高亮）。
/// 顶栏：左 Dozer 标题、中 并行项目页签行 + "＋"、右 金色目标胶囊 + 设置齿轮。
///
/// 参数从 `&Workspace` 改成 `&App`：页签行要读的是**外壳级**的
/// `projects`/`project_order`/`active_project_id`（哪些项目开着、什么顺序、
/// 谁在前台），单个 `Workspace` 里没有这份信息。目标胶囊仍只讲当前项目，
/// 从 `app.active_workspace()` 取——没有项目在前台时它自然不画。
///
/// 原先中间的 ⌘K 搜索框是视觉占位（没有任何交互接线），让位给页签行；
/// 搜索入口日后回来时应另找位置，不要再把页签挤掉。
/// 顶栏内容行直接吃满 `top_bar_height()` 并 `align_y(Center)` 垂直居中——
/// 高度由 `workspace.json` 的 `geometry.top_bar_height` 单一来源驱动。
/// (`MACOS_TRAFFIC_LIGHT_BAND_HEIGHT` 的 28px 顶对齐约定已废弃:用户要
/// 求顶栏用自身高度居中内容,不再贴 macOS 交通灯基准。)
///
/// Figma 设计稿(Dozer Phase 1 UI,node-id=87:31)里顶栏标题/页签/加号
/// 文字标的都是 Inter Medium——应用没绑定 Inter,用系统默认字体的
/// Medium 档位贴近这个字重意图,不引入新字体文件。
pub(crate) fn top_bar_font() -> Font {
    Font {
        weight: Weight::Medium,
        ..Font::default()
    }
}

/// 上面那个的纯逻辑内核(可单测:构造 `Workspace` 需要 daemon + EventLoop,
/// headless 测试里造不出来,与本文件既有约定一致)。`agent == Unknown` 的
/// 会话(纯 shell/git shell/hook 还没上报过——`SessionInfo::agent` 文档:
/// "首个 hook 事件到达前恒 Unknown")不参与正常优先级竞争,它们的
/// `AgentState` 只是从未被真实 hook 改写过的默认值,不代表真实"空闲"——
/// 混进竞争会让纯 shell 页签显示成跟真实 agent 完成一轮工作同款的
/// cyan"空闲"点,分不清"agent 真空下来了"和"这压根不是 agent 会话"。
/// 若项目里**还有**真实 agent 存活,优先级/颜色照旧只看那些;若存活会话
/// **全是** Unknown,显示"死会话"灰点(不是"没有点"——用户仍要看得出这个
/// 项目有存活会话,只是状态不可知)。
pub(crate) fn project_dot(alive: &[(AgentState, AgentKind)]) -> Option<Color> {
    let real_states: Vec<AgentState> = alive
        .iter()
        .filter(|(_, agent)| *agent != AgentKind::Unknown)
        .map(|(state, _)| *state)
        .collect();
    if let Some(state) = winning_agent_state(&real_states) {
        return Some(agent_state_dot(state));
    }
    if alive.iter().any(|(_, agent)| *agent == AgentKind::Unknown) {
        return Some(byteui::theme::color::current().dim);
    }
    None
}

/// 一组存活会话状态里"最值得关注"的那个(2026-08-17 用户重新定案的优先级):
/// AwaitingInput(agent 在等你)> Running(还在跑)> TurnEnded(该你出手了)
/// > Idle > 无存活会话(`None`,不画点)。
///
/// 与 [`agent_state_dot`] 分家是为了让 `Stub` 页签也能用:启动恢复时那些还没
/// 促成的页签手上只有 daemon 的 `SessionInfo` 列表,没有 `Workspace`,但"哪个
/// 状态优先"这条规则必须与 `Loaded` 页签**完全一致**,不能各写一份
/// (最终审查 Required Fix #5)。
pub(crate) fn winning_agent_state(alive_states: &[AgentState]) -> Option<AgentState> {
    [
        AgentState::AwaitingInput,
        AgentState::Running,
        AgentState::TurnEnded,
        AgentState::Idle,
    ]
    .into_iter()
    .find(|candidate| alive_states.contains(candidate))
}

/// 胜出状态 → 颜色。不另造一套表,直接问既有 `dot_color`——页签点与 tab
/// 点讲的是同一种语言,两份颜色表迟早会漂。
pub(crate) fn agent_state_dot(state: AgentState) -> Color {
    dot_color(state, true)
}

/// 启动恢复时给每个 `Stub` 页签算后台活动状态:从 daemon 一次性吐出的全量
/// 会话列表里,挑出属于该项目的**存活**会话,套用与 `Loaded` 页签相同的优先级。
///
/// 为什么非要有这一步:重启后除了上次聚焦的那一个,**所有**页签都是 `Stub`,
/// 而 `Stub` 手上没有会话列表 → 指示点恒为空。也就是说"切走了还想知道另一个
/// 项目有没有在动"这个整套功能存在的核心理由(设计文档 §6),在最常见的
/// "刚打开 app"场景下完全不工作。这里只额外花一次 `list()` 往返、不促成任何
/// `Workspace`,懒加载照旧(最终审查 Required Fix #5)。
pub(crate) fn stub_activity(sessions: &[SessionInfo], project_id: i64) -> Option<Color> {
    let alive: Vec<(AgentState, AgentKind)> = sessions
        .iter()
        .filter(|s| s.alive && s.project_id == Some(project_id))
        .map(|s| (s.agent_state, s.agent))
        .collect();
    project_dot(&alive)
}

/// 面板区里某块 pane 在外框圆角处要收圆的外角:`Left`/`Right` 配对视图里
/// 左 pane 收左侧、右 pane 收右侧;`All` 是 Web 单 pane 收全部四角;`None`
/// 不收(放大态下 pane 直接撑满放大盒子,外框由金色浮层负责,方角才对)。
#[derive(Clone, Copy)]
pub(crate) enum PaneCorner {
    None,
    Left,
    Right,
    All,
}

/// 把 `left_zone`/`right_zone` 的圆角背景"透"到内部 pane 上:iced 的
/// `Container::clip(true)` 只把子元素裁成**矩形**,裁不出圆角,所以 pane
/// 自己的方角会戳出 zone 的圆角 CARD 背景,在四角形成小尖角。让 pane 的外
/// 圆角跟随 zone 圆角(半径减掉 zone 内边距),方角就被收进圆角里,只在外
/// 侧那一边收(`corner` 决定),配对的内部接缝仍是方角(本来就藏在 zone 内)。
pub(crate) fn zone_pane_border(zone: theme::region::RegionStyle, corner: PaneCorner) -> Border {
    let r = zone.border.map(|b| b.radius.top_left).unwrap_or(0.0);
    let r = (r - zone.padding.top).max(0.0);
    let radius = match corner {
        PaneCorner::None => Radius::from(0.0),
        PaneCorner::All => Radius::from(r),
        PaneCorner::Left => Radius {
            top_left: r,
            bottom_left: r,
            ..Radius::from(0.0)
        },
        PaneCorner::Right => Radius {
            top_right: r,
            bottom_right: r,
            ..Radius::from(0.0)
        },
    };
    Border {
        color: Color::TRANSPARENT,
        width: 0.0,
        radius,
    }
}

/// 渲染"某一种面板"的内容——`left_panel_area`/`right_panel_area` 共用。
///
/// Stage 4a 给图标栏面板加了跨栏拖拽后,`left_view` 可以是原先挂右栏的
/// `Agent`/`Conversations`/`Usage`/`Acceptance`,`right_view` 也可以是原先
/// 挂左栏的 `Files`/`GitLog`/`Todo`/`Project`/`Database`/`Ssh`/`Web`——
/// 之前两侧各自 `match` 里那行 `_ => unreachable!("Stage 1 ... 面板还固定
/// 在各自原侧")` 已经不成立,再碰到跨栏后的对侧面板会在渲染期直接 abort
/// (GUI 拖拽核对抓到的崩溃)。
///
/// 所以把"渲染一个面板"抽到这里做穷尽 `match`。每个分支只依赖
/// `zone`(外框主题与内部分割线配色)和 `lc/rc/ac`(pane 圆角朝向),由两侧
/// 各自传入自己那一侧的主题——除此之外同一面板在左/右栏渲染完全一致
/// (内部分割线用的 `Divider` variant 是面板固有属性,和挂哪条栏无关;
/// `panel_mirrored(kind)` 已经按当前实际所在栏算出是否镜像、自行翻转
/// `row!` 顺序)。各面板分割线的几何(`apply_column_drag`)已经由
/// Task 5/6 做成 side+镜像感知,这里只需正确渲染,无需再按左/右分支。
pub(crate) fn panel_body<'a>(
    app: &'a App,
    ws: &'a Workspace,
    kind: PanelKind,
    zone: theme::region::RegionStyle,
    lc: PaneCorner,
    rc: PaneCorner,
    ac: PaneCorner,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    match kind {
        PanelKind::Files => {
            if app.dims.files_tree_collapsed {
                // 收起文件树:整个配对宽度都交给预览,项目树列表与分隔线都不
                // 渲染。`files_split` 比例保留,展开时按原比例恢复。
                return preview_pane(app, ws, Length::Fill, zone_pane_border(zone, ac));
            }
            let (list_portion, content_portion) = split_portions(app.dims.files_split);
            let list_pane: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
                if ws.project.is_some() {
                    files::view(
                        &ws.files,
                        Length::FillPortion(list_portion),
                        zone_pane_border(zone, lc),
                        app.hover_progress(HoverId::FilesSearchSubmit),
                        app.hover_progress(HoverId::FilesDotfiles),
                        app.hover_progress(HoverId::FilesBranchSwitch),
                    )
                    .map(Message::Files)
                } else {
                    no_project_placeholder(
                        ws,
                        Length::FillPortion(list_portion),
                        zone_pane_border(zone, lc),
                    )
                };
            let preview = preview_pane(
                app,
                ws,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, rc),
            );
            let list_bg = theme::region::project_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let preview_bg = theme::region::preview_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Files) {
                row![
                    preview,
                    divider_bar(
                        Divider::LeftPairSplit,
                        preview_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::LeftPairSplit),
                    ),
                    list_pane,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    list_pane,
                    divider_bar(
                        Divider::LeftPairSplit,
                        list_bg,
                        preview_bg,
                        Message::ColumnDragStart(Divider::LeftPairSplit),
                    ),
                    preview,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::GitLog => git_log::view(
            app,
            &app.git_log,
            app.dims.git_log_split,
            app.dims.git_log_file_diff_split,
            app.panel_mirrored(PanelKind::GitLog),
        )
        .map(Message::GitLog),
        PanelKind::Todo => {
            let collapsed = app.list_collapsed(PanelKind::Todo);
            // 列表列收起:内容拿满整个配对宽度,侧栏不渲染。
            if collapsed {
                return todo::view(
                    app,
                    &ws.todo,
                    ws,
                    Length::Fixed(0.0),
                    Border::default(),
                    Length::Fill,
                    zone_pane_border(zone, ac),
                )
                .1
                .map(Message::Todo);
            }
            let (list_portion, content_portion) = split_portions(app.dims.todo_split);
            let (sidebar_pane, content_pane) = todo::view(
                app,
                &ws.todo,
                ws,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, lc),
                Length::FillPortion(content_portion),
                zone_pane_border(zone, rc),
            );
            let sidebar = sidebar_pane.map(Message::Todo);
            let content = content_pane.map(Message::Todo);
            let sidebar_bg = theme::region::project_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let content_bg = theme::region::preview_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Todo) {
                row![
                    content,
                    divider_bar(
                        Divider::TodoSplit,
                        content_bg,
                        sidebar_bg,
                        Message::ColumnDragStart(Divider::TodoSplit),
                    ),
                    sidebar,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    sidebar,
                    divider_bar(
                        Divider::TodoSplit,
                        sidebar_bg,
                        content_bg,
                        Message::ColumnDragStart(Divider::TodoSplit),
                    ),
                    content,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Project => {
            if app.list_collapsed(PanelKind::Project) {
                return project_preview_pane(app, ws, Length::Fill, zone_pane_border(zone, ac));
            }
            let (list_portion, content_portion) = split_portions(app.dims.project_split);
            let info_pane: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
                project::view(
                    &ws.project_panel,
                    ws.project.as_ref(),
                    Length::FillPortion(list_portion),
                    zone_pane_border(zone, lc),
                    project::ProjectPaneHover {
                        docs_add: app.hover_progress(HoverId::ProjectDocsAdd),
                        memory_add: app.hover_progress(HoverId::ProjectMemoryAdd),
                        remote_add: app.hover_progress(HoverId::ProjectRemoteAdd),
                    },
                )
                .map(Message::Project);
            let preview = project_preview_pane(
                app,
                ws,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, rc),
            );
            let info_bg = theme::region::project_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let preview_bg = theme::region::preview_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Project) {
                row![
                    preview,
                    divider_bar(
                        Divider::ProjectSplit,
                        preview_bg,
                        info_bg,
                        Message::ColumnDragStart(Divider::ProjectSplit),
                    ),
                    info_pane,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    info_pane,
                    divider_bar(
                        Divider::ProjectSplit,
                        info_bg,
                        preview_bg,
                        Message::ColumnDragStart(Divider::ProjectSplit),
                    ),
                    preview,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Database => {
            // 数据库面板需要项目已打开才能读写 `.dozer/database.json`。
            if ws.project.is_none() {
                return column![].into();
            }
            if app.list_collapsed(PanelKind::Database) {
                return database::content_pane(
                    app,
                    &ws.database,
                    Length::Fill,
                    zone_pane_border(zone, ac),
                )
                .map(Message::Database);
            }
            let (list_portion, content_portion) = split_portions(app.dims.database_split);
            let list_pane = database::view(
                &ws.database,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, lc),
            )
            .map(Message::Database);
            let content_pane = database::content_pane(
                app,
                &ws.database,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, rc),
            )
            .map(Message::Database);
            let list_bg = theme::region::project_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let content_bg = theme::region::preview_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Database) {
                row![
                    content_pane,
                    divider_bar(
                        Divider::DatabaseSplit,
                        content_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::DatabaseSplit),
                    ),
                    list_pane,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    list_pane,
                    divider_bar(
                        Divider::DatabaseSplit,
                        list_bg,
                        content_bg,
                        Message::ColumnDragStart(Divider::DatabaseSplit),
                    ),
                    content_pane,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Ssh => {
            // 同 Files/Database 面板:`ws.project.is_none()` 是 Stub→Loaded
            // 促成期间的占位态。
            if ws.project.is_none() {
                return column![].into();
            }
            if app.list_collapsed(PanelKind::Ssh) {
                return ssh_terminal_pane(app, ws, Length::Fill, zone_pane_border(zone, ac));
            }
            let (list_portion, content_portion) = split_portions(app.dims.ssh_split);
            let list_pane = ssh::view(
                app,
                &ws.ssh,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, lc),
            )
            .map(Message::Ssh);
            let terminal = ssh_terminal_pane(
                app,
                ws,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, rc),
            );
            let list_bg = theme::region::project_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let terminal_bg = theme::region::preview_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Ssh) {
                row![
                    terminal,
                    divider_bar(
                        Divider::SshSplit,
                        terminal_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::SshSplit),
                    ),
                    list_pane,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    list_pane,
                    divider_bar(
                        Divider::SshSplit,
                        list_bg,
                        terminal_bg,
                        Message::ColumnDragStart(Divider::SshSplit),
                    ),
                    terminal,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Web => browser::view(
            &ws.browser,
            ws.project.as_ref().map(|p| p.id),
            app.dims.browser_bookmarks_split,
            Length::Fill,
            zone_pane_border(zone, ac),
            app.panel_mirrored(PanelKind::Web),
        )
        .map(Message::Browser),
        PanelKind::Agent => {
            if app.list_collapsed(PanelKind::Agent) {
                return terminal::terminal_pane(app, ws, Length::Fill, zone_pane_border(zone, ac));
            }
            let (list_portion, content_portion) = split_portions(app.dims.agent_split);
            let terminal = terminal::terminal_pane(
                app,
                ws,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, lc),
            );
            let list = agent_list_pane(
                app,
                ws,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, rc),
            );
            let terminal_bg = theme::region::terminal_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let list_bg = theme::region::agent_list_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Agent) {
                row![
                    list,
                    divider_bar(
                        Divider::RightPairSplit,
                        list_bg,
                        terminal_bg,
                        Message::ColumnDragStart(Divider::RightPairSplit),
                    ),
                    terminal,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    terminal,
                    divider_bar(
                        Divider::RightPairSplit,
                        terminal_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::RightPairSplit),
                    ),
                    list,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Conversations => {
            if app.list_collapsed(PanelKind::Conversations) {
                return review_content_pane(app, ws, Length::Fill, zone_pane_border(zone, ac));
            }
            let (list_portion, content_portion) = split_portions(app.dims.conversations_split);
            let review = review_content_pane(
                app,
                ws,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, lc),
            );
            let list = conversations::view(
                app,
                &ws.conversations,
                ws.todo.items(),
                &ws.open_transcript_paths(),
                Length::FillPortion(list_portion),
                zone_pane_border(zone, rc),
            )
            .map(Message::Conversations);
            let review_bg = theme::region::review_content_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let list_bg = theme::region::conversation_list_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Conversations) {
                row![
                    list,
                    divider_bar(
                        Divider::RightPairSplit,
                        list_bg,
                        review_bg,
                        Message::ColumnDragStart(Divider::RightPairSplit),
                    ),
                    review,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    review,
                    divider_bar(
                        Divider::RightPairSplit,
                        review_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::RightPairSplit),
                    ),
                    list,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Usage => {
            // 加载中/还没数据时没有 agent 筛选栏可拼(同改造前
            // `sidebar: Option<..>` 为 `None` 时的行为),内容侧独占全宽。
            if !ws.usage.has_agent_filter() {
                return usage::content_pane(
                    app,
                    &ws.usage,
                    Length::Fill,
                    zone_pane_border(zone, ac),
                )
                .map(Message::Usage);
            }
            if app.list_collapsed(PanelKind::Usage) {
                return usage::content_pane(
                    app,
                    &ws.usage,
                    Length::Fill,
                    zone_pane_border(zone, ac),
                )
                .map(Message::Usage);
            }
            let (list_portion, content_portion) = split_portions(app.dims.usage_split);
            let content_pane = usage::content_pane(
                app,
                &ws.usage,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, lc),
            )
            .map(Message::Usage);
            let list_pane = usage::list_pane(
                &ws.usage,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, rc),
            )
            .map(Message::Usage);
            let content_bg = byteui::theme::color::current().panel;
            let list_bg = byteui::theme::color::current().bg;
            if app.panel_mirrored(PanelKind::Usage) {
                row![
                    list_pane,
                    divider_bar(
                        Divider::UsageSplit,
                        list_bg,
                        content_bg,
                        Message::ColumnDragStart(Divider::UsageSplit),
                    ),
                    content_pane,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    content_pane,
                    divider_bar(
                        Divider::UsageSplit,
                        content_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::UsageSplit),
                    ),
                    list_pane,
                ]
                .width(Length::Fill)
                .into()
            }
        }
    }
}

/// 左面板区:按当前左视图组合"项目树+文件预览"配对或单个 Web 预览面板;
/// 收起时渲染成空元素(不占宽度)。
///
/// 宽度语义与 `left_zone_width` 严格对应:对侧收起时本区 `Fill` 独占
/// `zones_width`(否则整行会缩到"两条图标栏+一条分隔线"那么宽,右图标栏
/// 跑到窗口中间去);两侧都收起时由本区出一个 `Fill` 空白把窗口撑满;
/// 常规态用 `Workspace::effective_left_width()`——**不是**直接读持久化的
/// `dims.left_width`。持久化宽可能大过当前窗口容得下的范围(用户
/// 在大窗口拖宽后把窗口缩小),那样这条 `Length::Fixed` 会在 flex 第一趟
/// 把可用空间吃光,唯一 `Fill` 的右面板区拿到 0 宽(Fix round 2 Critical #1)。
///
/// `maximized`(放大态用):为 `true` 时强制 `Length::Fill`,不看持久化宽/
/// 对侧收起态——`maximize_overlay` 需要这块区域真正撑满整个放大盒子,而不是
/// 停在平时拖拽出来的 `left_width` 那么宽。放大态下 `left_collapsed` 仍可能
/// 为真(旧注释断言"恒为 false"是错的:放大浮层不拦图标栏点击,先放大再点
/// 图标收起本侧是可达路径),此时上面那条收起分支返回空元素;`maximized`
/// 会被 `PanelSelect` 无条件清掉,所以这个组合不会
/// 停留超过一帧(Fix round 2 #2)。
/// 非放大态下,左1(项目树/Web)+左2(预览)两栏被视觉框成一个整体,套
/// `theme::region::left_zone()` 的外框(四向 margin 做悬浮留白,无描边)。
/// 放大态跳过——`maximize_overlay` 已经用金色边框把同一块内容整体框起来,
/// 再套一层外框会在金框内侧多出一圈视觉噪音。
pub(crate) fn left_panel_area<'a>(
    app: &'a App,
    ws: &'a Workspace,
    maximized: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if app.left_collapsed {
        return if app.right_collapsed {
            iced_widget::space::horizontal().into()
        } else {
            column![].into()
        };
    }
    let total = if maximized || app.right_collapsed {
        Length::Fill
    } else {
        Length::Fixed(app.effective_left_width())
    };
    let zone = theme::region::left_zone();
    let (lc, rc, ac) = if maximized {
        (PaneCorner::None, PaneCorner::None, PaneCorner::None)
    } else {
        (PaneCorner::Left, PaneCorner::Right, PaneCorner::All)
    };
    let inner = panel_body(app, ws, app.left_view, zone, lc, rc, ac);
    if maximized {
        return inner;
    }
    let region = zone;
    // 其它 worktree 切换条已从 Git Log 面板移除(用户需求),这里不再包任何
    // 额外层,直接透传面板本体。
    let zone_body: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> = inner;
    // 聚焦态外框:本 zone 拿到焦点(= `active_zone`)时描 GOLD 边,否则沿用
    // region 的默认(无描边)外框。半径保持与默认外框一致。
    let left_focused = app.active_zone == Some(ZoneSide::Left);
    let base_border = region.border.unwrap_or_default();
    let zone_box = container(zone_body)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(region.padding)
        .clip(true)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: if left_focused {
                Border {
                    color: byteui::theme::color::current().gold,
                    width: 2.0,
                    radius: base_border.radius,
                }
            } else {
                base_border
            },
            ..container::Style::default()
        });
    // 四向 margin:把整块外边框从顶栏/窗口底/图标栏/对侧分隔条各推开一段,
    // 做出悬浮留白。左右 margin 来自 `left_zone` 配置(默认左 8、右 0)。
    let m = region.margin;
    container(zone_box)
        .width(total)
        .height(Length::Fill)
        .padding(Padding {
            top: m.top,
            right: m.right,
            bottom: m.bottom,
            left: m.left,
        })
        .into()
}

/// 右面板区:按当前右视图组合"Agent 列表+终端"或"对话列表+对话审阅"配对;
/// 收起时渲染成空元素。总宽恒为剩余空间(`Fill`),不像左面板区那样有持久化
/// 的固定像素宽——所以内部分割只能用 `FillPortion` 表达,不能预先算像素。
///
/// 两块 pane 的宽度直接由它们自己的外层容器声明成 `FillPortion`,不再套一层
/// 包装容器:`Limits::width(Fixed(w))` 会把子元素的 min/max 都钉成 `w`,父级
/// 的 `FillPortion` 只约束包装容器本身、传不进子元素,曾导致这四块 pane 全部
/// 以 0 宽布局(右半边整片空白)。
///
/// 非放大态下,右1(Agent 列表/对话列表)+右2(终端/审阅)两栏被视觉框成
/// 一个整体,套 `theme::region::right_zone()` 的外框(四向 margin 做悬浮留白,
/// 无描边),`maximized` 时跳过(理由同 `left_panel_area`)。
pub(crate) fn right_panel_area<'a>(
    app: &'a App,
    ws: &'a Workspace,
    maximized: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if app.right_collapsed {
        return column![].into();
    }
    // 两个配对都是"内容侧渲染在左、列表侧渲染在右"——终端在左/Agent 列表
    // 在右,审阅在左/对话列表在右。`agent_split`/`conversations_split` 仍是
    // "列表侧(Agent 列表/对话列表)占右面板区宽度的比例"这个原有语义不变
    // (`terminal_pane_pixel_size` 等既有几何公式全靠它,不能跟着挪);只是
    // `content_portion`(∝ 1-split)现在给左边那块、`list_portion`(∝ split)
    // 给右边那块,单纯是 `row!` 里两个 pane 的先后顺序换了。`apply_column_drag`
    // 的 `RightPairSplit` 分支要相应把算出来的 ratio 取反再写回,否则拖拽
    // 方向感会反过来(见该函数注释)。
    let zone = theme::region::right_zone();
    let (lc, rc, ac) = if maximized {
        (PaneCorner::None, PaneCorner::None, PaneCorner::None)
    } else {
        (PaneCorner::Left, PaneCorner::Right, PaneCorner::All)
    };
    let inner = panel_body(app, ws, app.right_view, zone, lc, rc, ac);
    if maximized {
        return inner;
    }
    let region = zone;
    // 聚焦态外框:本 zone 拿到焦点(= `active_zone`)时描 GOLD 边,否则沿用
    // region 的默认(无描边)外框。半径保持与默认外框一致。
    let right_focused = app.active_zone == Some(ZoneSide::Right);
    let base_border = region.border.unwrap_or_default();
    let zone_box = container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(region.padding)
        .clip(true)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: if right_focused {
                Border {
                    color: byteui::theme::color::current().gold,
                    width: 2.0,
                    radius: base_border.radius,
                }
            } else {
                base_border
            },
            ..container::Style::default()
        });
    // 四向 margin:同 `left_panel_area`,左右 margin 来自 `right_zone` 配置
    // (默认左 0、右 8)。
    let m = region.margin;
    container(zone_box)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: m.top,
            right: m.right,
            bottom: m.bottom,
            left: m.left,
        })
        .into()
}

/// 放大态浮层:两条图标栏之间的整个内容区变暗+背景虚化，放大的那一侧
/// 内容(左/右面板区，含其内部列表:内容子分隔线，原样渲染，只是占满整个
/// 中间区域)金色描边突出。点变暗区域(放大内容之外的部分)退出放大。
pub(crate) fn maximize_overlay<'a>(
    app: &'a App,
    ws: &'a Workspace,
    which: MaximizedPane,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let inner = match which {
        MaximizedPane::Left => left_panel_area(app, ws, true),
        MaximizedPane::Right => right_panel_area(app, ws, true),
    };
    // `bordered` 显式给 `Length::Fill`(不留给默认 `Length::Shrink`)——
    // iced 0.14 的 `Limits` 有个"compression"传染机制:一个 `Shrink` 容器
    // 包一个 `Fill`/`FillPortion` 子元素时,子元素的 Fill 不会展开到可用
    // 空间,而是退化成"贴着内容收缩"(`Limits::resolve` 对 Fill 的展开分支
    // 要求 `!compression`,`Shrink` 会把 compression 设 true 并一路往下传,
    // 直到遇到一个显式 `Length::Fixed` 才重置)。这里如果不显式给 Fill,
    // `inner`(`left_panel_area`/`right_panel_area` 内部大量 FillPortion
    // 组成)会整体收缩成远小于放大盒子的intrinsic 尺寸,金色描边就会贴着
    // 一小块内容而不是撑满两条图标栏之间的放大区域。
    let overlay_style = theme::region::maximize_overlay();
    let bordered = container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            border: overlay_style.border,
            ..container::Style::default()
        });
    // 放大内容自己再罩一层"吃掉点击/滚轮"的 MouseArea,拦住它们冒泡到外层
    // dim 遮罩的 `MaximizeClose`(Fix round 2 #4)。iced 的事件是子先父后:
    // 内容里真正可交互的控件(按钮、终端 canvas)会先自己 capture,压根到不了
    // 这一层;到得了这一层的正是"不消费点击的地方"——审阅正文、卡片下方空白、
    // 列表空处——此前它们会一路穿到 dim 遮罩上,点一下正文就把放大退掉,而
    // 放大审阅恰恰是这个功能存在的理由。滚轮同理:内部 scrollable 真滚动了
    // 会自己 capture,滚不动时旧行为是穿到下层(基础层那块被遮住的终端
    // canvas)去滚终端历史,这里一并吃掉。
    let content_guard = MouseArea::new(bordered)
        .on_press(Message::Noop)
        .on_scroll(|_delta| Message::Noop);
    let dim_bg = MouseArea::new(
        container(content_guard)
            .padding(overlay_style.scrim_padding)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(overlay_style.scrim_background.into()),
                ..container::Style::default()
            }),
    )
    .on_press(Message::MaximizeClose);

    // 顶部垫一条透明的 `byteui::theme::geometry::top_bar_height()` 高 Space,把变暗遮罩钉在顶栏
    // 之下——`base = column![top, body]` 里顶栏和内容区就是这么分的,
    // 这里镜像同一结构,让变暗区域精确对齐 `body` 的渲染范围,不覆盖顶栏
    // (Important:此前没有这条 Space,遮罩会盖住整个窗口高度,连顶栏的
    // 项目 tab 等控件都会被染黑)。
    column![
        iced_widget::space::Space::new()
            .height(Length::Fixed(byteui::theme::geometry::top_bar_height())),
        row![
            iced_widget::space::Space::new()
                .width(Length::Fixed(byteui::theme::geometry::icon_rail_width())),
            dim_bg,
            iced_widget::space::Space::new()
                .width(Length::Fixed(byteui::theme::geometry::icon_rail_width())),
        ],
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// 分隔线:命中区 `byteui::theme::geometry::divider_width()` 宽、`Length::Fill` 高,
/// 中间一条 2px BORDER 竖线。悬停变 resize 光标走 `MouseArea::interaction` →
/// iced 既有的 `mouse_interaction` → `window.set_cursor` 管线(main.rs:808-816
/// 已有),不必另起一套光标代码。`on_press` 只发起拖拽状态,不指望 `MouseArea`
/// 的 `on_move`/`on_release`——它们要求光标不离开这条窄带才触发,快速拖拽会
/// 在光标移出后"断掉";持续追踪交给 `main.rs` 原始事件层。
///
/// 配对视图(左1左2 / 右1右2)内部:`left_bg`/`right_bg` 是分隔线两侧紧贴的
/// pane 底色。命中区左右两半(各 `(divider_width-2)/2`)分别填上这两色,只留
/// 中间 2px BORDER 竖线——否则 8px 命中区是透明的,会露出 zone 的 CARD 底色,
/// 在两块 pane 之间顶出一条浅色"沟",看起来像多了 padding/margin。填色后两块
/// pane 视觉贴合、只剩一条分割线,命中区宽度(拖拽手感)不变。
///
/// `Divider::LeftRight` 不画那条 2px 竖线、也不填色——它两侧各自套了
/// `theme::region::left_zone()`/`right_zone()` 的整体外框,这条 8px 缝是故意
/// 空出来给两侧 zone 圆角边框各自收边的,不能填成某侧 pane 色。
pub(crate) fn divider_bar<'a, M: Clone + 'a>(
    divider: Divider,
    left_bg: Color,
    right_bg: Color,
    on_drag: M,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let show_line = !matches!(divider, Divider::LeftRight);
    let body: Element<'_, M, iced_widget::Theme, iced_renderer::Renderer> = if !show_line {
        iced_widget::Space::new()
            .width(Length::Fixed(byteui::theme::geometry::divider_width()))
            .height(Length::Fill)
            .into()
    } else {
        let line_w = 2.0_f32;
        let side_w = (byteui::theme::geometry::divider_width() - line_w) / 2.0;
        let left_side = container(iced_widget::Space::new())
            .width(Length::Fixed(side_w))
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(left_bg.into()),
                ..container::Style::default()
            });
        let right_side = container(iced_widget::Space::new())
            .width(Length::Fixed(side_w))
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(right_bg.into()),
                ..container::Style::default()
            });
        let line = container(iced_widget::Space::new())
            .width(Length::Fixed(line_w))
            .height(Length::Fill)
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(byteui::theme::color::current().border.into()),
                ..container::Style::default()
            });
        row![left_side, line, right_side]
            .width(Length::Fixed(byteui::theme::geometry::divider_width()))
            .height(Length::Fill)
            .into()
    };
    MouseArea::new(body)
        .interaction(mouse::Interaction::ResizingColumn)
        .on_press(on_drag)
        .into()
}

/// `divider_bar` 的纵向(上下)镜像:一条水平分割线,`row!`→`column!`、
/// `width`↔`height` 互换,鼠标样式 `ResizingRow`(对应横向的
/// `ResizingColumn`)。目前只有 Git Log 面板右侧"文件列表 | diff 内容"这条
/// 纵向拖拽线用它。粗细复用 `byteui::theme::geometry::divider_width()`,与横向一致。
pub(crate) fn horizontal_divider_bar<'a, M: Clone + 'a>(
    top_bg: Color,
    bottom_bg: Color,
    on_drag: M,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let line_h = 2.0_f32;
    let side_h = (byteui::theme::geometry::divider_width() - line_h) / 2.0;
    let top_side = container(iced_widget::Space::new())
        .height(Length::Fixed(side_h))
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(top_bg.into()),
            ..container::Style::default()
        });
    let bottom_side = container(iced_widget::Space::new())
        .height(Length::Fixed(side_h))
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(bottom_bg.into()),
            ..container::Style::default()
        });
    let line = container(iced_widget::Space::new())
        .height(Length::Fixed(line_h))
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..container::Style::default()
        });
    let col = column![top_side, line, bottom_side]
        .height(Length::Fixed(byteui::theme::geometry::divider_width()))
        .width(Length::Fill);
    MouseArea::new(col)
        .interaction(mouse::Interaction::ResizingRow)
        .on_press(on_drag)
        .into()
}

/// tab 栏下方的 1px 分割线。
pub(crate) fn tab_divider<'a, M: 'a>() -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>
{
    byteui::layout::divider::horizontal()
}

/// 受控 tooltip:iced 0.14 的 `Tooltip` 没有"延迟显示"开关(它一悬停就弹),
/// 所以这里不靠 `Tooltip` 自带的 hover 检测,而是**仅在 `show` 为真时才把
/// `content` 包进 `Tooltip`**——调用方按"悬停满 2s"算好 `show`(见
/// `App::hover_tooltip_ready` / `browser::State::hover_tooltip_ready`),满 2s
/// 那一刻视图层才挂载 `Tooltip`,气泡随即弹出;离开即 `show` 为假,直接返回
/// 裸 `content`,气泡消失。`position` 由调用方按页签位置定(顶栏页签用
/// `Bottom`、底部面板页签用 `Top`,免得气泡出屏)。`label` 收 `String`(拥有
/// 所有权),使气泡 `Element` 寿命不受调用方局部借用牵制,`Tooltip` 才能正常
/// 把它当 overlay 渲染。
pub(crate) fn controlled_tooltip<'a, M, R>(
    content: Element<'a, M, iced_widget::Theme, R>,
    label: String,
    position: Position,
    show: bool,
) -> Element<'a, M, iced_widget::Theme, R>
where
    M: Clone + 'a,
    R: iced_widget::core::text::Renderer + 'a,
{
    // 未悬停满 2s:不包 tooltip,直接返回裸内容,避免一悬停就弹气泡打扰。
    if !show {
        return content;
    }
    let bubble = container(
        text(label)
            .size(12)
            .color(byteui::theme::color::current().cream),
    )
    .padding([5, 9]);
    Tooltip::new(content, bubble, position)
        .gap(4)
        .style(icons::tooltip_bubble_style())
        .into()
}

/// `host_id` → `HoverId::SshTab{Item,Close}` 用的哈希键(`HoverId` 整体
/// `derive(Copy)`,`String` 不是 `Copy`,退化成 `u64`,不要求无碰撞——
/// 碰撞在同一台主机的 tab hover 高亮场景下不构成实际风险)。
pub(crate) fn ssh_tab_hover_key(host_id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    host_id.hash(&mut hasher);
    hasher.finish()
}

/// SSH 面板自己的 tab 条:固定一个"空白"占位 tab 打头,后面遍历
/// `ws.ssh_tabs`/`ws.sftp_tabs`,每个渲染一个可关闭 tab。直接复用
/// `panel_tab`(右侧共享终端条 `tab_item` 用的同一个函数)而不是自己拼
/// 容器样式,视觉/hover 动画与全应用其它 tab 完全一致——不需要
/// `tabs::tab_core` 手动接线。前缀图标固定用 `IconKind::Terminal`(阶段
/// 4 只有这一种;阶段 3 加 `Sftp` 变体后按 tab 的种类换图标,`SessionTab`
/// 本身不带 `SshTabKind` 字段,种类信息只在 `ws.ssh_active` 里——阶段 4
/// 全部 `ssh_tabs` 里的 tab 都是 `Terminal` 种类,这里暂时不需要按 tab
/// 查种类,阶段 3 扩展这个函数时才需要处理"同一个 host_id 可能对应两个
/// 不同种类的 tab,要分别渲染两个 tab 条目"这件事)。
///
/// 翻页箭头 + 窗口化裁剪(P1L T5 那套 `tab_window` 索引窗口)镜像
/// `workspace.rs::preview_pane_for`:先把全部 tab 元素连同估算宽度收进
/// `entries`,再用 `tab_window` 算出可视窗口起点 `first`,只渲染
/// `entries[first..]`,左右箭头到头置灰。原先没有这套窗口化,tab 一多
/// 就会被右侧"收起列表"按钮的 `clip` 直接裁没、连滚动入口都没有。
pub(crate) fn ssh_tab_bar<'a>(
    app: &'a App,
    ws: &'a Workspace,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut entries: Vec<(
        f32,
        Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    )> = Vec::new();
    // "空白"占位 tab:不对应 `ssh_tabs`/`sftp_tabs` 里任何一条记录,选中
    // 态即 `ssh_active == None`(未开任何主机 tab,或关到最后一个后的
    // 默认落点)。跟文件预览面板 `preview.rs::TabKind::Blank` 是同一个
    // 产品概念,但这边没有对应的轻量 tab 数据可插进 `ssh_tabs`,所以只在
    // 这里画一个固定条目,内容侧靠 `ssh_active == None` 分支渲染
    // `ssh_empty_state()`,不需要真的建一个 tab 结构体。用 `""` 当 hover
    // key(真实 host_id 是 UUID,不会是空串,不会撞)。
    let blank_key = ssh_tab_hover_key("");
    let blank_active = ws.ssh_active.is_none();
    entries.push((
        tab_display_width("空白"),
        tab_widget::panel_tab(tab_widget::PanelTabArgs {
            title: "空白".to_string(),
            active: blank_active,
            hover_t: app.hover_progress(HoverId::SshTabItem(blank_key)),
            close_hover_t: app.hover_progress(HoverId::SshTabClose(blank_key)),
            prefix: None,
            suffix: None,
            on_select: Message::Ssh(ssh::Message::SelectBlankTab),
            on_close: Message::Ssh(ssh::Message::SelectBlankTab),
            show_tooltip: app.hover_tooltip_ready(HoverId::SshTabItem(blank_key)),
            title_hover: move |h| Message::Hover(HoverId::SshTabItem(blank_key), h),
            close_hover: move |h| Message::Hover(HoverId::SshTabClose(blank_key), h),
        }),
    ));
    for tab in &ws.ssh_tabs {
        let host_id = tab
            .info
            .id
            .strip_prefix("ssh:")
            .unwrap_or(&tab.info.id)
            .to_string();
        let is_active = ws
            .ssh_active
            .as_ref()
            .is_some_and(|(h, k)| h == &host_id && *k == ssh::SshTabKind::Terminal);
        let key = ssh_tab_hover_key(&host_id);
        let title_hover_t = app.hover_progress(HoverId::SshTabItem(key));
        let close_hover_t = app.hover_progress(HoverId::SshTabClose(key));
        let icon = icons::view(
            icons::IconKind::Terminal,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().dim,
        );
        let select_id = host_id.clone();
        let close_id = host_id.clone();
        let title_hover_id = host_id.clone();
        let close_hover_id = host_id;
        let title = tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name);
        entries.push((
            tab_display_width(&title),
            tab_widget::panel_tab(tab_widget::PanelTabArgs {
                title,
                active: is_active,
                hover_t: title_hover_t,
                close_hover_t,
                prefix: Some(icon),
                suffix: None,
                on_select: Message::Ssh(ssh::Message::SelectSshTab(
                    select_id,
                    ssh::SshTabKind::Terminal,
                )),
                on_close: Message::Ssh(ssh::Message::CloseSshTab(
                    close_id,
                    ssh::SshTabKind::Terminal,
                )),
                show_tooltip: app.hover_tooltip_ready(HoverId::SshTabItem(key)),
                title_hover: move |h| {
                    Message::Hover(HoverId::SshTabItem(ssh_tab_hover_key(&title_hover_id)), h)
                },
                close_hover: move |h| {
                    Message::Hover(HoverId::SshTabClose(ssh_tab_hover_key(&close_hover_id)), h)
                },
            }),
        ));
    }
    // SFTP tab(阶段 3):`sftp_tabs` 按 host_id 去重,渲染形状跟终端 tab
    // 一致(复用 `panel_tab`/`tab_core`),只是图标用 FolderSync、标题用主机名。
    for (host_id, state) in &ws.sftp_tabs {
        let is_active = ws.ssh_active.as_ref() == Some(&(host_id.clone(), ssh::SshTabKind::Sftp));
        let label = ws
            .ssh
            .hosts()
            .iter()
            .find(|h| &h.id == host_id)
            .map(|h| h.name.clone())
            .unwrap_or_else(|| host_id.clone());
        let key = ssh_tab_hover_key(host_id);
        let title_hover_t = app.hover_progress(HoverId::SshTabItem(key));
        let close_hover_t = app.hover_progress(HoverId::SshTabClose(key));
        let icon = icons::view(
            icons::IconKind::FolderSync,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().dim,
        );
        let select_id = host_id.clone();
        let close_id = host_id.clone();
        let title_hover_id = host_id.clone();
        let close_hover_id = host_id.clone();
        entries.push((
            tab_display_width(&label),
            tab_widget::panel_tab(tab_widget::PanelTabArgs {
                title: label,
                active: is_active,
                hover_t: title_hover_t,
                close_hover_t,
                prefix: Some(icon),
                suffix: None,
                on_select: Message::Ssh(ssh::Message::SelectSshTab(
                    select_id,
                    ssh::SshTabKind::Sftp,
                )),
                on_close: Message::Ssh(ssh::Message::CloseSshTab(close_id, ssh::SshTabKind::Sftp)),
                show_tooltip: app.hover_tooltip_ready(HoverId::SshTabItem(key)),
                title_hover: move |h| {
                    Message::Hover(HoverId::SshTabItem(ssh_tab_hover_key(&title_hover_id)), h)
                },
                close_hover: move |h| {
                    Message::Hover(HoverId::SshTabClose(ssh_tab_hover_key(&close_hover_id)), h)
                },
            }),
        ));
        let _ = state;
    }
    let widths: Vec<f32> = entries.iter().map(|(w, _)| *w).collect();
    let window = tab_widget::tab_window(
        &widths,
        4.0,
        byteui::theme::geometry::tab_bar_avail_px(),
        ws.ssh_tab_first,
    );
    let items: Vec<_> = entries
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| (window.first..window.visible_end).contains(idx))
        .map(|(_, (_, el))| el)
        .collect();
    // tab 条本身占 Fill、裁掉右侧溢出,让"收起列表"钉在裁剪区外的最右侧
    // (镜像 `workspace.rs::preview_pane_for` 的 `clipped`/`collapse` 布局
    // ——之前 `bar` 整体是 `Shrink`,收起按钮只是跟在最后一个 tab 后面,
    // tab 少时会贴在中间而不是面板右边缘,验收反馈要求钉死在右侧)。
    let clipped = container(row(items).spacing(4))
        .width(Length::Fill)
        .clip(true);
    // V 只数**真实** tab(终端 + SFTP),不算"空白"占位——下拉本就不列空白
    // (点开也没什么可跳的,见 `ssh_tab_overflow_popup`),只剩空白页时 V
    // 本身也不该显示(验收反馈)。
    let ssh_tab_total = ws.ssh_tabs.len() + ws.sftp_tabs.len();
    let overflow_button = tab_widget::tab_overflow_button(
        ssh_tab_total,
        app.hover_progress(HoverId::SshTabOverflow),
        Message::Ssh(ssh::Message::TabOverflowToggle),
        move |hovered| Message::Hover(HoverId::SshTabOverflow, hovered),
    );
    // 内容侧"收起/展开列表列"按钮(收起左列主机列表后仍在此可见以便恢复)。
    let collapse = app.list_collapse_button(
        PanelKind::Ssh,
        app.list_collapsed(PanelKind::Ssh),
        HoverId::SshListCollapse,
        "收起列表",
        "展开列表",
        Message::TogglePanelListCollapse(PanelKind::Ssh),
        move |hovered| Message::Hover(HoverId::SshListCollapse, hovered),
    );
    let mut tab_bar_row = row![]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);
    if let Some(btn) = overflow_button {
        tab_bar_row = tab_bar_row.push(btn);
    }
    let tab_bar = tab_bar_row.push(clipped).push(collapse);

    let base: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> = tab_bar.into();
    base
}

/// SSH 面板 tab 栏"溢出下拉"浮层。**必须**在 `App::view` 顶层
/// `stack![base, ...]` 里拼(同 `terminal::term_tab_overflow_popup` 文档
/// 解释的理由——`anchor`/`window_size` 是全窗口坐标系,嵌在 `ssh_tab_bar`
/// 自己的局部布局里换算位置会跟真实点击位置对不上)。
pub(crate) fn ssh_tab_overflow_popup<'a>(
    app: &'a App,
    ws: &'a Workspace,
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let anchor = ws.ssh_tab_overflow_anchor?;
    // 下拉列出该 SSH 面板内**真实** tab(终端 + SFTP),即 horizontal tab
    // 条之上的完整视图——点 V 不是为了翻越隐藏项,而是一览/跳到任意 tab。
    // "空白"占位不进列表(点开也没什么可跳的);下标沿用
    // `ssh_tab_overflow_select_message`/`_close_message` 既有的 1 起步方案
    // (0 留给空白,虽然它现在不会出现在列表里,翻译函数不用跟着改)。
    let mut entries: Vec<tab_widget::TabOverflowEntry<'_, Message>> = Vec::new();
    for (i, tab) in ws.ssh_tabs.iter().enumerate() {
        let idx = i + 1;
        let host_id = tab
            .info
            .id
            .strip_prefix("ssh:")
            .unwrap_or(&tab.info.id)
            .to_string();
        entries.push(tab_widget::TabOverflowEntry {
            index: idx,
            prefix: Some(icons::view(
                icons::IconKind::Terminal,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().dim,
            )),
            title: tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name),
            active: ws
                .ssh_active
                .as_ref()
                .is_some_and(|(h, k)| h == &host_id && *k == ssh::SshTabKind::Terminal),
            closable: true,
            hover_t: app.hover_progress(HoverId::TabOverflowRow(idx)),
        });
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
        entries.push(tab_widget::TabOverflowEntry {
            index: idx,
            prefix: Some(icons::view(
                icons::IconKind::FolderSync,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().dim,
            )),
            title: label,
            active: ws.ssh_active.as_ref() == Some(&(host_id.clone(), ssh::SshTabKind::Sftp)),
            closable: true,
            hover_t: app.hover_progress(HoverId::TabOverflowRow(idx)),
        });
    }
    Some(tab_widget::tab_overflow_menu(
        tab_widget::TabOverflowMenuArgs {
            entries,
            anchor,
            window_size: app.window_size,
            on_select: |idx| ssh_tab_overflow_select_message(ws, idx),
            on_close: |idx| ssh_tab_overflow_close_message(ws, idx),
            on_dismiss: Message::Ssh(ssh::Message::TabOverflowDismiss),
            on_row_hover: move |idx, hovered| Message::Hover(HoverId::TabOverflowRow(idx), hovered),
        },
    ))
}

/// 把溢出下拉的扁平下标翻回 SSH 的 `(host_id, kind)`：下标 0 = 空白占位
/// tab；`1..=ssh_tabs.len()` 是终端段；再往后是 SFTP 段。越界兜底回空白态。
pub(crate) fn ssh_tab_overflow_select_message(ws: &Workspace, idx: usize) -> Message {
    if idx == 0 {
        return Message::Ssh(ssh::Message::SelectBlankTab);
    }
    let terminal_count = ws.ssh_tabs.len();
    if idx <= terminal_count {
        let tab = &ws.ssh_tabs[idx - 1];
        let host_id = tab
            .info
            .id
            .strip_prefix("ssh:")
            .unwrap_or(&tab.info.id)
            .to_string();
        return Message::Ssh(ssh::Message::SelectSshTab(
            host_id,
            ssh::SshTabKind::Terminal,
        ));
    }
    match ws.sftp_tabs.keys().nth(idx - 1 - terminal_count) {
        Some(host_id) => Message::Ssh(ssh::Message::SelectSshTab(
            host_id.clone(),
            ssh::SshTabKind::Sftp,
        )),
        None => Message::Ssh(ssh::Message::SelectBlankTab),
    }
}

/// `ssh_tab_overflow_select_message` 的关闭版。空白占位 `closable: false`
/// 保证下拉里它没有 x，走到这只能是兜底，发顶层 no-op。
pub(crate) fn ssh_tab_overflow_close_message(ws: &Workspace, idx: usize) -> Message {
    if idx == 0 {
        return Message::Noop;
    }
    let terminal_count = ws.ssh_tabs.len();
    if idx <= terminal_count {
        let tab = &ws.ssh_tabs[idx - 1];
        let host_id = tab
            .info
            .id
            .strip_prefix("ssh:")
            .unwrap_or(&tab.info.id)
            .to_string();
        return Message::Ssh(ssh::Message::CloseSshTab(
            host_id,
            ssh::SshTabKind::Terminal,
        ));
    }
    match ws.sftp_tabs.keys().nth(idx - 1 - terminal_count) {
        Some(host_id) => Message::Ssh(ssh::Message::CloseSshTab(
            host_id.clone(),
            ssh::SshTabKind::Sftp,
        )),
        None => Message::Noop,
    }
}

/// SSH 面板内嵌终端区:tab 条 + 终端画布(或空态)。镜像 `preview_pane`/
/// `project_preview_pane` 的既有模式——渲染函数不属于 `extensions::ssh`
/// 模块,因为它要用顶层 `Message` 直接操作 `ws.ssh_tabs`。
pub(crate) fn ssh_terminal_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::terminal_pane();
    let body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match ws.ssh_active.as_ref() {
            Some((host_id, ssh::SshTabKind::Terminal)) => {
                let tab = ws
                    .ssh_tabs
                    .iter()
                    .find(|t| t.info.id.strip_prefix("ssh:") == Some(host_id.as_str()));
                match tab {
                    Some(tab) => {
                        let focused =
                            terminal::keyboard_term_target(app.left_view, app.active_zone)
                                == terminal::TermTarget::SshPanel;
                        term_view::view(
                            &tab.model,
                            focused,
                            terminal::TermTarget::SshPanel,
                            focused.then(|| app.term_ime_preedit()).flatten(),
                        )
                    }
                    None => ssh_empty_state(),
                }
            }
            Some((host_id, ssh::SshTabKind::Sftp)) => match ws.sftp_tabs.get(host_id) {
                Some(state) => {
                    ssh::sftp::sftp_pane_view(state).map(|m| Message::Ssh(ssh::Message::Sftp(m)))
                }
                None => ssh_empty_state(),
            },
            None => ssh_empty_state(),
        };
    container(
        column![ssh_tab_bar(app, ws), tab_divider(), body]
            .spacing(region.gap)
            .height(Length::Fill),
    )
    .width(width)
    .padding(region.padding)
    .style(move |_t: &iced_widget::Theme| container::Style {
        background: region.background.map(Into::into),
        border: outer,
        ..container::Style::default()
    })
    .into()
}

/// "空白" tab(`ssh_active == None`)选中时的内容:跟文件预览面板
/// `preview.rs::TabKind::Blank` 是同一套视觉语言——居中放 Dozer 品牌标 +
/// 引导文案。SSH 这边每个真实 tab 都对应一条 PTY/SFTP 连接
/// (`SessionTab`/`SftpTabState`),没有轻量数据能塞进 `ws.ssh_tabs` 去
/// 表示"空白",所以"空白" tab 只在 `ssh_tab_bar()` 里画一个固定条目,
/// 内容侧靠 `ssh_active == None` 这个分支渲染,不是真的建一个 tab 结构体
/// (对照 preview 那边"tab 数据里有一个 `TabKind::Blank` 变体"的做法)。
pub(crate) fn ssh_empty_state<'a>()
-> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    container(
        column![
            icons::view(
                icons::IconKind::Dozer,
                72.0,
                byteui::theme::color::current().dim,
            ),
            text("点主机卡片的终端/文件传输图标开始")
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        ]
        .spacing(14)
        .align_x(iced_widget::core::alignment::Horizontal::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(iced_widget::core::alignment::Horizontal::Center)
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .into()
}
