# S6d-2 Part A 审核记录

日期:2026-10-21。被审:`feat/panel-agent-restore`(worktree `../dozer-restore-patch`,顶端 `adae877b`,单个提交,基于 `feat/panel-agent-2` 的 `8cd9bdc9`)。
计划:`plans/2026-10-18-agent-s6d2-sessions.md` Part A / Task 1。实施报告:该分支内 `reviews/2026-10-21-s6d2a-restore-patch-report.md`。

## 结论

**代码通过,报告需补。** 无阻塞缺陷;行为守恒与黄金测试的证据成立。报告本身不满足计划 Step 5 的要求(见 N1),需要实现者补写;另有 4 条非阻塞观察。

## 我自己实跑的验证(干净快照 `/private/tmp/s6d2a-clean`,`git archive HEAD`,用快照自己的 `target`)

| 项 | 结果 |
| --- | --- |
| 分支基线 | `8cd9bdc9` 是 HEAD 的祖先 ✔;工作区干净 |
| `cargo metadata --locked --offline` | 退出码 0(`Cargo.lock` 本期无改动) |
| `cargo test --workspace --no-fail-fast` | 3433 通过 / 3 失败 / 5 ignored。3 个失败正是 `git_log` 三条 dogfood(archive 无 `.git`,环境原因)。3433 + 3 = **3436 = 基线 3426 + 10 条新增**,与计划预期一致。`dozer-app` 1248 + 3 = 1251,与基线相同(搬走/删除的 Dozer 侧代码没有带走任何测试) |
| 测试名守恒 | `8cd9bdc9` 的每个测试函数名在 HEAD 仍在(丢失 0);新增 10 条:`fetch_restored_returns_only_live_sessions_of_the_project`、`a_failing_attach_skips_that_session_only`、`adopt_restored_*` 三条、`resize_message_resizes_every_live_tab_and_skips_exited_ones`、`scroll_moves_the_display_of_the_active_tab_only`、`selection_start_and_update_apply_to_the_active_tab_only`、`scroll_and_selection_without_an_active_tab_are_ignored`、`term_message_maps_every_term_event_variant` |
| 黄金测试 | 本期对 `fixtures/`、`dozer-client`、`Cargo.lock` **0 改动**;`session_stream_through_the_service_matches_the_golden`、`session_protocol_encoding_golden` 在快照里均 ok |
| `cargo clippy --workspace --all-targets` | 退出码 0;总警告 41(与 S6c-2 顶端相同),`dozer-app` bin 19(同基线);`dozer-panel-agent` 与本期改动文件**零警告**。注:报告里"all-target clippy 暴露 `workspace/view.rs` 类型不匹配"在我这里不成立,clippy 全量通过 |
| `rustfmt --check --edition 2024` 对本期改动的 8 个 `.rs` | 全部干净 |
| `check_migration.py` / `check_panel_boundary.py` | 退出码 0 / ok(213 refs in 5 files) |
| `scripts/check-log-scope.sh` | 只剩 `main` 上既有的 `preview_panel.rs:233` |
| `cargo tree -p dozer-panel-agent` | 无 `dozer-app/client/core`、`dozerd`、`wry`、`winit`、`portable-pty`、`russh` |

手工对照代码:
- `adopt_restored` 的页签构造与原 `from_restore` 循环逐字段一致(`set_answer_dynamic_color`、喂快照丢应答、`ingest_osc(&snapshot)` 在入栈前、`tab_id` 自 `next_tab_id` 递增、`backend: Daemon`)。
- `fetch_restored` 的过滤条件 `alive && project_id == Some(id)` 与原 `fetch_project_restore` 相同;单个 `attach` 失败只跳过该会话。
- 滚动/选区:Dozer 的 `TermTarget::Shared` 三个分支改为经 `Message::Agent` 转库,`active_project_id()` 与原 `with_focused_project` 取的是同一个项目;`TermTarget::SshPanel` 仍走 `ssh_active_tab_mut()`,逻辑未变。
- 被取代的 `Workspace` 侧 `forward_events` 与 `Message::AgentStateChanged` 已删,没有残留调用方(编译与 clippy 通过)。

## 发现

### N1 报告不满足计划 Step 5(小,需实现者补)
- 报告自己承认"干净快照复验、metadata、迁移与边界脚本尚未运行";计划要求这些写进报告并给出命令与退出码。
- 没有给出测试总数(应为 3436 = 3426 + 10)和各 crate 数字,也没有"测试名守恒"证据。
- 多条"通过"标注为"在最终清理之前跑的",即最终代码上没有一次完整重跑——与计划实现者须知 ⑥ 冲突。我这边已在最终代码的干净快照上全量重跑且结果一致,所以代码没问题,但报告必须据实补全。
- clippy 一条描述不准(见上),请更正为实测。
- 另:Step 1 要求"把接缝清单放进提交说明",提交说明里只有标题,清单只在报告里。可接受,不要求改提交。

## 非阻塞观察

1. **没有 Dozer 侧的 `Shared` 滚动/选区转库回归测试。** 计划 Step 2 写了"Shared 转库后与旧行为一致、SSH 三条路径不受影响"。这次只有库侧的测试;Dozer 侧靠"代码对照"成立,没有测试钉住(`App` 夹具基础设施缺口,已知)。目测清单里已列滚轮/选区/⌘C 与 SSH 滚动,验收时重点看。
2. **测试名与计划不完全一致:** 计划写 `selection_start_and_update_apply_to_the_active_tab_and_the_selected_text_follows`,实际为 `…_apply_to_the_active_tab_only`;断言覆盖了"选中文本随之变化"(`"sec"`)和"另一个页签不受影响",语义满足,仅命名。
3. **`list_sessions` 失败不再记日志。** 旧 `fetch_project_restore` 对 `list` 失败有 `log_warn`,新 `fetch_restored` 直接返回空集合(`let Ok(..) else { return Vec::new() }`)。调用方因此可能把"daemon 暂时不可用"当作"没有会话"而新开一个 shell。行为上与旧实现相同(旧实现也是空集合),只是丢了一条诊断日志。Digger 的 Task 3 用到时建议在调用方补日志,或让库返回 `Result`。
4. **每次滚轮事件都构造 `services_for` 和 `UpdateCtx`。** `agent_panel_message` 对每条滚动消息都走一遍(旧路径直接改模型)。开销很小,但滚轮事件频繁;若验收时觉得滚动发飘再优化,目前不处理。另外旧路径的 `with_focused_project` 顺带写 `preview_poll_project`,滚动/选区不再写它——只影响预览命令轮询的"当前项目"记录,其它消息仍会写,忽略不计。
5. 小:库里 `adopt_restored` 的 `TerminalModel::new(80, 24)` 是字面量,Dozer 里是常量 `DEFAULT_COLS/ROWS`(值相同)。库不依赖 Dozer 常量,合理;之后 `adopt` 后产品要紧跟一次 `Resize`(Dozer 已有对应逻辑,`update.rs:2693` 附近)。

## 未做/不属于本期

- Part B(Digger)按计划未执行;未合并、未打 tag、未推送。
- 目测清单在报告里,等整条 Dozer 链路完成后一次性验收(重启后会话仍在且内容连续、输入、终端滚轮与鼠标选区/⌘C、SSH 面板滚动与选区不受影响)。

## 链路状态

Dozer 侧叠分支:S6a Part A(`b7763bb8`)→ S6b(`726f86c7`)→ S6c-1(`dc513cbd`)→ S6c-2(`8cd9bdc9`)→ **S6d-2 Part A(`adae877b`,待补报告)**。剩 S6e-1,再人工验收。
