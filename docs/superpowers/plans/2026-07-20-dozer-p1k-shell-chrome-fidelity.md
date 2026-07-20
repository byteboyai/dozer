# Dozer Shell Chrome 还原度实现计划（P1k）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `dozer-app` 外观从"功能可用但粗糙"提到"整体气质接近 Figma S1、协调可用"——补齐顶栏、状态栏、文件树图标、卡片/pill 打磨。

**Architecture:** 逐区域增量（方案 A），一个 spec 五个可独立交付的 task。`view()` 由 `row![四栏]` 改为 `column![top_bar, row![四栏]]`（唯一顶层结构变化）；状态栏并入各 pane 的 `column!` 末尾。所有可判定逻辑抽成纯函数单测，视觉走真机 dogfood 目测。

**Tech Stack:** Rust, iced 0.14（`iced_widget` 的 `container`/`button`/`text`/`row!`/`column!`/`horizontal_space`），已有 `theme.rs`（14 色）、`goal.rs`（`parse_goal`）、dozerd `AcceptanceStore`、UDS JSON-Lines 协议。

## Global Constraints

- 颜色**只取自 `crates/dozer-app/src/theme.rs`**：BG `#0a0e16`、PANEL `#0e1620`、TERM_BG `#08141d`、CARD `#12202a`、BORDER `#1c3440`、CREAM `#ffe5b4`、BODY `#9ab4c4`、DIM `#6b7f8f`、GOLD `#f2d94e`、CYAN `#47def0`、GREEN `#1ad585`、PURPLE `#9580ff`、RED `#ff6e6e`。禁止新增硬编码色值。
- 金色 `GOLD` 是甲方动作专属色（目标胶囊、验收横幅）。
- **不引外部图标资源、不用 emoji**：mac GB18030 位图字体会毒化 CJK/emoji 回退（advance=inf，见 `fonts.rs`）。文件树图标只用已验证可渲染的几何字形（`▾ ▸ ● · + −` 之类，全项目已在用）。
- iced 0.14 生态，不引入 Swift/AppKit；核心不依赖 Node/Python。
- 每个可判定逻辑抽纯函数并单测；每 task 收尾 `cargo clippy --all-targets && cargo fmt -- --check` 干净、`cargo test -p <crate>` 绿。
- 标杆 = **神似**，非像素级；视觉占位元素（搜索框、齿轮、组件 tab）无副作用、不显假数据。
- macOS 二进制在 `target/aarch64-apple-darwin/debug/dozer`（本机有 `--target` 配置，`cargo build -p dozer-app` 产物在该三元组下）。

---

### Task 1: 顶栏（top bar + 目标胶囊 + 项目级 goal 载入）

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（加 `project_goal` 字段、`ProjectOpened`/`bootstrap` 载入、`goal_capsule_text` 纯函数、`top_bar` 渲染、`view()` 重构）
- Consume: `crates/dozer-app/src/goal.rs`（`Goal { title, criteria }`、`parse_goal`、`goal_path`）

**Interfaces:**
- Produces: `fn goal_capsule_text(goal: Option<&Goal>, max_chars: usize) -> Option<String>`；`fn top_bar(ws: &Workspace) -> Element<...>`；`Workspace.project_goal: Option<Goal>`。

- [ ] **Step 1: 写失败测试**（追加到 `workspace.rs` 的 `#[cfg(test)] mod tests`）

```rust
#[test]
fn goal_capsule_prefixes_and_truncates() {
    use crate::goal::Goal;
    let g = Goal { title: "会话存活 daemon 雏形".into(), criteria: vec![] };
    assert_eq!(goal_capsule_text(Some(&g), 100).as_deref(), Some("目标：会话存活 daemon 雏形"));
    // 过长按字符截断并加省略号（max_chars 含省略号位）
    let long = Goal { title: "一二三四五六七八九十".into(), criteria: vec![] };
    assert_eq!(goal_capsule_text(Some(&long), 5).as_deref(), Some("目标：一二三四…"));
    // 无 goal / 空标题 → None（胶囊隐藏）
    assert_eq!(goal_capsule_text(None, 10), None);
    let empty = Goal { title: "   ".into(), criteria: vec![] };
    assert_eq!(goal_capsule_text(Some(&empty), 10), None);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app goal_capsule_prefixes_and_truncates`
Expected: FAIL —— `cannot find function goal_capsule_text`。

- [ ] **Step 3: 实现纯函数**（加在 `workspace.rs` 其它纯函数附近，如 `project_branch_label` 后）

```rust
/// 顶栏目标胶囊文案：`目标：{标题}`；标题过长按字符截断加省略号。
/// 无 goal 或空标题 → None（胶囊隐藏）。`max_chars` 含省略号占位。
fn goal_capsule_text(goal: Option<&Goal>, max_chars: usize) -> Option<String> {
    let title = goal?.title.trim();
    if title.is_empty() {
        return None;
    }
    let shown = if title.chars().count() > max_chars {
        let mut s: String = title.chars().take(max_chars.saturating_sub(1)).collect();
        s.push('…');
        s
    } else {
        title.to_string()
    };
    Some(format!("目标：{shown}"))
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app goal_capsule_prefixes_and_truncates`
Expected: PASS。

- [ ] **Step 5: 加 `project_goal` 字段并在打开项目时载入**

在 `Workspace` 结构体字段区加（`goal` 已 `use crate::goal::{self, Goal};`）：
```rust
    /// 顶栏胶囊用的项目级目标（打开项目时同步读 .dozer/goal.md）。
    project_goal: Option<Goal>,
```
两个构造处初始化：`bootstrap` 的 `Self { ... }` 里，紧跟 `file_tree,` 之后加
```rust
            project_goal: project
                .as_ref()
                .and_then(|p| load_project_goal(&p.path)),
```
`with_daemon_error` 的 `Self { ... }` 里加 `project_goal: None,`。
`ProjectOpened` 处理分支里，在 `self.project = project;` **之后**加：
```rust
                self.project_goal = self
                    .project
                    .as_ref()
                    .and_then(|p| load_project_goal(&p.path));
```
并新增小工具函数（放纯函数区；小文件同步读可容忍，同 `last_turn_head` 的既有做法）：
```rust
/// 同步读 `.dozer/goal.md` 并解析（顶栏胶囊用；文件极小，可容忍同步读）。
fn load_project_goal(repo_path: &str) -> Option<Goal> {
    let md = std::fs::read_to_string(goal::goal_path(Path::new(repo_path))).ok()?;
    goal::parse_goal(&md)
}
```

- [ ] **Step 6: 实现 `top_bar` 并重构 `view()`**

在 pane 函数区加：
```rust
/// 顶栏：左 Dozer 标题、中 ⌘K 搜索框（视觉占位）、右 金色目标胶囊 + 设置齿轮（占位）。
fn top_bar(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let title = text("Dozer").size(15).color(theme::CREAM);

    let search = container(text("搜索作品、会话、产物…  ⌘K").size(13).color(theme::DIM))
        .padding([6, 12])
        .width(Length::Fixed(360.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border { color: theme::BORDER, width: 1.0, radius: 6.0.into() },
            ..container::Style::default()
        });

    let mut right = row![].spacing(10);
    if let Some(cap) = goal_capsule_text(ws.project_goal.as_ref(), 28) {
        let capsule = container(
            row![
                text("●").size(9).color(theme::GOLD),
                text(cap).size(13).color(theme::CREAM)
            ]
            .spacing(6),
        )
        .padding([5, 10])
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border { color: theme::GOLD, width: 1.0, radius: 6.0.into() },
            ..container::Style::default()
        });
        right = right.push(capsule);
    }
    right = right.push(text("⚙").size(15).color(theme::DIM));

    let bar = row![title, search, iced_widget::horizontal_space(), right]
        .spacing(16)
        .padding([0, 12])
        .align_y(iced_widget::core::Alignment::Center);

    container(bar)
        .width(Length::Fill)
        .height(Length::Fixed(44.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::BG.into()),
            border: Border { color: theme::BORDER, width: 0.0, radius: 0.0.into() },
            ..container::Style::default()
        })
        .into()
}
```
`view()` 改为：
```rust
    pub fn view(
        &self,
    ) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
        let top = top_bar(self);
        let col1 = project_pane(self);
        let col2 = preview_pane(self);
        let col3 = terminal_pane(self);
        let col4 = ai_pane(self);
        column![top, row![col1, col2, col3, col4]].into()
    }
```

- [ ] **Step 7: 编译 + 全量测试 + clippy + fmt**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 测试全绿；clippy `clean`；fmt 无输出。
（`horizontal_space`/`Alignment`/`column!` 若报未导入，按编译器提示补 `use`：`iced_widget::{column, horizontal_space}`、`iced_widget::core::Alignment`。）

- [ ] **Step 8: 真机目测**

Run: `target/aarch64-apple-darwin/debug/dozer`（打开本仓）
Expected: 顶栏出现——`Dozer` 标题、⌘K 搜索框、金框目标胶囊"目标：{goal 首行}"、齿轮；四栏在顶栏下方。无项目时胶囊隐、搜索框在。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(P1k顶栏): Dozer 标题 + ⌘K 搜索框(占位) + 金色目标胶囊 + 齿轮; view() 加顶栏"
```

---

### Task 2: 状态栏（项目栏底 + 终端栏底）

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（加 `agent_state_label`/`env_status_text` 纯函数、`project_status_bar`/`terminal_status_bar` 渲染，接到 `project_pane`/`terminal_pane` 末尾）

**Interfaces:**
- Consumes: `AgentState`（`Idle|Running|AwaitingInput|TurnEnded`）、`dot_color`、`project_branch_label`、`Workspace.daemon_error: Option<String>`、`Workspace.active`/`tabs`。
- Produces: `fn agent_state_label(AgentState) -> &'static str`；`fn env_status_text(bool) -> (&'static str, Color)`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn agent_state_label_covers_all() {
    assert_eq!(agent_state_label(AgentState::Running), "运行中");
    assert_eq!(agent_state_label(AgentState::AwaitingInput), "待输入");
    assert_eq!(agent_state_label(AgentState::TurnEnded), "回合毕");
    assert_eq!(agent_state_label(AgentState::Idle), "空闲");
}

#[test]
fn env_status_text_ok_and_down() {
    assert_eq!(env_status_text(true), ("环境正常 · dozerd 运行中", theme::GREEN));
    assert_eq!(env_status_text(false), ("dozerd 未连接", theme::RED));
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app -- agent_state_label_covers_all env_status_text_ok_and_down`
Expected: FAIL —— 函数未定义。

- [ ] **Step 3: 实现两个纯函数**

```rust
/// agent 四态中文（终端状态栏用）。
fn agent_state_label(state: AgentState) -> &'static str {
    match state {
        AgentState::Running => "运行中",
        AgentState::AwaitingInput => "待输入",
        AgentState::TurnEnded => "回合毕",
        AgentState::Idle => "空闲",
    }
}

/// 环境状态栏文案 + 点色：daemon 连通=绿"环境正常", 断=红"未连接"。
fn env_status_text(daemon_ok: bool) -> (&'static str, Color) {
    if daemon_ok {
        ("环境正常 · dozerd 运行中", theme::GREEN)
    } else {
        ("dozerd 未连接", theme::RED)
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app -- agent_state_label_covers_all env_status_text_ok_and_down`
Expected: PASS。

- [ ] **Step 5: 渲染两条状态栏并挂到 pane 末尾**

加渲染函数：
```rust
/// 项目栏底状态条：左 环境/dozerd 点，右 [文件|git {分支}|组件]（文件高亮,组件占位）。
fn project_status_bar(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let (env, dot) = env_status_text(ws.daemon_error.is_none());
    let left = row![text("●").size(9).color(dot), text(env).size(11).color(theme::BODY)].spacing(6);
    let git = format!("git {}", project_branch_label(ws.branch.as_deref(), ws.dirty));
    let tabs = row![
        text("文件").size(11).color(theme::CREAM),
        text("·").size(11).color(theme::DIM),
        text(git).size(11).color(theme::BODY),
        text("·").size(11).color(theme::DIM),
        text("组件").size(11).color(theme::DIM),
    ]
    .spacing(6);
    status_bar_container(row![left, iced_widget::horizontal_space(), tabs].align_y(iced_widget::core::Alignment::Center))
}

/// 终端栏底状态条：当前激活 tab 的 agent 态 · resume · dozerd 持有。
fn terminal_status_bar(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let (label, dot) = match ws.tabs.get(ws.active) {
        Some(t) => (agent_state_label(t.agent_state), dot_color(t.agent_state, t.alive)),
        None => ("空闲", theme::DIM),
    };
    let resume = ws.tabs.get(ws.active).map(|t| t.alive).unwrap_or(false);
    let line = row![
        text("●").size(9).color(dot),
        text(label).size(11).color(theme::BODY),
        text("·").size(11).color(theme::DIM),
        text(format!("resume {}", if resume { "✓" } else { "—" })).size(11).color(theme::BODY),
        text("·").size(11).color(theme::DIM),
        text("dozerd 持有 · 断连可恢复").size(11).color(theme::DIM),
    ]
    .spacing(6);
    status_bar_container(line)
}

/// 状态条通用外框：略深底 + 上边线 + 固定高。
fn status_bar_container<'a>(
    inner: impl Into<Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>>,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    container(inner)
        .width(Length::Fill)
        .height(Length::Fixed(26.0))
        .padding([0, 8])
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            border: Border { color: theme::BORDER, width: 1.0, radius: 0.0.into() },
            ..container::Style::default()
        })
        .into()
}
```
挂载：`project_pane` 结尾把内容包成上下结构——原本 `container(content.padding(8))`，改为在其外再叠状态栏。最省改法：把 `content` 主体设为可滚动/占满，末尾 push 一个 `iced_widget::vertical_space()` 撑开，再让整栏是 `column![主体(Fill), project_status_bar(ws)]`。具体：`project_pane` 的返回改为
```rust
    let body = container(content.padding(8))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(/* 原 PANEL 样式 */);
    container(column![body, project_status_bar(ws)])
        .width(Length::Fixed(PROJECT_COL_WIDTH))
        .height(Length::Fill)
        .into()
```
`terminal_pane` 同理：原 `container(content.spacing(4).padding(8))` 作为 `body`（`height(Fill)`），外层 `column![body, terminal_status_bar(ws)]`，整栏 `Length::Fill` 宽。

- [ ] **Step 6: 编译 + 测试 + clippy + fmt**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 全绿 / clean / 无输出。（`vertical_space`/`horizontal_space` 按提示补 `use`。）

- [ ] **Step 7: 真机目测**

Run: `target/aarch64-apple-darwin/debug/dozer`
Expected: 项目栏底出现绿点"环境正常 · dozerd 运行中"+右侧 [文件·git main·组件]；终端栏底出现 agent 态行。杀掉 dozerd 重连失败态时项目栏底显红"dozerd 未连接"。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(P1k状态栏): 项目栏底 环境/dozerd+文件/git/组件; 终端栏底 agent态/resume/持有"
```

---

### Task 3: 文件树图标 + 彩色状态点

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（加 `tree_row_glyph`/`tree_row_dot` 纯函数；重写 `project_pane` 文件树行渲染，把"文字变色+后缀"改为"字形 + 行尾彩色 ● 点"）

**Interfaces:**
- Consumes: `FileTree::visible_rows()`（`row.is_dir`/`row.expanded`/`row.depth`/`row.name`/`row.path`）、`FileStatus`、`delivery::dir_status`、`ws.git_statuses`。
- Produces: `fn tree_row_glyph(is_dir: bool, expanded: bool) -> &'static str`；`fn tree_row_dot(status: FileStatus) -> Color`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn tree_glyph_dir_toggles_file_is_dot() {
    assert_eq!(tree_row_glyph(true, true), "▾ ");
    assert_eq!(tree_row_glyph(true, false), "▸ ");
    assert_eq!(tree_row_glyph(false, false), "· ");
}

#[test]
fn tree_dot_maps_status_colors() {
    assert_eq!(tree_row_dot(FileStatus::Modified), theme::GOLD);
    assert_eq!(tree_row_dot(FileStatus::New), theme::GREEN);
    assert_eq!(tree_row_dot(FileStatus::Deleted), theme::RED);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app -- tree_glyph_dir_toggles_file_is_dot tree_dot_maps_status_colors`
Expected: FAIL —— 函数未定义。

- [ ] **Step 3: 实现两个纯函数**

```rust
/// 文件树行前导字形：目录展开/收拢三角,文件用中点。不用 emoji（字体毒化,见 fonts.rs）。
fn tree_row_glyph(is_dir: bool, expanded: bool) -> &'static str {
    match (is_dir, expanded) {
        (true, true) => "▾ ",
        (true, false) => "▸ ",
        (false, _) => "· ",
    }
}

/// 文件/目录 git 状态 → 行尾彩色圆点色。金=改/绿=新/红=删。
fn tree_row_dot(status: FileStatus) -> Color {
    match status {
        FileStatus::Modified => theme::GOLD,
        FileStatus::New => theme::GREEN,
        FileStatus::Deleted => theme::RED,
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app -- tree_glyph_dir_toggles_file_is_dot tree_dot_maps_status_colors`
Expected: PASS。

- [ ] **Step 5: 重写文件树行渲染**（`project_pane` 内 `for row in tree.visible_rows()` 循环体，替换现有 `glyph`/`deco`/`label` 逻辑）

```rust
                for row in tree.visible_rows() {
                    let indent = "  ".repeat(row.depth);
                    let glyph = tree_row_glyph(row.is_dir, row.expanded);
                    let status = if row.is_dir {
                        delivery::dir_status(&row.path, &ws.git_statuses)
                    } else {
                        ws.git_statuses.get(&row.path).copied()
                    };
                    let name_color = if row.is_dir { theme::BODY } else { theme::CREAM };
                    let mut line = row![
                        text(format!("{indent}{glyph}{}", row.name)).size(15).color(name_color)
                    ]
                    .spacing(6);
                    if let Some(st) = status {
                        line = line.push(iced_widget::horizontal_space());
                        line = line.push(text("●").size(8).color(tree_row_dot(st)));
                    }
                    let msg = if row.is_dir {
                        Message::ProjectTreeToggle(row.path.clone())
                    } else {
                        Message::PreviewOpenPath(row.path.clone())
                    };
                    content = content.push(
                        button(line)
                            .on_press(msg)
                            .width(Length::Fill)
                            .style(|_t, _s| button::Style {
                                background: None,
                                text_color: theme::BODY,
                                ..button::Style::default()
                            }),
                    );
                }
```
删除现已无用的 `decoration_for` 里"后缀字符"用途？——`decoration_for` 仍被终端/其它处引用则**保留**；仅本循环不再用它。若编译器报 `decoration_for` 未使用（unused）再删，否则留。

- [ ] **Step 6: 编译 + 测试 + clippy + fmt**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 全绿 / clean / 无输出。若 clippy 报 `decoration_for` dead_code，确认无他处引用后删除该函数及其测试。

- [ ] **Step 7: 真机目测**

Run: 改本仓某文件（不 commit）→ `target/aarch64-apple-darwin/debug/dozer` 打开本仓
Expected: 目录带 ▾/▸ 三角、文件带 · 前导；改过的文件/目录行尾出现金色 ● 点，新文件绿点，删除红点。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(P1k树): 文件树字形 + 行尾彩色状态点(金改/绿新/红删),替换文字变色后缀"
```

---

### Task 4: 卡片 / pill 视觉打磨（纯 app 侧）

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（`project_pane` 项目信息包圆角卡；`tab_item` 激活态 pill；交付横幅/agent 卡圆角内边距）

**Interfaces:**
- Consumes: 现有 `tab_item`（`workspace.rs` 内，`idx == ws.active` 已知）、`project_pane` 项目信息区、`ai_pane` agent 卡、交付横幅渲染。
- Produces: 无新纯函数（纯样式）；改动 `tab_item` 的激活分支样式。

- [ ] **Step 1: 项目信息包圆角卡**（`project_pane` 的 `Some(p)` 分支，把项目名/分支/路径三行包进一个 CARD 容器再 push）

将现有：
```rust
            content = content.push(text(p.name.clone()).size(15).color(theme::CREAM));
            let label = project_branch_label(ws.branch.as_deref(), ws.dirty);
            let bcolor = if ws.dirty { theme::GOLD } else { theme::BODY };
            content = content.push(text(label).size(12).color(bcolor));
            content = content.push(text(p.path.clone()).size(11).color(theme::DIM));
            content = content.push(open_btn);
```
改为：
```rust
            let label = project_branch_label(ws.branch.as_deref(), ws.dirty);
            let bcolor = if ws.dirty { theme::GOLD } else { theme::BODY };
            let card = container(
                column![
                    text(p.name.clone()).size(15).color(theme::CREAM),
                    text(label).size(12).color(bcolor),
                    text(p.path.clone()).size(11).color(theme::DIM),
                ]
                .spacing(2),
            )
            .width(Length::Fill)
            .padding(10)
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(theme::CARD.into()),
                border: Border { color: theme::BORDER, width: 1.0, radius: 8.0.into() },
                ..container::Style::default()
            });
            content = content.push(card);
            content = content.push(open_btn);
```

- [ ] **Step 2: 激活 tab pill 态**（找到 `tab_item` 函数中区分激活/非激活的 `button::Style`；给激活态加 CARD 底 + 圆角，非激活透明）

在 `tab_item` 里，激活分支的 `button::Style` 改为：
```rust
        button::Style {
            background: Some(theme::CARD.into()),
            text_color: theme::CREAM,
            border: Border { color: theme::BORDER, width: 1.0, radius: 6.0.into() },
            ..button::Style::default()
        }
```
非激活分支：`background: None`、`text_color: theme::BODY`、`border` 透明（width 0）。
（若 `tab_item` 现用闭包按 `active` 分流样式，则在闭包内按 `active` 返回上述两套。保持 `blink_on` 逻辑不变。）

- [ ] **Step 3: 交付横幅 / agent 卡圆角**

交付横幅容器（terminal_pane 内 banner）加 `padding([6,10])` + `radius: 6`；`ai_pane` 的 agent 卡容器加 `radius: 8` + `padding(10)`（若已是 container，仅调 `border.radius`/`padding`；无则包一层 CARD 容器）。颜色不变。

- [ ] **Step 4: 编译 + 测试 + clippy + fmt**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 全绿 / clean / 无输出。

- [ ] **Step 5: 真机目测**

Run: `target/aarch64-apple-darwin/debug/dozer`
Expected: 项目信息成圆角卡；激活 tab 有 CARD 底 pill 高亮、其余透明；交付横幅/agent 卡圆角柔和。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(P1k打磨): 项目卡圆角 + 激活 tab pill 态 + 横幅/agent 卡圆角"
```

---

### Task 5: 验收数 RPC + 项目卡"N 次验收"副行（跨 crate）

**Files:**
- Modify: `crates/dozerd/src/acceptance.rs`（加 `count_for_repo`）
- Modify: `crates/dozer-core/src/protocol.rs`（`Request::GetAcceptanceCount`、`Reply::AcceptanceCount`）
- Modify: `crates/dozerd/src/server.rs`（handler）
- Modify: `crates/dozer-client/src/lib.rs`（`acceptance_count` 方法）
- Modify: `crates/dozer-app/src/workspace.rs`（`project_acceptance_count` 字段 + 载入 + 项目卡副行）

**Interfaces:**
- Consumes: `AcceptanceStore`、`ProjectInfo.path`、Task 4 的项目卡 `column!`。
- Produces: `AcceptanceStore::count_for_repo(&self, repo: &str) -> Result<u64>`；`Request::GetAcceptanceCount { repo: String }`；`Reply::AcceptanceCount { count: u64 }`；`Client::acceptance_count(&self, repo: &str) -> Result<u64>`；`Workspace.project_acceptance_count: Option<u64>`。

- [ ] **Step 1: 写 `count_for_repo` 失败测试**（`acceptance.rs` 的 `mod tests`，扩展 `open_record_count_roundtrip` 之后新增）

```rust
    #[test]
    fn count_for_repo_filters_by_repo() {
        let dir = tempfile::tempdir().unwrap();
        let store = AcceptanceStore::open(&dir.path().join("t.db")).unwrap();
        let mk = |repo: &str, n: u64| AcceptanceRecord {
            repo: repo.into(), goal: "g".into(), criteria_checked: vec![],
            verdict: "accepted".into(), comment: "".into(),
            ref_name: format!("refs/dozer/accepted/{n}"), acceptor: "user".into(), ts_ms: n,
        };
        store.record(&mk("/a", 1)).unwrap();
        store.record(&mk("/a", 2)).unwrap();
        store.record(&mk("/b", 3)).unwrap();
        assert_eq!(store.count_for_repo("/a").unwrap(), 2);
        assert_eq!(store.count_for_repo("/b").unwrap(), 1);
        assert_eq!(store.count_for_repo("/none").unwrap(), 0);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozerd count_for_repo_filters_by_repo`
Expected: FAIL —— `no method named count_for_repo`。

- [ ] **Step 3: 实现 `count_for_repo`**（`acceptance.rs`，`count` 之后）

```rust
    pub fn count_for_repo(&self, repo: &str) -> Result<u64> {
        let n: u64 = self.conn.lock().expect("db lock").query_row(
            "SELECT COUNT(*) FROM acceptances WHERE repo = ?1",
            [repo],
            |row| row.get(0),
        )?;
        Ok(n)
    }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozerd count_for_repo_filters_by_repo`
Expected: PASS。

- [ ] **Step 5: 加协议请求/应答 + roundtrip 测试**

`protocol.rs` `enum Request` 末尾（`GetActiveProject,` 后）加：
```rust
    /// 取某仓库的验收次数（项目卡"N 次验收"用）。
    GetAcceptanceCount {
        repo: String,
    },
```
`enum Reply` 末尾（`Project { ... }` 后）加：
```rust
    /// 验收次数。
    AcceptanceCount {
        count: u64,
    },
```
`mod tests` 加：
```rust
    #[test]
    fn acceptance_count_request_roundtrips() {
        let req = Request::GetAcceptanceCount { repo: "/r".into() };
        let back: Request = decode_line(&encode_line(&req)).unwrap();
        assert_eq!(back, req);
        let rep = Reply::AcceptanceCount { count: 12 };
        let back: Reply = decode_line(&encode_line(&rep)).unwrap();
        assert_eq!(back, rep);
    }
```

- [ ] **Step 6: 跑协议测试**

Run: `cargo test -p dozer-core acceptance_count_request_roundtrips`
Expected: PASS。

- [ ] **Step 7: dozerd handler**（`server.rs` 的 `match req { ... }`，参照 `Request::ListProjects` 分支加一支）

```rust
                        Request::GetAcceptanceCount { repo } => {
                            let reply = match store.count_for_repo(&repo) {
                                Ok(count) => Reply::AcceptanceCount { count },
                                Err(e) => Reply::Error { message: format!("验收计数失败: {e}") },
                            };
                            reply
                        }
```
（`store` 是 handler 作用域内的 `AcceptanceStore` 句柄；命名对齐该文件里 `RecordAcceptance` 分支所用的同一变量。若分支体是"求值出 `Reply` 再统一写回"，按邻近分支写法收敛；若是"就地 `write` 应答"，照邻近分支就地写。）

- [ ] **Step 8: client 方法**（`dozer-client/src/lib.rs`，参照 `active_project` 加）

```rust
    pub async fn acceptance_count(&self, repo: &str) -> Result<u64> {
        match self
            .roundtrip(&Request::GetAcceptanceCount { repo: repo.into() })
            .await?
        {
            Reply::AcceptanceCount { count } => Ok(count),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => Err(anyhow::anyhow!("非预期应答: {other:?}")),
        }
    }
```

- [ ] **Step 9: workspace 载入 + 渲染副行**

`Workspace` 加字段 `project_acceptance_count: Option<u64>,`；两构造处初始化 `None`。
加异步载入（仿 `spawn_conversations_refresh`）：
```rust
    /// 异步取当前项目验收次数 → AcceptanceCountLoaded（项目卡副行）。
    fn spawn_acceptance_count_refresh(&self) {
        let Some(p) = &self.project else { return };
        let repo = p.path.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        self.handle.spawn(async move {
            let n = client.acceptance_count(&repo).await.ok();
            let _ = proxy.send_event(Message::AcceptanceCountLoaded(n));
        });
    }
```
加 `Message::AcceptanceCountLoaded(Option<u64>)` 变体及处理：
```rust
            Message::AcceptanceCountLoaded(n) => {
                self.project_acceptance_count = n;
            }
```
在 `ProjectOpened`（`self.project = project;` 后，紧跟 goal 载入）和 `RecordAcceptance` 成功落地处调用 `self.spawn_acceptance_count_refresh();`；`ProjectOpened` 里先 `self.project_acceptance_count = None;`。
把 Task 4 的项目卡内容改为可变 `column`，取到验收数才条件 push 金色副行（取不到/为 0 皆不显，不做无意义占位）：
```rust
            let mut card_col = column![
                text(p.name.clone()).size(15).color(theme::CREAM),
                text(label).size(12).color(bcolor),
                text(p.path.clone()).size(11).color(theme::DIM),
            ]
            .spacing(2);
            if let Some(n) = ws.project_acceptance_count.filter(|n| *n > 0) {
                card_col = card_col.push(text(format!("{n} 次验收")).size(11).color(theme::GOLD));
            }
            let card = container(card_col)
                .width(Length::Fill)
                .padding(10)
                .style(|_t: &iced_widget::Theme| container::Style {
                    background: Some(theme::CARD.into()),
                    border: Border { color: theme::BORDER, width: 1.0, radius: 8.0.into() },
                    ..container::Style::default()
                });
            content = content.push(card);
```

- [ ] **Step 10: 全量测试 + clippy + fmt（四 crate）**

Run: `cargo test --workspace && cargo clippy --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt --all -- --check`
Expected: 全绿 / clean / 无输出。

- [ ] **Step 11: 真机目测**

Run: `target/aarch64-apple-darwin/debug/dozer`（打开有验收记录的项目，如本仓 dozer）
Expected: 项目卡出现金色"N 次验收"副行（N 为该仓真实验收数）；无验收记录的项目不显该行。

- [ ] **Step 12: Commit**

```bash
git add crates/dozerd/src/acceptance.rs crates/dozer-core/src/protocol.rs crates/dozerd/src/server.rs crates/dozer-client/src/lib.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(P1k验收数): count_for_repo + GetAcceptanceCount RPC + 项目卡 N 次验收副行"
```

---

## 收尾（全 task 完成后）

- [ ] **回归 + 人工验收**：全量 `cargo test --workspace` 绿、clippy/fmt 干净；真机对 Figma S1 逐区域目测比对（顶栏/状态栏/树/卡片），逐项 ✓/✗ 记录到 `docs/superpowers/specs/2026-07-20-p1k-acceptance.md`。验收权归用户,实施方不代签。
- [ ] **落档**：验收通过后回填规格 §7（顶栏/状态栏落地标注），勾选本计划全部 box。
- [ ] **分支收尾**：`superpowers:finishing-a-development-branch` 合入 main。

## 自检记录（写计划时）

- **Spec 覆盖**：§3 Task1→本 Task1；§3 Task2→Task2；§3 Task3→Task3；§3 Task4 拆为 Task4(纯样式)+Task5(验收数 RPC)；§4 错误降级散落各 task（无 goal/断连/取不到数各有分支）；§5 测试策略→各 task 纯函数测试 + 协议 roundtrip + count 测试。无遗漏。
- **占位扫描**：无 TBD/TODO；视觉占位（搜索/齿轮/组件）为规格明确决策，非计划占位。
- **类型一致**：`goal_capsule_text`/`agent_state_label`/`env_status_text`/`tree_row_glyph`/`tree_row_dot`/`count_for_repo`/`GetAcceptanceCount`/`AcceptanceCount`/`acceptance_count`/`project_goal`/`project_acceptance_count` 各处签名一致；`Goal{title,criteria}` 与 `goal.rs` 一致；`AgentState` 四变体全覆盖。
