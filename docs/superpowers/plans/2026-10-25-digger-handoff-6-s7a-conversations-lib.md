# 交接任务 6:S7a 会话面板抽成共享库(Dozer)

> 给开发 agent 的任务说明,写于 2026-10-25。**本任务只动 Dozer 仓库。** 范围是 S7a 的 Task 1–3 与 Task 4 的 Step 1–2,然后**停下等用户在 Dozer 上目测验收**。合并、切 tag、推送、Digger 重钉(Task 5)都不在本任务内。

**目标:** 把 Dozer 的会话面板(列表、详情审阅数据与逻辑、总结生成、`review-trace` 页面资源与协议应答)抽成共享库 `dozer-panel-conversations`;Dozer 改为薄调用,**用户可见行为不变**。

**先读(只读,按顺序):**

1. `docs/superpowers/plans/2026-10-25-s7-s8-refresh.md`:**修订总览,优先于原稿**。重点 §1.3、§3「通用」11 条、§3 的 S7 设计稿与 S7a 一节、§6。
2. 设计稿(Digger 仓库,只读):`/Users/chrischiang/Projects/CoralProjects/byteboy/digger/docs/superpowers/specs/2026-10-18-conversations-panel-s7-design.md`(§2 现状、§6.1 接缝、§6.2 库边界、§7 验证)。
3. **计划(照这份做):** `/Users/chrischiang/Projects/CoralProjects/byteboy/digger/docs/superpowers/plans/2026-10-18-conversations-s7a-lib.md`。先读 Global Constraints 与 Review Focus,每个任务先写失败测试。
4. 模板:`docs/superpowers/plans/2026-10-15-panel-todo-lib.md` 与已完成的 `dozer-panel-todo`(库的形状、`PanelDisplay`、发件箱);S6c-2 的 `dozer-panel-agent`。

## 基线

- Dozer 仓库 `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer`,基线是 `main`(开工先 `git log --oneline -3` 与 `git status` 看清楚)。
- 新建 worktree `../dozer-panel-conversations`,分支 `feat/panel-conversations-lib`,从 `main` 切出。
- **不要动主工作区。** 它有用户未提交的改动:`Cargo.lock`(来自本地 `[patch]` 指向 `../byteui`、`../bytegit`)。**这些永远不提交。**
- 全 workspace 测试基线:2026-10-25 是 3497 个通过、0 失败(之后加了 6 个 `dozer-panel-agent` 测试)。**开工先自己重新跑一遍记基线**,数字写进第一个提交说明。
- 已知偶发失败:`dozer-panel-preview` 的 `scratch_editor` 与 `webview_spec_golden` 曾因共享状态偶发失败,已修(进程号加原子计数、`theme::test_support::lock_scheme()`)。**你搬走或新增的测试凡要改全局主题方案的,用 `lock_scheme()`。** 遇到别的偶发失败:单独重跑量化(次数、失败数写进报告),不要靠重跑到绿。

## 与原稿不同的地方(原稿 S7a 已被总览修订,这里只列对你最重要的)

- **行号全部过期**:`assets.rs 约 275–301 行`、`runtime.rs 约 320、529…` 等一律按**符号名**重新定位。
- **`native_menu`**:Dozer 的 `chrome/native_menu.rs` 现在只是 `byteui::native_menu` 的重新导出(92 处调用不变)。Agent 筛选仍按计划走 `ConversationsRequest::ShowAgentFilter`,适配层把它翻成原生菜单。
- **门禁多两条**:`bash scripts/check-shared-deps.sh`、`bash scripts/check-log-scope.sh`(后者只允许 `window_events.rs` 那一处基线告警)。
- **`ReviewSource::Session(_)`** 现在只剩读它的分支(`workspace/view.rs`、`app/update.rs` 里 `=> None`),没看到创建点。计划要你**动手删之前再 grep 一遍确认**,保留这一步。
- **库不得依赖** `dozer-app`、`dozer-client`、`dozer-core`、`dozerd`、`wry`、`winit`;库内日志用 `tracing` 薄封装(同 `dozer-panel-project`),不用 `dozer_core::log`。

## 容易踩的坑

- **黄金测试必须在改任何代码之前生成并提交**(审阅快照 JSON、`review-trace` 应答 `(mime, 字节数, sha256)`、三个请求/应答的协议编码)。之后字节不得变。这是「用户可见行为不变」唯一硬依据。
- **「加载更多」`append == true` 必须累加不能替换**(2026-08-27 的真实 bug,已有测试,随代码走)。
- **`web/review-trace/` 与 `assets/review-trace/` 要 `git mv`**,构建脚本、`build.rs`、打包脚本、`scripts/` 里引用旧路径的地方一并改——这是最容易漏的一处。完工前用打包自检确认页面资源编进了二进制、不依赖磁盘路径。
- **`WebviewEnvelope` / `ProtocolError` 归属**:原稿没核实。若仍在 `dozer-app`,在库里放一份最小等价类型并让 `dozer-app` 复用库的,**不得让库依赖 `dozer-app`**。
- **搜索框焦点探测(`CaptureConversationSearchFocus`)留在 `dozer-app`**,不进库。若发现它还牵着库内别的状态,先停下报告。
- 凡一次性提示一律经 `NoticeOutbox`,由 Dozer 排空成 Toast;**不要**在库里自己画提示条。

## 约束

- **Dozer 用户可见行为不变。** 搬走的测试随代码走、期望值不改;总数核对按「基线 − 已删(只允许 `Session` 死分支专属测试,逐条列出)+ 新增」算。
- 不改 `dozerd`、不改协议字节、不改 `review-trace` 页面本身、不放宽严格 CSP。
- 只用 git tag 依赖;Dozer 内部 `path = "../.."` 除外。
- commit 前看 staged 全貌(`git diff --cached --stat`);提交信息末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。
- 不碰用户的沙箱 `/private/tmp/digger-sandbox`、`/Applications` 里正在运行的应用及其 `dozerd` / `diggerd`;你的验证用独立数据目录与临时 `HOME`。
- **不合并 `main`、不切 tag、不推送、不发布、不安装。** Task 4 Step 3 起(用户目测验收、合并、tag)与 Task 5(Digger 重钉)都不在本任务内。
- **不在本任务内:** Digger 接入(S7b / S7c)、群聊(S8)、产品发布。

## 交付

- Task 1、2、3 各自的提交,以及 Task 4 Step 1(清理)、Step 2(全量)的结果。
- 完工前 `git archive HEAD` 解到独立临时目录(目录名不带 "dozer"),在其自己的 `target` 下跑:
  - `cargo build --workspace && cargo test --workspace --no-fail-fast && cargo clippy --workspace --all-targets && cargo fmt --check`
  - `bash scripts/check-i18n.sh && python3 scripts/audit/check_panel_boundary.py && bash scripts/check-shared-deps.sh && bash scripts/check-log-scope.sh`
  - `cd crates/dozer-panel-conversations/web/review-trace && npm test && npm run build`,再 `git diff --exit-code` 确认产物与提交一致
  - `cargo tree -p dozer-panel-conversations | grep -E "dozer-(app|client|core)|dozerd|wry|winit"` 无输出
- 报告写到 `docs/superpowers/reviews/2026-10-<日期>-s7a-report.md`:基线与终态测试总数及核对算式、三份黄金的哈希、每个 Review Focus 对应的测试名、被删的 `Session` 死代码与测试清单、与计划不一致处及原因、每条「通过」附命令与退出码。
- 报告里列一份**给用户的 Dozer 目测验收清单**(原稿 Task 4 Step 3 的条目),用户验收通过前不合并。
- 停在 `feat/panel-conversations-lib`。

## 接下来(不属于本任务,仅供你了解依赖)

用户验收并合并 S7a、切 libs tag 之后:Digger 同日重钉(原 Task 5,**必须排在 S3b-1 合并之后**,新 tag 里 byteui / bytehost / bytegit 的钉版要与 Digger 当时的逐个一致);随后 S8a(同样只动 Dozer,worktree 须从已含 S7a 的 `main` 切出)、S7b、S7c(含新增的 `native_host` 抽取)、S8b、S8c。
