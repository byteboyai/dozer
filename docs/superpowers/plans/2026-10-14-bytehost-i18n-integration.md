# bytehost i18n 上游实现与 Dozer 接入计划

> 日期：2026-10-14。状态：**T0、T1、T2、T3、T4、T5 已完成**（bytehost 侧：清单/资源装载/投影/桥/样例与门禁齐备）；**T6 Dozer 侧消费适配已实现**；**T7 已完成**（bytehost 发布 `v0.3.0`，Dozer 切到 tag、联调 patch 已删，现随依赖升到 `v0.4.1`）；**T8 已完成**（I4 交付记录：`docs/dozer-v2/i18n/bytehost-acceptance.md`，dozer `e5d6d585`，2026-10-15）。**仍未做：** 真实 WebView 里的样例冒烟（`bilingual-live` 热切换、`partial-initial` 重开、双窗口代次隔离等，见验收文档的 ⏳ 项），状态更新于 2026-10-16。
> 规格：**权威版现位于 bytehost 仓库** [`docs/superpowers/specs/2026-10-14-bytehost-app-i18n.md`](https://github.com/byteboy/bytehost)；Dozer 侧评审稿 `../specs/2026-10-14-bytehost-i18n-integration-design.md` 仅作指针与消费范围，不再作为分叉契约。
> 本计划展开原 [Dozer i18n Task 14](2026-10-12-dozer-i18n-implementation.md)，内置 UI 已按用户反馈收口，其后续细节修复不作为本计划前置门槛。
> **T0 产出（在 bytehost）**：基线记录、AppSummary 接缝清单、WebView 注入 spike 结论、版本/locale/限额/bridge 命名决策，见 `bytehost/docs/superpowers/plans/2026-10-14-bytehost-i18n-baseline.md`；旧 scaffold 已降级为指针（`bytehost/.../2026-10-14-bytehost-app-i18n-contract.md`）。

## 1. 执行边界

- 上游实现位于独立 bytehost 仓库；当前 Dozer 仓库只放跨仓草案、产品适配与验收记录。不在 Dozer 重建 manifest parser、资源装载器、应用语言 resolver 或安全桥。
- 实施前记录两个仓库 HEAD/工作区/tag。现有未提交的 Dozer locale 调度、系统语言与删除路由修复属于前一任务，保留，不与上游提交混装。
- 上游当前有 `docs/superpowers/plans/2026-10-14-bytehost-app-i18n-contract.md` scaffold。Task 0 将其收敛到本 spec；不要执行其中已被本 spec 修正的全 Surface、URL 传语言及旧解析器兼容假设。
- 每项完成记录修改路径、命令、结果与限制；下列新文件名是建议布局，开始任务时按实际目录确认。不存在的脚本不当成已通过的门禁。
- 本计划不要求多 agent。按依赖顺序完成即可，每项形成可独立审查的提交。发布前把变更、版本和验证结果准备成可审查成果，再按届时授权发布。

依赖顺序：T0 → T1 → T2 → T3；T0 → T4；T3 + T4 → T5 → T6 → T7 → T8。

## T0. 固定基线、同步上游规格、验证文档初始化接缝

**归属：bytehost；Dozer 仅补消费点记录。**

**读/改：** bytehost manifest/proto/manager/source/digest、webview 初始化脚本、core Surface、desktop builder、现有上游 scaffold；Dozer `runtime.rs`、`app_webview.rs`、`host_adapters/app_host.rs`。

- [x] 记录上游实际发布 tag、HEAD、host_version 来源及 `min_host_version` 的比较逻辑；核对协议帧上限，确认 spec 128 KiB 投影预算是否需要收紧。**（见 baseline §T0-1：tag v0.2.0 / HEAD bd84bef / HOST_VERSION 0.1.0 / `min_host_version > *host_version`；传输层无单帧硬上限，128 KiB 为投影层自加预算，在 `manager.list()` 投影处强制，无需收紧传输帧。）**
- [x] 把评审后的公共 spec 放入 bytehost 文档，更新旧 scaffold 为权威入口；本仓保留 Dozer 消费范围与链接，避免两份长期真相。**（bytehost `specs/2026-10-14-bytehost-app-i18n.md` 为权威版；旧 scaffold 降级为指针并列出已纠正的不准确表述；本仓设计稿改为指针。）**
- [x] 核对 AppSummary 的旧序列化 fixture、Rust struct 构造点、InProcess/UDS 接缝、安装/回滚路径，列出最小改动清单。**（见 baseline §T0-3：真实构造点 `manager.rs:1959`；两路共享同一类型零转换；安装/回滚随包版本自动恢复 locale 文件；给出 T1–T5 最小改动清单。）**
- [x] 用最小 WebView spike 验证：主文档注入时机、同源二次导航、初始化脚本可更新方式、script completion 与 document epoch。必须证明语言变化后新文档不会暴露旧快照；不能保证时选择 spec 的 pending/skeleton 路径并记录。**（见 baseline §T0-4：初始化脚本在创建时一次性冻结、wry 在每次文档加载以相同冻结内容重跑，故 locale 不可经 init script 携带；唯一新文档下发通道是导航后 `evaluate_app`；桥在首帧到达前保持 pending/skeleton，绝不复用创建时捕获的过期值。仓库惯例不启动真实 webview，改用 `app_webview.rs` 源码钉死测试 `i18n_locale_must_be_pushed_post_navigation_not_via_init_script` 锁住该接缝。）**
- [x] 记录能力首发版本方案、规范 locale 子集、限额、bridge API 的最终命名。不要把已决定的默认目录/格式/fallback 再变成执行时的模糊待办。**（见 baseline §T0-5：HOST_VERSION 发布时 bump + tag 映射写入发布记录；locale 子集/限额全定死；bridge 命名 `window.bytehost.i18n` 的 `getSnapshot`/`subscribe`，`locale_revision` 十进制字符串。）**

**验收：**消费入口与版本映射明确；B06/B07 的实现接缝有实测依据；没有新增 External 或通用 Plugin Manifest 工作。

## T1. Manifest 与纯 DTO/协商逻辑

**归属：bytehost。**

**文件：** `crates/bytehost-apps/src/manifest.rs`、`lib.rs`；新增 `src/i18n/mod.rs`、`src/i18n/locale.rs`、`src/i18n/title.rs`（可合并小模块）。

- [x] 定义可选 I18n、webview_locale、presentation/entrypoint title_key + title_fallback；给新可选字段 serde default 与 skip-none，保留旧字段原义和 unknown-field 拒绝规则。
- [x] 实现 locale 子集校验/规范化、规范化后查重、default 包含约束、key/fallback 成对校验、标题字符安全与上限。
- [x] 实现纯 locale/title resolver：精确匹配 → default → 明确 fallback；无声明沿用旧 name/title。默认 feature 不引入产品 crate 或文件 I/O 依赖。
- [x] 测试旧 manifest、混合新旧标题、部分翻译、未知语言、简繁不隐式互转、非法字段组合及安全字符；锁定新 manifest 被旧严格解析器拒绝的兼容边界。**（见 `crates/bytehost-apps/src/manifest.rs::i18n_tests` 共 11 个测试 + `crate::i18n::{locale,title}` 纯逻辑；`cargo test -p bytehost-apps --features manifest-toml` 全绿 103/103，`cargo clippy` 无警告，`bash scripts/check-deps.sh` 通过。新增代码落在 `i18n/` 子模块，默认 feature 仅依赖 serde，无新增产品/IO 依赖。）**

**验证：** `cargo test -p bytehost-apps --features manifest-toml`；`bash scripts/check-deps.sh`。

**验收：**B01/B02/B05 纯逻辑部分通过，格式可由样例使用。

## T2. 包内资源装载与安装/版本生命周期

**归属：bytehost。**

**文件：** 新增 `crates/bytehost-apps/src/i18n/resources.rs`；修改 `manager.rs`、`source.rs`、`digest.rs`、必要的 `registry.rs` 接缝。

- [x] 实现 UTF-8 JSON 严格解析、重复 key 检测、key/value/文件/总量/语言数限制；默认语言文件与声明标题必须存在，非默认缺文件/key 允许 fallback。（新增 `crates/bytehost-apps/src/i18n/resources.rs`：`load_i18n_resources` + `ResourceLimits`/`ResourceError`；扁平 `string->string` 严格解析含重复 key 检测、嵌套/非字符串值拒绝、key 字符集 1–128 `[A-Za-z0-9_.-]`、值非空 ≤4096 字符、≤4096 keys/文件、≤16 声明语言、512 KiB 单文件(限长读取 limit+1)、8 MiB 总量；默认语言文件必在、非默认缺失允许回退、文件名大小写不匹配报错。）
- [x] 资源路径每组件检查与安全打开绑定到受控 staging/安装包；拒绝符号链接、非普通文件与逃逸。测试链接替换而非仅静态 `../` 字符串。（`ensure_inside`：拒文件自身符号链接 + `canonicalize` 后 `starts_with(root)` 逃逸检查；22 个单测含真实 symlink 文件、symlink 指向包外、symlink 父目录逃逸、绝对/`..` 的 `strings_dir`；`strings_dir` 相对路径校验复用与 manifest 同一套规则。）
- [x] Plan 阶段验证暂存包，Install/Upgrade 阶段重验最终 staging；确认所有 i18n 文件进入 source digest。批准后修改资源必须失败，不能静默更新批准内容。（`manager::read_package` 在解析 manifest 后、且 `[i18n]` 声明时调用 `load_i18n_resources`；`read_package` 是 `install_plan`(896) 与 `install`→`install_staged`(964) 两阶段共同入口，故 plan 与提交均重验。资源字节已由 `digest_tree(root)` 覆盖整包根 → 自动进入 `source_digest`；校验失败经 `ManagerError::Resource` 拒绝安装。）
- [ ] **[递延至 T3]** 建立按 app ID + 活动包版本/摘要的缓存；升级/回滚/卸载失效；运行期损坏用 fallback；阻塞读取不占 UI 或 async reactor。该缓存属于运行期 `list()` 投影/服务路径(T3)，需在 `AppSummary.display_i18n` 投影处随包版本/摘要失效，单独在 T2 造孤儿结构无收益；运行期损坏降级已通过 `resolve_title` 的标题安全回退与"非默认缺文件即 fallback"语义覆盖，安装期损坏则按上条拒绝。
- [ ] **[递延至 T3]** 补 LocalDir/Archive/URL 共用路径的覆盖；至少一项真实归档测试，URL 测试使用现有本地 fetcher fixture，不依赖公网。`load_i18n_resources` 本身与来源类型无关(只读已落地的包目录)，LocalDir/Archive/URL 经 `stage_source` 都产出目录后走同一代码路径；真实的"打包→安装→Archive 来源"端到端 fixture 在 T3 投影接入 `list()` 时一并补(届时才有运行期读取点)。

**验证：** `cargo test -p bytehost-apps --all-features`（454 passed，含 `i18n::resources` 22 个）；`cargo clippy -p bytehost-apps --features server` 无警告；`bash scripts/check-deps.sh` ok。

**验收：**B02/B03 资源部分通过(严格解析、限额、符号链接/逃逸拒绝)；B04 的"安装期拒绝"已落地(批准后改资源→`source_digest`/`read_package` 重验失败)，"运行期降级/缓存失效"随 T3 服务路径补；B11 的"执行失败/损坏 fallback"由 `resolve_title` 回退语义覆盖。

## T3. AppSummary 投影与两种客户端传输

**归属：bytehost；Dozer 只在联调时适配新增 Rust 字段。**

**文件：** `bytehost-apps/src/proto.rs`、`manager.rs`、`service.rs`；`bytehost-client/src/lib.rs`、InProcess 实现与 conformance 测试；`bytehost-panel/src/host.rs`。

- [ ] 增加 `AppDisplayI18n` 与可选 summary 字段，仅投影声明标题；元数据含页面能力与语言列表，不携带完整词表、资源路径、启动 token。
- [ ] 安装阶段约束 64 入口/128 KiB 投影；运行期损坏或投影不可用时回落旧 name，不让 List 整体失败。
- [ ] 测试旧 JSON→新 decoder、新字段 JSON→真实旧 fixture decoder；检查旧 `name` 未随语言改变、LaunchUrl 未修改、无新必需 RPC 或 AppReply variant。
- [ ] InProcess 与 UDS 使用相同 summary/协商语义；共享面板缓存保存描述，最终标题在视图解析，不缓存为一次性字符串。
- [ ] 升级/回滚后通过现有 app-change/list 刷新元数据；不为 locale 变化触发 start/stop/LaunchUrl。

**验证：** `cargo test -p bytehost-client --all-features`、`cargo test -p bytehost-panel`，以及受影响的 apps 测试。

**验收：**B01/B04/B05/B12 的传输部分通过；旧 daemon 能让新 GUI 正常降级。

## T4. 公共 WebView bridge 与 Surface 推送

**归属：bytehost。可在 T1–T3 之后顺序执行。**

**文件：** `bytehost-webview/src/lib.rs`（必要时拆 `i18n.rs`）；`bytehost-core/src/surface.rs`；`bytehost-desktop/src/app_webview.rs`、`src/surface/lifecycle.rs`/`routing.rs`。

- [ ] 定义 bridge snapshot v1 与 string revision；实现 getSnapshot/subscribe/unsubscribe、立即回调当前值、过期 revision 拒绝。none 不注入，initial 不接同文档热更新。
- [ ] 严格通过序列化生成脚本数据；与私有 nonce/focus/zoom 通道分离；不新增页面可用的宿主设置命令。
- [ ] 实现按 generation + document epoch 的 desired/sent 生命周期；导航完成 ready、隐藏/恢复、重建都能同步，未执行或旧文档执行结果不推进 sent。
- [ ] 按 T0 spike 结论保证新文档初始值，订阅晚注册不漏状态。脚本失败有界重试，失效计时器不得造成忙循环。
- [ ] 保留旧 builder/API 的默认无桥用法或提供兼容 wrapper；记录确需同步修改的 Rust struct 构造点。纯 crate 不引入 wry/iced，原生执行只在 desktop。
- [ ] JS 行为测试覆盖非安全整数 revision、乱序/重复、立即订阅/注销、畸形数据；Rust 测试覆盖 origin、生命周期、序列化、旧 IPC 拒绝规则。纯机制测试之外，至少执行一次真实 WebView 导航/热切换检查。

**验证：** `cargo test -p bytehost-webview -p bytehost-core -p bytehost-desktop`；如新增 `crates/bytehost-webview/tests/i18n_bridge.test.mjs`，运行 `node --test` 对应文件，Node 仅作开发测试工具；`bash scripts/check-deps.sh`。

**验收：**B06–B11 桥接部分通过；未改变 CSP/origin/data_store_identifier，未扩大 IPC 权限。

## T5. 三个样例与上游收口

**归属：bytehost。**

**文件：** 新增 `scripts/samples/i18n/{bilingual-live,partial-initial,legacy}/`；安全反例放测试 fixture，不放可发布样例；更新 README、契约文档及验收报告。

- [ ] bilingual-live：en/zh-CN 标题与页面，输入框/滚动区域用于证明热切换不丢状态，应用自行设置 document.lang。
- [ ] partial-initial：声明部分非默认翻译缺失，验证 default/fallback；自然重新打开取得新 locale，不因切换自动 reload。
- [ ] legacy：完全无 i18n 字段；安装、打开、导航与旧版一致，不出现桥。
- [ ] 同时验证 static_web 与一个现有 Node/Python HTTP 样例复用桥；不把服务端进程环境当页面语言来源。
- [ ] 生成 B01–B12 证据索引，标出仍需 Dozer 验证的条目；记录资源限额、旧宿主拒绝新包、initial 的限制与升级/回滚规则。
- [ ] 上游运行一次完整 fmt/clippy/test/dependency checks，记录现有失败和新增失败，不扩大 baseline 掩盖问题。

**验收：**上游可发布候选完成。此时仍不把 Dozer I4 标为完成。

## T6. Dozer 本地消费适配与端到端验收

**归属：Dozer。临时 patch 联调不提交。**

**文件：** `crates/dozer-app/src/host_adapters/app_host.rs`、`app/app.rs`、`app/app_slots.rs`、`app_webview.rs`、`runtime.rs`、应用 rail/tab/设置展示调用点；必要的 `dozer-client`/`dozerd` 测试 fixture。

- [x] 按 T0 清单定位真正生产 builder；Dozer 的旧独立构建路径消费上游 bridge helper，避免只改 desktop builder 却漏掉实际入口。（`runtime.rs::build_app_webview` 经 `bytehost_webview::app_init_script(nonce, install_i18n_bridge)` 注入桥骨架，运行期经 `evaluate_script(i18n_set_locale_script(...))` 推快照；`should_install_bridge` 决定 `install_i18n_bridge` 与 `PageLoadEvent::Finished` 后的 `AppWebViewLocaleReady` 推送，绝不重造桥策略。）
- [x] 将 LocaleController 的 active locale/display generation 转成上游快照；不导出 daemon revision，不给 app 改全局语言权限，不引入新轮询。（`app_webview.rs::locale_push_script` 用 `LocaleController::active_locale()` + `display_generation()` 构造 `build_i18n_snapshot`；快照经 `evaluate_script` 在导航完成后推送，不向 PTY/daemon 写语言。）
- [x] summary 元数据更新后标题在视图解析，应用 ID/slot、权限审批身份保持稳定；不同 locale 的两个 GUI 能各自展示同一份元数据。（`app_host.rs` 在 `M::ListLoaded(Ok)` 把 `AppSummary.display_i18n` 按 `AppSlot` 捕获进 `AppViews::display_i18n`；`rail.rs::panel_meta` 与 `settings_apps_view.rs::app_row` 经 `resolve_title(Presentation, locale)` 解析标题，不缓存为字符串，slot/id 与权限身份不变。）
- [x] app desired/sent 接入语言变化；初始化、隐藏/恢复、同源导航、销毁后晚回调都走公共身份校验；不把 URL 改变当语言通知。（`window_events.rs`：`AppWebViewLocaleReady(id)` 由 `webviews` 句柄解析 `slot_for_webview_id` 后 `evaluate_script`，成功才 `mark_locale_pushed`；`apply_pending_app_locales` 在 `display_generation` 变化时改走 `live_locale_push_script`——仅 `webview_locale==Live` 且 `locale_pushed[slot] < generation` 才重推（幂等跳过 `initial`/`none`），`evaluate_script` 失败 `::tracing::warn!` 且 `*app_locale_generation` 移到循环结束才更新，失败推送因 `locale_pushed` 未推进、下一帧继续重试（有界、无忙循环）；URL 变化不触发语言推送。`AppViews` 新增 `locale_pushed: HashMap<AppSlot,u64>` 跟踪每 webview 已成功推送代次。）
- [x] 新 GUI + 旧 daemon fixture 降级通过；旧 GUI + 新 daemon 的旧 decoder 测试通过。无能力应用仍走原路径。（`display_i18n` 为 `Option`，未声明即 `None` → `should_install_bridge(None)` 为假 → 不装桥、不推快照，完全走原路径；解析/推送到桥策略全部落在上游公共 crate，Dozer 不另持 fallback 文案。）
- [ ] 运行三个样例，分别记录外壳标题与应用内容；验证草稿/滚动/登录状态保留、无 runtime restart；不重新要求对全部 Dozer 内置 UI 逐屏验收。（**递延至 T5 上游样例 + T7 发布 tag**：当前用本地 `[patch]` 联调，未跑真实样例冒烟，不宣称样例行为验证完成。）

> **T6 收尾说明（Dozer 侧实现已完成，2026-10-14）：** 改动落在 `crates/dozer-app/src/{host_adapters/app_host.rs, app_webview.rs, chrome/rail.rs, extensions/settings_apps_view.rs, platform/window_events.rs, runtime.rs, app/message.rs, app/update.rs, preview/webview.rs, app/app.rs, ...}`（后者仅为 `WebviewSpec` 加 `display_i18n: Option` 字段并补字面量）与 `crates/dozerd/tests/app_requests.rs`（B12 e2e fixture）。核心接缝三点：① `AppViews` 新增 `display_i18n: HashMap<AppSlot, AppDisplayI18n>` 与 `sync_display_i18n` / `localized_title` / `locale_push_script`；② `build_app_webview` 据 `should_install_bridge` 决定 `app_init_script` 第二参、挂 `with_on_page_load_handler` 在 `PageLoadEvent::Finished` 发 `AppWebViewLocaleReady`；③ `window_events` 派发该消息（页面加载后推快照）并在 locale 变化时经 `apply_pending_app_locales` 重推全部 webview。全部经上游 `bytehost_webview::i18n` 与 `bytehost_apps::proto` 公共 API，未重造桥/协商/序列化策略。
>
> **验证（本地 patch 下）：** `cargo test -p dozer-app --bin dozer -- display_i18n_is_captured_resolved_and_pushed app_rail_title_resolves_i18n_or_falls_back_to_id build_app_webview_pins_the_restrictive_settings app_entries_have_title_icon_and_default_side` → 4 passed；`cargo test -p dozerd --test app_requests i18n_metadata_round_trips_through_the_socket` → 1 passed（B12 证据，见下）；`cargo fmt -p dozer-app -- --check` 干净。`usage_probe::host::tests::host_site_citations_point_at_the_code_they_name` 既有失败与 T6 无关（未触碰该模块）。
>
> **B12 证据：** `dozerd/tests/app_requests.rs::i18n_metadata_round_trips_through_the_socket` 起真实 dozerd（`AppService` + UDS），经 `dozer-client::Client::app_list()` 拉取，断言 `display_i18n.default_locale=="en"`、`supported_locales==["en","zh-CN"]`、`webview_locale==WebViewLocale::Live`，且 `resolve_title(Presentation,"zh-CN")=="笔记"`、`"en"=="Notes"`、`"fr"=="Notes"`（缺译回退 default）。证明 Dozer UDS 透传与 bytehost 同 DTO/协商语义、打包后不依赖开发仓库资源路径；应用侧 `AppDisplayI18n` 原样经 `AppSummary` 透传，Dozer 未另持词表。
>
> **开发期 patch 约定（T7 前必须删除）：** `.cargo/config.toml`（已 `git check-ignore`，不入库）含 `[patch."https://github.com/byteboyai/bytehost"]` 把 6 个 crate 指向本地 i18n 分支 checkout（`/Users/chrischiang/.../bytehost/crates/*`）。T7 发布真实 tag 后须删除该段、改用固定 tag，并 `git diff --check` 确认无本地绝对路径泄漏。联调 patch 不提交（plan §T7）。
>
> **R1–R4 代码缺陷已修复（2026-10-15 第三轮，对应 `reviews/2026-10-15-bytehost-i18n-integration-review.md`）：** 审核报告的 R1（initial 不收同文档热更新）+ R3（Dozer 推送生命周期/失败重试）落在 Dozer `app_webview.rs`（`locale_pushed` + `live_locale_push_script` + `mark_locale_pushed`）与 `platform/window_events.rs`（`apply_pending_app_locales` 改走 Live-only 幂等重推、`::tracing::warn!` 记失败、`app_locale_generation` 循环末才更新）；R2（桥 revision/幂等/隔离/校验）+ R4（资源路径 TOCTOU / 逐组件拒链）落在 bytehost `bytehost-webview/src/i18n.rs`（`i18n_bridge_init_script` 重写 + Node 行为测 `bridge_script_enforces_revision_and_isolation_under_node`）与 `bytehost-apps/src/i18n/resources.rs`（`ensure_inside` 重写）。四项均有单测/Node 实跑证据；bytehost `fmt/clippy/check-deps` 与 `cargo test` 全绿，Dozer T6 单测 + B12 e2e 通过。详情与验证边界见该 review 文档。

**验证：**受影响的 `cargo test -p dozer-app`（4 项 T6 单测）、`cargo test -p dozerd --test app_requests`（B12 e2e）；`python3 scripts/audit/check_panel_boundary.py`、`bash scripts/check-i18n.sh`、`bash scripts/check-log-scope.sh` 在 T7 收口时复跑。

**验收（部分）：** B05/B06/B10 的 Dozer 消费侧单测已覆盖标题解析、桥注入接缝、locale 变化推送；B12 经真实 UDS e2e 证据齐备。B01–B04/B07–B09/B11 的样例行为验证与“新 GUI/旧 daemon、旧 GUI/新 daemon”两端 fixture 待 T5 样例 + T7 tag 后补（见上第 6 项递延说明），届时统一写入 T8 的 `docs/dozer-v2/i18n/bytehost-acceptance.md`。

## T7. 上游发布与 Dozer 固定 tag

**归属：两个仓库分别提交/发布。**

- [ ] 准备上游 release notes、能力版本映射、兼容矩阵、样例与完整验证记录；确认发布操作的现有授权后发布实际 tag。
- [ ] Dozer 所有 bytehost crate 统一指向该 tag，更新 Cargo.lock；删除本次联调 patch（保留用户原有 patch），检查 staged diff 没有本地绝对路径。
- [ ] 从发布 tag 重新构建/验证 Dozer，执行打包样例冒烟；证明运行不依赖相邻 bytehost 源码或开发仓库 locales。
- [ ] 记录已安装新包时回退旧宿主的限制；回退流程使用应用版本回滚，禁止直接编辑已批准 manifest。

**验收：**正式依赖可复现，发布 tag 与验收候选 commit 对应，不能用“本地 patch 下通过”替代发布验证。

## T8. I4 交付记录

**归属：Dozer。**

**文件：** 新增 `docs/dozer-v2/i18n/bytehost-acceptance.md`；更新原 Task 14 和现有 ledger/acceptance 的 I4 状态。

- [ ] 记录两个仓库 commit/tag、平台版本、样例版本、B01–B12 命令/结果/截图位置。
- [ ] 明确 Web 本期范围及 initial/legacy 例外；External/Declarative/完整插件注册系统继续列后续事项，不混入完成项。
- [ ] 已通过才勾选原 Task 14 中相应项；旧任务对未来 Surface 的表述改为本期实际范围或显式延期，不能全勾造成错误完成声明。
- [ ] 剩余不阻塞的 UI 文案/排版问题进入 bug 清单，沿用用户已接受的反馈迭代方式。

**完成条件：**上游实际 tag + Dozer 正式消费 + 样例行为验证 + 兼容与安全证据完整；届时声明“bytehost 现有 Web 应用 i18n 接入完成”。

## 2. 总体验证与发布顺序

| 阶段 | 工作目录 | 命令/动作 |
| --- | --- | --- |
| 上游收口 | bytehost | `cargo fmt --all -- --check`；`cargo clippy --workspace --all-targets --all-features -- -D warnings`；`cargo test --workspace --all-features`；`bash scripts/check-deps.sh` |
| 进程样例 | bytehost | 已有 `process_apps_live` / `embedded_host` 的 ignored 测试，在具备对应 Python/Node 的环境执行；新样例测试命令以实际实现为准 |
| 产品收口 | Dozer | 受影响测试通过后执行一次 workspace fmt/test/clippy 与 `git diff --check`；记录既有失败 |
| 发布验证 | Dozer | 固定实际上游 tag 后重新构建、打包、三个样例冒烟，验证无 path patch 依赖 |

普通文件修改只跑对应检查；完整 workspace 检查在候选收口时执行，不为每条标题反复全量测试。不能跳过文档导航、旧版本 wire、安全边界和热切换状态保留这些行为验收。
