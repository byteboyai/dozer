# 交接任务 1:S6d-2 会话功能(Digger Part B)

> 给开发 agent 的任务说明,写于 2026-10-23。权威细节在下面的 plan 里,本文只是入口与约束汇总。

**目标:** 让 Digger 的 Agent 面板真正可用。每个项目一份 Agent 状态,面板首次可见时恢复该项目在 `diggerd` 里的存活会话,没有就新开一个 shell。另外要做:新建会话选择器、终端渲染与键盘/粘贴/复制/拖入/输入法、关页签确认、窗口缩放时网格尺寸同步、关项目页签杀会话、关窗口只 detach 且等在途的 kill。页签用朴素版(选中加关闭),Dozer 同款外观留给 S6d-3。

**Plan(只读,不要改):**
`docs/superpowers/plans/2026-10-18-agent-s6d2-sessions.md`(Dozer 仓库)

- 只执行 **Part B**(约第 76 行起)。
- 先读文件顶部的 2026-10-22 和 2026-10-23 修订,冲突处以它们为准。
- Part A 属于 Dozer 侧,已完成,不要做。

## 基线

- 仓库 `/Users/chrischiang/Projects/CoralProjects/byteboy/digger`。
- 新建 worktree `../digger-s6d2`,分支 `feat/agent-sessions`,从 `feat/diggerd` 切出。
- 切之前先确认 `git rev-parse --short feat/diggerd` 是 `46bb4c9`,不是就停下来问。
- 依赖已钉 `libs-2026-10-23b`,`Cargo.lock` 已提交,不需要本地 patch。
- 测试基线是 423 个通过,0 失败。clippy 只允许 `block v0.1.6` 那 1 条警告。

## 前置阻塞

- 输入法探针(S6d-1 Step 4)的结果出来之前,**键盘和输入法的 Task 不得开始**。其余部分(每项目状态、恢复、选择器、尺寸同步、关闭语义)可以先做。
- 探针结果有失败时,不要自己选退路,报告给用户。

## 约束

- 库不改。库缺东西就停下报告,不要在 Digger 里绕开。
- 不碰 Digger 主工作区的未提交改动,不碰 `feat/preview-pairing`。
- 所有回归用隔离的 `DIGGER_DATA_DIR` 和临时 `HOME`。不要动真实 `HOME` 和用户常驻的 `dozerd`。
- 本期不发布版本。合并 `main`、打 tag、推送要先问用户。

## 交付

- 新增文件提交后用 `git status --ignored` 确认没被 `.gitignore` 吞掉。
- 完工前把 `git archive HEAD` 解到独立临时目录,在它自己的 `target` 下重跑测试。
- 报告里每条"通过"都是最终代码上刚跑的结果,附命令、退出码、测试总数、测试名守恒和 clippy 实测数。
- 报告写到 `docs/superpowers/reviews/2026-10-<日期>-s6d2-report.md`(Digger worktree 内)。
- 需要 GUI 点击或输入法的项写进报告的"待用户",不要卡住。
- 停在 `feat/agent-sessions`。
