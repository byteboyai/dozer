# 交接任务 7:S8a 群聊面板抽成共享库(Dozer)

> 给开发 agent 的任务说明,写于 2026-10-25。**本任务只动 Dozer 仓库。** 范围是 S8a 的 Task 1–3 与 Task 5 的 Step 1–2(原 Task 4 已删除,见下),然后**停下等用户在 Dozer 上目测验收**。合并、切 tag、推送、Digger 重钉(原 Task 6)都不在本任务内。

**目标:** 把 Dozer 的群聊面板(状态机与 `rev` 增量轮询、视图模型与推送协议、页面事件校验、群列表 / 新建群弹窗卡片 / 失败壳视图、`group-chat-content` 页面资源与协议应答、webview 事件与推送的宿主逻辑)抽成共享库 `dozer-panel-group-chat`;Dozer 改为薄调用,**用户可见行为不变,页面与协议字节不变**。

## 前置条件(开工第一步先核对,不满足就停下报告)

**本任务的 worktree 必须从已含 S7a 的 Dozer `main` 切出。** S7a(交接任务 6,`dozer-panel-conversations`)与本任务都会改 `app.rs`、`workspace/state.rs`、`runtime.rs`、`assets.rs`、`scripts/audit/check_panel_boundary.py`、`scripts/i18n/*.py`,并行做会冲突。核对办法:`git log --oneline main | head`、`ls crates | grep panel-conversations`。**若 `main` 里还没有 `dozer-panel-conversations`,不要开工**,告诉用户「S7a 尚未合并」。

## 2026-10-26 更新:前置已满足,以下以实际状态为准

- **S7a 已合并并发布 libs tag。** Dozer `main` 含 `dozer-panel-conversations`(合并提交 `609a18df`,tag `libs-2026-10-26a`,已推送到 origin),此后只多了文档提交。直接从当前本地 `main` 切 worktree 即可;上面的核对命令应当通过。
- **测试基线:** S7a 合并后在干净快照里是 **3548 通过 / 0 失败 / 5 忽略**。开工仍要自己重跑记数,写进第一个提交说明。
- **读 S7a 报告里的三条教训**(`docs/superpowers/reviews/2026-10-26-s7a-report.md`):
  1. **先红再绿要做到每个任务。** S7a 的 Task 3 没有先写失败测试,事后只补了变异测试。本任务不接受这个偏差;抽取与迁移期间新增的行为测试也要先看到它失败。
  2. **干净快照要带 git 历史。** `git archive` 导出的目录没有 `.git`,`extensions::git_log::tests` 里 3 个测试读仓库自身历史,会确定性失败(不是偶发)。验收时给快照补上历史(`git init` + `git fetch <worktree> HEAD` + `git reset --soft FETCH_HEAD`,工作树文件不变),或在报告里写明这 3 个是环境原因并单独证明通过。
  3. **沙箱可能不让写兄弟目录和原仓库 `.git`。** S7a 的开发 agent 因此改用 `/private/tmp` 下的独立克隆。若你也遇到,先试 `git worktree add ../dozer-panel-group-chat`,不行就同样用独立克隆并在报告开头说明;合并时用户会从该克隆 `git fetch` 分支。
- **库依赖方向:** S7a 里 `dozer-panel-preview` 反向依赖了 `dozer-panel-conversations`(为重导出 `parse_review_trace_event`)。群聊库**不得**照这个办法:`WebviewEnvelope` / `ProtocolError` 若要共用,在群聊库里放最小等价类型,不新增 preview → group-chat 依赖;发现 preview 里有群聊协议解析,同样先报告。
- **Digger 侧的 `Services` 已多了 `summary` 字段**,与本任务无关。若本任务改动了 `dozer-services` 的公开接口,**在报告开头单独标出**:Digger 重钉时会因此编译失败,需要同步补 `UnavailableService` 的占位实现。

## 先读(只读,按顺序)

1. `docs/superpowers/plans/2026-10-25-s7-s8-refresh.md`:**修订总览,优先于原稿**。重点 §1.1、§1.2、§1.3、§3「通用」11 条、§3「S8 设计稿与 S8a」一节、§5 Q1、§6。
2. 设计稿(Digger 仓库,只读):`/Users/chrischiang/Projects/CoralProjects/byteboy/digger/docs/superpowers/specs/2026-10-18-group-chat-panel-s8-design.md`(§2 现状、§3 G1–G2、§6.1 库边界、§7 验证)。**§6.2 与 §6.4 的 S8a 部分里关于 `capabilities.push_to_todo` 的内容已作废**,以本文为准。
3. **计划(照这份做):** `/Users/chrischiang/Projects/CoralProjects/byteboy/digger/docs/superpowers/plans/2026-10-18-group-chat-s8a-lib.md`。先读 Global Constraints 与 Review Focus,每个任务先写失败测试。
4. 模板:`dozer-panel-todo`、`dozer-panel-agent`,以及 S7a 刚完成的 `dozer-panel-conversations`(库的形状、`PanelDisplay`、`NoticeOutbox`、黄金测试的写法)。S7a 的报告 `docs/superpowers/reviews/*-s7a-report.md` 里的踩坑记录要读。

## 与原稿不同的地方(原稿 S8a 已被总览修订,这里只列对你最重要的)

- **原 Task 4(载荷 `capabilities` 字段与页面加法)整个删除。** 不改 `protocol.rs` 的序列化形状、不改前端 TS、**不重新构建 `assets/group-chat-content/*`**(只做 `git mv`,字节不变)、载荷黄金不需要「只允许新增字段变化」的例外。理由:S3b-1 合并后 Digger 直接开启「转待办」(总览 §5 Q1),页面无需按能力隐藏入口。
- **`GroupChatCapabilities` 删除(YAGNI)。** 原 Task 3 的 `host::handle_webview_event`、`host::take_content_script` 里的 `caps` 参数一并删除;`route_event` 不加 `caps`;`current_view_payload` 不加参数。`HostAction::{None, ShowTodoPanel}` 保留,Dozer 把 `ShowTodoPanel` 映射成 `self.show_panel(PanelKind::Todo)`(与现状行为一致)。
- **原 Review Focus 第 5 条(能力开关)删除**,随它的测试也不写。其余四条保留。
- **Task 1 Step 1(环境变量核对)改为只读确认。** 已核实:`dozerd/src/group_adapter.rs` 用 `dozer_core::paths::session_env()`(产品感知),不存在硬编码的 `DOZER_SESSION_ID`。你只需 `grep` 再确认一次,在提交说明里写一句结论;**不会有「发现硬编码就最小改 `dozerd`」的分支,不要改 `dozerd`**。
- **只读不变式已有测试,不新增、不删、不改。** `dozerd/src/group_adapter.rs` 的测试模块里已钉住:Claude 的 `--allowedTools` / `--disallowedTools` / `--no-session-persistence` 在、参数里没有含 `dangerously` 的项也没有 `-y`;Codex 是 `exec --ephemeral --sandbox read-only` 且不含 `dangerously`;`unsupported_agents_have_no_command` 钉住其余种类无命令。Task 1 里确认它们仍在并通过(记下测试名),抽库全程**不得删改**。这是群聊安全底线的回归保护。
- **成员 agent 是执行层只读的**(原 S8 设计稿 Q1 的「在项目目录里写文件」说法是错的,已订正)。本任务不碰这条线;若你在搬代码时看到任何会放宽它的改动(`--dangerously-skip-permissions`、`-y`、去掉 `--sandbox read-only`),停下报告。
- **行号全部过期**:`app.rs 约 4061–4090 行`、`runtime.rs 约 820–823 行` 等一律按**符号名**重新定位。`mod.rs` 45 条、`protocol.rs` 24 条、`assets.rs` 里 `group_chat_content_*` 5 条是 2026-10-18 的数,**开工先重新记基线**(S7a 合并后总数也变了)。
- **门禁多两条**:`bash scripts/check-shared-deps.sh`、`bash scripts/check-log-scope.sh`(后者只允许 `window_events.rs` 那一处基线告警)。
- **库不得依赖** `dozer-app`、`dozer-client`、`dozer-core`、`dozerd`、`wry`、`winit`;库内日志用 `tracing` 薄封装(同 `dozer-panel-project`),不用 `dozer_core::log`。

## 基线

- Dozer 仓库 `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer`,基线是**已含 S7a 的 `main`**(开工先 `git log --oneline -5` 与 `git status` 看清楚)。
- 新建 worktree `../dozer-panel-group-chat`,分支 `feat/panel-group-chat-lib`,从 `main` 切出。
- **不要动主工作区。** 它有用户未提交的改动:`Cargo.lock`(来自本地 `[patch]` 指向 `../byteui`、`../bytegit`)。**这些永远不提交。**
- 全 workspace 测试基线:**开工先自己重新跑一遍记数**,写进第一个提交说明(S7a 之后数字与 2026-10-25 的 3497 不同)。
- 已知偶发失败:`dozer-panel-preview` 的 `scratch_editor`、`webview_spec_golden` 曾因共享状态偶发失败,已修。**你搬走或新增的测试凡要改全局主题方案的,用 `theme::test_support::lock_scheme()`。** 遇到别的偶发失败:单独重跑量化(次数、失败数写进报告),不要靠重跑到绿。

## 容易踩的坑

- **黄金测试必须在改任何代码之前生成并提交**(载荷 JSON、事件解析表、`group-chat-content` 应答 `(mime, 字节数, sha256)`)。之后字节不得变,且因为 Task 4 删除,**整个任务里三份黄金都是原样守恒**,没有任何例外。这是「用户可见行为不变」唯一硬依据。
- **`web/group-chat-content/` 与 `assets/group-chat-content/` 要 `git mv`**,构建脚本、`build.rs`、打包脚本、`scripts/` 里引用旧路径的地方一并改——最容易漏。`assets/group-chat-content/*` 是运行时产物,**不要重新 `npm run build` 后提交变化的产物**:`npm run build` 之后 `git diff --exit-code` 必须为空(产物与提交一致);若不为空,说明构建环境差异,停下报告而不是提交新产物。完工前用打包自检确认页面资源编进了二进制、不依赖磁盘路径。
- **前端 `types.ts` 与后端 serde 同名同形**:搬迁不改任何一边。
- **agent 输出必须经转义渲染器,不得 `innerHTML` 原文**:前端不动,但搬 `web/` 时别丢它的测试(`npm test`、`render-smoke`)。
- **`WebviewEnvelope` / `ProtocolError` 归属**:同 S7a——若仍在 `dozer-app`,在库里放最小等价类型并让 `dozer-app` 复用库的,**不得让库依赖 `dozer-app`**。
- **新建群弹窗是 Dozer 的独立 winit 窗口(`platform/group_create_overlay.rs`)**:库只给卡片元素(`dialog::new_group_dialog_card`);窗口生命周期、IME、`sync_action` 全留 `dozer-app`。
- **`Message` / `Effect` 变体名与载荷原样搬,不增不减**。
- **Task 2 里 `creatable_agents_are_exactly_claude_codex_codebuddy_opencode_qodercn` 的交叉断言**:五种可创建、只有 Claude 与 Codex 能发言是 Dozer 现状(总览 §1.2),原样保留,**不要在库里「顺手」隐藏不能发言的种类**(那是 Digger S8b 的 §5 Q6 决定:不隐藏,只加一句说明)。
- 凡一次性提示一律经 `NoticeOutbox`,由 Dozer 排空成 Toast;**不要**在库里自己画提示条。
- 日志与敏感内容:别把群消息正文写进日志。

## 约束

- **Dozer 用户可见行为不变。** 搬走的测试随代码走、期望值不改;总数核对按「基线 + 新增」算(本任务**没有**允许删除的测试)。
- 不改 `dozerd`、不改协议字节、不改页面 HTML/CSS/JS、不放宽严格 CSP。
- 只用 git tag 依赖;Dozer 内部 `path = "../.."` 除外。
- commit 前看 staged 全貌(`git diff --cached --stat`);提交信息末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。
- 不碰用户的沙箱 `/private/tmp/digger-sandbox`、`/Applications` 里正在运行的应用及其 `dozerd` / `diggerd`;验证用独立数据目录与临时 `HOME`。
- **不合并 `main`、不切 tag、不推送、不发布、不安装。** 原 Task 5 Step 3 起(用户目测验收、合并、tag)与原 Task 6(Digger 重钉)不在本任务内。
- **不在本任务内:** Digger 接入(S8b / S8c)、`native_host` 抽取(已挪到 S7c Task 0)、产品发布。

## 交付

- Task 1、2、3 各自的提交,以及 Task 5 Step 1(清理)、Step 2(全量)的结果。
- 完工前 `git archive HEAD` 解到独立临时目录(目录名不带 "dozer"),在其自己的 `target` 下跑:
  - `cargo build --workspace && cargo test --workspace --no-fail-fast && cargo clippy --workspace --all-targets && cargo fmt --check`
  - `bash scripts/check-i18n.sh && python3 scripts/audit/check_panel_boundary.py && bash scripts/check-shared-deps.sh && bash scripts/check-log-scope.sh`
  - `cd crates/dozer-panel-group-chat/web/group-chat-content && npm test && npm run build`,再 `git diff --exit-code` 确认产物与提交一致
  - `cargo tree -p dozer-panel-group-chat | grep -E "dozer-(app|client|core)|dozerd|wry|winit"` 无输出
  - `cargo test -p dozerd group_adapter` 通过,且只读不变式那几条测试名在输出里
- 报告写到 `docs/superpowers/reviews/2026-10-<日期>-s8a-report.md`:基线与终态测试总数及核对算式、三份黄金的哈希(以及它们在整个任务期间未变)、每个 Review Focus(1–4)对应的测试名、只读不变式测试名清单与通过证据、环境变量核对结论、与计划不一致处及原因、每条「通过」附命令与退出码。
- 报告里列一份**给用户的 Dozer 目测验收清单**(原稿 Task 5 Step 3 的条目):群列表(空态、多群、选中高亮)、新建群弹窗(独立窗口、多行主题、勾选成员、取消 / 创建、Esc)、聊天页(成员条、消息流、状态点、流式回复)、发言与 @提及(单个 / 多个 / 不存在的句柄)、取消(单条 / 整群)、重试、增删改成员、删群、**转待办(仍可用,打开待办面板)**、webview 加载失败壳与「重试」(可临时拆掉页面文件复现)、输入框草稿在切面板后的恢复、折叠列表、窗口缩放、中英文切换。用户验收通过前不合并。
- 停在 `feat/panel-group-chat-lib`。

## 接下来(不属于本任务,仅供你了解依赖)

用户验收并合并 S8a、切 libs tag 之后:Digger 同日重钉(原 Task 6,**必须排在 S3b-1 合并之后**,新 tag 里 byteui / bytehost / bytegit 的钉版要与 Digger 当时的逐个一致);随后 S7b、S7c(含新增的 `native_host` 抽取)、S8b、S8c。
