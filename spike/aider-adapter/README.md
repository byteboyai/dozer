# Aider adapter spike

用本机真实 Aider 0.86.2 实测的 contract 记录。**已实跑**(有可用 LLM provider
`openai/deepseek-v3` 走本地 LiteLLM),核心格式为真实输出、非推测。

## 环境

```text
aider --version → 0.86.2
```

## CLI flags（`aider --help` 核实，全部存在）

- `--chat-history-file <FILE>`、`--input-history-file <FILE>`
- `--message/--msg/-m <CMD>`、`--no-stream`、`--no-pretty`
- `--no-auto-commits`、`--yes-always`、`--notifications`、`--notifications-command <CMD>`
- `--no-show-model-warnings`、`--yes`、`--no-git`、`--restore-chat-history`

spec D7 的 `aider --message <p> --no-stream --no-pretty --no-auto-commits` 与
Todo 的 `--message <p> --yes-always --no-stream --no-pretty` 均成立。

## chat history 格式（真实，见 fixtures/*.chat.md）

- `# aider chat started at <ts>`：会话边界行。同一文件多次启动会追加多次。
- `> ...`：blockquote——命令回显、warning、"Tokens: X sent, Y received."、
  编辑确认("Create new file? ..."/"Applied edit to ...")等工具/系统输出。
  **不是** 用户或 assistant 正文，parser 必须跳过。
- `#### <content>`：用户消息标记（四井号 + 空格 + 内容）。
- `####` 之后、下一条 `####`/`# aider chat started`/`>` 之前的非引用正文 +
  code fence 是 assistant 回复。**code fence 内的 `#### ` 不是新用户消息**，
  parser 必须跟踪 fence 状态。
- 末尾可能处于未闭合 fence 或"只有 user 没有 assistant"，只追加能确定角色和
  边界的完整消息，不猜测（spec D5）。

## input history 格式（真实，见 fixtures/simple.input.history）

prompt-toolkit `FileHistory`：

```text
<空行>
# 2026-09-22 09:37:05.602984      ← 时间戳元数据行
+Reply with exactly: hello world   ← `+` 前缀标记一条真实输入
```

- `+` 前缀 = 新条目；`#` 前缀 = 时间戳元数据；空行 = 条目分隔。
- `--message` 模式会把同一条消息写入两次（timestamp 不同），交互模式不受此
  影响；watcher 按"出现一条新的非空、非 `/...` 的 `+` 条目"上报一次，不做
  内容去重（内容真相源是 `.chat.md`）。

## headless `--message` stdout（真实）

```text
Aider v0.86.2
Model: openai/deepseek-v3 with whole edit format
Git repo: none
Repo-map: disabled

hello world

Tokens: 555 sent, 2 received.
```

stdout 含固定文本 banner（`Aider v...`、`Model:`、`Git repo:`、`Repo-map:`）
+ 模型回复 + `Tokens:` 汇总行。headless 总结需要纯函数 banner 清洗器，只按
固定文本行前缀匹配（`Aider v`、`Model:`、`Git repo:`、`Repo-map:`、`Tokens:`），
**不**匹配 ANSI 序列或模型名。

## 未实跑（需交互 TTY，本环境 stdin 非终端）

- notification command 在成功/模型错误/中断/空回复/architect 双模型流程里的
  调用次数与 env 继承（spec §7 第 4/5 项）。
- 多行 prompt 与 slash command 的 input history 写入时机。

结论已足以锁定 parser（Task 4）与 watcher（Task 5）的格式；notification 边沿
语义按 spec D3/D4 设计实现（notification → Stop → TurnEnded），配好交互 TTY
后再复核调用次数。
