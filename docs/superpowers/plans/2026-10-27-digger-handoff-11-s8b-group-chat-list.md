# 交接任务 11:S8b Digger 群聊面板(群列表、新建群、每项目状态、轮询)

> 给开发 agent 的任务说明,写于 2026-10-27。**本任务只动 Digger 仓库。** 范围是 S8b 的 Task 1–5(含抽共用 `NoticeBar`),跑完自动门禁后**停在分支上等用户验收**。合并、发版、tag、推送、安装都不在本任务内。

**目标:** Digger 右栏出现群聊面板(`digger.agent` 之后、`digger.conversations` 之前),配对布局(内容 | 群列表):群列表可建群、选群;新建群弹窗可勾选成员 Agent;内容块先放原生「聊天内容即将可用」占位并显示当前群的主题与成员句柄(真正的聊天页是 S8c)。数据来自 `diggerd` / `digger.db`,每项目一份状态,轮询只在需要且面板可见时运行。**库、`dozerd`、协议都不改。**

## 先读(只读,按顺序)

1. `dozer/docs/superpowers/plans/2026-10-25-s7-s8-refresh.md`:修订总览,**优先于原稿**。重点 §1.1(群聊成员只读)、§3「通用」11 条与「S8b」一节、§5 Q1/Q3/Q5/Q6。
2. **计划(照这份做,按下文修订):** `digger/docs/superpowers/plans/2026-10-18-group-chat-s8b-digger-list.md`。先读 Global Constraints 与 Review Focus,每个任务先写失败测试。
3. 设计稿:`digger/docs/superpowers/specs/2026-10-18-group-chat-panel-s8-design.md`(§6.3、§7;§8 Q1 以总览 §1.1 为准)。
4. 样板:S7b 的 `crates/desktop/src/adapters/conversations*.rs`、`product.rs` 里会话面板的接线、`shell.rs` 的 `migrate_missing_conversations` 一类迁移函数(**以 S7b 的实际形状为准**);S7b 报告 `docs/superpowers/reviews/2026-10-26-conversations-s7b-report.md`。
5. 库(以代码为准,**不按原稿设想的名字**):`dozer/crates/dozer-panel-group-chat/src/{update.rs,host.rs,dialog.rs,view.rs,adapter.rs}`。已核实的真实形状:`update::update(state, msg) -> Vec<Effect>`,`Effect` 只有 `FetchMessages` / `CreateGroup` / `AddMembers`,由 `run_effect(requests, effect, project_id, io)` 执行;`Message::project_id()` 用于路由;`host::poll_wanted(state, panel_visible)`;`update::{poll_if_due, on_activate, select_group, spawn_load_groups}`;`dialog::new_group_dialog_card(state, host, project_id) -> Option<Element>`;`PanelDisplay<M>`(`wrap/locale/panel_head/list_collapse_button/pane_style/agent_icon/agent_icon_color/render_localized`);`WorkspaceState::{groups, selected, messages, loaded, load_pending, has_active_turn, new_group_dialog_open, take_outbox}`;一次性失败走 `Message::Failed(project_id, LocalizedText)`(Dozer 里进 Toast,Digger 没有 Toast,进通知条并 `eprintln!`,文案经 `render_localized` 解出)。**没有 `GroupChatCapabilities`,没有 `caps` 参数**(S8a 已删,总览 §3.S8a)。

## 基线

- Digger 仓库 `/Users/chrischiang/Projects/CoralProjects/byteboy/digger`。**开工前先确认 S7c 已合并进 `main`**:S7c 与 S8b 都改 `product.rs`、`adapters/mod.rs`、`adapters/conversations.rs`(S8b 的 `NoticeBar` 抽取会动它),并行会冲突。若 S7c 还没合并,**停下告诉用户**,不要自己基于旧 `main` 开工。基线是 S7c 合并后的 `main`(此前为 `4495075`)。
- libs 已是 **`libs-2026-10-26b`**(= Dozer `c76302fc`,已含 S8a 的 `dozer-panel-group-chat`)。**不升 tag**;原稿 Task 1 Step 1「升依赖」缩减为:workspace `Cargo.toml` 与 `crates/desktop/Cargo.toml` 加 `dozer-panel-group-chat`(同 tag),跑 Cargo.lock 重复检查与 `check-shared-deps.sh`。
- 全 workspace 测试基线:S7c 合并后重跑记数(2026-10-27 为 713 个、712 通过 + 1 个偶发失败,集中在 `adapters::agent_hooks::tests::*` 与 `apps::tests::closing_during_startup_waits_and_releases_the_root`,单独重跑稳定通过)。失败**单独重跑量化(次数、失败数写进报告)**,不靠重跑到绿。
- worktree:`../digger-s8b`,分支 `feat/group-chat-list`,从 `main` 切出。**S7b 的 agent 发现 sibling 目录不可写,改用了 `/private/tmp` 下的独立 clone**;若你也遇到,用 `/private/tmp/digger-s8b` 并在报告写明。
- **不要动 Digger 主工作区**:用户有未提交改动(13 份已删除的 S6 文档、未跟踪的 `.qoder/`)。不碰 `feat/preview-pairing` / `digger-p4`。沙箱可能禁 `/bin/ps`,使 `digger-app-host` 的 recovery 测试在你那边失败,如实记录、不改该测试。

## 与原稿不同的地方

- **库 API 以上面第 5 条为准。** 原稿 Task 2 里「`GroupChatPanel::update` 对每个 `Effect` 经 `run_effect` 执行、随后排空 `take_outbox()`」这个形状是对的,但名字(`NewGroupDialogCancel`、`poll_wanted`、`dialog::new_group_dialog_card`)都存在,`caps` 与 `GroupChatCapabilities` 不存在。
- **「转待办」:** Digger 已有待办面板(S3b),群聊页的「转待办」按钮在页面里、属于 S8c。S8b 的内容块是占位,**没有任何转待办入口**;原稿测试 `no_todo_text_anywhere_in_the_view` **删除**(总览 §5 Q1 已改为开启,这条过时)。库的 `Message::TodoPushed` 与 `HostAction::ShowTodoPanel` 在 S8b 不会被触发,更新函数里留穷尽分支即可,S8c 映射成「切到待办面板」。
- **服务束:** 复用 S3b/S7b 已有的 `UnavailableService`(`pub(crate)`,`adapters/project.rs`)与 `Daemon::conversation_services()` 的做法,新增 `Daemon` 上取 `GroupChatService` 的方法(同一个 `Client`);**不另写一份占位**。`Services` 的字段集合以 `dozer-services` 现状为准(含 `summary`),漏字段会编译失败。
- **`list_collapse_button`:** Dozer 的适配里它返回空占位,Digger 要做**真的**折叠按钮,发 `ShellMsg::TogglePane(group_chat_id)`,且**始终包 `Tooltip::delay`**(控件树结构不得随显示状态翻转,否则子 `MouseArea` 状态被重置,见总览 §3 通用第 3 条);悬停用 `adapters/hover.rs` 的 `HoverTracker`,本期悬停为阶跃(设计 D3)。
- **`NoticeBar` 抽取(Task 2):** 现在 `agent.rs`、`conversations.rs` 各有 `notice: Option<String>`,群聊是第三个。抽成 `adapters/notice.rs` 的 `NoticeBar`,三者共用;**既有测试原样通过、条数不减**为界。`ProjectPanel` 与待办面板暂不改。
- **新建群弹窗(Task 4)** 仍是 iced `stack!` 模态卡片(Digger 没有独立浮层窗口设施,总览 §5 Q3)。S8b 期间内容块是占位所以不冲突;`Overlay::GroupChatCreate` 加入覆盖层互斥枚举(先 `grep -n "enum Overlay"` 找当前位置与规则),`overlays_open()` 把它算作打开——S8c 的聊天 webview 据此停放到屏外。点背景不关闭(保留草稿,与 Dozer 一致),Esc 或「取消」关闭。弹窗里的成员列表就是 `CREATABLE_AGENTS` 五种;加一行常驻说明「被 @ 的成员只读地读取项目文件,目前仅 Claude、Codex 可以发言」(总览 §5 Q5/Q6,**不加首次发言确认弹窗**,不隐藏不能发言的种类)。
- **键盘:** 弹窗打开(以及主题框聚焦)时,按键不得进 Agent 终端——`AgentPanel::on_key` 丢弃按键,**并把「群聊弹窗打开」并入 `agent_keys_blocked()` 所读的阻止条件**(S6d 里订阅回调读进程级 `AtomicBool`,S7b 的搜索焦点已这样接;群聊要同样加测试)。**`thread_local` 不得存订阅回调要读的状态。**
- **中文输入法:** 主题框是 `byteui::form::text_area`(iced `text_editor`),与终端的 `ImeSink` 是两条路径,拼音输入、候选框位置、提交/删除只能人工验证,写进用户验收清单。若 Esc 被 `text_editor` 吞掉,先停下报告(退路见原稿风险 ⑤)。

## 约束

- **建群、加成员、选群、删群本身绝不拉起任何 Agent 命令行**;S8b 没有发言入口,所以 Digger 不应出现任何由群聊产生的 Agent 进程。测试里假 `GroupChatService` 的记录不得出现 `post_group_message` / `retry_group_message`。**严禁**为群聊加 `--dangerously-skip-permissions` / `-y`(S8b 不碰命令行,但这是底线)。
- 面板 id `digger.group_chat` 是落盘线上名,一经发布不得改;旧 `shell.json` 缺该面板时放在 `digger.agent` 之后,**用户当前选中的面板不变**;更高 schema 版本的文件不回写。
- 轮询纪律:`iced::time::every(500ms)` **只在 `poll_wanted && 面板可见 && !closing` 时订阅**,`poll_wanted` 在 `subscription()` 里按值计算;只轮询聚焦项目;同一项目同一时刻至多一个在途请求;`diggerd` 出错按库的 5 秒退避;项目页签关闭后其状态、草稿、在途结果全部丢弃。
- 项目隔离:A 的 `Polled` / `GroupsLoaded` / `GroupCreated` 不改 B;状态不存在或项目已关闭的迟到结果忽略。
- 失败不静默:守护进程调用失败显示在面板顶部可关闭的通知条并 `eprintln!`;Digger 没有 Toast。
- 只用 git tag 依赖;不改 Dozer / `dozerd` / 协议 / 库;库的缺口只报告,不绕。
- 验收测试在 `git archive` 的**干净快照**里跑(路径不含 "digger";快照要有 git 历史:`git init` + `git fetch <worktree> HEAD` + `git reset --soft FETCH_HEAD`,否则 `extensions::git_log` 的 3 个测试确定性失败)。`CARGO_TARGET_DIR` 不与别的工作区共享。
- commit 前 `git diff --cached --stat`;只 `git add` 具体路径,不 `git add -A`/`.`;提交信息末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。
- 不碰用户的沙箱 `/private/tmp/digger-sandbox`、`/Applications` 里正在运行的应用及其 `diggerd`;验证用独立 `DIGGER_DATA_DIR` 与临时 `HOME`。

## 交付

- Task 1–5 各自的提交,每个任务先 red 后 green。
- 干净快照里全绿(偶发失败量化后注明):`cargo build --workspace && cargo test --workspace --no-fail-fast && cargo clippy --workspace --all-targets && cargo fmt --check`;`bash scripts/check-host-deps.sh`(含 `--self-test`)、`bash scripts/check-shared-deps.sh`(在 worktree 里跑,archive 无 `.git` 会 exit 128)、`bash scripts/check-shell-regions.sh`、Cargo.lock 重复检查。
- 报告 `docs/superpowers/reviews/2026-10-<日期>-group-chat-s8b-report.md`:基线与终态测试总数及核对算式(`基线 − 已删 + 新增`,`NoticeBar` 抽取不得减少既有测试)、每个 Review Focus 对应的测试名、与计划不一致处及原因、每条「通过」附命令与退出码、**没验的东西明说**。已知缺口写 `docs/superpowers/notes/2026-10-<日期>-group-chat-s8b.md`:内容块是临时占位;删除群的入口在 S8c 聊天页内;转待办入口在 S8c;悬停为阶跃;列表最小宽度同 S6d/S7 的待调项。
- **你的环境多半启不了 GUI**(S7b 就是这样)。那就如实写「GUI 未验」,准备好 `group_chat_gallery` 夹具(假服务预置 2 个群,含 Running 消息)、独立沙箱的 `launch.sh`,并把原稿 Task 5 Step 2 的十项真机回归(含:中文主题输入法、建群后进程列表里没有 `claude`/`codex` 等因群聊出现的进程、旧 `shell.json` 迁移、重启后群仍在、与 Dozer 同机并存、隐藏时 CPU 空闲)写成**给用户的验收清单**。不伪造截图,不替用户点击。
- 停在 `feat/group-chat-list`。

## 不在本任务内

合并、发版、tag、推送、安装、升版本号。聊天内容 webview、发言/取消/重试/成员管理/删群、转待办入口、与 Dozer 对照都在 S8c。S8c 直接复用 S7c 抽出的 `native_host.rs`。
