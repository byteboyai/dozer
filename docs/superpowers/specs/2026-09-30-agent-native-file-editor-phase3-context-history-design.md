# Agent-native 文件编辑器 Phase 3:Agent 上下文列表 + 修改历史弹窗

**状态:待批准(brainstorming 会话,2026-09-30)**

本设计是 `docs/superpowers/specs/2026-09-28-agent-native-file-editor-design.md`
(以下简称"Phase 1 spec")"范围拆分"表里 **Phase 3** 的正式落地,对应需求 5
里"列表/弹窗"部分。术语与分层原则跟随
`docs/dozer-v2/Dozer-V2_Agent-native_Editor_Intentional_Requirements_v0.1.md`
(以下简称"v0.1 意向文档")§10 Context Scope、§18 Change History。前置:
Phase 1(`file_edit_history` 存储 + `locate_in_file`/`apply_precise_edit`)与
Phase 2(`docs/superpowers/specs/2026-09-28-agent-native-file-editor-phase2-send-design.md`,
两个"发送"动作)均已落地。

## 背景与已确认的决定

Phase 2 的两个发送动作只把文本写进终端(`term_paste`),**没有任何持久化**;
Phase 1 的 `file_edit_history` 只有 dozerd 内部的 `FileEditHistoryStore::
list_for_path`,GUI 读不到。Phase 3 补齐这两块,并把它们呈现出来。
brainstorming 中已与用户确认:

1. **列表来源 = human 显式添加**。只收 human 右键"添加到 Agent 上下文"送进来的
   实体,**不**自动收录 agent 实际读写过的文件(v0.1 §10 的字面定义:Context
   Scope 是 human 声明的范围)。
2. **作用域 = 按项目一份,持久保存,human 手动移除**(与项目级 Memory/Todo 同模式)。
3. **实体范围 = 这一期只做文件/目录**,数据模型按 `entity_kind` + `entity_ref`
   通用设计(Phase 1 spec 前瞻性备注:以后要纳入 Todo/会话/SSH 主机/数据库
   连接,不能硬编码成文件路径),其他类型日后各自加入口即可,本期不做。
4. **权限位(v0.1 §11)不做**:表里预留 `permission` 列、默认值,不进 UI,
   `apply_precise_edit` 也不校验。理由同 Phase 1:没有输入渠道的开关没有意义。
5. **历史弹窗四个操作全做**:Diff / Locate / Revert / Ask Agent(v0.1 §18)。
   弹窗由按钮打开,**不做** Review Mode 自动弹出(v0.1 §20)。
6. **弹窗范围 = 本项目全部修改的时间线,可按文件/目录过滤**。
7. 预览选区的"发送给 Agent"**不入列表**:v0.1 §10 明确区分 Context Scope 与
   Selection,选区是一次性引用,不是"在用的资源"。

## 1. 数据层(dozerd + dozer-core 协议 + dozer-client)

### 新表 `agent_context_items`

权威存储在 `dozer.db`,`project_id` 作用域,沿用 `FileEditHistoryStore` 的
模式(project 级 SQLite,`Mutex<Connection>`,`CREATE TABLE IF NOT EXISTS`):

| 列 | 说明 |
|---|---|
| `id` | 自增主键 |
| `project_id` | 项目作用域 |
| `entity_kind` | 本期取值 `"file"` / `"dir"` |
| `entity_ref` | 本期为项目内相对路径;目录**不带**尾部 `/` |
| `permission` | 预留占位,默认 `"default"`,本期不读不写不进 UI |
| `created_ms` | 添加时间 |

`UNIQUE(project_id, entity_kind, entity_ref)`。**重复添加幂等**:不报错、不新增
行、不改 `created_ms`,返回既有行。

`entity_kind` 用字符串而非枚举落库,以便日后新增类型不需要迁移表结构;Rust
侧用带 `as_str`/`from_str` 的枚举承载,未知取值读出时保留原字符串、UI 显示为
"未知类型"而不是丢弃或崩溃。

### 新增协议请求(`crates/dozer-core/src/protocol.rs`)

- `AddContextItem { project_id, entity_kind, entity_ref }` → 应答返回该行。
- `RemoveContextItem { project_id, id }` → 应答 ok;目标不存在也视为成功(幂等)。
- `ListContextItems { project_id }` → 按 `created_ms` 升序。
- `ListFileEditHistory { project_id, path_filter: Option<String>, limit: u32 }`
  → 按 `created_ms` 倒序。`path_filter` 命中规则:`target_path` **等于**该值,
  或以 `{该值}/` 开头(即位于该目录之下)。`None` 表示不过滤。

`dozer-client` 各加对应方法。历史查询**只暴露给 GUI**,不加 MCP 工具(沿用
Phase 1 结论:这是给 human 看的治理层信息,YAGNI)。

### 校验

`AddContextItem` 在 dozerd 侧校验 `entity_ref` 落在项目根目录子树内、无 `..`
越界,越界拒绝——与 `apply_precise_edit` 第一条校验同一套逻辑,复用不重写。
目标路径在磁盘上不存在时允许添加(文件可能被 agent 稍后创建/删除,列表项
只是声明,不是快照),但 UI 上要显示为"缺失"状态(见 §2)。

## 2. 上下文条 UI(Agent 面板终端下方)

**落点**:Agent 面板终端下方的一条可折叠条(方案 A)。不放进 Files 面板(不符合
"终端下方"的原始需求,且 Agent 面板不可见时看不到)。

- **折叠态**占一行:`Agent 上下文 · N 项`,右侧"修改历史"按钮。
- **展开态**逐项列出:文件/目录图标 + 相对路径 + "移除"按钮;磁盘上已不存在的项
  灰显并标"缺失"。目录项代表其下全部文件(v0.1 §10),UI 不展开列举目录内容。
- 点击某项:文件 → 在 Preview 打开;目录 → 在 Files 面板定位该目录。
- 点击某项旁的"历史"入口(或该项右键)→ 打开历史弹窗且预先按该项过滤。
- 折叠/展开状态是 per-project 的 UI 状态,不落库(重启回默认折叠即可)。

### 状态与消息路由

- 列表状态(项目 A 的条目、折叠态)按项目存放,项目切换互不影响。
- 所有异步结果(`ListContextItems`/`AddContextItem`/`RemoveContextItem` 应答)
  **必须携带 `project_id` 并按它路由**(参见并行多项目里程碑的
  `with_project`/`project_id` 陷阱),不能写进"当前聚焦项目"。
- 条目在以下时机刷新:进入项目、本机执行 Add/Remove 之后、历史弹窗关闭之后。
  本期不做 dozerd → GUI 的推送,不轮询。

### 终端几何(需要专门测试)

条的折叠/展开会改变终端可用高度。终端网格行数必须经
`terminal_pane_pixel_size`(`crates/dozer-app/src/app/layout.rs`)重算并给存活
PTY 发 SIGWINCH,否则 PTY 行数与视觉高度不一致。条的高度要纳入这条既有几何
公式,而不是另写一份。

## 3. 发送动作补落库(改 Phase 2)

- 文件树右键"添加到 Agent 上下文"改为:**先 `AddContextItem`,再 `term_paste`**
  (文本模板与 Phase 2 完全一致)。
- **落库失败仍然粘贴**,并在界面提示一次"已发送到终端,但未能记录到上下文列表",
  不因存储失败让 human 的操作白做。
- 目录项 `entity_kind = "dir"`、文件项 `"file"`,`entity_ref` 用与"复制相对路径"
  同一个路径计算逻辑。
- 预览选区"发送给 Agent"**保持不变**,不落库。

## 4. 修改历史弹窗(独立原生窗口)

按 `CLAUDE.md` 关键裁决,新浮层默认走 `platform/overlay_window.rs` 的独立原生
窗口机制(天然叠在 webview 之上,不需要显式隐藏 webview 的逻辑)。非 mac 回退
路径沿用 `docs/superpowers/specs` 中"弹窗独立窗口通用化第二份 spec 暂停"的结论,
不在本期处理。

**列表**:按时间倒序,每条显示时间、`target_path`、`summary`、署名(`actor`)。
顶部文件/目录过滤框:从条上"修改历史"按钮打开时不预过滤;从某个上下文项打开
时预过滤到该项。数据来自 `ListFileEditHistory`,`limit` 默认 200。

**每条记录四个操作**:

- **Diff**:用该条的 `old_text`/`new_text` 通过现有 CodeMirror diff 宿主展示
  前后对比(`file_history`/`git_log` 已在用的那一套,不新造渲染)。
- **Locate**:在 Preview 打开该文件,发 `Reveal`/`Select` 命令到该条**修改后**的
  坐标区间。目标文件已不存在则明确提示,不静默无反应。坐标只对"该次修改之后
  文件未再变化"的情形精确;文件之后又被改过时定位到坐标仍可能偏移——这是已知
  限制,UI 不承诺精确,不做重新解析。
- **Revert**:反向调用 `apply_precise_edit`——`expected_text` = 该条 `new_text`,
  `new_text` = 该条 `old_text`,坐标用该条修改后的区间。走同一套 Conflict
  Detection:磁盘在那次修改之后又被改过,得到 `MutationConflict`,弹窗显示
  "文件已在此后被修改,无法撤销",**不覆盖**。脏 tab 拒绝的结果同样原样提示。
  - **署名**:撤销也写一条新历史,`actor = "dozer"`(human 经 GUI 触发,不冒充
    agent)。`session_id` 使用 GUI 侧生成的稳定标识,不复用 agent 会话 id。
    `Request::ApplyPreciseEdit` 本来就带 `actor`/`session_id` 字段(MCP 路径填
    当前 agent),GUI 直接填 `"dozer"` 即可,协议与 dozerd 逻辑不需要改。
  - `summary` 自动填 `撤销:{原 summary}`。
- **Ask Agent**:用 `term_paste` 把下列文本写进当前项目激活的 agent 终端,不自动
  回车(与 Phase 2 同一原语):

  ```
  关于 {relative_path}:{start_line}:{start_col}-{end_line}:{end_col} 的这次修改({summary}):

  修改前:
  {old_text}

  修改后:
  {new_text}
  ```

  无可见/激活终端时该按钮置灰(复用 Phase 2 的 `terminal_visible()`/
  `ssh_terminal_visible()` 门槛)。不使用代码围栏,理由同 Phase 2。

## 非目标(Phase 3)

- 权限位 UI 与校验(§11)——仅留列占位。
- Todo/会话/SSH 主机/数据库连接等其他实体类型——仅数据模型留口子。
- Review Mode 自动弹出(§20)、Accept 操作。
- 预览选区入列表。
- dozerd → GUI 的推送或轮询刷新。
- 非 mac 平台的独立窗口回退实现。
- 新增给 agent 使用的 MCP 工具。
- 表格/图片 Adapter 相关历史(Phase 4)。

## 测试

- **`agent_context_items` 存储**:添加、重复添加幂等(行数不变、`created_ms` 不变)、
  移除、移除不存在的 id 视为成功、按项目隔离(A 的项不出现在 B)、未知
  `entity_kind` 读出后原样保留。
- **路径校验**:`..` 越界、绝对路径越界被拒;含空格/中文路径可正常添加;
  磁盘不存在的路径允许添加。
- **`ListFileEditHistory`**:倒序;`path_filter` 等于文件路径命中该文件;等于目录
  命中其下所有文件且**不**误命中同前缀的兄弟(如过滤 `src/a` 不命中 `src/ab/x`);
  `None` 返回全部;`limit` 生效。
- **上下文条**:折叠/展开后 `terminal_pane_pixel_size` 与 PTY 网格行数一致;缺失项
  灰显;项目切换后各项目条目与折叠态互不串;异步应答按 `project_id` 路由到
  正确项目(在项目 A 发起、切到 B 后应答到达,不写进 B)。
- **发送动作落库**:成功路径既落库又粘贴;`AddContextItem` 失败时仍粘贴且给出
  提示;重复右键添加不产生重复项。
- **历史弹窗**:
  - Diff 展示的 old/new 与记录一致。
  - Locate 目标文件缺失时给出提示。
  - Revert 成功路径:文件内容还原、新增一条 `actor = "dozer"` 的历史;
    磁盘已变时得到冲突提示且文件未被改动;目标为脏 tab 时被拒绝。
  - Ask Agent 文本模板对含反引号/多行/中文的 `old_text`/`new_text` 不丢内容;
    无终端时按钮置灰。
  - 从上下文项打开时预过滤生效,从条上按钮打开时无预过滤。
- **多项目场景**:在项目 A 触发 Ask Agent/添加,文本写进项目 A 激活的终端
  (与 Phase 2 同款集成测试锁定)。
