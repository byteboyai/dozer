# 交接任务 9:S7b Digger 接入会话面板(列表、历史导入、回合结束联动)

> 给开发 agent 的任务说明,写于 2026-10-26。**本任务只动 Digger 仓库。** 范围是 S7b 计划的 Task 1–5(**不含**原 Task 1 Step 1「升依赖」,已经做完),完工后**停下等用户在 Digger 上目测验收**。合并、发布、安装都不在本任务内。审阅详情 webview 是 S7c,**不在本任务内**。

**目标:** Digger 右栏出现「会话」面板(`digger.conversations`,排在 Agent 之后):左右配对,会话列表(搜索、Agent 筛选、分页、卡片)是配对的第一块,内容块先放「审阅详情即将可用」占位。数据来自 `diggerd` / `digger.db`;Agent 回合结束时列表自动刷新;历史会话经一个**明确的按钮**导入,不静默复制。

**先读(只读,按顺序):**

1. `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer/docs/superpowers/plans/2026-10-25-s7-s8-refresh.md`:**修订总览,优先于原稿**。重点 §3「通用」11 条与 §3 的「S7b」一节。
2. 设计稿(只读):Digger 仓库 `docs/superpowers/specs/2026-10-18-conversations-panel-s7-design.md`(§6.3 登记与布局、数据、历史导入、键盘;§6.4 S7b;§8 Q1–Q3)。
3. **计划(照这份做):** Digger 仓库 `docs/superpowers/plans/2026-10-18-conversations-s7b-digger-list.md`。先读 Global Constraints 与 Review Focus,每个任务先写失败测试。
4. 样板:S3b-1 的 `crates/desktop/src/adapters/todo.rs`(每项目状态、`Services` 构造、`UnavailableService` 共用)和 `product.rs` 里待办面板的登记、可见性、折叠处理;S6d 的 `adapters/agent.rs`(`drain_host_events`、通知条)、`adapters/native_menu.rs`(原生菜单转换样例)。
5. 库的真实接口(**以它为准,不以计划为准**):`/Users/chrischiang/Projects/CoralProjects/byteboy/dozer/crates/dozer-panel-conversations/src/`(`adapter.rs`、`update.rs`、`state.rs`、`view.rs`),报告 `docs/superpowers/reviews/2026-10-26-s7a-report.md`。

## 基线

- Digger 仓库 `/Users/chrischiang/Projects/CoralProjects/byteboy/digger`,基线是 `main` 的 `682fc53`(含 S3b-1、S3b-2、重钉)。开工先 `git log --oneline -3` 与 `git status` 看清楚。
- 新建 worktree `../digger-s7b`,分支 `feat/conversations-list`,从 `main` 切出。
- **不要动主工作区。** 它有用户未提交的改动:13 个已删除的 S6 文档(`docs/superpowers/` 下)和未跟踪的 `.qoder/`。暂存时只用具体路径,**不要** `git add -A` / `git add .`。不碰 `feat/preview-pairing`、`digger-p4`。
- 全 workspace 测试基线:2026-10-26 在干净快照里是 **627 通过 / 0 失败 / 0 忽略**。开工先自己重跑一遍记基线,数字写进第一个提交说明。
- 已知偶发失败:`apps::tests::closing_during_startup_waits_and_releases_the_root`,并发重编译时偶发,单独跑稳定通过。遇到它或别的偶发失败:单独重跑量化(次数、失败数写进报告),不要靠重跑到绿。

## 依赖已升好,和计划不同的地方

- **依赖已经是 `libs-2026-10-26a`**(`Cargo.toml` 里 dozer 系列 18 处,`Cargo.lock` 里已有 `dozer-panel-conversations`)。Task 1 Step 1 只剩一件事:把 `dozer-panel-conversations` 加进 workspace 依赖表(同 tag)和 `crates/desktop/Cargo.toml`。**不要**再改任何 tag;`bash scripts/check-shared-deps.sh` 必须仍然通过。
- **`dozer-services` 已带新接口**:`ConversationService::get_conversation_turns`、`SummaryService`、`Services.summary`。Digger 的 `UnavailableService` 已补好占位实现。本任务要提供**真实的** `ConversationService` / `SummaryService` / `TranscriptService`(都是 `dozer_client::Client` 的实现),经 `Daemon::conversation_services()` 取。
- **库的真实接口和计划里的不同,逐项核对,缺口只报告、不在 Digger 里绕开(库不改):**
  - 更新入口是 `dozer_panel_conversations::update::update(ws_state, msg, &UpdateCtx, emit)`。`UpdateCtx { project_id, project_path, conversation, summary, handle, caps, locale, now_ms, relative_time }`,其中 `relative_time` 是回调 `&dyn Fn(u64, u64, &str) -> String`。刷新用 `spawn_refresh(...)`,激活用 `on_activate(&state, &ctx, emit)`。
  - `Message` 的变体包括 `AsyncResult(token, Box<Message>)`、`SessionsRefreshed`、`SessionOpen`、`ListMore`、`SearchInput`、`SearchSubmit`、`AgentFilterSelect/Open/Close`、`Hover`、`TextInputMenuOpen`、`SummaryGenerate`、`ReviewLoaded` 等。**结果消息要用 `Message::result_project()` 路由回正确的项目**,不要自己猜。
  - 一次性提示经库的 `take_outbox()` 排空(`Notice` 列表);请求经 `take_requests()`,目前只有 `ConversationsRequest::ShowAgentFilter { at, current, present }`。
  - 视图入口:列表 `view::conversations_list(app, ws_state, todos, caps, width, outer)`。`PanelDisplay<M>` 的方法比计划多:`menu_item`、`menu_shell`、`cursor_position`(给非 macOS 的 iced 回退菜单用)、`ConvHover` 只有 `SearchSubmit` 与 `More` 两个槽。
  - `TextInputMenuOpen`(搜索框右键菜单)Digger 没有对应设施:**不要新造**,映射为空操作,在报告里记一条已知缺位。
  - `WorkspaceState` 有 `search_focused()` / `set_search_focused(bool)`:字段只用于渲染(搜索框是否高亮),库内没有逻辑依赖它。见下面「搜索焦点」。
- **本期点击会话行的行为:** 库的 `SessionOpen` 会正常加载详情数据到状态里(S7c 才有 webview 可显示)。本期**不要**阻止它,也不要为它另写逻辑;内容块始终显示占位。点行不得报错、不得卡住,把「点行时库做了什么」写进报告,供 S7c 参考。

## 与计划相比的修订(总览已定,这里列最重要的)

- **Agent 筛选菜单用原生菜单。** macOS 用 `byteui::native_menu`(条目「全部」+ 各 Agent,当前项锁定;选中后发 `Message::AgentFilterSelect`),参照 `adapters/native_menu.rs`。**`Overlay` 枚举不为它增项,`overlays_open()` 不把它算作覆盖层。** 非 macOS 才回退到库自带的 iced 菜单(`set_agent_picker_open` + 视图里的菜单),回退路径只要求能编译。测试 `overlays_stay_mutually_exclusive_…` 改成只覆盖 iced 内的覆盖层。
- **`drain_host_events` 已存在**(`adapters/agent.rs`,`product.rs` 约 2595 行已在排空)。不要新增,只在排空处对 `TurnEnded` 调 `ConversationsPanel::on_turn_ended`;`StateChanged` 不触发刷新。
- **通用 11 条**(总览 §3)全部适用,尤其:`thread_local` 不得存订阅回调要读的状态;带提示的控件始终包 `Tooltip`;`UnavailableService` 与「一项真实其余占位」的 `Services` 构造复用 `adapters/project.rs` 的 `pub(crate) UnavailableService`,不再各写一份(参考 `todo.rs` 的 `services()`,注意现在要多填 `summary`)。
- **能力全关:** `ConversationsCapabilities::default()`。没有「生成总结」按钮,没有「关联待办」文字;`LinkedTodos` 用空实现。

## 搜索焦点与键盘(新增的重点)

- 搜索框是 iced `text_input`。Dozer 用每帧的 `CaptureConversationSearchFocus` 查真实焦点再写回库;Digger 没有现成的等价物。选一个最小办法让 `set_search_focused` 在搜索框真正获得/失去焦点时更新(照 Files 面板在 Digger 里的做法,或用 iced 的焦点查询 Operation),**做不到就如实报告**,代价只是高亮不随焦点变。不得为此改库。
- **敲搜索框时,字符绝不能进 Agent 终端。** 现在 `agent_keys_blocked()`(`product.rs` 约 1195 行)只看 Agent 自己的浮层,不看当前选中的面板。先读 `agent_key_message` 与订阅,弄清会话面板被选中、焦点在搜索框时按键会不会转给终端:会,就加入阻止条件并加测试;不会,把依据写进报告并加一条测试固定。
- 中文输入法(拼音、候选框位置)要人工验证一次,写进真机回归:S6d 里输入法要单独探针验证过,不要假设 iced 控件天然可用。

## 容易踩的坑

- **项目隔离:** 消息带项目 id;没有该项目状态的消息忽略;项目页签关闭后晚到的结果忽略。父子目录项目(`/a` 与 `/a/b`)各传各的 `cwd`,匹配语义在 `diggerd` 一侧,Digger 只传项目路径。若真机发现父目录项目看到子目录项目的会话,作为 Dozer 同样存在的行为报告,**不要在 Digger 里过滤**。
- **历史导入只能由按钮触发**,只针对当前项目,调用 `TranscriptService::backfill_project_transcripts(cwd)`(已确认 `Client` 实现了它);按钮旁固定一行说明会读取 `~/.claude/` 等位置的会话文件、把文本存进 `digger.db`。**`ProjectPanel` 里的 `NoTranscript` 占位保持不变。** 导入在后台任务里跑,`update` 本身不得阻塞(计划里有 200ms 阻塞假服务的测试)。
- **「已导入」标记存独立小文件** `<config_dir>/conversations-imported.json`(计划 D9;原子写、`schema_version`、读不出来按未导入、更高版本不回写),它只是提示位,真相永远是「列表是否为空」。不要往 `digger.db` 加表。
- **面板 id `digger.conversations` 是落盘线上名**,一经发布不得改。已有用户的 `shell.json` 不被重置;旧文件缺该面板时放到 `digger.agent` 之后,**用户当前选中的面板不变**(计划 Task 1 的迁移测试逐条照做)。
- **失败不静默:** 守护进程调用失败显示在面板顶部一行可关闭的红字,同时 `eprintln!`;沿用 Agent 面板现有的通知条,不另造一套(抽成共用 `NoticeBar` 是 S8b 的事)。
- 守护进程不可用时显示原因而不是「没有会话」;恢复后「重试」重新取数;不可用期间导入按钮不可点并说明原因。共用 `AgentShell` 的守护进程状态,不各自再 `ensure`。
- 悬停动画 Digger 一期不做,`hover_progress` 恒 0。

## 约束

- **只动 Digger。** 不改 Dozer、不改 `dozer-panel-conversations`、不改 `dozerd`、不改协议。库有缺口:停下来报告,不绕开。
- 不发布、不改版本号、不切 tag、不推送、不安装。合并到 `main` 前向用户确认。
- 只用 git tag 依赖,不提交本地 `[patch]` 造成的 `Cargo.lock` 改动。
- commit 前看 staged 全貌(`git diff --cached --stat`);提交信息末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。
- 不碰用户的沙箱 `/private/tmp/digger-sandbox`、`/Applications` 里正在运行的应用及其 `diggerd`;你的验证用独立数据目录(`DIGGER_DATA_DIR`)与临时 `HOME`。
- 验收测试在 `git archive HEAD` 导出的干净快照里跑,**目录路径不含 "digger"**,用它自己的 `target`;`CARGO_TARGET_DIR` 不与别的工作区共享。
- **不在本任务内:** 审阅详情 webview 与 `native_host` 抽取(S7c)、关联待办与生成总结(另议)、群聊(S8)、产品发布。

## 交付

- Task 1–5 各自的提交(Task 1 去掉升依赖那一步)。
- 完工前在干净快照里:
  - `cargo build --workspace && cargo test --workspace --no-fail-fast && cargo clippy --workspace --all-targets && cargo fmt --check`
  - `bash scripts/check-shared-deps.sh && bash scripts/check-host-deps.sh && bash scripts/check-shell-regions.sh`(`check-host-deps.sh` 含 `--self-test`)、Cargo.lock 重复检查
  - 快照没有 `.git` 时 `check-shared-deps.sh` 会退出 128,该项在 worktree 里跑并注明。
- 报告写到 `docs/superpowers/reviews/2026-10-<日期>-conversations-s7b-report.md`:基线与终态测试总数及核对算式、每个 Review Focus 对应的测试名、与计划不一致处及原因、库接口缺口、每条「通过」附命令与退出码。
- 已知缺口写到 `docs/superpowers/notes/2026-10-26-conversations-s7b.md`:Digger 里删除项目不会清理该项目的会话(`delete_project_transcripts` 未接,留给用户裁决);搜索框右键菜单缺位;搜索焦点实现方式;列表宽度取值;悬停为阶跃。
- 报告里列一份**给用户的 Digger 目测验收清单**(计划 Task 5 Step 2 的 ① 至 ⑧ 加上中文输入法一项),点击由用户做,你启动沙箱并截图;任一项不符先停下报告。
- 停在 `feat/conversations-list`。

## 接下来(不属于本任务,仅供你了解依赖)

用户验收并合并 S7b 之后是 S7c:先抽 `native_host`(Digger 已有 `usage_native`、`todo_native` 两份,审阅宿主是第三份),再接审阅详情 webview(协议名 `digger-review`),最后发布。S8a(Dozer 侧群聊抽库)与本任务无依赖,可并行,但它收尾时的 libs tag 与 Digger 重钉必须排在 S7b 合并之后。
