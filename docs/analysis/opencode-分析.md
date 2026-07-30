# OpenCode 代码级分析

> 分析对象:https://github.com/anomalyco/opencode(原 `sst/opencode`,`gh repo view sst/opencode` 已确认重定向到同一仓库,owner ID 一致,不是两个不同项目;仓库描述"The open source coding agent")
> 本地副本:`/Users/chrischiang/AI/opencode`(`git clone --depth 1`,默认分支 `dev`,不是 `main`)
> 分析日期:2026-07-28 · TypeScript/TSX 60.6 万行(3,137 个 `.ts`/`.tsx` 文件,657 个测试文件)/ 32 个 workspace 包 / MIT / 190,420 star、24,190 fork(2025-04-30 创建,分析当天仍在推送)

## 一句话定位

OpenCode 和 jcode/seek_code 同属**"agent 本身"**范畴——自己做 tool-calling 循环、自己对接模型 API、不依赖外部 agent CLI,而不是 kooky/orca/workmux/t3code 那种"驾驭已有 CLI"的编排层。三者体量差异巨大且有代表性:jcode(70.7 万行 Rust)是"重工程但个人/小团队维护"的极致,seek_code(1.4 万行 TypeScript)是"够用就好"的个人规模,**OpenCode(60.6 万行 TypeScript,SST 团队 + 大量外部贡献者维护,32 个 workspace 包)是"主流、被广泛采用、工程化程度最高"的那一极**——体量接近 jcode 但组织方式完全不同:jcode 是单一作者把所有子系统(供应商适配、TUI 渲染引擎、记忆图、swarm 协作)都从零手写;OpenCode 则大量借力外部生态(Vercel AI SDK、models.dev 模型目录、Effect 生态、Agent Client Protocol 标准),自己的核心代码集中在"把这些拼起来还要处理好几十种供应商的私有怪癖"这件事上。这是本系列第一次拿到一个 star 数两个数量级于 jcode/seek_code 的"agent 本身"项目做对照,填的是"主流答案长什么样"这个空。

**一个先澄清的事实核实**:分析前不确定 OpenCode 是否曾用 Go 实现(有传闻),本地代码库里 `.go` 文件数为 0,`packages/tui` 现在是 `.tsx`(用 SST 自研的终端渲染库 `opentui`,`package.json` 里有 `upgrade-opentui` 脚本),运行时是 **Bun**(`packageManager: "bun@1.3.14"`,`@effect/sql-sqlite-bun` 做 SQLite 层)而不是 Node。历史上是否有过 Go 版 TUI 未能本地核实(仓库是 `--depth 1` 浅克隆,没有完整历史),但**至少当前这份代码是纯 TypeScript**,这一点是实测确认的。

## 工程结构

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

## 核心机制一:Client/Server 架构——HTTP+SSE 为主,WebSocket 只管 PTY,和 T3 Code 不同构

`packages/server/src/routes.ts` 用 Effect 的 `HttpApiBuilder.layer(Api, { openapiPath: "/openapi.json" })` 装配路由——**是一个有 OpenAPI 描述的类型化 HTTP API**,不是裸 WebSocket server。实时事件走 `packages/opencode/src/server/routes/instance/httpapi/groups/event.ts` 里的 `GET /event`,声明为 `HttpApiSchema.asText({ contentType: "text/event-stream" })`——**SSE**,单向服务器推送。真正用 WebSocket 的地方是 `.../httpapi/handlers/pty.ts` + `websocket-tracker.ts`,专门给内嵌终端面板用,和 T3 Code 里"`apps/server/src/terminal` 大概率是辅助面板不是控制通道"的猜测正好对上——**两个项目都把 WebSocket 限定在终端面板这个辅助功能上,核心协议走的是另一条路**,只是 T3 Code 核心协议本身也是 WebSocket(JSON-RPC over WebSocket),OpenCode 核心协议是 HTTP+SSE,两者不同构。

TUI(`packages/tui`)、CLI、Web app(`packages/app`)、Desktop(Electron 壳套 `app`)全部通过同一个生成的 `OpencodeClient`(`packages/sdk`)访问这个 HTTP+SSE 接口。IDE 集成分两条路:VSCode 走独立扩展(`sdks/vscode/src/extension.ts`);Zed(以及其他支持 **Agent Client Protocol** 的编辑器)走 `packages/opencode/src/acp/service.ts`——这层代码本身也只是 `OpencodeClient` 的又一个调用方(`ACP.newSession`/`loadSession`/`prompt` 内部全部转成 `input.sdk.session.*` 调用),即 **ACP 适配层和 TUI/Web 是对等的、同一个 HTTP API 的两种前端**,不是单独的另一套后端逻辑。

**对 Dozer 的意义**:这是本系列第一次实测确认"一个后端多前端"不必然意味着 WebSocket——REST+SSE(单向推送够用,PTY 这种双向交互才需要 WebSocket)是另一条同样成立的路线,而且是 OpenCode 这种规模验证过的路线。`dozerd` 目前的 UDS 协议如果未来要对外暴露(比如给一个假设的编辑器插件),不必默认抄 WebSocket——"控制面 HTTP/SSE、终端面板才用双向通道"这个拆分本身就是可以直接借鉴的架构判断,和 T3 Code 的 access/launch 分离是两条不同但互补的经验。ACP 是 Zed 发起的跨 vendor 标准,T3 Code 也在用(`effect-acp` 包做 ACP client),OpenCode 这边是反过来做 **ACP agent 端**——如果 Dozer 未来考虑被第三方编辑器驱动而不是自己做 GUI,ACP 是已经有两个不同项目分别从两端验证过的现成协议,不需要自己发明。

## 核心机制二:Tool-Calling 循环——大量借力 Vercel AI SDK,不是从零重做

`packages/opencode/src/session/llm.ts` 的默认路径是把请求交给 Vercel 的 `ai` 包(`streamText`/`wrapLanguageModel`),自己只做:包一层 Effect 服务、注入 `experimental_repairToolCall`(工具名大小写纠正+参数解析失败时降级成 `invalid` 工具、把错误原样喂回模型)、`activeTools` 过滤、`experimental_telemetry`。这是**默认路径**;还有一条 `flags.experimentalNativeLlm` 开关控制的"native"运行时(`@opencode-ai/llm`),特性标记关着,一旦某个 provider 返回"不支持"就自动回落到 AI SDK 路径——说明团队在往"自己掌控请求/响应解析"方向走,但目前还没把 AI SDK 这条腿撤掉。

工具执行侧(`packages/opencode/src/tool/tool.ts` + `registry.ts`)是自己写的:`Tool.define` 把每个工具包成 Effect,统一做参数 schema 解码(失败时用类型化的 `InvalidArgumentsError`,把"如何重新组织入参"的提示原样喂回模型而不是吞掉错误)、per-agent 输出截断(`Truncate.output`)、执行前的 `ctx.ask()` permission 挂钩。内建工具集(`shell`/`read`/`edit`/`write`/`glob`/`grep`/`task`/`webfetch`/`websearch`/`skill`/`apply_patch`/`plan`/`lsp`)加上插件工具(从 `tool/*.js` 文件或插件包动态 import)、MCP 工具统一进这个 registry,按 provider/model 特性做条件过滤(比如 GPT 系模型用 `apply_patch` 代替 `edit`/`write`)。

**对 Dozer 的意义**:这是和 jcode 的一处直接、具体的路线分歧,值得记下来对照——jcode 选择"完全自己实现 20+ 家供应商的 HTTP/SSE 解析",OpenCode 选择"默认交给 Vercel AI SDK 统一处理协议细节,自己只管工具分发和治理层"。哪条路线更好取决于目标:jcode 要的是极致的性能/内存控制(README 里 27.8MB vs OpenCode 的 318.4MB 追加会话内存),OpenCode 要的是"尽快接入尽可能多的 provider,把工程精力放在工具/权限/会话这些用户能感知的层面"。Dozer 是治理层,不做模型循环,这条分歧本身不直接可抄,但揭示了一个可迁移的判断:**"provider 覆盖面"和"运行时资源效率"是有工程取舍的两端,不是免费的**——如果 Dozer 未来要做"多 agent CLI 统一驾驭",也会面对类似取舍(自己写每个 CLI 的适配器 vs 复用一层通用抽象)。

## 核心机制三:Provider 抽象——统一接口是"胶水层",真正的资产是处理供应商私有怪癖的那部分代码

`packages/opencode/src/provider/provider.ts` 里 `BUNDLED_PROVIDERS` 是一张 npm 包名到工厂函数的表(`@ai-sdk/anthropic`/`@ai-sdk/openai`/`@ai-sdk/amazon-bedrock`/`@ai-sdk/google-vertex`/`@openrouter/ai-sdk-provider`/`gitlab-ai-provider` 等约 20 个),这些包本身就是符合 Vercel AI SDK `LanguageModelV3` 接口的第三方实现;不在这张表里的 provider(来自 models.dev 目录但没预打包)靠 `Npm.add()` **运行时动态安装**对应的 npm 包再 `import()`。模型元数据(价格、上下文窗口、是否 deprecated)来自外部目录 `@opencode-ai/core/models-dev`(即 models.dev,一个独立维护的开源模型数据库),不是 OpenCode 自己录入。

真正需要手写的是 `custom()` 里针对具体 provider 的怪癖处理:Azure 要在 `resourceName`/`baseURL`/`useCompletionUrls` 之间做优先级判断并选择 `chat`/`responses`/`languageModel` 三种端点之一;Bedrock 要处理 profile/access key/bearer token/web identity token 四种鉴权方式的优先级、还要按 region 前缀(`us.`/`eu.`/`global.`)决定要不要给 modelID 加跨区推理前缀;GitHub Copilot 要按模型代数(`gpt-5` 以上且非 mini)决定走 `responses` 还是 `chat` 端点;Google Vertex + Anthropic 组合要手工拼 `aiplatform.{region}.rep.googleapis.com` 这种区域化 endpoint URL。**这些判断加起来才是这层抽象真正值钱的部分**——统一接口本身是 Vercel AI SDK 免费提供的,OpencCode 自己的工程投入集中在"每个供应商在鉴权、endpoint 选择、跨区路由上到底有什么不一样"这件事上。

**对 Dozer 的意义**:这是"agent 本身"范畴里第一次看到"provider 抽象"被拆解到这个细度,回答了任务里问的"这层抽象是不是最值钱的资产"——答案是:**接口统一不值钱(社区已经有),处理每个供应商私有怪癖的知识才值钱**,而且这类知识高度易腐(供应商随时改鉴权方式、改端点)。这对 Dozer 没有直接的代码借鉴点(Dozer 不做模型接入),但对"以后要不要接入除 Claude Code 外的第二个 agent CLI(比如 Codex)"这个问题有参照意义:真正的成本不在"写一个通用 adapter 接口",而在于每个 CLI 私有的启动参数/鉴权/hook 事件格式差异,这块工作量不会因为抽象设计得好而消失。

## 核心机制四:权限/审批模型——运行时可阻塞的细粒度规则引擎,是第四条路线

`packages/opencode/src/permission/index.ts` 的核心是一个 `evaluate(permission, pattern, ...rulesets)` 函数:每条规则是 `{permission, pattern, action}` 三元组(`permission` 是工具类别如 `edit`/`bash`/`task`,`pattern` 是具体对象如文件路径/命令前缀/子 agent 类型),用 `Wildcard.match` 做双重匹配,`action` 取 `allow`/`ask`/`deny`,规则来自全局配置 + agent 定义 + session 级 + 子 agent 派生,按声明顺序 `findLast` 取最后一条命中的规则(后声明覆盖先声明)。命中 `ask` 时不是简单弹窗——用 Effect 的 `Deferred` 把整个工具执行**真正挂起**,直到 UI 侧调用 `reply()`;`reply` 支持 `once`/`always`/`reject`,`reject` 可以带一段 `message` 文本,包成 `CorrectedError` 原样喂回模型(模型据此重新组织下一步,而不是被静默拒绝)。`always` 生效时还会反向检查其余 pending 请求,同 pattern 的一并放行,减少连续弹窗。

对 `bash` 工具单独多一层:`tool/shell.ts` 用 `web-tree-sitter`(bash 语法树)解析命令,提取涉及的目录/文件路径喂给 permission pattern;`permission/arity.ts` 有一张人工标注的"命令前缀粒度表"(比如 `git` 记 2 个 token、`npm run` 记 3 个 token),决定用户点"总是允许"时到底该记住 `npm run dev` 这么细还是 `npm run` 这么粗——这是一个没有在 jcode/seek_code/t3code 里见过的具体细节,解决的是"允许规则该多精确"这个纯 UX 问题。**但要清楚地指出这套系统缺什么**:通篇没有类似 jcode `RiskLevel::Catastrophic` 那种"无论用户规则怎么写、无论传参怎么构造,都硬拒绝"的绝对底线;规则完全由用户/配置决定,并且整个 permission 系统可以被 `--yolo`/`--dangerously-skip-permissions` 命令行参数整体跳过(`cli/cmd/run.ts`/`tui.ts` 里直接写着 `describe: "...(dangerous!)"`)。

**对 Dozer 的意义**:回答任务里问的"这是第几条路线"——**这是一条新的第四条路线,和已知三条都不同**:不是 jcode 的"自动语义分级+强制拒绝底线",不是 seek_code 的"命令内容正则匹配",也不是 T3 Code 的"直接暴露被驾驭 CLI 自带的沙盒开关"。OpenCode 这条路线的本质是"**自己就是工具执行者,所以能做到真正的运行时阻塞等待人工确认**,规则本身是用户配置的模式匹配而非自动风险判断"。这和 jcode 的 `jcode-tui-permissions`(阻塞式审阅队列)其实解决的是同一个问题(执行前拦截+等真人),但 OpenCode 把"规则该怎么写"这件事做得更细(双维度 wildcard、AST 提取模式、reject 时可附反馈文本);同时它印证了 jcode 分析里记的那句话——"Dozer 只能通过 `PreToolUse` hook 退出码这条更弱的路径拦截,因为 Dozer 不是执行者本身"——OpenCode 用一个 190K star 的主流项目又确认了一次:**细粒度的执行前拦截,前提是拦截者本身拥有执行权**,Dozer 没有这个前提,这不是能靠"抄一份好的规则引擎设计"绕开的结构性限制。

## 核心机制五:会话持久化与 resume——SQL 数据库承载,重启即接回

`packages/opencode/src/session/session.ts` 用 `drizzle-orm` 定义 `SessionTable`/`PartTable`/`ProjectTable`,落在真正的 SQL 数据库(`@effect/sql-sqlite-bun`,Bun 内建 SQLite),不是 jcode/seek_code 那类 JSON 文件存储。`Info` schema 里记了 `parentID`(fork 树)、`revert`(指向某条消息/snapshot/diff 的撤销指针)、`permission`(整条 session 级规则集)、`cost`/`tokens`(累计用量)、`time.archived`。

Resume 的实际路径在 ACP 层看得最清楚(`packages/opencode/src/acp/service.ts`):`loadSession`/`resumeSession`/`forkSession` 全部先 `sdk.session.messages(...)` 把该 session 的历史消息从服务端拉回来,再用 `restoreFromMessages()` 从最后一条带 model 信息的用户/助手消息里反推出当时用的 model/variant/mode,重新构造一个 `state` 接着跑——**resume 不依赖任何内存态,完全靠服务端持久化的消息记录重建**,进程重启、client 换一个都能接上。`resumeSession` 额外只拉最近 20 条做快速恢复,`listSessions` 支持跨目录(`roots: true`)列出全部 session 并按更新时间游标分页。

**对 Dozer 的意义**:这是三个"agent 本身"项目里第一次看到**用真正的关系型数据库**做会话存储(jcode/seek_code 具体存储机制本系列未深入到这个细度),SQL 表 + 显式 schema 演进(`fromRow`/`toRow` 转换函数)比裸 JSON 文件更经得起字段增删的历史包袱。`dozerd` 的 `AcceptanceStore` 目前存储机制如果还是文件式的,这里给了一个"要不要上真正的嵌入式 SQL(比如 `rusqlite`)"的具体参照点——尤其 Dozer 已经有"验收记录需要长期可查、可跨会话检索"这个需求,和 OpenCode 的 session 持久化诉求是同类问题。

## 核心机制六:子任务/Sub-agent(`task` 工具)——fan-out/fan-in,不是 jcode 式 swarm

`packages/opencode/src/tool/task.ts` 里 `task` 工具启动子 agent 的方式是:创建一个新 session(`parentID` 指向当前 session),子 session 的权限从父 session 权限 + 子 agent 自身权限定义派生(`deriveSubagentSessionPermission`),并强制拒绝子 agent 使用 `todowrite`/`task`(除非其权限定义里显式声明允许)——**防止子 agent 无限递归开子子 agent**,配合 `cfg.subagent_depth`(默认 1)做深度上限。子 agent 跑完后结果通过 `renderOutput()` 包成 `<task>` XML 块塞回父 session 的下一条消息,期间没有任何"父子 agent 互相看对方文件改动、互相发消息"的机制。有一个实验性的"后台模式"(`background: true`,需要 `OPENCODE_EXPERIMENTAL_BACKGROUND_SUBAGENTS` 环境变量开关):子 agent 异步跑,完成后通过一条 `synthetic: true` 的文本消息注入回父 session,提示词里明确写"不要 sleep/轮询/重复问进度"。

**对 Dozer 的意义**:这个拓扑结构和 seek_code 的 `subagents.ts`(fan-out/fan-in,无跨 agent 通信)是同一类,和 jcode 的 swarm(agent 间 DM/广播、"代码在脚下位移"实时感知)是不同类——**这是本系列第二次验证"fan-out/fan-in 无跨 agent 通信"是主流做法,jcode 的 swarm 通信层目前是三个"agent 本身"项目里的少数派**,不是行业共识。二期设计"多 agent 协作"时,这条计数应该更新:2(OpenCode/seek_code)对 1(jcode)支持"隔离子任务、不做实时跨 agent 通信"这个更保守的模型,jcode 自己的 README 也承认这是它的反主流立场。深度限制(`subagent_depth`)和递归工具屏蔽(拒绝子 agent 再调 `task`/`todowrite`)这两个具体防护点,是 Dozer 二期如果做子任务编排时可以直接参照的最小防护集。

## 其他验证点(简述)

- **上下文压缩**(`session/compaction.ts`):按"回合"(turn,一问一答为一组)从最近往前累加 token 预算,超出部分交给一个专门的 `compaction` agent/model 生成摘要,同时有独立的"裁剪"(prune)机制在摘要之外把更早的、已完成的工具输出内容清空(按 token 预算保护最近 N token 不裁)。和 jcode 的"80%/95% 阈值 + 双档压缩"比,OpenCode 的选择更"按对话轮次语义分段"而非纯阈值触发,细节不同但解决的是同一类问题——**再次确认压缩/裁剪是"agent 本身"范畴的标配子系统,不是 jcode 独有**。
- **Checkpoint**(`snapshot/index.ts`):隐藏 git ref 快照 + `diff`/`revert`,止步于"能不能撤销重来",没有 verdict/goal/acceptor 这类用户可见的验收元数据——和 T3 Code 分析的结论完全一致,**这次是在"agent 本身"范畴里又确认了一遍**:六/七个分析对象里仍然没有一个做到 Dozer 现有的验收闭环深度。
- **Worktree**:`control-plane/adapters/worktree.ts` 是一个可插拔的 `WorkspaceAdapter` 实现,创建 git worktree 承载会话——"一 session 一 worktree"这个此前只在编排层(kooky/orca/workmux)项目里验证过的标准件,现在在"agent 本身"范畴里也验证了一次,是更普遍的共识,不是编排层特有的模式。

## 对 Dozer 的启示汇总

| OpenCode 机制 | 对应 Dozer 位置/路线图 | 借鉴点 |
|---|---|---|
| HTTP(OpenAPI)+SSE 为主协议,WebSocket 只管 PTY 面板;ACP 适配层与 TUI/Web 对等,同一套 HTTP API | `dozer-app`↔`dozerd` 的 UDS 协议;未来若要给编辑器插件暴露接口 | "控制面单向推送用 SSE、双向交互才用 WebSocket"的拆分是可直接参考的架构判断;ACP 是已有两个项目从两端验证过的现成跨 vendor 协议 |
| 默认路径交给 Vercel AI SDK 处理协议细节,自己只管工具分发/治理;有一条尚未转正的"自建 native 运行时"暗线 | Dozer 的"agent 中立"定位(不做模型循环) | 非直接借鉴,是"provider 覆盖面 vs 运行时可控性"这条工程取舍的又一实例,印证 jcode 分析里"包一层有天花板"的观察不是唯一解读——OpenCode 选的是相反的权衡且活得很好 |
| Provider 抽象的真实资产是处理供应商鉴权/endpoint/跨区路由怪癖的代码,不是统一接口本身 | 未来是否接入第二个 agent CLI(如 Codex)的成本预估 | 统一接口设计得好不好不决定成本,每个 CLI 私有差异的知识量才决定成本,这块投入无法靠抽象设计规避 |
| 细粒度 wildcard 规则引擎(permission×pattern→allow/ask/deny)+ Effect Deferred 真阻塞 + AST 提取模式 + reject 可带反馈文本;但无绝对拒绝底线,可被 `--yolo` 整体跳过 | spec 里"验收/治理"定位,命令风险门禁待办(与 jcode/seek_code/t3code 并列的第四条路线) | 确认这是全新第四条路线;也再次印证"执行前拦截前提是自己是执行者",Dozer 目前只能走 `PreToolUse` hook 退出码这条更弱的路径,是结构性限制不是设计取舍 |
| Session 落 SQL 数据库(`SessionTable`/`PartTable`),resume 完全靠服务端持久化消息重建(不依赖内存态) | `dozerd` 的 `AcceptanceStore` 存储机制 | 若目前是文件式存储,这是"上真正的嵌入式 SQL"的具体参照点,尤其验收记录需要跨会话长期可查这个诉求和 session 持久化同类 |
| `task` 工具 fan-out/fan-in 子 session,深度限制 + 递归工具屏蔽,无跨 agent 通信 | 路线图二期"编排与资产" | 再次确认 fan-out/fan-in(而非 swarm)是主流(2:1),深度限制和递归屏蔽这两个具体防护点可直接参照 |
| 压缩按"回合"语义分段 + 独立裁剪阈值;checkpoint 止步于"快照+diff",无验收元数据;worktree 是可插拔 adapter | 三期"记忆"路线图;Dozer 现有验收闭环定位 | 压缩机制细节可与 jcode 的阈值方案并列参考;验收闭环的差异化定位在"agent 本身"范畴里被二次确认为真空;worktree 标准件的普遍性又提高一档 |
