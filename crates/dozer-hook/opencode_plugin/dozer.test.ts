// `dozer.ts` 本身故意只有一个具名导出（`DozerPlugin`）——opencode 把
// `plugins/` 目录下每个文件的每个具名导出都当 Plugin 工厂调一次，多导出
// 一个 `resolveState`/`stateFor` 会被误当插件调用，产出运行时报错（跟
// `dozer-translate.ts` 必须塞进 `dozer-lib/` 子目录躲开扫描是同一条硬性
// 约束）。所以这里不直接 import 内部函数，走 `DozerPlugin` 整个工厂函数
// 拿到 hooks，用假的 `$`/`client` 驱动，只断言最终有没有 spawn
// `dozer-hook`（即 emit 到没到）。

import { describe, test, expect } from "bun:test"
import { DozerPlugin } from "./dozer"

type ShellCall = { values: any[] }

function makeShellStub() {
  const calls: ShellCall[] = []
  const $ = ((_strings: TemplateStringsArray, ...values: any[]) => {
    calls.push({ values })
    return { quiet: () => Promise.resolve() }
  }) as any
  return { $, calls }
}

function makeClientStub(sessions: Record<string, { directory: string; parentID?: string }>) {
  const getCalls: string[] = []
  return {
    getCalls,
    client: {
      session: {
        get: async ({ path }: { path: { id: string } }) => {
          getCalls.push(path.id)
          const info = sessions[path.id]
          if (!info) return { error: { name: "NotFoundError" } }
          return { data: { id: path.id, ...info } }
        },
      },
    },
  }
}

async function loadPlugin($: any, client: any) {
  return DozerPlugin({ $, client } as any)
}

describe("resume gap: 未经本进程 session.created 的会话", () => {
  test("session.idle 命中 client.session.get 兜底 → 照常 emit Stop", async () => {
    const { $, calls } = makeShellStub()
    const { client, getCalls } = makeClientStub({
      ses_resumed: { directory: "/private/tmp/proj" },
    })
    const plugin = await loadPlugin($, client)

    await plugin.event!({
      event: { type: "session.idle", properties: { sessionID: "ses_resumed" } },
    })

    expect(getCalls).toEqual(["ses_resumed"])
    expect(calls.length).toBe(1)
    expect(calls[0].values[2]).toBe("Stop") // emit($, {event: "Stop", ...}) 的第三个插值
  })

  test("message.part.updated 同样能兜底解析出 state → emit PreToolUse", async () => {
    const { $, calls } = makeShellStub()
    const { client } = makeClientStub({
      ses_resumed_2: { directory: "/private/tmp/proj" },
    })
    const plugin = await loadPlugin($, client)

    await plugin.event!({
      event: {
        type: "message.part.updated",
        properties: {
          part: {
            sessionID: "ses_resumed_2",
            type: "tool",
            callID: "call_1",
            tool: "read",
            state: { status: "running" },
          },
        },
      },
    })

    expect(calls.length).toBe(1)
    expect(calls[0].values[2]).toBe("PreToolUse")
  })

  test("子会话（parentID 非空）绝不能被兜底建 state，即使先到达的是别的事件", async () => {
    const { $, calls } = makeShellStub()
    const { client } = makeClientStub({
      ses_child: { directory: "/private/tmp/proj", parentID: "ses_root" },
    })
    const plugin = await loadPlugin($, client)

    await plugin.event!({
      event: { type: "session.idle", properties: { sessionID: "ses_child" } },
    })

    expect(calls.length).toBe(0)
  })

  test("session.get 查不到（已删除/未知 id）→ 静默丢弃，不抛出", async () => {
    const { $, calls } = makeShellStub()
    const { client } = makeClientStub({})
    const plugin = await loadPlugin($, client)

    await expect(
      plugin.event!({
        event: { type: "session.idle", properties: { sessionID: "ses_gone" } },
      })
    ).resolves.toBeUndefined()
    expect(calls.length).toBe(0)
  })

  test("已经在本进程见过（session.created）的会话不会再打 client.session.get", async () => {
    const { $, calls } = makeShellStub()
    const { client, getCalls } = makeClientStub({})
    const plugin = await loadPlugin($, client)

    await plugin.event!({
      event: {
        type: "session.created",
        properties: { info: { id: "ses_fresh", directory: "/private/tmp/proj" } },
      },
    })
    await plugin.event!({
      event: { type: "session.idle", properties: { sessionID: "ses_fresh" } },
    })

    expect(getCalls).toEqual([]) // session.created 走的是 stateFor，不经过 resolveState
    expect(calls.length).toBe(2) // SessionStart + Stop
    expect(calls[0].values[2]).toBe("SessionStart")
    expect(calls[1].values[2]).toBe("Stop")
  })

  test("chat.message hook 同样能兜底解析出 resume 会话的 state", async () => {
    const { $, calls } = makeShellStub()
    const { client } = makeClientStub({
      ses_resumed_3: { directory: "/private/tmp/proj" },
    })
    const plugin = await loadPlugin($, client)

    await plugin["chat.message"]!(
      { sessionID: "ses_resumed_3", messageID: "msg_1" },
      { message: { sessionID: "ses_resumed_3", role: "user", id: "msg_1" }, parts: [{ type: "text", text: "hi" }] }
    )

    expect(calls.length).toBe(1)
    expect(calls[0].values[2]).toBe("UserPromptSubmit")
  })

  test("chat.message 的 output.message.model 格式化成 providerID/modelID 写进 transcript 行", async () => {
    const { $, calls } = makeShellStub()
    const { client } = makeClientStub({
      ses_resumed_4: { directory: "/private/tmp/proj" },
    })
    const plugin = await loadPlugin($, client)

    await plugin["chat.message"]!(
      { sessionID: "ses_resumed_4", messageID: "msg_2" },
      {
        message: {
          sessionID: "ses_resumed_4",
          role: "user",
          id: "msg_2",
          model: { providerID: "litellm", modelID: "deepseek-v3" },
        },
        parts: [{ type: "text", text: "hi" }],
      }
    )

    expect(calls.length).toBe(1)
    const stdin = JSON.parse(calls[0].values[0])
    expect(stdin.transcript_line.message.model).toBe("litellm/deepseek-v3")
  })
})
