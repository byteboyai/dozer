# 配置与数据位置

Dozer **没有图形化的设置面板**——顶栏右上角那个齿轮图标目前只是视觉占位,没有接任何点击行为。所有配置都以文件形式存在磁盘上,靠 Dozer 自己读写,不需要(目前也不能)你手动编辑大部分内容。

## 主题

固定使用内置的深色主题 **ByteBoy2077**(以 Blade Runner 2049 的 Zed 主题为基准、按 byteboy.ai 官网配色调整而成),编译进二进制,**没有浅色模式,没有主题切换开关**。核心配色:

| token | 颜色 |
|---|---|
| 窗口底色 | `#0a0e16` |
| 主强调金 | `#F2D94E`(专用于目标/交付/验收等"甲方动作") |
| 正文奶油色 | `#FFE5B4` |
| 次强调青 | `#47DEF0` |
| 运行绿 | `#1AD585` |

## 配置文件都在哪

都在 `~/Library/Application Support/ai.byteboy.dozer/` 下:

| 文件 | 内容 |
|---|---|
| `layout.json` | 窗口大小、图标栏布局、左右分栏比例 |
| `panel_layouts.json` | 每个项目自己的面板尺寸/折叠状态 |
| `ui_scale.json` | `⌘/Ctrl +/-` 缩放级别 |
| `open_projects.json` | 上次退出时开着哪些项目标签 |
| `dozer.db` | `dozerd` 的 SQLite 库:项目列表、浏览器书签、会话摘要、已摄取的对话/transcript 索引 |

项目内部(每个项目自己的 `.dozer/` 目录下):

| 文件 | 内容 |
|---|---|
| `.dozer/todo.md` | 待办清单(见 [面板参考 · Todo](panels.md#todo)) |
| `.dozer/links.json` | 自动发现的项目文档/agent 记忆文件链接 |
| `.dozer/ssh_hosts.json` | SSH 面板的主机连接配置 |

## 会话/进程

`dozerd`(常驻 daemon,持有所有 PTY 会话、项目状态)由 `dozer` GUI 自动检测并按需拉起,不需要你手动启动或配置。它监听一个本地 Unix Domain Socket,GUI 和 `dozer-mcp`(见 [dozer-mcp](mcp.md))都是通过这个 socket 跟它对话。

## 目前没有的东西

- 没有图形化设置页(计划里未来会有通用/外观/Agents/模型/领域模板/集成/快捷键/关于八节,一期只字面上没做)。
- 没有浅色主题、没有自定义配色。
- 没有可视化编辑 `.dozer/goal.md` 的入口——纯文本文件,直接手写或让 agent 帮你写(`.dozer/todo.md` 例外,Todo 面板本身就是它的可视化编辑器)。
