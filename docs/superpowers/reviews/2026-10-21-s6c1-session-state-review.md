# S6c-1 审核记录

日期:2026-10-21。被审:`feat/panel-agent-1`(worktree `../dozer-panel-agent`,顶端 `42550fa2`,基于 `feat/panel-term` 的 `726f86c7`)。
计划:`plans/2026-10-18-agent-s6c1-session-state.md`。实施报告:该分支内 `reviews/2026-10-21-s6c1-session-state-report.md`。

## 结论

**通过**(N1、N2 已于 `dc513cbd` 修复并复验,见文末)。行为守恒的证据成立,无阻塞性缺陷。

## 我自己实跑的验证(干净快照,`git archive HEAD`)

| 项 | 结果 |
| --- | --- |
| 分支基线 | `726f86c7` 是 HEAD 的祖先 ✔ |
| `cargo metadata --locked --offline` | 退出码 0 |
| `cargo test --workspace --no-fail-fast`(未补拷任何东西) | 退出码 101;**3405 通过 / 3 失败 / 5 ignored**;3 个失败全是 `extensions::git_log::tests::{build_marks_head_branch_and_labels, build_respects_max_count, commit_detail_uses_first_parent_diff}`,要求仓库自带 git 历史,archive 没有 `.git`,属环境原因,非代码问题。3405+3 = 3408,与报告总数一致 |
| 测试名守恒 | 基线 726f86c7 的每个测试函数名在 HEAD 中都还在(无丢失),新增 29 |
| 会话字节流黄金 | 夹具只在 `e92d5fb2` 出现一次,此后未改。**旧代码**(`e92d5fb2` 快照,旧 `Workspace::on_tab_attached` 路径)上 `workspace::tests::session_stream_golden` 通过;新库 `session_stream_through_the_service_matches_the_golden` 通过。载荷、20 个分块大小、渲染格式两边一致。把夹具第 5 行一个字符改掉后新库测试 FAILED,还原后通过 ✔ |
| 会话协议黄金 | 通过,但**本期完全没动 `dozer-core`**,协议类型原本就没变,这条是护栏,不是回归证据 |
| `check_migration.py` | 退出码 0 |
| `check_panel_boundary.py` | ok(213 refs in 5 files) |
| `scripts/check-log-scope.sh` | 只剩 `main` 上既有的 `workspace/preview_panel.rs:233` |
| `rustfmt --check` 对改动文件 | 与基线逐文件对比 diff 数相同(services 4/4、client 1/1、project fakes 1/1、service_fakes 1/1、view.rs 2/2),本期没有引入新的格式问题 |
| `cargo tree -p dozer-panel-agent` | 无 `dozer-app/client/core`、`dozerd`、`wry`、`winit`、`portable-pty`、`russh` |
| `cargo clippy --workspace --all-targets` | 退出码 0,但见 N1 |

另外手工对照了抽库前后的 `term_output`(OSC 摄取 → feed → 查询应答写回 PTY)和 `SessionExited` 处理:语义一致。i18n:四个错误 key 的默认英文文案逐字保留,owner 改为 `dozer.agent`,host 语言包里本来就没有这些 key,用户可见文本不变。

## 发现

### N1 新增 2 条 clippy 警告(小,需修)
报告写"既有 warning 与本期无关",不属实。本期新增:
- `crates/dozer-services/src/lib.rs:370`(`e92d5fb2`):`SessionService::attach` 返回类型 `very complex type`。建议抽 `type` 别名(如 `AttachResult`)。
- `crates/dozer-client/src/lib.rs:6`:`unused import: AgentState`。`TermEvent` 搬走后不再使用。

### N2 报告里"复验环境前置"一段不准确(小,需改报告)
报告称干净 archive 缺本地生成的 `assets/`,导致 `dozer-panel-preview` 静态资源测试失败。我的实测:`crates/dozer-app/assets`、`crates/dozer-panel-preview/assets` 都是已跟踪文件,archive 里就有,**不需要补拷**;唯一需要 `.git` 的是上面 3 个 `git_log` 测试。请改成真实情况,并写明"3 个 git_log 测试因 archive 无 `.git` 而失败,是环境原因,基线同样如此"。

## 非阻塞观察(记录,不要求本期处理)

1. **库的 `update` 与 Dozer 侧并存两套实现。** 生产里 `Message::TermOutput` / `SessionExited` / `SelectTab` 等仍走 Dozer 自己的处理函数(`term_output`、`app/update.rs`),库 `update` 里同一语义由库内测试覆盖,但 Dozer 只用到 attach/失败/输出回灌那部分。报告偏差 3 已披露,S6c-2 必须收口,否则会出现"改了一份忘了另一份"。
2. **`UpdateCtx.locale` 字段在库里无人读取,Dozer 侧写死 `"en"`。** 目前不影响行为(文案由 `LocalizedText` 在 Toast 层解析),但属于死字段,S6c-2 要么删掉要么接上。
3. **迟到的 `Attached` 不 kill daemon 会话**,与抽库前一致,报告已上报。会泄漏一个 daemon 会话,建议单独立项,不在迁移里夹带修复。
4. **`dozer-services` 新增 `tokio` 依赖**(原先只依赖 `dozer-model`),已在 crate 注释里记为例外,可接受;注意 Digger 侧消费时会带入。

## 需要的修复

1. 修 N1 两条警告;提交后 `cargo clippy` 警告数应比修前少 2。
2. 改报告 N2 一段,并把"既有 warning 与本期无关"改成实测描述。
3. 修复后在干净快照里复验 `cargo metadata --locked`、`cargo test -p dozer-services -p dozer-client -p dozer-panel-agent`。

## 修复与复验(2026-10-21)

- 修复提交 `dc513cbd`(`feat/panel-agent-1` 顶端):`dozer-services` 新增 `SessionAttach` 类型别名;`dozer-client` 去掉顶部未用的 `AgentState` 导入,两个测试模块(`session_protocol_golden`、`session_service_tests`)各自导入;报告的 clippy 与快照环境两段已更正。
- worktree 内 `cargo clippy --workspace --all-targets` 退出码 0,`dozer-services` / `dozer-client` 不再出现警告。
- 新的干净快照(`git archive HEAD`):`cargo metadata --locked --offline` 退出码 0;`cargo test -p dozer-services -p dozer-client -p dozer-panel-agent --no-fail-fast` 全部 `ok`,无 FAILED(`dozer-client` 56、`dozer-panel-agent` 33、`dozer-services` 24,其余目标全过)。

**结论:S6c-1 通过。** 未合并、未打 tag、未推送。下一环是 S6c-2(计划里仍写"从 main 切出",开工前按总设计 §0 第 9 条改为从 `feat/panel-agent-1` 顶端切出,并把观察 1、2 作为必收口项写进去)。
