//! Database 面板 view:驱动弹窗/数据源树/表单/内容 pane/浏览与查询视图/
//! schema 树/结果表格/tab 栏与焦点捕获。

use byteui::interaction::icons;
use iced_widget::core::{Border, Element, Length};
use iced_widget::{MouseArea, Scrollable, button, column, container, row, scrollable, text};

use super::*;

/// 驱动管理弹窗:窗口级居中浮层,视觉模板同 `delete_confirm_popup`(CARD
/// 底 + 圆角描边 + 标题图标)。此前走 `crate::chrome::menu::shell_frosted`(右键
/// 菜单同款外壳)、内联挂在数据源列表下方,不居中也没有遮罩——改成跟本面板
/// 其它弹窗一致的普通弹窗(2026-09-15)。每行一个 checkbox 前置位的条目
/// (启用的打 ✓),点按切换启用/禁用,不关弹窗。窗口级 overlay,由
/// `app.rs` 挂载(见其调用点注释),`pub` 是为了让那边能调到。
pub fn drivers_popup<'a>(
    app_state: &'a AppState,
    window_width: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let title = row![
        icons::view(
            icons::IconKind::Database,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().cream,
        ),
        text("管理驱动")
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().cream),
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);

    let mut items_col = column![].spacing(2);
    for driver in DriverKind::ALL {
        let enabled = app_state.is_enabled(driver);
        let checkbox: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
            text(if enabled { "✓" } else { " " })
                .size(byteui::theme::font::body())
                .into();
        items_col = items_col.push(crate::chrome::menu::item_row_fill(
            Some(checkbox),
            driver.label(),
            byteui::theme::color::current().body,
            Some(Message::ToggleDriver(driver)),
        ));
    }

    let close = button(
        text("关闭")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim),
    )
    .on_press(Message::DriversPopupToggle)
    .padding([6, 12])
    .style(crate::dialog::action_button_style(
        byteui::theme::color::current().dim,
    ));

    let dialog = container(
        column![
            title,
            text("勾选的驱动才会出现在「新增数据源」的驱动下拉里。")
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().dim),
            items_col,
            crate::dialog::actions(row![close]),
        ]
        .spacing(10),
    )
    .padding(16)
    // 宽度改用 `dialog::width`(整窗 1/3,2026-09-15 统一约定),取代此前
    // 写死的 280px。
    .width(crate::dialog::width(window_width))
    .style(crate::dialog::card_style);

    container(dialog)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}

/// 数据源根节点图标:关系型驱动统一用 `Database`(圆柱),MongoDB 用
/// `Leaf`。现有图标库没有贴切的各厂商品牌图标,退而求其次按"关系型/
/// 文档型"两类区分形状——颜色仍走主题染色(dim),不引入固定品牌色,
/// 跟本仓库其它 icon 的处理口径一致。
pub(crate) fn driver_icon(driver: DriverKind) -> icons::IconKind {
    match driver {
        DriverKind::MongoDB => icons::IconKind::Leaf,
        DriverKind::Postgres | DriverKind::MySQL | DriverKind::Sqlite => icons::IconKind::Database,
    }
}

/// 树行左侧缩进宽度,对齐 header 行的 chevron + 图标列(子行没有自己的
/// chevron 时用这个占位,行内文字信息用这个当左边距)。
pub(crate) fn source_tree_indent() -> Length {
    Length::Fixed(byteui::theme::icon_size::chevron() + byteui::theme::icon_size::tree_row_gap())
}

pub(crate) fn indented_line<'a>(
    text_str: impl Into<String>,
    color: iced_widget::core::Color,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    row![
        iced_widget::space::Space::new().width(source_tree_indent()),
        text(text_str.into())
            .size(byteui::theme::font::caption_sm())
            .color(color),
    ]
    .into()
}

/// 数据源树的一个根节点:header 行(chevron + 驱动图标 + 名字,左键展开/
/// 收起、右键弹测试连接/编辑/删除/刷新菜单,见 `app.rs` 的
/// `database_source_context_menu_popup`)+ 展开时的 schema/表子树。子树
/// 结构复用阶段 2 现成的 `SchemaState`/`tree_rows`/`schema_tree_row`——此前
/// 是切到独立整页 `schema_tree_view`,这次改成内联挂在同一个可滚动列表里。
pub(crate) fn source_tree_node<'a>(
    source: &'a DataSource,
    status: &'a TestStatus,
    expanded: bool,
    st: Option<&'a SchemaState>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let chevron = if expanded {
        icons::IconKind::ChevronDown
    } else {
        icons::IconKind::ChevronRight
    };
    let header = MouseArea::new(
        button(
            row![
                icons::view(
                    chevron,
                    byteui::theme::icon_size::chevron(),
                    byteui::theme::color::current().dim
                ),
                icons::view(
                    driver_icon(source.driver),
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().dim
                ),
                text(source.name.clone())
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().cream),
            ]
            .spacing(byteui::theme::icon_size::tree_row_gap())
            .align_y(iced_widget::core::Alignment::Center),
        )
        .on_press(Message::ToggleSourceExpanded(source.id.clone()))
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
            background: None,
            ..iced_widget::button::Style::default()
        }),
    )
    .on_right_press(Message::SourceContextMenu(source.id.clone()));

    let mut col = column![header].spacing(2);

    let status_line = match status {
        TestStatus::Idle => None,
        TestStatus::Testing => Some(("测试中…".to_string(), byteui::theme::color::current().dim)),
        TestStatus::Ok => Some((
            "✓ 连接成功".to_string(),
            byteui::theme::color::current().green,
        )),
        TestStatus::Err(e) => Some((format!("✗ {e}"), byteui::theme::color::current().red)),
    };
    if let Some((line, color)) = status_line {
        col = col.push(indented_line(line, color));
    }

    if !expanded {
        return col.into();
    }

    if source.driver != DriverKind::MongoDB {
        col = col.push(row![
            iced_widget::space::Space::new().width(source_tree_indent()),
            button(
                text("+ 新查询")
                    .size(byteui::theme::font::caption_sm())
                    .color(byteui::theme::color::current().dim)
            )
            .on_press(Message::OpenQueryTab(source.id.clone()))
            .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
                background: None,
                ..iced_widget::button::Style::default()
            }),
        ]);
    }

    let dim = byteui::theme::color::current().dim;
    let Some(st) = st else {
        return col.push(indented_line("加载中…", dim)).into();
    };

    // 旧快照在手 + 正在刷新 → 一行"刷新中…"提示,旧树照常(同 git_log 不闪空惯例)
    if st.loading_tables() && !st.tables().is_empty() {
        col = col.push(indented_line("刷新中…", dim));
    }
    if let Some(e) = st.tables_error()
        && !st.tables().is_empty()
    {
        col = col.push(indented_line(
            format!("刷新失败:{e}"),
            byteui::theme::color::current().red,
        ));
    }

    if st.loading_tables() && st.tables().is_empty() {
        col.push(indented_line("加载中…", dim)).into()
    } else if st.tables().is_empty() {
        if let Some(e) = st.tables_error() {
            col.push(indented_line(
                format!("✗ {e}(右键菜单可重试)"),
                byteui::theme::color::current().red,
            ))
            .into()
        } else {
            col.push(indented_line("该库没有表或视图", dim)).into()
        }
    } else {
        for r in tree_rows(st, source.driver) {
            col = col.push(row![
                iced_widget::space::Space::new().width(source_tree_indent()),
                schema_tree_row(&source.id, source.driver, r),
            ]);
        }
        col.into()
    }
}

/// 新增/编辑数据源弹窗:窗口级居中浮层,视觉模板同 `delete_confirm_popup`
/// (CARD 底 + 圆角描边 + 标题图标)。SQLite 只留"文件路径"一栏,其它驱动
/// 列出 host/port/database/username/password。标题图标用面板自己的
/// `icons::IconKind::Database`(同 `home_panel_head` 头部图标),不用
/// footer 按钮的 `SquarePlus`——同 Todo「清空列表」弹窗的既有口径:标题
/// 图标标的是"这是哪个面板的弹窗",不重复按钮本身的动作语义。`pub` 是
/// 为了让 `app.rs` 的窗口级 overlay 能调到。
pub fn source_form<'a>(
    draft: &'a DataSourceDraft,
    app_state: &'a AppState,
    test_status: &'a TestStatus,
    window_width: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let title_text = if draft.id.is_some() {
        "编辑数据源"
    } else {
        "新增数据源"
    };
    let title = row![
        icons::view(
            icons::IconKind::Database,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().cream,
        ),
        text(title_text)
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().cream),
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);

    let mut driver_options: Vec<DriverOption> = Vec::new();
    for driver in DriverKind::ALL {
        // 只列已启用的驱动;若正在编辑的数据源本身用的驱动已被禁用,
        // 仍把它加回来并标注"已禁用"(设计文档"目标"第 4 条)——这里判断
        // "已禁用但是当前草稿正用着"这个特例。
        let is_current = draft.driver == driver;
        if !app_state.is_enabled(driver) && !is_current {
            continue;
        }
        let label = if app_state.is_enabled(driver) {
            driver.label().to_string()
        } else {
            format!("{}(已禁用)", driver.label())
        };
        driver_options.push(DriverOption { driver, label });
    }
    let selected_driver_option = driver_options
        .iter()
        .find(|o| o.driver == draft.driver)
        .cloned();
    let driver_select = byteui::form::select::view(
        driver_options,
        selected_driver_option,
        |opt: DriverOption| Message::DraftDriverChanged(opt.driver),
    );

    let mut col = column![title, driver_select].spacing(8);
    col = col.push(wrap_form_input(
        byteui::form::input_text::view_on_bg(
            "名字",
            &draft.name,
            false,
            Some(form_field_id("name")),
            false,
            None,
            false,
            Message::DraftNameChanged,
        ),
        form_field_id("name"),
        false,
    ));
    if draft.driver == DriverKind::Sqlite {
        col = col.push(wrap_form_input(
            byteui::form::input_text::view_on_bg(
                "文件路径",
                &draft.database,
                false,
                Some(form_field_id("sqlite-database")),
                false,
                None,
                false,
                Message::DraftDatabaseChanged,
            ),
            form_field_id("sqlite-database"),
            false,
        ));
    } else {
        col = col.push(wrap_form_input(
            byteui::form::input_text::view_on_bg(
                "连接 URI(可选,填了则忽略下面各项,例如 postgres://user:pw@host:5432/db)",
                &draft.uri,
                false,
                Some(form_field_id("uri")),
                false,
                None,
                false,
                Message::DraftUriChanged,
            ),
            form_field_id("uri"),
            false,
        ));
        col = col.push(wrap_form_input(
            byteui::form::input_text::view_on_bg(
                "host",
                &draft.host,
                false,
                Some(form_field_id("host")),
                false,
                None,
                false,
                Message::DraftHostChanged,
            ),
            form_field_id("host"),
            false,
        ));
        col = col.push(wrap_form_input(
            byteui::form::input_text::view_on_bg(
                "port",
                &draft.port,
                false,
                Some(form_field_id("port")),
                false,
                None,
                false,
                Message::DraftPortChanged,
            ),
            form_field_id("port"),
            false,
        ));
        col = col.push(wrap_form_input(
            byteui::form::input_text::view_on_bg(
                "database",
                &draft.database,
                false,
                Some(form_field_id("database")),
                false,
                None,
                false,
                Message::DraftDatabaseChanged,
            ),
            form_field_id("database"),
            false,
        ));
        col = col.push(wrap_form_input(
            byteui::form::input_text::view_on_bg(
                "username",
                &draft.username,
                false,
                Some(form_field_id("username")),
                false,
                None,
                false,
                Message::DraftUsernameChanged,
            ),
            form_field_id("username"),
            false,
        ));
        col = col.push(wrap_form_input(
            byteui::form::input_text::view_on_bg(
                "password(留空则不修改)",
                &draft.password,
                true,
                Some(form_field_id("password")),
                false,
                None,
                false,
                Message::DraftPasswordChanged,
            ),
            form_field_id("password"),
            true,
        ));
    }
    // 按钮样式照抄 `ssh.rs::host_form` 的 `text_btn`:透明背景 + 1px
    // 描边(描边色=文字色),不再用纯色填充按钮,跟主机表单保持同一产品
    // 语言(设计文档回顾)。
    let text_btn = |label: &'a str, color: iced_widget::core::Color, msg: Message| {
        button(text(label).size(byteui::theme::font::label()).color(color))
            .on_press(msg)
            .padding([6, 12])
            .style(
                move |_t: &iced_widget::Theme, _s| iced_widget::button::Style {
                    background: Some(byteui::theme::color::current().bg.into()),
                    border: iced_widget::core::Border {
                        color,
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    text_color: color,
                    ..iced_widget::button::Style::default()
                },
            )
    };
    // 按钮分两组:"测试连接"靠左;保存/取消靠右,中间用 `Fill` 空位把
    // 两组顶到卡片两端——同 `ssh.rs::host_form` 的既有布局(2026-09-15 新增
    // "测试连接",此前只有保存/取消,靠 `dialog::actions` 整行右对齐;现在
    // 两组各自的对齐关系仍旧,`row!` 里混进一个 `Length::Fill` 子元素会让
    // 外层 `row!` 自动升级成 `Fill` 宽度,不需要再套一层 `dialog::actions`)。
    let left = row![text_btn(
        "测试连接",
        byteui::theme::color::current().cream,
        Message::DraftTestConnection
    )]
    .spacing(6);
    let right = row![
        text_btn(
            "保存",
            byteui::theme::color::current().cream,
            Message::DraftSave
        ),
        text_btn(
            "取消",
            byteui::theme::color::current().dim,
            Message::DraftCancel
        ),
    ]
    .spacing(6);
    col = col.push(
        row![left, iced_widget::Space::new().width(Length::Fill), right]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
    );

    // 测试结果提示行,视觉/文案照抄 `source_tree_node` 卡片上那行状态文字
    // (`Idle` 不显示任何内容,同 `ssh.rs::host_form` 的 `status_text` 既有
    // 处理)。
    let (status_text, status_color) = match test_status {
        TestStatus::Idle => (String::new(), byteui::theme::color::current().dim),
        TestStatus::Testing => ("测试中…".to_string(), byteui::theme::color::current().dim),
        TestStatus::Ok => (
            "✓ 连接成功".to_string(),
            byteui::theme::color::current().green,
        ),
        TestStatus::Err(e) => (format!("✗ {e}"), byteui::theme::color::current().red),
    };
    if !status_text.is_empty() {
        col = col.push(
            text(status_text)
                .size(byteui::theme::font::caption_sm())
                .color(status_color),
        );
    }

    // 边框/底色统一成原生预览"文件内搜索"风格(`find_field_shell`/`find_rows`
    // 外层组合的既有配色):底色 card、边框普通态 `colors.border`(不再恒描
    // 金)——2026-09-11 需求,数据库/主机新增表单跟文件内搜索输入框对齐。
    // 宽度从 `Fill`(此前内联挂在数据源列表下方,撑满面板宽度)改成
    // `dialog::width`(整窗 1/3,2026-09-15 统一约定,取代中间态的写死
    // 420px)——现在是窗口级居中弹窗(见函数文档),撑满宽度会让输入框
    // 铺满整个窗口,不像"普通弹窗"。
    let dialog = container(col)
        .padding(12)
        .width(crate::dialog::width(window_width))
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(byteui::theme::color::current().card.into()),
            border: iced_widget::core::Border {
                color: byteui::theme::color::current().border,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..iced_widget::container::Style::default()
        });

    container(dialog)
        .width(iced_widget::core::Length::Fill)
        .height(iced_widget::core::Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}

/// 面板主视图:数据源卡片列表。驱动管理/新增编辑表单/删除确认三个弹窗
/// 已改为窗口级 overlay(见 `app.rs` 的 `popped` 分支),不再需要
/// `AppState` 参数。
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 顶部面板标题固定、中间可滚动、底部「管理驱动/新增数据源」footer-bar
    // 固定在面板最下方——镜像 `ssh.rs::view`/`ssh_footer_bar` 的既有布局
    // (原先两个按钮跟标题挤在同一行,不随内容滚动分区,验收反馈参照主机
    // 面板"添加主机"统一到 footer-bar)。
    let head = container(crate::chrome::homespace::home_panel_head(
        icons::IconKind::Database,
        "数据库",
    ))
    .padding(crate::theme::region::project_pane().padding);

    let mut list = column![].spacing(12).padding([0, 20]);

    if ws_state.sources().is_empty() {
        list = list.push(
            text("还没有数据源")
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        );
    } else {
        for source in ws_state.sources() {
            let expanded = ws_state.is_expanded(&source.id);
            list = list.push(source_tree_node(
                source,
                ws_state.test_status(&source.id),
                expanded,
                ws_state.schema_state(&source.id),
            ));
        }
    }

    let scroll = Scrollable::new(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .direction(scrollable::Direction::Vertical(
            byteui::interaction::scrollbar::scrollbar(),
        ))
        .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style());

    let body = column![head, scroll, database_footer_bar()].spacing(0);

    let base = container(body)
        .width(width)
        .height(iced_widget::core::Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(byteui::theme::color::current().bg.into()),
                border: outer,
                ..iced_widget::container::Style::default()
            },
        );

    // 「删除数据源」确认框 / 「新增/编辑数据源」表单 / 「管理驱动」**不**
    // 在这里叠(此前的 panel-level `stack!` 只在本面板的 `width` 范围内
    // 居中,而不是整个软件窗体——2026-09-15 改为窗口级 overlay,由
    // `app.rs` 顶层 `popped` 分支挂载,同 `todo::clear_confirm_popup` 的
    // 既有口径,见 `delete_confirm_popup`/`source_form`/`drivers_popup`
    // 文档。三者互斥优先级(app.rs 侧 if/else if 链保证同一时刻只显示
    // 一个):待确认删除 > 新增/编辑表单 > 驱动管理。
    base.into()
}

/// 删除数据源确认框:居中浮层,列出要删的数据源名,确认(红)才执行
/// `DeleteSource`,取消/遮罩只清待确认态。视觉照抄 `ssh.rs::
/// delete_confirm_popup`(CARD 底 + 圆角描边 + 取消/确认两个圆角按钮)。
/// 窗口级 overlay,由 `app.rs` 挂载(见其调用点注释),`pub` 是为了让那边
/// 能调到。
pub fn delete_confirm_popup<'a>(
    ws_state: &'a WorkspaceState,
    source_id: &'a str,
    window_width: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let name = ws_state
        .sources()
        .iter()
        .find(|s| s.id == source_id)
        .map(|s| s.name.to_string())
        .unwrap_or_else(|| source_id.to_string());
    crate::dialog::confirm(
        crate::dialog::ConfirmDialog {
            icon: None,
            title: format!("删除数据源 \"{name}\"?"),
            description: "这会永久删除这条连接记录及其保存的密码。".to_string(),
            cancel_label: "取消".to_string(),
            cancel_msg: Message::DeleteSourceCancel,
            confirm_label: "删除".to_string(),
            confirm_msg: Message::DeleteSource(source_id.to_string()),
            confirm_color: byteui::theme::color::current().red,
            content_spacing: 8.0,
        },
        window_width,
    )
}

/// 数据库面板底部 footer-bar:1px `BORDER` 分隔线 + `padding([6, 8])` 容器,
/// 结构照抄 `ssh.rs::ssh_footer_bar`(同一产品语言——"管理驱动"+"新增数据源"
/// 两个按钮固定在面板最下方,不随数据源列表滚动)。两者都是非破坏性操作,
/// 按统一按钮规范(见 `dialog::action_button_border_color` 文档)走灰字 +
/// 描边静止态 `border`、悬浮/按下态变 `gold`(此前固定奶油字 + 不响应
/// hover 的静态描边)。
pub(crate) fn database_footer_bar<'a>()
-> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let btn = |icon: icons::IconKind, label: &'static str, on_press: Message| {
        button(
            row![
                icons::view(
                    icon,
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().dim,
                ),
                text(label)
                    .size(byteui::theme::font::label())
                    .color(byteui::theme::color::current().dim),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .on_press(on_press)
        .padding([4, 8])
        .style(|_t: &iced_widget::Theme, s| button::Style {
            background: Some(byteui::theme::color::current().bg.into()),
            border: iced_widget::core::Border {
                color: crate::dialog::action_button_border_color(s),
                width: 1.0,
                radius: 4.0.into(),
            },
            text_color: byteui::theme::color::current().dim,
            ..button::Style::default()
        })
    };

    let bar = row![
        iced_widget::space::horizontal(),
        btn(
            icons::IconKind::Settings,
            "管理驱动",
            Message::DriversPopupToggle,
        ),
        btn(
            icons::IconKind::SquarePlus,
            "新增数据源",
            Message::AddSourceStart
        ),
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);

    let top_line = container(iced_widget::Space::new())
        .width(iced_widget::core::Length::Fill)
        .height(iced_widget::core::Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..iced_widget::container::Style::default()
        });

    let pp = crate::theme::region::project_pane().padding;
    container(column![top_line, bar].spacing(4))
        .width(iced_widget::core::Length::Fill)
        .padding(iced_widget::core::Padding {
            top: 6.0,
            right: pp.right,
            bottom: 6.0,
            left: pp.left,
        })
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: None,
            ..iced_widget::container::Style::default()
        })
        .into()
}

/// 右侧内容窗格:tab 栏(表/集合/查询)+ 当前激活 tab 的内容。骨架照抄
/// `workspace.rs::preview_pane_for`,**不**带拖拽换位/右键菜单(设计文档
/// 非目标)。
pub fn content_pane<'a>(
    app: &'a crate::app::App,
    ws_state: &'a WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let content = ws_state.content();
    // "空白"占位 tab 固定打头,不对应 `content.tabs()` 里任何一条记录,
    // 选中态即 `content.active_idx() == None`——参考 SSH 面板
    // `app.rs::ssh_tab_bar` 同款设计(见 `Message::SelectBlankTab`)。
    // hover key 用 `usize::MAX`,真实 tab 下标不可能到这个值。
    const BLANK_HOVER_KEY: usize = usize::MAX;
    let blank_active = content.active_idx().is_none();
    // 空白占位 tab 标题前挂 Dozer 品牌标(同 SSH tab 的 Terminal 前缀图标口径),
    // 让"还没有打开任何数据源"的落点带上产品识别度。
    let blank_icon = icons::view(
        icons::IconKind::Dozer,
        byteui::theme::icon_size::row(),
        byteui::theme::color::current().dim,
    );
    let mut entries: Vec<(
        f32,
        Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    )> = vec![(
        // tab 宽度估算要把前缀图标(图标宽 + 4px 间距)算进去,窗口裁剪才准。
        tab_title_display_width("空白") + byteui::theme::icon_size::row() + 4.0,
        crate::chrome::tab_widget::panel_tab(crate::chrome::tab_widget::PanelTabArgs {
            title: "空白".to_string(),
            active: blank_active,
            hover_t: app.hover_progress(crate::app::HoverId::DatabaseTabItem(BLANK_HOVER_KEY)),
            close_hover_t: app
                .hover_progress(crate::app::HoverId::DatabaseTabClose(BLANK_HOVER_KEY)),
            prefix: Some(blank_icon),
            suffix: None,
            on_select: Message::SelectBlankTab,
            on_close: Message::SelectBlankTab,
            show_tooltip: app
                .hover_tooltip_ready(crate::app::HoverId::DatabaseTabItem(BLANK_HOVER_KEY)),
            title_hover: move |h| {
                Message::TabHover(DatabaseTabHoverTarget::Title, BLANK_HOVER_KEY, h)
            },
            close_hover: move |h| {
                Message::TabHover(DatabaseTabHoverTarget::Close, BLANK_HOVER_KEY, h)
            },
        }),
    )];
    entries.extend(content.tabs().iter().enumerate().map(|(idx, tab)| {
        let active = Some(idx) == content.active_idx();
        let title_hover_t = app.hover_progress(crate::app::HoverId::DatabaseTabItem(idx));
        let close_hover_t = app.hover_progress(crate::app::HoverId::DatabaseTabClose(idx));
        let title = tab_title(tab, ws_state);
        (
            tab_title_display_width(&title),
            crate::chrome::tab_widget::panel_tab(crate::chrome::tab_widget::PanelTabArgs {
                title,
                active,
                hover_t: title_hover_t,
                close_hover_t,
                prefix: None,
                suffix: None,
                on_select: Message::SelectTab(idx),
                on_close: Message::CloseTab(idx),
                show_tooltip: app.hover_tooltip_ready(crate::app::HoverId::DatabaseTabItem(idx)),
                title_hover: move |h| Message::TabHover(DatabaseTabHoverTarget::Title, idx, h),
                close_hover: move |h| Message::TabHover(DatabaseTabHoverTarget::Close, idx, h),
            }),
        )
    }));
    let widths: Vec<f32> = entries.iter().map(|(w, _)| *w).collect();
    let window = crate::chrome::tab_widget::tab_window(
        &widths,
        4.0,
        byteui::theme::geometry::tab_bar_avail_px(),
        content.tab_scroll_first(),
    );
    let items: Vec<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> = entries
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| (window.first..window.visible_end).contains(idx))
        .map(|(_, (_, el))| el)
        .collect();
    let tabs_row = row(items).spacing(4);
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    // V 只数**真实** tab,不算"空白"占位——下拉本就不列空白(见
    // `tab_overflow_popup`),只剩空白页时 V 本身也不该显示(验收反馈)。
    let db_tab_total = content.tabs().len();
    let overflow_button = crate::chrome::tab_widget::tab_overflow_button(
        db_tab_total,
        app.hover_progress(crate::app::HoverId::DatabaseTabOverflow),
        Message::TabOverflowToggle,
        move |hovered| Message::Hover(crate::app::HoverId::DatabaseTabOverflow, hovered),
    );
    // 内容侧"收起/展开列表列"按钮(收起左列 schema 树后仍在此可见以便恢复)。
    // 消息为本地 `Message::ToggleListCollapse`,由内核 `App::update` 拦截。
    let collapse = app.list_collapse_button(
        crate::app::PanelKind::Database,
        app.list_collapsed(crate::app::PanelKind::Database),
        crate::app::HoverId::DatabaseListCollapse,
        "收起列表",
        "展开列表",
        Message::ToggleListCollapse,
        move |hovered| Message::Hover(crate::app::HoverId::DatabaseListCollapse, hovered),
    );
    let mut tab_bar_row = row![]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);
    if let Some(btn) = overflow_button {
        tab_bar_row = tab_bar_row.push(btn);
    }
    let tab_bar = tab_bar_row.push(clipped).push(collapse);

    // tab 栏下方 1px 分割线,同 SSH/预览面板的 `tab_divider()`(此前漏加,
    // 验收反馈 tab 下方少了一根横线)。外层 padding/tab 栏间距改用
    // `terminal_pane` region(同 `app.rs::ssh_terminal_pane` 的既有取值:
    // padding 8、gap 4)——之前硬编码 16/8 是 SSH 面板的两倍,tab 栏看起来
    // 比主机面板厚一圈(验收反馈"右侧 tab 高度太高")。
    let region = crate::theme::region::terminal_pane();
    let mut col = column![tab_bar, crate::app::tab_divider()].spacing(region.gap);

    match content.active_tab() {
        None => {
            col = col.push(
                container(
                    text("在左侧 schema 树点一张表/视图/集合,或点 + 新查询")
                        .size(byteui::theme::font::subtitle())
                        .color(byteui::theme::color::current().dim),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(iced_widget::core::alignment::Horizontal::Center)
                .align_y(iced_widget::core::alignment::Vertical::Center),
            );
        }
        Some(tab) => {
            let tab_id = tab.id;
            match content.content(tab_id) {
                Some(TabContent::Browse(b)) => col = col.push(browse_view(tab_id, b)),
                Some(TabContent::Query(q)) => col = col.push(query_view(tab_id, q)),
                None => {}
            }
        }
    }

    let outer_container = container(col.padding(region.padding))
        .width(width)
        .height(iced_widget::core::Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(byteui::theme::color::current().bg.into()),
                border: outer,
                ..iced_widget::container::Style::default()
            },
        );
    outer_container.into()
}

/// `tab_overflow_popup` 的原生菜单版本——纯选择列表,由内核在
/// `app.rs::Message::Database(TabOverflowToggle)` 里调用(需要 `App::last_cursor`
/// 当锚点,同 `tab_overflow_popup` 的既有拦截理由)。仅 macOS 编译。
#[cfg(target_os = "macos")]
pub(crate) fn tab_overflow_items(
    ws_state: &WorkspaceState,
) -> Vec<crate::chrome::native_menu::Item<Message>> {
    let content = ws_state.content();
    let mut entries: Vec<(usize, String, bool)> = Vec::new();
    for (i, tab) in content.tabs().iter().enumerate() {
        let idx = i + 1;
        entries.push((
            idx,
            tab_title(tab, ws_state),
            Some(i) == content.active_idx(),
        ));
    }
    crate::chrome::tab_widget::tab_overflow_items(&entries, |idx| {
        if idx == 0 {
            Message::SelectBlankTab
        } else {
            Message::SelectTab(idx - 1)
        }
    })
}

/// Database 面板 tab 栏"溢出下拉"浮层。**必须**在 `App::view` 顶层
/// `stack![base, ...]` 里拼(同 `terminal::term_tab_overflow_popup` 文档
/// 解释的理由——`anchor`/`window_size` 是全窗口坐标系,嵌在 `content_pane`
/// 自己的局部布局里换算位置会跟真实点击位置对不上)。
pub fn tab_overflow_popup<'a>(
    app: &'a crate::app::App,
    ws_state: &'a WorkspaceState,
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let content = ws_state.content();
    let anchor = content.tab_overflow_anchor()?;
    // 下拉列出该面板内**真实** tab,方便一览/直接跳到任一项,而不是只列
    // 当前被挤出可见区的子集。"空白"占位不进列表(点开也没什么可跳的);
    // 下标沿用调用方 `on_select`/`on_close`(`idx - 1` 换算)既有的 1 起步
    // 方案(0 留给空白,虽然它现在不会出现在列表里,翻译逻辑不用跟着改)。
    let mut overflow_entries: Vec<crate::chrome::tab_widget::TabOverflowEntry<'_, Message>> =
        Vec::new();
    for (i, tab) in content.tabs().iter().enumerate() {
        let idx = i + 1;
        overflow_entries.push(crate::chrome::tab_widget::TabOverflowEntry {
            index: idx,
            prefix: None,
            title: tab_title(tab, ws_state),
            active: Some(i) == content.active_idx(),
            closable: true,
            hover_t: app.hover_progress(crate::app::HoverId::TabOverflowRow(idx)),
        });
    }
    Some(crate::chrome::tab_widget::tab_overflow_menu(
        crate::chrome::tab_widget::TabOverflowMenuArgs {
            entries: overflow_entries,
            anchor,
            window_size: app.window_size,
            on_select: |idx| {
                if idx == 0 {
                    Message::SelectBlankTab
                } else {
                    Message::SelectTab(idx - 1)
                }
            },
            on_close: |idx| Message::CloseTab(idx - 1),
            on_dismiss: Message::TabOverflowDismiss,
            on_row_hover: move |idx, hovered| {
                Message::Hover(crate::app::HoverId::TabOverflowRow(idx), hovered)
            },
        },
    ))
}

pub(crate) fn tab_title(tab: &DatabaseTab, ws_state: &WorkspaceState) -> String {
    let source_name = |id: &str| {
        ws_state
            .sources()
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| id.to_string())
    };
    match &tab.kind {
        DatabaseTabKind::Table {
            source_id,
            schema,
            table,
        } => match schema {
            Some(s) => format!("{table}@{}.{s}", source_name(source_id)),
            None => format!("{table}@{}", source_name(source_id)),
        },
        DatabaseTabKind::Collection { source_id, name } => {
            format!("{name}@{}", source_name(source_id))
        }
        DatabaseTabKind::Query {
            source_id,
            console_seq,
        } => format!("查询 {console_seq}@{}", source_name(source_id)),
    }
}

pub(crate) fn tab_title_display_width(title: &str) -> f32 {
    // 同 `workspace.rs::preview_tab_display_width` 的估算思路:字符数 *
    // 单字宽 + tab 内边距/关闭按钮的固定开销,不做真实文本测量(tab_window
    // 只需要一个足够准的相对宽度做窗口裁剪)。
    title.chars().count() as f32 * 8.0 + 56.0
}

/// 按"空白占位 + 各数据库 tab"的真实估算宽重建宽度向量并调用
/// `DatabaseContentState::reveal_tab` 把选中 tab 带入可见区。用实际内容宽
/// (而非早年的统一上限 `PANEL_TAB_MAX_W`)喂给翻页窗口数学,见渲染侧
/// `content_pane` 同款 `tab_title_display_width` 口径。
pub(crate) fn reveal_tab_widths(ws_state: &mut WorkspaceState, target: usize) {
    let widths = {
        let content = ws_state.content();
        let mut w = vec![tab_title_display_width("空白")];
        w.extend(
            content
                .tabs()
                .iter()
                .map(|tab| tab_title_display_width(&tab_title(tab, ws_state))),
        );
        w
    };
    ws_state.content_mut().reveal_tab(&widths, target);
}

// 各输入框/SQL 编辑器的稳定 `widget::Id`,供右键菜单把焦点移到被右键的
// 输入(复制/粘贴作用于它,见 main.rs 的合成键盘事件)。表单字段是单例、
// 固定 id;浏览 WHERE/ORDER BY 与查询 SQL 编辑器按 `tab_id` 区分(同个 tab
// 内是单例,多个 tab 互不抢焦点)。
pub(crate) fn form_field_id(component: &'static str) -> iced_widget::core::widget::Id {
    match component {
        "name" => iced_widget::core::widget::Id::new("db-form-name"),
        "sqlite-database" => iced_widget::core::widget::Id::new("db-form-sqlite-database"),
        "uri" => iced_widget::core::widget::Id::new("db-form-uri"),
        "host" => iced_widget::core::widget::Id::new("db-form-host"),
        "port" => iced_widget::core::widget::Id::new("db-form-port"),
        "database" => iced_widget::core::widget::Id::new("db-form-database"),
        "username" => iced_widget::core::widget::Id::new("db-form-username"),
        "password" => iced_widget::core::widget::Id::new("db-form-password"),
        _ => iced_widget::core::widget::Id::new("db-form-unknown"),
    }
}

/// `form_field_id` 全部合法 component 名字,`CaptureFormFocus` 用它逐个比对
/// 当前遍历到的 focusable id 是不是新增/编辑表单的某个字段——不用挨个字段
/// 单独开一个 `HoverId` 式的 static,一次遍历顺带查完这张表单所有字段。
const DB_FORM_FIELD_COMPONENTS: &[&str] = &[
    "name",
    "sqlite-database",
    "uri",
    "host",
    "port",
    "database",
    "username",
    "password",
];

static DB_FORM_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走并复位(消费式)数据库新增/编辑表单**任意一个字段**上一帧是否持有
/// iced 内部真实焦点——同 `extensions::files::take_search_focused` 的既有
/// 桥接手法。main.rs 键盘路由据此放行给标准 iced 管线,不再像旧版那样只看
/// "表单是否打开"这个粗粒度信号(2026-09 用户反馈根因:数据库/主机面板与
/// Agent 终端分栏同屏显示时,表单开着但用户实际点进的是右侧终端输入框,
/// 旧版信号仍卡真,导致终端收不到任何按键——面板"可不可见"跟字段"是否真
/// 聚焦"是两回事,必须查后者)。
pub(crate) fn take_form_focused() -> bool {
    std::mem::replace(&mut *DB_FORM_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍:命中 `DB_FORM_FIELD_COMPONENTS` 里任一
/// 字段且该字段真聚焦,就把 `DB_FORM_FOCUSED` 置真。只在找到"真聚焦"时才
/// 写 `true`,不在遇到未聚焦的匹配字段时写回 `false`——由 `take_form_focused`
/// 消费式复位负责清零(同一帧内至多一个 widget 持有真焦点,不会有两个匹配
/// 字段互相覆盖出错误结果)。`traverse` 必须调用传入的 `operate` 闭包才能
/// 继续递归子节点,道理同 `extensions::files::CaptureSearchFocus` 的既有文档。
pub(crate) struct CaptureFormFocus;
impl iced_widget::core::widget::Operation<()> for CaptureFormFocus {
    fn focusable(
        &mut self,
        id: Option<&iced_widget::core::widget::Id>,
        _bounds: iced_widget::core::Rectangle,
        state: &mut dyn iced_widget::core::widget::operation::Focusable,
    ) {
        let Some(id) = id else { return };
        for component in DB_FORM_FIELD_COMPONENTS {
            if *id == form_field_id(component) && state.is_focused() {
                *DB_FORM_FOCUSED.lock().unwrap() = true;
                return;
            }
        }
    }

    fn traverse(
        &mut self,
        operate: &mut dyn for<'a> FnMut(
            &'a mut (dyn iced_widget::core::widget::Operation<()> + 'a),
        ),
    ) {
        operate(self);
    }
}
pub(crate) fn new_tab_field_id(
    component: &'static str,
    _tab_id: usize,
) -> iced_widget::core::widget::Id {
    // 内容窗格同一时刻只渲染激活 tab 的控件,不同 tab 用同一个静态 id 不冲突
    // (激活 tab 的编辑器是唯一在 widget 树里的那个)。
    match component {
        "browse-where" => iced_widget::core::widget::Id::new("db-browse-where"),
        "browse-order" => iced_widget::core::widget::Id::new("db-browse-order"),
        "query" => iced_widget::core::widget::Id::new("db-query"),
        _ => iced_widget::core::widget::Id::new("db-unknown"),
    }
}

/// 给数据库表单/浏览/编辑器等输入框套上统一右键菜单封装:外包
/// `MouseArea::on_right_press`,右键下发 `TextInputMenuOpen`(见
/// `byteui::interaction::context_menu`)。`secure` 输入(密码)禁用
/// 复制/剪切(菜单项灰掉),见 `text_input_menu_popup`。
pub(crate) fn wrap_form_input<'a, I>(
    input: I,
    id: iced_widget::core::widget::Id,
    secure: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>
where
    I: Into<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
{
    byteui::interaction::context_menu::wrap(
        input.into(),
        Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
            id,
            secure,
        })),
    )
}

pub(crate) fn browse_view<'a>(
    tab_id: usize,
    b: &'a BrowseState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let toolbar = row![
        wrap_form_input(
            byteui::form::input_text::view(
                "WHERE(原始 SQL 片段,例如 id > 100)",
                &b.where_clause,
                false,
                Some(new_tab_field_id("browse-where", tab_id)),
                false,
                Some(Message::BrowseRun(tab_id)),
                false,
                move |v| Message::BrowseWhereChanged(tab_id, v),
            ),
            new_tab_field_id("browse-where", tab_id),
            false,
        ),
        wrap_form_input(
            byteui::form::input_text::view(
                "ORDER BY(原始 SQL 片段,例如 title DESC)",
                &b.order_by,
                false,
                Some(new_tab_field_id("browse-order", tab_id)),
                false,
                Some(Message::BrowseRun(tab_id)),
                false,
                move |v| Message::BrowseOrderByChanged(tab_id, v),
            ),
            new_tab_field_id("browse-order", tab_id),
            false,
        ),
        byteui::form::select::view(&PAGE_SIZES[..], Some(&b.page_size), move |v| {
            Message::BrowsePageSizeChanged(tab_id, v)
        }),
        button(text("上一页")).on_press_maybe((b.page > 0).then_some(Message::BrowsePrev(tab_id))),
        button(text("下一页")).on_press_maybe(b.has_more.then_some(Message::BrowseNext(tab_id))),
    ]
    .spacing(8)
    .align_y(iced_widget::core::Alignment::Center);

    let mut col = column![toolbar].spacing(8);

    if b.loading {
        col = col.push(
            text("加载中…")
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().dim),
        );
    }
    if let Some(e) = &b.error {
        col = col.push(
            text(format!("✗ {e}"))
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().red),
        );
    }

    match &b.result {
        None => {
            if !b.loading && b.error.is_none() {
                col = col.push(
                    text("暂无数据")
                        .size(byteui::theme::font::body())
                        .color(byteui::theme::color::current().dim),
                );
            }
        }
        Some(result) if result.rows.is_empty() => {
            col = col.push(
                text("该表当前没有数据(或筛选条件不匹配任何行)")
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().dim),
            );
        }
        Some(result) => {
            col = col.push(result_table(tab_id, result, true));
        }
    }

    col.into()
}

/// 结果网格。`sortable` 为 `true` 时(浏览页)列头可点写排序;查询控制台
/// 的结果(`sortable=false`)纯展示,不接排序点击(设计文档"架构与数据流
/// §6":查询结果不支持再排序)。
pub(crate) fn result_table<'a>(
    tab_id: usize,
    result: &'a QueryResult,
    sortable: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 点列头 = 把 `"{列名} ASC"` 写进 ORDER BY 框(设计文档"架构与数据流
    // §5":同列再点一次不做两态切换,DESC 由用户在框里手动追加——原始片段
    // 输入框的既定口径下,点击只是个"快速起手")。
    let header = row(result
        .columns
        .iter()
        .map(|c| {
            let label = text(c.clone())
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().cream);
            if sortable {
                let asc = format!("{c} ASC");
                button(label)
                    .on_press(Message::BrowseOrderByChanged(tab_id, asc))
                    .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
                        background: None,
                        ..iced_widget::button::Style::default()
                    })
                    .into()
            } else {
                container(label).into()
            }
        })
        .collect::<Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>>>())
    .spacing(12);

    let rows_col = column(
        result
            .rows
            .iter()
            .map(|r| {
                row(r
                    .iter()
                    .map(|cell| match cell {
                        CellValue::Text(s) => text(s.clone())
                            .size(byteui::theme::font::caption_sm())
                            .color(byteui::theme::color::current().body)
                            .into(),
                        CellValue::Null => text("NULL")
                            .size(byteui::theme::font::caption_sm())
                            .color(byteui::theme::color::current().dim)
                            .into(),
                    })
                    .collect::<Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>>>())
                .spacing(12)
                .into()
            })
            .collect::<Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>>>(),
    )
    .spacing(4);

    scrollable(column![header, rows_col].spacing(6))
        .direction(scrollable::Direction::Both {
            vertical: byteui::interaction::scrollbar::scrollbar(),
            horizontal: byteui::interaction::scrollbar::scrollbar(),
        })
        .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style())
        .into()
}

/// SQL 查询控制台 tab。`text_editor` 多行输入 + "执行"按钮 + 结果区;
/// `QueryOutcome` 三态(行集 / 受影响行数 / DDL)分别渲染。
pub(crate) fn query_view<'a>(
    tab_id: usize,
    q: &'a QueryState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let toolbar = row![
        button(text(if q.running { "执行中…" } else { "执行" }))
            .on_press_maybe((!q.running).then_some(Message::QueryRun(tab_id))),
        text("Cmd+Enter 快捷执行")
            .size(byteui::theme::font::caption_sm())
            .color(byteui::theme::color::current().dim),
    ]
    .spacing(8)
    .align_y(iced_widget::core::Alignment::Center);

    let editor = wrap_form_input(
        byteui::form::text_area::view(
            &q.sql,
            "SELECT * FROM ...",
            Some(new_tab_field_id("query", tab_id)),
            false,
            Some(160.0),
            move |action| Message::QueryTextAction(tab_id, action),
        ),
        new_tab_field_id("query", tab_id),
        false,
    );

    let mut col = column![toolbar, editor].spacing(8);

    if let Some(e) = &q.error {
        col = col.push(
            text(format!("✗ {e}"))
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().red),
        );
    }

    match &q.result {
        None => {}
        Some(QueryOutcome::Rows(result)) if result.rows.is_empty() => {
            col = col.push(
                text("查询未返回任何行")
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().dim),
            );
        }
        Some(QueryOutcome::Rows(result)) => {
            col = col.push(result_table(tab_id, result, false));
        }
        Some(QueryOutcome::Affected(n)) => {
            col = col.push(
                text(format!("{n} 行受影响"))
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().green),
            );
        }
        Some(QueryOutcome::Ddl) => {
            col = col.push(
                text("执行成功")
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().green),
            );
        }
    }

    col.into()
}

/// 单行渲染。source_id 用于构造 `ToggleTable`/`OpenTableTab`/`OpenCollectionTab`,driver
/// 决定 MongoDB 集合行(无 chevron、点文字开集合 tab)与关系型表/视图行的差异。
pub(crate) fn schema_tree_row<'a>(
    source_id: &str,
    driver: DriverKind,
    r: SchemaRow<'a>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let indent = text("  ".repeat(r.depth)).size(crate::workspace::tree_row_font_size());
    match r.kind {
        SchemaRowKind::Schema(name) => {
            let chevron = if r.expanded {
                icons::IconKind::ChevronDown
            } else {
                icons::IconKind::ChevronRight
            };
            let folder = if r.expanded {
                icons::IconKind::FolderOpen
            } else {
                icons::IconKind::Folder
            };
            button(
                row![
                    indent,
                    icons::view(
                        chevron,
                        byteui::theme::icon_size::chevron(),
                        byteui::theme::color::current().dim
                    ),
                    icons::view(
                        folder,
                        byteui::theme::icon_size::row(),
                        byteui::theme::color::current().dim
                    ),
                    text(name.to_string())
                        .size(crate::workspace::tree_row_font_size())
                        .color(byteui::theme::color::current().cream),
                ]
                .spacing(byteui::theme::icon_size::tree_row_gap())
                .align_y(iced_widget::core::Alignment::Center),
            )
            .on_press(Message::ToggleSchema(
                source_id.to_string(),
                name.to_string(),
            ))
            .width(Length::Fill)
            .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
                background: None,
                ..iced_widget::button::Style::default()
            })
            .into()
        }
        SchemaRowKind::Table(t) => {
            let icon_kind = if t.is_view {
                icons::IconKind::Eye
            } else {
                icons::IconKind::Table // MongoDB 集合复用这个图标,视觉上够用(设计文档)
            };
            let icon = icons::view(
                icon_kind,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().dim,
            );
            let label = text(t.name.clone())
                .size(crate::workspace::tree_row_font_size())
                .color(byteui::theme::color::current().cream);
            let open_msg = if driver == DriverKind::MongoDB {
                Message::OpenCollectionTab {
                    source_id: source_id.to_string(),
                    name: t.name.clone(),
                }
            } else {
                Message::OpenTableTab {
                    source_id: source_id.to_string(),
                    schema: t.schema.clone(),
                    table: t.name.clone(),
                }
            };
            let label_btn = button(
                row![icon, label]
                    .spacing(byteui::theme::icon_size::tree_row_gap())
                    .align_y(iced_widget::core::Alignment::Center),
            )
            .on_press(open_msg)
            .width(Length::Fill)
            .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
                background: None,
                ..iced_widget::button::Style::default()
            });

            let mut row_el = row![indent].spacing(byteui::theme::icon_size::tree_row_gap());
            if driver == DriverKind::MongoDB {
                // MongoDB 集合无列层可展开,不画 chevron,留同宽空位对齐。
                row_el = row_el.push(
                    iced_widget::space::Space::new()
                        .width(Length::Fixed(byteui::theme::icon_size::chevron())),
                );
            } else {
                let chevron_icon = if r.expanded {
                    icons::IconKind::ChevronDown
                } else {
                    icons::IconKind::ChevronRight
                };
                let chevron_btn = button(icons::view(
                    chevron_icon,
                    byteui::theme::icon_size::chevron(),
                    byteui::theme::color::current().dim,
                ))
                .on_press(Message::ToggleTable {
                    source_id: source_id.to_string(),
                    schema: t.schema.clone(),
                    table: t.name.clone(),
                })
                .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
                    background: None,
                    ..iced_widget::button::Style::default()
                });
                row_el = row_el.push(chevron_btn);
            }
            row_el
                .push(label_btn)
                .align_y(iced_widget::core::Alignment::Center)
                .into()
        }
        SchemaRowKind::Column(c) => {
            // 可空性用颜色深浅表达:非空 CREAM、可空 BODY(不加 "NOT NULL" 文本)
            let name_color = if c.nullable {
                byteui::theme::color::current().body
            } else {
                byteui::theme::color::current().cream
            };
            row![
                indent,
                iced_widget::space::Space::new()
                    .width(Length::Fixed(
                        byteui::theme::icon_size::chevron()
                            + byteui::theme::icon_size::tree_row_gap()
                            + byteui::theme::icon_size::row()
                            + byteui::theme::icon_size::tree_row_gap(),
                    ))
                    .height(Length::Shrink),
                text(c.name.clone())
                    .size(crate::workspace::tree_row_font_size())
                    .color(name_color),
                text(c.type_name.clone())
                    .size(byteui::theme::font::caption_sm())
                    .color(byteui::theme::color::current().dim),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center)
            .into()
        }
        SchemaRowKind::ColumnsLoading => row![
            indent,
            iced_widget::space::Space::new()
                .width(Length::Fixed(
                    byteui::theme::icon_size::chevron()
                        + byteui::theme::icon_size::tree_row_gap()
                        + byteui::theme::icon_size::row()
                        + byteui::theme::icon_size::tree_row_gap(),
                ))
                .height(Length::Shrink),
            text("加载列中…")
                .size(crate::workspace::tree_row_font_size())
                .color(byteui::theme::color::current().dim),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center)
        .into(),
        SchemaRowKind::ColumnsFailed(e) => row![
            indent,
            iced_widget::space::Space::new()
                .width(Length::Fixed(
                    byteui::theme::icon_size::chevron()
                        + byteui::theme::icon_size::tree_row_gap()
                        + byteui::theme::icon_size::row()
                        + byteui::theme::icon_size::tree_row_gap(),
                ))
                .height(Length::Shrink),
            text(format!("列加载失败:{e}"))
                .size(crate::workspace::tree_row_font_size())
                .color(byteui::theme::color::current().red),
            text("(收起再展开可重试)")
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().dim),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center)
        .into(),
    }
}

/// 右侧内容窗格里的一个 tab。`id` 是跨重排/关闭都稳定的标识(消息/异步
/// 结果按它路由,不用索引——索引会随关闭其它 tab 而漂移)。
pub struct DatabaseTab {
    pub id: usize,
    pub kind: DatabaseTabKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DatabaseTabKind {
    Table {
        source_id: String,
        schema: Option<String>,
        table: String,
    },
    Collection {
        source_id: String,
        name: String,
    },
    /// `console_seq` 只用来生成默认标题("查询 1"/"查询 2"),不参与去重
    /// 比较——`open_query` 永远新开,同一数据源可以有多个查询 tab。
    Query {
        source_id: String,
        console_seq: u32,
    },
}

impl DatabaseTabKind {
    fn source_id(&self) -> &str {
        match self {
            DatabaseTabKind::Table { source_id, .. }
            | DatabaseTabKind::Collection { source_id, .. }
            | DatabaseTabKind::Query { source_id, .. } => source_id,
        }
    }
}

/// 表格/集合浏览页的页大小可选项。
pub const PAGE_SIZES: [u32; 3] = [50, 100, 500];

/// 表格/集合浏览 tab 的状态。`run_seq` 是过期结果防线:每次发起查询
/// `+1` 并带进异步闭包,结果落地时核对是否仍是发出时那个值(设计文档
/// "异步路由与过期防线"一节)。
pub struct BrowseState {
    pub where_clause: String,
    pub order_by: String,
    pub page: u32,
    pub page_size: u32,
    pub has_more: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub result: Option<QueryResult>,
    pub(crate) run_seq: u64,
}

impl Default for BrowseState {
    fn default() -> Self {
        Self {
            where_clause: String::new(),
            order_by: String::new(),
            page: 0,
            page_size: PAGE_SIZES[0],
            has_more: false,
            loading: false,
            error: None,
            result: None,
            run_seq: 0,
        }
    }
}

impl BrowseState {
    /// 发起一次新请求前调用:`run_seq +1` 并置 loading,返回新 seq 供
    /// 异步闭包携带。
    pub fn begin_run(&mut self) -> u64 {
        self.run_seq += 1;
        self.loading = true;
        self.run_seq
    }

    /// 结果落地时核对:seq 不是当前这轮 → 过期,调用方应丢弃不落地。
    pub fn is_current_run(&self, seq: u64) -> bool {
        self.loading && self.run_seq == seq
    }
}

/// SQL 查询控制台 tab 的状态。`sql` 用 `text_editor::Content`(同
/// `todo.rs` 任务内容多行编辑框的既有用法),不是纯 `String`——
/// `byteui::form::text_area::view` 要求这个类型。
pub struct QueryState {
    pub sql: iced_widget::text_editor::Content,
    pub running: bool,
    pub error: Option<String>,
    pub result: Option<QueryOutcome>,
    pub(crate) run_seq: u64,
}

impl Default for QueryState {
    fn default() -> Self {
        Self {
            sql: iced_widget::text_editor::Content::new(),
            running: false,
            error: None,
            result: None,
            run_seq: 0,
        }
    }
}

impl QueryState {
    pub fn begin_run(&mut self) -> u64 {
        self.run_seq += 1;
        self.running = true;
        self.run_seq
    }

    pub fn is_current_run(&self, seq: u64) -> bool {
        self.running && self.run_seq == seq
    }
}

pub enum TabContent {
    Browse(BrowseState),
    Query(QueryState),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Text(String),
    Null,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<CellValue>>,
}

#[derive(Debug, Clone)]
pub enum QueryOutcome {
    Rows(QueryResult),
    Affected(u64),
    Ddl,
}

/// 右侧内容窗格:多个表/集合/查询 tab,`active` 是**索引**(同
/// `PreviewPane::active_idx()` 的约定,tab 栏渲染/hover 状态按索引找)。
/// `None` = 当前选中的是 tab 栏最前面固定的"空白"占位 tab(不在
/// `tabs` 里,参考 `extensions/ssh.rs::ssh_active` 同款 `Option` 设计)。
/// `contents` 按**稳定 id**存(消息/异步结果按 id 路由,索引会随关闭
/// 漂移)。
#[derive(Default)]
pub struct DatabaseContentState {
    pub(crate) tabs: Vec<DatabaseTab>,
    pub(crate) contents: std::collections::HashMap<usize, TabContent>,
    pub(crate) active: Option<usize>,
    pub(crate) next_id: usize,
    pub(crate) next_console_seq: u32,
    /// tab 栏箭头翻页的窗口起点(同 `Workspace::preview_tab_first` 的用法),
    /// 每次渲染都交给 `tab_window` 钳到合法范围,这里存的只是"用户上次翻到
    /// 哪"的粗略意图。
    pub(crate) tab_scroll_first: usize,
    /// tab 栏溢出下拉的悬浮锚点,语义同 `Workspace::term_tab_overflow_anchor`。
    pub(crate) tab_overflow_anchor: Option<(f32, f32)>,
}

impl DatabaseContentState {
    pub fn tabs(&self) -> &[DatabaseTab] {
        &self.tabs
    }

    pub fn active_idx(&self) -> Option<usize> {
        self.active
    }

    pub fn tab_scroll_first(&self) -> usize {
        self.tab_scroll_first
    }

    pub fn tab_overflow_anchor(&self) -> Option<(f32, f32)> {
        self.tab_overflow_anchor
    }

    // 仅非 mac 平台调用:mac 上 `TabOverflowToggle` 直接同步弹原生 NSMenu,
    // 不再写这个锚点(见 `app.rs` 对应分支)。
    #[cfg(not(target_os = "macos"))]
    pub fn toggle_tab_overflow(&mut self, cursor: (f32, f32)) {
        self.tab_overflow_anchor = if self.tab_overflow_anchor.is_some() {
            None
        } else {
            Some(cursor)
        };
    }

    pub fn dismiss_tab_overflow(&mut self) {
        self.tab_overflow_anchor = None;
    }

    /// 选中 `target`(扁平下标,0 留给空白占位 tab)后,若它当前隐藏,重新
    /// 钳出包含它的窗口;已可见则不动。`widths` 由渲染侧按 `tab_bar_avail_px`
    /// 同一套口径传入(含开头的空白占位 tab 宽度)。
    pub fn reveal_tab(&mut self, widths: &[f32], target: usize) {
        self.tab_scroll_first = crate::chrome::tab_widget::tab_window_reveal(
            widths,
            4.0,
            byteui::theme::geometry::tab_bar_avail_px(),
            self.tab_scroll_first,
            target,
        );
    }

    pub fn active_tab(&self) -> Option<&DatabaseTab> {
        self.tabs.get(self.active?)
    }

    pub fn content(&self, id: usize) -> Option<&TabContent> {
        self.contents.get(&id)
    }

    pub fn content_mut(&mut self, id: usize) -> Option<&mut TabContent> {
        self.contents.get_mut(&id)
    }

    fn push(&mut self, kind: DatabaseTabKind, content: TabContent) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(DatabaseTab { id, kind });
        self.contents.insert(id, content);
        self.active = Some(self.tabs.len() - 1);
        id
    }

    /// 已开同一张表的 tab → 聚焦(内容/游标不重置);否则新开一个空浏览态。
    pub fn open_table(
        &mut self,
        source_id: String,
        schema: Option<String>,
        table: String,
    ) -> usize {
        if let Some((idx, tab)) = self.tabs.iter().enumerate().find(|(_, t)| {
            matches!(&t.kind, DatabaseTabKind::Table { source_id: s, schema: sc, table: tb }
                if *s == source_id && *sc == schema && *tb == table)
        }) {
            self.active = Some(idx);
            return tab.id;
        }
        self.push(
            DatabaseTabKind::Table {
                source_id,
                schema,
                table,
            },
            TabContent::Browse(BrowseState::default()),
        )
    }

    /// 已开同一个集合的 tab → 聚焦;否则新开。
    pub fn open_collection(&mut self, source_id: String, name: String) -> usize {
        if let Some((idx, tab)) = self.tabs.iter().enumerate().find(|(_, t)| {
            matches!(&t.kind, DatabaseTabKind::Collection { source_id: s, name: n }
                if *s == source_id && *n == name)
        }) {
            self.active = Some(idx);
            return tab.id;
        }
        self.push(
            DatabaseTabKind::Collection { source_id, name },
            TabContent::Browse(BrowseState::default()),
        )
    }

    /// 查询 tab 永不去重,`console_seq` 递增当默认标题的编号来源。
    pub fn open_query(&mut self, source_id: String) -> usize {
        self.next_console_seq += 1;
        let seq = self.next_console_seq;
        self.push(
            DatabaseTabKind::Query {
                source_id,
                console_seq: seq,
            },
            TabContent::Query(QueryState::default()),
        )
    }

    pub fn select(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active = Some(idx);
        }
        self.tab_overflow_anchor = None;
    }

    /// 点固定的"空白"占位 tab:不对应 `tabs` 里任何一条记录,选中态就是
    /// `active == None`(同 `ssh::update` 里 `Message::SelectBlankTab` 的
    /// 处理)。
    pub fn select_blank(&mut self) {
        self.active = None;
        self.tab_overflow_anchor = None;
    }

    /// 关闭指定索引的 tab。`active` 调整规则同浏览器标签页惯例:关掉
    /// active 之前的 tab → active 索引减 1(仍指向原 tab);关掉 active
    /// 自己且不是最后一个 → active 索引不变(自然落到后一个 tab 上);
    /// 关掉最后一个 tab 且它正是 active → active 收缩到新的最后一个。
    /// `active == None`(正显示"空白"占位 tab)时关掉某个后台 tab 不改变
    /// 选中态,仍留在空白 tab 上。
    pub fn close(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let id = self.tabs[idx].id;
        self.tabs.remove(idx);
        self.contents.remove(&id);
        if self.tabs.is_empty() {
            self.active = None;
            return;
        }
        match self.active {
            None => {}
            Some(a) if a >= self.tabs.len() => self.active = Some(self.tabs.len() - 1),
            Some(a) if idx < a => self.active = Some(a - 1),
            Some(_) => {}
        }
    }

    /// 数据源被删除/编辑保存(连接信息可能变了)时,关掉所有关联 tab。
    pub fn close_by_source(&mut self, source_id: &str) {
        while let Some(idx) = self
            .tabs
            .iter()
            .position(|t| t.kind.source_id() == source_id)
        {
            self.close(idx);
        }
    }
}

/// 数据浏览/查询执行用的驱动原生连接池(区别于阶段 1/2 introspection
/// 专用的 `sqlx::AnyPool`——原生池才能正确解码真实列类型,设计文档
/// "架构与数据流 §4"一节)。
pub(crate) enum NativePool {
    Pg(sqlx::PgPool),
    MySql(sqlx::MySqlPool),
    Sqlite(sqlx::SqlitePool),
}

pub(crate) async fn connect_native(kind: DriverKind, url: &str) -> Result<NativePool, String> {
    match kind {
        DriverKind::Postgres => sqlx::PgPool::connect(url)
            .await
            .map(NativePool::Pg)
            .map_err(|e| e.to_string()),
        DriverKind::MySQL => sqlx::MySqlPool::connect(url)
            .await
            .map(NativePool::MySql)
            .map_err(|e| e.to_string()),
        DriverKind::Sqlite => sqlx::SqlitePool::connect(url)
            .await
            .map(NativePool::Sqlite)
            .map_err(|e| e.to_string()),
        DriverKind::MongoDB => unreachable!("connect_native 不处理 MongoDB"),
    }
}

pub(crate) async fn close_native(pool: NativePool) {
    match pool {
        NativePool::Pg(p) => p.close().await,
        NativePool::MySql(p) => p.close().await,
        NativePool::Sqlite(p) => p.close().await,
    }
}

/// 行 → 结果集:表头来自首行的列元信息(0 行结果时没有列头——已知限制,
/// 见设计文档"错误处理"一节的补充说明,浏览页对 0 行走友好提示而不是
/// 空表头)。三种 `Row` 类型共享这一个泛型函数。
pub(crate) fn rows_to_result<R: sqlx::Row>(
    rows: &[R],
    stringify: impl Fn(&R) -> Vec<CellValue>,
) -> QueryResult
where
    <R::Database as sqlx::Database>::Column: sqlx::Column,
{
    use sqlx::Column;
    let columns = rows
        .first()
        .map(|r| r.columns().iter().map(|c| c.name().to_string()).collect())
        .unwrap_or_default();
    let rows = rows.iter().map(&stringify).collect();
    QueryResult { columns, rows }
}

/// Postgres 行转字符串。常见标量类型直接 match `TypeInfo::name()`;
/// UUID/时间/JSON(B)/NUMERIC 走 Step 1 新加的 sqlx feature 解码;解不出的
/// 生僻类型(数组、自定义枚举、复合类型…)显示占位,不 panic、不让整行
/// 失败(设计文档"架构与数据流 §4")。
pub(crate) fn stringify_pg_row(row: &sqlx::postgres::PgRow) -> Vec<CellValue> {
    use sqlx::{Row, TypeInfo, ValueRef};
    let mut out = Vec::with_capacity(row.len());
    for i in 0..row.len() {
        let Ok(raw) = row.try_get_raw(i) else {
            out.push(CellValue::Null);
            continue;
        };
        if raw.is_null() {
            out.push(CellValue::Null);
            continue;
        }
        let type_name = raw.type_info().name().to_ascii_uppercase();
        let decoded: Option<String> = match type_name.as_str() {
            "BOOL" => row.try_get::<bool, _>(i).ok().map(|v| v.to_string()),
            "INT2" => row.try_get::<i16, _>(i).ok().map(|v| v.to_string()),
            "INT4" => row.try_get::<i32, _>(i).ok().map(|v| v.to_string()),
            "INT8" => row.try_get::<i64, _>(i).ok().map(|v| v.to_string()),
            "FLOAT4" => row.try_get::<f32, _>(i).ok().map(|v| v.to_string()),
            "FLOAT8" => row.try_get::<f64, _>(i).ok().map(|v| v.to_string()),
            "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" | "CITEXT" => row.try_get::<String, _>(i).ok(),
            "BYTEA" => row
                .try_get::<Vec<u8>, _>(i)
                .ok()
                .map(|b| format!("<{} bytes>", b.len())),
            "UUID" => row
                .try_get::<sqlx::types::Uuid, _>(i)
                .ok()
                .map(|v| v.to_string()),
            "TIMESTAMP" => row
                .try_get::<sqlx::types::chrono::NaiveDateTime, _>(i)
                .ok()
                .map(|v| v.to_string()),
            "TIMESTAMPTZ" => row
                .try_get::<sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>, _>(i)
                .ok()
                .map(|v| v.to_string()),
            "DATE" => row
                .try_get::<sqlx::types::chrono::NaiveDate, _>(i)
                .ok()
                .map(|v| v.to_string()),
            "JSON" | "JSONB" => row
                .try_get::<serde_json::Value, _>(i)
                .ok()
                .map(|v| v.to_string()),
            "NUMERIC" => row
                .try_get::<sqlx::types::Decimal, _>(i)
                .ok()
                .map(|v| v.to_string()),
            _ => None,
        };
        out.push(match decoded {
            Some(s) => CellValue::Text(s),
            None => CellValue::Text(format!("<不支持的类型: {type_name}>")),
        });
    }
    out
}

/// MySQL 行转字符串,类型名集合参照 `information_schema.columns.data_type`
/// 在 MySQL 里的常见取值(大写)。
pub(crate) fn stringify_mysql_row(row: &sqlx::mysql::MySqlRow) -> Vec<CellValue> {
    use sqlx::{Row, TypeInfo, ValueRef};
    let mut out = Vec::with_capacity(row.len());
    for i in 0..row.len() {
        let Ok(raw) = row.try_get_raw(i) else {
            out.push(CellValue::Null);
            continue;
        };
        if raw.is_null() {
            out.push(CellValue::Null);
            continue;
        }
        let type_name = raw.type_info().name().to_ascii_uppercase();
        let decoded: Option<String> = match type_name.as_str() {
            "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "INTEGER" => {
                row.try_get::<i64, _>(i).ok().map(|v| v.to_string())
            }
            "BIGINT" => row.try_get::<i64, _>(i).ok().map(|v| v.to_string()),
            "FLOAT" => row.try_get::<f32, _>(i).ok().map(|v| v.to_string()),
            "DOUBLE" => row.try_get::<f64, _>(i).ok().map(|v| v.to_string()),
            "VARCHAR" | "CHAR" | "TEXT" | "ENUM" => row.try_get::<String, _>(i).ok(),
            "BLOB" | "VARBINARY" | "BINARY" => row
                .try_get::<Vec<u8>, _>(i)
                .ok()
                .map(|b| format!("<{} bytes>", b.len())),
            "DATE" => row
                .try_get::<sqlx::types::chrono::NaiveDate, _>(i)
                .ok()
                .map(|v| v.to_string()),
            "DATETIME" | "TIMESTAMP" => row
                .try_get::<sqlx::types::chrono::NaiveDateTime, _>(i)
                .ok()
                .map(|v| v.to_string()),
            "JSON" => row
                .try_get::<serde_json::Value, _>(i)
                .ok()
                .map(|v| v.to_string()),
            "DECIMAL" => row
                .try_get::<sqlx::types::Decimal, _>(i)
                .ok()
                .map(|v| v.to_string()),
            _ => None,
        };
        out.push(match decoded {
            Some(s) => CellValue::Text(s),
            None => CellValue::Text(format!("<不支持的类型: {type_name}>")),
        });
    }
    out
}

/// SQLite 行转字符串。SQLite 只有 5 种存储类型(含 NULL),`Any` 驱动
/// 原本也能应付——这里用原生池只是为了和 Postgres/MySQL 走同一套
/// `NativePool`/`rows_to_result` 代码路径,不是因为 SQLite 真的需要。
pub(crate) fn stringify_sqlite_row(row: &sqlx::sqlite::SqliteRow) -> Vec<CellValue> {
    use sqlx::{Row, TypeInfo, ValueRef};
    let mut out = Vec::with_capacity(row.len());
    for i in 0..row.len() {
        let Ok(raw) = row.try_get_raw(i) else {
            out.push(CellValue::Null);
            continue;
        };
        if raw.is_null() {
            out.push(CellValue::Null);
            continue;
        }
        let type_name = raw.type_info().name().to_ascii_uppercase();
        let decoded: Option<String> = match type_name.as_str() {
            "INTEGER" | "BOOLEAN" => row.try_get::<i64, _>(i).ok().map(|v| v.to_string()),
            "REAL" => row.try_get::<f64, _>(i).ok().map(|v| v.to_string()),
            "TEXT" => row.try_get::<String, _>(i).ok(),
            "BLOB" => row
                .try_get::<Vec<u8>, _>(i)
                .ok()
                .map(|b| format!("<{} bytes>", b.len())),
            _ => row.try_get::<String, _>(i).ok(), // SQLite 动态类型,兜底当文本试一次
        };
        out.push(match decoded {
            Some(s) => CellValue::Text(s),
            None => CellValue::Text(format!("<不支持的类型: {type_name}>")),
        });
    }
    out
}

pub(crate) fn quote_table(kind: DriverKind, schema: Option<&str>, table: &str) -> String {
    match kind {
        DriverKind::Postgres => match schema {
            Some(s) => format!("\"{s}\".\"{table}\""),
            None => format!("\"{table}\""),
        },
        DriverKind::MySQL | DriverKind::Sqlite => format!("`{table}`"),
        DriverKind::MongoDB => unreachable!("quote_table 不处理 MongoDB"),
    }
}

/// 浏览页 SQL 生成:`LIMIT page_size+1` 用来判断"是否有下一页"(多出的
/// 第 page_size+1 行渲染前丢弃),不做 `COUNT(*)`(设计文档"架构与数据流
/// §5")。`where_clause`/`order_by` 原样拼接,不转义——原始片段输入框的
/// 既定口径,用户对拼错/注入自担。
pub(crate) fn build_browse_sql(
    kind: DriverKind,
    schema: Option<&str>,
    table: &str,
    where_clause: &str,
    order_by: &str,
    page: u32,
    page_size: u32,
) -> String {
    let mut sql = format!("SELECT * FROM {}", quote_table(kind, schema, table));
    let w = where_clause.trim();
    if !w.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(w);
    }
    let o = order_by.trim();
    if !o.is_empty() {
        sql.push_str(" ORDER BY ");
        sql.push_str(o);
    }
    sql.push_str(&format!(
        " LIMIT {} OFFSET {}",
        page_size as u64 + 1,
        page as u64 * page_size as u64
    ));
    sql
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatementKind {
    /// `fetch_all` 走结果集渲染。
    Rows,
    /// `execute` 走"N 行受影响"/DDL 文案。
    Execute,
}

/// 按 SQL 文本首个关键字(大小写不敏感、忽略前导空白)分流。设计文档
/// "架构与数据流 §6":不识别 `RETURNING` 子句,`INSERT`/`UPDATE`/`DELETE`
/// 一律走 `Execute`。
pub(crate) fn classify_statement(sql: &str) -> StatementKind {
    let first_word: String = sql
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase();
    match first_word.as_str() {
        "SELECT" | "WITH" | "SHOW" | "EXPLAIN" | "PRAGMA" => StatementKind::Rows,
        _ => StatementKind::Execute,
    }
}

/// `Execute` 分支里进一步区分"DDL(执行成功,不显示行数)" vs "DML(显示
/// 受影响行数)"。
pub(crate) fn is_ddl_keyword(sql: &str) -> bool {
    let first_word: String = sql
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase();
    matches!(
        first_word.as_str(),
        "CREATE" | "ALTER" | "DROP" | "TRUNCATE"
    )
}

/// 浏览页一次查询的结果:`has_more` 由"多取一行"判断(设计文档"架构与
/// 数据流 §5"),渲染前已把多出的那行丢弃。
#[derive(Debug, Clone)]
pub struct BrowsePage {
    pub result: QueryResult,
    pub has_more: bool,
}

/// `browse_table` 的参数对象:8 个位置参数里 `schema`/`table`/
/// `where_clause`/`order_by` 四个 `Option<&str>`/`&str` 挨在一起,顺序传错
/// 编译器发现不了(Rust Design Patterns:Builder,用具名字段替代同类型
/// 位置参数)。
pub(crate) struct BrowseTableParams<'a> {
    pub(crate) kind: DriverKind,
    pub(crate) url: &'a str,
    pub(crate) schema: Option<&'a str>,
    pub(crate) table: &'a str,
    pub(crate) where_clause: &'a str,
    pub(crate) order_by: &'a str,
    pub(crate) page: u32,
    pub(crate) page_size: u32,
}

pub(crate) async fn browse_table(params: BrowseTableParams<'_>) -> Result<BrowsePage, String> {
    let BrowseTableParams {
        kind,
        url,
        schema,
        table,
        where_clause,
        order_by,
        page,
        page_size,
    } = params;
    let sql = build_browse_sql(kind, schema, table, where_clause, order_by, page, page_size);
    let work = async {
        let pool = connect_native(kind, url).await?;
        let mut result = match &pool {
            NativePool::Pg(p) => {
                let rows = sqlx::query(&sql)
                    .fetch_all(p)
                    .await
                    .map_err(|e| e.to_string())?;
                rows_to_result(&rows, stringify_pg_row)
            }
            NativePool::MySql(p) => {
                let rows = sqlx::query(&sql)
                    .fetch_all(p)
                    .await
                    .map_err(|e| e.to_string())?;
                rows_to_result(&rows, stringify_mysql_row)
            }
            NativePool::Sqlite(p) => {
                let rows = sqlx::query(&sql)
                    .fetch_all(p)
                    .await
                    .map_err(|e| e.to_string())?;
                rows_to_result(&rows, stringify_sqlite_row)
            }
        };
        close_native(pool).await;
        let has_more = result.rows.len() > page_size as usize;
        if has_more {
            result.rows.truncate(page_size as usize);
        }
        Ok::<_, String>(BrowsePage { result, has_more })
    };
    match tokio::time::timeout(std::time::Duration::from_secs(15), work).await {
        Ok(r) => r,
        Err(_) => Err("查询超时(15秒)".to_string()),
    }
}

/// MongoDB 集合浏览:游标式拉取,`limit(page_size+1)` 判"是否有下一页"
/// (同浏览页其余路径,不做 `COUNT(*)`)。单列 `document`,整份 JSON 文本
/// (设计文档"架构与数据流 §4")。
pub(crate) async fn browse_collection(
    url: &str,
    db_name: &str,
    name: &str,
    page: u32,
    page_size: u32,
) -> Result<BrowsePage, String> {
    use futures::stream::TryStreamExt;
    let work = async {
        let client = mongodb::Client::with_uri_str(url)
            .await
            .map_err(|e| e.to_string())?;
        let coll: mongodb::Collection<mongodb::bson::Document> =
            client.database(db_name).collection(name);
        let mut cursor = coll
            .find(mongodb::bson::doc! {})
            .skip(page as u64 * page_size as u64)
            .limit(page_size as i64 + 1)
            .await
            .map_err(|e| e.to_string())?;
        let mut docs = Vec::new();
        while let Some(doc) = cursor.try_next().await.map_err(|e| e.to_string())? {
            docs.push(doc);
        }
        let has_more = docs.len() > page_size as usize;
        docs.truncate(page_size as usize);
        let rows = docs
            .into_iter()
            .map(|d| {
                let json = mongodb::bson::Bson::Document(d).into_relaxed_extjson();
                vec![CellValue::Text(
                    serde_json::to_string_pretty(&json).unwrap_or_default(),
                )]
            })
            .collect();
        Ok::<_, String>(BrowsePage {
            result: QueryResult {
                columns: vec!["document".to_string()],
                rows,
            },
            has_more,
        })
    };
    match tokio::time::timeout(std::time::Duration::from_secs(15), work).await {
        Ok(r) => r,
        Err(_) => Err("查询超时(15秒)".to_string()),
    }
}

/// 任意 SQL 执行入口。语句分流见 `classify_statement`(设计文档"架构与
/// 数据流 §6");超时比被动加载的 5 秒更宽(30秒)——用户主动点"执行"、
/// 愿意等,且任意 SQL 可能是有意的慢查询。
pub(crate) async fn run_query(
    kind: DriverKind,
    url: &str,
    sql: &str,
) -> Result<QueryOutcome, String> {
    let work = async {
        let pool = connect_native(kind, url).await?;
        let outcome = match classify_statement(sql) {
            StatementKind::Rows => {
                let result = match &pool {
                    NativePool::Pg(p) => rows_to_result(
                        &sqlx::query(sql)
                            .fetch_all(p)
                            .await
                            .map_err(|e| e.to_string())?,
                        stringify_pg_row,
                    ),
                    NativePool::MySql(p) => rows_to_result(
                        &sqlx::query(sql)
                            .fetch_all(p)
                            .await
                            .map_err(|e| e.to_string())?,
                        stringify_mysql_row,
                    ),
                    NativePool::Sqlite(p) => rows_to_result(
                        &sqlx::query(sql)
                            .fetch_all(p)
                            .await
                            .map_err(|e| e.to_string())?,
                        stringify_sqlite_row,
                    ),
                };
                QueryOutcome::Rows(result)
            }
            StatementKind::Execute => {
                let affected = match &pool {
                    NativePool::Pg(p) => sqlx::query(sql)
                        .execute(p)
                        .await
                        .map_err(|e| e.to_string())?
                        .rows_affected(),
                    NativePool::MySql(p) => sqlx::query(sql)
                        .execute(p)
                        .await
                        .map_err(|e| e.to_string())?
                        .rows_affected(),
                    NativePool::Sqlite(p) => sqlx::query(sql)
                        .execute(p)
                        .await
                        .map_err(|e| e.to_string())?
                        .rows_affected(),
                };
                if is_ddl_keyword(sql) {
                    QueryOutcome::Ddl
                } else {
                    QueryOutcome::Affected(affected)
                }
            }
        };
        close_native(pool).await;
        Ok::<_, String>(outcome)
    };
    match tokio::time::timeout(std::time::Duration::from_secs(30), work).await {
        Ok(r) => r,
        Err(_) => Err("查询超时(30秒)".to_string()),
    }
}
