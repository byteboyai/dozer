# bytehost 应用 i18n — I4 交付记录（Dozer 侧）

- 日期：2026-10-15
- 范围：bytehost 现有 **Web 应用** 的 i18n 接入（清单/资源/桥/投影 + Dozer 消费适配），含 R1–R4 代码缺陷修复与上游发布。
- 权威规格：`bytehost/docs/superpowers/specs/2026-10-14-bytehost-app-i18n.md`；Dozer 侧证据索引见 [`docs/superpowers/specs/2026-10-14-bytehost-app-i18n-acceptance.md`](../../superpowers/specs/2026-10-14-bytehost-app-i18n-acceptance.md)；接入计划见 [`docs/superpowers/plans/2026-10-14-bytehost-i18n-integration.md`](../../superpowers/plans/2026-10-14-bytehost-i18n-integration.md)；审核见 [`docs/superpowers/reviews/2026-10-15-bytehost-i18n-integration-review.md`](../../superpowers/reviews/2026-10-15-bytehost-i18n-integration-review.md)。

## 本期范围与例外

**本期完成**：bytehost 现有 Web 应用（HTTP/静态/Node 等经 webview 渲染的应用）的 i18n 接入——`manifest [i18n]` 声明、locale 协商、标题解析与回退、只读语言桥 `window.bytehost.i18n`、资源安全装载、Dozer 外壳标题渲染与语言推送生命周期。

**例外（明确不在本期）**：
- `legacy`（`webview_locale` 未声明 / `none`）：不装桥、不推快照，完全走原路径。
- `initial`：`webview_locale="initial"` 仅首帧 + 一次立即回调，**同文档语言切换不推送**（见 B09 / R1）。
- External / Declarative 应用、完整插件注册系统：列为后续事项，不混入本期完成项。

## 仓库版本

| 仓库 | 版本 | 说明 |
| --- | --- | --- |
| bytehost | commit `a98fd04`，tag **`v0.3.0`**（已推送 `github`） | 整个 i18n 特性 + R1–R4 修复，单提交 `feat(i18n): app internationalization, read-only bridge, and safe resource loader` |
| Dozer | 依赖统一指向 **bytehost `v0.3.0`** | 6 个 crate 的 `tag` 由 `v0.2.0` 升到 `v0.3.0`，删除本地联调 `[patch."…/bytehost"]`；`Cargo.lock` 已刷新 |
| bytehost `HOST_VERSION` | 仍为 `0.1.0` | 沿用仓库既有约定（tag 版本与宿主能力版本解耦，且旧宿主对未知 `[i18n]` 字段由 `deny_unknown_fields` 拒绝），故未改 |

## 交付内容清单（Dozer 侧）

- `app_webview.rs`：`AppViews::display_i18n` / `sync_display_i18n` / `localized_title` / `locale_push_script`；新增 `locale_pushed` + `live_locale_push_script`（仅 `Live`、按代次幂等）+ `mark_locale_pushed`（R1/R3）。
- `platform/window_events.rs`：`apply_pending_app_locales` 改走 `live_locale_push_script`，`evaluate_script` 失败 `::tracing::warn!` 且 `*app_locale_generation` 循环末才更新；`AppWebViewLocaleReady` 成功后 `mark_locale_pushed`（R1/R3）。
- `runtime.rs` / `chrome/rail.rs` / `extensions/settings_apps_view.rs` / `host_adapters/app_host.rs`：导航完成后推首帧快照、外壳标题经 `resolve_title` 解析、slot/权限身份稳定。
- `dozerd/tests/app_requests.rs`：`i18n_metadata_round_trips_through_the_socket`（B12 UDS e2e）。
- `Cargo.toml`（dozer-app/dozerd/dozer-client/dozer-core/dozer-mcp/dozer-panel-usage）：bytehost 依赖升 `v0.3.0`；`.cargo/config.toml` 移除 bytehost 联调 patch。

## B01–B12 汇总

完整证据与状态见 [`2026-10-14-bytehost-app-i18n-acceptance.md`](../../superpowers/specs/2026-10-14-bytehost-app-i18n-acceptance.md)。摘要：

| ID | 场景 | 状态 |
| --- | --- | --- |
| B01 | 旧应用 / 旧 wire fixture | bytehost ✅ / Dozer ✅ 单测级；⏳ 待两仓真实旧 fixture 端到端 |
| B02 | manifest 组合 / 非法资源 | bytehost ✅ |
| B03 | 路径穿越 / 链接 / 替换 | ✅ code-fixed（R4 已修，含 `symlinked_parent_dir_inside_package_is_rejected`）；⏳ 路径替换竞态实跑归真实环境 |
| B04 | 批准后改资源 / 升级回滚 | bytehost ⚠️ 部分（安装拒绝✅，运行期降级待 T3）；⏳ Dozer 待 T7 |
| B05 | zh-CN/en / 缺译 / 未知宿主语言 | bytehost ✅ / Dozer ✅ 解析级；⏳ 页面渲染待真实样例 |
| B06 | 首开 / 订阅晚注册 / 语言变更后导航 | bytehost ✅ / Dozer ✅ 接缝级；⏳ 真实 webview 导航待 T7 |
| B07 | 隐藏/恢复/重建/旧脚本晚回/同值重推 | ✅ code-fixed（R3 已修）；⏳ 真实生命周期待 T7 |
| B08 | live 热切换 | ⏳ 待真实样例热切换冒烟 |
| B09 | initial / none / 双窗口代次 | ✅ code-fixed（R1 已修）；⏳ 双窗口代次隔离待 T7 |
| B10 | origin/CSP/nonce/权限隔离 | bytehost ✅ |
| B11 | 执行失败 / 运行期资源损坏 | ✅ code-fixed（R3 已修，Dozer 推送路径已接入失败重试/日志）；⏳ 原生失败注入归 T7 |
| B12 | Dozer UDS + bytehost InProcess | Dozer ✅（B12 e2e 通过） |

## 验证证据（本回合实跑）

**bytehost（v0.3.0 提交内容）**
- `cargo fmt --all -- --check` → OK
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` → 通过（仅依赖 `block v0.1.6` 未来不兼容提示，非本仓库警告）
- `bash scripts/check-deps.sh` → ok
- `cargo test -p bytehost-apps --features server --lib` → **475 passed / 0 failed**（含 R4 新测 `symlinked_parent_dir_inside_package_is_rejected`）
- `cargo test -p bytehost-webview` → **20 passed / 0 failed**（含 R2 Node 桥测，打印 `BRIDGE_RESULTS:imm-ok,mono-ok,idem-ok,iso-ok,mal-ok`）

**Dozer（依赖 v0.3.0，本地构建）**
- `cargo build -p dozer-app` → 通过（仅既有 dead_code / linker 提示）
- `cargo test -p dozer-app --bin dozer i18n` → **10 passed / 0 failed**
- `cargo test -p dozer-app --bin dozer locale` → **24 passed / 0 failed**
- `cargo test -p dozerd --test app_requests i18n_metadata_round_trips_through_the_socket` → **1 passed / 0 failed**（B12 e2e）

**R1–R4 修复落点**
- R1（initial 不收同文档热更新）：`dozer-app` `live_locale_push_script` 排除 `Initial`/`None`；`app_webview.rs` / `window_events.rs`。
- R2（桥 revision/幂等/隔离/校验）：`bytehost-webview/src/i18n.rs` `i18n_bridge_init_script` 重写 + `bridge_script_enforces_revision_and_isolation_under_node`（Node 实跑）。
- R3（Dozer 推送生命周期/错误）：`dozer-app` `locale_pushed` + `mark_locale_pushed`，失败 `::tracing::warn!`、循环末更新代次、下一帧重试。
- R4（资源路径 TOCTOU / 逐组件拒链）：`bytehost-apps/src/i18n/resources.rs` `ensure_inside` 重写，canonical 路径直接交 `File::open`。
- 审核结论已从「暂不通过」改为「R1–R4 代码层阻塞解除」，见 review 文档。

## 已知限制与待补测（⏳，归真实环境，不阻断本次代码交付）

以下项按接入计划属 T7 tag 之后真实样例/真实 WebView 验证，本回合以单测/Node/e2e 证据收口，未在真实 webview 运行：
- 真实 WebView 中 `bilingual-live` 热切换（input/scroll 状态保留）、`partial-initial` 重新打开取新 locale；
- 隐藏/恢复、重建、同源导航、旧实例/旧文档晚回调；
- 双窗口不同显示代次隔离；
- 原生 `evaluate_script` 失败注入重试的精确时序；
- `resources.rs` 路径替换竞态（`/tmp` 等系统符号链接、父目录链接替换）的实跑复现。

## 其他说明

- **一个既有无关失败**：`dozer-app` 全量套件存在 `usage_probe::host::tests::host_site_citations_point_at_the_code_they_name` 失败（代码引用行号断言过期，未触碰该模块），与本次 i18n 工作无关。
- **Dozer 提交范围**：本次提交包含 T6 消费适配、R1–R4 修复、v0.3.0 依赖切换及本交付文档，均为 i18n 相关工作；不含其他无关改动。

## I4 完成判定

按接入计划「完成定义」：上游实际 tag（**v0.3.0**，已发布）+ Dozer 正式消费（已切 tag、构建/测试通过）+ 样例行为验证 + 兼容与安全证据。其中**代码、发布 tag、单测/Node/e2e 证据均已齐备**；真实 WebView 行为验证（⏳ 项）为计划明确的 T7 之后真实环境补测，不阻断本期代码交付。可据实声明：「bytehost 现有 Web 应用 i18n 接入完成（代码与发布层面）」，并同步在原始 Task 14 中将对应项勾选为「代码/发布完成、真实样例待补测」。
