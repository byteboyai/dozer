# 输入框改用 iced 原生控件 Stage 2(Files 搜索框迁移)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把文件树搜索框(`extensions::files`)从自绘输入(手写光标符号 +
`main.rs` 拦截层路由成 `SearchEvent`)迁移成真正的 `iced_widget::text_input`,
作为 4 个目标字段里第一个真正接入 `main.rs` 键盘路由放行判断的字段,验证
Stage 1 铺好的 `byteui::form::input_text` 组件层 + 一套全新的"每帧查询 iced
真实焦点态"架构模式,供后续 Stage(浏览器地址栏/首页项目搜索框/Todo 添加框/
SSH·Database 表单路由)复用。

**Architecture:** 搜索框从"点击进入自绘编辑态"改成始终渲染为真正的
`text_input`(iced 标准鼠标点击/键盘事件管线自己处理聚焦,不需要应用层手动
触发)。`main.rs` 现有键盘拦截层完全没有 Task 执行器(`App::update` 签名是
`fn update(&mut self, message: Message)`,不返回 `Task<Message>`;整个渲染
循环是手搓的 `UserInterface::build`/`interface.update()`/`interface.operate()`/
`interface.draw()`,不是 iced 标准 `Program` 运行时),所以 spec 里提到的
"`text_input::focus(id)` 返回 `Task`"这条路线在本仓库不可行。改用本仓库
`extensions::todo::CaptureFieldBounds` 已经在用的手法:每帧渲染循环里跑一次
`interface.operate()`,用一个自定义 `Operation<()>`(内部用 `Focusable` 钩子
查 `iced_widget::core::widget::operation::is_focused` 同款逻辑)把搜索框
是否持有 iced 内部真实焦点写进一个 `static` 桥接,立刻读走塞进当前
`Workspace`。`main.rs` 键盘路由新增的 `app.files_search_focused()` 直接读
这份"每帧问 iced 真相"的状态,不再依赖应用层自己维护、可能与真实焦点脱节的
`SearchEditStart`/`search_editing: bool` 手动镜像——命中即整个
`WindowEvent` 提前 `return`,不吞、不转自绘消息,直接放行给标准 iced 事件
转换管线(`iced_winit::conversion::window_event` → `program.update()`,与
既有 `FocusIntent::Preview` 原生编辑器那道闸门同款手法)。

**Tech Stack:** Rust 2024,iced 0.14(`iced_widget::text_input`,
`iced_core::widget::operation::focusable::Focusable`)。

**Spec:** `docs/superpowers/specs/2026-08-20-native-text-input-adoption-design.md`
(本计划实现该 spec 目标 1 里的"文件树搜索框"一项,以及目标 3 的键盘路由放行
判断——仅覆盖这一个字段,浏览器地址栏/首页项目搜索框/Todo 添加框/
SSH·Database 路由留给后续 Stage;本计划的"每帧查询真实焦点"架构与 spec 原文
"`text_input::focus(id)` 返回 `Task`"的设想不同,因为本仓库没有 Task 执行器,
下面 Architecture 一节已记录改用的等价方案)。

## Global Constraints

- **在独立分支上开发,不直接提交 main。** 本仓库有长驻自动化在 main 上持续
  开发(建 worktree 前用 `git -C /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
  worktree list` 确认没有同名 worktree),从 main 当前 tip 拉新 worktree:
  `git worktree add ../dozer-native-text-input-stage2 -b
  feature/native-text-input-stage2 main`。后续所有命令都在这个新 worktree
  目录里跑,每次 `git commit`/`git add` 前先 `git branch --show-current`
  确认不在 `main` 上。
- 下面每个 Task 引用的行号以 2026-08-20 main tip(commit `17a63cd`)为准;
  执行前先用对应的 `grep -n` 命令核对实际行号,若有出入以代码现状为准
  (本仓库有并发自动化开发,main tip 可能已经前进)。
- 每个 Task 结束都要求对应 crate `cargo build` 干净通过;涉及 `dozer-app`
  的 Task 额外要求 `cargo test -p dozer-app --bin dozer <关键字>` 通过。
- 最终 Task 要求全 workspace `cargo build && cargo test -p byteui -p
  dozer-app --bin dozer && cargo clippy --all-targets && cargo fmt --check`
  全绿,记录实际 pass/fail 数字(已知基线:`git_log` 分支名 dogfood 环境
  脆弱性失败不算本计划引入)。
- **迁移完成后 Enter/⌘V/方向键/鼠标拖拽选中的行为验证只能靠人工 GUI 走查**
  (自动化测不出真光标是否可见、IME 候选框是否跟手),Task 4 列出完整走查
  清单,不能跳过。

---

## Task 1: `byteui::form::input_text` 补齐 `highlight` + `on_submit`

**Files:**
- Modify: `crates/byteui/src/form/input_text.rs`
- Modify: `crates/byteui/src/form/mod.rs`(既有测试调用点)
- Modify: `crates/dozer-app/src/extensions/ssh.rs`(7 处调用)
- Modify: `crates/dozer-app/src/extensions/database.rs`(8 处调用)

**Interfaces:**
- Produces: `input_text::view` 新签名——在 `id` 之后、`on_input` 之前插入
  两个参数:`highlight: bool`(为真时无论是否聚焦都强制画金框,搜索框用来
  表示"当前树已被搜索词过滤"这个持久状态,与 `Status::Focused` 驱动的临时
  聚焦框是两回事,互相独立、可同时为真)、`on_submit: Option<Message>`
  (映射到 iced `text_input::on_submit_maybe`,搜索框靠它让 Enter 直接提交
  过滤词,不需要 `main.rs` 手动拦截 Enter 键)。新签名:
  `view(placeholder, value, secure, id, highlight, on_submit, on_input)`。
  SSH/Database 现有 15 处调用全部传 `false, None,`,零行为变化。

- [ ] **Step 1: 确认当前签名**

```bash
command grep -n "pub fn view" crates/byteui/src/form/input_text.rs
```

预期看到 Stage 1 定的 5 参数签名(`placeholder`/`value`/`secure`/`id`/
`on_input`)。

- [ ] **Step 2: 写签名变化的失败断言**

`crates/byteui/src/form/mod.rs` 里把:

```rust
        let _ = input_text::view("placeholder", "value", false, None, Msg::Input);
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
            Msg::Input,
        );
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p byteui all_form_components -- --nocapture`
Expected: 编译失败(`input_text::view` 目前只有 5 参数,新测试传了 7 个)。

- [ ] **Step 4: 改 `input_text::view` 签名**

`crates/byteui/src/form/input_text.rs` 当前内容(Stage 1 落地后的样子):

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

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p byteui all_form_components -- --nocapture`
Expected: PASS。

- [ ] **Step 6: 编译 dozer-app,确认因签名变化报错的调用方清单**

Run: `cargo build -p dozer-app --bin dozer 2>&1 | grep "error\[" `
Expected:报一批 `ssh.rs`/`database.rs` 里 `input_text::view` 调用"参数
数量不对"的错误。

- [ ] **Step 7: 机械修复 `ssh.rs` 的 7 处调用**

```bash
command grep -n "byteui::form::input_text::view(" -A7 crates/dozer-app/src/extensions/ssh.rs
```

每处调用在现有的 `false,` / `true,`(secure)与 `None,`(id)之后,
`Message::DraftXxxChanged` 之前,插入两行 `false,` `None,`(highlight=false,
on_submit=None)。例如:

```rust
        byteui::form::input_text::view(
            "主机名称",
            &draft.name,
            false,
            None,
            false,
            None,
            Message::DraftNameChanged,
        ),
```

对 7 处调用逐一同款处理(`Host`/`port(22)`/`user name`/`私钥文件路径`/
`私钥口令`/`password`)。

- [ ] **Step 8: 机械修复 `database.rs` 的 8 处调用**

```bash
command grep -n "byteui::form::input_text::view(" -A7 crates/dozer-app/src/extensions/database.rs
```

同款处理(`名字`/`文件路径`/`连接 URI`/`host`/`port`/`database`/`username`/
`password`)。

- [ ] **Step 9: 构建 + 测试确认绿**

Run: `cargo build -p byteui -p dozer-app --bin dozer && cargo test -p byteui -p dozer-app --bin dozer ssh && cargo test -p byteui -p dozer-app --bin dozer database`
Expected: 编译通过,`ssh`/`database` 相关既有测试全部 PASS。

- [ ] **Step 10: clippy + fmt**

Run: `cargo clippy -p byteui -p dozer-app --all-targets && cargo fmt --package byteui --package dozer-app`
Expected: 无新增警告;fmt 无残留改动(若有,一并提交)。

- [ ] **Step 11: Commit**

```bash
git branch --show-current
git add crates/byteui/src/form/input_text.rs crates/byteui/src/form/mod.rs \
  crates/dozer-app/src/extensions/ssh.rs crates/dozer-app/src/extensions/database.rs
git commit -m "feat(byteui): input_text 新增 highlight/on_submit,支持持久高亮与原生回车提交"
```

---

## Task 2: Files 搜索框状态与渲染改用真正的 `text_input`

**Files:**
- Modify: `crates/dozer-app/src/extensions/files.rs`

**Interfaces:**
- Consumes: `byteui::form::input_text::view`(Task 1 新签名)。
- Produces: `pub fn search_field_id() -> iced_widget::core::widget::Id`(供
  Task 3 main.rs 渲染循环里的 `Operation` 与本文件 `view()` 里的 `.id()`
  共用同一个稳定 `Id`);`pub struct CaptureSearchFocus`(实现
  `Operation<()>`,供 Task 3 每帧 `interface.operate()` 调用);
  `pub fn take_search_focused() -> bool`(读走 `CaptureSearchFocus` 写进的
  `static` 桥接值,供 Task 3 塞进 `Workspace`);
  `pub fn search_focused(&self) -> bool` / `pub fn set_search_focused(&mut
  self, focused: bool)`(`WorkspaceState` 新增的一对访问器,取代旧的
  `search_editing()`/字段)。

- [ ] **Step 1: 确认当前状态(核对行号)**

```bash
command grep -n "search_editing\|SearchEditStart\|SearchEvent\|search_box_widget" crates/dozer-app/src/extensions/files.rs
```

- [ ] **Step 2: `WorkspaceState` 字段与 import 改动**

文件顶部 import(约第 7-12 行)当前:

```rust
use crate::workspace::AddrEvent;
use crate::{delivery, theme};
use byteui::interaction::icons;
use iced_widget::core::text::LineHeight;
use iced_widget::core::{Border, Color, Element, Length, Padding};
use iced_widget::{MouseArea, Scrollable, button, column, container, row, scrollable, text};
```

加两行(`Rectangle` 并入既有 core 导入,新增 `widget::{Id, Operation}` 与
`operation::Focusable`):

```rust
use crate::workspace::AddrEvent;
use crate::{delivery, theme};
use byteui::interaction::icons;
use iced_widget::core::text::LineHeight;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use iced_widget::core::{Border, Color, Element, Length, Padding, Rectangle};
use iced_widget::{MouseArea, Scrollable, button, column, container, row, scrollable, text};
```

`WorkspaceState` 结构体里(约第 47-63 行)把:

```rust
    /// 搜索框是否处于自绘编辑态:键盘走 main.rs 拦截层(同树内行编辑
    /// `tree_edit`/验收意见框,不用 iced 原生 text_input)。为真时 main.rs 把
    /// 按键路由成 `SearchEvent`,不再喂给 PTY——否则在搜索框里打字会同时
    /// 漏进已聚焦的终端(本项目所有文本输入都是自绘,理由一致)。
    search_editing: bool,
```

改成:

```rust
    /// 搜索框是否持有 iced 内部真实焦点。**不是**应用层手动置位的镜像——
    /// 每帧渲染循环里 `CaptureSearchFocus` 问一遍 iced 真相后立刻写进这里
    /// (`set_search_focused`),`main.rs` 键盘路由读它决定要不要把事件放行
    /// 给标准 iced 管线(同 `search_field_id`/`CaptureSearchFocus` 说明)。
    search_focused: bool,
```

- [ ] **Step 3: `reset_for_project`/访问器改动**

`impl WorkspaceState` 里(约第 224-256 行)把:

```rust
    pub fn reset_for_project(&mut self, file_tree: FileTree) {
        self.file_tree = Some(file_tree);
        self.tree_selected = None;
        self.git_statuses = HashMap::new();
        self.tree_search.clear();
        self.search_query.clear();
        self.search_editing = false;
    }
```

改成(仅换字段名):

```rust
    pub fn reset_for_project(&mut self, file_tree: FileTree) {
        self.file_tree = Some(file_tree);
        self.tree_selected = None;
        self.git_statuses = HashMap::new();
        self.tree_search.clear();
        self.search_query.clear();
        self.search_focused = false;
    }
```

紧接着把:

```rust
    /// 搜索框失焦退出编辑态(`Workspace::blur_inputs` 用)`:草稿 `tree_search`
    /// 保留,退出后仍作为盒子里的占位/已输入文本继续显示。
    pub fn cancel_search_edit(&mut self) {
        self.search_editing = false;
    }

    /// 供内核 `Workspace::search_editing`(main.rs 键盘路由用)判断搜索框
    /// 是否处于自绘编辑态。
    pub fn search_editing(&self) -> bool {
        self.search_editing
    }
```

整段删除(不再需要——真实 iced 焦点丢失时下一帧 `CaptureSearchFocus`
自然会把 `search_focused` 更新成 `false`,不需要点击别处手动清),换成:

```rust
    /// 每帧渲染循环读走 `CaptureSearchFocus` 查到的真实焦点态后写进来。
    pub fn set_search_focused(&mut self, focused: bool) {
        self.search_focused = focused;
    }

    /// 供内核 `Workspace::files_search_focused`(main.rs 键盘路由用)判断
    /// 搜索框是否持有 iced 真实焦点。
    pub fn search_focused(&self) -> bool {
        self.search_focused
    }
```

- [ ] **Step 4: `Message` 枚举改动**

`Message` 枚举里(约第 148-157 行)当前:

```rust
    EditEvent(AddrEvent),
    /// 点进搜索框开始编辑:置 `search_editing = true`,此后按键交 main.rs
    /// 拦截层路由成 `SearchEvent`(不再漏进终端)。
    SearchEditStart,
    SearchEvent(AddrEvent),
    SearchSubmit,
```

改成:

```rust
    EditEvent(AddrEvent),
    /// 搜索框草稿变化(iced `text_input::on_input`,每次按键给全量当前
    /// 字符串,不是逐字符追加)。只进草稿,不触发过滤——同现状,过滤词由
    /// `SearchSubmit` 落定。
    SearchInput(String),
    SearchSubmit,
```

(`AddrEvent` 仍被 `EditEvent`/`Message::EditEvent` 等非目标字段使用,
`use crate::workspace::AddrEvent;` import 不删。)

- [ ] **Step 5: `update()` 里的处理分支改动**

约第 596-619 行当前:

```rust
        Message::SearchEditStart => {
            ws_state.search_editing = true;
        }
        Message::SearchEvent(ev) => {
            // 只在搜索框编辑态处理按键(点击盒子进入编辑态后,main.rs 才把
            // 按键路由成这个变体);未进入时收到属异常,直接忽略。
            if !ws_state.search_editing {
                return;
            }
            match ev {
                AddrEvent::Text(s) => ws_state.tree_search.push_str(&s),
                AddrEvent::Backspace => {
                    ws_state.tree_search.pop();
                }
                AddrEvent::Cancel => ws_state.search_editing = false,
                AddrEvent::Submit => {
                    ws_state.search_query = ws_state.tree_search.clone();
                    ws_state.search_editing = false;
                }
            }
        }
        Message::SearchSubmit => {
            ws_state.search_query = ws_state.tree_search.clone();
        }
```

改成:

```rust
        Message::SearchInput(s) => {
            ws_state.tree_search = s;
        }
        Message::SearchSubmit => {
            ws_state.search_query = ws_state.tree_search.clone();
        }
```

- [ ] **Step 6: `search_field_id`/`CaptureSearchFocus` 新增(紧跟
  `FilesToolbarTarget` 枚举之后,约第 208 行之后)**

```rust
/// 搜索框稳定的 iced widget id:`view()` 里 `.id()` 挂给真正的
/// `text_input`,`CaptureSearchFocus` 每帧靠它在 widget 树里认出这一个
/// (同 `extensions::todo::add_field_id` 的既有手法——`Id::new` 而非
/// `Id::unique()`,保证同一个字符串在两处各自构造出的 `Id` 相等)。
pub fn search_field_id() -> Id {
    Id::new("files-search-box")
}

static SEARCH_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走(非消费——值留在 `static` 里直到下次 `CaptureSearchFocus` 覆盖)
/// 搜索框上一帧是否持有 iced 内部真实焦点。`main.rs` 渲染循环每帧跑完
/// `CaptureSearchFocus` 后立刻调用本函数,把结果塞进当前 `Workspace`
/// (`set_search_focused`)——`static` 只是临时桥接,不是长期状态存放处
/// (同 `extensions::todo::take_add_field_bounds` 的既有手法)。
pub fn take_search_focused() -> bool {
    *SEARCH_FOCUSED.lock().unwrap()
}

/// 每帧 `interface.operate()` 跑一遍,把 `search_field_id()` 命中的
/// `text_input` 当前是否持有 iced 焦点写进 `SEARCH_FOCUSED`。`traverse`
/// 留空——本仓库的渲染循环直接调 `UserInterface::operate()`,它走的是
/// widget 树自己的递归 `operate()` 实现,不经过 `Operation::traverse`
/// (同 `extensions::todo::CaptureFieldBounds` 的既有手法与注释)。
pub struct CaptureSearchFocus;
impl Operation<()> for CaptureSearchFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&search_field_id()) {
            *SEARCH_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, _: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {}
}
```

- [ ] **Step 7: `view()` 里替换搜索框构造**

约第 871-882 行当前:

```rust
    // 文件树搜索框:按文件/目录名称筛选整棵树(大小写不敏感子串匹配)。
    // 不会边输入边过滤——敲回车/点右侧"搜索"按钮后,由 `SearchSubmit` 把
    // 草稿落成为生效的 `search_query`。这是自绘输入(同树内行编辑/验收意见
    // 框):键盘走 main.rs 拦截层路由成 `SearchEvent`,不用 iced 原生
    // text_input——原生输入无法让 main.rs 知道它挂在焦点上,打字会同时漏进
    // 已聚焦的终端(本项目所有文本输入都为此自绘,理由一致)。
    let search_active = !ws_state.search_query.is_empty();
    let search_box = search_box_widget(
        &ws_state.tree_search,
        ws_state.search_editing,
        search_active,
    );
```

改成:

```rust
    // 文件树搜索框:按文件/目录名称筛选整棵树(大小写不敏感子串匹配)。
    // 不会边输入边过滤——敲回车/点右侧"搜索"按钮后,由 `SearchSubmit` 把
    // 草稿落成为生效的 `search_query`。Stage 2 迁移成真正的
    // `iced_widget::text_input`:鼠标点击聚焦、方向键/选区/IME 全部走 iced
    // 标准管线自己处理,`main.rs` 只需要每帧问一遍它是否持有真实焦点
    // (`files_search_focused`)决定要不要把键盘事件放行,不再需要点击盒子
    // 手动进入自绘编辑态。`highlight` 传 `search_active`:即使当前没聚焦,
    // 只要树被搜索词过滤中就持续金框提示。
    let search_active = !ws_state.search_query.is_empty();
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

- [ ] **Step 8: 删除 `search_box_widget`**

```bash
command grep -n "fn search_box_widget" -A40 crates/dozer-app/src/extensions/files.rs
```

把整个函数(含文档注释,约第 1483-1526 行)删除。

- [ ] **Step 9: 编译确认**

Run: `cargo build -p dozer-app --bin dozer 2>&1 | grep "error\[" `
Expected: 报 `search_editing`/`SearchEditStart`/`SearchEvent`/
`search_box_widget` 相关的未定义错误(测试模块与 app.rs/workspace.rs 里
的调用点,Task 2 Step 10 与 Task 3 分别修)。此步只确认 files.rs 自身逻辑
改动的编译面貌,不必等全绿。

- [ ] **Step 10: 重写测试**

约第 1854-1949 行两个测试(`search_input_then_submit_commits_draft_and_reset_clears`/
`search_editing_flag_and_cancel_work`)整体替换为:

```rust
    #[tokio::test]
    async fn search_input_then_submit_commits_draft_and_reset_clears() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();

        // text_input::on_input 每次给全量当前字符串,不是逐字符追加。
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchInput("main".to_string()),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_search, "main");
        assert!(ws_state.search_query.is_empty());

        // 提交(点右侧"搜索"按钮,或 iced text_input::on_submit 触发的
        // Enter):草稿落成为生效过滤词。
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchSubmit,
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.search_query, "main");

        // 认领其它项目时清空草稿、生效词与焦点镜像,避免旧筛选残留在新
        // 项目树上。
        ws_state.reset_for_project(FileTree::new(dir.path().to_path_buf()));
        assert!(ws_state.tree_search.is_empty() && ws_state.search_query.is_empty());
        assert!(!ws_state.search_focused());
    }

    #[test]
    fn search_field_id_is_stable_across_calls() {
        // `CaptureSearchFocus` 靠 `search_field_id()` 在两处(view() 的
        // `.id()` 与每帧焦点查询)各自构造出的 `Id` 相等来认出同一个字段,
        // 这个前提必须成立。
        assert_eq!(search_field_id(), search_field_id());
    }

    #[test]
    fn set_search_focused_updates_accessor() {
        let mut ws_state = WorkspaceState::default();
        assert!(!ws_state.search_focused());
        ws_state.set_search_focused(true);
        assert!(ws_state.search_focused());
        ws_state.set_search_focused(false);
        assert!(!ws_state.search_focused());
    }
```

- [ ] **Step 11: 跑 files 测试确认通过**

Run: `cargo test -p dozer-app --bin dozer files:: -- --nocapture`
Expected: 全部 PASS(含新增的 3 个测试)。

- [ ] **Step 12: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | grep files.rs; cargo fmt --package dozer-app`
Expected: files.rs 无新增警告(其它文件的编译错误留给 Task 3,此步只关注
本文件)。

- [ ] **Step 13: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/files.rs
git commit -m "feat(dozer-app): Files 搜索框改用真正的 iced text_input"
```

---

## Task 3: `main.rs` 接入原生焦点查询 + 键盘路由放行

**Files:**
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/workspace.rs`
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes: `extensions::files::{CaptureSearchFocus, take_search_focused,
  search_focused, set_search_focused}`(Task 2)。
- Produces: `App::files_search_focused(&self) -> bool` /
  `App::set_files_search_focused(&mut self, bool)`;
  `Workspace::files_search_focused(&self) -> bool`(取代旧的
  `Workspace::search_editing`/`App::search_editing`)。

- [ ] **Step 1: 确认当前调用点(核对行号)**

```bash
command grep -n "search_editing\|fn left_view\|PanelKind::Todo\|CaptureFieldBounds" crates/dozer-app/src/app.rs crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
```

- [ ] **Step 2: `workspace.rs` 访问器改名**

约第 2078-2080 行当前:

```rust
    pub fn search_editing(&self) -> bool {
        self.files.search_editing()
    }
```

改成:

```rust
    pub fn files_search_focused(&self) -> bool {
        self.files.search_focused()
    }
```

约第 2109-2116 行 `blur_inputs()` 里当前:

```rust
    pub fn blur_inputs(&mut self) {
        if self.browser.addr_editing() {
            self.browser.addr_cancel();
        }
        self.acceptance.clear_comment_editing();
        self.files.cancel_tree_edit();
        self.files.cancel_search_edit();
        self.todo.cancel_search_edit();
```

删掉 `self.files.cancel_search_edit();` 这一行(方法已在 Task 2 删除——
搜索框不再需要点击别处手动清编辑态,`CaptureSearchFocus` 下一帧自然会把
真实焦点丢失同步进来):

```rust
    pub fn blur_inputs(&mut self) {
        if self.browser.addr_editing() {
            self.browser.addr_cancel();
        }
        self.acceptance.clear_comment_editing();
        self.files.cancel_tree_edit();
        self.todo.cancel_search_edit();
```

- [ ] **Step 3: `app.rs` 访问器改名 + 新增 setter**

约第 3235-3238 行当前:

```rust
    pub fn search_editing(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.search_editing())
    }
```

改成(改名 + 紧接着加 setter,setter 镜像 `advance_todo_flash` 那种
"拿到当前工作区就写,拿不到就什么都不做"写法):

```rust
    pub fn files_search_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.files_search_focused())
    }

    /// 每帧渲染循环调用:把 `extensions::files::CaptureSearchFocus` 问到
    /// 的真实焦点态写进当前工作区(`main.rs` 键盘路由随后读
    /// `files_search_focused` 消费)。
    pub fn set_files_search_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.files.set_search_focused(focused);
        }
    }
```

- [ ] **Step 4: `main.rs` 渲染循环里每帧查询焦点**

```bash
command grep -n "CaptureFieldBounds" -B3 -A6 crates/dozer-app/src/main.rs
```

在现有(约第 2194-2199 行):

```rust
                                if matches!(app.left_view(), crate::app::PanelKind::Todo) {
                                    interface.operate(
                                        renderer,
                                        &mut extensions::todo::CaptureFieldBounds,
                                    );
                                }
```

之后紧接着加:

```rust
                                // Files 搜索框(Stage 2,唯一已迁移到 iced
                                // 原生 text_input 的字段)每帧查一遍真实
                                // 焦点态——旧版靠 `SearchEditStart` 手动
                                // 置位的 bool 已随迁移废弃,main.rs 键盘
                                // 路由改成"每帧问 iced 真相"而不是自己维护
                                // 一份可能脱节的镜像。切走 Files 左栏时显式
                                // 置 false,避免残留上一次的 true(Files 不
                                // 可见时 view() 里没有这个 text_input,
                                // CaptureSearchFocus 找不到匹配 id,不会自己
                                // 覆盖成 false)。
                                if matches!(app.left_view(), crate::app::PanelKind::Files) {
                                    interface.operate(
                                        renderer,
                                        &mut extensions::files::CaptureSearchFocus,
                                    );
                                    app.set_files_search_focused(
                                        extensions::files::take_search_focused(),
                                    );
                                } else {
                                    app.set_files_search_focused(false);
                                }
```

- [ ] **Step 5: 键盘路由新增放行判断**

```bash
command grep -n "FocusIntent::Preview(kind) = \*current_focus" -A6 crates/dozer-app/src/main.rs
command grep -n "let to_browser = app.browser_addr_editing" crates/dozer-app/src/main.rs
```

在现有(约第 1018-1022 行)Preview 原生编辑器闸门:

```rust
            if let FocusIntent::Preview(kind) = *current_focus
                && app.active_preview_tab_has_native_editor(kind)
            {
                return;
            }
```

之后、`let to_browser = app.browser_addr_editing();`(约第 1029 行)之前,
插入新闸门:

```rust
            // Files 搜索框(Stage 2,唯一已迁移到 iced 原生 text_input 的
            // 字段):不再手工路由成 `SearchEvent`,命中就直接放行给标准
            // iced 事件转换管线,交真正的 text_input 自己处理光标/选区/
            // IME(同上面 Preview 原生编辑器那道闸门的手法)。必须放在下面
            // `to_self_drawn_input` 判断之前——未来某个自绘面板与它同时报
            // "编辑态为真"时,不能让自绘分支抢先吞掉按键;也必须在 ⌘ 组合键
            // 判断(下方 `modifiers.super_key()` 分支)之前,否则 ⌘V 粘贴会
            // 被错误地转发进终端而不是交给 text_input 自己内置的粘贴处理。
            if app.files_search_focused() {
                return;
            }
```

- [ ] **Step 6: 从 `to_self_drawn_input`/`addr_message` 里去掉 Files 搜索框**

约第 1033 行当前:

```rust
            let to_search = app.search_editing();
```

整行删除。

约第 1040-1050 行 `to_self_drawn_input` 的 OR 链当前:

```rust
            let to_self_drawn_input = to_browser
                || to_comment
                || to_tree_edit
                || to_project_name
                || to_search
                || to_search_popup
                || to_todo_search
                || to_todo_add
                || to_todo_content
                || to_todo_markdown
                || to_home_project_search;
```

删掉 `|| to_search`:

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

约第 1059-1083 行 `addr_message` 闭包当前:

```rust
            let addr_message = |ev: workspace::AddrEvent| -> Message {
                if to_search_popup {
                    Message::Search(extensions::search::Message::QueryEvent(ev))
                } else if to_browser {
                    Message::Browser(extensions::browser::Message::AddrEvent(ev))
                } else if to_comment {
                    Message::Acceptance(extensions::acceptance::Message::CommentEvent(ev))
                } else if to_tree_edit {
                    Message::Files(extensions::files::Message::EditEvent(ev))
                } else if to_project_name {
                    Message::Project(extensions::project::Message::NameEditEvent(ev))
                } else if to_search {
                    Message::Files(extensions::files::Message::SearchEvent(ev))
                } else if to_todo_search {
```

删掉 `else if to_search { ... }` 这一支(`Message::Files(...::SearchEvent(..))`
变体已在 Task 2 删除):

```rust
            let addr_message = |ev: workspace::AddrEvent| -> Message {
                if to_search_popup {
                    Message::Search(extensions::search::Message::QueryEvent(ev))
                } else if to_browser {
                    Message::Browser(extensions::browser::Message::AddrEvent(ev))
                } else if to_comment {
                    Message::Acceptance(extensions::acceptance::Message::CommentEvent(ev))
                } else if to_tree_edit {
                    Message::Files(extensions::files::Message::EditEvent(ev))
                } else if to_project_name {
                    Message::Project(extensions::project::Message::NameEditEvent(ev))
                } else if to_todo_search {
```

上方约第 1051-1058 行的优先级说明注释里提到"文件树搜索框"的措辞,同步删掉
那一项(其余顺序不变)。

- [ ] **Step 7: 全量编译确认**

Run: `cargo build --workspace 2>&1 | grep "error\[" `
Expected: 无输出(全绿)。若还有残留 `search_editing`/`SearchEditStart`/
`SearchEvent` 引用报错,回到对应文件核实是否漏改。

- [ ] **Step 8: 跑 dozer-app 全量测试**

Run: `cargo test -p dozer-app --bin dozer 2>&1 | tail -30`
Expected: 全绿(已知基线:`git_log` 分支名 dogfood 环境脆弱性失败不算本
计划引入)。

- [ ] **Step 9: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets && cargo fmt --package dozer-app`
Expected: 无新增警告;fmt 无残留改动(若有,一并提交)。

- [ ] **Step 10: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): main.rs 键盘路由为 Files 搜索框新增原生输入放行判断"
```

---

## Task 4: 全量校验 + 人工 GUI 走查 + 提请审阅

**Files:** 无新增修改,本任务只跑校验与人工验证。

- [ ] **Step 1: 全量构建 + 测试 + lint**

```bash
cargo build --workspace
cargo test -p byteui
cargo test -p dozer-app --bin dozer
cargo clippy --all-targets
cargo fmt --check
```

记录实际 pass/fail 数字,只允许已知的 `git_log` 基线失败。

- [ ] **Step 2: 人工 GUI 走查(不可替代,自动化测不出这些)**

跑 `cargo run -p dozer-app`,打开一个有文件的项目,点开 Files 左栏,
逐条核对文件树搜索框:

1. 真光标显示(不再是假的 "▏" 字符,是 iced 原生闪烁光标)。
2. 方向键(←→/Home/End)能在已输入文字中移动光标(旧版完全不支持,这是
   本次新增能力,重点验证)。
3. 鼠标点击输入框内任意位置能把光标定位到点击处的字符间隙。
4. 鼠标拖拽能选中文字(选中态有背景高亮)。
5. Shift+方向键能扩展/收缩选区。
6. 敲回车直接提交过滤词(不用再点右侧"搜索"图标按钮,虽然按钮仍可用)。
7. 输入法(拼音)候选框跟随光标出现,拼词过程中的字符实时显示在光标处。
8. **关键回归项**:右侧终端 tab 正在跑一个交互进程(例如打开一个 agent
   会话)时,在搜索框里打字/⌘V 粘贴/按方向键,均不漏进终端——终端内容
   不应该出现搜索框里输入的字符。
9. 输入词后搜索到结果、再点击文件树空白处或切到其它面板,搜索框边框
   金色高亮应保持(`highlight` 由 `search_query` 非空驱动,与是否聚焦
   无关)——清空搜索词后金色高亮应消失。
10. 切换项目(顶栏项目页签)后,搜索框草稿与过滤词都应清空。

- [ ] **Step 3: 确认全部 commit 都在特性分支上**

Run: `git log --oneline main..HEAD`(在
`feature/native-text-input-stage2` 分支上执行)
Expected: 列出 Task 1-3 的三个 commit,`git branch --show-current` 不是
`main`。

- [ ] **Step 4: 提请审阅**

不在这一步直接合并——按仓库惯例提请代码审阅,通过后再合并回 `main`
(参考 `superpowers:finishing-a-development-branch`)。审阅通过合并后,
Stage 3(浏览器地址栏迁移,或首页项目搜索框/Todo 添加框——留给下一次开工
前按当时 main 决定顺序)的 plan 待开工前基于合并后的 main 重新核对行号
再写;同时可以评估要不要把 Task 3 新增的"每帧焦点查询"模式抽成一个可复用
的小工具函数(目前只有 Files 一个消费方,YAGNI,先不抽)。
