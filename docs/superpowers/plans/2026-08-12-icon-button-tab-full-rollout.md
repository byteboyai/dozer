# icon 按钮 / tab 共享组件全量推广 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `icons::icon_button_entry` 泛化(加 `card`/`button_size`/`interactive` 三个参数),迁移 7 个符合条件的面板操作 icon 按钮,`panel_tab` 迁移到 `tabs::tab_core`。

**Architecture:** 纯组件参数扩展 + 调用点替换,不改变任何可观察行为。11 个已迁移的 rail 调用点补三个显式实参(与当前隐式行为完全相同);`browser`/`database`/`files`/`usage` 7 处已用 `icons::icon_button()` 的面板操作按钮换成 `icons::icon_button_entry(...)`;`panel_tab` 内部的 `select`/`close_btn`/`close` 构造换成一次 `tabs::tab_core(...)` 调用。

**Tech Stack:** Rust workspace;不新增依赖。

## Global Constraints

- **在独立分支上开发**:建分支 `feature/icon-button-tab-full-rollout`(或对应 worktree),完成后提请审阅,通过再合并回 `main`。
- **这个计划基于当前 `main`(commit `10ed585`)分析**。工作目录被多个并行会话共享,开工前用本文档的 `grep -n` 模式核对实际行号。
- **不改变任何可观察行为**——像素级尺寸、颜色、动画曲线、点击可用性(尤其是收藏星标按钮"无 URL 时不可点"这条)必须与迁移前完全一致。
- 每个任务结束都要 `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer && cargo fmt -p dozer-app -- --check` 干净通过。`cargo test` 预期 483 passed / 3 failed(两个 terminal grid + 一个 git_log 既有失败,均与本计划无关;若数字不同,先按当前 main 单独确认这些失败是否本来就存在,不要假设是本计划引入的)。
- **人工验证时绝不能碰用户正在跑的正式 app**。用独立命名的临时二进制(如 `cp target/aarch64-apple-darwin/debug/dozer /tmp/dozer-test-<topic>`),只用它的 PID/进程名定位窗口,不要用 `tell process "dozer"`(会撞上 `/Applications/Dozer AI Coder.app`)。验证完 `kill` 并删除临时二进制。
- 设计文档:`docs/superpowers/specs/2026-08-12-icon-button-tab-full-rollout-design.md`,有疑问以它为准。

---

### Task 1: `icon_button_entry` 泛化 + 11 个 rail 调用点补参数

**Files:**
- Modify: `crates/dozer-app/src/icons.rs`
- Modify: `crates/dozer-app/src/app.rs`(`left_icon_rail`/`right_icon_rail`)

**Interfaces:**
- Produces:`icon_button_entry` 新签名 `pub fn icon_button_entry<'a, M: Clone + 'a>(kind: IconKind, size: f32, active: bool, hover_t: f32, card: bool, button_size: f32, interactive: bool, on_select: M, on_hover: impl Fn(bool) -> M + 'a) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>`(在 `card`/`button_size`/`hover_t` 之后、`on_select` 之前插入 `interactive`)。

- [ ] **Step 1: 定位 `icon_button_entry`**

```bash
command grep -n "pub fn icon_button_entry" crates/dozer-app/src/icons.rs
```

预期在 `icons.rs:287`。

- [ ] **Step 2: 替换整个函数体**

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
    // 图标颜色:选中态恒为金;未选中时 hover 平滑过渡到金(SVG 颜色构建时
    // 定死、不吃 `button::Status`,所以 hover 进度靠 `hover_t` 参数从调用方
    // 算进来)。
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
            // 圆角正方形背景常驻(`card` 为真时);金色外框只在选中态出现,
            // hover 不放金框——所以样式完全由 `active`/`card` 决定,与
            // 交互态无关。
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
    // `interactive` 为假时不挂 `on_press`——收藏星标按钮在没有 URL 时应该
    // 不可点(2026-08-12 全量推广时发现,`browser.rs` 的星标按钮此前用的
    // 是"条件挂载 on_press"这个手法,不能无条件套用)。
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

- [ ] **Step 3: 编译确认签名变化(此时调用点还没更新,预期报错)**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译失败,报 11 处 `icon_button_entry` 调用参数数量不对(Step 4 更新完调用点后消失)。

- [ ] **Step 4: 更新 `left_icon_rail`(11 处里的 7 处)**

```bash
command grep -n "^fn left_icon_rail" crates/dozer-app/src/app.rs
```

预期在 `app.rs:5056`。把函数体内 7 处 `icons::icon_button_entry(...)` 调用,在
`app.hover_progress(...)` 之后、`Message::LeftIconSelect(...)` 之前,插入三行
新实参(以 Project 那一处为例,其余 6 处同理逐一插入):

```rust
        icons::icon_button_entry(
            icons::IconKind::Briefcase,
            crate::theme::icon_size::rail(),
            app.left_view == LeftView::Project && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftProject)),
            true,
            crate::theme::geometry::rail_button_size(),
            true,
            Message::LeftIconSelect(LeftView::Project),
            |hovered| Message::Hover(HoverId::Rail(RailButton::LeftProject), hovered),
        ),
```

其余 6 处(`ListTodo`/`FolderTree`/`GitGraph`/`Database`/`Server`/`Globe`)
按同样的位置插入同样的三行(`true, crate::theme::geometry::rail_button_size(), true,`),
只是紧邻的 `IconKind`/`LeftView`/`RailButton` 变体不同,其余参数不变。

- [ ] **Step 5: 更新 `right_icon_rail`(11 处里剩下的 4 处)**

```bash
command grep -n "^fn right_icon_rail" crates/dozer-app/src/app.rs
```

预期在 `app.rs:5144`。同样在 4 处 `icons::icon_button_entry(...)` 调用里插入
`true, crate::theme::geometry::rail_button_size(), true,`(`Brain`/
`BotMessageSquare`/`BarChart3`/`BadgeCheck` 各一处,`BadgeCheck` 那处在一个
`{ let pending = ...; ... }` 代码块里,插入方式相同)。

- [ ] **Step 6: 编译 + 格式检查**

Run: `cargo build -p dozer-app --bin dozer && cargo fmt -p dozer-app -- --check`
Expected: 编译成功、无新增 warning、`fmt --check` 无差异。

- [ ] **Step 7: `cargo test`**

Run: `cargo test -p dozer-app --bin dozer`
Expected: 483 passed / 3 failed(三个已知的既有失败)。

- [ ] **Step 8: 人工验证**

按 Global Constraints 的临时二进制方法起一个 `dozer-test-railparams` 实例,
截图确认左右 rail 共 11 个按钮的静止态/hover 过渡/选中态外观与迁移前
(补参数前)完全一致——这一步理论上不该有任何视觉差异,纯粹是把隐式
硬编码值变成显式实参。

- [ ] **Step 9: 提交**

```bash
git add crates/dozer-app/src/icons.rs crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
refactor(app): icon_button_entry 加 card/button_size/interactive 参数

此前硬编码 rail 专属视觉(常驻卡片底+固定 rail 尺寸+无条件 on_press),
服务不了其余不同形状的图标按钮。补三个参数补齐底层 icon_button() 已有
的灵活性,11 个 rail 调用点同步补显式实参(card: true, button_size:
rail_button_size(), interactive: true),视觉/行为不变。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: 迁移 7 个面板操作 icon 按钮

**Files:**
- Modify: `crates/dozer-app/src/extensions/browser.rs`
- Modify: `crates/dozer-app/src/extensions/database.rs`
- Modify: `crates/dozer-app/src/extensions/files.rs`
- Modify: `crates/dozer-app/src/extensions/usage.rs`

**Interfaces:**
- Consumes:`icons::icon_button_entry`(Task 1 产出的新签名)。

- [ ] **Step 1: `browser.rs` 收藏星标按钮(`browser.rs:1108` 附近)**

```bash
command grep -n "let mut btn = icons::icon_button(" crates/dozer-app/src/extensions/browser.rs
```

原文(函数 `star_button` 或同名的收藏按钮构造函数,`browser.rs:1108-1124`):

```rust
    let mut btn = icons::icon_button(
        icons::IconKind::Star,
        icon_size::row(),
        starred,
        state.star_hover(),
        false,
    )
    .width(Length::Fixed(theme::geometry::tab_button_size()))
    .height(Length::Fixed(theme::geometry::tab_button_size()));
    if url.is_some() {
        btn = btn.on_press(Message::StarClick);
    }
    MouseArea::new(btn)
        .interaction(iced_widget::core::mouse::Interaction::Pointer)
        .on_enter(Message::Hover(STAR_HOVER_KEY, false, true))
        .on_exit(Message::Hover(STAR_HOVER_KEY, false, false))
        .into()
```

改成:

```rust
    icons::icon_button_entry(
        icons::IconKind::Star,
        icon_size::row(),
        starred,
        state.star_hover(),
        false,
        theme::geometry::tab_button_size(),
        url.is_some(),
        Message::StarClick,
        |hovered| Message::Hover(STAR_HOVER_KEY, false, hovered),
    )
```

（原来的 `on_enter`/`on_exit` 各自发一次 `Message::Hover(STAR_HOVER_KEY,
false, true/false)`,现在收进 `icon_button_entry` 的 `on_hover` 闭包里,
`hovered` 参数就是原来 `true`/`false` 那个位置——闭包体内直接透传。）

- [ ] **Step 2: `browser.rs` 收藏夹开关按钮(`bookmarks_toggle_button`,`browser.rs:1423` 附近)**

```bash
command grep -n "fn bookmarks_toggle_button" crates/dozer-app/src/extensions/browser.rs
```

原文(整个函数体):

```rust
fn bookmarks_toggle_button(
    state: &State,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let btn = icons::icon_button(
        icons::IconKind::Bookmark,
        icon_size::row(),
        false,
        state.bookmark_hover(),
        false,
    )
    .on_press(Message::BookmarksToggle)
    .width(Length::Fixed(theme::geometry::tab_button_size()))
    .height(Length::Fixed(theme::geometry::tab_button_size()));

    MouseArea::new(btn)
        .interaction(iced_widget::core::mouse::Interaction::Pointer)
        .on_enter(Message::Hover(STAR_HOVER_KEY, true, true))
        .on_exit(Message::Hover(STAR_HOVER_KEY, true, false))
        .into()
}
```

注意这里的 hover 消息复用的也是 `STAR_HOVER_KEY` 这个哨兵键,靠第二个
`bool` 参数(`true`)和星标按钮(`false`)区分,不是笔误,原样保留。改成:

```rust
fn bookmarks_toggle_button(
    state: &State,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    icons::icon_button_entry(
        icons::IconKind::Bookmark,
        icon_size::row(),
        false,
        state.bookmark_hover(),
        false,
        theme::geometry::tab_button_size(),
        true,
        Message::BookmarksToggle,
        |hovered| Message::Hover(STAR_HOVER_KEY, true, hovered),
    )
}
```

- [ ] **Step 3: `database.rs` schema 返回箭头(`database.rs:1488` 附近)**

原文:

```rust
    let back_button = MouseArea::new(
        icons::icon_button(
            icons::IconKind::ChevronLeft,
            crate::theme::icon_size::row(),
            false,
            schema_back_hover_t,
            false,
        )
        .width(Length::Fixed(box_len))
        .height(Length::Fixed(box_len))
        .on_press(Message::SchemaBack),
    )
    .interaction(iced_widget::core::mouse::Interaction::Pointer)
    .on_enter(Message::ToolbarHover(
        DatabaseToolbarTarget::SchemaBack,
        true,
    ))
    .on_exit(Message::ToolbarHover(
        DatabaseToolbarTarget::SchemaBack,
        false,
    ));
```

改成:

```rust
    let back_button = icons::icon_button_entry(
        icons::IconKind::ChevronLeft,
        crate::theme::icon_size::row(),
        false,
        schema_back_hover_t,
        false,
        box_len,
        true,
        Message::SchemaBack,
        |hovered| Message::ToolbarHover(DatabaseToolbarTarget::SchemaBack, hovered),
    );
```

- [ ] **Step 4: `files.rs` 目录搜索按钮(`files.rs:767` 附近)**

原文:

```rust
    let search_button = MouseArea::new(
        icons::icon_button(
            icons::IconKind::FolderSearch,
            crate::theme::icon_size::row(),
            false,
            search_hover_t,
            true,
        )
        .width(Length::Fixed(box_len))
        .height(Length::Fixed(box_len))
        .on_press(Message::SearchSubmit),
    )
    .interaction(iced_widget::core::mouse::Interaction::Pointer)
    .on_enter(Message::ToolbarHover(
        FilesToolbarTarget::SearchSubmit,
        true,
    ))
    .on_exit(Message::ToolbarHover(
        FilesToolbarTarget::SearchSubmit,
        false,
    ));
```

改成:

```rust
    let search_button = icons::icon_button_entry(
        icons::IconKind::FolderSearch,
        crate::theme::icon_size::row(),
        false,
        search_hover_t,
        true,
        box_len,
        true,
        Message::SearchSubmit,
        |hovered| Message::ToolbarHover(FilesToolbarTarget::SearchSubmit, hovered),
    );
```

- [ ] **Step 5: `files.rs` 隐藏文件切换按钮(`files.rs:798` 附近)**

```bash
command grep -n "dotfiles_button = MouseArea::new" crates/dozer-app/src/extensions/files.rs
```

原文(`icons::icon_button(...)` 那部分的 `if dotfiles_shown {...} else {...}`
图标选择表达式整体原样保留,只是外层从 `MouseArea::new(icons::icon_button(...)
...).on_enter(...).on_exit(...)` 换成 `icons::icon_button_entry(...)`):

```rust
    let dotfiles_button = MouseArea::new(
        icons::icon_button(
            if dotfiles_shown {
                icons::IconKind::Eye
            } else {
                icons::IconKind::EyeOff
            },
            crate::theme::icon_size::row(),
            false,
            dotfiles_hover_t,
            true,
        )
        .width(Length::Fixed(box_len))
        .height(Length::Fixed(box_len))
        .on_press(Message::ToggleDotfiles),
    )
```

改成:

```rust
    let dotfiles_button = icons::icon_button_entry(
        if dotfiles_shown {
            icons::IconKind::Eye
        } else {
            icons::IconKind::EyeOff
        },
        crate::theme::icon_size::row(),
        false,
        dotfiles_hover_t,
        true,
        box_len,
        true,
        Message::ToggleDotfiles,
        |hovered| Message::ToolbarHover(FilesToolbarTarget::Dotfiles, hovered),
    );
```

（已核对:`.on_enter`/`.on_exit` 对应的变体确实是
`FilesToolbarTarget::Dotfiles`。）

- [ ] **Step 6: `files.rs` 分支选择器箭头(`files.rs:1107` 附近)**

原文:

```rust
        let switch = MouseArea::new(
            icons::icon_button(
                if ws_state.branch_picker_open {
                    icons::IconKind::ChevronUp
                } else {
                    icons::IconKind::ChevronDown
                },
                crate::theme::icon_size::row(),
                false,
                branch_hover_t,
                false,
            )
            .width(Length::Fixed(box_len))
            .height(Length::Fixed(box_len))
            .on_press(Message::BranchPickerOpen),
        )
        .interaction(iced_widget::core::mouse::Interaction::Pointer)
        .on_enter(Message::ToolbarHover(
            FilesToolbarTarget::BranchSwitch,
            true,
        ))
        .on_exit(Message::ToolbarHover(
            FilesToolbarTarget::BranchSwitch,
            false,
        ));
```

改成:

```rust
        let switch = icons::icon_button_entry(
            if ws_state.branch_picker_open {
                icons::IconKind::ChevronUp
            } else {
                icons::IconKind::ChevronDown
            },
            crate::theme::icon_size::row(),
            false,
            branch_hover_t,
            false,
            box_len,
            true,
            Message::BranchPickerOpen,
            |hovered| Message::ToolbarHover(FilesToolbarTarget::BranchSwitch, hovered),
        );
```

- [ ] **Step 7: `usage.rs` 刷新按钮(`usage.rs:406` 附近)**

原文:

```rust
    let refresh = MouseArea::new(
        icons::icon_button(
            icons::IconKind::RefreshCw,
            crate::theme::icon_size::row(),
            false,
            refresh_hover_t,
            false,
        )
        .width(Length::Fixed(crate::theme::geometry::rail_button_size()))
        .height(Length::Fixed(crate::theme::geometry::rail_button_size()))
        .on_press(Message::Refresh),
    )
    .interaction(iced_widget::core::mouse::Interaction::Pointer)
    .on_enter(Message::Hover(true))
    .on_exit(Message::Hover(false));
```

改成:

```rust
    let refresh = icons::icon_button_entry(
        icons::IconKind::RefreshCw,
        crate::theme::icon_size::row(),
        false,
        refresh_hover_t,
        false,
        crate::theme::geometry::rail_button_size(),
        true,
        Message::Refresh,
        Message::Hover,
    );
```

（`usage.rs` 的 `Message::Hover` 本身就是 `Hover(bool)` 单字段元组变体
——直接把它当函数指针传给 `on_hover: impl Fn(bool) -> M` 即可,不需要
包一层闭包;写计划时用 `command grep -n "Hover(bool)\|Hover(" crates/dozer-app/src/extensions/usage.rs`
确认这一点,若实际不是单字段元组变体,改回 `|hovered| Message::Hover(hovered)`。）

- [ ] **Step 8: 编译 + 格式检查**

Run: `cargo build -p dozer-app --bin dozer && cargo fmt -p dozer-app -- --check`
Expected: 编译成功、无新增 warning、`fmt --check` 无差异。检查 `MouseArea`
的 `use` 引入在这四个文件里是否因为不再直接使用而报 `unused import`——
若某个文件里 `MouseArea` 还有其它调用点在用,不用动 `use`;若这是该文件
唯一的 `MouseArea` 用法,按 warning 提示删掉多余的 `use`。

- [ ] **Step 9: `cargo test`**

Run: `cargo test -p dozer-app --bin dozer`
Expected: 483 passed / 3 failed(三个已知的既有失败)。

- [ ] **Step 10: 人工验证**

按 Global Constraints 的临时二进制方法起一个 `dozer-test-panelbuttons`
实例,逐一确认:
- `files.rs` 目录搜索/隐藏文件切换两个按钮保留卡片底(`card: true`)。
- 其余 5 个按钮(星标、书签、schema 返回、分支选择器、刷新)无卡片底。
- hover 过渡、点击各自对应的功能(打开搜索/切换隐藏文件/收起 schema/
  打开分支选择器/刷新用量)与迁移前一致。
- **收藏星标按钮**:切到一个没有加载任何 URL 的浏览器 tab(或新建一个
  空白 tab),确认星标按钮点击无反应;切到一个有 URL 的 tab,确认点击
  正常弹出收藏菜单——这是本次唯一有行为回归风险的点。

- [ ] **Step 11: 提交**

```bash
git add crates/dozer-app/src/extensions/browser.rs crates/dozer-app/src/extensions/database.rs crates/dozer-app/src/extensions/files.rs crates/dozer-app/src/extensions/usage.rs
git commit -m "$(cat <<'EOF'
refactor(app): 7 个面板操作 icon 按钮迁移到 icon_button_entry

browser(收藏星标/书签开关)、database(schema 返回)、files(目录搜索/
隐藏文件切换/分支选择器)、usage(刷新)共 7 处,原先各自手写 MouseArea+
on_enter/on_exit 三件套,现在统一走 icon_button_entry 一次调用。收藏
星标按钮的"无 URL 时不可点"通过新的 interactive 参数保留,不是行为
回归。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: `panel_tab` 迁移到 `tabs::tab_core`

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes:`tabs::tab_core`(已存在,签名见 `crates/dozer-app/src/tabs.rs`)。

- [ ] **Step 1: 定位 `panel_tab`**

```bash
command grep -n "^pub(crate) fn panel_tab" crates/dozer-app/src/app.rs
```

预期在 `app.rs:5870`。

- [ ] **Step 2: 替换 `select`/`close_btn`/`close` 构造**

找到从这段注释开始:

```rust
    // 选中改走 `MouseArea::on_press`——iced 的 `Button::on_press` 实际在
```

到 `let close = MouseArea::new(close_btn)...on_exit(close_hover(false));`
结束的整段(`app.rs:5914-5964` 附近,即 `select`/`close_base`/
`close_color`/`close_btn`/`close` 这几个 `let` 绑定),替换成:

```rust
    // 选中/关闭的接线逻辑收在 `tabs::tab_core`(2026-08-12 抽取,试点已
    // 验证过——项目页签早已用它;这里是把面板 tab 自己那份原版实现换
    // 成同一个共享内核)。四个现有参数 title_hover/close_hover/on_select/
    // on_close 与 tab_core 的 on_select_hover/on_close_hover/on_select/
    // on_close 逐个对应,直接透传。
    let close_base = theme::color::mix(theme::color::DIM, theme::color::GOLD, close_hover_t);
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
```

（`title_row` 原本直接传给 `MouseArea::new(title_row)`,现在作为
`tab_core` 的 `content` 参数,需要 `.into()` 转成 `Element`——`title_row`
是 `row![...]` 构建器,`Row` 实现了 `Into<Element>`,这一步是必需的类型
转换,不是可选项。）

- [ ] **Step 3: 编译 + 格式检查**

Run: `cargo build -p dozer-app --bin dozer && cargo fmt -p dozer-app -- --check`
Expected: 编译成功、无新增 warning、`fmt --check` 无差异。若报
`mouse`(`mouse::Interaction::Pointer`)或 `button`/`text` 相关的
`unused import`,检查 `panel_tab` 所在文件(`app.rs`)是否还有其它地方
用到这些——`app.rs` 体量很大,几乎必然还有其它用法,预期不会真的产生
未使用的 import,但仍需跑一遍确认。

- [ ] **Step 4: `cargo test`**

Run: `cargo test -p dozer-app --bin dozer`
Expected: 483 passed / 3 failed(三个已知的既有失败)。

- [ ] **Step 5: 人工验证**

按 Global Constraints 的临时二进制方法起一个 `dozer-test-paneltab`
实例,分别在终端面板、预览面板、浏览器面板(三个都调用 `panel_tab`)
逐一确认:
- mousedown 即选中(不依赖 mouseup)。
- 拖拽换位不触发原生窗口拖动(与之前 rail/项目页签验证过的现象一致)。
- 关闭按钮仅悬停时可点、可见。
- 悬停胶囊背景、激活态实底背景与迁移前逐像素比对无差异。

这组 tab 在 brainstorming 阶段用户已确认"panel 处 tab 已经完美,不用
修改",这里的验证目标是确认"迁移后依然完美",不是重新设计或改进。

- [ ] **Step 6: 提交**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
refactor(app): panel_tab 迁移到 tabs::tab_core

终端/预览/浏览器三组面板 tab 共用的 panel_tab,原先自己实现一份
select/close 接线逻辑(与项目页签迁移前的 project_tab_item 是同一套
重复),现在换成共享的 tabs::tab_core。四个现有参数直接透传,行为不变。
至此 app.rs 内两处 tab 渲染函数(project_tab_item/panel_tab)统一用
同一个交互内核。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## 完工检查

三个任务都完成后:

- [ ] `cargo build -p dozer-app --bin dozer` 全量编译无 warning。
- [ ] `cargo test -p dozer-app --bin dozer` 仍是 483 passed / 3 failed(与开工前一致)。
- [ ] `cargo fmt -p dozer-app -- --check` 无差异。
- [ ] `cargo clippy -p dozer-app --all-targets` 没有新增 lint。
- [ ] `command grep -rn "icons::icon_button(" crates/dozer-app/src/` 应该只剩
      `icons.rs` 里 `icon_button_entry` 自己内部调用的那一份逻辑相关代码
      (`icon_button` 函数定义本身),不应该再有任何外部调用点——如果还有,
      说明有遗漏的迁移目标。
- [ ] 提请审阅,通过后合并回 `main`。
