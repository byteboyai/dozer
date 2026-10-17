# bytehost 后续事项(A 组 + B 组)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 做完 2026-10-17 验收后留下的、已有明确方案的小项:重订阅退避、两个偶发测试、压缩包 `./` 前缀、应用图标栏首字母徽标、重启后统一重载页面、运行时区域按钮被挤;并把"有意不改"的行为和验收结果写回文档。

**Architecture:** 三个仓库各自一条分支:`bytehost`(状态机/归档/测试,发 `v0.4.7`)、`byteui`(新增"字符徽标"图标入口,发 `v0.4.5`)、`dozer`(消费前两者并做自己的改动)。发版与推送都是对外动作,每次先问用户。

**Tech Stack:** Rust、iced 0.14、`bytehost-panel` / `bytehost-apps`、`byteui::interaction::icons`、cargo 测试。

**Spec:** `docs/superpowers/manual_tests/2026-10-16-bytehost-manual-acceptance.md`(结果与"发现"各行)、`docs/superpowers/unfinished/2026-10-16-bytehost-remaining.md`(§4 待修项)。用户裁决(2026-10-17):图标=首字母徽标;hang 无提示=记为有意行为不改;重启后=统一重载页面;Crashed 页等待=不改策略只记录。

## Global Constraints

- 依赖一律用 git tag;本地联调 `[patch]` 只放不提交的 `.cargo/config.toml`;提交前还原被它改动的 `Cargo.lock` 行。
- **dozer 与 digger 同日升级 bytehost / byteui**(ADR);本计划只做 dozer 侧,digger 另行(用户处理)。
- **新 UI 复用统一组件**:徽标走 byteui 的图标按钮外壳(`icon_button_entry` 的共享外壳),不在 dozer 里复制一份按钮样式。
- 字体:徽标字符用系统默认字体 + `Shaping::Advanced`(CJK 回退),不用等宽字体。
- wire 形状只追加不修改;本计划不改任何 wire 类型。
- 路径安全:压缩包条目仍必须拒绝 `..` 段、绝对路径、反斜杠、NUL、空段;只放宽**开头的 `./`**。
- bytehost 测试需 `--features server` / `--all-features`。
- 分支基线:dozer 的分支基于 `origin/main`(**不要**基于本地 `main`,里面可能有别的会话未推送的提交);推送用 `git push origin <分支>:main`(快进)。
- **推送、打 tag、合并 main 每次先问用户。** byteui 本地 `main` 比远程多一个提交(`3677749` 新增 `teal` 颜色 token,是 dozer 本地另两个未推送提交所需);基于它发 `v0.4.5` 会顺带发布它——发布前要明确告知用户。

## Review Focus

1. 重订阅退避:真实流程(经过 `subscribe_if_due`,即 `Backoff → Pending → 再次失败`)下档位必须逐档拉长并封顶;抖动恢复后档位要复位。
2. 压缩包 `./`:`./a/b` 被接受并规范成 `a/b`;`./`、`.` 单独成条目被跳过而不是报错;`a/./b`、`./../x`、`.//a`、`/a`、`..` 仍被拒绝。
3. 徽标字符:中文取第一个字、英文取首字母大写、前导空白被跳过、空名回落到 `?`;同一个应用 id 永远得到同一颜色;所有候选底色上的字符对比度 ≥ 3。
4. 重启后重载:快速重启(状态机可能没观察到 `Starting`)与慢重启两种情形都要让页面重新加载;不能让"还停在崩溃前的旧页面"的情形继续存在。
5. 运行时区域:状态文字很长(`可用 · uv 0.8.2 (21fadbcc1 2025-07-22)`)时「安装…」按钮完整可见、不被压成两行。

---

## 范围说明

| 项 | 处置 |
|---|---|
| hang 期间界面无"无响应"提示 | **有意不改**:要显示就得新增"无响应"状态并追加 wire、改 supervisor 与面板;收益小。只写进文档(Task 9) |
| Crashed 页要等约 40 秒 | **有意不改**:退避 1/2/4/8/16 秒重启 5 次是 supervisor 的设计。只写进文档(Task 9) |
| `PanelEffect` 加 `#[non_exhaustive]` | **不在本计划**:会让所有现有穷尽 `match` 编译失败,等 digger 升完再单独做 |
| manifest 增加 `icon` 字段 | **不在本计划**:本次只做首字母徽标;自定义图标以后再加 manifest 字段并让徽标作为回落 |

---

## Task 1(bytehost): 重订阅退避真正逐档拉长

**Files:** Modify: `crates/bytehost-panel/src/host.rs`(`PanelMessage::SubscriptionLost` 分支,`Subscription` 枚举与 `subscribe_if_due`)

**Interfaces:** 无公开 API 变化。行为:`Subscription::Pending` 记住上一次失败的档位——把枚举改成 `Pending { prev_attempt: Option<usize> }`(`Idle` 之外所有构造点同步);`subscribe_if_due` 在 `Backoff { attempt, .. }` 到点时转成 `Pending { prev_attempt: Some(attempt) }`;`SubscriptionLost` 的档位 = `prev_attempt.map(|a| (a + 1).min(RESUBSCRIBE_BACKOFF.len() - 1))`,否则(从 `Live` 丢失,或 `Idle/Pending{None}`)为 0;`Subscribed` 复位(`Live` 不带档位)。

- [ ] **Step 1: 写失败测试** `resubscribe_backoff_escalates_through_the_real_retry_flow`:`live_state()`(已有辅助)→ 循环 5 轮:`SubscriptionLost` → 取 `Backoff` 的 `retry_at` → `subscribe_if_due(retry_at)`(断言返回 `[Subscribe]`)→ 下一轮。断言每轮 `retry_at - now` 依次等于 `RESUBSCRIBE_BACKOFF[0], [1], [2], [3], [3]`(封顶)。再加 `a_recovered_subscription_starts_the_backoff_over`:丢失 → 重订阅成功 `Subscribed` → 再丢失,断言第一档(2 秒)。
- [ ] **Step 2: 运行确认失败**:`cd crates/bytehost-panel && cargo test --lib -- resubscribe_backoff a_recovered_subscription` → 第二轮起的间隔仍是 2 秒。
- [ ] **Step 3: 实现**(同上 Interfaces);同步改所有 `Subscription::Pending` 的匹配与构造;既有测试 `losing_the_subscription_goes_back_to_fast_polling_and_retries_with_backoff` 若因形状变化需调整则保持其断言语义。
- [ ] **Step 4: 运行**:`cargo test --all-features && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --all --check` → 全绿。
- [ ] **Step 5: Commit**:`git commit -am "fix(panel): resubscribe backoff now escalates through the real retry flow"`

---

## Task 2(bytehost): 修偶发测试 `the_first_run_is_unavailable_when_the_port_cannot_be_saved`

**Files:** Modify: `crates/bytehost-apps/src/service.rs`(该测试,约 1415 行;测试辅助 `free_port()` 附近)

**Interfaces:** 新增测试辅助 `fn bind_eventually(port: u16, within: Duration) -> bool`:每 50ms 重试 `TcpListener::bind(("127.0.0.1", port))`,`within` 内任一次成功返回 `true`。

- [ ] **Step 1: 复现并记录**:`for i in $(seq 1 30); do cargo test -p bytehost-apps --all-features --lib -- --test-threads=16 service:: ; done`,记下失败次数(无需先写失败测试——这是测试本身的竞态,失败是不确定的;记录基线即可)。
- [ ] **Step 2: 改测试**:把 `assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(), "gateway 已释放端口")` 换成 `assert!(bind_eventually(port, Duration::from_secs(2)), "gateway 已释放端口")`;注释写明原因(`free_port()` 取到的临时端口在 gateway 释放后可能被并行测试抢走,单次绑定断言会偶发失败)。
- [ ] **Step 3: 验证**:重复 Step 1 的 30 次循环,**0 次失败**;再跑 `cargo test --all-features` 全绿。
- [ ] **Step 4: Commit**:`git commit -am "test(service): retry the port-released assertion; a parallel test can grab the ephemeral port"`

---

## Task 3(bytehost): 压缩包条目接受开头的 `./`

**Files:** Modify: `crates/bytehost-apps/src/archive.rs`(`safe_segments`,约 156-190 行;调用处约 225 行)

**Interfaces:** 新增 `fn strip_leading_dot_slashes(name: &str) -> &str`(私有):反复去掉开头的 `./`。条目名**恰好**是 `.` 或 `./`(或 `././`…)是根目录条目:调用处在进入 `safe_segments` 之前判断并**跳过**该条目(不报错、不计入条目数/大小)。

- [ ] **Step 1: 写失败测试**(该文件 `mod tests`,沿用现有构造归档的辅助):
  - `a_leading_dot_slash_is_stripped_from_entry_names`:`./a/b.txt` 解出 `a/b.txt`;`././a.txt` 解出 `a.txt`。
  - `root_dot_entries_are_skipped_not_rejected`:含 `./`、`.` 条目的 tar 能正常解出其余文件。
  - `dot_segments_elsewhere_and_traversal_are_still_rejected`:`a/./b`、`./../x`、`.//a`、`/a`、`..`、`a\\b` 各自得到 `UnsafeEntry`。
  - `a_macos_tar_with_dot_slash_entries_installs`:用 `tar -czf --no-mac-metadata` 风格的真实布局(顶层 `./` + `./manifest.toml` + `./server.py`)构造 tar.gz,装得上。
- [ ] **Step 2: 运行确认失败**:`cargo test --all-features --lib -- archive::tests::a_leading root_dot dot_segments a_macos` → 前两条因 `含 . 段` 失败。
- [ ] **Step 3: 实现**。
- [ ] **Step 4: 运行**:`cargo test --all-features && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --all --check` → 全绿。
- [ ] **Step 5: Commit**:`git commit -am "feat(archive): accept a leading ./ in entry names (root ./ entries are skipped)"`;同时把 `unfinished` 里"压缩包条目路径不接受开头的 ./"那条在 Task 9 中划掉。

---

## Task 4(bytehost): 发版 `v0.4.7`(需用户批准后执行)

- [ ] **Step 1:** `cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features && bash scripts/check-deps.sh` 全绿(整仓**跑两遍**,确认没有偶发)。
- [ ] **Step 2:** 给用户审阅;批准后合并 `main`;再次批准后 `git tag -a v0.4.7 -m "…" && git push origin main v0.4.7`,`git ls-remote --tags origin v0.4.7` 有输出。

---

## Task 5(byteui): 字符徽标图标入口

**Files:** Modify: `src/interaction/icons.rs`(把 `icon_button_entry` 的外壳抽成私有 `button_shell`;新增 `glyph_view`、`glyph_button_entry`、`readable_ink`);`examples/gallery.rs`(加一个徽标样例);`CHANGELOG.md`。

**Interfaces:**
- `pub fn readable_ink(fill: Color, light: Color, dark: Color) -> Color`:按 WCAG 对比度在 `light`/`dark` 里选与 `fill` 对比度更高的一个。
- `pub fn glyph_view<'a, M: 'a>(ch: char, size: f32, fill: Color, ink: Color) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>`:`size×size` 的圆角方块(圆角 `size * 0.25`),底色 `fill`,字符居中、字号 `size * 0.6`、颜色 `ink`、`Shaping::Advanced`、系统默认字体。
- `pub fn glyph_button_entry<'a, M: Clone + 'a>(ch: char, fill: Color, ink: Color, size: f32, active: bool, dim: bool, hover_t: f32, card: bool, button_size: f32, interactive: bool, on_select: M, on_hover: impl Fn(bool) -> M + 'a, tooltip: impl Into<Cow<'a, str>>) -> Element<'a, M, ..>`:与 `icon_button_entry` **共用同一个 `button_shell`**(选中金框、hover 渐变、拖拽 dim、卡片底、tooltip 全部一致);`dim` 时 `fill`/`ink` 经现有 `dimmed(..)` 压暗。
- 参数个数 > 7 且有相邻同类型(`Color, Color`、`bool × 4`):沿用现有 `icon_button_entry` 的位置参数风格**不改**(为与它保持调用形状一致);不引入参数结构体。

- [ ] **Step 1: 写失败测试**(同文件 `mod tests`,纯逻辑):`readable_ink_picks_the_higher_contrast_option`(白底选深色、黑底选浅色、中灰在两者之间选对比度更高的那个,断言选择结果的对比度 ≥ 另一个);`readable_ink_reaches_3_to_1_on_every_accent_token`(对 `ColorTokens::byteboy2077()` 与 `byteboy2077_light()` 的所有强调色 token〔`orange/magenta/blue/lime/teal/…`,以结构体里实际存在的为准〕,用各自的 `cream`/`bg` 作 `light`/`dark` 候选,断言对比度 ≥ 3.0)。
- [ ] **Step 2: 运行确认失败**:`cargo test readable_ink` → 函数未定义。
- [ ] **Step 3: 实现** `readable_ink`、`glyph_view`、`glyph_button_entry` 与 `button_shell` 抽取;`icon_button_entry` 行为逐字不变(调用 `button_shell`)。
- [ ] **Step 4: 运行**:`cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`;`cargo run --example gallery` 目测徽标的选中/hover/dim 三态(截图确认)。
- [ ] **Step 5: Commit**:`git commit -am "feat(icons): glyph badge entry sharing the icon button shell"`。

---

## Task 6(byteui): 发版 `v0.4.5`(需用户批准后执行)

- [ ] **Step 1:** `cargo test && cargo clippy --all-targets -- -D warnings`;`CHANGELOG.md` 补 `0.4.5` 一节(**明确写出**其中包含本地早已存在的未发布提交 `3677749 feat(theme): teal 语义色 token`)。
- [ ] **Step 2:** **向用户说明这次发版会顺带发布别人的 `teal` 提交**(它解除 dozer 本地另外两个提交的推送阻塞),批准后合并、`git tag -a v0.4.5`、`git push origin main v0.4.5`,`git ls-remote --tags origin v0.4.5` 核对。

---

## Task 7(dozer): 应用图标栏使用首字母徽标

**Files:** Create: `crates/dozer-app/src/chrome/app_badge.rs`;Modify: `crates/dozer-app/src/chrome.rs`(声明模块)、`crates/dozer-app/src/chrome/rail.rs`(`panel_meta` 约 623 行;循环约 473 行;拖拽幽灵约 717 行;测试约 1497-1511 行);各 `crates/*/Cargo.toml` 的 byteui tag(→ `v0.4.5`)与 bytehost tag(→ `v0.4.7`);`Cargo.lock`;`CLAUDE.md` 的 bytehost 一行。

**Interfaces:**
- Produces(`app_badge.rs`,纯函数):
  - `pub(crate) fn badge_glyph(title: &str) -> char`:第一个非空白字符;ASCII 字母转大写;全空返回 `'?'`。
  - `pub(crate) fn badge_fill(tokens: &ColorTokens, app_id: &str) -> Color`:FNV-1a 32 位哈希 `app_id` 的字节,对**调色板**(`tokens` 里的强调色:`orange/magenta/blue/lime/teal`,以 Task 5 测试用的集合为准)取模。
  - `pub(crate) fn badge_ink(tokens: &ColorTokens, fill: Color) -> Color` = `byteui::interaction::icons::readable_ink(fill, tokens.cream, tokens.bg)`。
- `rail.rs`:新增 `enum RailVisual { Icon(IconKind), Badge { glyph: char, fill: Color, ink: Color } }` 与 `fn rail_visual(kind, locale, app_title) -> (RailVisual, String /*tooltip*/)`;应用面板返回 `Badge`(字符取自**当前语言下的标题**,无标题回落应用 id),其余面板返回 `Icon(descriptor.icon)`;图标栏循环与拖拽幽灵按 `RailVisual` 分别调 `icons::icon_button_entry` / `icons::glyph_button_entry`。`panel_meta` 对非应用面板保持原样(其余调用点与测试不动)。

- [ ] **Step 1: 升依赖**:byteui `v0.4.4 → v0.4.5`、bytehost `v0.4.6 → v0.4.7`(两个 tag 必须已在远程);`cargo update -p byteui -p bytehost-…`;审阅 lock 变动。
- [ ] **Step 2: 写失败测试**(`app_badge.rs` 的 `mod tests`):`glyph_rules`(`"Py Notes"→'P'`、`"双语实时"→'双'`、`"  node"→'N'`、`""→'?'`、`"12"→'1'`);`the_same_app_id_always_gets_the_same_fill`(调用两次相等);`different_ids_spread_over_the_palette`(10 个不同 id 至少映射到 3 种底色);`ink_is_readable_on_every_fill_in_both_themes`(两套 token × 全调色板,`badge_ink` 与底色对比度 ≥ 3.0)。`rail.rs` 的测试:`an_app_panel_gets_a_badge_not_the_generic_list_icon`(`rail_visual(PanelKind::App(a), "zh-CN", |_| Some("应用X".into()))` 得到 `Badge { glyph: '应', .. }`,tooltip 是 `应用X`;无标题时字符取 id 首字母);`builtin_panels_keep_their_icons`(`Files` 仍是原 `IconKind`)。
- [ ] **Step 3: 运行确认失败**:`cargo test -p dozer-app chrome::app_badge chrome::rail` → 未定义。
- [ ] **Step 4: 实现**;原来断言"应用面板图标是 `LayoutList`"的旧测试按新行为更新并在提交说明里点名。
- [ ] **Step 5: 全量验证**:`cargo build && cargo test -p dozer-app -p dozerd -p dozer-client && cargo clippy -p dozer-app --all-targets`(触及文件无新告警);**目视**(隔离环境,`orca computer`):装 `py-notes`、`py-crashy`、`bilingual-live` 三个应用,图标栏出现 `P`、`P`、`双`(前两个底色相同则因 id 不同应不同——若碰巧相同,换样例 id 再看)徽标,选中/hover 态正常;截图留证。
- [ ] **Step 6: Commit**:`git commit -am "feat(rail): installed apps get a first-letter badge instead of the generic list icon"`。

---

## Task 8(dozer): 运行时区域按钮不再被挤成两行

**Files:** Modify: `crates/dozer-app/src/extensions/settings_apps_view.rs`(`runtime_row_view`,约 145-205 行)

**Interfaces:** 新增纯函数 `fn runtime_actions(probe: &RuntimeProbe, runtime: Option<ManagedRuntime>, uninstalling: &dyn Fn(ManagedRuntime, &str) -> bool) -> Vec<RuntimeAction>`,`enum RuntimeAction { InstallRuntime(ManagedRuntime), UninstallRuntime(ManagedRuntime, String), Processing(String) }`(类型名以 `bytehost_panel::install` 里 `managed_runtime_of` 实际返回的为准)。布局:第一层 = 运行时名 + 状态文字(可换行);第二层 = 动作按钮,放进 `row![..].spacing(8).wrap()`(与 `app_row` 同一做法)。

- [ ] **Step 1: 写失败测试** `an_installable_runtime_with_no_managed_version_offers_install`、`a_managed_version_offers_uninstall_and_shows_processing_while_uninstalling`、`a_probe_without_a_known_runtime_offers_nothing`(纯逻辑,断言返回的动作序列)。
- [ ] **Step 2: 运行确认失败** → 未定义。
- [ ] **Step 3: 实现** `runtime_actions` 并把 `runtime_row_view` 改成两层;`probe_is_installing` 分支保持原样(进度文字 + 空白,无按钮)。
- [ ] **Step 4: 运行**:`cargo test -p dozer-app settings_apps_view` → 通过。
- [ ] **Step 5: 目视**:设置 → 应用,`Python` 与 `Node.js` 两行的「安装…」按钮完整显示在单行内(截图)。
- [ ] **Step 6: Commit**:`git commit -am "fix(settings-apps): runtime rows put their buttons on a wrapping row so Install… is not squeezed"`。

---

## Task 8b(dozer): 修偶发测试 `git_log::tests::build_marks_head_branch_and_labels`

**Files:** Modify: `crates/dozer-app/src/extensions/git_log.rs`(该测试,约 1912-1935 行)

**Interfaces:** 无新增。该测试读 dozer 自己的 git 仓库,原先断言"历史第 0 行的 `refs` 含当前分支";当 `HEAD` 落后于仓库里最新提交时(例如落后于 `main` 的 worktree)第 0 行是别的提交,断言稳定失败。改为:在 `snapshot.rows` 里**找到 `refs` 含当前分支(`RefKind::LocalBranch`)的那一行**并断言存在(窗口 50 行,HEAD 总在最新提交附近)。同文件的 `commit_detail_uses_first_parent_diff` 也假设第 0 行是 HEAD,但它对任意有 parent 的提交都成立,不动。

- [ ] **Step 1: 复现**:`git worktree add ../dozer-flaky -b tmp/flaky <比 main 旧的某个提交>`,在其中 `cargo test -p dozer-app extensions::git_log::tests::build_marks_head_branch_and_labels` → 稳定失败("HEAD 所在行的 refs 应包含当前分支")。(这一步是"先红":失败已由此复现。)
- [ ] **Step 2: 改测试**(同上 Interfaces),断言信息保留原文并补一句"在前 50 行里没找到带当前分支的提交"。
- [ ] **Step 3: 验证**:在那个落后的 worktree 里该测试**通过**;回到主目录再跑一遍 `cargo test -p dozer-app extensions::git_log` 全过;`git worktree remove ../dozer-flaky && git branch -D tmp/flaky` 清掉临时 worktree。
- [ ] **Step 4: Commit**:`git commit -am "test(git_log): find HEAD's branch label on any row instead of assuming row 0 is HEAD"`。

---

## Task 9(dozer): 应用进程重启后统一重载页面(先调查,再按结论修)

**背景:** 验收 2.3/2.4:crash 恢复后页面仍是崩溃前内容(`hits` 未变、SSE 已断);hang 恢复后页面重载了。`ClearUrl` 只有在某次列表快照**观察到**应用不是 `Running` 时才由状态机发出(`bytehost-panel/src/host.rs` 约 497-505 行),随后 `AppViews::clear` 让池回收 webview,`SetUrl` 再建新的。

**Files(预期,以调查结论为准):** `crates/bytehost-panel/src/host.rs`、`crates/dozer-app/src/app_webview.rs`、`crates/dozer-app/src/webview_geometry.rs` / `runtime.rs`(webview 池按 URL 对账处)。

- [ ] **Step 1: 状态机层复现**(`bytehost-panel`):写测试 `a_restart_seen_as_starting_clears_then_refetches_the_url`:`Running` → 列表 `Starting` → 列表 `Running`,断言依次出现 `ClearUrl` 与 `FetchLaunchUrl`。**若失败 → 修状态机**(Step 3a);**若通过 → Step 2**。
- [ ] **Step 2: 判断是"没观察到"还是"观察到了但没重载"**:在隔离 GUI 里复现 crash(`/crash`)并在 `dozer-app.log` 里看是否出现状态机的 `Starting` 观察(用 `RUST_LOG=dozer::module::app_host=debug` 起 GUI;必要时先给 `apply_effects` 的 `ClearUrl`/`SetUrl` 临时加 debug 日志)。
  - **(a) 没观察到 `Starting`**(重启快于列表刷新):状态机没有重启信号。修法 3a:在 `AppSummary` 里**没有**可用的重启标志(wire 不能改),所以改用**已有信息**——`ListLoaded` 时对每个 `Running` 应用拿到的 `url`/`launch` 不变,无法区分。此情形下的可行修法是 `Changed` 推送到来时(`StateChanged`)强制把该应用当作"需要重取启动地址并让 webview 重载":在 `PanelEffect` 追加 `ReloadUrl(K)`(**新增变体,需要 bytehost 再发版并提醒 digger**),由 dozer 适配层调用 webview `reload`。**若走到这一步,先停下向用户确认**(新增效果变体 + 再发一次版)。
  - **(b) 观察到了 `Starting`,`ClearUrl` 也发了,但页面没重载**:根因在 dozer 的 webview 池对账(新旧 URL 字符串相同被判定"无变化")。修法 3b:`AppViews::clear` 同时记一个递增的 `generation`,池对账以 `(url, generation)` 为键,代次变化即重建。
- [ ] **Step 3a/3b: 实现与测试**:3b 的测试 `clearing_and_setting_the_same_url_forces_a_new_webview_generation`(`AppViews` 纯逻辑:`set_url(a, u)` → 取代次 `g1` → `clear(a)` → `set_url(a, u)` → 代次 `g2 > g1`);池对账处按代次比较的测试放在对账函数旁(若对账逻辑嵌在 `App` 里,抽成纯函数 `needs_rebuild(current: Option<(&str, u64)>, desired: Option<(&str, u64)>) -> bool` 再测)。
- [ ] **Step 4: 真 GUI 回归**:隔离环境装 `py-notes` → 打开面板记下 `hits` → 触发 `/crash` → 恢复后 `hits` **比之前大 1**且 SSE 的 `tick` 重新滚动;再 `/hang`(等约 35 秒恢复)同样验证;各截图一张。
- [ ] **Step 5: Commit**:`git commit -am "fix(app-host): reload the app page after any restart (crash path kept the stale page)"`。

---

## Task 10(dozer): 文档回写

**Files:** Modify: `docs/superpowers/unfinished/2026-10-16-bytehost-remaining.md`、`docs/superpowers/manual_tests/2026-10-16-bytehost-manual-acceptance.md`、bytehost 仓库五份验收报告(`docs/superpowers/specs/2026-10-0*-bytehost-a5/a6e/a6f/a6g/a6h-acceptance-report.md`)、`docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md` §7 的 A5 行。

- [ ] **Step 1: `unfinished` 文档**:开头的版本信息改成现状(bytehost `v0.4.7`、dozer/digger 的依赖版本以实际为准);§1 改成"手工验收的真实进度"(做了什么、没做什么,引用手测文档);**划掉**已完成的:重订阅退避、两个偶发测试、压缩包 `./`、图标区分度、重启后重载、运行时按钮;**新增**"有意不改"两条:hang 无提示(原因同本计划范围说明)、Crashed 页等待约 40 秒(退避 1/2/4/8/16 秒重启 5 次属 supervisor 设计)。
- [ ] **Step 2: 手测文档**:把对应的"发现/缺陷"行标已修复并写提交号。
- [ ] **Step 3: 五份验收报告**:把"待人工在真实 GUI 里完成"清单里已验的项勾上并注明"2026-10-17 手工验收,见 dozer `manual_tests/…`";WebSocket(A6e 的 4 号)标注"原先用裸客户端验证,真 WKWebView 下握手失败,`v0.4.5` 修复";仍未做的项保持未勾并注明原因(Excalidraw 需联网打包、网络来源需 https、双窗口需分离窗口)。`design.md` §7 的 A5 行同步现状(**五个切片仍不标"已完成"**,因为仍有未做项)。
- [ ] **Step 4: H8 评估**:在 `docs/dozer-v2/bytehost-H8-evaluation.md` 记下 Q-H8-2 的事实结论(Digger 已在复用 Usage 面板,`libs-2026-10-16c`),Q-H8-3 保持待裁决。
- [ ] **Step 5: Commit**(bytehost 与 dozer 各一个文档提交):`git commit -am "docs: record the 2026-10-17 acceptance results and refresh remaining-work status"`。

---

## Self-Review

- **Spec 覆盖:** A 组(退避→T1、偶发测试→T2 与 T8b、`./`→T3、文档回写→T10),B 组(图标→T5–T7、hang/Crashed 等待→范围说明与 T10、重载→T9、Python 按钮→T8)。dozer 的偶发测试对应 Task 8b。
- **类型一致:** `badge_glyph/badge_fill/badge_ink`(T7)消费 `readable_ink`(T5);`RailVisual` 只在 `rail.rs` 内使用;`runtime_actions`/`RuntimeAction`(T8)与 `app_row` 的 `row_actions` 同风格;T9 的 `needs_rebuild` 仅在 3b 路径出现。
- **风险点:** T9 的 3a 路径需要新增 `PanelEffect` 变体并再发版,计划里要求走到那一步先停下确认;T6 发版会顺带发布 `teal` 提交,已要求明确告知;T7 依赖 T4、T6 两个 tag 已在远程。

## 决策记录

(待填:各 tag 的发布时间、T9 走了 3a 还是 3b、提交号。)
