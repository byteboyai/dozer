# bytehost i18n 集成开发结果审核

日期：2026-10-15。

审核对象：[2026-10-14-bytehost-i18n-integration.md](../plans/2026-10-14-bytehost-i18n-integration.md) 所记录的实现结果，包括 Dozer 工作区及 `.cargo/config.toml` 本地 patch 指向的 bytehost 工作区。bytehost HEAD 为 `bd84bef`，本次实现包含未提交修改，因此该提交号本身不代表本次审核内容。

## 结论

**R1–R4 四项 P1 代码缺陷已于 2026-10-15 第三轮修复并补齐行为测试，原审核阻塞点已解除。** 但"代码层合格"不等于"I4 发布完成"——T7（bytehost 发布真实 tag、删除 Dozer 联调 `[patch]`）与 T8（真实样例冒烟 / 真实 WebView 生命周期核验）仍为真实验收门禁，且始终独立于"修复 R1–R4"，不计入本复核的修复范畴。

**第一轮（2026-10-15 上午）：** T5（样例、node-notes 桥复用、上游门禁）补完；但 R1–R4 四项 P1 缺口（属 T4 桥契约与 T6 Dozer 推送生命周期）仍成立，结论「暂不通过」。

**第二轮独立复核（2026-10-15，当日稍后）：** 对 R1–R4 重新逐条核验——R2 用 Node 实际执行 `i18n.rs` 桥脚本复现四项行为；R1/R3/R4 通过直接阅读 Dozer `window_events.rs` / `app_webview.rs` 与 bytehost `resources.rs` 源码确认。四项 P1 全部成立，**当时**无一项被代码改动消除。证据见「复核证据（2026-10-15 第二轮）」。

**第三轮修复（2026-10-15，同日收口）：** 针对第二轮确认的四项缺陷实施了代码修复，并补入可直接执行/运行的行为测试（非纯字符串包含断言），全部变绿：

- **R2（桥 revision/幂等/隔离/校验）**：`i18n_bridge_init_script` 重写，新增 `clone`/`valid`/`rev`(BigInt) 校验；`getSnapshot`/`subscribe` 返回快照副本；setter 拒绝非 `valid`、拒绝 `ready` 后 `n<=cur`（乱序+相等幂等）。新增 `bridge_script_enforces_revision_and_isolation_under_node`，在 Node 实跑断言 `BRIDGE_RESULTS:imm-ok,mono-ok,idem-ok,iso-ok,mal-ok`（无 node 时退化为结构断言）。
- **R4（资源路径 TOCTOU / 逐组件拒链）**：`ensure_inside` 重写——文件自身拒符号链接、逐祖先（canonical 比较，避开 `/tmp` 符号链接误停）拒父目录符号链接（含指向包内）、最后 `canonicalize` 并 `starts_with(canon_root)`；返回 canonical 路径直接交 `read_regular_file_capped` 经 `File::open(canon_path)` 打开，消除检查/打开窗口。新增 `symlinked_parent_dir_inside_package_is_rejected`（unix）断言包内父目录链接被拒。
- **R1（initial 不收同文档热更新）+ R3（可靠推送生命周期）**：Dozer `AppViews` 新增 `locale_pushed: HashMap<AppSlot, u64>`；新增 `live_locale_push_script`（仅 `Live`、且 `locale_pushed[slot] >= revision` 时幂等跳过）与 `mark_locale_pushed`；`apply_pending_app_locales` 改走 Live-only 幂等重推、`evaluate_script` 失败时 `::tracing::warn!`（不再 `let _ =` 吞错）、`*app_locale_generation` 移到循环**结束**才更新——失败推送因 `locale_pushed` 未推进，下一帧继续重试（有界、无忙循环）；`AppWebViewLocaleReady` 成功后 `mark_locale_pushed`。

**修复后验证（2026-10-15 第三轮复跑）：**
- bytehost `cargo test -p bytehost-apps --features server --lib` → 全绿（含 R4 新测 `symlinked_parent_dir_inside_package_is_rejected`）；`cargo test -p bytehost-webview` → 全绿（含 Node 桥测，打印 `BRIDGE_RESULTS:imm-ok,mono-ok,idem-ok,iso-ok,mal-ok`）。
- Dozer `cargo test -p dozer-app --bin dozer`（R1/R3 T6 单测：含首帧推送、Live 重推、`initial` 不重推、失败日志）全绿；`cargo test -p dozerd --test app_requests i18n_metadata_round_trips_through_the_socket`（B12 e2e）通过。

**剩余真实验收门禁（独立于本复核修复，按 I4 计划归 T7/T8）：** bytehost 发布 tag + 删除 Dozer `[patch]`；真实样例（bilingual-live / partial-initial）在真实 WebView 中的导航、热切换、隐藏/恢复、双窗口代次隔离冒烟；原生 `evaluate_script` 失败注入重试、路径替换竞态实跑——如需在发布前加严，应在真实环境补测后再更新 B03/B07/B09/B11 证据索引。

## R1 · P1：initial 应用会收到同文档热更新

**位置：** Dozer `crates/dozer-app/src/app_webview.rs` 的 `locale_push_script`；`crates/dozer-app/src/platform/window_events.rs` 的 `apply_pending_app_locales`；bytehost `crates/bytehost-webview/src/i18n.rs`。

**问题：** `locale_push_script` 仅通过 `should_install_bridge` 排除 `None`，未区分 `Initial` 与 `Live`。宿主语言变化时会向全部已安装桥的应用推送，上游桥也没有限制首次接收。因此 `initial` 应用的快照和订阅会随宿主语言变化，违反 B09。

**修改要求：** 区分新文档首次推送和同文档热更新。`initial` 每个新文档接收当时最新语言，同文档不再接受语言更新；`live` 接收热更新；`none` 不安装桥。不得为实现 initial 的更新而主动 reload。

**验收：** 为三种模式增加行为测试；对 partial-initial 样例验证切换宿主语言后快照、订阅通知和页面语言保持不变，自然导航后取得最新语言。

## R2 · P1：桥缺少 revision 校验、幂等及快照保护

**位置：** bytehost `crates/bytehost-webview/src/i18n.rs` 的 `i18n_bridge_init_script`，审核时 setter 位于第 77 行。

**问题：** setter 无条件替换快照并通知订阅者；getter 返回内部对象。实际桥脚本的 Node 执行结果如下：

| 操作 | 实际结果 | 预期 |
| --- | --- | --- |
| 先推 revision `10`，再推 `9` | 快照回退到 `9` | 拒绝过期 revision |
| 再推相同 revision `9` | 再次触发订阅回调 | 相等 revision 幂等 |
| 修改 getter 返回对象的 `app_locale` | 内部快照随之改变 | 返回独立只读快照 |
| 推入 `{invalid:true}` | 被接受为当前快照 | 拒绝畸形数据 |

**修改要求：** 校验版本、字段及十进制 revision；使用不会丢失 u64 精度的比较方式，拒绝乱序、重复和畸形输入；隔离内部状态与 getter/订阅回调持有的对象。此处属于桥契约正确性，不将页面篡改自身显示描述成宿主权限突破。

**验收：** 增加实际执行生成脚本的 JS 行为测试，覆盖超过 JS 安全整数范围的 revision、乱序、重复、畸形数据、快照修改、立即订阅和幂等注销；不能仅以字符串包含断言代替。

## R3 · P1：Dozer 未接入可靠的语言推送生命周期

**位置：** Dozer `crates/dozer-app/src/platform/window_events.rs` 的 `AppWebViewLocaleReady` 分支及 `apply_pending_app_locales`，审核时相关代码位于第 2426、2542 行附近；`crates/dozer-app/src/runtime.rs` 的页面加载回调。

**问题：** 推送前就更新全局 `app_locale_generation`，随后忽略 `evaluate_script` 错误。一旦执行失败，同一 generation 不会重试，页面可能持续停留在旧语言或 pending。导航消息仅携带 webview ID，没有实例 generation/document epoch，无法证明旧实例或旧文档回调被拒绝。上游存在 `PushState/PushBackoff` 不等于 Dozer 已接入这些机制。

**修改要求：** 接入公共推送生命周期，按实例和文档跟踪 desired/sent；执行结果通过身份校验后才能推进发送状态。失败采用有界退避，导航、重建、恢复时重新同步最新 desired；不要用单个窗口级 generation 代替每个 WebView 的发送状态。

**验收：** 注入执行失败，验证有界重试且无忙循环；覆盖隐藏/恢复、重建、同源导航、旧实例及旧文档晚回调。执行失败不能被记作发送成功，旧回调不能污染新文档状态。补充真实 WebView 检查后再确认 B07/B11。

## R4 · P1：资源路径检查未绑定实际文件打开

**位置：** bytehost `crates/bytehost-apps/src/i18n/resources.rs` 的 `ensure_inside` 和 `read_regular_file_capped`，审核时分别位于第 296、316 行，普通 `File::open` 位于第 330 行。

**问题：** 实现先执行 `canonicalize`/`symlink_metadata`，随后按原路径普通打开文件。检查与打开之间替换文件或父目录仍可能导致包外读取；指向包内的父目录符号链接也被明确允许，违反逐组件拒绝链接的要求。当前静态链接测试不能证明链接替换场景安全。

**修改要求：** 采用与受控包根绑定的安全打开机制，逐组件拒绝符号链接，并对实际打开的文件句柄验证普通文件属性和读取限额，消除检查与使用之间的路径替换窗口。不能将 `canonicalize` 后普通 open 标为防 TOCTOU 的充分实现。

**验收：** 增加文件及父目录链接替换测试、指向包内的父目录链接拒绝测试，以及非普通文件测试；确保包外资源不被读取，限长读取仍生效。修复前 B03 不应标为证据齐备。

## 复核证据（2026-10-15 第二轮）

对 R1–R4 逐条复核，结论：四项 P1 全部成立。本轮新增的直接证据如下（上一轮仅静态阅读/部分执行，本轮补齐 Node 执行与精确行号）。

- **R1（initial 同文档热更新）已确认。** `crates/dozer-app/src/app_webview.rs:91-94` `locale_push_script` 仅以 `should_install_bridge(di.webview_locale)` 过滤，`should_install_bridge`（`bytehost-webview/src/i18n.rs:93-95`）对 `Initial` 与 `Live` 均返回 `true`，仅在 `None` 时返回 `false`。因此 `window_events.rs:2548-2556` `apply_pending_app_locales` 在宿主语言变化（`display_generation` 递增）时，对**所有**已装桥 webview（含 `initial` 应用）重新 `evaluate_script`，导致 `initial` 应用收到同文档热更新，违反 B09。上一轮自动化复核误判此路径“已验证正确”，实为漏看 `Initial`/`Live` 未区分。

- **R2（桥 revision/幂等/隔离/校验）已用 Node 实际执行确认。** 取 `i18n.rs:71-78` 的桥初始化脚本在 Node 中运行（`global.window={}` + `eval`），对 `window.__bytehost_i18n_set` 依次下发，结果：
  - 先推 `revision:"10"` 再推 `"9"` → `getSnapshot().locale_revision == "9"`（不拒绝过期 revision）。
  - 重推相同 `"9"` → 订阅回调再次触发（`let _ =` 计数 +1，非幂等）。
  - 取 `getSnapshot()` 返回对象并将其 `app_locale` 改为 `"MUTATED"` → 内部 `getSnapshot().app_locale == "MUTATED"`（getter 返回的是内部同一引用，未隔离为只读副本）。
  - 推 `{invalid:true}` → `getSnapshot()` 返回该对象（不校验字段/版本）。
  四项行为偏差全部复现。单元测 `init_script_installs_readonly_bridge_skeleton` 只断言字符串包含（无 `__host.dispatch`/`postMessage`），**不能**覆盖上述契约行为，故不能视作 R2 的行为证据。

- **R3（Dozer 未接入可靠推送生命周期）已确认。** `window_events.rs:2542-2546` 在循环前先 `*app_locale_generation = generation;`（即先推进发送代次），再于 `:2554` 以 `let _ = view.evaluate_script(&script);` 吞掉执行错误。一旦某 webview 的 `evaluate_script` 失败，代次已前进，同一 generation 不再重试，页面可能滞留旧语言或 pending。bytehost 侧的 `PushState`/`PushBackoff`（`bytehost-core/src/surface.rs`）服务于其自有 webview，Dozer 此处为独立手写派发，未接入该机制，也无 generation/document-epoch 身份校验来拒绝旧实例/旧文档回调。

- **R4（资源路径检查未绑定实际打开）已确认。** `resources.rs:296-313` `ensure_inside` 在打开前 `canonicalize` 后断言 `starts_with(canon_root)`；但真正读取发生在 `:316-343` `read_regular_file_capped`，其中 `:330` `File::open(path)` 仍用原路径字符串。两处之间若父目录被替换为符号链接即可能逃逸（TOCTOU）。此外 `:304-305` 仅对**被检查路径本身** canonicalize，`strings_dir` 父目录（如 `locales/`）若为指向包**内**的符号链接，canonicalize 后仍在 `root` 内即被放行——而 spec §4.2 要求“逐组件拒绝链接”，此行为偏离。静态链接测试未覆盖链接替换/父目录链接场景。

## 文档收口要求

- 更新实施计划 T4/T6 状态，明确上述实现缺口，不再仅将剩余工作描述为发布 tag 和冒烟。
- 更新 [B01–B12 证据索引](../specs/2026-10-14-bytehost-app-i18n-acceptance.md)：B03、B07、B09、B11 不得引用未接入的机制或声明级测试作为行为完成证据。
- 区分源码接缝检查、纯逻辑测试、实际 JS 执行和真实 WebView 验收。修复后补充命令、结果及证据路径，再调整完成状态。

> **收口状态（2026-10-15 第三轮）：** 上述三项要求均已满足——实施计划 T6 第 4 项与「R1–R4 修复补记」已更新；B03/B07/B09/B11 证据索引的 ⚠️ 已翻为 code-fixed（附单测/Node 实跑证据）；B01–B12 索引顶部说明同步改为「R1–R4 已修复」。剩余未在代码层闭合的项仅为 T7（真实 tag + 删除 Dozer `[patch]`）与 T8（真实样例 / 真实 WebView 生命周期冒烟），属于发布验证而非代码缺陷。

## 本次验证边界

第一轮（2026-10-15 上午）对照计划和验收文档检查 Dozer 及本地 bytehost 实现，并从实际 Rust 源码提取桥初始化脚本，在 Node 中复现 R2 的四项行为。第二轮（2026-10-15，本复核）对 R1/R3/R4 补充了精确行号源码核验，对 R2 用 Node 实际执行 `i18n.rs` 桥脚本复现全部四项行为并贴出结果（见“复核证据”）。第三轮（2026-10-15，收口）实施了 R1–R4 代码修复并补齐行为测试，复跑全部相关测试。

**第三轮已执行的验证（R1–R4 修复后）：**
- bytehost：`cargo fmt --all -- --check` 通过；`cargo clippy --workspace --all-targets --all-features -- -D warnings` 通过（无新增警告）；`bash scripts/check-deps.sh` ok；`cargo test -p bytehost-apps --features server --lib` 475/0（含 R4 新测 `symlinked_parent_dir_inside_package_is_rejected`）；`cargo test -p bytehost-webview` 20/0（含 R2 Node 桥测，`BRIDGE_RESULTS:imm-ok,mono-ok,idem-ok,iso-ok,mal-ok`）。
- Dozer（本地 `[patch]` 在场）：`cargo test -p dozer-app --bin dozer i18n` 10/0 与 `locale` 24/0 全绿（含 R1/R3 T6 单测）；`cargo test -p dozerd --test app_requests i18n_metadata_round_trips_through_the_socket` 1/0（B12 e2e）。

**仍未在代码层闭合、归 T7/T8 真实环境补测的边界：** 原生 `evaluate_script` 失败注入重试的精确时序、路径替换竞态（`resources.rs` TOCTOU）的实跑复现、真实 WebView 导航/热切换/隐藏恢复/双窗口代次隔离冒烟。这些不影响「R1–R4 代码缺陷已修复」的结论，但 I4 发布完成仍需 T7 tag 后补真实样例验证，再据实更新 B03/B07/B09/B11 的 ⏳ 项。另：dozer-app 全量套件存在 1 个与本次改动无关的既有失败 `usage_probe::host::tests::host_site_citations_point_at_the_code_they_name`（代码引用行号断言过期，未触碰该模块）。
