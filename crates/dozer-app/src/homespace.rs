// crates/dozer-app/src/homespace.rs
//! 首页落地页(`AppPage::Home`,点顶栏 Dozer 页签进入)专属代码:类型定义与
//! 全部视图构建函数。`App` struct 字段声明/`Message` 枚举/`App::update` 的
//! 消息处理逻辑仍留在 `workspace.rs`(单一数据源+集中调度),拆分边界比照
//! `preview.rs`/`conversation.rs` 的既有先例——本文件不持有 `App`/`Workspace`
//! 的 `impl` 块。见 `docs/superpowers/specs/2026-08-08-home-page-4col-layout-design.md`。

use crate::app::{
    App, HoverId, Message, PaneCorner, RailButton, rail_icon_button, zone_pane_border,
};
use crate::conversation::{self, ConversationMeta};
use crate::delivery;
use crate::extensions::browser;
use crate::theme;
use crate::workspace::{lh, relative_time_text};
use byteui::interaction::icons;
use dozer_core::protocol::ProjectInfo;
use iced_widget::core::{Border, Color, Element, Length, Padding};
use iced_widget::{MouseArea, Scrollable, button, column, container, row, scrollable, text};
use std::path::PathBuf;

/// 首页左栏当前显示哪个 pane。语义、命名对齐工作区 `LeftView`,但这是独立
/// 枚举——首页导航态不与工作区共用,也不持久化(每次 `Message::TopBarHome`
/// 进首页都重置为默认值,见 `workspace.rs` 对应处理器)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum HomeLeftView {
    #[default]
    ProjectList,
    Recents,
}

/// 首页右栏当前显示哪个 pane。目前只有 `Browser` 一个变体,为将来扩展占位
/// (呼应"以后再加其它 pane"的既定方向)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum HomeRightView {
    #[default]
    Browser,
}

/// H0"最近的文件"卡一行(跨项目合并前的中间表示；D4)。`Message::
/// HomeRecentsLoaded` 的载荷用到它，因此至少是 `pub(crate)`(见 `private_interfaces`)。
/// 字段本身也是 `pub(crate)`——`workspace.rs` 里画卡片的视图函数要读它们。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HomeRecentFile {
    pub(crate) path: PathBuf,
    pub(crate) project_name: String,
    pub(crate) modified_ms: u64,
}

/// H0"最近的对话"卡一行；语义同上。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HomeRecentConversation {
    pub(crate) project_name: String,
    pub(crate) meta: ConversationMeta,
}

/// 首页落地页(点顶栏 Dozer 进入):四栏结构,镜像工作区
/// `left_icon_rail / left_zone / right_zone / right_icon_rail`(见
/// `workspace::left_icon_rail`/`left_panel_area` 等),但不做拖拽调宽/收起/
/// 放大——首页没有这个产品需求。左栏默认"项目列表"(`HomeLeftView::
/// ProjectList`),右栏固定"浏览器"(全局态,不绑定项目)。
pub(crate) fn home_page<'a>(
    app: &'a App,
    footbar_state: &'a crate::extensions::footbar::AppState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    // 图标栏要贯穿到页面最底部(覆盖 footbar 上方),因此 footbar 放进中间
    // 列、与三栏面板同行,左右图标栏作为本 row 的兄弟节点一起吃到 Fill 高度。
    let body = row![
        home_left_icon_rail(app),
        column![
            row![
                home_left_zone(app, now_ms),
                home_divider(),
                home_right_zone(app),
            ]
            .height(Length::Fill)
            .width(Length::Fill),
            crate::extensions::footbar::view(footbar_state).map(Message::Footbar),
        ]
        .width(Length::Fill),
        home_right_icon_rail(app),
    ]
    .height(Length::Fill);

    let mut col = column![body].height(Length::Fill);
    if let Some(err) = &app.daemon_error {
        col = col.push(
            text(format!("⚠ {err}"))
                .size(theme::homespace_font::body())
                .color(theme::homespace_color::error()),
        );
    }

    container(col)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::region::background().into()),
            ..container::Style::default()
        })
        .into()
}

/// 首页左右栏之间的静态分隔(不可拖拽)。**不能**复用 `workspace::divider_bar
/// (Divider::LeftRight, ..)`——那个分支的 `MouseArea::on_press` 直接派发
/// `Message::ColumnDragStart`,接入工作区自己的拖宽状态机;首页没有拖拽
/// 调宽的产品需求(spec"目标"第 6 条),原样复用会在首页意外改写工作区
/// 的 `shell_layout` 宽度状态。这里只留一块与 `divider_width()` 同宽的
/// 空白——`Divider::LeftRight` 分支本身也不画任何可见的线/背景色,视觉
/// 效果与之一致,只是去掉了 `MouseArea`/`on_press`。
fn home_divider<'a>() -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    iced_widget::Space::new()
        .width(Length::Fixed(theme::geometry::divider_width()))
        .height(Length::Fill)
        .into()
}

/// 首页左图标栏:项目列表 / Recents 两个图标。没有 collapse 概念(见
/// `Message::HomeLeftIconSelect` 处理器注释),点哪个就切到哪个,恒有一个
/// pane 显示。
fn home_left_icon_rail(
    app: &App,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::left_icon_rail();
    let content = column![
        MouseArea::new(rail_icon_button(
            icons::IconKind::LayoutList,
            app.home_left_view == HomeLeftView::ProjectList,
            app.hover_progress(HoverId::Rail(RailButton::HomeProjectList)),
            Message::HomeLeftIconSelect(HomeLeftView::ProjectList),
            "项目列表",
        ))
        .on_enter(Message::Hover(
            HoverId::Rail(RailButton::HomeProjectList),
            true
        ))
        .on_exit(Message::Hover(
            HoverId::Rail(RailButton::HomeProjectList),
            false
        )),
        MouseArea::new(rail_icon_button(
            icons::IconKind::History,
            app.home_left_view == HomeLeftView::Recents,
            app.hover_progress(HoverId::Rail(RailButton::HomeRecents)),
            Message::HomeLeftIconSelect(HomeLeftView::Recents),
            "最近记录",
        ))
        .on_enter(Message::Hover(HoverId::Rail(RailButton::HomeRecents), true))
        .on_exit(Message::Hover(
            HoverId::Rail(RailButton::HomeRecents),
            false
        )),
    ]
    .spacing(region.gap)
    .padding(region.padding);

    container(content)
        .width(Length::Fixed(theme::geometry::icon_rail_width()))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
}

/// 首页右图标栏:只有浏览器一个图标(`HomeRightView` 目前只有一个变体)。
fn home_right_icon_rail(
    app: &App,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::right_icon_rail();
    let content = column![
        MouseArea::new(rail_icon_button(
            icons::IconKind::Globe,
            app.home_right_view == HomeRightView::Browser,
            app.hover_progress(HoverId::Rail(RailButton::HomeBrowser)),
            Message::HomeRightIconSelect(HomeRightView::Browser),
            "浏览器",
        ))
        .on_enter(Message::Hover(HoverId::Rail(RailButton::HomeBrowser), true))
        .on_exit(Message::Hover(
            HoverId::Rail(RailButton::HomeBrowser),
            false
        )),
    ]
    .spacing(region.gap)
    .padding(region.padding);

    container(content)
        .width(Length::Fixed(theme::geometry::icon_rail_width()))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
}

/// 首页左栏:按 `app.home_left_view` 切换项目列表/Recents 两个 pane。固定宽
/// `h0_sidebar_width()`,不支持拖拽调宽(见 `home_page` 文档)。
fn home_left_zone(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let zone = theme::region::left_zone();
    let inner: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match app.home_left_view {
            HomeLeftView::ProjectList => home_project_list_view(app, now_ms),
            HomeLeftView::Recents => home_recents_view(app, now_ms),
        };
    let zone_box = container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(zone.padding)
        .clip(true)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: zone.background.map(Into::into),
            border: zone.border.unwrap_or_default(),
            ..container::Style::default()
        });
    let m = zone.margin;
    container(zone_box)
        .width(Length::Fixed(theme::geometry::h0_sidebar_width()))
        .height(Length::Fill)
        .padding(Padding {
            top: m.top,
            right: m.right,
            bottom: m.bottom,
            left: m.left,
        })
        .into()
}

/// 首页右栏:恒显示全局浏览器 pane(`app.home_browser`,`project_id` 传
/// `None`)。铺满剩余宽度,不支持拖拽调宽。
fn home_right_zone(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let zone = theme::region::right_zone();
    let inner = browser::view(
        &app.home_browser,
        None,
        theme::geometry::default_split_ratio(),
        Length::Fill,
        zone_pane_border(zone, PaneCorner::All),
    )
    .map(Message::HomeBrowser);
    let zone_box = container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(zone.padding)
        .clip(true)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: zone.background.map(Into::into),
            border: zone.border.unwrap_or_default(),
            ..container::Style::default()
        });
    let m = zone.margin;
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

/// 首页左栏"项目列表" pane(`HomeLeftView::ProjectList`):搜索框占位(D7)、
/// 最近项目卡(取 `app.recent_projects` 前 5 条,D2/D3)、"更多项目"占位(D7)、
/// "＋新增项目"(复用 `Message::ProjectTabPickFolder`)。不画品牌行——顶栏
/// 本身已有 `dozer_home_tab` 品牌页签,这里重复画属于视觉冗余。
/// 通用面板标题组件:图标 + 标题(金色 `subtitle` 字号),标题底部一条 1px
/// panel 标题与图标用的强调色(暖金 `#dcc9a3`)——刻意区别于甲方动作专属的
/// GOLD(`#F2D94E`):标题是装饰性的「section 标」,不是可点的甲方动作。
const PANEL_HEAD_ACCENT: Color = Color::from_rgb8(0xdc, 0xc9, 0xa3);

/// 分割线。各 pane / 卡片标题统一复用,保证视觉一致(首页项目列表、Recents
/// 两卡、工作区文件树、Git 提交图等)。`Message` 泛型——本身不发出任何
/// 交互消息,可在任意 `Message` 类型的视图里直接内嵌。`actions` 为
/// `Some(...)` 时,把该元素推到标题同一行的右侧(靠 `Length::Fill` 的
/// 间隔撑开),用于面板头部右侧的操作按钮(如 Agent 面板的"＋"新建)。
pub(crate) fn home_panel_head_with_actions<'a, Message>(
    icon: icons::IconKind,
    title: &'a str,
    actions: Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>
where
    Message: 'a,
{
    let head_row = row![
        icons::view(icon, crate::theme::icon_size::row(), PANEL_HEAD_ACCENT,),
        text(title)
            .size(theme::homespace_font::subtitle())
            .color(PANEL_HEAD_ACCENT),
    ]
    .spacing(8)
    .align_y(iced_widget::core::Alignment::Center);
    // 有右侧操作就把标题行撑满宽度,用 Fill 间隔把操作推到最右。
    let head_row = if let Some(actions) = actions {
        head_row
            .push(iced_widget::space::horizontal())
            .push(actions)
            .width(Length::Fill)
    } else {
        head_row
    };

    column![
        head_row,
        container(iced_widget::Space::new())
            .width(Length::Fill)
            .height(Length::Fixed(1.0))
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(theme::homespace_color::border().into()),
                ..container::Style::default()
            }),
    ]
    .spacing(8)
    .into()
}

/// 不带右侧操作的 `home_panel_head_with_actions` 简写,其余面板照旧调用它。
pub(crate) fn home_panel_head<'a, Message>(
    icon: icons::IconKind,
    title: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>
where
    Message: 'a,
{
    home_panel_head_with_actions(icon, title, None)
}

/// `app.recent_projects` 为空时画"还没有项目"兜底文案,不崩(spec §4)。
fn home_project_list_view(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![].spacing(16);

    col = col.push(home_panel_head(icons::IconKind::LayoutList, "项目"));

    // 搜索框:视觉占位,不接线(D7；precedent:顶栏 ⌘K 搜索框同款"先视觉后接线")。
    col = col.push(
        container(
            row![
                icons::view(
                    icons::IconKind::Search,
                    crate::theme::icon_size::row(),
                    theme::homespace_color::dim()
                ),
                text("搜索项目…")
                    .size(theme::homespace_font::body())
                    .color(theme::homespace_color::dim()),
            ]
            .spacing(8)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .padding([6, 10])
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::homespace_color::card_bg().into()),
            border: Border {
                color: theme::homespace_color::border(),
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        }),
    );

    if app.recent_projects.is_empty() {
        col = col.push(
            text("还没有项目")
                .size(theme::homespace_font::body())
                .color(theme::homespace_color::dim()),
        );
    } else {
        let mut list = column![].spacing(8);
        for p in app.recent_projects.iter().take(5) {
            let card = button(
                column![
                    lh(text(p.name.clone())
                        .size(theme::homespace_font::body())
                        .color(theme::homespace_color::cream())),
                    lh(
                        text(format!("更新 {}", relative_time_text(p.updated_ms, now_ms)))
                            .size(theme::homespace_font::caption_sm())
                            .color(theme::homespace_color::dim())
                    ),
                    lh(
                        text(format!("创建 {}", relative_time_text(p.created_ms, now_ms)))
                            .size(theme::homespace_font::caption_sm())
                            .color(theme::homespace_color::dim())
                    ),
                    lh(text(p.path.clone())
                        .size(theme::homespace_font::caption_sm())
                        .color(theme::homespace_color::dim())),
                ]
                .spacing(2),
            )
            .on_press(Message::ProjectSelect(p.id))
            .width(Length::Fill)
            .padding(10)
            .style(byteui::interaction::cards::button_card(
                false,
                byteui::theme::color::current().card,
            ));
            list = list.push(card);
        }
        col = col.push(
            Scrollable::new(list)
                .width(Length::Fill)
                .height(Length::Fill)
                .direction(scrollable::Direction::Vertical(
                    byteui::interaction::scrollbar::scrollbar(),
                ))
                .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style()),
        );
    }

    // "更多项目":视觉占位,不接线——对应的"全部项目列表"视图现在不存在,
    // 属于后续增量(D7)。
    col = col.push(
        container(
            text("更多项目")
                .size(theme::homespace_font::caption())
                .color(theme::homespace_color::dim()),
        )
        .padding([6, 0]),
    );

    col = col.push(
        button(
            text("＋新增项目")
                .size(theme::homespace_font::body())
                .color(theme::homespace_color::gold()),
        )
        .on_press(Message::ProjectTabPickFolder)
        .padding([8, 16])
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: Some(theme::homespace_color::card_bg().into()),
            border: Border {
                color: theme::homespace_color::gold(),
                width: 1.0,
                radius: 6.0.into(),
            },
            text_color: theme::homespace_color::gold(),
            ..button::Style::default()
        }),
    );

    // 与外边框保持标准内边距:外层 `zone_box` 只留 1px 圆角裁切余量,内容
    // 若直接贴边会顶到圆角边框,因此这里补一层标准面板内距(与 project_pane
    // 的 8 / 卡片的 10 同量级),让"项目"标题、搜索框、卡片、底部按钮
    // 四周都不顶边框。
    container(col)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(12)
        .into()
}

/// 首页左栏"Recents" pane(`HomeLeftView::Recents`):合并原"最近的文件"/
/// "最近的对话"两卡。左栏宽度固定较窄(`h0_sidebar_width()`),两卡挤不下
/// 并排,改上下堆叠(两张卡自己的外层容器相应把 `width`/`height` 的
/// `FillPortion` 轴对调,见 `home_recent_files_card`/
/// `home_recent_conversations_card`)。
fn home_recents_view(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 与 project list 面板同款:外层 `zone_box` 仅留 1px 圆角裁切余量,这里
    // 再补一层标准内距,让两张卡片(含各自的标题)四周都不顶圆角边框。
    let inner = column![
        home_recent_files_card(app, now_ms),
        home_recent_conversations_card(app, now_ms)
    ]
    .spacing(24)
    .height(Length::Fill);
    container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(12)
        .into()
}

/// "最近的文件"卡：`app.home_recents_loaded` 为 false 时(刚点进 Home 还没等
/// 到异步结果)画"加载中…"，避免第一帧空白跳变(spec §4)。
fn home_recent_files_card(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![home_panel_head(icons::IconKind::FileText, "最近的文件")].spacing(8);

    if !app.home_recents_loaded {
        col = col.push(
            text("加载中…")
                .size(theme::homespace_font::body())
                .color(theme::homespace_color::dim()),
        );
    } else if app.home_recent_files.is_empty() {
        col = col.push(
            text("暂无最近改动的文件")
                .size(theme::homespace_font::body())
                .color(theme::homespace_color::dim()),
        );
    } else {
        for (i, f) in app.home_recent_files.iter().enumerate() {
            let filename = f
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| f.path.display().to_string());
            let row_el = row![
                icons::view(
                    icons::icon_for_file(&filename),
                    crate::theme::icon_size::row(),
                    theme::homespace_color::dim()
                ),
                column![
                    lh(text(filename.clone())
                        .size(theme::homespace_font::body())
                        .color(theme::homespace_color::cream())),
                    lh(text(format!(
                        "{} · {}",
                        f.project_name,
                        relative_time_text(f.modified_ms, now_ms)
                    ))
                    .size(theme::homespace_font::caption_sm())
                    .color(theme::homespace_color::dim())),
                ]
                .spacing(2),
            ]
            .spacing(8)
            .align_y(iced_widget::core::Alignment::Center);
            let hovered = app.hover_progress(HoverId::RecentFile(i)) > 0.0;
            col = col.push(
                MouseArea::new(container(row_el).padding(10).width(Length::Fill).style(
                    move |_t: &iced_widget::Theme| {
                        byteui::interaction::cards::container_card(
                            false,
                            hovered,
                            byteui::theme::color::current().card,
                        )
                    },
                ))
                .on_enter(Message::Hover(HoverId::RecentFile(i), true))
                .on_exit(Message::Hover(HoverId::RecentFile(i), false)),
            );
        }
    }

    container(col)
        .width(Length::Fill)
        .height(Length::FillPortion(1))
        .into()
}

/// "最近的对话"卡：语义同 `home_recent_files_card`。裁剪掉"进行中/已验收
/// vN"状态字(D4)，只显示"标题 · 项目名 · agent · 相对时间"。
fn home_recent_conversations_card(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![home_panel_head(
        icons::IconKind::MessageSquare,
        "最近的对话"
    )]
    .spacing(8);

    if !app.home_recents_loaded {
        col = col.push(
            text("加载中…")
                .size(theme::homespace_font::body())
                .color(theme::homespace_color::dim()),
        );
    } else if app.home_recent_conversations.is_empty() {
        col = col.push(
            text("暂无对话记录")
                .size(theme::homespace_font::body())
                .color(theme::homespace_color::dim()),
        );
    } else {
        for (i, c) in app.home_recent_conversations.iter().enumerate() {
            let sub = format!(
                "{} · {} · {}",
                c.project_name,
                c.meta.agent.label(),
                relative_time_text(c.meta.modified_ms, now_ms)
            );
            let hovered = app.hover_progress(HoverId::RecentConversation(i)) > 0.0;
            col = col.push(
                MouseArea::new(
                    container(
                        column![
                            lh(text(c.meta.title.clone())
                                .size(theme::homespace_font::body())
                                .color(theme::homespace_color::cream())),
                            lh(text(sub)
                                .size(theme::homespace_font::caption_sm())
                                .color(theme::homespace_color::dim())),
                        ]
                        .spacing(4),
                    )
                    .padding(10)
                    .width(Length::Fill)
                    .style(move |_t: &iced_widget::Theme| {
                        byteui::interaction::cards::container_card(
                            false,
                            hovered,
                            byteui::theme::color::current().card,
                        )
                    }),
                )
                .on_enter(Message::Hover(HoverId::RecentConversation(i), true))
                .on_exit(Message::Hover(HoverId::RecentConversation(i), false)),
            );
        }
    }

    container(col)
        .width(Length::Fill)
        .height(Length::FillPortion(1))
        .into()
}

/// D4 纯 IO 内核：对给定项目列表分别取"最近改动的文件"(git 改动/未跟踪 +
/// fs mtime)与"最近的对话"(三个 agent 来源已聚合、按 mtime 倒序)，跨项目
/// 合并后各自按时间倒序，取前 4 条 / 前 3 条(对齐 Figma 卡片行数)。
///
/// 必须在 `spawn_blocking` 里跑，不能在 UI 线程直呼——内部既有阻塞 git
/// 子进程调用，也有阻塞文件系统调用。签名固定(`&[ProjectInfo]` 输入，两个
/// `Vec` 输出)方便 headless 单测：不需要 daemon 连接或 winit `EventLoopProxy`。
/// `pub(crate)`——`workspace.rs` 的 `Message::TopBarHome` 处理器在
/// `spawn_blocking` 闭包里直接调用它。
pub(crate) fn load_home_recents(
    projects: &[ProjectInfo],
) -> (Vec<HomeRecentFile>, Vec<HomeRecentConversation>) {
    let mut files: Vec<HomeRecentFile> = Vec::new();
    let mut convs: Vec<HomeRecentConversation> = Vec::new();
    for p in projects {
        let cwd = PathBuf::from(&p.path);
        if let Some(repo) = delivery::repo_root(&cwd) {
            for (path, _status) in delivery::file_statuses(&repo) {
                let Ok(meta) = std::fs::metadata(&path) else {
                    continue; // 路径已在磁盘消失(用户手动删了),静默跳过(spec §4)
                };
                let modified_ms = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                files.push(HomeRecentFile {
                    path,
                    project_name: p.name.clone(),
                    modified_ms,
                });
            }
        }
        for meta in conversation::list_all_conversations(&cwd) {
            convs.push(HomeRecentConversation {
                project_name: p.name.clone(),
                meta,
            });
        }
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.modified_ms));
    files.truncate(4);
    convs.sort_by_key(|c| std::cmp::Reverse(c.meta.modified_ms));
    convs.truncate(3);
    (files, convs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_left_view_defaults_to_project_list() {
        assert_eq!(HomeLeftView::default(), HomeLeftView::ProjectList);
    }

    #[test]
    fn home_right_view_defaults_to_browser() {
        assert_eq!(HomeRightView::default(), HomeRightView::Browser);
    }

    #[test]
    fn load_home_recents_empty_input_returns_empty_vecs() {
        let (files, convs) = load_home_recents(&[]);
        assert!(files.is_empty());
        assert!(convs.is_empty());
    }

    #[test]
    fn load_home_recents_merges_and_sorts_across_projects() {
        let proj_a = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(proj_a.path())
            .status()
            .unwrap();
        std::fs::write(proj_a.path().join("a.txt"), "changed").unwrap();

        let proj_b = tempfile::tempdir().unwrap(); // 非 git 目录,没有改动可报告

        let projects = vec![
            ProjectInfo {
                id: 1,
                path: proj_a.path().to_string_lossy().into_owned(),
                name: "proj-a".into(),
                last_active_ms: 0,
                created_ms: 0,
                updated_ms: 0,
            },
            ProjectInfo {
                id: 2,
                path: proj_b.path().to_string_lossy().into_owned(),
                name: "proj-b".into(),
                last_active_ms: 0,
                created_ms: 0,
                updated_ms: 0,
            },
        ];

        let (files, convs) = load_home_recents(&projects);
        assert_eq!(files.len(), 1, "只有项目 A(git repo)贡献一条改动文件");
        assert_eq!(files[0].project_name, "proj-a");
        assert!(files[0].path.ends_with("a.txt"));
        assert!(convs.is_empty(), "两个项目都没有可达的 agent 对话目录");
    }

    #[test]
    fn load_home_recents_truncates_files_to_top_4() {
        let proj = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(proj.path())
            .status()
            .unwrap();
        for i in 0..6 {
            std::fs::write(proj.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        let projects = vec![ProjectInfo {
            id: 1,
            path: proj.path().to_string_lossy().into_owned(),
            name: "proj".into(),
            last_active_ms: 0,
            created_ms: 0,
            updated_ms: 0,
        }];
        let (files, _convs) = load_home_recents(&projects);
        assert_eq!(
            files.len(),
            4,
            "跨项目合并后只取前 4 条(对齐 Figma 卡片行数)"
        );
    }
}
