# OpenCode 插件（TypeScript）实现 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付真正跑在用户 opencode 进程里的 Dozer 插件——把 OpenCode 原生事件（`session.created`/`session.idle`/`chat.message`/`message.part.updated` 等）翻译成 Dozer 的 7 个规范事件之一，spawn `dozer-hook opencode <event>` 转发；`dozer-hook` 侧的落盘/转发逻辑（`crates/dozer-hook/src/opencode.rs`、`main.rs::forward`）在多 agent foundation 计划里已交付且有测试覆盖，本计划只补"没有车在跑"的那一端。

**Architecture:** 纯翻译逻辑（会话状态机、边沿检测、transcript 行构造）写成不碰 opencode SDK/不 spawn 子进程的纯函数模块 `dozer-translate.ts`，用 `bun test` 覆盖；插件入口 `dozer.ts` 只做"接 opencode 的 hook API → 调纯函数 → spawn dozer-hook"这层胶水，因为要连真实 opencode 进程无法自动化测试，靠 Task 4 的人工验证清单收尾。两个 `.ts` 文件通过 `include_str!` 嵌进 `dozer-hook` 二进制，新增的 `dozer-hook install/uninstall opencode` 子命令把它们写进 `~/.config/opencode/plugins/`（只放复数目录，单数 `plugin/` 是别的应用占用的，不碰）。

**Tech Stack:** TypeScript（Bun 运行时，`bun:test` 内置测试框架，opencode 自带）；Rust（`dozer-hook` crate，`include_str!` 嵌入静态资源，模式与现有 `install.rs` 的 `settings_path_for`/`DOZER_CLAUDE_SETTINGS` 环境变量覆盖手法一致）。

## Global Constraints

- 依据文档：`docs/superpowers/specs/2026-07-31-opencode-plugin-spike-findings.md`（字段名/事件形状/加载路径的唯一事实来源）与 `docs/superpowers/specs/2026-07-31-dozer-multi-agent-codebuddy-opencode-design.md` §5.3。任何本计划新增的字段假设必须能在这两份文档里找到依据，找不到的一律在代码注释里标注"未确认"并留给 Task 4 人工验证。
- **范围裁剪（design §2 已与用户核对，不得擅自扩大）**：只做观测层，不加"新建会话选 agent"的 UI，不改变"用户在 shell 里自己敲 opencode"的现状。
- **明确排除**：`Notification` 规范事件不在本计划实现范围——spike 文档自己承认"需进一步确认具体触发点（本次 spike 未深挖）"，没有确凿的 `permission.ask` 输入形状依据，写代码等于瞎猜。留空是诚实的降级，不是占位符；后续要做需要先补一次独立 spike。
- 插件文件只写入 `~/.config/opencode/plugins/`（复数）。绝不创建或碰 `~/.config/opencode/plugin/`（单数，是 kooky 应用托管的，spike 已确认双数目录会导致事件双发）。
- `dozer.ts`/`dozer-translate.ts` 不接入本仓库的 `tsc`/CI 类型检查流水线（用户机器上没有这个 repo 的 devDependencies，`@opencode-ai/plugin` 的类型导入是 `import type`，运行时被 Bun 直接抹掉，不影响执行）——正确性保证分两层：Task 1 的 `bun test` 覆盖纯翻译逻辑，Task 4 的人工验证覆盖胶水层与真实 opencode 集成。
- 所有 hook 回调必须整体包 `try/catch`，dozer 侧任何异常都不能抛到 opencode 主流程（design §5.3 硬性要求）。
- `crates/dozer-hook/src/opencode.rs`（`append_transcript_line`/`transcript_path`）与 `main.rs::forward` 里 `agent == AgentKind::Opencode` 的落盘分支是**多 agent foundation 计划已交付的既有代码**，本计划不修改，只是新增的 CLI 调用方（真实插件）会开始真正触发它们。

---

### Task 1: `dozer-translate.ts`——纯函数事件翻译状态机

**Files:**
- Create: `crates/dozer-hook/opencode_plugin/dozer-translate.ts`
- Test: `crates/dozer-hook/opencode_plugin/dozer-translate.test.ts`

**Interfaces:**
- Consumes: 无（纯 TS，不依赖仓库里任何既有符号）。
- Produces：给 Task 2 使用的具名导出——
  - `type CanonicalEvent = "SessionStart" | "PreToolUse" | "PostToolUse" | "UserPromptSubmit" | "Stop" | "SessionEnd"`
  - `interface TranslatedEvent { event: CanonicalEvent; cwd: string; transcriptLine?: TranscriptLine }`
  - `interface SessionState { ... }`（内部字段，Task 2 只把它当不透明句柄传递，不直接读字段）
  - `createSessionState(cwd: string): SessionState`
  - `onSessionCreated(state, info: {id, parentID?, directory}): TranslatedEvent | null`
  - `onUserMessage(state, parts: Array<{type, text?}>): TranslatedEvent`
  - `onToolPartUpdated(state, part: {callID, tool, state: {status, input?, output?, title?, error?}}): TranslatedEvent | null`
  - `onTextPartUpdated(state, part: {id, text}): void`
  - `onSessionIdle(state): TranslatedEvent`
  - `onSessionDeleted(state): TranslatedEvent`

- [ ] **Step 1: 确认 bun 可用**

Run: `bun --version`
Expected: 打印版本号（spike 验证时用的是 opencode 自带的 bun runtime；若命令不存在，先按 `bun.sh` 官方指引安装，这是运行 opencode 本身的前置依赖，不是本计划引入的新依赖）。

- [ ] **Step 2: 写失败测试**

创建 `crates/dozer-hook/opencode_plugin/dozer-translate.test.ts`：

```typescript
import { describe, test, expect } from "bun:test"
import {
  createSessionState,
  onSessionCreated,
  onUserMessage,
  onToolPartUpdated,
  onTextPartUpdated,
  onSessionIdle,
  onSessionDeleted,
} from "./dozer-translate"

describe("onSessionCreated", () => {
  test("根会话首次到达 → SessionStart", () => {
    const state = createSessionState(".")
    const result = onSessionCreated(state, { id: "ses_1", directory: "/private/tmp" })
    expect(result).toEqual({ event: "SessionStart", cwd: "/private/tmp" })
  })

  test("子会话（带 parentID）不转发", () => {
    const state = createSessionState(".")
    const result = onSessionCreated(state, {
      id: "ses_2",
      parentID: "ses_1",
      directory: "/private/tmp",
    })
    expect(result).toBeNull()
  })

  test("重复到达只发一次", () => {
    const state = createSessionState(".")
    onSessionCreated(state, { id: "ses_1", directory: "/private/tmp" })
    const second = onSessionCreated(state, { id: "ses_1", directory: "/private/tmp" })
    expect(second).toBeNull()
  })
})

describe("onUserMessage", () => {
  test("拼接 text 类型的 parts，跳过其他类型", () => {
    const state = createSessionState("/private/tmp")
    const result = onUserMessage(state, [
      { type: "text", text: "第一段" },
      { type: "file" },
      { type: "text", text: "第二段" },
    ])
    expect(result).toEqual({
      event: "UserPromptSubmit",
      cwd: "/private/tmp",
      transcriptLine: {
        type: "user",
        message: { role: "user", content: "第一段\n第二段" },
      },
    })
  })
})

describe("onToolPartUpdated", () => {
  test("running 首次 → PreToolUse，带 tool_use transcript 行", () => {
    const state = createSessionState("/private/tmp")
    const result = onToolPartUpdated(state, {
      callID: "call_1",
      tool: "read",
      state: { status: "running", input: { filePath: "/tmp/x.txt" } },
    })
    expect(result).toEqual({
      event: "PreToolUse",
      cwd: "/private/tmp",
      transcriptLine: {
        type: "assistant",
        message: {
          role: "assistant",
          content: [{ type: "tool_use", name: "read", input: { filePath: "/tmp/x.txt" } }],
        },
      },
    })
  })

  test("同一 callID 的 running 只发一次（边沿检测）", () => {
    const state = createSessionState(".")
    onToolPartUpdated(state, { callID: "call_1", tool: "read", state: { status: "running" } })
    const second = onToolPartUpdated(state, {
      callID: "call_1",
      tool: "read",
      state: { status: "running" },
    })
    expect(second).toBeNull()
  })

  test("error → PostToolUse，tool_result 带 error 文本", () => {
    const state = createSessionState("/private/tmp")
    onToolPartUpdated(state, { callID: "call_1", tool: "read", state: { status: "running" } })
    const result = onToolPartUpdated(state, {
      callID: "call_1",
      tool: "read",
      state: { status: "error", error: "拒绝授权" },
    })
    expect(result).toEqual({
      event: "PostToolUse",
      cwd: "/private/tmp",
      transcriptLine: {
        type: "user",
        message: {
          role: "user",
          content: [{ type: "tool_result", tool_use_id: "call_1", content: "拒绝授权" }],
        },
      },
    })
  })

  test("同一 callID 的 completed/error 只发一次", () => {
    const state = createSessionState(".")
    onToolPartUpdated(state, { callID: "call_1", tool: "read", state: { status: "running" } })
    onToolPartUpdated(state, { callID: "call_1", tool: "read", state: { status: "completed" } })
    const second = onToolPartUpdated(state, {
      callID: "call_1",
      tool: "read",
      state: { status: "completed" },
    })
    expect(second).toBeNull()
  })
})

describe("onTextPartUpdated + onSessionIdle", () => {
  test("多个文本 part 按首次出现顺序拼接，后续更新覆盖同一 part 的内容", () => {
    const state = createSessionState("/private/tmp")
    onTextPartUpdated(state, { id: "prt_1", text: "第一句的部分内容" })
    onTextPartUpdated(state, { id: "prt_2", text: "第二个 part" })
    onTextPartUpdated(state, { id: "prt_1", text: "第一句的完整内容" }) // 覆盖，不是追加
    const result = onSessionIdle(state)
    expect(result).toEqual({
      event: "Stop",
      cwd: "/private/tmp",
      transcriptLine: {
        type: "assistant",
        message: {
          role: "assistant",
          content: [{ type: "text", text: "第一句的完整内容\n第二个 part" }],
        },
      },
    })
  })

  test("没有文本块时 Stop 不带 transcriptLine", () => {
    const state = createSessionState(".")
    const result = onSessionIdle(state)
    expect(result).toEqual({ event: "Stop", cwd: ".", transcriptLine: undefined })
  })

  test("Stop 之后缓冲被清空，下一回合不会带上上一回合的文本", () => {
    const state = createSessionState(".")
    onTextPartUpdated(state, { id: "prt_1", text: "第一回合" })
    onSessionIdle(state)
    onTextPartUpdated(state, { id: "prt_2", text: "第二回合" })
    const result = onSessionIdle(state)
    expect(result.transcriptLine?.message.content).toEqual([{ type: "text", text: "第二回合" }])
  })
})

describe("onSessionDeleted", () => {
  test("→ SessionEnd，不带 transcriptLine", () => {
    const state = createSessionState("/private/tmp")
    expect(onSessionDeleted(state)).toEqual({ event: "SessionEnd", cwd: "/private/tmp" })
  })
})
```

- [ ] **Step 3: 运行测试确认失败**

Run: `bun test crates/dozer-hook/opencode_plugin/dozer-translate.test.ts`
Expected: FAIL——`dozer-translate.ts` 还不存在，模块解析报错。

- [ ] **Step 4: 实现 `dozer-translate.ts`**

创建 `crates/dozer-hook/opencode_plugin/dozer-translate.ts`：

```typescript
// 纯函数的 OpenCode 原生事件 → Dozer 规范事件（7 个 Claude 事件名之一）
// 翻译状态机。不碰 opencode SDK client、不 spawn 子进程、不读
// process.env——所有副作用都在 dozer.ts 里做，这里只管"喂进来一个观察到
// 的变化，吐出一个（或没有）要转发给 dozer-hook 的规范事件 + 可选的
// transcript 行"。字段名与状态流转依据
// docs/superpowers/specs/2026-07-31-opencode-plugin-spike-findings.md
// 的实测记录。

export type CanonicalEvent =
  | "SessionStart"
  | "PreToolUse"
  | "PostToolUse"
  | "UserPromptSubmit"
  | "Stop"
  | "SessionEnd"

export interface TranscriptLine {
  type: "user" | "assistant"
  message: {
    role: "user" | "assistant"
    content: string | Array<Record<string, unknown>>
  }
}

export interface TranslatedEvent {
  event: CanonicalEvent
  cwd: string
  transcriptLine?: TranscriptLine
}

interface ToolCallState {
  preSent: boolean
  postSent: boolean
}

interface TextPart {
  order: number
  text: string
}

export interface SessionState {
  sessionStartSent: boolean
  cwd: string
  toolCalls: Map<string, ToolCallState>
  textParts: Map<string, TextPart>
  nextOrder: number
}

export function createSessionState(cwd: string): SessionState {
  return {
    sessionStartSent: false,
    cwd,
    toolCalls: new Map(),
    textParts: new Map(),
    nextOrder: 0,
  }
}

/**
 * `session.created` 事件。子会话（`info.parentID` 非空）与重复到达都不
 * 发——spike Step 3 确认子 agent 也会触发 session.created，只有根会话该
 * 转发。
 */
export function onSessionCreated(
  state: SessionState,
  info: { id: string; parentID?: string; directory: string }
): TranslatedEvent | null {
  if (info.parentID || state.sessionStartSent) return null
  state.sessionStartSent = true
  state.cwd = info.directory
  return { event: "SessionStart", cwd: state.cwd }
}

/**
 * `chat.message` hook，`role === "user"` 分支。`parts` 里 `type:"text"`
 * 的文本按出现顺序拼接成 prompt 原文，写成 Claude 形状的 user transcript
 * 行。
 */
export function onUserMessage(
  state: SessionState,
  parts: Array<{ type: string; text?: string }>
): TranslatedEvent {
  const text = parts
    .filter((p) => p.type === "text" && typeof p.text === "string")
    .map((p) => p.text as string)
    .join("\n")
  return {
    event: "UserPromptSubmit",
    cwd: state.cwd,
    transcriptLine: {
      type: "user",
      message: { role: "user", content: text },
    },
  }
}

/**
 * `message.part.updated`，`part.type === "tool"`。边沿检测：同一
 * `callID` 的 PreToolUse/PostToolUse 各只发一次。`status` 首次进入
 * `"running"` → PreToolUse（连带一行只含 tool_use 块的 assistant
 * transcript 行）；`status` 转入 `"completed"`/`"error"` → PostToolUse
 * （连带一行 tool_result 的 user transcript 行）。
 *
 * 已知不确定项：`completed`（成功）状态下工具输出具体落在 `state` 的
 * 哪个字段，spike 未观测到成功样本（只观测到 `pending`/`running`/
 * `error`）。这里按 `output`/`title` 兜底尝试，真实字段名需要 Task 4
 * 人工验证时核对，核对结果如与此不符，回来改这一处，不影响其余逻辑。
 */
export function onToolPartUpdated(
  state: SessionState,
  part: {
    callID: string
    tool: string
    state: {
      status: string
      input?: Record<string, unknown>
      output?: string
      title?: string
      error?: string
    }
  }
): TranslatedEvent | null {
  let call = state.toolCalls.get(part.callID)
  if (!call) {
    call = { preSent: false, postSent: false }
    state.toolCalls.set(part.callID, call)
  }
  if (part.state.status === "running" && !call.preSent) {
    call.preSent = true
    return {
      event: "PreToolUse",
      cwd: state.cwd,
      transcriptLine: {
        type: "assistant",
        message: {
          role: "assistant",
          content: [{ type: "tool_use", name: part.tool, input: part.state.input ?? {} }],
        },
      },
    }
  }
  if ((part.state.status === "completed" || part.state.status === "error") && !call.postSent) {
    call.postSent = true
    const content =
      part.state.status === "error"
        ? part.state.error ?? ""
        : part.state.output ?? part.state.title ?? ""
    return {
      event: "PostToolUse",
      cwd: state.cwd,
      transcriptLine: {
        type: "user",
        message: {
          role: "user",
          content: [{ type: "tool_result", tool_use_id: part.callID, content }],
        },
      },
    }
  }
  return null
}

/**
 * `message.part.updated`，`part.type === "text"`。假定每次更新携带的是
 * "截至目前的完整文本"（覆盖写，不是增量 delta）——依据：spike 观测到
 * 独立的 `message.part.delta` 事件类型（未被本插件订阅），暗示
 * `part.updated` 携带的是全量快照，工具 part 的 `state.input` 同样是
 * 全量而非增量。Task 4 人工验证需实测确认；如证实是增量，把这里的
 * "覆盖写"改成"追加"即可，`onSessionIdle` 的拼接逻辑不用变。
 */
export function onTextPartUpdated(state: SessionState, part: { id: string; text: string }): void {
  const existing = state.textParts.get(part.id)
  if (existing) {
    existing.text = part.text
  } else {
    state.textParts.set(part.id, { order: state.nextOrder++, text: part.text })
  }
}

/**
 * `session.idle` 事件 → Stop。把本回合累积的文本块按首次出现顺序拼成
 * 一行 assistant transcript（没有文本块时不带 transcriptLine，只发状态
 * 转换），然后清空缓冲——下一回合的文本不该跟这一回合的拼在一起。
 */
export function onSessionIdle(state: SessionState): TranslatedEvent {
  const parts = [...state.textParts.entries()]
    .sort((a, b) => a[1].order - b[1].order)
    .map(([, v]) => v.text)
  state.textParts.clear()
  state.nextOrder = 0
  const text = parts.join("\n")
  return {
    event: "Stop",
    cwd: state.cwd,
    transcriptLine: text
      ? { type: "assistant", message: { role: "assistant", content: [{ type: "text", text }] } }
      : undefined,
  }
}

/** `session.deleted` 事件 → SessionEnd。不带 transcript 行。 */
export function onSessionDeleted(state: SessionState): TranslatedEvent {
  return { event: "SessionEnd", cwd: state.cwd }
}
```

- [ ] **Step 5: 运行测试确认通过**

Run: `bun test crates/dozer-hook/opencode_plugin/dozer-translate.test.ts`
Expected: PASS，全部 12 个测试绿。

- [ ] **Step 6: 提交**

```bash
git add crates/dozer-hook/opencode_plugin/dozer-translate.ts crates/dozer-hook/opencode_plugin/dozer-translate.test.ts
git commit -m "feat(opencode-plugin): pure event-translation state machine + bun tests"
```

---

### Task 2: `dozer.ts`——插件入口（胶水层）

**Files:**
- Create: `crates/dozer-hook/opencode_plugin/dozer.ts`

**Interfaces:**
- Consumes: Task 1 的全部具名导出（`createSessionState`/`onSessionCreated`/`onUserMessage`/`onToolPartUpdated`/`onTextPartUpdated`/`onSessionIdle`/`onSessionDeleted`/`type SessionState`/`type TranslatedEvent`）。
- Produces: `export const DozerPlugin: Plugin`，供 opencode 插件加载器识别（spike Step 2 确认的具名导出形状）。`HOOK_BIN_PLACEHOLDER` 字符串常量 `"__DOZER_HOOK_BIN_PATH__"`——Task 3 的 Rust 安装器要在写文件前把它替换成真实 exe 路径，字符串必须逐字节一致。

- [ ] **Step 1: 实现**

创建 `crates/dozer-hook/opencode_plugin/dozer.ts`：

```typescript
// Dozer 的 OpenCode 插件入口。只做"翻译 + spawn"——把 opencode 原生事件
// 交给 dozer-translate.ts 的纯函数翻成 Dozer 规范事件，然后 spawn 一次
// `dozer-hook opencode <event>`，JSON 走 stdin（与 Claude Code hook 机制
// 同构，socket 连接/协议编码全部留在 Rust 里复用）。
//
// 硬性要求（design 2026-07-31-dozer-multi-agent-codebuddy-opencode-design.md
// §5.3）：插件跑在用户的 opencode 进程内部，任何 dozer 侧故障都不能拖垮
// 或拖慢用户的会话——每个 hook 回调整体包 try/catch，吞掉一切异常。
//
// hookBin 在安装时（crates/dozer-hook/src/opencode_install.rs）被替换成
// dozer-hook 二进制的绝对路径；DOZER_HOOK_BIN 环境变量可覆盖，供开发/
// 测试用。

import type { Plugin } from "@opencode-ai/plugin"
import {
  createSessionState,
  onSessionCreated,
  onUserMessage,
  onToolPartUpdated,
  onTextPartUpdated,
  onSessionIdle,
  onSessionDeleted,
  type SessionState,
  type TranslatedEvent,
} from "./dozer-translate"

const HOOK_BIN_PLACEHOLDER = "__DOZER_HOOK_BIN_PATH__"
const hookBin = process.env.DOZER_HOOK_BIN || HOOK_BIN_PLACEHOLDER

const sessions = new Map<string, SessionState>()

function stateFor(sessionID: string, cwd: string): SessionState {
  let s = sessions.get(sessionID)
  if (!s) {
    s = createSessionState(cwd)
    sessions.set(sessionID, s)
  }
  return s
}

// `$` 是 opencode 插件传入的 Bun shell 标签函数，故意不精确标类型——
// 这个文件不接入 tsc 检查（见 plan Global Constraints），标注是给人看的
// 文档，不是类型安全保证。
async function emit($: any, translated: TranslatedEvent | null): Promise<void> {
  if (!translated) return
  const stdin = JSON.stringify({
    cwd: translated.cwd,
    transcript_line: translated.transcriptLine ?? null,
  })
  // `echo ... | hookBin opencode <event>`：真实 shell 管道，把 stdin 喂给
  // dozer-hook——Bun `$` 的插值会自动给 stdin 加引号转义成单个 echo 参数。
  await $`echo ${stdin} | ${hookBin} opencode ${translated.event}`
}

export const DozerPlugin: Plugin = async ({ $ }) => {
  return {
    event: async ({ event }: { event: { type: string; properties?: Record<string, any> } }) => {
      try {
        switch (event.type) {
          case "session.created": {
            const info = event.properties?.info
            if (!info?.id) return
            const state = stateFor(info.id, info.directory ?? ".")
            await emit($, onSessionCreated(state, info))
            return
          }
          case "session.idle": {
            const sessionID = event.properties?.sessionID
            const state = sessionID ? sessions.get(sessionID) : undefined
            if (!state) return
            await emit($, onSessionIdle(state))
            return
          }
          case "session.deleted": {
            const sessionID = event.properties?.sessionID
            const state = sessionID ? sessions.get(sessionID) : undefined
            if (!state) return
            await emit($, onSessionDeleted(state))
            sessions.delete(sessionID)
            return
          }
          case "message.part.updated": {
            const part = event.properties?.part
            const sessionID = event.properties?.sessionID
            const state = sessionID ? sessions.get(sessionID) : undefined
            if (!state || !part) return
            if (part.type === "tool") {
              await emit($, onToolPartUpdated(state, part))
            } else if (part.type === "text" && typeof part.text === "string") {
              onTextPartUpdated(state, part)
            }
            return
          }
          default:
            return
        }
      } catch {
        // 任何翻译/转发失败都吞掉，绝不抛到 opencode 主流程。
      }
    },
    "chat.message": async (
      _input: unknown,
      output: {
        message?: { sessionID?: string; role?: string }
        parts?: Array<{ type: string; text?: string }>
      }
    ) => {
      try {
        const sessionID = output?.message?.sessionID
        if (!sessionID || output?.message?.role !== "user") return
        const state = sessions.get(sessionID)
        if (!state) return
        await emit($, onUserMessage(state, output.parts ?? []))
      } catch {
        // 同上。
      }
    },
  }
}
```

- [ ] **Step 2: 静态检查（无 tsc，用 bun 语法检查代替）**

Run: `bun build crates/dozer-hook/opencode_plugin/dozer.ts --target=bun --outfile=/dev/null`
Expected: 无语法错误退出（`import type { Plugin } from "@opencode-ai/plugin"` 在没有该包时 bun 的 bundler 可能报"找不到模块"——若报这个错，改用 `// @ts-ignore` 风格注释跳过或直接删掉该 type-only import 改成 `type Plugin = any`，因为它只在类型层面使用、不影响运行时；本步骤的目的是抓真正的语法错误，不是强求这个 import 能 resolve）。

- [ ] **Step 3: 提交**

```bash
git add crates/dozer-hook/opencode_plugin/dozer.ts
git commit -m "feat(opencode-plugin): plugin entry wiring translate state machine to opencode hook API"
```

---

### Task 3: `dozer-hook install/uninstall opencode`——Rust 安装器

**Files:**
- Create: `crates/dozer-hook/src/opencode_install.rs`
- Modify: `crates/dozer-hook/src/main.rs:1-19`（新增 `mod opencode_install;`，`install`/`uninstall` 分派加 opencode 分支）
- Test: `crates/dozer-hook/src/opencode_install.rs`（同文件内 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: Task 1/2 产出的 `crates/dozer-hook/opencode_plugin/dozer.ts`、`crates/dozer-hook/opencode_plugin/dozer-translate.ts`（通过 `include_str!` 在编译期嵌入，路径必须与 Task 1/2 实际创建的文件路径完全一致）；`dozer.ts` 里的 `HOOK_BIN_PLACEHOLDER` 字符串字面量 `"__DOZER_HOOK_BIN_PATH__"` 必须与本任务的 `HOOK_BIN_PLACEHOLDER` 常量逐字节一致。
- Produces: `pub fn plugins_dir() -> PathBuf`、`pub fn run_at(dir: &Path, install: bool) -> i32`，供 `main.rs` 调用。

- [ ] **Step 1: 写失败测试**

创建 `crates/dozer-hook/src/opencode_install.rs`，先写 `#[cfg(test)] mod tests` 部分（函数体先留 `todo!()`，下一步再补，这里的"失败"是编译失败，属于 TDD 正常节奏，不是"占位符"——两步之间不提交）：

```rust
use std::path::{Path, PathBuf};

pub fn plugins_dir() -> PathBuf {
    todo!()
}

pub fn run_at(_dir: &Path, _install: bool) -> i32 {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_writes_both_files_with_exe_path_substituted() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run_at(dir.path(), true), 0);
        let dozer_ts = std::fs::read_to_string(dir.path().join("dozer.ts")).unwrap();
        assert!(
            !dozer_ts.contains("__DOZER_HOOK_BIN_PATH__"),
            "占位符必须被替换成真实 exe 路径"
        );
        assert!(dozer_ts.contains("DozerPlugin"));
        let translate_ts =
            std::fs::read_to_string(dir.path().join("dozer-translate.ts")).unwrap();
        assert!(translate_ts.contains("onSessionCreated"));
    }

    #[test]
    fn install_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run_at(dir.path(), true), 0);
        assert_eq!(run_at(dir.path(), true), 0);
        assert!(dir.path().join("dozer.ts").exists());
    }

    #[test]
    fn uninstall_removes_both_files() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run_at(dir.path(), true), 0);
        assert_eq!(run_at(dir.path(), false), 0);
        assert!(!dir.path().join("dozer.ts").exists());
        assert!(!dir.path().join("dozer-translate.ts").exists());
    }

    #[test]
    fn uninstall_on_missing_dir_does_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("not-created-yet");
        assert_eq!(run_at(&nested, false), 0);
    }

    #[test]
    fn plugins_dir_honors_env_override() {
        unsafe { std::env::set_var("DOZER_OPENCODE_PLUGIN_DIR", "/tmp/probe-opencode-plugins") };
        assert_eq!(plugins_dir(), PathBuf::from("/tmp/probe-opencode-plugins"));
        unsafe { std::env::remove_var("DOZER_OPENCODE_PLUGIN_DIR") };
    }
}
```

添加 `mod opencode_install;` 到 `crates/dozer-hook/src/main.rs` 第 1-3 行（跟现有 `mod codebuddy; mod install; mod opencode;` 并列，按字母序插入）。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-hook opencode_install:: -- --nocapture`
Expected: FAIL/panic（`todo!()` 触发 panic），确认测试确实在跑这份新代码。

- [ ] **Step 3: 实现**

把 `crates/dozer-hook/src/opencode_install.rs` 顶部（`todo!()` 那两个函数）替换成：

```rust
//! OpenCode 插件安装：把 `dozer.ts`/`dozer-translate.ts` 写进
//! `~/.config/opencode/plugins/`（只放复数目录——spike
//! `2026-07-31-opencode-plugin-spike-findings.md` Step 2 实测确认单数
//! `plugin/` 目录会导致同一事件被同时加载两次触发两次，后续代码不碰
//! 它）。跟 Claude/CodeBuddy 的 JSON 增量合并不同，这两个文件是 Dozer
//! 独占的文件名，不与用户/其他工具共享同一份配置——install 直接整体
//! 覆盖写，uninstall 直接删，不需要合并逻辑。
//!
//! `dozer.ts` 里 spawn `dozer-hook` 用的二进制路径在安装时被替换成
//! `current_exe()` 的绝对路径（跟 Claude/CodeBuddy 安装器把 exe 路径写
//! 进 hook command 字符串是同一手法），避免依赖插件运行时的 PATH。

use std::path::{Path, PathBuf};

const DOZER_TS_TEMPLATE: &str = include_str!("../opencode_plugin/dozer.ts");
const TRANSLATE_TS: &str = include_str!("../opencode_plugin/dozer-translate.ts");
const HOOK_BIN_PLACEHOLDER: &str = "__DOZER_HOOK_BIN_PATH__";

/// 插件目录路径，`DOZER_OPENCODE_PLUGIN_DIR` 覆盖用于测试（跟
/// `install.rs` 的 `DOZER_CLAUDE_SETTINGS`/`DOZER_CODEBUDDY_SETTINGS`
/// 同一套手法）。
pub fn plugins_dir() -> PathBuf {
    if let Ok(p) = std::env::var("DOZER_OPENCODE_PLUGIN_DIR") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    PathBuf::from(home)
        .join(".config")
        .join("opencode")
        .join("plugins")
}

pub fn run_at(dir: &Path, install: bool) -> i32 {
    if install {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("建目录失败: {e}");
            return 1;
        }
        let exe = std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "dozer-hook".into());
        let dozer_ts = DOZER_TS_TEMPLATE.replace(HOOK_BIN_PLACEHOLDER, &exe);
        if let Err(e) = std::fs::write(dir.join("dozer.ts"), dozer_ts) {
            eprintln!("写 dozer.ts 失败: {e}");
            return 1;
        }
        if let Err(e) = std::fs::write(dir.join("dozer-translate.ts"), TRANSLATE_TS) {
            eprintln!("写 dozer-translate.ts 失败: {e}");
            return 1;
        }
        println!("已安装: {}", dir.display());
    } else {
        for name in ["dozer.ts", "dozer-translate.ts"] {
            let p = dir.join(name);
            if p.exists()
                && let Err(e) = std::fs::remove_file(&p)
            {
                eprintln!("删 {} 失败: {e}", p.display());
                return 1;
            }
        }
        println!("已卸载: {}", dir.display());
    }
    0
}
```

在 `crates/dozer-hook/src/main.rs` 里，把第 12-19 行的 `install`/`uninstall` 分支改成：

```rust
        Some("install") => {
            let agent = std::env::args().nth(2).unwrap_or_else(|| "claude".into());
            if agent == "opencode" {
                std::process::exit(opencode_install::run_at(&opencode_install::plugins_dir(), true));
            }
            std::process::exit(install::run_at(&install::settings_path_for(&agent), &agent, true))
        }
        Some("uninstall") => {
            let agent = std::env::args().nth(2).unwrap_or_else(|| "claude".into());
            if agent == "opencode" {
                std::process::exit(opencode_install::run_at(&opencode_install::plugins_dir(), false));
            }
            std::process::exit(install::run_at(&install::settings_path_for(&agent), &agent, false))
        }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-hook -- --nocapture`
Expected: PASS，全量 `dozer-hook` 测试绿（含 `opencode_install::` 新增 5 个测试 + 既有全部测试无回归）。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-hook/src/opencode_install.rs crates/dozer-hook/src/main.rs
git commit -m "feat(dozer-hook): install/uninstall opencode plugin into ~/.config/opencode/plugins/"
```

---

### Task 4: 人工验证清单——真实 opencode 会话端到端

**Files:** 无代码改动，纯验证。

**Interfaces:**
- Consumes: Task 1-3 的全部产出。
- Produces: 无新符号；若验证发现 Task 1 里"已知不确定项"（工具成功输出字段名、文本 part 是否增量）与实测不符，回 Task 1 修正对应函数并重跑该 Task 的 `bun test`，不需要重开新任务。

> **注意（风险提示，执行前请确认）**：以下步骤会启动真实 opencode 会话、产生真实 LLM API 调用（消耗对应账号的额度）。安装目标特意选在**项目级插件目录**（scratch 临时目录下的 `.opencode/plugins/`），不写入用户真实的 `~/.config/opencode/plugins/`全局目录，避免影响用户日常 opencode 使用；确认要装到全局目录是后续用户自己决定的事，不在本计划自动执行范围内。

- [ ] **Step 1: 构建**

Run: `cargo build -p dozer-hook`
Expected: 编译成功，产出 `target/debug/dozer-hook`。

- [ ] **Step 2: 装进 scratch 项目的项目级插件目录**

```bash
mkdir -p /tmp/dozer-opencode-verify/.opencode/plugins
DOZER_OPENCODE_PLUGIN_DIR=/tmp/dozer-opencode-verify/.opencode/plugins \
  ./target/debug/dozer-hook install opencode
cat /tmp/dozer-opencode-verify/.opencode/plugins/dozer.ts | grep -c "__DOZER_HOOK_BIN_PATH__"
```
Expected: 装成功打印"已安装: ..."；`grep -c` 输出 `0`（占位符确实被替换掉了）。

- [ ] **Step 3: 跑一次真实会话**

```bash
cd /tmp/dozer-opencode-verify
DOZER_SESSION_ID=dozer-verify-1 opencode run "reply with exactly one word: hello"
```
Expected: 正常输出 `hello`（或近似），opencode 进程正常退出，没有因为插件报错而崩溃或挂起（这是 try/catch 覆盖是否到位的直接信号——如果这一步卡住或抛栈，说明某处 emit 调用没被正确 catch 住，回 Task 2 补）。

- [ ] **Step 4: 确认 transcript 文件被正确代写**

```bash
find ~/.dozer/agents/opencode/projects -name 'dozer-verify-1.jsonl'
cat "$(find ~/.dozer/agents/opencode/projects -name 'dozer-verify-1.jsonl')"
```
Expected: 文件存在；内容至少两行——一行 `{"type":"user","message":{"role":"user","content":"reply with exactly one word: hello"}}` 形状，一行 `{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hello"}]}}` 形状（若模型走了工具调用，中间还会有 `tool_use`/`tool_result` 行）。这份文件本身就是 `dozer-app/src/transcript.rs::parse_transcript(AgentKind::Opencode, ...)` 的真实输入——形状对得上，说明 Plan 1（多 agent foundation）已交付的解析器无需改动即可消费。

- [ ] **Step 5: 若观测到工具调用，核对 Task 1 标注的"已知不确定项"**

若 Step 3 的 prompt 换成会触发工具调用的内容（例如 `"list files in /tmp using ls"`），重复 Step 3-4，检查 transcript 里 `tool_result` 的 `content` 字段是否为空字符串——为空说明 `state.output`/`state.title` 猜测的字段名不对，需要额外打印实际的 `part.state` 全量 JSON（可在 `dozer.ts` 的 `message.part.updated` 分支临时加 `console.error(JSON.stringify(part))` 调试，确认后删掉），改 Task 1 的 `onToolPartUpdated` 里 `content` 那一行取正确字段名，重跑 `bun test`。

- [ ] **Step 6: 清理**

```bash
rm -rf /tmp/dozer-opencode-verify
```

---

## 完成检查

- [ ] `bun test crates/dozer-hook/opencode_plugin/dozer-translate.test.ts` 全绿（12 个用例覆盖 SessionStart 去重/子会话过滤、UserPromptSubmit 拼接、PreToolUse/PostToolUse 边沿检测各发一次、文本累积与回合边界清空、SessionEnd）。
- [ ] `cargo test -p dozer-hook` 全绿，含新增 `opencode_install::` 5 个测试。
- [ ] `dozer-hook install opencode` 把两个文件写进目标 `plugins/` 目录，`dozer.ts` 里的 hookBin 占位符被替换成真实绝对路径；`dozer-hook uninstall opencode` 能干净移除。
- [ ] Task 4 人工验证：真实 opencode 会话触发插件、transcript 文件被正确代写成 Claude 形状、opencode 进程本身不受任何 dozer 侧故障影响。
- [ ] 本计划范围明确排除（不是遗漏，是记录在案的后续项）：
  - `Notification` 规范事件——`permission.ask` 的精确触发语义未经验证，不实现。
  - 工具调用成功（`completed`）时输出内容的确切字段名——留给 Task 4 Step 5 现场核对，若与预设不符需回 Task 1 小改。
  - 同一 opencode 进程内"多 session 切换"的精确区分——design §5.3 已写明是已知局限，本计划不解决。
