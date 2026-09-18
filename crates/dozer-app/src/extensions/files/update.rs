//! Files 面板 update 消息分发 + git 信息加载 spawn。

use crate::delivery;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::*;

/// 处理 `CopyPath` 之外的全部消息。内核在到达这里之前已经拦截了
/// `CopyPath`(真正的剪贴板写入需要 `main.rs` 的 `Clipboard` 句柄),它传
/// 进来会 `unreachable!`(同 Todo 试点 `DispatchToExisting`/`DispatchNew`
/// 的处理方式)。`project_id` 由内核路由时已经解析出正确值传入。
pub fn update(
    ws_state: &mut WorkspaceState,
    app_state: &mut AppState,
    msg: Message,
    project_id: i64,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::Toggle(dir) => {
            ws_state.tree_selected = Some(dir.clone());
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
            ws_state.dir_statuses = delivery::rollup_dir_statuses(&statuses);
            ws_state.git_statuses = statuses;
            // 项目 git 状态更新(git_watch 拾起 HEAD/refs 变化后)常伴随分支
            // 切换,顺手把分支栏的仓库信息一并刷新,让底栏与树保持一致。
            spawn_git_info_load(ws_state, project_id, handle, emit);
        }
        Message::RightClickAt { x, y } => {
            app_state.last_right_click = (x, y);
        }
        Message::ContextMenuOpen { path, is_dir } => {
            ws_state.tree_selected = Some(path.clone());
            // 与分支切换弹层互斥:开右键菜单时收起分支弹层,避免两个浮层
            // 同时挂着(同 `PreviewTabContextMenu` 关文件树右键菜单的约定)。
            ws_state.branch_picker_open = false;
            #[cfg(target_os = "macos")]
            {
                let (x, y) = app_state.last_right_click;
                let is_root = ws_state
                    .file_tree
                    .as_ref()
                    .map(|t| t.root() == path.as_path())
                    .unwrap_or(false);
                let has_clipboard = ws_state.tree_clipboard.is_some();
                let items =
                    context_menu_items(&path, is_dir, is_root, has_clipboard, ws_state.git_is_repo);
                if let Some(msg) = crate::chrome::native_menu::show(items, (x, y)) {
                    // 不能直接递归调用本函数(`update`)——`OpenSearch`/
                    // `CopyPath` 这两个菜单项产出的消息是"内核拦截处理"的
                    // (打开搜索弹窗要跨到 `search::Message`,写系统剪贴板
                    // 需要 main.rs 的 `Clipboard` 句柄),在 `files::update`
                    // 内部就是 `unreachable!()`,文档写明"它们永远不该落到
                    // 这里"。用 `emit` 把消息送回内核顶层(`App::update()`
                    // 的 `Message::Files(...)` 分派),和其它异步产出的
                    // `files::Message` 走的是同一条既有回路,才能命中
                    // `OpenSearch`/`CopyPath` 的顶层拦截分支。
                    emit(msg);
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                let (x, y) = app_state.last_right_click;
                app_state.context_menu = Some(ContextMenu {
                    x,
                    y,
                    target: path,
                    is_dir,
                });
            }
        }
        Message::ContextMenuClose => {
            app_state.context_menu = None;
        }
        Message::RevealInFinder(path) => {
            app_state.context_menu = None;
            let _ = std::process::Command::new("open")
                .arg("-R")
                .arg(&path)
                .spawn();
        }
        Message::Copy(path, is_dir) => {
            app_state.context_menu = None;
            ws_state.tree_clipboard = Some((path, is_dir));
        }
        Message::Paste(target_dir) => {
            app_state.context_menu = None;
            ws_state.tree_error = None;
            let Some((source, source_is_dir)) = ws_state.tree_clipboard.clone() else {
                return;
            };
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    crate::project::paste_item(&source, source_is_dir, &target_dir)
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                emit(Message::PasteDone(project_id, result));
            });
        }
        Message::PasteDone(_, result) => match result {
            Ok(new_path) => {
                if let (Some(tree), Some(parent)) = (&mut ws_state.file_tree, new_path.parent()) {
                    tree.refresh(parent);
                }
            }
            Err(e) => ws_state.tree_error = Some(e),
        },
        Message::FileHistoryRollbackDone(_project_id, abs_path, result) => match result {
            Ok(()) => {
                // 还原成功:刷新被回滚文件所在的父目录,让树里该文件的状态
                // (大小/可能的新增标记)重新读盘。
                if let (Some(tree), Some(parent)) =
                    (&mut ws_state.file_tree, abs_path.parent())
                {
                    tree.refresh(parent);
                }
            }
            Err(e) => ws_state.tree_error = Some(e),
        },
        Message::DeleteRequest(path, is_dir) => {
            app_state.context_menu = None;
            ws_state.tree_delete_confirm = Some((path, is_dir));
        }
        Message::DeleteCancel => {
            ws_state.tree_delete_confirm = None;
        }
        Message::DeleteConfirm => {
            let Some((path, _)) = ws_state.tree_delete_confirm.take() else {
                return;
            };
            ws_state.tree_error = None;
            handle.spawn(async move {
                let parent = path.parent().map(|p| p.to_path_buf());
                let result = tokio::task::spawn_blocking(move || {
                    trash::delete(&path).map_err(|e| e.to_string())
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                let outcome = match (result, parent) {
                    (Ok(()), Some(p)) => Ok(p),
                    (Ok(()), None) => Err("删除的是项目根,无父目录可刷新".to_string()),
                    (Err(e), _) => Err(e),
                };
                emit(Message::OpDone {
                    project_id,
                    parent: outcome,
                    expand: false,
                });
            });
        }
        Message::OpDone { parent, expand, .. } => match parent {
            Ok(parent) => {
                ws_state.tree_error = None;
                if let Some(tree) = &mut ws_state.file_tree {
                    tree.refresh(&parent);
                    if expand {
                        tree.ensure_expanded(&parent);
                    }
                }
            }
            Err(e) => ws_state.tree_error = Some(e),
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
            ws_state.tree_error = None;
            if let Some(tree) = &mut ws_state.file_tree {
                tree.reload_from_disk();
            }
        }
        Message::SearchInput(s) => {
            ws_state.tree_search = s;
        }
        Message::SearchSubmit => {
            ws_state.search_query = ws_state.tree_search.clone();
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
            ws_state.branch_picker_open = true;
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
                    crate::delivery::checkout_branch(&root, &name)
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
                    let result =
                        tokio::task::spawn_blocking(move || crate::delivery::init_repo(&root))
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
            ws_state.tree_error = None;
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
        Message::OpenSearch(..) => {
            unreachable!("由内核拦截处理,映射成 search::Message::SearchOpen")
        }
        Message::FileHistoryOpen(_) => {
            unreachable!("由内核拦截处理,见 files::Message::FileHistoryOpen 文档")
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
        // 外部文件被松开在某目录上:逐个移动(同 Finder 拖拽的 move 语义,
        // 跨文件系统在 `move_item` 里降级为复制+删源)。全部完成后 `emit`
        // `FileDropDone` 刷新目标目录。
        Message::FileDrop { paths, target } => {
            ws_state.tree_error = None;
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
                ws_state.tree_error = Some(format!(
                    "{} 已在项目内,不支持用拖拽移动项目内文件",
                    bad.display()
                ));
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
                    source: only.clone(),
                    source_is_dir: only.is_dir(),
                    name_draft: name,
                    dir_draft: target.display().to_string(),
                });
                ws_state.move_focus_pending = true;
                return;
            }
            // 理论上不会走到这里(见上面 `Message::FileDrop` 文档),多文件
            // 批量改名弹一个框没有意义,原地保留旧的"直接移动"兜底行为。
            let drop_target = target.clone();
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    for p in &paths {
                        let is_dir = std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false);
                        crate::project::move_item(p, is_dir, &target)?;
                    }
                    Ok::<(), String>(())
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                emit(Message::FileDropDone(project_id, drop_target, result));
            });
        }
        Message::FileDropDone(_, target, result) => match result {
            Ok(()) => {
                ws_state.tree_error = None;
                if let Some(tree) = &mut ws_state.file_tree {
                    tree.refresh(&target);
                }
            }
            Err(e) => ws_state.tree_error = Some(e),
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
        Message::MoveDirBrowse => {
            unreachable!(
                "由 main.rs Runner::dispatch 拦截处理,见 files::Message::MoveDirBrowse 文档"
            )
        }
        Message::MoveCancel => {
            ws_state.pending_move = None;
        }
        // 校验同行内编辑框既有口径(`submit_tree_edit`):名字非空、不含路径
        // 分隔符;目标目录必须真实存在。任一失败都把 `pending_move` 放
        // 回去(草稿保留用户已输入的内容,只是换上校验后的值),对话框留
        // 在屏幕上,不当成"取消"处理。
        Message::MoveConfirm => {
            let Some(pending) = ws_state.pending_move.take() else {
                return;
            };
            ws_state.tree_error = None;
            let name = pending.name_draft.trim().to_string();
            let dir_text = pending.dir_draft.trim().to_string();
            if name.is_empty() || !crate::project::is_single_path_component(&name) {
                ws_state.tree_error = Some("名字不能为空或包含路径分隔符".to_string());
                ws_state.pending_move = Some(PendingMove {
                    name_draft: name,
                    ..pending
                });
                return;
            }
            let target_dir = PathBuf::from(&dir_text);
            if !target_dir.is_dir() {
                ws_state.tree_error = Some(format!("{} 不是有效目录", target_dir.display()));
                ws_state.pending_move = Some(PendingMove {
                    dir_draft: dir_text,
                    ..pending
                });
                return;
            }
            let dest = target_dir.join(&name);
            let source = pending.source;
            let is_dir = pending.source_is_dir;
            // 路径压根没变(没改名也没改目录,原地确认)、或目录被改成要移进
            // 它自己的子树——这两种在 `move_item_to` 里都会撞上"已存在同名
            // 项"/"不能移到它自己或其子树"的校验失败,但对用户来说这不是
            // "出错了",只是"什么都没变"或"这么改没有意义"——静默当取消处理
            // (不提示错误、不动磁盘),不吓用户一跳(2026-09 用户实测反馈:
            // 这类情形不该弹错误)。
            if dest == source || (is_dir && dest.starts_with(&source)) {
                return;
            }
            let refresh_target = target_dir.clone();
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    crate::project::move_item_to(&source, is_dir, &dest)?;
                    Ok::<(), String>(())
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                emit(Message::FileDropDone(project_id, refresh_target, result));
            });
        }
        Message::TreeRowPress { .. } => {
            unreachable!("由内核拦截处理,见 files::Message::TreeRowPress 文档")
        }
        Message::TreeDragRelease => {
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
        Message::TreeDragEnd(confirmed) => {
            let Some(drag) = ws_state.tree_drag.take() else {
                return;
            };
            ws_state.drag_hover = HashSet::new();
            if !confirmed {
                ws_state.tree_selected = Some(drag.source.clone());
                return;
            }
            let TreeDragPhase::Dragging { target } = drag.phase else {
                return;
            };
            let Some(target) = target else {
                return;
            };
            let name = drag
                .source
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            ws_state.pending_move = Some(PendingMove {
                source: drag.source,
                source_is_dir: drag.source_is_dir,
                name_draft: name,
                dir_draft: target.display().to_string(),
            });
            ws_state.move_focus_pending = true;
        }
    }
}

/// 异步加载一次 git 仓库信息:判项目根是否在仓库内、读当前分支、读本地分支
/// 表,结果通过 `emit(GitInfoLoaded(..))` 回投。走 `spawn_blocking` 防止阻塞
/// UI 线程(交付层的 git 调用都是同步阻塞,见 delivery.rs 模块头注释)。
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
        let info = tokio::task::spawn_blocking(move || {
            use crate::delivery::{branch, current_branch_has_commits, local_branches, repo_root};
            match repo_root(&root) {
                Some(repo) => GitInfo {
                    is_repo: true,
                    current_branch: branch(&repo),
                    current_branch_has_commits: current_branch_has_commits(&repo),
                    branches: local_branches(&repo).unwrap_or_default(),
                },
                None => GitInfo {
                    is_repo: false,
                    current_branch: None,
                    current_branch_has_commits: false,
                    branches: Vec::new(),
                },
            }
        })
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
