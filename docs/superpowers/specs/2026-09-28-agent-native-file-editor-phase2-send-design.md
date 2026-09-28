# Agent-native 文件编辑器 Phase 2:Typed Selection 的发送动作

**状态:待批准(brainstorming 会话,2026-09-28)**

本设计是 `docs/superpowers/specs/2026-09-28-agent-native-file-editor-design.md`
(以下简称"Phase 1 spec")"范围拆分"表里 **Phase 2** 的正式落地——那份表格
当时只留了一句话:"Typed Selection 的发送动作(右键菜单:文件树的文件/目录 +
预览内选区),对应需求 2、5 里的'发送'部分,状态'后续单独 brainstorm'"。本
spec 就是那次单独 brainstorm 的产出,术语和分层原则跟随
`docs/dozer-v2/Dozer-V2_Agent-native_Editor_Intentional_Requirements_v0.1.md`
(以下简称"v0.1 意向文档"),不重新发明一套。

## 背景

v0.1 §10 明确区分三个不能混淆的概念:

> Context Scope(Agent 可以使用哪些资源) / Live View(Human 当前正在查看
> 什么) / Selection(Human 当前明确指出了什么)

Phase 2 覆盖后两者里"主动发送"的部分——文件树右键把文件/目录"送进"agent
的工作上下文(Context Scope 的写入动作),以及预览内选中一段文本后右键
"发送给 Agent"(v0.1 §7 Typed Selection 的发送动作)。**列表 UI(终端下方
"正在使用哪些资源"的展示、历史/Review 弹窗)不在本 spec 范围内**,那是
Phase 3 的工作,依赖关系见下面"与 Phase 3 的关系"一节。

## 核心决策:两个动作共用同一个投递原语

Phase 1 花了大篇幅设计"MCP 可用 agent(Claude/CodeBuddy/Codex/OpenCode/
v8agent)直接调 `locate_in_file`/`apply_precise_edit`,Goose/Aider 没有 MCP
通道,要并行 dispatch 一个 headless v8agent 兜底"这套路由,是因为 Phase 1
的落地动作(精确修改文件)必须由 agent 亲自调用 MCP 工具完成,而 Goose/Aider
没有 MCP 通道。

Phase 2 不落在这个问题里。两个"发送"动作的实现都是同一件事:**把一段
结构化文本写进当前项目当前激活的 agent 终端输入框**,复用已存在的
`term_paste(target, text)`(`crates/dozer-app/src/app/update.rs:4615`,⌘V
粘贴走的同一条 PTY 写入路径,只写入不自动回车)。`term_paste` 只关心"往哪个
终端写字节",完全不关心终端里跑的是哪家 agent——不管是 Claude 还是 Goose,
"发送"对它们一视同仁,都只是在它的输入框里多打了几行字,由 human 决定要不要
再补一句话、要不要回车。**Phase 1 那套 MCP/headless 路由判断,Phase 2 完全
不需要重新做一遍。**

这也是为什么 Phase 2 不需要引入任何新的持久化存储或新的 MCP 工具:两个动作
都是"写终端"这一个原语的两种文本模板,没有第三条路径。

## 需要新增/修改的两个动作

### 动作一:文件树右键"添加到 Agent 上下文"(文件/目录)

对应 v0.1 §10 Context Scope 的写入动作,需求 5"发送"部分。

**触发点**:`crates/dozer-app/src/extensions/files/view.rs:908`
`context_menu_spec()`——现有三段式(顶部操作组/中间主操作组/底部工具组)
基础上,在**顶部操作组**新增一项:

- 图标:`icons::IconKind::MessageSquare`
- 标签:"添加到 Agent 上下文"
- 消息:新增 `Message::SendToAgentContext(PathBuf, bool)`(`bool` = `is_dir`,
  与现有 `ContextMenuOpen { path, is_dir }` 同款签名习惯)

文件夹当前顶部组是 搜索/新建文件/新建文件夹,文件当前顶部组是 回滚/历史
(仅 git 仓库文件才有)。新项在两种情况下都追加到顶部组末尾,不影响现有
分隔线逻辑(`context_menu_spec` 尾部"顶部组非空才加分隔线"的判断不变,
因为文件的顶部组从"可能为空"变成"恒非空",分隔线会恒渲染——这是预期内
的视觉变化,不需要额外处理)。

**文本模板**(纯文本拼接,不用 Markdown 代码围栏,避免路径或后续内容里的
反引号污染格式):

```
请将 {relative_path} 文件纳入你的工作上下文。
```

目录同理,路径末尾补 `/` 以示区分,不展开列举目录下的文件——按 brainstorming
阶段的决定,agent 自己有工具(ls/glob/read)可以探索目录内容,不需要 Dozer
在发送时就把整棵子树的路径列表灌进终端;目录可能有几百上千个文件,展开
列举会把终端输入框刷屏,也不符合 v0.1"Agent 自己动手"的精神:

```
请将 {relative_path}/ 目录纳入你的工作上下文。
```

`relative_path` 是相对项目根目录的路径,与文件树右键既有的"复制相对路径"
(`Message::CopyPath(target, PathKind::Relative)`)用同一个路径计算逻辑,
不重新实现一遍。

### 动作二:预览内选区右键"发送给 Agent"

对应 v0.1 §7 Typed Selection 的发送动作,需求 2"发送"部分。**范围限定为
Phase 1 TextAdapter 覆盖的 CodeMirror 承载的文本类预览**——与 Phase 1 的
`apply_precise_edit`/`locate_in_file` 同一个文件类型范围,JSON tree 视图、
图片、dozer-tabular 等非 CodeMirror 宿主的预览不受影响(它们目前也没有
`SelectionChanged` 事件可以复用,谈不上"关闭"这个功能,只是本来就没有)。

**背景**:CodeMirror host(`crates/dozer-app/assets/editor/`)目前完全没有
拦截浏览器的 `contextmenu` 事件,选中文字后右键会弹系统原生菜单。已有的
`EditorEvent::SelectionChanged`(`crates/dozer-app/src/preview/webview_protocol.rs:82`)
只在选区**变化**时上报一次,不适合直接拿来做右键时刻的数据源(存在"选区
变化后、右键之前又发生了别的操作"的时序竞态风险)。

**JS 侧新增**:在 `assets/editor/` 里给 CodeMirror 容器加一个
`contextmenu` 监听器:

1. 现场读取当前选区(CodeMirror `EditorState.selection`),若为空
   (`selection.main.empty`),**不拦截**,不调用 `preventDefault()`——右键
   照常弹系统原生菜单。这是刻意的范围收紧:Phase 2 只做"选中了内容之后
   发送这段内容",不做"没选中任何东西时发送整篇文件"这类超出 brainstorm
   范围的功能,后者如果将来需要,应该另开一次 brainstorm。
2. 选区非空时,`event.preventDefault()`,通过既有 webview envelope
   (`WebviewEnvelope<EditorEvent>`)上报新事件变体:

   ```rust
   ContextMenuRequested {
       x: f32,
       y: f32,
       range: TextRange,
       selected_text: String,
   }
   ```

   `x`/`y` 是鼠标在 webview 内容区域内的本地坐标(`event.clientX/clientY`);
   `range`/`selected_text` 是**当次事件现场取值**,不依赖 Rust 侧已缓存的
   `tab.web_selection`,避免"最后一次 `SelectionChanged` 和这次右键之间
   选区已经变了"的竞态。

**Rust 侧新增**:

1. `crates/dozer-app/src/preview/webview_protocol.rs` 的 `EditorEvent`
   枚举新增 `ContextMenuRequested` 变体(字段同上)。
2. 收到该事件后,用 `crate::webview_geometry::preview_content_bounds_for`
   算出当前 webview 在窗口里的原点 `(x0, y0)`,把事件里的 `(x, y)` 换算成
   窗口坐标——与 Files 右键既有的 `last_right_click` 是同一套坐标体系,
   换算方式照抄 `Message::RightClickAt` 的用法(`extensions/files/update.rs:41`)。
3. 弹出的菜单**只有一项**:"发送给 Agent"(图标 `IconKind::MessageSquare`,
   与动作一同款,视觉上强化"这是同一类操作"),消息为新增的
   `Message::SendSelectionToAgent { path: PathBuf, range: TextRange, selected_text: String }`。
   - macOS:走既有的原生 NSMenu 路径(`menu_spec::to_native`),这条路径
     本身就是操作系统原生层,天然浮在 webview 之上,不受 CLAUDE.md 里
     "老式 iced 内浮层需要显式隐藏 webview"那条约束的影响。
   - 非 mac:走 Files 已有的 iced fallback 弹层(`context_menu_popup` 同款
     结构),需要把这个新弹层的"展开中"状态并入 `App::preview_desired`
     判断 webview 矩形是否要下推/隐藏的既有逻辑里(与 Files/Project 的
     `tab_context_menu`/`context_menu` 互斥清理约定一致,新状态同样要在
     `ContextMenuClose` 等收尾路径里被清空)。
   - 若当前项目**没有可见/激活的 agent 终端**(判定复用 `term_paste` 已有
     的 `terminal_visible()`/`ssh_terminal_visible()` 门槛),"发送给
     Agent"这一项直接置灰不可点——与 Files 右键"粘贴无剪贴内容时置灰"
     是同一套现成模式(`context_menu_spec` 里 `has_clipboard` 的用法),
     不是点了之后再报错。

**文本模板**(复用 Phase 1 已定的 1-based line/col 坐标格式——这正好是
`apply_precise_edit` 需要的坐标+`expected_text`组合,agent 如果要直接照做
修改,可以跳过 `locate_in_file` 直接调 `apply_precise_edit`,因为精确坐标
和原文已经在这段引用文字里了):

```
参考 {relative_path}:{start_line}:{start_col}-{end_line}:{end_col} 这段内容:

{selected_text}
```

不用代码围栏(```),原样拼接——`selected_text` 本身可能含有反引号或恰好是
一段 Markdown/diff 内容,用围栏包裹反而可能被内容本身破坏格式;纯文本前缀
+ 空行分隔已经足够让 agent 分清"这是引用的原文"。

## 与 Phase 3 的关系(一处对 Phase 1 spec 措辞的修正)

Phase 1 spec"范围拆分"表原话是:"Phase 3……依赖 Phase 1 的历史数据 +
Phase 2 的上下文数据"。本次 brainstorm 的决定是 Phase 2 **不建任何新的
持久化存储**——两个"发送"动作都只是往终端里写一段文本,不落库。这与那句
"依赖 Phase 2 的上下文数据"的措辞对不上号,在这里显式记一笔:

- Phase 3 设计"终端下方的资源列表"UI 时,不能假设 Phase 2 已经准备好一张
  可以直接查询的 Context Scope 表——那张表(如果需要)得由 Phase 3 自己
  设计数据模型并决定写入时机,可能是重新在"添加到 Agent 上下文"这个入口
  上挂一次持久化写入,也可能是别的方案。
- 之所以现在不做,是因为 Phase 1 spec 自己也指出的前瞻性备注还没解决:
  Phase 3 的上下文列表数据模型需要按"引用某个实体(文件/目录/todo/会话/
  ssh 主机/数据库连接)"这种通用口子设计,而不是硬编码成文件路径;现在
  Phase 2 阶段这个通用模型影子都没有,强行建一张"文件专用"的表,大概率
  在 Phase 3 落地时要推倒重来(与 Phase 1 spec 里 `task_ref`/`source_ref`
  提前占位、避免后续迁移表结构是同一种顾虑,只是这次的结论正好相反:
  提前建反而更可能返工,所以选择完全不建)。

## 非目标(Phase 2)

- **不建 Context Scope 持久化存储**——见上一节,两个动作都只写终端,不
  落库。
- **不做 Context Permission(v0.1 §11)权限位**——跟 Phase 1 一样的理由,
  没有列表 UI,开关无处挂,现在加没有意义。
- **不支持非文本类型的选区**(表格单元格、图片区域、DOCX 段落)——那些
  Adapter 要 Phase 4 才有,本 spec 的"预览内选区"严格限定 CodeMirror
  文本预览。
- **不做"发送后自动回车执行"**——`term_paste` 只写输入框,不模拟回车键,
  执行与否完全由 human 决定。
- **不重新实现原生 Copy/Cut/Paste**——选区右键菜单只有"发送给 Agent"
  一项,拦截 `contextmenu` 意味着右键菜单里不再有浏览器原生的复制/剪切/
  粘贴选项,但 ⌘C/⌘V 键盘快捷键完全不受影响,这个取舍已经确认可以接受。
- **不为 Goose/Aider 做任何特殊路由**——`term_paste` 天然不区分 agent
  种类,Phase 1 那套"MCP 可用直接调工具、否则并行 dispatch headless
  v8agent"的路由判断在 Phase 2 完全不需要。
- **没有选中内容时右键预览不新增任何行为**——不做"发送整篇文件"这类
  超出本次 brainstorm 范围的功能,选区为空时 `contextmenu` 不被拦截,
  浏览器原生菜单照常弹出。

## 测试

- **文件树右键**:文件/目录都能生成新菜单项;新增项在 `context_menu_items`
  (native)和 `context_menu_spec`(iced fallback 数据源)两处都要出现,
  两者共用同一份数据组装,不需要分别测;当前项目无可见/激活终端时该项
  置灰;点击后文本模板按文件/目录分别正确拼出 `relative_path`(含空格/
  中文路径的路径需要单独测一条,确认拼接不做任何转义或截断)。
- **预览选区右键**:
  - 选区非空时右键触发 `contextmenu` 拦截,上报的 `ContextMenuRequested`
    携带正确的 `range`/`selected_text`(现场取值,不依赖旧的
    `SelectionChanged` 缓存)。
  - 选区为空时右键**不拦截**,不产生 `ContextMenuRequested` 事件。
  - webview 本地坐标 `(x, y)` 经 `preview_content_bounds_for` 换算成
    窗口坐标的正确性(不同 `Side`/放大态下的边界场景,复用
    `webview_geometry.rs` 已有的测试夹具风格)。
  - 当前项目无可见/激活终端时菜单项置灰。
  - `selected_text` 含反引号/多行 diff 等特殊内容时,文本模板拼接后不
    破坏格式、不丢内容。
  - 非 mac iced fallback 弹层弹出时,`App::preview_desired` 正确把
    webview 矩形下推/隐藏;弹层关闭后状态正确清空,不影响 Files/Project
    既有的 `tab_context_menu`/`context_menu` 互斥清理。
- **多项目场景**:在项目 A 的文件树/预览触发发送动作,文本应该写进项目 A
  当前激活的终端,不能因为其他项目当前也开着而发错地方
  (`with_focused_project` 的既有语义应该天然保证这点,但要补一条集成
  测试锁定,防止将来重构改坏)。
- **`term_paste` 本身**:确认粘贴含空格/中文/特殊符号的路径或引用文本
  时,不会被终端误当作按键序列或命令解析(`term_paste` 现有实现应该天然
  成立,这里是回归测试,不是新行为)。
