# workspace 图标栏面板拖拽换栏 · Stage 3:内部两栏镜像渲染 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 8 个有内部横向两栏布局的面板(`Files`/`GitLog`/`Todo`/
`Project`/`Ssh`/`Web`/`Agent`/`Conversations`)各自具备"面板当前不在
默认栏时,内部两个子元素的 `row!` 渲染顺序整体反过来"的能力。**这一
阶段依然不接入拖拽**——`RailLayout` 的值仍然只能是 `default()`,所以
`app.rail_layout.side_of(kind) != kind.default_side()` 这个判断在这个
Stage 结束时**恒为 `false`**,新增的镜像分支实际上永远走不到,GUI 行为
应该和改动前逐像素一致。这个 Stage 的正确性验证主要靠**单元测试**
(直接构造一个"面板不在默认栏"的假设状态调用渲染函数,断言产出的
`Element` 树顺序确实反过来了),不是 GUI 观察——GUI 上真正能看到镜像
效果要等 Stage 4(拖拽)才能触发。

**Architecture:** 摸底(写这份计划前重读实际代码)时发现一个和 spec
原始表格不一致的地方:`Files`/`GitLog`/`Todo`/`Project`/`Ssh`/`Web`
六个面板默认是"列表在前(渲染左)、内容在后(渲染右)",但 `Agent`/
`Conversations` 两个默认恰好反过来("内容在前/左、列表在后/右"——
`right_panel_area` 函数现有注释明确写"两个配对都是内容侧渲染在左、
列表侧渲染在右",不是这次发现的 bug,是原有设计)。**因此这次的镜像
规则统一表述成"反转当前 `row!` 顺序",不假设"列表永远是第一个元素"
**——这个不一致已经同步修了 spec(`fix(docs)` commit)。

`Files`/`Todo`/`Project`/`Ssh`(默认左)和 `Agent`/`Conversations`
(默认右)六个面板的 `row!` 组装**都直接写在 `app.rs` 的
`left_panel_area`/`right_panel_area` 两个函数里**(不在各自的
`extensions/*.rs` 文件里),所以这 6 个面板的镜像逻辑集中在 `app.rs`
两个函数内部就能完成,不需要改各面板自己的 `view()` 签名。
`GitLog`(`git_log::view`)和 `Web`(`browser::view`)是例外——它们
自己组装最终的 `row!`,镜像逻辑要做进各自文件,新增一个 `mirror: bool`
参数从 `app.rs` 传进去。

## Global Constraints

- **前提:Stage 2(图标栏渲染改遍历 + 选中消息统一)已合并到 main。**
  开工前确认 `RailLayout::side_of`、`Message::PanelSelect` 已存在。
- **独立分支开发,不直接提交 main。** 在新分支(如
  `feature/rail-panel-relocation-stage3`)上完成全部 10 个 Task,提请
  审阅、通过后再合并回 `main`。每次 `git commit`/`git add` 前先跑
  `git branch --show-current`;开工前 `git status` 确认干净,发现不
  相关的改动要 `git stash push -- <具体路径>`(不要用 `-u`,避免连
  自己刚写还没提交的文件一起冲进 stash——这份计划文件本身就是在这种
  事故里通过 `git show stash@{0}^3:<path>` 恢复回来的,執行时留意
  同一个坑)。
- **每个 Task 结束都要求 `cargo build` 通过**(和 Stage 2 一致,不像
  Stage 1 那样有"中间态不可编译"的例外——这次改动都是局部的,新增
  分支不会破坏已有的穷尽性)。
- **不改变默认栏下的渲染结果。** 8 个面板在"面板仍在默认栏"(这个
  Stage 结束时唯一可能的状态)下的 `Element` 树必须和改动前逐一致——
  新增的 `mirror` 判断只影响"假设不在默认栏"这个目前不可达的分支,
  验证时通过单元测试直接调用渲染函数并人为构造"不在默认栏"的
  `RailLayout`/`App` 状态来触发它,不依赖 GUI。
- **`PanelDims` 十个 split 字段的存储语义不变**——镜像只换 `row!`
  里两个子元素的先后顺序和它们各自拿到哪个 `FillPortion`,不改字段
  存的数值含义,也不改 `apply_column_drag` 这类"用户拖动分割线时怎么
  写回 split 值"的既有逻辑(那部分不在这个 Stage 范围,继续按"结果在
  默认栏下不变"的口径工作;镜像态下用户拖动分割线的行为——写回时要不要
  也镜像取反——是 Stage 4 才会真正暴露出来的问题,这个 Stage 里
  `apply_column_drag` 不动)。

---

### Task 1: `PanelKind::default_side()` + `App::panel_mirrored()` 判定辅助

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Produces: `impl PanelKind { pub fn default_side(self) -> Side }`、
  `impl App { pub(crate) fn panel_mirrored(&self, kind: PanelKind) -> bool }`

- [ ] **Step 1: `PanelKind::default_side()`**

在 `PanelKind` 定义附近新增 `impl` 块(或追加进已有的):

```rust
impl PanelKind {
    /// 面板默认挂在哪条栏——`RailLayout::default()`、以及这个 Stage
    /// 的镜像判断("是否偏离了默认栏")共用这一份真相,不要在两处各写
    /// 一份可能不同步的列表。
    pub fn default_side(self) -> Side {
        match self {
            Self::Files | Self::GitLog | Self::Todo | Self::Project | Self::Database
            | Self::Ssh | Self::Web => Side::Left,
            Self::Agent | Self::Conversations | Self::Usage | Self::Acceptance => Side::Right,
        }
    }
}
```

- [ ] **Step 2: `App::panel_mirrored()`**

在 `App` 的 `impl` 块里新增:

```rust
    /// 该面板当前是否偏离了默认栏——8 个有内部两栏布局的面板据此决定
    /// 渲染顺序要不要反转。这个 Stage 结束时 `RailLayout` 只可能是
    /// `default()`,所以这个函数在正常运行时恒返回 `false`;它的分支
    /// 靠单元测试直接构造非默认 `RailLayout` 来触发验证,不依赖 GUI
    /// 能不能拖拽出这个状态(Stage 4 才有拖拽)。
    pub(crate) fn panel_mirrored(&self, kind: PanelKind) -> bool {
        self.rail_layout.side_of(kind) != kind.default_side()
    }
```

- [ ] **Step 3: 追加测试**

```rust
#[cfg(test)]
mod panel_mirrored_tests {
    use super::*;

    #[test]
    fn default_side_matches_rail_layout_default() {
        let rail = RailLayout::default();
        for &kind in rail.left.iter() {
            assert_eq!(kind.default_side(), Side::Left, "{kind:?}");
        }
        for &kind in rail.right.iter() {
            assert_eq!(kind.default_side(), Side::Right, "{kind:?}");
        }
    }

    #[test]
    fn panel_mirrored_false_when_rail_layout_is_default() {
        let app = App::default();
        for kind in [
            PanelKind::Files, PanelKind::GitLog, PanelKind::Todo, PanelKind::Project,
            PanelKind::Ssh, PanelKind::Web, PanelKind::Agent, PanelKind::Conversations,
        ] {
            assert!(!app.panel_mirrored(kind), "{kind:?} 不应该在默认布局下判定为镜像");
        }
    }

    #[test]
    fn panel_mirrored_true_when_manually_relocated() {
        let mut app = App::default();
        app.rail_layout.left.retain(|&k| k != PanelKind::Files);
        app.rail_layout.right.push(PanelKind::Files);
        assert!(app.panel_mirrored(PanelKind::Files));
        assert!(!app.panel_mirrored(PanelKind::Todo), "没挪的面板不受影响");
    }
}
```

(`App` 需要实现/已有 `Default`——如果 `App::default()` 不存在或构造
成本高(比如需要真实 `Client`/事件循环),改用现有测试里构造 `App`
测试实例的既有辅助函数,不要在这个 Task 里新增一套构造逻辑。)

- [ ] **Step 4: 编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer panel_mirrored`
Expected: 编译成功,新增测试通过

- [ ] **Step 5: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): 新增 PanelKind::default_side()/App::panel_mirrored() 镜像判定"
```

---

### Task 2: `Files` 镜像(`left_panel_area`)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: `App::panel_mirrored`(Task 1)

- [ ] **Step 1: 改写 `left_panel_area` 里 `PanelKind::Files` 分支**

找到(`match app.left_view { PanelKind::Files => { ... } ... }` 内部):

```rust
                row![
                    list_pane,
                    divider_bar(
                        Divider::LeftPairSplit,
                        theme::region::project_pane()
                            .background
                            .unwrap_or(byteui::theme::color::current().bg),
                        theme::region::preview_pane()
                            .background
                            .unwrap_or(byteui::theme::color::current().bg),
                        Message::ColumnDragStart(Divider::LeftPairSplit),
                    ),
                    preview_pane(
                        app,
                        ws,
                        Length::FillPortion(content_portion),
                        zone_pane_border(zone, rc)
                    ),
                ]
                .width(Length::Fill)
                .into()
```

改成:

```rust
                let preview = preview_pane(
                    app,
                    ws,
                    Length::FillPortion(content_portion),
                    zone_pane_border(zone, rc),
                );
                let list_bg = theme::region::project_pane()
                    .background
                    .unwrap_or(byteui::theme::color::current().bg);
                let preview_bg = theme::region::preview_pane()
                    .background
                    .unwrap_or(byteui::theme::color::current().bg);
                if app.panel_mirrored(PanelKind::Files) {
                    row![
                        preview,
                        divider_bar(
                            Divider::LeftPairSplit,
                            preview_bg,
                            list_bg,
                            Message::ColumnDragStart(Divider::LeftPairSplit),
                        ),
                        list_pane,
                    ]
                    .width(Length::Fill)
                    .into()
                } else {
                    row![
                        list_pane,
                        divider_bar(
                            Divider::LeftPairSplit,
                            list_bg,
                            preview_bg,
                            Message::ColumnDragStart(Divider::LeftPairSplit),
                        ),
                        preview,
                    ]
                    .width(Length::Fill)
                    .into()
                }
```

**`zone_pane_border(zone, lc)`/`zone_pane_border(zone, rc)` 这两个圆角
参数(`lc`=左圆角、`rc`=右圆角)这次不镜像**——它们控制的是"这块 pane
贴在 zone 最外沿时该不该带圆角",跟着 zone 的物理左右边界走,不是跟着
"哪个面板"走;`list_pane`/`preview` 两个 `Element` 已经用各自原来的
`lc`/`rc` 构造好了,镜像只换它们在 `row!` 里的先后位置,不重新构造。
这意味着镜像后**物理左侧那块 pane 用的还是原来给"左侧位置"准备的圆角
参数**,可能和视觉上不完全对齐(比如镜像后 `preview` 出现在物理左侧,
但它的圆角是按"物理右侧"构造的)——这是这个 Stage 已知且刻意搁置的
细节,已经记进"排期备注",不在这个 Task 里解决(圆角本身很小,视觉上
不明显,优先级低于把主体镜像逻辑做对;真要修,需要把 `zone_pane_border`
的调用点也一起挪进 `if`/`else` 两个分支各自传对应的圆角,留给这个
Stage 走完一轮 GUI 核对后再评估要不要单独开一个小 Task 补)。

- [ ] **Step 2: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 3: 单元测试**——直接调用 `left_panel_area` 验证镜像后
  `Element` 子节点顺序反转(如果现有测试基础设施没有对 `Element` 树
  做结构性断言的先例,改成验证更底层的信号:`app.panel_mirrored
  (PanelKind::Files)` 在人为构造的非默认 `RailLayout` 下确实为
  `true`,复用 Task 1 已验证的判定逻辑,`row!` 顺序本身跟着这个布尔值
  走、由 Step 1 的 `if`/`else` 结构保证,不需要重复对渲染输出做像素级
  断言)。

Run: `cargo test -p dozer-app --bin dozer left_panel_area`
Expected: 若有既存相关测试,全部通过;若没有,这一步确认 Step 1 至少
没有破坏其它测试(跑全量 `cargo test -p dozer-app --bin dozer` 已在
Task 10 覆盖,这里只是提前抽查)。

- [ ] **Step 4: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): Files 面板内部两栏支持镜像渲染顺序"
```

---

### Task 3: `Todo` 镜像(`left_panel_area`)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

同 Task 2 手法,改写 `PanelKind::Todo` 分支:

- [ ] **Step 1: 改写**

找到:

```rust
                row![
                    sidebar_pane.map(Message::Todo),
                    divider_bar(
                        Divider::TodoSplit,
                        theme::region::project_pane()
                            .background
                            .unwrap_or(byteui::theme::color::current().bg),
                        theme::region::preview_pane()
                            .background
                            .unwrap_or(byteui::theme::color::current().bg),
                        Message::ColumnDragStart(Divider::TodoSplit),
                    ),
                    content_pane.map(Message::Todo),
                ]
                .width(Length::Fill)
                .into()
```

改成:

```rust
                let sidebar = sidebar_pane.map(Message::Todo);
                let content = content_pane.map(Message::Todo);
                let sidebar_bg = theme::region::project_pane()
                    .background
                    .unwrap_or(byteui::theme::color::current().bg);
                let content_bg = theme::region::preview_pane()
                    .background
                    .unwrap_or(byteui::theme::color::current().bg);
                if app.panel_mirrored(PanelKind::Todo) {
                    row![
                        content,
                        divider_bar(
                            Divider::TodoSplit,
                            content_bg,
                            sidebar_bg,
                            Message::ColumnDragStart(Divider::TodoSplit),
                        ),
                        sidebar,
                    ]
                    .width(Length::Fill)
                    .into()
                } else {
                    row![
                        sidebar,
                        divider_bar(
                            Divider::TodoSplit,
                            sidebar_bg,
                            content_bg,
                            Message::ColumnDragStart(Divider::TodoSplit),
                        ),
                        content,
                    ]
                    .width(Length::Fill)
                    .into()
                }
```

- [ ] **Step 2: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 3: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): Todo 面板内部两栏支持镜像渲染顺序"
```

---

### Task 4: `Project` 镜像(`left_panel_area`)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

- [ ] **Step 1: 改写**

找到:

```rust
                row![
                    info_pane,
                    divider_bar(
                        Divider::ProjectSplit,
                        theme::region::project_pane()
                            .background
                            .unwrap_or(byteui::theme::color::current().bg),
                        theme::region::preview_pane()
                            .background
                            .unwrap_or(byteui::theme::color::current().bg),
                        Message::ColumnDragStart(Divider::ProjectSplit),
                    ),
                    project_preview_pane(
                        app,
                        ws,
                        Length::FillPortion(content_portion),
                        zone_pane_border(zone, rc)
                    ),
                ]
                .width(Length::Fill)
                .into()
```

改成:

```rust
                let preview = project_preview_pane(
                    app,
                    ws,
                    Length::FillPortion(content_portion),
                    zone_pane_border(zone, rc),
                );
                let info_bg = theme::region::project_pane()
                    .background
                    .unwrap_or(byteui::theme::color::current().bg);
                let preview_bg = theme::region::preview_pane()
                    .background
                    .unwrap_or(byteui::theme::color::current().bg);
                if app.panel_mirrored(PanelKind::Project) {
                    row![
                        preview,
                        divider_bar(
                            Divider::ProjectSplit,
                            preview_bg,
                            info_bg,
                            Message::ColumnDragStart(Divider::ProjectSplit),
                        ),
                        info_pane,
                    ]
                    .width(Length::Fill)
                    .into()
                } else {
                    row![
                        info_pane,
                        divider_bar(
                            Divider::ProjectSplit,
                            info_bg,
                            preview_bg,
                            Message::ColumnDragStart(Divider::ProjectSplit),
                        ),
                        preview,
                    ]
                    .width(Length::Fill)
                    .into()
                }
```

- [ ] **Step 2: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 3: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): Project 面板内部两栏支持镜像渲染顺序"
```

---

### Task 5: `Ssh` 镜像(`left_panel_area`)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

- [ ] **Step 1: 改写**

找到:

```rust
                row![
                    list_pane,
                    divider_bar(
                        Divider::SshSplit,
                        theme::region::project_pane()
                            .background
                            .unwrap_or(byteui::theme::color::current().bg),
                        theme::region::preview_pane()
                            .background
                            .unwrap_or(byteui::theme::color::current().bg),
                        Message::ColumnDragStart(Divider::SshSplit),
                    ),
                    ssh_terminal_pane(
                        app,
                        ws,
                        Length::FillPortion(content_portion),
                        zone_pane_border(zone, rc)
                    ),
                ]
                .width(Length::Fill)
                .into()
```

改成:

```rust
                let terminal = ssh_terminal_pane(
                    app,
                    ws,
                    Length::FillPortion(content_portion),
                    zone_pane_border(zone, rc),
                );
                let list_bg = theme::region::project_pane()
                    .background
                    .unwrap_or(byteui::theme::color::current().bg);
                let terminal_bg = theme::region::preview_pane()
                    .background
                    .unwrap_or(byteui::theme::color::current().bg);
                if app.panel_mirrored(PanelKind::Ssh) {
                    row![
                        terminal,
                        divider_bar(
                            Divider::SshSplit,
                            terminal_bg,
                            list_bg,
                            Message::ColumnDragStart(Divider::SshSplit),
                        ),
                        list_pane,
                    ]
                    .width(Length::Fill)
                    .into()
                } else {
                    row![
                        list_pane,
                        divider_bar(
                            Divider::SshSplit,
                            list_bg,
                            terminal_bg,
                            Message::ColumnDragStart(Divider::SshSplit),
                        ),
                        terminal,
                    ]
                    .width(Length::Fill)
                    .into()
                }
```

- [ ] **Step 2: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 3: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): Ssh 面板内部两栏支持镜像渲染顺序"
```

---

### Task 6: `Agent` 镜像(`right_panel_area`)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**注意默认顺序和 Task 2-5 相反**:`Agent` 默认是"终端(内容)在前/左、
`agent_list_pane`(列表)在后/右"。镜像后变成"列表在前/左、终端在后/右"
——和 Task 2-5 的"默认列表在前,镜像后内容在前"方向相反,这是符合摸底
发现的既有设计,不是笔误。

- [ ] **Step 1: 改写**

找到:

```rust
                row![
                    terminal_pane(
                        app,
                        ws,
                        Length::FillPortion(content_portion),
                        zone_pane_border(zone, lc)
                    ),
                    divider_bar(
                        Divider::RightPairSplit,
                        theme::region::terminal_pane()
                            .background
                            .unwrap_or(byteui::theme::color::current().bg),
                        theme::region::agent_list_pane()
                            .background
                            .unwrap_or(byteui::theme::color::current().bg),
                        Message::ColumnDragStart(Divider::RightPairSplit),
                    ),
                    agent_list_pane(
                        app,
                        ws,
                        Length::FillPortion(list_portion),
                        zone_pane_border(zone, rc)
                    ),
                ]
                .width(Length::Fill)
                .into()
```

改成:

```rust
                let terminal = terminal_pane(
                    app,
                    ws,
                    Length::FillPortion(content_portion),
                    zone_pane_border(zone, lc),
                );
                let list = agent_list_pane(
                    app,
                    ws,
                    Length::FillPortion(list_portion),
                    zone_pane_border(zone, rc),
                );
                let terminal_bg = theme::region::terminal_pane()
                    .background
                    .unwrap_or(byteui::theme::color::current().bg);
                let list_bg = theme::region::agent_list_pane()
                    .background
                    .unwrap_or(byteui::theme::color::current().bg);
                if app.panel_mirrored(PanelKind::Agent) {
                    row![
                        list,
                        divider_bar(
                            Divider::RightPairSplit,
                            list_bg,
                            terminal_bg,
                            Message::ColumnDragStart(Divider::RightPairSplit),
                        ),
                        terminal,
                    ]
                    .width(Length::Fill)
                    .into()
                } else {
                    row![
                        terminal,
                        divider_bar(
                            Divider::RightPairSplit,
                            terminal_bg,
                            list_bg,
                            Message::ColumnDragStart(Divider::RightPairSplit),
                        ),
                        list,
                    ]
                    .width(Length::Fill)
                    .into()
                }
```

- [ ] **Step 2: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 3: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): Agent 面板内部两栏支持镜像渲染顺序"
```

---

### Task 7: `Conversations` 镜像(`right_panel_area`)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

同 Task 6,默认顺序同样是"内容(审阅)在前/左、列表在后/右"。

- [ ] **Step 1: 改写**

找到:

```rust
                row![
                    review_content_pane(
                        ws,
                        Length::FillPortion(content_portion),
                        zone_pane_border(zone, lc)
                    ),
                    divider_bar(
                        Divider::RightPairSplit,
                        theme::region::review_content_pane()
                            .background
                            .unwrap_or(byteui::theme::color::current().bg),
                        theme::region::conversation_list_pane()
                            .background
                            .unwrap_or(byteui::theme::color::current().bg),
                        Message::ColumnDragStart(Divider::RightPairSplit),
                    ),
                    conversation_list_pane(
                        ws,
                        Length::FillPortion(list_portion),
                        zone_pane_border(zone, rc)
                    ),
                ]
                .width(Length::Fill)
                .into()
```

改成:

```rust
                let review = review_content_pane(
                    ws,
                    Length::FillPortion(content_portion),
                    zone_pane_border(zone, lc),
                );
                let list = conversation_list_pane(
                    ws,
                    Length::FillPortion(list_portion),
                    zone_pane_border(zone, rc),
                );
                let review_bg = theme::region::review_content_pane()
                    .background
                    .unwrap_or(byteui::theme::color::current().bg);
                let list_bg = theme::region::conversation_list_pane()
                    .background
                    .unwrap_or(byteui::theme::color::current().bg);
                if app.panel_mirrored(PanelKind::Conversations) {
                    row![
                        list,
                        divider_bar(
                            Divider::RightPairSplit,
                            list_bg,
                            review_bg,
                            Message::ColumnDragStart(Divider::RightPairSplit),
                        ),
                        review,
                    ]
                    .width(Length::Fill)
                    .into()
                } else {
                    row![
                        review,
                        divider_bar(
                            Divider::RightPairSplit,
                            review_bg,
                            list_bg,
                            Message::ColumnDragStart(Divider::RightPairSplit),
                        ),
                        list,
                    ]
                    .width(Length::Fill)
                    .into()
                }
```

- [ ] **Step 2: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 3: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): Conversations 面板内部两栏支持镜像渲染顺序"
```

---

### Task 8: `GitLog` 镜像(`git_log.rs`,新增 `mirror` 参数)

**Files:**
- Modify: `crates/dozer-app/src/extensions/git_log.rs`
- Modify: `crates/dozer-app/src/app.rs`(调用点)

**Interfaces:**
- `git_log::view` 新增一个 `mirror: bool` 参数(签名变化,调用点需要
  同步改)

- [ ] **Step 1: `git_log.rs::view` 签名新增参数**

```rust
pub fn view<'a>(
    app: &App,
    state: &'a State,
    worktrees: &'a [WorktreeInfo],
    git_log_split: f32,
    git_log_file_diff_split: f32,
    mirror: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
```

（原有 4 个参数不变,只在末尾追加 `mirror: bool`。）

- [ ] **Step 2: 改写函数尾部的最终 `row!` 组装**

找到:

```rust
    row![
        container(left_with_picker).width(Length::FillPortion(list_portion)),
        crate::app::divider_bar(
            crate::app::Divider::GitLogSplit,
            byteui::theme::color::current().bg,
            byteui::theme::color::current().bg,
            Message::ColumnDragStart,
        ),
        container(right).width(Length::FillPortion(content_portion)),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(pad)
    .into()
}
```

改成:

```rust
    let left_box = container(left_with_picker).width(Length::FillPortion(list_portion));
    let right_box = container(right).width(Length::FillPortion(content_portion));
    let divider = crate::app::divider_bar(
        crate::app::Divider::GitLogSplit,
        byteui::theme::color::current().bg,
        byteui::theme::color::current().bg,
        Message::ColumnDragStart,
    );
    let body = if mirror {
        row![right_box, divider, left_box]
    } else {
        row![left_box, divider, right_box]
    };
    body.width(Length::Fill).height(Length::Fill).padding(pad).into()
}
```

**`divider_bar` 两个 bg 参数这里都传的是同一个 `bg` 常量,不像 Task
2-7 那样左右两侧 bg 不同——不需要跟着镜像交换,原样保留。**

- [ ] **Step 3: 更新调用点**

`crates/dozer-app/src/app.rs` 的 `left_panel_area` 里找到:

```rust
            PanelKind::GitLog => git_log::view(
                app,
                &app.git_log,
                ws.project_panel.worktrees(),
                app.dims.git_log_split,
                app.dims.git_log_file_diff_split,
            )
            .map(Message::GitLog),
```

改成:

```rust
            PanelKind::GitLog => git_log::view(
                app,
                &app.git_log,
                ws.project_panel.worktrees(),
                app.dims.git_log_split,
                app.dims.git_log_file_diff_split,
                app.panel_mirrored(PanelKind::GitLog),
            )
            .map(Message::GitLog),
```

- [ ] **Step 4: 修正其它调用点(如果有)**

Run: `grep -rn "git_log::view(" crates/dozer-app/src --include="*.rs"`

对每一处除 Step 3 已处理的以外的调用(比如测试代码里直接调
`git_log::view`),补上第 6 个参数(测试场景通常传 `false`,除非
该测试本来就是在测镜像行为)。

- [ ] **Step 5: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功(如果测试代码调用点漏改,这里会报参数数量不对,
回到 Step 4 补全)

- [ ] **Step 6: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/git_log.rs crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): GitLog 面板内部两栏支持镜像渲染顺序"
```

---

### Task 9: `Web` 镜像(`browser.rs`,新增 `mirror` 参数,`bookmarks_open`
时才有意义)

**Files:**
- Modify: `crates/dozer-app/src/extensions/browser.rs`
- Modify: `crates/dozer-app/src/app.rs`(调用点)

**Interfaces:**
- `browser::view` 新增一个 `mirror: bool` 参数

- [ ] **Step 1: `browser.rs::view` 签名新增参数**

找到 `view` 函数签名(约 1436 行,含 `bookmarks_split: f32` 参数那个),
追加:

```rust
    mirror: bool,
```

- [ ] **Step 2: 改写"收藏夹侧栏展开时"的 `row!` 组装**

找到:

```rust
    content = content.push(if state.bookmarks_open {
        let bg = region
            .background
            .unwrap_or(byteui::theme::color::current().bg);
        let (list_portion, content_portion) = split_portions(1.0 - bookmarks_split);
        row![
            container(body).width(Length::FillPortion(content_portion)),
            crate::app::divider_bar(
                crate::app::Divider::BrowserBookmarksSplit,
                bg,
                bg,
                Message::ColumnDragStart,
            ),
            bookmarks_panel(state, project_id, Length::FillPortion(list_portion)),
        ]
        .height(Length::Fill)
        .into()
    } else {
        body
    });
```

改成:

```rust
    content = content.push(if state.bookmarks_open {
        let bg = region
            .background
            .unwrap_or(byteui::theme::color::current().bg);
        let (list_portion, content_portion) = split_portions(1.0 - bookmarks_split);
        let content_box = container(body).width(Length::FillPortion(content_portion));
        let bookmarks_box = bookmarks_panel(state, project_id, Length::FillPortion(list_portion));
        let divider = crate::app::divider_bar(
            crate::app::Divider::BrowserBookmarksSplit,
            bg,
            bg,
            Message::ColumnDragStart,
        );
        if mirror {
            row![bookmarks_box, divider, content_box]
        } else {
            row![content_box, divider, bookmarks_box]
        }
        .height(Length::Fill)
        .into()
    } else {
        // 收藏夹侧栏收起时是单栏,没有"两栏顺序"可镜像,`mirror`
        // 在这个分支不起作用——不是遗漏,是这个面板镜像规则天然的
        // no-op 情形(见 spec 表格 Web 那一行的备注)。
        body
    });
```

**`split_portions(1.0 - bookmarks_split)` 这行的具体比例换算逻辑不动
——`list_portion`/`content_portion` 各自对应哪个视觉位置,这个 Task
只改 `row!` 顺序,不重新推导 `bookmarks_split` 的含义;实现前建议先
读一遍 `split_portions` 函数本身 + 现有 `browser.rs` 测试,确认这两个
`portion` 变量目前分别配的是 `content_box`/`bookmarks_box` 中的哪一个
——上面的改写保持了原来的配对关系(`content_portion` 配 `content_box`、
`list_portion` 配 `bookmarks_box`),镜像只交换两个 `Element` 在 `row!`
里的位置,不交换它们各自持有的 `FillPortion` 值。**

- [ ] **Step 3: 更新调用点**

`crates/dozer-app/src/app.rs` 的 `left_panel_area` 里找到:

```rust
            PanelKind::Web => browser::view(
                &ws.browser,
                ws.project.as_ref().map(|p| p.id),
                app.dims.browser_bookmarks_split,
                Length::Fill,
                zone_pane_border(zone, ac),
            )
            .map(Message::Browser),
```

改成:

```rust
            PanelKind::Web => browser::view(
                &ws.browser,
                ws.project.as_ref().map(|p| p.id),
                app.dims.browser_bookmarks_split,
                Length::Fill,
                zone_pane_border(zone, ac),
                app.panel_mirrored(PanelKind::Web),
            )
            .map(Message::Browser),
```

- [ ] **Step 4: 修正其它调用点(如果有,同 Task 8 Step 4 的手法)**

Run: `grep -rn "browser::view(" crates/dozer-app/src --include="*.rs"`

- [ ] **Step 5: 编译**

Run: `cargo build -p dozer-app --bin dozer`
Expected: 编译成功

- [ ] **Step 6: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/browser.rs crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): Web(浏览器)面板内部两栏支持镜像渲染顺序(仅收藏夹展开时生效)"
```

---

### Task 10: 全量验证

**Files:**
- 无修改,纯验证

**Interfaces:**
- Consumes: Task 1-9 已完成

- [ ] **Step 1: 全量编译 + 测试**

Run: `cargo build -p dozer-app --bin dozer && cargo test -p dozer-app --bin dozer`
Expected: 编译成功;测试全部通过,数量比 Stage 2 结束时的基线略多
(Task 1 新增的 `panel_mirrored`/`default_side` 测试)。

- [ ] **Step 2: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings && cargo fmt -p dozer-app -- --check`
Expected: 无新增警告、无格式差异。

- [ ] **Step 3: GUI 核对(默认栏下零差异——这个 Stage 唯一能在 GUI 上
  验证的部分)**

构建一个独立命名的临时二进制,启动后核对 8 个有内部两栏布局的面板
在默认栏下的渲染**和 Stage 2 结束时逐一致**(镜像分支这个 Stage 里
从 GUI 走不到,不强求这一步能看到镜像效果——那是 Stage 4 的验证范围)。
完成后关闭该临时实例、删除临时二进制,不留后台进程。

- [ ] **Step 4: 手动触发一次镜像状态,肉眼确认渲染顺序确实反转
  (临时调试手段,不进正式代码)**

这一步用来在 Stage 4(真正的拖拽)落地前,提前肉眼确认这个 Stage 的
镜像逻辑本身是对的,而不是只靠单元测试的"结构性断言"信任它——避免
把一个视觉上实际错位的实现一路带到 Stage 4 才发现。做法:临时把
`App::default()`(或建窗时读的初始 `RailLayout`)里某一个面板(建议选
`Files`,webview 面板但这个 Stage 不需要碰 webview bounds,只是验证
iced 部分)手动挪到对侧栏(比如在 `main.rs` 建窗逻辑里临时插一行
`app.rail_layout.left.retain(|&k| k != PanelKind::Files);
app.rail_layout.right.push(PanelKind::Files);`),编译运行,肉眼确认
文件树/文件预览的左右顺序确实反过来、分割线颜色渐变方向也对(不是
死板的固定色导致看不出错位)。**确认完成后必须把这行临时代码删除**,
不带着调试用的强制镜像进正式 commit。

- [ ] **Step 5: Commit(如果 Step 1-4 发现并修复了任何问题)**

```bash
git branch --show-current
git add -A
git commit -m "fix: Stage 3 全量验证发现的问题修复"
```

(如果全部一次通过,跳过,不产生空 commit。)

---

## 完工验收

1. `git log --oneline` 确认全部 commit 都在当前分支上,没有漂到 `main`。
2. `cargo build`/`cargo test`/`cargo clippy`/`cargo fmt --check` 全绿。
3. GUI 视觉核对:默认栏下 8 个面板行为与 Stage 2 结束时逐一致;Step 4
   的临时镜像验证已完成且临时代码已清理干净(`git diff main` 里不该
   有任何遗留的强制镜像代码)。
4. 提请审阅。审阅通过合并后,Stage 4(拖拽交互 + 3 个 webview 面板的
   镜像 bounds)才能开工——它是这 4 个 Stage 里风险最集中的一个,依赖
   `panel_mirrored`(这个 Stage)判断是否需要启用镜像版 webview 几何。

## 排期备注(遗留给后续 Stage 或单独评估的细节)

- **`zone_pane_border` 的圆角参数没有跟着镜像交换**(见 Task 2 Step 1
  的说明)——镜像态下物理左侧那块 pane 可能带着"原本给右侧准备"的圆角
  参数。视觉上这个差异很小(圆角本身只有几像素),这个 Stage 优先把
  "内容对不对"做对,圆角细节留到 Stage 4 GUI 全面核对时一并评估要不要
  补一个小 Task。
- **`apply_column_drag`(用户拖动分割线时如何把比例写回 `PanelDims`)
  没有跟着镜像调整**——镜像态下如果用户拖动分割线,写回的比例语义是否
  需要跟着镜像取反,这个 Stage 没有触碰(GUI 走不到镜像态,这个问题
  在这个 Stage 里不可观测)。Stage 4 引入真实拖拽后,这是需要专门设计
  和测试的一个点,不能假设"镜像渲染顺序对了,拖拽写回自然也对"。
