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

  test("带 model 时写进 message.model（providerID/modelID 格式）", () => {
    const state = createSessionState("/private/tmp")
    const result = onUserMessage(
      state,
      [{ type: "text", text: "你好" }],
      "litellm/deepseek-v3"
    )
    expect(result).toEqual({
      event: "UserPromptSubmit",
      cwd: "/private/tmp",
      transcriptLine: {
        type: "user",
        message: { role: "user", content: "你好", model: "litellm/deepseek-v3" },
      },
    })
  })

  test("不带 model 时 message 里不出现 model 字段（不写 undefined）", () => {
    const state = createSessionState("/private/tmp")
    const result = onUserMessage(state, [{ type: "text", text: "你好" }])
    expect(result?.transcriptLine?.message).not.toHaveProperty("model")
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
