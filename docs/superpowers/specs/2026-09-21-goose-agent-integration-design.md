# Goose Agent 接入 Dozer 设计

> 状态：设计稿，待评审  
> 日期：2026-09-21  
> 实现计划：`docs/superpowers/plans/2026-09-21-goose-agent-integration.md`

## 1. 背景与目标

Dozer 当前正式支持 Claude、CodeBuddy、OpenCode、Codex 和 V8agent。接入 Goose 的目标不是把 Goose 嵌入成一套新的聊天 UI，而是让它成为与现有 agent 同级的一等公民：用户能从 Agent 选择器启动 Goose，Dozer 能识别其运行状态、摄取对话和工具轨迹、在会话关闭时生成摘要，并能把 Todo 任务派给 Goose 处理。

Goose 当前提供三类可用接口：

1. `goose session`：终端交互入口，最符合 Dozer 现有 PTY 工作区。
2. Open Plugins hooks：支持 `SessionStart`、`UserPromptSubmit`、`PreToolUse`、`PostToolUse`、`PostToolUseFailure`、`Stop`、`SessionEnd`、`AfterFileEdit` 等事件，事件 JSON 从 stdin 传给命令 hook。
3. ACP：`goose acp` 可通过 stdio 提供会话创建、流式文本、工具调用、权限请求和取消。

本期采用 **PTY + hooks**：PTY 保留 Goose 原生 CLI 体验，hooks 接入 Dozer 已有状态总线和 transcript 管线。ACP 是未来把 agent 交互原生化时更好的边界，但本期采用会迫使 Dozer 同时实现 ACP client、权限 UI、流式渲染和恢复语义，超出“新增一个 agent adapter”的合理范围。

官方依据：

- [Goose CLI Commands](https://goose-docs.ai/docs/guides/goose-cli-commands/)：交互会话命令为 `goose session`，一次性任务命令为 `goose run`；1.10.0 起原生会话存储为 SQLite，而非 JSONL。
- [Goose Hooks](https://goose-docs.ai/docs/guides/context-engineering/hooks/)：插件发现目录、事件集合、stdin payload 与失败语义。
- [Goose Custom Distributions / ACP](https://github.com/aaif-goose/goose/blob/main/CUSTOM_DISTROS.md)：ACP 支持会话、流式消息、工具调用、权限请求和取消。

## 2. 范围

### 2.1 本期目标

- 新增 `AgentKind::Goose`，贯通协议序列化、UI 标签、颜色、图标回退与所有穷尽分支。
- Agent 选择器新增 Goose，并在 PTY 中键入 `goose session`。
- 自动安装用户级 Dozer hook plugin，不修改 Goose 源码。
- 将 Goose hook 事件转成 `Request::HookEvent`，驱动 Dozer 四态状态机。
- 将 hook 事件持久化成 Dozer 自有的 Goose JSONL，并由 dozerd 增量摄取。
- 支持会话列表、对话详情、工具轨迹、文件修改统计和会话关闭摘要。
- 支持 headless 总结与 Todo 任务派发。
- 安装/卸载幂等，Dozer 故障不得阻塞 Goose。

### 2.2 非目标

- 不读取或直接写 Goose 的 `sessions.db`。这是 Goose 私有持久化实现，不是集成契约。
- 不把 dozer-app 改造成 ACP client；不在本期实现原生权限弹窗、流式消息 UI 或 ACP session resume。
- 不伪造 token 用量。官方 hook payload 没有稳定的 usage 字段，因此首期 Goose 会话的 token 统计显示为未知/零。
- 不接管 Goose 自身的权限、安全审查、provider、recipe、subagent 或 MCP 扩展管理。
- 不承诺摄取完整工具输出。当前官方 hook payload 公开了工具名、输入、调用 ID 和成功/失败事件，但未把工具输出列为稳定字段。
- 不导入 Dozer 外启动的全部 Goose 历史；首期只摄取带 `DOZER_SESSION_ID` 的 Dozer 会话。

## 3. 核心架构决策

### D1：交互继续走 PTY，不走 ACP

Agent 选择器启动 `goose session`。这保留 Goose 原生终端交互、审批和快捷键，也与 Dozer 现有 tab 生命周期一致。

`AgentKind::label()` 仍返回裸 CLI 名 `goose`，但启动命令不再能一律等于 `label()`：Goose 需要子命令 `session`。因此 `agent_cli_command` 改为显式穷尽映射并返回可带参数的命令字符串，Goose 分支返回 `goose session`。

### D2：使用用户级 Open Plugins hook，而非修改 Goose 配置数据库

安装目录：

```text
~/.agents/plugins/dozer/
├── plugin.json
└── hooks/
    └── hooks.json
```

`hooks.json` 注册：

- `SessionStart`
- `UserPromptSubmit`
- `PreToolUse`
- `PostToolUse`
- `PostToolUseFailure`
- `AfterFileEdit`
- `Stop`
- `SessionEnd`

每个 command action 都调用绝对路径：

```text
'/Applications/Dozer AI Coder.app/Contents/MacOS/dozer-hook' goose <Event>
```

不设置 `on_failure: block`，超时设为 1 秒。`dozer-hook` 自身仍以 200ms 写超时、恒成功退出、stdout 保持为空。这样 Dozer 不在线、socket 不存在或 JSON 异常都不会改变 Goose 的工具决策或阻塞会话。

安装器只维护 `~/.agents/plugins/dozer` 这个自有目录，不改用户其他插件。卸载只删除这两个由 Dozer 生成的文件；目录内出现未知文件时拒绝递归删除，避免误伤。

### D3：Dozer 会话 ID 仍是主关联键

Dozer 创建 PTY 时已经注入 `DOZER_SESSION_ID`。Goose hook command 应继承该环境变量；`dozer-hook` 以它作为 `Request::HookEvent.session_id`。hook payload 自带的 Goose `session_id` 保留在 `data.goose_session_id`，只作诊断和未来 resume 映射，不替代 Dozer ID。

实现前必须用真实 Goose 做 spike，确认：

- hook 子进程能读取 `DOZER_SESSION_ID`；
- hook command 的 cwd 是会话工作目录，或 payload 能稳定提供等价路径；
- 事件名和字段与当前官方文档一致；
- `Stop` 在普通完成、中断和错误场景下的触发行为。

任一项失败时，只调整 Goose adapter 的关联/字段提取，不改通用 HookEvent 协议。

### D4：写 Dozer 自有 hook journal，不解析 Goose SQLite

文件位置：

```text
~/.dozer/agents/goose/projects/<cwd-key>/<dozer-session-id>.jsonl
```

每次 hook 调用追加一行：

```json
{
  "schema_version": 1,
  "type": "goose_hook",
  "event": "PreToolUse",
  "ts_ms": 1789950000000,
  "dozer_session_id": "...",
  "goose_session_id": "...",
  "payload": {"tool_name":"developer__shell","tool_input":{"command":"cargo test"}}
}
```

采用原始事件 journal 而不是伪装成 Claude JSONL，原因是：

- Goose hooks 是稳定集成面，Goose SQLite schema 不是。
- hook payload 没有稳定的 token usage 和完整 tool output；伪造 Claude shape 容易让下游误以为数据完整。
- Goose 的 `AfterFileEdit` 是比猜测各个 MCP 工具输入更可靠的“发生了文件修改”信号，需要 Goose 专用 parser 才能准确表达。

`dozer-hook` 在转发前把 journal 的绝对路径补入 `data.transcript_path`，复用 dozerd 现有 `maybe_ingest_from_hook_data`。

### D5：Goose 专用增量 parser

`parse_goose_hook_chunk` 按事件生成 `ParsedTurn`：

| 事件 | 转换 |
|---|---|
| `UserPromptSubmit` | `role=human`，内容取 `message` |
| `PreToolUse` | `role=ai`，记录一个 tool call，保存 `tool_name`、`tool_input` 和 `tool_call_id` |
| `PostToolUse` | `role=tool_result`，内容为稳定的“工具执行成功”摘要，`is_error=false` |
| `PostToolUseFailure` | `role=tool_result`，内容为稳定的“工具执行失败”摘要，`is_error=true` |
| `AfterFileEdit` | `role=ai` 的文件修改事件，`mutating_tool_calls=1`，文件路径优先取 `matcher_context` |
| `Stop` | `role=ai`，内容取 `last_assistant_message`；为空则不产出 turn |
| 生命周期及未知事件 | 不产出 turn，但仍驱动状态或保留在 journal |

工具调用与最终回复分成多个 turn 是有意的：当前摄取接口是追加式、按 offset 增量解析，不能在 `Stop` 到达后回写先前 turn。排序由 journal 行顺序和 `turn_index` 保证。

parser 必须容忍字段缺失和未来新增事件，不 panic；未知 `schema_version` 跳过并记录 warning。

### D6：状态映射复用通用事件名

- `SessionStart` / `SessionEnd` → `Idle`
- `UserPromptSubmit` / `PreToolUse` / `PostToolUse` / `PostToolUseFailure` / `AfterFileEdit` → `Running`
- `Stop` → `TurnEnded`

Goose 的交互审批发生在其终端 UI 内。官方 hook 事件没有独立、稳定的“等待用户批准”事件，因此本期不把 Goose 映射为 `AwaitingInput`；不能用 `PreToolUse` 猜测，因为它也会在自动批准模式下触发。

### D7：headless 适配

- 总结：`goose run --no-session --quiet --text <prompt>`，移除 `DOZER_SESSION_ID`，避免为总结任务产生可见会话和 hook 数据。
- Todo：在项目 cwd 下执行 `goose run --quiet --text <prompt>`，保留 `DOZER_SESSION_ID`，使 hooks 能记录执行状态和 transcript。

具体 flag 以实现时本机 `goose --version` / `goose run --help` spike 为准；若 `--quiet` 与结构化输出冲突，优先保证 stdout 能提取 Dozer 的 summary marker。

### D8：MCP 不阻塞首期

Goose 支持通过 `--with-extension` 挂 stdio MCP，但用户级持久化配置格式及覆盖规则需要独立验证。首期 Goose 可以作为完整编码 agent 工作，但 Todo 内的 `toggle_todo` 等 Dozer MCP 工具不是验收门槛。

计划先做一个受控 spike：若能通过公开稳定配置幂等安装 `dozer-mcp serve`，则扩展 `dozer-mcp::install`；否则在 Goose 启动命令和 headless Todo 命令上追加 `--with-extension "dozer:<absolute-dozer-mcp> serve"`，并把安全 shell quoting 做成独立纯函数。两条路径都失败时明确降级为“无 Dozer MCP”，不修改 Goose 私有数据库。

## 4. 代码改动面

### `dozer-core`

- `protocol.rs`：新增 `AgentKind::Goose`、标签和序列化测试。
- `agent_paths.rs`：新增 `goose_project_dir(_in)`，根目录为 `.dozer/agents/goose`。

### `dozer-hook`

- `main.rs`：识别 `goose`。
- 新增 `goose.rs`：规范化 payload、确定 cwd、追加 hook journal、补 `transcript_path`。
- 新增 `goose_install.rs`：生成/卸载 Open Plugins manifest 与 hooks 配置。
- CLI 的 `install goose` / `uninstall goose` 走专用安装器。

### `dozerd`

- `transcripts/parse.rs`：新增 Goose parser 和 trace detail 提取。
- `transcripts/scan.rs`、`mod.rs`：扫描、回填、列表查询纳入 Goose 路径。
- `headless_agent.rs`：新增 Goose 总结和 Todo 命令。
- `default_agent_config.rs`、`todo.rs`、`session_summary.rs` 等 agent 字符串映射纳入 Goose。

### `dozer-app`

- picker 新增 Goose；`agent_cli_command` 显式返回 `goose session`。
- hook 安装目标新增 `GoosePlugin`。
- Agent 颜色新增映射；找不到合适且授权明确的 logo 时回落 `IconKind::Bot`。
- `should_summarize_on_close`、Agent 卡片 activity gate、对话筛选等纳入 Goose。
- Usage 主表允许 Goose 会话出现，但 token 为未知/零；每日趋势图不在本期扩列。

## 5. 错误处理与安全边界

- hook 只观察，不参与 Goose 的 allow/deny；stdout 永远为空。
- journal 写失败仍继续转发状态事件；UDS 转发失败仍退出 0。
- payload 可能包含 prompt、命令、路径等敏感信息，文件目录按现有 Dozer 数据目录权限创建，不额外上传。
- 单行 payload 设置大小上限（建议 1 MiB）；超限时保留事件元数据并把大字段替换为截断标记，防止磁盘和 SQLite 被异常工具输出拖垮。
- 安装器解析/写文件失败时拒绝覆盖，不生成半份配置；使用临时文件 + rename 原子替换。
- 现有用户插件、Goose 配置、provider 配置和 `sessions.db` 一律不碰。

## 6. 验收标准

1. 从 Dozer picker 选择 Goose，PTY 启动 `goose session`，tab 标题识别为 Goose。
2. 提交 prompt 后状态变为 Running，回合结束后变为 TurnEnded，退出后为 Idle。
3. 对话列表能看到人类 prompt、最终 assistant 回复、工具调用成功/失败及文件修改记录。
4. 关闭存活 Goose tab 时能生成会话摘要；失败时走现有启发式兜底。
5. Todo 可选择 Goose 并在项目 cwd 执行；无 Goose CLI 时给出可诊断的 spawn 错误，不影响其他 agent。
6. 重复安装不产生重复 hook；卸载后不残留 Dozer hook，且不影响其他插件。
7. dozerd 停止、socket 缺失或 journal 不可写时，Goose 会话与工具执行不被阻塞。
8. 全 workspace 单测、clippy、fmt 通过，并完成一次真实 Goose 人工验收。

## 7. 后续方向

- ACP 原生会话：当 Dozer 要提供独立于终端的权限 UI、消息流、取消和 resume 时，再新增 ACP client backend，而不是扩张 hook journal。
- Usage：等待 Goose hooks/ACP 暴露稳定的 per-turn usage，或获得官方稳定的会话导出 API 后再接；不读取私有 SQLite 猜字段。
- 外部历史导入：通过 Goose 官方 export/API 做显式导入，不扫描数据库内部表。
