# workspace 图标栏面板拖拽换栏 · Stage 2:图标栏渲染改遍历 + 选中消息统一 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 图标栏从"11 个手写 `icon_button_entry` 调用堆成的静态 `column!`
(`left_icon_rail`/`right_icon_rail` 两份几乎重复的函数体)"改成"按
`RailLayout.left`/`.right` 顺序遍历渲染"的一份共用函数;`Message::
LeftIconSelect`/`RightIconSelect` 两个消息 + `left_icon_select`/
`right_icon_select` 两个处理函数合并成一个 side-agnostic 的
`PanelSelect`/`panel_select`。**这一阶段依然不接入拖拽**——
`RailLayout` 的值还是 Stage 1 定的默认值,没有任何地方能改它,所以
GUI 行为依然应该和改动前逐像素一致,只是图标栏渲染代码从"手写 11 次"
变成"循环 11 次"。

**Architecture:** 不像 Stage 1 那样是一次跨全文件的原子性重命名,这次
改动集中在 `app.rs` 里三个此前互相独立的局部区域(`RailButton` 枚举 +
`left_icon_rail`/`right_icon_rail` 两个渲染函数、`panel_meta`/
`panel_badge` 两个新增的按面板取图标/徽标的辅助函数、`Message::
LeftIconSelect`/`RightIconSelect` + 对应处理函数),彼此耦合但**不**
像 Stage 1 那样牵连整个 9000+ 行文件——因此这次的 5 个 Task **每一个
结束都要求 `cargo build` 通过**,回到和 `byteui` 迁移系列一致的验证
节奏。

**Tech Stack:** Rust 2024,`byteui::interaction::icons::icon_button_entry`
(签名不变,建库时已有)。

**Spec:** `docs/superpowers/specs/2026-08-19-rail-panel-drag-relocation-design.md`

## Global Constraints

- **前提:Stage 1(数据模型统一)已合并到 main。** 开工前确认
  `grep -rn "\bLeftView\b\|\bRightView\b" crates/dozer-app/src`(排除
  `HomeLeftView`/`HomeRightView`)无输出,`PanelKind`/`RailLayout`/
  `Side` 三个类型已存在——不满足就先去完成/合并 Stage 1。
- **独立分支开发,不直接提交 main。** 在新分支(如
  `feature/rail-panel-relocation-stage2`)上完成全部 5 个 Task,提请
  审阅、通过后再合并回 `main`。每次 `git commit`/`git add` 前先跑
  `git branch --show-current` 确认当前分支;开工前 `git status` 确认
  干净,发现不相关的改动要 `git stash push -u -m "..."` 保留(注意:
  如果 stash 时同时有自己刚创建、还没提交的文件,用 `git stash push
  -- <具体路径>` 而不是 `-u`,避免连自己的文件一起冲进 stash——`-u`
  会无差别囊括所有未跟踪文件)。
- **这一阶段依然不改变任何行为。** `RailLayout` 的值在这个 Stage 结束
  时仍然只能是 `RailLayout::default()`(没有任何写入路径),`icon_rail`
  循环渲染的结果必须和手写的 11 次调用逐个参数对应,不能顺手"优化"
  参数(比如统一图标大小写法、合并重复的 `byteui::theme::icon_size::
  rail()` 调用之类)——这次是纯粹的"渲染方式从硬编码变遍历",不是
  顺手重构。
- **`Message::Acceptance` 徽标(验收面板待处理提醒的小圆点)行为不变。**
  这是 11 个面板里唯一带按钮徽标装饰的,合并进通用渲染路径时不能丢。
- **`RailButton` 的 3 个 `Home*` variant(`HomeProjectList`/
  `HomeRecents`/`HomeBrowser`)不动**——那是首页专属,`homespace.rs`
  引用,这次不碰。

---

### Task 1: `RailLayout::side_of()` 反查辅助方法

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: `RailLayout`、`PanelKind`、`Side`(Stage 1)
- Produces: `RailLayout::side_of(&self, kind: PanelKind) -> Side`——给定
  一个面板,查它当前在哪条栏。Task 4 的 `panel_select` 要用它判断"点的
  这个面板现在归哪条栏管"。

- [ ] **Step 1: 在 `RailLayout` 的 `impl` 块里追加**

```rust
    /// 给定面板,反查它当前挂在哪条栏。`RailLayout` 的不变式(见
    /// `sanitize_rail_layout`)保证 11 个面板不重不漏分布在两条栏,
    /// 所以这里的 `expect` 不会在合法状态下触发——`RailLayout` 一旦
    /// 通不过消毒就已经在 `layout::load_from` 里回落 `default()` 了,
    /// 不会带着"某个面板哪条栏都不在"的坏数据流到这里。
    pub fn side_of(&self, kind: PanelKind) -> Side {
        if self.left.contains(&kind) {
            Side::Left
        } else if self.right.contains(&kind) {
            Side::Right
        } else {
            unreachable!(
                "RailLayout 不变式被破坏:{kind:?} 不在任何一条栏——\
                 sanitize_rail_layout 应该已经挡掉这种坏数据"
            )
        }
    }
```

- [ ] **Step 2: 追加测试**

```rust
#[test]
fn side_of_finds_every_default_panel() {
    let rail = RailLayout::default();
    assert_eq!(rail.side_of(PanelKind::Files), Side::Left);
    assert_eq!(rail.side_of(PanelKind::Web), Side::Left);
    assert_eq!(rail.side_of(PanelKind::Agent), Side::Right);
    assert_eq!(rail.side_of(PanelKind::Acceptance), Side::Right);
}
```

（`Side` 需要 `PartialEq`/`Debug`——Stage 1 定义时已经带了这两个
derive,这里直接可用，不需要额外改动。）

- [ ] **Step 3: 编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer side_of`
Expected: 编译成功,新增测试通过

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): RailLayout 新增 side_of() 反查方法"
```

---

### Task 2: `panel_meta()`(图标+文案表)与 `panel_badge()`(验收徽标钩子)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: `PanelKind`(Stage 1)、`byteui::interaction::icons::IconKind`
- Produces: `fn panel_meta(kind: PanelKind) -> (icons::IconKind, &'static str)`、
  `fn panel_badge(app: &App, kind: PanelKind) -> Option<Element<'_, Message,
  iced_widget::Theme, iced_renderer::Renderer>>`。这两个函数目前还没有
  调用方(Task 3 才接进 `icon_rail`),新增代码本身应该能编译,只是会有
  "函数未使用"的 warning,这个 Task 结束时允许这条 warning 存在
  (Task 3 消费后自然消失)。

- [ ] **Step 1: 新增 `panel_meta`**

11 组图标/文案原样对应现有 `left_icon_rail`/`right_icon_rail` 里 11 次
`icon_button_entry` 调用的第 1、第 10 个参数(`IconKind` 与 tooltip 文案):

```rust
/// 面板 → (图标, 图标栏 tooltip 文案)。11 个 `PanelKind` variant 逐一
/// 对应,顺序与 `PanelKind` 定义顺序一致,不代表渲染顺序(渲染顺序看
/// `RailLayout`)。
fn panel_meta(kind: PanelKind) -> (icons::IconKind, &'static str) {
    match kind {
        PanelKind::Files => (icons::IconKind::FolderTree, "文件"),
        PanelKind::GitLog => (icons::IconKind::GitGraph, "Git 提交"),
        PanelKind::Todo => (icons::IconKind::ListTodo, "待办"),
        PanelKind::Project => (icons::IconKind::Briefcase, "项目"),
        PanelKind::Database => (icons::IconKind::Database, "数据库"),
        PanelKind::Ssh => (icons::IconKind::Server, "SSH 主机"),
        PanelKind::Web => (icons::IconKind::Globe, "浏览器"),
        PanelKind::Agent => (icons::IconKind::Brain, "代理"),
        PanelKind::Conversations => (icons::IconKind::BotMessageSquare, "对话"),
        PanelKind::Usage => (icons::IconKind::BarChart3, "用量"),
        PanelKind::Acceptance => (icons::IconKind::BadgeCheck, "验收"),
    }
}
```

- [ ] **Step 2: 新增 `panel_badge`**

原样对应现有 `right_icon_rail` 里 `RightView::Acceptance` 分支的
"待处理徽标"逻辑(小圆点,`stack!` 叠在按钮上):

```rust
/// 面板专属的按钮徽标装饰(目前只有验收面板有:当前激活 tab 有待处理
/// 交付时,右上角叠一个金色小圆点)。其余 10 个面板返回 `None`。
fn panel_badge(
    app: &App,
    kind: PanelKind,
) -> Option<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    if kind != PanelKind::Acceptance {
        return None;
    }
    let pending = app
        .active_workspace()
        .and_then(|ws| ws.tabs.get(ws.active))
        .map(|t| t.delivery_pending)
        .unwrap_or(false);
    if !pending {
        return None;
    }
    Some(
        container(iced_widget::Space::new())
            .width(Length::Fixed(8.0))
            .height(Length::Fixed(8.0))
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(byteui::theme::color::current().gold.into()),
                border: Border {
                    radius: 4.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .into(),
    )
}
```

- [ ] **Step 3: 编译**

Run: `cargo build -p dozer-app --bin dozer 2>&1 | grep -E "^error"`
Expected: 无输出(允许 `panel_meta`/`panel_badge` 未使用的 warning)

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): 新增 panel_meta()/panel_badge() 辅助函数,尚未接入渲染"
```

---

### Task 3: `RailButton` 合并 + 图标栏渲染改成统一遍历

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: `panel_meta`/`panel_badge`(Task 2)
- Produces: `RailButton::Panel(PanelKind)` 取代 11 个固定 `Left*`/`Right*`
  variant(`Home*` 三个不动);`fn icon_rail(app: &App, side: Side) ->
  Element<...>` 取代 `left_icon_rail`/`right_icon_rail` 两个函数,内部
  按 `app.rail_layout.side(side)` 遍历渲染。**这个 Task 结束后仍然
  发送 `Message::LeftIconSelect`/`RightIconSelect`**(选中消息统一是
  Task 4 的范围,这里先只改渲染)。

- [ ] **Step 1: 合并 `RailButton`**

找到:

```rust
pub enum RailButton {
    LeftFiles,
    LeftGit,
    LeftTodo,
    LeftProject,
    LeftDatabase,
    LeftSsh,
    LeftWeb,
    RightAgent,
    RightConversations,
    RightUsage,
    RightAcceptance,
    HomeProjectList,
    HomeRecents,
    HomeBrowser,
}
```

改成:

```rust
pub enum RailButton {
    Panel(PanelKind),
    HomeProjectList,
    HomeRecents,
    HomeBrowser,
}
```

- [ ] **Step 2: 用统一的 `icon_rail` 替换 `left_icon_rail`/`right_icon_rail`**

两个函数整段删除,替换成:

```rust
/// 图标栏:按 `app.rail_layout.side(side)` 的顺序遍历渲染。左右两条栏
/// 共用这一份实现——差异(区域样式、选中态取哪个 `*_view`/`*_collapsed`
/// 字段判断)通过 `side` 参数分派。
fn icon_rail(app: &App, side: Side) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = match side {
        Side::Left => theme::region::left_icon_rail(),
        Side::Right => theme::region::right_icon_rail(),
    };
    // 视觉"选中"= 该视图激活 **且**对应面板区展开。点已选中的图标会收起
    // 面板区,此时图标要退回未选中态,所以 `active` 得带上 `!collapsed`
    // ——语义同原 `left_icon_rail`/`right_icon_rail`。
    let (active_kind, open) = match side {
        Side::Left => (app.left_view, !app.left_collapsed),
        Side::Right => (app.right_view, !app.right_collapsed),
    };
    let mut content = column![].spacing(region.gap).padding(region.padding);
    for &kind in app.rail_layout.side(side) {
        let (icon, tooltip) = panel_meta(kind);
        let select_message = match side {
            Side::Left => Message::LeftIconSelect(kind),
            Side::Right => Message::RightIconSelect(kind),
        };
        let base: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            icons::icon_button_entry(
                icon,
                byteui::theme::icon_size::rail(),
                kind == active_kind && open,
                app.hover_progress(HoverId::Rail(RailButton::Panel(kind))),
                true,
                byteui::theme::geometry::rail_button_size(),
                true,
                select_message,
                move |hovered| Message::Hover(HoverId::Rail(RailButton::Panel(kind)), hovered),
                tooltip,
            );
        let entry = match panel_badge(app, kind) {
            Some(badge) => stack![base, badge].into(),
            None => base,
        };
        content = content.push(entry);
    }
    container(content)
        .width(Length::Fixed(byteui::theme::geometry::icon_rail_width()))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
}
```

**关于选中态判断的 `Side::Left => (app.left_view, ...)` / `Side::Right
=> (app.right_view, ...)`——这是这个 Task 唯一还需要读旧字段名
(`left_view`/`right_view`,Stage 1 已经把它们 retype 成 `PanelKind`,
字段名本身没变)的地方,不是遗漏。**

- [ ] **Step 3: 更新调用点**

找到(`view()` 函数里,`body` 变量构造处附近):

```rust
        let body = row![
            left_icon_rail(self),
```

改成:

```rust
        let body = row![
            icon_rail(self, Side::Left),
```

找到 `right_icon_rail(self)` 的调用点,同样改成 `icon_rail(self,
Side::Right)`。

- [ ] **Step 4: 残留检查**

Run: `grep -n "left_icon_rail\|right_icon_rail\|RailButton::Left\|RailButton::Right[ACU]" crates/dozer-app/src/app.rs`
Expected: 无输出

- [ ] **Step 5: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 6: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "refactor(dozer-app): RailButton 合并为 Panel(PanelKind),图标栏渲染改成统一遍历"
```

---

### Task 4: `Message::PanelSelect` + `panel_select()` 合并处理函数

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: `RailLayout::side_of`(Task 1)、`icon_rail`(Task 3)
- Produces: `Message::PanelSelect(PanelKind)` 取代 `LeftIconSelect`/
  `RightIconSelect`;`App::panel_select(&mut self, kind: PanelKind)`
  取代 `left_icon_select`/`right_icon_select`。

- [ ] **Step 1: 合并 `Message` 变体**

找到:

```rust
    LeftIconSelect(PanelKind),
```

和(相隔几行的):

```rust
    RightIconSelect(PanelKind),
```

各自删除,在其中一处位置(建议原 `LeftIconSelect` 的位置)新增:

```rust
    /// 图标栏点击选中某个面板——不区分左右栏,`panel_select` 内部按
    /// `RailLayout::side_of` 查它当前挂在哪条栏。
    PanelSelect(PanelKind),
```

- [ ] **Step 2: 合并处理函数**

删除 `left_icon_select`/`right_icon_select` 整段(包括 Stage 1 已经
retype 过参数类型的版本),替换成:

```rust
    fn panel_select(&mut self, kind: PanelKind) {
        let side = self.rail_layout.side_of(kind);
        match side {
            Side::Left => {
                if self.left_view == kind {
                    // 点的是已选中(激活)的图标:退回未选中并收起对应
                    // 面板区。但若右面板区也已经收起,左就是最后一个还
                    // 开着的 zone,不能关。
                    if !self.right_collapsed {
                        self.left_collapsed = !self.left_collapsed;
                    }
                } else {
                    self.left_view = kind;
                    self.left_collapsed = false;
                }
            }
            Side::Right => {
                if self.right_view == kind {
                    if !self.left_collapsed {
                        self.right_collapsed = !self.right_collapsed;
                    }
                } else {
                    self.right_view = kind;
                    self.right_collapsed = false;
                }
            }
        }
        // 面板专属的"切入时动作"——原 left_icon_select/right_icon_select
        // 七个分支合并到一处,判断条件从"当前是不是这个 view"改成
        // "点的是不是这个 kind 且确实发生了切换"（不区分左右，`side`
        // 已经决定了它读写哪一对 `*_view`/`*_collapsed`）。用一个
        // 局部变量记录"这次调用是否真的切换了面板"（而不是重复收起/
        // 展开),对齐原来两个函数只在 `else` 分支(真正切换时)才跑
        // 副作用、点已选中图标(收起/展开)不跑副作用的行为。
        let switched = match side {
            Side::Left => self.left_view == kind,
            Side::Right => self.right_view == kind,
        };
        if switched {
            match kind {
                PanelKind::GitLog => self.sync_git_log_to_active_project(),
                PanelKind::Todo => {
                    self.with_focused_project(|ws, _io| {
                        if let Some(project) = ws.project.as_ref() {
                            todo::reload_from_disk(
                                &mut ws.todo,
                                std::path::Path::new(&project.path),
                            );
                        }
                    });
                }
                PanelKind::Database => {
                    self.with_focused_project(|ws, _io| {
                        if let Some(project) = ws.project.as_ref() {
                            database::reload_from_disk(
                                &mut ws.database,
                                std::path::Path::new(&project.path),
                            );
                        }
                    });
                }
                PanelKind::Project => self.ensure_project_readme_and_reveal(),
                PanelKind::Ssh => {
                    self.with_focused_project(|ws, _io| {
                        if let Some(project) = ws.project.as_ref() {
                            ssh::reload_from_disk(
                                &mut ws.ssh,
                                std::path::Path::new(&project.path),
                            );
                        }
                    });
                }
                PanelKind::Usage => {
                    self.with_focused_project(|ws, io| {
                        ws.usage.set_loading(true);
                        ws.spawn_usage_refresh(io);
                    });
                }
                PanelKind::Acceptance => {
                    let tab_id = self
                        .active_workspace()
                        .and_then(|ws| ws.tabs.get(ws.active))
                        .map(|t| t.tab_id);
                    if let Some(tab_id) = tab_id {
                        self.update(Message::Acceptance(acceptance::Message::Open(tab_id)));
                    }
                }
                PanelKind::Files | PanelKind::Web | PanelKind::Agent | PanelKind::Conversations => {}
            }
        }
        // 图标栏点击一律退出放大态,同原 left_icon_select/right_icon_select。
        self.maximized = None;
        self.on_shell_layout_changed();
    }
```

**这里的 `switched` 判断("点完之后 `self.left_view`/`right_view` 是否
等于 `kind`")和原代码"是不是走了 `else` 分支"不是同一种写法,但结果
等价**——原代码里 `else` 分支恰好就是"切换成了新面板"的分支,`if` 分支
(已选中,收起/展开)不跑副作用;这里改成"赋值完之后检查当前值"是为了
避免在 `match side` 里把副作用逻辑重复写两遍(Left/Right 各一遍),
两种写法在所有输入下产出相同结果(赋值总是发生在副作用判断之前)。

- [ ] **Step 3: 更新 `update()` 里的消息分派**

找到:

```rust
            Message::LeftIconSelect(v) => self.left_icon_select(v),
            Message::RightIconSelect(v) => self.right_icon_select(v),
```

改成:

```rust
            Message::PanelSelect(v) => self.panel_select(v),
```

- [ ] **Step 4: 更新 `icon_rail`(Task 3)里的 `select_message` 构造**

`crates/dozer-app/src/app.rs` 的 `icon_rail` 函数里找到:

```rust
        let select_message = match side {
            Side::Left => Message::LeftIconSelect(kind),
            Side::Right => Message::RightIconSelect(kind),
        };
```

整段删除,直接在 `icon_button_entry(...)` 调用的对应参数位置传
`Message::PanelSelect(kind)`。

- [ ] **Step 5: 残留检查**

Run: `grep -n "LeftIconSelect\|RightIconSelect\|left_icon_select\|right_icon_select" crates/dozer-app/src/app.rs`
Expected: 无输出

- [ ] **Step 6: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 7: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "refactor(dozer-app): LeftIconSelect/RightIconSelect 合并为 PanelSelect"
```

---

### Task 5: 全量验证

**Files:**
- 无修改,纯验证

**Interfaces:**
- Consumes: Task 1-4 已完成

- [ ] **Step 1: 全量编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer`
Expected: 编译成功;测试全部通过,数量比 Stage 1 结束时的基线略多
(Task 1 新增的 `side_of` 测试)。

- [ ] **Step 2: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 无新增警告(执行前可在 `main` 上跑一遍同样命令留基线比对)、
无格式差异。

- [ ] **Step 3: 独立临时二进制视觉核对**

构建一个独立命名的临时二进制(不要用会撞到用户正在跑的正式
`/Applications/Dozer AI Coder.app` 或其他调试会话进程名的路径),启动
后核对:**11 个面板行为与 Stage 1 结束时完全一致**——

- 每个图标位置、图标形状、tooltip 文案与改动前一致(11 个逐一点一遍)。
- 选中态/hover 态视觉过渡正常(金色高亮、平滑过渡)。
- 验收面板徽标:制造一个"待处理交付"状态,确认金色小圆点正确显示;
  处理完后确认徽标消失。
- 点已选中的图标能正确收起/展开对应面板区;两侧都收起时点其中一侧的
  已选中图标不应该把它也收起(最后一个开着的 zone 不能关,这条规则
  在合并后必须保留)。
- 切进 GitLog/Todo/Database/Project/Ssh/Usage/Acceptance 七个面板,
  确认各自的"切入时动作"仍然触发(Git 图重新布局、Todo/Database/Ssh
  从磁盘重读、Project 面板 README 生成与预览、Usage 手动刷新、
  Acceptance 自动打开当前 tab 的验收视图)。
- 退出重开,确认所有行为在重启后依然一致。

完成后关闭该临时实例、删除临时二进制,不留后台进程。

- [ ] **Step 4: Commit(如果 Step 1-3 发现并修复了任何问题)**

```bash
git branch --show-current
git add -A
git commit -m "fix: Stage 2 全量验证发现的问题修复"
```

(如果 Step 1-3 全部一次通过,这一步跳过,不产生空 commit。)

---

## 完工验收

1. `git log --oneline` 确认全部 commit 都在当前分支上,没有漂到 `main`。
2. `grep -n "left_icon_rail\|right_icon_rail\|LeftIconSelect\|RightIconSelect\|RailButton::Left\|RailButton::Right[ACU]" crates/dozer-app/src/app.rs` 零匹配。
3. `cargo build`/`cargo test`/`cargo clippy`/`cargo fmt --check` 全绿。
4. GUI 视觉核对:11 个面板行为与 Stage 1 结束时逐一致,零差异。
5. 提请审阅。审阅通过合并后,Stage 3(8 个面板的镜像渲染顺序,可按
   面板逐个独立 Task)才能开工——它不直接依赖 Stage 2 的具体实现细节,
   但依赖 `PanelKind`/`RailLayout` 已经是渲染路径的一部分(Stage 2
   证明了这一点在图标栏渲染层是可行的,给 Stage 3/4 在面板内容层做
   同样的事打了样)。
