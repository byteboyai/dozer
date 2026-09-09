// crates/dozer-app/src/tab_widget.rs
//! 面板内 tab 栏的共享部件:`panel_tab`(单个 tab 渲染器,被终端
//! `terminal.rs`、SSH 面板 `app.rs::ssh_tab_bar`、文件/项目预览
//! `workspace.rs`、浏览器 `extensions::browser` 四处跨模块复用)、
//! `tab_window`/`tab_window_reveal`(窗口化滚动算法 + 选中自动带入可见区),
//! 以及 `tab_overflow_button`/`tab_overflow_menu`(V 溢出下拉入口与悬浮
//! 菜单,列出某 tab 组内全部 tab,供 `terminal.rs`/`workspace.rs`/
//! `ssh_tab_bar`/Database 面板复用)。
//!
//! `tab_bar`/`tab_drag_surface`/`tab_item`/`active_tab_view` 表面上看
//! 起来也是"共享 chrome",摸底后发现实际唯一调用方只有终端自己,已经
//! 搬进 `terminal.rs`,不在这里。

use crate::app::{controlled_tooltip, top_bar_font};
use byteui::interaction::{icons, tabs};
use iced_widget::core::{Border, Color, Element, Length, Padding};
use iced_widget::tooltip::Position;
use iced_widget::{MouseArea, button, column, container, row, scrollable, stack, text};

/// 面板 tab 内边距:横向留白给 hover 胶囊,纵向收紧以缩小高度。左侧单独
/// 放大(原先与右侧同为 4,标题贴左缘太紧),右侧维持贴近关闭按钮的窄距。
/// 上下 `PANEL_TAB_PAD_Y` 从 1 收到 0(验收反馈:tab 栏分割线要跟左栏
/// header 分割线对齐,tab 栏整体降 2px 更贴近 header 那侧的基线)。
const PANEL_TAB_PAD_LEFT: f32 = 10.0;
const PANEL_TAB_PAD_X: f32 = 4.0;
const PANEL_TAB_PAD_Y: f32 = 0.0;

/// 传给 `tab_label::max_width` 表示"不设上限"的名目值——面板 tab 随标题实际
/// 内容伸缩,标题区用不上限宽参数 (`tab_label` 仅在下拉行需要按行宽裁列)。
pub(crate) const NO_TAB_W_LIMIT: f32 = f32::MAX;

/// 面板内 tab（终端 / 预览 / 浏览器三处共用）的渲染器，样式对齐顶栏未选中
/// 页签：标题 `body()`(13px) + `top_bar_font()`，静止 `DIM`、hover 动画
/// `DIM→金`；关闭 `×` 静止 `DIM`、hover `DIM→金`、24×24 命中框；未选中
/// hover 显 `TAB_HOVER` 胶囊背景（radius 8）。tab 宽度随标题实际内容伸缩
/// (`Length::Shrink`)，不设上限——标题超宽即在所在行多余空间放不下时被外层
/// `.clip` 截断（不换行、不补省略号），整行放得下时完整展示全称。激活态外观
/// (CREAM 标题 + CARD 实底 + 1px 边框)由本函数统一绘制，未选中态额外画
/// hover 细节。
///
/// 泛型 over 消息类型 `M`：终端/预览传 `app::Message`，浏览器传
/// `browser::Message`，保证三处渲染完全一致。`hover_t`/`close_hover_t` 是
/// 调用方动画源给的插值进度(0..=1)；`prefix` 承载终端状态点(标题左侧)，
/// `suffix` 承载预览编辑图标(标题右侧、关闭按钮前，仍是独立可点元素)。
/// `show_tooltip` 由调用方按"悬停满 2s"算好(`App::hover_tooltip_ready` /
/// `browser::State::hover_tooltip_ready`)，满则包一层 tooltip 显示标题全称。
// 共享的 panel tab 渲染器,被终端/预览/browser/database 多处跨模块复用。
// 曾因"拆结构体要引入 `Box<dyn Fn>`,得不偿失"没做 Builder 化——实际上
// 把两个闭包做成结构体自己的泛型参数(而非 trait object)就不用装箱,
// `PanelTabArgs` 就是这样做的:`on_select`/`on_close` 同为 `M`、
// `title_hover`/`close_hover`/`prefix`/`suffix` 各自同类型相邻,原先 11
// 个位置参数顺序传错编译器发现不了(Rust Design Patterns:Builder,用
// 具名字段替代同类型位置参数)。
/// tab 的"图标+文字"渲染,横向 tab(`panel_tab`)与下拉行(`tab_overflow_menu`)
/// 共用。`active`/`hover_t` 决定标题颜色(选中恒 CREAM,未选中 DIM→GOLD
/// 按 `hover_t` 插值,与原 `panel_tab` 配色公式一致)。
pub(crate) fn tab_label<'a, M: 'a>(
    prefix: Option<Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>>,
    title: String,
    active: bool,
    hover_t: f32,
    max_width: f32,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let title_color = if active {
        byteui::theme::color::current().cream
    } else {
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
            text(title)
                .font(top_bar_font())
                .size(byteui::theme::font::body())
                // iced `Text` 默认 `Wrapping::Word`,不是曾经以为的
                // `None`——不显式关掉,标题超宽时会真的折成两行,而不是
                // 靠下面 `.clip(true)` 单行截断(验收反馈:较长标题换行)。
                .wrapping(iced_widget::core::text::Wrapping::None)
                .color(title_color),
        )
        // 超宽不补省略号、不换行:`max_width` 只对下拉行(有行宽预算)起到
        // 截列作用;面板横向 tab 传 `NO_TAB_W_LIMIT`(≈无限)则不触发截列,
        // 标题完整展示,由外层行的 `.clip` 决定哪里裁掉。
        .width(Length::Shrink)
        .max_width(max_width)
        .clip(true),
    );
    title_row.into()
}

/// tab 容器的背景/边框:选中态 CARD 实底+1px 边框,未选中 hover 时
/// TAB_HOVER 胶囊背景,都不是则透明。从 `panel_tab` 抽取,`tab_overflow_menu`
/// 的行高亮复用同一份判断,避免两处各写一份容易分叉的样式逻辑。
pub(crate) fn tab_container_style(
    active: bool,
    hover: f32,
) -> impl Fn(&iced_widget::Theme) -> container::Style {
    move |_theme: &iced_widget::Theme| {
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
    }
}

pub(crate) struct PanelTabArgs<'a, M, F1, F2>
where
    M: Clone + 'a,
    F1: Fn(bool) -> M + 'a,
    F2: Fn(bool) -> M + 'a,
{
    pub title: String,
    pub active: bool,
    pub hover_t: f32,
    pub close_hover_t: f32,
    pub prefix: Option<Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>>,
    pub suffix: Option<Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>>,
    pub on_select: M,
    pub on_close: M,
    pub show_tooltip: bool,
    pub title_hover: F1,
    pub close_hover: F2,
}

pub(crate) fn panel_tab<'a, M, F1, F2>(
    args: PanelTabArgs<'a, M, F1, F2>,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>
where
    M: Clone + 'a,
    F1: Fn(bool) -> M + 'a,
    F2: Fn(bool) -> M + 'a,
{
    let PanelTabArgs {
        title,
        active,
        hover_t,
        close_hover_t,
        prefix,
        suffix,
        on_select,
        on_close,
        show_tooltip,
        title_hover,
        close_hover,
    } = args;
    let close_sz = byteui::theme::geometry::tab_button_size();
    // 组合 hover:悬停标题或 × 任一,胶囊背景都浮现、× 显形。
    let hover = hover_t.max(close_hover_t).clamp(0.0, 1.0);
    let hovered = hover > 0.001;
    // 标题宽度不设上限:tab 随标题实际内容伸缩。给 `tab_label` 一个足够大的
    // 名义上限即可对其不产生截断效果(真实上限由外层行 `.clip` 决定)。
    let title_row = tab_label(prefix, title.clone(), active, hover_t, NO_TAB_W_LIMIT);

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
    let (select, close) = tabs::tab_core(tabs::TabCoreArgs {
        content: title_row,
        close_sz,
        close_color,
        close_interactive: hovered,
        on_select,
        on_close,
        on_select_hover: title_hover,
        on_close_hover: close_hover,
    });

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
        .style(tab_container_style(active, hover))
        .into();
    // 面板页签在屏幕底部,tooltip 用 `Top` 弹在页签上方,免出屏。仅当悬停
    // 满 2s(`show_tooltip`)才显示标题全称(见 `App::hover_tooltip_ready`)。
    controlled_tooltip(el, title, Position::Top, show_tooltip)
}

/// 给定各 tab 宽、tab 间距、可视宽、当前 first,算出实际渲染窗口:
/// (钳制后的 first, 可见区间的独占结束下标)。
/// - 全部 tab 能放下(总宽<=avail) → first=0, visible_end=n(全可见,无溢出)。
/// - 溢出 → 先按原算法算 max_first(从右往左累加,找最大窗口起点使尾部放得下),
///   钳 first 到 [0, max_first];再从钳后的 first 往右累加,算出这一屏实际能
///   放下几个(`visible_end`)——原算法只钳 first,不知道"从 first 起到底能看见
///   几个",全靠调用方外层 `.clip(true)` 视觉裁切,拿不到索引,这次要靠这个
///   新窗口的可见区间构建"隐藏了哪些 tab"的列表,必须补上这个正向累加。
pub(crate) struct TabOverflow {
    pub first: usize,
    pub visible_end: usize,
}

// 该窗口结构体最终在 `tab_window_reveal` 里按是否落在窗口内决定是否松开
// 钳制。下面三组"隐藏段"取法只在既有单元测试里断言布局时用到(V 下拉不再
// 消费它们:新版下拉列出组内全部 tab,不区分子集),生产 build 里去重以免
// dead-code 告警,故整块挂在 `#[cfg(test)]` 下。
#[cfg(test)]
impl TabOverflow {
    pub(crate) fn hidden_before(&self) -> std::ops::Range<usize> {
        0..self.first
    }

    pub(crate) fn hidden_after(&self, len: usize) -> std::ops::Range<usize> {
        self.visible_end..len
    }

    pub(crate) fn has_overflow(&self, len: usize) -> bool {
        self.first > 0 || self.visible_end < len
    }
}

pub(crate) fn tab_window(widths: &[f32], gap: f32, avail: f32, first: usize) -> TabOverflow {
    let n = widths.len();
    if n == 0 {
        return TabOverflow {
            first: 0,
            visible_end: 0,
        };
    }
    let total: f32 = widths.iter().sum::<f32>() + gap * (n.saturating_sub(1)) as f32;
    if total <= avail {
        return TabOverflow {
            first: 0,
            visible_end: n,
        };
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
    // 从钳后的 first 往右累加，算这一屏实际放得下几个。
    let mut visible_end = clamped;
    let mut fwd = 0.0;
    for (i, w) in widths.iter().enumerate().skip(clamped) {
        let w = *w + if i > clamped { gap } else { 0.0 };
        if fwd + w <= avail {
            fwd += w;
            visible_end = i + 1;
        } else {
            break;
        }
    }
    TabOverflow {
        first: clamped,
        visible_end,
    }
}

/// 选中某个 tab(`target`)后:若它已经在当前窗口可见区间内,`first` 原样
/// 不变(避免"点已可见的 tab 也跟着跳一下"的抖动);若它当前隐藏(在窗口外),
/// 把候选 first 设为 `target` 本身,交给 `tab_window` 重新钳出一个包含它的
/// 窗口——这就是"自动滚动带入可见区"的全部逻辑,没有新算法,只是换个候选值
/// 重跑一次既有的钳制。
pub(crate) fn tab_window_reveal(
    widths: &[f32],
    gap: f32,
    avail: f32,
    first: usize,
    target: usize,
) -> usize {
    let current = tab_window(widths, gap, avail, first);
    if target >= current.first && target < current.visible_end {
        current.first
    } else {
        tab_window(widths, gap, avail, target).first
    }
}

/// 溢出下拉入口。V 按钮在 tab 组非空时始终显示(即使当前没有横向溢出,
/// 下拉也要列出**该组全部 tab**,供随时跳转)。仅当 tab 组为空(编号 0)时
/// 返回 `None` 让调用方跳过——不走"禁用态灰按钮"(见 spec 语义确认)。
/// 图标 `SquareChevronDown`,套 `icons::icon_button_entry` 标准图标按钮
/// (静止 DIM、hover 平滑过渡到 GOLD),与面板内其余图标按钮同一套视觉,
/// 不再自绘 hover 背景/边框;`hover_t`/`on_hover` 由调用方接自己那组专属
/// `HoverId`(如 `HoverId::TermTabOverflow`)——固定单按钮,不随 tab
/// 增减/拖拽换位漂移,不需要 `rekey_hover_range`。命中区尺寸沿用原先的
/// `tab_arrow_button_size`,保证换皮前后不跳动。
pub(crate) fn tab_overflow_button<'a, M: Clone + 'a>(
    tab_count: usize,
    hover_t: f32,
    on_press: M,
    on_hover: impl Fn(bool) -> M + 'a,
) -> Option<Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>> {
    if tab_count == 0 {
        return None;
    }
    Some(icons::icon_button_entry(
        icons::IconKind::SquareChevronDown,
        byteui::theme::icon_size::chevron(),
        false,
        false,
        hover_t,
        false,
        byteui::theme::geometry::tab_arrow_button_size(),
        true,
        on_press,
        on_hover,
        "展开全部标签页",
    ))
}

/// 预览/代码模式切换按钮:只在 `preview::wry_toggle_eligible` 为真的文件
/// tab 上画(调用方判断,这里只管渲染)。`in_code_mode` 决定图标——预览态
/// 显示 `FileCode`(点它切到代码),代码态显示 `Eye`(点它切回预览)。hover
/// 用 iced 内置 `button::Status`,不接入 `HoverId` 动画体系:这个按钮会随
/// tab 增减/拖拽换位下标漂移,`rekey_hover_range` 目前只接受两个 `HoverId`
/// 构造器(item/close),犯不着为它扩展签名(区别于 `tab_overflow_button`——
/// 后者是单个固定按钮,不随下标漂移,已改用 `HoverId` 动画体系)。
pub(crate) fn tab_render_mode_button<'a, M: Clone + 'a>(
    in_code_mode: bool,
    on_press: M,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let icon = if in_code_mode {
        icons::IconKind::Eye
    } else {
        icons::IconKind::FileCode
    };
    let color = byteui::theme::color::current().dim;
    let btn = button(icons::view(icon, byteui::theme::icon_size::row(), color))
        .width(Length::Fixed(byteui::theme::geometry::tab_button_size()))
        .height(Length::Fixed(byteui::theme::geometry::tab_button_size()))
        .padding(0)
        .on_press(on_press)
        .style(move |_theme, status| {
            let base = button::Style {
                background: None,
                text_color: color,
                ..button::Style::default()
            };
            match status {
                button::Status::Hovered | button::Status::Pressed => button::Style {
                    text_color: byteui::theme::color::current().gold,
                    ..base
                },
                _ => base,
            }
        });
    btn.into()
}

/// 悬浮下拉里的一行,对应该 tab 组里的**某个 tab**(V 菜单列的是组内全部
/// tab,不局限于当前横向被裁掉的)。`prefix` 与横向 tab 用同一个已经建好的
/// `Element`(状态点/图标/无),`active` 决定是否高亮——菜单里可能同时出现
/// 已经横向可见的当前选中项,高亮让用户看得出"这其实是当前那个"。
/// `closable=false` 用于 SSH/Database 的固定"空白"占位 tab(本来就不可关闭)。
pub(crate) struct TabOverflowEntry<'a, M> {
    pub index: usize,
    pub prefix: Option<Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>>,
    pub title: String,
    pub active: bool,
    pub closable: bool,
}

/// `tab_overflow_menu` 的参数:仿 `tabs::TabCoreArgs`/`tab_widget::PanelTabArgs`
/// 的具名字段结构体风格(闭包走结构体自身泛型参数,不用 `Box<dyn Fn>`)。
/// `anchor` 是点 V 按钮那一刻的 `App::last_cursor` 快照(逻辑坐标),
/// `window_size` 是当前窗口尺寸,两者一起用于把下拉钉在按钮附近同时不越出
/// 窗口边界。
pub(crate) struct TabOverflowMenuArgs<'a, M, FSel, FClose>
where
    M: Clone + 'a,
    FSel: Fn(usize) -> M,
    FClose: Fn(usize) -> M,
{
    pub entries: Vec<TabOverflowEntry<'a, M>>,
    pub anchor: (f32, f32),
    pub window_size: (f32, f32),
    pub on_select: FSel,
    pub on_close: FClose,
    pub on_dismiss: M,
}

const TAB_OVERFLOW_MENU_WIDTH: f32 = 220.0;
const TAB_OVERFLOW_MENU_MAX_HEIGHT: f32 = 320.0;

/// 悬浮下拉列表:每行用既有的 `tabs::tab_core` 包装选中(mousedown)/关闭(×)
/// 交互(CLAUDE.md 裁决:tab 类 UI 优先复用 `tab_core`,不手写
/// `MouseArea`+`on_enter`/`on_exit`)。关闭按钮**始终可点**(不像横向 tab 那样
/// hover 才显形)——这是刻意的自定义(见截图:下拉里的 x 是常显的,不是
/// hover-only),因为悬浮列表本就是"已经主动点开来看"的场景,hover-only 反而
/// 多一次交互成本。行内不做 hover 动画(`title_hover`/`close_hover` 两个
/// 回调固定传 `on_dismiss.clone()` 以外的**不产生副作用**的消息——各调用点
/// 传入自己那套 `Message` 里已有的 no-op 变体,terminal/preview/ssh 用顶层
/// `Message::Noop`,database 用新增的 `database::Message::Noop`,详见各自
/// 任务),避免为一个短生命周期的浮层再铺一整套 `HoverId` 动画状态。
///
/// 悬浮定位:向上弹(同已移除的预览 tab 右键菜单——文件/项目预览的 tab 栏下方是
/// wry webview 子视图,webview 恒在 iced 内容之上,向下弹会被盖住;为了让
/// 5 处调用点共用同一套定位逻辑、不用按面板特判,统一向上弹),同时按
/// `project_add_menu_popup` 的手法钳一次 x/y 防止超出窗口右/下边缘。
pub(crate) fn tab_overflow_menu<'a, M, FSel, FClose>(
    args: TabOverflowMenuArgs<'a, M, FSel, FClose>,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>
where
    M: Clone + 'a,
    FSel: Fn(usize) -> M + 'a,
    FClose: Fn(usize) -> M + 'a,
{
    let TabOverflowMenuArgs {
        entries,
        anchor,
        window_size,
        on_select,
        on_close,
        on_dismiss,
    } = args;
    let close_sz = byteui::theme::geometry::tab_button_size();
    let row_max_w = TAB_OVERFLOW_MENU_WIDTH - close_sz - 16.0;

    let mut rows: Vec<Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>> = Vec::new();
    for entry in entries {
        let idx = entry.index;
        let label = tab_label(entry.prefix, entry.title, entry.active, 0.0, row_max_w);
        let close_color = byteui::theme::color::current().dim;
        let (select, close) = tabs::tab_core(tabs::TabCoreArgs {
            content: label,
            close_sz,
            close_color,
            close_interactive: entry.closable,
            on_select: (on_select)(idx),
            on_close: (on_close)(idx),
            on_select_hover: {
                let d = on_dismiss.clone();
                move |_h| d.clone()
            },
            on_close_hover: {
                let d = on_dismiss.clone();
                move |_h| d.clone()
            },
        });
        let row_content = row![select, close]
            .spacing(4)
            .align_y(iced_widget::core::Alignment::Center);
        rows.push(
            container(row_content)
                .width(Length::Fill)
                .padding(Padding::new(4.0))
                .style(tab_container_style(entry.active, 0.0))
                .into(),
        );
    }

    let list = crate::menu::shell(
        vec![
            scrollable(column(rows).spacing(2))
                .height(Length::Shrink)
                .into(),
        ],
        Length::Fixed(TAB_OVERFLOW_MENU_WIDTH),
    );
    let list = container(list).max_height(TAB_OVERFLOW_MENU_MAX_HEIGHT);

    // 全屏透明遮罩,接住"点外部关闭"(同已移除的预览 tab 右键菜单与
    // `project_add_menu_popup` 的既有套路)。内容本身不画任何东西(空白
    // Space),让容器撑满全窗。
    let dismiss = MouseArea::new(
        container(iced_widget::Space::new())
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(on_dismiss.clone());

    let (ax, ay) = anchor;
    let (window_w, window_h) = window_size;
    let x = ax.min((window_w - TAB_OVERFLOW_MENU_WIDTH).max(0.0));
    let bottom = (window_h - ay).max(0.0);
    let positioned = container(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(iced_widget::core::alignment::Vertical::Bottom)
        .padding(Padding {
            top: 0.0,
            left: x,
            right: 0.0,
            bottom,
        });

    stack![dismiss, positioned].into()
}
