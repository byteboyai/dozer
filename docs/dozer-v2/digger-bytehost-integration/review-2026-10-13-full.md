# Digger / bytehost 集成复审（2026-10-13）

结论：不通过“23 个 Task 已完整完成”的验收。当前有可复现的数据覆盖问题，且多个桌面功能只有库、控制器和单测，没有生产调用链。后者需要补实现，不能仅补人工截图。

快照：Dozer `0cd91e3e`，Digger `71280a7`，bytehost `7f25d71`（v0.2.0）。本次核对主计划、既有两轮审核、当前三仓源码，重点检查最终组合根与 Tasks 20–23，并复核前轮修复。没有修改生产代码、锁文件或用户 WIP；未将 Dozer 文件树等未提交功能计入集成变更。

以下路径以 `D/` 表示同级 digger 仓库，`B/` 表示 bytehost，`Z/` 表示 dozer。

## 1. [P1] 用量运行 ID 跨重启重复，覆盖历史操作

位置：`D/crates/desktop/src/apps/mod.rs:184`；`D/crates/app-host/src/request.rs:103`；`D/crates/store/src/ledger.rs:168`。

生产者把 `action + ticket.sequence` 拼成持久 run_id，但 RequestTable 每次启动从 0 计数。用户重启后按相同顺序执行同类操作，会得到与上次相同的 run_id。账本按 run_id upsert，因此第二次真实操作覆盖第一次，而非增加记录。它不是同一事件重放，不能按幂等去重处理。

定向复现引用实际 RequestTable、TaskRunner、UsageLedger，使用生产 ID 拼接规则，创建两个新请求表模拟重启并写同一个持久账本：期望 2 条，实际 1 条。修复应采用跨进程唯一的运行 ID（如 UUID，或持久序号），同一次运行的重试才复用 ID。

## 2. [P1] 正常退出直接终止 GUI，没有执行应用服务收尾

位置：`D/crates/desktop/src/product.rs:365`；`D/crates/desktop/src/main.rs:34`。

`CloseRequested | Exit` 只保存布局并返回 `iced::exit()`。生产桌面没有调用 `HostService::begin_shutdown/await_shutdown`，也没有撤 Surface、等待 shutdown report 或显示超时/失败的路径。库中存在这些 API、recovery 测试直接调用它们，不代表桌面退出调用了它们。

应用运行或运行时下载期间退出，不能保证按服务约定 suspend、持久化状态并报告收尾失败；即使某些资源由 Drop 强制清理，也不等价于正常 shutdown。应先进入退出状态、阻止新操作、完成异步收尾，再决定退出，并接管窗口默认关闭行为。

## 3. [P1] 安装后没有启动/打开入口，也没有实际应用视口

位置：`D/crates/desktop/src/apps/actions.rs:118`；`D/crates/desktop/src/apps/mod.rs:423`；`D/crates/desktop/src/product.rs:385`。

应用行只提供停止、回滚、卸载和日志，注释称启动交给宿主栏，但 ProductShell 实际只有信息/应用/用量三个固定页签，没有已安装应用 rail，也没有调用 `Executor::run(StartApp/FetchLaunchUrl)` 的生产路径。`surface_open_pending`、`surface_task` 同样无生产调用者，launch URL 分支仅缓存地址。

因此安装应用后无法在 Digger 中启动/打开并交互；共享 builder 的存在不能满足 Task 15/16、A7。需要接通真实应用选择→启动→获取地址→窗口线程创建→resize/关闭的完整路径。

## 4. [P1] Usage 页取到数据后仍只有标题，没有显示统计内容

位置：`D/crates/desktop/src/adapters/usage.rs:118`；`Z/crates/dozer-panel-usage/src/view.rs:45`。

Digger 仅调用共享 `content_pane`。该函数在 loading 时画动画，完成后只保留标题和背景；实际统计、空态与图表全部由 WebView 展示。Digger 没有注册/创建 Usage Surface、提供资源或推送 payload，故即使真实账本查询成功也无法看到数据。这与有没有真实 agent token 生产者是两个独立问题。

需要挂载共享 Usage 的 Web 内容与生命周期，验证至少一条实际记录能在 UI 中显示，而非只验证 UsageService 返回行。

## 5. [P1] “连数据一起删”只登记清理待办，没有执行浏览器数据删除

位置：`D/crates/desktop/src/apps/mod.rs:550`、`:607`、`:767`。

卸载成功后调用 `store.request`，但 `store_take_ready`、`store_removal_task` 和重试唤醒没有生产调用者；恢复待办也无人消费。清理失败列表没有进入管理视图。另外 `surfaces.close` 返回的原生 Destroy 动作直接被丢弃。

当前尚无应用视口，因此新增安装尚不能走出完整浏览器数据流程；但 Task 17 的删除实现明确未闭环，已有/恢复的待办永远不会完成，接入视口后同 ID 重装可能恢复用户要求删除的数据。应先确认原生视图销毁，再调实际 store removal，持久展示错误并驱动重试。

## 6. [P2] 应用变化订阅和列表兜底轮询未接，状态会长期陈旧

位置：`D/crates/desktop/src/apps/mod.rs:469`；`D/crates/desktop/src/product.rs:475`；`D/crates/app-host/src/effects.rs:268`。

`service_ready` 只发一次 Opened 读取；桌面没有调用 AppHostApi::subscribe，也没有消费 `change_to_message`。产品 subscription 只监听关窗，现有 Tick 只用于日志。执行器对 Subscribe 返回 false，桌面亦未补处理。

后台进程崩溃、健康状态改变等不由管理动作主动触发的变更，无法自动更新已安装列表；切走再回来又被 opened 标志挡住。需要实际持有事件接收任务，接入 Changed/Resync/Disconnected、重订阅和列表轮询。

## 7. [P2] 服务启动失败后，界面承诺的“重开面板重试”无效

位置：`D/crates/desktop/src/apps/mod.rs:233`、`:366`；`D/crates/desktop/src/apps/management.rs:55`。

第一次 Opened 在启动前就把 opened 置 true；ServiceFailed 只更新 startup，未复位 opened。端口占用或同根锁冲突消除后，切走再回应用页仍返回 Task::none，永远不重新启动，虽界面提示“修复后重开此面板会再试”。端口设置/重置模块也未接桌面入口。

应提供真正的重试动作或在 Unavailable 时重新启动，并为重试请求更新身份；增加失败→修复条件→重开成功的产品状态测试。

## 8. [P2] 动作失败通知被静默丢弃

位置：`D/crates/desktop/src/apps/mod.rs:331`；`D/crates/app-host/src/effects.rs:563`。

安装流产生的 Notice 经兜底分支送入只处理服务请求的执行器，命中 `_ => None` 后消失。共享状态机对回滚失败、运行时卸载失败等主要通过 Notice 暴露错误，并退出操作中状态。用户因此只看到操作结束，拿不到失败原因。

应在桌面组合根消费 Notice，渲染持续错误或可见通知，不应把全部 effect 送给返回 None 的执行器后忽略。

## 9. [P2] 损坏账本被替换为临时成功服务，后续记录无法正常恢复

位置：`D/crates/desktop/src/product.rs:119`。

正式账本读取失败后，产品创建 PID 命名的临时空账本，并同时注入生产者和 UsageService。结果查询返回 Ok(empty)，之后操作也写往临时文件；下次启动仍读取原损坏路径，既不会恢复这些新记录，也不再引用上次临时文件。boot_note 虽有提示，但不能兑现“查询失败呈现 Unavailable、完成记录跨重启保留”的契约。

应保留明确的 unavailable 状态，或执行有备份和可恢复路径的正式账本修复；不能让正常记录悄悄转入一次性临时账本。

## 验证结果与边界

- 从 Digger 根目录运行 `RUSTC_WRAPPER= CARGO_TARGET_DIR=/tmp/digger-review-20261013-target cargo test --locked --offline -p digger-store -p digger-app-host --lib`：39 + 15，共 54 项通过。
- Dozer `python3 scripts/audit/check_panel_boundary.py`：通过，213 refs / 5 files。
- 临时复现 `/tmp/digger-review-20261013/src/lib.rs`：跨重启不同操作应保留两条记录的断言失败，实际只有一条。仅借用实际非 GUI 生产模块和生产 ID 规则，不冒充完整桌面端到端测试。
- 前轮请求 sequence 重用、Action latest-only、布局覆盖/宽度、Surface 递归均已有对应修复；bytehost shutdown 超时路径也已改为保留锁并使用总 deadline。上述不再按原问题重复报告；本次发现 1 是不同层级的持久 ID 重用。
- 未运行三仓全量测试、真实窗口/恶意页面/强杀恢复/打包签名验收。当前用户 WIP 未清理、未提交。
- 验收报告自身最新结论仍为 pass 7 / partial 11，A17 和 A18 未通过。因此“完成 23 个任务的提交”与“满足全部完成条件”需要明确区分。修复优先级：历史记录覆盖及退出闭环 → 应用/Usage 真实视口 → 存储清理和事件刷新 → 错误与恢复交互。
