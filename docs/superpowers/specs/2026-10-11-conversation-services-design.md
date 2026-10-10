# Conversations 面板迁移到服务 trait(第五个面板)

> 日期:2026-10-11。状态:待用户审阅。依据:`2026-10-09-dozer-services-x0-design.md`(接缝形状、`ServiceError`)、`2026-10-10-todo-services-design.md`(读失败分档口径)、`2026-10-10-project-services-design.md`、`docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`。

## 1. 已对齐的理解

**目标(沿用前几个面板):** 会话列表面板可进程内测试/复用——取数只依赖小 trait,测试注入内存实现。
**范围:** 只迁 `extensions/conversations.rs` 里的 1 个服务调用(`list_conversations_with_summaries`)及其两条调用链:面板切入的 `on_activate`,和 host 的 `Workspace::spawn_conversations_refresh`(`workspace/state.rs`,被 `app/update.rs` 与 `workspace/state.rs` 共 4 处调用)。
**明确不在范围:** host 侧的会话打开(`SessionOpen`)、详情翻页(`DetailLoadMore`)、总结生成(`SummaryGenerate*`)、`chrome/homespace.rs` 与 `workspace/hook.rs` 里的 `list_conversations`、`get_conversation_turns`、总结任务相关调用。
**不变量:** 线上协议零变化;`SessionRow::from_row` 的映射不变。
**用户已确认的失败口径(用户可见的行为变化):** 刷新失败时沿用 Todo/Project 的分档——已有列表保留;首次加载失败仍显示空列表(不卡在"加载中",与现状一致);`Unavailable` 静默;其余弹带 key 的 Error Toast,相同文本 60 秒内只提示一次。

## 2. 现状(读代码得到)

- `spawn_refresh(project_id, cwd, client, handle, emit)`(`conversations.rs` 约 260 行):调 `client.list_conversations_with_summaries(&cwd, None, 500, 0)`,把每行 `(ConversationSummary, Option<SessionSummaryPayload>)` 映射成 `SessionRow::from_row`,结果 `Result<Vec<SessionRow>, String>` 经 `Message::SessionsRefreshed(project_id, result)` 回传。
- `update` 对 `SessionsRefreshed(_, Err(e))`:只写一条 warn 日志,然后 `sessions = Some(Vec::new())`——**失败会把已显示的会话列表整体抹成空**(G4 的同类缺陷,且更糟)。
- host 对 `SessionsRefreshed` 有一个专属处理臂(`app/update.rs` 约 1514 行):`with_project(project_id, …)` 里调 `conversations::update`,再用刷新后的行去更新已打开的评审/总结视图。该臂把 `result` 原样透传,类型换了之后臂的文本基本不变。
- `WorkspaceState` 没有 `outbox`;日志来源 `LOG`(`panel, "conversations"`)已在该文件声明。
- 测试:`on_activate_refreshes_the_session_list_of_that_project` 用 `offline_ctx`(指向不存在的 socket)并断言 `SessionsRefreshed(7, Err(_))`,迁移后仍成立(默认服务束由 `Client` 适配)。
- `R-CLIENT` 基线:`extensions/conversations.rs` 为 2(`spawn_refresh` 的参数类型与 `on_activate` 的 `ctx.io.client()`)。

## 3. 设计

### 3.1 `ConversationService`(放进 `dozer-services`,仍只依赖 `dozer-model`)

| 方法 | 返回 |
|---|---|
| `list_conversations_with_summaries(cwd: String, agent: Option<AgentKind>, limit: u32, offset: u32)` | `Vec<(ConversationSummary, Option<SessionSummaryPayload>)>` |

参数按值传,返回 `BoxFuture<'_, Result<_, ServiceError>>`。为避免 clippy `type_complexity`,返回值给一个公开别名 `ConversationRows`(同 `PostedMessages` 先例)。`ConversationSummary`、`SessionSummaryPayload`、`AgentKind` 均已在 `dozer-model`。`Services` 增加 `conversation: Arc<dyn ConversationService>`;`Client` 实现该 trait(调用同名 inherent 方法,`map_err(into_service_error)`)。目前只有 1 个方法,host 侧的会话相关调用以后随各自迁移加入同一 trait。

### 3.2 面板与 host

- `SessionsRefreshed` 的结果类型改为 `Result<Vec<SessionRow>, ServiceError>`(`Message` 已 `Debug + Clone`,`ServiceError` 满足)。不新增消息变体。
- `spawn_refresh(project_id, cwd, service: Arc<dyn ConversationService>, handle, emit)`;`on_activate` 用 `ctx.io.services().conversation.clone()`;`Workspace::spawn_conversations_refresh` 用 `services_for(&io.client).conversation`。
- `update` 的 `SessionsRefreshed` 分支:
  - `Ok(rows)` → `sessions = Some(rows)`(不变)。
  - `Err(e)` → 若 `sessions` 是 `None`(首次),置 `Some(Vec::new())`(保持现状,不卡加载中);否则**保留旧列表**。再按 `e.kind` 分档:`Unavailable` 只写 debug 日志;其余 `outbox.push_keyed(LOG, Error, "会话列表加载失败: {message}", "conversations-load")`,并用 `last_load_failure: Option<(String, Instant)>` 做 60 秒同文本去重(常量 `LOAD_FAILURE_TOAST_COOLDOWN`,与 Todo/Project 同值,各模块各一份,见 §6)。
- `WorkspaceState` 增加 `outbox: toast::Outbox`、`last_load_failure` 与 `take_outbox`;`App::drain_outboxes` 加一行排空。
- host 的 `SessionsRefreshed` 专属臂:`result` 类型随之改变,其后续的"刷新已打开评审视图"逻辑只依赖 `ws.conversations.sessions()`,不变。**不新增面板专属臂**(`R-HOST-PANEL-ARMS` 不上升)。

## 4. 步骤(每步先写失败测试;每步一个提交)

- **C0 基线:** 记录测试数、clippy 警告数、`R-CLIENT` 中 `conversations.rs`(2)、门禁输出。
- **C1 `ConversationService` trait:** `dozer-services` 契约测试(内存实现、`tokio::spawn` Send 检查、读失败可与空列表区分)。
- **C2 `Client` 实现:** 回放服务端适配测试(成功读且元组原样带回、`HangUp → Unavailable "daemon 断开"`、`ErrorReply → Rejected "daemon 错误: boom"`)。
- **C3 `Services.conversation` + 假实现:** `services_for` 与 `base_services()` 补字段;进程内 `FakeConversation`(记录调用、可预设行或失败)。
- **C4 面板与 host 切换:** 面板层测试(用 `FakeConversation`):成功时 `sessions` 映射正确;首次失败得到空列表;已有列表时失败保留旧列表;`Unavailable` 不弹 Toast;`Rejected` 弹一条且 60 秒内同文本不重复、换文本再弹;`spawn_refresh` 把 `cwd`、`agent: None`、`limit: 500`、`offset: 0` 原样传给服务。`on_activate` 的既有测试保持。探针 `host_site` 行号因 `drain_outboxes` 多一行整体 +1,只改行号。
- **C5 门禁与验收:** `R-CLIENT` 基线中 `conversations.rs` 清零;回填 `08-client-use-inventory.md`、`CLAUDE.md` 的 `dozer-services` 行(Usage、Todo、Project、GroupChat、Conversations 已迁)。

## 5. 验收标准(机器可验证)

1. `cargo tree -p dozer-services --edges normal` 仍只有 `dozer-model` 与 serde 系。
2. `extensions/conversations.rs` 不再含 `dozer_client` / `io.client()`;`R-CLIENT` 在该文件为 0,基线只降。
3. 刷新失败的四种情形(首次失败、已有列表时失败、`Unavailable`、`Rejected` 重复)有测试。
4. 全量测试数 ≥ 基线 + 新增;已知失败(`files::delete_confirm_spec_reflects_pending_target`、间歇的 `git_log::build_*`、`R-HOST-EXECUTORS 0->1`)不算回归也不顺手修。
5. `R-HOST-PANEL-ARMS` 不上升;clippy 警告数不高于基线;`cargo fmt --check`、`scripts/check-log-scope.sh` 通过;`model_golden` 仍通过。

## 6. 明确不做

- 不迁 host 侧会话调用(打开、翻页、总结、homespace、hook)。
- 不给 `SessionsRefreshed` 补代数保护(慢的旧结果可能覆盖新结果,同 Todo/Project 的既有问题,登记为后续项)。
- 不把冷却逻辑抽到 `toast::Outbox`(Todo、Project、Conversations 现在各一份,抽取另立项)。
- 不对齐 Usage 的失败口径;不改线上协议;不动 `PanelIo::client()`(`codehealth` 仍在用)。

## 7. 风险与已知问题

- **用户可见行为变化:** 已有列表时刷新失败不再清空(原来会清空)。这是修缺陷,用户已确认。
- **探针行号漂移:** `drain_outboxes` 多一行,`usage_probe/host.rs` 的 `host_site` 引用整体 +1,只改行号。
- **host 臂的类型改动:** `SessionsRefreshed` 的 `result` 类型变化会波及 `app/update.rs` 约 1514–1530 行的臂和 `workspace/state.rs` 的 emit 闭包;改动应仅限类型,不改逻辑。
- **WIP 文件:** 用户 WIP(`Cargo.lock`、`crates/dozer-app/Cargo.toml`、`Info.plist`)在 main 工作区未提交;在独立 worktree 实施,不提交 `Info.plist` 与版本号改动,`Cargo.lock` 剔除 bytegit/byteui 的 `source =` 行。

## 8. 分支与执行

规格分支 `spec/conversation-services`(worktree `../dozer-conversation-spec`);实施另开分支。执行计划由 writing-plans 产出,待本规格审阅通过后进行;合并进 main 前需询问用户。
