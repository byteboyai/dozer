// Dozer 的 OpenCode 插件入口。只做"翻译 + spawn"——把 opencode 原生事件
// 交给 dozer-translate.ts 的纯函数翻成 Dozer 规范事件，然后 spawn 一次
// `dozer-hook opencode <event>`，JSON 走 stdin（与 Claude Code hook 机制
// 同构，socket 连接/协议编码全部留在 Rust 里复用）。
//
// 两代 opencode 插件格式并存于同一个 default export：v2（2.x，`setup(ctx)` +
// `ctx.event.subscribe`，事件模型是 `session.text.*`/`session.tool.*`/
// `session.execution.*`）与 v1（`server()` 返回 `event`/`chat.message`
// hook，事件是 `session.idle`/`message.part.updated`）。两边都翻译进
// dozer-translate.ts 的同一组纯函数，Dozer 侧看到的规范事件不变。v2 不认
// 具名导出（"Plugin must export a default definition with an id and an
// effect or setup function"），v1 要 opencode >= 1.18.29 才认对象形式的
// default export；所以这里故意不再有具名导出。迁移规范见
// https://opencode.ai/v2/docs/build/plugins/migrate-v1/。
//
// 硬性要求（design 2026-07-31-dozer-multi-agent-codebuddy-opencode-design.md
// §5.3）：插件跑在用户的 opencode 进程内部，任何 dozer 侧故障都不能拖垮
// 或拖慢用户的会话——每个 hook 回调整体包 try/catch，吞掉一切异常。
//
// hookBin 在安装时（crates/dozer-hook/src/opencode_install.rs）被替换成
// dozer-hook 二进制的绝对路径；DOZER_HOOK_BIN 环境变量可覆盖，供开发/
// 测试用。

import { spawn } from "node:child_process"
import {
  createSessionState,
  onSessionCreated,
  onUserMessage,
  onToolPartUpdated,
  onTextPartUpdated,
  onMessageUpdated,
  onSessionIdle,
  onSessionDeleted,
  type SessionState,
  type TranslatedEvent,
} from "./dozer-translate"

// 安装时替换成产品名（`dozer`/`digger`…）：v2 用 id 做存储隔离与诊断，两个
// 产品的插件并存时不能撞 id。
const PLUGIN_ID = "__DOZER_PLUGIN_ID__"
const HOOK_BIN_PLACEHOLDER = "__DOZER_HOOK_BIN_PATH__"
// 每次取值而不是模块加载时固定：测试要在 import 之后才设 `DOZER_HOOK_BIN`。
const hookBin = () => process.env.DOZER_HOOK_BIN || HOOK_BIN_PLACEHOLDER

// `session.created` 是这个 Map 的常规填充路径，但用户 resume/continue
// 一个不是在本 opencode 进程里新建的会话时不会有 `session.created`——这
// 种情况下 `resolveState`（见下）会用 `client.session.get` 兜底补建，
// 不再让整段会话生命周期不可见（history：这里曾经只信 session.created，
// 见 git blame）。
const sessions = new Map<string, SessionState>()

// sessionID -> 已知的"用户消息" messageID 集合。`message.part.updated`
// 对用户 prompt 的文本 part 也会触发（不只是 assistant 回复），如果不
// 区分，用户自己敲的话会被当成 assistant 输出缓冲进 Stop 的 transcript
// 行里。在 `chat.message` hook（只在用户消息时触发）里记录 messageID，
// 转发文本 part 前拿 part.messageID 查一下这个集合来排除。独立于
// `SessionState`（Task 1 的纯状态类型，有测试覆盖）之外维护，避免给它
// 加一个胶水层专用、没测试覆盖的字段。
const userMessageIds = new Map<string, Set<string>>()

function stateFor(sessionID: string, cwd: string): SessionState {
  let s = sessions.get(sessionID)
  if (!s) {
    s = createSessionState(cwd)
    sessions.set(sessionID, s)
  }
  return s
}

// 给非 `session.created` 分支用的兜底查找：Map 里没有就查一次 opencode
// 的会话详情，补建 state。`client.session.get` 返回的 `Session` 跟
// `session.created` 事件里的 `info` 是同一个类型（两者共享
// `@opencode-ai/sdk` 的 `Session`），所以 `parentID` 过滤规则可以照抄
// `session.created` 分支——子/subagent 会话依然绝不能被建 state（否则
// 它自己的 session.idle 会被误当根会话的 Stop 转发出去，见文件顶部
// `session.created` 分支的同款注释）。会话已被删除/查询失败：跟历史行为
// 一致，静默丢弃，不是这次要补的缺口。
async function resolveState(
  client: { session: { get: (opts: { path: { id: string } }) => Promise<{ data?: { directory: string; parentID?: string } }> } },
  sessionID: string | undefined
): Promise<SessionState | undefined> {
  if (!sessionID) return undefined
  const existing = sessions.get(sessionID)
  if (existing) return existing
  try {
    const res = await client.session.get({ path: { id: sessionID } })
    const info = res?.data
    if (!info || info.parentID) return undefined
    return stateFor(sessionID, info.directory)
  } catch {
    return undefined
  }
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
  // `.quiet()`：Bun `$` 默认把子进程 stdout/stderr 转发到当前进程——
  // dozer-hook 失败时会 eprintln 中文错误信息，不加 `.quiet()` 会直接喷
  // 进用户的 opencode 终端。
  await $`echo ${stdin} | ${hookBin()} opencode ${translated.event}`.quiet()
}

// v1：`server()` 的返回值。
async function v1Hooks({ $, client }: { $: any; client: any }) {
  return {
    event: async ({ event }: { event: { type: string; properties?: Record<string, any> } }) => {
      try {
        switch (event.type) {
          case "session.created": {
            const info = event.properties?.info
            // parentID 非空 = 子/subagent 会话——在这里就拦掉，不能先建
            // state 再指望 onSessionCreated 内部的 parentID 检查兜底：
            // state 一旦进了 `sessions` map，子会话自己的后续事件（比如
            // 它自己的 session.idle）就会命中 map、被当成根会话转发，
            // 平白多出一次 Stop/TurnEnded，把 Dozer 的回合状态搞乱。
            if (!info?.id || info.parentID) return
            const state = stateFor(info.id, info.directory ?? ".")
            await emit($, onSessionCreated(state, info))
            return
          }
          case "session.idle": {
            const sessionID = event.properties?.sessionID
            const state = await resolveState(client, sessionID)
            if (!state) return
            await emit($, onSessionIdle(state))
            return
          }
          case "session.deleted": {
            // `session.deleted` 的 properties 实测（opencode 1.18.11）
            // 是 `{ info: Session }`，sessionID 走 `info.id`；同时保留对
            // 顶层 `sessionID` 的兜底，防止运行时/SDK 版本差异。
            const sessionID = event.properties?.info?.id ?? event.properties?.sessionID
            // 这个分支不能用 `resolveState` 兜底：会话此刻已经被删了，
            // `client.session.get` 大概率打空（404），永远建不出 state。
            // 一个从未被本进程见过就已经删除的会话，本来就没有意义可发
            // SessionEnd——跟历史行为一致，静默丢弃。
            const state = sessionID ? sessions.get(sessionID) : undefined
            if (!state) return
            await emit($, onSessionDeleted(state))
            sessions.delete(sessionID)
            userMessageIds.delete(sessionID)
            return
          }
          case "message.updated": {
            // `AssistantMessage.tokens`/`cost`(实测字段名见
            // `@opencode-ai/sdk` 的 `types.gen.d.ts`)——只在这个事件里
            // 出现，`message.part.updated` 没有。不 emit，只更新 state，
            // 等 `session.idle` 时随 Stop 一起转发(见 `onMessageUpdated`
            // 文档)。
            const info = event.properties?.info
            if (!info?.id || info.role !== "assistant") return
            const state = await resolveState(client, info.sessionID)
            if (!state) return
            onMessageUpdated(state, info)
            return
          }
          case "message.part.updated": {
            const part = event.properties?.part
            // 同上：`part.updated` 的 properties 实测是 `{ part, delta?
            // }`，sessionID 走 `part.sessionID`；顶层 `sessionID` 兜底。
            const sessionID = part?.sessionID ?? event.properties?.sessionID
            const state = await resolveState(client, sessionID)
            if (!state || !part) return
            if (part.type === "tool") {
              await emit($, onToolPartUpdated(state, part))
            } else if (part.type === "text" && typeof part.text === "string") {
              // 用户自己的 prompt 文本也会走这个分支（不只是 assistant
              // 回复）——如果不排除，会被误当成 assistant 输出缓冲进
              // Stop 的 transcript 行。`chat.message` hook 记录了已知的
              // 用户消息 messageID，这里查一下跳过。
              const knownUserMessageIds = userMessageIds.get(sessionID)
              if (part.messageID && knownUserMessageIds?.has(part.messageID)) return
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
    // `input` 类型依据 @opencode-ai/plugin 的 `Hooks["chat.message"]`
    // 签名核对（`~/.config/opencode/node_modules/@opencode-ai/plugin/
    // dist/index.d.ts`，对应实测运行时 opencode 1.18.11）：
    // `{ sessionID: string; messageID?: string; ... }`。`output.message`
    // 是 `UserMessage`，有保证非空的 `id`/`sessionID`/`role` 字段，用作
    // `input.messageID` 缺失时的兜底。`model` 字段（spike 实测，见
    // docs/superpowers/specs/2026-07-31-opencode-plugin-spike-findings.md
    // §Step 3）在 user 角色的消息上也带着，代表这条消息将由哪个模型处理——
    // 格式化成 `"providerID/modelID"` 传给 `onUserMessage`，写进 Dozer 侧
    // agent 卡片认得的 `message.model` 字段。
    "chat.message": async (
      input: { sessionID?: string; messageID?: string },
      output: {
        message?: {
          sessionID?: string
          role?: string
          id?: string
          model?: { providerID?: string; modelID?: string }
        }
        parts?: Array<{ type: string; text?: string }>
      }
    ) => {
      try {
        const sessionID = output?.message?.sessionID ?? input?.sessionID
        if (!sessionID || output?.message?.role !== "user") return
        // 记录这条用户消息的 messageID，供 `message.part.updated` 排除
        // 用户自己的文本 part（见 finding 2）。即使当前会话还没有
        // `SessionState`（比如 resume 场景），也先记下 messageID——万一
        // state 后面才出现，至少不会把旧数据当成新的漏判。
        const messageID = input?.messageID ?? output?.message?.id
        if (messageID) {
          let ids = userMessageIds.get(sessionID)
          if (!ids) {
            ids = new Set()
            userMessageIds.set(sessionID, ids)
          }
          ids.add(messageID)
        }
        const state = await resolveState(client, sessionID)
        if (!state) return
        const rawModel = output?.message?.model
        const model =
          rawModel?.providerID && rawModel?.modelID
            ? `${rawModel.providerID}/${rawModel.modelID}`
            : undefined
        await emit($, onUserMessage(state, output.parts ?? [], model))
      } catch {
        // 同上。
      }
    },
  }
}

// ---------------------------------------------------------------------------
// v2（opencode 2.x）
//
// 事件形状（`@opencode/schema` 的 session-event + 实测）：`{ type, data }`，
// `data.sessionID` 在几乎所有 session.* 事件里。没有 `session.idle`——回合
// 结束是 `session.execution.succeeded|failed|interrupted`；没有 `message.*`
// ——文本是 `session.text.ended`（带整段 `text`），工具是
// `session.tool.input.started`（唯一带工具名的）→ `called` → `success|failed`，
// 用量在 `session.step.ended`。`session.created` 只有会话在本插件已订阅后
// 才新建时才会到，CLI `run`/resume 的会话往往收不到，所以首个事件到达时
// 用 `ctx.session.get` 补建 state 并补发 SessionStart。
// ---------------------------------------------------------------------------

// v2 没有 Bun `$`，直接 spawn；stdin 约定与 v1 的 `echo … | hookBin` 相同。
// stdio 全丢弃：dozer-hook 失败时的 eprintln 不能喷进用户的 opencode 终端。
function emitSpawn(translated: TranslatedEvent | null): Promise<void> {
  if (!translated) return Promise.resolve()
  const stdin = JSON.stringify({
    cwd: translated.cwd,
    transcript_line: translated.transcriptLine ?? null,
  })
  return new Promise((resolve) => {
    try {
      const child = spawn(hookBin(), ["opencode", translated.event], {
        stdio: ["pipe", "ignore", "ignore"],
      })
      child.on("error", () => resolve())
      child.on("close", () => resolve())
      child.stdin.on("error", () => {})
      child.stdin.end(stdin)
    } catch {
      resolve()
    }
  })
}

// `session.tool.success` 的 `content` 是非空数组；已知元素带 `text`，其余
// 形状（图片/文件引用）没有可读文本，拼不出来就留空，不抛。
function v2ContentText(content: unknown): string {
  if (!Array.isArray(content)) return ""
  return content
    .map((c) => (c && typeof c.text === "string" ? c.text : ""))
    .filter((t) => t)
    .join("\n")
}

async function v2Setup(ctx: any): Promise<() => void> {
  // v2 的 state 与 v1 共用 `SessionState`，但各用各的 Map：同一个进程里
  // 只会跑其中一代。
  const states = new Map<string, SessionState>()
  // 子/subagent 会话与查不到的会话：记住，之后的事件直接丢，不再反复查。
  const ignored = new Set<string>()
  const toolNames = new Map<string, string>()
  const models = new Map<string, string>()
  const fallbackCwd: string = ctx.location?.directory ?? "."

  async function resolve(sessionID: unknown): Promise<SessionState | undefined> {
    if (typeof sessionID !== "string") return undefined
    const existing = states.get(sessionID)
    if (existing) return existing
    if (ignored.has(sessionID)) return undefined
    let info: any
    try {
      info = await ctx.session.get({ sessionID })
    } catch {
      return undefined
    }
    if (!info || info.parentID) {
      if (info) ignored.add(sessionID)
      return undefined
    }
    const state = createSessionState(info.location?.directory ?? fallbackCwd)
    states.set(sessionID, state)
    if (info.model?.providerID && info.model?.id) {
      models.set(sessionID, `${info.model.providerID}/${info.model.id}`)
    }
    await emitSpawn(onSessionCreated(state, { id: sessionID, directory: state.cwd }))
    return state
  }

  async function onEvent(event: { type: string; data?: Record<string, any> }): Promise<void> {
    const d = event.data ?? {}
    switch (event.type) {
      case "session.created": {
        if (!d.sessionID || d.parentID || states.has(d.sessionID)) {
          if (d.parentID && d.sessionID) ignored.add(d.sessionID)
          return
        }
        const state = createSessionState(d.location?.directory ?? fallbackCwd)
        states.set(d.sessionID, state)
        await emitSpawn(onSessionCreated(state, { id: d.sessionID, directory: state.cwd }))
        return
      }
      case "session.deleted": {
        const state = states.get(d.sessionID)
        if (!state) return
        await emitSpawn(onSessionDeleted(state))
        states.delete(d.sessionID)
        models.delete(d.sessionID)
        return
      }
      case "session.step.started": {
        if (d.model?.providerID && d.model?.id && typeof d.sessionID === "string") {
          models.set(d.sessionID, `${d.model.providerID}/${d.model.id}`)
        }
        return
      }
      case "session.text.ended": {
        const state = await resolve(d.sessionID)
        if (!state || typeof d.text !== "string") return
        onTextPartUpdated(state, { id: `${d.assistantMessageID}:${d.ordinal}`, text: d.text })
        return
      }
      case "session.step.ended": {
        const state = await resolve(d.sessionID)
        if (!state || !d.tokens) return
        onMessageUpdated(state, { id: d.assistantMessageID, role: "assistant", tokens: d.tokens })
        return
      }
      case "session.tool.input.started": {
        if (d.id && d.name) toolNames.set(d.id, d.name)
        return
      }
      case "session.tool.called": {
        const state = await resolve(d.sessionID)
        if (!state) return
        await emitSpawn(
          onToolPartUpdated(state, {
            callID: d.id,
            tool: toolNames.get(d.id) ?? "tool",
            state: { status: "running", input: d.input },
          })
        )
        return
      }
      case "session.tool.success":
      case "session.tool.failed": {
        const state = await resolve(d.sessionID)
        if (!state) return
        const failed = event.type === "session.tool.failed"
        await emitSpawn(
          onToolPartUpdated(state, {
            callID: d.id,
            tool: toolNames.get(d.id) ?? "tool",
            state: failed
              ? { status: "error", error: d.error?.message ?? "" }
              : { status: "completed", output: v2ContentText(d.content) },
          })
        )
        toolNames.delete(d.id)
        return
      }
      case "session.execution.succeeded":
      case "session.execution.failed":
      case "session.execution.interrupted": {
        const state = await resolve(d.sessionID)
        if (!state) return
        await emitSpawn(onSessionIdle(state))
        return
      }
      default:
        return
    }
  }

  const controller = new AbortController()
  // 串行消费：事件顺序就是 Dozer 状态机看到的顺序，不能并发处理。
  void (async () => {
    try {
      for await (const event of ctx.event.subscribe({ signal: controller.signal })) {
        try {
          await onEvent(event)
        } catch {
          // 任何翻译/转发失败都吞掉，绝不抛到 opencode 主流程。
        }
      }
    } catch {
      // 订阅被 abort 或流断开：静默结束。
    }
  })()

  // `prompt` hook 在提示被接收时跑一次，是 v1 `chat.message` 的对应物。
  // 只读不改 event，且必须吞掉一切异常——抛出会让用户的提示被拒。
  try {
    await ctx.session.hook("prompt", async (e: any) => {
      try {
        const state = await resolve(e?.sessionID)
        if (!state) return
        await emitSpawn(
          onUserMessage(state, [{ type: "text", text: e.prompt?.text }], models.get(e.sessionID))
        )
      } catch {
        // 同上。
      }
    })
  } catch {
    // hook 注册失败：只丢 UserPromptSubmit，事件订阅照常。
  }

  return () => controller.abort()
}

export default {
  id: PLUGIN_ID,
  setup: v2Setup,
  server: v1Hooks,
}
