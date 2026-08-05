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
