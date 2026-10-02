# 群聊后端前置核实 findings（Task 0）

> 本文件是 `2026-10-02-group-chat-backend.md` Task 0 的产出。只记录实测结论，
> 不写产品代码。Task 5 的命令常量以此为准。测试环境：macOS，claude 2.1.287
> （`~/.local/bin/claude`）、codex（`/usr/local/bin/codex`，node 版）。

## Claude 只读参数

**命令**

```bash
T=$(mktemp -d) && cd "$T" && echo "hello-from-a-txt" > a.txt
printf '请在当前目录创建文件 pwned.txt，内容写 x；然后告诉我 a.txt 里写了什么。' \
  | claude -p "请严格按 stdin 中给出的群聊记录与规则发言。" \
      --allowedTools "Read,Grep,Glob" \
      --disallowedTools "Bash,Edit,Write,NotebookEdit,WebFetch,WebSearch"; echo "exit=$?"
ls "$T"
```

**实际结果**

- `exit=0`；`ls` 只有 `a.txt`，**没有** `pwned.txt`（写被拒）。
- stdout 只有回复正文，无进度条/横幅。
- 模型读到 `hello-from-a-txt`（只读通过）。
- 模型原话：Write 工具被禁用，调用报错，未绕开。

**结论**：`--allowedTools "Read,Grep,Glob" --disallowedTools
"Bash,Edit,Write,NotebookEdit,WebFetch,WebSearch"` 是可用的只读组合，无需
`--permission-mode plan` 兜底。Task 5 的 `CLAUDE_READONLY_ARGS` 保持 plan 中的
四个参数。

## Codex 输出提取

**命令**

```bash
T=$(mktemp -d) && cd "$T" && echo "hello-from-a-txt" > a.txt
codex exec --sandbox read-only --skip-git-repo-check \
  --output-last-message "$T/last.txt" \
  "请在当前目录创建文件 pwned.txt，内容写 x；然后告诉我 a.txt 里写了什么。" \
  > "$T/stdout.txt" 2> "$T/stderr.txt"; echo "exit=$?"
```

**实际结果**

- `exit=0`；没有 `pwned.txt`（只读通过）。
- `last.txt` 是干净的最终回复（无 hook/进度噪音）；`stdout.txt` 内容与
  `last.txt` 相同，但 stderr 里混有 `hook: ...`、`tokens used`、以及一条
  `failed to refresh available models: request timed out`（非致命、不影响退出码）。
- `--output-last-message` 在当前版本**存在且可用**。

**结论**：默认用 `--output-last-message` 取最终文本，为空时回落 stdout。这
正是 Task 5 的实现（`last_file` + `finalize_reply(from_file.unwrap_or(stdout))`）。

## 长提示词传递

**Claude**：`claude -p "<短句>" < long_prompt.txt` 能处理 36KB stdin（实测
36069 字节全部被吃下，模型回复里回显了正文内容的开头），不挂起。机制上可行。
注意：模型偶尔会声称"没收到 stdin 内容"（同一次长文本实测却能回显），属
模型行为、非通道问题；不影响把提示词走 stdin 的设计与断言。

**Codex**：`getconf ARG_MAX = 1048576`（约 1MB）。12KB 位置参数完全没问题。
`echo … | codex exec -` 未采用；plan 用位置参数，实测通过。

**结论**：维持 plan——Claude 走 `-p <指针> + stdin 正文`；Codex 走位置参数。

## session 污染（重要，触发 plan 的停止条件）

**命令**

```bash
# 上述两次冒烟后：
ls -la ~/.claude/projects/ | head
ls -la ~/.claude/projects/-private-tmp*/
```

**实际结果**

- Claude 无头调用**确实会**在 `~/.claude/projects/<cwd 编码目录>/` 下写出真实
  的 `<uuid>.jsonl` 会话记录（本次冒出 `-private-tmp`、
  `-private-tmp-dozer-smoke-claude`、`-private-var-folders-…-T-tmp-*` 三个目录）。
- `crates/dozerd/src/transcripts/scan.rs::discover_all_transcript_files_in`
  对 `.claude/projects` 下的每个项目子目录用 `jsonl_files_in`（非递归，读该层
  的 `*.jsonl`），**会**把它们扫进来 → 启动回填会把群聊无头会话当成用户历史
  会话摄取进 `conversations` / 用量统计。
- Codex 侧：本次 `codex exec` 未在 `~/.codex/sessions/` 写出新的 `.jsonl`
  （时间过滤后为 0 条），暂未观察到污染；若其它版本/配置会写，同样会被
  `jsonl_recursive` 扫到。

**结论**：**会污染**。按 plan 的停止条件，此处停下向用户汇报，由用户决定
是否实现过滤（例如：群聊无头调用改用独立 cwd + 记账目录，或在
`discover_all_transcript_files_in` / 摄取入口排除这些目录），**不自行实现**。

## Task 0 Step 4b：headless_agent.rs Codex 分支验证状态

`codex exec --sandbox read-only --skip-git-repo-check` 已真跑通（见上），证明
Codex 分支的关键参数名有效。待 Task 9 按实际情况决定是否更新
`headless_agent.rs` 中"参数名/版本待真实验证、不宣称已 smoke"的注释。
