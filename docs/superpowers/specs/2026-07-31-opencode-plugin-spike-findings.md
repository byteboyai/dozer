# OpenCode 插件 API 形状——Spike 验证记录

> 日期：2026-08-01
> 验证人：opencode CLI v1.18.3（macOS arm64，bun runtime）
> 关联：`docs/superpowers/specs/2026-07-31-dozer-multi-agent-codebuddy-opencode-design.md` §5.3、§6
> 关联计划：`docs/superpowers/plans/2026-07-31-dozer-opencode-adapter.md` Task 2
> 配套：Task 1 已落地 `crates/dozer-hook/src/opencode.rs`（`append_transcript_line`/`transcript_path`），本 spike 为后续 TS 插件实现计划提供事实依据。

## 结论一览

| spec §5.3 / §6 假设 | 实测 | 一致？ |
|---|---|---|
| 插件活在 opencode 进程内部 | ✓ 插件是 opencode 启动时加载的 TS/JS 模块，跑在同一进程 | ✅ |
| 插件天然继承父进程环境变量 | ✓ `process.env.DOZER_SESSION_ID` 在插件内可直接读 | ✅ |
| 插件 spawn `dozer-hook opencode <event>` | ✓ `$\`${bin} opencode ${state}\`` 可执行（Bun shell） | ✅ |
| 事件订阅 `session.*`/`message.*`/`part.*` | ✓ 全部可订阅，字段名见下 | ✅ |
| 翻译复杂度封在插件里，不泄漏 Rust 侧 | ✓ Task 1 的 `append_transcript_line` 只认 `transcript_line` 参数，不关心 OpenCode 事件结构 | ✅ |

**对后续插件计划的影响**：spec §5.3 的"翻译 + spawn"架构完全可行。插件用 `event` hook 收所有事件、按 `event.type` + `part.state.status` 边沿检测翻出 7 个规范事件，每次 spawn 一次 `dozer-hook opencode <event>`，stdin 带上 `{cwd, transcript_line}`。Task 1 的 Rust 侧落盘逻辑零改动可复用。

---

## Step 1：CLI 可用性

`opencode --version` → `1.18.3`。`opencode run [message..]` 提供非交互模式（等价于 codebuddy 的 `-p`），配合 `DOZER_SESSION_ID=... opencode run "..."` 可在一个 shell 命令里跑完一轮会话触发 `session.created` → `session.idle`。`--pure` 可禁用外部插件（验证插件确实在加载）。

## Step 2：插件加载位置 + manifest/导出形状

### 加载位置（两种，都会自动加载）

| 来源 | 目录 / 方式 | 备注 |
|---|---|---|
| 本地文件 | `~/.config/opencode/plugins/`（全局）或 `.opencode/plugins/`（项目级） | 启动时自动扫描，无需在 config 里声明 |
| npm 包 | `opencode.jsonc` 的 `plugin` 字段：`"plugin": ["pkg-name", ["pkg-with-opts", {...}]]` | 启动时 bun 自动安装到 `~/.cache/opencode/node_modules/` |

> **实测意外发现**：本机 v1.18.3 同时识别 `plugin/`（单数）和 `plugins/`（复数）目录——把同一份探针放两处，每个事件被触发两次。官方文档（opencode.ai/docs/plugins）写的是 `plugins/`（复数），后续插件计划应只放 `plugins/`，避免重复触发。仓库里现有的 `~/.config/opencode/plugin/kooky.ts`（单数，由 kooky 应用托管）能加载说明旧目录名也兼容，但新代码不应依赖它。

### 导出形状

```ts
// 本地 .ts 文件，默认导出或具名导出均可；这里用具名导出（与 SDK 示例一致）
import type { Plugin } from "@opencode-ai/plugin"

export const DozerPlugin: Plugin = async ({ $, client, directory, project, worktree, serverUrl }) => {
  // 初始化阶段（启动时跑一次）：可读 process.env、注册工具等
  return {
    // Hooks 对象——见 Step 3 的可用 hook 清单
  }
}
```

`PluginInput` 字段（来自 `@opencode-ai/plugin/dist/index.d.ts`）：
- `client`：opencode SDK client（`createOpencodeClient` 返回值，可 `client.session.get({ path:{id}, query:{directory} })` 等查询 session/消息/part）
- `project`：Project 信息
- `directory`：当前工作目录（cwd）
- `worktree`：git worktree 路径
- `serverUrl`：opencode 内部 server 的 URL
- `$`：Bun 的 [shell API](https://bun.com/docs/runtime/shell)（`$\`cmd args\`` 模板标签 spawn 子进程，`.quiet()` 静默）

### 插件依赖管理

`~/.config/opencode/package.json` 声明 npm 依赖（本机已有 `@opencode-ai/plugin@1.4.3`），opencode 启动时跑 `bun install`。TS 类型从 `@opencode-ai/plugin` 导入（`import type { Plugin } ...`）。

## Step 3：能订阅的事件 + 字段形状

### 可用 Hooks（来自 `@opencode-ai/plugin/dist/index.d.ts` 的 `Hooks` interface）

| Hook 名 | 用途 |
|---|---|
| `event` | **catch-all**，收到所有 `Event` 对象（`session.*`/`message.*`/`part.*`/`tool.*`/`file.*`/...），靠 `event.type` 分派。**Dozer 插件主用这个**。 |
| `"chat.message"` | 新消息到达时触发，带 `sessionID`/`agent`/`model`/`messageID` + `output:{ message, parts }` |
| `"chat.params"` / `"chat.headers"` | 改 LLM 请求参数/头 |
| `"permission.ask"` | 权限决策 hook |
| `"tool.execute.before"` / `"tool.execute.after"` | 工具执行前后（注意：实测这俩没作为 `event` 推送，是独立 hook） |
| `"command.execute.before"` | 命令执行前 |

### `event` hook 收到的事件对象形状

通用包络：`{ id: "evt_...", type: "<event-type>", properties: {...} }`。

#### `session.created`（→ 规范 SessionStart）
```json
{
  "type": "session.created",
  "properties": {
    "sessionID": "ses_04583e3e0ffe...",
    "info": {
      "id": "ses_04583e3e0ffe...",
      "slug": "mighty-lagoon",
      "version": "1.18.3",
      "projectID": "global",
      "directory": "/private/tmp",
      "path": "private/tmp",
      "title": "New session - 2026-07-31T23:22:03.680Z",
      "permission": [...],
      "cost": 0,
      "tokens": {"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}},
      "time": {"created":1785540123680,"updated":1785540123680}
    }
  }
}
```
- 会话 id 在 `properties.sessionID`（也等于 `properties.info.id`）
- cwd 在 `properties.info.directory`
- 子 agent 也会触发 `session.created`——靠 `properties.info.parentID` 是否存在区分根会话/子会话（kooky.ts 已验证此模式：`if (info?.id && !info?.parentID)` 才处理）

#### `session.idle`（→ 规范 Stop）
```json
{
  "type": "session.idle",
  "properties": { "sessionID": "ses_04583e3e0ffe..." }
}
```
- 只带 `sessionID`，无额外信息。assistant 消息完成 / session 转空闲时触发。

#### `chat.message` hook（→ 区分 UserPromptSubmit / assistant 回复）
input: `{ sessionID, agent?, model?, messageID?, variant? }`
output: `{ message: {...}, parts: [...] }`
```json
output.message = {
  "id": "msg_...",
  "role": "user",            // 或 "assistant"——靠这个字段区分
  "sessionID": "ses_...",
  "time": {"created": 1785540123816},
  "agent": "build",
  "model": {"providerID":"litellm","modelID":"deepseek-v3"}
}
output.parts[0] = {
  "type": "text",
  "text": "\"reply with exactly one word: hello\"",
  "messageID": "msg_...",
  "sessionID": "ses_...",
  "id": "prt_..."
}
```
- `message.role === "user"` → UserPromptSubmit
- `message.role === "assistant"` → 不直接映射规范事件（assistant 回复内容会逐步经 `message.part.updated` 推送，最终 `session.idle` 才是 Stop）

#### `message.part.updated`（→ PreToolUse / PostToolUse / 文本流）

**文本 part**（不需要为 Dozer 翻译，但要知道它存在）：
```json
{
  "type": "message.part.updated",
  "properties": {
    "sessionID": "ses_...",
    "part": {
      "type": "text",
      "text": "...",
      "messageID": "msg_...",
      "sessionID": "ses_...",
      "id": "prt_..."
    },
    "time": 1785540123947
  }
}
```

**工具 part 的完整生命周期**（实测状态流转 `pending` → `running` → `completed`/`error`）：

`pending`（刚创建，无 input）：
```json
"part": {
  "type": "tool",
  "tool": "read",
  "callID": "call_00_N7lqkiunVnUmePHlCf7H3768",
  "state": {"status": "pending", "input": {}, "raw": ""},
  "id": "prt_...", "sessionID": "ses_...", "messageID": "msg_..."
}
```

`running`（→ 规范 PreToolUse 边沿）：
```json
"part": {
  "type": "tool",
  "tool": "read",
  "callID": "call_00_...",
  "state": {
    "status": "running",
    "input": {"filePath": "/tmp/tool-trigger-marker.txt"},
    "time": {"start": 1785540170328}
  },
  "id": "prt_...", "sessionID": "ses_...", "messageID": "msg_..."
}
```
- `state.input` 是工具入参（read 工具是 `{filePath}`，bash 工具会是 `{command}` 等）
- **status 首次进入 `running` → 翻译成 PreToolUse**

`error`（→ 规范 PostToolUse 边沿；成功时是 `completed`）：
```json
"part": {
  "type": "tool",
  "tool": "read",
  "callID": "call_00_...",
  "state": {
    "status": "error",
    "input": {"filePath": "/tmp/tool-trigger-marker.txt"},
    "error": "The user rejected permission to use this specific tool call.",
    "time": {"start": 1785540170328, "end": 1785540170340}
  },
  "id": "prt_...", "sessionID": "ses_...", "messageID": "msg_..."
}
```
- **status 进入 `completed` 或 `error` → 翻译成 PostToolUse**（成功/失败归并，跟 spec §5.2 对 CodeBuddy 的处理同构）
- `time.end` 出现 = 工具调用彻底结束

#### 其他实测到的事件类型（Dozer 不直接用，记录备查）
`session.updated`、`session.status`、`session.diff`、`message.updated`、`message.part.delta`、`plugin.added`、`catalog.updated`、`integration.updated`、`reference.updated`。

### 规范事件翻译表（spec §5.3 实测可落地）

| OpenCode 观察 | 规范事件名 | 翻译逻辑 |
|---|---|---|
| `session.created`（且 `info.parentID` 为空，即根会话） | SessionStart | 首次出现即发 |
| `chat.message` 且 `message.role === "user"` | UserPromptSubmit | message hook 触发即发 |
| `message.part.updated`，`part.type==="tool"`，`state.status` 首次 `running` | PreToolUse | 边沿检测：同一 `callID` 只发一次 |
| `message.part.updated`，`part.type==="tool"`，`state.status` 转 `completed`/`error` | PostToolUse | 边沿检测：同一 `callID` 只发一次 |
| `permission.ask` / `session.status` 等待用户 | Notification | 需进一步确认具体触发点（本次 spike 未深挖） |
| `session.idle` | Stop | 触发即发 |
| `session.deleted` 或插件感知进程退出 | SessionEnd | `session.deleted` 事件存在；进程退出需在插件内做 finally 逻辑 |

## Step 4：环境变量继承——DOZER_SESSION_ID 可读

探针插件在顶层（模块加载时）和 `Plugin` 函数体内都打印了 `process.env.DOZER_SESSION_ID`：

```
===PROBE PLUGIN LOADED at 2026-07-31T23:22:03.615Z===
DOZER_SESSION_ID=probe-123
cwd=/private/tmp
```

启动命令 `DOZER_SESSION_ID=probe-123 opencode run "..."` 注入的环境变量，插件进程内**可直接读到**。spec §5.3 "插件天然继承父进程环境变量"这条假设**成立**——dozerd spawn PTY 时注入的 `DOZER_SESSION_ID` 会一路继承到 opencode 进程、再到插件。

> 实现提示：插件每次 spawn `dozer-hook opencode <event>` 时，`dozer-hook` 自己也读 `DOZER_SESSION_ID`（Plan 1 已实现），所以插件其实不用主动传 session_id——但 `transcript_line` 和 `cwd` 得在 stdin JSON 里带上（cwd 从 `directory` 字段拿，transcript_line 插件自己构造）。

## Step 5：能否从插件里 spawn 子进程——可以

探针在 `Plugin` 函数体内执行：
```ts
await $`echo spawn-ok >> /tmp/opencode-spawn-probe.log`.quiet()
```
结果 `/tmp/opencode-spawn-probe.log` 出现 `spawn-ok`。**`$`（Bun shell）在插件内可正常 spawn 子进程**，spec §5.3 "插件 spawn `dozer-hook opencode <event>`" 这条转发路径**完全成立**，不需要改用"插件直连 dozerd socket"的备选方案。

> 实现提示：`$\`${hookBin} opencode ${event}\`` 的参数插值由 Bun shell 负责转义；`hookBin` 从 `process.env` 读 dozer-hook 的路径（dozerd spawn PTY 时可一并注入 `DOZER_HOOK_BIN` 环境变量，或插件硬编码 `dozer-hook` 假定在 PATH 上）。stdin 的 JSON 需要手动 pipe——Bun shell 的 `$\`...\`` 默认不接 stdin，要用 `.stdin()` 或 `Bun.spawn` 显式传 stdin。这是后续插件实现计划要解决的最后一个工程细节，不影响 Task 1 已交付的 Rust 侧。

## 对后续插件实现计划的影响

1. **架构定案**：spec §5.3 的"插件内做事件边沿检测 → spawn `dozer-hook opencode <event>`"路径无需改动，按本 spike 的字段名直接实现。
2. **`transcript_line` 契约**（Task 1 已实现 `append_transcript_line`）：插件需要在 stdin JSON 里构造 Claude 格式的行：
   - user 消息：`{"type":"user","message":{"role":"user","content":"<prompt 文本>"}}`
   - assistant 文本：`{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"..."}]}}`
   - assistant 工具调用：`content` 数组里加 `{"type":"tool_use","name":"<tool>","input":{...}}`
   - 工具结果：`{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"<callID>","content":"..."}]}}`
   这些行会被 `dozer-app/src/transcript.rs::parse_transcript(AgentKind::Opencode, ...)`（Plan 1 已实现，复用 Claude 分支）正确解析。
3. **目录只放 `plugins/`**：避免单/复数目录双加载导致事件双发。
4. **边沿状态机**：插件需维护极小的每-session 本地状态——记录已发过 PreToolUse/PostToolUse 的 `callID` 集合，防止同一工具调用的 `message.part.updated` 重复触发。
5. **异常处理**：spec §5.3 硬性要求——所有插件逻辑整体包 try/catch，dozer 侧 bug 不能拖垮用户 opencode 会话。`$` spawn 已验证失败可被 `try { await $\`...\`.quiet() } catch {}` 吞掉。
