//! Files 面板 view:文件树列表/行内编辑/git footer/分支选择器/右键菜单/
//! 删除与移动确认浮层。

use crate::menu_spec::{MenuSpec, MenuSpecItem};
use crate::project::PathKind;
use crate::theme::terminal_font;
use crate::{delivery, theme};
use byteui::interaction::icons;
use iced_widget::core::mouse;
use iced_widget::core::text::LineHeight;
use iced_widget::core::{Border, Color, Element, Length, Padding};
use iced_widget::{MouseArea, Scrollable, button, column, container, row, scrollable, text};
use std::path::Path;

use super::*;

/// 文件树可滚动列表(现有 `workspace.rs::project_pane` 的搬家版本,签名改吃
/// 本模块状态,去掉了不再归属本模块的项目信息卡/底部状态条)。
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    width: Length,
    outer: Border,
    search_hover_t: f32,
    dotfiles_hover_t: f32,
    branch_hover_t: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::project_pane();
    let mut header = column![].spacing(region.gap).width(Length::Fill);
    let mut tree_col = column![].spacing(region.gap);

    // 文件树搜索框:按文件/目录名称筛选整棵树(大小写不敏感子串匹配)。
    // 不会边输入边过滤——敲回车/点右侧"搜索"按钮后,由 `SearchSubmit` 把
    // 草稿落成为生效的 `search_query`。Stage 2 迁移成真正的
    // `iced_widget::text_input`:鼠标点击聚焦、方向键/选区/IME 全部走 iced
    // 标准管线自己处理,`main.rs` 只需要每帧问一遍它是否持有真实焦点
    // (`files_search_focused`)决定要不要把键盘事件放行,不再需要点击盒子
    // 手动进入自绘编辑态。样式收敛到 `byteui::form::search_box`(需求:所有
    // 面板搜索框统一成首页项目列表搜索框那一套),不再是各画一套的
    // `input_text` + 独立图标按钮;`highlight`(内部叫 `search_active`)
    // 传真实聚焦态或已生效搜索词非空,即使当前没聚焦,只要树被搜索词
    // 过滤中就持续金框提示。
    let search_active = ws_state.search_focused() || !ws_state.search_query.is_empty();
    let box_len = byteui::theme::icon_size::row() + 12.0;
    let search_box = byteui::form::search_box::view(
        "搜索目录…",
        &ws_state.tree_search,
        Some(search_field_id()),
        search_active,
        Message::SearchInput,
        Message::SearchSubmit,
        search_hover_t,
        |hovered| Message::ToolbarHover(FilesToolbarTarget::SearchSubmit, hovered),
    );
    let search_box = byteui::interaction::context_menu::wrap(
        search_box,
        Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
            id: search_field_id(),
            secure: false,
        })),
    );

    // "显示/隐藏点文件"按钮:切换后 `ToggleDotfiles` 调
    // `set_show_dotfiles` 重读树。图标反映当前口径——正显示(`eye`)时点它
    // 隐藏点文件;隐藏(`eye-off`)时点它恢复显示。切换只换图标,不套任何
    // "选中生效"的视觉信号(无 GOLD 边框/无点亮图标),保持按钮常驻常态外观。
    let dotfiles_shown = ws_state
        .file_tree
        .as_ref()
        .map(|t| t.dotfiles_shown())
        .unwrap_or(true);
    let dotfiles_button = icons::icon_button_entry(
        if dotfiles_shown {
            icons::IconKind::Eye
        } else {
            icons::IconKind::EyeOff
        },
        byteui::theme::icon_size::row(),
        false,
        false,
        dotfiles_hover_t,
        true,
        box_len,
        true,
        Message::ToggleDotfiles,
        |hovered| Message::ToolbarHover(FilesToolbarTarget::Dotfiles, hovered),
        "切换点文件",
    );
    header = header.push(
        row![search_box, dotfiles_button]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center)
            .padding([6, 0]),
    );

    // 根目录头部:只显示名称(CREAM 高亮),不再直接显示完整路径;名称前
    // 挂 folder-open-dot 图标(lucide 的展开文件夹 + 圆点,有别于普通展开目录
    // 的 folder-open,特标项目根)。与上方工具行的间距由搜索行的底部 padding
    // 承担,这里不再额外加顶边距。右键根目录打开目录右键菜单(新建文件/文件夹、
    // 复制、粘贴、删除、重命名、在 Finder 打开、从磁盘重新加载…),坐标复用
    // `main.rs` 右键时写入的 `last_right_click`。
    if let Some(tree) = &ws_state.file_tree {
        let root = tree.root();
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        // 根目录名称颜色跟着 git 状态走(与树行同款 `tree_state_color`),
        // 图标恒为灰(`DIM`),不再用 CREAM 高亮。
        let root_state = delivery::dir_status(root, &ws_state.git_statuses)
            .unwrap_or(delivery::TreeState::Unchanged);
        let root_color = tree_state_color(root_state);
        // 根目录本身也是合法的拖拽落点(项目内移动到顶层),但它不在
        // `visible_tree_rows()` 循环里(单独渲成静态头部,见上方注释),
        // 得在这里单独补上同一套"命中即高亮 + 悬停上报"逻辑,否则永远拖不
        // 到根目录(2026-09 用户实测反馈)。
        let root_is_drop_target = ws_state.drag_hover.contains(root);
        let root_header = container(
            row![
                icons::view(
                    icons::IconKind::FolderOpenDot,
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().dim
                ),
                text(name)
                    .size(byteui::theme::font::body())
                    .color(root_color),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .width(Length::Fill)
        .padding([0, 0])
        .style(move |_t: &iced_widget::Theme| container::Style {
            border: if root_is_drop_target {
                Border {
                    color: byteui::theme::color::current().gold,
                    width: 1.0,
                    radius: 6.0.into(),
                }
            } else {
                Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 0.0.into(),
                }
            },
            ..container::Style::default()
        });
        let mut root_area = MouseArea::new(root_header).on_right_press(Message::ContextMenuOpen {
            path: root.to_path_buf(),
            is_dir: true,
        });
        // 只在拖拽已确认(`Dragging`,越过距离+时长两道阈值)时才挂
        // `on_move`——`Pending` 期间必须完全没有反应,见 `TreeDragPhase`
        // 文档("点一下就进入拖拽态"的根因)。
        if ws_state.tree_drag_confirmed() {
            let root_target = root.to_path_buf();
            root_area = root_area
                .on_move(move |_| Message::TreeDragOver(root_target.clone()))
                .interaction(mouse::Interaction::Grabbing);
        }
        header = header.push(root_area);
    }

    if let Some(err) = &ws_state.tree_error {
        header = header.push(
            text(format!("⚠ {err}"))
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().red),
        );
    }
    if let Some(tree) = &ws_state.file_tree {
        // 搜索激活时走全树搜索(递归遍历含未展开深层目录),否则走当前展开
        // 的可见行。`search_rows` 只读不写缓存/展开态,view 的不变借用即可。
        let rows: Vec<crate::project::TreeRow> = if search_active {
            tree.search_rows(&ws_state.search_query)
        } else {
            tree.visible_rows()
        };
        for row in rows {
            let is_renaming = matches!(
                &ws_state.tree_edit,
                Some(TreeEdit { mode: TreeEditMode::Rename(p), .. }) if *p == row.path
            );
            if is_renaming {
                let buffer = ws_state
                    .tree_edit
                    .as_ref()
                    .map(|e| e.buffer.as_str())
                    .unwrap_or("");
                tree_col = tree_col.push(tree_edit_row(row.depth, buffer));
                continue;
            }
            let indent = "  ".repeat(row.depth);
            // git 状态编码名称颜色:未加入版本=红(最高优先),加入版本未提交
            // 的新文件=绿,修改/删除未提交=青,一般=灰,被忽略=弱灰。目录
            // 聚合取子孙中最高档(`dir_status`),让用户先注意到没加入版本
            // 管理的文件。无任何 git 记录的干净条目(状态 `None`)补成"一般"。
            let state: delivery::TreeState = if row.is_dir {
                delivery::dir_status(&row.path, &ws_state.git_statuses)
                    .unwrap_or(delivery::TreeState::Unchanged)
            } else {
                ws_state
                    .git_statuses
                    .get(&row.path)
                    .copied()
                    .map(delivery::TreeState::from)
                    .unwrap_or(delivery::TreeState::Unchanged)
            };
            let is_selected = ws_state.tree_selected.as_deref() == Some(row.path.as_path());
            // 选中行背景改半透明奶油色(见下方 `row_btn` 的 `background`,
            // alpha 0.3)——不再是实底亮底,深色 `bg` 顶替字/图标反而看不清,
            // 改用 `cream`(同 hover/active 页签既有配色),半透明底上亮字
            // 对比度足够,未选中保持原有颜色不变。
            let icon_color = if is_selected {
                byteui::theme::color::current().cream
            } else {
                byteui::theme::color::current().dim
            };
            let name_color = if is_selected {
                byteui::theme::color::current().cream
            } else {
                tree_state_color(state)
            };
            let row_icon: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> = if row
                .is_dir
            {
                let chevron = if row.expanded {
                    icons::IconKind::ChevronDown
                } else {
                    icons::IconKind::ChevronRight
                };
                let folder = if row.expanded {
                    icons::IconKind::FolderOpen
                } else {
                    icons::IconKind::Folder
                };
                // 箭头自己挂一个独立的 `MouseArea::on_press`(见
                // `Message::ToggleNoSelect` 文档):点箭头立即切换展开态、
                // 不改变选中,不走整行那套"按下武装拖拽→松开才决定单击/
                // 双击"的延迟判定。iced 事件先派发给子节点(`MouseArea::
                // update` 见其源码注释),箭头处理完会 `shell.capture_event()`,
                // 不会再冒泡触发外层整行的 `TreeRowPress`/`TreeRowDoubleClick`。
                let chevron_target = row.path.clone();
                let chevron_el: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
                    MouseArea::new(icons::view(
                        chevron,
                        byteui::theme::icon_size::chevron(),
                        icon_color,
                    ))
                    .on_press(Message::ToggleNoSelect(chevron_target))
                    .interaction(mouse::Interaction::Pointer)
                    .into();
                row![
                    chevron_el,
                    icons::view(folder, byteui::theme::icon_size::row(), icon_color),
                ]
                .spacing(byteui::theme::icon_size::tree_row_gap())
                .align_y(iced_widget::core::Alignment::Center)
                .into()
            } else {
                row![
                    iced_widget::space::Space::new()
                        .width(Length::Fixed(
                            byteui::theme::icon_size::chevron()
                                + byteui::theme::icon_size::tree_row_gap(),
                        ))
                        .height(Length::Shrink),
                    icons::view(
                        icons::icon_for_file(&row.name),
                        byteui::theme::icon_size::row(),
                        icon_color
                    ),
                ]
                .spacing(0)
                .align_y(iced_widget::core::Alignment::Center)
                .into()
            };
            let line = row![
                text(indent)
                    .size(crate::workspace::tree_row_font_size())
                    .line_height(LineHeight::Relative(terminal_font::line_height_factor()))
                    .color(name_color),
                row_icon,
                text(row.name.clone())
                    .size(crate::workspace::tree_row_font_size())
                    .line_height(LineHeight::Relative(terminal_font::line_height_factor()))
                    .color(name_color),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center);
            let msg = Message::TreeRowPress {
                path: row.path.clone(),
                is_dir: row.is_dir,
            };
            // 拖拽(外部 OS 拖入/内部树拖拽共用 `drag_hover`)悬停命中这一行
            // → 整行金色描边高亮——文件行也高亮(2026-09 用户实测反馈),
            // 即便文件本身不是真正落点(落点会退到其父目录,见
            // `files::DropHit`/`Message::TreeDragOver` 文档),这里只管"光标
            // 压中的是哪一行"的视觉反馈。
            let is_drop_target = ws_state.drag_hover.contains(&row.path);
            // 这里**不**接内层 `button` 自己的 `on_press`——iced 的
            // `iced_widget::button` 名字叫 `on_press`,实际却是在
            // `ButtonReleased`(且松开时光标仍在按钮上)才 `shell.publish`
            // (标准"点击"语义,允许按下后拖出范围取消),不是真的
            // `ButtonPressed` 就发。树内拖拽的"按下即武装"(`TreeRowPress`,
            // 见其文档)必须在物理按下那一刻就拿到消息,才能撑起后续
            // `CursorMoved` 期间的 `Pending → Dragging` 判断——若接在内层
            // button 上,武装会推迟到松手那一刻才发生,而 `main.rs` 的
            // `WindowEvent::MouseInput{Released}` 收尾检查(`TreeDragRelease`)
            // 在这次事件分发里跑在它前面,永远看到"未武装",`TreeDragEnd`
            // 因此永远不会为这次点击触发——表现为"点击没有任何反应"
            // (2026-09 用户实测反馈,带微秒级时间戳日志实锤:`TreeRowPress`
            // dispatch 的时间点几乎精确对齐松开而不是按下)。同 `rail.rs`
            // `icon_button_entry` 早就踩过的坑(见其"必须传 interactive:
            // false,不接 button::on_press"的文档)。改接到下面包裹的
            // `MouseArea::on_press` 上——那是真·`ButtonPressed` 就发。
            let row_btn: iced_widget::Button<
                '_,
                Message,
                iced_widget::Theme,
                iced_renderer::Renderer,
            > = button(line)
                .width(Length::Fill)
                .style(move |_t, _s| button::Style {
                    // 选中态背景改半透明(验收反馈:实底奶油太抢,0.3 透明度
                    // 让下面的行/缩进线隐约透出)——文字色跟着从"反色"
                    // (`bg` 深色压亮底)改回 `cream`(同 hover/active 页签的
                    // 既有配色),半透明底上深色字对比度会不够。
                    background: if is_selected {
                        Some(
                            Color {
                                a: 0.3,
                                ..byteui::theme::color::current().cream
                            }
                            .into(),
                        )
                    } else {
                        None
                    },
                    text_color: if is_selected {
                        byteui::theme::color::current().cream
                    } else {
                        byteui::theme::color::current().body
                    },
                    // 圆角恒为 6px——不只是拖拽落点描边要圆角,选中态的奶油色
                    // 实底同样要圆角(2026-09 用户实测反馈),不能只在有描边
                    // 时才圆,否则选中背景会露出方角。未选中且非落点时颜色
                    // 透明、宽度 0,圆角设了也看不出来,不需要另外分支。
                    border: Border {
                        color: if is_drop_target {
                            byteui::theme::color::current().gold
                        } else {
                            Color::TRANSPARENT
                        },
                        width: if is_drop_target { 1.0 } else { 0.0 },
                        radius: 6.0.into(),
                    },
                    ..button::Style::default()
                });
            let mut row_area = MouseArea::new(row_btn)
                .on_press(msg)
                .on_double_click(Message::TreeRowDoubleClick {
                    path: row.path.clone(),
                    is_dir: row.is_dir,
                })
                .on_right_press(Message::ContextMenuOpen {
                    path: row.path.clone(),
                    is_dir: row.is_dir,
                });
            // 树内拖拽已确认(`Dragging`,越过距离+时长两道阈值):光标划过
            // 任意行(文件或目录都上报,`Message::TreeDragOver` 里再解析
            // 落点/高亮,见其文档)即上报为悬停命中,驱动 `TreeDragOver` 校验
            // 落点合法性并刷新 `drag_hover` 高亮(同外部 OS 拖拽复用的那一圈
            // 金色描边),顺带把光标换成抓取图标。仍处于 `Pending` 时不挂
            // `on_move`——`Pending` 期间必须完全没有反应,见 `TreeDragPhase`
            // 文档("点一下就进入拖拽态"的根因)。
            if ws_state.tree_drag_confirmed() {
                let drag_target = row.path.clone();
                row_area = row_area
                    .on_move(move |_| Message::TreeDragOver(drag_target.clone()))
                    .interaction(mouse::Interaction::Grabbing);
            }
            tree_col = tree_col.push(row_area);
            let is_new_target = matches!(
                &ws_state.tree_edit,
                Some(TreeEdit {
                    mode: TreeEditMode::NewFile | TreeEditMode::NewFolder,
                    parent_dir,
                    ..
                }) if *parent_dir == row.path
            );
            if is_new_target && row.expanded {
                let buffer = ws_state
                    .tree_edit
                    .as_ref()
                    .map(|e| e.buffer.as_str())
                    .unwrap_or("");
                tree_col = tree_col.push(tree_edit_row(row.depth + 1, buffer));
            }
        }
        // 项目根目录不出现在 `visible_rows()` 里(它只渲染成上方静态头部),
        // 所以上面循环里的 `is_new_target` 永远匹配不到 root。这里单独补一段:
        // 当选中根目录作为新建父目录时,在根头部下方、按子项深度渲染编辑框,
        // 否则在根目录右键"新建文件/文件夹"会"点了菜单却没有任何输入框"。
        let root_new_edit = matches!(
            &ws_state.tree_edit,
            Some(TreeEdit {
                mode: TreeEditMode::NewFile | TreeEditMode::NewFolder,
                parent_dir,
                ..
            }) if parent_dir.as_path() == tree.root()
        );
        if root_new_edit {
            let buffer = ws_state
                .tree_edit
                .as_ref()
                .map(|e| e.buffer.as_str())
                .unwrap_or("");
            tree_col = tree_col.push(tree_edit_row(1, buffer));
        }
    }

    let body = container(
        column![
            crate::chrome::homespace::home_panel_head(icons::IconKind::FolderTree, "文件"),
            header,
            Scrollable::new(tree_col)
                .width(Length::Fill)
                .height(Length::Fill)
                .direction(scrollable::Direction::Vertical(
                    byteui::interaction::scrollbar::scrollbar()
                ))
                .on_scroll(|viewport| { Message::TreeScroll(viewport.absolute_offset().y) })
                .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style()),
            git_footer_bar(ws_state, branch_hover_t),
        ]
        .spacing(region.gap),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(region.padding)
    .style(move |_t: &iced_widget::Theme| container::Style {
        background: region.background.map(Into::into),
        border: outer,
        ..container::Style::default()
    });

    container(body).width(width).height(Length::Fill).into()
}

/// 行内编辑框(新建/重命名共用):真正的 iced `text_input`(`bare: false` 由
/// `byteui::form::input_text` 自己画卡片背景 + 聚焦金框描边)。缩进不再用
/// 等宽空格字符模拟,**改用外层容器真正的左内边距**——旧版把缩进拼进文本
/// 内容,新版用 `Padding::left` 让编辑框整体右移,视觉跟树层级绑定对齐。
fn tree_edit_row(
    depth: usize,
    buffer: &str,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 每级缩进逻辑像素:原自绘版每级用两个全角空格字符,换算成像素 =
    // `tree_row_font_size() * 0.6`(ASCII 字符宽经验值,同
    // `extensions::todo::cursor_from_x` 的换算口径)* 2(原来每级两个空格)。
    // 数字来源见 Stage 5 计划 Task 1 Step 8 的说明,不是随手拍脑袋的魔法值。
    let indent_px = depth as f32 * crate::workspace::tree_row_font_size() * 0.6 * 2.0;
    let field: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        container(byteui::form::input_text::view(
            "",
            buffer,
            false,
            Some(tree_edit_field_id()),
            false,
            Some(Message::EditSubmit),
            false,
            Message::EditInput,
        ))
        .width(Length::Fill)
        .padding(Padding {
            left: indent_px,
            ..Padding::default()
        })
        .into();
    byteui::interaction::context_menu::wrap(
        field,
        Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
            id: tree_edit_field_id(),
            secure: false,
        })),
    )
}

/// 文件树底部 git 栏三元组(图标 + 文案元素 + 可选操作按钮)。
type GitFooterTriple<'a> = (
    icons::IconKind,
    Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
);

/// 文件树底部 git 栏:项目在仓库内显示
/// `folder-git-2 当前分支名 〔切换按钮〕`;项目无 git 仓库显示
/// `folder-minus 未受Git保护 〔新建Git仓库〕`;仓库信息尚未加载显示中性
/// 占位。最左图标与文字之间、右缘切换/新建按钮始终可见;整栏无底色、
/// 顶部一条 BORDER 分隔线,与上方滚动树区隔。
fn git_footer_bar(
    ws_state: &WorkspaceState,
    branch_hover_t: f32,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let box_len = byteui::theme::icon_size::row() + 12.0;
    let (icon, label, action): GitFooterTriple<'_> = if !ws_state.git_loaded {
        (
            icons::IconKind::GitBranch,
            text("加载仓库信息…")
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().cream)
                .into(),
            None,
        )
    } else if ws_state.git_is_repo {
        // 有仓库:当前分支名(detached/无提交时 None → "无分支"),右侧切换按钮。
        // 若工作区有未提交改动,分支名以对应 git 状态色高亮(色即提示;
        // "(Uncommitted)" 文案只在展开的分支下拉菜单里对当前分支追加)。
        let root = ws_state.file_tree.as_ref().map(|t| t.root().to_path_buf());
        let dirty_state = root
            .as_deref()
            .and_then(|r| delivery::dir_status(r, &ws_state.git_statuses))
            // `dir_status` 聚合时忽略被忽略文件,`Some` 即真实未提交改动。
            .filter(|st| *st != delivery::TreeState::Ignored);
        let branch_name = ws_state
            .current_branch
            .clone()
            .unwrap_or_else(|| "无分支".to_string());
        let label_color = if let Some(st) = dirty_state {
            tree_state_color(st)
        } else {
            byteui::theme::color::current().cream
        };
        let switch = icons::icon_button_entry(
            if ws_state.branch_picker_open {
                icons::IconKind::ChevronUp
            } else {
                icons::IconKind::ChevronDown
            },
            byteui::theme::icon_size::row(),
            false,
            false,
            branch_hover_t,
            false,
            box_len,
            true,
            Message::BranchPickerOpen,
            |hovered| Message::ToolbarHover(FilesToolbarTarget::BranchSwitch, hovered),
            "切换分支",
        );
        (
            icons::IconKind::FolderGit2,
            text(branch_name)
                .size(byteui::theme::font::label())
                .color(label_color)
                .into(),
            Some(switch),
        )
    } else {
        // 无 git 仓库:提示未受 git 保护 + 新建仓库按钮。
        let init = button(
            row![
                icons::view(
                    icons::IconKind::FolderMinus,
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().cream
                ),
                text("新建Git仓库")
                    .size(byteui::theme::font::label())
                    .color(byteui::theme::color::current().cream),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .on_press(Message::GitInit)
        .padding([4, 8])
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: Some(byteui::theme::color::current().bg.into()),
            border: Border {
                color: byteui::theme::color::current().border,
                width: 1.0,
                radius: 4.0.into(),
            },
            text_color: byteui::theme::color::current().cream,
            ..button::Style::default()
        });
        (
            icons::IconKind::FolderMinus,
            text("未受Git保护")
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().cream)
                .into(),
            Some(init.into()),
        )
    };

    let bar = row![
        icons::view(
            icon,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().cream
        ),
        label,
        iced_widget::space::horizontal(),
        if let Some(btn) = action {
            btn
        } else {
            iced_widget::space::Space::new()
                .height(Length::Fixed(byteui::theme::icon_size::row() + 8.0))
                .into()
        },
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);

    let top_line = container(iced_widget::space::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..container::Style::default()
        });

    let mut content = column![top_line, bar].spacing(4);
    if let Some(err) = &ws_state.git_error {
        content = content.push(
            text(format!("⚠ {err}"))
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().red),
        );
    }

    container(content)
        .width(Length::Fill)
        .padding([6, 0])
        .style(|_t: &iced_widget::Theme| container::Style {
            background: None,
            ..container::Style::default()
        })
        .into()
}

/// 分支切换弹层（窗口级浮层）:底栏"切换按钮"按下(`branch_picker_open`)时在
/// git 底栏上方弹出全部本地分支(当前分支高亮),点某行即 `BranchSwitch(name)`
/// 切换并收起。**以 window-wide overlay 渲染**(`App::view` 的 `stack!` 里,
/// 下层垫一块透明 `MouseArea` 承接"点别处收起")——所以返回的是**占满全窗的
/// 填充容器**,靠 `Padding{bottom, left}` 把下拉框钉到 git 底栏正上方;这与
/// `context_menu_popup` 用 `Padding{top,left}` 手算像素定位是同一套约定。非
/// git/未加载/未展开时返回空(零高度元素)。
///
/// 宽度注意:与右键菜单同款——每行按钮用 `Length::Fixed(menu_item_width())`
/// 固定宽,列容器保持 `Length::Shrink`,于是整个菜单总宽恒定、不会随分支名
/// 长短自动收缩(短分支名时下拉框保持同一宽度)。不能在 Shrink 容器里给按钮
/// `Length::Fill`,否则 Fill 子在无确定宽的 Shrink 轴上会折叠成 0 宽,整个
/// 菜单就消失;`align_y(End)`(配合外层 `Padding`)负责把菜单压在 git 底栏
/// 正上方、并把下沉量交给动画起点,不会让它跑到窗口顶部。
///
/// 视觉与右键菜单(`context_menu_popup` 的 `menu_item`)对齐:同一套
/// `context_menu` 区域底色/描边/内外边距、`TAB_HOVER` hover 底。
/// 当前分支带未提交改动(dirty)时,除当前分支外的其余分支全部置灰且
/// 不可点——dirty 下切分支会被 git 拒绝(checkout 报错),提前禁用避免
/// 触发错误;同时给当前分支行追加 "(Uncommitted)" 提示。
pub fn branch_picker_popup(
    ws_state: &WorkspaceState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !ws_state.git_loaded || !ws_state.git_is_repo || !ws_state.branch_picker_open {
        return iced_widget::space::Space::new().into();
    }
    let current = ws_state.current_branch.as_deref();
    // 当前分支是否带未提交改动(dirty)?是则锁定其余分支(禁用切换)并给
    // 当前分支行追加 "(Uncommitted)"。
    let is_dirty = ws_state
        .file_tree
        .as_ref()
        .map(|t| t.root().to_path_buf())
        .as_deref()
        .and_then(|r| delivery::dir_status(r, &ws_state.git_statuses))
        .filter(|st| *st != delivery::TreeState::Ignored)
        .is_some();
    // dirty → 除当前分支外的其余分支全部置灰禁用。
    let lock_others = is_dirty;
    // 面板项/间隔统一走 `crate::chrome::menu`(样式基准即文件树右键菜单)。
    let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
        Vec::new();
    if ws_state.git_branches.is_empty() {
        items.push(
            text("暂无本地分支")
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim)
                .into(),
        );
    }
    for name in &ws_state.git_branches {
        let is_current = Some(name.as_str()) == current;
        // 当前分支 GOLD 高亮 + 指示点;其余分支:dirty 锁定时 DIM 置灰,否则
        // 常规 BODY(同上下文菜单项文字)。
        let color = if is_current {
            byteui::theme::color::current().gold
        } else if lock_others {
            byteui::theme::color::current().dim
        } else {
            byteui::theme::color::current().body
        };
        let indicator: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            if is_current {
                text("● ")
                    .size(byteui::theme::font::body())
                    .color(color)
                    .into()
            } else {
                iced_widget::space::Space::new()
                    .width(Length::Fixed(18.0))
                    .into()
            };
        let label = {
            let mut n = name.clone();
            if is_current && is_dirty {
                n.push_str("(Uncommitted)");
            }
            n
        };
        items.push(crate::chrome::menu::item_row(
            Some(indicator),
            label,
            color,
            // dirty 锁定时,非当前分支不可点(不挂 `on_press`)。
            (is_current || !lock_others).then(|| Message::BranchSwitch(name.clone())),
        ));
    }
    let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        crate::chrome::menu::shell_frosted(items, Length::Shrink);

    // 把下拉框钉到 git 底栏正上方:左缘对齐文件面板(左图标栏 + project_pane
    // 左 padding),底缘对齐 git 底栏顶部(footbar 高 + project_pane 底 padding
    // + git 底栏自身高)。外层容器铺满全窗,靠 `Padding{left,bottom}` + 子原件
    // `align_x(Start)`/`align_y(End)` 把它推到左下角(仅 `bottom` padding 而不
    // `align_y(End)` 时,Shrink 高子原件会落在内容区**顶部**,菜单就跑到窗口
    // 最上方去了——与右键菜单 `top` 定位同源,方向相反)。宽度用 `Shrink` 让
    // 菜单贴合最宽项,不会铺满窗口右缘。
    let (left, bottom) = branch_picker_popup_offset(ws_state);
    iced_widget::Container::new(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: 0.0,
            right: 0.0,
            bottom,
            left,
        })
        .align_x(iced_widget::core::Alignment::Start)
        .align_y(iced_widget::core::Alignment::End)
        .into()
}

/// `branch_picker_popup` 的基准偏移:左缘=左图标栏宽 + project_pane 左 padding;
/// 底缘=footbar 高 + project_pane 底 padding + git 底栏高。二者都吃全局 scale,
/// 随主题/缩放联动,不写死像素。
fn branch_picker_popup_offset(ws_state: &WorkspaceState) -> (f32, f32) {
    let rail = byteui::theme::geometry::icon_rail_width();
    let pane = theme::region::project_pane();
    let left = rail + pane.padding.left;
    // git 底栏高度:顶部分隔 1px + 栏内容(icon_box + 上下 padding 6) + 栏间
    // spacing 4 + 可能的 git_error 一行;project_pane gap 计入把下拉钉紧底栏。
    let git_bar_top_line = 1.0;
    let git_bar_vpad = 6.0 * 2.0;
    let bar_h = byteui::theme::icon_size::row() + 12.0;
    let error_line = if ws_state.git_error.is_some() {
        18.0
    } else {
        0.0
    };
    let git_bar_h = git_bar_top_line + bar_h + git_bar_vpad + 4.0 + error_line;
    let bottom =
        byteui::theme::geometry::footbar_height() + pane.padding.bottom + git_bar_h + pane.gap;
    (left, bottom)
}

/// `context_menu_popup` 的原生菜单版本——纯数据组装,不碰渲染/AppKit,和
/// 旧版共用完全相同的条件分支(是否目录/是否根/是否有剪贴内容),方便
/// 单测覆盖,行为上二者应保持一致。仅 macOS 编译(`native_menu` 是平台专属
/// 模块),非 mac 平台继续走 `context_menu_popup` 的 iced 弹层。数据组装
/// 收拢到 `context_menu_spec()`,这里只做 native 转换。
#[cfg(target_os = "macos")]
pub fn context_menu_items(
    target: &Path,
    is_dir: bool,
    is_root: bool,
    has_clipboard: bool,
) -> Vec<crate::chrome::native_menu::Item<Message>> {
    crate::menu_spec::to_native(context_menu_spec(target, is_dir, is_root, has_clipboard))
}

/// 文件树右键菜单内容——native(`context_menu_items`)和 iced fallback
/// (`context_menu_popup`)共用同一份数据,11 个条件分支只写一遍。
pub(crate) fn context_menu_spec(
    target: &Path,
    is_dir: bool,
    is_root: bool,
    has_clipboard: bool,
) -> MenuSpec<Message> {
    let dim = byteui::theme::color::current().dim;
    let target = target.to_path_buf();
    let mut items = vec![
        MenuSpecItem::entry(
            Some(icons::IconKind::Search),
            "搜索",
            Message::OpenSearch(target.clone(), is_dir),
        ),
        MenuSpecItem::separator(),
    ];
    if is_dir {
        items.push(MenuSpecItem::entry(
            Some(icons::IconKind::FilePlus),
            "新建文件",
            Message::NewFile(target.clone()),
        ));
        items.push(MenuSpecItem::entry(
            Some(icons::IconKind::FolderPlus),
            "新建文件夹",
            Message::NewFolder(target.clone()),
        ));
        items.push(MenuSpecItem::separator());
    }
    items.push(MenuSpecItem::entry(
        Some(icons::IconKind::Copy),
        "复制",
        Message::Copy(target.clone(), is_dir),
    ));
    if is_dir {
        items.push(MenuSpecItem::Entry {
            icon: Some(icons::IconKind::ClipboardPaste),
            icon_color: None,
            label: "粘贴".into(),
            color: if has_clipboard {
                byteui::theme::color::current().body
            } else {
                dim
            },
            enabled: has_clipboard,
            msg: Message::Paste(target.clone()),
        });
    }
    if !is_root {
        items.push(MenuSpecItem::entry(
            Some(icons::IconKind::Trash),
            "删除",
            Message::DeleteRequest(target.clone(), is_dir),
        ));
        items.push(MenuSpecItem::entry(
            Some(icons::IconKind::Rename),
            "重命名",
            Message::RenameStart(target.clone()),
        ));
    }
    items.push(MenuSpecItem::separator());
    items.push(MenuSpecItem::entry(
        None,
        "复制绝对路径",
        Message::CopyPath(target.clone(), PathKind::Absolute),
    ));
    items.push(MenuSpecItem::entry(
        None,
        "复制相对路径",
        Message::CopyPath(target.clone(), PathKind::Relative),
    ));
    items.push(MenuSpecItem::entry(
        Some(icons::IconKind::FolderOpen),
        "在 Finder 中打开",
        Message::RevealInFinder(target.clone()),
    ));
    items.push(MenuSpecItem::entry(
        Some(icons::IconKind::RefreshCw),
        "从磁盘重新加载",
        Message::ReloadFromDisk,
    ));
    items
}

/// 右键菜单浮层本体:纵向按钮列表,`container` 用 `Padding{top,left,..}`
/// 手算定位到点击坐标——`Stack` 各层共享同一份 bounds,不像原生系统菜单
/// 那样自带绝对定位,这是本仓一贯的手算像素定位风格(`ime_cursor_area`/
/// `preview_content_bounds` 同款)。
pub fn context_menu_popup<'a>(
    app_state: &'a AppState,
    ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(menu) = &app_state.context_menu else {
        return column![].into();
    };
    // 目标是否为项目根:根目录不可删除/重命名(否则会连整个项目目录一起
    // 删/改名),据此从菜单隐去对应项。
    let is_root = ws_state
        .file_tree
        .as_ref()
        .map(|t| t.root() == menu.target.as_path())
        .unwrap_or(false);
    let has_clipboard = ws_state.tree_clipboard.is_some();
    // 菜单内容组装收拢到 `context_menu_spec()`(与 native 版共用同一份
    // 条件分支),这里只做 iced 转换。注意宽度保持原值 `Length::Shrink`。
    let spec = context_menu_spec(&menu.target, menu.is_dir, is_root, has_clipboard);
    let list = crate::menu_spec::to_iced(spec, Length::Shrink);

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

/// 删除确认框:居中浮层,显示目标文件名 + 确认/取消两个按钮。
pub fn delete_confirm_popup(
    ws_state: &WorkspaceState,
    window_width: f32,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some((path, is_dir)) = &ws_state.tree_delete_confirm else {
        return column![].into();
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let kind = if *is_dir { "文件夹" } else { "文件" };
    crate::dialog::confirm(
        crate::dialog::ConfirmDialog {
            icon: None,
            title: format!("删除{kind} \"{name}\"?"),
            description: "会移入系统回收站,可从回收站找回。".to_string(),
            cancel_label: "取消".to_string(),
            cancel_msg: Message::DeleteCancel,
            confirm_label: "删除".to_string(),
            confirm_msg: Message::DeleteConfirm,
            confirm_color: byteui::theme::color::current().red,
            content_spacing: 8.0,
        },
        window_width,
    )
}

/// 拖拽移动确认框:居中浮层,视觉模板同 `delete_confirm_popup`(卡片 +
/// 取消/确认按钮)。多出"新名称"/"到目录"两个真正的 `iced_widget::
/// text_input`(复用 `byteui::form::input_text::view`),用户可在确认前
/// 改文件名/改目标目录——2026-09 用户实测反馈:拖拽移动不该悄无声息直接
/// 改路径,得让用户确认,见 `PendingMove` 文档。
pub fn move_confirm_popup(
    ws_state: &WorkspaceState,
    window_width: f32,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(pending) = &ws_state.pending_move else {
        return column![].into();
    };
    let kind = if pending.source_is_dir {
        "文件夹"
    } else {
        "文件"
    };
    let source_name = pending
        .source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| pending.source.display().to_string());

    let label = |s: &str| {
        text(s.to_string())
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().dim)
    };
    let name_field = container(byteui::form::input_text::view(
        "",
        &pending.name_draft,
        false,
        Some(move_name_field_id()),
        false,
        Some(Message::MoveConfirm),
        false,
        Message::MoveNameInput,
    ))
    .width(Length::Fixed(320.0));
    let dir_field = container(byteui::form::input_text::view(
        "",
        &pending.dir_draft,
        false,
        Some(move_dir_field_id()),
        false,
        Some(Message::MoveConfirm),
        false,
        Message::MoveDirInput,
    ))
    .width(Length::Fixed(264.0));
    let browse_btn = button(
        text("…")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream),
    )
    .on_press(Message::MoveDirBrowse)
    .padding([6, 10])
    .style(|_t, _s| button::Style {
        background: Some(byteui::theme::color::current().card.into()),
        text_color: byteui::theme::color::current().cream,
        border: Border {
            color: byteui::theme::color::current().border,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..button::Style::default()
    });

    let mut body = column![
        text(format!("移动{kind} \"{source_name}\""))
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().cream),
        column![label("新名称:"), name_field].spacing(4),
        column![
            label("到目录:"),
            row![dir_field, browse_btn]
                .spacing(6)
                .align_y(iced_widget::core::Alignment::Center)
        ]
        .spacing(4),
    ]
    .spacing(10);

    if let Some(err) = &ws_state.tree_error {
        body = body.push(
            text(err.clone())
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().red),
        );
    }

    body = body.push(crate::dialog::actions(
        row![
            button(
                text("取消")
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().dim)
            )
            .on_press(Message::MoveCancel)
            .padding([6, 12])
            .style(crate::dialog::action_button_style(
                byteui::theme::color::current().dim
            )),
            button(
                text("确定")
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().gold)
            )
            .on_press(Message::MoveConfirm)
            .padding([6, 12])
            .style(crate::dialog::action_button_style(
                byteui::theme::color::current().gold
            )),
        ]
        .spacing(8)
        .align_y(iced_widget::core::Alignment::Center),
    ));

    // 宽度改用 `dialog::width`(整窗 1/3,2026-09-15 统一约定)——此前没给
    // 显式宽度,靠内容(新名称/到目录两个输入框各自的固定宽度)撑开。
    let dialog = container(body)
        .padding(16)
        .width(crate::dialog::width(window_width))
        .style(crate::dialog::card_style);

    container(dialog)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}
