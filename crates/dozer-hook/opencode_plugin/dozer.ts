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
  await $`echo ${stdin} | ${hookBin} opencode ${translated.event}`.quiet()
}

export const DozerPlugin: Plugin = async ({ $, client }) => {
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
    // `input.messageID` 缺失时的兜底。
    "chat.message": async (
      input: { sessionID?: string; messageID?: string },
      output: {
        message?: { sessionID?: string; role?: string; id?: string }
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
        await emit($, onUserMessage(state, output.parts ?? []))
      } catch {
        // 同上。
      }
    },
  }
}
