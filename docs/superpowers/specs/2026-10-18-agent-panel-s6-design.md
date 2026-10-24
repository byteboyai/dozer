# S6 Agent 面板与产品自己的守护进程 `diggerd`

> **迁移说明(2026-10-20):** 本文原在 Digger 仓库,现移入 Dozer 仓库;文内 `docs/superpowers/...` 路径均相对 **Dozer 仓库根**。写作「Digger(仓库 `../digger`)」的部分,实施仍在 Digger 仓库的 worktree 里进行,但文档不在那里。Digger 自有文档(`docs/adr/*`、`2026-10-14-window-shell-design.md`、`docs/superpowers/notes/`)仍在 Digger 仓库。最新要求与决定见 `docs/superpowers/specs/2026-10-18-agent-panel-s6-design.md` §0。


- 日期:2026-10-18
- 状态:**代码已实现并合入 Digger 本地 `main`(`52df4b9`,2026-10-24,未推送);真机验收与发布未完成**
- 实施状态(2026-10-24):
  - Dozer 侧(S6a Part A、S6b、S6c-1、S6c-2、S6d-2 Part A、S6e-1)已完成,库快照 tag 为 `libs-2026-10-23b`。
  - Digger 侧代码已完成并经审核:S6a Part C(`diggerd`、`digger-hook`、`digger.db`)、S6d-1、S6d-2 Part B(含 Task 5 键盘与输入法、审核补修)、S6d-3、S6e-2。合并后干净快照 536 通过 / 0 失败,clippy 仅基线警告。
  - 未完成:各期报告里的真机项(见 S6d、S6e 文档)、与 Dozer 的逐状态对照、Dozer 与 Digger 各发一个版本(M1 的发布要求)。因此 M1 **尚不能算完成**。
  - 已知遗留:库侧 `create` 成功但 `attach` 失败时会留下孤儿会话(产品侧无会话 id,需库配合);`apps::tests::closing_during_startup_waits_and_releases_the_root` 偶发失败(旧有竞态,已决定暂不处理)。
- 路径:架构级,**分四期交付,每期单独写实施计划**(S6a 守护进程与产品身份、S6b 终端控件库、S6c Agent 面板库、S6d Digger 接入)
- 相关:ADR-014(产品运行时互相独立、只共享代码)、ADR-011(bytehost 进程内应用宿主)、`2026-10-18-project-panel-s5a-design.md`(Project 与项目注册表)、Dozer `2026-07-18-dozer-p1e-agent-integration-design.md`、`2026-08-16-agent-card-workspace-design.md`

## 0. 2026-10-20 修订(最新要求)

本系列原在 Digger 仓库,2026-10-20 移入 Dozer 仓库,并按下列最新要求修订。未列出的内容不变。

1. **里程碑与成功标准:** S6a–S6d 合起来是 M1。成功 = 在 Digger 里打开 Agent 页签能跑起 opencode(Claude Code 同样走通),且 Dozer 的终端与会话行为不变(各期测试总数守恒)。Digger 有了 agent 才算可用。
2. **库先留在 Dozer 仓库:** `dozer-panel-term`、`dozer-panel-agent`、共享的 `dozerd` 库继续放在 Dozer 仓库,Digger 按 `libs-*` tag 引用,计划里的仓库目标与发版流程不变。长期方向是 agent 基座整体进 bytehost、Digger 不依赖 Dozer 与 dozerd(用户 2026-10-20 的方向),**M1 跑通之后再单独立项评估迁移,本期不做**。因此 M1 期间 Digger 仍以 git tag 依赖 Dozer 仓库的库,这是已知的过渡状态。
3. **hook 维持并排共存(§6.3、D2):** 各产品装自己名字的 hook 条目。2026-10-20 早些时候曾讨论过「全局只装一条共享 hook」,已撤回。
4. **基座与呈现的划分(M1 之后路线的依据):** 基座 = 别的能力要建在上面的东西:会话核心、对话摄取、共享记忆、会话总结、无头 agent 调用、MCP 安装、git(`bytegit`)。呈现 = 对基座数据的展示,如用量面板、git log 面板(类比 htop 之于 `/proc`)。用量的**数据**来自对话摄取,属基座;用量**面板与统计查询**属呈现。Todo、群聊等面板自带业务逻辑与存储,**只需接入 `dozerd` / `diggerd`**,不进基座。
5. **对本系列现有内容的影响:** D3(`diggerd` 本期不做功能裁剪,后台功能照常运行)保持不变。上面第 4 条只指导 M1 之后如何整理守护进程内部的基座与面板后端边界、以及「面板后端挂载接口」,另立设计;S6a–S6e 不因此改动。
6. **引用方:** Digger 仓库里的 S7(会话面板)、S8(群聊面板)设计引用了本系列,其中的文件名现在对应 Dozer 仓库里的同名文件。
7. **前置状态(2026-10-20 核对,Digger `main`):** S5a(Project 面板、每项目布局)已合并并发布,含评审修复;S5b-1(文件树)与 Project/Files 同预览配对也已在 `main` 上。因此 S6a Part C 与 S6d 的这两个前置**已满足**,M1 可直接从 S6a Part A 与 S6b(都只动 Dozer)开始。Digger 另有一条进行中的 `feat/preview-pairing`(worktree `digger-p4`),与本系列无关,不要动。
8. **S6a 复审后的两项裁决(2026-10-20,用户):** ① **Q3 协议版本兼容:** 版本号明确不一致、或连到别的产品 → 拒绝并给可读错误;**同一产品**的旧守护进程(不认识 `Hello`)→ 放行并记日志,不阻断(否则升级后的 App 连不上常驻的旧 `dozerd`,杀掉它会丢全部会话)。② **Agent 日志目录:** opencode、goose、aider 的 hook journal 根目录按产品参数化(Dozer 仍是 `~/.dozer/agents/<agent>`,Digger 为 `~/.digger/agents/<agent>`),避免 Digger 读写 Dozer 的目录;Claude、codebuddy、codex、v8agent 读各 agent 自有目录,不改。详见 `docs/superpowers/reviews/2026-10-20-s6a-product-id-review.md` R2、R3。
9. **验收与集成规则(用户 2026-10-20 决定,与各计划里的"从 `main` 切出""前置:X 已合并""未验收不继续"冲突处,以本条为准):**
   - **人工验收推迟到整条 agent 迁移完成后一次性做**(该功能迁移前本就没有问题)。各计划里"请用户目测验收、未验收不继续"的步骤改为"把验收清单原样写进报告、不阻塞"。因此各期的信心完全来自黄金测试与测试总数守恒,这两项必须真跑通。
   - **叠分支,验收前不合 `main`:** Dozer 侧每份计划的分支从**上一份分支的当前顶端**切出,基线测试数取该顶端的数;不合并到 `main`、不打 tag、不推送。原因:`main` 上有长驻的自动发版流程,合入后可能在验收之前就被发出;且 S6c-1 要改的 `dozer-client` 与 S6a 大幅重叠,分开从 `main` 切会冲突。
   - **Dozer 侧链路与分支(按此顺序叠):** S6a Part A(已完成并复审,`feat/product-id`,顶端 `72a3544c`)→ S6b(`feat/panel-term`,从 `feat/product-id` 切)→ S6c-1(从 S6b 分支切)→ S6c-2(从 S6c-1 分支切)→ S6d-2 Part A(从 S6c-2 分支切)→ S6e-1(从链上最新顶端切)。
   - **验收与发布:** 以上全部完成后,用户一次性验收;通过后按链的顺序合并到 `main`、切 `libs-*` tag、推送(这些是对外动作,每次先向用户确认)。
   - **Digger 侧最后做:** S6a Part C、S6d-1、S6d-2 Part B、S6d-3、S6e-2 都依赖那个 `libs-*` tag。tag 之前只能用**不提交**的本地 `[patch]` 开发,不得合并。

## 1. 用户意图

Digger 要有与 Dozer 一模一样的 Agent 面板(迁移计划里排在 project、files 之后)。2026-10-18 的决定:

1. **Agent 运行时走方案 A:Digger 自带独立的守护进程**,而不是把 `dozerd` 嵌进进程。
2. **Digger 用自己名字的守护进程和数据库:`diggerd` 与 `digger.db`**,不借用 `dozerd` / `dozer.db`。两个产品只共享代码,不共享进程、socket、目录、数据库(ADR-014 第 3 条)。

## 2. 现状(读代码得到)

- Agent 面板是配对视图:左 `term/` 终端(`alacritty_terminal` + iced 自绘,约 2000 行),右 Agent 会话卡片列表(`workspace/view.rs` 的 `agent_list_pane`)。面板状态混在 `workspace/state.rs`(3388 行的 `Workspace`)里,没有独立成库。
- 会话与 PTY 由守护进程 `dozerd` 持有(`session.rs` 用 `portable_pty`);界面经 `dozer-client`(约 3000 行,85 个方法)走 Unix socket 连它。会话状态靠推送:Agent 的 hook 事件 → `dozerd` 广播 → `dozer-client` → 面板。
- `dozerd`(约 2 万行)不只管会话:总结流水线、群聊存储、待办、书签、IDE 桥接等都在里面,**共用 `state_dir/dozer.db`**,其中包含项目注册表。`main.rs` 只有 216 行,逻辑大部分已在 `[lib]` 里。
- 数据根、运行态目录、`dozerd.sock` 路径写死在 `dozer-core::paths`(`ProjectDirs("ai","byteboy","dozer")`),没有任何覆盖方式。
- hook 由 Agent 命令行在自己的会话里调用,`dozer-hook` 只有在环境里有 `DOZER_SESSION_ID`(由 `dozerd` 拉起会话时注入)才工作,并连编译期默认的 `socket_path()`。hook 命令被**全局**写进各 Agent 的配置(`~/.claude/settings.json` 等),`ensure_hook_installed` 每次启动都会把它改写成当前产品旁边的 `dozer-hook` 路径(2026-08 出过"堆出重复坏 hook"的事故,所以这段有回归测试)。
- Digger 一侧:项目注册表在进程内的 `<config>/projects.db`(S3p,`dozer-store-project`);没有任何守护进程。

## 3. 目标

- G1 `dozerd` 变成**共享的守护进程库**,Dozer 的 `dozerd` 与 Digger 的 `diggerd` 都是它的薄入口;两者数据根、socket、数据库文件、环境变量名各自独立。
- G2 Digger 打包并拉起自己的 `diggerd`(以及它的 `digger-hook`),数据库是 Digger 数据根下的 **`digger.db`**。
- G3 终端控件与 Agent 面板各抽成 Dozer 里的共享库,Digger 依赖同一份实现。
- G4 Digger 里 Agent 面板可用:在当前项目目录里新建各种 Agent 会话、终端输入输出、会话列表(状态、标题、模型、模式、工作区分支)、切换/结束会话,会话随守护进程存活。
- G5 Dozer 与 Digger 同时安装并同时运行时,两个产品的 hook、会话、事件互不串线。

## 4. 非目标

- 不做 `dozer-mcp` / `digger-mcp`(把面板能力暴露给 Agent 的工具)。留作 S6e,Agent 面板稳定后再做。
- 不做会话(转录)面板、群聊面板(各自单独一轮);但 `diggerd` 里这些后台功能**本期照常运行**(不做功能裁剪),Digger 只是暂时没有对应界面。
- 不把 `dozerd` 嵌进 Digger 进程(方案 B)。
- 不改变 Dozer 的任何用户可见行为、协议、数据位置。

## 5. 反目标(技术上做到了也算失败)

- Dozer 或 Digger 的 hook 事件被对方的守护进程收到,或一个产品把另一个产品的 hook 配置覆盖掉导致对方失效。
- Digger 里出现两份互不知情的项目列表。
- Digger 的 `diggerd` 读写了 Dozer 的目录、socket 或数据库(反过来同理)。
- 库里出现 `dozer-app` 的依赖,或 Digger 里复制一份 Agent 面板 / 终端实现。

## 6. 设计

### 6.1 产品身份(S6a,Dozer 一侧)

在 `dozer-core::paths` 引入 **`ProductId`**(进程启动时由入口设置一次,`OnceLock`):携带产品名(`dozer` / `digger`)、`ProjectDirs` 的 qualifier 三元组、socket 文件名(`dozerd.sock` / `diggerd.sock`)、数据库文件名(`dozer.db` / `digger.db`)、会话环境变量名(`DOZER_SESSION_ID` / `DIGGER_SESSION_ID`)、hook 二进制名(`dozer-hook` / `digger-hook`)。未设置时默认 `dozer`,所以 Dozer 现有行为字节不变。`config_dir()`、`state_dir()`、`socket_path()`、`logs_dir()` 都按它推导。

### 6.2 守护进程成为库

把 `dozerd/src/main.rs` 的启动装配搬进 `dozerd::run(config: DaemonConfig) -> Result<()>`(`DaemonConfig` 含 `ProductId` 与数据库路径)。Dozer 的 `dozerd` 入口变成 3 行;Digger 在自己的仓库里新建 `crates/diggerd`,依赖 `dozerd` 库,入口同样 3 行,设置 `ProductId::digger()` 后调 `run`。

### 6.3 hook:各产品一份,并排共存

- 每个产品安装**自己名字的 hook 命令**(`dozer-hook` / `digger-hook`),写在同一份 Agent 配置的 hook 数组里**并排**,互不覆盖。
- 每个 hook 二进制编译进自己的 `ProductId`:只读自己产品的会话环境变量(`DIGGER_SESSION_ID`),连自己产品的 socket;环境里没有自己的变量就直接返回。于是 Dozer 会话里,`digger-hook` 什么都不做;Digger 会话里,`dozer-hook` 什么都不做。
- 安装逻辑按"命令里含本产品的 hook 名"识别与更新自己的条目,**不碰别人的条目**。这一条要有钉死测试(见 §8),因为 2026-08 的"重复坏 hook"事故正是这里。
- 其余 Agent 的安装器(opencode 插件、goose 插件、aider 启动器、codex 等)里写死的 `dozer` 命名同样要参数化,具体清单由实现者在 S6a 逐个列出。

### 6.4 项目注册表:`digger.db` 单一文件

`diggerd` 的所有表(项目、待办、书签、群聊、会话……)放在 Digger 数据根下的 **`digger.db`**。Digger 进程内的项目存储(S3p 的 `ProjectHost` / `dozer-store-project`)**改为打开同一个 `digger.db`**,不再用 `projects.db`;首次启动时若存在 `projects.db` 就把它的项目与记忆迁入 `digger.db`(幂等,迁入后改名保留,不删除)。两个进程同时读写同一个 SQLite 文件,靠 WAL 和忙等待,**S6a 里要有并发写入的钉死测试**(见 §8)。

### 6.5 终端控件库 `dozer-panel-term`(S6b)

把 `term_model`(`alacritty_terminal` 的 VTE 与网格)与 `term_view`(iced 自绘控件,约 1700 行)抽成库;不含 PTY,PTY 在守护进程里。依赖闭包不含 `dozer-app` / `dozer-client` / `dozer-core`。

### 6.6 Agent 面板库 `dozer-panel-agent`(S6c)

把 Agent 相关状态从 `Workspace` 里拆出来:会话列表、卡片、状态、选中、新建会话入口、与守护进程的订阅。面板对外只依赖一个小 trait(参照已有的 `DaemonService`,按面板实际用到的调用补全:列会话、新建、结束、写入、调整大小、订阅事件),由产品实现——Dozer 用 `dozer-client`,Digger 也用 `dozer-client`(只是连 `diggerd`)。`transcript.rs` 里被卡片用到的字段解析(`permissionMode`、模型 id)随库走。

### 6.7 Digger 接入(S6d)

- 打包:`diggerd`、`digger-hook` 放进 `Digger.app/Contents/MacOS/`(`package-macos.sh` / `release.sh` 同步);拉起逻辑同 Dozer 的 `spawn_dozerd`:在自己的可执行文件旁边找,单实例检查。
- 面板:`digger.agent` 配对视图(左终端、右列表),放进左栏;当前项目的路径作为新会话的工作目录;无已开项目时显示空状态。
- 会话随守护进程存活:关掉 Digger 窗口不杀会话,下次启动能重新连上(与 Dozer 一致)。
- Agent 种类:与 Dozer 一致的全部种类;某个 Agent 命令行没装时,新建时给出可读错误。

## 7. 分期

- **S6a 守护进程与产品身份**:`ProductId`、`dozerd::run`、hook 并排共存与参数化、协议版本字段与兼容检查;Dozer 一侧全做、测试守恒;Digger 一侧新建 `diggerd` 与 `digger.db`、项目存储迁移、打包与拉起。**本期结束时 Digger 有自己的守护进程在跑,但还没有 Agent 界面。**
- **S6b 终端控件库**:抽 `dozer-panel-term`,Dozer 改引用。
- **S6c Agent 面板库**:抽 `dozer-panel-agent`,Dozer 改引用,目测验收,发 tag。
- **S6d Digger 接入**:依赖升级、`digger.agent` 面板、真机回归。

每期都是"Dozer 抽库 → 目测验收 → 发 tag → Digger 接入"的节奏,Dozer 每步编译且测试全绿、测试总数守恒。S6a 要求 S5a 已合并(项目存储与左栏);S6d 还要求 S5b-1(文件树)之后的左栏顺序已定。(2026-10-20 核对:这两个前置在 Digger `main` 上均已满足,见 §0 第 7 条。)

## 8. 验证

- Dozer:每期前后测试总数守恒;`ProductId` 默认值下路径、socket、环境变量名与改造前字节一致(黄金测试)。
- **隔离钉死测试(S6a)**:同一台机器上同时起 `dozerd` 与 `diggerd`,各拉起一个会话:① Dozer 会话里触发 `digger-hook`,`diggerd` 收不到事件;② Digger 会话里触发 `dozer-hook`,`dozerd` 收不到事件;③ 各自的 socket、数据库、日志目录互不重叠。
- **hook 配置钉死测试(S6a)**:对一份已含 Dozer hook 条目的 Agent 配置,Digger 安装后两个条目并存;重复安装不产生重复条目;卸载只移除自己的条目;反过来同理。
- **并发写入钉死测试(S6a)**:`diggerd` 与 Digger 进程内的项目存储同时对 `digger.db` 做上千次交错写,不丢写、不出现 `database is locked`。
- **迁移测试(S6a)**:存在旧 `projects.db` 时首次启动迁入 `digger.db`,再次启动不重复迁入,旧文件被改名保留。
- Digger 真机(只能截图,点击要人来做):Dozer 与 Digger 同时运行,各开一个 Claude 会话,状态点各自更新、互不影响;在 Digger 里新建会话、输入输出正常、会话列表卡片信息正确;退出再启动能重连会话;项目列表只有一份。

## 9. 我的决定与未解问题

我的决定(可以推翻):

- D1 `ProductId` 用进程级 `OnceLock`,由入口设置,而不是到处传参。
- D2 hook 并排共存、各认各的环境变量,不用"互换同一个 hook"的方案。
- D3 `diggerd` 本期不做功能裁剪,后台总结/群聊等照常运行。
- D4 `dozer-mcp` 对应的 Digger 版推迟到 S6e。
- D5 旧 `projects.db` 迁入后改名保留,不删除。
- D6 守护进程与进程内存储共用 `digger.db`(而不是让 Digger 也经守护进程读写项目)。

未解:

- Q1 `diggerd` 本期完整运行会占用多少资源(总结流水线是否会在没有界面的情况下自己调用外部 LLM、产生费用)?我倾向在 S6a 里先加一个"后台总结默认关闭"的配置,由 Digger 的入口设置;请你确认这样做是否符合预期。
- Q2 两个进程写同一个 SQLite 文件在 macOS 上的表现需要 S6a 里实测。若实测不稳,退路是 D6 的反面:Digger 的项目存储改为经 `diggerd` 读写。
- Q3 协议版本不一致时(Digger 打包的 `diggerd` 与 `digger-hook` 版本不同步)如何降级,实现者在 S6a 里按"拒绝连接并给出可读错误"处理,是否需要更复杂的兼容,请你定。

## 10. 留给实现者决定

`ProductId` 的具体字段与构造函数、`DaemonConfig` 的形状、各 Agent 安装器里需要参数化的命名清单、迁移的触发点、`diggerd` 单实例检查的实现、S6b / S6c 里状态与视图的逐个字段归属。
