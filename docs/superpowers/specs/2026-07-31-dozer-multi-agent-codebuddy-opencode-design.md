# Dozer 多 agent 支持——CodeBuddy 与 OpenCode 观测层接入设计

> 状态：设计中，待用户终审。
> 需求来源：2026-07-31 用户提出"让 dozer 支持 opencode agent"，讨论后确认一并把 CodeBuddy 也纳入
> 同一份设计（两者共享同一套多 agent 抽象），本文档是那一轮 brainstorming 的产出。
> 范围：本设计**只做观测层**——识别、观测、展示 CodeBuddy/OpenCode 会话产生的事件与
> transcript，复用现状"新建会话只开一个 `$SHELL`、用户自己在里面手敲 agent CLI"的交互，
> 不新增"agent 选择器"或任何启动侧改动（已与用户核对）。

## 1. 现状与动机

Dozer 今天对 agent 的支持只有 Claude Code 一家，且耦合分散在四个地方：

- `dozer-hook`：只会把自己注册进 `~/.claude/settings.json` 的 7 个 hook 事件
  （`SessionStart`/`UserPromptSubmit`/`PreToolUse`/`PostToolUse`/`Notification`/`Stop`/
  `SessionEnd`），装的 command 写死 `{exe} {event}`。
- `dozer-core/protocol.rs`：`SessionInfo.transcript_path` 注释直接写"Claude Code JSONL"；
  更关键的是，**协议里完全没有 `agent` 字段**——没有任何地方记录"这个会话当前跑的是哪个 agent"。
- `dozerd/server.rs`：`agent_state_for(event: &str) -> Option<AgentState>` 只认 Claude 的
  事件名字面量。
- `dozer-app`：`transcript.rs::parse_transcript` 是 Claude JSONL 专用解析器；
  `conversation.rs::claude_project_dir`/`list_conversations` 扫的是 `~/.claude/projects/<cwd>`，
  `ConversationMeta.agent` 字段现状恒为字符串 `"claude"`（代码注释已明确写"为多 agent 留维度，现恒
  'claude'"，说明这个缺口是预留过的）。

调研确认 CodeBuddy（腾讯云）和 OpenCode（sst/opencode）都有可用的结构化事件机制，但形态不同：

- **CodeBuddy**：hook payload 字段名（`session_id`/`transcript_path`/`cwd`/`hook_event_name`/
  `tool_name`/`tool_input`/`tool_response`）与 Claude Code **逐字对齐**，事件名族群
  （`SessionStart`/`PreToolUse`/`PostToolUse`/`PostToolUseFailure`/`Stop`/`StopFailure`/
  `SessionEnd`/`SubagentStart`/`SubagentStop`/…）也是同一套词汇的超集。session 落盘目录
  `~/.codebuddy/projects/{projectDir}/{sessionId}/` 与 Claude 的 `~/.claude/projects/<cwd>/`
  结构平行。但具体的 hook **注册方式**（全局 `settings.json` 补丁 vs plugin 包里的
  `hooks/hooks.json`）与主会话 transcript 的**精确 schema**，公开文档没有给出足够细节，需要
  实现阶段验证（见 §6）。
- **OpenCode**：没有"CLI 调用 hook 脚本"这套机制，而是本地跑一个 HTTP server（Hono），通过 SSE
  广播 `session.*`/`message.*`/`part.*` 事件，session 数据落 SQLite（不是 JSONL 文件）。但它有
  TypeScript 插件系统，插件运行在 opencode 进程内部，可以直接订阅进程内的事件总线（SSE 对外暴露
  的就是这条总线），且天然继承父进程的环境变量。

## 2. 范围裁剪（已与用户核对）

- **只做观测层**：不加"新建会话时选 agent"的 UI，维持"用户在 shell 里自己敲 CLI 名字"的现状。
- **一份 spec，两个 adapter 共享一套抽象**：CodeBuddy 和 OpenCode 的具体实现可以分头交付、互不
  阻塞（尤其是 §6 提到的 CodeBuddy 验证如果失败，不影响 OpenCode 独立交付），但协议层改动
  （`AgentKind`、规范事件名机制）只设计一次、只评审一次。
- **`agent` 由首个 hook 事件坐实，不是会话创建时猜的**：`CreateSession` 时 dozerd 不知道用户会在
  shell 里敲哪个 CLI，`SessionInfo.agent` 初始为 `Unknown`；直到第一条来自某个 agent 适配层的
  `HookEvent` 到达，才把 `agent` 和 `transcript_path` 一起坐实。用户在 shell 里跑的不是三家之一
  （或没装 hook/插件），会话就永远停在 `Unknown`——这是正常状态，不是错误。

## 3. 架构总览

```
                    ┌─────────────────────────────────────────┐
                    │  PTY 会话（dozerd 起的 shell，继承         │
                    │  DOZER_SESSION_ID env）                  │
                    │                                          │
        用户手敲 →   │  claude / codebuddy / opencode 进程       │
                    └──────────────┬───────────────────────────┘
                                   │ 触发各自的 hook/插件机制
              ┌────────────────────┼────────────────────┐
              ▼                    ▼                    ▼
      ~/.claude/settings   CodeBuddy hook 注册    opencode 插件（TS，
      .json 注册的 hook     （settings 补丁或       进程内直接跑，见 §5）
      （dozer-hook claude   plugin 包，见 §6）
       <event>）                   │                    │
              │             dozer-hook codebuddy  spawn dozer-hook
              │             <event>                opencode <event>
              └──────────┬─────────┴──────────┬─────────┘
                         ▼                    ▼
              dozer-hook 二进制：把各 agent 原生事件名翻译成
              7 个规范事件名之一（翻译表见各 agent 小节），
              转发 Request::HookEvent{ session_id, agent, event, ts_ms, data }
                         │
                         ▼
                    dozerd（Unix socket）
              agent_state_for(event) → AgentState  ← 签名不变，规范化已在上游做完
              首个 HookEvent 到达时坐实 Session.agent / Session.transcript_path
              广播 Reply::AgentEvent{ agent, state, transcript_path, … }
                         │
                         ▼
                    dozer-app（GUI）
              按 session.agent 分派到对应的 transcript 解析器 / 对话历史目录
```

核心设计决定：**规范事件词汇表就是现有的 7 个 Claude 事件名，不为多 agent 扩展。** 把不同 agent
的原生事件翻译成这 7 个名字，是每个 agent 适配层（`dozer-hook <agent>` 子命令或 opencode 插件）自
己的职责，`dozerd::agent_state_for` 和状态广播逻辑完全不用感知"有几种 agent、各自叫什么"。新增
第四个 agent 时，只要照着写一层"翻译成规范名字"的适配代码，核心状态机永远不用碰。

## 4. 协议改动

`dozer-core/protocol.rs` 新增：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    #[default]
    Unknown,
    Claude,
    Codebuddy,
    Opencode,
}
```

`SessionInfo`、`Request::HookEvent`、`Reply::AgentEvent` 均加 `agent: AgentKind`
（`#[serde(default)]`，旧协议帧缺该字段时回落 `Unknown`，与现有 `agent_state`/`transcript_path`/
`project_id` 字段的"迁移期兼容"处理方式一致）。

`dozerd/session.rs` 的 `Session` 结构体加 `agent: Mutex<AgentKind>`，与现有
`transcript_path: Mutex<Option<String>>` 并列，首条 `HookEvent` 到达时一并写入。

`agent_state_for(event: &str) -> Option<AgentState>` **签名与实现均不改**——规范化已经在
`dozer-hook`/opencode 插件那一层做完（见 §5、§6）。

## 5. dozer-hook 改动 + OpenCode 插件设计

### 5.1 CLI 形态改动

`dozer-hook` 从 `dozer-hook <event>` 改成 `dozer-hook <agent> <event>`。`agent` 是安装时写死
进 hook command 的字符串（`claude`/`codebuddy`/`opencode`），不是运行时猜的——不同 agent 装的
位置不同，天然知道自己是谁。Claude 分支完全复用现有 `install.rs` 逻辑（`run_at`/`EVENTS`/
`settings_path`），只是解析出的 `agent` 固定填 `AgentKind::Claude`。

### 5.2 CodeBuddy 事件翻译表

| CodeBuddy 原生事件 | 规范事件名 | 备注 |
|---|---|---|
| SessionStart | SessionStart | 直通 |
| UserPromptSubmit | UserPromptSubmit | 直通（若实际字段名不同，以 §6 验证结果为准） |
| PreToolUse / PostToolUse | PreToolUse / PostToolUse | 直通 |
| PostToolUseFailure | PostToolUse | 归并，失败与否不影响四态机 |
| Notification | Notification | 直通（如存在） |
| Stop / StopFailure | Stop | 归并 |
| SessionEnd | SessionEnd | 直通 |
| SubagentStart / SubagentStop | （丢弃，不转发） | 子 agent 生命周期不影响顶层四态机 |

### 5.3 OpenCode 插件

**转发方式**：插件不直接连 dozerd 的 Unix socket，而是复用 `dozer-hook`——每次识别到一个规范
事件，spawn 一次 `dozer-hook opencode <event>`，JSON 走 stdin。与 Claude Code 的"事件发生时起
一个短命进程"机制同构，socket 连接、协议编码等逻辑全部留在 Rust 里复用，插件本身只做"翻译 +
spawn"。

**事件识别**（从连续状态流里抠离散边沿，插件内维护极小的每-session 本地状态）：

| 观察到的变化 | 翻出的规范事件 |
|---|---|
| `session.created` | SessionStart |
| 新的 `message.updated`，role=user | UserPromptSubmit |
| `part.updated`，type=tool，状态首次进入 running | PreToolUse |
| `part.updated`，type=tool，状态转入 completed/error | PostToolUse |
| 需要用户批准权限、正在等待 | Notification |
| assistant 消息完成 / session 转 idle | Stop |
| `session.deleted` 或插件感知进程即将退出 | SessionEnd |

**Session 关联**：插件启动时读 `process.env.DOZER_SESSION_ID`（dozer spawn PTY 时注入，opencode
作为该 PTY 里的子进程天然继承）。每条转发事件都带正确的 dozer session_id，不需要靠 cwd+时间猜。
已知局限：如果 opencode 在同一进程内支持"多 session 切换"，dozer 的粗粒度四态机只能反映"当前
活跃的那个"，不精确区分——这次不解决，写入已知限制。

**Transcript 落盘（顺带省掉一个解析器）**：`dozer-hook opencode <event>` 收到规范化事件后，除了
转发 `Request::HookEvent`，还向 `~/.dozer/agents/opencode/projects/<cwd-key>/<session-id>.jsonl`
**追加一行、且直接写成 Claude transcript 的字段形状**（不是发明新 schema）。每次只做一次 append
（不做 read-modify-write），写入失败就跳过、记日志，不影响会话继续、不腐化已有内容。这样
`transcript.rs` 对 OpenCode **完全不用新增解析代码**——它认的是"Claude 风格 JSONL"，不关心这个
文件是 Claude 自己写的还是 dozer-hook 代写的。

**异常处理（硬性要求）**：插件运行在用户工具进程内部，风险等级最高——所有逻辑必须整体包一层
try/catch，任何 dozer 侧的 bug 都不能导致用户的 opencode 会话崩溃或卡死。

## 6. 未决问题——实现前必须验证的技术尖峰（spike）

不靠猜测定案，写 plan 时第一步就要验证：

1. **CodeBuddy hook 注册方式**：起一个真实 CodeBuddy CLI，分别试"全局 `settings.json` 补丁"和
   "plugin 包 `hooks/hooks.json`"两种注册方式，确认 hook 实际能否触发、`transcript_path`
   字段是否存在且指向可解析的文件。验证结果只决定 `codebuddy_install.rs` 具体怎么写，不影响本
   spec 其余部分——因为不管哪种注册方式，dozer-hook 收到的 stdin JSON 字段与转发出去的
   `Request::HookEvent` 形状是一样的，差异被完全封在"怎么把自己装进 CodeBuddy"这一小块里。
2. **CodeBuddy transcript 精确 schema**：拿到 §6.1 验证过程中产生的真实 transcript 文件，对着写
   `transcript.rs` 的 `Codebuddy` 分支解析代码，而不是假设它跟 Claude 逐字段一致。
3. 如果 §6.1 两种方式都验证失败：CodeBuddy adapter 单独降级为"暂不支持"，不阻塞 OpenCode 独立
   交付（呼应 §2 的范围裁剪）。

## 7. dozer-app 消费侧改动

- `transcript.rs::parse_transcript` 签名从 `(jsonl: &str)` 改成 `(agent: AgentKind, jsonl: &str)`。
  `Opencode` 分支直接复用 `Claude` 分支代码（因为落盘格式一致）；`Codebuddy` 分支是独立解析代码，
  依赖 §6 的验证结果。
- `conversation.rs::claude_project_dir` 泛化成 `project_dir(agent: AgentKind, cwd: &Path) ->
  PathBuf`：Claude 用现有规则；CodeBuddy 换根目录 `~/.codebuddy/projects/`；OpenCode 指向
  `~/.dozer/agents/opencode/projects/`。
- `ConversationMeta.agent` 类型从 `String` 改成 `AgentKind`，与协议层保持一致。
- `list_conversations` 改成对同一个 cwd **依次扫三个 agent 的目录、合并、按 mtime 统一倒序**——
  历史侧栏展示"这个项目下所有对话"，不管当年用哪个 agent 跑的，每条记录自带 `agent` 标签用于
  展示图标/角标区分（默认行为选择，非用户明确要求，如需按 agent 过滤可后续调整）。

## 8. 错误处理与降级行为

- Agent 一直是 `Unknown`：正常状态，不特殊处理。
- `dozer-hook` 转发失败（dozerd 未运行/socket 不存在）：静默失败，绝不影响 agent CLI 本身运行——
  观测层从设计上不能反向拖累用户的编码会话。
- OpenCode 插件异常：见 §5.3"异常处理（硬性要求）"。
- 事件 schema 漂移（CodeBuddy/OpenCode 未来版本改字段名或事件类型）：翻译层对未知事件/字段一律
  忽略并跳过，不 panic、不阻塞。接受"要长期维护适配层"这个现实。
- CodeBuddy spike 失败：见 §6.3。

## 9. 测试策略

- `protocol.rs`：新增 `agent` 字段的 roundtrip 测试 + "老协议帧缺该字段回落 `Unknown`"测试，
  与现有 `old_session_info_without_agent_state_decodes_as_idle` 同一套路。
- `dozer-hook`：CodeBuddy/OpenCode 各自的事件名翻译表，纯函数表驱动测试（与 `install.rs` 现有
  测试风格一致）。
- `transcript.rs`：CodeBuddy 分支测试 fixture 依赖 §6.2 抓到的真实样本文件；OpenCode 分支因复用
  Claude 解析代码，靠现有测试覆盖，不新写。
- `conversation.rs`：`project_dir` 按 agent 分派的测试、多 agent 目录合并排序的测试。
- OpenCode 插件（TypeScript）：独立于 Rust 测试体系，用 opencode 插件生态自带的测试工具对"事件
  翻译状态机"做纯函数测试——喂一串模拟原生事件，断言吐出的规范事件序列正确。
- 人工验证清单（两个 spike 都需要，写进 plan 而非自动化测试）：真实装 hook/插件 → 跑真实会话 →
  确认 `SessionInfo.agent` 从 `Unknown` 正确翻转 → 确认 `transcript_path` 可解析 → 确认历史侧栏
  能看到这条对话。
