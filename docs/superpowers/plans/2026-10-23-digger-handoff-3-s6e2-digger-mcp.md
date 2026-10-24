# 交接任务 3:S6e-2 `digger-mcp`

> 给开发 agent 的任务说明,写于 2026-10-23。权威细节在下面的 plan 里,本文只是入口与约束汇总。

**目标:** 给 Digger 接上 MCP:`digger-mcp` 二进制、打包、Agent 会话创建时自动注册、`TurnEnded` 后刷新 Project 面板记忆。

**Plan(只读):**
`docs/superpowers/plans/2026-10-18-mcp-s6e2-digger.md`(Dozer 仓库)

先读顶部修订,再看 Global Constraints 里的 Q1–Q3 和 D3、D5。

## 基线(审核方的判断,不是 plan 原文)

- plan 写的是"从已含前置的 `main` 切出",但 `ensure_mcp` 要加在 `DiggerHooks` 上,而 `digger-hook` 在 `feat/diggerd` 里,`main` 上还没有。
- 建议从 `feat/diggerd`(`46bb4c9`)切 `feat/digger-mcp`,worktree 叫 `../digger-s6e2`。
- 开工先核对这一点,不符就停下来问。
- 依赖已钉 `libs-2026-10-23b`,`Cargo.lock` 已提交。测试基线 423 个通过,0 失败。

**可以并行:** 任务 1、2 不改 MCP 相关文件。但 `product.rs` 和 `Cargo.toml` 可能冲突,合并时留意。

## 要点

- **工具集恰为三个:** `digger-mcp serve` 在 `DIGGER_SESSION_ID` 下,`tools/list` 恰为 `write_memory`、`list_memories`、`get_memory`。描述和参数 schema 要与 Dozer 的同名工具逐字相同(共用同一份代码)。不得出现待办、预览、定位、编辑、总结工具。
- **与 Dozer 并存:**
  - `digger` 与 `dozer` 条目互不覆盖,卸载互不影响。
  - Dozer 会话里的 `digger-mcp` 和 Digger 会话里的 `dozer-mcp` 进入零工具安静模式。Agent 的 MCP 状态里这不是失败。
  - `digger-mcp` 不读写 Dozer 的目录、socket、数据库。
- **安装时机:**
  - `ensure_mcp` 对支持的 Agent(Claude、Codebuddy、Codex、Opencode、Qodercn)每个进程只装一次。
  - 退出码 `3` 记为"被保护拒绝",登记警告,不重试。
  - 缺二进制不阻塞建会话。
  - Aider、Goose、V8agent、Unknown 空操作。
- **记忆联动:** 对 `AgentHostEvent::TurnEnded`,当 Project 面板可见、是当前项目且状态已存在时,调 `ProjectPanel::refresh_memories`。它只调库的 `on_activate`,不要调会补 README、git、磁盘占用的 `activate`。这是相对 Dozer 的有意改进。
- **真机验收:** 各 Agent 对零工具服务的显示(连接或空工具,不是失败)只能真机看。写进报告,GUI 项标"待用户"。

## 约束

- 不改 `dozer-mcp` 库。缺东西就报告。
- 不发布产品版本,随任务 2 的发布一起带上。
- 所有回归用隔离的 `DIGGER_DATA_DIR` 和临时 `HOME`,不碰真实 `HOME` 和常驻 `dozerd`。
- 不碰 Digger 主工作区的未提交改动,不碰 `feat/preview-pairing`。
- 合并 `main` 要先问用户。

## 交付

- 交付要求同任务 1:`git archive` 重跑测试、最终代码上的实测数字、`git status --ignored`。
- `release.sh` 的校验要包含 `digger-mcp` 二进制。
- 报告写到 `docs/superpowers/reviews/2026-10-<日期>-s6e2-report.md`(Digger worktree 内)。
- 文档:ADR-015 补一节"MCP"(`digger-mcp` 只暴露记忆工具、与 `dozer-mcp` 并存与安静模式)。`2026-10-18-agent-mcp-s6e-design.md` 的状态更新由审核方在 Dozer 仓库做,实现者在报告里写清结果(含各 Agent 对零工具服务的实测表现、"只装一次"的取舍、Project 面板刷新是有意改进)。
- 记录 S6e-3(待办面板、预览引擎)的依赖。
- 停在 `feat/digger-mcp`。
