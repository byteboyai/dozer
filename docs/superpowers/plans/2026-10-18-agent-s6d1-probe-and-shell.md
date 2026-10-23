# S6d-1 探针与骨架:键位转换、输入法探针、字体、Agent 面板壳 Implementation Plan

> **迁移说明(2026-10-20):** 本文原在 Digger 仓库,现移入 Dozer 仓库;文内 `docs/superpowers/...` 路径均相对 **Dozer 仓库根**。写作「Digger(仓库 `../digger`)」的部分,实施仍在 Digger 仓库的 worktree 里进行,但文档不在那里。Digger 自有文档(`docs/adr/*`、`2026-10-14-window-shell-design.md`、`docs/superpowers/notes/`)仍在 Digger 仓库。最新要求与决定见 `docs/superpowers/specs/2026-10-18-agent-panel-s6-design.md` §0。

> **2026-10-23 修订(Dozer 侧验收通过后,以此为准,与下文及其它顶部修订冲突处以本条为准):**
> - **Dozer 侧链路已验收、合并 `main`、推送。** 库快照 tag 是 **`libs-2026-10-23a`**(上一个是 `libs-2026-10-20a`);本文及其它修订里"尚未验收/未合并/没有 `libs-*` tag""不合并 `main`、不打 tag、不推送""用不提交的本地 `[patch]`""占位 tag `libs-2026-10-20a`""`Cargo.lock` 不提交/没有 `--locked` 可验"一律作废:依赖直接钉 tag、**提交 `Cargo.lock`**、`cargo metadata --locked` 与 `cargo tree -d` 门禁照常跑。打 tag/推送仍是对外动作,逐次向用户确认。
> - **`reorder_term_tab` 回到库里,且签名变了:** `dozer_panel_agent::update::reorder_term_tab(state, from, to) -> bool`(显式来源下标,越界或 `from == to` 返回 `false`;`active` 始终指向同一个会话)。库里**不再有** `Message::TabDragMove` / `TabDragEnd`;`PanelDisplay::drag_surface(&self, content, index)` 只有两个参数(没有 `on_move`)。**含这个函数的 tag 是 `libs-2026-10-23b`(待切,见 Dozer 侧提交);在它存在之前不要开始依赖它的任务(S6d-3 Task 3)。** 拖拽手势(按下武装、位移阈值、组校验)全部由产品的拖拽状态机负责,库只做换位。
> - **opencode 验收要覆盖 2.x:** opencode 2.x 拒绝具名导出的插件。Dozer 的插件模板已改成 `export default { id, setup, server }`(`c15804ad`),Digger 的 `digger.ts` 由同一模板生成,插件 id 是 `digger`。验收时在 opencode 2.x 下确认:`opencode` 日志里没有 `failed to load plugin`,状态点随回合更新。
> - **hook 对旧守护进程的取舍已付过一次代价:** 新版 hook 遇到不认识 `Hello` 的旧守护进程会放弃上报(S6a 的刻意取舍),Dozer 因此出过"卡片不更新"。Dozer 的配套是 `Client::daemon_is_outdated()` + 顶栏"需重启 dozerd"徽标(`c016794b`)。Digger 首发不会遇到(`diggerd` 是新进程),但以后升级会;如要复用,在 S6d-3 的顶栏里接同一个标记。


> **2026-10-21 修订(审核方):** 按总设计 §0 第 9 条调整执行范围。Dozer 侧链路(S6a Part A → … → S6e-1,顶端 `feat/mcp-product` `faf24115`)已全部开发并审核通过,但**尚未验收、未合并、没有 `libs-*` tag**。本期是 Digger 侧第一份被执行的计划,规则如下:
> - **依赖用不提交的本地 `[patch]`,不得合并 `main`、不打 tag、不推送。** 在 worktree 里建 `.cargo/config.toml`(已被 `.gitignore` 忽略),照 `scripts/dev-patch.example.toml` 的写法,把 `[patch."https://github.com/byteboyai/dozer"]` 指向 `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-mcp-product/crates/<crate>`。**Digger 依赖的每一个 dozer crate 都要一起指**(`dozer-model`、`dozer-services`、`dozer-panel-{usage,project,files,preview,host,term}`、`dozer-preview-core`、`dozer-store-project`,以及本期新增的 `dozer-panel-term`),否则依赖图里会出现两份 `dozer-model`、类型对不上。bytehost `v0.4.11`、byteui `v0.4.5`、bytegit `v0.8.1` 与 Dozer 该分支一致(已核对),不需要 patch。
> - **`Cargo.toml` 里新增的 dozer 库依赖暂用现有的 `libs-2026-10-20a` 作占位 tag**(该 tag 里没有 `dozer-panel-term`,只有 patch 在时才能解析);**`Cargo.lock` 不提交**(patch 会把它写成本地路径,`pre-push` 也会拦)。这意味着本期**没有** `cargo metadata --locked` 可验,计划里涉及 `--locked` 与 Cargo.lock 重复检查的步骤改为:在带 patch 的 worktree 里 `cargo tree -d` 确认没有重复的 `dozer-*` / `byteui` / `bytehost-*`,并跑 `scripts/check-host-deps.sh`、`scripts/check-shared-deps.sh`。真正升 tag 与提交 Cargo.lock 留到验收后切了 tag 的那一天。
> - **前置修正:** 本文原写"S6a 已合并(`diggerd`、`daemon::Daemon`)"不成立——Digger 里 S6a Part C 完全没做(没有 `diggerd`、`digger-hook`、`daemon.rs`)。因此 **Task 4 拆成 4a 与 4b**:本期只做 **4a**(面板登记、布局与旧布局迁移、配对尺寸、`AgentShell` 状态机与视图,`ensure` 由调用方注入,测试用假实现);**4b**(把 `AgentShell` 接到真实的 `Daemon::ensure` 与 `boot`)**待 S6a Part C 完成后另行追加**,本期不做、不造假的 `Daemon`。
> - **输入法探针(Task 2 Step 4)不再阻塞:** 总设计 §0 第 9 条把人工验收推迟,所以实现者做完探针程序后,把"需要真机做的 5 项"写进探针笔记和报告,**不等待用户**,整条链路验收时由用户一并做。探针结论是 **S6d-2 Part B 里 Task 5(键盘/输入法)的前置**:在 `docs/superpowers/notes/2026-10-18-ime-probe.md` 里记录结果之前,不要开始那一部分(与本期无关的其余 Task 不受影响)。
> - **分支:** worktree `../digger-s6d1`、分支 `feat/agent-shell`,从 Digger `main` 当前顶端 `27f40e3` 切出。开工先在 `/Users/chrischiang/Projects/CoralProjects/byteboy/digger` 里 `git rev-parse --short main`,必须是 `27f40e3`,不是就停下来问;然后 `git worktree add ../digger-s6d1 -b feat/agent-shell main`。**不要碰** Digger 主工作区里别人未提交的改动(`Cargo.lock`、`crates/desktop/Cargo.toml`、`Info.plist` 的修改,以及 13 个 S6 文档的未提交删除),也不要动 `feat/preview-pairing`(worktree `digger-p4`)。
> - **基线(`27f40e3`,干净快照实测):** `cargo test --workspace --no-fail-fast` = **366 通过 / 0 失败 / 0 ignored**(`digger` 桌面 bin 226、`digger_app_host` 39、`digger_editor` 35、`digger_store` 11,余为其它目标)。clippy 警告数开工时自己记录,完工不得更多。
> - **实现者须知(前几期审核里真踩过的坑):** ① 新增文件(尤其 `assets/fonts/*`、`examples/*`)提交后用 `git status --ignored` 核对没被 `.gitignore` 吞掉;② 完工前把 `git archive HEAD` 解到**独立临时目录**,在该目录自己的 `target` 下重跑测试——但该快照没有 `.cargo/config.toml`,所以要把你的 patch 配置**拷一份**进快照再跑,并在报告里写明;③ `cargo fmt --check` 若在 `main` 上本就失败,只要求自己改的文件干净(`rustfmt --check --edition 2024` 逐个文件);④ 报告里每条「通过」必须是**最终代码**上刚实跑的,附命令与退出码,须含测试总数(基线 + 新增)、测试名守恒、clippy 实测数,审核人会逐条重跑;基线对不上、库缺东西或计划与代码不符时先停下来问,不要自己绕过;⑤ Dozer 的键位字节表是"抄值不自己推",抄的来源写进测试注释;⑥ 报告写到 Digger worktree 的 `docs/superpowers/reviews/2026-10-<日期>-s6d1-report.md`(Digger 仓库内;本计划文件在 Dozer 仓库,实现者读它但不修改它)。


> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Digger 右栏最前出现 Agent 面板的**壳**(配对布局、登记、旧布局迁移、`diggerd` 不可用态),并把后面两期依赖的三块底座先做实:iced 键盘事件 → 终端字节的转换、macOS 中文输入法的送达探针(`ImeSink`)、终端字体与 `TermStyle`。**本期不建会话、不连终端**,那是 S6d-2。

**Architecture:** 全部在 `digger-desktop`(宿主适配层)里。`adapters/agent_keys.rs` 把 `iced::keyboard::{Key, Modifiers}` 转成 `dozer_panel_term::input::{TermKey, TermMods}`;`adapters/agent_ime.rs` 的 `ImeSink` 是一个包住任意 `Element` 的薄控件,在 `Widget::update` 里处理 `Event::InputMethod` 并 `shell.request_input_method(..)`;`fonts.rs` 注册内嵌 JetBrains Mono 并产出 `TermStyle`;`shell.rs` / `product.rs` 登记 `digger.agent`。输入法是否真能送达只能由人在真机上验证,所以本期有一个**人工验收门**,探针失败则停下报告,不自行选退路。

**Tech Stack:** Rust、iced 0.14(`iced_core::input_method`、`Widget`)、`dozer-panel-agent` / `dozer-panel-term`(Dozer `libs-*` tag)、byteui、bytehost-chrome。

**Spec:** `docs/superpowers/specs/2026-10-18-agent-panel-s6d-design.md`(§6.1、§6.4、§6.5、§7 S6d-1、§9 Q1–Q3)。已批准的取舍:Q1 新用户默认选中 Agent;Q2 先做探针再定退路;Q3 卡片文字回落 transcript 最后一句。

## Global Constraints

- **前置:** S5a 已合并(左栏有 Project、`project_id()` 与旧布局迁移的做法已存在)。**S6a Part C 未做**(见顶部修订,Task 4b 才需要它);Dozer 侧库(含 `dozer-panel-term`、`dozer-panel-agent`)在**未合并**的叠分支上、**尚无 tag**,本期经本地 `[patch]` 使用,见顶部修订。
- 依赖方向:`dozer-panel-agent`、`dozer-panel-term` 只允许 `digger-desktop` 依赖;`scripts/check-host-deps.sh`、`scripts/check-shared-deps.sh`、Cargo.lock 重复检查必须通过(同一 crate 一个版本)。
- **不改 Dozer 与库**;库缺什么就停下报告。
- **已有用户的 `shell.json` 不被重置:** 旧文件里没有 `digger.agent` → 回默认栏并移到右栏最前,**该用户右栏当前选中的面板不变**;左栏不动。新用户(无文件)右栏默认选中 Agent(Q1)。
- 面板 id `digger.agent` 是落盘的线上名,一经发布不得改。
- 主工作区有别人未提交的改动(`Cargo.lock`、`crates/desktop/Cargo.toml`、`Info.plist`,以及 13 个 S6 文档的未提交删除),**不动**;实现在独立 worktree `../digger-s6d1`、分支 `feat/agent-shell`(从 `main` 的 `27f40e3` 切出)。**不合并、不打 tag、不推送。**
- 不引入新的日志依赖。字体文件用 SIL OFL 的 JetBrains Mono,随仓库附许可证文本。
- 终端字体度量与 Dozer 一致:`font_size = 14.0`、`line_height_factor = 1.2`(Dozer `assets/theme/terminal.json`)。

## Review Focus

1. **按键进错地方或丢失:** 项目面板/编辑器的输入框聚焦时按键不得进终端;⌘ 组合键绝不进 PTY;修饰键单独按下、死键、组合键中间态不产生字节。→ Task 1。
2. **输入法状态残留:** 预编辑被取消、焦点离开、面板被切走后,候选框与预编辑文字必须清除;提交文本为空串、含换行、含超长文本时不 panic。→ Task 2。
3. **旧布局迁移的边界:** 旧文件只有一侧、右栏为空、右栏已含 `digger.agent`(重复迁移)、更高 schema 版本(不得回写);迁移幂等。→ Task 4。
4. **配对面板的退化尺寸:** 右区窄到 `min_first_px` 以下时列表自动收起,终端块宽/高算出 0 或负数时不 panic、网格至少 1×1。→ Task 4。
5. **`diggerd` 不可用时的体验:** 面板显示可读原因与「重试」,重试成功后面板正常;重复点击不并发多个 `ensure`;主窗口其余面板不受影响。→ Task 4。

---

## Digger(仓库 `../digger`,worktree `../digger-s6d1`,分支 `feat/agent-shell`)

### Task 1: 键位转换 `adapters/agent_keys.rs`

**Files:**
- Create:`crates/desktop/src/adapters/agent_keys.rs`
- Modify:`crates/desktop/src/adapters/mod.rs`、`crates/desktop/Cargo.toml`(加 `dozer-panel-term`)、workspace `Cargo.toml`(升 dozer libs tag、加 `dozer-panel-term`、`dozer-panel-agent` 的 workspace 依赖)、`Cargo.lock`
- Test:同文件

**Interfaces:**
- Produces:
  - `agent_keys::to_term_key(key: &iced::keyboard::Key) -> Option<TermKey>`:`Key::Character(s)` → `TermKey::Char(s)`;`Key::Named(n)` → 对应 `TermNamedKey`,**只有库里实际存在的 9 个**:`Space`、`Enter`、`Backspace`、`Tab`、`Escape`、`ArrowUp`、`ArrowDown`、`ArrowRight`、`ArrowLeft`(见 `dozer-panel-term/src/input.rs`;库是从 Dozer 旧 `keymap.rs` 原样搬来的,"no more, no less");**其余具名键一律 `None`**,包括 `Home`、`End`、`PageUp`、`PageDown`、`Delete`、`Insert`、`F1`–`F12`(Dozer 现状同样不把它们送进 PTY,本期要的是与 Dozer 一致,不是新增行为)以及 `Shift`、`Control`、`Alt`、`Super`、`CapsLock`、`Dead`、`Key::Unidentified`。
  - `agent_keys::to_term_mods(m: iced::keyboard::Modifiers) -> TermMods`(`ctrl = m.control()`、`alt = m.alt()`、`shift = m.shift()`、`super_key = m.logo()`)。
  - `agent_keys::key_bytes(key: &Key, mods: Modifiers, app_cursor: bool) -> Option<Vec<u8>>`:`mods.logo()` 为真恒 `None`(⌘ 组合键不进 PTY,应用快捷键另行处理);否则 `to_term_key` + `dozer_panel_term::input::key_to_bytes`。
  - `agent_keys::AppShortcut { Copy, Paste }` 与 `agent_keys::shortcut(key: &Key, mods: Modifiers) -> Option<AppShortcut>`(⌘C → `Copy`,⌘V → `Paste`,其余 `None`;规则对照 Dozer `platform/window_events.rs` 约 929–1010 行,实现者把对照结论写在文件头注释里)。
- Consumes:`dozer_panel_term::input::{TermKey, TermNamedKey, TermMods, key_to_bytes}`(S6c-1 Task 2)。

- [ ] **Step 1: 建 worktree 并接本地 patch。** 按顶部修订建 worktree 与 `.cargo/config.toml`(patch 全部 dozer crate 指向 `../dozer-mcp-product`)。workspace `Cargo.toml` 新增 `dozer-panel-term`(占位 tag `libs-2026-10-20a`,见顶部修订);`dozer-panel-agent` 本期只在 Task 4 实际引用其类型时才加,否则不加。**不改现有 dozer libs 的 tag,不提交 `Cargo.lock`。** 跑 `cargo tree -d` 确认没有重复的 `dozer-*`/`byteui`/`bytehost-*`,再跑 `scripts/check-host-deps.sh` 与 `scripts/check-shared-deps.sh`。
- [ ] **Step 2: 写失败测试**(同文件):
  - `byte_matrix_matches_dozers_known_outputs`:表驱动,期望字节取自 Dozer `dozer-panel-term::input` 的测试(实现者从该库的 `key_to_bytes` 测试逐条抄值,**不自己推**):字符键 `a`、`A`(shift)、空格、回车(`\r`)、退格(`0x7f`)、Tab、Esc、Ctrl+A..Z(`0x01..0x1a`)、Ctrl+C(`0x03`)、Alt+字符(`ESC` 前缀)、方向键在 `app_cursor` 真/假下(`ESC [ A` / `ESC O A`);
  - `command_combinations_never_reach_the_pty`(⌘+任意字符/具名键 `key_bytes` 恒 `None`);
  - `keys_outside_the_legacy_table_produce_nothing`(`Home`、`End`、`PageUp`、`PageDown`、`Delete`、`Insert`、`F1`–`F12` 经 `key_bytes` 恒 `None`,与 Dozer 现状一致;注释注明"若将来要支持,须先在 `dozer-panel-term` 里加 `TermNamedKey` 变体,Dozer 与 Digger 同时获得");
  - `modifier_only_and_dead_keys_produce_nothing`(`Shift`、`Control`、`Alt`、`Super`、`CapsLock`、`Dead`、`Unidentified`);
  - `every_term_named_key_is_reachable_from_an_iced_named_key`(遍历 `TermNamedKey` 的全部变体,断言存在一个 iced `Named` 能映射到它,防止新增变体漏接);
  - `shortcut_recognizes_command_c_and_v_only`(含大小写、带 shift 的 ⌘⇧C 不算复制);
  - `multi_char_character_keys_are_passed_through_unchanged`(输入法提交之外的、`Key::Character` 含多个 Unicode 标量时原样转成 `Char`)。
- [ ] **Step 3: 运行确认失败**,然后实现(纯函数,不碰全局状态)。
- [ ] **Step 4: 验证。** `cargo test -p digger-desktop agent_keys`。
- [ ] **Step 5: 提交** `feat(agent): iced 键位到终端字节的转换`。

### Task 2: 输入法探针 `ImeSink`

**Files:**
- Create:`crates/desktop/src/adapters/agent_ime.rs`、`crates/desktop/examples/ime_probe.rs`(探针程序,不进产品)
- Modify:`crates/desktop/src/adapters/mod.rs`
- Test:`agent_ime.rs`

**Interfaces:**
- Produces:
  - `agent_ime::ImeState { preedit: Option<String>, active: bool }`(`Default`),方法 `on_event(&mut self, ev: &input_method::Event) -> Option<String>`:`Preedit(text, _)` 空串 → `preedit = None`、非空 → `Some(text)`,返回 `None`;`Commit(text)` → 清预编辑,**非空**时返回 `Some(text)`(调用方写入会话),空串返回 `None`;`Opened` → `active = true`;`Closed` → `active = false` 且清预编辑。
  - `agent_ime::ImeSink<'a, M>`:`ImeSink::new(content: Element<'a, M>, enabled: bool, cursor: impl Fn(Rectangle) -> Rectangle + 'a, on_commit: impl Fn(String) -> M + 'a, on_preedit: impl Fn(Option<String>) -> M + 'a) -> Self`,实现 `Into<Element<'a, M>>`。`enabled` 为真时,在每次 `RedrawRequested` 里 `shell.request_input_method(&InputMethod::Enabled { cursor: cursor(layout.bounds()), purpose: Purpose::Terminal, preedit })`;收到 `Event::InputMethod` 时通过 `ImeState` 处理并 `shell.publish(..)`;`enabled` 为假时不请求(输入法关闭)。`draw`、`layout`、`mouse_interaction`、`operate`、`overlay` 全部原样转给 `content`。
- Consumes:`iced_core::{input_method, Widget, Shell}`。

- [ ] **Step 1: 写失败测试**(状态机,纯逻辑):`preedit_update_replaces_the_previous_preedit`、`commit_returns_the_text_and_clears_the_preedit`、`empty_commit_returns_none_and_still_clears_the_preedit`、`commit_with_newlines_and_a_100k_char_string_is_returned_whole_without_panicking`、`closed_clears_everything`、`preedit_then_losing_enabled_is_cleared_by_the_sink`(`ImeSink` 的 `enabled` 由真变假时下一次事件处理清掉预编辑——用对 `ImeState` 的显式 `reset()` 调用固定)。
- [ ] **Step 2: 运行确认失败**,然后实现状态机与 `ImeSink`。`ImeSink` 的 `Widget::update` 先处理自己的事件,再调用 `content.update`(内容仍要收到鼠标/键盘事件)。
- [ ] **Step 3: 写探针程序 `examples/ime_probe.rs`:** 一个最小 `iced::application`:中间是一块深色矩形(代表终端格,固定光标格矩形),`ImeSink` 包住它,窗口里用等宽文本显示「已提交文本」与「当前预编辑」,另有一行状态显示最近的 `Opened/Closed` 事件。点击矩形获得焦点(`enabled = true`),点别处取消(`false`)。
- [ ] **Step 4: 真机探针清单(不阻塞,见顶部修订;用户在整条链路验收时做)。** `cargo run -p digger-desktop --example ime_probe`,依次验证:① 切到系统中文拼音输入法,输入 `nihao` 看到候选框出现在光标格附近、预编辑文字出现;② 回车/空格选词后「已提交文本」得到汉字;③ 输入中途 Esc 取消,预编辑清空;④ 点击矩形外,候选框消失;⑤ 连续快速输入一整句。实现者先用 `cargo run --example ime_probe` 确认程序能启动、能显示,把这 5 项做成待填表写进 `docs/superpowers/notes/2026-10-18-ime-probe.md`(结果列留空,标明"待用户真机填写")。**探针结果出现失败时,不选退路**(退路是用户的决定,见设计 Q2),报告给用户;在结果填入之前,S6d-2 Part B 的键盘/输入法 Task 不得开始。
- [ ] **Step 5: 验证与提交。** `cargo test -p digger-desktop agent_ime`;提交 `feat(agent): ImeSink 与输入法探针`(探针结论文档一并提交)。

### Task 3: 字体与 `TermStyle`

**Files:**
- Create:`crates/desktop/src/fonts.rs`、`crates/desktop/assets/fonts/JetBrainsMono[wght].ttf`(从 Dozer `crates/dozer-app/assets/fonts/` 拷贝)、`crates/desktop/assets/fonts/OFL.txt`
- Modify:`crates/desktop/src/main.rs`(应用构建器注册字体、启动时剔除毒字体)、`crates/desktop/src/main.rs` 的 `mod` 声明
- Test:`crates/desktop/src/fonts.rs`

**Interfaces:**
- Produces:`fonts::{CODE_FONT_FAMILY: &str = "JetBrains Mono", JETBRAINS_MONO: &[u8] (include_bytes!), code_font() -> iced::Font, terminal_style() -> dozer_panel_term::style::TermStyle, sanitize_font_db()}`;`terminal_style()` 为 `TermStyle { font: code_font(), font_size: 14.0, line_height_factor: 1.2 }`。`sanitize_font_db` 与 Dozer `assets/fonts.rs` 同一做法(剔除纯位图的 "GB18030 Bitmap" / `NISC18030.ttf`,否则 CJK 回退会把整行排到 x=inf);重复调用无害。

- [ ] **Step 1: 写失败测试:** `code_font_is_jetbrains_mono`(`family == Family::Name("JetBrains Mono")`)、`embedded_font_bytes_are_a_truetype_font_that_names_itself_jetbrains_mono`(前四字节为 `00 01 00 00` 或 `true`,且字节里含 `JetBrains Mono` 的 UTF-16BE 或 ASCII 名字串)、`terminal_style_matches_dozers_terminal_json`(14.0 与 1.2)、`terminal_grid_size_is_stable_for_a_known_pane`(同输入同输出:对 `terminal_style().grid_size(w, h)` 取 `(600.0, 400.0)` 与 Dozer 同输入的结果相等,期望值由实现者在 Dozer worktree 里用同一公式算出后写死,附算式)、`sanitize_font_db_is_idempotent`。
- [ ] **Step 2: 运行确认失败**,然后实现。字体注册优先用 `iced::application(..).font(fonts::JETBRAINS_MONO)`;`sanitize_font_db` 在 `main()` 里 `run` 之前调用,拿 font system 的方式沿用 Dozer 的写法(经 iced 的 wgpu 图形层),如 Digger 的依赖里拿不到就停下报告。
- [ ] **Step 3: 验证。** `cargo test -p digger-desktop fonts`;`cargo build -p digger-desktop` 通过。
- [ ] **Step 4: 提交。**

### Task 4: Agent 面板壳:登记、布局迁移、不可用态(**本期只做 4a**;4b 见下)

**Files:**
- Create:`crates/desktop/src/adapters/agent.rs`(本期只含壳:`AgentShellView` 与不可用态,会话状态留给 S6d-2)
- Modify:`crates/desktop/src/shell.rs`(`agent_id()`、`all_panels`、`meta`、`rules`、`defaults`、旧布局迁移)、`crates/desktop/src/product.rs`(`ShellHost::body` 增 `"digger.agent"`、`catalog` 登记、`on_visible`、`Msg::Agent(AgentShellMsg)`、与 `daemon::Daemon` 的 `ensure` 接线)、`crates/desktop/src/adapters/mod.rs`
- Test:`crates/desktop/src/shell.rs`、`crates/desktop/src/adapters/agent.rs`、`crates/desktop/src/product.rs`

**Interfaces:**
- Produces:
  - `shell::agent_id() -> PanelId`(`"digger.agent"`);`meta`:标题「Agent」、`IconKind::Brain`;`rules`:`default_side: Side::Right`,`pane: Some(PaneRules { spec: PaneSpec::new(false, true), default_split: byteui::theme::geometry::default_split_ratio(), min_first_px: list_min_width() })`(列表是第一块、可拖窄收起,与用量同形;卡片行宽度的取值真机上再调);`defaults()`:右栏 `[agent, apps, usage]`,`selected` 的右侧为 `agent`(Q1)。
  - `shell::migrate_missing_agent(model: &mut ShellModel, had_agent_in_file: bool)`:旧文件里没有 `digger.agent` 时把它从落点移到右栏第一位,**不改任何一侧的选中面板**;已含则什么都不做(幂等)。用 S5a 迁移 Project 时用过的现有移动消息,不新造。
  - `adapters::agent::AgentShellMsg { Retry, Ensured(Result<(), String>) }` 与 `AgentShell { status: Status }`(`Default` 为 `Status::Ready`,本期 `product.rs` 不接 `Daemon`,面板直接显示占位),`Status { Unknown, Checking, Ready, Unavailable(String) }`;`AgentShell::view(&self) -> Element<'_, Msg>`:`Unavailable(reason)` 显示原因文案与「重试」按钮(按下发 `Retry`);`Checking` 显示「正在连接 Agent 服务…」;`Ready` 显示占位文案「Agent 面板即将可用」(S6d-2 替换为真实内容);无项目时显示与 Project 面板一致的空态。`update(&mut self, msg, ensure: impl FnOnce() -> Task<Msg>) -> Task<Msg>`(**`ensure` 由调用方注入**,本期测试用假实现,4b 注入真实的 `Daemon::ensure`):`Retry` 在 `Checking` 状态下被忽略(不并发多个 `ensure`),否则置 `Checking` 并返回 `ensure()`。
- Consumes:S1 的 `ShellModel`、`persist::load`。(4b 才会用到 S6a Part C 的 `daemon::Daemon::ensure`、`DaemonError`。)

- [ ] **Step 1: 写失败测试:**
  - `shell.rs`:`agent_panel_id_is_the_persisted_wire_name`(`"digger.agent"`);`the_default_layout_puts_agent_first_on_the_right_and_selects_it`(右栏以 `agent` 开头、`selected(Right) == agent`、左栏与 S5a 之后的断言一致);`an_old_file_without_the_agent_panel_moves_it_to_the_front_of_the_right_rail_and_keeps_the_users_selection`(文件右栏 `["digger.apps","digger.usage"]`、选中 `usage` → 迁移后 `[agent, apps, usage]`、选中仍是 `usage`、`left_width` 不变);`migration_is_idempotent_and_never_touches_a_file_that_already_has_the_agent`;`a_file_with_an_empty_right_rail_gets_the_agent_alone`;`a_newer_schema_file_is_still_never_written`(沿用已有断言并覆盖本迁移路径);`agent_is_a_pair_panel_and_usage_still_is_too`(更新原 `only_the_usage_panel_is_a_pair…`:现在有两个配对面板,其余仍单栏);`pair_rects_for_agent_on_the_right_zone_have_a_nonnegative_size_at_the_minimum_window`(最小窗口尺寸下 `first.w`、`second.w` 均 ≥ 0,宽到不足 `min_first_px` 时列表被收起而不是出现负宽)。
  - `agent.rs`:`unavailable_shows_the_reason_and_a_retry_that_checks_once`(连续两次 `Retry` 只发出一个 `ensure` 任务)、`a_successful_retry_turns_unavailable_into_ready`、`no_project_shows_the_same_empty_state_as_the_project_panel`。
  - `product.rs`:`the_agent_panel_is_registered_in_the_catalog_and_its_body_is_not_empty`、`switching_to_the_agent_panel_triggers_its_on_visible_action_once`(沿用 `visible_before` 机制)。
- [ ] **Step 2: 运行确认失败。**
- [ ] **Step 3: 实现(4a)。** `body` 的 `"digger.agent"` 分支返回 `AgentShell::view`,配对面板的双块摆放本期不做(S6d-2 替换成 `pair_view(列表, 终端)`)。**4b(待 S6a Part C 之后,本期不做):** `boot` 里的 `Daemon::ensure` 结果同时喂给 `AgentShell`(`Ensured`),不重复调用;`Retry` 注入真实 `ensure`。
- [ ] **Step 4: 验证。** `cargo test -p digger-desktop`;`cargo build -p digger-desktop`;手动起一次 Digger(沙箱 `DIGGER_DATA_DIR`),截图确认右栏最前有 Agent 图标且默认选中、点开显示占位文案。(`diggerd` 不可用态的真机验证属 4b。)
- [ ] **Step 5: 提交。**

### Task 5: 门禁与交付

- [ ] **Step 1: 全量。** `cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --check`;`bash scripts/check-host-deps.sh`、`bash scripts/check-shared-deps.sh`、`cargo tree -d`(带 patch);测试总数 = 基线 366 + 新增;再做一次干净快照复验(见顶部须知 ②)。
- [ ] **Step 2: 旧布局真机验证。** 用沙箱数据根放一份**不含 `digger.agent` 的旧 `shell.json`**(取自现有用户的真实形状),启动后右栏顺序、选中面板、左栏均符合 Task 4 的断言;再启动一次确认文件不再变化。
- [ ] **Step 3: 报告** 写明:探针程序已就绪、5 项真机清单(待用户填)、哪些项依赖人工、本地 patch 的写法与占位 tag、`min_first_px` 取值、测试总数与基线核对、遇到的与计划不符之处,以及 4b 待办。
- [ ] **Step 4: 停在 `feat/agent-shell`。** 不合并、不打 tag、不推送。

---

## Self-Review

- **Spec 覆盖:** 设计 §6.1 登记/布局/旧布局迁移/Q1 → Task 4;§6.4 键盘转换与 ⌘ 规则 → Task 1,输入法 `ImeSink` 与探针 → Task 2;§6.5 字体与 `TermStyle`(网格尺寸的实际同步属 S6d-2)→ Task 3;§6.3 末句与 G6(`diggerd` 不可用态)→ Task 4;§7 S6d-1 的「依赖升级」→ Task 1 Step 1。**不在本期:** 会话、每项目状态、懒启动、尺寸同步、选择器、页签条、确认框、退出语义(S6d-2 / S6d-3)。
- **类型一致:** `TermKey` / `TermMods` / `key_to_bytes` 来自 S6c-1 Task 2,Task 1 引用同名;`ImeState`、`ImeSink` 在 Task 2 定义;`terminal_style()` 在 Task 3 定义,S6d-2 的 `PanelDisplay::term_style` 复用;`AgentShell` 在 Task 4 定义,S6d-2 扩成完整的 `AgentPanel`。
- **风险点:** ① `TermNamedKey` 的变体清单以 S6c-1 实际落地为准,Task 1 的「每个 TermNamedKey 可达」测试会在清单变化时失败提醒;② `ImeSink` 只在 `RedrawRequested` 期间请求才生效(iced 文档明示),实现者要确认 `update` 里在该事件分支调用;③ 输入法行为无法自动化验证,Step 4 的人工门是本期唯一的真实证据;④ 字体注册与 `sanitize_font_db` 取 font system 的路径依赖 Digger 的 iced 特性,拿不到就停下报告;⑤ `min_first_px` 先沿用 `list_min_width()`,卡片实际需要的宽度在 S6d-2 有卡片后再调;⑥ S5a、S5b-1 若尚未合并,Task 4 里关于左栏的断言以实际合并结果为准,只保证右栏与选中面板。
