# Garcon 代码级分析

> 分析对象:https://github.com/cfal/garcon(本地副本 `/Users/chrischiang/AI/garcon`,`git clone --depth 1` 一次成功,未遇到大仓卡住的问题)
> 分析日期:2026-07-28 · TypeScript/Svelte 34.6 万行 / 约 1,900 源文件 / 665 个 `*.test.ts` 测试文件(另有 `integration-tests/` 黑盒集成套件,1.6 万行,不计入上面的单元测试数)· LICENSE 文件正文为 GPL-3.0,但 GitHub 的 SPDX 识别结果是 `Other`/`NOASSERTION`(license 文件头部有大段附加条款,不是纯 GPL-3.0 模板,识别器判不准,按仓库自己的说法是 GPL-3.0)· 41 star / 7 fork(2026-02-23 创建,是七个分析对象里最年轻、体量最小的一个,仍在活跃推送)

## 一句话定位

Garcon 是"a cross-platform agentic coding UI for Claude Code, Codex, and Opencode"(`package.json` description),实际支持面更广:Claude Code、Codex、Cursor Agent、OpenCode、Amp、Factory Droid、Pi 七个 agent CLI,外加直连 Anthropic/OpenAI 兼容端点——和 kooky/orca/workmux/t3code 同一范畴的**编排层**:自托管 browser+mobile 工作区,并行跑多个 agent 会话,不自己重造模型循环。技术栈是 **Bun + SvelteKit(Svelte 5)**,单一 Bun HTTP/WebSocket server(`server/`)作为唯一执行边界,`web/`(SvelteKit,17.1 万行,全仓库最大的一块)是前端,没有独立桌面壳、没有原生移动 app——"手机端"是这个 SvelteKit 应用的 PWA 形态(`web/static/site.webmanifest` + service worker),不是 t3code 那种 Swift/Kotlin 原生客户端。

七个分析对象里,Garcon 和 Dozer 定位重叠度最高:都做"审批"、都碰"git 工作流"。但实测下来,这两处重叠都是**表层重叠、深度不同**——Garcon 的"移动审批"是工具调用级别的 allow/deny 推送(和 Claude Code 自己的 permission-prompt 一一对应),"PR 工作流"是`gh` CLI 的只读包装层(明确写在代码注释里),都不构成 Dozer 那种"目标/标准/verdict/accepted ref"的验收闭环。真正有价值的发现在别处:Garcon 把"结构化协议优先、PTY 只做终端展示"这条路线,在**七个 agent CLI 上全部验证了一遍**,是七个分析对象里这条标准件覆盖面最广的一次实测。

## 工程结构

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

## 核心机制一:七个 agent CLI 全部走"结构化子进程管道",PTY 只留给用户终端面板

逐个查了 `server-agents/*/src` 下的传输层:

- **Claude**:`server-agents/claude/src/agents/claude/cli-invocation.ts` 的 `buildClaudeCLIArgs` 拼的是 `claude --print --output-format stream-json --input-format stream-json --replay-user-messages --verbose --permission-prompt-tool stdio`,resume 靠 `--resume=<id>`——走的是 Claude CLI 自己的 JSON 流协议,不是解析终端字节流。
- **Codex**:`server-agents/codex/src/agents/codex/app-server/{client,protocol,runtime,converter}.ts`,和 T3 Code 用的是同一个东西——`codex app-server` 的 JSON-RPC over stdio。
- **Cursor**:`server-agents/cursor/src/acp/client.ts` + `agents/shared/acp-agent-runtime.ts`,走 **Agent Client Protocol(ACP)**——和 T3 Code 的 `packages/effect-acp` 对接的是同一套跨 vendor 标准。
- **Amp/Factory/Pi**:`server-agents/{amp,factory,pi}/src/.../{amp-cli,factory-cli,pi-cli}.ts` 全部是 `Bun.spawn([binary, ...args], {stdout: 'pipe', ...})` + 各自的 `--output-format`/`--stream-json-thinking` 参数,spawn-per-turn(`amp-cli.ts` 文件头注释明写"Uses a spawn-per-turn model: each user message spawns a fresh `amp` process")。

七个适配器,没有一个是"起一个 PTY、往里敲字、拿 OSC/ANSI 扫结果"的路数。**PTY 只在一个地方出现**:`server/terminals/terminal-manager.ts` 用 `bun-pty`(`import type { IPty } from "bun-pty"`),这是给用户看的"打开一个终端"面板,和驾驭 agent 完全是两回事——两条路径在代码里物理隔离,没有混用。

**对 Dozer 的意义**:这直接把"新 agent CLI 先查结构化协议、没有才退回 PTY+hook"这条准则(T3 Code 验证过一次)又验证了一遍,而且覆盖面更大——T3 Code 当时只有 Codex 一个 provider 真正跑通、Claude Code 是"contracts 里占位但未实现",Garcon 是七个 CLI**全部**在生产可用的状态下走通了这条路线,包括 Claude Code(`stream-json` + `--permission-prompt-tool stdio`)。这对 Dozer 有直接参考价值:Dozer 目前对 Claude Code 走的是 PTY + hook + OSC 扫描,是因为规格阶段判断 Claude Code 没有对等的结构化协议;Garcon 的 `cli-invocation.ts` 证明 **Claude Code 其实有 `--output-format stream-json`/`--permission-prompt-tool stdio` 这条路**,如果 Dozer 未来要重新评估"能不能少依赖 PTY+OSC 消毒这一套",这是一份现成的、被验证过的实现参考(尤其是前面刚记过的"macOS GB18030 字体毒化"问题,根源就在终端渲染层——如果控制通道能换成结构化 JSON 流,这类问题的影响面会显著缩小,但這是一个需要重新评估的架构级决策,不是本次分析要下的结论)。

## 核心机制二:审批推送是真实的双向闭环,但"移动端"是 PWA、Telegram 只是单向提醒

审批(工具调用许可)链路是本次任务要求重点核实的地方,查到的实现相当完整:

1. Claude 侧:`server-agents/claude/src/agents/claude/claude-cli.ts` 里 `#pendingPermissions`(第 470 行起)在收到 CLI 通过 `--permission-prompt-tool stdio` 回调的许可请求时,生成 `permissionRequestId`(第 506 行),转换成 `PermissionRequestMessage` 并塞进这个会话的消息流(第 527-542 行)。
2. 消息类型定义在 `common/chat-types.ts` 第 629-641 行:`PermissionRequestMessage`/`PermissionResolvedMessage`/`PermissionCancelledMessage`,和普通 `ChatMessage` 走同一个联合类型、同一条 WebSocket 通道推送(`server/ws/chat.ts`)——**推送不是轮询**,是和 transcript 同一条流里的一条消息。
3. 客户端(浏览器或手机 PWA)调用许可决定后,回到 `claude-cli.ts` 第 721 行 `resolveInternalToolApproval(permissionRequestId, decision)`,兑现 pending 的 promise,再把结果转成 `PermissionResolvedMessage` 回推(第 744 行)。
4. 超时/放弃语义存在:回合结束、被中止、会话完成时,所有还挂着的许可请求会被批量清空并广播 `PermissionCancelledMessage`(第 470-474、549-554 行),reason 分 `cancelled`/`session-complete`/`aborted` 三种。

但"移动端"和"Telegram 通知"这两处容易被 README 的措辞误导,查代码后要纠正:

- **没有原生 iOS/Android app**。`web/static/site.webmanifest` + service worker(`web/src/service-worker-helpers.ts`)说明这是一个装到手机主屏幕的 **PWA**,底层还是同一个 SvelteKit 应用走同一条 WebSocket——不是 T3 Code 那种带 Swift/Kotlin 原生代码的独立客户端。
- **Telegram 是单向提醒,不能拿来审批**。`server/notifications/telegram.ts` 只调了 `sendMessage`(第 176 行),全文没有 `inline_keyboard`/`callback_query`,`server/notifications/attention-tracker.ts` 的注释也自己写明"Permission request - immediate, deduped by permissionRequestId"(第 5 行)是拿来去重提醒用的,不是审批通道本身——真正点"允许/拒绝"还是要回到 PWA 或浏览器里,Telegram 只是"告诉你有事要处理"。

**对 Dozer 的意义**:这是一份"工具调用许可"场景下推送架构的可用参考(单一消息流承载 transcript + 许可事件、超时/中止有显式广播语义),但**这不是 Dozer 的验收闭环要学的东西**——Garcon 这里做的是"这一步工具调用能不能执行"的运行时门禁,和 Dozer `.dozer/goal.md` + `AcceptanceStore` 要回答的"这一轮/这个任务算不算通过验收"是两个不同层次的问题,前者是执行期的许可,后者是交付期的判定。README 里"approve a blocked step"说的就是前者,不要因为字面上出现"approval"就当成和 Dozer 验收闭环同构。

## 核心机制三:Git/PR 工作流——worktree 管理是真的,PR 支持明确是只读薄壳

`server/git/worktrees.ts` 里 `createWorktreeOperations().getWorktrees` 真的调 `git worktree list --porcelain`(第 71-77 行),配合 `ref-validation.ts` 的 `assertSafeBranchName`/`assertExistingCommitRef` 做输入校验,是可用的 worktree 管理,不是摆设。`git/diff-engine.ts`(5 万行级)、`review-diff-batch.ts`、`review-document-service.ts` 支撑起 README 说的"stage individual lines, hunks, files, or folders"——这部分是做实的。

但 PR 支持这块,`server/routes/gh.ts` 文件顶部注释写得非常直白:"GitHub pull request routes. **Read-only viewer surface backed by the `gh` CLI**."(第 8 行)。逐一核对 `server/gh/gh-service.ts`:

- `getStatus` → `gh auth status --json hosts`
- `listPullRequests` → `gh pr list --state open --json <fields>`
- `getPullRequest` → `gh pr view` + `gh pr diff` + `gh api .../pulls/{n}/comments`(读 review thread)

全仓库搜索 `gh pr create`、`gh pr merge` 没有任何命中——**Garcon 不自动开 PR,不做合并,没有"合并前必须过审批"这类门禁**。README 里"turn pull request feedback into agent tasks"指的是把读到的 PR/评论内容转成一条发给 agent 的消息(UI 便利动作),不是自动化的 PR 生命周期管理。真正的"提交变更"停留在 git commit/push 这一层,PR 本身要么用户自己去 GitHub 网页开,要么手动敲 `gh pr create`。

**对 Dozer 的意义**:这是这次分析最值得记录的一条差异化确认——Garcon 是七个分析对象里在"PR/git 工作流"这一项做得**看起来**最丰富的(有 worktree、有行级 diff staging、有 PR 阅读器),但一旦查到 `gh.ts` 的注释和 `gh-service.ts` 的方法列表,就能确认它止步于"读 PR + 转发评论"这个薄壳,没有把"验收通过"和"能不能合并/能不能推"绑定起来。这进一步印证了 t3code-分析.md 里已经下的判断:六个(现在是七个)分析对象里没有一个把"验收"做成 Dozer 现在这个深度——Garcon 的 git 能力比 workmux/kooky 都强,但强的是操作面(worktree、diff、staging),不是治理面(谁批准的、依据什么标准、批准结果有没有落成可追溯的 ref)。Dozer 的 `refs/dozer/accepted/<n>` 目前仍然是七个项目里唯一把"验收结果"物化成 git 对象的设计。

## 核心机制四:跨 agent 会话转移——真实存在,但是"转录文字重放",不是原生上下文迁移

`server/agents/agent-switch-service.ts` 的 `AgentSwitchService#switchAgentCrossAgent`(第 40-100 行)是真实可调用的功能:校验目标 agent 不在运行中(第 44-46 行,`SESSION_BUSY` 门禁),加载源 agent 的渲染后 transcript(`source.transcript.load(...)`,第 48-53 行),再调用 `ownership.transfer(...)` 把这段历史存成一个 `CarryOverSegment` 交给新 agent。

关键在于新 agent 怎么"用"这段历史。`server/chats/chat-carryover-store.ts` 文件头注释直接写明设计意图(第 1-4 行):"A switch persists the outgoing agent's rendered ChatMessage[] as a segment so the conversation stays visible on reload **even though the new native session starts empty**."——也就是说,底层的 Claude/Codex/... 原生会话状态并不会被"迁移"过去,而是 `server-agents/claude/src/agents/claude/execution.ts` 第 82-84 行这么处理:

```ts
command: request.carryOver.length > 0
  ? `${renderTranscriptSeed([...request.carryOver])}\n\n${request.prompt}`
  : request.prompt,
```

把历史 transcript 渲染成一段文字,拼在新 agent 收到的第一条 prompt 前面——本质是"prompt 里塞一段历史摘要",不是结构化的上下文/状态迁移。

**对 Dozer 的意义**:这是任务里明确要求核实的点,结论是"功能真实存在,但比 README 字面读起来的深度浅"——"continue a conversation under another agent"做到了"让新 agent 看到一段文本形式的历史",但做不到"让新 agent 拥有和旧 agent 完全等价的内部状态"(这在 agent CLI 之间本来就是不可能的,因为 Claude/Codex/Cursor 各自的原生 session 格式互不兼容,Garcon 没有假装解决这个不可能的问题,只是老实地退到"文字重放"这条能落地的路)。对 Dozer 没有直接借鉴点(Dozer 目前 spec 没有"跨 agent 转移"这个需求),但作为判断准则记录下来:**遇到"跨 agent 无缝切换"这类宣传语,默认假设它是"transcript 转文字重放"这种务实实现,而不是真的做了状态级迁移**,除非验证到相反的证据。

## 核心机制五:resume id 持久化——带 owner 校验的编解码器,比"损坏即删"更严格

`server-agents/common/src/native-session/path-native-session.ts` 的 `createPathNativeSessionCodec(ownerId)` 把 `{path, agentSessionId, modelEndpointId}` 编码成一个带 `schemaVersion`(当前 1)和 `ownerId` 的 `AgentNativeSessionRef` 落盘。解码时(第 28-38 行)如果 `ownerId` 或 `schemaVersion` 对不上,直接抛 `AgentIntegrationError('TRANSCRIPT_UNAVAILABLE', ...)`,不是静默丢弃走默认值。

**对 Dozer 的意义**:这是"resume id 持久化"这条标准件的又一次独立验证(六个已分析对象里已多次见到),但在容错哲学上和"运行时字段不落盘、磁盘值不可信、损坏即删"这条 Dozer/已知标准件的默认做法**不完全一致**——Garcon 这里选择的是"格式对不上就报错并要求上层处理",而不是"静默删除退回空状态"。两种做法都合理,分别对应不同的失败代价假设(Garcon 认为"错误地把 A agent 的 native session 当成 B agent 的用"比"抛错打断用户"更危险,所以选择显式失败;"损坏即删"哲学假设的是"能继续跑比较重要,丢一个 resume id 影响有限")。记录为一个值得在 Dozer `dozerd` 做类似编解码器时权衡的分歧点,不是谁更优的定论。

## 核心机制六:权限模式——同样是"透传底层 CLI 自带旋钮"这条第三路线

`common/chat-modes.ts` 第 4-10 行定义的 `PERMISSION_MODE_VALUES = ['default', 'acceptEdits', 'manualBypass', 'bypassPermissions', 'plan']`,这几个值本身就是 Claude Code CLI 自己的术语(`default`/`acceptEdits`/`plan`/`bypassPermissions`)。`server-agents/common/src/execution/permission-modes.ts` 的 `providerStartupPermissionMode` 把它们映射回 Claude CLI 的 `--permission-mode <mode>` / `--dangerously-skip-permissions` 启动参数——没有自己的风险分级器,也没有自建沙盒,直接把底层 CLI 已经暴露的许可旋钮做成一个统一的下拉菜单。

**对 Dozer 的意义**:这是"命令风险门禁"三条已知路线里的第三条(T3 Code 验证过一次,这次在七个 CLI 上又验证了一遍,证据面更广)——工程量最小、前提是被驾驭的 CLI 自己带了权限参数。Dozer 排这块待办时,同样的判断准则再次被印证:先查 Claude Code/未来的 Codex 适配器自己暴露了哪些权限旋钮,能直接透传的不要重新发明分类器,只有 CLI 本身没给细粒度控制时才需要 jcode/seek_code 那种外挂方案。

## 未能本地核实/留有不确定性的点

- **用量/限额监控**:只在 Codex 侧(`server-agents/codex/src/agents/codex/app-server/protocol.ts` 第 65、431 行,`'usageLimitExceeded'`/`'usageLimited'`)看到透传自 `codex app-server` 的用量受限事件,Claude 适配器（`claude-cli.ts`）没有查到 usage/cost 字段的处理(只有 `num_turns`)。判断:Garcon 没有自建统一的跨 agent 用量监控仪表盘,现有的用量信号是"provider 自己上报什么就转发什么",覆盖不全,这一条留有不确定性,不排除某处还有未查到的用量统计代码。
- **OSC 7/133 shell 集成**:没有查到 Garcon 依赖 OSC 序列做 cwd/命令边界识别的证据(符合"agent 控制走结构化协议、不碰终端转义"的整体架构),但没有专门验证 `server/terminals/terminal-manager.ts` 这个用户终端面板本身是否处理 OSC 133,这块和"驾驭 agent"无关,不影响本次结论,故未深入查。
- **License 判定**:仓库 `LICENSE` 文件正文是 GPL-3.0(含额外条款),`package.json` 声明 `"license": "GPL-3.0"`,但 GitHub API 返回的 SPDX 识别结果是 `Other`/`NOASSERTION`——这是 GitHub 侧识别器的局限,不是仓库license本身有歧义,按 `package.json`/`LICENSE` 内容判定为 GPL-3.0。

## 对 Dozer 的启示汇总

| Garcon 机制 | 对应 Dozer 位置/路线图 | 借鉴点 |
|---|---|---|
| 七个 agent CLI(含 Claude Code)全部走结构化子进程 I/O(JSON 流/JSON-RPC/ACP),PTY 只留给用户终端面板 | `dozer-hook`/`dozerd` 的 agent 适配面 | "先查结构化协议、没有才退 PTY+hook"这条准则的第二次实证,而且证明 Claude Code 本身就有 `--output-format stream-json` + `--permission-prompt-tool stdio` 可用,是重新评估 Dozer 控制通道时的现成参考 |
| `PermissionRequestMessage`/`PermissionResolvedMessage` 走同一条 chat WS 流做工具许可推送,超时/中止有显式广播 | `dozer-client` 的连接层、`dozerd` 的会话事件流(如果未来要做运行时许可门禁) | 消息流承载许可事件而非单开一条轮询通道的设计可以参考;但明确不是验收闭环的替代品,不要混淆"工具调用许可"和"任务验收" |
| "移动端"是 PWA、Telegram 是单向提醒,不是审批通道 | Dozer 目前无移动端规划 | 纠偏参考:遇到"移动审批"字面描述时,先查是不是走同一个 web 前端的安装态,而不是假设有独立原生客户端 |
| Git worktree 管理 + 行级/hunk 级 diff staging 做实,PR 支持明确止步于 `gh` 只读包装(`server/routes/gh.ts` 自证注释) | Dozer 已有的验收闭环(goal/criteria/verdict/accepted ref) | 非借鉴点,是差异化再确认——操作面(worktree/diff/staging)做得比 workmux/kooky 都强,但治理面(验收判定物化成 git ref)七个分析对象里仍然只有 Dozer 做到 |
| 跨 agent 会话转移 = 渲染 transcript 转文字、拼进新 agent 首条 prompt(`renderTranscriptSeed`),新原生会话本身"从空开始" | 无直接对应(Dozer spec 未定跨 agent 转移需求) | 判断准则记录:"无缝切换 agent"类宣传语默认按"transcript 转文字重放"核实,不要预设有结构化状态迁移 |
| `PathNativeSessionCodec` 编解码 resume 引用,`ownerId`/`schemaVersion` 不符直接抛错而非静默丢弃 | `dozerd` 未来做类似编解码器时的容错哲学参考 | 和"损坏即删"的默认哲学形成对照组,记录为待权衡的分歧点而非定论 |
| `PermissionMode` 直接复用 Claude Code 自己的模式词汇(`default`/`acceptEdits`/`plan`/`bypassPermissions`) | 命令风险门禁待办(三条已知路线之一) | 工程量最小的治理路线在七个 CLI 上又跑通一次,强化"能透传就不重新发明分类器"的优先级 |
