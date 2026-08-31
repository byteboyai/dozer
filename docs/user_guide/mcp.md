# dozer-mcp:面向外部 agent 的接口

大部分时候你不需要直接接触这一层——[启动 agent 会话](agents.md#mcp让-agent-知道你在看什么) 时 Dozer 会自动帮支持的 agent 注册好。这一页是给需要手动管理、或者想搞清楚 Dozer 到底给了 agent 多少权限的人看的。

## 这是什么

`dozer-mcp` 是一个独立的、**只读**的 MCP stdio server(二进制名 `dozer-mcp`),给外部 CLI agent(Claude Code、CodeBuddy、Codex、OpenCode)提供两个工具。它刻意做得很窄——不给 agent 任何写文件、改数据库、操作 UI 的能力,只做两件"治理层需要的"事:agent 能报告自己在干什么,能知道你在看什么。这和"Dozer 不是又一个 agent"这个定位是一致的:它不扩大 agent 的能力面,只服务人机验收这一件事。

## 手动安装/卸载

正常情况下不需要手动跑这个——[启动会话时 Dozer 已经自动做了](agents.md#hook让-dozer-知道-agent-在做什么)。如果你需要手动管理:

```bash
dozer-mcp install <agent>    # 把 dozer MCP server 注册进该 agent 的配置
dozer-mcp uninstall <agent>  # 撤销注册
```

支持的 `<agent>`:`claude`、`codebuddy`、`codex`、`opencode`。

会写入的配置文件:

| agent | 配置文件 |
|---|---|
| Claude | `~/.claude.json` |
| CodeBuddy | `~/.codebuddy/.mcp.json` |
| Codex | `~/.codex/config.toml`(用无损编辑,保留你原有的注释和格式) |
| OpenCode | `~/.config/opencode/opencode.json` |

Kilo、v8agent 目前没有对应的 MCP 注册支持。

## 提供的两个工具

### `get_preview_context`

无参数。返回你当前在 Dozer 预览面板里打开的文件路径,以及光标/选中范围(1 起始行号),外加一个更新时间戳。如果你当前没在预览任何东西,返回 `reason: "no_active_preview"`。

用途:agent 想知道"用户现在正盯着哪段代码"时可以主动查这个,不用你手动复制粘贴文件路径或代码片段。

### `submit_session_summary`

参数:`title`(≤200 字符)、`summary`(≤8000 字符)。把这次会话的标题和摘要记录到 `dozerd`,回填到 [Conversations 面板](agents.md#agent-面板-vs-对话历史) 里对应会话的展示上。

按工具描述本身的措辞:只应该在**被明确要求总结这次会话时**调用一次,不应该主动、反复调用。

## 运行前提

`dozer-mcp serve` 需要环境变量 `DOZER_SESSION_ID`——这个变量只会在 Dozer 自己启动的 PTY 会话里存在,所以这个 server **只能在 Dozer 启动的 agent 会话内部工作**,靠这个变量反查"这是哪个项目、哪个会话",不需要你另外传任何配置。它通过和 GUI 一样的本地 Unix Domain Socket 连回 `dozerd`。
