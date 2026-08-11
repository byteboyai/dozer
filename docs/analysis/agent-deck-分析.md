# Agent Deck 代码级分析

> 分析对象:https://github.com/asheshgoplani/agent-deck (本地副本 `/Users/chrischiang/AI/agent-deck`,`--depth 1` 浅克隆,当前机器上多个并发 clone 任务导致带宽紧张,耗时较久)
> 分析日期:2026-07-28 · Go 36.9 万行(不含测试)+ 20.1 万行测试 / 1,335 个 `.go` 文件(895 个 `_test.go`)/ 5,885 个 `func Test` / MIT · 613 star,102 fork,开源 8 个月(2025-12-03 建仓)

## 一句话定位

Agent Deck 是**驾驭 Claude Code/Codex/Gemini/OpenCode/Cursor/Pi 等多个 agent CLI 的 Go + Bubble Tea TUI 会话管理器**——终端渲染和会话存活完全外包给 tmux(与 workmux 同构,不自研终端引擎),自己的核心价值是"一张牌桌上看清所有 agent 在干什么"。它区别于此前六个分析对象的差异化卖点是 **`session fork`**:声称"分叉一个会话,新会话继承父会话的完整对话历史,而不是像 workmux/kooky 的 `--resume` 那样接回同一个会话"。核实结论见"核心机制一"——这个卖点是真的,但实现方式和直觉预期不同:**Agent Deck 自己完全不碰对话历史,fork 出的会话历史续接 100% 委托给被驾驭 agent CLI 自带的原生 fork/resume 命令**(`claude --resume <id> --fork-session`、`codex fork <id>`、`opencode -s <id> --fork`、`pi --fork <jsonl路径>`),Agent Deck 的自研代码只解决"新会话该不该有一份独立的、包含父会话未提交改动的工作目录"这个正交问题(git 工作树物化)。

体量上,36.9 万行非测试代码 + 20.1 万行测试代码,测试代码比生产代码还多——这是七个分析对象里测试密度最高的一个(测试行数/生产行数 ≈ 1.19,workmux 是六个项目里此前的密度冠军但也只到"1,532 测试/7.6 万行"这个量级,行数比例上 Agent Deck 更极端)。项目由 [asheshgoplani](https://github.com/asheshgoplani) 一人主导(contributors 接口显示其贡献占比压倒性),8 个月内打了上百个 tag(`v1.9.x` 到 `v1.10.11` 密集连续发布,像是每个通过 CI 的 PR 就切一个版本),属于"单人高频迭代 + 测试驱动"的工程文化,而不是团队协作的产物。

## 工程结构

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

## 核心机制一:`session fork`——委托给被驾驭 CLI 原生命令,不做对话重放,也不是写时复制

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

## 核心机制二:会话持有——遥控 tmux,与 workmux 同构但独立实现

`internal/tmux/pty.go` 只用 `creack/pty` + `golang.org/x/term` 做**本地终端到 tmux pane 的 attach/detach**(处理 detach 快捷键的三种编码:原始字节、xterm modifyOtherKeys、kitty CSI u),`internal/tmux/tmux.go` 则是对 `tmux list-windows`/`has-session` 等命令的封装。也就是说:**agent CLI 进程本身跑在 tmux 窗口里,tmux server 才是真正的会话持有者**,Agent Deck 进程退出对存活会话无影响——这与 workmux "不自研终端引擎,遥控 tmux/Zellij/WezTerm/kitty"的架构结论完全一致,只是 Agent Deck 只驾驭 tmux 一种后端(未见 Zellij/WezTerm/kitty 支持),换来的代价是不需要 workmux 那层 `Multiplexer` trait 抽象。

会话状态从早期的单文件 JSON(`session.StorageData`,`internal/statedb/migrate.go` 里仍保留迁移代码)升级成了 SQLite(`modernc.org/sqlite`,纯 Go 驱动,无需 CGO)。`internal/statedb/statedb.go` 里的 `withBusyRetry` 处理并发写入的 `SQLITE_BUSY`,`backupDBFile` 在 schema 迁移前先备份,`saveInstancesOnce` 里显式防"空扫场景"（sweep 相关注释:"empty sweep turns silent data loss into a loud, recoverable error"）——即批量保存时如果传入的实例列表意外为空,不能被当成"全部清空"来执行,必须报错而不是静默清空数据库。这是一条与 kooky/workmux"运行时字段不落盘、磁盘值不可信、损坏即删"不同的容错哲学:**SQL 事务 + 显式防御性断言**,比"单文件损坏就删"更重,但换来的是并发多进程写同一份状态时不会互相踩踏。

**对 Dozer 的意义**:再次印证"自持 PTY 是重路线、遥控宿主终端复用器是轻路线"这条 workmux 已经给出的结论,Agent Deck 是第二个独立收敛到同一选择的项目,进一步降低了这条经验的偶然性。状态存储从"单 JSON 文件"到"SQLite"的迁移路径,对 `dozerd` 未来如果要解决"`SessionRegistry` 纯内存 HashMap、进程重启全丢"这个已知缺口时是一个可考虑的方向——但要注意 Dozer 目前的容错哲学(运行时字段不落盘)和这条路线的前提(把几乎所有状态都塞进一个可查询的数据库)是两种不同的设计取舍,不能不假思索照搬。

## 核心机制三:MCP 集成——跨会话共享 MCP 进程的 socket 代理池

`internal/mcppool/socket_proxy.go`(642 行)是这次七个分析对象里第一次见到的、有实际工程深度的 MCP 集成方案。核心问题:如果每个 agent 会话(每个 tmux pane)都独立起一份 MCP server 子进程,MCP server 数量随会话数线性增长,启动开销和资源占用都不划算。Agent Deck 的解法:

- `SocketProxy` 包一个 stdio MCP 子进程,对外暴露一个 Unix domain socket;多个客户端(多个 agent 会话)连到同一个 socket,请求经代理转发给底层唯一的 MCP 进程
- **JSON-RPC ID 重写**:每个进来的请求 ID 被替换成代理自己维护的单调计数器(`nextID atomic.Int64`),同时记录 `idMapping{sessionID, originalID}` 存入 `sync.Map`,响应回来时按重写后的 ID 查到原始请求属于哪个 session、原始 ID 是什么,再换回去发给对应客户端——这是标准的多路复用做法,保证多个会话共用同一个 MCP 进程时 ID 空间不冲突
- `stdinMu` 序列化对 MCP 进程 stdin 的写入,防止并发请求把 JSON 行写乱掉(帧同步问题)
- `waitOnce sync.Once` 保证子进程只被 `Wait()` 收尸一次,源码注释提到 v1.7.43 之前 `broadcastResponses` 检测到 MCP 进程 stdout EOF(进程死了)但没有正确 reap,导致僵尸进程一直挂到 `Stop()` 被调用(可能永远不被调用)——这是一个真实修复过的资源泄漏 bug
- `internal/mcppool/pool_simple.go` 的 `PoolConfig` 支持 `PoolAll` + 排除名单或白名单两种模式来决定哪些 MCP server 走池化、哪些保留独立 stdio 生命周期

**对 Dozer 的意义**:这是六个先前分析对象里都没有深入讨论过的一块,Agent Deck 给出了目前样本里唯一"多会话共享同一 MCP 进程"的具体实现。如果 Dozer 未来要做 MCP 集成(spec 目前未定),这套"socket 代理 + JSON-RPC ID 重写 + 单调计数器 + sync.Once 收尸"的模式是一份可直接参考的最小实现;`waitOnce` 那个真实修复过的僵尸进程 bug 也提醒:**转发型 MCP 代理必须显式设计子进程收尸路径**,不能假设 EOF 检测等价于进程已回收。

## 核心机制四:多 provider 抽象——一个 9,371 行的巨对象,不是契约层

任务要求核实"多 provider 会话模型怎么统一抽象",结论:**很薄,而且不是显式契约层**。对照此前分析过的 t3code(`packages/contracts` 独立包,WebSocket 协议的类型化契约,client/server 共享)和 workmux(单个 `Multiplexer` trait,~30 方法,四个后端各自实现),Agent Deck 走的是完全不同的路:

- 单个 `Instance` struct(`internal/session/instance.go:115`)同时容纳所有 provider 的专属字段:`ClaudeSessionID`/`ClaudeDetectedAt`、`CodexSessionID`/`CodexDetectedAt`、`OpenCodeSessionID`/`OpenCodeDetectedAt`、`GeminiSessionID`……每加一个 provider 就往这个 struct 里加一组字段,而不是定义一个 `Session` 接口让各 provider 实现
- 判断"能不能 fork"“怎么 fork”全靠字符串比较分支(`if i.Tool == "opencode"`、`if i.Tool == "pi"`、`IsCodexCompatible(i.Tool)`)加一族并列的 per-provider 方法(`CanFork`/`CanForkOpenCode`/`CanForkPi`/`CanForkCodex`,`buildClaudeForkCommandForTarget`/`buildCodexForkCommandForTarget`/`buildPiForkCommandForTarget`),没有一个共同的 `Forkable` 接口把这些方法收拢起来
- 连"识别一个命令字符串属于哪个 tool"这种基础功能都是最近才收敛的:`internal/session/builtins.go` 的注释直接写明,在此之前这段逻辑分裂在两个手工同步的函数里(`detectTool()` 和 `isBuiltinToolName()`),issue #1258 才把它们合并成一份 `builtinTools()` 数据表——**项目自己也在事后重构掉重复代码**,不是一开始就设计好的
- 目前登记在册的 built-in tool 有 11 个:claude/opencode/gemini/codex/pi/copilot/crush/cursor/hermes/aider/shell,其中 "aider" 有名字但没有探测规则(遗留的不对称,注释里原样承认),"shell" 是兜底分类

**对 Dozer 的意义**:这是一次有效的反例确认,不是借鉴点——**规模大、迭代快的项目不必然收敛出干净的抽象层**;t3code 的 contracts 包和 workmux 的 trait 都是刻意设计的结果,Agent Deck 证明"不做这层设计"同样能撑起一个 600+ star、11 个 provider 的项目,只是代价是核心文件涨到 9,371 行、判断逻辑散在几十个 `if Tool ==` 分支里,且连"命令字符串识别 tool"这种基础工具都要事后专门重构一次去重。Dozer 目前只适配 Claude Code 一家,CLAUDE.md 的"agent 中立"是北极星而非现状——真到了要适配第二个 agent CLI(如 Codex)时,这份反例提醒:**趁早为"provider 专属字段/方法"设计一层薄接口(哪怕只是一个 `AgentAdapter` trait),比等到规模上来后再重构一个巨对象成本低得多**。

## 核心机制五:Hook 端点 + 环境变量路由——与 workmux 几乎同构

`internal/session/claude_hooks.go`(`InjectClaudeHooks`/`RemoveClaudeHooks`/`mergeHookEvent`)的实现模式与 workmux 的 `agent_setup/hooks.rs` 高度相似:按事件名分组的 JSON 树,`mergeHookEvent` 做语义合并而非整体覆盖写,`hooksAlreadyInstalled`/`eventHasAgentDeckHookMatchingConfig` 保证安装操作幂等。会话身份路由靠 `AGENTDECK_INSTANCE_ID` 环境变量在拼 shell 命令时内联注入(`instance.go:984` 等多处),Claude 的 hook 子进程读这个变量识别自己属于哪个 Agent Deck 会话——和 Dozer 的 `DOZER_SESSION_ID`、kooky 的 `KOOKY_SURFACE_ID` 是同一形状的方案。除 Claude 外,gemini_hooks_cmd.go/cursor_hooks_cmd.go/codex_hooks_cmd.go/hermes_hooks_cmd.go 分别对应各家 CLI 自己的 hook 配置格式,是"每个 provider 一个安装器"而非统一抽象,呼应核心机制四的结论。

**对 Dozer 的意义**:这是七个分析对象里第三次独立验证"hook 配置必须做语义合并/精确摘除,不能覆盖写整个配置文件"这条纪律(kooky、workmux 之后的第三例),`dozer-hook` 的 `install.rs` 如果尚未做到"合并 + 摘除 + 空壳清理"三件套的完整覆盖,这条经验的说服力又增加了一份独立样本。

## 对 Dozer 的启示汇总

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

## 未能本地核实的部分

- Claude Code 是否真的把 `--fork-session`/`--resume` 当作官方稳定接口维护(本文只核实到 Agent Deck 如何调用,未核实 Claude Code 官方文档对该标志的稳定性承诺)
- Agent Deck 是否有 Zellij/WezTerm/kitty 等 tmux 之外的多路复用器后端支持——浅克隆 + 关键词搜索未发现相关文件,倾向于判断为"无",但未做穷尽式确认
- `internal/fleet/`、`internal/openclaw/`、`internal/selfheal/`、`internal/watcher/`、web 终端桥接(`internal/web`)等外围子系统只看了目录规模,未展开分析——这些不在任务要求的"标准件"清单范围内,体量合计约 3.5 万行,如果后续需要可以单独立项深挖
- 项目整体 commit 数量(浅克隆只能看到 1 个 commit;`gh api contributors` 显示主贡献者 1,652 次贡献,但这是否等于总提交数未交叉验证)