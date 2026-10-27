# 交接任务 10:S7c Digger 审阅详情 webview(含抽 `native_host`)

> 给开发 agent 的任务说明,写于 2026-10-27。**本任务只动 Digger 仓库。** 范围是 Task 0–6(抽 `native_host`、审阅 webview、覆盖层与可见性、Dozer 对照、压力与容错、文档),跑完自动门禁后**停在分支上等用户验收**。**Task 7(发版、合并)不在本任务内**,见文末。

**目标:** 把 S7b 里「审阅详情即将可用」的占位换成真实的会话详情:点列表一行,内容块用库里**同一份 `review-trace` 页面**显示时间线,支持「加载更多」;与 Dozer 逐状态对照;压力与容错回归。**库、页面、`dozerd`、协议都不改。**

## 先读(只读,按顺序)

1. `dozer/docs/superpowers/plans/2026-10-25-s7-s8-refresh.md`:修订总览,**优先于原稿**。重点 §3「通用」11 条与「S7c」一节、§4、§6。
2. **计划(照这份做,但按下文修订):** `digger/docs/superpowers/plans/2026-10-18-conversations-s7c-review-and-release.md`。先读 Global Constraints 与 Review Focus,每个任务先写失败测试。
3. 设计稿:`digger/docs/superpowers/specs/2026-10-18-conversations-panel-s7-design.md`(§6.3、§7、§8)。
4. S7b 的交付:`digger/docs/superpowers/reviews/2026-10-26-conversations-s7b-report.md`、`docs/superpowers/notes/2026-10-26-conversations-s7b.md`。
5. 样板:`crates/desktop/src/adapters/usage_native.rs`(266 行)、`todo_native.rs`(467 行)。
6. 库的审阅 API(以代码为准,不按原稿设想):`dozer/crates/dozer-panel-conversations/src/{snapshot.rs,assets.rs,view.rs}`——`review_webview_spec(review, locale, scheme) -> Option<ReviewWebviewSpec{url,visible,park_offscreen}>`、`review_pane_state`、`serve_review_trace(path, review_json) -> Option<(mime, bytes)>`、`parse_review_trace_event(raw)`、`ReviewTraceEvent::DocumentLoaded`、`view::review_pane(state, app, caps, width, outer)`。库里「页面加载完成」的消息名以 `Message` 枚举为准(原稿叫 `ReviewTraceLoaded`,实际名先 grep)。

## 基线

- Digger 仓库 `/Users/chrischiang/Projects/CoralProjects/byteboy/digger`,基线 `main` **`4495075`**(含 S7b `e455c46`,libs 钉在 **`libs-2026-10-26b`** = Dozer `c76302fc`)。
- 全 workspace 测试基线:**713 个,712 通过,1 个偶发失败**(2026-10-27 干净快照实测)。偶发失败集中在 `adapters::agent_hooks::tests::*` 与 `apps::tests::closing_during_startup_waits_and_releases_the_root`,全量并发时超时、单独重跑稳定通过。**开工先自己重跑记基线**;遇到失败**单独重跑量化(次数、失败数写进报告)**,不靠重跑到绿。
- worktree:`../digger-s7c`,分支 `feat/conversations-review`,从 `main` 切出。**S7b 的 agent 发现 sibling 目录不可写,改用了 `/private/tmp/digger-s7b` 的独立 clone**;若你也遇到,同样用 `/private/tmp/digger-s7c` 并在报告里写明。
- **不要动 Digger 主工作区。** 它有用户未提交的改动:13 份已删除的 S6 文档(`docs/superpowers/` 下)和未跟踪的 `.qoder/`。不碰 `feat/preview-pairing` / `digger-p4`。
- 沙箱会禁用 `/bin/ps`,导致 `digger-app-host` 的 `a_restart_after_a_kill_reaps_the_recorded_orphan…` 在你那边稳定失败(S7b 报告已记,用户机器上不失败)。**如实记录,不要弱化或修改该测试。**

## 与原稿不同的地方

- **新增 Task 0:抽 `adapters/native_host.rs`。** 此时 Digger 已有 `usage_native`、`todo_native` 两份 wry 宿主样板,审阅宿主是第三份,按「第二次出现才抽」现在抽。内容:单槽 `wry::WebView` + 代数(epoch)防过期 + 纯函数 `plan(current, desired) -> Vec<Action>` + 导航白名单(只放行 `{scheme}://{host}/…`,含 `digger-review://review-trace.evil/`、`…@evil/` 这类变体)+ 新窗口/下载拒绝 + 缩放 + `window::run` 套路。**让 `usage_native.rs` 与 `todo_native.rs` 改用它;两者既有测试原样通过、条数不减**(新增只许加不许删)。各面板特有的部分(协议应答、IPC 解析、`todo_native` 的 `dispatch`)留在各自文件。先写抽取后的失败测试,抽取前后行为须一致。
- **协议名 `digger-review`**(不是原稿的 `digger`):库的 URL 构造本来就带 `scheme` 参数,传 `"digger-review"`;导航白名单只放行 `digger-review://review-trace/`。嵌入资源与 `build.rs` 若涉及同名 `host.html`,加面板前缀避免互相覆盖。
- **Agent 筛选是原生菜单,不隐藏 webview。** 原稿 Task 3 的「打开 Agent 筛选菜单隐藏审阅 webview」**反过来**:打开原生菜单时 webview **保持可见**(原生菜单浮在 wry 视图之上)。测试改为 `opening_the_native_agent_filter_menu_keeps_the_review_webview_visible`。`overlays_open()` 只含 **iced 内**的 Agent 选择器、溢出菜单(非 macOS 仍是 iced 版时)、确认框、项目删除确认卡;S7b 已保证原生菜单不计入。
- **审阅头部没有「生成总结」。** 能力全关(`ConversationsCapabilities::default()`),`LinkedTodos` 保持空实现,本期不开。
- **`thread_local` 只许存 wry 视图句柄,且只在 `window::run` 闭包里(主线程)访问**;订阅回调要读的状态一律用进程级原子或消息。这是 S6d 键盘整个失效的教训。
- **webview 几何必须同源:** 内容块上方有面板头(iced),webview 矩形用库/shell 导出的度量算,**不手写像素常数**,并加「不重叠」几何测试。Dozer 对照数字(`webview_geometry::content_pane_bounds_for(PanelKind::Conversations, ..)`)在 Dozer 里算出后写死进测试,附算式。
- **键盘焦点:** webview 获得点击焦点后会吃键盘。销毁或隐藏 webview 时让窗口内容视图重新成为第一响应者(参考 `byteui::native_menu::install_content_view` 的 `ns_view` 做法,非 macOS 空操作),并加测试或手工检查项:关闭详情后在 Agent 终端和会话搜索框里打字都正常。S7b 的搜索焦点拦截(`AGENT_KEYS_BLOCKED`)不能被破坏。
- 发布步骤按总览 §4,但**不在本任务内**,见文末。

## 容易踩的坑(Review Focus 之外再强调)

- **加载状态机:** 连点两行,旧页面的 `DocumentLoaded` 晚到不得让新内容提前可见(校验页面 URL 里的 `_r` 等于当前 nonce);出错或条目为空时库会销毁 webview、`review_pane_state` 归为 Ready 显示文案,**不能永远停在「加载中」**;「加载更多」追加后重新导航并换新 nonce。若 wry 的 IPC `Request::uri()` 拿不到页面 URL,退路是把 nonce 记进实例并在每次 `Loaded` 前核对,仍不可靠就**停下报告**。
- **单槽跨项目:** 切项目、关项目页签、详情被清空,必须先销毁或先清数据再换,**不得短暂显示别的项目的内容**。协议闭包持有可换的数据槽:切项目先置 `None`(此时 `data.json` 应答 404,与 Dozer 一致),再换新项目的 `review_snapshot`。
- **隐藏时机:** 面板切走、收起、被对侧放大盖住、窗口关闭、最小化、全屏进出、缩放、拖分隔线、列表折叠/展开,位置与可见性都要对;面板刚变为可见的第一帧不闪旧内容;未 `DocumentLoaded` 前离屏停放(避免 hidden 下 rAF 被挂起拖慢首帧)。
- **缩放:** 创建时与 `icon_size::scale()` 变化时调用 `webview.zoom(scale)`。
- **失败不静默:** webview 创建失败、协议异常走 S7b 已有的通知条 + stderr,不留空白块。Digger 没有 Toast。
- **`load_more_button` 在 webview 存在时是否可见:** Dozer 现状没实测,**照现状、记账**,不在 Digger 里改。
- **用 `HOME` 隔离 Dozer 的对照环境**依赖其数据目录随 `$HOME` 变化(macOS `ProjectDirs` 读 `$HOME`),先验证再用。
- 库与页面的问题(卡顿、渲染缺陷)**报告给用户,不在 Digger 里加绕过**。

## 约束

- 库不改、页面不改、严格 CSP 不放宽;页面不得请求外部资源。**不改 Dozer、不改 `dozerd`、不改协议。** 若发现库或接口缺口,报告,不绕。
- 只用 git tag 依赖;libs 钉版保持 `libs-2026-10-26b`,不升。
- 验收测试在 `git archive` 导出的**干净快照**里跑(路径不含 "digger";快照要有 git 历史:`git init` + `git fetch <worktree> HEAD` + `git reset --soft FETCH_HEAD`,否则 `extensions::git_log` 的 3 个测试确定性失败)。`CARGO_TARGET_DIR` 不与别的工作区共享。
- commit 前 `git diff --cached --stat` 看 staged 全貌;只 `git add` 具体路径,不 `git add -A`/`.`。提交信息末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。
- 不碰用户的沙箱 `/private/tmp/digger-sandbox`、`/Applications` 里正在运行的应用及其 `dozerd` / `diggerd`;验证用独立 `DIGGER_DATA_DIR` 与临时 `HOME`。
- **删除项目清会话是破坏性动作**(`delete_project_transcripts` 在 Dozer 里会同时删用户 home 下的 transcript 文件),**本期不接**,写进 open-items 交用户裁决(只清 `digger.db` 副本,还是也删原文件)。
- 对照验收的「已接受差异」只允许 S6d 记过的两条(悬停阶跃、延迟提示精度)与经用户确认的;其余必须修或交用户裁决。

## 交付

- Task 0–6 各自的提交,每个任务先 red 后 green。
- 完工前干净快照里全绿(偶发失败量化后注明):`cargo build --workspace && cargo test --workspace --no-fail-fast && cargo clippy --workspace --all-targets && cargo fmt --check`;`bash scripts/check-host-deps.sh`(含 `--self-test`)、`bash scripts/check-shared-deps.sh`(在 worktree 里跑,archive 无 `.git` 会 exit 128)、`bash scripts/check-shell-regions.sh`、Cargo.lock 重复检查。
- 夹具(Task 4)全为自造数据,不含真实用户内容;种子脚本拒绝在 `$TMPDIR` 之外写入、不碰真实 `HOME`。
- 报告写到 `docs/superpowers/reviews/2026-10-<日期>-conversations-s7c-report.md`:基线与终态测试总数及核对算式(`基线 − 已删 + 新增`,`usage_native`/`todo_native` 既有测试不得减少)、每个 Review Focus 对应的测试名、与计划不一致处及原因、每条「通过」附命令与退出码、**没验的东西明说**。另写 `notes/2026-10-18-conversations-parity.md`、`…-regression.md`、`…-open-items.md`(三条:删除项目清会话、总结生成开通与费用、页签部件/菜单弹层/`review_native` 样板迁入 byteui/bytehost 的跨仓库任务)。
- **你的环境多半启不了 GUI**(S7b 就是这样)。那就如实写「GUI 未验」,准备好画廊(`conversations_gallery --state …`)、种子脚本与一个 `launch.sh`(隔离 HOME 与数据根),把 Digger 一侧的状态摆法和 Dozer 一侧的摆法清单写成**给用户的目测验收清单**(含:审阅 webview 位置与 Dozer 对齐、折叠/展开、拖分隔线、开关原生 Agent 筛选时 webview 保持、切项目不串内容、关闭详情后键盘归还、中文输入法、`huge.jsonl`/`hostile.jsonl`/`broken.jsonl`/`empty.jsonl` 表现、`kill diggerd` 后详情仍可读)。不伪造截图,不替用户点击。
- 停在 `feat/conversations-review`。

## 不在本任务内

**合并、发版、tag、推送、安装、升版本号都不做。** 原计划 Task 7 的「Dozer 与 Digger 各发一个新版本」由用户决定节奏(用户倾向把多处改动攒成一个版本);Dozer 自 0.5.306 起还有 Codex 兜底、S7a、S8a、MCP 守卫未发,Digger 自 0.1.6 起有 S3b-1、S3b-2、S7b 未发。你只需保证门禁全绿、版本文件不动。之后:S8b(Digger 群列表)、S8c(聊天页 webview,直接复用本任务抽出的 `native_host`)。
