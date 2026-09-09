# Tab 组溢出下拉 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 5 处 tab 组(终端会话、文件预览、项目预览、SSH、Database 连接)现有的 `<`/`>` 翻页箭头,全部换成"tab 组右侧一个仅在溢出时出现的 V 按钮 + 悬浮下拉列表(只列隐藏 tab,可选中/可关闭,选中后主条自动滚动带入可见区)"。

**Architecture:** 在 `crates/dozer-app/src/tab_widget.rs` 抽出一套共享基础设施——`tab_window` 的返回值从 `(first, can_left, can_right)` 换成暴露"隐藏区间"的 `TabOverflow`;新增纯函数 `tab_window_reveal`(选中一个隐藏 tab 时重新钳出包含它的窗口,已可见则不动,避免误触发抖动);新增 `tab_label`(从 `panel_tab` 抽取的图标+文字渲染,横向 tab 与下拉行共用)、`tab_overflow_button`(仅溢出时渲染的 V 按钮)、`tab_overflow_menu`(悬浮下拉列表,每行用既有的 `tabs::tab_core` 包装选中/关闭交互)。5 个调用点(`terminal.rs`/`workspace.rs`/`app.rs::ssh_tab_bar`/`extensions/database.rs`)各自接入这套基础设施,替换掉原来的箭头按钮 + 手动翻页 Message。悬浮定位复用 `App::last_cursor`(点 V 那一刻的光标快照,同 `project_add_menu_anchor` 手法)+ 向上弹出(同 `PreviewTabMenu` 的坐标换算,规避文件/项目预览 tab 栏下方 wry webview 恒在 iced 内容之上、向下弹会被盖住的问题)。

**Tech Stack:** Rust workspace,iced 0.14(`iced_widget`/`iced_renderer`),项目自有 `byteui` 组件库(`interaction::{icons, tabs}`、`theme::{geometry, icon_size, color, font}`)。

**Spec:** `docs/superpowers/specs/2026-09-09-tab-overflow-dropdown-design.md`

## Global Constraints

- 5 处调用点全部替换,不留混用状态(spec 目标 1)。
- V 按钮仅在存在溢出时渲染;不溢出时完全不出现,不做禁用态灰按钮(spec 目标 2、关键语义确认 4)。
- 下拉列表只列当前被挤出去看不到的 tab,不重复列出已可见的(spec 目标 3、关键语义确认 2)。
- 选中下拉里的隐藏 tab 后,主 tab 条的可见窗口自动滚动把它带入视野并高亮,下拉自动收起;点击已可见的 tab 不应触发多余滚动(spec 目标 4、关键语义确认 3)。
- 下拉每一项都能直接点 x 关闭对应 tab;SSH/Database 的空白占位 tab 若被挤入隐藏列表,不出现关闭按钮(spec 目标 5、非目标)。
- 点击下拉外部区域收起下拉(spec 目标 6)。
- `extensions/browser.rs` 不在本次范围(spec 非目标)。
- 新增 UI 一律走 `icons::icon_button_entry`/`tabs::tab_core` 等既有共享组件,不手写 `MouseArea`+`on_enter`/`on_exit` 三件套(CLAUDE.md 关键裁决)。
- 不新增 App fixture 级自动化 GUI 测试;新增测试限于纯逻辑单测(spec 测试策略)。

---

### Task 1: 共享基础设施(`tab_widget.rs`)

**Files:**
- Modify: `crates/dozer-app/src/tab_widget.rs`(改造 `tab_window` 返回值、抽取 `tab_label`、新增 `tab_overflow_button`/`tab_overflow_menu`/`tab_window_reveal`)
- Modify: `crates/dozer-app/src/workspace.rs:5156-5168`(更新 `tab_window` 相关单测)

**Interfaces:**
- Produces(后续 4 个任务都依赖这些新增/改造的公开 API):
  - `pub(crate) struct TabOverflow { pub first: usize, pub visible_end: usize }`,方法 `hidden_before() -> Range<usize>`、`hidden_after(len: usize) -> Range<usize>`、`has_overflow(len: usize) -> bool`
  - `pub(crate) fn tab_window(widths: &[f32], gap: f32, avail: f32, first: usize) -> TabOverflow`(签名不变,返回类型变了)
  - `pub(crate) fn tab_window_reveal(widths: &[f32], gap: f32, avail: f32, first: usize, target: usize) -> usize`
  - `pub(crate) fn tab_label<'a, M: 'a>(prefix: Option<Element<'a, M, Theme, Renderer>>, title: String, active: bool, hover_t: f32, max_width: f32) -> Element<'a, M, Theme, Renderer>`
  - `pub(crate) fn tab_container_style(active: bool, hover: f32) -> impl Fn(&Theme) -> container::Style`(从 `panel_tab` 抽取的背景/边框样式判断,`tab_overflow_menu` 的行高亮复用)
  - `pub(crate) fn tab_overflow_button<M: Clone + 'a>(hidden_count: usize, on_press: M) -> Option<Element<'a, M, Theme, Renderer>>`
  - `pub(crate) struct TabOverflowEntry<'a, M> { pub index: usize, pub prefix: Option<Element<'a, M, Theme, Renderer>>, pub title: String, pub active: bool, pub closable: bool }`
  - `pub(crate) struct TabOverflowMenuArgs<'a, M, FSel, FClose> { pub entries: Vec<TabOverflowEntry<'a, M>>, pub anchor: (f32, f32), pub window_size: (f32, f32), pub on_select: FSel, pub on_close: FClose, pub on_dismiss: M }`
  - `pub(crate) fn tab_overflow_menu<'a, M, FSel, FClose>(args: TabOverflowMenuArgs<'a, M, FSel, FClose>) -> Element<'a, M, Theme, Renderer>`

- [ ] **Step 1: 改造 `tab_window` 返回值,加 `TabOverflow`/`hidden_before`/`hidden_after`/`has_overflow`**

把 `crates/dozer-app/src/tab_widget.rs:266-299` 的 `tab_window` 整段换成:

```rust
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
    for i in clamped..n {
        let w = widths[i] + if i > clamped { gap } else { 0.0 };
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
pub(crate) fn tab_window_reveal(widths: &[f32], gap: f32, avail: f32, first: usize, target: usize) -> usize {
    let current = tab_window(widths, gap, avail, first);
    if target >= current.first && target < current.visible_end {
        current.first
    } else {
        tab_window(widths, gap, avail, target).first
    }
}
```

- [ ] **Step 2: 更新 `workspace.rs` 里的 `tab_window` 单测**

把 `crates/dozer-app/src/workspace.rs:5155-5168` 的两个测试换成:

```rust
    #[test]
    fn tab_window_no_overflow_all_visible() {
        let w = tab_window(&[50.0, 50.0, 50.0], 4.0, 500.0, 0);
        assert_eq!((w.first, w.visible_end), (0, 3));
        assert!(!w.has_overflow(3));
    }

    #[test]
    fn tab_window_overflow_clamps_and_computes_visible_end() {
        let widths = [100.0; 5];
        let w = tab_window(&widths, 0.0, 250.0, 0);
        assert_eq!((w.first, w.visible_end), (0, 2));
        assert!(w.has_overflow(5));
        assert_eq!(w.hidden_before(), 0..0);
        assert_eq!(w.hidden_after(5), 2..5);

        // 请求的 first 越界 → 钳到 max_first(=3),此时尾部 3 个恰好全可见。
        let w = tab_window(&widths, 0.0, 250.0, 99);
        assert_eq!((w.first, w.visible_end), (3, 5));
        assert_eq!(w.hidden_before(), 0..3);
        assert_eq!(w.hidden_after(5), 5..5);

        let w = tab_window(&widths, 0.0, 250.0, 1);
        assert_eq!((w.first, w.visible_end), (1, 3));
        assert_eq!(w.hidden_before(), 0..1);
        assert_eq!(w.hidden_after(5), 3..5);
    }

    #[test]
    fn tab_window_reveal_keeps_visible_tab_still_no_jump() {
        let widths = [100.0; 5];
        // first=1 时可见区间是 [1,3):选中已经可见的 tab 1,first 不应该变。
        assert_eq!(tab_window_reveal(&widths, 0.0, 250.0, 1, 1), 1);
    }

    #[test]
    fn tab_window_reveal_scrolls_hidden_tab_into_view() {
        let widths = [100.0; 5];
        // first=0 时可见区间是 [0,2):选中隐藏在右侧的 tab 4,应重新钳出
        // 一个包含它的窗口。
        let new_first = tab_window_reveal(&widths, 0.0, 250.0, 0, 4);
        let w = tab_window(&widths, 0.0, 250.0, new_first);
        assert!((w.first..w.visible_end).contains(&4));
    }
```

- [ ] **Step 3: 跑测试确认新逻辑正确**

Run: `cargo test -p dozer-app tab_window --lib`
Expected: 编译失败(因为 `panel_tab`/`terminal.rs`/`workspace.rs`/`app.rs`/`database.rs` 里所有 `tab_window(...)` 调用点还在用旧的三元组解构 `let (first, can_left, can_right) = tab_window(...)`,类型对不上)。这是预期的中间态——Step 4 会先把这些调用点改成只解构 `.first`(暂不处理箭头按钮,箭头按钮的 `can_left`/`can_right` 调用先保留、临时硬编码 `true`,箭头按钮本身在 Task 2-5 才会被删除)。

- [ ] **Step 4: 临时打通编译(4 个调用点的箭头逻辑先不删,只改解构方式)**

在 `terminal.rs:116`、`workspace.rs:3627`、`app.rs:9528`、`database.rs:2157` 这 4 处,把:

```rust
let (first, can_left, can_right) = tab_widget::tab_window(&widths, 4.0, ..., xxx_tab_first);
```

改成:

```rust
let window = tab_widget::tab_window(&widths, 4.0, ..., xxx_tab_first);
let first = window.first;
let can_left = first > 0;
let can_right = window.visible_end < widths.len();
```

(`database.rs` 用 `crate::tab_widget::tab_window`,其余同名路径不变;`can_left`/`can_right` 这两行是过渡态,箭头按钮和这两个变量会在 Task 2-5 里随各自调用点一起删掉,这里只是让 Task 1 能独立编译通过。)

Run: `cargo test -p dozer-app tab_window --lib`
Expected: PASS(4 个新测试 + 原有测试全绿)

- [ ] **Step 5: 从 `panel_tab` 抽取 `tab_label` + `tab_container_style`**

把 `tab_widget.rs:109-132`(`title_row` 构建)抽成:

```rust
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
                .wrapping(iced_widget::core::text::Wrapping::None)
                .color(title_color),
        )
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
```

然后把 `panel_tab` 函数体里原来内联构建 `title_row` 的那段(109-132 行)删掉,换成:

```rust
    let title_row = tab_label(prefix, title.clone(), active, hover_t, title_max);
```

`panel_tab` 里原来 166-206 行 `container(tab_row)...style(move |_t: &iced_widget::Theme| { ... })` 那一大段 if/else 换成:

```rust
    let el: Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> = container(tab_row)
        .padding(Padding {
            top: PANEL_TAB_PAD_Y,
            right: PANEL_TAB_PAD_X,
            bottom: PANEL_TAB_PAD_Y,
            left: PANEL_TAB_PAD_LEFT,
        })
        .width(Length::Shrink)
        .max_width(PANEL_TAB_MAX_W)
        .style(tab_container_style(active, hover))
        .into();
```

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 编译通过,`panel_tab` 渲染出的横向 tab 视觉不变(纯提取,无行为改动)。

- [ ] **Step 6: 新增 `tab_overflow_button`**

在 `tab_widget.rs` 末尾追加(紧跟 `tab_arrow_button` 之后,视觉尺寸复用同一套 geometry/icon_size 常量,保持与箭头按钮一致的 hover/颜色手感):

```rust
/// 溢出下拉入口:仅当 `hidden_count > 0` 时渲染,否则返回 `None`——调用方
/// 直接跳过这一项,不走"禁用态灰按钮"(见 spec 关键语义确认 4)。尺寸/颜色
/// 复用 `tab_arrow_button` 同一套 geometry/icon_size 常量,保证视觉一致。
pub(crate) fn tab_overflow_button<'a, M: Clone + 'a>(
    hidden_count: usize,
    on_press: M,
) -> Option<Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>> {
    if hidden_count == 0 {
        return None;
    }
    let color = byteui::theme::color::current().tab_active_border;
    let btn = button(icons::view(
        icons::IconKind::ChevronDown,
        byteui::theme::icon_size::chevron(),
        color,
    ))
    .width(Length::Fixed(byteui::theme::geometry::tab_arrow_button_size()))
    .height(Length::Fixed(byteui::theme::geometry::tab_arrow_button_size()))
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
    Some(btn.into())
}
```

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: PASS(此函数目前还没有调用方,`dead_code` 警告是预期的,Task 2 起会消掉)

- [ ] **Step 7: 新增 `TabOverflowEntry`/`TabOverflowMenuArgs`/`tab_overflow_menu`**

在 `tab_widget.rs` 末尾追加:

```rust
/// 悬浮下拉里的一行,对应一个"当前被挤出可见区看不到"的 tab。`prefix` 与
/// 横向 tab 用同一个已经建好的 `Element`(状态点/图标/无),`active` 决定
/// 是否高亮(理论上活动 tab 不该被挤出去,但初次加载等边界场景仍可能发生,
/// 高亮让用户看得出"这其实是当前选中的那个")。`closable=false` 用于
/// SSH/Database 的固定"空白"占位 tab(它本来就不可关闭)。
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
/// 悬浮定位:向上弹(同 `PreviewTabMenu`——文件/项目预览的 tab 栏下方是
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
        let on_dismiss_for_hover = on_dismiss.clone();
        let (select, close) = tabs::tab_core(tabs::TabCoreArgs {
            content: label,
            close_sz,
            close_color,
            close_interactive: entry.closable,
            on_select: (on_select)(idx),
            on_close: (on_close)(idx),
            on_select_hover: move |_h| on_dismiss_for_hover.clone(),
            on_close_hover: move |_h| on_dismiss.clone(),
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
            iced_widget::scrollable(iced_widget::column(rows).spacing(2))
                .height(Length::Shrink)
                .into(),
        ],
        Length::Fixed(TAB_OVERFLOW_MENU_WIDTH),
    );
    let list = container(list).max_height(TAB_OVERFLOW_MENU_MAX_HEIGHT);

    // 全屏透明遮罩,接住"点外部关闭"(同 `PreviewTabMenu`/`project_add_menu_popup`
    // 的既有套路)。
    let dismiss = iced_widget::MouseArea::new(
        container(iced_widget::Space::new(Length::Fill, Length::Fill))
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

    iced_widget::stack![dismiss, positioned].into()
}
```

在文件顶部 `use` 区块补上这几个新用到的类型:

```rust
use iced_widget::core::Padding as _; // 若已引入 Padding 可跳过,当前文件已 `use ... Padding` 见开头
```

(实际只需确认 `Padding`/`container`/`row`/`button`/`text` 已在文件顶部 `use` 列表里——现有 `use` 已覆盖,新增的只有 `iced_widget::{scrollable, column, stack, MouseArea, Space}`,按文件现有 `use iced_widget::{button, container, row, text};` 那一行追加进去。)

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: PASS,可能有 `dead_code` 警告(尚无调用方,Task 2-5 会消掉)。

- [ ] **Step 8: `cargo clippy` + `cargo fmt` + 提交**

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | tail -60 && cargo fmt`
Expected: 无新增 clippy 报错(允许已有的 `dead_code` warning,下个任务起会消)

```bash
git add crates/dozer-app/src/tab_widget.rs crates/dozer-app/src/workspace.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): tab_widget 新增溢出下拉共享基础设施

tab_window 返回值从 (first, can_left, can_right) 换成暴露隐藏区间的
TabOverflow;新增 tab_window_reveal(选中隐藏 tab 时自动滚动,已可见则不
动)、tab_label/tab_container_style(从 panel_tab 抽取,横向 tab 与下拉行
共用)、tab_overflow_button(仅溢出时渲染的 V 按钮)、tab_overflow_menu
(悬浮下拉,行内复用 tabs::tab_core)。4 个既有调用点先做最小改动打通编译,
真正接入在后续任务。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01GNaJ3sH2f2tP2cPRBQ4qri
EOF
)"
```

---

### Task 2: 终端会话 tab

**Files:**
- Modify: `crates/dozer-app/src/terminal.rs:107-180`(`tab_bar`)
- Modify: `crates/dozer-app/src/app.rs`(Message 枚举 `TermTabScroll` 附近 ~1812-1814;`Message::TermTabScroll`/`Message::SelectTab` 处理分支 ~5116-5124;`Workspace` 结构体 `term_tab_first` 字段附近 ~446;`App` 结构体新增锚点字段处,参照 `project_add_menu_anchor` 字段 ~2231)

**Interfaces:**
- Consumes: Task 1 的 `tab_widget::{TabOverflow, tab_window, tab_window_reveal, tab_overflow_button, tab_overflow_menu, TabOverflowEntry, TabOverflowMenuArgs}`;`App::last_cursor: (f32, f32)`(既有字段);`App::window_size: (f32, f32)`(既有字段)
- Produces: 无(叶子调用点,后续任务不依赖它)

- [ ] **Step 1: `Workspace` 新增锚点字段**

在 `crates/dozer-app/src/workspace.rs:446` 附近(`term_tab_first` 声明处)追加:

```rust
    /// 终端 tab 栏溢出下拉的悬浮锚点:`None`=下拉关闭,`Some(x,y)`=打开且
    /// 记录了点 V 按钮那一刻的 `App::last_cursor` 快照(逻辑坐标),同
    /// `project_add_menu_anchor` 手法。
    pub(crate) term_tab_overflow_anchor: Option<(f32, f32)>,
```

并在 `Workspace` 的 `Default`/构造处(`term_tab_first: 0,` 所在的初始化块,约 671 行)追加:

```rust
            term_tab_overflow_anchor: None,
```

Run: `cargo build -p dozer-app 2>&1 | tail -30`
Expected: 报 `Workspace` 构造处缺字段的编译错——按报错位置补全(可能不止一处初始化,项目里 `Workspace` 若有多个构造点,逐一补 `term_tab_overflow_anchor: None,`)。

- [ ] **Step 2: Message 枚举——删 `TermTabScroll`,加 `TermTabOverflowToggle`/`TermTabOverflowDismiss`**

把 `app.rs:1812-1814` 的:

```rust
    /// 终端 tab 栏箭头翻页（`true`=右/`false`=左）。一次翻 2 个 tab；
    /// 上界不在此钳，渲染时 `tab_window` 钳制显示（P1L T5 验收返工）。
    TermTabScroll(bool),
```

换成:

```rust
    /// 终端 tab 栏溢出下拉开关:点 V 按钮切换;打开时把 `App::last_cursor`
    /// 记进 `Workspace::term_tab_overflow_anchor` 作为悬浮定位锚点。
    TermTabOverflowToggle,
    /// 终端 tab 栏溢出下拉:点击外部区域关闭。
    TermTabOverflowDismiss,
```

- [ ] **Step 3: `App::update` 里替换 `TermTabScroll` 分支,`SelectTab` 分支追加自动滚动**

把 `app.rs:5116-5124` 的 `Message::TermTabScroll(right) => {...}` 整段换成:

```rust
            Message::TermTabOverflowToggle => {
                let last_cursor = self.last_cursor;
                self.with_focused_project(|ws, _io| {
                    ws.term_tab_overflow_anchor = if ws.term_tab_overflow_anchor.is_some() {
                        None
                    } else {
                        Some(last_cursor)
                    };
                });
            }
            Message::TermTabOverflowDismiss => {
                self.with_focused_project(|ws, _io| {
                    ws.term_tab_overflow_anchor = None;
                });
            }
```

找到 `Message::SelectTab(idx)` 的处理分支(terminal.rs:216 是渲染侧发消息处,实际 `update` 分支在 `app.rs` 里搜索 `Message::SelectTab(idx) =>`),在里面设置 `ws.active = idx` 之后追加:

```rust
                    let widths: Vec<f32> = ws
                        .tabs
                        .iter()
                        .map(|t| terminal::tab_display_width(&terminal::tab_title(t.agent, t.cwd.as_deref(), &t.info.name)))
                        .collect();
                    ws.term_tab_first = tab_widget::tab_window_reveal(
                        &widths,
                        4.0,
                        byteui::theme::geometry::tab_bar_avail_px(),
                        ws.term_tab_first,
                        idx,
                    );
                    ws.term_tab_overflow_anchor = None;
```

(`terminal::tab_display_width`/`terminal::tab_title` 若不是 `pub(crate)` 可见,按报错改成 `pub(crate)`——这两个函数目前只在 `terminal.rs` 内部用,`app.rs` 要跨模块调用需要可见性;找到它们的定义处直接加 `pub(crate)` 前缀即可,不改签名/实现。)

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: 报 `terminal.rs`/`terminal_pane`/`ssh_tab_bar` 等处 `Message::TermTabScroll` 找不到的编译错(还没改渲染侧)——继续 Step 4。

- [ ] **Step 4: 改造 `terminal.rs::tab_bar` 渲染**

把 `terminal.rs:116-121` 的:

```rust
    let (first, can_left, can_right) = tab_widget::tab_window(
        &widths,
        4.0,
        byteui::theme::geometry::tab_bar_avail_px(),
        ws.term_tab_first,
    );
```

换成:

```rust
    let window = tab_widget::tab_window(
        &widths,
        4.0,
        byteui::theme::geometry::tab_bar_avail_px(),
        ws.term_tab_first,
    );
```

把 `terminal.rs:123-127` 的 `.filter(|(idx, _)| *idx >= first)` 换成 `.filter(|(idx, _)| (window.first..window.visible_end).contains(idx))`(不再渲染窗口外的 tab,而不是像原来那样渲染到底靠 `clip` 裁——这样后面才能正确统计"隐藏了多少个")。

把 `terminal.rs:148-160`(`clipped`/`left_arrow`/`right_arrow` 那三行)换成:

```rust
    let tabs_row = row(items).spacing(4);
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    let hidden_count = window.hidden_before().len() + window.hidden_after(ws.tabs.len()).len();
    let overflow_button = tab_widget::tab_overflow_button(hidden_count, Message::TermTabOverflowToggle);
```

把 `terminal.rs:162-177` 的 `tab_row` 组装:

```rust
    let mut tab_row = row![clipped].spacing(4).align_y(iced_widget::core::Alignment::Center);
    if let Some(btn) = overflow_button {
        tab_row = tab_row.push(btn);
    }
    tab_row = tab_row.push(app.list_collapse_button(
        PanelKind::Agent,
        app.list_collapsed(PanelKind::Agent),
        HoverId::AgentListCollapse,
        "收起列表",
        "展开列表",
        Message::TogglePanelListCollapse(PanelKind::Agent),
        move |hovered| Message::Hover(HoverId::AgentListCollapse, hovered),
    ));
```

在 `tab_bar` 函数最后 `column![tab_row, tab_divider()].spacing(4).into()` 之前,插入下拉浮层的叠加:

```rust
    let base = column![tab_row, tab_divider()].spacing(4);
    if let Some(anchor) = ws.term_tab_overflow_anchor {
        if window.has_overflow(ws.tabs.len()) {
            let entries: Vec<tab_widget::TabOverflowEntry<'_, Message>> = ws
                .tabs
                .iter()
                .enumerate()
                .filter(|(idx, _)| !(window.first..window.visible_end).contains(idx))
                .map(|(idx, tab)| tab_widget::TabOverflowEntry {
                    index: idx,
                    prefix: Some(byteui::feedback::status::dot(dot_color(tab.agent_state, tab.alive))),
                    title: tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name),
                    active: idx == ws.active,
                    closable: true,
                })
                .collect();
            let menu = tab_widget::tab_overflow_menu(tab_widget::TabOverflowMenuArgs {
                entries,
                anchor,
                window_size: app.window_size,
                on_select: Message::SelectTab,
                on_close: Message::CloseTab,
                on_dismiss: Message::TermTabOverflowDismiss,
            });
            return iced_widget::stack![base, menu].into();
        }
    }
    base.into()
```

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: PASS

- [ ] **Step 5: 手动验证**

Run: `cargo run -p dozer-app`

1. 开一个项目,连续新建终端会话(Agent 面板"＋")直到 tab 数超过可视宽度。
2. 确认 V 按钮在溢出时出现,不溢出时不出现。
3. 点 V,确认下拉只列被挤出去的 tab,带状态点+名称。
4. 点下拉里某一项,确认对应会话被选中、主条自动滚动把它带入可见区并高亮、下拉自动收起。
5. 点已经可见的 tab(不经过下拉),确认主条不会莫名滚动/跳动。
6. 点下拉某一项的 x,确认对应会话被关闭,下拉列表随之更新。
7. 打开下拉后点其它区域,确认下拉收起。
8. 关闭 tab 直到不再溢出,确认 V 按钮消失(若下拉还开着,应一并消失,不留游离浮层)。

- [ ] **Step 6: 提交**

```bash
git add crates/dozer-app/src/terminal.rs crates/dozer-app/src/workspace.rs crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): 终端会话 tab 溢出改用 V 下拉替换左右箭头

TermTabScroll 换成 TermTabOverflowToggle/Dismiss;SelectTab 追加
tab_window_reveal 自动滚动,已可见 tab 不受影响。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01GNaJ3sH2f2tP2cPRBQ4qri
EOF
)"
```

---

### Task 3: 文件预览 + 项目预览 tab

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs:3556-3720`(`preview_pane_for` 及其两个包装 `preview_pane`/`project_preview_pane`)
- Modify: `crates/dozer-app/src/app.rs`(Message 枚举 `PreviewTabScroll`/`ProjectPreviewTabScroll` ~1816/1930;对应 `update` 分支 ~5125-5133/5289-5299;`preview_select_tab`/`project_preview_select_tab` 方法 ~7189 起;`Workspace` 字段 `preview_tab_first`/`project_preview_tab_first` 附近 ~448-451)

**Interfaces:**
- Consumes: Task 1 的共享基础设施;`preview.tabs()`/`PreviewTab.title`(既有 API,`preview.rs`)
- Produces: 无

- [ ] **Step 1: `Workspace` 新增两个锚点字段**

在 `workspace.rs:448-451` 附近追加(紧跟 `preview_tab_first`/`project_preview_tab_first` 之后):

```rust
    /// 文件预览 tab 栏溢出下拉锚点,语义同 `term_tab_overflow_anchor`。
    pub(crate) preview_tab_overflow_anchor: Option<(f32, f32)>,
    /// 项目预览 tab 栏溢出下拉锚点,语义同上。
    pub(crate) project_preview_tab_overflow_anchor: Option<(f32, f32)>,
```

构造处(`preview_tab_first: 0,`/`project_preview_tab_first: 0,` 所在初始化块)追加对应的 `None,`。

- [ ] **Step 2: Message 枚举替换**

把 `app.rs:1816` 的 `PreviewTabScroll(bool)` 和它的文档注释,换成:

```rust
    /// 文件预览 tab 栏溢出下拉开关,语义同 `TermTabOverflowToggle`。
    PreviewTabOverflowToggle,
    /// 文件预览 tab 栏溢出下拉:点击外部关闭。
    PreviewTabOverflowDismiss,
```

把 `app.rs:1930` 的 `ProjectPreviewTabScroll(bool)` 和它的文档注释,换成:

```rust
    /// Project 面板右配对预览 tab 栏溢出下拉开关,语义同上。
    ProjectPreviewTabOverflowToggle,
    /// Project 面板右配对预览 tab 栏溢出下拉:点击外部关闭。
    ProjectPreviewTabOverflowDismiss,
```

- [ ] **Step 3: `App::update` 分支替换**

把 `app.rs:5125-5133` 的 `Message::PreviewTabScroll(right) => {...}` 换成:

```rust
            Message::PreviewTabOverflowToggle => {
                let last_cursor = self.last_cursor;
                self.with_focused_project(|ws, _io| {
                    ws.preview_tab_overflow_anchor = if ws.preview_tab_overflow_anchor.is_some() {
                        None
                    } else {
                        Some(last_cursor)
                    };
                });
            }
            Message::PreviewTabOverflowDismiss => {
                self.with_focused_project(|ws, _io| {
                    ws.preview_tab_overflow_anchor = None;
                });
            }
```

把 `app.rs:5289-5299` 的 `Message::ProjectPreviewTabScroll(right) => {...}` 换成同样形状,字段换成 `project_preview_tab_overflow_anchor`。

- [ ] **Step 4: `preview_select_tab`/`project_preview_select_tab` 追加自动滚动**

在 `app.rs:7189` 起的 `preview_select_tab` 方法里,`ws.preview.select(idx);` 之后追加:

```rust
            let widths: Vec<f32> = ws
                .preview
                .tabs()
                .iter()
                .map(|t| preview_tab_display_width(&t.title))
                .collect();
            ws.preview_tab_first = tab_widget::tab_window_reveal(
                &widths,
                4.0,
                byteui::theme::geometry::tab_bar_avail_px(),
                ws.preview_tab_first,
                idx,
            );
            ws.preview_tab_overflow_anchor = None;
```

在 `project_preview_select_tab`(与 `preview_select_tab` 同结构,函数名类推,`self.with_focused_project` 里搜 `Message::ProjectPreviewSelectTab(idx) => self.project_preview_select_tab(idx),` 定位到方法定义)里做镜像改动,字段换成 `project_preview`/`project_preview_tab_first`/`project_preview_tab_overflow_anchor`。

- [ ] **Step 5: 改造 `preview_pane_for` 渲染**

在 `workspace.rs:3572-3579` 的 `match kind` 元组解构里,把 `tab_first` 一起取出的地方扩展成同时取 anchor:

```rust
    let (preview, tab_first, error, overflow_anchor) = match kind {
        PreviewPaneKind::Files => (
            &ws.preview,
            ws.preview_tab_first,
            &ws.preview_error,
            ws.preview_tab_overflow_anchor,
        ),
        PreviewPaneKind::Project => (
            &ws.project_preview,
            ws.project_preview_tab_first,
            &ws.project_preview_error,
            ws.project_preview_tab_overflow_anchor,
        ),
    };
```

在 `scroll_msg` 闭包(`workspace.rs:3602-3605`)旁边新增两个闭包并删掉 `scroll_msg`(不再需要翻页消息):

```rust
    let overflow_toggle_msg = move || match kind {
        PreviewPaneKind::Files => Message::PreviewTabOverflowToggle,
        PreviewPaneKind::Project => Message::ProjectPreviewTabOverflowToggle,
    };
    let overflow_dismiss_msg = move || match kind {
        PreviewPaneKind::Files => Message::PreviewTabOverflowDismiss,
        PreviewPaneKind::Project => Message::ProjectPreviewTabOverflowDismiss,
    };
```

把 `workspace.rs:3627-3632` 的 `let (first, can_left, can_right) = tab_window(...)` 换成:

```rust
    let window = tab_window(&widths, 4.0, byteui::theme::geometry::tab_bar_avail_px(), tab_first);
```

把 `workspace.rs:3638` 的 `.filter(|(idx, _)| *idx >= first)` 换成 `.filter(|(idx, _)| (window.first..window.visible_end).contains(idx))`。

把 `workspace.rs:3684-3686`(`clipped`/`left_arrow`/`right_arrow`)换成:

```rust
    let tabs_row = row(items).spacing(4);
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    let hidden_count = window.hidden_before().len() + window.hidden_after(preview.tabs().len()).len();
    let overflow_button = tab_overflow_button(hidden_count, overflow_toggle_msg());
```

把 `workspace.rs:3711` 的 `let tab_bar = row![left_arrow, right_arrow, clipped, collapse]...` 换成:

```rust
    let mut tab_bar_row = row![clipped].spacing(4).align_y(iced_widget::core::Alignment::Center);
    if let Some(btn) = overflow_button {
        tab_bar_row = tab_bar_row.push(btn);
    }
    let tab_bar = tab_bar_row.push(collapse);
```

函数末尾原本是(`workspace.rs:4119-4128`,读代码确认过,就是这几行,不是别的表达式):

```rust
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
```

把这一段换成(先把原表达式存进 `base`,再按 `overflow_anchor`/溢出情况决定是否叠加下拉菜单):

```rust
    let base: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        container(content.padding(region.padding))
            .width(width)
            .height(Length::Fill)
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: region.background.map(Into::into),
                border: outer,
                ..container::Style::default()
            })
            .into();
    if let Some(anchor) = overflow_anchor {
        if window.has_overflow(preview.tabs().len()) {
            let entries: Vec<TabOverflowEntry<'_, Message>> = preview
                .tabs()
                .iter()
                .enumerate()
                .filter(|(idx, _)| !(window.first..window.visible_end).contains(idx))
                .map(|(idx, tab)| TabOverflowEntry {
                    index: idx,
                    prefix: None,
                    title: tab.title.clone(),
                    active: idx == preview.active_idx(),
                    closable: true,
                })
                .collect();
            let menu = tab_overflow_menu(TabOverflowMenuArgs {
                entries,
                anchor,
                window_size: app.window_size,
                on_select: select_msg,
                on_close: close_msg,
                on_dismiss: overflow_dismiss_msg(),
            });
            return iced_widget::stack![base, menu].into();
        }
    }
    base
}
```

(`select_msg`/`close_msg` 是函数前面(`workspace.rs:3594-3601`)已经定义好的闭包,类型是 `impl Fn(usize) -> Message`,直接传给 `on_select`/`on_close`,不用再包一层。)

Run: `cargo build -p dozer-app 2>&1 | tail -80`
Expected: PASS(过程中会报多处未用到的旧变量/import,按报错清理,如 `left_arrow`/`right_arrow`/`tab_arrow_button` import 若不再被这个文件其它地方用到就删掉 import)

- [ ] **Step 6: 手动验证**

Run: `cargo run -p dozer-app`

1. 项目树连续点开多个文件直到文件预览 tab 溢出,重复 Task 2 Step 5 的 8 项检查(V 出现/下拉内容/自动滚动/已可见不抖动/关闭/点外部关闭/V 消失)。
2. Project 面板右配对预览同样重复一遍。
3. 额外确认:下拉悬浮层没有被下方的原生预览 webview 盖住(向上弹是否生效)。

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): 文件预览/项目预览 tab 溢出改用 V 下拉替换左右箭头

PreviewTabScroll/ProjectPreviewTabScroll 换成对应的
TabOverflowToggle/Dismiss;PreviewSelectTab/ProjectPreviewSelectTab
追加 tab_window_reveal 自动滚动。悬浮定位向上弹,避开下方 wry webview。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01GNaJ3sH2f2tP2cPRBQ4qri
EOF
)"
```

---

### Task 4: SSH tab

**Files:**
- Modify: `crates/dozer-app/src/app.rs:9395-9570`(`ssh_tab_bar`);Message 枚举与 `App::update` 分支(`ssh::Message::TabScroll`/`SelectSshTab` 相关,~5815-5834 及 `ssh.rs` 里 `Message` 枚举定义处)
- Modify: `crates/dozer-app/src/workspace.rs`(`Workspace.ssh_tab_first` 附近 ~359-361,新增锚点字段)

**Interfaces:**
- Consumes: Task 1 的共享基础设施
- Produces: 无

- [ ] **Step 1: `Workspace` 新增锚点字段**

`workspace.rs:359-361` 附近追加:

```rust
    /// SSH tab 栏溢出下拉锚点,语义同 `term_tab_overflow_anchor`。
    pub(crate) ssh_tab_overflow_anchor: Option<(f32, f32)>,
```

构造处补 `ssh_tab_overflow_anchor: None,`。

- [ ] **Step 2: `ssh::Message` 枚举替换**

在 `crates/dozer-app/src/extensions/ssh.rs` 里找到 `Message::TabScroll(bool)` 的定义,换成:

```rust
    /// SSH tab 栏溢出下拉开关,语义同顶层 `Message::TermTabOverflowToggle`。
    TabOverflowToggle,
    /// SSH tab 栏溢出下拉:点击外部关闭。
    TabOverflowDismiss,
```

- [ ] **Step 3: `App::update` 分支替换 + `SelectSshTab`/`SelectBlankTab` 追加自动滚动**

把 `app.rs:5826-5834` 的 `Message::Ssh(ssh::Message::TabScroll(right)) => {...}` 换成:

```rust
            Message::Ssh(ssh::Message::TabOverflowToggle) => {
                let last_cursor = self.last_cursor;
                self.with_focused_project(|ws, _io| {
                    ws.ssh_tab_overflow_anchor = if ws.ssh_tab_overflow_anchor.is_some() {
                        None
                    } else {
                        Some(last_cursor)
                    };
                });
            }
            Message::Ssh(ssh::Message::TabOverflowDismiss) => {
                self.with_focused_project(|ws, _io| {
                    ws.ssh_tab_overflow_anchor = None;
                });
            }
```

SSH 的 tab 是按 `(host_id, SshTabKind)` 寻址,不是像终端/预览那样的纯 vec 下标,渲染时的"扁平顺序"是:下标 0 固定给空白占位 tab,之后是 `ws.ssh_tabs`(按 vec 顺序)算终端 tab,再之后是 `ws.sftp_tabs`(`HashMap`,按当前迭代顺序——同一个未被增删的 `HashMap` 两次迭代顺序一致,渲染侧和这里用的是同一份数据、中间没有插入/删除,顺序可靠)。在 `Message::Ssh(ssh::Message::SelectSshTab(host_id, kind)) => {...}` 分支(约 `app.rs:5815` 之前几行,调用 `ws.select_ssh_tab(host_id, kind)` 那一段)和 `Message::Ssh(ssh::Message::SelectBlankTab) => {...}` 分支里,各自在设置完选中态之后追加:

```rust
                    let mut widths: Vec<f32> = vec![tab_display_width("空白")];
                    widths.extend(
                        ws.ssh_tabs
                            .iter()
                            .map(|t| tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name))),
                    );
                    widths.extend(ws.sftp_tabs.keys().map(|host_id| {
                        tab_display_width(
                            &ws.ssh
                                .hosts()
                                .iter()
                                .find(|h| &h.id == host_id)
                                .map(|h| h.name.clone())
                                .unwrap_or_else(|| host_id.clone()),
                        )
                    }));
                    let target = match &ws.ssh_active {
                        None => 0,
                        Some((active_host, ssh::SshTabKind::Terminal)) => ws
                            .ssh_tabs
                            .iter()
                            .position(|t| t.info.id.strip_prefix("ssh:") == Some(active_host.as_str()))
                            .map(|i| i + 1)
                            .unwrap_or(0),
                        Some((active_host, ssh::SshTabKind::Sftp)) => ws
                            .sftp_tabs
                            .keys()
                            .position(|h| h == active_host)
                            .map(|i| i + 1 + ws.ssh_tabs.len())
                            .unwrap_or(0),
                    };
                    ws.ssh_tab_first = tab_widget::tab_window_reveal(
                        &widths,
                        4.0,
                        byteui::theme::geometry::tab_bar_avail_px(),
                        ws.ssh_tab_first,
                        target,
                    );
                    ws.ssh_tab_overflow_anchor = None;
```

(这段"算扁平 target 下标"的逻辑在两个分支里重复,是可以接受的小重复——两个分支各自的前置条件不同(一个已知 `host_id`+`kind`,一个是空白),硬拆成共享函数收益不大,不必强行 DRY。)

- [ ] **Step 4: 改造 `ssh_tab_bar` 渲染**

把 `app.rs:9528-9533` 的 `let (first, can_left, can_right) = tab_widget::tab_window(...)` 换成:

```rust
    let window = tab_widget::tab_window(
        &widths,
        4.0,
        byteui::theme::geometry::tab_bar_avail_px(),
        ws.ssh_tab_first,
    );
```

把 `app.rs:9534-9539` 的 `.filter(|(idx, _)| *idx >= first)` 换成 `.filter(|(idx, _)| (window.first..window.visible_end).contains(idx))`。

把 `app.rs:9544-9556`(`clipped`/`left_arrow`/`right_arrow`)换成:

```rust
    let clipped = container(row(items).spacing(4))
        .width(Length::Fill)
        .clip(true);
    let hidden_count = window.hidden_before().len() + window.hidden_after(widths.len()).len();
    let overflow_button =
        tab_widget::tab_overflow_button(hidden_count, Message::Ssh(ssh::Message::TabOverflowToggle));
```

紧接着的 `collapse` 按钮定义不变,把原本组装 `tab_bar`/最终返回的那部分(9557 行往后,读一下确认最终 `Element` 组装方式)改成:先 `row![clipped]` 起步,`if let Some(btn) = overflow_button { push }`,再 push `collapse`;函数末尾追加与 Task 2/3 相同结构的条件性 `stack![base, menu]`,`entries` 构建为:

```rust
    if let Some(anchor) = ws.ssh_tab_overflow_anchor {
        if window.has_overflow(widths.len()) {
            let mut entries: Vec<tab_widget::TabOverflowEntry<'_, Message>> = Vec::new();
            let hidden: std::collections::HashSet<usize> = window
                .hidden_before()
                .chain(window.hidden_after(widths.len()))
                .collect();
            if hidden.contains(&0) {
                entries.push(tab_widget::TabOverflowEntry {
                    index: 0,
                    prefix: None,
                    title: "空白".to_string(),
                    active: ws.ssh_active.is_none(),
                    closable: false,
                });
            }
            for (i, tab) in ws.ssh_tabs.iter().enumerate() {
                let idx = i + 1;
                if !hidden.contains(&idx) {
                    continue;
                }
                let host_id = tab.info.id.strip_prefix("ssh:").unwrap_or(&tab.info.id).to_string();
                entries.push(tab_widget::TabOverflowEntry {
                    index: idx,
                    prefix: Some(icons::view(
                        icons::IconKind::Terminal,
                        byteui::theme::icon_size::row(),
                        byteui::theme::color::current().dim,
                    )),
                    title: tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name),
                    active: ws
                        .ssh_active
                        .as_ref()
                        .is_some_and(|(h, k)| h == &host_id && *k == ssh::SshTabKind::Terminal),
                    closable: true,
                });
            }
            for (i, (host_id, _)) in ws.sftp_tabs.iter().enumerate() {
                let idx = i + 1 + ws.ssh_tabs.len();
                if !hidden.contains(&idx) {
                    continue;
                }
                let label = ws
                    .ssh
                    .hosts()
                    .iter()
                    .find(|h| &h.id == host_id)
                    .map(|h| h.name.clone())
                    .unwrap_or_else(|| host_id.clone());
                entries.push(tab_widget::TabOverflowEntry {
                    index: idx,
                    prefix: Some(icons::view(
                        icons::IconKind::FolderSync,
                        byteui::theme::icon_size::row(),
                        byteui::theme::color::current().dim,
                    )),
                    title: label,
                    active: ws.ssh_active.as_ref() == Some(&(host_id.clone(), ssh::SshTabKind::Sftp)),
                    closable: true,
                });
            }
            let menu = tab_widget::tab_overflow_menu(tab_widget::TabOverflowMenuArgs {
                entries,
                anchor,
                window_size: app.window_size,
                on_select: |idx| ssh_tab_overflow_select_message(ws, idx),
                on_close: |idx| ssh_tab_overflow_close_message(ws, idx),
                on_dismiss: Message::Ssh(ssh::Message::TabOverflowDismiss),
            });
            return iced_widget::stack![base, menu].into();
        }
    }
```

(`on_select`/`on_close` 需要把"扁平下标"翻回 `(host_id, kind)` 或空白态——写两个小 helper 函数,放在 `ssh_tab_bar` 附近。下标 0 是空白占位 tab;`1..=ws.ssh_tabs.len()` 是终端段;再往后是 SFTP 段:)

```rust
fn ssh_tab_overflow_select_message(ws: &Workspace, idx: usize) -> Message {
    if idx == 0 {
        return Message::Ssh(ssh::Message::SelectBlankTab);
    }
    let terminal_count = ws.ssh_tabs.len();
    if idx <= terminal_count {
        let tab = &ws.ssh_tabs[idx - 1];
        let host_id = tab.info.id.strip_prefix("ssh:").unwrap_or(&tab.info.id).to_string();
        return Message::Ssh(ssh::Message::SelectSshTab(host_id, ssh::SshTabKind::Terminal));
    }
    match ws.sftp_tabs.keys().nth(idx - 1 - terminal_count) {
        Some(host_id) => Message::Ssh(ssh::Message::SelectSshTab(host_id.clone(), ssh::SshTabKind::Sftp)),
        // 越界防御:理论上不会走到这里(entries 的 index 都来自实际渲染过
        // 的 tab),兜底回空白态。
        None => Message::Ssh(ssh::Message::SelectBlankTab),
    }
}

fn ssh_tab_overflow_close_message(ws: &Workspace, idx: usize) -> Message {
    // 空白占位 tab 的 `closable: false` 已经保证下拉里它没有 x,这里理论上
    // 不会被调用,兜底发一个明确的 no-op(顶层 `Message::Noop` 已存在)。
    if idx == 0 {
        return Message::Noop;
    }
    let terminal_count = ws.ssh_tabs.len();
    if idx <= terminal_count {
        let tab = &ws.ssh_tabs[idx - 1];
        let host_id = tab.info.id.strip_prefix("ssh:").unwrap_or(&tab.info.id).to_string();
        return Message::Ssh(ssh::Message::CloseSshTab(host_id, ssh::SshTabKind::Terminal));
    }
    match ws.sftp_tabs.keys().nth(idx - 1 - terminal_count) {
        Some(host_id) => Message::Ssh(ssh::Message::CloseSshTab(host_id.clone(), ssh::SshTabKind::Sftp)),
        None => Message::Noop,
    }
}
```

Run: `cargo build -p dozer-app 2>&1 | tail -100`
Expected: PASS

- [ ] **Step 5: 手动验证**

Run: `cargo run -p dozer-app`

连到多台主机 + 开若干 SFTP tab 直到 SSH tab 栏溢出,重复 Task 2 Step 5 的 8 项检查,额外确认:

- 空白占位 tab 若被挤入隐藏列表,下拉里对应行没有 x。
- 终端 tab 与 SFTP tab 混合溢出时,点选下拉里的 SFTP 项能正确跳转(不会跳错成终端 tab)。

- [ ] **Step 6: 提交**

```bash
git add crates/dozer-app/src/app.rs crates/dozer-app/src/workspace.rs crates/dozer-app/src/extensions/ssh.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): SSH tab 溢出改用 V 下拉替换左右箭头

ssh::Message::TabScroll 换成 TabOverflowToggle/Dismiss;
SelectSshTab/SelectBlankTab 追加按扁平下标计算的 tab_window_reveal
自动滚动,兼容终端/SFTP 混合 tab 顺序。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01GNaJ3sH2f2tP2cPRBQ4qri
EOF
)"
```

---

### Task 5: Database tab

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`(`Message` 枚举 ~789-797;`update` 分支 ~1204-1207;`DatabaseContentState` 字段与方法 ~2888-2924;tab 栏渲染 ~2100-2200)
- Modify: `crates/dozer-app/src/app.rs`(新增 `Message::Database(database::Message::TabOverflowToggle)` 拦截分支,参照 `ToggleListCollapse` 的拦截手法 ~4816-4818)

**Interfaces:**
- Consumes: Task 1 的共享基础设施;`App::last_cursor`/`App::window_size`(渲染函数已持有 `app: &App`,可直接用)
- Produces: 无

- [ ] **Step 1: `database::Message` 枚举新增变体**

把 `database.rs:797` 的 `TabScroll(bool)` 和它的文档注释换成:

```rust
    /// tab 栏溢出下拉开关,语义同顶层 `Message::TermTabOverflowToggle`。由
    /// `App::update` 拦截处理(需要 `App::last_cursor`),不进 `database::update`。
    TabOverflowToggle,
    /// tab 栏溢出下拉:点击外部关闭。同样由 `App::update` 拦截。
    TabOverflowDismiss,
    /// 下拉行的 hover 占位消息(不产生任何副作用),`tab_overflow_menu` 的
    /// `on_select_hover`/`on_close_hover` 要求返回一个消息,这里补一个纯
    /// no-op,避免为一个短生命周期浮层铺一整套 hover 动画状态。
    Noop,
```

在 `database.rs:825` 起的 `pub fn update(...)` 里,`match msg` 最前面追加:

```rust
        Message::Noop => {}
```

- [ ] **Step 2: `DatabaseContentState` 新增锚点字段与方法**

`database.rs:2888-2899` 的 `DatabaseContentState` struct 追加字段:

```rust
    /// tab 栏溢出下拉的悬浮锚点,语义同 `Workspace::term_tab_overflow_anchor`。
    tab_overflow_anchor: Option<(f32, f32)>,
```

`impl DatabaseContentState` 里,把 `scroll_tabs` 方法(2914-2923 行)删掉,换成:

```rust
    pub fn tab_overflow_anchor(&self) -> Option<(f32, f32)> {
        self.tab_overflow_anchor
    }

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
```

在既有的 `select`/`select_blank` 方法体末尾(`database.rs:2998`/`3007` 起,读一下方法体确认 `self.active = ...` 这行的位置)各自追加:

```rust
        self.tab_overflow_anchor = None;
```

(`select`/`select_blank` 目前不掌握 `widths`/`avail`,"自动滚动带入可见区"的 `tab_window_reveal` 调用挪到渲染函数里做——见 Step 4,这里只需要保证选中后下拉自动收起。)

- [ ] **Step 3: `database.rs::update` 删除 `TabScroll` 分支**

把 `database.rs:1207` 的 `Message::TabScroll(right) => ws_state.content.scroll_tabs(right),` 整行删除(`TabOverflowToggle`/`TabOverflowDismiss` 由 `App::update` 拦截,不会走到这个 `update` 函数里——若编译器报 `match` 缺分支,说明拦截没生效,回去检查 Step 5)。

- [ ] **Step 4: `app.rs` 新增拦截分支 + "自动滚动"逻辑挪到渲染侧**

参照 `app.rs:4816-4818` 的 `Message::Database(database::Message::ToggleListCollapse) => {...}` 拦截手法,在它附近追加:

```rust
            Message::Database(database::Message::TabOverflowToggle) => {
                let last_cursor = self.last_cursor;
                self.with_focused_project(|ws, _io| {
                    ws.database.content.toggle_tab_overflow(last_cursor);
                });
            }
            Message::Database(database::Message::TabOverflowDismiss) => {
                self.with_focused_project(|ws, _io| {
                    ws.database.content.dismiss_tab_overflow();
                });
            }
```

(具体访问路径 `ws.database.content` 需按 `database::update(&mut ws.database, ...)` 调用处(`app.rs:6407-6410`)核对——`ws.database` 是传给 `database::update` 的 `ws_state`,其内部 `content` 字段类型是 `DatabaseContentState`,与 `database.rs:1204` 里 `ws_state.content.select(idx)` 的访问路径一致。)

由于 `select`/`select_blank` 不掌握渲染用的 `widths`,"选中隐藏 tab 后自动滚动"这次改成在渲染函数里做:每次渲染都检查当前 `active_idx` 是否落在窗口外,若是则用 `tab_window_reveal` 重新钳一次并把结果**写回** `tab_scroll_first`——但 `view` 函数不能直接改状态。改法:在 `Message::SelectTab(idx)`/`Message::SelectBlankTab` 对应的 `database::update` 分支追加(`database.rs:1204-1205`):

```rust
        Message::SelectTab(idx) => {
            ws_state.content.select(idx);
            ws_state.content.reveal_tab(idx + 1); // +1:下拉/渲染侧扁平下标里 0 留给空白占位 tab
        }
        Message::SelectBlankTab => {
            ws_state.content.select_blank();
            ws_state.content.reveal_tab(0);
        }
```

在 `DatabaseContentState` 上新增 `reveal_tab`,但它同样需要 `widths`——数据库 tab 的宽度依赖 `tab_title(tab, ws_state)` 这个渲染期才方便算的函数,直接搬进 `DatabaseContentState` 方法里会造成 `ws_state`/`self` 借用冲突。改成传入已经算好的宽度数组:

```rust
    /// 选中 `target`(扁平下标,0 留给空白占位 tab)后,若它当前隐藏,重新
    /// 钳出包含它的窗口;已可见则不动。`widths` 由渲染侧按 `tab_bar_avail_px`
    /// 同一套口径传入(含开头的空白占位 tab 宽度)。
    pub fn reveal_tab(&mut self, widths: &[f32], target: usize) {
        self.tab_scroll_first = crate::tab_widget::tab_window_reveal(
            widths,
            4.0,
            byteui::theme::geometry::tab_bar_avail_px(),
            self.tab_scroll_first,
            target,
        );
    }
```

(相应地,`database.rs:1204-1205` 那两行改成不带 `widths` 是不行的——上面 `reveal_tab` 签名改成带 `widths: &[f32]` 参数,但 `database::update` 拿不到渲染用的宽度数组;这里退一步用**近似宽度**(每个 tab 固定估一个 `crate::tab_widget::PANEL_TAB_MAX_W`,不追求精确到像素,只是为了让"该不该滚"这个判断方向正确——精确宽度只影响滚动量,不影响正确性上限)：)

```rust
        Message::SelectTab(idx) => {
            ws_state.content.select(idx);
            let approx_widths = vec![crate::tab_widget::PANEL_TAB_MAX_W; ws_state.content.tabs().len() + 1];
            ws_state.content.reveal_tab(&approx_widths, idx + 1);
        }
        Message::SelectBlankTab => {
            ws_state.content.select_blank();
            let approx_widths = vec![crate::tab_widget::PANEL_TAB_MAX_W; ws_state.content.tabs().len() + 1];
            ws_state.content.reveal_tab(&approx_widths, 0);
        }
```

- [ ] **Step 5: 改造 tab 栏渲染**

把 `database.rs:2157-2162` 的 `let (first, can_left, can_right) = crate::tab_widget::tab_window(...)` 换成:

```rust
    let window = crate::tab_widget::tab_window(
        &widths,
        4.0,
        byteui::theme::geometry::tab_bar_avail_px(),
        content.tab_scroll_first(),
    );
```

把 `database.rs:2163-2168` 的 `.filter(|(idx, _)| *idx >= first)` 换成 `.filter(|(idx, _)| (window.first..window.visible_end).contains(idx))`。

把 `database.rs:2169-2180`(`clipped`/`left_arrow`/`right_arrow`)换成:

```rust
    let tabs_row = row(items).spacing(4);
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    let hidden_count = window.hidden_before().len() + window.hidden_after(widths.len()).len();
    let overflow_button =
        crate::tab_widget::tab_overflow_button(hidden_count, Message::TabOverflowToggle);
```

`database.rs:2192` 的 `tab_bar` 组装换成 `row![clipped]` 起步、`if let Some(btn) = overflow_button { push }`、再 push `collapse`(同 Task 2/3/4 模式)。

在函数末尾原本返回 `col.into()`(或类似)的地方之前,插入(注意这里 `entries` 里第 0 条固定是空白占位 tab,`entries.extend(content.tabs()...)` 那段已有的构建逻辑在 2117-2155 行,直接复用同一份 `entries` 数据源,只是这次要额外收集"标题+图标+active+closable"而不是已经建好的 `Element`——写一份独立的、只在有溢出时才跑的小循环即可,不需要改动 2117-2155 行已有的 `entries` 构建):

```rust
    if let Some(anchor) = content.tab_overflow_anchor() {
        if window.has_overflow(widths.len()) {
            let hidden: std::collections::HashSet<usize> = window
                .hidden_before()
                .chain(window.hidden_after(widths.len()))
                .collect();
            let mut overflow_entries: Vec<crate::tab_widget::TabOverflowEntry<'_, Message>> = Vec::new();
            if hidden.contains(&0) {
                overflow_entries.push(crate::tab_widget::TabOverflowEntry {
                    index: 0,
                    prefix: None,
                    title: "空白".to_string(),
                    active: content.active_idx().is_none(),
                    closable: false,
                });
            }
            for (i, tab) in content.tabs().iter().enumerate() {
                let idx = i + 1;
                if !hidden.contains(&idx) {
                    continue;
                }
                overflow_entries.push(crate::tab_widget::TabOverflowEntry {
                    index: idx,
                    prefix: None,
                    title: tab_title(tab, ws_state),
                    active: Some(i) == content.active_idx(),
                    closable: true,
                });
            }
            let menu = crate::tab_widget::tab_overflow_menu(crate::tab_widget::TabOverflowMenuArgs {
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
            });
            return iced_widget::stack![col, menu].into();
        }
    }
    col.into()
```

(`hidden` 集合里的 0 是空白占位 tab 的扁平下标,`content.tabs()` 里第 `i` 个真实 tab 对应扁平下标 `i+1`,`on_select`/`on_close` 里 `idx-1` 把扁平下标转回 `content.tabs()` 的真实 vec 下标,和 `entries.extend(content.tabs().iter().enumerate().map(...))` 里 2117 行往下已有的 `idx` 用法保持同一套换算口径。)

Run: `cargo build -p dozer-app 2>&1 | tail -100`
Expected: PASS

- [ ] **Step 6: `cargo clippy` 全量检查**

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | tail -100 && cargo fmt`
Expected: 无新增报错(Task 1 Step 4 里过渡用的 `can_left`/`can_right` 到这里应该已经在各任务里随箭头按钮一起清理干净,若还有残留按报错位置清掉)

- [ ] **Step 7: 手动验证**

Run: `cargo run -p dozer-app`

开若干数据源的表/查询 tab 直到 Database 面板 tab 栏溢出,重复 Task 2 Step 5 的 8 项检查,额外确认:

- 空白占位 tab 若被挤入隐藏列表,下拉里对应行没有 x。
- 由于自动滚动用的是近似宽度(每个 tab 按 `PANEL_TAB_MAX_W` 估),验证"选中隐藏 tab 后确实滚动带入了可见区"这个方向性结果依然成立(不要求像终端/预览那样滚动量精确到刚好贴边)。

- [ ] **Step 8: 提交**

```bash
git add crates/dozer-app/src/extensions/database.rs crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): Database tab 溢出改用 V 下拉替换左右箭头

database::Message::TabScroll 换成 TabOverflowToggle/Dismiss(App::update
拦截,同 ToggleListCollapse 手法,因为需要 App::last_cursor);SelectTab/
SelectBlankTab 追加按近似宽度估算的 reveal_tab 自动滚动。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01GNaJ3sH2f2tP2cPRBQ4qri
EOF
)"
```

---

### Task 6: 人工 GUI 验收清单

**Files:** 无代码改动,纯验证。

**Interfaces:** 无

- [ ] **Step 1: 全量回归验证**

Run: `cargo build --workspace && cargo clippy --workspace --all-targets && cargo fmt --check`
Expected: 全绿,`fmt --check` 无差异(若有差异先 `cargo fmt` 再重新提交一次格式化 commit)

Run: `cargo test --workspace`
Expected: 全绿,包含 Task 1 新增/改造的 `tab_window`/`tab_window_reveal` 单测

- [ ] **Step 2: 逐面板过一遍验收清单**

`cargo run -p dozer-app`,对以下 5 个面板逐一执行 Task 2 Step 5 的 8 项检查(V 出现时机/下拉内容只含隐藏 tab/选中自动滚动/已可见不抖动/关闭 tab/点外部关闭/tab 变少后 V 消失/视觉与其它面板一致):

- [ ] 终端会话 tab
- [ ] 文件预览 tab
- [ ] 项目预览 tab
- [ ] SSH tab(含终端+SFTP 混合、空白占位 tab 场景)
- [ ] Database tab(含空白占位 tab 场景)

- [ ] **Step 3: 交叉场景检查**

- [ ] 同时打开两个面板的下拉(比如文件预览下拉开着时切到终端面板再开终端的下拉),确认互不干扰、只有当前操作的那个下拉显示。
- [ ] 窗口 resize 到很窄再变宽,确认 V 按钮/下拉能正确响应可视宽度变化(变宽后不再溢出,V 自动消失)。
- [ ] 深色主题下(ByteBoy2077 配色)下拉的选中高亮、hover、关闭按钮颜色符合金/奶油/DIM 配色规范,不出现对比度过低看不清的情况。

- [ ] **Step 4: 更新 CLAUDE.md(若相关裁决段落需要修订)**

检查 `CLAUDE.md` 关键裁决里是否有提到旧的"翻页箭头"设计的措辞(全文搜索 `tab_arrow_button`/`翻页箭头`),若有明确提到当前实现细节且已被这次改造废弃的表述,补一条简短说明(参照现有"⌘K 命令面板未实现...早期设计陈述作废"的写法),避免后续读者被过时描述误导。

- [ ] **Step 5: 通知用户走 `superpowers:finishing-a-development-branch`**

以上全部勾完后,按 `superpowers:finishing-a-development-branch` 流程决定如何把这个 worktree 分支合并回 main(该 skill 会引导做最终的 diff 审阅/合并方式选择,这里不重复其步骤)。
