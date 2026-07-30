# T3 Code 代码级分析

> 分析对象:https://github.com/pingdotgg/t3code (本地副本 `/Users/chrischiang/AI/t3code`,克隆耗时较长,期间以 `gh api` 逐文件拉取源码 + 全部架构文档并行核对)
> 分析日期:2026-07-28 · TypeScript 53.9 万行 / 1.3 万文件 / 2,224 测试文件(另有 Swift/Kotlin/C++ 原生模块)· MIT · 15,351 star,今日仍在推送

## 一句话定位

T3 Code(Theo/pingdotgg 出品)是"a minimal web GUI for coding agents"——和 kooky/orca/workmux 同一范畴:**驾驭已有 agent CLI**(Codex、Claude、Cursor、OpenCode),不自己重做模型循环,和 jcode/seek_code 相反。形态是 **Node.js WebSocket server 包一层 React web app**,桌面端是 Electron 壳,还有 iOS/Android 原生客户端(`apps/mobile`,含 Swift/Kotlin 代码)——四种前端(web/desktop/mobile/CLI `npx t3@latest`)共享同一个 server 作为唯一执行边界。

**诚实的范围说明**:README 写"currently Codex, Claude, Cursor, and OpenCode",但架构文档明确写"Codex is the only implemented provider. `claudeCode` is reserved in contracts/UI"——项目自己也承认"very very early, expect bugs, not accepting contributions yet"。分析时按"架构已经这么设计,但目前只有一个 provider 真正跑通"来读,不要当成四个 provider 都成熟。

体量上,53.9 万行 TypeScript 排在 jcode(70.7 万行)之后、超过 orca(39.4 万行),是六个分析对象里第二大,**七个分析对象共同标准件里唯一原生覆盖 web+desktop+mobile 三端**的编排层项目(orca 有手机伴侣端但核心是 Electron;kooky/workmux 单一前端)。按 `apps`/`packages` 拆分:`apps/server`(17.3 万行,核心执行边界)> `apps/web`(13.7 万)> `apps/mobile`(7.0 万,原生 iOS/Android)> `packages/effect-codex-app-server`(4.7 万)> `apps/desktop`(3.4 万,Electron 壳,体量远小于 server/web,印证"桌面只是薄壳")。

## 工程结构

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

## 核心机制一:Provider 接入——优先用结构化协议,PTY 是补充不是主力

`packages/effect-codex-app-server` 包装的是 **`codex app-server`**——Codex CLI 自带的 JSON-RPC over stdio 模式,不是解析终端字节流。`packages/effect-acp` 对接的是 **Agent Client Protocol**(Zed 发起的跨 vendor 标准,部分 agent CLI 已原生支持)。`apps/server/src/terminal` 目录仍然存在,大概率承担"给用户看的内嵌终端面板"这类辅助功能,而不是 provider 控制通道本身(未能本地核实到具体调用点,这里留有一定不确定性)。

**对 Dozer 的意义**:这是目前六个分析对象里唯一采用"优先用 agent CLI 自带结构化协议,PTY 只做补充"路线的项目。kooky/workmux/Dozer 全部走的是"PTY + hook + OSC 扫描"这条路,因为 Claude Code 没有类似 `codex app-server` 的官方 JSON-RPC 模式。这不是"Dozer 该改路线"的理由(Claude Code 目前确实没给这个接口),而是一条值得记住的判断准则:**遇到新 agent CLI 时,先查它有没有结构化协议(app-server 式 JSON-RPC 或 ACP),有就优先用,PTY 兜底**——T3 Code 的架构选择证明了这条准则已经被一个 15K star 项目验证过,不是理论空想。

## 核心机制二:远程架构——目前六个分析对象里最成熟的设计

`docs/architecture/remote.md` 是一份"architecture-first"的设计文档,核心是把"连不连得上"拆成四层正交概念:

- **`ExecutionEnvironment`**:一个运行中的 T3 server 实例,拥有 provider 状态、项目/线程、终端进程、git/文件系统——**跨端(web/desktop/mobile)共享的唯一概念**
- **`KnownEnvironment`**:客户端本地保存的"知道怎么连到某个环境"的记录,不是 server 权威的
- **`AccessEndpoint`**:连到某个环境的一条具体路径(直连 `wss://`、隧道、desktop 托管的 SSH 转发)——**同一个环境可以有多条路径,这是防止"SSH 接管整个模型"的关键抽象**
- **`AdvertisedEndpoint`**:server/desktop 主动建议的候选端点,客户端当提示用,不当"能连通"的证据
- **Endpoint providers**:可插拔的端点发现器,第一个实现是 Tailscale(发现 Tailnet IP/MagicDNS,作为候选端点),未来的隧道服务走同一套接口

最关键的设计判断是**"access(怎么连)"和"launch(server 怎么起来)"严格分离**——同一个 `ExecutionEnvironment` 可能是手动起的、SSH 远程起的、或者本地起了再靠隧道发布出去,连接方式和启动方式是两条独立的轴,不要耦合。文档里明确对照 Zed:"借 Zed 的远程启动纪律(显式 bootstrap、reconnect 优先、连接体验归客户端),但不借它的传输协议"——Zed 因为远程边界在 editor/project runtime 之下,不得不发明自定义代理协议;T3 的运行时边界已经是"一个 server,标准 WebSocket",没有这个包袱,SSH 只是"起服务+建端口转发"的助手,连上之后还是走普通 WebSocket。

**对 Dozer 的意义**:kooky 只有 OSC 带内标记这种轻量远程信号,orca 需要自研 relay 协议(因为 orca 的执行边界更碎、更贴近 PTY),T3 Code 因为**执行边界已经收敛成"一个进程、一套 WebSocket 协议"**,远程支持几乎是自然延伸,不需要重新发明传输层。这直接印证了一条设计原则:**执行边界收得越干净,以后加远程支持的成本越低**。Dozer 的 `dozerd` 已经是"一个 daemon、一套 UDS 协议"这种干净边界,如果未来真要做远程(spec 目前没定这个方向),T3 的四层模型(Environment/Known/Access/Advertised)是比 orca relay 更轻量、更适合直接抄的起点——把 UDS 换成可选的 TCP/WSS 监听,语义上不需要大改。

## 核心机制三:Server 自更新——版本漂移场景下的能力探测+安全交接

`docs/architecture/server-updates.md`:server 在 `ExecutionEnvironmentDescriptor` 里广播自己"能不能被替换、怎么替换"(`capabilities.serverSelfUpdate`),按进程形态分四档:

| 广播值 | 场景 | 客户端动作 |
|---|---|---|
| `boot-service` | Linux,T3 托管的 systemd 用户服务 | 调更新 RPC,服务单元被替换重启 |
| `respawn` | npm CLI 前台跑在 macOS/Linux | 调更新 RPC,进程交接给一个新起的替身,自己退出 |
| `desktop-managed` | 被 desktop app 监管 | 提示用户去更新 server 所在机器上的 desktop app |
| 缺省 | 旧版本/开发环境/Windows 前台进程/未识别的 supervisor | 给出精确的手动重启命令 |

安装流程有 **preflight 校验**:装好新版本先跑一次版本预检,失败就删掉失败的运行时、保留当前 server 不动,不会半途而废让 server 处于不可用状态。**desktop 托管优先级高于进程形态探测**——被 desktop app 管着的 server 绝不能被"respawn 路径"误伤,自己再起一个进程和 desktop 打架。

**对 Dozer 的意义**:和之前记的"`dozerd` 自身崩溃恢复"缺口不是同一个问题(这是主动升级,不是被动崩溃),但工程纪律相通——**"server 能不能自己换血"需要显式声明能力、而不是客户端瞎猜**,这条思路可以直接套用到 Dozer 未来做 `dozerd` 版本升级流程时的设计(`dozer-app` 检测到 `dozerd` 版本落后,如何安全触发替换而不丢会话)。目前 Dozer 连"`dozerd` 崩溃后怎么办"都还没做,这条属于更后面的问题,但值得记在同一条待办脉络下。

## 核心机制四:Checkpoint/Review——比 Dozer 验收闭环更轻的一层

`apps/server/src/checkpointing/CheckpointStore.ts` 源码注释写得很清楚:"Owns hidden Git-ref checkpoint capture/restore and diff computation…**It does not store user-facing checkpoint metadata and does not coordinate provider conversation rollback**"——回合开始/结束时自动打隐藏 git ref 快照,供 diff 和恢复用,但**不存目标/标准/通过与否这类用户可见的验收元数据**。`ReviewService.ts` 只做一件事:`getDiffPreview`,而且专门校验 diff 的 cwd 必须落在配置的 workspace/worktrees 根目录内(防越权读取仓库外内容)。

**对 Dozer 的意义**:这是一个重要的差异化确认,不是借鉴点——T3 Code 的 checkpoint/review 是"撤销重来+看 diff"基础设施,**没有** Dozer `.dozer/goal.md`(目标/验收标准)+ `AcceptanceStore`(verdict/comment/acceptor 结构化记录)+ `refs/dozer/accepted/<n>` 这一整套"验收闭环"。六个分析对象里,**没有一个做到 Dozer 现在已经做的这个深度**——workmux 没有,kooky 没有,jcode/seek_code 的 checkpoint 概念也只到"回合边界快照",T3 Code 同样止步于"快照+diff"。这从侧面印证 Dozer 的"验收闭环"定位是真实的差异化空白,不是重新发明轮子。

## 核心机制五:运行时模式——直接复用 agent CLI 自带的权限旋钮,不重造

`docs/architecture/runtime-modes.md` 全文只有两档:

- **Full access**(默认):`approvalPolicy: never` + `sandboxMode: danger-full-access`
- **Supervised**:`approvalPolicy: on-request` + `sandboxMode: workspace-write`,应用内弹审批

这两个参数名(`approvalPolicy`/`sandboxMode`)直接是 **Codex CLI 自己的启动参数**——T3 Code 没有像 jcode(`RiskLevel` 分级器)、seek_code(`cmdsafety.ts` 正则黑名单)、workmux(容器/VM 沙盒)那样自己造一套风险判断或隔离机制,而是**直接把 Codex 自带的沙盒模式暴露成一个 UI 开关**。

**对 Dozer 的意义**:这是"治理"这个主题下第三种真实存在的实现路线(前两种是 jcode/seek_code 的"自己判断危险"、workmux 的"自己建隔离边界"),而且是**工程量最小的一种**——前提是被驾驭的 agent CLI 自己就带了权限/沙盒参数。Claude Code 恰好也有类似的东西(`--permission-mode`/hooks 里的 allow/deny/ask 决策),Dozer 排"命令风险门禁"这类待办时,第一步应该先查 Claude Code(以及未来的 Codex 适配器)自己暴露了哪些权限旋钮,能直接透传的就不要重新发明——只有 agent CLI 自己没提供细粒度控制时,才需要 jcode/seek_code 那种外挂分类器。

## 工程文化:连接状态机的纪律

`docs/architecture/connection-runtime.md` 里 `EnvironmentSupervisor` 是**唯一的重试所有者**——离线就释放会话且不消耗重试次数,在线才向 broker 要一次准备好的连接、向 session factory 要一次 RPC 会话;瞬时失败无限重试、指数退避封顶 16 秒;认证/配置错误保持阻塞直到外部唤醒;意外断连保留注册和缓存后重试;显式移除才真正清缓存。UI 层的"已连接"状态要求"socket 打开 + 首次 config RPC 成功"两个条件都满足才成立,**不能靠"有缓存数据"或"transport 对象存在"来推断健康度**——连接健康和数据同步健康是两个独立状态,分开展示。

**对 Dozer 的意义**:这套纪律(单一重试所有者、退避封顶、连接态与数据态解耦)是"客户端-daemon 连接管理"这个具体问题的一份高质量参考实现,`dozer-app` 与 `dozerd` 之间目前的连接管理如果还没有类似的显式状态机(单一重试点、退避策略、连接健康与数据同步健康分离展示),这是可以直接对照抄设计原则的一份现成范本。

## 对 Dozer 的启示汇总

| T3 Code 机制 | 对应 Dozer 位置/路线图 | 借鉴点 |
|---|---|---|
| 优先用 agent CLI 自带结构化协议(app-server/ACP),PTY 兜底 | `dozer-hook`/`dozerd` 的 agent 适配面 | 判断准则:新增 agent 适配器前先查有没有结构化协议,没有才退回 PTY+hook 这条更重的路 |
| Environment/KnownEnvironment/AccessEndpoint/AdvertisedEndpoint 四层远程模型 + access/launch 分离 | spec 未定的"远程"方向 | 目前六个分析对象里最成熟的远程设计,`dozerd` 的干净 UDS 边界如果要加远程支持,这是比 orca relay 更轻的起点 |
| server 自更新的能力探测(boot-service/respawn/desktop-managed/absent)+ preflight + 安全交接 | "`dozerd` 自身崩溃恢复"待办的邻近问题(主动升级 vs 被动崩溃) | 主动升级场景的设计纪律可提前参考,即便当前待办优先级是崩溃恢复不是升级 |
| checkpoint/review 止步于"快照+diff",无验收元数据 | Dozer 已有的验收闭环(goal/criteria/verdict/accepted ref) | 非借鉴点,是差异化确认——六个分析对象里没有一个做到 Dozer 现在的验收深度,这个定位是真空,不是重复造轮子 |
| Runtime modes 直接透传 Codex 自带的 `approvalPolicy`/`sandboxMode` | 命令风险门禁待办(与 jcode/seek_code 并列的第三条路线) | 工程量最小的治理路线:先查被驾驭的 agent CLI 自己暴露了哪些权限旋钮,能透传就不重新发明分类器 |
| `EnvironmentSupervisor` 单一重试所有者 + 连接态/数据态解耦 | `dozer-app`↔`dozerd` 的连接管理 | 现成的连接状态机设计范本,可直接对照当前实现查缺口 |
| Effect(`Context.Service`+`Layer`+类型化错误)贯穿全 server | Dozer 的 Rust 实现,无直接语言对应 | 不是代码可抄,是"用严肃的效果系统/错误类型统一整个后端"的工程态度参照——Rust 的 `thiserror`/`anyhow` 已经在做类似的事,態度上是一致的 |
