# V8agent 接入 Dozer：对话摄取 + 用量统计

**状态:已批准(brainstorming 会话,2026-08-24)**

**跨仓库**:这份 spec 同时覆盖 `dozer`(本仓库)和 `v8agent`(独立仓库,
`../v8agent`,sibling checkout)两边的改动。v8agent 是 2026-08-08
brainstorming 决定"内置 agent 核心逻辑挪独立新项目先做"([[dozer-builtin-agent-work-paused]])
的产物,现已有 `v8agent-core`(rig 封装)+ `v8agent-cli`(REPL 二进制)。

## 背景

`AgentKind::V8agent` 在 dozer 里已经是正式枚举成员,picker 可选、有专属
图标/主题色、CLI 命令可拉起进程。2026-08-20 kooky 对比评价 triage 时
([[dozer-kooky-comparison-review-2026-08-20]])判定它是"纯 GUI 占位,没有
真实 hook 支持",用户当时选择维持现状搁置。

重新核实后发现这个判断已经过时(至少部分):`v8agent-cli/src/dozer_bridge.rs`
在 `DOZER_SESSION_ID` 存在时,已经把 `UserPromptSubmit`/`PreToolUse`/
`PostToolUse`/`Stop` 四种事件通过 UDS socket 直接上报给 dozerd(不是走
Claude Code 那种"外部 hook 配置文件"机制,是进程内直连)。dozerd 的
`agent_state_for()` 按事件名映射四态是通用的,不分 agent kind——**会话
存活状态追踪现在就已经对 V8agent 生效**,这部分不需要开发。

真正缺的是"对话内容"和"用量统计",而且缺口的根因是架构不匹配,不是
少写了一个解析分支:dozerd 现有的摄取管线(`TranscriptStore::ingest_session`
+ `transcripts/parse.rs`)整条链路都是**文件驱动**的——hook 事件带
`data.transcript_path` 字段,dozerd 据此打开磁盘上的 JSONL 文件按 offset
增量读取,再交给按 `AgentKind` 分派的 `parse_chunk`。但 V8agent 现在完全
不往磁盘写会话记录,`dozer_bridge.rs` 的 `data` 字段里只有
`tool_name`/`tool_input`/`tool_result`/`is_error`,没有 `transcript_path`——
所以即使给它补一个 `parse_chunk` 分支也无源可读。`usage.rs` 的
`ConversationUsage` 完全下游于解析出的 turn 数据,所以用量统计不是独立的
第三块工作,是对话摄取做完自动带出来的副产品;但 v8agent-core 自己也有
一个独立缺口——它把 rig 流里的 per-request usage(token 数)直接丢弃
(`event.rs` 注释:"per-request usage ... 不产出可见事件"),这部分需要
先在 v8agent 仓库补上。

## 架构决策:复用文件式摄取,而非新开内存态摄取路径

考虑过两个方案:

- **方案 A(采用)**:让 v8agent-cli 自己把会话写成本地 JSONL 文件,格式
  对齐 dozerd 现有的"Claude 形状"解析器(`parse_claude_shaped_chunk`,
  Claude/Opencode/Kilo 三家共用),hook 上报时带上 `transcript_path`。
  dozerd 这边只需要把 `AgentKind::V8agent` 从"恒返回空"分支挪进
  claude-shaped 分支,零新解析器。
- **方案 B(放弃)**:v8agent 不落盘,dozerd 新开一条内存态摄取路径,
  在会话存活期间按 session_id 攒 turn 数据,状态转 `Idle`/`AwaitingInput`
  时落库。放弃原因:dozerd 要为一个 agent 单独维护一套跟现有 6 个
  agent 共用的文件式管线完全不同的架构,daemon 重启会丢失进行中回合的
  状态,也没法像其他 agent 那样"回填历史会话"(`backfill_project`)。
  长期维护成本明显更高,不符合 YAGNI。

方案 A 的额外收益:v8agent 现在 REPL 一退出所有对话历史就没了,这个
transcript 写入器顺带补上了它自己独立的会话持久化能力,不依赖 dozer。

## 改动一:v8agent-core —— 捕获 usage

`crates/v8agent-core/src/event.rs` 里 `translate_stream_item` 的
`MultiTurnStreamItem::FinalResponse(response)` 分支目前直接丢弃 `response`
携带的数据,只产出无字段的 `AgentEvent::TurnEnded`。核实发现 `response`
的类型 `rig::agent::PromptResponse`(准确说是 `rig_agent::prompt_request::PromptResponse`,
经 `rig::agent` 重导出)本身就有一个 `pub usage: rig::completion::Usage`
字段——"本轮运行的聚合 token 用量",不需要再手动调用
`GetTokenUsage::token_usage()`,也不需要新定义一个 `TokenUsage` 结构体。
`rig::completion::Usage` 派生了 `Debug, PartialEq, Eq, Clone, Copy,
Serialize, Deserialize`,字段是
`input_tokens`/`output_tokens`/`total_tokens`/`cached_input_tokens`/
`cache_creation_input_tokens`/`tool_use_prompt_tokens`/`reasoning_tokens`,
直接满足 `AgentEvent` 现有的 `#[derive(Debug, Clone, PartialEq)]` 约束。改动:

- `AgentEvent::TurnEnded` 从无字段变体改为
  `TurnEnded { usage: rig::completion::Usage }`(非 `Option`——
  `PromptResponse.usage` 永远有值,provider 没报用量时是全零的
  `Usage::new()` 哨兵值,不是缺失)。
- `translate_stream_item` 的 `FinalResponse` 分支改为
  `Some(AgentEvent::TurnEnded { usage: response.usage })`。
- 不新开一个独立的 `AgentEvent::Usage` 变体——下游(transcript 写入器、
  hook 上报)都是"一轮结束时落一条 usage 记录",没有需要独立时序对齐的
  消费场景,拆分是不必要的复杂度。
- `AgentEvent::Error` 分支没有 usage 可带,维持不变。
- 这是一处破坏性类型改动:`event.rs`/`engine.rs` 测试里所有构造裸
  `AgentEvent::TurnEnded`(无花括号)的位置,以及 `dozer_bridge.rs`/
  `repl.rs` 里匹配 `AgentEvent::TurnEnded => ...` 的位置,都需要跟着改成
  带字段的形式——具体位置见对应实现计划。

## 改动二:v8agent-cli —— 本地 transcript 写入器

新增 `crates/v8agent-cli/src/transcript_writer.rs`:

- **文件位置**:`~/.v8agent/sessions/<id>.jsonl`。有 `DOZER_SESSION_ID`
  时用它做文件名(天然对齐 dozer 会话身份,避免额外维护一套 id 映射);
  standalone 运行(未设置该环境变量)时生成一个 UUID 做文件名。
- **写入时机**,按 `AgentEvent` 流映射成 Claude 形状的 JSONL 行(schema
  细节见下方"文件格式"一节):
  - 用户提交 prompt → 立即写一行 `type:"user"`,`message.content` 是
    纯字符串。
  - `ToolCallStarted` → 缓冲进当前轮的 `tool_use` 块列表,不立即落盘
    (要等这一轮所有工具调用/文本增量收集完才能落成一行 assistant 消息)。
  - `ToolCallFinished` → 立即单独写一行 `type:"user"`,`message.content`
    是 `[{"type":"tool_result",...}]` 数组——不等轮次结束,因为
    `parse_claude_shaped_chunk` 把每条这样的行当独立的 `tool_result`
    turn 处理,不需要跟对应的 `tool_use` 行强绑定。
  - `TurnEnded { usage }` → 把本轮缓冲的文本增量(`TextDelta` 累加)、
    工具调用块(`tool_use` 列表)、`usage` 合并写成**一行** `type:"assistant"`,
    然后清空缓冲区。
  - `Error` → 把已缓冲内容 flush 成一行 assistant(不带 usage,避免半轮
    内容丢失),同样清空缓冲区。
- **hook 上报改动**:`dozer_bridge.rs` 的 `send()` 在每次上报的 `data`
  JSON 里加一个 `transcript_path` 字段,指向这个文件的绝对路径。dozerd
  现有的 `extract_transcript_path()` 逻辑会自动捕获它——不改 dozerd 侧的
  字段名/协议。

### 文件格式(与 Claude Code transcript 完全同构)

```jsonl
{"type":"user","message":{"content":"帮我改一下这个函数"},"uuid":"...","timestamp":1234567890000}
{"type":"user","message":{"content":[{"type":"tool_result","is_error":false,"content":"file contents..."}]},"uuid":"...","timestamp":...}
{"type":"assistant","message":{"content":[{"type":"text","text":"改好了"},{"type":"tool_use","name":"edit_file","input":{"path":"foo.rs"}}],"usage":{"input_tokens":120,"output_tokens":45,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}},"uuid":"...","timestamp":...}
```

磁盘上的 key 名(`input_tokens`/`output_tokens`/`cache_read_input_tokens`/
`cache_creation_input_tokens`)直接照抄 `parse_claude_shaped_chunk` 现在
读取的四个 key,不新增/改名。`transcript_writer.rs` 负责把"改动一"里
`rig::completion::Usage` 的字段映射成这四个磁盘 key:
`input_tokens`→`input_tokens`,`output_tokens`→`output_tokens`,
`cached_input_tokens`→`cache_read_input_tokens`,
`cache_creation_input_tokens`→`cache_creation_input_tokens`(这两个字段
恰好同名,不需要映射);`total_tokens`/`tool_use_prompt_tokens`/
`reasoning_tokens` 三个 `Usage` 字段磁盘格式不需要,不落盘。

## 改动三:dozer —— 解析器 dispatch + 排除表清理

`crates/dozerd/src/transcripts/parse.rs`:

- `parse_claude_shaped_chunk` 新增一个 `agent: AgentKind` 参数,函数内部
  按 agent kind 选用不同的"哪些工具算 mutating"列表:
  - Claude/Opencode/Kilo/Unknown:沿用现有
    `MUTATING_TOOLS = ["Edit","Write","MultiEdit","NotebookEdit"]`。
  - V8agent:新增 `V8AGENT_MUTATING_TOOLS = ["write_file","edit_file","git_commit"]`——
    v8agent 自己的工具名是 snake_case,跟 Claude 的工具名完全不同,直接
    复用 Claude 的列表会导致"改了文件但不计入 mutating_tool_calls/
    files_touched"的统计错误。
- `parse_chunk()` 和 `extract_turn_trace_detail()` 两处 dispatch:把
  `AgentKind::V8agent` 从"恒返回空"分支(`Codex | V8agent => ...`)移到
  claude-shaped 分支。**`Codex` 保留在恒空分支**,这次不动它——它是
  独立的已知缺口(spike fixture 缺失,注释已说明是刻意的诚实设计),不在
  这次范围内。

`crates/dozer-app/src/workspace.rs`:

- `needs_activity` 排除表(约 2241 行)去掉 `AgentKind::V8agent`(Codex
  保留排除)。
- `ensure_hook_installed`(约 3697 行)对 `AgentKind::V8agent` 返回
  `None` 这一行为**保持不变,不是要修的 bug**——v8agent 走 socket 直连
  上报,本来就不需要 Claude 那种"往 settings.json 写 hook 命令"的外部
  配置机制。只更新周边那条过时注释("Kilo/V8agent 纯 GUI 占位,没有真实
  hook 支持"),改成说明"V8agent 走直连上报,这里返回 None 是正确行为;
  Kilo 仍是真实缺口,未来要给 Kilo 补真实 hook 支持时从这条注释接着做"。

`crates/dozer-app/src/extensions/usage.rs`:

- **不需要改动**——写 spec 时以为这里有一张类似 `needs_activity` 那样的
  简单排除表,实现前重新核实(读代码)发现判断有误:usage 面板的主表
  (`WorkspaceState::rows()`,`Vec<(ConversationMeta, ConversationUsage)>`)
  完全下游于 dozerd 的 `list_conversations`/`get_usage_summary`,不按
  agent kind 过滤——`parse_chunk` 分派改对之后,V8agent 的会话/用量行
  自动出现在这张表里,dozer-app 侧没有需要跟着改的代码。
  `AgentKind::Unknown | Codex | Kilo | V8agent => {}` 那一行(167 行)
  属于另一个东西:`daily_totals_by_agent()` 产出的**按天用量趋势图**,
  `DayAgentTotals` 结构体硬编码只有 `claude`/`codebuddy`/`opencode` 三个
  字段(图表画布渲染、图例、配色都是按这三家写死的)。给 V8agent 在这张
  图里也开一列,需要新增结构体字段 + 改画布渲染 + 加图例配色,是独立量级
  的 UI 改动,不是"去掉一行排除"。这次不做,归入下面"非目标"。

### 测试改动

以下现有断言随行为变化需要更新或删除:

- `dozerd/src/transcripts/parse.rs:767`
  `assert!(parse_chunk(AgentKind::V8agent, text, "c", 0).is_empty())`——
  行为已变,需替换成断言 V8agent 走 claude-shaped 解析路径且 mutating
  判定使用新工具列表的正例测试。
- `dozer-app/workspace.rs` 里 `ensure_hook_installed_is_noop_for_kilo_v8agent_and_unknown`
  等测试:V8agent 部分保留(返回 None 的行为没变),但测试名/注释里
  "V8agent 是占位"的措辞需要更新,避免误导未来读者以为这是待修的缺口。
- `dozer-app/workspace.rs` 的 `agent_card_refresh_plan_decides_by_agent_and_cwd`
  测试:目前完全没有覆盖 V8agent 这个 case(排除表本身此前也没被
  单测锁定过),需要新增一条断言。

新增测试覆盖:

- v8agent-core:`TurnEnded` 携带 usage 的转换测试。
- v8agent-cli:`transcript_writer` 按 `AgentEvent` 流写出正确 JSONL 行
  (对齐 `dozer_bridge.rs` 现有测试的写法,用临时目录 + 读回校验)。
- dozerd:`parse_chunk(AgentKind::V8agent, ...)` 对含 `write_file`/
  `edit_file` 的样例文本正确识别为 mutating,`Edit`/`Write` 这类 Claude
  专属工具名对 V8agent 样例不触发 mutating(交叉验证参数化没有做反)。

## 非目标

- 不改 Kilo 的 hook 支持现状(仍是真实缺口,维持搁置)。
- 不改 Codex 的摄取现状(仍返回空,维持既有的"诚实设计决定")。
- 不新增 `AgentEvent::Usage` 独立事件类型(见"改动一"里的取舍说明)。
- 不在 dozerd 里新开内存态摄取路径(见"架构决策"一节)。
- v8agent 自己的 transcript 文件不做垃圾回收/轮转策略——量级和现有其他
  agent 的 transcript 目录一致,回填/清理沿用 dozerd 现有机制,不在这次
  范围内单独设计。
- 不给 `usage.rs` 的按天用量趋势图(`DayAgentTotals`)加 V8agent 列——
  详见"改动三"里的说明,是独立量级的图表 UI 改动。V8agent 的用量数据
  在 usage 面板的主表(逐会话列表)里正常可见,只是不出现在这张趋势图上,
  跟 Codex/Kilo/Unknown 现在的处境一致。
