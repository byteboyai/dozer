//! 预览面板的 iced 视图:页签条、工具栏、查找条、整文件搜索条、加载中、降级页、冲突条、空白信息卡。
//!
//! 逻辑与度量从 Dozer 的 `workspace/view.rs` 逐字搬来;耦合处换成 [`PanelDisplay`]:
//! 悬停表 / 页签部件 / 折叠按钮 / 配色区域 / 顶层消息类型都由产品注入。
//!
//! **webview 的占位**:`pane` 在内容区只画 iced 兜底(加载中 / 降级 / 冲突 / 空白信息)与 webview
//! 要覆盖的空白底板;webview 本身由产品按 `desired_*_webviews` 摆放。

use dozer_model::{LocalizedText, ProjectInfo};
use iced_widget::core::text::{IntoFragment, LineHeight, Wrapping};
use iced_widget::core::{Border, Length, Padding};
use iced_widget::{Text, button, column, container, row};

use crate::display::{
    Element, OverflowEntry, OverflowMenuArgs, PanelDisplay, PreviewHover, TabArgs,
};
use crate::view_model::{
    blank_info_model, conflict_bar_model, fallback_model, preview_tab_label,
    tab_close_selects_instead, toolbar_model,
};
use crate::{
    LoadObservation, Message, PANEL_OWNER, PREVIEW_CONFLICT_BAR_HEIGHT, PREVIEW_FIND_BAR_HEIGHT,
    PreviewLoadState, PreviewPane, PreviewTab, TabKind, ViewerPanel,
};
use byteui::interaction::icons;
use byteui::interaction::tab_strip::tab_window;

/// 视图要的、与产品无关的上下文。
pub struct ViewCtx<'a> {
    /// 当前项目(空白页标题 / 信息卡用);启动初帧 / 切项目中间为 `None`。
    pub project: Option<&'a ProjectInfo>,
}

fn text<'a>(
    content: impl IntoFragment<'a>,
) -> Text<'a, iced_widget::Theme, iced_renderer::Renderer> {
    Text::new(content).wrapping(Wrapping::WordOrGlyph)
}

fn t(key: &str, fallback: &str, locale: &str) -> String {
    crate::i18n::panel_text(PANEL_OWNER, key, fallback, locale)
}

fn ta(key: &str, fallback: &str, locale: &str, args: &[(&str, &str)]) -> String {
    crate::i18n::panel_text_args(PANEL_OWNER, key, fallback, locale, args)
}

/// 悬停回调:两种取值的消息预先算好,闭包不借 `host`。
fn hover_fn<M: Clone + 'static>(
    host: &impl PanelDisplay<M>,
    panel: ViewerPanel,
    hover: PreviewHover,
) -> impl Fn(bool) -> M + 'static {
    let on = host.hover_message(panel, hover, true);
    let off = host.hover_message(panel, hover, false);
    move |hovered| if hovered { on.clone() } else { off.clone() }
}

/// 文件内 Find 的「查询」与「替换」两个输入框各自的外框壳(底色同编辑器,有内容整框描金)。
pub fn find_field_shell<'a, M: 'a>(
    inner: Element<'a, M>,
    colors: byteui::theme::color::ColorTokens,
    vert: f32,
    horiz: f32,
    active: bool,
) -> Element<'a, M> {
    iced_widget::container(inner)
        .width(Length::Fill)
        .padding([vert, horiz])
        .style({
            let bg = colors.bg;
            let border_color = if active { colors.gold } else { colors.border };
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(bg.into()),
                border: Border {
                    color: border_color,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..iced_widget::container::Style::default()
            }
        })
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}

/// 空白页(`TabKind::Blank`)的 Finder "Get Info" 风格信息卡:folder icon + 项目名(2 倍 title 字号),
/// 下方四行 dim label / cream value——位置 / 大小 / 创建时间 / 修改时间。`:` 严格对齐于同一条竖线;
/// `blank_info` 为 `None` 时三行值画 `—`;无项目只画头部。
pub fn blank_info_card<'a, M: 'a>(
    project: Option<&ProjectInfo>,
    pane: &PreviewPane,
    locale: &str,
) -> Element<'a, M> {
    let theme_colors = byteui::theme::color::current();
    let model = blank_info_model(project, pane.blank_info.as_ref(), locale);
    // `label_colon_w=96` 容下「修改时间:」(5 CJK + 冒号 @body_font 14,实测约 90px)。
    let label_colon_w: f32 = 96.0;
    let stats_padding_left: f32 = 16.0;
    // icon 槽位宽 = stats 的行首 padding + label 列宽,槽位内右对齐 → icon 右缘正好落在 `:` 竖线上。
    let icon_slot_w: f32 = stats_padding_left + label_colon_w;
    let icon_size: f32 = 64.0;
    let header_spacing: f32 = 20.0;
    let header = row![
        container(icons::view(
            icons::IconKind::FolderDot,
            icon_size,
            theme_colors.dim
        ))
        .width(Length::Fixed(icon_slot_w))
        .align_x(iced_widget::core::alignment::Horizontal::Right),
        text(model.header_label)
            .size(byteui::theme::font::title() * 2)
            .color(theme_colors.cream),
    ]
    .spacing(header_spacing)
    .align_y(iced_widget::core::Alignment::Center);

    let Some(rows) = model.stats else {
        return container(
            column![header]
                .spacing(0)
                .align_x(iced_widget::core::Alignment::Start),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into();
    };

    let dim = theme_colors.dim;
    let cream = theme_colors.cream;
    let body_font = byteui::theme::font::body();
    let stat_row = |(label, value): (String, String)| {
        row![
            container(text(label).size(body_font).color(dim))
                .width(Length::Fixed(label_colon_w))
                .align_x(iced_widget::core::alignment::Horizontal::Right),
            text(value).size(body_font).color(cream),
        ]
        .spacing(8)
    };
    let [location, size, created, modified] = rows;
    let stats = column![
        stat_row(location),
        stat_row(size),
        stat_row(created),
        stat_row(modified),
    ]
    .spacing(8)
    .padding(Padding {
        top: 0.0,
        bottom: 0.0,
        left: stats_padding_left,
        right: 0.0,
    });

    let card = column![header, stats]
        .spacing(28)
        .align_x(iced_widget::core::Alignment::Start);

    container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}

/// 文件预览的统一 loading 视图:按当前加载阶段取文案,用 math_curve 画自驱动动画;阶段为 `Idle`
/// 时返回 `None`。有进度时在动画下方追加一行已完成计数(`progress` 由调用方限频更新)。
///
/// T11 bullet 4:视图组装即「动画开始画」;在此写回首帧时刻(经 `observe` 的共享单元),用于区分
/// 「动画没机会画」与「后台任务本身慢」。
pub fn loading_view<'a, M: 'a>(
    load_state: &PreviewLoadState,
    observe: Option<&LoadObservation>,
    locale: &str,
) -> Option<Element<'a, M>> {
    if let Some(observe) = observe {
        observe.mark_first_frame(load_state.generation);
    }
    let label = load_state.stage.localized(locale)?;
    // byteui::loading_hint 只收 `&'static str`,本地化文案是运行期 owned,故就地复用其 `view_animated`。
    let hint: Element<'a, M> = container(
        column![
            byteui::feedback::math_curve::view_animated(
                byteui::feedback::math_curve::Curve::RoseThree,
                48.0,
            ),
            text(label)
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        ]
        .spacing(12)
        .align_x(iced_widget::core::alignment::Horizontal::Center),
    )
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .into();
    let Some(progress) = load_state.progress else {
        return Some(hint);
    };
    let progress_text = match progress.total {
        Some(total) => format!("{}/{}", progress.completed, total),
        None => format!("{}", progress.completed),
    };
    Some(
        column![
            hint,
            text(progress_text)
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().dim),
        ]
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .into(),
    )
}

/// External / Unsupported(及 Failed)统一的降级页:文件类型 / 路径 / route reason / 失败原因 / 安全提示,
/// 按 `fallback_actions` 给出可用动作(重试 / 纯文本只读 / 系统应用打开)。`None` 表示该 tab 不该走这页。
pub fn fallback_page<'p: 'a, 'b: 'a, 'a, M: Clone + 'static>(
    host: &'a impl PanelDisplay<M>,
    panel: ViewerPanel,
    tab: &'a PreviewTab,
) -> Option<Element<'a, M>> {
    let locale: &'a str = host.locale();
    let product: &'a str = host.product_name();
    let model = fallback_model(tab, locale, product)?;
    // `model` 里的字符串在 `widget::text` 里被重新分配,只有各 `&str` 字段
    // 会进 `Element<'a>`;因此要求 locale/product 活到 `'a` 即可,不再另求 `'b`。
    let _: &'a str = locale;
    let tab_id = model.tab_id;
    let actions = model.actions;

    let colors = byteui::theme::color::current();
    let body = byteui::theme::font::body();
    let label = byteui::theme::font::label();
    let caption = byteui::theme::font::caption_sm();
    let mut col = column![
        text(model.title)
            .size(byteui::theme::font::title())
            .color(colors.cream),
        text(model.headline).size(body).color(colors.dim),
    ]
    .spacing(8)
    .align_x(iced_widget::core::Alignment::Start);
    if let Some(path) = model.path {
        col = col.push(text(path).size(caption).color(colors.dim));
    }
    if let Some(route) = model.route {
        col = col.push(text(route).size(caption).color(colors.dim));
    }
    if let Some(failure) = model.failure {
        col = col.push(text(failure).size(caption).color(colors.red));
    }
    col = col.push(text(model.security_note).size(caption).color(colors.dim));

    // 三个动作按钮走产品统一的操作按钮样式;「重试」是推荐动作用奶油色主按钮,其余为次要动作走 dim。
    let mut actions_row = row![]
        .spacing(8)
        .align_y(iced_widget::core::Alignment::Center);
    if actions.retry {
        actions_row = actions_row.push(
            button(text(t("preview.retry", "Retry", locale)).size(label))
                .on_press(host.wrap(panel, Message::Retry(tab_id)))
                .padding([8, 20])
                .style(host.action_button_style(colors.cream)),
        );
    }
    if actions.plain_text_read_only {
        actions_row = actions_row.push(
            button(
                text(t(
                    "preview.fallback_try_plain_text",
                    "Try as read-only plain text",
                    locale,
                ))
                .size(label),
            )
            .on_press(host.wrap(panel, Message::PlainTextOpen(tab_id)))
            .padding([8, 20])
            .style(host.action_button_style(colors.dim)),
        );
    }
    if actions.external_open {
        actions_row = actions_row.push(
            button(text(t("preview.open_external", "Open in system app", locale)).size(label))
                .on_press(host.wrap(panel, Message::OpenExternal(tab_id)))
                .padding([8, 20])
                .style(host.action_button_style(colors.dim)),
        );
    }
    col = col.push(actions_row);

    Some(
        container(col)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced_widget::core::alignment::Horizontal::Center)
            .align_y(iced_widget::core::alignment::Vertical::Center)
            .into(),
    )
}

/// 激活 CodeMirror tab 的磁盘冲突条——提示 + 「保留我的修改」/「重载磁盘」两键。
/// `armed` 为真时重载键变成二次确认文案(再点一次才丢弃改动)。
pub fn conflict_bar<'a, M: Clone + 'static>(
    host: &impl PanelDisplay<M>,
    panel: ViewerPanel,
    tab_id: usize,
    armed: bool,
) -> Element<'a, M> {
    let colors = byteui::theme::color::current();
    let body = byteui::theme::font::body();
    let model = conflict_bar_model(armed, host.locale());
    let keep = iced_widget::button(text(model.keep_label).size(body))
        .on_press(host.wrap(panel, Message::ConflictKeep(tab_id)));
    let reload = iced_widget::button(text(model.reload_label).size(body))
        .on_press(host.wrap(panel, Message::ConflictReload(tab_id)));
    container(
        row![
            text(model.notice).size(body).color(colors.red),
            iced_widget::space::horizontal(),
            keep,
            reload,
        ]
        .spacing(8)
        .align_y(iced_widget::core::Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding([4, 8])
    .into()
}

/// 文件内 Find 条的共有渲染:原生档与 webview(flyfish)档复用同一套查询框 / 「Aa」大小写开关 / n-m 计数 /
/// 上下命中按钮。`show_replace` 控制是否渲染替换行与其展开圆盘箭头(webview 档 flyfish 没有替换概念)。
pub fn find_bar<'p: 'a, 'b: 'a, 'a, M: Clone + 'static>(
    host: &'a impl PanelDisplay<M>,
    panel: ViewerPanel,
    pane: &'a PreviewPane,
    show_replace: bool,
) -> Element<'a, M> {
    let Some(find) = pane.find_state() else {
        return iced_widget::Space::new().into();
    };
    let locale = host.locale();
    let colors = byteui::theme::color::current();
    let find_font = host.find_font_size();
    let case_toggle: Element<'a, M> = {
        let active = find.case_sensitive;
        button(text("Aa").size(byteui::theme::font::body()))
            .on_press(host.wrap(panel, Message::FindCase(!active)))
            .padding(2)
            .style(move |_t, _s| button::Style {
                background: None,
                text_color: if active { colors.cyan } else { colors.dim },
                ..button::Style::default()
            })
            .into()
    };
    let input = byteui::form::input_text::view_with_suffix_unframed_at_size(
        find_font,
        &t("find.search_placeholder", "Search", locale),
        &find.query,
        false,
        Some(crate::find_field_id(panel)),
        !find.query.is_empty(),
        None,
        move |q: String| host.wrap(panel, Message::FindText(q)),
        case_toggle,
    );
    let count_label: Element<'a, M> = if find.count > 0 {
        text(format!("{}/{}", find.current + 1, find.count))
            .size(byteui::theme::font::body())
            .color(colors.dim)
            .into()
    } else if !find.query.is_empty() {
        text(t("find.no_results", "0 results", locale))
            .size(byteui::theme::font::body())
            .color(colors.red)
            .into()
    } else {
        container(iced_widget::Row::<
            M,
            iced_widget::Theme,
            iced_renderer::Renderer,
        >::new())
        .into()
    };
    let bordered = move |btn: Element<'static, M>| -> Element<'static, M> {
        container(btn)
            .style(
                move |_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: None,
                    border: Border {
                        color: colors.border,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..iced_widget::container::Style::default()
                },
            )
            .into()
    };
    let step_icon = |next: bool| -> Element<'static, M> {
        let hover = if next {
            PreviewHover::FindNext
        } else {
            PreviewHover::FindPrev
        };
        let btn = icons::icon_button_entry(
            if next {
                icons::IconKind::ArrowDown
            } else {
                icons::IconKind::ArrowUp
            },
            byteui::theme::icon_size::row(),
            false,
            false,
            host.hover_progress(panel, hover),
            false,
            byteui::theme::geometry::tab_button_size(),
            true,
            host.wrap(panel, Message::FindGo(next)),
            hover_fn(host, panel, hover),
            if next {
                t("find.next", "Next", locale)
            } else {
                t("find.prev", "Previous", locale)
            },
        );
        bordered(btn)
    };
    // 替换行展开圆盘箭头只在 show_replace(原生档)出现。
    let replace_toggle_opt: Option<Element<'a, M>> = if show_replace {
        let replace_open = find.replace_open;
        Some(icons::icon_button_entry(
            if replace_open {
                icons::IconKind::ChevronDown
            } else {
                icons::IconKind::ChevronRight
            },
            byteui::theme::icon_size::row(),
            false,
            false,
            host.hover_progress(panel, PreviewHover::FindReplaceToggle),
            false,
            byteui::theme::geometry::tab_button_size(),
            true,
            host.wrap(panel, Message::FindReplaceToggle),
            hover_fn(host, panel, PreviewHover::FindReplaceToggle),
            if replace_open {
                t("find.collapse_replace", "Hide replace", locale)
            } else {
                t("find.expand_replace", "Show replace", locale)
            },
        ))
    } else {
        None
    };
    const FIND_TRAILING_ZONE: f32 = 150.0;
    let query_trailing = container(
        row![count_label, step_icon(false), step_icon(true)]
            .spacing(4)
            .align_y(iced_widget::core::alignment::Alignment::Center),
    )
    .width(Length::Fixed(FIND_TRAILING_ZONE))
    .align_x(iced_widget::core::alignment::Horizontal::Right);
    let query_active = !find.query.is_empty() || pane.find_query_focused();
    let mut query_row_children: Vec<Element<'a, M>> = Vec::new();
    if let Some(rt) = replace_toggle_opt {
        query_row_children.push(rt);
    }
    query_row_children.push(find_field_shell(input, colors, 7.0, 10.0, query_active));
    query_row_children.push(query_trailing.into());
    let mut query_row_inner = iced_widget::Row::new()
        .spacing(4)
        .align_y(iced_widget::core::alignment::Alignment::Center);
    for c in query_row_children {
        query_row_inner = query_row_inner.push(c);
    }
    let query_row: Element<'a, M> = container(query_row_inner).width(Length::Fill).into();
    let mut find_rows: Vec<Element<'a, M>> = vec![query_row];
    // 替换行只在 show_replace(原生档)且展开时出现;webview 档恒不渲染。
    if show_replace && find.replace_open {
        let armed = find.count > 0;
        let replacement_input = byteui::form::input_text::view_at_size(
            find_font,
            &t("find.replace_placeholder", "Replace", locale),
            &find.replacement,
            false,
            None,
            !find.replacement.is_empty(),
            None,
            true,
            move |s: String| host.wrap(panel, Message::FindReplacement(s)),
        );
        let replace_icon_button = |kind: icons::IconKind,
                                   hover: PreviewHover,
                                   msg: M,
                                   tooltip: String,
                                   disabled: bool|
         -> Element<'static, M> {
            let btn = icons::icon_button_entry(
                kind,
                byteui::theme::icon_size::row(),
                false,
                disabled,
                host.hover_progress(panel, hover),
                false,
                byteui::theme::geometry::tab_button_size(),
                !disabled,
                msg,
                hover_fn(host, panel, hover),
                tooltip,
            );
            bordered(btn)
        };
        let replace_indent = (byteui::theme::geometry::tab_button_size() + 4.0 - 6.0).max(0.0);
        let replace_trailing = container(
            row![
                replace_icon_button(
                    icons::IconKind::Replace,
                    PreviewHover::FindReplaceCurrent,
                    host.wrap(panel, Message::FindReplaceCurrent),
                    t("find.replace_current", "Replace", locale),
                    !armed,
                ),
                replace_icon_button(
                    icons::IconKind::ReplaceAll,
                    PreviewHover::FindReplaceAll,
                    host.wrap(panel, Message::FindReplaceAll),
                    t("find.replace_all", "Replace All", locale),
                    !armed,
                ),
            ]
            .spacing(6)
            .align_y(iced_widget::core::alignment::Alignment::Center),
        )
        .width(Length::Fixed(FIND_TRAILING_ZONE))
        .align_x(iced_widget::core::alignment::Horizontal::Right);
        let replace_row = container(
            row![
                iced_widget::Space::new().width(Length::Fixed(replace_indent)),
                find_field_shell(
                    replacement_input,
                    colors,
                    4.0,
                    10.0,
                    !find.replacement.is_empty()
                ),
                replace_trailing,
            ]
            .spacing(6)
            .align_y(iced_widget::core::alignment::Alignment::Center),
        )
        .width(Length::Fill)
        .into();
        find_rows.push(replace_row);
    }
    container(column(find_rows).spacing(4))
        .width(Length::Fill)
        .padding(8)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(colors.card.into()),
                border: Border {
                    color: colors.border,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..iced_widget::container::Style::default()
            },
        )
        .into()
}

/// 窗口化大文件整文件搜索条:查询框 + n/m 计数 + 上一条/下一条 + 关闭。整文件流式扫描只做「找到并
/// 跳转」,没有大小写开关 / 替换行。`tab_id` 是 session 锁定的窗口化 CodeMirror tab。
pub fn large_file_search_bar<'p: 'a, 'b: 'a, 'a, M: Clone + 'static>(
    host: &'a impl PanelDisplay<M>,
    panel: ViewerPanel,
    pane: &'a PreviewPane,
    tab_id: usize,
) -> Element<'a, M> {
    let Some(session) = pane.large_file_search_state() else {
        return iced_widget::Space::new().into();
    };
    let locale = host.locale();
    let colors = byteui::theme::color::current();
    let query_for_submit = session.query.clone();
    // T9 bullet 1:搜索在途时只在**搜索条**内显示一个小尺寸 math_curve + 「搜索中…」。
    let status_label = if session.running {
        let mut label = t("find.searching", "Searching…", locale);
        if let Some(p) = session.progress
            && let Some(total) = p.total
            && total > 0
        {
            let pct = (p.completed.saturating_mul(100) / total).min(100);
            label = ta(
                "find.searching_pct",
                "Searching… {pct}%",
                locale,
                &[("pct", &pct.to_string())],
            );
        }
        text(label)
            .size(byteui::theme::font::body())
            .color(colors.dim)
    } else if let Some(error) = &session.error {
        text(ta(
            "find.search_failed",
            "Search failed: {error}",
            locale,
            &[("error", error)],
        ))
        .size(byteui::theme::font::body())
        .color(colors.gold)
    } else {
        // 命中计数:截断时总数标注为 `total_matches+`(T9 bullet 4)。
        let shown = if session.hits.is_empty() {
            0
        } else {
            session.current + 1
        };
        let total = if session.truncated {
            format!("{}+", session.total_matches)
        } else {
            session.total_matches.to_string()
        };
        text(format!("{shown}/{total}"))
            .size(byteui::theme::font::body())
            .color(colors.dim)
    };
    let mut status_row = row![]
        .spacing(6)
        .align_y(iced_widget::core::alignment::Alignment::Center);
    if session.running {
        status_row = status_row.push(byteui::feedback::math_curve::view_animated::<M>(
            byteui::feedback::math_curve::Curve::RoseThree,
            16.0,
        ));
    }
    status_row = status_row.push(status_label);
    let input = byteui::form::input_text::view(
        &t(
            "find.search_file_placeholder",
            "Search file contents…",
            locale,
        ),
        &session.query,
        false,
        Some(crate::large_file_search_field_id(panel)),
        !session.query.is_empty(),
        Some(host.wrap(
            panel,
            Message::LargeFileSearchSubmit {
                tab_id,
                query: query_for_submit,
            },
        )),
        false,
        move |s: String| host.wrap(panel, Message::LargeFileSearchSubmit { tab_id, query: s }),
    );
    let row_el = row![
        input,
        status_row,
        button(text("↑")).on_press(host.wrap(panel, Message::LargeFileSearchGo(false))),
        button(text("↓")).on_press(host.wrap(panel, Message::LargeFileSearchGo(true))),
        button(text("×")).on_press(host.wrap(panel, Message::LargeFileSearchClose)),
    ]
    .spacing(6)
    .align_y(iced_widget::core::alignment::Alignment::Center);
    container(row_el)
        .width(Length::Fill)
        .padding(8)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(colors.card.into()),
                border: Border {
                    color: colors.border,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..iced_widget::container::Style::default()
            },
        )
        .into()
}

/// 页签条「溢出下拉」浮层。**必须**由产品在 `App::view` 顶层 `stack!` 里拼(锚点 / 窗口尺寸是全窗口
/// 坐标系)。下拉列出该面板**全部** tab(不管当前是否横向可见),便于随时跳转。
pub fn overflow_menu<'b: 'a, 'a, M: Clone + 'static>(
    host: &'a impl PanelDisplay<M>,
    panel: ViewerPanel,
    pane: &'a PreviewPane,
    anchor: Option<(f32, f32)>,
) -> Option<Element<'a, M>> {
    let anchor = anchor?;
    if pane.tabs().is_empty() {
        return None;
    }
    let entries: Vec<OverflowEntry> = pane
        .tabs()
        .iter()
        .enumerate()
        .map(|(idx, tab)| OverflowEntry {
            index: idx,
            title: tab.title.clone(),
            active: idx == pane.active_idx(),
            // index 0 的 `Blank` 占位固定存在、不可关闭,下拉里这一行不画 ×。
            closable: !matches!(tab.kind, TabKind::Blank),
            hover_t: host.hover_progress(panel, PreviewHover::TabOverflowRow(idx)),
        })
        .collect();
    Some(host.overflow_menu(OverflowMenuArgs {
        entries,
        anchor,
        on_select: Box::new(move |idx| host.wrap(panel, Message::SelectTab(idx))),
        on_close: Box::new(move |idx| host.wrap(panel, Message::CloseTab(idx))),
        on_dismiss: host.wrap(panel, Message::TabOverflowDismiss),
        on_row_hover: Box::new(move |idx, hovered| {
            host.hover_message(panel, PreviewHover::TabOverflowRow(idx), hovered)
        }),
    }))
}

/// 预览 pane 的共同渲染(Files 预览与 Project 面板右配对复用同一套页签条 / 兜底页 / 条)。
///
/// **空白页 tab(`TabKind::Blank`)的标题显示项目根目录 basename**(见 `preview_tab_label`);
/// 宽度估计和实际渲染共用同一份 helper,避免选错宽度导致标题裁切。
#[allow(clippy::too_many_arguments)]
pub fn pane<'b: 'a, 'p: 'a, 'a, M: Clone + 'static>(
    preview: &'p PreviewPane,
    tab_first: usize,
    error: Option<&'a LocalizedText>,
    panel: ViewerPanel,
    host: &'a impl PanelDisplay<M>,
    ctx: &ViewCtx<'a>,
    width: Length,
    outer: Border,
) -> Element<'a, M> {
    let locale = host.locale();
    let region = host.pane_style();

    let widths: Vec<f32> = preview
        .tabs()
        .iter()
        .map(|tab| {
            host.tab_display_width(&crate::view_model::preview_tab_title(
                ctx.project,
                tab,
                locale,
            ))
        })
        .collect();
    let window = tab_window(&widths, 4.0, host.tab_bar_avail_px(panel), tab_first);

    let items: Vec<Element<'a, M>> = preview
        .tabs()
        .iter()
        .enumerate()
        .filter(|(idx, _)| (window.first..window.visible_end).contains(idx))
        .map(|(idx, tab)| {
            let active = idx == preview.active_idx();
            let item = PreviewHover::TabItem(idx);
            let close = PreviewHover::TabClose(idx);
            // 就地可写的 tab 有未保存改动:标题后缀 ` *`。宽度预算仍按不带星的标题估。
            let display_title = preview_tab_label(ctx.project, tab, locale);
            // 页签右键菜单:仅对真实文件 tab(排除 index 0 的 `Blank` 占位)生效。
            let tab_path = match &tab.kind {
                TabKind::File(p) => Some(p.clone()),
                _ => None,
            };
            let select_msg = host.wrap(panel, Message::SelectTab(idx));
            let tab_el = host.panel_tab(TabArgs {
                title: display_title,
                active,
                hover_t: host.hover_progress(panel, item),
                close_hover_t: host.hover_progress(panel, close),
                on_select: select_msg.clone(),
                // Blank 占位 tab 的 ×:有兄弟 tab 时是真关;单独存在时是 select(等价 no-op,但点击 ×
                // 不会有「列表瞬空再补」的视觉跳动)。文件 tab 一律真关。
                on_close: if tab_close_selects_instead(tab, preview.tabs().len()) {
                    select_msg
                } else {
                    host.wrap(panel, Message::CloseTab(idx))
                },
                show_tooltip: host.hover_tooltip_ready(panel, item),
                title_hover: Box::new(hover_fn(host, panel, item)),
                close_hover: Box::new(hover_fn(host, panel, close)),
            });
            host.tab_gestures(panel, idx, tab_el, tab_path)
        })
        .collect();
    // tab 列表进 clip 容器占 Fill,裁掉右侧溢出;V 按钮钉在裁剪区外、tab 组最左侧。
    let tabs_row = row(items).spacing(4);
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    // 三个模式切换按钮出现与否、当前态都由 `toolbar_model` 决定(纯函数,有测试)。
    let toolbar = toolbar_model(preview);
    let render_mode_button = toolbar.render_mode.map(|b| {
        host.render_mode_button(
            b.in_code_mode,
            host.hover_progress(panel, PreviewHover::RenderMode),
            host.wrap(panel, Message::ToggleRenderMode(b.tab_idx)),
            Box::new(hover_fn(host, panel, PreviewHover::RenderMode)),
        )
    });
    let tabular_mode_button = toolbar.tabular_mode.map(|(tab_id, in_text)| {
        host.tabular_mode_button(
            in_text,
            host.hover_progress(panel, PreviewHover::TabularMode),
            host.wrap(panel, Message::TabularTextModeToggle(tab_id)),
            Box::new(hover_fn(host, panel, PreviewHover::TabularMode)),
        )
    });
    let json_mode_button = toolbar.json_mode.map(|(tab_id, in_text)| {
        host.json_mode_button(
            in_text,
            host.hover_progress(panel, PreviewHover::JsonMode),
            host.wrap(panel, Message::JsonModeToggle(tab_id)),
            Box::new(hover_fn(host, panel, PreviewHover::JsonMode)),
        )
    });
    let overflow_button = host.overflow_button(
        preview.tabs().len(),
        host.hover_progress(panel, PreviewHover::TabOverflow),
        host.wrap(panel, Message::TabOverflowToggle { cursor: (0.0, 0.0) }),
        Box::new(hover_fn(host, panel, PreviewHover::TabOverflow)),
    );
    let mut tab_bar_row = row![]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);
    if let Some(btn) = overflow_button {
        tab_bar_row = tab_bar_row.push(btn);
    }
    tab_bar_row = tab_bar_row.push(clipped);
    if let Some(btn) = render_mode_button {
        tab_bar_row = tab_bar_row.push(btn);
    }
    if let Some(btn) = tabular_mode_button {
        tab_bar_row = tab_bar_row.push(btn);
    }
    if let Some(btn) = json_mode_button {
        tab_bar_row = tab_bar_row.push(btn);
    }
    let tab_bar = tab_bar_row.push(host.collapse_button(panel));

    let mut content = column![tab_bar, host.tab_divider()].spacing(region.gap);

    if let Some(err) = error {
        content = content.push(
            text(format!("⚠ {}", host.render_text(err)))
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().red)
                .line_height(LineHeight::Relative(host.line_height_factor())),
        );
        // 失败时的外部打开 fallback:路径取自当前文件 tab(不隐式执行文件本身)。
        if let Some(tab) = preview.tabs().get(preview.active_idx())
            && matches!(tab.kind, TabKind::File(_))
        {
            content = content.push(
                iced_widget::button(
                    text(t("preview.open_external", "Open in system app", locale))
                        .size(byteui::theme::font::label()),
                )
                .on_press(host.wrap(panel, Message::OpenExternal(tab.id)))
                .padding([8, 20])
                .style(host.action_button_style(byteui::theme::color::current().dim)),
            );
        }
    }

    // `PreviewPane` 恒定携带第 0 项 `TabKind::Blank` 占位,所以这里的列表正常不会空——保留空分支纯属防御。
    if preview.tabs().is_empty() {
        content = content.push(
            iced_widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fill),
        );
    } else {
        let active_idx = preview.active_idx().min(preview.tabs().len() - 1);
        let active_tab = &preview.tabs()[active_idx];
        if let Some(page) = fallback_page(host, panel, active_tab) {
            // External / Unsupported(及 Failed)统一降级页。失败优先于 loading:终态不能被动画盖住。
            content = content.push(page);
        } else if let Some(loading) = loading_view(
            &active_tab.load_state,
            active_tab.load_observe.as_ref(),
            locale,
        ) {
            // 有阶段在途时显示统一 loading。原生 WebView 在期望清单里因 `backend_state != Ready` 而不产出
            // spec(或不置 visible),所以动画不会被原生子视图遮挡。
            content = content.push(loading);
        } else if active_tab.kind == TabKind::Blank {
            content = content.push(blank_info_card(ctx.project, preview, locale));
        }
        // webview(flyfish)档 Find 条:渲染在内容区顶部,给已被下推的 webview 矩形让出固定高度的那一条。
        if preview.find_state().is_some_and(|f| f.is_webview) {
            let bar = find_bar(host, panel, preview, false);
            content = content.push(
                container(bar)
                    .width(Length::Fill)
                    .height(Length::Fixed(PREVIEW_FIND_BAR_HEIGHT)),
            );
        }
        // 窗口化大文件整文件搜索条:窗口化 host 同样是原生 webview,条渲染时必须显式把它的矩形下推让位。
        if let Some(session) = preview.large_file_search_state() {
            let bar = large_file_search_bar(host, panel, preview, session.tab_id);
            content = content.push(
                container(bar)
                    .width(Length::Fill)
                    .height(Length::Fixed(PREVIEW_FIND_BAR_HEIGHT)),
            );
        }
        // T10 冲突条:激活 CodeMirror tab 处于磁盘冲突态时,内容区顶部显示「保留我的修改 / 重载磁盘」。
        if let Some((conflict_tab_id, armed)) = preview.active_conflict() {
            let bar = conflict_bar(host, panel, conflict_tab_id, armed);
            content = content.push(
                container(bar)
                    .width(Length::Fill)
                    .height(Length::Fixed(PREVIEW_CONFLICT_BAR_HEIGHT)),
            );
        }
    }

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
