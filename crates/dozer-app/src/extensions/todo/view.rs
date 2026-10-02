//! Todo 面板 view:主视图/列表/卡片/搜索栏/footer/清空确认/状态与日历浮层。
use crate::app::{App, HoverId};

use crate::theme;
use crate::workspace::Workspace;
use byteui::interaction::icons;
use iced_widget::core::mouse;
use iced_widget::core::{Border, Color, Element, Length, Padding};
use iced_widget::{MouseArea, button, column, container, row, space, text};

use super::*;

/// Todo 面板渲染成两个独立的边框 pane(镜像 Files/Project 面板已有的
/// "侧栏 + 内容区，中间一条可拖拽分隔线"两栏模式，不再是单个面板内部一个
/// `row![sidebar, body]`)——调用方(`app.rs` 的 `PanelKind::Todo` 分支)负责
/// 拼 `row![sidebar_pane, divider_bar(Divider::TodoSplit, ..), content_pane]`。
/// 左栏：面板头 + 分类导航。右栏：列表视图主体 + 底部新增输入。
#[allow(clippy::too_many_arguments)]
pub fn view<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
    sidebar_width: Length,
    sidebar_outer: Border,
    content_width: Length,
    content_outer: Border,
) -> (
    Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
) {
    // ---- header（挂在左栏，同 Project/Files 面板"头在列表侧"的既有惯例） ----
    // 内边距对齐文件树面板(body 用 `project_pane` region 的 `padding` 把头
    // 及其自带分割线整体内缩):不再用 `[20,20]` 额外撑高头部、也不让分割线
    // 被大 padding 顶下去,与文件面板头部高度/分割线位置一致。
    let header = container(crate::chrome::homespace::home_panel_head(
        icons::IconKind::ListTodo,
        "Todo",
    ))
    .padding(theme::region::project_pane().padding);

    // ---- 左栏 pane：header + 分类导航 + 底部「清空列表」栏 ----
    // 去掉按任务状态分类的列表(全部/待办/进行中/已完成),只保留下方的
    // 自定义分类导航。原 content pane 右下角的 `todo_clear_footer_bar` 整体
    // 迁到左栏:分类导航之下、靠底。计数与「清空列表」不再堆在内容区右下方,
    // 改由左栏底部统一呈现——内容区底部只留「新增任务」输入框。
    // ---- 分类树导航:左栏唯一的多类别入口 ----
    let category_nav = category_tree_nav(app, ws_state);
    let sidebar_pane: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        container(
            column![
                header,
                category_nav,
                space::Space::new().height(Length::Fill),
                todo_clear_footer_bar(ws_state),
            ]
            .height(Length::Fill),
        )
        .width(sidebar_width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().bg.into()),
            border: sidebar_outer,
            ..container::Style::default()
        })
        .into();

    // ---- 右栏 pane：视图切换 tab + 收起/展开列表列按钮 + 视图主体 ----
    // 右栏内容区按 `TodoView` 渲染:列表视图(现有逐行列表)与看板视图
    // (一期仅占位,见 `kanban_placeholder`)。顶部一行左侧是这两个视图切换
    // tab、右端是收起/展开列表列按钮。收起/展开按钮消息为本地
    // `Message::ToggleListCollapse`,由内核 `App::update` 拦截转发成顶层
    // `Message::TogglePanelListCollapse`。
    let collapse = app.list_collapse_button(
        crate::app::PanelKind::Todo,
        app.list_collapsed(crate::app::PanelKind::Todo),
        crate::app::HoverId::TodoListCollapse,
        "收起",
        "展开",
        Message::ToggleListCollapse,
        move |hovered| Message::Hover(crate::app::HoverId::TodoListCollapse, hovered),
    );
    // 右区顶栏(原生,保留):左侧「列表视图 / 看板视图」切换 tab,右端收起/展开
    // 列表列按钮。**钉成固定高度**(`todo_top_row_inner_h_px`,tab 按钮同样钉
    // `tab_button_size()` 高),不靠内容自然撑高——几何侧
    // (`webview_geometry::todo_content_pane_bounds_for`)据 `todo_top_row_h_px`
    // 给 webview 让出这段,两侧必须同源。
    let top_row = container(
        row![
            todo_view_tab(
                "列表视图",
                TodoView::List,
                ws_state.view() == TodoView::List
            ),
            todo_view_tab(
                "看板视图",
                TodoView::Kanban,
                ws_state.view() == TodoView::Kanban
            ),
            space::Space::new().width(Length::Fill),
            collapse,
        ]
        .width(Length::Fill)
        .align_y(iced_widget::core::Alignment::Center)
        .spacing(4)
        .padding([0, 20]),
    )
    .width(Length::Fill)
    .height(Length::Fixed(theme::geometry::todo_top_row_inner_h_px()))
    .align_y(iced_widget::core::alignment::Vertical::Center);
    let body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match ws_state.view() {
            TodoView::List => todo_content_slot(app.todo_webview.failed()),
            // 看板视图一期仅占位:此时 webview 不挂载(`preview_desired` 的
            // `content_desired` 为假),原生占位可见。
            TodoView::Kanban => kanban_placeholder(),
        };
    let content_pane: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        container(column![top_row, crate::app::tab_divider(), body].height(Length::Fill))
            .width(content_width)
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(byteui::theme::color::current().bg.into()),
                border: content_outer,
                ..container::Style::default()
            })
            .into();

    (sidebar_pane, content_pane)
}

/// 右区顶部一个视图模式切换 tab(纯文字 pill):选中态 cream 文字 + `card`
/// 实底 + 1px `theme.border` 描边,未选中态静止为 `dim` 文字。
///
/// 视觉对齐文件/浏览器/终端那套共享 `tab_widget::panel_tab` 的激活态
/// (cream 文字 + `card` 底 + 1px 中性描边,非原来自成一派的金描边):因此两
/// 颗「列表视图/看板视图」切换钮与 readme 预览/浏览器打开的文件页签长相一
/// 致 —— 同一行里选中那颗 = `panel_tab` 激活页签,未选中的 = 未激活页签
/// (hover 时浮现 `tab_hover` 胶囊、文字 `dim`→`gold`)。区别只在它是无关闭
/// × 的互斥选择开关(视图切换没有"关掉列表视图"这种语义),故不复用
/// `panel_tab` 那个关不掉关闭按钮的带 close 形状,只 `button::status` 做同一套
/// 静止/hover 两态。点击下发 `Message::SelectView`,切换列表/看板视图。
pub(crate) fn todo_view_tab<'a>(
    label: &'static str,
    view: TodoView,
    active: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let theme = byteui::theme::color::current();
    // 文字颜色不固定在构造期:不显式 `.color()`(默认 `None` = 继承父级),
    // 这样才会读 `button::Style.text_color`,按下/hover 驱动 `dim`→`gold`
    // (同 `panel_tab` 的标题染色)。之前误加 `.color(Color::TRANSPARENT)`
    // ——iced `Text::color()` 一旦调用就是固定值、不是"占位待继承",导致
    // 文字恒透明不可见(2026-09-04 用户反馈肉眼看不到 tab 文字)。
    let content = text(label).size(byteui::theme::font::body());
    let btn = button(content)
        .on_press(Message::SelectView(view))
        .width(Length::Shrink)
        .height(Length::Fixed(byteui::theme::geometry::tab_button_size()))
        .padding([5, 14])
        .style(move |_t: &iced_widget::Theme, status| {
            if active {
                // 选中 → 同 `panel_tab` 激活态:CARD 实底 + 1px `theme.border`。
                button::Style {
                    background: Some(theme.card.into()),
                    text_color: theme.cream,
                    border: Border {
                        color: theme.border,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..button::Style::default()
                }
            } else {
                // 未选中 → 同 `panel_tab` 未激活态:静止透明 dim;hover/press
                // 浮现 `tab_hover` 胶囊、文字 `dim`→`gold`。
                let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: if hovered {
                        Some(theme.tab_hover.into())
                    } else {
                        None
                    },
                    text_color: if hovered { theme.gold } else { theme.dim },
                    border: Border {
                        radius: 6.0.into(),
                        ..Border::default()
                    },
                    ..button::Style::default()
                }
            }
        });
    btn.into()
}

/// 看板视图的一期占位:视觉几列状态分区我们不渲染任何任务(功能未实现),
/// 只在内容区中央给一行淡淡的提示文字,说明该视图待实现。等接入了真正
/// 的看板渲染(按状态分列的那批 `todo_card`)再替换这里。
pub(crate) fn kanban_placeholder<'a>()
-> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    container(
        text("看板视图待实现")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(iced_widget::core::alignment::Horizontal::Center)
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .into()
}

/// 右栏内容区的占位槽:列表视图下 webview 作为原生子视图叠在这块区域之上,所以
/// 这里只放一个撑满的透明占位;webview 加载失败时改显示原因与「重试」
/// (不保留旧 iced 列表作回退)。
pub(crate) fn todo_content_slot<'a>(
    failed: Option<&str>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    match failed {
        Some(reason) => container(
            column![
                text("Todo 页面加载失败")
                    .size(byteui::theme::font::body())
                    .color(tokens.body),
                text(reason.to_string())
                    .size(byteui::theme::font::caption())
                    .color(tokens.dim),
                button(text("重试").size(byteui::theme::font::label()))
                    .on_press(Message::ContentRetry),
            ]
            .spacing(10)
            .padding(16),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into(),
        None => container(space::Space::new())
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
    }
}

/// 新增任务框高度上/下限(逻辑像素)。下限即默认高(约 5 行正文,保证多行
/// 任务内容可见);上限让列表区至少留出约 140px,且 `app.rs::RowDrag` 的
/// `TodoAddGrow` 分支会再按窗口高夹一道,这里给的是硬上限(窗口极矮时由
/// 那里兜底)。
pub const ADD_INPUT_MIN_HEIGHT: f32 = 120.0;
pub const ADD_INPUT_MAX_HEIGHT: f32 = 400.0;

/// 左栏底部栏(位于分类导航之下、靠底):"清空列表"按钮撑满整行,不再展示
/// 左侧任务计数与图标。已从 content pane 右下角迁到左栏(见 `view`)。样式
/// 对齐 `project/view.rs::project_footer_bar` / `database/view.rs::
/// database_footer_bar` 的统一规范(footer 按钮撑满整行、左缘对齐面板内边距,
/// 顶部分隔线内缩对齐 `project_pane` 水平内距)。危险操作(清空整个列表
/// 不可撤销),点按钮先弹确认框(`clear_confirm_popup`)而非直接清空;
/// 按统一按钮规范(见 `dialog::action_button_border_color` 文档)走红字 +
/// 描边静止态 `border`、悬浮/按下态变 `gold`(此前固定奶油字 + 不响应
/// hover 的静态描边)。**清空本身尚未实现**:`ClearListConfirm` 在
/// `update` 里只收起弹窗,是 no-op,仅占位——弹窗流程已就位,后续接入
/// 清空逻辑时在此落地。
pub(crate) fn todo_clear_footer_bar<'a>(
    _ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 内容套一层 `width(Fill).align_x(Center)` 容器,让图标+文字在撑满整行
    // 的按钮里整体居中(与 `project/view.rs::footer_button_label` 同款手法)。
    let clear_label = container(
        row![
            icons::view(
                icons::IconKind::Trash,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().red,
            ),
            text("清空列表")
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().red),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    )
    .width(Length::Fill)
    .align_x(iced_widget::core::alignment::Horizontal::Center);

    let clear = button(clear_label)
        .on_press(Message::ClearListRequest)
        .width(Length::Fill)
        .padding([4, 8])
        .style(|_t: &iced_widget::Theme, s| button::Style {
            background: Some(byteui::theme::color::current().bg.into()),
            border: Border {
                color: byteui::feedback::dialog::action_button_border_color(s),
                width: 1.0,
                radius: 4.0.into(),
            },
            text_color: byteui::theme::color::current().red,
            ..button::Style::default()
        });

    let bar = row![clear]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center);

    let top_line = container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..container::Style::default()
        });

    // 顶部分割线左右内缩对齐面板 header 的分割线(`home_panel_head` 被
    // `project_pane().padding` 整体内缩),否则底部 footbar 分割线会比头部
    // 的更长、两端对齐不上。竖直 6px 间距沿用原 `[6,0]` 的观感。
    let pp = theme::region::project_pane().padding;
    container(column![top_line, bar].spacing(4))
        .width(Length::Fill)
        .padding(Padding {
            top: 6.0,
            right: pp.right,
            bottom: 6.0,
            left: pp.left,
        })
        .style(|_t: &iced_widget::Theme| container::Style {
            background: None,
            ..container::Style::default()
        })
        .into()
}

/// "清空列表"确认弹窗:窗口级居中浮层,视觉模板同
/// `extensions::project::project_delete_confirm_popup`/
/// `ssh.rs::delete_confirm_popup`(CARD 底 + 圆角描边 + 标题图标 +
/// 取消/确认两个圆角按钮)。标题图标用 Todo 面板自己的
/// `icons::IconKind::ListTodo`(同 `home_panel_head` 头部图标),不用
/// footbar 按钮的 `Trash`——图标标的是"这是 Todo 面板的弹窗",危险语义已
/// 由红色"清空"按钮本身表达,不需要标题图标重复。
/// "清空列表"确认弹窗的内容描述——宿主(`platform::confirm_overlay`)取这
/// 一份渲染,保证文案/消息不因迁移而分叉。
pub(crate) fn clear_confirm_spec() -> byteui::feedback::dialog::ConfirmDialog<Message> {
    byteui::feedback::dialog::ConfirmDialog {
        icon: Some(icons::IconKind::ListTodo),
        title: "清空列表".to_string(),
        description: "这会清空当前项目的全部任务,操作不可撤销。".to_string(),
        cancel_label: "取消".to_string(),
        cancel_msg: Message::ClearListCancel,
        confirm_label: "清空".to_string(),
        confirm_msg: Message::ClearListConfirm,
        close_msg: None,
        confirm_color: byteui::theme::color::current().red,
        // 原 `clear_confirm_popup` 的 `column.spacing(12)`，其它三处弹窗
        // 是 8，这里原样保留 12，不随 `confirm()` 默认值归一。
        content_spacing: 12.0,
    }
}

/// 分类树导航区:钉顶的"全部"/"未分类"伪节点 + 用户自建分类节点(可
/// 展开/收起、点选切过滤)。本函数只做展示 + 选中;右键菜单/增删改在
/// 后续任务接入。
pub(crate) fn category_tree_nav<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![].spacing(2).padding([4, 8]);

    let drag_confirmed = ws_state.category_drag_confirmed();

    // "全部"/"未分类"伪节点可右键弹出顶层"新建分类"(见
    // `CategoryContextMenuOpen(None)` 的语义)。两者都新建的是顶层分类
    // (新建后并不会真挂在哪个名字下面——全部/未分类只是视图桶)。
    // 「全部」是"移到顶层"的合法放置目标(拖拽期间挂 `on_move` + 整行金色
    // 描边);「未分类」不是,但拖到它上面时要清空悬停目标,免得上个目标的
    // 高亮残留。
    let root_is_drop_target = ws_state
        .category_drag()
        .is_some_and(|d| d.over == Some(DropTarget::Root));
    let mut all_area = MouseArea::new(category_pseudo_row(
        icons::IconKind::CircleSmall,
        "全部",
        ws_state.category_selected() == CategoryFilter::All,
        Message::CategorySelect(CategoryFilter::All),
    ));
    if drag_confirmed {
        all_area = all_area
            .on_move(|_| Message::CategoryDragOver(Some(DropTarget::Root)))
            .interaction(mouse::Interaction::Grabbing);
    }
    let all_el: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if root_is_drop_target {
            container(all_area)
                .style(|_t: &iced_widget::Theme| container::Style {
                    border: Border {
                        color: byteui::theme::color::current().gold,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..container::Style::default()
                })
                .into()
        } else {
            all_area.into()
        };
    col = col.push(byteui::interaction::context_menu::wrap(
        all_el,
        Some(Message::CategoryContextMenuOpen(None)),
    ));

    let mut uncat_area = MouseArea::new(category_pseudo_row(
        icons::IconKind::CircleSmall,
        "未分类",
        ws_state.category_selected() == CategoryFilter::Uncategorized,
        Message::CategorySelect(CategoryFilter::Uncategorized),
    ));
    if drag_confirmed {
        uncat_area = uncat_area.on_enter(Message::CategoryDragOver(None));
    }
    col = col.push(byteui::interaction::context_menu::wrap(
        uncat_area.into(),
        Some(Message::CategoryContextMenuOpen(None)),
    ));

    let rows = visible_category_rows(ws_state.categories(), ws_state.category_expanded());
    for row in rows {
        if let Some((renaming_id, draft)) = ws_state.category_renaming()
            && renaming_id == row.id
        {
            col = col.push(category_rename_row(row.depth, draft));
            continue;
        }
        let active = ws_state.category_selected() == CategoryFilter::Node(row.id);
        let fg = if active {
            byteui::theme::color::current().cream
        } else {
            byteui::theme::color::current().dim
        };
        // 拖动进行时:源行变淡,合法落点描边金色(都只在已确认 `Dragging`
        // 时才有视觉——`Pending` 期间必须完全没反应)。
        let is_source =
            ws_state.category_drag().is_some_and(|d| d.source == row.id) && drag_confirmed;
        let is_drop_target = ws_state
            .category_drag()
            .is_some_and(|d| d.over == Some(DropTarget::Node(row.id)));
        let chevron: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
            if row.has_children {
                icons::icon_button_entry(
                    if row.expanded {
                        icons::IconKind::ChevronDown
                    } else {
                        icons::IconKind::ChevronRight
                    },
                    byteui::theme::icon_size::row(),
                    active,
                    !active,
                    app.hover_progress(HoverId::TodoCategoryRow(row.id)),
                    false,
                    byteui::theme::geometry::tab_button_size(),
                    true,
                    Message::CategoryToggleExpand(row.id),
                    move |hovered| Message::Hover(HoverId::TodoCategoryRow(row.id), hovered),
                    if row.expanded { "收起" } else { "展开" },
                )
            } else {
                space::Space::new()
                    .width(byteui::theme::geometry::tab_button_size())
                    .into()
            };
        // 行内 `button` 不接 `on_press`(iced 的 `on_press` 实际在松开时才发,
        // 会把"按下即武装拖拽"错开);点击选中改由松手收尾
        // (`CategoryDragRelease` → `ReleaseAction::Select`)完成。chevron 是
        // 独立 `icon_button_entry`,按下被它捕获,不会冒泡武装拖拽。
        let label = button(
            row![
                chevron,
                text(row.name.clone())
                    .size(byteui::theme::font::body())
                    .color(fg),
            ]
            .spacing(4)
            .align_y(iced_widget::core::alignment::Vertical::Center),
        )
        .width(Length::Fill)
        .padding([6, 4 + (row.depth as u16) * 16])
        .style(move |_t: &iced_widget::Theme, _s| button::Style {
            background: if active {
                Some(byteui::theme::color::current().card.into())
            } else {
                None
            },
            text_color: if is_source {
                Color { a: 0.4, ..fg }
            } else {
                fg
            },
            border: Border {
                color: if is_drop_target || active {
                    byteui::theme::color::current().gold
                } else {
                    Color::TRANSPARENT
                },
                width: if is_drop_target || active { 1.0 } else { 0.0 },
                radius: 6.0.into(),
            },
            ..button::Style::default()
        });
        let mut area = MouseArea::new(label).on_press(Message::CategoryRowPress(row.id));
        // 已确认(`Dragging`)才挂 `on_move` 与抓取光标;`Pending` 期间必须完全
        // 没有反应("点一下就进入拖拽态"的根因,见文件树 `TreeDragPhase`)。
        if drag_confirmed {
            let target = DropTarget::Node(row.id);
            area = area
                .on_move(move |_| Message::CategoryDragOver(Some(target)))
                .interaction(mouse::Interaction::Grabbing);
        }
        col = col.push(byteui::interaction::context_menu::wrap(
            area.into(),
            Some(Message::CategoryContextMenuOpen(Some(row.id))),
        ));
    }

    // 光标移出整棵分类树(空白处/别的面板)时清空悬停目标,否则会停在最后一个
    // 目标上,松手时误落。只在拖拽确认期间挂,平时零负担。
    let tree: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> = col.into();
    if drag_confirmed {
        MouseArea::new(tree)
            .on_exit(Message::CategoryDragOver(None))
            .into()
    } else {
        tree
    }
}

/// 分类树一行的行内改名输入框,镜像 `files.rs::tree_edit_row`(同款
/// `byteui::form::input_text::view` 单行 `text_input`,`on_submit` 直接
/// 回车提交,缩进用外层 `Padding::left` 换算,不拼进文本内容)。
pub(crate) fn category_rename_row<'a>(
    depth: usize,
    draft: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let indent_px = 4.0 + depth as f32 * 16.0; // 对齐 category_tree_nav 里真实行的缩进算法
    let field = container(byteui::form::input_text::view(
        "",
        draft,
        false,
        Some(category_rename_field_id()),
        false,
        Some(Message::CategoryRenameSubmit),
        false,
        Message::CategoryRenameEdit,
    ))
    .width(Length::Fill)
    .padding(Padding {
        left: indent_px,
        ..Padding::default()
    });
    byteui::interaction::context_menu::wrap(
        field.into(),
        Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
            id: category_rename_field_id(),
            secure: false,
        })),
    )
}

/// "全部"/"未分类"两个不可删除/不可右键的伪节点行,视觉对齐真实分类
/// 节点但没有展开箭头。
pub(crate) fn category_pseudo_row<'a>(
    icon: icons::IconKind,
    label: &'a str,
    active: bool,
    on_press: Message,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let fg = if active {
        byteui::theme::color::current().cream
    } else {
        byteui::theme::color::current().dim
    };
    button(
        row![
            icons::view(
                icon,
                byteui::theme::icon_size::row(),
                if active {
                    byteui::theme::color::current().gold
                } else {
                    byteui::theme::color::current().dim
                }
            ),
            text(label).size(byteui::theme::font::body()).color(fg),
        ]
        .spacing(8)
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .on_press(on_press)
    .width(Length::Fill)
    .padding([6, 10])
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: if active {
            Some(byteui::theme::color::current().card.into())
        } else {
            None
        },
        text_color: fg,
        border: Border {
            color: if active {
                byteui::theme::color::current().gold
            } else {
                Color::TRANSPARENT
            },
            width: if active { 1.0 } else { 0.0 },
            radius: 6.0.into(),
        },
        ..button::Style::default()
    })
    .into()
}

#[cfg(test)]
mod clear_confirm_tests {
    use super::*;

    #[test]
    fn clear_confirm_spec_carries_cancel_and_confirm_messages() {
        let spec = clear_confirm_spec();
        assert_eq!(spec.title, "清空列表");
        assert_eq!(spec.confirm_label, "清空");
        assert_eq!(spec.cancel_label, "取消");
        assert!(matches!(spec.confirm_msg, Message::ClearListConfirm));
        assert!(matches!(spec.cancel_msg, Message::ClearListCancel));
        assert_eq!(spec.content_spacing, 12.0);
    }
}

/// 任务详情弹窗的卡片本体:由独立原生窗口宿主
/// `platform::todo_detail_overlay` 渲染。原生 iced 渲染(不复用会话面板的
/// webview trace——那套渲染实际内容在 `dozer://review-trace/host.html` 里,
/// 任务详情只需要看人类/agent 往来文本,不需要工具调用折叠/trace 可视化,
/// 塞进一个跟随光标定位、随时开合的原生弹窗里没有必要也不合适)。
/// 宽度撑满宿主窗口(整窗逻辑尺寸由 `todo_detail_overlay::card_logical_size`
/// 给定)。
pub fn todo_detail_card(
    ws: &Workspace,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
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
        Some(detail_reply_field_id()),
        false,
        None,
        false,
        |s| Message::DetailReplyInput(s),
    ))
    .width(Length::Fill);
    let submit_label = if ws.todo.detail_processing() {
        "处理中…"
    } else {
        "处理"
    };
    let submit = button(text(submit_label))
        .on_press_maybe((!ws.todo.detail_processing()).then_some(Message::DetailReplySubmit))
        .padding([6, 12])
        .style(byteui::feedback::dialog::action_button_style(
            byteui::theme::color::current().cream,
        ));

    let card = column![header, turns_scroll, row![reply_box, submit].spacing(8)]
        .spacing(12)
        .padding(16)
        .width(Length::Fill);
    let card = container(card).style(byteui::feedback::dialog::card_style);

    container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}
