# GroupChat 面板迁移到服务 trait Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 群聊面板(`extensions/group_chat/**`)的 11 个服务调用改为经 `dozer-services` 的 `GroupChatService`;用户可见行为零变化。

**Architecture:** `dozer-services` 新增 `GroupChatService`(11 个方法,参数按值传,`BoxFuture` 返回)与 `Services.group_chat`;`Client` 实现它(复用 `into_service_error`);`PanelIo::spawn` 的闭包改拿 `Services`;`spawn_load_groups`/`spawn_fetch_messages`/`spawn_command` 改调服务。失败消息(`GroupsLoaded(Err)`、`Polled(Err)`、`Failed`)的文本逐字不变。

**Tech Stack:** Rust 2024 workspace、tokio、iced 0.14(只涉及面板 spawn 层)、Python 门禁脚本。

**Spec:** `docs/superpowers/specs/2026-10-11-groupchat-services-design.md`(先例:`2026-10-10-todo-services-design.md`、`2026-10-10-project-services-design.md`、`2026-10-09-dozer-services-x0-design.md`)

## Global Constraints

- `dozer-services` 只依赖 `dozer-model`;不加 `dozer-core`/`dozer-client`/`tokio` 到 `[dependencies]`。
- 线上协议零变化:`Request`/`Reply` 不动,`crates/dozer-core/tests/model_golden.rs` 不改且通过。
- 失败文本逐字不变:原 `e.to_string()` 等于 `ServiceError.message`(`roundtrip` 已把 `"daemon 断开"`、`"daemon 错误: …"` 装进 `ServiceError`,`Display` = message)。
- 面板日志来源名必须是 `group_chat`(已声明的 `LOG`);不新增裸 `tracing::`/`eprintln!`(`scripts/check-log-scope.sh`)。
- 群聊的既有约束不动:只讨论不分配任务;不新增 `LoadFailed`/分档;不改 `Polled` 退避与 `GroupsLoaded` Toast 口径。
- `PanelIo::client()` 保留(`conversations`、`codehealth` 仍在用);不迁其它面板;不改 `app/update.rs`(`run_effect(effect, project_id, &pio)` 签名不变),`R-HOST-PANEL-ARMS` 不上升。
- 在独立 worktree 实施;不提交用户 WIP(`Info.plist`、版本号相关改动);`Cargo.lock` 提交前剔除 bytegit/byteui 的 `source =` 行(若 `git status` 显示 `Cargo.lock` 只有这类噪声,直接 `git checkout Cargo.lock`)。
- main 上已知问题不修不算回归:`files::tests::delete_confirm_spec_reflects_pending_target` 失败、`git_log::tests::build_*` 间歇失败、门禁 `R-HOST-EXECUTORS 0 -> 1`。
- 提交信息末尾加 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## Review Focus

- 9 个写命令各自的失败前缀(`新建群聊失败`、`删除群聊失败`、`添加成员失败`、`修改成员失败`、`移除成员失败`、`发送失败`、`停止失败`、`重试失败`、`转为待办失败`)与成功时发出的消息,切换后逐字一致 → Task 4 测试(失败文本用**相等**断言,不用 `starts_with`)。
- `Cancel` 成功时**不发任何消息**(只有失败才发 `Failed`)→ Task 4 测试。
- `spawn_fetch_messages` 把 `after_rev` 与 `POLL_LIMIT`(200)原样传给服务 → Task 4 测试。
- `Post` 成功消息 `Posted` 三个字段(human / placeholders / unknown)原样带回 → Task 4 测试。
- `impl GroupChatService for Client` 内部必须调 inherent 方法而不是递归调 trait 方法;非 `()` 的返回值(元组、`GroupInfo` 等)要原样带回 → Task 2 适配测试。

---

### Task 1: `GroupChatService` trait(`dozer-services`)

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`

**Interfaces:**
- Consumes: `dozer_model::{AgentKind, GroupCancelScope, GroupInfo, GroupMessageInfo, TodoInfo}`(均已在 `dozer-model`)。
- Produces: `pub trait GroupChatService: Send + Sync`,全部返回 `BoxFuture<'_, Result<_, ServiceError>>`,参数按值传:
  - `list_groups(project_id: i64) -> Vec<GroupInfo>`
  - `list_group_messages(group_id: i64, after_rev: i64, limit: u32) -> (Vec<GroupMessageInfo>, i64)`
  - `create_group(project_id: i64, topic: String) -> GroupInfo`
  - `delete_group(group_id: i64) -> ()`
  - `add_group_member(group_id: i64, agent: AgentKind, handle: String, role_prompt: String) -> GroupInfo`
  - `update_group_member(member_id: i64, handle: String, role_prompt: String) -> GroupInfo`
  - `remove_group_member(member_id: i64) -> GroupInfo`
  - `post_group_message(group_id: i64, text: String) -> (GroupMessageInfo, Vec<GroupMessageInfo>, Vec<String>)`
  - `cancel_group(group_id: i64, scope: GroupCancelScope) -> ()`
  - `retry_group_message(message_id: i64) -> GroupMessageInfo`
  - `push_group_message_to_todo(message_id: i64, text: String) -> TodoInfo`

- [ ] **Step 0: 记录基线(只测量,写进 ledger;同步执行,跑完再进 Step 1,避免并发编译污染)**

```bash
W=$(ls -d .superpowers/sdd/*/ | head -1)
grep -c 'dozer_client' crates/dozer-app/src/extensions/group_chat/mod.rs
cargo test --workspace --no-fail-fast > ${W}baseline-tests.txt 2>&1; grep -E '^test result' ${W}baseline-tests.txt | awk '{s+=$4;f+=$6} END{print "passed",s,"failed",f}'; grep -E '^test .* FAILED' ${W}baseline-tests.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py; echo exit=$?
```
Expected: 失败仅已知的;门禁仅 `R-HOST-EXECUTORS 0 -> 1`;`group_chat/mod.rs` 的 `dozer_client` 引用数为 1;把数字写进 ledger。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-services/src/lib.rs` 现有 `#[cfg(test)] mod tests` 末尾(最后一个测试之后、模块结尾 `}` 之前)追加(`unit()` 是 Todo 契约测试里已有的辅助函数):

```rust
    // ---- GroupChatService 契约 ----

    use dozer_model::{GroupCancelScope, GroupInfo, GroupMessageInfo};

    struct MemGroupChat(Result<Vec<GroupInfo>, ServiceError>);

    fn proto(m: &'static str) -> ServiceError {
        ServiceError::new(ServiceErrorKind::Protocol, m)
    }

    impl GroupChatService for MemGroupChat {
        fn list_groups(&self, _p: i64) -> BoxFuture<'_, Result<Vec<GroupInfo>, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
        fn list_group_messages(&self, _g: i64, _a: i64, _l: u32) -> BoxFuture<'_, Result<(Vec<GroupMessageInfo>, i64), ServiceError>> {
            Box::pin(async { Ok((vec![], 0)) })
        }
        fn create_group(&self, _p: i64, _t: String) -> BoxFuture<'_, Result<GroupInfo, ServiceError>> {
            Box::pin(async { Err(proto("mem")) })
        }
        fn delete_group(&self, _g: i64) -> BoxFuture<'_, Result<(), ServiceError>> { unit() }
        fn add_group_member(&self, _g: i64, _a: AgentKind, _h: String, _r: String) -> BoxFuture<'_, Result<GroupInfo, ServiceError>> {
            Box::pin(async { Err(proto("mem")) })
        }
        fn update_group_member(&self, _m: i64, _h: String, _r: String) -> BoxFuture<'_, Result<GroupInfo, ServiceError>> {
            Box::pin(async { Err(proto("mem")) })
        }
        fn remove_group_member(&self, _m: i64) -> BoxFuture<'_, Result<GroupInfo, ServiceError>> {
            Box::pin(async { Err(proto("mem")) })
        }
        fn post_group_message(&self, _g: i64, _t: String) -> BoxFuture<'_, Result<(GroupMessageInfo, Vec<GroupMessageInfo>, Vec<String>), ServiceError>> {
            Box::pin(async { Err(proto("mem")) })
        }
        fn cancel_group(&self, _g: i64, _s: GroupCancelScope) -> BoxFuture<'_, Result<(), ServiceError>> { unit() }
        fn retry_group_message(&self, _m: i64) -> BoxFuture<'_, Result<GroupMessageInfo, ServiceError>> {
            Box::pin(async { Err(proto("mem")) })
        }
        fn push_group_message_to_todo(&self, _m: i64, _t: String) -> BoxFuture<'_, Result<TodoInfo, ServiceError>> {
            Box::pin(async { Err(proto("mem")) })
        }
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn GroupChatService>` 的 async 块必须满足。
    #[test]
    fn dyn_group_chat_futures_run_inside_tokio_spawn() {
        let svc: Arc<dyn GroupChatService> = Arc::new(MemGroupChat(Ok(vec![])));
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let (groups, cancel) = rt.block_on(async move {
            tokio::spawn(async move {
                let groups = svc.list_groups(7).await;
                let cancel = svc.cancel_group(5, GroupCancelScope::Turn).await;
                (groups, cancel)
            })
            .await
            .unwrap()
        });
        assert_eq!(groups, Ok(vec![]));
        assert_eq!(cancel, Ok(()));
    }

    #[test]
    fn group_read_failure_is_distinguishable_from_an_empty_list() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let empty = MemGroupChat(Ok(vec![]));
        let down = MemGroupChat(Err(ServiceError::new(ServiceErrorKind::Unavailable, "x")));
        assert!(rt.block_on(empty.list_groups(1)).is_ok());
        assert_eq!(
            rt.block_on(down.list_groups(1)).unwrap_err().kind,
            ServiceErrorKind::Unavailable
        );
    }
```
(`AgentKind`、`TodoInfo` 已在该测试模块 import 过;若没有,按编译器提示补 `use dozer_model::{AgentKind, TodoInfo};`。)

Run: `cargo test -p dozer-services 2>&1 | tail -8` → Expected: 编译失败 `cannot find trait GroupChatService`(RED)。

- [ ] **Step 2: 实现**

顶部 `use dozer_model::{...}` 合并追加 `GroupCancelScope, GroupInfo, GroupMessageInfo`(`AgentKind`、`TodoInfo` 已在)。在 `ProjectService` 之后、`Services` 之前加:

```rust
/// 群聊面板的读写。参数按值传。返回值保持 `Client` 原样(面板确实使用)。
pub trait GroupChatService: Send + Sync {
    fn list_groups(&self, project_id: i64) -> BoxFuture<'_, Result<Vec<GroupInfo>, ServiceError>>;
    fn list_group_messages(
        &self,
        group_id: i64,
        after_rev: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<(Vec<GroupMessageInfo>, i64), ServiceError>>;
    fn create_group(&self, project_id: i64, topic: String) -> BoxFuture<'_, Result<GroupInfo, ServiceError>>;
    fn delete_group(&self, group_id: i64) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn add_group_member(
        &self,
        group_id: i64,
        agent: AgentKind,
        handle: String,
        role_prompt: String,
    ) -> BoxFuture<'_, Result<GroupInfo, ServiceError>>;
    fn update_group_member(
        &self,
        member_id: i64,
        handle: String,
        role_prompt: String,
    ) -> BoxFuture<'_, Result<GroupInfo, ServiceError>>;
    fn remove_group_member(&self, member_id: i64) -> BoxFuture<'_, Result<GroupInfo, ServiceError>>;
    fn post_group_message(
        &self,
        group_id: i64,
        text: String,
    ) -> BoxFuture<'_, Result<(GroupMessageInfo, Vec<GroupMessageInfo>, Vec<String>), ServiceError>>;
    fn cancel_group(&self, group_id: i64, scope: GroupCancelScope) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn retry_group_message(&self, message_id: i64) -> BoxFuture<'_, Result<GroupMessageInfo, ServiceError>>;
    fn push_group_message_to_todo(
        &self,
        message_id: i64,
        text: String,
    ) -> BoxFuture<'_, Result<TodoInfo, ServiceError>>;
}
```
**不要**在本任务给 `Services` 加字段(Task 3 做,否则 `dozer-app` 编译失败)。

Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|error'` → Expected: 全绿(原 8 个 + 新 2 个 = 10 passed)。

- [ ] **Step 3: 依赖验收与提交**

```bash
cargo tree -p dozer-services --edges normal | grep -vE 'dozer-model|serde|proc-macro2|quote|syn|unicode-ident|dozer-services' | head   # 期望无输出
cargo fmt -p dozer-services && cargo clippy -p dozer-services --all-targets 2>&1 | grep -c '^warning'   # 期望 0
git add crates/dozer-services && git commit -m "feat(services): GroupChat S1 GroupChatService trait(11 方法)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `Client` 实现 `GroupChatService`

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`(impl + 测试模块 `group_chat_service_tests`)

**Interfaces:**
- Consumes: Task 1;`into_service_error`、`BoxFuture`、`ServiceError`;Todo 任务里已 `pub(super)` 的 `todo_service_tests::serve_once`。
- Produces: `impl GroupChatService for Client`(同名 inherent 方法不改)。

- [ ] **Step 1: 写失败测试**

在文件末尾追加(`serve_once(tag, Option<Reply>)` 复用 Todo 测试模块里的版本,已为 `pub(super)`):

```rust
#[cfg(test)]
mod group_chat_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{GroupChatService, ServiceErrorKind};

    #[tokio::test]
    async fn list_groups_comes_back_as_ok() {
        let sock = serve_once("gc-list", Some(Reply::Groups { groups: vec![] }));
        let got = GroupChatService::list_groups(&Client::new(sock.clone()), 7).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn a_unit_write_returns_unit_on_success() {
        let sock = serve_once("gc-delete", Some(Reply::Ok));
        let got = GroupChatService::delete_group(&Client::new(sock.clone()), 3).await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("gc-hangup", None);
        let e = GroupChatService::list_groups(&Client::new(sock.clone()), 7).await.unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Unavailable, "daemon 断开"));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once("gc-reject", Some(Reply::Error { message: "boom".into() }));
        let e = GroupChatService::delete_group(&Client::new(sock.clone()), 3).await.unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Rejected, "daemon 错误: boom"));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn a_tuple_result_is_carried_back_unchanged() {
        let sock = serve_once(
            "gc-msgs",
            Some(Reply::GroupMessages { messages: vec![], latest_rev: 42 }),
        );
        let got = GroupChatService::list_group_messages(&Client::new(sock.clone()), 5, 3, 200).await;
        assert_eq!(got, Ok((vec![], 42)));
        let _ = std::fs::remove_file(&sock);
    }
}
```
Run: `cargo test -p dozer-client group_chat_service_tests 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`Client: GroupChatService` 未实现)(RED)。

- [ ] **Step 2: 实现**

顶部 `use dozer_services::{...}` 追加 `GroupChatService`。在 `impl ProjectService for Client` 之后追加。**每个方法都用 `Client::method(self, ..)` 路径调 inherent 方法**:

```rust
impl GroupChatService for Client {
    fn list_groups(&self, project_id: i64) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::GroupInfo>, ServiceError>> {
        Box::pin(async move { Client::list_groups(self, project_id).await.map_err(into_service_error) })
    }
    fn list_group_messages(
        &self,
        group_id: i64,
        after_rev: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<(Vec<dozer_core::protocol::GroupMessageInfo>, i64), ServiceError>> {
        Box::pin(async move {
            Client::list_group_messages(self, group_id, after_rev, limit)
                .await
                .map_err(into_service_error)
        })
    }
    fn create_group(&self, project_id: i64, topic: String) -> BoxFuture<'_, Result<dozer_core::protocol::GroupInfo, ServiceError>> {
        Box::pin(async move { Client::create_group(self, project_id, &topic).await.map_err(into_service_error) })
    }
    fn delete_group(&self, group_id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move { Client::delete_group(self, group_id).await.map_err(into_service_error) })
    }
    fn add_group_member(
        &self,
        group_id: i64,
        agent: dozer_core::protocol::AgentKind,
        handle: String,
        role_prompt: String,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::GroupInfo, ServiceError>> {
        Box::pin(async move {
            Client::add_group_member(self, group_id, agent, &handle, &role_prompt)
                .await
                .map_err(into_service_error)
        })
    }
    fn update_group_member(
        &self,
        member_id: i64,
        handle: String,
        role_prompt: String,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::GroupInfo, ServiceError>> {
        Box::pin(async move {
            Client::update_group_member(self, member_id, &handle, &role_prompt)
                .await
                .map_err(into_service_error)
        })
    }
    fn remove_group_member(&self, member_id: i64) -> BoxFuture<'_, Result<dozer_core::protocol::GroupInfo, ServiceError>> {
        Box::pin(async move { Client::remove_group_member(self, member_id).await.map_err(into_service_error) })
    }
    fn post_group_message(
        &self,
        group_id: i64,
        text: String,
    ) -> BoxFuture<'_, Result<(dozer_core::protocol::GroupMessageInfo, Vec<dozer_core::protocol::GroupMessageInfo>, Vec<String>), ServiceError>> {
        Box::pin(async move { Client::post_group_message(self, group_id, &text).await.map_err(into_service_error) })
    }
    fn cancel_group(&self, group_id: i64, scope: dozer_core::protocol::GroupCancelScope) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move { Client::cancel_group(self, group_id, scope).await.map_err(into_service_error) })
    }
    fn retry_group_message(&self, message_id: i64) -> BoxFuture<'_, Result<dozer_core::protocol::GroupMessageInfo, ServiceError>> {
        Box::pin(async move { Client::retry_group_message(self, message_id).await.map_err(into_service_error) })
    }
    fn push_group_message_to_todo(
        &self,
        message_id: i64,
        text: String,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::TodoInfo, ServiceError>> {
        Box::pin(async move {
            Client::push_group_message_to_todo(self, message_id, &text)
                .await
                .map_err(into_service_error)
        })
    }
}
```
类型路径写法以文件里已有 import 为准(例如已 `use dozer_core::protocol::{GroupInfo, …}` 则去掉前缀),`dozer_core::protocol::X` 与 `dozer_model::X` 是同一类型的再导出。

Run: `cargo test -p dozer-client 2>&1 | grep -E 'test result|FAILED|^error'` → Expected: 原有测试 + 5 个新测试全绿。

- [ ] **Step 3: 提交**

```bash
cargo fmt -p dozer-client
git checkout Cargo.lock 2>/dev/null
git add crates/dozer-client && git status --short
git commit -m "feat(client): GroupChat S2 Client 实现 GroupChatService

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: `Services.group_chat` + 默认适配 + `FakeGroupChat`

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`(`Services` 加字段;测试里的构造)
- Modify: `crates/dozer-app/src/panel_host.rs`(`services_for`)
- Modify: `crates/dozer-app/src/service_fakes.rs`(新增 `FakeGroupChat` 与辅助;`base_services()` 补字段)

**Interfaces:**
- Consumes: Task 1/2。
- Produces:
  - `Services { usage, todo, project, memory, group_chat: Arc<dyn GroupChatService> }`;`services_for(&Client)` 也适配 `group_chat`。
  - `service_fakes::group_info(id: i64, topic: &str) -> GroupInfo`(`project_id: 1`、`created_ms: 0`、`members: vec![]`);`service_fakes::group_message(id: i64, group_id: i64, rev: i64) -> GroupMessageInfo`(`seq: rev`、`author: Human`、`text: format!("m{id}")`、`mentions: vec![]`、`status: None`、`duration_ms: None`、`created_ms: 0`、`todo_id: None`);`service_fakes::todo_info(id: i64) -> TodoInfo`(字段取固定值:`project_id: 1`、`text: "t"`、`done: false`、`paused: false`、`rank: 0`、`created_ms: 0`、`completed_at_ms: None`、`plan_date: None`、`dispatch_session_id: None`、`dispatch_at_ms: None`、`category_id: None`、`assigned_agent: None`)。
  - `FakeGroupChat`(`Clone`,克隆共享状态,`Default` = 读写都成功、空):`.with_groups(Vec<GroupInfo>)`、`.with_messages(Vec<GroupMessageInfo>, latest_rev: i64)`、`FakeGroupChat::failing_reads(kind, message)`、`FakeGroupChat::failing_writes(kind, message)`、`calls() -> Vec<String>`、`into_services()`(`Services { group_chat: Arc::new(self), ..base_services() }`)。写成功时的返回值:`create_group` → `group_info(900, &topic)`;`add/update/remove_group_member` → `group_info(1, "g")`;`post_group_message` → `(group_message(901, group_id, 1), vec![group_message(902, group_id, 2)], vec!["unknown".to_string()])`;`retry_group_message` → `group_message(message_id, 1, 1)`;`push_group_message_to_todo` → `todo_info(77)`;`delete_group`/`cancel_group` → `()`。调用记录格式:`"list_groups(7)"`、`"list_group_messages(5,3,200)"`、`"create_group(7,主题)"`、`"delete_group(5)"`、`"add_group_member(5,Claude,h)"`(agent 用 `{:?}`)、`"update_group_member(1,h)"`、`"remove_group_member(1)"`、`"post_group_message(5,hi)"`、`"cancel_group(5,Turn)"`(scope 用 `{:?}`)、`"retry_group_message(9)"`、`"push_group_message_to_todo(9,x)"`。

- [ ] **Step 1: 写失败测试**

(a) `panel_host.rs` 测试模块(`default_services_route_project_and_memory_…` 旁)追加:
```rust
    #[test]
    fn default_services_route_group_chat_through_the_client_adapter() {
        let rt = test_runtime();
        let (io, _rx) = recording_io(&rt);
        // recording_io 的 Client 指向不存在的 socket → Unavailable
        let e = rt.block_on(io.services().group_chat.list_groups(1)).unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
    }
```
(b) `dozer-services` 的 `services_is_cloneable_and_shares_the_service` 测试同时构造 `group_chat: Arc::new(MemGroupChat(Ok(vec![])))` 并追加 `Arc::ptr_eq(&s.group_chat, &t.group_chat)`。

Run: `cargo test -p dozer-services 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`Services` 没有字段 `group_chat`)(RED)。

- [ ] **Step 2: 实现**

1. `Services` 加 `pub group_chat: Arc<dyn GroupChatService>,`;`services_for` 加 `group_chat: Arc::new(client.clone()),`。
2. `service_fakes.rs`:实现 `FakeGroupChat` 与三个辅助函数,**模式同已有的 `FakeMemory`/`FakeProject`**(`Arc<Mutex<State>>`;读返回预设,写按 `write_error` 决定成功值或错误;每次调用 `calls.push(..)`)。`base_services()` 补 `group_chat: Arc::new(FakeGroupChat::default())`。`GroupInfo`/`GroupMessageInfo`/`GroupAuthor`/`TodoInfo` 字段以 `crates/dozer-model/src/{group_chat,todo}.rs` 为准(上面列的是已核对的字段)。
3. 新增的构造函数在 Task 4 之前未被使用时,加 `#[allow(dead_code)]`(Task 4 去掉)。

Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|^error'`;`cargo test -p dozer-app default_services_route_group_chat 2>&1 | grep -E 'test result|^error'` → Expected: 通过。

- [ ] **Step 3: 全 workspace 编译与提交**

```bash
cargo build --workspace 2>&1 | grep -E '^error'          # 期望无输出
cargo test -p dozer-app -- panel_host usage_probe 2>&1 | grep -E 'test result|FAILED'
cargo clippy --all-targets 2>&1 | grep -c '^warning'      # 期望 ≤ 基线
cargo fmt -p dozer-app -p dozer-services
git checkout Cargo.lock 2>/dev/null
git add crates/dozer-services crates/dozer-app/src && git status --short
git commit -m "feat(host): GroupChat S3 Services.group_chat + FakeGroupChat

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: `PanelIo::spawn` 改拿 `Services`,群聊面板切换

> 说明(Ruling,计划阶段):`PanelIo::spawn` 的闭包签名与群聊的三个 spawn 函数必须在同一个提交里改——群聊是 `spawn` 的唯一生产用户,签名一变,闭包里的 `client.xxx` 立刻编译失败。

**Files:**
- Modify: `crates/dozer-app/src/panel_host.rs`(`spawn` 签名与文档注释;`spawn_hands_the_task_the_hosts_client` 测试)
- Modify: `crates/dozer-app/src/extensions/group_chat/mod.rs`(三个 spawn 函数;测试)
- Modify: `crates/dozer-app/src/service_fakes.rs`(去掉临时 `#[allow(dead_code)]`)

**Interfaces:**
- Consumes: Task 3 的 `Services`、`FakeGroupChat`、`group_info`/`group_message`/`todo_info`。
- Produces:
  - `PanelIo::spawn<F, Fut>(&self, task: F) where F: FnOnce(Services, PanelIo<M>) -> Fut + Send + 'static, Fut: Future<Output = ()> + Send + 'static`
  - `spawn_load_groups`、`spawn_fetch_messages`、`spawn_command`、`run_effect`、`select_group`、`poll_if_due` 的**公开签名不变**。

- [ ] **Step 1: 写失败测试**

(a) `panel_host.rs`:把 `spawn_hands_the_task_the_hosts_client` 改成
```rust
    #[test]
    fn spawn_hands_the_task_the_hosts_services() {
        let rt = test_runtime();
        let (io, rx) = recording_io(&rt);
        io.spawn(|services, io| async move {
            // 指向不存在 socket 的 Client 适配出的服务:真的发请求必然失败——证明拿到的是 host 的服务束。
            let failed = services.group_chat.list_groups(1).await.is_err();
            io.emit(u32::from(failed));
        });
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(), 1);
    }
```
并把同模块里其它 `io.spawn(|_client, io| …)` / `io.spawn(|client, io| …)` 的闭包参数名改为 `_services`(参数未被使用的那些)。

(b) `group_chat/mod.rs` 的 `mod tests`:先把 `offline_io` 改为不出现 `dozer_client`(`R-CLIENT` 清零要求):
```rust
    fn offline_io(rt: &tokio::runtime::Runtime) -> (PanelIo<Message>, Receiver<Message>) {
        // 默认服务束由指向不存在 socket 的 Client 适配:所有请求快速失败,语义同迁移前。
        let (ctx, rx) = crate::panel_host::testing::offline_ctx::<Message>(rt, 7, None);
        (ctx.io, rx)
    }

    /// 用进程内 `FakeGroupChat` 构造 io(不起任何 socket)。
    fn fake_io(
        rt: &tokio::runtime::Runtime,
        fake: &FakeGroupChat,
    ) -> (PanelIo<Message>, Receiver<Message>) {
        let (ctx, rx) = crate::panel_host::testing::offline_ctx::<Message>(rt, 7, None);
        (ctx.io.with_services(fake.clone().into_services()), rx)
    }
```
(`use crate::service_fakes::FakeGroupChat; use dozer_services::ServiceErrorKind;` 加到测试模块 import;原测试里用 `mpsc`/`channel`/`Arc`/`Mutex` 的 import 若因 `offline_io` 不再用而变成未使用,按编译器提示删。)然后追加新测试:

```rust
    #[test]
    fn successful_commands_emit_their_messages() {
        let rt = runtime();
        let fake = FakeGroupChat::default();
        let (io, rx) = fake_io(&rt, &fake);
        spawn_command(7, Command::CreateGroup { topic: "主题".into() }, &io);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::GroupCreated(7, g) => assert_eq!((g.id, g.topic.as_str()), (900, "主题")),
            other => panic!("unexpected: {other:?}"),
        }
        spawn_command(7, Command::DeleteGroup { group_id: 5 }, &io);
        assert!(matches!(rx.recv_timeout(TIMEOUT).unwrap(), Message::GroupDeleted(7, 5)));
        for cmd in [
            Command::AddMember { group_id: 5, agent: dozer_model::AgentKind::Claude, handle: "h".into(), role_prompt: String::new() },
            Command::UpdateMember { member_id: 1, handle: "h".into(), role_prompt: String::new() },
            Command::RemoveMember { member_id: 1 },
        ] {
            spawn_command(7, cmd.clone(), &io);
            assert!(matches!(rx.recv_timeout(TIMEOUT).unwrap(), Message::GroupChanged(7, _)), "{cmd:?}");
        }
        spawn_command(7, Command::Post { group_id: 5, text: "hi".into() }, &io);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Posted { project_id: 7, group_id: 5, human, placeholders, unknown } => {
                assert_eq!(human.id, 901);
                assert_eq!(placeholders.len(), 1);
                assert_eq!(unknown, vec!["unknown".to_string()]);
            }
            other => panic!("unexpected: {other:?}"),
        }
        spawn_command(7, Command::Retry { message_id: 9 }, &io);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::MessageUpdated(7, m) => assert_eq!(m.id, 9),
            other => panic!("unexpected: {other:?}"),
        }
        spawn_command(7, Command::PushTodo { group_id: 5, message_id: 9, text: "x".into() }, &io);
        assert!(matches!(
            rx.recv_timeout(TIMEOUT).unwrap(),
            Message::TodoPushed { project_id: 7, group_id: 5, message_id: 9, todo_id: 77 }
        ));
    }

    #[test]
    fn a_successful_cancel_emits_nothing() {
        let rt = runtime();
        let fake = FakeGroupChat::default();
        let (io, rx) = fake_io(&rt, &fake);
        spawn_command(7, Command::Cancel { group_id: 5, scope: dozer_model::GroupCancelScope::Turn }, &io);
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "成功的停止不发任何消息");
        assert_eq!(fake.calls(), vec!["cancel_group(5,Turn)".to_string()]);
    }

    #[test]
    fn command_failure_text_is_prefix_plus_the_service_message_unchanged() {
        let cases: Vec<(Command, &str)> = vec![
            (Command::CreateGroup { topic: "t".into() }, "新建群聊失败"),
            (Command::DeleteGroup { group_id: 5 }, "删除群聊失败"),
            (Command::AddMember { group_id: 5, agent: dozer_model::AgentKind::Claude, handle: "h".into(), role_prompt: String::new() }, "添加成员失败"),
            (Command::UpdateMember { member_id: 1, handle: "h".into(), role_prompt: String::new() }, "修改成员失败"),
            (Command::RemoveMember { member_id: 1 }, "移除成员失败"),
            (Command::Post { group_id: 5, text: "hi".into() }, "发送失败"),
            (Command::Cancel { group_id: 5, scope: dozer_model::GroupCancelScope::Turn }, "停止失败"),
            (Command::Retry { message_id: 9 }, "重试失败"),
            (Command::PushTodo { group_id: 5, message_id: 9, text: "x".into() }, "转为待办失败"),
        ];
        let rt = runtime();
        for (cmd, wording) in cases {
            let fake = FakeGroupChat::failing_writes(ServiceErrorKind::Rejected, "daemon 错误: boom");
            let (io, rx) = fake_io(&rt, &fake);
            spawn_command(7, cmd.clone(), &io);
            match rx.recv_timeout(TIMEOUT).unwrap() {
                Message::Failed(7, text) => assert_eq!(text, format!("{wording}: daemon 错误: boom"), "{cmd:?}"),
                other => panic!("{cmd:?}: unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn fetch_messages_passes_after_rev_and_the_poll_limit_to_the_service() {
        let rt = runtime();
        let fake = FakeGroupChat::default().with_messages(vec![], 42);
        let (io, rx) = fake_io(&rt, &fake);
        spawn_fetch_messages(7, 5, 3, &io);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Polled { project_id: 7, group_id: 5, result: Ok((msgs, latest)) } => {
                assert!(msgs.is_empty());
                assert_eq!(latest, 42);
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(fake.calls(), vec![format!("list_group_messages(5,3,{POLL_LIMIT})")]);
    }

    #[test]
    fn load_groups_failure_text_is_the_service_message_unchanged() {
        let rt = runtime();
        let fake = FakeGroupChat::failing_reads(ServiceErrorKind::Unavailable, "daemon 断开");
        let (io, rx) = fake_io(&rt, &fake);
        spawn_load_groups(7, &io);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::GroupsLoaded(7, Err(text)) => assert_eq!(text, "daemon 断开"),
            other => panic!("unexpected: {other:?}"),
        }
    }
```
Run: `cargo test -p dozer-app extensions::group_chat panel_host 2>&1 | grep -E '^error' | head -6` → Expected: 编译失败(`spawn` 闭包类型不符、`FakeGroupChat` 未 import 等)(RED)。

- [ ] **Step 2: 实现**

`panel_host.rs`:
```rust
    /// 在 host 的 runtime 上跑一个后台任务。任务拿到 host 的服务束 `Services` 和一份 `PanelIo` 副本,
    /// 想发多少条消息(包括零条:只在失败时才发的命令)由任务自己决定。
    pub(crate) fn spawn<F, Fut>(&self, task: F)
    where
        F: FnOnce(Services, PanelIo<M>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let services = self.services.clone();
        let io = self.clone();
        self.handle.spawn(async move { task(services, io).await });
    }
```
`extensions/group_chat/mod.rs`(顶部 `use dozer_services::ServiceError;`):
```rust
pub fn spawn_load_groups(project_id: i64, io: &PanelIo<Message>) {
    io.spawn(move |services, io| async move {
        let result = services
            .group_chat
            .list_groups(project_id)
            .await
            .map_err(|e| e.message);
        io.emit(Message::GroupsLoaded(project_id, result));
    });
}

pub fn spawn_fetch_messages(project_id: i64, group_id: i64, after_rev: i64, io: &PanelIo<Message>) {
    io.spawn(move |services, io| async move {
        let result = services
            .group_chat
            .list_group_messages(group_id, after_rev, POLL_LIMIT)
            .await
            .map_err(|e| e.message);
        io.emit(Message::Polled { project_id, group_id, result });
    });
}
```
`spawn_command`:`io.spawn(move |services, io| async move {`;`let chat = services.group_chat;`;`fail` 闭包改为 `|what: &str, e: ServiceError| Message::Failed(project_id, format!("{what}: {e}"))`;各分支调用改为(参数按值,去掉 `&`):
- `chat.create_group(project_id, topic)`、`chat.delete_group(group_id)`、`chat.add_group_member(group_id, agent, handle, role_prompt)`、`chat.update_group_member(member_id, handle, role_prompt)`、`chat.remove_group_member(member_id)`、`chat.post_group_message(group_id, text)`、`chat.cancel_group(group_id, scope)`、`chat.retry_group_message(message_id)`、`chat.push_group_message_to_todo(message_id, text)`;
- 成功分支的消息构造、`Command::SelectGroup { .. } | Command::OpenTodo { .. } => {}` 保持不变。

去掉 `service_fakes.rs` 里 Task 3 临时加的 `#[allow(dead_code)]`。

- [ ] **Step 3: 运行面板与 host 测试**

```bash
cargo test -p dozer-app extensions::group_chat 2>&1 | grep -E 'test result|FAILED|^error'
cargo test -p dozer-app panel_host 2>&1 | grep -E 'test result|FAILED|^error'
```
Expected: 全绿(含 5 个新测试;原 `offline_io` 测试继续通过,因为默认服务束仍由 dead-socket `Client` 适配)。

- [ ] **Step 4: 探针守卫、全量、门禁、提交**

```bash
cargo test -p dozer-app usage_probe::host 2>&1 | grep -E 'test result|FAILED|实际是'   # 本任务不碰 app/update.rs,不应漂移;若漂移只改行号
W=$(ls -d .superpowers/sdd/*/ | head -1)
cargo test --workspace --no-fail-fast > ${W}t4.txt 2>&1; grep -E '^test result' ${W}t4.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep -E '^test .* FAILED' ${W}t4.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py        # 只许剩 R-HOST-EXECUTORS 0 -> 1;R-HOST-PANEL-ARMS 不得上升
bash scripts/check-log-scope.sh | tail -1
cargo fmt -p dozer-app && cargo fmt --check && echo fmt-ok
```
Expected: 失败仅已知的;clippy 警告数 ≤ 基线;门禁无新增回退;log scope ok。
```bash
git checkout Cargo.lock 2>/dev/null; git add crates/dozer-app/src && git status --short
git commit -m "feat(group_chat): GroupChat S4 PanelIo::spawn 改拿 Services,群聊面板改用 GroupChatService

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `R-CLIENT` 基线下降、验收与文档回填

**Files:**
- Modify: `scripts/audit/panel-boundary.baseline.json`(仅 `extensions/group_chat/mod.rs` 的 `R-CLIENT` 条目)
- Modify: `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`、`CLAUDE.md`

**Interfaces:**
- Produces: `extensions/group_chat/mod.rs` 不再出现在 `R-CLIENT` 基线里。

- [ ] **Step 1: 确认门禁现状并下调基线**

```bash
grep -rn 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/group_chat/ | head -3   # 期望无输出
python3 scripts/audit/check_panel_boundary.py   # 此时 group_chat 的 R-CLIENT 实测应为 0(低于基线),门禁只显示既有 R-HOST-EXECUTORS
python3 - <<'EOF'
import json
p = "scripts/audit/panel-boundary.baseline.json"
base = json.load(open(p, encoding="utf-8"))
for rel in [r for r in list(base) if r.startswith("extensions/group_chat")]:
    base[rel].pop("R-CLIENT", None)
    if not base[rel]:
        del base[rel]
json.dump(base, open(p, "w", encoding="utf-8"), ensure_ascii=False, indent=1, sort_keys=True)
open(p, "a").write("\n")
EOF
git diff --stat scripts/audit/panel-boundary.baseline.json
python3 scripts/audit/check_panel_boundary.py; echo exit=$?
python3 scripts/audit/test_check_panel_boundary.py 2>&1 | tail -1
```
Expected: 基线 diff 只含 `extensions/group_chat/mod.rs` 条目的删除;门禁仍只报 `R-HOST-EXECUTORS 0 -> 1`。

- [ ] **Step 2: 验收清单(逐条跑、读输出)**

```bash
cargo tree -p dozer-services --edges normal                    # 1: 仅 dozer-model + serde 系
python3 scripts/audit/unit_edges.py ext:group_chat             # 2: 无 dozer_client
cargo test -p dozer-app extensions::group_chat 2>&1 | grep -E 'test result|FAILED'   # 3
cargo test -p dozer-core --test model_golden 2>&1 | grep 'test result'                # 5
W=$(ls -d .superpowers/sdd/*/ | head -1)
cargo test --workspace --no-fail-fast > ${W}final.txt 2>&1; grep -E '^test result' ${W}final.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep -E '^test .* FAILED' ${W}final.txt   # 4
cargo clippy --all-targets 2>&1 | grep -c '^warning'; cargo fmt --check && echo fmt-ok
bash scripts/check-log-scope.sh && echo log-scope-ok
git status --short                                             # 无 Info.plist、无无关 Cargo 改动
```
任一不满足即不收尾,回到对应任务修。

- [ ] **Step 3: 文档回填并提交**

- `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md` 末尾追加一节"GroupChat 迁移(2026-10-11)":`extensions/group_chat/**` 经 `GroupChatService`(`dozer-services`)取数与写入;`R-CLIENT` 基线中 `group_chat/mod.rs` 已清零;用户可见行为零变化(群聊失败本来就可见,不新增 `LoadFailed`);`PanelIo::spawn` 闭包改拿 `Services`。**登记后续项**:`GroupsLoaded`/`Polled` 没有项目/代数保护(`Polled` 靠 `selected` 比对,`GroupsLoaded` 无);剩余直接用 `Client` 的面板:`conversations`、`codehealth`、`agent_context`、`browser`、`edit_history`、`settings`、`project_create`。
- `CLAUDE.md` Workspace 布局表 `dozer-services` 行里"Usage、Todo、Project 已迁,其余面板待迁"改为"Usage、Todo、Project、GroupChat 已迁,其余面板待迁"。
```bash
git add scripts/audit docs CLAUDE.md
git commit -m "chore(audit): GroupChat S5 R-CLIENT 基线下调 + 文档回填

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

之后按 executing-plans 的 Final Review(整分支 fresh reviewer)与 finishing-a-development-branch;**合并进 main 属于需先询问用户的外部副作用**,合并前核对 main 上用户 WIP(`Cargo.lock`、`crates/dozer-app/Cargo.toml`、`Info.plist` 及 `extensions/files/*` 等)与本分支的重叠,用 `git stash` → 合并 → `git stash pop`。
