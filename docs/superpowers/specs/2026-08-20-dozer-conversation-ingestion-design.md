# Agent 对话/用量摄取管线(落库进 dozerd)

**状态:已批准(brainstorming 会话,2026-08-20)**

## 背景

Dozer 现有"对话面板"(`crates/dozer-app/src/conversation.rs` + `transcript.rs`)
和"用量面板"(`crates/dozer-app/src/extensions/usage.rs`)完全是"每次打开/
刷新都直接读盘解析"的架构:`list_all_conversations` 扫描三个 agent 各自
的会话目录(`~/.claude/projects/*.jsonl`、`~/.codebuddy/projects/*.jsonl`、
`~/.dozer/agents/opencode/projects/*.jsonl`——OpenCode 本身不落盘 JSONL,
是 dozer-hook 代写成 Claude 形状),`transcript.rs` 按 `AgentKind` 分派两套
schema(Claude 形状 vs CodeBuddy 独立 schema,OpenCode/Kilo/Unknown 复用
Claude 形状)把整份文件解析成 `ReviewEntry::Human/AiTurn`;`usage.rs` 走同一
套文件发现,独立再解析一遍算 token 用量。`dozerd` 在这条路径上**零参与**:
它现有的 3 张表(`acceptances`/`bookmarks`/`projects`,`rusqlite` 单
`Mutex<Connection>`)都不涉及对话内容;hook 事件(`Request::HookEvent`)
只携带元数据(`session_id`、`transcript_path` 等),不含对话正文,且这条
状态(`SessionHandle` 里的 `agent_state`/`transcript_path`)只存在内存里,
daemon 重启即丢。

这个架构是"对话面板乱、不流畅"的根源(全量重读+重解析,无索引无缓存),
也是"agent 间共享记忆"这类后续功能完全无地基可用的原因——全仓库搜索确认
目前没有任何模块、spec 或计划涉及"跨会话/跨 agent 共享上下文"。

本设计是拆分后的**第一个子项目**:把摄取/解析/存储收拢进 `dozerd`,
用 sqlite 落库,面板改为纯查询。第二个子项目(基于此落地的"共享记忆",
先做跨会话再做跨 agent)是后续动机而非本次范围,不在这份 spec 里展开。

**参考调研**(brainstorming 会话内完成,详见对话记录,不重复贴引用):
调研了本机另外两个相关项目 Kooky(Swift 终端多路复用器)和 Orca
(TS/Electron 多 agent IDE)的同类实现。Kooky 完全不落库,transcript 只读
文件头渲染列表,hook 层刻意只传元数据摘要、放弃完整内容管道——设计目标
与本次相反,不构成参照。Orca 有两点值得借鉴的教训:(1)它的"展示用解析"
和"用量统计解析"是两套独立实现,长期重复维护,本设计通过统一的
`conversation_turns` 表 + 查询时聚合用量来规避这个问题;(2)它踩过
"agent fork/resume 时旧对话内容被复制进新 session 文件,导致用量重复
计入"的坑,用全局 `message.id` 去重表解决——本设计据此在用量聚合查询层
引入同等的去重机制(见下方"用量去重"一节)。

## 目标 / 非目标

**目标**:

1. 摄取/解析逻辑从 `dozer-app` 整体搬进 `dozerd`(新目录
   `crates/dozerd/src/transcripts/`),`dozer-app` 侧 `conversation.rs`/
   `transcript.rs` 删除,`usage.rs` 里的直读解析函数
   (`parse_usage`/`parse_claude_shaped_usage`/`parse_codebuddy_shaped_usage`)
   删除,面板改为通过 `dozer-client` 走 UDS 查 `dozerd`。**不保留
   直读文件的 fallback 分支**——`dozerd` 是 Dozer 生态的必需常驻进程,不存在
   "dozerd 没跑"的正常形态。
2. 新增两张 sqlite 表(复用 `acceptance.rs` 那套"`CREATE TABLE IF NOT
   EXISTS` + `Mutex<Connection>`"迁移模式):`conversations`(会话级索引)
   和 `conversation_turns`(回合级明细,`raw_json` 兜底原始片段)。
3. 摄取覆盖已有解析器支持的三家:**Claude / CodeBuddy / OpenCode**。
   Codex / Qoder / V8agent 现状是空实现,本次不顺带补齐,保持现状(面板
   不显示它们的历史)。
4. 摄取触发三条路径:
   - hook 事件带 `transcript_path` 时的快速通道(增量解析该文件)。
   - `agent_state_for` 把某 session 状态转入 `Idle`/`AwaitingInput` 时的
     兜底扫描(捕捉 hook 可能漏发或文件未写完整的情况),**替代定时轮询**。
   - `dozerd` 启动时对三个已知目录做一次性历史回填。
5. 增量解析:每个 session 在 `conversations` 表里维护已解析字节偏移
   (`parsed_offset`),避免每次全量重读大文件;检测到文件被截断/重写
   (当前文件大小 < `parsed_offset`)时偏移归零重新全量解析。
6. 用量统计不单独建表,`GetUsageSummary` 对 `conversation_turns` 现算
   聚合,并在聚合时按 `message_key` 去重(见下)。
7. `dozer-core::protocol` 新增 3 组 `Request`/`Reply`
   (`ListConversations`/`GetConversationTurns`/`GetUsageSummary`),
   `dozer-client` 加对应方法。`dozer-mcp` 本次不接入(留后续)。
8. 迁移完成后 `cargo build`(全 workspace)、`cargo test -p dozerd -p
   dozer-app -p dozer-core -p dozer-client`、`cargo clippy --all-targets`、
   `cargo fmt` 全绿;对话面板/用量面板在真实 GUI 里验证数据与迁移前
   一致、增量刷新生效、重启后历史不丢。

**非目标**:

- **不实现 Codex/Qoder/V8agent 的 transcript 解析**——维持这三家面板
  不显示历史的现状,不在这次顺带补齐(会拉长子项目一的周期)。
- **不做运行中回合的实时流式展示**——面板只反映已摄取入库的完整回合
  (hook/待命态触发后的结果),当前正在输出的内容仍走现有实时状态机
  (`Running`/`AwaitingInput` 指示灯),不在面板里读字机效果。
- **不接入 `dozer-mcp`**——它是面向外部 CLI agent 的只读 MCP server,
  这次范围只覆盖 `dozer-app` 内部面板,MCP 侧的对话查询能力留作自然
  延伸,不在这份 spec 展开。
- **不做共享记忆功能本体**(子项目二)——本设计只交付摄取管线这一层
  基础设施,跨会话/跨 agent 记忆共享是后续独立 spec。
- **不引入 `sqlite-vec` 等向量检索扩展**——调研时记录为子项目二可能会
  用到的选项,本次摄取管线不需要语义检索能力。
- **不做"孤儿行清理"**——文件被截断重写触发重新全量解析时,只增量
  upsert,不删除库里已存在但新内容里没有对应 `message_key` 的旧行
  (这类场景理论上少见;且 Dozer 定位是治理/审计层,保留历史比激进清理
  更符合产品取向)。

## 架构与数据流

### 模块搬迁

新增 `crates/dozerd/src/transcripts/` 目录(按 `acceptance.rs` 单文件量级
判断,这次搬迁内容(现状 `conversation.rs` 386 行 + `transcript.rs` 613 行
合计约 1000 行)超过单文件舒适阈值,拆三个子模块):

- `transcripts/scan.rs`——从 `dozer-app/src/conversation.rs` 搬来的目录
  发现逻辑(`list_conversations`/`list_all_conversations`/
  `*_project_dir` 系列函数),用于启动回填和"发现新 session"。
- `transcripts/parse.rs`——从 `dozer-app/src/transcript.rs` 搬来的
  `AgentKind` 分派解析器(`parse_claude_shaped_jsonl`/
  `parse_codebuddy_shaped_jsonl`/`parse_transcript`),**改造为支持"从任意
  完整行边界开始解析一段文本"**(不再假设总是整份文件从头解析)。
- `transcripts/mod.rs`——`TranscriptStore`(表结构、`ingest_session`
  编排、查询方法),对外暴露的唯一入口。

`AgentKind` 保留在 `dozer-core::protocol`(现状已在此,`dozerd`/
`dozer-app` 都能用)。

### 数据模型

```sql
CREATE TABLE IF NOT EXISTS conversations (
    session_id TEXT PRIMARY KEY,
    agent_kind TEXT NOT NULL,
    project_path TEXT NOT NULL,
    file_path TEXT NOT NULL,
    title TEXT,
    first_ts INTEGER NOT NULL,
    last_ts INTEGER NOT NULL,
    turn_count INTEGER NOT NULL DEFAULT 0,
    parsed_offset INTEGER NOT NULL DEFAULT 0,
    file_size_at_parse INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS conversation_turns (
    session_id TEXT NOT NULL,
    turn_index INTEGER NOT NULL,
    message_key TEXT NOT NULL,
    role TEXT NOT NULL,       -- human / ai / tool
    content TEXT NOT NULL,
    ts INTEGER,
    tokens_used INTEGER,      -- 该回合的 token usage(取自 raw_json,便于用量查询走索引)
    raw_json TEXT NOT NULL,
    PRIMARY KEY (session_id, message_key)
);
CREATE INDEX IF NOT EXISTS idx_turns_session_order
    ON conversation_turns(session_id, turn_index);
```

`message_key` 生成规则:优先取 agent 原始消息的稳定 ID(Claude/CodeBuddy
行内的 `message.id`/`uuid` 字段);拿不到稳定 ID 的场景(如 OpenCode 复用
Claude 形状但可能缺该字段)退化为 `format!("{session_id}:{turn_index}")`
——退化后行为等价于纯顺序 append,不具备跨重解析的去重能力,但保证不
panic、不丢数据。

**用量去重**(2026-08-20 brainstorming 会话内发现的设计要点,不可省略):
`conversation_turns` 的存储主键是 `(session_id, message_key)`,**按
session 保留完整独立的回合列表**——fork/resume 场景下新 session 复制的
旧内容会在新 session_id 下正常入库,`GetConversationTurns` 查询单个
session 时永远能拿到该 session 完整的回合序列,不会因为去重丢失内容。
去重只发生在 `GetUsageSummary` 的聚合查询里:

```sql
SELECT SUM(tokens_used) FROM (
    SELECT message_key, MAX(tokens_used) AS tokens_used
    FROM conversation_turns
    WHERE project_path = ? AND ts >= ?   -- project_path 需 JOIN conversations
    GROUP BY message_key
);
```

即"先按 `message_key` 折叠掉跨 session 重复的同一条消息,再求和",避免
fork/resume 把旧对话内容复制进新文件后被重复计入用量,同时不影响对话
展示层的完整性。**如果只是不加"必要时才做"的过度设计前提下,这是本设计
在存储层与查询层之间刻意做的分层,不是权宜之计。**

### 摄取触发与生命周期

三条触发路径共用同一个 `TranscriptStore::ingest_session(session_id,
file_path)` 入口:

1. **hook 快速通道**:`server.rs::handle_conn` 收到 `Request::HookEvent`
   且 `extract_transcript_path` 有值时,触发一次 `ingest_session`。
2. **待命态兜底**:`agent_state_for` 算出新状态是 `Idle`/`AwaitingInput`
   且该 session 已知 `transcript_path` 时,同样触发一次
   `ingest_session`——替代定时轮询,复用现有状态机,天然只在文件写稳定
   的时机(agent 不再输出)扫描。
3. **启动回填**:`dozerd` 启动时跑一次 `scan::list_all_conversations`
   等价的三目录扫描,对不在 `conversations` 表里的 session 全部
   `ingest_session`(`parsed_offset` 从 0 开始)。

`ingest_session` 内部:

1. `stat` 文件,若 `file_size < parsed_offset`(截断/重写),
   `parsed_offset` 归零。
2. 从 `parsed_offset` 处 seek 读到 EOF,**只在遇到完整的 `\n` 结尾行时
   才纳入本次解析**(文件可能正在被写入,尾部半行不消费、不推进
   offset,留到下次触发)。
3. 用 `parse::parse_transcript`(改造后的增量版)解析新增的完整行,
   逐条计算 `message_key`,`INSERT ... ON CONFLICT(session_id,
   message_key) DO UPDATE`(fork 重放同一段内容时是无害 no-op)。
4. 更新 `conversations` 的 `last_ts`/`turn_count`/`parsed_offset`/
   `file_size_at_parse`;首次摄取时顺带算 `title`(复用现有
   `conversation_title` 逻辑)。

**并发**:不引入额外的 per-session 锁。`TranscriptStore` 和
`AcceptanceStore` 同构,用单个 `Mutex<Connection>`,`ingest_session` 的
"读文件+解析+写库"整段在锁内完成——同一文件被并发触发两次时,第二次
进入临界区时 `parsed_offset` 已被第一次推进,天然读到空增量,等价于
no-op,不需要额外的去重锁。

### 协议扩展

`dozer-core::protocol` 新增:

```rust
// Request
ListConversations {
    project_path: Option<String>,
    agent: Option<AgentKind>,
    limit: u32,
    offset: u32,
},
GetConversationTurns {
    session_id: String,
    after_turn_index: i64,  // -1 表示从头
    limit: u32,
},
GetUsageSummary {
    project_path: Option<String>,
    since_ts: Option<u64>,
},

// Reply
Conversations { conversations: Vec<ConversationSummary> },
ConversationTurns { session_id: String, turns: Vec<TurnRecord> },
UsageSummary { summary: UsageSummaryPayload },
```

`ConversationSummary`/`TurnRecord`/`UsageSummaryPayload` 是新增的协议
结构体,字段直接对应上面两张表(风格参照现有 `ProjectInfo`/
`BookmarkInfo`)。`GetConversationTurns` 用 `after_turn_index` 做 keyset
分页(而非 offset)——长会话可能有几千个回合,keyset 分页在
`idx_turns_session_order` 索引上是 O(limit),不会随翻页深度退化。
`ListConversations` 数量级小(单项目几百个 session),简单 offset
分页够用。

`dozer-client` 的 `Client` 加 `list_conversations`/
`get_conversation_turns`/`get_usage_summary` 三个方法,内部走既有的
UDS 请求/响应模式。

### `dozer-app` 面板改造

`conversation.rs`/`transcript.rs` 整体删除。`usage.rs` 里的
`parse_usage`/`parse_claude_shaped_usage`/`parse_codebuddy_shaped_usage`
删除,`spawn_refresh` 改成调 `Client::get_usage_summary`;
`aggregate`/`group_usage_by_agent`/`daily_totals_by_agent` 等纯计算函数
若逻辑已经等价于 SQL 聚合就删除,若面板还需要额外的客户端侧分组
(如按 agent 分组展示)则保留、改吃 `UsageSummaryPayload` 而不是
`Vec<ConversationUsage>`。具体每处调用点的改法留给实现计划阶段(涉及
`extensions/usage.rs` 内 `view`/`update` 等 UI 代码的逐处改造,不在这份
架构 spec 里穷举)。

## 错误处理

- 单行解析失败(未知 schema、字段缺失)——跳过该行、`tracing::warn!`,
  不中断本次 `ingest_session`(延续 `agent_state_for` 对未知事件"不处理
  不报错"的降级哲学)。
- 文件读取失败(权限/暂时不存在)——`ingest_session` 返回 `Err`,调用方
  记日志后放弃本次触发,不 panic、不重试;下次 hook 事件或待命态转换
  会自然重新触发,不需要额外的 backoff 机制。
- 文件截断/重写——`parsed_offset` 归零重新全量解析;因为写入走
  `(session_id, message_key)` 主键的 `ON CONFLICT DO UPDATE`,不会产生
  重复行。
- `dozerd` 重启——`parsed_offset` 持久化在 sqlite 里,重启后从原位置
  续读,不丢摄取进度,也不会因为"从头重扫全部历史"造成启动卡顿(只有
  真正的新 session 才会触发全量解析)。
- `RailLayout`/`ShellLayout` 一类"运行时持久化数据格式不对就回落默认值"
  的宽松策略不适用于这里——`conversations`/`conversation_turns` 是
  追加式历史数据,没有"回落默认值"的对应语义,读取失败就是返回错误
  给调用方,不静默丢数据。

## 测试策略

1. `parse::parse_transcript` 的分片解析测试:同一份 fixture JSONL,分两次
   (前半/后半)喂给解析器,断言两次结果拼接等于一次性整份解析的结果
   (验证"支持从中间行边界继续解析"这个改造没有引入行为偏差)。
2. `ingest_session` 增量测试:构造 fixture,先写入部分内容 + 摄取,断言
   `conversation_turns` 只有对应部分、`parsed_offset` 正确推进;再追加
   剩余内容 + 摄取,断言新增回合入库、原有行不重复。
3. 半行不消费测试:fixture 文件以未终止的半行结尾(模拟 agent 正在写),
   断言该半行不产生任何 turn、`parsed_offset` 不越过最后一个完整换行符。
4. 截断测试:摄取一次后把 fixture 文件截断成更短内容再摄取,断言
   `parsed_offset` 归零重新解析、原有行通过 upsert 覆盖而非重复。
5. fork 去重测试:两个不同 `session_id` 的 fixture 文件共享若干相同
   `message_key` 的行,摄取两者后:
   - `GetConversationTurns` 分别查两个 session,各自完整(不因为
     `message_key` 重复而缺行)。
   - `GetUsageSummary` 对涉及两个 session 的 `project_path` 聚合时,
     重复的 `message_key` 只计入一次 token 用量。
6. 待命态兜底触发测试:模拟 hook 事件不带 `transcript_path`(或事件
   本身丢失),断言 session 转入 `Idle`/`AwaitingInput` 后仍完成摄取。
7. 启动回填测试:预置磁盘上有 `conversations` 表未记录的历史文件,
   `dozerd` 启动后断言这些历史被摄取入库。
8. `cargo build`(全 workspace)、`cargo test -p dozerd -p dozer-app -p
   dozer-core -p dozer-client`、`cargo clippy --all-targets -- -D
   warnings`、`cargo fmt -- --check`。
9. 真实 GUI 验证(单测覆盖不到端到端体验):对话面板、用量面板展示的
   数据与迁移前(直读文件版本)一致;新开一个 agent 会话对话后,面板在
   合理延迟内(下一次 hook/待命态触发)看到新内容;重启 `dozerd` 后历史
   对话/用量不丢。

## 排期备注

搬迁量级中等:核心解析逻辑是"抄现有代码 + 改造增量接口",风险集中在
"从中间行边界继续解析"这个改造(现有 `parse_claude_shaped_jsonl` 等函数
假设输入是完整文件文本,需要确认改造后行为等价)和"用量去重"这个新引入
的查询逻辑(此前完全没有,是这次调研中新发现的需求)。建议实现阶段顺序:
先落数据模型+ `TranscriptStore` 骨架(表结构、`ingest_session` 的读写
逻辑,不接触发路径),再落三条触发路径的接线(hook/待命态/启动回填,
可各自独立验证),再落协议扩展 + `dozer-client` 方法,最后落
`dozer-app` 面板改造(删旧代码、接新查询,建议逐面板——先对话面板、
再用量面板——分别验证,不要求一次性两个面板同时切换完成)。
