//! Files 面板 update 消息分发 + git 信息加载 spawn。

use crate::git_status;

use dozer_panel_host::{HostRequest, PanelCommand};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::Message;
use crate::adapter::{FilesCapabilities, FilesRequest};
use crate::state::{AppState, GitInfo, PendingMove, TreeEdit, TreeEditMode, WorkspaceState};
use crate::menu::{context_menu_items, tab_context_menu_items};
use crate::tree::{TreeDragPhase, is_valid_move_target};
use crate::menu::FileContextMenuParams;
use crate::{fs, i18n};
use crate::external_apps;

/// 面板日志作用域名。库不依赖 `dozer-core`,产品自己用它建 `log::Scope`。
pub const LOG_NAME: &str = "files";

/// 右键"添加到 Agent 上下文"的负载:项目内相对路径(与"复制相对路径"同一套
/// `path_string` 逻辑,原样不转义)。模板文本留给内核层按当前 locale 拼
/// (见 `App::update` 的 `RequestSendToAgentTerminal` 分支)。
fn send_payload(root: &Path, target: &Path, is_dir: bool) -> (String, bool) {
    let relative = fs::path_string(fs::PathKind::Relative, target, root);
    (relative, is_dir)
}

/// 处理 `CopyPath` 之外的全部消息。内核在到达这里之前已经拦截了
/// `CopyPath`(真正的剪贴板写入需要 `main.rs` 的 `Clipboard` 句柄),它传
/// 进来会 `unreachable!`(同 Todo 试点 `DispatchToExisting`/`DispatchNew`
/// 的处理方式)。`project_id` 由内核路由时已经解析出正确值传入。
/// 参数天然异质(状态/消息/句柄/回调/配置/locale),无相邻同型易错位,
/// 保持位置参数、不加具名字段结构体(见 `database/update.rs` 同款处理)。
#[allow(clippy::too_many_arguments)]
pub fn update(
    ws_state: &mut WorkspaceState,
    app_state: &mut AppState,
    msg: Message,
    project_id: i64,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
    external_apps: &external_apps::ExternalAppsConfig,
    caps: &FilesCapabilities,
    locale: &str,
) {
    match msg {
        Message::Toggle(dir) => {
            ws_state.select_single(dir.clone());
            if let Some(tree) = &mut ws_state.file_tree {
                tree.toggle(&dir);
            }
        }
        Message::ToggleNoSelect(dir) => {
            if let Some(tree) = &mut ws_state.file_tree {
                tree.toggle(&dir);
            }
        }
        Message::StatusesRefreshed(project_id, statuses) => {
            ws_state.dir_statuses = git_status::rollup_dir_statuses(&statuses);
            ws_state.git_statuses = statuses;
            // 项目 git 状态更新(git_watch 拾起 HEAD/refs 变化后)常伴随分支
            // 切换,顺手把分支栏的仓库信息一并刷新,让底栏与树保持一致。
            spawn_git_info_load(ws_state, project_id, handle, emit);
        }
        Message::RightClickAt { x, y } => {
            app_state.last_right_click = (x, y);
        }
        Message::ContextMenuOpen { path, is_dir } => {
            // 右键落在已有选区内:保留现有多选(高亮不变),只把锚点挪到被右
            // 键的那一项,方便后续 Shift+点击以它为起点延展;右键落在选区外:
            // 退化为单选这一项(同普通点击,避免右键把整片多选悄悄改成只选
            // 一个又没高亮反馈)。右键菜单项动作本身仍只针对 `path` 这一个
            // 目标——批量操作(多选后删/复制整组)超出本次范围。
            if !ws_state.tree_selected.contains(&path) {
                ws_state.select_single(path.clone());
            } else {
                ws_state.tree_selection_anchor = Some(path.clone());
            }
            // 与分支切换弹层互斥:开右键菜单时收起分支弹层,避免两个浮层
            // 同时挂着(同 `PreviewTabContextMenu` 关文件树右键菜单的约定)。
            ws_state.branch_picker_open = false;
            // 与预览 tab 右键菜单互斥:开文件树菜单时收起 tab 菜单,二者同为
            // 文件相关右键浮层,不能同挂(见 `TabContextMenuOpen` 的清理约定)。
            app_state.tab_context_menu = None;
            let (x, y) = app_state.last_right_click;
            let is_root = ws_state
                .file_tree
                .as_ref()
                .map(|t| t.root() == path.as_path())
                .unwrap_or(false);
            let has_clipboard = ws_state.tree_clipboard.is_some();
            let items = context_menu_items(
                FileContextMenuParams {
                    target: &path,
                    is_dir,
                    is_root,
                    has_clipboard,
                    is_git_repo: ws_state.git_is_repo,
                    external_apps,
                },
                caps,
                locale,
            );
            // 弹菜单交给产品:macOS 走原生的产品会把它渲染成 NSMenu、经回调
            // 把选中的 `Message` 送回 `update`;其余产品退回自己的 iced 浮层。
            // 两条路都不在这里递推 `update`——菜单项里的 `OpenSearch`/
            // `CopyPath` 是内核拦截的,必须经 `emit` 走回顶层。
            ws_state.requests.push(FilesRequest::ShowNativeMenu {
                items,
                at: (x, y),
            });
        }
        Message::ContextMenuClose => {
            app_state.context_menu = None;
            app_state.tab_context_menu = None;
        }
        Message::SendToAgentContext(target, is_dir) => {
            // 真正的 PTY 写入需要 `main.rs` 的终端句柄,`files::update` 拿不到,
            // 同 `CopyPath`/`OpenSearch` 的既有模式——把拼好的文本经 `emit`
            // 回内核顶层,由 `App::update` 拦截并调 `term_paste`。
            let Some(tree) = ws_state.file_tree.as_ref() else {
                return;
            };
            let (relative, is_dir) = send_payload(tree.root(), &target, is_dir);
            emit(Message::RequestSendToAgentTerminal { is_dir, relative });
        }
        Message::RequestSendToAgentTerminal { .. } => {
            // 内核拦截处理(`App::update` 的 `Message::Files(files::Message::
            // RequestSendToAgentTerminal)` 分支),永远不该落回这里。
            unreachable!("RequestSendToAgentTerminal 由内核拦截处理");
        }
        Message::TabContextMenuOpen { kind, path } => {
            // 与文件树右键菜单/分支切换弹层互斥:开 tab 菜单时一并收起两者
            // (`close_context_menu` 清文件树菜单 + tab 菜单本身;branch 单独清),
            // 避免两个浮层同挂导致 dismiss 串味。
            ws_state.branch_picker_open = false;
            app_state.close_context_menu();
            let (x, y) = app_state.last_right_click;
            let is_git_repo = ws_state.git_is_repo;
            let items = tab_context_menu_items(
                kind,
                &path,
                is_git_repo,
                external_apps,
                caps,
                locale,
            );
            ws_state.requests.push(FilesRequest::ShowNativeMenu {
                items,
                at: (x, y),
            });
        }
        Message::TabContextMenuClose => {
            app_state.context_menu = None;
            app_state.tab_context_menu = None;
        }
        Message::OpenWithDefault(path, app_name) => {
            app_state.close_context_menu();
            // `app_name` 为 `None` 退回系统默认打开方式(`open <path>`,不带 `-a`)。
            // `open -a 不存在的App` 的失败发生在 `open` 进程退出之后(`spawn()` 本身
            // 会成功),所以后台等退出状态,失败经 `OpenWithDefaultDone` 回来进 outbox。
            let mut command = std::process::Command::new("open");
            if let Some(app_name) = &app_name {
                command.arg("-a").arg(app_name);
            }
            command.arg(&path);
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    external_apps::run_open_command(command)
                })
                .await
                .unwrap_or_else(|e| {
                    Err(external_apps::OpenError::Spawn {
                        program: "open".to_string(),
                        error: e.to_string(),
                    })
                });
                emit(Message::OpenWithDefaultDone(
                    project_id, path, app_name, result,
                ));
            });
        }
        Message::OpenWithDefaultDone(_, path, app_name, result) => {
            if let Err(reason) = result {
                // Toast 会把完整文案写进日志(含文件名与原因),这里不再重复写。
                ws_state.outbox.push(                    dozer_panel_host::NoticeLevel::Error,
                    external_apps::open_failure_message(app_name.as_deref(), &path, &reason),
                );
            }
        }
        Message::Copy(path, is_dir) => {
            app_state.context_menu = None;
            ws_state.tree_clipboard = Some((path, is_dir));
        }
        Message::Paste(target_dir) => {
            app_state.context_menu = None;
            let Some((source, source_is_dir)) = ws_state.tree_clipboard.clone() else {
                return;
            };
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    fs::paste_item(&source, source_is_dir, &target_dir)
                })
                .await
                .unwrap_or_else(|e| {
                    Err(fs::ProjectFsError::Io {
                        detail: e.to_string(),
                    })
                });
                emit(Message::PasteDone(project_id, result));
            });
        }
        Message::PasteDone(_, result) => match result {
            Ok(new_path) => {
                if let (Some(tree), Some(parent)) = (&mut ws_state.file_tree, new_path.parent()) {
                    tree.refresh(parent);
                }
            }
            Err(e) => ws_state.outbox.push(                dozer_panel_host::NoticeLevel::Error,
                i18n::keyed_args(
                    "toast.paste_failed",
                    "Paste failed: {e}",
                    &[("e", e.stable().as_str())],
                ),
            ),
        },
        Message::FileHistoryRollbackDone(_project_id, abs_path, result) => match result {
            Ok(()) => {
                // 还原成功:刷新被回滚文件所在的父目录,让树里该文件的状态
                // (大小/可能的新增标记)重新读盘。
                if let (Some(tree), Some(parent)) = (&mut ws_state.file_tree, abs_path.parent()) {
                    tree.refresh(parent);
                }
            }
            Err(e) => ws_state.outbox.push(                dozer_panel_host::NoticeLevel::Error,
                i18n::keyed_args(
                    "toast.rollback_failed",
                    "Restore file failed: {e}",
                    &[("e", e.as_str())],
                ),
            ),
        },
        Message::DeleteRequest(path, is_dir) => {
            app_state.context_menu = None;
            // 右键菜单只携带单一 `path`,但若 `path` 属于当前多选集(且选中多于
            // 一项),则把整组选中项一并纳入待删除——实现"多选后删整组"。项目
            // 根恒排除(不能删项目根)。单选(或 `path` 不在选中集里)时退化为只
            // 删 `path` 这一项。
            let root = ws_state.file_tree.as_ref().map(|t| t.root().to_path_buf());
            let targets: Vec<(PathBuf, bool)> =
                if ws_state.tree_selected.contains(&path) && ws_state.tree_selected.len() > 1 {
                    let visible: HashMap<PathBuf, bool> = ws_state
                        .visible_tree_rows()
                        .into_iter()
                        .map(|r| (r.path, r.is_dir))
                        .collect();
                    ws_state
                        .tree_selected
                        .iter()
                        .filter(|p| Some(p.as_path()) != root.as_deref())
                        .map(|p| {
                            let is_dir = visible.get(p).copied().unwrap_or_else(|| p.is_dir());
                            (p.clone(), is_dir)
                        })
                        .collect()
                } else {
                    vec![(path, is_dir)]
                };
            if targets.is_empty() {
                // 极端情况:整组只剩根——无任何可删项,不弹确认框。
                return;
            }
            ws_state.tree_delete_confirm = Some(targets);
        }
        Message::DeleteCancel => {
            ws_state.tree_delete_confirm = None;
        }
        Message::DeleteConfirm => {
            let Some(targets) = ws_state.tree_delete_confirm.take() else {
                return;
            };
            if targets.is_empty() {
                return;
            }
            handle.spawn(async move {
                let results: Vec<(PathBuf, std::result::Result<(), String>)> =
                    tokio::task::spawn_blocking(move || {
                        targets
                            .into_iter()
                            .map(|(path, _)| {
                                let result = trash::delete(&path).map_err(|e| e.to_string());
                                (path, result)
                            })
                            .collect()
                    })
                    .await
                    .unwrap_or_else(|e| vec![(PathBuf::new(), Err(e.to_string()))]);
                emit(Message::DeleteApplied(project_id, results));
            });
        }
        Message::DeleteApplied(_, results) => {
            for (path, result) in results {
                match result {
                    Ok(()) => {
                        if let Some(parent) = path.parent()
                            && let Some(tree) = &mut ws_state.file_tree
                        {
                            tree.refresh(parent);
                        }
                    }
                    Err(e) => {
                        if path != std::path::Path::new("") {
                            ws_state.outbox.push(
                                dozer_panel_host::NoticeLevel::Error,
                                i18n::keyed_args(
                                    "toast.delete_failed",
                                    "Delete {name} failed: {e}",
                                    &[
                                        ("name", path.display().to_string().as_str()),
                                        ("e", e.as_str()),
                                    ],
                                ),
                            );
                        }
                    }
                }
            }
        }
        Message::OpDone { parent, expand, .. } => match parent {
            Ok(parent) => {
                if let Some(tree) = &mut ws_state.file_tree {
                    tree.refresh(&parent);
                    if expand {
                        tree.ensure_expanded(&parent);
                    }
                }
            }
            Err(e) => ws_state.outbox.push(                dozer_panel_host::NoticeLevel::Error,
                i18n::keyed_args(
                    "toast.op_failed",
                    "File operation failed: {e}",
                    &[("e", e.as_str())],
                ),
            ),
        },
        Message::NewFile(parent) => {
            app_state.context_menu = None;
            ws_state.start_tree_new(parent, TreeEditMode::NewFile);
        }
        Message::NewFolder(parent) => {
            app_state.context_menu = None;
            ws_state.start_tree_new(parent, TreeEditMode::NewFolder);
        }
        Message::ReloadFromDisk => {
            app_state.context_menu = None;
            if let Some(tree) = &mut ws_state.file_tree {
                tree.reload_from_disk();
            }
        }
        Message::SearchInput(s) => {
            ws_state.tree_search = s;
        }
        Message::SearchSubmit => {
            ws_state.search_query = ws_state.tree_search.clone();
            if let Some(tree) = &mut ws_state.file_tree {
                tree.clear_search();
            }
        }
        Message::ToggleDotfiles => {
            if let Some(tree) = &mut ws_state.file_tree {
                tree.set_show_dotfiles(!tree.dotfiles_shown());
            }
        }
        Message::GitInfoLoaded(_, info) => {
            ws_state.git_loaded = true;
            ws_state.git_is_repo = info.is_repo;
            ws_state.current_branch = info.current_branch;
            ws_state.current_branch_has_commits = info.current_branch_has_commits;
            ws_state.git_branches = info.branches;
        }
        Message::BranchPickerOpen => {
            // macOS 由内核 `App::update` 拦截(`files::update` 的 app_state
            // 够不到内核 `last_cursor`)直接弹原生 NSMenu;这里只剩非 macOS
            // 平台的 iced 弹层兜底(`branch_picker_popup`)。
            #[cfg(not(target_os = "macos"))]
            {
                ws_state.branch_picker_open = true;
            }
            ws_state.git_error = None;
        }
        Message::BranchPickerClose => {
            ws_state.branch_picker_open = false;
        }
        Message::BranchSwitch(name) => {
            ws_state.branch_picker_open = false;
            ws_state.git_error = None;
            let Some(tree) = &ws_state.file_tree else {
                return;
            };
            let root = tree.root().to_path_buf();
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    bytegit::Repo::discover(&root)
                        .and_then(|repo| repo.checkout_branch(&name))
                        .map_err(|e| e.message().to_string())
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                emit(Message::BranchSwitchDone(project_id, result));
            });
        }
        Message::BranchSwitchDone(_, result) => {
            match result {
                Ok(()) => {
                    // checkout 成功后 HEAD/refs 变化由 git_watch 拾起,会再次
                    // 触发 StatusesRefreshed → spawn_git_info_load。这里再主动
                    // 刷一次分支信息(脱离 watch 兜底,如未启动 watch 时)。
                    spawn_git_info_load(ws_state, project_id, handle, emit);
                }
                Err(e) => ws_state.git_error = Some(e),
            }
        }
        Message::GitInit => {
            ws_state.git_error = None;
            if let Some(tree) = &ws_state.file_tree {
                let root = tree.root().to_path_buf();
                handle.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        bytegit::init(&root)
                            .map(|_| ())
                            .map_err(|e| e.message().to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    emit(Message::GitInitDone(project_id, result));
                });
            }
        }
        Message::GitInitDone(_, result) => match result {
            Ok(()) => {
                ws_state.git_loaded = true;
                ws_state.git_is_repo = true;
                // 新建的空仓库没有提交/分支,分支栏仍显示"分支切换"态但当前
                // 分支为空;git status / 文件树颜色等后续由 git_watch 正常驱动。
                spawn_git_info_load(ws_state, project_id, handle, emit);
            }
            Err(e) => ws_state.git_error = Some(e),
        },
        Message::RenameStart(path) => {
            app_state.context_menu = None;
            let Some(parent) = path.parent().map(|p| p.to_path_buf()) else {
                return;
            };
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            ws_state.tree_edit = Some(TreeEdit {
                parent_dir: parent,
                mode: TreeEditMode::Rename(path),
                buffer: name,
            });
            ws_state.tree_edit_focus_pending = true;
        }
        Message::EditInput(s) => {
            let Some(edit) = &mut ws_state.tree_edit else {
                return;
            };
            edit.buffer = s;
        }
        Message::EditSubmit => {
            ws_state.submit_tree_edit(project_id, handle, emit);
        }
        Message::TextInputMenuOpen(_) => {
            unreachable!("由内核拦截处理,见 files::Message::TextInputMenuOpen 文档")
        }
        Message::CopyPath(..) => {
            unreachable!("由内核拦截处理,见 files::Message::CopyPath 文档")
        }
        // 右键菜单"搜索":先关右键菜单(否则搜索弹窗 dismiss 一关,旧菜单又冒回来),再请 host 在该路径下
        // 开搜索——目录按目录递归搜,文件只搜单文件(由 host 的 `SearchIn` 处理)。
        Message::OpenSearch(path, is_dir) => {
            app_state.close_context_menu();
            ws_state
                .host
                .push(HostRequest::Command(PanelCommand::SearchIn {
                    path,
                    is_dir,
                }));
        }
        // 右键"查看此文件历史":先收起右键菜单(同 OpenSearch 的既有约定),再请 host 打开历史。
        Message::FileHistoryOpen(path) => {
            app_state.close_context_menu();
            ws_state
                .host
                .push(HostRequest::Command(PanelCommand::ShowFileHistory { path }));
        }
        Message::FileHistoryRollbackPrevious(_) => {
            unreachable!("由 app 拦截处理,见 files::Message::FileHistoryRollbackPrevious 文档")
        }
        // 工具行 icon 按钮的 hover 由内核 `Message::Files` 分支转发到
        // `HoverId`(文件树面板不挂 App 的 hover 动画表),`update` 吃不到
        // 这里;保 no-op 分支维持 match 穷尽。
        Message::ToolbarHover(..) => {}
        // 文件树滚动偏移:写进 `tree_scroll` 供外部拖拽命中测试。
        Message::TreeScroll(off) => {
            ws_state.tree_scroll = off;
        }
        // 外部文件拖拽悬停命中目录集合:整行高亮这些目录为"拖入落点"。
        // `view` 读 `drag_hover` 渲染金色高亮边框。
        Message::FileDragHover(dirs) => {
            ws_state.drag_hover = dirs;
        }
        // 鼠标进入/离开树行:维护 hover 行,`view` 据此画行 hover 背景
        // (同菜单项 hover 的 `TAB_HOVER` 底色)。
        Message::TreeRowHover(path) => {
            ws_state.tree_hover = path;
        }
        // 外部文件被松开在某目录上:逐个移动(同 Finder 拖拽的 move 语义,
        // 跨文件系统在 `move_item` 里降级为复制+删源)。全部完成后 `emit`
        // `FileDropDone` 刷新目标目录。
        Message::FileDrop { paths, target } => {
            ws_state.move_error = None;
            ws_state.drag_hover = HashSet::new();
            if paths.is_empty() {
                return;
            }
            // 已经在项目内的文件不许走这条"外部文件拖拽"通路移动路径——核心
            // 裁决要求直接改动项目产物的入口收着给,不能让鼠标拖拽变成一个
            // 可以随手重排项目文件结构的隐藏功能(容易误操作,又绕开 AI 侧
            // 的变更记录)。整批只要有一个源在项目树内就整体拒绝,不做"部分
            // 生效"。
            if let Some(tree) = &ws_state.file_tree
                && let Some(bad) = paths.iter().find(|p| p.starts_with(tree.root()))
            {
                ws_state.outbox.push(                    dozer_panel_host::NoticeLevel::Warning,
                    i18n::keyed_args(
                        "toast.drop_in_project",
                        "{name} is already in the project; dragging to move files inside the project is not supported",
                        &[("name", bad.display().to_string().as_str())],
                    ),
                );
                return;
            }
            if let [only] = paths.as_slice() {
                // 单文件(main.rs 每次 `DroppedFile` 恒只带一个路径,见
                // `Message::FileDrop` 文档):弹确认框,不立即移动。
                let name = only
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                ws_state.pending_move = Some(PendingMove {
                    sources: vec![(only.clone(), only.is_dir())],
                    name_draft: name,
                    dir_draft: target.display().to_string(),
                });
                ws_state.move_focus_pending = true;
                return;
            }
            // 多文件拖入(理论上今天走不到——main.rs 每次 `DroppedFile` 恒只带
            // 一个路径,但 `paths` 留了多元素口子):与单文件同走 `PendingMove`
            // 确认框——全部待移动源列进 `sources`,确认时只让用户指目标目录、
            // 不改名(各用原名)。不再"直接移动",与单文件"拖拽移动要确认"的
            // 2026-09 既定行为保持一致。
            let mut sources: Vec<(PathBuf, bool)> = Vec::with_capacity(paths.len());
            for p in &paths {
                let is_dir = std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false);
                sources.push((p.clone(), is_dir));
            }
            ws_state.pending_move = Some(PendingMove {
                sources,
                name_draft: String::new(),
                dir_draft: target.display().to_string(),
            });
        }
        Message::FileDropDone(_, target, result) => match result {
            Ok(()) => {
                if let Some(tree) = &mut ws_state.file_tree {
                    tree.refresh(&target);
                }
            }
            Err(e) => ws_state.outbox.push(                dozer_panel_host::NoticeLevel::Error,
                i18n::keyed_args(
                    "toast.drop_failed",
                    "Drag-move failed: {e}",
                    &[("e", e.as_str())],
                ),
            ),
        },
        Message::MoveNameInput(s) => {
            if let Some(pending) = &mut ws_state.pending_move {
                pending.name_draft = s;
            }
        }
        Message::MoveDirInput(s) => {
            if let Some(pending) = &mut ws_state.pending_move {
                pending.dir_draft = s;
            }
        }
        // 拖拽移动确认框"到目录"旁的浏览按钮:本模块不认识系统对话框,交给 host 弹,起始目录用当前草稿。
        Message::MoveDirBrowse => {
            let start = ws_state.move_dir_draft().map(PathBuf::from);
            ws_state.host.push(HostRequest::PickDirectory {
                start,
                on_picked: |dir| Message::MoveDirInput(dir.display().to_string()),
            });
        }
        Message::MoveCancel => {
            ws_state.pending_move = None;
            ws_state.move_error = None;
        }
        // 校验同行内编辑框既有口径(`submit_tree_edit`):名字非空、不含路径
        // 分隔符;目标目录必须真实存在。任一失败都把 `pending_move` 放
        // 回去(草稿保留用户已输入的内容,只是换上校验后的值),对话框留
        // 在屏幕上,不当成"取消"处理。
        Message::MoveConfirm => {
            let Some(pending) = ws_state.pending_move.take() else {
                return;
            };
            ws_state.move_error = None;
            let single = pending.sources.len() == 1;
            let name = pending.name_draft.trim().to_string();
            let dir_text = pending.dir_draft.trim().to_string();
            // 只有单文件移动才校验"新名称"字段(多文件移动不改名,忽略它)。
            if single && (name.is_empty() || !fs::is_single_path_component(&name)) {
                ws_state.move_error = Some(i18n::keyed(
                    "move.error.name",
                    "Name cannot be empty or contain a path separator",
                ));
                ws_state.pending_move = Some(PendingMove {
                    name_draft: name,
                    ..pending
                });
                return;
            }
            let target_dir = PathBuf::from(&dir_text);
            if !target_dir.is_dir() {
                ws_state.move_error = Some(i18n::keyed_args(
                    "move.error.dir",
                    "{path} is not a valid directory",
                    &[("path", target_dir.display().to_string().as_str())],
                ));
                ws_state.pending_move = Some(PendingMove {
                    dir_draft: dir_text,
                    ..pending
                });
                return;
            }
            // 为每个源算出目标路径:单文件用 `name`(允许改名),多文件用各自原名。
            // 跳过无意义/非法落点(原地、移进自身子树),其余交给 `move_item_to`。
            let mut ops: Vec<(PathBuf, bool, PathBuf)> = Vec::with_capacity(pending.sources.len());
            for (src, is_dir) in &pending.sources {
                let dest_name = if single {
                    name.clone()
                } else {
                    src.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                };
                let dest = target_dir.join(&dest_name);
                // 路径压根没变(没改名也没改目录,原地确认)、或目录被改成要移进
                // 它自己的子树——这类对单文件是"什么都没变",对多文件是其中一种
                // 源碰巧原地;都静默跳过(不提示错误、不动磁盘),不吓用户一跳
                // (2026-09 用户实测反馈:这类情形不该弹错误)。
                if dest == *src || (*is_dir && dest.starts_with(src)) {
                    continue;
                }
                ops.push((src.clone(), *is_dir, dest));
            }
            if ops.is_empty() {
                return;
            }
            let refresh_target = target_dir.clone();
            handle.spawn(async move {
                let mut errs: Vec<String> = Vec::new();
                for (src, is_dir, dest) in ops {
                    if let Err(e) = fs::move_item_to(&src, is_dir, &dest) {
                        errs.push(format!("{}: {}", src.display(), e.stable()));
                    }
                }
                let result = if errs.is_empty() {
                    Ok(())
                } else {
                    Err(errs.join("；"))
                };
                emit(Message::FileDropDone(project_id, refresh_target, result));
            });
        }
        Message::TreeRowPress { .. } => {
            unreachable!("由内核拦截处理,见 files::Message::TreeRowPress 文档")
        }
        Message::TreeDragRelease { .. } => {
            unreachable!("由内核拦截处理,见 files::Message::TreeDragRelease 文档")
        }
        Message::TreeRowDoubleClick { .. } => {
            unreachable!("由内核拦截处理,见 files::Message::TreeRowDoubleClick 文档")
        }
        // 树内拖拽悬停:悬停命中的行(文件或目录都可,2026-09 用户实测反馈
        // "悬浮到文件上也该有高亮")真正的落点目录若命中的是目录本身即为
        // 目标;命中文件则退到其父目录(同 Finder"拖到某个文件上=拖进它
        // 所在文件夹"的既有语义,文件所在目录既然渲染出这一行就必然已经
        // 展开,退到父目录永远是"已展开"的 no-op)。只在落点合法(非自身/
        // 自身子树,见 `is_valid_move_target`)时记为待定目标并高亮命中的
        // 那一行——复用外部拖拽同一份 `drag_hover` 渲染,不用另画一套。
        // 悬停到折叠的目录不立即展开,武装 `DRAG_HOVER_EXPAND_DELAY` 计时,
        // 真正展开推迟到 `App::advance_drag_hover_expand` 满时才做(见
        // `drag_expand_pending` 文档:立即展开会让拖着划过沿途目录疯狂
        // 跳动布局)。
        // 只有 `Dragging` 阶段的行才会挂 `on_move`(见 `view()`),所以这条
        // 消息到达时 `drag.phase` 理论上总是 `Dragging`——仍用 `let ... else`
        // 防御性处理,不假设调用方永远遵守约定。
        Message::TreeDragOver(hovered) => {
            let is_root = ws_state
                .file_tree
                .as_ref()
                .is_some_and(|t| t.root() == hovered);
            let hovered_row = ws_state
                .visible_tree_rows()
                .into_iter()
                .find(|r| r.path == hovered);
            let hovered_is_dir = is_root || hovered_row.as_ref().is_some_and(|r| r.is_dir);
            let already_expanded = is_root || hovered_row.is_none_or(|r| r.expanded);
            let Some(drag) = &mut ws_state.tree_drag else {
                return;
            };
            let TreeDragPhase::Dragging { target: current } = &mut drag.phase else {
                return;
            };
            let target_dir = if hovered_is_dir {
                Some(hovered.clone())
            } else {
                hovered.parent().map(Path::to_path_buf)
            };
            let Some(target_dir) = target_dir else {
                *current = None;
                ws_state.drag_hover = HashSet::new();
                ws_state.clear_drag_expand();
                return;
            };
            if is_valid_move_target(&drag.source, drag.source_is_dir, &target_dir) {
                *current = Some(target_dir);
                ws_state.drag_hover = std::iter::once(hovered.clone()).collect();
                if hovered_is_dir {
                    ws_state.arm_drag_expand(hovered, already_expanded);
                } else {
                    ws_state.clear_drag_expand();
                }
            } else {
                *current = None;
                ws_state.drag_hover = HashSet::new();
                ws_state.clear_drag_expand();
            }
        }
        // 树内拖拽松开:`confirmed` 就是"这场拖拽有没有走到 Dragging 阶段"
        // (由 `App::maybe_confirm_tree_drag` 在越过距离+时长两道阈值时
        // 推进,一次到位,松开时不用重新算——见 `TreeDragPhase` 文档)。
        // `!confirmed`(仍是 `Pending`):这其实只是一次单击,只选中——文件/
        // 目录的展开/打开都推迟到双击(`TreeRowDoubleClick`)或箭头
        // (`Message::Toggle`),不在这里做。
        // `confirmed` 且有合法待定目标:不立即移动,弹 `PendingMove` 确认框
        // (同 `FileDrop` 单文件那支,见其文档)——用户点"确定"才真正提交。
        Message::TreeDragEnd {
            confirmed,
            shift,
            command,
        } => {
            let Some(drag) = ws_state.tree_drag.take() else {
                return;
            };
            ws_state.drag_hover = HashSet::new();
            if !confirmed {
                // 只是一次单击:按住 Cmd/Ctrl 走不连续多选,按住 Shift 走区间
                // 多选,否则单选被点项。注意 `drag.source` 永远是"被按下的那一项",
                // 多选取这个锚点即可。
                if command {
                    ws_state.select_toggle(drag.source.clone());
                } else if shift {
                    ws_state.select_shift_range(drag.source.clone());
                } else {
                    ws_state.select_single(drag.source.clone());
                }
                return;
            }
            let TreeDragPhase::Dragging { target } = drag.phase else {
                return;
            };
            let Some(target) = target else {
                return;
            };
            // `drag.sources` 已被 `arm_tree_drag` 填好:若被按下项在选区里且
            // 选区>1,它就是整组选中项;否则就是单文件 `[被按项]`。
            let name = drag
                .sources
                .first()
                .and_then(|(p, _)| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let single = drag.sources.len() == 1;
            ws_state.move_error = None;
            ws_state.pending_move = Some(PendingMove {
                sources: drag.sources,
                name_draft: name,
                dir_draft: target.display().to_string(),
            });
            // 多文件移动不能重命名,自然也用不着抢焦点;单文件才进"新名称"输入框。
            if single {
                ws_state.move_focus_pending = true;
            }
        }
        // `TabReloadFromDisk` 由 app 层(`App::update`)拦截处理:在 Files/
        // Project 两个预览面板按路径找 tab 调 `PreviewPane::bump_reload`,不进
        // `files::update`。这里留空臂保持 match 穷尽(同 `CopyPath` 在 app 层
        // 写剪贴板的既定分工)。
        Message::TabReloadFromDisk(_) => {}
        // `TabContextMenuCloseTab` 由 app 层(`App::update`)拦截处理:在 Files/
        // Project 两个预览面板按路径找 tab 调 `PreviewPane::close`,不进
        // `files::update`。这里留空臂保持 match 穷尽(同 `TabReloadFromDisk` 的
        // app 层拦截口径)。
        Message::TabContextMenuCloseTab(_, _) => {}
    }
}

/// 异步加载一次 git 仓库信息:判项目根是否在仓库内、读当前分支、读本地分支
/// 表,结果通过 `emit(GitInfoLoaded(..))` 回投。走 `spawn_blocking` 防止阻塞
/// UI 线程(bytegit 的调用都是同步阻塞的)。
fn spawn_git_info_load(
    ws_state: &WorkspaceState,
    project_id: i64,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let Some(tree) = &ws_state.file_tree else {
        return;
    };
    let root = tree.root().to_path_buf();
    handle.spawn(async move {
        let info = tokio::task::spawn_blocking(move || load_git_info(&root))
            .await
            .unwrap_or_else(|_| GitInfo {
                is_repo: false,
                current_branch: None,
                current_branch_has_commits: false,
                branches: Vec::new(),
            });
        emit(Message::GitInfoLoaded(project_id, info));
    });
}

/// 分支栏用的 git 信息:`root` 所属的(非 bare)仓库;不在仓库里为 `is_repo: false`。
/// `root` 在仓库子目录里照样向上找到仓库。
pub fn load_git_info(root: &Path) -> GitInfo {
    match bytegit::Repo::discover_workdir(root) {
        Ok(repo) => {
            let head = repo.head().ok();
            GitInfo {
                is_repo: true,
                current_branch: head.as_ref().and_then(|h| h.branch.clone()),
                current_branch_has_commits: head.as_ref().is_some_and(|h| h.has_commits()),
                branches: repo.local_branches().unwrap_or_default(),
            }
        }
        Err(_) => GitInfo {
            is_repo: false,
            current_branch: None,
            current_branch_has_commits: false,
            branches: Vec::new(),
        },
    }
}

#[cfg(test)]
mod git_info_tests {
    use super::*;
    use bytegit::testutil::TempRepo;

    fn not_a_repo() -> GitInfo {
        GitInfo {
            is_repo: false,
            current_branch: None,
            current_branch_has_commits: false,
            branches: Vec::new(),
        }
    }

    #[test]
    fn a_non_repo_is_not_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_git_info(dir.path()), not_a_repo());
    }

    #[test]
    fn a_repo_reports_branch_commits_and_sorted_branches() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "x\n", "one");
        t.branch("feature");
        assert_eq!(
            load_git_info(t.path()),
            GitInfo {
                is_repo: true,
                current_branch: Some("main".to_string()),
                current_branch_has_commits: true,
                branches: vec!["feature".to_string(), "main".to_string()],
            }
        );
    }

    #[test]
    fn a_repo_without_commits_has_no_branch_and_no_branches() {
        let t = TempRepo::new();
        assert_eq!(
            load_git_info(t.path()),
            GitInfo {
                is_repo: true,
                current_branch: None,
                current_branch_has_commits: false,
                branches: Vec::new(),
            }
        );
    }

    #[test]
    fn a_repo_subdirectory_is_looked_up_to_the_repo() {
        let t = TempRepo::new();
        t.commit_file("sub/a.txt", "x\n", "one");
        let info = load_git_info(&t.path().join("sub"));
        assert!(info.is_repo);
        assert_eq!(info.current_branch.as_deref(), Some("main"));
    }

    #[test]
    fn a_file_path_is_not_a_repo() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "x\n", "one");
        assert_eq!(load_git_info(&t.path().join("a.txt")), not_a_repo());
    }
}

#[cfg(test)]
mod send_payload_tests {
    use super::send_payload;
    use std::path::Path;

    #[test]
    fn relative_path_for_file_and_dir() {
        let root = Path::new("/proj");
        let (rel, is_dir) = send_payload(root, Path::new("/proj/src/main.rs"), false);
        assert_eq!(rel, "src/main.rs");
        assert!(!is_dir);
        let (rel, is_dir) = send_payload(root, Path::new("/proj/research"), true);
        assert_eq!(rel, "research", "落库用的相对路径不带尾部 /");
        assert!(is_dir);
    }

    #[test]
    fn keeps_spaces_and_unicode_verbatim() {
        let root = Path::new("/proj");
        let (rel, is_dir) = send_payload(root, Path::new("/proj/我的 报告/draft v2.md"), false);
        assert_eq!(rel, "我的 报告/draft v2.md");
        assert!(!is_dir);
    }
}
