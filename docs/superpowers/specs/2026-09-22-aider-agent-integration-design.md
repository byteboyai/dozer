# Aider Agent 接入 Dozer 设计

> 状态：设计稿，待评审  
> 日期：2026-09-22  
> 实现计划：`docs/superpowers/plans/2026-09-22-aider-agent-integration.md`

## 1. 背景与目标

Aider 是一个运行在终端中的 AI pair-programming agent，支持多家模型、代码库地图、文件编辑、lint/test 和自动 git commit。Dozer 接入 Aider 的目标，是让它和 Claude、CodeBuddy、Codex、Goose、OpenCode、v8agent 一样成为一等 agent：可从 picker 启动、可显示状态、可浏览对话、可生成会话摘要、可被 Todo 派发。

Aider 与已有 adapter 的关键差异是：

- 没有 Claude/Goose 那样的稳定生命周期 hooks。
- 没有 ACP/app-server 之类结构化双向协议。
- 官方 Python scripting API 明确不保证向后兼容，不适合作为 Dozer 的生产边界。
- 有稳定 CLI 参数、每会话 chat history、input history，以及“回复完成、等待用户输入”时执行的 notification command。

因此本期采用 **PTY + Rust launcher bridge + Aider 官方 history/notification 接口**。Dozer 不修改 Aider 源码、不 import Aider Python 包，也不解析 ANSI 终端输出。

官方依据：

- [Aider 首页](https://aider.chat/)：Aider 是终端 pair-programming agent，并会编辑及提交代码。
- [Options reference](https://aider.chat/docs/config/options.html)：公开 `--chat-history-file`、`--input-history-file`、`--message`、`--yes-always`、`--notifications-command` 等 CLI 契约。
- [Notifications](https://aider.chat/docs/usage/notifications.html)：notification command 在 LLM 完成回复并等待用户输入时执行。
- [Scripting aider](https://aider.chat/docs/scripting.html)：`--message` 执行单条指令后退出；Python API 不承诺兼容。
- [Aider io.py](https://github.com/Aider-AI/aider/blob/main/aider/io.py)：chat history 的用户输入使用 `#### ` 标记，assistant 回复作为 Markdown 正文追加；该文件本身也是官方分享 transcript 的数据源。

## 2. 范围

### 2.1 本期目标

- 新增 `AgentKind::Aider` 并贯通协议、UI、筛选、摘要和 Todo。
- picker 启动 Dozer 自带的 Aider launcher，由 launcher 拉起真实 `aider`。
- 每个 Dozer 会话使用独立的 Aider chat/input history 文件，不污染项目根目录的默认历史。
- 输入提交后状态变为 Running；回复完成后变为 TurnEnded；进程退出后变为 Idle。
- 将 Aider Markdown chat history 转成 Dozer 自有 canonical JSONL，再复用现有 transcript store。
- 支持人类/assistant 对话摄取、历史列表、详情、关闭摘要和 headless Todo。
- 保留用户原有 Aider provider/model/git 等配置；Dozer 只覆盖集成所需的 history 与 notification 参数。

### 2.2 非目标

- 不 import 或 monkey-patch `aider.coders.Coder` 等 Python API。
- 不解析 Aider 的 ANSI/TTY 屏幕文本来判断状态。
- 不声称支持精确工具调用、思考内容、文件修改数或 token usage；Aider chat history 没有这些稳定结构化字段。
- 不扫描项目默认的 `.aider.chat.history.md` 作为全局历史导入；首期只管理 Dozer launcher 创建的独立会话。
- 不支持会话恢复或把 Dozer conversation ID 映射回 Aider 内部上下文；`--restore-chat-history` 默认不启用。
- 不替用户改变 `auto-commits`、`dirty-commits`、模型、API key 或确认策略。
- 不接 Dozer MCP。Aider 截至本设计时没有官方原生 MCP client，相关需求仍处于开放讨论。

## 3. 架构概览

```text
dozer-app picker
  │ 键入: '<dozer-hook>' launch aider
  ▼
dozer-hook launcher（保持 stdio/TTY 原样继承）
  ├─ 生成 session paths
  ├─ 上报 SessionStart
  ├─ 监听 input history 增长 → UserPromptSubmit
  └─ spawn aider
       --chat-history-file <session>.chat.md
       --input-history-file <session>.input.history
       --notifications
       --notifications-command "'<dozer-hook>' aider Stop"
          │
          ├─ 用户继续在原生 Aider TUI 操作
          └─ 回复完成 → dozer-hook aider Stop
                    ├─ Markdown history → canonical JSONL 增量同步
                    └─ HookEvent{Aider, Stop, transcript_path}

launcher 等待 aider 退出
  ├─ 最后同步一次 transcript
  └─ 上报 SessionEnd
```

## 4. 核心架构决策

### D1：新增 launcher 子命令，不直接键入裸 `aider`

picker 对 Aider 返回：

```text
'<dozer-hook-absolute-path>' launch aider
```

复用已随 app bundle 分发的 `dozer-hook`，不新增一个二进制。launcher 用 `std::process::Command` 拉起 `aider`，stdin/stdout/stderr 全部继承，因此用户看到的仍是原生 Aider TUI，Dozer PTY 不发生嵌套。

launcher 负责解析 Aider 二进制：优先继承当前 shell 的 PATH；找不到时以明确错误退出。它不得自动安装 Aider。官方安装仍由用户完成。

### D2：会话文件由 Dozer 隔离管理

每个 Dozer session 使用以下文件：

```text
~/.dozer/agents/aider/projects/<cwd-key>/<dozer-session-id>.chat.md
~/.dozer/agents/aider/projects/<cwd-key>/<dozer-session-id>.input.history
~/.dozer/agents/aider/projects/<cwd-key>/<dozer-session-id>.jsonl
~/.dozer/agents/aider/projects/<cwd-key>/<dozer-session-id>.bridge.json
```

- `.chat.md`：Aider 原生 Markdown history。
- `.input.history`：Aider/prompt-toolkit 输入历史，仅供状态 watcher 使用。
- `.jsonl`：Dozer canonical transcript，唯一进入 dozerd parser 的文件。
- `.bridge.json`：同步 cursor、已输出消息 hash 和 schema version；由 bridge 原子更新。

launcher 以 CLI 参数显式传入 history 路径。CLI 参数优先于用户 `.aider.conf.yml`，因此只覆盖这两个文件位置和 notification，不改写用户配置文件。

### D3：状态由 input history + notification + 进程生命周期组合产生

- launcher 启动成功 → `SessionStart` → Idle。
- `.input.history` 出现一条新的非空、非 slash-command 输入 → `UserPromptSubmit` → Running。
- notification command → `Stop` → TurnEnded。
- Aider 子进程退出 → `SessionEnd` → Idle。

input watcher 轮询文件 metadata，发现增长后按 prompt-toolkit `FileHistory` 格式读取新增记录；只要确认有一条完整的新输入就上报一次。它不把输入内容当 transcript 真相源，内容仍以 `.chat.md` 为准。

`/run`、`/git` 等 Aider slash command 是否触发 Running 由 spike 决定。默认排除所有 `/...`，因为多数是本地控制命令，不代表一次 LLM turn；`/ask`、`/code`、`/architect` 等会调用模型的命令若可稳定识别，可加入白名单。

Aider 在内部确认问题时没有公开事件，因此本期不产生 AwaitingInput，不能从终端提示字符串猜测。

### D4：notification command 只承担“完成边沿”

launcher 强制追加：

```text
--notifications
--notifications-command "'<dozer-hook>' aider Stop"
```

notification 子进程继承：

- `DOZER_SESSION_ID`
- `DOZER_AIDER_CWD`
- `DOZER_AIDER_CHAT_HISTORY`
- `DOZER_AIDER_INPUT_HISTORY`
- `DOZER_AIDER_CANONICAL_TRANSCRIPT`
- `DOZER_AIDER_BRIDGE_STATE`

`dozer-hook aider Stop` 先执行 transcript 同步，再把 canonical `.jsonl` 绝对路径放进 `data.transcript_path`，随后发送通用 `Request::HookEvent`。

notification command 没有结构化 payload，`dozer-hook` 不从 stdin 等数据；事件名和路径全部来自受控参数/环境变量。

用户原本设置的 notification command 会在 Dozer 启动的这个会话里被覆盖。该取舍必须写进用户文档；Dozer 不修改持久配置，用户直接启动 Aider 时完全不受影响。

### D5：Markdown 先同步成 canonical JSONL，再摄取

不让 dozerd 直接增量解析 `.chat.md`，因为一次 assistant 回复会在既有 `#### user` 段后继续追加，单纯按 byte offset 读取新 chunk 会丢失“当前处于哪个角色”的上下文。

bridge 每次 Stop/SessionEnd：

1. 读取完整 `.chat.md`，限制最大文件大小，防止异常文件拖垮 hook。
2. 按 Aider 稳定格式切分：`#### ` 开始用户消息；随后非引用 Markdown 正文为 assistant 回复；`# aider chat started at ...` 是会话边界；`>` 工具/确认输出不作为人类或 assistant 正文。
3. 生成稳定消息 ID：`sha256(role + normalized_content + occurrence_index)`。
4. 与 `.bridge.json` 的已同步 ID 集合比较，只把新增的完整消息追加到 canonical JSONL。
5. 使用文件锁保护并发 notification/SessionEnd；先写 canonical 行并 `flush`，再原子替换 bridge state。

canonical JSONL 形状：

```jsonl
{"schema_version":1,"type":"aider_message","message_id":"...","role":"human","content":"请修复测试","ts_ms":1790030000000}
{"schema_version":1,"type":"aider_message","message_id":"...","role":"ai","content":"已修复……","ts_ms":1790030010000}
```

Markdown 自身没有逐消息时间戳，因此：

- 同一次同步产生的消息以同步时刻为基准，按文件顺序递增 1ms，保证稳定排序。
- 已同步消息永不重写时间戳。

如果 Markdown parser 遇到未闭合 fence、未知结构或只有用户消息没有 assistant 回复，只追加能确定角色和边界的消息；不 panic、不猜测。

### D6：Dozerd 使用简单的 Aider canonical parser

`parse_aider_chunk` 只解析 `type:"aider_message"`：

- `role:"human"` → human turn。
- `role:"ai"` → ai turn。
- `message_key` 使用 `message_id`。
- 工具、mutation、files touched、thinking 和 token 字段均为零/空。
- 未知 schema、role 或畸形行跳过并 warning。

这样所有 Aider 格式脆弱性集中在 launcher bridge；dozerd 只消费 Dozer 自己控制的稳定格式。

### D7：headless 总结与 Todo

Aider 官方支持：

```text
aider --message <instruction>
```

该模式执行一条消息并退出。Dozer adapter：

- 总结：`aider --message <summary-prompt> --no-stream --no-pretty --no-auto-commits`，移除 `DOZER_SESSION_ID`。总结只读取传入文本，不允许产生代码提交。
- Todo：在项目 cwd 执行 `aider --message <task-prompt> --yes-always --no-stream --no-pretty`，保留用户默认 auto-commit 行为，不额外传 `--auto-commits` 或 `--no-auto-commits`。

Todo 的 stdout 作为结果回合保存；headless Todo 不依赖 notification bridge，避免同一结果同时由 stdout 和 history 重复落库。为此 headless 命令显式移除 `AIDER_NOTIFICATIONS_COMMAND` 和 Dozer bridge 环境变量。

实现前 spike 必须确认 `--no-pretty` 的实际 flag、stdout 是否只含可接受文本，以及退出码非零时的 stderr 行为。若 stdout 混有启动 banner，先写纯函数清洗器；不得用易变的颜色/光标序列作为协议。

### D8：Git 与文件修改统计

Aider 默认可能自动提交修改。Dozer 不改变这个默认行为，也不把 commit 当作 transcript tool call。

首期 `mutating_tool_calls` 和 `files_touched` 保持空值，因为：

- chat history 中的 tool output 是人类可读文本，不是稳定结构化协议。
- 用 git diff 推断会漏掉 Aider 自动提交，也会混入用户并发修改。
- 用新 commit 推断会混入 hook/其他进程产生的 commit。

未来若 Aider提供稳定 hook/JSON event stream，再单独补工具与修改统计。

## 5. 代码改动面

### `dozer-core`

- `protocol.rs`：新增 `AgentKind::Aider`。
- `agent_paths.rs`：新增 Aider project/session paths。

### `dozer-hook`

- `main.rs`：新增 `launch aider`、`aider Stop` 路由。
- 新增 `aider_launcher.rs`：路径、子进程、input watcher、生命周期事件。
- 新增 `aider_bridge.rs`：Markdown parser、去重同步、锁和 canonical JSONL。
- 抽取可复用的 `send_hook_event()`，避免 launcher 与普通 forward 各写一套 UDS 代码。

### `dozerd`

- transcript scan/list/backfill 纳入 `.dozer/agents/aider`。
- `parse.rs` 新增 canonical parser。
- headless adapter、默认 agent、Todo 和 session summary 映射纳入 Aider。

### `dozer-app`

- picker、分组、筛选、颜色、图标回退纳入 Aider。
- Aider 启动命令需要 `dozer-hook` 绝对路径，不能继续使用只返回静态 `&str` 的简单映射；新增 `agent_launch_command(agent, sibling_bins)` 或等价具名参数结构体。
- Aider 不需要 hook/config 安装器；它的 bridge 参数由 launcher 每次启动注入。
- 会话关闭摘要和 Agent 卡片 activity gate 纳入 Aider。

## 6. 错误处理与安全

- 缺 `DOZER_SESSION_ID`：launcher 拒绝进入集成模式并给出清晰错误；`dozer-hook aider Stop` 则静默退出 0。
- 找不到 `aider`：错误显示在 PTY，SessionEnd 仍上报。
- notification/UDS/同步失败：不能杀死或阻塞 Aider；notification 子命令恒退出 0。
- history 文件最大读取量建议 16 MiB；超过后停止同步并 warning，不截断原始 Aider 文件。
- canonical JSONL 单条消息最大 1 MiB，超限截断并写 `truncated:true`。
- bridge state 不保存 API key、环境变量或完整项目配置。
- session ID 只接受现有 Dozer UUID/任务 ID安全字符集，禁止路径分隔符和 `..`。
- launcher 参数使用 `Command::arg`，notification command 中的可执行文件路径使用 POSIX 单引号转义。

## 7. 必做 Spike

实现生产代码前，用当前官方 Aider 版本验证：

1. `--chat-history-file` 和 `--input-history-file` 是否覆盖用户配置且自动建父目录。
2. input history 的真实 prompt-toolkit 格式，普通、多行和 slash command 的写入时机。
3. chat history 对普通回复、代码 fence、确认问题、工具输出、architect mode、失败响应的真实 Markdown。
4. notification command 是否继承全部 `DOZER_*` 环境变量、cwd 和 PATH。
5. notification 在成功、模型错误、用户中断、空回复、architect 双模型流程中的调用次数。
6. headless `--message --no-stream --no-pretty` 的 stdout/stderr/退出码。

所有样例去敏后保存到 `crates/dozer-hook/fixtures/aider/`；生产 parser 必须以 fixture 测试锁定，不能只依赖手写 JSON/Markdown。

## 8. 验收标准

1. picker 选择 Aider 后进入原生 Aider TUI，用户现有 model/provider/git 配置仍生效。
2. 普通和多行 prompt 提交后状态变 Running，回复完成后变 TurnEnded，退出后变 Idle。
3. 每个 tab 产生独立 history；同项目并行两个 Aider tab 不串会话。
4. 对话列表正确显示 human/assistant 文本，代码 fence 和正文中的 `####` 不误切分。
5. notification 重复执行、进程异常退出或 app 重启回填不会产生重复 turn。
6. 关闭存活会话能生成摘要；headless 失败时走现有启发式兜底。
7. Todo 能用 Aider 修改项目并记录输出；用户默认 auto-commit 策略未被 Dozer 改写。
8. Dozer/UDS 不在线时 Aider 仍能正常工作。
9. token/tool/file mutation 缺失以“暂无数据”呈现，不显示伪造数字。
10. workspace tests、clippy、fmt 与真实 Aider 人工验收全部通过。

## 9. 后续方向

- 若 Aider 提供官方稳定 JSON event stream/hooks，逐步替换 input watcher 与 Markdown bridge。
- 若 Aider 原生支持 MCP，再接入 `dozer-mcp`，不通过第三方代理冒充官方能力。
- 独立设计“导入项目已有 `.aider.chat.history.md`”，需要解决同一文件多次启动、会话分段和项目归属，不能顺带塞进首期。
