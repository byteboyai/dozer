# 输入框改用 iced 原生控件 Stage 3(浏览器地址栏迁移)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把浏览器地址栏(`extensions::browser`)从自绘输入(手写光标符号 +
`main.rs` 拦截层路由成 `AddrEvent`)迁移成真正的 `iced_widget::text_input`,
复用 Stage 2(Files 搜索框)验证过的"每帧查询 iced 真实焦点态"架构模式,并
在此基础上处理一个 Files 搜索框没遇到的新形状:地址栏未聚焦时恒显示占位符
"输入网址"(不回显当前网址,这是现状既有行为),聚焦瞬间要预填当前激活 tab
的网址(旧版靠点击时的 `addr_begin()`,这次改成"从 iced 焦点假变真的那一帧"
触发同款预填);以及地址栏与收藏夹星标按钮共享同一个一体化胶囊边框,
`byteui::form::input_text` 目前每次都画自己的边框/背景,需要一个新的
`bare` 参数让调用方接管边框。

**Architecture:** 完全复用 Stage 2 的两条主线,不发明新模式:
1. `main.rs` 渲染循环每帧跑一个新的 `Operation<()>`(`CaptureAddrFocus`,
   同 `extensions::files::CaptureSearchFocus`,但 `traverse` 从一开始就写对,
   不重蹈 [[dozer-operation-traverse-noop-bug]] 那次的坑),把地址栏是否持有
   iced 真实焦点写进 `extensions::browser` 的一个 `static` 桥接,渲染循环里
   `interface` 释放对 `app` 的不可变借用之后立刻读走塞进当前 `Workspace`
   (与 Files 搜索框那次一样,受借用检查器约束,必须分两阶段:`interface
   .operate()` 时只能存进局部变量,`interface.into_cache()` 之后才能
   `app.set_browser_addr_focused(..)`)。
2. `main.rs` 键盘拦截链新增的原生输入放行判断(Stage 2 建的那道)追加一个
   `|| app.browser_addr_focused()`,同 Files 搜索框/SSH·Database 表单共用
   一道闸门。

**焦点从假变真时预填、从真变假时清空**这条边缘触发逻辑放在
`browser::Tabs::set_addr_focused` 里(比较新旧值决定要不要触发,不是靠
`AddrClick` 这类点击消息触发——真正的 `text_input` 点击聚焦是 iced 标准鼠标
管线自己处理的,不需要应用层发消息)。

**Tech Stack:** Rust 2024,iced 0.14(`iced_widget::text_input`,
`iced_core::widget::operation::focusable::Focusable`)。

**Spec:** `docs/superpowers/specs/2026-08-20-native-text-input-adoption-design.md`
(本计划实现该 spec 目标 1 里的"浏览器地址栏"一项,以及目标 3 键盘路由放行
判断在这个字段上的落地、目标 5 清理里 `App::ime_cursor_area` 的
`browser_addr_editing()` 分支删除。首页项目搜索框/Todo 添加框留给 Stage 4)。

## Global Constraints

- **在独立分支上开发,不直接提交 main。** 本仓库有长驻自动化在 main 上持续
  开发(建 worktree 前用 `git -C /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
  worktree list` 确认没有同名 worktree),从 main 当前 tip 拉新 worktree:
  `git worktree add ../dozer-native-text-input-stage3 -b
  feature/native-text-input-stage3 main`。后续所有命令都在这个新 worktree
  目录里跑,每次 `git commit`/`git add` 前先 `git branch --show-current`
  确认不在 `main` 上。
- 下面每个 Task 引用的行号以 2026-08-20 main tip(commit `c379a22`)为准;
  执行前先用对应的 `grep -n` 命令核对实际行号,若有出入以代码现状为准
  (本仓库有并发自动化开发,main tip 可能已经前进)。
- **`Operation<()>` 的 `traverse` 必须实现成调用传入闭包**(`fn traverse(&mut
  self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a)))
  { operate(self); }`)——这是 [[dozer-operation-traverse-noop-bug]] 记录的
  真实 Critical bug 教训,Task 2 新增的 `CaptureAddrFocus` 必须从一开始就
  写对,不能照抄任何空实现。
- 每个 Task 结束都要求对应 crate `cargo build` 干净通过;涉及 `dozer-app`
  的 Task 额外要求 `cargo test -p dozer-app --bin dozer <关键字>` 通过。
- 最终 Task 要求全 workspace `cargo build && cargo test -p byteui -p
  dozer-app --bin dozer && cargo clippy --all-targets && cargo fmt --check`
  全绿,记录实际 pass/fail 数字(已知基线:633 个既有测试 + 本计划新增的
  测试;`git_log` 分支名 dogfood 环境脆弱性失败不算本计划引入)。
- **迁移完成后的人工 GUI 走查不可省略**(自动化测不出真光标/IME 候选框
  跟手),Task 4 列出完整走查清单。
- **本次是一个已知的、经用户确认的行为变化**:旧版点击地址栏胶囊内任意
  位置(含 4px padding 区域)都能进入编辑态,新版精确到 `text_input` 自身
  的命中范围(padding 区域不再可点)。这是为了避免给 `byteui::form::
  input_text` 塞一个只有这一个调用点用得到的"外部点击代理"能力,判定为
  可接受的小回归,不在本计划范围内修复,写进 Task 4 的走查清单里确认一下
  就好,不需要额外设计。

---

## Task 1: `byteui::form::input_text` 补齐 `bare`(无边框/背景变体)

**Files:**
- Modify: `crates/byteui/src/form/input_text.rs`
- Modify: `crates/byteui/src/form/mod.rs`(既有测试调用点)
- Modify: `crates/dozer-app/src/extensions/ssh.rs`(7 处调用)
- Modify: `crates/dozer-app/src/extensions/database.rs`(8 处调用)
- Modify: `crates/dozer-app/src/extensions/files.rs`(1 处调用)

**Interfaces:**
- Produces: `input_text::view` 新签名——在 `on_submit` 之后、`on_input` 之前
  插入 `bare: bool`(为真时不画自己的卡片背景/边框/聚焦金框,只负责文字/
  光标/选区渲染,由调用方外层容器接管视觉包装;为假时行为与现状完全一致)。
  新签名:`view(placeholder, value, secure, id, highlight, on_submit, bare,
  on_input)`。SSH/Database(15 处)与 Files 搜索框(1 处)现有调用全部传
  `false`,零行为变化。浏览器地址栏(Task 2)是第一个传 `true` 的调用点。

- [x] **Step 1: 确认当前签名**

```bash
command grep -n "pub fn view" crates/byteui/src/form/input_text.rs
```

预期看到 Stage 2 Task 1 定的 7 参数签名(`placeholder`/`value`/`secure`/
`id`/`highlight`/`on_submit`/`on_input`)。

- [x] **Step 2: 写签名变化的失败断言**

`crates/byteui/src/form/mod.rs` 里把:

```rust
        let _ = input_text::view(
            "placeholder",
            "value",
            false,
            None,
            false,
            None,
            Msg::Input,
        );
```

改成:

```rust
        let _ = input_text::view(
            "placeholder",
            "value",
            false,
            None,
            false,
            None,
            false,
            Msg::Input,
        );
```

- [x] **Step 3: 跑测试确认失败**

Run: `cargo test -p byteui all_form_components -- --nocapture`
Expected: 编译失败(`input_text::view` 目前只有 7 参数,新测试传了 8 个)。

- [x] **Step 4: 改 `input_text::view` 签名**

`crates/byteui/src/form/input_text.rs` 当前内容(Stage 2 Task 1 落地后的
样子——`highlight`/`on_submit_maybe` 已经在,`command grep -n "pub fn view"
-A50 crates/byteui/src/form/input_text.rs` 核对实际内容后再改):

```rust
pub fn view<'a, Message: Clone + 'a>(
    placeholder: &str,
    value: &str,
    secure: bool,
    id: Option<widget::Id>,
    highlight: bool,
    on_submit: Option<Message>,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let input = iced_widget::text_input(placeholder, value)
        .secure(secure)
        .on_input(on_input)
        .on_submit_maybe(on_submit)
        .size(crate::theme::font::body())
        .padding(8);
    let input = if let Some(id) = id {
        input.id(id)
    } else {
        input
    };
    input
        .style(move |_theme: &iced_widget::Theme, status: Status| {
            let colors = crate::theme::color::current();
            let focused = matches!(status, Status::Focused { .. });
            text_input::Style {
                background: colors.card.into(),
                border: Border {
                    color: if focused || highlight {
                        colors.gold
                    } else {
                        colors.border
                    },
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
pub fn view<'a, Message: Clone + 'a>(
    placeholder: &str,
    value: &str,
    secure: bool,
    id: Option<widget::Id>,
    highlight: bool,
    on_submit: Option<Message>,
    bare: bool,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let input = iced_widget::text_input(placeholder, value)
        .secure(secure)
        .on_input(on_input)
        .on_submit_maybe(on_submit)
        .size(crate::theme::font::body())
        .padding(8);
    let input = if let Some(id) = id {
        input.id(id)
    } else {
        input
    };
    input
        .style(move |_theme: &iced_widget::Theme, status: Status| {
            let colors = crate::theme::color::current();
            let focused = matches!(status, Status::Focused { .. });
            if bare {
                return text_input::Style {
                    background: iced_widget::core::Color::TRANSPARENT.into(),
                    border: Border {
                        color: iced_widget::core::Color::TRANSPARENT,
                        width: 0.0,
                        radius: 0.0.into(),
                    },
                    icon: colors.dim,
                    placeholder: colors.dim,
                    value: colors.cream,
                    selection: crate::theme::color::mix(colors.gold, colors.card, 0.6),
                };
            }
            text_input::Style {
                background: colors.card.into(),
                border: Border {
                    color: if focused || highlight {
                        colors.gold
                    } else {
                        colors.border
                    },
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

- [x] **Step 5: 跑测试确认通过**

Run: `cargo test -p byteui all_form_components -- --nocapture`
Expected: PASS。

- [x] **Step 6: 编译 dozer-app,确认因签名变化报错的调用方清单**

Run: `cargo build -p dozer-app --bin dozer 2>&1 | grep "error\[" `
Expected:报一批 `ssh.rs`(7)/`database.rs`(8)/`files.rs`(1)里
`input_text::view` 调用"参数数量不对"的错误。

- [x] **Step 7: 机械修复 `ssh.rs` 的 7 处调用**

```bash
command grep -n "byteui::form::input_text::view(" -A8 crates/dozer-app/src/extensions/ssh.rs
```

每处调用在现有的 `false,`/`None,`(on_submit)之后、`Message::DraftXxx
Changed` 之前插入一行 `false,`(bare)。7 处逐一同款处理。

- [x] **Step 8: 机械修复 `database.rs` 的 8 处调用**

```bash
command grep -n "byteui::form::input_text::view(" -A8 crates/dozer-app/src/extensions/database.rs
```

同款处理,8 处。

- [x] **Step 9: 机械修复 `files.rs` 的 1 处调用**

```bash
command grep -n "byteui::form::input_text::view(" -A8 crates/dozer-app/src/extensions/files.rs
```

Stage 2 落地的调用现状:

```rust
    let search_box = byteui::form::input_text::view(
        "搜索目录…",
        &ws_state.tree_search,
        false,
        Some(search_field_id()),
        search_active,
        Some(Message::SearchSubmit),
        Message::SearchInput,
    );
```

在 `Some(Message::SearchSubmit),` 之后插入 `false,`:

```rust
    let search_box = byteui::form::input_text::view(
        "搜索目录…",
        &ws_state.tree_search,
        false,
        Some(search_field_id()),
        search_active,
        Some(Message::SearchSubmit),
        false,
        Message::SearchInput,
    );
```

- [x] **Step 10: 构建 + 测试确认绿**

Run: `cargo build -p byteui -p dozer-app --bin dozer && cargo test -p byteui -p dozer-app --bin dozer ssh && cargo test -p byteui -p dozer-app --bin dozer database && cargo test -p byteui -p dozer-app --bin dozer files::`
Expected: 编译通过,相关既有测试全部 PASS。

- [x] **Step 11: clippy + fmt**

Run: `cargo clippy -p byteui -p dozer-app --all-targets && cargo fmt --package byteui --package dozer-app`
Expected: 无新增警告;fmt 无残留改动(若有,一并提交)。

- [x] **Step 12: Commit**

```bash
git branch --show-current
git add crates/byteui/src/form/input_text.rs crates/byteui/src/form/mod.rs \
  crates/dozer-app/src/extensions/ssh.rs crates/dozer-app/src/extensions/database.rs \
  crates/dozer-app/src/extensions/files.rs
git commit -m "feat(byteui): input_text 新增 bare 参数,支持外部容器接管边框/背景"
```

---

## Task 2: 浏览器地址栏状态与渲染改用真正的 `text_input`

**Files:**
- Modify: `crates/dozer-app/src/extensions/browser.rs`

**Interfaces:**
- Consumes: `byteui::form::input_text::view`(Task 1 新签名)。
- Produces: `pub fn addr_field_id() -> iced_widget::core::widget::Id`;
  `pub struct CaptureAddrFocus`(实现 `Operation<()>`);`pub fn
  take_addr_focused() -> bool`;`State::addr_focused(&self) -> bool` /
  `State::set_addr_focused(&mut self, focused: bool)`(取代旧的
  `addr_editing()`/`addr_cancel()`)。

- [x] **Step 1: 确认当前状态(核对行号)**

```bash
command grep -n "addr_editing\|addr_begin\|addr_text\|addr_backspace\|addr_cancel\|AddrClick\|AddrEvent\|struct Tabs" crates/dozer-app/src/extensions/browser.rs
```

- [x] **Step 2: import 改动**

文件顶部(约第 11-20 行)当前:

```rust
use crate::app::{panel_tab, tab_divider};
use crate::preview::WebviewSpec;
use crate::theme;
use crate::workspace::{lh, split_portions};
use byteui::interaction::icons;
use dozer_client::Client;
use dozer_core::protocol::{BookmarkInfo, BookmarkScope};
use iced_widget::core::{Border, Element, Length};
use iced_widget::{MouseArea, button, column, container, row, text};
use std::collections::HashMap;
```

改成(`Rectangle` 并入既有 core 导入,新增 `widget::{Id, Operation}` 与
`operation::Focusable`):

```rust
use crate::app::{panel_tab, tab_divider};
use crate::preview::WebviewSpec;
use crate::theme;
use crate::workspace::{lh, split_portions};
use byteui::interaction::icons;
use dozer_client::Client;
use dozer_core::protocol::{BookmarkInfo, BookmarkScope};
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use iced_widget::core::{Border, Element, Length, Rectangle};
use iced_widget::{MouseArea, button, column, container, row, text};
use std::collections::HashMap;
```

- [x] **Step 3: `Tabs` 结构体字段改动**

约第 30-37 行当前:

```rust
#[derive(Default)]
pub struct Tabs {
    tabs: Vec<BrowserTab>,
    active: usize,
    next_id: usize,
    addr_editing: bool,
    addr_buffer: String,
}
```

改成:

```rust
#[derive(Default)]
pub struct Tabs {
    tabs: Vec<BrowserTab>,
    active: usize,
    next_id: usize,
    /// 地址栏是否持有 iced 内部真实焦点。**不是**应用层手动置位的镜像——
    /// 每帧渲染循环里 `CaptureAddrFocus` 问一遍 iced 真相后立刻写进这里
    /// (`set_addr_focused`),`main.rs` 键盘路由读它决定要不要把事件放行
    /// 给标准 iced 管线。
    addr_focused: bool,
    addr_buffer: String,
}
```

- [x] **Step 4: `Tabs` 的 addr 方法整体改动**

约第 114-175 行当前(`addr_editing`/`addr_buffer`/`addr_begin`/
`addr_text`/`addr_backspace`/`addr_cancel`/`addr_submit`):

```rust
    pub fn addr_editing(&self) -> bool {
        self.addr_editing
    }

    pub fn addr_buffer(&self) -> &str {
        &self.addr_buffer
    }

    /// 进入地址栏编辑:预填当前激活 tab 的 URL(浏览器 tab 恒为网页,不像
    /// `PreviewPane::addr_begin` 还要 match `TabKind`)。空标签页(`about:blank`)
    /// 没有可编辑的网址,预填会把 `about:blank` 带进输入框、再被后续键入
    /// 拼成 `about:blankhttp://x.com` 这类垃圾——遇到空标签就当空输入处理,
    /// 让用户直接打新地址。
    pub fn addr_begin(&mut self) {
        self.addr_editing = true;
        self.addr_buffer = self
            .tabs
            .get(self.active)
            .map(|t| {
                if t.url.is_empty() || t.url == "about:blank" {
                    String::new()
                } else {
                    t.url.clone()
                }
            })
            .unwrap_or_default();
    }

    pub fn addr_text(&mut self, s: &str) {
        self.addr_buffer.push_str(s);
    }

    pub fn addr_backspace(&mut self) {
        self.addr_buffer.pop();
    }

    pub fn addr_cancel(&mut self) {
        self.addr_editing = false;
        self.addr_buffer.clear();
    }

    /// 提交解析:`Ok(Some(url))` = 有效网址(裸域名自动补 `https://`);
    /// `Ok(None)` = 空输入,no-op;`Err(message)` = 本地路径(以 `/` 或
    /// `~/` 开头),浏览器不支持,`message` 是"浏览器不支持打开本地文件"
    /// 这条文案。带 `://` 的完整 URL(如 `https://x.com`)和单冒号 scheme
    /// (如 `about:blank`/`data:`/`mailto:`)都原样保留,只有既无 `://` 也
    /// 无 scheme 的裸输入(域名/IP/`host:port`)才补 `https://`。
    pub fn addr_submit(&mut self) -> Result<Option<String>, String> {
        self.addr_editing = false;
        let input = std::mem::take(&mut self.addr_buffer);
        let input = input.trim();
        if input.is_empty() {
            return Ok(None);
        }
        if input.starts_with('/') || input.starts_with("~/") {
            return Err("浏览器不支持打开本地文件".to_string());
        }
        if input.contains("://") || has_explicit_scheme(input) {
            return Ok(Some(input.to_string()));
        }
        Ok(Some(format!("https://{input}")))
    }
```

整体替换为:

```rust
    /// 地址栏是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn addr_focused(&self) -> bool {
        self.addr_focused
    }

    pub fn addr_buffer(&self) -> &str {
        &self.addr_buffer
    }

    /// `iced_widget::text_input::on_input` 每次给全量当前字符串。
    pub fn set_addr_buffer(&mut self, s: String) {
        self.addr_buffer = s;
    }

    /// 每帧渲染循环读走 `CaptureAddrFocus` 查到的真实焦点态后写进来。焦点
    /// 从假变真(刚获得焦点)时预填当前激活 tab 的网址——浏览器 tab 恒为
    /// 网页,不像 `PreviewPane::addr_begin` 还要 match `TabKind`。空标签页
    /// (`about:blank`)没有可编辑的网址,预填会把 `about:blank` 带进输入框、
    /// 再被后续键入拼成 `about:blankhttp://x.com` 这类垃圾——遇到空标签就
    /// 当空输入处理,让用户直接打新地址。焦点从真变假(刚失去焦点)时清空
    /// 草稿——不聚焦的地址栏恒显示占位符"输入网址",不回显当前网址(现状
    /// 既有行为,不是本次新增)。
    pub fn set_addr_focused(&mut self, focused: bool) {
        if !self.addr_focused && focused {
            self.addr_buffer = self
                .tabs
                .get(self.active)
                .map(|t| {
                    if t.url.is_empty() || t.url == "about:blank" {
                        String::new()
                    } else {
                        t.url.clone()
                    }
                })
                .unwrap_or_default();
        } else if self.addr_focused && !focused {
            self.addr_buffer.clear();
        }
        self.addr_focused = focused;
    }

    /// 提交解析:`Ok(Some(url))` = 有效网址(裸域名自动补 `https://`);
    /// `Ok(None)` = 空输入,no-op;`Err(message)` = 本地路径(以 `/` 或
    /// `~/` 开头),浏览器不支持,`message` 是"浏览器不支持打开本地文件"
    /// 这条文案。带 `://` 的完整 URL(如 `https://x.com`)和单冒号 scheme
    /// (如 `about:blank`/`data:`/`mailto:`)都原样保留,只有既无 `://` 也
    /// 无 scheme 的裸输入(域名/IP/`host:port`)才补 `https://`。**不**自动
    /// 失焦(同 Files 搜索框 Stage 2 的决定,Enter 提交后光标仍留在输入框
    /// 里,是记录在案的小行为变化,不是遗漏)。
    pub fn addr_submit(&mut self) -> Result<Option<String>, String> {
        let input = std::mem::take(&mut self.addr_buffer);
        let input = input.trim();
        if input.is_empty() {
            return Ok(None);
        }
        if input.starts_with('/') || input.starts_with("~/") {
            return Err("浏览器不支持打开本地文件".to_string());
        }
        if input.contains("://") || has_explicit_scheme(input) {
            return Ok(Some(input.to_string()));
        }
        Ok(Some(format!("https://{input}")))
    }
```

- [x] **Step 5: `addr_field_id`/`CaptureAddrFocus` 新增(紧跟 `Tabs` 的
  `impl` 块之后)**

```rust
/// 地址栏稳定的 iced widget id:`view()` 里 `.id()` 挂给真正的
/// `text_input`,`CaptureAddrFocus` 每帧靠它在 widget 树里认出这一个(同
/// `extensions::files::search_field_id` 的既有手法)。
pub fn addr_field_id() -> Id {
    Id::new("browser-addr-box")
}

static ADDR_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走(非消费)地址栏上一帧是否持有 iced 内部真实焦点。`main.rs` 渲染
/// 循环每帧跑完 `CaptureAddrFocus` 后立刻调用本函数,把结果塞进当前
/// `Workspace`(`State::set_addr_focused`)——`static` 只是临时桥接(同
/// `extensions::files::take_search_focused` 的既有手法)。
pub fn take_addr_focused() -> bool {
    *ADDR_FOCUSED.lock().unwrap()
}

/// 每帧 `interface.operate()` 跑一遍,把 `addr_field_id()` 命中的
/// `text_input` 当前是否持有 iced 焦点写进 `ADDR_FOCUSED`。`traverse`
/// **必须**调用传入的 `operate` 闭包才会继续递归子节点——地址栏嵌在
/// `row!`/`container!` 里,空 `traverse` 会导致 `Row`/`Column` 的
/// `operate()` 直接跳过子节点,`focusable()` 永远不会被触达(见
/// `extensions::files::CaptureSearchFocus` 修复过的同款 Critical bug,
/// commit `b8281cd`,这次从一开始就不能再犯)。
pub struct CaptureAddrFocus;
impl Operation<()> for CaptureAddrFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&addr_field_id()) {
            *ADDR_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}
```

- [x] **Step 6: `State` 的访问器改动**

约第 992-1007 行当前:

```rust
    /// 地址栏是否在编辑态(内核 `App::browser_addr_editing` 键盘路由用)。
    pub fn addr_editing(&self) -> bool {
        self.tabs.addr_editing()
    }
```

紧接着一段之后(`bookmarks_open` 访问器之后)当前:

```rust
    /// 取消地址栏编辑(内核 `App::blur_inputs` 用)。
    pub fn addr_cancel(&mut self) {
        self.tabs.addr_cancel();
    }
```

两处一起改成:

```rust
    /// 地址栏是否持有 iced 真实焦点(内核 `App::browser_addr_focused` 键盘
    /// 路由用)。
    pub fn addr_focused(&self) -> bool {
        self.tabs.addr_focused()
    }
```

```rust
    /// 每帧渲染循环调用:把 `CaptureAddrFocus` 问到的真实焦点态写进来;
    /// 焦点从假变真时顺带清掉上一次提交失败留下的错误提示(同旧版
    /// `AddrClick` 里的 `state.error = None`)。
    pub fn set_addr_focused(&mut self, focused: bool) {
        if !self.tabs.addr_focused() && focused {
            self.error = None;
        }
        self.tabs.set_addr_focused(focused);
    }
```

(即:删除 `addr_editing`/`addr_cancel` 两个方法,新增
`addr_focused`/`set_addr_focused` 两个方法,位置不变。)

- [x] **Step 7: `Message` 枚举改动**

约第 888-918 行 `Message` 枚举里把:

```rust
    AddrClick,
    AddrEvent(crate::workspace::AddrEvent),
```

改成:

```rust
    /// 地址栏草稿变化(iced `text_input::on_input`,每次按键给全量当前
    /// 字符串)。
    AddrInput(String),
    AddrSubmit,
```

(枚举其它变体不动;紧邻的文档注释提到"`AddrEvent` 是地址栏/验收意见框/
项目树行内编辑三处共用"这句话改成"验收意见框/项目树行内编辑两处共用"——
地址栏已不再是消费方,但类型定义本身不删,`acceptance`/`files::EditEvent`/
`extensions::search`/`extensions::project` 仍在用。)

- [x] **Step 8: `update()` 里的处理分支改动**

约第 1202-1222 行当前:

```rust
        Message::AddrClick => {
            state.error = None;
            state.tabs.addr_begin();
        }
        Message::AddrEvent(ev) => match ev {
            crate::workspace::AddrEvent::Text(s) => state.tabs.addr_text(&s),
            crate::workspace::AddrEvent::Backspace => state.tabs.addr_backspace(),
            crate::workspace::AddrEvent::Cancel => state.tabs.addr_cancel(),
            crate::workspace::AddrEvent::Submit => match state.tabs.addr_submit() {
                Ok(Some(url)) => update(
                    state,
                    Message::OpenUrl(url),
                    project_id,
                    client,
                    handle,
                    emit,
                ),
                Ok(None) => {}
                Err(message) => state.error = Some(message),
            },
        },
```

改成:

```rust
        Message::AddrInput(s) => state.tabs.set_addr_buffer(s),
        Message::AddrSubmit => match state.tabs.addr_submit() {
            Ok(Some(url)) => update(state, Message::OpenUrl(url), project_id, client, handle, emit),
            Ok(None) => {}
            Err(message) => state.error = Some(message),
        },
```

- [x] **Step 9: `view()` 里替换地址栏构造**

约第 1583-1636 行当前:

```rust
    let editing = state.addr_editing();
    let addr_text = if editing {
        format!("{}▏", state.tabs.addr_buffer())
    } else {
        "输入网址".to_string()
    };
    let addr_body = lh(text(addr_text)
        .size(byteui::theme::font::body())
        .color(if editing {
            byteui::theme::color::current().cream
        } else {
            byteui::theme::color::current().dim
        }));

    // 地址栏本体:单个带边框的容器,把"网址文字 + 收藏夹按钮"一起包进边框
    // 内(复用 todo 新增输入框 / `crate::search_box` 的布局模式)。整框包
    // 一层 `MouseArea`——点框内(非按钮处)进地址编辑态;收藏夹按钮是内层
    // widget,会先截获自己的点击(开/关收藏夹面板)。框高由收藏按钮的方形
    // 尺寸撑起,文字垂直居中,视觉上按钮嵌在地址栏右侧。
    let content_h = byteui::theme::geometry::tab_button_size();
    let addr_box = MouseArea::new(
        container(
            row![
                container(addr_body)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_y(iced_widget::core::alignment::Vertical::Center)
                    .align_x(iced_widget::core::alignment::Horizontal::Left),
                container(bookmarks_toggle_button(state))
                    .height(Length::Fill)
                    .align_y(iced_widget::core::alignment::Vertical::Center),
            ]
            .width(Length::Fill)
            .height(Length::Fixed(content_h))
            .align_y(iced_widget::core::Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fixed(content_h + 8.0))
        .padding([4, 8])
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().term_bg.into()),
            border: Border {
                color: if editing {
                    byteui::theme::color::current().gold
                } else {
                    byteui::theme::color::current().border
                },
                width: 1.0,
                radius: 2.0.into(),
            },
            ..container::Style::default()
        }),
    )
    .on_press(Message::AddrClick);
```

改成(`text`/`lh` 不再用于地址栏文字,`byteui::form::input_text::view` 的
`bare: true` 接管文字/光标渲染,外层 `container` 继续画那个一体化胶囊边框
——`editing` 现在读的是上一帧 `CaptureAddrFocus` 查到的真实焦点,不是手动
置位的旧字段,变量名沿用不改名,语义变化写进上面 `State::addr_focused`/
`Tabs::set_addr_focused` 的文档注释里,这里不重复):

```rust
    let editing = state.addr_focused();
    let addr_input = byteui::form::input_text::view(
        "输入网址",
        state.tabs.addr_buffer(),
        false,
        Some(addr_field_id()),
        false,
        Some(Message::AddrSubmit),
        true,
        Message::AddrInput,
    );

    // 地址栏本体:单个带边框的容器,把"网址文字 + 收藏夹按钮"一起包进边框
    // 内(复用 todo 新增输入框 / `crate::search_box` 的布局模式)。不再需要
    // 外层 `MouseArea`/`AddrClick`——`text_input` 是真控件,点击命中范围内
    // 就由 iced 标准鼠标管线自己处理聚焦,不需要应用层代理点击(唯一影响:
    // 点击胶囊的 4px padding 空白处不再能进编辑态,只有点在输入框自身范围
    // 内才行,判定为可接受的小回归,见本计划 Global Constraints)。收藏夹
    // 按钮是内层 widget,自己截获点击(开/关收藏夹面板)。框高由收藏按钮的
    // 方形尺寸撑起,文字垂直居中,视觉上按钮嵌在地址栏右侧。
    let content_h = byteui::theme::geometry::tab_button_size();
    let addr_box = container(
        row![
            container(addr_input)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(iced_widget::core::alignment::Vertical::Center)
                .align_x(iced_widget::core::alignment::Horizontal::Left),
            container(bookmarks_toggle_button(state))
                .height(Length::Fill)
                .align_y(iced_widget::core::alignment::Vertical::Center),
        ]
        .width(Length::Fill)
        .height(Length::Fixed(content_h))
        .align_y(iced_widget::core::Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(content_h + 8.0))
    .padding([4, 8])
    .style(move |_t: &iced_widget::Theme| container::Style {
        background: Some(byteui::theme::color::current().term_bg.into()),
        border: Border {
            color: if editing {
                byteui::theme::color::current().gold
            } else {
                byteui::theme::color::current().border
            },
            width: 1.0,
            radius: 2.0.into(),
        },
        ..container::Style::default()
    });
```

- [x] **Step 10: 编译确认**

Run: `cargo build -p dozer-app --bin dozer 2>&1 | grep "error\[" `
Expected: 报测试模块里 `addr_editing`/`addr_begin`/`addr_text`/
`addr_backspace`/`addr_cancel`/`AddrClick`/`AddrEvent` 相关的未定义错误
(Step 11 修),以及 `main.rs`/`workspace.rs`/`app.rs` 里 `browser_addr_
editing`/相关调用点的错误(Task 3 修)。此步只确认 browser.rs 自身逻辑
改动的编译面貌。

- [x] **Step 11: 重写测试**

`Tabs` 单元测试(约第 258-319 行,`addr_edit_and_submit_parses_url_and_
rejects_local_paths`/`addr_submit_keeps_explicit_schemes_and_prepends_
https`/`addr_begin_prefills_current_tab_url`)整体替换为:

```rust
    #[test]
    fn addr_focus_and_submit_parses_url_and_rejects_local_paths() {
        let mut t = Tabs::default();
        t.set_addr_focused(true);
        assert!(t.addr_focused());
        t.set_addr_buffer("localhost:3000/x".to_string());
        assert_eq!(t.addr_submit(), Ok(Some("https://localhost:3000/x".into())));

        t.set_addr_focused(true);
        t.set_addr_buffer("baidu.com".to_string());
        assert_eq!(t.addr_submit(), Ok(Some("https://baidu.com".into())));

        t.set_addr_focused(true);
        t.set_addr_buffer("https://example.com".to_string());
        assert_eq!(t.addr_submit(), Ok(Some("https://example.com".into())));

        t.set_addr_focused(true);
        t.set_addr_buffer("/tmp/x".to_string());
        assert_eq!(t.addr_submit(), Err("浏览器不支持打开本地文件".to_string()));

        t.set_addr_focused(true);
        t.set_addr_buffer("~/x".to_string());
        assert_eq!(t.addr_submit(), Err("浏览器不支持打开本地文件".to_string()));

        t.set_addr_focused(true);
        t.set_addr_buffer("".to_string());
        assert_eq!(t.addr_submit(), Ok(None), "空输入不产生动作");

        // 失焦清空草稿,不影响下一次聚焦时重新预填。
        t.set_addr_focused(true);
        t.set_addr_buffer("x".to_string());
        t.set_addr_focused(false);
        assert_eq!(t.addr_buffer(), "");
        assert!(!t.addr_focused());
    }

    #[test]
    fn addr_submit_keeps_explicit_schemes_and_prepends_https() {
        let mut t = Tabs::default();

        // 单冒号特殊 scheme 原样保留,不补 https://。
        for scheme_url in ["about:blank", "data:text/html,hi", "mailto:a@b.com"] {
            t.set_addr_focused(true);
            t.set_addr_buffer(scheme_url.to_string());
            assert_eq!(t.addr_submit(), Ok(Some(scheme_url.into())));
        }

        // host:port 不是 scheme,应补 https://。
        t.set_addr_focused(true);
        t.set_addr_buffer("example.com:8080".to_string());
        assert_eq!(t.addr_submit(), Ok(Some("https://example.com:8080".into())));
    }

    #[test]
    fn set_addr_focused_true_prefills_current_tab_url() {
        let mut t = Tabs::default();
        t.open_url("http://a.com".into());
        t.set_addr_focused(true);
        assert_eq!(t.addr_buffer(), "http://a.com");
    }

    #[test]
    fn set_addr_focused_true_on_blank_tab_prefills_empty() {
        let mut t = Tabs::default();
        // 默认带一个 about:blank 标签页。
        t.set_addr_focused(true);
        assert_eq!(t.addr_buffer(), "", "about:blank 不预填,避免拼出垃圾");
    }

    #[test]
    fn addr_field_id_is_stable_across_calls() {
        assert_eq!(addr_field_id(), addr_field_id());
    }
```

`update()` 测试(约第 547-619 行,`update_addr_event_submit_ok_recurses_
into_open_url`/`update_addr_event_submit_local_path_sets_error_without_
opening_tab`)改成:

```rust
    #[tokio::test]
    async fn update_addr_submit_ok_recurses_into_open_url() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::AddrInput("http://a.com".to_string()),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        update(
            &mut state,
            Message::AddrSubmit,
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(
            state.tabs.tabs().len(),
            2,
            "默认带一个 about:blank(下标 0),提交应再开一个真实 tab(下标 1)"
        );
        assert_eq!(state.tabs.tabs()[1].url, "http://a.com");
    }

    #[tokio::test]
    async fn update_addr_submit_local_path_sets_error_without_opening_tab() {
        let mut state = State::default();
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(
            &mut state,
            Message::AddrInput("/tmp/x".to_string()),
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        update(
            &mut state,
            Message::AddrSubmit,
            Some(1),
            &client,
            &handle,
            |_| {},
        );
        assert_eq!(state.error.as_deref(), Some("浏览器不支持打开本地文件"));
        assert_eq!(
            state.tabs.tabs().len(),
            1,
            "仍是默认的 about:blank,未新增 tab"
        );
    }
```

`state_accessors_delegate_to_tabs`(约第 784-799 行)改成:

```rust
    #[test]
    fn state_accessors_delegate_to_tabs() {
        let mut state = State::default();
        assert!(!state.addr_focused());
        // 默认带一个 about:blank 标签页,它就是激活 tab
        assert_eq!(state.active_webview_id(), Some(0));
        assert_eq!(state.desired_webviews().len(), 1);
        state.tabs.open_url("http://a.com".into());
        state.set_addr_focused(true);
        assert!(state.addr_focused());
        state.set_addr_focused(false);
        assert!(!state.addr_focused());
        // 新开的 a.com 成为激活 tab,空标签仍在列表里
        assert_eq!(state.active_webview_id(), Some(1));
        assert_eq!(state.desired_webviews().len(), 2);
    }
```

- [x] **Step 12: 跑 browser 测试确认通过**

Run: `cargo test -p dozer-app --bin dozer browser:: -- --nocapture`
Expected: 全部 PASS(含改写/新增的测试)。

- [x] **Step 13: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | grep browser.rs; cargo fmt --package dozer-app`
Expected: browser.rs 无新增警告(其它文件的编译错误留给 Task 3)。

- [x] **Step 14: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/browser.rs
git commit -m "feat(dozer-app): 浏览器地址栏改用真正的 iced text_input"
```

---

## Task 3: `main.rs` 接入原生焦点查询 + 键盘路由放行 + 清理

**Files:**
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/workspace.rs`
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes: `extensions::browser::{CaptureAddrFocus, take_addr_focused}`
  (Task 2)。
- Produces: `App::browser_addr_focused(&self) -> bool` /
  `App::set_browser_addr_focused(&mut self, bool)`;
  `Workspace::browser_addr_focused(&self) -> bool`(取代旧的
  `Workspace::browser_addr_editing`/`App::browser_addr_editing`)。

- [x] **Step 1: 确认当前调用点(核对行号)**

```bash
command grep -n "browser_addr_editing\|CaptureSearchFocus\|files_search_focused" crates/dozer-app/src/app.rs crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
```

- [x] **Step 2: `workspace.rs` 访问器改名 + blur_inputs 清理**

约第 1992-1994 行当前:

```rust
    pub fn browser_addr_editing(&self) -> bool {
        self.browser.addr_editing()
    }
```

改成:

```rust
    pub fn browser_addr_focused(&self) -> bool {
        self.browser.addr_focused()
    }
```

约第 2124-2127 行 `blur_inputs()` 里当前:

```rust
    pub fn blur_inputs(&mut self) {
        if self.browser.addr_editing() {
            self.browser.addr_cancel();
        }
```

删掉这三行(方法已在 Task 2 删除——地址栏不再需要点击别处手动清编辑态,
`CaptureAddrFocus` 下一帧自然会把真实焦点丢失同步进来,`set_addr_focused`
的边缘触发逻辑负责清空草稿):

```rust
    pub fn blur_inputs(&mut self) {
```

(紧接着原有的 `self.acceptance.clear_comment_editing();` 等其它行不动。)

约第 2070-2071 行注释提到 `browser_addr_editing()` 的措辞(`tree_editing`
方法的文档注释"同款 browser_addr_editing()/acceptance_comment_editing()")
同步改成 `browser_addr_focused()`。

- [x] **Step 3: `app.rs` 访问器改名 + 新增 setter**

约第 3217-3220 行当前:

```rust
    pub fn browser_addr_editing(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.browser_addr_editing())
    }
```

改成:

```rust
    pub fn browser_addr_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.browser_addr_focused())
    }

    /// 每帧渲染循环调用:把 `extensions::browser::CaptureAddrFocus` 问到
    /// 的真实焦点态写进当前工作区(`main.rs` 键盘路由随后读
    /// `browser_addr_focused` 消费)。
    pub fn set_browser_addr_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.browser.set_addr_focused(focused);
        }
    }
```

- [x] **Step 4: `ime_cursor_area` 删除已死的近似坐标分支**

约第 3974-3985 行当前:

```rust
    pub fn ime_cursor_area(&self, window_w: f32, window_h: f32) -> (f32, f32, f32) {
        let state = self.shell_state();
        if self.browser_addr_editing() {
            let side = state.layout.rail_layout.side_of(PanelKind::Web);
            let (bx, by, _bw, _bh) = preview_content_bounds_for(side, window_w, window_h, &state);
            return (bx + 4.0, by, 20.0);
        }
        if self.acceptance_comment_editing() {
```

删掉 `if self.browser_addr_editing() { ... }` 这一整段(4 行),`if self.
acceptance_comment_editing() { ... }` 分支保留不动(验收意见框不在本计划
范围):

```rust
    pub fn ime_cursor_area(&self, window_w: f32, window_h: f32) -> (f32, f32, f32) {
        let state = self.shell_state();
        if self.acceptance_comment_editing() {
```

上方第 3970-3973 行的文档注释"地址栏/意见框编辑态用预览列上部近似"改成
"意见框编辑态用预览列上部近似(地址栏已迁移 iced 原生 text_input,IME 位置
由 iced 自己算准)"。

- [x] **Step 5: `main.rs` 渲染循环里每帧查询焦点**

```bash
command grep -n "CaptureSearchFocus\|set_files_search_focused\|files_focused" crates/dozer-app/src/main.rs
```

在现有(约第 2213-2236 行)Files 搜索框焦点查询块:

```rust
                                let files_focused =
                                    if matches!(app.left_view(), crate::app::PanelKind::Files) {
                                        interface.operate(
                                            renderer,
                                            &mut extensions::files::CaptureSearchFocus,
                                        );
                                        extensions::files::take_search_focused()
                                    } else {
                                        false
                                    };
```

之后紧接着加(同款两阶段写法——`interface` 还没释放对 `app` 的不可变
借用,只能先存进局部变量;`PanelKind::Web` 是浏览器面板对应的 `left_view`
取值,同 `FocusIntent::Browser` 分支查 `PanelKind::Web` 的既有用法):

```rust
                                let browser_addr_focused =
                                    if matches!(app.left_view(), crate::app::PanelKind::Web) {
                                        interface.operate(
                                            renderer,
                                            &mut extensions::browser::CaptureAddrFocus,
                                        );
                                        extensions::browser::take_addr_focused()
                                    } else {
                                        false
                                    };
```

在现有(约第 2277 行)`app.set_files_search_focused(files_focused);` 之后
紧接着加:

```rust
                                app.set_browser_addr_focused(browser_addr_focused);
```

- [x] **Step 6: 键盘路由放行判断追加浏览器地址栏**

约第 1036 行当前:

```rust
            if app.files_search_focused() || app.ssh_form_open() || app.database_form_open() {
                return;
            }
```

改成:

```rust
            if app.files_search_focused()
                || app.browser_addr_focused()
                || app.ssh_form_open()
                || app.database_form_open()
            {
                return;
            }
```

上方第 1024-1035 行的说明注释里补一句提到浏览器地址栏也是"每帧查真实
焦点"这一支(跟 Files 搜索框同款,不是跟 SSH/Database 同款的"表单是否
打开"粗粒度信号)。

- [x] **Step 7: 从 `to_self_drawn_input`/`addr_message` 里去掉浏览器地址栏**

约第 1045 行当前:

```rust
            let to_browser = app.browser_addr_editing();
```

整行删除(`app.browser_addr_editing()` 已在 Step 3 改名删除,这里本来就
会报编译错误)。

约第 1055-1064 行 `to_self_drawn_input` 的 OR 链当前:

```rust
            let to_self_drawn_input = to_browser
                || to_comment
                || to_tree_edit
                || to_project_name
                || to_search_popup
                || to_todo_search
                || to_todo_add
                || to_todo_content
                || to_todo_markdown
                || to_home_project_search;
```

删掉 `to_browser ||`,把 `to_comment` 提到链首:

```rust
            let to_self_drawn_input = to_comment
                || to_tree_edit
                || to_project_name
                || to_search_popup
                || to_todo_search
                || to_todo_add
                || to_todo_content
                || to_todo_markdown
                || to_home_project_search;
```

约第 1073-1077 行 `addr_message` 闭包当前:

```rust
            let addr_message = |ev: workspace::AddrEvent| -> Message {
                if to_search_popup {
                    Message::Search(extensions::search::Message::QueryEvent(ev))
                } else if to_browser {
                    Message::Browser(extensions::browser::Message::AddrEvent(ev))
                } else if to_comment {
```

删掉 `else if to_browser { ... }` 这一支(`Message::Browser(...::
AddrEvent(..))` 变体已在 Task 2 删除):

```rust
            let addr_message = |ev: workspace::AddrEvent| -> Message {
                if to_search_popup {
                    Message::Search(extensions::search::Message::QueryEvent(ev))
                } else if to_comment {
```

上方第 1065-1072 行的优先级说明注释里删掉"浏览器地址栏 >"这一项(其余
顺序不变,措辞同 Stage 2 处理"文件树搜索框"时的写法)。

- [x] **Step 8: 全量编译确认**

Run: `cargo build --workspace 2>&1 | grep "error\[" `
Expected: 无输出(全绿)。若还有残留 `addr_editing`/`AddrClick`/
`AddrEvent`(浏览器域)引用报错,回到对应文件核实是否漏改。

- [x] **Step 9: 跑 dozer-app 全量测试**

Run: `cargo test -p dozer-app --bin dozer 2>&1 | tail -30`
Expected: 全绿(已知基线:`git_log` 分支名 dogfood 环境脆弱性失败不算本
计划引入)。

- [x] **Step 10: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets && cargo fmt --package dozer-app`
Expected: 无新增警告;fmt 无残留改动(若有,一并提交)。

- [x] **Step 11: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): main.rs 键盘路由为浏览器地址栏新增原生输入放行判断"
```

---

## Task 4: 全量校验 + 人工 GUI 走查 + 提请审阅

**Files:** 无新增修改,本任务只跑校验与人工验证。

- [x] **Step 1: 全量构建 + 测试 + lint**

```bash
cargo build --workspace
cargo test -p byteui
cargo test -p dozer-app --bin dozer
cargo clippy --all-targets
cargo fmt --check
```

记录实际 pass/fail 数字,只允许已知的 `git_log` 基线失败。

- [ ] **Step 2: 人工 GUI 走查(不可替代,自动化测不出这些)**

跑 `cargo run -p dozer-app`,切到浏览器面板,逐条核对地址栏:

1. 真光标显示、方向键(←→/Home/End)移动、鼠标点击定位、鼠标拖拽选中、
   Shift+方向键扩展选区——同 Files 搜索框 Stage 2 走查过的六项基础能力。
2. 点击地址栏(输入框自身范围内)进入编辑态,预填当前 tab 的真实网址;
   在 `about:blank` 空标签页上点击,预填为空(不出现 `about:blank` 字样)。
3. 输入网址敲回车能正常打开(裸域名自动补 `https://`,完整 URL/`about:`/
   `data:`/`mailto:` 原样保留)。
4. 输入本地路径(如 `/tmp/x`)敲回车,提示"浏览器不支持打开本地文件",
   不新开 tab。
5. 点击地址栏外任意位置(失焦),草稿清空、恢复显示占位符"输入网址"
   (不回显刚才输错的内容——现状既有行为)。
6. 收藏夹星标按钮点击不误触发地址栏编辑态(按钮点击应该被按钮自己截获)。
7. 输入法(拼音)候选框跟随光标出现,拼词过程中的字符实时显示在光标处。
8. **关键回归项**:右侧终端 tab 正在跑一个交互进程时,在地址栏里打字/
   ⌘V 粘贴/按方向键,均不漏进终端。
9. 确认已知的小回归:点击地址栏胶囊的 4px padding 空白区域(输入框自身
   范围之外)不再进入编辑态,只有点在文字/光标区域内才行——预期内的行为,
   不是 bug。

- [x] **Step 3: 确认全部 commit 都在特性分支上**

Run: `git log --oneline main..HEAD`(在
`feature/native-text-input-stage3` 分支上执行)
Expected: 列出 Task 1-3 的三个 commit,`git branch --show-current` 不是
`main`。

- [ ] **Step 4: 提请审阅**

不在这一步直接合并——按仓库惯例提请代码审阅,通过后再合并回 `main`
(参考 `superpowers:finishing-a-development-branch`)。审阅通过合并后,
Stage 4(首页项目搜索框 + Todo 添加框,后者要接 Stage 1 已经建好但还没
实际调用点的 `byteui::form::text_area`)的 plan 待开工前基于合并后的 main
重新核对行号再写。
