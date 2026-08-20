# 输入框改用 iced 原生控件 Stage 6(验收意见框 + 项目名称编辑 + 右键搜索弹窗)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把验收意见框(`acceptance.rs`)、项目名称编辑(`project.rs`)、
右键"搜索"弹窗查询框(`search.rs`)三个自绘输入改成真正的 iced
`text_input`。这三个是原 spec 明确列为"非目标、留给后续"的最后 3 个
自绘输入面——做完这个,spec 点名的 6 个非目标字段全部收尾。

**Architecture:** 完全复用 Stage 5 建立的"每帧查询真实焦点 + 一次性程序化
聚焦"主线(触发点击都落在旧的自绘 `button`/`MouseArea` 上,不是新出现的
`text_input` 本身,三个都需要 `operation::focusable::focus` 强制聚焦,
同 `files::tree_edit_focus_pending`/`todo::content_edit_focus_pending`
的既有手法,不再重复推导)。三处各自的特殊点:

1. **验收意见框**:三个里最简单的——`comment: String` 纯 append/pop,
   `comment_editing: bool` 单标记,失焦即丢弃(不落盘),形状接近 Stage 2
   的 Files 搜索框。
2. **项目名称编辑**:`name_editing: Option<String>`,失焦要**落盘**(调
   daemon `rename_project`),跟 Todo 内容编辑同款"失焦即提交"边缘触发
   模式——但现状代码把这条异步改名逻辑写了两份(回车提交 `NameEditEvent
   (Submit)` 一份、失焦 `App::blur_inputs` 一份,几乎一样的
   `client.rename_project(...)` 调用)。本计划顺带合并成一个共享的
   `submit_name_edit` 函数,两条路径都调用它,不留重复——不是新增范围,
   是把已经存在的"两条路径共享一条落盘逻辑"这个既有设计(`commit_content_
   edit` 那种形状)在这里也做到,当前没做到是历史遗留。
3. **右键搜索弹窗查询框**:唯一一个"两步触发"的字段——右键菜单先打开
   整个弹窗(`SearchOpen`),弹窗内还要再点一下查询框才能打字
   (`QueryEditing(true)`)。本计划顺手让弹窗一打开就自动聚焦查询框(同一套
   一次性聚焦机制,只是触发点从"点查询框"提前到"弹窗打开"),省掉这次
   迁移前就存在的"开了弹窗还要再点一下"这个多余步骤——跟这整个系列"消灭
   触发点击落在别处导致的多余点击"的主线一致。**弹窗还有一个前几个字段
   都没有的按键行为要保留**:main.rs 现有一个专门的"Esc 优先关闭弹窗"闸门
   (约第 862-880 行,与 agent 选择菜单/顶栏新增项目菜单同款处理口径),
   目前是两级(第一下 Esc 退出查询编辑态、第二下才真正关弹窗)。迁移后
   查询框不再有独立的"编辑态"布尔(真控件的聚焦态由 iced 自己管),两级
   Esc 简化成一级——**任何时候按 Esc 都直接关闭整个弹窗**,不再区分"先退出
   编辑态"这个中间态。这是本计划唯一一处**不采用**"迁移字段完全不响应 Esc"
   这条系列既有口径的地方——原因是这个字段是浮层(同 agent 选择菜单/顶栏
   新增项目菜单那批),Esc 关闭浮层是这类弹层的通用能力,不是"取消打字"
   这种可接受丢失的小细节,不能跟着一起丢。

**Tech Stack:** Rust 2024,iced 0.14(`iced_widget::text_input`,
`iced_core::widget::operation::focusable`)。

**Spec:** 本计划不对应 `2026-08-20-native-text-input-adoption-design.md`
的任何目标编号(该 spec 明确把这三个字段列为非目标),是 Stage 5 之后
继续同一轮"非目标字段迁移"工作的延续,brainstorming 阶段确认沿用既有
架构。做完本计划,spec 非目标清单里点名的 6 个自绘输入面(验收意见框/
项目树行内编辑/项目名称编辑/右键搜索弹窗/Todo 任务内容编辑/Todo MARKDOWN
整文件编辑)里,只剩 Todo MARKDOWN 整文件编辑没有迁移计划——该字段是
多行整文件编辑,形状接近 Todo 添加框(text_area),但 spec 从未把它列进
明确的迁移目标,是否要做留给用户决定,不在本计划范围。

## Global Constraints

- **在独立分支上开发,不直接提交 main。** 本仓库有长驻自动化在 main 上持续
  开发(建 worktree 前用 `git -C /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
  worktree list` 确认没有同名 worktree),从 main 当前 tip 拉新 worktree:
  `git worktree add ../dozer-native-text-input-stage6 -b
  feature/native-text-input-stage6 main`。后续所有命令都在这个新 worktree
  目录里跑,每次 `git commit`/`git add` 前先 `git branch --show-current`
  确认不在 `main` 上。
- 下面每个 Task 引用的行号以 2026-08-20 main tip(commit `35f48ab`)为准;
  执行前先用对应的 `grep -n` 命令核对实际行号,若有出入以代码现状为准。
- **`Operation<()>` 的 `traverse` 必须实现成调用传入闭包**(`fn traverse(&mut
  self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a)))
  { operate(self); }`)——[[dozer-operation-traverse-noop-bug]] 记录的
  Critical bug 教训,本计划新增的三个 `CaptureXxxFocus` 必须从一开始写对。
- **一次性程序化聚焦的取用时机**:`take_xxx_focus_pending()` 必须在
  `UserInterface::build(app.view(), ...)` **之前**调用(此时能自由
  `&mut app`),结果存局部变量;`operation::focusable::focus::<()>(id)`
  在 `interface.operate()` 阶段用;每帧焦点查询结果的写回(`app.set_xxx_
  focused(..)`)必须等 `interface.into_cache()` **之后**——同 Stage 5 的
  既有时序,不要搞反。
- 每个 Task 结束都要求对应 crate `cargo build` 干净通过;涉及 `dozer-app`
  的 Task 额外要求 `cargo test -p dozer-app --bin dozer <关键字>` 通过。
- 最终 Task 要求全 workspace `cargo build && cargo test -p byteui -p
  dozer-app --bin dozer && cargo clippy --all-targets && cargo fmt --check`
  全绿,记录实际 pass/fail 数字(已知基线:637 个既有测试;`git_log` 分支
  名 dogfood 环境脆弱性失败不算本计划引入,若失败先确认没有残留的旧
  `feature/native-text-input-*` 分支/worktree)。
- **迁移完成后的人工 GUI 走查不可省略**,Task 5 列出完整走查清单。
- **已知的、经用户确认的行为变化**(验收意见框/项目名称编辑):Esc 不再能
  取消编辑,原生控件不处理 Esc,main.rs 也不再拦截转发,不额外加 Esc 拦截
  闸门——同 Stage 2-5 的既有口径。**右键搜索弹窗例外**(见 Architecture
  一节的说明,Esc 关闭整个浮层这个能力必须保留,只是简化成一级)。

---

## Task 1: 验收意见框改用真正的 `text_input`

**Files:**
- Modify: `crates/dozer-app/src/extensions/acceptance.rs`
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Produces: `pub fn comment_field_id() -> Id`;`pub struct
  CaptureCommentFocus`;`pub fn take_comment_focused() -> bool`;
  `WorkspaceState::comment_focused(&self) -> bool` /
  `set_comment_focused(&mut self, bool)`;
  `WorkspaceState::take_comment_focus_pending(&mut self) -> bool`。

- [ ] **Step 1: 确认当前状态(核对行号)**

```bash
command grep -n "comment_editing\|comment_draft\|CommentClick\|CommentEvent\|clear_comment_editing\|struct AcceptanceSession" crates/dozer-app/src/extensions/acceptance.rs
```

- [ ] **Step 2: `AcceptanceSession` 字段改动**

约第 68-80 行 `AcceptanceSession` 结构体里 `comment_editing: bool` 那行
改成两个字段(一个持续镜像、一个一次性标记,同 `files::tree_edit_focused_
flag`/`tree_edit_focus_pending` 的既有分工):

```rust
    /// 意见框是否持有 iced 内部真实焦点,每帧由 `CaptureCommentFocus` 写入。
    comment_focused: bool,
    /// 一次性标记:点意见框(`CommentClick`)刚触发编辑时置真,main.rs 渲染
    /// 循环取走后用 `operation::focusable::focus` 强制聚焦——点击落在旧的
    /// 自绘 `button` 上,不是新出现的 `text_input` 本身,不会自动带焦点。
    comment_focus_pending: bool,
```

(相应地,`AcceptanceSession` 构造处——约第 137/538 行——把
`comment_editing: false,` 改成 `comment_focused: false, comment_focus_
pending: false,`。)

- [ ] **Step 3: 访问器改动**

约第 28-37 行:

```rust
    pub fn comment_editing(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.comment_editing)
    }

    pub fn clear_comment_editing(&mut self) {
        if let Some(s) = &mut self.session {
            s.comment_editing = false;
        }
    }
```

改成:

```rust
    /// 意见框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn comment_focused(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.comment_focused)
    }

    /// 每帧渲染循环读走 `CaptureCommentFocus` 查到的真实焦点态后写进来。
    pub fn set_comment_focused(&mut self, focused: bool) {
        if let Some(s) = &mut self.session {
            s.comment_focused = focused;
        }
    }

    /// 读走(消费式)一次性聚焦标记,同 `files::take_tree_edit_focus_pending`
    /// 的既有手法。
    pub fn take_comment_focus_pending(&mut self) -> bool {
        self.session
            .as_mut()
            .is_some_and(|s| std::mem::take(&mut s.comment_focus_pending))
    }
```

(`clear_comment_editing` 整个删除——不再需要点击别处手动清编辑态,
`Workspace::blur_inputs` 里调它那一行在 Task 4 一并删。)

- [ ] **Step 4: `comment_field_id`/`CaptureCommentFocus` 新增**

紧邻访问器之后(同 `files::tree_edit_field_id`/`CaptureTreeEditFocus` 的
既有位置手法):

```rust
pub fn comment_field_id() -> Id {
    Id::new("acceptance-comment-box")
}

static COMMENT_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

pub fn take_comment_focused() -> bool {
    *COMMENT_FOCUSED.lock().unwrap()
}

/// 每帧 `interface.operate()` 跑一遍。`traverse` 必须调用传入闭包(见
/// [[dozer-operation-traverse-noop-bug]])。
pub struct CaptureCommentFocus;
impl Operation<()> for CaptureCommentFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&comment_field_id()) {
            *COMMENT_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}
```

(文件顶部 import 补齐 `iced_widget::core::widget::operation::Focusable`/
`iced_widget::core::widget::{Id, Operation}`/`Rectangle`——`command grep
-n "^use" crates/dozer-app/src/extensions/acceptance.rs` 核对现状,这个
文件之前没有任何 `CaptureXxxFocus`,这几个 import 大概率都要新加,不像
files.rs/todo.rs 那样可能已存在。)

- [ ] **Step 5: `Message` 枚举 + `update()` 改动**

约第 106 行 `CommentClick`/`CommentEvent(AddrEvent)` 改成:

```rust
    /// 点意见框进入编辑态(`comment_focus_pending` 置位,main.rs 据此程序
    /// 化聚焦)。
    CommentClick,
    /// 意见框草稿变化(iced `text_input::on_input`,每次给全量当前字符串)。
    CommentInput(String),
```

约第 179-192 行:

```rust
        Message::CommentClick => {
            if let Some(session) = &mut ws_state.session {
                session.comment_editing = true;
            }
        }
        Message::CommentEvent(ev) => {
            if let Some(session) = &mut ws_state.session {
                match ev {
                    AddrEvent::Text(s) => session.comment.push_str(&s),
                    AddrEvent::Backspace => {
                        session.comment.pop();
                    }
                    AddrEvent::Submit | AddrEvent::Cancel => session.comment_editing = false,
                }
            }
        }
```

改成:

```rust
        Message::CommentClick => {
            if let Some(session) = &mut ws_state.session {
                session.comment_focus_pending = true;
            }
        }
        Message::CommentInput(s) => {
            if let Some(session) = &mut ws_state.session {
                session.comment = s;
            }
        }
```

(意见框没有独立的"提交"按钮/回车语义——原版 `AddrEvent::Submit` 跟
`Cancel` 是同一个分支、都只是退出编辑态,不落盘、不触发别的动作,原生
版不需要 `on_submit`,`byteui::form::input_text::view` 的 `on_submit`
参数传 `None`。)

- [ ] **Step 6: `view()` 里替换意见框构造**

约第 393-427 行(先 `command grep -n "let editing = session.comment_editing" -A35
crates/dozer-app/src/extensions/acceptance.rs` 核对确切现状)当前是
`button(text(...)).on_press(Message::CommentClick)` 那套自绘。改成:

```rust
    let editing = session.comment_focused;
    content = content.push(
        container(byteui::form::input_text::view(
            "验收意见…（打回时注回会话）",
            &session.comment,
            false,
            Some(comment_field_id()),
            false,
            None,
            true,
            Message::CommentInput,
        ))
        .width(Length::Fill)
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
    );
```

(`bare: true` + 外层 `container` 复刻旧版视觉,同 Stage 5 Todo 内容编辑的
处理方式;旧版 `button` 本身没有独立 padding,新版按 `byteui::form::
input_text` 内置的 `padding(8)` 走,不用额外包一层——若走查时发现跟旧版
留白观感差太多,在外层 `container` 上加 `.padding(...)` 微调,不是编译期
硬约束。)

- [ ] **Step 7: 重写测试 + 构建 + 测试 + clippy + fmt**

`command grep -n "CommentClick\|CommentEvent\|comment_editing" crates/dozer-app/src/extensions/acceptance.rs`
找到测试模块断言,按 Stage 5 的既有改法重写(`CommentEvent(AddrEvent::
Text(s))` → `CommentInput("..".to_string())`;编辑态断言改用
`comment_focused()`/`set_comment_focused()` 的构造性测试)。

Run: `cargo build -p dozer-app --bin dozer 2>&1 | grep "error\[" `(此步
只关注 acceptance.rs 自身,main.rs/app.rs/workspace.rs 的残留错误留给
Task 4)。

Run: `cargo test -p dozer-app --bin dozer acceptance:: -- --nocapture 2>&1 | tail -40`
Expected: 全部 PASS。

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | grep acceptance.rs; cargo fmt --package dozer-app`

- [ ] **Step 8: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/acceptance.rs
git commit -m "feat(dozer-app): 验收意见框改用真正的 iced text_input"
```

---

## Task 2: 项目名称编辑改用真正的 `text_input`

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Produces: `pub fn name_field_id() -> Id`;`pub struct
  CaptureNameEditFocus`;`pub fn take_name_edit_focused() -> bool`;
  `WorkspaceState::name_edit_focused(&self) -> bool` /
  `set_name_edit_focused_flag(&mut self, bool)`(纯状态镜像,不做提交
  判断,同 Stage 5 Todo 内容编辑的分工);
  `WorkspaceState::take_name_edit_focus_pending(&mut self) -> bool`;
  `pub fn submit_name_edit(ws_state: &mut WorkspaceState, project_id: i64,
  current_name: &str, client: Client, handle: &tokio::runtime::Handle,
  emit: impl Fn(Message) + Send + 'static)`(新增的共享提交函数,合并现有
  两份重复的 `rename_project` 调用)。

- [ ] **Step 1: 确认当前状态(核对行号)**

```bash
command grep -n "name_editing\|NameEditStart\|NameEditEvent\|NameRenamed\|take_name_edit\|fn blur_inputs" crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/app.rs
```

- [ ] **Step 2: `WorkspaceState` 字段改动**

约第 29 行 `name_editing: Option<String>` 保留不动(草稿本体,类型不变),
紧邻新增两个字段(同 Task 1 的分工):

```rust
    /// 名称编辑框是否持有 iced 内部真实焦点,每帧由 `CaptureNameEditFocus`
    /// 写入。
    name_edit_focused: bool,
    /// 一次性标记:点项目名(`NameEditStart`)刚触发编辑时置真。
    name_edit_focus_pending: bool,
```

- [ ] **Step 3: 访问器改动**

约第 75-83 行:

```rust
    pub fn name_editing_is_some(&self) -> bool {
        self.name_editing.is_some()
    }
```

紧邻的 `take_name_edit`(约第 82-83 行,`App::blur_inputs` 用)**保留不动**
——落盘边缘触发仍然需要取走草稿。`name_editing_is_some` 改成:

```rust
    /// 名称编辑框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn name_edit_focused(&self) -> bool {
        self.name_edit_focused
    }

    /// 每帧渲染循环读走 `CaptureNameEditFocus` 查到的真实焦点态后写进来。
    /// **只更新焦点镜像标记,不做提交判断**——落盘需要 `project_id`/
    /// `client`,`WorkspaceState` 自己拿不到,这个判断在
    /// `App::set_project_name_focused` 里做(见 Task 4)。
    pub fn set_name_edit_focused_flag(&mut self, focused: bool) {
        self.name_edit_focused = focused;
    }

    /// 读走(消费式)一次性聚焦标记。
    pub fn take_name_edit_focus_pending(&mut self) -> bool {
        std::mem::take(&mut self.name_edit_focus_pending)
    }
```

- [ ] **Step 4: `name_field_id`/`CaptureNameEditFocus` 新增**

同 Task 1 Step 4 手法,新增:

```rust
pub fn name_field_id() -> Id {
    Id::new("project-name-edit-box")
}

static NAME_EDIT_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

pub fn take_name_edit_focused() -> bool {
    *NAME_EDIT_FOCUSED.lock().unwrap()
}

/// 每帧 `interface.operate()` 跑一遍。`traverse` 必须调用传入闭包(见
/// [[dozer-operation-traverse-noop-bug]])。
pub struct CaptureNameEditFocus;
impl Operation<()> for CaptureNameEditFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&name_field_id()) {
            *NAME_EDIT_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}
```

- [ ] **Step 5: 新增共享的 `submit_name_edit` 函数(合并两份重复的改名逻辑)**

`command grep -n "fn update" -A5 crates/dozer-app/src/extensions/project.rs`
核对 `update()` 函数签名(需要 `client`/`handle`/`emit` 这几个参数——本文件
`update()` 现有签名应该已经带这些,照抄参数类型)。在 `update()` 函数
之外新增一个自由函数:

```rust
/// 项目名称编辑的共享提交逻辑:回车提交(`NameEditSubmit`)与失焦提交
/// (`App::set_project_name_focused` 的边缘触发)两条路径共用,避免两份
/// 重复的 `client.rename_project` 调用(现状历史遗留,这次一并合并)。
/// 空名字/未改动直接退出编辑态,不发请求。
pub fn submit_name_edit(
    ws_state: &mut WorkspaceState,
    project_id: i64,
    current_name: &str,
    client: Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let Some(raw) = ws_state.name_editing.take() else {
        return;
    };
    let name = raw.trim().to_string();
    if name.is_empty() || name == current_name {
        return;
    }
    handle.spawn(async move {
        let result = client
            .rename_project(project_id, &name)
            .await
            .map_err(|e| e.to_string())
            .and_then(|opt| opt.ok_or_else(|| "项目不存在".to_string()));
        emit(Message::NameRenamed(project_id, result));
    });
}
```

(**执行前必须先跑 Step 1 的 grep 拿到 `NameEditEvent(Submit)` 分支与
`App::blur_inputs` 里那份的逐字精确代码**,确认上面这个合并版本的行为
跟两份原始逻辑完全一致——尤其注意原 `NameEditEvent(Submit)` 分支在
"空名或未改动"时会显式把 `ws_state.name_editing = None`,上面用
`.take()` 已经隐含做到这一步,不需要额外再清一次;但如果原两份逻辑之间
有任何这次 grep 才会发现的细节差异,以现状代码的真实行为为准,不要
盲信本计划这段复原代码。)

- [ ] **Step 6: `Message` 枚举 + `update()` 改动**

约第 127-128 行 `NameEditStart`/`NameEditEvent(AddrEvent)` 改成:

```rust
    /// 点项目名进入编辑态(`name_edit_focus_pending` 置位)。
    NameEditStart,
    /// 名称编辑框草稿变化(iced `text_input::on_input`)。
    NameEditInput(String),
    /// 回车提交(与失焦提交共用 `submit_name_edit`)。
    NameEditSubmit,
```

约第 205-236 行(先核对确切现状,行号以 Step 1 grep 结果为准)当前形如:

```rust
        Message::NameEditStart => {
            ws_state.name_editing = Some(current_name.to_string());
        }
        Message::NameEditEvent(ev) => match ev {
            AddrEvent::Text(s) => {
                if let Some(buf) = &mut ws_state.name_editing {
                    buf.push_str(&s);
                }
            }
            AddrEvent::Backspace => {
                if let Some(buf) = &mut ws_state.name_editing {
                    buf.pop();
                }
            }
            AddrEvent::Cancel => ws_state.name_editing = None,
            AddrEvent::Submit => {
                let Some(raw) = ws_state.name_editing.clone() else {
                    return;
                };
                let name = raw.trim().to_string();
                if name.is_empty() || name == current_name {
                    ws_state.name_editing = None;
                    return;
                }
                let client = client.clone();
                handle.spawn(async move {
                    let result = client
                        .rename_project(project_id, &name)
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|opt| opt.ok_or_else(|| "项目不存在".to_string()));
                    emit(Message::NameRenamed(project_id, result));
                });
            }
        },
```

改成:

```rust
        Message::NameEditStart => {
            ws_state.name_editing = Some(current_name.to_string());
            ws_state.name_edit_focus_pending = true;
        }
        Message::NameEditInput(s) => {
            if let Some(buf) = &mut ws_state.name_editing {
                *buf = s;
            }
        }
        Message::NameEditSubmit => {
            submit_name_edit(ws_state, project_id, current_name, client.clone(), handle, emit);
        }
```

- [ ] **Step 7: `view()` 里替换名称编辑框构造**

约第 397-425 行(先核对确切现状)当前是 `container(text(format!(
"{buf}▏")))` 自绘展示 + 一个独立的 `button(...).on_press(Message::
NameEditStart)` 非编辑态展示。改成:

```rust
    let editing = ws_state.name_edit_focused();
    let name_row: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        if ws_state.name_editing.is_some() {
            container(byteui::form::input_text::view(
                "",
                ws_state.name_editing.as_deref().unwrap_or(""),
                false,
                Some(name_field_id()),
                false,
                Some(Message::NameEditSubmit),
                true,
                Message::NameEditInput,
            ))
            .padding([8, 12])
            .width(Length::Fill)
            .style(move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(byteui::theme::color::current().card.into()),
                border: Border {
                    color: if editing {
                        byteui::theme::color::current().gold
                    } else {
                        byteui::theme::color::current().border
                    },
                    width: 1.5,
                    radius: 8.0.into(),
                },
                ..iced_widget::container::Style::default()
            })
            .into()
        } else {
            button(
                text(p.name.clone())
                    .size(byteui::theme::font::title())
                    .color(byteui::theme::color::current().cream),
            )
            .on_press(Message::NameEditStart)
            .style(|_t, _s| iced_widget::button::Style {
                background: None,
                text_color: byteui::theme::color::current().cream,
                ..iced_widget::button::Style::default()
            })
            .into()
        };
```

(标题字号 `byteui::theme::font::title()` 跟正文 `body()` 不一样——
`byteui::form::input_text::view` 内部固定用 `crate::theme::font::body()`
渲染,不接受外部传字号,这里会造成编辑态时字号比非编辑态展示态小一圈的
视觉差异。**这是一个已知的、需要人工 GUI 走查确认是否可接受的观感变化**
——如果实测差异明显,需要给 `byteui::form::input_text::view` 追加一个
可选字号覆盖参数,不在本计划的编译期硬约束范围,先落地功能,视觉细节
留给 Task 5 走查后判断要不要追加处理。)

- [ ] **Step 8: 重写测试 + 构建 + 测试 + clippy + fmt**

`command grep -n "NameEditStart\|NameEditEvent\|name_editing_is_some" crates/dozer-app/src/extensions/project.rs`
找到测试模块断言,按 Stage 5 的既有改法重写。

Run: `cargo build -p dozer-app --bin dozer 2>&1 | grep "error\[" `

Run: `cargo test -p dozer-app --bin dozer project:: -- --nocapture 2>&1 | tail -50`
Expected: 全部 PASS。

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | grep project.rs; cargo fmt --package dozer-app`

- [ ] **Step 9: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/project.rs
git commit -m "feat(dozer-app): 项目名称编辑改用真正的 iced text_input,合并两份重复的改名提交逻辑"
```

---

## Task 3: 右键搜索弹窗查询框改用真正的 `text_input`(含自动聚焦 + Esc 简化)

**Files:**
- Modify: `crates/dozer-app/src/extensions/search.rs`
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Produces: `pub fn query_field_id() -> Id`;`pub struct
  CaptureQueryFocus`;`pub fn take_query_focused() -> bool`;
  `WorkspaceState::query_focused(&self) -> bool` /
  `set_query_focused(&mut self, bool)`;
  `WorkspaceState::take_query_focus_pending(&mut self) -> bool`
  (触发点从"点查询框"改成"弹窗打开",见下)。

- [ ] **Step 1: 确认当前状态(核对行号)**

```bash
command grep -n "query_editing\|QueryEvent\|QueryEditing\|SearchOpen\|SearchClose\|fn open\b" crates/dozer-app/src/extensions/search.rs
```

- [ ] **Step 2: `WorkspaceState` 字段改动**

约第 106-114 行 `query_editing: bool` 改成:

```rust
    /// 查询框是否持有 iced 内部真实焦点,每帧由 `CaptureQueryFocus` 写入。
    query_focused: bool,
    /// 一次性标记:弹窗打开(`open()`)时置真——不再等用户点一下查询框才
    /// 进编辑态,打开即自动聚焦,省掉这次迁移前就存在的多余一次点击
    /// (原版 `QueryEditing(true)` 需要额外点击触发,见本计划 Architecture
    /// 一节的说明)。
    query_focus_pending: bool,
```

- [ ] **Step 3: 访问器改动**

约第 118-123 行 `query_editing()` 改成:

```rust
    pub fn query_focused(&self) -> bool {
        self.query_focused
    }

    /// 每帧渲染循环读走 `CaptureQueryFocus` 查到的真实焦点态后写进来。
    pub fn set_query_focused(&mut self, focused: bool) {
        self.query_focused = focused;
    }

    /// 读走(消费式)一次性聚焦标记。
    pub fn take_query_focus_pending(&mut self) -> bool {
        std::mem::take(&mut self.query_focus_pending)
    }
```

- [ ] **Step 4: `query_field_id`/`CaptureQueryFocus` 新增**

同 Task 1/2 手法新增(位置紧邻访问器之后)。

- [ ] **Step 5: `open()`/`Message`/`update()` 改动**

约第 147-153 行 `open()` 末尾补上一行 `ws.query_focus_pending = true;`
(弹窗一打开就置位一次性聚焦标记,不用等 `QueryEditing(true)` 这个额外
点击)。

`Message` 枚举里(约第 128-135 行)当前:

```rust
    QueryEvent(crate::workspace::AddrEvent),
    QueryEditing(bool),
```

改成:

```rust
    /// 查询框草稿变化(iced `text_input::on_input`)。
    QueryInput(String),
```

(`QueryEditing(bool)` 整体删除——不再需要点击查询框手动切换编辑态,
`open()` 已经自动置好聚焦标记。)

`update()` 里(约第 166-184 行)当前:

```rust
        Message::QueryEvent(ev) => {
            if !ws.query_editing {
                return;
            }
            match ev {
                crate::workspace::AddrEvent::Text(s) => ws.query.push_str(&s),
                crate::workspace::AddrEvent::Backspace => {
                    ws.query.pop();
                }
                crate::workspace::AddrEvent::Cancel => ws.query_editing = false,
                crate::workspace::AddrEvent::Submit => {
                    ws.query_editing = false;
                    update(ws, Message::QuerySubmit, project_id, handle, emit);
                }
            }
        }
        Message::QueryEditing(b) => ws.query_editing = b,
```

改成:

```rust
        Message::QueryInput(s) => ws.query = s,
```

(`QuerySubmit` 消息本身**保留不动**——回车提交交给 `byteui::form::
input_text::view` 的 `on_submit` 参数,不需要 `update()` 里再手写一层
转发。)

- [ ] **Step 6: `query_box()` 视图改动**

约第 218-245 行(先 `command grep -n "fn query_box" -A35 crates/dozer-app/src/extensions/search.rs`
核对确切现状)当前是 `button(text(...)).on_press(Message::QueryEditing
(true))` 自绘。改成:

```rust
fn query_box(
    ws: &WorkspaceState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let active = ws.query_focused() || ws.running;
    byteui::form::input_text::view(
        "搜索内容…",
        &ws.query,
        false,
        Some(query_field_id()),
        active,
        Some(Message::QuerySubmit),
        false,
        Message::QueryInput,
    )
}
```

(**执行前先跑上面的 grep 核对 `query_box` 后续还有没有其它渲染逻辑
(比如外层容器/hover 处理)**,只替换函数体内 `body`/`button` 那部分,
不要连同调用方的外层布局一起动;`active` 语义照抄原版"编辑中或搜索
进行中都持续高亮"的逻辑,不是本计划新增。)

- [ ] **Step 7: main.rs 的 Esc 双级简化成单级**

`command grep -n "search_popup_editing()" -B5 -A15 crates/dozer-app/src/main.rs`
核对现状(约第 862-880 行,与 agent 选择菜单同款 Esc 优先关闭处理口径)
当前形如:

```rust
                if app.search_popup_editing() {
                    app.update(Message::Search(extensions::search::Message::QueryEvent(
                        workspace::AddrEvent::Cancel,
                    )));
                } else {
                    app.update(Message::Search(extensions::search::Message::SearchClose));
```

改成(不再区分"先退出编辑态"这个中间态,任何时候 Esc 直接关闭整个弹窗
——`QueryEvent`/`AddrEvent::Cancel` 已经不存在了,这是本计划**唯一**
保留 Esc 拦截的字段,理由见 Architecture 一节):

```rust
                {
                    app.update(Message::Search(extensions::search::Message::SearchClose));
```

(**执行前先跑上面的 grep 拿到这段完整的 `if`/`else` 结构逐字精确代码,
包括外层的 `if app.search_popup_open() && ...` 判断条件本身**——上面
只给出了内层要改的那一段,外层条件判断不受影响,原样保留,只是把
内层的 if/else 两分支合并成一个无条件分支;改完之后这段代码的花括号
配对要重新核对,不要因为删掉一层 `if/else` 导致大括号数量对不上。)

- [ ] **Step 8: 重写测试 + 构建 + 测试 + clippy + fmt**

`command grep -n "QueryEvent\|QueryEditing\|query_editing" crates/dozer-app/src/extensions/search.rs`
找到测试模块断言,按 Stage 5 的既有改法重写(额外验证 `open()` 之后
`query_focus_pending()` 应为真——覆盖"打开即自动聚焦"这条新行为)。

Run: `cargo build -p dozer-app --bin dozer 2>&1 | grep "error\[" `

Run: `cargo test -p dozer-app --bin dozer search:: -- --nocapture 2>&1 | tail -40`
Expected: 全部 PASS。

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | grep search.rs; cargo fmt --package dozer-app`

- [ ] **Step 9: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/extensions/search.rs
git commit -m "feat(dozer-app): 右键搜索弹窗查询框改用真正的 iced text_input,打开即自动聚焦"
```

---

## Task 4: `main.rs` 路由收尾 + `App`/`Workspace` 接线 + 清理

**Files:**
- Modify: `crates/dozer-app/src/main.rs`
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/workspace.rs`

- [ ] **Step 1: 确认当前状态(核对行号,注意 Task 1-3 的独立 commit 落地后
  main.rs 行号会挪动)**

```bash
command grep -n "to_comment\|to_project_name\|to_search_popup\|acceptance_comment_editing\|project_name_editing\|search_popup_editing" crates/dozer-app/src/main.rs crates/dozer-app/src/app.rs crates/dozer-app/src/workspace.rs
```

- [ ] **Step 2: `Workspace`/`App` 新增委托访问器(三套,同 Stage 5 的既有
  写法照抄三遍)**

`workspace.rs` 里新增(位置靠近 `tree_edit_focused` 那批委托):

```rust
    pub fn comment_focused(&self) -> bool {
        self.acceptance.comment_focused()
    }

    pub fn take_comment_focus_pending(&mut self) -> bool {
        self.acceptance.take_comment_focus_pending()
    }

    pub fn name_edit_focused(&self) -> bool {
        self.project_panel.name_edit_focused()
    }

    pub fn take_name_edit_focus_pending(&mut self) -> bool {
        self.project_panel.take_name_edit_focus_pending()
    }

    pub fn query_focused(&self) -> bool {
        self.search.query_focused()
    }

    pub fn take_query_focus_pending(&mut self) -> bool {
        self.search.take_query_focus_pending()
    }
```

(**`ws.acceptance`/`ws.project_panel`/`ws.search` 这三个字段名是根据
`Workspace` 结构体既有字段命名推断的合理猜测——执行前先跑 `command grep
-n "acceptance:\|project_panel:\|search:" crates/dozer-app/src/workspace.rs`
核对 `Workspace` 结构体里这三个 extension 状态字段的真实名字,按实际
名字改,不要直接照抄上面的猜测。**)

`app.rs` 里新增(位置靠近 `tree_edit_focused`/`set_tree_edit_focused`
那批方法):

```rust
    pub fn comment_focused(&self) -> bool {
        self.active_workspace().is_some_and(|ws| ws.comment_focused())
    }

    pub fn set_comment_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.acceptance.set_comment_focused(focused);
        }
    }

    pub fn project_name_focused(&self) -> bool {
        self.active_workspace().is_some_and(|ws| ws.name_edit_focused())
    }

    /// 每帧渲染循环调用:把 `CaptureNameEditFocus` 问到的真实焦点态写进
    /// 当前工作区,并在"焦点从真变假"的那一刻做落盘判断(同 `App::
    /// set_todo_content_focused` 的既有手法,`submit_name_edit` 走这条
    /// 而不是重新手写一份 `rename_project` 调用)。
    pub fn set_project_name_focused(&mut self, focused: bool) {
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let was_focused = ws.name_edit_focused();
        if was_focused && !focused
            && let Some(project) = ws.project.clone()
        {
            let client = self.client.clone();
            let handle = self.handle.clone();
            let proxy = self.proxy.clone();
            let emit = move |m: project::Message| {
                let _ = proxy.send_event(Message::Project(m));
            };
            extensions::project::submit_name_edit(
                &mut ws.project_panel,
                project.id,
                &project.name,
                client,
                &handle,
                emit,
            );
        }
        ws.project_panel.set_name_edit_focused_flag(focused);
    }

    pub fn query_focused(&self) -> bool {
        self.active_workspace().is_some_and(|ws| ws.query_focused())
    }

    pub fn set_query_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.search.set_query_focused(focused);
        }
    }
```

(`set_project_name_focused` 里的 `extensions::project::submit_name_edit`
调用签名要跟 Task 2 Step 5 实际落地的签名对上;`self.client`/`self.
handle`/`self.proxy` 这几个 `App` 字段的确切名字/类型,照抄
`App::blur_inputs` 现有代码——`command grep -n "fn blur_inputs" -A20
crates/dozer-app/src/app.rs` 核对——那段代码本来就在算同样的
`client`/`handle`/`proxy`/`emit` 组合,直接抄现状写法,不要凭空猜。)

`workspace.rs::blur_inputs()` 里删掉:

```rust
        self.acceptance.clear_comment_editing();
```

(方法已在 Task 1 删除。)`app.rs::blur_inputs()` 里删掉取
`pending_name`/发起改名请求那一整段(约第 3467-3495 行,Stage 6 之前的
"App::blur_inputs" 那段代码,已经整体搬进 `set_project_name_focused`
的边缘触发)——**只删这一段落盘逻辑,`ws.blur_inputs()` 那行调用与其它
无关逻辑保留不动**。

- [ ] **Step 3: `main.rs` 原生放行闸门 + `to_self_drawn_input` 清理**

`command grep -n "if app.files_search_focused" -A20 crates/dozer-app/src/main.rs`
核对现状后追加:

```rust
                || app.comment_focused()
                || app.project_name_focused()
                || app.query_focused()
```

`to_self_drawn_input` 链、`addr_message` 闭包里删掉 `to_comment`/
`to_project_name`/`to_search_popup` 三支(同 Stage 2-5 的既有删除手法);
`addr_message` 闭包删完这三支后应该只剩 `to_todo_content`(已在 Stage 5
删除,这里核对是不是已经不剩任何分支、只剩一个无条件的 `Message::Todo
(extensions::todo::Message::MarkdownEvent(ev))` 兜底——**如果确实如此,
整个 `addr_message` 闭包和 `to_self_drawn_input`/`if to_self_drawn_input
{ ... }` 这一大段可能已经退化成"只服务 Todo MARKDOWN 编辑一个字段",
是否要进一步简化成专门的 `if to_todo_markdown { ... }` 而不是保留这套
"通用自绘输入路由"框架,是一个值得判断的简化机会,但不是本计划的编译期
硬约束——先确认三支删完后代码还能正确编译运行,简化与否留给实现时或
未来单独一轮再看**)。

- [ ] **Step 4: 渲染循环接入三处一次性聚焦 + 每帧焦点查询**

`command grep -n "let tree_edit_focus_pending" -A5 crates/dozer-app/src/main.rs`
核对 Stage 5 建的一次性位取用点,紧邻追加:

```rust
            let comment_focus_pending = app
                .active_workspace_mut()
                .is_some_and(|ws| ws.take_comment_focus_pending());
            let name_edit_focus_pending = app
                .active_workspace_mut()
                .is_some_and(|ws| ws.take_name_edit_focus_pending());
            let query_focus_pending = app
                .active_workspace_mut()
                .is_some_and(|ws| ws.take_query_focus_pending());
```

`command grep -n "if tree_edit_focus_pending" -A7 crates/dozer-app/src/main.rs`
核对 Stage 5 建的 `operation::focusable::focus` 消费点,紧邻追加三段同款
(分别用 `extensions::acceptance::comment_field_id()`/`extensions::
project::name_field_id()`/`extensions::search::query_field_id()`)。

`command grep -n "let tree_edit_focused =" -A11 crates/dozer-app/src/main.rs`
核对 Stage 5 建的每帧焦点查询代码块,紧邻追加三段同款(gating 条件:
验收意见框用 `PanelKind::Acceptance`,项目名称编辑跟"项目信息面板"绑定
——**先确认项目名称编辑框实际渲染在哪个 `PanelKind` 下,`command grep -n
"PanelKind::" crates/dozer-app/src/app.rs | head -20` 核对枚举有哪些
取值,项目名称大概率挂在首页或 `PanelKind::Project`,不要凭空假设**;
右键搜索弹窗是全局浮层,不挂靠任何 `left_view`,gating 条件用
`app.search_popup_open()` 而不是 `PanelKind` 匹配)。

`command grep -n "app.set_tree_edit_focused" -A3 crates/dozer-app/src/main.rs`
核对写回代码块,紧邻追加:

```rust
                                app.set_comment_focused(comment_focused);
                                app.set_project_name_focused(name_edit_focused);
                                app.set_query_focused(query_focused);
```

- [ ] **Step 5: `ime_cursor_area` 清理**

约第 4043 行(先 `command grep -n "fn ime_cursor_area" -A15 crates/dozer-app/src/app.rs`
核对确切现状)`if self.acceptance_comment_editing() { ... }` 分支——
**注意这个访问器名字在 Task 4 Step 2/3 会被改成 `comment_focused()`,
这里的判断条件要同步改名,不是删除整个分支**(验收意见框仍然需要 IME
候选框跟随光标,只是判断条件的方法名变了;不要照抄 Stage 3 浏览器地址栏
"整个分支删除"的处理方式——那次是因为迁移后 iced 自己算准确坐标,这次
验收意见框同样迁移到原生控件,**理由相同,这个分支其实也应该整个删除
而不是改名**——执行时两种处理方式选哪个,以"迁移后 iced 是否自己算准
IME 位置"这条判断为准,跟浏览器地址栏 Stage 3 的删除理由完全一样,应该
删除整个 `if` 分支,不是照抄改名;上面先写"改名"是为了提醒这处易错点,
实际操作请执行"删除整个分支",同 Stage 3 `browser_addr_editing()` 分支
的处理方式)。

- [ ] **Step 6: 全量编译确认**

Run: `cargo build --workspace 2>&1 | grep "error\[" `
Expected: 无输出。

- [ ] **Step 7: 跑 dozer-app 全量测试**

Run: `cargo test -p dozer-app --bin dozer 2>&1 | tail -30`
Expected: 全绿(已知基线:`git_log` 分支名 dogfood 环境脆弱性)。

- [ ] **Step 8: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets && cargo fmt --package dozer-app`

- [ ] **Step 9: Commit**

```bash
git branch --show-current
git add crates/dozer-app/src/main.rs crates/dozer-app/src/app.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): main.rs 键盘路由为验收意见框/项目名称编辑/搜索弹窗新增原生输入放行判断"
```

---

## Task 5: 全量校验 + 人工 GUI 走查 + 提请审阅

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

**验收意见框**:
1. 点意见框,不用额外点一下就能直接打字。
2. 真光标/方向键/鼠标定位/拖拽选中/IME 候选框跟随。
3. 点别处失焦,草稿丢弃(不落盘——这是验收意见框现状既有行为,跟项目
   名称/Todo 内容编辑不是同一类)。
4. 背景终端聚焦时不漏键。

**项目名称编辑**:
1. 点项目名,不用额外点一下就能直接打字。
2. 编辑态字号跟非编辑态展示态是否有肉眼可见差异(Task 2 Step 7 提到的
   已知细节,确认是否需要追加字号覆盖参数)。
3. 敲回车提交改名;点别处失焦也要能正确改名(不是丢弃)——这是本计划
   保留的关键行为,尤其要测"失焦提交"这条路径,不要只测回车提交就跳过。
4. 空名字/未改名时点别处失焦,不应该发起任何改名请求。
5. 背景终端聚焦时不漏键。

**右键搜索弹窗**:
1. 右键文件/目录选"搜索",弹窗打开后**不用再点一下查询框**就能直接
   打字(本计划新增的"打开即聚焦",重点验证)。
2. 真光标/方向键/鼠标定位/拖拽选中/IME 候选框跟随。
3. 敲回车/点搜索按钮触发搜索。
4. 按 Esc 直接关闭整个弹窗(简化后的单级行为,不再有"先退出编辑态"
   这个中间态,确认现在的单级关闭符合直觉,不会让用户觉得"按一下 Esc
   就把还没搜完的东西弄没了"这种意外感——如果实测体感不好,这是本计划
   里少数几个"实现完了但可能需要根据实测调整"的设计决策点,记下来跟
   用户确认是否要恢复两级)。
5. 背景终端聚焦时不漏键。

- [ ] **Step 3: 确认全部 commit 都在特性分支上**

Run: `git log --oneline main..HEAD`(在
`feature/native-text-input-stage6` 分支上执行)
Expected: 列出 Task 1-4 的四个 commit,`git branch --show-current` 不是
`main`。

- [ ] **Step 4: 提请审阅**

不在这一步直接合并——按仓库惯例提请代码审阅,通过后再合并回 `main`
(参考 `superpowers:finishing-a-development-branch`)。审阅通过合并后,
spec 非目标清单里点名的字段只剩 Todo MARKDOWN 整文件编辑没有迁移计划
——是否要做、什么时候做,留给用户决定,不在本计划范围内提前定案。
