# S6c-2 审核记录

日期:2026-10-21。被审:`feat/panel-agent-2`(worktree `../dozer-panel-agent-2`,顶端 `8cd9bdc9`,基于 `feat/panel-agent-1` 的 `dc513cbd`)。
计划:`plans/2026-10-18-agent-s6c2-views.md`。实施报告:该分支内 `reviews/2026-10-21-s6c2-views-report.md`。

## 结论

**通过。** 无阻塞缺陷,没有需要实现者返工的项;有 4 条非阻塞观察,其中 2 条要带进后续计划。

## 我自己实跑的验证(干净快照,`git archive HEAD`)

| 项 | 结果 |
| --- | --- |
| 分支基线 | `dc513cbd` 是 HEAD 的祖先 ✔ |
| `cargo metadata --locked --offline` | 退出码 0 |
| `cargo test --workspace --no-fail-fast` | 3419 通过 / 7 失败 / 5 ignored。7 个失败全是环境原因:① 3 条 `git_log` dogfood(archive 无 `.git`,与前几期相同);② 4 条 `dozerd` `product_isolation`——是**我的**操作问题(让快照共用了另一个 target 目录,而该测试按 `<repo>/target/` 找示例二进制)。把快照的 `target` 链到共用目录后重跑 `cargo test -p dozerd`:`product_isolation` 4 通过,`dozerd` 全部 ok。3419 + 4 + 3 = 3426,与报告一致 |
| 测试名守恒 | `dc513cbd` 的每个测试函数名在 HEAD 仍在(无丢失);函数数 3407 → 3425(+18)。报告里 `dozer-app` 1261 → 1251 的减少对应搬进库的测试 |
| 搬走的 transcript 逻辑 | 把 `dozer-app/src/transcript.rs`(`dc513cbd`)与 `dozer-panel-agent/src/transcript_tail.rs` 做空白归一化比对:4 个函数(`latest_model_mode_and_activity`、`extract_line_activity`、`join_text_blocks`、`truncate_activity`)**逐字相同**;12 条搬走的测试**函数体逐字相同**,没有改期望值 |
| 两份 S6c-1 黄金 | `crates/dozer-app/tests/fixtures/` 与 `dozer-client` 在本分支 **0 改动**;`session_stream_through_the_service_matches_the_golden`、`session_protocol_encoding_golden` 均 ok |
| i18n 文案 | 库内 `locales/{en,zh-CN}.json` 11 个 key 与 `dozer-app/locales/host/*.json` 的原文逐项比对(`close_confirm.cancel/confirm` 对 `common.cancel/close`):**全部相同**。locale 取自 `app.locale.active_locale()`,没有写死,中英文切换路径保留 |
| `cargo clippy --workspace --all-targets` | 退出码 0;库与本期新增文件**零警告**;`dozer-app` bin 警告 22 → 19(减少的是搬走的代码) |
| `rustfmt --check --edition 2024`(项目的 edition)对本期改动的全部 `.rs` | 全部干净 |
| `check_panel_boundary.py` | ok(213 refs in 5 files) |
| `scripts/check-log-scope.sh` | 只剩 `main` 上既有的 `preview_panel.rs:233` |
| `cargo tree -p dozer-panel-agent` | 无 `dozer-app/client/core`、`dozerd`、`wry`、`winit`、`portable-pty`、`russh` |

另手工核对:库 `update` 的 `AgentState` 分支与抽库前 `agent_state_changed` 的字段写入一致(`agent_state`、`agent`、`set_answer_dynamic_color(agent == Opencode)`、`transcript_path`),且"不存在的 tab 不起刷新任务"有测试钉住;卡片刷新用 `ctx.project.path` 作重锚根,与旧 `active_repo` 同源。

## 非阻塞观察

1. **库里没有终端滚动/选区消息。** Step 3b(a) 保留 `TermScroll` / `TermSelStart` / `TermSelUpdate` / `TermOutput` / `SessionExited` 在 Dozer 侧,理由(SSH 面板共用 `SessionTab`、事件泵同时服务 SSH 与恢复)成立,我接受。但这意味着库的 `terminal_pane` 通过 `PanelDisplay::term_event_fn` 把 `TermEvent` 交回产品,**滚动与选区的处理逻辑仍在 Dozer**。**S6d-2 的计划必须把这一块列为 Digger 要自己实现或要从 Dozer 再抽进库的工作**,否则 Digger 的 agent 页签没有滚轮和选区。
2. **`dozer-app/locales/host/*.json` 里 9 个已搬走的 key 现在无人引用**(`agent.no_conversations` 等),是死 key,与库里的一份重复。i18n 门禁不报。建议最终合并前统一清理,不在本期动。
3. **`PanelDisplay` 方法偏多**(页签四件、终端若干),这是计划 D9 的已知代价;偏差 1(`PanelDisplay<'a, M>` 带生命周期)、偏差 2(`TaskTitleSource` 不要求 `Send + Sync`)的理由成立,Digger 实现时会受这两点约束,S6d 计划引用时以代码为准。
4. **库自带独立 i18n resolver**(偏差 5)避免了与 `dozer-app` 同 owner 资源互相覆盖,合理;代价是将来这个 owner 在两处各有一份词表,清理 3 里的死 key 时一并核对。

## 未做/不属于本期

- Part B(Digger 钉版)按计划未执行;未合并、未打 tag、未推送。
- 目测验收清单在报告里,等整条 Dozer 侧链路完成后一次性验收。

## 链路状态

Dozer 侧叠分支:S6a Part A(`b7763bb8`)→ S6b(`726f86c7`)→ S6c-1(`dc513cbd`)→ **S6c-2(`8cd9bdc9`)** ✔。剩 S6d-2 Part A、S6e-1,再人工验收。
