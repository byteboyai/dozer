# S6e-3a Part A 实施报告(共享库抽取)

日期:2026-10-24。分支 `feat/preview-bridge-lib`(worktree `../dozer-preview-bridge`),基线 Dozer `main` `c6a8bbb1`。只做 Part A(Task 1–2 + Task 3 的门禁/报告),**没有合并 `main`、没有打 tag、没有推送**,停在等待用户确认。

## 提交

- `e2b4ac92` feat(preview): 抽出 agent_bridge(预览上下文、模式 token、命令目标解析)
- `b62d0270` refactor(app): 预览上下文与命令目标解析改用 dozer-panel-preview::agent_bridge

## Task 1:`dozer-panel-preview::agent_bridge`

新增 `crates/dozer-panel-preview/src/agent_bridge.rs` 并在 `lib.rs` `pub mod` + `pub use`:

- `preview_context_from_web_state(path, selection, selected_text, viewport, revision, now_ms)`:从 `workspace/hook.rs` 原样搬入,函数体未改。
- `mode_token(PreviewMode)`:原样搬入,函数体未改。
- `build_preview_context(pane, now_ms)`:活动页签 → `PreviewContext`;CodeMirror host 走镜像字段;表格页签填 `PreviewTabularContext`;Blank/图片/webview 返回 `None`;末尾写 `mode`/`read_only`。逻辑对应 `state.rs::enqueue_preview_context_push` 原有块。
- `resolve_command_target(files, project, cmd)`:对应 `Workspace::resolve_preview_command_target`;`Tab` 按显式面板名;`Path` 先 Files 后 Project。
- 库只依赖 `dozer-model` / `dozer-preview-core`;没有 `dozer_client` / `dozer_core` / tokio 任务循环,不读时钟(调用方传 `now_ms`)、不碰 IO。

测试:搬入 3 条(`preview_context_from_web_state_maps_selection_and_viewport`、`preview_context_from_web_state_cursor_only_and_missing_viewport`、`mode_token_covers_all_modes`),新增 6 条:

- `build_preview_context_is_none_for_blank_and_for_non_text_tabs`
- `build_preview_context_for_a_code_tab_carries_selection_mode_and_read_only`
- `build_preview_context_for_a_tabular_tab_fills_the_tabular_block`
- `resolve_target_prefers_files_over_project_for_the_same_path`
- `resolve_target_for_an_explicit_tab_uses_the_named_panel`
- `resolve_target_returns_none_when_the_path_is_open_nowhere`

`cargo test -p dozer-panel-preview` → 411 通过 / 0 失败;`agent_bridge` 9 条全过。文档、实现见提交 `e2b4ac92`。

## Task 2:Dozer 改引用

- `workspace/hook.rs`:删掉 `preview_context_from_web_state`、`mode_token`。
- `workspace/state.rs`:`enqueue_preview_context_push` 的中间构造块换成 `dozer_panel_preview::build_preview_context(&self.preview, now_ms)`;`resolve_preview_command_target` 调 `dozer_panel_preview::resolve_command_target(...)`,再把 `ViewerPanel` 映射回 `PanelKind`。
- `workspace/tests.rs`:删掉已搬走的 3 条同名测试;原有 `resolve_preview_command_target_by_path_and_tab` 等保持不动并通过。

测试名守恒核对:

- 旧位置(`dozer-app -- --list`):3 个名字均不再出现。
- 新位置(`dozer-panel-preview -- --list`):3 个名字均存在,另有 6 条新增。

## 测试数

- **基线 N = 3473**(Dozer `main` `c6a8bbb1`;在 detached HEAD 上跑时 `extensions::git_log::tests::build_marks_head_branch_and_labels` 因"没有当前分支"环境失败 1 条,按分支上应通过计为 3473)。
- Task 1 之后:3482(基线 + 9,其中 3 条是搬移造成的临时重复)。
- Task 2 之后:**3479 = N + 6**;`cargo test --workspace --no-fail-fast` 退出 0,0 失败。

## 验证(最终代码)

| 项 | 命令 | 退出码 | 结果 |
| --- | --- | --- | --- |
| 全量测试 | `cargo test --workspace --no-fail-fast` | 0 | **3479 通过 / 0 失败** |
| 单包 | `cargo test -p dozer-panel-preview` | 0 | 411 通过 / 0 失败 |
| clippy | `cargo clippy --workspace --all-targets` | 0 | 本期新增/改动文件 0 警告;仅剩既有警告(如 `preview_tab_first_mut` 在非测试构建里未用) |
| 改动文件格式 | `rustfmt --edition 2024 --check` 对 5 个改动文件 | 0 | 干净 |
| 面板边界 | `python3 scripts/audit/check_panel_boundary.py` | 0 | `ok (213 refs in 5 files, baseline ratchet)` |
| 共享依赖 | `bash scripts/check-shared-deps.sh` | 0 | 0 阻断 / 0 警告 |
| 日志范围 | `bash scripts/check-log-scope.sh` | 1 | **基线既有**:`crates/dozer-app/src/workspace/preview_panel.rs:233` 的 `tracing::warn!` 未带 `dozer_core::log_*`,不是本期改动 |
| 全仓格式 | `cargo fmt --check` | 1 | **基线既有**:`app/view.rs`、`extensions/files/state.rs` 等既有文件;本期 5 个改动文件单独 rustfmt 干净 |

干净快照见下。

### 干净快照实测

`git archive HEAD` 解到 `/private/tmp/dozer-preview-clean.Kp1q4w`(路径不含 "digger"),数据根 `/private/tmp/dozer-preview-data.lYzhm6`,临时 `HOME` `/private/tmp/dozer-preview-home.HYwH3E`,自己的 `target`:

| 命令 | 退出码 | 结果 |
| --- | --- | --- |
| `cargo metadata --locked --offline` | 0 | 锁文件可复现 |
| `cargo test --workspace --no-fail-fast` | 101 | **3476 通过 / 3 失败**;3 条全是 `extensions::git_log::tests::*` 因归档目录没有 `.git` 报 `could not find repository`,与基线在无 `.git` 快照里的既有环境失败一致(3476 + 3 = 3479,与工作区分支上的 3479 / 0 对得上) |
| `cargo clippy --workspace --all-targets` | 0 | 无新增警告(本期 5 个改动文件 0 警告;全仓仍有基线警告) |

快照目录、数据根、临时 HOME 已删除。

## 待用户确认(Task 3 Step 3,对外动作)

1. 新 `libs-*` tag 的名字(建议 `libs-2026-10-24a`,实际按现有命名规则定)。
2. 是否合并 `main`、是否推送 tag(都还没做)。
3. Task 4 依赖核对用版本:本仓库当前钉 `bytehost` = `v0.4.11`、`byteui` = `v0.4.5`、`bytegit` = `v0.8.1`。

## 与计划不符 / 备注

- Task 2 Step 1 的基线本应在 Task 1 前跑;本轮先做了 Task 1,因此报告用 `c6a8bbb1` 的实测基线 3473,并用 3479 = 3473 + 6 对账。
- `resolve_command_target` 的 `ViewerPanel::GitLog` 在 Dozer 原实现里不存在;映射处按 `Files` 兜底,不改变现有 `PanelKind::{Files,Project}` 语义。
- 未执行 Part B。
