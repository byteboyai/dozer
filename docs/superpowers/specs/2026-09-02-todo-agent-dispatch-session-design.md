# Todo 任务派发 agent + 会话关联 + 详情弹窗双向对话(headless 处理)

**状态:已批准(brainstorming 会话,2026-09-02)**

## 背景

**现状:"派发"是往活着的 PTY 写字节,不是任务分派**

`crates/dozer-app/src/extensions/todo.rs:2584-2646`(`todo_dispatch_overlay`)
只能列出当前存活的 agent tab(`ws.tabs`/`ws.ssh_tabs`,过滤 `alive`),选中后
`Message::Todo(todo::Message::DispatchToExisting(idx, session_id))` 一路走到
`crates/dozer-app/src/workspace.rs:971-998`
(`dispatch_todo_to_existing`)——本质是把任务文本拼上 `\n` 当作"人类在终端
里敲字回车",通过 `Client::write` 写进那个会话的 PTY
(`crates/dozer-client/src/lib.rs:102-112`)。没有任何结构化的"任务分派"
消息,也没有 agent 侧的接收/确认动作。"派发到新会话"这条路径已在
2026-08-17 被显式砍掉(`todo.rs:2578-2583` 注释),现在只能派给已经开着的
tab。

`TodoInfo`(`crates/dozer-core/src/protocol.rs:219-242`)已经有
`dispatch_session_id: Option<String>` / `dispatch_at_ms: Option<u64>`
两个字段,由 `crates/dozerd/src/todo.rs:224-234`
(`TodoStore::record_dispatch`)以 `UPDATE ... SET dispatch_session_id = ?1`
的方式写入——单指针语义,重新派发直接覆盖旧值,没有历史记录。本设计**保留
这个"只留最新一次"的语义**(brainstorming 阶段已确认,不引入派发历史表)。

**现状:session_id 与 conversation_id 是两个不同的 id 空间**

`session_id` 是 PTY/tab 的活体身份,由 `crates/dozerd/src/session.rs:73-79`
在 spawn 子进程前生成并通过 `DOZER_SESSION_ID` 环境变量注入(agent 内的
`dozer-hook`/`dozer-mcp`/`v8agent-cli` 都读这个变量识别自己属于哪个
session,`crates/dozer-mcp/src/server.rs:15,57-58,246-250`)。
`conversation_id` 是文件摄取管线(`crates/dozerd/src/transcripts/mod.rs`)
按 transcript 文件生成的身份,两者只靠
`session_summaries.conversation_id` 这一列做弱关联
(`crates/dozerd/src/session_summary.rs:74-89`,该列是后补迁移的)。

`conversation_turns` 表的读路径**强依赖** `conversations` 表存在对应行:
`TranscriptStore::get_conversation_turns`
(`crates/dozerd/src/transcripts/mod.rs:275-290`)是
`conversation_turns t JOIN conversations c ON c.conversation_id =
t.conversation_id` 的内连接,只在 `conversation_turns` 里插行、不在
`conversations` 里插对应行的话,现成的读接口会静默返回空列表。
`conversations` 表(`transcripts/mod.rs:65-76`)的 `file_path` 列是
`NOT NULL`——这是为"扫描磁盘 transcript 文件"设计的,本设计产生的会话没有
真实 transcript 文件,插入时需要一个占位值。

**现状:headless 一次性 agent 调用已有先例,但只服务单一窄用途**

`crates/dozerd/src/headless_agent.rs` 是本仓库第一个"不摸 PTY,起一次 agent
CLI 处理单个 prompt 后退出"的机制,唯一调用方是
`crates/dozerd/src/session_summary_backfill.rs:109`——用于给已摄取的会话
补生成标题+摘要。特点:
- 固定指令模板"总结这段对话",要求模型输出**必须**用
  `<<<DOZER_SUMMARY_JSON>>>...<<<END_DOZER_SUMMARY_JSON>>>` 包裹的 JSON
  (`extract_summary`,`headless_agent.rs:143-158`),不是自由文本。
- 只有 4 家 CLI 有适配器(`bare_program_name`,`headless_agent.rs:90-98`):
  Claude、Codebuddy、Opencode、V8agent;Codex/Kilo/Unknown 返回
  `HeadlessError::Unsupported`。
- 只有 Codebuddy 加了跳过权限确认的参数(`-y`),其余三家没加——因为现有
  唯一用途是"输出一段文字",不需要真正的文件/命令权限。
- **没有设置 `current_dir`**——总结不需要碰项目文件,不需要在项目目录下跑。
- 超时常量 90 秒(`HEADLESS_TIMEOUT`),对总结这种任务够用。

本设计需要一个新的调用变体(真正让 agent 动手做任务,不是输出摘要),不能
直接复用 `summarize_headless` 的分隔符协议、权限参数、`current_dir`
缺失、90 秒超时这几点——细节见下方"headless 任务处理调用"一节。

**现状:dozerd 完全没有周期性后台任务基础设施**

全仓搜索 `tokio::time::interval`/`tokio::time::sleep` 循环,只在
`#[cfg(test)]` 里出现过一次性 `tokio::time::timeout`(headless 超时用)。
本设计里的"轮询开关"要求的**周期性**扫描是 dozerd 第一个这样的机制,范围
限定在这一个用例,不做成通用 scheduler。

**现状:agent 已经有权限修改 Todo 状态,不需要新工具**

`dozer-mcp` 已经暴露 `mcp__dozer__toggle_todo` / `mcp__dozer__edit_todo_text`
/ `mcp__dozer__add_todo` / `mcp__dozer__list_todos`
这几个写工具(见系统里已挂载的 MCP 工具列表)。本设计的 headless 一次性
进程只要正确注入 `DOZER_SESSION_ID` 并且该 agent 已经把 `dozer-mcp`
接进配置(现有 `ensure_hook_installed` 类似的自动注册机制覆盖的四家 CLI),
agent 自己在完成任务后调用 `toggle_todo` 标记完成即可,不需要为"标记完成"
专门再设计一条新协议。

## 目标 / 非目标

**目标**:

1. Todo 卡片"派发"改为纯粹的"指派"动作——记录 `assigned_agent`
   (`AgentKind`),不要求存在活着的 PTY 会话,不触发任何执行、不写 PTY。
2. `todo_categories` 增加按分类维度的"自动处理"轮询开关;dozerd 后台按
   固定间隔扫描开启该开关的分类下"待处理"的任务,逐个触发 headless 处理。
3. 新增 headless 任务处理调用:喂入任务文本 + 该任务已有的会话历史 + 人类
   最新指令,让 agent 在项目目录下真正读写文件、跑命令,产出的内容记成新
   的会话回合,和该任务关联起来。
4. Todo 卡片新增"详情"按钮,弹窗展示任务信息 + 关联会话的回合列表(复用
   会话面板现成的回合渲染),底部有回复输入框 + "处理"按钮,人类回复后可
   立即触发一次 headless 处理(不必等轮询)。
5. 会话面板的会话列表行,如果关联着某个 task,展示一个"关联任务"标签。
6. 完全替换掉旧的"派发到现有活着的 tab"UI/流程,不保留双轨。

**非目标**:

- 不做派发历史(多次派发的完整轨迹)——`dispatch_session_id` 保持"只留
  最新一次"的覆盖语义,只是触发时机和写入内容变了。
- 不做"人类回复直接打断/中止正在跑的 headless 进程"这类实时控制能力
  (无人值守本来就是这次要接受的设计前提,brainstorming 阶段已确认接受
  这个风险边界)。
- 不新增"标记任务完成"的专用协议——复用 agent 已有的
  `mcp__dozer__toggle_todo`。
- 不做会话面板"点击关联任务标签跳转回 Todo 面板"的深链——v1 只展示文本
  标签。
- 不改动现有交互式 PTY 会话本身的任何行为(term_view、keymap、hook 事件
  上报等一律不动)。

## 数据模型变更

### `todos` 表(`crates/dozerd/src/todo.rs`)

新增一列:

```sql
ALTER TABLE todos ADD COLUMN assigned_agent TEXT;
```

存 `AgentKind::label()` 返回的字符串(`"claude"`/`"codebuddy"`/
`"opencode"`/`"v8agent"`;`Codex`/`Kilo`/`Unknown` 不可被指派,选择器里不
出现这三个选项)。指派动作只写这一列 + `dispatch_at_ms`,不碰
`dispatch_session_id`。

`dispatch_session_id` 的语义改写(字段名不变,更新周边注释):不再是"派发
到已存在会话时立刻写入的那个会话 id",而是**首次真正触发 headless 处理
时**由 dozerd 现铸的 `session_id`,之后每次处理复用同一个,不再变化(除非
任务被重新指派给不同 agent——重新指派后下一次处理会铸造新的
`session_id`,覆盖旧值,和现有"覆盖语义"一致)。

`TodoInfo`(`protocol.rs:219-242`)相应加 `assigned_agent:
Option<AgentKind>` 字段,协议序列化按现有 `AgentKind` 的 serde 实现走。

### `todo_categories` 表(`crates/dozerd/src/todo_category.rs`)

```sql
ALTER TABLE todo_categories ADD COLUMN auto_poll_enabled INTEGER NOT NULL DEFAULT 0;
```

`CategoryInfo`(`protocol.rs:247-256`)加 `auto_poll_enabled: bool`。

### `session_summaries` 表(`crates/dozerd/src/session_summary.rs`)

```sql
ALTER TABLE session_summaries ADD COLUMN task_id INTEGER;
```

铸造 `session_id` 处理某个任务时一并写入该任务的 `todos.id`,供会话面板
反查关联的 task(展示"关联任务"标签用)。这一列的迁移写法照抄
`conversation_id` 列已有的"`pragma_table_info` 探测 + `ALTER TABLE`"模式
(`session_summary.rs:93-108`)。

### `conversations` / `conversation_turns` 表(`crates/dozerd/src/transcripts/mod.rs`)

不改表结构,但**必须同时**给新铸造的 `session_id`(复用同一个字符串当
`conversation_id`)在 `conversations` 表插入一行占位记录——不插的话
`get_conversation_turns` 的内连接会对这个 `conversation_id` 永远返回空
列表(见"背景"一节的发现)。占位记录:

```sql
INSERT INTO conversations
  (conversation_id, agent_kind, dir, file_path, title, first_ts, last_ts, turn_count, parsed_offset, file_size_at_parse)
VALUES
  (?1, ?2, ?3 /* 项目目录 */, '' /* 占位:无真实 transcript 文件 */, ?4 /* 任务文本截断 */, ?5, ?5, 0, 0, 0);
```

每次处理成功后累加 `turn_count`、更新 `last_ts`。`conversation_turns` 插入
两条:一条 `role="human"`(人类最新指令/首次指派时的任务文本)、一条
`role="ai"`(headless 调用捕获的 stdout),`turn_index` 从该
`conversation_id` 现有最大值 +1 开始递增,`message_key` 用
`{conversation_id}:{turn_index}` 保证 `(conversation_id, message_key)`
主键不冲突。

### "待处理"判定:不新增标记字段

判定某任务是否需要处理,直接看它关联 `conversation_id` 下
`conversation_turns` 里 `turn_index` 最大的那一条的 `role`:

- 还没有任何回合(刚指派、从未处理过)→ 待处理。
- 最新一条 `role == "human"`(人类刚回复,agent 还没响应;或者上一次
  headless 调用失败/超时,没能写入 `role == "ai"` 的回合)→ 待处理。
- 最新一条 `role == "ai"` → 不需要处理,等下一次人类输入。

好处:不需要额外一个"待处理"布尔字段和真实数据保持同步,状态永远由回合
数据本身推导,不会出现"标记待处理但其实已经处理完"的不一致。

## headless 任务处理调用

新增 `crates/dozerd/src/headless_agent.rs::process_task_headless`,和
`summarize_headless` 并列、不复用其分隔符协议:

```rust
pub async fn process_task_headless(
    agent: AgentKind,
    project_dir: &Path,
    task_text: &str,
    prior_turns_text: &str,   // build_transcript_text 同款拼接手法,喂历史
    human_instruction: &str,  // 本次触发的人类指令(首次处理时=task_text)
) -> Result<String, HeadlessError>  // Ok(stdout 原文)
```

与 `summarize_headless` 的关键差异:

1. **`current_dir(project_dir)`**——`summarize_headless` 完全没设置这个,
   任务处理必须在项目目录下跑,agent 才能看到/编辑正确的文件。
2. **四家 CLI 都需要能真正执行工具调用而不卡在授权确认上**。Claude 已知
   要加 `--dangerously-skip-permissions`(`-p` 模式下的标准跳过参数);
   Codebuddy 现有 `-y` 直接复用。**Opencode 和 V8agent 现在的 `build_command`
   从未加过任何权限参数**(`headless_agent.rs:184-198`)——因为现有唯一
   用途"只输出一段摘要文字"从不触发过工具调用,没人验证过这两家在真正
   跑工具时是否会卡在确认上、卡住了该加什么参数。这两家的具体参数**留到
   实现计划阶段逐家实测核实**,不在这份设计里假设,避免凭空猜的参数名
   静默失效(表现为 headless 进程超时或卡死在无人能应答的确认提示上)。
3. **不要求 JSON 分隔符协议**——指令模板改成"这是任务描述 + 到目前为止的
   往来记录 + 我现在的指示,请继续处理这个任务",直接把完整 stdout 存成
   一条 `ai` 回合内容,不做 `extract_summary` 那样的结构化提取。
4. **超时常量独立**——`summarize_headless` 的 90 秒是给"读一遍对话写两句
   话"用的,任务处理可能要跑真正的编辑/构建,先定 10 分钟
   (`TASK_PROCESS_TIMEOUT = Duration::from_secs(600)`),后续按实测调整。
5. **注入 `DOZER_SESSION_ID`**——传入铸造的 `session_id`,让 agent 进程内的
   `dozer-mcp`(如果该 agent 已注册)能正确识别自己所属的 session/project,
   从而让 agent 自己调用 `toggle_todo` 之类工具时命中正确的项目。

`bare_program_name`/`resolve_binary_path`(`headless_agent.rs:88-139`)两个
辅助函数直接复用,不用改。

## 轮询器

dozerd 启动时 `tokio::spawn` 一个常驻后台任务(dozerd 里第一个周期性
scheduler,`crates/dozerd/src/task_poller.rs` 新文件):

- 固定间隔 30 秒(`POLL_INTERVAL = Duration::from_secs(30)`)。
- 每次醒来:`SELECT` 出 `auto_poll_enabled = 1` 的分类下、`assigned_agent
  IS NOT NULL`、未完成(`done = 0`)、按上一节判定逻辑算出"待处理"的任务。
- 维护一个进程内 `HashSet<i64>`(任务 id)记录"正在处理中",避免同一任务
  在上一次处理还没跑完时被下一轮轮询重复触发(处理耗时可能超过 30 秒的
  轮询间隔,尤其是接近 10 分钟超时上限的情况)。
- 逐个(不并发)调用 `process_task_headless`,成功则按上一节写入
  `conversations`/`conversation_turns`;失败/超时则记录一条 `ai` 角色的
  错误说明回合(内容包含 `HeadlessError` 的可读描述),避免这个任务在没有
  任何新人类输入的情况下被下一轮轮询无限重试——错误回合本身会让"待处理"
  判定翻转成"最新一条是 ai"(即使内容是错误说明),需要人类看到错误后主动
  在详情弹窗里重新回复才会再次触发处理。

## 派发流程变化

`todo_dispatch_overlay`(`todo.rs:2584-2646`,现列出活着的 tab)整体替换成
"选 agent 类型"——四个选项(Claude/CodeBuddy/OpenCode/V8agent)对应
`AgentKind::label()` 有 headless 适配器的那四种。`Message::Todo(todo::
Message::DispatchToExisting(idx, session_id))` 改成
`Message::Todo(todo::Message::AssignAgent(idx, AgentKind))`,`app.rs:4474-
4476` 的路由和 `workspace.rs:971-998`(`dispatch_todo_to_existing`,PTY
写入)一并删除,新逻辑只调 `record_todo_dispatch` 的替代 RPC(见下)写
`assigned_agent` + `dispatch_at_ms`,不碰任何 PTY/`Client::write`。

`todo.rs:450-455`(`task_title_for_session`,给 Agent 卡片"当前工作内容"
用的反查)需要跟着改:现在 `dispatch_session_id` 不再等价于"某个活着的
tab 的 session_id",Agent 卡片如果还要展示"当前在处理哪个任务",判定条件
要看该 tab 的 `session_id` 是否等于某任务已铸造的 `dispatch_session_id`
——这个反查逻辑本身不用大改,只是不能再假设"有值就等于这个 tab 正在跑"
(现在即便任务被指派了、也可能还没真正触发过 headless 处理,
`dispatch_session_id` 还是 `None`)。

## 协议层(`dozer-core::protocol` / `dozer-client` / `dozerd::server`)

新增/替换的 `Request`/`Reply`:

- `Request::AssignTodoAgent { id: i64, agent: AgentKind }` → 替换
  `RecordTodoDispatch`(旧的 `{ id, session_id }` 形态删除,PTY 派发路径
  整体下线)。dozerd 端只写 `assigned_agent` + `dispatch_at_ms`。
- `Request::SetCategoryAutoPoll { id: i64, enabled: bool }`。
- `Request::ProcessTodoNow { id: i64, human_reply: Option<String> }`——
  详情弹窗"处理"按钮用,`human_reply` 为 `None` 表示"首次触发,没有新增
  人类文本"(极端情况:指派后从未回复过就先点了处理)。dozerd 收到后
  **同步**跑一次 `process_task_headless`(不经过轮询器的
  `HashSet` 去重,但如果该任务 id 已经在轮询器的处理中集合里,直接返回
  "正在处理中"的错误,避免同一任务被并发跑两次)。
- `Request::GetTodoDetail { id: i64 }` → `Reply::TodoDetail { info:
  TodoInfo, turns: Vec<TurnRecord> }`——详情弹窗打开时一次性拿任务信息 +
  完整回合列表(内部就是查 `TodoInfo` + 调用现成的
  `get_conversation_turns(dispatch_session_id, -1, u32::MAX)`,
  `dispatch_session_id` 为 `None` 时 `turns` 返回空数组)。

`dozer-client` 对应加 `assign_todo_agent`/`set_category_auto_poll`/
`process_todo_now`/`get_todo_detail` 四个方法,模式照抄现有
`record_todo_dispatch`(`dozer-client/src/lib.rs:345`附近)。

## Todo 详情弹窗(`dozer-app` 新 UI)

Todo 卡片新增"详情"图标按钮,点击打开一个窗口级弹窗
(参照 `category_picker_popup` 一类弹窗的 `Length::Fill` + `padding`
定位手法,而不是像分类改名框那样的内联行编辑控件),内容:

- 顶部:任务文本、当前分类、`assigned_agent`(未指派则显示 agent 类型
  选择器,选中即调用 `AssignTodoAgent`)。
- 中部:回合列表,数据源是 `GetTodoDetail` 返回的 `turns`,渲染逻辑直接
  复用会话面板现成的回合渲染组件/函数(`workspace.rs:147-180` 状态结构 /
  `app.rs:7061-7072` 一带的渲染代码),不重新写一套气泡 UI。
- 底部:一个原生 `text_input` 回复框 + "处理"按钮。提交时:
  1. 乐观本地插入一条 `role="human"` 的回合到当前显示的列表(不等 RPC
     回来就先看到自己刚发的内容)。
  2. 发 `ProcessTodoNow { id, human_reply: Some(text) }`。
  3. RPC 返回后用服务端权威的 `turns` 刷新列表(替换掉乐观插入的那条,
     避免和服务端最终写入的 `turn_index`/`message_key` 不一致)。
  4. 处理中(RPC 未返回)期间按钮显示 loading 态、禁用重复提交——这次
     调用可能长达 10 分钟,弹窗需要能在等待期间正常关闭/切换到别的面板
     而不阻塞 UI(RPC 走异步 `handle.spawn`,现有 Todo 面板其它 RPC
     调用的标准写法)。

这个回复输入框、"处理"按钮的焦点态,**必须**按 CLAUDE.md 现有的原生输入
焦点路由规范接入 `main.rs` 的键盘路由闸门(参照本次会话早些时候修的
`category_rename_focused` 那个缺口——新增输入框必须同时补 getter 和
gate 里的条件,两者缺一都会导致按键漏到别处)。

## 会话面板改动

会话列表行渲染逻辑(`app.rs` 里读 `session_summaries` 数据渲染列表的那段)
读到 `task_id` 不为空时,在该行追加一个"关联任务:<task_text 截断>"的小
标签——`task_text` 需要额外查一次 `todos` 表按 id 取 `text`
字段(`task_id` 是纯粹的整数外键,`session_summaries` 表本身不冗余存任务
文本)。v1 不做点击跳转,纯展示。

## 错误处理

- `process_task_headless` 失败(spawn 失败/超时/该 `AgentKind` 没有
  headless 适配器):写一条 `role="ai"`、内容为可读错误描述的回合,不静默
  丢弃、不无限重试(见"轮询器"一节)。
- `ProcessTodoNow` 命中"该任务正在处理中"时返回明确错误,前端弹窗展示提示
  文案,不是通用错误态。
- 项目目录在处理时被删除/移动:`process_task_headless` 的 `current_dir`
  设置会导致 spawn 失败,归入上面第一类错误处理,不需要专门分支。

## 测试策略

- `headless_agent.rs`:新增 `process_task_headless` 的单测,复用现有
  "用真实存在的 `sh`/不存在的二进制名做确定性单测"手法
  (`run_and_extract` 已经是这么测的),覆盖 spawn 失败、超时两个分支;
  `current_dir` 设置可以用一个跑 `pwd` 的假命令断言真的切到了目标目录。
- `todo.rs`(dozerd):`assigned_agent` 列的读写单测,以及"待处理"判定逻辑
  (给定不同的 `conversation_turns` 末尾角色,断言判定结果)的单测。
- `transcripts/mod.rs`:补一个单测断言"只插 `conversation_turns` 不插
  `conversations` 时 `get_conversation_turns` 返回空"——把这次踩到的坑
  写成回归测试,防止以后有人在别处犯同样的错。
- `task_poller.rs`:用一个可注入的 fake `process_task_headless`(trait 或
  函数指针参数化)测试"正在处理中"去重逻辑,不需要真的起子进程。
- dozer-app 侧:详情弹窗的焦点路由按 `category_rename_focused` 同款模式,
  补 getter + main.rs 闸门条件后,人工验收时明确用截图/操作确认输入不会
  漏到终端(这条本身不是自动化测试能覆盖的,需要在计划的验收步骤里写
  明确的手动检查项)。
