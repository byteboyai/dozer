# Agent 会话

Dozer 本身**不接任何模型 API**,也没有内置的聊天面板。你在 Dozer 里看到的每一个"agent 会话"都是一个真实的终端会话——由 `dozerd` 持有的一个 PTY,跑着你机器上真实安装的那个 agent CLI,用一个完整的 VT100 级终端模拟器渲染(不是纯文本日志,颜色、光标、TUI 界面都正常显示)。Dozer 不会代理、过滤或复述你和 agent 的对话。

## 支持哪些 agent

Dozer 认识六种 agent CLI,加两种"纯终端"选项,一共八个启动器选项(按标签首字母排序):

- **Claude**(Claude Code)
- **CodeBuddy**
- **Codex**
- **Kilo**
- **OpenCode**
- **v8agent**
- **Git Shell**——开一个终端并自动敲入 `git status`
- **纯 Shell**——就是一个空终端,什么都不敲

## 启动一个会话

右侧 **Agent** 面板点 "＋",在弹出的菜单里选一个。Dozer 会依次做三件事:

1. 在 `dozerd` 上开一个新的 `$SHELL` PTY 会话。
2. 如果这个 agent 支持,静默给它接好 **hook** 和 **MCP**(见下文)。
3. 把这个 agent 的 CLI 命令自动敲进这个刚开的终端(比如选 Claude 就敲 `claude` 然后回车),交给你接手。

也就是说,"启动 agent"本质上就是"开个终端,帮你把该配的都配好,再帮你敲上启动命令"——之后完全是普通终端交互,没有任何 Dozer 特有的输入层。

## hook:让 Dozer 知道 agent 在做什么

Dozer 会尝试给 agent 装一个轻量级钩子(`dozer-hook`),让它在自己的生命周期节点(开始跑、等你输入、一轮结束……)把事件报给 `dozerd`,驱动出你在 UI 上看到的状态点(空闲/运行中/等待输入/一轮结束)。

hook 目前接了 **Claude、CodeBuddy、Codex**(直接写进各自的 settings 配置)和 **OpenCode**(写一个插件文件)。**Kilo 没有可用的 hook 机制,接不了**;**v8agent** 走的是另一条路——它自己的 CLI 会带着 `DOZER_SESSION_ID` 直接向 socket 上报,不需要 hook。

## MCP:让 agent 知道你在看什么

Dozer 同时会给支持的 agent 注册一个只读的 `dozer` MCP server(见 [dozer-mcp](mcp.md)),让 agent 可以:

- 查到你当前在 Dozer 预览面板里看的文件路径和光标/选中范围。
- 主动提交一段这次会话的标题+摘要,回填到 Dozer 的历史对话列表里。

MCP 注册目前支持 **Claude**(写 `~/.claude.json`)、**CodeBuddy**(写 `~/.codebuddy/.mcp.json`)、**Codex**(写 `~/.codex/config.toml`,用无损编辑保留你原有的注释)、**OpenCode**(写 `~/.config/opencode/opencode.json`)。Kilo/v8agent 同样不支持。

## 会话状态胶囊

终端标签和顶栏项目点上看到的颜色状态,来自 hook 上报驱动的状态机:空闲(Idle)、运行中(Running)、等待你输入(AwaitingInput)、一轮结束(TurnEnded)。如果一个会话完全没收到过 hook 事件(比如 Kilo,或者你选的是纯 Shell),状态点会保持灰色的"未知"。

`TurnEnded` 是触发 [验收](acceptance.md) 检查的信号——一轮结束后 Dozer 会去看工作区有没有改动、HEAD 有没有前移,决定要不要在验收图标上点一个金色小红点提醒你。

## 会话存活

所有 PTY 会话由 `dozerd`(常驻后台进程)持有,不是 `dozer` GUI 进程。这意味着:

- 关掉 Dozer 窗口、甚至 app 崩溃,agent 会话不会被打断——它该跑还在跑。
- 重新打开 Dozer,终端会话自动恢复,滚屏历史完整,可以直接继续输入。

## Agent 面板 vs 对话历史

右侧图标栏上 **Agent** 和 **Conversations** 是两个不同的面板:

- **Agent**——当前项目里"活着"的会话列表,按 agent 种类分组,每张卡片显示模型/模式标签、最后活动时间、分支状态、实时状态点。点了直接跳到那个终端标签。
- **Conversations**(对话)——历史对话浏览器,读的是 `dozerd` 摄取进 SQLite 的会话记录索引,每条历史会话有一个 AI 生成(或规则兜底)的标题/摘要,点进去是完整的逐轮审阅视图(人类消息、AI 回复、"思考"文本、结构化工具调用与结果、每轮 token 用量)。这个视图是给你"回看这轮 agent 到底干了什么"用的,不能在这里继续对话。

两者的具体交互细节见 [面板参考](panels.md)。
