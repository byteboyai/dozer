# dozer-mcp:面向外部 agent 的接口

大部分时候你不需要直接接触这一层——[启动 agent 会话](agents.md#mcp让-agent-知道你在看什么) 时 Dozer 会自动帮支持的 agent 注册好。这一页是给需要手动管理、或者想搞清楚 Dozer 到底给了 agent 多少权限的人看的。

## 这是什么

`dozer-mcp` 是一个独立的 MCP stdio server(二进制名 `dozer-mcp`),给外部 CLI agent(Claude Code、CodeBuddy、Codex、OpenCode、v8agent)提供一组工具。目前(不含下面"设计已定、尚未实现"的部分)它刻意做得很窄——不代理"agent 原本自己就能做的事"(读写你项目里的源文件、跑 shell 命令这些 agent CLI 自带的能力,Dozer 不插手,也不重复实现一遍),只服务"治理层需要的"两件事:

1. **agent 能报告自己在干什么**(会话总结)、**能知道你在看什么**(预览上下文/定位)——这部分是只读的,不写任何东西。
2. **agent 能读写 Dozer 自己的治理层状态**——具体是 **Todo 列表** 和 **项目共享记忆** 这两块。这两块数据权威存储都在 `dozerd` 的 `dozer.db`(SQLite),不是普通文本文件,agent 没有 MCP 之外的路子能碰到它们;开放读写是因为这本来就是"Dozer 管理的、给 agent 协作用的"数据,不是你的项目产物。

这个"不代理 agent 已有能力"的边界已经往外挪了一格:2026-09-28 批准的 [Agent-native 文件编辑器设计](../superpowers/specs/2026-09-28-agent-native-file-editor-design.md)明确要让 agent 成为编辑你项目文件的主体(推翻了"预览优先渲染而非编辑"这条旧裁决),第一阶段给 Files/Preview 加了两个真正写项目文件的工具(`locate_in_file`/`apply_precise_edit`)——见下面"Files 面板"一节。所以更准确的说法是:**MCP 现在会扩大 agent 对你项目源文件本身的能力面(精确修改,受 Conflict Detection 与路径边界约束)**;Todo、共享记忆这两块治理层状态的读写口子不受这次调整影响,逻辑不变。

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

**v8agent** 不走这条注册路径,但一样能用到全部工具——`v8agent-cli` 检测到 `DOZER_SESSION_ID` 环境变量(Dozer 拉起的会话本来就有)就会自动把 `dozer-mcp serve` 接成自己的 MCP source,不需要改任何配置文件,`dozer-mcp install/uninstall` 对它是 no-op。

Goose、Aider 目前没有对应的 MCP 支持(接入方式和其余几家不同,首期未覆盖)。

## 现有工具:按面板/模块

### Preview 面板

- **`get_preview_context`**——无参数。返回你当前在预览面板里打开的文件路径,以及光标/选中范围(1 起始行号)、是否只读、可见行区间、表格 sheet/cell 等扩展字段,外加更新时间戳。没在预览任何东西时返回 `reason: "no_active_preview"`。agent 想知道"用户现在正盯着哪段代码"时可以主动查,不用你手动复制粘贴。
- **`preview_navigate`**——传 `path` + `line`/`column`(1-based),让预览滚动并定位光标;再给 `end_line`/`end_column` 就变成选中一段范围。只读导航,不写文件。

两个工具都不能编辑内容。预览命令协议本身其实已经定义了一个 `Replace`(按范围替换文本,受 `expected_revision` 保护,防止你和 agent 同时改同一处)动作,但目前没有任何 MCP 工具驱动它,唯一的调用方是内部测试;下面"Files 面板"一节要加的写入能力**不会**接这个 `Replace` 动作——`Replace` 继续只留给 UI 自己触发的编辑路径用,agent 精确修改走的是另一套直接对磁盘操作的机制(不要求文件已经开在某个 tab 里),两者不合并。

### Files 面板

设计见 [Agent-native 文件编辑器设计](../superpowers/specs/2026-09-28-agent-native-file-editor-design.md),实现计划见
[Phase 1 implementation plan](../superpowers/plans/2026-09-28-agent-native-file-editor-phase1.md)。

- **`locate_in_file(path, query)`**——只读。在项目内某文本文件里搜索一段文字,唯一匹配时返回精确坐标(1-based 行列)+ 上下文;匹配到多处就把候选全部列出,不擅自选一个,逼你把 `query` 写得更具体。agent 应该先用这个工具拿到准确坐标,不是自己数行号。
- **`apply_precise_edit(path, start_line, start_col, end_line, end_col, expected_text, new_text, summary)`**——写。精确替换给定坐标区间的内容,`expected_text` 必须等于该区间当前的原样内容(Conflict Detection:磁盘内容对不上就拒绝,把真实内容连同坐标一起回给 agent 重算,不会盲目覆盖)。`summary` 必填,一句话说明这次改了什么。整篇重写就是把区间设成整个文件,不是单独的工具。

这两个工具**直接对磁盘操作**,不要求目标文件当前开在某个 Preview tab 里;改完之后 Dozer 现有的文件系统监听会让已打开的干净 tab 自动刷新,并额外自动定位、短暂高亮到刚被改的位置。每次成功的精确修改都会写进一张新的历史表(`file_edit_history`),但 Phase 1 这张表只由 GUI 读取,不额外开一个"查历史"的 MCP 工具给 agent。

Goose、Aider 这两家因为没有 MCP 通道,拿不到这两个工具;spec 里的应对方案是当前会话是 Goose/Aider 时,由 Dozer 另外并行 dispatch 一个 headless v8agent 去执行精确修改(历史记录署名 `v8agent`,如实反映谁动的手)——这条兜底路由要等 Phase 2("发送到上下文"提供一个结构化的意见输入框)才有触发信号,Phase 1 阶段还用不上。

### Todo 面板

- **`list_todos`**——列出当前项目的任务(id/文字/是否完成)。
- **`add_todo`**——新增一条任务,置顶到列表最前。
- **`toggle_todo`**——勾选/取消勾选完成状态。
- **`edit_todo_text`**——改任务文字。

Todo 的权威存储是 `dozerd` 的 SQLite(2026-09 已从早期的 `.dozer/todo.md` 纯文本迁移过来),所以这几个工具是 agent 能碰到任务列表的**唯一**渠道,不是"文件本来就能改,MCP 只是图方便"。列表侧还有排序、到期日期、分类、指派、"派给现有会话"等能力(见 [Todo 面板](panels.md#todo)),目前都只在 GUI 里,没有对应的 MCP 工具——包括删除:没有 `remove_todo`。

### Project 面板 → 共享记忆

- **`write_memory`**——按标题在项目内 upsert(标题已存在就更新,不存在就新建),参数 `title`/`body`/`kind`/`description`。
- **`list_memories`**——列出当前项目的共享记忆(标题/分类/摘要/最后更新方,不含正文)。
- **`get_memory`**——查一条记忆的完整正文 + 最近改动历史,`title_or_id` 可传标题或数字 id。

这是 Claude Code/CodeBuddy/Codex/OpenCode/v8agent 五家共用的一份记忆(goose/aider 因为没有 MCP 通道,仍是已知的覆盖缺口)——写入即时对其它 agent 可见,不是"碰巧都读同一个本地文件"式的尽力而为。**刻意不提供 `delete_memory`**:删除权限不给 agent,只能人工在 Project 面板里删(删除后历史记录仍保留可查)。

### Conversations 面板(会话总结)

- **`submit_session_summary`**——参数 `title`(≤200 字符)、`summary`(≤200 字符,涉及多个事件建议分点)。把这次会话的标题和摘要记录到 `dozerd`,回填到 [Conversations 面板](agents.md#agent-面板-vs-对话历史) 里对应会话的展示上。

按工具描述本身的措辞:只应该在**被明确要求总结这次会话时**调用一次,不应该主动、反复调用。这个方向目前是单向的——agent 能写总结,但没有对应的 `list_session_summaries`/`get_session_summary` 能读回其它会话的总结,想看历史总结只能在 GUI 里翻。

## 没有对应 MCP 工具的面板

Git Log、Database、SSH、Web(浏览器)、Usage 目前都没有专属 MCP 工具(Files 面板已经有工具,见上面"Files 面板"一节,不再算在这里)。原因不完全一样:

- **Git Log**:agent CLI 本来就有更好的原生 `git` 命令能力,Dozer 没必要代理一遍——这类"agent 自己就能做"的事,刻意不接进 MCP。
- **Database / SSH**:面板里保存的是连接凭证(密码/私钥,存在钥匙串或加密配置里)。要把"查询这个已保存连接"开放给 agent,等于把凭证访问权也间接给了 agent,目前没有做,也还没有想清楚权限模型(比如按连接单独授权、还是整块面板级开关)。
- **Web(浏览器) / Usage**:纯粹是还没做,不涉及凭证或"agent 自己就能做"这类边界问题。

## 下一步可以达到的能力(探索方向,不是承诺)

以下是几个在现有代码/协议基础上"顺手就能做"的方向,尚未有 spec 或排期,列出来是为了让"现在有什么、缺什么"更完整,不代表已经决定要做:

- **Web(浏览器)面板暴露当前 URL**:和 `get_preview_context` 同一个思路——agent 想知道"用户当前在浏览器里看哪个页面"时可以查,不用你口述链接。只读,风险和 `get_preview_context` 基本一致。
- **Usage 面板自查用量**:agent 查自己当前会话/项目的 token 用量,用来自我判断要不要收敛输出长度。只读,不涉及凭证。
- **Todo 面板补齐删除/分类/到期日期**:`reorder`/`set_category`/`set_plan_date`/`assign_agent` 这些能力在 `dozerd` 的 `TodoStore` 里已经存在,只是还没接出对应的 MCP 工具;删除是否要开放给 agent 需要单独决定(参考共享记忆"删除只能人工做"的既有先例)。
- **Database/SSH 面板的只读能力**(比如"列出已保存连接名"而不涉及凭证):比直接暴露查询能力风险低很多,但目前也还没有设计。
- **Files 面板 Phase 2-4**:上面"Files 面板(设计已定,尚未实现)"一节只是 Agent-native 文件编辑器的 Phase 1(纯文本、精确修改)。Phase 2(右键"发送到上下文")、Phase 3(agent 上下文列表 + 修改历史弹窗 UI)、Phase 4(表格/图片定位)都已经在 spec 里拆出来了,但都还没单独 brainstorm 出各自的设计,不算"探索方向"这种未定案的东西,是已经排好队、只是还没写的后续阶段。

## 运行前提

`dozer-mcp serve` 需要环境变量 `DOZER_SESSION_ID`——这个变量只会在 Dozer 自己启动的 PTY 会话里存在,所以这个 server **只能在 Dozer 启动的 agent 会话内部工作**,靠这个变量反查"这是哪个项目、哪个会话",不需要你另外传任何配置。它通过和 GUI 一样的本地 Unix Domain Socket 连回 `dozerd`。
