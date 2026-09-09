# 文件预览:去模态编辑 + 预览/代码切换按钮设计

**状态:已批准(brainstorming 会话,2026-09-09)**

## 背景

文件预览面板(File 预览 + Project 面板右配对预览,共用 `workspace.rs::preview_pane_for`)现状有两套完全独立的编辑机制:

1. **全局编辑弹层**(`Workspace.edit_session: Option<EditSession>`,应用级模态,至多一份):唯一入口是 tab 右键菜单的"编辑"。服务对象是那些 `is_editable_extension` 为真、但因为 `prefers_rendered_preview` 为真而被 `push_tab` 判去走 wry/flyfish 渲染(`editor: None`)的文件——目前只有 `.md`/`.markdown` 这一类。这类文件没有原生 tab 可写,只能靠这个弹层编辑。
2. **原生 tab 就地编辑**(`PreviewTab.editor: Option<CodeView>`,`push_tab` 时一次性决定):其余可编辑文本文件(`.rs`/`.py`/`.json`/纯文本等)默认就是这个模式,靠 `PreviewEditorEvent`/`PreviewSaveActive` 驱动,语法高亮+⌘S 保存。

右键菜单本身还有"刷新"(`PreviewReload`,恒显示,当前没有其它入口)和"关闭"(`PreviewCloseTab`,但 tab 自己已经有独立的 `×` 关闭按钮,不依赖菜单)。

这次要去掉右键菜单和全局编辑弹层这两套模态/浮层类 UI,给"默认走 wry/flyfish 渲染的文本可编辑文件"新增一个 tab 上的预览/代码切换按钮,直接复用既有的原生 tab 就地编辑机制作为"代码模式",替代被去掉的编辑弹层。

一个关键实现线索:`tab_widget.rs::panel_tab` 早就有一个 `suffix` 参数插槽,文档注释写明"承载预览编辑图标(标题右侧、关闭按钮前)"——这个位置正是为了曾经存在过的 tab 级编辑图标预留的,后来编辑入口收进右键菜单后一直传 `None` 闲置。这次直接复用这个插槽放新按钮,不需要新增布局。

## 目标 / 非目标

**目标**:

1. 整条移除右键菜单链路:`PreviewTabContextMenu`/`ProjectPreviewTabContextMenu`/`PreviewTabContextMenuClose`/`ProjectPreviewTabContextMenuClose` 四个 Message 及其处理分支、`preview_tab_context_menu_popup`/`project_preview_tab_context_menu_popup` 两个渲染函数、`App.preview_tab_menu`/`project_preview_tab_menu` 状态字段(`PreviewTabMenu` struct)、`preview_pane_for` 里给每个 tab 包一层 `on_right_press` 的 `MouseArea`。"刷新"(`PreviewReload`/`ProjectPreviewReload`)随菜单一起去掉,不补替代入口。
2. 整条移除全局编辑弹层链路:`Workspace.edit_session`/`EditSession` struct、`edit_modal`/`edit_discard_confirm_popup` 两个渲染函数、app.rs 里对应的 `stack!` 叠加、以及 `EditorEvent`/`EditorUndo`/`EditorRedo`/`PreviewEditSave`/`PreviewEditCloseRequest`/`PreviewEditConfirmDiscard`/`PreviewEditConfirmCancel`/`PreviewEditOpen`/`ProjectPreviewEditOpen` 全部相关 Message 及其处理方法(`preview_edit_open`/`preview_edit_open_for`/`preview_edit_save`/`preview_edit_confirm_discard`/`preview_edit_confirm_cancel` 及 project 对应版本)。
3. 新增预览/代码切换按钮,复用 `panel_tab` 的 `suffix` 插槽。
4. 按钮只在满足 `is_editable_extension(path) && prefers_rendered_preview(path)` 的 tab 上出现——即目前默认走 wry/flyfish 渲染的文本类文件(如 `.md`)。图片/PDF/压缩包等真二进制文件不出现(它们根本没有"代码"可看);本来就默认原生可编辑的其它文本文件(`.rs`/`.py`/`.json` 等)也不出现,它们从来不是 wry 打开的,维持现状永远原生、无切换。
5. 点按钮切到代码模式:现场给该 tab 建一个原生 `CodeView`(复用 `push_tab` 建原生 tab 时的同一份初始化逻辑,抽成共享函数避免分叉),完全等同其它原生 tab 的编辑体验——语法高亮、可打字、dirty 星号、⌘S 保存。点按钮切回预览模式:若该 tab 当前 `dirty`,先静默走一遍与 ⌘S(`PreviewSaveActive`)相同的落盘逻辑,再清空 `editor` 字段,转回 wry/flyfish 渲染;不脏则直接切,不做多余的空保存。
6. 关闭一个 `dirty=true` 的原生可编辑 tab 时(`PreviewCloseTab`/`ProjectPreviewCloseTab`),移除前先走一遍同款静默保存,不丢弃未保存改动——这个行为对所有原生可编辑 tab 生效,不局限于这次新增的切换类型(本来就默认原生可编辑的 `.rs`/`.py` 等文件关闭时同样受益)。

**非目标**:

- 不改变"本来就默认原生可编辑"的其它文本文件在切换按钮出现与否上的行为——它们没有切换按钮,永远原生,不会被这次改造接入 wry/flyfish。
- 不给图片/PDF/压缩包等真二进制文件造切换按钮或伪代码视图。
- 不新增任何确认弹窗/模态类 UI 作为被移除项的替代品(去模态是这次的明确目标,包括"放弃改动二次确认"这类交互也不保留,统一用静默保存解决)。
- File 预览与 Project 面板右配对预览两个面板都要接入、行为一致(`preview_pane_for` 同一份函数服务两者)。

## 关键语义确认(brainstorming 会话定案)

1. **"刷新"去留**:直接去掉,不补替代入口(文件外部改动后,切 tab/重新打开文件能达到类似效果)。
2. **"代码模式"语义**:可编辑,直接复用原生 tab 就地编辑机制(语法高亮+可改+⌘S),不是只读源码查看——正是用它接替被去掉的编辑弹层。
3. **切换按钮适用范围**:只对"文本可编辑且默认走 wry/flyfish 渲染"的文件出现(`is_editable_extension && prefers_rendered_preview`),不对图片/PDF 等真二进制文件出现。
4. **代码模式有未保存改动时切回预览**:自动静默保存,不弹确认框、不阻断切换。
5. **(本次追加)关闭有未保存改动的原生可编辑 tab**:同样自动静默保存后再关闭,不丢弃、不弹确认框——与第 4 点同一思路的自然延伸,统一了"任何可能丢改动的操作都先静默保存"这条原则。

## 架构

数据模型几乎不用改——现状 `PreviewTab.editor: Option<CodeView>` 已经是 `desired_webviews()` 判定"这个 tab 要不要长 wry webview"的唯一开关(`desired_webviews()` 直接 `filter(tab.editor.is_none())`)。这次的核心改动只是让这个字段在 `push_tab` 之后也能被运行时切换,而不是像现在这样一次性决定死。

**新增**:

- 判定函数(`preview.rs`):

  ```rust
  fn wry_toggle_eligible(path: &Path) -> bool {
      is_editable_extension(path) && prefers_rendered_preview(path)
  }
  ```

  直接组合 `push_tab` 已经在用的两个既有判据,不新建规则、不新增文件类型注册表。

- `Message::PreviewToggleRenderMode(usize)` / `Message::ProjectPreviewToggleRenderMode(usize)`,`usize` 语义同 `PreviewSelectTab`(tab 在 `preview.tabs()` 里的 vec 位置)。

- 一个从 `push_tab` 抽出来的共享函数,专职"给一个路径建原生 `CodeView`"(读盘+建 `CodeView` 实例),`push_tab` 初始决定原生 tab 时调它,切换到代码模式时也调它,避免两处各写一份读盘/建 view 的逻辑,后续分叉。

- 切回预览模式 / 关闭 dirty tab 这两处共享同一段"静默保存"逻辑,复用 `PreviewSaveActive` 现有的落盘实现(不是重新发明一套保存),差别只是触发时机(用户主动 ⌘S vs. 切换/关闭前自动触发)。

**改动**:

- `preview_pane_for` 构建每个 tab 时(`panel_tab` 调用处),`suffix` 参数从恒为 `None` 改成:`wry_toggle_eligible(path)` 为真时给 `Some(切换按钮)`,按钮图标按 `tab.editor.is_some()` 二选一(预览态显示"切到代码"的图标,代码态显示"切回预览"的图标,具体用哪个 `IconKind` 由实现阶段核对 `icons.rs` 现有素材决定,没有合适的再新增)。
- `PreviewCloseTab`/`ProjectPreviewCloseTab` 的处理分支:关闭前检查目标 tab 是否 `editor.is_some() && dirty`,是则先跑静默保存再移除,不是则维持现状直接移除。

**移除**(见"目标"1、2 完整清单,此处不重复)。

## 错误处理

- 切到代码模式时读盘失败(文件被外部删除、权限变化等):复用 `push_tab` 建原生 tab 失败时已有的处理路径,不新造一套错误展示——大概率是保持该 tab 原本的 wry/flyfish 渲染 + 走 tab 栏下方既有的 `preview_error`/`project_preview_error` 提示条。
- 静默保存(切回预览触发、关闭触发)失败(磁盘满、权限变化等):复用 ⌘S(`PreviewSaveActive`)保存失败时已有的错误展示路径(`preview_error` 提示条)——"静默"只是指不弹确认框打断操作流程,失败信息仍然要让用户看得到,不能完全无声无息。保存失败时,切换/关闭动作本身**不阻断**(用户已经决定要切走/关闭,不能因为保存失败就把人卡在原地),但要把失败原因展示在提示条里,让用户知道这次的改动可能没有真正落盘。

## 测试策略

- 纯逻辑单测:`wry_toggle_eligible()` 对代表性扩展名的判定——`.md`→true;`.rs`/`.py`/`.json`→false(默认原生,不是 wry 打开);`.png`/`.pdf`/`.zip`→false(真二进制,is_editable_extension 本身就是 false)。
- 手动验证:
  1. 打开一个 `.md` 文件,确认默认走预览渲染(wry/flyfish)且 tab 上出现切换按钮。
  2. 点按钮切到代码模式,确认语法高亮/可编辑/dirty 星号与其它原生 tab 一致。
  3. 改点内容不保存,点按钮切回预览,确认改动被静默保存且预览显示的是最新内容(不是切换前的旧渲染)。
  4. 再点回代码模式,确认看到的是刚才保存过的内容(没有丢失、也没有回退)。
  5. 改点内容不保存,直接点 tab 的 `×` 关闭,重新打开同一文件,确认改动已经落盘(关闭前静默保存生效)。
  6. 打开一个 `.rs`/`.py` 等本来就原生可编辑的文件,确认 tab 上**没有**切换按钮,行为与改造前一致。
  7. 打开一张图片/一个 PDF,确认 tab 上没有切换按钮。
  8. 右键任意预览 tab,确认没有菜单弹出;确认没有任何入口能再打开旧的全局编辑弹层。

## 排期备注

- 独立 worktree 分支开发,完工经代码审阅通过后再合并 main。
- 建议任务拆分:先做"移除右键菜单"和"移除全局编辑弹层"这两块删除性质的改动(风险低、互相独立,可并行或先后做,合并后代码库更干净),再做"新增切换按钮 + 静默保存"这一块新增行为(依赖前两块清空后的 `preview_pane_for`/`PreviewTab` 结构更利于插入新逻辑),最后一个 Task 做人工 GUI 验收清单(覆盖上面测试策略的 8 项)。
