# Dozer V2 开源生态调研

> 调研日期：2026-09-24（§3.7-§4.11 为同日第二轮补充调研）  
> 目标：围绕《Dozer V2 架构分析》验证产品定位与工程路线，并识别可以借鉴、集成或直接依赖的开源项目。  
> 范围：优先采用项目官方仓库、官方文档和协议规范；活跃度与 API 状态以选型时再次核验为准。第二轮全部项目均已用 `gh api` 核实 stargazers/license/language/最近 push 时间，避免复述未经验证的网络摘要。

## 1. 结论先行

Dozer V2 的方向具备产品必要性和工程可行性，但不应被定义为“又一个多 Agent GUI”。当前开源生态已经充分验证了以下需求：

- 多 Agent 并行和“一任务一 worktree”正在成为 Agent IDE 的基础能力；
- 用户需要在同一个工作台中查看会话、终端、变更、浏览器和交付状态；
- Agent 执行环境正从本机进程扩展到容器、远程 workspace 和按任务创建的 sandbox；
- MCP 适合工具与 Agent 协作，但不能独自承担 Host 插件生命周期、UI 嵌入和权限管理；
- AI 开发过程需要结构化事件、可回放证据、成本统计和人工验收，而不只是保存聊天记录。

同时，现有项目大多只覆盖其中一部分：

1. Tidebreak、Daintree、Arbor、Pane 等验证了并行 Agent 工作台；
2. Zed、Lapce、Extism 验证了 manifest、Wasm 与贡献点式插件；
3. OpenHands、Coder、Daytona 验证了外部执行环境和 workspace 控制面；
4. Playwright、Phoenix、Basic Memory 分别提供测试证据、可观测性和记忆机制参考；
5. 尚未发现同时把“可扩展 Host + 插件 App + Agent/MCP 网络 + 工作流闭环 + 审计治理”作为统一产品模型的成熟开源项目。

因此，Dozer 的机会不在于单点功能领先，而在于把这些已经分别成立的机制组合成一个稳定、可扩展、local-first 的 Vibe Coding 操作系统。

### 1.1 建议采用的四级分类

| 级别 | 含义 | 当前候选 |
|---|---|---|
| A：直接采用 | Rust 依赖或已有依赖，可进入近期实现 | `rmcp`、`notify`；`testcontainers-rs` 用于测试 |
| B：做适配器 | 作为外部进程、服务或 CLI 集成，不嵌入 Host | Docker/Podman/Apple container、Playwright、Laya、OpenTelemetry 后端；container-use（参照实现，不直接依赖其 Go 二进制） |
| C：做技术验证 | 有价值，但须通过 spike 验证边界、体积和维护成本 | `jsonrpsee`、Bollard、Extism/Wasmtime、OTel Rust |
| D：只借鉴设计 | 学习产品或架构，不形成代码依赖 | Tidebreak、Arbor、Zed、OpenHands、Basic Memory、Phoenix、Vibe Kanban、Crystal、Zellij、wasmCloud、Temporal、Tauri ACL、Deno 权限模型 等 |

最重要的选型纪律是：**参考一个项目，不等于依赖一个项目；能通过稳定协议连接，就不要把大型运行时编进 Host。**

## 2. Dozer V2 的比较坐标

本调研使用以下能力轴，而不是简单比较功能数量：

| 能力轴 | Dozer V2 目标 |
|---|---|
| Host | iced 原生微内核，负责窗口、导航、状态、权限、生命周期和审计 |
| 插件 | 进程隔离优先；声明式 UI、WebView 或外部窗口；独立开发、发布、升级 |
| Agent | 多 Agent 并行，任务隔离，人工可监督、可接管、可验收 |
| 协作 | Plugin Protocol 管 Host 生命周期；MCP 管 Agent 工具与跨插件语义协作 |
| Workspace | 项目、任务、worktree、终端、上下文、记忆和交付物统一关联 |
| 执行环境 | 本机、容器、SSH、远程 sandbox 通过 `ExecutionEnvironment` 抽象接入 |
| 治理 | 会话、Token、工具调用、代码质量、测试证据、权限与决策均可审计 |
| 决策 | Laya 一类小模型作为可选决策 Provider，不进入 Host 可信核心 |

## 3. 整体上与 Dozer 相似的项目

### 3.1 第一组：应持续跟踪的直接参照

| 项目 | 已验证的能力 | 对 Dozer 最有价值的部分 | 不应照搬的部分 |
|---|---|---|---|
| [Tidebreak](https://github.com/brightwave-inc/tidebreak) | Rust/Tauri、本地优先、多 Agent、分支与 worktree、PR/CI/diff、浏览器测试、用量和审批 | 产品闭环与 Dozer 最接近；适合检查工作台信息架构和 Agent 事件结构 | 项目较新；不能以其当前实现成熟度替代 Dozer 自己的协议设计 |
| [Daintree](https://github.com/daintreehq/daintree) | Electron、多 Agent 并行、worktree、上下文注入、Git 操作、自动化 | 其插件规划包含 manifest、贡献点、Host API 和开发循环，适合对照 Dozer 插件 SDK | Electron 扩展模型和原生 iced Host 的 UI 边界不同 |
| [Arbor](https://github.com/penso/arbor) | Rust/GPUI、daemon-backed、桌面/Web/CLI/MCP、多 worktree、终端、diff、PR 上下文 | “daemon 持有状态、多个客户端访问”非常适合 Dozer 会话和 PTY 分层 | GPUI 不是 Dozer 必须更换 UI 框架的理由；借鉴进程模型即可 |
| [OpenChamber](https://github.com/openchamber/openchamber) | 桌面/Web/VS Code/移动端、目标持续执行、同任务多模型并跑、结果选择与融合 | 对“同一任务多实现—比较—验收”的产品流程有直接参考价值 | 多端覆盖会显著扩大 V2 范围，不能早于 Host/协议稳定性 |
| [Limboo](https://github.com/limboo-ai/limboo) | 项目、会话、文件监听、索引、Git/worktree、终端、memory、权限、任务与 artifacts | Session Bundle 将仓库、分支、聊天、终端、检查点、权限、记忆、任务和产物关联起来 | 不应把所有状态重新耦合成一个大应用对象；Dozer 要保留领域边界 |
| [Pane](https://github.com/greenfield-inc/Pane) | Agent 无关、自动 worktree、diff/Git/浏览器/资源管理、终端间上下文 | 每个 pane 独立 worktree、端口范围和 secret 的隔离方式 | AGPL 代码不可在未评估许可证影响前直接复用 |

这组项目共同证明：worktree 已经不是高级功能，而是并行 Agent 的安全基线。Dozer 当前“多个 Agent 并行但欠缺 worktree”的缺口，应提升到 V2 第一阶段，而不是作为后续增强。

### 3.2 第二组：聚焦调度与监督体验

| 项目 | 参考价值 |
|---|---|
| [Ateam](https://github.com/clawnify/ateam) | worktree-per-task、Mission Control、commit/push/update/merge、状态 hook 和安全清理建议，适合设计任务卡的 Git 生命周期 |
| [ai-14all](https://github.com/ai-creed/ai-14all) | session 固定绑定 worktree 和 terminal，以人工监督和合并闸门为核心，适合定义“Agent 完成”与“人类接受”的区别 |
| [Argos](https://github.com/dvaJi/argos) | 将 Agent、模型、MCP、skills 和 memory 放入统一控制面，适合参考 Provider Registry 与能力发现 |
| [Elves](https://github.com/mvmcode/elves) | Tauri、任务/worktree workspace、多仓库协同和本地 SQLite，适合多仓库任务模型 |

### 3.3 补充调研：tty7、CleeCode、MonoCode、AutoDev、IfAI

这五个项目都有参考价值，但不处于同一层次。建议按下表分级：

| 项目 | 参考等级 | 主要价值 | 对 Dozer 的定位 | 是否适合作为依赖 |
|---|---|---|---|---|
| [tty7](https://github.com/l0ng-ai/tty7) | **A：重点研究** | Rust daemon、持久 PTY、Agent 状态、CLI、远程 workspace、SSH、协议版本治理 | Host/daemon/terminal/remote execution 的直接工程参照 | 不直接依赖整个项目；评估可复用 crate，并核验公共 API 稳定性 |
| [CleeCode](https://github.com/msavox/cleecode) | **A-：重点研究局部机制** | Agent CLI 原样运行、编辑器反向暴露 MCP、UI 操作授权、外部修改协调 | MCP Context Bridge 与人类控制权的优秀参考 | 不依赖应用；可研究其 MIT 代码中的 MCP 和 buffer 协调实现 |
| [MonoCode](https://github.com/hardbeat920/monocode) | **B：产品对照** | 多 Agent CLI 的统一 GUI、复用用户既有订阅和登录、跨平台分发 | Agent launcher/session shell 的最低产品基线 | 不建议依赖；适合作为 UX 和 provider compatibility 测试对象 |
| [AutoDev](https://github.com/phodal/auto-dev) | **A-：架构与能力参考** | 多 Agent runtime、MCP、ACP、skills、AGENTS.md、code graph、多端 shell | Provider 抽象、Agent 生态兼容和 core/UI/server 分层参考 | Kotlin/MPL-2.0，不适合作为 Rust Host 依赖；可协议互操作 |
| [IfAI](https://github.com/peterfei/ifai) | **B+：工作流产品参考** | YAML DAG、并行 Agent、共享知识、结果聚合、任务树、审批注册表、事件日志 | Workflow 插件、Todo 编排和审计 UI 的参考 | 不建议依赖；核心 AI 模块不在 MIT 许可范围内 |

#### tty7：对 Dozer 最有工程参考价值

tty7 的关键不只是“又一个 Rust 终端”，而是把终端状态从窗口生命周期中剥离：后台 server 持有 shell 和 pane，GUI 与 CLI 都是客户端。它还将相同的 `LocalHost` 能力放到远端 `tty7-server` 中，使文件、仓库、diff、worktree、终端与 Agent 状态整体迁移到远端，而不是仅提供一个 SSH shell。

这直接验证了 Dozer 的以下设计：

- PTY 和 Agent session 应由 daemon/runtime 持有，不能由 iced widget 持有；
- GUI、CLI、Agent 和未来插件应通过同一控制协议访问 session；
- local 与 remote 不应形成两套领域模型，差异应收敛在 Provider；
- daemon 协议必须有明确 dialect/version，版本不兼容时需要可解释的升级与重连流程；
- Agent 自动化需要非交互原语，例如 `run`、`send`、`capture`、`events`、`wait`，而不是模拟用户 attach 到终端；
- pane 输出必须有 observer budget 和 backpressure，防止慢客户端或 runaway output 拖垮 daemon；
- shell 注入的上下文变量可以把 pane、workspace、socket 身份传给 Agent CLI，实现零配置路由。

tty7 已将代码拆分为 `tty7-core`、`tty7-server` 和 `tty7-cli`，对 Dozer 的 crate 边界也有直接参考价值。建议单独开展一次代码级调研，重点阅读：

1. server 单实例锁、stale socket 恢复和崩溃重连；
2. control/pane 双协议、握手和版本升级；
3. PTY 持久化、输出回压和多 observer；
4. Agent 检测、hook、native session id、waiting/done 状态；
5. 本地与远程 `LocalHost` 对称实现；
6. SSH server 安装、升级、校验和 secret 管理。

它不意味着 Dozer 应从 iced 切到 GPUI。tty7 最值得学习的是无 UI 的 core/server 边界；渲染框架属于可替换的客户端实现。

#### CleeCode：MCP 不只是调用外部工具，Host 本身也可以成为工具

CleeCode 不重建 Agent 聊天 UI，而是在真实 PTY 中运行 Claude Code、Codex、OpenCode、Gemini 等 CLI，并让 CLI 自己处理账号与凭据。这与 Dozer 的 Agent Adapter 路线一致：Host 不需要成为所有模型供应商的认证代理。

更有价值的是，CleeCode 自己可作为 MCP server，向 Agent 提供：打开文件、当前选择、诊断、定位与预览，以及经过用户许可的 `edit_buffer`。其几个细节值得写进 Dozer 的 MCP Context Bridge：

- unsaved buffer 必须明确标记，不能让 Agent 误以为磁盘内容就是用户眼前内容；
- Agent 修改磁盘后可以自动刷新，但不得覆盖含有用户未保存编辑的 buffer；
- 写 UI buffer 属于高风险能力，应支持 `once`、`always this session`、`deny` 等授权范围；
- Host 可以把 selection/diagnostic/cursor 作为引用写入 Agent 输入，但不替用户自动提交；
- 只有从正确 session 启动的 Agent 才能获得 UI context，错误上下文宁可拒绝，也不要返回另一个窗口的数据；
- 可选外部预览工具缺失时应能力降级，而不是导致整个 App 无法启动。

这说明 Dozer 的 MCP 拓扑不应只有“Host 连接插件 MCP server”，还应包含一个受 capability 控制的 `dozer-context` server，让 Agent 安全读取当前 workspace、task、selection、diagnostic、memory 和 artifact，并请求 Host UI 动作。

#### MonoCode：产品基线有用，架构区分度较低

MonoCode 用 Tauri 包装用户已经安装并登录的多个 coding-agent CLI，把 tab 作为 session、composer 作为输入。它验证了两个产品判断：

- 用户希望复用已有订阅和认证，而不是在每个桌面工具中重新配置模型密钥；
- “统一启动和管理多个 Agent CLI”本身已经快速商品化，不能成为 Dozer 的长期差异化。

对 Dozer 来说，MonoCode 更适合作为回归矩阵：相同 Agent 在 Dozer 中是否同样易于发现、启动、恢复、输入和结束。它目前公开定位较聚焦且项目自称仍处早期，没有展示插件 runtime、worktree 隔离、审计闭环或环境 Provider，因此不宜作为核心架构蓝本。

#### AutoDev：重点看跨宿主 Agent Runtime，而不是复制多端范围

AutoDev 3.0 将共享 Agent engine、tools、MCP 与 `AGENTS.md` 加载放在 `mpp-core`，再通过 Desktop、IDEA、VS Code、CLI、Web、移动端和 server 等不同宿主提供 UI。其代码还包含 Agent Client Protocol model，并支持 MCP、A2A command、Claude Skill 和 SpecKit 等生态入口。

对 Dozer 的启示是：

- Agent runtime 应独立于 iced 页面，Host、CLI、headless server 和插件可以共享；
- `AGENTS.md`、skills、MCP、项目规则都应归一为可追踪的 Context Source；
- coding、review、document、database、artifact 等不必硬编码为 Host 面板，可以是共享 runtime 上的不同 Agent/插件；
- code graph 更适合作为可选索引 Provider，由 Code Health、Memory 和 Agent Context 共同消费；
- 协议生态可能不只有 MCP，Host 的 Provider Registry 应允许 ACP/A2A 类 adapter。

但它的全平台目标明显大于 Dozer V2 当前范围。Dozer 应借鉴 core/UI/server 分层，不应在微内核和插件协议尚未稳定时同步扩展移动端和 Web。AutoDev 采用 MPL-2.0/Kotlin Multiplatform，也更适合架构参考或协议互操作，而非 Rust 直接依赖。

#### IfAI：工作流能力高度相关，但需要区分 README 能力与可复用开源实现

IfAI 展示了与 Dozer Todo/Workflow 方向高度相关的一组产品能力：YAML DAG、顺序/并行节点、Agent 间调用、知识共享、结果聚合、自然语言路由、Task Tree、逐项 diff 接受/拒绝、Token 成本和工作流可视化。其版本记录还提出 JSONL append-only 会话日志、事件总线和声明式 `ToolApprovalRegistry`。

值得 Dozer 借鉴的不是“内置九个固定 Agent”，而是以下通用模型：

```text
WorkflowDefinition
  └─ Node
      ├─ input contract
      ├─ assigned capability / agent selector
      ├─ permission policy
      ├─ retry / timeout / compensation
      └─ output artifact contract

WorkflowRun
  └─ NodeRun
      ├─ status + attempts
      ├─ event stream
      ├─ decision + provenance
      ├─ usage
      └─ artifacts
```

Dozer 还应避免其中风险较高的方向：

- 不采用全局“100% 信任模型”，工具权限必须由 Host capability broker 管理；
- Agent 链式调用要有深度、预算、时间和取消传播限制；
- “知识共享”必须落到有 scope、provenance、有效期和访问控制的 Memory API；
- DAG 图只是定义与状态视图，真正权威数据仍是 append-only run events 与 artifact；
- 项目许可证明确说明核心 AI、RAG、Agent 工具链和上下文算法受独立商业许可约束，因此只能借鉴公开模型和开源框架，不能默认相关实现可复用。

### 3.4 对这五个项目的综合判断

它们进一步强化了 Dozer V2 的四个优先事项：

1. **daemon 化要早于插件化完成**：如果 PTY、session 和 Agent 状态仍绑在 GUI 中，插件只是拆 UI，无法形成稳定底座；
2. **Host 应同时是 MCP client 与受限 MCP server**：既消费插件能力，也向 Agent 暴露当前 workspace 和 UI context；
3. **工作流必须建在统一 Run/Event/Artifact 模型上**：DAG、Todo、并行 Agent 和审计才不会各自维护一套状态；
4. **“支持多个 Agent CLI”是基础能力而非护城河**：Dozer 的差异仍然是插件生态、执行环境、共享上下文和可治理闭环。

### 3.5 已有代码级分析项目

`docs/analysis` 中已有 11 个项目的代码级分析。这些材料比 README 层面的横向调研更深入，本节将其结论纳入 Dozer V2 的统一比较框架；原文继续保留，供实现具体模块时追溯源码证据。

#### 3.5.1 驾驭外部 Agent 的编排层

这一组与 Dozer 产品边界最接近：它们不取代 Claude Code、Codex 等 Agent，而是管理其进程、会话、工作目录和状态。

| 项目 | 路线 | 最值得参考 | 主要限制/反例 | 详细分析 |
|---|---|---|---|---|
| [Kooky](https://github.com/iAmCorey/kooky) | SwiftUI + libghostty，原生 macOS Agent 终端 | 独立轻量 hook、surface id 路由、OSC 7/133、tool call 配对、session resume、运行时状态不落盘 | 平台与 libghostty 绑定；没有平台级插件和工作流 | [代码级分析](../analysis/kooky-分析.md) |
| [Orca](https://github.com/stablyai/orca) | Electron 调度台 + PTY daemon + worktree + relay | daemon 持有 PTY、多客户端 attach、执行宿主抽象、远程 relay、App 可被 Agent 操作 | 产品面过宽，Node daemon 为性能和终端保真承担较大成本 | [代码级分析](../analysis/orca-分析.md) |
| [Workmux](https://github.com/raine/workmux) | Rust CLI/TUI，复用 tmux/Zellij/WezTerm/kitty | multiplexer trait、worktree 生命周期、hook 语义合并、状态 reconciliation、sandbox、测试密度 | 会话能力受外部 multiplexer 上限约束，没有统一原生 PTY 模型 | [代码级分析](../analysis/workmux-分析.md) |
| [T3 Code](https://github.com/pingdotgg/t3code) | Web/desktop/mobile 共用 server，结构化 Agent 协议优先 | Agent app-server/ACP 优先、远程四层模型、server 自更新、连接状态机、checkpoint/review | 调研时真正落地的 Provider 少于 README 宣称范围；验收只到 diff/review | [代码级分析](../analysis/t3code-分析.md) |
| [Garcon](https://github.com/cfal/garcon) | Bun/Svelte 自托管多 Agent UI | 七类 CLI 的结构化子进程管道、permission 双向闭环、worktree/diff staging、跨 Agent transcript 转移 | PR 支持偏只读；“跨 Agent 迁移”是文字重放；许可证有附加条款 | [代码级分析](../analysis/garcon-分析.md) |
| [Agent Deck](https://github.com/asheshgoplani/agent-deck) | Go/Bubble Tea + tmux 的多 Agent 会话牌桌 | 原生 session fork 委托、一次性启动指令、共享 MCP 进程代理、JSON-RPC id 重写、子进程收尸 | Provider 巨对象是反例；依赖 tmux；部分抽象边界形成较晚 | [代码级分析](../analysis/agent-deck-分析.md) |
| [Claude Squad](https://github.com/smtg-ai/claude-squad) | Go/Bubble Tea + tmux + worktree | Pause/Resume：保留分支、释放 worktree、恢复时重建；轻量多 Agent UX | 纯终端文本匹配状态、单文件非原子存储、缺少 sandbox，适合作为降级反例 | [代码级分析](../analysis/claude-squad-分析.md) |

这组项目形成了几个强共识：

1. **Agent Adapter 应优先使用结构化协议。** Codex app-server、ACP、Claude stream-json 等存在时，结构化通道负责控制和审计；PTY 保留给真实交互或无协议 Agent；hook 是第二层；终端文本匹配只能是明确降级。
2. **会话所有权必须在底层确定。** tmux 路线以较低成本获得持久会话；Dozer 自持 PTY 会更重，但能获得跨 GUI/CLI/plugin/remote 的统一语义。既然选择后者，就必须把 daemon 恢复、回压、observer 和协议版本列入 P0。
3. **worktree 生命周期不止 create/remove。** 还需要 pause、resume、archive、reconcile、dirty 检查、分支占用检测、容器/端口/secret 资源回收。
4. **hook 安装必须语义合并与精确摘除。** 不覆盖用户完整配置；hook 小程序保持无状态、快速启动，配对和去重在 daemon 侧完成。
5. **原生 Agent 能力应优先透传。** session fork、resume、permission mode、sandbox 参数如果 Agent 原生提供，不要通过解析私有会话文件重新实现。
6. **“工具调用批准”不同于“交付验收”。** 前者是执行前 capability gate；后者是 Goal/Acceptance Criteria 对 Delivery 的治理结论，两者需要分别记录并关联。

#### 3.5.2 自己实现模型循环的 Agent Runtime

这一组不是 Dozer 的整体产品模板，因为它们自己拥有模型调用与工具执行权；但它们在权限、记忆、子任务、协议和工程治理上提供了更深的参考。

| 项目 | 路线 | 最值得参考 | 对 Dozer 的边界提醒 | 详细分析 |
|---|---|---|---|---|
| [jcode](https://github.com/1jehuang/jcode) | 全 Rust、自研模型循环/TUI/记忆/swarm | 按破坏半径的四级命令风险、语义记忆图、上下文压缩、Agent 间变更感知、CI 棘轮护栏 | 深度治理来自“自己是执行者”；Dozer 经外部 Agent hook 能获得的控制力更弱 | [代码级分析](../analysis/jcode-分析.md) |
| [SeekCode](https://github.com/kafkazhang/seek_code) | 小型 Electron Agent | 强制危险命令审批、出网白名单、工作记忆固定注入、克制的 fan-out/fan-in | 小规模可行方案，适合 P0/P1；正则安全判断不能视为完整沙箱 | [代码级分析](../analysis/seek_code-分析.md) |
| [Goose](https://github.com/aaif-goose/goose) | Rust Agent core + Electron/Node UI + MCP/ACP | 多路 `ToolInspector`、MCP extension manager、统一 Provider registry、SQLite session、压缩优先记忆、Open Plugins hooks | 支持 MCP 不等于治理 MCP；stdio 扩展没有天然隔离，安全能力必须另建 | [代码级分析](../analysis/goose-分析.md) |
| [OpenCode](https://github.com/anomalyco/opencode) | Bun/TypeScript 主流 Agent 平台 | HTTP/OpenAPI+SSE 控制面、细粒度阻塞式权限、SQLite schema 演进、ACP、受限递归 subagent | Provider 接口本身不难，供应商私有怪癖才是长期成本；Dozer 不应进入模型 Provider 维护战 | [代码级分析](../analysis/opencode-分析.md) |

这组项目帮助 Dozer 明确四项治理设计：

- **权限判断应是可组合 inspector pipeline。** 静态破坏半径、用户策略、MCP annotations、出网目的地、Agent 自带模式和人工审批分别提供信号，由 Policy Engine 汇总为 `allow / ask / deny`；不把所有判断写进一个大函数。
- **必须承认控制权差异。** Dozer 驾驭外部 CLI 时，只能在其协议、hook 和 sandbox 允许的边界内治理；需要强保证的任务应放入 Dozer 掌控的 `ExecutionEnvironment` 中。
- **Memory 没有唯一答案。** jcode 采用语义图、自动抽取和整理；Goose 偏向完整上下文、摘要压缩和主动记录；SeekCode 用有界工作记忆。Dozer 应按 working/project/shared 三层组合，而不是一次选定单一技术流派。
- **Subagent 默认采用有界 fan-out/fan-in。** 限制递归深度、预算和可调用工具；实时 swarm 通信作为高级能力，不作为 V2 最小编排模型。

#### 3.5.3 跨项目机制选择矩阵

| Dozer 问题 | 首要参考 | 次要/反例参考 | V2 选择 |
|---|---|---|---|
| PTY 与会话持有 | tty7、Orca、Kooky | Workmux、Agent Deck 的 tmux 路线 | Dozer daemon 自持，GUI 只是客户端 |
| Agent 接入 | Garcon、T3 Code、Goose | Kooky hook；Claude Squad 文本匹配 | 结构化协议 → hook → PTY/text 降级 |
| Worktree | Workmux、Orca、Claude Squad | jcode 的共享目录 swarm 反方意见 | 默认 task/worktree 隔离，显式共享须声明 |
| Agent 状态 | Kooky、Workmux、tty7 | Claude Squad 字符串匹配 | 结构化事件 + reconciliation |
| MCP 多会话 | Agent Deck、Goose、CleeCode | 单 session 直连 | Host service directory + 可共享 server + id/租户隔离 |
| 权限治理 | Goose、jcode、OpenCode | SeekCode 正则、Agent 原生权限模式 | inspector pipeline + capability broker |
| Session fork/resume | Agent Deck、Kooky、Garcon | 自行复制私有历史文件 | 委托原生命令，Dozer 只保存身份和 lineage |
| Memory | jcode、Goose、SeekCode | Basic Memory、Mem0 | working/project/shared 分层，统一 provenance |
| 子任务编排 | OpenCode、Goose、SeekCode | jcode swarm、IfAI DAG | 有界 fan-out/fan-in，DAG 建在统一 Run 模型上 |
| 远程运行 | tty7、T3 Code、Orca | 单纯 SSH pane | 与 local 同领域模型，Provider/transport 可替换 |
| 验收治理 | Dozer 自身 | T3 checkpoint、Garcon approval | 保留 Goal→Delivery→Acceptance 差异化 |
| 工程规模治理 | jcode guardrail、Workmux/Agent Deck 测试 | Agent Deck 巨对象、Claude Squad 薄测试 | crate 边界 + dependency rules + CI ratchet |

#### 3.5.4 对 P0 顺序的修正

将这 11 份代码级分析与本轮 GitHub 调研合并后，P0 的先后关系应更明确：

```text
1. dozer-protocol / dozer-core 领域契约
          ↓
2. daemon 持有 PTY、session、Agent state 和 event stream
          ↓
3. Task–Worktree–Environment 资源模型
          ↓
4. Plugin Supervisor + Todo 外置插件
          ↓
5. dozer-context MCP + 插件 MCP service directory
          ↓
6. Permission / Audit / Artifact 统一闭环
```

插件化不能替代 daemon 化：如果 session 和 PTY 仍由 iced 页面持有，Todo 即使被拆成独立进程，Agent、worktree、远程执行和崩溃恢复依然没有稳定宿主。反过来，daemon 也不能成为新的超级单体；它只持有运行时资源和协议状态，Todo、Code Health、Memory、Browser Test 等领域规则仍属于插件。

### 3.6 Dozer 与同类产品的差异化判断

Dozer 不应把差异化表述为“支持更多 Agent”。更持久的差异是：

- **它是可扩展底座，不是固定功能集合**：Todo、Code Health、SSH、Browser Test 都可以是独立插件 App；
- **它把开发过程建模为闭环**：Goal → Task → Execution → Delivery → Check → Acceptance；
- **它将人类监督设为一等能力**：对话、Token、权限、测试、质量和决策均可追溯；
- **它允许插件互相提供 Agent 能力**：MCP 是能力网络，Host 只负责注册、授权、发现和路由；
- **它不绑定唯一执行位置**：同一任务可运行在 local、container、SSH 或远程 sandbox；
- **它允许不同 UI 技术共存**：iced 保持稳定 Host shell，插件按安全级别选择声明式 UI、WebView 或外部 App。

### 3.7 第二轮补充：与 Dozer 语言/架构更接近的两个直接参照

| 项目 | 语言/许可证/star（2026-09-24 核实） | 已验证的能力 | 对 Dozer 最有价值的部分 | 不应照搬的部分 |
|---|---|---|---|---|
| [Vibe Kanban](https://github.com/BloopAI/vibe-kanban) | Rust，Apache-2.0，28.2k star，活跃至今（**官方已宣布 sunsetting**，转社区维护） | Kanban 任务面板 + 每任务一 workspace（分支/终端/dev server）+ 内置浏览器预览 + diff 内联评论 + 10+ Agent 切换 + PR 生成/合并 | 全流程最接近 Dozer 的"目标→任务→执行→审阅→合并"闭环，且是 **Rust 实现**，crate 边界、worktree-per-workspace、PR 生成流程可直接读源码核对；npx 一条命令启动，值得对照其"降低试用门槛"的分发方式 | 它是单体 Web/Tauri 式应用，没有插件 runtime 和权限模型；sunsetting 状态提醒 Dozer 不能假设"功能覆盖全就等于产品能立住"，仍需回答差异化问题 |
| [Crystal](https://github.com/stravu/crystal)（现由 [Nimbalyst](https://nimbalyst.com/crystal/) 延续） | TypeScript/Electron，MIT，3.1k star，**已停止更新（2026-02 起），仓库标注被 Nimbalyst 取代** | 多 Claude Code/Codex 会话并行 worktree、会话持久化可恢复、squash 多次 commit 后统一 rebase 回主分支 | "先在 worktree 里让 Agent 自由 commit，交付前 squash+rebase 成一次干净变更"这个 Git 生命周期值得写进 Dozer 的 Worktree Manager；会话持久化恢复的 UX 细节也可参考 | Electron 技术栈和已废弃状态；不要把 Crystal 当作活跃维护的参照实现，只取其已验证过的工作流设计 |

此外，**Conductor（Melty Labs）** 是同一细分市场里认知度很高的 macOS 原生 App（每 Agent 一个隔离 worktree、可视化 dashboard、diff viewer），但**官方未开源**，只能作为竞品认知补充，不构成开源参照，不纳入 §9 仓库清单。

这两个项目连同 §3.1-§3.3 已有条目一起，把"Kanban/Task 编排 + worktree 隔离 + 内联 diff 审阅 + PR 收尾"进一步坐实为 Vibe Coding 工具的标配而非差异化功能；Vibe Kanban 的 sunsetting 也提示一个具体风险：**光把这套标配做全，不足以构成长期产品护城河**，Dozer 仍要靠"治理/验收层 + 可扩展插件底座"这两条差异化路线站住脚，不能只对标"功能对齐"。

## 4. 对某个部分具有参考价值的项目

### 4.1 插件清单、贡献点和兼容性：Zed

[Zed Extensions](https://github.com/zed-industries/zed/blob/main/docs/src/extensions/developing-extensions.md) 值得作为 Dozer 插件开发体验的首要参考：

- `extension.toml` 描述身份、版本和贡献能力；
- 扩展可以贡献语言、主题、调试器、代码片段和 MCP server；
- Rust 扩展编译到 `wasm32-wasip2`，Host 通过版本化 API 提供能力；
- 支持本地安装开发版扩展，形成短反馈循环；
- [扩展注册表](https://github.com/zed-industries/extensions) 将扩展源码、版本和发布治理分离。

Dozer 应借鉴的是 manifest、贡献点、API 版本、开发安装和 registry 流程，而不是强制所有插件采用 Wasm UI。

建议 manifest 至少包含：

```toml
[plugin]
id = "org.example.todo"
version = "2.0.0"
protocol = "2"

[contributes]
panels = ["todo"]
mcp_servers = ["todo-tools"]
commands = ["todo.create", "todo.assign"]
event_subscriptions = ["task.*", "agent.run.*"]

[permissions]
workspace = "read-write"
network = ["api.example.com"]
process = false
secrets = ["example.token"]
```

### 4.2 Wasm 插件：Lapce、Extism、Wasmtime

[Lapce](https://github.com/lapce/lapce) 证明了编辑器插件可通过 WASI 隔离；[Extism](https://github.com/extism/extism) 在 Wasm 之上提供多语言 PDK、Rust Host SDK、Host functions、HTTP 控制和资源限制；[Wasmtime](https://github.com/bytecodealliance/wasmtime) 则是更底层、生产级的 Wasm/WASI runtime。

对 Dozer 的建议：

- V2 默认仍采用**进程插件**，因为 Agent、SSH、浏览器和语言工具经常需要子进程、网络、长任务与原生依赖；
- Wasm 作为“轻量逻辑插件”实验通道，适合 formatter、parser、规则、转换器和纯计算能力；
- 先用 Extism 做一项 spike，再决定是否直接面对 Wasmtime；
- 不把 Wasm 当成通用 UI 沙箱；`embedded_gpui` 一类方案仍偏实验性，且与 iced 的显示树不兼容；
- WASI 版本仍在演进，插件 ABI 必须由 Dozer 自己版本化，不能直接等同于 WASI 版本。

### 4.3 执行环境：OpenHands、Coder、Daytona

[OpenHands Runtime](https://github.com/OpenHands/docs/blob/main/openhands/usage/architecture/runtime.mdx) 使用 client-server 模型：后端启动容器内的 action executor，Agent 发送 Action，Runtime 返回 Observation。这个结构非常适合 Dozer 的执行事件模型：

```text
Task / Agent
    │ Action
    ▼
ExecutionEnvironment Provider
    │ Observation + Artifact + ResourceUsage
    ▼
Event Store / Audit / UI
```

[OpenHands Agent Server](https://github.com/OpenHands/docs/blob/main/sdk/arch/agent-server.mdx) 还说明 Agent SDK 可以放在 HTTP/WebSocket 服务之后，调用方无需嵌入具体运行时。这支持 Dozer 将 Agent provider 与 Host 解耦。

[Coder](https://github.com/coder/coder/blob/main/docs/ai-coder/agents/getting-started.md) 的价值在于控制面：模板、身份、策略和密钥可由控制面管理，Agent 需要工具和文件时再获得 workspace。Dozer 可据此设计：

- Host 保存 workspace 模板与授权策略；
- 凭据通过 capability/secret handle 下发，而不是复制进插件配置；
- workspace 与发起用户、任务、Agent run 建立明确归属；
- 预构建镜像或 warm pool 解决容器启动延迟。

[Daytona](https://github.com/daytonaio/daytona) 将 sandbox 拆分为 control、interface 和 compute plane，并提供快照、生命周期和自动停止/归档。它适合作为未来远程 `ExecutionEnvironment` Provider，而不是当前 Host 的 Rust 依赖。

最终抽象不应叫 `ContainerManager`，而应是：

```rust
trait ExecutionEnvironment {
    async fn prepare(&self, spec: EnvironmentSpec) -> Result<EnvironmentHandle>;
    async fn exec(&self, handle: &EnvironmentHandle, action: ExecAction)
        -> Result<ExecObservation>;
    async fn collect_artifacts(&self, handle: &EnvironmentHandle)
        -> Result<Vec<Artifact>>;
    async fn stop(&self, handle: &EnvironmentHandle) -> Result<()>;
}
```

容器只是 `DockerProvider`、`PodmanProvider`、`AppleContainerProvider` 中的一类实现；未来还可以有 `LocalProvider`、`SshProvider` 和 `RemoteSandboxProvider`。

### 4.4 Memory：Basic Memory 与 Mem0

[Basic Memory](https://github.com/basicmachines-co/basic-memory) 采用本地 Markdown、SQLite 索引、知识图谱和 MCP 工具，值得借鉴：

- 人类可读、可编辑的知识资产；
- 关系化检索与渐进式上下文发现；
- MCP 工具明确标注只读、破坏性、幂等和开放世界等语义；
- 记忆变更具有事件与历史语义。

但 Dozer 不应直接复制其“文件为事实源”的全部假设。Dozer 需要把 task、run、decision、artifact、acceptance 等强结构对象保存在 SQLite 事实源中，再按需投影成 Markdown，避免文件编辑破坏工作流一致性。

[Mem0](https://github.com/mem0ai/mem0) 的 MCP 工具形态——add/search/list/get/update/delete、entity 和 event——可作为 Agent Shared Memory API 的参考，但 Dozer 应坚持 local-first，并在 workspace、project、task、agent、user 五个 scope 上实施隔离。

### 4.5 会话、Token 与 Agent 可观测性：Phoenix、Langfuse、OpenTelemetry

[Arize Phoenix](https://github.com/Arize-ai/phoenix) 提供基于 OpenTelemetry/OpenInference 的 trace、dataset、experiment、evaluation 与 prompt 管理；[Langfuse](https://github.com/langfuse/langfuse) 提供 trace、evaluation、dataset、prompt management 和人工标注。两者证明 Dozer 的“对话审计”和“用量面板”应该建立在统一 trace/event schema 上，而不是分别解析日志。

建议内部建立以下层次：

```text
Trace: 一次 Goal / Task / Agent Run
  └─ Span: model call / tool call / command / browser step / human decision
       ├─ Event: stdout、approval、retry、checkpoint、error
       ├─ Usage: input/output/cache token、时间、金额、CPU/内存
       └─ Artifact: diff、截图、trace、报告、提交、PR
```

[OpenTelemetry Rust](https://github.com/open-telemetry/opentelemetry-rust) 可作为导出层候选，但其不同 signal 的稳定度不完全一致。V2 应先稳定 Dozer 自己的领域事件 schema，以 `tracing` 做 Rust 内部埋点；OTLP exporter 作为可选适配器，不让产品数据模型依赖某个观测后端。

### 4.6 浏览器测试与证据：Playwright MCP、Trace Viewer、Open Browser Use

[Playwright MCP](https://github.com/microsoft/playwright/blob/main/docs/src/getting-started-mcp.md) 使用可访问性快照让 Agent 操作页面，并支持持久或隔离的浏览器 profile；[Playwright Trace Viewer](https://github.com/microsoft/playwright/blob/main/docs/src/trace-viewer.md) 能展示 action、截图时间线、DOM snapshot、console 和 network。

这非常符合 Dozer Browser Test 模块所需的“可重放证据包”：

- 测试步骤和自然语言意图；
- 每步 DOM/可访问性快照；
- screenshot 或 screencast；
- console、network、下载和异常；
- 关联 commit、task、agent run 与环境；
- 最终 verdict 及人工接受记录。

[Open Browser Use](https://github.com/open-browser-use/open-browser-use) 还提供一个值得借鉴的安全边界：长驻 Node/Playwright runtime 通过 owner-only Unix domain socket 暴露 JSON-RPC，Host 对能力进行门控，并可切换 WebExtension/CDP 后端。Dozer 的浏览器自动化应采用类似 broker/sidecar，而不是把 Node 和 Playwright 嵌入 Rust Host。

选择顺序建议是：

1. Playwright/Playwright MCP 作为确定性测试和证据采集主路径；
2. Browser Use 或 Stagehand 类 AI 自动化作为高级 Provider；
3. Host 只消费统一的 `BrowserAction`、`BrowserObservation` 和 artifact schema。

### 4.7 小型决策模型：Laya

[Laya](https://github.com/NandhaKishorM/laya) 是 Python/PyTorch 实现的小型 typed decision model，适合在容器中作为可选 Provider 运行。其官方说明同时表明，未针对具体任务微调时的 zero-shot 表现可能接近随机，因此它不能直接成为工作流裁决者。

合理用途包括：

- Agent/模型/工具路由；
- 下一步动作或测试优先级排序；
- 风险分层、升级人工判断；
- 上下文压缩或 memory 召回候选排序；
- 在多 Agent 输出之间做低成本初筛。

必须保留的边界：

- Laya 输出是 recommendation，不是不可覆写的 truth；
- 记录输入摘要、模型版本、置信度、候选结果和最终人类/规则决策；
- 设置规则 fallback 和低置信度升级；
- 通过 MCP 或专用 decision protocol 调用，不把 Python/PyTorch 链接进 Host；
- 容器负责隔离依赖和资源，Host 负责授权、超时、审计和停止。

### 4.8 执行环境补充：dagger/container-use 比通用远程开发平台更贴题

[container-use](https://github.com/dagger/container-use)（Go，Apache-2.0，4k star，持续活跃）比 §4.3 的 OpenHands/Coder/Daytona 更接近 Dozer `ExecutionEnvironment` 的具体使用场景：它不是通用远程开发平台，而是专门解决"多个 coding agent 在同一台机器上安全并行工作"这一个问题——用 Dagger 容器化工具链，每个 agent 的环境同时绑定一个独立 git worktree/分支，人类可以随时 `container-use watch` 看到 agent 的完整命令历史，或直接进入某个 agent 的 sandbox 接管。它以 MCP server + CLI 形式暴露给 Claude Code、Cursor 等客户端，Host 侧不需要嵌入 Dagger 引擎本身。

对 Dozer §16.6 的直接印证：

- "Worktree 隔离代码分支，Container 隔离工具链/进程/网络"这个二元拆分，container-use 已经是一个可运行的实现，不只是设计假设；
- 它把"agent 的完整操作历史"作为一等产物（对应 Dozer 的 `CheckResult`/审计事件），而不只是最终 diff；
- 通过 MCP 暴露而不是自己做 GUI，验证了 Dozer"Host 消费 ExecutionEnvironment Provider 的标准接口，不用吃下具体容器引擎"这条路线是可行的。

建议列入 §7 P1 spike 的首选参照实现（比照读源码，而不是直接依赖 Go 二进制），并保留 OpenHands/Coder/Daytona 作为"更大规模远程 workspace 控制面"方向的补充参考。

### 4.9 插件权限与能力模型：Tauri ACL 与 Deno 的分层设计更完整

当前架构分析文档 §8.2/§9 的权限草案是一份扁平的 capability 字符串列表（`project.read`、`network`…）。两个已经在生产环境验证过的分层模型可以直接改进它：

**[Tauri 2.0 ACL](https://v2.tauri.app/security/capabilities/)** 把权限拆成三层，职责边界清楚：

```text
Capability   — 谁能获得这组权限（哪个窗口/webview/插件实例）
Permission   — 能调用哪些具体命令（如 fs:allow-read-file）
Scope        — 命令被限制在哪些资源上（如仅 $HOME/Documents 下）
```

Webview/插件默认被当作不可信方，每条 IPC 都必须显式声明 capability + permission + scope 三元组才放行，插件自己的权限标识符带命名空间前缀（`${plugin-name}:${permission-name}`），避免不同插件的同名权限互相冲突。

**[Deno 权限模型](https://docs.deno.com/runtime/fundamentals/security/)** 提供了另一个互补经验：默认零权限（无文件/网络/环境/子进程访问），权限按 `--allow-*` 显式授予且可加白名单范围（如 `--allow-net=api.example.com`），并且 `--deny-*` 优先级高于 `--allow-*`——这个"拒绝优先于允许"的顺序值得直接写进 Dozer Permission Service 的裁决逻辑，避免"先全局允许、再局部禁止"这种容易被绕过的实现。

建议把 Dozer 的 manifest `[permissions]` 从当前的布尔/字符串列表，改为 Tauri 式三层结构（capability 归属、permission 命名空间化、scope 显式限定资源），并在 Permission Service 的判定顺序上采纳 Deno 的"deny 优先"规则。

### 4.10 Wasm 插件补充：Zellij 的 WASI 插件协议、wasmCloud 的 capability provider

[Zellij](https://github.com/zellij-org/zellij)（Rust，MIT，35.5k star，高活跃）已经把 §4.2 讨论的"Wasm 插件"从设计变成了可读的生产代码：插件用 `wasmi` 解释器在隔离内存空间运行，Host 与插件之间用 Protocol Buffers 传递事件，插件按订阅模型只接收自己关心的事件类型，并有可配置的内存/栈资源上限。它的插件生命周期（加载→初始化→事件循环）和"插件不能直接访问 Host 或彼此内存，只能通过 WASI 受控访问系统资源"这两点，是比 Extism/Wasmtime 官方文档更具体的同语言（Rust）参考实现，建议 §7 P2 的 Wasm spike 直接对照读它的插件通信层源码，而不仅参考 Extism 的通用文档。

[wasmCloud](https://github.com/wasmCloud/wasmCloud)（Rust，Apache-2.0，CNCF 项目，2.4k star）验证了一个更进一步的模式："capability provider"——把网络、存储、消息队列等能力抽象成可替换的 provider，Wasm 组件通过标准接口声明依赖的能力，不直接链接具体实现。这与 Dozer《架构分析》§4.1 `PluginContext` 的能力接口设计思路一致，可以作为"插件声明依赖能力、Host 注入具体 Provider 实现"这一模式的命名参照，但 wasmCloud 本身面向分布式云原生场景，其 host/lattice/actor 运行时比 Dozer 单机 Host 需要的复杂得多，**只借鉴 capability provider 这个概念和命名，不引入其运行时**。

### 4.11 工作流持久执行参考：Temporal（仅借鉴设计，不作为依赖）

《架构分析》§16.2 给 Task/Execution/Delivery 设计的状态机，本质是一个需要在 daemon 崩溃、重启、Agent 掉线后仍能正确恢复的持久工作流。[Temporal](https://github.com/temporalio/sdk-rust)（Rust Core/SDK，MIT，持续活跃）是这一类问题最成熟的开源实现，它的几个机制值得直接对照设计 Dozer 的 Execution 持久化：

- Workflow 状态通过 event history 重放恢复，而不是定期快照，天然避免"崩溃时刻状态不一致"；
- Activity（对应 Dozer 的一次 agent 调用/verifier 执行）失败后按声明式重试策略自动重试，重试时长可以是数月级别，不需要业务代码自己写重试循环；
- Signal 机制支持外部事件（对应人工审批/取消）随时插入一个正在运行的 workflow，不需要 workflow 主动轮询。

**不建议引入 Temporal 本身**：它需要一个独立的 Temporal Service 集群，与 Dozer "本地优先、单机可用" 的定位冲突，纯粹是为了单机场景引入分布式系统的运维成本。正确用法是照抄它"event history 重放 + 声明式 Activity 重试 + signal 打断"这三个机制的思路，用 SQLite append-only 事件表在 `dozerd` 里自己实现一个轻量版本，这也与 §3.5.3 已经得出的"DAG 图只是视图，真正权威数据是 append-only run events"结论完全一致。

### 4.12 MCP Gateway 生态现状：仍然碎片化，没有可以直接采用的成熟实现

`docs/dozer-v2开源生态调研.md` §5.3 已建议 Dozer 自建 MCP Gateway 而非依赖外部项目；第二轮调研核实了具体现状，这个判断被进一步坐实。检索到的候选实现成熟度差异很大：

| 项目 | 语言/许可证/star（2026-09-24 核实） | 状态 |
|---|---|---|
| [IBM/mcp-context-forge](https://github.com/IBM/mcp-context-forge) | Python，Apache-2.0，4.5k star，活跃 | 相对最成熟：注册表 + 代理 + guardrail + 插件机制，面向 MCP/A2A/REST 统一网关 |
| [microsoft/mcp-gateway](https://github.com/microsoft/mcp-gateway) | C#，MIT，850 star | 面向 Kubernetes 的 session-aware 路由，偏企业集群部署，不适合本地单机场景 |
| [MikkoParkkola/mcp-gateway](https://github.com/MikkoParkkola/mcp-gateway) | Rust，**许可证未声明（NOASSERTION）**，仅 76 star | 单文件 Rust 二进制、工具面收窄到固定数量以省 token，思路有参考价值，但许可证状态不能直接引入生产 |
| [jonfairbanks/mcp-gateway](https://github.com/jonfairbanks/mcp-gateway) | Python，Apache-2.0，0 star | 刚起步，活跃度和验证程度都不足 |

没有一个项目是"本地优先、嵌入桌面 Host、面向单用户 workspace"的 MCP Gateway——现有实现要么面向企业多租户部署（IBM、Microsoft），要么是个人实验项目（Mikko、Jonfairbanks）。这验证了 Dozer 应该自己实现一个轻量 MCP Gateway（namespaced tool 注册 + 会话/项目身份注入 + 调用审计），而不是等待或依赖这个生态成熟；可以借鉴 IBM/mcp-context-forge 的 guardrail/registry 概念，但不作为依赖引入。[e2b-dev/awesome-mcp-gateways](https://github.com/e2b-dev/awesome-mcp-gateways) 可作为该生态的持续观察入口。

## 5. 可以作为依赖库的项目

### 5.1 近期建议

| 库 | 用途 | 建议 | 理由与注意事项 |
|---|---|---|---|
| [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk/tree/main/crates/rmcp) | MCP client/server、stdio、Streamable HTTP、schema | **继续采用并扩展** | Dozer 已依赖；是官方 Rust SDK。应封装在 `dozer-mcp-runtime`，避免领域层到处出现 SDK 类型 |
| [`notify`](https://github.com/notify-rs/notify) | 跨平台文件系统事件 | **继续采用** | Dozer 已依赖；用于 workspace、插件开发热重载和索引触发。需做 debounce、事件合并和 overflow 后全量校验 |
| [`testcontainers-rs`](https://github.com/testcontainers/testcontainers-rs) | 容器 Provider 的集成测试 | **作为 dev-dependency 评估** | 适合验证 Docker/Podman 下的生命周期、网络、挂载和失败恢复；不是生产容器管理器 |
| [`opentelemetry-rust`](https://github.com/open-telemetry/opentelemetry-rust) | trace/metric/log 导出 | **先做 exporter spike** | 生态标准明确，但部分 Rust signal 仍非 Stable；内部领域事件不应直接等同 OTel span |

### 5.2 条件采用

| 库 | 采用条件 | 当前建议 |
|---|---|---|
| [`jsonrpsee`](https://github.com/paritytech/jsonrpsee) | Plugin Protocol 确认采用 JSON-RPC，且需要 HTTP/WebSocket、subscription、middleware 和生成 API | 先以协议测试验证；本地 stdio/UDS 边界明确后再引入，避免为了一个简单协议引入过重网络栈 |
| [`bollard`](https://github.com/fussybeaver/bollard) | Docker-compatible Provider 需要原生 API、事件流、stats 和跨平台 socket | 可用于单个 Provider；`ExecutionEnvironment` 接口绝不能泄漏 Bollard 类型 |
| [`extism`](https://github.com/extism/extism) | 社区确实需要跨语言、轻量、受限的逻辑插件 | 先做 parser/rule 插件 spike，评估包体、启动、调试、Host functions 和 ABI 升级 |
| [`wasmtime`](https://github.com/bytecodealliance/wasmtime) | Extism 无法满足资源限制、component model 或深度 Host 控制 | 不在第一阶段直接引入；维护成本和编译成本与“微 Host”目标存在张力 |

### 5.3 不建议作为 Host 依赖

以下项目更适合作为外部 Provider、sidecar 或设计参考：

- Playwright、Browser Use：Node/Python 生态，走 MCP/JSON-RPC/stdio；
- Laya：Python/PyTorch，运行于容器或独立本地服务；
- Basic Memory、Mem0：通过 MCP 互操作，避免将其存储假设绑定到核心；
- Phoenix、Langfuse：作为可选导出/分析后端，不成为本地工作台启动依赖；
- Daytona、Coder、OpenHands：对接其 API 或借鉴 runtime，不嵌入其控制面；
- Apple container、Docker、Podman：通过 CLI/API adapter 使用，Dozer 不实现容器运行时。

## 6. 对 Host 重构的直接影响

调研结果强化了“Host 继续使用 iced，但必须缩小职责”的判断。Host 应收敛为以下稳定能力：

```text
dozer-host (iced)
├─ App Shell / Dock / Navigation
├─ Plugin Supervisor
├─ Capability & Permission Broker
├─ Command / Event / Contribution Registry
├─ Workspace & Worktree Registry
├─ Execution Environment Registry
├─ MCP Runtime & Service Directory
├─ Audit / Event / Artifact Store
├─ Secret Broker
└─ UI Surfaces
   ├─ Native declarative surface
   ├─ WebView surface
   └─ External-app handoff
```

以下能力应移出 Host 主二进制或至少移出 UI crate：

- Agent provider 适配；
- SSH、GitHub、浏览器、容器和远程 sandbox 实现；
- Todo、Code Health、Memory、Usage 等领域规则；
- Laya 和其他本地模型 runtime；
- 索引、静态分析、测试执行和报告生成；
- 第三方 SDK 的具体类型。

每个内建模块都应先通过与社区插件相同的协议运行。若内建插件拥有社区插件无法获得的隐式 Host API，插件体系最终会退化成“只能做小装饰”的二等生态。

## 7. 建议的工程验证项目

### P0：先证明边界，而不是先迁移全部功能

1. **Worktree Runtime spike**
   - Task 创建/绑定/暂停/归档 worktree；
   - 端口、环境变量、终端、Agent session 和 artifact 跟随 task；
   - 检测 dirty tree、冲突、主分支推进和孤儿 worktree；
   - 参考 Arbor、Pane、Ateam，但优先调用 Git CLI 保持行为一致。

2. **进程插件 vertical slice**
   - 将 Todo 作为首个外置插件；
   - manifest、启动握手、心跳、崩溃恢复、command、event、权限和升级；
   - Todo 暴露 MCP tools，使 Agent 能读取、创建、认领和完成任务；
   - 插件停止后 Host 仍可启动和浏览其他模块。

3. **统一事件与 Artifact schema**
   - 打通对话、Token、tool call、command、diff、test、decision 和 acceptance；
   - UI 可以从事件重建一次 Agent run；
   - 可选导出 OTLP，但 SQLite 仍是本地权威数据源。

### P1：验证外部运行时

4. **ExecutionEnvironment Provider**
   - `LocalProvider` 和一个 Docker-compatible Provider；
   - workspace mount、网络策略、secret injection、资源限制、超时和回收；
   - 用 `testcontainers-rs` 建立集成测试矩阵；
   - Laya 容器作为第一个非开发工具 workload。

5. **Browser Test sidecar**
   - Playwright sidecar + MCP/JSON-RPC；
   - 输出 trace、screenshot、console、network 和 verdict；
   - 证据关联 task/run/commit/environment；
   - 无浏览器插件时 Host 不携带 Node runtime。

6. **Shared Memory API**
   - workspace/project/task/agent/user scope；
   - fact、decision、preference、summary、artifact reference 分型；
   - provenance、validity、conflict、supersede 和删除审计；
   - 用 Basic Memory MCP 做互操作试验，但不改变 Dozer 的事实源。

### P2：生态与高级能力

7. **插件 SDK 与模板**
   - Rust SDK + 一个 TypeScript/Python SDK；
   - `dozer plugin dev/pack/validate/install`；
   - 本地开发热重载、协议兼容检查和权限预览；
   - registry 索引签名、校验和与撤回机制。

8. **Wasm logic plugin spike**
   - Extism 与 Wasmtime 二选一验证；
   - 只开放最小 Host functions；
   - 测量冷启动、内存、包体、编译时间和调试体验；
   - 不阻塞进程插件路线。

9. **Decision Provider**
   - Laya 与规则引擎实现同一接口；
   - shadow mode 记录建议但不自动执行；
   - 建立离线数据集评估准确率、成本、延迟和拒答阈值；
   - 达标后才允许进入低风险自动路由。

## 8. 选型闸门

任何新依赖或外部项目进入 Dozer 前，应回答：

1. 它解决的是核心抽象问题，还是单一 Provider 的实现问题？
2. 能否通过 MCP、JSON-RPC、HTTP、stdio 或 CLI 隔离？
3. 会不会增加 Host 的冷启动、编译时间、崩溃半径或许可证风险？
4. 数据能否导出，服务不可用时 Dozer 能否降级启动？
5. API/ABI 是否稳定，维护者和发布节奏是否可靠？
6. macOS、Linux、Windows 的行为差异由谁吸收？
7. 是否支持 capability-based 权限、取消、超时和审计？
8. 替换它时，领域模型和用户数据是否仍然成立？

如果一个项目只提供某种实现，就将它放在 adapter 后；只有定义 Dozer 自己核心语义的代码，才进入 core。

## 9. 建议持续跟踪的仓库清单

### 产品与工作流

- [brightwave-inc/tidebreak](https://github.com/brightwave-inc/tidebreak)
- [daintreehq/daintree](https://github.com/daintreehq/daintree)
- [penso/arbor](https://github.com/penso/arbor)
- [openchamber/openchamber](https://github.com/openchamber/openchamber)
- [limboo-ai/limboo](https://github.com/limboo-ai/limboo)
- [greenfield-inc/Pane](https://github.com/greenfield-inc/Pane)
- [clawnify/ateam](https://github.com/clawnify/ateam)
- [ai-creed/ai-14all](https://github.com/ai-creed/ai-14all)
- [l0ng-ai/tty7](https://github.com/l0ng-ai/tty7)
- [msavox/cleecode](https://github.com/msavox/cleecode)
- [hardbeat920/monocode](https://github.com/hardbeat920/monocode)
- [phodal/auto-dev](https://github.com/phodal/auto-dev)
- [peterfei/ifai](https://github.com/peterfei/ifai)
- [iAmCorey/kooky](https://github.com/iAmCorey/kooky)
- [stablyai/orca](https://github.com/stablyai/orca)
- [raine/workmux](https://github.com/raine/workmux)
- [pingdotgg/t3code](https://github.com/pingdotgg/t3code)
- [cfal/garcon](https://github.com/cfal/garcon)
- [asheshgoplani/agent-deck](https://github.com/asheshgoplani/agent-deck)
- [smtg-ai/claude-squad](https://github.com/smtg-ai/claude-squad)
- [1jehuang/jcode](https://github.com/1jehuang/jcode)
- [kafkazhang/seek_code](https://github.com/kafkazhang/seek_code)
- [aaif-goose/goose](https://github.com/aaif-goose/goose)
- [anomalyco/opencode](https://github.com/anomalyco/opencode)
- [BloopAI/vibe-kanban](https://github.com/BloopAI/vibe-kanban)
- [stravu/crystal](https://github.com/stravu/crystal)

### 插件与协议

- [zed-industries/zed](https://github.com/zed-industries/zed)
- [zed-industries/extensions](https://github.com/zed-industries/extensions)
- [lapce/lapce](https://github.com/lapce/lapce)
- [extism/extism](https://github.com/extism/extism)
- [bytecodealliance/wasmtime](https://github.com/bytecodealliance/wasmtime)
- [modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk)
- [zellij-org/zellij](https://github.com/zellij-org/zellij)
- [wasmCloud/wasmCloud](https://github.com/wasmCloud/wasmCloud)
- [IBM/mcp-context-forge](https://github.com/IBM/mcp-context-forge)
- [e2b-dev/awesome-mcp-gateways](https://github.com/e2b-dev/awesome-mcp-gateways)

### 执行、记忆、观测与浏览器

- [OpenHands/OpenHands](https://github.com/OpenHands/OpenHands)
- [coder/coder](https://github.com/coder/coder)
- [daytonaio/daytona](https://github.com/daytonaio/daytona)
- [dagger/container-use](https://github.com/dagger/container-use)
- [basicmachines-co/basic-memory](https://github.com/basicmachines-co/basic-memory)
- [mem0ai/mem0](https://github.com/mem0ai/mem0)
- [Arize-ai/phoenix](https://github.com/Arize-ai/phoenix)
- [langfuse/langfuse](https://github.com/langfuse/langfuse)
- [microsoft/playwright](https://github.com/microsoft/playwright)
- [open-browser-use/open-browser-use](https://github.com/open-browser-use/open-browser-use)
- [NandhaKishorM/laya](https://github.com/NandhaKishorM/laya)
- [temporalio/sdk-rust](https://github.com/temporalio/sdk-rust)

## 10. 最终判断

这轮调研没有发现需要推翻 Dozer V2 初步设计的证据，反而强化了几项关键决策：

- iced 继续作为 Host UI 是可行的，关键在于 Host 微内核化，而不是换成 Web UI；
- 插件应以进程隔离为默认，Wasm 是补充，WebView 是 UI surface，不是整个插件模型；
- MCP 是 Agent 能力网络，Plugin Protocol 是 Host 控制面，两者必须分层；
- worktree、事件审计和 artifact 是 V2 地基，不是普通面板功能；
- 容器、浏览器和小模型都应该是外部 Provider，Dozer 管理连接、权限、生命周期和证据；
- 真正值得开源社区参与的不是 15 万行应用内部，而是一套稳定协议、SDK、模板、开发工具和可组合的插件 App。

因此，下一步最有价值的工作不是继续横向增加面板，而是完成三个纵向切片：**Todo 进程插件、Task–Worktree 运行时、统一 Run/Event/Artifact 审计链**。这三项成立后，容器、Laya、Browser Test、Memory 和 Code Health 才能以一致方式接入，而不是再次长进 Host。

### 10.1 第二轮补充调研的结论（§3.7、§4.8-§4.12）

不改变上述最终判断，但把两处最薄弱的环节补上了具体参照：

- **ExecutionEnvironment**：dagger/container-use 证明"容器化 agent 沙箱 + worktree 绑定 + MCP 暴露、不嵌入 Host"这条路线已经有可运行实现，应作为 P1 spike 的首选参照，优先级高于 OpenHands/Coder/Daytona 这类更重的远程 workspace 平台；
- **插件权限模型**：现有 `[permissions]` 草案偏扁平，应改用 Tauri ACL 的 capability/permission/scope 三层结构，并采纳 Deno "deny 优先于 allow"的裁决顺序，这是一处应该在 P0 Plugin Protocol spike 之前就修正的设计缺口，成本很低；
- **MCP Gateway**：核实后确认这个生态仍然碎片化（企业级 K8s 网关 或 个人实验项目两极分化，没有本地单机场景的成熟实现），坐实了 Dozer 应自建轻量 Gateway、不等生态成熟的判断；
- **Vibe Kanban 的 sunsetting** 是本轮最重要的产品警示：一个功能覆盖面已经和 Dozer 目标高度重合、Rust 实现、28k star 的项目主动停止维护，说明"把 Kanban+worktree+diff审阅+PR 这套标配做全"不足以构成可持续产品，Dozer 的治理/验收层定位和插件生态才是必须守住的差异化，不能满足于对齐这类项目的功能列表。
