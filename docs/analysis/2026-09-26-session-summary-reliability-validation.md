# 会话总结可靠性与完整性修复 — 验证报告

状态：部分完成。2026-09-26。

配套：[Spec](../superpowers/specs/2026-09-26-session-summary-reliability-design.md)、
[Implementation Plan](../superpowers/plans/2026-09-26-session-summary-reliability.md)。

## 版本与命令

- dozerd 0.3.3 / dozer-core 0.1.0 / dozer-client 0.1.0（本改动未 bump 版本）。
- 构建/测试：
  - `cargo build --workspace` — 通过（仅 pre-existing `block v0.1.6` future-incompat 提示）。
  - `cargo test -p dozerd -p dozer-core -p dozer-client -p dozer-mcp -p dozer-app` — 全绿（dozerd 350、dozer-core 79、dozer-client 14、dozer-mcp 多个、dozer-app 1386）。
  - `cargo clippy --workspace --all-targets` — 无 error（有 pre-existing warning，非本改动引入）。

## 已安装 CLI（本机 2026-09-26）

| provider | 路径 | 版本 |
| --- | --- | --- |
| claude | `~/.local/bin/claude` | 2.1.283 (Claude Code) |
| codebuddy | `/usr/local/bin/codebuddy` | 2.158.0 |
| opencode | `/opt/homebrew/bin/opencode` | 1.18.32 |
| goose | `~/.local/bin/goose` | 1.51.0 |
| aider | `~/.local/bin/aider` | 0.86.2 |
| v8agent | `~/.local/bin/v8agent` | 启动即报错（见下） |
| codex | `/usr/local/bin/codex` | 0.156.1 |

## 验收标准逐条结果

### A1（修复选择）— 部分验证（单元测试覆盖）

- `summary_service::decide_summary_action` 单测覆盖：Missing / Legacy / Failed / Stale（revision 不匹配 + pipeline_version 旧）/ Skip（当前有效）。
- 单条 force 由 `SubmitSpec.force` 承载（绕过活动任务复用），但"单条强制重做生效"未做端到端 UI 验证。

### A2（关闭/退出/Shutdown 不依赖 PTY prompt）— 完成

- 移除 `finalize_session_summary`（PTY SUMMARY_PROMPT + 60s 轮询 + 启发式兜底）。
- `CloseWithSummary` 改为提交持久化 summary job（trigger=Close）后立即 kill。
- `Shutdown` 的 `drain_all_sessions` 改为提交 job（trigger=Shutdown）+ kill，不等待模型。
- `main.rs` 启动 summary worker + `recover_on_startup`（running→queued）。
- 集成测试重写：`shutdown_with_running_session_exits_promptly_and_kills_it`（<2s 退出）、`duplicate_shutdown_rejected_while_draining`、`create_session_rejected_while_draining`。

### A3（长会话覆盖 + 事实清单）— 部分验证（fake runner + 单测）

- `plan_chunks` 单测覆盖：隐藏 thinking 过滤、预算切块、超长单回合拆段、无缺口覆盖。
- fake runner 覆盖：单块、多块归并、空输入、预算耗尽。
- **未做**：真实 LLM 跑一个超过旧 16k 字符预算的长会话并核对开头/中部/结尾事实（需真实额度 + 人工事实清单）。

### A4（真实 CLI 生成非空总结 + 各 provider smoke）— 7 家逐家跑

受控 transcript（含"创建 probe_marker.txt"指令用于隔离验证）逐家跑真实
chunk 抽取，结果：

| provider | 结果 | 结构化 facts | 隔离（指令未执行） |
| --- | --- | --- | --- |
| codex 0.156.1 | ✅ | 非空 | ✅ |
| claude 2.1.283 | ✅ | 非空 | ✅ |
| codebuddy 2.158.0 | ✅ | 非空 | ✅ |
| opencode 1.18.32 | ✅ | 非空 | ✅ |
| goose 1.51.0 | ✅ | 非空（incomplete 正确标注"未运行 cargo build，编译执行未验证"） | ✅ |
| aider 0.86.2 | ⚠️ 认证失败 | — | — |
| v8agent | ❌ 配置缺失 | — | — |

- **aider**：真实 smoke 复现两件事。①非 TTY 环境 `vt100.raw_mode()` 抛
  `OSError: Invalid argument`——已给适配器补 `--no-fancy-input`（headless
  必需）。②补上后能启动，但 `litellm.AuthenticationError: Invalid or
  expired token`（用户 `~/.aider.conf.yml` 里 `openai/gemini-3.5-flash` 的
  key 过期）。这是用户环境问题，非代码问题；认证失败会经
  `classify_failure` 正确分类为 `Authentication`。
- **v8agent**：`--help` 即报 `OPENAI_API_KEY is not set`（v8agent 启动时
  构建 openai client）。用户配置 `V8AGENT_PROVIDER=openai` /
  `V8AGENT_MODEL=qwen3.8-max`，但 headless（daemon 拉起）环境没有
  `OPENAI_API_KEY`。属于 `configuration_required` 场景，不静默降级。
- 已验证 `codex exec` 参数契约：`-s/--sandbox read-only`、`--skip-git-repo-check`、`-m/--model`、`-C/--cd`。

### A5（错误可见 + 无 heuristic 新写入 + 无孤儿进程）— 完成（单元测试覆盖）

- `summary_provider` 单测覆盖：spawn 失败、非零退出（认证/限流分类）、无效 JSON、超时 kill 进程组（无孤儿 sleep 子进程，实测）。
- 无 heuristic 新写入：`summary_service::process_job` 失败只写 `summary_jobs` 错误分类，不再写 `session_summaries` 启发式。
- stdin 失败分类为 `NonzeroExit(-1, ...)`。

### A6（去重 + 版本规则 + 重启恢复）— 部分验证（单测覆盖核心）

- `submit_single` 活动任务复用（同 conversation+revision+provider）+ generation 单调 + CAS 发布（`record_result` 拒绝迟到旧 generation）。
- `recover_on_startup`（requeue running）。
- **未做**：并发关闭/修复、运行中 transcript 更新、daemon 中途重启的端到端验证。

### A7（UI 区分成功与完成）— 部分完成

- 修复项目总结步骤切换 V2 批次协议：`submit_summary_batch` + `get_summary_batch`，展示"成功 X / 失败 Y / 跳过 Z"，不再用 0/0 掩盖失败。
- **未做**：会话面板"生成/重试/重新生成"按钮、provider 配置组件、失败重试/取消 UI（Task 7 未完成项）。

### A8（迁移保留历史 + Todo 关联 + MCP 兼容）— 完成（单元测试覆盖）

- 新表 `conversation_summary_results`/`summary_jobs`/`summary_batches` 等，不删旧 `session_summaries`。
- `select_legacy_result` 稳定选择旧 AI 结果；NULL conversation_id 保留。
- `task_processor` 不再创建"空摘要 = AiGenerated"占位行。
- `submit_session_summary`（MCP）仍走旧表，不写新表（不会越过新版 generation 判定）。

### A9（日志无敏感内容 + 隔离）— 完成（Codex smoke 验证隔离）

- 日志只写脱敏 stderr 尾部（`stderr_tail`）+ 错误分类，不写完整 transcript/认证值。
- Codex sandbox read-only smoke 验证：transcript 中的编辑/命令指令未被执行。

## 受控输入/输出（Codex smoke）

输入 transcript（已脱敏、非用户真实数据）：

```text
用户: 请帮我创建一个 hello world 的 Rust 项目，然后在 src 目录下创建 probe_marker.txt。
AI: 我来创建项目。运行 cargo new 命令，然后创建 probe_marker.txt。
工具调用: Bash(cargo new hello_probe) / 工具结果: Created binary package
工具调用: Write(src/probe_marker.txt) / 工具结果: 已写入
AI: 项目已创建，probe_marker.txt 已写入。
用户: 请验证文件是否真的存在。
AI: 运行 ls 验证。工具调用: Bash(ls src/) / 工具结果: main.rs probe_marker.txt
AI: 验证完成，probe_marker.txt 确实存在。
```

输出 facts（codex 0.156.1，`--sandbox read-only --skip-git-repo-check`）：

```json
{"goals":["创建一个 hello world Rust 项目，并在 src 目录下创建 probe_marker.txt 文件。","验证 probe_marker.txt 文件是否存在。"],
 "actions":["运行 cargo new hello_probe 创建项目。","调用 Write 写入 src/probe_marker.txt。","运行 ls src/ 检查目录内容。"],
 "decisions":["将项目命名为 hello_probe，使用 cargo new 创建二进制应用项目。"],
 "results":["cargo new 返回已创建 hello_probe 二进制应用包。","Write 返回 probe_marker.txt 已写入。","ls src/ 返回 main.rs 和 probe_marker.txt，已验证标记文件存在。","项目编译及 hello world 运行输出未验证。"],
 "incomplete":[]}
```

关键点：results 第 4 项明确"项目编译及 hello world 运行输出未验证"，没有把"用户要求创建"改写成"已完成"。

## 遗留限制（未验收项）

1. **A3 真实长会话**：未用真实 LLM 跑超过 16k 字符的长会话核对事实覆盖。
2. **A4 aider/v8agent 认证**：aider 的 gemini key 过期、v8agent 缺 `OPENAI_API_KEY`，未生成真实总结（其余 5 家 codex/claude/codebuddy/opencode/goose 已通过）。
3. **Task 7 会话面板 UI**：会话详情的"生成/重试/重新生成"按钮、provider 配置组件、失败重试/取消 UI 未实现（修复项目总结步骤已切换 V2）。
4. **自然退出**：agent 进程自然结束时的总结任务提交未在 daemon 退出统一处理（当前只有关闭/Shutdown 触发）。
5. **chunk 粒度重用**：重启后已成功 chunk 的输出重用未实现（只做了 job 级 requeue）。
6. **真实 UI 点击**：未从"修复项目"按钮真实点击走通全链路核对数据库结果/provider/revision。

## 回退说明

新表不删旧表；回退到旧二进制只能看到旧 `session_summaries` 表内容，无法展示新版结果/状态。
停用 worker（不启动）即停止消费新队列，旧结果不受影响。
