// crates/dozer-app/src/homespace.rs
//! 首页落地页(`AppPage::Home`,点顶栏 Dozer 页签进入)专属代码:类型定义与
//! 全部视图构建函数。`App` struct 字段声明/`Message` 枚举/`App::update` 的
//! 消息处理逻辑仍留在 `workspace.rs`(单一数据源+集中调度),拆分边界比照
//! `preview.rs`/`conversation.rs` 的既有先例——本文件不持有 `App`/`Workspace`
//! 的 `impl` 块。见 `docs/superpowers/specs/2026-08-08-home-page-4col-layout-design.md`。

use crate::app::{App, HoverId, Message, PaneCorner, zone_pane_border};
use crate::chrome::rail::RailButton;
use crate::conversation::ConversationMeta;
use crate::delivery;
use crate::extensions::browser;
use crate::theme;
use crate::workspace::{agent_dot_color, agent_icon, lh, relative_time_text};
use byteui::interaction::icons;
use dozer_core::protocol::ProjectInfo;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use iced_widget::core::{Border, Color, Element, Length, Padding, Rectangle};
use iced_widget::{Scrollable, button, column, container, row, scrollable, text};
use std::path::PathBuf;

/// 首页项目列表搜索框(iced 原生 `text_input`)的 `widget::Id`:main.rs 每帧
/// `interface.operate` 用 `CaptureHomeSearchFocus` 问真实焦点态。
pub fn home_search_field_id() -> Id {
    Id::new("home-project-search-box")
}

static HOME_SEARCH_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走并复位(消费式),同 `extensions::files::take_search_focused` 的
/// 消费式复位手法,避免搜索框不可见的帧卡死上一次 `true` 永久堵死终端
/// 键盘转发。
pub fn take_home_search_focused() -> bool {
    std::mem::replace(&mut *HOME_SEARCH_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍。`traverse` 必须调用传入闭包(见
/// [[dozer-operation-traverse-noop-bug]])。
pub struct CaptureHomeSearchFocus;
impl Operation<()> for CaptureHomeSearchFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&home_search_field_id()) {
            *HOME_SEARCH_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

/// 首页左栏当前显示哪个 pane。语义、命名对齐工作区面板(原 `HomeLeftView`/
/// `HomeRightView` 各自独立,不与工作区共用),但这是首页专属枚举——首页
/// 导航态不持续化(每次 `Message::TopBarHome`
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
/// `project_id` 供点击卡片打开所属项目(`ProjectSelect(project_id)`)，与项目
/// 列表面板同款跳转。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HomeRecentFile {
    pub(crate) project_id: i64,
    pub(crate) path: PathBuf,
    pub(crate) project_name: String,
    pub(crate) modified_ms: u64,
}

/// H0"最近的对话"卡一行；语义同上，`project_id` 同样供 `ProjectSelect`。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HomeRecentConversation {
    pub(crate) project_id: i64,
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
    // 「＋新增项目」按钮已移到"项目列表"面板的 panel footbar(见
    // `home_project_list_view`),这里只保留全局状态 footbar。
    let footbar = row![crate::extensions::footbar::view(footbar_state).map(Message::Footbar),];
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
            footbar,
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
        .width(Length::Fixed(byteui::theme::geometry::divider_width()))
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
        icons::icon_button_entry(
            icons::IconKind::LayoutList,
            byteui::theme::icon_size::rail(),
            app.home_left_view == HomeLeftView::ProjectList,
            false,
            app.hover_progress(HoverId::Rail(RailButton::HomeProjectList)),
            true,
            byteui::theme::geometry::rail_button_size(),
            true,
            Message::HomeLeftIconSelect(HomeLeftView::ProjectList),
            |hovered| Message::Hover(HoverId::Rail(RailButton::HomeProjectList), hovered),
            "项目列表",
        ),
        icons::icon_button_entry(
            icons::IconKind::History,
            byteui::theme::icon_size::rail(),
            app.home_left_view == HomeLeftView::Recents,
            false,
            app.hover_progress(HoverId::Rail(RailButton::HomeRecents)),
            true,
            byteui::theme::geometry::rail_button_size(),
            true,
            Message::HomeLeftIconSelect(HomeLeftView::Recents),
            |hovered| Message::Hover(HoverId::Rail(RailButton::HomeRecents), hovered),
            "最近记录",
        ),
    ]
    .spacing(region.gap)
    .padding(region.padding);

    container(content)
        .width(Length::Fixed(byteui::theme::geometry::icon_rail_width()))
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
    let content = column![icons::icon_button_entry(
        icons::IconKind::Globe,
        byteui::theme::icon_size::rail(),
        app.home_right_view == HomeRightView::Browser,
        false,
        app.hover_progress(HoverId::Rail(RailButton::HomeBrowser)),
        true,
        byteui::theme::geometry::rail_button_size(),
        true,
        Message::HomeRightIconSelect(HomeRightView::Browser),
        |hovered| Message::Hover(HoverId::Rail(RailButton::HomeBrowser), hovered),
        "浏览器",
    ),]
    .spacing(region.gap)
    .padding(region.padding);

    container(content)
        .width(Length::Fixed(byteui::theme::geometry::icon_rail_width()))
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
        .width(Length::Fixed(byteui::theme::geometry::h0_sidebar_width()))
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
        byteui::theme::geometry::default_split_ratio(),
        Length::Fill,
        zone_pane_border(zone, PaneCorner::All),
        false,
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
/// 最近项目卡(取 `app.recent_projects` 按 `updated_ms` 排序后分页,首屏
/// 5 条、"更多..."逐页展开)。"＋新增项目"按钮放在本面板的 panel footbar
/// 底部(`home_new_project_button`),不在页面全局 footbar。不画品牌行——顶栏
/// 本身已有 `dozer_home_tab` 品牌页签,这里重复画属于视觉冗余。
/// 通用面板标题组件:图标 + 标题(金色 `subtitle` 字号),标题底部一条 1px
/// panel 标题与图标用的强调色(暖金,对应 `ColorTokens::tab_active_border`,
/// 深色版 `#dcc9a3`)——刻意区别于甲方动作专属的 GOLD(`#F2D94E`):标题是
/// 装饰性的「section 标」,不是可点的甲方动作。现读 `color::current()`
/// 而非编译期 `const`,配色方案运行时切换(设置弹窗)才能立即生效——
/// 曾经写死成 `const Color`,是 2026-09-15 用户实测发现"切浅色后首页
/// 标题强调色纹丝不动"的同批 bug 之一。
fn panel_head_accent() -> Color {
    byteui::theme::color::current().tab_active_border
}

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
    // 高度钉死到跟预览/终端 tab 栏一样高(`tab_button_size`,tab 页签里 ×
    // 关闭按钮的固定命中区尺寸,是那条 tab 栏实际渲染高度的下限)——此前只
    // 靠图标/文字自身撑开,比 tab 栏矮了一截,导致 Files/Todo/Project/Git/
    // SSH 等面板头部跟右侧 tab 栏高度对不齐(2026-09 用户反馈)。
    let head_row = row![
        icons::view(icon, byteui::theme::icon_size::row(), panel_head_accent(),),
        text(title)
            .size(theme::homespace_font::subtitle())
            .color(panel_head_accent()),
    ]
    .spacing(8)
    .height(Length::Fixed(byteui::theme::geometry::tab_button_size()))
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
    // 标题行到分割线的间距从 8 收到 4(验收反馈:左栏 Todo/Files/Project/
    // Git/SSH 及 Agent 面板的头部分割线比 Agent 终端 tab 栏的分割线低了近
    // 9px,几个并排面板的分割线高度对不齐;终端那条已确认是最合适的
    // 基准,这里收紧靠拢它)。
    .spacing(4)
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

/// 子栏目标题,样式参考 workspace Project 面板「文件存储 / 项目文档」
/// (`extensions::project` 的用量行 / `links_section` 头部):CircleSmall
/// 圆点 + cream `label` 字号,无下划线——层级低于 `home_panel_head`
/// 的面板标题(暖金 accent + subtitle + 1px 分割线)。跟 `home_panel_head`
/// 一样对 `Message` 泛型(2026-08-23 起 `pub(crate)`,供 `extensions::usage`
/// 复用,不是只有首页用得上——纯展示、不产生消息,泛型化零风险)。
pub(crate) fn home_section_head<'a, Message>(
    title: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>
where
    Message: 'a,
{
    row![
        icons::view(
            icons::IconKind::CircleSmall,
            byteui::theme::icon_size::row(),
            theme::homespace_color::cream(),
        ),
        lh(text(title)
            .size(theme::homespace_font::label())
            .color(theme::homespace_color::cream())),
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center)
    .into()
}

/// 首页"项目列表" pane 一页显示的条数。首屏 1 页,点"更多..."页数递增、
/// 显示 `pages * PROJECT_PAGE_SIZE` 条。
const PROJECT_PAGE_SIZE: usize = 5;

/// 首页"项目列表" pane 的核心分页切片:把 `projects` 先按 `updated_ms`
/// (最后更新时间,git 感知)倒序排好,再取前 `pages * PROJECT_PAGE_SIZE` 条。
/// 抽成纯函数是为了 headless 单测;视图只负责把返回值画出来。排序字段刻意用
/// `updated_ms` 而非 daemon `list_projects()` 默认的 `last_active_ms`,对应
/// 产品需求"项目按最后更新时间排序"。
fn paginate_recent_projects(projects: &[ProjectInfo], pages: usize) -> Vec<ProjectInfo> {
    let mut sorted = projects.to_vec();
    sorted.sort_by_key(|p| std::cmp::Reverse(p.updated_ms));
    sorted.truncate(pages.saturating_mul(PROJECT_PAGE_SIZE));
    sorted
}

/// 首页项目列表搜索框已提交的过滤词 → 匹配的项目子集(名称大小写不敏感
/// 包含匹配)。`query` 为空串时不过滤,原样返回全量——抽成纯函数同
/// `paginate_recent_projects` 一样是为了 headless 单测。
fn filter_projects_by_search(projects: &[ProjectInfo], query: &str) -> Vec<ProjectInfo> {
    if query.is_empty() {
        return projects.to_vec();
    }
    let needle = query.to_lowercase();
    projects
        .iter()
        .filter(|p| p.name.to_lowercase().contains(&needle))
        .cloned()
        .collect()
}

/// `app.recent_projects` 为空时画"还没有项目"兜底文案,不崩(spec §4)。
fn home_project_list_view(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![].spacing(16);

    col = col.push(home_panel_head(icons::IconKind::LayoutList, "项目"));

    // 搜索框:原为手写实现(不经共享组件),现收敛到
    // `byteui::form::search_box`——所有面板搜索框统一样式的基准就是这份
    // 首页项目列表搜索框的设计(需求:其它面板搜索框都要跟它一致)。
    let editing = app.home_project_search_focused();
    let highlight = editing || !app.home_project_search.is_empty();
    let search_box = byteui::form::search_box::view(
        "搜索项目…",
        &app.home_project_search_draft,
        Some(home_search_field_id()),
        highlight,
        Message::HomeProjectSearchInput,
        Message::HomeProjectSearchSubmit,
        app.hover_progress(HoverId::HomeProjectSearchSubmit),
        |hovered| Message::Hover(HoverId::HomeProjectSearchSubmit, hovered),
    );
    col = col.push(byteui::interaction::context_menu::wrap(
        search_box,
        Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
            id: home_search_field_id(),
            secure: false,
        })),
    ));

    // 按已提交的搜索词过滤,过滤后再分页——"更多..."按钮/页数据此按过滤后
    // 的结果集算,不是全量 `recent_projects`。
    let filtered = filter_projects_by_search(&app.recent_projects, &app.home_project_search);

    // 面板底部 panel footbar:放「＋新增项目」按钮(甲方动作),钉在面板最下方。
    // 空列表 / 搜索无结果时也照样显示,空态下用垂直 filler 把按钮顶到面板底。
    // 与 workspace 项目面板 `project_footer_bar` 同款结构:1px 顶部分隔线
    // (`border`)+ `padding([6, 8])` 容器;不再给整个 footbar 套四边 1px 边框
    // (那是 homespace 自己偏离既有 footer-bar 规范的画法)。
    let top_line = container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(theme::homespace_color::border().into()),
            ..iced_widget::container::Style::default()
        });
    let panel_footbar: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        container(column![top_line, home_new_project_button()].spacing(4))
            .width(Length::Fill)
            .padding([6, 8])
            .style(|_t: &iced_widget::Theme| container::Style {
                background: None,
                ..container::Style::default()
            })
            .into();

    if app.recent_projects.is_empty() {
        col = col.push(
            text("还没有项目")
                .size(theme::homespace_font::body())
                .color(theme::homespace_color::dim()),
        );
        col = col.push(iced_widget::Space::new().height(Length::Fill));
    } else if filtered.is_empty() {
        col = col.push(
            text("没有匹配的项目")
                .size(theme::homespace_font::body())
                .color(theme::homespace_color::dim()),
        );
        col = col.push(iced_widget::Space::new().height(Length::Fill));
    } else {
        let visible = paginate_recent_projects(&filtered, app.home_project_pages);
        let more_remain = visible.len() < filtered.len();

        let mut list = column![].spacing(8);
        for p in &visible {
            let card = button(
                column![
                    lh(text(p.name.clone())
                        .size(theme::homespace_font::body())
                        .color(theme::homespace_color::cream())),
                    lh(text(format!(
                        "更新于{}",
                        relative_time_text(p.updated_ms, now_ms)
                    ))
                    .size(theme::homespace_font::caption_sm())
                    .color(theme::homespace_color::dim())),
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

        // 还有更多项目时,在最后一张卡下面左右居中的"更多..."翻页图标按钮
        // (Lucide ellipsis,无外边框/背景);点它再展开下一页(见
        // `Message::HomeMoreProjects`)。全部显示完就消失。
        if more_remain {
            let more_button = icons::icon_button_entry(
                icons::IconKind::Ellipsis,
                byteui::theme::icon_size::row(),
                false,
                false,
                app.hover_progress(HoverId::HomeProjectMore),
                false,
                byteui::theme::geometry::tab_button_size(),
                true,
                Message::HomeMoreProjects,
                |hovered| Message::Hover(HoverId::HomeProjectMore, hovered),
                "更多",
            );
            list = list.push(
                container(more_button)
                    .width(Length::Fill)
                    .align_x(iced_widget::core::Alignment::Center),
            );
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

    col = col.push(panel_footbar);

    // 与外边框保持标准内边距:外层 `zone_box` 只留 1px 圆角裁切余量,内容
    // 若直接贴边会顶到圆角边框,因此这里补一层标准面板内距(与 project_pane
    // 的 8 / 卡片的 10 同量级),让"项目"标题、搜索框、卡片四周都不顶边框。
    container(col)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(12)
        .into()
}

/// 底部 footbar 里的「＋新增项目」按钮(甲方动作)。样式对齐全局统一按钮
/// 规范(2026-09-15 起,见 `dialog::action_button_border_color` 文档):
/// 面板底色实底(`card_bg`)+ 描边静止态 `border`、悬浮/按下态变
/// `gold`(此前描边固定不响应 hover)+ 灰字 `dim`(此前用奶油字
/// `cream`,普通按钮该用灰字,危险按钮才用红字)。字号用 `label`(13px,
/// 同 `project_footer_bar` 的 `byteui::theme::font::label()`)。宽度改为
/// footbar 行宽的一半、靠右对齐(此前 `width(Fill)` 撑满整行,视觉上比
/// workspace 项目面板的功能按钮粗重;之后又短暂改成居中,2026-09-15 定为
/// 右对齐)。
fn home_new_project_button()
-> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 按钮内容用 `container` 撑满 + `align_x(Center)` 居中标题文字——`button`
    // 的 `layout::padded` 不会把 Shrink 宽度的内容自动居中,只贴左上角(同
    // `extensions::project::footer_button_label` 的既有处理)。
    let label = container(
        text("＋新建项目")
            .size(theme::homespace_font::label())
            .color(theme::homespace_color::dim()),
    )
    .width(Length::Fill)
    .align_x(iced_widget::core::alignment::Horizontal::Center);

    let btn = button(label)
        .on_press(Message::ProjectTabPickFolder)
        .padding([6, 8])
        .style(|_t: &iced_widget::Theme, s: button::Status| button::Style {
            background: Some(theme::homespace_color::card_bg().into()),
            border: Border {
                color: match s {
                    button::Status::Hovered | button::Status::Pressed => {
                        theme::homespace_color::gold()
                    }
                    _ => theme::homespace_color::border(),
                },
                width: 1.0,
                radius: 4.0.into(),
            },
            text_color: theme::homespace_color::dim(),
            ..button::Style::default()
        });

    row![
        iced_widget::Space::new().width(Length::FillPortion(1)),
        btn.width(Length::FillPortion(1)),
    ]
    .into()
}

/// 首页左栏"Recents" pane(`HomeLeftView::Recents`):面板标题「最近」,
/// 下挂「最近的对话」「最近的文件」两个子栏目(子栏目标题样式参考
/// workspace Project 面板「文件存储 / 项目文档」)。左栏宽度固定较窄
/// (`h0_sidebar_width()`),两子栏目上下堆叠(各自外层容器把 `height`
/// 设成 `FillPortion(1)` 平分剩余高度,见 `home_recent_files_card`/
/// `home_recent_conversations_card`)。
fn home_recents_view(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 与 project list 面板同款:外层 `zone_box` 仅留 1px 圆角裁切余量,这里
    // 再补一层标准内距,让两张卡片(含各自的标题)四周都不顶圆角边框。
    let inner = column![
        home_panel_head(icons::IconKind::History, "最近"),
        home_recent_conversations_card(app, now_ms),
        home_recent_files_card(app, now_ms)
    ]
    .spacing(16)
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
    let mut col = column![home_section_head("最近的文件")].spacing(8);

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
        for f in &app.home_recent_files {
            let filename = f
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| f.path.display().to_string());
            let row_el = row![
                icons::view(
                    icons::icon_for_file(&filename),
                    byteui::theme::icon_size::row(),
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
            // 原生 button + button_card:hover 由 iced `button::Status` 驱动,
            // 立即亮/立即灭——与项目列表面板同一套机制,避免 `container_card`
            // + `hover_progress` 指数衰减让高亮在鼠标离开后残留过久。点击在
            // 激活 tab 打开所属项目(`ProjectSelect`,见 `App::project_select`)。
            col = col.push(
                button(container(row_el).padding(10).width(Length::Fill))
                    .on_press(Message::ProjectSelect(f.project_id))
                    .width(Length::Fill)
                    .style(byteui::interaction::cards::button_card(
                        false,
                        byteui::theme::color::current().card,
                    )),
            );
        }
    }

    container(col)
        .width(Length::Fill)
        .height(Length::FillPortion(1))
        .into()
}

/// "最近的对话"卡：语义同 `home_recent_files_card`。按需求改为三行,文字
/// 整体相对 agent 图标缩进(`row![icon, column![三行]]`,而不是图标单独占
/// 一行的 `column![row![icon, 名称], 内容, 元信息]`——后者内容/元信息会
/// 跟图标左边缘对齐,不是跟名称文字对齐):
///   第一行:agent 图标(按 `agent_dot_color` 彩色) + agent 名称(奶油色)
///   第二行:对话内容(即对话标题,灰色)
///   第三行:项目名称 · 相对时间(灰色)
fn home_recent_conversations_card(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![home_section_head("最近的对话")].spacing(8);

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
        for c in &app.home_recent_conversations {
            // 卡片样式对齐会话面板 `workspace::conversation_list_pane`(需求:
            // "改为和会话面板-会话卡片一样的方式")——图标+标题一行,agent
            // 名称挪到第二行(元信息),跟时间一起缩进到标题下方。项目名称
            // 会话面板不需要(已经限定在单个项目内),这里跨项目必须保留,
            // 插进 agent 与时间之间。
            let meta = format!(
                "{} · {} · {}",
                c.meta.agent.label(),
                c.project_name,
                relative_time_text(c.meta.modified_ms, now_ms)
            );
            col = col.push(
                button(
                    container(
                        row![
                            icons::view(
                                agent_icon(c.meta.agent),
                                byteui::theme::icon_size::row(),
                                agent_dot_color(c.meta.agent),
                            ),
                            column![
                                lh(text(c.meta.title.clone())
                                    .size(theme::homespace_font::body())
                                    .color(theme::homespace_color::cream())),
                                lh(text(meta)
                                    .size(theme::homespace_font::caption_sm())
                                    .color(theme::homespace_color::dim())),
                            ]
                            .spacing(4),
                        ]
                        .spacing(8)
                        .align_y(iced_widget::core::Alignment::Center),
                    )
                    .padding(10)
                    .width(Length::Fill),
                )
                .on_press(Message::ProjectSelect(c.project_id))
                .width(Length::Fill)
                .style(byteui::interaction::cards::button_card(
                    false,
                    byteui::theme::color::current().card,
                )),
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
/// 必须在异步上下文里跑——内部既有阻塞 git 子进程调用（走 `spawn_blocking`），
/// 也有走 `dozer_client` 的异步会话查询。签名固定
/// (`&Client` + `&[ProjectInfo]` 输入，两个 `Vec` 输出)方便 headless 单测。
/// `pub(crate)`——`app.rs` 的 `top_bar_home` 在 `handle.spawn` 的 `async move`
/// 里直接 `.await` 它。
pub(crate) async fn load_home_recents(
    client: &dozer_client::Client,
    projects: &[ProjectInfo],
) -> (Vec<HomeRecentFile>, Vec<HomeRecentConversation>) {
    let projects_owned = projects.to_vec();
    let files = tokio::task::spawn_blocking(move || {
        let mut files: Vec<HomeRecentFile> = Vec::new();
        for p in &projects_owned {
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
                        project_id: p.id,
                        path,
                        project_name: p.name.clone(),
                        modified_ms,
                    });
                }
            }
        }
        files
    })
    .await
    .unwrap_or_default();
    let mut files = files;
    files.sort_by_key(|f| std::cmp::Reverse(f.modified_ms));
    files.truncate(4);

    let mut convs: Vec<HomeRecentConversation> = Vec::new();
    for p in projects {
        let cwd = PathBuf::from(&p.path);
        let cwd_str = cwd.to_string_lossy().into_owned();
        if let Ok(summaries) = client.list_conversations(&cwd_str, None, 50, 0).await {
            for s in summaries {
                convs.push(HomeRecentConversation {
                    project_id: p.id,
                    project_name: p.name.clone(),
                    meta: ConversationMeta::from_summary(&s),
                });
            }
        }
    }
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

    fn client_for_test() -> dozer_client::Client {
        dozer_client::Client::new(std::path::PathBuf::from("/tmp/dz-home-rec-test.sock"))
    }

    #[tokio::test]
    async fn load_home_recents_empty_input_returns_empty_vecs() {
        let (files, convs) = load_home_recents(&client_for_test(), &[]).await;
        assert!(files.is_empty());
        assert!(convs.is_empty());
    }

    #[tokio::test]
    async fn load_home_recents_merges_and_sorts_across_projects() {
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

        let (files, convs) = load_home_recents(&client_for_test(), &projects).await;
        assert_eq!(files.len(), 1, "只有项目 A(git repo)贡献一条改动文件");
        assert_eq!(files[0].project_name, "proj-a");
        assert_eq!(files[0].project_id, 1, "卡片点击应带回所属项目 id");
        assert!(files[0].path.ends_with("a.txt"));
        assert!(convs.is_empty(), "两个项目都没有 daemon 返回的会话");
    }

    #[tokio::test]
    async fn load_home_recents_truncates_files_to_top_4() {
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
        let (files, _convs) = load_home_recents(&client_for_test(), &projects).await;
        assert_eq!(
            files.len(),
            4,
            "跨项目合并后只取前 4 条(对齐 Figma 卡片行数)"
        );
    }

    fn project(id: i64, name: &str, updated_ms: u64) -> ProjectInfo {
        ProjectInfo {
            id,
            path: format!("/tmp/{name}"),
            name: name.into(),
            last_active_ms: 0,
            created_ms: 0,
            updated_ms,
        }
    }

    #[test]
    fn paginate_recent_projects_sorts_by_updated_ms_desc() {
        // 输入乱序 + last_active_ms 全 0,唯一排序依据是 updated_ms。
        let projects = vec![
            project(1, "old", 100),
            project(2, "newest", 300),
            project(3, "mid", 200),
        ];
        let page = paginate_recent_projects(&projects, 1);
        let names: Vec<&str> = page.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["newest", "mid", "old"]);
    }

    #[test]
    fn paginate_recent_projects_truncates_to_page_size() {
        let projects: Vec<ProjectInfo> = (1..=12)
            .map(|i| project(i, &format!("p{i}"), i as u64))
            .collect();
        assert_eq!(paginate_recent_projects(&projects, 1).len(), 5);
        assert_eq!(paginate_recent_projects(&projects, 2).len(), 10);
        // 第 3 页显示到 15 条的上限,但只有 12 个项目 → 返回全部 12 个。
        assert_eq!(paginate_recent_projects(&projects, 3).len(), 12);
        // 页数远大于所需也不越界(仍是 12)。
        assert_eq!(paginate_recent_projects(&projects, 99).len(), 12);
    }

    #[test]
    fn paginate_recent_projects_empty_input_yields_empty() {
        assert!(paginate_recent_projects(&[], 1).is_empty());
        assert!(paginate_recent_projects(&[], 5).is_empty());
    }

    #[test]
    fn filter_projects_by_search_empty_query_returns_all() {
        let projects = vec![project(1, "byteboy", 100), project(2, "dozer", 200)];
        let filtered = filter_projects_by_search(&projects, "");
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filter_projects_by_search_matches_case_insensitive_substring() {
        let projects = vec![
            project(1, "ByteBoy", 100),
            project(2, "dozer", 200),
            project(3, "anrong_fincalc", 300),
        ];
        let filtered = filter_projects_by_search(&projects, "byte");
        let names: Vec<&str> = filtered.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["ByteBoy"],
            "大小写不敏感,只匹配名称包含子串的项目"
        );
    }

    #[test]
    fn filter_projects_by_search_no_match_yields_empty() {
        let projects = vec![project(1, "byteboy", 100)];
        assert!(filter_projects_by_search(&projects, "nonexistent").is_empty());
    }
}
