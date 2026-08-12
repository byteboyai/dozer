# icon 按钮 / tab 共享组件全量推广设计

**状态:已批准(brainstorming 会话,2026-08-12)**

## 背景

2026-08-12 稍早的试点(`docs/superpowers/specs/2026-08-12-tab-icon-button-shared-components-design.md`)
落地了 `icons::icon_button_entry`(迁移 rail 11 个按钮)与
`tabs::tab_core`(迁移 `project_tab_item`),两个组件已经过审阅+人工验证,
证明可行。这次把它们推广到仓库里其余符合条件的调用点。

开工前重新核实一遍现状,发现两个此前没预料到的问题:

1. **`icon_button_entry` 目前硬编码了 rail 专属的视觉**(`icons.rs:287`
   起):`.width/.height` 固定用 `theme::geometry::rail_button_size()`,
   `background` 固定 `Some(theme::color::CARD.into())`(常驻卡片底)。而
   底层它包的 `icon_button()`(`icons.rs:214`)本来就有 `card: bool` 参数,
   支持"卡片底"/"透明底"两种样式——`icon_button_entry` 把这条灵活性
   丢了。真正的迁移目标(见下方第 3 点核实结果)里,`files.rs` 两处是
   卡片底、其余是透明底,尺寸也从 `rail_button_size()` 到调用方局部变量
   `box_len` 不等,直接套用现在的 `icon_button_entry` 要么画出不该有的
   卡片背景,要么尺寸不对。**这次要先把 `card`/`button_size` 补成参数**,
   rail 那 11 处调用改成显式传 `card: true, button_size:
   rail_button_size()`(视觉不变)。
2. **多数裸 `icons::view(...)` 调用根本不是按钮**,而是纯装饰图标(文件树
   图标、footbar 状态图标、git 分支标签图标、面板标题图标)或图标+文字
   组合按钮(Todo 的"派发"按钮、右键菜单项、Dozer 品牌页签)——`icon_
   button_entry` 是纯图标方按钮的形状,套不上这些。
3. **顶栏 `Settings`/`Add project`/`AgentPickerToggle` 三个一开始以为符合
   条件,逐个核对源码后发现全部不行**,原因各不相同:
   - **`Settings`(`app.rs:4484` 起)根本不是按钮**——只有
     `container(icons::view(...))` 包一层 `MouseArea` 取悬停变色,**没有
     `on_press`**(源码注释:"目前尚未接入设置面板,先只还原视觉,不加
     on_press——没有对应 Message 变体可派发")。`icon_button_entry` 的
     签名要求 `on_select: M`,这里没有消息可传,没有东西可迁移。
   - **`Add project`(`app.rs:4674` 起)与 `AgentPickerToggle`
     (`workspace.rs:1950` 起)都是 `.padding([6, 8])` 撑出尺寸**,没有
     `icon_button_entry` 要求的固定 `.width/.height(Fixed(button_size))`
     ——是内容+留白撑开的可变尺寸,不是固定方形命中区。真要套
     `icon_button_entry` 得先把这两个"按 padding 撑开"的尺寸换算成一个
     等效的固定 `button_size`,这是**尺寸口径改动**,有引入像素级视觉
     差异的风险,不是纯重构。
   
   已核实的 `icons::icon_button()` 7 处面板操作按钮(`browser`/
   `database`/`files`/`usage`)则全部使用显式 `.width(Length::Fixed(..))
   .height(Length::Fixed(..))`——与 rail 11 个同一个"固定方形"形状族,
   是真正安全的迁移目标。**这次 icon 按钮迁移范围收窄成这 7 个,顶栏三个
   按钮不动**(brainstorming 后追加核实的结论,原提案的"10 个"里有 3 个
   经核实并不成立)。
4. 另有面板 tab 滚动箭头(`app.rs:5781` 的 `tab_scroll_arrow` 之类)走的是
   iced 原生 `button::Status::Hovered` 瞬时切换,不是 `hover_t` 动画——
   brainstorming 已确认这次**不**把它们也改成动画 hover(那是一次真实的
   交互行为改动,不是纯重构),留在范围外。

`panel_tab`(`app.rs:5870`)复查后发现是个理想的迁移目标:它当前的
`select`/`close_btn`/`close` 构造与 `tab_core` 的参数**逐项对应**(见
"架构"一节),搬过去只是替换掉现有构造语句,不需要新增任何参数管线。

## 目标 / 非目标

**目标**:

1. `icons::icon_button_entry` 加 `card: bool`、`button_size: f32` 两个
   参数,补齐 `icon_button()` 已有的灵活性。已迁移的 rail 11 处调用点
   同步补上显式实参(`card: true, button_size: rail_button_size()`),
   视觉不变。
2. 迁移 7 个符合条件的 icon 按钮到 `icon_button_entry`:`browser.rs` 2 处
   (收藏星标/书签开关)、`database.rs` 1 处(schema 返回箭头)、
   `files.rs` 3 处(目录搜索/隐藏文件切换/分支选择器箭头)、`usage.rs`
   1 处(刷新)——均已使用 `icons::icon_button()` + 固定
   `.width/.height(Fixed(..))`,是与 rail 同形状的安全迁移目标。
3. `panel_tab` 迁移到 `tabs::tab_core`——现有的 `title_hover`/
   `close_hover`/`on_select`/`on_close` 四个参数直接对应 `tab_core` 的
   同名/同形参数,不需要新增任何参数。

**非目标**:

- **不迁移顶栏 `Settings`/`Add project`/`AgentPickerToggle`**——核实后
  发现它们不符合 `icon_button_entry` 的形状(`Settings` 没有 `on_press`
  可迁;后两个是 padding 撑开的可变尺寸,不是固定方形),硬套需要额外的
  尺寸口径改动,不是纯重构,这次不做(见"背景"第 3 点)。
- **不迁移面板 tab 滚动箭头**(`app.rs` 里几处 `button::Status::Hovered`
  的箭头按钮)——原生瞬时 hover 换成动画 hover_t 是行为改动,不是本次
  "消除重复"范围;brainstorming 已确认。
- **不迁移任何纯装饰图标**(文件树图标、footbar 状态图标、git 分支标签、
  面板标题图标)——它们不是按钮,没有可消除的重复交互逻辑。
- **不迁移图标+文字组合按钮**(Todo"派发"按钮、右键菜单项、Dozer 品牌
  页签)——`icon_button_entry`/`tab_core` 都是特定形状的组件,硬套会
  产生跟现在不一致的布局,不在这次范围。
- **不改变任何可观察行为**——像素级尺寸、颜色、动画曲线,迁移前后必须
  完全一致。
- **不引入新的 `HoverId` 变体或新消息类型**——7 个 icon 按钮和
  `panel_tab` 都已经有现成的 `hover_t`/`close_hover_t` 来源(`app.
  hover_progress(...)` 或调用方自己的等价机制),迁移只是换个函数调用,
  不新增状态。

## 架构

### 1. `icon_button_entry` 加 `card`/`button_size`/`interactive` 参数(`icons.rs:287`)

写这份设计后、进一步核对 7 个迁移目标的完整代码时又发现一处:
`browser.rs:1108`(收藏星标按钮)的 `on_press` 是**条件挂载**的——只有
当前 tab 有 URL 时才挂 `Message::StarClick`,没有 URL 时完全不挂(源码
注释:未收藏静止 DIM,已收藏恒金;没有 URL 时点星标没有意义,不该弹出
收藏菜单)。`icon_button_entry` 现在对 `on_press` 是无条件挂载,直接套用
会让没有 URL 时也能点开收藏菜单,是行为改动。这与 `tab_core` 处理"关闭
按钮仅悬停可点"的 `close_interactive: bool` 是同一类需求,补一个同款
`interactive: bool` 参数(为真才挂 `on_press`,其余 6 个迁移目标恒传
`true`,行为不变):

```rust
pub fn icon_button_entry<'a, M: Clone + 'a>(
    kind: IconKind,
    size: f32,
    active: bool,
    hover_t: f32,
    card: bool,
    button_size: f32,
    interactive: bool,
    on_select: M,
    on_hover: impl Fn(bool) -> M + 'a,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let color = if active {
        crate::theme::color::GOLD
    } else {
        crate::theme::color::mix(crate::theme::color::DIM, crate::theme::color::GOLD, hover_t)
    };
    let inner = container(view(kind, size, color))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center);

    let radius = 8.0;
    let base_border = Border {
        color: Color::TRANSPARENT,
        width: 1.0,
        radius: radius.into(),
    };
    let mut btn = button(inner)
        .width(Length::Fixed(button_size))
        .height(Length::Fixed(button_size))
        .padding(0)
        .style(move |_t: &iced_widget::Theme, _status: button::Status| {
            button::Style {
                background: if card {
                    Some(crate::theme::color::CARD.into())
                } else {
                    None
                },
                border: Border {
                    color: if active {
                        crate::theme::color::GOLD
                    } else {
                        Color::TRANSPARENT
                    },
                    ..base_border
                },
                ..button::Style::default()
            }
        });
    if interactive {
        btn = btn.on_press(on_select);
    }

    MouseArea::new(btn)
        .interaction(mouse::Interaction::Pointer)
        .on_enter(on_hover(true))
        .on_exit(on_hover(false))
        .into()
}
```

`card: false` 时背景恒 `None`(原先的 `Some(CARD)` 变成条件分支),其余
逻辑不变。11 个 rail 调用点(`left_icon_rail`/`right_icon_rail`)加三个
实参:`card: true, button_size: crate::theme::geometry::rail_button_size(),
interactive: true`——与当前硬编码值/行为完全相同,纯粹把隐式变显式。

### 2. 7 个 icon 按钮迁移清单

| 调用点 | `active` | `card` | `button_size` | `interactive` |
|---|---|---|---|---|
| `browser.rs:1108`(收藏星标) | `starred`(动态) | `false` | `theme::geometry::tab_button_size()` | `url.is_some()` |
| `browser.rs:1427`(书签开关) | `false` | `false` | `theme::geometry::tab_button_size()` | `true` |
| `database.rs:1488`(schema 返回箭头) | `false` | `false` | `box_len`(调用方局部变量,原样传入) | `true` |
| `files.rs:767`(目录搜索) | `false` | `true` | `box_len` | `true` |
| `files.rs:798`(隐藏文件切换) | `false` | `true` | `box_len` | `true` |
| `files.rs:1107`(分支选择器箭头) | `false` | `false` | `box_len` | `true` |
| `usage.rs:406`(刷新) | `false` | `false` | `crate::theme::geometry::rail_button_size()` | `true` |

调用方原有的 `MouseArea::new(...).on_enter(...).on_exit(...)` 包裹层删除,
换成 `icon_button_entry(...)` 一次调用返回已经接好线的 `Element`。

### 3. `panel_tab` 迁移到 `tabs::tab_core`

当前 `panel_tab`(`app.rs:5870`)的 `select`/`close_btn`/`close` 构造与
`tab_core` 的参数逐项对应:

| `panel_tab` 现有变量/参数 | `tab_core` 对应形参 |
|---|---|
| `title_row`(prefix+标题拼好的 row) | `content` |
| `close_sz` | `close_sz` |
| `close_color`(`Color{a:hover,..close_base}`) | `close_color` |
| `hovered`(`hover > 0.001`) | `close_interactive` |
| `on_select` | `on_select` |
| `on_close` | `on_close` |
| `title_hover`(`impl Fn(bool) -> M`) | `on_select_hover` |
| `close_hover`(`impl Fn(bool) -> M`) | `on_close_hover` |

四个闭包参数(`title_hover`/`close_hover`)与 `on_select`/`on_close` 都是
`panel_tab` 已有的函数参数,直接透传给 `tab_core`,调用方(`app.rs`/
`workspace.rs`/`extensions/browser.rs` 三处 `panel_tab(...)` 调用)不需要
任何改动。`panel_tab` 自己的 `select`/`close_btn`/`close` 三段构造代码
删除,换成一次 `tabs::tab_core(...)` 调用,取到的 `(select, close)` 接入
后续 `tab_row`/`container` 拼装,其余布局(标题最大宽/内边距/激活态实底
背景/悬停胶囊)不变。

## 错误处理

不适用——纯组件参数扩展与调用点替换,无新增可失败路径。

## 测试策略

- `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin
  dozer && cargo fmt -p dozer-app -- --check` 干净通过。`cargo test` 基线
  是当前 483 passed / 3 failed(两个 terminal grid + 一个 git_log 既有
  失败,均与本次改动无关)。
- 人工验证(独立命名临时二进制,不碰用户正式 app):
  - 11 个 rail 按钮(补参数后)外观/交互与迁移前逐一比对无差异。
  - `browser`/`database`/`files`/`usage` 7 个面板操作按钮:hover 过渡、
    点击行为、卡片背景(`files.rs` 两处应保留卡片底,其余不应有)与
    迁移前一致;收藏星标按钮额外验证"当前 tab 无 URL 时点击无反应"这条
    (`interactive: url.is_some()`)在迁移后依然成立。
  - `panel_tab`:终端/预览/浏览器三组面板 tab 的选中(mousedown 即选中)、
    关闭按钮(仅悬停可点)、hover 胶囊背景与迁移前完全一致——这组此前
    brainstorming 阶段用户已确认"panel 处 tab 已经完美",这次验证的
    是"迁移后依然完美",不是重新设计。

## 排期备注

三块改动(泛化 `icon_button_entry`、7 个按钮迁移、`panel_tab` 迁移)有
顺序依赖——`icon_button_entry` 泛化必须先做,后两块都依赖新参数;7 个
按钮迁移与 `panel_tab` 迁移彼此独立,泛化完成后可并行。与已合并的
rail/`project_tab_item` 试点、已合并的 `App::update` 抽方法、已合并的
唤醒定时器解耦均已落地,无冲突;工作目录仍是多会话共享环境,写计划时
按惯例用 `grep -n` 核实实际行号,不要假设与本文档一致。
