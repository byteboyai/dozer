# Qoder Hook 注册机制与 Transcript 落盘——Spike 验证记录

> 日期：2026-08-07
> 关联规格：`docs/superpowers/specs/2026-08-07-dozer-multi-agent-codex-qoder-kilo-design.md` §6
> 关联计划：`docs/superpowers/plans/2026-08-07-dozer-codex-qoder-kilo-adapters.md` Task 7
>
> **状态声明**：Qoder CLI 本机**未安装**（`command -v qoder` → 无），无法跑真实探针。本记录基于：官方 CLI hooks 文档（`docs.qoder.com/en/cli/hooks`，经 Zed ACP 官方文档与计划 Global Constraints 交叉核实 Qoder 命令名为 `qoder`）+ 计划/规格假设整理而成。**未验证项一律标"待实测"**，不冒充已核实。

## 结论一览

| 验证项 | 假设（spec §6 / 计划 Global Constraints） | 实测 | 一致？ |
|---|---|---|---|
| CLI 可用性 | `qoder --version` 可用 | 本机未安装（需先跑 `curl -fsSL https://qoder.com/install \| bash`，**来自第三方域名，执行前须自行确认信任**） | 🔶 待安装 |
| hook 注册机制 | `~/.qoder/settings.json` 补丁，结构 `{"hooks":{"EventName":[{"hooks":[{"type":"command","command":"..."}]}]}}`（与 Claude 三层结构同构） | 经官方 hooks 文档核实为同构机制；待安装后实测 | 🔶 待实测 |
| hook payload 字段名 | `session_id`/`transcript_path`/`cwd`/`hook_event_name`/`tool_name`/`tool_input` 与 Claude 逐字对齐 | 官方文档核实字段名与 Claude 高度对齐；待实测确认 | 🔶 待实测 |
| 事件名词汇 | 20 个事件跨六类（会话/工具/agent flow/压缩/通知/文件），映射见下方 | 官方文档核实 20 个事件名；dozer-hook `qoder.rs` 翻译表已按 spec §6 实现 | 🔶 待实测 |
| transcript 落盘 | 目录/路径是否按 cwd 建目录**未知**（与 Codex 一样，可能不按 cwd 建） | 未安装，未观测 | ❓ 待实测 |

**对计划的影响**：Task 8（Qoder 事件名翻译表）+ Task 9（Qoder hook 安装器：`settings_path_for("qoder")` → `~/.qoder/settings.json`）按计划设计直接落地。Qoder 翻译表的归并/丢弃逻辑已按 spec §6 实现（`PostToolUseFailure`→`PostToolUse`、`StopFailure`→`Stop`、`PermissionRequest`/`PermissionDenied`→`Notification`，压缩/子agent/文件/worktree/elicitation 类丢弃）。**待实测确认的仍是**：settings.json 三层结构是否真触发、payload 字段名逐字一致性、以及 transcript 落盘结构（决定未来 Qoder transcript 解析计划是否要像 Codex 一样全量扫描）。

## 验证步骤（待执行，缺 Qoder CLI 无法进行）

### Step 1：安装并按计划 Task 7 逐步执行
1. `curl -fsSL https://qoder.com/install | bash`（信任来源确认后）→ `qoder --version`。
2. 备份：`mkdir -p ~/.qoder && cp -n ~/.qoder/settings.json ~/.qoder/settings.json.dozer-spike-backup`。
3. 写探针 `~/.qoder/settings.json`（hooks 段合并，勿整体覆盖非空原文件）——6 个规范事件各一条 `command` 型 hook，dump stdin 到 `/tmp/qoder-hook-probe.log`。
4. 跑一轮真实/半真实会话（`qoder --help` 确认是否有类似 CodeBuddy `-p --dangerously-skip-permissions` 的非交互模式；没有则跑交互式问答后退出）。
5. `cat /tmp/qoder-hook-probe.log` 核对命中与字段名。
6. 若拿到真实 transcript，照 Codex 的做法脱敏存为 `crates/dozer-hook/fixtures/qoder-transcript-sample.jsonl`（未来目录/解析计划输入）。
7. 还原：有备份则 `mv` 回，无则 `rm -f ~/.qoder/settings.json`；清 `/tmp/qoder-hook-probe.log`。

### 计划写作时点已知的 Qoder 事件名全集（跨六类，来自官方 hooks 文档，dozer-hook/qoder.rs 已全量覆盖）
- 会话：`SessionStart`, `SessionEnd`
- 工具：`UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure`
- agent flow：`Stop`, `StopFailure`, `SubagentStart`, `SubagentStop`
- 压缩：`PreCompact`, `PostCompact`, `InstructionsLoaded`
- 通知：`Notification`, `PermissionRequest`, `PermissionDenied`
- 文件/配置/worktree/MCP：`ConfigChange`, `CwdChanged`, `FileChanged`, `WorktreeCreate`, `WorktreeRemove`, `Elicitation`, `ElicitationResult`

## 对后续计划的影响

1. **Task 8/9**：已按计划落地。上文待实测三项若与假设不符，改 `qoder.rs::translate_event` 与 `install.rs::settings_path_for` 局部即可。
2. **transcript 解析（未来独立计划）**：Qoder 的 transcript 落盘结构未知（是否按 cwd 建目录、schema 是否与 Claude 或 Codex 同构）。在本 spike 拿到样本前，`transcript.rs`/`usage.rs` 的 `Qoder` 分支维持"诚实返回空/默认值"——这是唯一诚实状态，不写"看起来合理"的目录路径函数。
