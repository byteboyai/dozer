# Dozer V2 开源项目逐仓代码复核

> 复核日期：2026-09-24  
> 源码位置：`/Users/chrischiang/Projects/GitHubProjects/dozer-v2/<owner>/<repo>`  
> 范围：生态调研文档列出的 59 个仓库；58 个已取得浅克隆，`kafkazhang/seek_code` 已下架。  
> 目标：判断项目对 Dozer V2 的具体价值，而不是复述 README 或按 star 排名。

## 1. 方法与裁决标准

每个项目至少检查当前 HEAD、顶层模块、构建清单、架构/安全文档以及与 Dozer 有关的核心源码入口。结论分为：

- **A 深入跟踪**：直接影响 Host、daemon、插件、Agent、worktree 或治理模型；
- **B 专题参考**：某个子系统有明确价值；
- **C 互操作/依赖候选**：通过 adapter、协议或 Rust crate 使用；
- **D 删除出重点清单**：没有新增证据、内容重复、只有链接集合或源码不可用。

“删除”表示从 Dozer V2 重点调研和路线决策中删除，不删除上游本地 clone，也不抹掉已经形成的历史报告。

## 2. 整体产品与 Agent 工作台

| 项目 | 代码证据/核心边界 | 对 Dozer V2 的价值 | 裁决 |
|---|---|---|---|
| Tidebreak | Rust workspace 已拆出 `tidebreak-core`、supervised/sandbox agent、shell policy、egress、MCP、remote、worker runtime、code delivery；插件目录有独立 `PLUGIN.md` | 当前最接近 Dozer“治理工作台 + 插件 + sandbox”的整体参照；重点比较 shell policy、egress 和 delivery，而非 Tauri UI | **A** |
| Daintree | `packages/plugin-sdk`、`create-daintree-plugin`、`plugin-vite`、`daintree-plugin` 与 `docs/plugins/architecture.md` 已形成开发链 | 插件脚手架、开发热加载、source model、trust preview 比早期调研更成熟；适合校准 Dozer SDK/CLI | **A** |
| Arbor | daemon-backed Rust/GPUI，多客户端访问 repository/worktree/terminal/diff | 验证 daemon 是产品核心而非实现细节；重点比较 session owner、客户端 attach 和 worktree identity | **A** |
| Vibe Kanban | Rust crates 包含 worktree manager、review、deployment、remote、relay、preview proxy、desktop bridge | Todo 不应只是列表；它可以成为 task/worktree/review/deployment 控制面。这是 Dozer Todo 插件首个 vertical slice 的直接参照 | **A** |
| tty7 | `tty7-core`、`tty7-server`、`tty7-cli`；同一 LocalHost 服务本地/SSH，控制协议有 dialect，CLI 提供 run/send/capture/events/wait | Dozer daemon、远程 workspace、回压、observer、Agent 状态和协议升级的最高优先级工程参照 | **A** |
| Argos | monorepo 明确拆成 daemon、agent/acp/mcp/memory/skills/knowledge/remote-control runtime 与 shared contracts | 与 Dozer V2 模块表高度同构；重点研究 shared contracts 和 runtime registry，防止 core/UI 反向依赖 | **A** |
| Orca | [§12.8](#128-orca)：Electron 调度台、PTY daemon、worktree、remote authority 与 VM 执行宿主 | daemon、remote authority、VM、permission、vault | **A** |
| Workmux | [§12.11](#1211-workmux)：Rust CLI/TUI，复用多种 multiplexer，覆盖完整 worktree 生命周期与 reconciliation | worktree、安全 Git 配置、sandbox、reconciliation | **A** |
| Agent Deck | [§12.1](#121-agent-deck)：Go/tmux 会话管理、原生 session fork、共享 MCP proxy 与运行时健康管理 | session fork lineage、共享 MCP proxy、runtime health | **A** |
| Goose | [§12.4](#124-goose)：Rust Agent core，包含 MCP/ACP、ToolInspector、extension manager 与 SQLite session | MCP/ACP、inspector pipeline、session/context crates | **A** |
| OpenChamber | monorepo 有 SDK、Electron、Web、mobile、VS Code 和 extension 文档；`isolated-spaces/DESIGN.md` 管隔离空间 | “同一 core，多宿主 + SDK extension”参考价值高；isolated spaces 可对照 Task–Worktree–Environment | **A-** |
| Limboo | `docs/architecture/subsystems/worktree-manager.md`、runtime telemetry 和 security model 均为独立子系统 | Session Bundle、worktree manager、telemetry 与 permission 的组合适合 Dozer Run 模型 | **A-** |
| CleeCode | 单 Rust 应用，同时通过 `clee --mcp` 暴露 open files、selection、diagnostics、preview 和需授权的 buffer edit | 证明 Host 应作为受限 MCP server；unsaved buffer、session scope 与 UI 授权是 `dozer-context` 的直接需求 | **A-** |
| Kooky | [§12.6](#126-kooky)：SwiftUI/libghostty，轻量 hook、OSC 7/133、Agent adapter 与 worktree manager | hook、shell integration、Agent adapter 和新 worktree manager | **A-** |
| T3 Code | [§12.10](#1210-t3-code)：共享 server contract，结构化 Agent 协议优先，包含 remote access 与连接状态机 | server contract、结构化 Agent、remote access、连接状态机 | **A-** |
| Garcon | [§12.3](#123-garcon)：七类 Agent CLI 的结构化子进程管道、permission 闭环与 worktree/diff staging | 多 Agent 结构化 adapter 和 permission channel | **A-** |
| OpenCode | [§12.7](#127-opencode)：独立 HTTP/OpenAPI+SSE protocol、细粒度 permission、fork 与受限 subagent | 独立 protocol、permission、fork、subagent | **A-** |
| Pane | Electron/移动端/remote，包含 pane chat bundle、Agent farm、浏览器和大量 E2E | 适合产品信息架构与独立 pane 资源模型；AGPL 限制代码复用，且范围过宽 | **B** |
| AutoDev | KMP core/UI/server/IDEA/VS Code/codegraph 分层，core 支持 MCP、AGENTS.md、skills、A2A/ACP | 跨宿主 runtime、Context Source 和 codegraph Provider 有价值；不扩张 Dozer V2 到全端 | **B** |
| ai-14all | Electron，shared architecture decisions，session 绑定 worktree/terminal，人工合并闸门 | “完成不等于接受”与 Dozer一致；适合验收状态词和监督视图，不作为架构模板 | **B** |
| jcode | [§12.5](#125-jcode)：全 Rust Agent runtime，覆盖命令风险、语义记忆、swarm 与 CI 棘轮 | 风险、memory、swarm、CI ratchet | **B** |
| IfAI | Tauri/React，DAG、工作流、OpenSpec、事件/审批概念；许可证排除核心 AI/RAG/Agent 工具链 | 只参考 WorkflowDefinition/Run UI 与审批体验；核心实现不可假定开源 | **B-** |
| Ateam | worktree-per-task、Mission Control、merge/update/cleanup | 保留为 Git 生命周期 UX 参考；工程机制被 Workmux/Vibe Kanban 覆盖 | **B-** |
| Claude Squad | [§12.2](#122-claude-squad)：Go/tmux/worktree，Pause/Resume 可释放并重建 worktree | suspend/resume 资源释放；其余主要是反例 | **C** |
| Crystal | 当前版本为 0.3.5，面向并行 coding session/worktree 管理 | 与同类项目机制重叠，保留用于 UX 比较；在 daemon、插件或治理上没有超过 tty7/Vibe Kanban 的独立证据 | **D：移出重点** |
| MonoCode | Tauri + React，vendor `portable-pty`，统一运行用户已登录的 Agent CLI | 适合 Agent 发现/启动/输入的产品基线；无独立插件、worktree、治理创新 | **D：移出重点** |
| Elves | Tauri workspace、tasks、packaging 和本地项目模型 | 多仓 worktree 有参考，但当前活跃度与机制独特性不足 | **D：移出重点** |
| SeekCode | 上游 404，无法取得当前源码 | 历史上只有 working memory/egress/危险动作的补充证据，均已有更强替代 | **D：停止跟踪** |

## 3. 插件与 Wasm Runtime

### Zed 与 Extensions

Zed 的价值不只是 Wasm。`extension.toml`、版本化 extension API、开发安装、贡献点和独立 registry 共同构成完整生态。`zed-industries/extensions` 是注册与审查数据，不能单独视为 runtime。

对 Dozer V2：采用 manifest/contribution/API compatibility/dev-install/registry 五件套；不要复制 Zed 的编辑器特定贡献点，也不要让 Wasm 成为唯一插件形式。两仓合并为一份研究对象，评级 **A**。

### Lapce

Lapce 将 app、core、proxy、RPC 分 crate，插件走 WASI。其价值是“UI 客户端与 proxy 执行面分离”和 WASI 插件经验，而不是 Floem UI。

对 Dozer V2：作为 Wasm logic plugin 的 B 级对照；进程插件仍是默认。评级 **B**。

### Extism 与 Wasmtime

Extism 当前直接建立在 Wasmtime 48 LTS 上，提供 manifest、kernel、runtime、Host functions 和多语言 PDK；Wasmtime 自身是庞大的编译/runtime/security 工程。

对 Dozer V2：先用 Extism 做规则/parser 插件 spike，只有需要 component model 或更深资源控制时才直接使用 Wasmtime。两者分别为 **C（spike）** 与 **C（底层候选）**，不能同时无目的引入。

### wasmCloud

wasmCloud 解决分布式 component、lattice 和 capability provider，远大于桌面插件运行时。

对 Dozer V2：capability provider 命名和组件契约有参考，但其分布式控制面会过度设计本地 Host。评级 **D：移出重点**。

## 4. MCP SDK 与 Gateway

### Rust SDK (`rmcp`)

官方仓库已到 v3.4.1，包含 client/server、transport、WASI examples、conformance、versioning 与 dependency policy。Dozer 已使用它。

对 Dozer V2：继续采用，但封装在 `dozer-mcp-runtime`；领域事件、权限和 service directory 不直接使用 SDK 类型。评级 **A：直接依赖**。

### 四个 Gateway 的差异

| 项目 | 实现重心 | 对 Dozer V2 的价值 | 裁决 |
|---|---|---|---|
| IBM Context Forge | Python 主体 + Rust runtime，virtual server、plugin hooks、多租户、安全与 API translation | service directory、virtual aggregation、plugin hook point、审计模型最完整 | **A 专题参考** |
| Microsoft MCP Gateway | .NET、portal、deployment、OpenAPI | 企业部署与门户参考，但不适合本地 Rust Host | **D** |
| MikkoParkkola Gateway | Rust `gateway-core`，capabilities、architecture、安全审计 | 本地/边缘 Rust gateway 的最直接代码参考；检查 routing、policy、backpressure | **A-** |
| jonfairbanks Gateway | Python、配置和 schema 驱动 | 轻量实现，但独特价值被前两者覆盖 | **D** |

`e2b-dev/awesome-mcp-gateways` 只是链接集合，不是实现，**从深入代码分析清单删除**。它可以作为发现入口，但不应影响架构裁决。

Dozer 不应单独部署一个“万能 gateway”作为启动前提。Host 内只保留本地 Service Directory、租约、授权与路由；企业级 federation 通过外部 gateway Provider 接入。

## 5. Execution Environment 与容器

| 项目 | 核心机制 | 对 Dozer V2 的价值 | 裁决 |
|---|---|---|---|
| OpenHands + docs | Action/Observation runtime、Docker action executor、browser/bash/plugin、REST；docs 描述 SDK agent server | `ExecutionEnvironment` 的 action/observation/event/artifact contract；镜像 tag 与重建缓存 | **A**，两仓合并分析 |
| Coder | workspace template、用户身份、策略、secret/control plane | 远程 workspace Provider 与身份继承；不嵌入 Host | **B** |
| Daytona | 当前 GitHub 仓库只剩 README/assets，核心实现不在此快照 | 无法作为代码级依赖评估；只保留产品/API 参考 | **D：从代码分析删除** |
| container-use | 每次任务使用隔离容器环境，围绕 Agent 工作流组织 | container/worktree-like UX 与环境生命周期参考；Go 二进制不嵌入 | **B** |
| Bollard | Rust Docker API、socket/transport、模型 codegen | Docker-compatible Provider 候选，必须藏在 adapter 后 | **C：依赖 spike** |
| testcontainers-rs | 测试期容器生命周期 | 用于 Provider 集成测试，不用于生产 runtime | **A：dev dependency** |

## 6. Memory 与上下文

### Basic Memory

当前仓库包含明确 `ARCHITECTURE.md`、note format、MCP/UI 实验和多种 Agent integration。优势是 local-first、人类可读知识、SQLite 索引和 MCP 互操作。

对 Dozer V2：学习 provenance、关系、渐进发现和 Agent 集成；Dozer 的 task/run/decision 仍以 SQLite 结构化事实为主，Markdown 是投影。评级 **A**。

### Mem0

仓库同时覆盖 Python/TypeScript、server、evaluation 和大量集成，定位是通用记忆平台而非 workspace 工作流数据库。

对 Dozer V2：借鉴 add/search/update/delete、entity scope 和 evaluation；不引入其全栈。评级 **B**。

### Laya

当前版本 0.3.20，Python/PyTorch 与 TS 客户端齐全，提供容器和 HTTP 部署。它是 typed decision model，不是通用 LLM。

对 Dozer V2：只作为 `DecisionProvider` sidecar，以 shadow mode 验证路由/排序；必须记录模型版本、置信度、fallback 与最终裁决。评级 **B：外部 Provider**。

## 7. 浏览器测试

### Playwright

Playwright 的真正价值是确定性 automation、browser context 隔离、trace/DOM/network/console artifact 与成熟测试 runner。

对 Dozer V2：Browser Test 插件以 Playwright sidecar 为主路径；Host 只消费统一 BrowserAction/Observation/Artifact。评级 **A：外部 runtime**。

### Open Browser Use

仓库包含 Rust host/wire/node-repl 与 TypeScript SDK/extension，`DESIGN.md`、runtime descriptor 明确进程边界。相比纯 Python browser agent，它更接近 Dozer 的 broker 模型。

对 Dozer V2：参考 owner-only IPC、capability gate、长驻 browser session、runtime descriptor；AI 自动化位于 Playwright 确定性层之上。评级 **A-**。

## 8. 可观测性与审计

| 项目 | 能力 | 对 Dozer V2 的价值 | 裁决 |
|---|---|---|---|
| Phoenix | trace、dataset、eval、experiment、OpenInference/OTel、sandbox evaluator | Agent run trace 和质量评估概念最贴近 Code Health/Usage | **A 专题参考** |
| Langfuse | trace、prompt、dataset、人工标注、复杂服务端 | UI/人工 labeling 有参考；部署栈过重 | **B** |
| OpenTelemetry Rust | API/SDK/OTLP/bridges；不同 signal 稳定度不同 | `tracing` 埋点 + 可选 OTLP exporter；领域事件不能等同 span | **C：exporter spike** |

Dozer 本地 SQLite Event/Artifact Store 是权威事实源；OTLP 是导出协议，不是产品数据模型。

## 9. 通用基础库

| 项目 | 结论 | 对 Dozer V2 的价值 | 裁决 |
|---|---|---|---|
| notify | 跨平台 FS event + full/mini debouncer | 继续用于 workspace/plugin hot reload；overflow 后全量 reconcile | **A：已有依赖** |
| jsonrpsee | HTTP/WS client/server、subscription、middleware、macro | 仅当 Plugin Protocol 确认采用 JSON-RPC 且需要其网络能力才引入 | **C** |
| Temporal Rust SDK | durable workflow client/worker 语义 | 重试、history、determinism 可借鉴；本地桌面引入 Temporal 服务过重 | **D：不依赖** |
| Zellij | Rust terminal workspace/multiplexer、插件和 session | PTY/multiplexer/插件参考，但 Dozer 不复制终端复用器产品 | **B** |

## 10. 最终去留

### 保留为最高优先级

Tidebreak、Daintree、Arbor、Vibe Kanban、tty7、Argos、Orca、Workmux、Agent Deck、Goose、Zed、OpenHands、Basic Memory、Playwright、Open Browser Use、IBM Context Forge、MikkoParkkola MCP Gateway。

### 保留为专题或互操作参考

OpenChamber、Limboo、Pane、CleeCode、AutoDev、IfAI、Ateam、ai-14all、Kooky、T3 Code、Garcon、jcode、OpenCode、Lapce、Extism、Wasmtime、Coder、container-use、Mem0、Laya、Phoenix、Langfuse、OpenTelemetry、Bollard、testcontainers-rs、notify、jsonrpsee、Zellij。

### 从重点调研清单删除

- Crystal：与 worktree 工作台样本重复，没有新增架构机制；
- MonoCode：只验证多 Agent CLI GUI 的基础需求；
- Elves：独特性和近期活跃度不足；
- Claude Squad：除 suspend/resume 外主要作为反例，保留旧报告即可；
- SeekCode：上游不可访问且关键结论已有更强替代；
- wasmCloud：分布式控制面超出本地插件 Host 所需；
- Microsoft/jonfairbanks MCP Gateway：分别偏企业部署和轻量重复实现；
- awesome-mcp-gateways：链接集合，不是可分析实现；
- Daytona：当前 clone 没有核心源码；
- Temporal Rust SDK：需要外部 Temporal 系统，超出 V2；
- extensions registry、OpenHands docs：分别并入 Zed、OpenHands 主报告，不单独计为产品。

## 11. 对 Dozer V2 的统一修正

逐仓复核后，Dozer V2 的优先架构应明确为：

1. `dozer-core` 定义 Task、Run、Environment、Artifact、Permission、Decision、Acceptance；
2. daemon 持有 PTY/session/Agent state/event stream，并处理回压、重连和版本协商；
3. Task Runtime 管 worktree、container/remote environment、port、secret 和清理；
4. Plugin Supervisor 管 manifest、进程、贡献点、权限、升级和故障隔离；
5. Host 同时是 MCP client 和受限 `dozer-context` MCP server；
6. Permission Engine 采用多 inspector、三值/证据化合并，而不是单一 allow/deny；
7. Todo 插件成为第一个 Task–Worktree–Run vertical slice；
8. Browser、Memory、Code Health、Laya 都作为 Provider/插件接入统一事件与 artifact 模型。

最大的风险不是某个技术不可行，而是把上述能力再次写进 `dozer-app` 或 `dozerd` 的巨对象。所有新增模块都应先回答：它定义核心语义，还是只是一个可替换实现？只有前者进入 core。

## 12. 原有 11 份代码级分析归档

本节合并原有 11 份独立报告的完整正文，保留其源码考古、历史判断和 V2 复核说明。原独立文件已在合并校验完成后删除；后续结论以本总报告的“最终去留”和各归档开头的“Dozer V2 复核结论”为准。

---

### 12.1 agent-deck

> 已合并自原独立报告 `agent-deck-分析.md`。

### Agent Deck 代码级分析

> V2 复核：2026-09-24，源码快照 `3b41e36de82d89f84a951e5c0f490fc6e1973c41`。

#### Dozer V2 复核结论

- **仍然成立**：原生 session fork 委托、tmux 持有会话、hook 环境变量路由和共享 MCP 代理池仍是其最独特价值。
- **修正旧结论**：仓库当前已有 `docs/rfc/PLUGIN_ATTACH.md`、runtime health 和 watchdog 设计，插件/附着与运行时健康已成为正式演进方向，不能再只把它视为单体 TUI。
- **对 V2 最有价值**：共享 MCP server 必须重写 request id、隔离调用方、引用计数/租约、显式回收子进程；session fork 需要区分一次性启动参数与持久身份。
- **反例仍成立**：Provider 巨对象说明没有早期 contract 会导致分支扩散。Dozer 第二个 Agent Adapter 接入前就应冻结最小 trait/event schema。
- **采用建议**：A 级 MCP multiplexing 与 session lineage 参考；不采用 tmux 作为 Dozer 的核心会话持有者。

> 分析对象:https://github.com/asheshgoplani/agent-deck (本地副本 `/Users/chrischiang/AI/agent-deck`,`--depth 1` 浅克隆,当前机器上多个并发 clone 任务导致带宽紧张,耗时较久)
> 分析日期:2026-07-28 · Go 36.9 万行(不含测试)+ 20.1 万行测试 / 1,335 个 `.go` 文件(895 个 `_test.go`)/ 5,885 个 `func Test` / MIT · 613 star,102 fork,开源 8 个月(2025-12-03 建仓)

#### 一句话定位

Agent Deck 是**驾驭 Claude Code/Codex/Gemini/OpenCode/Cursor/Pi 等多个 agent CLI 的 Go + Bubble Tea TUI 会话管理器**——终端渲染和会话存活完全外包给 tmux(与 workmux 同构,不自研终端引擎),自己的核心价值是"一张牌桌上看清所有 agent 在干什么"。它区别于此前六个分析对象的差异化卖点是 **`session fork`**:声称"分叉一个会话,新会话继承父会话的完整对话历史,而不是像 workmux/kooky 的 `--resume` 那样接回同一个会话"。核实结论见"核心机制一"——这个卖点是真的,但实现方式和直觉预期不同:**Agent Deck 自己完全不碰对话历史,fork 出的会话历史续接 100% 委托给被驾驭 agent CLI 自带的原生 fork/resume 命令**(`claude --resume <id> --fork-session`、`codex fork <id>`、`opencode -s <id> --fork`、`pi --fork <jsonl路径>`),Agent Deck 的自研代码只解决"新会话该不该有一份独立的、包含父会话未提交改动的工作目录"这个正交问题(git 工作树物化)。

体量上,36.9 万行非测试代码 + 20.1 万行测试代码,测试代码比生产代码还多——这是七个分析对象里测试密度最高的一个(测试行数/生产行数 ≈ 1.19,workmux 是六个项目里此前的密度冠军但也只到"1,532 测试/7.6 万行"这个量级,行数比例上 Agent Deck 更极端)。项目由 [asheshgoplani](https://github.com/asheshgoplani) 一人主导(contributors 接口显示其贡献占比压倒性),8 个月内打了上百个 tag(`v1.9.x` 到 `v1.10.11` 密集连续发布,像是每个通过 CI 的 PR 就切一个版本),属于"单人高频迭代 + 测试驱动"的工程文化,而不是团队协作的产物。

#### 工程结构

```
cmd/agent-deck/        CLI 入口 + 三十多个子命令(session/worktree/hook/mcp/conductor/watcher/…)
internal/
  session/   12.7万行(含测试)  Instance 巨对象:每个 agent 会话的全部状态与逻辑,9,371 行单文件 instance.go
  ui/        7.6万行            Bubble Tea TUI:dashboard、各种 dialog(含 ForkDialog)
  tmux/      2.9万行            tmux 会话/窗口/pane 的驱动层,本地终端 attach/detach 用 creack/pty
  web/       1.8万行            web 终端桥接(terminal-bridge,可通过浏览器远程操作 tmux 会话)
  git/       0.8万行            worktree 创建、fork-with-state 物化、setup/destruction 钩子脚本
  statedb/   0.7万行            SQLite(modernc.org/sqlite 纯 Go 驱动)会话状态存储,从旧版 JSON 单文件迁移而来
  mcppool/   0.3万行            MCP server 进程池 + Unix socket 代理(多会话共享一个 MCP 进程)
  jujutsu/                     Jujutsu(jj)VCS 后端支持,与 git 后端并列在 internal/vcs 抽象之下
  fleet/ costs/ docker/ watcher/ openclaw/ selfheal/ …  一批更外围的子系统(遥测、成本追踪、沙盒、自愈)
```

`internal/session/instance.go` 单文件 9,371 行,是全项目的中枢——`Instance` 结构体同时装了 Claude/Codex/Gemini/OpenCode/Pi/Cursor 等每个 provider 的专属字段(`ClaudeSessionID`/`CodexSessionID`/`OpenCodeSessionID`/`GeminiSessionID`…),配合几十个 `if i.Tool == "xxx"` 分支和per-provider 方法族(`CanForkCodex`/`buildCodexForkCommandForTarget`/`CanForkOpenCode`/…)。这个结构在"核心机制四"里详细讨论。

#### 核心机制一:`session fork`——委托给被驾驭 CLI 原生命令,不做对话重放,也不是写时复制

这是本次分析的核心任务。结论:**Agent Deck 的"fork 继承历史"完全建立在被驾驭 agent CLI 自己已有的 fork/resume 原语之上,Agent Deck 自己一行对话内容都不解析、不复制、不重放。**

按 provider 逐一核实(均来自 `internal/session/instance.go` 与对应的 `instance_<tool>_fork_test.go`):

- **Claude**(`buildClaudeForkCommandForTarget`,instance.go:7584):预生成一个新 UUID 作为子会话 ID,拼出 `claude --session-id "<新UUID>" --resume <父session-id> --fork-session`,直接 `exec` 起一个新的 claude 进程。`--fork-session` 是 Claude Code CLI 自带的官方标志,Agent Deck 只是负责"知道父会话的 session id、生成一个新 UUID、拼好命令行"。
- **Codex**(`buildCodexForkCommandForTarget`,instance.go:7915,源码注释原文):"Mirrors buildCodexCommand's resume path... uses `fork`, which clones the parent transcript into a new thread with a fresh id while leaving the parent intact"——命令是 `codex fork <父session-id>`,`fork` 同样是 Codex CLI 自带子命令。`CanForkCodex()` 的前置条件是**父会话的 rollout JSONL 文件已经落盘**(`codexRolloutExistsInHome`),这是"resume/fork 前必须确认磁盘上真有会话记录"这条判断准则的具体体现,而不是自己去读那份 JSONL 做逻辑复制。
- **OpenCode**(`ForkOpenCodeWithOptions`/测试 `TestOpenCodeForkUsesNativeForkFlag`):命令是 `opencode -s <父session-id> --fork`。测试注释特别写明这是**替换了旧实现**——旧版本曾经是 `opencode export | sed | import` 这种"导出会话 JSON、文本替换、再导入"的手工克隆手法,后来 OpenCode CLI 自己加了 `--fork` 原生标志,Agent Deck 就把手工克隆路径整个换掉了。这条历史本身就是强证据:**手工做会话克隆是被验证过、且被放弃的路线,原生标志一旦出现就应该立刻切过去**。
- **Pi**(`buildPiForkCommandForTarget`,instance.go:1848):命令是一段 shell:在 Agent Deck 自己的 `~/.pi/agent-deck/<父实例ID>/` 目录下 `find ... -name '*.jsonl' | head -n 1` 找到父会话最新的 JSONL 文件路径,再拼 `pi --fork "$source_file" --session-dir "$session_dir"`。这是四种里最"手工"的一种——Agent Deck 确实要自己去定位文件路径——但定位到路径之后仍然是**把路径原样交给 Pi 自己的 `--fork` 标志**,不解析、不拷贝 JSONL 内容本身。
- **Gemini**:`CanFork()`(instance.go:7486)开头就是一条硬编码:`// Gemini CLI doesn't support forking; if i.Tool == "gemini" { return false }`。**这是明确的负面证据**——Gemini CLI 没有原生 fork/resume-fork 原语,Agent Deck 索性不支持,没有做任何"自己实现一套等价物"的尝试。这条比任何"如何做"的细节都更能说明问题:fork 能力的天花板完全由被驾驭 CLI 决定,Agent Deck 自己没有独立于宿主 CLI 的会话历史复制能力。

**新旧会话之间的资源关系**:两个会话是各自独立的进程 + 各自独立的 provider 侧 session id(新会话是新 UUID/新线程 id),彼此不共享底层 PTY 或 provider 进程——是"两个独立进程,分别 attach 到宿主 CLI 自己维护的会话存储",不是共享内存态、不是 fork(2) 式的进程复制、也不是任何数据库层面的写时复制。真正决定"新会话是否忠实继承历史"的,是宿主 CLI(Claude/Codex/OpenCode/Pi)自己的 `--resume`/`fork` 实现质量,Agent Deck 只是**正确调用**这些原语的调度层,谈不上"逻辑复制",更谈不上 COW。

**一个真实存在的坑及其修法**(`fork_start_dispatch_test.go` 里记录的 #745 回归):Fork 出的新 `Instance` 需要把"待启动的一次性 fork 命令"(含 `--resume`/`--fork-session`/`fork`/`--fork` 这些一次性参数)和"这个会话以后重启该用什么命令"分开存——否则 tmux pane 意外死掉后自动重启时,会**把一次性的 fork 命令重放一遍**,对父会话再 fork 一次,产生连锁分叉(`issue1728_fork_storm_test.go` 专门测的就是这种"fork 风暴")。解法是 `IsForkAwaitingStart` + `ForkStartCommand` 这对瞬态字段(不落盘,`json:"-"`):首次启动消费一次性命令,消费后清空,之后的重启走各 provider 各自的稳定 resume/bare 命令。这是"一次性启动指令"和"持久重启身份"必须分离存储的通用教训,不是 fork 专属。

**"fork-with-state":一个正交的、真正自己实现的机制**——git 工作树物化。`docs/superpowers/specs/2026-05-18-fork-with-state-followup-design.md` 和 `internal/git/materialize_wip.go` 显示,`--with-state`/`--with-state-and-gitignored` 做的是:`git -C parent diff --cached --binary | git -C new apply --index`(复制暂存态)→ `git -C parent diff --binary | git -C new apply`(复制未暂存改动)→ `git -C parent ls-files --others -z --exclude-standard` 逐个拷贝未跟踪文件(→ 可选再拷贝 gitignored 文件,注意 `--ignored` 是过滤器而非叠加,需要两趟 union,历史上有一版实现在这里出过 bug)。这一步**只读父工作树、只写新 worktree**,对话历史续接(上面那四种原生命令)和文件状态续接(这一步)是完全独立的两条流水线,靠 CLI/TUI 的编排代码组合在一起,不是同一份底层机制的两个视图。

**对 Dozer 的意义**:这是七个分析对象里第一次看到"驾驭层完全不实现会话历史续接,纯粹委托宿主 CLI 原生命令"的清晰案例——比 t3code"优先用结构化协议,PTY 兜底"更进一步:t3code 至少要包一层 JSON-RPC 客户端(`effect-codex-app-server`),Agent Deck 连包装都省了,直接拼 shell 命令字符串转交。这给 Dozer 两条具体启示:(1) 若未来要做"分叉会话"这类功能,第一步应该查 Claude Code 自己有没有类似 `--fork-session` 的原生标志(需要验证,截至目前未在本仓库内核实到),有就直接透传,不要自己去解析 `~/.claude/projects/**/*.jsonl` 做"逻辑复制"——OpenCode 从手工 export/sed/import 切到原生 `--fork` 的历史本身就是"手工复制路线迟早被原生标志淘汰"的活教材;(2) `IsForkAwaitingStart`/`ForkStartCommand` 这对"一次性启动指令 vs 持久重启身份"分离存储的模式,`dozerd::Session` 如果未来也需要"用特殊参数启动一次,之后按正常方式重启"这类场景(不只是 fork),可以直接照搬这个瞬态字段 + 消费即清空的设计。

#### 核心机制二:会话持有——遥控 tmux,与 workmux 同构但独立实现

`internal/tmux/pty.go` 只用 `creack/pty` + `golang.org/x/term` 做**本地终端到 tmux pane 的 attach/detach**(处理 detach 快捷键的三种编码:原始字节、xterm modifyOtherKeys、kitty CSI u),`internal/tmux/tmux.go` 则是对 `tmux list-windows`/`has-session` 等命令的封装。也就是说:**agent CLI 进程本身跑在 tmux 窗口里,tmux server 才是真正的会话持有者**,Agent Deck 进程退出对存活会话无影响——这与 workmux "不自研终端引擎,遥控 tmux/Zellij/WezTerm/kitty"的架构结论完全一致,只是 Agent Deck 只驾驭 tmux 一种后端(未见 Zellij/WezTerm/kitty 支持),换来的代价是不需要 workmux 那层 `Multiplexer` trait 抽象。

会话状态从早期的单文件 JSON(`session.StorageData`,`internal/statedb/migrate.go` 里仍保留迁移代码)升级成了 SQLite(`modernc.org/sqlite`,纯 Go 驱动,无需 CGO)。`internal/statedb/statedb.go` 里的 `withBusyRetry` 处理并发写入的 `SQLITE_BUSY`,`backupDBFile` 在 schema 迁移前先备份,`saveInstancesOnce` 里显式防"空扫场景"（sweep 相关注释:"empty sweep turns silent data loss into a loud, recoverable error"）——即批量保存时如果传入的实例列表意外为空,不能被当成"全部清空"来执行,必须报错而不是静默清空数据库。这是一条与 kooky/workmux"运行时字段不落盘、磁盘值不可信、损坏即删"不同的容错哲学:**SQL 事务 + 显式防御性断言**,比"单文件损坏就删"更重,但换来的是并发多进程写同一份状态时不会互相踩踏。

**对 Dozer 的意义**:再次印证"自持 PTY 是重路线、遥控宿主终端复用器是轻路线"这条 workmux 已经给出的结论,Agent Deck 是第二个独立收敛到同一选择的项目,进一步降低了这条经验的偶然性。状态存储从"单 JSON 文件"到"SQLite"的迁移路径,对 `dozerd` 未来如果要解决"`SessionRegistry` 纯内存 HashMap、进程重启全丢"这个已知缺口时是一个可考虑的方向——但要注意 Dozer 目前的容错哲学(运行时字段不落盘)和这条路线的前提(把几乎所有状态都塞进一个可查询的数据库)是两种不同的设计取舍,不能不假思索照搬。

#### 核心机制三:MCP 集成——跨会话共享 MCP 进程的 socket 代理池

`internal/mcppool/socket_proxy.go`(642 行)是这次七个分析对象里第一次见到的、有实际工程深度的 MCP 集成方案。核心问题:如果每个 agent 会话(每个 tmux pane)都独立起一份 MCP server 子进程,MCP server 数量随会话数线性增长,启动开销和资源占用都不划算。Agent Deck 的解法:

- `SocketProxy` 包一个 stdio MCP 子进程,对外暴露一个 Unix domain socket;多个客户端(多个 agent 会话)连到同一个 socket,请求经代理转发给底层唯一的 MCP 进程
- **JSON-RPC ID 重写**:每个进来的请求 ID 被替换成代理自己维护的单调计数器(`nextID atomic.Int64`),同时记录 `idMapping{sessionID, originalID}` 存入 `sync.Map`,响应回来时按重写后的 ID 查到原始请求属于哪个 session、原始 ID 是什么,再换回去发给对应客户端——这是标准的多路复用做法,保证多个会话共用同一个 MCP 进程时 ID 空间不冲突
- `stdinMu` 序列化对 MCP 进程 stdin 的写入,防止并发请求把 JSON 行写乱掉(帧同步问题)
- `waitOnce sync.Once` 保证子进程只被 `Wait()` 收尸一次,源码注释提到 v1.7.43 之前 `broadcastResponses` 检测到 MCP 进程 stdout EOF(进程死了)但没有正确 reap,导致僵尸进程一直挂到 `Stop()` 被调用(可能永远不被调用)——这是一个真实修复过的资源泄漏 bug
- `internal/mcppool/pool_simple.go` 的 `PoolConfig` 支持 `PoolAll` + 排除名单或白名单两种模式来决定哪些 MCP server 走池化、哪些保留独立 stdio 生命周期

**对 Dozer 的意义**:这是六个先前分析对象里都没有深入讨论过的一块,Agent Deck 给出了目前样本里唯一"多会话共享同一 MCP 进程"的具体实现。如果 Dozer 未来要做 MCP 集成(spec 目前未定),这套"socket 代理 + JSON-RPC ID 重写 + 单调计数器 + sync.Once 收尸"的模式是一份可直接参考的最小实现;`waitOnce` 那个真实修复过的僵尸进程 bug 也提醒:**转发型 MCP 代理必须显式设计子进程收尸路径**,不能假设 EOF 检测等价于进程已回收。

#### 核心机制四:多 provider 抽象——一个 9,371 行的巨对象,不是契约层

任务要求核实"多 provider 会话模型怎么统一抽象",结论:**很薄,而且不是显式契约层**。对照此前分析过的 t3code(`packages/contracts` 独立包,WebSocket 协议的类型化契约,client/server 共享)和 workmux(单个 `Multiplexer` trait,~30 方法,四个后端各自实现),Agent Deck 走的是完全不同的路:

- 单个 `Instance` struct(`internal/session/instance.go:115`)同时容纳所有 provider 的专属字段:`ClaudeSessionID`/`ClaudeDetectedAt`、`CodexSessionID`/`CodexDetectedAt`、`OpenCodeSessionID`/`OpenCodeDetectedAt`、`GeminiSessionID`……每加一个 provider 就往这个 struct 里加一组字段,而不是定义一个 `Session` 接口让各 provider 实现
- 判断"能不能 fork"“怎么 fork”全靠字符串比较分支(`if i.Tool == "opencode"`、`if i.Tool == "pi"`、`IsCodexCompatible(i.Tool)`)加一族并列的 per-provider 方法(`CanFork`/`CanForkOpenCode`/`CanForkPi`/`CanForkCodex`,`buildClaudeForkCommandForTarget`/`buildCodexForkCommandForTarget`/`buildPiForkCommandForTarget`),没有一个共同的 `Forkable` 接口把这些方法收拢起来
- 连"识别一个命令字符串属于哪个 tool"这种基础功能都是最近才收敛的:`internal/session/builtins.go` 的注释直接写明,在此之前这段逻辑分裂在两个手工同步的函数里(`detectTool()` 和 `isBuiltinToolName()`),issue #1258 才把它们合并成一份 `builtinTools()` 数据表——**项目自己也在事后重构掉重复代码**,不是一开始就设计好的
- 目前登记在册的 built-in tool 有 11 个:claude/opencode/gemini/codex/pi/copilot/crush/cursor/hermes/aider/shell,其中 "aider" 有名字但没有探测规则(遗留的不对称,注释里原样承认),"shell" 是兜底分类

**对 Dozer 的意义**:这是一次有效的反例确认,不是借鉴点——**规模大、迭代快的项目不必然收敛出干净的抽象层**;t3code 的 contracts 包和 workmux 的 trait 都是刻意设计的结果,Agent Deck 证明"不做这层设计"同样能撑起一个 600+ star、11 个 provider 的项目,只是代价是核心文件涨到 9,371 行、判断逻辑散在几十个 `if Tool ==` 分支里,且连"命令字符串识别 tool"这种基础工具都要事后专门重构一次去重。Dozer 目前只适配 Claude Code 一家,CLAUDE.md 的"agent 中立"是北极星而非现状——真到了要适配第二个 agent CLI(如 Codex)时,这份反例提醒:**趁早为"provider 专属字段/方法"设计一层薄接口(哪怕只是一个 `AgentAdapter` trait),比等到规模上来后再重构一个巨对象成本低得多**。

#### 核心机制五:Hook 端点 + 环境变量路由——与 workmux 几乎同构

`internal/session/claude_hooks.go`(`InjectClaudeHooks`/`RemoveClaudeHooks`/`mergeHookEvent`)的实现模式与 workmux 的 `agent_setup/hooks.rs` 高度相似:按事件名分组的 JSON 树,`mergeHookEvent` 做语义合并而非整体覆盖写,`hooksAlreadyInstalled`/`eventHasAgentDeckHookMatchingConfig` 保证安装操作幂等。会话身份路由靠 `AGENTDECK_INSTANCE_ID` 环境变量在拼 shell 命令时内联注入(`instance.go:984` 等多处),Claude 的 hook 子进程读这个变量识别自己属于哪个 Agent Deck 会话——和 Dozer 的 `DOZER_SESSION_ID`、kooky 的 `KOOKY_SURFACE_ID` 是同一形状的方案。除 Claude 外,gemini_hooks_cmd.go/cursor_hooks_cmd.go/codex_hooks_cmd.go/hermes_hooks_cmd.go 分别对应各家 CLI 自己的 hook 配置格式,是"每个 provider 一个安装器"而非统一抽象,呼应核心机制四的结论。

**对 Dozer 的意义**:这是七个分析对象里第三次独立验证"hook 配置必须做语义合并/精确摘除,不能覆盖写整个配置文件"这条纪律(kooky、workmux 之后的第三例),`dozer-hook` 的 `install.rs` 如果尚未做到"合并 + 摘除 + 空壳清理"三件套的完整覆盖,这条经验的说服力又增加了一份独立样本。

#### 对 Dozer 的启示汇总

| Agent Deck 机制 | 对应 Dozer 位置/路线图 | 借鉴点 |
|---|---|---|
| `session fork` 完全委托宿主 CLI 原生命令(`--resume --fork-session`/`fork <id>`/`-s <id> --fork`),Gemini 无原生原语则明确不支持 | 若未来做"分叉会话"功能 | 先查 Claude Code 有没有类似原生标志再动手(本仓库未核实到);OpenCode 从手工 export/sed/import 切到原生 `--fork` 的历史证明手工复制路线迟早被原生标志淘汰,不宜自己重造 |
| `IsForkAwaitingStart` + `ForkStartCommand` 瞬态字段,消费即清空,防止 tmux 重启后重放一次性 fork 命令引发"fork 风暴"(issue #745/#1728) | `dozerd::Session` 的启动/重启逻辑 | "一次性启动指令"与"持久重启身份"必须分离存储,不只适用于 fork 场景 |
| fork-with-state 的 git 工作树物化(`diff --cached \| apply --index` → `diff \| apply` → `ls-files --others` 拷贝),与对话历史续接完全正交、独立实现 | 若 Dozer 要做"帯着未提交改动分叉工作区"这类功能 | 两条流水线(会话历史续接 vs 工作树文件续接)应该保持解耦,不要因为要做其中一个就误以为必须重新发明另一个 |
| tmux 是真正的会话持有者,Agent Deck 自己的 PTY 代码只做本地终端 attach/detach | Dozer 已选择"自持 PTY"(`dozerd`),与此架构相反 | workmux 之后第二个独立收敛到"遥控 tmux"的样本,进一步印证"自持 PTY 更重、换来跨前端统一会话模型的自由度"这条已定的取舍是有代价的,但代价被至少两个独立项目证明是可以不付的 |
| 状态存储从单 JSON 文件迁移到 SQLite,`withBusyRetry` + 迁移前备份 + 防"空扫"防御性断言 | `dozerd`"进程重启全丢会话"的已知缺口 | 若走持久化路线,SQL 事务 + 显式防御性断言是比"单文件损坏就删"更重但更抗并发写的选择;需要和 Dozer 现有的"运行时字段不落盘"哲学权衡,不能直接照搬 |
| MCP socket 代理池:JSON-RPC ID 重写 + 单调计数器多路复用 + `sync.Once` 收尸 | Dozer 未定的 MCP 集成方向 | 七个分析对象里第一份有实际深度的"多会话共享 MCP 进程"实现,`waitOnce` 修复过的僵尸进程 bug 提醒转发型代理必须显式设计收尸路径 |
| 巨对象(`Instance` 9,371 行)+ 字符串分支的多 provider 适配,而非 t3code 契约层/workmux trait 式的显式抽象 | Dozer 的"agent 中立"北极星与当前仅适配 Claude Code 的现状之间的落差 | 反面教材:趁早为 provider 专属逻辑设计一层薄接口,比等规模上来后再拆巨对象成本低得多;Agent Deck 自己也在事后专门重构过一次"命令字符串识别 tool"的重复逻辑(issue #1258) |
| hook JSON 树语义合并 + `AGENTDECK_INSTANCE_ID` 环境变量路由 | `dozer-hook` 的安装逻辑 | 与 kooky/workmux 之后的第三个独立样本,"合并/摘除而非覆盖写"这条纪律的说服力進一步增加 |

#### 未能本地核实的部分

- Claude Code 是否真的把 `--fork-session`/`--resume` 当作官方稳定接口维护(本文只核实到 Agent Deck 如何调用,未核实 Claude Code 官方文档对该标志的稳定性承诺)
- Agent Deck 是否有 Zellij/WezTerm/kitty 等 tmux 之外的多路复用器后端支持——浅克隆 + 关键词搜索未发现相关文件,倾向于判断为"无",但未做穷尽式确认
- `internal/fleet/`、`internal/openclaw/`、`internal/selfheal/`、`internal/watcher/`、web 终端桥接(`internal/web`)等外围子系统只看了目录规模,未展开分析——这些不在任务要求的"标准件"清单范围内,体量合计约 3.5 万行,如果后续需要可以单独立项深挖
- 项目整体 commit 数量(浅克隆只能看到 1 个 commit;`gh api contributors` 显示主贡献者 1,652 次贡献,但这是否等于总提交数未交叉验证)

---

### 12.2 claude-squad

> 已合并自原独立报告 `claude-squad-分析.md`。

### Claude Squad 代码级分析

> V2 复核：2026-09-24，源码快照 `ce1ffb4392b01f38e2c4599c7c84d2a93973b138`（最近提交 2026-08-20）。

#### Dozer V2 复核结论

- **保留的独立价值只有一项**：Pause/Resume 将“任务身份”与“占用中的 worktree/进程资源”分离；暂停可删除 worktree 但保留分支，恢复时重建。
- **反例继续成立**：状态依赖终端文本匹配、单文件存储和较薄测试，不适合作为 Dozer daemon 或状态协议模板。
- **对 V2 最有价值**：Task 状态应区分 `active / suspended / archived`；suspended task 可释放 worktree、container、port 和 PTY，同时保留 branch、session lineage、memory 和 artifacts。
- **采用建议**：从持续重点跟踪降为 C 级产品机制参考；若后续没有新的独特机制，可从主调研清单删除，但保留这份历史报告。

> 分析对象:https://github.com/smtg-ai/claude-squad(本地副本 `/Users/chrischiang/AI/claude-squad`)
> 分析日期:2026-07-28 · 最后一次 push 2026-06-17(HEAD `5a604f7`,距分析日约 6 周,活跃度明显低于同系列其他项目)
> Go 9,198 行 / 47 文件 / 9 个测试文件 / 33 个单测 · AGPL-3.0 · 8,195 star(fork 593)

#### 一句话定位

Claude Squad(二进制名 `cs`)是**驾驭已有 agent CLI**(README 宣称 Claude Code/Codex/Gemini/Aider/OpenCode/Amp,但代码实际只对 Claude/Aider/Gemini 三家做了专属文本识别,其余均是"随便一个 shell 命令")的 **Go + Bubbletea TUI + tmux + git worktree** 编排器——和已有的 **workmux 是同一条路线的另一次独立实现**:都不自研终端引擎,都靠 `git worktree` 做"一 agent 一工作区"隔离,都用 tmux 的 detach/attach 机制白拿"会话存活不依赖自己进程"这个免费红利。

**结论先给**:这不是一次"另一种解法"的验证,而是**同一标准件的第五次独立验证,但工程严谨度和范围都明显更薄**——没有 hook 安装(纯文本模式匹配代替结构化状态感知)、没有会话核对/清理机制、没有 sandbox、单文件状态存储、33 个测试(workmux 是 1,532 个)。值得写进文档的不是"claude-squad 引入了什么新机制",而是"claude-squad 用一种更轻/更脆弱的方式解决了 workmux 已经用更硬核方式解决过的同一批问题",以及一两个 workmux 分析里没有明确提到的小而具体的 UI 差异点。

#### 工程结构

```
main.go                入口:解析 --daemon/--reset/--autoyes 等 flag
app/        1,722 行   Bubbletea 主循环(home model)+ 帮助屏
cmd/           32 行   os/exec 包装(可测试的 Executor 接口)
config/       654 行   ~/.claude-squad/config.json(默认程序/AutoYes/轮询间隔/profile)+ state.json(单文件持久化)
daemon/       196 行   --daemon 子进程:轮询所有已存实例做 AutoYes,PID 文件管理生命周期
keys/         134 行   全局按键表(vi 风格 hjkl + 专属键)
log/           76 行   日志
session/      800 行   Instance(状态机)+ Storage(序列化)
session/git   965 行   GitWorktree:创建/清理/commit/push(gh CLI)/dirty 检查
session/tmux  764 行   TmuxSession:唯一的多路复用后端,自带一个 PTY
ui/         2,583 行   list(侧栏)/preview(只读预览)/terminal(内嵌 shell 面板)/diff(diff 视图)/tabbed_window
ui/overlay  1,079 行   confirmationOverlay/branchPicker/profilePicker/textInput
web/        —          纯 Next.js 官网(marketing),与运行时无关
```

对比 workmux 的 178 文件、七万六千行、四个多路复用后端(tmux/Zellij/WezTerm/kitty)+ 独立 sandbox 子系统,claude-squad 是一个**单文件可执行、单一 tmux 后端、无隔离层**的精简实现——体量差 8 倍,不是因为它做得更聚焦,而是因为它砍掉了 workmux 里"hook 结构化合并""三层会话核对""容器/VM 沙盒"这三块工程量最大的子系统,全部换成了更简单(也更脆弱)的替代方案,下面逐条对照。

#### 核心机制一:会话托管——遥控 tmux,但自己也认领了一段 PTY

和 workmux 的判定一致:**detach 的 tmux session 才是真正的执行边界**——`TmuxSession.Start()` 执行 `tmux new-session -d -s <name> -c <workdir> <program>`,agent 进程活在 tmux server 里,`cs` 进程退出、崩溃都不影响 agent 存活,这条和 workmux 完全同构,不重复展开。

真正值得记录的分歧点是**"谁来渲染/转发终端字节流"**:

- workmux 完全不持有 PTY,用户是靠自己已经在跑的 tmux/Zellij/WezTerm 客户端去 attach,workmux 进程只管建窗口、装 hook、读状态。
- claude-squad **在自己的进程内持有一个 PTY**(`session/tmux/pty.go`,`github.com/creack/pty`),但这个 PTY 包的命令不是 agent 本身,而是 `tmux attach-session -t <name>`——即 claude-squad 把"attach 一个已存在的 tmux 会话"这个动作,内嵌进了自己的单一 Bubbletea 进程里,而不是让用户切换到另一个真实终端窗口。`TmuxSession.Attach()` 起两个 goroutine:一个 `io.Copy(os.Stdout, ptmx)` 把 tmux 客户端的渲染输出原样转发到当前终端,另一个读 `os.Stdin` 转发按键给 tmux(并用一个 50ms 窗口"吞掉"attach 瞬间终端自身吐出的控制序列,`Ctrl+Q` 硬编码为退出键)。

后果:claude-squad 的"全屏进入某个 agent 会话"体验是无缝的(不用 `Ctrl+B d` 再切窗口),但代价是这段 PTY 转发逻辑(50ms 窗口猜测控制序列、`panic` 式的 Detach 失败处理"没法恢复,不如让用户重开程序")比 workmux 的"完全甩给用户自己的终端"更脆弱——`Detach()` 里明确写着"如果关闭失败,恐慌好过弄坏用户的终端 pane",说明作者自己也认为这段状态机没有把所有分支想清楚。

**对 Dozer 的意义**:这不是一条可以直接抄的机制,而是一个反例参照——claude-squad 证明了"在单进程里内嵌一段 attach-passthrough PTY 来模拟无缝全屏体验"这条路是可行的,但引入了一类 workmux 完全不用面对的新故障域(控制序列吞吐时序、attach/detach 状态机的边界条件、异常退出要不要 panic)。Dozer 的 `dozerd` 本身就是终端引擎的所有者,不需要"内嵌一个转发层去 attach 外部 tmux",这条分歧点对 Dozer 没有直接借鉴价值,只是确认了"自己造终端 vs 转发别人的终端"这两条路线之间还存在"转发但内嵌"这样一种中间态,而这种中间态工程上并不比两端更省心。

#### 核心机制二:Agent 状态感知——纯文本模式匹配,不装 hook

这是和 workmux 差异最大、也最值得展开的一点。workmux 的状态感知靠**给 8 家 agent CLI 装结构化 hook**(JSON 树语义合并/摘除,幂等,详见 workmux-分析.md),状态从 agent 自己上报。claude-squad **完全没有 hook 安装机制**,`session/tmux/tmux.go` 里状态感知的全部实现是:

```go
if t.program == ProgramClaude {
    hasPrompt = strings.Contains(content, "No, and tell Claude what to do differently")
} else if strings.HasPrefix(t.program, ProgramAider) {
    hasPrompt = strings.Contains(content, "(Y)es/(N)o/(D)on't ask again")
} else if strings.HasPrefix(t.program, ProgramGemini) {
    hasPrompt = strings.Contains(content, "Yes, allow once")
}
```

`HasUpdated()` 每次调用 `tmux capture-pane -p -e -J` 抓取当前渲染文本,对内容做 SHA-256 哈希对比判断"是否变化",再对固定字符串做子串匹配判断"是否出现了确认提示",匹配上就调用 `TapEnter()`(往 PTY 写 `0x0D`)模拟自动确认——这就是 claude-squad 的"yolo/autoyes 模式"全部实现。**没有 hook,没有结构化事件,纯粹是对 agent CLI 渲染出的 UI 文案做字符串包含判断**,只覆盖 Claude/Aider/Gemini 三家,README 宣传的 Codex/OpenCode/Amp 支持在代码层面只是"任意一个可执行命令字符串",没有专属的确认提示识别或 hook 安装,只能靠用户自己在这些 CLI 里配置免确认参数。

这条比 workmux 脆弱在几个具体位置:agent CLI 的 UI 文案换一个版本(比如 Claude Code 把提示语从"No, and tell Claude..."改了措辞)就会让 autoyes 静默失效而不报错;不装 hook 意味着完全没有"回合开始/结束""工具调用"这类语义边界,只有"pane 内容变了"这个粗粒度信号;而且 `TapEnter()` 是无差别地对匹配到的确认提示按回车,如果某次提示实际上是危险操作确认(比如误伤性的 `rm -rf` 二次确认),claude-squad 的 autoyes 模式会无差别地帮用户按下"是"——workmux/Dozer 目前都没有做这类"危险命令强制打断自动模式"的机制,但至少没有像 claude-squad 这样把"看到确认提示就自动点头"当作产品默认可选项来卖(README 的"yolo / auto-accept mode"卖点)。

**对 Dozer 的意义**:这是一次有价值的负面确认,而不是借鉴点——claude-squad 独立验证了"不装 hook、纯靠屏幕文案匹配"这条路径**可以工作但天然脆弱**(耦合 agent CLI 的 UI 文案、无语义边界、无法区分提示类型)。Dozer 已经选择了 hook + 结构化事件这条更重但更稳的路线(`dozer-hook` + `dozerd::agent_state_for`),claude-squad 的实现从反面印证了这个选择的必要性:如果 Dozer 未来要支持一个没有 hook 能力的 agent CLI,退回到"capture 输出 + 字符串匹配"是可行的兜底方案,但应该被当作**明确降级、而非常规路径**来对待,并且"自动确认"这类功能不应该在没有语义分级的情况下无差别启用。

#### 核心机制三:Worktree 与 Pause/Resume 生命周期

和 workmux 结论一致的部分一句话带过:一 agent 一 worktree(`git worktree add -b <branch> <path> <base-commit>`),`Cleanup()` 走 `worktree remove -f` + `branch -D` + `worktree prune`,`PushChanges` 通过 `gh repo sync` 把变更同步到远程分支——这套"worktree 生命周期"的具体 git 命令序列和 workmux 的 `workflow/` 没有本质区别。

值得单独记录的是 claude-squad 特有的 **Pause/Resume** 状态:`Instance.Pause()` 是一个**用户显式触发**的动作(不是自动 reap),行为是:检查 worktree 是否 dirty → dirty 就本地 commit(不 push)→ `DetachSafely()` 断开 attach 的 PTY → `git worktree remove` 删掉工作区目录但保留分支 → 把分支名复制到剪贴板 → 状态置为 `Paused`。`Resume()` 反向操作:检查目标分支当前有没有被 checkout 到别处(避免冲突)→ 重新 `worktree add` 用同一分支 → 如果 tmux session 还在就 `Restore()`(仅重新接上 PTY),不在就整个重新 `Start()`。

这个"Pause 保留分支、丢弃 worktree 目录和磁盘占用,Resume 时按需重建"的模式,workmux 的 `resurrect` 命令覆盖的是相近但更窄的场景(tmux server 重启后恢复 pane 关联),claude-squad 这里把它做成了用户主动触发的、明确对外暴露的一等公民操作,并且专门处理了"worktree 目录/`.git` 文件缺失"(orphaned)这种边缘状态——`IsValidWorktree()` 检测到 orphaned 就跳过 dirty 检查和 `git worktree remove`(这两个操作在 orphaned 状态下都会报错),直接清理残留目录和 git 元数据。这条边界处理是 claude-squad 代码里少数几处体现出"认真考虑过失败模式"的地方。

**对 Dozer 的意义**:Pause/Resume 这个显式的"归还磁盘空间但保留身份(分支名)"生命周期状态,是一个 workmux 分析里没有明确对应物的具体产品设计点——如果 Dozer 未来要支持"agent 会话长期挂起、不占用磁盘/内存,但随时可以按原状态恢复"这类场景(比如用户开了几十个任务但只有几个在跑),claude-squad 这个"保留分支、丢弃 worktree、恢复时重建"的三段式操作序列是一个可以直接参考的小颗粒度设计,比 workmux 的"resurrect"覆盖场景更完整(显式用户操作而非仅限崩溃恢复)。

#### 简短带过:与 workmux 结论一致、不重复展开的部分

- **状态落盘容错哲学**:`config/state.go` 的 `LoadState()` 遇到 JSON 解析失败直接返回 `DefaultState()`(相当于清空实例列表)——"磁盘值不可信"这条哲学方向和 workmux/kooky 一致,但**没有 workmux 的"一 pane 一文件 + 原子写(temp+rename)"**,而是把所有实例序列化进单个 `instances.json` 字段整体重写(`os.WriteFile`,非原子),一次写入中途失败或磁盘满,理论上可能损坏整个实例列表而不只是一条记录。方向一致,工程细节明显更粗糙。
- **会话核对/清理**:**基本不存在**。没有 workmux 的 `boot_id`/PID/command 三层判定,只有零散的 `DoesSessionExist()` 检查(访问某个 pane 时才顺带查一次,查不到就地删缓存重建,`ui/terminal.go:100-144`),没有统一的"启动时批量核对所有已存实例,清理确实已消失的"流程——`FromInstanceData()` 在非 Paused 状态下直接调用 `Start(false)` 尝试 `Restore()` attach 到存盘记录的 tmux 名,如果该会话已经不存在,`Restore()` 返回 error,整条 `LoadInstances()` 直接失败退出,没有"跳过这一条、继续加载其余实例"的降级路径。这是明显弱于 workmux 拉取式 reconciliation 的地方。
- **daemon**:`--daemon` 子进程只做一件事——轮询所有存盘实例,`HasUpdated()` 命中确认提示就 `TapEnter()`,退出前把实例列表存盘。生命周期靠一个 PID 文件(`~/.claude-squad/daemon.pid`)管理,`StopDaemon()` 直接按 PID kill,没有 workmux runtime 信号那样的结构化产出,也没有校验"这个 PID 现在是否真的还是当初启动的那个 daemon 进程"(理论上存在 PID 复用杀错进程的窗口,虽然概率低)。
- **Sandbox/隔离**:全仓库检索 `sandbox`/`docker`/`container`/`lima`/`seccomp` 关键字**零命中**。和 workmux 的容器/VM/RPC 桥/CONNECT 代理形成鲜明对比——claude-squad 完全没有这层,agent 在用户自己的账号权限下直接跑,YOLO 模式没有任何隔离边界兜底。这与 workmux 分析结论一致的部分是"没有 sandbox 也能是个可用产品",但反过来看也印证了 workmux 这块投入的稀缺性:六个同类项目里目前仍然只有 workmux 一家做了这件事。

#### 测试文化:33 个测试,六个项目里(连同 workmux)密度最低的编排层项目

9 个 `_test.go` 文件,`grep -c "^func Test"` 共 33 个,集中在 `worktree_ops_test.go`(孤儿 worktree 清理场景)、`tmux_test.go`(后端探测/命令构造)、`config_test.go`、`app_test.go`(状态机)、`ui/list_test.go`/`preview_test.go`/`terminal_test.go`。没有端到端测试套件(workmux 有独立的 174 个 Python pytest 覆盖真实 tmux/git 交互),纯 Go 单测里也没见到系统性的"穷举分支"风格(workmux 的 `resolve_backend` 全排列测试那种)。9,198 行代码配 33 个测试,测试密度(测试数/千行 ≈ 3.6)远低于 workmux 的 ≈ 17.9(1,358 Rust 测试 / 75,922 行,不算 Python 端到端）,也低于 kooky 的 ≈ 23.9。**这一点上 claude-squad 没有提供任何新信息,只是又一次印证"工程严谨度和 star 数/受欢迎程度不成正比"——8,195 star 排在六个分析对象之外单独看也是最高的,但测试投入是最低的。**

#### 对 Dozer 的启示汇总

大部分机制是 workmux 已验证标准件的更薄实现,新增借鉴点确实有限——这个结论本身就是发现:claude-squad 是第五个独立收敛到"TUI + tmux + git worktree"这套编排范式的项目,进一步提高了这套范式作为"编排层标准配置"的置信度,但它自己在工程细节上没有给 Dozer 带来 workmux 尚未覆盖的正面新知识。

| claude-squad 机制 | 与 workmux 的关系 | 对 Dozer 的意义 |
|---|---|---|
| tmux detach session 做执行边界 | 结论一致,不重复展开 | 再次印证"借用宿主换会话存活"是可行范式,但 Dozer 已选自持 PTY 路线,不适用 |
| 自己持有 PTY 去 attach 外部 tmux(内嵌全屏 passthrough) | workmux 完全不持有 PTY,用户自己的终端 attach | 负面参照:证明"转发但内嵌"是可行的中间态,但引入了新的故障域(控制序列吞吐时序、panic 式异常处理),Dozer 自持终端引擎不需要走这条路 |
| 纯文本模式匹配识别确认提示,无 hook 安装 | workmux 是结构化 hook 语义合并 | 负面确认:印证 hook 路线的必要性——屏幕文案匹配脆弱(耦合 UI 文案版本、无语义边界、无法按危险等级区分),只应作为无 hook 能力时的明确降级方案,不应是默认路径 |
| Pause/Resume(保留分支、丢弃 worktree、按需重建)+ orphaned worktree 边界处理 | workmux 的 `resurrect` 场景更窄(仅崩溃恢复) | 少数真正差异化的小颗粒度设计:若 Dozer 未来要支持"长期挂起会话、按需恢复"场景,这个三段式操作序列和 orphaned 检测逻辑可直接参考 |
| 单文件 `instances.json`、非原子写、加载失败即整体放弃 | workmux 是一 pane 一文件 + 原子写 + 拉取式 reconciliation | 反面教材,不建议参考——再次确认 workmux 那套设计是更值得抄的版本,不需要额外从 claude-squad 学 |
| 无 sandbox | 与 workmux 结论一致(有 vs 没有的对比本身即结论) | 无新增借鉴点,workmux 仍是六个项目里唯一给出隔离参考实现的 |
| 33 测试 / 9,198 行,测试密度六个项目里最低 | 与"workmux 测试密度最高"互为印证 | 无新增借鉴点,只是进一步佐证"star 数不代表工程严谨度",维持 Dozer 已有的测试纪律判断 |

**未能本地核实的部分**:Codex/OpenCode/Amp 在 claude-squad 里的实际运行体验(代码层面只是任意程序字符串,是否有用户自己配置的外部脚本弥补确认提示识别,未追踪到);`gh repo sync` 依赖的具体失败模式(网络/权限报错的用户可见程度)未做实测,只读了源码路径。

---

### 12.3 garcon

> 已合并自原独立报告 `garcon-分析.md`。

### Garcon 代码级分析

> V2 复核：2026-09-24，源码快照 `5642e21f6e3c092f40920227855d51fa16b2f379`。

#### Dozer V2 复核结论

- **仍然成立**：Agent 控制优先走结构化子进程协议，PTY 只承担用户终端；permission prompt 形成双向闭环。
- **当前价值提升**：仓库已有独立 `server-agents/*` 包，Claude/Codex/OpenCode/Factory/直连 API 等适配器的进程边界很适合校准 Dozer `AgentAdapter` contract。
- **对 V2 最有价值**：adapter 输出应统一成 transcript event、permission request、usage、native session id 和 terminal handoff，而不是把供应商原始 JSON 泄漏给 UI。
- **不可照搬**：跨 Agent continuation 仍只能保证可移植 transcript，而非内部推理状态；PR 阅读和工具批准不能冒充验收闭环。
- **采用建议**：A-级 Agent Adapter 参考；许可证带附加条款，除非完成法务核验，不复制代码。

> 分析对象:https://github.com/cfal/garcon(本地副本 `/Users/chrischiang/AI/garcon`,`git clone --depth 1` 一次成功,未遇到大仓卡住的问题)
> 分析日期:2026-07-28 · TypeScript/Svelte 34.6 万行 / 约 1,900 源文件 / 665 个 `*.test.ts` 测试文件(另有 `integration-tests/` 黑盒集成套件,1.6 万行,不计入上面的单元测试数)· LICENSE 文件正文为 GPL-3.0,但 GitHub 的 SPDX 识别结果是 `Other`/`NOASSERTION`(license 文件头部有大段附加条款,不是纯 GPL-3.0 模板,识别器判不准,按仓库自己的说法是 GPL-3.0)· 41 star / 7 fork(2026-02-23 创建,是七个分析对象里最年轻、体量最小的一个,仍在活跃推送)

#### 一句话定位

Garcon 是"a cross-platform agentic coding UI for Claude Code, Codex, and Opencode"(`package.json` description),实际支持面更广:Claude Code、Codex、Cursor Agent、OpenCode、Amp、Factory Droid、Pi 七个 agent CLI,外加直连 Anthropic/OpenAI 兼容端点——和 kooky/orca/workmux/t3code 同一范畴的**编排层**:自托管 browser+mobile 工作区,并行跑多个 agent 会话,不自己重造模型循环。技术栈是 **Bun + SvelteKit(Svelte 5)**,单一 Bun HTTP/WebSocket server(`server/`)作为唯一执行边界,`web/`(SvelteKit,17.1 万行,全仓库最大的一块)是前端,没有独立桌面壳、没有原生移动 app——"手机端"是这个 SvelteKit 应用的 PWA 形态(`web/static/site.webmanifest` + service worker),不是 t3code 那种 Swift/Kotlin 原生客户端。

七个分析对象里,Garcon 和 Dozer 定位重叠度最高:都做"审批"、都碰"git 工作流"。但实测下来,这两处重叠都是**表层重叠、深度不同**——Garcon 的"移动审批"是工具调用级别的 allow/deny 推送(和 Claude Code 自己的 permission-prompt 一一对应),"PR 工作流"是`gh` CLI 的只读包装层(明确写在代码注释里),都不构成 Dozer 那种"目标/标准/verdict/accepted ref"的验收闭环。真正有价值的发现在别处:Garcon 把"结构化协议优先、PTY 只做终端展示"这条路线,在**七个 agent CLI 上全部验证了一遍**,是七个分析对象里这条标准件覆盖面最广的一次实测。

#### 工程结构

```
server/            Bun HTTP/WebSocket server(核心执行边界)
  agents/            agent 注册表、跨 agent 切换、runtime 路由
  chat-execution/     执行状态机(queue/interrupt/abort/carry-over)
  chats/              会话存储、carry-over、fork、分享只读 transcript
  git/                worktree 管理、diff 引擎、行级/hunk 级 staging
  gh/                 gh CLI 只读包装(PR 列表/详情/评论线程)
  notifications/      Telegram 单向提醒 + 前台"需要关注"追踪
  terminals/          bun-pty 驱动的用户可见终端面板(唯一用 PTY 的地方)
  ws/                 WebSocket 传输(chat 流、终端流)
server-agents/      每个 agent CLI 一个 workspace 包
  claude/ codex/ cursor/ opencode/ amp/ factory/ pi/   各自的 CLI 适配器
  interface/          AgentExecution/AgentHost 等公共契约
  common/             execution 工具、native-session 编解码、跨 agent forking
  direct-*-compatible/  Anthropic/OpenAI Chat/OpenAI Responses 直连端点
common/             chat/transport/agent/provider/model/settings 共享契约
web/                SvelteKit(Svelte 5)前端,17.1 万行,全仓库最大板块
integration-tests/  黑盒集成测试(真实 server + 假 OpenAI 兼容端点 + Lightpanda SPA 测试)
```

#### 核心机制一:七个 agent CLI 全部走"结构化子进程管道",PTY 只留给用户终端面板

逐个查了 `server-agents/*/src` 下的传输层:

- **Claude**:`server-agents/claude/src/agents/claude/cli-invocation.ts` 的 `buildClaudeCLIArgs` 拼的是 `claude --print --output-format stream-json --input-format stream-json --replay-user-messages --verbose --permission-prompt-tool stdio`,resume 靠 `--resume=<id>`——走的是 Claude CLI 自己的 JSON 流协议,不是解析终端字节流。
- **Codex**:`server-agents/codex/src/agents/codex/app-server/{client,protocol,runtime,converter}.ts`,和 T3 Code 用的是同一个东西——`codex app-server` 的 JSON-RPC over stdio。
- **Cursor**:`server-agents/cursor/src/acp/client.ts` + `agents/shared/acp-agent-runtime.ts`,走 **Agent Client Protocol(ACP)**——和 T3 Code 的 `packages/effect-acp` 对接的是同一套跨 vendor 标准。
- **Amp/Factory/Pi**:`server-agents/{amp,factory,pi}/src/.../{amp-cli,factory-cli,pi-cli}.ts` 全部是 `Bun.spawn([binary, ...args], {stdout: 'pipe', ...})` + 各自的 `--output-format`/`--stream-json-thinking` 参数,spawn-per-turn(`amp-cli.ts` 文件头注释明写"Uses a spawn-per-turn model: each user message spawns a fresh `amp` process")。

七个适配器,没有一个是"起一个 PTY、往里敲字、拿 OSC/ANSI 扫结果"的路数。**PTY 只在一个地方出现**:`server/terminals/terminal-manager.ts` 用 `bun-pty`(`import type { IPty } from "bun-pty"`),这是给用户看的"打开一个终端"面板,和驾驭 agent 完全是两回事——两条路径在代码里物理隔离,没有混用。

**对 Dozer 的意义**:这直接把"新 agent CLI 先查结构化协议、没有才退回 PTY+hook"这条准则(T3 Code 验证过一次)又验证了一遍,而且覆盖面更大——T3 Code 当时只有 Codex 一个 provider 真正跑通、Claude Code 是"contracts 里占位但未实现",Garcon 是七个 CLI**全部**在生产可用的状态下走通了这条路线,包括 Claude Code(`stream-json` + `--permission-prompt-tool stdio`)。这对 Dozer 有直接参考价值:Dozer 目前对 Claude Code 走的是 PTY + hook + OSC 扫描,是因为规格阶段判断 Claude Code 没有对等的结构化协议;Garcon 的 `cli-invocation.ts` 证明 **Claude Code 其实有 `--output-format stream-json`/`--permission-prompt-tool stdio` 这条路**,如果 Dozer 未来要重新评估"能不能少依赖 PTY+OSC 消毒这一套",这是一份现成的、被验证过的实现参考(尤其是前面刚记过的"macOS GB18030 字体毒化"问题,根源就在终端渲染层——如果控制通道能换成结构化 JSON 流,这类问题的影响面会显著缩小,但這是一个需要重新评估的架构级决策,不是本次分析要下的结论)。

#### 核心机制二:审批推送是真实的双向闭环,但"移动端"是 PWA、Telegram 只是单向提醒

审批(工具调用许可)链路是本次任务要求重点核实的地方,查到的实现相当完整:

1. Claude 侧:`server-agents/claude/src/agents/claude/claude-cli.ts` 里 `#pendingPermissions`(第 470 行起)在收到 CLI 通过 `--permission-prompt-tool stdio` 回调的许可请求时,生成 `permissionRequestId`(第 506 行),转换成 `PermissionRequestMessage` 并塞进这个会话的消息流(第 527-542 行)。
2. 消息类型定义在 `common/chat-types.ts` 第 629-641 行:`PermissionRequestMessage`/`PermissionResolvedMessage`/`PermissionCancelledMessage`,和普通 `ChatMessage` 走同一个联合类型、同一条 WebSocket 通道推送(`server/ws/chat.ts`)——**推送不是轮询**,是和 transcript 同一条流里的一条消息。
3. 客户端(浏览器或手机 PWA)调用许可决定后,回到 `claude-cli.ts` 第 721 行 `resolveInternalToolApproval(permissionRequestId, decision)`,兑现 pending 的 promise,再把结果转成 `PermissionResolvedMessage` 回推(第 744 行)。
4. 超时/放弃语义存在:回合结束、被中止、会话完成时,所有还挂着的许可请求会被批量清空并广播 `PermissionCancelledMessage`(第 470-474、549-554 行),reason 分 `cancelled`/`session-complete`/`aborted` 三种。

但"移动端"和"Telegram 通知"这两处容易被 README 的措辞误导,查代码后要纠正:

- **没有原生 iOS/Android app**。`web/static/site.webmanifest` + service worker(`web/src/service-worker-helpers.ts`)说明这是一个装到手机主屏幕的 **PWA**,底层还是同一个 SvelteKit 应用走同一条 WebSocket——不是 T3 Code 那种带 Swift/Kotlin 原生代码的独立客户端。
- **Telegram 是单向提醒,不能拿来审批**。`server/notifications/telegram.ts` 只调了 `sendMessage`(第 176 行),全文没有 `inline_keyboard`/`callback_query`,`server/notifications/attention-tracker.ts` 的注释也自己写明"Permission request - immediate, deduped by permissionRequestId"(第 5 行)是拿来去重提醒用的,不是审批通道本身——真正点"允许/拒绝"还是要回到 PWA 或浏览器里,Telegram 只是"告诉你有事要处理"。

**对 Dozer 的意义**:这是一份"工具调用许可"场景下推送架构的可用参考(单一消息流承载 transcript + 许可事件、超时/中止有显式广播语义),但**这不是 Dozer 的验收闭环要学的东西**——Garcon 这里做的是"这一步工具调用能不能执行"的运行时门禁,和 Dozer `.dozer/goal.md` + `AcceptanceStore` 要回答的"这一轮/这个任务算不算通过验收"是两个不同层次的问题,前者是执行期的许可,后者是交付期的判定。README 里"approve a blocked step"说的就是前者,不要因为字面上出现"approval"就当成和 Dozer 验收闭环同构。

#### 核心机制三:Git/PR 工作流——worktree 管理是真的,PR 支持明确是只读薄壳

`server/git/worktrees.ts` 里 `createWorktreeOperations().getWorktrees` 真的调 `git worktree list --porcelain`(第 71-77 行),配合 `ref-validation.ts` 的 `assertSafeBranchName`/`assertExistingCommitRef` 做输入校验,是可用的 worktree 管理,不是摆设。`git/diff-engine.ts`(5 万行级)、`review-diff-batch.ts`、`review-document-service.ts` 支撑起 README 说的"stage individual lines, hunks, files, or folders"——这部分是做实的。

但 PR 支持这块,`server/routes/gh.ts` 文件顶部注释写得非常直白:"GitHub pull request routes. **Read-only viewer surface backed by the `gh` CLI**."(第 8 行)。逐一核对 `server/gh/gh-service.ts`:

- `getStatus` → `gh auth status --json hosts`
- `listPullRequests` → `gh pr list --state open --json <fields>`
- `getPullRequest` → `gh pr view` + `gh pr diff` + `gh api .../pulls/{n}/comments`(读 review thread)

全仓库搜索 `gh pr create`、`gh pr merge` 没有任何命中——**Garcon 不自动开 PR,不做合并,没有"合并前必须过审批"这类门禁**。README 里"turn pull request feedback into agent tasks"指的是把读到的 PR/评论内容转成一条发给 agent 的消息(UI 便利动作),不是自动化的 PR 生命周期管理。真正的"提交变更"停留在 git commit/push 这一层,PR 本身要么用户自己去 GitHub 网页开,要么手动敲 `gh pr create`。

**对 Dozer 的意义**:这是这次分析最值得记录的一条差异化确认——Garcon 是七个分析对象里在"PR/git 工作流"这一项做得**看起来**最丰富的(有 worktree、有行级 diff staging、有 PR 阅读器),但一旦查到 `gh.ts` 的注释和 `gh-service.ts` 的方法列表,就能确认它止步于"读 PR + 转发评论"这个薄壳,没有把"验收通过"和"能不能合并/能不能推"绑定起来。这进一步印证了 t3code-分析.md 里已经下的判断:六个(现在是七个)分析对象里没有一个把"验收"做成 Dozer 现在这个深度——Garcon 的 git 能力比 workmux/kooky 都强,但强的是操作面(worktree、diff、staging),不是治理面(谁批准的、依据什么标准、批准结果有没有落成可追溯的 ref)。Dozer 的 `refs/dozer/accepted/<n>` 目前仍然是七个项目里唯一把"验收结果"物化成 git 对象的设计。

#### 核心机制四:跨 agent 会话转移——真实存在,但是"转录文字重放",不是原生上下文迁移

`server/agents/agent-switch-service.ts` 的 `AgentSwitchService#switchAgentCrossAgent`(第 40-100 行)是真实可调用的功能:校验目标 agent 不在运行中(第 44-46 行,`SESSION_BUSY` 门禁),加载源 agent 的渲染后 transcript(`source.transcript.load(...)`,第 48-53 行),再调用 `ownership.transfer(...)` 把这段历史存成一个 `CarryOverSegment` 交给新 agent。

关键在于新 agent 怎么"用"这段历史。`server/chats/chat-carryover-store.ts` 文件头注释直接写明设计意图(第 1-4 行):"A switch persists the outgoing agent's rendered ChatMessage[] as a segment so the conversation stays visible on reload **even though the new native session starts empty**."——也就是说,底层的 Claude/Codex/... 原生会话状态并不会被"迁移"过去,而是 `server-agents/claude/src/agents/claude/execution.ts` 第 82-84 行这么处理:

```ts
command: request.carryOver.length > 0
  ? `${renderTranscriptSeed([...request.carryOver])}\n\n${request.prompt}`
  : request.prompt,
```

把历史 transcript 渲染成一段文字,拼在新 agent 收到的第一条 prompt 前面——本质是"prompt 里塞一段历史摘要",不是结构化的上下文/状态迁移。

**对 Dozer 的意义**:这是任务里明确要求核实的点,结论是"功能真实存在,但比 README 字面读起来的深度浅"——"continue a conversation under another agent"做到了"让新 agent 看到一段文本形式的历史",但做不到"让新 agent 拥有和旧 agent 完全等价的内部状态"(这在 agent CLI 之间本来就是不可能的,因为 Claude/Codex/Cursor 各自的原生 session 格式互不兼容,Garcon 没有假装解决这个不可能的问题,只是老实地退到"文字重放"这条能落地的路)。对 Dozer 没有直接借鉴点(Dozer 目前 spec 没有"跨 agent 转移"这个需求),但作为判断准则记录下来:**遇到"跨 agent 无缝切换"这类宣传语,默认假设它是"transcript 转文字重放"这种务实实现,而不是真的做了状态级迁移**,除非验证到相反的证据。

#### 核心机制五:resume id 持久化——带 owner 校验的编解码器,比"损坏即删"更严格

`server-agents/common/src/native-session/path-native-session.ts` 的 `createPathNativeSessionCodec(ownerId)` 把 `{path, agentSessionId, modelEndpointId}` 编码成一个带 `schemaVersion`(当前 1)和 `ownerId` 的 `AgentNativeSessionRef` 落盘。解码时(第 28-38 行)如果 `ownerId` 或 `schemaVersion` 对不上,直接抛 `AgentIntegrationError('TRANSCRIPT_UNAVAILABLE', ...)`,不是静默丢弃走默认值。

**对 Dozer 的意义**:这是"resume id 持久化"这条标准件的又一次独立验证(六个已分析对象里已多次见到),但在容错哲学上和"运行时字段不落盘、磁盘值不可信、损坏即删"这条 Dozer/已知标准件的默认做法**不完全一致**——Garcon 这里选择的是"格式对不上就报错并要求上层处理",而不是"静默删除退回空状态"。两种做法都合理,分别对应不同的失败代价假设(Garcon 认为"错误地把 A agent 的 native session 当成 B agent 的用"比"抛错打断用户"更危险,所以选择显式失败;"损坏即删"哲学假设的是"能继续跑比较重要,丢一个 resume id 影响有限")。记录为一个值得在 Dozer `dozerd` 做类似编解码器时权衡的分歧点,不是谁更优的定论。

#### 核心机制六:权限模式——同样是"透传底层 CLI 自带旋钮"这条第三路线

`common/chat-modes.ts` 第 4-10 行定义的 `PERMISSION_MODE_VALUES = ['default', 'acceptEdits', 'manualBypass', 'bypassPermissions', 'plan']`,这几个值本身就是 Claude Code CLI 自己的术语(`default`/`acceptEdits`/`plan`/`bypassPermissions`)。`server-agents/common/src/execution/permission-modes.ts` 的 `providerStartupPermissionMode` 把它们映射回 Claude CLI 的 `--permission-mode <mode>` / `--dangerously-skip-permissions` 启动参数——没有自己的风险分级器,也没有自建沙盒,直接把底层 CLI 已经暴露的许可旋钮做成一个统一的下拉菜单。

**对 Dozer 的意义**:这是"命令风险门禁"三条已知路线里的第三条(T3 Code 验证过一次,这次在七个 CLI 上又验证了一遍,证据面更广)——工程量最小、前提是被驾驭的 CLI 自己带了权限参数。Dozer 排这块待办时,同样的判断准则再次被印证:先查 Claude Code/未来的 Codex 适配器自己暴露了哪些权限旋钮,能直接透传的不要重新发明分类器,只有 CLI 本身没给细粒度控制时才需要 jcode/seek_code 那种外挂方案。

#### 未能本地核实/留有不确定性的点

- **用量/限额监控**:只在 Codex 侧(`server-agents/codex/src/agents/codex/app-server/protocol.ts` 第 65、431 行,`'usageLimitExceeded'`/`'usageLimited'`)看到透传自 `codex app-server` 的用量受限事件,Claude 适配器（`claude-cli.ts`）没有查到 usage/cost 字段的处理(只有 `num_turns`)。判断:Garcon 没有自建统一的跨 agent 用量监控仪表盘,现有的用量信号是"provider 自己上报什么就转发什么",覆盖不全,这一条留有不确定性,不排除某处还有未查到的用量统计代码。
- **OSC 7/133 shell 集成**:没有查到 Garcon 依赖 OSC 序列做 cwd/命令边界识别的证据(符合"agent 控制走结构化协议、不碰终端转义"的整体架构),但没有专门验证 `server/terminals/terminal-manager.ts` 这个用户终端面板本身是否处理 OSC 133,这块和"驾驭 agent"无关,不影响本次结论,故未深入查。
- **License 判定**:仓库 `LICENSE` 文件正文是 GPL-3.0(含额外条款),`package.json` 声明 `"license": "GPL-3.0"`,但 GitHub API 返回的 SPDX 识别结果是 `Other`/`NOASSERTION`——这是 GitHub 侧识别器的局限,不是仓库license本身有歧义,按 `package.json`/`LICENSE` 内容判定为 GPL-3.0。

#### 对 Dozer 的启示汇总

| Garcon 机制 | 对应 Dozer 位置/路线图 | 借鉴点 |
|---|---|---|
| 七个 agent CLI(含 Claude Code)全部走结构化子进程 I/O(JSON 流/JSON-RPC/ACP),PTY 只留给用户终端面板 | `dozer-hook`/`dozerd` 的 agent 适配面 | "先查结构化协议、没有才退 PTY+hook"这条准则的第二次实证,而且证明 Claude Code 本身就有 `--output-format stream-json` + `--permission-prompt-tool stdio` 可用,是重新评估 Dozer 控制通道时的现成参考 |
| `PermissionRequestMessage`/`PermissionResolvedMessage` 走同一条 chat WS 流做工具许可推送,超时/中止有显式广播 | `dozer-client` 的连接层、`dozerd` 的会话事件流(如果未来要做运行时许可门禁) | 消息流承载许可事件而非单开一条轮询通道的设计可以参考;但明确不是验收闭环的替代品,不要混淆"工具调用许可"和"任务验收" |
| "移动端"是 PWA、Telegram 是单向提醒,不是审批通道 | Dozer 目前无移动端规划 | 纠偏参考:遇到"移动审批"字面描述时,先查是不是走同一个 web 前端的安装态,而不是假设有独立原生客户端 |
| Git worktree 管理 + 行级/hunk 级 diff staging 做实,PR 支持明确止步于 `gh` 只读包装(`server/routes/gh.ts` 自证注释) | Dozer 已有的验收闭环(goal/criteria/verdict/accepted ref) | 非借鉴点,是差异化再确认——操作面(worktree/diff/staging)做得比 workmux/kooky 都强,但治理面(验收判定物化成 git ref)七个分析对象里仍然只有 Dozer 做到 |
| 跨 agent 会话转移 = 渲染 transcript 转文字、拼进新 agent 首条 prompt(`renderTranscriptSeed`),新原生会话本身"从空开始" | 无直接对应(Dozer spec 未定跨 agent 转移需求) | 判断准则记录:"无缝切换 agent"类宣传语默认按"transcript 转文字重放"核实,不要预设有结构化状态迁移 |
| `PathNativeSessionCodec` 编解码 resume 引用,`ownerId`/`schemaVersion` 不符直接抛错而非静默丢弃 | `dozerd` 未来做类似编解码器时的容错哲学参考 | 和"损坏即删"的默认哲学形成对照组,记录为待权衡的分歧点而非定论 |
| `PermissionMode` 直接复用 Claude Code 自己的模式词汇(`default`/`acceptEdits`/`plan`/`bypassPermissions`) | 命令风险门禁待办(三条已知路线之一) | 工程量最小的治理路线在七个 CLI 上又跑通一次,强化"能透传就不重新发明分类器"的优先级 |

---

### 12.4 goose

> 已合并自原独立报告 `goose-分析.md`。

### Goose 代码级分析

> V2 复核：2026-09-24，源码快照 `80c1197583cc9dc909b7e010c78b4ad58c81e8ce`。

#### Dozer V2 复核结论

- **重要修正**：Goose 已拆出更多独立 crate（agent、context management、providers、ACP 等），旧报告中的具体文件数和模块位置会快速失效；应引用 contract，而不是依赖目录统计。
- **仍然成立**：MCP extension manager、ACP 外部 Agent、Provider registry、多信号 permission inspection 和 SQLite session 是 Dozer 的高价值参照。
- **当前安全演进值得跟踪**：最新提交明确把“prompt classifier 不可用”改为“无信号”，说明 inspector 合并必须区分 `deny`、`allow`、`unknown/error`，不能把分类器失败偷偷解释成安全。
- **对 V2 最有价值**：Policy Engine 应保存每个 inspector 的证据与失败状态，最终 verdict 可解释；MCP server 的 annotation 只是信号，不是可信授权。
- **采用建议**：A 级治理/MCP/ACP 参考；Dozer 继续使用外部 Agent，不复制 Goose 的模型循环。

> 分析对象:https://github.com/aaif-goose/goose(原 `block/goose`,已随项目治理权移交 Linux Foundation 旗下 Agentic AI Foundation〔AAIF〕迁移到 `aaif-goose` 组织下,旧地址会 302 重定向,本次核实以新地址为准)
> 本地副本:`/Users/chrischiang/AI/goose-src`(本环境到 GitHub 的 `git clone`/`codeload` tar 传输反复因连接中断/重置失败,重试多次后以 `codeload` tarball 快照完整落地,过程中同步用 `gh api .../git/trees/main?recursive=1`〔2,910 个条目,非截断〕逐文件核实)
> 分析日期:2026-07-28 · Apache-2.0 · 51,838 star / 5,758 fork(`gh api repos/aaif-goose/goose` 核实)· **Rust**:219,357 行 / 463 个 `.rs` 文件(其中 `crates/goose/src` 下 274 个)/ 2,550 个 `#[test]`+`#[tokio::test]` 用例(另有 26 个独立的 `tests/` 目录集成测试文件)· **TypeScript**(Electron 桌面壳 `ui/desktop` + 终端 UI `ui/text` + SDK):104,633 行 / 589 个 `.ts`/`.tsx` 文件(`ui/desktop` 476 个、`ui/text` 18 个)/ 68 个测试文件、552 处 `it()`/`test()` 用例

#### 一句话定位

Goose 和 jcode/seek_code 同属"agent 本身"范畴——不是驾驭外部 agent CLI 的编排层,而是自带模型循环、自带工具执行的完整 agent。但它与 jcode 走的是相反的产品哲学:jcode 的原则是"自研一切"(自己的 tool-calling 循环、自己的 TUI 渲染引擎、自己的记忆图),Goose 的原则是"**只做通用的 agent 内核,能力靠 MCP 扩展生态和 15+ provider 接入外部世界**"——Goose 自己甚至没有一个原生 Rust GUI,桌面壳是 Electron+React,连它的终端 UI(`goose tui`)都是用 Node/Ink(React for terminal)写的 TypeScript 包,不是 Rust。这使 Goose 成为"Rust 核心 + 非 Rust 壳"这一架构选择目前唯一的实测参照点,和 Dozer"全 Rust、iced 原生 GUI"的选择正好构成一组直接对照。

还有一个在立项之初没预料到的复杂性:Goose 并不是单纯的"agent 本身"——它通过 Agent Client Protocol(ACP)同时扮演两种角色:`goose acp` 可以作为 ACP **server** 被 Zed/JetBrains 这类编辑器直接接入(这时 Goose 是"被驾驭"的一方);而它的 provider 层里又有 6 个文件(`claude_acp.rs`/`codex_acp.rs`/`copilot_acp.rs`/`cursor_agent.rs`/`amp_acp.rs`/`pi_acp.rs`)把 Claude Code、Codex、GitHub Copilot、Cursor Agent、Amp、Pi 这些外部 agent CLI 当作可插拔的"provider"来驱动(这时 Goose 变成了编排层,和 kooky/orca/workmux 是同一件事)。所以准确的定位是:**Goose 的核心自带完整 agent 循环,但同时用同一套 provider 抽象把"自己推理"和"驱动别的 agent CLI"统一了起来**——这是六个分析对象里第一次看到两种范畴在同一产品里合流的实例。

#### 工程结构:Rust 单体 workspace + Electron/Node 双前端

```
crates/
  goose                   核心:agent 循环、provider 抽象、扩展管理、权限/安全巡查、会话存储、ACP 双角色实现(274 个 .rs 源文件,crates/goose/src 下)
  goose-cli               CLI 入口(session/recipe/schedule/review/gateway 等子命令)
  goose-mcp               内置 MCP 扩展(developer 工具实际在 crates/goose/src/agents/platform_extensions 下,goose-mcp 只装 autovisualiser/computercontroller/memory/peekaboo/tutorial 等少数几个)
  goose-provider-types    provider 无关的类型层(GooseMode、ModelConfig、错误类型、规范化的 canonical 模型目录)
  goose-providers         provider trait 与通用实现骨架
  goose-acp-macros        ACP 相关的过程宏
  goose-local-inference   本地推理后端(llama.cpp / MLX——Apple Silicon 专用,与 Dozer mac 先发的定位有交集)
  goose-download-manager  模型/资源下载管理
  goose-sdk(-types)       对外嵌入用的 SDK 类型(对应 README"An API to embed it anywhere"的说法)
ui/
  desktop/                Electron 41 + React 19 + Vite 桌面壳(实际项目里唯一的图形界面)
  text/                   `goose tui` 的真实实现——不是 Rust,是 Node + Ink(React for terminal)写的 .tsx 包,goose-cli 的 tui 子命令只是找到这个 npm 包并 spawn 它
  sdk/                    JS/TS SDK
evals/harbor              评测集
documentation/             Docusaurus 文档站(含 goose-architecture 官方架构说明)
```

`AGENTS.md`(项目自己给贡献者/agent 看的说明)第一句话就是:"goose is an AI agent framework in Rust with CLI and Electron desktop interfaces"——项目自己对"Rust 核心 + Electron 壳"这个二元结构毫不讳言。

#### 核心机制一:Rust 核心与 Electron/Node 壳的边界——对 Dozer 技术选型最直接相关的一节

**边界怎么划的**:`ui/desktop/src/gooseServe.ts` 是 Electron 主进程里管理 Rust 后端生命周期的模块——`findGooseBinaryPath()` 在打包环境下从 `resourcesPath/bin/goose` 找到 Rust 编译出的 `goose` 二进制,`spawn()` 把它当子进程拉起,随机分配一个本地端口和一次性 `serverSecret`,轮询一个健康检查 URL 直到就绪,连接串是 `GOOSED_CERT_FINGERPRINT=` 前缀的证书指纹(TLS,不是明文 HTTP)。也就是说:**Electron 渲染进程和 Rust 核心之间不是走 Electron 原生的 `ipcRenderer`/`ipcMain`,而是走本机回环 HTTP/WebSocket + TLS 证书指纹钉扎 + 一次性共享密钥**——`ipcMain`/`contextBridge`(`ui/desktop/src/preload.ts`,363 行)只用来处理纯操作系统层面的事情(`open-external`、`directory-chooser`、`add-recent-dir` 等,`main.ts` 3,162 行里能看到这些 handler),真正的 agent 对话、工具调用流、会话数据全部通过 HTTP/WS 打给 Rust 后端的 `goosed` server。这个边界划分和 t3code 的"Node.js WebSocket server 作为唯一执行边界"是同一种思路,也是 Dozer `dozer-app`(iced)↔`dozerd`(UDS)边界的另一种实现方式——差别是 Goose 选择了"本机网络+证书钉扎",Dozer 选择了"Unix Domain Socket",后者天然把攻击面限制在同一台机器的文件系统权限内,前者需要额外的密钥/证书机制来达到同等的信任边界,这是 Dozer 现有选择的一个佐证而非需要修改的地方。

**是否后悔没做全 Rust GUI**:能查到明确证据。2026-02-17,外部贡献者 `balcsida` 提交了 PR #7269"feat: migrate desktop app from Electron to Tauri v2",附完整方案:用 Tauri v2 替换 Electron 主进程,新写约 1,600 行 Rust(8 个模块:`lib.rs`/`commands.rs`/`goosed.rs`/`tray.rs`/`menu.rs`/`settings.rs`/`wakelock.rs`/`dock.rs`,42 个 IPC command)去掉当时 2,394 行的 `main.ts`,前端 40+ 个文件通过一个 `tauri-bridge.ts` 兼容层(模拟 `window.electron`)保持不动,还给出了具体收益(免打包 Chromium 省约 150MB 体积、Rust 后端替代 Node 主进程降内存、"Rust alignment: keeps the entire stack in Rust, matching the existing goose codebase")和 46 个单元测试。维护者 `jamadeo` 的回复是关键证据:"a change of this magnitude should be discussed before diving into implementation. I'm not saying it isn't the right move for goose, but the implications... go beyond just a review of the code"——**因为流程问题(未经讨论直接实现)关闭,不是因为技术上被否决**,同日贡献者又开了一个配套讨论 issue(PR 引用的 #7332,但该 issue 在核实时已 404,可能被删除或从未真正建好),此后未见后续合并记录。截至分析时(`main.ts` 已经涨到 3,162 行),Electron 仍是唯一的桌面壳。

**对 Dozer 的意义**:这是三点可以直接拿来对照 Dozer 选型的证据:(1)Goose 官方从未正面反驳"全 Rust 桌面壳"这个方向,只是嫌"事情太大、没走流程"——说明"Rust 核心 + Rust GUI(Tauri/iced 等)"在工程上是可行的,只是切换成本(3000+ 行主进程逻辑、40+ 个 IPC command、17 个 Electron 插件替换)让在位项目难以启动重写,这恰恰是 Dozer 现在(项目还小)就把 GUI 定在 iced 而不是 Electron 的证据——越早定型越不用背这笔迁移债;(2)即便是"更轻"的终端 UI,Goose 也选择用 Node/Ink 写而非 Rust(`ui/text` 全是 .tsx),说明"native 终端渲染"这件事在 Goose 眼里成本高到连 TUI 都外包给了 JS 生态,这是 jcode(自己写终端渲染引擎、自己写 scrollback)路线的反面参照,Dozer 应该记住这是光谱的两端,不是只有一种"正确"选择;(3)本机 HTTP/WS + TLS 指纹 + 一次性密钥这套连接方案,是"进程边界确实需要网络协议,又不想用裸 IPC"时的一份可执行范本,如果 Dozer 未来考虑给 `dozerd` 加一个可选的网络监听面(例如给远程/多设备场景),这是比自造协议更省事的起点。

#### 核心机制二:MCP 扩展系统——70+ 扩展是生态数字,不是 Goose 自研深度

**加载方式**:`ExtensionConfig`(`crates/goose/src/agents/extension.rs`)有 `Stdio`(子进程,`TokioChildProcess`)、`StreamableHttp`(远程 MCP,`StreamableHttpClientTransport`)、`Builtin`(进程内)、`Sse`(已废弃,仅配置兼容)、`Platform`/`Frontend`/`InlinePython` 几种变体,`extension_manager.rs` 统一管理这些客户端的初始化、工具列举、调用转发。**没有找到任何运行时沙盒机制**——不像 workmux 那样有容器/VM 隔离,Stdio 型扩展就是普通子进程,和 agent 本体共享文件系统/网络权限。

**唯一的供应链防护**是 `agents/extension_malware_check.rs` 里的 `OsvChecker`:当扩展是通过 `npx`/`uvx` 安装的包时,调用 [OSV](https://osv.dev)(Open Source Vulnerabilities)数据库查该包名/版本有没有 `MAL-*`(恶意包)标记的安全公告,查到就拒绝安装;**查不到生态类型(非 npx/uvx)就直接放行("fail open")**,这是明确写在注释里的取舍。也就是说 Goose 对"70+ 扩展"生态的治理仅止于"装的时候查一下是不是已知的恶意包",不做运行时行为限制。

内置的 `Builtin` 扩展本身很少——`ui/desktop/src/built-in-extensions.json` / `bundled-extensions.json` 只列了 5-7 个(Developer、Computer Controller、Auto Visualiser、Memory、Tutorial 等),README 里"connect to 70+ extensions via MCP"指的是**外部 MCP 服务器生态**(GitHub、Slack 等第三方或社区维护的 MCP server,通过 `Stdio`/`StreamableHttp` 配置接入),这些服务器本身不是 Goose 专属开发的,任何 MCP 客户端(Claude Desktop、Cursor 等)都能用同一批服务器——**"70+"是 MCP 这个开放协议的生态体量,不是 Goose 在扩展深度上比别人做得更深**。

**对 Dozer 的意义**:jcode/seek_code 都没有对等的 MCP 扩展系统可比较(它们各自的工具是内建的,不是走 MCP 协议),这一节的价值在于纠偏——如果 Dozer 未来考虑"要不要也接入 MCP 生态"作为差异化卖点,先想清楚"MCP 生态数字大"和"MCP 治理深"是两件独立的事:Goose 用了 70+ 这个数字,治理却只到"装包前查一次 OSV 恶意库",这不是 Dozer 应该抄的深度上限,而是提醒"接入 MCP"本身不构成治理能力,治理要另外做(见机制四)。

#### 核心机制三:Provider 抽象——15+ 是保守数字,且把"驱动外部 agent"也纳入了同一套接口

`crates/goose/src/providers/` 下 54 个文件里,刨去 `mod.rs`/`base.rs`/`utils.rs`/`provider_registry.rs`/`provider_secrets.rs`/`provider_test.rs`/`testprovider.rs`/`oauth*.rs`/`*auth.rs`/`catalog_util.rs`/`custom_provider_config.rs`/`usage_estimator.rs`/`toolshim.rs`/`cli_common.rs`/`acp_tooling.rs`/`private_file.rs` 这些基础设施文件,真正意义上的 LLM/推理服务适配至少有 25 个(`anthropic_def`/`openai_def`/`google_def`/`azure`/`bedrock`/`databricks_def`/`databricks_v2_def`/`ollama_def`/`ollama_cloud`/`openrouter`/`huggingface`/`litellm`/`nanogpt`/`sagemaker_tgi`/`snowflake_def`/`tetrate`/`xai`/`gcpvertexai`/`githubcopilot`/`kimicode`/`avian`/`local_inference`/`codex`/`chatgpt_codex`/`gemini_cli` 等),超过 README 宣传的"15+"。另有 6 个文件是通过 ACP 协议驱动**外部 agent CLI**(而不是原始模型 API)的适配器:`claude_acp.rs`/`codex_acp.rs`/`copilot_acp.rs`/`cursor_agent.rs`/`amp_acp.rs`/`pi_acp.rs`。

统一接口在 `goose-providers` crate 的 `Provider`/`ProviderDef` trait(`crates/goose/src/providers/base.rs` re-export)加 `provider_registry.rs` 里的 `ProviderEntry`(元数据 + 构造函数闭包 + 可选清理函数 + 是否支持库存刷新),`normalize_model_config()` 负责把不同厂商的 context 窗口大小/模型元数据统一补全。所有 provider(不管是打真实 API 还是拉起外部 agent CLI)都通过同一个注册表暴露,UI 层不需要区分二者。

**对 Dozer 的意义**:这印证了"provider 抽象"和"driving 外部 agent"在实现上完全可以是同一套接口——Dozer 目前只驱动 Claude Code 一家,如果二期真要扩展到 Codex/其他 CLI,`ProviderDef` 这种"元数据 + 构造闭包 + 统一 trait"的设计,比给每个 agent CLI 写一套平行代码更值得参考;但也要注意 Goose 的 provider 抽象本质是"LLM 请求/响应格式"层面的统一,ACP 适配器能塞进同一接口是因为 ACP 本身就是"agent 对话协议"标准化的产物,这再次印证了 t3code 分析文档里"遇到新 agent CLI 先查有没有结构化协议(ACP/app-server),没有才退回 PTY"这条准则——Goose 的 provider 层就是这条准则的另一个独立实现证据。

#### 核心机制四:命令风险/权限——目前七个分析对象里最系统化的多信号治理管线

这是对 Dozer"治理"定位最直接相关的一节。Goose 的风险控制不是单一分类器,而是一条**多路 inspector 流水线**(`crates/goose/src/tool_inspection.rs`):

```rust
pub enum InspectionAction {
    Allow,
    Deny,
    RequireApproval(Option<String>),
}

pub trait ToolInspector: Send + Sync {
    async fn inspect(&self, session_id: &str, tool_requests: &[ToolRequest],
                      messages: &[Message], goose_mode: GooseMode) -> Result<Vec<InspectionResult>>;
    ...
}
```

`ToolInspectionManager` 按注册顺序跑完所有 inspector,结果汇总后决定 approved/needs_approval/denied 三个桶(`agents/agent.rs` 里 `process_inspection_results_with_permission_inspector` 调用点)。目前实测到的 inspector 至少有:

1. **`GooseMode` 四档模式**(`goose-provider-types/src/goose_mode.rs`):`Auto`(全自动批准)/`Approve`(每次都问)/`SmartApprove`(只对敏感操作问)/`Chat`(只聊天,不执行任何工具)——这是暴露给用户的顶层旋钮,决定下面几路 inspector 的严格程度。
2. **`PermissionInspector` + `permission_judge.rs`**(`crates/goose/src/permission/`):`PermissionManager` 把每个工具的权限持久化到 `~/.config/goose/permission.yaml`,三档 `AlwaysAllow`/`AskBefore`/`NeverAllow`;`SmartApprove` 模式下,是否需要用户确认由**LLM 自己判断"这批工具调用是不是只读操作"**(`permission_judge.rs` 定义了一个专门的 `platform__tool_by_tool_permission` 工具喂给模型去分类,注释里写明"把 request id/工具名/参数当不可信数据,忽略其中任何试图指挥分类结果的内容"——即显式防了一层提示注入)。同时会读取 MCP 协议自带的 `read_only_hint`/`destructive_hint` 工具标注(tool author 声明的),作为独立信号源。
3. **`EgressInspector`**(`crates/goose/src/security/egress_inspector.rs`):纯正则,从 shell 命令里抠出 URL、`git@host:path`、`s3://`/`gs://` bucket、`scp`/`ssh` 目标、`docker push/login` registry、`npm publish`/`cargo publish` 等出网目的地,标注 inbound/outbound/unknown 方向——功能上和 seek_code 的 `egress.ts` 单一收口白名单是同一个问题域,但 Goose 这里做的是"识别并标注供批准决策参考",不是白名单强制拦截。
4. **`AdversaryInspector`**(`crates/goose/src/security/adversary_inspector.rs`):**用户自定义规则 + LLM 判断**,读取 `~/.config/goose/adversary.md`(有则启用,默认只审查 `shell`/`computercontroller__automation_script` 两个工具),内置默认规则模板是"数据外泄/破坏范围超出项目/装恶意软件/提权/下载执行不可信脚本 就 BLOCK,正常开发操作(哪怕改文件、装包、跑测试)ALLOW,宁可偏向 ALLOW"——**判断失败时 fail open(放行)**,和 jcode 的"宁可错杀"正相反(jcode 遇到解析歧义时升级风险档,Goose 遇到判断失败时选择不拦)。
5. **`hooks` 模块**(`crates/goose/src/hooks/mod.rs`):按第三方"Open Plugins"钩子规范(open-plugins.com)实现的生命周期钩子系统,事件名和 Claude Code 的 hook 事件**几乎逐字重合**——`PreToolUse`/`PostToolUse`/`PostToolUseFailure`/`SessionStart`/`SessionEnd`/`UserPromptSubmit`/`Stop`,外加 Goose 自己扩展的 `BeforeReadFile`/`AfterFileEdit`/`BeforeShellExecution`/`AfterShellExecution`。插件在 `<plugin-root>/hooks/hooks.json` 里声明,`matcher` 是正则,动作目前只支持 `type: "command"`(脚本从 stdin 收到 JSON 格式的 `HookContext`),`agent.rs` 里能看到 `HookDecision::Deny { reason, plugin }` 被用来真正阻断工具调用。

这五路信号(模式旋钮、持久化权限表、只读语义分类、出网目的地识别、用户自定义规则)加上 MCP 工具标注,一起喂给同一个决策管线,是本系列(kooky/orca/workmux/jcode/seek_code/t3code)里**目前发现的最系统化的治理架构**——jcode 的 `RiskLevel` 是单一静态分类器,seek_code 的 `dangerousCmd` 是单点强制审批,t3code 是直接透传 Codex 自带的两档旋钮,Goose 是"多信号 + 可插拔 inspector 流水线 + 独立的第三方 hook 规范"三层叠加。

**对 Dozer 的意义**:两点直接可用。第一,`ToolInspector` 这种"多个独立 inspector 各自产出 Allow/Deny/RequireApproval,统一汇总"的架构模式,比单一分类器更适合 Dozer 未来叠加多种判断依据(命令风险 + 出网检测 + 用户自定义规则)时避免耦合成一个巨大的 if-else。第二,也是最值得立刻关注的一点——**Goose 的 hooks 模块对齐的是一个第三方"Open Plugins"钩子规范,事件名和 Claude Code 的 `PreToolUse`/`PostToolUse`/`SessionStart` 等几乎完全一致**,这意味着"跨 agent 的 hook 事件命名"正在形成事实标准。`dozer-hook` 目前订阅的是 Claude Code 自己的 hook 协议,如果未来要适配 Codex/其他 agent CLI,应该去看一眼这个 Open Plugins 规范是不是已经成为该抄的公约数,而不是每接入一个新 agent CLI 就重新设计一遍事件命名。

#### 核心机制五:会话/记忆/子任务——"压缩优先"而非"语义检索优先",与 jcode 形成清晰对照

- **会话持久化**:`crates/goose/src/session/session_manager.rs` 用 SQLite(`sqlx::Pool<Sqlite>`)存储,`SessionManager`/`SessionStorage` 支持创建、按 id 取回(含全部消息)、用量统计、列表分页搜索——是完整的 resume 支持,不是仅追加日志。
- **子任务/subagent**:`crates/goose/src/agents/subagent_execution_tool` + `subagent_handler.rs`,`run_subagent_task()` 递归构造一个完整的 `Agent`(带自己的 `AgentConfig`/`TaskConfig`/`Recipe`),可选择只返回最后一条消息(`return_last_only`),支持取消令牌和消息回调——子任务是"完整 agent 的递归调用",不是 jcode swarm 那种多个平行 agent 互相感知文件变更的协作模型,更接近"任务委派后等结果"的单向 fan-out。
- **记忆**:内置的 `memory` 扩展(`crates/goose-mcp/src/memory/mod.rs`)只是一个按 `category`/`tags` 存取的平铺文件存储(`remember_memory`/相关工具),**没有语义向量、没有图结构、没有相似度检索**——是"模型主动决定记什么、按类目存"的简单键值存储,而不是自动抽取+语义索引。
- **上下文压缩**:`context_mgmt/mod.rs`,`DEFAULT_COMPACTION_THRESHOLD = 0.8`(80% 阈值触发),支持"结构化摘要"(`StructuredSummary`)、工具调用对批量摘要(`TOOLCALL_SUMMARIZATION_BATCH_SIZE = 10`)、手动/自动压缩走不同的续接提示语。**官方架构文档明确写着 Goose 的取舍**(`documentation/docs/goose-architecture/goose-architecture.md`):"goose includes everything **versus a semantic search**"——即 Goose 的哲学是"能塞多少上下文就塞多少,靠算法删旧内容/摘要压缩来省 token",**明确不做语义检索**。

**对 Dozer 的意义**:这一节的价值是给 jcode 记忆子系统的分析提供一个反例校准——jcode 的"语义图 + 检索 + ambient 整理"不是记忆问题的唯一解,Goose(体量、star 数都更大的项目)选择了完全相反的"brute-force 全部塞进去 + 压缩兜底 + 用户主动记录偏好"路线,而且是**显式写进架构文档的立场**,不是没做到位。Dozer 排三期"记忆"设计时,这两个真实存在的路线(jcode 的语义检索 vs Goose 的"全塞 + 压缩")应该并列评估,不能默认语义图是唯一正确答案——如果 Dozer 的记忆需求场景更接近"记住少量用户偏好/项目约定"而非"海量历史语义检索",Goose 这种轻量方案的工程成本明显更低。子任务机制上,Goose"递归调用完整 Agent、等结果返回"这种同步委派模型,也比 jcode swarm 的多 agent 实时协作更接近 Dozer 现有的单会话验收模型,二期如果先做"子任务委派"再考虑"多 agent 协作",Goose 这条路径工程量更小。

#### 对 Dozer 的启示汇总

| Goose 机制 | 对应 Dozer 位置/路线图 | 借鉴点 |
|---|---|---|
| Electron 壳 + Rust `goosed` 通过本机 HTTP/WS + TLS 指纹钉扎通信,连 `ipcMain` 都只管操作系统层面的事 | `dozer-app`(iced)↔`dozerd`(UDS)边界 | 佐证而非否定 Dozer 现有选择:UDS 天然把信任边界收在文件系统权限内,比 Goose 这套"证书指纹+一次性密钥"更省事;如果未来要开网络监听面,这是可执行的最小实现范本 |
| PR #7269/#7331 Electron→Tauri 迁移被"流程原因"关闭,而非技术否决;`ui/text` 终端 UI 也是 Node/Ink 而非 Rust | Dozer 已定的"全 Rust、iced GUI"选型 | 越早把 GUI 定在 Rust 生态,越不用背 Goose 这种"数千行主进程 + 数十个 IPC command"的迁移债;但 Goose 连 TUI 都外包给 JS,说明"全 Rust 渲染"这件事上限不低但工程量也不低,不必因为"人人都这么选"而默认它是唯一正确路径 |
| MCP 扩展只在安装 npx/uvx 包时查一次 OSV 恶意库(fail open),无运行时沙盒 | 若考虑接入 MCP 生态作为差异化 | "70+扩展"是协议生态体量,不是 Goose 自研深度;真正的治理要另外做,不能假设"支持 MCP"本身就等于"扩展是安全的" |
| `ProviderDef` trait 把"调用真实模型 API"和"通过 ACP 驱动外部 agent CLI"统一进同一注册表 | 二期如需接入 Codex/其他 agent CLI | 统一接口设计可参考;同时再次印证"新 agent CLI 优先查结构化协议(ACP/app-server)"这条已由 t3code 验证过的准则 |
| `ToolInspector` 多路流水线(GooseMode 旋钮 + 持久化权限表 + LLM 只读分类 + 正则出网识别 + 用户自定义规则 + MCP 工具标注) | Dozer"命令风险门禁"待办(与 jcode/seek_code/t3code 并列的第四条路线) | 目前七个分析对象里最系统化的治理架构;多信号可插拔流水线的模式值得直接参考 |
| `hooks` 模块对齐第三方 Open Plugins 规范,事件名与 Claude Code hook 协议高度重合 | `dozer-hook` 的事件命名/适配面 | 值得专门调研这个规范是否已成为跨 agent 事实标准,再决定 `dozer-hook` 未来适配新 agent CLI 时是否对齐它,而非每次重新设计 |
| SQLite 会话存储 + 完整 resume;subagent 是"递归完整 Agent、同步等结果"模型 | Dozer 会话存储 / 未来子任务委派设计 | 子任务委派模型比 jcode 的多 agent 实时协作更贴近 Dozer 现有单会话验收假设,工程量更小,可作为二期"先做委派、再考虑协作"的中间步骤参考 |
| 记忆止步于按类目存取的平铺文件,官方文档明确"include everything vs semantic search" | 路线图三期"记忆",目前只有一句话 | jcode 的语义图检索和 Goose 的"brute-force 全塞+压缩"是两个真实存在且都在生产中的路线,需要按 Dozer 实际记忆场景(少量偏好 vs 海量历史检索)选边,不能默认语义图是唯一答案 |

---

### 12.5 jcode

> 已合并自原独立报告 `jcode-分析.md`。

### jcode 代码级分析

> V2 复核：2026-09-24，源码快照 `b659310328b345442226dcdc860527a7e0f2f4b7`。

#### Dozer V2 复核结论

- **旧报告的范畴判断仍成立**：jcode 是完整 Agent harness，不是 Dozer 的整体模板；但当前工程已继续扩展 SSH identity、host key pinning、移动端和 SDK，能力面更加说明“不要在 Host 内重做 Agent”。
- **仍最有价值的子系统**：命令风险按破坏半径分级、memory/compaction、swarm 变更感知、crate 边界和 CI ratchet。
- **对 V2 最有价值**：Code Health 不应只审查用户项目，也应对 Dozer 插件执行依赖边界、panic/error、体积和测试棘轮；权限策略应把 catastrophic 规则置于模型判断之上。
- **边界提醒**：jcode 能强制拦截工具是因为它拥有执行循环。Dozer 对外部 CLI 的强保证必须依靠 Agent 原生协议、PreToolUse hook 或隔离环境，不能只靠 UI 提示。
- **采用建议**：A-级治理与工程文化参考；不引入其 provider/runtime 代码。

> 分析对象:https://github.com/1jehuang/jcode (本地副本 `/Users/chrischiang/AI/jcode`)
> 分析日期:2026-07-28 · Rust 70.7 万行 / 80 crate / 611 测试 / MIT · 12,320 star

#### 一句话定位,以及一个范畴警告

jcode **不是**又一个 kooky/orca/workmux 式的"agent 编排/驾驭层"——它自己就是一个完整的编码 agent harness,**直接对标并替代 Claude Code/Codex CLI 本身**,自带对 20+ 家模型供应商(Anthropic/OpenAI/Gemini/Bedrock/OpenRouter/Copilot/Antigravity/Cursor/DeepSeek/Groq/…)的原生 tool-calling 循环,不需要外部 agent CLI 存在。三个前序分析对象(kooky/orca/workmux)全部"驾驭已有的 agent CLI",jcode 是**要取代它们**。

这个范畴差异直接影响怎么读它:jcode 的整体产品形态(自己做模型循环、自己做 TUI 前端)与 Dozer"agent 中立、站在用户侧治理已有 agent"的路线是**相反**的选择,不构成架构模板;但它 80 个 crate 里有好几块子系统——命令风险分级、agent 记忆、多 agent swarm 协作、RAM 工程——精确对应 Dozer 自己规格里"二期编排/三期记忆"的路线图空位,以及"验收/治理"这个核心定位下还没人做过的具体机制,值得单独拆开看。

代码体量上,jcode 70.7 万行是 workmux(7.6 万)的 9.3 倍、kooky(2.1 万)的 33.7 倍,比 orca(39.4 万)还大 1.8 倍——四个分析对象里规模最大,`jcode-tui` 一个 crate(19.3 万行)就比 workmux 全部代码还多。这个体量对应的是"自己实现完整 agent 循环 + 20 家供应商适配 + 自绘 TUI 渲染引擎 + 语义记忆图 + swarm 协作服务端",是四个项目里唯一同时吃下"agent 循环"和"终端渲染"两座山的。

#### 工程结构:80 个按能力切分的 crate

无单体 `src/`,主 workspace 是薄壳(`src/` 仅 4 个文件),逻辑全在 `crates/*` 里按能力切碎——供应商(`jcode-provider-{anthropic,openai,gemini,bedrock,openrouter,copilot,antigravity,cursor}[-runtime]`)、TUI(`jcode-tui-{core,render,markdown,mermaid,permissions,session-picker,account-picker,usage-overlay,visual-debug,workspace,style,anim,tool-display}`)、记忆(`jcode-memory-types`/`jcode-compaction-core`/`jcode-embedding`)、协作(`jcode-swarm-core`/`jcode-overnight-core`)、平台(`jcode-desktop`/`jcode-desktop2`,GUI 壳;`ios/`,原生 iOS 客户端筹备中)。最大的四个 crate——`jcode-tui`(19.3 万)、`jcode-app-core`(12.9 万)、`jcode-base`(10.1 万)、`jcode-desktop`(7.3 万)——占了总量的一半以上,是真正的核心。

#### 核心机制一:命令风险分级(`jcode-command-risk`)——四个项目里唯一的"治理"实现

这是对 Dozer 最直接相关的一块。`jcode-command-risk` 是一个纯静态、零网络调用的 shell 命令风险分类器,给 agent 每次 `bash` 工具调用做二级门禁的第一级:

```
Safe        无破坏可能,直接放行
Low         破坏有界(工作目录内/可 git 恢复/临时目录下),放行但记录
Confirm     不可逆且越出工作目录,要求模型对着用户原始请求"再证明一次"才放行(reflection turn)
Catastrophic 会摧毁用户主目录/根目录/凭证,硬拒绝,不接受任何模型说辞
```

源码注释直接引用了触发这个设计的真实事故:"issue #604,一个用户丢了整个 home 目录"——`ToolRegistry::execute` 原本只有一个默认关闭的 `pre_tool` hook,模型决定跑 `rm -rf ~` 就会被直接执行。

两个设计决策值得记下来:

1. **按"破坏半径"分类,不按命令名分类**——denylist 挡得住 `rm -rf` 挡不住 `find -delete`、`shred`、`truncate`、`dd`、`> file`。分类器做的是语义判断(会毁掉什么、能不能恢复),不是字符串匹配。
2. **宁可错杀,不可放过(bias toward recall)**——解析有歧义时一律升级到更高风险档,因为"假阳性成本是一次多余的 reflection turn,假阴性成本是一个 home 目录"。

`Confirm` 档不是硬拦截,是**逼模型对着用户真实意图重新自证**(reflection gate,阶段二,只有阶段一非 Safe 才触发,常见安全路径零开销);`Catastrophic` 档才是绝对拒绝,且**不依赖命令解析的正确性**——路径式的黑名单兜底,承认"解析可以被 `sh -c "$(printf ...)"` 这类构造绕过"这一现实,防御纵深不是沙盒。

配套的 `jcode-tui-permissions`(866 行)是待决权限请求的审阅队列 UI——批量列出、逐条批准/拒绝、支持"deny 附理由"。

**对 Dozer 的意义**:kooky/orca/workmux 里没有一个做"agent 要执行的动作值不值得被拦下来"这件事(workmux 的 sandbox 是外部隔离边界,不是判断)。这恰好是 Dozer"治理"定位下应该有、但目前 spec 里完全没提过的一层。关键的可行性问题是:**jcode 能做风险分级是因为它自己就是 tool-calling 循环的执行者,能在执行前拦下来;Dozer 不是**——Dozer 只能通过 Claude Code 的 `PreToolUse` hook(dozer-hook 已经订阅这个事件)拿到工具名和参数,要拦截需要让 hook 返回非零退出码(Claude Code 的 hook 协议支持用退出码 2 阻断工具调用)。这条路径理论上可行但目前 `dozer-hook`/`dozerd` 完全没有走到"根据内容判断、决定阻断"这一步,只是被动记录状态——如果要做,`RiskLevel` 的四档分类逻辑(而不是分级机制本身,机制要重写)是最直接能搬的部分。

#### 核心机制二:Agent 记忆(`jcode-memory-types` + `jcode-compaction-core` + `jcode-embedding`)

对应 Dozer 路线图"三期:自治与记忆、多 agent memory 共享(agent 平台)"这个目前只有一句话的空位,jcode 给出了一份已经在生产里跑的具体实现:

- **写入路径**:每个对话回合被嵌入为语义向量;每隔一定轮次/语义漂移量/会话结束等触发点,一个"记忆侧 agent"(memory sideagent)从对话里抽取记忆条目,写入一张记忆图(`MemoryGraph`,带 `Edge`/`EdgeKind`/`TagEntry`,是图不是扁平列表)
- **读取路径**:每个新回合都对记忆图做一次余弦相似度检索,命中的记忆注入上下文;可选再过一层验证 side-agent 判断相关性,避免"检索到了但不相关"污染上下文
- **主动检索**:harness 提供显式记忆工具(agent 可以主动查/存),外加传统 RAG 式的历史会话搜索,被动注入和主动调用两条路都留了
- **维护**:ambient 模式下定期整理记忆图——查重、查过期、查冲突,不是写入后永久不变
- **配套的上下文压缩**(`jcode-compaction-core`):独立的 token 预算管理,`DEFAULT_TOKEN_BUDGET=200_000` 对齐 Claude 实际上下文上限,80% 阈值触发压缩、95% 阈值触发同步硬压缩(丢老消息保证这次 API 调用不失败)、双档压缩策略(保留最近 N 轮 + emergency 模式砍工具结果/图片到字符上限)——这套参数化的压缩策略本身就是一份可直接参考的"上下文快满了怎么办"的工程清单

**对 Dozer 的意义**:三期"记忆"目前在 Dozer spec 里没有任何设计细节,jcode 这套"图结构记忆 + 语义检索 + 侧 agent 验证 + ambient 整理"的架构是目前四个分析对象里唯一给出记忆子系统参考实现的,排三期设计时应该回来看这一节,而不是从零推演。

#### 核心机制三:Swarm 多 agent 协作(`jcode-swarm-core` + `jcode-overnight-core`)

对应路线图"二期:编排与资产(orca 视野)"。设计比 orca 分析文档描述的编排更进一步——orca 是"人类在调度台上开 worktree、指派任务",jcode 的 swarm 是**agent 之间直接协作**:

- 同仓库内起两个以上 agent,服务端自动管理;A 改了 B 读过的文件("代码在脚下位移"),服务端主动通知 B,B 自行判断是否需要看 diff、要不要处理冲突
- 消息能力:DM 单个 agent、广播给所有 agent、广播给同仓库内的 agent 三种粒度
- agent 自己可以再 spawn 子 swarm(`MAX_SWARM_MEMBERS=1000`),主 agent 自动变成协调者、子 agent 变成 worker——可以无头(headless)或有头运行
- 完成报告有强制摘要约束(`SWARM_TLDR_REQUIRED_OVER_CHARS=240` 时必须附 ≤200 字符的 `tldr`),防止长报告直接倾倒进其他 agent/协调者的上下文

**对 Dozer 的意义**:这是"文件层面的实时协作感知"(而不是"开 worktree 完全隔离,merge 时才碰面")的具体实现,和 kooky/orca/workmux 共同验证的"一 agent 一 worktree,靠 git 隔离"标准件是**不同的哲学**——jcode 的立场是"worktree 不是给多 agent 协作场景设计的好方案"(README 原话:"git 显然不是为多 agent workflow 设计的,worktree 不是好方案"),二期设计时这是一个值得认真考虑的反方意见,不能因为前三个项目都用 worktree 就当成定论。

#### 核心机制四:RAM/性能工程——为什么它自称"最省内存的 harness"

README 给了可验证的基准表(vs. pi/Codex CLI/OpenCode/Copilot CLI/Cursor Agent/Claude Code/Antigravity CLI):单会话 jcode 27.8MB,追加会话每个仅 +9.9~10.4MB;对照组里 Claude Code 单会话 212.7MB(21.5 倍)、OpenCode 追加会话 318.4MB(32.2 倍)。做到这一点的手段:

- 自己实现 scrollback(不满足于原生终端滚动能力,理由是"想要比原生更强的能力,比如局部平滑滚动")
- 因为自定义 scrollback 撞到了终端本身的能力天花板,**索性开始自己写一个终端**(`Handterm`,https://github.com/1jehuang/handterm,独立仓库,WIP)
- 自研 mermaid 渲染库(`mermaid-rs-renderer`),纯 Rust 无浏览器/TypeScript 依赖,号称比现有方案快 1800 倍
- "info widget"(信息控件只占屏幕负空间,没内容就自动让位)、千帧渲染上限(避免闪烁,不是为了真的跑千帧)

**对 Dozer 的意义**:这条不是"抄具体实现"的意义,是"工程决心"的参照——Dozer 已经在 canvas 逐格定位这件事上验证了同样的态度(为了终端渲染正确性,放弃更省事的 rich_text 方案)。jcode 把这条路走得更远(连滚动都不满足于原生终端,连 mermaid 渲染都要自己写),说明"native 终端 UI 这条路能走多深"这件事上限比想象中高,如果 Dozer 后续要打磨终端体验,jcode 的取舍(自绘 scrollback、自绘图表渲染)是可以对标的天花板参照,不是必须抄的下限。

#### 工程文化:护栏比测试更早生效

`AGENTS.md` 里 `scripts/check_guardrails.sh` 是 CI 强制的"格式+质量护栏":fmt、`clippy -D warnings`,外加warning 数量、代码体积、测试体积、panic 使用、吞掉的 error、依赖边界、通配符 re-export 这几项的 **ratchet(棘轮)**——只能变好不能变差,新增违规直接挡 CI,但存量问题不需要一次清零(`--fix` 用于有意增长时重新定基线)。这是 707K 行 / 80 crate 规模下防止腐化的具体机制,不是抽象原则。

**对 Dozer 的意义**:Dozer 现在 1 万行,规模小,还没到"需要棘轮防腐化"的痛点,但这类机制**越早引入成本越低**——等到规模膨胀到需要它的时候,存量违规已经多到没法一次清零了。工程文化上的启示:priority 应该是"现在开始积累这类护栏脚本",而不是"等规模大了再补"。

#### 一个明确的反例:jcode 尝试过"包一层 Claude Code CLI",后来放弃了

`crates/jcode-provider-claude-cli-runtime` 的文件头注释写着"Deprecated Claude CLI provider runtime (subprocess transport)"——jcode 一度用子进程方式包装真实的 Claude Code CLI 作为一个 provider,后来废弃,转向直接对接 Anthropic API 自己做 tool-calling 循环。同时它保留了"读取其他 harness(Codex/Claude Code/OpenCode/pi)的会话文件、在 jcode 里继续跑"这个会话吸收能力——**包一层 vs 吃进来重做**,jcode 试过前者、选择了后者。

这对 Dozer 是一个值得记住的反面参照:Dozer 的路线是"包一层"(driving 已有 agent CLI,不重做 agent 循环),jcode 用真实产品决策验证了这条路径**存在被放弃的先例**——原因大概率是"包一层"在深度控制力(比如这里分析的命令风险分级、精细化的上下文压缩)上有天花板,子进程边界挡住了很多东西。这不构成"Dozer 应该重新考虑agent 中立"的理由(定位不同,Dozer 的价值在治理层不在模型循环),但如果未来"治理"想做得更深(比如真正意义上的执行前拦截,而不是 hook 退出码这种间接手段),会撞到 jcode 撞过的同一堵墙,提前知道这一点比临时发现更好。

#### 对 Dozer 的启示汇总

| jcode 子系统 | 对应 Dozer 位置/路线图 | 借鉴点 |
|---|---|---|
| `jcode-command-risk` 四档风险分级 | spec 里"验收/治理"定位,目前无对应机制 | 分级逻辑(按破坏半径而非命令名、宁可多问不可漏判)可直接参考;执行机制需要重新设计,因为 Dozer 没有 jcode 那样的执行前拦截点,只能走 `PreToolUse` hook 退出码这条更弱的路径 |
| `jcode-memory-types`/`compaction-core` | 路线图三期"自治与记忆",目前只有一句话 | 排三期设计时的具体参考起点:图结构记忆 + 语义检索 + 侧 agent 验证 + ambient 整理;压缩策略的分级阈值(80%/95%)可直接参考 |
| `jcode-swarm-core` | 路线图二期"编排与资产",目前只有一句话 | 文件层面实时协作感知(而非纯 worktree 隔离)是二期设计时应该纳入对比的另一种范式,不要默认"一 agent 一 worktree"是唯一答案 |
| RAM 工程(自绘 scrollback/终端/mermaid) | Dozer 已验证的"canvas 逐格定位"同类态度 | 不是具体代码可抄,是"这条路能走多深"的参照上限 |
| `scripts/check_guardrails.sh` 棘轮护栏 | Dozer 当前工程文化 | 规模小的现在就该开始引入,越晚成本越高 |
| claude-cli-runtime 被废弃的历史 | Dozer"agent 中立、包一层"的路线选择 | 不是否定依据,是"这条路的天花板在哪"的提前预警——想在治理深度上更进一步时会撞到同一堵墙 |

---

### 12.6 kooky

> 已合并自原独立报告 `kooky-分析.md`。

### Kooky 代码级分析

> V2 复核：2026-09-24，源码快照 `42e1bb5a80ab05dbee6d020316e1c34f4af5fcea`。原报告基于 v0.35.0；当前已演进到包含 `WorktreeManager`、Codex usage/session 恢复和进程环境探测的新版本。

#### Dozer V2 复核结论

- **修正旧结论**：Kooky 已不再只是“终端 + 会话壳”，当前源码已有 worktree 创建/删除/列举、Git watcher、Agent session history、Codex usage monitor，因此它现在也能参考 Task–Worktree–Session 绑定。
- **仍然成立**：独立无状态 hook、小型 payload crate、surface id 环境变量路由、OSC 7/133 语义事件、Pre/Post tool call 配对，是 Dozer `dozer-hook → dozerd` 最直接的实现参照。
- **对 V2 最有价值**：把 `AgentTemplate` 保持为数据，将 resume 语法、prompt flag、是否报告 tool call 等差异留在 adapter；Host 不写 `if agent == ...`。
- **不可照搬**：Kooky 仍是 Swift/macOS/libghostty 产品，UI、终端引擎和平台集成不能成为 Dozer 插件 ABI。
- **采用建议**：保留为 A 级机制参考；重点复核 `HookServer.swift`、`AgentTemplate.swift`、`WorktreeManager.swift`、`AgentSessionHistory.swift`，不形成代码依赖。

> 分析对象:https://github.com/iAmCorey/kooky (本地副本 `/Users/chrischiang/AI/kooky`)
> 分析日期:2026-07-14 · 版本 v0.35.0 · Swift 21K 行 / 63 文件 / 502 测试 / MIT

#### 一句话定位

Kooky 是 **libghostty(Ghostty 终端的 C 内核)之上的一层 AI 工作流管理壳**,不是从零写的终端模拟器。真正的终端能力(VT 解析、GPU/Metal 渲染、PTY、滚动回看、搜索)全部来自 `Vendor/GhosttyKit.xcframework` 二进制依赖;自己的 2 万行代码全花在:会话/窗格模型、agent 状态感知、shell 集成、持久化和 SwiftUI 界面。

#### 工程结构(Package.swift 四个 target)

| Target | 作用 | 关键点 |
|--------|------|--------|
| `Kooky` | 只有 main.swift 的薄可执行文件 | 逻辑全在 KookyKit(SPM 不允许测试导入 executable) |
| `KookyKit` | 全部应用逻辑 | 链接 GhosttyKit + Metal 等系统框架 |
| `KookyHook` | 独立小 CLI,被 agent 的 hook 调用 | **刻意不链接 KookyKit**,保持启动快、零依赖 |
| `KookyHookKit` | hook 的 payload 构造/解析 | 抽出来是为了不起子进程就能单测 |

"厚 lib + 薄 exe + 独立 hook 小工具"的拆分本身就值得借鉴。

#### 核心机制一:终端引擎抽象(`Terminal/TerminalEngine.swift`)

`TerminalEngine` 是 protocol,`LibghosttyEngine` 是唯一实现。接口全是**回调驱动的语义事件**,不是原始字节流:

- `onPwdChange` — shell 发 OSC 7 时触发(cwd 跟踪的唯一来源)
- `onCommandFinished(exitCode, duration)` — OSC 133;D(shell integration 协议)
- `onTitleChange` — OSC 0/2,显示 ssh 远端标题,也被用作**远程 agent 状态的带内信道**
- `onUserInput`、`onFocus`、搜索生命周期;`sendInput`/`paste` 分离(paste 走 bracketed-paste)

工程化细节:`beginSizePropagationSuspension()` 是**引用计数的**——窗格缩放动画期间挂起 resize 传播,否则一次动画触发 12–24 次 SIGWINCH 会把 conda 用户的 scrollback 冲掉。终端嵌入的坑主要在 resize/焦点/剪贴板,不在渲染。

#### 核心机制二:Agent 状态感知(全项目最有含金量的设计)

双通道设计:

##### 通道 1:unix socket + hook 小工具(本地 agent)

1. kooky 启动每个 tab 时注入环境变量 `KOOKY_SURFACE_ID=<session UUID>`
2. kooky 给 Claude Code 写一份 `--settings` hooks 配置,让 Claude 在 Stop/UserPromptSubmit/Notification/PreToolUse/PostToolUse 等事件时调用 `kooky-hook <agent> <event>`
3. `kooky-hook` 读环境变量里的 surface id,连 `~/Library/Application Support/kooky/socket`,**写一行 JSON 就退出**(fork-per-event,完全无状态)
4. 主 app 的 `HookServer` 用 DispatchSource accept,单次 read 4KiB,解析后按 UUID 路由到对应 Session

无状态 hook 意味着**配对逻辑全在 app 侧**:`Session.recordToolCallEnd` 优先用 `tool_use_id` 匹配 Pre/Post(并发同名工具调用只有这个是正确身份),回退到 (toolName, identifier);另有 5 秒一次的 orphan 扫描,60 秒没等到 Post 的调用标记为 `stalled`(Claude 崩了/断网)。

退出码契约:0 = 成功或"不在 kooky 里跑,别重试";1 = IPC 失败,shell 侧不推进去重缓存,下个 prompt 重试。

##### 通道 2:OSC 标题标记(SSH 远端)

远端机器上没法连本地 socket,让远端 shell 发特制 OSC 标题序列,kooky 在本地终端字节流里识别,设置 `transientAgent`/`remoteHost`。带内信道有污染风险(`claude -p > out` 重定向时不能吐 OSC 字节),所以有 tty 检测门控。

##### 会话恢复

Claude 的 hook JSON 带 `session_id`,kooky 持久化为 `conversationId`,下次启动拼 `--resume <id>` 无缝续聊。Agent 定义(`AgentTemplate`)是纯数据:`initialCommand`、`promptLaunchFlag`(如 Copilot 的 `-p`)、`resumeFlag`(Claude 的 `--resume`)、`reportsToolCalls`——加一个 agent 就是加一条记录。

#### 核心机制三:Shell 集成(1853 行,坑最多的模块)

需求:shell 在 cd 时发 OSC 7、命令结束发 OSC 133、启动时自动执行 agent 命令但退出后留下干净的 shell。三种 shell 三套方案:

- **zsh**:`ZDOTDIR` 劫持——指向 kooky 的临时目录,里面的 `.zshrc` 先恢复用户原始 ZDOTDIR、source 用户真实的 `.zshenv/.zprofile/.zshrc`,再装 `chpwd`/`precmd` 钩子,最后 `eval $KOOKY_AGENT` 内联启动 agent
- **bash**:launcher 脚本 re-exec(libghostty 强制 login shell,`--rcfile` 语义会被剥掉)
- **fish**:往 `XDG_DATA_DIRS` 前插目录,靠 `vendor_conf.d` 自动加载——不用 `-C` 是因为 Fig/Amazon Q 这类包壳工具会吞掉后者

**shell 集成是此类产品真正的工程成本所在,且方案与实现语言完全无关**——用 Rust 重写时这套脚本可以原样移植。

#### 数据模型与持久化

```
窗口 → WorkspaceStore(1730 行,中枢/所有事件的汇聚点)
  └─ Workspace(一个项目目录,可以是 git worktree 或 SSH 远端)
      └─ PaneNode 二叉分割树(.pane 叶子 | .split(方向, 左, 右, 比例))
          └─ Pane → tabs: [Session]
              └─ Session(engine + agent + cwd + git 状态 + 工具调用事件流…)
```

- 持久化(`Persistence.swift`)只存元数据到 `state.json`:PTY 状态无法跨进程存活,重启时按存的 cwd 重新 spawn 引擎、拼 `--resume`
- 运行时字段(activityState、toolCallEvents、搜索状态)**从 Codable 结构里整体缺席**,靠类型系统而非运行时判断保证不落盘
- 新增字段一律 Optional 以兼容旧 state 文件;"从磁盘读的值要 clamp,不能信任"
- git 状态是 **shell 出去调 git CLI**:读路径 1 秒超时 + 丢弃 stderr,写路径(worktree)完整错误透传——同一件事两种容错策略
- 前台进程环境(sysinfo 式)只做兜底,真值来自 prompt hook 上报(`nvm use`/`activate` 改的是 shell 内存,内核 proc env 快照是过期的)

#### 用 Rust 实现类似项目的考量

##### 第 0 个决策:产品形态

- **路线 A:TUI 复用器(推荐起点)**——跑在用户现有终端里,像 Zellij。`ratatui` + `portable-pty` + VT 解析器。核心价值(agent 状态感知、workspace/worktree、hook 集成)一分不丢,砍掉 GPU 渲染、窗口管理、剪贴板/IME 这些 Rust 生态最痛的部分
- **路线 B:原生 GUI 终端**——FFI 嵌 libghostty(C ABI,Rust 调用和 Swift 一样可行),或 `alacritty_terminal` + `wgpu` 自绘(Zed 的做法)。工作量是路线 A 的 3–5 倍
- **路线 C:Tauri + xterm.js**——最快出 demo,但 GPU 加速/低延迟的卖点没了,不建议

##### 子系统 → Rust 映射

| kooky 子系统 | Rust 对应 | 备注 |
|---|---|---|
| libghostty 嵌入 | FFI(bindgen)或 `alacritty_terminal`/`termwiz` | C ABI 回调 → `Box<dyn Fn>` + userdata |
| PTY spawn | `portable-pty`(wezterm 出品) | 跨平台,含 Windows ConPTY |
| `TerminalEngine` protocol | trait + 事件 enum + mpsc channel | `enum EngineEvent { PwdChange(String), … }` 比逐个回调字段更惯用 |
| PaneNode 二叉树 | `enum PaneContent { Pane(Pane), Split{ … Box<PaneNode> … } }` | Swift 的 `indirect case` 就是在模仿 Rust enum |
| HookServer unix socket | `tokio::net::UnixListener` + serde_json 按行 | 几十行的事 |
| KookyHook CLI | 独立 bin,**只用 std** | 启动速度是硬指标(每个 hook 事件 fork 一次);保留 0/1 退出码契约 |
| @Observable UI 响应 | 无等价物:state 变更 → 事件 → 重绘 | 从 Swift 迁移最大的范式差异 |
| @MainActor 单线程 | tokio 单 runtime + actor 风格(状态归一个 task,channel 通信) | 别用 `Arc<Mutex<World>>` |
| Persistence | serde_json + `#[serde(default)]` | 运行时字段不派生 Serialize 即天然不落盘 |
| git 状态/worktree | **照抄:shell 出去调 git**,`tokio::process` + timeout | 别上 git2/gix,status 语义对齐 CLI 很难 |
| 目录监控 | `notify` crate | fsevents 后端现成 |
| 前台进程信息 | `sysinfo` / `libproc` | |
| shell 集成脚本 | **原样移植** | 与宿主语言无关,直接省掉最大一块试错成本 |

##### 值得原样继承的设计决策

1. **hook 小工具无状态、配对逻辑在主进程**——fork-per-event 天然崩溃安全,orphan 扫描(60s 判 stalled)状态机照搬
2. **`KOOKY_SURFACE_ID` 环境变量路由**——一个 env var 解决"哪个 tab 的 agent 在说话",多 tab 多 agent 不串线
3. **AgentTemplate 纯数据化**——支持新 agent = 加一条配置记录(与 byteboy 的 `[agents]` TOML 理念一致)
4. **双信道**——本地 socket(可靠、结构化)+ 远程 OSC 带内标记(唯一穿透 ssh 的通道),重定向场景做 tty 门控
5. **运行时状态与持久化状态用类型区分**,新字段一律可缺省

##### 工作量判断

21K 行、502 测试、93 个 release 中,"能跑的 demo"(PTY + 分屏 + 启 agent + socket 状态点)只占 15–20%,**其余 80% 全是 shell 集成、resize、粘贴、焦点、恢复这类边角打磨**。kooky 的注释密度极高,等于附赠一份踩坑地图;但这些成本躲不掉,只能靠选路线 A(TUI)把终端渲染那一半外包给用户自己的终端。

---

### 12.7 opencode

> 已合并自原独立报告 `opencode-分析.md`。

### OpenCode 代码级分析

> V2 复核：2026-09-24，源码快照 `0027387dc5c59793c12dfc531abc78f825ed6868`。

#### Dozer V2 复核结论

- **重要修正**：当前仓库已具有独立 `packages/protocol`、桌面端、TUI、App、SDK 和重新组织的 LLM/protocol 层；旧报告关于 workspace 数量、目录和具体实现位置只能视为历史快照。
- **仍然成立**：统一协议、权限阻塞、MCP、session fork、SQLite schema 演进和受限 subagent，是 Dozer 的局部参考；Dozer 不应进入模型供应商适配层。
- **对 V2 最有价值**：Plugin/Agent protocol 应形成单独 package/crate，并覆盖 permission、PTY、session、event 等稳定 group；UI 通过生成或强类型 client 使用，不直接依赖 daemon 内部对象。
- **新增启示**：session fork 已成为主流交互，Dozer 数据模型应显式保存 `parent_run_id / parent_session_id / fork_point`，而不只保存 resume id。
- **采用建议**：A-级协议、权限和 session 模型参考；供应商兼容代码不复用。

> 分析对象:https://github.com/anomalyco/opencode(原 `sst/opencode`,`gh repo view sst/opencode` 已确认重定向到同一仓库,owner ID 一致,不是两个不同项目;仓库描述"The open source coding agent")
> 本地副本:`/Users/chrischiang/AI/opencode`(`git clone --depth 1`,默认分支 `dev`,不是 `main`)
> 分析日期:2026-07-28 · TypeScript/TSX 60.6 万行(3,137 个 `.ts`/`.tsx` 文件,657 个测试文件)/ 32 个 workspace 包 / MIT / 190,420 star、24,190 fork(2025-04-30 创建,分析当天仍在推送)

#### 一句话定位

OpenCode 和 jcode/seek_code 同属**"agent 本身"**范畴——自己做 tool-calling 循环、自己对接模型 API、不依赖外部 agent CLI,而不是 kooky/orca/workmux/t3code 那种"驾驭已有 CLI"的编排层。三者体量差异巨大且有代表性:jcode(70.7 万行 Rust)是"重工程但个人/小团队维护"的极致,seek_code(1.4 万行 TypeScript)是"够用就好"的个人规模,**OpenCode(60.6 万行 TypeScript,SST 团队 + 大量外部贡献者维护,32 个 workspace 包)是"主流、被广泛采用、工程化程度最高"的那一极**——体量接近 jcode 但组织方式完全不同:jcode 是单一作者把所有子系统(供应商适配、TUI 渲染引擎、记忆图、swarm 协作)都从零手写;OpenCode 则大量借力外部生态(Vercel AI SDK、models.dev 模型目录、Effect 生态、Agent Client Protocol 标准),自己的核心代码集中在"把这些拼起来还要处理好几十种供应商的私有怪癖"这件事上。这是本系列第一次拿到一个 star 数两个数量级于 jcode/seek_code 的"agent 本身"项目做对照,填的是"主流答案长什么样"这个空。

**一个先澄清的事实核实**:分析前不确定 OpenCode 是否曾用 Go 实现(有传闻),本地代码库里 `.go` 文件数为 0,`packages/tui` 现在是 `.tsx`(用 SST 自研的终端渲染库 `opentui`,`package.json` 里有 `upgrade-opentui` 脚本),运行时是 **Bun**(`packageManager: "bun@1.3.14"`,`@effect/sql-sqlite-bun` 做 SQLite 层)而不是 Node。历史上是否有过 Go 版 TUI 未能本地核实(仓库是 `--depth 1` 浅克隆,没有完整历史),但**至少当前这份代码是纯 TypeScript**,这一点是实测确认的。

#### 工程结构

`packages/*` 32 个包,按 TS/TSX 行数排序前几名:

| 包 | 行数 | 角色 |
|---|---|---|
| `opencode` | 17.5 万 | 核心引擎:session/tool/provider/agent/permission/acp/mcp/lsp/snapshot,CLI 入口 |
| `app` | 11.5 万 | Web GUI(Vite),desktop 复用它的 `desktop-menu`/`updater` 导出 |
| `core` | 6.8 万 | 跨包共享:数据库/session-sql/provider 类型/Effect 工具层,`server`←`core`←`protocol`←`schema` 的依赖方向被 `AGENTS.md` 显式强制 |
| `console` | 4.1 万 | 企业管理台(SST 云側) |
| `tui` | 3.2 万 | 终端 UI,`opentui`(自研 React-for-terminal)+ `.tsx` |
| `sdk` | 3.0 万 | 生成的 HTTP 客户端(`OpencodeClient`),CLI/TUI/ACP/插件共用同一个 |
| `ui`/`session-ui` | 2.4 万+2.1 万 | web/desktop 共享的渲染组件 |
| `llm` | 2.1 万 | 实验性"原生"模型调用运行时(绕过 Vercel AI SDK) |
| `codemode` | 1.1 万 | 实验性"code mode"(agent 写代码调工具,而非逐次单工具调用) |
| `desktop` | 0.78 万 | Electron 壳(electron-vite/electron-builder),体量远小于 `app`——同一种"桌面只是薄壳"模式在这里又出现一次 |
| `server` | 0.17 万 | HTTP 路由装配层,业务逻辑不在这里,只是把 `opencode`/`core` 的 service 挂到 Effect `HttpApiBuilder` 上 |

此外 `sdks/vscode`(独立目录,`src/extension.ts`)是 VSCode 扩展源码;根目录 `.zed` 是开发者自己的编辑器配置,不是 Zed 集成代码本身,真正的 Zed/ACP 集成在 `packages/opencode/src/acp`。全项目大量使用 **Effect**(`Context.Service`+`Layer`+`Effect.gen`),`AGENTS.md` 明确写了依赖方向纪律("Schema → Core/Protocol → Server","Client runtime 不得依赖 Core/Server")——这和 T3 Code 分析里记的"用严肃效果系统写后端"是同一个技术选择,但 OpenCode 的使用面更广、包数更多,依赖方向被写成显式规则而不只是约定。

#### 核心机制一:Client/Server 架构——HTTP+SSE 为主,WebSocket 只管 PTY,和 T3 Code 不同构

`packages/server/src/routes.ts` 用 Effect 的 `HttpApiBuilder.layer(Api, { openapiPath: "/openapi.json" })` 装配路由——**是一个有 OpenAPI 描述的类型化 HTTP API**,不是裸 WebSocket server。实时事件走 `packages/opencode/src/server/routes/instance/httpapi/groups/event.ts` 里的 `GET /event`,声明为 `HttpApiSchema.asText({ contentType: "text/event-stream" })`——**SSE**,单向服务器推送。真正用 WebSocket 的地方是 `.../httpapi/handlers/pty.ts` + `websocket-tracker.ts`,专门给内嵌终端面板用,和 T3 Code 里"`apps/server/src/terminal` 大概率是辅助面板不是控制通道"的猜测正好对上——**两个项目都把 WebSocket 限定在终端面板这个辅助功能上,核心协议走的是另一条路**,只是 T3 Code 核心协议本身也是 WebSocket(JSON-RPC over WebSocket),OpenCode 核心协议是 HTTP+SSE,两者不同构。

TUI(`packages/tui`)、CLI、Web app(`packages/app`)、Desktop(Electron 壳套 `app`)全部通过同一个生成的 `OpencodeClient`(`packages/sdk`)访问这个 HTTP+SSE 接口。IDE 集成分两条路:VSCode 走独立扩展(`sdks/vscode/src/extension.ts`);Zed(以及其他支持 **Agent Client Protocol** 的编辑器)走 `packages/opencode/src/acp/service.ts`——这层代码本身也只是 `OpencodeClient` 的又一个调用方(`ACP.newSession`/`loadSession`/`prompt` 内部全部转成 `input.sdk.session.*` 调用),即 **ACP 适配层和 TUI/Web 是对等的、同一个 HTTP API 的两种前端**,不是单独的另一套后端逻辑。

**对 Dozer 的意义**:这是本系列第一次实测确认"一个后端多前端"不必然意味着 WebSocket——REST+SSE(单向推送够用,PTY 这种双向交互才需要 WebSocket)是另一条同样成立的路线,而且是 OpenCode 这种规模验证过的路线。`dozerd` 目前的 UDS 协议如果未来要对外暴露(比如给一个假设的编辑器插件),不必默认抄 WebSocket——"控制面 HTTP/SSE、终端面板才用双向通道"这个拆分本身就是可以直接借鉴的架构判断,和 T3 Code 的 access/launch 分离是两条不同但互补的经验。ACP 是 Zed 发起的跨 vendor 标准,T3 Code 也在用(`effect-acp` 包做 ACP client),OpenCode 这边是反过来做 **ACP agent 端**——如果 Dozer 未来考虑被第三方编辑器驱动而不是自己做 GUI,ACP 是已经有两个不同项目分别从两端验证过的现成协议,不需要自己发明。

#### 核心机制二:Tool-Calling 循环——大量借力 Vercel AI SDK,不是从零重做

`packages/opencode/src/session/llm.ts` 的默认路径是把请求交给 Vercel 的 `ai` 包(`streamText`/`wrapLanguageModel`),自己只做:包一层 Effect 服务、注入 `experimental_repairToolCall`(工具名大小写纠正+参数解析失败时降级成 `invalid` 工具、把错误原样喂回模型)、`activeTools` 过滤、`experimental_telemetry`。这是**默认路径**;还有一条 `flags.experimentalNativeLlm` 开关控制的"native"运行时(`@opencode-ai/llm`),特性标记关着,一旦某个 provider 返回"不支持"就自动回落到 AI SDK 路径——说明团队在往"自己掌控请求/响应解析"方向走,但目前还没把 AI SDK 这条腿撤掉。

工具执行侧(`packages/opencode/src/tool/tool.ts` + `registry.ts`)是自己写的:`Tool.define` 把每个工具包成 Effect,统一做参数 schema 解码(失败时用类型化的 `InvalidArgumentsError`,把"如何重新组织入参"的提示原样喂回模型而不是吞掉错误)、per-agent 输出截断(`Truncate.output`)、执行前的 `ctx.ask()` permission 挂钩。内建工具集(`shell`/`read`/`edit`/`write`/`glob`/`grep`/`task`/`webfetch`/`websearch`/`skill`/`apply_patch`/`plan`/`lsp`)加上插件工具(从 `tool/*.js` 文件或插件包动态 import)、MCP 工具统一进这个 registry,按 provider/model 特性做条件过滤(比如 GPT 系模型用 `apply_patch` 代替 `edit`/`write`)。

**对 Dozer 的意义**:这是和 jcode 的一处直接、具体的路线分歧,值得记下来对照——jcode 选择"完全自己实现 20+ 家供应商的 HTTP/SSE 解析",OpenCode 选择"默认交给 Vercel AI SDK 统一处理协议细节,自己只管工具分发和治理层"。哪条路线更好取决于目标:jcode 要的是极致的性能/内存控制(README 里 27.8MB vs OpenCode 的 318.4MB 追加会话内存),OpenCode 要的是"尽快接入尽可能多的 provider,把工程精力放在工具/权限/会话这些用户能感知的层面"。Dozer 是治理层,不做模型循环,这条分歧本身不直接可抄,但揭示了一个可迁移的判断:**"provider 覆盖面"和"运行时资源效率"是有工程取舍的两端,不是免费的**——如果 Dozer 未来要做"多 agent CLI 统一驾驭",也会面对类似取舍(自己写每个 CLI 的适配器 vs 复用一层通用抽象)。

#### 核心机制三:Provider 抽象——统一接口是"胶水层",真正的资产是处理供应商私有怪癖的那部分代码

`packages/opencode/src/provider/provider.ts` 里 `BUNDLED_PROVIDERS` 是一张 npm 包名到工厂函数的表(`@ai-sdk/anthropic`/`@ai-sdk/openai`/`@ai-sdk/amazon-bedrock`/`@ai-sdk/google-vertex`/`@openrouter/ai-sdk-provider`/`gitlab-ai-provider` 等约 20 个),这些包本身就是符合 Vercel AI SDK `LanguageModelV3` 接口的第三方实现;不在这张表里的 provider(来自 models.dev 目录但没预打包)靠 `Npm.add()` **运行时动态安装**对应的 npm 包再 `import()`。模型元数据(价格、上下文窗口、是否 deprecated)来自外部目录 `@opencode-ai/core/models-dev`(即 models.dev,一个独立维护的开源模型数据库),不是 OpenCode 自己录入。

真正需要手写的是 `custom()` 里针对具体 provider 的怪癖处理:Azure 要在 `resourceName`/`baseURL`/`useCompletionUrls` 之间做优先级判断并选择 `chat`/`responses`/`languageModel` 三种端点之一;Bedrock 要处理 profile/access key/bearer token/web identity token 四种鉴权方式的优先级、还要按 region 前缀(`us.`/`eu.`/`global.`)决定要不要给 modelID 加跨区推理前缀;GitHub Copilot 要按模型代数(`gpt-5` 以上且非 mini)决定走 `responses` 还是 `chat` 端点;Google Vertex + Anthropic 组合要手工拼 `aiplatform.{region}.rep.googleapis.com` 这种区域化 endpoint URL。**这些判断加起来才是这层抽象真正值钱的部分**——统一接口本身是 Vercel AI SDK 免费提供的,OpencCode 自己的工程投入集中在"每个供应商在鉴权、endpoint 选择、跨区路由上到底有什么不一样"这件事上。

**对 Dozer 的意义**:这是"agent 本身"范畴里第一次看到"provider 抽象"被拆解到这个细度,回答了任务里问的"这层抽象是不是最值钱的资产"——答案是:**接口统一不值钱(社区已经有),处理每个供应商私有怪癖的知识才值钱**,而且这类知识高度易腐(供应商随时改鉴权方式、改端点)。这对 Dozer 没有直接的代码借鉴点(Dozer 不做模型接入),但对"以后要不要接入除 Claude Code 外的第二个 agent CLI(比如 Codex)"这个问题有参照意义:真正的成本不在"写一个通用 adapter 接口",而在于每个 CLI 私有的启动参数/鉴权/hook 事件格式差异,这块工作量不会因为抽象设计得好而消失。

#### 核心机制四:权限/审批模型——运行时可阻塞的细粒度规则引擎,是第四条路线

`packages/opencode/src/permission/index.ts` 的核心是一个 `evaluate(permission, pattern, ...rulesets)` 函数:每条规则是 `{permission, pattern, action}` 三元组(`permission` 是工具类别如 `edit`/`bash`/`task`,`pattern` 是具体对象如文件路径/命令前缀/子 agent 类型),用 `Wildcard.match` 做双重匹配,`action` 取 `allow`/`ask`/`deny`,规则来自全局配置 + agent 定义 + session 级 + 子 agent 派生,按声明顺序 `findLast` 取最后一条命中的规则(后声明覆盖先声明)。命中 `ask` 时不是简单弹窗——用 Effect 的 `Deferred` 把整个工具执行**真正挂起**,直到 UI 侧调用 `reply()`;`reply` 支持 `once`/`always`/`reject`,`reject` 可以带一段 `message` 文本,包成 `CorrectedError` 原样喂回模型(模型据此重新组织下一步,而不是被静默拒绝)。`always` 生效时还会反向检查其余 pending 请求,同 pattern 的一并放行,减少连续弹窗。

对 `bash` 工具单独多一层:`tool/shell.ts` 用 `web-tree-sitter`(bash 语法树)解析命令,提取涉及的目录/文件路径喂给 permission pattern;`permission/arity.ts` 有一张人工标注的"命令前缀粒度表"(比如 `git` 记 2 个 token、`npm run` 记 3 个 token),决定用户点"总是允许"时到底该记住 `npm run dev` 这么细还是 `npm run` 这么粗——这是一个没有在 jcode/seek_code/t3code 里见过的具体细节,解决的是"允许规则该多精确"这个纯 UX 问题。**但要清楚地指出这套系统缺什么**:通篇没有类似 jcode `RiskLevel::Catastrophic` 那种"无论用户规则怎么写、无论传参怎么构造,都硬拒绝"的绝对底线;规则完全由用户/配置决定,并且整个 permission 系统可以被 `--yolo`/`--dangerously-skip-permissions` 命令行参数整体跳过(`cli/cmd/run.ts`/`tui.ts` 里直接写着 `describe: "...(dangerous!)"`)。

**对 Dozer 的意义**:回答任务里问的"这是第几条路线"——**这是一条新的第四条路线,和已知三条都不同**:不是 jcode 的"自动语义分级+强制拒绝底线",不是 seek_code 的"命令内容正则匹配",也不是 T3 Code 的"直接暴露被驾驭 CLI 自带的沙盒开关"。OpenCode 这条路线的本质是"**自己就是工具执行者,所以能做到真正的运行时阻塞等待人工确认**,规则本身是用户配置的模式匹配而非自动风险判断"。这和 jcode 的 `jcode-tui-permissions`(阻塞式审阅队列)其实解决的是同一个问题(执行前拦截+等真人),但 OpenCode 把"规则该怎么写"这件事做得更细(双维度 wildcard、AST 提取模式、reject 时可附反馈文本);同时它印证了 jcode 分析里记的那句话——"Dozer 只能通过 `PreToolUse` hook 退出码这条更弱的路径拦截,因为 Dozer 不是执行者本身"——OpenCode 用一个 190K star 的主流项目又确认了一次:**细粒度的执行前拦截,前提是拦截者本身拥有执行权**,Dozer 没有这个前提,这不是能靠"抄一份好的规则引擎设计"绕开的结构性限制。

#### 核心机制五:会话持久化与 resume——SQL 数据库承载,重启即接回

`packages/opencode/src/session/session.ts` 用 `drizzle-orm` 定义 `SessionTable`/`PartTable`/`ProjectTable`,落在真正的 SQL 数据库(`@effect/sql-sqlite-bun`,Bun 内建 SQLite),不是 jcode/seek_code 那类 JSON 文件存储。`Info` schema 里记了 `parentID`(fork 树)、`revert`(指向某条消息/snapshot/diff 的撤销指针)、`permission`(整条 session 级规则集)、`cost`/`tokens`(累计用量)、`time.archived`。

Resume 的实际路径在 ACP 层看得最清楚(`packages/opencode/src/acp/service.ts`):`loadSession`/`resumeSession`/`forkSession` 全部先 `sdk.session.messages(...)` 把该 session 的历史消息从服务端拉回来,再用 `restoreFromMessages()` 从最后一条带 model 信息的用户/助手消息里反推出当时用的 model/variant/mode,重新构造一个 `state` 接着跑——**resume 不依赖任何内存态,完全靠服务端持久化的消息记录重建**,进程重启、client 换一个都能接上。`resumeSession` 额外只拉最近 20 条做快速恢复,`listSessions` 支持跨目录(`roots: true`)列出全部 session 并按更新时间游标分页。

**对 Dozer 的意义**:这是三个"agent 本身"项目里第一次看到**用真正的关系型数据库**做会话存储(jcode/seek_code 具体存储机制本系列未深入到这个细度),SQL 表 + 显式 schema 演进(`fromRow`/`toRow` 转换函数)比裸 JSON 文件更经得起字段增删的历史包袱。`dozerd` 的 `AcceptanceStore` 目前存储机制如果还是文件式的,这里给了一个"要不要上真正的嵌入式 SQL(比如 `rusqlite`)"的具体参照点——尤其 Dozer 已经有"验收记录需要长期可查、可跨会话检索"这个需求,和 OpenCode 的 session 持久化诉求是同类问题。

#### 核心机制六:子任务/Sub-agent(`task` 工具)——fan-out/fan-in,不是 jcode 式 swarm

`packages/opencode/src/tool/task.ts` 里 `task` 工具启动子 agent 的方式是:创建一个新 session(`parentID` 指向当前 session),子 session 的权限从父 session 权限 + 子 agent 自身权限定义派生(`deriveSubagentSessionPermission`),并强制拒绝子 agent 使用 `todowrite`/`task`(除非其权限定义里显式声明允许)——**防止子 agent 无限递归开子子 agent**,配合 `cfg.subagent_depth`(默认 1)做深度上限。子 agent 跑完后结果通过 `renderOutput()` 包成 `<task>` XML 块塞回父 session 的下一条消息,期间没有任何"父子 agent 互相看对方文件改动、互相发消息"的机制。有一个实验性的"后台模式"(`background: true`,需要 `OPENCODE_EXPERIMENTAL_BACKGROUND_SUBAGENTS` 环境变量开关):子 agent 异步跑,完成后通过一条 `synthetic: true` 的文本消息注入回父 session,提示词里明确写"不要 sleep/轮询/重复问进度"。

**对 Dozer 的意义**:这个拓扑结构和 seek_code 的 `subagents.ts`(fan-out/fan-in,无跨 agent 通信)是同一类,和 jcode 的 swarm(agent 间 DM/广播、"代码在脚下位移"实时感知)是不同类——**这是本系列第二次验证"fan-out/fan-in 无跨 agent 通信"是主流做法,jcode 的 swarm 通信层目前是三个"agent 本身"项目里的少数派**,不是行业共识。二期设计"多 agent 协作"时,这条计数应该更新:2(OpenCode/seek_code)对 1(jcode)支持"隔离子任务、不做实时跨 agent 通信"这个更保守的模型,jcode 自己的 README 也承认这是它的反主流立场。深度限制(`subagent_depth`)和递归工具屏蔽(拒绝子 agent 再调 `task`/`todowrite`)这两个具体防护点,是 Dozer 二期如果做子任务编排时可以直接参照的最小防护集。

#### 其他验证点(简述)

- **上下文压缩**(`session/compaction.ts`):按"回合"(turn,一问一答为一组)从最近往前累加 token 预算,超出部分交给一个专门的 `compaction` agent/model 生成摘要,同时有独立的"裁剪"(prune)机制在摘要之外把更早的、已完成的工具输出内容清空(按 token 预算保护最近 N token 不裁)。和 jcode 的"80%/95% 阈值 + 双档压缩"比,OpenCode 的选择更"按对话轮次语义分段"而非纯阈值触发,细节不同但解决的是同一类问题——**再次确认压缩/裁剪是"agent 本身"范畴的标配子系统,不是 jcode 独有**。
- **Checkpoint**(`snapshot/index.ts`):隐藏 git ref 快照 + `diff`/`revert`,止步于"能不能撤销重来",没有 verdict/goal/acceptor 这类用户可见的验收元数据——和 T3 Code 分析的结论完全一致,**这次是在"agent 本身"范畴里又确认了一遍**:六/七个分析对象里仍然没有一个做到 Dozer 现有的验收闭环深度。
- **Worktree**:`control-plane/adapters/worktree.ts` 是一个可插拔的 `WorkspaceAdapter` 实现,创建 git worktree 承载会话——"一 session 一 worktree"这个此前只在编排层(kooky/orca/workmux)项目里验证过的标准件,现在在"agent 本身"范畴里也验证了一次,是更普遍的共识,不是编排层特有的模式。

#### 对 Dozer 的启示汇总

| OpenCode 机制 | 对应 Dozer 位置/路线图 | 借鉴点 |
|---|---|---|
| HTTP(OpenAPI)+SSE 为主协议,WebSocket 只管 PTY 面板;ACP 适配层与 TUI/Web 对等,同一套 HTTP API | `dozer-app`↔`dozerd` 的 UDS 协议;未来若要给编辑器插件暴露接口 | "控制面单向推送用 SSE、双向交互才用 WebSocket"的拆分是可直接参考的架构判断;ACP 是已有两个项目从两端验证过的现成跨 vendor 协议 |
| 默认路径交给 Vercel AI SDK 处理协议细节,自己只管工具分发/治理;有一条尚未转正的"自建 native 运行时"暗线 | Dozer 的"agent 中立"定位(不做模型循环) | 非直接借鉴,是"provider 覆盖面 vs 运行时可控性"这条工程取舍的又一实例,印证 jcode 分析里"包一层有天花板"的观察不是唯一解读——OpenCode 选的是相反的权衡且活得很好 |
| Provider 抽象的真实资产是处理供应商鉴权/endpoint/跨区路由怪癖的代码,不是统一接口本身 | 未来是否接入第二个 agent CLI(如 Codex)的成本预估 | 统一接口设计得好不好不决定成本,每个 CLI 私有差异的知识量才决定成本,这块投入无法靠抽象设计规避 |
| 细粒度 wildcard 规则引擎(permission×pattern→allow/ask/deny)+ Effect Deferred 真阻塞 + AST 提取模式 + reject 可带反馈文本;但无绝对拒绝底线,可被 `--yolo` 整体跳过 | spec 里"验收/治理"定位,命令风险门禁待办(与 jcode/seek_code/t3code 并列的第四条路线) | 确认这是全新第四条路线;也再次印证"执行前拦截前提是自己是执行者",Dozer 目前只能走 `PreToolUse` hook 退出码这条更弱的路径,是结构性限制不是设计取舍 |
| Session 落 SQL 数据库(`SessionTable`/`PartTable`),resume 完全靠服务端持久化消息重建(不依赖内存态) | `dozerd` 的 `AcceptanceStore` 存储机制 | 若目前是文件式存储,这是"上真正的嵌入式 SQL"的具体参照点,尤其验收记录需要跨会话长期可查这个诉求和 session 持久化同类 |
| `task` 工具 fan-out/fan-in 子 session,深度限制 + 递归工具屏蔽,无跨 agent 通信 | 路线图二期"编排与资产" | 再次确认 fan-out/fan-in(而非 swarm)是主流(2:1),深度限制和递归屏蔽这两个具体防护点可直接参照 |
| 压缩按"回合"语义分段 + 独立裁剪阈值;checkpoint 止步于"快照+diff",无验收元数据;worktree 是可插拔 adapter | 三期"记忆"路线图;Dozer 现有验收闭环定位 | 压缩机制细节可与 jcode 的阈值方案并列参考;验收闭环的差异化定位在"agent 本身"范畴里被二次确认为真空;worktree 标准件的普遍性又提高一档 |

---

### 12.8 orca

> 已合并自原独立报告 `orca-分析.md`。

### Orca 代码级分析

> V2 复核：2026-09-24，源码快照 `b2940a18ff0abb8c7527813f43954ad2e9c933b3`。

#### Dozer V2 复核结论

- **仍然是整体架构的高价值样本**：当前源码继续将 PTY、worktree、filesystem watch、agent hook、远程 authority 和 ephemeral VM 暴露为独立 bridge/API，证明“传输无关 core + 多宿主客户端”可持续扩展。
- **修正旧结论**：Orca 已不仅是 relay/PTY 调度台；`ephemeral-vm-api`、computer-use permission、AI vault resume、authoritative PTY snapshot capability 表明它正在形成环境、权限、凭据和状态恢复控制面。
- **对 V2 最有价值**：Dozer 应把 `ExecutionEnvironment`、Secret Broker、PTY snapshot capability 和 remote authority 都设计成能力协商，而不是假设每个 Provider 等价。
- **不可照搬**：Electron preload bridge 数量巨大，正是 Host API 面过宽的警示。Dozer Plugin Protocol 应少而稳定，领域功能通过插件或 MCP 暴露。
- **采用建议**：A 级架构参考；持续追踪 daemon、remote host 和 VM provider，不复用前端实现。

> 分析对象:https://github.com/stablyai/orca (本地副本 `/Users/chrischiang/AI/orca`)
> 分析日期:2026-07-14 · TypeScript ~39.4 万行 / 6900 文件(近半为测试) / MIT

#### 一句话定位

Orca 是 **Electron + TypeScript 的跨平台 AI agent 编排器**:每个 agent 跑在自己的 git worktree 里,统一在一处跟踪。支持 macOS/Windows/Linux + iOS/Android 手机伴侣端。规模是 kooky 的 ~19 倍——公司级产品,不是个人项目。

与 kooky 的本质区别:**kooky 是"更懂 agent 的终端"(深度在渲染和 macOS 原生体验),orca 是"agent 的调度台"(深度在编排、远程和分发)——终端在 orca 里只是一个组件,xterm.js 够用就行。**

#### 分层架构

```
src/
├── main/       Electron 主进程,~80 个按领域切分的模块目录
│               (claude/ codex/ cursor/ …30 个 agent 各一目录,
│                pty/ daemon/ ssh/ git/ github/ gitlab/ linear/ jira/
│                browser/ computer/ sqlite/ speech/ emulator/ …)
├── renderer/   React UI(shadcn + design tokens)
├── shared/     ~500 文件的传输无关纯逻辑层 —— 全项目的精华
├── relay/      部署到远程主机/WSL 的独立服务端(fs/git/pty/hook 处理器)
├── cli/        orca CLI(通过 RPC 操控运行中的 app)
├── mobile/     React Native 手机端(独立 pnpm workspace)
└── native/     每 OS 一个的 computer-use 原生模块、macOS 通知等
```

运行时依赖出奇地少(22 个):`node-pty`、`@xterm/headless`、`ssh2`、`ws`、`tweetnacl`(手机端 E2EE)、`zod`、`sherpa-onnx`(本地语音)。重的东西都是自己写的。

注意:`src/main/ghostty/` 不是嵌入 libghostty,只是**解析 ghostty 主题/配置文件格式**用于导入——与 kooky 走了完全相反的终端路线。

#### 四个最有含金量的子系统

##### 1. PTY 守护进程(`src/main/daemon/`,~90 文件)——技术含量最高

PTY 所有权从 Electron 主进程剥离到**独立后台 daemon**:

- unix socket + token 鉴权,NDJSON/二进制帧协议
- daemon 内跑 `@xterm/headless` 无头终端模拟器维护每个会话的屏幕状态,`addon-serialize` 做快照
- 效果:**app 重启/升级/崩溃时 agent 继续跑,重开后 reattach 恢复现场**——本质上是用 Node 重新实现了 tmux
- 工程深度的痕迹:`session-ingest-throughput.bench.test.ts`(吞吐基准)、`headless-emulator-fidelity.fuzz.test.ts`(模糊测试保真度)、`hibernation-cold-restore-repro.test.ts`、`priority-semaphore.ts`(背压)——这是和 Node 运行时性能搏斗的证据

##### 2. 执行宿主抽象 + relay(自研远程开发协议)

- `ExecutionHostId = 'local' | 'ssh:<target>' | 'runtime:<env>'`(后者为临时 VM),外加 WSL
- `src/relay/` 是部署到远端的自包含服务端:fs(带 ripgrep 安装/回退)、git(完整 worktree/staging/diff)、pty、agent-hook 四类 handler——相当于自研小号 VS Code Server
- AGENTS.md 铁律:"所有改动必须考虑 SSH 场景"
- `GitCapabilityCache` 按宿主隔离做能力探测 + 降级(Git 2.25 基线),因为 native/WSL/SSH 三处 git 版本可能都不同

##### 3. Agent 状态感知——kooky 的加强混战版

- `shared/agent-hook-listener.ts` 单文件 3987 行,**传输无关**的 hook 监听管线(HTTP 端点,1MB 大小上限、slowloris 防护),同一份代码在本地主进程和远程 relay 各跑一份("relay 归一化,Orca 路由")
- hook 只覆盖 Claude 这类有 hook 系统的 agent;orca 支持 30 个 agent,大部分没有 hook,所以是**多信号融合**:OSC 标题提取、ANSI 输出流扫描(`agent-tui-ansi-fuzz-stream`)、进程表扫描识别(`agent-process-recognition`)、agent 会话文件解析(`grok-session-paths`、Claude subagent roster)
- 外加每家的用量/限额跟踪(`claude-usage`、`codex-usage`、`rate-limits`)和多账号热切换(`claude-accounts`、`codex-accounts`)

##### 4. App 本身可被 agent 编程

`orca` CLI 通过 RPC 操控运行中的 app:`orca worktree create`、`snapshot`、`click`、`fill`(内嵌浏览器自动化)、computer use。**agent 可以驱动 orca 自己**完成"开 worktree → 跑另一个 agent → 截图验证"的闭环——产品设计上最超前的一点。

#### 工程文化观察

- **shared/ 纯函数化**:几乎每个 `.ts` 配同名 `.test.ts`,状态显式建模为可注入结构(如 `HookListenerState`),39 万行里近半测试还能 daily ship
- **AGENTS.md 是写给 AI 的规范**(项目明显大量由 agent 开发):禁止 `utils/helpers` 命名、禁止解除 max-lines、注释只写"为什么";repro 测试带 issue 号(`repro-7329-remote-snapshot-corruption.test.ts`)
- 工具链全是 Rust 的:oxlint、oxfmt

#### kooky vs orca 对照

| | kooky | orca |
|---|---|---|
| 形态 | 原生 macOS 终端 app | Electron 跨平台编排器 |
| 终端 | libghostty(GPU) | node-pty + xterm.js + 自研 PTY daemon |
| 会话存活 | 不存活,重启靠 `--resume` | **daemon 持有 PTY,app 重启无感** |
| agent 状态 | socket hook + OSC 双通道,4 个 agent 精做 | 多信号融合,30 个 agent 广撒网 |
| 远程 | OSC 带内标记(轻) | relay 协议服务端(重,自研远程开发协议) |
| 组织单位 | 目录/tab | **git worktree**(一 agent 一 worktree) |

两个项目独立收敛到了同一套地基:OSC 7/OSC 133 shell 集成、hook 端点 + surface/pane 路由、resume id 持久化、worktree 管理、用量监控。**这套就是此品类的标准件,用什么语言实现都绕不开。**

#### 对 Rust 实现的启示

1. **最值得用 Rust 做的,就是 orca 用 Node 做得最吃力的那块:PTY daemon。** 无头终端模拟(`alacritty_terminal` 是现成的无头 VT 状态机)+ 会话快照 + 多客户端 attach——orca 为吞吐和保真写基准/模糊测试对抗 Node 开销,而这正是 Rust 的主场(Zellij 已验证)。一个 Rust session daemon 可同时服务 GUI、TUI、CLI、手机 relay 多种前端
2. **relay 是第二个 Rust 甜点**:orca 必须往远程主机分发 Node bundle(还要处理 `daemon-bundle-staleness` 这类问题);musl 静态链接的单二进制 relay 在分发上是碾压性优势
3. **架构上抄"传输无关核心 + 薄宿主"**:`shared/` 对应 Rust workspace 的核心 crate(协议 + 纯逻辑),daemon/cli/gui/relay 都是薄壳——与 byteboy 设计文档的小 crate 原则同构
4. **不要抄它的广度**:39 万行大量是 30 个 agent 适配、5 个 git 提供商、Linear/Jira、手机端、内嵌浏览器——靠团队每天堆出来的面积。单人/小团队应选 kooky 的深度路线,但把 **"会话存活"这一个 orca 独有的架构点(daemon)纳入 v0 设计**,因为它必须在最底层定下来,后补等于重写

#### 对 byteboy 的演进路径

**v0 CLI(agent/model/doctor/config,已规划)→ v1 加 Rust session daemon(PTY 持有 + hook socket,即 orca 的 daemon + kooky 的 HookServer 合体)→ 前端(TUI 或 GUI)只是 daemon 的客户端。** 每一层独立可测,符合设计文档 trait-first、为 v1 留接口的原则。

---

### 12.9 seek_code

> 已合并自原独立报告 `seek_code-分析.md`。

### SeekCode 代码级分析

> V2 复核：2026-09-24。原仓库 `kafkazhang/seek_code` 当前返回 Repository not found，本机旧分析副本也已不存在，因此本报告无法针对最新源码复核。

#### Dozer V2 复核结论

- 本文以下源码路径和实现结论仅对应 2026-07-28 的历史快照，不再代表可获取的当前项目。
- 仍有独立价值的只有三个设计点：危险动作可否决 `auto` 模式、出网目的地统一收口、上下文压缩时固定保留 working memory。
- 这些机制已有 jcode、Goose、OpenCode 等可持续核验的项目提供更强证据，因此 SeekCode 不再列为 V2 重点跟踪对象。
- **处理建议**：保留本报告作为历史样本，但从“可复用项目/候选依赖”清单删除，不引用其代码，也不使用同名的其他 GitHub 项目替代。

> 分析对象:https://github.com/kafkazhang/seek_code (本地副本 `/Users/chrischiang/AI/seek_code`)
> 分析日期:2026-07-28 · 版本 0.5.5 · TypeScript 1.4 万行 / Electron+React / MIT · 26 star

#### 一句话定位

SeekCode 和上次分析的 jcode 是**同一范畴**:不是编排层,是agent 本身——Electron 桌面壳里跑一个完整的工具调用循环,深度适配 DeepSeek-V4,自称"融合 Claude Code(结对编程)与 Codex(任务委派)思路"。区别在规模量级:jcode 70.7 万行、80 crate、12K star,是团队级产品;SeekCode 1.4 万行、单体 Electron 应用、26 star,是**个人开发者规模**的同类实现。这个量级差恰好让它成为一份有用的对照——同一类问题(命令安全、出网控制、agent 记忆、子任务编排),在"没有团队资源死磕"的约束下,答案长什么样。

对 Dozer 的意义和 jcode 一样是"拆开看子系统",不是整体架构模板——Dozer 驾驭已有 agent CLI,SeekCode 和 jcode 一样是自己重做 agent 循环,产品路线相反。

#### 核心机制一:命令安全——正则黑名单,而非语义分级

`src/main/cmdsafety.ts`(89 行,零 Electron/Node 依赖,纯函数便于单测)做两件事:

- `isAppKiller`:识别"会误伤应用自身"的命令(按进程名批量杀 node/electron/seekcode),硬拒绝
- `isDangerousCommand`:一份相当详尽的破坏性命令正则表(`rm -rf`、`dd`、`mkfs`、`git reset --hard`、`git push --force`、fork bomb 模式、Windows 的 `Remove-Item -Recurse`/`diskpart`、docker 批量清理…)

这和 jcode 的 `RiskLevel` 四档分级(按"破坏半径"语义判断)是**同一问题的两种答案**:jcode 的方案更本质(能识别 `find -delete` 这类不含 `rm` 字样但同样致命的命令),SeekCode 的方案是穷举正则,注释里能看到"新增:xxx"的迭代痕迹——**每次漏判之后补一条正则**,这是资源有限时最诚实的演进路径,也是这类静态分类器天然的维护模式(黑名单会不断长,永远追不完)。

真正值得记的是**审批判断和权限模式的关系**(`src/main/agent/tool_exec.ts:81-88`):

```ts
const dangerousCmd = (t.name === 'run_command' || t.name === 'run_background')
  && isDangerousCommand(args.command ?? '')
...
if (needsApproval(perm, t.name) || dangerousCmd || mcpNeedsApproval || fetchNeedsApproval) {
  // 弹审批,即使 perm === 'auto'
}
```

四档权限模式(`ask`/`acceptEdits`/`plan`/`auto`,README 明确写"参考 Claude Code")本身只决定"默认要不要问";`dangerousCmd` 命中时**无视当前权限模式强制弹审批**,连"全自动"档都拦——这条"静态危险分类器可以否决运行模式"的设计和 jcode 的 `Catastrophic` 档(不接受任何模型说辞)是同一个原则,只是分类器精细度不同。**两个独立项目在这一点上给出了相同答案**,值得当作这类"危险动作强制打断"设计的共识,而不是某个项目的个人偏好。

#### 核心机制二:出网白名单闸门——单一收口点,而非沙盒

`src/main/egress.ts`(69 行)是主进程唯一允许出网的入口 `guardedFetch`,三级白名单:

1. **推理出口**:DeepSeek baseURL(用户代码/上下文唯一去处)
2. **生态出口**:固定的公共只读源(GitHub、MCP registry、context7)——用于装 Skills/MCP,不带用户代码
3. **用户显式信任**:用户主动配置的远程 MCP server host,连接时动态登记

非白名单 host 直接抛错拒绝;所有请求走 Electron `net.fetch`,复用渲染层同一套 session/webRequest 栈,不是另开一条不受审计的出网路径。

这是 workmux sandbox 里 CONNECT 代理+域名白名单那套机制的**极简版**——没有 iptables、没有独立代理进程、没有 host↔guest RPC 桥,就是一个函数级的"发请求前检查 host 在不在表里"。工程量差两个数量级,但解决的是同一个问题:**agent 的网络访问面必须显式收敛,不能是"默认能访问任何东西,靠自觉不滥用"**。对不需要容器/VM 级隔离的场景(比如 agent 本身就在受控的宿主进程里跑,不是跑在完全不受信任的沙盒里),这个"单一收口函数+白名单"模式比 sandbox 子系统轻量得多,是更现实的起点。

#### 核心机制三:双层记忆——比 jcode 的记忆图更朴素的可行方案

`working_memory.ts`(会话内)+ `memory.ts`(跨会话)分工明确写在注释里:

- **工作记忆**(`working_memory.ts`):内存 KV,按 sessionId 分区,LLM 执行中发现的关键实体(端口号/文件路径/错误信息/配置值)可被工具写入;**上下文压缩时作为独立 system 消息保留,不参与摘要裁剪**——解决"长任务中关键发现随压缩丢失"这个具体问题。有界:单会话最多 30 条,key ≤60 字符、value ≤400 字符,满了淘汰最早条目;会话结束即清空,不跨会话
- **长期记忆**(`memory.ts`,file-backed):`SEEK.md`/`memory.md` 落盘,跨会话保留

对比 jcode 的记忆图(语义嵌入 + 余弦检索 + 侧 agent 验证相关性 + ambient 定期整理):SeekCode 这套没有嵌入、没有检索,就是一个有界 KV + 一个 markdown 文件。**但"工作记忆在压缩时被钉住不裁剪"这个具体机制,jcode 的记忆图设计里没有对应物**——jcode 解决的是"记忆的新写入/检索",SeekCode 解决的是"当前任务的关键事实不能在压缩时被摸掉",是两个正交的问题,后者更小但更容易在 Dozer 现有的 P1 范围内落地(Dozer 目前没有上下文压缩机制,但如果做,这条"压缩时钉住关键工作记忆"的经验值得直接搬)。

#### 核心机制四:子代理编排——jcode swarm 的克制版

`subagents.ts`(162 行):主对话 agent 调 `spawn_subagents` 把独立子任务并发委派出去(上限 6 个任务、3 个并发),每个子代理是一次独立 agent 循环,继承父会话权限模式与项目目录,**审批经父会话通道上浮**(摘要带"子代理"标识),完成后回灌一份长度受限(1600 字符裁剪)的结果报告给编排者;**子代理不可再派生子代理**(硬编码防递归,不是软约束)。

和 jcode 的 swarm 对比:jcode 做的是"同仓库内多个平行 agent 之间互相感知、能收到别人改了自己读过的文件的通知、能互相发消息";SeekCode 的子代理**互相看不见**(prompt 里明确写"你看不到主对话历史,任务描述即全部上下文"),只是"并行执行 + 结果收敛回主 agent"的经典 fan-out/fan-in,没有 jcode 那套跨 agent 消息/冲突感知层。这是"如果二期不需要 agent 间实时协作,只需要把一个任务拆成几个独立子任务并行跑"这种更小需求的现成参考——比 jcode swarm 便宜得多,如果 Dozer 二期编排的第一步只是"并行派单"而不是"多 agent 协作同一份代码",这个模型是更合适的起点,不必一步到位抄 swarm。

#### 一个明确的取舍:放弃 PTY,换取零原生依赖

`src/main/terminal.ts` 开头写明"轻量终端:用系统默认 shell 执行单条命令并流式回传输出。**非 PTY(无需 node-pty 原生依赖)**,适合 git/npm/构建/测试等命令"。这意味着 SeekCode 内置终端跑不了 vim/top 这类需要真实 TTY 语义的全屏程序,cwd 靠渲染层显式跟踪、每条命令重新传入(不是 shell 进程自己维护的 cwd)。

这是和 kooky/orca/Dozer 的路线**相反**的选择——Dozer spec 明确要"原生 agent 终端…流畅度对标 kooky"、"PTY 由常驻 daemon 持有",这条不适用。但值得记录为一个清晰的成本参照:**放弃 PTY 换来的是"不需要 node-pty 这个常见故障源"(原生模块跨平台编译/权限问题是 Electron 生态出了名的坑)**——如果 Dozer 未来在某个受限场景(比如某个不需要真实终端交互、只需要跑固定命令流的子功能)遇到类似取舍,这是一个已经在生产验证过的"降级方案"存在的证据,不是无人区。

#### 对 Dozer 的启示汇总

| SeekCode 机制 | 对应 Dozer 位置/路线图 | 借鉴点 |
|---|---|---|
| `dangerousCmd` 无视权限模式强制审批 | spec"治理"定位,与 jcode command-risk 的 `Catastrophic` 档同一原则 | 两个独立项目收敛到同一条:静态危险分类器应该能否决"自动模式"。Dozer 若做命令风险门禁,这条应作为设计约束,不只是 jcode 一家之言 |
| `guardedFetch` 三级白名单单一收口 | 若 Dozer 需要限制 agent 网络访问面,又不想上 workmux 级 sandbox | 比容器/代理轻两个数量级的现实起点:一个函数 + 白名单表,不需要 iptables/RPC 桥 |
| `working_memory.ts` 压缩时钉住关键事实 | 路线图三期"自治与记忆";Dozer 目前无上下文压缩机制 | jcode 记忆图之外的正交小问题的直接解法,比嵌入检索便宜得多,值得在设计压缩机制时一并纳入 |
| `subagents.ts` fan-out/fan-in 无跨 agent 通信 | 路线图二期"编排与资产" | 比 jcode swarm 更小的第一步参考——先做"并行派单+结果收敛",不必一步到位做"多 agent 实时协作同一份代码" |
| `cmdsafety.ts` 正则黑名单 vs jcode 语义分级 | 同上,风险分级机制的实现选择 | 两种真实存在的实现路线,黑名单起步快但会不断追加正则;分级更本质但工程量更大——排期时按团队资源量级选,不是非此即彼 |
| 放弃 PTY 换零原生依赖 | Dozer 已定"原生 agent 终端对标 kooky",此路线不适用 | 仅作为"受限场景下的已验证降级方案"记录,非当前借鉴项 |

---

### 12.10 t3code

> 已合并自原独立报告 `t3code-分析.md`。

### T3 Code 代码级分析

> V2 复核：2026-09-24，源码快照 `e67abcf798f8c4d8458755e3b4dde02c2c1f628b`。

#### Dozer V2 复核结论

- **旧报告需要降级使用**：项目在两个月内快速演进，旧报告关于 Provider 数量和实现成熟度只代表 2026-07-28 快照，不能继续作为现状断言。
- **仍然成立**：共享 server 是唯一执行边界、结构化 Agent 协议优先、连接状态机和远程 access model，均适合 Dozer daemon/remote provider。
- **对 V2 最有价值**：控制协议、session 状态和 UI contract 应独立包发布；桌面、Web、移动端只是客户端。Dozer 当前只实现 iced，但协议不应携带 iced 类型。
- **不可照搬**：多端产品面积和 Effect/TypeScript 技术栈不是 V2 近期目标；checkpoint/review 也不能替代 Dozer 的 Acceptance 记录。
- **采用建议**：A-级协议参考；后续引用必须注明具体 commit，避免将 README 宣称能力当成已落地能力。

> 分析对象:https://github.com/pingdotgg/t3code (本地副本 `/Users/chrischiang/AI/t3code`,克隆耗时较长,期间以 `gh api` 逐文件拉取源码 + 全部架构文档并行核对)
> 分析日期:2026-07-28 · TypeScript 53.9 万行 / 1.3 万文件 / 2,224 测试文件(另有 Swift/Kotlin/C++ 原生模块)· MIT · 15,351 star,今日仍在推送

#### 一句话定位

T3 Code(Theo/pingdotgg 出品)是"a minimal web GUI for coding agents"——和 kooky/orca/workmux 同一范畴:**驾驭已有 agent CLI**(Codex、Claude、Cursor、OpenCode),不自己重做模型循环,和 jcode/seek_code 相反。形态是 **Node.js WebSocket server 包一层 React web app**,桌面端是 Electron 壳,还有 iOS/Android 原生客户端(`apps/mobile`,含 Swift/Kotlin 代码)——四种前端(web/desktop/mobile/CLI `npx t3@latest`)共享同一个 server 作为唯一执行边界。

**诚实的范围说明**:README 写"currently Codex, Claude, Cursor, and OpenCode",但架构文档明确写"Codex is the only implemented provider. `claudeCode` is reserved in contracts/UI"——项目自己也承认"very very early, expect bugs, not accepting contributions yet"。分析时按"架构已经这么设计,但目前只有一个 provider 真正跑通"来读,不要当成四个 provider 都成熟。

体量上,53.9 万行 TypeScript 排在 jcode(70.7 万行)之后、超过 orca(39.4 万行),是六个分析对象里第二大,**七个分析对象共同标准件里唯一原生覆盖 web+desktop+mobile 三端**的编排层项目(orca 有手机伴侣端但核心是 Electron;kooky/workmux 单一前端)。按 `apps`/`packages` 拆分:`apps/server`(17.3 万行,核心执行边界)> `apps/web`(13.7 万)> `apps/mobile`(7.0 万,原生 iOS/Android)> `packages/effect-codex-app-server`(4.7 万)> `apps/desktop`(3.4 万,Electron 壳,体量远小于 server/web,印证"桌面只是薄壳")。

#### 工程结构

```
apps/
  server    Node.js WebSocket server(核心执行边界,唯一真相源)
  desktop   Electron 壳(41.5.0),用 Clerk 做云账号登录
  web       托管版静态 web app
  mobile    iOS/Android 原生客户端
  marketing 官网
packages/
  contracts               WebSocket 协议类型定义(client/server 共享)
  effect-codex-app-server  包装 codex app-server 的 JSON-RPC 客户端
  effect-acp               Agent Client Protocol(跨 vendor 的 agent 通信标准)客户端
  client-runtime            web/mobile 共享的连接运行时
  shared                    通用工具(DrainableWorker 等)
  ssh / tailscale            远程访问的两个"端点提供者"
```

全项目用 **Effect**(TypeScript 函数式副作用系统:`Context.Service` 做依赖注入、`Layer` 组合服务、类型化错误而非抛异常)构建,`apps/server/src` 下每个子系统(`provider`/`orchestration`/`checkpointing`/`review`/`vcs`/`terminal`/`git`)都是这个模式——这是六个分析对象里第一次见到用严肃的效果系统写 agent 编排后端,不是裸 async/await + try/catch。

#### 核心机制一:Provider 接入——优先用结构化协议,PTY 是补充不是主力

`packages/effect-codex-app-server` 包装的是 **`codex app-server`**——Codex CLI 自带的 JSON-RPC over stdio 模式,不是解析终端字节流。`packages/effect-acp` 对接的是 **Agent Client Protocol**(Zed 发起的跨 vendor 标准,部分 agent CLI 已原生支持)。`apps/server/src/terminal` 目录仍然存在,大概率承担"给用户看的内嵌终端面板"这类辅助功能,而不是 provider 控制通道本身(未能本地核实到具体调用点,这里留有一定不确定性)。

**对 Dozer 的意义**:这是目前六个分析对象里唯一采用"优先用 agent CLI 自带结构化协议,PTY 只做补充"路线的项目。kooky/workmux/Dozer 全部走的是"PTY + hook + OSC 扫描"这条路,因为 Claude Code 没有类似 `codex app-server` 的官方 JSON-RPC 模式。这不是"Dozer 该改路线"的理由(Claude Code 目前确实没给这个接口),而是一条值得记住的判断准则:**遇到新 agent CLI 时,先查它有没有结构化协议(app-server 式 JSON-RPC 或 ACP),有就优先用,PTY 兜底**——T3 Code 的架构选择证明了这条准则已经被一个 15K star 项目验证过,不是理论空想。

#### 核心机制二:远程架构——目前六个分析对象里最成熟的设计

`docs/architecture/remote.md` 是一份"architecture-first"的设计文档,核心是把"连不连得上"拆成四层正交概念:

- **`ExecutionEnvironment`**:一个运行中的 T3 server 实例,拥有 provider 状态、项目/线程、终端进程、git/文件系统——**跨端(web/desktop/mobile)共享的唯一概念**
- **`KnownEnvironment`**:客户端本地保存的"知道怎么连到某个环境"的记录,不是 server 权威的
- **`AccessEndpoint`**:连到某个环境的一条具体路径(直连 `wss://`、隧道、desktop 托管的 SSH 转发)——**同一个环境可以有多条路径,这是防止"SSH 接管整个模型"的关键抽象**
- **`AdvertisedEndpoint`**:server/desktop 主动建议的候选端点,客户端当提示用,不当"能连通"的证据
- **Endpoint providers**:可插拔的端点发现器,第一个实现是 Tailscale(发现 Tailnet IP/MagicDNS,作为候选端点),未来的隧道服务走同一套接口

最关键的设计判断是**"access(怎么连)"和"launch(server 怎么起来)"严格分离**——同一个 `ExecutionEnvironment` 可能是手动起的、SSH 远程起的、或者本地起了再靠隧道发布出去,连接方式和启动方式是两条独立的轴,不要耦合。文档里明确对照 Zed:"借 Zed 的远程启动纪律(显式 bootstrap、reconnect 优先、连接体验归客户端),但不借它的传输协议"——Zed 因为远程边界在 editor/project runtime 之下,不得不发明自定义代理协议;T3 的运行时边界已经是"一个 server,标准 WebSocket",没有这个包袱,SSH 只是"起服务+建端口转发"的助手,连上之后还是走普通 WebSocket。

**对 Dozer 的意义**:kooky 只有 OSC 带内标记这种轻量远程信号,orca 需要自研 relay 协议(因为 orca 的执行边界更碎、更贴近 PTY),T3 Code 因为**执行边界已经收敛成"一个进程、一套 WebSocket 协议"**,远程支持几乎是自然延伸,不需要重新发明传输层。这直接印证了一条设计原则:**执行边界收得越干净,以后加远程支持的成本越低**。Dozer 的 `dozerd` 已经是"一个 daemon、一套 UDS 协议"这种干净边界,如果未来真要做远程(spec 目前没定这个方向),T3 的四层模型(Environment/Known/Access/Advertised)是比 orca relay 更轻量、更适合直接抄的起点——把 UDS 换成可选的 TCP/WSS 监听,语义上不需要大改。

#### 核心机制三:Server 自更新——版本漂移场景下的能力探测+安全交接

`docs/architecture/server-updates.md`:server 在 `ExecutionEnvironmentDescriptor` 里广播自己"能不能被替换、怎么替换"(`capabilities.serverSelfUpdate`),按进程形态分四档:

| 广播值 | 场景 | 客户端动作 |
|---|---|---|
| `boot-service` | Linux,T3 托管的 systemd 用户服务 | 调更新 RPC,服务单元被替换重启 |
| `respawn` | npm CLI 前台跑在 macOS/Linux | 调更新 RPC,进程交接给一个新起的替身,自己退出 |
| `desktop-managed` | 被 desktop app 监管 | 提示用户去更新 server 所在机器上的 desktop app |
| 缺省 | 旧版本/开发环境/Windows 前台进程/未识别的 supervisor | 给出精确的手动重启命令 |

安装流程有 **preflight 校验**:装好新版本先跑一次版本预检,失败就删掉失败的运行时、保留当前 server 不动,不会半途而废让 server 处于不可用状态。**desktop 托管优先级高于进程形态探测**——被 desktop app 管着的 server 绝不能被"respawn 路径"误伤,自己再起一个进程和 desktop 打架。

**对 Dozer 的意义**:和之前记的"`dozerd` 自身崩溃恢复"缺口不是同一个问题(这是主动升级,不是被动崩溃),但工程纪律相通——**"server 能不能自己换血"需要显式声明能力、而不是客户端瞎猜**,这条思路可以直接套用到 Dozer 未来做 `dozerd` 版本升级流程时的设计(`dozer-app` 检测到 `dozerd` 版本落后,如何安全触发替换而不丢会话)。目前 Dozer 连"`dozerd` 崩溃后怎么办"都还没做,这条属于更后面的问题,但值得记在同一条待办脉络下。

#### 核心机制四:Checkpoint/Review——比 Dozer 验收闭环更轻的一层

`apps/server/src/checkpointing/CheckpointStore.ts` 源码注释写得很清楚:"Owns hidden Git-ref checkpoint capture/restore and diff computation…**It does not store user-facing checkpoint metadata and does not coordinate provider conversation rollback**"——回合开始/结束时自动打隐藏 git ref 快照,供 diff 和恢复用,但**不存目标/标准/通过与否这类用户可见的验收元数据**。`ReviewService.ts` 只做一件事:`getDiffPreview`,而且专门校验 diff 的 cwd 必须落在配置的 workspace/worktrees 根目录内(防越权读取仓库外内容)。

**对 Dozer 的意义**:这是一个重要的差异化确认,不是借鉴点——T3 Code 的 checkpoint/review 是"撤销重来+看 diff"基础设施,**没有** Dozer `.dozer/goal.md`(目标/验收标准)+ `AcceptanceStore`(verdict/comment/acceptor 结构化记录)+ `refs/dozer/accepted/<n>` 这一整套"验收闭环"。六个分析对象里,**没有一个做到 Dozer 现在已经做的这个深度**——workmux 没有,kooky 没有,jcode/seek_code 的 checkpoint 概念也只到"回合边界快照",T3 Code 同样止步于"快照+diff"。这从侧面印证 Dozer 的"验收闭环"定位是真实的差异化空白,不是重新发明轮子。

#### 核心机制五:运行时模式——直接复用 agent CLI 自带的权限旋钮,不重造

`docs/architecture/runtime-modes.md` 全文只有两档:

- **Full access**(默认):`approvalPolicy: never` + `sandboxMode: danger-full-access`
- **Supervised**:`approvalPolicy: on-request` + `sandboxMode: workspace-write`,应用内弹审批

这两个参数名(`approvalPolicy`/`sandboxMode`)直接是 **Codex CLI 自己的启动参数**——T3 Code 没有像 jcode(`RiskLevel` 分级器)、seek_code(`cmdsafety.ts` 正则黑名单)、workmux(容器/VM 沙盒)那样自己造一套风险判断或隔离机制,而是**直接把 Codex 自带的沙盒模式暴露成一个 UI 开关**。

**对 Dozer 的意义**:这是"治理"这个主题下第三种真实存在的实现路线(前两种是 jcode/seek_code 的"自己判断危险"、workmux 的"自己建隔离边界"),而且是**工程量最小的一种**——前提是被驾驭的 agent CLI 自己就带了权限/沙盒参数。Claude Code 恰好也有类似的东西(`--permission-mode`/hooks 里的 allow/deny/ask 决策),Dozer 排"命令风险门禁"这类待办时,第一步应该先查 Claude Code(以及未来的 Codex 适配器)自己暴露了哪些权限旋钮,能直接透传的就不要重新发明——只有 agent CLI 自己没提供细粒度控制时,才需要 jcode/seek_code 那种外挂分类器。

#### 工程文化:连接状态机的纪律

`docs/architecture/connection-runtime.md` 里 `EnvironmentSupervisor` 是**唯一的重试所有者**——离线就释放会话且不消耗重试次数,在线才向 broker 要一次准备好的连接、向 session factory 要一次 RPC 会话;瞬时失败无限重试、指数退避封顶 16 秒;认证/配置错误保持阻塞直到外部唤醒;意外断连保留注册和缓存后重试;显式移除才真正清缓存。UI 层的"已连接"状态要求"socket 打开 + 首次 config RPC 成功"两个条件都满足才成立,**不能靠"有缓存数据"或"transport 对象存在"来推断健康度**——连接健康和数据同步健康是两个独立状态,分开展示。

**对 Dozer 的意义**:这套纪律(单一重试所有者、退避封顶、连接态与数据态解耦)是"客户端-daemon 连接管理"这个具体问题的一份高质量参考实现,`dozer-app` 与 `dozerd` 之间目前的连接管理如果还没有类似的显式状态机(单一重试点、退避策略、连接健康与数据同步健康分离展示),这是可以直接对照抄设计原则的一份现成范本。

#### 对 Dozer 的启示汇总

| T3 Code 机制 | 对应 Dozer 位置/路线图 | 借鉴点 |
|---|---|---|
| 优先用 agent CLI 自带结构化协议(app-server/ACP),PTY 兜底 | `dozer-hook`/`dozerd` 的 agent 适配面 | 判断准则:新增 agent 适配器前先查有没有结构化协议,没有才退回 PTY+hook 这条更重的路 |
| Environment/KnownEnvironment/AccessEndpoint/AdvertisedEndpoint 四层远程模型 + access/launch 分离 | spec 未定的"远程"方向 | 目前六个分析对象里最成熟的远程设计,`dozerd` 的干净 UDS 边界如果要加远程支持,这是比 orca relay 更轻的起点 |
| server 自更新的能力探测(boot-service/respawn/desktop-managed/absent)+ preflight + 安全交接 | "`dozerd` 自身崩溃恢复"待办的邻近问题(主动升级 vs 被动崩溃) | 主动升级场景的设计纪律可提前参考,即便当前待办优先级是崩溃恢复不是升级 |
| checkpoint/review 止步于"快照+diff",无验收元数据 | Dozer 已有的验收闭环(goal/criteria/verdict/accepted ref) | 非借鉴点,是差异化确认——六个分析对象里没有一个做到 Dozer 现在的验收深度,这个定位是真空,不是重复造轮子 |
| Runtime modes 直接透传 Codex 自带的 `approvalPolicy`/`sandboxMode` | 命令风险门禁待办(与 jcode/seek_code 并列的第三条路线) | 工程量最小的治理路线:先查被驾驭的 agent CLI 自己暴露了哪些权限旋钮,能透传就不重新发明分类器 |
| `EnvironmentSupervisor` 单一重试所有者 + 连接态/数据态解耦 | `dozer-app`↔`dozerd` 的连接管理 | 现成的连接状态机设计范本,可直接对照当前实现查缺口 |
| Effect(`Context.Service`+`Layer`+类型化错误)贯穿全 server | Dozer 的 Rust 实现,无直接语言对应 | 不是代码可抄,是"用严肃的效果系统/错误类型统一整个后端"的工程态度参照——Rust 的 `thiserror`/`anyhow` 已经在做类似的事,態度上是一致的 |

---

### 12.11 workmux

> 已合并自原独立报告 `workmux-分析.md`。

### Workmux 代码级分析

> V2 复核：2026-09-24，源码快照 `267bfc44ffc15e60acf1bd4c4b8cad5b56a7a2ad`。

#### Dozer V2 复核结论

- **仍然成立**：multiplexer 后端抽象、worktree-first、hook 状态 reconciliation 和 sandbox 是 Dozer Task Runtime 的高价值参考。
- **新增关注点**：当前 `git/security.rs` 显式压制 `core.hooksPath`、upload-pack hook 等 Git 配置，说明创建 Agent worktree 时仅隔离目录并不够，还要处理仓库级 Git 执行面。
- **对 V2 最有价值**：把 worktree 视为带生命周期和安全策略的资源，而非一个路径；创建、合并、rebase、清理、残留检测和关联 container 都应进入 Runtime API。
- **不可照搬**：Dozer 已决定自持 PTY，不能依赖用户安装 tmux/Zellij；Workmux 只作为 Git/worktree/sandbox 参照。
- **采用建议**：A 级局部参考；优先阅读 `src/git/worktree.rs`、`src/git/security.rs`、multiplexer 与状态存储代码。

> 分析对象:https://github.com/raine/workmux (本地副本 `/Users/chrischiang/AI/workmux`)
> 分析日期:2026-07-28 · 版本 v0.1.229 · Rust 7.6 万行 / MIT · 1947 star

#### 一句话定位

Workmux 是 **驾驭现有终端复用器(tmux/Zellij/WezTerm/kitty)的 CLI + TUI 编排层**,自己完全不做 PTY 持有和终端渲染——`git worktree` 建隔离目录,tmux 起窗口,agent CLI(Claude/Codex/Gemini/…)在窗口里跑,workmux 只负责"建窗口、装 hook、读状态、画 dashboard"。这是三个分析对象里(kooky/orca/workmux)**唯一不自研终端引擎**的一个:kooky 嵌 libghostty,orca 自研 PTY daemon,workmux 直接把 tmux 当黑盒来遥控。

后果是双向的:终端渲染、resize、粘贴、焦点这类"坑"完全外包给了 tmux,7.6 万行代码几乎全花在编排逻辑上(比 kooky 21K 行大 3.6 倍,但同样不碰 VT 解析);代价是**能力天花板锁死在 tmux 暴露的接口上**——`capture-pane` 只能拿到已渲染的文本快照,没有结构化事件流,所以 agent 状态感知必须完全依赖 hook + 标题这类带外信道,不能像 kooky 那样在字节流里做语义解析。

#### 与 kooky/orca 的根本区别:借来的终端 vs 自研的终端

| | kooky | orca | workmux |
|---|---|---|---|
| 终端引擎 | 嵌入 libghostty(GPU 渲染) | 自研 PTY daemon(node-pty + xterm/headless) | **不自研,遥控 tmux/Zellij/WezTerm/kitty** |
| 组织单位 | tab | git worktree | git worktree(同构) |
| 会话存活 | 不存活,`--resume` | daemon 持有 PTY,app 重启无感 | **tmux 本身就是 daemon**,workmux 进程退出无所谓 |
| 读取代理输出 | VT 解析出的语义事件 | 无头终端模拟(结构化) | `capture-pane` 拿渲染后纯文本(降级到"读屏") |

"会话存活"这个 kooky/orca 都要专门解决的架构难题,workmux 是**白拿**的——tmux server 本身天然常驻、天然扛得住前台进程崩溃。这是"借用宿主"路线最大的免费红利,也是它 7.6 万行里完全没有 PTY/daemon-crash-recovery 类代码的原因。

#### 工程结构(按子系统 LOC)

| 子系统 | 行数 | 职责 |
|---|---|---|
| `command/sidebar/` | 11559 | 常驻 daemon 轮询 tmux + 广播快照给"侧边栏"TUI 客户端(ratatui) |
| `command/dashboard/` | 10169 | 独立的全屏 TUI:agent 列表、diff 预览、worktree 管理 |
| `sandbox/` | 8919 | 容器(Docker/Podman/Apple Container)+ Lima VM 隔离,RPC 桥,网络代理 |
| `multiplexer/` | 7864 | tmux/WezTerm/Zellij/kitty 四后端的 trait 抽象 |
| `workflow/` | 6675 | create/merge/remove/rename/resurrect 等高层业务流程编排 |
| `config.rs` | 5570 | 单文件 YAML 配置 schema(agent 定义、pane 布局、hook、sandbox 规则) |
| `agent_setup/` | 3167 | 8 个 agent(Claude/Codex/Gemini/Copilot/Antigravity/OpenCode/pi/omp)的 hook 安装器 |
| `git/` | 2370 | worktree 生命周期、状态查询、merge/rebase |

`command/` 下还有 `add`/`merge`/`rebase`/`sync_files`/`reap_agents` 等三十多个子命令文件,是典型"每个 CLI 子命令一个文件"的 clap 项目结构,没有 kooky/orca 那种"厚 lib + 薄壳"的分层——**workmux 本身就是唯一的可执行文件**,没有拆出独立小 hook 二进制(hook 命令是 `workmux set-window-status`,复用同一个二进制,靠 fork 的进程启动开销换来了架构简单)。

#### 核心机制一:Multiplexer trait 抽象(`multiplexer/mod.rs`)

单个 ~30 方法的 `Multiplexer` trait,四个实现(tmux/WezTerm/Zellij/kitty)。设计上大量方法给了默认实现(返回 `Err`/no-op/空集合),新增后端只需实现窗口分屏、发送按键这几个原语——`setup_panes()` 的完整编排逻辑(命令解析、agent 占位符替换、handshake 同步、sandbox 包裹、resume 参数注入)写在 trait 默认方法里,四个后端共享,不需要各自重复。

后端探测顺序体现了一个值得记住的细节:**内层复用器优先于外层**——`$TMUX` 先于 `$WEZTERM_PANE` 先于 `$ZELLIJ_*` 先于 `$KITTY_WINDOW_ID`,因为 tmux 常被嵌套在其他终端里运行,env var 是运行时最新写入的那个赢,顺序错了会认错后端。9 个单测直接覆盖了全排列组合,而不是靠人肉走查——这种"纯函数化 + 穷举分支测试"的模式在全项目重复出现(`resolve_backend` 与实际 `detect_backend` 分离,前者可测,后者才碰真实环境变量)。

`create_handshake()` 抽象出了"pane 起了但 shell 还没就绪"这个所有后端共同的竞态:先 spawn 一个内含 handshake 脚本的 shell,脚本执行到位后通过 unix pipe 通知,workmux 收到通知才发送真正的启动命令——比 kooky 的方案更规整(kooky 靠 sleep/轮询应付类似问题未见文档提及,workmux 是显式同步原语)。

#### 核心机制二:Agent 状态感知(hook 安装 + 落盘 + 实时核对)

三段式,和 kooky 的"unix socket + hook 小工具"同构但工程细节更硬核:

##### 1. Hook 安装:JSON 树合并,不是覆盖写

`agent_setup/hooks.rs` 是这块的核心。8 家 agent CLI 的 hook 配置全是"JSON 里一个 `hooks` 键,按事件名分组,组内是 command 数组"这同一种形状(Claude/Codex/Gemini 三家共享这个格式,Copilot/Antigravity/OpenCode/pi/omp 各自适配)。安装逻辑不是"写文件",是**语义化合并**:

- `merge_hook_groups`:按 `serde_json::Value` 相等性去重后 push 进已有事件数组,不存在的事件直接插入整个数组——用户自己配的其他 hook(如 `afplay` 提示音)原样保留
- `remove_workmux_hooks`:按 command 字符串包含 `workmux set-window-status` 精确摘除,同一 group 里混有用户 hook 时只删 workmux 那一条,不删整个 group
- `remove_empty_hooks_wrapper`:摘完后空对象/空数组要连壳一起清掉,否则配置文件里留一堆 `{}`

**幂等性作为一等公民**:每个函数都有"再调一次应返回 false(无变化)"的测试。这比 kooky 文档里描述的"写一份 hooks 配置"更接近真实生产系统该有的样子——用户的 settings.json 是共享可写资源,agent 自己的其他插件/hook 随时可能并存,合并/摘除必须无损。

##### 2. 状态落盘:一 pane 一文件,而不是一个大数据库

`state/store.rs` + `state/types.rs`:`$XDG_STATE_HOME/workmux/agents/{backend}__{instance}__{pane_id}.json`,一个 agent 一个文件,`PaneKey`(backend + instance + pane_id)做复合主键防止多 tmux server/多后端撞车。文件名里的 `/`、`:`、`%` 会被 percent-encode(tmux socket 路径本身带 `/`)。写入统一走 `write_atomic`(临时文件 + rename),读取遇到损坏 JSON 直接删除重来而不是报错阻塞——这两条和 kooky"运行时字段一律 Optional、从磁盘读的值不可信"的裁决完全一致,是两个独立项目收敛到的同一条经验。

##### 3. 状态核对:拉取式 reconciliation,不是定时器扫描

kooky 用"60 秒没等到 Post 的调用标记为 stalled"的**后台定时器**做 orphan 检测;workmux 走的是**拉取式**路径——`load_reconciled_agents()` 在 dashboard/sidebar 每次刷新时,一次性批量查询 tmux 所有 pane 的实时信息(`get_all_live_pane_info`,单条 tmux 命令),逐个比对存盘状态:

- pane 完全查不到 → 判定关闭,删状态文件(除非检测到 tmux server 重启过,此时保留以支持 `resurrect` 复活)
- `pid` 对不上存盘的 `pane_pid` → pane ID 被复用(旧 pane 关了,tmux 把 ID 分给了新进程),判定失效
- `current_command` 变了(如 `node` 变成 `zsh`)→ agent 进程退出,判定失效

三层判定共用一个"是否跨过 server 生命周期"(`boot_id` 比对)的前置分支——server 重启导致的 PID/command 突变要保留(等用户 resurrect),真实退出导致的突变要清理,这是全模块里最容易踩坑但被显式测试覆盖到的分支。**没有后台线程,没有轮询定时器,核对只在真正需要展示状态时才做一次批量查询**——比定时器扫描更省资源,代价是"刚发生的退出"要等下次 UI 刷新才能被发现,workmux 用 sidebar daemon 的轮询(见下)填上这个延迟。

#### 核心机制三:Sandbox(三个项目里独一份,工程含量最高)

kooky/orca 都没有的子系统:agent 可以跑在**容器(Docker/Podman/Apple Container)或 Lima VM**里,与宿主机的 SSH key、AWS 凭证、GPG key 完全隔离,这样"YOLO 模式"(免权限确认自动执行)才敢开给 agent。

- **双后端**:容器是进程级/VM 级隔离 + 每会话新建即弃,自带 6 个 agent 的预置 Dockerfile(`docker/Dockerfile.{base,claude,codex,gemini,opencode,pi,omp}`,`include_str!` 编译进二进制);Lima 是持久化 VM,内建 Nix/Devbox 工具链支持
- **RPC 桥**(`sandbox/rpc.rs`,1778 行):guest 里的 workmux 二进制通过 TCP 连回 host 端 RPC server,JSON-lines 协议,支持 `SetStatus`/`SetTitle`/`SpawnAgent`/`Merge`/`Exec` 等请求——**沙盒内的 agent 调用 `workmux merge` 这类命令时,实际执行发生在 host 侧**,guest 只是转发请求,这样容器里不需要装 git/gh 等宿主工具链
- **网络代理**(`sandbox/network_proxy.rs`):HTTP CONNECT 代理 + 域名白名单,host 侧做 DNS 解析并拒绝解析到内网 IP 的域名(防止 allowlist 绕过内网访问),配合容器内 iptables 默认拒绝出站、只放行代理端口——**双保险**:代理挡应用层,iptables 挡"agent 直接无视代理环境变量"这种绕过
- **鉴权**:RPC token 和代理 token 都走 `constant_time_eq`(逐字节异或,不提前 return)做比较,防时序攻击——一个 10 行的工具函数,但说明这条 host↔guest 通道被当作真实的信任边界在设计,不是"能跑就行"

这套东西直接对应 Dozer"用户侧验收层"的核心诉求之一:**agent 可以自动执行,但不能拿到不该拿的东西**。workmux 证明了这条边界可以不侵入 agent CLI 本身(不需要 fork Claude Code)、靠外部容器/VM + 代理转发就能建立起来。

#### 数据模型速览

```
$XDG_STATE_HOME/workmux/
├── settings.json              # 全局 dashboard 偏好(排序/过滤/侧栏宽度…)
├── agents/
│   └── {backend}__{instance}__{pane_id}.json   # 一 pane 一状态文件
├── containers/{worktree_handle}/{container_name}   # 容器归属标记(空文件,内容是 runtime 名)
└── runtime/{backend}__{instance}.json         # sidebar daemon 产出的临时信号(如"疑似卡死"pane 集合)
```

`containers/` 目录值得一提:标记文件本身没有语义内容,只是"这个 worktree 名下曾起过这个容器"的存在性记录,配合 `list_containers` 在 `workmux remove` 时找到该清理的容器——**用文件系统当轻量级关系表**,和 `agents/` 目录同一套哲学,不引入 sqlite。

#### 测试文化:1358 Rust 单测 + 174 Python 端到端

Rust 侧 1358 个 `#[test]`,集中在纯函数(JSON 合并、backend 探测、PaneKey 编解码这类)——可以摆脱真实 tmux 环境快速跑。但 tmux/git worktree 的交互终究没法在 Rust 单测里高保真模拟,workmux 的解法是**独立的 Python 测试套件**(`tests/`,174 个 `test_*` 函数,`pytest` + 真实 tmux/git 子进程),覆盖 `add`/`merge`/`rebase`/`sandbox`/多 agent hook 安装等端到端场景。两层分工清晰:Rust 测纯逻辑,Python 测"真的起一个 tmux server 会不会翻车"。这是一个可复用的测试策略经验:**语言实现和验收测试的语言不必一致**,选对每层最省心的工具。

#### 对 Dozer 的启示

Dozer 与 kooky 同构(GUI + 自持 PTY 池 + hook socket),与 workmux 在"是否自研终端"这条轴上正好站在对面——所以 workmux 的 multiplexer 抽象层本身不直接可搬(Dozer 不遥控外部 tmux,`dozerd` 自己就是那个"tmux"),但其余三块高度可复用:

| workmux 子系统 | 对应 Dozer 位置 | 借鉴点 |
|---|---|---|
| `agent_setup/hooks.rs` 的 JSON 树合并/摘除 | `dozer-hook` 的安装逻辑 | 幂等合并 + 精确摘除 + 空壳清理,而不是覆盖写整个 settings 文件;用户自己的其他 hook 必须原样保留 |
| `state/store.rs` 一 pane 一文件 + 批量 reconciliation | `dozerd` 的会话存活/验收闭环存储 | 拉取式核对(dashboard 刷新时批量查一次)比后台定时器更省资源;`boot_id` 判定"服务重启 vs 真实退出"这条分支值得直接照搬到 dozerd 的 PTY 池崩溃恢复逻辑 |
| `sandbox/`(容器 + Lima + RPC 桥 + CONNECT 代理) | 若 Dozer 的"验收层"未来要做权限边界/网络限制 | 证明了隔离可以不侵入 agent CLI,靠外部容器/VM + host↔guest RPC 转发达成;`constant_time_eq` 这类细节说明这条通道要按信任边界设计,不是内部 IPC 随便传 |
| Rust 单测 + Python 端到端双层测试 | Dozer 自己的测试策略 | `cargo test` 测 dozer-core 纯逻辑,真实拉起 `dozerd`/PTY 的场景交给独立脚本层(Python 或 shell),不要硬塞进 `cargo test` |
| `config.rs` 单文件 5570 行 | Dozer 的配置 schema 设计 | 反面教材:功能全塞进一个大文件会让"新增一个 agent = 改一处"退化成"新增一个 agent = 改十几处分散在同一文件里"。Dozer 若走 TOML `[agents]` 配置,应从一开始按 agent/子系统拆文件,不要等到 5000+ 行才重构 |

最值得单独展开的一点:**workmux 没有"会话存活"问题是因为它把这个问题甩给了 tmux**。Dozer 选择自己持有 PTY(`dozerd` 的 PTY 池),就是主动放弃了这个免费红利,换来的是不依赖用户装 tmux、跨前端(GUI/未来 TUI/hook)统一会话模型的自由度——这个取舍在 [[dozer-vision]] 里已经定过,workmux 的存在只是从反面印证了"自持 PTY"确实是条更重的路,重量换来的是控制权。

---

## 13. 对照 Dozer 当前代码的参考价值再判断（2026-09-24 补充）

本节是在上述逐仓复核完成之后，对照 Dozer 当前真实代码状态（而不是 spec/分析文档描述的状态）重新核验各项目结论的适用性，并补充一个本报告尚未涵盖的过滤条件：同日的产品定位 brainstorming 已经把 [[dozer-vision]]（见 `CLAUDE.md` 项目现实）定稿为"治理/验收仍是核心身份，覆盖范围扩大到 vibe coding 全流程，agent 中立不变；但具体落点（是否/如何插件化、Workflow Kernel 等数据模型、Execution Environment、Decision Service）尚未立项"。这意味着本节的价值判断要按两层过滤：

1. **无论落点如何选都成立的"无悔"发现**——优先级最高，不依赖插件化/Workflow Kernel 是否成立；
2. **依赖某个具体落点才有意义的发现**——价值判断保留，但明确标注"等落点定了再排期"，不建议现在拍板。

### 13.1 用代码核验后的结论修正

逐个核对报告里点名的"Dozer 已知缺口"，结果如下（`gh`/`grep` 核实，非转述）：

| 报告里的判断 | 代码核验结果 | 结论 |
|---|---|---|
| §2 agent-deck 分析：Dozer `SessionRegistry` 纯内存 HashMap，daemon 重启全丢 | 属实：`crates/dozerd/src/registry.rs:9` 就是 `Mutex<HashMap<String, Arc<Session>>>`，没有任何 sqlite/save/load 调用 | **确认为真实缺口**，是本报告证据链最扎实的一条 |
| §12.4/§12.1 "hook 必须语义合并、精确摘除，不能覆盖写" | `crates/dozer-hook/src/install.rs` 已有 `arr.retain(\|e\| !entry_is_dozer(e))`，并且有专门测试 `install_is_idempotent_and_preserves_foreign_hooks`、`uninstall_removes_only_ours` | **这条纪律 Dozer 已经做到了**，不是缺口——三次独立跨项目验证的经验，Dozer 是第四个已经符合的样本，值得记一笔而不是列进"待办" |
| §12.1 Agent Deck 反例："多 provider 用巨对象+字符串分支，没有契约层" | Dozer 已有 `dozer-core/src/protocol.rs::AgentKind` 枚举（Claude/Codebuddy/Opencode/Codex/Goose/Aider/V8agent），31 个文件按枚举 match，分散在各自领域文件（`transcripts/parse.rs` 解析、`*_install.rs` 装 hook），不是单文件巨对象 | **反例已被规避**，不需要"趁早建 Agent Adapter trait"这条待办；Dozer 起步时机比 Agent Deck 晚，直接吸收了这条教训 |
| §1 表格：Agent 面板"多个 agent 并行管理，目前欠缺 worktree" | 全仓搜索 `worktree` 只命中 `git_log.rs`（git log 展示用）和 `files/state.rs`，Agent 面板/session 层没有 worktree 绑定 | **确认为真实缺口**，且是全部 24+ 个"整体相似"项目里共识最高的一条标准件 |
| §7 "Browser Test 插件以 Playwright sidecar 为主路径" | `extensions/browser.rs` 未发现 playwright/automation/screenshot 相关代码，当前确实只是手动浏览 | **确认为真实缺口**，与 CLAUDE.md 现状描述一致（"当前以浏览为主"） |

### 13.2 按新定位重新分层的参考价值

- **直接服务"治理/验收"核心身份、且不依赖落点决定的**（价值上调）：Vibe Kanban 的 diff 内联审阅、Basic Memory 的 provenance/关系化检索、Phoenix 的 trace/eval 概念、Open Browser Use 与 Playwright 的可重放证据包、Goose 的 inspector pipeline 式权限判断——这些都是"人类如何审阅/验收 AI 产出"这条主线上的具体机制，和 Dozer 的核心身份直接对应，不需要等 Workflow Kernel 或插件协议拍板就能吸收。
- **依赖具体落点、暂不建议排期的**（价值不变，但明确挂起）：Zed 的 manifest/extension registry、Extism/Wasmtime、wasmCloud、四个 MCP Gateway 的比较、container-use 的容器执行环境——这些结论全部成立，但它们回答的是"如果做插件化/Execution Environment，该怎么做"，属于用户明确搁置的"具体落点"，现在采纳等于替用户拍了板。报告本身的四级分类（A/B/C/D）没有这层区分，是本节新加的过滤维度。
- **daemon 化相关的发现优先级应上调到落点决定之前**：报告 §3.5.4/§11 已经指出"daemon 化要早于插件化完成"，本节的代码核验进一步坐实——`SessionRegistry` 持久化和 worktree 集成都不依赖"要不要插件化"这个决定，现在就能做，而且是后续任何落点（无论插件化与否）都绕不开的地基。

## 14. 立即跟进的 5 个功能点

以下 5 项从本报告的全部证据中挑出，筛选标准是：**(a) 至少两个独立项目给出一致证据，(b) 已用 Dozer 当前代码核实是真实缺口而非臆测，(c) 不需要等"具体落点"决定就能开始**。按建议着手顺序排列，前两项互相依赖、建议合并规划。

### 1. Worktree 生命周期接入 Agent 面板（create / pause / resume / archive，不只 create/remove）

**证据**：本报告 24+ 个"整体相似"项目里，tty7、Workmux、Agent Deck、Tidebreak、Arbor、Orca、Vibe Kanban 全部独立收敛到"一 agent 一 worktree"是并行 agent 的安全基线，而不是高级功能；Claude Squad 唯一被保留的独立价值就是 Pause/Resume——暂停释放 worktree 但保留分支，恢复时重建（§12.2）；agent-deck §12.1 进一步给出"任务状态应区分 active/suspended/archived"的具体状态词。

**对应 Dozer 现状**：Agent 面板目前只有多会话并行，没有 worktree 隔离（§13.1 已核实），也是用户在本轮定位讨论中自己点名的现有缺口。

**为什么现在做**：不依赖插件化或 Workflow Kernel 是否成立——无论 V2 最终选哪条路线，"多 agent 并行时物理隔离工作目录"都是绕不开的地基，且是外部证据密度最高的一条。

### 2. `dozerd` 持久化 Session/Worktree 状态，并在启动时与真实进程核对（reconciliation，不盲信磁盘缓存）

**证据**：`SessionRegistry` 纯内存 HashMap 已核实为真实缺口（§13.1）；tty7、Agent Deck 都从"单文件/纯内存"迁移到了更持久的存储（Agent Deck 迁到 SQLite，`withBusyRetry` + 迁移前备份 + 防"空扫"防御性断言）；同时 kooky/workmux/agent-deck 三次独立验证"运行时字段不落盘、启动时按真实进程/PTY 状态核对，不轻信磁盘缓存"这条纪律（§12.1 核心机制二）。

**为什么现在做**：报告 §11 已把它列为统一修正的第 2 条（"daemon 持有 PTY/session/Agent state/event stream，并处理回压、重连和版本协商"），且这是做第 1 项（worktree 集成）时必然要触碰的同一层代码——建议合并规划，一次做透，而不是先做 worktree 再回头补持久化。

**顺手做**：agent-deck 的 `IsForkAwaitingStart`/`ForkStartCommand` 模式——把"一次性启动参数"和"持久重启身份"分开存、消费即清空，防止 daemon 重启后重放一次性动作（§12.1 核心机制一），做第 2 项时同批实现，成本很低。

### 3. Todo 面板向"任务 = 隔离 workspace + 内联 diff 审阅"靠拢

**证据**：Vibe Kanban（A 级，Rust 同语言）验证了"Todo 不应只是列表，可以成为 task/worktree/review/deployment 控制面"，报告明确评价"这是 Dozer Todo 插件首个 vertical slice 的直接参照"（§2）。

**为什么现在做**：diff 内联审阅本质上就是"验收"这个核心身份的具体产品化，直接服务本轮定位裁决，不需要等插件化/Workflow Kernel 落点——先把"一个任务能直接看到它绑定的 worktree 和变更 diff"这个体验做出来，落点决定后再考虑要不要把 Todo 拆成独立插件。

### 4. 浏览器自动化测试证据采集，走 Playwright sidecar + broker 协议（不嵌入 Node/Playwright 进 Rust Host）

**证据**：Playwright（A 级）提供确定性 automation、trace/DOM/network/console 证据包；Open Browser Use（A- 级）验证了"owner-only IPC + capability gate + 长驻 browser session"这个更贴近 Dozer broker 模型的实现路径，两者独立收敛到同一结论。

**对应 Dozer 现状**：`extensions/browser.rs` 目前只有手动浏览（§13.1 已核实），也是用户在本轮讨论中主动点名的"后续将增强"方向。

**为什么现在做**：这是用户已经明确的既定方向，两个独立项目验证了同一条实现路径（sidecar 进程 + 受控协议，不吃进 Rust Host），风险很低，且证据采集能力本身就是"验收"核心身份的一部分。

### 5. Agent CLI 的 fork/resume 优先透传原生标志，不自己解析会话文件"逻辑复制"

**证据**：Agent Deck 核心机制一（§12.1）给出四个独立样本：Claude `--fork-session`、Codex `fork`、OpenCode `--fork`（且**有明确的历史教训**——OpenCode 曾经用 `export | sed | import` 手工克隆，后来切到原生标志后整条手工路径被废弃）、Pi 需要手工定位文件路径但仍交给原生 `--fork` 消费；Gemini 因为没有原生原语，Agent Deck 干脆不支持，没有自己发明等价实现。

**为什么现在做**：这不是一个独立工程任务，而是一条应该写进第 1/2 项设计评审的具体准则——当 Dozer 后续给 worktree/session 加"恢复历史对话"这类能力时，第一步应该是核实目标 agent CLI 是否已有原生 `--resume`/`fork` 标志并直接透传，而不是解析 `~/.claude/projects/**/*.jsonl` 自己实现"逻辑复制"；OpenCode 的历史证明手工复制路线迟早被原生标志淘汰。现在把这条准则写进报告，是为了避免第 1/2 项落地时重蹈覆辙。
