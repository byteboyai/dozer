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
| 05 | 设置入口与面板标题 | 3 |
| 06 | Host chrome/菜单/原生 overlay | 0 |
| 07 | Toast 与错误呈现 | 105 |
| 08 | WebView locale 传递基础 | 8 |
| 09 | Todo / Usage / Code Health 首批面板 | 240 |
| 10 | 其余官方面板与辅助界面 | 413 |
| 11 | 编辑/预览 Web 与 HTML 模板 | 112 |
| **合计** | | **881** |

未分配的 UI 区域：**0**（生成器 `build_inventory.py` 在存在无任务候选时报警并列出）。

## 3. 未解决 UI 候选按 owner 分布

| owner | 数量 |
|---|---|
| dozer.host | 422 |
| dozer.codehealth | 153 |
| dozer.project | 122 |
| dozer.group_chat | 92 |
| dozer.preview | 91 |
| dozer.database | 69 |
| dozer.files | 62 |
| dozer.todo | 60 |
| dozer.ssh | 34 |
| dozer.git_log | 27 |
| dozer.usage | 27 |
| dozer.web | 20 |
| dozer.agent | 15 |
| dozer.conversations | 13 |

## 4. 任务进度

| Task | 状态 | 证据 | 残留 |
|---|---|---|---|
| 01 清单与生产入口基线 | ✅ 完成 | 本文件、`baseline.md`、`glossary.md`、`inventory.json`、`scripts/i18n/` | 迁移尚未开始，见 `acceptance.md` |
| 02 双语资源、解析器与资源校验 | ✅ 完成 | `dozer-model/src/i18n.rs`、`dozer-i18n/`(解析/校验/插值/向量)、`dozer-app/locales/host/`、`web/shared/i18n/`、`scripts/i18n/check_resources.py` | 标题接入 `PanelDescriptor`/设置页在 Task 04/05 |
| 03 语言持久化、服务与兼容传输 | ✅ 完成 | `dozerd/src/locale_settings.rs`(CAS/原子落盘/损坏恢复/溢出失败/无变化不推进 revision)、`dozer-model` 的 `LocaleSnapshot`/`LocaleWriteOutcome`、`dozer-services::LocaleService`、`protocol` 的 `Get/SetLocale`+`Locale/LocaleWrite`、`Client` 适配、`dozerd/tests/locale_requests.rs`、`dozer-client` 降级测试 | 系统探测(`system_candidates` 来源)、App 快照与窗口同步在 Task 04 |
| 04 App 语言快照、系统探测和窗口同步 | ✅ 完成 | `dozer-app/src/platform/locale.rs`(macOS `NSLocale.preferredLanguages` 有序探测 + 非 mac 环境变量回落)、`dozer-app/src/app/locale.rs`(`LocaleController`:epoch/序号/display generation/2s 节拍/在途合并/迟到丢弃/revision 防回退)、`dozer-app/src/locale_cache.rs`(最后确认快照缓存)、`app/{app,message,update}.rs` 接线、`platform/window_events.rs` 节拍与获焦刷新 | 文案渲染接缝(`host_resolver`/`host_text`)的**消费**在 Task 05;webview locale 注入在 Task 08 |
| 05 设置入口与面板标题纵向交付 | ✅ 完成 | `panel_registry.rs`(`PanelDescriptor{title_key,title_fallback}`+`title(locale)`)、`product.rs`(`panel.*` key+英文 fallback)、`chrome/rail.rs::panel_meta(kind,locale)`、`extensions/settings.rs`(语言页 + `LanguageStatus` 状态机 + `LanguageSelected`/`LanguageSaveOutcome`;Git/高级文案经 `host_text`)、`app/update.rs`(拦截提交→`spawn_locale_write`)、`app/app.rs`(`open_settings`/`sync_settings_locale`/`spawn_locale_write`/`apply_local_write`)、`settings_overlay.rs`(传 locale)、`i18n.rs::interpolate`、`locales/host/{en,zh-CN}.json`、`build_inventory.py` 的 `NON_UI_STRINGS` | Task 05 未解决候选 28→3：剩余 3 条是断开账号 Toast + 本地记录写失败文案,归 Task 07;4 条面板清单构造器不变量错误按非 UI 排除;其余 chrome/菜单/overlay 文案在 Task 06;webview 注入在 Task 08 |
| 06 Host chrome、菜单、原生 overlay 与项目创建 | ✅ 完成 | `chrome/{rail,topbar,tab_widget,homespace}`、`app/{view,app,update}.rs`、`workspace/view.rs`、`extensions/{settings,project_create,search,edit_history,file_history,settings_apps_view,conversations,project/view}.rs`、`platform/*_overlay.rs` 与 `window_events.rs`、`menu_spec.rs`、`i18n.rs::host_text_args`（删除 `interpolate`）、`locales/host/{en,zh-CN}.json`、`build_inventory.py` 的 `STRING_TASK_OVERRIDES`;byteui v0.4.2 放宽 label/placeholder 为 `&str` | Task 06 未解决候选清零(重投影后归 07/08/09/10/11);bytehost 自产的应用页文字与 agent 品牌名不在范围 |
| 07–08 | 未开始 | — | 依赖 06 |
| 09–11 | 未开始 | — | 依赖 07/08 |
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
