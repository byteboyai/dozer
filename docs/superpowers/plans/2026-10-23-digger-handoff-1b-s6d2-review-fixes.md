# 交接任务 1b:S6d-2 审核后补测试与修正

> 给 S6d-2 原开发 agent 的修改清单,写于 2026-10-23。审核对象:`feat/agent-sessions` 的 `1ed6129`(worktree `../digger-s6d2`)。在同一分支上追加提交,不改写已有提交。
> Task 5(键盘、指针焦点、粘贴复制、拖入、输入法)仍等输入法探针结果,**不在本清单内**。

**审核结论:** Task 2、3、4、6 通过(干净快照 458 通过 / 0 失败,clippy 与基线相同)。以下是审核时发现的缺口。

## A. 补测试(必做)

对照 `docs/superpowers/plans/2026-10-18-agent-s6d2-sessions.md` Part B 各任务的 Step 1 测试清单,下面这些名字在分支里找不到对应测试。每条要么补上,要么在报告里指出已被哪个现有测试覆盖(写测试名和它断言的内容),不接受"间接覆盖"的笼统说法。

| # | 计划里的测试名 | 所属 Task | 要断言什么 |
|---|---|---|---|
| 1 | `switching_away_and_back_during_restore_does_not_double_start` | 3 | 恢复在途时切走再切回,`list_sessions` 仍只被调一次,没有第二个 shell |
| 2 | `list_collapse_button_toggles_the_pane_of_the_agent_panel` | 4 | 按下折叠按钮发出的是 Agent 面板自己的 `Msg::Shell(ShellMsg::TogglePane(agent_id))` |
| 3 | `plain_tab_close_sends_close_for_that_tab` | 4 | 点页签的 × 发出对应页签的关闭消息,不是别的页签 |
| 4 | `collapsed_list_shows_only_the_terminal_block` | 4 | 列表收起时视图只含终端块 |
| 5 | `a_new_project_starts_with_the_current_grid_so_its_first_session_is_created_at_the_right_size` | 6 | 新项目首次启动时,`create` 收到的 cols/rows 是当时的网格,不是 80×24。要覆盖 `start_grid` 这条路径 |

另外两条的计划原名与实现名不同,请确认语义没丢:

- `a_hundred_resizes_to_the_same_grid_send_once`:现有 `repeated_sync_with_the_same_grid_sends_nothing` 只覆盖"相同网格不发",没覆盖"首次发一次"。补一条 100 次同网格只发 1 次的断言。
- `selecting_a_picker_entry_creates_a_session_in_the_project_directory`:确认断言了 `create` 的 `command` 与 `cwd`,不只是"有一次 create"。

## B. 修测试的环境敏感问题(必做)

`crates/paths` 的 `config_dir_ends_with_digger`、`state_and_logs_dirs_are_under_digger_root` 断言路径字符串含 "digger"。设了 `DIGGER_DATA_DIR` 且路径不含该词时会失败(审核时在 `/private/tmp/.../s6d2-review.XXXX` 下复现)。

- 改成与环境无关:要么测试内显式清掉或设定 `DIGGER_DATA_DIR`,要么断言相对结构而不是字符串里含 "digger"。
- 验证方式:在路径不含 "digger" 的目录里,设 `DIGGER_DATA_DIR` 与临时 `HOME` 跑 `cargo test --workspace`,全部通过。
- 这是 S6a(`feat/diggerd`)遗留的问题。修在本分支即可,报告里注明来源。

## C. 评估并记录(不要求改代码,除非结论是确有问题)

`AgentPanel::close_project` 会 abort 还在创建中的 `state.pending` 任务。若守护进程那边 `create` 已成功,会话成为孤儿,没人去 kill。

- 先写一个能复现或证伪的测试(假服务:`create` 慢返回,期间关项目)。
- 若确认会留下孤儿:在报告里写明,并给出最小修复方案;**修复前先向用户确认**,因为可能需要库配合。库不改。
- 若证伪(例如库侧已处理):把测试留下作为护栏,报告写清原因。

## 约束与交付

- 基线:`feat/agent-sessions` 的 `1ed6129`;开工先核对,不符就停下来问。库不改。
- 隔离环境:`DIGGER_DATA_DIR` 与临时 `HOME`,不碰真实数据和常驻 `dozerd`。
- 完工前 `git archive HEAD` 解到独立临时目录(目录名不要带 "digger"),在其自己的 `target` 下跑全量:`cargo test --workspace --no-fail-fast`、`cargo clippy --workspace --all-targets`、`cargo metadata --locked --offline`。
- 报告追加到 `docs/superpowers/reviews/2026-10-<日期>-s6d2-fixes-report.md`:测试总数(基线 458 + 新增)、测试名守恒、A 表每条的结果、B 的复现与修复、C 的结论。每条"通过"附命令和退出码,是最终代码上刚跑的。
- 不合并 `main`、不打 tag、不推送。停在 `feat/agent-sessions`。
