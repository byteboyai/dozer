# 菜单/确认弹窗公共代码提取 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 消除两类已验证的重复代码——(1) 同一个菜单的内容在 macOS 原生 `NSMenu`(`chrome::native_menu::Item<Msg>`)和 iced 弹层(`crate::chrome::menu`)两套渲染后端里各写一遍；(2) 至少 4 处"标题 + 说明 + 取消/确认按钮"确认弹窗的组装骨架手写重复。提取共享数据结构和辅助函数，行为保持不变（除两处顺带修复的 bug：Task 3 文案不一致、Task 4 iced 侧双分隔线）。

**Architecture:** 新增两个平台无关的小模块：`crates/dozer-app/src/menu_spec.rs`(菜单内容的中间表示 + 两个转换器，喂给 native/iced 两套渲染)、`crates/dozer-app/src/dialog.rs` 内新增 `confirm()` 函数(复用已有的 `card_style`/`actions`/`action_button_style` 原语，只补"标题+说明+两按钮"这层组装)。每个具体菜单/弹窗各自一个 Task，逐个把手写的重复代码改成调用共享函数，输出类型不变，旧单测原样保留作回归网。`crate::menu`/`crate::native_menu`/`topbar.rs` 等已随 `2026-09-17-app-module-reorganization.md` 迁入 `chrome/` 目录(下文路径均已按此更新)。

**Tech Stack:** Rust workspace；iced 0.14；`#[cfg(target_os = "macos")]` 条件编译（`native_menu` 模块整体只在 mac 编译）。

**Spec:** 无独立 spec。这是 `docs/superpowers/plans/2026-09-17-app-module-reorganization.md`(文件目录重组计划)评审过程中发现、但因为"改函数体逻辑"跟那份计划"纯重构、零行为变化"的约束冲突而被拆出来的独立小计划。**重组计划已合并进 `main`**(Phase 4.1–4.5e 均已落地，git log 里可见，最新为 0.5.151 release)——本计划所有文件路径/模块前缀/行号均已按重组后的布局核对更新(`menu.rs`/`native_menu.rs`/`topbar.rs` → `chrome/`，`workspace.rs` → `workspace/view.rs`，`extensions/{files,todo,database}.rs` → `extensions/{files,todo,database}/view.rs`)，执行时直接以当前 `main` 为基线开分支即可，无需再等另一份计划。

## Global Constraints

- **在独立分支 `refactor/menu-dialog-dedup` 上开发**——`refactor/module-reorganization` 已合并，本计划直接以当前 `main` 为基线开分支，不要复用旧的重组分支名。本计划会碰到 `chrome/topbar.rs`、`chrome/menu.rs`、`workspace/view.rs`、`extensions/files/view.rs`、`extensions/ssh.rs`、`extensions/todo/view.rs`、`extensions/database/view.rs`。
- **不是纯重构，是行为等价的重构**：每个 Task 迁移前后，函数的**输出类型不变**（`Vec<native_menu::Item<Message>>`/`Element<Message>` 保持原样），旧单测原样保留且必须继续通过——这是判断"没改坏行为"的第一道网。iced 渲染部分因为无法在单测里断言 `Element` 内部结构，收尾统一靠 `cargo run -p dozer-app` 人工核对视觉（每个 Task 都有明确的人工核对步骤，不能跳过）。
- **已验证适用的菜单/弹窗清单，不要在此基础上自行扩大范围**：本计划只处理下面 Task 列出的 3 个菜单对 + 4 个确认弹窗，均已逐个读过源码确认形状吻合。以下三个明确**不**适用、已排除，执行时不要顺手"顺便也做了"：
  - `extensions/files/view.rs::move_confirm_popup`（带两个文本输入框 + 浏览按钮，是表单不是简单确认框）
  - `extensions/project/view.rs::project_delete_confirm_popup`（带单选按钮组选删除范围，不是简单确认框）
  - `extensions/todo/view.rs::dispatch_items`/`status_items` 及其对应的 iced 弹层、`extensions/ssh/sftp.rs::context_menu_items`（native 版已确认存在，对应的 iced fallback 版本本计划没有逐一读过源码核实是否真的重复，不在本计划范围内，需要的话另开 Task 先核实）
- 每个 Task 结束都要 `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过。
- **参数结构体**：`dialog::confirm()` 的参数按 CLAUDE.md 关键裁决"≥7 个参数且有相邻同类型参数用具名字段结构体"处理——`title`/`description` 两个相邻 `impl Into<String>` 传错顺序编译器发现不了，Task 5 直接设计成 `ConfirmDialog<Msg>` 结构体，不要写成一长串位置参数。

---

## 现状证据（写这份计划前实测，供执行者复核）

### 类别一：native/iced 菜单内容重复

| 菜单 | native 版（mac，`chrome::native_menu::Item<Message>`） | iced 版（非 mac fallback） | 已确认的重复/不一致 |
|---|---|---|---|
| Agent 选择器 | `workspace/view.rs:278 agent_picker_items()` | `workspace/view.rs:320 agent_picker_popup()` | 六个 agent + Git Shell + 纯 Shell 列表，图标/颜色/文案独立写两遍 |
| 顶栏"＋新增项目" | `chrome/topbar.rs:411 project_add_menu_items()` | `chrome/topbar.rs:454 project_add_menu_popup()` | 同一段 `filter`+`sort_by_key(Reverse(updated_ms))` 逻辑写两遍；**且菜单文案与按钮行为不一致**：native/iced 两版菜单最后一项都写"新建项目"，而顶栏"＋"按钮自己的 tooltip 写"打开项目"，触发的都是 `Message::ProjectTabPickFolder`（rfd 文件夹选择器，语义是"打开一个已有文件夹当项目"）——"新建项目"是错的，Task 3 顺带统一改成"打开项目"。（2026-09-17 审核时按 HEAD 源码复核更正：本行原先写"native 版写'新建项目'，iced 版写'打开项目'"不准确——两版菜单文案相同，不一致的是菜单项 vs 按钮 tooltip） |
| 文件树右键菜单 | `extensions/files/view.rs:784 context_menu_items()` | `extensions/files/view.rs:873 context_menu_popup()` | 11 个条件分支（是否目录/是否根/是否有剪贴板内容）逐条写两遍，函数自己的文档注释（view.rs:779）已经承认"和旧版共用完全相同的条件分支……行为上二者应保持一致"——这条本计划直接兑现 |

### 类别二：确认弹窗骨架重复

已确认形状吻合（标题 + 说明文字 + 取消/确认两个按钮，均已共用 `crate::dialog::actions`/`action_button_style`/`card_style`/`width` 底层原语，只是"标题+说明+按钮行"这层组装每处手写一遍）：

| 弹窗 | 位置 | 差异点 |
|---|---|---|
| 文件删除确认 | `extensions/files/view.rs:1000 delete_confirm_popup` | 标题纯文字，无图标 |
| SSH 主机删除确认 | `extensions/ssh.rs:1364 delete_confirm_popup` | 标题纯文字，无图标 |
| Todo 清空列表确认 | `extensions/todo/view.rs:430 clear_confirm_popup` | 标题带一个 `ListTodo` 图标（`row![icon, text]`）；列间距用 `.spacing(12)`（其余三处是 8，Task 8 用 `content_spacing` 字段原样保留） |
| 数据源删除确认 | `extensions/database/view.rs:580 delete_confirm_popup` | 标题纯文字，无图标 |

`dialog.rs` 模块自己的头部文档注释（1-31 行）已经写明"之前各面板各写一份……视觉与'弹窗该有的分量感'逐处不一致，现在把外壳原语收拢成共享的两件套"——本计划是这个既定方向的下一步，不是新方向。

---

## Phase A：菜单内容去重

### Task 1：`menu_spec.rs` —— 平台无关的菜单内容中间表示

**Files:**
- Create: `crates/dozer-app/src/menu_spec.rs`
- Modify: `crates/dozer-app/src/main.rs`（加 `mod menu_spec;`）
- Modify: `crates/dozer-app/src/chrome/menu.rs:118`（`icon_leading` 从私有改 `pub(crate)`，供 `menu_spec::to_iced` 复用，避免在 `menu_spec.rs` 里重复一遍图标转元素的逻辑）

**Interfaces:**
- Produces：
  - `pub enum MenuSpecItem<Msg> { Entry { icon: Option<IconKind>, icon_color: Option<Color>, label: String, color: Color, enabled: bool, msg: Msg }, Separator }`
  - `pub type MenuSpec<Msg> = Vec<MenuSpecItem<Msg>>;`
  - `impl<Msg> MenuSpecItem<Msg> { pub fn entry(...) -> Self; pub fn entry_tinted(...) -> Self; pub fn separator() -> Self; }`
  - `#[cfg(target_os = "macos")] pub fn to_native<Msg>(spec: MenuSpec<Msg>) -> Vec<crate::chrome::native_menu::Item<Msg>>`
  - `pub fn to_iced<'a, Msg: 'a + Clone>(spec: MenuSpec<Msg>, width: iced_widget::core::Length) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer>`

- [ ] **Step 1：写 `menu_spec.rs`**

```rust
// crates/dozer-app/src/menu_spec.rs
//! 平台无关的菜单内容中间表示。`native_menu`(mac 专属 NSMenu)和
//! `crate::chrome::menu`(iced 弹层，非 mac fallback)如果各自独立组装同一份
//! 菜单数据，容易踩文案/行为漂移的坑——2026-09-17 迁移顶栏"＋新增项目"
//! 菜单时发现，最后一项在 native/iced 两版都写"新建项目"，而"＋"按钮
//! 自己的 tooltip 写"打开项目"，它们触发的其实是同一个
//! `Message::ProjectTabPickFolder`（rfd 文件夹选择器，语义是"打开已有
//! 目录"不是"新建"），菜单文案已统一改成"打开项目"。这类漂移正是同一份
//! 数据手写多遍的产物：菜单内容只在这里组装一次，两个渲染后端各自从
//! 同一份 `MenuSpec` 转换消费。

use byteui::interaction::icons::IconKind;
use iced_widget::core::{Color, Element, Length};

/// 一条菜单内容——跟 `native_menu::Item` 字段完全对应，但不依赖
/// `native_menu` 模块（那个模块整体 `#[cfg(target_os = "macos")]`），
/// 因此这个类型能在所有平台编译，`to_iced` 才能在非 mac 平台使用。
#[derive(Debug, Clone, PartialEq)]
pub enum MenuSpecItem<Msg> {
    Entry {
        icon: Option<IconKind>,
        icon_color: Option<Color>,
        label: String,
        color: Color,
        enabled: bool,
        msg: Msg,
    },
    Separator,
}

/// 一整条菜单的内容。
pub type MenuSpec<Msg> = Vec<MenuSpecItem<Msg>>;

impl<Msg> MenuSpecItem<Msg> {
    /// 常规可点项：图标/文字都用主题 BODY 色。
    pub fn entry(icon: Option<IconKind>, label: impl Into<String>, msg: Msg) -> Self {
        let body = byteui::theme::color::current().body;
        Self::Entry {
            icon,
            icon_color: None,
            label: label.into(),
            color: body,
            enabled: true,
            msg,
        }
    }

    /// 图标独立着色项（agent 选择器等双色菜单用）：文字仍用 BODY 色。
    pub fn entry_tinted(
        icon: IconKind,
        icon_color: Color,
        label: impl Into<String>,
        msg: Msg,
    ) -> Self {
        let body = byteui::theme::color::current().body;
        Self::Entry {
            icon: Some(icon),
            icon_color: Some(icon_color),
            label: label.into(),
            color: body,
            enabled: true,
            msg,
        }
    }

    /// 分组分隔线。
    pub fn separator() -> Self {
        Self::Separator
    }
}

/// 转成 `native_menu::show` 要的原生菜单条目——纯数据搬运，字段一一对应。
#[cfg(target_os = "macos")]
pub fn to_native<Msg>(spec: MenuSpec<Msg>) -> Vec<crate::chrome::native_menu::Item<Msg>> {
    spec.into_iter()
        .map(|item| match item {
            MenuSpecItem::Entry {
                icon,
                icon_color,
                label,
                color,
                enabled,
                msg,
            } => crate::chrome::native_menu::Item::Entry {
                icon,
                icon_color,
                label,
                color,
                enabled,
                msg,
            },
            MenuSpecItem::Separator => crate::chrome::native_menu::Item::Separator,
        })
        .collect()
}

/// 转成 `crate::chrome::menu` 的 iced 弹层——非 mac 平台的 fallback 渲染路径。
/// `enabled: false` 的项走 `menu::item_locked`（不挂 `on_press`，`msg`
/// 字段被丢弃，语义同 native 侧"禁用项点不动"）；`enabled: true` 走
/// `menu::item_row`。图标着色跟 `native_menu::show` 内部同一条规则：
/// `icon_color` 缺省时跟随文字 `color`。
pub fn to_iced<'a, Msg: 'a + Clone>(
    spec: MenuSpec<Msg>,
    width: Length,
) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    let items = spec
        .into_iter()
        .map(|item| match item {
            MenuSpecItem::Entry {
                icon,
                icon_color,
                label,
                color,
                enabled,
                msg,
            } => {
                let tint = icon_color.unwrap_or(color);
                if enabled {
                    crate::chrome::menu::item_row(crate::chrome::menu::icon_leading(icon, tint), label, color, Some(msg))
                } else {
                    crate::chrome::menu::item_locked(icon, label, color)
                }
            }
            MenuSpecItem::Separator => crate::chrome::menu::separator(),
        })
        .collect();
    crate::chrome::menu::shell_frosted(items, width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    enum TestMsg {
        A,
        B,
    }

    #[test]
    fn entry_defaults_to_body_color_and_enabled() {
        let item = MenuSpecItem::entry(None, "标签", TestMsg::A);
        assert!(matches!(
            item,
            MenuSpecItem::Entry {
                icon: None,
                icon_color: None,
                enabled: true,
                msg: TestMsg::A,
                ..
            }
        ));
    }

    #[test]
    fn entry_tinted_sets_icon_color_independent_of_text_color() {
        let red = Color::from_rgb(1.0, 0.0, 0.0);
        let item = MenuSpecItem::entry_tinted(IconKind::Trash, red, "删除", TestMsg::B);
        match item {
            MenuSpecItem::Entry {
                icon: Some(IconKind::Trash),
                icon_color: Some(c),
                color,
                ..
            } => {
                assert_eq!(c, red);
                assert_eq!(color, byteui::theme::color::current().body);
            }
            _ => panic!("expected tinted entry"),
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn to_native_preserves_order_and_field_values() {
        let spec: MenuSpec<TestMsg> = vec![
            MenuSpecItem::entry(None, "第一项", TestMsg::A),
            MenuSpecItem::separator(),
            MenuSpecItem::entry(None, "第二项", TestMsg::B),
        ];
        let items = to_native(spec);
        assert_eq!(items.len(), 3);
        assert!(matches!(items[1], crate::chrome::native_menu::Item::Separator));
        assert!(matches!(
            items[2],
            crate::chrome::native_menu::Item::Entry {
                msg: TestMsg::B,
                ..
            }
        ));
    }
}
```

- [ ] **Step 2：`chrome/menu.rs:118` 的 `icon_leading` 改可见性**

把：
```rust
fn icon_leading<'a, Msg: 'a>(
```
改成：
```rust
pub(crate) fn icon_leading<'a, Msg: 'a>(
```

- [ ] **Step 3：`main.rs` 里加 `mod menu_spec;`**

找到现有 `mod menu;` 那一行（`main.rs` 顶部 `mod` 声明区），紧挨着加一行 `mod menu_spec;`。

- [ ] **Step 4：跑测试**

```
cargo test -p dozer-app menu_spec
```
Expected：4 个新测试全部 PASS（`to_native_preserves_order_and_field_values` 只在 mac 上跑，非 mac 平台这个测试被 `#[cfg(target_os = "macos")]` 排除，属预期）。

- [ ] **Step 5：`cargo build -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过，然后 commit**

```bash
git add crates/dozer-app/src/menu_spec.rs crates/dozer-app/src/menu.rs crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): 新增 menu_spec 平台无关菜单中间表示"
```

---

### Task 2：迁移 Agent 选择器菜单（`workspace/view.rs`）

**Files:**
- Modify: `crates/dozer-app/src/workspace/view.rs:278-393`（`agent_picker_items`/`agent_picker_popup` 及其间的共享数据）

**Interfaces:**
- Consumes：`menu_spec::{MenuSpec, MenuSpecItem, to_native, to_iced}`（Task 1）
- Produces：`pub(crate) fn agent_picker_spec() -> MenuSpec<Message>`（新函数，`agent_picker_items`/`agent_picker_popup` 内部改调用它）

- [ ] **Step 1：读现有测试，确认迁移后必须仍然通过**

`workspace/tests.rs` 现有测试（约 1045-1072 行）：
```rust
#[cfg(target_os = "macos")]
#[test]
fn agent_picker_items_has_nine_rows_matching_old_picker() {
    // 六个 agent + 1 条分隔线 + 两个 shell(Git Shell/纯 Shell)= 9 行,
    assert_eq!(agent_picker_items().len(), 9);
}

#[cfg(target_os = "macos")]
#[test]
fn agent_picker_items_separator_splits_agents_from_shells() {
    let items = agent_picker_items();
    // ...（断言下标 6 是分隔线）
}
```
这两个测试**不改**，迁移后必须原样通过——它们是这次重构"没改坏行为"的验收标准。

- [ ] **Step 2：写新函数 `agent_picker_spec()`**

在 `workspace/view.rs` 里 `agent_picker_items`（278 行）和 `agent_picker_popup`（320 行）之间，插入：

```rust
/// Agent 选择器菜单内容——native(`agent_picker_items`)和 iced fallback
/// (`agent_picker_popup`)共用同一份数据，只在这里组装一次。
pub(crate) fn agent_picker_spec() -> MenuSpec<Message> {
    let agents: [(&str, PickerLaunch); 6] = [
        ("Claude", PickerLaunch::Agent(Some(AgentKind::Claude))),
        ("CodeBuddy", PickerLaunch::Agent(Some(AgentKind::Codebuddy))),
        ("Codex", PickerLaunch::Agent(Some(AgentKind::Codex))),
        ("Kilo", PickerLaunch::Agent(Some(AgentKind::Kilo))),
        ("OpenCode", PickerLaunch::Agent(Some(AgentKind::Opencode))),
        ("v8agent", PickerLaunch::Agent(Some(AgentKind::V8agent))),
    ];
    let shells: [(&str, PickerLaunch); 2] = [
        ("Git Shell", PickerLaunch::Git),
        ("纯 Shell", PickerLaunch::Agent(None)),
    ];
    let mk = |label: &str, launch: PickerLaunch| {
        let (icon, icon_color) = match launch {
            PickerLaunch::Agent(Some(kind)) => (agent_icon(kind), agent_dot_color(kind)),
            PickerLaunch::Agent(None) => (IconKind::Terminal, byteui::theme::color::current().body),
            PickerLaunch::Git => (IconKind::GitBranch, byteui::theme::color::current().body),
        };
        MenuSpecItem::entry_tinted(icon, icon_color, label, Message::AgentPickerSelect(launch))
    };
    let mut spec: MenuSpec<Message> = agents
        .into_iter()
        .map(|(label, launch)| mk(label, launch))
        .collect();
    spec.push(MenuSpecItem::separator());
    spec.extend(shells.into_iter().map(|(label, launch)| mk(label, launch)));
    spec
}
```

- [ ] **Step 3：`agent_picker_items` 改成调用 `agent_picker_spec`**

把（278-310 行）整个函数体替换为：
```rust
pub(crate) fn agent_picker_items() -> Vec<crate::chrome::native_menu::Item<Message>> {
    crate::menu_spec::to_native(agent_picker_spec())
}
```

- [ ] **Step 4：`agent_picker_popup` 改成调用 `agent_picker_spec`**

把 320-393 行里"组装 `list`"那一段（原本手写 `agents`/`shells`/`mk_item`/`list.push` 一整段，直到 `let list: Element<...> = crate::chrome::menu::shell_frosted(list, ...)` 之前）替换为：
```rust
pub(crate) fn agent_picker_popup(
    ws: &Workspace,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !ws.agent_picker_open {
        return column![].into();
    }
    let list = crate::menu_spec::to_iced(
        agent_picker_spec(),
        Length::Fixed(byteui::theme::geometry::menu_item_width()),
    );
    // 右上角固定偏移:48px 避开顶栏,16px 避开窗口右边缘,原有逻辑不变。
    container(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Right)
        .align_y(iced_widget::core::alignment::Vertical::Top)
        .padding(Padding {
            top: 48.0,
            left: 0.0,
            right: 16.0,
            bottom: 0.0,
        })
        .into()
}
```

- [ ] **Step 5：跑测试确认没改坏行为**

```
cargo test -p dozer-app agent_picker
```
Expected：`agent_picker_items_has_nine_rows_matching_old_picker`、`agent_picker_items_separator_splits_agents_from_shells` 两个旧测试原样 PASS。

- [ ] **Step 6：人工核对（mac 环境）**

`cargo run -p dozer-app`，点开 Agent 面板"＋新建 Agent 会话"，核对弹出的原生菜单：六个 agent 图标/颜色跟改动前截图一致、Git Shell/纯 Shell 在分隔线下方、点击各项行为不变。非 mac 平台改用同样步骤核对 iced 弹层视觉一致（如果没有非 mac 测试环境，至少确认 `cargo build` 在交叉编译目标下能过，不强求实机视觉核对）。

- [ ] **Step 7：`cargo build -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过，然后 commit**

```bash
git add crates/dozer-app/src/workspace/view.rs
git commit -m "refactor(dozer-app): agent 选择器菜单内容改用共享 menu_spec"
```

---

### Task 3：迁移顶栏"＋新增项目"菜单（`chrome/topbar.rs`），顺带修正文案不一致

**Files:**
- Modify: `crates/dozer-app/src/chrome/topbar.rs:408-522`（`project_add_menu_items`/`project_add_menu_popup`）

**Interfaces:**
- Consumes：`menu_spec::{MenuSpec, MenuSpecItem, to_native, to_iced}`（Task 1）
- Produces：`pub(crate) fn project_add_menu_spec(recent_projects: &[ProjectInfo], open_project_ids: &std::collections::HashSet<i64>) -> MenuSpec<Message>`

- [ ] **Step 1：写新函数 `project_add_menu_spec`，统一文案为"打开项目"**

`Message::ProjectTabPickFolder` 触发的是 rfd 文件夹选择器，语义是"打开一个已有目录当项目"，不是"新建"——菜单原文案在 native/iced 两版都写"新建项目"，与"＋"按钮 tooltip"打开项目"不一致，是 bug，这里统一改成"打开项目"（跟按钮 tooltip 和实际行为对齐）。在 `chrome/topbar.rs` 里 `project_add_menu_items`（411 行）之前插入：

```rust
/// 顶栏"＋新增项目"菜单内容——native(`project_add_menu_items`)和 iced
/// fallback(`project_add_menu_popup`)共用同一份数据。2026-09-17 迁移时
/// 发现最后一项在两个渲染后端都写"新建项目",而"＋"按钮自己的 tooltip
/// 写"打开项目"——统一成"打开项目"(跟 `Message::ProjectTabPickFolder`
/// 触发的 rfd 文件夹选择器语义一致——是"打开已有目录"不是"新建")。
pub(crate) fn project_add_menu_spec(
    recent_projects: &[ProjectInfo],
    open_project_ids: &std::collections::HashSet<i64>,
) -> MenuSpec<Message> {
    let mut projects: Vec<&ProjectInfo> = recent_projects
        .iter()
        .filter(|p| !open_project_ids.contains(&p.id))
        .collect();
    projects.sort_by_key(|p| std::cmp::Reverse(p.updated_ms));
    let mut spec: MenuSpec<Message> = projects
        .into_iter()
        .map(|p| MenuSpecItem::entry(None, p.name.clone(), Message::ProjectSelect(p.id)))
        .collect();
    if !spec.is_empty() {
        spec.push(MenuSpecItem::separator());
    }
    spec.push(MenuSpecItem::entry(
        Some(icons::IconKind::SquarePlus),
        "打开项目",
        Message::ProjectTabPickFolder,
    ));
    spec
}
```

- [ ] **Step 2：`project_add_menu_items` 改成调用共享函数**

把（411-434 行）函数体替换为：
```rust
#[cfg(target_os = "macos")]
pub(crate) fn project_add_menu_items(
    recent_projects: &[ProjectInfo],
    open_project_ids: &std::collections::HashSet<i64>,
) -> Vec<crate::chrome::native_menu::Item<Message>> {
    crate::menu_spec::to_native(project_add_menu_spec(recent_projects, open_project_ids))
}
```

- [ ] **Step 3：`project_add_menu_popup` 改成调用共享函数**

把（454 行起）函数体里"组装 `items`/`list`"那段（从 `let mut projects: Vec<&ProjectInfo> = ...` 到 `let list: Element<...> = crate::chrome::menu::shell_frosted(items, ...)` 之前）替换为：
```rust
pub(crate) fn project_add_menu_popup(
    app: &App,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !app.project_add_menu_open {
        return column![].into();
    }
    let open_ids: std::collections::HashSet<i64> = app.projects.keys().copied().collect();
    let spec = project_add_menu_spec(&app.recent_projects, &open_ids);
    let row_count = spec.len();
    let list = crate::menu_spec::to_iced(
        spec,
        Length::Fixed(byteui::theme::geometry::menu_item_width()),
    );
    // 窗口边界钳制部分不变，仍需要 row_count 估算高度。
    let (ax, ay) = app.project_add_menu_anchor;
    let (window_w, window_h) = app.window_size;
    let pop_w = byteui::theme::geometry::menu_item_width();
    let region = theme::region::context_menu();
    let item_h = region.padding.top + region.padding.bottom + byteui::theme::font::body() as f32;
    let pop_h = region.padding.top
        + region.padding.bottom
        + row_count as f32 * item_h
        + (row_count.saturating_sub(1)) as f32 * region.gap;
    let x = ax.min((window_w - pop_w).max(0.0));
    let y = ay.min((window_h - pop_h).max(0.0));

    container(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: y,
            left: x,
            right: 0.0,
            bottom: 0.0,
        })
        .into()
}
```

注意：原 `project_add_menu_popup` 用的过滤条件是 `!app.projects.contains_key(&p.id)`（`app.projects` 是 `HashMap<i64, _>`），而原生版用的是调用方传入的 `open_project_ids: &HashSet<i64>`（构造方式见 Step 4）——迁移后统一走 `project_add_menu_spec` 接收的 `open_project_ids` 参数，`project_add_menu_popup` 内部现造一份 `open_ids`（`app.projects.keys().copied().collect()`），跟原来的 `contains_key` 判断等价，不改变过滤结果。

- [ ] **Step 4：跑测试**

```
cargo test -p dozer-app project_add_menu
```
Expected：`chrome/topbar.rs` 里已有 4 个针对 `project_add_menu_items` 的单测（`project_add_menu_items_excludes_already_open_projects`、`_sorts_recent_projects_by_updated_ms_desc`、`_always_ends_with_open_project_action`、`_no_separator_when_no_recent_projects`，见 `chrome/topbar.rs:783-830`），它们全部只断言 `msg`/数量/分隔线，**不**断言"新建项目"这个 label 文字——所以 Task 3 改文案为"打开项目"**不会**破坏任何现有测试，这 4 个必须原样 PASS（其中 `_always_ends_with_open_project_action` 断的是 `Message::ProjectTabPickFolder`，跟 label 无关）。本 Task 不新增测试（`project_add_menu_spec` 的过滤/排序逻辑已被 `agent_picker_spec` 同款模式在 Task 2 验证过）。

- [ ] **Step 5：人工核对**

`cargo run -p dozer-app`，点顶栏"＋"新增项目按钮，确认：最近项目列表正确（已打开的项目不出现在列表里）、排序仍按 `updated_ms` 降序、最后一项文案是"打开项目"（不再是"新建项目"）、点击行为不变。

- [ ] **Step 6：`cargo build -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过，然后 commit**

```bash
git add crates/dozer-app/src/chrome/topbar.rs
git commit -m "refactor(dozer-app): 顶栏新增项目菜单改用共享 menu_spec，修正文案不一致"
```

---

### Task 4：迁移文件树右键菜单（`extensions/files/view.rs`）

这是收益最大的一个——11 个条件分支目前逐条手写两遍。

**Files:**
- Modify: `crates/dozer-app/src/extensions/files/view.rs:784-989`（`context_menu_items`/`context_menu_popup`）

**Interfaces:**
- Consumes：`menu_spec::{MenuSpec, MenuSpecItem, to_native, to_iced}`（Task 1）
- Produces：`pub(crate) fn context_menu_spec(target: &Path, is_dir: bool, is_root: bool, has_clipboard: bool) -> MenuSpec<Message>`

- [ ] **Step 1：确认现有测试覆盖，迁移后必须原样通过**

`extensions/files/mod.rs` 现有测试（约 1635-1718 行）：
```rust
#[test]
fn context_menu_items_hides_delete_rename_for_root() { .. }
#[test]
fn context_menu_items_shows_delete_rename_for_non_root() { .. }
#[test]
fn context_menu_items_paste_locked_when_clipboard_empty() { .. }
#[test]
fn context_menu_items_paste_enabled_when_clipboard_has_content() { .. }
#[test]
fn context_menu_items_file_target_has_no_new_file_or_paste() { .. }
```
这五个测试全部调用 `context_menu_items(...)`，迁移后函数签名/返回类型不变，测试**不改**，必须原样通过。

- [ ] **Step 2：写新函数 `context_menu_spec`**

把 `context_menu_items`（files/view.rs:784-867 行）现有函数体**原样搬进**新函数，只做两处替换：`Item::entry(...)` → `MenuSpecItem::entry(...)`、`Item::separator()` → `MenuSpecItem::separator()`、手写的 `Item::Entry { .. }` 字面量（粘贴那一条，files/view.rs:820-831 行）→ `MenuSpecItem::Entry { .. }` 字面量（字段完全一致，只换类型名）。返回类型从 `Vec<crate::chrome::native_menu::Item<Message>>` 改成 `MenuSpec<Message>`：

```rust
/// 文件树右键菜单内容——native(`context_menu_items`)和 iced fallback
/// (`context_menu_popup`)共用同一份数据，11 个条件分支只写一遍。
pub(crate) fn context_menu_spec(
    target: &Path,
    is_dir: bool,
    is_root: bool,
    has_clipboard: bool,
) -> MenuSpec<Message> {
    let dim = byteui::theme::color::current().dim;
    let target = target.to_path_buf();
    let mut items = vec![
        MenuSpecItem::entry(
            Some(icons::IconKind::Search),
            "搜索",
            Message::OpenSearch(target.clone(), is_dir),
        ),
        MenuSpecItem::separator(),
    ];
    if is_dir {
        items.push(MenuSpecItem::entry(
            Some(icons::IconKind::FilePlus),
            "新建文件",
            Message::NewFile(target.clone()),
        ));
        items.push(MenuSpecItem::entry(
            Some(icons::IconKind::FolderPlus),
            "新建文件夹",
            Message::NewFolder(target.clone()),
        ));
        items.push(MenuSpecItem::separator());
    }
    items.push(MenuSpecItem::entry(
        Some(icons::IconKind::Copy),
        "复制",
        Message::Copy(target.clone(), is_dir),
    ));
    if is_dir {
        items.push(MenuSpecItem::Entry {
            icon: Some(icons::IconKind::ClipboardPaste),
            icon_color: None,
            label: "粘贴".into(),
            color: if has_clipboard {
                byteui::theme::color::current().body
            } else {
                dim
            },
            enabled: has_clipboard,
            msg: Message::Paste(target.clone()),
        });
    }
    if !is_root {
        items.push(MenuSpecItem::entry(
            Some(icons::IconKind::Trash),
            "删除",
            Message::DeleteRequest(target.clone(), is_dir),
        ));
        items.push(MenuSpecItem::entry(
            Some(icons::IconKind::Rename),
            "重命名",
            Message::RenameStart(target.clone()),
        ));
    }
    items.push(MenuSpecItem::separator());
    items.push(MenuSpecItem::entry(
        None,
        "复制绝对路径",
        Message::CopyPath(target.clone(), PathKind::Absolute),
    ));
    items.push(MenuSpecItem::entry(
        None,
        "复制相对路径",
        Message::CopyPath(target.clone(), PathKind::Relative),
    ));
    items.push(MenuSpecItem::entry(
        Some(icons::IconKind::FolderOpen),
        "在 Finder 中打开",
        Message::RevealInFinder(target.clone()),
    ));
    items.push(MenuSpecItem::entry(
        Some(icons::IconKind::RefreshCw),
        "从磁盘重新加载",
        Message::ReloadFromDisk,
    ));
    items
}
```

- [ ] **Step 3：`context_menu_items` 改成调用共享函数**

```rust
#[cfg(target_os = "macos")]
pub fn context_menu_items(
    target: &Path,
    is_dir: bool,
    is_root: bool,
    has_clipboard: bool,
) -> Vec<crate::chrome::native_menu::Item<Message>> {
    crate::menu_spec::to_native(context_menu_spec(target, is_dir, is_root, has_clipboard))
}
```

- [ ] **Step 4：跑测试确认没改坏行为**

```
cargo test -p dozer-app context_menu_items
```
Expected：五个现有测试全部原样 PASS（因为 `context_menu_items` 的返回类型/字段值完全没变，只是内部实现改成委托给 `context_menu_spec`）。

- [ ] **Step 5：`context_menu_popup` 改成调用共享函数**

`context_menu_popup`（files/view.rs:873-989 行）目前是"判断 `is_root`/`has_clipboard`，手写 11 个条件分支组装 `Vec<Element>`"——这部分逻辑改成:先算出 `is_root`/`has_clipboard`（这两行不变，原本就在函数开头），再调用 `context_menu_spec` 拿 `MenuSpec`，最后 `menu_spec::to_iced` 转成 `Element`：

```rust
pub fn context_menu_popup<'a>(
    app_state: &'a AppState,
    ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(menu) = &app_state.context_menu else {
        return column![].into();
    };
    let is_root = ws_state
        .file_tree
        .as_ref()
        .map(|t| t.root() == menu.target.as_path())
        .unwrap_or(false);
    let has_clipboard = ws_state.tree_clipboard.is_some();
    let spec = context_menu_spec(&menu.target, menu.is_dir, is_root, has_clipboard);
    // 注意：原 `context_menu_popup` 这里传的是 `Length::Shrink`（不是
    // `menu_item_width` 定宽，也不是 agent/顶栏菜单的 `Fixed`），必须保持
    // `Shrink` 才行为等价。
    let list = crate::menu_spec::to_iced(spec, Length::Shrink);
    // 定位逻辑（`Padding{top,left}` 手算像素定位到点击坐标）保持不变，
    // 从原函数剩余部分原样搬过来（不在本代码块重复贴出，执行时直接保留
    // `context_menu_popup` 原有的定位/容器包装代码，只替换"组装 items"
    // 那一段）。
    list
}
```

执行时的具体做法：**只删掉**原函数里"组装 `items: Vec<Element<...>>`"那一大段（`let mut items = Vec::new();` 到最后一个 `items.push(...)` 为止，包含 `push_sep` 闭包定义），**替换成**上面 `let spec = ...; let list = ...;` 两行；函数最外层把 `items` 包进定位容器的那部分代码（`context_menu_popup` 函数体最后，用 `container(...)` 手算 `Padding{top,left}` 定位到 `menu.pos` 的那段）原样保留，只是原来接的是 `crate::chrome::menu::shell_frosted(items, ...)` 的结果，现在接 `list`（`menu_spec::to_iced` 内部已经调用了 `shell_frosted`，返回值形状一致，接口不变）。

> **已知行为变化（顺带修 bug）**：当前 iced 版 `context_menu_popup` 的 `push_sep` 闭包在"新建文件/新建文件夹"块之后**无条件**再推一条分隔线（files/view.rs:915），导致对**文件**目标（`is_dir == false`，那个块为空）时"搜索"和"复制"之间出现**两条连续分隔线**——native 版 `context_menu_items` 把这条分隔线放在 `if is_dir` 块**内部**，只有一条。本计划以 native 版为唯一真源，会顺带把 iced 侧这条双分隔线修成单条。这是非 mac 平台专属的视觉修正，Task 6 的人工核对建议在非 mac 环境（或至少截图对比）补看一眼，确认文件目标右键菜单不再有双分隔线。

- [ ] **Step 6：人工核对**

`cargo run -p dozer-app`，在文件树里对文件、目录、根目录、有/无剪贴板内容四种情况分别右键，逐条核对菜单项(搜索/新建文件/新建文件夹/复制/粘贴/删除/重命名/复制绝对路径/复制相对路径/在 Finder 中打开/从磁盘重新加载)出现与否、启用/禁用状态、点击行为均与改动前一致。

- [ ] **Step 7：`cargo build -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过，然后 commit**

```bash
git add crates/dozer-app/src/extensions/files/view.rs
git commit -m "refactor(dozer-app): 文件树右键菜单内容改用共享 menu_spec"
```

---

## Phase B：确认弹窗骨架去重

### Task 5：`dialog.rs` 新增 `confirm()` —— 共享确认弹窗骨架

**Files:**
- Modify: `crates/dozer-app/src/dialog.rs`（新增 `ConfirmDialog<Msg>` + `confirm()`）

**Interfaces:**
- Produces：
  - `pub struct ConfirmDialog<Msg> { pub icon: Option<IconKind>, pub title: String, pub description: String, pub cancel_label: String, pub cancel_msg: Msg, pub confirm_label: String, pub confirm_msg: Msg, pub confirm_color: Color, pub content_spacing: f32 }`
  - `pub fn confirm<'a, Msg: 'a + Clone>(spec: ConfirmDialog<Msg>, window_width: f32) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer>`

- [ ] **Step 1：写 `ConfirmDialog`/`confirm`，加在 `dialog.rs` 文件末尾（`scrim_layer` 之后）**

```rust
use byteui::interaction::icons::IconKind;

/// `confirm()` 的入参——字段数≥7 且 `title`/`description` 两个相邻同类型
/// `String` 传错顺序编译器发现不了，按 CLAUDE.md 关键裁决用具名字段结构体
/// 代替位置参数。
pub struct ConfirmDialog<Msg> {
    /// 标题前的可选图标（无图标传 `None`，如文件/主机/数据源删除确认）。
    pub icon: Option<IconKind>,
    pub title: String,
    pub description: String,
    pub cancel_label: String,
    pub cancel_msg: Msg,
    pub confirm_label: String,
    pub confirm_msg: Msg,
    /// 确认按钮文字色：`red` 给危险删除，`gold` 给非破坏性主要确认。
    pub confirm_color: Color,
    /// 标题/说明/按钮行之间的纵向间距——四处原弹窗的 `column.spacing`
    /// 不统一(files/ssh/database 用 8、todo 用 12)，为了让 `confirm()` 覆盖
    /// 四处又不改视觉，这里不写死，由各调用方原样搬它原本的间距值。
    pub content_spacing: f32,
}

/// 标题 + 说明 + 取消/确认两按钮的确认弹窗骨架——`files::delete_confirm_popup`/
/// `ssh::delete_confirm_popup`/`todo::clear_confirm_popup`/
/// `database::delete_confirm_popup` 共用同一份组装，取代此前四处手写。
/// 只适用于"纯文字+两按钮"的简单确认框；带输入框/单选组等额外控件的弹窗
/// (`files::move_confirm_popup`/`project::project_delete_confirm_popup`)
/// 不适用，继续各自实现。
pub fn confirm<'a, Msg: 'a + Clone>(
    spec: ConfirmDialog<Msg>,
    window_width: f32,
) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    let title_row: Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> = match spec.icon
    {
        Some(icon) => row![
            byteui::interaction::icons::view(
                icon,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().cream,
            ),
            iced_widget::text(spec.title)
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().cream),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center)
        .into(),
        None => iced_widget::text(spec.title)
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().cream)
            .into(),
    };
    let cancel = button(
        iced_widget::text(spec.cancel_label)
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim),
    )
    .on_press(spec.cancel_msg)
    .padding([6, 12])
    .style(action_button_style(byteui::theme::color::current().dim));
    let confirm = button(
        iced_widget::text(spec.confirm_label)
            .size(byteui::theme::font::body())
            .color(spec.confirm_color),
    )
    .on_press(spec.confirm_msg)
    .padding([6, 12])
    .style(action_button_style(spec.confirm_color));

    let dialog = container(
        column![
            title_row,
            iced_widget::text(spec.description)
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().dim),
            actions(iced_widget::row![cancel, confirm].spacing(8)),
        ]
        .spacing(spec.content_spacing),
    )
    .width(width(window_width))
    .padding(16)
    .style(card_style);

    container(dialog)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}
```

（`row!`/`column!`/`text`/`button` 等宏/函数需要跟文件顶部已有 `use iced_widget::{...}` 保持一致——`dialog.rs` 目前顶部 `use` 里没有 `text`/`row`，Step 1 落地时按 `iced_widget::{button, column, container, row, text, MouseArea, Row, Stack}` 补全 `use` 列表，避免每处手写全限定路径。）

- [ ] **Step 2：写单测**

```rust
#[cfg(test)]
mod confirm_tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    enum TestMsg {
        Cancel,
        Confirm,
    }

    #[test]
    fn confirm_dialog_struct_carries_all_fields() {
        // 这个测试只验证 ConfirmDialog 结构体字段可以正常构造和读取——
        // confirm() 返回 iced Element，无法在单测里断言内部渲染结构，
        // 真正的视觉/交互核对在 Task 6-9 的人工核对步骤里做。
        let spec = ConfirmDialog {
            icon: None,
            title: "删除文件 \"a.txt\"?".to_string(),
            description: "会移入系统回收站。".to_string(),
            cancel_label: "取消".to_string(),
            cancel_msg: TestMsg::Cancel,
            confirm_label: "删除".to_string(),
            confirm_msg: TestMsg::Confirm,
            confirm_color: byteui::theme::color::current().red,
            content_spacing: 8.0,
        };
        assert_eq!(spec.title, "删除文件 \"a.txt\"?");
        assert_eq!(spec.confirm_msg, TestMsg::Confirm);
    }
}
```

- [ ] **Step 3：跑测试**

```
cargo test -p dozer-app confirm_dialog_struct_carries_all_fields
```
Expected：PASS

- [ ] **Step 4：`cargo build -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过，然后 commit**

```bash
git add crates/dozer-app/src/dialog.rs
git commit -m "feat(dozer-app): dialog 新增 confirm() 共享确认弹窗骨架"
```

---

### Task 6：迁移文件删除确认（`extensions/files/view.rs::delete_confirm_popup`）

**Files:**
- Modify: `crates/dozer-app/src/extensions/files/view.rs:1000-1060` 左右

- [ ] **Step 1：把函数体改成调用 `dialog::confirm`**

```rust
pub fn delete_confirm_popup(
    ws_state: &WorkspaceState,
    window_width: f32,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some((path, is_dir)) = &ws_state.tree_delete_confirm else {
        return column![].into();
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let kind = if *is_dir { "文件夹" } else { "文件" };
    crate::dialog::confirm(
        crate::dialog::ConfirmDialog {
            icon: None,
            title: format!("删除{kind} \"{name}\"?"),
            description: "会移入系统回收站,可从回收站找回。".to_string(),
            cancel_label: "取消".to_string(),
            cancel_msg: Message::DeleteCancel,
            confirm_label: "删除".to_string(),
            confirm_msg: Message::DeleteConfirm,
            confirm_color: byteui::theme::color::current().red,
            content_spacing: 8.0,
        },
        window_width,
    )
}
```

- [ ] **Step 2：人工核对**

`cargo run -p dozer-app`，文件树右键删除一个文件/文件夹，确认弹窗标题/说明文字/按钮文字/取消确认行为跟改动前一致。

- [ ] **Step 3：`cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过，然后 commit**

```bash
git add crates/dozer-app/src/extensions/files/view.rs
git commit -m "refactor(dozer-app): 文件删除确认弹窗改用 dialog::confirm"
```

---

### Task 7：迁移 SSH 主机删除确认（`extensions/ssh.rs::delete_confirm_popup`）

**Files:**
- Modify: `crates/dozer-app/src/extensions/ssh.rs:1364-1421`

- [ ] **Step 1：把函数体改成调用 `dialog::confirm`**

```rust
pub fn delete_confirm_popup<'a>(
    ws_state: &'a WorkspaceState,
    host_id: &'a str,
    window_width: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let name = ws_state
        .hosts()
        .iter()
        .find(|h| h.id == host_id)
        .map(|h| h.name.as_str())
        .unwrap_or(host_id);
    crate::dialog::confirm(
        crate::dialog::ConfirmDialog {
            icon: None,
            title: format!("删除主机 \"{name}\"?"),
            description: "这会永久删除这台主机的连接记录。".to_string(),
            cancel_label: "取消".to_string(),
            cancel_msg: Message::DeleteHostCancel,
            confirm_label: "删除".to_string(),
            confirm_msg: Message::DeleteHost(host_id.to_string()),
            confirm_color: byteui::theme::color::current().red,
            content_spacing: 8.0,
        },
        window_width,
    )
}
```

- [ ] **Step 2：人工核对**

`cargo run -p dozer-app`，SSH 面板删除一台主机，确认弹窗跟改动前一致。

- [ ] **Step 3：`cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过，然后 commit**

```bash
git add crates/dozer-app/src/extensions/ssh.rs
git commit -m "refactor(dozer-app): SSH 主机删除确认弹窗改用 dialog::confirm"
```

---

### Task 8：迁移 Todo 清空列表确认（`extensions/todo/view.rs::clear_confirm_popup`）

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo/view.rs:430-490` 左右

- [ ] **Step 1：把函数体改成调用 `dialog::confirm`（这个是唯一带标题图标的，且列间距是 12）**

```rust
pub fn clear_confirm_popup(
    _ws_state: &WorkspaceState,
    window_width: f32,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    crate::dialog::confirm(
        crate::dialog::ConfirmDialog {
            icon: Some(icons::IconKind::ListTodo),
            title: "清空列表".to_string(),
            description: "这会清空当前项目的全部任务,操作不可撤销。".to_string(),
            cancel_label: "取消".to_string(),
            cancel_msg: Message::ClearListCancel,
            confirm_label: "清空".to_string(),
            confirm_msg: Message::ClearListConfirm,
            confirm_color: byteui::theme::color::current().red,
            // 注意：原 `clear_confirm_popup` 的 `column.spacing(12)`，其它
            // 三处弹窗是 8，这里原样保留 12，不随 `confirm()` 默认值归一。
            content_spacing: 12.0,
        },
        window_width,
    )
}
```

- [ ] **Step 2：人工核对**

`cargo run -p dozer-app`，Todo 面板点"清空列表"，确认弹窗（含图标）跟改动前一致。

- [ ] **Step 3：`cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过，然后 commit**

```bash
git add crates/dozer-app/src/extensions/todo/view.rs
git commit -m "refactor(dozer-app): Todo 清空列表确认弹窗改用 dialog::confirm"
```

---

### Task 9：迁移数据源删除确认（`extensions/database/view.rs::delete_confirm_popup`）

**Files:**
- Modify: `crates/dozer-app/src/extensions/database/view.rs:580-636` 左右

- [ ] **Step 1：把函数体改成调用 `dialog::confirm`**

```rust
pub fn delete_confirm_popup<'a>(
    ws_state: &'a WorkspaceState,
    source_id: &'a str,
    window_width: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let name = ws_state
        .sources()
        .iter()
        .find(|s| s.id == source_id)
        .map(|s| s.name.to_string())
        .unwrap_or_else(|| source_id.to_string());
    crate::dialog::confirm(
        crate::dialog::ConfirmDialog {
            icon: None,
            title: format!("删除数据源 \"{name}\"?"),
            description: "这会永久删除这条连接记录及其保存的密码。".to_string(),
            cancel_label: "取消".to_string(),
            cancel_msg: Message::DeleteSourceCancel,
            confirm_label: "删除".to_string(),
            confirm_msg: Message::DeleteSource(source_id.to_string()),
            confirm_color: byteui::theme::color::current().red,
            content_spacing: 8.0,
        },
        window_width,
    )
}
```

- [ ] **Step 2：人工核对**

`cargo run -p dozer-app`，数据库面板删除一个数据源，确认弹窗跟改动前一致。

- [ ] **Step 3：`cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过，然后 commit**

```bash
git add crates/dozer-app/src/extensions/database/view.rs
git commit -m "refactor(dozer-app): 数据源删除确认弹窗改用 dialog::confirm"
```

---

### Task 10：全量校验 + 人工验收

- [ ] **Step 1：全 workspace 校验**

```bash
cargo build && cargo test -p dozer-app && cargo clippy --all-targets && cargo fmt --check
```
Expected：全绿。

- [ ] **Step 2：人工走一遍所有改动过的入口**

`cargo run -p dozer-app`，依次核对：
- Agent 面板"＋"菜单（Task 2）
- 顶栏"＋新增项目"菜单，确认文案是"打开项目"（Task 3）
- 文件树右键菜单四种场景：普通文件/目录/根目录/有剪贴板内容（Task 4）
- 文件删除确认（Task 6）
- SSH 主机删除确认（Task 7）
- Todo 清空列表确认（Task 8）
- 数据源删除确认（Task 9）

- [ ] **Step 3：提请审阅**

全绿后提请审阅，审阅通过才合并 `refactor/menu-dialog-dedup` 到 `main`。

---

## 已知未覆盖、留给后续的候选（不在本计划范围）

- `extensions/todo/view.rs::dispatch_items`/`status_items`（native）——本计划没有读过对应的 iced fallback 版本源码确认是否真的重复，需要的话先读代码核实再另开 Task。
- `extensions/ssh/sftp.rs::context_menu_items`（native）——同上，没有核实对应 iced 版本。
- `extensions/files/view.rs::move_confirm_popup`、`extensions/project/view.rs::project_delete_confirm_popup`——已读源码确认形状不适用 `dialog::confirm`（前者是表单，后者带单选组），不建议强行套用。
