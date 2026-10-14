# Dozer i18n 验收记录

> 日期：2026-10-12。逐任务记录命令/证据/结果；最终按规格 §12 L01–L14 逐条验收（Task 13）。
> Task 01 只交付基线，尚未实施功能迁移；本文件当前只记录 Task 01 验收与整体待办。

## Task 01 验收

计划 Task 01 验收条件：**无未分配的 UI 区域；候选有分类/负责人任务；未迁移清单可重跑。尚未审计项保持 unresolved。**

| 检查项 | 命令/证据 | 结果 |
|---|---|---|
| 无未分配的 UI 区域 | `python3 scripts/i18n/build_inventory.py` 未列出无任务候选 | ✅（unassigned unresolved = 0） |
| 候选有分类与 owner | `inventory.json` 每条含 `usage`/`owner`/`task`/`status`/`reason` | ✅ |
| 候选有任务 | `inventory.json` `unresolved_by_task` 覆盖全部未解决项 | ✅ |
| 未迁移清单可重跑 | 重跑扫描 + 生成器，数字与 `ledger.md` §1 一致 | ✅ |
| 未审计项保持 unresolved | `inventory.json` 中无候选被标 `migrated` | ✅ |
| 总账恒等式 | `0 + 3001 + 1207 = 4208` | ✅ |

### 重跑命令

```bash
python3 scripts/i18n/scan_sources.py --json /tmp/i18n_candidates.json
python3 scripts/i18n/build_inventory.py
```

### 交付物

- `docs/dozer-v2/i18n/baseline.md`
- `docs/dozer-v2/i18n/inventory.json`
- `docs/dozer-v2/i18n/glossary.md`
- `docs/dozer-v2/i18n/ledger.md`
- `docs/dozer-v2/i18n/acceptance.md`（本文件）
- `scripts/i18n/scan_sources.py`、`scripts/i18n/build_inventory.py`
- 规格 §5 资源路径修订（Usage 独立 crate 归属）

### 残留问题

- 迁移尚未开始（`migrated = 0`），官方语言包不存在；所有未解决 UI 候选待各自任务处理。
- 扫描器以静态字面量为主；无静态字面量但可显示文本的入口（如 Rust 拼接 payload、服务 message）由各迁移任务沿调用链人工补登。
- 既有无关测试失败保留：`usage_probe::host::tests::host_site_citations_point_at_the_code_they_name`。

## Task 02 验收

计划 Task 02 验收条件：**Rust/TS 共享向量同结果；注入缺 key/参数不一致会检查失败；中文展示确实来自 zh-CN 资源。对应 L05/L06。**

| 检查项 | 命令 | 结果 |
|---|---|---|
| Rust/TS 共享向量同结果 | `cargo test -p dozer-i18n --test vectors` + `node crates/dozer-app/web/shared/i18n/i18n.test.ts` | ✅ 两读 `vectors/interpolation.json`，同结果 |
| 缺 key 检查失败 | `python3 scripts/i18n/check_resources.py <临时目录>` | ✅ 注入缺 key/多余 key → FAIL |
| 参数不一致检查失败 | 同上（占位符集不同） | ✅ FAIL |
| 重复 key 覆盖前发现 | 同上（`object_pairs_hook`） | ✅ FAIL |
| 中文来自 zh-CN 资源 | `cargo test -p dozer-app i18n::` | ✅ `settings.title`→「设置」来自 `zh-CN.json`，非 fallback |
| 解析器一致性单测 | `cargo test -p dozer-i18n` | ✅ 18 单测 + 1 向量测试通过 |

### 交付物

- `crates/dozer-model/src/i18n.rs`：`Locale`/`LocalePreference`/`LocaleSnapshot`/`LocalizedText` + 具名参数。
- `crates/dozer-i18n/`：`bundle.rs`(装载/校验/parity)、`interp.rs`(插值)、`locale.rs`(规范化)、`lib.rs`(Resolver/fallback)、`vectors/interpolation.json`、`tests/`。
- `crates/dozer-app/locales/host/{en,zh-CN}.json`：最小 Host 词表。
- `crates/dozer-app/src/i18n.rs`：编译期嵌入 + `host_resolver()` 接缝。
- `crates/dozer-app/web/shared/i18n/i18n.ts`、`i18n.test.ts`：中立共享 TS 解析器。
- `scripts/i18n/check_resources.py`：资源完整性检查。

### 残留问题

- 面板标题尚未接入 `PanelDescriptor`/`product.rs`/设置页（Task 04/05）。
- 复数变体（`.zero`/`.one`/`.other`）由调用点选择，解析器不内建 ICU；具体规则见 spec §6.2 与 `glossary.md` §5。
- TS 解析器目前无包内 `npm test` 接线（各 Web 包接线在 Task 08/11）。

## Task 03 验收

计划 Task 03 验收条件：**离线/IO 失败/并发写/重启恢复通过；用户项目数据与终端配置不变。对应 L02/L03/L04/L12。**

| 检查项 | 命令/证据 | 结果 |
|---|---|---|
| LocaleService 接入 Services 全部构造点 | `crates/dozer-services` `Services.locale`；`dozer-app` `service_fakes::FakeLocale`/`host_adapters`；`dozerd` `Stores.locale` + 全部测试构造点 | ✅；`check_panel_boundary.py` 仍 ok（面板不新增 client 引用） |
| 追加 get/set 请求且旧形状不变 | `cargo test -p dozer-core locale`：`locale_request_and_replies_roundtrip`、`set_locale_without_system_candidates_decodes_empty` | ✅ 别名追加，旧 Request/Reply 未改 |
| 串行 CAS | `cargo test -p dozerd --lib locale`：`concurrent_cas_only_one_wins` | ✅ 8 并发提交仅 1 Applied / 7 Conflict |
| 原子持久化 + 重启恢复 | `cargo test -p dozerd --test locale_requests setting_survives_daemon_restart` | ✅ 重启后 snapshot/revision 保留 |
| 配置缺失/损坏恢复 | `... missing_file_uses_default`、`... corrupt_file_recovers_to_default` | ✅ 缺失/损坏均告警回落默认，不 panic |
| 无变化不推进 revision | `... no_op_set_does_not_advance_revision` | ✅ 相同偏好重提 → revision 不变、不落盘 |
| revision 冲突不覆盖 | `... stale_revision_is_conflict` | ✅ 过期 expected_revision → Conflict + 最新快照 |
| 计数溢出明确失败不回绕 | `... revision_overflow_fails_instead_of_wrapping` | ✅ `Err` 含「溢出」，内存保持 `u64::MAX` |
| System 解析用候选 | `... system_preference_resolved_from_candidates`、`... system_falls_back_to_en` | ✅ `resolve_from_candidates` 决定 active |
| 旧 daemon 拒绝/断连 → 客户端分类 | `cargo test -p dozer-client locale`：`old_daemon_error_reply_is_rejected`(Rejected)、`old_daemon_hang_up_is_unavailable`(Unavailable) | ✅ 不无限重试、不 panic |
| 意外应答 → Protocol 降级 | `... unexpected_reply_is_protocol` | ✅ 不吞成空结果 |
| 旧业务请求仍可用 | `cargo test -p dozer-client`、`-p dozerd` 全绿 | ✅ 旧 Request/Reply 路径未受影响 |
| 用户项目数据/终端配置不变 | 语言设置独立文件 `config_dir()/locale_settings.json`，不触碰项目/终端配置 | ✅ 无交叉写入 |

### 交付物

- `crates/dozer-model/src/i18n.rs`：`LocaleWriteOutcome`（`Applied`/`Conflict`，带 snapshot）+ `LocaleSnapshot::SCHEMA_VERSION`/`default_snapshot()`。
- `crates/dozer-services/src/lib.rs`：`LocaleService` trait（`get_locale`/`set_locale`）+ `Services.locale` + `MemLocale` 假服务。
- `crates/dozer-core/src/protocol.rs`：`Request::GetLocale`/`SetLocale`、`Reply::Locale`/`LocaleWrite` 及类型重导出。
- `crates/dozer-client/src/lib.rs`：`Client::get_locale`/`set_locale` + `impl LocaleService` + 降级测试。
- `crates/dozerd/src/locale_settings.rs`：`LocaleSettingsStore`（CAS/原子 temp+rename+fsync/损坏恢复/溢出失败/无变化不推进）+ 10 单测；`dozerd/src/{lib,main,server}.rs` 接线。
- `crates/dozerd/tests/locale_requests.rs`：4 条 UDS 契约测试。

### 残留问题

- 系统语言候选（`system_candidates`）的实际来源（读取 OS 有序语言偏好）与 App 快照/窗口同步在 Task 04；本任务只定义字段与解析。
- 「启动 System 刷新带 expected_revision 提交」的完整循环（读取后提交、陈旧探测回包不覆盖显式偏好）由 Task 04 的平台探测 + 哨兵驱动实现；本任务保证 CAS 语义到位。
- 既有无关测试失败保留：`usage_probe::host::tests::host_site_citations_point_at_the_code_they_name`。

## Task 04 验收

计划 Task 04 验收条件：**正常网络/服务延迟下按约定周期发起同步，断线与乱序测试通过，两个窗口一致。对应 L01/L04/L14。**

### 证据

| 验收要点 | 证据 |
|---|---|
| 平台隔离的有序系统语言探测（不依赖 `LANG`；macOS `NSLocale.preferredLanguages`） | `crates/dozer-app/src/platform/locale.rs`：macOS 经 objc2 autoreleasepool 读 `NSLocale::preferredLanguages()`（有序），非 mac 回落到 `LC_ALL`/`LC_MESSAGES`/`LANG`；`system_supported_locale()` 经 `dozer_i18n::resolve_from_candidates` 归一。2 单测通过 |
| `zh-Hans-TW`/`zh-Hant-CN` 等冲突输入 | 归一规则由 `dozer-i18n::locale` 单点实现并在 Task 02 以共享向量覆盖（Hant/TW/HK/MO → None，不映射简体）；探测层复用同一函数 |
| App 快照 + 连接 epoch + 请求序号 + display generation；迟到回包检查 epoch/序号 | `crates/dozer-app/src/app/locale.rs` `LocaleController`：`apply_fetch` 丢弃错 seq（`late_reply_with_wrong_seq_is_dropped`）、同 epoch 拒绝 revision 回退（`same_epoch_never_regresses_revision`）、仅 active 变化时推进 generation（`generation_only_advances_when_active_changes`） |
| 缓存仅写最后确认快照；断线用缓存/临时系统值；重连以 daemon 为准并重建 revision 基线 | `crates/dozer-app/src/locale_cache.rs`（`load`/`save`，损坏/版本不符→None）+ `apply_fetch` 仅在 `source == Daemon` 时回写缓存；`on_reconnect` 推进 epoch 清在途并强制立即刷新（`reconnect_resets_baseline_and_forces_immediate_fetch`）；daemon 离线启动走 `with_daemon_error` 只采纳缓存/系统临时值，不在离线时发起读取 |
| 每 2 秒跨进程读取（在途合并），获焦立即刷新 | `REFRESH_INTERVAL = 2s`；`begin_fetch` 合并在途（`cadence_limits_to_refresh_interval`）；`platform/window_events.rs` `about_to_wait` 的 wakes 数组加第 11 项恒 active 的 `next_locale_wake()`；`ResumeTimeReached` 调 `poll_locale_if_due()`；新增 `WindowEvent::Focused(true)` 调 `refresh_locale_now()` 绕过节拍 |
| 单次 App 状态更新广播，不各自轮询 | 生效值变化经 `App::locale_fetched` → `locale_broadcast_change()` 单点；独立 overlay/webview 各自渲染路径读取共享 `App` 快照 |
| 文案上下文可注入、不引用全局可变 App | `LocaleController` 为纯度状态机（不依赖 `App`/`Client`，输入为 `FetchOutcome`）；`fetch` 的 I/O 由 `App::spawn_locale_fetch` 承担，控制器单测在无 App 环境下运行 |

### 交付物

- `crates/dozer-app/src/platform/locale.rs`（新）：`system_language_candidates`、`system_supported_locale`；注册于 `platform/mod.rs`。
- `crates/dozer-app/src/app/locale.rs`（新）：`LocaleController` + `SnapshotSource`/`FetchOutcome`/`ApplyEffect`；11 单测。
- `crates/dozer-app/src/locale_cache.rs`（新）：最后确认快照缓存；3 单测。
- `crates/dozer-app/src/app/{app,message,update}.rs`：`LocaleController` 字段与启动/轮询/获焦/重连接线；`Message::LocaleFetched`。
- `crates/dozer-app/src/platform/window_events.rs`：2s 节拍唤醒 + 获焦立即刷新。
- `crates/dozer-app/src/main.rs`：`mod locale_cache;`。

### 验证命令与结果

- `cargo test -p dozer-app -- app::locale locale_cache`：11 passed（含 `cadence_limits_to_refresh_interval`、`late_reply_with_wrong_seq_is_dropped`、`reconnect_resets_baseline_and_forces_immediate_fetch`、`same_epoch_never_regresses_revision`、`generation_only_advances_when_active_changes`）。
- `cargo test -p dozer-app`：1985 passed / 1 failed（既有无关 citation 测试）/ 2 ignored。
- `cargo fmt --all -- --check`、`cargo clippy -p dozer-app --all-targets`（新文件零告警）、`scripts/check-log-scope.sh`、`scripts/i18n/check_resources.py`、`scripts/audit/check_panel_boundary.py`：全绿。

### 残留问题

- 文案渲染接缝（`i18n::host_resolver`/`host_text`）目前只被测试消费；实际消费（设置页与面板标题）在 Task 05。
- webview 的 locale 注入与去重键的完整机制在 Task 08；本任务只保证快照与广播到位。
- 2s 节拍恒 active：空闲时也会每 2s 唤醒一次（仅为发起读取，语言无变化不触发重绘；重绘只由 `locale_broadcast_change` 触发）。
- 既有无关测试失败保留：`usage_probe::host::tests::host_site_citations_point_at_the_code_they_name`。
## Task 05：设置入口与面板标题纵向交付

### 交付物

- `crates/dozer-app/src/panel_registry.rs`：`PanelDescriptor` 改为 `{id, title_key, title_fallback, icon}` + `title(locale) -> String`（不在 `OnceLock` 冻结译文、不 `Box::leak`）；测试构造器同步。
- `crates/dozer-app/src/product.rs`：`dozer_catalog` 提供 `panel.*` key + 英文 fallback（不再内联中文）。
- `crates/dozer-app/src/chrome/rail.rs`：`panel_meta(kind, locale: &str) -> (IconKind, String)`；调用点与 3 组标题测试显式传 locale。
- `crates/dozer-app/src/extensions/settings.rs`：新增 `SettingsTab::Language`；`LanguageStatus`（Idle/Saving/Failed/Conflict/Unsupported）+ `LanguageState`（confirmed/pending/shown）；`Message::LanguageSelected`/`LanguageSaveOutcome`；`language_page` 三选项 + 状态行；`settings_card(state, locale)`；tab 按钮/主题行/`provider_row`/`advanced_row` 经 `host_text`（`provider_row`/`advanced_row`/`settings_tab_button` 改为带 `locale`/`'a`）。
- `crates/dozer-app/src/i18n.rs`：`interpolate(template, &[(&str,&str)])` 便捷插值（失败原样返回模板）。
- `scripts/i18n/build_inventory.py`：`NON_UI_STRINGS` 显式排除 4 条面板清单构造器不变量错误（仅 `.expect` panic）。
- `crates/dozer-app/src/platform/settings_overlay.rs`：`settings_card(state, app.locale.active_locale())`（两处）。
- `crates/dozer-app/src/app/app.rs`：`open_settings`（注入已确认偏好）、`sync_settings_locale`、`locale_cache_save`、`spawn_locale_write`（权威 revision + 系统候选 → `set_locale`）、`LocaleController::apply_local_write`。
- `crates/dozer-app/src/app/update.rs`：拦截 `LanguageSelected` 提交；`LanguageSaveOutcome` 先采纳快照（`apply_local_write` + 缓存 + 广播）再交设置状态机；`open_settings` 替换两处 `State::load`。
- `crates/dozer-app/locales/host/{en,zh-CN}.json`：`settings.*`/`settings.language.*`/`panel.*` key。

### 验证命令与结果

- `cargo test -p dozer-app -- language_`：7 passed（乐观选择、Saving 防重复、Applied 清 pending、Failed 保留重试、Conflict 采纳服务端、Unsupported 清 pending、shown 回退）。
- `cargo test -p dozer-app`：1992 passed / 1 failed（既有无关 citation 测试）/ 2 ignored。
- `cargo fmt --all -- --check`、`cargo clippy -p dozer-app --all-targets`（新改文件零告警）、`python3 scripts/i18n/check_resources.py`：全绿。

### 残留问题

- 语言页保存失败文案使用通用文案（不透传服务 message），避免把服务原文直接呈现为 UI 文案。
- 面板标题纵向交付完成的是 Rail 标题；chrome/菜单/overlay 的其余文案在 Task 06。
- 冲突态提示后用户需重新点选（不自动重提交），符合 spec。
- inventory：Task 05 未解决候选 28→3，剩余 3 条（`settings.rs` 断开账号两处 Toast + 写本地记录失败）属 Toast/错误呈现迁移，归 Task 07；4 条面板清单构造器不变量错误按非 UI 显式排除。
- 既有无关测试失败保留：`usage_probe::host::tests::host_site_citations_point_at_the_code_they_name`。

## Task 06：Host chrome、菜单、原生 overlay 与项目创建

### 交付物

- `crates/dozer-app/src/chrome/{rail.rs,topbar.rs,tab_widget.rs,homespace.rs}`：顶部/底部状态、tab、空态经 `host_text`/`host_text_args`。
- `crates/dozer-app/src/app/{view.rs,app.rs}`：`app/view.rs` 展示入口迁移；`app/app.rs` 原生右键菜单 builder 带 `locale`（`project_link_menu_items`/`text_input_menu_items`/`database_source_menu_items`/`category_context_menu_items`）；`list_collapse_button` tooltip 放宽为 `impl Into<Cow<'a,str>>`（`dozer-panel-usage` trait + `usage_panel`/`fake_panel_host` 两处实现）。
- `crates/dozer-app/src/workspace/view.rs`：`relative_time_text`/`format_time` 带 `locale`；预览 fallback 页、T10 冲突条、脏重载提示迁移。
- `crates/dozer-app/src/workspace/state.rs`：`review_content(content, ws, locale)`。
- `crates/dozer-app/src/extensions/{settings.rs,project_create.rs,search.rs,edit_history.rs,file_history.rs,settings_apps_view.rs,conversations.rs,project/view.rs}`：全部改为带 `locale` 的视图/状态更新入口。
- `crates/dozer-app/src/platform/{project_create_overlay.rs,search_overlay.rs,edit_history_overlay.rs,file_history_overlay.rs,confirm_overlay.rs,window_events.rs}`：overlay 宿主在 `redraw`/`handle_input` 先取 `locale` 再借用可变状态；`window_events.rs` 文件选择器文案经 `host_text`。
- `crates/dozer-app/src/menu_spec.rs`：`MenuSpecItem::entry(icon, label: impl Into<String>, msg)`。
- `crates/dozer-app/src/i18n.rs`：新增 `host_text_args(key, fallback, locale, &[(&str,&str)]) -> String`；**删除** `interpolate` 及其测试（无法渲染具名模板会落到 `SAFE_FALLBACK`）。
- `crates/dozer-app/locales/host/{en,zh-CN}.json`：Task 06 新增 `host.*`/`project_create.*`/`search.*`/`edit_history.*`/`file_history.*`/`settings.apps.*` 等 key（两包 key 集与占位符集一致）。
- `scripts/i18n/build_inventory.py`：`STRING_TASK_OVERRIDES` 把 `app/app.rs` 的 5 条 Toast/错误文案改判 Task 07。
- byteui（独立仓库）：`text_area::view`/`checkbox::view`/`switch::view`/`form::button::view` 的 label/placeholder 放宽为 `&str`（v0.4.2）；dozer 两侧 Cargo pin 升到 `v0.4.2`。

### 验证命令与结果

- `cargo fmt --all -- --check`：全绿。
- `cargo clippy -p dozer-app --all-targets`：仅既有无关告警（`GitProvider::ALL`、`project::move_item`、`confirm_overlay` 参数过多、`homespace` 文档缩进等）；无新增。
- `python3 scripts/i18n/check_resources.py`：ok（两包 key/占位符一致）。
- `bash scripts/check-log-scope.sh`、`python3 scripts/audit/check_panel_boundary.py`：全绿。
- `cargo test -p dozer-app`：1993 passed / 1 failed（既有无关 citation 测试）/ 2 ignored。

### 残留问题

- inventory 中 Task 06 未解决候选清零；重投影后归入 07/08/09/10/11。
- 结构化错误（`edit_history::RevertFailure`、搜索正则错误）保存结构化原因、视图层按 `locale` 本地化；不做服务 message 文本匹配。
- 应用页文字中来自 `bytehost_panel::install`（`plan_lines`/`plan_view`/`observed_label`/`progress_text`/`rollback_note_label`/`runtime_line`/`managed_runtime_label`）的数据保持 bytehost 自产，不在本任务范围；`agent_picker_spec()` 的 agent 品牌名、`issue_texts(issue)` 同样不在范围。
- 缓存译文以 key/参数语义存储（`list_collapse_button` tooltip 存 key），切换后独立窗口在 `redraw` 重取 locale。
- 既有无关测试失败保留：`usage_probe::host::tests::host_site_citations_point_at_the_code_they_name`。

## Task 07：Toast 与错误呈现迁移

### 交付物

- `crates/dozer-app/src/extensions/toast.rs`：`Toast.text`/`Outbox::Pending.text` 由裸 `String` 改为 `LocalizedText`（owner = `dozer.host`）；`ToastCenter::push` 与 `log_toast` 收 `impl Into<LocalizedText>`；空判定与去重基于语言无关稳定值 `stable_value`（无 key 的裸字符串 fallback 空白时返回空，不落 `SAFE_FALLBACK`）；渲染在 `platform/toast_overlay.rs`（`host_render` 后 `normalize` 折叠空白），日志走 `host_render_stable`。`failure_message(scope, key, fallback, res)` 改为操作语义 key 形态。测试辅助 `Toast::text_stable`/`Pending::text_stable`。
- `crates/dozer-app/src/i18n.rs`：新增 `host_render`/`host_render_stable`/`host_keyed`/`host_keyed_args`（后台产出无 locale 时构造 keyed 文案，视图层再渲染）。
- `crates/dozer-model/src/i18n.rs`：`LocalizedText::DEFAULT_OWNER` 与 `impl From<&str>`/`From<String>`（便利实现，让 `impl Into<LocalizedText>` 入口直接收裸字符串）。
- 结构化错误字段：`GitAccountError`（`git_accounts.rs`）、`ProjectFsError`/`FsOp`（`project.rs`）、`RepoListState::Error(LocalizedText)`（`project_create.rs`）、`WorkspaceState::preview_error`/`project_preview_error: Option<LocalizedText>`、`summary_errors: HashMap<String, LocalizedText>`（`conversations.rs`）；`workspace/hook.rs::agent_state_label(state, locale)`；`term/terminal.rs`。
- `crates/dozer-app/locales/host/{en,zh-CN}.json`：新增 `error.*`（keychain/network/token/git_account/open_with_app_failed/open_with_default_failed）、`preview.enter_code_mode_failed`/`file_missing_locate`/`file_unreadable`、`review.no_conversation`、`agent.state_*`、`term.no_sessions`、`file_history.load_failed`、`todo.process_failed`、`conversations.summary_*` 等 key（两包 key 集与占位符集一致）。
- `scripts/i18n/build_inventory.py`：新增 NON_UI 条目（`runtime.rs`/`app/state.rs`/`workspace/state.rs`/`app/update.rs`）与预览组改判 Task 11、SSH 连接错误改判 Task 10；`web/shared/i18n/i18n.ts`（运行时 `I18nError` 诊断文案）按非 UI 文件排除。

### 验证命令与结果

- `cargo fmt --all -- --check`：全绿。
- `cargo clippy -p dozer-app --all-targets`：仅既有无关告警；无新增。
- `python3 scripts/i18n/check_resources.py`：ok（两包 key/占位符一致）。
- `bash scripts/check-log-scope.sh`、`python3 scripts/audit/check_panel_boundary.py`：全绿。
- `cargo test -p dozer-app`：1993 passed / 1 failed（既有无关 citation 测试）/ 2 ignored。

### 残留问题

- inventory 中 Task 07 未解决候选清零；预览组（`web_error`/`PreviewError.message`/T10 冲突/资源预算 6 条）与 SSH 连接错误 4 条按任务重投影到 Task 11/10。
- 既有无关测试失败保留：`usage_probe::host::tests::host_site_citations_point_at_the_code_they_name`。

## Task 08：WebView locale 传递与资源打包基础

### 交付物

- `crates/dozer-model/src/i18n.rs`：新增 `LocaleStamp{ui_locale: Locale, locale_revision: u64}`（serde，线上形状即 spec §8 示例）+ 泛型载体 `Stamped<T>{stamp, payload}`（**非**线上形状，仅宿主本地比较用）。落在此中性 crate 因 `dozer-panel-usage` 不得依赖 `dozer-app`（`check_panel_boundary.py` 通过）。
- 四种推送 envelope 追加兼容字段 `#[serde(flatten, skip_serializing_if = "Option::is_none")] locale: Option<LocaleStamp>`：`extensions/todo/protocol.rs`（`encode_todo_push`）、`extensions/codehealth/protocol.rs`（`encode_codehealth_push`）、`extensions/group_chat/protocol.rs`（`encode_group_chat_push`）、`dozer-panel-usage/src/protocol.rs`（`encode_usage_push`，**同步补 `revision`**，此前 Usage envelope 无 revision）。缺字段 = 旧 Host，前端按 `en` 处理。
- Surface 去重改含 locale：`extensions/codehealth/surface.rs` 与 `dozer-panel-usage/src/surface.rs` 的 `SurfaceExecutor<Stamped<ViewPayload>>`，`build_script` 从 `Stamped` 同时取 stamp 与 payload。公共 `SurfaceExecutor<T>` 未改（框架中立，仅包装产品 payload 即解决）。
- `WebviewPushState.last_sent: Option<Stamped<..>>`（todo/group_chat）：`pending_push`/`mark_sent` 同时比较/保存 stamp。
- `crates/dozer-app/src/app/app.rs`：`App::locale_stamp()`（`active_locale()` + `display_generation()`，GUI 本地显示代，不与 daemon revision 比大小）；四处推送点（`take_usage_content_script`/`take_codehealth_content_script`/`take_todo_content_script`/群聊推送）接入。
- 前端：`web/{todo,group-chat,codehealth}-content/src/protocol.ts` 的 `PushState` 增 `locale`/`localeRevision`，`applyEnvelope` 原子应用并把 `document.documentElement.lang` 同步为当前语言；各自 `main.tsx` 初始态补齐；`dozer-panel-usage/web/usage-content/src/main.tsx` 增响应式 locale 信号 + html lang。
- `scripts/i18n/build_inventory.py`：`NON_UI_STRINGS` 新增 `preview/webview_protocol.rs` 的 8 条 IPC 校验 `Display`（只在 `log_warn!` 里格式化，不面向用户）。

### 验证命令与结果

- `cargo build -p dozer-app -p dozer-panel-usage --tests`：通过。
- `cargo test -p dozer-panel-usage`：54 passed。
- `cargo test -p dozer-app`：1996 passed / 1 failed（既有无关 citation 测试）/ 2 ignored；新增用例含 todo/group_chat/usage/codehealth 的"业务 payload 不变、仅语言变仍推送"与 envelope locale 内联/缺省。
- `node --test`：todo `protocol.test.ts`（7）、group_chat `protocol.test.ts`（6）、codehealth `protocol.test.ts`（5）全绿（含 locale 原子应用、旧 Host 沿用上一语言、同 revision 语言推送）。
- `cargo fmt --all -- --check`、`cargo clippy -p dozer-app -p dozer-panel-usage --all-targets`（无新增告警）、`python3 scripts/i18n/check_resources.py`、`bash scripts/check-log-scope.sh`、`python3 scripts/audit/check_panel_boundary.py`：全绿。
- `python3 scripts/i18n/build_inventory.py`：Task 08 未解决候选清零（`counts: {"excluded": 3077, "migrated": 0, "unresolved": 773}`；`unresolved by task: {"09": 240, "10": 418, "11": 115}`；未分配 0）。

### 残留问题（本任务显式延期，用户裁决）

- **字符串抽取**：按计划归 Task 09–11，本任务不抽任何面板/前端文案。
- **共享前端解析器接线**：`web/shared/i18n` 的 `Resolver` 实际查表渲染留待抽取之后；本任务只把 locale 作为响应式信号落到 `PushState` 与 `document.documentElement.lang`。
- **`locales/` 面板资源目录与 `build.mjs` 资源打包 / 构建失败门禁**（计划 Task 08 第 5 项）：资源尚未产出，随 Task 09 首批面板资源一并落地。
- **自有 HTML/预览 chrome 的最小语言消息**（计划 Task 08 第 6 项）：随 Task 11 迁移。
- 既有无关测试失败保留：`usage_probe::host::tests::host_site_citations_point_at_the_code_they_name`。

## Task 09 验收（Todo 面板，按面板拆分提交）

计划 Task 09 面向 Todo/Usage/Code Health 三个首批面板，按"每个面板单独盘点→抽取→替换→双语测试→清零"。本记录只覆盖 **Todo 面板**。

### 交付内容

- **资源**：`crates/dozer-app/locales/panels/todo/{en,zh-CN}.json`（owner `dozer.todo`）；宿主通用文案 `webview.load_timeout` 落 `crates/dozer-app/locales/host/{en,zh-CN}.json`。
- **Rust 接缝**：`i18n.rs` 增 `PANEL_OWNERS` + `panel_text`/`panel_text_args` + `build_host_resolver` 载入面板 owner；`panel_host.rs` 的 `PanelHost` 增 `locale()`（默认 `"en"`，`App` 返回 `active_locale()`）。
- **Rust 迁移**：`extensions/todo/view.rs`（标题/视图 tab/看板占位/折叠展开/加载失败+重试/清空列表/分类导航/详情卡/日历/确认弹窗）、`extensions/todo/update.rs`（三类操作失败 + 加载失败，改为面板 keyed `LocalizedText`）、`extensions/todo/protocol.rs`（加载超时改 keyed，`failed: Option<LocalizedText>`，`set_failed` 收 `LocalizedText`）、`platform/confirm_overlay.rs`、`platform/todo_detail_overlay.rs`（传 `active_locale()`）、`app/update.rs` 的 `set_failed`。
- **Web 迁移**：`web/todo-content/src/i18n.ts`（静态打包面板 JSON + `setLocale`/`t`，复用 `web/shared/i18n` 的 `Resolver`）、`main.tsx`（dispatch 时 `setLocale`）、`statusMeta.ts`（改存 `labelKey` + `statusLabel`）、`components/{App,AddBox,Popovers,TodoCard,Toolbar}.tsx`。兑现 Task 08 延后的"共享解析器接线 + `locales/` 目录"。
- **豁免**：`新分类`（新建分类默认名，用户数据初始值，按"保留用户分类名"）记入 `build_inventory.py` 的 `NON_UI_STRINGS`。

### 验证命令与结果

- `cargo build -p dozer-app`：通过。
- `cargo test -p dozer-app`：1994/1996 passed；2 failed = 既有无关 citation 测试 + 既有 flaky `assets::tests::serves_vendored_asset_with_mime`（单测隔离通过，`scratch()` 按进程 id 共享目录，并行下偶发）。
- `node --test src/*.test.ts`（todo-content）：60 passed（含新增 `i18n.test.ts`、改写 `statusMeta.test.ts` 的双语与回退用例）。
- `cargo fmt --all -- --check`、`cargo clippy -p dozer-app --all-targets`（无新增告警；顺带移除因迁移而孤立的 `Outbox::push_err`）、`python3 scripts/i18n/check_resources.py`、`bash scripts/check-log-scope.sh`、`python3 scripts/audit/check_panel_boundary.py`：全绿。
- `python3 scripts/i18n/build_inventory.py`：Task 09 内 todo 项候选清零（`counts: {"excluded": 3117, "migrated": 0, "unresolved": 683}`；`unresolved by task: {"09": 150, "10": 418, "11": 115}`；未分配 0）。

### 残留

- Task 09 剩余 150 条属 **Usage（27）** 与 **Code Health（123）**，各按面板后续单独提交。
- 双语**运行期视觉核对**（切换后原生/Web 同步）随 Task 13 的 L07/L09/L10/L14 执行。

## Task 09 验收（Usage 面板，按面板拆分提交）

计划 Task 09 面向 Todo/Usage/Code Health 三个首批面板。本记录只覆盖 **Usage 面板**（Todo 见上一节）。

### 交付内容

- **资源（独立 crate 自带）**：`crates/dozer-panel-usage/locales/{en,zh-CN}.json`（owner `dozer.usage`）；面板文案不外置到 dozer-app。
- **Rust 接缝**：`dozer-panel-usage/src/i18n.rs`（新增）用中性 `dozer-i18n` 解析器 `include_str!` 自身两种语言包，暴露 `USAGE_OWNER`/`t`/`t_static`/`resolve`，并含 en/zh key-parity 单测；`Cargo.toml` 增 `dozer-i18n` 依赖。**不反向依赖 dozer-app / daemon**——单面板 FakePanelHost 测试宿主即可渲染（计划 §Task 09 第 4 条）。
- **Rust 迁移**：`src/adapter.rs`（`UsageNotice.text: LocalizedText`、`UsageOutbox::{push,push_plain}` 收 `LocalizedText`、新增 `PanelHeadKind{Content,AgentFilter}` 令 `PanelDisplay::panel_head(kind, title, actions)` 按语义而非标题字面选图标）、`src/state.rs`（加载失败改 keyed `LocalizedText`）、`src/view.rs`（标题/折叠展开/加载中/筛选等经 `i18n::t`，`t_static` 供只收 `&'static str` 的 `loading_hint`）、`src/lib.rs`（导出 `PanelHeadKind`）。
- **原生调用方**：`app/view.rs`（传 `app.locale.active_locale()`）、`panel_host.rs`、`usage_probe/{scenarios,fake_panel_host}.rs`、`host_adapters/usage_panel.rs`（`panel_head` 按 kind 选图标、`notices_to_pending` 直传 `LocalizedText`）。
- **Web 迁移**：`web/usage-content/src/i18n.ts`（新增；**经相对路径静态 import 共享 `web/shared/i18n` 解析器**，并静态打包同一份 `dozer.usage` JSON）、`main.tsx`（dispatch 时 `setLocale` + 响应式 locale 信号）、`theme.ts`（`SERIES_META.label`→`labelKey`）、`components/{App,EmptyStates,ProjectSummaryBoxes,TrendSection,TrendLineChart}.tsx`。数字/金额/聚合与数据排序未改。
- 计划 Task 09 第 6 条"独立库自我携带资源"由本面板首次落地。

### 验证命令与结果

- `cargo build -p dozer-panel-usage -p dozer-app`：通过。
- `cargo test -p dozer-panel-usage`：58 passed（含新增 `i18n.rs` 的 en/zh key-parity 等 4 条）。
- `cargo test -p dozer-app`：1995 passed / 1 failed；唯一 failure 是**既有无关**的 `usage_probe::host::tests::host_site_citations_point_at_the_code_they_name`（在 HEAD `ec183de2` 的干净 worktree 上同样失败，与本次改动无关）。
- `node --test src/*.test.ts`（usage-content）：18 passed（含新增 `i18n.test.ts`、改写 `theme.test.ts` 的双语/回退用例）；`npm run typecheck`、`npm run build`（重建 `assets/usage-content/usage-content.js`）：通过。
- `cargo fmt --all -- --check`、`cargo clippy -p dozer-panel-usage -p dozer-app --all-targets`（无新增告警）、`python3 scripts/i18n/check_resources.py`、`bash scripts/check-log-scope.sh`、`python3 scripts/audit/check_panel_boundary.py`：全绿。
- `python3 scripts/i18n/build_inventory.py`：Task 09 内 usage 项候选清零（`counts: {"excluded": 3122, "migrated": 0, "unresolved": 656}`；`unresolved by task: {"09": 123, "10": 418, "11": 115}`）。

### 残留

- Task 09 剩余 123 条全部属 **Code Health**，按面板后续单独提交。
- 双语**运行期视觉核对**（切换后原生/Web 同步）随 Task 13 的 L07/L09/L10/L14 执行。

## Task 09 验收（Code Health 面板，按面板拆分提交）

计划 Task 09 面向 Todo/Usage/Code Health 三个首批面板。本记录覆盖 **Code Health 面板**（Todo/Usage 见上两节），任务 §Task 09 第 3 条（spec §167）在此完整落地。

### 交付内容

- **资源**：`crates/dozer-app/locales/panels/codehealth/{en,zh-CN}.json`（owner `dozer.code_health`，含 `analyze.*` 与 `error.*`）。
- **Rust 迁移（keyed 语义 + 英文兜底）**：`extensions/codehealth/mod.rs`（`CODE_HEALTH_OWNER`、`CodeHealthCategory::label_key`/`label(locale)`、加载失败改 keyed `error.load_failed`）、`view.rs`（locale-aware `list_pane`/`content_pane`/`category_button`）、`view_model.rs`（`hotspot_reasons`/`scope_summary`/`tier_label`/`sev_label`/`change_label` 经 key，`analyze_finding_text` 收 `locale` 并按 `analyze.*` 渲染）、`protocol.rs`/`arch_payload.rs`（`ScanBar` 增 `ErrorDto`；旧 `*_label`/`*_text` 兼容字段统一改**英文**）、`aggregate.rs`（扫描/保存错误改带稳定 key 的 `ErrorDto` + 英文兜底文案，spec §10）。
- **Rust 接缝**：`app/update.rs`（`code_health_analyze_finding` 取 `self.locale.active_locale()` 传入）、`i18n.rs`（`PANEL_OWNERS` 增 `dozer.code_health`）。
- **Web 迁移**：`web/codehealth-content/src/i18n.ts`（新增；静态 import 共享 `web/shared/i18n` 解析器 + 面板 JSON、`errorText(ErrorDto)`）、`types.ts`（`ErrorDto`）、`main.tsx`（dispatch 后 `setLocale`）、`components/{App,ScanHeader,EmptyState,FindingRow,OverviewPage,StructurePage,UiPage,ScopePage,ArchitecturePage}.tsx`（旧 Host 预格式化字段保留为兜底，语义字段优先）。

### 验证命令与结果

- `cargo build -p dozer-app`：通过。
- `cargo test -p dozer-app codehealth`：106 passed / 0 failed（含改写为英文断言的 `view_model`/`protocol`/`arch_payload` 用例与 `analyze_text_localizes_to_chinese`）。
- `node --test src/*.test.ts`（codehealth-content）：18 passed；`npm run typecheck`、`npm run build`（重建 `assets/codehealth-content/codehealth-content.js`）：通过。
- `cargo fmt --all -- --check`、`cargo clippy -p dozer-panel-usage -p dozer-app --all-targets`、`python3 scripts/i18n/check_resources.py`、`bash scripts/check-log-scope.sh`、`python3 scripts/audit/check_panel_boundary.py`：全绿。
- `python3 scripts/i18n/build_inventory.py`：Task 09 内 code health 项候选**清零**（`counts: {"excluded": 3111, "migrated": 0, "unresolved": 533}`；`unresolved by task: {"10": 418, "11": 115}`，无 `09`）。

### 残留

- Task 09 三个面板（Todo/Usage/Code Health）全部完成。
- 双语**运行期视觉核对**（切换后原生/Web 同步）随 Task 13 的 L07/L09/L10/L14 执行。

## Task 10 验收（git_log 面板，按面板拆分提交）

### 交付内容

- 资源：`crates/dozer-app/locales/panels/git_log/{en,zh-CN}.json`（owner `dozer.git_log`）。
- 迁移：`extensions/git_log.rs`（`GIT_LOG_OWNER`、本地 `t()`、`loading_hint_localized()`；`State.{error,detail,diff_load_error}` 与 `Message::{DetailLoaded,SnapshotLoaded,DiffContentLoaded,BranchSwitchDone}` 的错误改携带 `LocalizedText`；后台任务经 `i18n::panel_keyed_args` 造 keyed 文案、渲染端 `render_localized` 解析；`view`/`file_list_view`/`diff_pane_view`/`git_panel_footer_bar`/`branch_picker_view` 穿 `locale`）、`extensions/diff_content.rs`（`NotRenderable.reason` 由 `String` 改 `LocalizedText`）、`i18n.rs`（`PANEL_OWNERS` 增 `dozer.git_log`、新增 `panel_keyed_args`/`render_localized`）、`app/update.rs`（分支切换失败 keyed）。
- 附带修复：`usage_probe/host.rs` 的 `host_site()` 引用行号（`BuildActivationCtx`/`RouteLoaded`/`DropOtherProject`/`Intercept*`/`FramePush`/`DowngradeReady`/`SkipWhileLoading`）更新为当前实际行——该 `host_site_citations_*` 测试在提交 `e75a251d` 时**已因既有漂移失败**，此处一并修正。

### 验证命令与结果

- `cargo build -p dozer-app`：通过。
- `cargo test -p dozer-app git_log`：78 passed / 0 failed。
- `cargo test -p dozer-app`：**2001 passed / 0 failed**（含修复后的 `usage_probe::host::tests`）。
- `cargo fmt --all -- --check`、`cargo clippy -p dozer-panel-usage -p dozer-app --all-targets`（仅既有告警）、`python3 scripts/i18n/check_resources.py`（5 个 owner 目录）、`bash scripts/check-log-scope.sh`、`python3 scripts/audit/check_panel_boundary.py`：全绿。
- `python3 scripts/i18n/build_inventory.py`：git_log 与 diff_content 路径候选**清零**（`counts: {"excluded": 3111, "migrated": 0, "unresolved": 504}`；`unresolved by task: {"10": 389, "11": 115}`，无 git_log/host 残留）。

### 残留

- Task 10 其余面板（host/ssh/web/conversations/agent/files/database/project/group_chat）按面板逐个提交。
- 双语运行期视觉核对随 Task 13 执行。

## Task 10 验收（ssh 面板，按面板拆分提交）

### 交付内容

- 资源：`crates/dozer-app/locales/panels/ssh/{en,zh-CN}.json`（owner `dozer.ssh`）。
- 迁移：`extensions/ssh.rs`（`SSH_OWNER` + 本地 `t()`；`ssh_error_localized`/`terminal_error_{unknown_host,key_changed,timeout,channel}`/`os_probe_timeout` 供后台任务造 keyed 文案；`TestStatus::Err` 与 `Message::{TestConnectionResult,TerminalConnectFailed}` 错误改 `LocalizedText`、`FetchOsResult` 错误改 `LocalizedText`；`host_card`/`ssh_host_card`/`radio_dot`/`text_btn` 收 `String` 标签并穿 `locale`；`view`/`ssh_footer_bar`/`delete_confirm_spec` 穿 `locale`；`persist_hosts` 失败 Toast 改 keyed）、`extensions/ssh/sftp.rs`（`SftpTabState.status` 改 `LocalizedText`、树标题/上传下载菜单/连接失败/传输状态全 keyed；`sftp_pane_view`/`sftp_context_menu`/`context_menu_items`/`tree_column` 收 `locale`）、`app/{view,update}.rs`（`sftp_pane_view`/`context_menu_items` 传 `active_locale()`；`ssh_test_connection_result`/`ssh_terminal_connect_failed` 收 `LocalizedText`）、`workspace/state.rs`（终端建连各错误点改调 `ssh::terminal_error_*`/`ssh_error_localized`，移除裸中文）、`platform/{ssh_host_overlay,confirm_overlay}.rs`（穿 `locale`）。
- 附带修复：`scripts/i18n/scan_sources.py` 的 `strip_rust_comments` 未跳过含引号的 Rust 字符字面量（如 `trim_matches('"')`），导致 ssh.rs 从该处起的真实 UI 字符串被误判为注释吞掉；修复后 ssh.rs 候选由 25 恢复为 40。

### 验证命令与结果

- `cargo build -p dozer-app`：通过。
- `cargo test -p dozer-app`：**2001 passed / 0 failed**（含更新后的 `delete_confirm_spec_*` 与 `context_menu_items_*` 用例）。
- `cargo fmt --all -- --check`、`cargo clippy -p dozer-panel-usage -p dozer-app --all-targets`（仅既有告警）、`python3 scripts/i18n/check_resources.py`（6 个 owner 目录）、`bash scripts/check-log-scope.sh`、`python3 scripts/audit/check_panel_boundary.py`：全绿。
- `python3 scripts/i18n/build_inventory.py`：ssh 面板路径候选**清零**（`counts: {"excluded": 3111, "migrated": 0, "unresolved": 466}`；`unresolved by task: {"10": 351, "11": 115}`；`dozer.ssh` 0 条）。

### 残留

- Task 10 其余面板（host/web/conversations/agent/files/database/project/group_chat）按面板逐个提交。
- 双语运行期视觉核对随 Task 13 执行。

## Task 10 验收（web/browser 面板，按面板拆分提交）

### 交付内容

- 资源：`crates/dozer-app/locales/panels/web/{en,zh-CN}.json`（owner `dozer.web`）。
- 迁移：`extensions/browser.rs`（`WEB_OWNER` + 本地 `t()`；`Tabs::addr_submit` 返回 `Err(dozer_model::LocalizedText)`（key `error.local_path`）；`State.error` 由 `Option<String>` 改 `Option<LocalizedText>`，收藏夹读/写失败经 `i18n::panel_keyed_args`（`error.bookmarks_load_failed`/`error.bookmarks_op_failed`）；`view`/`nav_button`/`star_button`/`bookmarks_toggle_button`/`bookmarks_panel` 穿 `locale`，`bookmark_group` 标题改收 `String`；导航/星标/收藏夹/新标签页 tooltip、地址占位、侧栏分组与空态、空页面、错误行经 `t`/`render_localized`）、`app/view.rs` 与 `chrome/homespace.rs`（`browser::view` 传 `app.locale.active_locale()`）、`i18n.rs`（`PANEL_OWNERS` 增 `dozer.web`）。
- 附带修复：`scripts/i18n/scan_sources.py` 的 `is_test_context` 之前用"最后一个 `#[cfg(test)]` 位置"判定测试区，当测试模块 `mod tests` 之后还有别的 `#[cfg(test)]` 项（本文件有 `#[cfg(test)] impl Message`）时边界被推到文件末尾，测试模块内的夹具字面量被误判为 UI 文案；改为优先取 `#[cfg(test)] mod .. {` 模块起界后，browser.rs 的 7 条测试夹具字面量与 `dozer.database` 的同类 1 条一并归为 excluded。

### 验证命令与结果

- `cargo build -p dozer-app`：通过。
- `cargo test -p dozer-app`：**2001 passed / 0 failed**（含更新后的 `addr_submit`/`update_addr_submit_local_path_*`/`update_bookmarks_mutated_error_*`/收藏夹读失败/`stale_*` 用例）。
- `cargo fmt --all -- --check`、`cargo clippy -p dozer-panel-usage -p dozer-app --all-targets`（仅既有告警）、`python3 scripts/i18n/check_resources.py`（7 个 owner 目录）、`bash scripts/check-log-scope.sh`、`python3 scripts/audit/check_panel_boundary.py`：全绿。
- `python3 scripts/i18n/build_inventory.py`：browser 路径候选**清零**（`counts: {"excluded": 3114, "migrated": 0, "unresolved": 445}`；`unresolved by task: {"10": 330, "11": 115}`；`dozer.web` 0 条）。

### 残留

- Task 10 其余面板（host/conversations/agent/files/database/project/group_chat）按面板逐个提交。
- 双语运行期视觉核对随 Task 13 执行。

## Task 10 验收（conversations 面板，按面板拆分提交）

### 交付内容

- 资源：`crates/dozer-app/locales/panels/conversations/{en,zh-CN}.json`（owner `dozer.conversations`）。
- 迁移：`extensions/conversations.rs`（`CONVERSATIONS_OWNER` + 本地 `t()`/`t_args()`；`loading_hint_localized()` 复刻 `byteui` 的 `loading_hint` 以吃下运行时本地化文案；列表读失败 Toast 经 `i18n::panel_keyed_args` 改 keyed（`error.list_load_failed`）；`footer_bar`/`agent_picker_items`/`agent_picker_view` 收 `locale`（"全部"→`filter.all`）；`view` 的面板标题、搜索占位、加载提示、空态（`empty.no_sessions`/`empty.no_match`）、行副标题（`row.current`/`row.normal` + `row.task_suffix`/`row.task_suffix_truncated` 经 `t_args` 插值）、"更多" tooltip 全 keyed）。
- 接线：`app/update.rs`（`agent_picker_items` 传 `self.locale.active_locale()`）。
- 附带：`usage_probe/host.rs` 的职责引用行号随 `app/update.rs` 行位移同步（`BuildActivationCtx` 5421→5426、`RouteLoaded`/`DropOtherProject` 1646→1651）。

### 验证命令与结果

- `cargo build -p dozer-app`：通过。
- `cargo test -p dozer-app`：**2001 passed / 0 failed**（含 `agent_picker_items_*` 用例更新为传 `"en"`）。
- `cargo fmt --all -- --check`、`cargo clippy -p dozer-panel-usage -p dozer-app --all-targets`（仅既有告警）、`python3 scripts/i18n/check_resources.py`（8 个 owner 目录）、`bash scripts/check-log-scope.sh`、`python3 scripts/audit/check_panel_boundary.py`：全绿。
- `python3 scripts/i18n/build_inventory.py`：conversations 面板路径候选**清零**（`counts: {"excluded": 3114, "migrated": 0, "unresolved": 432}`；`unresolved by task: {"10": 317, "11": 115}`；`dozer.conversations` 0 条）。

### 残留

- Task 10 其余面板（host/agent/files/database/project/group_chat）按面板逐个提交。
- 双语运行期视觉核对随 Task 13 执行。

## Task 10 验收（agent 面板 Rust，按面板拆分提交）

### 交付内容

- 资源：`crates/dozer-app/locales/panels/agent/{en,zh-CN}.json`（owner `dozer.agent`，含 `strip.label`/`action.history`/`action.remove`/`entry.missing`/`error.*`）。
- 迁移：`extensions/agent_context.rs`（`AGENT_OWNER` + 本地 `t()`/`t_args()`；三条 outbox Toast 改 `i18n::panel_keyed_args` keyed；`strip`/`expanded_panel` 收 `locale`，条标题计数、修改历史、逐项历史/移除、缺失标记全 keyed；`expanded_panel` 加显式 `<'a>` 绑 `state: &'a State`）。
- 接线：`term/terminal.rs`（`strip`/`expanded_panel` 传 `app.locale.active_locale()`）。
- 范围裁决：`web/review-trace/`（Agent 审阅侧 webview host）归 **Task 11**（owner 仍 `dozer.agent`）；`build_inventory.py` 任务号 10→11。

### 验证命令与结果

- `cargo build -p dozer-app`：通过。
- `cargo test -p dozer-app`：**2001 passed / 0 failed**（`added_failure_*` 断言改英文 fallback "Sent to terminal"）。
- `cargo fmt --all -- --check`、`cargo clippy -p dozer-app --all-targets`（仅既有告警）、`python3 scripts/i18n/check_resources.py`（9 个 owner 目录）、`bash scripts/check-log-scope.sh`、`python3 scripts/audit/check_panel_boundary.py`：全绿。
- `python3 scripts/i18n/build_inventory.py`：`counts: {"excluded": 3113, "migrated": 0, "unresolved": 424}`；`unresolved by task: {"10": 302, "11": 122}`；`dozer.agent` Task 10 Rust 路径 **0 条**（余 8 条全为 Task 11 的 `web/review-trace/`）。

### 残留

- Task 10 其余面板（host/files/database/project/group_chat）按面板逐个提交。
- `web/review-trace/` 在 Task 11 迁移。
- 双语运行期视觉核对随 Task 13 执行。

## Task 10 验收（host：SFTP 建连超时文案，按面板拆分提交）

### 交付内容

- 迁移：`extensions/ssh.rs`（新增 `sftp_connect_failed(e)` → keyed `sftp.connect_failed`；复用已提交的 `terminal_error_timeout()` → `error.timeout_terminal`）。
- `extensions/ssh/sftp.rs`（`Message::Connected` 的 `Err` 臂由 `String` 改 `dozer_model::LocalizedText`，`Err(e)` 直接作 status 文案）。
- `workspace/state.rs`（`spawn_sftp_tab` 握手失败经 `ssh::sftp_connect_failed`、10 秒超时经 `ssh::terminal_error_timeout()`、`canonicalize` 失败经 `sftp_connect_failed`；删除裸中文 `"连接超时(10 秒)"`）。

### 验证命令与结果

- `cargo build -p dozer-app`：通过。
- `cargo test -p dozer-app`：**2001 passed / 0 failed**。
- `cargo fmt --all -- --check`、`cargo clippy -p dozer-app --all-targets`（仅既有告警）、`python3 scripts/i18n/check_resources.py`（9 个 owner 目录）、`bash scripts/check-log-scope.sh`、`python3 scripts/audit/check_panel_boundary.py`：全绿。
- `python3 scripts/i18n/build_inventory.py`：`counts: {"excluded": 3113, "migrated": 0, "unresolved": 423}`；`unresolved by task: {"10": 301, "11": 122}`；`dozer.host` Task 10 路径 **0 条**（余 37 条全在 Task 11 的 HTML 模板/预览预算）。

### 残留

- Task 10 其余面板（database/project/group_chat）按面板逐个提交。
- `dozer.host`/`dozer.agent` 的 Task 11 部分随 Task 11 迁移。
- 双语运行期视觉核对随 Task 13 执行。

## Task 10 验收（files 面板，按面板拆分提交）

### 交付内容

- 资源：`crates/dozer-app/locales/panels/files/{en,zh-CN}.json`（owner `dozer.files`；`head`/`search`/`dotfiles`/`drag_ghost`/`git.*`/`agent.ref_*`/`menu.*`/`kind.*`/`confirm.delete.*`/`move.*`/`edit.error.*`/`toast.*`）。
- `extensions/files/mod.rs`：`FILES_OWNER` + 本地 `t()`/`t_args()`。
- `extensions/files/view.rs`：`view`/`git_footer_bar`/`branch_picker_popup`/`context_menu_spec`/`context_menu_items`/`context_menu_popup`/`preview_selection_context_menu_spec`/`preview_selection_context_menu_items`/`preview_selection_context_menu_popup`/`tab_context_menu_spec`/`tab_context_menu_items`/`tab_context_menu_popup`/`delete_confirm_spec`/`files_move_card` 全穿 `locale`；`agent_context_reference_text`/`selection_reference_text` 增 `locale`；`move_error` 经 `crate::i18n::render_localized`。
- `extensions/files/tree.rs`：`tree_drag_ghost` 多选文案 keyed（`drag_ghost.multi`）。
- `extensions/files/state.rs`：`move_error` 由 `Option<String>` 改 `Option<dozer_model::LocalizedText>`（`set_move_error` 包 `LocalizedText::literal(FILES_OWNER, ..)`；accessor 返回 fallback）；`submit_tree_edit` 两条行内校验 Toast 改 keyed（`edit.error.invalid_name`/`edit.error.exists`）；`Message::RequestSendToAgentTerminal` 由携带成品文本改为 `{ is_dir, relative }`，模板由内核按 locale 拼。
- `extensions/files/update.rs`：6 条 Toast 与 2 条 `move_error` 改 `panel_keyed`/`panel_keyed_args`；`send_payload` 返回 `(String, bool)`；`update` 增 `locale` 形参（加 `#[allow(clippy::too_many_arguments)]`，与 `database/update.rs` 同款）。
- `app/{view,update}.rs`：全部 files 视图/菜单/确认/移动卡入口与 `files::update` 传 `app.locale.active_locale()`；`RequestSendToAgentTerminal` 处理改按 locale 拼模板；`app/update.rs` 行号迁移致 `usage_probe/host.rs` 引用 5426→5437。
- `platform/{files_move_overlay,confirm_overlay}.rs`：`files_move_card`/`delete_confirm_spec` 传 `locale`。
- `scripts/i18n/scan_sources.py`：`is_test_context` 改逐项语义（`#[cfg(test)]` 只标记所装饰项；module → 整个模块 brace 匹配；fn/impl/const → 该项），新增 `_skip_ws_and_comments`/`_item_span_end`，修 `files/state.rs` 中 `#[cfg(test)]` 方法之后再出现的生产串被误判的边界 bug（`dozer.files` 62→65）。

### 验证命令与结果

- `cargo build -p dozer-app`：通过。
- `cargo test -p dozer-app`：**2001 passed / 0 failed**。
- `cargo fmt --all -- --check`、`cargo clippy -p dozer-app --all-targets`（files 路径无新告警）、`python3 scripts/i18n/check_resources.py`（10 个 owner 目录）、`bash scripts/check-log-scope.sh`、`python3 scripts/audit/check_panel_boundary.py`：全绿。
- `python3 scripts/i18n/build_inventory.py`：`counts: {"excluded": 3095, "migrated": 0, "unresolved": 362}`；`unresolved by task: {"10": 240, "11": 122}`；`dozer.files` **0 条**。

### 残留

- Task 10 其余面板（database/project/group_chat）按面板逐个提交。
- 双语运行期视觉核对随 Task 13 执行。

## 规格 L01–L14 验收（Task 13 执行，当前未开始）
| ID | 场景 | 状态 |
|---|---|---|
| L01 | 系统语言匹配（en-US / zh-Hans-CN / zh-TW / 未知 / 空） | ⬜ 待实施 |
| L02 | 选择 zh-CN 后重启 GUI/daemon，再改 OS 语言 | ⬜ 待实施 |
| L03 | 配置缺失/损坏/写入失败/服务离线 | ⬜ 待实施 |
| L04 | 两窗口/两进程提交冲突，断线重连 | ⬜ 待实施 |
| L05 | 缺 key/空翻译/重复 key/错参数/未知 owner | ⬜ 待实施 |
| L06 | Rust/TS 共享测试向量（Unicode/花括号/恶意 HTML） | ⬜ 待实施 |
| L07 | 有编辑/滚动/终端/任务时切换，工作状态保留 | ⬜ 待实施 |
| L08 | 所有 overlay/菜单/现存 Toast | ⬜ 待实施 |
| L09 | WebView ready 前切换/隐藏后切换/重建/旧包晚到 | ⬜ 待实施 |
| L10 | Code Health/Usage 计数、日期、金额及服务失败 | ⬜ 待实施 |
| L11 | en/zh-CN + 明暗主题 + 最小窗口 + 缩放，CJK 无方块 | ⬜ 待实施 |
| L12 | 新旧 GUI/daemon/前端配对，旧第三方应用 | ⬜ 待实施 |
| L13 | MCP/日志/存储快照切换前后对比 | ⬜ 待实施 |
| L14 | 单面板 + Host 测试组合 | ⬜ 待实施 |
