# 输入框改用 iced 原生控件 Stage 1(byteui 组件层)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 `byteui::form` 补齐后续 4 个字段迁移(Files 搜索框/浏览器地址栏/
首页项目搜索框/Todo 添加框)与 SSH/Database 表单路由修复共同需要的两块
能力——单行 `input_text` 支持外部驱动焦点(`id` 参数)、新增多行自增高
`text_area` 组件——本 Stage 只动 `byteui` 与因签名变化必须机械同步的
`dozer-app` 调用点,不接入任何新交互行为,不改 `main.rs` 键盘路由。

**Architecture:** `input_text::view` 在 `secure` 之后插入一个新参数
`id: Option<widget::Id>`,为 `None` 时行为与现状完全一致(9 个非目标
调用点、SSH/Database 共 15 个调用点全部传 `None`,零行为变化)。新增
`byteui::form::text_area`,包一层 `iced_widget::text_editor`,样式手法照抄
`input_text`(卡片底色+描边,聚焦金框由 iced 内置 `Status::Focused`
驱动),高度不显式设置——`text_editor` 默认 `Length::Shrink`,天然随
内容自增高,不需要额外自增高逻辑。

**Tech Stack:** Rust 2024,iced 0.14(`iced_widget::text_input`/
`text_editor`)。

**Spec:** `docs/superpowers/specs/2026-08-20-native-text-input-adoption-design.md`
(本计划实现该 spec"byteui 新组件"一节;字段迁移与 `main.rs` 路由改动
是后续独立 Stage,该 spec 里已经点名这次的分批策略)。

## Global Constraints

- **在独立分支上开发,不直接提交 main。** 本仓库有长驻自动化在 main 上
  持续开发(建 worktree 前用 `git -C /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
  worktree list` 确认没有同名 worktree),从 main 当前 tip 拉新 worktree:
  `git worktree add ../dozer-native-text-input-stage1 -b
  feature/native-text-input-stage1 main`。后续所有命令都在这个新 worktree
  目录里跑,每次 `git commit`/`git add` 前先 `git branch --show-current`
  确认不在 `main` 上。
- **本计划已经过手工验证签名/生命周期正确**(直接在真实环境编译过
  `text_editor::Content`/`text_input::widget::Id` 相关代码),下面每个
  Task 的代码块可以直接落地,不是未经验证的推测。
- 每个 Task 结束都要求 `cargo build -p byteui` 干净通过;Task 2(改
  `input_text` 签名)额外要求 `cargo build -p dozer-app --bin dozer`
  干净通过(签名变化会波及 dozer-app 的 15 处调用点)。
- 最终 Task 要求全 workspace `cargo build && cargo test -p byteui -p
  dozer-app --bin dozer && cargo clippy --all-targets && cargo fmt --check`
  全绿,记录实际 pass/fail 数字(既有的 `git_log` 分支名 dogfood 环境
  脆弱性失败不算本计划引入,已知基线)。

---

## Task 1: 新增 `byteui::form::text_area`(多行自增高)

**Files:**
- Create: `crates/byteui/src/form/text_area.rs`
- Modify: `crates/byteui/src/form/mod.rs`(注册新模块 + 既有测试追加一条
  构造性验证)

**Interfaces:**
- Produces: `pub fn view<'a, Message: Clone + 'a>(content: &'a
  iced_widget::text_editor::Content, placeholder: &'a str, on_action:
  impl Fn(iced_widget::text_editor::Action) -> Message + 'a) ->
  Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>`——
  `content` 由调用方(未来的 Todo 添加框)在自己的状态里持有并跨帧复用
  (`text_editor::Content` 内部维护光标/选区,不能像 `input_text` 的
  `value: &str` 那样每帧从 `String` 现拼),调用方通过 `on_action` 拿到
  的 `Action` 调 `content.perform(action)` 落地编辑,这是本 Task 不实现
  的下游消费方职责(留给字段迁移的后续 Stage)。

- [ ] **Step 1: 写构造性测试**

`crates/byteui/src/form/mod.rs` 现状(确认后再改,若与下方不一致以
`cat crates/byteui/src/form/mod.rs` 实际内容为准):

```rust
pub mod checkbox;
pub mod input_text;
pub mod select;
pub mod switch;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    #[allow(dead_code)]
    enum Msg {
        Toggled(bool),
        Input(String),
        Selected(&'static str),
    }

    #[test]
    fn all_form_components_construct_without_panic() {
        let _ = checkbox::view("label", false, Msg::Toggled);
        let _ = switch::view("label", true, Msg::Toggled);
        let _ = input_text::view("placeholder", "value", false, Msg::Input);
        let options: &[&str] = &["a", "b"];
        let _ = select::view(options, Some(&"a"), Msg::Selected);
    }
}
```

改成:

```rust
pub mod checkbox;
pub mod input_text;
pub mod select;
pub mod switch;
pub mod text_area;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    #[allow(dead_code)]
    enum Msg {
        Toggled(bool),
        Input(String),
        Selected(&'static str),
        Edited(iced_widget::text_editor::Action),
    }

    #[test]
    fn all_form_components_construct_without_panic() {
        let _ = checkbox::view("label", false, Msg::Toggled);
        let _ = switch::view("label", true, Msg::Toggled);
        let _ = input_text::view("placeholder", "value", false, Msg::Input);
        let options: &[&str] = &["a", "b"];
        let _ = select::view(options, Some(&"a"), Msg::Selected);
        let content = iced_widget::text_editor::Content::new();
        let _ = text_area::view(&content, "placeholder", Msg::Edited);
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p byteui all_form_components -- --nocapture`
Expected: 编译失败(`text_area` 模块 / `text_area::view` 未定义)。

- [ ] **Step 3: 实现 `text_area::view`**

创建 `crates/byteui/src/form/text_area.rs`:

```rust
//! 多行自增高文本编辑框——包一层 `iced_widget::text_editor`,样式对齐
//! `form::input_text`(卡片底色+描边,聚焦金框由 iced 内置 `Status::Focused`
//! 驱动)。高度默认 `Length::Shrink`(iced text_editor 的默认值),随内容
//! 自然撑高,不需要额外的自增高逻辑。

use iced_widget::core::{Border, Element};
use iced_widget::text_editor::{self, Status};

pub fn view<'a, Message: Clone + 'a>(
    content: &'a text_editor::Content,
    placeholder: &'a str,
    on_action: impl Fn(text_editor::Action) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    iced_widget::text_editor(content)
        .placeholder(placeholder)
        .on_action(on_action)
        .size(crate::theme::font::body())
        .padding(8)
        .style(|_theme: &iced_widget::Theme, status: Status| {
            let colors = crate::theme::color::current();
            let focused = matches!(status, Status::Focused { .. });
            text_editor::Style {
                background: colors.card.into(),
                border: Border {
                    color: if focused { colors.gold } else { colors.border },
                    width: 1.0,
                    radius: 6.0.into(),
                },
                placeholder: colors.dim,
                value: colors.cream,
                selection: crate::theme::color::mix(colors.gold, colors.card, 0.6),
            }
        })
        .into()
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p byteui all_form_components -- --nocapture`
Expected: `all_form_components_construct_without_panic` PASS。

- [ ] **Step 5: 构建 + clippy + fmt**

Run: `cargo build -p byteui && cargo clippy -p byteui --all-targets && cargo fmt --package byteui`
Expected: 构建通过,无新增 clippy 警告,fmt 无残留改动(若有,`git add`
一并提交)。

- [ ] **Step 6: Commit**

```bash
git branch --show-current
git add crates/byteui/src/form/text_area.rs crates/byteui/src/form/mod.rs
git commit -m "feat(byteui): 新增 form::text_area 多行自增高编辑框"
```

---

## Task 2: `input_text` 加 `id` 参数(外部驱动焦点)

**Files:**
- Modify: `crates/byteui/src/form/input_text.rs`
- Modify: `crates/byteui/src/form/mod.rs`(既有测试调用点)
- Modify: `crates/dozer-app/src/extensions/ssh.rs`(7 处调用)
- Modify: `crates/dozer-app/src/extensions/database.rs`(8 处调用)

**Interfaces:**
- Produces: `input_text::view` 新签名——在 `secure` 之后、`on_input` 之前
  插入 `id: Option<iced_widget::core::widget::Id>`:
  `view(placeholder, value, secure, id, on_input)`。`id` 为 `Some(..)`
  时把它接到 iced `text_input` 的 `.id(id)`(`iced_widget::core::widget
  ::Id` 类型,`text_input::Id` 不是独立类型,是 `widget::Id` 的别名用法),
  供调用方后续用 `iced_widget::text_input::focus(id)` 返回的 `Task` 驱动
  程序化聚焦——本 Task 只出这个能力,不消费它(15 个现有调用点全部传
  `None`,零行为变化,消费方是后续字段迁移 Stage)。

- [ ] **Step 1: 确认当前签名**

```bash
command grep -n "pub fn view" crates/byteui/src/form/input_text.rs
```

预期看到 4 参数签名(`placeholder`/`value`/`secure`/`on_input`)。

- [ ] **Step 2: 写签名变化的失败断言(编译期断言,走既有构造性测试)**

`crates/byteui/src/form/mod.rs` 里把:

```rust
        let _ = input_text::view("placeholder", "value", false, Msg::Input);
```

改成:

```rust
        let _ = input_text::view("placeholder", "value", false, None, Msg::Input);
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p byteui all_form_components -- --nocapture`
Expected: 编译失败(`input_text::view` 目前只有 4 参数,新测试传了 5 个)。

- [ ] **Step 4: 改 `input_text::view` 签名**

`crates/byteui/src/form/input_text.rs` 当前内容:

```rust
//! amis `form/input-text`(单行文本输入):<https://baidu.github.io/amis/zh-CN/components/form/input-text>

use iced_widget::core::{Border, Element};
use iced_widget::text_input::{self, Status};

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

整体替换为:

```rust
//! amis `form/input-text`(单行文本输入):<https://baidu.github.io/amis/zh-CN/components/form/input-text>

use iced_widget::core::widget;
use iced_widget::core::{Border, Element};
use iced_widget::text_input::{self, Status};

pub fn view<'a, Message: Clone + 'a>(
    placeholder: &str,
    value: &str,
    secure: bool,
    id: Option<widget::Id>,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let input = iced_widget::text_input(placeholder, value)
        .secure(secure)
        .on_input(on_input)
        .size(crate::theme::font::body())
        .padding(8);
    let input = if let Some(id) = id {
        input.id(id)
    } else {
        input
    };
    input
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

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p byteui all_form_components -- --nocapture`
Expected: PASS。

- [ ] **Step 6: 编译 dozer-app,确认因签名变化报错的调用方清单**

Run: `cargo build -p dozer-app --bin dozer 2>&1 | grep "error\[" `
Expected:报一批 `ssh.rs`/`database.rs` 里 `input_text::view` 调用"参数
数量不对"的错误(`id` 是新增位置参数,不影响类型系统之外的任何东西)。

- [ ] **Step 7: 机械修复 `ssh.rs` 的 7 处调用**

`crates/dozer-app/src/extensions/ssh.rs` 当前(以
`grep -n "byteui::form::input_text::view(" -A6 crates/dozer-app/src/extensions/ssh.rs`
重新核对实际行号,执行前如有出入以代码现状为准):

```rust
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

改成:

```rust
        byteui::form::input_text::view(
            "主机名称",
            &draft.name,
            false,
            None,
            Message::DraftNameChanged,
        ),
        byteui::form::input_text::view(
            "Host",
            &draft.host,
            false,
            None,
            Message::DraftHostChanged,
        ),
        byteui::form::input_text::view(
            "port(22)",
            &draft.port,
            false,
            None,
            Message::DraftPortChanged,
        ),
        byteui::form::input_text::view(
            "user name",
            &draft.username,
            false,
            None,
            Message::DraftUsernameChanged,
        ),
```

紧接着:

```rust
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

改成:

```rust
        col = col.push(byteui::form::input_text::view(
            "私钥文件路径,如 ~/.ssh/id_ed25519",
            &draft.key_path,
            false,
            None,
            Message::DraftKeyPathChanged,
        ));
        col = col.push(byteui::form::input_text::view(
            "私钥口令(留空则不修改/无口令)",
            &draft.password,
            true,
            None,
            Message::DraftPasswordChanged,
        ));
    } else {
        col = col.push(byteui::form::input_text::view(
            "password(留空则不修改)",
            &draft.password,
            true,
            None,
            Message::DraftPasswordChanged,
        ));
    }
```

- [ ] **Step 8: 机械修复 `database.rs` 的 8 处调用**

`crates/dozer-app/src/extensions/database.rs` 当前(同样先用
`grep -n "byteui::form::input_text::view(" -A6 crates/dozer-app/src/extensions/database.rs`
核对实际行号):

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

改成:

```rust
    col = col.push(byteui::form::input_text::view(
        "名字",
        &draft.name,
        false,
        None,
        Message::DraftNameChanged,
    ));
    if draft.driver == DriverKind::Sqlite {
        col = col.push(byteui::form::input_text::view(
            "文件路径",
            &draft.database,
            false,
            None,
            Message::DraftDatabaseChanged,
        ));
    } else {
        col = col.push(byteui::form::input_text::view(
            "连接 URI(可选,填了则忽略下面各项,例如 postgres://user:pw@host:5432/db)",
            &draft.uri,
            false,
            None,
            Message::DraftUriChanged,
        ));
        col = col.push(byteui::form::input_text::view(
            "host",
            &draft.host,
            false,
            None,
            Message::DraftHostChanged,
        ));
        col = col.push(byteui::form::input_text::view(
            "port",
            &draft.port,
            false,
            None,
            Message::DraftPortChanged,
        ));
        col = col.push(byteui::form::input_text::view(
            "database",
            &draft.database,
            false,
            None,
            Message::DraftDatabaseChanged,
        ));
        col = col.push(byteui::form::input_text::view(
            "username",
            &draft.username,
            false,
            None,
            Message::DraftUsernameChanged,
        ));
        col = col.push(byteui::form::input_text::view(
            "password(留空则不修改)",
            &draft.password,
            true,
            None,
            Message::DraftPasswordChanged,
        ));
    }
```

- [ ] **Step 9: 构建 + 测试确认绿**

Run: `cargo build -p byteui -p dozer-app --bin dozer && cargo test -p byteui -p dozer-app --bin dozer ssh && cargo test -p byteui -p dozer-app --bin dozer database`
Expected: 编译通过,`ssh`/`database` 相关既有测试全部 PASS(纯参数新增,
不改消息/状态逻辑,不需要改任何测试用例)。

- [ ] **Step 10: clippy + fmt**

Run: `cargo clippy -p byteui -p dozer-app --all-targets && cargo fmt --package byteui --package dozer-app`
Expected: 无新增警告;fmt 无残留改动(若有,一并提交)。

- [ ] **Step 11: Commit**

```bash
git branch --show-current
git add crates/byteui/src/form/input_text.rs crates/byteui/src/form/mod.rs \
  crates/dozer-app/src/extensions/ssh.rs crates/dozer-app/src/extensions/database.rs
git commit -m "feat(byteui): input_text 新增 id 参数,支持外部驱动焦点"
```

---

## Task 3: 全量校验 + 提请审阅

**Files:** 无新增修改,本任务只跑校验。

- [ ] **Step 1: 全量构建 + 测试 + lint**

```bash
cargo build --workspace
cargo test -p byteui
cargo test -p dozer-app --bin dozer
cargo clippy --all-targets
cargo fmt --check
```

记录实际 pass/fail 数字。唯一预期的失败是已知的 `git_log` 分支名 dogfood
环境脆弱性(与本计划改动的文件无关,`git_log.rs` 本身零改动),其余必须
全绿。

- [ ] **Step 2: 确认全部 commit 都在特性分支上**

Run: `git log --oneline main..HEAD`(在
`feature/native-text-input-stage1` 分支上执行)
Expected: 列出 Task 1-2 的两个 commit,`git branch --show-current` 不是
`main`。

- [ ] **Step 3: 提请审阅**

不在这一步直接合并——按仓库惯例提请代码审阅,通过后再合并回 `main`
(参考 `superpowers:finishing-a-development-branch`)。审阅通过合并后,
Stage 2(Files 搜索框迁移,首个真正接入 `main.rs` 路由放行判断 + 消费
本 Stage 的 `id` 参数)的 plan 待开工前基于合并后的 main 重新核对行号
再写。
