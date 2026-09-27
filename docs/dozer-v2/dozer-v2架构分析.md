# Dozer V2 架构分析：面向 Agent 时代的可组合 Workspace 平台

> 文档性质：产品与技术方向分析，不是已批准的实现规格。
> 分析日期：2026-09-24。
> 后续若进入实施，插件协议、权限模型、UI Surface 和迁移顺序仍需分别形成 spec 与 plan。

## 0. 定位裁决（2026-09-24 追加，已定稿）

本文档触发了一轮产品定位 brainstorming，结论已写入 `CLAUDE.md` 项目现实开头
（[[dozer-vision]] memory 有完整讨论记录），**定位句本身已定稿**：

> Dozer 是站在用户（甲方）一侧、agent 中立的 vibe coding 全流程治理与验收
> 层——面向更懂技术的委托人，覆盖从任务编排、多 agent 并行执行、上下文
> 共享，到过程审计、结果验收的完整闭环。

三点已确认、对下文阅读有约束力：

1. **治理/验收仍是核心身份，不因扩大范围而降级为众多平台能力之一。** 下文
   §2.1 提出的"Dozer 是面向 Agent 工作流的开源 Workspace 平台；Todo、Code
   Health、SSH 等是官方插件"这一表述，其"平台"框架容易读成治理只是插件之
   一——**已被否决**，须按新定位重新理解：Workspace/插件生态（如果做）是
   承载治理/验收能力的车身，不是把发动机换成众多平行部件之一。
2. **agent 中立性不变**，用户群体比 Claude Desktop/Hermes 一类消费级产品
   更懂技术，但仍是"委托人"、不亲自碰 AI 产出的代码本身。
3. **"具体落点"（是否/如何插件化、§16 Workflow Kernel 等数据模型改造、
   Execution Environment/容器抽象、Decision Service/Laya 集成……）尚未
   立项，用户明确说"后面再谈"**。本文档下文仍是未批准的初步分析，其中
   "插件化必要性证据不足、Workflow Kernel 必要性更高且独立于插件化成立"
   的可行性判断也只是候选参考，不是结论。

**2026-09-24 补充：** 已请另一 agent 完成第一轮开源生态调研
（`docs/dozer-v2/dozer-v2开源生态调研.md`），随后本会话又补了一轮，
新增验证了 Vibe Kanban（BloopAI，Rust/Apache-2.0，与 Dozer 同语言、
已 sunsetting 但架构成熟度高）、Crystal（stravu，已被 Nimbalyst 取代，
squash/rebase 会话模型仍值得参考）、dagger/container-use（容器化
agent 沙箱，比 OpenHands/Coder/Daytona 更贴近"隔离单个 agent 执行"
这个具体场景）、Zellij WASM 插件系统与 Tauri 2.0 ACL/Capability/Scope
三层权限模型（比当前草案的扁平 capability 字符串列表更完整）。详见
调研文档 §3.7、§4.8-§4.12；结论没有推翻既有判断，但补强了 ExecutionEnvironment
和插件权限模型这两块的可行性证据。

## 一句话结论

Dozer V2 不应只是把现有单体按目录拆成更多 crate，而应成为一个**稳定的 Workspace Host + 可独立开发和运行的能力插件 + Agent/MCP 语义编排层**：Host 长期可以继续使用 iced；第三方插件不依赖 iced/Rust ABI，通过版本化 Plugin Protocol 接入，以声明式 UI、WebView、外部窗口或无 UI 四种形态贡献能力。

## 1. 背景与问题

Dozer 已接近 15 万行 Rust 代码，当前 workspace 已经具备若干正确的基础分层：

- `dozer-core`：共享类型、路径和 UDS 协议。
- `dozerd`：PTY、会话、项目数据和各类存储。
- `dozer-client`：GUI/MCP 共用客户端。
- `dozer-mcp`：Agent 面向 Dozer 能力的工具入口。
- `dozer-app`：iced Host、Workspace UI 和现有 `extensions::*`。
- `byteui`：Dozer 原生 UI 组件。
- `dozer-codehealth`：已经独立出来的领域核心。

Todo、Files、Usage、Code Health、Database、SSH 等功能也已经逐渐形成自己的 `Message / State / update / view`。这说明 Dozer 已经完成了“进程内 extension”的第一阶段，但还没有形成真正面向仓库外开发者的插件系统。

当前主要问题不是文件放得不够分散，而是以下边界仍然重叠：

1. `dozer-app` 静态依赖全部功能及其重型依赖，开发单一功能时仍会承受大范围编译和链接成本。
2. extension 会直接接触 `App`、`Workspace`、顶层 `Message`、iced 类型、Tokio runtime、窗口几何等宿主实现细节。
3. 领域能力持续进入 `dozer-core`/`dozerd` 的中央协议，核心会随每个插件一起增长。
4. 功能只能随完整 Dozer 一起构建、升级和崩溃，尚未形成独立构建与故障边界。
5. 社区开发者无法只依赖稳定 SDK，在主仓库之外完成插件开发、调试和分发。

## 2. 产品判断

### 2.1 插件化首先不是销售拆分

V2 的首要目标不是把 Todo App、Code Health App、SSH App 分别销售，而是让 Dozer 成为社区可以持续扩展的开放底座。独立 App 形态的主要价值是：

- 插件可以独立编译和调试。
- 插件作者不必启动或重新链接完整 Dozer。
- CI 可以在最小 Host 中独立验收插件。
- 插件可以脱离完整 Workbench 独立运行。
- 社区可以组合一组插件形成自己的发行版。
- 同一份插件实现不需要在独立 App 与 Dozer 内重复。

因此更准确的产品定义是：

> Dozer 是面向 Agent 工作流的开源 Workspace 平台；Todo、Code Health、SSH 等是官方插件，也是社区插件的参考实现。

### 2.2 AI 时代的 Workspace 定制

传统应用的扩展通常局限于主题、命令、语法或固定贡献点。AI 时代的 Workspace 定制会进一步包含：

- 用户选择哪些领域能力进入当前 Workspace。
- 用户决定 Agent 可以读取、调用和修改什么。
- 插件向 Agent 暴露领域工具、资源和上下文。
- Agent 在运行时组合事先互不认识的插件。
- 插件同时扩展 UI、存储、后台任务、自动化和工作流。
- 用户可以让 Agent 基于 SDK 生成或修改自己的插件。

市场上常见的两个极端是“稳定但封闭的应用”和“开放但缺少权限、审计、生命周期与可靠状态的脚本/Agent 环境”。Dozer 的机会在两者之间：

> 稳定的人类 Workspace + 开放的 Agent 能力空间。

### 2.3 相对 VS Code 的差异

Dozer 不需要在 API 数量、生态规模或兼容历史上复制 VS Code。其潜在差异是：

1. 插件不限于编辑器领域，可以定义完整工作领域。
2. 插件可通过 Agent/MCP 动态组合，而不必彼此硬编码依赖。
3. 插件可以跨语言、跨进程运行。
4. 同一个插件既能嵌入 Dozer，也能独立成为 App 或 MCP capability provider。

这是一种比“允许插件修改更多 UI”更有价值的开放性：

> VS Code 是可扩展的编辑器；Dozer 可以成为可组合的 Agent Workspace Runtime。

开放性不能等同于插件可任意修改宿主。稳定贡献点、权限隔离和兼容契约仍是生态成立的前提。

### 2.4 现有功能不是功能集合，而是 Vibe Coding 治理闭环

Dozer 当前已经在 Vibe Coding 的几个热门问题上做了初步实践。这些功能不应被理解为彼此独立、可以随意替换的普通工具面板，而应被理解为同一条 AI 开发治理链路上的不同观测和控制面：

| Vibe Coding 问题 | Dozer 当前能力 | 当前成熟度/缺口 | V2 中的角色 |
|------------------|----------------|-----------------|-------------|
| 多个 Agent 如何并行工作 | Agent 面板管理多个 Agent 会话 | 已有多会话管理，尚欠缺以 Git worktree 为核心的代码隔离与合并闭环 | Agent Runtime/Worktree 官方插件与 Host 会话能力 |
| 多个 Agent 如何共享长期上下文 | 项目面板的 Agent 共享 Memory | 共享存储、历史和 MCP 访问正在落地 | Memory 官方插件/项目级 Context Service |
| 人类如何把目标拆给 Agent | Todo 面板通过 Task 指派和编排 Agent | 已具备任务状态、分类、派发和 Agent 处理入口 | Human-to-Agent Orchestration 官方插件 |
| 如何知道 Agent 实际做了什么 | 对话面板保存并查看会话过程 | 已具备会话聚合、摘要和审计基础 | Conversation Audit 官方插件 |
| AI 开发投入了多少成本 | Usage 面板统计 Token 使用 | 已有用量聚合和展示 | Cost/Usage Audit 官方插件 |
| 快速生成的代码质量如何 | Code Health 面板分析代码质量与架构演进 | 已有独立分析核心和报告存储 | Quality Governance 官方插件，也是首个进程外试点候选 |
| 如何验证最终用户界面真的可用 | Browser 模块承载页面查看，后续增强浏览器自动化测试 | 当前以浏览为主，自动化测试、证据采集和验收闭环待增强 | Browser Test/Acceptance 官方插件 |

这些能力可以组成一条完整链路：

```text
人类定义目标
    │
    ▼
Todo/Task 编排
    │
    ▼
多个 Agent 并行执行 ── Worktree 隔离
    │                    │
    ├── 读取/更新共享 Memory
    │
    ├── Conversation 审计执行过程
    ├── Usage 审计 Token 投入
    ├── Code Health 审计代码质量
    └── Browser Automation 验证用户可见结果
                         │
                         ▼
                    人类验收/继续迭代
```

因此 Dozer 的产品内核可以进一步定义为：

> Dozer 是 Vibe Coding 的控制面与治理层：它不替代 Agent 编写代码，而是帮助人类组织 Agent、共享上下文、观察过程、审计投入、判断质量并验收结果。

这一定位也解释了为什么官方插件仍然重要。插件化不是要把 Dozer 变成一个没有观点的空壳；上述能力应构成官方维护的 **Vibe Coding Reference Suite**：

- 为普通用户提供开箱即用的完整闭环。
- 为社区展示不同类型插件如何接入 Host、数据、事件和 MCP。
- 作为 Plugin API 的真实消费者和兼容性测试集。
- 允许社区替换其中一环，例如替换 Memory、质量扫描器或浏览器测试引擎。
- 允许社区增加新的治理环节，例如安全审计、发布、线上观测和需求追踪。

### 2.5 Host、官方套件与社区插件的关系

产品应避免两个极端：

1. **Host 过厚**：所有 Vibe Coding 领域逻辑都固化在 Host，社区只能做边缘装饰。
2. **Host 过空**：Dozer 只有插件加载器，没有经过验证的默认工作流和产品观点。

建议形成三层关系：

```text
Dozer Host
  稳定运行时、Workspace、权限、布局、Agent/MCP 与插件生命周期

Dozer Official Vibe Coding Suite
  Agent/Worktree、Todo、Memory、Conversation、Usage、Code Health、Browser Test

Community Plugins
  替换、增强或新增任意治理环节
```

其中 Host 提供机制，官方套件提供方法论，社区插件提供多样性。用户既可以直接使用官方闭环，也可以按自己的开发方式重组 Workspace。

## 3. V2 目标与非目标

### 3.1 目标

1. 修改一个插件时，只编译和重启该插件，Host 尽量保持运行。
2. 插件崩溃不能带崩 Host 或其他插件。
3. 仓库外开发者只依赖公开 SDK 即可开发插件。
4. 插件能注册 UI、命令、后台任务、事件订阅和 MCP tools。
5. 插件不依赖 `dozer-app::App`、`Workspace`、iced 类型或私有数据库结构。
6. 插件与 Host 的协议可版本化、可测试、可审计。
7. 同一插件可在完整 Dozer、Plugin Dev Host 和独立 App 中复用。
8. Agent 可以发现并组合安装后的插件能力。
9. 核心 Host 长期保持小、稳定，新增领域功能通常不再修改 Host。
10. Manifest 与 Contribution Registry 从 Phase 1 起就支持声明式 UI 文案翻译，Host 与官方插件至少覆盖英语、简体中文（§19）。

### 3.2 非目标

1. V2 第一阶段不建立插件市场、收费系统或远程商店。
2. 不以 Rust `.dylib` 作为第三方插件的主要交付格式。
3. 不要求第一阶段支持任意远程不可信代码。
4. 不把所有 Host 内部调用都改成 MCP。
5. 不为插件化立即重写全部现有 UI。
6. 不要求所有官方功能一次性迁移。
7. 不将 iced 类型暴露为长期稳定的社区 ABI。

## 4. 总体架构

```text
┌──────────────────────────────────────────────────────────────┐
│                       Dozer Host                             │
│  window / workspace / layout / theme / permissions / audit  │
│  plugin lifecycle / event routing / MCP aggregation          │
└──────────────┬───────────────────────┬───────────────────────┘
               │ Plugin Protocol       │ Platform Services
               │                       │
        ┌──────┴──────┐        ┌───────┴────────────────────┐
        │Plugin Runtime│        │ project / session / store  │
        │install/start │        │ secret / job / git / file │
        │stop/upgrade  │        │ notification / permission │
        └──────┬──────┘        └────────────────────────────┘
               │
     ┌─────────┼───────────────┐
     │         │               │
  Todo       Code Health      SSH            Community Plugins
     │         │               │                    │
     └─────────┴──── MCP tools ┴────────────────────┘
                         │
                       Agent
```

### 4.1 Micro Host

Host 只拥有跨插件且必须统一的能力：

- 应用窗口和 Workspace 生命周期。
- Rail、Tab、布局和面板容器。
- 主题、菜单、快捷键和全局通知。
- 插件安装、启停、升级和崩溃恢复。
- 权限申请、用户确认和审计。
- Plugin Surface 的创建、隐藏、销毁与恢复。
- Plugin Protocol 路由。
- MCP tool 聚合和命名空间管理。

Host 不应理解 `TodoInfo`、`SshHost` 或 `CodeHealthReport` 等具体领域类型。

### 4.2 Platform Services

平台服务提供稳定、可授权、跨插件复用的基础能力：

- Project Service。
- Agent/Session Service。
- Plugin Storage。
- Secret Store。
- Background Job/Progress/Cancel。
- File/Git Service。
- Event Bus。
- Notification Service。
- Permission Service。
- MCP Gateway。

当前 `dozerd` 是这层的雏型，但长期应避免继续吸收全部领域协议。领域数据应由插件自己的存储或 namespaced service 持有，核心 daemon 只提供通用托管能力。

### 4.3 Plugin Runtime

Plugin Runtime 负责：

- 读取和校验 manifest。
- 检查 Plugin API 兼容范围。
- 展示并持久化权限授权。
- 启动插件进程并完成握手。
- 心跳、超时和崩溃重启。
- Surface 与插件进程的归属绑定。
- 事件订阅和背压。
- MCP tool 注册、撤销与冲突检查。
- 升级前后的状态迁移。

### 4.4 Plugin

一个插件可以贡献任意组合：

- Panel/Page。
- Command。
- MCP tools/resources/prompts。
- Background jobs。
- Event subscriptions。
- 数据源和索引。
- 独立窗口。
- 无 UI 的自动化能力。

插件不必同时拥有前端和后端。纯 MCP、纯后台任务或纯 UI 插件都应是一等公民。

## 5. 三种“插件化”必须区分

### 5.1 代码模块化

把 `extensions/todo` 拆为独立 crate，可以改善代码归属、依赖方向和单元测试，但如果 `dozer-app` 继续静态依赖所有插件，完整应用仍会重新链接。

### 5.2 编译插件化

每个插件可以作为独立 target/App 构建，修改 Todo 时无需编译 SSH、Database 等无关依赖。这能显著改善插件自身的开发循环。

### 5.3 运行时插件化

插件以独立进程安装、启动和升级，Host 通过协议连接。只有这一层能真正提供：

- Host 不重新链接。
- 插件热重启。
- 崩溃隔离。
- 多语言实现（插件可用不同编程语言开发，与 §19 讨论的自然语言 UI 多语言是两回事）。
- 独立版本和依赖树。
- 安装后启用/禁用。

因此 V2 的长期目标应是进程外插件；拆 crate 是迁移手段，不是最终结果。

## 6. 通信模型：可靠通道与 Agent 通道并存

“插件之间通过 Agent 使用 MCP 通信”是 Dozer 的差异化能力，但不能成为所有内部通信的基础设施。

| 场景 | 机制 | 示例 |
|------|------|------|
| 插件调用平台 | Typed Plugin RPC | 读取当前项目、申请 Secret |
| 插件状态通知 | Event Bus | Todo 更新后刷新 badge |
| 高频/事务操作 | Typed service | 写存储、启动后台任务 |
| Agent 调用插件 | MCP | Agent 调用 `todo.add` |
| Agent 跨插件编排 | MCP | 扫描 Code Health 后创建 Todo |

### 6.1 可靠通道

UI 刷新、状态同步、存储写入、生命周期和事务要求确定性，不得绕经 Agent：

```text
Plugin UI → Plugin RPC → Plugin/Platform Service → Store/Event
```

### 6.2 Agent/MCP 语义通道

Agent 适合完成开放式目标编排：

```text
用户：“扫描代码，把高风险问题生成任务”
                   │
                 Agent
           ┌───────┴────────┐
     code_health.scan    todo.add
```

Code Health 与 Todo 不需要硬编码认识彼此，只需暴露可发现、语义明确、权限清晰的工具。

### 6.3 核心原则

> 插件之间默认不直接依赖；确定性协作通过平台事件与服务，开放式语义协作通过 Agent + MCP。

MCP 是面向 Agent 的能力协议，不是 Dozer 的内部事件总线。

### 6.4 MCP 聚合

当前集中式 `dozer-mcp` 可以演化为 Gateway。每个插件注册自己的 namespaced tools：

- `todo.list`
- `todo.add`
- `code_health.scan`
- `ssh.exec`

Gateway 统一完成：

- 会话与项目身份注入。
- 用户授权。
- 工具发现。
- 调用审计。
- 超时与取消。
- 插件离线错误。
- 同名工具冲突处理。

## 7. UI 架构裁决

### 7.1 iced 长期作为 Host UI 可行

iced 适合继续承担：

- 主窗口和 Workspace Shell。
- Rail、Tab、布局与拖拽。
- 主题和 byteui。
- 终端及需要直接控制渲染/事件的组件。
- 权限确认、Secret 提示等可信 UI。
- Plugin Surface 容器。

它的优势是全 Rust 单一状态模型、编译期类型检查、统一事件管线和对复杂桌面交互的直接控制。现有 Dozer 也已经积累了大量 iced/byteui、窗口、overlay、终端和 WebView 管理资产，整体迁移不能自动解决插件协议问题，反而会产生大规模重写。

### 7.2 iced 不作为第三方插件 ABI

第三方协议中禁止出现：

```text
iced::Element
iced::Message
iced::Theme
winit::Window
dozer_app::App
dozer_app::Workspace
```

原因包括：

- Rust ABI 不稳定。
- `Element` 与 Message、生命周期、Renderer 和 Theme 强绑定。
- 插件必须锁定相同 iced 版本。
- 插件难以独立编译和热重启。
- 动态加载原生代码没有安全隔离。

iced 是 Host 实现细节，而不是 Plugin SDK。

### 7.3 四种 Plugin Surface

#### A. Declarative UI

插件提供可序列化 UI Document，由 Host 使用 byteui/iced 渲染。适合列表、表格、表单、属性、设置和简单报告。

优点：

- 原生一致性最好。
- 权限和事件边界清楚。
- 跨语言。
- 不需要插件自带前端 runtime。

限制：只能使用 Host 已提供的组件与交互模型。

#### B. WebView UI

插件打包本地 Web 应用，由 Host WebView 加载。插件作者可选择：

- React/Vue/Svelte/Solid。
- Vanilla HTML/CSS/JS。
- Leptos/Yew/Dioxus Web。
- WebAssembly 前端。

Host 只认识入口、Surface 生命周期和 JS Bridge，不认识具体前端框架。复杂可视化、数据库工具和社区创新型 UI 默认走这条路径。

#### C. External UI

插件使用 iced、egui、Slint、Qt、GTK 等任意原生框架时，以独立窗口或独立 App 运行，不尝试把另一套原生 widget tree 嵌入 iced 主窗口。

#### D. None

插件不提供 UI，只注册 MCP、后台任务、命令、索引或自动化能力。

### 7.4 Manifest 示例

```toml
id = "com.example.code-health"
name = "Code Health"
version = "1.2.0"
plugin_api = ">=1.0,<2.0"
entry = "bin/code-health-plugin"

[ui]
type = "webview"
entry = "dist/index.html"

[contributes]
panels = ["code-health"]
commands = ["code-health.scan"]
mcp_tools = true

[i18n]
default_locale = "en"
supported_locales = ["en", "zh-CN"]
strings_dir = "locales/"

[permissions]
project_read = true
project_write = false
network = false
process_spawn = false
terminal = false
secrets = []
```

`[i18n]` 是可选字段（§19）：`strings_dir` 下每个 locale 一个扁平 key-value 资源文件（如
`locales/en.json`、`locales/zh-CN.json`），供 `contributes` 里的 Panel/Command 标题在
`title_key` 查不到或插件完全不提供 `[i18n]` 时退回字面量 `title_fallback`，不阻塞插件正常
显示。

### 7.5 WebView 嵌入的已知难点

Dozer 已经有 wry/Preview 经验，但正式 Plugin Surface 仍需统一解决：

- iced 与 WebView 的坐标、缩放和尺寸同步。
- 焦点、快捷键和输入法归属。
- WebView 永远位于 GPU 内容之上的 z-order 限制。
- Tab 切换后的 suspend/resume 和资源回收。
- 拖拽经过 WebView 边界。
- Tooltip、菜单、弹窗由谁渲染。
- 主题和字体同步。
- 多 WebView 内存上限与池化。
- 外链、导航、下载和剪贴板权限。
- 本地资源 CSP 与网络访问白名单。

这些能力必须由 Host 的 `PluginSurface` 统一实现，不能由每个插件自行处理。

#### 7.5.1 WaveTerm Web Block 的可借鉴边界（2026-09-27 追加）

WaveTerm 的网页块采用 Electron `<webview>`（独立 Chromium guest process），React 层维护
URL、标题、加载状态、前进/后退、缩放、查找、User-Agent、媒体与焦点，preload/环境适配层
承接宿主能力，`partition` 隔离 Cookie 和站点存储。它证明了“网页是 Workspace 中与终端、
文件并列的一等 Block”在产品上成立，也提供了较完整的浏览器状态机参照。

Dozer 只借鉴其**产品形态、状态边界与事件覆盖面**，不迁移 Electron，也不随应用打包 Chromium：

- 保留 iced Host + wry 系统 WebView，避免显著增加安装体积、常驻内存和 renderer 进程成本。
- 浏览器领域状态不直接依赖 wry 句柄；通过 `BrowserHost`/命令接口使用导航、刷新、历史、
  截图、存储清理、外部打开等宿主能力，便于以后接 WebKit、WebView2、CDP 或 Playwright provider。
- 页面能力通过受控 Bridge/事件信封回传，网页默认不获得文件、进程、Secret 或任意 Host IPC。
- WebView 由 Host 创建、池化、隐藏、挂起、恢复和销毁；插件只声明期望 Surface 与会话作用域。
- 一个 Tab 对应稳定会话；普通切换不重新导航，资源压力下才按预算挂起或淘汰，并显式恢复状态。

Electron `<webview>` 的焦点/事件路由复杂度和官方长期演进风险也是反例：Dozer 不把 WebView
作为整个插件模型，只把它作为受限 UI Surface；Browser 自动化则走独立 provider/sidecar，
不能把 Node/Playwright 或一套 Chromium runtime 链接进 Rust Host。

### 7.6 是否迁移 Tauri

当前不建议为了插件化立即将 Host 从 iced 迁移到 Tauri。Tauri 能提供成熟的 Web 前端生态、IPC 和 Capability 思路，但不会替 Dozer 解决 manifest、插件进程、MCP 聚合、状态迁移和 Workspace 语义。

只有出现以下信号时，才应重新评估整体 Web/Tauri Shell：

1. 超过一半的主要 UI 已经是 WebView 插件。
2. iced Host 只剩很薄的窗口和布局容器。
3. WebView/iced 的焦点、层级和几何维护成本长期高于原生收益。
4. 社区明确偏好统一 Web 技术栈。
5. byteui 与 Web 组件库出现大量重复建设。

当前推荐形态是：

```text
iced Host
├── Native Surface：核心内建能力
├── Declarative Surface：简单社区插件
├── WebView Surface：复杂社区插件
└── External Surface：任意原生 UI/独立 App
```

### 7.7 WebView Surface 前端技术选型的两条独立探索（2026-09-25 追加）

在插件化落点尚未立项之前，本会话先针对"把 Todo/对话/用量这类面板迁成 WebUI 该选什么前端技术"做了两项轻量探索，结论记录于此供后续 §7.3 WebView Surface 与 §10 插件 SDK 设计时参考；**均为非正式спайк/调研，不是已批准的技术选型**。

#### 7.7.1 Preact + esbuild 离线打包 spike：结论是可行

沿用 `crates/dozer-app/web/editor`、`crates/dozer-app/web/json-editor` 已验证的"esbuild 离线 iife bundle、无 CDN、无运行时 Node、严格 CSP"模式，加一层 Preact（`jsx: 'automatic', jsxImportSource: 'preact'`，esbuild 原生支持，不需要 Vite/Babel），做了一个 Todo 列表 mock（增/切换完成/删除）验证可行性：

- 构建：11ms，产物 `todo.js` 压缩后 **13.9 KiB**（未 gzip），比现有两个 WebView 宿主小一个数量级。
- CSP：沿用 `default-src 'none'; script-src 'self'` 严格策略跑通，控制台全程零报错、零 CSP 违规。
- 交互：`useState` 驱动的增/切换/删除三条状态路径全部验证通过。
- **发现的真实坑**：mock 里图省事用裸 `<span onClick>` 做勾选框，Chrome 无障碍树完全看不到它（`find` 工具报告"未找到 checkbox 控件"）——真要做进正式面板必须用 `<input type="checkbox">` 或补 `role="checkbox"`/`aria-checked`/键盘操作，不能照抄这个 spike 的写法。

结论：Preact + esbuild 在"离线、无 CDN、CSP 严格、体积敏感"这几条本仓库硬约束下没有障碍，可作为 §7.3 WebView Surface 的默认前端选型候选；spike 代码是一次性的，未进入 `crates/dozer-app/web/`。

#### 7.7.2 组件库调研：Beautiful UI（beautifului.dev）——只做设计参考，不作为依赖

调研了 Turbo Design Studio 的 [Beautiful UI](https://www.beautifului.dev/)，一套面向"AI 原生界面"的组件目录（21 类：Loading/Thinking/Streaming Text、Approval Card、Tool Chips、Task Rows、Chat、Prompt Bar、Recommendation Card、Context Cards、Diff Table、Records Table、Filter Table、Sidebar Nav、Search、Flowchart、Insight Cards、Code Block、Fine-tune Card、Selection Actions、Agent Screen 等）。

- **技术栈**：Next.js + React + Tailwind CSS（页面实测 `className` 含 Tailwind 工具类），不是 shadcn，但同属"抄源码进项目"路数。
- **可获取性**：无 GitHub 仓库、无 npm 包、`/docs` 返回 404、无定价页，落地只有邮件订阅"Notify me"——目前是候补名单/获客页性质，不是可直接安装的成品库。
- **信息架构价值**：其组件分类与 §17-§18 计划新增的面板高度对应——Approval Card / Recommendation Card ≈ Decision Inbox（§17.3）、Diff Table ≈ Delivery/Acceptance 证据展示（§17.2）、Tool Chips / Task Rows / Thinking ≈ Runs/Execution Audit（§18.1 / §18.4）、Records Table / Filter Table ≈ Data Workspace 或 Files 面板（§18.11 / §18.8）、Context Cards ≈ Context Service 检索展示（§16.4）。
- **技术上不兼容当前方向**：§7.7.1 验证可行的路线是 Preact + 纯 CSS + esbuild 离线打包，Beautiful UI 是 React + Tailwind，直接复用代码意味着要么新引入 Tailwind 构建链（当前两个 web/ bundle 均未使用 Tailwind），要么把组件逻辑手工翻译成 Preact，成本不小；且现阶段代码本身不可获取，无法评估实现质量。

结论：**不纳入技术选型，仅作为 V2 新面板（Decision Inbox / Delivery / Runs / Data Workspace）的 UI 交互模式参考清单**，留到真正做这些面板视觉设计时对照，不等它开源/发包再考虑复用实现。

## 8. Plugin API 与协议原则

### 8.1 稳定 API，不暴露宿主对象

插件只能通过明确的 capability API 获取能力，不能拿到整个 Host、数据库连接或 Workspace 内部对象。

概念接口：

```rust
trait PluginContext {
    fn projects(&self) -> &dyn ProjectService;
    fn storage(&self) -> &dyn PluginStorage;
    fn secrets(&self) -> &dyn SecretStore;
    fn events(&self) -> &dyn EventBus;
    fn jobs(&self) -> &dyn JobService;
    fn agents(&self) -> &dyn AgentService;
}
```

这只是能力模型示意；进程外协议实际应使用可序列化请求/响应，不直接跨边界传 trait object。

### 8.2 协议必须具备

- 显式 protocol version 与 capability negotiation。
- 请求 ID、超时、取消和幂等标识。
- 结构化错误码，不依赖自然语言判断。
- 事件序号和断线重连策略。
- 大对象使用资源句柄/流，不把所有内容塞进单个 JSON。
- project/workspace/session/plugin identity 由 Host 注入和校验。
- 所有副作用调用经过权限检查和审计。
- 未知字段可忽略，新增字段有默认语义。

### 8.3 传输选择

本地第一版可以使用 UDS 上的 JSON-RPC 或自定义 framed protocol，优先保证可调试与跨语言。是否引入 Protobuf/Cap'n Proto 等二进制协议，应由真实性能数据决定，不应在分析阶段提前锁死。

MCP 不替代 Plugin Protocol：MCP 面向 Agent；Plugin Protocol 面向 Host 与插件的可靠运行。

## 9. 权限与安全模型

插件开放度越高，越不能默认继承 Host 权限。至少需要以下权限域：

- `project.read`
- `project.write`
- `network`
- `process.spawn`
- `terminal`
- `git.read` / `git.write`
- `secret.read:<scope>`
- `agent.observe`
- `agent.invoke`
- `mcp.register_read`
- `mcp.register_write`
- `ui.webview`
- `ui.external_window`

权限应满足：

1. 安装时展示声明，首次使用高风险能力时再次确认。
2. 按 Workspace 授权，而非一装全局永久放行。
3. MCP 的副作用工具不能因为是 Agent 调用就绕过授权。
4. WebView 默认无本地能力，通过受控 Bridge 调用。
5. 插件只能访问自己的存储和 Secret namespace。
6. Host 保留调用日志、调用方 Agent、项目和结果状态。

进程隔离本身不是完整沙箱。第一阶段应明确插件是“用户安装的本地可信程序”；真正的不可信代码沙箱需要另行设计，不应虚假承诺。

## 10. 插件开发体验

独立 App 形态应落为官方 Plugin Dev Host，而不是让作者复制 Dozer：

```bash
dozer plugin new
dozer plugin dev ./my-plugin
dozer plugin test ./my-plugin
dozer plugin package ./my-plugin
dozer plugin inspect ./my-plugin
```

Dev Host 至少应提供：

- 最小 iced Shell 与 Plugin Surface。
- 测试项目和模拟 Workspace context。
- 本地 daemon/platform services。
- MCP Inspector。
- RPC/Event 日志。
- 权限授权与拒绝模拟。
- 插件热重启。
- 主题、缩放和多尺寸预览。
- 崩溃、超时和断线测试。

理想开发循环：

```text
修改插件
→ 只编译/重启插件进程
→ Host 保持运行
→ 自动重新握手
→ 恢复 Surface 和必要状态
```

后续可支持组合式发行：

```bash
dozer app build \
  --name "My Dev Console" \
  --plugin todo \
  --plugin code-health \
  --plugin github.com/example/foo
```

## 11. 官方插件迁移策略

### 11.1 第一阶段：测量而不是凭代码行数判断

建立基线：

- clean build 总时间。
- 修改 Todo 后的增量编译和链接时间。
- 修改核心协议后的失效范围。
- 各重型依赖的构建占比。
- release 二进制和各插件产物体积。
- 运行时 WebView/插件进程内存。

代码行数说明维护压力，但编译问题真正由依赖图、泛型单态化、build script 和链接边界决定。

### 11.2 第一个试点：Code Health

推荐首先迁移 Code Health：

- 核心分析已经是独立 crate。
- 适合验证后台 Job、进度、取消和报告 artifact。
- UI 可用于验证 WebView Surface。
- MCP 可验证长任务注册与结果读取。
- 与终端/PTY 的深耦合较少。

通过标准：修改并重启 Code Health 插件不重新构建 Host，Host 可恢复面板，Agent 可调用其工具。

### 11.3 第二个试点：Todo

Todo 用于验证：

- 完整 CRUD。
- 实时事件和 badge 更新。
- 声明式 UI 或简单 Web UI。
- MCP 写工具。
- 项目级数据隔离。
- 与其他未知插件的 Agent 编排。

### 11.4 第三个试点：SSH

SSH 最后迁移，用来压力测试：

- Secret。
- 高风险权限。
- PTY 和长连接。
- SFTP。
- Host Tab/Terminal 集成。
- 插件崩溃后的会话处理。

如果 SSH 也能服从同一套协议，平台边界才算基本成立。

### 11.5 不批量搬 crate

在一个真实进程外插件跑通以前，不应把所有 `extensions::*` 机械迁移成 crate。否则可能得到更多目录和 package，却仍然共享同一个 App、Message、runtime 和链接单元。

## 12. 分阶段路线

### Phase 0：基线与边界审计

- 编译/链接性能基线。
- 现有 extension 对 Host 私有类型的依赖矩阵。
- `dozerd` 领域协议清单。
- WebView Surface 现有能力与缺口。

### Phase 1：最小 Plugin Protocol

- Manifest。
- 启停与握手。
- 版本协商。
- Command/MCP 注册。
- 基础权限。
- Event Bus。
- Dev Host。

暂不做市场、在线安装、强沙箱。

### Phase 2：Code Health 纵向切片

- 独立插件进程。
- WebView UI。
- Job/Progress/Cancel。
- MCP 注册。
- Host 热重连。

### Phase 3：Todo 纵向切片

- CRUD 与存储。
- Declarative UI 评估。
- 实时事件。
- 与 Code Health 的 Agent/MCP 组合验收。

### Phase 4：SDK 与仓库外示例

- Rust SDK。
- TypeScript SDK 或最小协议客户端。
- 仓库外 reference plugin。
- 文档、模板和兼容测试套件。

只有仓库外插件不引用 Dozer 私有 crate 仍能完成开发，API 边界才算真实。

### Phase 5：SSH 与安全强化

- Secret scope。
- PTY/terminal capability。
- 高风险确认。
- 长连接和恢复。
- 进程异常时资源回收。

### Phase 6：分发与生态

- 签名与来源展示。
- 包格式和离线安装。
- 更新与回滚。
- 兼容性索引。
- 社区目录；是否建设市场另行决策。

## 13. 关键风险

| 风险 | 影响 | 缓解方向 |
|------|------|----------|
| Plugin API 过早过宽 | Host 被历史兼容锁死 | 先做两个纵向试点，只公开最小 capability |
| UI Surface 过度自由 | 体验不一致、焦点和权限混乱 | Declarative 默认，WebView 隔离，高级能力显式授权 |
| MCP 被当内部总线 | 延迟、不确定、不可事务 | 坚持可靠 RPC/Event 与 Agent/MCP 双通道 |
| 进程化后状态分散 | 恢复和一致性复杂 | Host 管生命周期，插件拥有领域状态，事件带 revision |
| 核心 daemon 继续领域膨胀 | 插件仍需修改内核 | namespaced service/plugin-owned storage |
| WebView 数量增长 | 内存和窗口层级问题 | Surface 池化、后台 suspend、上限和诊断面板 |
| 多语言 SDK 行为不一致（编程语言，非 §19 的自然语言 UI 多语言） | 难以支持和调试 | 线协议为真相源，SDK 只做薄封装，统一 conformance tests |
| UI 文案翻译遗漏/不同步（§19 自然语言 i18n） | 中英文案缺失或过期，用户体验割裂 | 构建期校验 `default_locale` 与 `supported_locales` 的 key 集合是否对齐；缺失 key 一律退回 `title_fallback`，不空白 |
| 插件权限虚设 | 用户资产和 Secret 风险 | Host 统一授权/审计，Bridge 默认拒绝 |
| 原生动态库路线 | ABI、崩溃与供应链风险 | 第三方默认进程外，不承诺稳定 Rust ABI |
| Host 自己实现容器能力 | 微内核膨胀、平台维护失控 | 只定义 Execution Environment，通过 provider 使用外部运行时 |
| Container 被误认为安全沙箱 | 过度授权、宿主资产泄漏 | 保守挂载/网络/Secret 默认值，禁止容器 socket，明确风险模型 |
| 本地决策模型未经校准直接自动化 | 高置信度错误影响 Workflow | Shadow Mode、领域评估、阈值升级和人工最终裁决 |
| macOS Container 无 GPU 加速 | Laya 延迟和体验不达标 | CPU 实测、按需常驻；必要时增加 Native/MPS Provider |

## 14. 成功标准

V2 插件方向成立，需要至少满足以下可验证条件：

1. 仓库外插件只依赖公开 SDK/协议即可运行。
2. 修改插件无需重新编译或重启 Dozer Host。
3. 插件崩溃后 Host 与其他插件继续工作。
4. 插件能注册一个 Panel、Command 和 MCP tool。
5. WebView 插件无法越权访问项目、网络、进程或 Secret。
6. Agent 能发现两个互不依赖的插件，并完成跨插件目标。
7. 确定性 UI/数据流在无 Agent、无模型网络时仍然正常。
8. 同一插件可在 Dev Host 和完整 Dozer 中运行，无两套业务实现。
9. 至少一个插件后端可以不用 Rust 实现，证明协议没有泄漏 Rust ABI。
10. Host 升级时，兼容范围内的旧插件通过自动 conformance tests。
11. 同一 Execution 可以在 LocalWorktree 与至少一种外部 Container Provider 中运行，且 Delivery 记录可重现环境信息。
12. Laya 在 Shadow Mode 中可被替换或停用，不影响确定性 Workflow；每次建议可追溯并能升级给人类。

## 15. 最终裁决建议

### 应采纳

- 将 Dozer 定位为可组合的 Agent Workspace Runtime。
- 长期保留 iced 作为可信 Host UI。
- 第三方插件默认进程外运行。
- 插件 UI 框架自由，但嵌入面统一收敛为 Declarative/WebView；任意原生 UI 走外部窗口。
- Plugin Protocol 与 MCP 分层：前者保证可靠运行，后者负责 Agent 语义编排。
- 官方插件先作为 SDK 的真实消费者，再向社区承诺稳定性。
- 以 Code Health → Todo → SSH 的顺序验证边界。

### 不应采纳

- 仅把目录搬成 crate 就宣称插件化完成。
- 把 iced/Rust 类型作为第三方 ABI。
- 用 MCP 承担 UI 刷新、事务和内部事件。
- 第一阶段就建设动态库 ABI、插件市场或强沙箱。
- 在没有纵向试点前批量重构全部 extension。
- 为插件化立即把整个 Host 重写为 Tauri。

## 16. V2 Host 目标形态

V2 不应继续以“若干彼此平行的面板”为核心，而应围绕一条统一主线组织运行时与数据：

```text
Goal → Task → Execution → Delivery → Check → Acceptance
```

现有面板分别提供这条主线上的执行、上下文、观测、验证或资源能力。Host 不承载具体领域功能，但必须拥有跨插件共享的工作流骨架；否则各插件会分别发明 Task、执行状态、证据和验收，最终无法组合。

### 16.1 Workspace Shell

Host 继续负责：

- 窗口、项目页签、Rail、布局和多窗口。
- Panel、Page、Tab、Overlay 与 WebView Surface。
- 主题、菜单、快捷键和全局通知。
- UI 状态恢复。

### 16.2 Workflow Kernel

Host/daemon 应拥有少量跨插件公共对象：

```text
Workspace
Goal
Task
Execution
Delivery
CheckResult
Artifact
Decision
Acceptance
```

这些对象是插件协作的公共语言，不是某个面板的私有模型：Todo 创建和展示 `Task`，Agent 面板创建 `Execution`，Code Health 与 Browser 生成 `CheckResult`/`Artifact`，Conversation 审计 `Execution`，Acceptance 最终裁决 `Delivery`。

最低状态机建议：

```text
Task:
draft → ready → running → blocked → verifying
      → awaiting_acceptance → accepted/rejected/cancelled

Execution:
queued → starting → running → waiting
       → completed/failed/cancelled/timed_out

Delivery:
assembling → ready → checking
           → passed/failed → accepted/rejected
```

### 16.3 Contribution Registry

Rail 与顶层 Message 长期不应继续依赖硬编码的 `PanelKind` 穷举。插件统一注册：

- Panel/Page/Tab。
- Command。
- Verifier。
- MCP tool/resource/prompt。
- Background job。
- Data source。
- Context provider。
- Artifact renderer。

Host 以稳定的 `ContributionId` 管理贡献点，官方功能也通过同一机制注册，避免形成“社区插件一套、内建功能另一套”的双轨平台。

Panel/Page/Tab/Command 的展示名不是裸字符串，而是 `title_key`（可选，指向插件 `[i18n]`
资源里的 key）+ `title_fallback`（必填字面量）的结构（§19）：Host 渲染时按当前 locale 查
`title_key`，查不到（或插件没提供 `[i18n]`）就退回 `title_fallback`，保证任何插件都能正常
显示，只是不一定跟着切语言。

### 16.4 Host 横向运行能力

除前文的 Plugin Runtime、Permission、Event Bus 和 MCP Gateway 外，V2 Host 还需要补齐以下公共模块。

#### Worktree Manager

- 为 Task/Execution 创建 worktree 与分支。
- 绑定 Agent Session、cwd 和目标分支。
- 检测状态、ahead/behind 和潜在冲突。
- 支持接管、重试、更新、合并、保留、丢弃和清理。
- 将 diff/commit 归属到 Delivery。

所有权建议为：

```text
Task
 └── Worktree
      ├── Execution A
      ├── Execution B（重试/接管）
      └── Delivery
```

Worktree 归工作单元所有，Agent 只是某次执行者；这样才能支持暂停、换模型、接管和重试。

#### Verifier Runtime

统一承载命令测试、Code Health、Browser Test、lint/typecheck、安全扫描和社区验证器，输出标准结果：

```text
CheckResult
├── verifier_id / subject
├── status / severity / summary
├── findings
├── artifacts
└── started_at / finished_at
```

#### Decision/Approval Service

统一表达目标澄清、方案选择、权限申请、预算超限、冲突处理、验收、合并与高风险操作。各面板不得分别发明互不相通的确认状态。

#### Context Service

统一提供 Project、Shared Memory、Task、Execution、文件/符号和历史 Delivery 上下文，并记录 Agent 对上下文的读取与写入审计。

#### Artifact Store

统一管理 diff、日志、文件快照、图片、视频、Browser trace、测试报告、Code Health 报告、Token 报告和结构化 JSON。Host 管理身份、归属、索引、权限与生命周期，具体渲染器允许由插件贡献。

#### Budget/Policy Engine

提供 Task Token/时间上限、最大重试、模型和工具限制、文件写入范围、高风险确认边界，以及达到阈值后的暂停、降级或人工授权策略。

### 16.5 Host 边界裁决

V2 不追求“每个功能都是插件”的形式纯粹性：

- Host 拥有运行、安全、布局和跨插件 Workflow Kernel。
- 官方插件实现具体 Vibe Coding 能力与最佳实践。
- 社区插件替换、增强或新增能力。
- MCP 负责 Agent 语义编排；可靠状态由 Workflow Kernel、Plugin RPC 和 Event Bus 管理。

### 16.6 Execution Environment 与外部容器运行时

Dozer 需要借助容器解决依赖、进程、网络和运行环境隔离，但不提供也不实现容器运行时。产品边界是：

> Dozer 定义、分配和观察 Execution Environment；Docker、Podman、Apple `container` 或远程运行时负责真正创建和运行容器。

容器与 Worktree 解决不同问题：

- Worktree 隔离代码、分支与 Git 历史。
- Container 隔离工具链、依赖、进程、服务、端口和网络。

组合后的执行关系为：

```text
Task
 └── Worktree
      └── Execution Environment
           ├── Agent Session
           ├── Toolchain/Dependencies
           ├── Services/Ports
           ├── Secrets
           ├── Checks
           └── Delivery Artifacts
```

Host 应定义比 Container 更宽的抽象：

```text
ExecutionEnvironment
├── Local
├── LocalWorktree
├── Container
├── RemoteSSH
└── RemoteContainer
```

并通过 provider 接入具体后端：

```text
LocalWorktreeProvider
DockerProvider
PodmanProvider
AppleContainerProvider
SshProvider
RemoteContainerProvider
```

Dozer 负责：

- 探测 provider、版本、健康与 capability。
- 将 Task Worktree 以明确权限挂载进环境。
- 启动、停止、恢复和清理 Execution Environment。
- 管理资源限制、端口、服务与健康检查。
- 注入经过授权且有生命周期的环境变量/Secret。
- 收集日志、退出状态、资源使用与 Artifact。
- 让 Agent、Verifier 与 Browser Test 使用同一环境。
- 在 Delivery 中记录 image digest、工具版本、环境摘要和重现命令。
- provider 不可用时按策略降级到 Local/SSH。

Dozer 不负责：

- 实现 OCI runtime、namespace、cgroup、镜像格式或 registry。
- 替代 Docker Desktop、Podman 或 Apple `container`。
- 静默安装、升级或修改用户的容器运行时。
- 承诺 Container 本身是完整安全沙箱。
- 把 Workflow Kernel 写死到某个 provider 的 CLI/API。

安全默认值：不挂载用户 Home/`.ssh`/容器 socket，不允许 privileged，不共享宿主 PID，默认资源上限，Project 外路径需要确认，Secret 按 Execution 临时注入，Network 支持 restricted/offline，并记录实际 image digest 而非仅记录可变 tag。获得宿主容器 socket 的容器通常接近获得宿主控制权，必须默认禁止。

第一阶段可以先实现 `Local`/`LocalWorktree` 环境模型，再接入单一 Docker-compatible provider，跑通 worktree mount、exec、logs、动态端口、资源限制和异常清理；之后再增加 Apple `container`、Podman、Dev Container 与远程后端。`.devcontainer/devcontainer.json` 可以作为兼容输入，但不应让 Host 核心依赖 VS Code 专属行为。

UI 上不新增默认的 Container Rail 面板：Environment 状态进入 Task/Execution 详情，项目默认策略进入 Project Context，provider 管理进入 Settings/Diagnostics，远程与容器资源可在 `Environments` 视图统一呈现。

### 16.7 Decision Service 与 Laya 本地决策模型

Dozer V2 的大量控制点是边界明确的微决策，而不是文本生成问题，例如 Agent/模型路由、审计优先级、Tool Call 风险、Memory 分类、Finding 分流、Completion Review 和预算干预。Host 应提供通用 Decision Service：

```text
Decision Service
├── Deterministic Rules Provider
├── Laya Local Provider
├── Jev Cloud Provider
├── Structured LLM Provider
└── Human Provider
```

执行顺序遵循：

1. 确定性策略先处理明确允许或禁止的情况。
2. 模糊但候选有限的判断交给 Decision Model。
3. 低置信度升级到强模型或人类。
4. 高风险、不可逆操作最终仍由权限策略或人类裁决。

Decision Provider 使用统一的 typed contract（Boolean/Choice/Score）、概率和置信度，不向 Workflow Kernel 泄漏 Laya/Jev 的具体 API。每次决策必须保存 contract、provider、model/version、输入快照引用、候选概率、阈值、最终动作、是否升级、人工是否推翻和最终结果。

#### Laya 的推荐集成方式

Laya 是可本地运行、可微调的开源决策模型，适合成为第一个本地 Decision Provider，但不能成为 Dozer 的强制依赖。产品体验可以“内置”，工程上保持独立 sidecar：

```text
Dozer Decision Service
        │ Decision Protocol
        ▼
Laya Provider
├── Native Process
├── Container
└── Remote
```

第一版优先使用用户已有容器运行时承载 Laya 的 Python/PyTorch/Transformers/checkpoint，解决依赖、版本、清理和离线隔离问题；Dozer 不把 Python/PyTorch 链接进 `dozer-app` 或 `dozerd`。Laya Container 是 Host/Workspace 级共享的常驻按需服务，不应复制进每个 Agent Execution Container：

```text
Dozer-managed Laya Service Container
             ▲
             │ shared Decision Protocol
   ┌─────────┼──────────┐
   │         │          │
Execution A  Execution B  Host Audit
```

运行策略：首次调用启动、加载 checkpoint 后复用、空闲超时停止；checkpoint 与校准参数放命名 volume，Container 可销毁；默认 offline，只开放 UDS/loopback，不挂载项目目录、Home、SSH 或容器 socket。Laya 只接收 Host 构造的紧凑 Decision State，不直接扫描 Workspace。

macOS 上的 Linux Container 通常无法直接使用 Apple Metal/MPS，因此 Container Laya 应视为 CPU 部署方案，延迟必须在目标机器实测，不能套用 GPU benchmark。若后续高频决策需要更低延迟，再实现 macOS Native/MPS Provider；统一 Decision Protocol 保证部署方式变化不影响 Workflow Kernel。

#### Laya 的使用边界

Laya 基础模型不能被假定为开箱即用的 Vibe Coding Judge。必须先做领域评估和必要的微调/校准。推荐路径：

1. Shadow Mode，只记录建议，不改变真实 Workflow。
2. 从 Conversation Audit、Decision Inbox 排序、Task/Memory/Finding 分类等低风险场景开始。
3. 记录人类选择、模型置信度、推翻率和最终结果。
4. 为 `agent_trace_triage`、`completion_review`、`task_routing`、`memory_governance`、`finding_priority` 分别建立 Decision Contract 与评估集。
5. 仅对经过验证的高置信度低风险判断开放自动化。

以下行为不能仅凭 Laya 自动执行：接受 Delivery、合并 Git、删除数据、部署、数据库写入、Secret/权限提升或其他不可逆操作。Typed output 保证结果不超出候选集合，不保证选择一定正确。

Laya 的价值在于低成本处理大量微决策，将少数不确定或高风险情况准确送给强模型或人类，而不是替代 Agent、人类或确定性验证器。

## 17. V2 信息架构与新增页面

不建议继续横向增加同级 Rail 图标。V2 可以按四个领域组织贡献点：

```text
Control
  Mission/Overview · Tasks · Runs · Decisions

Workspace Resources
  Project/Context · Files · Git · Database · SSH · Browser

Governance
  Deliveries/Acceptance · Conversations · Usage · Code Health · Checks

Platform
  Plugins · Settings · Diagnostics
```

部分内容应作为首页、详情 Tab、二级视图或 Settings 页面，不必全部成为 Rail Panel。

### 17.1 Mission / Overview

新增项目级控制台，集中回答：

- 当前项目的 Goal 与进度。
- 正在运行、阻塞和失败的 Execution。
- 等待人类处理的 Decision。
- 等待验收的 Delivery。
- 当前预算、风险和 Check 摘要。
- Worktree 冲突和插件异常。

它不是展示更多统计数据的 Dashboard，而是人类进入项目后的行动入口。

### 17.2 Delivery / Acceptance

这是 V2 最关键的新页面。一次 Delivery 集中展示：

- Task 与验收标准。
- Agent 交付摘要。
- Worktree、diff 和 commits。
- 测试与 Verifier 结果。
- Code Health 变化。
- Browser 截图、录像或 trace。
- Token、时间与模型投入。
- 未验证事项和风险。
- 接受、拒绝、要求修改与合并操作。

人类不应在 Files、Conversation、Usage、Code Health 和 Browser 之间手工拼接证据。

### 17.3 Decision Inbox

统一汇集：

- Agent 请求澄清。
- 权限申请。
- 预算超限。
- 多方案选择。
- Worktree 冲突。
- 验收与合并请求。
- 高风险操作。

Decision 应支持关联对象、期限、候选项、推荐理由、最终选择和审计历史。

### 17.4 Plugin Manager

放入 Platform/Settings，而非默认占用 Rail。展示：

- 安装、启用范围、版本和兼容状态。
- 权限及其最近使用记录。
- 已注册 Surface、Commands 和 MCP tools。
- CPU/内存、日志和崩溃状态。
- 重启、禁用、升级和回滚。
- 开发模式与协议 Inspector。

### 17.5 不应单独成为面板的能力

- Worktree：属于 Task/Execution/Integration。
- Memory：主要属于 Project Context，也可嵌入 Task/Execution。
- Artifact：在 Delivery、Conversation 与 Checks 中呈现。
- Permissions：集中在 Decision Inbox 与 Settings。
- Goal：主要进入 Mission 与 Task 过滤。
- Budget：Usage 提供详情，其他页面只显示摘要。
- Handoff：属于 Execution 操作。
- Plugins：属于 Platform/Settings，开发模式下可独立打开。

## 18. 官方面板与能力的 V2 细化

### 18.1 Agent 面板 → Runs / Executions

关注点从“有哪些 Agent 终端”转为“哪些工作正在执行”。

新增：

- Goal/Task、Worktree、分支和 cwd。
- Agent、模型、启动参数和当前执行阶段。
- 最近工具/MCP 调用、Token、耗时和预算。
- 文件修改范围、阻塞原因和权限请求。
- 暂停、继续、终止、重试、换 Agent 接管。
- 创建 Reviewer Execution。
- 跳转 Conversation、Diff 和 Delivery。

必须区分：Agent 是执行者，Session 是连接，Execution 是一次工作尝试，Task 是工作目标。进程退出不能等同于任务完成。

### 18.2 Todo 面板 → Tasks

Task 在现有待办字段上增加：

- Goal、描述、优先级和风险。
- 验收标准。
- 依赖和阻塞关系。
- 预算与 Agent/模型策略。
- Worktree 策略。
- Verifier 列表。
- 当前/历史 Execution。
- Delivery 与 Acceptance 状态。

视图可提供列表、看板、依赖图、正在运行、等待验收和返工历史。“指派 Agent”应创建 Execution，而不是只向终端写入文本。

### 18.3 Project 面板 → Project Context

Project 成为“项目如何被 Agent 理解和治理”的真相源：

- 项目身份、仓库和 remote。
- README/项目文档。
- Shared Memory。
- 项目规则和 Agent instructions。
- 默认 Agent/模型、预算与 Verifier。
- Worktree 和验收策略。
- 环境启动方式、Secret 引用和已启用插件。

Shared Memory 进一步增加：类型（事实/决策/偏好/反馈/参考/临时状态）、来源、证据、作用域、历史、过期、冲突、敏感标记、读写审计，以及从 Delivery 自动提炼后由用户确认的入口。

### 18.4 Conversations 面板 → Execution Audit

时间线统一展示：

- Prompt/回复和 summary。
- Tool/MCP call。
- 文件修改和命令执行。
- 权限申请和 Memory 读写。
- Task 状态变化、Check 与 Decision。
- Agent handoff。

支持按 Project/Goal/Task/Execution/Agent/Tool/风险查询，从事件跳转到对应文件、diff、Task 和 Artifact，并比较同一 Task 的多个 Execution。自动摘要不能取代原始证据。

### 18.5 Usage 面板 → Cost & Efficiency

统计维度扩展到 Goal/Task/Execution、Agent/模型、插件/tool，以及成功、失败、被拒绝和最终是否产生 accepted Delivery。

新增指标：单 Task/accepted Delivery 成本、失败和重试浪费、模型效率差异、Token 与质量变化关系、人类等待时间。进一步通过 Budget Engine 支持阈值、超限暂停、模型降级和继续授权。

### 18.6 Code Health 面板 → Quality Governance

从静态报告升级为：

- Task 前基线与 Worktree 当前结果。
- 相对目标分支的新增、修复和恶化 Finding。
- Finding 归属文件与 Execution。
- Finding 转 Task。
- Quality Gate、趋势和有理由的豁免。
- Reviewer Agent 结论。
- 插件式扫描器。

Code Health 必须输出标准 `CheckResult` 进入 Delivery/Acceptance，而不能只存在于自己的面板。

### 18.7 Browser 面板 → Browser Test & Evidence

保留手动浏览，同时增加：

- 本地服务发现与启动。
- 测试用例、步骤和重放。
- Agent 驱动的导航、点击和输入。
- DOM、Console、Network 与 Accessibility 检查。
- 多 viewport、截图、录像/trace 和基线对比。
- 失败步骤定位。

Browser Run 归属 Task/Execution，证据进入 Artifact Store，断言进入 `CheckResult`，并在 Delivery 页面直接参与验收。

#### 18.7.1 产品边界：验收现场，不是通用 Chrome 替代品

WaveTerm 的 Web Block 说明网页可以成为 Workspace 的一等工作单元，但 Dozer 的差异化不在于
继续复制下载管理、扩展生态或完整浏览器历史，而在于把手动浏览和 Agent 自动化收敛到同一条
可审计验收链：

```text
Task / Acceptance Criteria
    → 启动或发现目标服务
    → 创建 BrowserRun（绑定 Project/Task/Execution）
    → 人类或 Agent 执行 BrowserAction
    → BrowserObservation / BrowserEvent
    → Screenshot / Console / Network / Trace Artifact
    → Assertion CheckResult
    → Delivery / Acceptance
```

手动浏览仍是必要入口，但每次正式验收必须可以选择“记录为证据”；没有归属、时间、环境和来源的
截图不能自动视为验收结论。Browser 插件负责领域状态与交互，Host 负责可信 Surface、权限和资源
预算，Workflow Kernel/Artifact Store 负责证据归属与生命周期。

#### 18.7.2 BrowserHost 与 Provider 边界

浏览器领域层不得持有具体 wry/WebKit/CDP 句柄，统一依赖版本化能力接口。概念接口至少覆盖：

```rust
trait BrowserHost {
    fn create_session(&mut self, scope: BrowserSessionScope) -> BrowserSessionId;
    fn execute(&mut self, target: BrowserTarget, action: BrowserAction) -> OperationId;
    fn capture(&mut self, target: BrowserTarget, kind: CaptureKind) -> OperationId;
    fn clear_storage(&mut self, scope: BrowserSessionScope) -> OperationId;
    fn open_external(&self, url: Url) -> OperationId;
}
```

接口是协议语义示意，不是已批准 Rust API。实现分成两类，不能混为一体：

1. **Interactive WebView Provider**：复用现有 wry 池，服务人类浏览、地址栏、前进后退、刷新、
   页面查找、缩放、外部打开和轻量截图。
2. **Automation Provider（规划中）**：CDP/Playwright/Browser Use 等以独立 sidecar 运行，通过
   owner-only UDS + 版本化 JSON-RPC/MCP 接入；负责可访问性树、可靠点击输入、Console/Network、
   trace、录像和多 viewport。Host 不内嵌 Node、Python 或 Chromium runtime。

两类 Provider 输出相同的 `BrowserEvent`、`BrowserObservation` 和 Artifact schema，使 Delivery
不需要理解后端差异。Interactive WebView 不应伪装成已具备可靠自动化；自动化 provider 未安装或
不可用时，UI 必须明确降级为手动验收，而不是静默跳过检查。

#### 18.7.3 会话、身份与站点数据隔离

借鉴 WaveTerm `partition` 的明确隔离语义，但映射到 Dozer 自己的 Host/Provider 协议：

| Scope | 生命周期 | 典型用途 | 默认存储策略 |
|------|---------|---------|-------------|
| `Global` | 跨项目持久 | 首页常驻浏览器、公共文档 | 持久化，显式清理 |
| `Project(project_id)` | 项目持久 | 项目内开发服务、长期登录 | 项目隔离持久化 |
| `Run(run_id)` | 单次 BrowserRun | 可重放验收、基线比较 | Run 结束后按策略保留或清理 |
| `Plugin(plugin_id)` | 插件生命周期 | 插件自带 WebView UI | 与 Browser 面板站点数据完全隔离 |
| `Ephemeral(id)` | 临时 | 无痕检查、不可信页面 | 关闭即清理 |

权限裁决同时考虑插件身份、Surface/Session 身份和资源 scope；不同项目、Run、插件不得意外共享
Cookie、LocalStorage、缓存、下载目录或认证信息。Secret 注入必须通过短期引用和显式授权，不能写入
浏览器 profile。会话清理、过期和异常退出后的回收都要产生审计事件。

#### 18.7.4 统一事件与证据模型

Browser Provider 至少规范化以下事件：`Ready`、`NavigationStarted`、`NavigationCommitted`、
`NavigationFailed`、`TitleChanged`、`ConsoleMessage`、`NetworkRequestFailed`、
`NewWindowRequested`、`DownloadRequested`、`Crashed`、`SessionClosed`。事件信封必须带
`project_id/task_id/execution_id/run_id/session_id/target_id/provider/timestamp` 中适用的身份字段，
并接受乱序、重复和迟到事件。

证据不是事件日志的别名。Artifact Store 保存截图、录像、trace、HAR/网络摘要、Console 摘要、
Accessibility snapshot 与结构化步骤；`CheckResult` 保存断言、期望、实际结果和关联 Artifact。
敏感 header、Cookie、Token、输入值在入库前按策略脱敏；原始 trace/HAR 设置大小、保留期和访问权限。

#### 18.7.5 生命周期与资源预算

沿用当前 `max_heavy_webviews` 和期望清单/池同步思路，并扩展为 Browser Surface 的正式契约：

- 可见目标优先运行，最近使用的后台目标可保活，超预算目标进入 `Suspended` 或被淘汰。
- 普通 Tab 切换不改变 URL、不重建 WebView，尽量保持滚动、历史与表单状态。
- 恢复失败、renderer 崩溃和 provider 断连进入显式可重试终态，不能留下空白 Surface。
- Console/Network 环形缓冲、截图、录像和 trace 分别设容量上限；压力下先停止采集，再淘汰后台
  Browser/Preview WebView，不影响终端与 Agent 会话。
- 新窗口、下载、剪贴板、摄像头、麦克风、地理位置和外部协议统一经过 Host 策略；默认拒绝或询问，
  插件页面不能自行放行。

第一条推荐纵向切片不是“补齐全部浏览器功能”，而是：`Task → BrowserRun → 手动/自动步骤 →
截图 + Console/Network 失败摘要 → CheckResult → Delivery`。该链路成立后，再增加录像、trace、
视觉基线与高级 AI Browser Provider。

### 18.8 Files 面板 → Worktree-aware Changes

新增：

- 当前主工作区/worktree 身份。
- Agent 修改标记及来源 Execution。
- 未提交、已提交和冲突状态。
- 与目标分支比较。
- 多 Execution 修改相同区域的预警。
- 从文件跳转相关 Conversation/Task。
- Delivery snapshot 只读查看。
- 敏感文件和受保护路径提示。

Files 继续坚持查看和验收优先，不演化成通用人工编码器。

### 18.9 Git Log 面板 → Integration

新增 Commit 与 Task/Execution/Delivery 关联、Worktree 分支、ahead/behind、目标分支变化、rebase/merge readiness、冲突预测、Check 状态、合并门禁、回滚点和多 Delivery 集成队列。复杂 Git 写操作统一经过 Decision/Approval。

### 18.10 SSH 面板 → Environments

从远程终端升级为远程执行环境管理：环境身份与标签、主机能力、项目映射、Remote Agent/Worktree、连接健康、Secret scope、命令和文件传输审计、端口转发、环境指纹、Task/Execution 绑定和高风险操作确认。

SSH 是权限、长连接、PTY 和资源回收模型的压力测试样本。

### 18.11 Database 面板 → Data Workspace

新增数据源作用域/环境标签、只读/读写模式、Schema Context Provider、查询历史和 Agent 查询审计、敏感列遮蔽、行数/成本限制、写操作确认、Transaction Preview、查询结果 Artifact 和测试前后数据验证。默认只读；写入与 DDL 需要显式权限。

### 18.12 Git/File History

历史不再只回答“文件何时变化”，还要回答：谁或哪个 Agent、属于哪个 Task/Execution、为什么修改、进入哪次 Delivery、是否被接受或后来回滚，并可跳转对应 Conversation 事件。

### 18.13 Settings

区分 Host、Workspace、插件、权限、Agent provider、Model policy、Secret、默认预算、默认 Verifier、Plugin API/开发模式、数据保留和隐私。插件配置由 manifest/schema 声明，Host 统一承载或打开插件 Surface。

### 18.14 Footbar → Status Center

只显示必须持续感知的跨平台状态：运行中 Execution、等待 Decision/Acceptance、插件异常、后台 Job、预算预警、Worktree 冲突和 daemon/MCP 状态。详细信息点击进入对应页面，避免信息堆积。

### 18.15 数据关联不变量

所有重要数据必须可沿主链追溯：

```text
Project
  └── Goal
       └── Task
            ├── Worktree
            ├── Execution
            │    ├── Agent Session
            │    ├── Conversation Events
            │    ├── Usage
            │    └── Tool/MCP Calls
            └── Delivery
                 ├── Changes/Commits
                 ├── CheckResults
                 ├── Artifacts
                 ├── Decisions
                 └── Acceptance
```

这个关系比面板如何拆更重要。数据关系统一后，面板才可能被插件替换或使用不同 UI；否则插件化只会把信息孤岛搬到不同进程。

### 18.16 对应实施顺序

1. 建立 Goal/Task/Execution/Delivery/CheckResult/Artifact/Decision/Acceptance 主模型，让现有面板先通过 ID 关联。
2. 实现 Worktree Manager；Task 派发创建 Execution，Conversation/Usage 自动归属，diff/commit 进入 Delivery。
3. 建立 Delivery/Acceptance 页面和 Verifier Runtime，先接入命令测试、Code Health 与 Browser Artifact。
4. 补齐 Budget、Approval、Retry/Handoff、Merge Gate 与 Decision Inbox。
5. 建立 Contribution Registry、Plugin Runtime、动态 Panel、MCP Gateway、UI Surface、SDK 与 Dev Host。
6. 再按 Code Health → Usage → Conversations → Todo → Browser → Database → SSH 的顺序迁移官方插件。

Agent/Worktree、Workflow Kernel 与 Delivery/Acceptance 在前期应视为平台能力；边界稳定后再判断其哪些 UI 或策略部分适合插件化。

## 19. 多语言（i18n）策略（2026-09-26 追加）

产品明确要求 V2 至少支持英语、简体中文两种界面语言。这里的“多语言”专指**人看的 UI 文案**，
与 §5.3、§13 提到的“插件可用多种编程语言实现”是两个不相关的概念，本节起统一按此含义使用。

### 19.1 范围裁决

覆盖：

- Host chrome 文案（rail、topbar、settings 等 Micro Host 自带界面）。
- 官方插件迁移插件化时的 UI 文案（面板标题、按钮、提示、错误信息）。
- Command Palette 等 Contribution Registry 暴露给用户的命令名。
- 第三方插件 Manifest 契约里“如何声明翻译”这一层（是否实际提供翻译由插件作者自行决定）。

不覆盖：

- MCP tool/resource/prompt 的 `description` 字段——面向 Agent/LLM，固定英文，翻译反而降低
  调用准确率。
- Agent 可读的日志、审计记录、Execution 事件。
- 插件向 `dozerd` 上报的结构化错误码本身（错误码不翻译，只有把错误码渲染成人看提示文本的
  那一层才走 i18n）。

### 19.2 技术方案取舍

| 方案 | 优点 | 缺点 |
|------|------|------|
| **扁平 key-value 资源文件（采纳）** | 任何语言（Rust/JS/外部进程）都能读；零构建步骤；契合当前 UI 文案以短标签为主的现状 | 无内置复数/性别语法，遇到计数敏感文案需手写规则 |
| Mozilla Fluent（`.ftl`） | 复数、变量、性别等语言学特性完备；Rust（`fluent-bundle`）与 JS（`@fluent/bundle`）都有成熟实现 | 给 Host、SDK、每个插件生态引入新概念和依赖，对当前需求是过度设计 |
| gettext（`.po`/`.mo`） | 工具链和翻译平台生态最成熟 | 更适合长文档/源码字符串抽取，不贴合“插件 manifest 声明式提供字符串表”的模式，还要求编译步骤 |

采纳扁平 key-value：每个 locale 一个资源文件（`en.json`/`zh-CN.json`），值支持 `{var}` 占位符
替换，不做复数语法。YAGNI——真出现复数/性别这类需求，再针对具体 key 升级实现，不影响协议
"key → string" 的外部形状。

### 19.3 Manifest 契约（对应 §7.4 示例）

`[i18n]` 为可选字段：`default_locale`、`supported_locales`、`strings_dir`。`strings_dir`
下每个 supported locale 一个扁平 JSON 资源文件。Host 与官方插件必须提供 `en` + `zh-CN`；
第三方插件可以完全不提供 `[i18n]`，此时所有展示名退回各自的 `title_fallback` 字面量。

### 19.4 Contribution Registry 集成（对应 §16.3）

Panel/Page/Tab/Command 的展示名从裸字符串改为 `title_key`（可选）+ `title_fallback`
（必填）。解析顺序：当前 locale 的 `title_key` → `default_locale` 的 `title_key` →
`title_fallback`。任何一层缺失都不会导致 UI 空白。

### 19.5 运行时归属

Key 解析器放在协议契约层（对应静态结构图 D 节 `dozer-protocol`），因为 `dozerd`、
`dozer-host`、`dozer-mcp` 与插件 SDK 都要用同一套解析规则；key 按插件 id 命名空间化，避免
不同插件的 key 冲突。Host 自己只持有一份全局状态——“当前 active locale”，存在 `dozerd`
Platform Services 侧（随 Host 重启保留），Settings 面板可切换，默认值来自系统语言探测，
兜底 `en`。

### 19.6 各 Surface 的实现边界（对应 §7.3）

- **Declarative / Host 自身 UI**：直接调用协议契约层提供的 `t(key)` 查表。
- **WebView（Surface B）**：Host 通过既有 envelope 机制告知当前 locale 及其变化，翻译资源
  由插件自己的前端 bundle 加载渲染，Host 不替插件注入译文。
- **External（Surface C）**：Host 通过启动参数/环境变量传递 locale，具体如何实现完全交给
  插件自己，不强制统一运行时（与 Surface C“Host 不干预内部实现”的既有裁决一致）。
- **None（Surface D）**：无 UI，不涉及。

### 19.7 阶段落点

从 Phase 1 起，Manifest schema 与 Contribution Registry 的 `title_key`/`title_fallback`
结构就必须落地，避免后续因为“协议里没留字段”而做破坏性变更；具体翻译内容和全量 UI 覆盖可以
晚于 Phase 1 逐步补齐，不阻塞其他阶段推进。

## 20. 核心战略总结

Dozer V2 的两条战略互相强化：

1. **工程战略**：把编译、升级和故障边界从整个 Dozer 缩小到单个插件。
2. **产品战略**：让 Workspace 从官方预制应用变成用户、社区和 Agent 可以共同塑造的平台。

最终目标不是“Dozer 支持很多插件”，而是：

> Dozer 核心长期保持小、稳定和兼容；绝大部分新增领域能力不再修改 Dozer Host，而是安装一个可独立运行、可由 Agent 组合的插件。
