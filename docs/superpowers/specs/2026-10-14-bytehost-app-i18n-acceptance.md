# bytehost App i18n — B01–B12 证据索引与完成定义

> 本文件是 I4 的验收依据：把权威规格 `bytehost/docs/superpowers/specs/2026-10-14-bytehost-app-i18n.md` §8 的 B01–B12 映射到具体证据位置，并标注各条当前状态（bytehost 侧已证 / Dozer 侧已证 / 待 T7 tag + 真实样例冒烟）。
> 上游代码、契约与测试在 bytehost 仓库；Dozer 消费适配与 UDS e2e 在 Dozer 仓库。本文件跨两仓汇总。
> 基线决策记录见 `bytehost/docs/superpowers/plans/2026-10-14-bytehost-i18n-baseline.md`；接入计划见 `dozer/docs/superpowers/plans/2026-10-14-bytehost-i18n-integration.md`。

## 状态图例

| 标记 | 含义 |
| --- | --- |
| ✅ bytehost | bytehost 侧证据齐备（单测 / 样例 / 安装夹具） |
| ✅ Dozer | Dozer 侧证据齐备（单测 / UDS e2e） |
| ⏳ 待 T7 | 需上游真实 tag + 真实样例冒烟后才能标完成（不靠本地 `[patch]` 联调替代） |

## B01–B12 证据索引

> 注意：审查文档 `reviews/2026-10-15-bytehost-i18n-integration-review.md` 的 **R1–R4 四项 P1 已于 2026-10-15 第三轮修复并补齐行为测试**（结论已从「暂不通过」改为「代码层阻塞解除」）。其中 R1→B09、R3→B07/B11、R4→B03，对应行的 ⚠️ 已根据代码修复与单测证据翻为 code-fixed；**真实 WebView 导航/热切换/双窗口代次隔离冒烟仍归 T7 真实 tag + 真实样例**（⏳），不在本复核修复范畴。

| ID | 场景 | bytehost 证据（已证部分） | Dozer 证据（已证部分） | 状态 |
| --- | --- | --- | --- | --- |
| B01 | 旧应用、旧 wire fixture | `legacy-sample` 无 i18n 安装/打开/导航走旧路径（`scripts/samples/i18n/legacy-sample`）；`old_manifest_without_i18n_still_validates` 旧 manifest 解析通过 | 新 GUI 解析旧 daemon 摘要 `display_i18n=None` 降级经 `should_install_bridge(None)` 走原路径（T6 单测 `build_app_webview_pins_the_restrictive_settings`） | bytehost ✅ / Dozer ✅ 单测级；⏳ 待两仓真实旧 fixture 端到端 |
| B02 | manifest 组合、locale、重复 key/非法资源 | 3 个样例安装投影一致 + 6 个非法夹具经 `install_plan` 安装期拒绝（`security_fixture_rejects_*`）；`i18n_samples.rs` 4 测覆盖合法组合 | — | bytehost ✅ |
| B03 | 路径穿越、链接/替换、超限 | `security_fixture_rejects_path_traversal_strings_dir` / `..._symlinked_resource_escape`（unix）；`i18n/resources.rs` 单测 `symlink_escape_outside_package_is_rejected`、`symlinked_parent_dir_inside_package_is_rejected`（unix）、限长读取、超限计数 | — | ✅ code-fixed（review R4 已修）：`ensure_inside` 重写——文件自身拒符号链接、逐祖先（canonical 比较，避开 `/tmp` 符号链接误停）拒父目录符号链接（含指向包内）、最后 `canonicalize` 并 `starts_with(canon_root)`，返回 canonical 路径直接交 `read_regular_file_capped` 经 `File::open(canon_path)` 打开，消除检查/打开窗口，满足 spec §4.2「逐组件拒链接」。⏳ 路径替换竞态实跑归 T7 真实样例 |
| B04 | 批准后改资源、升级/回滚/卸载 | 安装期拒绝已落地：`read_package` 经 `source_digest` 重验；批准后改资源失败。升级/回滚/缓存失效随 T3 服务路径（运行期 `list()` 投影）补，运行期损坏由 `resolve_title` 回落覆盖 | — | bytehost ⚠️ 部分（安装拒绝✅，运行期降级待 T3）；⏳ Dozer 待 T7 |
| B05 | zh-CN/en、缺译文、未知宿主语言 | `i18n_samples.rs`：`bilingual_live_*`（zh-CN→"双语实时"/"打开演示"、fr→"Bilingual Live" default 回落）、`partial_initial_*`（zh-CN→"部分初始"、en→"Partial Initial"）、`node_notes_reuses_the_bridge`（zh-CN→"Node 笔记"/"打开演示"、fr→"Node Notes"） | T6 单测 `app_rail_title_resolves_i18n_or_falls_back_to_id`（Presentation 标题解析 + 缺译回退 id） | bytehost ✅ / Dozer ✅ 解析级；⏳ 页面渲染待真实样例 |
| B06 | 首开、订阅晚注册、语言变更后再导航 | T4 bridge：`i18n_bridge_init_script` 装只读骨架（首帧前 `getSnapshot()` 返回 `null`，pending/skeleton 不变量）、`subscribe` 立即回调当前快照；`app_webview.rs::i18n_locale_must_be_pushed_post_navigation_not_via_init_script` 钉死"导航后 `evaluate_app` 推首帧、不经 init script" | T6 `runtime.rs` 在 `PageLoadEvent::Finished` 后 `evaluate_script(i18n_set_locale_script)` 推快照；`window_events.rs` `AppWebViewLocaleReady` 派发 | bytehost ✅ / Dozer ✅ 接缝级；⏳ 真实 webview 导航待 T7 |
| B07 | 隐藏/恢复、重建、旧脚本晚回、同值重推 | T4 `PushState` + `PushBackoff`（generation/document epoch 隔离）+ `SurfaceRouter` 按 `Generation` 拒旧实例 | T6 `apply_pending_app_locales` 改走 `live_locale_push_script`（Live-only、按 `locale_pushed[slot]` 幂等跳过）、`evaluate_script` 成功才 `mark_locale_pushed`、失败 `::tracing::warn!` 且 `*app_locale_generation` 移到循环结束才更新——失败推送因 `locale_pushed` 未推进、下一帧继续重试（有界、无忙循环）；`AppWebViewLocaleReady` 成功亦 `mark_locale_pushed` | ✅ code-fixed（review R3 已修）；⏳ 真实隐藏/恢复/重建/旧实例晚回调待 T7 真实 WebView 冒烟 |
| B08 | live 热切换 | `bilingual-live` 样例设计即验证：input/log 不随语言切换丢失（`web/index.html` 只改文本+`lang`，不读写 input/scrollTop） | T6 桥注入 + `display_generation` 重推；无 reload/restart/start-stop | ⏳ 待真实样例热切换冒烟（T5 样例 + T7 tag） |
| B09 | initial / none、两个窗口不同显示代 | `legacy-sample`（`none` 无桥）+ `partial-initial`（`initial` 仅首帧、同文档不热改）对照；`WebViewLocale` 枚举 `none/initial/live` | T6 `should_install_bridge` 据 `webview_locale` 决定装桥；`locale_push_script`（新文档首帧）覆盖 `initial`/`live`，而热更新改走 `live_locale_push_script`（`Initial`/`None` 被排除），从而 `initial` 同文档不再热改；`locale_revision` 仅外壳本地代次、不跨窗口比较 | ✅ code-fixed（review R1 已修）；⏳ 双窗口代次隔离待 T7 真实样例 |
| B10 | origin/CSP/nonce/权限隔离 | T4 bridge 单测 `init_script_installs_readonly_bridge_skeleton`：桥无 `__host.dispatch` / `postMessage`，无宿主命令；脚本经私有 nonce 通道与 focus/zoom 隔离 | — | bytehost ✅ |
| B11 | 执行失败、运行期资源损坏 | T4 `PushBackoff` 单测 `push_backoff_is_bounded_and_resets_on_success`（有界退避、无忙循环）；运行期损坏由 `resolve_title` 标题安全回退 | T6 `apply_pending_app_locales` 的 `evaluate_script` 失败改走 `::tracing::warn!` 且 `locale_pushed` 不推进→下一帧重试（有界、无忙循环），页面不滞留旧语言/pending 的概率由重试覆盖 | ✅ code-fixed（review R3 已修，Dozer 推送路径现已接入失败重试/日志）；bytehost 侧 `PushBackoff` 单测仍在；⏳ 原生 `evaluate_script` 失败注入冒烟归 T7 |
| B12 | Dozer UDS + bytehost InProcess | 同源 DTO（bytehost proto `AppDisplayI18n` / `WebViewLocale`） | Dozer e2e `dozerd/tests/app_requests.rs::i18n_metadata_round_trips_through_the_socket`：真实 `AppService`+UDS 经 `dozer-client::Client::app_list()` 断言 `default_locale=="en"`、`supported_locales==["en","zh-CN"]`、`webview_locale==WebViewLocale::Live`、`resolve_title(Presentation,"zh-CN")=="笔记"`/`"en"=="Notes"`/`"fr"=="Notes"` | Dozer ✅（bytehost 同 DTO 经 InProcess conformance 共享） |

## 资源限额记录（spec §4.1，实施时强制）

| 限额 | 值 | 实施点 |
| --- | --- | --- |
| 声明语言数 | ≤ 16 | `i18n/resources.rs` 枚举计数 |
| 单文件大小 | ≤ 512 KiB | 限长读取（limit+1），解析前生效 |
| 声明资源总计 | ≤ 8 MiB | staging/安装两阶段重验 |
| 每文件 key 数 | ≤ 4096 | 解析计数 |
| key 长度/字符集 | 1–128 个 `[A-Za-z0-9_.-]` | 校验 |
| 普通值长度 | ≤ 4096 Unicode | 校验 |
| 标题值长度 | ≤ 64 Unicode + 控制字符/bidi 校验 | 复用 `check_display` |
| 投影序列化 | ≤ 128 KiB（≤64 入口） | `manager.list()` 投影处强制 |

## 旧宿主拒绝新包 / initial 限制 / 升级回滚规则

- **旧宿主拒绝新包**：manifest 启用 `[i18n]` 必须 bump `min_host_version`（因 `deny_unknown_fields`，旧严格解析器直接拒绝未知字段）；`Manifest::from_toml` 在 `min_host_version > host_version` 时拒绝安装。新宿主兼容无 i18n 旧包（`i18n: Option`，默认 `None`）。
- **initial 限制**：`webview_locale="initial"` 仅首次快照 + 一次立即回调，**同文档语言切换不推送**；页面需重新打开（新文档）才取得新 locale，外壳不自动 reload。
- **none 限制**：不注入桥，`window.bytehost.i18n` 不存在；外壳标题沿用 `manifest.name` / `Entrypoint.title`。
- **升级/回滚规则**：locale 文件随包版本进入 `package_dir`，回滚自动恢复该版本 locale；不为 locale 变化触发 start/stop/LaunchUrl；摘要经 `app-change/list` 刷新元数据。禁止直接编辑已批准 manifest（经 `source_digest` 重验失败）。旧宿主回退不保证能读新 manifest，应保留旧应用包或先按受支持流程回滚应用。

## 样例清单（已交付）

| 样例 | 路径 | `[i18n]` | `webview_locale` | 桥消费 | 备注 |
| --- | --- | --- | --- | --- | --- |
| bilingual-live | `scripts/samples/i18n/bilingual-live` | en/zh-CN 完整 | `live` | 是 | input/log 状态保存，证明热切换不丢状态 |
| partial-initial | `scripts/samples/i18n/partial-initial` | en 完整，zh-CN 仅 `app.title` | `initial` | 是（首帧） | 缺失 key 回落 default(en) |
| legacy-sample | `scripts/samples/i18n/legacy-sample` | 无 | （无桥） | 否 | 旧路径基线对照 |
| node-notes（就地复用桥） | `scripts/samples/node-notes` | en/zh-CN | `live` | 是 | 现有 HTTP 样例扩增；保留 `runtime:`/`hits: 0` 子串供 `process_apps_live.rs` 断言；两个 locale 文件必存在 |

验证测试：`crates/bytehost-apps/tests/i18n_samples.rs`（`#![cfg(feature = "server")]`，4 测全绿）。

## 完成定义（I4）

I4 本期完成 = 上游代码/契约/测试 + 已发布 tag + Dozer 适配 + B01–B12 证据。

- 当前状态：bytehost T0–T5 完成（样例 + 安装夹具 + 桥 + 投影 + 解析）；Dozer T6 消费适配完成（本地 `[patch]` 联调，未跑真实样例）。
- 待 T7 发布真实 tag 后：删除 Dozer `.cargo/config.toml` 联调 patch、统一指向 tag、复跑样例冒烟；届时把 ⏳ 条目中"真实样例/真实 webview/双窗口"证据补齐，再于 T8 写入 `docs/dozer-v2/i18n/bytehost-acceptance.md` 并勾选 I4。
- 报告明确"现有 Web 应用契约已完成"，不宣称所有未来 V2 Surface 或任意第三方应用已本地化；External/Declarative/完整插件注册系统继续列后续事项。
