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
    /** `"providerID/modelID"`——Dozer 侧 `latest_model_mode_and_activity`
     * 认 `message.model` 这个字段名(Claude 真实 transcript 的原生形状),
     * 不看 `type`/`role`，同一路径直接复用，不需要 Dozer 那边加分支。 */
    model?: string
    /** Claude 真实 transcript 的 usage 形状(字段名不能改——
     * `dozerd::transcripts::parse::parse_claude_shaped_chunk` 就是照这
     * 几个字段名读的,OpenCode 走同一条 Claude 形状解析路径,没有为它
     * 单独加分支)。此前这个插件完全没写这个字段,导致用量面板 OpenCode
     * 那栏永远是 0(验收反馈)。 */
    usage?: {
      input_tokens: number
      output_tokens: number
      cache_read_input_tokens: number
      cache_creation_input_tokens: number
    }
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

interface MessageUsage {
  inputTokens: number
  outputTokens: number
  cacheReadTokens: number
  cacheWriteTokens: number
}

export interface SessionState {
  sessionStartSent: boolean
  cwd: string
  toolCalls: Map<string, ToolCallState>
  textParts: Map<string, TextPart>
  nextOrder: number
  /** 按 assistant message id 记的最新一次用量快照(`message.updated` 可能
   * 对同一条消息触发多次,流式增量地补全 tokens——只按 id 覆盖写,不累加,
   * 避免同一条消息被算两次)。一回合(`session.idle` 之前)可能有多条
   * assistant 消息(多轮工具调用),`onSessionIdle` 把这里所有条目的
   * tokens 加总当作这一回合的总用量。 */
  usageByMessage: Map<string, MessageUsage>
}

export function createSessionState(cwd: string): SessionState {
  return {
    sessionStartSent: false,
    cwd,
    toolCalls: new Map(),
    textParts: new Map(),
    nextOrder: 0,
    usageByMessage: new Map(),
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
 *
 * `model`（可选）来自同一个 hook 的 `output.message.model`
 * （`{providerID, modelID}`，spike 实测两个字段都在——见
 * docs/superpowers/specs/2026-07-31-opencode-plugin-spike-findings.md
 * §Step 3），调用方（`dozer.ts`）已经格式化成 `"providerID/modelID"` 字符
 * 串传进来，这里只管原样写进 `message.model`，不做二次解析。
 */
export function onUserMessage(
  state: SessionState,
  parts: Array<{ type: string; text?: string }>,
  model?: string
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
      message: model
        ? { role: "user", content: text, model }
        : { role: "user", content: text },
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
 * `message.updated` 事件，`info.role === "assistant"` 分支——SDK 类型见
 * `@opencode-ai/sdk` 的 `AssistantMessage`(`tokens: {input, output,
 * reasoning, cache: {read, write}}` + `cost`,均实测确认字段名，见
 * `node_modules/@opencode-ai/sdk/dist/gen/types.gen.d.ts`)。只更新
 * `state`，不产出 `TranslatedEvent`——用量数字要等这一回合结束
 * （`onSessionIdle`）才跟着 Stop 一起转发，不需要单独触发一次
 * dozer-hook 调用。同一条消息可能被多次调用(流式补全)，按 id 覆盖写。
 * `reasoning` 没有对应的 Claude usage 字段，不采集(dozerd 的
 * `conversation_turns` 表本来就没有推理 token 这一列)。
 */
export function onMessageUpdated(
  state: SessionState,
  message: {
    id: string
    role: string
    tokens?: { input: number; output: number; cache?: { read: number; write: number } }
  }
): void {
  if (message.role !== "assistant" || !message.tokens) return
  state.usageByMessage.set(message.id, {
    inputTokens: message.tokens.input ?? 0,
    outputTokens: message.tokens.output ?? 0,
    cacheReadTokens: message.tokens.cache?.read ?? 0,
    cacheWriteTokens: message.tokens.cache?.write ?? 0,
  })
}

/**
 * `session.idle` 事件 → Stop。把本回合累积的文本块按首次出现顺序拼成
 * 一行 assistant transcript,叠加 `usageByMessage` 累计出的 token 用量
 * （没有文本块也没有用量时不带 transcriptLine，只发状态转换），然后清空
 * 缓冲——下一回合的文本/用量不该跟这一回合的拼在一起。
 */
export function onSessionIdle(state: SessionState): TranslatedEvent {
  const parts = [...state.textParts.entries()]
    .sort((a, b) => a[1].order - b[1].order)
    .map(([, v]) => v.text)
  state.textParts.clear()
  state.nextOrder = 0
  const text = parts.join("\n")

  let usage: NonNullable<TranscriptLine["message"]["usage"]> | undefined
  if (state.usageByMessage.size > 0) {
    let inputTokens = 0
    let outputTokens = 0
    let cacheReadTokens = 0
    let cacheWriteTokens = 0
    for (const u of state.usageByMessage.values()) {
      inputTokens += u.inputTokens
      outputTokens += u.outputTokens
      cacheReadTokens += u.cacheReadTokens
      cacheWriteTokens += u.cacheWriteTokens
    }
    usage = {
      input_tokens: inputTokens,
      output_tokens: outputTokens,
      cache_read_input_tokens: cacheReadTokens,
      cache_creation_input_tokens: cacheWriteTokens,
    }
  }
  state.usageByMessage.clear()

  return {
    event: "Stop",
    cwd: state.cwd,
    transcriptLine:
      text || usage
        ? {
            type: "assistant",
            message: {
              role: "assistant",
              content: text ? [{ type: "text", text }] : [],
              ...(usage ? { usage } : {}),
            },
          }
        : undefined,
  }
}

/** `session.deleted` 事件 → SessionEnd。不带 transcript 行。 */
export function onSessionDeleted(state: SessionState): TranslatedEvent {
  return { event: "SessionEnd", cwd: state.cwd }
}
