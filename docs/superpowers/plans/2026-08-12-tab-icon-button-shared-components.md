# Tab / icon 按钮共享组件 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增两个无状态的共享 UI 组件——`icons::icon_button_entry`(图标按钮的 hover/点击接线)与 `tabs::tab_core`(tab 的选中/关闭接线)——并分别迁移一个试点调用点(左右图标栏共 11 个按钮;顶栏项目页签),消除"同一交互逻辑在多处各写一份、容易悄悄长歪"的重复模式。

**Architecture:** `icon_button_entry`加在现有 `crates/dozer-app/src/icons.rs`,包一层 `MouseArea` 把 `on_press`/`on_enter`/`on_exit` 接好,调用方只传一个 `on_hover: impl Fn(bool) -> M` 闭包。`tab_core` 是新文件 `crates/dozer-app/src/tabs.rs`,返回 `(select, close)` 两个 `Element`,调用方仍自己拼外层布局/背景/尺寸——两个组件都是纯 `fn(参数) -> Element<M>`,不持有 `App`/`Workspace`,不引入新状态或新消息类型。

**Tech Stack:** Rust workspace;iced 0.14(`iced_widget`);不新增依赖。

## Global Constraints

- **在独立分支上开发**:建分支 `feature/tab-icon-button-shared-components`(或对应 worktree),完成后提请审阅,通过再合并回 `main`。
- **这个计划基于当前 `main`(commit `ea05f1b`)分析**。这个仓库的工作目录目前被多个并行会话共享,`app.rs`/`main.rs` 随时可能已经被其他分支的改动挪动了行号——开工前先用本文档给出的函数名/`grep -n` 关键字符串核对你实际面对的代码形状,不要死认文档里写的具体行号。
- **不做任何视觉改版**:像素级尺寸、颜色、动画曲线必须与迁移前完全一致。这是一次纯粹的"消除重复"重构。
- **不合并 `project_tab_item` 与 `panel_tab`**,**不迁移 `panel_tab`**,**不迁移 rail 之外的 icon 按钮调用点**——这些都是设计文档明确的非目标,不在本计划范围内。
- **不引入新状态、新消息类型、新持久化字段**。`HoverId`/`hover_anims`/`Message::Hover`/`Message::LeftIconSelect`/`Message::RightIconSelect`/`Message::ProjectTabSwitch`/`Message::ProjectTabClose` 全部保持原样。
- 每个任务结束都要 `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer && cargo fmt -p dozer-app -- --check` 干净通过——`cargo test` 预期 484 passed / 2 failed(`terminal_grid_state_sizes_hidden_terminal_as_if_shown`/`terminal_pane_pixel_size_right_maximized_matches_overlay_box`,这两个是 `main` 上已有的、与本计划无关的既有失败,不是本计划引入的回归;若失败数字或失败用例变了,先停下来查清楚是不是本计划的改动导致的)。
- 设计文档:`docs/superpowers/specs/2026-08-12-tab-icon-button-shared-components-design.md`,有疑问以它为准。
- **人工/截图验证时绝不能碰用户正在跑的正式 app**。用户的日常正式实例是
  `/Applications/Dozer AI Coder.app`(进程名同样是 `dozer`,`osascript`/
  `System Events` 按名字找进程时会连正式实例一起选中)。每次验证前按下面
  的方法建一个**独一无二命名**的临时二进制、只用它的 PID 定位窗口:
  ```bash
  cargo build -p dozer-app --bin dozer
  cp ./target/aarch64-apple-darwin/debug/dozer /tmp/dozer-test-<topic>
  nohup /tmp/dozer-test-<topic> > /tmp/dozer-test-<topic>.log 2>&1 &
  # 记下这里打印的 pid,只用它,不要用 "tell process \"dozer\""
  osascript -e 'tell application "System Events" to get name of every process whose unix id is <pid>'
  # 应该原样打印 "dozer-test-<topic>"——确认这一步成功再继续操作,
  # 否则后续 "tell process ..." 有极小概率撞上其它同名 dozer 实例
  osascript -e 'tell application "System Events" to tell process "dozer-test-<topic>" to set frontmost to true'
  osascript -e 'tell application "System Events" to tell process "dozer-test-<topic>" to set position of window 1 to {40, 60}'
  ```
  验证结束后 `kill <pid>` 并 `rm -f /tmp/dozer-test-<topic>*`,不留后台进程。

---

### Task 1: `icons::icon_button_entry` + 迁移左右图标栏 11 个按钮

**Files:**
- Modify: `crates/dozer-app/src/icons.rs`(加 `icon_button_entry`)
- Modify: `crates/dozer-app/src/app.rs`(`left_icon_rail`/`right_icon_rail`,当前分别在 `fn left_icon_rail` / `fn right_icon_rail` 处——用 `grep -n "^fn left_icon_rail\|^fn right_icon_rail" crates/dozer-app/src/app.rs` 定位你实际面对的行号)

**Interfaces:**
- Produces:`pub fn icon_button_entry<'a, M: Clone + 'a>(kind: IconKind, size: f32, active: bool, hover_t: f32, on_select: M, on_hover: impl Fn(bool) -> M + 'a) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>`(`icons.rs`,新函数)。

- [ ] **Step 1: 在 `icons.rs` 加 `icon_button_entry`**

先在文件顶部的 `use` 块补两个新引入(现有 `use` 在 `icons.rs:5-9`):

```rust
use iced_widget::button;
use iced_widget::container;
use iced_widget::core::{Border, Color, Element, Length};
use iced_widget::core::mouse;
use iced_widget::svg;
use iced_widget::MouseArea;
use std::path::Path;
```

紧跟在现有 `pub fn icon_button` 定义(`icons.rs:214` 起)结束的 `}` 之后加:

```rust
/// `icon_button` 的完整接线版:图标按钮 + hover 动画 + 点击,一次性把
/// `MouseArea`(`Pointer` 光标 + `on_press`/`on_enter`/`on_exit`)接好。
/// 调用方只需算好 `hover_t`(通常是 `app.hover_progress(some_id)`,多数
/// view 函数拿不到 `&App`,这一步仍留给调用方)和一个 `on_hover` 闭包——
/// 闭包内部才知道具体 `HoverId`,`icon_button_entry` 本身不认识 `HoverId`
/// (定义在 `app.rs`,`icons.rs` 不依赖 `app.rs`)。`card` 恒传 `false`——
/// 目前所有调用点都是无卡片底的纯图标按钮;真需要卡片底时再加一个 `card`
/// 参数,不预留占位参数。
pub fn icon_button_entry<'a, M: Clone + 'a>(
    kind: IconKind,
    size: f32,
    active: bool,
    hover_t: f32,
    on_select: M,
    on_hover: impl Fn(bool) -> M + 'a,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    MouseArea::new(icon_button(kind, size, active, hover_t, false).on_press(on_select))
        .interaction(mouse::Interaction::Pointer)
        .on_enter(on_hover(true))
        .on_exit(on_hover(false))
        .into()
}
```

- [ ] **Step 2: 编译确认新函数无误(此时还没有调用点,预期一条 `dead_code` warning)**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功,新增一条 `warning: function icon_button_entry is never used`(Step 4 接上调用点后这条 warning 消失)。

- [ ] **Step 3: 迁移 `left_icon_rail`**

用 `grep -n "^fn left_icon_rail" crates/dozer-app/src/app.rs` 找到函数起点,把整个函数体替换成:

```rust
fn left_icon_rail(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::left_icon_rail();
    let left_open = !app.left_collapsed;
    let content = column![
        // Project 信息面板入口：项目名 / git 分支+脏标 / 验收次数 / 可编辑目标。
        // 置顶(用户 2026-08-11 指定)。
        icons::icon_button_entry(
            icons::IconKind::Briefcase,
            crate::theme::icon_size::rail(),
            app.left_view == LeftView::Project && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftProject)),
            Message::LeftIconSelect(LeftView::Project),
            |hovered| Message::Hover(HoverId::Rail(RailButton::LeftProject), hovered),
        ),
        // Todo 面板入口：`.dozer/todo.md` 任务列表。第二顺位(用户 2026-08-11
        // 指定)。
        icons::icon_button_entry(
            icons::IconKind::ListTodo,
            crate::theme::icon_size::rail(),
            app.left_view == LeftView::Todo && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftTodo)),
            Message::LeftIconSelect(LeftView::Todo),
            |hovered| Message::Hover(HoverId::Rail(RailButton::LeftTodo), hovered),
        ),
        // 文件列表入口：项目树 + 文件预览配对。
        icons::icon_button_entry(
            icons::IconKind::FolderTree,
            crate::theme::icon_size::rail(),
            app.left_view == LeftView::Files && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftFiles)),
            Message::LeftIconSelect(LeftView::Files),
            |hovered| Message::Hover(HoverId::Rail(RailButton::LeftFiles), hovered),
        ),
        // spike(2026-08-06):Git 提交图入口,验证 gleisbau 库可行性用。
        icons::icon_button_entry(
            icons::IconKind::GitGraph,
            crate::theme::icon_size::rail(),
            app.left_view == LeftView::GitLog && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftGit)),
            Message::LeftIconSelect(LeftView::GitLog),
            |hovered| Message::Hover(HoverId::Rail(RailButton::LeftGit), hovered),
        ),
        // 数据库面板入口:数据源管理 + 连接测试。
        icons::icon_button_entry(
            icons::IconKind::Database,
            crate::theme::icon_size::rail(),
            app.left_view == LeftView::Database && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftDatabase)),
            Message::LeftIconSelect(LeftView::Database),
            |hovered| Message::Hover(HoverId::Rail(RailButton::LeftDatabase), hovered),
        ),
        // SSH 主机面板入口。
        icons::icon_button_entry(
            icons::IconKind::Server,
            crate::theme::icon_size::rail(),
            app.left_view == LeftView::Ssh && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftSsh)),
            Message::LeftIconSelect(LeftView::Ssh),
            |hovered| Message::Hover(HoverId::Rail(RailButton::LeftSsh), hovered),
        ),
        // 浏览器面板入口:左图标栏最底部 Globe 按钮(2026-08-11 从右栏移回)。
        icons::icon_button_entry(
            icons::IconKind::Globe,
            crate::theme::icon_size::rail(),
            app.left_view == LeftView::Web && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftWeb)),
            Message::LeftIconSelect(LeftView::Web),
            |hovered| Message::Hover(HoverId::Rail(RailButton::LeftWeb), hovered),
        ),
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
```

（视觉"选中"= 该视图激活 **且**左面板区展开这条注释语义不变，行为完全对应
迁移前的 `active` 表达式，只是渲染改走 `icon_button_entry`。）

- [ ] **Step 4: 迁移 `right_icon_rail`**

同样用 `grep -n "^fn right_icon_rail"` 定位，把函数体替换成：

```rust
fn right_icon_rail(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::right_icon_rail();
    let right_open = !app.right_collapsed;
    let content = column![
        icons::icon_button_entry(
            icons::IconKind::Brain,
            crate::theme::icon_size::rail(),
            app.right_view == RightView::Agent && right_open,
            app.hover_progress(HoverId::Rail(RailButton::RightAgent)),
            Message::RightIconSelect(RightView::Agent),
            |hovered| Message::Hover(HoverId::Rail(RailButton::RightAgent), hovered),
        ),
        icons::icon_button_entry(
            icons::IconKind::BotMessageSquare,
            crate::theme::icon_size::rail(),
            app.right_view == RightView::Conversations && right_open,
            app.hover_progress(HoverId::Rail(RailButton::RightConversations)),
            Message::RightIconSelect(RightView::Conversations),
            |hovered| Message::Hover(HoverId::Rail(RailButton::RightConversations), hovered),
        ),
        icons::icon_button_entry(
            icons::IconKind::BarChart3,
            crate::theme::icon_size::rail(),
            app.right_view == RightView::Usage && right_open,
            app.hover_progress(HoverId::Rail(RailButton::RightUsage)),
            Message::RightIconSelect(RightView::Usage),
            |hovered| Message::Hover(HoverId::Rail(RailButton::RightUsage), hovered),
        ),
        {
            let pending = app
                .active_workspace()
                .and_then(|ws| ws.tabs.get(ws.active))
                .map(|t| t.delivery_pending)
                .unwrap_or(false);
            let base: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
                icons::icon_button_entry(
                    icons::IconKind::BadgeCheck,
                    crate::theme::icon_size::rail(),
                    app.right_view == RightView::Acceptance && right_open,
                    app.hover_progress(HoverId::Rail(RailButton::RightAcceptance)),
                    Message::RightIconSelect(RightView::Acceptance),
                    |hovered| Message::Hover(HoverId::Rail(RailButton::RightAcceptance), hovered),
                );
            if pending {
                let badge: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
                    stack![
                        base,
                        container(iced_widget::Space::new())
                            .width(Length::Fixed(8.0))
                            .height(Length::Fixed(8.0))
                            .style(|_t: &iced_widget::Theme| container::Style {
                                background: Some(theme::color::GOLD.into()),
                                border: Border {
                                    radius: 4.0.into(),
                                    ..Border::default()
                                },
                                ..container::Style::default()
                            }),
                    ]
                    .into();
                badge
            } else {
                base
            }
        },
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
```

`rail_icon_button`(`app.rs:4878`)**不要删**——`grep -n "rail_icon_button"
crates/dozer-app/src/*.rs` 会看到 `homespace.rs`(首页左栏，`homespace.rs:9`
的 `use` 列表里显式引入)还有 3 处调用，不在这次迁移范围内(设计文档非目标
明确写了"不迁移 rail 之外的 icon 按钮调用点"，首页 rail 是另一个独立调用
点)。`icon_button_entry` 内部改调的是通用版 `icons::icon_button`(着色公式
与 `rail_icon_button` 相同,只是不共享同一份代码),两者并存,`rail_icon_button`
迁移后应该**不会**报 `dead_code`——若报了,说明 `homespace.rs` 的调用点被
误删或改动了,回去检查 Step 3/4 有没有不小心动到 `left_icon_rail`/
`right_icon_rail` 之外的地方。

- [ ] **Step 5: 编译 + 格式检查**

Run: `cargo build -p dozer-app --bin dozer && cargo fmt -p dozer-app -- --check`
Expected: 编译成功、无 warning（`icon_button_entry` 已有调用点，`rail_icon_button`
若被删掉则不会再报 dead_code）、`fmt --check` 无差异（如有差异先 `cargo fmt -p
dozer-app` 再重新 check）。

- [ ] **Step 6: `cargo test`**

Run: `cargo test -p dozer-app --bin dozer`
Expected: 484 passed / 2 failed(两个已知的 terminal grid 尺寸测试，与本任务无关)。

- [ ] **Step 7: 人工截图验证**

按 Global Constraints 的临时二进制方法起一个 `dozer-test-rail` 实例，截图确认：
- 左栏 7 个图标(Project/Todo/Files/GitLog/Database/Ssh/Web)、右栏 3+1 个图标
  (Agent/Conversations/Usage/Acceptance)静止态颜色为 DIM，选中态(对应
  `LeftView`/`RightView` 当前值且对应侧面板展开)恒金色。
- 依次 hover 每个图标，确认颜色从 DIM 平滑过渡到金(不是瞬间跳变)、光标
  变成 Pointer 手型。
- 点击任一未选中图标，确认对应面板打开、图标变金；再点一次已选中图标，
  确认面板收起、图标退回 DIM。
- Acceptance 图标在 `delivery_pending` 为真时右上角有金色小圆点徽标(若当前
  没有 pending 交付可跳过这一项，不强求造数据验证)。

验证完关闭该临时实例、删除临时二进制。

- [ ] **Step 8: 提交**

```bash
git add crates/dozer-app/src/icons.rs crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
refactor(app): icon 按钮 hover/点击接线收敛成 icon_button_entry

新增 icons::icon_button_entry,把 MouseArea 的 on_press/on_enter/on_exit
三件套封装成一次调用,调用方只传一个 on_hover 闭包——此前每个 rail 按钮
都要在 hover_progress 查询/on_enter/on_exit 三处手动保持同一个 HoverId,
改错一处不会编译报错、只会表现成运行时 hover 动画不对。左右图标栏 11 个
按钮全部迁移,rail_icon_button 不再被调用后删除。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `tabs::tab_core` + 迁移项目页签

**Files:**
- Create: `crates/dozer-app/src/tabs.rs`
- Modify: `crates/dozer-app/src/main.rs`(加 `mod tabs;`，插在 `mod scrollbar;` 和 `mod term_model;` 之间，保持字母序)
- Modify: `crates/dozer-app/src/app.rs`(`project_tab_item` 里 `select`/`close`/`close_btn` 的构造——用 `grep -n "^fn project_tab_item"` 定位）

**Interfaces:**
- Produces:`pub fn tab_core<'a, M: Clone + 'a>(content: Element<'a, M, ...>, close_sz: f32, close_color: Color, close_interactive: bool, on_select: M, on_close: M, on_select_hover: impl Fn(bool) -> M + 'a, on_close_hover: impl Fn(bool) -> M + 'a) -> (Element<'a, M, ...>, Element<'a, M, ...>)`（`tabs.rs`，新函数，返回 `(select, close)`）。

- [ ] **Step 1: 新建 `crates/dozer-app/src/tabs.rs`**

```rust
//! tab 的共享交互内核——选中(mousedown 即选中+备拖)与关闭按钮(仅悬停时
//! 可点)。只管交互接线，不管布局/尺寸/背景：调用方(`project_tab_item`/
//! 未来的 `panel_tab`)各自决定怎么拼 `stack!`/`row!`、怎么画背景。见
//! `docs/superpowers/specs/2026-08-12-tab-icon-button-shared-components-design.md`。

use iced_widget::core::mouse;
use iced_widget::core::{Color, Element, Length};
use iced_widget::{MouseArea, button, container, text};

/// 返回 `(select, close)` 两个独立 `Element`。`content` 是调用方已经拼好
/// 的 tab 主体(通常是状态点+标题的一个 row，包在合适尺寸的 container
/// 里)。`close_color`/`close_interactive` 由调用方预先算好传入——两个
/// 现有实现(`project_tab_item`/`panel_tab`)的 hover 混色公式不完全相同
/// (项目页签用"标题 hover 与关闭 hover 取最大值"的组合 alpha,面板 tab
/// 只用自己的 `close_hover_t`),这个差异保留在调用方，`tab_core` 不替
/// 调用方做选择。
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
        .interaction(mouse::Interaction::Pointer)
        .into();

    let close_btn = button(
        container(text("×").size(crate::theme::font::body()).color(close_color))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced_widget::core::Alignment::Center)
            .align_y(iced_widget::core::Alignment::Center),
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

- [ ] **Step 2: 在 `main.rs` 注册模块**

在 `main.rs:20`(`mod scrollbar;`)之后、`mod term_model;` 之前插入一行：

```rust
mod tabs;
```

- [ ] **Step 3: 编译确认新模块无误(此时还没有调用点，预期一条 `dead_code` warning)**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功，新增一条 `warning: function tab_core is never used`（Step 4
接上调用点后这条 warning 消失）。

- [ ] **Step 4: 迁移 `project_tab_item` 的 `select`/`close`/`close_btn` 构造**

用 `grep -n "^fn project_tab_item"` 定位函数，找到从 `// 选中改走
\`MouseArea::on_press\`` 这条注释开始、到 `let close_layer = ...` 之前
（即 `select`/`close_base`/`close_color`/`close_btn`/`close` 这几个
`let` 绑定）的整段，替换成：

```rust
    // 选中/关闭的接线逻辑收在 `tabs::tab_core`(2026-08-12 抽取)——
    // mousedown 即选中+备拖、关闭按钮仅悬停时可点这两条规则只在一处维护。
    let close_base = theme::color::mix(theme::color::DIM, theme::color::GOLD, close_hover_t);
    let close_color = Color {
        a: hover,
        ..close_base
    };
    let (select, close) = tabs::tab_core(
        container(label)
            .width(Length::Fill)
            .height(Length::Fixed(tab_h))
            .into(),
        close_sz,
        close_color,
        hovered,
        Message::ProjectTabSwitch(id),
        Message::ProjectTabClose(id),
        move |hovered| Message::Hover(HoverId::ProjectTabItem(id), hovered),
        move |hovered| Message::Hover(HoverId::ProjectTabClose(id), hovered),
    );
```

（`hovered` 变量已经在函数前半部分算好——`let hovered = hover > 0.001;`，
就是原来 `close_btn.on_press` 的判定条件，直接复用，不用重新算。）

- [ ] **Step 5: 编译 + 格式检查**

Run: `cargo build -p dozer-app --bin dozer && cargo fmt -p dozer-app -- --check`
Expected: 编译成功、无 warning、`fmt --check` 无差异。

- [ ] **Step 6: `cargo test`**

Run: `cargo test -p dozer-app --bin dozer`
Expected: 484 passed / 2 failed(同 Task 1，两个已知的既有失败）。

- [ ] **Step 7: 人工截图验证**

按 Global Constraints 的临时二进制方法起一个 `dozer-test-projecttab` 实例，
打开至少 3 个项目页签，截图确认：
- **mousedown 即选中**:鼠标在某未选中页签上按下（不需要松开）就能看到它
  变成选中态（金色文字+实底背景），不依赖 mouseup。
- **拖拽不再触发原生拖窗**:在页签上按住拖动，整个 dozer 窗口不会跟着移动
  （这是 2026-08-12 已经修过的 bug，这里是确认这次重构没有让它复发）。
- **空白顶栏仍可拖窗**:在页签之外的空白顶栏区域按住拖动，窗口正常跟着
  移动。
- **关闭按钮仅悬停可点**:鼠标离开页签后 × 不可见也不可点；悬停页签任意
  位置后 × 显形且可点，点击后该页签关闭。
- **悬停胶囊背景与状态点保留间距**:悬停未选中页签时，金色状态点与胶囊
  背景圆角左缘之间有明显间距（不是贴在一起）——这是 2026-08-12 已经修
  过的另一个 bug，同样是确认没有复发。

验证完关闭该临时实例、删除临时二进制。

- [ ] **Step 8: 提交**

```bash
git add crates/dozer-app/src/tabs.rs crates/dozer-app/src/main.rs crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
refactor(app): 项目页签选中/关闭接线收敛成 tabs::tab_core

新增 tabs.rs::tab_core,把 tab 的 mousedown-即选中(2026-08-12 拖窗 bug
的根因修复)与关闭按钮悬停才可点这两条交互规则从 project_tab_item 里
抽出来,返回 (select, close) 两个 Element,布局/背景/尺寸仍由调用方
决定。panel_tab 暂不迁移(当前正确,不动没坏的代码),留作后续按同样
路径迁移的参考实现。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## 完工检查

两个任务都完成后：

- [ ] `cargo build -p dozer-app --bin dozer` 全量编译无 warning。
- [ ] `cargo test -p dozer-app --bin dozer` 仍是 484 passed / 2 failed（与开工前一致）。
- [ ] `cargo fmt -p dozer-app -- --check` 无差异。
- [ ] `cargo clippy -p dozer-app --all-targets` 没有新增 lint（对照开工前的 baseline，若不确定先在开工前跑一次记下基线）。
- [ ] 分支上没有遗留的临时测试二进制/进程(`pgrep -fla dozer-test`应该空)。
- [ ] 提请审阅，通过后合并回 `main`。
