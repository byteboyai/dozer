# Todo 面板 UI 重构 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Todo 面板的 List/Kanban 两视图重构成共用同一套边框卡片视觉，看板从
3 静态列占位符改造成真实可用的自适应换行网格，统一状态 pill、新增派发入口，
底部快速新建栏对齐 Project 面板的 footer-bar 规范。

**Architecture:** 全部改动集中在 `crates/dozer-app/src/extensions/todo.rs`(单
文件，跟现有惯例一致，不拆子模块)。新增一个共用渲染函数 `todo_card()` 取代
`todo_row()`；`todo_kanban_placeholder()` 换成真正的 `todo_kanban_view()`（用
`iced_aw::widget::Wrap` 排版）；`Message` 新增 `SetDone`/`StatePillOpen`/
`StatePillClose` 三个变体，`update()` 里把 `Toggle` 的写盘逻辑抽成共用私有函数
`set_done()`，`Toggle`/`SetDone` 都调它。

**Tech Stack:** Rust、iced 0.14（`iced_widget`/`iced_renderer`）、新增
`iced_aw`（仅 `wrap` feature）。

**Spec:** `docs/superpowers/specs/2026-08-13-todo-panel-ui-refactor-design.md`

## Global Constraints

- **分支要求**：必须在独立 worktree/分支(建议分支名
  `feature/todo-panel-ui-refactor`)完成，不直接在 `main` 上改；全部任务做完、
  测试通过后走代码审阅，审阅通过再合并回 `main`。开工前先用
  `superpowers:using-git-worktrees` 技能建好 worktree，下面所有任务默认在
  该 worktree 里执行。
- ByteBoy2077 配色沿用 `crates/dozer-app/src/theme/color.rs` 现有 token(`BG`/
  `CARD`/`BORDER`/`CREAM`/`DIM`/`GOLD`/`GREEN`)，不新增色值。
- GUI 只用 iced 0.14 生态；本次新增的唯一依赖是 `iced_aw`(`features =
  ["wrap"]`, `default-features = false`)，版本锁定 `0.13`，与 `Cargo.lock`
  里已经间接解析的 `iced_aw 0.13.1` 保持一致，不会触发第二份 `iced_widget`
  解析。
- `icons::icon_button_entry`/`tabs::tab_core` 评估结论(侧栏分类按钮/视图切换
  tab 形状与两者都不匹配，继续手写)已经在 spec"共享组件评估"一节写明理由，
  本计划不重新讨论、不迁移。
- `TodoState::InProgress` 永远是从"`done` + 存活派发 session"推导出来的
  只读展示值，不新增任何能直接把状态设成 `InProgress` 的消息/字段。
- 项目现有图标资源里没有 calendar/clock 专用图标(`grep` 确认过)，日期徽章
  复用已存在的 `icons::IconKind::History`(Lucide history，时钟+回溯箭头，
  语义上足够贴近"时间信息")，不新增 svg 资源；派发按钮复用已存在的
  `icons::IconKind::BotMessageSquare`。这两个图标选型已经定死，任务里不用
  重新决策。

---

### Task 1: 新增 `iced_aw` 依赖(`wrap` feature)

**Files:**
- Modify: `crates/dozer-app/Cargo.toml:19`（在 `iced_widget` 依赖行后插入）

**Interfaces:**
- Produces: `iced_aw::widget::Wrap`(后续 Task 6 用)、
  `iced_aw::widget::wrap::direction::Horizontal`。

- [ ] **Step 1: 加依赖**

在 `crates/dozer-app/Cargo.toml` 第 19 行(`iced_widget = { version = "0.14",
features = ["wgpu", "canvas", "svg"] }`)之后插入：

```toml
# 看板视图卡片自适应换行网格(类 CSS flex-wrap)用；iced_widget 本身不带
# 这个排版能力，iced_aw 已经通过 vendored 的 iced-code-editor 间接解析在
# Cargo.lock 里(版本 0.13.1，与本项目 iced_widget 0.14 线兼容)，只开
# `wrap` feature，避免拉起 iced_aw 其它一堆用不到的部件(color-picker 等)。
iced_aw = { version = "0.13", default-features = false, features = ["wrap"] }
```

- [ ] **Step 2: 验证能编译**

Run: `cargo build -p dozer-app 2>&1 | tail -30`
Expected: 编译成功(0 error)。这一步没有传统意义的"测试"——依赖新增本身
只能靠编译验证；`Cargo.lock` 应该只更新 `dozer-app` 那部分依赖树，不应该
新增第二份 `iced_widget`/`iced_core` 解析(可选核实：`grep -c '^name =
"iced_widget"$' Cargo.lock` 改动前后都应该是 `1`)。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
build(dozer-app): add iced_aw (wrap feature) for kanban card grid

EOF
)"
```

---

### Task 2: `Message::SetDone` — 显式设置完成态，不是翻转

**背景(给实现者)：** 现有 `Message::Toggle(idx)` 是"翻转" `done`。看板/列表
卡片的状态 pill 菜单需要"待办"/"已完成"两个**显式目标值**的选项——如果
菜单直接发 `Toggle`，当任务当前是 `InProgress`(`done == false`，但有一条
存活的派发记录)时，用户点"待办"选项本意是"确认/保持待办"，如果触发
`Toggle`，会把 `done` 从 `false` **翻转成 `true`**，被错误地标记成"已完成"
——这是真实的正确性 bug。所以要新增一个接受目标值的消息。

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`(`Message` 枚举
  L384-399；`update()` 函数 L415-515)

**Interfaces:**
- Produces: `Message::SetDone(usize, bool)`(参数:任务下标、目标 `done` 值)；
  私有函数 `fn set_done(ws_state: &mut WorkspaceState, app_state: &mut
  AppState, idx: usize, project_id: i64, project_path: &Path, target_done:
  bool)`。
- Consumes: 现有 `replace_todo_line`/`reload_from_disk`/
  `AppState::set_completed_at`(均已存在，签名不变)。

- [ ] **Step 1: 写三个失败的测试**

在 `crates/dozer-app/src/extensions/todo.rs` 的 `#[cfg(test)] mod tests`
里，紧跟在现有 `update_toggle_missing_original_line_reloads_without_setting_completed_at`
测试之后插入：

```rust
    #[test]
    fn update_set_done_pending_to_done_writes_and_stamps_completed_at() {
        let (_dir, root) = project_dir_with_todo("- [ ] 任务A\n");
        let mut ws_state = ws_with_item("任务A", false);
        let mut app_state = AppState::default();
        update(&mut ws_state, &mut app_state, Message::SetDone(0, true), 1, &root);
        assert!(ws_state.items[0].done);
        let key = todo_line_key("任务A");
        assert!(app_state.meta_for(1, key).unwrap().completed_at.is_some());
        let content = std::fs::read_to_string(todo_path(&root)).unwrap();
        assert!(content.contains("- [x] 任务A"));
    }

    #[test]
    fn update_set_done_noop_when_already_target_value() {
        // 模拟"进行中"态点"待办"：done 已经是 false，SetDone(idx, false)
        // 必须整个是 no-op(不读写文件、不碰 completed_at)，否则会把还在
        // 执行的任务误标记。
        let (_dir, root) = project_dir_with_todo("- [ ] 任务A\n");
        let mut ws_state = ws_with_item("任务A", false);
        let mut app_state = AppState::default();
        update(&mut ws_state, &mut app_state, Message::SetDone(0, false), 1, &root);
        assert!(!ws_state.items[0].done);
        let key = todo_line_key("任务A");
        assert!(
            app_state.meta_for(1, key).is_none(),
            "no-op 不该写 completed_at"
        );
        let content = std::fs::read_to_string(todo_path(&root)).unwrap();
        assert_eq!(content, "- [ ] 任务A\n", "no-op 不该改动磁盘文件");
    }

    #[test]
    fn update_set_done_done_to_pending_clears_completed_at() {
        let (_dir, root) = project_dir_with_todo("- [x] 任务A\n");
        let mut ws_state = ws_with_item("任务A", true);
        let mut app_state = AppState::default();
        app_state.set_completed_at(1, "任务A", true);
        update(&mut ws_state, &mut app_state, Message::SetDone(0, false), 1, &root);
        assert!(!ws_state.items[0].done);
        let key = todo_line_key("任务A");
        assert!(app_state.meta_for(1, key).unwrap().completed_at.is_none());
        let content = std::fs::read_to_string(todo_path(&root)).unwrap();
        assert!(content.contains("- [ ] 任务A"));
    }
```

- [ ] **Step 2: 跑测试确认失败(编译错误也算)**

Run: `cargo test -p dozer-app --lib extensions::todo:: -- --test-threads=1 2>&1 | tail -40`
Expected: 编译失败，报 `Message::SetDone` 不存在这个变体(因为还没定义)。

- [ ] **Step 3: 加 `Message::SetDone` 变体**

在 `crates/dozer-app/src/extensions/todo.rs` 的 `Message` 枚举(L384-399)里，
`PlanDateSubmit,` 那一行之后加：

```rust
    /// pill 菜单选中"待办"/"已完成"时发出，`bool` 是**目标** `done` 值
    /// (显式设置，不是翻转)。当前 `done` 已经等于目标值时视为 no-op，
    /// 不重复写盘——见 `set_done()` 的实现注释。
    SetDone(usize, bool),
```

- [ ] **Step 4: 把 `Toggle` 的写盘逻辑抽成 `set_done()`，`Toggle`/`SetDone`
      都调它**

把 `update()` 函数(L415-515)里 `Message::Toggle(idx) => { ... }` 那一整个
分支(原 L423-456)替换成：

```rust
        Message::Toggle(idx) => {
            let Some(item) = ws_state.items.get(idx) else {
                return;
            };
            let target = !item.done;
            set_done(ws_state, app_state, idx, project_id, project_path, target);
        }
        Message::SetDone(idx, target_done) => {
            set_done(ws_state, app_state, idx, project_id, project_path, target_done);
        }
```

然后在 `update()` 函数**之前**(紧挨着 `update` 定义，比如插在
`reload_from_disk` 和 `pub fn update(` 之间)新增私有函数：

```rust
/// `Toggle`/`SetDone` 共用的写盘逻辑：把任务行的 `[ ]`/`[x]` 改成
/// `target_done` 对应的目标值(不是翻转)。`item.done == target_done` 时
/// 直接 no-op 返回，不读写文件、不碰 `completed_at`——这是"进行中"态点
/// pill 菜单"待办"选项时的关键行为：`done` 本来就是 `false`，不应该因为
/// 用户点了这个选项就产生任何副作用。
fn set_done(
    ws_state: &mut WorkspaceState,
    app_state: &mut AppState,
    idx: usize,
    project_id: i64,
    project_path: &std::path::Path,
    target_done: bool,
) {
    let Some(item) = ws_state.items.get(idx) else {
        return;
    };
    if item.done == target_done {
        return;
    }
    let old_line = format!("- [{}] {}", if item.done { "x" } else { " " }, item.text);
    let new_line = format!("- [{}] {}", if target_done { "x" } else { " " }, item.text);
    let before_text = item.text.clone();
    let path = todo_path(project_path);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return;
    };
    match replace_todo_line(&content, &old_line, &new_line) {
        Some(new_content) => {
            if let Err(e) = std::fs::write(&path, &new_content) {
                tracing::warn!("写入 todo.md 失败: {e}");
                return;
            }
            reload_from_disk(ws_state, project_path);
        }
        None => {
            // 冲突:文件已经变了,放弃这次写入,直接重读展示最新状态。
            reload_from_disk(ws_state, project_path);
            return;
        }
    }
    // 文本没变(正常场景)才更新 completed_at;如果文本变了(文件可能在
    // 重读期间被 agent 并发改过),跳过,避免把完成时间错记到另一条任务上。
    if let Some(after) = ws_state.items.get(idx)
        && after.text == before_text
    {
        app_state.set_completed_at(project_id, &after.text, after.done);
    }
}
```

同时要在 `update()` 末尾的 `DispatchToExisting(..) | DispatchNew(..) =>
unreachable!(...)` 分支**之前**，确认没有漏掉任何 `Message` 变体——`match`
现在应该覆盖 `SetDone`(新增)。`cargo build` 会在漏 match arm 时直接报错，
按报错补全即可。

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p dozer-app --lib extensions::todo:: -- --test-threads=1 2>&1 | tail -60`
Expected: 全部 PASS，包括新增的 3 个 `update_set_done_*` 测试和原有的
`update_toggle_*` 系列(`set_done` 抽取不应该改变 `Toggle` 的外部行为)。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/todo.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): add Message::SetDone with explicit target state

Extracts Toggle's write-back logic into set_done(); SetDone lets a
caller set the exact done value instead of flipping it, which the
upcoming status-pill menu needs (flipping would wrongly complete an
in-progress task when the user just confirms "still pending").

EOF
)"
```

---

### Task 3: `Message::StatePillOpen`/`StatePillClose` — pill 菜单展开态

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`(`WorkspaceState` 结构体
  L256-267；`Message` 枚举；`update()` 函数)

**Interfaces:**
- Produces: `Message::StatePillOpen(usize)`、`Message::StatePillClose`；
  `WorkspaceState.state_pill_open: Option<usize>` 字段(模块内直接访问，
  跟现有 `dispatch_open` 字段的可见性一致)。

- [ ] **Step 1: 写失败的测试**

在 `mod tests` 里紧跟在 `update_dispatch_open_and_close_toggle_popup` 之后
插入：

```rust
    #[test]
    fn update_state_pill_open_and_close_toggle_field() {
        let (_dir, root) = project_dir_with_todo("# Todo\n");
        let mut ws_state = WorkspaceState::default();
        let mut app_state = AppState::default();
        update(
            &mut ws_state,
            &mut app_state,
            Message::StatePillOpen(3),
            1,
            &root,
        );
        assert_eq!(ws_state.state_pill_open, Some(3));
        update(
            &mut ws_state,
            &mut app_state,
            Message::StatePillClose,
            1,
            &root,
        );
        assert_eq!(ws_state.state_pill_open, None);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app --lib extensions::todo::tests::update_state_pill_open_and_close_toggle_field -- --test-threads=1 2>&1 | tail -30`
Expected: 编译失败(`state_pill_open` 字段、`StatePillOpen`/`StatePillClose`
变体都还不存在)。

- [ ] **Step 3: 加字段 + 消息变体 + update 分支**

`WorkspaceState` 结构体(L256-267)里 `editing_plan_date: Option<(usize,
String)>,` 之后加一行：

```rust
    /// 状态 pill 菜单展开态(卡片下标)，`None` = 未展开。跟 `dispatch_open`
    /// 同一种"同时只能有一个"模型，不做多卡片同时展开。
    state_pill_open: Option<usize>,
```

`Message` 枚举里 `SetDone(usize, bool),` 之后加：

```rust
    /// 展开某张卡片的状态 pill 菜单(待办/已完成 二选一)。
    StatePillOpen(usize),
    /// 收起状态 pill 菜单(选中某项后，或点击外部)。
    StatePillClose,
```

`update()` 的 `match` 里，`Message::SetDone(idx, target_done) => { ... }`
之后加：

```rust
        Message::StatePillOpen(idx) => ws_state.state_pill_open = Some(idx),
        Message::StatePillClose => ws_state.state_pill_open = None,
```

再回到 Task 2 里的 `Message::SetDone` 分支，补一行让选中选项后菜单自动收起
(镜像现有 `DispatchToExisting`/`DispatchNew` 选中后由内核调
`close_dispatch_popup()` 的既有模式)：

```rust
        Message::SetDone(idx, target_done) => {
            set_done(ws_state, app_state, idx, project_id, project_path, target_done);
            ws_state.state_pill_open = None;
        }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app --lib extensions::todo:: -- --test-threads=1 2>&1 | tail -60`
Expected: 全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/todo.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): add StatePillOpen/StatePillClose for status pill menu

EOF
)"
```

---

### Task 4: `todo_card()` — List/Kanban 共用的边框卡片组件

**背景(给实现者)：** 这是本次重构的核心视觉改动。现有 `todo_row()`(L815-
1055)渲染一条扁平高亮行；这个任务把它换成一张边框卡片，结构见 spec
"卡片组件 `todo_card()`"一节。这个任务先只接入列表视图(`todo_list_view`)，
看板视图留到 Task 6。

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`(删除 `todo_row`
  L815-1055；新增 `todo_card`/`state_pill`/`state_pill_menu`；改
  `todo_list_view` L604-735)

**Interfaces:**
- Consumes: `TodoItem`/`TodoState`/`TodoTaskMeta`/`DispatchRecord`(已有，
  不变)；`todo_dispatch_popup(idx, existing_tabs)`(已有，签名不变，L1060）；
  `format_todo_month_day(SystemTime) -> String`(已有，L1295)。
- Produces:
  - `fn todo_card<'a>(number: usize, idx: usize, item: &'a TodoItem, state:
    TodoState, meta: Option<&'a TodoTaskMeta>, dispatch: Option<&'a
    DispatchRecord>, selected: bool, dispatch_open: bool, state_pill_open:
    bool, existing_tabs: &'a [(&'a str, String)]) -> Element<'static,
    Message, iced_widget::Theme, iced_renderer::Renderer>`
  - `fn state_pill(idx: usize, state: TodoState) -> Element<'static,
    Message, iced_widget::Theme, iced_renderer::Renderer>`
  - `fn state_pill_menu(idx: usize) -> Element<'static, Message,
    iced_widget::Theme, iced_renderer::Renderer>`

- [ ] **Step 1: 删除 `todo_row`**

删除 `crates/dozer-app/src/extensions/todo.rs` 里整个 `fn todo_row<'a>(...)
{ ... }` 函数(原 L812-1055，含它上面那条文档注释)。

- [ ] **Step 2: 新增 `state_pill` 和 `state_pill_menu`**

在刚删除 `todo_row` 的位置(紧邻 `todo_dispatch_popup` 之前或之后都行，建议
放在 `todo_dispatch_popup` 之后，`todo_plan_date_edit_row` 之前)插入：

```rust
/// 状态 pill：三态统一成同一种紧凑圆角形状，文字/颜色随 `state` 变。
/// `Pending`/`Done` 可点击(发 `StatePillOpen`，弹出二选一菜单)；
/// `InProgress` 是推导值，不接受直接设置，pill 只读展示，不挂 `on_press`。
fn state_pill(
    idx: usize,
    state: TodoState,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (label, border_color, text_color, bg) = match state {
        TodoState::Pending => ("待办", theme::color::BORDER, theme::color::DIM, None),
        TodoState::InProgress => (
            "进行中",
            theme::color::GREEN,
            theme::color::GREEN,
            None,
        ),
        TodoState::Done => (
            "已完成",
            theme::color::GREEN,
            theme::color::BG,
            Some(theme::color::GREEN),
        ),
    };
    let mut btn = button(text(label).size(theme::font::caption()).color(text_color))
        .padding([4, 10])
        .style(move |_t: &iced_widget::Theme, _s| button::Style {
            background: bg.map(Into::into),
            text_color,
            border: Border {
                color: border_color,
                width: 1.0,
                radius: 10.0.into(),
            },
            ..button::Style::default()
        });
    if state != TodoState::InProgress {
        btn = btn.on_press(Message::StatePillOpen(idx));
    }
    btn.into()
}

/// pill 菜单：待办/已完成 二选一，选中发 `SetDone(idx, 目标值)`。样式镜像
/// 现有 `todo_dispatch_popup`(CARD 底 + BORDER 描边)。
fn state_pill_menu(
    idx: usize,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let option = |label: &'static str, target_done: bool, color: Color| {
        button(text(label).size(theme::font::body()).color(color))
            .on_press(Message::SetDone(idx, target_done))
            .width(Length::Fill)
            .padding([6, 12])
            .style(move |_t: &iced_widget::Theme, _s| button::Style {
                background: None,
                text_color: color,
                ..button::Style::default()
            })
    };
    container(
        column![
            option("待办", false, theme::color::DIM),
            option("已完成", true, theme::color::GREEN),
        ]
        .spacing(2),
    )
    .padding(6)
    .style(|_t: &iced_widget::Theme| container::Style {
        background: Some(theme::color::CARD.into()),
        border: Border {
            color: theme::color::BORDER,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}
```

- [ ] **Step 3: 新增 `todo_card`**

紧接着 `state_pill_menu` 之后插入：

```rust
/// 统一卡片组件：List/Kanban 共用同一套边框卡片视觉，取代原来的
/// `todo_row`(扁平高亮行)。结构自上而下：编号 + 日期徽章 → checkbox +
/// 任务文字 → 派发按钮(仅待办未派发时) + 状态 pill。选中态左侧加 3px
/// 金色竖条(对齐原 `todo_row` 的 `accent` 处理)。
#[allow(clippy::too_many_arguments)]
fn todo_card<'a>(
    number: usize,
    idx: usize,
    item: &'a TodoItem,
    state: TodoState,
    meta: Option<&'a TodoTaskMeta>,
    dispatch: Option<&'a DispatchRecord>,
    selected: bool,
    dispatch_open: bool,
    state_pill_open: bool,
    existing_tabs: &'a [(&'a str, String)],
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let done = item.done;

    // ---- 顶部行：编号 + 日期徽章 ----
    let number_text = text(format!("#{number:03}"))
        .size(theme::font::caption())
        .color(theme::color::DIM);

    let date_label = match state {
        TodoState::Done => meta
            .and_then(|m| m.completed_at)
            .map(format_todo_month_day)
            .unwrap_or_else(|| "-".to_string()),
        _ => meta
            .and_then(|m| m.plan_date.clone())
            .unwrap_or_else(|| "-".to_string()),
    };
    let date_badge: Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> =
        MouseArea::new(
            row![
                icons::view(
                    icons::IconKind::History,
                    crate::theme::icon_size::row(),
                    theme::color::DIM
                ),
                text(date_label).size(theme::font::caption()).color(theme::color::DIM),
            ]
            .spacing(4)
            .align_y(iced_widget::core::alignment::Vertical::Center),
        )
        .interaction(mouse::Interaction::Pointer)
        .on_press(Message::PlanDateEditStart(idx))
        .into();

    let top_row = row![
        number_text,
        iced_widget::space::Space::new()
            .width(Length::Fill)
            .height(Length::Shrink),
        date_badge,
    ]
    .align_y(iced_widget::core::alignment::Vertical::Center);

    // ---- 中部：checkbox + 任务文字（勾选/删除线处理与原 todo_row 一致）----
    let box_color = if done {
        theme::color::BORDER
    } else {
        theme::color::DIM
    };
    let checkbox = button(
        container(if done {
            text("✓")
                .size(theme::font::caption())
                .color(theme::color::DIM)
                .into()
        } else {
            Element::from(iced_widget::space::Space::new())
        })
        .width(Length::Fixed(18.0))
        .height(Length::Fixed(18.0))
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: if done {
                Some(theme::color::BORDER.into())
            } else {
                None
            },
            border: Border {
                color: box_color,
                width: 1.5,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        }),
    )
    .on_press(Message::Toggle(idx))
    .padding(0)
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: theme::color::CREAM,
        ..button::Style::default()
    });

    let label_color = if done {
        theme::color::DIM
    } else {
        theme::color::CREAM
    };
    let label: Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> = if done {
        let rich: iced_widget::text::Rich<
            '_,
            (),
            Message,
            iced_widget::Theme,
            iced_renderer::Renderer,
        > = rich_text![
            span(item.text.clone())
                .size(theme::font::body())
                .color(label_color)
                .strikethrough(true)
        ];
        rich.into()
    } else {
        text(item.text.clone())
            .size(theme::font::body())
            .color(label_color)
            .into()
    };
    let label_area: Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> =
        MouseArea::new(container(label).width(Length::Fill))
            .interaction(mouse::Interaction::Pointer)
            .on_press(Message::RowSelect(if selected { None } else { Some(idx) }))
            .into();

    let body_row = row![checkbox, label_area]
        .spacing(10)
        .align_y(iced_widget::core::alignment::Vertical::Center);

    // ---- 底部行：派发按钮(仅待办未派发) + 状态 pill ----
    let mut bottom = row![].spacing(8).align_y(iced_widget::core::alignment::Vertical::Center);
    if state == TodoState::Pending && dispatch.is_none() {
        let dispatch_btn = button(icons::view(
            icons::IconKind::BotMessageSquare,
            crate::theme::icon_size::row(),
            theme::color::GOLD,
        ))
        .on_press(Message::DispatchOpen(idx))
        .padding(6)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            border: Border {
                color: theme::color::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..button::Style::default()
        });
        bottom = bottom.push(dispatch_btn);
    }
    bottom = bottom.push(state_pill(idx, state));

    let bottom_row = row![
        iced_widget::space::Space::new()
            .width(Length::Fill)
            .height(Length::Shrink),
        bottom,
    ];

    let card_body = column![top_row, body_row, bottom_row].spacing(8);

    let accent = container(iced_widget::space::Space::new())
        .width(Length::Fixed(3.0))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: if selected {
                Some(theme::color::GOLD.into())
            } else {
                None
            },
            ..container::Style::default()
        });

    let inner = row![accent, container(card_body).padding(10).width(Length::Fill)].spacing(0);

    let card = container(inner)
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::CARD.into()),
            border: Border {
                color: theme::color::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        });

    let mut stacked = column![card];
    if dispatch_open {
        stacked = stacked.push(todo_dispatch_popup(idx, existing_tabs));
    }
    if state_pill_open {
        stacked = stacked.push(state_pill_menu(idx));
    }
    stacked.into()
}
```

- [ ] **Step 4: 改 `todo_list_view` 改调 `todo_card`**

把 `todo_list_view`(L604-735)里生成行的部分：

```rust
                None => {
                    list = list.push(todo_row(
                        display_no + 1,
                        idx,
                        item,
                        states[idx],
                        meta,
                        dispatch,
                        ws_state.selected_row == Some(idx),
                        ws_state.dispatch_open == Some(idx),
                        &existing_tabs,
                    ));
                }
```

改成：

```rust
                None => {
                    list = list.push(todo_card(
                        display_no + 1,
                        idx,
                        item,
                        states[idx],
                        meta,
                        dispatch,
                        ws_state.selected_row == Some(idx),
                        ws_state.dispatch_open == Some(idx),
                        ws_state.state_pill_open == Some(idx),
                        &existing_tabs,
                    ));
                }
```

同一个函数里，把 `let mut list = column![].spacing(2);` 改成
`let mut list = column![].spacing(8).padding([0, 20]);`(卡片之间需要更明显
的间距；原来行内 `[10, 20]` padding 现在挪到卡片自己的 `.padding(10)` 上，
所以外层列表要补回左右 20px 边距，否则卡片会贴着面板边缘)。

- [ ] **Step 5: 跑测试 + 编译确认**

Run: `cargo test -p dozer-app --lib extensions::todo:: -- --test-threads=1 2>&1 | tail -60`
Expected: 全部 PASS(这个任务不改 `update()`/纯函数逻辑，已有测试应该照常
通过；这一步主要是确认删除 `todo_row`、改 `todo_list_view` 没有引入编译
错误——渲染函数本身没有对应的单测，视觉效果留到 Task 7 人工验收)。

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | tail -60`
Expected: 无新增 warning(尤其留意闭包捕获、未使用变量)。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/todo.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): replace todo_row with shared bordered card component

todo_card() unifies the three different trailing-badge treatments
(plan-date/dispatch button, EXECUTING avatar, SUCCESS checkmark) into
one compact status pill, adds a dedicated dispatch entry point, and
moves the date display to a clickable top-right badge. Wired into the
list view only for now; kanban follows in a later task.

EOF
)"
```

---

### Task 5: `todo_footer_bar()` — 底部快速新建栏对齐 footer-bar 规范

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`(`todo_list_view`
  L604-735)

**Interfaces:**
- Produces: `fn todo_footer_bar<'a>(add_draft: &'a str) -> Element<'a,
  Message, iced_widget::Theme, iced_renderer::Renderer>`
- Consumes: 参考结构 `crates/dozer-app/src/extensions/project.rs:574-634`
  (`project_footer_bar`)，本任务照抄它的"1px BORDER 分隔线 +
  `padding([6, 8])`"结构，内容换成任务新增输入框。

- [ ] **Step 1: 新增 `todo_footer_bar`**

在 `todo_list_view` 函数**之前**插入(比如放在 `todo_card` 之后)：

```rust
/// 底部快速新建栏，结构对齐 `project.rs::project_footer_bar`(1px BORDER
/// 分隔线 + `padding([6, 8])`)。List/Kanban 两视图共用。
fn todo_footer_bar<'a>(
    add_draft: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let add_row = row![
        icons::view(
            icons::IconKind::SquarePlus,
            crate::theme::icon_size::row(),
            theme::color::GOLD
        ),
        text_input("Initiate new task protocol..", add_draft)
            .on_input(Message::AddInputChanged)
            .on_submit(Message::AddSubmit)
            .size(theme::font::body())
            .width(Length::Fill)
            .style(
                |_t: &iced_widget::Theme, _s| iced_widget::text_input::Style {
                    background: theme::color::BG.into(),
                    border: Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: 0.0.into(),
                    },
                    icon: theme::color::GOLD,
                    placeholder: theme::color::DIM,
                    value: theme::color::CREAM,
                    selection: theme::color::GOLD,
                },
            ),
    ]
    .spacing(8)
    .align_y(iced_widget::core::alignment::Vertical::Center);

    let top_line = container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::BORDER.into()),
            ..container::Style::default()
        });

    container(column![top_line, add_row].spacing(4))
        .width(Length::Fill)
        .padding([6, 8])
        .into()
}
```

- [ ] **Step 2: `todo_list_view` 改用 `todo_footer_bar`，删掉旧的
      `divider`/`add_row` 拼接**

`todo_list_view`(原 L685-734)里，把从 `let add_row = row![` 开始一直到
函数末尾 `column![ search, scrollable(list).height(Length::Fill), divider,
add_row, ] .height(Length::Fill) .into()` 这一整段，替换成：

```rust
    column![
        search,
        scrollable(list).height(Length::Fill),
        todo_footer_bar(&ws_state.add_draft),
    ]
    .height(Length::Fill)
    .into()
```

(旧的 `add_row` 变量定义、`divider` 变量定义都删掉，逻辑已经搬进
`todo_footer_bar`。)

- [ ] **Step 3: 编译确认**

Run: `cargo build -p dozer-app 2>&1 | tail -40`
Expected: 编译成功，无 unused variable/dead code 相关新增 warning。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/todo.rs
git commit -m "$(cat <<'EOF'
refactor(dozer-app): restyle todo quick-add bar as shared footer-bar

Mirrors project.rs::project_footer_bar's structure (1px BORDER divider
+ padding([6, 8])) so the visual language matches the Project pane;
extracted into todo_footer_bar() so kanban can reuse it in the next
task.

EOF
)"
```

---

### Task 6: `todo_kanban_view()` — 真正可用的自适应换行看板

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`(删除
  `todo_kanban_placeholder` L737-792；新增 `todo_search_bar`/
  `todo_kanban_view`；改 `view()` L578-583 的分发)

**Interfaces:**
- Consumes: `todo_card`(Task 4)、`todo_footer_bar`(Task 5)、
  `filter_todos`(已有，不变)、`iced_aw::widget::Wrap`(Task 1 新依赖)。
- Produces: `fn todo_search_bar<'a>(search: &'a str) -> Element<'a, Message,
  iced_widget::Theme, iced_renderer::Renderer>`(从 `todo_list_view` 里抽出，
  List/Kanban 共用同一份搜索框逻辑，对应 spec"共享同一份 search/filter 状态"
  要求)；`fn todo_kanban_view<'a>(app_state: &'a AppState, ws_state: &'a
  WorkspaceState, project_id: i64, states: &[TodoState], tabs: &[
  SessionTabSummary]) -> Element<'a, Message, iced_widget::Theme,
  iced_renderer::Renderer>`。

- [ ] **Step 1: 删除 `todo_kanban_placeholder`**

删除整个 `fn todo_kanban_placeholder<'a>(...) { ... }` 函数(原 L737-792，
含它上面的文档注释)。

- [ ] **Step 2: 从 `todo_list_view` 抽出 `todo_search_bar`**

`todo_list_view`(现在因为 Task 4/5 已经改动过，用文本定位而不是行号)里，
找到 `let search = row![` 开始到 `.align_y(iced_widget::core::alignment::
Vertical::Center);` 结束的那一整段(原始 L619-642)，删掉，换成：

```rust
    let search = todo_search_bar(&ws_state.search);
```

然后在 `todo_list_view` 函数**之前**(比如紧邻 `todo_footer_bar` 之后)新增：

```rust
/// 顶部搜索框，List/Kanban 两视图共用同一份 `ws_state.search` 状态——切
/// tab 不清空搜索词(对应 spec"List/Kanban 共用同一份搜索状态"要求)。
fn todo_search_bar<'a>(
    search: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    row![
        icons::view(
            icons::IconKind::Search,
            crate::theme::icon_size::row(),
            theme::color::DIM
        ),
        text_input("Search List parameters...", search)
            .on_input(Message::SearchChanged)
            .size(theme::font::body())
            .width(Length::Fill)
            .style(
                |_t: &iced_widget::Theme, _s| iced_widget::text_input::Style {
                    background: theme::color::BG.into(),
                    border: Border::default(),
                    icon: theme::color::DIM,
                    placeholder: theme::color::DIM,
                    value: theme::color::CREAM,
                    selection: theme::color::GOLD,
                }
            ),
    ]
    .spacing(8)
    .padding([12, 20])
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .into()
}
```

- [ ] **Step 3: 新增 `todo_kanban_view`**

在 `todo_list_view` 函数**之后**插入：

```rust
/// 看板视图主体：与 `todo_list_view` 共用同一份 `search`/`filter` 状态和
/// `todo_footer_bar`，唯一区别是卡片排布方式——单列自适应换行网格(类 CSS
/// flex-wrap，用 `iced_aw::widget::Wrap` 实现)而不是纵向单列堆叠。不做
/// 跨列拖拽(非目标，见 spec)。
fn todo_kanban_view<'a>(
    app_state: &'a AppState,
    ws_state: &'a WorkspaceState,
    project_id: i64,
    states: &[TodoState],
    tabs: &[SessionTabSummary],
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let visible_idx = filter_todos(&ws_state.items, states, ws_state.filter, &ws_state.search);

    let existing_tabs: Vec<(&str, String)> = tabs
        .iter()
        .filter(|t| t.alive)
        .map(|t| (t.session_id.as_str(), t.title.clone()))
        .collect();

    let body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if visible_idx.is_empty() {
            container(
                text("没有匹配的任务")
                    .size(theme::font::body())
                    .color(theme::color::DIM),
            )
            .padding([20, 20])
            .into()
        } else {
            let mut cards = Vec::with_capacity(visible_idx.len());
            for (display_no, &idx) in visible_idx.iter().enumerate() {
                let item = &ws_state.items[idx];
                let key = todo_line_key(&item.text);
                let meta = app_state.meta_for(project_id, key);
                let dispatch = meta.and_then(|m| m.dispatch.as_ref());
                let card = container(todo_card(
                    display_no + 1,
                    idx,
                    item,
                    states[idx],
                    meta,
                    dispatch,
                    ws_state.selected_row == Some(idx),
                    ws_state.dispatch_open == Some(idx),
                    ws_state.state_pill_open == Some(idx),
                    &existing_tabs,
                ))
                .width(Length::Fixed(280.0));
                cards.push(card.into());
            }
            iced_aw::widget::Wrap::with_elements(cards)
                .spacing(12.0)
                .line_spacing(12.0)
                .padding([12, 20])
                .into()
        };

    column![
        todo_search_bar(&ws_state.search),
        scrollable(body).height(Length::Fill),
        todo_footer_bar(&ws_state.add_draft),
    ]
    .height(Length::Fill)
    .into()
}
```

- [ ] **Step 4: `view()` 分发改调 `todo_kanban_view`**

`view()` 函数里(原 L578-583)：

```rust
    let body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match ws_state.view_mode {
            TodoViewMode::List => todo_list_view(app_state, ws_state, project_id, &states, tabs),
            TodoViewMode::Kanban => todo_kanban_placeholder(&states, ws_state),
            TodoViewMode::Markdown => todo_markdown_view(project_path),
        };
```

改成：

```rust
    let body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match ws_state.view_mode {
            TodoViewMode::List => todo_list_view(app_state, ws_state, project_id, &states, tabs),
            TodoViewMode::Kanban => todo_kanban_view(app_state, ws_state, project_id, &states, tabs),
            TodoViewMode::Markdown => todo_markdown_view(project_path),
        };
```

- [ ] **Step 5: 编译 + 测试确认**

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: 编译成功。重点检查 `iced_aw::widget::Wrap::with_elements(cards)`
这一行——如果类型推导失败(报 `Theme`/`Renderer` 不匹配)，把它改成显式
turbofish：
`iced_aw::widget::Wrap::<Message, iced_aw::widget::wrap::direction::Horizontal, iced_widget::Theme, iced_renderer::Renderer>::with_elements(cards)`。

Run: `cargo test -p dozer-app --lib extensions::todo:: -- --test-threads=1 2>&1 | tail -60`
Expected: 全部 PASS。

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | tail -60`
Expected: 无新增 warning。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/todo.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): implement real adaptive-wrap kanban view

Replaces the static three-column placeholder with a working kanban:
cards wrap by panel width via iced_aw::widget::Wrap, sharing search,
filter, and footer-bar state with the list view (switching tabs no
longer resets what you were looking at).

EOF
)"
```

---

### Task 7: 收尾清理 + 全量验证 + 人工 GUI 验收清单

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`(死代码清理，如果
  Step 1 发现有)

**Interfaces:** 无新增接口，这个任务是收口。

- [ ] **Step 1: 全仓库死代码/未使用符号检查**

Run: `cargo build -p dozer-app 2>&1 | grep -i "warning: unused\|warning: never" `
Expected: 空输出。如果不是空的(比如 `todo_line_key`/`format_todo_time` 等
以前被 `todo_row` 用到、现在没被 `todo_card` 用到的辅助函数)，逐个确认是否
`todo_card` 里其实换了名字调用（多半是漏改了调用点，回 Task 4 检查），
不是"顺手删掉"——除非确认这个函数除了 `todo_row` 没有其它调用点，才可以
删。

- [ ] **Step 2: 格式化 + 全量 lint**

Run: `cargo fmt -p dozer-app`
Run: `cargo clippy -p dozer-app --all-targets -- -D warnings 2>&1 | tail -80`
Expected: 无 error。

- [ ] **Step 3: 全量测试**

Run: `cargo test -p dozer-app 2>&1 | tail -80`
Expected: 全部 PASS，无回归(尤其关注 `extensions::todo::` 和其它模块——
`todo.rs` 改动不应该影响别的 extension)。

- [ ] **Step 4: Commit(如果 Step 1/2 有清理动作)**

```bash
git add crates/dozer-app/src/extensions/todo.rs
git commit -m "$(cat <<'EOF'
chore(dozer-app): clean up dead code after todo panel refactor

EOF
)"
```

（如果 Step 1/2 什么都没改，跳过这一步，不要造一个空 commit。）

- [ ] **Step 5: 人工 GUI 验收清单**

Run: `cargo run -p dozer-app`，打开任意一个有 `.dozer/todo.md` 的项目(没有
就先在项目根目录手动建一个，写几条 `- [ ] xxx`/`- [x] xxx`)，Todo 面板逐项
核对：

- [ ] 列表视图：任务显示成边框卡片(不再是扁平高亮行)，卡片间有可见间距。
- [ ] 看板视图：卡片按面板宽度自适应换行；缩窄窗口宽度，卡片从两列变一列。
- [ ] 待办任务卡片右下角显示"待办" pill + 一个派发图标按钮；点派发按钮
      弹出现有的 agent 选择弹窗，选一个目标不报错。
- [ ] 点"待办"/"已完成" pill，弹出二选一菜单；选"已完成"后卡片变成删除线
      文字 + 绿色"已完成" pill；再打开菜单选"待办"，能改回来。
- [ ] 手动把某条任务派发给一个存活的 agent 会话后(或者等一条已有任务进入
      "进行中")，对应卡片显示"进行中" pill 且**不可点击**(不弹菜单)，且
      卡片上不再显示派发按钮。
- [ ] 点卡片顶部右侧的日期徽标，进入现有的计划日期编辑输入框，输入后回车
      能保存，下次显示这个日期。
- [ ] 侧栏"待办/进行中/已完成"分类筛选在 列表/看板 之间切换 tab 时保留
      (不会切个 tab 就重置成"全部任务")；搜索框同理，输入关键字后切
      List↔Kanban，关键字还在、结果还是过滤后的。
- [ ] 底部快速新建栏视觉与 Project 面板的修复/删除按钮那条 footer-bar
      一致(1px 分隔线 + 类似留白)，回车提交能新增任务。
- [ ] MARKDOWN 视图外观未受影响(本次重构非目标)。

全部勾完，本次重构才算完成；有任何一项跟预期不符，回对应 Task 定位问题，
不要跳过。

## Self-Review Notes(写计划时已核对，供审阅者复核)

- Spec 覆盖：spec 目标 1-9 分别对应 Task 4(卡片组件)、Task 6(看板换行网格)、
  Task 4(状态 pill 统一)、Task 2+4(pill 语义/派发按钮)、Task 4(日期徽章)、
  Task 5(footer-bar)、Task 6(搜索共享)、spec 本身已写明评估结论(无需
  额外任务)。全部覆盖，无缺口。
- 类型一致性：`todo_card`/`state_pill`/`state_pill_menu`/`todo_footer_bar`/
  `todo_search_bar`/`todo_kanban_view` 的签名在各任务间保持一致(下一个
  任务引用上一个任务定义的函数名/参数顺序时逐一核对过)。
- `existing_tabs` 生命周期：`todo_card<'a>` 里 `existing_tabs: &'a [(&'a
  str, String)]` 与 `item`/`meta`/`dispatch` 共用同一个 `'a`，看起来像是
  要求"局部变量 `existing_tabs` 活得跟 `ws_state`/`app_state` 一样久"，
  实际上因为 Rust 在每次函数调用时会为泛型 `'a` 重新推导出满足当次调用的
  最短生命周期(协变),函数体内局部构造的 `existing_tabs`(生命周期短)
  和来自 `ws_state`/`app_state` 的长生命周期引用可以在同一次调用里统一
  取交集，这跟 Task 4 之前 `todo_row` 本来就是这么写、也确实能编译通过的
  写法完全一致，不是新引入的风险点。
