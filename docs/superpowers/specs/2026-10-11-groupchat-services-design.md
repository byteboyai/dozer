# GroupChat 面板迁移到服务 trait(第四个面板)

> 日期:2026-10-11。状态:待用户审阅。依据:`2026-10-09-dozer-services-x0-design.md`(接缝形状、`ServiceError`)、`2026-10-10-todo-services-design.md`、`2026-10-10-project-services-design.md`(先例)、`docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`。

## 1. 已对齐的理解

**目标(沿用前三个面板):** 群聊面板可进程内测试/复用——取数、写数只依赖小 trait,测试注入内存实现。
**范围:** 只迁 `extensions/group_chat/**` 里的 11 个服务调用;不碰其它面板与 host 侧调用。
**不变量:** 线上协议零变化;**用户可见行为零变化**——与 Todo/Project 不同,群聊面板不存在"吞错"问题(见 §2),所以本次是纯接缝迁移,不新增 `LoadFailed`、不改 Toast 口径。失败文本逐字不变(`"新建群聊失败: {e}"` 里的 `{e}` 仍是原文本)。
**与群聊相关的既有约束不受影响:** 群聊只讨论不分配任务;agent 发言走只读无头调用(这些在 dozerd 侧,与客户端接缝无关)。

## 2. 现状(读代码得到)

- 全部取数集中在 `group_chat/mod.rs` 的三个函数,都经 `PanelIo::spawn(|client, io| …)`:
  - `spawn_load_groups`:`list_groups` → `Message::GroupsLoaded(project_id, Result<_, String>)`;
  - `spawn_fetch_messages`:`list_group_messages(group_id, after_rev, POLL_LIMIT)` → `Message::Polled { result: Result<_, String> }`;
  - `spawn_command`:9 个写命令,失败统一 `fail(what, e)` → `Message::Failed(project_id, "{what}: {e}")`。
- 失败已可见:`GroupsLoaded(Err)` → Toast("加载群聊失败: …");`Polled(Err)` → 退避;`Failed` → Toast。所以无需分档/新消息。
- host 只经 `group_chat::run_effect(effect, project_id, &pio)`(`app/update.rs` 约 1932 行)调用,签名不变。
- `PanelIo::spawn` 目前的唯一生产用户就是群聊;`panel_host.rs` 里有它的测试(`io.spawn(|_client, io| …)`、`io.spawn(|client, io| …)`)。
- 现有群聊测试的 `offline_io` 用指向不存在 socket 的 `Client` 构造 `PanelIo`:`PanelIo::new` 默认的服务束由 `Client` 适配,所以这些测试在迁移后**不改也能通过**(服务照样失败于连不上 socket)。
- `R-CLIENT` 基线:`extensions/group_chat/mod.rs` 为 1(即测试里的 `dozer_client::Client::new`)。
- 返回值类型都已在 `dozer-model`:`GroupInfo`、`GroupMessageInfo`、`GroupCancelScope`、`AgentKind`、`TodoInfo`。

## 3. 设计

### 3.1 `GroupChatService`(放进 `dozer-services`,仍只依赖 `dozer-model`)

参数按值传,返回 `BoxFuture<'_, Result<T, ServiceError>>`。返回值保持 `Client` 原样(面板确实使用):

| 方法 | 返回 |
|---|---|
| `list_groups(project_id: i64)` | `Vec<GroupInfo>` |
| `list_group_messages(group_id: i64, after_rev: i64, limit: u32)` | `(Vec<GroupMessageInfo>, i64)` |
| `create_group(project_id: i64, topic: String)` | `GroupInfo` |
| `delete_group(group_id: i64)` | `()` |
| `add_group_member(group_id: i64, agent: AgentKind, handle: String, role_prompt: String)` | `GroupInfo` |
| `update_group_member(member_id: i64, handle: String, role_prompt: String)` | `GroupInfo` |
| `remove_group_member(member_id: i64)` | `GroupInfo` |
| `post_group_message(group_id: i64, text: String)` | `(GroupMessageInfo, Vec<GroupMessageInfo>, Vec<String>)` |
| `cancel_group(group_id: i64, scope: GroupCancelScope)` | `()` |
| `retry_group_message(message_id: i64)` | `GroupMessageInfo` |
| `push_group_message_to_todo(message_id: i64, text: String)` | `TodoInfo` |

`Services` 增加 `group_chat: Arc<dyn GroupChatService>`;`Client` 实现该 trait(调用同名 inherent 方法,`map_err(into_service_error)`),错误文本由 `roundtrip` 已有的 `ServiceError` 保证不变。

### 3.2 `PanelIo::spawn` 与面板

- `PanelIo::spawn` 的闭包签名从 `FnOnce(Client, PanelIo<M>) -> Fut` 改为 `FnOnce(Services, PanelIo<M>) -> Fut`(内部 `self.services.clone()`)。`PanelIo::client()` 保留(`conversations`、`codehealth` 仍在用)。
- `spawn_load_groups`/`spawn_fetch_messages`/`spawn_command` 改用 `services.group_chat`;`.map_err(|e| e.to_string())` → `.map_err(|e| e.message)`;`fail` 闭包的参数类型由 `anyhow::Error` 改为 `ServiceError`(`format!("{what}: {e}")` 不变,`Display` 即 message)。
- `Message`/`Effect`/`update`/`run_effect` 的签名与语义不变。

## 4. 步骤(每步先写失败测试;每步一个提交)

- **G0 基线:** 记录测试数、clippy 警告数、`R-CLIENT` 中 `group_chat/mod.rs`(1)、门禁输出。
- **G1 `GroupChatService` trait:** `dozer-services` 契约测试(内存实现、`tokio::spawn` Send 检查、读失败可与空列表区分)。
- **G2 `Client` 实现 `GroupChatService`:** 回放服务端适配测试(成功读、写返回值原样带回、`HangUp → Unavailable "daemon 断开"`、`ErrorReply → Rejected "daemon 错误: boom"`)。
- **G3 `Services.group_chat` + 假实现 + `PanelIo::spawn` 改签名:** `services_for` 与 `base_services()` 补字段;进程内 `FakeGroupChat`(记录调用、可预设成功值或失败);`panel_host.rs` 里 `spawn` 的既有测试跟着改闭包参数。
- **G4 面板切换:** 三个 spawn 函数改用服务。测试(用 `FakeGroupChat`):每个命令成功时发出的消息(`GroupCreated`/`GroupDeleted`/`GroupChanged`×3/`Posted`/`MessageUpdated`/`TodoPushed`,`Cancel` 成功不发消息);失败时 `Failed` 文本为 `"{前缀}: {服务文本}"` 逐字不变(11 个中的写命令各一个前缀);`spawn_fetch_messages` 把 `after_rev` 与 `POLL_LIMIT` 原样传给服务;`spawn_load_groups` 失败时 `GroupsLoaded(_, Err(text))` 的 text 逐字不变。
- **G5 门禁与验收:** `R-CLIENT` 基线中 `group_chat/mod.rs` 清零(`offline_io` 里的 `dozer_client::Client::new` 换成 `base_services()` 或假实现);回填 `08-client-use-inventory.md`、`CLAUDE.md` 的 `dozer-services` 行(Usage、Todo、Project、GroupChat 已迁)。

## 5. 验收标准(机器可验证)

1. `cargo tree -p dozer-services --edges normal` 仍只有 `dozer-model` 与 serde 系。
2. `extensions/group_chat/**` 不再含 `dozer_client` / `io.client()`;`R-CLIENT` 在该文件为 0,基线只降。
3. 11 个服务调用的成功/失败消息与文本逐字不变,有测试(G4)。
4. 全量测试数 ≥ 基线 + 新增;已知失败(`files::delete_confirm_spec_reflects_pending_target`、间歇的 `git_log::build_*`、`R-HOST-EXECUTORS 0->1`)不算回归也不顺手修。
5. `R-HOST-PANEL-ARMS` 不上升;clippy 警告数不高于基线;`cargo fmt --check`、`scripts/check-log-scope.sh` 通过;`model_golden` 仍通过。

## 6. 明确不做

- 不新增 `LoadFailed`/分档(群聊失败本来就可见);不改 `Polled` 退避、`GroupsLoaded` Toast 口径。
- 不改 `PanelIo::client()`;不迁其它面板或 host 侧 Todo/Conversations 调用。
- 不对齐 Usage 的失败口径;不改线上协议。

## 7. 风险与已知问题

- **`spawn_command` 是 9 分支 match:** 每个分支的失败前缀(`新建群聊失败`、`删除群聊失败`、`添加成员失败`、`修改成员失败`、`移除成员失败`、`发送失败`、`停止失败`、`重试失败`、`转为待办失败`)和成功消息都要保持一致——G4 逐分支写测试。
- **`PanelIo::spawn` 签名改动:** 影响 `panel_host.rs` 内的实现与其测试;`spawn` 的其它生产调用者已核实不存在。
- **探针行号:** 预计不碰 `app/update.rs`,探针 `host_site` 不应漂移;若漂移只改行号。
- **WIP 文件:** 用户 WIP 在 main 工作区未提交;在独立 worktree 实施,不提交 `Info.plist` 与版本号改动,`Cargo.lock` 剔除 bytegit/byteui 的 `source =` 行。

## 8. 分支与执行

规格分支 `spec/groupchat-services`(worktree `../dozer-groupchat-spec`);实施另开分支。执行计划由 writing-plans 产出,待本规格审阅通过后进行;合并进 main 前需询问用户。
