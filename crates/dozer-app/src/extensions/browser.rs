//! 浏览器面板扩展(阶段 1 扩展化重构第二个试点)。自己的 `Message`/
//! `State`/`update`/`view`,内核只认一个包装变体 `Message::Browser(..)`
//! 做转发,不知道自己被包在哪个外层类型里。收藏夹(全局/本项目两级书签)
//! 是浏览器域专属功能,状态/消息/UI 整体并入这个模块。
//!
//! `Tabs` 是浏览器专用的精简 tab/地址栏状态机,只认 URL——不复用
//! `crate::preview::PreviewPane`(文件预览专用,那边还有 `TabKind::File`/
//! `Acceptance`、`open_path`/`is_editable_extension`/`flyfish_url` 等浏览器
//! 用不到的逻辑),两者各自维护、互不知情。

use crate::app::tab_divider;
use crate::chrome::tab_widget::{PanelTabArgs, panel_tab};
use crate::preview::WebviewSpec;
use crate::theme;
use crate::workspace::{lh, split_portions};
use byteui::interaction::icons;
use dozer_model::{BookmarkInfo, BookmarkScope};
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use iced_widget::core::{Border, Element, Length, Rectangle};
use iced_widget::{MouseArea, button, column, container, row, text};

dozer_core::scope!(LOG, panel, "web");

/// 浏览器面板的 i18n owner(资源随 `dozer-panel-browser` 库)。
pub(crate) use dozer_panel_browser::WEB_OWNER;
// `BrowserTab` 目前宿主无直接引用,保留出口以维持原 `browser::BrowserTab` 路径。
#[allow(unused_imports)]
pub use dozer_panel_browser::BrowserTab;

/// 库产出的纯数据 webview 清单项 → 宿主的 `WebviewSpec`。其余字段的取值与搬迁前
/// `Tabs::desired_webviews` 里逐项相同:浏览器 webview 不参与预览资源 reserve、不走
/// Surface、没有隐藏预创建所以不需要离屏停放。
impl From<dozer_panel_browser::adapter::BrowserWebviewSpec> for WebviewSpec {
    fn from(s: dozer_panel_browser::adapter::BrowserWebviewSpec) -> Self {
        WebviewSpec {
            id: s.id,
            url: s.url,
            visible: s.visible,
            editor_binding: None,
            reserve: None,
            loading_generation: None,
            surface: None,
            park_offscreen: false,
            display_i18n: None,
        }
    }
}

/// 按当前 locale 取一条本面板文案。
fn t(key: &str, fallback: &str, locale: &str) -> String {
    crate::i18n::panel_text(WEB_OWNER, key, fallback, locale)
}

/// 地址栏稳定的 iced widget id:`view()` 里 `.id()` 挂给真正的
/// `text_input`,`CaptureAddrFocus` 每帧靠它在 widget 树里认出这一个(同
/// `extensions::files::search_field_id` 的既有手法)。
pub fn addr_field_id() -> Id {
    Id::new("browser-addr-box")
}

static ADDR_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走并复位(消费式)地址栏上一帧是否持有 iced 内部真实焦点。`main.rs`
/// 渲染循环每帧跑完 `CaptureAddrFocus` 后立刻调用本函数,把结果塞进当前
/// `Workspace`(`State::set_addr_focused`)——`static` 只是临时桥接(同
/// `extensions::files::take_search_focused` 的既有手法,含消费式复位避免
/// 地址栏不可见的帧卡死上一次 `true` 永久堵死终端键盘转发的理由)。
pub fn take_addr_focused() -> bool {
    std::mem::replace(&mut *ADDR_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍,把 `addr_field_id()` 命中的
/// `text_input` 当前是否持有 iced 焦点写进 `ADDR_FOCUSED`。`traverse`
/// **必须**调用传入的 `operate` 闭包才会继续递归子节点——地址栏嵌在
/// `row!`/`container!` 里,空 `traverse` 会导致 `Row`/`Column` 的
/// `operate()` 直接跳过子节点,`focusable()` 永远不会被触达(见
/// `extensions::files::CaptureSearchFocus` 修复过的同款 Critical bug,
/// commit `b8281cd`,这次从一开始就不能再犯)。
pub struct CaptureAddrFocus;
impl Operation<()> for CaptureAddrFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&addr_field_id()) {
            *ADDR_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

/// 状态机 / 消息 / `update` / 收藏夹逻辑已搬进共享库,这里重新导出,`app/` 里的调用点不动。
pub use dozer_panel_browser::{
    Message, NAV_BACK_KEY, NAV_FORWARD_KEY, NAV_REFRESH_KEY, NEW_TAB_KEY, NavAction,
    STAR_HOVER_KEY, State, bookmark_status, request_bookmarks_refresh, update,
};

/// 地址栏后退/前进/刷新导航按钮。复用统一 icon 按钮规范
/// (`icon_button_entry`),静止 DIM、hover 平滑过渡到 GOLD,与同行星标/
/// 收藏夹按钮观感一致。点击发 `Message::Nav(action)`,真正的 webview
/// 历史导航/刷新由 main.rs `dispatch` 拦截执行。
fn nav_button<'a>(
    state: &'a State,
    action: NavAction,
    locale: &str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (kind, tooltip, key) = match action {
        NavAction::Back => (
            icons::IconKind::CircleArrowLeft,
            t("nav.back", "Back", locale),
            NAV_BACK_KEY,
        ),
        NavAction::Forward => (
            icons::IconKind::CircleArrowRight,
            t("nav.forward", "Forward", locale),
            NAV_FORWARD_KEY,
        ),
        NavAction::Refresh => (
            icons::IconKind::RotateCw,
            t("nav.refresh", "Refresh", locale),
            NAV_REFRESH_KEY,
        ),
    };
    icons::icon_button_entry(
        kind,
        byteui::theme::icon_size::row(),
        false,
        false,
        state.nav_hover(action),
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::Nav(action),
        move |hovered| Message::Hover(key, false, hovered),
        tooltip,
    )
}

/// 地址栏星标:当前 URL 在全局/本项目任一边已收藏则 GOLD 实心,否则
/// DIM;当前无 tab 时禁用(浏览器 tab 恒为网页,不需要像旧版
/// `current_browser_url` 那样再判一次 `TabKind`)。`project_id` 必须传
/// 真实值,不能传 `None` 占位——否则"只在本项目收藏、没加全局收藏"的
/// 网址会被误判成未收藏(`bookmark_status` 的 `project` 字段只在传了
/// `Some(pid)` 时才会去匹配)。
fn star_button<'a>(
    state: &'a State,
    project_id: Option<i64>,
    locale: &str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let url = state
        .tabs
        .tabs()
        .get(state.tabs.active_idx())
        .map(|t| t.url.clone());
    let starred = url
        .as_ref()
        .map(|u| bookmark_status(&state.bookmarks, u, project_id).is_bookmarked())
        .unwrap_or(false);
    // 统一 icon 按钮规范:未收藏静止 DIM、hover 过渡到 GOLD;已收藏恒金
    // (active=true)。hover 动画走浏览器自己的 `State` 进度机(哨兵键)。
    icons::icon_button_entry(
        icons::IconKind::Star,
        byteui::theme::icon_size::row(),
        starred,
        false,
        state.star_hover(),
        false,
        byteui::theme::geometry::tab_button_size(),
        url.is_some(),
        Message::StarClick,
        |hovered| Message::Hover(STAR_HOVER_KEY, false, hovered),
        t("star.tooltip", "Bookmark", locale),
    )
}

/// 一组收藏条目:文件夹图标 + 标题(点击新开 tab)+ `×` 删除按钮,风格照抄
/// tab 关闭按钮。条目在文件夹标题下缩进一级,呼应收藏夹"文件夹树"视觉。
fn bookmark_group<'a>(
    title: String,
    items: &[&'a BookmarkInfo],
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let header = row![
        icons::view(
            icons::IconKind::Folder,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().dim
        ),
        lh(text(title)
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().dim)),
    ]
    .spacing(4)
    .align_y(iced_widget::core::Alignment::Center);
    let mut col = column![header].spacing(2);
    for b in items {
        let open = button(lh(text(b.title.clone())
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream)))
        .on_press(Message::OpenUrl(b.url.clone()))
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: byteui::theme::color::current().cream,
            ..button::Style::default()
        });
        let remove = button(lh(text("×")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim)))
        .on_press(Message::BookmarkRemove(b.id))
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: byteui::theme::color::current().dim,
            ..button::Style::default()
        });
        col = col.push(
            row![
                iced_widget::Space::new().width(Length::Fixed(16.0)),
                row![open, remove]
                    .spacing(4)
                    .align_y(iced_widget::core::Alignment::Center),
            ]
            .width(Length::Fill),
        );
    }
    col.into()
}

/// 收藏夹侧栏:分"全局收藏"/"本项目收藏"两组文件夹分组,都为空时显示占位
/// 文案。`width` 由调用方按配对布局里侧栏那一份宽度的 flex 权重传入,侧栏
/// 背景用列表侧一致的 `byteui::theme::color::current().bg`(结构性常驻侧栏,不再是盖在下方的
/// 浮层卡片)。
fn bookmarks_panel<'a>(
    state: &'a State,
    project_id: Option<i64>,
    width: Length,
    locale: &str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let global: Vec<&BookmarkInfo> = state
        .bookmarks
        .iter()
        .filter(|b| b.scope == BookmarkScope::Global)
        .collect();
    let project: Vec<&BookmarkInfo> = state
        .bookmarks
        .iter()
        .filter(|b| b.scope == BookmarkScope::Project && b.project_id == project_id)
        .collect();

    let both_empty = global.is_empty() && project.is_empty();
    let mut col = column![].spacing(6);
    col = col.push(bookmark_group(
        t("sidebar.global_group", "Global bookmarks", locale),
        &global,
    ));
    if project_id.is_some() {
        col = col.push(bookmark_group(
            t("sidebar.project_group", "Project bookmarks", locale),
            &project,
        ));
    }
    if both_empty {
        col = col.push(lh(text(t("sidebar.empty", "No bookmarks yet", locale))
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().dim)));
    }

    container(col)
        .padding(6)
        .width(width)
        .height(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().bg.into()),
            ..container::Style::default()
        })
        .into()
}

/// 浏览器面板:表头 + (可选 tab 栏)+ 地址栏(与星标/收藏夹按钮)+ (可选
/// 星标菜单/收藏夹面板)+ (可选错误文案)+ 激活 tab 的网页预览。
/// `project_id` 用于星标收藏态判定(全局/本项目两档)。
pub fn view<'a>(
    state: &'a State,
    project_id: Option<i64>,
    bookmarks_split: f32,
    width: Length,
    outer: Border,
    mirror: bool,
    locale: &str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::browser_pane();

    let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> = state
        .tabs
        .tabs()
        .iter()
        .enumerate()
        .map(|(idx, tab)| {
            let active = idx == state.tabs.active_idx();
            let title_hover_t = state.hover_progress(idx, false);
            let close_hover_t = state.hover_progress(idx, true);
            let tab = panel_tab(PanelTabArgs {
                title: tab.title.clone(),
                active,
                hover_t: title_hover_t,
                close_hover_t,
                prefix: None,
                suffix: None,
                on_select: Message::SelectTab(idx),
                on_close: Message::CloseTab(idx),
                show_tooltip: state.hover_tooltip_ready(idx),
                title_hover: move |h| Message::Hover(idx, false, h),
                close_hover: move |h| Message::Hover(idx, true, h),
            });
            // 拖拽换位:按住页签(App 侧把 `tab_drag` 置位)后光标扫过哪个
            // 页签,这个 `on_move` 发出 `DragHover(idx)`,再在 `App::update`
            // 翻译成 `TabDragMove`(组校验在那里做),完成换位。
            MouseArea::new(tab)
                .on_move(move |_| Message::DragHover(idx))
                .into()
        })
        .collect();
    // "新标签页"(+)按钮紧跟最后一个页签,一起放进同一行、按内容宽度排布:
    // 它也位于页签最右端紧挨着,而不是被 `Length::Fill` 顶到 tab 栏最右缘。
    // 行整体横向溢出被 clip(页签/按钮一多会被裁出可视区,与页签同一待遇)。
    let new_tab_btn = icons::icon_button_entry(
        icons::IconKind::CirclePlus,
        byteui::theme::icon_size::row(),
        false,
        false,
        state.new_tab_hover(),
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::OpenUrl("about:blank".to_string()),
        |hovered| Message::Hover(NEW_TAB_KEY, false, hovered),
        t("tab.new", "New tab", locale),
    );
    let tabs_row = row(items
        .into_iter()
        .chain(std::iter::once(new_tab_btn))
        .collect::<Vec<_>>())
    .spacing(4)
    .align_y(iced_widget::core::Alignment::Center);
    let tab_bar = container(tabs_row).width(Length::Shrink).clip(true);

    let editing = state.addr_focused();
    // 未聚焦时地址栏回显当前激活 tab 的完整网址(不再只显示"输入网址"
    // 占位符)——聚焦时仍用 `addr_buffer`(焦点刚获得时已预填当前网址)。
    let active_url = state
        .tabs
        .tabs()
        .get(state.tabs.active_idx())
        .map(|t| t.url.as_str())
        .unwrap_or("");
    let addr_value: &str = if editing {
        state.tabs.addr_buffer()
    } else {
        active_url
    };
    let addr_placeholder = t("addr.placeholder", "Enter a URL", locale);
    let addr_input = byteui::interaction::context_menu::wrap(
        byteui::form::input_text::view(
            &addr_placeholder,
            addr_value,
            false,
            Some(addr_field_id()),
            false,
            Some(Message::AddrSubmit),
            true,
            Message::AddrInput,
        ),
        Some(Message::TextInputMenuOpen(
            dozer_panel_browser::adapter::TextInputTarget {
                id: addr_field_id(),
                secure: false,
            },
        )),
    );

    // 地址栏本体:单个带边框的容器,把"网址文字 + 星标(收藏)按钮"一起包进
    // 边框内(复用 todo 新增输入框 / `crate::search_box` 的布局模式)。不再
    // 需要外层 `MouseArea`/`AddrClick`——`text_input` 是真控件,点击命中范围内
    // 就由 iced 标准鼠标管线自己处理聚焦,不需要应用层代理点击(唯一影响:
    // 点击胶囊的 4px padding 空白处不再能进编辑态,只有点在输入框自身范围
    // 内才行,判定为可接受的小回归,见本计划 Global Constraints)。星标按钮
    // 是内层 widget,自己截获点击(弹加入/移出收藏夹菜单)。框高由按钮的
    // 方形尺寸撑起,文字垂直居中,视觉上按钮嵌在地址栏右侧。
    let content_h = byteui::theme::geometry::tab_button_size();
    let addr_box = container(
        row![
            container(addr_input)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(iced_widget::core::alignment::Vertical::Center)
                .align_x(iced_widget::core::alignment::Horizontal::Left),
            container(star_button(state, project_id, locale))
                .height(Length::Fill)
                .align_y(iced_widget::core::alignment::Vertical::Center),
        ]
        .width(Length::Fill)
        .height(Length::Fixed(content_h))
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(content_h + 4.0))
    .padding([2, 8])
    .style(move |_t: &iced_widget::Theme| container::Style {
        background: Some(byteui::theme::color::current().term_bg.into()),
        border: Border {
            color: if editing {
                byteui::theme::color::current().gold
            } else {
                byteui::theme::color::current().border
            },
            width: 1.0,
            radius: 2.0.into(),
        },
        ..container::Style::default()
    });

    // 后退/前进/刷新三颗导航按钮紧凑成组(组内间距 0,比下方整体 4 更紧),
    // 再与地址栏/收藏等拉开到 4,突出"导航簇"的视觉聚合。
    let nav_buttons = row![
        nav_button(state, NavAction::Back, locale),
        nav_button(state, NavAction::Forward, locale),
        nav_button(state, NavAction::Refresh, locale),
    ]
    .spacing(0)
    .align_y(iced_widget::core::alignment::Vertical::Center);

    // 地址栏整行作为组件包一层,给它额外的下边距(用容器底部 padding 充当
    // margin),与下方内容拉出呼吸感;垂直 padding 已在上面的 `addr_box`
    // 内压低来瘦身高,这里只贡献下边距、不再加别的内边距。
    let addr_row = container(
        container(
            row![
                nav_buttons,
                addr_box,
                bookmarks_toggle_button(state, locale),
            ]
            .spacing(4)
            .align_y(iced_widget::core::alignment::Vertical::Center),
        )
        .width(Length::Fill),
    )
    .padding(iced_widget::core::padding::bottom(8.0));

    let mut content = column![tab_bar, tab_divider(), addr_row].spacing(region.gap);

    if let Some(err) = &state.error {
        content = content.push(lh(text(format!(
            "⚠ {}",
            crate::i18n::render_localized(err, locale)
        ))
        .size(byteui::theme::font::body())
        .color(byteui::theme::color::current().red)));
    }

    let body: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if state.tabs.tabs().is_empty() {
            container(lh(text(t(
                "empty.no_page",
                "No page yet — type a URL in the address bar",
                locale,
            ))
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().dim)))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            // 真实网页由 wry webview 叠加渲染,这里只需要一块透明占位
            // (不能有不透明背景,否则会盖住 webview)。
            iced_widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };

    content = content.push(if state.bookmarks_open() {
        let bg = region
            .background
            .unwrap_or(byteui::theme::color::current().bg);
        let (list_portion, content_portion) = split_portions(1.0 - bookmarks_split);
        let content_box = container(body).width(Length::FillPortion(content_portion));
        let bookmarks_box =
            bookmarks_panel(state, project_id, Length::FillPortion(list_portion), locale);
        let divider = crate::app::divider_bar(
            crate::app::Divider::BrowserBookmarksSplit,
            bg,
            bg,
            Message::ColumnDragStart,
        );
        if mirror {
            row![bookmarks_box, divider, content_box]
        } else {
            row![content_box, divider, bookmarks_box]
        }
        .height(Length::Fill)
        .into()
    } else {
        body
    });

    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

/// 地址栏"收藏夹"展开按钮:切换收藏夹侧栏。图标用 `FolderBookmark`,颜色
/// 恒定(不像星标那样带收藏状态)。走统一 icon 按钮规范(DIM→GOLD hover,
/// 无选中态),hover 动画走浏览器自己的 `State` 进度机(哨兵键)。
fn bookmarks_toggle_button<'a>(
    state: &'a State,
    locale: &str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    icons::icon_button_entry(
        icons::IconKind::FolderBookmark,
        byteui::theme::icon_size::row(),
        false,
        false,
        state.bookmark_hover(),
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::BookmarksToggle,
        |hovered| Message::Hover(STAR_HOVER_KEY, true, hovered),
        t("bookmarks.tooltip", "Bookmarks", locale),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addr_field_id_is_stable_across_calls() {
        assert_eq!(addr_field_id(), addr_field_id());
    }
}
