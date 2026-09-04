# 快速开始

Dozer 是一个跑在 macOS 上的桌面应用,它不是又一个 AI Agent,而是站在你(甲方)这一侧的**治理与验收层**:你在里面打开真实的 agent 终端(Claude Code、Codex、CodeBuddy……)、看它们改了什么、决定收不收。

## 运行环境

- macOS。
- 一个已经装好的 agent CLI —— 目前 Dozer 认识 Claude Code、CodeBuddy、Codex、Kilo、OpenCode、v8agent 六种(哪些能拿到 hook/MCP 自动接入见下文与 [Agent 会话](agents.md))。没有装任何 agent CLI 也能用 Dozer,只是终端面板里只能开纯 Shell。

## 启动

Dozer 由两部分组成:

- `dozer` —— 你实际打开的 GUI 应用。
- `dozerd` —— 一个常驻后台的 session daemon,负责持有 PTY 会话、项目数据、验收记录。

`dozer` 启动时会自动检测 `dozerd` 是否在跑,不在就自己拉起一个,你不需要手动管理它。这也是 Dozer 的一个关键设计:**关掉 app 窗口甚至 app 崩溃,`dozerd` 手里的 agent 会话不会跟着断**——下次打开 Dozer,之前的终端会话原样恢复(滚屏历史都在),可以继续往里输入。

## 第一次打开:创建或添加一个项目

Dozer 没有传统意义上的"新建工程向导"。首次启动会落在**首页**(点顶栏最左侧 "Dozer" 按钮随时可以回到这里),左栏是项目列表(启动时是空的),右侧是一个常驻的迷你浏览器。

添加一个项目:

1. 点顶栏的 "＋"(或首页项目列表底部的"＋新增项目")。
2. 系统会弹出一个原生的文件选择面板——注意它同时允许选**文件夹**(把这个目录作为新项目)或**文件**(用于后面给项目挂"项目文档/Agent 记忆"链接,见 [项目管理](projects.md))。选一个目录。

选中目录后,Dozer 会静默做几件事(不会打断你,也不会弹"欢迎向导"):

- 如果目录里还没有 `.dozer/` 缓存目录,创建它。
- 如果目录里没有任何 README/CHANGELOG/CONTRIBUTING/LICENSE 类文件,生成一个占位 `README.md`。
- 如果目录还不是 git 仓库,跑一次 `git init`。
- 扫描目录根下看起来像项目文档或 agent 记忆文件的东西(比如 `AGENTS.md`),记成"虚拟链接",方便以后在项目面板里快速跳转。

这些步骤都是幂等的——多次打开同一个项目不会重复创建或报错。项目面板里有一个"修复/同步"按钮可以随时手动重跑这套检查。

## 打开你的第一个 agent 会话

项目打开后,你会看到 Dozer 标志性的**四栏工作区**(完整讲解见 [工作区布局](workspace.md)):左侧项目/文件类面板、左二资产预览、左三/右侧终端与 Agent、AI 栏。

在右侧 **Agent** 面板点 "＋",弹出的选择菜单里选一个 agent(比如 Claude)。Dozer 会:

1. 开一个真实的 `$SHELL` 终端会话(由 `dozerd` 持有的 PTY,不是模拟输出)。
2. 如果这个 agent 支持,静默给它装好 hook(让 Dozer 知道它"正在跑/等你输入/一轮结束了")和 MCP server(让它能读到你当前在预览什么、能提交会话小结)。
3. 把对应的 CLI 命令(比如 `claude`)自动敲进这个终端里,回车交给你。

之后这就是一个正常的终端——你怎么跟 Claude Code 交互,在这里就怎么交互,Dozer 不会代理或过滤你们的对话。

## 一轮最简闭环

1. 让 agent 干活。
2. 一轮结束后,自己判断这轮做得怎么样、要不要继续。

## 下一步

- [工作区布局](workspace.md) —— 认识四栏、图标栏、如何拖拽/收起/放大面板。
- [项目管理](projects.md) —— 多项目标签、首页、删除项目。
- [Agent 会话](agents.md) —— 支持哪些 agent、hook/MCP 具体接了什么。
- [资产预览](preview.md) —— 文件/图片/PDF/网页怎么在 Dozer 里直接看。
- [面板参考](panels.md) —— Files/Todo/Git Log/Database/SSH/Browser/Usage/Conversations 逐一介绍。
- [快捷键](keybindings.md)
- [配置与数据位置](configuration.md)
