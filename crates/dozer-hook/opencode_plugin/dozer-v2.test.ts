// v2（opencode 2.x）入口 `setup(ctx)` 的测试：假 `ctx` 喂 `{type, data}` 事件，
// 真 spawn 一个记录 argv/stdin 的桩 `dozer-hook`，断言规范事件。事件形状取自
// 对 opencode 2.1.2 的实测（text/step/execution）与 `@opencode/schema` 的
// session-event 定义（tool.*）。

import { describe, test, expect, beforeAll } from "bun:test"
import { mkdtempSync, writeFileSync, chmodSync, readFileSync, existsSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import Dozer from "./dozer"

const dir = mkdtempSync(join(tmpdir(), "dozer-v2-"))
const log = join(dir, "calls.jsonl")
const stub = join(dir, "hook.sh")

beforeAll(() => {
  writeFileSync(stub, `#!/bin/sh\nprintf '{"event":"%s","stdin":' "$2" >> "${log}"\ncat >> "${log}"\nprintf '}\\n' >> "${log}"\n`)
  chmodSync(stub, 0o755)
  process.env.DOZER_HOOK_BIN = stub
})

function calls(): Array<{ event: string; stdin: any }> {
  if (!existsSync(log)) return []
  // 桩分三次写一行，读到半行就跳过，下一轮 settle 再读。
  const out: Array<{ event: string; stdin: any }> = []
  for (const l of readFileSync(log, "utf8").split("\n")) {
    try {
      if (l) out.push(JSON.parse(l))
    } catch {}
  }
  return out
}

function makeCtx(sessions: Record<string, any>) {
  const queue: any[] = []
  let wake: (() => void) | undefined
  let prompt: ((e: any) => Promise<void> | void) | undefined
  const gets: string[] = []
  const ctx = {
    location: { directory: "/loc" },
    event: {
      subscribe: ({ signal }: { signal: AbortSignal }) => ({
        async *[Symbol.asyncIterator]() {
          while (!signal.aborted) {
            if (queue.length === 0) await new Promise<void>((r) => (wake = r))
            while (queue.length) yield queue.shift()
          }
        },
      }),
    },
    session: {
      get: async ({ sessionID }: { sessionID: string }) => {
        gets.push(sessionID)
        if (!sessions[sessionID]) throw new Error("not found")
        return { id: sessionID, ...sessions[sessionID] }
      },
      hook: async (_n: string, cb: any) => {
        prompt = cb
        return { dispose: async () => {} }
      },
    },
  }
  return {
    ctx,
    gets,
    push: (e: any) => {
      queue.push(e)
      wake?.()
    },
    prompt: (e: any) => prompt!(e),
  }
}

async function settle(n: number) {
  for (let i = 0; i < 100 && calls().length < n; i++) await new Promise((r) => setTimeout(r, 20))
}
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

describe("v2 setup(ctx)", () => {
  test("default export 有 id 与 setup（v2 加载校验）和 server（v1）", () => {
    expect(typeof Dozer.id).toBe("string")
    expect(typeof Dozer.setup).toBe("function")
    expect(typeof Dozer.server).toBe("function")
  })

  test("一轮完整流程：SessionStart → UserPromptSubmit → Pre/PostToolUse → Stop（带文本与用量）", async () => {
    const before = calls().length
    const h = makeCtx({ ses_a: { location: { directory: "/proj" }, model: { providerID: "litellm", id: "m1" } } })
    const cleanup = await Dozer.setup(h.ctx)

    await h.prompt({ sessionID: "ses_a", messageID: "msg_u", prompt: { text: "hi", files: [] }, delivery: "steer" })
    h.push({ type: "session.tool.input.started", data: { sessionID: "ses_a", assistantMessageID: "msg_1", id: "call_1", name: "read" } })
    h.push({ type: "session.tool.called", data: { sessionID: "ses_a", assistantMessageID: "msg_1", id: "call_1", input: { filePath: "a.txt" }, executed: true } })
    h.push({ type: "session.tool.success", data: { sessionID: "ses_a", assistantMessageID: "msg_1", id: "call_1", content: [{ type: "text", text: "hello" }], executed: true } })
    h.push({ type: "session.text.ended", data: { sessionID: "ses_a", assistantMessageID: "msg_1", ordinal: 0, text: "done" } })
    h.push({ type: "session.step.ended", data: { sessionID: "ses_a", assistantMessageID: "msg_1", tokens: { input: 10, output: 2, reasoning: 0, cache: { read: 1, write: 0 } } } })
    h.push({ type: "session.execution.succeeded", data: { sessionID: "ses_a" } })
    await settle(before + 5)

    const got = calls().slice(before)
    expect(got.map((c) => c.event)).toEqual(["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop"])
    expect(got[0].stdin.cwd).toBe("/proj")
    expect(got[1].stdin.transcript_line.message).toEqual({ role: "user", content: "hi", model: "litellm/m1" })
    expect(got[2].stdin.transcript_line.message.content[0]).toEqual({ type: "tool_use", name: "read", input: { filePath: "a.txt" } })
    expect(got[3].stdin.transcript_line.message.content[0]).toEqual({ type: "tool_result", tool_use_id: "call_1", content: "hello" })
    const stop = got[4].stdin.transcript_line.message
    expect(stop.content).toEqual([{ type: "text", text: "done" }])
    expect(stop.usage).toEqual({ input_tokens: 10, output_tokens: 2, cache_read_input_tokens: 1, cache_creation_input_tokens: 0 })
    cleanup()
  })

  test("tool.failed → PostToolUse 带错误信息；execution.failed 同样收尾成 Stop", async () => {
    const before = calls().length
    const h = makeCtx({ ses_b: { location: { directory: "/proj" } } })
    const cleanup = await Dozer.setup(h.ctx)
    h.push({ type: "session.tool.input.started", data: { sessionID: "ses_b", id: "call_x", name: "bash" } })
    h.push({ type: "session.tool.called", data: { sessionID: "ses_b", id: "call_x", input: {} } })
    h.push({ type: "session.tool.failed", data: { sessionID: "ses_b", id: "call_x", error: { type: "x", message: "boom" } } })
    h.push({ type: "session.execution.failed", data: { sessionID: "ses_b" } })
    await settle(before + 5)
    const got = calls().slice(before)
    expect(got.map((c) => c.event)).toEqual(["SessionStart", "PreToolUse", "PostToolUse", "Stop"])
    expect(got[2].stdin.transcript_line.message.content[0].content).toBe("boom")
    cleanup()
  })

  test("子会话（parentID 非空）：不发任何事件，且只查一次 session.get", async () => {
    const before = calls().length
    const h = makeCtx({ ses_child: { location: { directory: "/proj" }, parentID: "ses_root" } })
    const cleanup = await Dozer.setup(h.ctx)
    h.push({ type: "session.execution.succeeded", data: { sessionID: "ses_child" } })
    h.push({ type: "session.execution.succeeded", data: { sessionID: "ses_child" } })
    await sleep(200)
    expect(calls().length).toBe(before)
    expect(h.gets).toEqual(["ses_child"])
    cleanup()
  })

  test("session.get 失败 → 静默丢弃；prompt hook 不抛", async () => {
    const before = calls().length
    const h = makeCtx({})
    const cleanup = await Dozer.setup(h.ctx)
    await expect(h.prompt({ sessionID: "ses_gone", prompt: { text: "x" } })).resolves.toBeUndefined()
    h.push({ type: "session.execution.succeeded", data: { sessionID: "ses_gone" } })
    await sleep(200)
    expect(calls().length).toBe(before)
    cleanup()
  })

  test("session.created 直接建 state，之后不再 session.get；子会话的 created 被忽略", async () => {
    const before = calls().length
    const h = makeCtx({})
    const cleanup = await Dozer.setup(h.ctx)
    h.push({ type: "session.created", data: { sessionID: "ses_new", location: { directory: "/new" } } })
    h.push({ type: "session.created", data: { sessionID: "ses_kid", parentID: "ses_new", location: { directory: "/new" } } })
    h.push({ type: "session.execution.succeeded", data: { sessionID: "ses_kid" } })
    h.push({ type: "session.execution.succeeded", data: { sessionID: "ses_new" } })
    await settle(before + 2)
    const got = calls().slice(before)
    expect(got.map((c) => c.event)).toEqual(["SessionStart", "Stop"])
    expect(got[0].stdin.cwd).toBe("/new")
    expect(h.gets).toEqual([])
    cleanup()
  })

  test("session.deleted → SessionEnd", async () => {
    const before = calls().length
    const h = makeCtx({})
    const cleanup = await Dozer.setup(h.ctx)
    h.push({ type: "session.created", data: { sessionID: "ses_d", location: { directory: "/d" } } })
    h.push({ type: "session.deleted", data: { sessionID: "ses_d" } })
    await settle(before + 2)
    expect(calls().slice(before).map((c) => c.event)).toEqual(["SessionStart", "SessionEnd"])
    cleanup()
  })
})
