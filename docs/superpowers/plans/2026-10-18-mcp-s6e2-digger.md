# S6e-2 Digger 接入 `digger-mcp`:二进制、打包、自动注册与记忆联动 Implementation Plan

> **迁移说明(2026-10-20):** 本文原在 Digger 仓库,现移入 Dozer 仓库;文内 `docs/superpowers/...` 路径均相对 **Dozer 仓库根**。写作「Digger(仓库 `../digger`)」的部分,实施仍在 Digger 仓库的 worktree 里进行,但文档不在那里。Digger 自有文档(`docs/adr/*`、`2026-10-14-window-shell-design.md`、`docs/superpowers/notes/`)仍在 Digger 仓库。最新要求与决定见 `docs/superpowers/specs/2026-10-18-agent-panel-s6-design.md` §0。

> **2026-10-23 修订(Dozer 侧验收通过后,以此为准,与下文及其它顶部修订冲突处以本条为准):**
> - **Dozer 侧链路已验收、合并 `main`、推送。** 库快照 tag 是 **`libs-2026-10-23a`**(上一个是 `libs-2026-10-20a`);本文及其它修订里"尚未验收/未合并/没有 `libs-*` tag""不合并 `main`、不打 tag、不推送""用不提交的本地 `[patch]`""占位 tag `libs-2026-10-20a`""`Cargo.lock` 不提交/没有 `--locked` 可验"一律作废:依赖直接钉 tag、**提交 `Cargo.lock`**、`cargo metadata --locked` 与 `cargo tree -d` 门禁照常跑。打 tag/推送仍是对外动作,逐次向用户确认。
> - **`reorder_term_tab` 回到库里,且签名变了:** `dozer_panel_agent::update::reorder_term_tab(state, from, to) -> bool`(显式来源下标,越界或 `from == to` 返回 `false`;`active` 始终指向同一个会话)。库里**不再有** `Message::TabDragMove` / `TabDragEnd`;`PanelDisplay::drag_surface(&self, content, index)` 只有两个参数(没有 `on_move`)。**含这个函数的 tag 是 `libs-2026-10-23b`(待切,见 Dozer 侧提交);在它存在之前不要开始依赖它的任务(S6d-3 Task 3)。** 拖拽手势(按下武装、位移阈值、组校验)全部由产品的拖拽状态机负责,库只做换位。
> - **opencode 验收要覆盖 2.x:** opencode 2.x 拒绝具名导出的插件。Dozer 的插件模板已改成 `export default { id, setup, server }`(`c15804ad`),Digger 的 `digger.ts` 由同一模板生成,插件 id 是 `digger`。验收时在 opencode 2.x 下确认:`opencode` 日志里没有 `failed to load plugin`,状态点随回合更新。
> - **hook 对旧守护进程的取舍已付过一次代价:** 新版 hook 遇到不认识 `Hello` 的旧守护进程会放弃上报(S6a 的刻意取舍),Dozer 因此出过"卡片不更新"。Dozer 的配套是 `Client::daemon_is_outdated()` + 顶栏"需重启 dozerd"徽标(`c016794b`)。Digger 首发不会遇到(`diggerd` 是新进程),但以后升级会;如要复用,在 S6d-3 的顶栏里接同一个标记。


> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Digger 里的 Agent 通过自己的 MCP 服务 `digger-mcp` 读写**项目共享记忆**(三个工具:`write_memory`、`list_memories`、`get_memory`),数据落在 `diggerd` / `digger.db`,与 Project 面板同一份;Digger 新建 Agent 会话前自动把 `digger` 条目注册进该 Agent 的全局 MCP 配置;Agent 写入记忆后,Project 面板在回合结束时刷新;与 Dozer 同机并存、互不串线、互不报错。不发布产品版本。

**Architecture:** 新 crate `crates/digger-mcp`(3 行入口,仿 `diggerd` / `digger-hook`):`dozer_mcp::run_main(digger_paths::product(), McpFeatures { memory: true, 其余 false })`。打包脚本把它与 `diggerd`、`digger-hook` 并列放进 `Digger.app/Contents/MacOS/`。S6d-2 里 `DiggerHooks::ensure_mcp` 的空实现改为运行同目录的 `digger-mcp install <agent>`(不继承 stdin、丢弃输出、10 秒上限),并在**同一次 Digger 运行内每个 Agent 只执行一次**(Dozer 每次建会话都重写全局配置,Digger 不照搬);`AgentCapabilities::mcp_install` 打开。回合结束联动经 S7b 加的 `AgentPanel::drain_host_events` 与 `ProjectPanel` 的一个新方法,只在 Project 面板在眼前时刷新记忆。

**Tech Stack:** Rust、`dozer-mcp`(S6e-1 的 `libs-*` tag)、`digger-paths`、`dozer-panel-project`、`dozer-services`。

**Spec:** `docs/superpowers/specs/2026-10-18-agent-mcp-s6e-design.md`(§6.2、§6.4 S6e-2、§7 Digger 段与真机、§8 D3 / D5 / D6)。前置:S6e-1 已合并并切 tag;S6a(`digger-paths`、`diggerd`、`digger-hook`、打包脚本的多二进制写法)、S6d-2(`DiggerHooks`、`AgentCapabilities`)、S7b(`drain_host_events`)已合并。已批准的取舍:Q1 静默写全局配置(`digger-mcp uninstall <agent>` 可移除);Q2 同名条目保护;Q3 工具集比 Dozer 少。

## Global Constraints

- **库不改。** `dozer-mcp` 缺什么只能报告,不在 Digger 里复制;依赖方向、`check-host-deps.sh`、`check-shared-deps.sh`、Cargo.lock 重复检查通过(新引入 `rmcp`、`schemars`、`toml_edit` 等,同一 crate 只能有一个版本)。
- **工具集恰为三个记忆工具:** `digger-mcp serve` 在 `DIGGER_SESSION_ID` 下 `tools/list` 恰为 `write_memory` / `list_memories` / `get_memory`(描述与参数 schema 与 Dozer 的同名工具逐字相同,因为共用同一份代码);不得出现待办、预览、定位、编辑、总结工具。
- **只暴露真实存在的能力:** 工具读写的是 `diggerd` 里的项目记忆,Project 面板读的同一份 `digger.db`;不允许 Digger 暴露「调用后无可见效果」的工具。
- **与 Dozer 并存:** `digger` 与 `dozer` 条目互不覆盖、卸载互不影响;Dozer 会话里 `digger-mcp` 与 Digger 会话里 `dozer-mcp` 都进入零工具安静模式,Agent 的 MCP 状态里**不是失败**;`digger-mcp` 不读写 Dozer 的目录、socket、数据库。
- **全局配置写入的克制(Digger 的取舍,标 D7):** 每个 Digger 进程生命周期内,每种 Agent 只执行一次 `digger-mcp install`(进程内记忆);同名保护拒绝(退出码 3)时**不重试**,在 Agent 面板顶部通知条给出一次可读提示(「未写入 Digger 的 MCP 条目:`<配置文件>` 里已有一个名为 `digger` 的服务器」);缺少 `digger-mcp` 二进制、超时、非零退出都不阻塞会话创建,只在通知条出现一次。
- 失败不静默:通知条文案可关;同时 `eprintln!`。
- 主工作区别人未提交的改动不动;worktree `../digger-s6e2`、分支 `feat/digger-mcp`(从已含前置的 `main` 切出)。合并前向用户确认。**不发布产品版本**(MCP 不是面板;随下一次面板迁移完成的发布一起带上)。

## Review Focus

1. **三个工具的端到端:** 经真实 `diggerd` 写 → 读 → 列;会话→项目解析(`list()` 找本会话的 `project_id`);会话尚未归属项目时的可读错误;同名标题 upsert 而不是重复;超长标题 / 正文 / 空标题;`updated_by` 归属为该会话的 Agent。→ Task 1。
2. **安静模式的真实表现:** Dozer 会话里的 `digger-mcp`、Digger 会话里的 `dozer-mcp`(若同机装了 Dozer)握手成功、零工具、不吐 stderr;多个 Agent(Claude / Codex / Codebuddy / Opencode / Qodercn)对零工具服务的状态显示(连接 / 空工具,不是失败)。→ Task 1、5。
3. **安装的克制与保护:** 同一次运行内只装一次;同名手写条目不被覆盖且只提示一次;缺二进制不阻塞建会话;`~/.claude.json` 这类混装状态文件在 Agent 运行时被改写的风险(Dozer 同样存在,Digger 通过「只装一次」降到最低);卸载只删自己的。→ Task 3、5。
4. **记忆联动:** 回合结束且 Project 面板可见才刷新;面板不可见不刷新;不是当前项目不刷新;连续多个回合结束不堆请求(库的请求追踪负责,这里用测试固定);Agent 在另一个项目的会话里写记忆不影响当前项目的列表。→ Task 4。
5. **打包与发布脚本:** `digger-mcp` 缺失即失败;签名校验覆盖新二进制;`release.sh --dry-run` 与真打包都通过。→ Task 2。

---

### Task 1: `digger-mcp` crate

**Files:**
- Create:`crates/digger-mcp/{Cargo.toml,src/main.rs}`、`crates/digger-mcp/tests/{tool_set.rs,quiet_mode.rs,memory_roundtrip.rs,install_entry.rs}`
- Modify:workspace `Cargo.toml`(成员加 `crates/digger-mcp`;workspace 依赖加 `dozer-mcp` 指向 S6e-1 的 tag;所有 dozer libs 的 `tag` 已在 S6e-1 Task 6 同步)、`Cargo.lock`、`scripts/check-host-deps.sh`(`HOST_ADAPTER_CRATES` 加 `digger-mcp`)、`shared-deps.toml`(按现有格式登记,若需要)
- Test:`crates/digger-mcp/tests/*`

**Interfaces:**
- Produces:二进制 `digger-mcp`(`serve` / `install <agent>` / `uninstall <agent>`),`main` 为 `dozer_mcp::run_main(digger_paths::product(), McpFeatures { memory: true, summary: false, todos: false, preview: false, file_edit: false })`,仅此而已。
- [ ] **Step 1: 写失败测试**(进程测试用隔离的 `DIGGER_DATA_DIR`、临时 `HOME`、受控 stdin):
  - `tool_set.rs`:`tools_list_is_exactly_the_three_memory_tools`(`tools/list` 的名称集合恰为三个;描述与参数 schema 与 Dozer 的 `tools_list_golden.json` 中同名项逐字相同——测试里读取 `dozer-mcp` 库的黄金数据或在同一测试里用 `McpFeatures::all()` 的路由取同名工具比较);`no_todo_preview_edit_or_summary_tool_is_listed`。
  - `quiet_mode.rs`:`with_only_dozer_session_env_the_server_initializes_and_lists_nothing`(`DIGGER_SESSION_ID` 移除、`DOZER_SESSION_ID=x`;`initialize` 成功、`tools` 为空、stdin 关闭后退出码 0、stderr 为空、socket 不存在也成功);`with_no_session_env_it_fails_with_the_digger_message`(文案:`缺少 DIGGER_SESSION_ID：digger-mcp 只能在 digger 拉起的会话里跑`)。
  - `memory_roundtrip.rs`(起一个真实的 `diggerd` 库实例,`dozerd::run` 的测试夹具,产品身份为 digger、临时数据根、临时 socket):`write_then_list_then_get_roundtrip`;`write_with_the_same_title_updates_instead_of_duplicating`;`session_without_a_project_gets_a_readable_error`;`memory_written_through_mcp_is_visible_through_the_project_store`(用 Digger 进程内的 `ProjectHost` 打开同一个 `digger.db` 读到);`updated_by_is_the_sessions_agent_label`;`empty_title_and_oversized_body_are_handled_like_dozer`(与 Dozer 的 `memory_tools` 测试同款输入同款期望)。
  - `install_entry.rs`:`install_writes_a_digger_entry_for_each_supported_agent`(五种 Agent,环境变量 `DIGGER_*_MCP_CONFIG` 指向临时文件);`install_next_to_an_existing_dozer_entry_keeps_it_byte_identical`;`uninstall_removes_only_the_digger_entry`;`a_same_named_user_entry_is_protected_and_exits_3`。
- [ ] **Step 2: 运行确认失败,建 crate,验证。** `cargo test -p digger-mcp`;`bash scripts/check-host-deps.sh`、`check-shared-deps.sh`、Cargo.lock 重复检查。
- [ ] **Step 3: 提交** `feat(digger): digger-mcp 二进制(仅记忆工具)`。

### Task 2: 打包与发布脚本

**Files:**
- Modify:`scripts/package-macos.sh`、`scripts/release.sh`
- Test:脚本自检

- [ ] **Step 1: `package-macos.sh`:** 在 S6a 已有的 `diggerd`、`digger-hook` 并列构建与拷贝处加入 `digger-mcp`(`-p digger-mcp --bin digger-mcp`,同样用 `jq` 从 `--message-format=json` 里取路径,不猜 `target/` 布局),拷进 `Digger.app/Contents/MacOS/`;脚本头注释更新 sibling 二进制清单,并保留对「sibling 缺失」事故的说明(缺了 `digger-mcp` 时 Agent 的 MCP 注册指向不存在的文件)。
- [ ] **Step 2: `release.sh` 的校验段:** 加 `test -x "$OUT/Contents/MacOS/digger-mcp"`;缺失即失败并说明;`codesign --verify --deep --strict` 继续覆盖整个 bundle。
- [ ] **Step 3: 验证。** `bash scripts/release.sh --skip-tests --allow-dirty --no-bump --dry-run` 门禁通过;再 `--no-bump --skip-tests --allow-dirty` 真打包一次:`ls Digger.app/Contents/MacOS` 含 `digger`、`diggerd`、`digger-hook`、`digger-mcp`,`codesign --verify --deep --strict` 通过;故意移走 `digger-mcp` 后 `release.sh` 的校验段失败(验证门禁有效)。
- [ ] **Step 4: 提交。**

### Task 3: 自动注册(`ensure_mcp`)与能力开关

**Files:**
- Modify:`crates/desktop/src/adapters/agent_hooks.rs`(`DiggerHooks`)、`crates/desktop/src/adapters/agent.rs`(`AgentCapabilities { mcp_install: true, .. }`、通知)、`crates/desktop/src/product.rs`
- Test:`agent_hooks.rs`、`agent.rs`

**Interfaces:**
- Produces:
  - `DiggerHooks::ensure_mcp(&self, agent: AgentKind)`:支持的 Agent(Claude / Codebuddy / Codex / Opencode / Qodercn)且本进程尚未对它执行过 → 运行 `<同目录 digger-mcp> install <agent.label()>`(**不继承 stdin、丢弃 stdout / stderr、10 秒上限**),按退出码记录结果:`0` 记为已装;`3` 记为「被保护拒绝」并登记一条待展示警告;其它非零 / 超时 / 找不到二进制 记为「失败」并登记一条待展示警告;无论结果,该 Agent 在本进程内不再重试。Aider / Goose / V8agent / Unknown 空操作。
  - `DiggerHooks::take_warnings(&self) -> Vec<String>`(取走并清空);`AgentPanel` 在每次 `update` 之后调用它,把文案推进 `notice`(S6d-2 的通知条)。
  - `AgentCapabilities` 在 Digger 里 `mcp_install = true`、`summary_on_close = false`。
- [ ] **Step 1: 写失败测试**(用记录参数的假脚本充当 `digger-mcp`):`ensure_mcp_runs_install_with_the_agent_label_once_per_process`(同一 Agent 连续建 3 个会话只调用一次);`different_agents_each_get_one_install`;`unsupported_agents_do_nothing`;`exit_code_3_registers_one_readable_protection_warning_and_is_not_retried`;`missing_binary_registers_one_warning_and_does_not_block_or_panic`;`hanging_install_is_abandoned_after_the_timeout_and_warns_once`(假脚本 `sleep 60`,注入较短上限);`install_does_not_inherit_stdin`;`warnings_are_taken_once_and_cleared`;`the_notice_text_names_the_config_file_for_protection_refusals`;`mcp_install_capability_is_on_but_summary_on_close_is_off`;`session_creation_still_proceeds_when_install_fails`(假服务断言 `create` 仍被调用)。
- [ ] **Step 2: 运行确认失败,实现,验证,提交。** 保护提示文案里的配置文件路径不从 `digger-mcp` 的输出里取(本函数丢弃输出),而是用 `config_path_for_product(agent, &digger_product)` 在 Digger 侧重新推导(同一个库函数,不重复实现)。

### Task 4: 回合结束时刷新 Project 面板的记忆列表

**Files:**
- Modify:`crates/desktop/src/adapters/project.rs`(`ProjectPanel::refresh_memories`)、`crates/desktop/src/product.rs`(排空 `AgentHostEvent` 后调用)
- Test:`project.rs`、`product.rs`

**Interfaces:**
- Produces:`ProjectPanel::refresh_memories(&mut self, project: &ProjectInfo)`:只调库的 `dozer_panel_project::on_activate(state, &ctx)`(该函数**只**刷新记忆列表;不要调用本适配层的 `activate`,它还会补 README、git、磁盘占用);`ctx` 的构造照 `activate` 里现有的 `ActivationCtx`。`ProductShell` 在排空 `AgentPanel::drain_host_events()` 时,对每个 `(pid, AgentHostEvent::TurnEnded { .. })`:当 `panel_visible(project_id())` 且 `projects.current_project_id() == Some(pid)` 且该项目的 Project 面板状态已存在时,调 `refresh_memories`;其余情况什么都不做。(这是 Digger 相对 Dozer 的**有意改进**:Dozer 的 Project 面板只在切入时刷新记忆,Agent 写入后要重新切入才看到。)
- [ ] **Step 1: 写失败测试:** `turn_ended_refreshes_memories_only_when_the_project_panel_is_visible_and_the_project_is_current`(四个条件逐个取反各一条);`turn_ended_for_a_background_project_does_not_refresh_the_current_one`;`turn_ended_without_a_project_panel_state_does_nothing`;`repeated_turn_ended_events_do_not_stack_refreshes`(假 `MemoryService` 慢返回,连发 5 个事件,最终列表正确、旧结果不覆盖新结果,请求追踪令牌生效);`state_changed_events_do_not_refresh`;`refresh_memories_does_not_touch_readme_git_or_disk_usage`(假服务与文件系统断言这三者零调用 / 零写入);`a_memory_written_through_the_store_appears_after_the_refresh`(用 `ProjectHost` 的内存服务写一条,刷新后列表含它)。
- [ ] **Step 2: 运行确认失败,实现,验证,提交。**

### Task 5: 门禁、真机回归与交付

- [ ] **Step 1: 全量。** `cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --check`;`check-host-deps.sh`(含 `--self-test`)、`check-shared-deps.sh`、`check-shell-regions.sh`、Cargo.lock 重复检查。
- [ ] **Step 2: 真机回归**(隔离的 `DIGGER_DATA_DIR` 与临时 `HOME`;点击由用户做,我启动并截图;需要本机装有 Claude Code CLI,没有则 Claude 项改为协议级验证并注明):① 在 Digger 里开 Claude 会话,`~/.claude.json`(临时 `HOME` 下)出现 `mcpServers.digger`,且只被写一次(再开第二个会话,文件 mtime 不变);在会话里运行 `claude mcp list`(或在 Claude 里 `/mcp`),`digger` 显示已连接、工具恰为三个记忆工具;② 让 Agent 写一条记忆,回合结束后 **Project 面板无需切换**出现该条;③ 同机再启动 Dozer(临时 `HOME` 下先 `dozer-mcp install claude`),两个条目并存;在 Digger 会话里 `/mcp` 看到 `dozer` 为已连接但零工具(不是失败),在 Dozer 会话里 `digger` 同理;④ 在 Dozer 里写一条记忆,Digger 的 Project 面板看不到(各自的库);⑤ 预置一个手写的同名 `digger` 服务器条目:Digger 不覆盖,Agent 面板顶部出现一次提示,之后不再出现;⑥ `digger-mcp uninstall claude` 后 `dozer` 条目仍在;⑦ 临时移走 `digger-mcp` 二进制:建会话不被阻塞,通知条出现一次可读提示;⑧ Codex / Opencode / Codebuddy / Qodercn 若本机已装,各做一次安装与零工具模式观察,没装的在报告里标「未验」。逐项记录,任一项不符先停下,不要为了「过」而改验证。
- [ ] **Step 3: 文档。** `2026-10-18-agent-mcp-s6e-design.md` 状态改「S6e-1、S6e-2 已实现」并补「实现结果」(含各 Agent 对零工具服务的实测表现、D7 的「只装一次」取舍、Project 面板刷新是对 Dozer 的有意改进);`docs/adr/ADR-015-product-owned-daemon.md` 补一节「MCP」(`digger-mcp` 只暴露记忆工具、与 `dozer-mcp` 并存与安静模式);记录 S6e-3 的依赖(待办面板、预览引擎)。
- [ ] **Step 4: 报告** 写明:各项真机结果与截图路径、依赖升级到的 tag、各 Agent 的实测表现、哪些依赖人工或本机缺失而未验、与计划不符之处、遗留(工具集 S6e-3;`ensure_mcp` 的「只装一次」是进程内记忆,重启 Digger 会再装一次,属预期)。合并到 `main` 前向用户确认。**不发布。**

---

## Self-Review

- **Spec 覆盖:** §6.2 `digger-mcp` crate → Task 1;打包与 `release.sh` 校验 → Task 2;安装时机、`mcp_install` 打开、Aider 等空操作 → Task 3;记忆联动(G4、D6)→ Task 4;§7 Digger 段与真机(三工具、安静模式、并存、同名保护、卸载)→ Task 1、5;§8 D3 / D5 → Task 1、Global Constraints;Q1–Q3 → Global Constraints。**不在本期:** S6e-3(待办 / 预览 / 编辑工具)、`dozer-mcp` 库改动、产品发布。
- **类型一致:** `DiggerHooks` 与 `AgentCapabilities` 来自 S6d-2,本期只改 `ensure_mcp` 与能力值并加 `take_warnings`;`drain_host_events` 来自 S7b;`config_path_for_product` 来自 S6e-1 Task 4;`McpFeatures` 来自 S6e-1 Task 3;`digger_paths::product()` 来自 S6a。
- **风险点:** ① S6e-1 的 `run_main` / `McpFeatures` / 退出码 3 的最终签名以其合并结果为准;② `dozerd::run` 的测试夹具要能以 digger 身份在测试里起 `diggerd`,S6a 的集成测试应已有同类夹具,没有就停下报告;③ 各 Agent 对零工具 MCP 服务的显示因 Agent 与版本而异,只能真机看,记录实测表;④ 「只装一次」是进程内记忆,不跨重启,也不检测用户在运行期间手动删了条目,这是有意的简化;⑤ Project 面板刷新依赖库的 `on_activate` 只刷新记忆这一事实(读代码得出),实现者动手前再核对一次,若它将来加了别的动作,改用 `request_memories_refresh`(需库把 `requests` 暴露出来,届时先停下报告);⑥ `~/.claude.json` 在 Claude Code 运行时被改写的竞态风险 Dozer 同样存在,本计划只做到降频,不做文件锁;⑦ 合并是对外动作,先确认;⑧ 不发布产品版本。
