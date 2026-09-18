# 按面板的能力注册与 Agent Tool 暴露设计

**状态：待审阅（需求澄清会话，2026-09-18）**

## 背景与动机

Dozer 是站在甲方一侧的 AI 治理与验收层。当前 `crates/dozer-mcp` 已经是一个
只读 MCP stdio server，挂在每个由 dozer 拉起的 agent 会话上，现有 5 个 tool：
`get_preview_context`、`submit_session_summary`、`list_todos`/`add_todo`/
`toggle_todo`/`edit_todo_text`。

这 5 个 tool 是"手写枚举、逐个硬编码进 `#[tool_router]`"的形态
（`crates/dozer-mcp/src/server.rs:55`）。它有两个天花板：

1. **发现性差**：agent 只能看到编译期写死的 tool 列表，Dozer 面板新长出的能力
   （文件树搜索、git log、database schema…）不会被 agent 感知，除非再手写一遍。
2. **能力边界模糊**：哪些 tool 是只读、哪些会改用户环境、哪些要审批，全靠
   每个 `#[tool]` 方法体自己把握，没有统一的策略层。

本设计要解决的问题：**让 Dozer 的每个面板声明自己能提供哪些能力（capability），
由 dozerd 聚合成一份注册表，`dozer-mcp` 从注册表动态生成 MCP tool 列表暴露给
所有 CLI agent；每个能力按"面板默认策略 + 单 action 授权覆盖"决定只读/可写/
需审批，需审批的走 Dozer 原生确认框。**

## 目标 / 非目标

**目标：**

1. **面板声明能力**：每个 `extensions/*` 模块（= 一个面板）在同处声明一份
   能力清单——能力名、入参/出参 schema、默认执行策略（只读/可写/需审批）。
   不要求每个面板立刻都实现能力声明；先建机制 + 接 1~2 个面板做首个消费者。
2. **dozerd 聚合注册表**：dozerd 汇总各面板上报的能力，形成
   `CapabilityRegistry`，按 `project_id`/`session_id` 过滤后供查询。
3. **MCP 动态发现**：`dozer-mcp` 的 `tools/list` 从注册表生成（而非编译期写死），
   agent 能发现当前项目下**实际可用**的能力；`tools/call` 按能力名路由回 dozerd。
4. **按面板的默认策略 + 单 action 授权覆盖**：每个面板有一个默认执行策略，
   单个 action 可以覆盖它；写操作按策略决定是否必须走确认框。
5. **需审批的能力走 Dozer 原生确认框**：agent 发起 → dozerd 挂起 → GUI 弹
   原生确认框 → 用户批准/拒绝 → 结果回执给 agent。
6. **向后兼容**：现有 5 个 tool 迁移到新机制（或至少继续可用），不出现
   "能力注册上线后现有 tool 全部消失"的断层。

**非目标：**

- **不给 agent 直接读写项目文件的工具**。Dozer 是治理/验收层，文件读写交给
  agent 自己的原生 tool（OpenCode/Codex 都有）。Dozer 只暴露"agent 拿不到的
  用户态/项目态上下文"和"面板能力"。这是一条裁决，不是本期的裁剪。
- 不做跨项目的全局能力编排、不做能力间的依赖/流水线（YAGNI，等到有真实
  需求再说）。
- 不改 `dozer-mcp` 的 stdio 传输形态，不改会话→project 的解析方式（现状
  每次 tool call 现连 dozerd，本设计沿用）。
- 不实现所有面板的能力——本期落地机制 + 选定首批面板，其余面板按后续计划
  逐个接入。

## 关键语义确认（需求澄清会话定案）

1. **按面板提供 tool**（用户定案）：能力归属以面板为单位。每个
   `extensions/*` 模块是能力的来源，能力命名带面板前缀（如
   `files_search_name`、`git_log_list`），便于发现与权限粒度。
2. **MCP 做能力注册/发现**（用户定案）：MCP server 不再硬编码 tool 列表，
   而是从 dozerd 的注册表读取并映射成 MCP tool 描述；调用时按能力名回传。
3. **按面板定默认策略 + 单 action 授权**（用户定案）：面板级默认
   （如 `todo` 面板默认低风险放行、`database` 面板默认写操作需审批），
   单个 action 可覆盖（如 `database` 面板里 `db_query` 只读、`db_execute`
   需审批）。
4. **审批 = 弹 Dozer 确认框**（用户定案）：需审批的能力，在 GUI 里弹原生
   确认框，用户点同意才执行；拒绝则回执给 agent 一个明确的"被拒绝"结果，
   不静默失败。
5. **目标消费者是所有 CLI agent**（用户定案）：不针对某一个 agent 定制，
   走标准 MCP 协议面。

## 架构

### 1. 能力声明（面板侧）

每个面板在自身模块里声明一份能力清单。以搜索面板为例：

```rust
// extensions/search.rs（示意）
pub fn capabilities() -> Vec<CapabilityDecl> {
    vec![CapabilityDecl {
        name: "search_code",
        description: "在当前项目作用域内做全文内容搜索（字面子串，大小写不敏感）",
        panel: PanelId::Search,
        policy: CapabilityPolicy::ReadOnly,
        params_schema: /* JSON Schema */,
    }]
}
```

`CapabilityDecl` 与 `CapabilityPolicy` 定义在 `dozer-core`（协议共享层），
不放 `dozer-app`——因为 dozerd 和 dozer-mcp 都要消费它，不能反向依赖 GUI。

```rust
// dozer-core::protocol（示意）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelId {
    Files, Search, GitLog, FileHistory, Conversations,
    Todo, Database, Ssh, Browser, Usage, Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityPolicy {
    /// 只读，无需审批，直接执行。
    ReadOnly,
    /// 可写，按面板默认策略决定是否需要审批（见 `PanelPolicy`）。
    Write,
    /// 无论面板默认如何，此 action 一律需要确认框。
    AlwaysApprove,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityDecl {
    pub name: String,
    pub description: String,
    pub panel: PanelId,
    pub policy: CapabilityPolicy,
    /// 入参的 JSON Schema（MCP 的 inputSchema 直接复用）。
    pub params_schema: serde_json::Value,
}
```

**面板默认策略**（单 action 的 `policy` 优先于它）：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelDefaultPolicy {
    /// 该面板下 Write 能力默认无需审批（如 Todo）。
    WritesAutoApprove,
    /// 该面板下 Write 能力默认需要审批（如 Database/Ssh）。
    WritesRequireApproval,
}
```

生效规则（**合并函数是纯函数，好单测**）：

```
fn effective_needs_approval(decl: &CapabilityDecl, panel_default: PanelDefaultPolicy) -> bool {
    match decl.policy {
        ReadOnly     => false,
        AlwaysApprove => true,
        Write        => matches!(panel_default, WritesRequireApproval),
    }
}
```

用户可在设置里改某个 action 的授权（"这个 action 以后不用问了"），
这层覆盖存 dozerd 的配置表，参与 `effective_needs_approval` 的最终求值。

### 2. 能力注册表（dozerd 侧）

dozerd 新增 `CapabilityRegistry`（内存 + 持久化的策略覆盖）：

- **来源**：各面板声明的 `CapabilityDecl`。GUI 侧在项目打开时通过一个新的
  `Request::PublishCapabilities { project_id, decls }` 上报（面板能力是
  GUI 侧模块的产物，daemon 拿不到 `extensions/*` 代码）。
- **聚合**：dozerd 按 `project_id` 存一份，供 MCP 查询。
- **策略覆盖持久化**：`capability_policy_overrides` 表
  （`(project_id, capability_name) -> auto_approve: bool`），用户确认框勾选
  "以后不再询问"时写入。

新增协议面（示意）：

```rust
// Request
PublishCapabilities { project_id: i64, decls: Vec<CapabilityDecl> },
ListCapabilities  { project_id: i64 },
// Reply
Capabilities { decls: Vec<CapabilityDecl> },
```

`ListCapabilities` 返回时，dozerd 把 `effective_needs_approval` 算好一并
带回（或返回原始 decl + 覆盖状态，让 MCP 侧算——建议前者，避免两处逻辑）。

### 3. MCP 动态发现与路由（dozer-mcp 侧）

`DozerMcpServer` 的 `#[tool_router]` 目前是编译期宏。改成动态生成：

- **`tools/list`**：调用 `Client::list_capabilities(project_id)`，把每个
  `CapabilityDecl` 映射成一个 MCP `Tool{ name, description, input_schema }`。
  现有 5 个手写 tool 要么迁成面板能力（Todo 面板的 4 个 + Preview 面板的
  `get_preview_context`），要么保留一段过渡期。
- **`tools/call`**：不再按编译期方法名分发，而是把 `{ capability_name, args }`
  转发给 dozerd 的新 `Request::InvokeCapability`，由 dozerd 决定执行或挂起
  审批（见下）。MCP server 职责收敛成"协议翻译 + 每次现连 dozerd"。

> rmcp 的动态 tool 形态：需核实 `#[tool_router]` 是否支持运行时注册，若不支持，
> 则改为手写实现 `ServerHandler::list_tools`/`call_tool`（rmcp 提供该 trait
> 的默认方法，可覆盖）。这是实现计划第一步要验证的技术点，不在此定死写法。

### 4. 审批回合（能力调用 → 确认框 → 回执）

**问题**：现有 `dozer-mcp` 的每次 tool call 是"一次 request/一次 reply"的
同步往返。审批需要"挂起等待用户在人机界面点按钮"，这段等待可能数秒到数分钟，
不能占着 dozerd 的连接处理循环。

**方案**：dozerd 侧引入"挂起中的审批"概念，用独立 id 关联：

```rust
// Request
InvokeCapability {
    session_id: String,       // 谁在请求（解析 project_id / 回执用）
    capability: String,
    args: serde_json::Value,
},

// Reply（立即返回，不阻塞）
CapabilityPending { approval_id: u64 },   // 需审批
// 或直接
CapabilityResult { value: serde_json::Value },  // 只读/自动放行

// 回执（agent 侧轮询或长连接推送）
// Request
AwaitCapabilityResult { approval_id: u64 },   // 阻塞式等待
// Reply
CapabilityResult { value } / CapabilityDenied { reason }
```

dozerd 收到 `InvokeCapability`：

1. 算 `effective_needs_approval`。
2. **不需要审批**：直接执行能力（能力的具体执行仍是"发回 GUI 面板执行"——
   因为能力实现在 `extensions/*`，只有 GUI 进程有），把结果作为
   `CapabilityResult` 返回。
3. **需要审批**：`approval_id` 入"待审批"表，向 GUI 推一条待审批事件，
   GUI 弹原生确认框。用户点同意 → GUI 上报 `ApproveCapability{approval_id}`，
   dozerd 再走执行路径；点拒绝 → `DenyCapability{approval_id}`，回执
   `CapabilityDenied`。

**执行归属**：能力实现在 GUI（`extensions/*` 是 GUI 侧代码），所以"执行"这步
必须绕回 GUI。dozerd 是调度/审批中枢，GUI 是能力执行器。数据流：

```
agent → dozer-mcp → dozerd(审批判定/挂起) → GUI(弹框)
                                          → GUI(执行能力)
                                          → dozerd(回执) → dozer-mcp → agent
```

新增协议（GUI ↔ dozerd）：

```rust
// GUI → dozerd：能力执行结果回填
ReportCapabilityResult { approval_id: u64, result: Result<Value, String> },
// GUI → dozerd：审批决定
ApproveCapability { approval_id: u64, remember: bool },  // remember = 写入策略覆盖
DenyCapability    { approval_id: u64 },
```

### 5. 确认框的 GUI 形态

- 复用既有原生确认框形制（参照 `extensions/ssh.rs:1364`
  `delete_confirm_popup`、`extensions/file_history` 的悬浮层）。
- 内容：能力名（人话描述，非 tool 名）、面板来源、关键入参摘要、同意/拒绝，
  外加一个"以后同类操作不再询问"的勾选（写策略覆盖）。
- **重复请求合并**：同一 `capability + 关键入参` 在短时间内的多次待审批，
  合并成一次确认框（避免 agent 循环调用时刷屏）。合并去重的窗口与键属实现
  细节，设计上承认这个需求存在。

### 6. 面板 → 能力映射（首批 + 远景）

**首批落地**（机制验证 + 真实消费者，从低风险开始）：

| 面板 | 能力 | 策略 |
|------|------|------|
| **Preview** | `get_preview_context`（迁移现有） | ReadOnly |
| **Search** | `search_code(query, scope?)` | ReadOnly |
| **Files** | `files_list(path?)`、`files_git_status` | ReadOnly |
| **Todo** | `list_todos`/`add_todo`/`toggle_todo`/`edit_todo_text`（迁移现有） | Write（面板默认 AutoApprove） |
| **GitLog** | `git_log_list`、`git_commit_detail`、`git_diff` | ReadOnly |

**远景**（后续计划逐个接，不在本期）：

| 面板 | 只读 | 写（按默认策略定审批） |
|------|------|----------------------|
| Database | `db_schema`、`db_query` | `db_execute` |
| Ssh | `ssh_hosts` | `ssh_exec` |
| Browser | `browser_snapshot` | `browser_navigate` |
| Files | （见首批） | `files_open_preview`（引导查看，非改文件） |
| FileHistory | `file_history(path)` | — |
| Usage | `usage_stats` | — |

## 错误处理

- **能力未注册**：`InvokeCapability` 收到注册表里没有的名字 → 回执
  `CapabilityDenied{ reason: "unknown capability" }`，不 panic。
- **能力执行失败**（GUI 侧 `extensions/*` 返回 `Err`）：经
  `ReportCapabilityResult` 回填 `Err(String)` → `CapabilityResult` 里带错误
  文案，agent 能读到。
- **审批超时**：待审批挂起超过阈值（如 5 分钟）未决，dozerd 主动回执
  `CapabilityDenied{ reason: "approval timeout" }`，清理挂起项。阈值属实现
  细节，设计上承认需要。
- **GUI 不在线**：需要审批但 GUI 未运行/未连 → 直接回执拒绝（理由
  "no_gui"），不无限挂起。
- **协议版本不匹配**：老 GUI + 新 dozerd（或反之）→ 保持既有协议兼容
  惯例，新增 `Request`/`Reply` 变体对老版本表现为未知变体，按仓库现有做法
  处理（实现时核对现有未知变体分支）。

## 测试策略

- **`effective_needs_approval` 纯函数单测**：覆盖三种 `CapabilityPolicy` ×
  两种 `PanelDefaultPolicy` × 有/无用户覆盖的全部组合。
- **注册表单测**（dozerd）：`PublishCapabilities` → `ListCapabilities`
  往返；策略覆盖持久化后重查生效。
- **审批状态机单测**（dozerd）：只读直接执行、写按策略放行/挂起、
  批准/拒绝/超时/gui 不在线各自产出的回执形状。
- **MCP 映射单测**（dozer-mcp）：`CapabilityDecl` → MCP `Tool` 的字段映射；
  `tools/call` 的路由（不需要真起 GUI，用 mock client）。
- **确认框渲染**：headless 编译验证 + 真机目测（本仓惯例）。
- **端到端**：真机跑一个 agent 会话，让它调 `search_code`（只读，直接返回）
  与一个需审批的写能力（验证确认框弹出、批准执行、拒绝回执三条路径）。

## 依赖变更

无新增外部依赖。新增协议变体走既有 `serde`/`dozer-core` 机制；MCP 侧若
rmcp 的 `#[tool_router]` 不支持动态注册，改用其 `ServerHandler` trait 的
可覆盖方法，同样不引新依赖。

## 排期备注

1. **第一步技术验证**：核实 rmcp `#[tool_router]` 能否运行时动态注册 tool；
   不能的话，`ServerHandler::list_tools`/`call_tool` 覆盖是否可行（几行
   spike，不是先搭完再发现走不通）。
2. **数据模型先行**：`dozer-core` 的 `CapabilityDecl`/`Policy`/`PanelId` +
   纯函数策略求值 + 单测，先落地（无外部依赖，可独立验证）。
3. **注册表 + 协议**：dozerd 的 `PublishCapabilities`/`ListCapabilities` +
   GUI 上报。
4. **MCP 动态发现**：`tools/list` 从注册表生成（此时可先把现有 5 个 tool
   保持不变，只**新增**发现能力，验证不回归）。
5. **审批回合**：`InvokeCapability` + 确认框 + 回执。
6. **首批能力接入**：Search/Files/GitLog 只读 + Todo 写（AutoApprove）。
7. **迁移现有 5 个 tool** 到新机制（最后做，确保过渡期不断层）。

第 1、2 步互不依赖可并行；3 依赖 2；4、5 依赖 3；6 依赖 5；7 依赖 6。
