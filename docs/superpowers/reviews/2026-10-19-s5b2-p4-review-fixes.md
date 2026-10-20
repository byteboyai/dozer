# S5b-2 P4 审查修复清单

- 审查对象:`plans/2026-10-19-preview-engine-p4-pairing.md` 的开发结果
  - Dozer:`../dozer-p4`,分支 `feat/preview-events-lib`,提交 `4e05f0a4`(Task 2、3)
  - Digger:`../digger-p4`,分支 `feat/preview-pairing`,提交 `b7e5e8d` `f601f62` `6cfc9ba` `a5a8f0f` `6df84a5`(Task 5–9)
- 审查日期:2026-10-19(只读审查,**未 build、未跑测试、未真机**;凡标「推断」的必须由修复者用测试或真机证实,不要照单全收)
- 结论:Dozer 侧 **With fixes**;Digger 侧 **No,先修 C1–C5**。Task 10(全量验证、真机回归、文档、合并、发布)尚未开始,符合预期。
- 给修复者的总规则:
  1. 每条修复**先写会失败的测试**,再改代码;测试名可自定,但必须能在修复前失败。
  2. 搬迁过的测试**不得删除或削弱**;审计类测试不得把断言改成更弱的形式。
  3. **合并、切 tag、推送、发布版本都是对外动作,等用户明确确认。** 见 §3 流程问题。
  4. 修完在本文件对应条目后写「已修 + 提交号」或「不修 + 理由」,不要静默跳过。

## 1. Dozer(`feat/preview-events-lib`)

### D-C1 事件项目身份被换成「当前聚焦项目」(对应 C2 的 Dozer 部分)
- 位置:`crates/dozer-app/src/app/update.rs:4279`(`host_event`);`preview_ui` 内部用 `with_focused_project`。
- 问题:旧代码按 `binding.project_id` 路由(`with_project(binding.project_id, ..)`)。现在 A 项目的迟到页面事件在切到 B 后会作用到 B 的同号页签。注释写「取自绑定」与实现相反。
- 修:`HostEvent` 路径改回按绑定里的 `project_id` 路由;绑定项目已不存在则丢弃。
- 测试:`host_event_for_a_project_that_is_not_focused_applies_to_its_own_state`、`host_event_for_a_removed_project_is_dropped`。

### D-C2 「保存后关闭」在只读 / 冲突 / revision 不符时页签卡死(C6)
- 位置:`crates/dozer-panel-preview/src/host_update.rs:414`(`let _ = want_close;`)。
- 问题:这三条路径不产出 `Effect::SaveFile`,`pending_close` 永远不被消费;旧代码不论 gate 结果,`want_close` 就关。
- 修:`want_close` 时,三条路径直接完成关闭(与 main 上旧行为一致)。
- 测试:`save_and_close_closes_even_when_the_tab_is_read_only`、`…_when_there_is_a_conflict`、`…_when_the_revision_is_stale`。

### D-I1 `SaveFinished(Ok)` 无条件清脏
- 问题:`Effect::SaveFile` 不带 revision;异步写盘期间用户继续输入,回执会清掉新的脏标和恢复快照。`save_finished` 还漏了旧收尾:`tab_first = 0` 与状态持久化,并推了 `TabClosed{index:0, old_len:0}` 这种哑值。
- 修:`SaveFile` 带 `revision`,回执时 `tab.web_revision == revision` 才清脏;补回 `tab_first = 0` 和持久化效果;去掉哑 `TabClosed`。
- 测试:`typing_during_an_async_save_keeps_the_tab_dirty`、`save_finished_resets_the_tab_strip_start_and_persists`。

### D-I2 `AllowedFiles` 不是计划要求的「规范化」,且判定路径 ≠ 读取路径
- 位置:`crates/dozer-panel-preview/src/assets.rs`(`AllowedFiles`、约 256 行 `serve_allowlisted_file`)。
- 问题:
  - 文档注释自称「不 canonicalize」,只做词法清理;`/tmp` 与 `/private/tmp` 不等价;不拒绝目录。
  - `contains` 判断的是**清理后**的路径,`fs::read` 读的是**原始解码路径**。请求 `link/../ok.txt` 时判定看到 `/proj/ok.txt`,读取却会按 `link` 的真实父目录解析 → 可能越权(推断,需用符号链接测试证实)。旧代码是精确匹配,`..` 永远 404。
  - `insert` 是 `&mut self`,计划要求 `&self` + `Clone`(inventory Q4 已认可 `Arc<Mutex<_>>`,请保持一致并在注释里写明)。
- 修:读与判**同一个**路径;`serve_allowlisted_file` 像 `serve_html_file` 一样拒绝含 `..` 的分量;入表与查表都 `canonicalize`(失败则拒绝,不降级放行);拒绝目录。
- 测试(计划点名,必须按这些名字出现):
  - `allowlist_rejects_traversal_encoded_slash_nul_and_symlink_escape`(含 `link/../ok.txt`)
  - `allowlist_compares_canonical_paths_and_rejects_directories`(含 `/tmp` → `/private/tmp`)
  - `file_url_roundtrips_names_with_space_cjk_hash_question_percent_ampersand`

### D-I3 计划点名的安全测试与约束缺失
- 缺:`host_event_for_a_closed_tab_is_ignored`、`host_event_from_a_stale_generation_is_ignored`、`save_is_rejected_when_the_event_path_differs_from_the_bound_path`、`save_is_rejected_outside_the_project_root`、`oversized_text_change_is_rejected_without_changing_the_tab`(现有 `editor_event_for_a_foreign_path_is_dropped` 只近似覆盖第三条)。
- 约束未实现:**保存路径必须在项目根内**。`ctx.project_root` 在 `host_update.rs` 完全没用到,而 `Effect::SaveFile` 的文档注释声称「校验在 update 里做」。
- 修:实现项目根校验(规范化后比较前缀);补上五条测试;超大文本不要只依赖 `classify_ipc` 的 8 MiB 上限,在 update 里也限制。

### D-I4 审计测试被削弱
- 位置:`crates/dozer-app/src/workspace/tests.rs` 的 `ui_handlers_do_not_call_blocking_preview_entry_points`。
- 问题:原 `assert!(audited > 0, ..)` 被改成「为 0 时只检查源码含 `PreviewPane::` / `run_effect` 字样」;新加进禁止名单的 `save_text_atomic(` 在 `dozer-app` 内已无调用点(审计空转);审计范围没扩到 `dozer-panel-preview/src/host_update.rs`。
- 修:恢复 `audited > 0` 的强断言(或改成对新位置同等强度的断言);把审计范围扩到库里的 `host_update.rs`;在禁止名单里保留对库内阻塞调用的检查。

### D-M1 提交说明数字对不上
- 提交说明写「+27 in dozer-panel-preview, +9 in dozer-app」,按 `#[test]` 计数实际约 +13(assets +4、update_tests +9),dozer-app 新增 0。请用 `git diff main --stat` + `grep -c '#\[test\]'` 重新核对并更正文档。

### D-M2 死变体
- `Effect::PushHostScript` 在 `run_effect` 是空操作,且没有任何地方产出它。要么接上要么删除,并在注释里写明现状。

## 2. Digger(`feat/preview-pairing`)

### G-C1 每次切项目都删旧项目的持久化文件并清空全局白名单(C1)
- 位置:`crates/desktop/src/product.rs:462`(`on_current_project_changed` 里对旧项目调 `preview.forget_project(old)`);`adapters/preview.rs:162`(`forget_project` 会 `persistence.remove(id)` 且 `host.clear_allowed()`)。
- 问题:`forget_project` 是「删除项目」语义,全程只有这一个调用点,没有区分切换和删除。切走再切回,`preview_state/<id>.json` 已被删,页签恢复不了(D4 落空);白名单是跨项目共享的 `Host.allowed`,切换或删别的项目会清空当前项目已打开文件的授权 → 读取 404。
- 修:切换只做 `set_current_project`;`forget_project` 只在**项目真被删除**时调用。白名单按项目隔离(`HashMap<i64, AllowedFiles>`),或切换时重建当前项目的授权。
- 测试:`switching_projects_keeps_the_persisted_preview_state`、`switching_back_restores_the_tabs`、`deleting_project_a_does_not_revoke_project_bs_allowlist`、`forget_project_is_only_called_on_delete`。

### G-C2 迟到消息不防串台(C2 的 Digger 部分)
- 位置:`product.rs:1299`(`Msg::Preview(panel, message)` 用此刻的 `current_project_id()`);`adapters/preview.rs` 的 `run_effect` 回投闭包不带 `project_id`;`Msg::PreviewEvent` 转发 Host 事件时没拿 `binding.project_id` 比对。
- 问题:`dispatch` 里的 `project_id` 过滤永远相等,防护是空操作。A 项目迟到的 `SaveFinished`、`WindowIndex`、`Profiled`、webview IPC 在切到 B 后落到 B 的同号页签(`tab_id` 每个状态都从头数)。
- 修:`Msg::Preview` 携带 `project_id`(`Msg::Preview(ViewerPanel, i64, Message)`),效果回投时带上发起时的项目;Host 事件要求 `binding.project_id` 等于目标状态的项目,否则丢弃。
- 测试:`a_late_save_result_for_project_a_never_reaches_project_b`、`a_late_ipc_event_with_a_foreign_binding_is_dropped`、`tab_ids_are_not_shared_across_projects`。

### G-C3 页签条可用宽度恒为 0(C3,推断)
- 位置:`product.rs:35` `PREVIEW_DISPLAY` 是 `const`,`tab_bar_px: 0.0`;`preview.set_tab_bar_width(..)` 只喂给 `UpdateCtx`;库 `view.rs:756` 调 `tab_window(widths, 4.0, 0.0, first)`,byteui 在 `avail = 0` 且页签数 ≥ 2 时保底只放 1 个。
- 影响:至少有「空白占位页签 + 文件页签」两个页签,页签条看起来只剩 1 个;Review Focus 5(50 个页签)直接失败。
- 修:`PreviewDisplay` 每帧用 `preview.tab_bar_px(panel)` 构造,不要用 `const`;宽度来源与库 `UpdateCtx.tab_bar_avail_px` 用同一个值。
- 测试:`the_tab_bar_gets_the_real_available_width_each_frame`、`fifty_tabs_show_more_than_one_tab_and_paginate`;**真机必验**。

### G-C4 Project 与 Files 写同一个持久化文件互相覆盖(C4)
- 位置:`adapters/preview.rs:455` `behavior_of` 对两个面板都 `persist_tabs: true`;`PersistTabs` 只带 `project_id`,都写 `<id>.json`。
- 问题:Dozer 的 Project 面板是 `persist_tabs = false`;Digger 注释对此的解释站不住。Project 的页签会覆盖 Files 的,重启后 Files 恢复出 Project 的页签。
- 修:`persist_tabs = (panel == ViewerPanel::Files)`,与 Dozer 一致;或者两个面板各写各的文件(要先和用户确认是否有意偏离)。
- 测试:`only_the_files_panel_persists_its_tabs`、`project_tabs_never_overwrite_the_files_state_file`。

### G-C5 无当前项目 / 关闭最后一个项目时预览 webview 不销毁(C5)
- 位置:`product.rs:655-664` `sync_preview_native` 在无当前项目或 `closing` 时直接 `return Vec::new()`,没有对 `Host::sync` 传空清单;函数注释说「清单为空,宿主销毁全部页面」,代码没这么做。
- 影响(推断):残留 webview 悬在窗口上。
- 修:无项目时也调 `host.sync(window, &[])`;`closing` 时调用 `destroy_all`。
- 测试:`closing_the_last_project_tab_destroys_every_preview_webview`、`quitting_destroys_every_preview_webview`(用可注入的 live 计数或 `plan()` 输出断言)。

### G-I1 `host_scripts()` 对 Project 面板重复加偏移
- 位置:`adapters/preview.rs:205-220` 对库 `take_host_scripts` 的结果又加 `id_offset`;库返回的 `binding.webview_id()` 已含 `PROJECT_PREVIEW_ID_OFFSET`;`Host::take_host_scripts` 同理。
- 影响(推断):Project 面板下延后取出的编辑器命令(`SetDocument`、视图恢复、跳行)发给不存在的 webview。
- 修:只加一次偏移(优先:库已含则 Digger 不再加)。测试:`project_panel_host_scripts_target_the_right_webview_id`。

### G-I2 webview 矩形可能比 Dozer 多 8px 上下空隙(推断)
- 位置:`product.rs:165`:`slot.y + chrome(38) + inset(8)`,高度再减 `inset`。Dozer `webview_geometry.rs:169` 非放大态 `y = zone 顶 + 38`、`h` 到 zone 底;放大态 `y0 + 38`;库 `view::pane` 里 38 已含 8px 内边距。
- 修:对照 Dozer 的两种态改成同一公式;`preview_rect_is_the_second_block_below_the_chrome` 现在把这个多余的 `inset` 钉死了,先改测试期望(附 Dozer 公式出处),再改代码。**真机截图对比 Dozer 同一文件的 webview 顶 / 底边。**

### G-I3 切项目丢弃整份内存状态(与计划「每项目一份状态」不符)
- 问题:Project 面板页签不持久化,切走即丢;脏缓冲只靠 debounce 的恢复快照兜底。Dozer 是按项目常驻 workspace。
- 处理:若这是有意的 D4 取舍,请在计划 D4 与 spec 里**明确写下**:切项目会丢弃 Project 面板的页签(仅 Files 页签持久化),并让用户确认;否则改成按项目常驻状态。

### G-I4 `.png` 的行为三处自相矛盾
- 事实:`digger_hosts_url`(`preview.rs`)排除 `image-annotate`,测试 `tabular_json_and_media_files_get_the_degraded_page_not_a_webview` 断言 `p.png` 没有 webview;但 `package-macos.sh` 强制要求 `image-annotate/`,Task 10 真机清单要求 `.png` 渲染,Task 9 selftest 期望 `.png` 是 webview(selftest 是独立 pane,不走 `PreviewHub`,测不到这个过滤)。
- 修:三者统一。**建议开放 image-annotate**(Digger 能力集已含 `rendered`;打包本来就带);同步改测试、selftest 和计划清单。需用户确认选哪个。
- 测试:`png_and_svg_open_in_a_webview_in_the_preview_hub`(走 `PreviewHub::desired`)。

### G-I5 原生 webview 盖住 iced 浮层(推断)
- 影响:页签溢出下拉(`tab_chrome::overflow_menu`)向下展开会被 webview 遮住;Project 删除确认卡与右半边重叠同理。Dozer 在 macOS 用原生菜单规避。
- 处理:真机验证;若成立,方案二选一并告知用户:弹浮层时临时藏起预览 webview(显示一个占位),或溢出菜单改成向上 / 局限在页签条区域。

### G-M 其他小项
- `forget_project` 的注释只在 `project_id == self.project_id` 时清状态,与调用方语义不一致(随 G-C1 一并处理)。`attach_handle` 之前 `execute` 会静默丢弃库侧效果(含 `SaveFile`),`boot()` 顺序目前正确但无断言 → 加一条断言测试。
- `selftest_fails_when_a_fixture_is_missing` 实际测的是 `missing_in`(资源目录),名不副实;`selftest_preconditions` 本身没测试;计划的 `bundled_layout_has_the_three_first_batch_dirs` 被改成了打包脚本里的 shell 检查 → 补一个真正的单元测试或在计划里记录替代。
- `open_file_command_…` 改写后不再断言系统打开器「路径作为单个参数」的语义,现在只有 `system_opener` 自己守着 → 补回测试。
- `shell.rs` 新测试里 `expect("{panel} has a pane")` 没有格式化(`expect` 不做插值),改用 `unwrap_or_else(|| panic!(..))`。
- `thread_local` 的 `CURRENT_WINDOW` / `CURRENT_PANES` 是给 `PanelDisplay` 取外部状态的绕法,可接受;在模块头注释里写明为何,并加一条「帧间不残留」的测试。

## 3. 流程问题(需要用户裁决,开发者不得自行处理)

- `libs-2026-10-19g` 已推送到 `origin`(tag 对象 `b627b861`,指向 `4e05f0a4`,创建于 20:35:26;提交于 20:33:58,仅隔约 90 秒),而 `feat/preview-events-lib` **未合并到 `main`、也未推送分支**,`origin` 上只有 `main` 和 `feat/preview-host-lib`。
- 计划 Task 4 要求:Dozer 目测验收通过 → 用户确认 → 合并 → 切 tag → 推送。**没有任何验收证据。** inventory 的状态栏写的还是「等用户过目」。
- Digger 分支已把 9 个 dozer 依赖全部钉到 `libs-2026-10-19g`(`Cargo.lock` 指向 `4e05f0a4`),该 tag 不在 Dozer `main` 的历史里。
- 请用户决定:保留该 tag 并在修复后另切 `…h`,还是删除远端 tag。**修复期间 Digger 可继续用本地 `path`/`[patch]` 联调,不要再依赖未验收的 tag。** 修完以后:Dozer 目测验收 → 用户确认 → 合并 → 新 tag → Digger 改钉。

## 4. 目测 / 真机清单要补的项

在 Dozer 的目测清单(计划 Task 4 Step 2)和 Digger 的真机清单(Task 10 Step 3)里各加:
1. **迟到事件串项目:** 在项目 A 里打开较大的文件(或触发保存),立即切到项目 B,B 的同号页签不受影响。
2. **保存后关闭:** 对只读文件、被外部修改的文件(冲突)、revision 过期的页签各做一次「保存后关闭」,页签能关掉。
3. **异步保存期间继续输入:** 保存回执后脏标仍在。
4. **项目 A → B → A:** 回到 A 后页签恢复;B 打开的文件仍可预览。
5. **关闭最后一个项目页签:** 窗口上不残留预览 webview。
6. **50 个页签、CJK 与超长标题、同名文件:** 页签条翻页与溢出菜单正确,`live_count()` 随关闭回落。
7. **溢出菜单与删除确认卡** 展开时不被 webview 遮住。
8. **webview 顶 / 底边与 Dozer 同文件对比** 无多余空隙。

## 5. 修复完成的验收标准

- Dozer:`cargo test --workspace` 总数 ≥ 修复前且无任何被删 / 削弱的测试;三份黄金字节不变;`dozer-panel-preview` 依赖闭包无 `wry` / `winit` / `dozer-app` / `dozer-client` / `dozer-core`;`clippy` 警告不高于基线;`check-i18n`、`check-shared-deps`、边界脚本通过。
- Digger:`cargo test --workspace`、`clippy`、`fmt --check`、`check-shared-deps.sh`、`check-host-deps.sh --self-test`、`check-shell-regions.sh` 通过;`run-editor-selftest.sh` 通过;`preview_smoke --selftest` 在包里通过。
- 本文件每一条标注「已修 + 提交号」或「不修 + 理由」;再交我复审(只读)。
- 未经用户确认:不合并、不切 tag、不推送、不发布、不提交版本号。
