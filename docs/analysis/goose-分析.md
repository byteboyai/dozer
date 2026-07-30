# Goose 代码级分析

> 分析对象:https://github.com/aaif-goose/goose(原 `block/goose`,已随项目治理权移交 Linux Foundation 旗下 Agentic AI Foundation〔AAIF〕迁移到 `aaif-goose` 组织下,旧地址会 302 重定向,本次核实以新地址为准)
> 本地副本:`/Users/chrischiang/AI/goose-src`(本环境到 GitHub 的 `git clone`/`codeload` tar 传输反复因连接中断/重置失败,重试多次后以 `codeload` tarball 快照完整落地,过程中同步用 `gh api .../git/trees/main?recursive=1`〔2,910 个条目,非截断〕逐文件核实)
> 分析日期:2026-07-28 · Apache-2.0 · 51,838 star / 5,758 fork(`gh api repos/aaif-goose/goose` 核实)· **Rust**:219,357 行 / 463 个 `.rs` 文件(其中 `crates/goose/src` 下 274 个)/ 2,550 个 `#[test]`+`#[tokio::test]` 用例(另有 26 个独立的 `tests/` 目录集成测试文件)· **TypeScript**(Electron 桌面壳 `ui/desktop` + 终端 UI `ui/text` + SDK):104,633 行 / 589 个 `.ts`/`.tsx` 文件(`ui/desktop` 476 个、`ui/text` 18 个)/ 68 个测试文件、552 处 `it()`/`test()` 用例

## 一句话定位

Goose 和 jcode/seek_code 同属"agent 本身"范畴——不是驾驭外部 agent CLI 的编排层,而是自带模型循环、自带工具执行的完整 agent。但它与 jcode 走的是相反的产品哲学:jcode 的原则是"自研一切"(自己的 tool-calling 循环、自己的 TUI 渲染引擎、自己的记忆图),Goose 的原则是"**只做通用的 agent 内核,能力靠 MCP 扩展生态和 15+ provider 接入外部世界**"——Goose 自己甚至没有一个原生 Rust GUI,桌面壳是 Electron+React,连它的终端 UI(`goose tui`)都是用 Node/Ink(React for terminal)写的 TypeScript 包,不是 Rust。这使 Goose 成为"Rust 核心 + 非 Rust 壳"这一架构选择目前唯一的实测参照点,和 Dozer"全 Rust、iced 原生 GUI"的选择正好构成一组直接对照。

还有一个在立项之初没预料到的复杂性:Goose 并不是单纯的"agent 本身"——它通过 Agent Client Protocol(ACP)同时扮演两种角色:`goose acp` 可以作为 ACP **server** 被 Zed/JetBrains 这类编辑器直接接入(这时 Goose 是"被驾驭"的一方);而它的 provider 层里又有 6 个文件(`claude_acp.rs`/`codex_acp.rs`/`copilot_acp.rs`/`cursor_agent.rs`/`amp_acp.rs`/`pi_acp.rs`)把 Claude Code、Codex、GitHub Copilot、Cursor Agent、Amp、Pi 这些外部 agent CLI 当作可插拔的"provider"来驱动(这时 Goose 变成了编排层,和 kooky/orca/workmux 是同一件事)。所以准确的定位是:**Goose 的核心自带完整 agent 循环,但同时用同一套 provider 抽象把"自己推理"和"驱动别的 agent CLI"统一了起来**——这是六个分析对象里第一次看到两种范畴在同一产品里合流的实例。

## 工程结构:Rust 单体 workspace + Electron/Node 双前端

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

## 核心机制一:Rust 核心与 Electron/Node 壳的边界——对 Dozer 技术选型最直接相关的一节

**边界怎么划的**:`ui/desktop/src/gooseServe.ts` 是 Electron 主进程里管理 Rust 后端生命周期的模块——`findGooseBinaryPath()` 在打包环境下从 `resourcesPath/bin/goose` 找到 Rust 编译出的 `goose` 二进制,`spawn()` 把它当子进程拉起,随机分配一个本地端口和一次性 `serverSecret`,轮询一个健康检查 URL 直到就绪,连接串是 `GOOSED_CERT_FINGERPRINT=` 前缀的证书指纹(TLS,不是明文 HTTP)。也就是说:**Electron 渲染进程和 Rust 核心之间不是走 Electron 原生的 `ipcRenderer`/`ipcMain`,而是走本机回环 HTTP/WebSocket + TLS 证书指纹钉扎 + 一次性共享密钥**——`ipcMain`/`contextBridge`(`ui/desktop/src/preload.ts`,363 行)只用来处理纯操作系统层面的事情(`open-external`、`directory-chooser`、`add-recent-dir` 等,`main.ts` 3,162 行里能看到这些 handler),真正的 agent 对话、工具调用流、会话数据全部通过 HTTP/WS 打给 Rust 后端的 `goosed` server。这个边界划分和 t3code 的"Node.js WebSocket server 作为唯一执行边界"是同一种思路,也是 Dozer `dozer-app`(iced)↔`dozerd`(UDS)边界的另一种实现方式——差别是 Goose 选择了"本机网络+证书钉扎",Dozer 选择了"Unix Domain Socket",后者天然把攻击面限制在同一台机器的文件系统权限内,前者需要额外的密钥/证书机制来达到同等的信任边界,这是 Dozer 现有选择的一个佐证而非需要修改的地方。

**是否后悔没做全 Rust GUI**:能查到明确证据。2026-02-17,外部贡献者 `balcsida` 提交了 PR #7269"feat: migrate desktop app from Electron to Tauri v2",附完整方案:用 Tauri v2 替换 Electron 主进程,新写约 1,600 行 Rust(8 个模块:`lib.rs`/`commands.rs`/`goosed.rs`/`tray.rs`/`menu.rs`/`settings.rs`/`wakelock.rs`/`dock.rs`,42 个 IPC command)去掉当时 2,394 行的 `main.ts`,前端 40+ 个文件通过一个 `tauri-bridge.ts` 兼容层(模拟 `window.electron`)保持不动,还给出了具体收益(免打包 Chromium 省约 150MB 体积、Rust 后端替代 Node 主进程降内存、"Rust alignment: keeps the entire stack in Rust, matching the existing goose codebase")和 46 个单元测试。维护者 `jamadeo` 的回复是关键证据:"a change of this magnitude should be discussed before diving into implementation. I'm not saying it isn't the right move for goose, but the implications... go beyond just a review of the code"——**因为流程问题(未经讨论直接实现)关闭,不是因为技术上被否决**,同日贡献者又开了一个配套讨论 issue(PR 引用的 #7332,但该 issue 在核实时已 404,可能被删除或从未真正建好),此后未见后续合并记录。截至分析时(`main.ts` 已经涨到 3,162 行),Electron 仍是唯一的桌面壳。

**对 Dozer 的意义**:这是三点可以直接拿来对照 Dozer 选型的证据:(1)Goose 官方从未正面反驳"全 Rust 桌面壳"这个方向,只是嫌"事情太大、没走流程"——说明"Rust 核心 + Rust GUI(Tauri/iced 等)"在工程上是可行的,只是切换成本(3000+ 行主进程逻辑、40+ 个 IPC command、17 个 Electron 插件替换)让在位项目难以启动重写,这恰恰是 Dozer 现在(项目还小)就把 GUI 定在 iced 而不是 Electron 的证据——越早定型越不用背这笔迁移债;(2)即便是"更轻"的终端 UI,Goose 也选择用 Node/Ink 写而非 Rust(`ui/text` 全是 .tsx),说明"native 终端渲染"这件事在 Goose 眼里成本高到连 TUI 都外包给了 JS 生态,这是 jcode(自己写终端渲染引擎、自己写 scrollback)路线的反面参照,Dozer 应该记住这是光谱的两端,不是只有一种"正确"选择;(3)本机 HTTP/WS + TLS 指纹 + 一次性密钥这套连接方案,是"进程边界确实需要网络协议,又不想用裸 IPC"时的一份可执行范本,如果 Dozer 未来考虑给 `dozerd` 加一个可选的网络监听面(例如给远程/多设备场景),这是比自造协议更省事的起点。

## 核心机制二:MCP 扩展系统——70+ 扩展是生态数字,不是 Goose 自研深度

**加载方式**:`ExtensionConfig`(`crates/goose/src/agents/extension.rs`)有 `Stdio`(子进程,`TokioChildProcess`)、`StreamableHttp`(远程 MCP,`StreamableHttpClientTransport`)、`Builtin`(进程内)、`Sse`(已废弃,仅配置兼容)、`Platform`/`Frontend`/`InlinePython` 几种变体,`extension_manager.rs` 统一管理这些客户端的初始化、工具列举、调用转发。**没有找到任何运行时沙盒机制**——不像 workmux 那样有容器/VM 隔离,Stdio 型扩展就是普通子进程,和 agent 本体共享文件系统/网络权限。

**唯一的供应链防护**是 `agents/extension_malware_check.rs` 里的 `OsvChecker`:当扩展是通过 `npx`/`uvx` 安装的包时,调用 [OSV](https://osv.dev)(Open Source Vulnerabilities)数据库查该包名/版本有没有 `MAL-*`(恶意包)标记的安全公告,查到就拒绝安装;**查不到生态类型(非 npx/uvx)就直接放行("fail open")**,这是明确写在注释里的取舍。也就是说 Goose 对"70+ 扩展"生态的治理仅止于"装的时候查一下是不是已知的恶意包",不做运行时行为限制。

内置的 `Builtin` 扩展本身很少——`ui/desktop/src/built-in-extensions.json` / `bundled-extensions.json` 只列了 5-7 个(Developer、Computer Controller、Auto Visualiser、Memory、Tutorial 等),README 里"connect to 70+ extensions via MCP"指的是**外部 MCP 服务器生态**(GitHub、Slack 等第三方或社区维护的 MCP server,通过 `Stdio`/`StreamableHttp` 配置接入),这些服务器本身不是 Goose 专属开发的,任何 MCP 客户端(Claude Desktop、Cursor 等)都能用同一批服务器——**"70+"是 MCP 这个开放协议的生态体量,不是 Goose 在扩展深度上比别人做得更深**。

**对 Dozer 的意义**:jcode/seek_code 都没有对等的 MCP 扩展系统可比较(它们各自的工具是内建的,不是走 MCP 协议),这一节的价值在于纠偏——如果 Dozer 未来考虑"要不要也接入 MCP 生态"作为差异化卖点,先想清楚"MCP 生态数字大"和"MCP 治理深"是两件独立的事:Goose 用了 70+ 这个数字,治理却只到"装包前查一次 OSV 恶意库",这不是 Dozer 应该抄的深度上限,而是提醒"接入 MCP"本身不构成治理能力,治理要另外做(见机制四)。

## 核心机制三:Provider 抽象——15+ 是保守数字,且把"驱动外部 agent"也纳入了同一套接口

`crates/goose/src/providers/` 下 54 个文件里,刨去 `mod.rs`/`base.rs`/`utils.rs`/`provider_registry.rs`/`provider_secrets.rs`/`provider_test.rs`/`testprovider.rs`/`oauth*.rs`/`*auth.rs`/`catalog_util.rs`/`custom_provider_config.rs`/`usage_estimator.rs`/`toolshim.rs`/`cli_common.rs`/`acp_tooling.rs`/`private_file.rs` 这些基础设施文件,真正意义上的 LLM/推理服务适配至少有 25 个(`anthropic_def`/`openai_def`/`google_def`/`azure`/`bedrock`/`databricks_def`/`databricks_v2_def`/`ollama_def`/`ollama_cloud`/`openrouter`/`huggingface`/`litellm`/`nanogpt`/`sagemaker_tgi`/`snowflake_def`/`tetrate`/`xai`/`gcpvertexai`/`githubcopilot`/`kimicode`/`avian`/`local_inference`/`codex`/`chatgpt_codex`/`gemini_cli` 等),超过 README 宣传的"15+"。另有 6 个文件是通过 ACP 协议驱动**外部 agent CLI**(而不是原始模型 API)的适配器:`claude_acp.rs`/`codex_acp.rs`/`copilot_acp.rs`/`cursor_agent.rs`/`amp_acp.rs`/`pi_acp.rs`。

统一接口在 `goose-providers` crate 的 `Provider`/`ProviderDef` trait(`crates/goose/src/providers/base.rs` re-export)加 `provider_registry.rs` 里的 `ProviderEntry`(元数据 + 构造函数闭包 + 可选清理函数 + 是否支持库存刷新),`normalize_model_config()` 负责把不同厂商的 context 窗口大小/模型元数据统一补全。所有 provider(不管是打真实 API 还是拉起外部 agent CLI)都通过同一个注册表暴露,UI 层不需要区分二者。

**对 Dozer 的意义**:这印证了"provider 抽象"和"driving 外部 agent"在实现上完全可以是同一套接口——Dozer 目前只驱动 Claude Code 一家,如果二期真要扩展到 Codex/其他 CLI,`ProviderDef` 这种"元数据 + 构造闭包 + 统一 trait"的设计,比给每个 agent CLI 写一套平行代码更值得参考;但也要注意 Goose 的 provider 抽象本质是"LLM 请求/响应格式"层面的统一,ACP 适配器能塞进同一接口是因为 ACP 本身就是"agent 对话协议"标准化的产物,这再次印证了 t3code 分析文档里"遇到新 agent CLI 先查有没有结构化协议(ACP/app-server),没有才退回 PTY"这条准则——Goose 的 provider 层就是这条准则的另一个独立实现证据。

## 核心机制四:命令风险/权限——目前七个分析对象里最系统化的多信号治理管线

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

## 核心机制五:会话/记忆/子任务——"压缩优先"而非"语义检索优先",与 jcode 形成清晰对照

- **会话持久化**:`crates/goose/src/session/session_manager.rs` 用 SQLite(`sqlx::Pool<Sqlite>`)存储,`SessionManager`/`SessionStorage` 支持创建、按 id 取回(含全部消息)、用量统计、列表分页搜索——是完整的 resume 支持,不是仅追加日志。
- **子任务/subagent**:`crates/goose/src/agents/subagent_execution_tool` + `subagent_handler.rs`,`run_subagent_task()` 递归构造一个完整的 `Agent`(带自己的 `AgentConfig`/`TaskConfig`/`Recipe`),可选择只返回最后一条消息(`return_last_only`),支持取消令牌和消息回调——子任务是"完整 agent 的递归调用",不是 jcode swarm 那种多个平行 agent 互相感知文件变更的协作模型,更接近"任务委派后等结果"的单向 fan-out。
- **记忆**:内置的 `memory` 扩展(`crates/goose-mcp/src/memory/mod.rs`)只是一个按 `category`/`tags` 存取的平铺文件存储(`remember_memory`/相关工具),**没有语义向量、没有图结构、没有相似度检索**——是"模型主动决定记什么、按类目存"的简单键值存储,而不是自动抽取+语义索引。
- **上下文压缩**:`context_mgmt/mod.rs`,`DEFAULT_COMPACTION_THRESHOLD = 0.8`(80% 阈值触发),支持"结构化摘要"(`StructuredSummary`)、工具调用对批量摘要(`TOOLCALL_SUMMARIZATION_BATCH_SIZE = 10`)、手动/自动压缩走不同的续接提示语。**官方架构文档明确写着 Goose 的取舍**(`documentation/docs/goose-architecture/goose-architecture.md`):"goose includes everything **versus a semantic search**"——即 Goose 的哲学是"能塞多少上下文就塞多少,靠算法删旧内容/摘要压缩来省 token",**明确不做语义检索**。

**对 Dozer 的意义**:这一节的价值是给 jcode 记忆子系统的分析提供一个反例校准——jcode 的"语义图 + 检索 + ambient 整理"不是记忆问题的唯一解,Goose(体量、star 数都更大的项目)选择了完全相反的"brute-force 全部塞进去 + 压缩兜底 + 用户主动记录偏好"路线,而且是**显式写进架构文档的立场**,不是没做到位。Dozer 排三期"记忆"设计时,这两个真实存在的路线(jcode 的语义检索 vs Goose 的"全塞 + 压缩")应该并列评估,不能默认语义图是唯一正确答案——如果 Dozer 的记忆需求场景更接近"记住少量用户偏好/项目约定"而非"海量历史语义检索",Goose 这种轻量方案的工程成本明显更低。子任务机制上,Goose"递归调用完整 Agent、等结果返回"这种同步委派模型,也比 jcode swarm 的多 agent 实时协作更接近 Dozer 现有的单会话验收模型,二期如果先做"子任务委派"再考虑"多 agent 协作",Goose 这条路径工程量更小。

## 对 Dozer 的启示汇总

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
