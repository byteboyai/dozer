# Agent 用量统计面板设计

**状态：已批准（brainstorming 会话，2026-08-07）**

**设计稿**（Figma "Dozer Phase 1 UI"，"一期主界面v2" 页，从 `S1v2 主工作区` 克隆改造，
右图标栏新增"用量统计"图标并设为选中态，`对话列表 Rail` 替换为项目汇总条 + 按 agent
分组的会话明细，删除了不需要的 `对话审阅 Pane` 让面板本体拉宽到 764px）：
https://www.figma.com/design/NXfLQp5XQk1kF7Ohls2EbX/Dozer-Phase-1-UI?node-id=158-30

## 背景

用户提出新增一个"agent 用量统计面板"，核心用途两条：

1. 帮助用户确认自己还有多少剩余用量。
2. 帮助用户度量项目投入的 token 成本、时间成本、项目复杂度（这是重点）。

参考了 [tokdash](https://github.com/JingbiaoMei/tokdash)、[AIUsage](https://github.com/sylearn/AIUsage) 两个项目的思路，但本设计明确不追求它们的完整度，只聚焦上面两条核心需求——brainstorming 过程中进一步把这两条拆清楚了范围（见下）。

现状勘察：

- `dozer-core::protocol`、`dozerd` 存储目前完全没有 token/usage 相关字段，这是全新数据面。
- 三个 agent（Claude / CodeBuddy / OpenCode）各自的 JSONL transcript 里已经带了逐条消息的 token 用量：Claude 标准 `message.usage`（`input_tokens`/`output_tokens`/`cache_creation_input_tokens`/`cache_read_input_tokens`），CodeBuddy 的 `usage.inputTokens`/`usage.outputTokens`（样本另有一个非法定货币单位的 `credit` 字段，本设计不使用），OpenCode 由 `dozer-hook` 代写成 Claude 形状。
- `crates/dozer-app/src/conversation.rs::list_all_conversations(cwd)` 已经能枚举当前项目下三家 agent 的全部 transcript 文件（含历史上不经 Dozer 跑的原生 CLI 对话）；`crates/dozer-app/src/transcript.rs` 已经有按 agent 分派解析 JSONL 的先例（Claude/OpenCode 共用一套 schema，CodeBuddy 独立一套）。本设计直接复用这两块基础设施，不重新发明。

## 目标 / 非目标

**目标**：

1. 新增 `RightView::Usage` 面板，跟现有 `Agent`/`Conversations` 平级挂在右侧图标栏。
2. 统计范围：当前激活项目，按会话（= 一份 transcript 文件）分行、按 agent 分组展示；面板顶部另有项目级汇总条。
3. 统计维度：token 用量（input/output/cache read/cache write 分列，不合并）、会话/任务数量、工具调用次数（区分"改动类"）、触达的文件数（去重）。
4. 数据源与解析完全在 `dozer-app` 侧新增的纯函数模块（`usage.rs`）完成，不改 `dozer-core` 协议、不碰 `dozerd`（"客户端纯计算"路线，brainstorming 阶段的方案 A）。
5. 手动触发：面板打开或点刷新按钮时异步解析一遍；不做文件监听、不做自动实时更新。

**非目标**（brainstorming 阶段逐条问清楚、明确砍掉的）：

- **不做 5h/weekly limit 拝扯**。用户确认这两个数字来自 Claude Code 的 `/usage` 命令输出，不在 transcript/hook payload 里，要拿到就得向 PTY 发命令 + 解析终端输出，脆弱且只对 Claude 有效。这件事单独立项做后续，本设计完全不涉及。
- **不换算金额成本**。只显示 token 原始数；换算 `$`/`¥` 需要一份按 model 维护、会过时的定价表，用户明确选择"不做"。
- **不做跨项目全局汇总视图**。用户明确选择"只看当前项目"，跨项目汇总不在本设计范围。
- **不做点击行跳转到对话原文**。v1 明细行是纯展示表，不接交互跳转。
- **不做实时更新/文件监听**。不引入 `notify` 等新依赖，也不复用 Todo 面板的 mtime 轮询套路——这是一个"看统计"的面板，不是需要盯着实时变化的面板。
- **CodeBuddy 的工具调用解析先留空**。现有 fixture 样本（`crates/dozer-hook/fixtures/codebuddy-transcript-sample.jsonl`）里只见过纯文本回合，没见过 `tool_use` 形状的消息；CodeBuddy 分支的 `tool_calls`/`files_touched` 先恒为 0/空，不臆测其 JSON 形状，等真的观测到再补（同 `transcript.rs` 对 CodeBuddy 工具/thinking 字段的既有处理态度）。
- **不引入 dozerd 协议改动、新 SQLite 表、或本地缓存层**。brainstorming 阶段讨论过"dozerd 侧算 + SQLite 缓存"（方案 B）和"客户端 + mtime 缓存"（方案 C）两个替代方案，均因当前复杂度不匹配而否决；真的观测到"面板打开卡"的性能问题再考虑加缓存。

## 关键语义确认（brainstorming 会话定案）

- 统计对象是"这个项目目录下的全部 transcript 文件"，跟历史侧栏（`list_all_conversations`）口径一致——包含不经 Dozer 跑的原生 CLI 历史对话，因为需求是"项目总投入"而不是"仅 Dozer 会话"。
- "复杂度"由四个维度构成，全部保留：对话轮次/消息数、工具调用次数（尤其改动类）、触达的文件数、会话/任务数量。不做代码改动行数（diff 行级）统计——三家 agent 的工具入参形状不统一，精确行级 diff 需要接入 git，超出本次范围；"改动类工具调用次数"作为代理指标已经覆盖了"改动量"的意图。
- token 只分 input/output/cache read/cache write 四列原始数，不加总成一个"总 token"之外的衍生指标，不换算金额。
- 面板范围锁定当前项目，不做全局视图，也不做跨项目对比。

## 架构与数据流

### 1. `usage.rs`：数据结构与解析

新增模块，结构上镜像 `transcript.rs`（纯函数，不碰 iced/IO 副作用，`dozerd` 不参与）：

```rust
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConversationUsage {
    pub turns: u32,                     // 人类发言 + AI 回合数
    pub tool_calls: u32,                // 全部 tool_use 次数
    pub mutating_tool_calls: u32,       // Edit/Write/MultiEdit/NotebookEdit 等改动类工具次数
    pub files_touched: BTreeSet<String>, // 改动类工具 input 里的 file_path 去重集合
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cache_read: u64,
    pub tokens_cache_write: u64,
}

/// 按 agent 分派解析，Claude/OpenCode 共用一套 schema、CodeBuddy 独立一套，
/// 与 `transcript.rs::parse_transcript` 同一分派方式。单行解析失败/字段缺失
/// 一律跳过该行/记 0，不 panic、不中断整份文件的解析。
pub fn parse_usage(agent: AgentKind, jsonl: &str) -> ConversationUsage

/// 多个会话的 `ConversationUsage` 加总成项目级汇总。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectUsageTotals {
    pub conversation_count: u32,
    pub turns: u32,
    pub tool_calls: u32,
    pub mutating_tool_calls: u32,
    pub files_touched: u32, // 跨全部会话去重后的文件数
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cache_read: u64,
    pub tokens_cache_write: u64,
}

pub fn aggregate(rows: &[ConversationUsage]) -> ProjectUsageTotals
```

解析细节：

- Claude/OpenCode 分支：逐行找 `type=="assistant"` 的消息，`message.usage` 四个 token 字段直接读；`message.content[]` 里 `type=="tool_use"` 的块计入 `tool_calls`，工具名属于 `{Edit, Write, MultiEdit, NotebookEdit}` 集合的额外计入 `mutating_tool_calls` 并把 `input.file_path` 加进 `files_touched`（复用 `transcript.rs::tool_summary` 已经验证过的"取 `file_path`"读取方式，但这里要完整路径而不是取 basename）。`type=="user"`/`type=="assistant"` 各计一次 `turns`。
- CodeBuddy 分支：`type=="message"` 的行按 `role` 计 `turns`；token 从 `providerData.usage.inputTokens`/`outputTokens` 读（样本已证实存在）；`tool_calls`/`mutating_tool_calls`/`files_touched` 恒为 0/空（见"非目标"一节的说明）。

### 2. `RightView::Usage` 面板挂载

```rust
pub enum RightView {
    Agent,
    Conversations,
    Usage,
}
```

`RailButton` 加 `RightUsage` 变体，渲染方式与 `RightAgent`/`RightConversations` 逐字节一致（同一段代码参数化）。`icons.rs` 新增一个 `IconKind::BarChart3`（Lucide `bar-chart-3`，同现有图标内嵌方式加一个变体 + 一个 svg 文件）。

### 3. 触发与数据流

```rust
enum Message {
    // ...
    UsageRefresh,                                    // 面板打开 或 点刷新按钮 触发
    UsageLoaded(Vec<(ConversationMeta, ConversationUsage)>),
}
```

`UsageRefresh` 处理：`Task::perform` 把「`conversation::list_all_conversations(cwd)` → 对每个文件读全文 + `usage::parse_usage`」丢到阻塞线程执行（读取多份完整 jsonl 属于阻塞 IO，不能占 iced 的 update 线程），完成后回填 `Message::UsageLoaded`。面板打开（`RightIconSelect(RightView::Usage)` 首次选中，或从其他视图切回）时自动派发一次 `UsageRefresh`；此外面板内有一个复用 `RefreshCw` 图标的手动刷新按钮。

### 4. 面板渲染

自上而下（对应设计稿）：

1. 头部：标题"用量统计" + 当前项目名 + 右侧手动刷新按钮。
2. 项目汇总条：一张卡片，`ProjectUsageTotals` 各字段横排成一行统计位（轮次 / 工具调用(改动) / 触达文件 三项用主文字色，input / output / cache 读 / cache 写 四项 token 用青色 `#47DEF0`区分——token 数据在视觉上单独成一类，跟"活动量"三项分开）。
3. 按 agent 分组的明细列表：分组方式镜像 `group_tabs_by_agent`（新写一个 `group_usage_by_agent`，对象是 `(ConversationMeta, ConversationUsage)` 而不是 `SessionTab`），组内按 `modified_ms` 倒序（与 `list_all_conversations` 已排好的顺序一致）。分组标题行：agent 名 + "N 会话 · M tokens" 摘要。每行三部分：标题行（`ConversationMeta.title` + 最近活跃时间）、活动行（"N 轮 · M 次工具(K 改动) · J 文件"，`Roboto Mono` 暗灰）、token 行（四项 token 数，同上青色），纯展示、不接点击交互。
   - **视觉细节（来自设计稿，非硬性需求，实现时可按性价比取舍）**：若该会话就是当前打开着的会话（复用 `conversation.rs::is_current_conversation` 现成的"transcript 路径是否在已打开会话集合里"判断，不新增数据源），标题行前缀一个绿色圆点、整行改用金色描边，呼应 Conversations 面板"当前会话"的既有视觉语言。这不是本次 brainstorming 问清楚的核心需求，纯粹是画设计稿时顺手加的一致性细节，v1 没做也不影响功能完整。
4. 加载中占位态："统计中…"；解析完成前面板不显示陈旧数据（避免用户误读为最新值）。
5. 空态：项目下一个 transcript 都没有时，提示"这个项目还没有 agent 对话记录"。

## 错误处理

延续本代码库既有的"文件系统读不到/解析不动就跳过，不报错"哲学（`transcript.rs`/`conversation.rs` 现有代码同款）：

- 目录不存在/读不了 → 该 agent 那部分统计为空，不是错误态，不弹窗。
- 单行 JSON 解析失败 → 跳过该行，继续解析文件其余部分。
- 遇到未识别的工具名/字段缺失（如 CodeBuddy 工具调用形状还没见过）→ 相应字段计 0，不 panic、不猜测结构。
- 单个会话文件读取失败（权限/IO error）→ 该会话跳过、不计入汇总，不阻塞其他会话继续统计，也不让整个 `UsageRefresh` 失败。

## 测试策略

- `parse_usage`：Claude-shaped 用内联构造的 fixture JSONL（同 `transcript.rs` 测试里手写多行 JSON 字符串的方式）覆盖 token 四字段、tool_calls/mutating_tool_calls/files_touched 计数；CodeBuddy-shaped 复用现有 `crates/dozer-hook/fixtures/codebuddy-transcript-sample.jsonl`，验证 token 读取正确、tool_calls 恒为 0。
- 边界用例：空文件 → 全零 `ConversationUsage`；混入非法 JSON 行 → 跳过该行不影响其余行统计；只有人类发言没有 assistant 回复 → token 全零但 turns 计数正确。
- `aggregate`：多个 `ConversationUsage` 加总后的 `ProjectUsageTotals` 数值正确，包括跨会话 `files_touched` 去重（同一文件在两个会话里都出现过，汇总只算一次）。
- `group_usage_by_agent`：分组结果按 agent 归类正确，组内保持传入顺序。
- UI 侧不写自动化测试（现有面板惯例：view 函数不测，纯逻辑测），加载态/空态/手动刷新交互留给 `cargo run -p dozer-app` 人工验收。

## 依赖变更

无新增依赖。
