# 输入框改用 iced 原生控件 Stage 4(首页搜索框 + Todo 搜索框 + Todo 添加框)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把首页项目搜索框、Todo 面板搜索框(过滤任务列表)、Todo 添加框
(多行自增高)三个自绘输入全部改成真正的 iced 原生控件,完成原 spec 目标 1
的全部 4 个字段迁移(Files 搜索框/浏览器地址栏已在 Stage 2/3 落地),并
真正达成 spec 目标 5 的清理项——删除 `crate::search_box::view`/
`SearchBoxColors`(**注意:spec 原文把这条清理的触发条件写成"Todo 添加框
与首页项目搜索框都迁移后",经 grep 核实是笔误——`search_box::view` 真正
的两个调用方是首页项目搜索框与 Todo 面板自己的搜索框`todo_search_bar`
,不是 Todo 添加框(添加框从来是完全独立的手写多行框)。Todo 搜索框
原本不在 spec 的 4 个目标字段之列,也不在"非目标"清单里,是 spec 的一处
范围疏漏,本计划顺带把它并进来一起做完,经用户确认**)。

**Architecture:** 三个字段用两种不同的落地方式,不是同一套模板复制三次:

1. **Todo 搜索框**:形状与 Files 搜索框(Stage 2)几乎一致(单行、
   `byteui::theme::color` 工作区调色板、无预填/无手动调高),直接复用
   `byteui::form::input_text`,复制 Stage 2 的"每帧查询 iced 真实焦点"
   模式(新增 `CaptureTodoSearchFocus`,`traverse` 从一开始写对)。
2. **首页项目搜索框**:**不走 `byteui::form::input_text`**——这个字段用的
   是 `theme::homespace_color`(首页自己独立维护的调色板,与工作区
   `byteui::theme::color` 刻意分开,`homespace_*` 系列没有并入 byteui 的
   计划),而 `byteui::form::input_text` 内部写死 `byteui::theme::color::
   current()`。给 byteui 加一个只有这一处用得到的调色板覆盖参数不划算
   (会让本来就快到 8 参数的签名更臃肿),改成直接在 `homespace.rs` 里手写
   一个 `iced_widget::text_input(...)`,样式手法照抄 `byteui::form::
   input_text` 的写法(卡片底色+描边,聚焦金框由 `Status::Focused` 驱动),
   只是配色源换成 `theme::homespace_color`——这是本计划唯一一处不复用
   byteui 组件的地方,原因记在这里,不是遗漏。焦点查询模式(`Capture
   HomeSearchFocus`)与 Todo 搜索框同款。
3. **Todo 添加框**:唯一的多行字段,消费 Stage 1 建好但至今零调用点的
   `byteui::form::text_area`。这个字段有一个前两个阶段都没遇到的真实
   产品功能——顶部拖拽手柄可手动调高(`add_input_height`,120~400px,
   拖拽逻辑在 `app.rs::RowDrag` 里,本计划不碰那部分),与 `text_area`
   目前"不显式设高、纯内容自增高"的设计冲突。给 `text_area::view` 加一个
   可选定高参数(`height: Option<f32>`),传入时用 `Length::Fixed`,不传
   时保持原有 `Length::Shrink` 行为——不删除手动调高功能。同时补齐
   `id`/`bare` 两个参数(id 供焦点查询用,bare 因为提交按钮嵌在同一个
   边框内,同浏览器地址栏 Stage 3 的道理)。

三个字段共用的机制(与 Stage 2/3 完全一致,不重新设计):
`main.rs` 每帧 `interface.operate()` 查真实焦点态 → 写进对应
`WorkspaceState`/`App` 字段 → 键盘拦截链新增的原生输入放行闸门读它决定要不要
放行给标准 iced 事件转换管线。鼠标点击定位光标(Todo 添加框原有的
`AddCursorAt`/`CaptureFieldBounds::add_field_id` 那套"字段屏幕 bounds→
局部 x→字符下标"机制)随迁移整体作废——真正的 `text_editor` 自己处理点击
定位,不需要这套了(`content_field_id` 保留,`content_field_id` 是任务
内容行内编辑用的,不在本计划范围)。

**Tech Stack:** Rust 2024,iced 0.14(`iced_widget::text_input`/
`text_editor`,`iced_core::widget::operation::focusable::Focusable`)。

**Spec:** `docs/superpowers/specs/2026-08-20-native-text-input-adoption-design.md`
(本计划实现该 spec 目标 1 剩余的 2 个字段——首页项目搜索框、Todo 添加框——
以及一个未列入 spec 但经核实后并入本计划的字段 Todo 搜索框;目标 3 键盘
路由放行判断在这三个字段上的落地;目标 5 清理项——本计划做完后
`search_box::view`/`SearchBoxColors` 真正可以删,`move_cursor_in`/
`insert_at_cursor`/`delete_before_cursor`/`char_to_byte`/`draft_with_
caret`/`CursorDir` 等光标数学自由函数仍被 Todo 任务内容编辑/MARKDOWN 整
文件编辑依赖,不能删)。

## Global Constraints

- **在独立分支上开发,不直接提交 main。** 本仓库有长驻自动化在 main 上持续
  开发(建 worktree 前用 `git -C /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
  worktree list` 确认没有同名 worktree),从 main 当前 tip 拉新 worktree:
  `git worktree add ../dozer-native-text-input-stage4 -b
  feature/native-text-input-stage4 main`。后续所有命令都在这个新 worktree
  目录里跑,每次 `git commit`/`git add` 前先 `git branch --show-current`
  确认不在 `main` 上。
- **本计划的行号引用比前几个 Stage 更容易过时**:写作时 Stage 3(浏览器
  地址栏)尚未执行/合并,而 Stage 3 与本计划都会改 `main.rs` 的同一段键盘
  路由放行闸门(`if app.files_search_focused() || ...`)。执行本计划前
  **必须**先确认 Stage 3 是否已合并——`git log --oneline main | grep
  "浏览器地址栏"`;若已合并,`main.rs` 里那道闸门会比本计划写作时多一个
  `app.browser_addr_focused()` 分支,照实际内容合并进去,不要覆盖掉。
  下面每个 Task 引用的行号以 2026-08-20 main tip(commit `c379a22`,
  **Stage 3 落地前**)为准,执行前一律先用对应的 `grep -n` 命令核对实际
  行号。
- **`Operation<()>` 的 `traverse` 必须实现成调用传入闭包**(`fn traverse(&mut
  self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a)))
  { operate(self); }`)——[[dozer-operation-traverse-noop-bug]] 记录的
  Critical bug 教训,本计划新增的三个 `CaptureXxxFocus` 结构体必须从一开始
  就写对。
- 每个 Task 结束都要求对应 crate `cargo build` 干净通过;涉及 `dozer-app`
  的 Task 额外要求 `cargo test -p dozer-app --bin dozer <关键字>` 通过。
- 最终 Task 要求全 workspace `cargo build && cargo test -p byteui -p
  dozer-app --bin dozer && cargo clippy --all-targets && cargo fmt --check`
  全绿,记录实际 pass/fail 数字(已知基线:`git_log` 分支名 dogfood 环境
  脆弱性失败不算本计划引入)。
- **迁移完成后的人工 GUI 走查不可省略**,Task 6 列出完整走查清单,三个
  字段逐一过,含"背景终端聚焦时不漏键"这条关键回归项。
- **已知的、经用户确认的行为变化**(同 Stage 2/3 的判断口径,不在本计划
  范围内修复):Todo 添加框/搜索框、首页搜索框迁移后,点击框内 padding
  空白区域(不在文字/光标范围内)不再能进入编辑态,只有点在输入控件自身
  命中范围内才行。

---

## Task 1: `byteui::form::text_area` 补齐 `id`/`bare`/`height`

**Files:**
- Modify: `crates/byteui/src/form/text_area.rs`
- Modify: `crates/byteui/src/form/mod.rs`(既有测试调用点)

**Interfaces:**
- Produces: `text_area::view` 新签名——在 `placeholder` 之后插入
  `id: Option<widget::Id>`、`bare: bool`,在 `on_action` 之前插入
  `height: Option<f32>`。新签名:`view(content, placeholder, id, bare,
  height, on_action)`。`id` 供外部焦点查询用;`bare` 为真时不画自己的
  卡片背景/边框,由调用方外层容器接管(同 `input_text` Stage 3 的
  `bare`);`height` 为 `Some(h)` 时用 `Length::Fixed(h)`,为 `None` 时
  保持现状的 `Length::Shrink`(不设置 `.height()`)。目前唯一调用方是
  Task 3 的 Todo 添加框,传 `id: Some(..)`、`bare: true`、
  `height: Some(ws_state.add_input_height())`。

- [x] **Step 1: 确认当前签名**

```bash
command grep -n "pub fn view" crates/byteui/src/form/text_area.rs
```

预期看到 Stage 1 定的 3 参数签名(`content`/`placeholder`/`on_action`)。

- [x] **Step 2: 写签名变化的失败断言**

`crates/byteui/src/form/mod.rs` 里把:

```rust
        let content = iced_widget::text_editor::Content::new();
        let _ = text_area::view(&content, "placeholder", Msg::Edited);
```

改成:

```rust
        let content = iced_widget::text_editor::Content::new();
        let _ = text_area::view(&content, "placeholder", None, false, None, Msg::Edited);
```

- [x] **Step 3: 跑测试确认失败**

Run: `cargo test -p byteui all_form_components -- --nocapture`
Expected: 编译失败(`text_area::view` 目前只有 3 参数)。

- [x] **Step 4: 改 `text_area::view` 签名**

`crates/byteui/src/form/text_area.rs` 当前完整内容:

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

整体替换为:

```rust
//! 多行自增高文本编辑框——包一层 `iced_widget::text_editor`,样式对齐
//! `form::input_text`(卡片底色+描边,聚焦金框由 iced 内置 `Status::Focused`
//! 驱动)。`height` 为 `None` 时用 iced text_editor 的默认 `Length::Shrink`,
//! 随内容自然撑高;为 `Some(h)` 时用 `Length::Fixed(h)`——供需要"手动拖拽
//! 定高,内容超出走内部滚动"的调用方使用(如 Todo 添加框的拖拽调高手柄)。

use iced_widget::core::{Border, Element, Length, widget};
use iced_widget::text_editor::{self, Status};

pub fn view<'a, Message: Clone + 'a>(
    content: &'a text_editor::Content,
    placeholder: &'a str,
    id: Option<widget::Id>,
    bare: bool,
    height: Option<f32>,
    on_action: impl Fn(text_editor::Action) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let editor = iced_widget::text_editor(content)
        .placeholder(placeholder)
        .on_action(on_action)
        .size(crate::theme::font::body())
        .padding(8);
    let editor = if let Some(id) = id {
        editor.id(id)
    } else {
        editor
    };
    let editor = if let Some(h) = height {
        editor.height(Length::Fixed(h))
    } else {
        editor
    };
    editor
        .style(move |_theme: &iced_widget::Theme, status: Status| {
            let colors = crate::theme::color::current();
            let focused = matches!(status, Status::Focused { .. });
            if bare {
                return text_editor::Style {
                    background: iced_widget::core::Color::TRANSPARENT.into(),
                    border: Border {
                        color: iced_widget::core::Color::TRANSPARENT,
                        width: 0.0,
                        radius: 0.0.into(),
                    },
                    placeholder: colors.dim,
                    value: colors.cream,
                    selection: crate::theme::color::mix(colors.gold, colors.card, 0.6),
                };
            }
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

- [x] **Step 5: 跑测试确认通过**

Run: `cargo test -p byteui all_form_components -- --nocapture`
Expected: PASS。

- [x] **Step 6: 构建 + clippy + fmt**

Run: `cargo build -p byteui && cargo clippy -p byteui --all-targets && cargo fmt --package byteui`
Expected: 构建通过,无新增 clippy 警告,fmt 无残留改动。

- [x] **Step 7: Commit**

```bash
git branch --show-current
git add crates/byteui/src/form/text_area.rs crates/byteui/src/form/mod.rs
git commit -m "feat(byteui): text_area 新增 id/bare/height,支持焦点查询/无边框嵌入/手动定高"
```

---

## Task 2: Todo 搜索框改用真正的 `text_input`

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`

**Interfaces:**
- Consumes: `byteui::form::input_text::view`(Stage 3 的 8 参数签名:
  `placeholder, value, secure, id, highlight, on_submit, bare, on_input`)。
- Produces: `pub fn todo_search_field_id() -> Id`;`pub struct
  CaptureTodoSearchFocus`;`pub fn take_todo_search_focused() -> bool`;
  `WorkspaceState::search_focused(&self) -> bool` /
  `set_search_focused(&mut self, bool)`(取代 `search_editing()`/
  `cancel_search_edit()`)。

- [x] **Step 1: 确认当前状态(核对行号)**

```bash
command grep -n "search_editing\|search_draft\|search_cursor\|SearchEditStart\|SearchEvent\|SearchCursorMove\|todo_search_bar" crates/dozer-app/src/extensions/todo.rs
```

- [x] **Step 2: `WorkspaceState` 字段改动**

约第 377-388 行当前:

```rust
    /// 已生效的搜索关键词(列表过滤用)。打字期间只改草稿 `search_draft`,
    /// 回车/点右侧搜索按钮才落成这里(与文件树搜索 `search_query` 同款
    /// "草稿→提交"模型——本 app 的 iced 界面每帧重建、原生 `text_input`
    /// 留不住焦点,搜索必须用自绘输入 + main.rs 键盘拦截路由,见 design)。
    search: String,
    /// 搜索框编辑态草稿。`search_editing` 为真时按键经 main.rs 路由成
    /// `SearchEvent`,只动草稿,不重新过滤;回车/点搜索按钮才提交。
    search_draft: String,
    /// 搜索框草稿的光标位置(字符下标,见 `add_cursor` 注释)。
    search_cursor: usize,
    /// 搜索框是否处于自绘编辑态(main.rs 键盘路由用)。
    search_editing: bool,
```

改成:

```rust
    /// 已生效的搜索关键词(列表过滤用)。打字期间只改草稿 `search_draft`,
    /// 回车/点右侧搜索按钮才落成这里(与文件树搜索 `search_query` 同款
    /// "草稿→提交"模型)。
    search: String,
    /// 搜索框草稿(iced `text_input` 的 `value`)。
    search_draft: String,
    /// 搜索框是否持有 iced 内部真实焦点。**不是**应用层手动置位的镜像——
    /// 每帧渲染循环里 `CaptureTodoSearchFocus` 问一遍 iced 真相后立刻写
    /// 进这里(`set_search_focused`),`main.rs` 键盘路由读它决定要不要
    /// 放行给标准 iced 管线。
    search_focused: bool,
```

(`search_cursor` 整字段删除——`text_input` 自己管理光标。)

- [x] **Step 3: 访问器改动**

约第 533-541 行当前:

```rust
    /// 搜索框是否处于自绘编辑态(main.rs 键盘路由用)。
    pub fn search_editing(&self) -> bool {
        self.search_editing
    }

    /// 失焦退出搜索编辑态(`Workspace::blur_inputs` 用):草稿保留。
    pub fn cancel_search_edit(&mut self) {
        self.search_editing = false;
    }
```

改成:

```rust
    /// 搜索框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn search_focused(&self) -> bool {
        self.search_focused
    }

    /// 每帧渲染循环读走 `CaptureTodoSearchFocus` 查到的真实焦点态后写
    /// 进来。
    pub fn set_search_focused(&mut self, focused: bool) {
        self.search_focused = focused;
    }
```

- [x] **Step 4: `Message` 枚举改动**

约第 726-734 行当前:

```rust
    /// 点搜索框进入自绘编辑态(`search_editing = true`),后续按键经 main.rs
    /// 路由成 `SearchEvent`,不再漏进终端。
    SearchEditStart,
    /// 编辑态下的按键:只动草稿 `search_draft`,不重新过滤(需提交)。
    SearchEvent(AddrEvent),
    /// 回车 / 点右侧搜索按钮:把草稿落成生效的 `search` 过滤词。
    SearchSubmit,
    /// 方向键/Home/End 移动搜索框草稿光标(字符下标),见 `AddCursorMove`。
    SearchCursorMove(CursorDir),
```

改成:

```rust
    /// 搜索框草稿变化(iced `text_input::on_input`,每次给全量当前字符串)。
    SearchInput(String),
    /// 回车 / 点右侧搜索按钮:把草稿落成生效的 `search` 过滤词。
    SearchSubmit,
```

- [x] **Step 5: `todo_search_field_id`/`CaptureTodoSearchFocus` 新增**

紧跟 `WorkspaceState` 的 `impl` 块之后新增(照抄
`extensions::files::search_field_id`/`CaptureSearchFocus` 的手法):

```rust
/// 搜索框稳定的 iced widget id。
pub fn todo_search_field_id() -> Id {
    Id::new("todo-search-box")
}

static TODO_SEARCH_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

pub fn take_todo_search_focused() -> bool {
    *TODO_SEARCH_FOCUSED.lock().unwrap()
}

/// 每帧 `interface.operate()` 跑一遍。`traverse` 必须调用传入闭包(见
/// [[dozer-operation-traverse-noop-bug]]——同 `extensions::files::
/// CaptureSearchFocus` 修复过的手法,这里从一开始就写对)。
pub struct CaptureTodoSearchFocus;
impl Operation<()> for CaptureTodoSearchFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&todo_search_field_id()) {
            *TODO_SEARCH_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}
```

(检查文件顶部 import——`Id`/`Operation`/`Rectangle`/`Focusable` 是否已经
因为 `CaptureFieldBounds` 而导入过;`command grep -n "^use" crates/dozer-app/src/extensions/todo.rs`
核对,若已有 `use iced_widget::core::widget::{Id, Operation};` 等则不用
重复加。)

- [x] **Step 6: `update()` 里的处理分支改动**

约第 1128-1154 行当前:

```rust
        Message::SearchEditStart => ws_state.search_editing = true,
        Message::SearchEvent(ev) => {
            // 编辑态之外(失焦)的 `SearchEvent` 一律忽略,避免草稿被污染。
            if !ws_state.search_editing {
                return;
            }
            match ev {
                AddrEvent::Text(s) => {
                    insert_at_cursor(&mut ws_state.search_draft, &mut ws_state.search_cursor, &s)
                }
                AddrEvent::Backspace => {
                    delete_before_cursor(&mut ws_state.search_draft, &mut ws_state.search_cursor);
                }
                AddrEvent::Cancel => ws_state.search_editing = false,
                AddrEvent::Submit => ws_state.commit_search(),
            }
        }
        Message::SearchCursorMove(dir) => {
            if !ws_state.search_editing {
                return;
            }
            move_cursor_in(&ws_state.search_draft, &mut ws_state.search_cursor, dir);
        }
        Message::SearchSubmit => {
            ws_state.commit_search();
            ws_state.search_editing = false;
        }
```

改成:

```rust
        Message::SearchInput(s) => ws_state.search_draft = s,
        Message::SearchSubmit => ws_state.commit_search(),
```

- [x] **Step 7: `todo_search_bar` 视图函数改动**

约第 1675-1703 行(先 `command grep -n "fn todo_search_bar" -A30
crates/dozer-app/src/extensions/todo.rs` 核对实际结尾行号)当前调用
`crate::search_box::view(...)`。整个函数体替换为:

```rust
fn todo_search_bar<'a>(draft: &'a str, active: bool) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    byteui::form::input_text::view(
        "搜索任务…",
        draft,
        false,
        Some(todo_search_field_id()),
        active,
        Some(Message::SearchSubmit),
        false,
        Message::SearchInput,
    )
}
```

(原函数签名里的 `app: &App`/`editing: bool`/`cursor: usize` 三个参数不再
需要——`app.hover_progress` 是给 `search_box::view` 里提交按钮的 hover
动画用的,`byteui::form::input_text` 没有内嵌提交按钮这个概念,回车/失焦
外部按钮各自独立;如果现状 UI 在搜索框旁边还有一个独立的搜索图标按钮,
保留那个按钮的调用点不动,只是不再喂给 `search_box::view` 内嵌版本——
执行前先跑 `command grep -n "todo_search_bar(" crates/dozer-app/src/extensions/todo.rs`
核对调用点参数,按实际情况调整传参,不要凭这段描述机械套用。)

- [x] **Step 8: 编译确认**

Run: `cargo build -p dozer-app --bin dozer 2>&1 | grep "error\[" `
Expected: 报测试模块与 `main.rs`/`workspace.rs`/`app.rs` 相关调用点的
未定义错误(本 Task 剩余 Step 与 Task 5 分别修)。

- [x] **Step 9: 重写测试**

`command grep -n "SearchEditStart\|SearchEvent\|SearchCursorMove\|search_editing" crates/dozer-app/src/extensions/todo.rs`
找到所有测试断言,同 Stage 2 Files 搜索框 Task 2 Step 10 的改法——
`Message::SearchEditStart` 起手式删除,`Message::SearchEvent(AddrEvent::
Text(..))` 改成 `Message::SearchInput("..".to_string())`,`assert!(ws_state
.search_editing())` 改成直接调 `ws_state.set_search_focused(true)` 再断言
`search_focused()`(不再需要真的进 iced 焦点态才能测状态转换,同 Files
搜索框 Stage 2 `set_search_focused_updates_accessor` 那个新增测试的写法)。

- [x] **Step 10: 跑测试 + clippy + fmt**

Run: `cargo test -p dozer-app --bin dozer todo:: -- --nocapture 2>&1 | tail -40 && cargo clippy -p dozer-app --all-targets 2>&1 | grep todo.rs; cargo fmt --package dozer-app`
Expected: 通过(main.rs/workspace.rs/app.rs 的残留错误留给 Task 5)。

- [x] **Step 11: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/todo.rs
git commit -m "feat(dozer-app): Todo 搜索框改用真正的 iced text_input"
```

---

## Task 3: Todo 添加框改用真正的 `text_area`

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`

**Interfaces:**
- Consumes: `byteui::form::text_area::view`(Task 1 新签名)。
- Produces: `pub fn add_field_id() -> Id`(**已存在**,签名不变,继续复用);
  `pub struct CaptureAddFocus`;`pub fn take_add_focused() -> bool`;
  `WorkspaceState::add_focused(&self) -> bool` /
  `set_add_focused(&mut self, bool)`(取代 `add_editing()`/
  `cancel_add_edit()`)。

- [x] **Step 1: 确认当前状态(核对行号)**

```bash
command grep -n "add_draft\|add_cursor\|add_editing\|AddEditStart\|AddEvent\|AddCursorMove\|AddCursorAt\|add_field_id\|CaptureFieldBounds\|commit_add_task" crates/dozer-app/src/extensions/todo.rs
```

- [x] **Step 2: `WorkspaceState` 字段改动**

约第 349-362 行当前:

```rust
    add_draft: String,
    /// 新增任务框草稿的光标位置(字符下标,非字节)。自绘输入没有原生光标,
    /// 方向键/鼠标点击都靠这个下标重定位,渲染时把草稿从光标处劈成两段、
    /// 中间塞 `▏` 当光标。`add_draft` 为空时恒为 0。
    add_cursor: usize,
    /// 新增任务框是否处于自绘编辑态。本 app 每帧重建界面,原生 `text_input`
    /// 留不住焦点、也不参与 main.rs 的键盘路由裁决,不加这个标记的话打字
    /// 会同时漏进已聚焦的终端(agent 输入),见 `todo_footer_bar`。
    add_editing: bool,
    /// 新增任务框高度(逻辑像素)。框顶的拖拽手柄向上拉时由 app 层换算写回
    /// (见 `app.rs::RowDrag` 的 `TodoAddGrow` 分支),0 表示"未拖过、用默认
    /// 高",视图侧一律 `max(ADD_INPUT_MIN_HEIGHT)` 兜底——这样 `#[derive(
    /// Default)]` 给的 0 也不会渲染成 0 高框。
    add_input_height: f32,
```

改成:

```rust
    /// 新增任务框草稿。类型从 `String` 换成 `iced_widget::text_editor::
    /// Content`(实现 `Default`/`Clone`)——真正的 `text_editor` 自己管理
    /// 光标/选区,不再需要应用层维护 `add_cursor` 字符下标。
    add_draft: iced_widget::text_editor::Content,
    /// 新增任务框是否持有 iced 内部真实焦点。**不是**应用层手动置位的
    /// 镜像——每帧渲染循环里 `CaptureAddFocus` 问一遍 iced 真相后立刻写
    /// 进这里(`set_add_focused`)。
    add_focused: bool,
    /// 新增任务框高度(逻辑像素)。框顶的拖拽手柄向上拉时由 app 层换算写回
    /// (见 `app.rs::RowDrag` 的 `TodoAddGrow` 分支,本计划不改这部分),
    /// 0 表示"未拖过、用默认高",视图侧一律 `max(ADD_INPUT_MIN_HEIGHT)`
    /// 兜底。
    add_input_height: f32,
```

- [x] **Step 3: 访问器改动**

约第 543-552 行当前:

```rust
    /// 新增任务框是否处于自绘编辑态(main.rs 键盘路由用)。
    pub fn add_editing(&self) -> bool {
        self.add_editing
    }

    /// 失焦退出新增任务编辑态(`Workspace::blur_inputs` 用):草稿保留,
    /// 与搜索框同款——半输入的任务文字不该因为点了别处就丢。
    pub fn cancel_add_edit(&mut self) {
        self.add_editing = false;
    }
```

改成:

```rust
    /// 新增任务框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn add_focused(&self) -> bool {
        self.add_focused
    }

    /// 每帧渲染循环读走 `CaptureAddFocus` 查到的真实焦点态后写进来。
    pub fn set_add_focused(&mut self, focused: bool) {
        self.add_focused = focused;
    }
```

- [x] **Step 4: `Message` 枚举改动**

约第 703-716 行当前:

```rust
    /// 点新增任务框进入自绘编辑态(`add_editing = true`),后续按键经
    /// main.rs 路由成 `AddEvent`,不再漏进终端(同 `SearchEditStart`)。
    AddEditStart,
    /// 编辑态下的按键:文本/退格改草稿,`AddrEvent::Submit` 落盘新任务。
    AddEvent(AddrEvent),
    /// 点新增任务框右侧 circle-arrow-up 提交按钮:把草稿落盘成新任务
    /// (与回车 `AddEvent(Submit)` 共用 `commit_add_task` 一条路径)。
    AddSubmit,
    /// 方向键/Home/End 移动新增任务框草稿光标(字符下标)。main.rs 把键盘
    /// 方向键翻成 `CursorDir` 经此路由,自绘输入没原生光标,全靠这个。
    AddCursorMove(CursorDir),
    /// 鼠标点击新增任务框:把字段内局部点击 x(逻辑像素)折算成字符下标,
    /// 定位光标(见 `CursorDir` 同款的"自绘输入没原生光标"背景)。
    AddCursorAt(f32),
```

改成:

```rust
    /// 新增任务框编辑事件(iced `text_editor::on_action`,真正的
    /// `Action` 由组件自己产生,应用层只负责 `content.perform(action)`
    /// 落地——光标/选区/IME 全部交给 iced,不再是"文本/退格"这种自绘
    /// 事件分解)。
    AddEdit(iced_widget::text_editor::Action),
    /// 点新增任务框右侧 circle-arrow-up 提交按钮:把草稿落盘成新任务
    /// (与 `text_editor` 内置的 Enter 换行不冲突——提交只走按钮,不认
    /// 回车,见下方 `todo_footer_bar` 的按钮说明不变)。
    AddSubmit,
```

(`AddCursorMove`/`AddCursorAt` 整体删除——`text_editor` 自己处理方向键/
鼠标点击定位,不需要应用层折算。)

- [x] **Step 5: `add_field_id`/`CaptureAddFocus` 改动**

约第 844-846 行当前(**保留不动**,签名不变):

```rust
pub fn add_field_id() -> Id {
    Id::new("todo-add-field")
}
```

约第 868-883 行 `CaptureFieldBounds`(供 `content_field_id` 鼠标点击定位
用,Todo 任务内容编辑不在本计划范围,继续保留)当前:

```rust
pub struct CaptureFieldBounds;
impl Operation<()> for CaptureFieldBounds {
    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        match id {
            Some(id) if *id == add_field_id() => {
                *ADD_FIELD_BOUNDS.lock().unwrap() = Some(bounds);
            }
            Some(id) if *id == content_field_id() => {
                *CONTENT_FIELD_BOUNDS.lock().unwrap() = Some(bounds);
            }
            _ => {}
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}
```

删掉 `add_field_id` 那个 `match` 分支(`add_field_id()` 现在挂在真正的
`text_editor` 上,`text_editor::operate()` 走的是 `operation.focusable(..)`
钩子,不是 `container`,`CaptureFieldBounds` 本来就捕获不到它,这个分支
已经是死代码——只是这次真正确认并清理):

```rust
pub struct CaptureFieldBounds;
impl Operation<()> for CaptureFieldBounds {
    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if *id.unwrap_or(&content_field_id()) == content_field_id() {
            *CONTENT_FIELD_BOUNDS.lock().unwrap() = Some(bounds);
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}
```

(上面这个改法为了少改一行结构用了 `unwrap_or` 的技巧,如果读着别扭,写成
`match id { Some(id) if *id == content_field_id() => {...} _ => {} }` 效果
一样,按实现时哪种更符合团队既有风格选,不是强制要求。)

同时删掉与 `add_field_id` 配套的 `ADD_FIELD_BOUNDS`/`take_add_field_
bounds`(约第 852-853/857-859 行),只留 `CONTENT_FIELD_BOUNDS`/
`take_content_field_bounds`。

紧接着 `add_field_id()` 之后新增(照抄 Task 2 `CaptureTodoSearchFocus` 的
手法):

```rust
static ADD_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

pub fn take_add_focused() -> bool {
    *ADD_FOCUSED.lock().unwrap()
}

/// 每帧 `interface.operate()` 跑一遍。`traverse` 必须调用传入闭包(见
/// [[dozer-operation-traverse-noop-bug]])。
pub struct CaptureAddFocus;
impl Operation<()> for CaptureAddFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&add_field_id()) {
            *ADD_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}
```

- [x] **Step 6: `commit_add_task` 改动**

约第 917-946 行当前:

```rust
fn commit_add_task(ws_state: &mut WorkspaceState, project_path: &std::path::Path) {
    let text = ws_state.add_draft.trim().to_string();
    if text.is_empty() {
        return;
    }
    let path = todo_path(project_path);
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let new_content = prepend_todo_item(&content, &text);
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!("创建 .dozer 目录失败: {e}");
        return;
    }
    if let Err(e) = std::fs::write(&path, &new_content) {
        tracing::warn!("写入 todo.md 失败: {e}");
        return;
    }
    ws_state.add_draft.clear();
    ws_state.add_cursor = 0;
    reload_from_disk(ws_state, project_path);
```

把 `let text = ws_state.add_draft.trim().to_string();` 改成:

```rust
    let text = ws_state.add_draft.text();
    let text = text.trim().to_string();
```

把 `ws_state.add_draft.clear(); ws_state.add_cursor = 0;` 改成:

```rust
    ws_state.add_draft = iced_widget::text_editor::Content::new();
```

其余逻辑不动。

- [x] **Step 7: `update()` 里的处理分支改动**

约第 1066-1103 行当前:

```rust
        Message::AddEditStart => ws_state.add_editing = true,
        Message::AddEvent(ev) => {
            // 编辑态之外(失焦)的 `AddEvent` 一律忽略,避免草稿被污染
            // (同 `SearchEvent` 的既有约定)。
            if !ws_state.add_editing {
                return;
            }
            match ev {
                AddrEvent::Text(s) => {
                    insert_at_cursor(&mut ws_state.add_draft, &mut ws_state.add_cursor, &s)
                }
                AddrEvent::Backspace => {
                    delete_before_cursor(&mut ws_state.add_draft, &mut ws_state.add_cursor);
                }
                AddrEvent::Cancel => ws_state.add_editing = false,
                AddrEvent::Submit => commit_add_task(ws_state, project_path),
            }
        }
        Message::AddCursorMove(dir) => {
            if !ws_state.add_editing {
                return;
            }
            move_cursor_in(&ws_state.add_draft, &mut ws_state.add_cursor, dir);
        }
        Message::AddCursorAt(local_x) => {
            if !ws_state.add_editing {
                return;
            }
            ws_state.add_cursor = cursor_from_x(
                &ws_state.add_draft,
                local_x,
                byteui::theme::font::body() as f32,
            );
        }
        Message::AddSubmit => commit_add_task(ws_state, project_path),
```

改成:

```rust
        Message::AddEdit(action) => ws_state.add_draft.perform(action),
        Message::AddSubmit => commit_add_task(ws_state, project_path),
```

- [x] **Step 8: `todo_footer_bar` 视图函数改动**

约第 1503-1592 行(先 `command grep -n "fn todo_footer_bar" -A100
crates/dozer-app/src/extensions/todo.rs` 核对实际范围)当前用
`ws_state.add_editing`/`add_draft`(String)/`draft_with_caret` 手写文字 +
`MouseArea` 包一层进编辑态。改成:

```rust
fn todo_footer_bar<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let field = byteui::form::text_area::view(
        &ws_state.add_draft,
        "添加新任务",
        Some(add_field_id()),
        true,
        Some(ws_state.add_input_height()),
        Message::AddEdit,
    );

    // 提交按钮:circle-arrow-up,嵌在输入框右边框内、无独立边框(视觉上"在
    // 框里")。图标配色对齐其它 icon 按钮(agent 面板"＋"、文件树搜索等):
    // 静止 DIM、hover 平滑过渡到 GOLD。
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

    // 输入框本体:单个带边框的容器,把"文字区 + 提交按钮"一起包进边框内。
    // 不再需要外层 `MouseArea`/`AddEditStart`——`text_editor` 是真控件,
    // 点击命中范围内就由 iced 标准鼠标管线自己处理聚焦。高度仍可经顶部
    // 拖拽手柄放大(`add_input_height`,拖拽逻辑在 `app.rs::RowDrag`,
    // 本计划不改),`text_area` 的 `height` 参数按这个值定高。内部一行
    // 两格:左格文字(占满高度、靠顶左对齐)、右格提交按钮(占满高度、靠底)。
    let editing = ws_state.add_focused();
    container(
        row![
            container(field)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(iced_widget::core::alignment::Vertical::Top)
                .align_x(iced_widget::core::alignment::Horizontal::Left),
            container(submit)
                .height(Length::Fill)
                .align_y(iced_widget::core::alignment::Vertical::Bottom),
        ]
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fixed(ws_state.add_input_height()))
    .padding([10, 12])
    .style(move |_t: &iced_widget::Theme| container::Style {
        background: Some(byteui::theme::color::current().bg.into()),
        border: Border {
            color: if editing {
                byteui::theme::color::current().gold
            } else {
                byteui::theme::color::current().border
            },
            width: 1.0,
            radius: 4.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}
```

(执行前先跑 `command grep -n "fn todo_footer_bar" -A100
crates/dozer-app/src/extensions/todo.rs` 核对函数结尾——原函数体在这个
`input_box` 构造之后可能还有其它内容如"输入框自身已带 1px 边框,作为与
列表区之间的唯一分割线"那段注释提到的后续布局,原样保留、只替换本 Step
覆盖的这一段,不要整个函数体照抄本计划的版本生吞下去。)

- [x] **Step 9: 编译确认**

Run: `cargo build -p dozer-app --bin dozer 2>&1 | grep "error\[" `
Expected: 报测试模块与 `main.rs`/`workspace.rs`/`app.rs` 相关调用点的
未定义错误(本 Task 剩余 Step 与 Task 5 分别修)。

- [x] **Step 10: 重写测试**

`command grep -n "AddEditStart\|AddEvent\|AddCursorMove\|AddCursorAt\|add_editing" crates/dozer-app/src/extensions/todo.rs`
找到所有测试断言。改法同 Task 2 Step 9:`Message::AddEvent(AddrEvent::
Text(s))` 改成对应的 `text_editor::Action`(插入文字用
`text_editor::Action::Edit(text_editor::Edit::Insert(ch))` 按字符逐个插,
或更简单地直接调 `ws_state.add_draft = text_editor::Content::with_text(
"新任务")` 绕开逐字符 `Action` 拼装,只要测试断言的是"提交后文件内容/
`draft` 状态",不强求走 `Action` 管线——`Action` 管线本身的正确性是
iced 自己的职责,不是本仓库要测的);`assert!(ws_state.add_editing())`
改成 `ws_state.set_add_focused(true)` 再断言 `add_focused()`。

- [x] **Step 11: 跑测试 + clippy + fmt**

Run: `cargo test -p dozer-app --bin dozer todo:: -- --nocapture 2>&1 | tail -60 && cargo clippy -p dozer-app --all-targets 2>&1 | grep todo.rs; cargo fmt --package dozer-app`
Expected: 通过(main.rs/workspace.rs/app.rs 的残留错误留给 Task 5)。

- [x] **Step 12: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/todo.rs
git commit -m "feat(dozer-app): Todo 添加框改用真正的 iced text_editor(text_area)"
```

---

## Task 4: 首页项目搜索框改用真正的 `text_input`(手写,不经 byteui)

**Files:**
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/homespace.rs`

**Interfaces:**
- Produces: `pub(crate) fn home_search_field_id() -> Id`(建议放
  `homespace.rs`);`pub struct CaptureHomeSearchFocus`;`pub fn
  take_home_search_focused() -> bool`;`App::home_project_search_focused
  (&self) -> bool` / `set_home_project_search_focused(&mut self, bool)`
  (取代 `home_project_search_editing()`)。

- [x] **Step 1: 确认当前状态(核对行号)**

```bash
command grep -n "home_project_search" crates/dozer-app/src/app.rs crates/dozer-app/src/homespace.rs
```

- [x] **Step 2: `App` 状态字段改动**

约第 2378-2387 行当前:

```rust
    pub(crate) home_project_search: String,
    /// 回车/点搜索按钮才落成 `home_project_search`。
    pub(crate) home_project_search_draft: String,
    home_project_search_editing: bool,
    home_project_search_cursor: usize,
```

改成:

```rust
    pub(crate) home_project_search: String,
    /// 回车/点搜索按钮才落成 `home_project_search`。
    pub(crate) home_project_search_draft: String,
    /// 是否持有 iced 真实焦点。**不是**应用层手动置位的镜像——每帧渲染
    /// 循环里 `CaptureHomeSearchFocus` 问一遍 iced 真相后立刻写进这里
    /// (`set_home_project_search_focused`)。
    home_project_search_focused: bool,
```

(`home_project_search_cursor` 整字段删除——`text_input` 自己管理光标。)

约第 2703-2706 行构造初始值处对应删掉 `home_project_search_cursor: 0,`,
`home_project_search_editing: false,` 改成
`home_project_search_focused: false,`。

约第 6129-6132 行(重置态用的那处,先 `command grep -n
"home_project_search.clear" -B5 crates/dozer-app/src/app.rs` 核对是哪个
函数)当前:

```rust
        self.home_project_search.clear();
        self.home_project_search_draft.clear();
        self.home_project_search_editing = false;
        self.home_project_search_cursor = 0;
```

改成:

```rust
        self.home_project_search.clear();
        self.home_project_search_draft.clear();
        self.home_project_search_focused = false;
```

- [x] **Step 3: `Message` 枚举改动**

约第 2139-2148 行当前:

```rust
    /// `HomeProjectSearchEvent`,不再漏进终端(同 `todo::SearchEditStart`)。
    HomeProjectSearchEditStart,
    /// 自绘输入的文本/退格/取消/回车事件,`home_project_search_editing`
    HomeProjectSearchEvent(AddrEvent),
    HomeProjectSearchCursorMove(search_box::CursorDir),
    /// 回车 / 点搜索按钮:把草稿落成生效的 `home_project_search` 过滤词,
    HomeProjectSearchSubmit,
```

(先跑 `command grep -n "HomeProjectSearch" -B3 -A1
crates/dozer-app/src/app.rs` 核对每行完整的文档注释与前后行,本计划只
摘了核心那几行,按实际内容改,不要把邻近变体的注释一起删漏或错删。)

改成:

```rust
    /// 搜索框草稿变化(iced `text_input::on_input`)。
    HomeProjectSearchInput(String),
    /// 回车 / 点搜索按钮:把草稿落成生效的 `home_project_search` 过滤词。
    HomeProjectSearchSubmit,
```

- [x] **Step 4: `update()` 里的处理分支改动**

约第 4458-4496 行当前:

```rust
            Message::HomeProjectSearchEditStart => {
                self.home_project_search_editing = true;
            }
            Message::HomeProjectSearchEvent(ev) => {
                if !self.home_project_search_editing {
                    return;
                }
                match ev {
                    AddrEvent::Text(s) => insert_at_cursor(
                        &mut self.home_project_search_draft,
                        &mut self.home_project_search_cursor,
                        &s,
                    ),
                    AddrEvent::Backspace => {
                        delete_before_cursor(
                            &mut self.home_project_search_draft,
                            &mut self.home_project_search_cursor,
                        );
                    }
                    AddrEvent::Cancel => self.home_project_search_editing = false,
                    AddrEvent::Submit => self.commit_home_project_search(),
                }
            }
            Message::HomeProjectSearchCursorMove(dir) => {
                if !self.home_project_search_editing {
                    return;
                }
                move_cursor_in(
                    &self.home_project_search_draft,
                    &mut self.home_project_search_cursor,
                    dir,
                );
            }
            Message::HomeProjectSearchSubmit => {
                self.commit_home_project_search();
                self.home_project_search_editing = false;
            }
```

改成:

```rust
            Message::HomeProjectSearchInput(s) => self.home_project_search_draft = s,
            Message::HomeProjectSearchSubmit => self.commit_home_project_search(),
```

- [x] **Step 5: 新增访问器 + 焦点查询结构体**

紧邻现有 `pub fn home_project_search_editing(&self) -> bool { self.
home_project_search_editing }`(约第 3285 行)之处,整体替换为:

```rust
    /// 首页项目搜索框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn home_project_search_focused(&self) -> bool {
        self.home_project_search_focused
    }

    /// 每帧渲染循环调用:把 `CaptureHomeSearchFocus` 问到的真实焦点态
    /// 写进来。
    pub fn set_home_project_search_focused(&mut self, focused: bool) {
        self.home_project_search_focused = focused;
    }
```

`homespace.rs` 里(靠近顶部,搜索框相关代码之前)新增:

```rust
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use iced_widget::core::Rectangle;

pub fn home_search_field_id() -> Id {
    Id::new("home-project-search-box")
}

static HOME_SEARCH_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

pub fn take_home_search_focused() -> bool {
    *HOME_SEARCH_FOCUSED.lock().unwrap()
}

/// 每帧 `interface.operate()` 跑一遍。`traverse` 必须调用传入闭包(见
/// [[dozer-operation-traverse-noop-bug]])。
pub struct CaptureHomeSearchFocus;
impl Operation<()> for CaptureHomeSearchFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&home_search_field_id()) {
            *HOME_SEARCH_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}
```

(检查 `homespace.rs` 顶部现有 import,`iced_widget::core::{...}` 那行如果
已经引入了部分同名类型,合并进去,不要重复 `use`。)

- [x] **Step 6: `home_project_list_view` 里替换搜索框构造**

约第 412-431 行当前:

```rust
    col = col.push(crate::search_box::view(
        &app.home_project_search_draft,
        app.home_project_search_editing,
        !app.home_project_search.is_empty(),
        app.home_project_search_cursor,
        "搜索项目…",
        theme::homespace_font::body(),
        byteui::theme::icon_size::row(),
        crate::search_box::SearchBoxColors {
            bg: theme::homespace_color::card_bg(),
            border: theme::homespace_color::border(),
            active: theme::homespace_color::gold(),
            text: theme::homespace_color::cream(),
            dim: theme::homespace_color::dim(),
        },
        app.hover_progress(HoverId::HomeProjectSearchSubmit),
        Message::HomeProjectSearchEditStart,
        Message::HomeProjectSearchSubmit,
        |hovered| Message::Hover(HoverId::HomeProjectSearchSubmit, hovered),
    ));
```

改成(手写 `iced_widget::text_input`,样式手法照抄 `byteui::form::
input_text` 的 `.style()` 闭包写法,配色源换成 `theme::homespace_color`,
见本计划 Architecture 一节的说明——原提交按钮 `HoverId::
HomeProjectSearchSubmit` 那颗嵌入式图标按钮保留,只是不再嵌进
`search_box::view` 内部,改成外层 `row!` 拼接,同 Stage 3 浏览器地址栏
"输入框 + 星标按钮各自独立小部件、外层容器画共享边框"的处理方式):

```rust
    let editing = app.home_project_search_focused();
    let search_field: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        iced_widget::text_input("搜索项目…", &app.home_project_search_draft)
            .id(home_search_field_id())
            .on_input(Message::HomeProjectSearchInput)
            .on_submit(Message::HomeProjectSearchSubmit)
            .size(theme::homespace_font::body())
            .padding(0)
            .style(move |_t: &iced_widget::Theme, status: iced_widget::text_input::Status| {
                let focused = matches!(status, iced_widget::text_input::Status::Focused { .. });
                let _ = focused; // 边框由外层容器统一画,这里只需要透明背景
                iced_widget::text_input::Style {
                    background: iced_widget::core::Color::TRANSPARENT.into(),
                    border: iced_widget::core::Border {
                        color: iced_widget::core::Color::TRANSPARENT,
                        width: 0.0,
                        radius: 0.0.into(),
                    },
                    icon: theme::homespace_color::dim(),
                    placeholder: theme::homespace_color::dim(),
                    value: theme::homespace_color::cream(),
                    selection: byteui::theme::color::mix(
                        theme::homespace_color::gold(),
                        theme::homespace_color::card_bg(),
                        0.6,
                    ),
                }
            })
            .into();

    let submit_button = icons::icon_button_entry(
        icons::IconKind::Search,
        byteui::theme::icon_size::row(),
        false,
        false,
        app.hover_progress(HoverId::HomeProjectSearchSubmit),
        true,
        byteui::theme::icon_size::row() + 12.0,
        true,
        Message::HomeProjectSearchSubmit,
        |hovered| Message::Hover(HoverId::HomeProjectSearchSubmit, hovered),
        "搜索",
    );

    let content_h = byteui::theme::icon_size::row() + 12.0;
    col = col.push(
        container(
            row![
                container(search_field)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_y(iced_widget::core::alignment::Vertical::Center)
                    .align_x(iced_widget::core::alignment::Horizontal::Left),
                container(submit_button).align_y(iced_widget::core::alignment::Vertical::Center),
            ]
            .width(Length::Fill)
            .height(Length::Fixed(content_h))
            .align_y(iced_widget::core::Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fixed(content_h + 12.0))
        .padding([6, 8])
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::homespace_color::card_bg().into()),
            border: iced_widget::core::Border {
                color: if editing || !app.home_project_search.is_empty() {
                    theme::homespace_color::gold()
                } else {
                    theme::homespace_color::border()
                },
                width: 1.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        }),
    );
```

(执行前先跑 `command grep -n "^use\|fn home_project_list_view" -A5
crates/dozer-app/src/homespace.rs` 核对现有 `icons`/`container`/`row`/
`Element`/`Length` 等类型的 import 路径与既有写法是否一致,上面代码块里
的写法要跟文件现状对齐,不要引入第二套不一致的 import 风格;
`icons::icon_button_entry` 的确切参数顺序/含义以 `crates/byteui/src/
interaction/icons.rs` 现状为准,核对后再落地,本计划给出的调用是参照
`extensions::files.rs` 里 `search_button` 那处调用抄来的,细节可能需要
按 `home_project_list_view` 现有的其它按钮调用点微调。)

- [x] **Step 7: 编译确认**

Run: `cargo build -p dozer-app --bin dozer 2>&1 | grep "error\[" `
Expected: 报测试模块与 `main.rs`/`workspace.rs` 相关调用点的未定义错误
(Task 5 修)。

- [x] **Step 8: 跑相关测试 + clippy + fmt**

Run: `cargo test -p dozer-app --bin dozer home -- --nocapture 2>&1 | tail -40 && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "app.rs|homespace.rs"; cargo fmt --package dozer-app`
Expected: 通过(main.rs/workspace.rs 的残留错误留给 Task 5)。若有测试
断言 `home_project_search_editing`/`HomeProjectSearchEditStart` 等旧
API,同 Task 2/3 的改法重写。

- [x] **Step 9: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/app.rs crates/dozer-app/src/homespace.rs
git commit -m "feat(dozer-app): 首页项目搜索框改用真正的 iced text_input"
```

---

## Task 5: `main.rs` 路由收尾 + `search_box.rs` 清理

**Files:**
- Modify: `crates/dozer-app/src/main.rs`
- Modify: `crates/dozer-app/src/workspace.rs`
- Modify: `crates/dozer-app/src/search_box.rs`

**Interfaces:**
- Consumes: Task 2-4 新增的 `CaptureTodoSearchFocus`/`CaptureAddFocus`/
  `CaptureHomeSearchFocus` 与对应 `take_*_focused` 函数。

- [x] **Step 1: 确认当前状态(核对行号,尤其注意 Global Constraints 提到的
  Stage 3 合并影响)**

```bash
command grep -n "to_todo_search\|to_todo_add\|to_home_project_search\|files_search_focused\|browser_addr_focused\|CaptureSearchFocus" crates/dozer-app/src/main.rs
```

若 Stage 3 已合并,`main.rs` 现有闸门是:

```rust
            if app.files_search_focused()
                || app.browser_addr_focused()
                || app.ssh_form_open()
                || app.database_form_open()
            {
                return;
            }
```

在这个基础上(若未合并则是 Stage 2 落地的三项版本)追加
`app.todo_search_focused() || app.todo_add_focused() || app.
home_project_search_focused()`:

```rust
            if app.files_search_focused()
                || app.browser_addr_focused()
                || app.todo_search_focused()
                || app.todo_add_focused()
                || app.home_project_search_focused()
                || app.ssh_form_open()
                || app.database_form_open()
            {
                return;
            }
```

- [x] **Step 2: 移除 `to_self_drawn_input`/`addr_message` 里对应三支**

`command grep -n "to_todo_search\|to_todo_add\|to_home_project_search" crates/dozer-app/src/main.rs`
核对后,把这三个 `let to_xxx = app.xxx_editing();` 声明整行删除,把
`to_self_drawn_input` OR 链里对应三项删除,把 `addr_message` 闭包里
`else if to_todo_search {...}`/`else if to_todo_add {...}` 两支删除(
`to_home_project_search` 分支是链尾的隐式 `else`,删除前两支后自动归到
它,原有 `else { Message::HomeProjectSearchEvent(ev) }` 这个兜底分支本身
也要删除,因为 `Message::HomeProjectSearchEvent` 变体已在 Task 4 删除
——整个 `addr_message` 闭包在删完 Stage 2-4 覆盖的字段后,剩下的分支
应该只有:`to_search_popup`/`to_comment`(若 Stage 3 已合并且顺序不同,
以实际现状为准)/`to_tree_edit`/`to_project_name`/`to_todo_content`/
`to_todo_markdown`——这几个是本次 4 个 Stage 都没覆盖的非目标字段,原样
保留)。

同一处(约第 1178-1184 行区域,先 `command grep -n "if to_todo_add" -B10 -A15
crates/dozer-app/src/main.rs` 核对)方向键/Home/End 的自绘光标移动特判
块里,`if to_todo_add {...} else if to_todo_search {...} else if
to_home_project_search {...}` 这三支删除——它们现在都走标准 iced 管线,
不需要 main.rs 手工转方向键了;删完之后如果这个特判块只剩
`to_todo_content` 一支,连同其外层 `if let Some(dir) = dir {...}` 结构
一并保留(`to_todo_content` 不在本次范围)。

- [x] **Step 3: 渲染循环每帧查询焦点**

在 Task 2-4 新增的三个焦点查询点,照抄 Files 搜索框 Stage 2 的两阶段写法
(`interface.operate()` 时只能存进局部变量,`interface.into_cache()` 之后
才能写回 `app`)。`command grep -n "let files_focused" -A15
crates/dozer-app/src/main.rs`(若 Stage 3 已合并,还会看到
`browser_addr_focused` 那段,一并参照)找到现有写法,依葫芦画瓢加三段:

```rust
                                let todo_search_focused =
                                    if matches!(app.left_view(), crate::app::PanelKind::Todo) {
                                        interface.operate(
                                            renderer,
                                            &mut extensions::todo::CaptureTodoSearchFocus,
                                        );
                                        extensions::todo::take_todo_search_focused()
                                    } else {
                                        false
                                    };
                                let todo_add_focused =
                                    if matches!(app.left_view(), crate::app::PanelKind::Todo) {
                                        interface.operate(
                                            renderer,
                                            &mut extensions::todo::CaptureAddFocus,
                                        );
                                        extensions::todo::take_add_focused()
                                    } else {
                                        false
                                    };
                                let home_search_focused = if app.home_view_active() {
                                    interface.operate(
                                        renderer,
                                        &mut homespace::CaptureHomeSearchFocus,
                                    );
                                    homespace::take_home_search_focused()
                                } else {
                                    false
                                };
```

(`app.home_view_active()` 是占位名——先跑 `command grep -n "fn.*home.*view\|home_left_view\|fn is_home" crates/dozer-app/src/app.rs`
找到判断"当前是否在首页"的既有访问器,用实际存在的那个,不要凭空造一个
新方法;`PanelKind::Todo` 的判断同 `CaptureFieldBounds` 现有那处"只在
Todo 左栏可见时跑"的写法,原样照抄条件。`todo_search_focused`/
`todo_add_focused` 两段其实可以合并成一次 `interface.operate()` 只是
分开写两个 `Capture` 结构体会重复遍历两次树——如果想优化成一次
`operate()` 内同时查两个 id,把 `CaptureTodoSearchFocus`/`CaptureAddFocus`
合并成一个 `CaptureTodoInputFocus` 结构体的 `focusable()` 里判断两个不同
id 分别写两个 static,这个优化不是本计划强制要求,按实现时的判断取舍,
两种写法都要满足"traverse 调用闭包"这条硬性要求。)

紧跟现有 `app.set_files_search_focused(files_focused);`(以及 Stage 3
落地后应有的 `app.set_browser_addr_focused(browser_addr_focused);`)之后
追加:

```rust
                                app.set_todo_search_focused(todo_search_focused);
                                app.set_todo_add_focused(todo_add_focused);
                                app.set_home_project_search_focused(home_search_focused);
```

- [x] **Step 4: `App`/`Workspace` 新增委托访问器**

`workspace.rs` 里(靠近 `files_search_focused` 委托访问器)新增:

```rust
    pub fn todo_search_focused(&self) -> bool {
        self.todo.search_focused()
    }

    pub fn todo_add_focused(&self) -> bool {
        self.todo.add_focused()
    }
```

`app.rs` 里(靠近 `files_search_focused`/`set_files_search_focused`)
新增:

```rust
    pub fn todo_search_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo_search_focused())
    }

    pub fn set_todo_search_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.todo.set_search_focused(focused);
        }
    }

    pub fn todo_add_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo_add_focused())
    }

    pub fn set_todo_add_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.todo.set_add_focused(focused);
        }
    }
```

(`home_project_search_focused`/`set_home_project_search_focused` 已在
Task 4 Step 5 加过,这里不重复。)

- [x] **Step 5: `workspace.rs::blur_inputs()` 清理**

`command grep -n "fn blur_inputs" -A20 crates/dozer-app/src/workspace.rs`
核对,把 `self.todo.cancel_search_edit();`/`self.todo.cancel_add_edit();`
两行删除(方法已在 Task 2/3 删除——两个字段不再需要点击别处手动清编辑态,
`CaptureTodoSearchFocus`/`CaptureAddFocus` 下一帧自然会把真实焦点丢失
同步进来)。

- [x] **Step 6: `search_box.rs` 清理**

删除 `view()` 函数(约第 99-189 行)与 `SearchBoxColors` 结构体(约第
23-34 行)。**保留** `CursorDir`/`char_to_byte`/`insert_at_cursor`/
`delete_before_cursor`/`move_cursor_in`/`draft_with_caret` 这几个自由
函数——`todo.rs` 顶部 `use crate::search_box::{...}` 与 `pub use crate::
search_box::CursorDir` 仍被 Todo 任务内容编辑(`ContentEvent`/
`ContentCursorMove`)与 Todo MARKDOWN 整文件编辑(`MarkdownEvent`)依赖,
这两个不在本计划范围,不能跟着削掉它们的依赖(同 spec"清理"一节的既有
判断)。删除 `view()`/`SearchBoxColors` 后运行:

```bash
cargo build --workspace 2>&1 | grep "error\[" 
```

确认没有其它遗漏的调用方(Task 2/4 应该已经把仅有的两个调用方都改掉了,
若这里报错说明有遗漏,回去核对)。

- [x] **Step 7: 全量编译确认**

Run: `cargo build --workspace 2>&1 | grep "error\[" `
Expected: 无输出。

- [x] **Step 8: 跑 dozer-app 全量测试**

Run: `cargo test -p dozer-app --bin dozer 2>&1 | tail -40`
Expected: 全绿(已知基线:`git_log` 分支名 dogfood 环境脆弱性失败不算本
计划引入)。

- [x] **Step 9: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets && cargo fmt --package dozer-app`
Expected: 无新增警告;fmt 无残留改动。

- [x] **Step 10: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/main.rs crates/dozer-app/src/workspace.rs \
  crates/dozer-app/src/app.rs crates/dozer-app/src/search_box.rs
git commit -m "feat(dozer-app): main.rs 键盘路由为首页搜索框/Todo 搜索框/添加框新增原生输入放行判断,清理 search_box::view"
```

---

## Task 6: 全量校验 + 人工 GUI 走查 + 提请审阅

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

跑 `cargo run -p dozer-app`,三个字段逐一核对:

**首页项目搜索框**(切到首页):
1. 真光标显示、方向键移动、鼠标点击定位、拖拽选中、Shift+方向键选区。
2. 输入词过滤项目列表、敲回车或点搜索按钮提交,配色是首页自己的
   `homespace_color`(不是工作区的金/青配色体系——重点检查这条,因为
   这是本计划唯一没走 byteui 组件的字段)。
3. IME 候选框跟随光标。
4. 背景终端聚焦时,在搜索框里打字/⌘V/方向键,不漏进终端。

**Todo 搜索框**(切到 Todo 面板):
1. 同上六项基础能力 + 过滤任务列表 + 回车/按钮提交 + IME + 终端不漏键。

**Todo 添加框**(切到 Todo 面板):
1. 真光标显示、方向键移动、鼠标点击定位、拖拽选中、IME 候选框跟随。
2. 输入多行文字(text_editor 天然支持,确认按 Enter 是插入换行而不是
   提交——提交只走右下角按钮,这条是 Stage 1 设计就定下的行为,人工确认
   没有意外被哪里改成了 Enter 提交)。
3. 顶部拖拽手柄仍能手动调高/缩小(120~400px),调高后 `text_editor` 用
   `Length::Fixed` 定高、内容超出这个高度时能正常滚动(不是溢出裁切或
   撑破布局)。
4. 提交按钮点击正常落盘新任务,不会误触发进入编辑态之外的副作用。
5. 背景终端聚焦时,在添加框里打字/⌘V/方向键,不漏进终端。

- [x] **Step 3: 确认全部 commit 都在特性分支上**

Run: `git log --oneline main..HEAD`(在
`feature/native-text-input-stage4` 分支上执行)
Expected: 列出 Task 1-5 的五个 commit,`git branch --show-current` 不是
`main`。

- [ ] **Step 4: 提请审阅**

不在这一步直接合并——按仓库惯例提请代码审阅,通过后再合并回 `main`
(参考 `superpowers:finishing-a-development-branch`)。审阅通过合并后,
原 spec 目标 1 的 4 个字段迁移 + 顺带并入的 Todo 搜索框全部完工,`docs/
superpowers/specs/2026-08-20-native-text-input-adoption-design.md` 目标
6 列的人工走查清单(4 迁移 + 2 路由验证,本计划的 Todo 搜索框是额外的
第 5 个迁移字段)应该全部走完过一遍。届时可以更新 spec 状态或另开一份
收尾总结,不在本计划范围内。
