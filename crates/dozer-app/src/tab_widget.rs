// crates/dozer-app/src/tab_widget.rs
//! 面板内 tab 栏的共享部件:`panel_tab`(单个 tab 渲染器,被终端
//! `terminal.rs`、SSH 面板 `app.rs::ssh_tab_bar`、文件/项目预览
//! `workspace.rs`、浏览器 `extensions::browser` 四处跨模块复用)+
//! `tab_arrow_button`/`tab_window`(翻页箭头 + 窗口化滚动算法,被终端
//! `tab_bar` 和 `workspace.rs` 的预览页签栏两处复用)。
//!
//! `tab_bar`/`tab_drag_surface`/`tab_item`/`active_tab_view` 表面上看
//! 起来也是"共享 chrome",摸底后发现实际唯一调用方只有终端自己,已经
//! 搬进 `terminal.rs`,不在这里。

use crate::app::{controlled_tooltip, top_bar_font};
use byteui::interaction::{icons, tabs};
use iced_widget::core::{Border, Color, Element, Length, Padding};
use iced_widget::tooltip::Position;
use iced_widget::{button, container, row, text};

/// 面板 tab 统一上限宽（对齐顶栏 `project_tab_max_width`）。标题超宽时直接
/// 隐藏溢出(不换行、不省略号),正常情况下 tab 宽度随标题适配。导出给
/// `workspace.rs`/`browser.rs` 的翻页宽度估算共用,避免各处硬编码 160。
pub(crate) const PANEL_TAB_MAX_W: f32 = 160.0;
/// 面板 tab 内边距:横向留白给 hover 胶囊,纵向收紧以缩小高度。左侧单独
/// 放大(原先与右侧同为 4,标题贴左缘太紧),右侧维持贴近关闭按钮的窄距。
/// 上下 `PANEL_TAB_PAD_Y` 从 1 收到 0(验收反馈:tab 栏分割线要跟左栏
/// header 分割线对齐,tab 栏整体降 2px 更贴近 header 那侧的基线)。
const PANEL_TAB_PAD_LEFT: f32 = 10.0;
const PANEL_TAB_PAD_X: f32 = 4.0;
const PANEL_TAB_PAD_Y: f32 = 0.0;

/// 面板内 tab（终端 / 预览 / 浏览器三处共用）的渲染器，样式对齐顶栏未选中
/// 页签：标题 `body()`(13px) + `top_bar_font()`，静止 `DIM`、hover 动画
/// `DIM→金`；关闭 `×` 静止 `DIM`、hover `DIM→金`、24×24 命中框；未选中
/// hover 显 `TAB_HOVER` 胶囊背景（radius 8）。tab 宽度随标题适配
/// (`Length::Shrink`)，超过 `PANEL_TAB_MAX_W` 时标题超宽部分直接隐藏
/// (不换行、不补省略号，靠 `clip` 裁掉溢出，见 CODEBUDDY 需求)。激活态外观
/// (CREAM 标题 + CARD 实底 + 1px 边框)由本函数统一绘制，未选中态额外画
/// hover 细节。
///
/// 泛型 over 消息类型 `M`：终端/预览传 `app::Message`，浏览器传
/// `browser::Message`，保证三处渲染完全一致。`hover_t`/`close_hover_t` 是
/// 调用方动画源给的插值进度(0..=1)；`prefix` 承载终端状态点(标题左侧)，
/// `suffix` 承载预览编辑图标(标题右侧、关闭按钮前，仍是独立可点元素)。
/// `show_tooltip` 由调用方按"悬停满 2s"算好(`App::hover_tooltip_ready` /
/// `browser::State::hover_tooltip_ready`)，满则包一层 tooltip 显示标题全称。
// 共享的 panel tab 渲染器,被终端/预览/browser 三处复用;参数多是刻意保留的
// 单一职责接口(标题/激活态/两组 hover 进度与回调/前后缀/tooltip 开关),拆
// 结构体反而要引入 `Box<dyn Fn>`,得不偿失。
#[allow(clippy::too_many_arguments)]
pub(crate) fn panel_tab<'a, M: Clone + 'a>(
    title: String,
    active: bool,
    hover_t: f32,
    close_hover_t: f32,
    prefix: Option<Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>>,
    suffix: Option<Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>>,
    on_select: M,
    on_close: M,
    show_tooltip: bool,
    title_hover: impl Fn(bool) -> M + 'a,
    close_hover: impl Fn(bool) -> M + 'a,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let close_sz = byteui::theme::geometry::tab_button_size();
    // 组合 hover:悬停标题或 × 任一,胶囊背景都浮现、× 显形。
    let hover = hover_t.max(close_hover_t).clamp(0.0, 1.0);
    let hovered = hover > 0.001;
    // 标题区域最大宽 = 整 tab 上限 - 左右 padding - 与关闭按钮的间距 - 关闭按钮。
    let title_max = PANEL_TAB_MAX_W - PANEL_TAB_PAD_LEFT - PANEL_TAB_PAD_X - 2.0 - close_sz;
    let title_color = if active {
        byteui::theme::color::current().cream
    } else {
        // 未选中态:静止 DIM,hover 时平滑过渡到金(与顶栏页签同一套动画)。
        byteui::theme::color::mix(
            byteui::theme::color::current().dim,
            byteui::theme::color::current().gold,
            hover_t,
        )
    };

    let mut title_row = row![]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);
    if let Some(p) = prefix {
        title_row = title_row.push(p);
    }
    title_row = title_row.push(
        container(
            text(title.clone())
                .font(top_bar_font())
                .size(byteui::theme::font::body())
                // iced `Text` 默认 `Wrapping::Word`,不是曾经以为的
                // `None`——不显式关掉,标题超宽时会真的折成两行,而不是
                // 靠下面 `.clip(true)` 单行截断(验收反馈:较长标题换行)。
                .wrapping(iced_widget::core::text::Wrapping::None)
                .color(title_color),
        )
        // 标题超宽不补省略号、也不换行,直接裁掉溢出(见 CODEBUDDY 需求):
        // `clip` 把越界部分藏起,视觉上即"隐藏"。满 2s 悬停后由外层
        // `controlled_tooltip` 弹出全称。
        .width(Length::Shrink)
        .max_width(title_max)
        .clip(true),
    );

    // 选中/关闭的接线逻辑收在 `tabs::tab_core`(2026-08-12 抽取,试点已
    // 验证过——项目页签早已用它;这里是把面板 tab 自己那份原版实现换
    // 成同一个共享内核)。四个现有参数 title_hover/close_hover/on_select/
    // on_close 与 tab_core 的 on_select_hover/on_close_hover/on_select/
    // on_close 逐个对应,直接透传。
    let close_base = byteui::theme::color::mix(
        byteui::theme::color::current().dim,
        byteui::theme::color::current().gold,
        close_hover_t,
    );
    let close_color = Color {
        a: hover,
        ..close_base
    };
    let (select, close) = tabs::tab_core(
        title_row.into(),
        close_sz,
        close_color,
        hovered,
        on_select,
        on_close,
        title_hover,
        close_hover,
    );

    let mut tab_row = row![select]
        .spacing(2)
        .align_y(iced_widget::core::Alignment::Center);
    if let Some(s) = suffix {
        tab_row = tab_row.push(s);
    }
    tab_row = tab_row.push(close);
    let el: Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> = container(tab_row)
        .padding(Padding {
            top: PANEL_TAB_PAD_Y,
            right: PANEL_TAB_PAD_X,
            bottom: PANEL_TAB_PAD_Y,
            left: PANEL_TAB_PAD_LEFT,
        })
        .width(Length::Shrink)
        .max_width(PANEL_TAB_MAX_W)
        .style(move |_t: &iced_widget::Theme| {
            if active {
                container::Style {
                    background: Some(byteui::theme::color::current().card.into()),
                    border: Border {
                        color: byteui::theme::color::current().border,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..container::Style::default()
                }
            } else if hover > 0.0 {
                // 悬停胶囊铺满整片 tab(含 × 区),× 落在其内部。
                container::Style {
                    background: Some(
                        Color {
                            a: hover,
                            ..byteui::theme::color::current().tab_hover
                        }
                        .into(),
                    ),
                    border: Border {
                        radius: 6.0.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                }
            } else {
                container::Style::default()
            }
        })
        .into();
    // 面板页签在屏幕底部,tooltip 用 `Top` 弹在页签上方,免出屏。仅当悬停
    // 满 2s(`show_tooltip`)才显示标题全称(见 `App::hover_tooltip_ready`)。
    controlled_tooltip(el, title, Position::Top, show_tooltip)
}

/// 箭头翻页按钮：ChevronLeft / ChevronRight，可用时 GOLD，hover 显 CARD 圆角底，到头时 DIM 且不可点。
pub(crate) fn tab_arrow_button<'a, M: Clone + 'a>(
    icon: icons::IconKind,
    enabled: bool,
    msg: M,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    // 激活(可点)态用 `#dcc9a3`(同顶栏选中页签描边 `TAB_ACTIVE_BORDER`),
    // 静止不再用金;hover 再跳到金 `#F2D94E` 提亮。
    let color = if enabled {
        byteui::theme::color::current().tab_active_border
    } else {
        byteui::theme::color::current().dim
    };
    let mut btn = button(icons::view(
        icon,
        byteui::theme::icon_size::tab_arrow(),
        color,
    ))
    .width(Length::Fixed(
        byteui::theme::geometry::tab_arrow_button_size(),
    ))
    .height(Length::Fixed(
        byteui::theme::geometry::tab_arrow_button_size(),
    ))
    .padding(0)
    .style(move |_theme, status| {
        let base = button::Style {
            background: None,
            text_color: color,
            ..button::Style::default()
        };
        if !enabled {
            return base;
        }
        match status {
            button::Status::Hovered | button::Status::Pressed => button::Style {
                background: Some(byteui::theme::color::current().card.into()),
                text_color: byteui::theme::color::current().gold,
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..base
            },
            _ => base,
        }
    });
    if enabled {
        btn = btn.on_press(msg);
    }
    btn.into()
}

/// 给定各 tab 宽、tab 间距、可视宽、当前 first，算出：
/// (钳制后的 first, 左可滚, 右可滚)。
/// - 全部 tab 能放下(总宽<=avail) → first=0, 两端皆不可滚(箭头都变灰)。
/// - 溢出 → max_first = 最小的 i 使 tabs[i..] 总宽 <= avail(即从 i 起剩余恰好放得下);
///   钳制 first 到 [0, max_first]; 左可滚 = first>0; 右可滚 = first<max_first。
pub(crate) fn tab_window(
    widths: &[f32],
    gap: f32,
    avail: f32,
    first: usize,
) -> (usize, bool, bool) {
    let n = widths.len();
    if n == 0 {
        return (0, false, false);
    }
    let total: f32 = widths.iter().sum::<f32>() + gap * (n.saturating_sub(1)) as f32;
    if total <= avail {
        return (0, false, false);
    }
    // 求 max_first：从右往左累加，找最大的窗口起点使 tails 放得下。
    let mut max_first = n - 1;
    let mut acc = 0.0;
    for i in (0..n).rev() {
        let w = widths[i] + if i < n - 1 { gap } else { 0.0 };
        if acc + w <= avail {
            acc += w;
            max_first = i;
        } else {
            break;
        }
    }
    let clamped = first.min(max_first);
    (clamped, clamped > 0, clamped < max_first)
}
