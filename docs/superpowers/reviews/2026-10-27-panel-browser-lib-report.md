# 浏览器面板抽库验证报告(2026-10-27)

验证对象:`feat/panel-browser-lib` @ 4a66c574。方法:`git archive HEAD` 导出到 `/private/tmp/bb-lib-snap`(干净快照,独立 `CARGO_TARGET_DIR`),不含工作区未跟踪文件。

## 结果

| 项 | 结果 |
|----|------|
| `cargo build --workspace` | 通过(仅 warning) |
| `cargo test --workspace --no-fail-fast` | 3576 passed / 0 failed / 5 ignored |
| `cargo clippy --workspace --all-targets` | 退出 0,0 error。修复后另跑 `cargo clippy -p dozer-panel-browser -p dozer-app --all-targets`:退出 0,53 条 warning,其中 `dozer-panel-browser` 为 0 条;warning 所在文件里没有任何路径含 `browser`(即 `extensions/browser.rs`、`host_adapters/browser_panel.rs` 均 0 条)。只验证了这些,未与基线 warning 逐条比对 |
| `cargo fmt --check` | 通过 |
| `check-i18n.sh` / `check-log-scope.sh` / `check-shared-deps.sh` | 全部通过(shared-deps 有一条 guard-source 版本提示 INFO,非阻断) |
| `check_panel_boundary.py` | 通过(205 refs / 5 files,baseline ratchet) |
| `scripts/audit` python 单测 | 48 OK |
| `cargo tree -p dozer-panel-browser \| grep -E "dozer-(app\|client\|core)\|dozerd\|wry\|winit"` | 无输出 |

库依赖闭包(直接):bytehost-core、byteui、dozer-i18n、dozer-model、dozer-panel-host、dozer-services、iced_renderer、iced_widget、tokio、tracing。

## 测试数核对

- 基线:`extensions::browser` 41 条,workspace 3563。
- 终态:`dozer-panel-browser` 53 条 = 搬入 41 + 新增 12;dozer-app 1130 passed / 2 ignored;workspace 3576。
- workspace +13 = 库内新增 12 + dozer-app 新增 1(`region_projection_matches_browser_pane_theme`)。
- 41 条基线测试全部按名对上:`tabs.rs` 11 条,`tests.rs` 29 条,`addr_focus.rs` 1 条(`addr_field_id_is_stable_across_calls`)。
- 无偶发失败,全部一次通过。

### 库内新增 12 条
`addr_select_all_pending_is_one_shot`、`addr_submit_rejects_local_paths_and_blank_input`、`async_result_with_foreign_scope_is_dropped`、`chrome_height_has_one_source`、`closing_last_tab_leaves_one_blank_tab`、`closing_middle_tab_clears_hover_and_tooltip_state`、`desired_webviews_maps_one_to_one`、`global_instance_never_shows_project_bookmark_status`、`reorder_tab_moves_tooltip_timer_with_the_slot`、`the_bundled_catalogs_resolve_in_both_languages`、`view_builds_for_every_bookmarks_and_mirror_combination`、`view_builds_with_no_tabs`。

## Review Focus 对应测试

- F1(项目/全局隔离):`async_result_with_foreign_scope_is_dropped`、`global_instance_never_shows_project_bookmark_status`、`stale_bookmark_reads_and_reopened_panel_mutations_are_ignored`。
- F2(tab 增删与 hover/tooltip 状态):`closing_middle_tab_clears_hover_and_tooltip_state`、`closing_last_tab_leaves_one_blank_tab`、`reorder_tab_moves_tooltip_timer_with_the_slot`。
- F3(chrome 高度单一来源):`chrome_height_has_one_source`。注意它是同义反复:`chrome_top_px` 只转发给 byteui,`webview_geometry.rs` 直接调 byteui,所以该测试并不能证明两处一致。
- F4(地址栏全选一次性):`addr_select_all_pending_is_one_shot`。
- F5(地址栏提交校验):`addr_submit_rejects_local_paths_and_blank_input`。注意 plan 里的"URL 含空白"用例未覆盖。

## 发现并修复的真实 bug

commit 7ea738a8:`CloseTab` 后按序号索引的 hover/tooltip 状态残留(关闭中间 tab 后错位),且 `reorder_tab` 没有把 tooltip 计时器随槽位移动。对应测试:`closing_middle_tab_clears_hover_and_tooltip_state`、`reorder_tab_moves_tooltip_timer_with_the_slot`。

## 待用户裁决

地址栏接受 `file:///…`(不报 `error.local_path`)。这是抽库前就有的行为,已作为 golden 固定。是否要拒绝,由用户决定。

## 与 plan 的偏差

- `BrowserDisplay` 的实现放在 `dozer-app/src/host_adapters/browser_panel.rs`(R-APP 门禁要求,不放 `extensions/`)。
- trait 多了一个方法 `line_height_factor()`。
- 库内有私有的 `lh` / `split_portions` 副本。
- 库不依赖 `bytehost-desktop`。
- 边界门禁的测试加在 `test_check_panel_boundary.py`,不是 `test_unit_edges.py`。
- 边界 baseline ratchet 93 到 85 是此前就存在的松弛(记录值 213,实际 205),并非本次改动造成。

## 修复轮(终审意见)

- 哨兵键 `STAR_HOVER_KEY`/`NAV_*_KEY`/`NEW_TAB_KEY` 收窄为 `pub(crate)`,不再从 crate 根重导出;`view.rs` 改为从 `state` 取。
- `BROWSER_FORBIDDEN` 复用 `CONVERSATIONS_FORBIDDEN`(含 `dozerd`),测试补 `dozerd` 用例。
- `State` 文档注释移到 `pub struct State` 上。
- 重新生成 i18n inventory,移除 `extensions/browser.rs` 的过期规则;`check-i18n.sh` 与 `check_migration.py` 通过。
- 修复后复验(干净快照):`dozer-panel-browser` 53 passed;dozer-app 的 `extensions::browser host_adapters app::` 183 passed / 1 ignored;fmt 通过;四个门禁与 48 条 python 测试通过。

## 未覆盖

- 人工目测验收清单见 plan Task 6 Step 5,尚未进行。
