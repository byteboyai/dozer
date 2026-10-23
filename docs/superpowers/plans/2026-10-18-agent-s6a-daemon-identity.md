# S6a 产品身份与 `diggerd` 守护进程 Implementation Plan

> **迁移说明(2026-10-20):** 本文原在 Digger 仓库,现移入 Dozer 仓库;文内 `docs/superpowers/...` 路径均相对 **Dozer 仓库根**。写作「Digger(仓库 `../digger`)」的部分,实施仍在 Digger 仓库的 worktree 里进行,但文档不在那里。Digger 自有文档(`docs/adr/*`、`2026-10-14-window-shell-design.md`、`docs/superpowers/notes/`)仍在 Digger 仓库。最新要求与决定见 `docs/superpowers/specs/2026-10-18-agent-panel-s6-design.md` §0。

> **2026-10-23 修订(Dozer 侧验收通过后,以此为准,与下文及其它顶部修订冲突处以本条为准):**
> - **Dozer 侧链路已验收、合并 `main`、推送。** 库快照 tag 是 **`libs-2026-10-23a`**(上一个是 `libs-2026-10-20a`);本文及其它修订里"尚未验收/未合并/没有 `libs-*` tag""不合并 `main`、不打 tag、不推送""用不提交的本地 `[patch]`""占位 tag `libs-2026-10-20a`""`Cargo.lock` 不提交/没有 `--locked` 可验"一律作废:依赖直接钉 tag、**提交 `Cargo.lock`**、`cargo metadata --locked` 与 `cargo tree -d` 门禁照常跑。打 tag/推送仍是对外动作,逐次向用户确认。
> - **`reorder_term_tab` 回到库里,且签名变了:** `dozer_panel_agent::update::reorder_term_tab(state, from, to) -> bool`(显式来源下标,越界或 `from == to` 返回 `false`;`active` 始终指向同一个会话)。库里**不再有** `Message::TabDragMove` / `TabDragEnd`;`PanelDisplay::drag_surface(&self, content, index)` 只有两个参数(没有 `on_move`)。**含这个函数的 tag 是 `libs-2026-10-23b`(待切,见 Dozer 侧提交);在它存在之前不要开始依赖它的任务(S6d-3 Task 3)。** 拖拽手势(按下武装、位移阈值、组校验)全部由产品的拖拽状态机负责,库只做换位。
> - **opencode 验收要覆盖 2.x:** opencode 2.x 拒绝具名导出的插件。Dozer 的插件模板已改成 `export default { id, setup, server }`(`c15804ad`),Digger 的 `digger.ts` 由同一模板生成,插件 id 是 `digger`。验收时在 opencode 2.x 下确认:`opencode` 日志里没有 `failed to load plugin`,状态点随回合更新。
> - **hook 对旧守护进程的取舍已付过一次代价:** 新版 hook 遇到不认识 `Hello` 的旧守护进程会放弃上报(S6a 的刻意取舍),Dozer 因此出过"卡片不更新"。Dozer 的配套是 `Client::daemon_is_outdated()` + 顶栏"需重启 dozerd"徽标(`c016794b`)。Digger 首发不会遇到(`diggerd` 是新进程),但以后升级会;如要复用,在 S6d-3 的顶栏里接同一个标记。


> **2026-10-22 修订(审核方):** 本文只执行 **Part C(Task 7–11)**。Part A/Task 6b 已在 Dozer 叠分支上开发并审核通过,并已快进到 Dozer `main`(`673a0e77`);**尚未验收、没有 `libs-*` tag、未推送**。规则如下:
> - **依赖用不提交的本地 `[patch]`,不合并 `main`、不打 tag、不推送。** 在 worktree 的 `.cargo/config.toml`(已被忽略)里照 `scripts/dev-patch.example.toml`,把 `[patch."https://github.com/byteboyai/dozer"]` 指向 `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer-mcp-product/crates/<crate>`(与 `main` `673a0e77` 内容相同)。**Digger 直接依赖的每个 dozer crate 都要列**:沿用 S6d-1 已有的 10 项(`dozer-model`、`dozer-services`、`dozer-panel-{usage,project,files,preview,host,term}`、`dozer-preview-core`、`dozer-store-project`),**再加本期新增的 `dozer-core`、`dozerd`、`dozer-hook`、`dozer-client`**。workspace `Cargo.toml` 里新增这四项依赖暂用占位 tag `libs-2026-10-20a`(该 tag 里没有它们,只有 patch 在时才解析)。`Cargo.lock` 不提交。所以文内"指向 Task 6 的 tag""`Cargo.lock` 重复检查输出 `ok: one version each`"改为:在带 patch 的 worktree 里 `cargo tree -d`,**与基线对比**不得新增重复(基线已有一项 `bytehost-apps`,同一版本同一来源,不算);`scripts/check-shared-deps.sh` 会因 `lock-not-git` 报 1 个阻断,**这是预期**(本地 patch),报告里写明;`scripts/check-host-deps.sh` 与 `--self-test` 必须通过。
> - **分支叠在 S6d-1 上:** worktree `../digger-s6a`,分支 `feat/diggerd`,**从 `feat/agent-shell` 当前顶端 `fd17623` 切出**(不是 `main`)。原因:Task 9 要把 `ensure` 结果接进 S6d-1 已有的 `AgentShell`(即 S6d-1 的 **Task 4b**,并入本期,见下),两者都改 `product.rs`/`main.rs`/`Cargo.toml`,分开切会冲突。开工先 `git -C ../digger rev-parse --short feat/agent-shell`,必须是 `fd17623`,不是就停下来问。**不要碰** Digger 主工作区别人未提交的改动(`Cargo.lock`、`crates/desktop/Cargo.toml`、`Info.plist`、13 个 S6 文档的未提交删除),也不要动 `feat/preview-pairing`(worktree `digger-p4`)。
> - **基线(`fd17623`,干净快照拷入 patch 配置):** `cargo test --workspace --no-fail-fast` = **396 通过 / 0 失败 / 0 ignored**;`cargo clippy --workspace --all-targets` 退出码 0,警告 1 条(`block v0.1.6` 未来兼容提示);完工不得更多、不得更少测试名(测试名守恒)。
> - **SQLite 并发(Task 8 Step 2)的事实与决定:** 我核对过——`dozer-store-project` 的 `ProjectStore::new` / `MemoryStore::new` 都是裸 `Connection::open`,**没有 WAL**;但 `rusqlite 0.32.1` 打开连接时默认 `busy_timeout(5000)`(`inner_connection.rs:119`),所以"立刻报 `database is locked`"不会发生,风险是回滚日志模式下写者互相阻塞到超时。**不改 Dozer。** 决定:`digger.db` 是 Digger 自己的文件,由 Digger 在 `open_in` 里、打开各 store **之前**用一条一次性连接执行 `PRAGMA journal_mode=WAL`(持久在文件里,尽力而为,失败只记日志不阻断);`digger-store` 为此直接依赖 `rusqlite`(**版本必须与 Dozer 一致,0.32.1**,否则 `Cargo.lock` 出现两份)。并发测试照常写,若在这套配置下仍出现 `database is locked`,**停下报告,不要自己改 Dozer**——那是需要用户决定的事。
> - **S6d-1 的 Task 4b 并入本期 Task 9。** 见 Task 9 的追加步骤。
> - **文档:** Task 11 Step 3 只写 Digger 仓库内的文档(`docs/adr/ADR-015-...`、ADR-014 的 Consequences、`2026-10-14-window-shell-design.md`)。**不要改** Digger 仓库里 `2026-10-18-agent-panel-s6-design.md` 等 S6 文档:它们在 Digger 主工作区已处于"未提交删除"状态,权威版本在 Dozer 仓库;"S6a 已实现"的状态由审核方在 Dozer 仓库更新,实现者在报告里写清结果即可。
> - **Task 11 Step 4 作废**(不合并、不清理分支,停在 `feat/diggerd`)。Task 11 Step 2 的同机并存回归见下方改写。
> - **Task 10:** 真打包只跑 `scripts/package-macos.sh`(或 `release.sh --no-bump --skip-tests --allow-dirty`,二者都不提交、不打 tag、不推送),并确认产物在 worktree 的 `target` 下,**不安装到 `/Applications`**。
> - **实现者须知(前几期审核里真踩过的坑):** ① 新增文件(`crates/paths`、`diggerd`、`digger-hook`、`migrate.rs` 及打包脚本改动)提交后用 `git status --ignored` 核对没被 `.gitignore` 吞掉;② 完工前把 `git archive HEAD` 解到**独立临时目录**,**拷入你的 patch 配置**,在该目录自己的 `target` 下重跑测试,报告里写明;③ `cargo fmt --check` 若在 `main` 上本就失败,只要求自己改的文件干净(`rustfmt --check --edition 2024` 逐个文件);④ 报告里每条「通过」必须是**最终代码**上刚实跑的,附命令与退出码,含测试总数(基线 396 + 新增)、测试名守恒、clippy 实测数,审核人会逐条重跑;计划与代码不符、库缺东西、基线对不上时先停下来问,不要自己绕过;⑤ **绝不对真实 `HOME` / 真实数据目录 / 用户常驻的 `dozerd` 做任何操作**(含 `kill`、`rm`、写 `~/.claude/settings.json`):所有回归都用隔离的 `DIGGER_DATA_DIR` 与临时 `HOME`;⑥ 报告写到 Digger worktree 的 `docs/superpowers/reviews/2026-10-<日期>-s6a-partc-report.md`(Digger 仓库内;本计划文件在 Dozer 仓库,实现者读它但不修改它)。


> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Dozer 的守护进程变成"共享库 + 各产品薄入口",产品身份(数据目录、socket、数据库、会话环境变量、hook 名)可配置;Digger 拥有并拉起自己的 `diggerd` 与 `digger-hook`,数据库是 Digger 数据根下的 `digger.db`(项目注册表只有这一份);两个产品同机并存时事件、hook 配置、数据互不串线。**本期结束时 Digger 有自己的守护进程在跑,但还没有 Agent 界面。**

**Architecture:** (A)Dozer:`dozer-core::paths` 引入 `ProductId`,默认 `dozer`,行为字节不变;`dozerd/src/main.rs` 的装配搬进 `dozerd::run(DaemonConfig)`,`DaemonConfig` 带 `DaemonFeatures` 开关;hook 安装与识别、会话环境变量全部按 `ProductId` 推导;新增协议版本握手。(C)Digger:新建 `digger-paths`、`diggerd`、`digger-hook` 三个小 crate,`diggerd` / `digger-hook` 各 3 行入口;进程内项目存储改为打开 `digger.db` 并迁移旧 `projects.db`;`digger-desktop` 负责拉起 `diggerd`;打包脚本把两个二进制放进 `.app`。

**Tech Stack:** Rust、`dozer-core`、`dozerd`(库)、`dozer-hook`(库)、`dozer-client`、`dozer-store-project`、SQLite(WAL)、`portable_pty`(仅在 `dozerd` 内部)。

**Spec:** `docs/superpowers/specs/2026-10-18-agent-panel-s6-design.md`(§6.1–6.4、§7 S6a、§8)。

## Global Constraints

- **Dozer 用户可见行为与数据位置不变**:`ProductId` 默认值下,`config_dir()` / `state_dir()` / `socket_path()` / `logs_dir()` 的结果、`DOZER_SESSION_ID`、`dozer-hook` 命名与改造前**字节一致**(黄金测试);被搬走的测试随代码一起走,不删、不改期望值;抽库前后 Dozer 测试总数守恒。
- **产品独立**:`diggerd` 不得读写 Dozer 的目录、socket、数据库,反过来同理;两个产品只共享代码。
- **hook 并排共存**:每个产品只识别、安装、更新、卸载**自己名字**的 hook 条目,绝不改动别人的条目;每个 hook 二进制只认自己产品的会话环境变量。
- **`diggerd` 的后台功能默认关闭**(我的决定,标 D7,覆盖设计 Q1 与 §9 D3):应用宿主(Digger 的应用宿主在进程内)、历史 transcript 启动回填与 Codex SQLite 摄取、总结调度与 worker、`task_poller`、IDE 桥接(它往 `~/.claude/ide/` 写全局锁文件,两个守护进程会让 Claude Code 看到两个 IDE)。**Dozer 全开,与现状一致。**
- 只用 git tag 依赖,不提交本地 `[patch]`、不提交带本地路径的依赖(各仓库内部 `path = "../.."` 除外)。**打 tag、推送是对外动作,每次执行前向用户确认**;Digger 没有 remote。
- Dozer 与 Digger **同日升级**依赖(ADR)。
- 不动各主工作区别人未提交的改动:Dozer 主工作区有未提交的 `Cargo.lock`、`dozer-app/Cargo.toml`、`Info.plist` 改动。实现在独立 worktree 里做(Dozer:`../dozer-product-id`,分支 `feat/product-id`;Digger:`../digger-s6a`,分支 `feat/diggerd`)。
- 不引入新的日志依赖。
- **前置:S5a 已合并**(Digger 的 `ProjectsState` 与左栏)。Part A 不依赖它,可先做。

## Review Focus

1. **hook 配置并存与幂等:** 已含 Dozer 条目的配置里装 Digger 条目后两者并存;重复安装不产生重复条目;卸载只移除自己的;名字含空格的路径、`dozer_hook-<hash>` 形式的测试二进制名不被误认。→ Task 3。
2. **事件串线:** Dozer 会话里触发 `digger-hook`、Digger 会话里触发 `dozer-hook`,对方守护进程收不到任何事件;环境里没有自己的会话变量时 hook 静默返回、退出码 0、不写 stderr。→ Task 3、5。
3. **`digger.db` 并发与迁移:** `diggerd` 与进程内存储同时写同一个 SQLite 文件不丢写、不报 `database is locked`;旧 `projects.db` 迁入幂等、迁入中途失败不留下半个 `digger.db`、迁入后旧文件改名保留。→ Task 8。
4. **守护进程单实例与启动失败:** 同一数据根下第二个 `diggerd` 不动数据就退出;`diggerd` 二进制缺失/拉起失败时 Digger 主窗口仍正常启动并给出可读提示,不卡住。→ Task 9。
5. **协议版本不一致:** 客户端与守护进程协议版本不同时给出可读错误并拒绝使用,不是莫名其妙的反序列化失败;hook 遇到不一致时静默返回并写日志,不影响 Agent。→ Task 4。

---

## Part A — Dozer(仓库 `../dozer`,worktree `../dozer-product-id`,分支 `feat/product-id`,从 `main` 切出)

### Task 1: 基线、黄金测试、`ProductId`

**Files:**
- Modify: `crates/dozer-core/src/paths.rs`、`crates/dozer-core/src/lib.rs`
- Create: `crates/dozer-core/tests/paths_golden.rs`
- Test: `crates/dozer-core/src/paths.rs`

**Interfaces:**
- Produces:
  - `pub struct ProductId`,字段(均为 `&'static str` 或私有):`name`、`session_env`、`hook_bin`、`socket_file`、`db_file`,以及私有的目录来源(`ProjectDirs` 三元组或固定的 `(config, state)` 路径对)。
  - `ProductId::dozer() -> ProductId`(`name "dozer"`、`session_env "DOZER_SESSION_ID"`、`hook_bin "dozer-hook"`、`socket_file "dozerd.sock"`、`db_file "dozer.db"`、`ProjectDirs("ai","byteboy","dozer")`)。
  - `ProductId::custom(name, session_env, hook_bin, socket_file, db_file, dirs: ProductDirs) -> ProductId`;`pub enum ProductDirs { Project { qualifier: &'static str, organization: &'static str, application: &'static str }, Fixed { config: PathBuf, state: PathBuf } }`。
  - 方法:`config_dir(&self) -> PathBuf`、`state_dir(&self) -> PathBuf`、`socket_path(&self) -> PathBuf`、`logs_dir(&self) -> PathBuf`、`db_path(&self) -> PathBuf`(`state_dir().join(db_file)`)。
  - 进程级:`pub fn install(product: ProductId)`(只能成功一次,重复安装同值返回 `Ok`,不同值返回 `Err`)、`pub fn product() -> &'static ProductId`(未安装时返回并固定为 `ProductId::dozer()`);原有 `config_dir()` / `state_dir()` / `socket_path()` / `logs_dir()` 四个自由函数改为转调 `product()`,签名不变。

- [ ] **Step 1: 记录基线。** worktree 里 `cargo test --workspace 2>&1 | grep "^test result"`,把每个 crate 的通过数记入执行记录;后面每个 Dozer 任务都与它比对。
- [ ] **Step 2: 写黄金测试(在改任何代码之前)。** `tests/paths_golden.rs`:在当前代码上把 `config_dir()` / `state_dir()` / `socket_path()` / `logs_dir()` 的**值**(`HOME` 设为固定的临时目录)序列化成期望字符串写进测试;改造后同一个测试必须仍通过。再加 `product_default_is_dozer`:未调用 `install` 时 `product().name == "dozer"`、`session_env == "DOZER_SESSION_ID"`。
- [ ] **Step 3: 写失败测试**(`paths.rs`):`custom_product_derives_its_own_paths`(`ProductDirs::Fixed` 的实例,`socket_path()` 是 `state/diggerd.sock`、`db_path()` 是 `state/digger.db`、`logs_dir()` 是 `state/logs`)、`two_products_never_share_paths`(`dozer()` 与一个 `Fixed` 的 digger 实例,四个路径两两不相交)、`install_twice_with_a_different_product_is_an_error`、`install_twice_with_the_same_product_is_ok`(注意:`install` 是进程级,涉及它的测试放在各自独立的集成测试文件里,避免同进程互相污染)。
- [ ] **Step 4: 运行确认失败**,然后实现。`cargo test -p dozer-core` 通过,黄金测试通过。
- [ ] **Step 5: 验证。** `cargo test --workspace` 总数 = 基线 + 新增。
- [ ] **Step 6: 提交** `feat(core): ProductId,默认 dozer,路径按产品身份推导`。

### Task 2: 会话环境变量与 Agent 配置的产品化

**Files:**
- Modify:`crates/dozerd/src/{session,group_adapter,summary_provider}.rs`、`crates/dozer-hook/src/{main,forward,aider_launcher,goose}.rs`、`crates/dozer-mcp/src/server.rs`,以及 `grep -rn '"DOZER_SESSION_ID"' crates` 命中的其余文件
- Test: 各文件已有测试 + 新增

**Interfaces:**
- Consumes:Task 1 的 `product().session_env`。
- Produces:`dozer_core::paths::session_env() -> &'static str`(= `product().session_env`);**除 `ProductId::dozer()` 的定义与测试夹具外,代码里不再出现字面量 `"DOZER_SESSION_ID"`**。

- [ ] **Step 1: 写失败测试**:`no_literal_session_env_name_outside_the_product_definition`(一个扫描源码的测试,放在 `dozer-core/tests/`:遍历 `crates/*/src/**/*.rs`,断言字面量 `"DOZER_SESSION_ID"` 只出现在 `dozer-core/src/paths.rs` 与带 `#[cfg(test)]` 的测试代码里;命中其它位置则失败并列出文件)。
- [ ] **Step 2: 运行确认失败**,列出的文件就是需要改的清单(约 29 处)。
- [ ] **Step 3: 逐处替换为 `dozer_core::paths::session_env()`。** `cmd.env(...)`、`cmd.env_remove(...)`、`std::env::var(...)` 三类用法都改;`env_remove` 的位置(`group_adapter`、`summary_provider`)要一并改,否则 Digger 的嵌套会话会泄漏变量。`DOZER_AIDER_*`、`DOZER_ORIG_ZDOTDIR`、`DOZER_ZDOTDIR_WRAPPER`、`DOZER_SHELL_INTEGRATION` 等**不改**:它们指向各守护进程自己状态目录里的文件,不会跨产品串线。`DOZER_*_SETTINGS` / `DOZER_*_MCP_CONFIG` 这类测试覆盖变量也不改。
- [ ] **Step 4: 验证。** 扫描测试通过;`cargo test --workspace` 总数守恒(加新增);黄金测试通过。
- [ ] **Step 5: 提交。**

### Task 3: hook 的产品化与并排共存

**Files:**
- Modify:`crates/dozer-hook/src/install.rs`、`opencode_install.rs`、`goose_install.rs`、`aider_launcher.rs`、`main.rs`,`crates/dozer-app/src/workspace/hook.rs`(调用点)
- Test:`crates/dozer-hook/src/install.rs`(新增并存测试)

**Interfaces:**
- Produces:
  - `install::entry_is_product(entry: &Value, product: &ProductId) -> bool`:命令字符串含 `product.hook_bin` 的连字符形式或下划线形式(`dozer-hook` / `dozer_hook`、`digger-hook` / `digger_hook`)即为该产品的条目。原 `entry_is_dozer` 保留为 `entry_is_product(entry, &ProductId::dozer())` 的包装。
  - `install::run_at_with_exe(path: &Path, agent: &str, install: bool, exe: &str) -> i32` 的签名不变,内部按 `product()` 识别与写入;opencode / goose 插件文件名与目录标识同样取自 `product().name`(具体清单由实现者逐个列出,写进提交说明)。
  - hook 二进制入口:`pub fn run_main(product: ProductId) -> !`(`dozer-hook` 的 `main` 变成 `run_main(ProductId::dozer())`;`digger-hook` 在 Digger 仓库里调同一个函数)。`run_main` 第一件事 `paths::install(product)`。
  - hook 事件转发:环境里没有 `product().session_env` 时**静默返回、退出码 0、不写 stderr**(沿用现有的"缺少变量就退出"行为,只是变量名取自产品身份)。

- [ ] **Step 1: 写失败测试**(`install.rs`,每条用临时配置文件):
  - `digger_and_dozer_entries_coexist_in_one_settings_file`(先以 dozer 身份装,再以 digger 身份装,同一事件的 hook 数组里有两条、各含自己的命令);
  - `reinstalling_one_product_does_not_duplicate_or_touch_the_other`(各装两次,条目数仍各为 1,另一个产品的条目字节不变);
  - `uninstalling_one_product_leaves_the_other_intact`;
  - `a_foreign_entry_with_a_similar_name_is_not_recognised`(命令里含 `my-dozer-hook-wrapper` 之类**不**应被识别为 dozer 条目——若现有 `contains("dozer-hook")` 逻辑本来就会命中,先在现状上跑出结果,**保持现状行为**,把该行为写成测试,并把"识别过宽"记入报告交用户裁决,不在本计划里顺手改);
  - `paths_with_spaces_are_quoted_for_either_product`(沿用原 `/Applications/Dozer AI Coder.app/...` 回归用例,各加一个 `Digger.app` 版本);
  - `hook_without_own_session_env_exits_zero_and_writes_nothing`(`run_main` 的纯逻辑部分抽成 `handle(env: &dyn Fn(&str)->Option<String>, ...) -> ExitCode`,断言缺变量时返回成功且输出为空)。
- [ ] **Step 2: 运行确认失败。** `cargo test -p dozer-hook`。
- [ ] **Step 3: 实现。** `entry_is_dozer` → `entry_is_product`;安装与卸载只处理 `product()` 的条目;原有回归测试(`run_at_with_exe_quotes_paths_containing_spaces` 等)保持原样且仍通过。
- [ ] **Step 4: 验证。** `cargo test -p dozer-hook -p dozer-app` 总数守恒(加新增);`workspace/tests.rs` 里的 `ensure_hook_installed_*` 全部原样通过。
- [ ] **Step 5: 提交。**

### Task 4: 协议版本握手

**Files:**
- Modify:`crates/dozer-core/src/protocol.rs`、`crates/dozerd/src/server.rs`、`crates/dozer-client/src/lib.rs`、`crates/dozer-hook/src/forward.rs`
- Test:`crates/dozer-core/src/protocol.rs`、`crates/dozer-client/src/lib.rs`

**Interfaces:**
- Produces:
  - `dozer_core::protocol::PROTOCOL_VERSION: u32 = 1`。
  - `Request::Hello { protocol: u32 }` 与 `Reply::Hello { protocol: u32, product: String }`(新增变体,其余变体与序列化不变)。
  - `dozer_client::Client::check_version(&self) -> Result<(), ClientError>`:不匹配返回 `ClientError::ProtocolMismatch { client: u32, daemon: u32 }`,其 `Display` 为可读文案(含两个版本号与"请把 Digger/Dozer 与它的守护进程升级到同一版本")。`Client` 在首个业务请求前调用一次(结果缓存)。
  - hook:`forward` 在发事件前先发 `Hello`;版本不符或守护进程不识别 `Hello`(旧守护进程)时**静默返回**并写一条日志,不输出 stderr、退出码 0。

- [ ] **Step 1: 写失败测试:** `hello_round_trips_through_serde`、`old_daemon_that_does_not_know_hello_yields_a_readable_mismatch`(用一个只回复 `Reply::Error` 的假服务端)、`client_refuses_requests_after_a_protocol_mismatch_with_a_readable_error`、`hook_forward_is_silent_on_mismatch`。
- [ ] **Step 2: 运行确认失败。**
- [ ] **Step 3: 实现。** 注意:这是新增变体,**旧的请求/回复的字节编码不得改变**(为此加一个序列化黄金测试:对每个既有变体各取一个样例,与改造前的 JSON 字符串逐字比较)。
- [ ] **Step 4: 验证。** `cargo test --workspace` 总数守恒(加新增)。
- [ ] **Step 5: 提交。**

### Task 5: `dozerd::run(DaemonConfig)` 与功能开关

**Files:**
- Modify:`crates/dozerd/src/main.rs`(缩成参数解析 + 调用)、`crates/dozerd/src/lib.rs`(新增 `daemon` 模块导出)
- Create:`crates/dozerd/src/daemon.rs`、`crates/dozerd/tests/product_isolation.rs`
- Test:`crates/dozerd/src/daemon.rs`、`crates/dozerd/tests/product_isolation.rs`

**Interfaces:**
- Produces:
  - `pub struct DaemonFeatures { pub app_host: bool, pub transcript_backfill: bool, pub summaries: bool, pub task_poller: bool, pub ide_bridge: bool }`,`impl Default`(全 `true`,即 Dozer 现状);`DaemonFeatures::minimal() -> Self`(全 `false`,Digger 用)。
  - `pub struct DaemonConfig { pub product: ProductId, pub socket: PathBuf, pub features: DaemonFeatures }`,`DaemonConfig::for_product(product: ProductId) -> Self`(`socket = product.socket_path()`、`features = DaemonFeatures::default()`)。
  - `pub async fn run(config: DaemonConfig) -> anyhow::Result<()>`:搬自 `main.rs` 的装配;数据库路径取 `config.product.db_path()`;`features.app_host == false` 时传 `bytehost_apps::service::AppService::unavailable("app host runs in-process in this product")`(该构造函数已存在,`server::Stores.apps` 类型不变);`transcript_backfill == false` 时跳过启动回填与 Codex SQLite 摄取;`summaries == false` 时不构造 `summary_service`、不启动 worker(`Stores.summary_jobs` 仍可构造,只是没有消费者);`task_poller == false` 时不 `spawn`;`ide_bridge == false` 时 `server::serve` 不启动 IDE 桥接、不 `sweep_stale_locks`。**`run` 内部第一件事 `paths::install(config.product.clone())`**,其后 `main.rs` 里所有 `dozer_core::paths::*` 调用保持原样(它们现在读到的就是该产品)。
  - `dozerd` 二进制的 `main`:解析 `--socket` / `--version`(保持原有行为与文案),然后 `run(DaemonConfig::for_product(ProductId::dozer()))`。

- [ ] **Step 1: 写失败测试**(`daemon.rs`):`default_features_enable_everything`、`minimal_features_disable_everything`、`for_product_uses_the_products_socket_and_db`(用 `Fixed` 目录的实例,`socket` 与 `db_path` 落在其状态目录)。
- [ ] **Step 2: 写进程级隔离测试**(`tests/product_isolation.rs`,两个**独立的 OS 进程**,不是同进程两个实例,因为 `install` 是进程级):用一个测试辅助二进制(`crates/dozerd/examples/product_daemon.rs`,参数 `--name <n> --root <dir>`,按它构造 `ProductId::custom(...)` 与 `DaemonFeatures::minimal()` 后调 `run`)起两个守护进程,一个沿用 `ProductId::dozer()` 的 `Fixed` 目录版本、一个名为 `digger` 的版本,根目录是两个不同的临时目录。断言:`isolated_roots_sockets_and_dbs_do_not_overlap`(两个 socket、两个数据库文件、两个日志目录互不包含)、`events_do_not_cross_between_two_daemons`(对 A 的 socket 发一个会话事件,B 的订阅者收不到;反之亦然)、`second_daemon_on_the_same_root_exits_without_touching_data`(同一 root 起第二个,进程非零退出,数据库文件字节不变)、`app_host_is_unavailable_when_the_feature_is_off`(minimal 的守护进程对 `AppRequest::List` 回复 `Unavailable`,且没有占用任何端口)。
- [ ] **Step 3: 运行确认失败。** `cargo test -p dozerd daemon:: --test product_isolation`。
- [ ] **Step 4: 实现。** 搬装配;逐项接开关。**默认值(`Default`)下 `dozerd` 的启动顺序、日志、副作用与改造前逐项相同**——对照改造前的 `main.rs`,在提交说明里列出"开关位置清单"。
- [ ] **Step 5: 验证。** `cargo test -p dozerd -p dozer-core -p dozer-hook -p dozer-client -p dozer-app` 总数守恒(加新增);手动起 `dozerd`(`--socket` 指向临时路径,`HOME` 指向临时目录)确认能正常响应 `list`,Ctrl-C 干净退出。
- [ ] **Step 6: 提交。**

### Task 6: 清理、全量验证与发 tag

- [ ] **Step 1: 清理与自检。** `grep -rn '"DOZER_SESSION_ID"\|"dozerd.sock"\|"dozer.db"' crates` 只命中 `ProductId::dozer()` 的定义与测试夹具;`dozerd/src/main.rs` 不超过 40 行。
- [ ] **Step 2: 全量。** `cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --check`;总数与基线一致(加新增);黄金测试与协议序列化黄金测试通过。
- [ ] **Step 3: 请用户在 Dozer 上目测验收**:正常启动 Dozer,新建 Claude 会话,状态点随 hook 事件更新;重启 Dozer 会话仍在;`~/.claude/settings.json` 里的 hook 条目与改造前一致。**未验收不继续。**
- [ ] **Step 4: 合并到 Dozer `main`、切 `libs-2026-10-<日期>x` tag、推送——向用户确认后执行。** 该 tag 里 bytehost / byteui / bytegit 版本必须与 Digger 当时用的一致,逐个 crate 核对;`dozerd`、`dozer-hook`、`dozer-client`、`dozer-core` 要能被 Digger 经 git 依赖编译(新增的依赖不得含本地路径)。

---

## Part C — Digger(worktree `../digger-s6a`,分支 `feat/diggerd`,**从 `feat/agent-shell` 的 `fd17623` 切出**,见顶部 2026-10-22 修订)

### Task 7: `digger-paths`、`diggerd`、`digger-hook`

**Files:**
- Create:`crates/paths/{Cargo.toml,src/lib.rs}`(包名 `digger-paths`)、`crates/diggerd/{Cargo.toml,src/main.rs}`、`crates/digger-hook/{Cargo.toml,src/main.rs}`
- Modify:`Cargo.toml`(workspace 依赖:`dozerd`、`dozer-core`、`dozer-hook`、`dozer-client` 指向 Task 6 的 tag)、`Cargo.lock`、`crates/desktop/src/paths.rs`(改为转调 `digger-paths`)、`scripts/check-host-deps.sh`(`HOST_ADAPTER_CRATES` 加 `diggerd digger-hook`)、`shared-deps.toml`(按现有格式登记新增的共享库)
- Test:`crates/paths/src/lib.rs`

**Interfaces:**
- Produces:
  - `digger_paths::{config_dir() -> PathBuf, state_dir() -> PathBuf, projects_db_path() -> PathBuf, legacy_projects_db_path() -> PathBuf}`:`config_dir()` 沿用现在 `digger-desktop::paths::config_dir()` 的逻辑(`DIGGER_DATA_DIR` 优先,否则 `ProjectDirs("ai","byteboy","digger")`),`state_dir()` 同现有;`projects_db_path() = state_dir().join("digger.db")` **必须与 `diggerd` 实际使用的数据库路径相等**(测试断言);`legacy_projects_db_path() = config_dir().join("projects.db")`(迁移用)。
  - `digger_paths::product() -> dozer_core::paths::ProductId`:`custom("digger", "DIGGER_SESSION_ID", "digger-hook", "diggerd.sock", "digger.db", ProductDirs::Fixed { config: config_dir(), state: state_dir() })`。
  - `diggerd` 入口:`digger_paths::product()` → `dozerd::run(DaemonConfig { product, socket: product.socket_path(), features: DaemonFeatures::minimal() })`;支持 `--version`。
  - `digger-hook` 入口:`dozer_hook::run_main(digger_paths::product())`。

- [ ] **Step 1: 写失败测试**(`digger-paths`):`projects_db_path_equals_the_daemons_db_path`(`product().db_path() == projects_db_path()`)、`data_dir_env_moves_every_path_together`(设 `DIGGER_DATA_DIR` 到临时目录后 `config_dir` / `state_dir` / `socket_path` / `db_path` 都在其下)、`socket_and_session_env_are_digger_specific`(`socket_path()` 文件名为 `diggerd.sock`、`session_env == "DIGGER_SESSION_ID"`、`hook_bin == "digger-hook"`)、`paths_do_not_overlap_with_dozers_defaults`(与 `ProductId::dozer()` 的四个路径互不包含,`HOME` 设为同一临时目录)。
- [ ] **Step 2: 运行确认失败。** `cargo test -p digger-paths`。
- [ ] **Step 3: 实现三个 crate;`digger-desktop::paths` 改为 `pub use` 转调,保持其对外函数名与 `PROJECTS_DB_FILE` 之外的行为不变**(`PROJECTS_DB_FILE` 改动见 Task 8)。
- [ ] **Step 4: 验证。** 依赖门禁:`Cargo.lock` 重复检查输出 `ok: one version each`;`bash scripts/check-shared-deps.sh`、`bash scripts/check-host-deps.sh`、`bash scripts/check-host-deps.sh --self-test` 通过;`cargo test --workspace`、`cargo clippy --workspace --all-targets`、`cargo fmt --check`。依赖升级与新 crate 分成两个提交(先只升依赖、过门禁,再加代码)。
- [ ] **Step 5: 提交。**

### Task 8: `digger.db` 单一文件与迁移

**Files:**
- Modify:`crates/desktop/src/paths.rs`(`PROJECTS_DB_FILE` 改为 `digger.db`)、`crates/desktop/src/projects/mod.rs`(`open_in` 里先迁移再打开)、`crates/store/src/projects.rs`
- Create:`crates/store/src/migrate.rs`
- Test:`crates/store/src/migrate.rs`、`crates/store/src/projects.rs`

**Interfaces:**
- Produces:`digger_store::migrate::migrate_legacy_projects(legacy: &Path, target: &Path) -> Result<MigrationOutcome, ServiceError>`;`pub enum MigrationOutcome { NothingToMigrate, Migrated { projects: usize, memories: usize }, AlreadyMigrated }`。迁移用 SQLite 的 `ATTACH` 在**同一事务**里把 `legacy` 的项目表与记忆表行**按原 id** 拷入 `target`(两库的表结构都来自 `dozer-store-project`,由它负责建表),成功后把 `legacy` 改名为 `projects.db.migrated`(保留不删除,D5)。`ProjectsState::open_in(dir)` 在打开 `digger.db` **之前**调用它。

- [ ] **Step 1: 写失败测试**(`migrate.rs`,临时目录):`no_legacy_file_means_nothing_to_migrate`、`legacy_rows_are_copied_with_their_original_ids`(旧库建 3 个项目、各 2 条记忆,迁后 `digger.db` 里 id、路径、名称、记忆标题逐项相同)、`migration_is_idempotent`(第二次调用返回 `AlreadyMigrated` 或 `NothingToMigrate`,行数不变)、`a_failure_midway_leaves_no_partial_digger_db_and_keeps_the_legacy_file`(用只读的目标目录或注入的失败点制造失败,断言 `digger.db` 要么不存在要么与迁移前字节一致,`projects.db` 仍在原位)、`migrated_legacy_file_is_renamed_not_deleted`、`ids_do_not_collide_when_digger_db_already_has_projects`(目标库已有 id 冲突的行时,**不覆盖**已有行并返回错误,不部分写入)。
- [ ] **Step 2: 写并发测试**(`projects.rs`):`two_writers_on_one_digger_db_never_lose_writes_or_hit_locked`(两个线程各持一个 `ProjectStore` 连接,打开同一文件,各做 1000 次交错的 `open_project` / `rename` / 记忆写入,最后总行数等于两边写入之和,没有任何 `database is locked` 错误;连接的 `busy_timeout` 由 rusqlite 默认 5 秒提供;WAL 由 Digger 在 `open_in` 里先设(见顶部修订的"SQLite 并发"):测试里先用同样的一条一次性连接开 WAL 再各开 store,另加 `open_in_switches_digger_db_to_wal`(`open_in` 后 `PRAGMA journal_mode` 查询结果为 `wal`)。**不改 Dozer;若仍出现 `database is locked`,停下报告。**)
- [ ] **Step 3: 运行确认失败。**
- [ ] **Step 4: 实现。** `PROJECTS_DB_FILE` 改为 `"digger.db"`;`open_in` 里:迁移失败时给出 `Unavailable` 提示"项目功能不可用:迁移失败 …",**不静默回退**到临时库(与现有 `ProjectHost::open` 的口径一致)。WAL 设置失败只记日志、不算迁移失败。
- [ ] **Step 5: 验证。** `cargo test --workspace` 全绿。
- [ ] **Step 6: 提交。**

### Task 9: 拉起与生命周期

**Files:**
- Create:`crates/desktop/src/daemon.rs`
- Modify:`crates/desktop/src/main.rs`、`crates/desktop/src/product.rs`(启动时调 `ensure`,把提示接进现有的提示条)、`crates/desktop/Cargo.toml`(加 `dozer-client`)
- Test:`crates/desktop/src/daemon.rs`

**Interfaces:**
- Produces:`daemon::Daemon`:`new() -> Self`(用 `digger_paths::product().socket_path()` 构造 `dozer_client::Client`);`async fn ensure(&self) -> Result<(), DaemonError>`(先 `check_version`+`list`;连不上就 `spawn()`,隔 1 秒重试最多 3 次,仿 Dozer `runtime::ensure_daemon`);`fn spawn(exe_dir: Option<&Path>) -> std::io::Result<()>`(在 `current_exe` 同目录找 `diggerd`,找不到退回 PATH,**不继承 stdin**);`DaemonError` 的 `Display` 为可读中文。**关闭 Digger 窗口不终止 `diggerd`**(会话要随守护进程存活,与 Dozer 一致)。

- [ ] **Step 1: 写失败测试:** `missing_diggerd_binary_yields_a_readable_error_and_does_not_hang`(把 `exe_dir` 指向空目录、PATH 清空,`ensure` 在有限时间内返回 `Err`,错误文案含 `diggerd`)、`ensure_returns_ok_when_a_daemon_already_answers`(用测试辅助二进制或假服务端)、`protocol_mismatch_is_reported_as_such`(假服务端回不同的 `Hello`)、`spawn_does_not_inherit_stdin`。
- [ ] **Step 2: 运行确认失败。**
- [ ] **Step 3: 实现,并在 `product.rs` 的启动序列里异步调 `ensure`;失败时只显示提示,主窗口照常。**
- [ ] **Step 4: 并入 S6d-1 的 Task 4b(`AgentShell` 接真实 `ensure`)。** 先写失败测试,再实现:
  - `product.rs`:`startup_ensure_result_feeds_the_agent_shell`(启动时的 `ensure` 结果以 `AgentShellMsg::Ensured` 送进 `AgentShell`,**不再调用第二次** `ensure`——用计数的假 `ensure` 断言);`retry_runs_the_real_ensure_once_and_a_second_click_while_checking_is_ignored`(`Retry` 注入真实 `Daemon::ensure`,连点两次只发起一次);`a_failed_ensure_shows_the_readable_reason_and_the_other_panels_still_work`。
  - `AgentShell` 的 `Default` 状态由 `Ready` 改为 `Unknown`(启动序列开始时置 `Checking`,收到 `Ensured` 后 `Ready`/`Unavailable`);S6d-1 里 `the_default_shell_is_ready_because_this_phase_has_no_daemon_to_ask` 随之改名并改断言,**在报告里列出这条改动**;同时去掉 S6d-1 为 `Ensured`/`Unknown` 加的 `#[allow(dead_code)]`。
  - `on_visible`、`body` 的调用点不变。
- [ ] **Step 5: 验证。** `cargo test --workspace`。
- [ ] **Step 6: 提交。**

### Task 10: 打包与发布脚本

**Files:**
- Modify:`scripts/package-macos.sh`、`scripts/release.sh`、`scripts/check-host-deps.sh`(若 Task 7 未完成)
- Test:脚本自检

- [ ] **Step 1: `package-macos.sh`** 在构建 `digger` 之外同时构建 `diggerd`、`digger-hook`(`cargo build --release -p diggerd -p digger-hook -p digger-desktop --message-format=json`,按 Dozer `build-macos-app.sh` 的做法用 jq 取各自路径,不猜 `target/` 布局),拷进 `Digger.app/Contents/MacOS/`;注释里照抄 Dozer 脚本对 sibling 二进制缺失事故的说明(缺了 `digger-hook` 时 Agent 状态整个失效)。
- [ ] **Step 2: `release.sh` 的校验段**加:`test -x "$OUT/Contents/MacOS/diggerd"`、`test -x ".../digger-hook"`;缺任何一个就失败并说明。
- [ ] **Step 3: 验证。** `bash scripts/release.sh --skip-tests --allow-dirty --no-bump --dry-run` 门禁通过;再 `--no-bump --skip-tests --allow-dirty` 真打包一次,`ls Digger.app/Contents/MacOS` 含三个二进制,`codesign --verify --deep --strict` 通过。
- [ ] **Step 4: 提交。**

### Task 11: 全量验证、同机并存回归与文档

- [ ] **Step 1:** `cargo build --workspace`、`cargo test --workspace`(记录通过数)、`cargo clippy --workspace --all-targets`(零告警)、`cargo fmt --check`、`bash scripts/check-shared-deps.sh && bash scripts/check-host-deps.sh && bash scripts/check-host-deps.sh --self-test && bash scripts/check-shell-regions.sh`、`Cargo.lock` 重复检查。
- [ ] **Step 2: 同机并存回归**(命令行部分你做,需要 GUI 点击的项写进报告"待用户",不阻塞;用隔离的 `DIGGER_DATA_DIR` 与临时 `HOME`,不碰真实数据。Dozer 一侧用 `dozer-mcp-product` worktree 里 `cargo build` 出的二进制或主工作区 `target/aarch64-apple-darwin/bundle/macos/` 下的 0.5.303 包,**同样跑在临时 `HOME` 下**,不接用户机器上常驻的 `dozerd`):① 启动 Digger,确认 `diggerd` 进程出现、`<DIGGER_DATA_DIR>/state/diggerd.sock` 存在、数据目录里是 `digger.db` 且**没有** `dozer.db`;② 同时启动 Dozer,确认两个 socket、两个数据库、两个日志目录互不重叠,两个守护进程都在;③ 在临时 `HOME` 下用 `digger-hook install claude`、`dozer-hook install claude`,检查 `~/.claude/settings.json` 里两个产品的条目并存;④ 从旧版本升级:预置一个旧 `projects.db`,启动后项目与记忆都在 `digger.db` 里、旧文件被改名为 `projects.db.migrated`;⑤ 退出 Digger 后 `diggerd` 仍在运行,再启动能连上。逐项记录,任一项不符先停下,不要为了"过"而改验证。
- [ ] **Step 3: 文档。** 新增 `docs/adr/ADR-015-product-owned-daemon.md`(Digger 拥有 `diggerd` / `digger.db`,与 Dozer 只共享代码;hook 并排共存;`digger.db` 由守护进程与进程内存储共用;`projects.db` 迁移规则),并在 ADR-014 的 Consequences 里加一句指向它;`2026-10-18-agent-panel-s6-design.md` 的状态改"S6a 已实现"并补"实现结果";`2026-10-14-window-shell-design.md` 的相关行同步。
- [ ] **Step 4: 作废**(见顶部修订:不合并、不清理,停在 `feat/diggerd`)。
- [ ] **Step 5: 汇报:** 测试通过数(基线 396 + 新增)、依赖图结论(`cargo tree -d` 与基线对比)、并存回归结论、D1–D7 最终落地情况、WAL 在 Digger 侧设置的效果与并发测试结论、hook 识别过宽的现状(若有)、哪些没验。

---

## Self-Review

- **Spec 覆盖:** §6.1 `ProductId` → Task 1;§6.2 `dozerd::run` → Task 5;§6.3 hook 并排共存与参数化 → Task 2、3;§6.4 `digger.db` 与迁移 → Task 7、8;§6.7 打包与拉起(S6a 部分)→ Task 9、10;§8 的隔离、hook 配置、并发写入、迁移四类钉死测试 → Task 5、3、8、8;G1–G2、G5 → Task 5–11;Q1(后台默认关闭)→ D7 与 Task 5 的 `DaemonFeatures`;Q2(并发)→ Task 8 Step 2 与退路说明;Q3(协议不一致处理)→ Task 4。
- **类型一致:** `ProductId` / `ProductDirs` / `product()` 在 Task 1 定义,Task 2、3、5、7 引用同名;`DaemonConfig` / `DaemonFeatures` 在 Task 5 定义,Task 7 引用;`digger_paths::product()` 在 Task 7 定义,Task 8、9 引用;`Client::check_version` 在 Task 4 定义,Task 9 引用。
- **风险点:** ① `dozerd::run` 内部大量 `dozer_core::paths::*` 自由函数调用依赖进程级 `install` 先执行,Task 5 要保证 `install` 在任何 `paths::*` 调用之前;② 隔离测试必须是两个 OS 进程(Task 5 Step 2),同进程不可行;③ Dozer 侧 `dozer-store-project` 是否已设置 WAL / `busy_timeout` 我没有核实,Task 8 Step 2 的并发测试会暴露它;④ `entry_is_product` 沿用现有的子串识别,可能过宽,计划要求保持现状并上报,不在本期修;⑤ Digger 首次依赖 `dozerd` 库会带进一大批依赖(`bytehost-apps` 的 server 特性、`git2`、`rusqlite` 等),`check-shared-deps` 与 `Cargo.lock` 重复检查可能暴露版本分歧,Task 7 Step 4 要求依赖升级单独成提交先过门禁;⑥ tag 与推送是对外动作;Digger 部分依赖 S5a 已合并。

---

## 复审追加(2026-10-20)

Part A 复审结论见 `docs/superpowers/reviews/2026-10-20-s6a-product-id-review.md`:**合并前**还要完成下面的任务,逐条的位置、修改要求与验收以审核文档为准。Part C(Task 7–11)在这些任务通过复审并合并之后才开始。

### Task 6b: 复审修复 R1–R4(Dozer,同一 worktree `../dozer-product-id`,分支 `feat/product-id`)

- [ ] **R1 opencode 插件 import 路径**:安装后的 import 串由 `lib_dir_name` 动态生成;`opencode_install` / `goose_install` 增加接受 `ProductId` 的内核函数;新增非 Dozer 身份的安装/并存/单独卸载测试。
- [ ] **R2 客户端握手接入**:`Client` 在首个业务请求前自动 `check_version`(结果缓存);校验 `Reply::Hello.product`。**兼容策略(用户 2026-10-20 已确认):** 版本号明确不一致、或产品名不符 → 拒绝;**同一产品**的旧守护进程(不认识 `Hello`、只回 `Reply::Error`)→ 放行并记 warn。补上缺失的 `client_refuses_requests_after_a_protocol_mismatch_with_a_readable_error`;`old_daemon_that_only_replies_error_is_reported_as_too_old` 改为断言放行,注释写明原因;不再有产生路径的 `DaemonTooOld` 一并删除。
- [ ] **R3 日志目录按产品参数化(用户 2026-10-20 选方案 A)**:opencode / goose / aider 三家的 `~/.dozer/agents/<agent>` 根目录由 `paths::product().name` 推导,Dozer 默认值字节不变;hook 写入与 `dozerd` 摄取只经 `agent_paths` 取路径。Claude / codebuddy / codex / v8agent 不改。
- [ ] **R4 `DaemonFeatures` 非法组合**:`task_poller` 开而 `summaries` 关时返回可读 `Err` 或明确降级,不 panic。
- [ ] **验证:** `cargo test --workspace` 总数在 3356 的基础上只增不减;`cargo clippy --workspace --all-targets` 无新增警告;提交说明里写明每条 R 的处置。`cargo fmt --check` 与 `check-log-scope.sh` 在 `main` 上本就不通过,不计入本任务。
- [ ] **提交。** 然后交审核人复审。复审通过后才进入 Task 6 Step 3(用户目测验收)与 Step 4(合并 / 切 tag / 推送,须向用户确认)。
