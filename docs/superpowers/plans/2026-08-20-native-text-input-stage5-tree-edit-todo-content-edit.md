# 输入框改用 iced 原生控件 Stage 5(项目树行内编辑 + Todo 任务内容编辑)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把项目树行内编辑(新建文件/文件夹/重命名共用一个 `tree_edit`)和
Todo 任务内容行内编辑(`editing_content`)从自绘输入改成真正的 iced
`text_input`。这两个字段是原 spec(`2026-08-20-native-text-input-adoption-
design.md`)明确列为"非目标、留给后续 spec"的 6 个自绘输入面里的头两个
——本计划是它们的第一份实现计划,不属于原 spec 编号下的 Stage,是新开的
一轮(仍复用同一套已验证架构,brainstorming 会话已确认不需要重新设计)。

**Architecture:** 复用 Stage 2-4 验证过的"每帧查询 iced 真实焦点态 + 原生
放行闸门"主线,不发明新模式;但这两个字段的**触发方式**跟之前 6 个字段
都不一样——之前(Files/浏览器/Todo/首页搜索框等)都是"点输入框本身进入
编辑",点击天然落在真控件上,iced 自己给焦点。这两个字段是"点别的控件
(右键菜单项/任务卡片文字)触发,编辑框下一帧才在原地出现",新出现的
控件不会自动拿到焦点。解法:新增一个"一次性程序化聚焦"标记(与
`extensions::todo::scroll_to_top`/`take_scroll_to_top` 完全同构的既有
手法),触发编辑的消息处理里置位,`main.rs` 渲染循环在 `UserInterface::
build` **之前**(此时 `app` 还没被不可变借用,能自由 `take()`)取走标记,
若为真则在 `interface.operate()` 阶段用 iced 自带的 `operation::
focusable::focus::<()>(id)` 强制聚焦目标控件——不是新架构,是给已有的
"一次性位"模式换一个新消费场景。

Todo 任务内容编辑还有一个前 6 个字段都没有的行为:**失焦即落盘保存**(不是
丢弃草稿,也不是保留草稿等下次——`Workspace::blur_inputs` 现在调用
`self.todo.commit_content_edit(path)`,这是用户明确反馈过的行为,不能
迁移时弄丢)。迁移后这条"焦点丢失→写盘"的判断从"点击别处触发的
`blur_inputs`"搬到"每帧查询到的真实焦点从真变假那一刻",判断逻辑放在
`App::set_todo_content_focused` 里(不是 `WorkspaceState` 内部——落盘需要
`project_path`,`WorkspaceState` 自己拿不到,`App`/`Workspace` 层才有)。

`todo_card`(渲染任务卡片文字/编辑框的函数)现有签名把返回类型硬写成
`Element<'static, ...>`——这在"内容全部克隆成 `String` 再喂给 `text()`"的
自绘时代没问题,但真正的 `iced_widget::text_input(placeholder, value:
&str)` 内部持有对 `value` 的借用,返回类型天然带 `'a` 生命周期,不可能是
`'static`。这个函数本来就有自己的 `<'a>` 泛型参数(只是返回值没用上),
把返回类型从 `Element<'static, ...>` 松绑成 `Element<'a, ...>` 即可——
按协变规则,这不会破坏任何现有调用方(之前满足更严格的 `'static` 边界,
放宽只会让更多调用方式合法)。

**Tech Stack:** Rust 2024,iced 0.14(`iced_widget::text_input`,
`iced_core::widget::operation::focusable`)。

**Spec:** 本计划不对应 `2026-08-20-native-text-input-adoption-design.md`
的任何目标编号(该 spec 明确把这两个字段列为非目标),是 brainstorming
会话(2026-08-20,bounded 路径,未写独立 spec 文档——用户确认沿用既有
架构、不需要重新论证)后直接进入的实现计划。

## Global Constraints

- **在独立分支上开发,不直接提交 main。** 本仓库有长驻自动化在 main 上持续
  开发(建 worktree 前用 `git -C /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
  worktree list` 确认没有同名 worktree),从 main 当前 tip 拉新 worktree:
  `git worktree add ../dozer-native-text-input-stage5 -b
  feature/native-text-input-stage5 main`。后续所有命令都在这个新 worktree
  目录里跑,每次 `git commit`/`git add` 前先 `git branch --show-current`
  确认不在 `main` 上。
- 下面每个 Task 引用的行号以 2026-08-20 main tip(commit `7c5af46`)为准;
  执行前先用对应的 `grep -n` 命令核对实际行号,若有出入以代码现状为准
  (本仓库有并发自动化开发,main tip 可能已经前进)。
- **`Operation<()>` 的 `traverse` 必须实现成调用传入闭包**(`fn traverse(&mut
  self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a)))
  { operate(self); }`)——[[dozer-operation-traverse-noop-bug]] 记录的
  Critical bug 教训,本计划新增的 `CaptureTreeEditFocus`/
  `CaptureContentEditFocus` 必须从一开始写对。
- **写入 App 状态的每帧读写必须遵守既有的"两阶段"借用顺序**:
  `UserInterface::build(app.view(), ...)` 之前能自由 `&mut app`(一次性
  标记的 `take()` 就发生在这里,同 `take_todo_scroll_to_top` 的既有位置);
  `interface` 建出来之后到 `interface.into_cache()` 之前只能 `&app`(查询
  结果只能存局部变量);`into_cache()` 之后才能再 `&mut app` 写回查询结果。
  不要把这两类操作的时序搞反。
- 每个 Task 结束都要求对应 crate `cargo build` 干净通过;涉及 `dozer-app`
  的 Task 额外要求 `cargo test -p dozer-app --bin dozer <关键字>` 通过。
- 最终 Task 要求全 workspace `cargo build && cargo test -p byteui -p
  dozer-app --bin dozer && cargo clippy --all-targets && cargo fmt --check`
  全绿,记录实际 pass/fail 数字(已知基线:636 个既有测试 + 本计划新增的
  测试;`git_log` 分支名 dogfood 环境脆弱性失败不算本计划引入,若失败先
  确认没有残留的旧 feature 分支/worktree)。
- **迁移完成后的人工 GUI 走查不可省略**,Task 4 列出完整走查清单,重点是
  "触发编辑后不用再点一下就能直接打字"这条(一次性聚焦机制是否真的生效)
  和"Todo 内容编辑失焦仍然落盘"这条(commit-on-blur 行为没有被迁移弄丢)。
- **已知的、经用户确认的行为变化**(同 Stage 2-4 的判断口径):Esc 不再能
  取消这两处编辑(原生控件不处理 Esc,main.rs 也不再拦截转发)。不额外加
  Esc 拦截闸门,这是本计划开工前 brainstorming 阶段明确确认过的取舍。

---

## Task 1: 项目树行内编辑改用真正的 `text_input`

**Files:**
- Modify: `crates/dozer-app/src/extensions/files.rs`
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `byteui::form::input_text::view`(现有 8 参数签名)。
- Produces: `pub fn tree_edit_field_id() -> Id`;`pub struct
  CaptureTreeEditFocus`;`pub fn take_tree_edit_focused() -> bool`;
  `WorkspaceState::tree_edit_focused(&self) -> bool` /
  `set_tree_edit_focused(&mut self, bool)`(取代 `tree_edit_is_some`/
  `cancel_tree_edit` 在键盘路由上的角色);
  `WorkspaceState::take_tree_edit_focus_pending(&mut self) -> bool`(一次性
  聚焦标记,`Workspace`/`App` 各包一层同名委托)。

- [ ] **Step 1: 确认当前状态(核对行号)**

```bash
command grep -n "tree_edit\|TreeEdit\|EditEvent\|tree_editing\|tree_edit_row\|fn start_tree_new\|RenameStart" crates/dozer-app/src/extensions/files.rs
```

- [ ] **Step 2: `WorkspaceState` 字段改动**

约第 56 行当前:

```rust
    tree_edit: Option<TreeEdit>,
```

改成(新增一次性聚焦标记字段,紧跟其后):

```rust
    tree_edit: Option<TreeEdit>,
    /// 一次性标记:`tree_edit` 刚从 `None` 变成 `Some`(新建/重命名刚
    /// 触发)时置真,main.rs 渲染循环取走后用 `operation::focusable::
    /// focus` 强制聚焦真正的 `text_input`——右键菜单点"重命名"/"新建
    /// 文件"这类触发点击落在别的控件上,新出现的输入框不会自动拿到
    /// iced 焦点,需要这一下程序化聚焦(同 `todo::scroll_to_top` 的既有
    /// 一次性位手法)。
    tree_edit_focus_pending: bool,
```

- [ ] **Step 3: 访问器改动**

约第 281-307 行当前:

```rust
    /// 供内核 `Workspace::blur_inputs`(点击输入框外时退出所有自绘输入的
    /// 编辑态)调用——原逻辑直接 `self.tree_edit = None`,字段私有化后改走
    /// 这个访问器。
    pub fn cancel_tree_edit(&mut self) {
        self.tree_edit = None;
    }
```

```rust
    /// 供内核 `Workspace::tree_editing`(main.rs 键盘路由用,判断项目树是否
    /// 处于行内编辑态)调用。
    pub fn tree_edit_is_some(&self) -> bool {
        self.tree_edit.is_some()
    }
```

两处整体改成:

```rust
    /// 项目树行内编辑框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    /// **不是**应用层手动置位的镜像——每帧渲染循环里 `CaptureTreeEditFocus`
    /// 问一遍 iced 真相后立刻写进这里(`set_tree_edit_focused`)。
    pub fn tree_edit_focused(&self) -> bool {
        self.tree_edit_focused_flag
    }

    /// 每帧渲染循环读走 `CaptureTreeEditFocus` 查到的真实焦点态后写进来。
    /// 焦点从真变假(刚失去焦点)时清空 `tree_edit`——项目树重命名/新建
    /// 是"点别处就该退出"的一次性行内编辑,不像搜索框那样希望保留草稿
    /// (现状既有行为,`cancel_tree_edit` 原本就是这个语义,只是触发时机
    /// 从"点击外部"改成"真实焦点丢失")。
    pub fn set_tree_edit_focused(&mut self, focused: bool) {
        if self.tree_edit_focused_flag && !focused {
            self.tree_edit = None;
        }
        self.tree_edit_focused_flag = focused;
    }
```

(注意:上面引用了一个新增的 `tree_edit_focused_flag: bool` 字段——跟
`tree_edit_focus_pending` 是两个不同的东西,前者是"当前是否聚焦"的持续
状态镜像,后者是"该不该现在去抢一次焦点"的一次性标记。回到 Step 2,在
`tree_edit_focus_pending` 字段声明前面补上:

```rust
    tree_edit: Option<TreeEdit>,
    /// 项目树行内编辑框是否持有 iced 内部真实焦点,每帧由 `CaptureTreeEditFocus`
    /// 写入。
    tree_edit_focused_flag: bool,
    /// 一次性标记:`tree_edit` 刚从 `None` 变成 `Some`……(同 Step 2 原文)
    tree_edit_focus_pending: bool,
```

紧接着新增一次性标记的存取方法:

```rust
    /// 读走(消费式)一次性聚焦标记。main.rs 在 `UserInterface::build`
    /// 之前调用(此时还能自由 `&mut app`),同 `todo::take_scroll_to_top`
    /// 的既有调用时机。
    pub fn take_tree_edit_focus_pending(&mut self) -> bool {
        std::mem::take(&mut self.tree_edit_focus_pending)
    }
```

- [ ] **Step 4: `start_tree_new`/`RenameStart` 触发点补上置位**

约第 358-368 行 `start_tree_new` 当前:

```rust
    fn start_tree_new(&mut self, parent: PathBuf, mode: TreeEditMode) {
        self.tree_error = None;
        if let Some(tree) = &mut self.file_tree {
            tree.ensure_expanded(&parent);
        }
        self.tree_edit = Some(TreeEdit {
            parent_dir: parent,
            mode,
            buffer: String::new(),
        });
    }
```

改成(末尾加一行):

```rust
    fn start_tree_new(&mut self, parent: PathBuf, mode: TreeEditMode) {
        self.tree_error = None;
        if let Some(tree) = &mut self.file_tree {
            tree.ensure_expanded(&parent);
        }
        self.tree_edit = Some(TreeEdit {
            parent_dir: parent,
            mode,
            buffer: String::new(),
        });
        self.tree_edit_focus_pending = true;
    }
```

约第 712-726 行 `Message::RenameStart(path)` 当前:

```rust
        Message::RenameStart(path) => {
            app_state.context_menu = None;
            ws_state.tree_error = None;
            let Some(parent) = path.parent().map(|p| p.to_path_buf()) else {
                return;
            };
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            ws_state.tree_edit = Some(TreeEdit {
                parent_dir: parent,
                mode: TreeEditMode::Rename(path),
                buffer: name,
            });
        }
```

末尾同样加一行 `ws_state.tree_edit_focus_pending = true;`。

- [ ] **Step 5: `tree_edit_field_id`/`CaptureTreeEditFocus` 新增**

紧跟 `WorkspaceState` 的 `impl` 块之后(或任意模块级位置,参照
`extensions::files::search_field_id`/`CaptureSearchFocus` 的既有位置):

```rust
/// 项目树行内编辑框稳定的 iced widget id。同一时刻 `tree_edit` 只可能是
/// `Some` 一份(新建/重命名互斥,不会有两个编辑框同时存在),固定 id 够用,
/// 不需要按行号/路径动态生成。
pub fn tree_edit_field_id() -> Id {
    Id::new("files-tree-edit-box")
}

static TREE_EDIT_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

pub fn take_tree_edit_focused() -> bool {
    *TREE_EDIT_FOCUSED.lock().unwrap()
}

/// 每帧 `interface.operate()` 跑一遍。`traverse` 必须调用传入闭包(见
/// [[dozer-operation-traverse-noop-bug]])。
pub struct CaptureTreeEditFocus;
impl Operation<()> for CaptureTreeEditFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&tree_edit_field_id()) {
            *TREE_EDIT_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}
```

(文件顶部 import 若还没有 `Id`/`Operation`/`Rectangle`/
`operation::Focusable`,补齐——`command grep -n "^use" crates/dozer-app/src/extensions/files.rs`
核对现状,files.rs 在 Stage 2 已经因为 `CaptureSearchFocus` 加过这几个
import,大概率已存在,不用重复加。)

- [ ] **Step 6: `Message::EditEvent` 改动**

约第 152 行 `Message` 枚举里的 `EditEvent(AddrEvent)` 改成:

```rust
    /// 项目树行内编辑框草稿变化(iced `text_input::on_input`,每次给全量
    /// 当前字符串)。
    EditInput(String),
    /// 回车 / 失焦(由 `set_tree_edit_focused` 的边缘触发,不经过消息):
    /// 提交改名/新建。
    EditSubmit,
```

(`AddrEvent` 类型本身保留,`Message::EditEvent(AddrEvent)` 这一个消费方
删除,`crate::workspace::AddrEvent` 的 import 若文件里还有其它消费方
(验收意见/项目名称编辑等不在本计划范围,files.rs 里没有,但顶层
`use crate::workspace::AddrEvent;` 这行本身可能因为本文件只有这一个
消费方而变得多余——`command grep -n "AddrEvent" crates/dozer-app/src/extensions/files.rs`
核对,若确认 `EditEvent` 是本文件唯一用到 `AddrEvent` 的地方,这个 `use`
连同类型引用一并删除;若还有别处用到就保留)。

- [ ] **Step 7: `update()` 里的处理分支改动**

约第 728-740 行当前:

```rust
        Message::EditEvent(ev) => {
            let Some(edit) = &mut ws_state.tree_edit else {
                return;
            };
            match ev {
                AddrEvent::Text(s) => edit.buffer.push_str(&s),
                AddrEvent::Backspace => {
                    edit.buffer.pop();
                }
                AddrEvent::Cancel => ws_state.tree_edit = None,
                AddrEvent::Submit => ws_state.submit_tree_edit(project_id, handle, emit),
            }
        }
```

改成:

```rust
        Message::EditInput(s) => {
            let Some(edit) = &mut ws_state.tree_edit else {
                return;
            };
            edit.buffer = s;
        }
        Message::EditSubmit => {
            ws_state.submit_tree_edit(project_id, handle, emit);
        }
```

- [ ] **Step 8: `view()` 里替换 `tree_edit_row` 构造(两处调用点)**

约第 1017-1030 行(重命名分支)与约第 1150-1165 行(新建分支)当前都是:

```rust
                let buffer = ws_state
                    .tree_edit
                    .as_ref()
                    .map(|e| e.buffer.as_str())
                    .unwrap_or("");
                tree_col = tree_col.push(tree_edit_row(row.depth, buffer));
```

(新建分支是 `tree_edit_row(row.depth + 1, buffer)`,深度参数不同,其余
一致。)`tree_edit_row` 函数本身(约第 1199-1222 行)当前:

```rust
fn tree_edit_row(
    depth: usize,
    buffer: &str,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let indent = "  ".repeat(depth);
    container(
        text(format!("{indent}{buffer}▏"))
            .size(crate::workspace::tree_row_font_size())
            .line_height(LineHeight::Relative(terminal_font::line_height_factor()))
            .color(byteui::theme::color::current().cream),
    )
    .width(Length::Fill)
    .padding([2, 4])
    .style(|_t: &iced_widget::Theme| container::Style {
        background: Some(byteui::theme::color::current().card.into()),
        border: Border {
            color: byteui::theme::color::current().cream,
            width: 1.0,
            radius: 2.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}
```

改成(缩进用等宽空格前缀模拟,真实文本前面加 `indent` 字符串;`byteui::
form::input_text` 没有"文字前缀缩进"这个概念,缩进量跟树的层级绑定,这里
直接把缩进拼进 `placeholder`/`value` 前面不现实——正确做法是外层拿一个
`Padding::left(indent_px)`包一层,不把缩进字符拼进文本内容,这是本次
迁移顺带修正的一个小问题:旧版拿等宽空格字符模拟缩进,新版用真正的
左内边距,视觉效果更准,不再依赖字体等宽假设):

```rust
fn tree_edit_row(depth: usize, buffer: &str) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let indent_px = depth as f32 * crate::workspace::tree_row_indent_width();
    container(byteui::form::input_text::view(
        "",
        buffer,
        false,
        Some(tree_edit_field_id()),
        false,
        Some(Message::EditSubmit),
        false,
        Message::EditInput,
    ))
    .padding(Padding {
        left: indent_px,
        ..Padding::default()
    })
    .into()
}
```

(`crate::workspace::tree_row_indent_width()` 是**假设存在**的一个"每级
缩进多少逻辑像素"的辅助函数——**先跑 `command grep -n "tree_row_indent\|fn tree_row_font_size" crates/dozer-app/src/workspace.rs`
核对是否真的已经有这个函数**;如果没有,现有代码里"每级缩进用几个全角
空格字符"这个换算关系一定藏在某处渲染代码里,找到对应的像素换算逻辑
抄一份过来,不要凭空造一个不存在的函数签名——这是本计划里少数几个
"需要执行时二次确认现状"的点,如果查证后发现旧版缩进本来就是拿等宽
字符宽度估算、没有现成的像素级函数,退而求其次:保留"字符缩进"的思路,
但只缩进 `placeholder` 展示层不现实(会被当成用户输入的一部分),这种
情况下改成外层 `container` 用 `depth as f32 * <字体宽度估算 px>` 现算,
数值抄 `tree_row_font_size() * 0.6`(ASCII 宽度经验值,同
`extensions::todo::cursor_from_x` 里的换算口径)乘以 2(原来两个全角空格)
作为每级缩进量,写清楚这个数字的来源,不要留一个没有注释的魔法数字。)

- [ ] **Step 9: `App::blur_inputs` 清理**

`workspace.rs` 里 `blur_inputs()`(约第 2126-2146 行)当前有:

```rust
        self.files.cancel_tree_edit();
```

删除这一行(方法已在 Step 3 删除——项目树编辑不再需要点击别处手动清编辑
态,`set_tree_edit_focused` 的边缘触发逻辑负责清空,`CaptureTreeEditFocus`
下一帧自然会把真实焦点丢失同步进来)。

- [ ] **Step 10: `main.rs` 键盘路由改动**

```bash
command grep -n "to_tree_edit\|tree_editing\|app.tree_editing" crates/dozer-app/src/main.rs crates/dozer-app/src/app.rs crates/dozer-app/src/workspace.rs
```

`workspace.rs::tree_editing()`(约第 2074 行)与 `app.rs::tree_editing()`
(约第 3239 行)两处访问器改名为 `tree_edit_focused()`,内部委托改成读
`ws.files.tree_edit_focused()`(同 `files_search_focused` 那批访问器的
既有命名/结构)。

`main.rs` 原生放行闸门(现状约第 1037-1046 行,以实际内容为准——Stage 3/4
合并后已经是好几个字段拼起来的样子,先跑 `command grep -n "if app.files_search_focused" -A15 crates/dozer-app/src/main.rs`
核对现状)追加 `|| app.tree_edit_focused()`。`to_self_drawn_input` 链、
`addr_message` 闭包里删掉 `to_tree_edit`/`Message::Files(extensions::
files::Message::EditEvent(ev))` 那一支(同 Stage 2-4 处理其它字段时的
删除手法)。

- [ ] **Step 11: 渲染循环接入一次性聚焦 + 每帧焦点查询**

```bash
command grep -n "let scroll_pending = app.take_todo_scroll_to_top" -A3 crates/dozer-app/src/main.rs
```

在这一行**之前或紧邻**加(同一个"UserInterface::build 之前取一次性位"
的时机):

```rust
            let tree_edit_focus_pending =
                app.active_workspace_mut().is_some_and(|ws| ws.files.take_tree_edit_focus_pending());
```

(需要一个 `Workspace::take_tree_edit_focus_pending` 委托方法,同
`files_search_focused` 那批委托的写法,补在 `workspace.rs` 里;上面这行
如果 `Workspace` 没有直接暴露 `pub(crate) files: files::WorkspaceState`
字段给 `App` 直接 `ws.files.xxx()` 调用,按现状实际可见性调整成
`ws.take_tree_edit_focus_pending()` 委托调用,两种写法效果一样,以
`files_search_focused` 那批代码现状实际用的是哪种直接抄。)

`command grep -n "if scroll_pending" -A10 crates/dozer-app/src/main.rs`
核对现状后,在这段 `if scroll_pending { ... }` 逻辑**之后**追加:

```rust
                                if tree_edit_focus_pending {
                                    let mut op = iced_widget::core::widget::operation::focusable::focus::<()>(
                                        extensions::files::tree_edit_field_id(),
                                    );
                                    interface.operate(renderer, &mut op);
                                }
```

`command grep -n "let files_focused =" -A15 crates/dozer-app/src/main.rs`
核对现状后,在同一批每帧焦点查询代码块里追加(gating 条件用
`PanelKind::Files`,同 `CaptureSearchFocus` 那道查询的既有写法):

```rust
                                let tree_edit_focused =
                                    if matches!(app.left_view(), crate::app::PanelKind::Files) {
                                        interface.operate(
                                            renderer,
                                            &mut extensions::files::CaptureTreeEditFocus,
                                        );
                                        extensions::files::take_tree_edit_focused()
                                    } else {
                                        false
                                    };
```

`command grep -n "app.set_files_search_focused" -A3 crates/dozer-app/src/main.rs`
核对现状后,在写回代码块里追加:

```rust
                                app.set_tree_edit_focused(tree_edit_focused);
```

(需要一个 `App::set_tree_edit_focused` 委托方法,同 `set_files_search_
focused` 的既有写法,补在 `app.rs` 里,委托到
`ws.files.set_tree_edit_focused(focused)`。)

- [ ] **Step 12: 重写测试**

`command grep -n "EditEvent\|tree_edit_is_some\|fn.*tree_edit" crates/dozer-app/src/extensions/files.rs`
找到测试模块里对应断言。改法同 Stage 2 Task 2 Step 10/Stage 4 Task 2
Step 9 的既有手法:`Message::EditEvent(AddrEvent::Text(s))` 改成
`Message::EditInput("..".to_string())`;`AddrEvent::Submit` 改成
`Message::EditSubmit`;`assert!(ws_state.tree_edit_is_some())` 之类的断言
如果原本是检查"编辑态开着",改成检查 `ws_state.tree_edit.is_some()`(内部
字段测试里本就可以直接访问)或新增的 `tree_edit_focused()`/
`set_tree_edit_focused()` 的构造性测试(同 Stage 2/4 新增的
`set_xxx_focused_updates_accessor` 测试写法)。

- [ ] **Step 13: 构建 + 测试 + clippy + fmt**

Run: `cargo build -p dozer-app --bin dozer 2>&1 | grep "error\[" `
Expected: 无残留 `EditEvent`/`tree_edit_is_some`/`cancel_tree_edit`/
`tree_editing`(旧名)相关错误。

Run: `cargo test -p dozer-app --bin dozer files:: -- --nocapture 2>&1 | tail -50`
Expected: 全部 PASS。

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "files.rs|main.rs|app.rs|workspace.rs"; cargo fmt --package dozer-app`
Expected: 无新增警告;fmt 无残留改动。

- [ ] **Step 14: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/files.rs crates/dozer-app/src/app.rs \
  crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): 项目树行内编辑改用真正的 iced text_input,新增一次性程序化聚焦机制"
```

---

## Task 2: Todo 任务内容编辑改用真正的 `text_input`

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: Task 1 建立的一次性聚焦模式(`operation::focusable::focus`)、
  `byteui::form::input_text::view`。
- Produces: `pub fn content_edit_field_id() -> Id`(**复用现有
  `content_field_id()`,不新增——名字不变,职责从"bounds 捕获"变成
  "focusable 焦点捕获"**);`pub struct CaptureContentEditFocus`;`pub fn
  take_content_edit_focused() -> bool`;
  `WorkspaceState::content_edit_focused(&self) -> bool` /
  `set_content_edit_focused_flag(&mut self, bool)`(纯状态镜像,不做落盘
  判断——落盘判断在 `App::set_todo_content_focused` 里做,见下);
  `WorkspaceState::take_content_edit_focus_pending(&mut self) -> bool`。

- [ ] **Step 1: 确认当前状态(核对行号)**

```bash
command grep -n "editing_content\|content_cursor\|ContentEvent\|ContentEditStart\|ContentCursorMove\|ContentCursorAt\|content_field_id\|CaptureFieldBounds\|cancel_content_edit\|commit_content_edit\|fn todo_card" crates/dozer-app/src/extensions/todo.rs
```

- [ ] **Step 2: `WorkspaceState` 字段改动**

约第 402-406 行当前:

```rust
    /// 任务内容行内编辑态(卡片下标, 草稿)。点卡片任务文字进入,
    /// `ContentEvent(Submit)` 落盘改写任务文字。
    editing_content: Option<(usize, String)>,
    /// 任务内容行内编辑草稿的光标位置(字符下标,见 `add_cursor` 注释)。
    content_cursor: usize,
```

改成:

```rust
    /// 任务内容行内编辑态(卡片下标, 草稿)。点卡片任务文字进入,失焦或
    /// 回车落盘改写任务文字(`commit_content_edit`)。
    editing_content: Option<(usize, String)>,
    /// 任务内容编辑框是否持有 iced 内部真实焦点,每帧由
    /// `CaptureContentEditFocus` 写入。
    content_edit_focused: bool,
    /// 一次性标记:`editing_content` 刚从 `None` 变成 `Some`(点卡片文字
    /// 刚触发编辑)时置真,main.rs 渲染循环取走后用 `operation::
    /// focusable::focus` 强制聚焦(同 `files::tree_edit_focus_pending`
    /// 的既有手法——点卡片文字这个点击落在旧的文字 `MouseArea` 上,不是
    /// 新出现的 `text_input` 本身,不会自动带焦点)。
    content_edit_focus_pending: bool,
```

(`content_cursor` 整字段删除——`text_input` 自己管理光标。)

- [ ] **Step 3: 访问器改动**

约第 565-583 行当前:

```rust
    /// 任务内容行内编辑态是否打开(main.rs 键盘路由用)。
    pub fn content_editing(&self) -> bool {
        self.editing_content.is_some()
    }

    /// 失焦退出任务内容编辑态(`Workspace::blur_inputs` 用):直接丢弃半输入。
    /// 内容编辑是点卡片文字才弹出的一次性行内编辑,行为对齐项目树重命名
    /// (`cancel_tree_edit`)而不是搜索框。
    pub fn cancel_content_edit(&mut self) {
        self.editing_content = None;
    }

    /// 失焦退出任务内容编辑态并**写盘保存**(与回车 `ContentEvent(Submit)` 同
    /// 一条 `commit_content_edit` 落盘路径):改动且非空才写,否则丢弃。
    /// `Workspace::blur_inputs` 走这条,让"点别处"也等价于"按回车提交",
    /// 不丢用户刚改的任务文字(见用户反馈:内容编辑失焦应保存)。
    pub fn commit_content_edit(&mut self, project_path: &std::path::Path) {
        commit_content_edit(self, project_path);
    }
```

**保留** `cancel_content_edit`/`commit_content_edit` 两个方法**不动**(名字
和实现都不变——它们是落盘/丢弃两条路径的核心逻辑,`App::
set_todo_content_focused` 的边缘触发要调用它们)。`content_editing()`
改名新增(旧名先保留,新增一对):

```rust
    /// 任务内容行内编辑态是否打开(`editing_content.is_some()`)。跟"是否
    /// 持有 iced 真实焦点"是两回事——见 `content_edit_focused`。
    pub fn content_editing(&self) -> bool {
        self.editing_content.is_some()
    }

    /// 任务内容编辑框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn content_edit_focused(&self) -> bool {
        self.content_edit_focused
    }

    /// 每帧渲染循环读走 `CaptureContentEditFocus` 查到的真实焦点态后写
    /// 进来。**只更新焦点镜像标记,不做落盘/丢弃判断**——是否该落盘取决
    /// 于"有没有打开的项目",`WorkspaceState` 自己拿不到 `project_path`,
    /// 这个判断在 `App::set_todo_content_focused` 里做(见 main.rs 接线
    /// 部分)。
    pub fn set_content_edit_focused_flag(&mut self, focused: bool) {
        self.content_edit_focused = focused;
    }

    /// 读走(消费式)一次性聚焦标记,同 `files::take_tree_edit_focus_pending`
    /// 的既有手法。
    pub fn take_content_edit_focus_pending(&mut self) -> bool {
        std::mem::take(&mut self.content_edit_focus_pending)
    }
```

- [ ] **Step 4: `Message::ContentEditStart` 补上置位 + `Message` 枚举改动**

约第 776-785 行 `Message` 枚举里当前:

```rust
    /// 点卡片任务文字 → 进入内容行内编辑态(`editing_content` 置位)。
    ContentEditStart(usize),
    /// 内容编辑态下的按键:`Submit` 落盘改写任务文字,`Cancel` 丢弃退出。
    ContentEvent(AddrEvent),
    /// 方向键/Home/End 移动任务内容编辑草稿光标(字符下标)。内容编辑仍是
    /// 自绘输入(本计划只迁「搜索框/添加框」,任务内容编辑不迁),方向键靠
    /// main.rs 翻成 `CursorDir` 经此路由。
    ContentCursorMove(CursorDir),
    /// 鼠标点击任务内容编辑框:把字段内局部点击 x(逻辑像素)折算成字符下标,
    /// 定位光标(自绘输入没原生光标,靠 `content_field_id` 边界换算)。
    ContentCursorAt(f32),
```

(上面这条"内容编辑仍是自绘输入"的注释是 Stage 4 写的、现在过时了——本
计划正是来migrate它。)整体改成:

```rust
    /// 点卡片任务文字 → 进入内容行内编辑态(`editing_content` 置位 +
    /// `content_edit_focus_pending` 置位,main.rs 据此程序化聚焦)。
    ContentEditStart(usize),
    /// 内容编辑框草稿变化(iced `text_input::on_input`)。
    ContentInput(String),
    /// 回车提交:落盘改写任务文字(与失焦落盘共用 `commit_content_edit`
    /// 一条路径)。
    ContentSubmit,
```

(`ContentCursorMove`/`ContentCursorAt` 整体删除——`text_input` 自己处理
方向键/点击定位。)

约第 1243-1257 行 `Message::ContentEditStart(idx)` 的处理(核对确切现状后
按需调整)末尾补上 `ws_state.content_edit_focus_pending = true;`。

- [ ] **Step 5: `content_field_id`/`CaptureFieldBounds` 改动**

约第 874-885 行(先 `command grep -n "pub fn content_field_id" -A20 crates/dozer-app/src/extensions/todo.rs`
核对确切现状)当前 `content_field_id()` 是给 `CaptureFieldBounds`(容器
bounds 捕获,鼠标点击定位用)服务的。迁移后点击定位交给真 `text_input`
自己处理,`CaptureFieldBounds`/`CONTENT_FIELD_BOUNDS`/
`take_content_field_bounds` 三者失去唯一消费方(`ContentCursorAt` 已在
Step 4 删除,main.rs 里读 `take_content_field_bounds()` 的调用点会在
Task 3 一起删),**整体删除**这三个定义(先确认真的没有其它消费方再删,
`command grep -rn "CaptureFieldBounds\|CONTENT_FIELD_BOUNDS\|take_content_field_bounds" crates/dozer-app/src`
应该只剩定义本身和 main.rs 里即将删除的调用点)。

`content_field_id()` 函数本身**保留**(名字、实现都不变——`.id()`
现在挂给真正的 `text_input`,`focusable` 钩子会认它)。紧邻它新增(同
`extensions::files::CaptureTreeEditFocus` 的既有手法):

```rust
static CONTENT_EDIT_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

pub fn take_content_edit_focused() -> bool {
    *CONTENT_EDIT_FOCUSED.lock().unwrap()
}

/// 每帧 `interface.operate()` 跑一遍。`traverse` 必须调用传入闭包(见
/// [[dozer-operation-traverse-noop-bug]])。
pub struct CaptureContentEditFocus;
impl Operation<()> for CaptureContentEditFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&content_field_id()) {
            *CONTENT_EDIT_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}
```

- [ ] **Step 6: `update()` 里的处理分支改动**

约第 1259-1291 行(先核对确切现状)当前形如:

```rust
        Message::ContentEvent(ev) => {
            if ws_state.editing_content.is_none() {
                return;
            }
            match ev {
                AddrEvent::Text(s) => {
                    if let Some((_, draft)) = ws_state.editing_content.as_mut() {
                        insert_at_cursor(draft, &mut ws_state.content_cursor, &s);
                    }
                }
                AddrEvent::Backspace => {
                    if let Some((_, draft)) = ws_state.editing_content.as_mut() {
                        delete_before_cursor(draft, &mut ws_state.content_cursor);
                    }
                }
                AddrEvent::Cancel => ws_state.editing_content = None,
                AddrEvent::Submit => commit_content_edit(ws_state, project_path),
            }
        }
        Message::ContentCursorMove(dir) => {
            if ws_state.editing_content.is_none() {
                return;
            }
            if let Some((_, draft)) = &ws_state.editing_content {
                move_cursor_in(draft, &mut ws_state.content_cursor, dir);
            }
        }
        Message::ContentCursorAt(local_x) => {
            if ws_state.editing_content.is_none() {
                return;
            }
            if let Some((_, draft)) = &ws_state.editing_content {
                ws_state.content_cursor =
                    cursor_from_x(draft, local_x, byteui::theme::font::body() as f32);
            }
        }
```

(以上是根据字段命名推断的合理复原,**执行前必须先跑上面 Step 1 的
grep 拿到逐字精确的现状代码,按实际内容改,不要直接假设这段和本计划
写的字符级一致**——`ContentCursorAt` 的换算函数名/参数可能与 Files
`cursor_from_x` 不是同一个,以 `todo.rs` 内定义为准。)

整体改成:

```rust
        Message::ContentInput(s) => {
            if let Some((_, draft)) = ws_state.editing_content.as_mut() {
                *draft = s;
            }
        }
        Message::ContentSubmit => commit_content_edit(ws_state, project_path),
```

- [ ] **Step 7: `todo_card` 生命周期放宽 + `label_area` 改用真 `text_input`**

约第 1902-1915 行 `todo_card` 签名当前:

```rust
fn todo_card<'a>(
    number: usize,
    idx: usize,
    item: &'a TodoItem,
    state: TodoState,
    meta: Option<&'a TodoTaskMeta>,
    dispatch: Option<&'a DispatchRecord>,
    selected: bool,
    grabbing: bool,
    is_drag_source: bool,
    hovered: bool,
    editing_draft: Option<&'a str>,
    content_cursor: usize,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
```

改成(返回类型 `'static` → `'a`,删除 `content_cursor` 参数——不再需要,
真 `text_input` 自己管理光标):

```rust
fn todo_card<'a>(
    number: usize,
    idx: usize,
    item: &'a TodoItem,
    state: TodoState,
    meta: Option<&'a TodoTaskMeta>,
    dispatch: Option<&'a DispatchRecord>,
    selected: bool,
    grabbing: bool,
    is_drag_source: bool,
    hovered: bool,
    editing_draft: Option<&'a str>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
```

**这个函数体内部所有 `Element<'static, ...>` 类型标注(如果有局部变量
显式写了这个类型)一并改成 `Element<'a, ...>`**——`command grep -n
"Element<'static" crates/dozer-app/src/extensions/todo.rs` 核对本函数体
内是否还有其它显式 `'static` 标注(比如 `label_area` 那行,见下)。

约第 2045-2075 行 `label_area` 构造当前(核对确切现状后按需调整):

```rust
    let label_area: Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if let Some(draft) = editing_draft {
            let field = if draft.is_empty() {
                text("任务内容…")
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().dim)
            } else {
                text(draft_with_caret(draft, content_cursor))
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().cream)
            };
            container(field)
                .width(Length::Fill)
                .padding([10, 12])
                .id(content_field_id())
                .style(|_t: &iced_widget::Theme| container::Style {
                    background: None,
                    border: Border {
                        color: byteui::theme::color::current().border,
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    ..container::Style::default()
                })
                .into()
        } else {
            MouseArea::new(container(label).width(Length::Fill))
                .interaction(mouse::Interaction::Pointer)
                .on_press(Message::ContentEditStart(idx))
                .into()
        };
```

改成:

```rust
    let label_area: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if let Some(draft) = editing_draft {
            byteui::form::input_text::view(
                "任务内容…",
                draft,
                false,
                Some(content_field_id()),
                false,
                Some(Message::ContentSubmit),
                true,
                Message::ContentInput,
            )
        } else {
            MouseArea::new(container(label).width(Length::Fill))
                .interaction(mouse::Interaction::Pointer)
                .on_press(Message::ContentEditStart(idx))
                .into()
        };
```

(`bare: true`——理由同浏览器地址栏 Stage 3:原自绘版本的边框描边靠外层
`container` 画,`byteui::form::input_text` 若自己再画一层会跟卡片其它
区域的视觉不统一;这里选 `bare` 是因为原版本本来就没有独立卡片背景
`background: None`,只有一圈描边,`bare` 之后描边整体交回调用方——但
本计划给的 `input_text::view` 调用没有外层 `container` 包一层画边框,
**执行时需要确认**:如果直接去掉 `bare` 让 `input_text` 自己画卡片背景
+ 聚焦金框描边,视觉上可能比精确复刻旧版更合理(旧版那圈灰描边本来就是
"占位视觉",不是刻意的品牌设计);两种做法（`bare: true` 手动包一层
`container` 复刻旧描边,或者直接 `bare: false` 用 byteui 默认卡片视觉)
都能达成功能正确,人工 GUI 走查阶段（Task 4）目测选一个观感更好的,不是
本计划的编译期硬约束,写 `bare: false` 直接省掉额外的 `container` 包装
也是完全合理的简化,按实现时判断取舍。)

- [ ] **Step 8: `todo_list_row` 调用点同步(移除 `content_cursor` 传参)**

`command grep -n "content_cursor" crates/dozer-app/src/extensions/todo.rs`
核对所有残留引用,`todo_list_row` 里传给 `todo_card` 的
`ws_state.content_cursor` 那个实参连同函数调用一并删除(Step 7 已经把
`todo_card` 签名少了这个参数)。

- [ ] **Step 9: `App::set_todo_content_focused`(失焦落盘边缘触发)**

`app.rs` 里新增(位置靠近 `todo_add_focused`/`set_todo_add_focused` 那批
方法):

```rust
    /// Todo 任务内容编辑框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn todo_content_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo.content_edit_focused())
    }

    /// 每帧渲染循环调用:把 `CaptureContentEditFocus` 问到的真实焦点态
    /// 写进当前工作区的 Todo,并在"焦点从真变假"的那一刻做落盘判断
    /// (同 `Workspace::blur_inputs` 原先的"有项目就 commit、没项目就
    /// cancel"逻辑,只是触发时机从"点击别处"改成"真实焦点丢失")。
    pub fn set_todo_content_focused(&mut self, focused: bool) {
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let was_focused = ws.todo.content_edit_focused();
        if was_focused && !focused {
            if let Some(project) = ws.project.as_ref() {
                let path = std::path::PathBuf::from(&project.path);
                ws.todo.commit_content_edit(&path);
            } else {
                ws.todo.cancel_content_edit();
            }
        }
        ws.todo.set_content_edit_focused_flag(focused);
    }
```

(`ws.project`/`ProjectInfo.path` 的确切类型/字段名以
`Workspace::blur_inputs` 现有代码——`crates/dozer-app/src/workspace.rs`
约第 2132-2137 行——为准照抄,上面这段是照那段现状写的,执行前用
`command grep -n "fn blur_inputs" -A25 crates/dozer-app/src/workspace.rs`
核对一致。)

`workspace.rs::blur_inputs()` 里删掉:

```rust
            self.todo.commit_content_edit(path);
```

(这一行的落盘职责已经搬到 `App::set_todo_content_focused` 的边缘触发里,
`blur_inputs` 不再需要管——**注意同一行下面 `else` 分支里的
`self.todo.cancel_content_edit();` 也要删**,原本是"没项目时退回丢弃"
分支,新逻辑里这个判断已经内嵌进 `set_todo_content_focused` 自己的
`if let Some(project) = ... else { cancel }` 里了。)

- [ ] **Step 10: `main.rs` 键盘路由 + 渲染循环接线**

`command grep -n "to_todo_content\|todo_content_editing" crates/dozer-app/src/main.rs crates/dozer-app/src/app.rs`
核对现状。原生放行闸门追加 `|| app.todo_content_focused()`;
`to_self_drawn_input` 链、`addr_message` 闭包里删掉 `to_todo_content`/
`Message::Todo(extensions::todo::Message::ContentEvent(ev))` 那一支(注意
`addr_message` 闭包目前的最终 `else` 分支落在 `Message::Todo(extensions::
todo::Message::MarkdownEvent(ev))`——删掉 `to_todo_content` 分支后,这个
`else` 保持不变,`to_todo_content` 曾经是它前面的一个 `else if`,删掉后
链条继续往下接,不需要额外调整结构)。

`command grep -n "Message::Todo(extensions::todo::Message::ContentEditStart" -A15 crates/dozer-app/src/main.rs`
核对现状,当前形如:

```rust
                Message::Todo(extensions::todo::Message::ContentEditStart(idx)) => {
                    let was_editing = app.todo_content_editing();
                    app.update(Message::Todo(extensions::todo::Message::ContentEditStart(
                        idx,
                    )));
                    if was_editing
                        && let Some(bounds) = extensions::todo::take_content_field_bounds()
                    {
                        let scale = window.scale_factor();
                        let local_x = ((cursor_phys.x / scale) as f32 - bounds.x).max(0.0);
                        app.update(Message::Todo(extensions::todo::Message::ContentCursorAt(
                            local_x,
                        )));
                    }
                }
```

整段替换为(`ContentEditStart` 交回默认分发即可,不再需要点击定位换算,
`other => app.update(other)` 那个兜底分支自然会接住它,这个 `match` 分支
**整体删除**,不用留任何占位):

```rust
                // 任务内容编辑框已迁 iced 原生 text_input(本计划),点击
                // 命中区域内由组件自身接管聚焦与光标定位,不再有
                // `ContentCursorAt` 自绘换算分支,`ContentEditStart` 落到
                // 下面的默认 `other => app.update(other)` 分支即可。
```

在 `let tree_edit_focus_pending = ...`(Task 1 Step 11 加的)紧邻位置补上
同款一次性聚焦标记读取:

```rust
            let content_edit_focus_pending = app
                .active_workspace_mut()
                .is_some_and(|ws| ws.todo.take_content_edit_focus_pending());
```

在 Task 1 加的 `if tree_edit_focus_pending { ... }` 操作块之后追加:

```rust
                                if content_edit_focus_pending {
                                    let mut op = iced_widget::core::widget::operation::focusable::focus::<()>(
                                        extensions::todo::content_field_id(),
                                    );
                                    interface.operate(renderer, &mut op);
                                }
```

在 Task 1 加的每帧焦点查询代码块里追加(gating 条件用
`PanelKind::Todo`):

```rust
                                let content_edit_focused =
                                    if matches!(app.left_view(), crate::app::PanelKind::Todo) {
                                        interface.operate(
                                            renderer,
                                            &mut extensions::todo::CaptureContentEditFocus,
                                        );
                                        extensions::todo::take_content_edit_focused()
                                    } else {
                                        false
                                    };
```

在写回代码块里追加:

```rust
                                app.set_todo_content_focused(content_edit_focused);
```

- [ ] **Step 11: 重写测试**

`command grep -n "ContentEvent\|ContentCursorMove\|ContentCursorAt\|content_editing\|fn.*content.*test" crates/dozer-app/src/extensions/todo.rs`
找到测试模块里对应断言,改法同 Task 1 Step 12(消息名替换、状态断言
改用新访问器)。额外新增一条覆盖"失焦落盘"边缘触发的测试(在 `app.rs`
或 `todo.rs` 测试模块,取决于 `set_todo_content_focused` 最终定义在哪个
文件——按 Step 9 实际落地位置放):验证 `set_todo_content_focused(true)`
→ 改草稿 → `set_todo_content_focused(false)` 后,任务文字确实被写盘
改写(复用现有 `commit_content_edit` 相关测试的项目目录 fixture 手法)。

- [ ] **Step 12: 构建 + 测试 + clippy + fmt**

Run: `cargo build -p dozer-app --bin dozer 2>&1 | grep "error\[" `
Expected: 无残留旧 API 引用错误。

Run: `cargo test -p dozer-app --bin dozer todo:: -- --nocapture 2>&1 | tail -60`
Expected: 全部 PASS。

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "todo.rs|main.rs|app.rs|workspace.rs"; cargo fmt --package dozer-app`
Expected: 无新增警告;fmt 无残留改动。

- [ ] **Step 13: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/todo.rs crates/dozer-app/src/app.rs \
  crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): Todo 任务内容编辑改用真正的 iced text_input,失焦落盘边缘触发迁到焦点查询"
```

---

## Task 3: 全量校验 + 人工 GUI 走查 + 提请审阅

**Files:** 无新增修改,本任务只跑校验与人工验证。

- [ ] **Step 1: 全量构建 + 测试 + lint**

```bash
cargo build --workspace
cargo test -p byteui
cargo test -p dozer-app --bin dozer
cargo clippy --all-targets
cargo fmt --check
```

记录实际 pass/fail 数字,只允许已知的 `git_log` 基线失败(若失败,先确认
没有残留的旧 `feature/native-text-input-*` 分支/worktree 干扰这个测试)。

- [ ] **Step 2: 人工 GUI 走查(不可替代,自动化测不出这些)**

跑 `cargo run -p dozer-app`,两个字段逐一核对:

**项目树行内编辑**:
1. 右键文件/文件夹选"重命名",**不用额外点一下**就能直接打字(一次性
   聚焦机制是否真的生效——这是本计划最容易做错的一步,必须实测)。
2. 右键目录选"新建文件"/"新建文件夹",同样不用额外点击就能直接打字。
3. 真光标显示、方向键移动、鼠标点击定位、拖拽选中、IME 候选框跟随。
4. 敲回车提交改名/新建;点击树外任意位置,编辑框应该消失(丢弃,不是
   落盘——项目树重命名从来没有"失焦保存"这条,跟 Todo 内容编辑不是同一
   行为,不要搞混了去走查错误的预期)。
5. 缩进视觉:多层嵌套目录下触发编辑,确认左缩进量跟同级其它行对齐(Step
   8 提到的"缩进换算方式"这里要目测确认,不只是编译通过)。
6. 背景终端聚焦时,在编辑框里打字/⌘V/方向键,不漏进终端。

**Todo 任务内容编辑**:
1. 点任务卡片文字,**不用额外点一下**就能直接打字。
2. 真光标显示、方向键移动、鼠标点击定位、拖拽选中、IME 候选框跟随。
3. 敲回车提交,任务文字正确改写、列表刷新。
4. **关键行为**:改了文字后点卡片外任意位置(失焦,不敲回车),任务文字
   仍然要被写盘保存(不是丢弃)——这是用户明确反馈过的行为,本计划特意
   保留,走查时务必验证这条,不要只测回车提交那条路径就跳过。
5. 没打开项目(理论上不会发生,Todo 面板依赖项目——如果实测环境下真能
   触发这个分支,确认此时失焦是丢弃不报错,不崩溃)。
6. 背景终端聚焦时,在编辑框里打字/⌘V/方向键,不漏进终端。

- [ ] **Step 3: 确认全部 commit 都在特性分支上**

Run: `git log --oneline main..HEAD`(在
`feature/native-text-input-stage5` 分支上执行)
Expected: 列出 Task 1-2 的两个 commit,`git branch --show-current` 不是
`main`。

- [ ] **Step 4: 提请审阅**

不在这一步直接合并——按仓库惯例提请代码审阅,通过后再合并回 `main`
(参考 `superpowers:finishing-a-development-branch`)。审阅通过合并后,
原 spec 剩下的 3 个非目标字段(验收意见框、项目名称编辑、右键"搜索"
弹窗查询框)待开工前基于合并后的 main 重新核对行号再写——这三个的
触发方式跟 Files/浏览器/首页/Todo 搜索框那批(点输入框本身进入编辑)更
接近,大概率不需要本计划新引入的"一次性程序化聚焦"机制,但要先核实
各自的触发点再下结论,不要想当然照搬。
