# 错误反馈审计:哪些接入 Toast、哪些保留

状态:审计结论(2026-09-30),只读盘点,未改任何源码。为 `2026-09-30-unified-toast-design.md` 之后的"第二份迁移计划"提供依据。

行号是审计时刻的快照,主 checkout 有并发改动(例如 `app/update.rs` 相对我之前引用的行号已漂移约 10 行),执行迁移时以符号/上下文为准,不要按行号盲改。

范围:`crates/dozer-app/src`。盘点了两类来源:

1. 存进状态、由视图渲染的错误字段(约 27 个 `Option<String>`/枚举内嵌错误)。
2. 只写日志的失败(`tracing::warn!/error!`、`eprintln!`,共 80 处)。

**没有盘点**:静默吞错的写法(`let _ = ...`、`.ok()`、`Err(_) => {}`)。这类数量更大且不带日志,需要另一轮扫描,见「未覆盖」。

## 结论摘要

- **状态字段里,绝大多数应该保留原位**:它们是面板内容的一部分(加载失败、渲染失败、表单校验),带位置上下文,Toast 反而会丢失"哪里出的错"。真正应迁移的只有 `agent_context.notice`、`daemon_error` 里的两处一次性事件(已在第一份计划中),以及 `tree_error` 里的一半(见下)。
- **真正的缺口是"只写日志、用户完全看不到"的失败。** 80 处里我判断约 12 处是"用户刚做了某个操作、操作失败了、界面毫无反馈",这些才是 Toast 最有价值的接入点(表 C1)。其余大多是内部噪音、诊断输出或合理降级,保持日志即可(表 C2)。
- **阻塞第二份计划的是一个设计问题,不是工作量**:extension 的 `update` 只有自己的 `emit`,没有通往 App 级 Toast 的通道(见「extension 如何触达 Toast」)。第一份计划的 Toast 核心接口**不需要改**,该问题可以在第二份计划里用一个小的 `Outbox` 类型解决;建议把这个类型提前放进第一份计划(见「对第一份计划的影响」)。

## 判据

沿用 spec:**"刚刚发生了一件事" → Toast;"当前处于某状态" → 保留状态位。** 补充三条实操规则:

1. 错误依附于某个具体控件/字段/面板内容(表单校验、面板里的加载失败),留原位——用户需要在出错的地方看到它。
2. 需要重试/纠正上下文(停止 dozerd 失败后仍要保留"重新启动"按钮),留原位。
3. 用户发起了动作、动作在后台失败、界面没有任何变化——Toast。

## A. 状态字段:逐个判定

| 字段 | 位置 | 语义 | 判定 |
|---|---|---|---|
| `daemon_error`(打开项目失败) | `app/update.rs` `project_tab_opened` | 一次性事件 | **迁移**(第一份计划 Task 4) |
| `daemon_error`(删除项目未完全成功) | `app/update.rs` `ProjectDeleteDone` | 一次性事件 | **迁移**(第一份计划 Task 4) |
| `daemon_error`(dozerd 已停止/连不上) | `update.rs`、`runtime.rs` 启动 | 持久状态 | **保留**;阶段 3 改成不占布局的徽标并收敛三处渲染 |
| `agent_context.notice` | `extensions/agent_context.rs` | 一次性事件 | **迁移**(第一份计划 Task 5) |
| `tree_error`(**更正,2026-09-30**) | `extensions/files/update.rs`、`files/state.rs` | 原审计判为"加载/读取失败,保留"是**错的**:读代码后确认它的全部写入点都是**文件操作反馈**——粘贴失败、回滚失败、新建/重命名/删除失败(`OpDone`)、拖拽移动被拒/失败、移动对话框校验、行内新建/重命名校验;**没有任何"加载失败"用法**(仅有的测试访问器把它当路由标记用)。 | **拆分**:仅"移动对话框内的两处校验"(`MoveConfirm`,对话框保持打开)留在对话框内联(字段改名 `move_error`);其余全部是"操作被拒/失败"→ Toast。见迁移计划 Task 9 |
| `git_error` | `extensions/files/update.rs` | git 状态加载失败,面板内容 | 保留 |
| `web_error` | `preview/view.rs` | 预览渲染失败,由 `preview_fallback_page` 承载 | 保留(CLAUDE.md 已规定 fallback 页统一承载) |
| `preview_error`/`project_preview_error`("文件不存在或不可读") | `workspace/state.rs`、`app/update.rs` | 预览面板内容 | 保留 |
| `ReviewView.error` | `workspace/state.rs` | 审阅面板内容 | 保留 |
| `git_log.error`/`diff_load_error` | `extensions/git_log.rs` | 面板内容(内联渲染) | 保留 |
| `rollback_error` | `extensions/file_history.rs` | 回滚失败,渲染在 file_history 弹窗内 | 保留(独立 overlay 窗口内的上下文错误;Toast 在主窗口右下角,与该弹窗无视觉关联) |
| `codehealth.scan_error` | `extensions/codehealth/mod.rs` | 面板 banner,同时被 `view_model` 用来决定展示态 | 保留 |
| `codehealth.save_error` | `extensions/codehealth/*` | 结果旁的状态说明("未保存,无法用于下次比较") | 保留 |
| `database.tables_error`、`QueryState.error`、`database view` 里两处 `error` | `extensions/database/*` | 表列表/查询结果状态 | 保留 |
| `search.error` | `extensions/search.rs` | 搜索弹窗内容 | 保留 |
| `project_create.error` | `extensions/project_create.rs` | 表单内联 | 保留 |
| `settings` 的 `ConnectState::Editing.error`、`AdvancedState::{Idle,Stopped}.error` | `extensions/settings.rs` | 设置弹窗内联;失败后要保留原按钮 | 保留 |
| `browser.error` | `extensions/browser.rs` | 页签/地址栏状态 | 保留 |
| `project.error`("改名失败/保存失败/保存记忆失败") | `extensions/project/update.rs`,渲染于 `project/view.rs:292` | 用户提交编辑后失败,内联在面板顶部 | **可选迁移**(低优先级):它确实是"动作失败"语义,但内联显示在正在编辑的面板里也合理;倾向保留,除非用户觉得看不到 |

小结:A 表 24 行里只有 4 行(其中 3 行已在第一份计划)明确迁移,1 行需拆分,1 行可选,其余保留。

## B. 只写日志、应升级为 Toast(C1)

判据:用户发起了动作,失败后界面无任何变化。

| # | 位置 | 现状 | 建议 | 级别 |
|---|---|---|---|---|
| 1 | `extensions/files/update.rs`(`OpenWithDefault` 里 `command.spawn()` 失败) | 只日志,注释明确写"不弹 toast" | Toast:"无法用 {app} 打开 {文件名}" | Error |
| 2 | `app/update.rs`(`外部打开失败`,预览工具栏的外部打开) | 只日志 | 同上 | Error |
| 3 | `extensions/todo/update.rs`(`Mutated(Err)`,"Todo 写操作失败") | 只日志;随后照常 refresh,列表回到旧状态,用户以为没生效 | Toast:"Todo 操作失败:{e}" | Error |
| 4 | `extensions/todo/update.rs`(`CategoryMutated(Err)`) | 同上 | Toast | Error |
| 5 | `extensions/ssh.rs`(两处 `save_hosts` 失败) | 只日志;界面显示保存成功,重启后消失 | Toast:"SSH 主机未能保存到磁盘" | Error |
| 6 | `extensions/database/update.rs`(两处 `写入 database.json 失败`) | 同上 | Toast | Error |
| 7 | `extensions/database/state.rs`(`写入 database_drivers.json 失败`) | 同上 | Toast | Error |
| 8 | `app/app.rs`(`项目页签集合写盘失败`) | 只日志;下次启动丢页签 | Toast(Warning) | Warning |
| 9 | `app/update.rs`(`回报预览命令结果失败`)、`runtime.rs`(`预览导航失败`) | 只日志;agent 侧的 MCP 预览命令失败,人类不知道 | 待定:属于 agent 触发,不是用户动作 | 见未决项 |
| 10 | `extensions/conversations.rs`(`对话会话列表查询失败` 后回退空 `Vec`) | 只日志;列表静默变空,用户以为没有对话 | 优先做成面板内空态提示(状态),不是 Toast | — |
| 11 | `app/update.rs`(`提交总结任务失败`、`总结任务不存在`、`查询总结任务失败`)、`extensions/project/update.rs`(`SummaryBackfill*`) | 已经通过 `SummaryGenerateFinished`/`SummaryBackfillFailed` 消息回到面板状态 | 已有状态反馈,**不需要** Toast | — |
| 12 | `workspace/state.rs`(`写入终端失败`、`自动键入初始命令失败`、`派发任务文本失败`) | 只日志;用户点了"派发到终端"但什么都没发生 | Toast:"未能写入终端" | Error |

表中 #9、#10 不直接迁移(见判定),所以 C1 实际可确认的接入点是 **#1–#8、#12,共 9 处**。

## C. 只写日志、保持日志(C2)

约 50 处,分几类,不需要用户看到:

- **诊断/调试输出**:`window.rs`/`window_events.rs` 的 `[DIAG]`、`DEBUG term input fallback fired` 等,不是错误。**建议单独清理**(与 Toast 无关,是应删除的遗留调试日志)。
- **启动/系统探测降级**:`capabilities.rs` 内存探测、`runtime.rs` 安全启动、`自动拉起 dozerd`、`重试连接 dozerd`(后者已经间接触发 `daemon_error`)。
- **IPC 协议合法性拒绝**:`runtime.rs`/`file_history_overlay.rs`/`edit_history_overlay.rs` 的 `拒绝无效/无法解析 … IPC`(约 15 处)。这是 webview 发来非法消息的防御,用户无关。
- **平台钩子安装失败**:`window.rs`、`file_drag.rs`、`native_menu.rs` 的 selector 覆写失败。开发期问题。
- **后台清理/收尾**:关 tab 触发总结失败、结束会话失败、丢弃过期促成结果、退出前等待超时。用户已离开该上下文。
- **可自动恢复的降级**:`git_watch 启动失败,降级为手动刷新`、`恢复预览源码模式失败,回退渲染模式`、`终端事件滞后`、`搜索遍历跳过错误条目`、大文件行索引/表格首次加载失败(后两者已由面板 fallback 页承载)。
- **布局类写盘失败**(外壳/面板/退出前窗口尺寸):丢失代价是下次启动布局回到默认,用户几乎不会察觉;倾向保持日志。**#8 的项目页签集合写盘失败**代价更高(丢页签),故单独列入 C1。

## D. 阻塞问题:extension 如何触达 Toast

现状:`extension::update(state, msg, ..., emit)` 中的 `emit` 只能发**该 extension 自己的 Message**,`update` 也拿不到 `App`。表 C1 的 #1、#3–#7 都发生在这样的 `update` 里。第一份计划的 `Message::Toast` 需要一个能发顶层 `Message` 的通道,目前只有 App 自己和 `proxy`(`EventLoopProxy<Message>`)有。

候选方案:

| 方案 | 做法 | 优点 | 缺点 |
|---|---|---|---|
| **1. Outbox(推荐)** | 在 `toast.rs` 提供 `Outbox`(`Vec<(Level, String, Option<String>)>` + `push/take`),嵌进需要的 extension state;`update` 里失败时 `state.outbox.push(..)`;App 在处理完该 extension 的消息后统一 `drain` 成 `push_toast` | 纯函数、可单测(断言 outbox 内容,不需要窗口/运行时);extension 不依赖 App;与第一份计划里 `take_notice` 是同一模式,可统一 | 每个要弹 Toast 的 extension state 多一个字段;drain 点要覆盖"经 proxy 异步回来的消息"(它们同样经过 `update`,所以路径统一) |
| 2. 给 `emit` 一个通用出口 | 各 extension 的 `update` 多收一个 `notify: impl Fn(Level, String)` | 调用点最直接 | 参数列表已经很长(`files::update` 有 7 个参数,CLAUDE.md 反对继续加同型参数);所有 extension 的签名与全部调用点、测试都要改 |
| 3. 全局静态发送端 | `toast::notify(level, text)` 读一个 `OnceLock<EventLoopProxy>` | 任何线程/任何位置一行调用,改动最小 | 隐式全局依赖;单测里没有 proxy 需要 stub;与"extension 间只能显式消息通信"的原则相悖 |
| 4. 各 extension 加自己的 `Notify` Message 变体 | `Message::Notify(Level, String)`,由内核拦截 | 显式 | 每个 extension 重复一个变体 + 一段拦截;内核 `match` 膨胀 |

**推荐方案 1**,理由:测试性最好(这也是第一份计划坚持纯逻辑层的同一原因),不引入全局状态,不改任何现有函数签名。对**没有 extension state 的后台线程**(如 `app/app.rs` 的写盘失败、`workspace/state.rs` 的终端写入失败),它们本来就持有 `proxy`,直接 `proxy.send_event(Message::Toast(..))` 即可,不需要 Outbox。

drain 点:`App::update` 末尾对"本次消息可能触及的状态"统一取。最简单的做法是每次 `update` 之后扫一遍当前聚焦及被路由项目的 workspace 的 outbox 与 App 级 outbox;`Message` 数量大但每次扫的只是几个 `Vec::is_empty()`,开销可以忽略。

## E. 对第一份计划(`2026-09-30-unified-toast.md`)的影响

- **Toast 核心接口不需要改**:`push_toast`、`Message::Toast`、`ToastCenter` 足以承载后续迁移。
- **建议(可选,由你决定)把 `Outbox` 提前加进第一份计划的 Task 1**:约 15 行加 2 个单测。好处是 Task 5(`agent_context` 迁移)可以直接用 `Outbox` 替代临时的 `take_notice`,第二份计划不用再回头改第一份迁移过的代码。不加也不会阻塞:第二份计划里再引入 `Outbox`,把 `agent_context` 顺手改过去即可,只是多一次触碰。
- 第一份计划里"file open 外部应用失败"移出第一批的处理**与本审计一致**——它在表 C1 的 #1,等 `Outbox` 落地后接入。

## F. 第二份计划的建议批次

依赖:第一份计划合并,且 `Outbox` 已存在。

1. **批 1(最小、收益最高)**:C1 的 #1、#2(外部打开失败)、#3、#4(Todo 写失败)。改动集中在 `files`、`todo` 两个 extension 与 `app/update.rs` 一处。
2. **批 2(持久化写盘)**:#5(ssh)、#6、#7(database)、#8(项目页签集合)。都是"界面显示成功但没落盘"。
3. **批 3(终端写入)**:#12。需要决定级别与防刷屏:连续键入失败可能瞬间触发几十次,靠 `key` 去重成一条,同时要确认不会把终端本身的错误状态淹没。
4. **批 4(字段拆分,已并入 `2026-09-30-logging-and-toast-migration.md` Task 9)**:`tree_error` 拆成"移动对话框内联错误(`move_error`)"与"文件操作失败/被拒(Toast)",改动最大,需要动 `files/state.rs`、`files/update.rs` 的多处赋值与 `files/view.rs` 的两处渲染,单独评审。
5. **批 5(可选)**:`project.error`。

每批独立成一个 Task,单独提交、可单独合并。

## 未决项(不擅自定死)

1. **agent 触发的失败是否给用户看**(#9):agent 通过 MCP 让 Dozer 导航预览失败,是 agent 的操作而非用户操作,弹 Toast 可能对用户是噪音;也可能正是"AI 在你不知道的时候做了事并失败了"的可见性。取决于产品立场,需要你决定。
2. **`conversations` 列表查询失败**(#10)更像"面板空态里加一行说明",是状态问题,是否要做需要另议。
3. **Outbox 是否提前进第一份计划**(见 E)。
4. **`[DIAG]` 与 `DEBUG` 遗留日志的清理**:与 Toast 无关,但审计中发现,是否顺手清理?
5. **`project.error` 是否迁移**。

## 未覆盖

- 静默吞错(`let _ = ...`、`.ok()`、`Err(_) => {}`)。它们和"只写日志"是同一个问题的更糟版本(连日志都没有)。建议第二份计划批 1 完成后,另做一轮只针对用户发起动作路径的扫描。
- `dozerd`/`dozer-mcp`/`dozer-hook` 等其他 crate 的错误处理不在本审计范围(它们没有 UI)。
- 本审计没有逐个验证 C2 中每条日志的触发频率;个别(如 `终端事件滞后`)如果在正常使用中高频出现,可能值得升级为状态提示,需要真实使用数据。
