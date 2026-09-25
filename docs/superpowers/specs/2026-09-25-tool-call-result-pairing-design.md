# 工具调用/工具结果按 id 精确配对

**状态：已批准（brainstorming 会话，2026-09-25）**

## 背景

`review-trace`（会话审阅 trace host，见 `2026-09-25-review-trace-preact-migration-design.md`）
把一个 `AiTurn` 的 `tool_calls`/`tool_results` 渲染成两个平行数组。上一轮改造
（commit `9b350ea2`）已经把渲染顺序从"全部调用堆一起、全部结果堆一起"改成
"按数组下标 `call[i]`/`result[i]` 相邻渲染"，解决了视觉上不配对的问题，但这只是
**下标近似**——协议层 `ToolCallInfo`/`ToolResultEntry` 都没有调用 id，无法保证
下标顺序真的对应同一次调用（并行工具调用、结果乱序返回时会错位）。

本次目标：把已有的真实调用 id 接进来，让能拿到 id 的场景做到精确配对，拿不到的
场景继续吃已经上线的下标近似兜底（不会比现状差）。

## 关键架构事实（已读代码验证，决定了本次方案不需要动什么）

1. **`tool_calls`/`thinking_text` 是"读时解析"，不落库、不需要回填。**
   `dozerd::transcripts::mod::get_conversation_turns` 每次查询都对已存的
   `raw_json` 现场调 `parse::extract_turn_trace_detail()`（`parse.rs:933-936`
   原话："只在查询期间对 raw_json 现算，不在摄取时落库、不加新列"）。这意味着
   本次改动只需要扩展 `extract_turn_trace_detail` 这条读时解析路径，**不需要
   改摄取路径（`parse_claude_shaped_chunk`/`parse_goose_hook_chunk` 等热路径，
   有意不动，理由见该函数原有 doc 注释），不需要 schema 迁移，不需要任何回填
   任务**——代码上线后，所有历史会话下次打开审阅面板时查询就会带上新字段，
   不存在"老数据没有这个字段"的问题。
2. **`extract_turn_trace_detail` 按 `agent` 分派到 4 个函数**（`parse.rs:949-972`）：
   - `extract_claude_trace_detail`：服务 Claude/OpenCode/Unknown/V8agent（这四个
     agent 摄取时都走 `parse_claude_shaped_chunk`，读时解析沿用同一分派）。
   - `extract_codebuddy_trace_detail`：服务 CodeBuddy。
   - `extract_goose_trace_detail`：服务 Goose。
   - Codex/Aider：`TurnTraceDetail::default()`，读时解析目前完全不覆盖这两家
     （Codex 结构化工具调用不在 v1 范围；Aider 的 canonical 行没有结构化字段）。
3. **原始数据里 id 已经在，只是没被读出来：**
   - Claude/OpenCode/Unknown/V8agent：`message.content` 数组里 `tool_use` block
     有 `id` 字段，`tool_result` block 有 `tool_use_id` 字段（Anthropic 原生
     tool-use 协议）；`extract_claude_trace_detail` 现在只读 `thinking`/
     `tool_use` 两种 block type，没读 `tool_result` 也没读 `tool_use`
     的 `id`。
   - Goose：`PreToolUse` hook payload 有 `tool_call_id`
     （`parse_goose_hook_chunk:737-740` 已经在用它当 `message_key`，只是没有
     透传进 `ToolCallInfo`）；`PostToolUse`/`PostToolUseFailure`
     （`parse.rs:754-769`）**当前完全没有读 `tool_call_id`**，但**官方文档
     已确认这个字段同时存在于 `PreToolUse`/`PostToolUse`/
     `PostToolUseFailure` 三个事件，且同一次调用取值相同**（
     [goose-docs.ai hooks 文档](https://goose-docs.ai/docs/guides/context-engineering/hooks/)
     原话："Stable identifier for one tool call, on `PreToolUse`,
     `PreToolUseResult`, `PostToolUse`, and `PostToolUseFailure`.
     Correlates the events of a single call"）——**不再是开放问题**，
     `PostToolUse`/`PostToolUseFailure` 侧直接按同样方式提取即可。
   - CodeBuddy：`function_call`/`function_call_result` 各自有独立的顶层 `id`
     字段，互相不引用（`codebuddy_captures_function_call_and_result` 测试
     fixture 里 `fc1`/`fcr1`/`fcr2` 三个 id 互不相关）——**没有可用的配对信号,
     本次不动 CodeBuddy**，继续吃下标近似。
   - Aider：不提取结构化工具调用，不适用。
   - Codex：v1 范围外，不适用。
4. **一行 `raw_json` 可能合并了多个 `tool_result` block。**
   `parse_claude_shaped_chunk` 摄取 `tool_result` 时（`parse.rs:168-199`），
   如果一条 `user` 消息的 `content` 数组里有多个 `tool_result` block，会把它们
   的文本拼接、`is_error` 取 OR，合并成**一个** `ParsedTurn`。这种情况下无法把
   合并后的单一 `content` 字符串归到某一个具体的 `tool_use_id`，读时解析必须
   识别这种情况并放弃给 id（回落 `None`），不能瞎猜。

## 目标 / 非目标

**目标**：

1. `dozer_core::protocol::ToolCallInfo` 新增 `id: Option<String>`。
2. `dozer_core::protocol::TurnRecord` 新增 `tool_result_call_id: Option<String>`
   （只对 `role == "tool_result"` 有意义，语义同 `is_error` 字段的既有注释
   风格）。
3. `parse.rs::extract_claude_trace_detail`：`tool_use` block 顺手读出 `id`；
   新增扫描 `tool_result` block——**只有当这条消息的 content 数组里恰好一个
   `tool_result` block 时才取它的 `tool_use_id`**，零个或多个都回落 `None`
   （见上面事实 4）。
4. `parse.rs::extract_goose_trace_detail`：`PreToolUse` 的 `tool_call_id` 顺手
   填进 `ToolCallInfo.id`；`PostToolUse`/`PostToolUseFailure` 侧同样从 payload
   读 `tool_call_id` 填进 `tool_result_call_id`（官方文档已确认三个事件共享
   同一个值，见"关键架构事实"第 3 条）。
5. `TurnTraceDetail` 结构体新增 `tool_result_call_id: Option<String>` 字段，
   `get_conversation_turns` 把它接进 `TurnRecord`。
6. `dozer-app/src/transcript.rs`：
   - `ToolCallInfo`（跨 crate 复用同一个类型，见事实 1）不用改就自带 `id`。
   - `ToolResultEntry` 新增 `call_id: Option<String>`；`ReviewEntry::ToolResult`
     顶层孤儿变体同步加，保持字段对称。
   - `review_entries_from_turns` 折叠 `tool_result` 行时把
     `t.tool_result_call_id` 写进 `ToolResultEntry.call_id`。
7. `crates/dozer-app/web/review-trace/src/types.ts`：`ToolCall` 加
   `id?: string`，`ToolResult` 加 `call_id?: string`（`data.json` 契约的
   对应新增字段）。
8. `TraceToggle.tsx` 配对逻辑：
   - **全部 `tool_calls` 都有 `id`** → id 配对模式：每个 call 按原顺序渲染，
     后面紧跟所有 `call_id` 等于它的 `id` 的 result（保持这些 result 原本的
     相对顺序，即"一个 id 对应多个结果时按遇到顺序堆在那条 call 后面"，
     brainstorming 已确认口径，不特殊处理）；渲染完所有 call 后，任何
     `call_id` 对不上任何 call（或没有 `call_id`）的 result 追加在最后，
     确保不会有 result 被静默吞掉。
   - **只要有一个 `tool_calls` 缺 `id`** → 回落现有的下标近似模式（不判断部分
     场景,整体二选一,逻辑简单、行为可预期)。

**非目标**：

- 不覆盖 CodeBuddy/Aider/Codex——这三家现有数据/摄取范围拿不到可用的配对
  信号，继续吃下标近似，不在本次范围内造信号。
- 不改摄取路径（`parse_claude_shaped_chunk`/`parse_codebuddy_shaped_chunk`/
  `parse_goose_hook_chunk`）——按既有设计原则，读时解析和摄取解析是两条独立
  路径，本次只扩展前者。
- 不做 schema 迁移、不做数据库回填任务——见"关键架构事实"第 1 条，读时解析
  不需要。
- 不改变现有下标近似逻辑本身的实现（`TraceToggle.tsx` 里已经上线的
  `pairCount`/`Fragment` 那套），只是新增一个更精确的模式、按条件二选一。

## 风险与开放问题

1. ~~Goose `PostToolUse`/`PostToolUseFailure` payload 是否带 `tool_call_id`~~
   ——**已解决**：官方 hooks 文档确认存在，见"关键架构事实"第 3 条。
2. **多 `tool_result` block 合并成一行的判定必须用真实 fixture 验证。**
   `extract_claude_trace_detail` 新增的"恰好一个 tool_result block 才给 id"
   规则要有测试锁住：零个、一个、两个 `tool_result` block 三种输入分别断言
   `tool_result_call_id` 是 `None`/`Some(id)`/`None`。
3. ~~`ToolCallInfo`/`TurnRecord` 是跨 crate 共享类型，可能有遗漏消费方~~——
   **已解决**：`grep -rn "ToolCallInfo\|TurnRecord" crates/` 核实过，
   `ToolCallInfo` 只在 `dozer-core`/`dozerd`/`dozer-app::transcript.rs` 三处
   出现（本次要改的范围内）；`TurnRecord` 在 `dozerd`（session_summary.rs/
   headless_agent.rs/task_processor.rs）、`dozer-app`（todo/state.rs）、
   `dozer-client` 也有构造/传递，但全部通过 `..Default::default()`
   struct-update 语法或只读透传，新增 `Option` 字段默认 `None`，**这些位置
   不需要改代码**（`dozer-mcp` 不引用 `TurnRecord`/`ToolCallInfo`，无影响）。

## 测试策略

- **Rust 侧（`parse.rs`）**：给 `extract_claude_trace_detail`/
  `extract_goose_trace_detail`/`extract_turn_trace_detail` 补测试：
  - `tool_use` block 带 `id` → `ToolCallInfo.id` 命中。
  - 单个 `tool_result` block → `tool_result_call_id` 命中对应 `tool_use_id`。
  - 零个/多个 `tool_result` block → `tool_result_call_id` 为 `None`（风险 2）。
  - CodeBuddy/Aider/Codex 路径不受影响（现有测试应继续通过，不用新增）。
- **前端（`review-trace` 包）**：`types.ts` 改动后 `npm run typecheck` 过；
  给 `TraceToggle.tsx` 的配对逻辑补 `node:test`（用 `preact-render-to-string`
  渲染后断言 DOM 顺序，同 `markdown.test.ts` 的验证手法）：
  - 全部 call 有 id、一对一整齐 → id 模式，顺序等价于现在的下标模式。
  - 一个 id 对应两个 result → 两个 result 按原顺序堆在那条 call 后面。
  - result 的 `call_id` 对不上任何 call → 追加在最后，不丢失。
  - 部分 call 有 id、部分没有 → 整体回落下标模式（不是部分 id、部分下标混用）。
  - 全部 call 都没有 id（现状/CodeBuddy/Aider）→ 行为与现在完全一致
    （回归测试，防止这次改动误伤已经上线的下标近似路径）。
- **人工核对**：用真实 Claude 会话（本身就有并行工具调用的那种，比如一次回复
  里连续 Read 好几个文件）在审阅面板里过一遍，确认视觉上调用和结果确实一一
  对应，且和上一轮下标近似版本相比没有引入新的错位。
