# kooky 对比评价可落实项收尾 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `docs/my/kooky-dozer对比评价.txt` 里判定"可落实"的两个真实场景做完:①给
`byteui::form::input_text` 补 `secure` 参数并让 `ssh.rs`/`database.rs` 两个连接表单
(共 13 处手写 `text_input`)改用它,顺带修掉两个表单彼此风格不一致的问题;②
`todo.rs` 底部提交按钮换成统一的 `icon_button_entry`。

**Architecture:** 纯组件参数扩展 + 调用点替换,不新增任何新 widget 或抽象层。
`form::input_text::view()` 加一个 `secure: bool` 位置参数(当前唯一缺的能力,两个
表单的密码字段都要),并把 `.size(theme::font::body())` 直接内置进函数体(两个
调用点都要这个值,不当可选参数处理)。`ssh.rs`/`database.rs` 各自的手写
`text_input` + 本地样式闭包,原样替换成 `byteui::form::input_text::view(...)` 调用。
`todo.rs` 提交按钮的手写 `MouseArea::new(button(icons::view(...)))` 替换成
`icons::icon_button_entry(...)`,`button_size` 取同文件已有 `search_box`/浏览器面板
惯用的 `byteui::theme::geometry::tab_button_size()`(同量级的内嵌小图标按钮)。

**Tech Stack:** Rust workspace(iced 0.14 生态);不新增依赖。

**Spec:** 本计划无独立 spec 文档——这是 brainstorming 判定的 bounded 变更(设计
已在对话内确认),触发源是 `docs/my/kooky-dozer对比评价.txt` 里"byteui form/data
模块零调用"与"CLAUDE.md icon 按钮统一组件"两条发现。设计决定记录在本文件
Global Constraints 与各任务描述里。

## Global Constraints

- **在独立 worktree/分支上开发,不要碰当前签出的 `feature/conversation-ingestion`
  分支所在的工作目录。** 那个目录当前有另一个 agent 会话在实时提交(执行到
  task 13/96),开工前确认:`git -C /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
  worktree list` 里没有已存在的同名 worktree,然后从 `main`(commit `4e60dfa`)
  拉新 worktree:`git worktree add ../dozer-kooky-review-followups -b
  feature/kooky-review-followups main`。后续所有命令都在这个新 worktree 目录里跑。
- **这个计划基于 `main` commit `4e60dfa` 分析,所有行号以此为准。** 开工前用本
  文档给出的 `grep -n` 模式核对实际行号,如有偏差以代码现状为准,不要假设行号
  绝对正确。
- **这次改动会带来两处真实可见的视觉变化,不是"零行为变更"重构:**
  ①`ssh.rs` 表单聚焦文本框时边框会从"恒定灰"变成"金色高亮"(现在的本地样式
  没做聚焦态,`byteui::form::input_text` 有);②`database.rs` 表单从 iced 默认主题
  变成卡片底色+描边的统一风格(现在完全没设 `.style()`)。这两处变化是本计划的
  目的,不是意外副作用,人工走查时按"变得和 SSH 面板一致"去核对,不要当 bug 改回去。
- 每个任务结束都要求对应 crate `cargo build` 干净通过;全部任务完成后跑一次
  `cargo build --workspace && cargo test -p byteui && cargo test -p dozer-app --bin
  dozer && cargo clippy --all-targets && cargo fmt --check`,记录实际 pass/fail
  数字(工作区其他既有失败不算本计划引入,若数字有出入先去 `main` 上单独确认)。
- **人工验证时绝不能碰用户正在跑的正式 app。** 用独立命名的临时二进制
  (如 `cp target/debug/dozer /tmp/dozer-test-kooky-followups`),只用它的 PID/
  进程名定位窗口,不要用 `tell process "dozer"`(会撞上正式 app)。验证完 `kill`
  并删除临时二进制。

---

### Task 1: `byteui::form::input_text` 加 `secure` 参数

**Files:**
- Modify: `crates/byteui/src/form/input_text.rs`
- Modify: `crates/byteui/src/form/mod.rs`(既有测试调用点)

**Interfaces:**
- Produces:新签名 `pub fn view<'a, Message: Clone + 'a>(placeholder: &str, value:
  &str, secure: bool, on_input: impl Fn(String) -> Message + 'a) -> Element<'a,
  Message, iced_widget::Theme, iced_renderer::Renderer>`(在 `value` 和 `on_input`
  之间插入 `secure`),供 Task 2/Task 3 调用。

- [ ] **Step 1: 确认当前签名**

```bash
command grep -n "pub fn view" crates/byteui/src/form/input_text.rs
```

预期看到 3 参数签名(`placeholder`/`value`/`on_input`),在 `input_text.rs:6`。

- [ ] **Step 2: 替换函数体,加 `secure` 参数 + 内置 `.size()`**

把 `crates/byteui/src/form/input_text.rs` 第 6-31 行整段替换为:

```rust
pub fn view<'a, Message: Clone + 'a>(
    placeholder: &str,
    value: &str,
    secure: bool,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    iced_widget::text_input(placeholder, value)
        .secure(secure)
        .on_input(on_input)
        .size(crate::theme::font::body())
        .padding(8)
        .style(|_theme: &iced_widget::Theme, status: Status| {
            let colors = crate::theme::color::current();
            let focused = matches!(status, Status::Focused { .. });
            text_input::Style {
                background: colors.card.into(),
                border: Border {
                    color: if focused { colors.gold } else { colors.border },
                    width: 1.0,
                    radius: 6.0.into(),
                },
                icon: colors.dim,
                placeholder: colors.dim,
                value: colors.cream,
                selection: crate::theme::color::mix(colors.gold, colors.card, 0.6),
            }
        })
        .into()
}
```

- [ ] **Step 3: 更新 `form/mod.rs` 里的既有测试调用点**

```bash
command grep -n "input_text::view" crates/byteui/src/form/mod.rs
```

预期在 `all_form_components_construct_without_panic` 测试里看到
`input_text::view("placeholder", "value", Msg::Input)`,改成:

```rust
let _ = input_text::view("placeholder", "value", false, Msg::Input);
```

- [ ] **Step 4: 构建 + 测试**

```bash
cargo build -p byteui && cargo test -p byteui
```

预期:构建通过,`all_form_components_construct_without_panic` 通过。

- [ ] **Step 5: Commit**

```bash
git add crates/byteui/src/form/input_text.rs crates/byteui/src/form/mod.rs
git commit -m "feat(byteui): form::input_text 新增 secure 参数"
```

---

### Task 2: `ssh.rs` 连接表单改用 `byteui::form::input_text`

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs`

**Interfaces:**
- Consumes: Task 1 的 `byteui::form::input_text::view(placeholder, value, secure,
  on_input)`。

- [ ] **Step 1: 定位待替换区域**

```bash
command grep -n "input_style\|text_input(" crates/dozer-app/src/extensions/ssh.rs
```

预期看到:`input_style` 闭包定义(约 916-931 行)、7 处 `text_input(...)` 调用
(约 934-982 行,分别对应主机名称/Host/port/user name/私钥文件路径/私钥口令/
password)。

- [ ] **Step 2: 删除 `input_style` 闭包,7 处 `text_input` 全部替换**

把第 914-931 行(含"表单输入框统一底色"注释和整个 `input_style` 闭包定义)
删除,只留下 `fn host_form` 函数签名后直接进入 `let mut col = column![...]`。

把原本:

```rust
    let mut col = column![
        text_input("主机名称", &draft.name)
            .style(input_style)
            .on_input(Message::DraftNameChanged)
            .size(byteui::theme::font::body()),
        text_input("Host", &draft.host)
            .style(input_style)
            .on_input(Message::DraftHostChanged)
            .size(byteui::theme::font::body()),
        text_input("port(22)", &draft.port)
            .style(input_style)
            .on_input(Message::DraftPortChanged)
            .size(byteui::theme::font::body()),
        text_input("user name", &draft.username)
            .style(input_style)
            .on_input(Message::DraftUsernameChanged)
            .size(byteui::theme::font::body()),
```

改成:

```rust
    let mut col = column![
        byteui::form::input_text::view("主机名称", &draft.name, false, Message::DraftNameChanged),
        byteui::form::input_text::view("Host", &draft.host, false, Message::DraftHostChanged),
        byteui::form::input_text::view("port(22)", &draft.port, false, Message::DraftPortChanged),
        byteui::form::input_text::view(
            "user name",
            &draft.username,
            false,
            Message::DraftUsernameChanged,
        ),
```

(紧接着的 `radio_dot` 那个 `row![...]` 元素保持不变,还是 `column!` 宏的第
五个元素。)

把原本:

```rust
    if draft.use_private_key {
        col = col.push(
            text_input("私钥文件路径,如 ~/.ssh/id_ed25519", &draft.key_path)
                .style(input_style)
                .on_input(Message::DraftKeyPathChanged)
                .size(byteui::theme::font::body()),
        );
        col = col.push(
            text_input("私钥口令(留空则不修改/无口令)", &draft.password)
                .secure(true)
                .style(input_style)
                .on_input(Message::DraftPasswordChanged)
                .size(byteui::theme::font::body()),
        );
    } else {
        col = col.push(
            text_input("password(留空则不修改)", &draft.password)
                .secure(true)
                .style(input_style)
                .on_input(Message::DraftPasswordChanged)
                .size(byteui::theme::font::body()),
        );
    }
```

改成:

```rust
    if draft.use_private_key {
        col = col.push(byteui::form::input_text::view(
            "私钥文件路径,如 ~/.ssh/id_ed25519",
            &draft.key_path,
            false,
            Message::DraftKeyPathChanged,
        ));
        col = col.push(byteui::form::input_text::view(
            "私钥口令(留空则不修改/无口令)",
            &draft.password,
            true,
            Message::DraftPasswordChanged,
        ));
    } else {
        col = col.push(byteui::form::input_text::view(
            "password(留空则不修改)",
            &draft.password,
            true,
            Message::DraftPasswordChanged,
        ));
    }
```

- [ ] **Step 3: 收窄 `text_input` import**

```bash
command grep -n "text_input" crates/dozer-app/src/extensions/ssh.rs
```

替换完 Step 2 后这里应该 0 命中(`text_input` 不再被直接调用)。找到文件顶部
`use iced_widget::{ MouseArea, Scrollable, button, column, container, row,
scrollable, stack, text, text_input, ... };` 这一组 import,删掉列表里的
`text_input,`。

- [ ] **Step 4: 构建**

```bash
cargo build -p dozer-app --bin dozer
```

预期:构建通过,没有 unused import 警告。

- [ ] **Step 5: 测试**

```bash
cargo test -p dozer-app --bin dozer ssh
```

预期:`ssh` 模块相关测试全部通过(纯样式替换,不改消息/状态逻辑,理应无需改
任何测试用例)。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/ssh.rs
git commit -m "refactor(dozer-app): ssh 连接表单改用 byteui::form::input_text"
```

---

### Task 3: `database.rs` 连接表单改用 `byteui::form::input_text`

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`

**Interfaces:**
- Consumes: Task 1 的 `byteui::form::input_text::view(placeholder, value, secure,
  on_input)`。

- [ ] **Step 1: 定位待替换区域**

```bash
command grep -n "text_input(" crates/dozer-app/src/extensions/database.rs
```

预期看到 6 处 `text_input(...)` 调用(约 1336-1380 行,分别对应名字/文件路径/
连接 URI/host/port/database/username/password——SQLite 分支 2 个字段,非 SQLite
分支 6 个字段,两分支共享"名字"这一个)。

- [ ] **Step 2: 6 处全部替换**

把原本:

```rust
    col = col.push(
        text_input("名字", &draft.name)
            .on_input(Message::DraftNameChanged)
            .size(byteui::theme::font::body()),
    );
    if draft.driver == DriverKind::Sqlite {
        col = col.push(
            text_input("文件路径", &draft.database)
                .on_input(Message::DraftDatabaseChanged)
                .size(byteui::theme::font::body()),
        );
    } else {
        col = col.push(
            text_input(
                "连接 URI(可选,填了则忽略下面各项,例如 postgres://user:pw@host:5432/db)",
                &draft.uri,
            )
            .on_input(Message::DraftUriChanged)
            .size(byteui::theme::font::body()),
        );
        col = col.push(
            text_input("host", &draft.host)
                .on_input(Message::DraftHostChanged)
                .size(byteui::theme::font::body()),
        );
        col = col.push(
            text_input("port", &draft.port)
                .on_input(Message::DraftPortChanged)
                .size(byteui::theme::font::body()),
        );
        col = col.push(
            text_input("database", &draft.database)
                .on_input(Message::DraftDatabaseChanged)
                .size(byteui::theme::font::body()),
        );
        col = col.push(
            text_input("username", &draft.username)
                .on_input(Message::DraftUsernameChanged)
                .size(byteui::theme::font::body()),
        );
        col = col.push(
            text_input("password(留空则不修改)", &draft.password)
                .secure(true)
                .on_input(Message::DraftPasswordChanged)
                .size(byteui::theme::font::body()),
        );
    }
```

改成:

```rust
    col = col.push(byteui::form::input_text::view(
        "名字",
        &draft.name,
        false,
        Message::DraftNameChanged,
    ));
    if draft.driver == DriverKind::Sqlite {
        col = col.push(byteui::form::input_text::view(
            "文件路径",
            &draft.database,
            false,
            Message::DraftDatabaseChanged,
        ));
    } else {
        col = col.push(byteui::form::input_text::view(
            "连接 URI(可选,填了则忽略下面各项,例如 postgres://user:pw@host:5432/db)",
            &draft.uri,
            false,
            Message::DraftUriChanged,
        ));
        col = col.push(byteui::form::input_text::view(
            "host",
            &draft.host,
            false,
            Message::DraftHostChanged,
        ));
        col = col.push(byteui::form::input_text::view(
            "port",
            &draft.port,
            false,
            Message::DraftPortChanged,
        ));
        col = col.push(byteui::form::input_text::view(
            "database",
            &draft.database,
            false,
            Message::DraftDatabaseChanged,
        ));
        col = col.push(byteui::form::input_text::view(
            "username",
            &draft.username,
            false,
            Message::DraftUsernameChanged,
        ));
        col = col.push(byteui::form::input_text::view(
            "password(留空则不修改)",
            &draft.password,
            true,
            Message::DraftPasswordChanged,
        ));
    }
```

- [ ] **Step 3: 收窄 `text_input` import**

```bash
command grep -n "text_input" crates/dozer-app/src/extensions/database.rs
```

替换完 Step 2 后这里应该 0 命中。找到文件顶部
`use iced_widget::{button, column, container, row, scrollable, text,
text_input};`,删掉 `text_input`(注意去掉多余逗号,该行会变成
`use iced_widget::{button, column, container, row, scrollable, text};`)。

- [ ] **Step 4: 构建**

```bash
cargo build -p dozer-app --bin dozer
```

预期:构建通过,没有 unused import 警告。

- [ ] **Step 5: 测试**

```bash
cargo test -p dozer-app --bin dozer database
```

预期:`database` 模块相关测试全部通过。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/database.rs
git commit -m "refactor(dozer-app): database 连接表单改用 byteui::form::input_text"
```

---

### Task 4: `todo.rs` 提交按钮改用 `icon_button_entry`

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`

**Interfaces:**
- Consumes:`byteui::interaction::icons::icon_button_entry(kind: IconKind, size:
  f32, active: bool, dim: bool, hover_t: f32, card: bool, button_size: f32,
  interactive: bool, on_select: M, on_hover: impl Fn(bool) -> M, tooltip: &str)
  -> Element<...>`(文件已 `use byteui::interaction::icons;`,已有同文件内
  `icons::icon_button_entry` 用法可参照,如 `browser.rs:1198`/`usage.rs:308`)。

- [ ] **Step 1: 定位待替换区域**

```bash
command grep -n "let submit_color\|let submit = MouseArea" crates/dozer-app/src/extensions/todo.rs
```

预期 `submit_color` 在约 1525 行,`submit` 的 `MouseArea::new(...)` 构造在约
1530-1550 行(含 `.on_press(Message::AddSubmit)`、`.on_enter(Message::Hover(
HoverId::TodoAddSubmit, true))`、`.on_exit(Message::Hover(HoverId::TodoAddSubmit,
false))`)。

- [ ] **Step 2: 替换整段**

把原本(约 1525-1550 行):

```rust
    let submit_color = byteui::theme::color::mix(
        byteui::theme::color::current().dim,
        byteui::theme::color::current().gold,
        app.hover_progress(HoverId::TodoAddSubmit),
    );
    let submit = MouseArea::new(
        button(icons::view(
            icons::IconKind::CircleArrowUp,
            byteui::theme::icon_size::row(),
            submit_color,
        ))
        .on_press(Message::AddSubmit)
        .padding(6)
        .style(move |_t, _s| button::Style {
            background: None,
            border: Border {
                color: byteui::theme::color::current().border,
                width: 0.0,
                radius: 4.0.into(),
            },
            text_color: submit_color,
            ..button::Style::default()
        }),
    )
    .on_enter(Message::Hover(HoverId::TodoAddSubmit, true))
    .on_exit(Message::Hover(HoverId::TodoAddSubmit, false));
```

改成:

```rust
    let submit = icons::icon_button_entry(
        icons::IconKind::CircleArrowUp,
        byteui::theme::icon_size::row(),
        false,
        false,
        app.hover_progress(HoverId::TodoAddSubmit),
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::AddSubmit,
        |hovered| Message::Hover(HoverId::TodoAddSubmit, hovered),
        "提交",
    );
```

`submit` 变量后续用法(嵌入 `input_box` 的 row 里)不变,`icon_button_entry`
返回的 `Element` 类型与原来 `MouseArea` 表达式一致,直接可用。

- [ ] **Step 3: 检查残留引用**

```bash
command grep -n "submit_color" crates/dozer-app/src/extensions/todo.rs
```

预期 0 命中(该变量随替换一起消失,不需要保留)。

- [ ] **Step 4: 构建**

```bash
cargo build -p dozer-app --bin dozer
```

预期:构建通过。`button`/`Border` 两个 import 在文件其他地方仍有大量使用,
不需要动 import 列表。

- [ ] **Step 5: 测试**

```bash
cargo test -p dozer-app --bin dozer todo
```

预期:`todo` 模块相关测试全部通过。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/todo.rs
git commit -m "refactor(dozer-app): todo 提交按钮改用 icon_button_entry"
```

---

### Task 5: 全量校验 + 人工 GUI 走查 + 提请审阅

**Files:** 无新增修改,本任务只跑校验和人工验证。

- [ ] **Step 1: 全量构建 + 测试 + lint**

```bash
cargo build --workspace
cargo test -p byteui
cargo test -p dozer-app --bin dozer
cargo clippy --all-targets
cargo fmt --check
```

记录实际 pass/fail 数字。若 `cargo fmt --check` 报本计划未改动文件的既有漂移,
不用管;若报本计划改过的文件有格式问题,跑 `cargo fmt` 后重新 `git add` 对应
文件、单独 commit 一次格式修正。

- [ ] **Step 2: 编译临时二进制,人工走查**

```bash
cargo build -p dozer-app --bin dozer
cp target/debug/dozer /tmp/dozer-test-kooky-followups
/tmp/dozer-test-kooky-followups &
```

走查清单:
1. 打开任意项目的 SSH 面板,新建/编辑一条主机连接:确认所有输入框(主机名称/
   Host/port/user name/私钥文件路径或 password)是卡片底色 + 灰色描边,**点进
   输入框聚焦时描边变金色**,密码字段(password/私钥口令)输入时显示圆点掩码。
2. 打开 Database 面板,新建/编辑一条连接:确认输入框风格与 SSH 面板视觉一致
   (卡片底色 + 聚焦变金),密码字段掩码正常。
3. 打开 Todo 面板,底部输入框输入任意文字,确认右下角提交图标(circle-arrow-up)
   静止是 DIM 色、鼠标悬停平滑过渡到金色、点击能正常提交任务;鼠标悬停在图标
   上应该弹出"提交"文字气泡提示(这是新增行为,之前没有 tooltip)。

```bash
kill %1
rm /tmp/dozer-test-kooky-followups
```

- [ ] **Step 3: 提请审阅**

按仓库惯例走 superpowers:requesting-code-review,确认全部通过后合并回 `main`,
用 `git worktree remove` 清理临时 worktree。
