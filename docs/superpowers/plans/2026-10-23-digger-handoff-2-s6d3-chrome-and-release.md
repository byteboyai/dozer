# 交接任务 2:S6d-3 页签外观与发布

> 给开发 agent 的任务说明,写于 2026-10-23。权威细节在下面的 plan 里,本文只是入口与约束汇总。

**目标:** 把 S6d-2 的朴素页签换成 Dozer 同款外观。补齐溢出菜单、拖拽换位、悬停高亮和延迟提示,与 Dozer 逐状态对照验收,做压力和同机并存回归。**完成 Agent 面板迁移,Dozer 与 Digger 各发布一个新版本。**

**Plan(只读):**
`docs/superpowers/plans/2026-10-18-agent-s6d3-chrome-and-release.md`(Dozer 仓库)

先读文件顶部修订。关键两条:

- 页签的拖拽换位用库的 `dozer_panel_agent::update::reorder_term_tab(state, from, to) -> bool`。
- `PanelDisplay::drag_surface(&self, content, index)` 只有两个参数。手势(按下武装、位移阈值、组校验)全在产品的拖拽状态机里,库不管。

## 基线

- **依赖任务 1 与 1b 完成。** 2026-10-23 用户决定:S6d-2 审核修复(`2026-10-23-digger-handoff-1b-s6d2-review-fixes.md`)完成并复核通过后即开始,**不等** S6d-2 Task 5(键盘与输入法)。从 `feat/agent-sessions` 的修复后顶端切出新分支 `feat/agent-chrome`,worktree 叫 `../digger-s6d3`。
- Task 5 稍后另行合入 `feat/agent-sessions`,会改 `product.rs` 与 `adapters/agent.rs`,与本任务有冲突风险:少碰这两个文件里键盘、焦点、`ImeSink` 相关的部分,页签与菜单改动尽量收在 `agent_tabs.rs`、`agent_display.rs`。
- 真机对照与发布前的验收仍需要 Task 5 的键盘输入,发布步骤要等 Task 5 合入后再做。
- 依赖已是 `libs-2026-10-23b`。
- 测试基线以任务 1 的最终数字为准,先自己重跑确认。

## 要点

- 全部改动在 `digger-desktop`。
- 页签外观对照 Dozer 的 `chrome/tab_widget.rs`,用 byteui 的 `tabs::tab_core` 和 `tab_strip`。
- 悬停用"阶跃"(进入即满、离开即空)代替动画。
- 菜单外框复用 S6d-2 的 `adapters/menu_overlay.rs`。
- 新增 `examples/agent_gallery.rs`,接受 `--state <name>`,用假会话摆出固定状态,供与 Dozer 对照截图。
- 对照验收需要用户在 Dozer 里摆出同一状态。画廊只覆盖 Digger 一侧,对照步骤写进"待用户"。
- opencode 验收要覆盖 2.x:日志里没有 `failed to load plugin`,状态点随回合更新。
- 可选:顶栏接 `Client::daemon_is_outdated()` 的"需重启"徽标(Dozer 的做法见 `c016794b`)。

## 约束

- 库不改。缺口(尤其是拖拽状态归属)只报告,不绕开。
- 隔离的 `DIGGER_DATA_DIR` 和临时 `HOME`,不碰真实数据和常驻 `dozerd`。
- **发布是对外动作,先问用户再执行。** 包括升版本、`scripts/build-macos-app.sh release`(Dozer)、`scripts/release.sh`(Digger)、打 tag、推送、安装到本机。
- 压力问题多半出在库,报告而不绕过。

## 交付

- 交付要求同任务 1:`git archive` 重跑测试、最终代码上的实测数字、`git status --ignored`。
- 报告写到 `docs/superpowers/reviews/2026-10-<日期>-s6d3-report.md`(Digger worktree 内)。
- 在 Task 6 里把页签和菜单的重复件记入技术债。
- 发布前停下来等用户确认。
