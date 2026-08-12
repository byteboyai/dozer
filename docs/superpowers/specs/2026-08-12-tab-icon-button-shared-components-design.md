# Tab / icon 按钮共享组件设计

**状态:已批准(brainstorming 会话,2026-08-12)**

## 背景

2026-08-12 一天内连续修复了三个 bug,根因都是"同一个 UI 概念(tab 的选中/hover/
拖拽逻辑)在 `panel_tab`(`app.rs:5767`,终端/预览/浏览器三组共用)和
`project_tab_item`(`app.rs:4567` 附近,顶栏项目页签独用)里各写一份,两处后来
悄悄长歪":

1. `MouseArea::on_press` 修复(顶栏页签长按拖动整个窗口):`project_tab_item`
   和 `panel_tab` 各自把 `button().on_press()` 换成 `MouseArea::on_press()`,
   同一条注释("`Button::on_press` 实际在松开时才发消息")复制了两份。
2. 悬停胶囊背景左缘贴状态点:`project_tab_item` 独有的 `capsule` 平级层
   结构(而非 `panel_tab` 的容器自绘背景)导致外层 padding 调整对它不生效,
   得在 `label` 自己的 padding 上单独补。
3. 拖拽换位后残留悬停高亮:`tab_drag_move`(`app.rs:1733` 起)是 App 级共享
   处理,这条本身没有重复,但暴露了"项目页签拖拽此前从未真正跑通"(被顶栏
   原生拖窗吞掉),意味着 `project_tab_item` 这条路径长期没被实际测试过。

同一天验证 icon 按钮时发现同样的模式更普遍:

- `icons::icon_button`(`icons.rs:214`)只在 7 处调用(`browser.rs` ×2、
  `database.rs`、`files.rs` ×3、`usage.rs`),而裸 `icons::view(...)` 直接调用
  有 28 处分散在 8 个文件——多数 icon 按钮各自手写 `button + MouseArea +
  on_enter/on_exit` 三件套,不经过任何共享封装。
- `left_icon_rail`(`app.rs:4931`)/`right_icon_rail`(`app.rs:5028`)是最典型
  的重复:11 个按钮(左 7 右 4),每个都是同一段 6 行样板
  `MouseArea::new(rail_icon_button(...)).on_enter(...).on_exit(...)`,且
  `hover_progress(HoverId::Rail(X))` 查询、`on_enter` 里的 `HoverId::Rail(X)`、
  `on_exit` 里的 `HoverId::Rail(X)` 三处必须手动保持同一个 `X`,复制粘贴时
  改错任一处不会编译报错,只会表现成"这个按钮 hover 动画不对"的运行时 bug。

`panel_tab` 已经证明"一个共享函数服务多个调用方"这条路径在本仓库可行(它
本身就是终端/预览/浏览器三组共用),这次设计把同一手法系统化地用到
项目页签和 icon 按钮上。

## 目标 / 非目标

**目标**:

1. 新增 `icons::icon_button_entry`:把"图标按钮 + hover 动画 + 点击"这套
   三件套封装成一个返回完全接好线的 `Element` 的函数,调用方只传一个
   `on_hover: impl Fn(bool) -> M` 闭包,不再需要在查询/`on_enter`/`on_exit`
   三处分别手写同一个 `HoverId`。
2. 新增 `tabs::tab_core`(新文件 `crates/dozer-app/src/tabs.rs`):把 tab 的
   "选中(mousedown 即选中+备拖)+ 关闭按钮(hover 才可点)"这套交互逻辑
   抽成共享函数,返回 `(select, close)` 两个 `Element`,调用方仍自行拼装
   外层布局(定宽 `stack!` vs 自适应 `row!`)、背景与尺寸。
3. 试点验证:
   - `icon_button_entry` → 迁移 `left_icon_rail` + `right_icon_rail` 全部
     11 个按钮。
   - `tab_core` → 迁移 `project_tab_item`。
4. 迁移后的调用点行为/像素级视觉与迁移前一致(不是"重新设计",是"消除
   重复"),用第三个已建立的验证手法(建独立命名的临时二进制、截图对比、
   不碰用户正在跑的正式 app)逐一确认。

**非目标**:

- **不合并 `project_tab_item` 与 `panel_tab` 成一个函数**。两者宽度模型
  (定宽均分 vs 随标题自适应)、关闭按钮布局(叠层 vs 同行)是真实存在
  的产品差异,`tab_core` 只共享交互逻辑,布局/尺寸/背景继续各自决定。
- **不迁移 `panel_tab` 本身**。它当前是正确的(用户已确认"panel 处 tab
  已经完美"),这轮不动没坏的代码;`tab_core` 落地并经审阅后,把
  `panel_tab` 切过去是自然的下一步,但不在这次范围内。
- **不迁移其余 icon 按钮调用点**(顶栏 `+`/齿轮、各面板内的刷新/搜索/
  显隐/返回等操作按钮)。这些留给后续增量迁移,`icon_button_entry` 落地
  即可用,不需要一次性改完才算完工。
- **不引入新状态、新消息类型、新持久化字段**。`HoverId`/`hover_anims`/
  `Message::Hover`/`Message::LeftIconSelect`/`Message::ProjectTabSwitch`/
  `Message::ProjectTabClose` 全部保持原样,新函数只是重新组织"谁来构造
  这些消息"。
- **不做任何视觉改版**。像素级尺寸、颜色、动画曲线与迁移前完全一致——
  这是一次纯粹的"消除重复"重构,不是"顺手改进"。

## 架构与数据流

两个新增都是无状态的纯 `fn(参数) -> Element<M>`,不持有 `App`/`Workspace`
引用,不产生副作用——与 `panel_tab` 现有的泛型闭包风格一致,`M: Clone` 是
唯一的类型约束。

### 1. `icons::icon_button_entry`(加在现有 `icons.rs`,紧邻 `icon_button`)

```rust
/// `icon_button` 的完整接线版:图标按钮 + hover 动画 + 点击,一次性把
/// `MouseArea`(`Pointer` 光标 + `on_press`/`on_enter`/`on_exit`)接好。
/// 调用方只需算好 `hover_t`(通常是 `app.hover_progress(some_id)`,多数
/// view 函数拿不到 `&App`,所以这一步仍留给调用方)和一个 `on_hover`
/// 闭包——闭包内部才知道具体 `HoverId`,`icon_button_entry` 本身不认识
/// `HoverId`(定义在 `app.rs`,`icons.rs` 不依赖 `app.rs`)。
pub fn icon_button_entry<'a, M: Clone + 'a>(
    kind: IconKind,
    size: f32,
    active: bool,
    hover_t: f32,
    on_select: M,
    on_hover: impl Fn(bool) -> M + 'a,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    MouseArea::new(icon_button(kind, size, active, hover_t, false).on_press(on_select))
        .interaction(iced_widget::core::mouse::Interaction::Pointer)
        .on_enter(on_hover(true))
        .on_exit(on_hover(false))
        .into()
}
```

**试点用例**(`left_icon_rail`,`app.rs:4940-4950` 这一段):

```rust
// 迁移前——HoverId::Rail(RailButton::LeftProject) 出现 3 次:
MouseArea::new(rail_icon_button(
    icons::IconKind::Briefcase,
    app.left_view == LeftView::Project && left_open,
    app.hover_progress(HoverId::Rail(RailButton::LeftProject)),
    Message::LeftIconSelect(LeftView::Project),
))
.on_enter(Message::Hover(HoverId::Rail(RailButton::LeftProject), true))
.on_exit(Message::Hover(HoverId::Rail(RailButton::LeftProject), false)),

// 迁移后——出现 1 次:
icons::icon_button_entry(
    icons::IconKind::Briefcase,
    crate::theme::icon_size::rail(),
    app.left_view == LeftView::Project && left_open,
    app.hover_progress(HoverId::Rail(RailButton::LeftProject)),
    Message::LeftIconSelect(LeftView::Project),
    |hovered| Message::Hover(HoverId::Rail(RailButton::LeftProject), hovered),
),
```

`rail_icon_button`(`app.rs:4878`)现有的着色逻辑保持不变,`icon_button_entry`
内部改调 `icons::icon_button` 通用版——两者渲染的图标颜色公式相同
(`active ? GOLD : mix(DIM, GOLD, hover_t)`),迁移不改变外观。左右两个
rail 一共 11 处按同样的模式替换。

### 2. `tabs::tab_core`(新文件 `crates/dozer-app/src/tabs.rs`)

```rust
/// tab 的共享交互内核:选中(mousedown 即选中+备拖,`MouseArea::on_press`
/// ——2026-08-12 顶栏拖窗 bug 的根因修复)与关闭按钮(未 hover 时不挂
/// `on_press`,避免隐形 × 吃掉点击)。返回 `(select, close)` 两个独立
/// `Element`,调用方仍自己决定怎么拼(`stack!`/`row!`)、外层背景与尺寸。
/// `close_color` 由调用方预先算好传入(两个现有实现各自的 hover 混色/
/// alpha 公式不完全相同——项目页签用组合 hover 的 alpha,面板 tab 只用
/// 自己的 `close_hover_t`——这个差异保留在调用方,`tab_core` 不替调用方
/// 做选择)。`close_interactive` 对应"未悬停时不挂 on_press"这条规则,
/// 调用方传 `close_hover_t > 0.001`(两个现有实现的判定条件一致)。
pub fn tab_core<'a, M: Clone + 'a>(
    content: Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>,
    close_sz: f32,
    close_color: Color,
    close_interactive: bool,
    on_select: M,
    on_close: M,
    on_select_hover: impl Fn(bool) -> M + 'a,
    on_close_hover: impl Fn(bool) -> M + 'a,
) -> (
    Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>,
    Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>,
) {
    let select = MouseArea::new(content)
        .on_press(on_select)
        .on_enter(on_select_hover(true))
        .on_exit(on_select_hover(false))
        .interaction(iced_widget::core::mouse::Interaction::Pointer)
        .into();

    let close_btn = button(
        container(
            text("×")
                .size(theme::font::body())
                .color(close_color)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(iced_widget::core::alignment::Horizontal::Center)
                .align_y(iced_widget::core::alignment::Vertical::Center),
        ),
    )
    .width(Length::Fixed(close_sz))
    .height(Length::Fixed(close_sz))
    .padding(0)
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: close_color,
        ..button::Style::default()
    });
    let close_btn = if close_interactive {
        close_btn.on_press(on_close)
    } else {
        close_btn
    };
    let close = MouseArea::new(close_btn)
        .on_enter(on_close_hover(true))
        .on_exit(on_close_hover(false))
        .into();

    (select, close)
}
```

这与当前 `project_tab_item`/`panel_tab` 各自的 close 按钮构造逐行对应
(`app.rs` 里两处 `close_btn`/`close` 的现有写法),搬过来即是,没有需要
在实现阶段另行设计的部分。

**试点用例**(`project_tab_item`,`app.rs:4646` 起的 `select` 构造):

```rust
// 迁移前:project_tab_item 自己手写 MouseArea + on_press + on_enter/on_exit。
let select = MouseArea::new(container(label).width(Length::Fill).height(Length::Fixed(tab_h)))
    .on_press(Message::ProjectTabSwitch(id))
    .on_enter(Message::Hover(HoverId::ProjectTabItem(id), true))
    .on_exit(Message::Hover(HoverId::ProjectTabItem(id), false))
    .interaction(mouse::Interaction::Pointer);
// ...close 按钮同理手写一份。

// 迁移后:
let close_base = theme::color::mix(theme::color::DIM, theme::color::GOLD, close_hover_t);
let close_color = Color { a: hover, ..close_base }; // hover = title_hover_t.max(close_hover_t),现有逻辑不变
let (select, close) = tabs::tab_core(
    container(label).width(Length::Fill).height(Length::Fixed(tab_h)).into(),
    close_sz,
    close_color,
    hover > 0.001,
    Message::ProjectTabSwitch(id),
    Message::ProjectTabClose(id),
    move |hovered| Message::Hover(HoverId::ProjectTabItem(id), hovered),
    move |hovered| Message::Hover(HoverId::ProjectTabClose(id), hovered),
);
// project_tab_item 自己继续拼 stack![capsule_layer, select_layer, close_layer]、
// 自己的 capsule 背景、自己的 tab_h/padding——这些都不进 tab_core。
```

`panel_tab` 暂不改动,但保留同样的迁移路径以供后续参考。

## 错误处理

不适用——两个新函数都是纯组合,没有可能失败的操作,也不引入需要处理的
新状态。

## 测试策略

这是渲染层重构,没有新的可单测逻辑(`icon_button_entry`/`tab_core` 内部
只是搭 widget 树)。验证手段沿用当天已经用过三次、证明可靠的方法:

1. `cargo build -p dozer-app --bin dozer` 编译通过,`cargo fmt -p dozer-app
   -- --check` 无差异。
2. `cargo test -p dozer-app --bin dozer` 通过——预期与当前 `main` 一致的
   484 passed / 2 failed(两个已知的、与本次改动无关的 terminal grid
   尺寸测试失败,`app-workspace-split` 合并前就存在)。
3. 构建一个独立命名的临时二进制(如 `cp target/.../dozer /tmp/dozer-test-
   <topic>`,绝不用会与用户正在跑的正式 `/Applications/Dozer AI Coder.app`
   或其他会话的调试实例撞进程名的路径),用 `osascript`/`cliclick` 精确
   定位到该临时实例的窗口(按 PID 过滤,不用 `tell process "dozer"` 这种
   会撞到别的 dozer 实例的写法),截图对比:
   - 左右 rail 11 个按钮:静止态图标颜色、hover 过渡、点击切换面板,
     与迁移前逐一比对无差异。
   - 项目页签:mousedown 即选中(不依赖 mouseup)、拖拽换位不再触发原生
     拖窗、关闭按钮仅 hover 时可点、悬停胶囊背景左缘与状态点保留间距。
   完成后关闭该临时实例、删除临时二进制,不留后台进程。

## 排期备注

两个试点(`icon_button_entry` 迁移 rail、`tab_core` 迁移
`project_tab_item`)touch 的是同一个文件(`app.rs`)但不同函数、不重叠
的代码区域,互相之间没有依赖,可以按任意顺序做、也可以分两次提交分两次
审阅。`tabs.rs` 是新文件,不会和任何现有改动冲突。
