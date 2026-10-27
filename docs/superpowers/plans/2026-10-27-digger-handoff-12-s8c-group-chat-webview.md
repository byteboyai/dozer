# 交接任务 12:S8c Digger 群聊页 webview(聊天内容、发言流式、转待办、对照与回归)

> 给开发 agent 的任务说明,写于 2026-10-27。**本任务只动 Digger 仓库。** 范围是 S8c 的 Task 2–6(原 Task 1 已在 S7c 完成),跑完自动门禁后**停在分支上等用户验收**。合并、发版、tag、推送、安装都不在本任务内(原 Task 7 不做)。

**目标:** 把 S8b 内容块里的「聊天内容即将可用」占位换成真实群聊页:成员条、消息流(流式回复)、输入框、@ 提及、取消/重试、成员增删改、删群、**转待办**。页面是 Dozer 同一份 `group-chat-content`(库内置),经 Digger 的原生 webview 宿主显示;数据与动作全部走库的 `host::*` / `protocol::*`。**库、页面、`dozerd`、协议都不改。**

## 先读(只读,按顺序)

1. `dozer/docs/superpowers/plans/2026-10-25-s7-s8-refresh.md`:修订总览,**优先于原稿**。重点 §1.1(成员只读)、§3「通用」与「S8c」一节、§5 Q1/Q3/Q5/Q6。
2. **原稿(按下文修订执行):** `digger/docs/superpowers/plans/2026-10-18-group-chat-s8c-chat-and-release.md`。先读 Global Constraints 与 Review Focus;每个任务先写失败测试。
3. 设计稿:`digger/docs/superpowers/specs/2026-10-18-group-chat-panel-s8-design.md`(§6.3、§7)。
4. 样板(**以代码为准**):S7c 的 `crates/desktop/src/adapters/{native_host.rs,review_native.rs,review_geometry.rs}` 与 `product.rs` 里审阅 webview 的接线、S7c 报告 `docs/superpowers/reviews/2026-10-27-conversations-s7c-report.md`;S8b 的 `adapters/{group_chat.rs,group_chat_display.rs,group_chat_overlay.rs,notice.rs}`、报告与已知缺口 `docs/superpowers/reviews/2026-10-27-group-chat-s8b-report.md`、`docs/superpowers/notes/2026-10-27-group-chat-s8b.md`。
5. 库(`dozer/crates/dozer-panel-group-chat/src/`,已核实的真实形状,**不按原稿设想的名字**):
   - `host::handle_webview_event(state, push, project_id, event, io) -> HostAction`(**没有 `caps` 参数**);`HostAction::{None, ShowTodoPanel}`。`Ready` / `Failed` / `SetDraft` 在库内处理;其余经 `protocol::route_event` 变成 `Command`(`CreateGroup`/`SelectGroup`/`DeleteGroup`/`AddMember`/`UpdateMember`/`RemoveMember`/`Post`/`Cancel`/`Retry`/`PushTodo`/`OpenTodo`),`PushTodo` 等由 `spawn_command` 经 `GroupChatService` 执行,`OpenTodo` 返回 `ShowTodoPanel`。
   - `host::take_content_script(state, push, project_id, webview_available, now, stamp: LocaleStamp, locale) -> Vec<String>`:返回要 `evaluate_script` 的脚本(一次性草稿恢复 + 声明式推送,已含 `window.__dozer?.dispatch(...)` 包装,**宿主不再二次包装**)。
   - `assets::{serve_group_chat_content(path) -> Option<(mime, bytes)>, group_chat_webview_spec(failed, loaded_or_ready, scheme, theme) -> Option<GroupChatWebviewSpec{url,visible,park_offscreen}>}`;URL 形如 `{scheme}://group-chat-content/host.html?theme={theme}`(注意是 `theme`,不是原稿的 locale);`failed` 时返回 `None`,由宿主显示失败壳。
   - `protocol::{parse_group_chat_event, WebviewPushState{set_ready, set_failed, failed, clear_failed, ...}, GroupChatWebviewEvent}`;`host::poll_wanted(state, panel_visible)`。
   - **没有 `GroupChatCapabilities`**:S8a 已删除,「转待办」默认开启(§5 Q1)。

## 基线

- Digger 仓库 `/Users/chrischiang/Projects/CoralProjects/byteboy/digger`,`main` 在 **`3e0fefc`**(已含 S7c、S8b)。libs 为 `libs-2026-10-26b`,**不升 tag**。
- 测试基线:S8b 合并前干净快照 **807 个全过**,另有 S8b 提示补丁新增 1 个;开工先在干净快照记基线。偶发失败集中在 `adapters::agent_hooks::tests::*`(单独重跑也可能偶发)与 `apps::tests::closing_during_startup_waits_and_releases_the_root`;**单独重跑量化(次数、失败数写进报告),不靠重跑到绿**。
- worktree:`../digger-s8c`,分支 `feat/group-chat-chat`,从 `main` 切出;sibling 不可写时用 `/private/tmp/digger-s8c` 的独立 clone 并在报告写明。
- **不要动 Digger 主工作区**:用户有未提交改动(13 份已删除的 S6 文档、未跟踪的 `.qoder/`)。不碰 `feat/preview-pairing` / `digger-p4`。沙箱可能禁 `/bin/ps`,使 `digger-app-host` 的 recovery 测试在你那边失败,如实记录、不改该测试。

## 与原稿不同的地方

- **Task 1(抽 `native_host.rs`)已完成**,不再做。直接用 S7c 的 `native_host::{Desired, Document, Action, plan, allows_navigation, Host, apply_actions, sync_outcome, allows_download, new_window_response}`;群聊宿主 `adapters/group_chat_native.rs` 照 `review_native.rs` 的形状写。**不得改动 `native_host` 的既有行为;若必须扩展,既有测试条数不减、原样通过。**
- **协议名** `digger-groupchat`,主机名 `group-chat-content`;导航白名单只放行 `digger-groupchat://group-chat-content/…`,变体(`…content.evil/`、`…@evil/`、端口、大小写、反斜杠、控制字符)一律拒绝;新窗口、下载一律拒绝。
- **库函数以第 5 条为准:** 原稿 Task 2 里「`handle_webview_event(.., caps, ..)`」「`webview_available = 实例已创建`」「`ShowTodoPanel` 收到则忽略」都要改:无 `caps`;**`HostAction::ShowTodoPanel` 映射成「切到待办面板」**(`ShellMsg` 里选中 `digger.todo` 的消息,以 S3b 实际形状为准);`PushTodo` 成功后待办面板要能看到新条目(检查待办面板是否靠轮询刷新,不够就在 `PushTodo` 成功回执后触发一次待办刷新——**只报告所需最小改动,不改待办面板的既有行为**)。入口点了必须真的新建一条待办或切到待办面板,不得有死入口。
- **IPC 来源校验:** 原稿说核对 `request.uri()`;S7c 已用同样做法(`review_native.rs` 的 `nonce()` 读原始 URL,限长 2048、拒 `#`、`%`),群聊照做:来源不是 `digger-groupchat://group-chat-content/…` 的 IPC 丢弃并记日志;通过校验后才 `parse_group_chat_event`,解析失败走通知条与 `eprintln!`。
- **几何:** 原稿 `chat_bounds` 需要对照 Dozer `group_chat_content_pane_bounds_for(..)`(`dozer/crates/dozer-app/src/webview_geometry.rs:669`,走 `pair_content_pane_bounds_for` 的 `chrome_top = 0` 分支)。按 S7c 的做法:**先在锁定的 Dozer 源码快照里算出真实数字**(1200×800、右栏默认分割、折叠/展开、不同缩放),写进测试,差值不超过 1 像素;Digger 实际 shell 的 `footbar_height=22` 与 Dozer `status_bar_height=26` 的差用共享度量计算,**不写死补偿偏移**;S7c 报告里「原稿额外减 8px 的假设与真实 Dozer 函数不符」是前车之鉴,以真实函数为准。群聊详情列没有原生面板头。
- **覆盖层规则(§5 Q3):** 原生菜单(Agent 筛选)**不**隐藏聊天 webview;iced 内的模态(新建群弹窗 `Overlay::GroupChatCreate`、确认框、Agent 选择器、溢出菜单等)隐藏/停放聊天 webview,关闭后恢复。`overlays_open()` 只含 iced 内部覆盖层——S8b 已把新建群弹窗算进去,先读 `product.rs` 现状与 `product_review_tests.rs` 里对审阅 webview 的同类测试,**对称地写群聊的**。
- **单槽与两个 webview 并存:** 审阅 webview 在会话面板、聊天 webview 在群聊面板,二者各有自己的 `native_host::Host` 实例(单槽是每宿主一个)。同一时刻若两个面板在配对布局里都可见(右栏一次只显示一个选中面板,先核实),也要有测试证明互不影响、切换时旧的被销毁。
- **键盘焦点归还:** webview 销毁/隐藏后让窗口内容视图成为第一响应者。S7c 已在 `native_host` 做了焦点归还,**先读它的实现并复用**,不要重写;若发现对群聊场景不够,补测试再扩展。
- **中文输入法/粘贴:** 聊天输入框在 WKWebView 里,拼音输入、候选框位置、多行粘贴、⌘A/⌘C/⌘V、Esc 只能人工验,写进用户清单。
- **假 Agent 命令行(Task 4)约束收紧:** 假脚本**必须只记录、不执行**传入内容;它记录下来的命令行参数里要能看到**只读标志**(Claude 的工具白/黑名单参数、Codex 的 `--sandbox read-only`),并加测试断言**不出现** `--dangerously-skip-permissions` 或 `-y`。先读 `dozerd/src/group_adapter.rs` 的参数拼装,脚本输出约定以它为准,并在 Dozer 一侧(临时快照里)做一次往返自测证明约定没写错;**不要提交对 Dozer 仓库的改动**。
- **删除群:** 删群入口在聊天页内(库的页面事件)。删除是用户的显式操作,但不得拉起任何进程;测试断言删群只调用 `delete_group`。
- **悬停/延迟提示:** 对照验收的「已接受差异」只允许 S6d 记过的两条(悬停阶跃、延迟提示精度)与经用户确认记账的;其余必须修。S8b 记的「`list_collapse_button` 是库适配里的空占位、Digger 在产品层另加折叠按钮」属已知偏离,照常记账即可。

## 约束

- **成员 Agent 在执行层只读;严禁**给任何群聊调用加 `--dangerously-skip-permissions` / `-y`;群聊**不分配任务**,「转待办」只在 Todo 里新建一条;`@` 只由人触发,agent 回复里的 `@` 不解析(这些在库/`dozerd`,Digger 不得绕开或放宽)。
- 页面里所有 agent 输出走库页面自带的转义渲染器;宿主**不得**把任何消息内容拼进 `innerHTML` 或脚本字符串(注入脚本只用库给的 `take_content_script`)。
- 严格 CSP 不放宽,页面不得请求外部资源;消息里的外链、远程图片不得发出网络请求(用请求回调计数或系统网络日志确认零外部请求)。
- 单槽语义:任一时刻最多一个群聊 webview,显示聚焦项目的聊天;切项目页签、关项目页签时销毁或换数据,**不得短暂显示别的项目的消息或草稿**(草稿由库在宿主侧保存、页面重建时一次性恢复)。
- `ready` 之前离屏停放且不推送;`ready` 之后推全量;同一状态不重复推送;`ready` 10 秒超时判失败,显示库/产品的失败壳与「重试」;失败后迟到的 `ready` 不撤销失败;重试后重新计时。
- 项目隔离、`thread_local` 只放 wry 句柄、订阅回调读状态用进程级 `AtomicBool`(与 S6d/S7b 一致);轮询只在 `poll_wanted && 面板可见 && !closing` 时订阅(S8b 已做,不得退化)。
- 失败不静默:webview 创建失败、协议异常、IPC 解析失败显示在通知条并 `eprintln!`。
- 只用 git tag 依赖;库/页面/`dozerd`/协议的缺口只报告,不绕。
- 验收测试在 `git archive` 的**干净快照**里跑(路径不含 "digger";带 git 历史:`git init` + `git fetch <worktree> HEAD` + `git reset --soft FETCH_HEAD`);`CARGO_TARGET_DIR` 不与别的工作区共享。
- commit 前 `git diff --cached --stat`;只 `git add` 具体路径;提交信息末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。
- 不碰用户的沙箱 `/private/tmp/digger-sandbox`、`/Applications` 里正在运行的应用及其 `diggerd`、真实 `HOME` 与数据;验证用独立 `DIGGER_DATA_DIR` 与临时 `HOME`。真实 Agent 抽检只由用户操作。

## 任务(按原稿编号)

- **Task 2:** `group_chat_native.rs`、`group_chat_geometry.rs`、`GroupChatPanel::{desired_for_chat, on_webview_event}`、`Msg::GroupChatNative(Event)`、`sync_native` 注入脚本、关闭路径销毁。测试清单见原稿,按本文修订(`caps` 相关测试改成「`OpenTodo` → 切到待办面板、`PushTodo` → 调用 `push_group_message_to_todo` 恰好一次」「越界 id/他群 id/超长文本的伪造事件被库拒绝」)。
- **Task 3:** 覆盖层、可见性与键盘焦点(复用 S7c 的焦点归还)。
- **Task 4:** 假 Agent 命令行、`group_chat_gallery` 扩展状态(`empty`/`one-group-idle`/`streaming`/`long-history`/`failed-message`/`cancelled`/`removed-member`/`webview-failed`/`dialog-open`/`daemon-unavailable`/`hostile`)、`scripts/with-fake-agents.sh`、逐状态对照清单 `docs/superpowers/notes/2026-10-18-group-chat-parity.md`。**你的环境多半启不了 GUI**:准备好夹具与脚本,把 Dozer 对照摆法写成给用户的清单;不伪造截图,不替用户点击。
- **Task 5:** 压力、恶意内容、`diggerd` 中途死亡、键盘/输入法、同机并存、真实 Agent 抽检(用户操作),回归记录 `docs/superpowers/notes/2026-10-18-group-chat-regression.md`。无法在你的环境里做的项如实标「未验」并写进用户清单。
- **Task 6:** 文档:设计稿状态改「代码已实现,GUI 目测验收待用户」并补「实现结果」;`ADR-015` 补一句群聊读写各自产品的库、成员 Agent 由各自 `diggerd` 拉起;遗留缺口写 `docs/superpowers/notes/2026-10-18-group-chat-open-items.md`:① 创建失败时草稿丢失(库 `NewGroupSubmit` 先关弹窗,需改库);② 页签部件/菜单弹层/`native_host` 与 wry 样板迁入 byteui/bytehost 的跨仓库任务;③ S8b 的 `no_todo_text_anywhere_in_the_view` 测试没有断言(S8c 里改成真断言,聊天页有转待办入口后它应当删除或改写成「入口存在且可用」)。
- **Task 7(发布)不在本任务内。**

## 交付

- 各任务提交,每个任务先 red 后 green。
- 干净快照里全绿(偶发失败量化后注明):`cargo build --workspace && cargo test --workspace --no-fail-fast && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all -- --check`;`bash scripts/check-host-deps.sh`(含 `--self-test`)、`bash scripts/check-shared-deps.sh`(在 worktree 里跑)、`bash scripts/check-shell-regions.sh`、Cargo.lock 重复检查。
- 报告 `docs/superpowers/reviews/2026-10-<日期>-group-chat-s8c-report.md`:基线与终态测试总数及核对算式(`基线 − 已删 + 新增`,`native_host`/`review_native`/S8b 既有测试条数不得减少)、每个 Review Focus 对应的测试名、与计划不一致处及原因、每条「通过」附命令与退出码、**没验的东西明说**。
- 停在 `feat/group-chat-chat`。

## 不在本任务内

合并、发版(Dozer 与 Digger 的新版本)、tag、推送、安装、升版本号。**发版由用户确认后另行进行**;发版时 Digger 0.1.6→0.1.7 一并带上 S3b、S7b、S7c、S8b、S8c。
