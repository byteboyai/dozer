# 文件预览去模态编辑 + 代码/预览切换 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 去掉文件预览面板(File 预览 + Project 面板右配对预览)的 tab 右键菜单和全局编辑弹层这两套模态/浮层 UI,改为在 tab 上(复用 `panel_tab` 现成的 `suffix` 插槽)加一个预览/代码切换按钮——只对"默认走 wry/flyfish 渲染的文本可编辑文件"(如 `.md`/`.html`)出现,切到代码模式即复用既有的原生 tab 就地编辑机制;任何可能丢改动的操作(切回预览、关闭 tab)都先静默保存,不留确认框。

**Architecture:** 数据模型几乎不变——`PreviewTab.editor: Option<CodeView>` 本来就是 `desired_webviews()` 判定"这个 tab 要不要长 wry webview"的唯一开关,这次只是让它在 `push_tab` 决定一次之后也能被运行时切换。新增 `preview.rs::wry_toggle_eligible()` 判定函数(组合两个既有判据)、`PreviewPane::enter_code_mode`/`exit_code_mode` 两个纯状态切换方法、`Workspace::preview_pane_toggle_render_mode`/`preview_pane_save_at` 两个编排方法、`tab_widget.rs::tab_render_mode_button` 一个渲染函数。移除全部右键菜单与编辑弹层相关的 Message、状态字段、渲染函数、键盘路由拦截。

**Tech Stack:** Rust workspace,iced 0.14,`byteui` 共享组件库。

**Spec:** `docs/superpowers/specs/2026-09-09-preview-edit-mode-toggle-design.md`

## Global Constraints

- File 预览与 Project 面板右配对预览两个面板都要接入、行为一致(`preview_pane_for` 同一份函数服务两者)。
- 不新增任何确认弹窗/模态类 UI 作为被移除项的替代品——去模态是明确目标,未保存改动一律走静默保存,不阻断操作。
- 切换按钮只在 `is_editable_extension(path) && prefers_rendered_preview(path)` 为真的 tab 上出现;本来就默认原生可编辑的文件(`.rs`/`.py`/`.json` 等)和真二进制文件(图片/PDF/压缩包)都不出现。
- "刷新"(`PreviewReload`/`ProjectPreviewReload`)随右键菜单一起去掉,不补替代入口。
- 关闭一个 `dirty=true` 的原生可编辑 tab 前,先静默保存——这个行为对所有原生可编辑 tab 生效,不局限于新增的切换类型。

---

### Task 1: 移除预览 tab 右键菜单(含"刷新")

**Files:**
- Modify: `crates/dozer-app/src/app.rs`(Message 枚举 `PreviewTabContextMenu`/`PreviewTabContextMenuClose`/`ProjectPreviewTabContextMenu`/`ProjectPreviewTabContextMenuClose`/`PreviewReload`/`ProjectPreviewReload` 共 6 个变体定义,约 1849-1952 行区间;对应 `update` 处理分支,约 5210-5217、5296-5310、5340-5359 行区间;`PreviewTabMenu` struct 定义 2082-2090 行、字段 2267/2270 行、初始化 2647/2648 行;`preview_tab_context_menu_popup`/`project_preview_tab_context_menu_popup` 两个函数,7567-7620、7626-约 7680 行;`view()` 里对应的 `stack!` 分支,约 8257-8278 行)
- Modify: `crates/dozer-app/src/workspace.rs`(`preview_pane_for` 里的 `context_msg` 闭包定义 3634-3637 行、`on_right_press(context_msg(idx, editable))` 及局部 `editable` 变量,3673、3699 行;"刷新"落地方法 `preview_reload`/`project_preview_reload`/`preview_reload_for`,749-818 行区间——注意这三个方法紧挨着 `preview_edit_open` 系列,删除时只删这一段,`preview_edit_*` 留给 Task 2)

**Interfaces:**
- Consumes: 无(纯删除)
- Produces: 无(叶子改动,Task 2/3 不依赖此任务的产出,但建议先做,代码库更干净)

- [ ] **Step 1: 删除 app.rs 里的 6 个 Message 变体**

删除 `app.rs:1849-1852`:

```rust
    /// 预览:点 tab chip 上的"编辑"按钮,携带 tab 下标(渲染时发出,和
    /// `PreviewSelectTab`/`PreviewCloseTab` 同一约定)。
    PreviewEditOpen(usize),
    /// 预览 tab 右键菜单里的"刷新":从文件系统重新读盘并重新渲染该 tab
    /// (下标即 vec 位置,与 `PreviewCloseTab` 同约定)。
    PreviewReload(usize),
```

删除 `app.rs:1919-1927`:

```rust
    /// 预览 tab 右键菜单:在 `preview_pane` 某 tab 上右键打开,携带 tab 下标
    /// 与该文件是否可编辑(仅文本类文件,决定菜单"编辑"项是否出现)。定位
    /// 坐标复用 `files.last_right_click`(main.rs 右键时写入)。
    PreviewTabContextMenu {
        idx: usize,
        editable: bool,
    },
    /// 预览 tab 右键菜单关闭(点遮罩 / 按 Esc)。
    PreviewTabContextMenuClose,
```

删除 `app.rs:1942-1952`(注意保留 `ProjectPreviewEditorEvent` 那一条,只删 `ProjectPreviewEditOpen`/`ProjectPreviewReload`/`ProjectPreviewTabContextMenu`/`ProjectPreviewTabContextMenuClose`):

```rust
    /// Project 面板右配对预览:右键菜单里的"编辑"项,语义同 `PreviewEditOpen`。
    ProjectPreviewEditOpen(usize),
    /// Project 面板右配对预览 tab 右键菜单里的"刷新",语义同 `PreviewReload`。
    ProjectPreviewReload(usize),
    /// Project 面板右配对预览 tab 右键菜单,语义同 `PreviewTabContextMenu`。
    ProjectPreviewTabContextMenu {
        idx: usize,
        editable: bool,
    },
    /// Project 面板右配对预览 tab 右键菜单关闭。
    ProjectPreviewTabContextMenuClose,
```

Run: `cargo build -p dozer-app 2>&1 | tail -80`
Expected: 一堆 `no variant`/`unused` 编译错(update 分支/context_msg/context_menu_popup 还在引用这些变体)——继续后续 Step。

- [ ] **Step 2: 删除 app.rs::update 里对应的处理分支**

删除 `app.rs:5210-5217`:

```rust
            Message::PreviewEditOpen(idx) => {
                self.preview_tab_menu = None;
                self.with_focused_project(move |ws, _io| ws.preview_edit_open(idx));
            }
            Message::PreviewReload(idx) => {
                self.preview_tab_menu = None;
                self.with_focused_project(move |ws, _io| ws.preview_reload(idx));
            }
```

(`PreviewEditOpen` 分支整段删——它调 `preview_edit_open`,那个方法本身在 Task 2 才删,这里先只删 Message 分支,方法暂时变成"没有调用方"的 dead code,留给 Task 2 一并清)

删除 `app.rs:5296-5310`:

```rust
            Message::PreviewTabContextMenu { idx, editable } => {
                let (x, y) = self.files.last_right_click();
                // 与文件树右键菜单互斥——避免两者同时挂着,关掉一个把另一个
                // 意外顶出来。
                self.files.close_context_menu();
                self.preview_tab_menu = Some(PreviewTabMenu {
                    x,
                    y,
                    idx,
                    editable,
                });
            }
            Message::PreviewTabContextMenuClose => {
                self.preview_tab_menu = None;
            }
```

删除 `app.rs:5340-5359`(`ProjectPreviewEditOpen`/`ProjectPreviewReload`/`ProjectPreviewTabContextMenu`/`ProjectPreviewTabContextMenuClose` 四段,`ProjectPreviewEditorEvent`/`ProjectPreviewOpenPath`/`ProjectPreviewSelectTab`/`ProjectPreviewCloseTab`/`ProjectPreviewTabOverflow*` 保留):

```rust
            Message::ProjectPreviewEditOpen(idx) => {
                self.project_preview_tab_menu = None;
                self.with_focused_project(move |ws, _io| ws.project_preview_edit_open(idx));
            }
            Message::ProjectPreviewReload(idx) => {
                self.project_preview_tab_menu = None;
                self.with_focused_project(move |ws, _io| ws.project_preview_reload(idx));
            }
            Message::ProjectPreviewTabContextMenu { idx, editable } => {
                let (x, y) = self.files.last_right_click();
                self.project_preview_tab_menu = Some(PreviewTabMenu {
                    x,
                    y,
                    idx,
                    editable,
                });
            }
            Message::ProjectPreviewTabContextMenuClose => {
                self.project_preview_tab_menu = None;
            }
```

还要处理 `Message::PreviewCloseTab(idx)`/`Message::ProjectPreviewCloseTab(idx)` 两个分支里各自的 `self.preview_tab_menu = None;`/`self.project_preview_tab_menu = None;` 那一行(菜单状态马上就要整体删掉,这两行也要跟着删,分支其余部分不变):

```rust
            Message::PreviewCloseTab(idx) => {
                self.with_focused_project(|ws, io| {
                    ws.preview.close(idx);
                    ws.preview_tab_first = 0;
                    ws.spawn_preview_state_save(io);
                    ws.spawn_preview_context_push(io);
                });
            }
```

```rust
            Message::ProjectPreviewCloseTab(idx) => {
                self.with_focused_project(|ws, _io| {
                    ws.project_preview.close(idx);
                    ws.project_preview_tab_first = 0;
                });
            }
```

Run: `cargo build -p dozer-app 2>&1 | tail -80`
Expected: 剩下 `PreviewTabMenu`/`preview_tab_menu`/`project_preview_tab_menu`/两个 `*_context_menu_popup` 函数/`view()` 里的 `stack!` 分支报未使用或找不到——继续。

- [ ] **Step 3: 删除 `PreviewTabMenu` struct 与状态字段**

删除 `app.rs:2082-2090`:

```rust
/// 预览 tab 右键菜单浮层状态:定位坐标(屏幕空间,复用 `files`
/// 右键落点)+ 目标 tab 下标 + 该文件是否可编辑(仅文本类文件可编辑,
/// 决定菜单里"编辑"项是否出现)。见 `preview_pane` / 顶层 `view`。
struct PreviewTabMenu {
    x: f32,
    y: f32,
    idx: usize,
    editable: bool,
}
```

删除字段声明(`app.rs:2267`、`2270` 附近,`App` struct 里):

```rust
    preview_tab_menu: Option<PreviewTabMenu>,
```
```rust
    project_preview_tab_menu: Option<PreviewTabMenu>,
```

删除对应初始化(`app.rs:2647`、`2648` 附近):

```rust
            preview_tab_menu: None,
            project_preview_tab_menu: None,
```

- [ ] **Step 4: 删除两个右键菜单渲染函数**

删除 `app.rs:7564-7620`(`preview_tab_context_menu_popup` 及其文档注释):

```rust
    /// 文件预览 tab 右键菜单浮层:含"刷新"(恒有)、"编辑"(仅可编辑文本文件)
    /// 与"关闭"三项。定位坐标由 `PreviewTabContextMenu` 打开时记录,风格与
    /// 文件树右键菜单一致(`files::context_menu_popup`/`menu_item`)。
    fn preview_tab_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        // ...(整个函数体,见上面 Step 探索时读到的完整实现)
    }
```

删除 `app.rs:7622-约7680`(`project_preview_tab_context_menu_popup` 及其文档注释,结构与上面完全对称)。

(执行时直接把这两个函数从"文档注释开头"到"函数结尾的 `}`"整段删掉即可,函数体内容在探索阶段已经完整读过,不需要额外确认。)

- [ ] **Step 5: 删除 `view()` 里对应的 `stack!` 分支**

删除 `app.rs:8257-8278`:

```rust
        } else if self.preview_tab_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::PreviewTabContextMenuClose);
            stack![base, dismiss, self.preview_tab_context_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.project_preview_tab_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectPreviewTabContextMenuClose);
            stack![base, dismiss, self.project_preview_tab_context_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.project_link_menu.is_some() {
```

换成(去掉这两个 `else if` 分支,让链条直接从上一个条件跳到 `project_link_menu`):

```rust
        } else if self.project_link_menu.is_some() {
```

Run: `cargo build -p dozer-app 2>&1 | tail -80`
Expected: 报 `workspace.rs::preview_pane_for` 里 `context_msg`/`editable`/`on_right_press` 相关的编译错——继续 Step 6。

- [ ] **Step 6: `workspace.rs::preview_pane_for` 去掉右键触发**

删除 `workspace.rs:3634-3637`:

```rust
    let context_msg = move |idx, editable| match kind {
        PreviewPaneKind::Files => Message::PreviewTabContextMenu { idx, editable },
        PreviewPaneKind::Project => Message::ProjectPreviewTabContextMenu { idx, editable },
    };
```

把 `workspace.rs:3673`(局部 `editable` 变量)先保留——Task 3 还要用它来判断切换按钮是否显示,只是这次不再喂给已删除的 `context_msg`。

把 `workspace.rs:3694-3707` 的:

```rust
            // 右键 tab 弹上下文菜单:"编辑"(仅可编辑)/"关闭"。
            // 拖拽换位:按住页签(选中处理已把 `app.tab_drag` 置位)后光标
            // 扫过哪个页签,这个 `on_move` 就按它发 `TabDragMove`,完成换位。
            let armed = app.dragging_group(tab_group);
            let mut area = MouseArea::new(tab)
                .on_right_press(context_msg(idx, editable))
                .on_move(move |_| Message::TabDragMove {
                    group: tab_group,
                    index: idx,
                });
            if armed {
                area = area.interaction(mouse::Interaction::Grabbing);
            }
            area.into()
```

换成:

```rust
            // 拖拽换位:按住页签(选中处理已把 `app.tab_drag` 置位)后光标
            // 扫过哪个页签,这个 `on_move` 就按它发 `TabDragMove`,完成换位。
            let armed = app.dragging_group(tab_group);
            let mut area = MouseArea::new(tab).on_move(move |_| Message::TabDragMove {
                group: tab_group,
                index: idx,
            });
            if armed {
                area = area.interaction(mouse::Interaction::Grabbing);
            }
            area.into()
```

- [ ] **Step 7: 删除"刷新"落地方法**

删除 `workspace.rs:799-818`:

```rust
    /// 预览 tab 右键菜单"刷新"落地(Files 面板):按 tab 下标重载该文件——
    /// 从文件系统重新读盘并重新渲染(原生 editor 重建 / webview 换 URL 重载)。
    pub(crate) fn preview_reload(&mut self, idx: usize) {
        self.preview_reload_for(PreviewPaneKind::Files, idx);
    }

    /// Project 面板右配对预览的"刷新",语义同 `preview_reload`,状态取自
    /// `ws.project_preview`。
    pub(crate) fn project_preview_reload(&mut self, idx: usize) {
        self.preview_reload_for(PreviewPaneKind::Project, idx);
    }

    fn preview_reload_for(&mut self, kind: PreviewPaneKind, idx: usize) {
        // 下标越界(菜单目标已被拖拽/关闭换位)由 `reload_at` 防御性 no-op——
        // 与 `preview_edit_open_for` 同口径,正常路径走不到。
        match kind {
            PreviewPaneKind::Files => self.preview.reload_at(idx),
            PreviewPaneKind::Project => self.project_preview.reload_at(idx),
        };
    }
```

(`PreviewPane::reload_at`/`bump_reload` 本身不删——它们是 `select()`/切 tab 时"webview 切回来重载"这条既有机制的底层实现,与右键"刷新"无关,别的调用点还在用。)

Run: `cargo build -p dozer-app 2>&1 | tail -80 && cargo clippy -p dozer-app --all-targets 2>&1 | tail -80`
Expected: PASS(可能剩 `preview_edit_open`/`preview_edit_*` 系列方法变成 dead_code 警告——这些属于 Task 2 范围,先不管)。顺手把 `tab_widget.rs:426`、`492` 两处注释里提到的 "`PreviewTabMenu`" 改成 "已移除的旧右键菜单"或类似措辞(纯文档更新,不影响编译,可选但建议做,避免注释引用一个不存在的东西)。

- [ ] **Step 8: 提交**

```bash
git add crates/dozer-app/src/app.rs crates/dozer-app/src/workspace.rs crates/dozer-app/src/tab_widget.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): 移除文件预览 tab 右键菜单(含刷新)

去掉 PreviewTabContextMenu/ProjectPreviewTabContextMenu 整条链路
(Message/状态/渲染/触发),"刷新"随菜单一起去掉、不补替代入口。关闭
仍走 tab 自带的 × 按钮,不受影响。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01GNaJ3sH2f2tP2cPRBQ4qri
EOF
)"
```

---

### Task 2: 移除全局编辑弹层

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`(`EditSession` struct 188-202 行;字段 `edit_session`(472 行附近)、`pending_edit_session_focus`(475 行附近)及其初始化(694/695 行附近);方法 `preview_edit_open`/`project_preview_edit_open`/`preview_edit_open_for`/`take_edit_session_focus_pending`/`preview_edit_event`/`preview_edit_undo`/`preview_edit_redo`/`preview_edit_save`/`preview_edit_close_request`/`preview_edit_confirm_discard`/`preview_edit_confirm_cancel`,749-920 行区间;`edit_modal`(4197 行起)/`edit_discard_confirm_popup`(4308 行起)两个渲染函数;对应单测,5767-约5900 行区间——`preview_edit_open_reads_file_and_starts_clean_session`/`preview_edit_open_missing_file_reports_error_and_does_not_open`/`preview_edit_open_out_of_range_index_is_noop`/`preview_edit_save_writes_disk_clears_dirty_and_bumps_reload` 等)
- Modify: `crates/dozer-app/src/app.rs`(Message 枚举 `EditorEvent`/`EditorUndo`/`EditorRedo`/`PreviewEditSave`/`PreviewEditCloseRequest`/`PreviewEditConfirmDiscard`/`PreviewEditConfirmCancel` 共 7 个变体,1856-1918 行区间;对应 `update` 分支,5218-5231、5235-5237、5287-5295 行区间;`edit_session_open` 方法,4290-4299 行;`view()` 里的 `stack!` 分支,8175-8193 行)
- Modify: `crates/dozer-app/src/main.rs`(Esc 关闭弹层的键盘拦截,1456-1471 行;弹层打开期间吞掉全部剩余按键的闸门,1473-1515 行;`EditorEvent` 分发,1984-1987 行;程序化聚焦里的 `edit_session_editor_focus_id` 计算与消费,2646-2655、2777 行)

**Interfaces:**
- Consumes: 无(纯删除)
- Produces: 无

- [ ] **Step 1: 删除 `EditSession` struct 与字段**

删除 `workspace.rs:188-202`:

```rust
/// 预览编辑弹层的进行中会话(全局至多一个;弹层是应用级模态)。
pub struct EditSession {
    /// `PreviewTab.id`(webview 池用的稳定 id,不是 `tabs` vec 下标——见
    /// `Workspace::preview_edit_open` 的取值处)。
    pub tab_id: usize,
    pub path: PathBuf,
    /// 编辑器组件(`code_editor::CodeView`)。有状态 widget,持有内容与撤销栈。
    pub editor: CodeView,
    /// 上次落盘(或打开)时的内容快照。脏标记 = `editor.text() != saved_content`。
    pub saved_content: String,
    /// 打开失败(理论上不会,打开前已判过存在)或保存失败的错误文案。
    pub error: Option<String>,
    /// 脏改动下点关闭:先弹二次确认,不直接丢。
    pub confirm_discard: bool,
}
```

删除字段声明 `pub(crate) edit_session: Option<EditSession>,`(472 行附近)与 `pub(crate) pending_edit_session_focus: bool,`(475 行附近),以及对应初始化 `edit_session: None,`/`pending_edit_session_focus: false,`(694/695 行附近)。

Run: `cargo build -p dozer-app 2>&1 | tail -100`
Expected: 一堆找不到 `EditSession`/`edit_session` 的编译错——继续。

- [ ] **Step 2: 删除全部 `preview_edit_*`/`take_edit_session_focus_pending` 方法**

删除 `workspace.rs:746-920` 整段(`preview_edit_open`/`project_preview_edit_open`/`preview_edit_open_for`/`take_edit_session_focus_pending`/`preview_edit_event`/`preview_edit_undo`/`preview_edit_redo`/`preview_edit_save`/`preview_edit_close_request`/`preview_edit_confirm_discard`/`preview_edit_confirm_cancel`,共 11 个方法——注意这段区间里穿插着 Task 1 已经删掉的 `preview_reload`/`project_preview_reload`/`preview_reload_for`,若 Task 1 已完成,这次只需删剩下的 11 个;`preview_tab_editor_event`/`project_preview_tab_editor_event`/`send_input` 不在此范围内,保留)。

具体删除范围以方法名为准,逐个方法从文档注释开头删到该方法结尾的 `}`。

Run: `cargo build -p dozer-app 2>&1 | tail -100`
Expected: `edit_modal`/`edit_discard_confirm_popup` 报找不到 `ws.edit_session`——继续。

- [ ] **Step 3: 删除 `edit_modal`/`edit_discard_confirm_popup` 渲染函数**

删除 `workspace.rs:4197-约4318`(`edit_modal` 函数,签名约为 `pub(crate) fn edit_modal(ws: &Workspace) -> Element<...>`,函数体渲染标题行+`CodeView`+错误位+保存/关闭按钮)整段,以及紧接着的 `edit_discard_confirm_popup`(4308 行起,签名 `pub(crate) fn edit_discard_confirm_popup<'a>() -> Element<'a, ...>`)整段。

- [ ] **Step 4: 删除 `workspace.rs` 里对应的单测**

删除 `workspace.rs:5767-约5900` 区间内的:`preview_edit_open_reads_file_and_starts_clean_session`、`preview_edit_open_missing_file_reports_error_and_does_not_open`、`preview_edit_open_out_of_range_index_is_noop`、`preview_edit_save_writes_disk_clears_dirty_and_bumps_reload`(每个测试从 `#[test]` 上一行的注释开头删到函数结尾 `}`;若同一区间内还有其它同样测 `edit_session`/`EditSession` 相关行为的测试函数,一并删除;不测 `edit_session` 的测试保留)。

Run: `cargo test -p dozer-app --lib 2>&1 | tail -60`
Expected: 编译通过,测试全绿(不应该再有任何 `edit_session`/`EditSession` 相关符号)。

- [ ] **Step 5: 删除 `app.rs` 里的 7 个 Message 变体**

删除 `app.rs:1853-1861`:

```rust
    /// 预览编辑弹层:官方 `text_editor` 的 `Action`。由 `main.rs` 的 dispatch
    /// 直接转发给 `App::preview_edit_event`(剪贴板由 iced 运行时自己处理,
    /// 不需要像 vendored `iced-code-editor` 那样手动拆 `Task` 桥接)。
    EditorEvent(iced_widget::text_editor::Action),
    /// 预览编辑弹层:撤销(⌘Z / Ctrl+Z)。同样由 `main.rs` 命中组合键后直接
    /// 转发给 `App` 下的 `workspace`(见 `preview_edit_undo`)。
    EditorUndo,
    /// 预览编辑弹层:重做(⌘⇧Z / Ctrl+⇧Z)。
    EditorRedo,
```

删除 `app.rs:1866-1867`:

```rust
    /// 预览编辑弹层:"保存"按钮 / ⌘S。
    PreviewEditSave,
```

删除 `app.rs:1913-1918`:

```rust
    /// 预览编辑弹层:×按钮 / 点遮罩——脏改动会先转成二次确认,不直接关。
    PreviewEditCloseRequest,
    /// 预览编辑弹层二次确认:"放弃改动"。
    PreviewEditConfirmDiscard,
    /// 预览编辑弹层二次确认:"取消"(回到编辑态)。
    PreviewEditConfirmCancel,
```

Run: `cargo build -p dozer-app 2>&1 | tail -100`
Expected: `update` 分支报找不到变体——继续。

- [ ] **Step 6: 删除 `app.rs::update` 里对应的处理分支**

删除 `app.rs:5218-5231`:

```rust
            Message::EditorEvent(_action) => {
                // main.rs 的 dispatch 直接调 `App::preview_edit_event`,不经过
                // 这里的 `App::update`——到达此处说明未走 dispatch 拦截,忽略。
            }
            Message::EditorUndo => {
                // 键盘(⌘Z)经 `app.update` 进来时走这里真正撤销;main.rs 另有
                // `App::preview_edit_undo` 直呼口(绕过 `update`),两路都只操作
                // 聚焦项目编辑弹层的 editor。
                self.with_focused_project(|ws, _io| ws.preview_edit_undo());
            }
            Message::EditorRedo => {
                // 同 `EditorUndo`(⌘⇧Z)。
                self.with_focused_project(|ws, _io| ws.preview_edit_redo());
            }
```

删除 `app.rs:5235-5237`:

```rust
            Message::PreviewEditSave => {
                self.with_focused_project(|ws, _io| ws.preview_edit_save());
            }
```

删除 `app.rs:5287-5295`:

```rust
            Message::PreviewEditCloseRequest => {
                self.with_focused_project(|ws, _io| ws.preview_edit_close_request());
            }
            Message::PreviewEditConfirmDiscard => {
                self.with_focused_project(|ws, _io| ws.preview_edit_confirm_discard());
            }
            Message::PreviewEditConfirmCancel => {
                self.with_focused_project(|ws, _io| ws.preview_edit_confirm_cancel());
            }
```

- [ ] **Step 7: 删除 `edit_session_open` 方法**

删除 `app.rs:4290-4299`:

```rust
    /// 预览编辑弹层是否打开(main.rs 键盘路由用)。打开期间键盘必须走
    /// 弹层的文本编辑器,不能落进终端 PTY——弹层挂在左侧预览面板,不影响
    /// `terminal_visible()` 的判断条件(右侧展开与否),不加这道闸门的话,
    /// 默认布局(右侧终端可见)下编辑弹层里敲的每个字符,包括回车,都会
    /// 同时写进背后那个终端/agent 会话。
    pub fn edit_session_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.edit_session.is_some())
            .unwrap_or(false)
    }
```

- [ ] **Step 8: 删除 `view()` 里的 `stack!` 分支**

删除 `app.rs:8175-8193`:

```rust
        } else if ws.edit_session.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::PreviewEditCloseRequest);
            let confirm_discard = ws.edit_session.as_ref().is_some_and(|s| s.confirm_discard);
            if confirm_discard {
                stack![base, dismiss, edit_modal(ws), edit_discard_confirm_popup()]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            } else {
                stack![base, dismiss, edit_modal(ws)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
        } else if ws.files.tree_delete_confirm_is_some() {
```

换成:

```rust
        } else if ws.files.tree_delete_confirm_is_some() {
```

Run: `cargo build -p dozer-app 2>&1 | tail -100`
Expected: 报 `main.rs` 里 `edit_session_open`/`EditorRedo`/`EditorUndo`/`EditorEvent`/`take_edit_session_focus_pending`/`ws.edit_session` 找不到——继续。

- [ ] **Step 9: `main.rs` 删除键盘拦截与 dispatch**

删除 `main.rs:1456-1471`:

```rust
            // 预览编辑弹层打开时,Esc 优先触发关闭流程(脏则弹确认,不脏直接
            // 关),口径同上面几个弹层。
            if app.edit_session_open()
                && let WindowEvent::KeyboardInput {
                    event,
                    is_synthetic: false,
                    ..
                } = event
                && event.state == ElementState::Pressed
                && event.logical_key
                    == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
            {
                app.update(Message::PreviewEditCloseRequest);
                window.request_redraw();
                return;
            }
```

删除 `main.rs:1473-1515`(整个"编辑弹层打开时剩余按键一律不再往下走"的闸门,从注释开头到该 `if` 块结尾 `}`):

```rust
            // 预览编辑弹层打开时,剩余按键一律不再往下走 ⌘ 快捷键/地址栏/
            // 终端转发——弹层里的 `iced-code-editor` 走标准 iced 事件管线
            // (键盘事件经 `Canvas` widget 的 `on_event` 自己消化),这里不需要
            // 也不应该手工转发。不加这道闸门的话,`terminal_visible()` 只看右侧
            // 是否展开、对弹层状态一无所知,默认布局(右侧终端可见)下弹层
            // 里打的每个字符、包括回车,都会同时写进背后那个终端/agent 会话
            // (Critical,code review 发现)。
            if app.edit_session_open() {
                // ...(整个 if 块,含内部 ⌘Z/⌘⇧Z 判断与 `return`)
            }
```

删除 `main.rs:1984-1987`:

```rust
                Message::EditorEvent(action) => {
                    app.preview_edit_event(action);
                    window.request_redraw();
                }
```

(紧接着的 `Message::PreviewEditorEvent`/`Message::ProjectPreviewEditorEvent` 两个分支保留,不要删。)

删除 `main.rs:2646-2655` 的 `edit_session_editor_focus_id` 计算:

```rust
                                let edit_session_editor_focus_id =
                                    app.active_workspace_mut().and_then(|ws| {
                                        ws.take_edit_session_focus_pending()
                                            .then(|| {
                                                ws.edit_session
                                                    .as_ref()
                                                    .map(|s| s.editor.focus_id())
                                            })
                                            .flatten()
                                    });
```

把 `main.rs:2774-2778` 的:

```rust
                                for id in [
                                    preview_editor_focus_id,
                                    project_preview_editor_focus_id,
                                    edit_session_editor_focus_id,
                                ]
```

换成:

```rust
                                for id in [preview_editor_focus_id, project_preview_editor_focus_id]
```

Run: `cargo build -p dozer-app 2>&1 | tail -100 && cargo clippy -p dozer-app --all-targets 2>&1 | tail -100`
Expected: PASS,无 `edit_session`/`EditSession`/`EditorEvent`/`EditorUndo`/`EditorRedo`/`preview_edit_*` 残留(可跑 `grep -rn "edit_session\|EditSession\|EditorUndo\|EditorRedo" crates/dozer-app/src/` 确认为空,`EditorEvent` 若还剩只应是 `PreviewEditorEvent`/`ProjectPreviewEditorEvent`,那两个是保留项)。

- [ ] **Step 10: 手动验证**

Run: `cargo run -p dozer-app`

1. 打开任意文件预览 tab,确认没有任何方式能再打开旧的全屏编辑弹层(右键已经没了,也没有其它按钮/快捷键指向它)。
2. 正常敲字进终端,确认没有因为这次删除而出现按键漏发/多发的情况(重点验证:终端聚焦时键盘输入完全正常,预览面板不再吞任何按键)。
3. 打开一个原生可编辑 tab(如 `.rs`),⌘S 保存仍正常工作(`PreviewSaveActive` 路径不受影响)。

- [ ] **Step 11: 提交**

```bash
git add crates/dozer-app/src/app.rs crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): 移除文件预览全局编辑弹层

去掉 EditSession/edit_modal/edit_discard_confirm_popup 整条链路
(Message/状态/渲染/键盘路由拦截/程序化聚焦)。原生 tab 就地编辑
(PreviewEditorEvent/PreviewSaveActive)不受影响,后续由预览/代码切换
按钮接替被去掉的编辑入口(见下一个 commit)。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01GNaJ3sH2f2tP2cPRBQ4qri
EOF
)"
```

---

### Task 3: 新增预览/代码切换按钮 + 关闭前静默保存

**Files:**
- Modify: `crates/dozer-app/src/preview.rs`(新增 `wry_toggle_eligible` 判定函数,紧跟 `prefers_rendered_preview` 之后;`impl PreviewPane` 新增 `enter_code_mode`/`exit_code_mode` 两个方法,放在 `reload_at`/`bump_reload` 附近;更新 `prefers_rendered_preview` 文档注释里过时的"右键编辑"表述)
- Modify: `crates/dozer-app/src/tab_widget.rs`(新增 `tab_render_mode_button` 渲染函数,紧跟 `tab_arrow_button`/`tab_overflow_button` 之后)
- Modify: `crates/dozer-app/src/workspace.rs`(新增 `preview_pane_toggle_render_mode`/`preview_pane_save_at` 方法;`preview_pane_save_active` 改为委托 `preview_pane_save_at`;`preview_pane_for` 里 `suffix: None` 换成条件渲染切换按钮)
- Modify: `crates/dozer-app/src/app.rs`(新增 `Message::PreviewToggleRenderMode(usize)`/`Message::ProjectPreviewToggleRenderMode(usize)` 及处理分支;`PreviewCloseTab`/`ProjectPreviewCloseTab` 分支追加关闭前静默保存)

**Interfaces:**
- Consumes: Task 1、2 清理后的 `preview_pane_for`/`App::update`(不强依赖,但建议在干净的基础上做)
- Produces: 无(叶子任务)

- [ ] **Step 1: `preview.rs` 新增 `wry_toggle_eligible` 判定函数**

在 `preview.rs:175` 附近(`prefers_rendered_preview` 函数结尾之后)追加:

```rust
/// tab 上"预览/代码"切换按钮该不该出现的判定:只对"文本可编辑、但默认走
/// wry/flyfish 渲染"的文件出现(目前即 `.md`/`.markdown`/`.html`/`.htm`)。
/// 图片/PDF/压缩包等真二进制文件(`is_editable_extension` 为假)不出现;
/// 本来就默认原生可编辑的其它文本文件(`!prefers_rendered_preview`,如
/// `.rs`/`.py`/`.json`)也不出现——它们从来不是 wry 打开的,永远原生。
pub fn wry_toggle_eligible(path: &std::path::Path) -> bool {
    is_editable_extension(path) && prefers_rendered_preview(path)
}
```

把 `prefers_rendered_preview` 文档注释(`preview.rs:159-165`)里这一句过时表述:

```
/// (`preview_edit_open_for` 独立读盘建 editor,不依赖这个 tab 当前是不是
/// 走 webview),只是**默认预览**换成渲染效果。
```

换成:

```
/// tab 右侧的预览/代码切换按钮(`wry_toggle_eligible`)独立读盘建 editor,
/// 不依赖这个 tab 当前是不是走 webview),只是**默认预览**换成渲染效果。
```

- [ ] **Step 2: `impl PreviewPane` 新增 `enter_code_mode`/`exit_code_mode`**

在 `preview.rs:919`(`reload_at` 之前或 `bump_reload` 之后均可,建议紧跟 `bump_reload` 之后)追加:

```rust
    /// 预览→代码:按下标读盘建一个可写 `CodeView` 挂到该 tab 上(只应对
    /// `wry_toggle_eligible` 的文件 tab 调用,按钮只在这类 tab 上画)。下标
    /// 越界或该 tab 不是 `TabKind::File` 是 no-op;读盘失败把 `io::Error`
    /// 透传给调用方(`Workspace::preview_pane_toggle_render_mode`)写面板
    /// error,这里不生成错误文案——同 `preview_edit_open_for` 曾经的分工。
    pub fn enter_code_mode(&mut self, idx: usize) -> std::io::Result<()> {
        let Some(tab) = self.tabs.get(idx) else {
            return Ok(());
        };
        let TabKind::File(path) = &tab.kind else {
            return Ok(());
        };
        let editor = read_and_build_native_editor(path)?;
        if let Some(tab) = self.tabs.get_mut(idx) {
            tab.editor = Some(editor);
            tab.dirty = false;
        }
        Ok(())
    }

    /// 代码→预览:清空该 tab 的原生 editor,转回 wry/flyfish 渲染。调用方
    /// 负责在此之前先把脏改动落盘(`Workspace::preview_pane_toggle_render_mode`
    /// 里先 `preview_pane_save_at` 再调这个)——这里只做状态切换,不碰磁盘。
    /// 下标越界是 no-op。
    pub fn exit_code_mode(&mut self, idx: usize) {
        if let Some(tab) = self.tabs.get_mut(idx) {
            tab.editor = None;
        }
    }
```

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: PASS(暂无调用方,`dead_code` 警告是预期的)

- [ ] **Step 3: `tab_widget.rs` 新增 `tab_render_mode_button`**

紧跟 `tab_overflow_button` 之后追加:

```rust
/// 预览/代码模式切换按钮:只在 `preview::wry_toggle_eligible` 为真的文件
/// tab 上画(调用方判断,这里只管渲染)。`in_code_mode` 决定图标——预览态
/// 显示 `FileCode`(点它切到代码),代码态显示 `Eye`(点它切回预览)。hover
/// 用 iced 内置 `button::Status`,不接入 `HoverId` 动画体系——同 `tab_arrow_button`/
/// `tab_overflow_button` 的既有做法:这个按钮会随 tab 增减/拖拽换位下标
/// 漂移,`rekey_hover_range` 目前只接受两个 `HoverId` 构造器(item/close),
/// 犯不着为它扩展签名。
pub(crate) fn tab_render_mode_button<'a, M: Clone + 'a>(
    in_code_mode: bool,
    on_press: M,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let icon = if in_code_mode {
        icons::IconKind::Eye
    } else {
        icons::IconKind::FileCode
    };
    let color = byteui::theme::color::current().dim;
    let btn = button(icons::view(icon, byteui::theme::icon_size::row(), color))
        .width(Length::Fixed(byteui::theme::geometry::tab_button_size()))
        .height(Length::Fixed(byteui::theme::geometry::tab_button_size()))
        .padding(0)
        .on_press(on_press)
        .style(move |_theme, status| {
            let base = button::Style {
                background: None,
                text_color: color,
                ..button::Style::default()
            };
            match status {
                button::Status::Hovered | button::Status::Pressed => button::Style {
                    text_color: byteui::theme::color::current().gold,
                    ..base
                },
                _ => base,
            }
        });
    btn.into()
}
```

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: PASS

- [ ] **Step 4: `workspace.rs` 新增 `preview_pane_save_at`,`preview_pane_save_active` 改为委托**

把 `workspace.rs:2061-2117` 的 `preview_pane_save_active` 函数体改造:方法签名不变,内部改成:

```rust
    pub fn preview_pane_save_active(&mut self, kind: PanelKind) {
        let idx = if kind == PanelKind::Project {
            self.project_preview.active_idx()
        } else {
            self.preview.active_idx()
        };
        self.preview_pane_save_at(kind, idx);
    }

    /// 把 `kind` 面板**指定下标**tab 的就地改动保存到磁盘,语义同
    /// `preview_pane_save_active`(其实现已改为委托这个方法),差别只是不再
    /// 局限于"当前激活"——`preview_pane_toggle_render_mode`(代码→预览)、
    /// `PreviewCloseTab`/`ProjectPreviewCloseTab`(关闭前静默保存)都可能要
    /// 保存一个非激活的背景 tab。仅当该 tab 是原生可编辑且脏时动作,其余
    /// 沿用原实现(不脏不动磁盘、失败写面板 error)。
    fn preview_pane_save_at(&mut self, kind: PanelKind, idx: usize) {
        let project = kind == PanelKind::Project;
        let (tab_id, path) = {
            let pane = if project {
                &self.project_preview
            } else {
                &self.preview
            };
            let Some(tab) = pane.tabs().get(idx) else {
                return;
            };
            if !tab.dirty || tab.editor.is_none() {
                return;
            }
            let TabKind::File(p) = &tab.kind else {
                return;
            };
            (tab.id, p.clone())
        };
        let text = {
            let pane = if project {
                &mut self.project_preview
            } else {
                &mut self.preview
            };
            match pane.editor_mut(tab_id) {
                Some(e) => e.text(),
                None => return,
            }
        };
        match std::fs::write(&path, text) {
            Ok(()) => {
                let pane = if project {
                    &mut self.project_preview
                } else {
                    &mut self.preview
                };
                pane.clear_dirty_by_id(tab_id);
                pane.find_refresh_after_edit(tab_id);
            }
            Err(e) => {
                let err = Some(format!("保存失败: {e}"));
                if project {
                    self.project_preview_error = err;
                } else {
                    self.preview_error = err;
                }
            }
        }
    }
```

(这段本质是把原 `preview_pane_save_active` 里"定位激活 tab"那部分换成"定位任意下标 tab",其余读文本/写盘/清脏/刷新 Find 的逻辑原样保留,不是重新发明。)

Run: `cargo test -p dozer-app --lib preview_pane_save 2>&1 | tail -40`
Expected: PASS(既有 `preview_pane_save_active` 相关测试若有,应保持通过——纯内部重构,外部行为不变)

- [ ] **Step 5: `workspace.rs` 新增 `preview_pane_toggle_render_mode`**

紧跟 `preview_pane_save_at` 之后追加:

```rust
    /// 切换 `kind` 面板某个 tab 的预览/代码渲染模式(仅对 `wry_toggle_eligible`
    /// 的文件 tab 有意义——按钮只在这类 tab 上画,其它 tab 点不到)。
    /// 代码→预览:先 `preview_pane_save_at` 静默落盘(不脏则内部直接
    /// no-op),再 `exit_code_mode` 转回渲染。预览→代码:直接
    /// `enter_code_mode` 读盘建原生编辑器,失败写对应面板 error。
    pub(crate) fn preview_pane_toggle_render_mode(&mut self, kind: PanelKind, idx: usize) {
        let project = kind == PanelKind::Project;
        let in_code_mode = {
            let pane = if project {
                &self.project_preview
            } else {
                &self.preview
            };
            pane.tabs().get(idx).is_some_and(|t| t.editor.is_some())
        };
        if in_code_mode {
            self.preview_pane_save_at(kind, idx);
            let pane = if project {
                &mut self.project_preview
            } else {
                &mut self.preview
            };
            pane.exit_code_mode(idx);
        } else {
            let pane = if project {
                &mut self.project_preview
            } else {
                &mut self.preview
            };
            if let Err(e) = pane.enter_code_mode(idx) {
                let err = Some(format!("打开代码模式失败: {e}"));
                if project {
                    self.project_preview_error = err;
                } else {
                    self.preview_error = err;
                }
            }
        }
    }
```

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: PASS(暂无调用方)

- [ ] **Step 6: `preview_pane_for` 接入切换按钮**

在 `workspace.rs:3626-3633` 的 `overflow_toggle_msg`/`overflow_dismiss_msg` 闭包之后追加:

```rust
    let render_toggle_msg = move |idx| match kind {
        PreviewPaneKind::Files => Message::PreviewToggleRenderMode(idx),
        PreviewPaneKind::Project => Message::ProjectPreviewToggleRenderMode(idx),
    };
```

把 `workspace.rs:3687` 的 `suffix: None,` 换成:

```rust
                suffix: if crate::preview::wry_toggle_eligible(match &tab.kind {
                    TabKind::File(p) => p.as_path(),
                    TabKind::Blank => std::path::Path::new(""),
                }) {
                    Some(tab_widget::tab_render_mode_button(
                        tab.editor.is_some(),
                        render_toggle_msg(idx),
                    ))
                } else {
                    None
                },
```

(`TabKind::Blank` 分支传一个空路径进 `wry_toggle_eligible` 保证恒为 `false`——`Blank` 没有真实文件,`is_editable_extension`/`prefers_rendered_preview` 对空路径都会走扩展名为空的分支,自然判 `false`,不需要额外特判,但为了不 panic/不误判,这里显式给个占位路径而不是尝试从 `Blank` 变体解出路径。)

`workspace.rs:3673` 的 `editable` 局部变量此时应该已经没有其它调用方(Task 1 删掉了唯一消费它的 `context_msg`)——确认后连同它的赋值语句一起删掉:

```rust
            let editable = matches!(&tab.kind, TabKind::File(path) if is_editable_extension(path));
```

Run: `cargo build -p dozer-app 2>&1 | tail -80`
Expected: 报 `Message::PreviewToggleRenderMode`/`ProjectPreviewToggleRenderMode` 找不到变体——继续 Step 7。

- [ ] **Step 7: `app.rs` 新增 Message 变体与处理分支**

在 `app.rs:1844`(`PreviewCloseTab(usize)` 之后)追加:

```rust
    /// 预览:切换某个 tab 的预览/代码渲染模式(仅对
    /// `preview::wry_toggle_eligible` 为真的 tab 有意义,`usize` 是 tab
    /// vec 下标,约定同 `PreviewSelectTab`)。
    PreviewToggleRenderMode(usize),
```

在 `app.rs:1933`(`ProjectPreviewCloseTab(usize)` 之后)追加:

```rust
    /// Project 面板右配对预览:切换某个 tab 的预览/代码渲染模式,语义同
    /// `PreviewToggleRenderMode`。
    ProjectPreviewToggleRenderMode(usize),
```

在 `app.rs::update` 里,`Message::PreviewCloseTab(idx)` 分支之后追加:

```rust
            Message::PreviewToggleRenderMode(idx) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_pane_toggle_render_mode(PanelKind::Files, idx);
                });
            }
```

在 `Message::ProjectPreviewCloseTab(idx)` 分支之后追加:

```rust
            Message::ProjectPreviewToggleRenderMode(idx) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_pane_toggle_render_mode(PanelKind::Project, idx);
                });
            }
```

- [ ] **Step 8: 关闭 tab 前静默保存**

把 `Message::PreviewCloseTab(idx)` 分支:

```rust
            Message::PreviewCloseTab(idx) => {
                self.with_focused_project(|ws, io| {
                    ws.preview.close(idx);
                    ws.preview_tab_first = 0;
                    ws.spawn_preview_state_save(io);
                    ws.spawn_preview_context_push(io);
                });
            }
```

换成:

```rust
            Message::PreviewCloseTab(idx) => {
                self.with_focused_project(|ws, io| {
                    ws.preview_pane_save_at(PanelKind::Files, idx);
                    ws.preview.close(idx);
                    ws.preview_tab_first = 0;
                    ws.spawn_preview_state_save(io);
                    ws.spawn_preview_context_push(io);
                });
            }
```

把 `Message::ProjectPreviewCloseTab(idx)` 分支:

```rust
            Message::ProjectPreviewCloseTab(idx) => {
                self.with_focused_project(|ws, _io| {
                    ws.project_preview.close(idx);
                    ws.project_preview_tab_first = 0;
                });
            }
```

换成:

```rust
            Message::ProjectPreviewCloseTab(idx) => {
                self.with_focused_project(|ws, _io| {
                    ws.preview_pane_save_at(PanelKind::Project, idx);
                    ws.project_preview.close(idx);
                    ws.project_preview_tab_first = 0;
                });
            }
```

(`preview_pane_save_at` 目前是 `fn`(私有)——上面 Step 4 定义时改成 `pub(crate) fn`,让 `app.rs` 能调用到。)

Run: `cargo build -p dozer-app 2>&1 | tail -100 && cargo clippy -p dozer-app --all-targets 2>&1 | tail -100 && cargo fmt`
Expected: PASS,无警告残留

- [ ] **Step 9: 纯逻辑单测**

在 `preview.rs` 测试模块(`#[cfg(test)] mod tests` 内,靠近 `is_editable_extension_covers_common_text_types`/`is_editable_extension_rejects_unknown_and_binary_like` 的位置)追加:

```rust
    #[test]
    fn wry_toggle_eligible_true_for_rendered_preview_text_types() {
        use std::path::Path;
        assert!(wry_toggle_eligible(Path::new("README.md")));
        assert!(wry_toggle_eligible(Path::new("page.html")));
    }

    #[test]
    fn wry_toggle_eligible_false_for_always_native_text_types() {
        use std::path::Path;
        assert!(!wry_toggle_eligible(Path::new("main.rs")));
        assert!(!wry_toggle_eligible(Path::new("script.py")));
        assert!(!wry_toggle_eligible(Path::new("data.json")));
    }

    #[test]
    fn wry_toggle_eligible_false_for_binary_types() {
        use std::path::Path;
        assert!(!wry_toggle_eligible(Path::new("photo.png")));
        assert!(!wry_toggle_eligible(Path::new("doc.pdf")));
        assert!(!wry_toggle_eligible(Path::new("archive.zip")));
    }
```

Run: `cargo test -p dozer-app --lib wry_toggle_eligible 2>&1 | tail -30`
Expected: PASS

- [ ] **Step 10: 手动验证**

Run: `cargo run -p dozer-app`

1. 打开一个 `.md` 文件,确认默认走预览渲染(wry/flyfish)且 tab 上出现切换按钮(`FileCode` 图标)。
2. 点按钮切到代码模式,确认语法高亮/可编辑/dirty 星号与其它原生 tab 一致,按钮图标变成 `Eye`。
3. 改点内容不保存,点按钮切回预览,确认改动被静默保存(重新打开文件或看外部编辑器确认落盘)且预览显示的是最新内容。
4. 再点回代码模式,确认看到的是刚才保存过的内容。
5. 改点内容不保存,直接点 tab 的 `×` 关闭,重新打开同一文件,确认改动已经落盘。
6. 打开一个 `.rs`/`.py` 等本来就原生可编辑的文件,确认 tab 上**没有**切换按钮。
7. 打开一张图片/一个 PDF,确认 tab 上没有切换按钮。
8. Project 面板右配对预览重复 1-7。

- [ ] **Step 11: 提交**

```bash
git add crates/dozer-app/src/preview.rs crates/dozer-app/src/tab_widget.rs crates/dozer-app/src/workspace.rs crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
feat(dozer-app): 新增预览/代码切换按钮,接替被移除的编辑弹层

wry_toggle_eligible 判定 + PreviewPane::enter_code_mode/exit_code_mode
+ tab_render_mode_button;切回预览与关闭 tab 前都先静默保存
(preview_pane_save_at,从 preview_pane_save_active 重构出的按下标版本)。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01GNaJ3sH2f2tP2cPRBQ4qri
EOF
)"
```

---

### Task 4: 人工 GUI 验收清单

**Files:** 无代码改动,纯验证。

**Interfaces:** 无

- [ ] **Step 1: 全量回归**

Run: `cargo build --workspace && cargo clippy --workspace --all-targets && cargo fmt --check && cargo test --workspace`
Expected: 全绿

- [ ] **Step 2: 逐项过一遍验收清单**

`cargo run -p dozer-app`:

- [ ] 右键任意文件预览 tab / Project 预览 tab,确认没有菜单弹出。
- [ ] 确认没有任何入口能打开旧的全局编辑弹层(菜单已删、没有替代按钮/快捷键)。
- [ ] `.md`/`.html` 类文件:默认预览渲染,tab 上有切换按钮,预览↔代码来回切换正常,未保存改动在切换/关闭时都被静默保存不丢失。
- [ ] `.rs`/`.py`/`.json` 等原生可编辑文件:没有切换按钮,⌘S 保存正常,行为与改造前一致。
- [ ] 图片/PDF/压缩包:没有切换按钮,预览渲染正常。
- [ ] 终端聚焦时正常打字,确认没有按键被预览面板意外拦截(Task 2 删除键盘闸门后的回归点)。
- [ ] 深色主题(ByteBoy2077 配色)下切换按钮的静止/hover 颜色符合规范,不出现看不清的情况。

- [ ] **Step 3: 走 `superpowers:finishing-a-development-branch` 决定合并方式**

以上全部勾完后,按该 skill 的流程决定如何把这个 worktree 分支合并回 main。
