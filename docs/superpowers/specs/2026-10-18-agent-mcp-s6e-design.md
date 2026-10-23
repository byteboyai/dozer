# S6e Digger 版 MCP:`digger-mcp`

> **迁移说明(2026-10-20):** 本文原在 Digger 仓库,现移入 Dozer 仓库;文内 `docs/superpowers/...` 路径均相对 **Dozer 仓库根**。写作「Digger(仓库 `../digger`)」的部分,实施仍在 Digger 仓库的 worktree 里进行,但文档不在那里。Digger 自有文档(`docs/adr/*`、`2026-10-14-window-shell-design.md`、`docs/superpowers/notes/`)仍在 Digger 仓库。最新要求与决定见 `docs/superpowers/specs/2026-10-18-agent-panel-s6-design.md` §0。


- 日期:2026-10-18
- 状态:设计稿,待用户审阅
- 路径:架构级,**分两期**(S6e-1 Dozer 把 `dozer-mcp` 变成可按产品配置的共享库、S6e-2 Digger 接入);另有一期**依赖其它面板、现在不做**的 S6e-3(待办工具、预览与文件编辑工具)
- 相关:`2026-10-18-agent-panel-s6-design.md`(S6 的 §4 把本期留到 S6e;hook 并排共存的做法是本文的模板)、`plans/2026-10-18-agent-s6a-daemon-identity.md`(`ProductId`、`dozer-hook` 的 `run_main`)、`plans/2026-10-18-agent-s6d2-sessions.md`(`HookInstaller::ensure_mcp`、`AgentCapabilities::mcp_install`)

## 1. 用户意图

Dozer 里的 Agent 通过 MCP 服务 `dozer-mcp` 读写 Dozer 的东西(共享项目记忆、待办、预览上下文等)。Digger 里的 Agent 也需要同样的能力入口:`digger-mcp`,连的是 `diggerd` / `digger.db`,只暴露 Digger 里**真实存在**的能力。两个产品同机并存,互不串线。

## 2. 现状(读代码得到)

- **`dozer-mcp` 是一个独立二进制(`dozer-mcp serve|install|uninstall <agent>`),基于 `rmcp`(stdio 传输)。** 工具(`server.rs`,581 行):`get_preview_context`、`preview_navigate`、`submit_session_summary`、`list_todos`、`add_todo`、`toggle_todo`、`edit_todo_text`、`write_memory`、`list_memories`、`get_memory`、`locate_in_file`、`apply_precise_edit`。每次调用现连 `dozerd`(会话→项目的解析靠 `list()` 找到本会话的 `project_id` 和 Agent 种类)。
- **写死的产品标识:** `serve` 读 `DOZER_SESSION_ID`,缺则报错退出;连 `dozer_core::paths::socket_path()`;`install` 往各 Agent 的 MCP 配置里写名为 **`dozer`** 的条目(Claude / Codebuddy / Qodercn:`mcpServers.dozer`;Codex:TOML `[mcp_servers.dozer]`,用 `toml_edit` 保留用户注释;Opencode:命令数组),`command` 是当前可执行文件、`args: ["serve"]`;幂等,只碰自己的键。
- **能力依赖其它面板与后台:** 记忆工具读写 `diggerd` 里的项目记忆(与 Digger 的 Project 面板同一份 `digger.db`);待办工具依赖待办面板;预览/定位/编辑工具依赖 GUI 里的预览引擎(`dozerd` 缓存 GUI 推来的预览上下文,`preview_navigate` 发命令等 GUI 执行);`submit_session_summary` 服务于总结流水线(Digger 的 `diggerd` 默认关闭)。
- **Dozer 一侧安装时机:** 新建会话前 `ensure_mcp_installed(agent)`(同 hook),`caps.mcp_install` 控制(S6c 起为库的能力开关,Digger 的 S6d 里全关)。
- **并存时的一个陷阱(读代码得出):** 全局 MCP 配置里会同时有 `dozer` 与 `digger` 两条。在 Digger 的会话里只有 `DIGGER_SESSION_ID`,`dozer-mcp serve` 缺 `DOZER_SESSION_ID` 会**报错退出**,Agent 的 MCP 状态里就会一直显示 `dozer` 启动失败;Dozer 会话里 `digger-mcp` 同理。hook 对此已经静默处理(缺变量即 0 退出),MCP 还没有。

## 3. 目标

- G1 `dozer-mcp` 变成可按产品配置的共享库:`run_main(product, features)`,服务器名 / 会话环境变量 / socket / 安装条目名都取自 `ProductId`;Dozer 行为字节不变。
- G2 Digger 打包并安装自己的 `digger-mcp`;Digger 的 Agent 会话在启动前自动注册它;只暴露 Digger 里真实可用的工具。
- G3 两个产品并存:各自的条目共存、互不覆盖;一个产品的会话里,另一个产品的 MCP 服务**不报错**(安静地不提供工具)。
- G4 Agent 经 `digger-mcp` 写的项目记忆,在 Digger 的 Project 面板里可见。

## 4. 非目标

- 不做 Digger 的待办、预览、定位、精确编辑工具(S6e-3,依赖待办面板与预览引擎接入 Digger)。
- 不做 `submit_session_summary`(Digger 的总结流水线默认关闭,该工具没有消费者)。
- 不做群聊、会话面板的 MCP 工具。
- 不改工具的参数、返回形状与描述文案。
- 不做 V8agent 的自动挂载(那是 Dozer 专属)。

## 5. 反目标

- Dozer 或 Digger 的 MCP 配置条目被对方覆盖或删除;卸载一个产品误删另一个的条目。
- 任一产品的会话里出现「另一产品的 MCP 服务启动失败」。
- Digger 暴露一个调用后没有任何可见效果、或必然失败的工具(死入口)。
- `digger-mcp` 读写 Dozer 的目录、socket、数据库。
- 在用户没有任何 Digger Agent 会话时改写用户的全局 Agent 配置之外的文件;改写时丢失用户注释(Codex TOML 必须继续格式保留)。

## 6. 设计

### 6.1 `dozer-mcp` 共享库化(S6e-1,Dozer)

- 新增 `dozer_mcp::run_main(product: ProductId, features: McpFeatures) -> !`:第一件事 `paths::install(product)`(S6a 的进程级产品身份);`serve` / `install` / `uninstall` 三个子命令由它分发;`dozer-mcp` 的 `main` 变成 `run_main(ProductId::dozer(), McpFeatures::all())`。
- `McpFeatures { memory: bool, summary: bool, todos: bool, preview: bool, file_edit: bool }`。工具按特性分组:`memory` = `write_memory` / `list_memories` / `get_memory`;`summary` = `submit_session_summary`;`todos` = 四个待办工具;`preview` = `get_preview_context` / `preview_navigate` / `locate_in_file`;`file_edit` = `apply_precise_edit`。未开启的工具**不出现在工具列表里**(不是调用时报错)。具体如何在 `rmcp` 的 `#[tool_router]` 上做运行时筛选(移除路由或拆分路由结构),留给实现者。
- `serve` 的取值全部来自 `product()`:会话环境变量名(`DIGGER_SESSION_ID`)、`socket_path()`、服务器名(`initialize` 应答里的 `name`)。
- **并存时的安静模式:** `serve` 启动时若本产品的会话环境变量存在 → 正常;**不存在但另一个已知产品的会话环境变量存在**(`DOZER_SESSION_ID` / `DIGGER_SESSION_ID` 任一)→ 启动一个**零工具**的 MCP 服务(握手正常、`tools/list` 为空、stdin 关闭即 0 退出),不打印错误;**两者都不存在** → 保持现状:报错退出(提示「只能在 …拉起的会话里跑」)。这样 Dozer 现有的「无会话变量时报错」测试**期望不变**。
- `install` / `uninstall` 取条目名 = `product().name`:Claude 类 `mcpServers.<name>`、Codex `[mcp_servers.<name>]`、Opencode 同键;`command` 取调用方传入的可执行路径(`run_at_with_exe` 已有),`args` 仍为 `["serve"]`。**只增删自己名字的键,别的键(含另一个产品的)原样保留**,Codex 继续用 `toml_edit` 保格式。条目名与可执行文件名绑定检查:同名键但 `command` 不含本产品的 `mcp_bin` 时,覆盖前保留原值不动并返回错误码(防止误覆盖用户手写的同名服务器)。
- Dozer 的 `ensure_mcp_installed` 与既有安装测试原样通过(默认产品下写出的条目字节不变)。

### 6.2 Digger 接入(S6e-2)

- **新 crate `digger-mcp`**(`crates/digger-mcp`,3 行入口):`dozer_mcp::run_main(digger_paths::product(), McpFeatures { memory: true, summary: false, todos: false, preview: false, file_edit: false })`。
- **打包:** `package-macos.sh` / `release.sh` 同时构建并拷贝 `digger-mcp` 到 `Digger.app/Contents/MacOS/`(与 `diggerd`、`digger-hook` 并列),缺失即失败(照抄 Dozer 对「sibling 二进制缺失」事故的说明)。`check-host-deps.sh` 的宿主适配白名单加 `digger-mcp`。
- **安装时机:** S6d 里 `AgentCapabilities::mcp_install` 为假;本期改为真,`DiggerHooks::ensure_mcp(agent)` 运行同目录的 `digger-mcp install <agent>`(不继承 stdin、丢弃输出、10 秒上限,同 `ensure_hook`);只对支持 MCP 的 Agent(Claude / Codebuddy / Codex / Opencode / Qodercn)执行;Aider / Goose / V8agent / Unknown 空操作。
- **记忆联动(G4):** Agent 经 MCP 写入记忆后,Project 面板需要看到。Project 面板在切入时拉取记忆;面板已经在眼前时,Agent 写入不会触发刷新。Digger 在该项目的 `AgentHostEvent::TurnEnded` 到达时,若 Project 面板可见,刷新该项目的记忆列表(只做这一件,不做轮询)。若库里没有可复用的「只刷新记忆」入口,先停下报告,不在 Digger 里重写读取逻辑。

### 6.3 S6e-3(现在不做,写下依赖)

待办工具随待办面板接入 Digger 时打开 `todos`;预览 / 定位 / 精确编辑随预览引擎(S5b-2)接入 Digger、且 Digger 的 GUI 开始向 `diggerd` 推送预览上下文并执行 `PreviewCommand` 之后打开 `preview` / `file_edit`。届时只改 `digger-mcp` 入口的特性开关与 GUI 侧,不再动 `dozer-mcp`。

### 6.4 分期

- **S6e-1(Dozer):** 抽 `run_main` 与 `McpFeatures`、按产品取值的安装、安静模式;黄金测试固定默认产品下的行为不变;切 tag。**不发布。**
- **S6e-2(Digger):** `digger-mcp` crate、打包、`ensure_mcp` 打开、记忆刷新联动、同机并存回归。**不单独发布**(MCP 不是面板;随下一次面板迁移完成的发布一起带上)。

## 7. 验证

- Dozer:测试总数守恒;`dozer-mcp` 的现有测试(`memory_tools`、`todo_tools`、`file_mutation_tools`、`get_preview_context`、`submit_session_summary`、`serve_requires_session_id`、安装器各项)**原样通过**;**默认产品黄金测试**:`McpFeatures::all()` 下 `tools/list` 与改造前逐项相同(名称、描述、参数 schema 逐字)、`install` 写出的 JSON / TOML / Opencode 配置与改造前逐字节相同;新增:每个特性组关闭时对应工具不在列表里、`DIGGER_SESSION_ID` 存在而 `DOZER_SESSION_ID` 缺失时 Dozer 的 `serve` 进入零工具模式且 `initialize` 成功、两个变量都缺失时仍报错、`digger` 产品安装后与已有 `dozer` 条目共存(三种配置格式各一)、重复安装不产生重复、卸载只删自己的、同名但命令不含本产品二进制名时拒绝覆盖、Codex 配置的注释与无关表在安装后字节保留。
- Digger:`digger-mcp` 在 `DIGGER_SESSION_ID` 下 `tools/list` 恰为三个记忆工具;在只有 `DOZER_SESSION_ID` 的环境下为空列表且不报错;`ensure_mcp` 对不支持的 Agent 不执行;缺少 `digger-mcp` 二进制时给出可读失败而不卡会话创建;记忆刷新联动的单测(回合结束 + 面板可见才刷新)。
- 真机(沙箱数据根与临时 `HOME`;点击由人做):在 Digger 里开 Claude 会话,Agent 的 MCP 列表里 `digger` 已连接、`dozer`(若同机装了 Dozer 的条目)显示为已连接但无工具而非失败;让 Agent 写一条记忆,Project 面板在回合结束后出现该条;同一项目在 Dozer 里看不到这条(各自的库);Dozer 与 Digger 同机各开会话,互不影响;卸载 `digger-mcp uninstall claude` 后 `dozer` 条目仍在。

## 8. 我的决定与未解问题

我的决定(可以推翻):

- D1 `dozer-mcp` 共享库化而不是在 Digger 里复制一份(同 hook 的做法)。
- D2 「安静模式」只在「另一个已知产品的会话变量存在」时启用,两者皆无仍报错,保持 Dozer 现有行为与测试。
- D3 Digger v1 只开 `memory`;`summary` 关闭(没有消费者)。
- D4 未开启的工具不出现在列表里,而不是调用时报错。
- D5 `digger-mcp` 不单独发布产品版本。
- D6 记忆联动只在回合结束且 Project 面板可见时刷新一次,不轮询。

未解:

- Q1 全局 Agent 配置(如 `~/.claude.json`、`~/.codex/config.toml`)会被 Digger 写入一条 `digger` MCP 条目。这和 hook 一样是对用户全局配置的修改,Dozer 一直是静默做。Digger 沿用静默(`digger-mcp uninstall <agent>` 可移除)。是否接受?
- Q2 同名条目保护(6.1 末段)会让「用户手写了一个名叫 `digger` 的 MCP 服务器」时我们**不覆盖并报错**,Agent 里就没有 Digger 的 MCP 工具。我倾向保护用户的配置。是否接受?
- Q3 Digger v1 只有三个记忆工具,Agent 看到的工具集比 Dozer 里少(没有待办、预览、编辑)。这是「只暴露真实存在的能力」的直接结果。是否接受?

## 9. 留给实现者决定

`rmcp` 工具路由的运行时筛选方式、`McpFeatures` 的精确字段与默认值、零工具模式下 `initialize` 应答里的服务器名与能力声明、同名保护的具体判定(命令路径含 `mcp_bin` 的匹配规则)、`run_main` 与现有 `server::run` / `install::run` 的重构边界、Opencode 配置里命令数组的条目名处理、记忆刷新用的库入口名。
