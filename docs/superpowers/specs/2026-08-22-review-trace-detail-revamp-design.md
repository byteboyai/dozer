# 会话审阅面板：话题上下文 + 三段式 trace 折叠面板 Design

**Status:** 已批准设计，待写实现计划。

## 背景

"会话"面板右侧的审阅内容（`crates/dozer-app/src/workspace.rs::review_content_pane`）现在由一个 wry webview（`crates/dozer-app/src/review_trace.html`）渲染，数据源是 `ReviewEntry`（`crates/dozer-app/src/transcript.rs`），由 `dozerd` 查询回来的 `TurnRecord`（`crates/dozer-core/src/protocol.rs`）转换而来。当前效果：

- 一条扁平时间线，`Human`/`AiTurn`/`ToolResult` 三种条目依次排列，`ToolResult` 独立于产生它的 `AiTurn`（无归属关系）。
- `AiTurn` 只有一个"思考过程(略)"静态徽标（`thinking: bool`），没有真实思考文本；工具调用只有一行摘要（如"Edit README.md"），没有具体参数。
- 没有"当前话题在会话里前后是什么"的上下文——用户只能回到左侧列表重新点选。

参考设计稿（`dozer-paste-a916a75e...png`、`dozer-paste-188b6a5e...png`）要把这两点补上：

1. 展开的话题上下有"前一话题/下一话题"预览条（纯文字，静态展示）。
2. AI 回合的"轨迹"折叠面板从单一徽标改为三段式：思考过程 / 操作过程 / 工具结果，各自独立可折叠。

"话题"= 既有的 turn-group 概念（`TurnGroupRow`，已经在驱动 `Message::ConversationTurnGroupOpen(path, agent, start_turn_index, end_turn_index)`），不引入新分组概念。

## 范围边界

- 只改"审阅内容"这一条链路：`dozerd` 查询层（新增读时解析）→ `TurnRecord`/`ReviewEntry` 结构 → `review_trace.html`。
- 左侧 `conversation_list_pane`（列表本身的排序/搜索/筛选）、`dozerd` 的摄取解析（`parse_claude_shaped_chunk`/`parse_codebuddy_shaped_chunk`）、DB schema（`conversation_turns` 表结构）**都不动**。
- 前一话题/下一话题**只在同一 session 内**（同一 `TurnGroupRow.path`），不跨 session；**纯静态预览，不可点击跳转**（均为讨论中已拍板的范围裁剪）。
- 工具调用 ↔ 结果的归属用**位置邻接**（同一 `AiTurn` 之后、下一个 `Human`/`AiTurn` 之前的连续 `ToolResult` 都算它的），不引入 `tool_use_id` 精确匹配。
- 不做任何 DB 迁移/历史数据回填。

## 架构与数据流

### 1. 读时解析，不动摄取层

真实思考文本、工具调用的完整 `input` JSON，当前都在摄取时被丢弃——`parse.rs` 的 assistant 分支里，`Some("thinking") => thinking = true`（`parse.rs:238`）只置布尔位，`Some("tool_use")` 分支（`parse.rs:225-236`）把 `input` 折叠进 `tool_summary(name, &input)` 一行摘要后即丢弃结构化值（`parse.rs:227-228`）。但每行原始 JSONL 本身作为 `raw_json: String` 全量持久化在 `conversation_turns` 表（`ParsedTurn`/`raw_json` 字段，`parse.rs:273` 等构造点；`mod.rs:84` 建表；`mod.rs:186-216` 写入）。

因此新增内容**在读侧解析**，不碰摄取：`crates/dozerd/src/transcripts/parse.rs` 新增一个独立纯函数

```rust
pub fn extract_turn_trace_detail(raw_json: &str, agent: AgentKind) -> TurnTraceDetail {
    // 复刻 parse_claude_shaped_chunk/parse_codebuddy_shaped_chunk 里
    // 对 message.content blocks 的遍历逻辑,但改成"提取"而不是"摘要+丢弃":
    // Some("thinking") => 取 b.get("thinking") 的文本
    // Some("tool_use")  => 取 name + tool_summary(name,&input) + input 本身(pretty-print 成字符串)
}

pub struct TurnTraceDetail {
    pub thinking_text: Option<String>,
    pub tool_calls: Vec<ToolCallInfo>,
}

pub struct ToolCallInfo {
    pub summary: String,        // 沿用现有 tool_summary() 的一行摘要
    pub input_json: Option<String>, // pretty-print 后的 input,None 表示没有/解析失败
}
```

这是**独立函数**，不重构/复用 `parse_claude_shaped_chunk` 内部逻辑——摄取路径是热路径、已有测试覆盖，不承担本次改动风险；解析口径上两者对 `type` 字段的判断保持一致即可，允许少量重复代码。

调用点只有一处：`DozerdDb::get_conversation_turns`（`mod.rs:261-286`）。SELECT 语句新增 `raw_json` 列，每行在原有字段基础上调用 `extract_turn_trace_detail(&raw_json, agent)` 填充新字段（`agent` 需要作为参数传入这个函数，目前 `get_conversation_turns` 签名里没有——从调用方 `conversation_id` 对应的 session 记录取，或者直接加一个 `agent: AgentKind` 参数，由 `spawn_review_load` 已知的 `_agent`——当前这个参数带下划线前缀表示未使用，正好启用）。

只在用户打开审阅面板时触发（低频、单个 session 的几十行），不影响摄取吞吐、不需要预计算。

### 2. `TurnRecord` / `ReviewEntry` 结构调整

`crates/dozer-core/src/protocol.rs::TurnRecord` 新增字段：

```rust
pub struct TurnRecord {
    pub turn_index: i64,
    pub role: String,
    pub content: String,
    pub tool_calls: Vec<ToolCallInfo>,   // 替换原 tools_summary: Vec<String>
    pub thinking: bool,                   // 保留(其它调用点可能仍用这个布尔位判断"是否有思考")
    pub thinking_text: Option<String>,    // 新增:真实思考文本
    pub ts: Option<u64>,
    pub is_error: bool,
}
```

`ToolCallInfo` 从 `dozerd` 侧挪到 `dozer-core::protocol` 作为共享类型（跟 `TurnRecord` 一样走 UDS 序列化）。

`crates/dozer-app/src/transcript.rs::ReviewEntry` 调整：

```rust
pub enum ReviewEntry {
    Human { text: String },
    AiTurn {
        text: String,
        thinking_text: Option<String>,
        tool_calls: Vec<ToolCallInfo>,
        tool_results: Vec<ToolResultEntry>, // 新增:紧跟这个 AiTurn 的连续 ToolResult
    },
    /// 孤儿兜底:前面没有 AiTurn 的 ToolResult(理论边界情况,如导出片段
    /// 从工具结果行开始),保留在顶层、渲染方式不变。
    ToolResult { content: String, is_error: bool },
}

pub struct ToolResultEntry {
    pub content: String,
    pub is_error: bool,
}
```

`review_entries_from_turns` 的转换逻辑从"逐条 map"改成"顺序 fold"：遇到 `AiTurn` 先记下来，随后连续的 `tool_result` 角色的 `TurnRecord` 都塞进它的 `tool_results`，直到遇到下一个 `human`/`ai` 角色或数组结束才把这个 `AiTurn` 推进结果列表。会话开头就是 `tool_result`（没有前置 `AiTurn`）时，落入顶层 `ReviewEntry::ToolResult` 分支。

`thinking_text`/`tool_calls`/`tool_results` 任一为空就不进入对应字段（`tool_calls`/`tool_results` 是空 `Vec` 而非 `None`，模板侧按"数组非空才渲染这个子区"处理）。

### 3. 前一话题 / 下一话题

计算发生在 `dozer-app` 侧，不新增 dozerd RPC：`ws.conversation_turn_groups: Option<Vec<TurnGroupRow>>`（`workspace.rs:396`）已经是"当前项目全部 session 的 turn-group 扁平列表"（`spawn_all_turn_groups_refresh`，`list_all_turn_groups(cwd, 500)` 摄入，`workspace.rs:974-984`），左侧 Conversations 面板本来就靠它渲染。打开一个话题时（`ConversationTurnGroupOpen` 处理里），用这份已加载的列表：

1. 过滤出 `path` 与当前话题相同的行（同一 session）。
2. 按 `start_turn_index` 排序。
3. 取当前话题前一条 / 后一条，取不到（首/末话题，或该邻居因 500 条上限没被加载进来）就是 `None`——**不额外发请求去补**,这属于已知取舍,见下节。

预览文字直接用 `TurnGroupRow.title`（已加载、零成本），**不去抓那个话题的正文**——`title` 是一行短标签（如 `"Fix Login Bug"`），不是设计稿里 lorem ipsum 那种多行正文；为了不让审阅面板一次要发 3 份 `get_conversation_turns` 请求（当前 1 份变 3 份），接受这个观感落差。

`review_trace.html` 消费的 `data.json` payload 新增：

```json
{
  "entries": [...],
  "prev_topic": { "label": "Fix Login Bug" } | null,
  "next_topic": { "label": "..." } | null
}
```

（具体是 `ReviewLoaded` message 整体带上这两个字段，还是 `ReviewView` 结构体新增字段供 `review_webview_spec` 序列化时一并塞进去，是实现阶段的选择，不影响本设计接口。）

### 4. `review_trace.html` 渲染改动

- 页面顶部（时间戳下方）、条目列表结束后，各加一块"前一话题:"/"下一话题:" 静态文字块，`prev_topic`/`next_topic` 为 `null` 时整块不渲染（不是渲染空文案）。
- 每个 `AiTurn` 的"轨迹"折叠区（已存在的 `<details>` 外层）内部改成三个独立子区，复用现有的折叠样式：
  - **思考过程**：`thinking_text` 非空才渲染，纯文本块。
  - **操作过程**：`tool_calls` 非空才渲染，每条一行摘要 + 点开看 `input_json`（沿用现有"结果内容"的折叠展开样式）。
  - **工具结果**：`tool_results` 非空才渲染，每条按 `is_error` 走既有的成功/失败配色，折叠规则沿用现有"短内容自动展开"逻辑。
  - 三段都为空（`AiTurn` 既没思考也没工具活动）时，整个"轨迹"折叠入口不显示——与现状一致。

## 组件改动清单

- `crates/dozerd/src/transcripts/parse.rs`：新增 `extract_turn_trace_detail`/`TurnTraceDetail`/挪一份 `ToolCallInfo`（或从 `dozer-core::protocol` 导入）。**不改动**现有 `parse_claude_shaped_chunk`/`parse_codebuddy_shaped_chunk`。
- `crates/dozerd/src/transcripts/mod.rs`：`get_conversation_turns` 的 SELECT 加 `raw_json`，签名加 `agent: AgentKind` 参数（或从调用方一并传入 session 记录里已有的 agent 字段），逐行调用新解析函数填充 `TurnRecord` 新字段。
- `crates/dozer-core/src/protocol.rs`：`TurnRecord` 结构调整（见上），新增 `ToolCallInfo`。
- `crates/dozer-client`（如果 `get_conversation_turns` 的客户端签名也带 `agent` 参数）：同步调整调用签名。
- `crates/dozer-app/src/transcript.rs`：`ReviewEntry`/`ToolResultEntry` 调整，`review_entries_from_turns` 改为顺序 fold。
- `crates/dozer-app/src/workspace.rs`：`spawn_review_load` 把已知的 `_agent` 参数实际传给 `get_conversation_turns`；`ConversationTurnGroupOpen` 处理里新增"从 `conversation_turn_groups` 算前后邻居"这一步；`ReviewView`/`ReviewLoaded` payload 或 `review_webview_spec` 序列化处新增 `prev_topic`/`next_topic`。
- `crates/dozer-app/src/review_trace.html`：三段式轨迹渲染 + 前后话题预览条。

## 错误处理

- `raw_json` 缺失或解析失败（历史脏数据/字段变更）：`extract_turn_trace_detail` 返回全空的 `TurnTraceDetail`（`thinking_text: None`, `tool_calls: vec![]`），不 panic、不影响该行其它字段（`content`/`role` 等仍来自原有列，不依赖这次解析）。
- 邻居话题不在已加载的 `conversation_turn_groups`（500 条上限截断，或该 session 只有一个话题）：`prev_topic`/`next_topic` 为 `None`，前端整块不渲染。
- `AiTurn` 后没有任何 `ToolResult` 跟随：`tool_results` 是空 `Vec`，"工具结果"子区不渲染。

## 测试

- `extract_turn_trace_detail`：Claude 与 Codebuddy 两种 fixture（沿用 `crates/dozer-hook/fixtures/` 现有样例基础上补 thinking/tool_use 场景），覆盖"有思考文本"“单个/多个 tool_use”“无任何 block”三种输入。
- `review_entries_from_turns` 新的 fold 逻辑：连续 `ToolResult` 正确归入前一个 `AiTurn`；开头孤儿 `ToolResult` 落入顶层分支；多个 `AiTurn` 交替出现时不串组。
- 前一/下一话题邻居计算（新增的纯函数，从 `Vec<TurnGroupRow>` + 当前 `path`/`start_turn_index` 算邻居）：同 session 多话题排序正确、首末话题返回 `None`、跨 session 的行不会被误当邻居。
- `review_trace.html` 的 JS 渲染沿用现状：无自动化测试，人工 GUI 验收（该文件本身没有测试基础设施，符合项目现状）。

## 已知取舍（不在本设计范围内，记录避免以后重新讨论）

- 前后话题预览文字用 `TurnGroupRow.title`（一行短标签），不是话题正文摘录——避免审阅面板一次触发 3 份 `get_conversation_turns` 请求。如果未来观感上需要更长的正文预览，需要专门评估"预取相邻话题正文"的成本，不在本次范围。
- 500 条 turn-group 加载上限导致的邻居缺失不做兜底请求；如果后续发现这种截断在真实使用中很常见（比如单 session 话题数很多的重度用户），需要另外设计针对"给定 session 拿它的相邻 turn-group"的专用查询，而不是复用这份全局列表。
- `tool_use_id` 精确匹配（应对同一 `AiTurn` 内并行工具调用乱序返回的边界情况）明确不做，位置邻接假设"同一回合的工具结果严格按调用顺序紧跟着返回"，这在当前所有已知 agent（Claude/Codebuddy）的 transcript 里成立。
