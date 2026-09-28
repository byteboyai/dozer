# Agent-native 文件编辑器:Files & Preview 统一设计与 Phase 1(精确修改 + 历史)

**状态:待批准(brainstorming 会话,2026-09-28)**

本设计是 `docs/dozer-v2/Dozer-V2_Agent-native_Editor_Intentional_Requirements_v0.1.md`
(以下简称"v0.1 意向文档")的 Phase 1 落地——那份文档是产品意向/顶层需求,
定了术语(Anchor、Mutation Engine、Context Scope、Provenance……)和总体架构,
本 spec 是它在 Phase 1(文本、精确修改+历史)范围内的具体工程设计,术语和
分层原则跟随 v0.1,不重新发明一套。

## 背景与定位调整

Files 与 Preview 目前是两个各自独立、以"查看"为核心的面板。这次设计把两者
统一成 v0.1 意向文档定义的目标:**Agent-native 通用文件编辑器**——human 审阅
文件、提出修改意见,agent 通过对话实时把意见变成文件内容的精确修改,human
立刻在原地看到结果,形成"意见 → 修改 → 立即可见"的闭环。**agent 是编辑的
主体**,human 不需要自己动手改。工程原则沿用 v0.1 §2:

> Agent 的每一次修改,都必须**可定位、有边界、具备事务性、可追溯、可撤销**
> (Anchored + Scoped + Transactional + Traceable + Reversible)。

这是对现有裁决的正面推翻,需要同步修订(见"需要同步修订的既有结论"一节):

- `CLAUDE.md` 关键裁决:"预览类功能(文件预览、Preview WebView 等)优先做
  '渲染/查看'而非'编辑'"。
- `docs/superpowers/specs/2026-09-16-claude-code-ide-bridge-design.md` 的范围
  声明:"Dozer 核心裁决是预览优先于编辑,这条通道只用来让 Claude Code 知道
  用户在看哪个文件,不用来把 Claude Code 的改动通过这条协议推回 Dozer 显示
  diff"。

旧裁决没有错——在"human 自己编辑,Dozer 只负责验收"的定位下,预览优先于编辑
是对的收敛。这次是定位本身变了:核心场景从"验收"扩展到"实时协作编辑",在
新定位下,不给 agent 精确编辑能力才是缺陷。

## 需求全貌

一次性收集的九条原始需求,合并整理如下(与 v0.1 意向文档的对应关系见各条
括注):

1. human 必须能预览尽可能多的文件类型(v0.1 §4 Viewer)。
2. human 与 agent 必须能精确共享"视野"——被动的"human 现在正看着哪"
   (v0.1 §5 Shared Viewport),以及 human 主动通过右键把一段选中内容送出去
   (v0.1 §7 Typed Selection)。
3. agent 必须能精确执行 human 提出的修改,整篇重写和局部精确修改都要支持
   (v0.1 §12 Mutation Engine、§14 局部修改与整篇重写)。
4. agent 改完之后必须立刻在 human 视野里刷新可见,并定位回(改动后的)对应
   位置(v0.1 §17 Change Feedback)。
5. human 必须能看到 agent 正在使用/操作哪些文件——agent 终端下方要有一个
   上下文目录·文件列表(目录代表其下全部文件);可以通过右键把文件/目录
   送进这个列表;修改完成后可以弹窗看修改历史(是否弹出由按钮控制,不强制)
   (v0.1 §10 Context Scope、§18 Change History、§20 Review 模式)。
6. 定位信息按文件类型分别定义:文本给行列+选中内容,表格给单元格坐标,图片
   给选区信息(v0.1 §7 Typed Selection、§9 Stable Anchor)。

**关于 Phase 3 的前瞻性备注(brainstorming 过程中用户补充,2026-09-28)**:
agent 上下文列表的"一项"以后不会只是"文件/目录"——用户明确说了以后要把
Conversations 面板(对话/会话)、Todo 任务、SSH 主机、数据库连接也纳入这个
列表。Phase 1 不受影响(只处理文件精确修改),但真正设计 Phase 3 时,上下文
列表的数据模型不能硬编码成"文件路径",要按"引用某个实体(文件/目录/todo/
会话/ssh 主机/数据库连接)"这种更通用的口子设计,否则以后加类型要返工。
这条现在只是记录,不在本 spec 展开设计。

## 范围拆分

九条需求横跨四块相对独立的工作,建议按下面顺序分阶段、每阶段各自过一遍
spec → plan →实现:

| 阶段 | 内容 | 对应需求 | 状态 |
|---|---|---|---|
| **Phase 1**(本 spec) | Mutation Engine 的写入通道 + Change History,只做 TextAdapter(CodeMirror)一种 | 3、4,6 里的文本部分 | 本次设计 |
| Phase 2 | Typed Selection 的发送动作(右键菜单:文件树的文件/目录 + 预览内选区) | 2、5 里的"发送"部分 | 后续单独 brainstorm |
| Phase 3 | Context Scope 列表 UI(终端下方列表)+ Change History/Review 弹窗 UI | 5 里的"列表/弹窗"部分 | 后续单独 brainstorm,依赖 Phase 1 的历史数据 + Phase 2 的上下文数据 |
| Phase 4 | 新增 Adapter:表格(dozer-tabular,单元格 Anchor)、图片(选区 Anchor);DOCX 视具体选型另议 | 6 里的表格/图片部分 | 后续单独 brainstorm |

需求 1(预览覆盖面本身)是既有的持续性工作(文件预览路由/查看器系列),
不因这次定位调整而新增范围,不在这四个阶段里。

**MVP 文件类型范围的一处偏差**:v0.1 §24 建议 MVP 同时验证 Text/Markdown、
dozer-tabular、DOCX 三种类型,以证明协议能跨类型泛化。本系列把这一步推迟到
Phase 4——DOCX 目前连 viewer/adapter 都不存在,dozer-tabular 另有一份独立在
跑的 migration spec(`2026-09-28-tabular-webview-migration-design.md`),三者
在 Phase 1 一起做工作量和风险都明显更大。先用最便宜的 TextAdapter 把 Anchor/
Mutation/Conflict Detection/Change Feedback/History 这条协议完整跑通、验证
可行,Phase 4 再把同一套协议接到 dozer-tabular 和图片(DOCX 视具体选型另外
评估),不是削减 v0.1 的目标,只是改变验证顺序。

本 spec 只覆盖 **Phase 1**。

## Phase 1 设计

### 路由与署名

一次精确修改由谁执行、历史记录署名给谁,取决于当前会话的 agent 是否有
`dozer-mcp` 通道(见 `docs/user_guide/mcp.md`):

- **Claude、CodeBuddy、Codex、OpenCode、v8agent**:human 在自己正常聊天的
  终端里描述修改意见(不需要新的输入通道),agent 自己调
  `locate_in_file`/`apply_precise_edit` 完成修改,历史记录署名这个 agent
  本身。
- **Goose、Aider**(目前没有 MCP 通道,见 `mcp.md`):human 依然在自己正常
  聊天的终端里跟它们对话,但 Dozer **额外**并行 dispatch 一个 headless
  v8agent 一次性任务(复用 `dozerd::task_processor`/`headless_agent::
  process_task_headless` 这套 Todo 已经在用的一次性调用机制,而不是往
  Goose/Aider 的终端里模拟打字——那条路径它们本来就在正常使用,不需要
  Dozer 插手),由这个 headless v8agent 去调 `locate_in_file`/
  `apply_precise_edit` 真正落地这次精确修改,历史记录署名 `v8agent`,
  如实反映谁动的手。这个决定的先例是 `summary_config.rs` 里对话总结器
  默认 `provider: AgentKind::V8agent`,与当前会话是哪家 agent 无关。

这条路由判断只影响"谁来精确落地这次编辑",不影响 human 和 Goose/Aider 之间
正常的对话——那条通道完全不变,Goose/Aider 该怎么用还怎么用,只是它们自己
用原生工具做的编辑不会自动获得"精确坐标校验 + 自动重新定位 + 结构化历史"
这三样,这三样由并行 dispatch 的 headless v8agent 补上。

**依赖说明**:Dozer 不解析 Goose/Aider 终端里的自由聊天文本,所以"human 提了
个修改意见"这件事本身,对 Goose/Aider 会话来说目前没有一个可以拿来触发
headless v8agent dispatch 的信号——自由文本不算数。这个触发点实际上要等
Phase 2 的"发送到上下文"落地、提供一个结构化的意见输入框(不是自由终端文本)
才真正存在。也就是说 Phase 1 本身(直接调 MCP 工具)对 Claude/CodeBuddy/
Codex/OpenCode/v8agent 五家可以独立验证,但 Goose/Aider 的兜底路由要等
Phase 2 才能真正跑通,Phase 1 阶段只把路由逻辑和数据模型準备好。

### Stable Anchor(v0.1 §9):文本类型的定位模型

v0.1 定义 Anchor 的概念模型是 `{resource_id, object_id, position,
content_hash, surrounding_context}`,不同 Adapter 用不同策略——文本用
"line/column + content fingerprint"。Phase 1 的 TextAdapter 按这个策略实现:

- `position` = `start_line/start_col`~`end_line/end_col`(1-based)。
- `content_hash`/`surrounding_context` 在 Phase 1 具体实现为**字面的
  `expected_text`**(要修改范围当前内容的原样文本),而不是哈希——原因是
  精确修改的目标范围通常不大(局部修改是默认模式,见 v0.1 §14),逐字节
  比对比哈希更直观:校验失败时可以把"实际内容 vs expected_text"的差异
  直接展示/回传给 agent,不用像哈希那样只能回答"对不对"而答不出"哪里不对"。
  这是同一个 Anchor 概念在文本类型上的一个实现选择,不是偏离模型。

**新增两个 MCP 工具,对应 v0.1 的"定位"与"Mutation"两层**:

**`locate_in_file(path, query)`**——只读,解析 Stable Anchor。在 `path`
指向的文本文件里搜索 `query`,唯一匹配时返回该处坐标(`start_line/
start_col`~`end_line/end_col`)+ 前后若干行上下文;匹配到多处时返回全部
候选(各自的坐标+上下文片段),不擅自选一个,逼调用方把 `query` 写得更长/
更具体以缩小到唯一匹配。存在的意义是不能指望 agent 自己数出准确的行列
号——坐标应该由 Dozer 算好再给,不是让 agent 猜。

**`apply_precise_edit(path, start_line, start_col, end_line, end_col,
expected_text, new_text, summary)`**——写,是 Mutation Engine 对 TextAdapter
暴露的唯一 Operation。v0.1 §12 列出的 `replace/insert/delete/rewrite/move`
里,`insert`(零宽度区间,`start`==`end`)和 `delete`(`new_text` 为空)都是
`replace` 的特例,不需要单独开工具——减少 MCP 工具数量对 agent 的工具选择
可靠性更有利。`rewrite`(v0.1 §14,整篇重写,Markdown/小型文本/human 明确
要求重新生成时用)同样是特例:`start`/`end` 覆盖整个文件(`[1,1]` 到
`[末行,末列]`)。`move` 暂不支持(Phase 1 无跨文件/跨位置移动场景)。

语义:

1. 校验 `path` 落在项目根目录子树内,越界拒绝。
2. 校验目标文件不是二进制/非 UTF-8/lossy 编码(复用现有
   `save_gate`/`can_save` 那套边界,同一个理由:这些文件本来就不允许写)。
3. 校验该文件当前**没有**对应的脏 tab(human 有未保存的本地修改)——有则
   直接拒绝,报错提示"human 有未保存的本地修改,请提醒 ta 先处理",不做
   任何自动合并。
4. **Conflict Detection**(v0.1 §16):读磁盘当前内容,取
   `[start_line,start_col]`~`[end_line,end_col]` 这段,逐字节比对是否等于
   `expected_text`;不等则产生一个 `MutationConflict`——拒绝执行,把**当前
   实际内容**连同坐标一起回给调用方(不是笼统报错),让它能照着真实内容
   重新算坐标再调一次,而不是直接覆盖最新文件(v0.1:"Agent 应重新读取相关
   内容,而不是直接覆盖最新文件")。
5. 校验通过则把这段替换成 `new_text`,写回磁盘。
6. 计算替换后 `new_text` 对应的新坐标区间,写一条 Change History 记录
   (见"Change History / Provenance"一节),然后触发"Change Feedback"
   (见下一节)。

`summary` 是必填的一句话说明(这次改了什么/为什么),类似一条迷你 commit
message,对应 v0.1 Provenance 模型里的 `Reason`,供历史弹窗展示用,不是
可选项。

不需要旧协议里的 `expected_revision`(绑定 CodeMirror 内存里的编辑器
revision 计数器)——它要求目标文件必须已经开在某个 Preview tab 里才有意义,
但 agent 要改的文件不一定被 human 打开过(第 5 条"目录当上下文"意味着 agent
可能要碰一批 human 根本没点开过的文件)。改成直接对磁盘操作,`expected_text`
校验本身就同时覆盖了"坐标算对了没"和"内容是否被并发改过"两件事(即
Conflict Detection),不需要另一层版本号保护。

预览命令协议里已有的坐标式 `PreviewCommandAction::Replace`(`expected_
revision` 那一版)保留给 UI 自己触发的编辑路径用,**不**作为这次 agent 精确
修改的入口——两者服务的场景不同,不合并。

### Change Feedback(v0.1 §17):刷新 + 定位 + 高亮

`apply_precise_edit` 写盘这个动作本身会被现有的 `git_watch.rs`(通用文件
系统监听,不只管 git 状态)捕获,干净 tab 走既有的"原地重载"机制
(`reload_webviews_for` → `ReloadDocument`,不换 URL、不丢已渲染内容)——
这条路径已经存在,不需要为"agent 改完之后通知 Dozer"另开一条新通路。

新增的部分是 v0.1 §17 定义的完整反馈链路:

```
Mutation → refresh affected region → resolve anchor → scroll_to(anchor)
→ highlight changed content
```

具体到实现:`apply_precise_edit` 成功后自动排一条指向新坐标区间的
`Reveal`/`Select` 命令,在对应 tab 完成 `ReloadDocument` 之后触发,让视图
落到刚被替换的这段内容上;紧接着再叠加一个**短暂高亮**(新的
`PreviewCommandAction::Highlight{start,end,duration_ms}`,纯视觉装饰、
定时消退,不改变实际的文本选区状态——故意跟 `Select` 分开,`Select` 会
动到 human 自己后续操作会用到的"当前选中范围",高亮不应该有这个副作用)。
"重载完成之后再定位/高亮"的先后顺序细节留给实现计划处理,不在这里锁定
具体实现手法。核心原则是 v0.1 §17 说的"Human Attention Continuity"——human
不应该因为 agent 的修改而丢失注意力焦点。

如果目标文件当前没有开在任何 tab 里,不强制自动打开——只是这次编辑不会有
视觉上的定位/高亮效果,human 通过第 5 条要做的上下文列表/历史弹窗仍能看到
这处改动,只是不会被动弹出来抢焦点。

### Change History / Provenance(v0.1 §18、§19)

新增一张表(暂定 `file_edit_history`,权威存储在 `dozerd` 的 `dozer.db`,
`project_id` 作用域,和 `TodoStore`/`MemoryStore` 同一套既有模式),
`apply_precise_edit` 每次成功执行写一行,字段对应 v0.1 §19 的 Provenance
模型(Agent/Task/Source/Target/Mutation/Before/After/Reason/Session/
Timestamp):

- `id`、`project_id`
- `target_path`(项目内相对路径,对应 Provenance 的 `Target`)
- `actor`——`AgentKind::as_str()`,MCP 路径署名当前 agent,兜底路径恒
  `"v8agent"`(对应 `Agent`)
- `session_id`——产生这次修改的会话 id(MCP 路径是真实交互会话;兜底路径
  是 headless 一次性调用铸造的会话 id,同 Todo 的 `dispatch_session_id`
  做法),供历史弹窗跳转回 Conversations 面板看完整对话,不需要把整段
  对话复制进这张表(对应 `Session`)
- `task_ref`——**Phase 1 恒为 `NULL`,列先建好**。对应 Provenance 的
  `Task`(这次修改隶属于哪个更高层任务,比如一条 Todo)。Phase 1 的
  "意见→修改"是单次、即时的对话式交互,还没有"任务"这个更高层的概念可以
  挂;第 5 条前瞻性备注里提到以后 Todo 会纳入 agent context,那时候
  `task_ref` 才有值可填,现在先占位,免得那时候要迁移表结构。
- `source_ref`——**Phase 1 恒为 `NULL`,列先建好**。对应 Provenance 的
  `Source`(这次修改的依据是不是来自另一个文件,比如"根据 valuation.xlsx
  更新 report.docx"这种跨文件场景)。Phase 1 是单文件精确修改,不涉及跨
  文件溯源,同样先占位到 Phase 4(多 Adapter/跨文件场景更可能触发这个需求)
  再实际使用。
- `start_line`/`start_col`/`end_line`/`end_col`(改之前的坐标,对应
  `Mutation` 的定位部分)、`old_text`(`Before`)、`new_text`(`After`)——
  只存被改的那个 range,不是整份文件快照,精确修改本来就是局部的,存全量
  没必要,也避免了大文件整篇快照的存储开销
- `summary`——工具调用里那句必填说明(对应 `Reason`)
- `created_ms`(对应 `Timestamp`)

不做 upsert(不像 `MemoryStore` 按标题合并同一条记录)——这是一张纯追加
的事件日志,一次编辑一行,`target_path` 上的多次编辑天然按时间顺序排列成
一个"这个文件被改过什么"的列表,供 Phase 3 的历史弹窗直接读取展示(UI 交互
沿用 Memory 面板"列表 → 点进一条看详情"已经验证过的模式,这次不需要重新
设计一遍,细节留给 Phase 3)。

这张表本身只由 GUI(通过 `dozer-client`)读取,不额外暴露给 agent 的 MCP
工具——查询"这个文件被改过什么"是给 human 看的治理层信息,不是 agent
需要主动查询的能力,YAGNI。

**Revert 不需要新机制**(v0.1 §18 把 Locate/Diff/Revert/Ask Agent 列为
history 上的人工操作):这张表本身就存了每条记录的 `old_text`/`new_text`,
"撤销一条历史"等价于拿这条记录反过来再调一次 `apply_precise_edit`
(`expected_text` = 该记录的 `new_text`,`new_text` = 该记录的 `old_text`)
——复用同一个 Mutation Engine 入口,同样走 Conflict Detection(如果磁盘从
那次修改后又被改过,`expected_text` 会对不上,合理地拒绝撤销而不是盲目
覆盖)。Phase 1 不需要为 Revert 单独设计后端能力,Phase 3 只是在历史弹窗上
加一个按钮调用同一个工具,细节留给 Phase 3。

## 非目标(Phase 1)

- 表格(dozer-tabular 单元格)、图片(选区)的 Adapter——Phase 4,连同 v0.1
  §24 建议的 DOCX 一起放到那时候评估。
- Typed Selection 的发送动作、Context Scope 列表 UI、Change History/Review
  弹窗 UI——Phase 2/3。
- 扩大文件预览覆盖面——既有的独立持续性工作,不因这次调整而新增范围。
- 把 `PreviewCommandAction::Replace`(UI 自身编辑路径用的坐标式协议)删掉
  或改造——两条路径服务场景不同,保留不动。
- 给 Goose/Aider 补 MCP 通道本身——这是另一件更大的事,不在这次范围里,
  这次只是绕开它们缺 MCP 这件事的一个补偿方案。
- **Context Scope 的 Read/Read-Write 权限位**(v0.1 §11):v0.1 明确要求
  "加入 Context 不应自动意味着 Agent 可以修改",按资源区分只读/可写。
  Phase 1 还没有 Context Scope UI(Phase 2/3 才做),`apply_precise_edit`
  目前**默认项目内任何非二进制/非脏文件都可被修改**,没有"这个文件不给
  agent 改"的开关——这是已知留白,不是遗漏:没有 UI 也没有数据结构能表达
  "只读"这个状态,等 Phase 2/3 的 Context Scope 落地、真的有"加入上下文"
  这个动作时,权限位才有地方挂,现在强行加一个没有输入渠道的开关没有意义。

## 需要同步修订的既有结论

Phase 1 实现落地后(不是现在),下列文档需要跟着改,列在这里是为了不漏项,
不是本 spec 现在就要动手改:

- `CLAUDE.md` 关键裁决"预览优先渲染/查看而非编辑"——需要改写,明确"人不
  自己动手改"和"agent 可以精确改"并不矛盾,旧措辞已经不准确。
- `docs/superpowers/specs/2026-09-16-claude-code-ide-bridge-design.md` 的
  范围声明——"不做编辑类能力"这句话对 IDE 桥接这条**只读**自动上下文通道
  本身仍然成立(它确实不做编辑),但不能再被引用成"Dozer 整体上预览优先于
  编辑"的证据,需要加一句澄清或链接过来。
- `docs/user_guide/mcp.md`、`agents.md`、`panels.md`——Phase 1 落地后补上
  `locate_in_file`/`apply_precise_edit` 两个新工具的文档,同批更新 Files
  面板的能力描述。

## 测试

- `locate_in_file`:唯一匹配返回正确坐标;多处匹配返回全部候选且不擅自
  选择;文件不存在/路径越界的错误路径。
- `apply_precise_edit`:成功路径(坐标/内容/写盘/历史记录/自动定位+高亮
  命令都发生);`expected_text` 不匹配时产生 `MutationConflict` 且回传真实
  内容;脏 tab 拒绝;二进制/非 UTF-8 拒绝;路径越界拒绝;整篇覆盖
  (`[1,1]`~`[末行,末列]`)等价于整篇重写;插入(零宽度区间)和删除
  (`new_text` 为空)等价于 `replace` 的特例。
- 路由:MCP 可用 agent 直接执行且署名正确;Goose/Aider 会话下并行触发
  headless v8agent dispatch 且历史记录署名 `v8agent`。
- Change Feedback:干净 tab 收到 `ReloadDocument` 后能收到指向新坐标的
  `Reveal`/`Select` 与 `Highlight`;目标文件未打开任何 tab 时不强制打开。
- Revert:拿历史记录反向调用 `apply_precise_edit` 能正确撤销;磁盘在此之后
  又被改过时,反向调用应因 `expected_text` 不匹配而被拒绝,不能盲目覆盖。
