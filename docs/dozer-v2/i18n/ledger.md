# Dozer i18n 任务总账

> 日期：2026-10-12。对应计划 `2026-10-12-dozer-i18n-implementation.md`。
> 本文件跟踪任务级进度与扫描总账；逐条候选状态在 `inventory.json`。
> 总账恒等式（计划 §1.3）：**候选总数 = 已迁移 + 有理由排除 + 未解决**。

## 1. 扫描总账（Task 01 基线）

| 项 | 数量 |
|---|---|
| 候选总数 | 4208 |
| 已迁移（migrated） | 0 |
| 有理由排除（excluded） | 3001 |
| 未解决（unresolved，UI 待迁移） | 1207 |

复算：`0 + 3001 + 1207 = 4208` ✅

- 排除理由分布：测试/fixture、机器日志、开发期诊断宏、内部工具/黄金值、后端 crate 非 UI 字符串（详见 `inventory.json` 各条 `reason`）。
- 重跑方式：`python3 scripts/i18n/scan_sources.py --json /tmp/i18n_candidates.json` 后 `python3 scripts/i18n/build_inventory.py`。
- `inventory.json` 每次覆盖重写，禁止手改结构；未解决数变化须与扫描复核一致，禁止删基线记录让数字归零。

## 2. 未解决 UI 候选按任务分配

| Task | 主题 | 未解决候选 |
|---|---|---|
| 05 | 设置入口与面板标题 | 0 |
| 06 | Host chrome/菜单/原生 overlay | 0 |
| 07 | Toast 与错误呈现 | 0 |
| 08 | WebView locale 传递基础 | 8 |
| 09 | Todo / Usage / Code Health 首批面板 | 150 |
| 10 | 其余官方面板与辅助界面 | 418 |
| 11 | 编辑/预览 Web 与 HTML 模板 | 115 |
| **合计** | | **781** |

未分配的 UI 区域：**0**（生成器 `build_inventory.py` 在存在无任务候选时报警并列出）。

## 3. 未解决 UI 候选按 owner 分布

当前快照（`build_inventory.py`，共 424 条；已完成面板已从表中移除）：

| owner | 数量 |
|---|---|
| dozer.group_chat | 92 |
| dozer.project | 79 |
| dozer.preview | 79 |
| dozer.database | 68 |
| dozer.files | 62 |
| dozer.host | 37 |
| dozer.agent | 8（全在 Task 11 的 `web/review-trace/`；Task 10 的 Rust `agent_context.rs` 已清零） |
| dozer.todo | 0 |
| dozer.ssh | 0 |
| dozer.web | 0 |
| dozer.conversations | 0 |

## 4. 任务进度

| Task | 状态 | 证据 | 残留 |
|---|---|---|---|
| 01 清单与生产入口基线 | ✅ 完成 | 本文件、`baseline.md`、`glossary.md`、`inventory.json`、`scripts/i18n/` | 迁移尚未开始，见 `acceptance.md` |
| 02 双语资源、解析器与资源校验 | ✅ 完成 | `dozer-model/src/i18n.rs`、`dozer-i18n/`(解析/校验/插值/向量)、`dozer-app/locales/host/`、`web/shared/i18n/`、`scripts/i18n/check_resources.py` | 标题接入 `PanelDescriptor`/设置页在 Task 04/05 |
| 03 语言持久化、服务与兼容传输 | ✅ 完成 | `dozerd/src/locale_settings.rs`(CAS/原子落盘/损坏恢复/溢出失败/无变化不推进 revision)、`dozer-model` 的 `LocaleSnapshot`/`LocaleWriteOutcome`、`dozer-services::LocaleService`、`protocol` 的 `Get/SetLocale`+`Locale/LocaleWrite`、`Client` 适配、`dozerd/tests/locale_requests.rs`、`dozer-client` 降级测试 | 系统探测(`system_candidates` 来源)、App 快照与窗口同步在 Task 04 |
| 04 App 语言快照、系统探测和窗口同步 | ✅ 完成 | `dozer-app/src/platform/locale.rs`(macOS `NSLocale.preferredLanguages` 有序探测 + 非 mac 环境变量回落)、`dozer-app/src/app/locale.rs`(`LocaleController`:epoch/序号/display generation/2s 节拍/在途合并/迟到丢弃/revision 防回退)、`dozer-app/src/locale_cache.rs`(最后确认快照缓存)、`app/{app,message,update}.rs` 接线、`platform/window_events.rs` 节拍与获焦刷新 | 文案渲染接缝(`host_resolver`/`host_text`)的**消费**在 Task 05;webview locale 注入在 Task 08 |
| 05 设置入口与面板标题纵向交付 | ✅ 完成 | `panel_registry.rs`(`PanelDescriptor{title_key,title_fallback}`+`title(locale)`)、`product.rs`(`panel.*` key+英文 fallback)、`chrome/rail.rs::panel_meta(kind,locale)`、`extensions/settings.rs`(语言页 + `LanguageStatus` 状态机 + `LanguageSelected`/`LanguageSaveOutcome`;Git/高级文案经 `host_text`)、`app/update.rs`(拦截提交→`spawn_locale_write`)、`app/app.rs`(`open_settings`/`sync_settings_locale`/`spawn_locale_write`/`apply_local_write`)、`settings_overlay.rs`(传 locale)、`i18n.rs::interpolate`、`locales/host/{en,zh-CN}.json`、`build_inventory.py` 的 `NON_UI_STRINGS` | Task 05 未解决候选 28→3：剩余 3 条是断开账号 Toast + 本地记录写失败文案,归 Task 07;4 条面板清单构造器不变量错误按非 UI 排除;其余 chrome/菜单/overlay 文案在 Task 06;webview 注入在 Task 08 |
| 06 Host chrome、菜单、原生 overlay 与项目创建 | ✅ 完成 | `chrome/{rail,topbar,tab_widget,homespace}`、`app/{view,app,update}.rs`、`workspace/view.rs`、`extensions/{settings,project_create,search,edit_history,file_history,settings_apps_view,conversations,project/view}.rs`、`platform/*_overlay.rs` 与 `window_events.rs`、`menu_spec.rs`、`i18n.rs::host_text_args`（删除 `interpolate`）、`locales/host/{en,zh-CN}.json`、`build_inventory.py` 的 `STRING_TASK_OVERRIDES`;byteui v0.4.2 放宽 label/placeholder 为 `&str` | Task 06 未解决候选清零(重投影后归 07/08/09/10/11);bytehost 自产的应用页文字与 agent 品牌名不在范围 |
| 07 Toast 与错误呈现迁移 | ✅ 完成 | `extensions/toast.rs`(`Toast.text`/`Pending.text: LocalizedText`、`push`/`log_toast` 收 `impl Into<LocalizedText>`、`failure_message(scope,key,fallback,res)`、`stable_value` 空判定/去重)、`platform/toast_overlay.rs`(渲染后 `normalize`)、`i18n.rs`(`host_render`/`host_render_stable`/`host_keyed`/`host_keyed_args`)、`dozer-model/src/i18n.rs`(`DEFAULT_OWNER`+`From<&str>`/`From<String>`)、结构化错误(`GitAccountError`/`ProjectFsError`/`RepoListState::Error(LocalizedText)`/`WorkspaceState::{preview_error,summary_errors}`)、`workspace/hook.rs::agent_state_label(state,locale)`、`locales/host/{en,zh-CN}.json`、`build_inventory.py`(NON_UI/OVERRIDE) | Task 07 未解决候选清零;预览组(`web_error`/`PreviewError.message`/T10 冲突/资源预算 6 条)与 SSH 连接错误 4 条按任务重投影到 Task 11/10 |
| 08 WebView locale 传递与资源打包基础 | ✅ 完成 | `dozer-model/src/i18n.rs`(`LocaleStamp{ui_locale,locale_revision}`+泛型载体 `Stamped<T>`)、`extensions/{todo,codehealth,group_chat}/protocol.rs` 与 `dozer-panel-usage/src/protocol.rs` envelope 追加 `#[serde(flatten)] Option<LocaleStamp>`(Usage 同步补 `revision`)、`extensions/codehealth/surface.rs`/`dozer-panel-usage/src/surface.rs` 的 `SurfaceExecutor<Stamped<ViewPayload>>`(locale 参与逐帧去重)、todo/group_chat `WebviewPushState.last_sent: Option<Stamped<..>>`、`app/app.rs::locale_stamp()` + 四处推送点传当前 `active_locale()`/`display_generation()`、前端 `web/{todo,group-chat,codehealth}-content/src/{protocol.ts,main.tsx}`(PushState 带 `locale`/`localeRevision`、`document.documentElement.lang`)与 `dozer-panel-usage/web/usage-content/src/main.tsx`(响应式 locale 信号 + html lang)、各 `protocol.test.ts`/`lib_tests.rs` 新增"业务 payload 不变、仅语言变也推送"用例、`build_inventory.py` 的 `NON_UI_STRINGS`(preview IPC 校验 `Display` 8 条) | Task 08 未解决候选清零。**本任务范围内仅做 envelope + 响应式 locale 信号 + html lang;字符串抽取(09–11)、共享解析器接线、`locales/` 目录与 build.mjs 资源打包按用户裁决延后**,见本表 §2 与 `acceptance.md` Task 08 说明 |
| 09 (Todo) | ✅ 完成（按面板一提交） | `crates/dozer-app/locales/panels/todo/{en,zh-CN}.json`(owner `dozer.todo`)、Rust：`extensions/todo/{view,update,protocol}.rs`、`platform/confirm_overlay.rs`、`platform/todo_detail_overlay.rs`、`app/update.rs`(`set_failed` 收 `LocalizedText`)、`i18n.rs`(`PANEL_OWNERS`+`panel_text`/`panel_text_args`+`build_host_resolver` 载入面板 owner)、`panel_host.rs`(`PanelHost::locale()`)、Web：`web/todo-content/src/{i18n.ts(new),statusMeta.ts,components/{App,AddBox,Popovers,TodoCard,Toolbar}.tsx,main.tsx}`(静态打包面板 JSON、`setLocale` 随 envelope 注入)、`i18n.test.ts`/`statusMeta.test.ts`、宿主通用文案 `webview.load_timeout` 落 host 资源、`build_inventory.py`(`NON_UI_STRINGS` 增 `新分类`) | Task 09 内 todo 项候选清零;`新分类` 按"保留用户分类名"排除;**共享 `web/shared/i18n` 解析器接线 + 资源打包**在本面板落地(兑现 Task 08 延后项) |
| 09 (Usage) | ✅ 完成（按面板一提交） | 独立 crate 自带资源：`crates/dozer-panel-usage/locales/{en,zh-CN}.json`(owner `dozer.usage`)、`src/i18n.rs`(new，`dozer-i18n` 中性解析器 + 自有 JSON `include_str!` + `t`/`t_static`/`resolve` + en/zh key-parity 测试)、`src/{adapter,state,view,lib}.rs`(`UsageNotice.text`→`LocalizedText`、`UsageOutbox::push` 收 `LocalizedText`、新增 `PanelHeadKind{Content,AgentFilter}` 令 `panel_head` 按 kind 选图标而非标题字面、`state.rs` 加载失败走 `error.load_failed` key)、调用方 `app/view.rs`(`app.locale.active_locale()`)、`panel_host.rs`/`usage_probe/{scenarios,fake_panel_host}.rs`、`host_adapters/usage_panel.rs`(&kind 选图标 + `notices_to_pending` 直传 `LocalizedText`)、Web：`web/usage-content/src/{i18n.ts(new),main.tsx,theme.ts,components/{App,EmptyStates,ProjectSummaryBoxes,TrendSection,TrendLineChart}.tsx}`(共享 `web/shared/i18n` 相对 import + 静态 JSON 打包、`SERIES_META.label`→`labelKey`)、`i18n.test.ts`/`theme.test.ts` 双语用例 | Task 09 内 usage 项候选清零;**独立 Usage 库自行携带资源、locale 由 Host 注入**,单面板 FakePanelHost 测试宿主不依赖 dozer-app/daemon 即可渲染(计划 §Task 09 第 4/6 条) |
| 09 (Code Health) | ✅ 完成（按面板一提交） | `crates/dozer-app/locales/panels/codehealth/{en,zh-CN}.json`(owner `dozer.code_health`，含 `analyze.*`、`error.*`)、Rust：`extensions/codehealth/{mod,view,view_model,protocol,arch_payload,aggregate}.rs`(分类/严重度/变化/tier 经 key；Rust 侧旧 `*_label`/`*_text` 兼容字段统一改**英文**；`analyze_finding_text` 改为 locale-aware；`Message::Scanned(Err)` 与 save 错误改带稳定 key 的 `ErrorDto`(spec §10) 并保留英文兜底文案)、`app/{update,view}.rs`(`analyze` 调用点取 `self.locale.active_locale()`)、`i18n.rs`(`PANEL_OWNERS` 增 `dozer.code_health`)、Web：`web/codehealth-content/src/{i18n.ts(new),types.ts,main.tsx,fixtures.ts,render-smoke.tsx,protocol.test.ts,components/{App,ScanHeader,EmptyState,FindingRow,OverviewPage,StructurePage,UiPage,ScopePage,ArchitecturePage}.tsx}`(静态打包面板 JSON、`setLocale` 随 envelope 注入、`ErrorDto`→`errorText`) | Task 09 内 code health 项候选清零;`build_inventory.py` 该面板路径 0 unresolved |
| 10 (git_log 面板) | ✅ 完成（按面板一提交） | `crates/dozer-app/locales/panels/git_log/{en,zh-CN}.json`(owner `dozer.git_log`，含 `search.*`/`filter.*`/`empty.*`/`branch.*`/`error.*`)、Rust：`extensions/git_log.rs`(`GIT_LOG_OWNER` + 本地 `t()`/`loading_hint_localized()`；`State.{error,detail,diff_load_error}` 改存 `LocalizedText`，`Message::{DetailLoaded,SnapshotLoaded,DiffContentLoaded,BranchSwitchDone}` 错误改 `LocalizedText`，后台任务用 `i18n::panel_keyed_args` 造 keyed 文案、渲染端 `render_localized`；`view`/`file_list_view`/`diff_pane_view`/`git_panel_footer_bar`/`branch_picker_view` 穿 `locale`)、`extensions/diff_content.rs`(`NotRenderable.reason` 由 `String` 改 `LocalizedText`——blob 键 `dozer.git_log.empty.diff_unrenderable_blob`、workdir 键 `dozer.host.file_history.unrenderable_workdir`)、`i18n.rs`(`PANEL_OWNERS` 增 `dozer.git_log`；新增 `panel_keyed_args`/`render_localized`)、`app/update.rs`(分支切换错误 keyed) | Task 10 内 git_log 路径候选清零;`diff_content.rs` 的两条 host 文案一并迁走(host 候选清零) |
| 10 (ssh 面板) | ✅ 完成（按面板一提交） | `crates/dozer-app/locales/panels/ssh/{en,zh-CN}.json`(owner `dozer.ssh`，含 `head`/`empty`/`card`/`footer`/`form`/`status`/`confirm`/`error`/`sftp`)、Rust：`extensions/ssh.rs`(`SSH_OWNER` + 本地 `t()`；`ssh_error_localized`/`terminal_error_{unknown_host,key_changed,timeout,channel}`/`os_probe_timeout` 供后台任务造 keyed 文案；`TestStatus::Err` 与 `Message::{TestConnectionResult,TerminalConnectFailed}` 错误改 `LocalizedText`、`FetchOsResult` 错误改 `LocalizedText`；`host_card`/`ssh_host_card`/`radio_dot`/`text_btn`/`tree_column` 收 `String` 标签并穿 `locale`；`view`/`ssh_footer_bar`/`delete_confirm_spec` 穿 `locale`；`persist_hosts` 失败 Toast 改 keyed)、`extensions/ssh/sftp.rs`(`SftpTabState.status` 改 `LocalizedText`、树标题/上传下载菜单/连接失败/传输状态全 keyed；`sftp_pane_view`/`sftp_context_menu`/`context_menu_items`/`tree_column` 收 `locale`)、`app/{view,update}.rs`(`sftp_pane_view`/`context_menu_items` 传 `active_locale()`；`ssh_test_connection_result`/`ssh_terminal_connect_failed` 收 `LocalizedText`)、`workspace/state.rs`(终端建连各错误点改调 `ssh::terminal_error_*`/`ssh_error_localized`，移除裸中文)、`platform/{ssh_host_overlay,confirm_overlay}.rs`(穿 `locale`)、`app/update.rs` 顶层 `ssh_host_card` 键构造)、`scripts/i18n/scan_sources.py`(`strip_rust_comments` 修字符字面量 `'"'` 吞字符串的扫描 bug，使 ssh.rs 候选由 25 恢复到 40) | Task 10 内 ssh 面板候选清零(ssh.rs + sftp.rs);`dozer.host` 同批下降 41→37 |
| 10 (web/browser 面板) | ✅ 完成（按面板一提交） | `crates/dozer-app/locales/panels/web/{en,zh-CN}.json`(owner `dozer.web`，含 `nav.*`/`addr.placeholder`/`star.tooltip`/`bookmarks.tooltip`/`sidebar.*`/`tab.new`/`empty.no_page`/`error.*`)、Rust：`extensions/browser.rs`(`WEB_OWNER` + 本地 `t()`；`Tabs::addr_submit` 返回 `Err(LocalizedText)`(`error.local_path`)；`State.error` 由 `Option<String>` 改 `Option<LocalizedText>`，收藏夹读/写失败经 `panel_keyed_args`(`error.bookmarks_load_failed`/`error.bookmarks_op_failed`)；`view`/`nav_button`/`star_button`/`bookmarks_toggle_button`/`bookmarks_panel`/`bookmark_group` 穿 `locale`，`bookmark_group` 标题改收 `String`；导航/星标/收藏夹/新标签页 tooltip、地址占位、侧栏分组/空态、空页面、错误行经 `t`/`render_localized`)、`app/view.rs` 与 `chrome/homespace.rs`(`browser::view` 传 `app.locale.active_locale()`)、`i18n.rs`(`PANEL_OWNERS` 增 `dozer.web`)、`scripts/i18n/scan_sources.py`(`is_test_context` 修 `#[cfg(test)] mod tests` 之后再出现 `#[cfg(test)]` 项导致测试模块字面量被误判的边界 bug) | Task 10 内 browser 路径候选清零;测试夹具字面量随扫描器修正一并归为 excluded，`dozer.database` 同批下降 69→68 |
| 10 (conversations 面板) | ✅ 完成（按面板一提交） | `crates/dozer-app/locales/panels/conversations/{en,zh-CN}.json`(owner `dozer.conversations`，含 `head.title`/`search.placeholder`/`loading`/`empty.*`/`filter.all`/`row.*`/`more`/`error.list_load_failed`)、Rust：`extensions/conversations.rs`(`CONVERSATIONS_OWNER` + 本地 `t()`/`t_args()`/`loading_hint_localized()`——`byteui` 的 `loading_hint` label 是 `&'static str` 故复刻；列表读失败 Toast 改 `panel_keyed_args` keyed(`error.list_load_failed`)；`footer_bar`/`agent_picker_items`/`agent_picker_view` 收 `locale`("全部"→key)；`view` 的标题/搜索占位/加载/空态/行副标题(`row.current`/`row.normal`/`row.task_suffix*` 经 `t_args` 插值)/"更多" tooltip 全 keyed)、`app/update.rs`(`agent_picker_items` 传 `self.locale.active_locale()`；`usage_probe/host.rs` 引用行号随迁 5421→5426、1646→1651)、`i18n.rs`(`PANEL_OWNERS` 增 `dozer.conversations`) | Task 10 内 conversations 面板路径候选清零 |
| 10 (agent 面板 Rust) | ✅ 完成（按面板一提交） | `crates/dozer-app/locales/panels/agent/{en,zh-CN}.json`(owner `dozer.agent`，含 `strip.label`/`action.history`/`action.remove`/`entry.missing`/`error.*`)、Rust：`extensions/agent_context.rs`(`AGENT_OWNER` + 本地 `t()`/`t_args()`；三条 outbox Toast(加载失败/移除失败/落库失败)改 `i18n::panel_keyed_args` keyed(`error.list_load_failed`/`error.remove_failed`/`error.add_record_failed`)；`strip`/`expanded_panel` 收 `locale`(条标题计数、修改历史、逐项历史/移除/缺失标记全 keyed，`expanded_panel` 加显式 `<'a>` 绑 `state: &'a State`))、`term/terminal.rs`(`strip`/`expanded_panel` 传 `app.locale.active_locale()`)、`i18n.rs`(`PANEL_OWNERS` 增 `dozer.agent`) | 裁决(2026-10-14)：`web/review-trace/` 归 **Task 11**(owner 仍 `dozer.agent`)，`build_inventory.py` 任务号 10→11；本面板 Task 10 Rust 路径清零 |
| 10–11 (其余) | 未开始 | — | 依赖 07/08 |
| 12–14 | 未开始 | — | 见计划 §2 |

## 5. 门禁与工具

| 工具 | 路径 | 用途 |
|---|---|---|
| 候选扫描 | `scripts/i18n/scan_sources.py` | 提取 CJK 候选并分类（Task 01） |
| 清单生成 | `scripts/i18n/build_inventory.py` | 生成 `inventory.json`、校验任务分配（Task 01） |
| 面板边界 | `scripts/audit/check_panel_boundary.py` | 既有；面板不得新增 client 依赖 |
| 资源校验 | `scripts/i18n/check_resources.py` | key/占位符/空值/重复 key 对齐（Task 02） |
| 迁移门禁 | `scripts/check-i18n.sh` | 待 Task 12 新增 |

## 6. 完成定义提醒

- 单有 locale 设置、翻译函数或两份 JSON 文件，均不算完成多语言迁移（计划 §5）。
- 每条存量用户可见中文须有可追踪结论：迁入 `zh-CN.json` 的 key，或有明确非 UI 理由的保留项。
- 迁移完成 = 资源存在 + 代码调用已替换 + 中文显示经核对 + 英文翻译存在，四者同时成立。
