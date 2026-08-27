# 会话总结生成管线(关闭时触发,agent 经 dozer-mcp 写回)

**状态:已批准(brainstorming 会话,2026-08-27)**

## 背景

`dozer-app` 的 agent 卡片(`workspace.rs::agent_card`)用 `tab.last_activity`
兜底展示"当前工作内容"——这个字段不是摘要,是从 transcript JSONL 尾部扫出
的**用户最后一句原话**,硬截 60 字符(`transcript.rs::truncate_activity`),
没有语义压缩,长句直接截断加省略号,信息量低。

本设计的直接动机是给"已关闭的 session"生成一份真正有信息量的总结,但触发点
必须精确定在**关闭 tab 的那一刻**,理由是消除了一个真实的时序矛盾:如果
session 还活着,agent 本来就能被直接使用,不存在"总结"这个独立动作的必要性
(用户随时能看当前状态);如果 session 已经彻底结束(PTY/子进程已死),就没有
活的 agent 可用,总结只能退化成读盘算法或另起一个全新的 agent 进程(后者是
"headless 一次性 agent 调用机制"这块被显式搁置的独立地基,见"非目标")。
"关闭"这个动作是两者之间唯一的窗口:PTY 还活着、agent 还能被注入一次任务,
之后再真正杀掉。

本设计同时验证一条此前从未走通的路径:**Dozer GUI 主动往一个正在运行的
agent 会话注入任务 → agent 调用 `dozer-mcp` 的写工具 → 结果落回
`dozerd`**。这是 `dozer-mcp` 第一次拥有写能力(此前只有一个只读 tool
`get_preview_context`),也是未来"内置 agent"系列工作([[dozer-builtin-agent-work-paused]]
风格的能力)可以照抄的第一个真实先例。

**已确认的现有机制,本设计直接复用,不重新发明**:

- `Client::write(id, data)` → `Request::Write` → `Session::write` 往 PTY
  写字节,已被"自动键入初始命令""派发 todo 任务文本"两处生产代码使用
  (`workspace.rs` 约 1580-1594 行)。
- `dozerd::registry::kill` 杀子进程后**不会**把 session 从 registry 移除
  (死会话仍留着、保留 scrollback)——本设计只是延后"何时调用
  kill",不改变 kill 本身的语义。
- `AcceptanceStore`/`TranscriptStore` 的存储模式:单文件 `dozer.db`、
  每个 store 各自持有 `Mutex<Connection>`、`CREATE TABLE IF NOT EXISTS`
  幂等迁移。新表照抄这个模式。
- `ensure_hook_installed`(`workspace.rs:3803`)在新建 agent session 时
  静默、幂等地把 `dozer-hook` 注册进目标 agent 的配置——`dozer-mcp` 现在
  **没有**对应的自动注册,只能靠用户手动跑 `dozer-mcp install <agent>`
  (`crates/dozer-mcp/src/install.rs`),这是本设计必须补的缺口,否则
  agent 在会话里根本看不到新写工具。
- 对话摄取管线([[dozer-conversation-ingestion-pipeline]])已把 Claude/
  CodeBuddy/OpenCode 三家的对话增量落进 `dozerd` 的 `conversation_turns`
  表,本设计的启发式兜底直接读这张表,不需要重新解析 transcript 文件。
- **V8agent(用户自研 agent,以后新功能默认都要覆盖它)已经是第四家被
  真正打通的 agent**,接入方式和前三家都不一样,且成本更低:①对话摄取
  不走目录扫描,是 v8agent-cli 直连 socket 上报 `transcript_path`,命中
  `dozerd` 现有的 hook 快速通道(`extract_transcript_path()`),
  `conversation_turns` 数据是全的(2026-08-24 的
  `2026-08-24-v8agent-integration-design.md` 已落地);②v8agent-cli
  **硬编码**检测到 `DOZER_SESSION_ID` 环境变量(dozerd 本来就给所有
  session 子进程注入)时自动挂载 `dozer-mcp serve` 作为 MCP
  server(`v8agent-core/src/mcp/mod.rs` + `v8agent-cli/src/main.rs:67`),
  不需要 Claude/CodeBuddy/Codex/OpenCode 那套"改配置文件注册"的流程。

**已确认不存在、本设计不新造的东西**:

- `dozerd` 生产代码里**完全没有**定时器/延迟任务基础设施(全仓搜索
  `tokio::time`/`Instant`/`interval` 只在 `#[cfg(test)]` 里出现)。本设计
  需要引入第一个"N 秒后自动执行"的机制,范围限定在这一个用例,不做成通用
  scheduler。
- 没有任何"headless 起一个 agent CLI 处理单次 prompt"的先例或机制。本设计
  **不需要**这个能力(全程复用已存活的 PTY),但下一个依赖它的功能(项目
  修复时批量补总结)会需要——不在这份 spec 范围内。

## 目标 / 非目标

**目标**:

1. `dozerd` 新增 `session_summaries` 表(按 `session_id` 索引,持久化,
   可回看历史),记录标题+摘要两级内容,以及"是否真的由 agent 生成"的
   状态字段。
2. `dozer-core::protocol` 新增写入/查询该表的 Request/Reply,
   `dozer-client` 加对应方法——查询能力这次一并做好,后续"对话面板改版"
   (session 列表/详情,另一份独立 spec)直接复用,不需要再动协议层。
3. `dozer-mcp` 新增第一个**写**工具,供 agent 提交总结;同时补上自动注册
   (仿 `ensure_hook_installed` 模式),在支持的 agent 类型下新建 session
   时静默调用 `dozer-mcp install`,不要求用户手动操作。
4. `dozer-app` 关闭 tab(×)的行为改造:UI 侧仍然立即移除该 tab(用户体感
   不变,不阻塞);仅当 `tab.agent` 属于对话摄取管线已覆盖的四家
   (Claude/CodeBuddy/OpenCode/V8agent)且走 daemon 后端时,改发一个新的
   "总结后关闭"请求而不是直接 `Kill`；其余情况(纯 shell/`AgentKind::
   Unknown`/SSH 后端/Codex/Kilo)维持现状直接 `Kill`,不纳入本次范围。
5. `dozerd` 收到"总结后关闭"请求后:往该 session 的 PTY 注入一段固定总结
   prompt,进入等待;agent 调写工具落表则视为成功(status=`ai_generated`),
   立即真正 kill;若在超时窗口内没有等到,`dozerd` 自己从
   `conversation_turns` 算一份启发式兜底(status=`heuristic_fallback`)
   写表,再真正 kill。两条路径最终都会调用现有的 `kill`,只是"什么时候
   调"不同,不改变 kill 本身的行为。
6. 全部改动落地后:`cargo build`(全 workspace)、
   `cargo test -p dozerd -p dozer-app -p dozer-core -p dozer-client -p
   dozer-mcp`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt
   -- --check` 全绿;真机验证走人工(见"测试策略")。

**非目标**:

- **不做"翻看早已结束、和当前操作无关的历史 session"的补总结能力**——
  这需要 headless 一次性 agent 调用机制,是被显式搁置的独立地基
  (用户原话:"Headless 一次性 agent 调用机制先搁置,后续我会找到合适的
  功能点来落实"),不在这份 spec 里造。这也意味着**在本设计落地前已经
  关闭的 session、或本设计覆盖范围外关闭的 session(超时兜底都没数据的
  情况),永远不会补上总结**,这是已知、接受的限制,留给后续依赖 headless
  机制的子项目解决。
- **不在 `agent_card`/`last_activity` 上展示总结**——总结产出的那一刻,
  对应的 tab 已经从 `ws.tabs` 里移除,不可能出现在活跃卡片列表里。总结的
  展示方是"对话面板改版"这份独立 spec,本设计只负责产出与持久化。
- **不覆盖 Codex/Kilo/纯 shell/SSH 会话**——Codex 的 transcript 解析器
  现状是刻意的"恒返回空"(诚实设计,非缺口,这次不动);Kilo 目前既没有
  真实 hook 上报也没有对话摄取,是独立的已知缺口,不在本次范围;纯
  shell/`AgentKind::Unknown` 没有 agent 可注入 prompt;SSH 后端本来就
  不走 `registry.kill` 这条清理路径。这四类维持现状直接 `Kill`,不产出
  总结记录。**V8agent 不在这个排除列表里**——见"背景"一节,它的对话摄取
  与 MCP 挂载都已经是打通状态,且用户明确要求以后新功能默认覆盖它。
- **不做设置面板/"默认 agent"配置**——讨论中出现过"原 agent 无法工作时
  fallback 到默认 agent"的想法,调研确认 Dozer 目前连设置面板本体都不
  存在(设置齿轮是纯视觉占位)。本设计的失败路径统一走启发式兜底,不引入
  任何"换一个 agent 重试"的逻辑。
- **不引入通用 scheduler/延迟任务框架**——超时机制只服务于这一个用例
  (轮询 `session_summaries` 表判断是否已写入,而不是搭建通用的定时任务
  系统),细节见"错误处理"。
- **不做"项目新建/修复时批量补总结"的兜底扫描**——这是用户已确认的下一
  个子项目,依赖 headless 机制,且 `extensions/project.rs` 不应该直接
  耦合会话总结逻辑(用户定的原则:项目生命周期动作应该通过消息广播,让
  关心的 extension 自行订阅,不是硬编码调用)。本设计不涉及 `project.rs`
  的任何改动。

## 架构与数据流

### 数据模型(`dozerd`)

新增 `crates/dozerd/src/session_summary.rs`,结构照抄 `acceptance.rs`:

```rust
pub struct SessionSummaryStore {
    conn: Mutex<Connection>,
}
```

```sql
CREATE TABLE IF NOT EXISTS session_summaries (
    session_id  TEXT PRIMARY KEY,
    agent_kind  TEXT NOT NULL,
    title       TEXT NOT NULL,
    summary     TEXT NOT NULL,
    status      TEXT NOT NULL,   -- 'ai_generated' | 'heuristic_fallback'
    created_ts_ms INTEGER NOT NULL
);
```

`main.rs` 里仿 `AcceptanceStore::open`/`TranscriptStore::open` 的既有写法,
用同一个 `state_dir().join("dozer.db")` 文件打开独立连接,`Arc` 传入
`serve()`。

### 关闭流程改造(`dozer-app`)

`workspace.rs::close_tab` 分支改造:

```rust
let should_summarize = tab.alive
    && backend == Backend::Daemon
    && matches!(tab.agent, AgentKind::Claude | AgentKind::Codebuddy
        | AgentKind::Opencode | AgentKind::V8agent);

if should_summarize {
    io.handle.spawn(async move { client.close_with_summary(&id).await });
} else {
    io.handle.spawn(async move { client.kill(&id).await }); // 现状不变
}
```

`close_with_summary` 是新的 `dozer-client::Client` 方法,发送新协议消息后
**不等待**结果(与现有 `kill` 调用一样,失败只 `warn!`,不阻塞 UI——这本来
就是既有惯例,`close_tab` 本身不 await 拿到 kill 完成)。tab 从 `ws.tabs`
里移除的时机不变,仍是点击 × 后立即发生,`dozerd` 侧发生什么用户不感知。

### `dozerd` 侧处理

`Request::CloseWithSummary { session_id }` 处理器:

1. 查 `SessionRegistry` 拿到该 session 的 `AgentKind`(用于选择 prompt
   措辞/记录 `agent_kind` 列)。
2. 调用 `Session::write` 往 PTY 注入固定总结 prompt(内容大意:"请总结你
   在本次会话中完成的工作,给出一个简短标题和一段摘要,然后调用
   `dozer-mcp` 的总结提交工具交回,不需要征求确认"——具体措辞留实现阶段
   打磨,不同 agent 的 slash 命令/工具调用习惯可能需要微调)。
3. 立即返回 `Reply::Ok`(不阻塞调用方,`close_with_summary` 本来就不等)。
4. `tokio::spawn` 一个后台任务:每 2 秒轮询一次
   `session_summaries` 表是否已出现该 `session_id` 的行:
   - 出现了 → 直接 `registry.kill(session_id)`,任务结束。
   - 轮询到超时(**60 秒**,可调,无强约束理由,取"够 agent 想清楚
     一次,又不至于让死会话占资源太久"的经验值)仍未出现 → 调用启发式
     兜底(见下)写一行,再 `registry.kill(session_id)`。

轮询而非事件通知:`dozerd` 目前没有任何 pub/sub/`Notify` 基础设施,而
本用例时间尺度是秒级、数量级是"同时几个 session 在关闭",2 秒轮询的开销
可忽略,不值得为此引入新的并发原语。

### 启发式兜底算法

```sql
SELECT content FROM conversation_turns
WHERE session_id = ?1 AND role = 'human'
ORDER BY turn_index ASC;
```

- `title` = 第一条人类回合内容,截到一个固定长度(与 `last_activity` 的
  60 字符惯例保持一致,超长加 `…`)。
- `summary` = 全部人类回合内容按顺序拼接(每条一行),不做语义压缩——
  这是"没有 agent 参与"的降级形态,`status=heuristic_fallback` 明确标记
  低质量,消费方(对话面板)可以据此调整展示(比如加个"未经 AI 总结"标记,
  留给那份 spec 决定)。
- 若查询结果为空(该 session 完全没有已摄取的对话数据——常见于
  `dozer-mcp` 未安装导致 agent 没来得及产生任何摄取触发点,或本身是极短
  会话):`title`/`summary` 都写成固定占位文案(如"(无对话记录)"),仍然
  落一行,不跳过——保证"每个走过这条关闭路径的 session 都有一行记录"
  这个不变量,消费方不用处理"有的 session 干脆没有 summary 行"这种
  特例。

### 协议扩展(`dozer-core::protocol`)

```rust
// Request
CloseWithSummary { session_id: String },
RecordSessionSummary { session_id: String, title: String, summary: String },
GetSessionSummary { session_id: String },

// Reply
Ok,  // 复用现有 variant,CloseWithSummary/RecordSessionSummary 共用
SessionSummary { summary: Option<SessionSummaryPayload> },
```

```rust
pub struct SessionSummaryPayload {
    pub session_id: String,
    pub agent_kind: AgentKind,
    pub title: String,
    pub summary: String,
    pub status: SummaryStatus,   // enum: AiGenerated | HeuristicFallback
    pub created_ts_ms: u64,
}
```

`dozer-client::Client` 新增 `close_with_summary(id)`、
`record_session_summary(id, title, summary)`、
`get_session_summary(id)` 三个方法,内部走既有 UDS 请求/响应模式,风格
对齐 `list_conversations`/`get_conversation_turns` 那批已有方法。

`RecordSessionSummary` 的 `session_id` 由调用方(`dozer-mcp`)显式传入,
不在协议层做权限校验——和现有 `Write`/`HookEvent` 一样信任本机调用方,
`dozerd` 只做"这个 session_id 是否存在于 registry"的存在性检查,不存在
则 `Reply::Error`。

### `dozer-mcp` 新写工具与自动注册

新增 `#[tool]` 方法(暂定名 `submit_session_summary`),入参 `title:
String`、`summary: String`,内部逻辑与现有 `get_preview_context` 一样从
`DOZER_SESSION_ID` 环境变量解析当前 session_id,调
`Client::record_session_summary` 后返回确认文本。工具描述(提供给 agent
看的 tool description)需要清楚写明"仅在被要求总结当前会话时调用一次",
避免 agent 在其他场景误用。

自动注册:`dozer-mcp/src/install.rs` 补一个 `run_at_with_exe` 变体(现状
`install.rs` 内部靠 `current_exe()` 拿自身路径,与 `dozer-hook` 曾经踩过
的"跨进程拿错二进制"是同一类坑,必须显式传入 sibling 路径,不能偷懒复用
`current_exe()`)。`dozer-app/src/workspace.rs::ensure_hook_installed`
旁边新增 `ensure_mcp_installed(agent)`,同一个调用点(`spawn_new_tab`
内,`PickerLaunch::Agent(Some(agent))` 分支)顺带调用,仅当
`dozer_mcp::install::config_path_for(agent)` 返回 `Some`(即 Claude/
CodeBuddy/Codex/OpenCode 四家)时才执行,幂等、静默、失败只
`tracing::warn!`——完全复用 hook 那一套错误处理哲学,不因为 mcp 注册失败
阻塞用户开会话。

**V8agent 走完全不同的路,`ensure_mcp_installed` 对它直接返回
`None`/no-op**(与 `ensure_hook_installed` 对 V8agent 现状已经是 `None`
同理,不是遗漏):v8agent-cli 自己硬编码检测 `DOZER_SESSION_ID` 后自动
挂载 `dozer-mcp serve` 作为 MCP server(`v8agent-core/src/mcp/mod.rs`,
`v8agent-cli/src/main.rs:67`),不读任何配置文件,dozer-app 这边不需要写
任何东西。唯一的隐含前置条件是 `dozer-mcp` 这个二进制得能被 v8agent-cli
用裸命令名 `["serve"]`(无绝对路径)启动子进程时解析到——即需要在 PATH
上可找到,实现阶段用真机验证一次即可,不是需要设计决策的问题。

## 错误处理

- **agent 忽略/没理解注入的 prompt**:60 秒超时兜底覆盖,不会无限等待。
- **`dozer-mcp` 未注册成功**(自动注册失败、或用户环境有旧配置冲突):
  agent 压根看不到写工具,等价于"没响应",走超时兜底,不特殊处理。
- **agent 调写工具但内容为空/异常长**:`dozer-mcp` 侧对 `title`/
  `summary` 做长度上限校验(如 title ≤ 200 字符、summary ≤ 8000
  字符),超出截断,不拒绝整个请求(拒绝会导致这次总结彻底丢失,截断
  至少保留部分信息)。
- **同一 session 收到两次总结提交**(agent 重试/网络抖动):
  `INSERT OR REPLACE`(主键 `session_id`),后到的覆盖先到的,轮询任务
  第一次查到即结束,不会出现"两条记录"的情况。
- **轮询期间 `dozerd` 重启**:后台轮询任务是内存态,重启即丢失——重启后
  该 session 会保持"已发 CloseWithSummary 但从未真正被 kill"的状态,
  子进程可能残留。这是已知限制,理由是 `dozerd` 重启在现有架构里本就
  是低频事件(P1b 的 `session_survives_client_disconnect` 验证的是"客户端
  断开"而非"daemon 自身重启"),而给这一个用例单独做持久化任务队列的
  代价远超收益;可接受的后续改进方向是 `dozerd` 启动时扫描"存在
  `alive` 子进程但没有对应 `session_summaries` 行且已超过合理时长"的
  session 直接兜底 kill,但不在本次范围内实现。
- **`conversation_turns` 里没有该 session 数据**(见上文启发式算法的
  空结果分支):写占位文案而非跳过,保持"每个 session 都有一行"的不变量。
- **`title`/`summary` 写入 sqlite 失败**(磁盘满/权限问题等):
  `tracing::error!` 记日志,不重试(与仓库里"存储失败不做 backoff"的既有
  哲学一致,见 [[dozer-conversation-ingestion-pipeline]] 的同类决定),
  该 session 直接进入"无总结"的永久状态,`registry.kill` 仍会执行(不能
  因为总结失败就不清理进程)。

## 测试策略

1. `dozerd`:`SessionSummaryStore` 单测——写入/读取、`INSERT OR REPLACE`
   覆盖行为、`status` 字段正确性。
2. `dozerd`:启发式兜底算法单测——构造 `conversation_turns` fixture(含
   多条人类回合、含空结果场景),断言 `title`/`summary` 拼接逻辑与占位
   文案分支。
3. `dozerd`:`CloseWithSummary` 处理器集成测试——mock/构造一个真实
   session,断言 prompt 确实被写入 PTY(可通过读 session 的输出缓冲验证
   注入内容出现);断言轮询到 `RecordSessionSummary` 到达后立即 kill(不
   等满 60 秒,测试里可以把超时值做成可配置参数以缩短测试时间);断言
   超时分支触发启发式兜底并 kill。
4. `dozer-mcp`:新写工具单测,仿 `get_preview_context` 现有测试模式——
   验证 `DOZER_SESSION_ID` 解析、长度校验截断行为。
5. `dozer-client`:对真 daemon 的协议往返测试(`tests/
   against_real_daemon.rs` 同款模式)覆盖 `close_with_summary`/
   `record_session_summary`/`get_session_summary` 三个新方法。
6. `cargo build`(全 workspace)、`cargo test -p dozerd -p dozer-app -p
   dozer-core -p dozer-client -p dozer-mcp`、`cargo clippy --all-targets
   -- -D warnings`、`cargo fmt -- --check`。
7. **真机验证(单测覆盖不到端到端体验)**:
   - 已手动/自动注册好 `dozer-mcp` 的真实 Claude Code 会话:正常对话后
     点 × 关闭,确认 `session_summaries` 表出现一行 `status=ai_generated`
     且内容合理。
   - 故意不装 `dozer-mcp`(或断网/让 agent 不响应)的会话:关闭后确认
     60 秒左右出现 `status=heuristic_fallback` 的兜底行,内容是拼接的
     人类发言。
   - 纯 shell tab(未键入任何 agent CLI):关闭行为与改造前一致(直接
     kill,不产生总结行、不注入任何文本)。
   - Codex 会话:关闭行为与改造前一致(直接 kill,本次范围排除)。
   - V8agent 会话:不需要任何手动安装步骤,正常对话后点 × 关闭,确认
     `session_summaries` 出现一行 `status=ai_generated`(验证
     v8agent-cli 的自动 MCP 挂载在真实 Dozer 会话环境里确实生效,包括
     `dozer-mcp` 二进制能被裸命令名解析到);再故意让 `dozer-mcp` 从
     PATH 移除后重复一次,确认走 `heuristic_fallback` 且内容非空。

## 排期备注

建议实现顺序:先落 `SessionSummaryStore` 表结构 + 协议扩展 +
`dozer-client` 方法(纯后端,可独立验证);再落 `dozer-mcp` 新写工具 +
`ensure_mcp_installed` 自动注册(可用现成的 `get_preview_context` 手动
调用验证协议打通,不依赖关闭流程);最后落 `dozerd` 的
`CloseWithSummary` 处理器(注入 prompt + 轮询 + 超时兜底)与
`dozer-app` 侧 `close_tab` 分支改造,这两块耦合最紧,建议一起验证。
真机验证放在最后,需要真实网络环境和已安装的 agent CLI,不是纯
headless 能覆盖的部分。
