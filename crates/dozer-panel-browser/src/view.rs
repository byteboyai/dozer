//! 浏览器面板视图:页签栏、地址栏行、收藏夹侧栏、空态。页签控件 / 分隔条 / 区域样式
//! 经 [`BrowserDisplay`] 由宿主注入;图标按钮直接用 byteui 的统一 `icon_button_entry`。
//! 文字全部用系统默认字体(不给地址栏等 UI 上等宽代码字体)。

use crate::adapter::{BrowserDisplay, TabView, TextInputTarget};
use crate::find::{find_bar_bottom_pad, find_bar_height_px, find_field_id};
use crate::i18n::t;
use crate::state::{
    FIND_CLOSE_KEY, FIND_NEXT_KEY, FIND_PREV_KEY, NAV_BACK_KEY, NAV_FORWARD_KEY, NAV_REFRESH_KEY,
    NEW_TAB_KEY, STAR_HOVER_KEY,
};
use crate::{Message, NavAction, State, addr_field_id, bookmark_status};
use byteui::interaction::icons;
use dozer_model::{BookmarkInfo, BookmarkScope};
use iced_widget::core::text::LineHeight;
use iced_widget::core::{Border, Element, Length};
use iced_widget::{MouseArea, button, column, container, row, text};

type El<'a> = Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>;

/// 统一行高:把一段文字的行高设为宿主给的行高倍数(Dozer 里与终端行距同源),
/// `size`/`color` 仍由调用方设置。
fn lh<'a>(
    factor: f32,
    t: iced_widget::text::Text<'a, iced_widget::Theme, iced_renderer::Renderer>,
) -> iced_widget::text::Text<'a, iced_widget::Theme, iced_renderer::Renderer> {
    t.line_height(LineHeight::Relative(factor))
}

/// 配对布局"侧栏:内容"的 `FillPortion` 权重对(与 Dozer `workspace::split_portions`
/// 同一算法:`FillPortion` 在扣掉中间固定宽的分隔条之后按权重分剩余空间)。
fn split_portions(split: f32) -> (u16, u16) {
    let list = (split * 10_000.0).round() as u16;
    let content = ((1.0 - split) * 10_000.0).round() as u16;
    (list, content)
}

/// 地址栏后退/前进/刷新导航按钮。复用统一 icon 按钮规范
/// (`icon_button_entry`),静止 DIM、hover 平滑过渡到 GOLD,与同行星标/
/// 收藏夹按钮观感一致。点击发 `Message::Nav(action)`,真正的 webview
/// 历史导航/刷新由宿主拦截执行。
fn nav_button<'a>(state: &'a State, action: NavAction, locale: &str) -> El<'a> {
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

/// 查找条一颗图标按钮(上一个 / 下一个 / 关闭),样式与导航按钮同一个统一图标按钮。
fn find_button<'a>(
    state: &'a State,
    kind: icons::IconKind,
    key: usize,
    on_press: Message,
    tooltip: String,
) -> El<'a> {
    icons::icon_button_entry(
        kind,
        byteui::theme::icon_size::row(),
        false,
        false,
        state.find_button_hover(key),
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        on_press,
        move |hovered| Message::Hover(key, false, hovered),
        tooltip,
    )
}

/// 页内查找条:输入框(Enter = 下一个)+ 没有匹配的提示 + 上一个 / 下一个 / 关闭。高度是
/// `find_bar_height_px()`,下留白 `find_bar_bottom_pad()`——宿主的网页几何用 `find_bar_extra_px` 让出同样的高度。
fn find_bar<'a>(state: &'a State, locale: &str, lh_factor: f32) -> El<'a> {
    let query = state.find_query();
    let input = byteui::form::input_text::view(
        &t("find.placeholder", "Find in page", locale),
        query,
        false,
        Some(find_field_id()),
        false,
        Some(Message::FindNext),
        true,
        Message::FindInput,
    );
    let no_match = state.find_matched() == Some(false) && !query.is_empty();
    let status: El<'a> = if no_match {
        lh(
            lh_factor,
            text(t("find.no_match", "No matches", locale))
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().red),
        )
        .into()
    } else {
        iced_widget::Space::new().into()
    };
    let input_box = container(input)
        .width(Length::Fill)
        .height(Length::Fixed(find_bar_height_px()))
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .padding([2, 8])
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().term_bg.into()),
            border: Border {
                color: if no_match {
                    byteui::theme::color::current().red
                } else {
                    byteui::theme::color::current().border
                },
                width: 1.0,
                radius: 2.0.into(),
            },
            ..container::Style::default()
        });
    container(
        row![
            input_box,
            status,
            find_button(
                state,
                icons::IconKind::ChevronUp,
                FIND_PREV_KEY,
                Message::FindPrev,
                t("find.prev", "Previous match", locale),
            ),
            find_button(
                state,
                icons::IconKind::ChevronDown,
                FIND_NEXT_KEY,
                Message::FindNext,
                t("find.next", "Next match", locale),
            ),
            find_button(
                state,
                icons::IconKind::CircleX,
                FIND_CLOSE_KEY,
                Message::FindClose,
                t("find.close", "Close find bar", locale),
            ),
        ]
        .spacing(4)
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .width(Length::Fill)
    .padding(iced_widget::core::padding::bottom(find_bar_bottom_pad()))
    .into()
}

/// 地址栏星标:当前 URL 在全局/本项目任一边已收藏则 GOLD 实心,否则
/// DIM;当前无 tab 时禁用(浏览器 tab 恒为网页,不需要像旧版
/// `current_browser_url` 那样再判一次 `TabKind`)。`project_id` 必须传
/// 真实值,不能传 `None` 占位——否则"只在本项目收藏、没加全局收藏"的
/// 网址会被误判成未收藏(`bookmark_status` 的 `project` 字段只在传了
/// `Some(pid)` 时才会去匹配)。
fn star_button<'a>(state: &'a State, project_id: Option<i64>, locale: &str) -> El<'a> {
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
fn bookmark_group<'a>(title: String, items: &[&'a BookmarkInfo], lh_factor: f32) -> El<'a> {
    let header = row![
        icons::view(
            icons::IconKind::Folder,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().dim
        ),
        lh(
            lh_factor,
            text(title)
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().dim)
        ),
    ]
    .spacing(4)
    .align_y(iced_widget::core::Alignment::Center);
    let mut col = column![header].spacing(2);
    for b in items {
        let open = button(lh(
            lh_factor,
            text(b.title.clone())
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().cream),
        ))
        .on_press(Message::OpenUrl(b.url.clone()))
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: byteui::theme::color::current().cream,
            ..button::Style::default()
        });
        let remove = button(lh(
            lh_factor,
            text("×")
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        ))
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
    lh_factor: f32,
) -> El<'a> {
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
        lh_factor,
    ));
    if project_id.is_some() {
        col = col.push(bookmark_group(
            t("sidebar.project_group", "Project bookmarks", locale),
            &project,
            lh_factor,
        ));
    }
    if both_empty {
        col = col.push(lh(
            lh_factor,
            text(t("sidebar.empty", "No bookmarks yet", locale))
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().dim),
        ));
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

/// 浏览器面板:tab 栏、地址栏(与星标/收藏夹按钮)、可选错误文案、激活 tab 的
/// 网页预览占位(收藏夹侧栏展开时与之并排)。`project_id` 用于星标收藏态判定
/// (全局/本项目两档)。页签栏 + 地址栏让给 webview 的高度见 [`crate::adapter::chrome_top_px`]。
pub fn view<'a>(
    display: &impl BrowserDisplay,
    state: &'a State,
    project_id: Option<i64>,
    bookmarks_split: f32,
    width: Length,
    outer: Border,
    mirror: bool,
) -> El<'a> {
    let region = display.region();
    let locale = display.locale();
    let lh_factor = display.line_height_factor();

    let items: Vec<El<'a>> = state
        .tabs
        .tabs()
        .iter()
        .enumerate()
        .map(|(idx, tab)| {
            let active = idx == state.tabs.active_idx();
            let tab = display.tab(
                TabView {
                    title: tab.title.clone(),
                    active,
                    hover_t: state.hover_progress(idx, false),
                    close_hover_t: state.hover_progress(idx, true),
                    show_tooltip: state.hover_tooltip_ready(idx),
                },
                Message::SelectTab(idx),
                Message::CloseTab(idx),
                move |h| Message::Hover(idx, false, h),
                move |h| Message::Hover(idx, true, h),
            );
            // 拖拽换位:按住页签(宿主把 `tab_drag` 置位)后光标扫过哪个
            // 页签,这个 `on_move` 发出 `DragHover(idx)`,再由宿主翻译成
            // 自己的换位消息(组校验在那里做),完成换位。
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
        Some(Message::TextInputMenuOpen(TextInputTarget {
            id: addr_field_id(),
            secure: false,
        })),
    );

    // 地址栏本体:单个带边框的容器,把"网址文字 + 星标(收藏)按钮"一起包进
    // 边框内(复用 todo 新增输入框的布局模式)。不再需要外层
    // `MouseArea`/`AddrClick`——`text_input` 是真控件,点击命中范围内
    // 就由 iced 标准鼠标管线自己处理聚焦,不需要应用层代理点击(唯一影响:
    // 点击胶囊的 4px padding 空白处不再能进编辑态,只有点在输入框自身范围
    // 内才行,判定为可接受的小回归)。星标按钮是内层 widget,自己截获点击
    // (弹加入/移出收藏夹菜单)。框高由按钮的方形尺寸撑起,文字垂直居中,
    // 视觉上按钮嵌在地址栏右侧。
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

    let mut content = column![tab_bar, display.tab_divider(), addr_row].spacing(region.gap);

    if state.find_open() {
        content = content.push(find_bar(state, locale, lh_factor));
    }

    if let Some(err) = &state.error {
        content = content.push(lh(
            lh_factor,
            text(format!("⚠ {}", crate::i18n::render(err, locale)))
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().red),
        ));
    }

    let body: El<'a> = if state.tabs.tabs().is_empty() {
        container(lh(
            lh_factor,
            text(t(
                "empty.no_page",
                "No page yet — type a URL in the address bar",
                locale,
            ))
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().dim),
        ))
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
        let (list_portion, content_portion) = split_portions(1.0 - bookmarks_split);
        let content_box = container(body).width(Length::FillPortion(content_portion));
        let bookmarks_box = bookmarks_panel(
            state,
            project_id,
            Length::FillPortion(list_portion),
            locale,
            lh_factor,
        );
        let divider = display.bookmarks_divider(Message::ColumnDragStart);
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
fn bookmarks_toggle_button<'a>(state: &'a State, locale: &str) -> El<'a> {
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
