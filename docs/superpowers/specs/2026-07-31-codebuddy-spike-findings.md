# CodeBuddy Hook 注册机制与 Transcript 落盘——Spike 验证记录

> 日期：2026-08-01
> 验证人：CodeBuddy CLI v2.127.3（macOS arm64）
> 关联：`docs/superpowers/specs/2026-07-31-dozer-multi-agent-codebuddy-opencode-design.md` §6
> 关联计划：`docs/superpowers/plans/2026-07-31-dozer-codebuddy-adapter.md` Task 1

## 结论一览

| 验证项 | 假设（spec §1/§6） | 实测 | 一致？ |
|---|---|---|---|
| hook 注册机制 | 全局 `~/.codebuddy/settings.json` 补丁（与 Claude 同构） | 命中 | ✅ |
| hook payload 字段名 | `session_id`/`transcript_path`/`cwd`/`hook_event_name`/`tool_name`/`tool_input`/`tool_response` 与 Claude 逐字对齐 | 全部命中 | ✅ |
| 事件名词汇 | `SessionStart`/`UserPromptSubmit`/`PreToolUse`/`PostToolUse`/`Notification`/`Stop`/`SessionEnd` 与 Claude 同族 | 命中前 6 个（`SessionEnd` 在 `-p` 模式未观测到，疑只在交互式退出时触发） | ✅（spec §5.2 翻译表无需调整） |
| transcript 落盘路径 | `~/.codebuddy/projects/<cwd-key>/<sessionId>.jsonl`，与 Claude 平行 | 命中 | ✅ |
| transcript schema | 与 Claude JSONL 逐字段一致？（spec §6.2 假设需验证） | **不一致**，独立 schema（见下） | ❌（不影响本计划；transcript 解析另开计划） |

**对计划的影响**：Task 4（安装器）按"全局 settings.json 补丁"设计直接落地，无需重写为 plugin 包机制。Task 2/3（事件名翻译表、forward 接入）按 spec §5.2 直接落地，事件名实测与假设一致。

## 验证步骤

### Step 1：CLI 可用性
`codebuddy --version` → `2.127.3`。`-p`/`--print` 提供非交互模式，`--dangerously-skip-permissions` 跳过权限提示，二者配合可在一个 shell 命令里跑完一轮会话触发 Stop。

### Step 2：全局 settings.json 补丁机制
1. 备份 `~/.codebuddy/settings.json`。
2. 用 `jq` 在 settings 顶层写入 `hooks` 段，为 7 个规范事件各注册一条 `command` 型 hook：`sh -c 'printf "\n===PROBE <ts>===\n" >> /tmp/codebuddy-hook-probe.log; cat >> /tmp/codebuddy-hook-probe.log'`（先写分隔行再 dump stdin）。
3. 跑 `cd /tmp && codebuddy -p --dangerously-skip-permissions "reply with exactly one word: hello"`。
4. 探针日志 `/tmp/codebuddy-hook-probe.log` 非空——hook **被触发**。
5. 还原 settings.json，确认 `has("hooks")` 回到 `false`。

> 注：settings.json 自带 `sandbox.filesystem.denyWrite` 把 `~/.codebuddy/settings.json` 列入禁写清单，但那只约束 agent 工具调用（Bash 等），**不影响用户从外部用 `dozer-hook install codebuddy` 改 settings**——安装器是用户主动发起的、在 agent 进程之外的 shell 里跑。

### Step 3：payload 字段实测

探针捕获到的字段（按事件类型）：

| 事件 | 出现字段 |
|---|---|
| `SessionStart` | `session_id`, `transcript_path`, `hook_event_name`, `source`(`"startup"`), `permission_mode`, `client`(`"CLI"`), `version`, `model` |
| `UserPromptSubmit` | `session_id`, `transcript_path`, `cwd`, `hook_event_name`, `prompt`, `permission_mode`, `client`, `version`, `model` |
| `PreToolUse` | `session_id`, `transcript_path`, `cwd`, `hook_event_name`, `tool_name`(`"Bash"`), `tool_input`(`{command, description, timeout}`), `call_id`, `tool_use_id`, `agent_type`, `permission_mode`, `client`, `version`, `generation_id`, `model` |
| `PostToolUse` | 同 PreToolUse，外加 `tool_response`(`{exitCode, signal, interrupted, sandboxDenied, stderrBytesTruncated, stdoutBytesTruncated, tool_error_code}`) |
| `Notification` | `transcript_path`(`""` 空串), `cwd`, `hook_event_name`, `message`(`"auth_success: <用户>"`), `notification_type`(`"auth_success"`), `title`, `permission_mode`, `client`, `version`, `model` |
| `Stop` | `session_id`, `transcript_path`, `hook_event_name`, `stop_hook_active`(`false`), `agent_type`, `last_assistant_message`, `background_tasks`(`[]`), `session_crons`(`[]`), `permission_mode`, `client`, `version`, `generation_id`, `model` |

**与 Claude 对齐的核心字段全部命中**：`session_id`/`transcript_path`/`cwd`/`hook_event_name`/`tool_name`/`tool_input`/`tool_response`。CodeBuddy 额外带 `call_id`/`tool_use_id`/`agent_type`/`permission_mode`/`client`/`version`/`generation_id`/`model`——这些是附加信息，不影响 dozer-hook 转发逻辑（dozer-hook 把整个 stdin JSON 当 `data` 透传给 dozerd）。

**已知差异（不阻塞本计划）**：
- `Notification` 事件在 CodeBuddy 里覆盖范围比 Claude 广——`auth_success`（鉴权成功）这类非"等待用户输入"的通知也走 `Notification`。dozerd 的 `agent_state_for("Notification")` 现有映射会把会话标成 `AwaitingInput`，对 `auth_success` 这类通知语义不精确。spec §8 已把"事件 schema 漂移"列为接受项，不在本计划修。
- `SessionEnd` 在 `-p` 模式未观测到。推测只在交互式会话退出时触发；不影响本计划（翻译表里 `SessionEnd` 是直通，触发不触发都不影响 dozer-hook 逻辑正确性）。
- spec §5.2 列出的 `PostToolUseFailure`/`StopFailure`/`SubagentStart`/`SubagentStop` 本次最小会话未触发，但 spec §5.2 翻译表已覆盖，翻译逻辑按 spec 实现。

### Step 4：transcript 样本

从子会话 `05e2e3c8-771f-4499-9e0a-4950cabd07c0`（`-p` 那次）抓到真实 transcript，已脱敏存为 `crates/dozer-hook/fixtures/codebuddy-transcript-sample.jsonl`（3 行：1 用户消息 + 1 file-history-snapshot + 1 assistant 消息）。

**transcript schema 实测**（与 Claude JSONL **不**逐字段一致）：

| 字段 | CodeBuddy | Claude（对照） |
|---|---|---|
| 顶层 `type` | `"message"` / `"file-history-snapshot"` | `"user"` / `"assistant"` |
| `role` | `"user"` / `"assistant"` | （无 role，靠 type 区分） |
| `content[].type` | `"input_text"` / `"output_text"` | `"text"` / `"tool_use"` / `"tool_result"` |
| 额外字段 | `parentId`, `providerData`, `sessionId`, `cwd`, `status` | `parentUuid`, `sessionId`, `cwd` |
| 消息条目之外 | 有 `file-history-snapshot` 条目 | 无 |

**结论**：`transcript.rs` 的 `AgentKind::Codebuddy` 分支需要独立解析代码，不能直接复用 Claude 分支。这印证了 spec §6.2 的预判。按本计划"完成检查"一节，真实解析另开一个小计划写，不在本计划范围——`transcript.rs` 里 `Codebuddy` 分支目前返回空 `Vec`（Plan 1 已实现的诚实降级），待拿到本 fixture 后另写。

## 对后续计划的影响

1. **Task 4 安装器**：按"全局 settings.json 补丁"路径直接实现，`settings_path_for("codebuddy")` 指向 `~/.codebuddy/settings.json`，`run_at` 的"读 JSON、增量合并、写回"逻辑与 Claude 完全同构。
2. **Task 2/3 翻译表**：spec §5.2 翻译表实测无需调整，按原文实现。
3. **transcript 解析（未来小计划）**：本 fixture 已入库，可作为解析器开发的输入。解析器需处理 `type:"message"` + `role` + `content[].type:"input_text"/"output_text"` 这套 schema，并跳过 `file-history-snapshot` 条目。
