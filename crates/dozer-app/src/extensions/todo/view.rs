//! Todo 面板 view:主视图/列表/卡片/搜索栏/footer/清空确认/状态与日历浮层。
use crate::app::{App, HoverId};

use crate::theme;
use crate::workspace::{Workspace, agent_icon};
use byteui::interaction::icons;
use dozer_core::protocol::{AgentKind, CategoryInfo, TodoInfo};
use iced_widget::core::{Border, Color, Element, Length, Padding, mouse};
use iced_widget::{
    MouseArea, button, column, container, rich_text, row, scrollable, space, span, text,
};

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
    ws: &Workspace,
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

    // ---- 状态推导（每张任务卡片的状态标/派发判断共用,一次算好） ----
    let states: Vec<TodoState> = ws_state
        .items
        .iter()
        .map(|item| {
            let target_alive = item
                .dispatch_session_id
                .as_ref()
                .map(|sid| {
                    ws.tabs
                        .iter()
                        .chain(ws.ssh_tabs.iter())
                        .any(|t| t.alive && t.info.id == *sid)
                })
                .unwrap_or(false);
            todo_display_state(item, target_alive)
        })
        .collect();

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
    // 右区顶栏:左侧放「列表视图/看板视图」两个视图切换 tab,右端放
    // 收起/展开列表列按钮。中间 `Fill` 空间把右侧按钮顶到行尾,也让视图
    // tab 左贴内容区(都与下方任务卡片取平)。`collapse` 右缘对齐 20px 右边
    // 距(2026-08-28 用户反馈:改之前 collapse 紧贴 list 右边,没跟卡片右
    // 对齐)。
    let top_row = row![
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
    .padding([4, 20]);
    let body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match ws_state.view() {
            TodoView::List => todo_list_view(app, ws_state, &states),
            // 看板视图一期仅占位:切换有入口、渲染不崩,内容留空待后续实现。
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

/// 新增任务框高度上/下限(逻辑像素)。下限即默认高(约 5 行正文,保证多行
/// 任务内容可见);上限让列表区至少留出约 140px,且 `app.rs::RowDrag` 的
/// `TodoAddGrow` 分支会再按窗口高夹一道,这里给的是硬上限(窗口极矮时由
/// 那里兜底)。
pub const ADD_INPUT_MIN_HEIGHT: f32 = 120.0;
pub const ADD_INPUT_MAX_HEIGHT: f32 = 400.0;

/// 顶部宽 8px 的细窄拖拽手柄:把光标变 `ResizingRow`,按下经
/// `Message::AddResizeStart` 交给 app 层接管高度换算。视觉上只是顶边框上
/// 一道 1px 亮线(像输入框可被向上拉起的"抓手"),平时几乎隐形,拖拽时靠
/// 光标变化提示可拖。
pub(crate) fn todo_resize_handle<'a>()
-> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let grip = container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..container::Style::default()
        });
    MouseArea::new(
        container(column![grip].spacing(0))
            .width(Length::Fill)
            .height(Length::Fixed(8.0)),
    )
    .interaction(mouse::Interaction::ResizingRow)
    .on_press(Message::AddResizeStart)
    .into()
}

/// 底部快速新建栏。结构对齐 `project.rs::project_footer_bar`(1px BORDER
/// 分隔线),但左右间距对齐任务卡片的 20px、输入框加高到约 3 行文字,**提交
/// 按钮嵌在输入框边框内**(右下方、无独立边框,只是框里一枚 circle-arrow-up
/// 图标——视觉上按钮"在输入框内",且始终贴输入框右下角)。框顶还有一道可向上拖的 8px 手柄
/// (`todo_resize_handle`),拉高输入框(高度落在 `WorkspaceState::
/// add_input_height`)。输入框已是真正的 iced `text_editor`(Stage 4 添加框
/// 迁移,与 Todo 搜索框同期),点击/光标/IME 全由原生管线接管;键盘路由靠
/// main.rs 每帧 `CaptureAddFocus` 问真实焦点态裁决。
pub(crate) fn todo_footer_bar<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let field = byteui::form::text_area::view(
        &ws_state.add_draft,
        "添加新任务",
        Some(add_field_id()),
        true,
        Some(ws_state.add_input_height()),
        Message::AddEdit,
    );
    let field = byteui::interaction::context_menu::wrap(
        field,
        Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
            id: add_field_id(),
            secure: false,
        })),
    );

    // 提交按钮:circle-arrow-up,嵌在输入框右边框内、无独立边框(视觉上"在
    // 框里"),点它提交(与搜索框按钮同款,但不认回车——`text_editor` 内置
    // Enter 换行,回车不提交新任务)。它是输入框容器内右侧的按钮,会自己
    // 吃掉点击,不会把焦点让给别处。
    //
    // 图标配色对齐其它 icon 按钮(agent 面板"＋"、文件树搜索等):静止
    // DIM、hover 平滑过渡到 GOLD,由 `HoverId::TodoAddSubmit` + 外层
    // `MouseArea` 驱动同一套悬停动画(不再是恒 GOLD 的硬编码)。
    let submit = icons::icon_button_entry(
        icons::IconKind::CircleArrowUp,
        byteui::theme::icon_size::row(),
        false,
        false,
        app.hover_progress(HoverId::TodoAddSubmit),
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::AddSubmit,
        |hovered| Message::Hover(HoverId::TodoAddSubmit, hovered),
        "提交",
    );

    // 输入框本体:单个带边框的容器,把"文字区 + 提交按钮"一起包进边框内。
    // 不再需要 `MouseArea`/`AddEditStart`——`text_editor` 是真控件,点击
    // 命中范围内就由 iced 标准鼠标管线自己处理聚焦(边框金/灰由下面
    // `editing = add_focused()` 驱动)。高度仍可经顶部拖拽手柄放大
    // (`add_input_height`,拖拽逻辑在 `app.rs::RowDrag`,本计划不改),
    // `text_area` 的 `height` 参数按这个值定高。内部一行
    // 两格:左格文字(占满高度、靠顶左对齐)、右格提交按钮(占满高度、靠底)。
    let editing = ws_state.add_focused();
    let input_box = container(
        row![
            container(field)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(iced_widget::core::alignment::Vertical::Top)
                .align_x(iced_widget::core::alignment::Horizontal::Left),
            container(submit)
                .height(Length::Fill)
                .align_y(iced_widget::core::alignment::Vertical::Bottom),
        ]
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fixed(ws_state.add_input_height()))
    .padding([10, 12])
    .style(move |_t: &iced_widget::Theme| container::Style {
        background: Some(byteui::theme::color::current().bg.into()),
        border: Border {
            color: if editing {
                byteui::theme::color::current().gold
            } else {
                byteui::theme::color::current().border
            },
            width: 1.0,
            radius: 4.0.into(),
        },
        ..container::Style::default()
    });

    // 输入框自身已带 1px 边框,作为与列表区之间的唯一分割线;不再额外画
    // 一道 `top_line`,避免输入框上方出现两条并列分割线。
    container(column![todo_resize_handle(), input_box].spacing(4))
        .width(Length::Fill)
        .padding([8, 20])
        .into()
}

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
                color: crate::dialog::action_button_border_color(s),
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
pub fn clear_confirm_popup(
    _ws_state: &WorkspaceState,
    window_width: f32,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    crate::dialog::confirm(
        crate::dialog::ConfirmDialog {
            icon: Some(icons::IconKind::ListTodo),
            title: "清空列表".to_string(),
            description: "这会清空当前项目的全部任务,操作不可撤销。".to_string(),
            cancel_label: "取消".to_string(),
            cancel_msg: Message::ClearListCancel,
            confirm_label: "清空".to_string(),
            confirm_msg: Message::ClearListConfirm,
            confirm_color: byteui::theme::color::current().red,
            // 原 `clear_confirm_popup` 的 `column.spacing(12)`，其它三处弹窗
            // 是 8，这里原样保留 12，不随 `confirm()` 默认值归一。
            content_spacing: 12.0,
        },
        window_width,
    )
}

/// 顶部搜索框:真正的 `byteui::form::input_text`,形状与 Files 搜索框
/// (Stage 2)一致。草稿 `draft` 是 `text_input::on_input` 给的全量字符串,
/// 焦点态由 `CaptureTodoSearchFocus` 每帧查、`main.rs` 据此放行键盘给
/// 标准 iced 管线。`highlight` = 列表正被 `search` 过滤或搜索聚焦时持续
/// 金框提示(同 Files `search_box`)。左前区内嵌「按状态筛选」segment
/// (`view_with_prefix`),点它开 `Message::StatusFilterOpen` 弹"全部 + 四种
/// 状态"选择浮层(不再按分类——分类改由左侧分类树承担)。
pub(crate) fn todo_search_bar<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let highlight = ws_state.search_focused() || !ws_state.search.is_empty();
    let filter_label = status_filter_label(ws_state.status_filter());
    let filter_state = match ws_state.status_filter() {
        StatusFilter::All => None,
        StatusFilter::Status(st) => Some(st),
    };
    let prefix = status_filter_segment(filter_label, filter_state);
    let bar = byteui::form::search_box::view_with_prefix(
        "搜索任务…",
        &ws_state.search_draft,
        Some(todo_search_field_id()),
        highlight,
        Message::SearchInput,
        Message::SearchSubmit,
        app.hover_progress(HoverId::TodoSearchSubmit),
        |hovered| Message::Hover(HoverId::TodoSearchSubmit, hovered),
        Some(prefix),
    );
    byteui::interaction::context_menu::wrap(
        bar,
        Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
            id: todo_search_field_id(),
            secure: false,
        })),
    )
}

/// 当前 `StatusFilter` 在搜索 segment 上显示的名字:`All` = "全部"(不做
/// 状态维度过滤),`Status(s)` = 那态的中文(fm `status_meta`,带它自己的色)。
pub(crate) fn status_filter_label(filter: StatusFilter) -> String {
    match filter {
        StatusFilter::All => "全部".to_string(),
        StatusFilter::Status(st) => status_meta(st).0.to_string(),
    }
}

/// 搜索框左前方的"状态"segment:显示当前过滤名 + 下箭头,点开窗口级浮层
/// (列出"全部 + 待办/进行中/搁置/已完成")。色调对齐分类左栏选中行(hover
/// 卡底 + 金描边);本身无边框,与外层 `search_box` 共用同一圈搜索框边框,
/// 点击不开走输入焦点——通过 `byteui::form::search_box::view_with_prefix`
/// 内嵌在输入位左前区。文案按当前过滤单项变色:选中状态时用那态的颜色,
/// "全部"用奶油,一眼能看出当前筛在哪个态。
pub(crate) fn status_filter_segment<'a>(
    label: String,
    active_state: Option<TodoState>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let text_color = active_state
        .map(|st| status_meta(st).1)
        .unwrap_or(colors.cream);
    button(
        row![
            text(label)
                .size(byteui::theme::font::body())
                .color(text_color),
            icons::view(
                icons::IconKind::ChevronDown,
                byteui::theme::icon_size::chevron(),
                colors.dim,
            ),
        ]
        .spacing(4)
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .on_press(Message::StatusFilterOpen)
    .style(move |_t: &iced_widget::Theme, s: button::Status| {
        let hovered = matches!(s, button::Status::Hovered);
        button::Style {
            background: if hovered {
                Some(colors.card.into())
            } else {
                Some(colors.bg.into())
            },
            text_color,
            border: Border {
                color: if hovered { colors.gold } else { colors.border },
                width: 1.0,
                radius: 4.0.into(),
            },
            ..button::Style::default()
        }
    })
    .into()
}

/// 列表视图主体：搜索栏 + 编号行列表 + 底部新增输入。
pub(crate) fn todo_list_view<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
    states: &[TodoState],
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let keyword_idx = filter_todos(&ws_state.items, &ws_state.search);
    let category_allowed_ids = filter_todos_by_category(
        &ws_state.items,
        ws_state.categories(),
        ws_state.category_selected(),
    );
    // 状态维度:由搜索框左前「状态」筛选项决定(从 `states` 取每条的状态)。
    // 与分类维度并列,二者连同关键词取交集(`All` 不设约束)。
    let status_allowed = match ws_state.status_filter() {
        StatusFilter::All => None,
        StatusFilter::Status(st) => Some(st),
    };
    let visible_idx: Vec<usize> = keyword_idx
        .into_iter()
        .filter(|&i| category_allowed_ids.contains(&ws_state.items[i].id))
        .filter(|&i| status_allowed.map(|st| states[i] == st).unwrap_or(true))
        .collect();

    // 搜索框的水平/垂直间距对齐任务卡片的间距规格(卡片列表 `list` 是
    // `spacing(8)` + `padding([0, 20])`):左右 20、上下 8,不再贴边顶到
    // tab 分隔线与首张卡片。
    let search = container(todo_search_bar(app, ws_state))
        .padding([8, 20])
        .width(Length::Fill);

    let mut list = column![].spacing(8).padding([0, 20]);
    if visible_idx.is_empty() {
        list = list.push(
            container(
                text("没有匹配的任务")
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().dim),
            )
            .padding([20, 20]),
        );
    } else {
        // 三段布局:活动(待办/进行中,可拖拽)→ 搁置(拿回暂停,不可拖)→
        // 已完成(沉底,不可拖)。把 `visible_idx` 按每项 `is_active_todo`/
        // paused/done 拆开,各自保持原(items)次序。与 dozerd 落盘
        // `ORDER BY done, paused, rank` 的"活动-搁置-完成"顺序同构。
        let mut active_idx: Vec<usize> = Vec::new();
        let mut suspended_idx: Vec<usize> = Vec::new();
        let mut done_idx: Vec<usize> = Vec::new();
        for &i in &visible_idx {
            let it = &ws_state.items[i];
            if it.done {
                done_idx.push(i);
            } else if it.paused {
                suspended_idx.push(i);
            } else {
                active_idx.push(i);
            }
        }
        // 拖拽进行中不再对 active 段做展示置换——之前"每帧按新顺序
        // remove+insert 整个重排"会让被拖卡片之外的其它卡片瞬间跳位,
        // 没有任何过渡帧(用户反馈"动画不够流畅"的根因)。改成更常见的
        // "源卡片原位高亮 + 插入指示线"模式:active 子序列渲染顺序全程不变,
        // 被拖的那张卡片本身描边变金(`is_drag_source`),目标位置前插一条
        // 细的金色指示线提示"松手会落在这里"。真正的换位只在 `DragEnd`
        // 落盘时一次性发生。搁置/完成不可拖也不让拖到它们上头
        // (`DragMove` 只把落点夹在 active 段内)。
        let drag_source_idx = ws_state.drag.map(|d| d.source_idx);
        // 指示线该出现在 active 子序列的哪个展示位置之前:target_idx 对应
        // 的卡片在 active_idx 里的下标。落点是 active 段尾巴(悬停到搁置
        // 标题/完成段或末尾)时,指示线插这段最后一张之后。source ==
        // target(还没真的移动过)时不显示,跟换位逻辑"没移动不写盘"对齐。
        let (insert_before, insert_at_end) = match ws_state.drag {
            Some(drag) if drag.source_idx != drag.target_idx => {
                match active_idx.iter().position(|&x| x == drag.target_idx) {
                    Some(p) => (Some(p), false),
                    None => (None, true),
                }
            }
            _ => (None, false),
        };
        let grabbing = ws_state.drag.is_some();
        let mut shown = 0usize;
        // ---- 段一:活动(待办/进行中,可拖拽) ----
        for (display_no, &idx) in active_idx.iter().enumerate() {
            shown += 1;
            if insert_before == Some(display_no) {
                list = list.push(drag_insert_indicator());
            }
            list = list.push(todo_list_row(
                app,
                ws_state,
                states,
                shown,
                idx,
                grabbing,
                drag_source_idx == Some(idx),
            ));
        }
        if insert_at_end || insert_before == Some(active_idx.len()) {
            list = list.push(drag_insert_indicator());
        }
        // ---- 段二:搁置(已暂停,拿回待重启) ----
        if !suspended_idx.is_empty() {
            list = list.push(todo_segment_divider("搁置"));
            for &idx in &suspended_idx {
                shown += 1;
                list = list.push(todo_list_row(
                    app, ws_state, states, shown, idx, grabbing, false,
                ));
            }
        }
        // ---- 段三:已完成(沉底) ----
        if !done_idx.is_empty() {
            list = list.push(todo_segment_divider("已完成"));
            for &idx in &done_idx {
                shown += 1;
                list = list.push(todo_list_row(
                    app, ws_state, states, shown, idx, grabbing, false,
                ));
            }
        }
        let _ = shown;
    }

    column![
        search,
        // 任务列表滚动条对齐全应用统一滚动条规范(几何 + 外观,见
        // `byteui::interaction::scrollbar`),不再是 iced 默认滚动条。`.id` 是新增任务后
        // "滚回顶部使新任务可见"的定位锚点(main.rs `interface.operate`
        // 拿这个 Id 发 `scrollable::scroll_to`,见 `App::take_todo_scroll_to_top`)。
        scrollable(list)
            .id(iced_widget::Id::new(TODO_LIST_SCROLL_ID))
            .height(Length::Fill)
            .direction(scrollable::Direction::Vertical(
                byteui::interaction::scrollbar::scrollbar(),
            ))
            .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style()),
        todo_footer_bar(app, ws_state),
    ]
    .height(Length::Fill)
    .into()
}

/// `todo_list_view` 单行的渲染分派:内容编辑态 → `todo_content_edit_row`,
/// 否则 → `todo_card`。从 `todo_list_view` 的循环体里拆出来,好让 pending/
/// done 两段各自的 `for` 循环别重复这段查表+分支逻辑。
#[allow(clippy::too_many_arguments)]
pub(crate) fn todo_list_row<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
    states: &[TodoState],
    number: usize,
    idx: usize,
    grabbing: bool,
    is_drag_source: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let item = &ws_state.items[idx];
    let hovered = app.hover_target(HoverId::TodoCard(idx));
    // 内容编辑态:不再把整张卡替换成独立的编辑行,而是把 `editing_draft` 传进
    // `todo_card`,由卡片原地保留边框/背景、只把内容文字换成带 BORDER 描边的
    // 输入框(见 `todo_card` 内 `label_area` 的分支)。
    let editing_draft = if let Some((editing_idx, draft)) = &ws_state.editing_content
        && *editing_idx == idx
    {
        Some(draft)
    } else {
        None
    };
    todo_card(TodoCardArgs {
        number,
        idx,
        item,
        state: states[idx],
        selected: ws_state.selected_row == Some(idx),
        grabbing,
        is_drag_source,
        hovered,
        editing_draft,
        categories: ws_state.categories(),
    })
}

/// 拖拽换位的"插入指示线":一条细的金色横条,插在"松手会落到这里"的
/// 展示位置——取代之前逐帧重排其它卡片的做法(见 `todo_list_view`)。
/// 高度和左右 padding 跟卡片间距(`spacing(8)`)对齐,视觉上像卡片之间
/// 多出的一道缝被点亮,而不是新插了一整行。
pub(crate) fn drag_insert_indicator()
-> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    container(iced_widget::space::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(3.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().gold.into()),
            border: Border {
                radius: 2.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// 段标题(搁置 / 已完成):一段窄的暗色分隔条 + 缩进 caption 文字,用来
/// 把三段列表(活动 / 搁置 / 完成)在视觉上明确切开——前两段纯靠卡片排布
/// 分不出来,加个低频次、高信息量的分隔标题最省事。本身不响应任何输入。
pub(crate) fn todo_segment_divider(
    label: impl Into<String>,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    row![
        text(label.into())
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().dim),
        container(iced_widget::space::Space::new())
            .width(Length::Fill)
            .height(Length::Fixed(1.0))
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(byteui::theme::color::current().border.into()),
                ..container::Style::default()
            }),
    ]
    .spacing(8)
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .padding([10, 0])
    .into()
}

/// 统一卡片组件：列表视图使用的边框卡片视觉，取代原来的
/// `todo_row`(扁平高亮行)。结构自上而下：编号 + 状态(`#002 - 待办` 形式,
/// 状态紧跟序号)+ 日期徽章(calendar 图标 → 日历选择器)→ checkbox + 任务文字
/// (点文字进入内容编辑)→ 指派文本按钮(仅待办未派发时)。选中/一般/hover 三态
/// 走统一卡片样式(选中=金边、hover=金边+填充、一般态=描边)。
/// `todo_card` 的参数对象:11 个位置参数里 `selected`/`grabbing`/
/// `is_drag_source`/`hovered` 四个连续 `bool`,顺序传错编译器发现不了
/// (Rust Design Patterns:Builder,用具名字段替代同类型位置参数)。
pub(crate) struct TodoCardArgs<'a> {
    number: usize,
    idx: usize,
    item: &'a TodoInfo,
    state: TodoState,
    selected: bool,
    grabbing: bool,
    is_drag_source: bool,
    hovered: bool,
    editing_draft: Option<&'a iced_widget::text_editor::Content>,
    categories: &'a [CategoryInfo],
}

pub(crate) fn todo_card<'a>(
    args: TodoCardArgs<'a>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let TodoCardArgs {
        number,
        idx,
        item,
        state,
        selected,
        grabbing,
        is_drag_source,
        hovered,
        editing_draft,
        categories,
    } = args;
    let done = item.done;

    // ---- 顶部行：编号 + 日期徽章(calendar 图标 → 日历选择器)+ 状态文字 ----
    let number_text = text(format!("#{number:03}"))
        .size(byteui::theme::font::caption())
        .color(byteui::theme::color::current().dim);

    let date_label = match state {
        TodoState::Done => item
            .completed_at_ms
            .map(format_todo_month_day)
            .unwrap_or_else(|| "-".to_string()),
        _ => item.plan_date.clone().unwrap_or_else(|| "-".to_string()),
    };
    let date_badge: Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> =
        MouseArea::new(
            row![
                icons::view(
                    icons::IconKind::Calendar,
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().dim
                ),
                text(date_label)
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().dim),
            ]
            .spacing(4)
            .align_y(iced_widget::core::alignment::Vertical::Center),
        )
        .interaction(mouse::Interaction::Pointer)
        .on_press(Message::CalendarOpen(idx))
        .into();

    // 状态不再作为顶部静态文字跟在序号后面(`#002 - 待办` 形式废除)——四态
    // 移到卡片底部左下的状态按钮(`todo_status_button`)以"当前状态"作button
    // 文本,点开状态下拉四选。顶部这行只留序号,分类 chip 与日期徽章续排。

    // 分类 chip:显示任务所属分类(找不到就是"未分类"),点击打开分类选择器。
    let category_label = categories
        .iter()
        .find(|c| Some(c.id) == item.category_id)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "未分类".to_string());
    let chip = button(
        text(category_label)
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().dim),
    )
    .on_press(Message::CategoryPickerOpenForTodo(item.id))
    .padding([2, 8])
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: Some(byteui::theme::color::current().card.into()),
        border: Border {
            color: byteui::theme::color::current().border,
            width: 1.0,
            radius: 10.0.into(),
        },
        ..button::Style::default()
    });

    let top_row = row![
        number_text,
        chip,
        iced_widget::space::Space::new()
            .width(Length::Fill)
            .height(Length::Shrink),
        date_badge,
    ]
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .spacing(8);

    // ---- 中部：checkbox + 任务文字（勾选/删除线处理与原 todo_row 一致）----
    let box_color = if done {
        byteui::theme::color::current().border
    } else {
        byteui::theme::color::current().dim
    };
    let checkbox = button(
        container(if done {
            text("✓")
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().dim)
                .into()
        } else {
            Element::from(iced_widget::space::Space::new())
        })
        .width(Length::Fixed(18.0))
        .height(Length::Fixed(18.0))
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: if done {
                Some(byteui::theme::color::current().border.into())
            } else {
                None
            },
            border: Border {
                color: box_color,
                width: 1.5,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        }),
    )
    .on_press(Message::Toggle(idx))
    .padding(0)
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: byteui::theme::color::current().cream,
        ..button::Style::default()
    });

    // 任务内容文字:未完成用主题 `body`(#9AB4C4,与正文层级一致,不再用
    // 奶油色高亮整句),已完成保持 `dim` + 删除线。进入内容编辑态时文字
    // 仍走奶油色(见 `label_area` 编辑分支),这里只负责非编辑态静态内容。
    let label_color = if done {
        byteui::theme::color::current().dim
    } else {
        byteui::theme::color::current().body
    };
    let label: Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> = if done {
        let rich: iced_widget::text::Rich<
            '_,
            (),
            Message,
            iced_widget::Theme,
            iced_renderer::Renderer,
        > = rich_text![
            span(item.text.clone())
                .size(byteui::theme::font::body())
                .color(label_color)
                .strikethrough(true)
        ];
        rich.into()
    } else {
        text(item.text.clone())
            .size(byteui::theme::font::body())
            .color(label_color)
            .into()
    };
    // 点任务文字 → 进入内容行内编辑态(取代原来的"选中"——选中/拖拽仍由卡片
    // 外层的 `RowSelect` 承担,点文字只负责编辑)。编辑态下只把内容文字原地换成
    // 自绘输入框:卡片边框/背景/勾选/日期/指派全部保持原样,输入框尺寸对齐原
    // 内容(同字号 body + 同宽 Fill),仅加 #1c3440(=BORDER)描边、不另设背景
    // (透出卡片底),避免整卡被替换成另一个带金边的大框。
    let label_area: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if let Some(draft) = editing_draft {
            // 真正的 iced `text_editor`(`byteui::form::text_area`,`bare: true`
            // 不画自身背景/描边,把外框交回下面这个外层 `container` 复刻旧版
            // "只有描边、不透底"的观感)。`height: None` 走 iced 的
            // `Length::Shrink`,随内容自然撑高——多行任务文字编辑时不再被
            // 压成单行(见 `byteui::form::text_area` 头部注释)。`content_field_id`
            // 从旧版 `container` 挪到真 `text_editor` 上,`CaptureContentEditFocus`
            // 的 `focusable` 钩子才能认出它。
            container(byteui::interaction::context_menu::wrap(
                byteui::form::text_area::view(
                    draft,
                    "任务内容…",
                    Some(content_field_id()),
                    true,
                    None,
                    Message::ContentEdit,
                ),
                Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
                    id: content_field_id(),
                    secure: false,
                })),
            ))
            .width(Length::Fill)
            .padding([10, 12])
            .style(|_t: &iced_widget::Theme| container::Style {
                background: None,
                border: Border {
                    color: byteui::theme::color::current().border,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..container::Style::default()
            })
            .into()
        } else {
            MouseArea::new(container(label).width(Length::Fill))
                .interaction(mouse::Interaction::Pointer)
                .on_press(Message::ContentEditStart(idx))
                .into()
        };

    let body_row = row![checkbox, label_area]
        .spacing(10)
        .align_y(iced_widget::core::alignment::Vertical::Center);

    // ---- 底部行：左=四态状态按钮(所有卡片都有),右=指派按钮(仅
    // "待办且未派发"会出现——指派本身就是把某条待办推进到"进行中")----
    let status_btn = todo_status_button(state, idx);

    let mut bottom = row![status_btn].spacing(12);
    // 详情按钮:只要这条任务已指派过/已留过会话(有可回看的往来、可继续
    // 人工触发处理)就出现,点击打开任务详情弹窗(`Message::DetailOpen`)。
    if item.assigned_agent.is_some() || item.dispatch_session_id.is_some() {
        let detail_btn = button(
            text("详情")
                .size(byteui::theme::font::caption())
                .color(byteui::theme::color::current().gold),
        )
        .on_press(Message::DetailOpen(idx))
        .padding([6, 8])
        .style(move |_t: &iced_widget::Theme, s: button::Status| {
            let hovered = matches!(s, button::Status::Hovered);
            button::Style {
                background: if hovered {
                    Some(byteui::theme::color::current().card.into())
                } else {
                    None
                },
                border: Border {
                    color: if hovered {
                        byteui::theme::color::current().gold
                    } else {
                        byteui::theme::color::current().border
                    },
                    width: 1.0,
                    radius: 4.0.into(),
                },
                text_color: byteui::theme::color::current().gold,
                ..button::Style::default()
            }
        });
        bottom = bottom.push(detail_btn);
    }
    if state == TodoState::Pending && item.dispatch_session_id.is_none() {
        // 样式对齐 `project.rs::project_footer_bar` 的「修复项目」按钮:
        // BG 底 + 1px BORDER 描边 + 圆角 4 + CREAM 文字,label 字号 + 内边距
        // [6,8]。文字后跟 ChevronRight 图标,提示点击会弹出可指派的 agent
        // 列表(窗口级 overlay)。
        let assign_btn = button(
            row![
                text("指派")
                    .size(byteui::theme::font::label())
                    .color(byteui::theme::color::current().cream),
                icons::view(
                    icons::IconKind::ChevronRight,
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().cream,
                ),
            ]
            .spacing(4)
            .align_y(iced_widget::core::alignment::Vertical::Center),
        )
        .on_press(Message::DispatchOpen(idx))
        .padding([6, 8])
        .style(move |_t: &iced_widget::Theme, s: button::Status| {
            let hovered = matches!(s, button::Status::Hovered);
            button::Style {
                background: if hovered {
                    Some(byteui::theme::color::current().tab_hover.into())
                } else {
                    Some(byteui::theme::color::current().bg.into())
                },
                border: Border {
                    color: if hovered {
                        byteui::theme::color::current().gold
                    } else {
                        byteui::theme::color::current().border
                    },
                    width: 1.0,
                    radius: 4.0.into(),
                },
                text_color: byteui::theme::color::current().cream,
                ..button::Style::default()
            }
        });
        bottom = bottom.push(assign_btn);
    }

    let bottom_row = row![
        bottom,
        iced_widget::space::Space::new()
            .width(Length::Fill)
            .height(Length::Shrink),
    ];

    let card_body = column![top_row, body_row, bottom_row].spacing(8);

    // 统一卡片样式:选中=金边(无背景)、hover=金边+填充、一般态=描边(无
    // 背景)——与 Agent 卡片三态对齐,不再用左侧 3px 金竖条表示选中。
    let inner = container(card_body).padding(10).width(Length::Fill);

    let card = container(inner)
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| {
            // 正在被拖起的那张卡片描边变金、加粗——跟"插入指示线"配合给
            // 出"这张卡片被拿起来了/会落在指示线那里"的反馈,不再靠其它
            // 卡片瞬间跳位来表达换位(见 `todo_list_view` 的改版说明)。
            if is_drag_source {
                container::Style {
                    background: Some(byteui::theme::color::current().card.into()),
                    border: Border {
                        color: byteui::theme::color::current().gold,
                        width: 1.5,
                        radius: byteui::interaction::cards::CARD_RADIUS.into(),
                    },
                    ..container::Style::default()
                }
            } else {
                byteui::interaction::cards::container_card(
                    selected,
                    hovered,
                    byteui::theme::color::current().card,
                )
            }
        });

    let stacked = column![card];
    // 拖拽换位感应层:补 `on_move`(光标移动过本卡就发 `DragMove`)+
    // `on_press`(`RowSelect` 选中并武装拖拽——点文字/勾选/日期/指派 这些
    // 子元素各自吞掉自己的"按下",只有落在卡片空白处才走到这里)。拖拽中
    // 整张卡显示抓取光标。
    let area = MouseArea::new(stacked)
        .on_move(move |_| Message::DragMove(idx))
        .on_enter(Message::Hover(HoverId::TodoCard(idx), true))
        .on_exit(Message::Hover(HoverId::TodoCard(idx), false))
        .on_press(Message::RowSelect(if selected { None } else { Some(idx) }));
    if grabbing {
        area.interaction(mouse::Interaction::Grabbing).into()
    } else {
        area.into()
    }
}

/// `todo_dispatch_overlay` 的原生菜单版本,纯数据组装——只列出有 headless
/// 适配器的四种 agent。仅 macOS 编译。
#[cfg(target_os = "macos")]
pub(crate) fn dispatch_items(idx: usize) -> Vec<crate::chrome::native_menu::Item<Message>> {
    [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::V8agent,
    ]
    .into_iter()
    .map(|agent| {
        crate::chrome::native_menu::Item::entry(
            Some(agent_icon(agent)),
            agent.label(),
            Message::AssignAgent(idx, agent),
        )
    })
    .collect()
}

/// `todo_status_overlay` 的原生菜单版本,纯数据组装——四态,文字用各自
/// `status_meta` 色。仅 macOS 编译。
#[cfg(target_os = "macos")]
pub(crate) fn status_items(idx: usize) -> Vec<crate::chrome::native_menu::Item<Message>> {
    use crate::chrome::native_menu::Item;
    [
        TodoState::Pending,
        TodoState::InProgress,
        TodoState::Suspended,
        TodoState::Done,
    ]
    .into_iter()
    .map(|st| {
        let (label, color) = status_meta(st);
        Item::Entry {
            icon: None,
            icon_color: None,
            label: label.into(),
            color,
            enabled: true,
            msg: Message::StatusPick(idx, st),
        }
    })
    .collect()
}

/// Todo 指派选择层(窗口级 overlay 版):选一个 agent 种类完成指派,不再
/// 要求"存在活着的 tab"(2026-09-02 起,指派与执行解耦——指派只是记录,
/// 真正执行靠分类轮询开关或详情弹窗手动"处理")。样式沿用
/// `crate::chrome::menu::item_row_fill` + `menu::shell`,定位靠 `dispatch_anchor`。
pub fn todo_dispatch_overlay<'a>(
    ws: &Workspace,
    window_size: (f32, f32),
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let ws_state = &ws.todo;
    let idx = ws_state.dispatch_open?;
    let anchor = ws_state.dispatch_anchor?;

    // 只列出有 headless 适配器的四种(与 `AgentKind::label()` 的四个可指派
    // 值一致,`dozerd::headless_agent::bare_program_name` 同一份覆盖面)。
    let candidates = [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::V8agent,
    ];
    let items: Vec<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> = candidates
        .into_iter()
        .map(|agent| {
            let icon = icons::view(
                agent_icon(agent),
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().body,
            );
            crate::chrome::menu::item_row(
                Some(icon),
                agent.label().to_string(),
                byteui::theme::color::current().body,
                Some(Message::AssignAgent(idx, agent)),
            )
        })
        .collect();
    let popup = crate::chrome::menu::shell_frosted(items, Length::Shrink);

    // 全窗口容器 + padding 把弹层推到锚点;窗口边界钳制,避免超出右下。
    let (ax, ay) = anchor;
    let window_w = window_size.0;
    let window_h = window_size.1;
    // 菜单估算尺寸:常宽约 220(图标 + 文字 + 内边距)、四条候选高约 4*28 + 内边距。
    let pop_w = 224.0_f32;
    let pop_h = 160.0_f32;
    let x = ax.min((window_w - pop_w).max(0.0));
    let y = ay.min((window_h - pop_h).max(0.0));
    Some(
        container(popup)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: y,
                left: x,
                right: 0.0,
                bottom: 0.0,
            })
            .into(),
    )
}

/// 任务状态文案与专属色,四态一个不落:
/// 待办=青 `CYAN`、进行中=金 `GOLD`、搁置=奶油 `CREAM`(比 DIM 略亮一点、
/// 强调"还没完,只是被拿回来放着")、完成=灰 `DIM`。状态按钮文本与状态下拉
/// 菜单各条目统一从这里取色,不各写一份颜色表。
pub(crate) fn status_meta(state: TodoState) -> (&'static str, Color) {
    match state {
        TodoState::Pending => ("待办", byteui::theme::color::current().cyan),
        TodoState::InProgress => ("进行中", byteui::theme::color::current().gold),
        TodoState::Suspended => ("搁置", byteui::theme::color::current().cream),
        TodoState::Done => ("已完成", byteui::theme::color::current().dim),
    }
}

/// 卡片底部左下的「状态」下拉按钮:文本 = 当前状态(待办/进行中/搁置/已完成,
/// 颜色随 `status_meta`),后跟向下箭头提示展开。点开弹 `todo_status_overlay`
/// 的四个选项。整体观感对齐同一行的「指派」按钮(BG 底 + BORDER 描边 +
/// 圆角 4),避免一张卡里两种按钮风格打架。
pub(crate) fn todo_status_button(
    state: TodoState,
    idx: usize,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (label, color) = status_meta(state);
    button(
        row![
            text(label).size(byteui::theme::font::label()).color(color),
            icons::view(
                icons::IconKind::ChevronDown,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().dim,
            ),
        ]
        .spacing(4)
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .on_press(Message::StatusOpen(idx))
    .padding([6, 8])
    .style(move |_t: &iced_widget::Theme, s: button::Status| {
        let hovered = matches!(s, button::Status::Hovered);
        button::Style {
            background: if hovered {
                Some(byteui::theme::color::current().card.into())
            } else {
                Some(byteui::theme::color::current().bg.into())
            },
            border: Border {
                color: if hovered {
                    byteui::theme::color::current().gold
                } else {
                    byteui::theme::color::current().border
                },
                width: 1.0,
                radius: 4.0.into(),
            },
            text_color: color,
            ..button::Style::default()
        }
    })
    .into()
}

/// Todo 状态下拉选择层(窗口级 overlay 版,设计风格复用 `todo_dispatch_overlay`
/// ——都是"点一个卡片按钮弹出的四选/选项列表",用同一套 `menu::shell` 壳保证
/// 观感一致):从上到下依次列出 待办/进行中/搁置/已完成 四态,每个条目文字用
/// 该态自己的 `status_meta` 色。点某条发 `StatusPick(idx, 该态)`(真实的存储
/// 落盘 / 转派发选择层的分支都在 `Message::StatusPick` 里处理)。返回 `None`
/// 表示 `status_open` 为真但锚点缺失(理论上到不了,调用方降级为只铺 dismiss
/// 收起层)。
pub fn todo_status_overlay<'a>(
    ws: &Workspace,
    window_size: (f32, f32),
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let ws_state = &ws.todo;
    let idx = ws_state.status_open?;
    let anchor = ws_state.status_anchor?;

    let candidates = [
        TodoState::Pending,
        TodoState::InProgress,
        TodoState::Suspended,
        TodoState::Done,
    ];
    let items: Vec<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> = candidates
        .into_iter()
        .map(|st| {
            let (label, color) = status_meta(st);
            crate::chrome::menu::item_row(None, label, color, Some(Message::StatusPick(idx, st)))
        })
        .collect();
    let popup = crate::chrome::menu::shell_frosted(items, Length::Shrink);

    // 全窗口容器 + padding 把弹层推到锚点;窗口边界钳制,避免超出右下。
    let (ax, ay) = anchor;
    let window_w = window_size.0;
    let window_h = window_size.1;
    // 菜单估算尺寸:常宽约 160(文字 + 内边距,比派发列表窄,因为没有图标列),
    // 高约每项 28 + 壳内边距;给足余量。
    let pop_w = 160.0_f32;
    let pop_h = (4.0_f32 * 28.0) + 16.0;
    let x = ax.min((window_w - pop_w).max(0.0));
    let y = ay.min((window_h - pop_h).max(0.0));
    Some(
        container(popup)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: y,
                left: x,
                right: 0.0,
                bottom: 0.0,
            })
            .into(),
    )
}

/// 搜索框左前「按状态筛选」的选择浮层(窗口级 overlay):从上到下列出
/// 「全部」 + 待办 / 进行中 / 搁置 / 已完成。"全部"不设任何状态约束(奶油
/// 文案);其余四种用各自 `status_meta` 专属色,并在前面缀一个当前正选中的
/// 状态用 **✓** 单字做选中标记,方便一眼看到现在筛在哪个态。点某条发
/// `Message::StatusFilterPick(filter)`(纯本地改 `status_filter`,不动关键词、
/// 不落盘),随后由该消息关闭浮层。`app.rs` 在状态详情浮层(卡片状态按钮)
/// 与日历/派发层之外单独判断,与它们互斥不得同时弹出。
///
/// 返回 `None`:要么浮层根本没打开,要么锚点还没记上(理论上到不了,调用方
/// 降级为只铺 dismiss 收起层)。
pub fn todo_status_filter_overlay<'a>(
    ws: &Workspace,
    window_size: (f32, f32),
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let ws_state = &ws.todo;
    if !ws_state.status_filter_open {
        return None;
    }
    let anchor = ws_state.status_filter_anchor?;
    // 记下当前正筛中的状态(“全部”时 `None”),用于在浮层里给对应项打 ✓。
    let checked_state = match ws_state.status_filter() {
        StatusFilter::All => None,
        StatusFilter::Status(st) => Some(st),
    };

    let mut items: Vec<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> =
        Vec::new();
    let all_checked = checked_state.is_none();
    let marker = if all_checked { "✓ " } else { "  " };
    let cream = byteui::theme::color::current().cream;
    let all_btn = button(
        text(format!("{marker}全部"))
            .size(byteui::theme::font::body())
            .color(cream),
    )
    .on_press(Message::StatusFilterPick(StatusFilter::All))
    .width(Length::Fill)
    .padding([6, 10])
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: cream,
        ..button::Style::default()
    });
    items.push(all_btn.into());

    // 四种状态——带当前选中的先导记号(✓),其余补两个空格以对齐列宽。
    let order = [
        TodoState::Pending,
        TodoState::InProgress,
        TodoState::Suspended,
        TodoState::Done,
    ];
    for st in order {
        let (lbl, c) = status_meta(st);
        let checked = checked_state == Some(st);
        let lead = if checked { "✓ " } else { "  " };
        let row = button(
            text(format!("{lead}{lbl}"))
                .size(byteui::theme::font::body())
                .color(c),
        )
        .on_press(Message::StatusFilterPick(StatusFilter::Status(st)))
        .width(Length::Fill)
        .padding([6, 10])
        .style(move |_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: c,
            ..button::Style::default()
        });
        items.push(row.into());
    }

    let popup = container(column(items).spacing(2))
        .width(Length::Fixed(200.0))
        .padding(4)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().card.into()),
            border: Border {
                color: byteui::theme::color::current().border,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        });

    let (ax, ay) = anchor;
    let pop_w = 200.0_f32;
    let pop_h = 5.0_f32 * 34.0 + 10.0;
    let x = ax.min((window_size.0 - pop_w).max(0.0));
    let y = ay.min((window_size.1 - pop_h).max(0.0));
    Some(
        container(popup)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: y,
                left: x,
                right: 0.0,
                bottom: 0.0,
            })
            .into(),
    )
}

/// 日历日期选择器：点卡片日期徽章弹出,展示 `calendar_view` 那个月,上一月/
/// 下一月导航,点某天把 `plan_date` 写成 "MM-DD" 并关闭。样式对齐
/// `todo_dispatch_overlay`(CARD 底 + BORDER 描边)。
pub(crate) fn todo_calendar_popup(
    idx: usize,
    view: (i32, u32),
    selected: Option<String>,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (y, m) = view;
    let first_wd = first_weekday_of_month(y, m);
    let dim = days_in_month(y, m);
    // 没有 plan_date 时默认高亮"今天"(仅当当前视图月就是当前月);有
    // plan_date 则高亮那天的 MM-DD。满足"弹出时默认选中当天日期"。
    let selected_md = selected.as_deref().and_then(parse_month_day).or_else(|| {
        let (ty, tm, td) = today_ymd();
        if tm == m && ty == y {
            Some((tm, td))
        } else {
            None
        }
    });

    let nav_style = |_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: byteui::theme::color::current().cream,
        ..button::Style::default()
    };
    let prev = button(
        text("‹")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream),
    )
    .on_press(Message::CalendarPrevMonth)
    .padding([2, 8])
    .style(nav_style);
    let next = button(
        text("›")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream),
    )
    .on_press(Message::CalendarNextMonth)
    .padding([2, 8])
    .style(nav_style);
    let title = text(format!("{y}-{m:02}"))
        .size(byteui::theme::font::caption())
        .color(byteui::theme::color::current().cream);
    let header = row![prev, title, next]
        .spacing(6)
        .align_y(iced_widget::core::alignment::Vertical::Center);

    let weekday_labels = ["日", "一", "二", "三", "四", "五", "六"];
    let mut weekday_row = row![].spacing(0);
    for w in weekday_labels {
        weekday_row = weekday_row.push(
            container(
                text(w)
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().dim),
            )
            .width(Length::Fixed(28.0))
            .center_x(Length::Fill),
        );
    }

    let mut rows = column![].spacing(2);
    let mut day: u32 = 1;
    'outer: for r in 0..6u32 {
        let mut row_el = row![].spacing(0);
        for c in 0..7u32 {
            let cell = r * 7 + c;
            if cell < first_wd || day > dim {
                row_el = row_el.push(
                    container(iced_widget::space::Space::new())
                        .width(Length::Fixed(28.0))
                        .height(Length::Fixed(24.0)),
                );
            } else {
                let d = day;
                let is_sel = selected_md == Some((m, d));
                let cell_btn = button(
                    text(format!("{d}"))
                        .size(byteui::theme::font::caption())
                        .color(if is_sel {
                            byteui::theme::color::current().gold
                        } else {
                            byteui::theme::color::current().cream
                        }),
                )
                .on_press(Message::CalendarPick(idx, format!("{m:02}-{d:02}")))
                .width(Length::Fixed(28.0))
                .height(Length::Fixed(24.0))
                .padding(0)
                .style(move |_t: &iced_widget::Theme, _s| button::Style {
                    background: if is_sel {
                        Some(byteui::theme::color::current().card.into())
                    } else {
                        None
                    },
                    border: Border {
                        color: if is_sel {
                            byteui::theme::color::current().gold
                        } else {
                            Color::TRANSPARENT
                        },
                        width: if is_sel { 1.0 } else { 0.0 },
                        radius: 4.0.into(),
                    },
                    text_color: if is_sel {
                        byteui::theme::color::current().gold
                    } else {
                        byteui::theme::color::current().cream
                    },
                    ..button::Style::default()
                });
                row_el = row_el.push(cell_btn);
                day += 1;
                if day > dim {
                    rows = rows.push(row_el);
                    break 'outer;
                }
            }
        }
        rows = rows.push(row_el);
    }

    container(column![header, weekday_row, rows].spacing(4))
        .width(Length::Shrink)
        .padding(8)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().card.into()),
            border: Border {
                color: byteui::theme::color::current().border,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// Todo 日历日期选择器的窗口级 overlay 版本:不再挂在卡片下方,而是铺在
/// 整张窗口之上、定位到点击按钮时的光标锚点(像右键菜单一样出现在按钮旁
/// 边)。返回 `None` 表示没有可弹的日历(`calendar_open` 为真但锚点/项目
/// 缺失,理论上不会到——调用方降级为只铺 dismiss 收起层,避免卡在打开态)。
///
/// `window_size` 用于边界钳制。任务已存的 plan_date 直接读 `TodoInfo`。
pub fn todo_calendar_overlay<'a>(
    ws: &Workspace,
    window_size: (f32, f32),
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let ws_state = &ws.todo;
    let idx = ws_state.calendar_open?;
    let anchor = ws_state.calendar_anchor?;
    let item = ws_state.items.get(idx)?;
    let selected = item.plan_date.clone();
    let popup = todo_calendar_popup(idx, ws_state.calendar_view, selected);

    // 全窗口容器 + padding 把弹层推到锚点;窗口边界钳制,避免日历超出右下。
    let (ax, ay) = anchor;
    let window_w = window_size.0;
    let window_h = window_size.1;
    // 日历估算尺寸:7 列 × 28px + 内边距(8×2)≈ 212 宽;标题 + 星期行 + 6
    // 行 × 24px + 间距 + 内边距 ≈ 240 高。
    let pop_w = 216.0_f32;
    let pop_h = 240.0_f32;
    let x = ax.min((window_w - pop_w).max(0.0));
    let y = ay.min((window_h - pop_h).max(0.0));
    Some(
        container(popup)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: y,
                left: x,
                right: 0.0,
                bottom: 0.0,
            })
            .into(),
    )
}

/// 分类树导航区:钉顶的"全部"/"未分类"伪节点 + 用户自建分类节点(可
/// 展开/收起、点选切过滤)。本函数只做展示 + 选中;右键菜单/增删改在
/// 后续任务接入。
pub(crate) fn category_tree_nav<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![].spacing(2).padding([4, 8]);

    // "全部"/"未分类"伪节点可右键弹出顶层"新建分类"(见
    // `CategoryContextMenuOpen(None)` 的语义)。两者都新建的是顶层分类
    // (新建后并不会真挂在哪个名字下面——全部/未分类只是视图桶)。
    col = col.push(byteui::interaction::context_menu::wrap(
        category_pseudo_row(
            icons::IconKind::CircleSmall,
            "全部",
            ws_state.category_selected() == CategoryFilter::All,
            Message::CategorySelect(CategoryFilter::All),
        ),
        Some(Message::CategoryContextMenuOpen(None)),
    ));
    col = col.push(byteui::interaction::context_menu::wrap(
        category_pseudo_row(
            icons::IconKind::CircleSmall,
            "未分类",
            ws_state.category_selected() == CategoryFilter::Uncategorized,
            Message::CategorySelect(CategoryFilter::Uncategorized),
        ),
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
        .on_press(Message::CategorySelect(CategoryFilter::Node(row.id)))
        .width(Length::Fill)
        .padding([6, 4 + (row.depth as u16) * 16])
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
        });
        col = col.push(byteui::interaction::context_menu::wrap(
            label.into(),
            Some(Message::CategoryContextMenuOpen(Some(row.id))),
        ));
    }
    col.into()
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

/// 完成时间(毫秒) → "MM-DD"（SUCCESS 徽章用，只取月日）。
pub(crate) fn format_todo_month_day(ms: u64) -> String {
    let secs = ms / 1000;
    let days = (secs / 86400) as i64;
    let (_y, m, d) = civil_from_days(days);
    format!("{m:02}-{d:02}")
}

/// civil-from-days：把"自 1970-01-01 的天数"换算成 (年, 月, 日)。
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
